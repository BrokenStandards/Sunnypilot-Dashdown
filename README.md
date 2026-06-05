# Sunnypilot Dashdown

A mobile app (iOS + Android) for downloading dashcam footage from Comma devices running the
[Sunnypilot](https://github.com/sunnypilot/sunnypilot) openpilot fork, where footage is served
over a [copyparty](https://github.com/9001/copyparty) file server. It groups 1-minute segments
into gap-split **drives**, mirrors files locally for offline browsing, downloads whole drives in
the background with file-granular resume, and manages retention/auto-delete per device.

A shared **Rust core** (via **UniFFI**) holds all logic; **SwiftUI** and **Jetpack Compose**
provide native UIs.

## Status

**Phase 0 — environment bootstrap complete.** Next: **M0** (Cargo workspace scaffolding,
exported `ping()`, bindgen smoke, CI cross-compile) — see the master plan. No application code
exists yet.

## Getting started

```sh
# 1. Reference source (gitignored, read-only) used while developing:
tools/fetch-refs.sh                 # clones pinned copyparty / sunnypilot / uniffi-rs / uniffi-starter into ref/

# 2. Toolchain (Phase 0 already installed these locally):
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
cargo install cargo-ndk             # Android .so builds; needs an Android NDK (r27.x)
```

## Docs

- **Architecture & milestones:** [.claude/plans/sunnypilot-dashdown-master-plan.md](.claude/plans/sunnypilot-dashdown-master-plan.md)
- **Working conventions (incl. how `ref/` works):** [CLAUDE.md](CLAUDE.md)
- **Reference repos (pins, purpose, iOS-on-Linux recipe):** [docs/REFERENCES.md](docs/REFERENCES.md)
