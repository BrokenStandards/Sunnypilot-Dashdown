//! The native-facing UniFFI surface (M8): the `AppCore` facade object that wraps
//! the sync engine + index behind id-based, `async`-throwing methods, plus the
//! `SyncHandle` object and the boundary `DriveSyncStatus` record. SwiftUI /
//! Jetpack Compose (Phase B) consume the generated Swift + Kotlin bindings.
//!
//! Runtime model (uniffi 0.31): exported `async` methods are driven by the
//! foreign poll loop wrapped in `async_compat::Compat` — there is no persistent
//! uniffi runtime, so a detached `tokio::spawn` from inside a method would be
//! orphaned. `AppCore` therefore owns one long-lived multi-thread runtime and
//! launches the background drive download on it via `Handle::spawn`. No
//! `block_on` is ever called from inside an exported method.

pub mod callbacks;

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::connectivity::DeviceConnectivity;
use crate::db::Repo;
use crate::error::{CoreError, Result};
use crate::logging::{LogLevel, LogSink};
use crate::model::{ConnMode, Device, Drive, FileKind, JobState, SyncStatus};
use crate::settings::DeviceSettings;
use crate::storage::{paths::file_rel, MirrorStore};
use crate::sync_engine::{
    CancellationToken, DownloadProgress, ProgressSink, RetentionStatus, SyncEngine, SyncHandle,
    REALDATA_REL,
};

/// A pollable snapshot of a drive's download status (drive state + the latest
/// job-progress counters). The live progress stream is `ProgressSink`; this is
/// the on-demand accessor (`AppCore::get_drive_status`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DriveSyncStatus {
    pub drive_key: String,
    pub status: SyncStatus,
    pub files_done: u32,
    pub files_total: u32,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub error: Option<String>,
}

/// One mirrored segment file's absolute on-disk location, returned by
/// `AppCore::drive_local_paths` (ordered by `segment_num`). The native players
/// build a per-camera timeline from these instead of hand-constructing paths.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SegmentPath {
    pub segment_num: u32,
    pub path: String,
}

/// The root object the native layer constructs once and holds for the app's
/// lifetime.
#[derive(uniffi::Object)]
pub struct AppCore {
    engine: SyncEngine,
    repo: Arc<Repo>,
    /// Owned multi-thread runtime — its sole job is to drive the detached drive
    /// download so it outlives the `start_drive_download` call. Kept in an
    /// `Option` so `Drop` can shut it down *non-blocking* (`shutdown_background`):
    /// dropping a `Runtime` the normal way blocks, which panics if the drop
    /// happens inside another async context (e.g. a foreign async callback).
    runtime: Option<tokio::runtime::Runtime>,
    /// Spawn target into `runtime` (always valid for the object's lifetime).
    handle: tokio::runtime::Handle,
    progress_sink: RwLock<Option<Arc<dyn ProgressSink>>>,
}

impl Drop for AppCore {
    fn drop(&mut self) {
        // Non-blocking shutdown (a blocking `Runtime` drop panics if it happens
        // inside another async context, e.g. a foreign async callback). NOTE:
        // this ABANDONS any in-flight drive download with no terminal callback —
        // per the background contract the native layer owns task lifecycle and
        // must cancel + drain downloads before releasing the last `AppCore` ref.
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
    }
}

// ---- sync surface: constructor + sink setters -------------------------------

#[uniffi::export]
impl AppCore {
    /// Open the index at `db_path`, root the mirror at `mirror_root`, build the
    /// owned runtime, and install the logging bridge.
    #[uniffi::constructor]
    pub fn new(db_path: String, mirror_root: String) -> Result<Arc<Self>> {
        let repo = Arc::new(Repo::open(Path::new(&db_path))?);
        let engine = SyncEngine::new(repo.clone(), PathBuf::from(&mirror_root));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| CoreError::Io(format!("runtime build: {e}")))?;
        let handle = runtime.handle().clone();
        crate::logging::install();
        // Install the ring crypto provider before any reqwest client is built
        // (rustls is compiled with `rustls-no-provider`). Idempotent.
        crate::tls::ensure_crypto_provider();
        Ok(Arc::new(AppCore {
            engine,
            repo,
            runtime: Some(runtime),
            handle,
            progress_sink: RwLock::new(None),
        }))
    }

    /// Set (or clear) the progress sink used by `start_drive_download`.
    pub fn set_progress_sink(&self, sink: Option<Arc<dyn ProgressSink>>) {
        *self
            .progress_sink
            .write()
            .unwrap_or_else(|e| e.into_inner()) = sink;
    }

    /// Set (or clear) the log sink and verbosity threshold.
    pub fn set_log_sink(&self, sink: Option<Arc<dyn LogSink>>, level: LogLevel) {
        crate::logging::set_sink(sink, level);
    }

    /// Remux one HD camera segment's raw HEVC to MP4 **in memory**, returning the
    /// bytes — no file is written (cf. the disk-caching `ensure_playable`). The
    /// player feeds these bytes to ExoPlayer through a custom in-memory data source
    /// and remuxes lazily, segment-by-segment, as playback/seeks reach each window.
    /// `Ok(None)` for a non-HD `kind` or a segment that isn't completely mirrored.
    ///
    /// **Synchronous by design**: the Android player calls this from ExoPlayer's
    /// background loader thread, which must block on the (CPU/IO-bound) remux. Never
    /// call it from an async executor thread.
    pub fn remux_hd_bytes(
        &self,
        device_id: i64,
        drive_key: String,
        segment_num: u32,
        kind: FileKind,
    ) -> Result<Option<Vec<u8>>> {
        // Only the raw HEVC cameras need remuxing; qcamera/others are never routed here.
        if !matches!(
            kind,
            FileKind::FCamera | FileKind::ECamera | FileKind::DCamera
        ) {
            return Ok(None);
        }
        let Some(src) = self.resolve_local_path_sync(device_id, &drive_key, segment_num, kind)?
        else {
            return Ok(None);
        };
        Ok(Some(crate::video::remux_hevc_to_mp4_bytes(&src)?))
    }
}

// ---- async data surface -----------------------------------------------------

#[uniffi::export(async_runtime = "tokio")]
impl AppCore {
    pub async fn list_devices(&self) -> Result<Vec<Device>> {
        self.db(|r| r.list_devices()).await
    }

    pub async fn add_device(&self, device: Device) -> Result<Device> {
        let to_insert = device.clone();
        let id = self.db(move |r| r.insert_device(&to_insert)).await?;
        let mut device = device;
        device.id = id;
        Ok(device)
    }

    pub async fn update_device(&self, device: Device) -> Result<()> {
        self.db(move |r| r.update_device(&device)).await
    }

    pub async fn remove_device(&self, device_id: i64) -> Result<()> {
        self.db(move |r| r.delete_device(device_id)).await?;
        // Best-effort local mirror cleanup (index rows cascade in the DB). A
        // missing dir is fine; surface any other failure as a warning so a leak
        // is observable rather than silent.
        let dir = self.engine.mirror_root().join(device_id.to_string());
        if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(dir = %dir.display(), error = %e, "mirror cleanup failed after remove_device");
            }
        }
        Ok(())
    }

    pub async fn set_active_mode(&self, device_id: i64, mode: ConnMode) -> Result<()> {
        let mut dev = self.load_device(device_id).await?;
        dev.active_mode = mode;
        self.db(move |r| r.update_device(&dev)).await
    }

    pub async fn get_settings(&self, device_id: i64) -> Result<DeviceSettings> {
        Ok(self.load_device(device_id).await?.settings())
    }

    pub async fn set_settings(&self, device_id: i64, settings: DeviceSettings) -> Result<()> {
        let mut dev = self.load_device(device_id).await?;
        dev.apply_settings(settings);
        self.db(move |r| r.update_device(&dev)).await
    }

    /// List a device's drives. `offline=false` refreshes the index from the
    /// device (network); `offline=true` reclassifies from the local mirror with
    /// no network (correct `sync_state`, unlike a raw local scan).
    pub async fn list_drives(&self, device_id: i64, offline: bool) -> Result<Vec<Drive>> {
        let dev = self.load_device(device_id).await?;
        if offline {
            self.engine.reconcile_device(&dev).await
        } else {
            self.engine.sync_now(&dev).await
        }
    }

    pub async fn get_drive(&self, device_id: i64, drive_key: String) -> Result<Drive> {
        let key = drive_key.clone();
        self.db(move |r| r.get_drive(device_id, &drive_key))
            .await?
            .ok_or_else(|| CoreError::NotFound(format!("drive {key}")))
    }

    pub async fn get_drive_status(
        &self,
        device_id: i64,
        drive_key: String,
    ) -> Result<DriveSyncStatus> {
        let key = drive_key.clone();
        let (drive, job) = self
            .db(move |r| {
                let drive = r.get_drive(device_id, &drive_key)?;
                let job = r.get_job(device_id, &drive_key)?;
                Ok((drive, job))
            })
            .await?;
        let drive = drive.ok_or_else(|| CoreError::NotFound(format!("drive {key}")))?;
        Ok(DriveSyncStatus {
            drive_key: key,
            status: drive.sync_state,
            files_done: job.as_ref().map(|j| j.files_done).unwrap_or(0),
            files_total: job.as_ref().map(|j| j.files_total).unwrap_or(0),
            bytes_done: job.as_ref().map(|j| j.bytes_done).unwrap_or(0),
            bytes_total: job.as_ref().map(|j| j.bytes_total).unwrap_or(0),
            // Only surface a job error for a genuinely failed job — a stale error
            // from a prior attempt must not leak onto a since-Complete/Partial drive.
            error: job
                .filter(|j| j.state == JobState::Failed)
                .and_then(|j| j.error),
        })
    }

    pub async fn sync_now(&self, device_id: i64) -> Result<Vec<Drive>> {
        let dev = self.load_device(device_id).await?;
        self.engine.sync_now(&dev).await
    }

    /// Connect to the device and return the detected copyparty hostname
    /// (e.g. `comma-e0e384a`) for the UI to offer as the device's dongle id on
    /// first add. `Ok(None)` = connected but no hostname; `Err` = unreachable.
    pub async fn detect_device_name(&self, device_id: i64) -> Result<Option<String>> {
        let dev = self.load_device(device_id).await?;
        self.engine.detect_identity(&dev).await
    }

    pub async fn set_preserved(
        &self,
        device_id: i64,
        drive_key: String,
        preserved: bool,
    ) -> Result<()> {
        self.db(move |r| r.set_drive_preserved(device_id, &drive_key, preserved))
            .await
    }

    /// Start downloading a drive's selected files on the owned runtime; returns a
    /// handle to cancel it. Progress is delivered to the configured progress sink.
    ///
    /// INVARIANT: the detached task MUST be launched via `self.handle.spawn`
    /// (the owned runtime), never bare `tokio::spawn`. uniffi 0.31 drives each
    /// exported future on a transient `async_compat` context that does not
    /// outlive the call, so a bare-spawned task would be orphaned.
    pub async fn start_drive_download(
        &self,
        device_id: i64,
        drive_key: String,
    ) -> Result<Arc<SyncHandle>> {
        let dev = self.load_device(device_id).await?;
        // Clone the Arc out and drop the guard before spawning (never hold a std
        // lock across an await/spawn).
        let sink: Arc<dyn ProgressSink> = self
            .progress_sink
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| Arc::new(NoopSink));
        let token = CancellationToken::new();
        let child = token.clone();
        let engine = self.engine.clone();
        self.handle.spawn(async move {
            let _ = engine.download_drive(&dev, &drive_key, sink, child).await;
        });
        Ok(Arc::new(SyncHandle::new(token)))
    }

    /// Zip the drive's already-mirrored (selected) files to `dest_path`.
    pub async fn export_drive_zip(
        &self,
        device_id: i64,
        drive_key: String,
        dest_path: String,
    ) -> Result<()> {
        let device = self.load_device(device_id).await?;
        let repo = self.repo.clone();
        let mirror_root = self.engine.mirror_root().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let drive = repo
                .get_drive(device_id, &drive_key)?
                .ok_or_else(|| CoreError::NotFound(format!("drive {drive_key}")))?;
            let mirror = MirrorStore::new(mirror_root.join(device_id.to_string()));
            write_drive_zip(&device, &drive, &mirror, Path::new(&dest_path))
        })
        .await
        .map_err(|e| CoreError::Io(format!("zip task join: {e}")))?
    }

    /// Absolute on-disk path of one downloaded segment file, or `None` if that
    /// file isn't completely mirrored. The single source of truth for local paths
    /// — native layers must not hand-build mirror paths (the footage base lives at
    /// [`REALDATA_REL`]). `kind` selects the stream; the concrete filename comes
    /// from the drive index, so `.zst`/`.bz2` log variants resolve correctly.
    pub async fn local_file_path(
        &self,
        device_id: i64,
        drive_key: String,
        segment_num: u32,
        kind: FileKind,
    ) -> Result<Option<String>> {
        let paths = self
            .resolve_local_paths(device_id, drive_key, kind, Some(segment_num))
            .await?;
        Ok(paths.into_iter().next().map(|sp| sp.path))
    }

    /// Every completely-mirrored file of `kind` in the drive, ordered by
    /// `segment_num` — the input for a continuous per-camera playback timeline.
    pub async fn drive_local_paths(
        &self,
        device_id: i64,
        drive_key: String,
        kind: FileKind,
    ) -> Result<Vec<SegmentPath>> {
        self.resolve_local_paths(device_id, drive_key, kind, None)
            .await
    }

    /// Return a path the native player can open for this segment's `kind` stream,
    /// remuxing on demand if needed. `qcamera.ts` (already playable) and any
    /// non-video kind return their source path unchanged; the HD HEVC cameras
    /// (`f`/`e`/`d`camera) return a cached `*.hevc.mp4` derived once via a lossless
    /// HEVC→MP4 remux (see [`crate::video`]). `None` if the source isn't mirrored.
    ///
    /// The remux is CPU/IO-bound (reads the whole `.hevc`) so it runs on the
    /// blocking pool; a second call for the same file reuses the cached MP4.
    pub async fn ensure_playable(
        &self,
        device_id: i64,
        drive_key: String,
        segment_num: u32,
        kind: FileKind,
    ) -> Result<Option<String>> {
        let Some(src) = self
            .local_file_path(device_id, drive_key, segment_num, kind)
            .await?
        else {
            return Ok(None);
        };
        // Only the raw HEVC cameras need remuxing; everything else plays as-is.
        if !matches!(
            kind,
            FileKind::FCamera | FileKind::ECamera | FileKind::DCamera
        ) {
            return Ok(Some(src));
        }
        let src_path = PathBuf::from(src);
        let dst = tokio::task::spawn_blocking(move || crate::video::ensure_playable_mp4(&src_path))
            .await
            .map_err(|e| CoreError::Io(format!("remux task join: {e}")))??;
        Ok(Some(dst.to_string_lossy().into_owned()))
    }

    pub async fn run_maintenance(&self, device_id: i64) -> Result<()> {
        let dev = self.load_device(device_id).await?;
        self.engine.run_maintenance(&dev).await
    }

    /// Drive keys auto-sync should download (in the retention window + not yet
    /// complete on disk). The background scheduler iterates these instead of every
    /// not-downloaded drive, so footage beyond the budget is never fetched.
    pub async fn pending_download_keys(&self, device_id: i64) -> Result<Vec<String>> {
        let dev = self.load_device(device_id).await?;
        self.engine.pending_download_keys(&dev).await
    }

    /// Local-footage accounting for the storage readout + low-headroom warning.
    pub async fn retention_status(&self, device_id: i64) -> Result<RetentionStatus> {
        let dev = self.load_device(device_id).await?;
        self.engine.retention_status(&dev).await
    }

    pub async fn check_connectivity(&self, device_id: i64) -> Result<DeviceConnectivity> {
        let dev = self.load_device(device_id).await?;
        self.engine.check_connectivity(&dev).await
    }
}

// ---- private helpers (not exported) ----------------------------------------

impl AppCore {
    /// Run a synchronous `Repo` call on the blocking pool (mirrors the engine's
    /// `db()` contract).
    async fn db<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Repo) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let repo = self.repo.clone();
        tokio::task::spawn_blocking(move || f(&repo))
            .await
            .map_err(|e| CoreError::Db(format!("db task join: {e}")))?
    }

    async fn load_device(&self, id: i64) -> Result<Device> {
        self.db(move |r| r.get_device(id))
            .await?
            .ok_or_else(|| CoreError::NotFound(format!("device {id}")))
    }

    /// Shared resolver for [`local_file_path`]/[`drive_local_paths`]: load the
    /// drive, then for each segment (optionally only `only_segment`) emit the
    /// absolute path of its `kind` file when that file is fully mirrored. Runs the
    /// DB read + filesystem stats on the blocking pool.
    /// Synchronous single-segment counterpart of [`resolve_local_paths`]: the
    /// absolute path of the `kind` file in `segment_num` of the drive, when that
    /// file is fully mirrored (else `None`). Used by the synchronous
    /// `remux_hd_bytes` (called on the player's loader thread, so no blocking pool).
    fn resolve_local_path_sync(
        &self,
        device_id: i64,
        drive_key: &str,
        segment_num: u32,
        kind: FileKind,
    ) -> Result<Option<PathBuf>> {
        let drive = self
            .repo
            .get_drive(device_id, drive_key)?
            .ok_or_else(|| CoreError::NotFound(format!("drive {drive_key}")))?;
        let mirror = MirrorStore::new(self.engine.mirror_root().join(device_id.to_string()));
        let Some(seg) = drive
            .segments
            .iter()
            .find(|s| s.name.segment_num == segment_num)
        else {
            return Ok(None);
        };
        let Some(file) = seg.files.iter().find(|f| f.kind == kind) else {
            return Ok(None);
        };
        let rel = file_rel(REALDATA_REL, &seg.name, &file.name);
        if !mirror.is_complete(&rel) {
            return Ok(None);
        }
        Ok(Some(mirror.final_path(&rel)?))
    }

    async fn resolve_local_paths(
        &self,
        device_id: i64,
        drive_key: String,
        kind: FileKind,
        only_segment: Option<u32>,
    ) -> Result<Vec<SegmentPath>> {
        let repo = self.repo.clone();
        let mirror_root = self.engine.mirror_root().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let drive = repo
                .get_drive(device_id, &drive_key)?
                .ok_or_else(|| CoreError::NotFound(format!("drive {drive_key}")))?;
            let mirror = MirrorStore::new(mirror_root.join(device_id.to_string()));
            let mut out = Vec::new();
            for seg in &drive.segments {
                if only_segment.is_some_and(|n| n != seg.name.segment_num) {
                    continue;
                }
                let Some(file) = seg.files.iter().find(|f| f.kind == kind) else {
                    continue;
                };
                let rel = file_rel(REALDATA_REL, &seg.name, &file.name);
                if mirror.is_complete(&rel) {
                    out.push(SegmentPath {
                        segment_num: seg.name.segment_num,
                        path: mirror.final_path(&rel)?.to_string_lossy().into_owned(),
                    });
                }
            }
            Ok(out)
        })
        .await
        .map_err(|e| CoreError::Io(format!("path task join: {e}")))?
    }
}

/// A progress sink that drops everything — used when none is configured.
struct NoopSink;
impl ProgressSink for NoopSink {
    fn on_progress(&self, _p: DownloadProgress) {}
    fn on_completed(&self, _drive_key: String) {}
    fn on_failed(&self, _drive_key: String, _error: String) {}
}

/// Write the drive's selected, locally-complete files into a zip at `dest`.
/// Entry names are drive-relative (`<segment dir>/<file>`), forward-slashed.
fn write_drive_zip(
    device: &Device,
    drive: &Drive,
    mirror: &MirrorStore,
    dest: &Path,
) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::create(dest)?;
    let mut zw = zip::write::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    for seg in &drive.segments {
        for f in &seg.files {
            if !device.file_selection.includes(f.kind) {
                continue;
            }
            let rel = file_rel(REALDATA_REL, &seg.name, &f.name);
            if !mirror.is_complete(&rel) {
                continue;
            }
            let src = mirror.final_path(&rel)?;
            let entry = format!("{}/{}", seg.name.dir_name(), f.name);
            zw.start_file(entry, opts)
                .map_err(|e| CoreError::Io(format!("zip start_file: {e}")))?;
            let bytes = std::fs::read(&src)?;
            zw.write_all(&bytes)?;
        }
    }
    zw.finish()
        .map_err(|e| CoreError::Io(format!("zip finish: {e}")))?;
    Ok(())
}
