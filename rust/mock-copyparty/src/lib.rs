//! Hermetic copyparty-compatible fixture server for tests (and later UI tests).
//!
//! Serves `?ls=j` JSON directory listings (matching copyparty's shape: `dirs`/
//! `files` with `href`/`sz`/`ts`, and **no** `name` field) and plain `GET` file
//! downloads from a directory tree, with optional `pw` auth (query or `PW:`
//! header → 401/403). Bind to an ephemeral port; drop to stop.

pub mod control;
pub mod fixtures;
pub mod mutate;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use serde_json::json;
use tokio::net::{TcpListener, TcpSocket};
use tokio::task::JoinHandle;

pub use fixtures::Fixture;

#[derive(Clone)]
struct AppState {
    root: PathBuf,
    password: Option<String>,
    /// Relative-path → advertised `sz`, overriding on-disk size in listings.
    /// Used to fabricate a size-mismatch (listing claims N, the GET still returns
    /// the real M bytes) so the core classifies the downloaded file `SizeMismatch`.
    size_overrides: Arc<HashMap<String, u64>>,
}

/// Options for [`MockServer::spawn_with`].
#[derive(Default)]
pub struct ServeOptions {
    /// Bind address. `None` ⇒ ephemeral `127.0.0.1:0`. `Some(_)` ⇒ bind that exact
    /// port with `SO_REUSEADDR` so a just-closed port can be re-bound immediately
    /// (used by mock-comma-mcp to toggle reachability on a stable port).
    pub addr: Option<SocketAddr>,
    pub password: Option<String>,
    pub size_overrides: HashMap<String, u64>,
}

/// A running mock server. Dropping it stops the server (and frees any owned
/// fixture temp dir).
pub struct MockServer {
    base_url: String,
    addr: SocketAddr,
    handle: JoinHandle<()>,
    _root: Option<tempfile::TempDir>,
}

impl MockServer {
    /// Serve an existing directory on an ephemeral port (no size overrides).
    pub async fn spawn_path(root: PathBuf, password: Option<String>) -> std::io::Result<Self> {
        Self::spawn_with(
            root,
            ServeOptions {
                password,
                ..Default::default()
            },
        )
        .await
    }

    /// Serve a [`Fixture`], keeping its temp dir alive for the server's lifetime.
    /// Carries the fixture's `size_overrides` (for the size-mismatch fixture).
    pub async fn spawn(fixture: Fixture, password: Option<String>) -> std::io::Result<Self> {
        let root = fixture.dir.path().to_path_buf();
        let mut srv = Self::spawn_with(
            root,
            ServeOptions {
                password,
                size_overrides: fixture.size_overrides.clone(),
                ..Default::default()
            },
        )
        .await?;
        srv._root = Some(fixture.dir);
        Ok(srv)
    }

    /// Serve `root` with explicit [`ServeOptions`] — the general constructor.
    pub async fn spawn_with(root: PathBuf, opts: ServeOptions) -> std::io::Result<Self> {
        let listener = match opts.addr {
            Some(addr) => {
                // Bind a specific port with SO_REUSEADDR so reachability toggles
                // can re-bind the same port immediately after the prior listener
                // is dropped (no TIME_WAIT block on localhost).
                let socket = if addr.is_ipv4() {
                    TcpSocket::new_v4()?
                } else {
                    TcpSocket::new_v6()?
                };
                socket.set_reuseaddr(true)?;
                socket.bind(addr)?;
                socket.listen(1024)?
            }
            None => TcpListener::bind(("127.0.0.1", 0)).await?,
        };
        let addr = listener.local_addr()?;
        let state = AppState {
            root,
            password: opts.password,
            size_overrides: Arc::new(opts.size_overrides),
        };
        let app = Router::new().fallback(handle).with_state(state);
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app.into_make_service()).await;
        });
        Ok(MockServer {
            base_url: format!("http://{addr}/"),
            addr,
            handle,
            _root: None,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stop the server and **wait** for its listener to close before returning,
    /// so the caller can immediately re-bind the same port (the plain `Drop`
    /// `abort()` is asynchronous, racing a same-port rebind → `EADDRINUSE`).
    pub async fn shutdown(mut self) {
        self.handle.abort();
        // `&mut JoinHandle` is a Future (JoinHandle: Unpin); awaiting the aborted
        // task completes only after its future — and thus the `TcpListener` — is
        // dropped, releasing the port.
        let _ = (&mut self.handle).await;
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn handle(State(state): State<AppState>, uri: Uri, headers: HeaderMap) -> Response {
    if let Some(rejection) = check_auth(&state, &uri, &headers) {
        return rejection;
    }

    let rel = percent_encoding::percent_decode_str(uri.path())
        .decode_utf8_lossy()
        .trim_start_matches('/')
        .to_string();
    let Some(target) = safe_join(&state.root, &rel) else {
        return (StatusCode::FORBIDDEN, "bad path").into_response();
    };

    let is_ls = uri
        .query()
        .map(|q| q.split('&').any(|kv| kv == "ls" || kv.starts_with("ls=")))
        .unwrap_or(false);

    if is_ls {
        if !target.is_dir() {
            return (StatusCode::NOT_FOUND, "not a directory").into_response();
        }
        return listing_response(&state.root, &target, &state.size_overrides);
    }

    match std::fs::read(&target) {
        Ok(bytes) => (StatusCode::OK, bytes).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "no such file").into_response(),
    }
}

/// Returns `Some(error_response)` if the request is unauthorized, else `None`
/// (authorized, or the server requires no password).
fn check_auth(state: &AppState, uri: &Uri, headers: &HeaderMap) -> Option<Response> {
    let expected = state.password.as_ref()?; // no password configured ⇒ authorized
    let from_query = uri.query().and_then(|q| {
        q.split('&')
            .find_map(|kv| kv.strip_prefix("pw=").map(|v| v.to_string()))
    });
    let provided = from_query.or_else(|| {
        headers
            .get("PW")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    });
    match provided {
        None => Some((StatusCode::UNAUTHORIZED, "authentication required").into_response()),
        Some(p) if &p == expected => None,
        Some(_) => Some((StatusCode::FORBIDDEN, "forbidden").into_response()),
    }
}

fn listing_response(root: &Path, dir: &Path, overrides: &HashMap<String, u64>) -> Response {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (StatusCode::NOT_FOUND, "").into_response();
    };
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        let ts = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if meta.is_dir() {
            dirs.push(
                json!({"lead": "-", "href": format!("{name}/"), "sz": 0, "ext": "---", "ts": ts}),
            );
        } else {
            let ext = name.rsplit('.').next().unwrap_or("").to_string();
            // Advertise the override size if one is set for this file's path
            // (relative to root, forward-slashed); else its true on-disk size.
            let rel = entry
                .path()
                .strip_prefix(root)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| name.clone());
            let sz = overrides.get(&rel).copied().unwrap_or(meta.len());
            files.push(json!({"lead": "-", "href": name, "sz": sz, "ext": ext, "ts": ts}));
        }
    }
    // Mirror copyparty: name/dt are NOT serialized; extra keys are present.
    let body = json!({"dirs": dirs, "files": files, "taglist": [], "acct": "*", "perms": ["read"]});
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// Join `rel` onto `root`, rejecting any `..` traversal.
fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut p = root.to_path_buf();
    for comp in rel.split('/') {
        match comp {
            "" | "." => continue,
            ".." => return None,
            c => p.push(c),
        }
    }
    Some(p)
}
