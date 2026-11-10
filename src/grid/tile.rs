pub(crate) fn set_grid_scrub_active(active: bool) {
    GRID_SCRUB_ACTIVE.with(|flag| flag.set(active));
}

pub(crate) fn set_grid_zoom_animation_active(active: bool) {
    GRID_ZOOM_ANIMATION_ACTIVE.with(|flag| flag.set(active));
}

fn grid_scrub_active() -> bool {
    GRID_SCRUB_ACTIVE.with(Cell::get)
}

fn grid_zoom_animation_active() -> bool {
    GRID_ZOOM_ANIMATION_ACTIVE.with(Cell::get)
}

fn preserve_grid_paintable_during_motion() -> bool {
    grid_scrub_active() || grid_zoom_animation_active()
}

fn folder_thumbnail_cache_get(path: &str) -> Option<gtk::gdk::Paintable> {
    FOLDER_THUMBNAIL_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let index = cache
            .iter()
            .position(|(cached_path, _)| cached_path == path)?;
        let entry = cache.remove(index)?;
        let paintable = entry.1.clone();
        cache.push_back(entry);
        Some(paintable)
    })
}

fn folder_thumbnail_cache_insert(path: String, paintable: gtk::gdk::Paintable) {
    FOLDER_THUMBNAIL_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|(cached_path, _)| cached_path != &path);
        cache.push_back((path, paintable));
        loop {
            let bytes = cache.iter().fold(0usize, |bytes, (_, image)| {
                bytes.saturating_add(
                    image.intrinsic_width().max(0) as usize
                        * image.intrinsic_height().max(0) as usize
                        * 4,
                )
            });
            let over_quality_budget =
                crate::thumbnail::high_quality_thumbnails_enabled() && bytes > 64 * 1024 * 1024;
            if cache.len() <= FOLDER_THUMBNAIL_CACHE_CAPACITY && !over_quality_budget {
                break;
            }
            cache.pop_front();
        }
    });
}

pub(super) fn clear_folder_thumbnail_cache() {
    FOLDER_THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().clear());
}

fn folder_thumbnail_cache_remove(key: &str) {
    FOLDER_THUMBNAIL_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .retain(|(cached_key, _)| cached_key != key);
    });
}

fn photo_presentation_key(photo: &PhotoObject) -> Option<String> {
    let cached = photo.cached_thumbnail_path()?;
    Some(crate::thumbnail_display::presentation_key(
        &cached,
        &photo.path(),
        photo.rotation(),
        &photo.edit_recipe(),
        photo.width(),
        photo.height(),
    ))
}

fn photo_presentation_request(
    photo: &PhotoObject,
    visible_priority: bool,
) -> Option<crate::thumbnail_display::DisplayRequest> {
    let cached = photo.cached_thumbnail_path()?;
    Some(crate::thumbnail_display::request_for(
        cached,
        photo.path(),
        photo.mtime(),
        photo.size_bytes(),
        photo.rotation(),
        photo.edit_recipe(),
        photo.width(),
        photo.height(),
        visible_priority,
    ))
}

fn queue_photo_presentation_async(photo: &PhotoObject, visible_priority: bool) -> bool {
    let Some(request) = photo_presentation_request(photo, visible_priority) else {
        return false;
    };
    if folder_thumbnail_cache_get(&request.key).is_some() {
        return false;
    }
    crate::thumbnail_display::submit(request)
}

/// Local cache warming can cover several screens. Remote originals get only
/// a small current band, shared with the generator so late stale display
/// completions cannot enqueue reads for an old viewport.
fn queue_prefetch_photos<'a>(
    photos: impl Iterator<Item = &'a PhotoObject>,
    budget: usize,
) -> usize {
    let photos = photos.collect::<Vec<_>>();
    let wanted = photos
        .iter()
        .map(|photo| photo.path())
        .filter(|path| crate::thumbnail::private_network_preview(path))
        .take(8)
        .collect::<HashSet<_>>();
    crate::thumbnail::retain_network_prefetch_paths(&wanted);
    let mut queued = 0;
    for photo in photos {
        if queued >= budget {
            break;
        }
        let path = photo.path();
        if crate::thumbnail::private_network_preview(&path) && !wanted.contains(&path) {
            continue;
        }
        if queue_photo_presentation_async(photo, false) {
            queued += 1;
        }
    }
    queued
}

const FILENAME_CAPTION_HEIGHT: i32 = 24;

/// Row pitch that already surrounds every tile: 6 px of CSS padding/margin on
/// the top and bottom edge of the grid item or Folder line.
const TILE_ROW_SPACING: i32 = 12;

/// Margin every folder tile keeps around its thumbnail (the padding the old
/// FlowBoxChild supplied). GTK adds it to the tile's placed position, so a
/// section heading needs the same margin to line up with the first column
/// instead of sitting 6 px to its left.
pub(crate) const FOLDER_ITEM_MARGIN: i32 = 6;

thread_local! {
    static FILENAME_LABEL_TRACE_STATS: RefCell<(u64, u128, u128)> =
        const { RefCell::new((0, 0, 0)) };
}

/// Height of the single-line filename row drawn under the square thumbnail.
/// It is always *added* to the tile: the image never gives up any of its own
/// height for the caption.
fn filename_caption_height(visible: bool) -> i32 {
    if visible {
        FILENAME_CAPTION_HEIGHT
    } else {
        0
    }
}

/// Total height of one grid item: the square thumbnail plus the optional
/// filename row below it. Every row-pitch calculation must go through this so
/// rows can never overlap a caption.
fn tile_block_height(tile_height: i32, filename_visible: bool) -> i32 {
    tile_height.max(1) + filename_caption_height(filename_visible)
}

/// Pure geometry for one tile, split out of `size_allocate` so the square-image
/// guarantee is testable without a display.
///
/// The image frame keeps the full `tile_width x tile_height` square whenever
/// the parent allocates at least that much; the caption row is laid out under
/// the square and only ever extends the block downwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TileLayout {
    frame_x: i32,
    frame_y: i32,
    frame_width: i32,
    frame_height: i32,
    caption_y: i32,
    caption_height: i32,
    block_height: i32,
}

fn tile_layout(
    tile_width: i32,
    tile_height: i32,
    allocated_width: i32,
    allocated_height: i32,
    caption_visible: bool,
) -> TileLayout {
    let tile_width = tile_width.max(1);
    let tile_height = tile_height.max(1);
    let allocated_width = allocated_width.max(1);
    let allocated_height = allocated_height.max(1);
    let caption_height = filename_caption_height(caption_visible);
    let frame_width = tile_width.min(allocated_width);
    let frame_height = tile_height.min(allocated_height);
    let block_height = frame_height + caption_height;
    let frame_x = ((allocated_width - frame_width) / 2).max(0);
    let frame_y = ((allocated_height - block_height) / 2).max(0);
    TileLayout {
        frame_x,
        frame_y,
        frame_width,
        frame_height,
        caption_y: frame_y + frame_height,
        caption_height,
        block_height,
    }
}

fn update_filename_label(label: &gtk::Label, filename: &str) {
    let text_changed = label.text().as_str() != filename;
    let tooltip_changed = label.tooltip_text().as_deref() != Some(filename);
    if !text_changed && !tooltip_changed {
        return;
    }

    let tracing = std::env::var_os("PICASA_TRACE").is_some();
    let started = tracing.then(Instant::now);
    if text_changed {
        label.set_text(filename);
    }
    if tooltip_changed {
        label.set_tooltip_text(Some(filename));
    }
    let Some(started) = started else {
        return;
    };

    let elapsed_us = started.elapsed().as_micros();
    let batch = FILENAME_LABEL_TRACE_STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.0 += 1;
        stats.1 += elapsed_us;
        stats.2 = stats.2.max(elapsed_us);
        if stats.0 >= 64 {
            let batch = *stats;
            *stats = (0, 0, 0);
            Some(batch)
        } else {
            None
        }
    });
    if let Some((count, total_us, max_us)) = batch {
        eprintln!(
            "PIC_FILENAME label_update_batch count={count} total_us={total_us} max_us={max_us}"
        );
    }
}

mod square_tile {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use glib::subclass::prelude::*;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk4 as gtk;

    use crate::photo_object::PhotoObject;

    #[derive(Default)]
    pub struct SquareTile {
        pub honeycomb: Cell<bool>,
        pub hex_separator: Cell<f64>,
        pub hex_width: Cell<f64>,
        pub hex_height: Cell<f64>,
        pub hex_selected: Cell<bool>,
        pub hex_mask: RefCell<Option<(i32, i32, gtk::gsk::RenderNode)>>,
        pub honeycomb_mask_cached: Cell<bool>,
        pub honeycomb_mask_original: RefCell<Option<gtk::gdk::Paintable>>,
        pub honeycomb_mask_size: Cell<(i32, i32)>,
        pub honeycomb_mask_scale_factor: Cell<i32>,
        pub(super) honeycomb_mask_lookup_key:
            RefCell<Option<super::honeycomb_raster::MaskCacheKey>>,
        pub width: Cell<i32>,
        pub height: Cell<i32>,
        // Presentation-only FLIP offset. GTK allocation remains at the real
        // destination; snapshotting is temporarily translated from the old
        // visual position while a resize reflow animates.
        pub presentation_offset_x: Cell<f32>,
        pub presentation_offset_y: Cell<f32>,
        // In-cell zoom scale. GTK allocation stays at the destination tile
        // size; snapshotting scales the image frame around its centre.
        // Zero means identity so the derived Default stays safe.
        pub presentation_scale: Cell<f32>,
        pub filename_visible: Cell<bool>,
        pub filename_label: RefCell<Option<gtk::Label>>,
        pub favorite_indicators_visible: Cell<bool>,
        pub photo: RefCell<Option<PhotoObject>>,
        pub visual_loaded: Cell<bool>,
        pub applied_visual_key: RefCell<Option<String>>,
        // Folder mode stores the backing photo index on each realized tile so
        // prefetch can warm the photo model ahead of the viewport rather than
        // being limited to GTK's currently realized widget pool.
        pub photo_index: Cell<Option<usize>>,
        // Stored so the offline badge can be created lazily on first use
        // (folder tiles no longer build it eagerly).
        pub unavailable: RefCell<Option<Rc<dyn Fn(PhotoObject, gtk::Widget)>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SquareTile {
        const NAME: &'static str = "PicasaSquareTile";
        type Type = super::SquareTile;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for SquareTile {
        fn dispose(&self) {
            self.photo.take();
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl SquareTile {
        fn snapshot_hexagon(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let w = obj.width();
            let h = obj.height();
            if w <= 0 || h <= 0 {
                return;
            }
            if self.honeycomb_mask_cached.get()
                && (self.honeycomb_mask_size.get() != (w, h)
                    || self.honeycomb_mask_scale_factor.get() != obj.scale_factor())
            {
                obj.clear_honeycomb_mask_cached();
            }
            let bounds = gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
            if !self.honeycomb_mask_cached.get() {
                let stale = self
                    .hex_mask
                    .borrow()
                    .as_ref()
                    .is_none_or(|(mw, mh, _)| *mw != w || *mh != h);
                if stale {
                    let mask = gtk::Snapshot::new();
                    let cr = mask.append_cairo(&bounds);
                    let points = super::honeycomb::vertices(
                        self.hex_width.get().max(1.0),
                        self.hex_height.get().max(1.0),
                    );
                    cr.move_to(points[0].0, points[0].1);
                    for &(x, y) in &points[1..] {
                        cr.line_to(x, y);
                    }
                    cr.close_path();
                    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                    let _ = cr.fill();
                    drop(cr);
                    self.hex_mask
                        .replace(mask.to_node().map(|node| (w, h, node)));
                }
                snapshot.push_mask(gtk::gsk::MaskMode::Alpha);
                if let Some((_, _, node)) = self.hex_mask.borrow().as_ref() {
                    snapshot.append_node(node);
                }
                snapshot.pop();
            }
            if let Some(child) = obj.first_child() {
                // Layout/recycling can briefly leave the GtkOverlay child
                // without an allocation while the parent tile is visible.
                // GTK warns if snapshot_child is invoked in that state.
                if child.is_mapped() && child.width() > 0 && child.height() > 0 {
                    obj.snapshot_child(&child, snapshot);
                }
            }
            // The lattice retains its 2px geometric gap. A 3px stroke clipped
            // inside each hex contributes 1.5px per side: 5px visible separation.
            // Borderless keeps the same photos, masks and hit-test geometry.
            if self.hex_separator.get() > 0.0 {
                snapshot.push_mask(gtk::gsk::MaskMode::Alpha);
                let cr = snapshot.append_cairo(&bounds);
                let points =
                    super::honeycomb::vertices(self.hex_width.get(), self.hex_height.get());
                cr.move_to(points[0].0, points[0].1);
                for &(x, y) in &points[1..] {
                    cr.line_to(x, y);
                }
                cr.close_path();
                cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                cr.set_line_width(self.hex_separator.get());
                let _ = cr.stroke();
                drop(cr);
                snapshot.pop();
                let mut ancestor = obj.parent();
                while let Some(widget) = ancestor {
                    if widget.has_css_class("photo-grid") {
                        // A tile can be snapshotted in the overscan band and
                        // moved on screen without being snapshotted again. Paint
                        // the theme background over the whole tile so the cached
                        // separator is never clipped to an earlier viewport.
                        #[allow(deprecated)] // GTK 4.12 theme background painting.
                        snapshot.render_background(
                            &widget.style_context(),
                            0.0,
                            0.0,
                            w as f64,
                            h as f64,
                        );
                        break;
                    }
                    ancestor = widget.parent();
                }
                snapshot.pop();
            }
            let selected = self.hex_selected.get();
            let hover = obj.state_flags().contains(gtk::StateFlags::PRELIGHT);
            if selected || hover {
                #[allow(deprecated)] // Theme accent lookup has no GTK 4.12 replacement.
                let color = obj
                    .style_context()
                    .lookup_color("accent_bg_color")
                    .unwrap_or_else(|| obj.color());
                let cr = snapshot.append_cairo(&bounds);
                let inset = if selected { 2.0 } else { 1.0 };
                let factor = ((w as f64 - 2.0 * inset) / w as f64).max(0.0);
                let points = super::honeycomb::vertices(
                    self.hex_width.get().max(1.0),
                    self.hex_height.get().max(1.0),
                );
                for (i, &(x, y)) in points.iter().enumerate() {
                    let x = w as f64 / 2.0 + (x - w as f64 / 2.0) * factor;
                    let y = h as f64 / 2.0 + (y - h as f64 / 2.0) * factor;
                    if i == 0 {
                        cr.move_to(x, y);
                    } else {
                        cr.line_to(x, y);
                    }
                }
                cr.close_path();
                cr.set_source_rgba(
                    color.red() as f64,
                    color.green() as f64,
                    color.blue() as f64,
                    if selected { 1.0 } else { 0.55 },
                );
                cr.set_line_width(if selected { 3.0 } else { 1.5 });
                let _ = cr.stroke();
            }
            if !self.honeycomb_mask_cached.get() {
                snapshot.pop();
            }
        }

        #[cfg(test)]
        pub(super) fn snapshot_honeycomb_for_bench(&self, snapshot: &gtk::Snapshot) {
            self.snapshot_hexagon(snapshot);
        }

        fn caption_visible(&self) -> bool {
            self.filename_visible.get()
                && self
                    .filename_label
                    .borrow()
                    .as_ref()
                    .is_some_and(|label| label.is_visible())
        }
    }

    impl WidgetImpl for SquareTile {
        fn contains(&self, x: f64, y: f64) -> bool {
            if self.honeycomb.get() {
                super::honeycomb::contains(self.hex_width.get(), self.hex_height.get(), x, y)
            } else {
                self.parent_contains(x, y)
            }
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let requested = match orientation {
                gtk::Orientation::Horizontal => self.width.get().max(1),
                // The vertical request is the square thumbnail plus the
                // filename row underneath it, so a caption can never squeeze
                // the image out of its TILE_SIZE x TILE_SIZE allocation.
                gtk::Orientation::Vertical => {
                    self.height.get().max(1)
                        + super::filename_caption_height(self.caption_visible())
                }
                _ => self.width.get().max(1),
            };

            let requested = if self.obj().has_css_class("photo-wall-tile") {
                match orientation {
                    gtk::Orientation::Horizontal => self.width.get(),
                    _ => self.height.get(),
                }
            } else {
                requested
            };
            (requested, requested, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let layout = super::tile_layout(
                self.width.get(),
                self.height.get(),
                width,
                height,
                self.caption_visible(),
            );

            if let Some(child) = self.obj().first_child() {
                // Keep status/action badges in the central band of the polygon.
                // Reset the original offsets when this pooled tile changes layout.
                if let Some(frame) = child.downcast_ref::<gtk::Overlay>() {
                    let inset = if self.honeycomb.get() {
                        height / 4 + 4
                    } else {
                        8
                    };
                    for class in [
                        "offline-badge",
                        "corrupt-badge",
                        "favorite-badge",
                        "rating-badge",
                        "edited-badge",
                    ] {
                        if let Some(badge) = super::find_overlay_child(frame, class) {
                            if badge.valign() == gtk::Align::Start {
                                badge.set_margin_top(inset);
                            } else {
                                badge.set_margin_bottom(inset);
                            }
                        }
                    }
                }
                let transform = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(
                    layout.frame_x as f32,
                    layout.frame_y as f32,
                ));

                child.allocate(
                    layout.frame_width,
                    layout.frame_height,
                    baseline,
                    Some(transform),
                );
                if layout.caption_height > 0 {
                    if let Some(label) = self.filename_label.borrow().as_ref() {
                        let label_transform =
                            gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(
                                layout.frame_x as f32,
                                layout.caption_y as f32,
                            ));
                        label.allocate(
                            layout.frame_width,
                            layout.caption_height,
                            baseline,
                            Some(label_transform),
                        );
                    }
                }
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if self.honeycomb.get() {
                self.snapshot_hexagon(snapshot);
                return;
            }

            let dx = self.presentation_offset_x.get();
            let dy = self.presentation_offset_y.get();
            let stored_scale = self.presentation_scale.get();
            let scale = if stored_scale > 0.001 {
                stored_scale
            } else {
                1.0
            };
            let scaled = (scale - 1.0).abs() > 0.001;
            let translated = dx.abs() > 0.01 || dy.abs() > 0.01;
            let width = self.obj().width() as f32;
            let height = self.obj().height() as f32;

            // Clip the thumbnail to the cell while it is scaled. Zoom-out
            // starts above 1.0; without a clip it paints over neighbours and
            // reads as tiles sliding around.
            if scaled {
                snapshot.push_clip(&gtk::graphene::Rect::new(
                    0.0,
                    0.0,
                    width.max(1.0),
                    height.max(1.0),
                ));
            }

            let mut child = self.obj().first_child();
            let mut is_frame = true;
            while let Some(widget) = child {
                let next = widget.next_sibling();
                if widget.is_visible() {
                    // Scale only the image frame, around its own centre. The
                    // filename caption stays put so zoom is a thumbnail
                    // transform, not a whole-tile FLIP.
                    if scaled && is_frame {
                        snapshot.save();
                        let layout = super::tile_layout(
                            self.width.get(),
                            self.height.get(),
                            width.round() as i32,
                            height.round() as i32,
                            self.caption_visible(),
                        );
                        let cx = (layout.frame_x + layout.frame_width / 2) as f32;
                        let cy = (layout.frame_y + layout.frame_height / 2) as f32;
                        snapshot.translate(&gtk::graphene::Point::new(cx, cy));
                        snapshot.scale(scale, scale);
                        snapshot.translate(&gtk::graphene::Point::new(-cx, -cy));
                        if translated {
                            snapshot.translate(&gtk::graphene::Point::new(dx, dy));
                        }
                        self.obj().snapshot_child(&widget, snapshot);
                        snapshot.restore();
                    } else if translated && is_frame {
                        snapshot.save();
                        snapshot.translate(&gtk::graphene::Point::new(dx, dy));
                        self.obj().snapshot_child(&widget, snapshot);
                        snapshot.restore();
                    } else {
                        self.obj().snapshot_child(&widget, snapshot);
                    }
                }
                is_frame = false;
                child = next;
            }

            if scaled {
                snapshot.pop();
            }
        }
    }
}

#[cfg(test)]
mod filename_caption_tests {
    use super::*;

    #[test]
    fn rectangular_images_keep_axes_with_and_without_captions() {
        for (width, height) in [(240, 100), (80, 160)] {
            for captions in [false, true] {
                let caption = if captions { 24 } else { 0 };
                let geometry = tile_layout(width, height, width, height + caption, captions);
                assert_eq!(
                    (geometry.frame_width, geometry.frame_height),
                    (width, height)
                );
                assert_eq!(geometry.caption_y, height);
                assert_eq!(geometry.block_height, height + caption);
            }
        }
    }

    /// The four smallest zoom-ladder levels that were reported to render the
    /// thumbnail visibly non-square once the filename row was present.
    const VERIFY_SIZES: [i32; 4] = [100, 117, 137, 160];

    #[test]
    fn caption_row_is_appended_below_the_square_image() {
        for tile_size in VERIFY_SIZES {
            for caption_visible in [false, true] {
                let caption = filename_caption_height(caption_visible);
                assert_eq!(caption, if caption_visible { 24 } else { 0 });
                assert_eq!(
                    tile_block_height(tile_size, caption_visible),
                    tile_size + caption
                );

                // A parent that allocates exactly the measured block height
                // must still hand the image its whole square.
                let block = tile_size + caption;
                let layout = tile_layout(tile_size, tile_size, tile_size, block, caption_visible);
                assert_eq!(layout.frame_width, tile_size, "width at {tile_size}px");
                assert_eq!(layout.frame_height, tile_size, "height at {tile_size}px");
                assert_eq!(layout.caption_height, caption);
                assert_eq!(layout.block_height, block);
                assert_eq!(layout.caption_y, layout.frame_y + tile_size);
                assert!(layout.caption_y + caption <= block);
            }
        }
    }

    #[test]
    fn filename_visibility_never_changes_the_image_allocation() {
        for tile_size in VERIFY_SIZES {
            let without = tile_layout(tile_size, tile_size, tile_size, tile_size, false);
            let with = tile_layout(
                tile_size,
                tile_size,
                tile_size,
                tile_size + FILENAME_CAPTION_HEIGHT,
                true,
            );
            assert_eq!(with.frame_width, without.frame_width);
            assert_eq!(with.frame_height, without.frame_height);
            assert_eq!(with.frame_height, tile_size);
            assert_eq!(
                with.frame_width, with.frame_height,
                "frame must stay square"
            );
        }
    }

    #[test]
    fn folder_rows_reserve_the_caption_between_squares() {
        for tile_size in VERIFY_SIZES {
            assert_eq!(folder_line_height(tile_size, false), tile_size + 12);
            assert_eq!(
                folder_line_height(tile_size, true),
                tile_size + FILENAME_CAPTION_HEIGHT + 12
            );
        }
    }

    /// Real GTK allocation check: the Overlay that owns the picture must end
    /// up exactly `size x size` whether or not the filename row is shown.
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn allocated_thumbnail_frame_is_square_at_every_zoom_level() {
        gtk::init().unwrap();
        let context = glib::MainContext::default();
        let settle = || {
            for _ in 0..30 {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };

        for tile_size in VERIFY_SIZES {
            for caption_visible in [false, true] {
                let frame = gtk::Overlay::new();
                let tile = SquareTile::new(tile_size, tile_size, &frame);
                tile.set_halign(gtk::Align::Start);
                tile.set_valign(gtk::Align::Start);
                tile.set_filename_visible(caption_visible);
                let window = gtk::Window::builder().child(&tile).build();
                window.present();
                settle();

                let (_, natural_width, _, _) = tile.measure(gtk::Orientation::Horizontal, -1);
                let (_, natural_height, _, _) = tile.measure(gtk::Orientation::Vertical, -1);
                assert_eq!(
                    natural_height,
                    tile_size + filename_caption_height(caption_visible),
                    "tile height at {tile_size}px caption={caption_visible}"
                );
                assert_eq!(natural_width, tile_size, "tile width at {tile_size}px");
                assert_eq!(
                    (frame.width(), frame.height()),
                    (tile_size, tile_size),
                    "image frame at {tile_size}px caption={caption_visible}"
                );
                window.close();
                settle();
            }
        }
    }
}

glib::wrapper! {
    pub struct SquareTile(ObjectSubclass<square_tile::SquareTile>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SquareTile {
    #[cfg(test)]
    pub(super) fn snapshot_honeycomb_for_bench(&self, snapshot: &gtk::Snapshot) {
        self.imp().snapshot_honeycomb_for_bench(snapshot);
    }

    pub(crate) fn new(width: i32, height: i32, child: &impl IsA<gtk::Widget>) -> Self {
        let tile: Self = glib::Object::new();
        tile.imp().width.set(width.max(1));
        tile.imp().height.set(height.max(1));
        tile.imp().filename_visible.set(false);
        tile.imp().favorite_indicators_visible.set(true);
        child.as_ref().set_parent(&tile);
        tile
    }

    pub(crate) fn photo(&self) -> Option<PhotoObject> {
        self.imp().photo.borrow().as_ref().cloned()
    }

    pub(crate) fn transition_paintable(&self) -> Option<gtk::gdk::Paintable> {
        let frame = self.first_child().and_downcast::<gtk::Overlay>()?;
        let picture = frame.child().and_downcast::<gtk::Picture>()?;
        picture.paintable()
    }

    pub(crate) fn lightbox_transition_source(
        &self,
        fallback: impl FnOnce() -> Option<gtk::gdk::Paintable>,
    ) -> Option<(gtk::Widget, gtk::gdk::Paintable)> {
        if self.imp().honeycomb.get() {
            // The widget identifies the layout; Lightbox never paints this source.
            return Some((
                self.clone().upcast(),
                gtk::WidgetPaintable::new(Some(self)).upcast(),
            ));
        }
        self.transition_paintable()
            .or_else(fallback)
            .map(|paintable| (self.clone().upcast(), paintable))
    }

    pub(crate) fn presentation_offset(&self) -> (f32, f32) {
        (
            self.imp().presentation_offset_x.get(),
            self.imp().presentation_offset_y.get(),
        )
    }

    pub(crate) fn set_presentation_offset(&self, x: f32, y: f32) {
        if (self.imp().presentation_offset_x.get() - x).abs() < 0.01
            && (self.imp().presentation_offset_y.get() - y).abs() < 0.01
        {
            return;
        }
        self.imp().presentation_offset_x.set(x);
        self.imp().presentation_offset_y.set(y);
        self.queue_draw();
    }

    pub(crate) fn presentation_scale(&self) -> f32 {
        let scale = self.imp().presentation_scale.get();
        if scale > 0.001 {
            scale
        } else {
            1.0
        }
    }

    pub(crate) fn set_presentation_scale(&self, scale: f32) {
        let scale = scale.clamp(0.01, 4.0);
        if (self.presentation_scale() - scale).abs() < 0.001 {
            return;
        }
        self.imp().presentation_scale.set(scale);
        self.queue_draw();
    }

    fn set_tile_size(&self, width: i32, height: i32) {
        // Subpixel Wall images may round to an empty allocation. Preserve
        // shared edges instead of forcing a pixel that overlaps a neighbour.
        let minimum = if self.has_css_class("photo-wall-tile") {
            0
        } else {
            1
        };
        let width = width.max(minimum);
        let height = height.max(minimum);
        if self.imp().width.get() == width && self.imp().height.get() == height {
            return;
        }
        self.clear_honeycomb_mask_cached();
        self.imp().width.set(width);
        self.imp().height.set(height);
        self.queue_resize();
    }

    /// Show a single-line filename in its own row below the square thumbnail.
    ///
    /// The caption is *added* to the tile's vertical size request: the image
    /// frame always keeps its full TILE_SIZE x TILE_SIZE square, and row
    /// geometry (grid item height, Folder line height) grows by exactly
    /// `filename_caption_height()`.
    pub(crate) fn set_filename_visible(&self, visible: bool) -> bool {
        let visible = visible && !self.has_css_class("masonry-tile");
        let changed = self.imp().filename_visible.replace(visible) != visible;
        let mut created = false;
        if visible && self.imp().filename_label.borrow().is_none() {
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            label.set_yalign(0.5);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_single_line_mode(true);
            label.set_margin_start(4);
            label.set_margin_end(4);
            label.set_can_target(false);
            label.add_css_class("thumbnail-filename");
            label.set_parent(self);
            self.imp().filename_label.replace(Some(label));
            created = true;
        }
        if let Some(label) = self.imp().filename_label.borrow().as_ref() {
            if let Some(photo) = self.photo() {
                update_filename_label(label, &photo.filename());
            }
            label.set_visible(visible);
        }
        if changed || created {
            // The vertical size request changed (caption row added/removed),
            // so parents must re-measure, not just re-allocate.
            self.queue_resize();
        }
        created
    }

    /// Switch between cropping photos to the tile (Cover, the default) and
    /// letterboxing the whole photo (Contain) so portrait and landscape
    /// thumbnails both show the complete picture.
    fn set_content_fit(&self, fit: gtk::ContentFit) {
        let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() else {
            return;
        };
        if let Some(picture) = frame.child().and_downcast::<gtk::Picture>() {
            picture.set_content_fit(fit);
        }
    }

    /// Remember how to open the unavailable dialog so the offline badge can be
    /// created lazily instead of on every folder tile.
    fn set_unavailable_handler(&self, handler: Rc<dyn Fn(PhotoObject, gtk::Widget)>) {
        self.imp().unavailable.replace(Some(handler));
    }

    fn ensure_offline_badge(&self, frame: &gtk::Overlay) -> gtk::Button {
        if let Some(existing) = find_overlay_child(frame, "offline-badge") {
            if let Ok(button) = existing.downcast::<gtk::Button>() {
                return button;
            }
        }
        let badge = gtk::Button::with_label("!");
        badge.set_halign(gtk::Align::Start);
        badge.set_valign(gtk::Align::Start);
        badge.set_margin_top(8);
        badge.set_margin_start(8);
        badge.add_css_class("offline-badge");
        badge.set_tooltip_text(Some("Original photo unavailable"));
        badge.set_visible(false);
        frame.add_overlay(&badge);
        if let Some(handler) = self.imp().unavailable.borrow().clone() {
            let tile = self.downgrade();
            badge.connect_clicked(move |badge| {
                let Some(tile) = tile.upgrade() else {
                    return;
                };
                let Some(photo) = tile.imp().photo.borrow().as_ref().cloned() else {
                    return;
                };
                handler(photo, badge.clone().upcast());
            });
        }
        badge
    }

    fn set_favorite_indicator_visible(&self, visible: bool) {
        self.imp().favorite_indicators_visible.set(visible);
        self.refresh_favorite_indicator();
    }

    fn refresh_favorite_indicator(&self) {
        let Some(photo) = self.imp().photo.borrow().clone() else {
            return;
        };
        let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() else {
            return;
        };
        let visible = self.imp().favorite_indicators_visible.get() && photo.favorite();
        if visible {
            ensure_favorite_badge(&frame).set_visible(true);
        } else if let Some(child) = find_overlay_child(&frame, "favorite-badge") {
            child.set_visible(false);
        }
        self.refresh_rating_indicator();
    }

    fn refresh_rating_indicator(&self) {
        let Some(photo) = self.imp().photo.borrow().clone() else {
            return;
        };
        let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() else {
            return;
        };
        let rating = photo.rating().clamp(0, 5);
        if rating == 0 {
            if let Some(child) = find_overlay_child(&frame, "rating-badge") {
                child.set_visible(false);
            }
            return;
        }

        let favorite_visible = self.imp().favorite_indicators_visible.get() && photo.favorite();
        let badge = ensure_rating_badge(&frame);
        badge.set_text(&format!("★ {rating}"));
        badge.set_tooltip_text(Some(&format!("Rating: {rating} of 5")));
        // With no Favourite heart the rating owns the normal bottom-right
        // position. When both are present, leave the heart at the far right
        // and shift the rating just far enough left to sit beside it.
        badge.set_margin_end(if favorite_visible { 32 } else { 8 });
        badge.set_visible(true);
    }

    fn set_photo_deferred(&self, photo: &PhotoObject) {
        let same_photo = self
            .imp()
            .photo
            .borrow()
            .as_ref()
            .is_some_and(|current| current.id() == photo.id());
        let same_visual =
            same_photo && *self.imp().applied_visual_key.borrow() == photo_presentation_key(photo);
        if !same_visual {
            self.unload_visual();
        }
        self.imp().photo.replace(Some(photo.clone()));
        if photo.corrupt() {
            self.clear_corrupt_visual();
        }
    }

    fn clear_corrupt_visual(&self) {
        self.imp().visual_loaded.set(true);
        self.imp().applied_visual_key.borrow_mut().take();
        self.clear_honeycomb_mask_cached();
        if let Some(photo) = self.imp().photo.borrow().as_ref() {
            photo.set_thumbnail_available(false);
        }
        if let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() {
            if let Some(picture) = frame.child().and_downcast::<gtk::Picture>() {
                picture.set_paintable(gtk::gdk::Paintable::NONE);
                picture.add_css_class("missing-thumbnail");
                if let Some(placeholder) = picture.next_sibling().and_downcast::<gtk::Image>() {
                    placeholder.set_visible(true);
                }
            }
        }
    }

    fn apply_presentation_paintable(
        &self,
        expected_key: &str,
        paintable: &gtk::gdk::Paintable,
    ) -> bool {
        let started = std::time::Instant::now();
        let Some(photo) = self.imp().photo.borrow().as_ref().cloned() else {
            return false;
        };
        if photo_presentation_key(&photo).as_deref() != Some(expected_key) {
            return false;
        }
        if photo.corrupt() {
            return false;
        }
        if self.imp().visual_loaded.get()
            && self.imp().applied_visual_key.borrow().as_deref() == Some(expected_key)
        {
            return true;
        }
        self.clear_honeycomb_mask_cached();
        let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() else {
            return false;
        };
        let Some(picture) = frame.child().and_downcast::<gtk::Picture>() else {
            return false;
        };
        picture.set_paintable(Some(paintable));
        if picture.has_css_class("missing-thumbnail") {
            picture.remove_css_class("missing-thumbnail");
        }
        if let Some(placeholder) = picture.next_sibling().and_downcast::<gtk::Image>() {
            if placeholder.is_visible() {
                placeholder.set_visible(false);
            }
        }
        photo.set_thumbnail_available(true);
        self.imp().visual_loaded.set(true);
        *self.imp().applied_visual_key.borrow_mut() = Some(expected_key.to_owned());
        trace_first_visible_thumbnail_after_paint(self.upcast_ref());
        crate::thumbnail_trace::event(&photo.path(), "tile_apply", "paintable_applied");
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_THUMBNAIL paintable_assign elapsed_us={}",
                started.elapsed().as_micros()
            );
        }
        true
    }

    fn mark_presentation_missing(&self, expected_key: &str) -> bool {
        let Some(photo) = self.imp().photo.borrow().as_ref().cloned() else {
            return false;
        };
        if photo_presentation_key(&photo).as_deref() != Some(expected_key) {
            return false;
        }
        let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() else {
            return false;
        };
        let Some(picture) = frame.child().and_downcast::<gtk::Picture>() else {
            return false;
        };
        self.clear_honeycomb_mask_cached();
        picture.set_paintable(gtk::gdk::Paintable::NONE);
        if !picture.has_css_class("missing-thumbnail") {
            picture.add_css_class("missing-thumbnail");
        }
        if let Some(placeholder) = picture.next_sibling().and_downcast::<gtk::Image>() {
            if !placeholder.is_visible() {
                placeholder.set_visible(true);
            }
        }
        photo.set_thumbnail_available(false);
        // The presentation worker has already requested background regeneration.
        // Treat the placeholder as the current settled state so a frame pump does
        // not requeue the same missing cache file continuously. A targeted
        // ThumbnailCreated/priority completion invalidates this state.
        self.imp().visual_loaded.set(true);
        crate::thumbnail_trace::event(&photo.path(), "tile_apply", "missing_placeholder_settled");
        true
    }

    fn refresh_badges_from_model(&self) {
        let Some(photo) = self.imp().photo.borrow().as_ref().cloned() else {
            return;
        };
        let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() else {
            return;
        };

        if let Some(label) = self.imp().filename_label.borrow().as_ref() {
            update_filename_label(label, &photo.filename());
            if label.is_visible() != self.imp().filename_visible.get() {
                label.set_visible(self.imp().filename_visible.get());
            }
        }

        if let Some(caption) = photo.history_caption() {
            let label = find_overlay_child(&frame, "history-caption")
                .and_then(|child| child.downcast::<gtk::Label>().ok())
                .unwrap_or_else(|| {
                    let label = gtk::Label::new(None);
                    label.add_css_class("history-caption");
                    label.add_css_class("osd");
                    label.set_valign(gtk::Align::End);
                    label.set_halign(gtk::Align::Fill);
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    label.set_margin_bottom(30);
                    label.set_can_target(false);
                    frame.add_overlay(&label);
                    label
                });
            label.set_text(&caption);
            label.set_tooltip_text(Some(&caption));
            label.set_visible(true);
        } else if let Some(label) = find_overlay_child(&frame, "history-caption") {
            label.set_visible(false);
        }

        let unavailable = !photo.original_available();
        if unavailable {
            let badge = self.ensure_offline_badge(&frame);
            if !badge.is_visible() {
                badge.set_visible(true);
            }
        } else if let Some(child) = find_overlay_child(&frame, "offline-badge") {
            if child.is_visible() {
                child.set_visible(false);
            }
        }
        if photo.corrupt() && !unavailable {
            ensure_corrupt_badge(&frame).set_visible(true);
        } else if let Some(child) = find_overlay_child(&frame, "corrupt-badge") {
            child.set_visible(false);
        }

        let favorite = self.imp().favorite_indicators_visible.get() && photo.favorite();
        if favorite {
            ensure_favorite_badge(&frame).set_visible(true);
        } else if let Some(child) = find_overlay_child(&frame, "favorite-badge") {
            child.set_visible(false);
        }
        self.refresh_rating_indicator();

        let edited = !crate::edit::EditRecipe::decode(&photo.edit_recipe()).is_default();
        if edited {
            ensure_edited_badge(&frame).set_visible(true);
        } else if let Some(child) = find_overlay_child(&frame, "edited-badge") {
            child.set_visible(false);
        }
    }

    fn queue_presentation_visual_async(&self, visible_priority: bool) -> bool {
        let Some(photo) = self.imp().photo.borrow().as_ref().cloned() else {
            return false;
        };
        let Some(request) = photo_presentation_request(&photo, visible_priority) else {
            return false;
        };

        if let Some(paintable) = folder_thumbnail_cache_get(&request.key) {
            self.apply_presentation_paintable(&request.key, &paintable);
            return true;
        }

        // During a direct scrollbar scrub GTK can recycle the same handful of
        // GridView cells hundreds of times per second. Keep the previous
        // paintable as a transient visual backstop instead of flashing the
        // empty template on every rebind. The correct thumbnail replaces it
        // atomically when the worker completion arrives. Once scrubbing
        // settles, refresh_visible_grid_tiles() clears any unresolved stale
        // image and restores the normal placeholder behavior.
        if !preserve_grid_paintable_during_motion() {
            if let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() {
                if let Some(picture) = frame.child().and_downcast::<gtk::Picture>() {
                    picture.set_paintable(gtk::gdk::Paintable::NONE);
                    if !picture.has_css_class("missing-thumbnail") {
                        picture.add_css_class("missing-thumbnail");
                    }
                    if let Some(placeholder) = picture.next_sibling().and_downcast::<gtk::Image>() {
                        if !placeholder.is_visible() {
                            placeholder.set_visible(true);
                        }
                    }
                }
            }
        }

        crate::thumbnail_display::submit(request)
    }

    fn load_visual(&self) {
        self.refresh_badges_from_model();
        if self.imp().visual_loaded.get() {
            return;
        }
        let Some(photo) = self.imp().photo.borrow().as_ref().cloned() else {
            return;
        };
        if photo.corrupt() {
            self.clear_corrupt_visual();
            return;
        }
        let cache_hit = if let Some(key) = photo_presentation_key(&photo) {
            if let Some(paintable) = folder_thumbnail_cache_get(&key) {
                self.apply_presentation_paintable(&key, &paintable);
                true
            } else {
                false
            }
        } else {
            false
        };
        if !cache_hit {
            self.queue_presentation_visual_async(true);
        }
    }

    fn unload_visual(&self) {
        self.imp().visual_loaded.set(false);
        self.imp().applied_visual_key.borrow_mut().take();
        self.clear_honeycomb_mask_cached();
        if !preserve_grid_paintable_during_motion() {
            if let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() {
                if let Some(picture) = frame.child().and_downcast::<gtk::Picture>() {
                    picture.set_paintable(gtk::gdk::Paintable::NONE);
                }
            }
        }
    }

    fn bind_photo(&self, photo: &PhotoObject) {
        self.clear_honeycomb_mask_cached();
        let started = std::time::Instant::now();
        crate::thumbnail_trace::event(&photo.path(), "tile_bind", "grid_photo");
        self.set_photo_deferred(photo);
        self.load_visual();
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_THUMBNAIL gtk_bind elapsed_us={} id={}",
                started.elapsed().as_micros(),
                photo.id()
            );
        }
    }

    /// Folder ListView bind must stay strictly presentation-only.
    ///
    /// GtkListView can recycle hundreds or thousands of rows during a large
    /// scrollbar jump. Do not stat files, probe originals, request thumbnail
    /// generation, or render RAW/edit transforms here. Those operations turn
    /// harmless widget recycling into synchronous main-thread work.
    fn bind_photo_folder_fast(&self, photo: &PhotoObject, photo_index: usize) {
        crate::thumbnail_trace::event(&photo.path(), "tile_bind", "folder_fast_photo");
        self.clear_honeycomb_mask_cached();
        // Folder bind immediately paints the correct paintable, so replace the
        // photo in place instead of running the full unload path. The old path
        // cleared the paintable and toggled CSS on every recycled tile, which
        // invalidated GTK's style tree for the whole realized list on each
        // scroll frame (measured: gtk_css_node_validate_internal dominated).
        let same_photo = self
            .imp()
            .photo
            .borrow()
            .as_ref()
            .is_some_and(|current| current.id() == photo.id());
        if !same_photo {
            self.imp().photo.replace(Some(photo.clone()));
        }
        self.imp().photo_index.set(Some(photo_index));

        let Some(bound) = self.imp().photo.borrow().as_ref().cloned() else {
            return;
        };
        if let Some(label) = self.imp().filename_label.borrow().as_ref() {
            update_filename_label(label, &bound.filename());
            if label.is_visible() != self.imp().filename_visible.get() {
                label.set_visible(self.imp().filename_visible.get());
            }
        }
        let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() else {
            return;
        };
        let Some(picture) = frame.child().and_downcast::<gtk::Picture>() else {
            return;
        };
        if bound.corrupt() {
            self.clear_corrupt_visual();
            self.refresh_badges_from_model();
            return;
        } else if let Some(child) = find_overlay_child(&frame, "corrupt-badge") {
            child.set_visible(false);
        }

        // RAM-only fast path. Never touch the filesystem from ListView bind,
        // but reuse a paintable that the settled-viewport loader has already
        // decoded. This keeps nearby thumbnails visible during wheel scrolling
        // without restoring the old per-bind I/O/RAW/edit work.
        let presentation_key = photo_presentation_key(&bound);
        let memory_hit = presentation_key
            .as_deref()
            .and_then(folder_thumbnail_cache_get);
        if let Some(paintable) = memory_hit.as_ref() {
            crate::thumbnail_trace::event(
                &bound.path(),
                "tile_apply",
                "folder_ram_paintable_applied",
            );
            picture.set_paintable(Some(paintable));
            // A RAM fast-path thumbnail is already the correct visual for this
            // photo. Keep its key in sync with the paintable, exactly as the
            // async completion path does, especially for recycled tiles.
            self.imp().applied_visual_key.replace(presentation_key.clone());
            if picture.has_css_class("missing-thumbnail") {
                picture.remove_css_class("missing-thumbnail");
            }
        } else {
            // A retained image from a recycled tile belongs to the *previous*
            // photo. It is a temporary visual backstop, not a loaded image.
            self.imp().applied_visual_key.borrow_mut().take();
            // Do not blank a recycled Folder tile just because the destination
            // thumbnail is not in RAM yet. Keep the previous paintable as a
            // temporary visual backstop and immediately submit the *current*
            // photo to the async presentation queue. The completion drain
            // validates the presentation key before replacing the paintable, so
            // a tile that gets recycled again cannot receive the wrong image.
            //
            // This is intentionally different from the old Folder design,
            // which displayed an empty template here and waited for a later
            // viewport pump to rediscover the tile. During Page Up/Down and
            // scrollbar scrubbing that rediscovery can lag several frames.
            if picture.paintable().is_none() {
                if !picture.has_css_class("missing-thumbnail") {
                    picture.add_css_class("missing-thumbnail");
                }
            }
            if let Some(request) = photo_presentation_request(&bound, true) {
                crate::thumbnail_display::submit_latest_visible(request);
            }
        }

        let unavailable = !bound.original_available();
        if unavailable {
            let badge = self.ensure_offline_badge(&frame);
            if !badge.is_visible() {
                badge.set_visible(true);
            }
            if badge.tooltip_text().as_deref() != Some("Original photo unavailable") {
                badge.set_tooltip_text(Some("Original photo unavailable"));
            }
        } else if let Some(child) = find_overlay_child(&frame, "offline-badge") {
            if child.is_visible() {
                child.set_visible(false);
            }
        }
        let favorite = self.imp().favorite_indicators_visible.get() && bound.favorite();
        if favorite {
            ensure_favorite_badge(&frame).set_visible(true);
        } else if let Some(child) = find_overlay_child(&frame, "favorite-badge") {
            child.set_visible(false);
        }
        self.refresh_rating_indicator();
        let edited = !crate::edit::EditRecipe::decode(&bound.edit_recipe()).is_default();
        if edited {
            let badge = ensure_edited_badge(&frame);
            badge.set_visible(true);
            if badge.tooltip_text().as_deref() != Some("Edited") {
                badge.set_tooltip_text(Some("Edited"));
            }
        } else if let Some(child) = find_overlay_child(&frame, "edited-badge") {
            child.set_visible(false);
        }
        if let Some(placeholder) = picture.next_sibling().and_downcast::<gtk::Image>() {
            // Only show the empty template when there is genuinely no paintable
            // to keep on screen. A recycled tile with an old paintable stays
            // visually populated until its correct async result arrives.
            let visible = memory_hit.is_none() && picture.paintable().is_none();
            if placeholder.is_visible() != visible {
                placeholder.set_visible(visible);
            }
        }
        self.imp().visual_loaded.set(memory_hit.is_some());
    }

    /// Queue a cached folder thumbnail without blocking the GTK thread.
    ///
    /// Fast Folder scrolling can recycle rows faster than the settled viewport
    /// loader runs. The bind path remains RAM-only; cache-file probing, JPEG
    /// reading, and decoding happen on a bounded worker set here. The result is
    /// applied only if this recycled tile still represents the same photo.
    fn queue_folder_cached_visual_async(&self, visible_priority: bool) -> bool {
        let Some(photo) = self.imp().photo.borrow().as_ref().cloned() else {
            return false;
        };
        if photo.corrupt() {
            return false;
        }
        let Some(request) = photo_presentation_request(&photo, visible_priority) else {
            return false;
        };

        if let Some(paintable) = folder_thumbnail_cache_get(&request.key) {
            self.apply_presentation_paintable(&request.key, &paintable);
            return true;
        }

        // Folder ListView rows are recycled aggressively when their model is
        // rebound. Keep any existing paintable as a temporary visual backstop
        // until the key-validated async result arrives. Missing/failed results
        // still clear the tile through mark_presentation_missing().
        crate::thumbnail_display::submit(request)
    }

    /// Load the final viewport thumbnail after Folder scrolling settles.
    ///
    /// Even after motion settles, thumbnail cache probing, decoding and visual
    /// transforms stay on the shared worker queue. The GTK thread only updates
    /// tooltip/badge state and submits a request when RAM has no paintable.
    fn load_folder_cached_visual(&self) {
        let Some(photo) = self.imp().photo.borrow().as_ref().cloned() else {
            return;
        };
        let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() else {
            return;
        };
        let Some(picture) = frame.child().and_downcast::<gtk::Picture>() else {
            return;
        };

        // Tooltip work stays out of the high-frequency bind path. Once motion
        // settles this is presentation-only and does not touch the original or
        // thumbnail file.
        let filename = photo.filename();
        if picture.tooltip_text().as_deref() != Some(filename.as_str()) {
            picture.set_tooltip_text(Some(&filename));
        }

        // Scrolling never probes originals. Offline state comes from the
        // folder/mount snapshot plus explicit background availability refreshes.
        if !self.imp().visual_loaded.get() {
            self.queue_folder_cached_visual_async(true);
        }
    }

    /// Reset a recycled Folder tile without blanking its current paintable.
    ///
    /// GtkListView can unbind/rebind the same tile pool during a Folder model
    /// splice. Clearing every image at unbind produces a wall of placeholders
    /// while async cache decodes catch up. The next bind immediately changes
    /// the photo identity and queues the correct presentation key; completion
    /// application validates that key before replacing this temporary backstop.
    fn clear_photo_folder_recycle(&self) {
        self.imp().visual_loaded.set(false);
        self.imp().applied_visual_key.borrow_mut().take();
        self.imp().photo.take();
        self.imp().photo_index.set(None);
        if let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() {
            if let Some(picture) = frame.child().and_downcast::<gtk::Picture>() {
                picture.set_tooltip_text(None);
                if let Some(placeholder) = picture.next_sibling().and_downcast::<gtk::Image>() {
                    let visible = picture.paintable().is_none();
                    if placeholder.is_visible() != visible {
                        placeholder.set_visible(visible);
                    }
                }
            }
            for class_name in ["manual-selected", "folder-photo-selected"] {
                frame.remove_css_class(class_name);
            }
            let mut child = frame.first_child();
            while let Some(current) = child {
                if let Some(image) = current.downcast_ref::<gtk::Image>() {
                    if image.has_css_class("favorite-badge")
                        || image.has_css_class("edited-badge")
                        || image.has_css_class("selection-badge")
                    {
                        image.set_visible(false);
                    }
                }
                if current.has_css_class("rating-badge") {
                    current.set_visible(false);
                }
                if let Some(button) = current.downcast_ref::<gtk::Button>() {
                    if button.has_css_class("offline-badge") {
                        button.set_visible(false);
                    }
                }
                child = current.next_sibling();
            }
        }
    }

    fn set_hex_dimensions(&self, width: f64, height: f64, border_width: f64) {
        let stroke = (border_width - 2.0).max(0.0);
        if self.imp().hex_separator.replace(stroke) != stroke {
            self.queue_draw();
        }
        if self.imp().hex_width.replace(width) != width || self.imp().hex_height.get() != height {
            self.clear_honeycomb_mask_cached();
            self.imp().hex_height.set(height);
            self.imp().hex_mask.take();
            self.queue_draw();
        }
    }

    fn honeycomb_mask_cache_key(
        &self,
        presentation_key: &str,
        source_width: u32,
        source_height: u32,
    ) -> Option<honeycomb_raster::MaskCacheKey> {
        if !self.imp().honeycomb.get() || !self.is_mapped() || grid_zoom_animation_active() {
            return None;
        }
        let photo = self.imp().photo.borrow();
        let photo = photo.as_ref()?;
        if photo_presentation_key(photo).as_deref() != Some(presentation_key) {
            return None;
        }
        let revision = format!(
            "{presentation_key}:{}:{}",
            photo.mtime(),
            photo.size_bytes()
        );
        honeycomb_raster::MaskCacheKey::new(
            revision,
            source_width,
            source_height,
            self.width().try_into().ok()?,
            self.height().try_into().ok()?,
            self.scale_factor().try_into().ok()?,
        )
    }

    fn honeycomb_source_dimensions(&self) -> Option<(u32, u32)> {
        let frame = self.first_child().and_downcast::<gtk::Overlay>()?;
        let picture = frame.child().and_downcast::<gtk::Picture>()?;
        let paintable = if self.imp().honeycomb_mask_cached.get() {
            self.imp().honeycomb_mask_original.borrow().clone()?
        } else {
            picture.paintable()?
        };
        Some((
            paintable.intrinsic_width().try_into().ok()?,
            paintable.intrinsic_height().try_into().ok()?,
        ))
    }

    fn current_honeycomb_mask_key(&self) -> Option<honeycomb_raster::MaskCacheKey> {
        let (source_width, source_height) = self.honeycomb_source_dimensions()?;
        let photo = self.imp().photo.borrow();
        let photo = photo.as_ref()?;
        let presentation_key = photo_presentation_key(photo)?;
        self.honeycomb_mask_cache_key(&presentation_key, source_width, source_height)
    }

    fn prepare_honeycomb_mask(&self, source: glib::Bytes) {
        if !honeycomb_raster::cached_render_enabled() || !self.is_mapped() {
            return;
        }
        let Some(key) = self.current_honeycomb_mask_key() else {
            return;
        };
        if self.imp().honeycomb_mask_cached.get() {
            return;
        }
        self.imp()
            .honeycomb_mask_lookup_key
            .replace(Some(key.clone()));
        if let Some(pixels) = honeycomb_raster::cache_get(&key) {
            if let Some((width, height)) = key.output_size() {
                let started = Instant::now();
                self.apply_honeycomb_mask_cached(&key, pixels, width, height);
                honeycomb_raster::record_main_texture_creation(
                    started.elapsed().as_micros() as u64,
                    width as usize * height as usize * 4,
                );
            }
            return;
        }
        honeycomb_raster::note_fallback();
        if !honeycomb_raster::submit(key, source, honeycomb_raster::current_generation()) {
            self.imp().honeycomb_mask_lookup_key.borrow_mut().take();
        }
    }

    fn apply_honeycomb_mask_cached(
        &self,
        key: &honeycomb_raster::MaskCacheKey,
        bytes: glib::Bytes,
        width: u32,
        height: u32,
    ) -> bool {
        let Some((source_width, source_height)) = self.honeycomb_source_dimensions() else {
            return false;
        };
        let Some(photo) = self.imp().photo.borrow().as_ref().cloned() else {
            return false;
        };
        let Some(presentation_key) = photo_presentation_key(&photo) else {
            return false;
        };
        if self
            .honeycomb_mask_cache_key(&presentation_key, source_width, source_height)
            .as_ref()
            != Some(key)
            || key.output_size() != Some((width, height))
        {
            return false;
        }
        let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() else {
            return false;
        };
        let Some(picture) = frame.child().and_downcast::<gtk::Picture>() else {
            return false;
        };
        if !self.imp().honeycomb_mask_cached.get() {
            self.imp()
                .honeycomb_mask_original
                .replace(picture.paintable());
        }
        let texture = gtk::gdk::MemoryTexture::new(
            width as i32,
            height as i32,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            width as usize * 4,
        );
        picture.set_paintable(Some(&texture));
        self.imp().honeycomb_mask_cached.set(true);
        self.imp()
            .honeycomb_mask_lookup_key
            .replace(Some(key.clone()));
        self.imp()
            .honeycomb_mask_size
            .set((self.width(), self.height()));
        self.imp()
            .honeycomb_mask_scale_factor
            .set(self.scale_factor());
        self.queue_draw();
        true
    }

    fn clear_honeycomb_mask_cached(&self) {
        let was_cached = self.imp().honeycomb_mask_cached.replace(false);
        self.imp().honeycomb_mask_lookup_key.borrow_mut().take();
        let original = self.imp().honeycomb_mask_original.take();
        self.imp().honeycomb_mask_size.set((0, 0));
        self.imp().honeycomb_mask_scale_factor.set(0);
        // No cached texture was installed: keep the normal GSK paintable intact.
        if !was_cached {
            return;
        }
        if let Some(picture) = self
            .first_child()
            .and_downcast::<gtk::Overlay>()
            .and_then(|frame| frame.child().and_downcast::<gtk::Picture>())
        {
            picture.set_paintable(original.as_ref());
        }
    }

    // A RAM thumbnail is already visible, but the mask worker requires decoded
    // RGBA bytes. Ask the existing display worker for them instead of reading
    // textures back from Vulkan on the GTK thread. The lookup key ensures a
    // single in-flight request per tile/geometry; failed submissions retry.
    fn queue_cached_honeycomb_mask_source(&self) -> bool {
        if !honeycomb_raster::cached_render_enabled()
            || !self.imp().honeycomb.get()
            || !self.is_mapped()
            || !self.imp().visual_loaded.get()
            || self.imp().honeycomb_mask_cached.get()
        {
            return false;
        }
        let Some(photo) = self.imp().photo.borrow().as_ref().cloned() else {
            return false;
        };
        let Some(key) = photo_presentation_key(&photo) else {
            return false;
        };
        if self.imp().applied_visual_key.borrow().as_deref() != Some(key.as_str()) {
            return false;
        }
        let Some(mask_key) = self.current_honeycomb_mask_key() else {
            return false;
        };
        if self.imp().honeycomb_mask_lookup_key.borrow().as_ref() == Some(&mask_key) {
            return false;
        }
        // A completed mask might already be available in the cache.
        if self.try_apply_cached_honeycomb_mask() {
            return true;
        }
        let Some(request) = photo_presentation_request(&photo, true) else {
            return false;
        };
        if crate::thumbnail_display::submit(request) {
            self.imp().honeycomb_mask_lookup_key.replace(Some(mask_key));
            return true;
        }
        false
    }

    fn try_apply_cached_honeycomb_mask(&self) -> bool {
        if !honeycomb_raster::cached_render_enabled() || self.imp().honeycomb_mask_cached.get() {
            return self.imp().honeycomb_mask_cached.get();
        }
        let Some((source_width, source_height)) = self.honeycomb_source_dimensions() else {
            return false;
        };
        let Some(photo) = self.imp().photo.borrow().as_ref().cloned() else {
            return false;
        };
        let Some(presentation_key) = photo_presentation_key(&photo) else {
            return false;
        };
        let Some(key) =
            self.honeycomb_mask_cache_key(&presentation_key, source_width, source_height)
        else {
            return false;
        };
        if self.imp().honeycomb_mask_lookup_key.borrow().as_ref() == Some(&key) {
            return false;
        }
        let Some(bytes) = honeycomb_raster::cache_get(&key) else {
            honeycomb_raster::note_fallback();
            return false;
        };
        self.imp()
            .honeycomb_mask_lookup_key
            .replace(Some(key.clone()));
        let Some((width, height)) = key.output_size() else {
            return false;
        };
        let started = Instant::now();
        let applied = self.apply_honeycomb_mask_cached(&key, bytes, width, height);
        honeycomb_raster::record_main_texture_creation(
            started.elapsed().as_micros() as u64,
            (width as usize) * (height as usize) * 4,
        );
        applied
    }

    fn set_honeycomb(&self, enabled: bool) {
        if self.imp().honeycomb.replace(enabled) != enabled {
            self.clear_honeycomb_mask_cached();
            if enabled {
                self.add_css_class("honeycomb-tile");
            } else {
                self.remove_css_class("honeycomb-tile");
                self.imp().hex_mask.take();
            }
            self.queue_draw();
        }
    }
    fn accepts_point(&self, x: f64, y: f64) -> bool {
        !self.imp().honeycomb.get()
            || honeycomb::contains(
                self.imp().hex_width.get(),
                self.imp().hex_height.get(),
                x,
                y,
            )
    }

    fn set_manual_selected(&self, selected: bool) {
        if self.imp().hex_selected.replace(selected) != selected {
            self.queue_draw();
        }
        let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() else {
            return;
        };
        // Recycling can hide the selection badge.
        // Opacity is the real on/off switch via CSS, so make the badge visible
        // again here or recycled tiles lose their indicator.
        if selected {
            let badge = ensure_selection_badge(&frame);
            if !badge.is_visible() {
                badge.set_visible(true);
            }
        }
        let has_class = frame.has_css_class("folder-photo-selected");
        if selected && !has_class {
            frame.add_css_class("folder-photo-selected");
        } else if !selected && has_class {
            frame.remove_css_class("folder-photo-selected");
        }
    }

    fn refresh_thumbnail(&self) {
        self.refresh_thumbnail_with_probe(true);
    }

    fn refresh_availability(&self) {
        let Some(photo) = self.imp().photo.borrow().clone() else {
            return;
        };
        // Read the value the async probe or the availability worker already
        // stored; never stat the original on the GTK thread here.
        let available = photo.original_available();

        if let Some(frame) = self.first_child().and_downcast::<gtk::Overlay>() {
            if !available {
                self.ensure_offline_badge(&frame).set_visible(true);
            } else if let Some(child) = find_overlay_child(&frame, "offline-badge") {
                child.set_visible(false);
            }
        }
    }

    fn refresh_thumbnail_with_probe(&self, _probe_thumbnail: bool) {
        let Some(photo) = self.imp().photo.borrow().clone() else {
            return;
        };
        crate::thumbnail_trace::event(&photo.path(), "tile_refresh", "thumbnail_refresh_requested");
        if let Some(key) = photo_presentation_key(&photo) {
            folder_thumbnail_cache_remove(&key);
        }
        self.imp().visual_loaded.set(false);
        self.imp().applied_visual_key.borrow_mut().take();
        self.refresh_badges_from_model();
        self.queue_presentation_visual_async(true);
    }
}

fn find_overlay_child(frame: &gtk::Overlay, css_class: &str) -> Option<gtk::Widget> {
    let mut child = frame.first_child();
    while let Some(current) = child {
        if current.has_css_class(css_class) {
            return Some(current);
        }
        child = current.next_sibling();
    }
    None
}

/// Folder tiles are built without badges; almost no photo needs one, and
/// creating four extra widgets per tile was the dominant cost of rebuilding the
/// Folder rows (~250-520 ms per column change). These helpers create a badge the
/// first time it is actually needed.
fn ensure_favorite_badge(frame: &gtk::Overlay) -> gtk::Image {
    if let Some(existing) = find_overlay_child(frame, "favorite-badge") {
        if let Ok(image) = existing.downcast::<gtk::Image>() {
            return image;
        }
    }
    let badge = gtk::Image::from_icon_name("emote-love-symbolic");
    badge.set_pixel_size(18);
    badge.set_halign(gtk::Align::End);
    badge.set_valign(gtk::Align::End);
    badge.set_margin_bottom(8);
    badge.set_margin_end(8);
    badge.add_css_class("favorite-badge");
    badge.set_visible(false);
    frame.add_overlay(&badge);
    badge
}

fn ensure_rating_badge(frame: &gtk::Overlay) -> gtk::Label {
    if let Some(existing) = find_overlay_child(frame, "rating-badge") {
        if let Ok(label) = existing.downcast::<gtk::Label>() {
            return label;
        }
    }
    let badge = gtk::Label::new(None);
    badge.set_halign(gtk::Align::End);
    badge.set_valign(gtk::Align::End);
    badge.set_margin_bottom(8);
    badge.set_margin_end(8);
    badge.add_css_class("rating-badge");
    badge.add_css_class("osd");
    badge.add_css_class("caption");
    badge.set_can_target(false);
    badge.set_visible(false);
    frame.add_overlay(&badge);
    badge
}

fn ensure_edited_badge(frame: &gtk::Overlay) -> gtk::Image {
    if let Some(existing) = find_overlay_child(frame, "edited-badge") {
        if let Ok(image) = existing.downcast::<gtk::Image>() {
            return image;
        }
    }
    let badge = gtk::Image::from_icon_name("document-edit-symbolic");
    badge.set_pixel_size(18);
    badge.set_halign(gtk::Align::Start);
    badge.set_valign(gtk::Align::End);
    badge.set_margin_bottom(8);
    badge.set_margin_start(8);
    badge.add_css_class("edited-badge");
    badge.set_tooltip_text(Some("Edited"));
    badge.set_visible(false);
    frame.add_overlay(&badge);
    badge
}

fn ensure_corrupt_badge(frame: &gtk::Overlay) -> gtk::Label {
    if let Some(existing) = find_overlay_child(frame, "corrupt-badge") {
        if let Ok(label) = existing.downcast::<gtk::Label>() {
            return label;
        }
    }
    let badge = gtk::Label::new(Some("Corrupt"));
    badge.add_css_class("corrupt-badge");
    badge.add_css_class("osd");
    badge.set_halign(gtk::Align::Start);
    badge.set_valign(gtk::Align::Start);
    badge.set_margin_top(8);
    badge.set_margin_start(8);
    badge.set_tooltip_text(Some("The original file cannot be decoded"));
    badge.set_can_target(false);
    badge.set_visible(false);
    frame.add_overlay(&badge);
    badge
}

fn ensure_selection_badge(frame: &gtk::Overlay) -> gtk::Image {
    if let Some(existing) = find_overlay_child(frame, "selection-badge") {
        if let Ok(image) = existing.downcast::<gtk::Image>() {
            return image;
        }
    }
    let badge = gtk::Image::from_icon_name("object-select-symbolic");
    badge.set_pixel_size(18);
    badge.set_halign(gtk::Align::End);
    badge.set_valign(gtk::Align::Start);
    badge.set_margin_top(8);
    badge.set_margin_end(8);
    badge.add_css_class("selection-badge");
    badge.set_visible(false);
    frame.add_overlay(&badge);
    badge
}

/// Nikon RAW thumbnails can contain a letterboxed embedded preview whose
/// pixel aspect ratio does not match the camera image dimensions. GTK cannot
/// crop bars that are already part of the cached pixels, so crop the small
/// cached image before handing it to the normal Cover-rendered Picture.
#[cfg(test)]
mod raw_thumbnail_orientation_tests {
    #[test]
    fn cached_preview_axes_determine_portrait_and_rotated_landscape() {
        assert_eq!(
            super::raw_display_dimensions(4032, 6040, 427, 640),
            (4032, 6040)
        );
        assert_eq!(
            super::raw_display_dimensions(4032, 6040, 640, 427),
            (6040, 4032)
        );
        assert_eq!(
            super::raw_display_dimensions(6040, 4032, 640, 427),
            (6040, 4032)
        );
        assert_eq!(
            super::raw_display_dimensions(6040, 4032, 427, 640),
            (4032, 6040)
        );
    }
}

fn raw_display_dimensions(
    source_width: i64,
    source_height: i64,
    preview_width: u32,
    preview_height: u32,
) -> (i64, i64) {
    let source_ratio = source_width as f64 / source_height as f64;
    let preview_ratio = preview_width as f64 / preview_height as f64;
    let direct = (preview_ratio / source_ratio).ln().abs();
    let swapped = (preview_ratio * source_ratio).ln().abs();
    if swapped < direct {
        (source_height, source_width)
    } else {
        (source_width, source_height)
    }
}

pub(crate) fn raw_cached_thumbnail(photo: &PhotoObject, path: &str) -> Option<gtk::gdk::Paintable> {
    let source_path = photo.path();
    if !crate::image_format::uses(&source_path, crate::image_format::DecoderKind::Raw) {
        return None;
    }

    let source_width = photo.width();
    let source_height = photo.height();
    if source_width <= 0 || source_height <= 0 {
        return None;
    }

    let rotation = photo.rotation().rem_euclid(360);
    let edit_recipe = photo.edit_recipe();
    let recipe = crate::edit::EditRecipe::decode(&edit_recipe);
    if let Some(paintable) = raw_thumbnail_cache_get(&source_path, rotation, &edit_recipe) {
        return Some(paintable);
    }

    let mut image = image::open(path).ok()?.to_rgba8();
    let image_width = image.width();
    let image_height = image.height();
    if image_width == 0 || image_height == 0 {
        return None;
    }

    // The cached preview already has EXIF orientation applied. Its axes tell
    // us whether the sensor dimensions need swapping without reading the
    // original RAW file (which may live on a slow network share).
    let (display_width, display_height) =
        raw_display_dimensions(source_width, source_height, image_width, image_height);
    let target_ratio = display_width as f64 / display_height as f64;
    let image_ratio = image_width as f64 / image_height as f64;
    if image_ratio > target_ratio {
        let crop_width =
            ((image_height as f64 * target_ratio).round() as u32).clamp(1, image_width);
        let left = (image_width - crop_width) / 2;
        image = image::imageops::crop_imm(&image, left, 0, crop_width, image_height).to_image();
    } else if image_ratio < target_ratio {
        let crop_height =
            ((image_width as f64 / target_ratio).round() as u32).clamp(1, image_height);
        let top = (image_height - crop_height) / 2;
        image = image::imageops::crop_imm(&image, 0, top, image_width, crop_height).to_image();
    }

    image = match rotation {
        90 => image::imageops::rotate90(&image),
        180 => image::imageops::rotate180(&image),
        270 => image::imageops::rotate270(&image),
        _ => image,
    };
    if !recipe.is_default() {
        image = crate::edit::render::apply_recipe(image, &recipe);
    }

    let width = image.width() as i32;
    let height = image.height() as i32;
    let bytes = glib::Bytes::from_owned(image.into_raw());
    let texture = gtk::gdk::MemoryTexture::new(
        width,
        height,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        width as usize * 4,
    );
    let paintable: gtk::gdk::Paintable = texture.upcast();
    raw_thumbnail_cache_insert(source_path, rotation, edit_recipe, paintable.clone());
    Some(paintable)
}

fn raw_thumbnail_cache_get(path: &str, rotation: i32, recipe: &str) -> Option<gtk::gdk::Paintable> {
    RAW_THUMBNAIL_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let index = cache
            .iter()
            .position(|(cached_path, cached_rotation, cached_recipe, _)| {
                cached_path == path && *cached_rotation == rotation && cached_recipe == recipe
            })?;
        let entry = cache.remove(index)?;
        let paintable = entry.3.clone();
        cache.push_back(entry);
        Some(paintable)
    })
}

fn raw_thumbnail_cache_insert(
    path: String,
    rotation: i32,
    recipe: String,
    paintable: gtk::gdk::Paintable,
) {
    RAW_THUMBNAIL_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|(cached_path, cached_rotation, cached_recipe, _)| {
            cached_path != &path || *cached_rotation != rotation || cached_recipe != &recipe
        });
        cache.push_back((path, rotation, recipe, paintable));
        while cache.len() > RAW_THUMBNAIL_CACHE_CAPACITY {
            cache.pop_front();
        }
    });
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
                .iter()
                .map(|(_, image)| {
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
}
