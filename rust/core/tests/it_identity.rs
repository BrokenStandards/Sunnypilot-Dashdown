//! B1 transport & identity, end-to-end over a real self-signed TLS server: the
//! client connects via HTTPS, captures the leaf fingerprint and reads the
//! copyparty `srv_info` hostname; the engine auto-resolves across IPs, prefers
//! HTTPS, pins the hostname, and refuses a different-hostname endpoint.

use std::net::SocketAddr;
use std::sync::Arc;

use dashdown_core::copyparty_client::{CopypartyClient, Credentials};
use dashdown_core::db::Repo;
use dashdown_core::identity::{parse_hostname, DeviceIdentity};
use dashdown_core::model::{ConnMode, Device, FileSelection};
use dashdown_core::sync_engine::SyncEngine;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

/// A minimal self-signed HTTPS server that answers like copyparty: `/routes/`
/// returns an (empty) `?ls=j` listing; anything else returns an HTML page
/// carrying the `srv_info` hostname. Returns its address + the hex SHA-256 of
/// the leaf cert it presents. Binds on `ip` (use `127.0.0.2` for a "down" peer).
async fn spawn_tls(ip: &str, name: &str) -> (SocketAddr, String) {
    dashdown_core::tls::ensure_crypto_provider();
    let ck = rcgen::generate_simple_self_signed(vec![ip.to_string(), name.to_string()]).unwrap();
    let der = ck.cert.der().to_vec();
    let fp = hex(&Sha256::digest(&der));
    let certs = vec![CertificateDer::from(der)];
    let key = PrivateKeyDer::try_from(ck.signing_key.serialize_der()).unwrap();
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind((ip, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let name = name.to_string();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            let name = name.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let n = tls.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let target = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/");
                let (body, ctype) = if target.starts_with("/routes/") {
                    (r#"{"dirs":[],"files":[]}"#.to_string(), "application/json")
                } else {
                    (
                        format!(
                            "<html><head><title>{name} - /</title></head><body>\
                             <p><span id=\"srv_info\"><span>{name}</span> // \
                             <span>1 GiB free of 2 GiB</span></span></p></body></html>"
                        ),
                        "text/html",
                    )
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tls.write_all(resp.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    (addr, fp)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn device(hotspot: &str, wifi: Option<&str>, port: u16) -> Device {
    Device {
        id: 0,
        name: "T".into(),
        dongle_label: None,
        hotspot_ip: hotspot.into(),
        wifi_ip: wifi.map(str::to_string),
        port,
        active_mode: ConnMode::Hotspot,
        password: None,
        auto_sync: false,
        file_selection: FileSelection::previews_only(),
        retention_max_minutes: None,
        auto_delete_from_comma: false,
        auto_delete_min_age_min: 60,
        cap_warn_enabled: true,
        cap_warn_threshold_minutes: 10,
    }
}

struct Setup {
    repo: Arc<Repo>,
    engine: SyncEngine,
    _dir: tempfile::TempDir,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let repo = Arc::new(Repo::open(&dir.path().join("index.db")).unwrap());
    let engine = SyncEngine::new(repo.clone(), dir.path().join("mirror"));
    Setup {
        repo,
        engine,
        _dir: dir,
    }
}

/// The client speaks HTTPS to the self-signed server, captures the leaf
/// fingerprint, and the hostname is readable from the served HTML.
#[tokio::test(flavor = "multi_thread")]
async fn client_https_captures_fingerprint_and_hostname() {
    let (addr, fp) = spawn_tls("127.0.0.1", "comma-e0e384a").await;
    let client = CopypartyClient::new(&format!("https://{addr}/"), Credentials::Anonymous).unwrap();

    // No HTTPS request yet → nothing captured.
    assert_eq!(client.last_cert_sha256(), None);

    client.list_dir("routes/").await.unwrap();
    assert_eq!(
        client.last_cert_sha256().as_deref(),
        Some(fp.as_str()),
        "captured the server's actual leaf fingerprint"
    );

    let html = client.fetch_root_html().await.unwrap();
    assert_eq!(parse_hostname(&html).as_deref(), Some("comma-e0e384a"));
}

/// `sync_now` resolves over HTTPS and pins the device's identity + last-good base.
#[tokio::test(flavor = "multi_thread")]
async fn engine_resolves_https_and_pins_identity() {
    let (addr, fp) = spawn_tls("127.0.0.1", "comma-pinme").await;
    let s = setup();
    let mut dev = device("127.0.0.1", None, addr.port());
    dev.id = s.repo.insert_device(&dev).unwrap();

    s.engine.sync_now(&dev).await.unwrap();

    let pinned = s.repo.get_device_identity(dev.id).unwrap().unwrap();
    assert_eq!(pinned.hostname.as_deref(), Some("comma-pinme"));
    assert_eq!(pinned.cert_sha256.as_deref(), Some(fp.as_str()));
    let base = s.repo.get_last_good_base(dev.id).unwrap().unwrap();
    assert!(base.starts_with("https://"), "resolved over HTTPS: {base}");
}

/// With two IPs, an unreachable one is skipped and the reachable one wins.
#[tokio::test(flavor = "multi_thread")]
async fn resolves_across_multiple_ips() {
    let (addr, _fp) = spawn_tls("127.0.0.1", "comma-multi").await;
    let s = setup();
    // hotspot IP is down (nothing on 127.0.0.2:port); wifi IP hosts the server.
    let mut dev = device("127.0.0.2", Some("127.0.0.1"), addr.port());
    dev.id = s.repo.insert_device(&dev).unwrap();

    s.engine.sync_now(&dev).await.unwrap();

    let base = s.repo.get_last_good_base(dev.id).unwrap().unwrap();
    assert!(
        base.contains("127.0.0.1"),
        "failed over to the reachable IP: {base}"
    );
}

/// A reachable endpoint whose hostname differs from the pin is refused.
#[tokio::test(flavor = "multi_thread")]
async fn rejects_a_different_device() {
    let (addr, _fp) = spawn_tls("127.0.0.1", "comma-IMPOSTER").await;
    let s = setup();
    let mut dev = device("127.0.0.1", None, addr.port());
    dev.id = s.repo.insert_device(&dev).unwrap();
    // Pre-pin a *different* device.
    s.repo
        .set_device_identity(
            dev.id,
            &DeviceIdentity {
                hostname: Some("comma-trusted".into()),
                cert_sha256: None,
            },
        )
        .unwrap();

    let err = s.engine.sync_now(&dev).await.unwrap_err();
    assert!(
        matches!(err, dashdown_core::error::CoreError::IdentityMismatch(_)),
        "expected identity mismatch, got {err:?}"
    );
}
