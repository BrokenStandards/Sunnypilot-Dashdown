//! Integration: M5 partial/resume — byte-range resume (206 append / 200 restart),
//! drive reclassification, resume-only-missing, size-mismatch re-fetch,
//! later-contiguous → Partial, and restart recovery.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use dashdown_core::copyparty_client::{CopypartyClient, Credentials};
use dashdown_core::db::Repo;
use dashdown_core::model::{ConnMode, Device, DownloadState, FileSelection, JobState, SyncStatus};
use dashdown_core::storage::MirrorStore;
use dashdown_core::sync_engine::{
    download_file, resume, CancellationToken, DownloadProgress, ProgressSink, SyncEngine,
};
use mock_copyparty::{fixtures, MockServer};
use tokio::io::AsyncWriteExt;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer as WireServer, ResponseTemplate};

const ROUTE: &str = "000001a3--c20ba54385";

fn rel(route: &str, n: u32, name: &str) -> String {
    format!("realdata/{route}--{n}/{name}")
}

fn device_at(addr: SocketAddr, selection: FileSelection) -> Device {
    Device {
        id: 0,
        name: "dev".into(),
        dongle_label: None,
        hotspot_ip: addr.ip().to_string(),
        wifi_ip: None,
        port: addr.port(),
        active_mode: ConnMode::Hotspot,
        password: None,
        auto_sync: false,
        file_selection: selection,
        retention_max_minutes: None,
        auto_delete_from_comma: false,
        auto_delete_min_age_min: 60,
    }
}

#[derive(Default)]
struct Recorder {
    progress: Mutex<Vec<DownloadProgress>>,
}
impl ProgressSink for Recorder {
    fn on_progress(&self, p: DownloadProgress) {
        self.progress.lock().unwrap().push(p);
    }
    fn on_completed(&self, _drive_key: String) {}
    fn on_failed(&self, _drive_key: String, _error: String) {}
}
impl Recorder {
    /// Names of files actually fetched (progress events with `current_file=Some`).
    fn fetched(&self) -> BTreeSet<String> {
        self.progress
            .lock()
            .unwrap()
            .iter()
            .filter_map(|p| p.current_file.clone())
            .collect()
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    repo: Arc<Repo>,
    engine: SyncEngine,
    mirror_root: std::path::PathBuf,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let repo = Arc::new(Repo::open(&dir.path().join("index.db")).unwrap());
    let mirror_root = dir.path().join("mirror");
    let engine = SyncEngine::new(repo.clone(), mirror_root.clone());
    Setup {
        _dir: dir,
        repo,
        engine,
        mirror_root,
    }
}

impl Setup {
    fn mirror_for(&self, dev: &Device) -> MirrorStore {
        MirrorStore::new(self.mirror_root.join(dev.id.to_string()))
    }
}

/// `sync_now` + full `download_drive` of the `single_drive` fixture.
/// Returns everything kept alive plus the drive key and the served realdata dir.
async fn synced_and_downloaded(
    sel: FileSelection,
) -> (Setup, MockServer, Device, String, std::path::PathBuf) {
    let fixture = fixtures::single_drive();
    let realdata = fixture.path().join("realdata"); // capture before spawn consumes it
    let srv = MockServer::spawn(fixture, None).await.unwrap();
    let s = setup();
    let mut dev = device_at(srv.addr(), sel);
    dev.id = s.repo.insert_device(&dev).unwrap();
    let drives = s.engine.sync_now(&dev).await.unwrap();
    let dk = drives[0].drive_key.clone();
    s.engine
        .download_drive(
            &dev,
            &dk,
            Arc::new(Recorder::default()),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    (s, srv, dev, dk, realdata)
}

/// Write a partial `.part` to disk (flush before drop — tokio's File buffers,
/// so a bare drop would discard the bytes; the engine's `commit` always flushes).
async fn place_part(mirror: &MirrorStore, rel: &str, len: usize) {
    let mut pf = mirror.create_part(rel).await.unwrap();
    pf.writer().write_all(&vec![1u8; len]).await.unwrap();
    pf.writer().flush().await.unwrap();
    drop(pf);
}

fn drive_state(repo: &Repo, dev_id: i64, dk: &str) -> SyncStatus {
    repo.get_drives(dev_id)
        .unwrap()
        .into_iter()
        .find(|d| d.drive_key == dk)
        .unwrap()
        .sync_state
}

// ---- byte-range download_file (wiremock) ------------------------------------

#[tokio::test]
async fn resumes_partial_part_via_range() {
    let server = WireServer::start().await;
    // Only a `bytes=4-` ranged request is served (proves Range was sent), 206 + tail.
    Mock::given(method("GET"))
        .and(path("/f.bin"))
        .and(header("range", "bytes=4-"))
        .respond_with(ResponseTemplate::new(206).set_body_bytes(vec![2u8; 6]))
        .mount(&server)
        .await;

    let client = CopypartyClient::new(&server.uri(), Credentials::Anonymous).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mirror = MirrorStore::new(dir.path());

    // Pre-place a 4-byte `.part` (an interrupted download).
    place_part(&mirror, "f.bin", 4).await;
    assert_eq!(mirror.part_size("f.bin"), Some(4));

    let token = CancellationToken::new();
    let outcome = download_file(&client, &mirror, "f.bin", 10, &token, 1)
        .await
        .unwrap();
    assert_eq!(outcome, dashdown_core::sync_engine::FileOutcome::Complete);
    assert_eq!(mirror.local_size("f.bin"), Some(10), "4 kept + 6 appended");
}

#[tokio::test]
async fn restarts_when_server_ignores_range() {
    let server = WireServer::start().await;
    // Server ignores Range and returns the whole body as 200.
    Mock::given(method("GET"))
        .and(path("/f.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![9u8; 10]))
        .mount(&server)
        .await;

    let client = CopypartyClient::new(&server.uri(), Credentials::Anonymous).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mirror = MirrorStore::new(dir.path());

    place_part(&mirror, "f.bin", 4).await;

    let token = CancellationToken::new();
    let outcome = download_file(&client, &mirror, "f.bin", 10, &token, 1)
        .await
        .unwrap();
    assert_eq!(outcome, dashdown_core::sync_engine::FileOutcome::Complete);
    assert_eq!(
        mirror.local_size("f.bin"),
        Some(10),
        "stale .part discarded"
    );
}

// ---- drive-level resume (mock-copyparty) ------------------------------------

#[tokio::test]
async fn resume_downloads_only_missing_files() {
    let (s, _srv, dev, dk, _rd) = synced_and_downloaded(FileSelection::everything()).await;
    assert_eq!(drive_state(&s.repo, dev.id, &dk), SyncStatus::Complete);

    // Simulate two missing files (distinct kinds, same segment) and reclassify.
    let mirror = s.mirror_for(&dev);
    for name in ["qcamera.ts", "rlog.zst"] {
        std::fs::remove_file(mirror.final_path(&rel(ROUTE, 0, name)).unwrap()).unwrap();
    }
    s.engine.reconcile_device(&dev).await.unwrap();
    assert_eq!(drive_state(&s.repo, dev.id, &dk), SyncStatus::Partial);

    let rec = Arc::new(Recorder::default());
    s.engine
        .download_drive(&dev, &dk, rec.clone(), CancellationToken::new())
        .await
        .unwrap();

    // Exactly the two deleted files were re-fetched; the other 13 were skipped.
    assert_eq!(
        rec.fetched(),
        BTreeSet::from(["qcamera.ts".to_string(), "rlog.zst".to_string()])
    );
    let first = &rec.progress.lock().unwrap()[0];
    assert_eq!(first.current_file, None);
    assert_eq!(first.files_done, 13, "13 of 15 pre-credited");
    // All files present again; drive Complete.
    assert!(mirror.is_complete(&rel(ROUTE, 0, "qcamera.ts")));
    assert_eq!(drive_state(&s.repo, dev.id, &dk), SyncStatus::Complete);
}

#[tokio::test]
async fn size_mismatch_is_refetched() {
    let (s, _srv, dev, dk, _rd) = synced_and_downloaded(FileSelection::everything()).await;
    let mirror = s.mirror_for(&dev);
    let qrel = rel(ROUTE, 0, "qcamera.ts");

    // Corrupt one file to the wrong size.
    std::fs::write(mirror.final_path(&qrel).unwrap(), b"oops").unwrap();
    assert_eq!(classify(&mirror, &qrel, 1200), DownloadState::SizeMismatch);
    s.engine.reconcile_device(&dev).await.unwrap();
    assert_eq!(drive_state(&s.repo, dev.id, &dk), SyncStatus::Partial);

    s.engine
        .download_drive(
            &dev,
            &dk,
            Arc::new(Recorder::default()),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        mirror.local_size(&qrel),
        Some(1200),
        "corrected to remote size"
    );
    assert_eq!(drive_state(&s.repo, dev.id, &dk), SyncStatus::Complete);
}

#[tokio::test]
async fn later_contiguous_segment_makes_drive_partial() {
    let (s, _srv, dev, dk, realdata) = synced_and_downloaded(FileSelection::everything()).await;
    assert_eq!(drive_state(&s.repo, dev.id, &dk), SyncStatus::Complete);

    // The drive grows on the device: add a 4th contiguous segment on disk.
    let seg3 = realdata.join(format!("{ROUTE}--3"));
    std::fs::create_dir_all(&seg3).unwrap();
    for (name, len) in [
        ("qcamera.ts", 1200usize),
        ("rlog.zst", 300),
        ("qlog.zst", 100),
        ("fcamera.hevc", 7600),
        ("ecamera.hevc", 7600),
    ] {
        std::fs::write(seg3.join(name), vec![0u8; len]).unwrap();
    }

    // Re-sync: the drive now spans 0..3 (same key), and is Partial (seg 3 missing).
    let drives = s.engine.sync_now(&dev).await.unwrap();
    let grown = drives.iter().find(|d| d.drive_key == dk).unwrap();
    assert_eq!(grown.segment_count, 4, "drive grew, key unchanged");
    assert_eq!(grown.sync_state, SyncStatus::Partial);

    // Downloading fetches only segment 3's files → Complete.
    let rec = Arc::new(Recorder::default());
    s.engine
        .download_drive(&dev, &dk, rec.clone(), CancellationToken::new())
        .await
        .unwrap();
    let mirror = s.mirror_for(&dev);
    assert!(mirror.is_complete(&rel(ROUTE, 3, "qcamera.ts")));
    assert_eq!(drive_state(&s.repo, dev.id, &dk), SyncStatus::Complete);
}

#[tokio::test]
async fn restart_recovery_resets_stale_job_and_resumes() {
    let (s, _srv, dev, dk, _rd) = synced_and_downloaded(FileSelection::everything()).await;
    let mirror = s.mirror_for(&dev);

    // Simulate a crash mid-download: a missing file + a stale Downloading/running state.
    std::fs::remove_file(mirror.final_path(&rel(ROUTE, 0, "fcamera.hevc")).unwrap()).unwrap();
    s.repo
        .set_drive_sync_state(dev.id, &dk, SyncStatus::Downloading)
        .unwrap();
    s.repo.upsert_job(dev.id, &dk, 15, 1).unwrap(); // state=running

    // Recovery reclassifies the drive and moves the stale job off `running`.
    s.engine.reconcile_device(&dev).await.unwrap();
    assert_eq!(drive_state(&s.repo, dev.id, &dk), SyncStatus::Partial);
    let job = s.repo.get_job(dev.id, &dk).unwrap().unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(job.error.as_deref(), Some("interrupted"));

    // Resume fetches only the missing file.
    let rec = Arc::new(Recorder::default());
    s.engine
        .download_drive(&dev, &dk, rec.clone(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(rec.fetched(), BTreeSet::from(["fcamera.hevc".to_string()]));
    assert_eq!(drive_state(&s.repo, dev.id, &dk), SyncStatus::Complete);
}

/// Thin wrapper so the test reads cleanly.
fn classify(mirror: &MirrorStore, rel: &str, remote_size: u64) -> DownloadState {
    resume::classify_file(mirror, rel, remote_size)
}
