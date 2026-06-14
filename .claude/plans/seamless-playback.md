# Near-seamless seek & segment-boundary playback

## Context

HD playback remuxes each comma HEVC segment to MP4 in memory, lazily, on ExoPlayer's loader thread
([HevcRemuxDataSource.open()](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/HevcRemuxDataSource.kt#L44-L64)).
The user wants seeking + boundary crossings to feel near-seamless. Two asks: (1) on a seek, decode
from the keyframe **before** the target rather than waiting for the whole segment; (2) seeking near
the **end** of a segment should cross into the next segment seamlessly.

## What the research found (and what's already true)

- **Ask #1 is already satisfied at the decode level.** The remux writes a real `stss` sync-sample
  table ([remux.rs:419-429](../../rust/core/src/video/remux.rs#L419)), and Media3's default
  `Mp4Extractor` seeks to the nearest sync sample ≤ target — it never decodes from frame 0. Measured
  comma GOP ≈ **1 s** (20-frame GOP, 2 slices/frame, 20 fps, 1200-frame/60 s segment), so the
  decode-forward after a keyframe seek is ≤1 s. The latency the user feels is **not decode** — it's
  the **cold-window remux** that `open()` blocks on. An LRU-hit (already-remuxed) seek is instant.
- **The remux is fast in release; the app ships it in debug.** Measured on host (36 MB segment):
  `fs::read` 25 ms + rewrap **46 ms release / 95 ms debug**. The emulator's ~1.6 s is emulation +
  FUSE-I/O + debug overhead; the installed core `.so` is the 106 MB **debug** build (release 9.8 MB).
  `cargoNdk` ([android/core/build.gradle:62-67](../../android/core/build.gradle#L62-L67)) sets no
  profile, so `installDebug` builds the core unoptimized.
- **Boundary-after-seek stalls** because ExoPlayer only requests seg+1's remux after buffering the
  current window's ~1 s tail (DefaultLoadControl, no `setLoadControl`), so the crossing races the
  ~1.6 s remux ([MultiCamPlayer.kt:143-146](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L143-L146)).
- **fMP4 streaming — rejected.** Decompiling Media3 1.10.1 `FragmentedMp4Extractor`: it needs a
  top-level `sidx` **before** the first `moof` or it declares `SeekMap.Unseekable`; building that
  `sidx` requires the same whole-segment scan. Net negative.
- **Partial (keyframe→end) remux — rejected.** Feasible to mux, but the window's duration becomes
  `(end-K)/fps` not 60 s, which desyncs the per-segment `MergingMediaSource` (HD vs qcamera) and
  corrupts [locate()/globalPosition()](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/RouteTimeline.kt#L12-L35)
  (they assume window == full segment). Big blast radius for a gain the fast remux already delivers.

## Approach (two low-risk changes)

### 1. Make the remux fast everywhere

- **`[profile.dev.package.dashdown-core] opt-level = 3`** (workspace `Cargo.toml`) so debug Android
  builds run an optimized core — matches the production (release) reality and ~halves the rewrap.
- **SIMD start-code scan**: replace `iter_nals`' byte-at-a-time `00 00 01` search
  ([remux.rs:186-212](../../rust/core/src/video/remux.rs#L186-L212)) with `memchr::memmem` — the
  prime CPU target. Must produce **byte-identical** output (guarded by the existing remux unit tests
  + `in_memory_bytes_match_written_file`). Add `memchr` via `cargo add`; opt-level it in dev too.
- The 37 MB read + 37 MB `mdat` memcpy are memory-bandwidth-bound (~5 ms on a real device) — leave
  them; don't change the `read()`/cached-bytes contract.

### 2. Prewarm the next segment on seek (the boundary fix)

- Add `HevcRemuxDataSource.Factory.prewarm(keys: List<String>)` that runs the existing `getOrRemux`
  path on a small background dispatcher (the LRU + per-key locks are already thread-safe, so a
  prewarm and a loader `open()` of the same key won't double-remux). Fire-and-forget; respects the
  LRU budget. Refactor `getOrRemux`'s body into a shared `warm(key)` used by both `open()` and
  `prewarm`.
- Hook in [seekGlobal](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L260-L264):
  after `locate()`, if the landed offset is within ~`PREWARM_TAIL_MS` (~8 s) of the window's
  duration, prewarm the **next** window's HD keys for the cams that have that segment on disk (reuse
  the `segsOf`/`hdSegs` membership). Guard `idx+1` in range.
- **Optional backstop:** `setLoadControl(DefaultLoadControl … maxBufferMs = ~90 s)` so steady-state
  playback prepares seg+1 earlier (default 50 s < 60 s segment is why the native look-ahead is
  tight). Keep min modest.

### Out of scope / rejected

fMP4 streaming; partial/keyframe-trimmed remux; sub-segment windowing; `ConcatenatingMediaSource2`
(would rework the 1:1 window↔segment timeline for no seek-latency gain). Frame-accurate EXACT seek
is kept (don't switch to `PREVIOUS_SYNC`).

## Verification

- **Rust:** existing remux tests + `in_memory_bytes_match_written_file` stay green (byte-exact);
  `bench_remux` (gated on `REMUX_BENCH_FILE`) shows the rewrap speedup (debug-with-opt vs old debug).
- **On-device (Pixel + emulator, real comma):** measure the `HevcRemux` `remux … in Xms` log before/
  after (cold-seek latency); seek to ~59 s and confirm the boundary crossing is seamless (seg+1 is a
  `hit`, no "Preparing HD…" flash) vs a stall before the prewarm; in-window/cached seeks stay instant.
- **Gates:** `cargo test`/`fmt`/`clippy`; `:app:testDebugUnitTest` + `:app:ktfmtCheck`.

## Results (validated)

- **Remux rewrap** (host, 36 MB segment): **95 ms → 18 ms** (memchr + dev opt-level=3), byte-exact
  (all 12 remux tests + `in_memory_bytes_match_written_file` green). `fs::read` ~25 ms.
- **On-device cold-seek remux**: emulator **~1.6 s → 289 ms**; Pixel (arm64) **~400 ms** (now
  dominated by the 36 MB external-storage/FUSE read, not the rewrap — a storage floor, out of scope).
  Cached/recent-window re-seeks are LRU hits (instant).
- **Prewarm** (emulator): seek to 13:57 (near the seg13→14 boundary) → `remux seg14` fired 270 ms
  later on the prewarm thread *while paused*; playing across 14:00 was a **cache hit, no spinner, no
  re-remux** — seamless. `prewarm` correctly skips a next segment with no HD on disk (frontier).
- Ask #1 (keyframe seek) needed no code: confirmed it was already the behavior; comma GOP ≈ 1 s.
