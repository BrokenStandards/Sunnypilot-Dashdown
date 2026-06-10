//! Integration: the sync/download engine — `download_file` retry/cancel,
//! `sync_now` index refresh, full + previews-only drive downloads, and engine
//! cancellation. mock-copyparty for happy paths; wiremock for truncation/delay.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dashdown_core::copyparty_client::{CopypartyClient, Credentials};
use dashdown_core::db::Repo;
use dashdown_core::model::{ConnMode, Device, FileSelection, JobState, SyncStatus};
use dashdown_core::storage::MirrorStore;
use dashdown_core::sync_engine::{
    download_file, CancellationToken, DownloadProgress, FileOutcome, JobOutcome, ProgressSink,
    SyncEngine,
};
use mock_copyparty::{fixtures, MockServer};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer as WireServer, ResponseTemplate};

const ROUTE: &str = "000001a3--c20ba54385";

fn rel(route: &str, n: u32, name: &str) -> String {
    format!("routes/{route}--{n}/{name}")
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

/// A `ProgressSink` that records every callback for assertions.
#[derive(Default)]
struct Recorder {
    progress: Mutex<Vec<DownloadProgress>>,
    completed: Mutex<Vec<String>>,
    failed: Mutex<Vec<(String, String)>>,
}
impl ProgressSink for Recorder {
    fn on_progress(&self, p: DownloadProgress) {
        self.progress.lock().unwrap().push(p);
    }
    fn on_completed(&self, drive_key: String) {
        self.completed.lock().unwrap().push(drive_key);
    }
    fn on_failed(&self, drive_key: String, error: String) {
        self.failed.lock().unwrap().push((drive_key, error));
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

// ---- download_file (wiremock) -----------------------------------------------

#[tokio::test]
async fn download_file_refetches_on_truncated_body() {
    let server = WireServer::start().await;
    // First request: a short (truncated) body; subsequent: the full body.
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8; 3]))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8; 10]))
        .with_priority(2)
        .mount(&server)
        .await;

    let client = CopypartyClient::new(&server.uri(), Credentials::Anonymous).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mirror = MirrorStore::new(dir.path());
    let token = CancellationToken::new();

    let outcome = download_file(&client, &mirror, "file.bin", 10, &token, 2)
        .await
        .unwrap();
    assert_eq!(outcome, FileOutcome::Complete);
    assert_eq!(mirror.local_size("file.bin"), Some(10));
    assert!(!mirror.part_path("file.bin").unwrap().exists());
}

#[tokio::test]
async fn download_file_fails_when_always_truncated() {
    let server = WireServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8; 3]))
        .mount(&server)
        .await;

    let client = CopypartyClient::new(&server.uri(), Credentials::Anonymous).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mirror = MirrorStore::new(dir.path());
    let token = CancellationToken::new();

    let res = download_file(&client, &mirror, "file.bin", 10, &token, 2).await;
    assert!(res.is_err(), "exhausted size mismatch should error");
    assert!(!mirror.is_complete("file.bin"), "no committed final");
}

#[tokio::test]
async fn download_file_does_not_retry_on_404() {
    let server = WireServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1) // verified on drop: a 404 must NOT be retried
        .mount(&server)
        .await;

    let client = CopypartyClient::new(&server.uri(), Credentials::Anonymous).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mirror = MirrorStore::new(dir.path());
    let token = CancellationToken::new();

    let res = download_file(&client, &mirror, "missing.bin", 10, &token, 3).await;
    assert!(res.is_err());
    assert!(!mirror.is_complete("missing.bin"));
}

#[tokio::test]
async fn download_file_cancels_mid_stream() {
    let server = WireServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(vec![0u8; 100])
                .set_delay(Duration::from_secs(30)),
        )
        .mount(&server)
        .await;

    let client = CopypartyClient::new(&server.uri(), Credentials::Anonymous).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mirror = MirrorStore::new(dir.path());
    let token = CancellationToken::new();

    let dl = download_file(&client, &mirror, "slow.bin", 100, &token, 1);
    tokio::pin!(dl);
    tokio::select! {
        r = &mut dl => panic!("download should not finish before cancel: {r:?}"),
        _ = tokio::time::sleep(Duration::from_millis(100)) => token.cancel(),
    }
    let outcome = dl.await.unwrap();
    assert_eq!(outcome, FileOutcome::Canceled);
    assert!(
        !mirror.is_complete("slow.bin"),
        "nothing committed on cancel"
    );
}

// ---- engine (mock-copyparty) ------------------------------------------------

#[tokio::test]
async fn sync_now_populates_index() {
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let s = setup();
    let mut dev = device_at(srv.addr(), FileSelection::previews_only());
    dev.id = s.repo.insert_device(&dev).unwrap();

    let drives = s.engine.sync_now(&dev).await.unwrap();
    assert_eq!(drives.len(), 1);
    assert_eq!(drives[0].segment_count, 3);
    // Persisted: a fresh read returns the same drive.
    assert_eq!(s.repo.get_drives(dev.id).unwrap().len(), 1);
}

#[tokio::test]
async fn downloads_full_drive_with_progress() {
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let s = setup();
    let mut dev = device_at(srv.addr(), FileSelection::everything());
    dev.id = s.repo.insert_device(&dev).unwrap();

    let drives = s.engine.sync_now(&dev).await.unwrap();
    let dk = drives[0].drive_key.clone();
    let rec = Arc::new(Recorder::default());
    let token = CancellationToken::new();

    let outcome = s
        .engine
        .download_drive(&dev, &dk, rec.clone(), token)
        .await
        .unwrap();
    assert_eq!(outcome, JobOutcome::Complete);

    // Every selected file committed at its exact fixture size (5 files × 3 segs).
    let mirror = MirrorStore::new(s.mirror_root.join(dev.id.to_string()));
    for n in 0..3 {
        assert_eq!(mirror.local_size(&rel(ROUTE, n, "qcamera.ts")), Some(1200));
        assert_eq!(mirror.local_size(&rel(ROUTE, n, "rlog.zst")), Some(300));
        assert_eq!(
            mirror.local_size(&rel(ROUTE, n, "fcamera.hevc")),
            Some(7600)
        );
    }

    // Drive + job reflect completion.
    let drive = s
        .repo
        .get_drives(dev.id)
        .unwrap()
        .into_iter()
        .find(|d| d.drive_key == dk)
        .unwrap();
    assert_eq!(drive.sync_state, SyncStatus::Complete);
    let job = s.repo.get_job(dev.id, &dk).unwrap().unwrap();
    assert_eq!(job.state, JobState::Complete);
    assert_eq!(job.files_total, 15);
    assert_eq!(job.files_done, 15);

    // ProgressSink fired completion; files_done is monotonic and ends at total.
    assert_eq!(rec.completed.lock().unwrap().as_slice(), [dk]);
    let progress = rec.progress.lock().unwrap();
    assert!(progress
        .windows(2)
        .all(|w| w[0].files_done <= w[1].files_done));
    assert_eq!(progress.last().unwrap().files_done, 15);
}

#[tokio::test]
async fn previews_only_downloads_qcamera_only() {
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let s = setup();
    let mut dev = device_at(srv.addr(), FileSelection::previews_only());
    dev.id = s.repo.insert_device(&dev).unwrap();

    let drives = s.engine.sync_now(&dev).await.unwrap();
    let dk = drives[0].drive_key.clone();
    let rec = Arc::new(Recorder::default());

    s.engine
        .download_drive(&dev, &dk, rec, CancellationToken::new())
        .await
        .unwrap();

    let mirror = MirrorStore::new(s.mirror_root.join(dev.id.to_string()));
    for n in 0..3 {
        assert!(mirror.is_complete(&rel(ROUTE, n, "qcamera.ts")));
        assert!(!mirror.is_complete(&rel(ROUTE, n, "fcamera.hevc")));
        assert!(!mirror.is_complete(&rel(ROUTE, n, "rlog.zst")));
    }
    let job = s.repo.get_job(dev.id, &dk).unwrap().unwrap();
    assert_eq!(job.files_total, 3, "one qcamera per segment");
}

#[tokio::test]
async fn cancel_mid_download_stops_the_job() {
    // Seed the index directly. The resolver lists `routes/` for liveness, so
    // answer that fast and delay only the actual file transfer (the cancellable
    // part) — mirroring a real server (listings are quick, the big file is slow).
    let server = WireServer::start().await;
    Mock::given(method("GET"))
        .and(path("/routes/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"dirs":[],"files":[]}"#))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"qcamera\.ts$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(vec![0u8; 1200])
                .set_delay(Duration::from_secs(30)),
        )
        .mount(&server)
        .await;

    let s = setup();
    let mut dev = device_at(*server.address(), FileSelection::previews_only());
    dev.id = s.repo.insert_device(&dev).unwrap();
    // One segment with a single qcamera file.
    let drives = {
        use dashdown_core::drive_grouping::group_segments;
        use dashdown_core::model::{FileKind, Segment, SegmentFile, SegmentName};
        let segs = vec![Segment {
            name: SegmentName {
                route_id: ROUTE.into(),
                segment_num: 0,
            },
            files: vec![SegmentFile {
                kind: FileKind::QCamera,
                name: "qcamera.ts".into(),
                remote_size: 1200,
                mtime_s: 1000,
            }],
            recording: false,
        }];
        s.repo.upsert_segments(dev.id, &segs).unwrap();
        let drives = group_segments(segs);
        s.repo.replace_drives(dev.id, &drives).unwrap();
        drives
    };
    let dk = drives[0].drive_key.clone();

    let engine = s.engine.clone();
    let dev2 = dev.clone();
    let dk2 = dk.clone();
    let rec: Arc<dyn ProgressSink> = Arc::new(Recorder::default());
    let token = CancellationToken::new();
    let child = token.clone();
    let handle = tokio::spawn(async move { engine.download_drive(&dev2, &dk2, rec, child).await });

    tokio::time::sleep(Duration::from_millis(150)).await;
    token.cancel();
    let outcome = handle.await.unwrap().unwrap();
    assert_eq!(outcome, JobOutcome::Canceled);

    let mirror = MirrorStore::new(s.mirror_root.join(dev.id.to_string()));
    assert!(!mirror.is_complete(&rel(ROUTE, 0, "qcamera.ts")));
    let job = s.repo.get_job(dev.id, &dk).unwrap().unwrap();
    assert_eq!(job.state, JobState::Canceled);
    let drive = s
        .repo
        .get_drives(dev.id)
        .unwrap()
        .into_iter()
        .find(|d| d.drive_key == dk)
        .unwrap();
    assert_ne!(drive.sync_state, SyncStatus::Complete);
}
