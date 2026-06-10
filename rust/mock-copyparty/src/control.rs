//! A supervisor that serves a fixture tree on a fixed **data** port and exposes
//! an HTTP **control** plane on a separate, always-up port to mutate the tree
//! and toggle reachability at runtime.
//!
//! Used by `mock-copyparty --fixture <f> --port <data> --control-port <ctl>` so
//! on-device instrumented tests and Maestro `runScript` can inject state changes
//! (add a drive, append a segment, drop/restore the server) over `adb reverse`.
//! The control port stays up regardless of data reachability — that's why it is
//! a separate port: "bring the data server back up" must always be deliverable.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::{mutate, MockServer, ServeOptions};

/// Owns the served `root` and the data [`MockServer`] on a fixed port; toggles
/// reachability by dropping / re-binding the listener on that same port (the
/// `SO_REUSEADDR` path proven by `RunningDevice` in mock-comma-mcp).
pub struct Supervisor {
    root: PathBuf,
    data_addr: SocketAddr,
    password: Option<String>,
    overrides: HashMap<String, u64>,
    server: Option<MockServer>,
}

impl Supervisor {
    /// Build a supervisor and bring the data server up on `data_addr`.
    pub async fn new(
        root: PathBuf,
        data_addr: SocketAddr,
        password: Option<String>,
        overrides: HashMap<String, u64>,
    ) -> std::io::Result<Self> {
        let mut s = Self {
            root,
            data_addr,
            password,
            overrides,
            server: None,
        };
        s.set_reachable(true).await?;
        Ok(s)
    }

    /// `true` re-binds the data port if it isn't already up; `false` closes the
    /// listening socket (TCP connect refused → Red) and waits for release.
    pub async fn set_reachable(&mut self, up: bool) -> std::io::Result<()> {
        if up {
            if self.server.is_none() {
                self.server = Some(
                    MockServer::spawn_with(
                        self.root.clone(),
                        ServeOptions {
                            addr: Some(self.data_addr),
                            password: self.password.clone(),
                            size_overrides: self.overrides.clone(),
                        },
                    )
                    .await?,
                );
            }
        } else if let Some(srv) = self.server.take() {
            srv.shutdown().await;
        }
        Ok(())
    }

    pub fn reachable(&self) -> bool {
        self.server.is_some()
    }

    /// JSON snapshot: reachability, the data port, and the current route list.
    pub fn status(&self) -> Value {
        json!({
            "reachable": self.reachable(),
            "data_port": self.data_addr.port(),
            "routes": mutate::list_routes(&self.root),
        })
    }
}

type Shared = Arc<Mutex<Supervisor>>;

/// The control-plane router (JSON in, `{ "ok": bool, ... }` out).
pub fn control_router(sup: Shared) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/reachable", post(reachable))
        .route("/add_segment", post(add_segment))
        .route("/add_drive", post(add_drive))
        .route("/remove_drive", post(remove_drive))
        .with_state(sup)
}

/// Bind the control server on `addr` and spawn it; returns once it is listening
/// (so tests can issue requests without racing the bind).
pub async fn serve_control(addr: SocketAddr, sup: Shared) -> std::io::Result<JoinHandle<()>> {
    let listener = TcpListener::bind(addr).await?;
    let app = control_router(sup);
    Ok(tokio::spawn(async move {
        let _ = axum::serve(listener, app.into_make_service()).await;
    }))
}

fn one() -> usize {
    1
}

#[derive(Deserialize)]
struct Reachable {
    up: bool,
}

#[derive(Deserialize)]
struct AddSegment {
    #[serde(default)]
    route: Option<String>,
    #[serde(default = "one")]
    n: usize,
}

#[derive(Deserialize)]
struct AddDrive {
    route: String,
    #[serde(default = "one")]
    segs: usize,
}

#[derive(Deserialize)]
struct RemoveDrive {
    route: String,
}

fn reply(res: std::io::Result<Value>) -> Json<Value> {
    match res {
        Ok(status) => Json(json!({ "ok": true, "status": status })),
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn status(State(sup): State<Shared>) -> Json<Value> {
    Json(sup.lock().await.status())
}

async fn reachable(State(sup): State<Shared>, Json(p): Json<Reachable>) -> Json<Value> {
    let mut s = sup.lock().await;
    let res = s.set_reachable(p.up).await;
    reply(res.map(|_| s.status()))
}

async fn add_segment(State(sup): State<Shared>, Json(p): Json<AddSegment>) -> Json<Value> {
    let s = sup.lock().await;
    let res = mutate::add_segment(&s.root, p.route.as_deref(), p.n);
    reply(res.map(|_| s.status()))
}

async fn add_drive(State(sup): State<Shared>, Json(p): Json<AddDrive>) -> Json<Value> {
    let s = sup.lock().await;
    let res = mutate::add_drive(&s.root, &p.route, p.segs);
    reply(res.map(|_| s.status()))
}

async fn remove_drive(State(sup): State<Shared>, Json(p): Json<RemoveDrive>) -> Json<Value> {
    let s = sup.lock().await;
    let res = mutate::remove_drive(&s.root, &p.route);
    reply(res.map(|_| s.status()))
}
