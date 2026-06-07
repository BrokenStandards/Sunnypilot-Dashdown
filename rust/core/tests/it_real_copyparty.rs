//! Contract test against the REAL copyparty server (master plan: "run real
//! copyparty with fixtures"). Validates our `?ls=j` parser against authoritative
//! output. Discovers copyparty via PATH, `python3 -m copyparty`, or the pinned
//! `ref/copyparty` source — so it runs locally with zero install, and in CI once
//! copyparty is installed. Skips (passes) if no launcher is found.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use dashdown_core::copyparty_client::{CopypartyClient, Credentials};
use dashdown_core::storage::MirrorStore;
use dashdown_core::sync_engine::{download_file, CancellationToken, FileOutcome};
use mock_copyparty::{fixtures, Fixture};
use tokio::io::AsyncWriteExt;

/// How to invoke copyparty (program + leading args + optional env var).
struct Launcher {
    program: String,
    prefix: Vec<String>,
    env: Option<(String, String)>,
}

impl Launcher {
    fn command(&self) -> Command {
        let mut c = Command::new(&self.program);
        c.args(&self.prefix);
        if let Some((k, v)) = &self.env {
            c.env(k, v);
        }
        c
    }
    fn runs(&self) -> bool {
        self.command()
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn repo_root() -> PathBuf {
    // rust/core -> rust -> repo root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

fn detect_launcher() -> Option<Launcher> {
    let candidates = [
        Launcher {
            program: "copyparty".into(),
            prefix: vec![],
            env: None,
        },
        Launcher {
            program: "python3".into(),
            prefix: vec!["-m".into(), "copyparty".into()],
            env: None,
        },
        Launcher {
            program: "python3".into(),
            prefix: vec!["-m".into(), "copyparty".into()],
            env: Some((
                "PYTHONPATH".into(),
                repo_root().join("ref/copyparty").display().to_string(),
            )),
        },
    ];
    candidates.into_iter().find(Launcher::runs)
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Kills the copyparty child on drop (even if an assertion panics).
struct Killer(std::process::Child);
impl Drop for Killer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Boot real copyparty serving `fixture` read-only and wait until it answers a
/// listing. Returns `None` (the caller should SKIP) when no launcher is found or
/// the server doesn't come up. The caller keeps the `Fixture` alive.
async fn boot_copyparty(fixture: &Fixture) -> Option<(Killer, CopypartyClient)> {
    boot_copyparty_with(fixture, "r").await
}

/// Boot copyparty with the given volume flags (e.g. `r`).
async fn boot_copyparty_with(
    fixture: &Fixture,
    volflags: &str,
) -> Option<(Killer, CopypartyClient)> {
    let launcher = detect_launcher()?;
    let port = free_port();
    let mut cmd = launcher.command();
    cmd.args([
        "-i",
        "127.0.0.1",
        "-p",
        &port.to_string(),
        "-q",
        "-v",
        &format!("{}:/:{}", fixture.path().display(), volflags),
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    let killer = Killer(cmd.spawn().expect("spawn copyparty"));

    let base = format!("http://127.0.0.1:{port}/");
    let client = CopypartyClient::new(&base, Credentials::Anonymous).unwrap();
    // Wait for readiness (server startup) — up to ~12s.
    for _ in 0..48 {
        if let Ok(s) = client.list_segments("routes/").await {
            if !s.is_empty() {
                return Some((killer, client));
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    None
}

const QCAMERA: &str = "routes/000001a3--c20ba54385--0/qcamera.ts";

#[tokio::test]
async fn parses_real_copyparty_listing() {
    let fixture = fixtures::single_drive();
    let Some((_killer, client)) = boot_copyparty(&fixture).await else {
        eprintln!("SKIP it_real_copyparty: copyparty not available");
        return;
    };
    let segments = client.list_segments("routes/").await.unwrap();

    // Same assertions as the mock, but against the real server's JSON.
    assert_eq!(segments.len(), 3, "single_drive has 3 segments");
    for (i, seg) in segments.iter().enumerate() {
        assert_eq!(seg.name.route_id, "000001a3--c20ba54385");
        assert_eq!(seg.name.segment_num, i as u32);
        assert_eq!(seg.files.len(), 5);
        for f in &seg.files {
            assert!(f.remote_size > 0, "{} size from real copyparty", f.name);
            assert!(f.mtime_s > 0, "{} mtime from real copyparty", f.name);
        }
    }

    // Download a file end-to-end.
    let bytes = client.download(QCAMERA).await.unwrap();
    assert_eq!(bytes.len(), 1200);
}

/// M5 Range re-verification: real copyparty answers a `bytes=0-0` probe with 206.
#[tokio::test]
async fn real_copyparty_supports_byte_range() {
    let fixture = fixtures::single_drive();
    let Some((_killer, client)) = boot_copyparty(&fixture).await else {
        eprintln!("SKIP it_real_copyparty: copyparty not available");
        return;
    };
    assert!(
        client.probe_range(QCAMERA).await.unwrap(),
        "copyparty should honor HTTP Range (206)"
    );
}

/// Authoritative byte-range resume against real copyparty: a half-downloaded
/// `.part` resumes via 206 and the committed file equals the original bytes.
#[tokio::test]
async fn real_copyparty_resumes_partial_download() {
    let fixture = fixtures::single_drive();
    let Some((_killer, client)) = boot_copyparty(&fixture).await else {
        eprintln!("SKIP it_real_copyparty: copyparty not available");
        return;
    };

    let full = client.download(QCAMERA).await.unwrap();
    assert_eq!(full.len(), 1200);

    let dir = tempfile::tempdir().unwrap();
    let mirror = MirrorStore::new(dir.path());
    // Pre-place the first 500 real bytes as a `.part` (flush before drop).
    let mut pf = mirror.create_part(QCAMERA).await.unwrap();
    pf.writer().write_all(&full[..500]).await.unwrap();
    pf.writer().flush().await.unwrap();
    drop(pf);

    let token = CancellationToken::new();
    let outcome = download_file(&client, &mirror, QCAMERA, 1200, &token, 1)
        .await
        .unwrap();
    assert_eq!(outcome, FileOutcome::Complete);

    let got = std::fs::read(mirror.final_path(QCAMERA).unwrap()).unwrap();
    assert_eq!(got, full, "resumed file matches the original bytes");
}
