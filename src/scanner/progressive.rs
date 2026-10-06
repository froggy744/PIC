//! Network discovery never waits for metadata or the complete directory tree.
//! The catalog writer owns commits and acknowledges durable directory ends.
use super::{send, IndexedPhoto, ScanControl, ScanEvent};
use crate::{db, thumbnail};
use anyhow::{Context, Result};
use gio::prelude::*;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

const BATCH_SIZE: usize = 128;
const FLUSH_INTERVAL: Duration = Duration::from_millis(200);
const QUEUE_SIZE: usize = 1024;

#[derive(Clone)]
struct Record {
    path: String,
    name: String,
    is_dir: bool,
    mtime: Option<i64>,
    size: Option<i64>,
}

trait Source: Send + Sync {
    fn available(&self, root: &str) -> bool;
    fn visit(
        &self,
        directory: &str,
        control: &ScanControl,
        emit: &mut dyn FnMut(Record) -> bool,
    ) -> Result<()>;
    fn metadata(&self, work: &db::NetworkWork) -> Result<db::PhotoMetadata>;
    /// True means a preview exists, false means the remote format is unsupported.
    fn thumbnail(&self, work: &db::NetworkWork) -> Result<bool>;
}

struct NetworkSource;
impl Source for NetworkSource {
    fn available(&self, root: &str) -> bool {
        crate::network_shares::stat(root).is_ok_and(|meta| meta.is_dir)
    }
    fn visit(
        &self,
        directory: &str,
        control: &ScanControl,
        emit: &mut dyn FnMut(Record) -> bool,
    ) -> Result<()> {
        let mut error = None;
        crate::network_shares::visit_scan(directory, &mut |record| {
            if control.is_cancelled() {
                return false;
            }
            let name = record.entry.name.to_ascii_lowercase();
            if record.entry.is_dir
                && (name.ends_with(".lrdata") || name == "previews" || name == "cache")
            {
                return true;
            }
            let metadata = if record.entry.is_dir {
                record.metadata
            } else {
                match record
                    .metadata
                    .map(Ok)
                    .unwrap_or_else(|| crate::network_shares::stat(&record.entry.uri))
                {
                    Ok(metadata) => Some(metadata),
                    Err(failure) => {
                        error = Some(failure);
                        return false;
                    }
                }
            };
            emit(Record {
                path: record.entry.uri,
                name: record.entry.name,
                is_dir: record.entry.is_dir,
                mtime: metadata.as_ref().and_then(|meta| meta.mtime),
                size: metadata.and_then(|meta| i64::try_from(meta.size).ok()),
            })
        })?;
        if let Some(error) = error {
            return Err(error);
        }
        Ok(())
    }
    fn metadata(&self, work: &db::NetworkWork) -> Result<db::PhotoMetadata> {
        let info = gio::FileInfo::new();
        info.set_size(work.size.unwrap_or_default());
        if let Some(mtime) = work.mtime.filter(|value| *value >= 0) {
            info.set_attribute_uint64("time::modified", mtime as u64);
        }
        super::read_metadata(&work.path, &info)
    }
    fn thumbnail(&self, work: &db::NetworkWork) -> Result<bool> {
        if super::remote_raw_thumbnail_unsupported(&work.path) {
            return Ok(false);
        }
        let preview = thumbnail::create(&work.path, work.mtime, work.size)?;
        anyhow::ensure!(
            preview.is_file(),
            "preview is unavailable or still being generated"
        );
        Ok(true)
    }
}

// Native discovery sessions are thread-owned, and must be released on every
// exit path, including a canceled callback or a panic in the worker.
struct DiscoverySession;
impl Drop for DiscoverySession {
    fn drop(&mut self) {
        crate::private_nfs::close_scan_session();
        crate::private_smb::close_scan_session();
    }
}
struct Shutdown(ScanControl);
impl Drop for Shutdown {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

enum Discovery {
    Begin {
        path: String,
        parent: Option<String>,
    },
    Entry {
        folder: String,
        record: Record,
    },
    End {
        path: String,
        error: Option<String>,
    },
    Done,
    Fatal(String),
}
struct Enriched {
    work: db::NetworkWork,
    metadata: Option<db::PhotoMetadata>,
    metadata_state: String,
    thumbnail_state: String,
    errors: Vec<String>,
}
enum Enrichment {
    Offline,
    Photo(Enriched),
    Done,
    Fatal(String),
}

fn send_bounded<T>(sender: &SyncSender<T>, mut value: T, control: &ScanControl) -> bool {
    loop {
        if control.is_cancelled() {
            return false;
        }
        match sender.try_send(value) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(returned)) => {
                value = returned;
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
fn acknowledged(receiver: &Receiver<()>, control: &ScanControl) -> bool {
    loop {
        if control.is_cancelled() {
            return false;
        }
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(()) => return true,
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn discover(
    root: &str,
    database: &Path,
    source: &dyn Source,
    control: &ScanControl,
    sender: &SyncSender<Discovery>,
    ack: &Receiver<()>,
) -> Result<()> {
    let _session = DiscoverySession;
    let connection = db::open_existing(database)?;
    while let Some((path, parent)) = db::next_network_directory(&connection, root)? {
        if !send_bounded(
            sender,
            Discovery::Begin {
                path: path.clone(),
                parent,
            },
            control,
        ) {
            return Ok(());
        }
        let result = source.visit(&path, control, &mut |record| {
            send_bounded(
                sender,
                Discovery::Entry {
                    folder: path.clone(),
                    record,
                },
                control,
            )
        });
        if !send_bounded(
            sender,
            Discovery::End {
                path,
                error: result.err().map(|error| error.to_string()),
            },
            control,
        ) {
            return Ok(());
        }
        // All preceding records must commit before this checkpoint becomes
        // eligible for recovery or before selecting the next child directory.
        if !acknowledged(ack, control) {
            return Ok(());
        }
    }
    send_bounded(sender, Discovery::Done, control);
    Ok(())
}

fn enrich(
    root: &str,
    generation: i64,
    database: &Path,
    source: &dyn Source,
    control: &ScanControl,
    discovery_done: &AtomicBool,
    sender: &SyncSender<Enrichment>,
    ack: &Receiver<()>,
) -> Result<()> {
    let _read_cancellation = crate::private_nfs::cancel_scan_reads(control);
    let connection = db::open_existing(database)?;
    while !control.is_cancelled() {
        // Observe completion before querying SQLite: observing it afterwards
        // could race the writer's final commit and strand the final work item.
        let completed = discovery_done.load(Ordering::Acquire);
        let Some(work) = db::next_network_work(&connection, root, generation)? else {
            if completed {
                send_bounded(sender, Enrichment::Done, control);
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        while thumbnail::priority_pending_count() > 0 && !control.is_cancelled() {
            std::thread::sleep(Duration::from_millis(10));
        }
        if control.is_cancelled() {
            break;
        }
        let mut result = Enriched {
            work: work.clone(),
            metadata: None,
            metadata_state: work.metadata_state.clone(),
            thumbnail_state: work.thumbnail_state.clone(),
            errors: Vec::new(),
        };
        if work.metadata_pending {
            match source.metadata(&work) {
                Ok(metadata) => {
                    result.metadata = Some(metadata);
                    result.metadata_state = "ready".into();
                }
                Err(error) => {
                    if control.is_cancelled() {
                        break;
                    }
                    if !source.available(root) {
                        send_bounded(sender, Enrichment::Offline, control);
                        return Ok(());
                    }
                    result.metadata_state = "failed".into();
                    result.errors.push(format!("metadata: {error}"));
                }
            }
        }
        if work.thumbnail_pending && !control.is_cancelled() {
            // Let newly visible photos run between a header read and decoding.
            while thumbnail::priority_pending_count() > 0 && !control.is_cancelled() {
                std::thread::sleep(Duration::from_millis(10));
            }
            if control.is_cancelled() {
                break;
            }
            match source.thumbnail(&work) {
                Ok(true) => result.thumbnail_state = "ready".into(),
                Ok(false) => result.thumbnail_state = "unsupported".into(),
                Err(error) => {
                    if control.is_cancelled() {
                        break;
                    }
                    if !source.available(root) {
                        send_bounded(sender, Enrichment::Offline, control);
                        return Ok(());
                    }
                    result.thumbnail_state = "failed".into();
                    result.errors.push(format!("thumbnail: {error}"));
                }
            }
        }
        if !send_bounded(sender, Enrichment::Photo(result), control) || !acknowledged(ack, control)
        {
            break;
        }
    }
    Ok(())
}

pub(super) fn scan(
    root: &str,
    database: &Path,
    events: Option<&mpsc::Sender<ScanEvent>>,
    control: &ScanControl,
    resume: bool,
) -> Result<usize> {
    run_with_source(
        root,
        database,
        events,
        control,
        resume,
        Arc::new(NetworkSource),
    )
}

fn run_with_source(
    root: &str,
    database: &Path,
    events: Option<&mpsc::Sender<ScanEvent>>,
    control: &ScanControl,
    resume: bool,
    source: Arc<dyn Source>,
) -> Result<usize> {
    if control.is_cancelled() {
        send(events, ScanEvent::Cancelled { imported: 0 });
        return Ok(0);
    }
    let connection = db::open_existing(database)?;
    let folder_id = db::insert_folder(&connection, root)?;
    let job = db::start_network_job(&connection, root, resume)?;
    let available = source.available(root);
    if control.is_cancelled() {
        if !control.should_resume() {
            db::pause_network_import(&connection, root)?;
        } else {
            connection.execute("UPDATE network_import_jobs SET status='waiting' WHERE root=?1 AND status!='paused'",[root])?;
        }
        send(events, ScanEvent::Cancelled { imported: 0 });
        return Ok(0);
    }
    if !available {
        connection.execute(
            "UPDATE network_import_jobs SET status='waiting' WHERE root=?1 AND status!='paused'",
            [root],
        )?;
        send(events,ScanEvent::Failed {path:PathBuf::from(root),error:"Network share is unavailable; the import will continue when PIC reopens or you add the folder again".into()});
        send(
            events,
            ScanEvent::Finished {
                imported: 0,
                failed: 1,
            },
        );
        return Ok(0);
    }
    if let Some(folder) = db::folders(&connection)?
        .into_iter()
        .find(|folder| folder.id == folder_id)
    {
        send(events, ScanEvent::FolderStarted { folder });
    }
    let (discovery_sender, discovery_receiver) = mpsc::sync_channel(QUEUE_SIZE);
    let (discovery_ack, discovery_ack_receiver) = mpsc::sync_channel(1);
    let (enrichment_sender, enrichment_receiver) = mpsc::sync_channel(1);
    let (enrichment_ack, enrichment_ack_receiver) = mpsc::sync_channel(1);
    let worker_control = control.child();
    let discovery_done = AtomicBool::new(false);
    let mut imported = connection.query_row(
        "SELECT added FROM network_import_jobs WHERE root=?1",
        [root],
        |row| row.get::<_, usize>(0),
    )?;
    let mut failed = 0usize;
    let mut found = connection.query_row(
        "SELECT COUNT(*) FROM network_import_seen WHERE root=?1 AND kind='photo'",
        [root],
        |row| row.get::<_, usize>(0),
    )?;
    let mut counts = db::sidebar_counts(&connection)?;
    let recently_added_limit = db::recently_added_limit(&connection) as i64;
    let (mut metadata_ready,mut previews_ready)=connection.query_row(
        "SELECT COALESCE(SUM(w.metadata_state='ready'),0),COALESCE(SUM(w.thumbnail_state='ready'),0)
         FROM network_photo_work w JOIN network_import_seen s ON s.root=w.root AND s.path=w.path WHERE w.root=?1 AND w.generation=?2",
        params![root,job.generation],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?)))?;
    let mut suspended = false;
    let result = std::thread::scope(|scope| -> Result<()> {
        let _shutdown = Shutdown(worker_control.clone());
        let discovery_source = source.clone();
        let discovery_control = &worker_control;
        scope.spawn(move || {
            if let Err(error) = discover(
                root,
                database,
                discovery_source.as_ref(),
                discovery_control,
                &discovery_sender,
                &discovery_ack_receiver,
            ) {
                send_bounded(
                    &discovery_sender,
                    Discovery::Fatal(error.to_string()),
                    discovery_control,
                );
            }
        });
        let enrichment_source = source.clone();
        let enrichment_control = &worker_control;
        let done = &discovery_done;
        scope.spawn(move || {
            if let Err(error) = enrich(
                root,
                job.generation,
                database,
                enrichment_source.as_ref(),
                enrichment_control,
                done,
                &enrichment_sender,
                &enrichment_ack_receiver,
            ) {
                send_bounded(
                    &enrichment_sender,
                    Enrichment::Fatal(error.to_string()),
                    enrichment_control,
                );
            }
        });
        let mut batch = Vec::with_capacity(BATCH_SIZE);
        let mut batch_started = Instant::now();
        let mut last_progress = Instant::now() - Duration::from_secs(1);
        let mut catalog_complete = false;
        let mut enrichment_complete = false;
        loop {
            if control.is_cancelled() {
                break;
            }
            if let Ok(message) = enrichment_receiver.try_recv() {
                match message {
                    Enrichment::Photo(result) => {
                        let transaction = connection.unchecked_transaction()?;
                        let photo = db::finish_network_work(
                            &transaction,
                            &result.work,
                            result.metadata.as_ref(),
                            &result.metadata_state,
                            &result.thumbnail_state,
                        )?;
                        transaction.commit()?;
                        if let Some(photo) = photo {
                            if result.work.metadata_state != "ready"
                                && result.metadata_state == "ready"
                            {
                                metadata_ready += 1;
                            }
                            if result.work.thumbnail_state != "ready"
                                && result.thumbnail_state == "ready"
                            {
                                previews_ready += 1;
                            }
                            if result.metadata.is_some() {
                                send(
                                    events,
                                    ScanEvent::PhotosUpdated {
                                        photos: vec![photo],
                                    },
                                );
                            }
                            if result.work.thumbnail_pending && result.thumbnail_state == "ready" {
                                send(
                                    events,
                                    ScanEvent::ThumbnailCreated {
                                        path: PathBuf::from(&result.work.path),
                                    },
                                );
                            }
                            for error in result.errors {
                                failed += 1;
                                send(
                                    events,
                                    ScanEvent::Failed {
                                        path: PathBuf::from(&result.work.path),
                                        error,
                                    },
                                );
                            }
                        }
                        let _ = enrichment_ack.try_send(());
                    }
                    Enrichment::Offline => {
                        failed += 1;
                        send(events, ScanEvent::Failed { path: PathBuf::from(root), error: "Network share disconnected; committed photos are kept and the import will continue when PIC reopens or you add the folder again".into() });
                        suspended = true;
                        break;
                    }
                    Enrichment::Done => enrichment_complete = true,
                    Enrichment::Fatal(error) => anyhow::bail!("network enrichment: {error}"),
                }
            }
            match discovery_receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(Discovery::Begin { path, parent }) => {
                    flush(
                        &connection,
                        root,
                        job.generation,
                        &mut batch,
                        &mut imported,
                        &mut found,
                        &mut counts,
                        recently_added_limit,
                        &mut metadata_ready,
                        &mut previews_ready,
                        events,
                    )?;
                    let transaction = connection.unchecked_transaction()?;
                    db::catalog_network_directory(&transaction, root, &path, parent.as_deref())?;
                    transaction.commit()?;
                }
                Ok(Discovery::Entry { folder, record }) => batch.push((folder, record)),
                Ok(Discovery::End { path, error }) => {
                    flush(
                        &connection,
                        root,
                        job.generation,
                        &mut batch,
                        &mut imported,
                        &mut found,
                        &mut counts,
                        recently_added_limit,
                        &mut metadata_ready,
                        &mut previews_ready,
                        events,
                    )?;
                    let transaction = connection.unchecked_transaction()?;
                    transaction.execute(
                        "UPDATE network_import_dirs SET state=?3 WHERE root=?1 AND path=?2",
                        params![root, path, if error.is_some() { "failed" } else { "done" }],
                    )?;
                    transaction.execute("UPDATE folders SET raw_jpeg_pair_count=(SELECT COUNT(*) FROM network_import_pairs WHERE root=?1 AND folder=?2 AND raw=1 AND jpeg=1) WHERE path=?2",params![root,path])?;
                    transaction.commit()?;
                    if let Some(error) = error {
                        failed += 1;
                        send(
                            events,
                            ScanEvent::Failed {
                                path: PathBuf::from(path),
                                error,
                            },
                        );
                    }
                    let _ = discovery_ack.try_send(());
                }
                Ok(Discovery::Done) => {
                    flush(
                        &connection,
                        root,
                        job.generation,
                        &mut batch,
                        &mut imported,
                        &mut found,
                        &mut counts,
                        recently_added_limit,
                        &mut metadata_ready,
                        &mut previews_ready,
                        events,
                    )?;
                    catalog_complete = true;
                    discovery_done.store(true, Ordering::Release);
                    send(events, ScanEvent::IndexingFinished { imported });
                }
                Ok(Discovery::Fatal(error)) => anyhow::bail!("network discovery: {error}"),
                Err(mpsc::RecvTimeoutError::Disconnected)
                    if !catalog_complete && !control.is_cancelled() =>
                {
                    anyhow::bail!("network discovery worker stopped unexpectedly")
                }
                Err(_) => {
                    if catalog_complete {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
            if batch.len() >= BATCH_SIZE
                || (!batch.is_empty() && batch_started.elapsed() >= FLUSH_INTERVAL)
            {
                flush(
                    &connection,
                    root,
                    job.generation,
                    &mut batch,
                    &mut imported,
                    &mut found,
                    &mut counts,
                    recently_added_limit,
                    &mut metadata_ready,
                    &mut previews_ready,
                    events,
                )?;
                batch_started = Instant::now();
            }
            if last_progress.elapsed() >= Duration::from_millis(500)
                || (catalog_complete && enrichment_complete)
            {
                send(
                    events,
                    ScanEvent::NetworkProgress {
                        found,
                        added: imported,
                        metadata_ready: metadata_ready.max(0) as usize,
                        previews_ready: previews_ready.max(0) as usize,
                        catalog_complete,
                    },
                );
                last_progress = Instant::now();
            }
            if catalog_complete && enrichment_complete {
                break;
            }
        }
        // Stop workers before the final durable flush, rather than allowing
        // discovery or enrichment to start another read while it commits.
        worker_control.cancel();
        // Even Stop preserves records already emitted by discovery. The
        // current directory stays pending, so recovery can safely enumerate it.
        flush(
            &connection,
            root,
            job.generation,
            &mut batch,
            &mut imported,
            &mut found,
            &mut counts,
            recently_added_limit,
            &mut metadata_ready,
            &mut previews_ready,
            events,
        )?;
        Ok(())
    });
    if let Err(error) = result {
        connection.execute(
            "UPDATE network_import_jobs SET status='waiting' WHERE root=?1 AND status!='paused'",
            [root],
        )?;
        return Err(error);
    }
    if control.is_cancelled() {
        if control.should_resume() {
            connection.execute("UPDATE network_import_jobs SET status='waiting' WHERE root=?1 AND status!='paused'",[root])?;
        } else {
            db::pause_network_import(&connection, root)?;
        }
        send(events, ScanEvent::Cancelled { imported });
        return Ok(imported);
    }
    let incomplete: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM network_import_dirs WHERE root=?1 AND state!='done')",
        [root],
        |row| row.get(0),
    )?;
    let available = source.available(root);
    if control.is_cancelled() {
        if !control.should_resume() {
            db::pause_network_import(&connection, root)?;
        }
        send(events, ScanEvent::Cancelled { imported });
        return Ok(imported);
    }
    if !incomplete && available && !suspended && !job.recovered {
        reconcile(&connection, root, folder_id, events, control)?;
    }
    connection.execute(
        "UPDATE network_import_jobs SET status=?2 WHERE root=?1 AND status!='paused'",
        params![
            root,
            if control.is_cancelled() && !control.should_resume() {
                "paused"
            } else if incomplete || !available || suspended || control.should_resume() {
                "waiting"
            } else {
                "complete"
            }
        ],
    )?;
    if control.is_cancelled() {
        if !control.should_resume() {
            db::pause_network_import(&connection, root)?;
        }
        send(events, ScanEvent::Cancelled { imported });
        return Ok(imported);
    }
    send(
        events,
        ScanEvent::LibraryCountsChanged {
            counts: db::sidebar_counts(&connection)?,
        },
    );
    send(events, ScanEvent::Finished { imported, failed });
    Ok(imported)
}

#[allow(clippy::too_many_arguments)]
fn flush(
    connection: &Connection,
    root: &str,
    generation: i64,
    batch: &mut Vec<(String, Record)>,
    imported: &mut usize,
    found: &mut usize,
    counts: &mut db::SidebarCounts,
    recently_added_limit: i64,
    metadata_ready: &mut i64,
    previews_ready: &mut i64,
    events: Option<&mpsc::Sender<ScanEvent>>,
) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let transaction = connection.unchecked_transaction()?;
    let mut photos = Vec::new();
    let mut folder_ids = Vec::new();
    for (folder, record) in batch.drain(..) {
        if record.is_dir {
            let existing: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM folders WHERE path=?1)",
                [&record.path],
                |row| row.get(0),
            )?;
            let id =
                db::catalog_network_directory(&transaction, root, &record.path, Some(&folder))?;
            if !existing {
                folder_ids.push(id);
            }
            continue;
        }
        let seen: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM network_import_seen WHERE root=?1 AND path=?2)",
            params![root, record.path],
            |row| row.get(0),
        )?;
        let folder_id =
            transaction.query_row("SELECT id FROM folders WHERE path=?1", [&folder], |row| {
                row.get(0)
            })?;
        let row = db::catalog_network_photo(
            &transaction,
            root,
            generation,
            &record.path,
            folder_id,
            record.mtime,
            record.size,
        )?;
        *metadata_ready += row.metadata_delta;
        *previews_ready += row.preview_delta;
        if !seen {
            *found += 1;
        }
        if row.newly_discovered {
            *imported += 1;
            if !row.photo.trashed {
                counts.photos += 1;
                counts.recently_added = counts.photos.min(recently_added_limit);
            }
            photos.push(IndexedPhoto {
                photo: row.photo,
                newly_discovered: true,
            });
        }
        let raw = crate::image_format::uses(&record.name, crate::image_format::DecoderKind::Raw);
        let jpeg =
            crate::image_format::for_path(&record.name).is_some_and(|format| format.id == "jpeg");
        if raw || jpeg {
            let stem = Path::new(&record.name)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            transaction.execute("INSERT INTO network_import_pairs(root,folder,stem,raw,jpeg) VALUES (?1,?2,?3,?4,?5)
                ON CONFLICT(root,folder,stem) DO UPDATE SET raw=MAX(raw,?4),jpeg=MAX(jpeg,?5)",params![root,folder,stem,raw,jpeg])?;
        }
    }
    transaction.commit()?;
    if !folder_ids.is_empty() {
        for id in folder_ids {
            let folder=connection.query_row("SELECT id,path,COALESCE(name,path),parent_id,imported_root,watched FROM folders WHERE id=?1",[id],|row|Ok(db::Folder {
                id:row.get(0)?,path:row.get(1)?,name:row.get(2)?,parent_id:row.get(3)?,imported_root:row.get(4)?,watched:row.get(5)?,photo_count:0,subfolder_count:0,available:true,
            }))?;
            send(events, ScanEvent::FolderStarted { folder });
        }
    }
    if !photos.is_empty() {
        send(
            events,
            ScanEvent::PhotosIndexed {
                photos,
                counts: *counts,
            },
        );
    }
    Ok(())
}

fn reconcile(
    connection: &Connection,
    root: &str,
    folder_id: i64,
    events: Option<&mpsc::Sender<ScanEvent>>,
    control: &ScanControl,
) -> Result<()> {
    // Seen paths live in SQLite rather than retaining the full tree in Rust.
    // Bounded deletes keep normal editing writes responsive.
    loop {
        if control.is_cancelled() {
            return Ok(());
        }
        let ids=connection.prepare("WITH RECURSIVE descendants(id) AS (
            SELECT ?1 UNION ALL SELECT f.id FROM folders f JOIN descendants d ON f.parent_id=d.id)
            SELECT p.id FROM photos p WHERE p.folder_id IN descendants
            AND NOT EXISTS(SELECT 1 FROM network_import_seen s WHERE s.root=?2 AND s.path=p.path AND s.kind='photo') LIMIT 128")?
            .query_map(params![folder_id,root],|row|row.get::<_,i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if ids.is_empty() {
            break;
        }
        let transaction = connection.unchecked_transaction()?;
        for id in &ids {
            transaction.execute("DELETE FROM photos WHERE id=?1", [id])?;
        }
        transaction.commit()?;
        send(events, ScanEvent::PhotosRemoved { ids });
    }
    let mut removed_folders = false;
    loop {
        if control.is_cancelled() {
            return Ok(());
        }
        let ids=connection.prepare("WITH RECURSIVE descendants(id) AS (
            SELECT ?1 UNION ALL SELECT f.id FROM folders f JOIN descendants d ON f.parent_id=d.id)
            SELECT f.id FROM folders f WHERE f.id IN descendants AND f.id!=?1
            AND NOT EXISTS(SELECT 1 FROM folders child WHERE child.parent_id=f.id)
            AND NOT EXISTS(SELECT 1 FROM network_import_seen s WHERE s.root=?2 AND s.path=f.path AND s.kind='folder') LIMIT 128")?
            .query_map(params![folder_id,root],|row|row.get::<_,i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if ids.is_empty() {
            break;
        }
        let transaction = connection.unchecked_transaction()?;
        for id in ids {
            transaction.execute("DELETE FROM folders WHERE id=?1", [id])?;
        }
        transaction.commit()?;
        removed_folders = true;
    }
    if removed_folders {
        send(events, ScanEvent::FoldersRemoved);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
