//! Temp-dir fixture trees mimicking a comma device's `realdata/` directory.

use std::fs;
use std::path::Path;

use tempfile::TempDir;

/// A fixture directory tree (kept alive while in scope).
pub struct Fixture {
    pub dir: TempDir,
}

impl Fixture {
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// Write the full file set for one segment under `realdata/<seg>/`.
fn full_segment(root: &Path, seg: &str) {
    let base = root.join("realdata").join(seg);
    fs::create_dir_all(&base).unwrap();
    fs::write(base.join("qcamera.ts"), vec![0u8; 1200]).unwrap();
    fs::write(base.join("rlog.zst"), vec![1u8; 300]).unwrap();
    fs::write(base.join("qlog.zst"), vec![2u8; 100]).unwrap();
    fs::write(base.join("fcamera.hevc"), vec![3u8; 7600]).unwrap();
    fs::write(base.join("ecamera.hevc"), vec![4u8; 7600]).unwrap();
}

/// One route with 3 consecutive segments, each with the full file set.
pub fn single_drive() -> Fixture {
    let dir = TempDir::new().unwrap();
    let route = "000001a3--c20ba54385";
    for n in 0..3 {
        full_segment(dir.path(), &format!("{route}--{n}"));
    }
    Fixture { dir }
}

/// Two distinct routes (a new route ⇒ a new drive in M2).
pub fn gap_split() -> Fixture {
    let dir = TempDir::new().unwrap();
    for n in 0..2 {
        full_segment(dir.path(), &format!("000001a3--c20ba54385--{n}"));
    }
    for n in 0..2 {
        full_segment(dir.path(), &format!("000001a4--aabbccddee--{n}"));
    }
    Fixture { dir }
}

/// One drive whose last segment is still recording (`rlog.lock` present) and
/// has only a partial file set.
pub fn partial() -> Fixture {
    let dir = TempDir::new().unwrap();
    let route = "000001a5--1122334455";
    full_segment(dir.path(), &format!("{route}--0"));

    let base = dir.path().join("realdata").join(format!("{route}--1"));
    fs::create_dir_all(&base).unwrap();
    fs::write(base.join("qcamera.ts"), vec![0u8; 600]).unwrap();
    fs::write(base.join("rlog.lock"), b"").unwrap(); // recording marker
    Fixture { dir }
}
