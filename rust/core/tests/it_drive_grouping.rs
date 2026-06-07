//! Integration: the remote drive-grouping path (`list_segments → group_segments`)
//! against the hermetic axum mock-copyparty server, plus DB persistence of the
//! grouped drives (`replace_drives`/`get_drives`).

use dashdown_core::copyparty_client::{CopypartyClient, Credentials};
use dashdown_core::db::Repo;
use dashdown_core::drive_grouping::{group_segments, remote::group_remote};
use dashdown_core::model::{
    ConnMode, Device, FileKind, FileSelection, Segment, SegmentFile, SegmentName, SyncStatus,
};
use mock_copyparty::{fixtures, MockServer};

fn client_for(srv: &MockServer) -> CopypartyClient {
    CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap()
}

/// Two routes × two consecutive segments each ⇒ two drives. Built directly
/// (no server) for the persistence tests.
fn two_route_segments() -> Vec<Segment> {
    let mut out = Vec::new();
    for (route, base) in [
        ("000001a3--c20ba54385", 1000i64),
        ("000001a4--aabbccddee", 5000),
    ] {
        for n in 0..2u32 {
            out.push(Segment {
                name: SegmentName {
                    route_id: route.to_string(),
                    segment_num: n,
                },
                files: vec![SegmentFile {
                    kind: FileKind::QCamera,
                    name: "qcamera.ts".into(),
                    remote_size: 1200,
                    mtime_s: base + n as i64 * 60,
                }],
                recording: false,
            });
        }
    }
    out
}

#[tokio::test]
async fn single_drive_groups_into_one() {
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let drives = group_remote(&client_for(&srv), "routes/").await.unwrap();

    assert_eq!(drives.len(), 1);
    let d = &drives[0];
    assert_eq!(d.route_id, "000001a3--c20ba54385");
    assert_eq!(d.segment_count, 3);
    assert_eq!(d.first_segment_num, 0);
    assert_eq!(d.last_segment_num, 2);
    assert_eq!(d.drive_key, "000001a3--c20ba54385--0");
    assert!(!d.recording);
    assert!(d.start_ms.is_some() && d.end_ms.is_some());
    assert!(d.end_ms >= d.start_ms);
}

#[tokio::test]
async fn gap_split_groups_into_two_drives() {
    let srv = MockServer::spawn(fixtures::gap_split(), None)
        .await
        .unwrap();
    let drives = group_remote(&client_for(&srv), "routes/").await.unwrap();

    assert_eq!(drives.len(), 2);
    assert_ne!(drives[0].route_id, drives[1].route_id);
    assert_eq!(drives[0].segment_count, 2);
    assert_eq!(drives[1].segment_count, 2);
}

#[tokio::test]
async fn partial_drive_flags_recording() {
    let srv = MockServer::spawn(fixtures::partial(), None).await.unwrap();
    let drives = group_remote(&client_for(&srv), "routes/").await.unwrap();

    // Segments 0 and 1 of one route are consecutive → a single in-progress drive.
    assert_eq!(drives.len(), 1);
    let d = &drives[0];
    assert_eq!(d.segment_count, 2);
    assert!(d.recording, "last segment has rlog.lock → drive recording");
    assert!(d.start_ms.is_some());
    assert!(d.end_ms.is_some());
}

// ---- persistence ----------------------------------------------------------

fn test_device() -> Device {
    Device {
        id: 0,
        name: "dev".into(),
        dongle_label: None,
        hotspot_ip: "192.168.43.1".into(),
        wifi_ip: None,
        port: 3923,
        active_mode: ConnMode::Hotspot,
        password: None,
        auto_sync: false,
        file_selection: FileSelection::previews_only(),
        retention_max_minutes: None,
        auto_delete_from_comma: false,
        auto_delete_min_age_min: 60,
    }
}

#[tokio::test]
async fn persists_drives_and_hydrates_segments() {
    let srv = MockServer::spawn(fixtures::gap_split(), None)
        .await
        .unwrap();
    let segments = client_for(&srv).list_segments("routes/").await.unwrap();

    let repo = Repo::open_in_memory().unwrap();
    let device_id = repo.insert_device(&test_device()).unwrap();
    repo.upsert_segments(device_id, &segments).unwrap();

    let grouped = group_segments(segments);
    repo.replace_drives(device_id, &grouped).unwrap();

    let read_back = repo.get_drives(device_id).unwrap();
    assert_eq!(
        read_back, grouped,
        "round-trip equals the in-memory grouping"
    );
    // Segments are hydrated, not left empty.
    assert!(read_back
        .iter()
        .all(|d| d.segment_count as usize == d.segments.len()));
    assert!(read_back.iter().all(|d| !d.segments.is_empty()));
}

#[test]
fn regroup_preserves_user_flags_and_prunes_orphans() {
    let repo = Repo::open_in_memory().unwrap();
    let device_id = repo.insert_device(&test_device()).unwrap();

    // Two routes' worth of segments → two drives.
    let segments = two_route_segments();
    repo.upsert_segments(device_id, &segments).unwrap();
    let drives = group_segments(segments.clone());
    assert_eq!(drives.len(), 2);
    repo.replace_drives(device_id, &drives).unwrap();

    // User pins drive 0 and the sync engine marks it Complete.
    let key0 = drives[0].drive_key.clone();
    repo.set_drive_preserved(device_id, &key0, true).unwrap();
    repo.set_drive_sync_state(device_id, &key0, SyncStatus::Complete)
        .unwrap();

    // Regroup with only the FIRST route's segments → drive 1 is now an orphan.
    let route0 = drives[0].route_id.clone();
    let kept: Vec<_> = segments
        .into_iter()
        .filter(|s| s.name.route_id == route0)
        .collect();
    let regrouped = group_segments(kept);
    assert_eq!(regrouped.len(), 1);
    repo.replace_drives(device_id, &regrouped).unwrap();

    let read_back = repo.get_drives(device_id).unwrap();
    assert_eq!(read_back.len(), 1, "the second drive was pruned");
    assert_eq!(read_back[0].drive_key, key0);
    assert!(read_back[0].preserved, "preserved survived the regroup");
    assert_eq!(
        read_back[0].sync_state,
        SyncStatus::Complete,
        "sync_state survived the regroup"
    );
}

#[test]
fn empty_input_clears_device_drives() {
    let repo = Repo::open_in_memory().unwrap();
    let device_id = repo.insert_device(&test_device()).unwrap();

    let segments = two_route_segments();
    repo.upsert_segments(device_id, &segments).unwrap();
    repo.replace_drives(device_id, &group_segments(segments))
        .unwrap();
    assert!(!repo.get_drives(device_id).unwrap().is_empty());

    repo.replace_drives(device_id, &[]).unwrap();
    assert!(repo.get_drives(device_id).unwrap().is_empty());
}
