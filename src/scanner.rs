use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use chrono::{Local, TimeZone};
use exif::{In, Reader as ExifReader, Tag, Value};
use gio::prelude::*;
use rusqlite::Connection;

use crate::db::{self, PhotoMetadata};
use crate::thumbnail;

#[derive(Debug, Clone)]
pub enum ScanEvent {
    Started {
        root: PathBuf,
    },
    FolderStarted {
        folder: db::Folder,
    },
    DiscoveryProgress {
        found: usize,
    },
    PhotosIndexed {
        photos: Vec<IndexedPhoto>,
        counts: db::SidebarCounts,
    },
    LibraryCountsChanged {
        counts: db::SidebarCounts,
    },
    IndexingFinished {
        imported: usize,
    },
    ThumbnailsStarted {
        total: usize,
    },
    ThumbnailsDeferred {
        total: usize,
    },
    ThumbnailCreated {
        path: PathBuf,
    },
    Failed {
        path: PathBuf,
        error: String,
    },
    Finished {
        imported: usize,
        failed: usize,
    },
    Cancelled {
        imported: usize,
    },
}

#[derive(Debug, Clone)]
pub struct IndexedPhoto {
    pub photo: db::Photo,
    pub newly_discovered: bool,
}

/// Cooperative cancellation handle for an import and its thumbnail pass.
#[derive(Clone, Default)]
pub struct ScanControl(Arc<AtomicBool>);

impl ScanControl {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

// Refresh may enqueue several stored folders at once.  Scans share the same
// SQLite database and must therefore run one at a time; otherwise concurrent
// transactions can make all but one scan fail before thumbnails are created.
static SCAN_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn supported(path: &Path) -> bool {
    crate::image_format::supported(path)
}

pub fn scan(root: &str, events: Option<&Sender<ScanEvent>>) -> Result<usize> {
    let database = db::active_database_path()?;
    scan_with_control(root, &database, events, &ScanControl::default())
}

fn scan_with_control(
    root: &str,
    database: &Path,
    events: Option<&Sender<ScanEvent>>,
    control: &ScanControl,
) -> Result<usize> {
    if !root_is_available(root) {
        anyhow::bail!("scan root is unavailable: {root}");
    }
    let connection = db::open_existing(database)?;
    let folder_id = db::insert_folder(&connection, root)?;
    let folder = db::folders(&connection)?
        .into_iter()
        .find(|folder| folder.id == folder_id);
    if let Some(folder) = folder {
        send(events, ScanEvent::FolderStarted { folder });
    }
    let indexed = db::photo_fingerprints(&connection)?;
    let root_file = crate::source::file(root);
    let (files, discovered_folders) = collect_files(&root_file, events, control)?;
    let raw_jpeg_pair_counts = raw_jpeg_pair_counts(&files, &discovered_folders);
    if control.is_cancelled() {
        send(events, ScanEvent::Cancelled { imported: 0 });
        return Ok(0);
    }

    // Only reconcile deletions after the complete tree was enumerated and the
    // root is still available. If a removable drive went offline, discovery
    // fails or this second check fails, so existing library records survive.
    if !root_is_available(root) {
        anyhow::bail!("scan root became unavailable: {root}");
    }
    let present_paths = files
        .iter()
        .map(|(file, _, _)| crate::source::reference(file))
        .collect::<HashSet<_>>();
    let removed = db::remove_missing_photos(&connection, folder_id, &present_paths)?;
    if removed > 0 {
        send_library_counts(events, &connection);
    }

    let mut imported = 0;
    let mut failed = 0;
    let mut thumbnails = Vec::new();
    let mut indexed_photos = Vec::new();
    let mut folder_ids = HashMap::from([(root.to_string(), folder_id)]);
    // Folder reconciliation is SQL-only and committed in bounded batches.
    // In particular, no network enumeration or metadata read occurs while a
    // write transaction is active.
    for chunk in discovered_folders.chunks(128) {
        let transaction = connection.unchecked_transaction()?;
        for (path, parent_path) in chunk {
            if path == root {
                continue;
            }
            let parent_id = parent_path
                .as_ref()
                .and_then(|parent| folder_ids.get(parent))
                .copied()
                .unwrap_or(folder_id);
            let id = db::insert_discovered_folder(&transaction, path, parent_id)?;
            folder_ids.insert(path.clone(), id);
        }
        transaction.commit()?;
    }
    // Pair detection reuses the discovery result that is already in memory.
    // Header visibility therefore never triggers a filesystem/network rescan.
    db::set_raw_jpeg_pair_counts(&connection, &raw_jpeg_pair_counts)?;

    let mut prepared = Vec::with_capacity(32);
    for (file, info, folder_path) in files {
        if control.is_cancelled() {
            break;
        }
        let path = crate::source::reference(&file);
        let folder_id = if let Some(folder_id) = folder_ids.get(&folder_path) {
            *folder_id
        } else {
            let parent_path = crate::source::file(&folder_path)
                .parent()
                .map(|parent| crate::source::reference(&parent));
            let parent_id = parent_path
                .as_ref()
                .and_then(|parent| folder_ids.get(parent))
                .copied()
                .unwrap_or(folder_id);
            // Defensive fallback for a source enumerator that omitted a
            // folder record. This is one short statement, never source I/O.
            let folder_id = db::insert_discovered_folder(&connection, &folder_path, parent_id)?;
            folder_ids.insert(folder_path.clone(), folder_id);
            folder_id
        };
        let fingerprint = (
            info.modification_date_time().map(|time| time.to_unix()),
            Some(info.size()),
        );
        let existing = indexed.get(&path);
        let fingerprint_matches =
            existing.is_some_and(|(mtime, size, _, _, _, _)| (*mtime, *size) == fingerprint);
        let missing_raw_dimensions = is_raw(&path)
            && !remote_raw_thumbnail_unsupported(&path)
            && existing.is_some_and(|(_, _, width, height, _, _)| {
                width.unwrap_or_default() <= 0 || height.unwrap_or_default() <= 0
            });
        // NULL means the row predates aperture indexing. A zero is the stored
        // "metadata examined but absent" sentinel and must not re-trigger a
        // source read on every refresh.
        let missing_aperture_metadata =
            existing.is_some_and(|(_, _, _, _, aperture, _)| aperture.is_none());
        // NULL ISO marks rows that predate the expanded EXIF index. Zero means
        // the source was examined and did not contain an ISO value.
        let missing_exif_metadata = existing.is_some_and(|(_, _, _, _, _, iso)| iso.is_none());
        let missing_heif_thumbnail = is_heif(&path)
            && existing.is_some()
            && thumbnail::existing_cache_path(&path, fingerprint.0, fingerprint.1)
                .ok()
                .flatten()
                .is_none();
        let missing_raw_thumbnail = is_raw(&path)
            && !remote_raw_thumbnail_unsupported(&path)
            && existing.is_some()
            && thumbnail::existing_cache_path(&path, fingerprint.0, fingerprint.1)
                .ok()
                .flatten()
                .is_none();
        if fingerprint_matches
            && !missing_raw_dimensions
            && !missing_aperture_metadata
            && !missing_exif_metadata
            && !missing_heif_thumbnail
            && !missing_raw_thumbnail
        {
            prepared.push(PreparedPhoto::Existing { path, folder_id });
        } else {
            let newly_discovered = existing.is_none();
            let create_thumbnail =
                (!fingerprint_matches || missing_heif_thumbnail || missing_raw_thumbnail)
                    && !remote_raw_thumbnail_unsupported(&path);
            // This may read a remote original and decode EXIF/RAW metadata.
            // It must remain outside the transaction below.
            match read_metadata(&path, &info) {
                Ok(metadata) => prepared.push(PreparedPhoto::Upsert {
                    path,
                    folder_id,
                    metadata,
                    newly_discovered,
                    create_thumbnail,
                }),
                Err(error) => {
                    failed += 1;
                    send(
                        events,
                        ScanEvent::Failed {
                            path: PathBuf::from(path),
                            error: error.to_string(),
                        },
                    );
                }
            }
        }
        if prepared.len() == 32 {
            let counts = commit_prepared(
                &connection,
                &mut prepared,
                &mut imported,
                &mut thumbnails,
                &mut indexed_photos,
            )?;
            if !indexed_photos.is_empty() {
                send(
                    events,
                    ScanEvent::PhotosIndexed {
                        photos: std::mem::take(&mut indexed_photos),
                        counts: counts.expect("indexed batch has a count snapshot"),
                    },
                );
            }
        }
    }
    let counts = commit_prepared(
        &connection,
        &mut prepared,
        &mut imported,
        &mut thumbnails,
        &mut indexed_photos,
    )?;
    if !indexed_photos.is_empty() {
        send(
            events,
            ScanEvent::PhotosIndexed {
                photos: std::mem::take(&mut indexed_photos),
                counts: counts.expect("indexed batch has a count snapshot"),
            },
        );
    }
    if control.is_cancelled() {
        send(events, ScanEvent::Cancelled { imported });
        return Ok(imported);
    }
    send_library_counts(events, &connection);
    send(events, ScanEvent::IndexingFinished { imported });
    send(
        events,
        ScanEvent::ThumbnailsStarted {
            total: thumbnails.len(),
        },
    );
    // Do not hold the import worker open for thumbnail generation. This worker
    // continues independently while indexing and browsing remain available.
    if let Some(sender) = events.cloned() {
        let control = control.clone();
        std::thread::spawn(move || {
            let progress_sender = sender.clone();
            let thumbnail_results = thumbnail::create_many_cancellable(
                &thumbnails,
                || control.is_cancelled(),
                |path| {
                    let _ = progress_sender.send(ScanEvent::ThumbnailCreated {
                        path: PathBuf::from(path),
                    });
                },
            );
            let mut thumbnail_failed = 0;
            for ((path, _mtime, _size_bytes), result) in
                thumbnails.into_iter().zip(thumbnail_results)
            {
                if let Some(Err(error)) = result {
                    thumbnail_failed += 1;
                    let _ = sender.send(ScanEvent::Failed {
                        path: PathBuf::from(path),
                        error: format!("thumbnail: {error}"),
                    });
                }
            }
            if control.is_cancelled() {
                let _ = sender.send(ScanEvent::Cancelled { imported });
            } else {
                let _ = sender.send(ScanEvent::Finished {
                    imported,
                    failed: failed + thumbnail_failed,
                });
            }
        });
    } else {
        send(events, ScanEvent::Finished { imported, failed });
    }
    Ok(imported)
}

enum PreparedPhoto {
    Existing {
        path: String,
        folder_id: i64,
    },
    Upsert {
        path: String,
        folder_id: i64,
        metadata: PhotoMetadata,
        newly_discovered: bool,
        create_thumbnail: bool,
    },
}

/// Commit already-prepared rows. This function intentionally performs no
/// filesystem access or image decoding while the write lock is held.
fn commit_prepared(
    connection: &Connection,
    prepared: &mut Vec<PreparedPhoto>,
    imported: &mut usize,
    thumbnails: &mut Vec<(String, Option<i64>, Option<i64>)>,
    indexed_photos: &mut Vec<IndexedPhoto>,
) -> Result<Option<db::SidebarCounts>> {
    if prepared.is_empty() {
        return Ok(None);
    }
    let transaction = connection.unchecked_transaction()?;
    for item in prepared.drain(..) {
        match item {
            PreparedPhoto::Existing { path, folder_id } => {
                db::set_photo_folder(&transaction, &path, folder_id)?;
            }
            PreparedPhoto::Upsert {
                path,
                folder_id,
                metadata,
                newly_discovered,
                create_thumbnail,
            } => {
                let id =
                    db::upsert_photo(&transaction, Path::new(&path), Some(folder_id), &metadata)?;
                let photo = db::photo(&transaction, id)?.context("indexed photo disappeared")?;
                *imported += 1;
                if create_thumbnail {
                    if std::env::var_os("PICASA_TRACE").is_some() {
                        eprintln!(
                            "PIC_THUMBNAIL schedule source=scan raw={} uri={path}",
                            is_raw(&path)
                        );
                    }
                    thumbnails.push((path.clone(), metadata.mtime, metadata.size_bytes));
                }
                indexed_photos.push(IndexedPhoto {
                    photo,
                    newly_discovered,
                });
            }
        }
    }
    let counts = (!indexed_photos.is_empty())
        .then(|| db::sidebar_counts(&transaction))
        .transpose()?;
    transaction.commit()?;
    Ok(counts)
}

fn root_is_available(root: &str) -> bool {
    #[cfg(target_os = "linux")]
    if crate::network_shares::private(root) {
        return crate::network_shares::stat(root)
            .map(|m| m.is_dir)
            .unwrap_or(false);
    }
    if root.contains("://") {
        crate::source::file(root).query_exists(gio::Cancellable::NONE)
    } else {
        Path::new(root).is_dir()
    }
}

pub fn spawn_scan(root: String, database: PathBuf, events: Sender<ScanEvent>) -> ScanControl {
    let control = ScanControl::default();
    let worker_control = control.clone();
    std::thread::spawn(move || {
        let lock = SCAN_LOCK.get_or_init(|| Mutex::new(()));
        let _guard = lock.lock().expect("scan lock poisoned");
        send(
            Some(&events),
            ScanEvent::Started {
                root: PathBuf::from(&root),
            },
        );
        if let Err(error) = scan_with_control(&root, &database, Some(&events), &worker_control) {
            send(
                Some(&events),
                ScanEvent::Failed {
                    path: PathBuf::from(root),
                    error: error.to_string(),
                },
            );
        }
    });
    control
}

fn send(events: Option<&Sender<ScanEvent>>, event: ScanEvent) {
    if let Some(events) = events {
        let _ = events.send(event);
    }
}

fn send_library_counts(events: Option<&Sender<ScanEvent>>, connection: &Connection) {
    if let Ok(counts) = db::sidebar_counts(connection) {
        send(events, ScanEvent::LibraryCountsChanged { counts });
    }
}

fn collect_files(
    root: &gio::File,
    events: Option<&Sender<ScanEvent>>,
    control: &ScanControl,
) -> Result<(
    Vec<(gio::File, gio::FileInfo, String)>,
    Vec<(String, Option<String>)>,
)> {
    let root_path = crate::source::reference(root);
    let mut pending = vec![(root.clone(), root_path.clone(), None)];
    let mut files = Vec::new();
    let mut folders = Vec::new();
    let mut last_reported = 0usize;
    let mut last_report = std::time::Instant::now();
    while let Some((directory, folder_path, parent_path)) = pending.pop() {
        if control.is_cancelled() {
            break;
        }

        folders.push((folder_path.clone(), parent_path));
        #[cfg(target_os = "linux")]
        if crate::network_shares::private(&folder_path) {
            for item in crate::network_shares::list(&folder_path)
                .with_context(|| format!("could not list {folder_path}"))?
            {
                if control.is_cancelled() {
                    break;
                }
                if item.is_dir {
                    let name = item.name.to_ascii_lowercase();
                    if !name.ends_with(".lrdata") && name != "previews" && name != "cache" {
                        let child = crate::source::file(&item.uri);
                        pending.push((child, item.uri, Some(folder_path.clone())));
                    }
                } else if supported(Path::new(&item.name)) {
                    let child = crate::source::file(&item.uri);
                    // Metadata is requested explicitly in the scanner; no originals
                    // are ever written to cache/source by a network scan.
                    let info = crate::network_shares::info(&item.uri)
                        .with_context(|| format!("could not stat {}", item.uri))?;
                    files.push((child, info, folder_path.clone()));
                    report_discovery_progress(
                        events,
                        files.len(),
                        &mut last_reported,
                        &mut last_report,
                    );
                }
            }
            continue;
        }
        let enumerator = directory
            .enumerate_children(
                "standard::name,standard::type,time::modified,standard::size",
                gio::FileQueryInfoFlags::NONE,
                gio::Cancellable::NONE,
            )
            .with_context(|| format!("could not list {}", directory.uri()))?;
        while let Some(info) = enumerator.next_file(gio::Cancellable::NONE)? {
            if control.is_cancelled() {
                break;
            }
            let child = enumerator.child(&info);
            match info.file_type() {
                gio::FileType::Directory => {
                    // Lightroom stores thousands of Smart Preview DNGs in
                    // *.lrdata folders. They are cache artifacts, not user
                    // photos; indexing them makes scrolling trigger a slow
                    // raw decode (several seconds per item).
                    let name = info.name().to_string_lossy().to_ascii_lowercase();
                    if !name.ends_with(".lrdata") && name != "previews" && name != "cache" {
                        pending.push((
                            child.clone(),
                            crate::source::reference(&child),
                            Some(folder_path.clone()),
                        ));
                    }
                }
                gio::FileType::Regular if supported(Path::new(&info.name())) => {
                    files.push((child, info, folder_path.clone()));
                    report_discovery_progress(
                        events,
                        files.len(),
                        &mut last_reported,
                        &mut last_report,
                    );
                }
                _ => {}
            }
        }
    }
    if files.len() != last_reported {
        send(events, ScanEvent::DiscoveryProgress { found: files.len() });
    }
    Ok((files, folders))
}

fn raw_jpeg_pair_counts(
    files: &[(gio::File, gio::FileInfo, String)],
    discovered_folders: &[(String, Option<String>)],
) -> HashMap<String, i64> {
    #[derive(Default)]
    struct PairState {
        raw: bool,
        jpeg: bool,
    }

    let mut counts = discovered_folders
        .iter()
        .map(|(path, _)| (path.clone(), 0_i64))
        .collect::<HashMap<_, _>>();
    let mut pairs = HashMap::<(String, String), PairState>::new();

    for (_, info, folder_path) in files {
        let name = info.name();
        let path = Path::new(&name);
        let Some(format) = crate::image_format::for_path(path) else {
            continue;
        };
        if format.decoder != crate::image_format::DecoderKind::Raw && format.id != "jpeg" {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if stem.is_empty() {
            continue;
        }
        let state = pairs.entry((folder_path.clone(), stem)).or_default();
        if format.decoder == crate::image_format::DecoderKind::Raw {
            state.raw = true;
        } else if format.id == "jpeg" {
            state.jpeg = true;
        }
    }

    for ((folder_path, _), state) in pairs {
        if state.raw && state.jpeg {
            *counts.entry(folder_path).or_insert(0) += 1;
        }
    }
    counts
}

fn report_discovery_progress(
    events: Option<&Sender<ScanEvent>>,
    found: usize,
    last_reported: &mut usize,
    last_report: &mut std::time::Instant,
) {
    if found.saturating_sub(*last_reported) >= 50
        || last_report.elapsed() >= std::time::Duration::from_millis(250)
    {
        send(events, ScanEvent::DiscoveryProgress { found });
        *last_reported = found;
        *last_report = std::time::Instant::now();
    }
}

fn read_metadata(path: &str, attributes: &gio::FileInfo) -> Result<PhotoMetadata> {
    let mtime = attributes
        .modification_date_time()
        .map(|time| time.to_unix());
    // Index network originals using directory/stat metadata only, without
    // eager full-size photo reads. Their thumbnails fetch bytes separately
    // using the bounded background queue. RAW stays metadata-only for now:
    // its path-only decoder must never trigger persistent original copying.
    #[cfg(target_os = "linux")]
    if crate::network_shares::private(path) {
        let taken_at = mtime.and_then(|seconds| {
            Local
                .timestamp_opt(seconds, 0)
                .single()
                .map(|date| date.to_rfc3339())
        });
        return Ok(PhotoMetadata {
            taken_at,
            size_bytes: Some(attributes.size()),
            mtime,
            ..Default::default()
        });
    }
    let (width, height, exif) = if is_raw(path) {
        // Prefer the cheap EXIF dimensions, then ask the RAW decoder for its
        // metadata-only image geometry. PixelX/YDimension are missing from
        // many DNG and NEF containers.
        let local = crate::source::materialize(path)?;
        let exif = fs::File::open(local).ok().and_then(|file| {
            ExifReader::new()
                .read_from_container(&mut BufReader::new(file))
                .ok()
        });
        let exif_width = exif
            .as_ref()
            .and_then(|data| exif_u32(data, Tag::PixelXDimension));
        let exif_height = exif
            .as_ref()
            .and_then(|data| exif_u32(data, Tag::PixelYDimension));
        let raw_dimensions = if exif_width.is_none() || exif_height.is_none() {
            crate::thumbnail::dimensions(path, &[]).ok()
        } else {
            None
        };
        let width = exif_width.or_else(|| raw_dimensions.map(|(width, _)| width));
        let height = exif_height.or_else(|| raw_dimensions.map(|(_, height)| height));
        (width, height, exif)
    } else {
        let bytes = crate::source::read(path)?;
        // image-rs does not decode every HEIF variant. Keep the record when
        // dimensions are unavailable; thumbnail generation will report a
        // per-file failure without aborting the rest of the scan.
        let dimensions = crate::thumbnail::dimensions(path, &bytes).ok();
        let exif = ExifReader::new()
            .read_from_container(&mut Cursor::new(&bytes))
            .ok();
        (
            dimensions.map(|(width, _)| width),
            dimensions.map(|(_, height)| height),
            exif,
        )
    };
    let taken_at = exif.as_ref().and_then(exif_date).or_else(|| {
        mtime.and_then(|seconds| {
            Local
                .timestamp_opt(seconds, 0)
                .single()
                .map(|date| date.to_rfc3339())
        })
    });
    let camera = exif.as_ref().and_then(|data| {
        let make = data.get_field(Tag::Make, In::PRIMARY).and_then(field_text);
        let model = data.get_field(Tag::Model, In::PRIMARY).and_then(field_text);
        match (make, model) {
            (Some(make), Some(model)) if model.starts_with(&make) => Some(model),
            (Some(make), Some(model)) => Some(format!("{make} {model}")),
            (Some(make), None) => Some(make),
            (None, Some(model)) => Some(model),
            _ => None,
        }
    });
    let aperture = exif.as_ref().and_then(exif_aperture);
    let lens = exif
        .as_ref()
        .and_then(|data| exif_text(data, Tag::LensModel));
    let shutter_speed = exif
        .as_ref()
        .and_then(|data| exif_rational(data, Tag::ExposureTime))
        .filter(|value| *value > 0.0);
    let iso = exif.as_ref().and_then(exif_iso);
    let focal_length = exif
        .as_ref()
        .and_then(|data| exif_rational(data, Tag::FocalLength))
        .filter(|value| *value > 0.0);
    let exposure_bias = exif
        .as_ref()
        .and_then(|data| exif_rational(data, Tag::ExposureBiasValue));
    // image-rs/rawler report stored sensor axes, while the viewer applies the
    // EXIF orientation to decoded pixels. Persist display-oriented dimensions
    // so the shared-element destination has the same aspect ratio before and
    // after a RAW/JPEG decode completes.
    let (width, height) = if exif
        .as_ref()
        .and_then(exif_orientation_value)
        .is_some_and(|orientation| matches!(orientation, 5..=8))
    {
        (height, width)
    } else {
        (width, height)
    };
    Ok(PhotoMetadata {
        taken_at,
        camera,
        aperture,
        lens,
        shutter_speed,
        iso,
        focal_length,
        exposure_bias,
        width: width.map(i64::from),
        height: height.map(i64::from),
        size_bytes: Some(attributes.size()),
        mtime,
    })
}

fn exif_aperture(exif: &exif::Exif) -> Option<f64> {
    // RAW cameras often store FNumber outside the primary IFD. Search all
    // parsed fields, then fall back to the equivalent APEX aperture value.
    let rational = |tag| {
        exif.fields()
            .find(|field| field.tag == tag)
            .and_then(|field| match &field.value {
                Value::Rational(values) => values.first().and_then(|value| {
                    (value.denom != 0).then_some(value.num as f64 / value.denom as f64)
                }),
                _ => None,
            })
    };
    rational(Tag::FNumber)
        .or_else(|| rational(Tag::ApertureValue).map(|value| 2_f64.powf(value / 2.0)))
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn exif_rational(exif: &exif::Exif, tag: Tag) -> Option<f64> {
    exif.fields()
        .find(|field| field.tag == tag)
        .and_then(|field| rational_value(&field.value))
}

fn rational_value(value: &Value) -> Option<f64> {
    match value {
        Value::Rational(values) => values
            .first()
            .and_then(|value| (value.denom != 0).then_some(value.num as f64 / value.denom as f64)),
        Value::SRational(values) => values
            .first()
            .and_then(|value| (value.denom != 0).then_some(value.num as f64 / value.denom as f64)),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

fn exif_text(exif: &exif::Exif, tag: Tag) -> Option<String> {
    exif.fields()
        .find(|field| field.tag == tag)
        .and_then(field_text)
}

fn exif_iso(exif: &exif::Exif) -> Option<i64> {
    // PhotographicSensitivity is the EXIF tag historically named ISOSpeedRatings.
    [Tag::PhotographicSensitivity, Tag::ISOSpeed]
        .into_iter()
        .find_map(|tag| exif.fields().find(|field| field.tag == tag))
        .and_then(|field| iso_value(&field.value))
}

fn iso_value(value: &Value) -> Option<i64> {
    match value {
        Value::Short(values) => values.first().map(|value| i64::from(*value)),
        Value::Long(values) => values.first().map(|value| i64::from(*value)),
        _ => None,
    }
    .filter(|value| *value > 0)
}

fn exif_orientation_value(exif: &exif::Exif) -> Option<u16> {
    exif.fields()
        .find(|field| field.tag == Tag::Orientation)
        .and_then(|field| match &field.value {
            Value::Short(values) => values.first().copied(),
            Value::Long(values) => values.first().copied().map(|value| value as u16),
            _ => None,
        })
        .filter(|orientation| (1..=8).contains(orientation))
}

fn exif_date(exif: &exif::Exif) -> Option<String> {
    exif.get_field(Tag::DateTimeOriginal, In::PRIMARY)
        .and_then(field_text)
        .or_else(|| {
            exif.get_field(Tag::DateTime, In::PRIMARY)
                .and_then(field_text)
        })
        .map(|date| date.replace(':', "-").replacen('-', "-", 2))
}

fn field_text(field: &exif::Field) -> Option<String> {
    match &field.value {
        Value::Ascii(values) => values
            .first()
            .map(|value| {
                String::from_utf8_lossy(value)
                    .trim_matches(|character: char| character == '\0' || character.is_whitespace())
                    .to_string()
            })
            .filter(|value| !value.is_empty()),
        _ => Some(field.display_value().to_string()),
    }
}

fn exif_u32(exif: &exif::Exif, tag: Tag) -> Option<u32> {
    exif.get_field(tag, In::PRIMARY)
        .and_then(|field| match &field.value {
            Value::Long(values) => values.first().copied(),
            Value::Short(values) => values.first().copied().map(u32::from),
            _ => None,
        })
}

fn remote_raw_thumbnail_unsupported(path: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        return is_raw(path)
            && crate::network_shares::private(path)
            && !crate::image_format::for_path(path).is_some_and(|format| format.id == "nikon_raw");
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        false
    }
}

fn is_raw(path: &str) -> bool {
    crate::image_format::uses(path, crate::image_format::DecoderKind::Raw)
}

fn is_heif(path: &str) -> bool {
    crate::image_format::uses(path, crate::image_format::DecoderKind::Heif)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exif_numeric_values_handle_unsigned_signed_and_missing() {
        assert_eq!(
            rational_value(&Value::Rational(vec![exif::Rational {
                num: 1,
                denom: 1000
            }])),
            Some(0.001)
        );
        assert_eq!(
            rational_value(&Value::SRational(vec![exif::SRational {
                num: -2,
                denom: 3
            }])),
            Some(-2.0 / 3.0)
        );
        assert_eq!(
            rational_value(&Value::Rational(vec![exif::Rational { num: 1, denom: 0 }])),
            None
        );
        assert_eq!(iso_value(&Value::Short(vec![3200])), Some(3200));
        assert_eq!(iso_value(&Value::Long(vec![12800])), Some(12800));
        assert_eq!(iso_value(&Value::Short(vec![0])), None);
    }
    use std::io::BufReader;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn recursive_collection_reaches_nested_photo_below_imported_root() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "picasa-rs-scanner-recursion-{}-{unique}",
            std::process::id()
        ));
        let nested = root.join("Marianne Lotter").join("FB-Marianne");
        fs::create_dir_all(&nested).unwrap();
        let photo = nested.join("photo.jpg");
        fs::write(&photo, []).unwrap();

        let (files, folders) =
            collect_files(&gio::File::for_path(&root), None, &ScanControl::default()).unwrap();
        assert!(files
            .iter()
            .any(|(_, _, folder)| folder.ends_with("FB-Marianne")));
        assert!(folders
            .iter()
            .any(|(folder, _)| folder.ends_with("FB-Marianne")));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn collection_reports_discovery_progress_in_bounded_batches() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "picasa-rs-scanner-progress-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        for index in 0..51 {
            fs::write(root.join(format!("photo-{index}.jpg")), []).unwrap();
        }
        let (sender, receiver) = std::sync::mpsc::channel();

        let (files, _) = collect_files(
            &gio::File::for_path(&root),
            Some(&sender),
            &ScanControl::default(),
        )
        .unwrap();
        drop(sender);
        let progress = receiver
            .into_iter()
            .filter_map(|event| match event {
                ScanEvent::DiscoveryProgress { found } => Some(found),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(files.len(), 51);
        assert!(progress.contains(&50));
        assert_eq!(progress.last(), Some(&51));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn configured_raw_fixture_exposes_aperture_and_orientation() {
        let Ok(path) = std::env::var("PICASA_TEST_RAW_METADATA") else {
            return;
        };
        let file = fs::File::open(path).unwrap();
        let exif = ExifReader::new()
            .read_from_container(&mut BufReader::new(file))
            .unwrap();
        assert!(exif_aperture(&exif).is_some_and(|value| value > 0.0));
        assert!(exif_orientation_value(&exif).is_some());
    }
}
