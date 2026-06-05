//! SQLite index (rusqlite + r2d2 pool, WAL). The async core calls these sync
//! methods via `tokio::task::spawn_blocking` (wired up by M4/M8 callers).

pub mod migrations;

use std::path::Path;

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{CoreError, Result};
use crate::model::{ConnMode, Device, FileKind, FileSelection, Segment, SegmentFile, SegmentName};

pub type Pool = r2d2::Pool<SqliteConnectionManager>;
type PooledConn = r2d2::PooledConnection<SqliteConnectionManager>;

const DEVICE_COLS: &str = "id, name, dongle_label, hotspot_ip, wifi_ip, port, active_mode, \
    password, auto_sync, file_selection, retention_max_minutes, auto_delete_from_comma, \
    auto_delete_min_age_min";

pub struct Repo {
    pool: Pool,
}

fn init_conn(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")
}

impl Repo {
    /// Open (creating if needed) a file-backed database and run migrations.
    pub fn open(path: &Path) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path).with_init(init_conn);
        Self::from_manager(manager, None)
    }

    /// In-memory database (single connection so the schema persists across
    /// `get()`s). Intended for tests.
    pub fn open_in_memory() -> Result<Self> {
        let manager = SqliteConnectionManager::memory().with_init(init_conn);
        Self::from_manager(manager, Some(1))
    }

    fn from_manager(manager: SqliteConnectionManager, max_size: Option<u32>) -> Result<Self> {
        let mut builder = r2d2::Pool::builder();
        if let Some(m) = max_size {
            builder = builder.max_size(m);
        }
        let pool = builder.build(manager)?;
        {
            let mut conn = pool.get()?;
            migrations::apply(&mut conn)?;
        }
        Ok(Self { pool })
    }

    fn conn(&self) -> Result<PooledConn> {
        self.pool.get().map_err(CoreError::from)
    }

    pub fn schema_version(&self) -> Result<i64> {
        let conn = self.conn()?;
        migrations::current_version(&conn)
    }

    // ---- devices ----------------------------------------------------------

    pub fn insert_device(&self, d: &Device) -> Result<i64> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO device (name, dongle_label, hotspot_ip, wifi_ip, port, active_mode, \
                password, auto_sync, file_selection, retention_max_minutes, \
                auto_delete_from_comma, auto_delete_min_age_min) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                d.name,
                d.dongle_label,
                d.hotspot_ip,
                d.wifi_ip,
                d.port,
                d.active_mode.as_str(),
                d.password,
                d.auto_sync,
                d.file_selection.as_str(),
                d.retention_max_minutes,
                d.auto_delete_from_comma,
                d.auto_delete_min_age_min,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_device(&self, id: i64) -> Result<Option<Device>> {
        let conn = self.conn()?;
        let raw = conn
            .query_row(
                &format!("SELECT {DEVICE_COLS} FROM device WHERE id = ?1"),
                params![id],
                map_raw_device,
            )
            .optional()?;
        raw.map(raw_to_device).transpose()
    }

    pub fn list_devices(&self) -> Result<Vec<Device>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("SELECT {DEVICE_COLS} FROM device ORDER BY id"))?;
        let raws: rusqlite::Result<Vec<RawDevice>> = stmt.query_map([], map_raw_device)?.collect();
        raws?.into_iter().map(raw_to_device).collect()
    }

    // ---- segments + files -------------------------------------------------

    /// Insert or update the given segments (and their files) for a device.
    pub fn upsert_segments(&self, device_id: i64, segments: &[Segment]) -> Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        for seg in segments {
            tx.execute(
                "INSERT INTO segment (device_id, route_id, segment_num, recording) \
                 VALUES (?1,?2,?3,?4) \
                 ON CONFLICT(device_id, route_id, segment_num) \
                 DO UPDATE SET recording = excluded.recording",
                params![
                    device_id,
                    seg.name.route_id,
                    seg.name.segment_num as i64,
                    seg.recording
                ],
            )?;
            let seg_id: i64 = tx.query_row(
                "SELECT id FROM segment WHERE device_id=?1 AND route_id=?2 AND segment_num=?3",
                params![device_id, seg.name.route_id, seg.name.segment_num as i64],
                |r| r.get(0),
            )?;
            for f in &seg.files {
                tx.execute(
                    "INSERT INTO seg_file (segment_id, kind, name, remote_size, mtime_s) \
                     VALUES (?1,?2,?3,?4,?5) \
                     ON CONFLICT(segment_id, name) \
                     DO UPDATE SET kind=excluded.kind, remote_size=excluded.remote_size, \
                        mtime_s=excluded.mtime_s",
                    params![
                        seg_id,
                        f.kind.as_str(),
                        f.name,
                        f.remote_size as i64,
                        f.mtime_s
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_segments(&self, device_id: i64) -> Result<Vec<Segment>> {
        let conn = self.conn()?;
        // Collect segment rows first so the statement is dropped before we
        // prepare the per-segment file query.
        let seg_rows: Vec<(i64, String, i64, bool)> = {
            let mut stmt = conn.prepare(
                "SELECT id, route_id, segment_num, recording FROM segment \
                 WHERE device_id=?1 ORDER BY route_id, segment_num",
            )?;
            let rows = stmt.query_map(params![device_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };

        let mut out = Vec::with_capacity(seg_rows.len());
        for (seg_id, route_id, segment_num, recording) in seg_rows {
            let mut fstmt = conn.prepare(
                "SELECT kind, name, remote_size, mtime_s FROM seg_file \
                 WHERE segment_id=?1 ORDER BY name",
            )?;
            let files: rusqlite::Result<Vec<SegmentFile>> = fstmt
                .query_map(params![seg_id], |r| {
                    Ok(SegmentFile {
                        kind: FileKind::from_db(&r.get::<_, String>(0)?),
                        name: r.get(1)?,
                        remote_size: r.get::<_, i64>(2)? as u64,
                        mtime_s: r.get(3)?,
                    })
                })?
                .collect();
            out.push(Segment {
                name: SegmentName {
                    route_id,
                    segment_num: segment_num as u32,
                },
                files: files?,
                recording,
            });
        }
        Ok(out)
    }
}

/// Raw device row (primitive columns), converted to `Device` afterwards so enum
/// parsing can surface a `CoreError` instead of being trapped in a closure.
struct RawDevice {
    id: i64,
    name: String,
    dongle_label: Option<String>,
    hotspot_ip: String,
    wifi_ip: Option<String>,
    port: i64,
    active_mode: String,
    password: Option<String>,
    auto_sync: bool,
    file_selection: String,
    retention_max_minutes: Option<i64>,
    auto_delete_from_comma: bool,
    auto_delete_min_age_min: i64,
}

fn map_raw_device(r: &rusqlite::Row) -> rusqlite::Result<RawDevice> {
    Ok(RawDevice {
        id: r.get(0)?,
        name: r.get(1)?,
        dongle_label: r.get(2)?,
        hotspot_ip: r.get(3)?,
        wifi_ip: r.get(4)?,
        port: r.get(5)?,
        active_mode: r.get(6)?,
        password: r.get(7)?,
        auto_sync: r.get(8)?,
        file_selection: r.get(9)?,
        retention_max_minutes: r.get(10)?,
        auto_delete_from_comma: r.get(11)?,
        auto_delete_min_age_min: r.get(12)?,
    })
}

fn raw_to_device(r: RawDevice) -> Result<Device> {
    Ok(Device {
        id: r.id,
        name: r.name,
        dongle_label: r.dongle_label,
        hotspot_ip: r.hotspot_ip,
        wifi_ip: r.wifi_ip,
        port: r.port as u16,
        active_mode: ConnMode::parse(&r.active_mode)?,
        password: r.password,
        auto_sync: r.auto_sync,
        file_selection: FileSelection::parse(&r.file_selection)?,
        retention_max_minutes: r.retention_max_minutes,
        auto_delete_from_comma: r.auto_delete_from_comma,
        auto_delete_min_age_min: r.auto_delete_min_age_min,
    })
}
