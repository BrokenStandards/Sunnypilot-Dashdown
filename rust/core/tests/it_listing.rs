//! Integration: the copyparty client against the hermetic axum mock server.

use dashdown_core::copyparty_client::{CopypartyClient, Credentials};
use dashdown_core::model::FileKind;
use mock_copyparty::{fixtures, MockServer};

#[tokio::test]
async fn lists_single_drive_segments() {
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let client = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();

    let segments = client.list_segments("routes/").await.unwrap();

    assert_eq!(segments.len(), 3);
    for (i, seg) in segments.iter().enumerate() {
        assert_eq!(seg.name.route_id, "000001a3--c20ba54385");
        assert_eq!(seg.name.segment_num, i as u32);
        assert!(!seg.recording);
        // Full file set (5 files), all with positive size + mtime.
        assert_eq!(seg.files.len(), 5);
        assert!(seg.files.iter().any(|f| f.kind == FileKind::QCamera));
        assert!(seg.files.iter().any(|f| f.kind == FileKind::FCamera));
        assert!(seg.files.iter().any(|f| f.kind == FileKind::RLog));
        for f in &seg.files {
            assert!(f.remote_size > 0, "{} has zero size", f.name);
            assert!(f.mtime_s > 0, "{} has zero mtime", f.name);
        }
    }
}

#[tokio::test]
async fn gap_split_exposes_two_routes() {
    let srv = MockServer::spawn(fixtures::gap_split(), None)
        .await
        .unwrap();
    let client = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();

    let segments = client.list_segments("routes/").await.unwrap();

    assert_eq!(segments.len(), 4);
    let routes: std::collections::BTreeSet<_> =
        segments.iter().map(|s| s.name.route_id.clone()).collect();
    assert_eq!(routes.len(), 2, "two distinct routes expected");
}

#[tokio::test]
async fn partial_segment_flags_recording_and_skips_lock() {
    let srv = MockServer::spawn(fixtures::partial(), None).await.unwrap();
    let client = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();

    let segments = client.list_segments("routes/").await.unwrap();
    assert_eq!(segments.len(), 2);

    let recording = segments.iter().find(|s| s.name.segment_num == 1).unwrap();
    assert!(recording.recording, "segment 1 has rlog.lock → recording");
    // The lock marker is never surfaced as a downloadable file.
    assert!(recording
        .files
        .iter()
        .all(|f| f.kind != FileKind::LockMarker));
    assert!(recording.files.iter().any(|f| f.kind == FileKind::QCamera));
}

#[tokio::test]
async fn downloads_a_file() {
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let client = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();

    let bytes = client
        .download("routes/000001a3--c20ba54385--0/qcamera.ts")
        .await
        .unwrap();
    assert_eq!(bytes.len(), 1200);
}

#[tokio::test]
async fn password_auth_round_trips() {
    let srv = MockServer::spawn(fixtures::single_drive(), Some("hunter2".into()))
        .await
        .unwrap();

    // Correct password → ok.
    let ok = CopypartyClient::new(srv.base_url(), Credentials::Password("hunter2".into())).unwrap();
    assert_eq!(ok.list_segments("routes/").await.unwrap().len(), 3);

    // Anonymous → 401 AuthRequired.
    let anon = CopypartyClient::new(srv.base_url(), Credentials::Anonymous).unwrap();
    let err = anon.list_dir("routes/").await.unwrap_err();
    assert!(
        matches!(err, dashdown_core::error::CoreError::AuthRequired),
        "expected AuthRequired, got {err:?}"
    );

    // Wrong password → 403 Forbidden.
    let bad = CopypartyClient::new(srv.base_url(), Credentials::Password("nope".into())).unwrap();
    let err = bad.list_dir("routes/").await.unwrap_err();
    assert!(
        matches!(err, dashdown_core::error::CoreError::Forbidden),
        "expected Forbidden, got {err:?}"
    );
}
