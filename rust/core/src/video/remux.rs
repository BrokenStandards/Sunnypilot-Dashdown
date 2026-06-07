//! Lossless remux of a raw HEVC (H.265) Annex-B bytestream into a plain
//! progressive MP4 carrying an `hvc1` sample entry. The comma HD cameras
//! (`fcamera`/`ecamera`/`dcamera.hevc`) are raw HEVC with no container, which
//! ExoPlayer/AVPlayer cannot play; after this remux they play with frame-accurate
//! seeking. Pure bytestream surgery — **no decode, no re-encode, no C deps** —
//! so it is deterministic, unit-testable, and reused verbatim on iOS.
//!
//! Pipeline:
//!   1. split the Annex-B stream into NAL units (3- or 4-byte start codes),
//!   2. collect VPS/SPS/PPS into the `hvcC` configuration record (parameter sets
//!      go out-of-band → `hvc1`, not `hev1`),
//!   3. group the VCL slice NALs into one length-prefixed (4-byte) sample per
//!      coded picture — the comma encoder emits **2 slices per frame**, joined by
//!      `first_slice_segment_in_pic_flag` — marking IRAP pictures as sync samples,
//!   4. synthesize 20 fps timing — a raw stream has no timestamps; the comma
//!      cameras are 20 fps CBR — and
//!   5. write `ftyp` + `moov` (a full `stbl`: `stts`/`stss`/`stsc`/`stsz`/`co64`
//!      → exact sample tables → frame-accurate `seekTo`) + `mdat`.
//!
//! We emit a plain (non-fragmented) MP4, not fMP4: the whole input is known up
//! front and the output is a cached file, so a single `moov` is simpler and just
//! as seekable; fragmentation only earns its keep for live/streaming muxing.
//!
//! Assumption valid for the comma encoder (low-latency hardware HEVC): no
//! B-frames, so output order is monotonic and no `ctts` is needed. (Multi-slice
//! pictures *are* handled — see step 3 — but a B-frame stream would need POC
//! parsing for composition offsets.) Verified end-to-end against real comma-4
//! footage: a 60 s `fcamera.hevc` round-trips to 1200 frames @ 20 fps that a
//! conformant HEVC decoder plays and seeks without warnings.

use crate::error::{CoreError, Result};

const FPS: u32 = 20;
const MEDIA_TIMESCALE: u32 = 90_000; // ticks/sec for the media (track) timeline
const SAMPLE_DELTA: u32 = MEDIA_TIMESCALE / FPS; // 4500 ticks = 50 ms/frame
const MOVIE_TIMESCALE: u32 = 1_000; // movie/track header timeline (ms)

const NAL_VPS: u8 = 32;
const NAL_SPS: u8 = 33;
const NAL_PPS: u8 = 34;

/// Identity 3x3 video transform matrix (16.16 fixed point) for `tkhd`/`mvhd`.
const IDENTITY_MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

/// One coded picture = one MP4 sample. A picture may be split into several VCL
/// slice NALs (the comma encoder emits 2 slices per frame); they are grouped into
/// a single sample by `first_slice_segment_in_pic_flag`. Each NAL is kept as a
/// byte range into the input so frame data isn't copied until `mdat` assembly.
struct SampleRef {
    nals: Vec<(usize, usize)>,
    is_sync: bool,
}

impl SampleRef {
    /// On-disk sample size: each slice gets a 4-byte length prefix.
    fn size(&self) -> usize {
        self.nals.iter().map(|(s, e)| 4 + (e - s)).sum()
    }
}

/// The parameter-set NAL bytes (Annex-B payload, no start code), kept in stream
/// order with exact duplicates dropped.
#[derive(Default)]
struct ParameterSets {
    vps: Vec<Vec<u8>>,
    sps: Vec<Vec<u8>>,
    pps: Vec<Vec<u8>>,
}

/// The SPS fields needed to build a correct `hvcC` + visual sample entry.
struct SpsInfo {
    general_ptl: [u8; 12], // general profile_tier_level: copied verbatim into hvcC
    max_sub_layers_minus1: u8,
    temporal_id_nesting: u8,
    chroma_format_idc: u8,
    bit_depth_luma_minus8: u8,
    bit_depth_chroma_minus8: u8,
    width: u16,
    height: u16,
}

/// Remux a raw HEVC Annex-B stream into a self-contained `hvc1` MP4. Errors if no
/// SPS or no VCL frames are present (an unplayable/empty stream).
pub fn hevc_annexb_to_mp4(input: &[u8]) -> Result<Vec<u8>> {
    let mut params = ParameterSets::default();
    let mut samples: Vec<SampleRef> = Vec::new();

    for (start, end) in iter_nals(input) {
        if end - start < 2 {
            continue; // too short to hold a NAL header
        }
        let nal_type = (input[start] >> 1) & 0x3F;
        match nal_type {
            NAL_VPS => push_unique(&mut params.vps, &input[start..end]),
            NAL_SPS => push_unique(&mut params.sps, &input[start..end]),
            NAL_PPS => push_unique(&mut params.pps, &input[start..end]),
            // VCL NAL units (coded slices) = 0..=31. Everything else
            // (AUD 35, EOS/EOB/FD 36-38, SEI 39/40, reserved/unspec) is dropped:
            // a decoder reconstructs from the parameter sets + slices alone.
            0..=31 => {
                // first_slice_segment_in_pic_flag is the first bit of the slice
                // segment header (right after the 2-byte NAL header); 1 ⇒ this
                // slice starts a new picture, 0 ⇒ it continues the current one.
                // The comma encoder emits 2 slices per frame, so continuation
                // slices must be merged into one sample or the decoder drops them.
                let first_slice = end - start >= 3 && (input[start + 2] >> 7) & 1 == 1;
                if first_slice || samples.last().is_none() {
                    samples.push(SampleRef {
                        nals: vec![(start, end)],
                        // IRAP pictures (BLA 16-18, IDR 19-20, CRA 21, reserved
                        // 22-23) are random-access points → sync samples.
                        is_sync: (16..=23).contains(&nal_type),
                    });
                } else {
                    // Continuation slice of the current picture.
                    samples.last_mut().unwrap().nals.push((start, end));
                }
            }
            _ => {}
        }
    }

    if params.sps.is_empty() {
        return Err(CoreError::Parse("hevc: no SPS in stream".into()));
    }
    if samples.is_empty() {
        return Err(CoreError::Parse("hevc: no video frames in stream".into()));
    }

    let sps = parse_sps(&params.sps[0])?;
    let hvcc = build_hvcc(&params, &sps);

    let n = samples.len() as u32;
    let sizes: Vec<u32> = samples.iter().map(|s| s.size() as u32).collect();
    let mut sync: Vec<u32> = samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_sync)
        .map(|(i, _)| i as u32 + 1) // stss indices are 1-based
        .collect();
    // A stream with no random-access point is normally unplayable; mark the first
    // frame so a player can at least start it rather than refuse to seek at all.
    if sync.is_empty() {
        sync.push(1);
    }

    let mdat_payload_len: u64 = sizes.iter().map(|&s| s as u64).sum();
    let ftyp = build_ftyp();
    // co64 holds a fixed-width 64-bit offset, so moov's length is independent of
    // the offset value: measure with a placeholder, then build with the real one.
    let moov_len = build_moov(0, &sps, &hvcc, n, &sizes, &sync).len();
    let mdat_offset = ftyp.len() as u64 + moov_len as u64 + 8; // +8 for mdat header
    let moov = build_moov(mdat_offset, &sps, &hvcc, n, &sizes, &sync);
    debug_assert_eq!(
        moov.len(),
        moov_len,
        "moov length must not depend on co64 value"
    );

    let mdat_box_size = 8 + mdat_payload_len;
    if mdat_box_size > u32::MAX as u64 {
        return Err(CoreError::Parse(
            "hevc: stream too large for 32-bit mdat".into(),
        ));
    }

    let total = ftyp.len() + moov.len() + mdat_box_size as usize;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&ftyp);
    out.extend_from_slice(&moov);
    push_u32(&mut out, mdat_box_size as u32);
    out.extend_from_slice(b"mdat");
    for s in &samples {
        for &(start, end) in &s.nals {
            let nal = &input[start..end];
            push_u32(&mut out, nal.len() as u32); // 4-byte length prefix (lengthSizeMinusOne=3)
            out.extend_from_slice(nal);
        }
    }
    Ok(out)
}

/// Iterate NAL unit payload ranges `(start, end)` (excluding start codes) in an
/// Annex-B stream, handling both 3-byte (`00 00 01`) and 4-byte (`00 00 00 01`)
/// start codes and trimming trailing zero bytes before the next start code.
fn iter_nals(data: &[u8]) -> Vec<(usize, usize)> {
    let n = data.len();
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= n {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3); // payload begins right after the 00 00 01
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut nals = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        // The next NAL's payload begins at starts[k+1]; its start code is the 3
        // bytes before that, plus any leading zero of a 4-byte variant. Trim
        // trailing zeros so they aren't mistaken for NAL content.
        let mut e = starts.get(k + 1).map(|&ns| ns - 3).unwrap_or(n);
        while e > s && data[e - 1] == 0 {
            e -= 1;
        }
        if e > s {
            nals.push((s, e));
        }
    }
    nals
}

fn push_unique(set: &mut Vec<Vec<u8>>, nal: &[u8]) {
    if !set.iter().any(|x| x == nal) {
        set.push(nal.to_vec());
    }
}

/// Strip HEVC emulation-prevention bytes: every `00 00 03` becomes `00 00` (the
/// `03` was inserted by the encoder to prevent false start codes inside an NAL).
fn remove_emulation_prevention(ebsp: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ebsp.len());
    let mut i = 0;
    while i < ebsp.len() {
        if i + 2 < ebsp.len() && ebsp[i] == 0 && ebsp[i + 1] == 0 && ebsp[i + 2] == 3 {
            out.push(0);
            out.push(0);
            i += 3; // drop the emulation_prevention_three_byte
        } else {
            out.push(ebsp[i]);
            i += 1;
        }
    }
    out
}

/// Parse the fields of an SPS NAL needed for `hvcC` + the visual sample entry.
/// Walks `profile_tier_level` (general + any sub-layers) and the exp-Golomb
/// chroma/dimension/bit-depth fields; everything after is ignored.
fn parse_sps(nal: &[u8]) -> Result<SpsInfo> {
    if nal.len() < 3 {
        return Err(CoreError::Parse("hevc: SPS NAL too short".into()));
    }
    let rbsp = remove_emulation_prevention(&nal[2..]); // skip the 2-byte NAL header
    let mut r = BitReader::new(&rbsp);

    let _sps_video_parameter_set_id = r.read_bits(4)?;
    let max_sub_layers_minus1 = r.read_bits(3)? as u8;
    let temporal_id_nesting = r.read_bits(1)? as u8;

    // profile_tier_level(profilePresentFlag=1, maxNumSubLayersMinus1).
    let mut general_ptl = [0u8; 12];
    for b in general_ptl.iter_mut() {
        *b = r.read_bits(8)? as u8; // general PTL = 96 bits (incl. general_level_idc)
    }
    if max_sub_layers_minus1 > 0 {
        let mut sub_profile = [false; 8];
        let mut sub_level = [false; 8];
        for i in 0..max_sub_layers_minus1 as usize {
            sub_profile[i] = r.read_bits(1)? == 1;
            sub_level[i] = r.read_bits(1)? == 1;
        }
        for _ in max_sub_layers_minus1..8 {
            r.read_bits(2)?; // reserved_zero_2bits
        }
        for i in 0..max_sub_layers_minus1 as usize {
            if sub_profile[i] {
                r.skip_bits(88)?; // sub_layer profile fields (2+1+5+32+48)
            }
            if sub_level[i] {
                r.read_bits(8)?; // sub_layer_level_idc
            }
        }
    }

    let _sps_seq_parameter_set_id = r.read_ue()?;
    let chroma_format_idc = r.read_ue()?;
    if chroma_format_idc == 3 {
        r.read_bits(1)?; // separate_colour_plane_flag
    }
    let width = r.read_ue()?;
    let height = r.read_ue()?;
    if r.read_bits(1)? == 1 {
        // conformance_window_flag → 4 cropping offsets
        r.read_ue()?;
        r.read_ue()?;
        r.read_ue()?;
        r.read_ue()?;
    }
    let bit_depth_luma_minus8 = r.read_ue()?;
    let bit_depth_chroma_minus8 = r.read_ue()?;

    Ok(SpsInfo {
        general_ptl,
        max_sub_layers_minus1,
        temporal_id_nesting,
        chroma_format_idc: chroma_format_idc as u8,
        bit_depth_luma_minus8: bit_depth_luma_minus8 as u8,
        bit_depth_chroma_minus8: bit_depth_chroma_minus8 as u8,
        width: width.min(u16::MAX as u32) as u16,
        height: height.min(u16::MAX as u32) as u16,
    })
}

/// Build the HEVCDecoderConfigurationRecord (`hvcC` box payload). The general
/// `profile_tier_level` bytes are copied straight from the SPS; the remaining
/// fields use the SPS-derived chroma/bit-depth and safe defaults elsewhere.
fn build_hvcc(ps: &ParameterSets, sps: &SpsInfo) -> Vec<u8> {
    let mut h = Vec::new();
    h.push(1); // configurationVersion
    h.extend_from_slice(&sps.general_ptl); // 12 bytes: profile_space..general_level_idc
    push_u16(&mut h, 0xF000); // reserved(4)=1111 | min_spatial_segmentation_idc(12)=0
    h.push(0xFC); // reserved(6) | parallelismType(2)=0
    h.push(0xFC | (sps.chroma_format_idc & 0x3)); // reserved(6) | chromaFormat
    h.push(0xF8 | (sps.bit_depth_luma_minus8 & 0x7)); // reserved(5) | bitDepthLumaMinus8
    h.push(0xF8 | (sps.bit_depth_chroma_minus8 & 0x7)); // reserved(5) | bitDepthChromaMinus8
    push_u16(&mut h, 0); // avgFrameRate (0 = unspecified)
    let num_temporal_layers = sps.max_sub_layers_minus1 + 1;
    // constantFrameRate(2)=0 | numTemporalLayers(3) | temporalIdNested(1) | lengthSizeMinusOne(2)=3
    h.push(((num_temporal_layers & 0x7) << 3) | ((sps.temporal_id_nesting & 1) << 2) | 0x3);

    let arrays: [(u8, &Vec<Vec<u8>>); 3] =
        [(NAL_VPS, &ps.vps), (NAL_SPS, &ps.sps), (NAL_PPS, &ps.pps)];
    let num_arrays = arrays.iter().filter(|(_, v)| !v.is_empty()).count() as u8;
    h.push(num_arrays);
    for (nal_type, nalus) in arrays {
        if nalus.is_empty() {
            continue;
        }
        h.push(0x80 | (nal_type & 0x3F)); // array_completeness=1 | reserved=0 | NAL_unit_type
        push_u16(&mut h, nalus.len() as u16);
        for nalu in nalus {
            push_u16(&mut h, nalu.len() as u16);
            h.extend_from_slice(nalu);
        }
    }
    h
}

// ---- box construction -------------------------------------------------------

fn push_u16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn push_u32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn push_u64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// Wrap `payload` in an MP4 box with the four-character `typ`.
fn mp4box(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(8 + payload.len());
    push_u32(&mut b, (8 + payload.len()) as u32);
    b.extend_from_slice(typ);
    b.extend_from_slice(payload);
    b
}

fn build_ftyp() -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(b"isom"); // major_brand
    push_u32(&mut p, 0x200); // minor_version
    for brand in [b"isom", b"iso2", b"mp41", b"hvc1"] {
        p.extend_from_slice(brand);
    }
    mp4box(b"ftyp", &p)
}

fn build_visual_sample_entry(sps: &SpsInfo, hvcc: &[u8]) -> Vec<u8> {
    let mut e = Vec::new();
    e.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // SampleEntry reserved[6]
    push_u16(&mut e, 1); // data_reference_index
    push_u16(&mut e, 0); // pre_defined
    push_u16(&mut e, 0); // reserved
    e.extend_from_slice(&[0u8; 12]); // pre_defined[3]
    push_u16(&mut e, sps.width);
    push_u16(&mut e, sps.height);
    push_u32(&mut e, 0x0048_0000); // horizresolution 72 dpi
    push_u32(&mut e, 0x0048_0000); // vertresolution 72 dpi
    push_u32(&mut e, 0); // reserved
    push_u16(&mut e, 1); // frame_count
    let mut name = [0u8; 32]; // compressorname: 1-byte length + padded string
    let label = b"HEVC";
    name[0] = label.len() as u8;
    name[1..1 + label.len()].copy_from_slice(label);
    e.extend_from_slice(&name);
    push_u16(&mut e, 0x0018); // depth
    e.extend_from_slice(&[0xFF, 0xFF]); // pre_defined = -1
    e.extend_from_slice(&mp4box(b"hvcC", hvcc));
    mp4box(b"hvc1", &e)
}

fn build_stbl(
    sps: &SpsInfo,
    hvcc: &[u8],
    n: u32,
    sizes: &[u32],
    sync: &[u32],
    mdat_offset: u64,
) -> Vec<u8> {
    let hvc1 = build_visual_sample_entry(sps, hvcc);
    let mut stsd = Vec::new();
    push_u32(&mut stsd, 0); // version + flags
    push_u32(&mut stsd, 1); // entry_count
    stsd.extend_from_slice(&hvc1);
    let stsd = mp4box(b"stsd", &stsd);

    let mut stts = Vec::new();
    push_u32(&mut stts, 0);
    push_u32(&mut stts, 1); // entry_count
    push_u32(&mut stts, n); // sample_count
    push_u32(&mut stts, SAMPLE_DELTA); // sample_delta (all frames equal)
    let stts = mp4box(b"stts", &stts);

    // stss is only needed when not every sample is a sync sample.
    let stss = if (sync.len() as u32) < n {
        let mut s = Vec::new();
        push_u32(&mut s, 0);
        push_u32(&mut s, sync.len() as u32);
        for &idx in sync {
            push_u32(&mut s, idx);
        }
        Some(mp4box(b"stss", &s))
    } else {
        None
    };

    let mut stsc = Vec::new();
    push_u32(&mut stsc, 0);
    push_u32(&mut stsc, 1); // entry_count
    push_u32(&mut stsc, 1); // first_chunk
    push_u32(&mut stsc, n); // samples_per_chunk (one chunk holds all samples)
    push_u32(&mut stsc, 1); // sample_description_index
    let stsc = mp4box(b"stsc", &stsc);

    let mut stsz = Vec::new();
    push_u32(&mut stsz, 0);
    push_u32(&mut stsz, 0); // sample_size = 0 → per-sample sizes follow
    push_u32(&mut stsz, n);
    for &sz in sizes {
        push_u32(&mut stsz, sz);
    }
    let stsz = mp4box(b"stsz", &stsz);

    let mut co64 = Vec::new();
    push_u32(&mut co64, 0);
    push_u32(&mut co64, 1); // entry_count (single chunk)
    push_u64(&mut co64, mdat_offset);
    let co64 = mp4box(b"co64", &co64);

    let mut body = Vec::new();
    body.extend_from_slice(&stsd);
    body.extend_from_slice(&stts);
    if let Some(stss) = stss {
        body.extend_from_slice(&stss);
    }
    body.extend_from_slice(&stsc);
    body.extend_from_slice(&stsz);
    body.extend_from_slice(&co64);
    mp4box(b"stbl", &body)
}

fn build_minf(
    sps: &SpsInfo,
    hvcc: &[u8],
    n: u32,
    sizes: &[u32],
    sync: &[u32],
    mdat_offset: u64,
) -> Vec<u8> {
    let mut vmhd = Vec::new();
    push_u32(&mut vmhd, 1); // version 0, flags 1
    push_u16(&mut vmhd, 0); // graphicsmode
    push_u16(&mut vmhd, 0); // opcolor[0]
    push_u16(&mut vmhd, 0);
    push_u16(&mut vmhd, 0);
    let vmhd = mp4box(b"vmhd", &vmhd);

    let mut url = Vec::new();
    push_u32(&mut url, 1); // flags=1 → media is self-contained (no URL string)
    let url = mp4box(b"url ", &url);
    let mut dref = Vec::new();
    push_u32(&mut dref, 0);
    push_u32(&mut dref, 1); // entry_count
    dref.extend_from_slice(&url);
    let dref = mp4box(b"dref", &dref);
    let dinf = mp4box(b"dinf", &dref);

    let stbl = build_stbl(sps, hvcc, n, sizes, sync, mdat_offset);

    let mut body = Vec::new();
    body.extend_from_slice(&vmhd);
    body.extend_from_slice(&dinf);
    body.extend_from_slice(&stbl);
    mp4box(b"minf", &body)
}

fn build_mdia(
    sps: &SpsInfo,
    hvcc: &[u8],
    n: u32,
    sizes: &[u32],
    sync: &[u32],
    mdat_offset: u64,
) -> Vec<u8> {
    let mut mdhd = Vec::new();
    push_u32(&mut mdhd, 0); // version 0, flags 0
    push_u32(&mut mdhd, 0); // creation_time
    push_u32(&mut mdhd, 0); // modification_time
    push_u32(&mut mdhd, MEDIA_TIMESCALE);
    push_u32(&mut mdhd, n * SAMPLE_DELTA); // duration in media timescale
    push_u16(&mut mdhd, 0x55C4); // language 'und'
    push_u16(&mut mdhd, 0); // pre_defined
    let mdhd = mp4box(b"mdhd", &mdhd);

    let mut hdlr = Vec::new();
    push_u32(&mut hdlr, 0); // version + flags
    push_u32(&mut hdlr, 0); // pre_defined
    hdlr.extend_from_slice(b"vide"); // handler_type
    push_u32(&mut hdlr, 0); // reserved[0]
    push_u32(&mut hdlr, 0); // reserved[1]
    push_u32(&mut hdlr, 0); // reserved[2]
    hdlr.extend_from_slice(b"VideoHandler\0"); // name
    let hdlr = mp4box(b"hdlr", &hdlr);

    let minf = build_minf(sps, hvcc, n, sizes, sync, mdat_offset);

    let mut body = Vec::new();
    body.extend_from_slice(&mdhd);
    body.extend_from_slice(&hdlr);
    body.extend_from_slice(&minf);
    mp4box(b"mdia", &body)
}

fn build_trak(
    sps: &SpsInfo,
    hvcc: &[u8],
    n: u32,
    sizes: &[u32],
    sync: &[u32],
    mdat_offset: u64,
) -> Vec<u8> {
    let movie_duration = (n * MOVIE_TIMESCALE) / FPS; // ms
    let mut tkhd = Vec::new();
    push_u32(&mut tkhd, 0x0000_0007); // version 0, flags: enabled | in_movie | in_preview
    push_u32(&mut tkhd, 0); // creation_time
    push_u32(&mut tkhd, 0); // modification_time
    push_u32(&mut tkhd, 1); // track_id
    push_u32(&mut tkhd, 0); // reserved
    push_u32(&mut tkhd, movie_duration);
    push_u32(&mut tkhd, 0); // reserved[0]
    push_u32(&mut tkhd, 0); // reserved[1]
    push_u16(&mut tkhd, 0); // layer
    push_u16(&mut tkhd, 0); // alternate_group
    push_u16(&mut tkhd, 0); // volume (0 for video)
    push_u16(&mut tkhd, 0); // reserved
    for m in IDENTITY_MATRIX {
        push_u32(&mut tkhd, m);
    }
    push_u32(&mut tkhd, (sps.width as u32) << 16); // width (16.16)
    push_u32(&mut tkhd, (sps.height as u32) << 16); // height (16.16)
    let tkhd = mp4box(b"tkhd", &tkhd);

    let mdia = build_mdia(sps, hvcc, n, sizes, sync, mdat_offset);

    let mut body = Vec::new();
    body.extend_from_slice(&tkhd);
    body.extend_from_slice(&mdia);
    mp4box(b"trak", &body)
}

fn build_moov(
    mdat_offset: u64,
    sps: &SpsInfo,
    hvcc: &[u8],
    n: u32,
    sizes: &[u32],
    sync: &[u32],
) -> Vec<u8> {
    let movie_duration = (n * MOVIE_TIMESCALE) / FPS; // ms
    let mut mvhd = Vec::new();
    push_u32(&mut mvhd, 0); // version + flags
    push_u32(&mut mvhd, 0); // creation_time
    push_u32(&mut mvhd, 0); // modification_time
    push_u32(&mut mvhd, MOVIE_TIMESCALE);
    push_u32(&mut mvhd, movie_duration);
    push_u32(&mut mvhd, 0x0001_0000); // rate 1.0
    push_u16(&mut mvhd, 0x0100); // volume 1.0
    push_u16(&mut mvhd, 0); // reserved
    push_u32(&mut mvhd, 0); // reserved[0]
    push_u32(&mut mvhd, 0); // reserved[1]
    for m in IDENTITY_MATRIX {
        push_u32(&mut mvhd, m);
    }
    for _ in 0..6 {
        push_u32(&mut mvhd, 0); // pre_defined[6]
    }
    push_u32(&mut mvhd, 2); // next_track_id
    let mvhd = mp4box(b"mvhd", &mvhd);

    let trak = build_trak(sps, hvcc, n, sizes, sync, mdat_offset);

    let mut body = Vec::new();
    body.extend_from_slice(&mvhd);
    body.extend_from_slice(&trak);
    mp4box(b"moov", &body)
}

// ---- bit reader (exp-Golomb) ------------------------------------------------

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize, // current bit position
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read_bit(&mut self) -> Result<u32> {
        let byte = self.pos / 8;
        if byte >= self.data.len() {
            return Err(CoreError::Parse("hevc: SPS bit read past end".into()));
        }
        let bit = 7 - (self.pos % 8);
        self.pos += 1;
        Ok(((self.data[byte] >> bit) & 1) as u32)
    }

    fn read_bits(&mut self, n: u32) -> Result<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.read_bit()?;
        }
        Ok(v)
    }

    fn skip_bits(&mut self, n: u32) -> Result<()> {
        for _ in 0..n {
            self.read_bit()?;
        }
        Ok(())
    }

    /// Unsigned exp-Golomb `ue(v)`.
    fn read_ue(&mut self) -> Result<u32> {
        let mut zeros = 0u32;
        while self.read_bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return Err(CoreError::Parse("hevc: exp-Golomb overflow".into()));
            }
        }
        if zeros == 0 {
            return Ok(0);
        }
        let rest = self.read_bits(zeros)?;
        Ok((1u32 << zeros) - 1 + rest)
    }
}

/// Minimal MSB-first bit writer for synthesizing test NAL payloads (shared by
/// this module's tests and `super`'s `ensure_playable_mp4` test).
#[cfg(test)]
pub(crate) struct BitWriter {
    data: Vec<u8>,
    nbits: usize,
}
#[cfg(test)]
impl BitWriter {
    pub(crate) fn new() -> Self {
        Self {
            data: Vec::new(),
            nbits: 0,
        }
    }
    pub(crate) fn put_bit(&mut self, b: u32) {
        if self.nbits.is_multiple_of(8) {
            self.data.push(0);
        }
        if b & 1 == 1 {
            let i = self.data.len() - 1;
            self.data[i] |= 1 << (7 - (self.nbits % 8));
        }
        self.nbits += 1;
    }
    pub(crate) fn put_bits(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            self.put_bit((v >> i) & 1);
        }
    }
    pub(crate) fn put_ue(&mut self, v: u32) {
        let code = v + 1;
        let nbits = 32 - code.leading_zeros();
        for _ in 0..(nbits - 1) {
            self.put_bit(0);
        }
        self.put_bits(code, nbits);
    }
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.data
    }
}

/// A synthetic SPS NAL (type 33) with a zeroed PTL, 4:2:0 chroma, and the given
/// coded dimensions — enough for the parser + muxer to exercise.
#[cfg(test)]
pub(crate) fn synth_sps(width: u32, height: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.put_bits(0, 4); // sps_video_parameter_set_id
    w.put_bits(0, 3); // sps_max_sub_layers_minus1
    w.put_bits(1, 1); // sps_temporal_id_nesting_flag
    for _ in 0..12 {
        w.put_bits(0, 8); // general profile_tier_level (96 bits)
    }
    w.put_ue(0); // sps_seq_parameter_set_id
    w.put_ue(1); // chroma_format_idc = 1 (4:2:0)
    w.put_ue(width);
    w.put_ue(height);
    w.put_bit(0); // conformance_window_flag
    w.put_ue(0); // bit_depth_luma_minus8
    w.put_ue(0); // bit_depth_chroma_minus8
    let mut nal = vec![0x42, 0x01]; // NAL header: type 33 (SPS)
    nal.extend_from_slice(&w.into_bytes());
    nal
}

/// Prefix each NAL with a 4-byte start code to build an Annex-B stream.
#[cfg(test)]
pub(crate) fn annexb(nals: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for n in nals {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(n);
    }
    out
}

/// One VCL slice NAL of `nal_type`. `first_slice` sets
/// `first_slice_segment_in_pic_flag` (MSB of the slice-header byte) so the muxer
/// can tell picture starts from continuation slices.
#[cfg(test)]
pub(crate) fn vcl_nal(nal_type: u8, first_slice: bool, tail: &[u8]) -> Vec<u8> {
    let mut nal = vec![(nal_type << 1) & 0x7E, 0x01]; // 2-byte NAL header
    nal.push(if first_slice { 0x80 } else { 0x00 }); // slice_segment_header[0]
    nal.extend_from_slice(tail);
    nal
}

/// A minimal but valid Annex-B stream: SPS + a single IDR frame. Used by
/// `super`'s I/O-level test without re-deriving exp-Golomb by hand.
#[cfg(test)]
pub(crate) fn minimal_stream() -> Vec<u8> {
    annexb(&[synth_sps(64, 64), vcl_nal(19, true, &[0x10])]) // SPS + IDR (type 19)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Find the byte offset of a four-character box/code in the output.
    fn find(data: &[u8], tag: &[u8; 4]) -> Option<usize> {
        data.windows(4).position(|w| w == tag)
    }
    fn be_u32(data: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
    }

    #[test]
    fn parses_sps_dimensions() {
        let sps = synth_sps(1928, 1208);
        let info = parse_sps(&sps).unwrap();
        assert_eq!(info.width, 1928);
        assert_eq!(info.height, 1208);
        assert_eq!(info.chroma_format_idc, 1);
        assert_eq!(info.max_sub_layers_minus1, 0);
        assert_eq!(info.temporal_id_nesting, 1);
    }

    #[test]
    fn removes_emulation_prevention_bytes() {
        // 00 00 03 00  →  00 00 00  (the 0x03 is stripped)
        assert_eq!(remove_emulation_prevention(&[0, 0, 3, 0]), vec![0, 0, 0]);
        // 03 not preceded by two zeros is kept
        assert_eq!(remove_emulation_prevention(&[1, 3, 4]), vec![1, 3, 4]);
        // multiple occurrences
        assert_eq!(
            remove_emulation_prevention(&[0, 0, 3, 1, 0, 0, 3, 2]),
            vec![0, 0, 1, 0, 0, 2]
        );
    }

    #[test]
    fn splits_3_and_4_byte_start_codes() {
        // 4-byte, then 3-byte start code.
        let stream = [0, 0, 0, 1, 0x26, 0xAA, 0, 0, 1, 0x02, 0xBB];
        let nals = iter_nals(&stream);
        assert_eq!(nals.len(), 2);
        assert_eq!(&stream[nals[0].0..nals[0].1], &[0x26, 0xAA]);
        assert_eq!(&stream[nals[1].0..nals[1].1], &[0x02, 0xBB]);
    }

    #[test]
    fn remux_builds_seekable_hvc1_mp4() {
        let vps = vec![0x40, 0x01, 0x00]; // type 32
        let sps = synth_sps(1928, 1208); // type 33
        let pps = vec![0x44, 0x01, 0x00]; // type 34
        let idr = vcl_nal(19, true, &[0x11, 0x22]); // IDR_W_RADL, picture start → sync
        let p1 = vcl_nal(1, true, &[0x33]); // TRAIL_R, picture start
        let p2 = vcl_nal(1, true, &[0x44, 0x55]); // TRAIL_R, picture start
        let stream = annexb(&[vps, sps, pps, idr.clone(), p1.clone(), p2.clone()]);

        let mp4 = hevc_annexb_to_mp4(&stream).unwrap();

        // Structural sanity: brands + the boxes a player needs.
        assert_eq!(&mp4[4..8], b"ftyp");
        assert!(find(&mp4, b"moov").is_some());
        assert!(find(&mp4, b"hvc1").is_some());
        assert!(find(&mp4, b"hvcC").is_some());
        assert!(find(&mp4, b"co64").is_some());
        let mdat = find(&mp4, b"mdat").expect("mdat present");

        // stsz sample_count == the 3 VCL NALs (parameter sets excluded).
        let stsz = find(&mp4, b"stsz").unwrap();
        assert_eq!(be_u32(&mp4, stsz + 12), 3, "3 video samples");

        // stss marks exactly the IDR (sample 1).
        let stss = find(&mp4, b"stss").unwrap();
        assert_eq!(be_u32(&mp4, stss + 8), 1, "one sync sample");
        assert_eq!(be_u32(&mp4, stss + 12), 1, "sync sample is frame 1");

        // mdat payload = sum of (4-byte length prefix + NAL) for the 3 VCL NALs.
        // `mdat` is the tag offset; the box size is the 4 bytes before it.
        let expected_payload = (4 + idr.len()) + (4 + p1.len()) + (4 + p2.len());
        assert_eq!(be_u32(&mp4, mdat - 4) as usize, 8 + expected_payload);

        // co64's single chunk offset points exactly at the mdat payload, which
        // begins right after the 4-byte "mdat" tag (the size precedes it).
        let payload = mdat + 4;
        let co64 = find(&mp4, b"co64").unwrap();
        let chunk_off = u64::from_be_bytes(mp4[co64 + 12..co64 + 20].try_into().unwrap());
        assert_eq!(chunk_off as usize, payload);

        // First length-prefixed sample in mdat is the IDR.
        let first_len = be_u32(&mp4, payload) as usize;
        assert_eq!(first_len, idr.len());
        assert_eq!(&mp4[payload + 4..payload + 4 + first_len], &idr[..]);
    }

    #[test]
    fn errors_without_sps() {
        let stream = annexb(&[vec![0x26, 0x01, 0x11]]); // only a VCL NAL
        assert!(hevc_annexb_to_mp4(&stream).is_err());
    }

    #[test]
    fn errors_without_video_frames() {
        let stream = annexb(&[synth_sps(64, 64)]); // SPS but no VCL NAL
        assert!(hevc_annexb_to_mp4(&stream).is_err());
    }

    #[test]
    fn all_sync_omits_stss() {
        // Two IDR frames → every sample is sync → no stss box.
        let stream = annexb(&[
            synth_sps(64, 64),
            vcl_nal(19, true, &[0x01]), // IDR (picture start)
            vcl_nal(19, true, &[0x02]), // IDR (picture start)
        ]);
        let mp4 = hevc_annexb_to_mp4(&stream).unwrap();
        assert!(find(&mp4, b"stss").is_none(), "stss omitted when all-sync");
        let stsz = find(&mp4, b"stsz").unwrap();
        assert_eq!(be_u32(&mp4, stsz + 12), 2);
    }

    #[test]
    fn groups_multislice_pictures_into_one_sample() {
        // The comma encoder splits each frame into 2 slices: the second has
        // first_slice_segment_in_pic_flag = 0 and must merge into the same sample.
        // Two pictures × 2 slices each → 2 samples (not 4).
        let stream = annexb(&[
            synth_sps(64, 64),
            vcl_nal(19, true, &[0xAA]),  // pic 0, slice 0 (IDR, sync)
            vcl_nal(19, false, &[0xBB]), // pic 0, slice 1 (continuation)
            vcl_nal(1, true, &[0xCC]),   // pic 1, slice 0
            vcl_nal(1, false, &[0xDD]),  // pic 1, slice 1 (continuation)
        ]);
        let mp4 = hevc_annexb_to_mp4(&stream).unwrap();

        let stsz = find(&mp4, b"stsz").unwrap();
        assert_eq!(be_u32(&mp4, stsz + 12), 2, "2 pictures, not 4 slices");

        // Each sample carries both of its slices (4-byte prefix per slice).
        // pic 0: (4+4) + (4+4) = 16 bytes; pic 1 the same.
        let s0 = be_u32(&mp4, stsz + 16); // first sample_size entry
        assert_eq!(s0, (4 + 4) + (4 + 4), "both slices in one sample");

        // Only the IDR picture is a sync sample.
        let stss = find(&mp4, b"stss").unwrap();
        assert_eq!(be_u32(&mp4, stss + 8), 1, "one sync sample");
        assert_eq!(be_u32(&mp4, stss + 12), 1, "sync sample is picture 1");
    }
}
