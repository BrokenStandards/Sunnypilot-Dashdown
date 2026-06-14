# HD player hardening — post-remux robustness (`hdplayer-*`)

## What this is

The in-memory **lazy HEVC remux** landed (PR #40, [inmemory-lazy-remux.md](inmemory-lazy-remux.md)).
On-device debugging with real comma footage on the emulator (see memory
`emulator-real-comma-footage`) validated it — remux ~1.6 s/segment/camera, no `.hevc.mp4` written,
interior boundaries seamless — and surfaced four follow-ups. This umbrella tracks them.

**Naming convention (per request):** each follow-up is its own sub-plan whose filename carries the
**`hdplayer-` prefix** and a sequence number — `hdplayer-<n>-<slug>.md` — so it is obvious which
parent effort it belongs to (not bare "Phase A/B"). Steps are listed in recommended execution order.

## The steps

| Step | File | Fixes | Effort |
|---|---|---|---|
| 1 | [`hdplayer-1-diagnose-edge.md`](hdplayer-1-diagnose-edge.md) | Confirm whether the "spinner on a *fully-downloaded* drive" is the active-download **rebuild** (step 2) or a **genuine per-camera gap** (step 3). | XS |
| 2 | [`hdplayer-2-live-playlist.md`](hdplayer-2-live-playlist.md) | Player rebuilds whenever `hdCameras` grows → stuck at `0:00` / permanent spinner while a drive downloads. | M |
| 3 | [`hdplayer-3-frontier-fallback.md`](hdplayer-3-frontier-fallback.md) | Indefinite "Preparing HD…" on an HD tile past the download frontier **or** on a segment a camera genuinely lacks. | S |
| 4 | [`hdplayer-4-lru-budget.md`](hdplayer-4-lru-budget.md) | Remux LRU (64 MB floor) can't cache one 3-cam window (~111 MB) → every fresh seek re-remuxes. | S |

## Shared ground truth (researched, with `file:line`)

- **"Preparing HD…"** shows when a renderer has no track for the current window:
  `firstFrameRendered` is set `true` on `onRenderedFirstFrame`, `false` on `onVideoDisabled`
  ([MultiRenderersFactory.java:82-96](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiRenderersFactory.java#L82-L96)),
  mirrored into `ready[i]` every 100 ms
  ([MultiCamPlayer.kt:279-282](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L279-L282))
  and consumed by `CameraTile` ([:498-505](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L498-L505)).
- **One `MergingMediaSource` per qcamera segment**, windows 1:1 with qcamera; an HD child is added
  only when `q.segmentNum in segsOf[cam]`
  ([MultiCamPlayer.kt:209-220](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L209-L220)).
  `segsOf` = `CameraTrack.segmentNums` ([:203-206](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L203-L206)).
  **Both the frontier and a genuine camera gap reduce to the same predicate** `seg ∉ segsOf[cam]`.
- **`segmentNums`** comes from `DriveDetailViewModel.resolveHdCameras()`
  ([:91-97](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/DriveDetailViewModel.kt#L91-L97))
  → `driveLocalPaths` → [`resolve_local_paths`](../../rust/core/src/ffi/mod.rs#L476-L510), which
  skips a segment when its `seg.files` lack the kind or it isn't `mirror.is_complete` — so a
  *Complete* drive can still have per-camera holes (driver cam is written "only if RecordFront",
  [file_kind.rs:8-10](../../rust/core/src/model/file_kind.rs#L8-L10)).
- **The LRU is already stable** (`remember(deviceId, driveKey)`,
  [MultiCamPlayer.kt:149-157](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L149-L157))
  and keyed by `HdMediaUri` — so rebuilding the playlist re-reads already-remuxed windows as **cache
  hits**, no double-remux. This is why step 2's delta approach is cheap.

## Sequencing & rationale

1. **Step 1 first** — one read-only DB query on the offending drive decides whether the user's
   report is step 2 (rebuild) or step 3 (genuine gap). Cheap, de-risks the rest.
2. **Step 2** — the core correctness fix; independent of 3/4.
3. **Step 3** — UI; **subsumes the Area-4 genuine-gap UX** (same predicate). Touches the same file
   as step 2 ([MultiCamPlayer.kt](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt)),
   so land it after step 2 to avoid conflicts.
4. **Step 4** — perf; mostly independent (LRU + manifest), do last.

Each step is its own branch + PR + commit (per the master-plan "commit per milestone" rule).
Steps 2–4 enter plan mode individually only if their sub-plan needs refinement; the sub-plan files
here are detailed enough to implement directly.

## Shared verification

- **Hermetic:** `:app:testDebugUnitTest` + `:app:ktfmtCheck` (JDK 17); `cargo test`/`fmt`/`clippy`.
- **On-device:** emulator pointed at the **real comma** `192.168.1.181:8080` for decodable HD
  (memory `emulator-real-comma-footage`); `tools/dd-db.sh` (read-only) for DB state. Never
  `connectedAndroidTest` on the Pixel (memory `dont-uninstall-app-physical-device`).
- The 3-segment drive `00000015--f684dd21fc` (fully downloaded HD) and the 100-min
  `00000049--96b9edc930` (HD prefix → a real frontier) are the standing fixtures from this session.

## Out of scope

iOS player parity; streaming un-downloaded footage from copyparty; changing the remux algorithm or
the qcamera path. The download **flap** itself (failed→resume churn, `DownloadService.kt:59-66`) is a
separate robustness question — step 2 makes the player immune to it, but a dedicated look at why the
download interrupts is tracked elsewhere.
