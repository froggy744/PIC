use image::RgbaImage;
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

const CACHE_BUDGET_BYTES: usize = 128 * 1024 * 1024;
const PENDING_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const PENDING_JOB_LIMIT: usize = 18;
const QUEUED_JOB_LIMIT: usize = 16;
const WORKER_COUNT: usize = 2;
const CACHE_ALGORITHM_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct MaskCacheKey {
    presentation_revision: String,
    source_width: u32,
    source_height: u32,
    tile_width: u32,
    tile_height: u32,
    scale_factor: u32,
    render_width: u32,
    render_height: u32,
    crop_x: i32,
    crop_y: i32,
    algorithm_version: u8,
}

impl MaskCacheKey {
    pub(super) fn new(
        presentation_revision: impl Into<String>,
        source_width: u32,
        source_height: u32,
        tile_width: u32,
        tile_height: u32,
        scale_factor: u32,
    ) -> Option<Self> {
        let (render_width, render_height, crop_x, crop_y) =
            gtk_cover_geometry(source_width, source_height, tile_width, tile_height)?;
        tile_width.checked_mul(scale_factor)?;
        tile_height.checked_mul(scale_factor)?;
        Some(Self {
            presentation_revision: presentation_revision.into(),
            source_width,
            source_height,
            tile_width,
            tile_height,
            scale_factor,
            render_width,
            render_height,
            crop_x,
            crop_y,
            algorithm_version: CACHE_ALGORITHM_VERSION,
        })
    }

    fn output_dimensions(&self) -> Option<(u32, u32)> {
        Some((
            self.tile_width.checked_mul(self.scale_factor)?,
            self.tile_height.checked_mul(self.scale_factor)?,
        ))
    }

    pub(super) fn output_size(&self) -> Option<(u32, u32)> {
        self.output_dimensions()
    }

    fn output_bytes(&self) -> Option<usize> {
        let (width, height) = self.output_dimensions()?;
        (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)
    }
}

#[derive(Default)]
struct MaskCache {
    entries: VecDeque<(MaskCacheKey, glib::Bytes)>,
    bytes: usize,
    budget: usize,
    evictions: u64,
}

impl MaskCache {
    fn new(budget: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            budget,
            evictions: 0,
        }
    }

    fn get(&mut self, key: &MaskCacheKey) -> Option<glib::Bytes> {
        let index = self
            .entries
            .iter()
            .position(|(entry_key, _)| entry_key == key)?;
        let entry = self.entries.remove(index)?;
        let bytes = entry.1.clone();
        self.entries.push_back(entry);
        Some(bytes)
    }

    fn insert(&mut self, key: MaskCacheKey, data: glib::Bytes) -> bool {
        let size = data.len();
        if size > self.budget {
            return false;
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|(entry_key, _)| entry_key == &key)
        {
            if let Some((_, old)) = self.entries.remove(index) {
                self.bytes = self.bytes.saturating_sub(old.len());
            }
        }
        while self.bytes.saturating_add(size) > self.budget {
            let Some((_, oldest)) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(oldest.len());
            self.evictions = self.evictions.saturating_add(1);
        }
        self.bytes = self.bytes.saturating_add(size);
        self.entries.push_back((key, data));
        true
    }

    fn bytes(&self) -> usize {
        self.bytes
    }
}

#[derive(Clone)]
struct MaskJob {
    key: MaskCacheKey,
    source: glib::Bytes,
    generation: u64,
    queued_at: Instant,
    reserved_bytes: usize,
}

pub(super) struct MaskResult {
    pub(super) key: MaskCacheKey,
    pub(super) pixels: glib::Bytes,
    pub(super) width: u32,
    pub(super) height: u32,
    generation: u64,
    reserved_bytes: usize,
}

#[derive(Default)]
struct QueueState {
    jobs: VecDeque<MaskJob>,
    ready: VecDeque<MaskResult>,
    pending: HashSet<(MaskCacheKey, u64)>,
    visible_keys: HashSet<MaskCacheKey>,
    active_keys: HashSet<MaskCacheKey>,
    reserved_bytes: usize,
    peak_reserved_bytes: usize,
}

type Queue = Arc<(Mutex<QueueState>, Condvar)>;

static MASK_CACHE: OnceLock<Mutex<MaskCache>> = OnceLock::new();
static MASK_QUEUE: OnceLock<Queue> = OnceLock::new();
static MASK_CURRENT_GENERATION: AtomicU64 = AtomicU64::new(0);
static MASK_VIEW_GEOMETRY: OnceLock<Mutex<Option<(bool, i32, i32, i32, i32)>>> = OnceLock::new();
static MASK_WORKERS_STARTED: AtomicBool = AtomicBool::new(false);
static MASK_HITS: AtomicU64 = AtomicU64::new(0);
static MASK_MISSES: AtomicU64 = AtomicU64::new(0);
static MASK_CACHE_EVICTIONS: AtomicU64 = AtomicU64::new(0);
static MASK_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static MASK_STALE_DISCARDS: AtomicU64 = AtomicU64::new(0);
static MASK_QUEUE_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static MASK_WORKER_US: AtomicU64 = AtomicU64::new(0);
static MASK_QUEUE_LATENCY_US: AtomicU64 = AtomicU64::new(0);
static MASK_QUEUE_SAMPLES: AtomicU64 = AtomicU64::new(0);
static MASK_QUEUE_LATENCY_MAX_US: AtomicU64 = AtomicU64::new(0);
static MASK_TEXTURE_MAIN_US: AtomicU64 = AtomicU64::new(0);
static MASK_UPLOAD_ESTIMATE_BYTES: AtomicU64 = AtomicU64::new(0);
static MASK_LAST_TRACE: OnceLock<Mutex<Instant>> = OnceLock::new();

fn global_cache() -> &'static Mutex<MaskCache> {
    MASK_CACHE.get_or_init(|| Mutex::new(MaskCache::new(CACHE_BUDGET_BYTES)))
}

fn global_queue() -> Queue {
    MASK_QUEUE
        .get_or_init(|| Arc::new((Mutex::new(QueueState::default()), Condvar::new())))
        .clone()
}

pub(super) fn cached_render_enabled() -> bool {
    std::env::var_os("PIC_HONEYCOMB_RENDER_MODE").is_some_and(|value| value == "cached")
}

pub(super) fn set_generation(generation: u64) {
    MASK_CURRENT_GENERATION.store(generation, Ordering::Release);
    let queue = global_queue();
    let (state, _) = &*queue;
    let Ok(mut state) = state.lock() else {
        return;
    };
    let mut retained = VecDeque::with_capacity(state.jobs.len());
    while let Some(job) = state.jobs.pop_front() {
        if job.generation != generation {
            state.reserved_bytes = state.reserved_bytes.saturating_sub(job.reserved_bytes);
            state.pending.remove(&(job.key, job.generation));
            MASK_STALE_DISCARDS.fetch_add(1, Ordering::Relaxed);
        } else {
            retained.push_back(job);
        }
    }
    state.jobs = retained;
    let mut ready = VecDeque::with_capacity(state.ready.len());
    while let Some(result) = state.ready.pop_front() {
        if result.generation != generation {
            state.reserved_bytes = state.reserved_bytes.saturating_sub(result.reserved_bytes);
            state.pending.remove(&(result.key, result.generation));
            MASK_STALE_DISCARDS.fetch_add(1, Ordering::Relaxed);
        } else {
            ready.push_back(result);
        }
    }
    state.ready = ready;
}

pub(super) fn update_view_geometry(
    enabled: bool,
    width: i32,
    height: i32,
    scale: i32,
    viewport_width: i32,
) -> u64 {
    let geometry = MASK_VIEW_GEOMETRY.get_or_init(|| Mutex::new(None));
    let Ok(mut previous) = geometry.lock() else {
        return current_generation();
    };
    let current = (enabled, width, height, scale, viewport_width);
    if previous.as_ref() != Some(&current) {
        *previous = Some(current);
        let generation = MASK_CURRENT_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
        set_generation(generation);
        return generation;
    }
    current_generation()
}

pub(super) fn update_visible_keys(
    visible_keys: HashSet<MaskCacheKey>,
    active_keys: HashSet<MaskCacheKey>,
) {
    let queue = global_queue();
    let (state, _) = &*queue;
    let Ok(mut state) = state.lock() else {
        return;
    };
    state.visible_keys = visible_keys;
    state.active_keys = active_keys;
    let mut retained = VecDeque::with_capacity(state.jobs.len());
    while let Some(job) = state.jobs.pop_front() {
        if !state.active_keys.contains(&job.key) {
            state.reserved_bytes = state.reserved_bytes.saturating_sub(job.reserved_bytes);
            state.pending.remove(&(job.key, job.generation));
            MASK_STALE_DISCARDS.fetch_add(1, Ordering::Relaxed);
        } else {
            retained.push_back(job);
        }
    }
    state.jobs = retained;
    let visible_keys = state.visible_keys.clone();
    state
        .jobs
        .make_contiguous()
        .sort_by_key(|job| !visible_keys.contains(&job.key));
}

pub(super) fn current_generation() -> u64 {
    MASK_CURRENT_GENERATION.load(Ordering::Acquire)
}

pub(super) fn cache_get(key: &MaskCacheKey) -> Option<glib::Bytes> {
    let result = global_cache().lock().ok()?.get(key);
    if result.is_some() {
        MASK_HITS.fetch_add(1, Ordering::Relaxed);
    } else {
        MASK_MISSES.fetch_add(1, Ordering::Relaxed);
    }
    result
}

pub(super) fn cache_insert(key: MaskCacheKey, pixels: glib::Bytes) -> bool {
    global_cache()
        .lock()
        .map(|mut cache| {
            let previous_evictions = cache.evictions;
            let inserted = cache.insert(key, pixels);
            MASK_CACHE_EVICTIONS.fetch_add(cache.evictions - previous_evictions, Ordering::Relaxed);
            inserted
        })
        .unwrap_or(false)
}

pub(super) struct MaskMetrics {
    pub(super) hits: u64,
    pub(super) misses: u64,
    pub(super) fallbacks: u64,
    pub(super) evictions: u64,
    pub(super) cache_bytes: usize,
    pub(super) pending_bytes: usize,
    pub(super) pending_peak_bytes: usize,
    pub(super) worker_us: u64,
    pub(super) queue_avg_us: u64,
    pub(super) queue_max_us: u64,
    pub(super) main_texture_us: u64,
    pub(super) estimated_upload_bytes: u64,
}

pub(super) fn metrics_snapshot() -> MaskMetrics {
    let cache_bytes = global_cache()
        .lock()
        .map(|cache| cache.bytes())
        .unwrap_or(0);
    let (pending_bytes, pending_peak_bytes) = global_queue()
        .0
        .lock()
        .map(|state| (state.reserved_bytes, state.peak_reserved_bytes))
        .unwrap_or_default();
    let queue_samples = MASK_QUEUE_SAMPLES.load(Ordering::Relaxed);
    MaskMetrics {
        hits: MASK_HITS.load(Ordering::Relaxed),
        misses: MASK_MISSES.load(Ordering::Relaxed),
        fallbacks: MASK_FALLBACKS.load(Ordering::Relaxed),
        evictions: MASK_CACHE_EVICTIONS.load(Ordering::Relaxed),
        cache_bytes,
        pending_bytes,
        pending_peak_bytes,
        worker_us: MASK_WORKER_US.load(Ordering::Relaxed),
        queue_avg_us: if queue_samples == 0 {
            0
        } else {
            MASK_QUEUE_LATENCY_US.load(Ordering::Relaxed) / queue_samples
        },
        queue_max_us: MASK_QUEUE_LATENCY_MAX_US.load(Ordering::Relaxed),
        main_texture_us: MASK_TEXTURE_MAIN_US.load(Ordering::Relaxed),
        estimated_upload_bytes: MASK_UPLOAD_ESTIMATE_BYTES.load(Ordering::Relaxed),
    }
}

pub(super) fn note_fallback() {
    MASK_FALLBACKS.fetch_add(1, Ordering::Relaxed);
}

pub(super) fn submit(key: MaskCacheKey, source: glib::Bytes, generation: u64) -> bool {
    if generation != current_generation() {
        MASK_STALE_DISCARDS.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    let Some(output_bytes) = key.output_bytes() else {
        MASK_QUEUE_REJECTIONS.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    let input_bytes = source.len();
    let Some(reserved_bytes) = input_bytes.checked_add(output_bytes) else {
        MASK_QUEUE_REJECTIONS.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    if reserved_bytes > PENDING_BUDGET_BYTES {
        MASK_QUEUE_REJECTIONS.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    ensure_workers();
    let queue = global_queue();
    let (state, wake) = &*queue;
    let Ok(mut state) = state.lock() else {
        return false;
    };
    let pending_key = (key.clone(), generation);
    if state.pending.contains(&pending_key) {
        return true;
    }
    if state.jobs.len() >= QUEUED_JOB_LIMIT
        || state.pending.len() >= PENDING_JOB_LIMIT
        || state.reserved_bytes.saturating_add(reserved_bytes) > PENDING_BUDGET_BYTES
    {
        MASK_QUEUE_REJECTIONS.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    state.pending.insert(pending_key);
    state.reserved_bytes += reserved_bytes;
    state.peak_reserved_bytes = state.peak_reserved_bytes.max(state.reserved_bytes);
    let job = MaskJob {
        key,
        source,
        generation,
        queued_at: Instant::now(),
        reserved_bytes,
    };
    if state.visible_keys.contains(&job.key) {
        let insertion = state
            .jobs
            .iter()
            .position(|queued| !state.visible_keys.contains(&queued.key))
            .unwrap_or(state.jobs.len());
        state.jobs.insert(insertion, job);
    } else {
        state.jobs.push_back(job);
    }
    wake.notify_one();
    true
}

fn ensure_workers() {
    if MASK_WORKERS_STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    for _ in 0..WORKER_COUNT {
        let queue = global_queue();
        std::thread::Builder::new()
            .name("pic-honeycomb-mask".into())
            .spawn(move || mask_worker(queue))
            .expect("failed to start Honeycomb mask worker");
    }
}

fn mask_worker(queue: Queue) {
    loop {
        let (state, wake) = &*queue;
        let job = {
            let mut state = match state.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            while state.jobs.is_empty() {
                state = match wake.wait(state) {
                    Ok(state) => state,
                    Err(_) => return,
                };
            }
            state.jobs.pop_front().unwrap()
        };

        if job.generation != current_generation() {
            finish_stale(&queue, &job);
            continue;
        }
        let queue_latency_us = job.queued_at.elapsed().as_micros() as u64;
        MASK_QUEUE_LATENCY_US.fetch_add(queue_latency_us, Ordering::Relaxed);
        MASK_QUEUE_SAMPLES.fetch_add(1, Ordering::Relaxed);
        MASK_QUEUE_LATENCY_MAX_US.fetch_max(queue_latency_us, Ordering::Relaxed);
        let started = Instant::now();
        let image = rasterize_hex_cover(
            job.source.as_ref(),
            job.key.source_width,
            job.key.source_height,
            job.key.tile_width,
            job.key.tile_height,
            job.key.scale_factor,
        );
        let worker_us = started.elapsed().as_micros() as u64;
        MASK_WORKER_US.fetch_add(worker_us, Ordering::Relaxed);
        let Some(image) = image else {
            finish_stale(&queue, &job);
            continue;
        };
        if job.generation != current_generation() {
            finish_stale(&queue, &job);
            continue;
        }
        let (width, height) = (image.width(), image.height());
        let pixels = glib::Bytes::from_owned(image.into_raw());
        if let Ok(mut state) = state.lock() {
            if job.generation == current_generation() {
                state.ready.push_back(MaskResult {
                    key: job.key,
                    pixels,
                    width,
                    height,
                    generation: job.generation,
                    reserved_bytes: job.reserved_bytes,
                });
            } else {
                state.reserved_bytes = state.reserved_bytes.saturating_sub(job.reserved_bytes);
                state.pending.remove(&(job.key, job.generation));
                MASK_STALE_DISCARDS.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn finish_stale(queue: &Queue, job: &MaskJob) {
    let (state, _) = &**queue;
    if let Ok(mut state) = state.lock() {
        state.reserved_bytes = state.reserved_bytes.saturating_sub(job.reserved_bytes);
        state.pending.remove(&(job.key.clone(), job.generation));
    }
    MASK_STALE_DISCARDS.fetch_add(1, Ordering::Relaxed);
}

pub(super) fn take_ready(generation: u64) -> Vec<MaskResult> {
    let queue = global_queue();
    let (state, _) = &*queue;
    let Ok(mut state) = state.lock() else {
        return Vec::new();
    };
    let mut ready = Vec::new();
    while let Some(result) = state.ready.pop_front() {
        state.reserved_bytes = state.reserved_bytes.saturating_sub(result.reserved_bytes);
        state
            .pending
            .remove(&(result.key.clone(), result.generation));
        if result.generation == generation && result.generation == current_generation() {
            ready.push(result);
        } else {
            MASK_STALE_DISCARDS.fetch_add(1, Ordering::Relaxed);
        }
    }
    ready
}

pub(super) fn result_parts(result: MaskResult) -> (MaskCacheKey, glib::Bytes, u32, u32) {
    (result.key, result.pixels, result.width, result.height)
}

pub(super) fn record_main_texture_creation(micros: u64, bytes: usize) {
    MASK_TEXTURE_MAIN_US.fetch_add(micros, Ordering::Relaxed);
    MASK_UPLOAD_ESTIMATE_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}

pub(super) fn log_metrics_if_due() {
    if std::env::var_os("PIC_HONEYCOMB_MASK_TRACE").is_none() {
        return;
    }
    let now = Instant::now();
    let last = MASK_LAST_TRACE.get_or_init(|| Mutex::new(now));
    let Ok(mut last) = last.lock() else {
        return;
    };
    if now.duration_since(*last) < Duration::from_secs(5) {
        return;
    }
    *last = now;
    let cache_bytes = global_cache()
        .lock()
        .map(|cache| cache.bytes())
        .unwrap_or(0);
    let (pending_bytes, pending_peak, pending_jobs) = global_queue()
        .0
        .lock()
        .map(|state| {
            (
                state.reserved_bytes,
                state.peak_reserved_bytes,
                state.pending.len(),
            )
        })
        .unwrap_or_default();
    let queue_samples = MASK_QUEUE_SAMPLES.load(Ordering::Relaxed);
    let queue_average_us = if queue_samples == 0 {
        0
    } else {
        MASK_QUEUE_LATENCY_US.load(Ordering::Relaxed) / queue_samples
    };
    let rss_hwm_kib = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmHWM:")
                    .and_then(|value| value.split_whitespace().next())
                    .and_then(|value| value.parse::<u64>().ok())
            })
        })
        .unwrap_or_default();
    let hits = MASK_HITS.load(Ordering::Relaxed);
    let misses = MASK_MISSES.load(Ordering::Relaxed);
    let cache_hit_rate = if hits + misses == 0 {
        0.0
    } else {
        hits as f64 * 100.0 / (hits + misses) as f64
    };
    eprintln!(
        "PIC_HONEYCOMB_MASK cache_hits={} misses={} hit_rate_pct={:.1} fallbacks={} stale={} queue_rejected={} pending_jobs={} pending_bytes={} pending_peak={} cache_bytes={} rss_hwm_kib={} worker_total_us={} queue_avg_us={} queue_max_us={} main_texture_total_us={} upload_estimate_bytes={}",
        hits,
        misses,
        cache_hit_rate,
        MASK_FALLBACKS.load(Ordering::Relaxed),
        MASK_STALE_DISCARDS.load(Ordering::Relaxed),
        MASK_QUEUE_REJECTIONS.load(Ordering::Relaxed),
        pending_jobs,
        pending_bytes,
        pending_peak,
        cache_bytes,
        rss_hwm_kib,
        MASK_WORKER_US.load(Ordering::Relaxed),
        queue_average_us,
        MASK_QUEUE_LATENCY_MAX_US.load(Ordering::Relaxed),
        MASK_TEXTURE_MAIN_US.load(Ordering::Relaxed),
        MASK_UPLOAD_ESTIMATE_BYTES.load(Ordering::Relaxed),
    );
}

/// GTK 4.12+ GtkPicture's Cover snapshot rounds the rendered paintable up to
/// integer logical dimensions, then centers it with C integer truncation.
/// Keep this separate so those rounding rules can be unit-tested directly.
fn gtk_cover_geometry(
    src_w: u32,
    src_h: u32,
    tile_w: u32,
    tile_h: u32,
) -> Option<(u32, u32, i32, i32)> {
    if src_w == 0 || src_h == 0 || tile_w == 0 || tile_h == 0 {
        return None;
    }

    let ratio = src_w as f64 / src_h as f64;
    let tile_ratio = tile_w as f64 / tile_h as f64;
    let (render_w, render_h) = if ratio > tile_ratio {
        ((tile_h as f64 * ratio).ceil(), (tile_h as f64).ceil())
    } else {
        ((tile_w as f64).ceil(), (tile_w as f64 / ratio).ceil())
    };
    if !render_w.is_finite()
        || !render_h.is_finite()
        || render_w > i32::MAX as f64
        || render_h > i32::MAX as f64
    {
        return None;
    }

    let render_w = render_w as u32;
    let render_h = render_h as u32;
    // gtkpicture.c computes x = (width - w) / 2 and
    // y = floor(height - h) / 2, assigning each double to an int.
    // Rust's float-to-int cast truncates toward zero, matching C here.
    let x = ((tile_w as f64 - render_w as f64) / 2.0) as i32;
    let y = ((tile_h as f64 - render_h as f64).floor() / 2.0) as i32;
    Some((render_w, render_h, x, y))
}

/// Produce the `GtkPicture(ContentFit::Cover)` result clipped by the same
/// pointy-top hexagon used by the live Honeycomb snapshot path. Sampling uses
/// bilinear filtering in source texel coordinates; mask coverage is estimated
/// with an 8x8 grid per output pixel for antialiased edges.
pub(super) fn rasterize_hex_cover(
    source: &[u8],
    src_w: u32,
    src_h: u32,
    tile_w: u32,
    tile_h: u32,
    scale_factor: u32,
) -> Option<RgbaImage> {
    let expected_len = (src_w as usize)
        .checked_mul(src_h as usize)?
        .checked_mul(4)?;
    if src_w == 0
        || src_h == 0
        || source.len() != expected_len
        || tile_w == 0
        || tile_h == 0
        || scale_factor == 0
    {
        return None;
    }
    let (render_w, render_h, offset_x, offset_y) =
        gtk_cover_geometry(src_w, src_h, tile_w, tile_h)?;
    let out_w = tile_w.checked_mul(scale_factor)?;
    let out_h = tile_h.checked_mul(scale_factor)?;
    let mut output = RgbaImage::new(out_w, out_h);
    let sample_grid = 8u32;
    let sample_count = (sample_grid * sample_grid) as f64;
    let vertices = super::honeycomb::vertices(tile_w as f64, tile_h as f64);

    for py in 0..out_h {
        for px in 0..out_w {
            let logical_x = (px as f64 + 0.5) / scale_factor as f64;
            let logical_y = (py as f64 + 0.5) / scale_factor as f64;
            let u = (logical_x - offset_x as f64) / render_w as f64;
            let v = (logical_y - offset_y as f64) / render_h as f64;
            let color = sample_bilinear(source, src_w, src_h, u, v);

            let mut covered = 0u32;
            for sy in 0..sample_grid {
                for sx in 0..sample_grid {
                    let x =
                        (px as f64 + (sx as f64 + 0.5) / sample_grid as f64) / scale_factor as f64;
                    let y =
                        (py as f64 + (sy as f64 + 0.5) / sample_grid as f64) / scale_factor as f64;
                    if point_in_hex(&vertices, x, y) {
                        covered += 1;
                    }
                }
            }
            let alpha = ((color[3] as f64 * covered as f64 / sample_count).round()) as u8;
            output.put_pixel(px, py, image::Rgba([color[0], color[1], color[2], alpha]));
        }
    }
    Some(output)
}

fn sample_bilinear(source: &[u8], width: u32, height: u32, u: f64, v: f64) -> [u8; 4] {
    // GSK texture nodes use normalized texture coordinates. Mapping the
    // destination pixel center to texel space therefore subtracts 0.5.
    let x = (u * width as f64 - 0.5).clamp(0.0, width.saturating_sub(1) as f64);
    let y = (v * height as f64 - 0.5).clamp(0.0, height.saturating_sub(1) as f64);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let fx = x - x0 as f64;
    let fy = y - y0 as f64;
    let at = |px: u32, py: u32, channel: usize| {
        source[((py * width + px) * 4) as usize + channel] as f64
    };
    let mut result = [0; 4];
    for (channel, value) in result.iter_mut().enumerate() {
        let top = at(x0, y0, channel) * (1.0 - fx) + at(x1, y0, channel) * fx;
        let bottom = at(x0, y1, channel) * (1.0 - fx) + at(x1, y1, channel) * fx;
        *value = (top * (1.0 - fy) + bottom * fy).round() as u8;
    }
    result
}

fn point_in_hex(vertices: &[(f64, f64); 6], x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut previous = vertices.len() - 1;
    for current in 0..vertices.len() {
        let (xi, yi) = vertices[current];
        let (xj, yj) = vertices[previous];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::prelude::*;
    use gtk4 as gtk;

    fn solid_rgba(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
        pixel.repeat((width * height) as usize)
    }

    #[test]
    fn cover_raster_has_expected_dimensions_and_center_colour() {
        let source = solid_rgba(7, 3, [30, 90, 180, 255]);
        let output = rasterize_hex_cover(&source, 7, 3, 9, 7, 1).unwrap();
        assert_eq!((output.width(), output.height()), (9, 7));
        assert_eq!(output.get_pixel(4, 3).0, [30, 90, 180, 255]);
    }

    #[test]
    fn gtk_cover_rounding_matches_picture_snapshot_dimensions_and_offsets() {
        // GtkPicture 4.22 rounds the oversized paintable up to 17 logical px;
        // centering the 9px viewport gives -4, after truncation toward zero.
        assert_eq!(gtk_cover_geometry(7, 3, 9, 7), Some((17, 7, -4, 0)));
        assert_eq!(gtk_cover_geometry(3, 7, 9, 7), Some((9, 21, 0, -7)));
        assert_eq!(gtk_cover_geometry(5, 5, 9, 7), Some((9, 9, 0, -1)));
    }

    #[test]
    fn cover_crop_centres_landscape_portrait_square_and_extreme_sources() {
        for (src_w, src_h) in [(31, 9), (9, 31), (15, 15), (101, 3), (3, 101)] {
            let pixels = solid_rgba(src_w, src_h, [100, 40, 20, 255]);
            let output = rasterize_hex_cover(&pixels, src_w, src_h, 17, 19, 1).unwrap();
            assert_eq!((output.width(), output.height()), (17, 19));
            assert_eq!(output.get_pixel(8, 9).0, [100, 40, 20, 255]);
        }
    }

    #[test]
    fn fractional_device_scale_keeps_hex_edges_antialiased() {
        let pixels = solid_rgba(13, 11, [20, 80, 140, 255]);
        let output = rasterize_hex_cover(&pixels, 13, 11, 17, 19, 2).unwrap();
        assert_eq!((output.width(), output.height()), (34, 38));
        assert_eq!(output.get_pixel(0, 0)[3], 0);
        assert_eq!(output.get_pixel(17, 19)[3], 255);
        assert!(output.pixels().any(|pixel| pixel[3] > 0 && pixel[3] < 255));
    }

    #[test]
    fn rejects_mismatched_source_buffers_and_zero_dimensions() {
        assert!(rasterize_hex_cover(&[0; 8], 2, 2, 8, 8, 1).is_none());
        assert!(rasterize_hex_cover(&[], 0, 2, 8, 8, 1).is_none());
        assert!(rasterize_hex_cover(&[], 2, 2, 8, 8, 0).is_none());
    }

    #[test]
    fn mask_cache_key_tracks_presentation_revision_geometry_and_scale() {
        let key = MaskCacheKey::new("photo-v4", 900, 600, 180, 208, 1).unwrap();
        assert_ne!(
            key,
            MaskCacheKey::new("photo-v5", 900, 600, 180, 208, 1).unwrap()
        );
        assert_ne!(
            key,
            MaskCacheKey::new("photo-v4", 600, 900, 180, 208, 1).unwrap()
        );
        assert_ne!(
            key,
            MaskCacheKey::new("photo-v4", 900, 600, 219, 253, 1).unwrap()
        );
        assert_ne!(
            key,
            MaskCacheKey::new("photo-v4", 900, 600, 180, 208, 2).unwrap()
        );
    }

    #[test]
    fn mask_cache_lru_never_exceeds_its_byte_budget() {
        let mut cache = MaskCache::new(8);
        let first = MaskCacheKey::new("first", 2, 2, 1, 1, 1).unwrap();
        let second = MaskCacheKey::new("second", 2, 2, 1, 1, 1).unwrap();
        assert!(cache.insert(first.clone(), glib::Bytes::from_owned(vec![1; 6])));
        assert!(cache.insert(second.clone(), glib::Bytes::from_owned(vec![2; 6])));
        assert_eq!(cache.bytes(), 6);
        assert_eq!(cache.evictions, 1);
        assert!(cache.get(&first).is_none());
        assert_eq!(cache.get(&second).unwrap().as_ref(), &[2; 6]);
        assert!(!cache.insert(first.clone(), glib::Bytes::from_owned(vec![3; 12])));
        assert_eq!(cache.bytes(), 6);
        assert!(cache.get(&second).is_some());
    }

    #[test]
    #[ignore = "requires a native GTK display; captures GSK versus CPU-raster comparisons"]
    fn gtk_cover_and_honeycomb_mask_visual_comparison() {
        gtk::init().unwrap();
        let output_dir = std::env::var_os("PIC_HONEYCOMB_RASTER_COMPARE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("pic-honeycomb-raster-compare"));
        std::fs::create_dir_all(&output_dir).unwrap();
        let host = gtk::Fixed::new();
        let window = gtk::Window::builder().child(&host).build();

        let mut reports = Vec::new();
        let mut benchmark_nodes = None;
        for (tile_w, tile_h) in [(100, 116), (180, 208), (300, 346)] {
            for (case, (src_w, src_h)) in [(37, 13), (13, 37), (23, 23), (53, 5), (5, 53), (31, 17)]
                .into_iter()
                .enumerate()
            {
                let mut source = RgbaImage::new(src_w, src_h);
                for (x, y, pixel) in source.enumerate_pixels_mut() {
                    *pixel = image::Rgba([
                        ((x * 251 / src_w.max(1)) & 255) as u8,
                        ((y * 239 / src_h.max(1)) & 255) as u8,
                        (((x + y) * 197 / (src_w + src_h).max(1)) & 255) as u8,
                        255,
                    ]);
                }
                let source_bytes = glib::Bytes::from_owned(source.clone().into_raw());
                let source_texture = gtk::gdk::MemoryTexture::new(
                    src_w as i32,
                    src_h as i32,
                    gtk::gdk::MemoryFormat::R8g8b8a8,
                    &source_bytes,
                    src_w as usize * 4,
                );
                let picture = gtk::Picture::for_paintable(&source_texture);
                picture.set_content_fit(gtk::ContentFit::Cover);
                picture.set_can_shrink(true);
                picture.set_size_request(tile_w, tile_h);
                host.put(&picture, 0.0, 0.0);
                window.set_default_size(tile_w, tile_h);
                window.present();
                for _ in 0..40 {
                    while glib::MainContext::default().iteration(false) {}
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                assert_eq!((picture.width(), picture.height()), (tile_w, tile_h));

                let bounds = gtk::graphene::Rect::new(0.0, 0.0, tile_w as f32, tile_h as f32);
                let mask = gtk::Snapshot::new();
                let cr = mask.append_cairo(&bounds);
                let vertices = super::super::honeycomb::vertices(tile_w as f64, tile_h as f64);
                cr.move_to(vertices[0].0, vertices[0].1);
                for &(x, y) in &vertices[1..] {
                    cr.line_to(x, y);
                }
                cr.close_path();
                cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                cr.fill().unwrap();
                drop(cr);

                let normal_snapshot = gtk::Snapshot::new();
                normal_snapshot.push_mask(gtk::gsk::MaskMode::Alpha);
                normal_snapshot.append_node(&mask.to_node().unwrap());
                normal_snapshot.pop();
                gtk::WidgetPaintable::new(Some(&picture)).snapshot(
                    &normal_snapshot,
                    tile_w as f64,
                    tile_h as f64,
                );
                normal_snapshot.pop();
                let normal_node = normal_snapshot.to_node().unwrap();

                let raster_started = std::time::Instant::now();
                let cached = rasterize_hex_cover(
                    source.as_raw(),
                    src_w,
                    src_h,
                    tile_w as u32,
                    tile_h as u32,
                    1,
                )
                .unwrap();
                let cached_bytes = glib::Bytes::from_owned(cached.into_raw());
                let cached_texture = gtk::gdk::MemoryTexture::new(
                    tile_w,
                    tile_h,
                    gtk::gdk::MemoryFormat::R8g8b8a8,
                    &cached_bytes,
                    tile_w as usize * 4,
                );
                let cached_snapshot = gtk::Snapshot::new();
                cached_snapshot.append_texture(&cached_texture, &bounds);
                let cached_node = cached_snapshot.to_node().unwrap();
                let renderer = window.renderer().unwrap();
                if tile_w == 180 && case == 5 {
                    benchmark_nodes =
                        Some((normal_node.clone(), cached_node.clone(), renderer.clone()));
                }
                let normal_path = output_dir.join(format!("normal-{tile_w}-{case}.png"));
                let cached_path = output_dir.join(format!("cached-{tile_w}-{case}.png"));
                renderer
                    .render_texture(&normal_node, Some(&bounds))
                    .save_to_png(&normal_path)
                    .unwrap();
                renderer
                    .render_texture(&cached_node, Some(&bounds))
                    .save_to_png(&cached_path)
                    .unwrap();
                let normal = image::open(&normal_path).unwrap().to_rgba8();
                let cached = image::open(&cached_path).unwrap().to_rgba8();
                let mut sum = 0u64;
                let mut max = 0u8;
                let mut differing = 0usize;
                for (a, b) in normal.pixels().zip(cached.pixels()) {
                    for channel in 0..4 {
                        let av = if channel == 3 {
                            a[channel]
                        } else {
                            ((a[channel] as u16 * a[3] as u16 + 127) / 255) as u8
                        };
                        let bv = if channel == 3 {
                            b[channel]
                        } else {
                            ((b[channel] as u16 * b[3] as u16 + 127) / 255) as u8
                        };
                        let delta = av.abs_diff(bv);
                        sum += delta as u64;
                        max = max.max(delta);
                        differing += usize::from(delta > 2);
                    }
                }
                let samples = (normal.width() * normal.height() * 4) as u64;
                let mean_error = sum as f64 / samples as f64;
                assert!(mean_error < 0.5, "visual diff too large: {mean_error}");
                assert!(max <= 16, "visible channel diff too large: {max}");
                reports.push(format!(
                    "tile={tile_w}x{tile_h} source={src_w}x{src_h} premul_mae={mean_error:.3} premul_max={max} differing_channels_gt2={differing} raster_us={}",
                    raster_started.elapsed().as_micros()
                ));
                host.remove(&picture);
            }
        }
        if let Some((normal, cached, renderer)) = benchmark_nodes {
            let render = |node: &gtk::gsk::RenderNode| {
                std::hint::black_box(renderer.render_texture(node, None));
            };
            for _ in 0..12 {
                render(&normal);
                render(&cached);
            }
            let mut normal_us = Vec::with_capacity(100);
            let mut cached_us = Vec::with_capacity(100);
            for i in 0..100 {
                let cached_first = i % 2 == 1;
                let (first, second) = if cached_first {
                    (&cached, &normal)
                } else {
                    (&normal, &cached)
                };
                let start = std::time::Instant::now();
                render(first);
                let first_us = start.elapsed().as_micros() as u64;
                let start = std::time::Instant::now();
                render(second);
                let second_us = start.elapsed().as_micros() as u64;
                if cached_first {
                    cached_us.push(first_us);
                    normal_us.push(second_us);
                } else {
                    normal_us.push(first_us);
                    cached_us.push(second_us);
                }
            }
            let summarize = |samples: &mut Vec<u64>| {
                samples.sort_unstable();
                let at = |p: usize| samples[(samples.len() - 1) * p / 100] as f64 / 1000.0;
                (at(50), at(95), at(99))
            };
            let normal_stats = summarize(&mut normal_us);
            let cached_stats = summarize(&mut cached_us);
            eprintln!(
                "Honeycomb Vulkan offscreen render_texture ms (100 samples, alternating): normal median/p95/p99={:.3}/{:.3}/{:.3}, cpu-masked={:.3}/{:.3}/{:.3}",
                normal_stats.0,
                normal_stats.1,
                normal_stats.2,
                cached_stats.0,
                cached_stats.1,
                cached_stats.2
            );
        }
        window.close();
        eprintln!(
            "Honeycomb GSK/CPU raster comparison: {}",
            reports.join("; ")
        );
    }
}
