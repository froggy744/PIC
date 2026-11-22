// Gallery thumbnail presentation module: getting the right thumbnail, at the
// right size, onto every visible gallery tile.
//
// Interface (what the rest of the app uses):
// - `Gallery::present_frame()` once per frame (gallery tick): keeps the decode
//   size in step with zoom, then delivers finished thumbnails.
// - `queue_visible_cached_tiles_async`, `apply_visible_cached_paintables` and
//   `prefetch_cached_tiles`, keyed by `GalleryScroller`, from the scroll pumps.
// - `thumbnail_prefetch_allowance()` for the settle prefetch budget.
//
// Behind it: the visible-tiles seam (`visible_presentation_tiles`, one adapter
// per surface), the decoded-thumbnail RAM cache, presentation targets, and
// delivery. Invariants are listed in AUDIT-INDEX.md, "Gallery thumbnail &
// scroll pipeline" (placement vs allocation, visible-queue replacement,
// prefetch must fit the cache).

/// The gallery scroller a per-frame thumbnail pump belongs to.
///
/// All Photos (`grid_scroll`, the `GtkGridView`) and Folder (`folder_scroll`,
/// the sectioned view or the legacy list) each run their own pump. A pump only
/// acts while its own surface is the one on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GalleryScroller {
    Photos,
    Folder,
}

impl GalleryScroller {
    /// How many visible-screens of photos prefetch reaches (ahead, behind).
    /// Folder reaches further: its sections make jumps between screens larger.
    fn prefetch_reach(self) -> (usize, usize) {
        match self {
            GalleryScroller::Photos => (5, 1),
            GalleryScroller::Folder => (6, 2),
        }
    }
}

/// Photo indexes to prefetch around the visible span `(first, last)`, in
/// priority order: `reach.0` screens ahead in the scroll `direction` (nearest
/// first), then `reach.1` screens behind. A "screen" is the visible count, at
/// least one row of `columns`. Indexes stay within `photo_count`.
fn prefetch_order(
    (first, last): (usize, usize),
    photo_count: usize,
    columns: usize,
    (ahead_screens, behind_screens): (usize, usize),
    direction: f64,
) -> Vec<usize> {
    if photo_count == 0 {
        return Vec::new();
    }
    let visible = last.saturating_sub(first).saturating_add(1).max(columns);
    let ahead = visible.saturating_mul(ahead_screens);
    let behind = visible.saturating_mul(behind_screens);
    let mut order = Vec::with_capacity((ahead + behind).min(photo_count));
    if direction < 0.0 {
        order.extend((first.saturating_sub(ahead)..first).rev());
        order.extend((last + 1)..(last + 1 + behind).min(photo_count));
    } else {
        order.extend((last + 1)..(last + 1 + ahead).min(photo_count));
        order.extend((first.saturating_sub(behind)..first).rev());
    }
    order
}

/// A tile on the active surface with its vertical span in surface coordinates.
struct VisibleTile {
    tile: SquareTile,
    top: f32,
    bottom: f32,
}

/// Snapshot of the active surface: its visible band and every tile that has a
/// usable vertical extent. Tiles outside the band are included; callers decide
/// whether they need the band or the overscan around it.
struct VisibleTiles {
    band: (f32, f32),
    tiles: Vec<VisibleTile>,
}

impl VisibleTiles {
    fn band_center(&self) -> f32 {
        (self.band.0 + self.band.1) * 0.5
    }

    fn on_screen(&self) -> impl Iterator<Item = &VisibleTile> {
        let (top, bottom) = self.band;
        self.tiles
            .iter()
            .filter(move |tile| tile.bottom >= top && tile.top <= bottom)
    }
}

impl Gallery {
    /// The surface root a scroller's tiles live in, if that scroller's surface
    /// is the one currently showing tiles in Grid layout.
    fn presentation_root(&self, scroller: GalleryScroller) -> Option<gtk::Widget> {
        if self.layout() != PhotoLayout::Grid {
            return None;
        }
        let folder_mode = self.group_mode.get() == GroupMode::Folder;
        match scroller {
            GalleryScroller::Photos => {
                let folder_surface_active = folder_mode
                    && (crate::grid::sectioned_folder_view_enabled()
                        || !crate::grid::folder_gridview_experiment_enabled());
                (!folder_surface_active && self.root.height() > 0)
                    .then(|| self.root.clone().upcast())
            }
            GalleryScroller::Folder => folder_mode.then(|| {
                if crate::grid::sectioned_folder_view_enabled() {
                    self.folder_sectioned_root.clone().upcast()
                } else {
                    self.folder_root.clone().upcast()
                }
            }),
        }
    }

    /// Visible band and placed tiles of a scroller's surface (Grid layout).
    ///
    /// - All Photos: the `GtkGridView` is its own viewport; allocations are
    ///   reliable there, so mapped tiles are measured with `compute_bounds`.
    /// - Folder: the sectioned view's `gtk::Fixed` is as tall as the folder,
    ///   so the band comes from the scroll adjustment, and tiles are measured
    ///   where the view *placed* them (`folder_tile_span`), never by their
    ///   allocation, which lags one frame behind a fast scroll.
    fn visible_presentation_tiles(&self, scroller: GalleryScroller) -> Option<VisibleTiles> {
        let root = self.presentation_root(scroller)?;
        let band = match scroller {
            GalleryScroller::Photos => (0.0, root.height() as f32),
            GalleryScroller::Folder => self.folder_viewport_band(&root)?,
        };
        let mut widgets = Vec::new();
        collect_tiles(&root, &mut widgets);
        let tiles = widgets
            .into_iter()
            .filter_map(|tile| {
                let (top, bottom) = match scroller {
                    GalleryScroller::Photos => {
                        if !tile.is_mapped() || tile.height() <= 0 {
                            return None;
                        }
                        let bounds = tile.compute_bounds(&root)?;
                        (bounds.y(), bounds.y() + bounds.height())
                    }
                    GalleryScroller::Folder => {
                        if !self.folder_tile_has_extent(&root, &tile) {
                            return None;
                        }
                        self.folder_tile_span(&root, &tile)?
                    }
                };
                Some(VisibleTile { tile, top, bottom })
            })
            .collect();
        Some(VisibleTiles { band, tiles })
    }

    /// Request thumbnails for the visible tiles of `scroller`'s surface that
    /// do not yet show one sharp enough, nearest the middle of the screen
    /// first; tiles whose thumbnail is already in RAM get it immediately.
    ///
    /// The requests *replace* the queued visible work (see
    /// `thumbnail_display::replace_visible_requests`), so the snapshot must
    /// describe the current viewport. Wall layouts use their layout model.
    pub fn queue_visible_cached_tiles_async(
        &self,
        scroller: GalleryScroller,
        budget: usize,
    ) -> usize {
        if scroller == GalleryScroller::Folder && self.layout() != PhotoLayout::Grid {
            if let Some(scroll) = self.sectioned_folder.scroll.borrow().as_ref() {
                let a = scroll.vadjustment();
                return self.wall_target_requests(a.value(), a.page_size(), budget);
            }
            return 0;
        }
        if budget == 0 {
            return 0;
        }
        let Some(visible) = self.visible_presentation_tiles(scroller) else {
            return 0;
        };
        let center = visible.band_center();
        let mut candidates: Vec<(f32, &SquareTile)> = visible
            .on_screen()
            .filter(|entry| {
                let photo = entry.tile.imp().photo.borrow();
                photo.as_ref().is_some_and(|photo| {
                    visible_thumbnail_candidate(
                        entry.tile.presentation_settled(),
                        photo.thumbnail_available(),
                        photo.corrupt(),
                    )
                })
            })
            .map(|entry| (((entry.top + entry.bottom) * 0.5 - center).abs(), &entry.tile))
            .collect();
        candidates.sort_by(|left, right| left.0.total_cmp(&right.0));

        let mut requests = Vec::new();
        for (_, tile) in candidates.into_iter().take(budget) {
            let Some(photo) = tile.imp().photo.borrow().as_ref().cloned() else {
                continue;
            };
            let Some(mut request) = photo_presentation_request(&photo, true) else {
                continue;
            };
            request.display_width = self.tile_width.get();
            if let Some(paintable) = folder_thumbnail_cache_get(&request.key) {
                tile.apply_presentation_paintable(&request.key, &paintable);
            } else {
                requests.push(request);
            }
        }
        crate::thumbnail_display::replace_visible_requests(requests)
    }

    /// Paint thumbnails already in RAM onto visible tiles of `scroller`'s
    /// surface without touching the decode queue.
    ///
    /// Used every frame during a direct scrollbar scrub: tiles bind once,
    /// usually before their decode completes, and the completion drain misses
    /// tiles recycled again mid-scrub. Re-checking against the RAM cache heals
    /// exactly those tiles.
    pub fn apply_visible_cached_paintables(&self, scroller: GalleryScroller) -> usize {
        if scroller == GalleryScroller::Folder && self.layout() != PhotoLayout::Grid {
            return self.wall_apply_cached();
        }
        let Some(visible) = self.visible_presentation_tiles(scroller) else {
            return 0;
        };
        let mut applied = 0usize;
        for entry in visible.on_screen() {
            if entry.tile.presentation_settled() {
                continue;
            }
            let Some(photo) = entry.tile.imp().photo.borrow().as_ref().cloned() else {
                continue;
            };
            let Some(key) = photo_presentation_key(&photo) else {
                continue;
            };
            if let Some(paintable) = folder_thumbnail_cache_get(&key) {
                if entry.tile.apply_presentation_paintable(&key, &paintable) {
                    applied += 1;
                }
            }
        }
        applied
    }

    /// First and last photo index among the tiles on screen.
    fn visible_photo_index_span(&self, scroller: GalleryScroller) -> Option<(usize, usize)> {
        let visible = self.visible_presentation_tiles(scroller)?;
        let mut span: Option<(usize, usize)> = None;
        for entry in visible.on_screen() {
            let Some(index) = entry.tile.imp().photo_index.get() else {
                continue;
            };
            span = Some(span.map_or((index, index), |(first, last)| {
                (first.min(index), last.max(index))
            }));
        }
        span
    }

    /// Warm decoded thumbnails around the viewport from the photo model, not
    /// just from realized tiles, so a fast scroll finds them already in RAM.
    /// Tiles ahead in the scroll `direction` come first. Wall layouts use
    /// their layout model.
    pub fn prefetch_cached_tiles(
        &self,
        scroller: GalleryScroller,
        budget: usize,
        direction: f64,
    ) -> usize {
        if scroller == GalleryScroller::Folder && self.layout() != PhotoLayout::Grid {
            return self.wall_prefetch(budget, direction);
        }
        if budget == 0 {
            return 0;
        }
        let Some(visible_span) = self.visible_photo_index_span(scroller) else {
            return 0;
        };
        let photos = self.current_photos.borrow();
        let order = prefetch_order(
            visible_span,
            photos.len(),
            self.current_columns.get() as usize,
            scroller.prefetch_reach(),
            direction,
        );
        queue_prefetch_photos(order.into_iter().filter_map(|index| photos.get(index)), budget)
    }

    /// Vertical span of a folder tile in `root` coordinates.
    ///
    /// For the sectioned folder view this is where the view *placed* the
    /// tile, as recorded when it moved it. That is current as soon as the
    /// scroll handler runs. GTK only updates allocations (and with them
    /// `compute_bounds` and even `gtk::Fixed::child_position`) in the next
    /// layout phase, after the thumbnail pumps ran, so during a fast
    /// scrollbar drag those describe the previous frame's viewport.
    fn folder_tile_span(&self, root: &gtk::Widget, tile: &SquareTile) -> Option<(f32, f32)> {
        if root == self.folder_sectioned_root.upcast_ref::<gtk::Widget>() {
            let index = tile.imp().photo_index.get()?;
            let placed = self
                .sectioned_folder
                .live_tiles
                .borrow()
                .get(&(index as u32))
                .filter(|entry| &entry.tile == tile)
                .and_then(|entry| entry.position.get());
            if let Some((_, y)) = placed {
                let top = y + f64::from(FOLDER_ITEM_MARGIN);
                let bottom = top + f64::from(self.tile_height.get().max(1));
                return Some((top as f32, bottom as f32));
            }
        }
        if tile.height() <= 0 {
            return None;
        }
        let bounds = tile.compute_bounds(root)?;
        Some((bounds.y(), bounds.y() + bounds.height()))
    }

    /// Whether a folder tile has a usable vertical extent. A tile the
    /// sectioned view just placed has no allocation until GTK's next layout
    /// phase, but its placed position and the tile height are already known.
    /// Skipping it would drop exactly the tiles a fast drag reveals.
    fn folder_tile_has_extent(&self, root: &gtk::Widget, tile: &SquareTile) -> bool {
        tile.height() > 0 || root == self.folder_sectioned_root.upcast_ref::<gtk::Widget>()
    }

    fn folder_viewport_band(&self, root: &gtk::Widget) -> Option<(f32, f32)> {
        if root.height() <= 0 {
            return None;
        }
        if root == self.folder_sectioned_root.upcast_ref::<gtk::Widget>() {
            let scroll = self.sectioned_folder.scroll.borrow();
            let adjustment = scroll.as_ref()?.vadjustment();
            let top = adjustment.value() as f32;
            let page = adjustment.page_size() as f32;
            if page <= 0.0 {
                return None;
            }
            return Some((top, top + page));
        }
        Some((0.0, root.height() as f32))
    }
}

// ---- Decoded-thumbnail RAM cache and presentation targets ----

/// RAM cache of decoded thumbnails for the visible viewport.
///
/// The scroll pump asks for every visible tile on every frame, so the lookup
/// has to be cheap. Recency is tracked with a stamp per entry instead of
/// moving keys inside `order`: a hit pushes a new `(stamp, key)` record and
/// leaves the old one behind as stale. Eviction skips stale records, and the
/// queue is compacted once stale records dominate, so every operation stays
/// O(1) amortised instead of scanning up to `FOLDER_THUMBNAIL_CACHE_CAPACITY`
/// keys per lookup.
struct ThumbnailPaintableCache {
    /// Recency records, least recently used first. A record is live only
    /// while its stamp matches the entry's current stamp.
    order: VecDeque<(u64, String)>,
    /// Paintable, byte size, the stamp of its most recent use, and the
    /// presentation target it was decoded for (`None` = full size).
    entries: HashMap<String, (gtk::gdk::Paintable, usize, u64, Option<u32>)>,
    bytes: usize,
    next_stamp: u64,
}

impl ThumbnailPaintableCache {
    fn new() -> Self {
        Self {
            order: VecDeque::new(),
            entries: HashMap::new(),
            bytes: 0,
            next_stamp: 0,
        }
    }

    fn take_stamp(&mut self) -> u64 {
        let stamp = self.next_stamp;
        self.next_stamp = self.next_stamp.wrapping_add(1);
        stamp
    }

    /// Look up `path`, treating an entry decoded for a smaller presentation
    /// target than `required` as a miss. The entry is kept: it is still the
    /// best stand-in until the sharper version arrives and replaces it (see
    /// `get_backstop`).
    fn get(&mut self, path: &str, required: Option<u32>) -> Option<gtk::gdk::Paintable> {
        let (_, _, _, decoded_for) = self.entries.get(path)?;
        if !presentation_target_satisfies(*decoded_for, required) {
            return None;
        }
        self.get_backstop(path).map(|(paintable, _)| paintable)
    }

    /// Look up `path` whatever size it was decoded for, returning the
    /// paintable and the presentation target it was decoded for.
    fn get_backstop(&mut self, path: &str) -> Option<(gtk::gdk::Paintable, Option<u32>)> {
        let newest = self.next_stamp.wrapping_sub(1);
        let current = self.entries.get(path)?.2;
        if current != newest {
            let stamp = self.take_stamp();
            if let Some(entry) = self.entries.get_mut(path) {
                entry.2 = stamp;
            }
            self.order.push_back((stamp, path.to_owned()));
            self.compact_order();
        }
        self.entries
            .get(path)
            .map(|(paintable, _, _, decoded_for)| (paintable.clone(), *decoded_for))
    }

    /// Presentation target `paintable` was decoded for, if it is the entry
    /// cached under `path`. Does not touch recency.
    fn target_of(&self, path: &str, paintable: &gtk::gdk::Paintable) -> Option<Option<u32>> {
        self.entries
            .get(path)
            .filter(|(cached, ..)| cached == paintable)
            .map(|(_, _, _, decoded_for)| *decoded_for)
    }

    fn insert(&mut self, path: String, paintable: gtk::gdk::Paintable, target: Option<u32>) {
        let bytes = paintable.intrinsic_width().max(0) as usize
            * paintable.intrinsic_height().max(0) as usize
            * 4;
        let stamp = self.take_stamp();
        if let Some((_, previous, ..)) = self
            .entries
            .insert(path.clone(), (paintable, bytes, stamp, target))
        {
            self.bytes = self.bytes.saturating_sub(previous);
        }
        self.order.push_back((stamp, path));
        self.bytes = self.bytes.saturating_add(bytes);
        self.evict_to_budget();
        self.compact_order();
    }

    fn remove(&mut self, path: &str) {
        // The recency record becomes stale and is skipped or compacted later.
        if let Some((_, bytes, ..)) = self.entries.remove(path) {
            self.bytes = self.bytes.saturating_sub(bytes);
        }
    }

    fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
        self.bytes = 0;
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn is_live(&self, stamp: u64, path: &str) -> bool {
        self.entries
            .get(path)
            .is_some_and(|(_, _, current, _)| *current == stamp)
    }

    /// Drop stale recency records once they outnumber live ones, keeping the
    /// queue bounded at roughly twice the entry count.
    fn compact_order(&mut self) {
        if self.order.len() <= self.entries.len() * 2 + 64 {
            return;
        }
        let order = std::mem::take(&mut self.order);
        self.order = order
            .into_iter()
            .filter(|(stamp, path)| self.is_live(*stamp, path))
            .collect();
    }

    /// Roughly how many entries fit before eviction starts, judged from the
    /// average size of what is cached now (a full 640 px thumbnail when
    /// empty).
    fn entry_capacity(&self) -> usize {
        if !crate::thumbnail::high_quality_thumbnails_enabled() {
            return FOLDER_THUMBNAIL_CACHE_CAPACITY;
        }
        let average = if self.entries.is_empty() {
            640 * 427 * 4
        } else {
            (self.bytes / self.entries.len()).max(1)
        };
        (FOLDER_THUMBNAIL_CACHE_QUALITY_BYTES / average).min(FOLDER_THUMBNAIL_CACHE_CAPACITY)
    }

    fn evict_to_budget(&mut self) {
        loop {
            let over_quality_budget = crate::thumbnail::high_quality_thumbnails_enabled()
                && self.bytes > FOLDER_THUMBNAIL_CACHE_QUALITY_BYTES;
            if self.len() <= FOLDER_THUMBNAIL_CACHE_CAPACITY && !over_quality_budget {
                break;
            }
            let Some((stamp, oldest)) = self.order.pop_front() else {
                break;
            };
            if !self.is_live(stamp, &oldest) {
                continue;
            }
            if let Some((_, bytes, ..)) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(bytes);
            }
        }
    }
}

/// Byte budget of the decoded-thumbnail RAM cache with High quality on.
const FOLDER_THUMBNAIL_CACHE_QUALITY_BYTES: usize = 64 * 1024 * 1024;

fn folder_thumbnail_cache_entry_capacity() -> usize {
    FOLDER_THUMBNAIL_CACHE.with(|cache| cache.borrow().entry_capacity())
}

fn folder_thumbnail_cache_get(path: &str) -> Option<gtk::gdk::Paintable> {
    let required = current_presentation_target();
    FOLDER_THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().get(path, required))
}

/// Cached thumbnail for `path` even if it was decoded for a smaller zoom
/// level, with whether it is sharp enough for the current one. A stale one
/// is shown as a stand-in while the sharper version loads, never a blank tile.
fn folder_thumbnail_cache_get_with_backstop(path: &str) -> Option<(gtk::gdk::Paintable, bool)> {
    let required = current_presentation_target();
    FOLDER_THUMBNAIL_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .get_backstop(path)
            .map(|(paintable, decoded_for)| {
                (paintable, presentation_target_satisfies(decoded_for, required))
            })
    })
}

fn folder_thumbnail_cache_target_of(path: &str, paintable: &gtk::gdk::Paintable) -> Option<u32> {
    FOLDER_THUMBNAIL_CACHE
        .with(|cache| cache.borrow().target_of(path, paintable))
        .flatten()
}

#[cfg(test)]
fn folder_thumbnail_cache_insert(path: String, paintable: gtk::gdk::Paintable) {
    folder_thumbnail_cache_insert_for_target(path, paintable, None);
}

fn folder_thumbnail_cache_insert_for_target(
    path: String,
    paintable: gtk::gdk::Paintable,
    target: Option<u32>,
) {
    FOLDER_THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().insert(path, paintable, target));
}

/// Short-edge sizes gallery thumbnails are decoded at when zoomed out.
///
/// Few fixed sizes keep the RAM cache reusable across nearby zoom levels.
/// Above the largest one the cached thumbnail is used at full size.
const PRESENTATION_TARGET_BUCKETS: [u32; 4] = [128, 192, 256, 384];

/// Headroom over the tile width. Masonry and Photo Wall size items from the
/// tile width but justify rows and columns, so an item can be drawn somewhat
/// larger than the nominal tile; Cover-fit Grid tiles need exactly the width.
const PRESENTATION_TARGET_HEADROOM: f64 = 1.5;

/// Short edge, in device pixels, gallery thumbnails need for tiles of
/// `tile_width` logical pixels at `scale_factor`, rounded up to a bucket.
fn presentation_target_for_tile(tile_width: i32, scale_factor: i32) -> Option<u32> {
    if tile_width <= 0 {
        return None;
    }
    let needed = f64::from(tile_width) * f64::from(scale_factor.max(1)) * PRESENTATION_TARGET_HEADROOM;
    PRESENTATION_TARGET_BUCKETS
        .into_iter()
        .find(|bucket| f64::from(*bucket) >= needed)
}

/// Whether a thumbnail decoded for `decoded_for` is sharp enough for
/// `required`. `None` means full size on either side.
fn presentation_target_satisfies(decoded_for: Option<u32>, required: Option<u32>) -> bool {
    match (decoded_for, required) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(have), Some(need)) => have >= need,
    }
}

fn current_presentation_target() -> Option<u32> {
    PRESENTATION_TARGET.with(Cell::get)
}

/// Publish a new presentation target. Returns true when it grew, meaning
/// thumbnails already on screen may now be too small.
fn set_presentation_target(target: Option<u32>) -> bool {
    let previous = PRESENTATION_TARGET.with(|cell| cell.replace(target));
    previous != target && !presentation_target_satisfies(previous, target)
}

pub(super) fn clear_folder_thumbnail_cache() {
    FOLDER_THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().clear());
}

fn folder_thumbnail_cache_remove(key: &str) {
    FOLDER_THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().remove(key));
}

impl Gallery {
    /// Per-frame entry point of thumbnail presentation, called once per frame
    /// by the gallery tick before painting. It keeps the decode size in step
    /// with the zoom level, then delivers finished thumbnails. The order
    /// matters: completions are judged against the current target.
    pub fn present_frame(&self) -> usize {
        self.sync_presentation_target();
        self.drain_thumbnail_display_completions()
    }

    /// How many thumbnails one settle's background prefetch may add.
    ///
    /// Prefetch keeps going until a pass finds nothing left to load. When its
    /// band is larger than the decoded-thumbnail RAM cache (at High quality
    /// the 64 MB budget holds only ~58 full 640 px thumbnails, while Photo
    /// Wall prefetches three screens ahead), each pass evicts the previous
    /// one and the loop never ends, keeping every display worker busy and
    /// delaying the tiles actually on screen. Cap the total at half of what
    /// the cache can hold, leaving the rest for visible tiles.
    pub fn thumbnail_prefetch_allowance(&self) -> usize {
        folder_thumbnail_cache_entry_capacity() / 2
    }

    /// Keep the gallery's thumbnail decode size in step with the zoom level.
    ///
    /// Called once per frame before draining completions. Zooming out only
    /// makes new decodes smaller. Zooming in (or moving to a higher scale
    /// factor) re-requests sharper thumbnails for the tiles on screen, while
    /// they keep showing the current image until the sharper one arrives.
    fn sync_presentation_target(&self) {
        let root = self.visible_root();
        let target = presentation_target_for_tile(self.tile_width.get(), root.scale_factor());
        if !set_presentation_target(target) {
            return;
        }
        let mut tiles = Vec::new();
        collect_tiles(&root, &mut tiles);
        let mut requested = 0usize;
        for tile in tiles {
            if !tile.is_mapped() || !tile.imp().visual_loaded.get() || tile.presentation_settled()
            {
                continue;
            }
            let Some(photo) = tile.imp().photo.borrow().as_ref().cloned() else {
                continue;
            };
            let Some(mut request) = photo_presentation_request(&photo, true) else {
                continue;
            };
            request.display_width = self.tile_width.get();
            // A sharp enough cached version lands on the next pump; only
            // decode what is missing.
            if let Some(paintable) = folder_thumbnail_cache_get(&request.key) {
                tile.apply_presentation_paintable(&request.key, &paintable);
                continue;
            }
            if crate::thumbnail_display::submit(request) {
                requested += 1;
            }
        }
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_THUMBNAIL presentation_target_grew target={:?} tile_width={} requested={}",
                target,
                self.tile_width.get(),
                requested
            );
        }
    }

    /// Drain worker completions once per frame. Disk I/O and image transforms
    /// have already happened on background workers; GTK only creates textures,
    /// updates the RAM LRU, and repaints currently realized matching tiles.
    pub fn drain_thumbnail_display_completions(&self) -> usize {
        let completions = crate::thumbnail_display::take_completions();
        let due_retries = crate::thumbnail_display::take_due_retry_keys();
        let mask_enabled = honeycomb_raster::cached_mask_applies(self.tile_width.get());
        let mask_results = if mask_enabled {
            honeycomb_raster::take_ready(honeycomb_raster::current_generation())
        } else {
            Vec::new()
        };
        if completions.is_empty() && due_retries.is_empty() && mask_results.is_empty() {
            if mask_enabled {
                honeycomb_raster::log_metrics_if_due();
            }
            return 0;
        }

        let mut loaded = HashMap::<String, gtk::gdk::Paintable>::new();
        let mut missing = HashSet::<String>::new();
        let cached_mode = honeycomb_raster::cached_mask_applies(self.tile_width.get())
            && self.layout() == PhotoLayout::Honeycomb
            && !grid_zoom_animation_active();
        let mut tiles = Vec::new();
        collect_tiles(self.root.upcast_ref(), &mut tiles);
        collect_tiles(self.folder_root.upcast_ref(), &mut tiles);
        collect_tiles(self.folder_sectioned_root.upcast_ref(), &mut tiles);

        // Resolve the presentation key of every tile once. The key combines
        // the thumbnail path, the rotation and the edit recipe, so building it
        // per completion and per tile made this drain quadratic in the number
        // of visible tiles and delivered thumbnails.
        let mut tile_keys: Vec<Option<String>> = Vec::with_capacity(tiles.len());
        let mut tiles_by_key: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, tile) in tiles.iter().enumerate() {
            let Some(photo) = tile.imp().photo.borrow().as_ref().cloned() else {
                tile_keys.push(None);
                continue;
            };
            let Some(key) = photo_presentation_key(&photo) else {
                tile_keys.push(None);
                continue;
            };
            tiles_by_key.entry(key.clone()).or_default().push(index);
            tile_keys.push(Some(key));
        }

        for completion in completions {
            let completion_outcome = match &completion.outcome {
                crate::thumbnail_display::DisplayOutcome::Loaded { .. } => "loaded",
                crate::thumbnail_display::DisplayOutcome::Missing => "missing",
                crate::thumbnail_display::DisplayOutcome::Failed => "failed",
            };
            crate::thumbnail_trace::event(
                &completion.source_path,
                "completion_delivery",
                completion_outcome,
            );
            let decoded_for = completion.target_short_edge;
            if matches!(
                completion.outcome,
                crate::thumbnail_display::DisplayOutcome::Loaded { .. }
            ) && !presentation_target_satisfies(decoded_for, current_presentation_target())
            {
                // Decoded for a smaller zoom level than the current one. It is
                // still delivered below as a stand-in (a blank tile is worse
                // than a soft one), and requested again at the current size.
                if let Some(indices) = tiles_by_key.get(&completion.key) {
                        if let Some(tile) = indices
                            .iter()
                            .map(|index| &tiles[*index])
                            .find(|tile| tile.is_mapped())
                        {
                            let photo = tile.imp().photo.borrow().as_ref().cloned();
                            if let Some(mut request) =
                                photo.and_then(|photo| photo_presentation_request(&photo, true))
                            {
                                request.display_width = self.tile_width.get();
                                crate::thumbnail_display::submit(request);
                            }
                        }
                    }
            }
            match completion.outcome {
                crate::thumbnail_display::DisplayOutcome::Loaded {
                    width,
                    height,
                    pixels,
                } => {
                    let bytes = glib::Bytes::from_owned(pixels);
                    let texture = gtk::gdk::MemoryTexture::new(
                        width,
                        height,
                        gtk::gdk::MemoryFormat::R8g8b8a8,
                        &bytes,
                        width as usize * 4,
                    );
                    let paintable: gtk::gdk::Paintable = texture.upcast();
                    // Cache first so tiles applying it can record which
                    // presentation target it was decoded for.
                    folder_thumbnail_cache_insert_for_target(
                        completion.key.clone(),
                        paintable.clone(),
                        decoded_for,
                    );
                    if cached_mode && width > 0 && height > 0 {
                        if let Some(indices) = tiles_by_key.get(&completion.key) {
                            for index in indices {
                                let tile = &tiles[*index];
                                if tile.is_mapped() {
                                    tile.apply_presentation_paintable(&completion.key, &paintable);
                                    tile.prepare_honeycomb_mask(bytes.clone());
                                }
                            }
                        }
                    }
                    loaded.insert(completion.key, paintable);
                }
                crate::thumbnail_display::DisplayOutcome::Missing
                | crate::thumbnail_display::DisplayOutcome::Failed => {
                    missing.insert(completion.key);
                }
            }
        }

        for (index, tile) in tiles.iter().enumerate() {
            let Some(photo) = tile.imp().photo.borrow().as_ref().cloned() else {
                continue;
            };
            let Some(key) = tile_keys[index].as_ref() else {
                continue;
            };
            if due_retries.contains(key) && tile.is_mapped() {
                crate::thumbnail_trace::event(&photo.path(), "display_retry", "due_visible_tile");
                if let Some(mut request) = photo_presentation_request(&photo, true) {
                    request.display_width = self.tile_width.get();
                    crate::thumbnail_display::submit_latest_visible(request);
                }
            }
            if let Some(paintable) = loaded.get(key) {
                tile.apply_presentation_paintable(key, paintable);
                if let Some(frame) = tile.first_child().and_downcast::<gtk::Overlay>() {
                    if let Some(picture) = frame.child().and_downcast::<gtk::Picture>() {
                        if picture.tooltip_text().is_none() && tile.is_mapped() {
                            picture.set_tooltip_text(Some(&photo.filename()));
                        }
                    }
                }
            } else if missing.contains(key) {
                // A missing outcome can arrive after the thumbnail was already
                // delivered and applied. Marking the tile missing then would
                // clear the available flag of a tile that shows a picture, and
                // the tile would keep asking for the same thumbnail again.
                let already_applied = tile.imp().applied_visual_key.borrow().as_deref()
                    == Some(key.as_str());
                if !already_applied {
                    tile.mark_presentation_missing(key);
                }
            }
        }

        let mut mask_count = 0;
        for result in mask_results {
            let (key, pixels, width, height) = honeycomb_raster::result_parts(result);
            let expected_len = (width as usize)
                .checked_mul(height as usize)
                .and_then(|size| size.checked_mul(4));
            if key.output_size() != Some((width, height))
                || expected_len != Some(pixels.len())
            {
                continue;
            }
            honeycomb_raster::cache_insert(key.clone(), pixels.clone());
            let started = Instant::now();
            let mut applied = false;
            for tile in &tiles {
                if tile.current_honeycomb_mask_key().as_ref() == Some(&key) {
                    applied |=
                        tile.apply_honeycomb_mask_cached(&key, pixels.clone(), width, height);
                }
            }
            let elapsed = started.elapsed().as_micros() as u64;
            if applied {
                honeycomb_raster::record_main_texture_creation(
                    elapsed,
                    width as usize * height as usize * 4,
                );
            }
            if !applied {
                // The cache remains reusable when a tile was recycled while
                // rasterization ran; no stale widget receives the texture.
            }
            mask_count += 1;
        }
        if mask_enabled {
            honeycomb_raster::log_metrics_if_due();
        }
        loaded.len() + missing.len() + mask_count
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;

    #[test]
    fn prefetch_reaches_ahead_in_the_scroll_direction_first() {
        // Visible photos 20..=29 (one 10-column screen), 1 ahead + 1 behind.
        let down = prefetch_order((20, 29), 100, 10, (1, 1), 1.0);
        assert_eq!(&down[..10], &(30..40).collect::<Vec<_>>()[..]);
        assert_eq!(&down[10..], &(10..20).rev().collect::<Vec<_>>()[..]);

        let up = prefetch_order((20, 29), 100, 10, (1, 1), -1.0);
        assert_eq!(&up[..10], &(10..20).rev().collect::<Vec<_>>()[..]);
        assert_eq!(&up[10..], &(30..40).collect::<Vec<_>>()[..]);
    }

    #[test]
    fn prefetch_stays_inside_the_library_and_covers_at_least_a_row() {
        assert!(prefetch_order((0, 0), 0, 8, (5, 1), 1.0).is_empty());
        // Near the end: ahead is clipped, behind still filled.
        let end = prefetch_order((95, 99), 100, 5, (2, 1), 1.0);
        assert!(end.iter().all(|index| *index < 95));
        assert_eq!(end.len(), 5);
        // A single visible tile still reaches a full row per screen.
        let single = prefetch_order((50, 50), 1000, 8, (2, 0), 1.0);
        assert_eq!(single, (51..67).collect::<Vec<_>>());
        // Reach multipliers per scroller are kept.
        assert_eq!(GalleryScroller::Photos.prefetch_reach(), (5, 1));
        assert_eq!(GalleryScroller::Folder.prefetch_reach(), (6, 2));
    }
}

#[cfg(test)]
mod folder_thumbnail_cache_tests {
    use super::*;

    #[test]
    #[ignore = "requires a GTK display; run individually"]
    fn selected_quality_keeps_decoded_thumbnail_cache_bounded() {
        gtk::init().unwrap();
        struct Restore(bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                crate::thumbnail::set_high_quality_thumbnails_enabled(self.0);
                clear_folder_thumbnail_cache();
            }
        }
        let _restore = Restore(crate::thumbnail::high_quality_thumbnails_enabled());
        clear_folder_thumbnail_cache();
        crate::thumbnail::set_high_quality_thumbnails_enabled(true);
        let high_bytes = glib::Bytes::from_owned(vec![0u8; 640 * 640 * 4]);
        let high: gtk::gdk::Paintable = gtk::gdk::MemoryTexture::new(
            640,
            640,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &high_bytes,
            640 * 4,
        )
        .upcast();
        for index in 0..42 {
            folder_thumbnail_cache_insert(format!("high-{index}"), high.clone());
        }
        FOLDER_THUMBNAIL_CACHE.with(|cache| {
            let cache = cache.borrow();
            let bytes = cache
                .entries
                .values()
                .map(|(image, ..)| {
                    image.intrinsic_width() as usize * image.intrinsic_height() as usize * 4
                })
                .sum::<usize>();
            assert!(bytes <= 64 * 1024 * 1024);
        });

        crate::thumbnail::set_high_quality_thumbnails_enabled(false);
        clear_folder_thumbnail_cache();
        let standard_bytes = glib::Bytes::from_owned(vec![0u8; 320 * 320 * 4]);
        let standard: gtk::gdk::Paintable = gtk::gdk::MemoryTexture::new(
            320,
            320,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &standard_bytes,
            320 * 4,
        )
        .upcast();
        for index in 0..513 {
            folder_thumbnail_cache_insert(format!("standard-{index}"), standard.clone());
        }
        FOLDER_THUMBNAIL_CACHE.with(|cache| {
            assert_eq!(cache.borrow().len(), FOLDER_THUMBNAIL_CACHE_CAPACITY);
        });
    }

    #[test]
    fn presentation_target_rounds_up_to_a_bucket_with_headroom() {
        // 100 px tiles need 150 device px -> 192 bucket.
        assert_eq!(presentation_target_for_tile(100, 1), Some(192));
        assert_eq!(presentation_target_for_tile(80, 1), Some(128));
        assert_eq!(presentation_target_for_tile(170, 1), Some(256));
        // HiDPI doubles the device pixels needed.
        assert_eq!(presentation_target_for_tile(100, 2), Some(384));
        // Tiles beyond the largest bucket use the full cached thumbnail.
        assert_eq!(presentation_target_for_tile(300, 1), None);
        assert_eq!(presentation_target_for_tile(0, 1), None);
    }

    #[test]
    fn presentation_target_growth_is_detected_only_when_sharper_is_needed() {
        assert!(presentation_target_satisfies(None, None));
        assert!(presentation_target_satisfies(None, Some(128)));
        assert!(presentation_target_satisfies(Some(256), Some(192)));
        assert!(!presentation_target_satisfies(Some(192), Some(256)));
        assert!(!presentation_target_satisfies(Some(384), None));

        let previous = current_presentation_target();
        set_presentation_target(Some(256));
        assert!(!set_presentation_target(Some(128)), "zooming out is not growth");
        assert!(!set_presentation_target(Some(128)), "unchanged is not growth");
        assert!(set_presentation_target(Some(192)));
        assert!(set_presentation_target(None), "full size is larger than any bucket");
        set_presentation_target(previous);
    }

    #[test]
    fn decoded_thumbnail_cache_misses_entries_too_small_for_the_zoom() {
        let bytes = glib::Bytes::from_owned(vec![0u8; 4 * 4 * 4]);
        let texture: gtk::gdk::Paintable =
            gtk::gdk::MemoryTexture::new(4, 4, gtk::gdk::MemoryFormat::R8g8b8a8, &bytes, 4 * 4)
                .upcast();
        let mut cache = ThumbnailPaintableCache::new();
        cache.insert("small".to_string(), texture.clone(), Some(128));
        cache.insert("full".to_string(), texture.clone(), None);

        assert!(cache.get("small", Some(128)).is_some());
        assert!(cache.get("full", Some(128)).is_some());
        assert!(cache.get("full", None).is_some());
        // Zoomed in: the 128 px entry is a miss, but stays as a stand-in.
        assert!(cache.get("small", Some(256)).is_none());
        assert_eq!(cache.len(), 2);
        assert_eq!(
            cache.get_backstop("small").map(|(_, target)| target),
            Some(Some(128))
        );
        assert_eq!(cache.target_of("small", &texture), Some(Some(128)));
        assert_eq!(cache.target_of("full", &texture), Some(None));
    }

    #[test]
    fn decoded_thumbnail_cache_evicts_least_recently_used() {
        // `MemoryTexture` is a plain GObject; no display or `gtk::init` needed.
        let previous_quality = crate::thumbnail::high_quality_thumbnails_enabled();
        crate::thumbnail::set_high_quality_thumbnails_enabled(false);
        let bytes = glib::Bytes::from_owned(vec![0u8; 4 * 4 * 4]);
        let texture: gtk::gdk::Paintable =
            gtk::gdk::MemoryTexture::new(4, 4, gtk::gdk::MemoryFormat::R8g8b8a8, &bytes, 4 * 4)
                .upcast();

        let mut cache = ThumbnailPaintableCache::new();
        for index in 0..FOLDER_THUMBNAIL_CACHE_CAPACITY {
            cache.insert(format!("photo-{index}"), texture.clone(), None);
        }
        // Touch the oldest entry many times: it must become most recent, and
        // the repeated hits must not grow the recency queue without bound.
        for _ in 0..10_000 {
            assert!(cache.get("photo-0", None).is_some());
            assert!(cache.get("photo-5", None).is_some());
        }
        assert!(cache.order.len() <= cache.entries.len() * 2 + 64);

        // A removed entry leaves a stale record that eviction must skip.
        cache.remove("photo-2");
        cache.insert("photo-2".to_string(), texture.clone(), None);

        cache.insert("new-a".to_string(), texture.clone(), None);
        cache.insert("new-b".to_string(), texture.clone(), None);
        assert_eq!(cache.len(), FOLDER_THUMBNAIL_CACHE_CAPACITY);
        assert!(cache.get("photo-0", None).is_some(), "recently used entry evicted");
        assert!(cache.get("photo-5", None).is_some(), "recently used entry evicted");
        assert!(cache.get("photo-2", None).is_some(), "re-inserted entry evicted");
        assert!(cache.get("photo-1", None).is_none(), "least recently used survived");
        assert!(cache.get("photo-3", None).is_none(), "least recently used survived");
        assert!(cache.get("photo-4", None).is_some());

        crate::thumbnail::set_high_quality_thumbnails_enabled(previous_quality);
    }
}
