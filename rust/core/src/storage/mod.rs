//! The local mirror store: crash-safe `.part` → fsync → rename writes that match
//! copyparty's layout. The M4 download engine streams response bodies into a
//! [`PartFile`] and commits it atomically; [`scan`] reconstructs `Vec<Segment>`
//! from the committed files for offline browsing/grouping.

pub mod paths;
pub mod scan;

use std::path::{Path, PathBuf};

use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::error::{CoreError, Result};

/// A mirror rooted at one directory that mirrors a copyparty base. All methods
/// take copyparty-relative paths (the same `rel` the client downloads). The
/// per-device subdirectory under the app's mirror root is chosen by M8/AppCore.
pub struct MirrorStore {
    root: PathBuf,
}

impl MirrorStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Absolute path of the committed file for `rel` (rejecting `..`).
    pub fn final_path(&self, rel: &str) -> Result<PathBuf> {
        paths::safe_join(&self.root, rel)
            .ok_or_else(|| CoreError::Io(format!("path traversal rejected: {rel}")))
    }

    /// Absolute path of the in-progress `.part` file for `rel`.
    pub fn part_path(&self, rel: &str) -> Result<PathBuf> {
        let mut os = self.final_path(rel)?.into_os_string();
        os.push(".part");
        Ok(PathBuf::from(os))
    }

    /// Whether the committed file exists. A stray `.part` is ignored; size /
    /// mismatch reconciliation is M5.
    pub fn is_complete(&self, rel: &str) -> bool {
        self.final_path(rel).map(|p| p.is_file()).unwrap_or(false)
    }

    /// Size of the committed file, if present.
    pub fn local_size(&self, rel: &str) -> Option<u64> {
        let p = self.final_path(rel).ok()?;
        std::fs::metadata(p).ok().map(|m| m.len())
    }

    /// Size of an in-progress `.part`, if present (the byte-range resume offset).
    pub fn part_size(&self, rel: &str) -> Option<u64> {
        let p = self.part_path(rel).ok()?;
        std::fs::metadata(p).ok().map(|m| m.len())
    }

    /// Open a fresh `.part` for `rel`, creating parent dirs. Truncates any stale
    /// `.part` — used for a download starting from byte 0 (or a Range-ignoring
    /// server that returns the whole body).
    pub async fn create_part(&self, rel: &str) -> Result<PartFile> {
        self.open_part(rel, true).await
    }

    /// Open `rel`'s `.part` in **append** mode (existing bytes preserved, writes
    /// go to the end) — used to resume a partial download from its offset.
    pub async fn open_part_append(&self, rel: &str) -> Result<PartFile> {
        self.open_part(rel, false).await
    }

    async fn open_part(&self, rel: &str, truncate: bool) -> Result<PartFile> {
        let final_ = self.final_path(rel)?;
        let part = self.part_path(rel)?;
        if let Some(parent) = final_.parent() {
            fs::create_dir_all(parent).await?;
        }
        let mut opts = fs::OpenOptions::new();
        opts.create(true);
        if truncate {
            opts.write(true).truncate(true);
        } else {
            // Append: writes go to the end; existing partial bytes are kept.
            opts.append(true);
        }
        let file = opts.open(&part).await?;
        Ok(PartFile { file, part, final_ })
    }

    /// Remove the committed file and any stray `.part` for `rel`. Idempotent —
    /// a missing file is success (mirrors [`PartFile::abort`]). Used by retention
    /// to reclaim local space file-by-file.
    pub async fn remove_file(&self, rel: &str) -> Result<()> {
        remove_if_exists(&self.final_path(rel)?).await?;
        remove_if_exists(&self.part_path(rel)?).await
    }

    /// Recursively remove a whole segment directory by its mirror-relative dir
    /// path (e.g. `realdata/<seg>/`). Idempotent; rejects `..` traversal. Used by
    /// retention to drop an entire pruned drive's footage at once.
    pub async fn remove_dir(&self, rel_dir: &str) -> Result<()> {
        let dir = paths::safe_join(&self.root, rel_dir)
            .ok_or_else(|| CoreError::Io(format!("path traversal rejected: {rel_dir}")))?;
        match fs::remove_dir_all(&dir).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Convenience: write `bytes` to `rel` and commit atomically (tests and the
    /// M4 non-streaming path). Shares the durability path with streamed writes.
    pub async fn write_all(&self, rel: &str, bytes: &[u8]) -> Result<()> {
        let mut pf = self.create_part(rel).await?;
        pf.writer().write_all(bytes).await?;
        pf.commit().await
    }
}

/// An in-progress download target: a `.part` file that becomes the committed
/// file on [`PartFile::commit`]. Dropping without committing leaves the `.part`
/// behind (benign — scans ignore `.part` and M5 re-fetches).
pub struct PartFile {
    file: fs::File,
    part: PathBuf,
    final_: PathBuf,
}

impl PartFile {
    /// The async sink to stream the body into:
    /// `client.download_to(rel, pf.writer()).await?`.
    pub fn writer(&mut self) -> &mut fs::File {
        &mut self.file
    }

    /// Flush + fsync the data, then atomically rename `.part` → final, then
    /// best-effort fsync the parent dir so the rename itself is durable.
    pub async fn commit(self) -> Result<()> {
        let PartFile {
            mut file,
            part,
            final_,
        } = self;
        file.flush().await?;
        file.sync_all().await?; // fsync data + metadata before linking into place
        drop(file);
        fs::rename(&part, &final_).await?;
        // Best-effort directory fsync: not portable on all filesystems, and the
        // data fsync above is the load-bearing durability guarantee.
        if let Some(parent) = final_.parent() {
            if let Ok(dir) = fs::File::open(parent).await {
                if let Err(e) = dir.sync_all().await {
                    tracing::debug!(error = %e, "parent dir fsync failed (non-fatal)");
                }
            }
        }
        Ok(())
    }

    /// Discard the in-progress `.part` (ignoring a missing file).
    pub async fn abort(self) -> Result<()> {
        let PartFile { file, part, .. } = self;
        drop(file);
        remove_if_exists(&part).await
    }
}

/// Remove a file, treating "not found" as success (idempotent delete).
async fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, MirrorStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = MirrorStore::new(dir.path());
        (dir, store)
    }

    const REL: &str = "realdata/000001a3--c20ba54385--0/qcamera.ts";

    #[tokio::test]
    async fn atomic_write_commits() {
        let (_d, store) = store();
        let mut pf = store.create_part(REL).await.unwrap();
        pf.writer().write_all(b"hello").await.unwrap();
        // Pre-commit: the `.part` exists, the final file does not.
        assert!(store.part_path(REL).unwrap().is_file());
        assert!(!store.is_complete(REL));

        pf.commit().await.unwrap();
        // Post-commit: final present with exact bytes, `.part` gone.
        assert!(store.is_complete(REL));
        assert!(!store.part_path(REL).unwrap().exists());
        assert_eq!(store.local_size(REL), Some(5));
        let bytes = tokio::fs::read(store.final_path(REL).unwrap())
            .await
            .unwrap();
        assert_eq!(bytes, b"hello");
    }

    #[tokio::test]
    async fn create_part_makes_segment_dirs() {
        let (_d, store) = store();
        let pf = store.create_part(REL).await.unwrap();
        assert!(store.part_path(REL).unwrap().parent().unwrap().is_dir());
        pf.abort().await.unwrap();
    }

    #[tokio::test]
    async fn drop_without_commit_leaves_part() {
        let (_d, store) = store();
        {
            let mut pf = store.create_part(REL).await.unwrap();
            pf.writer().write_all(b"partial").await.unwrap();
        } // dropped without commit
        assert!(store.part_path(REL).unwrap().is_file());
        assert!(!store.is_complete(REL));
    }

    #[tokio::test]
    async fn abort_removes_part() {
        let (_d, store) = store();
        let mut pf = store.create_part(REL).await.unwrap();
        pf.writer().write_all(b"x").await.unwrap();
        pf.abort().await.unwrap();
        assert!(!store.part_path(REL).unwrap().exists());
        assert!(!store.is_complete(REL));
    }

    #[tokio::test]
    async fn write_all_commits() {
        let (_d, store) = store();
        store.write_all(REL, b"data").await.unwrap();
        assert!(store.is_complete(REL));
        assert_eq!(store.local_size(REL), Some(4));
    }

    #[tokio::test]
    async fn rejects_traversal() {
        let (_d, store) = store();
        assert!(store.create_part("../escape").await.is_err());
        assert!(store.final_path("../escape").is_err());
    }

    #[tokio::test]
    async fn remove_file_deletes_final_and_part_idempotently() {
        let (_d, store) = store();
        // Missing file → Ok (idempotent).
        store.remove_file(REL).await.unwrap();

        // Place both a committed file and a stray `.part`; remove clears both.
        store.write_all(REL, b"data").await.unwrap();
        let mut pf = store.create_part(REL).await.unwrap();
        pf.writer().write_all(b"x").await.unwrap();
        drop(pf); // leaves a `.part` behind
        assert!(store.is_complete(REL));
        assert!(store.part_path(REL).unwrap().is_file());

        store.remove_file(REL).await.unwrap();
        assert!(!store.is_complete(REL));
        assert!(!store.part_path(REL).unwrap().exists());
    }

    #[tokio::test]
    async fn remove_dir_is_recursive_and_idempotent() {
        let (_d, store) = store();
        const SEG_DIR: &str = "realdata/000001a3--c20ba54385--0/";
        // Missing dir → Ok.
        store.remove_dir(SEG_DIR).await.unwrap();

        store.write_all(REL, b"data").await.unwrap();
        store
            .write_all("realdata/000001a3--c20ba54385--0/rlog.zst", b"log")
            .await
            .unwrap();
        let seg_path = store
            .final_path(REL)
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        assert!(seg_path.is_dir());

        store.remove_dir(SEG_DIR).await.unwrap();
        assert!(!seg_path.exists());
        assert!(!store.is_complete(REL));
    }

    #[tokio::test]
    async fn removal_rejects_traversal() {
        let (_d, store) = store();
        assert!(store.remove_file("../escape").await.is_err());
        assert!(store.remove_dir("../escape/").await.is_err());
    }
}
