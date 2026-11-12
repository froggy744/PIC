use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

use anyhow::{Context, Result};
use fast_image_resize::{
    images::{Image, ImageRef},
    pixels::PixelType,
    FilterType, ResizeAlg, ResizeOptions, Resizer,
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
    bool,
);

type PriorityQueue = (Mutex<VecDeque<PriorityRequest>>, Condvar);

static PRIORITY_QUEUE: OnceLock<Arc<PriorityQueue>> = OnceLock::new();
static PRIORITY_PENDING: OnceLock<Mutex<HashMap<PathBuf, ThumbnailWorkState>>> = OnceLock::new();
static PRIORITY_COMPLETIONS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
// The canonical cache path does not encode thumbnail quality, and the cache is
// shared by every library. A library with High quality off can therefore leave
// a 320 px file that a High quality library would otherwise reuse forever.
// These markers let a display worker ask the priority pool to replace such an
// entry in place, while attempts stay session-scoped so a source that cannot
// produce 640 px is not decoded again on every bind.
static PRIORITY_UPGRADES: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
static QUALITY_UPGRADE_ATTEMPTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
static RAW_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static NEF_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static VISIBLE_GENERATION_WANTED: OnceLock<Mutex<Option<HashMap<String, usize>>>> = OnceLock::new();
static PRIORITY_DISPATCHES: AtomicUsize = AtomicUsize::new(0);
// Grid, Photo Wall and Masonry share one canonical cache. The setting only changes
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

/// Whether a decoded cache entry is smaller than the current quality setting.
pub(crate) fn thumbnail_needs_quality_upgrade(width: u32, height: u32) -> bool {
    high_quality_thumbnails_enabled() && width.max(height) < HIGH_QUALITY_THUMBNAIL_SIZE
}

fn priority_upgrade_marked(destination: &Path) -> bool {
    PRIORITY_UPGRADES
        .get()
        .and_then(|marked| marked.lock().ok())
        .is_some_and(|marked| marked.contains(destination))
}

fn mark_priority_upgrade(destination: &Path) {
    if let Ok(mut marked) = PRIORITY_UPGRADES
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
    {
        marked.insert(destination.to_path_buf());
    }
}

fn clear_priority_upgrade(destination: &Path) {
    if let Some(marked) = PRIORITY_UPGRADES.get() {
        if let Ok(mut marked) = marked.lock() {
            marked.remove(destination);
        }
    }
}

/// Forget that an upgrade was already requested.
///
/// An upgrade that never ran - because it was evicted, discarded with the
/// viewport or failed - has to be requestable again, otherwise the photo
/// stays at the lower resolution forever.
fn forget_quality_upgrade_attempt(destination: &Path) {
    if let Some(attempts) = QUALITY_UPGRADE_ATTEMPTS.get() {
        if let Ok(mut attempts) = attempts.lock() {
            attempts.remove(destination);
        }
    }
}

const PRIORITY_QUEUE_CAPACITY: usize = 512;
// Network prefetch is speculative; a scrollbar jump must not leave a library
// of old reads competing with the destination's visible previews.
const NETWORK_PREFETCH_CAPACITY: usize = 8;
static NETWORK_PREFETCH_WANTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

pub(crate) fn private_network_preview(path: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::network_shares::private(path)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        false
    }
}

#[cfg(test)]
pub(crate) fn network_prefetch_queued_count() -> usize {
    PRIORITY_QUEUE
        .get()
        .and_then(|queue| queue.0.lock().ok())
        .map(|jobs| {
            jobs.iter()
                .filter(|job| private_network_preview(&job.0) && !job.5)
                .count()
        })
        .unwrap_or(0)
}

fn network_prefetch_has_capacity(jobs: &VecDeque<PriorityRequest>) -> bool {
    jobs.iter()
        .filter(|job| private_network_preview(&job.0) && !job.5)
        .count()
        < NETWORK_PREFETCH_CAPACITY
}

fn release_discarded_generation(jobs: Vec<PriorityRequest>) {
    if let Some(pending) = PRIORITY_PENDING.get() {
        if let Ok(mut pending) = pending.lock() {
            release_discarded_generation_from(&mut pending, jobs);
        }
    }
}

fn release_discarded_generation_from(
    pending: &mut HashMap<PathBuf, ThumbnailWorkState>,
    jobs: Vec<PriorityRequest>,
) {
    for job in jobs {
        crate::thumbnail_trace::event(&job.0, "generation_request", "discarded_by_viewport");
        if pending.get(&job.3) == Some(&ThumbnailWorkState::Queued) {
            pending.remove(&job.3);
        }
        // The upgrade never ran, so the photo may ask for it again.
        clear_priority_upgrade(&job.3);
        forget_quality_upgrade_attempt(&job.3);
    }
}

pub(crate) fn retain_network_prefetch_paths(wanted: &HashSet<String>) {
    let queue = priority_queue();
    let mut jobs = queue.0.lock().expect("priority queue lock");
    let mut allowed = NETWORK_PREFETCH_WANTED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .expect("network prefetch lock");
    *allowed = wanted
        .iter()
        .take(NETWORK_PREFETCH_CAPACITY)
        .cloned()
        .collect();
    let mut discarded = Vec::new();
    jobs.retain(|job| {
        let keep = !private_network_preview(&job.0) || job.5 || allowed.contains(&job.0);
        if !keep {
            discarded.push(job.clone());
        }
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
            let _jobs = self.2 .0.lock().expect("priority queue lock");
            if self.0 {
                RAW_ACTIVE.fetch_sub(1, Ordering::Relaxed);
            }
            if self.1 {
                NEF_ACTIVE.fetch_sub(1, Ordering::Relaxed);
            }
            self.2 .1.notify_all();
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
    request_priority_impl(path, mtime, size_bytes, visible, false);
}

/// Replace a cache entry that is smaller than the High quality setting.
///
/// The canonical cache path is shared by every library, so a 320 px file
/// created while another library had High quality off would otherwise be shown
/// forever. Unlike a normal request this must run even though the file exists,
/// but only when the original is reachable: an offline 320 px preview is still
/// better than no preview at all.
/// Quality upgrades rebuild a thumbnail at the higher resolution.
///
/// An upgrade only sharpens what is already on screen, so it is queued as
/// background work and never competes with the thumbnail the user is
/// scrolling towards. It is also skipped while the gallery is moving, and
/// requested again once the gallery settles.
pub(crate) fn request_quality_upgrade(path: String, mtime: Option<i64>, size_bytes: Option<i64>) {
    let Ok(destination) = cache_path(&path, mtime, size_bytes) else {
        crate::thumbnail_trace::event(&path, "generation_request", "cache_path_error");
        return;
    };
    if !crate::source::cached_file_available(&path) {
        crate::thumbnail_trace::event(
            &path,
            "generation_request",
            "quality_upgrade_source_unavailable",
        );
        return;
    }
    let Ok(mut attempts) = QUALITY_UPGRADE_ATTEMPTS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
    else {
        return;
    };
    if !attempts.insert(destination.clone()) {
        return;
    }
    drop(attempts);
    mark_priority_upgrade(&destination);
    // An upgrade only sharpens what is already on screen, so it always runs
    // as background work. Marked visible it would compete with the
    // thumbnails the user is scrolling towards, and the gallery would spend
    // its whole decode budget on re-encoding photos that are about to leave
    // the viewport.
    request_priority_impl(path, mtime, size_bytes, false, true);
}

fn request_priority_impl(
    path: String,
    mtime: Option<i64>,
    size_bytes: Option<i64>,
    visible: bool,
    upgrade: bool,
) {
    let Ok(destination) = cache_path(&path, mtime, size_bytes) else {
        crate::thumbnail_trace::event(&path, "generation_request", "cache_path_error");
        return;
    };
    if !upgrade
        && existing_cache_path(&path, mtime, size_bytes)
            .ok()
            .flatten()
            .is_some()
    {
        crate::thumbnail_trace::event(&path, "generation_request", "already_cached");
        return;
    }
    if known_decode_failure(&path, &destination) {
        crate::thumbnail_trace::event(&path, "generation_request", "decode_failure_suppressed");
        // The upgrade never runs, so the pending mark has to go. Otherwise the
        // display worker waits for a file that is never rewritten and the tile
        // stays empty for the whole handoff timeout.
        clear_priority_upgrade(&destination);
        return;
    }
    if !crate::source::cached_file_available(&path) {
        crate::thumbnail_trace::event(&path, "generation_request", "unavailable_suppressed");
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!("PIC_THUMBNAIL skip reason=unavailable uri={path}");
        }
        clear_priority_upgrade(&destination);
        return;
    }

    let queue = priority_queue();
    let (jobs, wake) = &**queue;
    let mut jobs = jobs.lock().expect("priority queue lock");
    let requested_visible = visible;
    // A quality upgrade is background work even when its photo is in the
    // viewport: the sharp thumbnail can wait, the thumbnail the user is
    // scrolling towards cannot.
    let visible = if upgrade {
        false
    } else {
        current_generation_priority(&path, visible)
    };
    if private_network_preview(&path) && !visible {
        let wanted = NETWORK_PREFETCH_WANTED
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .expect("network prefetch lock");
        if requested_visible || !wanted.contains(&path) || !network_prefetch_has_capacity(&jobs) {
            crate::thumbnail_trace::event(
                &path,
                "generation_request",
                "network_prefetch_suppressed",
            );
            return;
        }
    }
    let pending = PRIORITY_PENDING.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut pending) = pending.lock() else {
        crate::thumbnail_trace::event(&path, "generation_request", "pending_lock_failed");
        return;
    };
    if pending.contains_key(&destination) {
        crate::thumbnail_trace::event(&path, "generation_request", "already_pending");
        if visible {
            if let Some(job) = jobs.iter_mut().find(|job| job.3 == destination) {
                job.5 = true;
                wake.notify_all();
            }
        }
        // The queued job owns the upgrade now; a stale mark would make the
        // display worker wait for a file that the queued job never writes.
        // The queued job is not necessarily the upgrade itself, so the photo
        // stays allowed to ask for it again.
        clear_priority_upgrade(&destination);
        forget_quality_upgrade_attempt(&destination);
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
        let eviction = jobs
            .iter()
            .rposition(|job| !job.5)
            .or_else(|| visible.then(|| jobs.len() - 1));
        let Some(eviction) = eviction else {
            drop(PendingGuard(destination.clone()));
            clear_priority_upgrade(&destination);
            crate::thumbnail_trace::event(&path, "generation_request", "queue_capacity_rejected");
            return;
        };
        if let Some((evicted_path, _, _, evicted, _, _)) = jobs.remove(eviction) {
            crate::thumbnail_trace::event(
                &evicted_path,
                "generation_request",
                "evicted_for_visible_request",
            );
            if let Ok(mut pending) = PRIORITY_PENDING
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
            {
                pending.remove(&evicted);
            }
            // The evicted upgrade never ran, so the photo may ask again.
            clear_priority_upgrade(&evicted);
            forget_quality_upgrade_attempt(&evicted);
        }
    }
    jobs.push_front((
        path.clone(),
        mtime,
        size_bytes,
        destination.clone(),
        std::time::Instant::now(),
        visible,
    ));
    crate::thumbnail_trace::event(
        &path,
        "generation_request",
        if visible {
            "queued_visible"
        } else {
            "queued_prefetch"
        },
    );
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
mod nef_scene;
#[cfg(test)]
mod raw_quality;
include!("thumbnail/decoders.rs");
include!("thumbnail/batch.rs");

#[cfg(test)]
mod priority_queue_tests {
    use super::*;
    fn job(visible: bool) -> PriorityRequest {
        (
            "/absent.jpg".into(),
            None,
            None,
            PathBuf::from("a"),
            std::time::Instant::now(),
            visible,
        )
    }
    /// A quality upgrade is background work: it must never be dispatched
    /// ahead of a visible thumbnail, no matter how long the upgrade has been
    /// waiting.
    #[test]
    fn quality_upgrade_never_precedes_a_visible_thumbnail() {
        let mut upgrade = job(false);
        upgrade.0 = "/photo/upgrade.jpg".into();
        let mut visible = job(true);
        visible.0 = "/photo/visible.jpg".into();
        let jobs = VecDeque::from([upgrade, visible]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 0), Some(1));
    }

    /// Once the visible work is done the upgrade has to run, otherwise the
    /// photo would stay at the lower resolution forever.
    #[test]
    fn quality_upgrade_runs_when_no_visible_work_is_queued() {
        let mut upgrade = job(false);
        upgrade.0 = "/photo/upgrade.jpg".into();
        let jobs = VecDeque::from([upgrade]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 0), Some(0));
    }

    /// An upgrade that was evicted before it ran must be requestable again,
    /// otherwise the photo keeps the low resolution thumbnail forever.
    #[test]
    fn an_evicted_upgrade_can_be_requested_again() {
        let destination = PathBuf::from("/tmp/pic-upgrade-test-thumb.jpg");
        mark_priority_upgrade(&destination);
        QUALITY_UPGRADE_ATTEMPTS
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .expect("upgrade attempt lock")
            .insert(destination.clone());
        // This is what the eviction path does.
        clear_priority_upgrade(&destination);
        forget_quality_upgrade_attempt(&destination);
        assert!(!priority_upgrade_marked(&destination));
        let attempts = QUALITY_UPGRADE_ATTEMPTS
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .expect("upgrade attempt lock");
        assert!(!attempts.contains(&destination));
    }

    #[test]
    fn visible_jpeg_precedes_newer_raw_requests() {
        let mut raw = job(false);
        raw.0 = "/photo/newer.DNG".into();
        let mut jpeg = job(false);
        jpeg.0 = "/photo/visible.JPG".into();
        let jobs = VecDeque::from([raw, jpeg]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 0), Some(1));
    }

    #[test]
    fn active_raw_leaves_workers_available_for_visible_images() {
        let mut raw = job(false);
        raw.0 = "/photo/a.dng".into();
        let mut jobs = VecDeque::from([raw]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 1, 0), None);
        let mut jpeg = job(true);
        jpeg.0 = "/photo/visible.jpg".into();
        jobs.push_back(jpeg);
        assert_eq!(priority_dispatch_index(&jobs, 0, 1, 0), Some(1));
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 0), Some(1));
    }

    #[test]
    fn visible_nef_preview_does_not_wait_for_active_dng() {
        let mut nef = job(false);
        nef.0 = "/photo/current.nef".into();
        let jobs = VecDeque::from([nef]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 1, 0), Some(0));
    }

    #[test]
    fn two_nef_previews_still_leave_a_worker_for_visible_jpeg() {
        let mut nef = job(false);
        nef.0 = "/photo/current.nef".into();
        let mut jobs = VecDeque::from([nef]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 1, 2), None);
        jobs.push_back(job(false));
        assert_eq!(priority_dispatch_index(&jobs, 0, 1, 2), Some(1));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn network_prefetch_budget_does_not_limit_visible_or_local_requests() {
        let mut remote = job(false);
        remote.0 = "nfs://nas/photos/prefetch.jpg".into();
        remote.5 = false;
        let mut jobs = VecDeque::from(vec![remote.clone(); NETWORK_PREFETCH_CAPACITY - 1]);
        assert!(network_prefetch_has_capacity(&jobs));
        jobs.push_back(remote.clone());
        assert!(!network_prefetch_has_capacity(&jobs));
        jobs[0].5 = true;
        assert!(network_prefetch_has_capacity(&jobs));
        let mut local = remote;
        local.0 = "/photos/local.jpg".into();
        jobs.extend(vec![local; 100]);
        assert!(network_prefetch_has_capacity(&jobs));
        let mut pending = HashMap::from([
            (PathBuf::from("generating"), ThumbnailWorkState::Generating),
            (PathBuf::from("queued"), ThumbnailWorkState::Queued),
        ]);
        // Removal must only release queued ownership, never an in-flight read.
        let mut dropped = job(false);
        dropped.3 = PathBuf::from("queued");
        let mut active = dropped.clone();
        active.3 = PathBuf::from("generating");
        release_discarded_generation_from(&mut pending, vec![dropped, active]);
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[&PathBuf::from("generating")],
            ThumbnailWorkState::Generating
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn viewport_change_drops_queued_network_reads_but_keeps_current_and_local_work() {
        let mut old_remote = job(false);
        old_remote.0 = "nfs://nas/photos/old.jpg".into();
        let mut current_remote = job(false);
        current_remote.0 = "smb://nas/photos/current.jpg".into();
        current_remote.5 = false;
        let local = job(false);
        let mut jobs = VecDeque::from([old_remote, local, current_remote]);
        update_generation_visibility(
            &mut jobs,
            &HashMap::from([("smb://nas/photos/current.jpg".into(), 0)]),
        );
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].0, "smb://nas/photos/current.jpg");
        assert!(jobs[0].5);
        assert_eq!(jobs[1].0, "/absent.jpg");
    }

    #[test]
    fn viewport_change_demotes_old_generation_and_promotes_current_requests() {
        let mut old = job(false);
        old.0 = "/photo/old.dng".into();
        let mut current = job(false);
        current.0 = "/photo/current.nef".into();
        current.5 = false;
        let mut jobs = VecDeque::from([old, current]);
        let wanted = HashMap::from([("/photo/current.nef".into(), 0)]);
        update_generation_visibility(&mut jobs, &wanted);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].0, "/photo/current.nef");
        assert!(jobs[0].5);
        assert!(!jobs[1].5);
    }

    #[test]
    fn late_prefetch_becoming_visible_uses_current_viewport_priority() {
        let wanted = HashMap::from([("/photo/current.nef".into(), 0)]);
        assert!(generation_priority_for(
            "/photo/current.nef",
            false,
            Some(&wanted)
        ));
        assert!(!generation_priority_for(
            "/photo/old.dng",
            true,
            Some(&wanted)
        ));
        assert!(generation_priority_for("/photo/initial.jpg", true, None));
    }

    #[test]
    fn empty_viewport_demotes_all_queued_generation() {
        let mut jobs = VecDeque::from([job(false), job(true)]);
        update_generation_visibility(&mut jobs, &HashMap::new());
        assert!(jobs.iter().all(|job| !job.5));
        assert_eq!(jobs.len(), 2);
    }

    #[test]
    fn visible_raw_precedes_prefetched_jpeg() {
        let mut raw = job(true);
        raw.0 = "/photo/visible.dng".into();
        let mut jpeg = job(false);
        jpeg.0 = "/photo/prefetch.jpg".into();
        let jobs = VecDeque::from([jpeg, raw]);
        assert_eq!(priority_dispatch_index(&jobs, 0, 0, 0), Some(1));
        assert_eq!(priority_dispatch_index(&jobs, 0, 1, 0), Some(0));
    }
}

fn update_generation_visibility(
    jobs: &mut VecDeque<PriorityRequest>,
    wanted: &HashMap<String, usize>,
) -> Vec<PriorityRequest> {
    let mut discarded = Vec::new();
    jobs.retain(|job| {
        let keep = !private_network_preview(&job.0) || job.5 || wanted.contains_key(&job.0);
        if !keep {
            discarded.push(job.clone());
        }
        keep
    });
    for job in jobs.iter_mut() {
        // A quality upgrade stays background work even when its photo is in
        // the viewport: the sharp thumbnail can wait, the thumbnail that the
        // user is scrolling towards cannot.
        if priority_upgrade_marked(&job.3) {
            job.5 = false;
            continue;
        }
        job.5 = wanted.contains_key(&job.0);
    }
    jobs.make_contiguous()
        .sort_by_key(|job| wanted.get(&job.0).copied().unwrap_or(usize::MAX));
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
    let wanted = VISIBLE_GENERATION_WANTED
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("generation viewport lock");
    generation_priority_for(path, requested, wanted.as_ref())
}

/// Generation and cache presentation must agree about the current viewport.
/// Drop queued remote reads from old viewports; local cache work can be demoted.
pub fn retain_visible_generation_requests(requests: &[crate::thumbnail_display::DisplayRequest]) {
    let wanted: HashMap<String, usize> = requests
        .iter()
        .enumerate()
        .map(|(index, request)| (request.source_path.clone(), index))
        .collect();
    let queue = priority_queue();
    let mut jobs = queue.0.lock().expect("priority queue lock");
    let mut current = VISIBLE_GENERATION_WANTED
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("generation viewport lock");
    if current.as_ref() != Some(&wanted) {
        release_discarded_generation(update_generation_visibility(&mut jobs, &wanted));
        NETWORK_PREFETCH_WANTED
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .expect("network prefetch lock")
            .clear();
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
                let (path, mtime, size_bytes, destination, queued_at, visible) = {
                    let (queue, wake) = &*queue;
                    let mut queue = queue.lock().expect("priority queue should not be poisoned");
                    let job = loop {
                        let dispatch = PRIORITY_DISPATCHES.load(Ordering::Relaxed);
                        if let Some(index) = priority_dispatch_index(
                            &queue,
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
                            break job;
                        }
                        queue = wake
                            .wait(queue)
                            .expect("priority queue should not be poisoned");
                    };
                    job
                };
                let _raw_guard = RawWorkerGuard(
                    is_raw(&path) && !is_nikon_raw(&path),
                    is_nikon_raw(&path),
                    queue.clone(),
                );
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_THUMBNAIL queue_wait kind={} elapsed_us={} uri={path}",
                        if visible { "visible" } else { "prefetch" },
                        queued_at.elapsed().as_micros()
                    );
                }
                let started = std::time::Instant::now();
                let failure_marker = destination.with_extension("failed");
                let _pending_guard = PendingGuard(destination.clone());
                let deadline = started + PRIORITY_HANDOFF_TIMEOUT;
                crate::thumbnail_trace::event(&path, "generation_worker", "start");
                while (!destination.is_file() || priority_upgrade_marked(&destination))
                    && !known_decode_failure(&path, &destination)
                {
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
                clear_priority_upgrade(&destination);
                if destination.is_file() {
                    crate::thumbnail_trace::event(&path, "generation_complete", "cache_ready");
                    if let Ok(mut completions) = PRIORITY_COMPLETIONS
                        .get_or_init(|| Mutex::new(Vec::new()))
                        .lock()
                    {
                        completions.push(PathBuf::from(&path));
                    }
                }
                if !destination.is_file() {
                    crate::thumbnail_trace::event(
                        &path,
                        "generation_complete",
                        if known_decode_failure(&path, &destination) {
                            "decode_failure"
                        } else {
                            "no_cache_before_timeout_or_error"
                        },
                    );
                    // The upgrade did not produce a file, so the photo may
                    // ask for it again later.
                    forget_quality_upgrade_attempt(&destination);
                }
            });
        }
        queue
    })
}

fn priority_dispatch_index(
    jobs: &VecDeque<PriorityRequest>,
    dispatch: usize,
    active_raw: usize,
    active_nef: usize,
) -> Option<usize> {
    // NEF embedded previews must not wait behind DNG sensor recovery.
    // At most two NEFs and one other RAW occupy the four-worker pool,
    // reserving at least one worker for visible JPEGs and other formats.
    let eligible = |job: &PriorityRequest| {
        if is_nikon_raw(&job.0) {
            active_nef < 2
        } else {
            !is_raw(&job.0) || active_raw == 0
        }
    };
    let select = |predicate: &dyn Fn(&PriorityRequest) -> bool| {
        if dispatch % (PRIORITY_NEWEST_DISPATCHES + 1) == PRIORITY_NEWEST_DISPATCHES {
            jobs.iter().rposition(predicate)
        } else {
            jobs.iter().position(predicate)
        }
    };
    let ordinary = |visible: bool| {
        select(&|job| job.5 == visible && is_jpeg(&job.0))
            .or_else(|| {
                select(&|job| {
                    job.5 == visible && (!is_raw(&job.0) || is_nikon_raw(&job.0)) && eligible(job)
                })
            })
            .or_else(|| select(&|job| job.5 == visible && eligible(job)))
    };
    ordinary(true).or_else(|| ordinary(false))
}
