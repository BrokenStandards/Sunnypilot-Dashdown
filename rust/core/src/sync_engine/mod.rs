//! The sync/download engine: refresh a device's index (`sync_now`) and download
//! a drive's selected files with progress + cancellation (`download_drive` /
//! `start_drive_download`). The Rust core owns the transfer engine; native
//! scheduling (Android Foreground Service / iOS BGTask) drives it in Phase B.

pub mod download_job;

use std::path::PathBuf;
use std::sync::Arc;

pub use tokio_util::sync::CancellationToken;

pub use download_job::{
    download_file, DownloadProgress, FileOutcome, JobOutcome, ProgressSink, MAX_ATTEMPTS,
};

use crate::copyparty_client::{CopypartyClient, Credentials};
use crate::db::Repo;
use crate::drive_grouping::group_segments;
use crate::error::{CoreError, Result};
use crate::model::{Device, Drive, JobState, SyncStatus};
use crate::storage::{paths::file_rel, MirrorStore};

/// Segments path under each device's copyparty root.
/// (TODO M8: make device-configurable if a device serves realdata elsewhere.)
const REALDATA_REL: &str = "realdata/";

/// One selected file to (maybe) download.
struct Item {
    route_id: String,
    segment_num: u32,
    name: String,
    rel: String,
    size: u64,
}

enum Terminal {
    Complete,
    Canceled,
    Failed(String),
}

#[derive(Clone)]
pub struct SyncEngine {
    repo: Arc<Repo>,
    mirror_root: PathBuf,
}

/// A cancellable handle to a running drive download. The native layer owns the
/// task lifecycle (per the background contract); this only signals cancellation.
pub struct SyncHandle {
    token: CancellationToken,
}

impl SyncHandle {
    pub fn cancel(&self) {
        self.token.cancel();
    }
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

impl SyncEngine {
    pub fn new(repo: Arc<Repo>, mirror_root: impl Into<PathBuf>) -> Self {
        Self {
            repo,
            mirror_root: mirror_root.into(),
        }
    }

    fn client_for(device: &Device) -> Result<CopypartyClient> {
        let creds = match &device.password {
            Some(p) => Credentials::Password(p.clone()),
            None => Credentials::Anonymous,
        };
        CopypartyClient::new(&device.base_url(), creds)
    }

    fn mirror_for(&self, device: &Device) -> MirrorStore {
        MirrorStore::new(self.mirror_root.join(device.id.to_string()))
    }

    /// Refresh the device's index: list segments, persist them, regroup, persist
    /// drives. Does NOT download. Returns the device's drives (hydrated,
    /// preserving stored `sync_state`/`preserved`).
    pub async fn sync_now(&self, device: &Device) -> Result<Vec<Drive>> {
        let client = Self::client_for(device)?;
        let segments = client.list_segments(REALDATA_REL).await?;

        let device_id = device.id;
        db(self.repo.clone(), move |r| {
            r.upsert_segments(device_id, &segments)?;
            let drives = group_segments(segments);
            r.replace_drives(device_id, &drives)?;
            r.get_drives(device_id)
        })
        .await
    }

    /// Spawn a drive download on its own task; return a handle to cancel it.
    pub fn start_drive_download(
        &self,
        device: Device,
        drive_key: String,
        sink: Arc<dyn ProgressSink>,
    ) -> SyncHandle {
        let token = CancellationToken::new();
        let engine = self.clone();
        let child = token.clone();
        tokio::spawn(async move {
            let _ = engine
                .download_drive(&device, &drive_key, sink, child)
                .await;
        });
        SyncHandle { token }
    }

    /// Awaitable, cancellable download of one drive's selected files. Updates the
    /// `download_job` row + the drive's `sync_state` and fires `ProgressSink`.
    pub async fn download_drive(
        &self,
        device: &Device,
        drive_key: &str,
        sink: Arc<dyn ProgressSink>,
        cancel: CancellationToken,
    ) -> Result<JobOutcome> {
        let client = Self::client_for(device)?;
        let mirror = self.mirror_for(device);
        let device_id = device.id;
        let selection = device.file_selection;
        let dk = drive_key.to_string();

        // Load the target drive from the index.
        let want = dk.clone();
        let drive = db(self.repo.clone(), move |r| {
            Ok(r.get_drives(device_id)?
                .into_iter()
                .find(|d| d.drive_key == want))
        })
        .await?
        .ok_or_else(|| CoreError::NotFound(format!("drive {drive_key}")))?;

        // Build the selected file list.
        let mut items: Vec<Item> = Vec::new();
        for seg in &drive.segments {
            for f in &seg.files {
                if selection.includes(f.kind) {
                    items.push(Item {
                        route_id: seg.name.route_id.clone(),
                        segment_num: seg.name.segment_num,
                        name: f.name.clone(),
                        rel: file_rel(REALDATA_REL, &seg.name, &f.name),
                        size: f.remote_size,
                    });
                }
            }
        }
        let files_total = items.len() as u32;
        let bytes_total: u64 = items.iter().map(|i| i.size).sum();

        // Open the job row + mark the drive downloading.
        self.start_job(device_id, &dk, files_total, bytes_total)
            .await?;

        // Pre-credit files already complete on disk (fast stats — no spawn_blocking).
        let mut files_done: u32 = 0;
        let mut bytes_done: u64 = 0;
        let mut todo: Vec<Item> = Vec::new();
        for it in items {
            if mirror.is_complete(&it.rel) && mirror.local_size(&it.rel) == Some(it.size) {
                files_done += 1;
                bytes_done += it.size;
            } else {
                todo.push(it);
            }
        }
        self.bump(device_id, &dk, files_done, bytes_done).await?;
        sink.on_progress(DownloadProgress {
            drive_key: dk.clone(),
            files_done,
            files_total,
            bytes_done,
            bytes_total,
            current_file: None,
        });

        for it in todo {
            if cancel.is_cancelled() {
                return self
                    .finish(device_id, drive_key, &sink, Terminal::Canceled)
                    .await;
            }
            let Item {
                route_id,
                segment_num,
                name,
                rel,
                size,
            } = it;
            match download_file(&client, &mirror, &rel, size, &cancel, MAX_ATTEMPTS).await {
                Ok(FileOutcome::Complete) => {
                    files_done += 1;
                    bytes_done += size;
                    let dk2 = dk.clone();
                    let (rt, nm) = (route_id.clone(), name.clone());
                    let (fd, bd) = (files_done, bytes_done);
                    db(self.repo.clone(), move |r| {
                        r.set_file_complete(device_id, &rt, segment_num, &nm, size)?;
                        r.bump_job_progress(device_id, &dk2, fd, bd)
                    })
                    .await?;
                    sink.on_progress(DownloadProgress {
                        drive_key: dk.clone(),
                        files_done,
                        files_total,
                        bytes_done,
                        bytes_total,
                        current_file: Some(name),
                    });
                }
                Ok(FileOutcome::Canceled) => {
                    return self
                        .finish(device_id, drive_key, &sink, Terminal::Canceled)
                        .await;
                }
                Err(e) => {
                    return self
                        .finish(device_id, drive_key, &sink, Terminal::Failed(e.to_string()))
                        .await;
                }
            }
        }

        self.finish(device_id, drive_key, &sink, Terminal::Complete)
            .await
    }

    async fn start_job(
        &self,
        device_id: i64,
        drive_key: &str,
        files_total: u32,
        bytes_total: u64,
    ) -> Result<()> {
        let dk = drive_key.to_string();
        db(self.repo.clone(), move |r| {
            r.upsert_job(device_id, &dk, files_total, bytes_total)?;
            r.set_drive_sync_state(device_id, &dk, SyncStatus::Downloading)
        })
        .await
    }

    async fn bump(
        &self,
        device_id: i64,
        drive_key: &str,
        files_done: u32,
        bytes_done: u64,
    ) -> Result<()> {
        let dk = drive_key.to_string();
        db(self.repo.clone(), move |r| {
            r.bump_job_progress(device_id, &dk, files_done, bytes_done)
        })
        .await
    }

    async fn finish(
        &self,
        device_id: i64,
        drive_key: &str,
        sink: &Arc<dyn ProgressSink>,
        t: Terminal,
    ) -> Result<JobOutcome> {
        let (drive_state, job_state, err, outcome) = match &t {
            Terminal::Complete => (
                SyncStatus::Complete,
                JobState::Complete,
                None,
                JobOutcome::Complete,
            ),
            // Cancellation leaves the drive not-downloaded; M5 reclassifies any
            // partially-downloaded files as resumable from disk.
            Terminal::Canceled => (
                SyncStatus::NotDownloaded,
                JobState::Canceled,
                None,
                JobOutcome::Canceled,
            ),
            Terminal::Failed(e) => (
                SyncStatus::Failed,
                JobState::Failed,
                Some(e.clone()),
                JobOutcome::Failed(e.clone()),
            ),
        };
        let dk = drive_key.to_string();
        let err2 = err.clone();
        db(self.repo.clone(), move |r| {
            r.set_drive_sync_state(device_id, &dk, drive_state)?;
            r.set_job_state(device_id, &dk, job_state, err2.as_deref())
        })
        .await?;
        match &t {
            Terminal::Complete => sink.on_completed(drive_key),
            Terminal::Canceled => {}
            Terminal::Failed(e) => sink.on_failed(drive_key, e),
        }
        Ok(outcome)
    }
}

/// Run a synchronous `Repo` call on the blocking pool (per the db layer's
/// `spawn_blocking` contract), mapping a join failure to a `CoreError`.
async fn db<T, F>(repo: Arc<Repo>, f: F) -> Result<T>
where
    F: FnOnce(&Repo) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(move || f(&repo))
        .await
        .map_err(|e| CoreError::Db(format!("db task join: {e}")))?
}
