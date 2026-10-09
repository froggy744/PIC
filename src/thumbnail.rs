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
const THUMBNAIL_CACHE_VERSION: &[u8] = b"picasa-thumb-v5-icc-srgb";
const REMOTE_RAW_THUMBNAIL_CACHE_VERSION: &[u8] = b"picasa-thumb-v1-remote-raw";
const RAW_THUMBNAIL_CACHE_VERSION: &[u8] = b"picasa-thumb-v6-generic-raw";
const REMOTE_NEF_THUMBNAIL_CACHE_VERSION: &[u8] = b"picasa-thumb-v1-remote-nef-preview";
const REMOTE_JPEG_THUMBNAIL_CACHE_VERSION: &[u8] = b"picasa-thumb-v2-remote-jpeg-icc-srgb";
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
    bool,
);

type PriorityQueue = (Mutex<VecDeque<PriorityRequest>>, Condvar);

static PRIORITY_QUEUE: OnceLock<Arc<PriorityQueue>> = OnceLock::new();
static PRIORITY_PENDING: OnceLock<Mutex<HashMap<PathBuf, ThumbnailWorkState>>> = OnceLock::new();
static PRIORITY_COMPLETIONS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
static WALL_WAITERS: OnceLock<
    Mutex<HashMap<PathBuf, Vec<crate::thumbnail_display::DisplayRequest>>>,
> = OnceLock::new();
static WALL_WANTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static RAW_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static NEF_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static VISIBLE_GENERATION_WANTED: OnceLock<Mutex<Option<HashMap<String, usize>>>> = OnceLock::new();
static WALL_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static PRIORITY_DISPATCHES: AtomicUsize = AtomicUsize::new(0);
// Grid and Photo Wall share one canonical cache. The setting only changes
// that cache's generated longest edge (320 or 640); it never creates a
// second view-specific thumbnail file.
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
// Network prefetch is speculative; a scrollbar jump must not leave a library
// of old reads competing with the destination's visible previews.
const NETWORK_PREFETCH_CAPACITY: usize = 8;
static NETWORK_PREFETCH_WANTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

pub(crate) fn private_network_preview(path: &str) -> bool {
    #[cfg(target_os = "linux")]
    { crate::network_shares::private(path) }
    #[cfg(not(target_os = "linux"))]
    { let _ = path; false }
}

#[cfg(test)]
pub(crate) fn network_prefetch_queued_count() -> usize {
    PRIORITY_QUEUE.get().and_then(|queue| queue.0.lock().ok()).map(|jobs| {
        jobs.iter().filter(|job| private_network_preview(&job.0) && !job.6 && job.5.is_none()).count()
    }).unwrap_or(0)
}

fn network_prefetch_has_capacity(jobs: &VecDeque<PriorityRequest>) -> bool {
    jobs.iter().filter(|job| private_network_preview(&job.0) && !job.6 && job.5.is_none())
        .count() < NETWORK_PREFETCH_CAPACITY
}

fn release_discarded_generation(jobs: Vec<PriorityRequest>) {
    if let Some(pending) = PRIORITY_PENDING.get() {
        if let Ok(mut pending) = pending.lock() {
            release_discarded_generation_from(&mut pending, jobs);
        }
    }
}

fn release_discarded_generation_from(
    pending: &mut HashMap<PathBuf, ThumbnailWorkState>, jobs: Vec<PriorityRequest>,
) {
    for job in jobs {
        if pending.get(&job.3) == Some(&ThumbnailWorkState::Queued) { pending.remove(&job.3); }
    }
}

pub(crate) fn retain_network_prefetch_paths(wanted: &HashSet<String>) {
    let queue = priority_queue();
    let mut jobs = queue.0.lock().expect("priority queue lock");
    let mut allowed = NETWORK_PREFETCH_WANTED.get_or_init(|| Mutex::new(HashSet::new()))
        .lock().expect("network prefetch lock");
    *allowed = wanted.iter().take(NETWORK_PREFETCH_CAPACITY).cloned().collect();
    let mut discarded = Vec::new();
    jobs.retain(|job| {
        let keep = !private_network_preview(&job.0) || job.6 || job.5.is_some()
            || allowed.contains(&job.0);
        if !keep { discarded.push(job.clone()); }
        keep
    });
    release_discarded_generation(discarded);
}

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

// Missing, Ready and Failed are represented by the cache and its failure
// marker. Keep ownership for both pending states to suppress repeated binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThumbnailWorkState {
    Queued,
    Generating,
}

struct RawWorkerGuard(bool, bool, Arc<PriorityQueue>);

impl Drop for RawWorkerGuard {
    fn drop(&mut self) {
        if self.0 || self.1 {
            let _jobs = self.2.0.lock().expect("priority queue lock");
            if self.0 { RAW_ACTIVE.fetch_sub(1, Ordering::Relaxed); }
            if self.1 { NEF_ACTIVE.fetch_sub(1, Ordering::Relaxed); }
            self.2.1.notify_all();
        }
    }
}

struct PendingGuard(PathBuf);

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = PRIORITY_PENDING
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
        {
            pending.remove(&self.0);
        }
    }
}

/// Ask the dedicated foreground thumbnail worker to create a thumbnail for a
/// tile that is being bound or prefetched. Visible requests are promoted
/// ahead of prefetch work and ownership lasts through generation.
pub fn request_priority(path: String, mtime: Option<i64>, size_bytes: Option<i64>, visible: bool) {
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

    let queue = priority_queue();
    let (jobs, wake) = &**queue;
    let mut jobs = jobs.lock().expect("priority queue lock");
    let requested_visible = visible;
    let visible = current_generation_priority(&path, visible);
    if private_network_preview(&path) && !visible {
        let wanted = NETWORK_PREFETCH_WANTED.get_or_init(|| Mutex::new(HashSet::new()))
            .lock().expect("network prefetch lock");
        if requested_visible || !wanted.contains(&path) || !network_prefetch_has_capacity(&jobs) {
            return;
        }
    }
    let pending = PRIORITY_PENDING.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut pending) = pending.lock() else {
        return;
    };
    if pending.contains_key(&destination) {
        if visible {
            if let Some(job) = jobs.iter_mut().find(|job| job.3 == destination) {
                job.6 = true;
                wake.notify_all();
            }
        }
        return;
    }
    pending.insert(destination.clone(), ThumbnailWorkState::Queued);
    drop(pending);
    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!(
            "PIC_THUMBNAIL schedule source={} uri={path} cache={}",
            if visible { "visible" } else { "prefetch" },
            destination.display()
        );
    }

    if jobs.len() >= PRIORITY_QUEUE_CAPACITY {
        // Prefetch may replace older prefetch, but cannot evict onscreen work.
        let eviction = jobs.iter().rposition(|job| !job.6)
            .or_else(|| visible.then(|| jobs.len() - 1));
        let Some(eviction) = eviction else {
            drop(PendingGuard(destination));
            return;
        };
        if let Some((_, _, _, evicted, _, quality, _)) = jobs.remove(eviction) {
            if let Some(request) = quality {
                crate::thumbnail_display::retry_wall_request(request.key);
            }
            if let Ok(mut pending) = PRIORITY_PENDING
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
            {
                pending.remove(&evicted);
            }
        }
    }
    jobs.push_front((
        path,
        mtime,
        size_bytes,
        destination.clone(),
        std::time::Instant::now(),
        None,
        visible,
    ));
    wake.notify_all();
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
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map(|pending| pending.len())
        .unwrap_or_default()
}

// Structural split only: included files remain in this module scope.
include!("thumbnail/cache.rs");
include!("thumbnail/maintenance.rs");
include!("thumbnail/recovery.rs");
include!("thumbnail/viewer.rs");
include!("thumbnail/nef.rs");
include!("thumbnail/dng.rs");
mod color;
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
            true,
        )
    }
    #[test]
    fn visible_jpeg_precedes_newer_raw_requests() {
        let mut raw = job(false);
        raw.0 = "/photo/newer.DNG".into();
        let mut jpeg = job(false);
        jpeg.0 = "/photo/visible.JPG".into();
        let jobs = VecDeque::from([raw, jpeg]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 0, 0), Some(1));
    }

    #[test]
    fn active_raw_leaves_workers_available_for_visible_images() {
        let mut raw = job(false);
        raw.0 = "/photo/a.dng".into();
        let mut quality_raw = raw.clone();
        quality_raw.5 = job(true).5;
        let mut jobs = VecDeque::from([raw, quality_raw]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 1, 0), None);
        jobs.push_back(job(false));
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 1, 0), Some(2));
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 0, 0), Some(2));
    }

    #[test]
    fn visible_nef_preview_does_not_wait_for_active_dng() {
        let mut nef = job(false);
        nef.0 = "/photo/current.nef".into();
        let jobs = VecDeque::from([nef]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 1, 0), Some(0));
    }

    #[test]
    fn two_nef_previews_still_leave_a_worker_for_visible_jpeg() {
        let mut nef = job(false);
        nef.0 = "/photo/current.nef".into();
        let mut jobs = VecDeque::from([nef]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 1, 2), None);
        jobs.push_back(job(false));
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 1, 2), Some(1));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn network_prefetch_budget_does_not_limit_visible_or_local_requests() {
        let mut remote = job(false);
        remote.0 = "nfs://nas/photos/prefetch.jpg".into();
        remote.6 = false;
        let mut jobs = VecDeque::from(vec![remote.clone(); NETWORK_PREFETCH_CAPACITY - 1]);
        assert!(network_prefetch_has_capacity(&jobs));
        jobs.push_back(remote.clone());
        assert!(!network_prefetch_has_capacity(&jobs));
        jobs[0].6 = true;
        assert!(network_prefetch_has_capacity(&jobs));
        let mut local = remote;
        local.0 = "/photos/local.jpg".into();
        jobs.extend(vec![local; 100]);
        assert!(network_prefetch_has_capacity(&jobs));
        let mut pending = HashMap::from([(PathBuf::from("generating"), ThumbnailWorkState::Generating),
            (PathBuf::from("queued"), ThumbnailWorkState::Queued)]);
        // Removal must only release queued ownership, never an in-flight read.
        let mut dropped = job(false);
        dropped.3 = PathBuf::from("queued");
        let mut active = dropped.clone();
        active.3 = PathBuf::from("generating");
        release_discarded_generation_from(&mut pending, vec![dropped, active]);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[&PathBuf::from("generating")], ThumbnailWorkState::Generating);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn viewport_change_drops_queued_network_reads_but_keeps_current_and_local_work() {
        let mut old_remote = job(false);
        old_remote.0 = "nfs://nas/photos/old.jpg".into();
        let mut current_remote = job(false);
        current_remote.0 = "smb://nas/photos/current.jpg".into();
        current_remote.6 = false;
        let local = job(false);
        let mut jobs = VecDeque::from([old_remote, local, current_remote]);
        update_generation_visibility(&mut jobs, &HashMap::from([
            ("smb://nas/photos/current.jpg".into(), 0)]));
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].0, "smb://nas/photos/current.jpg");
        assert!(jobs[0].6);
        assert_eq!(jobs[1].0, "/absent.jpg");
    }

    #[test]
    fn viewport_change_demotes_old_generation_and_promotes_current_requests() {
        let mut old = job(false);
        old.0 = "/photo/old.dng".into();
        let mut current = job(false);
        current.0 = "/photo/current.nef".into();
        current.6 = false;
        let mut jobs = VecDeque::from([old, current]);
        let wanted = HashMap::from([("/photo/current.nef".into(), 0)]);
        update_generation_visibility(&mut jobs, &wanted);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].0, "/photo/current.nef");
        assert!(jobs[0].6);
        assert!(!jobs[1].6);
    }

    #[test]
    fn late_prefetch_becoming_visible_uses_current_viewport_priority() {
        let wanted = HashMap::from([("/photo/current.nef".into(), 0)]);
        assert!(generation_priority_for("/photo/current.nef", false, Some(&wanted)));
        assert!(!generation_priority_for("/photo/old.dng", true, Some(&wanted)));
        assert!(generation_priority_for("/photo/initial.jpg", true, None));
    }

    #[test]
    fn empty_viewport_demotes_all_queued_generation() {
        let mut jobs = VecDeque::from([job(false), job(true)]);
        update_generation_visibility(&mut jobs, &HashMap::new());
        assert!(jobs.iter().all(|job| !job.6));
        assert_eq!(jobs.len(), 2);
    }

    #[test]
    fn visible_raw_precedes_prefetched_jpeg() {
        let mut raw = job(false);
        raw.0 = "/photo/visible.dng".into();
        let mut jpeg = job(false);
        jpeg.0 = "/photo/prefetch.jpg".into();
        jpeg.6 = false;
        let jobs = VecDeque::from([jpeg, raw]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 0, 0), Some(1));
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 1, 0), Some(0));
    }

    #[test]
    fn wall_quality_queue_preserves_normal_precedence_and_two_worker_limit() {
        let mut jobs = VecDeque::from([job(true), job(false), job(true)]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 0, 0), Some(1));
        jobs.remove(1);
        assert_eq!(priority_dispatch_index(&jobs, 2, 0, 0, 0), None);
        assert_eq!(priority_dispatch_index(&jobs, 1, 0, 0, 0), Some(0));
    }
}

fn update_generation_visibility(
    jobs: &mut VecDeque<PriorityRequest>,
    wanted: &HashMap<String, usize>,
) -> Vec<PriorityRequest> {
    let mut discarded = Vec::new();
    jobs.retain(|job| {
        let keep = !private_network_preview(&job.0) || job.5.is_some() || wanted.contains_key(&job.0);
        if !keep { discarded.push(job.clone()); }
        keep
    });
    for job in jobs.iter_mut() {
        job.6 = wanted.contains_key(&job.0);
    }
    jobs.make_contiguous().sort_by_key(|job| {
        wanted.get(&job.0).copied().unwrap_or(usize::MAX)
    });
    discarded
}

fn generation_priority_for(
    path: &str,
    requested: bool,
    wanted: Option<&HashMap<String, usize>>,
) -> bool {
    wanted.map_or(requested, |wanted| wanted.contains_key(path))
}

// Call under the generation queue lock, as viewport updates do.
fn current_generation_priority(path: &str, requested: bool) -> bool {
    let wanted = VISIBLE_GENERATION_WANTED.get_or_init(|| Mutex::new(None))
        .lock().expect("generation viewport lock");
    generation_priority_for(path, requested, wanted.as_ref())
}

/// Generation and cache presentation must agree about the current viewport.
/// Drop queued remote reads from old viewports; local cache work can be demoted.
pub fn retain_visible_generation_requests(requests: &[crate::thumbnail_display::DisplayRequest]) {
    let wanted: HashMap<String, usize> = requests.iter().enumerate()
        .map(|(index, request)| (request.source_path.clone(), index)).collect();
    let queue = priority_queue();
    let mut jobs = queue.0.lock().expect("priority queue lock");
    let mut current = VISIBLE_GENERATION_WANTED.get_or_init(|| Mutex::new(None))
        .lock().expect("generation viewport lock");
    if current.as_ref() != Some(&wanted) {
        release_discarded_generation(update_generation_visibility(&mut jobs, &wanted));
        NETWORK_PREFETCH_WANTED.get_or_init(|| Mutex::new(HashSet::new()))
            .lock().expect("network prefetch lock").clear();
        *current = Some(wanted);
    }
    queue.1.notify_all();
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
                let (path, mtime, size_bytes, destination, queued_at, quality, visible) = {
                    let (queue, wake) = &*queue;
                    let mut queue = queue.lock().expect("priority queue should not be poisoned");
                    let job = loop {
                        let dispatch = PRIORITY_DISPATCHES.load(Ordering::Relaxed);
                        if let Some(index) = priority_dispatch_index(
                            &queue,
                            WALL_ACTIVE.load(Ordering::Relaxed),
                            dispatch,
                            RAW_ACTIVE.load(Ordering::Relaxed),
                            NEF_ACTIVE.load(Ordering::Relaxed),
                        ) {
                            PRIORITY_DISPATCHES.fetch_add(1, Ordering::Relaxed);
                            let job = queue.remove(index).expect("selected queued request");
                            if is_nikon_raw(&job.0) {
                                NEF_ACTIVE.fetch_add(1, Ordering::Relaxed);
                            } else if is_raw(&job.0) {
                                RAW_ACTIVE.fetch_add(1, Ordering::Relaxed);
                            }
                            if let Ok(mut pending) = PRIORITY_PENDING.get().unwrap().lock() {
                                pending.insert(job.3.clone(), ThumbnailWorkState::Generating);
                            }
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
                let _raw_guard = RawWorkerGuard(
                    is_raw(&path) && !is_nikon_raw(&path), is_nikon_raw(&path), queue.clone()
                );
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_THUMBNAIL queue_wait kind={} elapsed_us={} uri={path}",
                        if visible { "visible" } else { "prefetch" },
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
                            .get_or_init(|| Mutex::new(HashMap::new()))
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
    active_raw: usize,
    active_nef: usize,
) -> Option<usize> {
    // NEF embedded previews must not wait behind DNG sensor recovery.
    // At most two NEFs and one other RAW occupy the four-worker pool,
    // reserving at least one worker for visible JPEGs and other formats.
    let eligible = |job: &PriorityRequest| {
        if is_nikon_raw(&job.0) { active_nef < 2 }
        else { !is_raw(&job.0) || active_raw == 0 }
    };
    let select = |predicate: &dyn Fn(&PriorityRequest) -> bool| {
        if dispatch % (PRIORITY_NEWEST_DISPATCHES + 1) == PRIORITY_NEWEST_DISPATCHES {
            jobs.iter().rposition(predicate)
        } else {
            jobs.iter().position(predicate)
        }
    };
    let ordinary = |visible: bool| {
        select(&|job| job.6 == visible && job.5.is_none() && is_jpeg(&job.0))
            .or_else(|| select(&|job| job.6 == visible && job.5.is_none() && (!is_raw(&job.0) || is_nikon_raw(&job.0)) && eligible(job)))
            .or_else(|| select(&|job| job.6 == visible && job.5.is_none() && eligible(job)))
    };
    let quality = |visible: bool| {
        (active_quality < 2)
            .then(|| jobs.iter().position(|job| job.6 == visible && job.5.is_some() && eligible(job)))
            .flatten()
    };
    ordinary(true)
        .or_else(|| quality(true))
        .or_else(|| ordinary(false))
        .or_else(|| quality(false))
}

pub fn request_wall_quality(request: crate::thumbnail_display::DisplayRequest) -> bool {
    let destination = PathBuf::from(&request.cached_path);
    let queue = priority_queue();
    let mut jobs = queue.0.lock().expect("priority queue lock");
    let visible = current_generation_priority(&request.source_path, request.visible_priority);
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
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("priority pending lock");
    record_wall_waiter(
        &mut WALL_WAITERS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .expect("wall waiters lock"),
        request.clone(),
    );
    if pending.contains_key(&destination) {
        if visible {
            if let Some(job) = jobs.iter_mut().find(|job| job.3 == destination) {
                job.6 = true;
                queue.1.notify_all();
            }
        }
        return true;
    }
    pending.insert(destination.clone(), ThumbnailWorkState::Queued);
    jobs.push_back((
        request.source_path.clone(),
        Some(request.mtime),
        Some(request.size_bytes),
        destination,
        std::time::Instant::now(),
        Some(request.clone()),
        visible,
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
        .get_or_init(|| Mutex::new(HashMap::new()))
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
