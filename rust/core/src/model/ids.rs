//! Parsing of on-disk segment directory names.

use crate::error::{CoreError, Result};

/// A copyparty/on-disk segment directory name, decomposed into the route id and
/// the 0-indexed segment number.
///
/// sunnypilot names segment dirs `{route}--{N}` where `route = {8hex}--{10hex}`
/// (e.g. `000001a3--c20ba54385--0`); the route id carries no timestamp. This
/// parser keys only on the trailing numeric segment index, so it also tolerates
/// the legacy comma-cloud form `dongleid|YYYY-MM-DD--HH-MM-SS--N`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct SegmentName {
    pub route_id: String,
    pub segment_num: u32,
}

impl SegmentName {
    pub fn parse(dir_name: &str) -> Result<Self> {
        let (route_id, num) = dir_name
            .rsplit_once("--")
            .ok_or_else(|| CoreError::Parse(format!("not a segment dir: {dir_name}")))?;
        let segment_num: u32 = num
            .parse()
            .map_err(|_| CoreError::Parse(format!("segment index not numeric: {dir_name}")))?;
        if route_id.is_empty() {
            return Err(CoreError::Parse(format!("empty route id: {dir_name}")));
        }
        Ok(SegmentName {
            route_id: route_id.to_string(),
            segment_num,
        })
    }

    pub fn dir_name(&self) -> String {
        format!("{}--{}", self.route_id, self.segment_num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_on_disk_name() {
        let s = SegmentName::parse("000001a3--c20ba54385--0").unwrap();
        assert_eq!(s.route_id, "000001a3--c20ba54385");
        assert_eq!(s.segment_num, 0);
        assert_eq!(s.dir_name(), "000001a3--c20ba54385--0");
    }

    #[test]
    fn parses_higher_index() {
        let s = SegmentName::parse("000001a3--c20ba54385--42").unwrap();
        assert_eq!(s.segment_num, 42);
    }

    #[test]
    fn parses_legacy_cloud_name() {
        let s = SegmentName::parse("a2a0ccea32023010|2023-07-27--13-01-19--3").unwrap();
        assert_eq!(s.route_id, "a2a0ccea32023010|2023-07-27--13-01-19");
        assert_eq!(s.segment_num, 3);
    }

    #[test]
    fn rejects_route_dir_without_index() {
        // A bare route id (no numeric suffix) is not a segment.
        assert!(SegmentName::parse("000001a3--c20ba54385").is_err());
    }

    #[test]
    fn rejects_no_separator() {
        assert!(SegmentName::parse("realdata").is_err());
    }

    #[test]
    fn rejects_non_numeric_index() {
        assert!(SegmentName::parse("route--abc").is_err());
    }
}
