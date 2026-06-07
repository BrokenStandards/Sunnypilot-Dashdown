//! Integration: offline grouping (scan the local mirror) must equal online
//! grouping (list over copyparty) for the same tree — the M3 headline guarantee.
//! Both sides read the same inodes, so sizes and second-truncated mtimes match
//! exactly, making this a full `Vec<Drive>` equality.

use dashdown_core::copyparty_client::{CopypartyClient, Credentials};
use dashdown_core::drive_grouping::{local::group_local, remote::group_remote};
use mock_copyparty::{fixtures, Fixture, MockServer};

/// Group the fixture both ways and assert they are identical.
async fn assert_offline_matches_online(fixture: Fixture) {
    // Capture the realdata path before `spawn` consumes the Fixture (the temp dir
    // stays alive inside the returned MockServer).
    let realdata = fixture.path().join("routes");
    let srv = MockServer::spawn(fixture, None).await.unwrap();
    let client = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();

    let online = group_remote(&client, "routes/").await.unwrap();
    let offline = group_local(&realdata).unwrap();

    assert!(
        !online.is_empty(),
        "fixture should produce at least one drive"
    );
    assert_eq!(
        online, offline,
        "offline grouping must equal online grouping"
    );
}

#[tokio::test]
async fn single_drive_offline_equals_online() {
    assert_offline_matches_online(fixtures::single_drive()).await;
}

#[tokio::test]
async fn gap_split_offline_equals_online() {
    assert_offline_matches_online(fixtures::gap_split()).await;
}

#[tokio::test]
async fn partial_offline_equals_online() {
    assert_offline_matches_online(fixtures::partial()).await;
}

/// `.part` files are scanner artifacts that never appear in copyparty's listing,
/// so this is a scan-only test (no server): a `.part` must not affect grouping.
#[test]
fn scan_ignores_part_files() {
    let dir = tempfile::tempdir().unwrap();
    let rd = dir.path().join("routes");
    let route = "000001a3--c20ba54385";
    for n in 0..2 {
        let seg = rd.join(format!("{route}--{n}"));
        std::fs::create_dir_all(&seg).unwrap();
        std::fs::write(seg.join("qcamera.ts"), b"data").unwrap();
    }

    let baseline = group_local(&rd).unwrap();
    assert_eq!(baseline.len(), 1);

    // Drop an in-progress `.part` into seg 0 — grouping must be unchanged.
    std::fs::write(
        rd.join(format!("{route}--0")).join("fcamera.hevc.part"),
        b"x",
    )
    .unwrap();
    let with_part = group_local(&rd).unwrap();

    assert_eq!(baseline, with_part, ".part must not affect grouping");
}
