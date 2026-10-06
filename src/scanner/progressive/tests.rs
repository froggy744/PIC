use super::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pic-progressive-{}-{unique}.db",
            std::process::id()
        ));
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(db::SCHEMA).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(format!("{}-wal", self.0.display()));
        let _ = std::fs::remove_file(format!("{}-shm", self.0.display()));
    }
}

struct FakeSource {
    directories: HashMap<String, Vec<Record>>,
    blocked: Option<String>,
    release: Arc<AtomicBool>,
    visits: AtomicUsize,
    fail_directory: bool,
}
impl Source for FakeSource {
    fn available(&self, _: &str) -> bool {
        true
    }
    fn visit(
        &self,
        path: &str,
        control: &ScanControl,
        emit: &mut dyn FnMut(Record) -> bool,
    ) -> Result<()> {
        self.visits.fetch_add(1, Ordering::Relaxed);
        for record in self.directories.get(path).context("unexpected directory")? {
            if !emit(record.clone()) {
                return Ok(());
            }
        }
        if self.blocked.as_deref() == Some(path) {
            while !self.release.load(Ordering::Acquire) && !control.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        if self.fail_directory {
            anyhow::bail!("directory disconnected");
        }
        Ok(())
    }
    fn metadata(&self, work: &db::NetworkWork) -> Result<db::PhotoMetadata> {
        Ok(db::PhotoMetadata {
            width: Some(6000),
            height: Some(4000),
            camera: Some("Test Camera".into()),
            mtime: work.mtime,
            size_bytes: work.size,
            ..Default::default()
        })
    }
    fn thumbnail(&self, _: &db::NetworkWork) -> Result<bool> {
        Ok(true)
    }
}
fn photo_record(path: &str) -> Record {
    Record {
        path: path.into(),
        name: path.rsplit('/').next().unwrap().into(),
        is_dir: false,
        mtime: Some(10),
        size: Some(100),
    }
}
fn source(records: Vec<Record>, blocked: Option<&str>, fail: bool) -> Arc<FakeSource> {
    Arc::new(FakeSource {
        directories: HashMap::from([("nfs://nas/photos".into(), records)]),
        blocked: blocked.map(str::to_owned),
        release: Arc::new(AtomicBool::new(false)),
        visits: AtomicUsize::new(0),
        fail_directory: fail,
    })
}

#[test]
fn first_photo_is_committed_while_directory_listing_is_still_blocked() {
    let fixture = Fixture::new();
    let source = source(
        vec![photo_record("nfs://nas/photos/first.jpg")],
        Some("nfs://nas/photos"),
        false,
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    let control = ScanControl::default();
    let worker_control = control.clone();
    let database = fixture.0.clone();
    let worker_source = source.clone();
    let worker = std::thread::spawn(move || {
        run_with_source(
            "nfs://nas/photos",
            &database,
            Some(&sender),
            &worker_control,
            false,
            worker_source,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut visible = false;
    while Instant::now() < deadline {
        if let Ok(ScanEvent::PhotosIndexed { photos, .. }) =
            receiver.recv_timeout(Duration::from_millis(100))
        {
            assert_eq!(photos[0].photo.path, "nfs://nas/photos/first.jpg");
            let connection = db::open_existing(&fixture.0).unwrap();
            assert_eq!(db::photos(&connection, None, false, None).unwrap().len(), 1);
            visible = true;
            break;
        }
    }
    control.cancel();
    source.release.store(true, Ordering::Release);
    assert!(worker.join().unwrap().is_ok());
    assert!(
        visible,
        "partial catalog batch must flush before the directory finishes"
    );
}

#[test]
fn incomplete_listing_keeps_committed_photos_and_never_reconciles_deletions() {
    let fixture = Fixture::new();
    let connection = db::open_existing(&fixture.0).unwrap();
    let folder = db::mark_import_root(&connection, "nfs://nas/photos").unwrap();
    db::upsert_photo(
        &connection,
        Path::new("nfs://nas/photos/old.jpg"),
        Some(folder),
        &db::PhotoMetadata::default(),
    )
    .unwrap();
    let source = source(vec![photo_record("nfs://nas/photos/new.jpg")], None, true);
    run_with_source(
        "nfs://nas/photos",
        &fixture.0,
        None,
        &ScanControl::default(),
        false,
        source,
    )
    .unwrap();
    assert_eq!(db::photos(&connection, None, false, None).unwrap().len(), 2);
    assert_eq!(
        db::resumable_network_imports(&connection).unwrap(),
        vec!["nfs://nas/photos"]
    );
}

#[test]
fn restart_uses_committed_directory_checkpoints_and_preserves_unseen_photos() {
    let fixture = Fixture::new();
    let connection = db::open_existing(&fixture.0).unwrap();
    let root = "nfs://nas/photos";
    let folder = db::mark_import_root(&connection, root).unwrap();
    db::start_network_job(&connection, root, false).unwrap();
    db::catalog_network_directory(&connection, root, root, None).unwrap();
    db::catalog_network_directory(&connection, root, "nfs://nas/photos/remaining", Some(root))
        .unwrap();
    connection
        .execute(
            "UPDATE network_import_dirs SET state='done' WHERE path=?1",
            [root],
        )
        .unwrap();
    db::upsert_photo(
        &connection,
        Path::new("nfs://nas/photos/old.jpg"),
        Some(folder),
        &db::PhotoMetadata::default(),
    )
    .unwrap();
    let source = Arc::new(FakeSource {
        directories: HashMap::from([(
            "nfs://nas/photos/remaining".into(),
            vec![photo_record("nfs://nas/photos/remaining/new.jpg")],
        )]),
        blocked: None,
        release: Arc::new(AtomicBool::new(true)),
        visits: AtomicUsize::new(0),
        fail_directory: false,
    });
    run_with_source(
        root,
        &fixture.0,
        None,
        &ScanControl::default(),
        true,
        source.clone(),
    )
    .unwrap();
    assert_eq!(source.visits.load(Ordering::Relaxed), 1);
    assert_eq!(db::photos(&connection, None, false, None).unwrap().len(), 2);
    assert!(db::resumable_network_imports(&connection)
        .unwrap()
        .is_empty());
}

#[test]
fn completed_scan_enriches_once_and_unchanged_refresh_skips_original_reads() {
    let fixture = Fixture::new();
    let source = source(vec![photo_record("nfs://nas/photos/a.jpg")], None, false);
    assert_eq!(
        run_with_source(
            "nfs://nas/photos",
            &fixture.0,
            None,
            &ScanControl::default(),
            false,
            source.clone()
        )
        .unwrap(),
        1
    );
    let connection = db::open_existing(&fixture.0).unwrap();
    let rows = db::photos(&connection, None, false, None).unwrap();
    assert_eq!(rows[0].camera.as_deref(), Some("Test Camera"));
    assert_eq!(rows[0].width, Some(6000));
    assert!(db::next_network_work(&connection, "nfs://nas/photos", 1)
        .unwrap()
        .is_none());
    assert_eq!(
        run_with_source(
            "nfs://nas/photos",
            &fixture.0,
            None,
            &ScanControl::default(),
            false,
            source
        )
        .unwrap(),
        0
    );
    assert_eq!(db::photos(&connection, None, false, None).unwrap().len(), 1);
}

#[test]
fn cancellation_releases_a_producer_waiting_on_a_full_queue() {
    let control = ScanControl::default();
    let producer_control = control.clone();
    let (sender, _receiver) = mpsc::sync_channel(1);
    sender.send(1).unwrap();
    let worker = std::thread::spawn(move || send_bounded(&sender, 2, &producer_control));
    control.cancel();
    assert!(!worker.join().unwrap());
}

#[test]
fn successful_fresh_scan_reconciles_missing_photos_and_nested_folders() {
    let fixture = Fixture::new();
    let connection = db::open_existing(&fixture.0).unwrap();
    let root = "nfs://nas/photos";
    let folder = db::mark_import_root(&connection, root).unwrap();
    let missing =
        db::insert_discovered_folder(&connection, "nfs://nas/photos/missing", folder).unwrap();
    db::upsert_photo(
        &connection,
        Path::new("nfs://nas/photos/missing/old.jpg"),
        Some(missing),
        &db::PhotoMetadata::default(),
    )
    .unwrap();
    let source = source(vec![photo_record("nfs://nas/photos/a.jpg")], None, false);
    run_with_source(
        root,
        &fixture.0,
        None,
        &ScanControl::default(),
        false,
        source,
    )
    .unwrap();
    assert_eq!(db::photos(&connection, None, false, None).unwrap().len(), 1);
    assert!(!db::folder_exists(&connection, missing).unwrap());
}

struct DisconnectSource {
    inner: Arc<FakeSource>,
    probes: AtomicUsize,
    reads: AtomicUsize,
    stop: Option<ScanControl>,
}
impl Source for DisconnectSource {
    fn available(&self, _: &str) -> bool {
        if let Some(control) = &self.stop {
            control.cancel();
            return false;
        }
        self.probes.fetch_add(1, Ordering::Relaxed) == 0
    }
    fn visit(
        &self,
        path: &str,
        control: &ScanControl,
        emit: &mut dyn FnMut(Record) -> bool,
    ) -> Result<()> {
        self.inner.visit(path, control, emit)
    }
    fn metadata(&self, _: &db::NetworkWork) -> Result<db::PhotoMetadata> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        anyhow::bail!("share disconnected")
    }
    fn thumbnail(&self, _: &db::NetworkWork) -> Result<bool> {
        anyhow::bail!("share disconnected")
    }
}

#[test]
fn share_loss_suspends_without_draining_pending_metadata() {
    let fixture = Fixture::new();
    let source = Arc::new(DisconnectSource {
        inner: source(
            (0..256)
                .map(|i| photo_record(&format!("nfs://nas/photos/{i}.jpg")))
                .collect(),
            None,
            false,
        ),
        probes: AtomicUsize::new(0),
        reads: AtomicUsize::new(0),
        stop: None,
    });
    run_with_source(
        "nfs://nas/photos",
        &fixture.0,
        None,
        &ScanControl::default(),
        false,
        source.clone(),
    )
    .unwrap();
    assert_eq!(source.reads.load(Ordering::Relaxed), 1);
    let connection = Connection::open(&fixture.0).unwrap();
    let status: String = connection
        .query_row("SELECT status FROM network_import_jobs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(status, "waiting");
    assert!(db::next_network_work(&connection, "nfs://nas/photos", 1)
        .unwrap()
        .is_some());
}

#[test]
fn stop_during_unavailable_probe_stays_paused() {
    let fixture = Fixture::new();
    let control = ScanControl::default();
    let source = Arc::new(DisconnectSource {
        inner: source(vec![], None, false),
        probes: AtomicUsize::new(0),
        reads: AtomicUsize::new(0),
        stop: Some(control.clone()),
    });
    run_with_source(
        "nfs://nas/photos",
        &fixture.0,
        None,
        &control,
        false,
        source,
    )
    .unwrap();
    let connection = Connection::open(&fixture.0).unwrap();
    assert!(db::resumable_network_imports(&connection)
        .unwrap()
        .is_empty());
    let status: String = connection
        .query_row("SELECT status FROM network_import_jobs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(status, "paused");
}

#[test]
fn committed_subfolders_are_published_before_enrichment_finishes() {
    let fixture = Fixture::new();
    let source = source(
        vec![Record {
            path: "nfs://nas/photos/child".into(),
            name: "child".into(),
            is_dir: true,
            mtime: None,
            size: None,
        }],
        Some("nfs://nas/photos"),
        false,
    );
    let control = ScanControl::default();
    let worker_control = control.clone();
    let database = fixture.0.clone();
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_with_source(
            "nfs://nas/photos",
            &database,
            Some(&sender),
            &worker_control,
            false,
            source,
        )
    });
    let mut published = false;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Ok(ScanEvent::FolderStarted { folder }) =
            receiver.recv_timeout(Duration::from_millis(100))
        {
            if folder.path.ends_with("/child") {
                published = true;
                break;
            }
        }
    }
    control.cancel();
    worker.join().unwrap().unwrap();
    assert!(
        published,
        "child folder must be browsable while discovery is blocked"
    );
}
