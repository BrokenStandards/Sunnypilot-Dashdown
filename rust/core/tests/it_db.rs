//! Integration: migrations + device/segment round-trips through the Repo.

use dashdown_core::db::Repo;
use dashdown_core::model::{
    ConnMode, Device, FileKind, FileSelection, Segment, SegmentFile, SegmentName,
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
        file_selection: FileSelection::PreviewsOnly,
        retention_max_minutes: Some(120),
        auto_delete_from_comma: false,
        auto_delete_min_age_min: 60,
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
    assert_eq!(repo.schema_version().unwrap(), 1);

    let id = repo.insert_device(&sample_device()).unwrap();
    let got = repo.get_device(id).unwrap().unwrap();
    assert_eq!(got.id, id);
    assert_eq!(got.name, "Comma 3X");
    assert_eq!(got.active_mode, ConnMode::Hotspot);
    assert_eq!(got.file_selection, FileSelection::PreviewsOnly);
    assert_eq!(got.port, 3923);
    assert_eq!(got.retention_max_minutes, Some(120));
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
        assert_eq!(repo.schema_version().unwrap(), 1);
        repo.insert_device(&sample_device()).unwrap()
    };

    // Re-open the same file: migrations must NOT re-run, data must persist.
    let repo = Repo::open(&path).unwrap();
    assert_eq!(repo.schema_version().unwrap(), 1);
    assert!(repo.get_device(id).unwrap().is_some());
    assert_eq!(repo.list_devices().unwrap().len(), 1);
}
