//! Integration: local retention pruning. Drops the oldest drives beyond the
//! device's `retention_max_minutes` budget (skipping `preserved`), deleting only
//! local mirror files — the remote is never touched.
//!
//! Remote auto-delete-from-comma was removed: sunnypilot serves footage on a
//! read-only copyparty volume, so it can't be deleted over HTTP (a future phase
//! adds SSH-based remote sync/delete).

use std::net::SocketAddr;
use std::sync::Arc;

use dashdown_core::db::Repo;
use dashdown_core::drive_grouping::group_segments;
use dashdown_core::model::{
    ConnMode, Device, FileKind, FileSelection, Segment, SegmentFile, SegmentName, SyncStatus,
};
use dashdown_core::storage::MirrorStore;
use dashdown_core::sync_engine::SyncEngine;

fn rel(route: &str, n: u32, name: &str) -> String {
    format!("routes/{route}--{n}/{name}")
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
