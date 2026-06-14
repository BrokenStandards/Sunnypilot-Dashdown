# hdplayer-1 — diagnose the fully-downloaded-drive spinner

_Part of [hdplayer-hardening](hdplayer-hardening.md). Effort: **XS** (read-only spike, no code change)._

## Goal

Decide whether the "Preparing HD… on a drive that should be fully downloaded" symptom is:

- **(A) the active-download rebuild** — fixed by [`hdplayer-2`](hdplayer-2-live-playlist.md), or
- **(B) a genuine per-camera gap** — a segment where the camera was never recorded remotely (driver
  cam written "only if RecordFront", [file_kind.rs:8-10](../../rust/core/src/model/file_kind.rs#L8-L10)),
  handled by [`hdplayer-3`](hdplayer-3-frontier-fallback.md).

Both render identically (renderer disabled → `firstFrameRendered=false` → spinner), so the symptom
alone can't tell them apart. This spike disambiguates before any code changes.

## Why a "Complete" drive can still have holes (confirmed)

`drive_status` counts only **selected files that exist remotely**
([resume.rs:43-77](../../rust/core/src/sync_engine/resume.rs#L43-L77)); a segment lacking a camera
adds nothing to `total`, so the drive reaches **Complete** without it. But
[`resolve_local_paths`](../../rust/core/src/ffi/mod.rs#L476-L510) skips that segment for that kind, so
`CameraTrack.segmentNums` has a hole → `buildWindows` adds no HD child for it
([MultiCamPlayer.kt:211-213](../../android/app/src/main/java/org/sunnypilot/dashdown/ui/detail/MultiCamPlayer.kt#L211-L213))
→ that tile spins on those segments. So **(B) is real**, but only where a camera is genuinely absent.

## Procedure

1. Pick the drive that showed the spinner; get its `route_id` (e.g. `00000049--96b9edc930`).
2. Run, read-only, via `tools/dd-db.sh "<SQL>"` (kind tokens per [file_kind.rs:44-54](../../rust/core/src/model/file_kind.rs#L44-L54)):

   ```sql
   -- per-kind segment coverage for the drive
   SELECT f.kind, COUNT(*) AS segs_with_kind
   FROM seg_file f JOIN segment s ON s.id = f.segment_id
   WHERE s.route_id = :route
     AND f.kind IN ('qcamera','fcamera','ecamera','dcamera')
   GROUP BY f.kind;

   -- exact segments missing the driver cam (repeat for ecamera/fcamera)
   SELECT s.segment_num
   FROM segment s
   WHERE s.route_id = :route
     AND NOT EXISTS (SELECT 1 FROM seg_file f WHERE f.segment_id=s.id AND f.kind='dcamera')
   ORDER BY s.segment_num;
   ```

3. **Interpret:**
   - `dcamera`/`ecamera`/`fcamera` count **== `qcamera` count** (second query empty) → **no genuine
     gap** → the observed spinner was **(A) the rebuild** → [`hdplayer-2`](hdplayer-2-live-playlist.md)
     is the fix. (Expected for the user's footage, which keeps the driver cam on.)
   - a camera's count **< `qcamera` count** → genuine gaps at the listed segments → **(B)**; those
     tiles will spin there until [`hdplayer-3`](hdplayer-3-frontier-fallback.md) lands.
4. Confirm the (A) mechanism (already observed this session, re-confirm cleanly): on the emulator
   open a partially-downloaded drive while the download is **flapping** (failed→resume) → total
   sticks at `0:00`, all tiles spin even on seg 0; **stop the download, reopen** → clean
   (`0:00 / 101:59`, HD renders). This is the rebuild, not a gap.

## Output

A one-line verdict per offending drive: "(A) rebuild" or "(B) gap at segs [...]", which sets the
priority of steps 2 vs 3 and gives step 3 a concrete on-device fixture (a Complete drive with a real
hole, if one exists) to verify the fallback against.

## Findings (run 2026-06-13, emulator DB)

**Verdict: (A) the active-download rebuild — no genuine gaps in recent footage.**

- **100m drive `00000049--96b9edc930`**: `total_segs=102`; `qcamera/fcamera/ecamera/dcamera` remote
  rows all **= 102**. Genuine-gap query → **(no rows)**. So every segment has all three HD cameras
  remotely; the stuck-at-`0:00` spinner observed this session was **not** a camera gap (it occurred
  even on seg 0, which had HD) → it is the rebuild churn fixed by [`hdplayer-2`](hdplayer-2-live-playlist.md).
  Downloaded (complete) HD counts were `f=17, e=18, d=18` — a **ragged frontier** (the cameras don't
  all stop at the same segment), which [`hdplayer-3`](hdplayer-3-frontier-fallback.md)'s per-tile
  classification handles.
- **`00000015--f684dd21fc` (Complete)**: `ecamera 3/3, fcamera 3/3, qcamera 3/3`, **zero dcamera
  rows** — driver cam off for the whole (May) route → "camera entirely absent" → no Driver toggle
  appears (graceful, no dead spinner). Confirms recent footage keeps dcamera on.
- **`0000004a--84b6217c1c`**: all four cameras present for all 33 segments (none downloaded) — a clean
  full-coverage fixture for steps 2/3.

**Consequences:** prioritize [`hdplayer-2`](hdplayer-2-live-playlist.md) (the real fix for the user's
report). [`hdplayer-3`](hdplayer-3-frontier-fallback.md) is still needed for the **download frontier**
(common when a drive isn't fully downloaded) and is verifiable via the ragged partial-download edge
above; a *genuine mid-drive hole* doesn't exist in current fixtures, so don't block step 3 on one.

## Verification

N/A — this *is* verification. No build, no commit; findings recorded above.
