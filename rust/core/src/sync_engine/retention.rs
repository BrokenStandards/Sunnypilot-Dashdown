//! Pure storage-management policy for M6: which local drives to prune to fit a
//! retention budget, and whether a drive may be auto-deleted from the comma
//! device. All functions here are side-effect-free and unit-testable; the
//! [`SyncEngine`](super::SyncEngine) does the actual file/DB/HTTP work.

use crate::model::{Drive, SyncStatus};
use crate::storage::paths::dir_rel;

/// Whether a drive holds local data that occupies mirror space (and so counts
/// toward the retention budget). `Downloading` is excluded — it is an in-flight
/// job we must never evict from under itself.
fn has_local_data(d: &Drive) -> bool {
    matches!(d.sync_state, SyncStatus::Complete | SyncStatus::Partial)
}

/// The drive_keys to prune so the mirror fits `budget_minutes` of footage,
/// newest kept first. `None` ⇒ unlimited ⇒ never prune.
///
/// Only drives with local data are considered (others occupy nothing). Drives
/// are walked newest-first; each consumes `segment_count` minutes (segments are
/// ~1 min). Once the running total exceeds the budget, every *older*,
/// non-`preserved` drive is pruned. Preserved drives consume budget (they really
/// occupy disk) but are never returned — a user pin always wins, even if that
/// means staying over budget. Pure: takes `&[Drive]`, touches no I/O.
pub fn plan_prune(drives: &[Drive], budget_minutes: Option<i64>) -> Vec<String> {
    let Some(budget) = budget_minutes else {
        return Vec::new();
    };

    // Newest-first by end_ms, then start_ms, then last segment index, then key
    // (a total order so the result is deterministic across equal timestamps).
    let mut local: Vec<&Drive> = drives.iter().filter(|d| has_local_data(d)).collect();
    local.sort_by(|a, b| {
        b.end_ms
            .cmp(&a.end_ms)
            .then(b.start_ms.cmp(&a.start_ms))
            .then(b.last_segment_num.cmp(&a.last_segment_num))
            .then(b.drive_key.cmp(&a.drive_key))
    });

    let mut kept_minutes: i64 = 0;
    let mut over = false;
    let mut prune = Vec::new();
    for d in local {
        kept_minutes += d.segment_count as i64;
        // A drive that brings the running total to exactly the budget is kept;
        // the first one strictly over flips us into pruning the older tail.
        if over {
            if !d.preserved {
                prune.push(d.drive_key.clone());
            }
        } else if kept_minutes > budget {
            over = true;
            if !d.preserved {
                prune.push(d.drive_key.clone());
            }
        }
    }
    prune
}

/// Whether a drive may be auto-deleted from the comma device. All three guards
/// must hold: the selection is fully mirrored (`Complete`), the drive is not
/// actively recording, and its last segment ended at least `min_age_min` ago
/// (so the device is no longer writing to it). `now_ms`/`end_ms` are epoch ms;
/// `end_ms == None` ⇒ ineligible (age cannot be proven). Pure + injectable clock.
pub fn eligible_for_remote_delete(drive: &Drive, now_ms: i64, min_age_min: i64) -> bool {
    drive.sync_state == SyncStatus::Complete
        && !drive.recording
        && drive
            .end_ms
            .is_some_and(|end| now_ms - end >= min_age_min * 60_000)
}

/// The remote paths to delete for one drive: the whole segment directories
/// (recursive on the server), maximizing reclaimed device space. One entry per
/// segment, each with a trailing slash (see [`dir_rel`]).
pub fn remote_delete_targets(drive: &Drive, realdata_rel: &str) -> Vec<String> {
    drive
        .segments
        .iter()
        .map(|seg| dir_rel(realdata_rel, &seg.name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Segment, SegmentName};

    const RR: &str = "realdata/";

    /// A drive with `n` segments, the given local-data status and pin, timed so
    /// that a larger `end` is "newer".
    fn drive(
        key: &str,
        segs: u32,
        end_ms: Option<i64>,
        state: SyncStatus,
        preserved: bool,
    ) -> Drive {
        let segments = (0..segs)
            .map(|i| Segment {
                name: SegmentName {
                    route_id: key.to_string(),
                    segment_num: i,
                },
                files: vec![],
                recording: false,
            })
            .collect();
        Drive {
            drive_key: format!("{key}--0"),
            route_id: key.to_string(),
            first_segment_num: 0,
            last_segment_num: segs.saturating_sub(1),
            start_ms: end_ms.map(|e| e - segs as i64 * 60_000),
            end_ms,
            segment_count: segs,
            recording: false,
            sync_state: state,
            preserved,
            segments,
        }
    }

    // ---- plan_prune -------------------------------------------------------

    #[test]
    fn none_budget_never_prunes() {
        let d = vec![drive("a", 100, Some(1_000), SyncStatus::Complete, false)];
        assert!(plan_prune(&d, None).is_empty());
    }

    #[test]
    fn under_budget_keeps_everything() {
        let d = vec![
            drive("a", 5, Some(2_000), SyncStatus::Complete, false),
            drive("b", 5, Some(1_000), SyncStatus::Complete, false),
        ];
        assert!(plan_prune(&d, Some(20)).is_empty());
    }

    #[test]
    fn over_budget_prunes_oldest_keeps_newest() {
        // newest=c(3), then b(2), then a(1). Budget 10 keeps c+b (10 exactly),
        // prunes a.
        let d = vec![
            drive("a", 5, Some(1_000), SyncStatus::Complete, false),
            drive("b", 5, Some(2_000), SyncStatus::Complete, false),
            drive("c", 5, Some(3_000), SyncStatus::Complete, false),
        ];
        assert_eq!(plan_prune(&d, Some(10)), vec!["a--0".to_string()]);
    }

    #[test]
    fn boundary_exactly_at_budget_keeps_all() {
        let d = vec![
            drive("a", 5, Some(1_000), SyncStatus::Complete, false),
            drive("b", 5, Some(2_000), SyncStatus::Complete, false),
        ];
        assert!(plan_prune(&d, Some(10)).is_empty());
    }

    #[test]
    fn one_over_budget_prunes_oldest() {
        let d = vec![
            drive("a", 5, Some(1_000), SyncStatus::Complete, false),
            drive("b", 5, Some(2_000), SyncStatus::Complete, false),
        ];
        // Budget 9: keep b (5), a brings total to 10 > 9 ⇒ prune a.
        assert_eq!(plan_prune(&d, Some(9)), vec!["a--0".to_string()]);
    }

    #[test]
    fn preserved_is_skipped_but_consumes_budget() {
        // newest=c(preserved), b, a. Budget 6. c consumes 5 (kept, pinned).
        // b brings total to 10 > 6 ⇒ over; b pruned. a also pruned.
        // The pinned c does NOT save b from eviction — it still eats budget.
        let d = vec![
            drive("a", 5, Some(1_000), SyncStatus::Complete, false),
            drive("b", 5, Some(2_000), SyncStatus::Complete, false),
            drive("c", 5, Some(3_000), SyncStatus::Complete, true),
        ];
        let mut got = plan_prune(&d, Some(6));
        got.sort();
        assert_eq!(got, vec!["a--0".to_string(), "b--0".to_string()]);
    }

    #[test]
    fn preserved_old_drive_never_pruned() {
        // Budget 5: newest c fills it; b goes over and is pruned; a is also over
        // but pinned ⇒ kept. Proves a pin survives eviction.
        let d = vec![
            drive("a", 5, Some(1_000), SyncStatus::Complete, true),
            drive("b", 5, Some(2_000), SyncStatus::Complete, false),
            drive("c", 5, Some(3_000), SyncStatus::Complete, false),
        ];
        assert_eq!(plan_prune(&d, Some(5)), vec!["b--0".to_string()]);
    }

    #[test]
    fn only_local_data_drives_considered() {
        // NotDownloaded/Downloading/Failed occupy nothing ⇒ never pruned even
        // though they would blow a tiny budget if counted.
        let d = vec![
            drive("a", 100, Some(1_000), SyncStatus::NotDownloaded, false),
            drive("b", 100, Some(2_000), SyncStatus::Downloading, false),
            drive("c", 100, Some(3_000), SyncStatus::Failed, false),
            drive("d", 5, Some(4_000), SyncStatus::Complete, false),
        ];
        assert!(plan_prune(&d, Some(10)).is_empty());
    }

    #[test]
    fn partial_drives_count_and_can_be_pruned() {
        let d = vec![
            drive("a", 8, Some(1_000), SyncStatus::Partial, false),
            drive("b", 5, Some(2_000), SyncStatus::Complete, false),
        ];
        // newest=b(5), a(8) brings total to 13 > 5 ⇒ prune a (Partial counts).
        assert_eq!(plan_prune(&d, Some(5)), vec!["a--0".to_string()]);
    }

    #[test]
    fn equal_end_ms_tie_break_is_deterministic() {
        // Same end_ms; order resolved by start_ms then last_seg then key. With a
        // budget that keeps exactly one, the *newest by tie-break* is kept.
        let d = vec![
            drive("a", 5, Some(1_000), SyncStatus::Complete, false),
            drive("b", 5, Some(1_000), SyncStatus::Complete, false),
        ];
        // Both end at 1000; start_ms equal; last_seg equal; key "b">"a" so b is
        // "newer" → a pruned. Budget 5 keeps one.
        assert_eq!(plan_prune(&d, Some(5)), vec!["a--0".to_string()]);
    }

    // ---- eligible_for_remote_delete --------------------------------------

    const MIN_AGE: i64 = 30; // minutes
    const AGE_MS: i64 = MIN_AGE * 60_000;

    #[test]
    fn eligible_exactly_at_min_age() {
        let d = drive("a", 1, Some(0), SyncStatus::Complete, false);
        assert!(eligible_for_remote_delete(&d, AGE_MS, MIN_AGE));
    }

    #[test]
    fn ineligible_one_ms_under_min_age() {
        let d = drive("a", 1, Some(0), SyncStatus::Complete, false);
        assert!(!eligible_for_remote_delete(&d, AGE_MS - 1, MIN_AGE));
    }

    #[test]
    fn ineligible_when_recording() {
        let mut d = drive("a", 1, Some(0), SyncStatus::Complete, false);
        d.recording = true;
        assert!(!eligible_for_remote_delete(&d, AGE_MS, MIN_AGE));
    }

    #[test]
    fn ineligible_when_not_complete() {
        for state in [
            SyncStatus::NotDownloaded,
            SyncStatus::Partial,
            SyncStatus::Downloading,
            SyncStatus::Failed,
        ] {
            let d = drive("a", 1, Some(0), state, false);
            assert!(
                !eligible_for_remote_delete(&d, AGE_MS, MIN_AGE),
                "{state:?} must not be eligible"
            );
        }
    }

    #[test]
    fn ineligible_when_end_ms_unknown() {
        let d = drive("a", 1, None, SyncStatus::Complete, false);
        assert!(!eligible_for_remote_delete(&d, AGE_MS, MIN_AGE));
    }

    // ---- remote_delete_targets -------------------------------------------

    #[test]
    fn delete_targets_are_segment_dirs() {
        let d = drive(
            "000001a3--c20ba54385",
            3,
            Some(1_000),
            SyncStatus::Complete,
            false,
        );
        assert_eq!(
            remote_delete_targets(&d, RR),
            vec![
                "realdata/000001a3--c20ba54385--0/".to_string(),
                "realdata/000001a3--c20ba54385--1/".to_string(),
                "realdata/000001a3--c20ba54385--2/".to_string(),
            ]
        );
    }
}
