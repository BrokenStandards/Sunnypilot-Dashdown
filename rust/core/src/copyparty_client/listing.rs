//! Parsing of copyparty's `?ls=j` JSON directory listing.
//!
//! copyparty emits `{ "dirs": [...], "files": [...], ... }`. Each entry has
//! `href` (percent-encoded), `sz` (bytes), `ts` (mtime, Unix seconds) — but the
//! `name` field is popped before JSON serialization (httpcli.py:6734), so the
//! filename is derived from `href`. Unknown keys are ignored.

use serde::Deserialize;

use crate::error::Result;

#[derive(Debug, Deserialize)]
struct RawListing {
    #[serde(default)]
    dirs: Vec<RawEntry>,
    #[serde(default)]
    files: Vec<RawEntry>,
}

#[derive(Debug, Deserialize)]
struct RawEntry {
    href: String,
    #[serde(default)]
    sz: u64,
    #[serde(default)]
    ts: i64,
}

/// A parsed listing entry with the filename decoded from `href`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub href: String,
    pub size: u64,
    pub mtime_s: i64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirListing {
    pub dirs: Vec<Entry>,
    pub files: Vec<Entry>,
}

pub fn parse_listing(json: &str) -> Result<DirListing> {
    let raw: RawListing = serde_json::from_str(json)?;
    Ok(DirListing {
        dirs: raw.dirs.into_iter().map(|e| to_entry(e, true)).collect(),
        files: raw.files.into_iter().map(|e| to_entry(e, false)).collect(),
    })
}

fn to_entry(e: RawEntry, is_dir: bool) -> Entry {
    Entry {
        name: decode_name(&e.href),
        href: e.href,
        size: e.sz,
        mtime_s: e.ts,
        is_dir,
    }
}

/// Derive the basename from a (possibly percent-encoded) href, stripping any
/// trailing `/` and parent path.
fn decode_name(href: &str) -> String {
    let trimmed = href.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or(trimmed);
    percent_encoding::percent_decode_str(last)
        .decode_utf8_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dirs_and_files_with_derived_names() {
        let json = r#"{
            "dirs":  [ {"href": "000001a3--c20ba54385--0/", "sz": 0, "ts": 1690462879} ],
            "files": [ {"href": "qcamera.ts", "sz": 12345, "ts": 1690462880},
                       {"href": "rlog.zst",   "sz": 1024,  "ts": 1690462881} ],
            "taglist": [], "acct": "*"
        }"#;
        let l = parse_listing(json).unwrap();
        assert_eq!(l.dirs.len(), 1);
        assert_eq!(l.dirs[0].name, "000001a3--c20ba54385--0");
        assert!(l.dirs[0].is_dir);
        assert_eq!(l.files.len(), 2);
        assert_eq!(l.files[0].name, "qcamera.ts");
        assert_eq!(l.files[0].size, 12345);
        assert_eq!(l.files[0].mtime_s, 1690462880);
        assert!(!l.files[0].is_dir);
    }

    #[test]
    fn decodes_percent_encoding() {
        let json = r#"{"dirs":[],"files":[{"href":"a%20b.ts","sz":1,"ts":2}]}"#;
        let l = parse_listing(json).unwrap();
        assert_eq!(l.files[0].name, "a b.ts");
    }
}
