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
    let segs = client.list_segments("realdata/").await.unwrap();
    let drives = group_segments(segs);
    assert_eq!(drives.len(), 2, "index gap 0,1,3 → two drives");
}

/// The size-mismatch fixture advertises an inflated `sz` (1200) for a file that
/// is really 600 bytes on disk — the listing the client sees reports the lie.
#[tokio::test(flavor = "multi_thread")]
async fn size_mismatch_advertises_inflated_size() {
    let srv = MockServer::spawn(fixtures::size_mismatch(), None)
        .await
        .unwrap();
    let client = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();
    let segs = client.list_segments("realdata/").await.unwrap();
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
