# Sunnypilot Dashdown — working conventions

Mobile app (iOS + Android) that downloads dashcam footage from Comma devices running the
Sunnypilot openpilot fork, where footage is served over a **copyparty** file server.

**Architecture:** a shared **Rust core** (logic: copyparty client, drive grouping, mirror
storage, sync/resume engine, retention, connectivity, rusqlite index) exposed over **UniFFI**
to **native UIs** — SwiftUI (iOS) and Jetpack Compose (Android). Full detail and the milestone
breakdown live in [.claude/plans/sunnypilot-dashdown-master-plan.md](.claude/plans/sunnypilot-dashdown-master-plan.md).
Phase 0 bootstrap is recorded in `.claude/plans/Phase 0 — Environment Bootstrap & Reference Setup.md`.

## Reference source — `ref/` (read-only, gitignored)

Upstream code we read but never ship lives under `ref/` (copyparty, sunnypilot, uniffi-rs,
uniffi-starter; iOS-on-Linux tools on demand). It is **gitignored** and in **`.ignore`**, so
`rg`, the Grep/Glob tools, and the Explore agent **skip it by default** — searches over our
code stay clean.

- **Never** `use`/import, build, or copy `ref/` code into our crates. Reference only.
- Search it on purpose: **`tools/refgrep <pattern>`** (or `rg --no-ignore -g 'ref/**' …`).
- Recreate/repin it: **`tools/fetch-refs.sh`**. Pins + purpose + key paths: [docs/REFERENCES.md](docs/REFERENCES.md).

## Workflow (from the master plan)

- **Plan per phase.** Each milestone/phase enters plan mode and writes its own plan under
  `.claude/plans/` before coding.
- **Build complete, test complete, then advance.** A milestone is done only when built, wired
  end-to-end, and verified by automated tests — including the hard cases (app backgrounded/
  killed mid-download, interrupted transfer resumed, device unreachable). Nothing is stubbed
  and carried forward.
- **Commit per milestone.** `git commit` after each milestone. If a phase uses a branch, merge
  back to `main` at the end of the phase.
- **Use agents** for research, code verification, running tests, and debugging.
- If a new MCP would help (e.g. UI-alignment checks, or a fixture controller), advise building
  it rather than improvising.

## Toolchain (installed in Phase 0)

- Rust nightly + targets: Android (`aarch64-linux-android`, `armv7-linux-androideabi`,
  `x86_64-linux-android`, `i686-linux-android`) and iOS std (`aarch64-apple-ios`,
  `aarch64-apple-ios-sim`, `x86_64-apple-ios`).
- **Android:** `cargo-ndk` + NDK `r27.3.13750724` at `/opt/android-sdk/ndk/27.3.13750724`.
  cargo-ndk auto-detects it; export `ANDROID_NDK_HOME=/opt/android-sdk/ndk/27.3.13750724` to be explicit.
- **UniFFI bindgen:** no global binary (version must match the `uniffi` dep). Use the
  in-workspace `rust/bindgen` crate (M0): `cargo run -p bindgen -- …`.
- **iOS is built on Linux** via `xtool` (+ `libimobiledevice` for devices) — set up in Phase B,
  recipe in docs/REFERENCES.md. `XcodeBuildMCP`/Xcode are **not** part of this path.

## MCPs

- **Phase A (Rust core): none required.** Verified locally with `cargo test`, wiremock, and the
  in-repo `mock-copyparty` fixture. The "doc-lookup MCP" idea is replaced by `ref/` + `refgrep`.
- **GitHub MCP** connected for PR/CI workflow.
- **Phase B:** `mobile-mcp` (Android UI automation), and a `mock-comma-mcp` wrapper around the
  `mock-copyparty` fixture for hermetic, state-injecting UI tests.

## Device test tooling (real hardware)

One-command helpers for common on-device tasks (instead of many UI taps / adb calls):

- **`tools/dd-db.sh [devices|identity|drives|segments|schema|"<SQL>"]`** — pull the live
  on-device `index.sqlite` (+WAL) and print a query result.
- **`tools/dd-ui.sh <flow> [KEY=VAL …]`** — run a parameterized Maestro flow under
  `android/maestro/` (`add_device`, `remove_device`, `clear_devices`); sets JDK 17 (Maestro needs 17+).
  E.g. `tools/dd-ui.sh add_device NAME=escape2020 IP=192.168.1.100`.
- After a flow, dump the screen **once** via mobile-mcp `list-elements` to read the end state and
  find new buttons — rather than many list calls mid-flow.

## Don't commit

`ref/`, `target/`, generated bindings, `.env`/secrets, Android/iOS build outputs (see `.gitignore`).
