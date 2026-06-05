//! Drive grouping: turn a flat list of [`Segment`]s into [`Drive`]s.
//!
//! A *drive* is a maximal run of consecutive 1-minute segments within one route.
//! Two segments continue the same drive **iff** they share a `route_id` and their
//! `segment_num`s are consecutive; a new route or any index break starts a new
//! drive. The copyparty `ts` mtime is only a secondary *sanity* signal — it never
//! splits a drive (see [`gap_is_sane`]); an anomalous gap just logs a warning.
//!
//! This is the shared grouping core. The offline mirror scan reuses
//! [`group_segments`] against locally-scanned segments via [`local::group_local`].

pub mod local;
pub mod remote;

use crate::model::time::SEGMENT_MS;
use crate::model::{Drive, Segment, SyncStatus};

/// Advisory tolerance for the segment-to-segment mtime gap. Purely a logging
/// knob — grouping splits on route/index, never on time. 30 s comfortably
/// absorbs sub-segment finalization lag while staying well short of a genuinely
/// skipped minute (60 s).
const GAP_TOLERANCE_MS: i64 = 30_000;

/// Group pre-parsed segments into drives. Pure, infallible, and takes ownership
/// (segments are *moved* into their drives — no clones). Output is sorted by
/// `(route_id, first_segment_num)` and is order-independent of the input.
pub fn group_segments(mut segments: Vec<Segment>) -> Vec<Drive> {
    // Sort by the grouping key so the walk is linear and duplicates are adjacent.
    segments.sort_by(|a, b| {
        a.name
            .route_id
            .cmp(&b.name.route_id)
            .then(a.name.segment_num.cmp(&b.name.segment_num))
    });
    let segments = dedup_richest(segments);

    let mut drives: Vec<Drive> = Vec::new();
    let mut current: Vec<Segment> = Vec::new();

    for seg in segments {
        let split = match current.last() {
            Some(prev) => {
                let contiguous = seg.name.route_id == prev.name.route_id
                    && seg.name.segment_num == prev.name.segment_num + 1;
                if contiguous && !gap_is_sane(prev.approx_time_ms(), seg.approx_time_ms()) {
                    tracing::warn!(
                        route_id = %prev.name.route_id,
                        prev = prev.name.segment_num,
                        next = seg.name.segment_num,
                        "contiguous segments have an anomalous mtime gap; keeping in one drive"
                    );
                }
                !contiguous
            }
            None => false,
        };
        if split {
            drives.push(finalize(std::mem::take(&mut current)));
        }
        current.push(seg);
    }
    if !current.is_empty() {
        drives.push(finalize(current));
    }

    // Deterministic output order, independent of the optional time signal.
    drives.sort_by(|a, b| {
        a.route_id
            .cmp(&b.route_id)
            .then(a.first_segment_num.cmp(&b.first_segment_num))
    });
    drives
}

/// True when the wall-clock gap between two adjacent segments looks like one
/// segment length. A `None` on either side means "no time signal" — we can't
/// judge, so treat it as sane (don't warn).
fn gap_is_sane(prev: Option<i64>, next: Option<i64>) -> bool {
    match (prev, next) {
        (Some(p), Some(n)) => (n - p - SEGMENT_MS).abs() <= GAP_TOLERANCE_MS,
        _ => true,
    }
}

/// Collapse duplicate `(route_id, segment_num)` keys, keeping the *richest* view
/// (most files, then `recording` wins, then larger approx time). Input must be
/// sorted by key so duplicates are adjacent. Deterministic ⇒ order-independent.
/// Duplicates don't arise from one `list_segments`, but the function must be total.
fn dedup_richest(segments: Vec<Segment>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::with_capacity(segments.len());
    for seg in segments {
        match out.last_mut() {
            Some(prev)
                if prev.name.route_id == seg.name.route_id
                    && prev.name.segment_num == seg.name.segment_num =>
            {
                if richer(&seg, prev) {
                    *prev = seg;
                }
            }
            _ => out.push(seg),
        }
    }
    out
}

/// Total, content-based ordering: is `a` a richer view of the same segment than
/// `b`? More files, else recording-wins, else larger approx time.
fn richer(a: &Segment, b: &Segment) -> bool {
    a.files
        .len()
        .cmp(&b.files.len())
        .then(a.recording.cmp(&b.recording))
        .then(a.approx_time_ms().cmp(&b.approx_time_ms()))
        .is_gt()
}

/// Build a [`Drive`] from a non-empty, index-ordered run of one route's segments.
fn finalize(segments: Vec<Segment>) -> Drive {
    debug_assert!(!segments.is_empty(), "finalize called with no segments");
    let first = &segments[0];
    let last = &segments[segments.len() - 1];
    Drive {
        drive_key: first.name.dir_name(),
        route_id: first.name.route_id.clone(),
        first_segment_num: first.name.segment_num,
        last_segment_num: last.name.segment_num,
        start_ms: first.approx_time_ms(),
        end_ms: last.approx_time_ms().map(|t| t + SEGMENT_MS),
        segment_count: segments.len() as u32,
        recording: segments.iter().any(|s| s.recording),
        sync_state: SyncStatus::NotDownloaded,
        preserved: false,
        segments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileKind, SegmentFile, SegmentName};

    /// A segment with an optional time signal (one `qcamera.ts` file when `Some`,
    /// no files when `None`).
    fn seg(route: &str, num: u32, mtime_s: Option<i64>) -> Segment {
        seg_full(route, num, mtime_s, false, 1)
    }

    fn seg_full(
        route: &str,
        num: u32,
        mtime_s: Option<i64>,
        recording: bool,
        files: usize,
    ) -> Segment {
        let files = (0..files)
            .map(|i| SegmentFile {
                kind: FileKind::QCamera,
                name: format!("f{i}.ts"),
                remote_size: 1,
                mtime_s: mtime_s.unwrap_or(0),
            })
            .collect::<Vec<_>>();
        let files = if mtime_s.is_none() { Vec::new() } else { files };
        Segment {
            name: SegmentName {
                route_id: route.to_string(),
                segment_num: num,
            },
            files,
            recording,
        }
    }

    const A: &str = "000001a3--c20ba54385";
    const B: &str = "000001a4--aabbccddee";

    #[test]
    fn single_route_consecutive_is_one_drive() {
        let drives = group_segments(vec![
            seg(A, 0, Some(1000)),
            seg(A, 1, Some(1060)),
            seg(A, 2, Some(1120)),
        ]);
        assert_eq!(drives.len(), 1);
        let d = &drives[0];
        assert_eq!(d.segment_count, 3);
        assert_eq!(d.drive_key, format!("{A}--0"));
        assert_eq!(d.route_id, A);
        assert_eq!(d.first_segment_num, 0);
        assert_eq!(d.last_segment_num, 2);
        assert_eq!(d.start_ms, Some(1_000_000));
        // end = last mtime (1120 s) + one segment.
        assert_eq!(d.end_ms, Some(1_120_000 + SEGMENT_MS));
        assert!(!d.recording);
    }

    #[test]
    fn two_routes_are_two_drives() {
        let drives = group_segments(vec![
            seg(A, 0, Some(1000)),
            seg(A, 1, Some(1060)),
            seg(B, 0, Some(5000)),
            seg(B, 1, Some(5060)),
        ]);
        assert_eq!(drives.len(), 2);
        assert_eq!(drives[0].route_id, A);
        assert_eq!(drives[0].segment_count, 2);
        assert_eq!(drives[1].route_id, B);
        assert_eq!(drives[1].segment_count, 2);
    }

    #[test]
    fn internal_index_gap_splits() {
        // 0,1,3 within one route → [0,1] and [3].
        let drives = group_segments(vec![
            seg(A, 0, Some(1000)),
            seg(A, 1, Some(1060)),
            seg(A, 3, Some(1180)),
        ]);
        assert_eq!(drives.len(), 2);
        assert_eq!(drives[0].first_segment_num, 0);
        assert_eq!(drives[0].last_segment_num, 1);
        assert_eq!(drives[1].first_segment_num, 3);
        assert_eq!(drives[1].last_segment_num, 3);
        assert_eq!(drives[1].drive_key, format!("{A}--3"));
    }

    #[test]
    fn unordered_input_same_as_sorted() {
        let sorted = group_segments(vec![
            seg(A, 0, Some(1000)),
            seg(A, 1, Some(1060)),
            seg(B, 0, Some(5000)),
        ]);
        let shuffled = group_segments(vec![
            seg(B, 0, Some(5000)),
            seg(A, 1, Some(1060)),
            seg(A, 0, Some(1000)),
        ]);
        assert_eq!(sorted, shuffled);
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(group_segments(Vec::new()).is_empty());
    }

    #[test]
    fn single_segment_is_one_drive() {
        let drives = group_segments(vec![seg(A, 7, Some(2000))]);
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].segment_count, 1);
        assert_eq!(drives[0].first_segment_num, 7);
        assert_eq!(drives[0].last_segment_num, 7);
        assert_eq!(drives[0].drive_key, format!("{A}--7"));
    }

    #[test]
    fn no_files_segment_groups_by_index_with_no_time() {
        // Both segments lack files → grouped by index, no time signal.
        let drives = group_segments(vec![seg(A, 0, None), seg(A, 1, None)]);
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].segment_count, 2);
        assert_eq!(drives[0].start_ms, None);
        assert_eq!(drives[0].end_ms, None);
    }

    #[test]
    fn start_and_end_are_independent() {
        // First has a time, last does not → start Some, end None.
        let drives = group_segments(vec![seg(A, 0, Some(1000)), seg(A, 1, None)]);
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].start_ms, Some(1_000_000));
        assert_eq!(drives[0].end_ms, None);
    }

    #[test]
    fn recording_is_true_if_any_segment_recording() {
        let drives = group_segments(vec![
            seg_full(A, 0, Some(1000), false, 1),
            seg_full(A, 1, Some(1060), true, 1),
        ]);
        assert_eq!(drives.len(), 1);
        assert!(drives[0].recording);
    }

    #[test]
    fn duplicate_index_keeps_richest_in_both_orders() {
        let poor = seg_full(A, 0, Some(1000), false, 1);
        let rich = seg_full(A, 0, Some(1000), false, 3);
        let forward = group_segments(vec![poor.clone(), rich.clone()]);
        let backward = group_segments(vec![rich.clone(), poor.clone()]);
        assert_eq!(forward, backward);
        assert_eq!(forward.len(), 1);
        assert_eq!(forward[0].segment_count, 1);
        assert_eq!(forward[0].segments[0].files.len(), 3, "richest kept");
    }

    #[test]
    fn anomalous_mtime_gap_does_not_split() {
        // Consecutive indices but a 10-minute mtime jump: still one drive (warn only).
        let drives = group_segments(vec![seg(A, 0, Some(1000)), seg(A, 1, Some(1600))]);
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].segment_count, 2);
    }

    #[test]
    fn gap_is_sane_boundary() {
        let base = 1_000_000; // ms
                              // Exactly at tolerance: gap = SEGMENT_MS + GAP_TOLERANCE_MS → sane.
        assert!(gap_is_sane(
            Some(base),
            Some(base + SEGMENT_MS + GAP_TOLERANCE_MS)
        ));
        // One ms over → not sane.
        assert!(!gap_is_sane(
            Some(base),
            Some(base + SEGMENT_MS + GAP_TOLERANCE_MS + 1)
        ));
        // Symmetric: a backwards jump beyond tolerance is also not sane.
        assert!(!gap_is_sane(Some(base), Some(base - GAP_TOLERANCE_MS - 1)));
        // Missing time signal on either side → treated as sane.
        assert!(gap_is_sane(None, Some(base)));
        assert!(gap_is_sane(Some(base), None));
        assert!(gap_is_sane(None, None));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::model::{FileKind, SegmentFile, SegmentName};
    use proptest::prelude::*;
    use std::collections::HashSet;

    fn route_id_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("000001a3--c20ba54385".to_string()),
            Just("000001a4--aabbccddee".to_string()),
            Just("000001a5--1122334455".to_string()),
        ]
    }

    prop_compose! {
        /// One segment: a key + an optional time signal (one file when `Some`,
        /// none when `None`, exercising `approx_time_ms() == None`). mtime is
        /// generated independently of `segment_num` — proving grouping ignores it.
        fn seg_strategy()(
            route_id in route_id_strategy(),
            segment_num in 0u32..8,
            mtime_s in proptest::option::of(1_600_000_000i64..1_900_000_000i64),
            recording in any::<bool>(),
        ) -> Segment {
            let files = match mtime_s {
                Some(t) => vec![SegmentFile {
                    kind: FileKind::QCamera,
                    name: "qcamera.ts".to_string(),
                    remote_size: 1,
                    mtime_s: t,
                }],
                None => Vec::new(),
            };
            Segment { name: SegmentName { route_id, segment_num }, files, recording }
        }
    }

    prop_compose! {
        /// A `Vec<Segment>` with UNIQUE `(route_id, segment_num)` keys, so the
        /// dedup path is a no-op and the invariants below isolate grouping.
        /// (Dedup determinism is covered by a table-driven unit test.)
        fn segments_strategy()(
            raw in proptest::collection::vec(seg_strategy(), 0..24)
        ) -> Vec<Segment> {
            let mut seen = HashSet::new();
            raw.into_iter()
                .filter(|s| seen.insert((s.name.route_id.clone(), s.name.segment_num)))
                .collect()
        }
    }

    fn key(s: &Segment) -> (String, u32) {
        (s.name.route_id.clone(), s.name.segment_num)
    }

    proptest! {
        /// (a) Partition completeness: the drives' segments are exactly the input.
        #[test]
        fn partition_is_complete(input in segments_strategy()) {
            let drives = group_segments(input.clone());
            let mut got: Vec<Segment> = drives.into_iter().flat_map(|d| d.segments).collect();
            let mut exp = input;
            got.sort_by_key(key);
            exp.sort_by_key(key);
            prop_assert_eq!(got, exp);
        }

        /// (b)+(e) Each drive is non-empty, has a constant route, strictly +1
        /// indices, and derived fields that agree with its segments.
        #[test]
        fn drives_are_contiguous_and_consistent(input in segments_strategy()) {
            for d in group_segments(input) {
                prop_assert!(!d.segments.is_empty());
                prop_assert_eq!(d.segment_count as usize, d.segments.len());
                prop_assert_eq!(d.first_segment_num, d.segments.first().unwrap().name.segment_num);
                prop_assert_eq!(d.last_segment_num, d.segments.last().unwrap().name.segment_num);
                prop_assert_eq!(&d.route_id, &d.segments[0].name.route_id);
                prop_assert_eq!(&d.drive_key, &d.segments[0].name.dir_name());
                for w in d.segments.windows(2) {
                    prop_assert_eq!(&w[0].name.route_id, &w[1].name.route_id);
                    prop_assert_eq!(w[1].name.segment_num, w[0].name.segment_num + 1);
                }
            }
        }

        /// (c) Idempotence: regrouping the concatenation of all drives' segments
        /// yields the identical `Vec<Drive>`.
        #[test]
        fn grouping_is_idempotent(input in segments_strategy()) {
            let d1 = group_segments(input);
            let concat: Vec<Segment> = d1.iter().flat_map(|d| d.segments.clone()).collect();
            let d2 = group_segments(concat);
            prop_assert_eq!(d1, d2);
        }

        /// (d) Order-independence: grouping a shuffled permutation of the same
        /// input yields the identical `Vec<Drive>`.
        #[test]
        fn grouping_is_order_independent(
            (input, shuffled) in segments_strategy()
                .prop_flat_map(|input| (Just(input.clone()), Just(input).prop_shuffle()))
        ) {
            prop_assert_eq!(group_segments(input), group_segments(shuffled));
        }
    }
}
