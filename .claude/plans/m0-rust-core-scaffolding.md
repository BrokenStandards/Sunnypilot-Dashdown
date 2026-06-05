# M0 — Rust Core Scaffolding & Cross-Compile

## Context

Phase 0 prepared the environment (toolchain, references, repo conventions). **M0 is the first
build milestone** from the [master plan](sunnypilot-dashdown-master-plan.md): stand up the
Cargo workspace and a `core` crate that compiles to every platform target through **UniFFI**,
plus an in-workspace bindgen and **CI that proves the cross-compile is green**.

Per the master plan's operating principle, M0 is **scaffolding + a smoke FFI surface only** —
sync `ping()`/`version()` and an async `ping_async()` (to de-risk the async+tokio path early).
It does **not** create stub modules for M1–M8 (no `copyparty_client`, `db`, `mock-copyparty`,
etc. — "no functionality is stubbed and carried forward"). Those arrive in their own milestones.

**Decisions locked:** pin a **stable** `rust-toolchain.toml` with all 7 targets; validate the
**async + tokio** UniFFI path in M0. **References to mirror:** `ref/uniffi-starter` (workspace +
cargo-ndk + bindgen + iOS staticlib flow) and `ref/uniffi-rs` v0.31 (`examples/arithmetic-procmacro`,
`docs/manual/src/tutorial/foreign_language_bindings.md`). Search them with `tools/refgrep`.

**Definition of done:** `cargo test --workspace` green; fmt/clippy clean; `cargo-ndk` produces a
`.so` for all 4 Android ABIs; iOS staticlib builds (or `cargo check`, see risk) for
`aarch64-apple-ios(-sim)`; bindgen generates Kotlin + Swift containing the ping/version symbols;
CI green; committed.

---

## Target file layout (created in M0)

```
Cargo.toml                      # [workspace] members + [workspace.dependencies] + [workspace.package]
rust-toolchain.toml             # channel="stable" + all 7 targets (rustup auto-provisions)
rustfmt.toml                    # minimal/default
rust/
  core/
    Cargo.toml                  # package dashdown-core; [lib] name=dashdown_core, crate-type=[cdylib,staticlib,lib]
    src/lib.rs                  # uniffi::setup_scaffolding!(); ping/version/ping_async + #[cfg(test)]
  bindgen/
    Cargo.toml                  # uniffi {features=["cli"]}; [[bin]] uniffi-bindgen
    src/main.rs                 # fn main(){ uniffi::uniffi_bindgen_main() }
tools/gen-bindings.sh           # host build + generate kotlin & swift into target/bindings/
.github/workflows/rust-ci.yml   # host test/fmt/clippy + Android cdylib + iOS staticlib + bindgen smoke
```

Workspace `members = ["rust/core", "rust/bindgen"]` (M1 will add `rust/mock-copyparty`).

**Naming refinement vs master plan:** the master plan informally calls the crate `core`. Use
package **`dashdown-core`** / `[lib] name = "dashdown_core"` to avoid shadowing std's `core` when
`bindgen`/`mock-copyparty` depend on it, and to give a clean artifact (`libdashdown_core.so/.a`)
and UniFFI namespace (`dashdown_core`). Directory stays `rust/core`.

---

## Implementation steps

### 1. Workspace root `Cargo.toml`
```toml
[workspace]
members = ["rust/core", "rust/bindgen"]
resolver = "2"

[workspace.package]
version = "0.0.0"
edition = "2021"
license = "..."          # confirm during execution

[workspace.dependencies]
uniffi = "0.31"          # set via `cargo add`; exact version pinned in Cargo.lock
tokio  = { version = "1", features = ["rt-multi-thread", "macros", "time"] }
thiserror = "2"
```
> **Critical:** `core` and `bindgen` must use the **same uniffi version** (library-mode bindgen
> reads metadata embedded by the proc-macros — a version skew breaks generation). The workspace
> dep guarantees this. Get the latest with `cargo add uniffi` rather than hand-editing.

### 2. `rust/core`
- `Cargo.toml`:
  ```toml
  [package]
  name = "dashdown-core"
  version.workspace = true
  edition.workspace = true

  [lib]
  name = "dashdown_core"
  crate-type = ["cdylib", "staticlib", "lib"]

  [dependencies]
  uniffi = { workspace = true, features = ["tokio"] }   # "tokio" needed for async_runtime="tokio"
  tokio  = { workspace = true }
  ```
  Pure proc-macro ⇒ **no `build.rs`, no build-dependencies** (confirmed in
  `ref/uniffi-rs/docs/manual/src/proc_macro/index.md`).
- `src/lib.rs`:
  ```rust
  uniffi::setup_scaffolding!();

  /// Sync smoke export — proves the basic FFI path.
  #[uniffi::export]
  pub fn ping() -> String { "pong".to_string() }

  /// Build/version smoke export.
  #[uniffi::export]
  pub fn version() -> String { env!("CARGO_PKG_VERSION").to_string() }

  /// Async smoke export — proves the async + tokio FFI path generates on all targets.
  #[uniffi::export(async_runtime = "tokio")]
  pub async fn ping_async() -> String {
      tokio::time::sleep(std::time::Duration::from_millis(1)).await;
      "pong".to_string()
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      #[test] fn ping_works() { assert_eq!(ping(), "pong"); }
      #[tokio::test] async fn ping_async_works() { assert_eq!(ping_async().await, "pong"); }
  }
  ```
  > These three are explicitly **smoke/health exports**, removed/replaced once real APIs land
  > (M8 brings the real `AppCore` surface). Pattern mirrors `ref/uniffi-rs/examples/arithmetic-procmacro`.

### 3. `rust/bindgen` (mirrors `ref/uniffi-starter/rust/uniffi-bindgen`)
- `Cargo.toml`: `uniffi = { workspace = true, features = ["cli"] }`, `[[bin]] name = "uniffi-bindgen"`, `required-features = ["cli"]`.
- `src/main.rs`: `fn main() { uniffi::uniffi_bindgen_main() }`.

### 4. `rust-toolchain.toml`
```toml
[toolchain]
channel = "stable"
targets = [
  "aarch64-apple-ios", "aarch64-apple-ios-sim", "x86_64-apple-ios",
  "aarch64-linux-android", "armv7-linux-androideabi", "x86_64-linux-android", "i686-linux-android",
  "x86_64-unknown-linux-gnu",
]
components = ["clippy", "rustfmt"]
```

### 5. `tools/gen-bindings.sh` (host bindgen smoke, reusable)
```sh
#!/usr/bin/env bash
set -euo pipefail
cargo build -p dashdown-core                       # host cdylib -> target/debug/libdashdown_core.so
LIB=target/debug/libdashdown_core.so
for lang in kotlin swift; do
  cargo run -p bindgen --bin uniffi-bindgen -- generate \
    --library "$LIB" --language "$lang" --out-dir "target/bindings/$lang"
done
```
(Library mode + flags confirmed in `ref/uniffi-rs/docs/.../foreign_language_bindings.md`.)

### 6. Cross-compile commands (also the CI body)
- **Android (cdylib .so, all 4 ABIs)** — `cargo-ndk` auto-detects the NDK:
  ```sh
  cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -t x86 build -p dashdown-core --release
  ```
- **iOS (staticlib .a)** — build **only** the staticlib to avoid the cdylib link step (no Apple
  linker on Linux):
  ```sh
  cargo rustc -p dashdown-core --lib --release --crate-type staticlib --target aarch64-apple-ios
  cargo rustc -p dashdown-core --lib --release --crate-type staticlib --target aarch64-apple-ios-sim
  ```

### 7. `.github/workflows/rust-ci.yml` (ubuntu-latest; separate from the existing claude*.yml)
Steps: checkout → `dtolnay/rust-toolchain@stable` (or rely on `rust-toolchain.toml`) →
`Swatinem/rust-cache` → `cargo fmt --check` → `cargo clippy --workspace -- -D warnings` →
`cargo test --workspace` → `nttld/setup-ndk@v1` (r27) + `cargo install cargo-ndk` + the Android
build → the two iOS staticlib builds → `bash tools/gen-bindings.sh` + assert
`target/bindings/kotlin` & `…/swift` contain `ping`/`ping_async`/`version`.

### 8. Commit / PR
Work on branch `phase-a/m0-scaffolding`; push; let `rust-ci.yml` + the existing Claude review
workflow run; open a PR and merge to `main` (exercises the Phase 0 GitHub/CI setup). Commit
message: `M0: Rust workspace + core (UniFFI ping) + bindgen + cross-compile CI`.

---

## Risks / things to verify during M0

- **iOS staticlib on Linux (primary risk).** Producing a `.a` for `aarch64-apple-ios` via
  `cargo rustc --crate-type staticlib` should work (rustc archives rlibs with `ar`; no Apple
  linker needed). **If it fails** (toolchain wants Apple `ar`/libs), fall back to
  `cargo check -p dashdown-core --target aarch64-apple-ios(-sim)` as the M0 iOS smoke and defer
  the real `.a` to Phase B via xtool's Swift SDK. Either way, **report which path worked**.
- **uniffi version drift:** use `cargo add` for the latest; pin via committed `Cargo.lock`. Keep
  `core` and `bindgen` on the identical version (workspace dep).
- **NDK in CI:** `nttld/setup-ndk` exports `ANDROID_NDK_HOME`; locally cargo-ndk auto-detects
  `/opt/android-sdk/ndk/27.3.13750724`.

---

## Verification (M0 done)

1. `cargo test --workspace` green (`ping_works`, `ping_async_works`).
2. `cargo fmt --check` clean; `cargo clippy --workspace -- -D warnings` clean.
3. `cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -t x86 build -p dashdown-core` → a
   `libdashdown_core.so` for each ABI.
4. iOS: both `cargo rustc … --crate-type staticlib --target aarch64-apple-ios(-sim)` produce a
   `libdashdown_core.a` (or `cargo check` green per the fallback).
5. `bash tools/gen-bindings.sh` → `target/bindings/kotlin/uniffi/dashdown_core/*.kt` and
   `target/bindings/swift/*.swift` exist and contain `ping`, `ping_async`, `version`.
6. `rust-ci.yml` green on the M0 PR; merged to `main`.

After M0 merges, the next milestone is **M1 (copyparty client + model + db + mock-copyparty
fixture)**, which gets its own plan.
