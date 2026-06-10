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
