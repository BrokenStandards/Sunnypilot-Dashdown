//! The runtime control plane: the [`Supervisor`] toggles reachability on a fixed
//! port, and the HTTP control router mutates the served tree live. Driven over a
//! raw HTTP/1.1 socket (no HTTP-client dep); mutations are asserted on disk and
//! via `GET /status`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use mock_copyparty::control::{serve_control, Supervisor};
use mock_copyparty::mutate;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

/// A free 127.0.0.1 port (bound then released).
async fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

async fn tcp_ok(port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_millis(500),
            TcpStream::connect(("127.0.0.1", port)),
        )
        .await,
        Ok(Ok(_))
    )
}

/// Send one HTTP/1.1 request with `Connection: close` and return the full
/// response text (headers + body).
async fn http(port: u16, method: &str, path: &str, body: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = if method == "GET" {
        format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
    } else {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

fn body_json(resp: &str) -> Value {
    let body = resp.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    serde_json::from_str(body).unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn supervisor_toggles_reachability_on_same_port() {
    let tmp = tempfile::TempDir::new().unwrap();
    mutate::add_drive(tmp.path(), "000001a3--c20ba54385", 2, None).unwrap();
    let dport = free_port().await;
    let data_addr = SocketAddr::from(([127, 0, 0, 1], dport));

    let mut sup = Supervisor::new(tmp.path().to_path_buf(), data_addr, None, HashMap::new())
        .await
        .unwrap();
    assert!(tcp_ok(dport).await, "data server up after new()");

    sup.set_reachable(false).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!tcp_ok(dport).await, "down after set_reachable(false)");

    sup.set_reachable(true).await.unwrap();
    assert!(tcp_ok(dport).await, "up again on the SAME port");
}

#[tokio::test(flavor = "multi_thread")]
async fn control_plane_mutates_tree_and_reports_status() {
    let tmp = tempfile::TempDir::new().unwrap();
    let route = "000001a3--c20ba54385";
    mutate::add_drive(tmp.path(), route, 3, None).unwrap();

    let dport = free_port().await;
    let data_addr = SocketAddr::from(([127, 0, 0, 1], dport));
    let sup = Arc::new(Mutex::new(
        Supervisor::new(tmp.path().to_path_buf(), data_addr, None, HashMap::new())
            .await
            .unwrap(),
    ));
    let cport = free_port().await;
    let _ctl = serve_control(SocketAddr::from(([127, 0, 0, 1], cport)), sup.clone())
        .await
        .unwrap();

    // add_segment (default route, n=1) → route grows 3 → 4 on disk, live.
    let resp = http(cport, "POST", "/add_segment", "{}").await;
    assert!(resp.contains("\"ok\":true"), "add_segment: {resp}");
    assert!(tmp
        .path()
        .join("routes")
        .join(format!("{route}--3"))
        .is_dir());

    // status reports reachable + the grown segment count.
    let status = body_json(&http(cport, "GET", "/status", "").await);
    assert_eq!(status["reachable"], json!(true));
    assert_eq!(status["data_port"], json!(dport));
    assert_eq!(status["routes"][0]["segments"], json!(4));

    // add_drive then remove_drive change the route set.
    http(
        cport,
        "POST",
        "/add_drive",
        r#"{"route":"000009ff--new0","segs":2}"#,
    )
    .await;
    let status = body_json(&http(cport, "GET", "/status", "").await);
    assert_eq!(status["routes"].as_array().unwrap().len(), 2);
    http(
        cport,
        "POST",
        "/remove_drive",
        r#"{"route":"000009ff--new0"}"#,
    )
    .await;
    let status = body_json(&http(cport, "GET", "/status", "").await);
    assert_eq!(status["routes"].as_array().unwrap().len(), 1);

    // reachable:false drops the data listener (Red); control port stays up.
    let resp = http(cport, "POST", "/reachable", r#"{"up":false}"#).await;
    assert!(resp.contains("\"ok\":true"), "reachable: {resp}");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!tcp_ok(dport).await, "data port down after reachable:false");
    assert!(tcp_ok(cport).await, "control port still up");
}
