//! Pure storage-management policy: the **segment retention window** — which local
//! segments to keep to fit a budget. Side-effect-free and unit-testable; the
//! [`SyncEngine`](super::SyncEngine) does the actual file/DB work and uses this for
//! BOTH sides so they can never fight:
//! - downloads fetch only **in-window** segments, and
//! - pruning deletes only **out-of-window** local segments,
//!
//! which makes a prune↔re-download loop structurally impossible.

use std::collections::HashSet;

use crate::model::Drive;

/// A segment's identity within a device: `(route_id, segment_num)`.
pub type SegRef = (String, u32);

/// The segments to KEEP locally for `budget_minutes` of footage: **every segment of a
/// `preserved` drive** (a user pin — always kept, never counted against the budget) plus
/// **the newest `budget` non-preserved segments** (each segment is ~1 minute, ordered
/// newest-first by approximate time). `None` budget ⇒ unlimited ⇒ keep everything.
///
/// Pure: takes `&[Drive]`, touches no I/O. Newest-first order is total
/// (`approx_time_ms`, then route, then segment number) so the result is deterministic
/// across equal timestamps.
pub fn retention_window(drives: &[Drive], budget_minutes: Option<i64>) -> HashSet<SegRef> {
    let mut keep: HashSet<SegRef> = HashSet::new();

    let Some(budget) = budget_minutes else {
        for d in drives {
            for s in &d.segments {
                keep.insert((s.name.route_id.clone(), s.name.segment_num));
            }
        }
        return keep;
    };

    // Preserved drives: every segment kept, none counted toward the budget.
    let mut candidates = Vec::new();
    for d in drives {
        if d.preserved {
            for s in &d.segments {
                keep.insert((s.name.route_id.clone(), s.name.segment_num));
            }
        } else {
            candidates.extend(d.segments.iter());
        }
    }

    // Newest-first; keep the freshest `budget` non-preserved segments.
    candidates.sort_by(|a, b| {
        b.approx_time_ms()
            .cmp(&a.approx_time_ms())
            .then(b.name.route_id.cmp(&a.name.route_id))
            .then(b.name.segment_num.cmp(&a.name.segment_num))
    });
    for s in candidates.into_iter().take(budget.max(0) as usize) {
        keep.insert((s.name.route_id.clone(), s.name.segment_num));
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileKind, Segment, SegmentFile, SegmentName, SyncStatus};

    fn seg(route: &str, n: u32, mtime_s: i64) -> Segment {
        Segment {
            name: SegmentName {
                route_id: route.to_string(),
                segment_num: n,
            },
            files: vec![SegmentFile {
                kind: FileKind::QCamera,
                name: "qcamera.ts".to_string(),
                remote_size: 1200,
                mtime_s,
            }],
            recording: false,
        }
    }

    /// A drive on `route` with one segment per `(segment_num, mtime_s)`.
    fn drive(route: &str, segs: &[(u32, i64)], preserved: bool) -> Drive {
        let segments: Vec<Segment> = segs.iter().map(|(n, m)| seg(route, *n, *m)).collect();
        Drive {
            drive_key: format!("{route}--{}", segs.first().map(|(n, _)| *n).unwrap_or(0)),
            route_id: route.to_string(),
            first_segment_num: segs.first().map(|(n, _)| *n).unwrap_or(0),
            last_segment_num: segs.last().map(|(n, _)| *n).unwrap_or(0),
            start_ms: segs.first().map(|(_, m)| m * 1000),
            end_ms: segs.last().map(|(_, m)| m * 1000),
            segment_count: segments.len() as u32,
            recording: false,
            sync_state: SyncStatus::Complete,
            preserved,
            segments,
        }
    }

    fn r(route: &str, n: u32) -> SegRef {
        (route.to_string(), n)
    }

    #[test]
    fn none_budget_keeps_everything() {
        let drives = vec![drive("a", &[(0, 1), (1, 2)], false), drive("b", &[(0, 3)], false)];
        let w = retention_window(&drives, None);
        assert_eq!(w.len(), 3);
        assert!(w.contains(&r("a", 0)) && w.contains(&r("a", 1)) && w.contains(&r("b", 0)));
    }

    #[test]
    fn keeps_newest_n_nonpreserved_segments() {
        // One drive, 3 segments oldest→newest by mtime; budget 2 keeps the newest two.
        let drives = vec![drive("a", &[(0, 1000), (1, 2000), (2, 3000)], false)];
        let w = retention_window(&drives, Some(2));
        assert_eq!(w.len(), 2);
        assert!(w.contains(&r("a", 2)) && w.contains(&r("a", 1)));
        assert!(!w.contains(&r("a", 0))); // oldest pruned
    }

    #[test]
    fn preserved_excluded_from_budget_and_always_kept() {
        // A is preserved (3 segs, OLD); B is non-preserved (3 segs, NEW). Budget 2.
        let a = drive("a", &[(0, 1000), (1, 1100), (2, 1200)], true);
        let b = drive("b", &[(0, 5000), (1, 5100), (2, 5200)], false);
        let w = retention_window(&[a, b], Some(2));
        // All of preserved A is kept (not counted); only the newest 2 of B.
        assert!(w.contains(&r("a", 0)) && w.contains(&r("a", 1)) && w.contains(&r("a", 2)));
        assert!(w.contains(&r("b", 2)) && w.contains(&r("b", 1)));
        assert!(!w.contains(&r("b", 0))); // B's oldest is out of budget
        assert_eq!(w.len(), 5);
    }

    #[test]
    fn long_drive_kept_partially() {
        // A single 5-segment drive with budget 3 → only the newest 3 segments kept.
        let drives = vec![drive("a", &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)], false)];
        let w = retention_window(&drives, Some(3));
        assert_eq!(w.len(), 3);
        assert!(w.contains(&r("a", 4)) && w.contains(&r("a", 3)) && w.contains(&r("a", 2)));
        assert!(!w.contains(&r("a", 0)) && !w.contains(&r("a", 1)));
    }

    #[test]
    fn equal_mtime_tiebreak_is_deterministic() {
        // All same mtime → newest-first falls back to route desc, then segment desc.
        let drives = vec![drive("a", &[(0, 7), (1, 7)], false), drive("b", &[(0, 7)], false)];
        let w = retention_window(&drives, Some(2));
        // Order desc: ("b",0), ("a",1), ("a",0) → keep the first two.
        assert!(w.contains(&r("b", 0)) && w.contains(&r("a", 1)));
        assert!(!w.contains(&r("a", 0)));
    }

    #[test]
    fn budget_at_least_total_keeps_all_nonpreserved() {
        let drives = vec![drive("a", &[(0, 1), (1, 2)], false)];
        let w = retention_window(&drives, Some(10));
        assert_eq!(w.len(), 2);
    }
}
