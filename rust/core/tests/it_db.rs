//! Integration: migrations + device/segment round-trips through the Repo.

use dashdown_core::db::Repo;
use dashdown_core::model::{
    ConnMode, Device, FileKind, FileSelection, JobState, Segment, SegmentFile, SegmentName,
};

fn sample_device() -> Device {
    Device {
        id: 0,
        name: "Comma 3X".into(),
        dongle_label: Some("a2a0ccea32023010".into()),
        hotspot_ip: "192.168.43.1".into(),
        wifi_ip: Some("10.0.0.5".into()),
        port: 3923,
        active_mode: ConnMode::Hotspot,
        password: Some("hunter2".into()),
        auto_sync: true,
        file_selection: FileSelection::previews_only(),
        retention_max_minutes: Some(120),
        auto_delete_from_comma: false,
        auto_delete_min_age_min: 60,
        cap_warn_enabled: true,
        cap_warn_threshold_minutes: 10,
    }
}

fn sample_segments() -> Vec<Segment> {
    vec![
        Segment {
            name: SegmentName {
                route_id: "000001a3--c20ba54385".into(),
                segment_num: 0,
            },
            recording: false,
            files: vec![
                SegmentFile {
                    kind: FileKind::QCamera,
                    name: "qcamera.ts".into(),
                    remote_size: 1200,
                    mtime_s: 1_690_462_880,
                },
                SegmentFile {
                    kind: FileKind::RLog,
                    name: "rlog.zst".into(),
                    remote_size: 300,
                    mtime_s: 1_690_462_881,
                },
            ],
        },
        Segment {
            name: SegmentName {
                route_id: "000001a3--c20ba54385".into(),
                segment_num: 1,
            },
            recording: true,
            files: vec![SegmentFile {
                kind: FileKind::QCamera,
                name: "qcamera.ts".into(),
                remote_size: 600,
                mtime_s: 1_690_462_940,
            }],
        },
    ]
}

#[test]
fn migrates_and_round_trips() {
    let repo = Repo::open_in_memory().unwrap();
    assert_eq!(repo.schema_version().unwrap(), 5);

    let id = repo.insert_device(&sample_device()).unwrap();
    let got = repo.get_device(id).unwrap().unwrap();
    assert_eq!(got.id, id);
    assert_eq!(got.name, "Comma 3X");
    assert_eq!(got.active_mode, ConnMode::Hotspot);
    assert_eq!(got.file_selection, FileSelection::previews_only());
    assert_eq!(got.port, 3923);
    assert_eq!(got.retention_max_minutes, Some(120));
    assert!(got.cap_warn_enabled);
    assert_eq!(got.cap_warn_threshold_minutes, 10);
    assert_eq!(repo.list_devices().unwrap().len(), 1);

    let segs = sample_segments();
    repo.upsert_segments(id, &segs).unwrap();
    assert_eq!(repo.get_segments(id).unwrap(), segs);

    // Idempotent: re-upsert doesn't duplicate.
    repo.upsert_segments(id, &segs).unwrap();
    assert_eq!(repo.get_segments(id).unwrap().len(), 2);
}

#[test]
fn reopen_is_idempotent_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");

    let id = {
        let repo = Repo::open(&path).unwrap();
        assert_eq!(repo.schema_version().unwrap(), 5);
        repo.insert_device(&sample_device()).unwrap()
    };

    // Re-open the same file: migrations must NOT re-run, data must persist.
    let repo = Repo::open(&path).unwrap();
    assert_eq!(repo.schema_version().unwrap(), 5);
    assert!(repo.get_device(id).unwrap().is_some());
    assert_eq!(repo.list_devices().unwrap().len(), 1);
}

#[test]
fn update_device_changes_mutable_fields() {
    let repo = Repo::open_in_memory().unwrap();
    let id = repo.insert_device(&sample_device()).unwrap();
    let mut d = repo.get_device(id).unwrap().unwrap();
    d.name = "renamed".into();
    d.active_mode = ConnMode::Wifi;
    d.auto_sync = false;
    d.retention_max_minutes = None;
    d.cap_warn_enabled = false;
    d.cap_warn_threshold_minutes = 25;
    repo.update_device(&d).unwrap();

    let got = repo.get_device(id).unwrap().unwrap();
    assert_eq!(got.name, "renamed");
    assert_eq!(got.active_mode, ConnMode::Wifi);
    assert!(!got.auto_sync);
    assert_eq!(got.retention_max_minutes, None);
    assert!(!got.cap_warn_enabled);
    assert_eq!(got.cap_warn_threshold_minutes, 25);
}

#[test]
fn delete_device_cascades_to_children() {
    use dashdown_core::drive_grouping::group_segments;
    let repo = Repo::open_in_memory().unwrap();
    let id = repo.insert_device(&sample_device()).unwrap();
    let segs = sample_segments();
    repo.upsert_segments(id, &segs).unwrap();
    let drives = group_segments(segs);
    repo.replace_drives(id, &drives).unwrap();
    let dk = drives[0].drive_key.clone();
    repo.upsert_job(id, &dk, 1, 100).unwrap();
    assert!(!repo.get_drives(id).unwrap().is_empty());

    repo.delete_device(id).unwrap();
    assert!(repo.get_device(id).unwrap().is_none());
    assert!(repo.get_drives(id).unwrap().is_empty(), "drives cascade");
    assert!(
        repo.get_segments(id).unwrap().is_empty(),
        "segments cascade"
    );
    assert!(repo.get_job(id, &dk).unwrap().is_none(), "jobs cascade");
}

#[test]
fn download_job_round_trips() {
    let repo = Repo::open_in_memory().unwrap();
    let dev = repo.insert_device(&sample_device()).unwrap();
    let key = "000001a3--c20ba54385--0";

    assert!(repo.get_job(dev, key).unwrap().is_none());

    repo.upsert_job(dev, key, 5, 17_000).unwrap();
    let j = repo.get_job(dev, key).unwrap().unwrap();
    assert_eq!(j.state, JobState::Running);
    assert_eq!(j.files_total, 5);
    assert_eq!(j.bytes_total, 17_000);
    assert_eq!(j.files_done, 0);

    repo.bump_job_progress(dev, key, 3, 9_000).unwrap();
    repo.set_job_state(dev, key, JobState::Complete, None)
        .unwrap();
    let j = repo.get_job(dev, key).unwrap().unwrap();
    assert_eq!(j.state, JobState::Complete);
    assert_eq!(j.files_done, 3);
    assert_eq!(j.bytes_done, 9_000);

    // Failure carries an error string.
    repo.set_job_state(dev, key, JobState::Failed, Some("boom"))
        .unwrap();
    let j = repo.get_job(dev, key).unwrap().unwrap();
    assert_eq!(j.state, JobState::Failed);
    assert_eq!(j.error.as_deref(), Some("boom"));

    // Re-running resets to a fresh running job.
    repo.upsert_job(dev, key, 2, 100).unwrap();
    let j = repo.get_job(dev, key).unwrap().unwrap();
    assert_eq!(j.state, JobState::Running);
    assert_eq!(j.files_done, 0);
    assert_eq!(j.error, None);
}

#[test]
fn set_file_complete_updates_seg_file() {
    let repo = Repo::open_in_memory().unwrap();
    let dev = repo.insert_device(&sample_device()).unwrap();
    repo.upsert_segments(dev, &sample_segments()).unwrap();

    // Marking an existing file complete succeeds; a missing one is a no-op.
    repo.set_file_complete(dev, "000001a3--c20ba54385", 0, "qcamera.ts", 1200)
        .unwrap();
    repo.set_file_complete(dev, "000001a3--c20ba54385", 0, "nope.bin", 1)
        .unwrap();
}
