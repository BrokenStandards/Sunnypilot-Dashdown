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
| **A0** ✅ **PASS** | Does single-player/N-renderer actually frame-**lock** video, and does disabling a renderer's track **free its decoder**? | **DONE on the Pixel** (commit `800f559`, debug-only `app/src/debug/.../spike/`). Results in §7. |
| **R1** ✅ **PASS** | `MergingMediaSource` equal-period-count requirement vs multi-segment/ragged playlists. | **DONE** — dissolved by the design (§7). Use a player **playlist of per-segment merges** (one window per segment), so every merge is 1-period → mismatch impossible, and the drive timeline reuses RP3's window math. Verified seamless boundary crossings + cross-window seek on the Pixel. |
| **R2** | Global stall (one renderer rebuffers → all pause) vs "show tile 1 fast, stream the rest" UX. | Delay one HD source in the instrumented test; decide readiness policy (don't enable a tile's track until its segments buffer). |
| **R3** 🟡 **mostly clear** | Concurrent HEVC decoder ceiling for GRID4 on real/low-end devices. | **3 simultaneous HEVC + audio = drop-free, locked on the Pixel** (§7). GRID4 ≤4 video decoders < CDD-guaranteed 6. Still query `getMaxSupportedInstances()` at runtime for low-end devices + degrade. |
| **R4** | Per-tile Surface recreation in Compose (rotation/recomposition) → stuck/black tile. | Rotate repeatedly during the spike; confirm `MSG_SET_VIDEO_OUTPUT` re-send restores the tile. |
| **R5** | Custom multi-renderer path historically breaks on media3 bumps. | Pin `media3 = 1.10.1`; add the instrumented multi-renderer test to CI as a regression gate before any media3 bump. |
| **R6** | If A0 fails, does B meet "exact same-frame at toggle"? | Prototype B's controller; confirm a one-shot exact seek at the toggle moment lands same-frame, then PLL holds smoothly. |

---

## 7. Spike A0 results — **PASS** (2026-06-10, Pixel 10 Pro XL, commit `800f559`)

Throwaway debug-only harness (`app/src/debug/.../spike/`): one `ExoPlayer`,
`MultiRenderersFactory` (N video renderers), a custom `TileTrackSelector`, a
`MergingMediaSource`, per-tile `SurfaceView`s, per-renderer `DecoderCounters`. Ran
against the real cached HD MP4s of drive `00000043--050c69d7d8` seg 0.

**Key correction to the plan — `MappingTrackSelector` does NOT work for this.**
In media3 1.10.1 (confirmed in source) `MappingTrackSelector.findRenderer` sets
`preferUnassociatedRenderer = (group.type == C.TRACK_TYPE_METADATA)` — hardcoded,
**no setter**. So for *video* it never spreads: all video groups map to renderer 0
and every other video renderer stays decoder-less (observed: tile 1 black, one
decoder). **Fix (validated):** extend `TrackSelector` **directly** and assign the
k-th merged video group → the k-th video renderer positionally, returning a
hand-built `TrackSelectorResult(configs, selections, Tracks.EMPTY, null)`. This is
the production pattern for Commit 2 — *not* `MappingTrackSelector`.

**Measured outcomes (all green):**
- **Multi-video frame-lock:** 2 HEVC tiles, one clock — `r0`/`r1` `rendered=1200/1200`, `dropped=0/0`, in lockstep the whole 60s segment (≈20 fps). Two independent HW decoders, never seeked.
- **Frame-lock visual:** paused → both tiles on the same instant (road + wide of the same night scene).
- **Decoder release on toggle:** tile off → its HW codec releases (`GC2_Dec onRelease`), counters go null; back on → decoder recreated, renders to its pre-attached surface. So GRID4 won't pin codecs for hidden cameras, and same-frame toggle = track reselect (no seek).
- **Audio + 2 video together (the old failure mode):** qcamera AAC on the audio renderer + 2 HEVC tiles — steady-state `dropped=0` on both tiles, **zero audio underruns**. Only cost: a one-time ~0.9 s (~18-frame) startup catch-up when the audio clock engages (mitigate by buffering before play; otherwise an imperceptible initial settle).
- **Surface routing:** attaching a `Surface` to a renderer *before* it has a selected track is a safe no-op; the decoder initializes to the stored surface on selection. Per-renderer `MSG_SET_VIDEO_OUTPUT` routing works; do **not** call `player.setVideoSurface` (broadcasts to all).

### R1 + R3 results — **PASS** (same harness, multi-segment mode)

Extended the spike to a **player playlist of per-segment `MergingMediaSource`s** (one
window = one segment) — the candidate production shape — and ran road+wide+audio over
6 segments, then road+wide+driver+audio over 4.

- **Seamless segment boundary crossing (continuous playback):** at the seg0→seg1 roll-over, `win` 0→1, `pos` 58.6s→0.4s, `rendered` kept climbing at a steady ~20 fps with **no stall and zero new drops**; both tiles stayed locked.
- **Cross-window seek** (jump to next segment's start): lands cleanly, both tiles re-render in exact lockstep, smooth resume, `dropped=0`. Frame-lock visually confirmed across the seek (downtown intersection, both cams same instant).
- **No `MergingMediaSource` period-count problem, ever:** per-segment merges are uniformly 1-period, so ragged/lazy availability can't trigger `REASON_PERIOD_COUNT_MISMATCH`. A segment missing a camera just blanks that tile for that window (per-window track selection).
- **R3 — 3 simultaneous HEVC decoders + audio:** all three tiles in lockstep, `dropped` flat at the one-time startup catch-up (drop-free steady state). GRID4 (≤4 video decoders) sits under the CDD-guaranteed 6.

### Finalized production design (validated end-to-end)

- One `ExoPlayer`; `MultiRenderersFactory` building N video renderers (N = max tiles + 1 qcamera-video slot when audio is merged) + the default audio renderer; `buildSecondaryVideoRenderer` → null (no pre-warming).
- A **direct `TrackSelector`** (`TileTrackSelector`) mapping the k-th merged video group → the k-th video renderer **positionally**, per window — NOT `MappingTrackSelector`.
- Player **playlist**: `setMediaSources([ per-segment MergingMediaSource(true, true, cams… , qcamera) ])`, one window per segment.
- Each tile's `Surface` routed to its renderer via `player.createMessage(renderer).setType(MSG_SET_VIDEO_OUTPUT)` (never `setVideoSurface`, which broadcasts).
- Timeline/scrubber/filmstrip = RP3's `windowsOf`/`locate`/`globalPosition` (windows = segments); seek = `seekTo(windowIndex, offsetMs)`.
- Camera toggle = flip the enabled flag + `trackSelector` re-selection (no seek); a disabled tile's HW decoder is released.
- Audio = qcamera's audio group on the audio renderer (same clock → no audio glitch).

**Still to handle during the build:** R4 (Compose `Surface` recreation on rotation —
re-send `MSG_SET_VIDEO_OUTPUT`), the ~0.9 s audio-engage startup catch-up (pre-buffer),
ragged-segment tile-blanking polish, and low-end `getMaxSupportedInstances()` gating.

---

**Bottom line:** A0 **passed decisively** — the core single-player/N-renderer
mechanism frame-locks video + audio with zero steady-state drops, and the one big
unknown (track routing) is solved with a **direct `TrackSelector`**. Proceed with
**A**: next de-risk **R1** (multi-segment merge), then the staged migration behind a
flag, then delete chase-sync. Fallback **B** stays in reserve only if R1/R3 surprise
us. D and E remain verified-unusable and off the build path.
