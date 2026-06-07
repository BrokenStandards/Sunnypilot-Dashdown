//! Make the comma HD camera streams playable. The road/wide/driver cameras are
//! raw HEVC (`*.hevc`, Annex-B, no container) which ExoPlayer/AVPlayer cannot
//! play; [`remux`] losslessly wraps them in an MP4 (`hvc1`) with exact sample
//! tables. The result is a **derived artifact** cached next to its source
//! (`fcamera.hevc` → `fcamera.hevc.mp4`): re-derivable any time, and removed for
//! free when retention prunes the whole segment directory.
//!
//! This logic lives in the core (not the native layer) so iOS reuses it verbatim
//! — both platforms get back a plain MP4 path their system player can open.

pub mod remux;

use std::path::{Path, PathBuf};

use crate::error::Result;

/// Path of the derived MP4 for a raw `*.hevc` source (`<src>.mp4`).
pub fn playable_mp4_path(src: &Path) -> PathBuf {
    let mut p = src.as_os_str().to_owned();
    p.push(".mp4");
    PathBuf::from(p)
}

/// Ensure a playable MP4 exists for the raw HEVC `src`, returning nothing on
/// success (the path is [`playable_mp4_path`]). Reuses an already-cached MP4;
/// otherwise remuxes `src` and writes the output atomically (`.part` → rename) so
/// a crash mid-write never leaves a truncated, half-playable file behind.
///
/// Synchronous and CPU/IO-bound (reads the whole `.hevc`, ~tens of MB) — callers
/// must run it on a blocking pool, never on an async executor thread.
pub fn ensure_playable_mp4(src: &Path) -> Result<PathBuf> {
    let dst = playable_mp4_path(src);
    if dst.is_file() {
        return Ok(dst);
    }
    let input = std::fs::read(src)?;
    let mp4 = remux::hevc_annexb_to_mp4(&input)?;

    let mut tmp = dst.as_os_str().to_owned();
    tmp.push(".part"); // distinct from the HEVC download's own "<name>.part"
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, &mp4)?;
    std::fs::rename(&tmp, &dst)?;
    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_path_appends_mp4() {
        let p = Path::new("/m/0/routes/aa--0/fcamera.hevc");
        assert_eq!(
            playable_mp4_path(p),
            PathBuf::from("/m/0/routes/aa--0/fcamera.hevc.mp4")
        );
    }

    #[test]
    fn ensure_creates_then_reuses_cached_mp4() {
        // A minimal valid stream (SPS + one IDR) round-trips to a cached MP4.
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("fcamera.hevc");
        std::fs::write(&src, remux::minimal_stream()).unwrap();

        let out1 = ensure_playable_mp4(&src).unwrap();
        assert!(out1.is_file());
        assert_eq!(out1, playable_mp4_path(&src));
        let bytes1 = std::fs::read(&out1).unwrap();

        // Second call reuses the cached file (no .part left behind).
        let out2 = ensure_playable_mp4(&src).unwrap();
        assert_eq!(out1, out2);
        assert_eq!(std::fs::read(&out2).unwrap(), bytes1, "cached, unchanged");
        let mut part = src.as_os_str().to_owned();
        part.push(".mp4.part");
        assert!(!PathBuf::from(part).exists(), "no leftover .part");
    }
}
