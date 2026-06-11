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

/// One route's segments, one qcamera file each, with an explicit per-segment mtime
/// (segment `i` carries `mtimes[i]`), so segment age order is deterministic.
fn route_segments_at(route: &str, mtimes: &[i64]) -> Vec<Segment> {
    mtimes
        .iter()
        .enumerate()
        .map(|(i, &mtime_s)| Segment {
            name: SegmentName {
                route_id: route.into(),
                segment_num: i as u32,
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

/// Segment-level retention: keep the newest N non-preserved SEGMENTS (a long drive is
/// kept partially), preserved drives are excluded from the budget and never pruned, and
/// a pruned segment is never re-listed for download (the loop is structurally impossible).
#[tokio::test]
async fn retention_prunes_old_segments_keeps_newest_and_preserved() {
    let s = setup();
    let dummy: SocketAddr = "127.0.0.1:1".parse().unwrap(); // no server: retention is local-only
    let mut dev = device(dummy, FileSelection::previews_only());
    dev.retention_max_minutes = Some(3); // keep the newest 3 non-preserved segments
    dev.id = s.repo.insert_device(&dev).unwrap();

    const D: &str = "000000d1--dddddddddd"; // non-preserved, 5 segments (newest footage)
    const P: &str = "000000a0--aaaaaaaaaa"; // preserved, 2 OLD segments
    let mut all = Vec::new();
    all.extend(route_segments_at(P, &[500, 560])); // oldest, but pinned
    all.extend(route_segments_at(D, &[1000, 1060, 1120, 1180, 1240]));

    s.repo.upsert_segments(dev.id, &all).unwrap();
    let drives = group_segments(all.clone());
    s.repo.replace_drives(dev.id, &drives).unwrap();

    // Mirror every segment's qcamera and pin P.
    let mirror = MirrorStore::new(s.mirror_root.join(dev.id.to_string()));
    for seg in &all {
        let r = rel(&seg.name.route_id, seg.name.segment_num, "qcamera.ts");
        mirror.write_all(&r, &[0u8; 1200]).await.unwrap();
    }
    s.repo
        .set_drive_preserved(dev.id, &format!("{P}--0"), true)
        .unwrap();
    let reconciled = s.engine.reconcile_device(&dev).await.unwrap();
    assert!(reconciled
        .iter()
        .all(|d| d.sync_state == SyncStatus::Complete));

    // Budget 3: keep P (preserved, free) + the newest 3 of D (segments 2,3,4). Prune D's
    // oldest two segments only — the drive is kept PARTIALLY.
    let pruned = s.engine.enforce_retention(&dev).await.unwrap();
    assert_eq!(pruned.len(), 2);
    assert!(pruned.contains(&format!("{D}--0")) && pruned.contains(&format!("{D}--1")));

    assert!(!mirror.is_complete(&rel(D, 0, "qcamera.ts")));
    assert!(!mirror.is_complete(&rel(D, 1, "qcamera.ts")));
    for n in 2..5 {
        assert!(mirror.is_complete(&rel(D, n, "qcamera.ts")));
    }
    assert!(mirror.is_complete(&rel(P, 0, "qcamera.ts")));
    assert!(mirror.is_complete(&rel(P, 1, "qcamera.ts")));

    // D is now Partial (lost 2 of 5 segments); P stays Complete + preserved.
    let by_key: std::collections::HashMap<_, _> = s
        .repo
        .get_drives(dev.id)
        .unwrap()
        .into_iter()
        .map(|d| (d.drive_key.clone(), d))
        .collect();
    assert_eq!(by_key[&format!("{D}--0")].sync_state, SyncStatus::Partial);
    assert_eq!(by_key[&format!("{P}--0")].sync_state, SyncStatus::Complete);
    assert!(by_key[&format!("{P}--0")].preserved);

    // Loop guard: the pruned (out-of-window) segments are never re-listed for download.
    let pending = s.engine.pending_download_keys(&dev).await.unwrap();
    assert!(
        pending.is_empty(),
        "pruned segments must not be re-listed for download: {pending:?}"
    );

    // Storage accounting: 5 local minutes (P:2 + D:3), 2 preserved, budget 3.
    let st = s.engine.retention_status(&dev).await.unwrap();
    assert_eq!(st.local_minutes, 5);
    assert_eq!(st.preserved_minutes, 2);
    assert_eq!(st.budget_minutes, Some(3));
}
