//! The sync/download engine: refresh a device's index (`sync_now`) and download
//! a drive's selected files with progress + cancellation (`download_drive` /
//! `start_drive_download`). The Rust core owns the transfer engine; native
//! scheduling (Android Foreground Service / iOS BGTask) drives it in Phase B.

pub mod download_job;
pub mod resume;
pub mod retention;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use tokio_util::sync::CancellationToken;

pub use download_job::{
    download_file, DownloadProgress, FileOutcome, JobOutcome, ProgressSink, MAX_ATTEMPTS,
};

use crate::connectivity::{self, DeviceConnectivity};
use crate::copyparty_client::{CopypartyClient, Credentials};
use crate::db::Repo;
use crate::drive_grouping::group_segments;
use crate::error::{CoreError, Result};
use crate::model::{ConnDot, Device, DownloadState, Drive, FileSelection, JobState, SyncStatus};
use crate::storage::{
    paths::{dir_rel, file_rel},
    MirrorStore,
};

/// URL-relative path under each device's copyparty root where drive segments are
/// served. sunnypilot's manager publishes the on-disk `realdata/` directory at the
/// copyparty URL alias `/routes` — read-only, anonymous (see
/// `ref/sunnypilot/system/manager/process_config.py`) — so against a real device
/// this is `routes/`, not `realdata/`. (TODO: make device-configurable if a device
/// ever serves footage elsewhere.)
pub const REALDATA_REL: &str = "routes/";

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
/// A UniFFI Object (M8) so Swift/Kotlin can hold it and call `cancel()`.
#[derive(uniffi::Object)]
pub struct SyncHandle {
    token: CancellationToken,
}

impl SyncHandle {
    /// Build a handle around an existing token (used by `AppCore`, which spawns
    /// the download on its own runtime and keeps the token to cancel it).
    pub(crate) fn new(token: CancellationToken) -> Self {
        Self { token }
    }
}

#[uniffi::export]
impl SyncHandle {
    pub fn cancel(&self) {
        self.token.cancel();
    }
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

/// Local-footage accounting for the retention UI (storage readout + low-headroom
/// warning). Minutes ≈ segments (each segment is ~1 minute).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RetentionStatus {
    /// Minutes of footage fully present on disk (all selected files complete).
    pub local_minutes: i64,
    /// Of `local_minutes`, how many belong to preserved (starred) drives.
    pub preserved_minutes: i64,
    /// The device's retention budget in minutes; `None` = unlimited.
    pub budget_minutes: Option<i64>,
}

impl SyncEngine {
    pub fn new(repo: Arc<Repo>, mirror_root: impl Into<PathBuf>) -> Self {
        Self {
            repo,
            mirror_root: mirror_root.into(),
        }
    }

    /// The mirror root this engine writes under (per-device subdirs hang off it).
    /// Exposed for `AppCore`'s `export_drive_zip` + `remove_device` cleanup.
    pub fn mirror_root(&self) -> &Path {
        &self.mirror_root
    }

    fn creds_for(device: &Device) -> Credentials {
        match &device.password {
            Some(p) => Credentials::Password(p.clone()),
            None => Credentials::Anonymous,
        }
    }

    /// Resolve a usable client for `device` without a manual mode switch: try the
    /// remembered-good base first, then every candidate IP over **HTTPS**
    /// (preferred) then HTTP, picking the first that responds as copyparty **and**
    /// matches the pinned identity (hostname). Persists the working base and
    /// (re)pins the identity. Real devices use HTTPS (copyparty serves it on the
    /// same port); HTTP is a fallback for non-TLS servers (e.g. the test mock).
    /// A reachable endpoint with a *different* hostname is skipped (it's not this
    /// device); unreachable bases are skipped too.
    async fn resolve(&self, device: &Device) -> Result<CopypartyClient> {
        let creds = Self::creds_for(device);
        let device_id = device.id;

        let mut bases: Vec<String> = Vec::new();
        if let Some(lg) = db(self.repo.clone(), move |r| r.get_last_good_base(device_id)).await? {
            bases.push(lg);
        }
        for ip in device.candidate_ips() {
            bases.push(device.base_for("https", ip));
            bases.push(device.base_for("http", ip));
        }
        bases.dedup();

        let stored = db(self.repo.clone(), move |r| r.get_device_identity(device_id)).await?;

        // A definitive identity mismatch (a reachable but *wrong* device) takes
        // precedence over mere connection failures on other candidates.
        let mut mismatch: Option<CoreError> = None;
        let mut last_err: Option<CoreError> = None;
        for base in bases {
            let client = match CopypartyClient::new(&base, creds.clone()) {
                Ok(c) => c,
                Err(e) => {
                    last_err = Some(e);
                    continue;
                }
            };
            // Liveness: the realdata listing is the same call `sync_now` makes,
            // and works over HTTP or HTTPS (the HTTPS handshake also primes the
            // cert capture).
            if let Err(e) = client.list_dir(REALDATA_REL).await {
                last_err = Some(e);
                continue;
            }
            // Best-effort identity: the copyparty hostname from the HTML root
            // (absent on minimal servers → unpinned) + the captured leaf fp.
            let hostname = client
                .fetch_root_html()
                .await
                .ok()
                .and_then(|h| crate::identity::parse_hostname(&h));
            let observed = crate::identity::DeviceIdentity {
                hostname,
                cert_sha256: client.last_cert_sha256(),
            };
            match crate::identity::decide(stored.as_ref(), &observed) {
                crate::identity::IdentityVerdict::Mismatch { pinned, seen } => {
                    mismatch = Some(CoreError::IdentityMismatch(format!(
                        "expected {pinned}, saw {seen} at {base}"
                    )));
                    continue;
                }
                crate::identity::IdentityVerdict::Ok { repin } => {
                    let base_to_save = base.clone();
                    db(self.repo.clone(), move |r| {
                        if let Some(p) = repin {
                            r.set_device_identity(device_id, &p)?;
                        }
                        r.set_last_good_base(device_id, &base_to_save)
                    })
                    .await?;
                    return Ok(client);
                }
            }
        }
        Err(mismatch.or(last_err).unwrap_or_else(|| {
            CoreError::Unreachable(format!("device {device_id}: no candidate IP"))
        }))
    }

    fn mirror_for(&self, device: &Device) -> MirrorStore {
        MirrorStore::new(self.mirror_root.join(device.id.to_string()))
    }

    /// Refresh the device's index: list segments, persist them, regroup, persist
    /// drives, then reclassify each drive's `sync_state` from disk. Does NOT
    /// download. Returns the device's drives (hydrated, with correct sync state).
    pub async fn sync_now(&self, device: &Device) -> Result<Vec<Drive>> {
        let client = self.resolve(device).await?;
        let segments = client.list_segments(REALDATA_REL).await?;

        let device_id = device.id;
        db(self.repo.clone(), move |r| {
            r.upsert_segments(device_id, &segments)?;
            let drives = group_segments(segments);
            r.replace_drives(device_id, &drives)
        })
        .await?;
        self.reconcile(device_id, device.file_selection).await
    }

    /// Connect to the device (resolving across its IPs and pinning identity) and
    /// return the detected copyparty hostname (e.g. `comma-e0e384a`), if any. The
    /// UI uses this to offer the auto-detected device id for confirmation when a
    /// device is first added. `Ok(None)` means connected but no hostname was
    /// readable; an `Err` means it couldn't be reached/verified.
    pub async fn detect_identity(&self, device: &Device) -> Result<Option<String>> {
        self.resolve(device).await?;
        let id = device.id;
        let pinned = db(self.repo.clone(), move |r| r.get_device_identity(id)).await?;
        Ok(pinned.and_then(|i| i.hostname))
    }

    /// Recompute every drive's `sync_state` from the local mirror and persist it;
    /// recover stale `running` jobs (a fresh process can't have a live download)
    /// to a terminal state. Offline-capable (index + disk, no network). Returns
    /// the re-read drives. Discovering newly-recorded remote segments needs
    /// `sync_now` first.
    pub async fn reconcile_device(&self, device: &Device) -> Result<Vec<Drive>> {
        self.reconcile(device.id, device.file_selection).await
    }

    /// Shared reconcile: classify + persist drive states and recover stale jobs,
    /// all in one blocking hop (the mirror's `std::fs` stats are sync).
    async fn reconcile(&self, device_id: i64, selection: FileSelection) -> Result<Vec<Drive>> {
        let mirror_root = self.mirror_root.clone();
        db(self.repo.clone(), move |r| {
            let mirror = MirrorStore::new(mirror_root.join(device_id.to_string()));
            let drives = r.get_drives(device_id)?;
            for d in &drives {
                let status = resume::drive_status(&mirror, d, &selection, REALDATA_REL);
                r.set_drive_sync_state(device_id, &d.drive_key, status)?;
                // Restart recovery: a job left `running` by a dead process is stale.
                if let Some(job) = r.get_job(device_id, &d.drive_key)? {
                    if job.state == JobState::Running {
                        let (js, err) = if status == SyncStatus::Complete {
                            (JobState::Complete, None)
                        } else {
                            (JobState::Failed, Some(resume::INTERRUPTED))
                        };
                        r.set_job_state(device_id, &d.drive_key, js, err)?;
                    }
                }
            }
            r.get_drives(device_id)
        })
        .await
    }

    /// Prune local **segments** outside the retention window (the newest
    /// `retention_max_minutes` non-preserved segments, plus all preserved drives).
    /// Deletes only local mirror files — the remote is untouched — then reconciles
    /// so drives that lost segments reclassify to `Partial`/`NotDownloaded`. No-op
    /// when no budget is set. Returns the pruned segment ids (`route--seg`, for
    /// logging). Shares [`retention::retention_window`] with the download path, so a
    /// pruned segment is never re-downloaded. Intended Phase-B trigger: after each
    /// sync/download, and on the offline maintenance sweep.
    pub async fn enforce_retention(&self, device: &Device) -> Result<Vec<String>> {
        let device_id = device.id;
        let drives = db(self.repo.clone(), move |r| r.get_drives(device_id)).await?;
        let window = retention::retention_window(&drives, device.retention_max_minutes);

        // Delete out-of-window segments that actually hold local data (FS ops, no DB lock).
        let mirror = self.mirror_for(device);
        let mut pruned = Vec::new();
        for d in &drives {
            if !matches!(d.sync_state, SyncStatus::Complete | SyncStatus::Partial) {
                continue; // NotDownloaded/Downloading/Failed occupy no prunable disk
            }
            for seg in &d.segments {
                if !window.contains(&(seg.name.route_id.clone(), seg.name.segment_num)) {
                    mirror.remove_dir(&dir_rel(REALDATA_REL, &seg.name)).await?;
                    pruned.push(format!("{}--{}", seg.name.route_id, seg.name.segment_num));
                }
            }
        }
        if pruned.is_empty() {
            return Ok(pruned);
        }
        // Reclassify from disk (drives with pruned segments → Partial / NotDownloaded).
        self.reconcile(device_id, device.file_selection).await?;
        Ok(pruned)
    }

    /// Drive keys auto-sync should download: those with ≥1 **in-window** segment whose
    /// selected files aren't all complete on disk. Out-of-window segments are never
    /// included, so a drive entirely beyond the budget is skipped (never downloaded
    /// only to be pruned). Replaces a naive "every NotDownloaded/Partial drive" filter.
    pub async fn pending_download_keys(&self, device: &Device) -> Result<Vec<String>> {
        let device_id = device.id;
        let selection = device.file_selection;
        let drives = db(self.repo.clone(), move |r| r.get_drives(device_id)).await?;
        let window = retention::retention_window(&drives, device.retention_max_minutes);
        let mirror = self.mirror_for(device);
        let mut keys = Vec::new();
        'drive: for d in &drives {
            for seg in &d.segments {
                if !window.contains(&(seg.name.route_id.clone(), seg.name.segment_num)) {
                    continue;
                }
                for f in &seg.files {
                    if !selection.includes(f.kind) {
                        continue;
                    }
                    let rel = file_rel(REALDATA_REL, &seg.name, &f.name);
                    if resume::classify_file(&mirror, &rel, f.remote_size)
                        != DownloadState::Complete
                    {
                        keys.push(d.drive_key.clone());
                        continue 'drive;
                    }
                }
            }
        }
        Ok(keys)
    }

    /// Local-footage accounting for the storage readout + low-headroom warning:
    /// minutes (≈ segments) fully present on disk, how many are preserved, and the
    /// budget. A segment counts as local when every selected file is complete.
    pub async fn retention_status(&self, device: &Device) -> Result<RetentionStatus> {
        let device_id = device.id;
        let selection = device.file_selection;
        let drives = db(self.repo.clone(), move |r| r.get_drives(device_id)).await?;
        let mirror = self.mirror_for(device);
        let (mut local_minutes, mut preserved_minutes) = (0i64, 0i64);
        for d in &drives {
            for seg in &d.segments {
                let (mut any, mut all_complete) = (false, true);
                for f in &seg.files {
                    if !selection.includes(f.kind) {
                        continue;
                    }
                    any = true;
                    let rel = file_rel(REALDATA_REL, &seg.name, &f.name);
                    if resume::classify_file(&mirror, &rel, f.remote_size)
                        != DownloadState::Complete
                    {
                        all_complete = false;
                        break;
                    }
                }
                if any && all_complete {
                    local_minutes += 1;
                    if d.preserved {
                        preserved_minutes += 1;
                    }
                }
            }
        }
        Ok(RetentionStatus {
            local_minutes,
            preserved_minutes,
            budget_minutes: device.retention_max_minutes,
        })
    }

    /// Phase-B maintenance pass: free local space by enforcing the device's
    /// retention budget.
    ///
    /// Remote auto-delete-from-comma was removed: sunnypilot publishes footage on a
    /// **read-only** copyparty volume (`/routes`, no delete permission), so a WebDAV
    /// `DELETE` is rejected (401/403) and cannot prune the device. Reclaiming space
    /// on the comma will return via an SSH-based sync/delete path in a later phase;
    /// the `auto_delete_from_comma` / `auto_delete_min_age_min` settings are retained
    /// (and surfaced in the UI) to drive that future mechanism.
    pub async fn run_maintenance(&self, device: &Device) -> Result<()> {
        self.enforce_retention(device).await?;
        Ok(())
    }

    /// Probe the device's connectivity and resolve its dot (M7): `Red` if the
    /// active `(ip, port)` isn't TCP-reachable; otherwise `Blue` when a download
    /// job is running for it, else `Green`. `Red` short-circuits — when the
    /// device is unreachable there is no point querying the index.
    ///
    /// "Active" means a running *download job*; a brief, untracked `sync_now`
    /// index refresh is intentionally not counted (matches the "Blue while
    /// downloading" contract). A job left `running` by a crashed process reads as
    /// `Blue` until the next `reconcile`/`sync_now` reclaims it (self-healing).
    pub async fn check_connectivity(&self, device: &Device) -> Result<DeviceConnectivity> {
        // Reachable if ANY of the device's IPs (hotspot / home Wi-Fi) answers —
        // the resolver auto-picks among them, so the dot reflects "can I reach
        // this device at all", not one hardcoded mode.
        let mut reachable = false;
        for ip in device.candidate_ips() {
            if connectivity::tcp_reachable(ip, device.port, connectivity::DEFAULT_CONNECT_TIMEOUT)
                .await
            {
                reachable = true;
                break;
            }
        }
        if !reachable {
            return Ok(DeviceConnectivity {
                dot: ConnDot::Red,
                reachable: false,
                downloading: false,
            });
        }
        let device_id = device.id;
        let downloading = db(self.repo.clone(), move |r| r.has_active_job(device_id)).await?;
        let dot = if downloading {
            ConnDot::Blue
        } else {
            ConnDot::Green
        };
        Ok(DeviceConnectivity {
            dot,
            reachable: true,
            downloading,
        })
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
        let client = self.resolve(device).await?;
        let mirror = self.mirror_for(device);
        let device_id = device.id;
        let selection = device.file_selection;
        let dk = drive_key.to_string();

        // Load ALL drives (needed for the retention window) and find the target.
        let all_drives = db(self.repo.clone(), move |r| r.get_drives(device_id)).await?;
        let drive = all_drives
            .iter()
            .find(|d| d.drive_key == dk)
            .cloned()
            .ok_or_else(|| CoreError::NotFound(format!("drive {drive_key}")))?;

        // Build the selected file list — but only for segments **inside the retention
        // window**, so we never download what retention would immediately prune (no
        // prune↔re-download loop). Preserved drives are entirely in-window.
        let window = retention::retention_window(&all_drives, device.retention_max_minutes);
        let mut items: Vec<Item> = Vec::new();
        for seg in &drive.segments {
            if !window.contains(&(seg.name.route_id.clone(), seg.name.segment_num)) {
                continue;
            }
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
            if resume::classify_file(&mirror, &it.rel, it.size) == DownloadState::Complete {
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
            Terminal::Complete => sink.on_completed(drive_key.to_string()),
            Terminal::Canceled => {}
            Terminal::Failed(e) => sink.on_failed(drive_key.to_string(), e.clone()),
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
