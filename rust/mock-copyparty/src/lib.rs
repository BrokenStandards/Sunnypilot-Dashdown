//! Hermetic copyparty-compatible fixture server for tests (and later UI tests).
//!
//! Serves `?ls=j` JSON directory listings (matching copyparty's shape: `dirs`/
//! `files` with `href`/`sz`/`ts`, and **no** `name` field) and plain `GET` file
//! downloads from a directory tree, with optional `pw` auth (query or `PW:`
//! header → 401/403). Bind to an ephemeral port; drop to stop.

pub mod fixtures;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub use fixtures::Fixture;

#[derive(Clone)]
struct AppState {
    root: PathBuf,
    password: Option<String>,
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
    /// Serve an existing directory.
    pub async fn spawn_path(root: PathBuf, password: Option<String>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let state = AppState { root, password };
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

    /// Serve a [`Fixture`], keeping its temp dir alive for the server's lifetime.
    pub async fn spawn(fixture: Fixture, password: Option<String>) -> std::io::Result<Self> {
        let root = fixture.dir.path().to_path_buf();
        let mut srv = Self::spawn_path(root, password).await?;
        srv._root = Some(fixture.dir);
        Ok(srv)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
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
        return listing_response(&target);
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

fn listing_response(dir: &Path) -> Response {
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
            files.push(json!({"lead": "-", "href": name, "sz": meta.len(), "ext": ext, "ts": ts}));
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
