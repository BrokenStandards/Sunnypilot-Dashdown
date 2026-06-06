//! Integration: M6 retention + auto-delete. Local retention pruning (oldest
//! beyond budget, skipping `preserved`) and auto-delete-from-comma behind the
//! Complete + age + fresh-re-verify guards, deleting whole segment directories
//! from the (mock) copyparty server while keeping the local copy.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use dashdown_core::copyparty_client::{CopypartyClient, Credentials};
use dashdown_core::db::Repo;
use dashdown_core::drive_grouping::group_segments;
use dashdown_core::model::{
    ConnMode, Device, FileKind, FileSelection, Segment, SegmentFile, SegmentName, SyncStatus,
};
use dashdown_core::storage::MirrorStore;
use dashdown_core::sync_engine::{CancellationToken, DownloadProgress, ProgressSink, SyncEngine};
use mock_copyparty::{fixtures, MockServer};

const SINGLE_ROUTE: &str = "000001a3--c20ba54385";

fn rel(route: &str, n: u32, name: &str) -> String {
    format!("realdata/{route}--{n}/{name}")
}

fn device(addr: SocketAddr, selection: FileSelection) -> Device {
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
        auto_delete_min_age_min: 30,
    }
}

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

// ---- (a) local retention pruning (no network) -------------------------------

/// One route's segments, each carrying a single qcamera file with a fixed mtime
/// (so the derived `end_ms` — and thus drive age order — is deterministic).
fn route_segments(route: &str, n_segs: u32, mtime_s: i64) -> Vec<Segment> {
    (0..n_segs)
        .map(|i| Segment {
            name: SegmentName {
                route_id: route.into(),
                segment_num: i,
            },
            files: vec![SegmentFile {
                kind: FileKind::QCamera,
                name: "qcamera.ts".into(),
                remote_size: 1200,
                mtime_s,
            }],
            recording: false,
        })
        .collect()
}

#[tokio::test]
async fn retention_prunes_oldest_keeps_newest_and_preserved() {
    let s = setup();
    // No server needed — retention is local-only.
    let dummy: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let mut dev = device(dummy, FileSelection::previews_only());
    dev.retention_max_minutes = Some(4); // budget: 4 minutes of footage
    dev.id = s.repo.insert_device(&dev).unwrap();

    // Three drives, newest→oldest by mtime: C(2 segs) newest, B(3 segs), A(2 segs)
    // oldest+pinned. Newest-first fill: C=2 ≤4; B→5 >4 ⇒ prune B; A over but pinned.
    const A: &str = "000000a1--aaaaaaaaaa"; // oldest, preserved
    const B: &str = "000000b2--bbbbbbbbbb"; // middle, pruned
    const C: &str = "000000c3--cccccccccc"; // newest, kept
    let mut all = Vec::new();
    all.extend(route_segments(A, 2, 1000));
    all.extend(route_segments(B, 3, 2000));
    all.extend(route_segments(C, 2, 3000));

    s.repo.upsert_segments(dev.id, &all).unwrap();
    let drives = group_segments(all.clone());
    s.repo.replace_drives(dev.id, &drives).unwrap();

    // Place the local mirror files and pin drive A.
    let mirror = MirrorStore::new(s.mirror_root.join(dev.id.to_string()));
    for seg in &all {
        let r = rel(&seg.name.route_id, seg.name.segment_num, "qcamera.ts");
        mirror.write_all(&r, &[0u8; 1200]).await.unwrap();
    }
    s.repo
        .set_drive_preserved(dev.id, &format!("{A}--0"), true)
        .unwrap();
    // Reconcile so all three are Complete (files present for the selection).
    let reconciled = s.engine.reconcile_device(&dev).await.unwrap();
    assert!(reconciled
        .iter()
        .all(|d| d.sync_state == SyncStatus::Complete));

    // Enforce retention: only B (middle, unpinned, over budget) is pruned.
    let pruned = s.engine.enforce_retention(&dev).await.unwrap();
    assert_eq!(pruned, vec![format!("{B}--0")]);

    // B's local files are gone; A and C remain.
    assert!(!mirror.is_complete(&rel(B, 0, "qcamera.ts")));
    assert!(!mirror.is_complete(&rel(B, 1, "qcamera.ts")));
    assert!(mirror.is_complete(&rel(A, 0, "qcamera.ts")));
    assert!(mirror.is_complete(&rel(C, 0, "qcamera.ts")));

    // Sync state reflects the prune: B NotDownloaded, A & C still Complete.
    let by_key: std::collections::HashMap<_, _> = s
        .repo
        .get_drives(dev.id)
        .unwrap()
        .into_iter()
        .map(|d| (d.drive_key.clone(), d))
        .collect();
    assert_eq!(
        by_key[&format!("{B}--0")].sync_state,
        SyncStatus::NotDownloaded
    );
    assert_eq!(by_key[&format!("{A}--0")].sync_state, SyncStatus::Complete);
    assert_eq!(by_key[&format!("{C}--0")].sync_state, SyncStatus::Complete);
    assert!(by_key[&format!("{A}--0")].preserved);
}

// ---- (b)-(d) auto-delete-from-comma (mock copyparty) ------------------------

/// Sync + fully download the single fixture drive (previews only). Returns the
/// drive_key and its `end_ms` (for age-guard arithmetic).
async fn sync_and_download(s: &Setup, dev: &Device) -> (String, i64) {
    let drives = s.engine.sync_now(dev).await.unwrap();
    let dk = drives[0].drive_key.clone();
    let end_ms = drives[0].end_ms.expect("fixture drive has a time");
    let rec: Arc<dyn ProgressSink> = Arc::new(Recorder::default());
    s.engine
        .download_drive(dev, &dk, rec, CancellationToken::new())
        .await
        .unwrap();
    (dk, end_ms)
}

#[tokio::test]
async fn auto_delete_removes_remote_keeps_local_and_survives_resync() {
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let s = setup();
    let mut dev = device(srv.addr(), FileSelection::previews_only());
    dev.auto_delete_from_comma = true;
    dev.id = s.repo.insert_device(&dev).unwrap();

    let (dk, end_ms) = sync_and_download(&s, &dev).await;
    let mirror = MirrorStore::new(s.mirror_root.join(dev.id.to_string()));
    assert!(mirror.is_complete(&rel(SINGLE_ROUTE, 0, "qcamera.ts")));

    // now_ms exactly at the age boundary ⇒ eligible.
    let now_ms = end_ms + dev.auto_delete_min_age_min * 60_000;
    let deleted = s.engine.auto_delete_from_comma(&dev, now_ms).await.unwrap();
    assert_eq!(deleted, vec![dk.clone()]);

    // Remote: the whole drive's segment dirs are gone (listing is now empty).
    let client = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();
    assert!(
        client.list_segments("realdata/").await.unwrap().is_empty(),
        "remote segments deleted"
    );
    // Local: the mirror copy is untouched.
    for n in 0..3 {
        assert!(mirror.is_complete(&rel(SINGLE_ROUTE, n, "qcamera.ts")));
    }

    // A follow-up sync against the now-empty remote keeps the drive (it has local
    // data) — proving the replace_drives refinement, without touching `preserved`.
    let after = s.engine.sync_now(&dev).await.unwrap();
    let kept = after
        .iter()
        .find(|d| d.drive_key == dk)
        .expect("comma-deleted drive stays in the library");
    assert_eq!(kept.sync_state, SyncStatus::Complete);
    assert!(!kept.preserved, "auto-delete must not auto-pin the drive");
}

#[tokio::test]
async fn auto_delete_skips_too_recent_drive() {
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let s = setup();
    let mut dev = device(srv.addr(), FileSelection::previews_only());
    dev.auto_delete_from_comma = true;
    dev.id = s.repo.insert_device(&dev).unwrap();

    let (_dk, end_ms) = sync_and_download(&s, &dev).await;

    // One ms short of the age threshold ⇒ not eligible.
    let now_ms = end_ms + dev.auto_delete_min_age_min * 60_000 - 1;
    let deleted = s.engine.auto_delete_from_comma(&dev, now_ms).await.unwrap();
    assert!(deleted.is_empty(), "too-recent drive must not be deleted");

    let client = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();
    assert_eq!(
        client.list_segments("realdata/").await.unwrap().len(),
        3,
        "remote untouched"
    );
}

#[tokio::test]
async fn auto_delete_skips_incomplete_drive() {
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let s = setup();
    let mut dev = device(srv.addr(), FileSelection::previews_only());
    dev.auto_delete_from_comma = true;
    dev.id = s.repo.insert_device(&dev).unwrap();

    let (_dk, end_ms) = sync_and_download(&s, &dev).await;

    // Make the drive Partial: drop one local file and reclassify.
    let mirror = MirrorStore::new(s.mirror_root.join(dev.id.to_string()));
    mirror
        .remove_file(&rel(SINGLE_ROUTE, 1, "qcamera.ts"))
        .await
        .unwrap();
    let drives = s.engine.reconcile_device(&dev).await.unwrap();
    assert_eq!(drives[0].sync_state, SyncStatus::Partial);

    // Old enough, but not Complete ⇒ not eligible.
    let now_ms = end_ms + dev.auto_delete_min_age_min * 60_000 + 60_000;
    let deleted = s.engine.auto_delete_from_comma(&dev, now_ms).await.unwrap();
    assert!(deleted.is_empty(), "incomplete drive must not be deleted");

    let client = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();
    assert_eq!(
        client.list_segments("realdata/").await.unwrap().len(),
        3,
        "remote untouched"
    );
}
