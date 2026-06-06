//! TLS handshake smoke (B0): proves the **ring** crypto provider actually
//! performs a handshake at runtime.
//!
//! The core is built with `rustls-no-provider` (so the dependency graph drops
//! aws-lc-rs and cross-compiles to iOS/Android). The price is that a provider
//! must be installed as the process default before any rustls config is built —
//! [`dashdown_core::tls::ensure_crypto_provider`] does that. Every other test
//! talks to the mock copyparty over plain **HTTP**, so none of them exercise the
//! TLS code path. This one stands up a local self-signed HTTPS server and does a
//! real `reqwest` GET over TLS, driving ring end-to-end on both client + server.

use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

#[tokio::test]
async fn ring_provider_completes_tls_handshake() {
    // The production install path; required before building any rustls config
    // under `rustls-no-provider`. Idempotent.
    dashdown_core::tls::ensure_crypto_provider();

    // Self-signed cert + key for "localhost".
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_der: CertificateDer<'static> = ck.cert.der().clone();
    let key_der: PrivateKeyDer<'static> =
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(ck.signing_key.serialize_der()));

    // Minimal TLS server built off the installed (ring) process-default provider.
    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // One-shot HTTPS responder: complete the TLS handshake, read the request,
    // reply with a fixed body.
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut tls = acceptor.accept(tcp).await.unwrap();
        let mut buf = [0u8; 2048];
        let _ = tls.read(&mut buf).await.unwrap();
        let body = "pong";
        let resp = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        tls.write_all(resp.as_bytes()).await.unwrap();
        let _ = tls.shutdown().await;
    });

    // reqwest client (rustls + ring) trusting our self-signed cert. `resolve`
    // pins "localhost" to the bound socket so cert-name verification passes
    // without depending on system DNS.
    let client = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_der(cert_der.as_ref()).unwrap())
        .resolve("localhost", addr)
        .build()
        .unwrap();

    let resp = client
        .get(format!("https://localhost:{}/", addr.port()))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    assert_eq!(resp.text().await.unwrap(), "pong");
}
