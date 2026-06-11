# Natural tiling multi-cam player — fit-correct, aspect-aware, reorderable

## Context

The drive detail screen embeds `MultiCamPlayer` in a vertically scrolling column with a **segment file
list** beneath it. Today the player is wrong in three ways:
- Every tile is forced to `Modifier.aspectRatio(16f/9f)` ([MultiCamPlayer.kt:358]) regardless of the
  real video shape (comma cameras are ~1928×1208 ≈ **16:10**, qcamera too) → video is mis-fit
  (stretched/cropped by the surface), not letterboxed.
- The layout is a fixed `TilePlan` enum with hardcoded weights ([RouteClock.kt:91-110]) — it ignores
  both the video aspect ratio and the device aspect ratio, so tiles don't fill the screen well.
- The player is boxed into a scroll column at `fillMaxWidth()`, so it never uses the available height;
  rotation recreates the Activity (no `configChanges`), interrupting playback.

**Goal:** a comprehensive player that fits videos to the display **without cropping or stretching**,
lets tiles **fill the screen**, **stacks in portrait / side-by-sides in landscape**, supports
**multiple tile strategies** (equal grid + main-with-side) sized by **video aspect × device aspect**
via a known packing algorithm, works **immersively in both orientations**, and (phase 2) supports
**drag-and-drop reordering**. Remove the segment file list.

**Decisions (user, 2026-06-11):**
- Strategy = **auto-selected with manual override** (app picks the max-area layout; a control cycles
  grid ↔ feature).
- **Immersive in both orientations** — chrome hidden, controls revealed on tap (auto-hide).
- Fit = **letterbox (no crop, no stretch)**; tiles fill the screen by maximizing fitted video area.
- Ship in **two phases**: tiling first, confirm on multiple aspect ratios, then drag-and-drop.

## Algorithm (researched)

Use the video-conferencing **"maximize fitted tile area"** grid sizing (Jitsi/LiveKit-style) plus a
**feature (main + side strip)** strategy. Core idea — a pure function over fractions of the container:

```
fun planTiles(count, containerAspect /*w/h*/, tileAspect /*w/h*/, landscape, strategy): List<TileBox>
// GRID: for cols in 1..count { rows=ceil(count/cols); cell=(1/cols,1/rows scaled to container);
//        fitted = fit(tileAspect, cell);  score = min fitted-area across tiles }
//        pick the cols maximizing score; in portrait bias rows≥cols, landscape cols≥rows.
// FEATURE: tile[0]=main; rest=strip. portrait→main top / strip row below; landscape→main left / strip col right.
// returns fractional rects (x,y,w,h in [0,1]); cells tile the container exactly (fill the screen),
// each video is letterboxed (centered, aspectRatio(tileAspect)) inside its cell → never cropped.
```

This is ~80 lines, no new dependency, and **pure/unit-testable** across device aspect ratios.

## Phase 1 — fit-correct tiling, immersive, orientation (the confirm-gate)

### 1. Layout engine (new, pure) — `ui/detail/TileLayout.kt`
`TileBox(xFrac,yFrac,wFrac,hFrac)`, `enum TileStrategy { GRID, FEATURE }`, `planTiles(...)` above, and
`autoStrategy(count, landscape): TileStrategy` (default GRID; FEATURE offered at 3–4). Replaces
`TilePlan`/`tilePlan()` in [RouteClock.kt]. Fully unit-tested (see Verification).

### 2. Per-tile fit (no crop) — `MultiCamPlayer.kt` + `MultiRenderersFactory.java`
- Capture each renderer's real display aspect from **`VideoRendererEventListener.onVideoSizeChanged`**
  (width, height, pixelWidthHeightRatio) into the existing `TileStats`, expose to Compose state.
- `CameraTile`: drop the hardcoded `aspectRatio(16f/9f)`; the tile's cell comes from the engine, and
  the `SurfaceView` inside is `Modifier.aspectRatio(displayAspect).align(Center)` → the surface matches
  the video aspect (renderer fills it with no stretch), letterboxed within the cell. Seed `displayAspect`
  with a known constant (`1928f/1208f`) until the first frame to avoid a layout jump.

### 3. Engine-driven `TileGrid` — `MultiCamPlayer.kt`
Replace the `when(TilePlan)` nested Row/Column ([MultiCamPlayer.kt:414-459]) with a
`BoxWithConstraints` that calls `planTiles(visible.size, maxWidth/maxHeight, tileAspect, landscape,
strategy)` and positions each tile via `Modifier.offset/size` (or a small custom `Layout`) from the
returned fractions. Keep `testTag("drive_detail_player")` on the container and `drive_tile_*` on tiles.

### 4. Full-bleed immersive screen — `DriveDetailScreen.kt`, `MainActivity.kt`
- **Remove** the segment list: `HorizontalDivider()` + `drive.segments.forEach { SegmentBlock }`
  ([:197-198]) and the `SegmentBlock` composable ([:204-218]); drop now-unused `formatBytes` if no
  other caller.
- Restructure detail into a full-bleed `Box`: the player fills the whole content area (no
  `verticalScroll`, no 16dp inset around the player). Title/status/download/export/back/star + the
  transport controls (play, scrubber, clock, camera toggles, audio, **strategy-cycle**) move into a
  **tap-to-reveal overlay** (top scrim + bottom scrim) that auto-hides after ~3 s; tap toggles it.
- **Immersive both orientations**: on entering detail, hide system bars via
  `WindowInsetsControllerCompat` (sticky immersive) + edge-to-edge; restore on leave (DisposableEffect).

### 5. Smooth rotation — `AndroidManifest.xml`
Add `android:configChanges="orientation|screenSize|smallestScreenSize|screenLayout|keyboardHidden"` to
the single activity so rotation no longer recreates it (ExoPlayer + immersive state survive); Compose
still recomposes via `LocalConfiguration`, and the engine re-runs for the new orientation.

### 6. Strategy auto + manual override
`var strategy by remember { mutableStateOf(autoStrategy(count, landscape)) }`; the overlay shows a
cycle button (GRID ↔ FEATURE, and rotate which tile is "main"). Auto re-applies when count/orientation
changes unless the user has overridden this session.

### Phase 1 verification (the confirm-gate)
- **Unit tests** `TileLayoutTest.kt`: matrix of container aspects {portrait phone 1080×2400, landscape
  2400×1080, square, tablet 4:3, foldable ~1:1} × counts {1..4} × strategies. Assert: rects in [0,1],
  non-overlapping, GRID cells cover the container, each fitted video ⊆ its cell (no crop), portrait
  ⇒ rows≥cols and landscape ⇒ cols≥rows for n=2, and the chosen grid maximizes min fitted area on
  known cases.
- **On-device**: boot emulator (docs/TESTING.md §4 runbook); drive `DriveDetailRoute` for a downloaded
  drive; screenshot via mobile-mcp at **multiple AVD profiles** (phone P/L, 7" tablet, foldable) and
  confirm: no crop/stretch, tiles fill the screen, rotate is smooth (playback continues), controls
  reveal/hide on tap, strategy cycles. Existing `MultiCamHevcPlaybackLiveTest` (testTag
  `drive_detail_player`) still passes; the `play_drive` Maestro flow still passes.
- Build gates: `:app:assembleDebug :app:testDebugUnitTest :app:compileDebugAndroidTestKotlin ktfmtCheck`
  + `cargo` unaffected (no core change).
- **STOP and confirm with the user** that tiling looks right on multiple aspect ratios before phase 2.

## Phase 2 — natural drag-and-drop reordering (after confirm)

Tiles render from an **ordered list of visible cameras**; the engine maps index→box. So reordering is
just permuting that list — works for **every** strategy (grid and feature) with no layout rewrite.

- **Interaction** (custom, no library — our FEATURE boxes aren't a uniform `LazyGrid`, so
  `Reorderable`/`LazyVerticalGrid` doesn't fit): `Modifier.pointerInput` +
  `detectDragGesturesAfterLongPress`. Long-press lifts a tile (scale + elevation + haptic); on drag,
  hit-test the pointer against the engine's px rects to find the target; on release, swap (or
  insert-and-shift) the two indices in the ordered list and animate boxes to their new fractions
  (`animateFloatAsState`).
- **SurfaceView z-order caveat**: a moving `SurfaceView` punches through overlays. During a drag, float
  a **static snapshot** (last `TextureView`/bitmap frame or a labeled placeholder) for the dragged tile
  and keep the live surfaces in place; swap back on drop. (Alternative: migrate tiles to `TextureView`,
  which composites normally but costs a copy — decide during phase 2 based on the snapshot approach.)
- Order persists for the session (`remember`/rememberSaveable); persisting per-device is out of scope.
- **Verification**: a unit test for the reorder/swap reducer; an instrumented/Maestro check that
  long-press-drag swaps two `drive_tile_*` tiles; manual multi-orientation check.

## Files
- **New:** `ui/detail/TileLayout.kt` (engine + strategies), `ui/detail/TileLayoutTest.kt` (unit).
- **Edit:** `ui/detail/MultiCamPlayer.kt` (engine-driven `TileGrid`, per-tile fit, `VideoSize` capture,
  strategy state, tap-to-reveal controls, phase-2 drag), `ui/detail/RouteClock.kt` (remove
  `TilePlan`/`tilePlan`), `ui/detail/DriveDetailScreen.kt` (remove segment list, full-bleed immersive,
  chrome→overlay), `MainActivity.kt` (immersive window controller per detail screen),
  `AndroidManifest.xml` (`configChanges`), `MultiRenderersFactory.java` (expose `onVideoSizeChanged`).
- **No Rust/core changes.** No new Gradle dependency (Compose BOM 2025.01.01 + Media3 1.10.1 suffice).

## Risks / mitigations
- **Fit correctness** — surface sized to the video's display aspect (from `onVideoSizeChanged`) avoids
  stretch; seed with `1928:1208` so the pre-first-frame layout rarely shifts.
- **SurfaceView drag z-order** (phase 2) — float a snapshot during drag, not the live surface.
- **Immersive + `configChanges`** — restore bars on screen-leave (DisposableEffect); test rotation +
  background/restore.
- **App-wide `configChanges`** — single-Activity Compose app already reacts to config via Compose; low
  risk, smoke-test other screens rotate fine.

## Out of scope
Persisting tile order across launches; per-device layout memory; a PlayerView migration; pinch-zoom of
a tile; picture-in-picture. Background auto-download/segment-pickup are unaffected.
