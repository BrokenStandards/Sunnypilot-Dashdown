//! Integration: the M8 `AppCore` UniFFI facade, end-to-end against mock-copyparty.
//! Exercises device CRUD, settings, sync, a real download via the progress-sink
//! callback, drive status, connectivity, drive-zip export, and the LogSink
//! bridge — all through the public FFI surface (the methods are plain async Rust
//! in-crate, so they're directly callable under a multi-thread test runtime).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dashdown_core::ffi::AppCore;
use dashdown_core::logging::{LogEvent, LogLevel, LogSink};
use dashdown_core::model::{ConnDot, ConnMode, Device, FileSelection, SyncStatus};
use dashdown_core::sync_engine::{DownloadProgress, ProgressSink};
use mock_copyparty::{fixtures, MockServer};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer as WireServer, ResponseTemplate};

const ROUTE: &str = "000001a3--c20ba54385";

fn device_at(addr: std::net::SocketAddr) -> Device {
    Device {
        id: 0,
        name: "garage".into(),
        dongle_label: None,
        hotspot_ip: addr.ip().to_string(),
        wifi_ip: None,
        port: addr.port(),
        active_mode: ConnMode::Hotspot,
        password: None,
        auto_sync: false,
        file_selection: FileSelection::previews_only(),
        retention_max_minutes: None,
        auto_delete_from_comma: false,
        auto_delete_min_age_min: 60,
    }
}

#[derive(Default)]
struct Recorder {
    progress: Mutex<Vec<DownloadProgress>>,
    completed: Mutex<Vec<String>>,
    failed: Mutex<Vec<(String, String)>>,
}
impl ProgressSink for Recorder {
    fn on_progress(&self, p: DownloadProgress) {
        self.progress.lock().unwrap().push(p);
    }
    fn on_completed(&self, drive_key: String) {
        self.completed.lock().unwrap().push(drive_key);
    }
    fn on_failed(&self, drive_key: String, error: String) {
        self.failed.lock().unwrap().push((drive_key, error));
    }
}

#[derive(Default)]
struct LogRecorder {
    events: Mutex<Vec<LogEvent>>,
}
impl LogSink for LogRecorder {
    fn on_log(&self, event: LogEvent) {
        self.events.lock().unwrap().push(event);
    }
}

fn app(dir: &std::path::Path) -> Arc<AppCore> {
    let db = dir.join("index.db");
    let mirror = dir.join("mirror");
    AppCore::new(db.to_string_lossy().into(), mirror.to_string_lossy().into()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn appcore_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let core = app(tmp.path());

    // ---- device CRUD ----
    let dev = core.add_device(device_at(srv.addr())).await.unwrap();
    assert!(dev.id > 0, "add_device echoes the assigned id");
    let id = dev.id;
    assert_eq!(core.list_devices().await.unwrap().len(), 1);

    // ---- settings round-trip (connection fields untouched) ----
    let mut settings = core.get_settings(id).await.unwrap();
    assert!(!settings.auto_sync);
    settings.auto_sync = true;
    settings.retention_max_minutes = Some(120);
    core.set_settings(id, settings.clone()).await.unwrap();
    assert_eq!(core.get_settings(id).await.unwrap(), settings);
    core.set_active_mode(id, ConnMode::Hotspot).await.unwrap();

    // ---- sync + download via the progress sink ----
    let recorder = Arc::new(Recorder::default());
    core.set_progress_sink(Some(recorder.clone() as Arc<dyn ProgressSink>));

    let drives = core.sync_now(id).await.unwrap();
    assert_eq!(drives.len(), 1);
    let dk = drives[0].drive_key.clone();
    assert_eq!(drives[0].sync_state, SyncStatus::NotDownloaded);

    let _handle = core.start_drive_download(id, dk.clone()).await.unwrap();
    // The detached download runs on AppCore's owned runtime; poll for completion.
    let mut waited = 0;
    while recorder.completed.lock().unwrap().is_empty() && waited < 200 {
        tokio::time::sleep(Duration::from_millis(25)).await;
        waited += 1;
    }
    assert_eq!(
        recorder.completed.lock().unwrap().as_slice(),
        std::slice::from_ref(&dk),
        "download completed and fired on_completed"
    );
    assert!(!recorder.progress.lock().unwrap().is_empty());

    // ---- drive status reflects completion ----
    let status = core.get_drive_status(id, dk.clone()).await.unwrap();
    assert_eq!(status.status, SyncStatus::Complete);
    assert_eq!(status.files_done, status.files_total);
    assert!(
        status.files_total >= 3,
        "qcamera per segment (single_drive=3)"
    );

    // ---- preserved pin ----
    core.set_preserved(id, dk.clone(), true).await.unwrap();
    assert!(core.get_drive(id, dk.clone()).await.unwrap().preserved);

    // ---- connectivity: reachable + idle == Green ----
    assert_eq!(
        core.check_connectivity(id).await.unwrap().dot,
        ConnDot::Green
    );

    // ---- export the (previews-only) drive to a zip and read it back ----
    let dest = tmp.path().join("drive.zip");
    core.export_drive_zip(id, dk.clone(), dest.to_string_lossy().into())
        .await
        .unwrap();
    let zip_file = std::fs::File::open(&dest).unwrap();
    let archive = zip::ZipArchive::new(zip_file).unwrap();
    assert!(archive.len() >= 3, "one qcamera entry per segment");
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    assert!(
        names.iter().all(|n| n.ends_with("qcamera.ts")),
        "only the previews selection was exported: {names:?}"
    );
    assert!(names.iter().any(|n| n.contains(ROUTE)));

    // ---- LogSink bridge: forwarding, level filtering, and redaction ----
    let logrec = Arc::new(LogRecorder::default());
    core.set_log_sink(Some(logrec.clone() as Arc<dyn LogSink>), LogLevel::Info);
    tracing::info!("appcore-log-probe");
    // At Info threshold: an Info event is forwarded, a Debug event is dropped.
    tracing::debug!("appcore-debug-DROP");
    tracing::info!("appcore-info-KEEP");
    // Secret-named fields are redacted; non-secret fields survive.
    tracing::info!(
        password = "hunter2",
        token = "abc123",
        device_id = 7,
        "auth probe REDACT"
    );
    {
        let evs = logrec.events.lock().unwrap();
        let has = |m: &str| evs.iter().any(|e| e.message.contains(m));
        assert!(has("appcore-log-probe"), "LogSink received the event");
        assert!(has("appcore-info-KEEP"), "Info forwarded");
        assert!(
            !has("appcore-debug-DROP"),
            "Debug dropped at Info threshold"
        );
        let red = evs
            .iter()
            .rev()
            .find(|e| e.message.contains("auth probe REDACT"))
            .expect("redaction event present");
        assert!(red.message.contains("password=<redacted>"));
        assert!(red.message.contains("token=<redacted>"));
        assert!(red.message.contains("device_id=7"), "non-secret field kept");
        assert!(!red.message.contains("hunter2"), "password value redacted");
        assert!(!red.message.contains("abc123"), "token value redacted");
    }

    // ---- update_device ----
    let mut dev = dev;
    dev.name = "driveway".into();
    core.update_device(dev).await.unwrap();
    assert_eq!(core.list_devices().await.unwrap()[0].name, "driveway");

    // ---- connectivity: server down == Red ----
    drop(srv);
    assert_eq!(core.check_connectivity(id).await.unwrap().dot, ConnDot::Red);

    // ---- offline list_drives reclassifies from disk (no network) == Complete ----
    let offline = core.list_drives(id, true).await.unwrap();
    assert_eq!(
        offline
            .iter()
            .find(|d| d.drive_key == dk)
            .unwrap()
            .sync_state,
        SyncStatus::Complete,
        "offline scan reflects the mirrored drive's real state"
    );

    // ---- remove_device: index rows gone, mirror dir removed ----
    core.remove_device(id).await.unwrap();
    assert!(core.list_devices().await.unwrap().is_empty());
    assert!(!tmp.path().join("mirror").join(id.to_string()).exists());
}

/// SyncHandle.cancel() through AppCore actually stops the runtime-detached
/// download. Uses a wiremock device whose file GET stalls, so cancel is
/// deterministic (the transfer cannot finish on its own within the test).
#[tokio::test(flavor = "multi_thread")]
async fn appcore_cancel_stops_download() {
    let seg = "000001a3--c20ba54385--0";
    let server = WireServer::start().await;
    // realdata/ listing -> one segment dir.
    Mock::given(method("GET"))
        .and(path("/realdata/"))
        .and(query_param("ls", "j"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            r#"{{"dirs":[{{"href":"{seg}/","sz":0,"ts":1690000000}}],"files":[]}}"#
        )))
        .mount(&server)
        .await;
    // segment listing -> one qcamera file.
    Mock::given(method("GET"))
        .and(path(format!("/realdata/{seg}/")))
        .and(query_param("ls", "j"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"dirs":[],"files":[{"href":"qcamera.ts","sz":1200,"ts":1690000000}]}"#,
        ))
        .mount(&server)
        .await;
    // file GET stalls 30s so the download is in-flight when we cancel.
    Mock::given(method("GET"))
        .and(path(format!("/realdata/{seg}/qcamera.ts")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(vec![0u8; 1200])
                .set_delay(Duration::from_secs(30)),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let core = app(tmp.path());
    let dev = core.add_device(device_at(*server.address())).await.unwrap();
    let id = dev.id;
    let dk = core.sync_now(id).await.unwrap()[0].drive_key.clone();

    let handle = core.start_drive_download(id, dk.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await; // let the download start + stall
    assert!(!handle.is_cancelled());
    handle.cancel();
    assert!(handle.is_cancelled());

    // It must land NotDownloaded (the Canceled terminal), never Complete.
    let mut waited = 0;
    loop {
        let st = core.get_drive_status(id, dk.clone()).await.unwrap();
        assert_ne!(
            st.status,
            SyncStatus::Complete,
            "cancelled must not Complete"
        );
        if st.status == SyncStatus::NotDownloaded || waited > 200 {
            assert_eq!(
                st.status,
                SyncStatus::NotDownloaded,
                "cancel reached terminal"
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
        waited += 1;
    }
}

/// export_drive_zip skips files that aren't fully mirrored: a synced-but-not-
/// downloaded drive exports to a valid, empty archive (rather than erroring).
#[tokio::test(flavor = "multi_thread")]
async fn appcore_export_skips_incomplete_files() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let core = app(tmp.path());
    let id = core.add_device(device_at(srv.addr())).await.unwrap().id;
    let dk = core.sync_now(id).await.unwrap()[0].drive_key.clone();
    // No download performed -> nothing is locally complete.

    let dest = tmp.path().join("nested/out/empty.zip");
    core.export_drive_zip(id, dk, dest.to_string_lossy().into())
        .await
        .unwrap();
    // Parent dirs were created and the archive opens — just with no entries.
    let archive = zip::ZipArchive::new(std::fs::File::open(&dest).unwrap()).unwrap();
    assert_eq!(archive.len(), 0, "no complete files -> empty archive");
}

/// Missing device / drive surface as CoreError::NotFound across the facade.
#[tokio::test(flavor = "multi_thread")]
async fn appcore_notfound_errors() {
    use dashdown_core::error::CoreError;
    let tmp = tempfile::tempdir().unwrap();
    let srv = MockServer::spawn(fixtures::single_drive(), None)
        .await
        .unwrap();
    let core = app(tmp.path());
    let id = core.add_device(device_at(srv.addr())).await.unwrap().id;

    // Missing drive on an existing device.
    assert!(matches!(
        core.get_drive(id, "nope--0".into()).await,
        Err(CoreError::NotFound(_))
    ));
    assert!(matches!(
        core.get_drive_status(id, "nope--0".into()).await,
        Err(CoreError::NotFound(_))
    ));
    // Missing device.
    assert!(matches!(
        core.get_settings(999_999).await,
        Err(CoreError::NotFound(_))
    ));
    assert!(matches!(
        core.check_connectivity(999_999).await,
        Err(CoreError::NotFound(_))
    ));
}
