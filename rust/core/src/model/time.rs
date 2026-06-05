//! Time helpers. On-disk segment names carry no timestamp, so wall-clock time
//! comes from copyparty's `ts` mtime (Unix seconds). M2 grouping consumes these.

/// Each sunnypilot segment is exactly 60 seconds.
pub const SEGMENT_MS: i64 = 60_000;

/// Convert copyparty `ts` (Unix seconds) to epoch milliseconds.
pub fn secs_to_ms(secs: i64) -> i64 {
    secs.saturating_mul(1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_length_is_one_minute() {
        assert_eq!(SEGMENT_MS, 60_000);
    }

    #[test]
    fn converts_seconds_to_millis() {
        assert_eq!(secs_to_ms(1_690_462_879), 1_690_462_879_000);
        assert_eq!(secs_to_ms(0), 0);
    }
}
