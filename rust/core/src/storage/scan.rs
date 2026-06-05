//! Local twin of [`CopypartyClient::list_segments`](crate::copyparty_client::CopypartyClient::list_segments):
//! reconstruct `Vec<Segment>` from the mirror's `realdata` directory.
//!
//! `SegmentFile::remote_size` carries the **on-disk** length here (for a complete
//! mirror local == remote; the real local/remote split is the M5 DB `local_size`
//! column). Skips `*.part` (incomplete downloads) and `rlog.lock` (sets
//! `recording`). Sync — callers wrap in `spawn_blocking`, like the DB layer.

use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::error::Result;
use crate::model::{FileKind, Segment, SegmentFile, SegmentName};

/// Scan `realdata_dir` (`<mirror>/realdata`) into the same `Vec<Segment>` shape
/// `list_segments` produces from the server, so `group_segments` is shared.
pub fn scan_segments(realdata_dir: &Path) -> Result<Vec<Segment>> {
    let mut segments = Vec::new();
    for entry in fs::read_dir(realdata_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().into_owned();
        let Ok(name) = SegmentName::parse(&dir_name) else {
            continue; // non-segment directory
        };

        let mut files = Vec::new();
        let mut recording = false;
        for f in fs::read_dir(entry.path())? {
            let f = f?;
            if !f.file_type()?.is_file() {
                continue;
            }
            let file_name = f.file_name().to_string_lossy().into_owned();
            if file_name.ends_with(".part") {
                continue; // incomplete download — not a committed file
            }
            let kind = FileKind::from_filename(&file_name);
            if kind == FileKind::LockMarker {
                recording = true;
                continue;
            }
            let meta = f.metadata()?;
            let mtime_s = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            files.push(SegmentFile {
                kind,
                name: file_name,
                remote_size: meta.len(),
                mtime_s,
            });
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));
        segments.push(Segment {
            name,
            files,
            recording,
        });
    }
    segments.sort_by(|a, b| {
        a.name
            .route_id
            .cmp(&b.name.route_id)
            .then(a.name.segment_num.cmp(&b.name.segment_num))
    });
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: std::path::PathBuf, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn scans_segments_skipping_part_and_lock() {
        let dir = tempfile::tempdir().unwrap();
        let rd = dir.path().join("realdata");
        let route = "000001a3--c20ba54385";

        // seg 0: two committed files + a `.part` (must be excluded).
        write(rd.join(format!("{route}--0")).join("qcamera.ts"), b"aaa");
        write(rd.join(format!("{route}--0")).join("rlog.zst"), b"bb");
        write(
            rd.join(format!("{route}--0")).join("fcamera.hevc.part"),
            b"x",
        );
        // seg 1: a file + a lock marker (→ recording, not listed).
        write(rd.join(format!("{route}--1")).join("qcamera.ts"), b"cccc");
        write(rd.join(format!("{route}--1")).join("rlog.lock"), b"");
        // a non-segment directory (ignored).
        fs::create_dir_all(rd.join("notaseg")).unwrap();

        let segs = scan_segments(&rd).unwrap();
        assert_eq!(segs.len(), 2);

        let s0 = &segs[0];
        assert_eq!(s0.name.segment_num, 0);
        assert!(!s0.recording);
        let names: Vec<_> = s0.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["qcamera.ts", "rlog.zst"], ".part excluded, sorted");
        let q = s0.files.iter().find(|f| f.name == "qcamera.ts").unwrap();
        assert_eq!(q.remote_size, 3);
        assert!(q.mtime_s > 0);

        let s1 = &segs[1];
        assert_eq!(s1.name.segment_num, 1);
        assert!(s1.recording, "rlog.lock → recording");
        assert_eq!(s1.files.len(), 1);
        assert!(s1.files.iter().all(|f| f.kind != FileKind::LockMarker));
    }

    #[test]
    fn missing_dir_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(scan_segments(&dir.path().join("nope")).is_err());
    }
}
