//! Domain model. Plain Rust types in M1 (no UniFFI derives yet — the boundary
//! is added in M8). The mirror folder is the source of truth; SQLite is a
//! rebuildable index over these.

pub mod file_kind;
pub mod ids;
pub mod time;

pub use file_kind::FileKind;
pub use ids::SegmentName;

use crate::error::{CoreError, Result};

/// Which IP a device is currently reached on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnMode {
    Hotspot,
    Wifi,
}

impl ConnMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ConnMode::Hotspot => "hotspot",
            ConnMode::Wifi => "wifi",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "hotspot" => Ok(ConnMode::Hotspot),
            "wifi" => Ok(ConnMode::Wifi),
            other => Err(CoreError::Parse(format!("bad conn mode: {other}"))),
        }
    }
}

/// Which file streams to sync for a device — one toggle per downloadable kind.
/// Audio is muxed into `qcamera.ts` upstream (sunnypilot `RecordAudio`), so it
/// rides with the `qcamera` toggle rather than being a separate file. Persisted
/// as a sorted CSV of the enabled kind tokens in `device.file_selection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSelection {
    pub fcamera: bool,
    pub ecamera: bool,
    pub dcamera: bool,
    pub qcamera: bool,
    pub rlog: bool,
    pub qlog: bool,
    pub bootlog: bool,
    pub other: bool,
}

impl FileSelection {
    /// Nothing selected.
    pub const NONE: FileSelection = FileSelection {
        fcamera: false,
        ecamera: false,
        dcamera: false,
        qcamera: false,
        rlog: false,
        qlog: false,
        bootlog: false,
        other: false,
    };

    /// Only the low-res preview (+ muxed audio) — the lightweight default.
    pub fn previews_only() -> Self {
        FileSelection {
            qcamera: true,
            ..Self::NONE
        }
    }

    /// Every downloadable stream.
    pub fn everything() -> Self {
        FileSelection {
            fcamera: true,
            ecamera: true,
            dcamera: true,
            qcamera: true,
            rlog: true,
            qlog: true,
            bootlog: true,
            other: true,
        }
    }

    /// Whether files of `kind` are selected for download. The lock marker is
    /// never a download target.
    pub fn includes(&self, kind: FileKind) -> bool {
        match kind {
            FileKind::FCamera => self.fcamera,
            FileKind::ECamera => self.ecamera,
            FileKind::DCamera => self.dcamera,
            FileKind::QCamera => self.qcamera,
            FileKind::RLog => self.rlog,
            FileKind::QLog => self.qlog,
            FileKind::BootLog => self.bootlog,
            FileKind::Other => self.other,
            FileKind::LockMarker => false,
        }
    }

    /// Serialize to a sorted CSV of enabled kind tokens (matching `FileKind::as_str`).
    pub fn as_str(&self) -> String {
        let mut tokens = Vec::new();
        for (on, kind) in [
            (self.fcamera, FileKind::FCamera),
            (self.ecamera, FileKind::ECamera),
            (self.dcamera, FileKind::DCamera),
            (self.qcamera, FileKind::QCamera),
            (self.rlog, FileKind::RLog),
            (self.qlog, FileKind::QLog),
            (self.bootlog, FileKind::BootLog),
            (self.other, FileKind::Other),
        ] {
            if on {
                tokens.push(kind.as_str());
            }
        }
        tokens.join(",")
    }

    /// Parse the CSV form. Also accepts the legacy preset names. Unknown tokens
    /// are ignored (forward-compatible with kinds a newer build might add).
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "previews_only" => return Ok(Self::previews_only()),
            "full_video" => {
                return Ok(FileSelection {
                    fcamera: true,
                    ecamera: true,
                    dcamera: true,
                    qcamera: true,
                    ..Self::NONE
                })
            }
            "full_video_plus_logs" => return Ok(Self::everything()),
            _ => {}
        }
        let mut sel = Self::NONE;
        for tok in s.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            match tok {
                "fcamera" => sel.fcamera = true,
                "ecamera" => sel.ecamera = true,
                "dcamera" => sel.dcamera = true,
                "qcamera" => sel.qcamera = true,
                "rlog" => sel.rlog = true,
                "qlog" => sel.qlog = true,
                "bootlog" => sel.bootlog = true,
                "other" => sel.other = true,
                _ => {} // ignore unknown tokens
            }
        }
        Ok(sel)
    }
}

impl Default for FileSelection {
    fn default() -> Self {
        Self::previews_only()
    }
}

/// Local download state of a single file. `local_size`/state are populated by
/// the mirror/sync engine in M3–M5; until then files default to `Missing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadState {
    Missing,
    InProgress,
    Complete,
    SizeMismatch,
}

impl DownloadState {
    pub fn as_str(self) -> &'static str {
        match self {
            DownloadState::Missing => "missing",
            DownloadState::InProgress => "in_progress",
            DownloadState::Complete => "complete",
            DownloadState::SizeMismatch => "size_mismatch",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "missing" => Ok(DownloadState::Missing),
            "in_progress" => Ok(DownloadState::InProgress),
            "complete" => Ok(DownloadState::Complete),
            "size_mismatch" => Ok(DownloadState::SizeMismatch),
            other => Err(CoreError::Parse(format!("bad download state: {other}"))),
        }
    }
}

/// Sync state of a whole drive against the active file selection. Stored in the
/// `drive` table from M2; only `NotDownloaded` is produced until the sync engine
/// (M5) computes the real value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    NotDownloaded,
    Partial,
    Complete,
    Downloading,
    Failed,
}

impl SyncStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncStatus::NotDownloaded => "not_downloaded",
            SyncStatus::Partial => "partial",
            SyncStatus::Complete => "complete",
            SyncStatus::Downloading => "downloading",
            SyncStatus::Failed => "failed",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "not_downloaded" => Ok(SyncStatus::NotDownloaded),
            "partial" => Ok(SyncStatus::Partial),
            "complete" => Ok(SyncStatus::Complete),
            "downloading" => Ok(SyncStatus::Downloading),
            "failed" => Ok(SyncStatus::Failed),
            other => Err(CoreError::Parse(format!("bad sync state: {other}"))),
        }
    }
}

/// State of a drive download job (`download_job.state`). A drive download is
/// either in progress or in a terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Running,
    Complete,
    Failed,
    Canceled,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Running => "running",
            JobState::Complete => "complete",
            JobState::Failed => "failed",
            JobState::Canceled => "canceled",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "running" => Ok(JobState::Running),
            "complete" => Ok(JobState::Complete),
            "failed" => Ok(JobState::Failed),
            "canceled" => Ok(JobState::Canceled),
            other => Err(CoreError::Parse(format!("bad job state: {other}"))),
        }
    }
}

/// One file inside a segment, as seen on the remote (copyparty) side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentFile {
    pub kind: FileKind,
    pub name: String,
    pub remote_size: u64,
    pub mtime_s: i64,
}

/// One 1-minute segment: its decomposed name, its files, and whether it is
/// still recording (an `rlog.lock` was present in the listing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub name: SegmentName,
    pub files: Vec<SegmentFile>,
    pub recording: bool,
}

impl Segment {
    /// Best-effort wall-clock time (epoch ms) from the newest file mtime.
    /// Used as the time signal for M2 grouping, since names carry no timestamp.
    pub fn approx_time_ms(&self) -> Option<i64> {
        self.files
            .iter()
            .map(|f| f.mtime_s)
            .max()
            .map(time::secs_to_ms)
    }
}

/// A *drive*: a maximal run of consecutive 1-minute segments within one route
/// (see `drive_grouping::group_segments`). Owns its segments so callers/tests
/// can expand it; the summary fields are derived from `segments` and are only
/// ever set by `group_segments` or DB hydration, so they stay consistent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    /// First segment's `dir_name()` — stable as the drive grows.
    pub drive_key: String,
    pub route_id: String,
    pub first_segment_num: u32,
    pub last_segment_num: u32,
    /// Earliest segment's approx wall-clock time (epoch ms); `None` if it has no files.
    pub start_ms: Option<i64>,
    /// Last segment's approx time + one segment length (half-open, conservative
    /// for the M6 retention age guard); `None` if the last segment has no files.
    pub end_ms: Option<i64>,
    pub segment_count: u32,
    /// Any segment still recording (`rlog.lock` present) — typically the last.
    pub recording: bool,
    /// Default `NotDownloaded` in M2; the real value is computed in M5.
    pub sync_state: SyncStatus,
    /// User pin; default `false`, behavior lands in M6.
    pub preserved: bool,
    pub segments: Vec<Segment>,
}

/// A configured Comma device. Connection fields are exercised in M1; the
/// settings fields (auto-sync, retention, auto-delete) are stored now but their
/// behavior lands in later milestones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: i64, // 0 = not yet persisted
    pub name: String,
    pub dongle_label: Option<String>,
    pub hotspot_ip: String,
    pub wifi_ip: Option<String>,
    pub port: u16,
    pub active_mode: ConnMode,
    pub password: Option<String>,
    pub auto_sync: bool,
    pub file_selection: FileSelection,
    pub retention_max_minutes: Option<i64>,
    pub auto_delete_from_comma: bool,
    pub auto_delete_min_age_min: i64,
}

impl Device {
    /// The IP currently in use, based on `active_mode`. Falls back to the
    /// hotspot IP if wifi is selected but unset.
    pub fn active_ip(&self) -> &str {
        match self.active_mode {
            ConnMode::Wifi => self.wifi_ip.as_deref().unwrap_or(&self.hotspot_ip),
            ConnMode::Hotspot => &self.hotspot_ip,
        }
    }

    /// Base copyparty URL for the active connection, e.g. `http://192.168.43.1:3923/`.
    pub fn base_url(&self) -> String {
        format!("http://{}:{}/", self.active_ip(), self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_status_str_round_trip() {
        for s in [
            SyncStatus::NotDownloaded,
            SyncStatus::Partial,
            SyncStatus::Complete,
            SyncStatus::Downloading,
            SyncStatus::Failed,
        ] {
            assert_eq!(SyncStatus::parse(s.as_str()).unwrap(), s);
        }
        assert!(SyncStatus::parse("nonsense").is_err());
    }

    #[test]
    fn file_selection_previews_only_is_qcamera() {
        let sel = FileSelection::previews_only();
        assert!(sel.includes(FileKind::QCamera));
        for k in [
            FileKind::FCamera,
            FileKind::ECamera,
            FileKind::DCamera,
            FileKind::RLog,
            FileKind::QLog,
            FileKind::BootLog,
            FileKind::Other,
        ] {
            assert!(!sel.includes(k), "{k:?} should not be selected");
        }
        // The lock marker is never a download target, even with everything on.
        assert!(!FileSelection::everything().includes(FileKind::LockMarker));
    }

    #[test]
    fn file_selection_everything_includes_all_downloadable() {
        let sel = FileSelection::everything();
        for k in [
            FileKind::FCamera,
            FileKind::ECamera,
            FileKind::DCamera,
            FileKind::QCamera,
            FileKind::RLog,
            FileKind::QLog,
            FileKind::BootLog,
            FileKind::Other,
        ] {
            assert!(sel.includes(k), "{k:?} should be selected");
        }
    }

    #[test]
    fn file_selection_csv_round_trip() {
        for sel in [
            FileSelection::NONE,
            FileSelection::previews_only(),
            FileSelection::everything(),
            FileSelection {
                fcamera: true,
                rlog: true,
                ..FileSelection::NONE
            },
        ] {
            assert_eq!(FileSelection::parse(&sel.as_str()).unwrap(), sel);
        }
        // Canonical CSV form, sorted by the fixed kind order.
        assert_eq!(
            FileSelection {
                qcamera: true,
                fcamera: true,
                rlog: true,
                ..FileSelection::NONE
            }
            .as_str(),
            "fcamera,qcamera,rlog"
        );
        // Unknown tokens are ignored; empty string => nothing.
        assert_eq!(
            FileSelection::parse("qcamera,bogus").unwrap(),
            FileSelection::previews_only()
        );
        assert_eq!(FileSelection::parse("").unwrap(), FileSelection::NONE);
    }

    #[test]
    fn file_selection_parses_legacy_presets() {
        assert_eq!(
            FileSelection::parse("previews_only").unwrap(),
            FileSelection::previews_only()
        );
        assert_eq!(
            FileSelection::parse("full_video_plus_logs").unwrap(),
            FileSelection::everything()
        );
        assert!(FileSelection::parse("full_video").unwrap().fcamera);
    }
}
