use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use anyhow::{Context, Result};
use fast_image_resize::{
    images::Image, pixels::PixelType, FilterType, ResizeAlg, ResizeOptions, Resizer,
};
use image::codecs::jpeg::JpegEncoder;
use image::ImageEncoder;
use image::{ColorType, DynamicImage, ImageReader};
use turbojpeg::{Decompressor, Image as TurboImage, PixelFormat, ScalingFactor};

const NORMAL_THUMBNAIL_SIZE: u32 = 320;
const HIGH_QUALITY_THUMBNAIL_SIZE: u32 = 640;
const THUMBNAIL_CACHE_VERSION: &[u8] = b"picasa-thumb-v4-heif-orientation";
const RAW_THUMBNAIL_CACHE_VERSION: &[u8] = b"picasa-thumb-v6-generic-raw";
const REMOTE_NEF_THUMBNAIL_CACHE_VERSION: &[u8] = b"picasa-thumb-v1-remote-nef-preview";
const REMOTE_JPEG_THUMBNAIL_CACHE_VERSION: &[u8] = b"picasa-thumb-v1-remote-jpeg-orientation";
const DNG_THUMBNAIL_CACHE_VERSION: &[u8] = b"picasa-thumb-v7-dng-full-raw";

// A folder import, startup recovery, and a manual refresh can overlap their
// thumbnail passes. Keep cache-key ownership separate from the filesystem
// existence check so two workers cannot generate the same preview together.
static IN_FLIGHT: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

type PriorityRequest = (
    String,
    Option<i64>,
    Option<i64>,
    PathBuf,
    std::time::Instant,
    Option<crate::thumbnail_display::DisplayRequest>,
);

type PriorityQueue = (Mutex<VecDeque<PriorityRequest>>, Condvar);

static PRIORITY_QUEUE: OnceLock<Arc<PriorityQueue>> = OnceLock::new();
static PRIORITY_PENDING: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
static PRIORITY_COMPLETIONS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
static WALL_WAITERS: OnceLock<
    Mutex<HashMap<PathBuf, Vec<crate::thumbnail_display::DisplayRequest>>>,
> = OnceLock::new();
static WALL_WANTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static WALL_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static PRIORITY_DISPATCHES: AtomicUsize = AtomicUsize::new(0);
// The normal gallery always uses the canonical 320 px cache.  Optional
// 640 px Photo Wall quality is opt-in (Settings > Interface) so ordinary
// browsing never reads originals just to sharpen thumbnails.
static HIGH_QUALITY_THUMBNAILS: AtomicBool = AtomicBool::new(false);

pub fn set_high_quality_thumbnails_enabled(enabled: bool) {
    HIGH_QUALITY_THUMBNAILS.store(enabled, Ordering::Relaxed);
}

pub fn high_quality_thumbnails_enabled() -> bool {
    HIGH_QUALITY_THUMBNAILS.load(Ordering::Relaxed)
}

fn thumbnail_size() -> u32 {
    if high_quality_thumbnails_enabled() {
        HIGH_QUALITY_THUMBNAIL_SIZE
    } else {
        NORMAL_THUMBNAIL_SIZE
    }
}

const PRIORITY_QUEUE_CAPACITY: usize = 512;
// Visible requests must not queue behind the single bulk RAW worker. Keep a
// small dedicated pool so several newly visible tiles can make progress while
// background generation continues; cache-key deduplication still prevents
// duplicate generation.
const PRIORITY_WORKERS: usize = 4;
const PRIORITY_NEWEST_DISPATCHES: usize = 7;
const PRIORITY_HANDOFF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

fn cache_entry_in_flight(destination: &Path) -> bool {
    IN_FLIGHT
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map(|in_flight| in_flight.contains(destination))
        .unwrap_or(false)
}

struct PendingGuard(PathBuf);

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = PRIORITY_PENDING
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
        {
            pending.remove(&self.0);
        }
    }
}

/// Ask the dedicated foreground thumbnail worker to create a thumbnail for a
/// tile that is currently being bound. The request is best-effort and
/// deduplicated; the regular bulk recovery/import pass remains untouched.
pub fn request_priority(path: String, mtime: Option<i64>, size_bytes: Option<i64>) {
    let Ok(destination) = cache_path(&path, mtime, size_bytes) else {
        return;
    };
    if existing_cache_path(&path, mtime, size_bytes)
        .ok()
        .flatten()
        .is_some()
    {
        return;
    }
    if known_decode_failure(&path, &destination) {
        return;
    }
    if !crate::source::cached_file_available(&path) {
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!("PIC_THUMBNAIL skip reason=unavailable uri={path}");
        }
        return;
    }

    let pending = PRIORITY_PENDING.get_or_init(|| Mutex::new(HashSet::new()));
    let Ok(mut pending) = pending.lock() else {
        return;
    };
    if !pending.insert(destination.clone()) {
        return;
    }
    drop(pending);
    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!(
            "PIC_THUMBNAIL schedule source=visible uri={path} cache={}",
            destination.display()
        );
    }

    let queue = priority_queue();
    let (queue, wake) = &**queue;
    let mut queue = queue.lock().expect("priority queue should not be poisoned");
    if queue.len() >= PRIORITY_QUEUE_CAPACITY {
        if let Some((_, _, _, evicted, _, quality)) = queue.pop_back() {
            if let Some(request) = quality {
                crate::thumbnail_display::retry_wall_request(request.key);
            }
            if let Ok(mut pending) = PRIORITY_PENDING
                .get_or_init(|| Mutex::new(HashSet::new()))
                .lock()
            {
                pending.remove(&evicted);
            }
        }
    }
    queue.push_front((
        path,
        mtime,
        size_bytes,
        destination.clone(),
        std::time::Instant::now(),
        None,
    ));
    wake.notify_one();
}

/// Return the source paths whose foreground thumbnails finished since the last UI poll.
pub fn take_priority_completions() -> Vec<PathBuf> {
    PRIORITY_COMPLETIONS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map(|mut completions| std::mem::take(&mut *completions))
        .unwrap_or_default()
}

pub fn priority_pending_count() -> usize {
    PRIORITY_PENDING
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map(|pending| {
            pending
                .iter()
                .filter(|path| !path.to_string_lossy().ends_with("-wall640.jpg"))
                .count()
        })
        .unwrap_or_default()
}

/// Keep bulk thumbnail work from competing with thumbnails currently needed
/// by visible gallery tiles. Foreground workers do not call this gate.
pub fn wait_for_priority_requests() {
    while priority_pending_count() > 0 {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

// Structural split only: included files remain in this module scope.
include!("thumbnail/cache.rs");
include!("thumbnail/maintenance.rs");
include!("thumbnail/recovery.rs");
include!("thumbnail/viewer.rs");
include!("thumbnail/nef.rs");
include!("thumbnail/decoders.rs");
include!("thumbnail/batch.rs");

#[cfg(test)]
mod wall_queue_tests {
    use super::*;
    fn job(quality: bool) -> PriorityRequest {
        let request = crate::thumbnail_display::request_for(
            "/cache/a-wall640.jpg".into(),
            "/absent.jpg".into(),
            0,
            0,
            0,
            "".into(),
            1200,
            800,
            false,
        );
        (
            "/absent.jpg".into(),
            None,
            None,
            PathBuf::from("a"),
            std::time::Instant::now(),
            quality.then_some(request),
        )
    }
    #[test]
    fn wall_quality_queue_preserves_normal_precedence_and_two_worker_limit() {
        let mut jobs = VecDeque::from([job(true), job(false), job(true)]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 0), Some(1));
        jobs.remove(1);
        assert_eq!(priority_dispatch_index(&jobs, 2, 0), None);
        assert_eq!(priority_dispatch_index(&jobs, 1, 0), Some(0));
    }
}

fn priority_queue() -> &'static Arc<PriorityQueue> {
    PRIORITY_QUEUE.get_or_init(|| {
        let queue = Arc::new((
            Mutex::new(VecDeque::<PriorityRequest>::new()),
            Condvar::new(),
        ));
        for _ in 0..PRIORITY_WORKERS {
            let queue = queue.clone();
            std::thread::spawn(move || loop {
                let (path, mtime, size_bytes, destination, queued_at, quality) = {
                    let (queue, wake) = &*queue;
                    let mut queue = queue.lock().expect("priority queue should not be poisoned");
                    let job = loop {
                        let dispatch = PRIORITY_DISPATCHES.load(Ordering::Relaxed);
                        if let Some(index) = priority_dispatch_index(
                            &queue,
                            WALL_ACTIVE.load(Ordering::Relaxed),
                            dispatch,
                        ) {
                            PRIORITY_DISPATCHES.fetch_add(1, Ordering::Relaxed);
                            let job = queue.remove(index).expect("selected queued request");
                            if job.5.is_some() {
                                WALL_ACTIVE.fetch_add(1, Ordering::Relaxed);
                            }
                            break job;
                        }
                        queue = wake
                            .wait(queue)
                            .expect("priority queue should not be poisoned");
                    };
                    job
                };
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_THUMBNAIL queue_wait kind=visible elapsed_us={}",
                        queued_at.elapsed().as_micros()
                    );
                }
                if let Some(_request) = quality {
                    if !destination.is_file() && crate::source::cached_file_available(&path) {
                        let _ = create_uncached_with_max(&path, &destination, 640);
                    }
                    let requests = {
                        let _jobs = queue.0.lock().expect("priority queue lock");
                        PRIORITY_PENDING
                            .get_or_init(|| Mutex::new(HashSet::new()))
                            .lock()
                            .expect("priority pending lock")
                            .remove(&destination);
                        WALL_WAITERS
                            .get_or_init(|| Mutex::new(HashMap::new()))
                            .lock()
                            .expect("wall waiters lock")
                            .remove(&destination)
                            .unwrap_or_default()
                    };
                    for request in requests {
                        crate::thumbnail_display::complete_wall_request(request);
                    }
                    WALL_ACTIVE.fetch_sub(1, Ordering::Relaxed);
                    queue.1.notify_all();
                    continue;
                }
                let started = std::time::Instant::now();
                let failure_marker = destination.with_extension("failed");
                let _pending_guard = PendingGuard(destination.clone());
                let deadline = started + PRIORITY_HANDOFF_TIMEOUT;
                while !destination.is_file() && !known_decode_failure(&path, &destination) {
                    if cache_entry_in_flight(&destination) {
                        if std::time::Instant::now() >= deadline {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(25));
                        continue;
                    }
                    if create(&path, mtime, size_bytes).is_err() {
                        break;
                    }
                    if destination.is_file()
                        || failure_marker.is_file()
                        || std::time::Instant::now() >= deadline
                    {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                if destination.is_file() {
                    if let Ok(mut completions) = PRIORITY_COMPLETIONS
                        .get_or_init(|| Mutex::new(Vec::new()))
                        .lock()
                    {
                        completions.push(PathBuf::from(&path));
                    }
                }
            });
        }
        queue
    })
}

fn priority_dispatch_index(
    jobs: &VecDeque<PriorityRequest>,
    active_quality: usize,
    dispatch: usize,
) -> Option<usize> {
    let ordinary = if dispatch % (PRIORITY_NEWEST_DISPATCHES + 1) == PRIORITY_NEWEST_DISPATCHES {
        jobs.iter().rposition(|job| job.5.is_none())
    } else {
        jobs.iter().position(|job| job.5.is_none())
    };
    ordinary.or_else(|| {
        (active_quality < 2)
            .then(|| jobs.iter().position(|job| job.5.is_some()))
            .flatten()
    })
}

pub fn request_wall_quality(request: crate::thumbnail_display::DisplayRequest) -> bool {
    let destination = PathBuf::from(&request.cached_path);
    let queue = priority_queue();
    let mut jobs = queue.0.lock().expect("priority queue lock");
    if !WALL_WANTED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .expect("wall wanted lock")
        .contains(&request.key)
    {
        return false;
    }
    if jobs.len() >= PRIORITY_QUEUE_CAPACITY {
        return false;
    }
    let mut pending = PRIORITY_PENDING
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .expect("priority pending lock");
    record_wall_waiter(
        &mut WALL_WAITERS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .expect("wall waiters lock"),
        request.clone(),
    );
    if !pending.insert(destination.clone()) {
        return true;
    }
    jobs.push_back((
        request.source_path.clone(),
        Some(request.mtime),
        Some(request.size_bytes),
        destination,
        std::time::Instant::now(),
        Some(request),
    ));
    drop(pending);
    drop(jobs);
    queue.1.notify_all();
    true
}

pub fn retain_wall_quality_requests(wanted: &HashSet<String>) {
    *WALL_WANTED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .expect("wall wanted lock") = wanted.clone();
    let Some(queue) = PRIORITY_QUEUE.get() else {
        return;
    };
    let mut jobs = queue.0.lock().expect("priority queue lock");
    let mut pending = PRIORITY_PENDING
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .expect("priority pending lock");
    WALL_WAITERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("wall waiters lock")
        .retain(|_, entries| {
            entries.retain(|request| wanted.contains(&request.key));
            !entries.is_empty()
        });
    jobs.retain(|job| {
        let retain = job
            .5
            .as_ref()
            .is_none_or(|request| wanted.contains(&request.key));
        if !retain {
            pending.remove(&job.3);
        }
        retain
    });
}

#[cfg(test)]
mod wall_waiter_tests {
    use super::*;
    #[test]
    fn wall_quality_deduplicates_generation_but_keeps_edited_presentation_waiters() {
        let first = crate::thumbnail_display::wall_request(crate::thumbnail_display::request_for(
            "/cache/a.jpg".into(),
            "/photo/a.jpg".into(),
            1,
            2,
            0,
            "".into(),
            1200,
            800,
            false,
        ));
        let mut rotated = first.clone();
        rotated.rotation = 90;
        rotated.key = crate::thumbnail_display::presentation_key(
            &rotated.cached_path,
            &rotated.source_path,
            rotated.rotation,
            &rotated.edit_recipe,
            1200,
            800,
        );
        let mut waiters = HashMap::new();
        record_wall_waiter(&mut waiters, first.clone());
        record_wall_waiter(&mut waiters, first.clone());
        record_wall_waiter(&mut waiters, rotated.clone());
        let values = waiters.remove(Path::new(&first.cached_path)).unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[1].key, rotated.key);
    }
}

fn record_wall_waiter(
    waiters: &mut HashMap<PathBuf, Vec<crate::thumbnail_display::DisplayRequest>>,
    request: crate::thumbnail_display::DisplayRequest,
) {
    let entries = waiters
        .entry(PathBuf::from(&request.cached_path))
        .or_default();
    if !entries.iter().any(|entry| entry.key == request.key) {
        entries.push(request);
    }
}
