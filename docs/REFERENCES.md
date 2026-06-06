# Reference material (`ref/`)

Third-party source we read for reference while building this app. It is **not** part of our
codebase: never imported, never built, never committed.

## Where it lives & why you won't grep it by accident

All reference clones live under **`ref/`** at the repo root, which is **gitignored**
(see [`.gitignore`](../.gitignore)) and also listed in [`.ignore`](../.ignore). ripgrep —
which Claude Code's Grep/Glob tools and the Explore agent use under the hood — respects both
files by default, so a normal `rg pattern` (or any code search) over the repo **skips `ref/`
entirely**. Our own code stays uncontaminated by upstream symbols.

### To search the references on purpose

```sh
tools/refgrep <pattern>            # wrapper: rg --no-ignore -g 'ref/**' <pattern>
rg --no-ignore -g 'ref/**' <pat>   # equivalent ad-hoc form
```

### To (re)create `ref/` from scratch

```sh
tools/fetch-refs.sh            # Phase A repos (default), pinned to the SHAs below
tools/fetch-refs.sh phaseb     # Phase B / iOS-on-Linux repos
tools/fetch-refs.sh all
```

The script pins each repo to an exact commit (shallow `init`+`fetch`+`checkout`), so refs are
reproducible and nothing reference-related ever enters our git history.

---

## Phase A references (cloned now)

| Repo | URL | Pinned commit | Why we keep it / where to look |
|------|-----|---------------|--------------------------------|
| **copyparty** | https://github.com/9001/copyparty | `6e75faa62349a59f4df328a4939ba8626d89ee1a` (branch `hovudstraum`) | The file server the app talks to. **M1:** JSON listing `?ls=j`, `?pw=`/`PW:` auth, streamed download. **M6:** WebDAV `DELETE`, `?zip`/`?tar`. Read `copyparty/httpcli.py`, the WebDAV handlers, and `docs/`. Confirm the Range-not-reliable behavior (issue #329) that drives our file-granular resume. |
| **sunnypilot** | https://github.com/sunnypilot/sunnypilot | `46b9253729193e47a8be99154bae41c35359a373` (branch `master`) | Source device firmware. **M1:** segment/route storage layout under `/data/media/0/realdata/`, route name `dongleid\|YYYY-MM-DD--HH-MM-SS`, per-segment files (`fcamera.hevc`, `ecamera.hevc`, `qcamera.ts`, `rlog`/`qlog`). Read `system/loggerd/` (logger + encoder + segment writing). Cloned `--no-recurse-submodules` (we read source, don't build). |
| **uniffi-rs** | https://github.com/mozilla/uniffi-rs | `1a6111c32f8be55bfedceddabbf27ec65f4c7755` (branch `main`) | The FFI generator. **M0/M8:** proc-macro `setup_scaffolding!`, `#[uniffi::export]`, async (`async_runtime="tokio"`), callback interfaces, error types. Read `examples/` and the user-guide sources. |
| **uniffi-starter** | https://github.com/ianthetechie/uniffi-starter | `b466bc276437250cca3b477b4840b49488205a91` (branch `main`) | A working end-to-end UniFFI project: Rust core + **cargo-ndk** (Android `.so` + Gradle) + **xcframework** (iOS) wiring and workspace layout. The closest template to our M0–M8 + Phase B shape. |

## Phase B / iOS-on-Linux references (cloned on demand: `tools/fetch-refs.sh phaseb`)

We are building iOS **on Linux**, so `XcodeBuildMCP`/Xcode do not apply. The toolchain is:

| Repo | URL | Role |
|------|-----|------|
| **xtool** | https://github.com/xtool-org/xtool | Cross-platform Xcode replacement. Builds/signs/bundles/deploys iOS apps with SwiftPM on Linux. Extracts a Swift SDK from `Xcode.xip` (`xtool setup`); uses `libxadi` for Apple-developer auth without a Mac. Primary iOS build path. |
| **osxcross** | https://github.com/tpoechtrager/osxcross | macOS/Darwin cross-compilation toolchain (clang + macOS SDK) on Linux — fallback for any native bits xtool doesn't cover. |
| **libimobiledevice** | https://github.com/libimobiledevice/libimobiledevice | Talk to / install on a real iPhone over USB from Linux (deploy + run XCUITest-equivalent flows). |

### iOS-on-Linux setup recipe (run when Phase B starts — not done yet)

1. Install a Swift toolchain for Linux (swiftly or swift.org tarball); confirm `swift --version`.
2. `git clone` + build/install `xtool`; run `xtool setup` (needs an `Xcode.xip` to extract the iOS Swift SDK once).
3. `rustup target add` the iOS triples are already done; produce the `.xcframework` via xtool's packaging (replaces `cargo-xcframework`, which assumes macOS).
4. For on-device runs, build `libimobiledevice`; pair the device.
5. Apple-developer auth via xtool/`libxadi` for signing.

---

## Toolchain installed during Phase 0 bootstrap

- Rust targets: `aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android` + iOS std targets `aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios`.
- `cargo-ndk` v4.1.2 (`~/.cargo/bin`).
- Android NDK **r27.3.13750724** at `/opt/android-sdk/ndk/27.3.13750724`. cargo-ndk auto-detects the highest installed NDK; set `ANDROID_NDK_HOME` to that path to be explicit.
- **No `uniffi-bindgen` global install** — there is no current published binary crate, and the bindgen version must match the `uniffi` dependency. The canonical generator is an **in-workspace `rust/bindgen` crate** built in M0 (`fn main() { uniffi::uniffi_bindgen_main() }`), invoked via `cargo run -p bindgen`.

---

## Phase B test tooling (installed in B1)

Hermetic UI testing for the native shells. Both MCP servers are registered in the repo-root `.mcp.json`.

- **`mock-comma-mcp`** (in-repo, `rust/mock-comma-mcp`) — MCP server wrapping the `mock-copyparty`
  fixture; provisions devices, injects states, toggles reachability. `cargo run -q -p mock-comma-mcp`.
- **Maestro** v2.6.0 — `curl -fsSL https://get.maestro.mobile.dev | bash` → `~/.maestro/bin/maestro`
  (added to `PATH` via the shell profile). Needs JDK 17+ and `adb`. Cross-platform YAML UI flows (B4).
- **mobile-mcp** (`@mobilenext/mobile-mcp`, v0.0.58) — Android UI automation over `adb` on Linux,
  run via `npx -y @mobilenext/mobile-mcp@latest` (stdio). Exposes `mobile_*` tools (launch, tap,
  screenshot, list elements). Used for agentic UI runs in B2/B3.

The Android emulator (`dashdown-b0` AVD) + SDK were set up in B0.
