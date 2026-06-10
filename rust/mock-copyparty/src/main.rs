//! Standalone runner: serve a mock copyparty server. Reused by UI tests, the
//! `mock-comma-mcp` wrapper, and the Android adb-reverse fixture checks. Not used
//! by Rust unit tests, which call `MockServer::spawn` directly.
//!
//! Usage:
//!   mock-copyparty <dir> [password]                          # serve a directory (ephemeral port)
//!   mock-copyparty --fixture <name> [--port P] [--password X] # serve a named fixture
//!
//! Fixtures: single_drive | gap_split | gap_index | partial | size_mismatch.
//! `--port` binds a fixed port (so a device can reach it via `adb reverse`); omit
//! for an ephemeral port. The chosen base URL is printed to stdout.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use mock_copyparty::control::{serve_control, Supervisor};
use mock_copyparty::{fixtures, MockServer, ServeOptions};
use tokio::sync::Mutex;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if let Some(i) = args.iter().position(|a| a == "--fixture") {
        let name = args
            .get(i + 1)
            .map(String::as_str)
            .unwrap_or("single_drive");
        let port = flag(&args, "--port").and_then(|s| s.parse::<u16>().ok());
        let control_port = flag(&args, "--control-port").and_then(|s| s.parse::<u16>().ok());
        let password = flag(&args, "--password");
        let fixture = match name {
            "gap_split" => fixtures::gap_split(),
            "gap_index" => fixtures::gap_index(),
            "partial" => fixtures::partial(),
            "size_mismatch" => fixtures::size_mismatch(),
            "single_drive" => fixtures::single_drive(),
            other => panic!("unknown fixture: {other}"),
        };

        // Supervisor mode: a fixed data port served alongside an always-up HTTP
        // control port that mutates the tree / toggles reachability at runtime.
        if let Some(cport) = control_port {
            let dport =
                port.expect("--control-port requires --port (a fixed data port for adb reverse)");
            let data_addr = SocketAddr::from(([127, 0, 0, 1], dport));
            let ctl_addr = SocketAddr::from(([127, 0, 0, 1], cport));
            let root = fixture.dir.path().to_path_buf();
            let overrides = fixture.size_overrides.clone();
            let sup = Arc::new(Mutex::new(
                Supervisor::new(root, data_addr, password, overrides)
                    .await
                    .expect("failed to start data server"),
            ));
            serve_control(ctl_addr, sup)
                .await
                .expect("failed to start control server");
            // `fixture` (its TempDir) must outlive the served root.
            let _keep = fixture;
            println!(
                "mock-copyparty serving fixture '{name}' at http://127.0.0.1:{dport}/ \
                 (control http://127.0.0.1:{cport}/)"
            );
            std::future::pending::<()>().await;
            return;
        }

        let addr: Option<SocketAddr> = port.map(|p| SocketAddr::from(([127, 0, 0, 1], p)));
        let server = MockServer::spawn_with(
            fixture.dir.path().to_path_buf(),
            ServeOptions {
                addr,
                password,
                size_overrides: fixture.size_overrides.clone(),
            },
        )
        .await
        .expect("failed to start mock-copyparty");
        // `fixture` (its TempDir) and `server` stay alive until the process exits.
        println!(
            "mock-copyparty serving fixture '{name}' at {}",
            server.base_url()
        );
        std::future::pending::<()>().await;
        return;
    }

    // Directory mode.
    let mut it = args.into_iter();
    let root = PathBuf::from(it.next().unwrap_or_else(|| ".".to_string()));
    let password = it.next();
    let server = MockServer::spawn_path(root, password)
        .await
        .expect("failed to start mock-copyparty");
    println!("mock-copyparty serving at {}", server.base_url());
    std::future::pending::<()>().await;
}

/// Value following `name` in `args`, if present (`--flag value`).
fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
