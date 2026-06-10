//! B1: exercise the new mock-copyparty harness capabilities (the ones
//! `mock-comma-mcp` drives) through the real core APIs the app uses —
//! fixed-port reachability toggling, the index-gap fixture, and the
//! size-mismatch listing override.

use std::net::SocketAddr;
use std::time::Duration;

use dashdown_core::connectivity::{tcp_reachable, DEFAULT_CONNECT_TIMEOUT};
use dashdown_core::copyparty_client::{CopypartyClient, Credentials};
use dashdown_core::drive_grouping::group_segments;
use mock_copyparty::{fixtures, MockServer, ServeOptions};

/// A free 127.0.0.1 port (bound then released).
async fn free_port() -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

/// Toggling reachability = dropping/re-binding the server on a STABLE port.
/// Dropping closes the listening socket (connect → refused → Red); re-binding
/// the same port (SO_REUSEADDR) restores it, so the app's configured URL stays
/// valid across toggles.
#[tokio::test(flavor = "multi_thread")]
async fn reachability_toggle_reuses_same_port() {
    let port = free_port().await;
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let opts = || ServeOptions {
        addr: Some(addr),
        ..Default::default()
    };

    // up
    let srv = MockServer::spawn_with(std::env::temp_dir(), opts())
        .await
        .unwrap();
    assert_eq!(srv.addr().port(), port);
    assert!(tcp_reachable("127.0.0.1", port, DEFAULT_CONNECT_TIMEOUT).await);

    // down: dropping closes the listener → connection refused
    drop(srv);
    tokio::time::sleep(Duration::from_millis(50)).await; // let the abort land
    assert!(!tcp_reachable("127.0.0.1", port, Duration::from_millis(500)).await);

    // up again on the SAME port
    let srv2 = MockServer::spawn_with(std::env::temp_dir(), opts())
        .await
        .unwrap();
    assert_eq!(srv2.addr().port(), port);
    assert!(tcp_reachable("127.0.0.1", port, DEFAULT_CONNECT_TIMEOUT).await);
}

/// A segment-index gap (0, 1, 3) splits into two drives.
#[tokio::test(flavor = "multi_thread")]
async fn gap_index_groups_into_two_drives() {
    let srv = MockServer::spawn(fixtures::gap_index(), None)
        .await
        .unwrap();
    let client = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();
    let segs = client.list_segments("routes/").await.unwrap();
    let drives = group_segments(segs);
    assert_eq!(drives.len(), 2, "index gap 0,1,3 → two drives");
}

/// Appending a segment to a served route shows up on the very next listing —
/// no restart — because the listing handler walks disk per request. This is the
/// mechanism Phase B's "segment added to an active drive → synced" relies on.
#[tokio::test(flavor = "multi_thread")]
async fn add_segment_grows_drive_live() {
    let fx = fixtures::single_drive(); // one route, segments 0,1,2
    let root = fx.dir.path().to_path_buf(); // TempDir is moved into the server but the path stays
    let srv = MockServer::spawn(fx, None).await.unwrap();
    let client = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();

    let before = group_segments(client.list_segments("routes/").await.unwrap());
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].segment_count, 3);

    mock_copyparty::mutate::add_segment(&root, None, 1).unwrap();

    let after = group_segments(client.list_segments("routes/").await.unwrap());
    assert_eq!(after.len(), 1, "still one drive (same route)");
    assert_eq!(
        after[0].segment_count, 4,
        "the appended segment is served live"
    );
}

/// Adding and removing a whole route (drive) is reflected live in the grouping —
/// the basis for Phase C's "drive list updates on add/remove without manual
/// refresh" and the Comma's own low-space auto-prune.
#[tokio::test(flavor = "multi_thread")]
async fn add_and_remove_drive_live() {
    let fx = fixtures::single_drive();
    let root = fx.dir.path().to_path_buf();
    let srv = MockServer::spawn(fx, None).await.unwrap();
    let client = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();

    assert_eq!(
        group_segments(client.list_segments("routes/").await.unwrap()).len(),
        1
    );

    mock_copyparty::mutate::add_drive(&root, "000009ff--newdrive00", 2).unwrap();
    assert_eq!(
        group_segments(client.list_segments("routes/").await.unwrap()).len(),
        2,
        "a new route appears as a second drive"
    );

    mock_copyparty::mutate::remove_drive(&root, "000009ff--newdrive00").unwrap();
    assert_eq!(
        group_segments(client.list_segments("routes/").await.unwrap()).len(),
        1,
        "removing the route drops the drive"
    );
}

/// The size-mismatch fixture advertises an inflated `sz` (1200) for a file that
/// is really 600 bytes on disk — the listing the client sees reports the lie.
#[tokio::test(flavor = "multi_thread")]
async fn size_mismatch_advertises_inflated_size() {
    let srv = MockServer::spawn(fixtures::size_mismatch(), None)
        .await
        .unwrap();
    let client = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();
    let segs = client.list_segments("routes/").await.unwrap();
    let qcam = segs
        .iter()
        .flat_map(|s| &s.files)
        .find(|f| f.name == "qcamera.ts")
        .expect("qcamera.ts present");
    assert_eq!(
        qcam.remote_size, 1200,
        "listing advertises the override, not the real 600 bytes"
    );
}
