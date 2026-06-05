//! Resume classification: derive a file's / drive's download state from the
//! local mirror (the source of truth) vs. the known remote sizes. Used by
//! [`SyncEngine::reconcile_device`](super::SyncEngine::reconcile_device) to
//! recompute drive status from disk and by the downloader to skip complete files.

use crate::model::{DownloadState, Drive, FileSelection, SyncStatus};
use crate::storage::{paths::file_rel, MirrorStore};

/// Stable `download_job.error` marker for a job that was `running` when the
/// process restarted (a crash/kill, not a real transport failure).
pub(crate) const INTERRUPTED: &str = "interrupted";

/// Classify one file's local state against its known remote size.
///
/// Final-first: a committed file decides `Complete` vs `SizeMismatch`; otherwise
/// a stray `.part` means an interrupted (resumable) download, else `Missing`.
/// Infallible — every `rel` here comes from [`file_rel`] over parsed names, so a
/// path error is a logged surprise that degrades to `Missing`.
pub fn classify_file(mirror: &MirrorStore, rel: &str, remote_size: u64) -> DownloadState {
    if mirror.is_complete(rel) {
        return match mirror.local_size(rel) {
            Some(sz) if sz == remote_size => DownloadState::Complete,
            _ => DownloadState::SizeMismatch,
        };
    }
    match mirror.part_path(rel) {
        Ok(p) if p.exists() => DownloadState::InProgress,
        Ok(_) => DownloadState::Missing,
        Err(e) => {
            tracing::warn!(rel, error = %e, "classify_file: bad path, treating as Missing");
            DownloadState::Missing
        }
    }
}

/// Disk-derived sync status of a drive against `selection`. Produces only
/// `NotDownloaded` / `Partial` / `Complete` — `Downloading`/`Failed` are
/// job-lifecycle states owned by the download path, not derivable from disk.
///
/// The "a later contiguous remote segment is missing locally → Partial" rule
/// needs no separate check: after `sync_now` refreshes the index, a grown drive
/// includes the new segment whose files classify `Missing`, yielding `Partial`.
pub fn drive_status(
    mirror: &MirrorStore,
    drive: &Drive,
    selection: &FileSelection,
    realdata_rel: &str,
) -> SyncStatus {
    let mut total = 0usize;
    let mut complete = 0usize;
    let mut missing = 0usize;
    for seg in &drive.segments {
        for f in &seg.files {
            if !selection.includes(f.kind) {
                continue;
            }
            total += 1;
            let rel = file_rel(realdata_rel, &seg.name, &f.name);
            match classify_file(mirror, &rel, f.remote_size) {
                DownloadState::Complete => complete += 1,
                DownloadState::Missing => missing += 1,
                // InProgress / SizeMismatch are partial contributors.
                _ => {}
            }
        }
    }
    if total == 0 {
        // Nothing selected ⇒ nothing outstanding.
        SyncStatus::Complete
    } else if complete == total {
        SyncStatus::Complete
    } else if missing == total {
        SyncStatus::NotDownloaded
    } else {
        SyncStatus::Partial
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileKind, Segment, SegmentFile, SegmentName};
    use std::fs;

    const RR: &str = "realdata/";
    const ROUTE: &str = "000001a3--c20ba54385";

    fn store() -> (tempfile::TempDir, MirrorStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = MirrorStore::new(dir.path());
        (dir, store)
    }

    /// Place a committed final file at `rel` with `bytes` bytes.
    fn place_final(store: &MirrorStore, rel: &str, len: usize) {
        let p = store.final_path(rel).unwrap();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, vec![7u8; len]).unwrap();
    }

    /// Place a `.part` at `rel` with `len` bytes (an interrupted download).
    fn place_part(store: &MirrorStore, rel: &str, len: usize) {
        let p = store.part_path(rel).unwrap();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, vec![1u8; len]).unwrap();
    }

    fn rel(name: &str) -> String {
        format!("{RR}{ROUTE}--0/{name}")
    }

    #[test]
    fn classify_file_states() {
        let (_d, s) = store();
        // Missing: nothing on disk.
        assert_eq!(
            classify_file(&s, &rel("qcamera.ts"), 1200),
            DownloadState::Missing
        );
        // InProgress: only a `.part`.
        place_part(&s, &rel("rlog.zst"), 100);
        assert_eq!(
            classify_file(&s, &rel("rlog.zst"), 300),
            DownloadState::InProgress
        );
        // Complete: final present, size matches.
        place_final(&s, &rel("qlog.zst"), 100);
        assert_eq!(
            classify_file(&s, &rel("qlog.zst"), 100),
            DownloadState::Complete
        );
        // SizeMismatch: final present, wrong size.
        place_final(&s, &rel("fcamera.hevc"), 50);
        assert_eq!(
            classify_file(&s, &rel("fcamera.hevc"), 7600),
            DownloadState::SizeMismatch
        );
    }

    fn drive_with(files: &[(&str, u64)]) -> Drive {
        let segments = vec![Segment {
            name: SegmentName {
                route_id: ROUTE.to_string(),
                segment_num: 0,
            },
            files: files
                .iter()
                .map(|(name, size)| SegmentFile {
                    kind: FileKind::from_filename(name),
                    name: name.to_string(),
                    remote_size: *size,
                    mtime_s: 1000,
                })
                .collect(),
            recording: false,
        }];
        Drive {
            drive_key: format!("{ROUTE}--0"),
            route_id: ROUTE.to_string(),
            first_segment_num: 0,
            last_segment_num: 0,
            start_ms: Some(1_000_000),
            end_ms: Some(1_060_000),
            segment_count: 1,
            recording: false,
            sync_state: SyncStatus::NotDownloaded,
            preserved: false,
            segments,
        }
    }

    #[test]
    fn drive_status_aggregates() {
        let files = [("qcamera.ts", 1200u64), ("rlog.zst", 300u64)];
        let drive = drive_with(&files);
        let sel = FileSelection::everything();

        // Nothing on disk → NotDownloaded.
        let (_d, s) = store();
        assert_eq!(
            drive_status(&s, &drive, &sel, RR),
            SyncStatus::NotDownloaded
        );

        // Both present, correct → Complete.
        let (_d, s) = store();
        place_final(&s, &rel("qcamera.ts"), 1200);
        place_final(&s, &rel("rlog.zst"), 300);
        assert_eq!(drive_status(&s, &drive, &sel, RR), SyncStatus::Complete);

        // One present, one missing → Partial.
        let (_d, s) = store();
        place_final(&s, &rel("qcamera.ts"), 1200);
        assert_eq!(drive_status(&s, &drive, &sel, RR), SyncStatus::Partial);

        // One correct, one wrong-size → Partial.
        let (_d, s) = store();
        place_final(&s, &rel("qcamera.ts"), 1200);
        place_final(&s, &rel("rlog.zst"), 999);
        assert_eq!(drive_status(&s, &drive, &sel, RR), SyncStatus::Partial);

        // Empty selection → Complete (nothing outstanding), even with files missing.
        let (_d, s) = store();
        assert_eq!(
            drive_status(&s, &drive, &FileSelection::NONE, RR),
            SyncStatus::Complete
        );
    }
}
