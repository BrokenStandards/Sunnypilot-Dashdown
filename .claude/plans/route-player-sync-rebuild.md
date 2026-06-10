# Route player — multi-camera sync rebuild

**Status:** planned (not started). Follow-on to RP3 (merged in #24).
**Branch:** `feat/single-player-multicam`
**Goal:** all enabled video tiles **and** audio play smoothly and frame-synced *at the same time* — fixing the structural defect where only one track is ever smooth.

Backed by the `multicam-sync-research` workflow (71 agents, 6 candidate
architectures, 3-vote adversarial verification of every decision-critical claim).
Verdicts cited inline below survived **3/3** unless noted.

---

## 1. Why the current player is choppy (mechanically)

`MultiCamPlayer.kt` runs **N independent `ExoPlayer`s** — `qPlayer` (qcamera) + one
per HD tile in `hdPlayers` — each with its **own `MediaClock`**. ExoPlayer provides
**no cross-instance sync** (verified: ExoPlayer #2855 — independent players are
explicitly not synchronized). The code masks divergence with a throttled
corrective-seek loop ([MultiCamPlayer.kt:295-322](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L295-L322), `TICK_MS=120`, `RESYNC_EVERY_TICKS=8`):

1. Elect a `master` ([:176](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L176)) — qcamera when audio is on, else the first visible HD tile.
2. Every ~960 ms, compare each follower's `globalPosition` to the master via `shouldResync` ([RouteClock.kt:64](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/RouteClock.kt#L64), 60 ms threshold).
3. Yank a drifted follower back with `seekTo` ([:314](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L314)).

**Root cause is structural:** every `seekTo()` **flushes that follower's decoder**.
The elected master is the only decoder never flushed → at best **1 track smooth,
N−1 stutter**. Audio can't be seeked without glitching, so when audio is on it
becomes master and *all* video tiles stutter. This cannot be tuned away; it's
inherent to using seek as the correction primitive across independent clocks.

---

## 2. Options considered (verified)

| # | Approach | Verdict |
|---|---|---|
| **A** | **Single `ExoPlayer`, one `MediaClock`, N video renderers (custom `RenderersFactory`) + custom `TrackSelector` + `MergingMediaSource`, each renderer on its own Surface** | **RECOMMENDED (primary).** Google's recommended multi-stream pattern (✓3/3); one clock ⇒ zero chase-seeks. *Caveat (✓ refuted the optimistic gloss):* renderers share one clock + render loop but per-frame lock is **not guaranteed** and there's **no first-party video sample** → must spike. |
| **B** | **Rate-PLL drift correction** — keep N players, nudge each video follower's `setPlaybackSpeed(1±%)` to converge instead of seeking; audio master stays 1.0× and is never touched | **RECOMMENDED (fallback).** All claims ✓3/3. Low-risk, localized to the existing loop. Honest ceiling: smooth but *approximately* synced (~±1 frame), not exact lockstep. |
| C | DIY MediaCodec→SurfaceTexture + GL composite, audio-clock-gated frame release | Same end-state as A, far higher cost (GL pipeline, per-decoder threads, new audio path, device fragmentation); the buffer-hold gating mechanism was partially refuted. Not worth it over A. |
| D | Media3 `CompositionPlayer` multi-video-sequence preview | **Disqualified.** Multi-video-sequence **seeking is unsupported/crashes** (`ERROR_CODE_VIDEO_FRAME_PROCESSING_FAILED`), `@ExperimentalApi`, "in active development", no committed timeline (✓3/3, androidx/media #2439). Our drive-wide scrubber is exactly the broken path. Revisit only when #2439 ships. |
| E | Pre-composite a tiled mosaic MP4 (Transformer encode) | **Disqualified.** Needs an **encoder the Rust core doesn't have**; gates time-to-first-frame on encoding a 15-min/16-segment drive; can't change layout/toggles without re-baking (✓3/3). |

Decoder budget for A/B/C: worst case 4 HEVC@20fps decoders, under the Android-16
CDD floor of 6 concurrent HW decoders (✓3/3, but Media-Performance-Class-gated → we
still query `getMaxSupportedInstances()` at runtime, risk **R3**).

---

## 3. Recommendation

**Primary: A** (single player, N renderers) — the only verified-surviving option
that gives true frame-lock **and** a drive-wide scrubber on our pinned Media3
1.10.1. It deletes the "elect audio master, sacrifice video" compromise: audio is
just another track on the same clock, so audio + all tiles are smooth together.
Same-frame switching and scrubbing collapse to one `player.seekTo(globalMs)`.

**Gate the rewrite behind a throwaway spike (A0).** The refuted claim is a real
warning: no first-party video sample, per-frame lock not guaranteed, a stall in any
one renderer pauses all tiles, and `MergingMediaSource` requires **equal period
counts** (which our lazily-grown HD playlists don't naturally satisfy). If A0 fails
on residual skew or device limits, **fall back to B** — a ~1-2 day change to the
existing loop that ships smooth (if ~±1-frame) tiles.

---

## 4. Migration plan (staged, reviewable, app stays shippable)

Keep the existing chase-sync path (V1) intact behind a feature flag until A is
proven. One PR per stage; CI green → merge; never push to `main` directly.

### Keep unchanged
- **Rust core / remux / FFI** — `resolveHd(FileKind, UInt) → String?` still yields cached `hvc1` MP4s. No encode added (core stays remux-only).
- **Timeline math** — [RouteTimeline.kt](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/RouteTimeline.kt): `locate`, `globalPosition`, `windowsOf`, `fmtTime`. Now feeds *one* `seekTo` instead of an N-way broadcast.
- **Tile layout** — [RouteClock.kt](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/RouteClock.kt): `tilePlan`, `TilePlan`, `CameraId`, `CameraTrack`.
- `hasAudio` detection (`onTracksChanged`), `Filmstrip`, qcamera-anchored scrubber/`totalMs`.

### Replace
- The N-`ExoPlayer` array (`qPlayer` + `hdPlayers`) → **one** `ExoPlayer` with custom factories.
- `master` / `timelineSource` / `activePlayers()` / `seekGlobal()` broadcast → single-player semantics.
- The sync loop ([:295-322](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L295-L322)) and `shouldResync`/`SYNC_THRESHOLD_MS`/`RESYNC_EVERY_TICKS` correction.
- Per-tile `PlayerView`/`AndroidView` → per-tile raw `Surface` plumbed to a specific renderer via `player.createMessage(renderer).setType(MSG_SET_VIDEO_OUTPUT)`.

### Stages
- **Stage 0 — Spike A0 (throwaway, not merged).** Go/no-go gate. See §6.
- **Commit 1 — Scaffolding behind flag.** `MultiCamPlayerV2.kt` + `usePlayerV2` flag; `DriveDetailScreen` chooses V1/V2. No V1 behavior change. (pure addition)
- **Commit 2 — Player core (no UI).** `MultiRenderersFactory : DefaultRenderersFactory` (emit `tileCount` `MediaCodecVideoRenderer`s + audio renderer); `TileTrackSelector : TrackSelector` (map each merged video `TrackGroup` → a distinct renderer via `FixedTrackSelection`; select nothing for an off tile); a `MergingMediaSource` builder that handles **period-count alignment** for ragged lazy HD playlists (risk R1).
- **Commit 3 — Per-tile surface routing.** Send each tile's Surface to its renderer; re-send on Compose surface recreation/rotation (risk R4); wire readiness to the existing `ready[id]` spinner.
- **Commit 4 — Wire V2 into UI.** Scrubber/filmstrip → one `seekTo`; same-frame toggle → track re-selection (no seek); audio toggle → volume on the one audio track.
- **Commit 5 — Flip default to V2** after on-device acceptance (§5) passes; keep V1 one commit from deletion.
- **Commit 6 — Delete V1** + dead constants. (pure deletion)

If A0 fails → pivot the branch to **B**: replace the corrective `seekTo` in the loop
with a per-follower proportional `setPlaybackSpeed` controller (deadband ≤1 frame,
clamp ±2-3%, suspend during `STATE_BUFFERING`/item transitions, one-shot exact seek
at toggle/scrub). Mirrors Google's `DefaultLivePlaybackSpeedControl` shape.

---

## 5. Verification plan — proving "smooth" before merge

**Merge bar — "smooth" means:**
1. **No periodic stutter** — per-renderer `DecoderCounters.droppedBufferCount` stays near-flat over a 60 s all-tiles-on window (target **< 1 dropped frame/s per tile**). This is the V1 failure mode we're killing.
2. **Frame-lock** — paused, all tiles show the same global timestamp within **≤1 frame (≤50 ms)** at 20 fps (capture surfaces, compare on a drive with a visible landmark/clock).
3. **Audio sync** — A/V drift ≤ ~1-2 frames over a 16-segment drive.

**Pure-JVM unit tests** (extend `RouteClockTest`):
- `RouteTimeline` round-trips across segment boundaries on a 16+ segment drive; window-edge off-by-ones.
- `TileTrackSelector`: mock merged `TrackGroup`s → one video track per renderer; off tiles select nothing.
- Period-alignment builder: ragged playlists (qcamera 16, HD 3-so-far) → equal period counts, no merge exception.

**Instrumented / decode tests** (Pixel; hermetic via `mock-comma-mcp` — `provision_device` + `set_state`):
- Assert one `ExoPlayer`, N renderers, N decoder instances; `getMaxSupportedInstances()` ≥ tile count or graceful degrade.
- Steady-state 60 s → drop counters flat.
- Global-stall: delay one HD source; confirm documented "all tiles pause together" + graceful recovery (validates the lazy-remux/buffering policy, R2).

**On-device acceptance** — real comma `192.168.1.100`, Pixel, real multi-segment HD drive, driven via `mobile-mcp` (screen-record for visual frame-lock review). Hard cases:
1. Toggle road/wide/driver during playback → same-frame appear, others don't stutter.
2. Seek mid-drive → all tiles land same-frame, smooth resume.
3. Audio on/off mid-play → no glitch, video unaffected.
4. 16+ segment drive → no per-segment hitch, scrubber spans whole drive.
5. Backgrounded / killed & resumed → resumes at position, sync intact.
6. Device unreachable mid-stream (`set_reachable(false)`) → graceful stall + recovery.

Merge only when drop counters are flat, all 6 hard cases pass, and recorded video shows visually locked tiles.

---

## 6. Open risks & spikes

| # | Risk / open question | Smallest experiment |
|---|---|---|
| **A0** | Does single-player/N-renderer actually frame-**lock** video, and does disabling a renderer's track **free its decoder** (not just blank the surface)? | **Go/no-go spike:** throwaway app, `MultiRenderersFactory` + `TileTrackSelector` + `MergingMediaSource`, 2 cached HD MP4s + qcamera audio, 2 SurfaceViews on the Pixel. Measure per-renderer `DecoderCounters` (flat?), pause-and-capture (same frame ≤1?), toggle a tile (decoder count drops?). |
| **R1** | `MergingMediaSource` equal-period-count requirement vs lazily/progressively grown HD playlists. | Merge a 16-segment qcamera with a 3-segment-so-far HD source via padding/placeholder periods; confirm no `IllegalMergeException` and that appending real segments doesn't re-throw. |
| **R2** | Global stall (one renderer rebuffers → all pause) vs "show tile 1 fast, stream the rest" UX. | Delay one HD source in the instrumented test; decide readiness policy (don't enable a tile's track until its segments buffer). |
| **R3** | Concurrent HEVC decoder ceiling for GRID4 (4 tiles + audio) on real/low-end devices. | `getMaxSupportedInstances()` on Pixel + a mid-range device; attempt 4 simultaneous HEVC decoders; cap/degrade if it fails. |
| **R4** | Per-tile Surface recreation in Compose (rotation/recomposition) → stuck/black tile. | Rotate repeatedly during the spike; confirm `MSG_SET_VIDEO_OUTPUT` re-send restores the tile. |
| **R5** | Custom multi-renderer path historically breaks on media3 bumps. | Pin `media3 = 1.10.1`; add the instrumented multi-renderer test to CI as a regression gate before any media3 bump. |
| **R6** | If A0 fails, does B meet "exact same-frame at toggle"? | Prototype B's controller; confirm a one-shot exact seek at the toggle moment lands same-frame, then PLL holds smoothly. |

**Bottom line:** build **A** behind a flag, **gated on Spike A0**. A0 passes →
finish the staged migration, delete chase-sync. A0 disappoints → ship **B**. D and E
are verified-unusable today and explicitly off the build path.
