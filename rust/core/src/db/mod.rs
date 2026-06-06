//! SQLite index (rusqlite + r2d2 pool, WAL). The async core calls these sync
//! methods via `tokio::task::spawn_blocking` (wired up by M4/M8 callers).

pub mod migrations;

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{CoreError, Result};
use crate::model::{
    ConnMode, Device, Drive, FileKind, FileSelection, JobState, Segment, SegmentFile, SegmentName,
    SyncStatus,
};

/// Current wall-clock time in epoch seconds (for `download_job.updated_s`).
fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub type Pool = r2d2::Pool<SqliteConnectionManager>;
type PooledConn = r2d2::PooledConnection<SqliteConnectionManager>;

const DEVICE_COLS: &str = "id, name, dongle_label, hotspot_ip, wifi_ip, port, active_mode, \
    password, auto_sync, file_selection, retention_max_minutes, auto_delete_from_comma, \
    auto_delete_min_age_min";

const DRIVE_COLS: &str = "drive_key, route_id, first_seg, last_seg, start_ms, end_ms, \
    segment_count, recording, preserved, sync_state";

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

    // ---- drives -----------------------------------------------------------

    /// Replace the device's drive set with `drives`: upsert each (updating only
    /// the *derived* columns, leaving `preserved`/`sync_state` intact) and prune
    /// vanished drives. Pruning keeps any drive that still holds local data
    /// (`sync_state` complete/partial/downloading) or is `preserved` — so a drive
    /// deliberately deleted from the comma (M6 auto-delete) but mirrored locally
    /// stays in the library; only purely-remote, unpinned drives are dropped.
    /// One transaction.
    pub fn replace_drives(&self, device_id: i64, drives: &[Drive]) -> Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        for d in drives {
            tx.execute(
                "INSERT INTO drive \
                    (device_id, drive_key, route_id, first_seg, last_seg, start_ms, end_ms, \
                     segment_count, recording) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) \
                 ON CONFLICT(device_id, drive_key) DO UPDATE SET \
                    route_id=excluded.route_id, first_seg=excluded.first_seg, \
                    last_seg=excluded.last_seg, start_ms=excluded.start_ms, \
                    end_ms=excluded.end_ms, segment_count=excluded.segment_count, \
                    recording=excluded.recording",
                params![
                    device_id,
                    d.drive_key,
                    d.route_id,
                    d.first_segment_num as i64,
                    d.last_segment_num as i64,
                    d.start_ms,
                    d.end_ms,
                    d.segment_count as i64,
                    d.recording,
                ],
            )?;
        }
        // Prune vanished drives, but keep any with local data or a user pin
        // (purely-remote, unpinned drives only).
        const KEEP_LOCAL: &str = "preserved=0 AND sync_state IN ('not_downloaded','failed')";
        if drives.is_empty() {
            tx.execute(
                &format!("DELETE FROM drive WHERE device_id=?1 AND {KEEP_LOCAL}"),
                params![device_id],
            )?;
        } else {
            let placeholders = vec!["?"; drives.len()].join(",");
            let sql = format!(
                "DELETE FROM drive WHERE device_id=? AND drive_key NOT IN ({placeholders}) \
                 AND {KEEP_LOCAL}"
            );
            let mut p: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(drives.len() + 1);
            p.push(&device_id);
            for d in drives {
                p.push(&d.drive_key);
            }
            tx.execute(&sql, p.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Read the device's drives (ordered by route then start index), hydrating
    /// each drive's segments via a contiguous-range fetch.
    pub fn get_drives(&self, device_id: i64) -> Result<Vec<Drive>> {
        let conn = self.conn()?;
        let raws: Vec<RawDrive> = {
            let mut stmt = conn.prepare(&format!(
                "SELECT {DRIVE_COLS} FROM drive WHERE device_id=?1 ORDER BY route_id, first_seg"
            ))?;
            let rows = stmt.query_map(params![device_id], map_raw_drive)?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut out = Vec::with_capacity(raws.len());
        for raw in raws {
            let segments =
                segments_in_range(&conn, device_id, &raw.route_id, raw.first_seg, raw.last_seg)?;
            out.push(Drive {
                drive_key: raw.drive_key,
                first_segment_num: raw.first_seg as u32,
                last_segment_num: raw.last_seg as u32,
                start_ms: raw.start_ms,
                end_ms: raw.end_ms,
                segment_count: raw.segment_count as u32,
                recording: raw.recording,
                sync_state: SyncStatus::parse(&raw.sync_state)?,
                preserved: raw.preserved,
                route_id: raw.route_id,
                segments,
            });
        }
        Ok(out)
    }

    /// Set a drive's `preserved` pin (M6 behavior; the setter exists from M2).
    pub fn set_drive_preserved(
        &self,
        device_id: i64,
        drive_key: &str,
        preserved: bool,
    ) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE drive SET preserved=?3 WHERE device_id=?1 AND drive_key=?2",
            params![device_id, drive_key, preserved],
        )?;
        Ok(())
    }

    /// Set a drive's `sync_state` (M5 behavior; the setter exists from M2).
    pub fn set_drive_sync_state(
        &self,
        device_id: i64,
        drive_key: &str,
        state: SyncStatus,
    ) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE drive SET sync_state=?3 WHERE device_id=?1 AND drive_key=?2",
            params![device_id, drive_key, state.as_str()],
        )?;
        Ok(())
    }

    // ---- seg_file local state ---------------------------------------------

    /// Mark one file complete: record its on-disk size and `download_state`.
    /// Keyed by the natural tuple; resolves `segment_id` via a sub-select so
    /// callers never juggle ids. A no-op if the seg_file row is absent.
    pub fn set_file_complete(
        &self,
        device_id: i64,
        route_id: &str,
        segment_num: u32,
        file_name: &str,
        local_size: u64,
    ) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE seg_file SET local_size=?1, download_state='complete' \
             WHERE name=?2 AND segment_id=( \
                SELECT id FROM segment WHERE device_id=?3 AND route_id=?4 AND segment_num=?5)",
            params![
                local_size as i64,
                file_name,
                device_id,
                route_id,
                segment_num as i64
            ],
        )?;
        Ok(())
    }

    // ---- download jobs ----------------------------------------------------

    /// Start (or restart) a drive's job row: state=running, progress reset.
    pub fn upsert_job(
        &self,
        device_id: i64,
        drive_key: &str,
        files_total: u32,
        bytes_total: u64,
    ) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO download_job \
                (device_id, drive_key, state, files_total, files_done, bytes_total, bytes_done, \
                 error, updated_s) \
             VALUES (?1,?2,'running',?3,0,?4,0,NULL,?5) \
             ON CONFLICT(device_id, drive_key) DO UPDATE SET \
                state='running', files_total=excluded.files_total, files_done=0, \
                bytes_total=excluded.bytes_total, bytes_done=0, error=NULL, \
                updated_s=excluded.updated_s",
            params![
                device_id,
                drive_key,
                files_total as i64,
                bytes_total as i64,
                now_s()
            ],
        )?;
        Ok(())
    }

    /// Move a job to a new state (terminal states carry an optional error).
    pub fn set_job_state(
        &self,
        device_id: i64,
        drive_key: &str,
        state: JobState,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE download_job SET state=?3, error=?4, updated_s=?5 \
             WHERE device_id=?1 AND drive_key=?2",
            params![device_id, drive_key, state.as_str(), error, now_s()],
        )?;
        Ok(())
    }

    /// Set absolute progress counters for a running job.
    pub fn bump_job_progress(
        &self,
        device_id: i64,
        drive_key: &str,
        files_done: u32,
        bytes_done: u64,
    ) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE download_job SET files_done=?3, bytes_done=?4, updated_s=?5 \
             WHERE device_id=?1 AND drive_key=?2",
            params![
                device_id,
                drive_key,
                files_done as i64,
                bytes_done as i64,
                now_s()
            ],
        )?;
        Ok(())
    }

    pub fn get_job(&self, device_id: i64, drive_key: &str) -> Result<Option<JobRow>> {
        let conn = self.conn()?;
        let raw = conn
            .query_row(
                "SELECT device_id, drive_key, state, files_total, files_done, bytes_total, \
                    bytes_done, error, updated_s \
                 FROM download_job WHERE device_id=?1 AND drive_key=?2",
                params![device_id, drive_key],
                map_raw_job,
            )
            .optional()?;
        raw.map(raw_to_job).transpose()
    }

    /// Whether the device has a download job currently `running` — the
    /// "download active for this device" signal behind the Blue connectivity dot
    /// (M7). A single-query EXISTS; reuses `JobState::Running` so the state token
    /// stays owned by the enum.
    pub fn has_active_job(&self, device_id: i64) -> Result<bool> {
        let conn = self.conn()?;
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM download_job WHERE device_id=?1 AND state=?2)",
            params![device_id, JobState::Running.as_str()],
            |r| r.get(0),
        )?;
        Ok(exists)
    }
}

/// A `download_job` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRow {
    pub device_id: i64,
    pub drive_key: String,
    pub state: JobState,
    pub files_total: u32,
    pub files_done: u32,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub error: Option<String>,
    pub updated_s: i64,
}

struct RawJob {
    device_id: i64,
    drive_key: String,
    state: String,
    files_total: i64,
    files_done: i64,
    bytes_total: i64,
    bytes_done: i64,
    error: Option<String>,
    updated_s: i64,
}

fn map_raw_job(r: &rusqlite::Row) -> rusqlite::Result<RawJob> {
    Ok(RawJob {
        device_id: r.get(0)?,
        drive_key: r.get(1)?,
        state: r.get(2)?,
        files_total: r.get(3)?,
        files_done: r.get(4)?,
        bytes_total: r.get(5)?,
        bytes_done: r.get(6)?,
        error: r.get(7)?,
        updated_s: r.get(8)?,
    })
}

fn raw_to_job(r: RawJob) -> Result<JobRow> {
    Ok(JobRow {
        device_id: r.device_id,
        drive_key: r.drive_key,
        state: JobState::parse(&r.state)?,
        files_total: r.files_total as u32,
        files_done: r.files_done as u32,
        bytes_total: r.bytes_total as u64,
        bytes_done: r.bytes_done as u64,
        error: r.error,
        updated_s: r.updated_s,
    })
}

/// Fetch one route's segments whose index falls in `[first, last]`, with files.
/// Safe because a drive's indices are contiguous by construction. Takes a
/// borrowed `Connection` so callers reuse a single pooled connection (the
/// in-memory test pool has only one).
fn segments_in_range(
    conn: &Connection,
    device_id: i64,
    route_id: &str,
    first: i64,
    last: i64,
) -> Result<Vec<Segment>> {
    let seg_rows: Vec<(i64, i64, bool)> = {
        let mut stmt = conn.prepare(
            "SELECT id, segment_num, recording FROM segment \
             WHERE device_id=?1 AND route_id=?2 AND segment_num BETWEEN ?3 AND ?4 \
             ORDER BY segment_num",
        )?;
        let rows = stmt.query_map(params![device_id, route_id, first, last], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };

    let mut out = Vec::with_capacity(seg_rows.len());
    for (seg_id, segment_num, recording) in seg_rows {
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
                route_id: route_id.to_string(),
                segment_num: segment_num as u32,
            },
            files: files?,
            recording,
        });
    }
    Ok(out)
}

/// Raw `drive` row; `sync_state` parsed into [`SyncStatus`] outside the row
/// closure so a bad value surfaces a `CoreError` (mirrors `RawDevice`).
struct RawDrive {
    drive_key: String,
    route_id: String,
    first_seg: i64,
    last_seg: i64,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
    segment_count: i64,
    recording: bool,
    preserved: bool,
    sync_state: String,
}

fn map_raw_drive(r: &rusqlite::Row) -> rusqlite::Result<RawDrive> {
    Ok(RawDrive {
        drive_key: r.get(0)?,
        route_id: r.get(1)?,
        first_seg: r.get(2)?,
        last_seg: r.get(3)?,
        start_ms: r.get(4)?,
        end_ms: r.get(5)?,
        segment_count: r.get(6)?,
        recording: r.get(7)?,
        preserved: r.get(8)?,
        sync_state: r.get(9)?,
    })
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
