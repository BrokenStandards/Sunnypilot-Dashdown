//! Integration: M7 connectivity dot. Green (reachable + idle), Blue (reachable +
//! a download job running for the device), and Red (unreachable), plus the
//! up→down transition. Reachability is a real TCP connect against a live
//! mock-copyparty server vs. a closed (never-bound) port.

use std::net::SocketAddr;
use std::sync::Arc;

use dashdown_core::db::Repo;
use dashdown_core::model::{ConnDot, ConnMode, Device, FileSelection, JobState};
use dashdown_core::sync_engine::SyncEngine;
use mock_copyparty::{fixtures, MockServer};

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

struct Setup {
    _dir: tempfile::TempDir,
    repo: Arc<Repo>,
    engine: SyncEngine,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let repo = Arc::new(Repo::open(&dir.path().join("index.db")).unwrap());
    let engine = SyncEngine::new(repo.clone(), dir.path().join("mirror"));
    Setup {
        _dir: dir,
        repo,
        engine,
    }
}

/// A 127.0.0.1 port that is bound then released — nothing listens on it.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test]
async fn green_when_reachable_and_idle() {
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let s = setup();
    let mut dev = device_at(srv.addr(), FileSelection::previews_only());
    dev.id = s.repo.insert_device(&dev).unwrap();

    let c = s.engine.check_connectivity(&dev).await.unwrap();
    assert_eq!(c.dot, ConnDot::Green);
    assert!(c.reachable);
    assert!(!c.downloading);
}

#[tokio::test]
async fn red_when_unreachable() {
    let s = setup();
    // Point at a closed localhost port — connect is refused fast (no timeout wait).
    let addr: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
    let mut dev = device_at(addr, FileSelection::previews_only());
    dev.id = s.repo.insert_device(&dev).unwrap();

    let c = s.engine.check_connectivity(&dev).await.unwrap();
    assert_eq!(c.dot, ConnDot::Red);
    assert!(!c.reachable);
    assert!(!c.downloading);
}

#[tokio::test]
async fn up_then_down_transitions_green_to_red() {
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let s = setup();
    let mut dev = device_at(srv.addr(), FileSelection::previews_only());
    dev.id = s.repo.insert_device(&dev).unwrap();

    assert_eq!(
        s.engine.check_connectivity(&dev).await.unwrap().dot,
        ConnDot::Green
    );

    // Device goes away: dropping the server frees the port → connect refused.
    drop(srv);
    assert_eq!(
        s.engine.check_connectivity(&dev).await.unwrap().dot,
        ConnDot::Red
    );
}

#[tokio::test]
async fn blue_while_downloading() {
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let s = setup();
    let mut dev = device_at(srv.addr(), FileSelection::previews_only());
    dev.id = s.repo.insert_device(&dev).unwrap();

    // Seed a running job (no real transfer needed — upsert_job writes state='running').
    s.repo.upsert_job(dev.id, "somedrive", 1, 100).unwrap();
    let c = s.engine.check_connectivity(&dev).await.unwrap();
    assert_eq!(c.dot, ConnDot::Blue);
    assert!(c.reachable);
    assert!(c.downloading);

    // Job finishes → back to Green.
    s.repo
        .set_job_state(dev.id, "somedrive", JobState::Complete, None)
        .unwrap();
    let c = s.engine.check_connectivity(&dev).await.unwrap();
    assert_eq!(c.dot, ConnDot::Green);
    assert!(!c.downloading);
}
