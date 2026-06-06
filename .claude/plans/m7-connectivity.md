# M7 — Connectivity (TCP reachability + dot logic)

## Context

Phase A milestones M0–M6 are merged: the Rust core indexes a device, groups drives, mirrors
files crash-safely, downloads/resumes, and manages retention + remote deletion. The app shows a
per-device status **dot**, but nothing computes it yet. M7 adds the connectivity check that
drives that dot, and is the last milestone before the UniFFI surface (M8) wraps the whole core
for the native shells.

**Dot semantics (verbatim from the master plan):** **Green** = reachable (TCP connect to the
active `(ip,port)` ok) & idle; **Blue** = reachable & a download active for this device; **Red**
= unreachable. Reachability uses `TcpStream::connect` with a timeout — **not** ICMP ping (raw
sockets are blocked on mobile; an unprivileged TCP connect to the copyparty port is the exact
"can I talk to it" signal).

This milestone delivers the **core** check + types + tests only. M8 will expose it on the
`AppCore` UniFFI facade as `check_connectivity(device_id)` returning `DeviceConnectivity`. Do
**not** add any UniFFI derives/exports or id-based wrappers here.

## Verified facts (grounding)

- **IP selection already exists:** `Device::active_ip()` ([model/mod.rs:338](rust/core/src/model/mod.rs#L338))
  returns `hotspot_ip`/`wifi_ip` per `active_mode`; `base_url()` builds the URL. The probe target
  is `(device.active_ip(), device.port)`.
- **"Download active" signal already exists:** the `download_job` table carries `state='running'`
  during a live download (`upsert_job` writes it, [db/mod.rs:381](rust/core/src/db/mod.rs#L381);
  `set_job_state` clears it on a terminal state; M5 `reconcile` reclaims a crash-stale `running`
  → Failed). So a DB `EXISTS` query is the right, testable signal — and `upsert_job` alone yields
  `state='running'`, so the Blue test needs no real transfer.
- **`ConnMode{Hotspot,Wifi}`** and `JobState` live in [model/mod.rs](rust/core/src/model/mod.rs)
  with an `as_str`/`parse` pattern (parse exists because they round-trip through TEXT columns).
  `ConnDot` is computed live and never persisted → `as_str` only, no `parse`.
- **No periodic-task infra; request-driven.** M7 just provides the method; the native layer
  (Phase B) polls it. No scheduler.
- **No new `CoreError` variant needed** — unreachable is a normal `false`, not an error.
- **Module convention:** master plan line 133 specifies `connectivity/mod.rs` (directory form),
  matching every other subsystem (`db/`, `storage/`, `sync_engine/`, …). Use the directory form.
- **Deps:** core's tokio is `features = ["io-util","fs"]`; `tcp_reachable` needs `net` + `time`
  (`tokio::net::TcpStream`, `tokio::time::timeout`). tokio `net` is already proven on the Android/
  iOS targets (reqwest/hyper pull it transitively). dev-deps tokio already has `macros`/`time`/
  `rt-multi-thread` for `#[tokio::test]`.
- **Testing:** `MockServer::spawn`/`addr()` ([mock-copyparty/src/lib.rs:58](rust/mock-copyparty/src/lib.rs#L58))
  gives a reachable TCP endpoint; an inline `free_port()` (bound-then-dropped, as in
  [it_real_copyparty.rs:77](rust/core/tests/it_real_copyparty.rs#L77)) gives a closed port →
  fast connection-refused → Red. All three dots test deterministically and fast. The genuine
  "SYN accepted, never answered" timeout path isn't hermetically reproducible; a short-timeout
  closed-port test proves the bound is wired.

## Design

### 1. `ConnDot` enum — `rust/core/src/model/mod.rs`
Add next to `ConnMode` (derives Debug/Clone/Copy/PartialEq/Eq; `as_str()` → "green"/"blue"/"red";
no `parse()` — not persisted). One `as_str` unit test in the existing model `tests` block.

### 2. New module `rust/core/src/connectivity/mod.rs`
```rust
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// True iff a TCP connection to (host, port) completes within `timeout`. Any
/// failure (refused, unreachable, DNS error, timeout) → false. The timeout also
/// bounds DNS resolution (it happens inside the wrapped connect future). Pure.
pub async fn tcp_reachable(host: &str, port: u16, timeout: Duration) -> bool {
    matches!(
        tokio::time::timeout(timeout, tokio::net::TcpStream::connect((host, port))).await,
        Ok(Ok(_))
    )
}

/// Result of one device's connectivity check (M8's check_connectivity returns this).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceConnectivity { pub dot: ConnDot, pub reachable: bool, pub downloading: bool }
```
Register with `pub mod connectivity;` in [lib.rs](rust/core/src/lib.rs).

### 3. `Repo::has_active_job` — `rust/core/src/db/mod.rs`
In the `// ---- download jobs ----` section, a single-query EXISTS reusing the enum token:
```rust
pub fn has_active_job(&self, device_id: i64) -> Result<bool> {
    let conn = self.conn()?;
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM download_job WHERE device_id=?1 AND state=?2)",
        params![device_id, JobState::Running.as_str()],
        |r| r.get(0),
    )?;
    Ok(exists)
}
```
(`JobState` already imported; rusqlite maps the 0/1 to `bool` like `recording`/`preserved`.)

### 4. `SyncEngine::check_connectivity` — `rust/core/src/sync_engine/mod.rs`
```rust
pub async fn check_connectivity(&self, device: &Device) -> Result<DeviceConnectivity> {
    let reachable = connectivity::tcp_reachable(
        device.active_ip(), device.port, connectivity::DEFAULT_CONNECT_TIMEOUT).await;
    if !reachable {
        return Ok(DeviceConnectivity { dot: ConnDot::Red, reachable: false, downloading: false });
    }
    let device_id = device.id;
    let downloading = db(self.repo.clone(), move |r| r.has_active_job(device_id)).await?;
    let dot = if downloading { ConnDot::Blue } else { ConnDot::Green };
    Ok(DeviceConnectivity { dot, reachable: true, downloading })
}
```
Red short-circuits (no DB query when unreachable). Reuses `Device::active_ip()` and the private
`db()` spawn_blocking helper. "Active" = a running **download job**; a brief, untracked `sync_now`
index refresh is intentionally not counted (matches the "Blue while downloading" contract — noted
in a doc comment). Add `use crate::connectivity::{self, DeviceConnectivity};` and `ConnDot` to the
existing `model` import.

### 5. Cargo — `rust/core/Cargo.toml`
`tokio = { workspace = true, features = ["io-util", "fs", "net", "time"] }`
(via `cargo add tokio -p dashdown-core --features net,time`, per the project's cargo-for-deps
convention — merges into the existing array).

## Files
- **edit** [rust/core/src/model/mod.rs](rust/core/src/model/mod.rs) — `ConnDot` + as_str test.
- **new**  rust/core/src/connectivity/mod.rs — `tcp_reachable`, `DEFAULT_CONNECT_TIMEOUT`, `DeviceConnectivity` (+ unit tests).
- **edit** [rust/core/src/lib.rs](rust/core/src/lib.rs) — `pub mod connectivity;`.
- **edit** [rust/core/src/db/mod.rs](rust/core/src/db/mod.rs) — `has_active_job`.
- **edit** [rust/core/src/sync_engine/mod.rs](rust/core/src/sync_engine/mod.rs) — `check_connectivity`.
- **edit** [rust/core/Cargo.toml](rust/core/Cargo.toml) — tokio `net`,`time`.
- **new**  rust/core/tests/it_connectivity.rs — up/down + dot matrix + Blue-while-downloading.

## Verification (TDD — tests first)

**Unit — `connectivity/mod.rs`:**
- `tcp_reachable` → true to a live `tokio::net::TcpListener` on 127.0.0.1:0 (kept alive during the probe).
- → false fast to a `free_port()` bound-then-dropped (connection refused).
- → false to a closed port with a short timeout (e.g. 200ms) — proves the bound is wired without hanging.

**Unit — `model`:** `ConnDot::as_str` mappings.

**Integration — `it_connectivity.rs`** (mirror the `it_download.rs` `setup()`/`device_at` harness;
`MockServer` for "up", inline `free_port()` for "down"):
- **green_when_reachable_and_idle**: server up, no jobs → `dot == Green`, reachable, !downloading.
- **red_when_unreachable**: device pointed at an unbound `free_port()` → `dot == Red`, !reachable
  (and/or: Green then `drop(srv)` then re-probe → Red, covering the up→down transition).
- **blue_while_downloading**: server up; `repo.upsert_job(dev.id, "drive", 1, 100)` seeds
  `state='running'` (no real transfer) → `dot == Blue`, downloading; then
  `set_job_state(.., JobState::Complete, None)` → back to `Green`. Directly exercises `has_active_job`.

**Gates:** `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`;
`cargo test --workspace`. Then commit, PR, watch CI (build/test/cross-compile/bindgen +
claude-review), squash-merge to `main`. Rename this plan file to `.claude/plans/m7-connectivity.md`
during implementation.

## Risks / verify during implementation
1. **`ToSocketAddrs` for `(&str, u16)`** — tokio implements it (resolves via its blocking DNS pool);
   our host is normally a literal IPv4, so trivial. Confirm it compiles on the pinned tokio.
2. **Timeout bounds DNS** — `connect` is lazy; resolution happens inside the future, so
   `time::timeout` bounds a hung resolver. Sanity-check against tokio's `TcpStream::connect`.
3. **Stale `running` job** — a job left `running` by a crash reads as Blue until the next
   `reconcile`/`sync_now` reclaims it (self-healing). Document in the `check_connectivity` doc
   comment; the heavier in-memory in-flight registry is out of scope.
4. **tokio `net` on Android/iOS** — already pulled transitively by the HTTP stack; re-run the
   4-ABI cross-compile after the Cargo change to confirm linking. No new system deps (TCP connect
   is unprivileged — the whole reason for TCP over ICMP).
5. **No M8 leakage** — keep `check_connectivity` on `SyncEngine` taking `&Device`; no
   `#[uniffi::export]`, no `AppCore`, no id-based wrapper. `DeviceConnectivity` stays a plain
   struct (UniFFI `Record` derive comes in M8).
