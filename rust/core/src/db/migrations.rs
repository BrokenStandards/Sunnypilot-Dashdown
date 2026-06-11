//! Tiny forward-only migration runner. Each entry's index+1 is its version;
//! applied in a transaction when greater than the current `schema_version`.
//! Idempotent: re-running applies nothing. Later milestones append entries.

use rusqlite::{params, Connection};

use crate::error::Result;

const MIGRATIONS: &[&str] = &[
    include_str!("schema.sql"),          // v1: device, segment, seg_file
    include_str!("schema_drive.sql"),    // v2: drive (M2)
    include_str!("schema_job.sql"),      // v3: download_job (M4)
    include_str!("schema_identity.sql"), // v4: device_identity (B1)
    include_str!("schema_cap_warn.sql"), // v5: device cap-warning toggle + threshold
];

pub const LATEST_VERSION: i64 = MIGRATIONS.len() as i64;

pub fn apply(conn: &mut Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")?;
    let current = current_version(conn)?;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let v = (i + 1) as i64;
        if v > current {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![v],
            )?;
            tx.commit()?;
        }
    }
    Ok(())
}

pub fn current_version(conn: &Connection) -> Result<i64> {
    let v: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |r| r.get(0),
    )?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device row created before v5 must come out enabled with the 10-minute
    /// default after upgrading (preserving the prior always-on warning behavior).
    #[test]
    fn v5_backfills_cap_warn_defaults_on_existing_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        // Build a pre-v5 (v4) database: apply migrations 1..=4 and record versions.
        conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")
            .unwrap();
        for (i, sql) in MIGRATIONS.iter().take(4).enumerate() {
            conn.execute_batch(sql).unwrap();
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![(i + 1) as i64],
            )
            .unwrap();
        }
        // Insert a device the old way — no cap_warn columns exist yet.
        conn.execute(
            "INSERT INTO device (name, hotspot_ip, port, active_mode, auto_sync) \
             VALUES ('old', '192.168.43.1', 3923, 'hotspot', 0)",
            [],
        )
        .unwrap();

        // Upgrade: only v5 runs, adding the two columns with their defaults.
        apply(&mut conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), 5);

        let (enabled, threshold): (i64, i64) = conn
            .query_row(
                "SELECT cap_warn_enabled, cap_warn_threshold_minutes FROM device",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((enabled, threshold), (1, 10));
    }
}
