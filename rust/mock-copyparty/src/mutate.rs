//! Runtime mutation of a served fixture tree — add/remove drives and append
//! segments to a route while the [`crate::MockServer`] keeps serving the same
//! root **live** (no restart; the listing handler walks disk per request).
//!
//! Segment directories are named `routes/<route>--<n>`; the route stem is the
//! dir name with the trailing `--<n>` stripped. `rsplit_once("--")` is used so
//! route ids that themselves contain `--` (e.g. `000001a3--c20ba54385`) parse
//! correctly.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::fixtures::full_segment;

const ROUTES: &str = "routes";

/// One route's identity and its current segment count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RouteInfo {
    pub route: String,
    pub segments: usize,
}

/// Split a segment dir name `"<route>--<n>"` into `(route, n)`, or `None` if it
/// doesn't end in a numeric `--<n>` suffix.
fn split_seg(name: &str) -> Option<(&str, u32)> {
    let (route, num) = name.rsplit_once("--")?;
    let n: u32 = num.parse().ok()?;
    Some((route, n))
}

/// Every segment dir under `root/routes/` as `(route, index)` pairs (missing
/// `routes/` ⇒ empty, not an error).
fn segment_dirs(root: &Path) -> io::Result<Vec<(String, u32)>> {
    let dir = root.join(ROUTES);
    let rd = match fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some((route, n)) = split_seg(&name) {
            out.push((route.to_string(), n));
        }
    }
    Ok(out)
}

/// The route stem of the lexically-first segment dir (`None` if there are none).
pub fn primary_route(root: &Path) -> Option<String> {
    let mut routes: Vec<String> = segment_dirs(root)
        .unwrap_or_default()
        .into_iter()
        .map(|(r, _)| r)
        .collect();
    routes.sort();
    routes.into_iter().next()
}

/// Each route under `root/routes/` with its current segment count, by route.
pub fn list_routes(root: &Path) -> Vec<RouteInfo> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (route, _) in segment_dirs(root).unwrap_or_default() {
        *counts.entry(route).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(route, segments)| RouteInfo { route, segments })
        .collect()
}

/// Append `n` consecutive segments to `route` (default: [`primary_route`]),
/// starting at `max_existing_index + 1` (or 0 if the route has none yet).
pub fn add_segment(root: &Path, route: Option<&str>, n: usize) -> io::Result<()> {
    let route = match route {
        Some(r) => r.to_string(),
        None => primary_route(root)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no route to grow"))?,
    };
    let next = segment_dirs(root)?
        .into_iter()
        .filter(|(r, _)| *r == route)
        .map(|(_, i)| i)
        .max()
        .map(|m| m + 1)
        .unwrap_or(0);
    for i in 0..n as u32 {
        full_segment(root, &format!("{route}--{}", next + i));
    }
    Ok(())
}

/// Create a brand-new drive: `route--0 .. route--(segs-1)`, each a full segment.
/// When `mtime_s` is set, every written file's modified time is forced to it so the
/// derived segment/drive age is deterministic (segment age = newest file mtime) —
/// lets tests stage drives in a known oldest→newest order for retention.
pub fn add_drive(root: &Path, route: &str, segs: usize, mtime_s: Option<i64>) -> io::Result<()> {
    for i in 0..segs as u32 {
        let seg = format!("{route}--{i}");
        full_segment(root, &seg);
        if let Some(m) = mtime_s {
            set_seg_mtime(root, &seg, m)?;
        }
    }
    Ok(())
}

/// Force every file in `routes/<seg>/` to modified-time `mtime_s` (epoch seconds).
fn set_seg_mtime(root: &Path, seg: &str, mtime_s: i64) -> io::Result<()> {
    let when = SystemTime::UNIX_EPOCH + Duration::from_secs(mtime_s.max(0) as u64);
    let times = fs::FileTimes::new().set_modified(when);
    for entry in fs::read_dir(root.join(ROUTES).join(seg))? {
        let path = entry?.path();
        if path.is_file() {
            fs::File::options().write(true).open(&path)?.set_times(times)?;
        }
    }
    Ok(())
}

/// Delete every `routes/<route>--*` segment dir (models the Comma's own
/// low-space auto-prune of old drives).
pub fn remove_drive(root: &Path, route: &str) -> io::Result<()> {
    let dir = root.join(ROUTES);
    for (r, n) in segment_dirs(root)? {
        if r == route {
            fs::remove_dir_all(dir.join(format!("{r}--{n}")))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn seg_dir(root: &Path, name: &str) -> std::path::PathBuf {
        root.join(ROUTES).join(name)
    }

    #[test]
    fn add_drive_then_segment_grows_in_place() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let route = "000001a3--c20ba54385"; // route id with an internal "--"

        add_drive(root, route, 2, None).unwrap();
        assert!(seg_dir(root, &format!("{route}--0")).is_dir());
        assert!(seg_dir(root, &format!("{route}--1")).is_dir());
        assert!(seg_dir(root, &format!("{route}--0"))
            .join("qcamera.ts")
            .is_file());

        // Defaults to the primary route and appends at max+1.
        add_segment(root, None, 1).unwrap();
        assert!(seg_dir(root, &format!("{route}--2")).is_dir());

        add_segment(root, Some(route), 2).unwrap();
        assert!(seg_dir(root, &format!("{route}--3")).is_dir());
        assert!(seg_dir(root, &format!("{route}--4")).is_dir());

        assert_eq!(
            list_routes(root),
            vec![RouteInfo {
                route: route.to_string(),
                segments: 5
            }]
        );
        assert_eq!(primary_route(root).as_deref(), Some(route));
    }

    #[test]
    fn add_segment_to_empty_starts_at_zero() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        add_segment(root, Some("abc--def"), 1).unwrap();
        assert!(seg_dir(root, "abc--def--0").is_dir());
    }

    #[test]
    fn add_segment_with_no_route_and_empty_tree_errs() {
        let tmp = TempDir::new().unwrap();
        assert!(add_segment(tmp.path(), None, 1).is_err());
    }

    #[test]
    fn remove_drive_only_touches_its_own_segments() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        add_drive(root, "keep--01", 2, None).unwrap();
        add_drive(root, "drop--02", 3, None).unwrap();

        remove_drive(root, "drop--02").unwrap();
        assert!(!seg_dir(root, "drop--02--0").exists());
        assert!(seg_dir(root, "keep--01--0").is_dir());
        assert_eq!(
            list_routes(root),
            vec![RouteInfo {
                route: "keep--01".to_string(),
                segments: 2
            }]
        );
    }

    #[test]
    fn primary_route_is_lexically_first() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        add_drive(root, "bbb--22", 1, None).unwrap();
        add_drive(root, "aaa--11", 1, None).unwrap();
        assert_eq!(primary_route(root).as_deref(), Some("aaa--11"));
    }

    #[test]
    fn add_drive_with_mtime_forces_file_times() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        add_drive(root, "ts--01", 1, Some(1_000_000)).unwrap();
        let f = seg_dir(root, "ts--01--0").join("qcamera.ts");
        let secs = std::fs::metadata(&f)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(secs, 1_000_000);
    }
}
