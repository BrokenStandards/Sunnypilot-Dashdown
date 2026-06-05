# Phase 0 — Environment Bootstrap & Reference Setup

## Context

The repo is greenfield: only the [master plan](sunnypilot-dashdown-master-plan.md) and a
`plansDirectory` setting exist. Before M0 (Rust-core scaffolding) can start, we need a
prepared environment and a clean way to keep third-party source on hand for reference
without polluting our own code searches.

This plan covers **only**: install the Phase A toolchain, clone + pin + document the
reference repos, establish repo conventions (`.gitignore`, `CLAUDE.md`, `docs/`, `tools/`),
and set up GitHub. **No crate code is written** — the Cargo workspace and all crates are
M0's job under M0's own plan. Scope was confirmed with the user: *"Env + refs + conventions
only"*, *"Set up GitHub now"*, and iOS will be pursued **on Linux** (xtool / Swift-on-Linux
/ osxcross / libimobiledevice) rather than deferred to a Mac.

### Environment facts discovered
- Rust nightly 1.96 + rustup present. Installed targets: host + wasm + windows-gnu only.
- Android SDK at `/opt/android-sdk` (sdkmanager, platform-tools). **NDK not installed**,
  `ANDROID_NDK_HOME` unset. No emulator package.
- `gh` not installed; **no git remote**. `docker`, `adb`, `java 26`, `gradle`, `node`,
  `python3`, `clang` present. `swiftc`/`xcodebuild` absent (expected on Linux).
- Useful MCPs: **none** connected (only unauthenticated claude.ai Google services). The
  master plan's "GitHub MCP already connected" is not true here.

---

## MCP decision (first phase = Phase A, Rust core)

**Phase A needs no new MCPs.** It is pure Rust verified locally (`cargo test`, wiremock,
the in-repo `mock-copyparty` fixture). The master plan's idea of a "doc-lookup MCP for the
parent codebase" is **replaced by a gitignored `ref/` dir + ripgrep** (see below) — cheaper,
zero-maintenance, and already grep-safe.

- **Connect now (per user):** GitHub MCP — supports the PR/CI workflow from M0 onward.
  Not strictly required for local Rust work, but requested.
- **Defer to Phase B:** `mobile-mcp` (Android UI automation — works on Linux against an
  emulator/device) and a to-be-built `mock-comma-mcp` wrapper around `mock-copyparty`.
- **Flagged N/A:** `XcodeBuildMCP` requires Xcode/macOS, so it does **not** fit the
  iOS-on-Linux path — iOS build/deploy will use the `xtool` CLI + `libimobiledevice`
  directly instead. We will revisit agentic iOS verification in Phase B.

---

## Reference directory strategy (the grep-safety requirement)

**Location:** `ref/` at the repo root, **fully gitignored**. This is the sweet spot:
- It lives right inside the tree (easy to open/read), but
- ripgrep — which Claude Code's Grep/Glob tools and the Explore agent all use — **respects
  `.gitignore` by default**, so `rg pattern` and every code search skip `ref/`
  automatically. No pollution of searches over *our* code.
- Belt-and-suspenders: a top-level `.ignore` file also listing `ref/` (ripgrep honors
  `.ignore`/`.rgignore` as a separate layer, covering tools that bypass VCS-ignore).
- The clones are huge and must never be committed anyway — gitignoring is required, not
  just convenient.

**Deliberate search escape hatch** (documented in `CLAUDE.md` + `docs/REFERENCES.md`):
- `tools/refgrep` wrapper → `rg --no-ignore -g 'ref/**' "$@"`, or ad-hoc `rg --no-ignore ref/ -e PATTERN`.

**Reproducible, not committed:** `ref/` is rebuildable from a committed script
`tools/fetch-refs.sh <group>` that clones each repo and checks out a **pinned commit SHA**.
The SHAs + purpose + key paths live in the committed manifest `docs/REFERENCES.md`. A fresh
checkout reconstructs `ref/` with one command; nothing reference-related bloats history.

**Clone strategy** (depth-1, no submodules/LFS — we read source, we don't build these):

| Repo | Command (into `ref/`) | Why / key paths |
|------|----------------------|-----------------|
| **copyparty** `9001/copyparty` | `git clone --depth 1` (branch `hovudstraum`) | M1 listing/auth/download + M6 delete. Look at `copyparty/httpcli.py` (`?ls=j`, `?zip`/`?tar`, `PW:`), WebDAV handlers, `docs/`. |
| **sunnypilot** `sunnypilot/sunnypilot` | `git clone --depth 1 --no-recurse-submodules` (branch `master`) | M1 storage layout: `system/loggerd/` (segment/realdata writing, `qcamera.ts`), route/segment naming. Optional sparse-checkout of `system/loggerd`,`system/hardware`,`selfdrive` if size matters. |
| **uniffi-rs** `mozilla/uniffi-rs` | `git clone --depth 1` | M0 scaffolding + M8 surface: `examples/`, async/callback-interface patterns, proc-macro `setup_scaffolding!`. |
| **uniffi-starter** `ianthetechie/uniffi-starter` | `git clone --depth 1` | High-value working reference: UniFFI + cargo-ndk (Android) + xcframework (iOS) wiring and workspace layout. |

**Phase B / iOS-on-Linux group** (listed in manifest, cloned when Phase B starts — or now
behind `tools/fetch-refs.sh phaseb`): `xtool-org/xtool` (build/sign/deploy iOS on Linux),
`tpoechtrager/osxcross` (macOS cross toolchain if needed), `libimobiledevice/libimobiledevice`
(device deploy/run from Linux).

---

## Toolchain installs (Phase A)

So that M0 can cross-compile the core to Android (and the host targets for tests):

1. **Rust targets:** `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android` (Android, incl. emulator). Also add the cheap iOS std targets now for later: `aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios` (note: linking needs the Phase-B Swift SDK; install is harmless now).
2. **Android NDK:** `sdkmanager --install "ndk;<latest-r27>"` and export `ANDROID_NDK_HOME`. ⚠️ `/opt/android-sdk` may be root-owned → this step (and `gh` install) may need **sudo**; I will surface and confirm if a password prompt blocks me.
3. **cargo tools:** `cargo install cargo-ndk uniffi-bindgen`. Optional dev niceties (mention only): `cargo-nextest`, `cargo-watch`.
4. **Deferred to Phase B (documented recipe, not installed now):** Swift-on-Linux toolchain + extracted iOS Swift SDK via `xtool setup`, `osxcross`, `libimobiledevice`, `cargo-xcframework`/`xtool` packaging. Captured in `docs/REFERENCES.md` so Phase B can execute it.

---

## GitHub setup (per user)

1. **Install `gh`** (Arch: `sudo pacman -S github-cli`, or fetch the release binary if sudo is unavailable).
2. **Auth:** `gh auth login` — ⚠️ interactive; I will pause for the user to complete the device/browser flow.
3. **Create remote:** default **private** repo named `sunnypilot-dashdown` under the user's account (`gh repo create sunnypilot-dashdown --private --source=. --remote=origin`). I will confirm name/visibility at execution if anything is ambiguous.
4. **Initial commit + push:** the convention/doc files created by this plan (below) + the existing `.claude/plans/`. This is a meaningful first commit with no half-built code.
5. **Connect GitHub MCP:** `claude mcp add` the GitHub MCP server (hosted GitHub MCP w/ PAT or the official `github-mcp-server`). ⚠️ may require a token/OAuth step — surfaced to user.

---

## Files created this session (conventions only — no crate code)

- **`.gitignore`** — Rust (`/target`), `/ref/`, Android (`/android/.gradle`,`/android/build`,`local.properties`), iOS build outputs (`.build/`, `*.xcframework/`), generated bindings, `.env`, `.DS_Store`.
- **`.ignore`** — contains `ref/` (ripgrep belt-and-suspenders).
- **`docs/REFERENCES.md`** — the reference manifest: per-repo URL, pinned SHA, purpose, key paths, how to (re)fetch, how to search (`tools/refgrep`). Two groups: Phase A (cloned now) + Phase B/iOS-on-Linux (deferred).
- **`tools/fetch-refs.sh`** — clones/pins each ref into `ref/` (`phasea` default, `phaseb` optional, `all`).
- **`tools/refgrep`** — `rg --no-ignore -g 'ref/**' "$@"` wrapper for deliberately searching references.
- **`CLAUDE.md`** — repo conventions: target architecture summary; `ref/` is read-only third-party source (gitignored, never imported/built, search via `tools/refgrep`); milestone→git-commit + branch-merge-per-phase workflow; "each phase enters plan mode and writes its own plan"; toolchain commands; iOS-on-Linux note.
- **`README.md`** — one-paragraph project intro + "getting started" (run `tools/fetch-refs.sh`, install toolchain) pointing at the master plan.

## Target architecture / file structure (planned, **built in M0** — not now)

Confirms the master plan's workspace layout, with the additions above. Reproduced here so the
structure is settled before M0:

```
sunnypilot-dashdown/
  Cargo.toml                 # [workspace]            ← M0
  rust/
    core/                    # staticlib+cdylib+lib   ← M0..M8 (ffi, model, copyparty_client,
                             #                            drive_grouping, storage, sync_engine,
                             #                            connectivity, db, settings, logging)
    mock-copyparty/          # axum fixture server     ← M1
    bindgen/                 # uniffi_bindgen wrapper   ← M0/M8
  ios/                       # SwiftPM app (xtool)      ← Phase B
  android/                   # Gradle (Compose)         ← Phase B
  tools/                     # fetch-refs.sh, refgrep, Maestro flows, CI  ← grows over time
  docs/                      # REFERENCES.md + design notes
  ref/                       # gitignored third-party source (this session)
  .claude/plans/             # per-phase plans
```

Correction carried into M0: Android armv7 triple is **`armv7-linux-androideabi`**.

---

## Verification (end of this session)

1. `rustup target list --installed` shows the four Android triples (+ iOS std targets).
2. `cargo ndk --version` and `uniffi-bindgen --version` succeed; `echo $ANDROID_NDK_HOME` points at an installed NDK.
3. `ls ref/` shows `copyparty/`, `sunnypilot/`, `uniffi-rs/`, `uniffi-starter/`; each at the pinned SHA in `docs/REFERENCES.md`.
4. **Grep-safety proof:** pick a token guaranteed to exist only in a ref (e.g. `copyparty` internal symbol) → `rg <token>` from repo root returns **nothing**, while `tools/refgrep <token>` returns hits. Confirms `ref/` is invisible to normal code search but reachable on demand.
5. `gh auth status` ok; `gh repo view` shows the new private remote; `git log` shows the initial commit pushed to `origin`.
6. `claude mcp list` shows the GitHub MCP connected/healthy.
7. `git status` is clean except intended files; `ref/` does not appear (gitignored).

After approval I'll also note the one-line follow-up: **the next step is to enter plan mode
for M0** (workspace + `ping()` + bindgen smoke + CI cross-compile), per the master plan's
"each phase writes its own plan" rule.
