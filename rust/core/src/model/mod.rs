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

/// Which files to sync for a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSelection {
    PreviewsOnly,
    FullVideo,
    FullVideoPlusLogs,
}

impl FileSelection {
    pub fn as_str(self) -> &'static str {
        match self {
            FileSelection::PreviewsOnly => "previews_only",
            FileSelection::FullVideo => "full_video",
            FileSelection::FullVideoPlusLogs => "full_video_plus_logs",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "previews_only" => Ok(FileSelection::PreviewsOnly),
            "full_video" => Ok(FileSelection::FullVideo),
            "full_video_plus_logs" => Ok(FileSelection::FullVideoPlusLogs),
            other => Err(CoreError::Parse(format!("bad file selection: {other}"))),
        }
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
