//! Live, READ-ONLY contract test against a REAL Comma device running a
//! sunnypilot/bluepilot fork with copyparty enabled.
//!
//! This is opt-in via environment variables so it never runs in CI or for
//! contributors without the hardware — it skip-passes when `DASHDOWN_LIVE_URL`
//! is unset (mirroring the SKIP convention in `it_real_copyparty.rs`).
//!
//!   DASHDOWN_LIVE_URL   copyparty base URL, e.g. http://192.168.1.181:8080/   (required)
//!   DASHDOWN_LIVE_REL   footage directory under the base (default: "routes/")
//!   DASHDOWN_LIVE_PW    copyparty password, if the volume needs one (default: none)
//!   DASHDOWN_LIVE_MAX   max segment dirs to enumerate, to stay gentle (default: 6)
//!
//! Run against the device on the LAN:
//!   DASHDOWN_LIVE_URL=http://192.168.1.181:8080/ \
//!     cargo test -p dashdown-core --test it_live_device -- --nocapture
//!
//! READ-ONLY GUARANTEE: this test only ever issues `GET` (listing + ranged
//! download). It NEVER calls `delete()` or uploads, so it cannot remove or
//! mutate anything on the device. The one file it transfers is streamed into a
//! local `TempDir` that is dropped at the end.
//!
//! Footage path note: on the observed bluepilot/comma-4 build, copyparty serves
//! the drive segments anonymously under `routes/` (the public alias), while
//! `realdata/` is a login-gated volume that answers `403` ("you'll have to log
//! in") to anonymous requests. That mismatch — our core's default base is
//! `realdata/` (`sync_engine::REALDATA_REL`) — is why the app reports an auth
//! error against this device. Hence `DASHDOWN_LIVE_REL` defaults to `routes/`.

use std::time::Duration;

use dashdown_core::copyparty_client::{CopypartyClient, Credentials};
use dashdown_core::drive_grouping::group_segments;
use dashdown_core::model::{Drive, FileKind, Segment, SegmentFile, SegmentName};
use tokio::io::AsyncWriteExt;

/// The opt-in live target, resolved from the environment. `None` ⇒ SKIP.
struct LiveTarget {
    client: CopypartyClient,
    rel: String,
    max: usize,
}

fn live_target() -> Option<LiveTarget> {
    let url = std::env::var("DASHDOWN_LIVE_URL")
        .ok()
        .filter(|s| !s.is_empty())?;
    let rel = std::env::var("DASHDOWN_LIVE_REL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "routes/".to_string());
    let pw = std::env::var("DASHDOWN_LIVE_PW").ok();
    let max = std::env::var("DASHDOWN_LIVE_MAX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6usize);
    let creds = Credentials::from_optional(pw.as_deref());
    let client = CopypartyClient::new(&url, creds).expect("build live copyparty client");
    Some(LiveTarget { client, rel, max })
}

fn ensure_slash(rel: &str) -> String {
    if rel.ends_with('/') {
        rel.to_string()
    } else {
        format!("{rel}/")
    }
}

/// Enumerate at most `max` segment directories under `rel` and build [`Segment`]s
/// — the same shape `CopypartyClient::list_segments` produces, but BOUNDED so a
/// device with hundreds of drives isn't fully crawled by a smoke test. Read-only.
async fn fetch_some_segments(t: &LiveTarget) -> Vec<Segment> {
    let base = ensure_slash(&t.rel);
    let top = t
        .client
        .list_dir(&base)
        .await
        .expect("list footage directory (anonymous read)");
    assert!(
        !top.dirs.is_empty(),
        "footage dir {base:?} should contain drive segment directories"
    );

    let mut segments = Vec::new();
    for d in &top.dirs {
        if segments.len() >= t.max {
            break;
        }
        let Ok(name) = SegmentName::parse(&d.name) else {
            continue; // non-segment dir (e.g. copyparty internals) — skip
        };
        let seg_rel = format!("{base}{}", d.href);
        let listing = t.client.list_dir(&seg_rel).await.expect("list segment dir");

        let mut files = Vec::new();
        let mut recording = false;
        for f in &listing.files {
            let kind = FileKind::from_filename(&f.name);
            if kind == FileKind::LockMarker {
                recording = true;
                continue;
            }
            files.push(SegmentFile {
                kind,
                name: f.name.clone(),
                remote_size: f.size,
                mtime_s: f.mtime_s,
            });
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));
        segments.push(Segment {
            name,
            files,
            recording,
        });
    }
    segments
}

/// List real drive segments, parse the on-disk names, and group them into drives
/// — proving our listing parser, `SegmentName` parser, and grouping all hold
/// against the authoritative output of a real device. Re-grouping is idempotent.
#[tokio::test]
async fn live_lists_and_groups_real_drives() {
    let Some(t) = live_target() else {
        eprintln!("SKIP it_live_device: set DASHDOWN_LIVE_URL to run against a device");
        return;
    };

    let segments = fetch_some_segments(&t).await;
    assert!(
        !segments.is_empty(),
        "expected at least one parseable segment under {:?}",
        t.rel
    );

    // Every enumerated segment is a real drive segment with footage files.
    for seg in &segments {
        assert!(
            !seg.files.is_empty(),
            "segment {} has no files",
            seg.name.dir_name()
        );
        for f in &seg.files {
            assert!(f.remote_size > 0, "{} reports zero size", f.name);
            assert!(f.mtime_s > 0, "{} reports no mtime", f.name);
        }
    }

    let drives: Vec<Drive> = group_segments(segments.clone());
    assert!(!drives.is_empty(), "grouping yielded no drives");
    let grouped: usize = drives.iter().map(|d| d.segment_count as usize).sum();
    assert_eq!(
        grouped,
        segments.len(),
        "every fetched segment lands in exactly one drive"
    );

    // Grouping the real data is idempotent (matches the property tests, live).
    let regrouped = group_segments(
        drives
            .iter()
            .flat_map(|d| d.segments.clone())
            .collect::<Vec<_>>(),
    );
    assert_eq!(drives, regrouped, "regrouping real segments is stable");

    eprintln!(
        "live_lists_and_groups_real_drives: {} segments -> {} drive(s) under {:?}",
        grouped,
        drives.len(),
        t.rel
    );
}

/// Copy ONE real file off the device: verify Range support (206) then stream a
/// `qcamera.ts` into a temp dir and confirm the bytes-on-disk equal the size the
/// listing advertised. This is the read/copy path the app's downloader uses.
#[tokio::test]
async fn live_range_download_one_qcamera() {
    let Some(t) = live_target() else {
        eprintln!("SKIP it_live_device: set DASHDOWN_LIVE_URL to run against a device");
        return;
    };

    // Find the first segment that actually has a qcamera.ts to copy.
    let segments = fetch_some_segments(&t).await;
    let base = ensure_slash(&t.rel);
    let pick = segments.iter().find_map(|seg| {
        seg.files.iter().find(|f| f.name == "qcamera.ts").map(|f| {
            (
                format!("{base}{}/{}", seg.name.dir_name(), f.name),
                f.remote_size,
            )
        })
    });
    let Some((rel, expected_size)) = pick else {
        panic!(
            "no qcamera.ts found in the first {} segments under {:?}",
            t.max, t.rel
        );
    };

    // The downloader relies on ranged GETs to resume; the device must answer 206.
    assert!(
        t.client.probe_range(&rel).await.expect("range probe"),
        "device should honor HTTP Range (206) for {rel}"
    );

    // Stream the whole file to local disk (read-only copy off the device).
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("qcamera.ts");
    let mut f = tokio::fs::File::create(&out).await.unwrap();
    let n = tokio::time::timeout(Duration::from_secs(60), t.client.download_to(&rel, &mut f))
        .await
        .expect("download did not time out")
        .expect("download qcamera.ts");
    f.flush().await.unwrap();

    assert_eq!(
        n, expected_size,
        "streamed byte count matches the listing size"
    );
    let on_disk = std::fs::metadata(&out).unwrap().len();
    assert_eq!(
        on_disk, expected_size,
        "file on disk matches the listing size"
    );

    eprintln!("live_range_download_one_qcamera: copied {n} bytes from {rel}");
}
