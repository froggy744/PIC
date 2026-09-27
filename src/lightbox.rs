use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Condvar, Mutex, OnceLock,
};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk4 as gtk;

use crate::photo_object::PhotoObject;

/// A cooperative decode used to be gated behind a single-permit Mutex, which
/// serialized every navigation: a new photo's decode thread would block on
/// the mutex until the *previous* photo's full decode finished, even though
/// that previous result was already stale and about to be discarded. That
/// made rapid navigation (arrow keys / scroll) as slow as decoding every
/// skipped photo in sequence.
///
/// A small counting semaphore keeps a handful of decodes in flight at once
/// (so we still don't spawn unbounded full-resolution RAW decodes if someone
/// holds an arrow key down) without forcing them to run one at a time.
struct DecodeSemaphore {
    available: Mutex<usize>,
    condvar: Condvar,
}

impl DecodeSemaphore {
    fn new(permits: usize) -> Self {
        Self {
            available: Mutex::new(permits),
            condvar: Condvar::new(),
        }
    }

    fn acquire_while(&self, keep_running: impl Fn() -> bool) -> Option<DecodePermit<'_>> {
        let mut available = self.available.lock().unwrap();
        while *available == 0 {
            if !keep_running() {
                return None;
            }
            let (next, _) = self
                .condvar
                .wait_timeout(available, Duration::from_millis(5))
                .unwrap();
            available = next;
        }
        if !keep_running() {
            return None;
        }
        *available -= 1;
        Some(DecodePermit { semaphore: self })
    }
}

struct DecodePermit<'a> {
    semaphore: &'a DecodeSemaphore,
}

impl Drop for DecodePermit<'_> {
    fn drop(&mut self) {
        let mut available = self.semaphore.available.lock().unwrap();
        *available += 1;
        self.semaphore.condvar.notify_one();
    }
}

const MAX_CONCURRENT_VIEWER_DECODES: usize = 3;
const VIEWER_PADDING: i32 = 0;
/// Highest magnification the continuous zoom paths allow, in native-image
/// terms. The InfoBar slider's right endpoint is this multiple of 1:1.
pub(crate) const LIGHTBOX_MAX_ZOOM_FACTOR: f64 = 2.0;
/// `applied_native_scale` sentinel: no manual zoom has been written since the
/// last reset, so any scale a control reports has to be applied. Native scales
/// are never negative, so this can never collide with a real value.
const APPLIED_SCALE_UNKNOWN: f64 = -1.0;

/// The single writer of the viewer's internal zoom state.
///
/// `applied_native_scale` records the last native-image scale the geometry path
/// applied, so a control that reports the same value again can be dropped
/// before any metadata, fit or layout work happens. Every path that changes
/// `zoom` therefore has to invalidate that record through here: a stale one
/// makes the next slider request look like a repeat and get dropped, leaving
/// the image at its old magnification while the slider shows the new one. The
/// fit-relative `zoom` can only be turned into a native scale where the fit
/// maths runs, so a change invalidates the record instead of recomputing it.
fn set_zoom_state(zoom: &Cell<f64>, applied_native_scale: &Cell<f64>, value: f64) {
    zoom.set(value);
    // Fit is the native scale 0.0 and is therefore already exact. Every other
    // value, including the reserved negative 1:1 mode, has to be recomputed by
    // the path that knows the fit scale.
    applied_native_scale.set(if value == 0.0 {
        0.0
    } else {
        APPLIED_SCALE_UNKNOWN
    });
}
static VIEWER_DECODE_GATE: OnceLock<DecodeSemaphore> = OnceLock::new();
// Nonzero while the selected photo has a foreground decode outstanding.
// Prefetch must not use decode slots while the user is waiting for a photo.
static VIEWER_FOREGROUND_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ViewerRequestKey {
    path: String,
    mtime: i64,
    size_bytes: i64,
    rotation: i32,
    edit_recipe: String,
    target_width: u32,
    target_height: u32,
}

struct ViewerDecodeResult {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

type ViewerDecodeOutcome = Result<Arc<ViewerDecodeResult>, Arc<str>>;

struct ViewerResultSlot {
    value: Mutex<Option<ViewerDecodeOutcome>>,
    wakers: Mutex<Vec<Waker>>,
}

impl ViewerResultSlot {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(None),
            wakers: Mutex::new(Vec::new()),
        })
    }

    fn send(&self, value: ViewerDecodeOutcome) {
        *self.value.lock().unwrap() = Some(value);
        for waker in self.wakers.lock().unwrap().drain(..) {
            waker.wake();
        }
    }

    fn wait(slot: Arc<Self>) -> ViewerResultFuture {
        ViewerResultFuture { slot }
    }

    fn ready(&self) -> bool {
        self.value.lock().unwrap().is_some()
    }
}

struct ViewerResultFuture {
    slot: Arc<ViewerResultSlot>,
}

impl Future for ViewerResultFuture {
    type Output = ViewerDecodeOutcome;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(result) = self.slot.value.lock().unwrap().as_ref() {
            return Poll::Ready(result.clone());
        }
        self.slot
            .wakers
            .lock()
            .unwrap()
            .push(context.waker().clone());
        if let Some(result) = self.slot.value.lock().unwrap().as_ref() {
            Poll::Ready(result.clone())
        } else {
            Poll::Pending
        }
    }
}

struct ViewerRequest {
    consumers: AtomicUsize,
    foreground: AtomicBool,
    result: Arc<ViewerResultSlot>,
}

impl ViewerRequest {
    fn has_consumers(&self) -> bool {
        self.consumers.load(Ordering::Acquire) != 0
    }
}

struct ViewerRequestLease {
    key: ViewerRequestKey,
    request: Arc<ViewerRequest>,
    cancelled: AtomicBool,
    released: AtomicBool,
}

impl ViewerRequestLease {
    fn new(key: ViewerRequestKey, request: Arc<ViewerRequest>, foreground: bool) -> Arc<Self> {
        request.consumers.fetch_add(1, Ordering::AcqRel);
        if foreground {
            request.foreground.store(true, Ordering::Release);
        }
        Arc::new(Self {
            key,
            request,
            cancelled: AtomicBool::new(false),
            released: AtomicBool::new(false),
        })
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.release();
    }

    fn release(&self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            if self.request.consumers.fetch_sub(1, Ordering::AcqRel) == 1
                && self.request.result.ready()
            {
                remove_finished_viewer_request(&self.key, &self.request);
            }
        }
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

enum ViewerRequestClaim {
    New,
    JoinedForeground,
    PromotedPrefetch,
}

static VIEWER_REQUESTS: OnceLock<Mutex<HashMap<ViewerRequestKey, Arc<ViewerRequest>>>> =
    OnceLock::new();

fn claim_viewer_request(
    key: &ViewerRequestKey,
    foreground: bool,
) -> (
    Arc<ViewerRequest>,
    Arc<ViewerRequestLease>,
    ViewerRequestClaim,
) {
    let requests = VIEWER_REQUESTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut requests = requests.lock().unwrap();
    if let Some(request) = requests.get(key).cloned() {
        let was_foreground = request.foreground.load(Ordering::Acquire);
        let lease = ViewerRequestLease::new(key.clone(), request.clone(), foreground);
        let claim = if foreground && !was_foreground {
            ViewerRequestClaim::PromotedPrefetch
        } else {
            ViewerRequestClaim::JoinedForeground
        };
        return (request, lease, claim);
    }
    let request = Arc::new(ViewerRequest {
        consumers: AtomicUsize::new(0),
        foreground: AtomicBool::new(foreground),
        result: ViewerResultSlot::new(),
    });
    requests.insert(key.clone(), request.clone());
    let lease = ViewerRequestLease::new(key.clone(), request.clone(), foreground);
    (request, lease, ViewerRequestClaim::New)
}

fn finish_viewer_request(
    key: &ViewerRequestKey,
    request: &Arc<ViewerRequest>,
    result: ViewerDecodeOutcome,
) {
    request.result.send(result);
    if !request.has_consumers() {
        remove_finished_viewer_request(key, request);
    }
}

fn remove_finished_viewer_request(key: &ViewerRequestKey, request: &Arc<ViewerRequest>) {
    let requests = VIEWER_REQUESTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut requests = requests.lock().unwrap();
    if requests
        .get(key)
        .is_some_and(|current| Arc::ptr_eq(current, request))
    {
        requests.remove(key);
    }
}

type PhotoChangedHandler = Rc<RefCell<Option<Box<dyn Fn(PhotoObject)>>>>;
type OneToOneSyncHandler = Rc<RefCell<Option<Box<dyn Fn(bool)>>>>;
type ZoomSyncHandler = Rc<RefCell<Option<Box<dyn Fn(f64)>>>>;
type ContextMenuHandler = Rc<RefCell<Option<Box<dyn Fn(PhotoObject, gtk::Widget, f64, f64)>>>>;
type CollectionNavigationHandler = Rc<RefCell<Option<Box<dyn Fn(i32)>>>>;

#[derive(Clone)]
struct NativeTextureCache {
    path: String,
    rotation: i32,
    edit_recipe: String,
    texture: gtk::gdk::MemoryTexture,
}

struct DisplayTextureCacheEntry {
    path: String,
    rotation: i32,
    edit_recipe: String,
    target_width: u32,
    target_height: u32,
    texture: gtk::gdk::MemoryTexture,
    // Approximate RGBA footprint, used by the byte budget below.
    bytes: usize,
}

type DisplayTextureCache = Rc<RefCell<VecDeque<DisplayTextureCacheEntry>>>;

// Full-size RAW/NEF viewer decodes can be expensive. Keep enough recently
// viewed display textures in RAM that stepping back through a burst is served
// from the cache instead of decoding again.
const DISPLAY_TEXTURE_CACHE_CAPACITY: usize = 32;
// Texture memory scales with the viewport, so a pure count is unsafe on large
// displays: a 4K viewer texture is ~33 MB. Cap total cached pixels as well and
// evict least-recently-used entries past either limit. At the default window
// size a texture is ~3.5 MB, so the count limit binds first and the budget only
// protects large/zoomed viewports.
const DISPLAY_TEXTURE_CACHE_BYTE_BUDGET: usize = 256 * 1024 * 1024;

#[derive(Default)]
struct WheelNavigationState {
    pending_target: Option<usize>,
    active_direction: i32,
}

impl WheelNavigationState {
    fn begin(&mut self, direction: i32) {
        self.pending_target = None;
        self.active_direction = direction;
    }

    fn queue_step(&mut self, current: usize, direction: i32, len: usize) -> Option<usize> {
        let base = self.pending_target.unwrap_or(current);
        let target = navigation_step(base, direction, len);
        (target != base).then(|| {
            self.pending_target = Some(target);
            target
        })
    }

    fn take_pending_target(&mut self) -> Option<usize> {
        self.active_direction = 0;
        self.pending_target.take()
    }

    fn cancel(&mut self) {
        self.pending_target = None;
        self.active_direction = 0;
    }
}

fn navigation_step(current: usize, direction: i32, len: usize) -> usize {
    if direction < 0 {
        current.saturating_sub(1)
    } else if direction > 0 && len > 0 {
        (current + 1).min(len - 1)
    } else {
        current
    }
}

pub struct Lightbox {
    pub root: gtk::Overlay,
    backdrop: gtk::Box,
    picture: gtk::Picture,
    picture_viewport: gtk::ScrolledWindow,
    photos: Rc<RefCell<Vec<PhotoObject>>>,
    index: Rc<Cell<usize>>,
    last_width: Rc<Cell<i32>>,
    last_height: Rc<Cell<i32>>,
    opening_fit_pending: Rc<Cell<bool>>,
    zoom: Rc<Cell<f64>>,
    zoom_before_one_to_one: Rc<Cell<f64>>,
    // Last native-image scale written by the manual zoom path, so a control
    // that reports the same value again can be dropped before any geometry,
    // metadata or layout work happens. Only `apply_manual_zoom_scale` knows the
    // real value; every other writer goes through `set_zoom_state`, which
    // invalidates it.
    applied_native_scale: Rc<Cell<f64>>,
    one_to_one_active: Rc<Cell<bool>>,
    native_texture: Rc<RefCell<Option<NativeTextureCache>>>,
    display_texture_cache: DisplayTextureCache,
    load_generation: Rc<Cell<u64>>,
    decode_cancel: Rc<RefCell<Option<Arc<ViewerRequestLease>>>>,
    key_navigation_ready: Rc<Cell<bool>>,
    wheel_navigation: Rc<RefCell<WheelNavigationState>>,
    photo_changed: PhotoChangedHandler,
    // Keeps the toolbar 1:1 toggle in sync when the lightbox changes the mode
    // itself (for example Ctrl+wheel leaves 1:1 for a manual zoom).
    one_to_one_sync: OneToOneSyncHandler,
    // Reports the visible photo scale in native-image terms: 0.0 is Fit,
    // 1.0 is 100% / 1:1, and values above 1.0 are magnified.
    zoom_sync: ZoomSyncHandler,
    context_menu: ContextMenuHandler,
    collection_navigation: CollectionNavigationHandler,
}

// Structural split only: included files remain in this module scope.
include!("lightbox/impl.rs");
include!("lightbox/render.rs");
include!("lightbox/tests.rs");
