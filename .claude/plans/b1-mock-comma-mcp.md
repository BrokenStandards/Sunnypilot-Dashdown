# B1 — `mock-comma-mcp` + hermetic test harness

## Context

Phase B builds the native iOS/Android shells; their UI must be tested **hermetically and
deterministically** — no real Comma device, no internet. B0 proved the binding pipeline on both
platforms. **B1 builds the test backbone both UI phases (B2/B3) depend on**, per the master plan's
"MCP we will develop": a thin **`mock-comma-mcp`** MCP server wrapping the in-repo `mock-copyparty`
axum fixture, exposing tools the agent (Claude) calls during agentic UI runs to **provision device
fixtures**, **inject states** (single-drive / gap-split / partial / **size-mismatch**, the new one),
and **toggle reachability** up/down to drive the green/blue/red connectivity dot. It also finishes
the deferred test tooling (Maestro, mobile-mcp). Built before B2/B3 because both consume it.

Decisions locked with the user: **rmcp** (official Rust MCP SDK) for the server; **install +
smoke-verify Maestro + mobile-mcp in B1**.

### What exists today (verified)
- `rust/mock-copyparty/` (`lib.rs`/`fixtures.rs`/`main.rs`): `MockServer::spawn_path(root, password)`
  binds `127.0.0.1:0`, spawns `axum::serve`; `spawn(fixture, password)` owns the `TempDir`; `Drop`
  aborts the task (the *only* lifecycle control). One fallback `handle` serves `?ls=j`, file GET
  (no Range), WebDAV DELETE. **Listing `sz` = on-disk `meta.len()`** ([lib.rs:176](rust/mock-copyparty/src/lib.rs#L176)).
  Fixtures `single_drive`/`gap_split`(two routes)/`partial` build real temp-dir trees.
- Core state detection (the MCP must produce these):
  - **Reachability** = bare `TcpStream::connect(host,port)` w/ 2s timeout ([connectivity/mod.rs](rust/core/src/connectivity/mod.rs)).
    A 503 / not-accepting still completes the TCP handshake → **looks reachable**; only **closing the
    listening socket** yields Red.
  - `classify_file`: committed final with `local_size == remote_size` → Complete, else **SizeMismatch**;
    `.part` → InProgress; absent → Missing ([resume.rs:19-34](rust/core/src/sync_engine/resume.rs#L19)).
    `drive_status` aggregates → NotDownloaded / **Partial** / Complete ([resume.rs:43-77](rust/core/src/sync_engine/resume.rs#L43)).
  - Drive grouping splits on **route change or segment-index break** (`drive_grouping`).
- No `.mcp.json` exists yet; root workspace `Cargo.toml` members = `rust/core`, `rust/bindgen`,
  `rust/mock-copyparty`.

---

## Approach

Two crates ship together: **extend `mock-copyparty` (lib)** with the two missing capabilities, then
**build `mock-comma-mcp`** (a thin rmcp server) that owns a registry of running devices and exposes
the tools. Register it in `.mcp.json`. Install + smoke-verify Maestro & mobile-mcp.

### 1. Extend `rust/mock-copyparty` (lib)

- **Fixed-port binding (for reachability toggle).** Add `MockServer::spawn_with(root: PathBuf, opts:
  ServeOptions)` where `ServeOptions { addr: Option<SocketAddr>, password: Option<String>,
  size_overrides: HashMap<String, u64> }`. `addr=None` → ephemeral (today's behavior); `addr=Some(_)`
  → bind that exact port with **SO_REUSEADDR** (via `socket2`, or `TcpSocket::reuseaddr`) so a closed
  port can be re-bound immediately. Keep `spawn_path`/`spawn` as thin wrappers (default opts) — **no
  change to the ~10 existing test call sites**.
- **Size-mismatch (listing-lie).** Thread `size_overrides` into `AppState`; in `listing_response`,
  use `overrides.get(name).copied().unwrap_or(meta.len())` for `sz`. **The GET handler is unchanged**
  — it returns the real on-disk bytes, so an advertised `sz=N` over a real `M`-byte file makes the
  downloaded file `M ≠ remote_size N` → core classifies **SizeMismatch** → drive **Partial**.
- **New fixtures** in `fixtures.rs`: `size_mismatch()` (one segment; `qcamera.ts` of M bytes + an
  overrides entry claiming N≠M) and `gap_index()` (one route, segments 0,1,3 — missing 2 → index-break
  split, truer to the plan's "1-min-gap" than two routes). Carry overrides on the fixture (e.g.
  `Fixture { dir, size_overrides }`, default empty) and pass them through `spawn`.
- Unit/integration tests for: listing reports the overridden size; `gap_index` groups into two drives
  (`group_segments`); `spawn_with(addr=fixed)` then drop → `tcp_reachable` false → re-`spawn_with` same
  port → reachable true.

### 2. New crate `rust/mock-comma-mcp` (binary, rmcp)

Add to root `Cargo.toml` `members`. Deps (via `cargo add`, latest): `rmcp` (features for server +
macros + stdio transport), `tokio`, `serde`, `serde_json`, `schemars`, `mock-copyparty` (path),
`tempfile`. **[RE-VERIFY]** the exact `rmcp` API (macro names, `Parameters<T>`, the `serve(stdio())`
entrypoint) against `cargo doc` — the exploration snippets are directional, not copy-paste.

- **Registry:** a server struct holding `tokio::sync::Mutex<HashMap<String, RunningDevice>>` where
  `RunningDevice { fixture: FixtureKind, temp: TempDir, port: u16, password: Option<String>,
  server: Option<MockServer>, reachable: bool }`. The `TempDir` is owned by `RunningDevice` (survives
  reachability toggles); `server` is dropped/recreated to toggle reachability on the fixed `port`.
- **Port model:** per-device. On provision, `spawn_with(addr=None)` to discover a free port, record it;
  all subsequent (re)binds use `addr=Some(127.0.0.1:port)` so the URL the app is configured with stays
  valid across toggles.
- **Tools** (`#[tool]`):
  | tool | input | output | behavior |
  |---|---|---|---|
  | `provision_device` | `device_id`, `fixture`, `password?` | `{device_id, base_url, host, port, reachable}` | build tree → `spawn_with(fixed port)` → register (replace if exists) |
  | `set_state` | `device_id`, `fixture` | `{base_url, port}` | rebuild tree on the **same** port |
  | `set_reachable` | `device_id`, `reachable` | `{reachable, base_url}` | `false` → drop `MockServer` (port closes → Red); `true` → re-`spawn_with` same port |
  | `status` | `device_id?` | `{devices:[…]}` | snapshot for assertions |
  | `teardown` | `device_id?` | `{torn_down:[…]}` | drop servers + temp dirs |

  `fixture` ∈ {`single_drive`,`gap_split`,`partial`,`size_mismatch`} (serde-renamed). `host` =
  `127.0.0.1`; document that the **Android emulator** reaches it at `10.0.2.2:<port>` (iOS sim uses
  `127.0.0.1:<port>`). Blue-vs-Green is **not** an MCP concern (core derives it from reachable +
  active job).

### 3. Register the MCP — `.mcp.json` (repo root, new)
```jsonc
{ "mcpServers": { "mock-comma": {
  "command": "cargo", "args": ["run", "-q", "-p", "mock-comma-mcp"], "env": { "RUST_LOG": "info" } } } }
```
(`cargo run` compiles once per session then caches; revisit a prebuilt release binary if startup
latency bites.) Optionally add a `mobile-mcp` entry here too (see §4).

### 4. Test tooling — install + smoke-verify (B1)
- **Maestro:** `curl -fsSL "https://get.maestro.mobile.dev" | bash` → `~/.maestro/bin/maestro`
  (needs JDK 17 + adb — both present). Verify `maestro --version`. **[RE-VERIFY]** install URL.
- **mobile-mcp** (mobile-next): run via `npx -y @mobilenext/mobile-mcp@latest` (stdio). Verify it
  launches and, with the `dashdown-b0` emulator booted, can see the device (`adb devices`).
  **[RE-VERIFY]** npm scope + that `node`/`npx` is installed. Add to `.mcp.json` if it composes cleanly.
- B1 only installs + smoke-verifies (no flows yet); real Maestro flows + agentic screenshot loops are
  B2/B3/B4.

### Files created / changed
- Edit: [rust/mock-copyparty/src/lib.rs](rust/mock-copyparty/src/lib.rs) (`spawn_with`/`ServeOptions`,
  fixed-port bind + SO_REUSEADDR, `size_overrides` in `AppState`/`listing_response`),
  [fixtures.rs](rust/mock-copyparty/src/fixtures.rs) (`size_mismatch`, `gap_index`, carry overrides),
  `rust/mock-copyparty/Cargo.toml` (`socket2` if needed).
- New: `rust/mock-comma-mcp/` (`Cargo.toml`, `src/main.rs`, server/registry/tools, tests); root
  `Cargo.toml` `members`; `.mcp.json`.
- Docs: short `rust/mock-comma-mcp/README.md` (tools + consumption model); note tooling in
  [docs/REFERENCES.md](docs/REFERENCES.md).

---

## Verification (B1 acceptance gates)
1. **Gates:** `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`
   (now covers both new crates); `cargo test --workspace` green.
2. **mock-copyparty:** tests prove (a) `size_mismatch` listing advertises `sz≠` file bytes; (b)
   `gap_index` → two drives via `group_segments`; (c) fixed-port `spawn_with` → drop → `tcp_reachable`
   (from `dashdown_core::connectivity`) returns `false` → re-spawn same port → `true`.
3. **mock-comma-mcp:** an in-process integration test drives the tool handlers end-to-end:
   `provision_device(single_drive)` → a `CopypartyClient` against the returned `base_url` lists exactly
   one drive; `set_reachable(false)` → `tcp_reachable` false; `set_reachable(true)` → true on the same
   port; `set_state(size_mismatch)` → listing advertises the wrong size; `teardown` frees it.
   Plus a **stdio MCP smoke**: spawn the binary, send `initialize` + `tools/list`, assert the 5 tools
   appear (mirrors B0's binding-smoke philosophy — exercise the real protocol path, not just units).
4. **size-mismatch end-state [RE-VERIFY]:** confirm whether the downloader commits a wrong-size final
   (→ `SizeMismatch`) or retries to `Failed`/`InProgress`; tune the fixture so it reliably yields the
   intended UI state (a `Partial`/resumable drive with a `SizeMismatch` file).
5. **Tooling:** `maestro --version` ok; `mobile-mcp` launches and sees the booted emulator.
6. **Registration:** `.mcp.json` valid; `cargo run -q -p mock-comma-mcp` starts and answers a scripted
   `initialize`/`tools/list` over stdio.
7. Commit on a branch → PR → `rust-ci` (+ any MCP smoke) green → squash-merge to `main`.

## Risks / open items
- **rmcp API drift** — pin the version (`cargo add rmcp`) and validate the macro/transport surface
  against `cargo doc` before writing the tools.
- **Fixed-port rebind** — confirm SO_REUSEADDR rebind of the same port works immediately after drop on
  Linux (no TIME_WAIT block) so `set_reachable(true)` reliably restores the port.
- **Maestro/mobile-mcp prerequisites** — install URL, node/npx presence, exact npm scope.
- **`cargo run` in `.mcp.json`** — first-use recompile latency; switch to a prebuilt binary if needed.
