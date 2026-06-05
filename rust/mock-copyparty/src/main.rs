//! Standalone runner: serve a directory as a mock copyparty server. Reused by
//! UI tests (and later the mock-comma-mcp wrapper). Not used by Rust unit tests,
//! which call `MockServer::spawn` directly.
//!
//! Usage: mock-copyparty <dir> [password]

use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().unwrap_or_else(|| ".".to_string()));
    let password = args.next();

    let server = mock_copyparty::MockServer::spawn_path(root, password)
        .await
        .expect("failed to start mock-copyparty");
    println!("mock-copyparty serving at {}", server.base_url());
    std::future::pending::<()>().await;
}
