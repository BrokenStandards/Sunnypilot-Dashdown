//! Temp-dir fixture trees mimicking a comma device's footage, served under the
//! copyparty URL alias `routes/` (sunnypilot maps the on-disk `realdata/` dir there).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use tempfile::TempDir;

/// A fixture directory tree (kept alive while in scope).
pub struct Fixture {
    pub dir: TempDir,
    /// Relative-path → advertised `sz` overriding the real on-disk size in the
    /// `?ls=j` listing. Empty for honest fixtures; set by [`size_mismatch`].
    pub size_overrides: HashMap<String, u64>,
}

impl Fixture {
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// An honest fixture: listings report true on-disk sizes.
    fn plain(dir: TempDir) -> Self {
        Self {
            dir,
            size_overrides: HashMap::new(),
        }
    }
}

/// Write the full file set for one segment under `routes/<seg>/`.
fn full_segment(root: &Path, seg: &str) {
    let base = root.join("routes").join(seg);
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
    Fixture::plain(dir)
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
    Fixture::plain(dir)
}

/// One route with a **segment-index gap** (0, 1, 3 — missing 2): drive grouping
/// splits at the index break into two drives. Mirrors a real >1-min recording
/// gap within a single loggerd session more closely than [`gap_split`]'s
/// distinct-routes case.
pub fn gap_index() -> Fixture {
    let dir = TempDir::new().unwrap();
    let route = "000001a7--5566778899";
    for n in [0u32, 1, 3] {
        full_segment(dir.path(), &format!("{route}--{n}"));
    }
    Fixture::plain(dir)
}

/// One drive whose last segment is still recording (`rlog.lock` present) and
/// has only a partial file set.
pub fn partial() -> Fixture {
    let dir = TempDir::new().unwrap();
    let route = "000001a5--1122334455";
    full_segment(dir.path(), &format!("{route}--0"));

    let base = dir.path().join("routes").join(format!("{route}--1"));
    fs::create_dir_all(&base).unwrap();
    fs::write(base.join("qcamera.ts"), vec![0u8; 600]).unwrap();
    fs::write(base.join("rlog.lock"), b"").unwrap(); // recording marker
    Fixture::plain(dir)
}

/// One drive whose `qcamera.ts` is served honestly (600 real bytes) but the
/// listing **advertises a larger `sz` (1200)**. A client that downloads it
/// commits the real 600 bytes, which then mismatch the recorded remote size →
/// `DownloadState::SizeMismatch` (drive `Partial`/resumable). Exercises the
/// re-fetch path without needing a truncating proxy.
pub fn size_mismatch() -> Fixture {
    let dir = TempDir::new().unwrap();
    let route = "000001a6--deadbeef00";
    let seg = format!("{route}--0");
    let base = dir.path().join("routes").join(&seg);
    fs::create_dir_all(&base).unwrap();
    fs::write(base.join("qcamera.ts"), vec![0u8; 600]).unwrap();

    let mut size_overrides = HashMap::new();
    size_overrides.insert(format!("routes/{seg}/qcamera.ts"), 1200u64);
    Fixture {
        dir,
        size_overrides,
    }
}
