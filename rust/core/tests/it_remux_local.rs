//! On-demand remux check against a *real* HEVC file (gated; not run in CI).
//!
//! Verifies the HEVC→MP4 remux ([`dashdown_core::video::remux`]) on actual comma
//! footage and writes the result so it can be validated with a conformant decoder
//! (ffprobe/ffmpeg) or pushed to a device. Set both env vars to run:
//!
//! ```sh
//! DASHDOWN_REMUX_INPUT=/tmp/fcamera_seg0.hevc \
//! DASHDOWN_REMUX_OUTPUT=/tmp/fcamera_seg0.mp4 \
//!   cargo test -p dashdown-core --test it_remux_local -- --nocapture
//! ```
//!
//! Without the env vars it self-skips (so a plain `cargo test` stays hermetic).

use dashdown_core::video::remux::hevc_annexb_to_mp4;

#[test]
fn remux_real_hevc_file() {
    let (Ok(input_path), Ok(output_path)) = (
        std::env::var("DASHDOWN_REMUX_INPUT"),
        std::env::var("DASHDOWN_REMUX_OUTPUT"),
    ) else {
        eprintln!("skipping: set DASHDOWN_REMUX_INPUT + DASHDOWN_REMUX_OUTPUT to run");
        return;
    };

    let input = std::fs::read(&input_path).expect("read input HEVC");
    let mp4 = hevc_annexb_to_mp4(&input).expect("remux succeeds");
    std::fs::write(&output_path, &mp4).expect("write output MP4");

    eprintln!(
        "remuxed {} ({} bytes) -> {} ({} bytes)",
        input_path,
        input.len(),
        output_path,
        mp4.len()
    );
    // Sanity: an MP4 with the boxes a player needs.
    assert_eq!(&mp4[4..8], b"ftyp");
    for tag in [b"moov", b"hvc1", b"hvcC", b"mdat"] {
        assert!(
            mp4.windows(4).any(|w| w == tag),
            "missing box {:?}",
            std::str::from_utf8(tag).unwrap()
        );
    }
    // Output should be close to the input size (lossless copy, container overhead
    // small relative to a multi-MB segment).
    assert!(mp4.len() > input.len() / 2, "output implausibly small");
}
