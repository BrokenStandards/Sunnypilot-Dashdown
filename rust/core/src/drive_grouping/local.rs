//! Offline drive grouping: scan the local mirror's `realdata` directory and group
//! with the shared [`group_segments`](super::group_segments) — the offline twin
//! of [`remote::group_remote`](super::remote::group_remote). Sync (filesystem);
//! callers wrap in `spawn_blocking`.

use std::path::Path;

use crate::error::Result;
use crate::model::Drive;
use crate::storage::scan::scan_segments;

/// Scan `realdata_dir` (`<mirror>/realdata`) and group it into drives.
pub fn group_local(realdata_dir: &Path) -> Result<Vec<Drive>> {
    Ok(super::group_segments(scan_segments(realdata_dir)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_dir_groups_to_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let rd = dir.path().join("realdata");
        std::fs::create_dir_all(&rd).unwrap();
        assert!(group_local(&rd).unwrap().is_empty());
    }

    #[test]
    fn missing_dir_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(group_local(&dir.path().join("nope")).is_err());
    }
}
