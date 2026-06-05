//! Mapping between copyparty-relative paths and the local mirror tree. The
//! mirror stores each file at `root.join(rel)` where `rel` is the same
//! copyparty-relative path the client uses for downloads, so the on-disk layout
//! matches the server exactly.

use std::path::{Path, PathBuf};

use crate::model::SegmentName;

/// copyparty-relative path of one segment file, e.g.
/// `realdata/000001a3--c20ba54385--0/qcamera.ts`.
pub fn file_rel(realdata_rel: &str, seg: &SegmentName, file_name: &str) -> String {
    let prefix = realdata_rel.trim_matches('/');
    if prefix.is_empty() {
        format!("{}/{}", seg.dir_name(), file_name)
    } else {
        format!("{}/{}/{}", prefix, seg.dir_name(), file_name)
    }
}

/// Inverse of [`file_rel`]: recover `(segment, file_name)` from a rel path under
/// `realdata_rel`. Returns `None` on prefix mismatch, `..`/`.` traversal, a
/// non-segment directory, or a nested file path.
pub fn parse_file_rel(realdata_rel: &str, rel: &str) -> Option<(SegmentName, String)> {
    let prefix = realdata_rel.trim_matches('/');
    let rel_t = rel.trim_start_matches('/');

    let rest = if prefix.is_empty() {
        rel_t
    } else {
        let after = rel_t.strip_prefix(prefix)?;
        // The prefix must end on a path boundary ("realdataX/.." must not match).
        if !after.is_empty() && !after.starts_with('/') {
            return None;
        }
        after.trim_start_matches('/')
    };

    if rest.split('/').any(|c| c == ".." || c == ".") {
        return None;
    }
    let (seg_dir, file_name) = rest.split_once('/')?;
    if file_name.is_empty() || file_name.contains('/') {
        return None;
    }
    let seg = SegmentName::parse(seg_dir).ok()?;
    Some((seg, file_name.to_string()))
}

/// Join `rel` onto `root`, rejecting any `..` traversal (mirror of
/// mock-copyparty's `safe_join`).
pub(crate) fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut p = root.to_path_buf();
    for comp in rel.split('/') {
        match comp {
            "" | "." => continue,
            ".." => return None,
            c => p.push(c),
        }
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(route: &str, num: u32) -> SegmentName {
        SegmentName {
            route_id: route.to_string(),
            segment_num: num,
        }
    }

    #[test]
    fn rel_round_trips_on_disk_name() {
        let s = seg("000001a3--c20ba54385", 0);
        let rel = file_rel("realdata/", &s, "qcamera.ts");
        assert_eq!(rel, "realdata/000001a3--c20ba54385--0/qcamera.ts");
        assert_eq!(
            parse_file_rel("realdata/", &rel),
            Some((s, "qcamera.ts".to_string()))
        );
    }

    #[test]
    fn rel_round_trips_legacy_cloud_name() {
        let s = seg("a2a0ccea32023010|2023-07-27--13-01-19", 3);
        let rel = file_rel("realdata", &s, "rlog.zst");
        assert_eq!(
            parse_file_rel("realdata", &rel),
            Some((s, "rlog.zst".to_string()))
        );
    }

    #[test]
    fn empty_prefix_round_trips() {
        let s = seg("000001a3--c20ba54385", 5);
        let rel = file_rel("", &s, "fcamera.hevc");
        assert_eq!(rel, "000001a3--c20ba54385--5/fcamera.hevc");
        assert_eq!(
            parse_file_rel("", &rel),
            Some((s, "fcamera.hevc".to_string()))
        );
    }

    #[test]
    fn rejects_prefix_mismatch() {
        assert_eq!(parse_file_rel("realdata", "other/seg--0/qcamera.ts"), None);
        // Substring-but-not-boundary must not match.
        assert_eq!(
            parse_file_rel("realdata", "realdataX/000001a3--c20ba54385--0/qcamera.ts"),
            None
        );
    }

    #[test]
    fn rejects_traversal_and_nesting() {
        assert_eq!(parse_file_rel("realdata", "realdata/../etc/passwd"), None);
        // Non-segment directory.
        assert_eq!(
            parse_file_rel("realdata", "realdata/notaseg/qcamera.ts"),
            None
        );
        // Nested file path under a segment.
        assert_eq!(
            parse_file_rel("realdata", "realdata/000001a3--c20ba54385--0/sub/x.ts"),
            None
        );
    }

    #[test]
    fn safe_join_rejects_dotdot_accepts_nested() {
        let root = Path::new("/mirror");
        assert_eq!(
            safe_join(root, "realdata/seg--0/qcamera.ts"),
            Some(PathBuf::from("/mirror/realdata/seg--0/qcamera.ts"))
        );
        assert_eq!(safe_join(root, "realdata/../escape"), None);
    }
}
