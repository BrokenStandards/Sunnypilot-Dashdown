# Phase A — Runtime-mutable mock + HTTP control plane

> Phase A of [e2e-and-sync-roadmap.md](e2e-and-sync-roadmap.md). On implementation this file is renamed
> `phase-a-mock-control-plane.md`. Branch `phase-a-mock-control-plane` → PR → CI green → merge.

## Context

The mock harness can't change served state at runtime: `mock-copyparty` fixtures are immutable
temp trees and `mock-comma`'s `set_state` swaps a whole fixture. The later phases need to inject
state changes mid-test — **add a drive, remove a drive, add a segment to an active drive, toggle
reachability** — so tests for live UI refresh and background segment-pickup can exist.

Two facts make this cheap (confirmed by reading the crates):
- `MockServer::handle` walks the served dir **live on every request** ([lib.rs:148-177](rust/mock-copyparty/src/lib.rs)), so
  mutating files in the temp tree is reflected on the next `?ls=j` listing with **no restart**.
- Reachability toggling by dropping/re-binding the listener on a fixed `SO_REUSEADDR` port is
  already proven ([it_mock_harness.rs:24-51](rust/core/tests/it_mock_harness.rs), and `RunningDevice::bring_up/down` in
  [server.rs:62-85](rust/mock-comma-mcp/src/server.rs)).

Outcome: one **mutation core** in the `mock-copyparty` lib, exposed two ways — an HTTP **control
port** on the standalone binary (so on-device tests and Maestro `runScript` drive it over a second
`adb reverse`) and matching **MCP tools** on `mock-comma` (for interactive agent runs). Both call
the same mutation functions (DRY).

## Design

### 1. Mutation core — `rust/mock-copyparty/src/mutate.rs` (new), pure fs ops on a served root
Segment dirs are `routes/<route>--<n>`; the route stem is the dir name with the trailing `--<n>`
stripped (use `rsplit_once("--")` so route ids containing `--`, e.g. `000001a3--c20ba54385`, parse
correctly). Reuse `fixtures::full_segment` (make it `pub`).
- `add_segment(root, route: Option<&str>, n: usize)` — append the next `n` consecutive segments to
  `route` (default = `primary_route`); start at `max_existing_index + 1` (or 0 if none).
- `add_drive(root, route: &str, segs: usize)` — create `route--0 .. route--(segs-1)`, full file set.
- `remove_drive(root, route: &str)` — delete every `routes/<route>--*` dir (models the Comma's own
  low-space auto-prune).
- `primary_route(root) -> Option<String>` — route stem of the lexically-first segment dir.
- `list_routes(root) -> Vec<RouteInfo{route, segments}>` — for the `status` endpoint/tool.
All return `io::Result`; operate under `root/routes/`.

### 2. Supervisor + control router — `rust/mock-copyparty/src/control.rs` (new), in the lib
A `Supervisor` owns the served `root` + the data `MockServer` on a **fixed** data port, and toggles
reachability exactly like `RunningDevice`:
```
pub struct Supervisor { root, data_addr, password, overrides, server: Option<MockServer> }
  new(root, data_addr, password, overrides) -> io::Result<Self>   // brings data server up
  set_reachable(up) -> io::Result<()>                              // shutdown().await / spawn_with
  add_segment / add_drive / remove_drive / reset(fixture) -> io::Result<()>  // call mutate::* on root
  status() -> serde_json::Value                                    // {reachable, data_port, routes}
pub fn control_router(Arc<Mutex<Supervisor>>) -> axum::Router
pub async fn serve_control(addr, Arc<Mutex<Supervisor>>) -> io::Result<JoinHandle<()>>
```
Control endpoints (JSON in, `{ "ok": true, ... }` out), served on the **always-up** control port so
"bring data back up" is always deliverable (the reason it's a separate port):
`POST /reachable {up}` · `POST /add_segment {route?, n?=1}` · `POST /add_drive {route, segs?=1}` ·
`POST /remove_drive {route}` · `POST /reset {fixture}` · `GET /status`.
Uses axum's `Json` extractor + `routing::{get,post}` (the default `json` feature is already on).

### 3. Binary flag — `rust/mock-copyparty/src/main.rs`
Add `--control-port <C>` to the existing `--fixture` mode. When set, **require `--port <F>`** (adb
reverse needs a fixed data port), build the fixture into a kept-alive `TempDir`, construct
`Supervisor` on `F`, `serve_control` on `C`, then `pending()`. Print both URLs. Existing
no-control-port behaviour is unchanged.

### 4. MCP parity — `rust/mock-comma-mcp/src/server.rs`
Add three `#[tool]`s — `add_segment`, `add_drive`, `remove_drive` — that lock the registry,
`get_mut(device_id)`, and call `mock_copyparty::mutate::*` on `dev.temp.path()` (no rebind needed —
the running server serves the mutation live). Param structs derive `Deserialize + JsonSchema` like
the existing ones; each returns `dev.info()` extended with the route list so the result is
assertable. Add a `routes` field to `RunningDevice::info` via `mutate::list_routes`.

## Files
- New: [mutate.rs](rust/mock-copyparty/src/mutate.rs), [control.rs](rust/mock-copyparty/src/control.rs); `pub mod` both in [lib.rs](rust/mock-copyparty/src/lib.rs) (+ `full_segment` → `pub`).
- Edit: [main.rs](rust/mock-copyparty/src/main.rs) (`--control-port`), [server.rs](rust/mock-comma-mcp/src/server.rs) (3 tools + `info.routes`).

## Tests (build complete, test complete)
- **mock-copyparty unit** (`mutate.rs` `#[cfg(test)]` or `tests/it_mutate.rs`): each `mutate::*` creates/
  removes the right dirs (fs assertions); `primary_route`/`list_routes` parse stems with internal `--`.
- **mock-copyparty control** (`tests/it_control.rs`, new): construct `Supervisor` on an ephemeral data
  port → reachable; `set_reachable(false)` → `TcpStream::connect` refused; `(true)` → reachable on the
  **same** port (mirror [it_mcp.rs](rust/mock-comma-mcp/tests/it_mcp.rs) `tcp_ok`). `serve_control` on an ephemeral control port,
  drive it with a tiny raw `tokio::net::TcpStream` HTTP/1.1 POST (`Connection: close`, no new dep) →
  assert the mutation landed on disk.
- **core** ([it_mock_harness.rs](rust/core/tests/it_mock_harness.rs)): the strongest end-to-end — serve a dir, `mutate::add_segment`,
  re-`list_segments()` via `CopypartyClient`, assert the drive's `segment_count` grew; `add_drive`/
  `remove_drive` change the drive count after `group_segments`.
- **MCP** ([it_mcp.rs](rust/mock-comma-mcp/tests/it_mcp.rs)): assert the three new tools are advertised; provision `single_drive`,
  `add_segment`, assert `status` routes show the incremented segment count.

## Verification
- `cargo test -p mock-copyparty -p mock-comma-mcp -p core` green; `cargo clippy` clean; `cargo fmt`.
- Manual: `cargo run -p mock-copyparty -- --fixture single_drive --port 8099 --control-port 8098`,
  then `curl -s localhost:8098/status`, `curl -XPOST localhost:8098/add_segment -d '{}'`,
  `curl -s 'localhost:8099/routes/?ls=j'` shows the new segment dir; `curl -XPOST
  localhost:8098/reachable -d '{"up":false}'` → `curl localhost:8099` refused.
- Interactive MCP: `provision_device` then the new `add_segment` tool, confirm `status` route count.

## Commit
Branch `phase-a-mock-control-plane`; single milestone commit once green; PR; merge after CI.
Rename this file to `phase-a-mock-control-plane.md` as part of the work.
