const SECTIONED_HEADER_HEIGHT: f64 = 70.0;
const SECTIONED_SIDE_MARGIN: f64 = 20.0;
const SECTIONED_OVERSCAN_PX: f64 = 320.0;
const SECTIONED_TILE_POOL_CAP: usize = 180;
const SECTIONED_HEADER_POOL_CAP: usize = 12;

#[derive(Clone, Copy, Debug)]
struct SectionedFolderGeometry {
    header_y: f64,
    first_photo_y: f64,
    end_y: f64,
}

#[derive(Clone)]
struct SectionedReflowSnapshot {
    old_columns: u32,
    old_geometry: Vec<SectionedFolderGeometry>,
    tile_width: i32,
    tile_height: i32,
    old_scroll_y: f64,
    old_width: i32,
    presentation: Option<StripPresentation>,
    photos: HashMap<u32, StripPhoto>,
    visual_px: HashMap<u32, f32>,
}

#[derive(Clone)]
struct SectionedFolderTile {
    tile: SquareTile,
    index: Rc<Cell<Option<u32>>>,
}

fn sectioned_scroll_extent_is_ready(content_height: f64, page_height: f64, upper: f64) -> bool {
    content_height <= page_height + 1.0 || upper + 1.0 >= content_height
}

fn visible_section_span(
    geometry: &[SectionedFolderGeometry],
    top: f64,
    bottom: f64,
) -> std::ops::Range<usize> {
    let start = geometry.partition_point(|section| section.end_y < top);
    let end = geometry.partition_point(|section| section.header_y <= bottom);
    start.min(end)..end
}

fn section_index_for_photo(ranges: &[GroupRange], photo_index: usize) -> Option<usize> {
    let index = ranges.partition_point(|range| range.end <= photo_index);
    ranges
        .get(index)
        .filter(|range| photo_index >= range.start && photo_index < range.end)
        .map(|_| index)
}

fn vertical_navigation_target(
    ranges: &[GroupRange],
    current: u32,
    columns: u32,
    preferred_column: u32,
    direction: i32,
) -> Option<u32> {
    let columns = columns.max(1) as usize;
    let section = section_index_for_photo(ranges, current as usize)?;
    let current_range = ranges.get(section)?;
    let local = current as usize - current_range.start;
    let row = local / columns;

    let pick_row = |range: &GroupRange, row: usize| -> Option<u32> {
        let count = range.end.saturating_sub(range.start);
        if count == 0 {
            return None;
        }
        let row_start = row.saturating_mul(columns);
        if row_start >= count {
            return None;
        }
        let row_len = (count - row_start).min(columns);
        let column = (preferred_column as usize).min(row_len.saturating_sub(1));
        Some((range.start + row_start + column) as u32)
    };

    if direction < 0 {
        if row > 0 {
            return pick_row(current_range, row - 1);
        }
        for range in ranges[..section].iter().rev() {
            let count = range.end.saturating_sub(range.start);
            if count == 0 {
                continue;
            }
            return pick_row(range, (count - 1) / columns);
        }
    } else if direction > 0 {
        let count = current_range.end.saturating_sub(current_range.start);
        let last_row = count.saturating_sub(1) / columns;
        if row < last_row {
            return pick_row(current_range, row + 1);
        }
        for range in ranges.iter().skip(section + 1) {
            if range.start < range.end {
                return pick_row(range, 0);
            }
        }
    }
    None
}

fn upper_edge_anchor(
    ranges: &[GroupRange],
    geometry: &[SectionedFolderGeometry],
    scroll_y: f64,
    lower: f64,
    row_height: f64,
) -> Option<(usize, f64)> {
    if scroll_y - lower > row_height {
        return None;
    }
    ranges
        .iter()
        .enumerate()
        .find(|(_, range)| range.start < range.end)
        .and_then(|(section_index, range)| {
            geometry
                .get(section_index)
                .map(|section| (range.start, section.first_photo_y - scroll_y))
        })
}

// A row is a clipped window onto one ordered strip. Fractional capacity lets
// a boundary move through a photo: its two pieces stay in adjacent rows.
#[derive(Clone)]
struct StripPhoto {
    node: gtk::gsk::RenderNode,
    width: f32,
    height: f32,
}

#[derive(Clone, Debug)]
struct StripSection {
    first_index: f64,
    first_y: f64,
    row_height: f64,
    header_y: f64,
    end_y: f64,
}

#[derive(Clone, Debug)]
struct StripLayout {
    columns: f64,
    tile_width: f64,
    tile_height: f64,
    pitch: f64,
    left: f64,
    sections: Vec<StripSection>,
}

impl StripLayout {
    fn between(&self, target: &Self, progress: f64) -> Self {
        let mix = |a: f64, b: f64| a + (b - a) * progress;
        Self {
            columns: mix(self.columns, target.columns),
            tile_width: mix(self.tile_width, target.tile_width),
            tile_height: mix(self.tile_height, target.tile_height),
            pitch: mix(self.pitch, target.pitch),
            left: mix(self.left, target.left),
            sections: self.sections.iter().zip(&target.sections).map(|(a, b)| StripSection {
                first_index: mix(a.first_index, b.first_index),
                first_y: mix(a.first_y, b.first_y),
                row_height: mix(a.row_height, b.row_height),
                header_y: mix(a.header_y, b.header_y),
                end_y: mix(a.end_y, b.end_y),
            }).collect(),
        }
    }
}

#[derive(Clone)]
struct StripPresentation {
    layout: StripLayout,
    photos: HashMap<u32, StripPhoto>,
}

#[derive(Clone, Debug)]
struct StripSlice {
    index: u32,
    row: i32,
    x: f64,
    y: f64,
    clip_x: f64,
    clip_y: f64,
    clip_width: f64,
    clip_height: f64,
}

fn strip_slices(layout: &StripLayout, ranges: &[GroupRange], top: f64, bottom: f64) -> Vec<StripSlice> {
    let mut slices = Vec::new();
    let span = layout.columns * layout.pitch;
    let inset = (layout.pitch - layout.tile_width) * 0.5;
    for (section, range) in layout.sections.iter().zip(ranges) {
        let photo_top = (section.header_y + SECTIONED_HEADER_HEIGHT).max(top);
        let photo_bottom = section.end_y.min(bottom);
        if photo_bottom <= photo_top { continue; }
        let first_row = ((photo_top - section.first_y) / section.row_height).floor() as i32;
        let last_row = ((photo_bottom - section.first_y) / section.row_height).ceil() as i32;
        let count = range.end.saturating_sub(range.start) as i64;
        for row in first_row..last_row {
            let y = section.first_y + f64::from(row) * section.row_height;
            let clip_y = y.max(photo_top);
            let clip_bottom = (y + layout.tile_height).min(photo_bottom);
            if clip_bottom <= clip_y { continue; }
            let strip_start = section.first_index + f64::from(row) * layout.columns;
            let first = (strip_start.floor() as i64 - 1).max(0);
            let last = ((strip_start + layout.columns).ceil() as i64 + 1).min(count);
            for local in first..last {
                let x = layout.left + (local as f64 - strip_start) * layout.pitch + inset;
                let clip_x = x.max(layout.left);
                let clip_right = (x + layout.tile_width).min(layout.left + span);
                if clip_right - clip_x <= 0.001 { continue; }
                slices.push(StripSlice {
                    index: (range.start as i64 + local) as u32, row, x, y,
                    clip_x, clip_y, clip_width: clip_right - clip_x,
                    clip_height: clip_bottom - clip_y,
                });
            }
        }
    }
    slices
}

fn viewport_intersects_row(
    row_top: f64,
    row_height: f64,
    viewport_top: f64,
    viewport_height: f64,
) -> bool {
    viewport_height > 0.0
        && row_height > 0.0
        && row_top < viewport_top + viewport_height
        && row_top + row_height > viewport_top
}

mod sectioned_strip_imp {
    use super::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct Layer {
        pub(super) draws: RefCell<Vec<(StripSlice, StripPhoto)>>,
        pub(super) size: Cell<(f64, f64)>,
        pub(super) top: Cell<f64>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for Layer {
        const NAME: &'static str = "PicasaSectionedStripLayer";
        type Type = super::SectionedStripLayer;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for Layer {}
    impl WidgetImpl for Layer {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let (width, height) = self.size.get();
            for (slice, photo) in self.draws.borrow().iter() {
                snapshot.push_clip(&gtk::graphene::Rect::new(
                    slice.clip_x as f32, (slice.clip_y - self.top.get()) as f32,
                    slice.clip_width as f32, slice.clip_height as f32,
                ));
                snapshot.save();
                snapshot.translate(&gtk::graphene::Point::new(
                    slice.x as f32, (slice.y - self.top.get()) as f32,
                ));
                snapshot.scale(width as f32 / photo.width, height as f32 / photo.height);
                snapshot.append_node(&photo.node);
                snapshot.restore();
                snapshot.pop();
            }
        }
    }
}

glib::wrapper! {
    pub struct SectionedStripLayer(ObjectSubclass<sectioned_strip_imp::Layer>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

fn freeze_strip_photo(tile: &SquareTile) -> Option<StripPhoto> {
    let width = tile.width().max(1) as f64;
    let height = tile.height().max(1) as f64;
    if tile.opacity() > 0.0 {
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(tile)).snapshot(&snapshot, width, height);
        if let Some(node) = snapshot.to_node() {
            return Some(StripPhoto { node, width: width as f32, height: height as f32 });
        }
    }
    let snapshot = gtk::Snapshot::new();
    if let Some(paintable) = tile.transition_paintable() {
        paintable.snapshot(&snapshot, width, height);
    } else {
        snapshot.append_color(&gtk::gdk::RGBA::new(0.5, 0.5, 0.5, 0.15),
            &gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32));
    }
    snapshot.to_node().map(|node| StripPhoto { node, width: width as f32, height: height as f32 })
}


struct SectionedFolderView {
    layout_mode: Cell<PhotoLayout>,
    group_mode: Rc<Cell<GroupMode>>,
    wall_state: RefCell<PhotoWallState>,
    model_generation: Cell<u64>,
    layout_switch_generation: Cell<u64>,
    root: gtk::Fixed,
    spacer: gtk::Box,
    current_photos: Rc<RefCell<Vec<PhotoObject>>>,
    group_ranges: Rc<RefCell<Vec<GroupRange>>>,
    selection: gtk::MultiSelection,
    current_columns: Rc<Cell<u32>>,
    tile_width: Rc<Cell<i32>>,
    tile_height: Rc<Cell<i32>>,
    fit_whole_photo: Rc<Cell<bool>>,
    show_file_names: Rc<Cell<bool>>,
    raw_jpeg_pair_folders: Rc<RefCell<HashSet<i64>>>,
    raw_jpeg_mode: Rc<Cell<crate::image_format::RawJpegPairMode>>,
    raw_jpeg_mode_changed:
        Rc<RefCell<Option<Rc<dyn Fn(crate::image_format::RawJpegPairMode)>>>>,
    zoom_animations_enabled: Rc<Cell<bool>>,
    activate: Rc<dyn Fn(Vec<PhotoObject>, usize, Option<(gtk::Widget, gtk::gdk::Paintable)>)>,
    context_menu: Rc<dyn Fn(PhotoObject, gtk::Widget, f64, f64)>,
    unavailable: Rc<dyn Fn(PhotoObject, gtk::Widget)>,
    collage_mode: Rc<Cell<bool>>,
    collage_ids: Rc<RefCell<HashSet<i64>>>,
    rubberband: gtk::DrawingArea,
    scroll: RefCell<Option<gtk::ScrolledWindow>>,
    geometry: RefCell<Vec<SectionedFolderGeometry>>,
    // During temporary sidebar hover/autohide, Photo Wall must keep using
    // its last stable justified-row width instead of chasing every animated
    // OverlaySplitView allocation.
    wall_width_frozen: Cell<bool>,
    geometry_width: Cell<i32>,
    geometry_columns: Cell<u32>,
    geometry_row_height: Cell<i32>,
    total_height: Cell<f64>,
    live_tiles: RefCell<HashMap<u32, SectionedFolderTile>>,
    tile_pool: RefCell<VecDeque<SectionedFolderTile>>,
    live_headers: RefCell<HashMap<usize, gtk::Label>>,
    header_pool: RefCell<VecDeque<gtk::Label>>,
    live_pair_buttons: RefCell<HashMap<usize, gtk::Button>>,
    pair_button_pool: RefCell<VecDeque<gtk::Button>>,
    selection_anchor: Rc<Cell<Option<u32>>>,
    keyboard_preferred_column: Rc<Cell<Option<u32>>>,
    scroll_animation_generation: Cell<u64>,
    reflow_animation_generation: Cell<u64>,
    reflow_active: Cell<bool>,
    preserve_headers_during_reflow: Cell<bool>,
    strip_layer: RefCell<Option<SectionedStripLayer>>,
    strip_presentation: RefCell<Option<StripPresentation>>,
}

impl SectionedFolderView {
    fn new(
        current_photos: Rc<RefCell<Vec<PhotoObject>>>,
        group_ranges: Rc<RefCell<Vec<GroupRange>>>,
        group_mode: Rc<Cell<GroupMode>>,
        selection: gtk::MultiSelection,
        current_columns: Rc<Cell<u32>>,
        tile_width: Rc<Cell<i32>>,
        tile_height: Rc<Cell<i32>>,
        fit_whole_photo: Rc<Cell<bool>>,
        show_file_names: Rc<Cell<bool>>,
        raw_jpeg_pair_folders: Rc<RefCell<HashSet<i64>>>,
        raw_jpeg_mode: Rc<Cell<crate::image_format::RawJpegPairMode>>,
        raw_jpeg_mode_changed:
            Rc<RefCell<Option<Rc<dyn Fn(crate::image_format::RawJpegPairMode)>>>>,
        activate: Rc<dyn Fn(Vec<PhotoObject>, usize, Option<(gtk::Widget, gtk::gdk::Paintable)>)>,
        context_menu: Rc<dyn Fn(PhotoObject, gtk::Widget, f64, f64)>,
        unavailable: Rc<dyn Fn(PhotoObject, gtk::Widget)>,
        collage_mode: Rc<Cell<bool>>,
        collage_ids: Rc<RefCell<HashSet<i64>>>,
        zoom_animations_enabled: Rc<Cell<bool>>,
    ) -> Rc<Self> {
        let root = gtk::Fixed::new();
        root.set_hexpand(true);
        root.set_vexpand(false);
        root.set_focusable(true);
        root.add_css_class("sectioned-folder-view");

        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_can_target(false);
        root.put(&spacer, 0.0, 0.0);

        let rubberband = gtk::DrawingArea::new();
        rubberband.set_can_target(false);
        rubberband.set_visible(false);
        rubberband.set_draw_func(|_, context, width, height| {
            context.set_source_rgba(0.30, 0.62, 0.86, 0.18);
            context.rectangle(0.0, 0.0, width as f64, height as f64);
            let _ = context.fill_preserve();
            context.set_source_rgba(0.47, 0.73, 0.91, 0.95);
            context.set_line_width(1.0);
            let _ = context.stroke();
        });
        root.put(&rubberband, 0.0, 0.0);

        let view = Rc::new(Self {
            layout_mode: Cell::new(PhotoLayout::Grid),
            group_mode,
            wall_state: RefCell::new(PhotoWallState::default()),
            model_generation: Cell::new(0),
            layout_switch_generation: Cell::new(0),
            root,
            spacer,
            current_photos,
            group_ranges,
            selection,
            current_columns,
            tile_width,
            tile_height,
            fit_whole_photo,
            show_file_names,
            raw_jpeg_pair_folders,
            raw_jpeg_mode,
            raw_jpeg_mode_changed,
            zoom_animations_enabled,
            activate,
            context_menu,
            unavailable,
            collage_mode,
            collage_ids,
            rubberband,
            scroll: RefCell::new(None),
            geometry: RefCell::new(Vec::new()),
            wall_width_frozen: Cell::new(false),
            geometry_width: Cell::new(0),
            geometry_columns: Cell::new(0),
            geometry_row_height: Cell::new(0),
            total_height: Cell::new(1.0),
            live_tiles: RefCell::new(HashMap::new()),
            tile_pool: RefCell::new(VecDeque::new()),
            live_headers: RefCell::new(HashMap::new()),
            header_pool: RefCell::new(VecDeque::new()),
            live_pair_buttons: RefCell::new(HashMap::new()),
            pair_button_pool: RefCell::new(VecDeque::new()),
            selection_anchor: Rc::new(Cell::new(None)),
            keyboard_preferred_column: Rc::new(Cell::new(None)),
            scroll_animation_generation: Cell::new(0),
            reflow_animation_generation: Cell::new(0),
            reflow_active: Cell::new(false),
            preserve_headers_during_reflow: Cell::new(false),
            strip_layer: RefCell::new(None),
            strip_presentation: RefCell::new(None),
        });

        let keyboard = gtk::EventControllerKey::new();
        keyboard.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&view);
        keyboard.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(view) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            if control && matches!(key, gtk::gdk::Key::a | gtk::gdk::Key::A) {
                view.selection.select_all();
                return glib::Propagation::Stop;
            }
            if control || modifiers.contains(gtk::gdk::ModifierType::ALT_MASK) {
                return glib::Propagation::Proceed;
            }

            let selected = selected_positions(&view.selection);
            let current = selected.first().copied().unwrap_or(0);
            if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) {
                let photos = view.current_photos.borrow().clone();
                if (current as usize) < photos.len() {
                    (view.activate)(photos, current as usize, None);
                    return glib::Propagation::Stop;
                }
                return glib::Propagation::Proceed;
            }

            let count = view.selection.n_items();
            if count == 0 {
                return glib::Propagation::Proceed;
            }
            let columns = view.current_columns.get().max(1);
            let next = match key {
                gtk::gdk::Key::Left => {
                    let next = current.checked_sub(1);
                    if let Some(next) = next {
                        view.keyboard_preferred_column
                            .set(view.column_for_index(next));
                    }
                    next
                }
                gtk::gdk::Key::Right => {
                    let candidate = current.saturating_add(1);
                    let next = (candidate < count).then_some(candidate);
                    if let Some(next) = next {
                        view.keyboard_preferred_column
                            .set(view.column_for_index(next));
                    }
                    next
                }
                gtk::gdk::Key::Up | gtk::gdk::Key::Down => {
                    let preferred = view
                        .keyboard_preferred_column
                        .get()
                        .or_else(|| view.column_for_index(current))
                        .unwrap_or(0);
                    view.keyboard_preferred_column.set(Some(preferred));
                    let direction = if key == gtk::gdk::Key::Up { -1 } else { 1 };
                    if view.is_wall() {
                        view.wall_vertical_neighbor(current, direction)
                    } else {
                        vertical_navigation_target(
                            &view.group_ranges.borrow(),
                            current,
                            columns,
                            preferred,
                            direction,
                        )
                    }
                }
                _ => return glib::Propagation::Proceed,
            };
            let Some(next) = next else {
                return glib::Propagation::Stop;
            };
            view.selection.select_item(next, true);
            view.selection_anchor.set(Some(next));
            view.smooth_keep_index_in_center_zone(next);
            if let Some(photo) = view.current_photos.borrow().get(next as usize) {
                view.focus_photo(photo.id());
            }
            glib::Propagation::Stop
        });
        view.root.add_controller(keyboard);

        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let drag_start = Rc::new(Cell::new(None::<(f64, f64)>));
        let drag_base = Rc::new(RefCell::new(HashSet::<u32>::new()));

        {
            let weak = Rc::downgrade(&view);
            let drag_start = drag_start.clone();
            let drag_base = drag_base.clone();
            drag.connect_drag_begin(move |gesture, x, y| {
                let Some(view) = weak.upgrade() else {
                    return;
                };
                if view.collage_mode.get() {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                drag_start.set(Some((x, y)));
                // Reinsert the band so it snapshots above recycled tiles/headers.
                view.root.remove(&view.rubberband);
                view.root.put(&view.rubberband, x, y);
                let control = gesture
                    .current_event_state()
                    .contains(gtk::gdk::ModifierType::CONTROL_MASK);
                let mut base = drag_base.borrow_mut();
                base.clear();
                if control {
                    base.extend(selected_positions(&view.selection));
                }
                view.rubberband.set_visible(false);
                view.root.set_cursor_from_name(Some("crosshair"));
            });
        }

        {
            let weak = Rc::downgrade(&view);
            let drag_start = drag_start.clone();
            let drag_base = drag_base.clone();
            drag.connect_drag_update(move |gesture, dx, dy| {
                if dx.hypot(dy) < DRAG_CLAIM_THRESHOLD {
                    return;
                }
                let Some(view) = weak.upgrade() else {
                    return;
                };
                let Some((sx, sy)) = drag_start.get() else {
                    return;
                };
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let ex = sx + dx;
                let ey = sy + dy;
                let left = sx.min(ex);
                let top = sy.min(ey);
                let right = sx.max(ex);
                let bottom = sy.max(ey);

                view.rubberband
                    .set_size_request((right - left).ceil() as i32, (bottom - top).ceil() as i32);
                view.root.move_(&view.rubberband, left, top);
                view.rubberband.set_visible(true);
                view.rubberband.queue_draw();

                let mut selected = drag_base.borrow().clone();
                for (index, entry) in view.live_tiles.borrow().iter() {
                    let Some(bounds) = entry.tile.compute_bounds(&view.root) else {
                        continue;
                    };
                    let tile_left = f64::from(bounds.x());
                    let tile_top = f64::from(bounds.y());
                    let tile_right = tile_left + f64::from(bounds.width());
                    let tile_bottom = tile_top + f64::from(bounds.height());
                    if tile_right >= left
                        && tile_left <= right
                        && tile_bottom >= top
                        && tile_top <= bottom
                    {
                        selected.insert(*index);
                    }
                }

                view.selection.unselect_all();
                let mut selected = selected.into_iter().collect::<Vec<_>>();
                selected.sort_unstable();
                for position in selected {
                    view.selection.select_item(position, false);
                }
            });
        }

        {
            let weak = Rc::downgrade(&view);
            let drag_start = drag_start.clone();
            drag.connect_drag_end(move |_, _, _| {
                drag_start.set(None);
                if let Some(view) = weak.upgrade() {
                    view.rubberband.set_visible(false);
                    view.root.set_cursor(None::<&gtk::gdk::Cursor>);
                }
            });
        }
        view.root.add_controller(drag);
        view
    }

    fn root(&self) -> &gtk::Fixed {
        &self.root
    }

    fn set_raw_jpeg_pair_folders(self: &Rc<Self>, folder_ids: HashSet<i64>) {
        self.raw_jpeg_pair_folders.replace(folder_ids);
        self.refresh();
    }

    fn set_raw_jpeg_mode(
        self: &Rc<Self>,
        mode: crate::image_format::RawJpegPairMode,
    ) {
        self.raw_jpeg_mode.set(mode);
        self.refresh();
    }

    fn set_raw_jpeg_mode_changed_handler(
        &self,
        handler: Rc<dyn Fn(crate::image_format::RawJpegPairMode)>,
    ) {
        self.raw_jpeg_mode_changed.replace(Some(handler));
    }

    fn attach_scroll(self: &Rc<Self>, scrolled: &gtk::ScrolledWindow) {
        self.scroll.replace(Some(scrolled.clone()));
        let weak = Rc::downgrade(self);
        self.root.connect_unmap(move |_| {
            if let Some(surface) = weak.upgrade() {
                surface.clear_wall_quality_state();
            }
        });

        let this = self.clone();
        scrolled
            .vadjustment()
            .connect_value_changed(move |adjustment| {
                if this.is_wall() && std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_WALL_SCROLL_WRITE value={:.3} upper={:.3} page={:.3} geometry_width={}",
                        adjustment.value(),
                        adjustment.upper(),
                        adjustment.page_size(),
                        this.geometry_width.get(),
                    );
                    if std::env::var_os("PICASA_TRACE_SCROLL_STACKS").is_some() {
                        eprintln!("{}", std::backtrace::Backtrace::force_capture());
                    }
                }
                this.refresh();
            });

        // The frame tick runs before GTK allocates the resized viewport.
        // Its adjustment notification arrives after the viewport allocated
        // its child. Update geometry and the mapped scroll, then apply that
        // scroll to the viewport's child allocation before painting.
        let weak = Rc::downgrade(self);
        let weak_scroll = scrolled.downgrade();
        scrolled.hadjustment().connect_changed(move |_| {
            let (Some(surface), Some(scroll)) = (weak.upgrade(), weak_scroll.upgrade()) else {
                return;
            };
            if surface.is_wall()
                && !surface.wall_width_frozen.get()
                && surface.geometry_width.get() > 1
                && scroll.width() > 1
                && surface.geometry_width.get() != scroll.width()
                && surface.geometry_row_height.get() == surface.tile_width.get()
            {
                let old_width = surface.geometry_width.get();
                let old_scroll = scroll.vadjustment().value();
                surface.refresh();
                if let Some(viewport) = surface.root.parent().and_downcast::<gtk::Viewport>() {
                    // GtkViewport freezes adjustment notifications while
                    // allocating, and thaws them after positioning its child.
                    // Setting the mapped value here only queues allocation;
                    // finish it synchronously to avoid painting the old shift.
                    viewport.size_allocate(&viewport.allocation(), viewport.allocated_baseline());
                }
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_WALL_ALLOCATION old_width={old_width} allocated_width={} geometry_width={} old_scroll={old_scroll:.3} mapped_scroll={:.3}",
                        scroll.width(),
                        surface.geometry_width.get(),
                        scroll.vadjustment().value(),
                    );
                }
            }
        });

        let this = self.clone();
        let scrolled_for_tick = scrolled.clone();
        let last_width = Rc::new(Cell::new(0_i32));
        let last_width_for_tick = last_width.clone();
        scrolled.add_tick_callback(move |_, _| {
            let width = scrolled_for_tick.width();
            if width > 0 && width != last_width_for_tick.get() {
                last_width_for_tick.set(width);
                // Photo Wall scales its existing geometry inside refresh();
                // other layouts only stretch headers here.
                this.refresh();
            }
            if this.is_wall() && this.geometry_width.get() == 0 {
                this.refresh();
            }
            this.poll_wall_quality();
            glib::ControlFlow::Continue
        });

        self.refresh();
    }

    fn invalidate_geometry(&self) {
        let generation = self.wall_state.borrow().generation.wrapping_add(1);
        self.wall_state.borrow_mut().generation = generation;
        self.geometry_width.set(0);
        self.geometry_columns.set(0);
        self.geometry_row_height.set(0);
    }

    fn geometry_for_current_layout(&self, width: i32) {
        if self.is_wall() {
            // Hover-autohide is a presentation-only sidebar animation. While
            // it is active, retain the Photo Wall's last stable geometry width
            // even though GtkScrolledWindow is allocated intermediate widths
            // on every animation frame. This prevents justified rows from
            // continuously reshuffling while the drawer moves.
            let width = if self.wall_width_frozen.get() && self.geometry_width.get() > 0 {
                self.geometry_width.get()
            } else {
                width
            };
            self.calculate_wall_geometry(width);
            return;
        }
        let columns = self.current_columns.get().max(1);
        let tile_height = self.tile_height.get();
        let range_count = self.group_ranges.borrow().len();
        let row_height_px = folder_line_height(tile_height, self.show_file_names.get());

        // Width by itself does not affect section Y positions. Only the number
        // of columns, the photo-row height (tile size plus the filename caption
        // row), or section membership changes vertical geometry. Remember the
        // latest width for diagnostics/header sizing, but avoid rebuilding every
        // frame while the sidebar/window animates.
        if self.geometry_columns.get() == columns
            && self.geometry_row_height.get() == row_height_px
            && self.geometry.borrow().len() == range_count
        {
            self.geometry_width.set(width);
            return;
        }

        let row_height = f64::from(row_height_px);
        let ranges = self.group_ranges.borrow();
        let mut y = 0.0;
        let mut geometry = Vec::with_capacity(ranges.len());

        for range in ranges.iter() {
            let count = range.end.saturating_sub(range.start);
            let rows = count.div_ceil(columns as usize);
            let header_y = y;
            let first_photo_y = header_y + SECTIONED_HEADER_HEIGHT;
            let end_y = first_photo_y + rows as f64 * row_height;
            geometry.push(SectionedFolderGeometry {
                header_y,
                first_photo_y,
                end_y,
            });
            y = end_y;
        }

        self.geometry.replace(geometry);
        self.geometry_width.set(width);
        self.geometry_columns.set(columns);
        self.geometry_row_height.set(row_height_px);
        self.total_height.set(y.max(1.0));
    }

    fn make_tile(self: &Rc<Self>) -> SectionedFolderTile {
        let tile = make_folder_tile(
            self.tile_width.get(),
            self.tile_height.get(),
            &self.unavailable,
        );
        tile.set_filename_visible(self.show_file_names.get() && !self.is_wall());
        tile.set_content_fit(if !self.is_wall() && self.fit_whole_photo.get() {
            gtk::ContentFit::Contain
        } else {
            gtk::ContentFit::Cover
        });

        let index = Rc::new(Cell::new(None::<u32>));

        let click = gtk::GestureClick::new();
        click.set_button(1);
        let index_for_click = index.clone();
        let selection = self.selection.clone();
        let selection_anchor = self.selection_anchor.clone();
        let photos = self.current_photos.clone();
        let activate = self.activate.clone();
        let tile_for_click = tile.clone();
        let self_for_click_collage_mode = self.collage_mode.clone();
        let self_for_click_collage_ids = self.collage_ids.clone();
        let self_for_click_columns = self.current_columns.clone();
        let self_for_click_ranges = self.group_ranges.clone();
        let self_for_click_preferred = self.keyboard_preferred_column.clone();
        click.connect_pressed(move |gesture, presses, _, _| {
            let Some(position) = index_for_click.get() else {
                return;
            };
            if self_for_click_collage_mode.get() {
                let Some(photo) = photos.borrow().get(position as usize).cloned() else {
                    return;
                };
                let mut ids = self_for_click_collage_ids.borrow_mut();
                if ids.remove(&photo.id()) {
                    selection.unselect_item(position);
                } else {
                    ids.insert(photo.id());
                    selection.select_item(position, false);
                }
                selection_anchor.set(Some(position));
                return;
            }
            let state = gesture.current_event_state();
            let control = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gtk::gdk::ModifierType::SHIFT_MASK);

            if shift {
                let selected = selected_positions(&selection);
                let next = folder_selection_after_click(
                    selection.n_items(),
                    &selected,
                    selection_anchor.get(),
                    position,
                    false,
                    true,
                );
                selection.unselect_all();
                for item in next {
                    selection.select_item(item, false);
                }
            } else if control {
                if selection.is_selected(position) {
                    selection.unselect_item(position);
                } else {
                    selection.select_item(position, false);
                }
                selection_anchor.set(Some(position));
            } else {
                selection.select_item(position, true);
                selection_anchor.set(Some(position));
            }

            if !shift && !control {
                // A direct click establishes the visual column that Up/Down
                // should keep while crossing short rows and folder headers.
                let columns = self_for_click_columns.get().max(1);
                let ranges = self_for_click_ranges.borrow();
                if let Some(section) = section_index_for_photo(&ranges, position as usize) {
                    if let Some(range) = ranges.get(section) {
                        let local = position as usize - range.start;
                        self_for_click_preferred.set(Some((local as u32) % columns));
                    }
                }
            }

            if presses == 2 {
                let index = position as usize;
                let source_paintable = tile_for_click.transition_paintable().or_else(|| {
                    let photos_ref = photos.borrow();
                    let photo = photos_ref.get(index)?;
                    let key = photo_presentation_key(photo)?;
                    folder_thumbnail_cache_get(&key)
                });
                let source = source_paintable
                    .map(|paintable| (tile_for_click.clone().upcast::<gtk::Widget>(), paintable));
                let photos = photos.borrow().clone();
                activate(photos, index, source);
            }
        });
        tile.add_controller(click);

        let right_click = gtk::GestureClick::new();
        right_click.set_button(3);
        right_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let index_for_context = index.clone();
        let selection_for_context = self.selection.clone();
        let photos_for_context = self.current_photos.clone();
        let context_menu = self.context_menu.clone();
        let tile_for_context = tile.clone();
        right_click.connect_pressed(move |gesture, _, x, y| {
            let Some(position) = index_for_context.get() else {
                return;
            };
            let Some(photo) = photos_for_context.borrow().get(position as usize).cloned() else {
                return;
            };
            if !selection_for_context.is_selected(position) {
                selection_for_context.select_item(position, true);
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            context_menu(
                photo,
                tile_for_context.clone().upcast::<gtk::Widget>(),
                x,
                y,
            );
        });
        tile.add_controller(right_click);

        SectionedFolderTile { tile, index }
    }

    fn refresh(self: &Rc<Self>) {
        let Some(scrolled) = self.scroll.borrow().as_ref().cloned() else {
            return;
        };
        let width = scrolled.width().max(1);
        self.geometry_for_current_layout(width);
        self.spacer
            .set_size_request(width, self.total_height.get().ceil() as i32);

        let adjustment = scrolled.vadjustment();
        let top = (adjustment.value() - SECTIONED_OVERSCAN_PX).max(0.0);
        let bottom = adjustment.value() + adjustment.page_size() + SECTIONED_OVERSCAN_PX;
        let columns = self.current_columns.get().max(1);
        let row_height = f64::from(folder_line_height(
            self.tile_height.get(),
            self.show_file_names.get(),
        ));
        let ranges = self.group_ranges.borrow();
        let geometry = self.geometry.borrow();

        let mut wanted_headers = Vec::<usize>::new();
        let mut wanted_tiles = Vec::<(u32, usize, u32, u32)>::new();
        if self.is_wall() {
            let wall = self.wall_state.borrow();
            for row in wall.layout.visible_rows(top, bottom) {
                for item in &wall.layout.items[wall.layout.rows[row].item_range.clone()] {
                    wanted_tiles.push((item.photo_index as u32, item.section, row as u32, 0));
                }
            }
            if self.group_mode.get() == GroupMode::Folder {
                wanted_headers.extend(visible_section_span(&geometry, top, bottom));
            }
        }

        // Section geometry is ordered by Y. Jump to the first section that
        // overlaps the overscan band and stop after its final section instead
        // of copying and scanning the complete folder catalog on every scroll.
        let section_span = visible_section_span(&geometry, top, bottom);
        for section_index in section_span {
            if self.is_wall() {
                break;
            }
            let range = &ranges[section_index];
            let geom = &geometry[section_index];
            wanted_headers.push(section_index);
            let count = range.end.saturating_sub(range.start) as u32;
            if count == 0 {
                continue;
            }
            let start_row = if top <= geom.first_photo_y {
                0
            } else {
                ((top - geom.first_photo_y) / row_height).floor().max(0.0) as u32
            };
            let end_row = (((bottom - geom.first_photo_y) / row_height).ceil().max(0.0) as u32)
                .min(count.div_ceil(columns));

            for row in start_row..end_row {
                let row_start = range.start as u32 + row * columns;
                for col in 0..columns {
                    let index = row_start + col;
                    if index >= range.end as u32 {
                        break;
                    }
                    wanted_tiles.push((index, section_index, row, col));
                }
            }
        }

        let wanted_ids = wanted_tiles
            .iter()
            .map(|item| item.0)
            .collect::<HashSet<_>>();
        let stale = self
            .live_tiles
            .borrow()
            .keys()
            .copied()
            .filter(|index| !wanted_ids.contains(index))
            .collect::<Vec<_>>();
        if !self.reflow_active.get() {
            for index in stale {
                // Unparenting can emit GTK callbacks that refresh the viewport.
                let tile = self.live_tiles.borrow_mut().remove(&index);
                if let Some(tile) = tile {
                    tile.tile.clear_wall_quality();
                    self.root.remove(&tile.tile);
                    tile.tile.set_opacity(1.0);
                    tile.tile.set_presentation_scale(1.0);
                    tile.index.set(None);
                    let mut pool = self.tile_pool.borrow_mut();
                    if pool.len() < SECTIONED_TILE_POOL_CAP {
                        pool.push_back(tile);
                    }
                }
            }
        }

        let photos = self.current_photos.borrow();
        for (index, section_index, row, col) in wanted_tiles {
            let existing = {
                let live = self.live_tiles.borrow();
                live.get(&index).cloned()
            };
            let was_existing = existing.is_some();
            let tile = if let Some(tile) = existing {
                tile
            } else {
                let pooled_tile = self.tile_pool.borrow_mut().pop_front();
                let tile = pooled_tile.unwrap_or_else(|| self.make_tile());
                let Some(photo) = photos.get(index as usize) else {
                    continue;
                };
                tile.tile.set_presentation_scale(1.0);
                tile.index.set(Some(index));
                tile.tile.set_opacity(1.0);
                tile.tile
                    .set_tile_size(self.tile_width.get(), self.tile_height.get());
                tile.tile
                    .set_filename_visible(self.show_file_names.get() && !self.is_wall());
                tile.tile
                    .set_content_fit(if !self.is_wall() && self.fit_whole_photo.get() {
                        gtk::ContentFit::Contain
                    } else {
                        gtk::ContentFit::Cover
                    });
                tile.tile.bind_photo_folder_fast(photo, index as usize);
                if self.strip_layer.borrow().is_some() {
                    tile.tile.set_opacity(0.0);
                }
                tile.tile
                    .set_manual_selected(self.selection.is_selected(index));
                self.root.put(&tile.tile, 0.0, 0.0);
                self.live_tiles.borrow_mut().insert(index, tile.clone());
                tile
            };

            if self.is_wall() {
                tile.tile.add_css_class("photo-wall-tile");
                tile.tile.set_overflow(gtk::Overflow::Hidden);
            } else {
                tile.tile.remove_css_class("photo-wall-tile");
                tile.tile.set_overflow(gtk::Overflow::Visible);
            }
            tile.tile
                .set_filename_visible(self.show_file_names.get() && !self.is_wall());
            tile.tile
                .set_manual_selected(self.selection.is_selected(index));

            // While a reflow is active, the frame-clock callback owns position
            // and size for already-realized tiles. A normal allocation/scroll
            // refresh must not teleport those widgets to their destination.
            // Newly realized overscan tiles still start at their correct target.
            if !self.reflow_active.get() || !was_existing {
                if self.is_wall() {
                    if let Some(item) = self.wall_state.borrow().layout.item(index as usize) {
                        for side in [
                            gtk::PositionType::Left,
                            gtk::PositionType::Right,
                            gtk::PositionType::Top,
                            gtk::PositionType::Bottom,
                        ] {
                            match side {
                                gtk::PositionType::Left => tile.tile.set_margin_start(0),
                                gtk::PositionType::Right => tile.tile.set_margin_end(0),
                                gtk::PositionType::Top => tile.tile.set_margin_top(0),
                                _ => tile.tile.set_margin_bottom(0),
                            }
                        }
                        tile.tile.set_tile_size(
                            item.width as i32,
                            item.height as i32,
                        );
                        tile.tile.set_content_fit(gtk::ContentFit::Cover);
                        tile.tile.set_presentation_scale(1.0);
                        tile.tile.set_presentation_offset(0.0, 0.0);
                        self.root.move_(&tile.tile, item.x, item.y);
                    }
                    continue;
                }
                tile.tile.set_margin_start(FOLDER_ITEM_MARGIN);
                tile.tile.set_margin_end(FOLDER_ITEM_MARGIN);
                tile.tile.set_margin_top(FOLDER_ITEM_MARGIN);
                tile.tile.set_margin_bottom(FOLDER_ITEM_MARGIN);
                tile.tile
                    .set_tile_size(self.tile_width.get(), self.tile_height.get());
                let (start_x, gap) = self.horizontal_grid_metrics(self.geometry_width.get());
                let x = start_x + f64::from(col) * (f64::from(self.tile_width.get()) + gap);
                let y = geometry[section_index].first_photo_y + f64::from(row) * row_height;
                self.root.move_(&tile.tile, x, y);
            }
        }
        drop(photos);

        let wanted_header_ids = wanted_headers.iter().copied().collect::<HashSet<_>>();
        if !self.preserve_headers_during_reflow.get() {
            let stale_headers = self
                .live_headers
                .borrow()
                .keys()
                .copied()
                .filter(|index| !wanted_header_ids.contains(index))
                .collect::<Vec<_>>();
            for index in stale_headers {
                if let Some(label) = self.live_headers.borrow_mut().remove(&index) {
                    self.root.remove(&label);
                    let mut pool = self.header_pool.borrow_mut();
                    if pool.len() < SECTIONED_HEADER_POOL_CAP {
                        pool.push_back(label);
                    }
                }
                if let Some(button) = self.live_pair_buttons.borrow_mut().remove(&index) {
                    self.root.remove(&button);
                    let mut pool = self.pair_button_pool.borrow_mut();
                    if pool.len() < SECTIONED_HEADER_POOL_CAP {
                        pool.push_back(button);
                    }
                }
            }
        }

        // Headings belong to the first tile column, not to the fixed side
        // margin. The tile grid centers itself whenever spare width remains
        // (the smallest zoom levels clamp the column count, so a lot of width
        // is left over in fullscreen), and a margin-pinned title slid away
        // from its own row as the window or zoom level changed (issue #104).
        let (heading_x, _) = if self.is_wall() {
            (SECTIONED_SIDE_MARGIN, 4.0)
        } else {
            self.horizontal_grid_metrics(width)
        };
        let heading_width =
            (width - heading_x.ceil() as i32 - FOLDER_ITEM_MARGIN - SECTIONED_SIDE_MARGIN as i32)
                .max(1);
        for section_index in wanted_headers {
            let existing = {
                let live = self.live_headers.borrow();
                live.get(&section_index).cloned()
            };
            let header_was_existing = existing.is_some();
            let label = if let Some(label) = existing {
                label
            } else {
                let label = self
                    .header_pool
                    .borrow_mut()
                    .pop_front()
                    .unwrap_or_else(|| {
                        let label = gtk::Label::new(None);
                        label.set_xalign(0.0);
                        label.set_yalign(0.5);
                        // Same margin as the tiles: the heading is placed at the
                        // column position, and GTK offsets a child by its margin.
                        label.set_margin_start(FOLDER_ITEM_MARGIN);
                        label.add_css_class("section-heading");
                        label.add_css_class("folder-section-heading");
                        label
                    });
                let range = &ranges[section_index];
                label.set_text(&format!(
                    "{}   ·   {} photos",
                    range.label,
                    range.end.saturating_sub(range.start)
                ));
                self.root.put(&label, heading_x, 0.0);
                self.live_headers
                    .borrow_mut()
                    .insert(section_index, label.clone());
                label
            };
            // Headers survive a photo-model refresh so a focused pair toggle
            // keeps its allocation and focus. Refresh its count in place.
            let range = &ranges[section_index];
            label.set_text(&format!(
                "{}   ·   {} photos",
                range.label,
                range.end.saturating_sub(range.start)
            ));
            label.set_size_request(heading_width, SECTIONED_HEADER_HEIGHT as i32);
            if !self.reflow_active.get() || !header_was_existing {
                self.root
                    .move_(&label, heading_x, geometry[section_index].header_y);
            }

            let range = &ranges[section_index];
            let show_pair_toggle = self
                .raw_jpeg_pair_folders
                .borrow()
                .contains(&range.folder_id);
            let existing_button = {
                let live = self.live_pair_buttons.borrow();
                live.get(&section_index).cloned()
            };
            let button = if let Some(button) = existing_button {
                button
            } else {
                let button = self
                    .pair_button_pool
                    .borrow_mut()
                    .pop_front()
                    .unwrap_or_else(|| {
                        let button = gtk::Button::new();
                        button.add_css_class("flat");
                        button.add_css_class("raw-jpeg-pair-toggle");
                        button.set_tooltip_text(Some(
                            "RAW + JPEG pair view: click to cycle Both / JPEG / RAW",
                        ));
                        let mode = self.raw_jpeg_mode.clone();
                        let changed = self.raw_jpeg_mode_changed.clone();
                        button.connect_clicked(move |button| {
                            let next = match mode.get() {
                                crate::image_format::RawJpegPairMode::Both => {
                                    crate::image_format::RawJpegPairMode::PreferJpeg
                                }
                                crate::image_format::RawJpegPairMode::PreferJpeg => {
                                    crate::image_format::RawJpegPairMode::PreferRaw
                                }
                                crate::image_format::RawJpegPairMode::PreferRaw => {
                                    crate::image_format::RawJpegPairMode::Both
                                }
                            };
                            mode.set(next);
                            button.set_label(match next {
                                crate::image_format::RawJpegPairMode::Both => "BOTH",
                                crate::image_format::RawJpegPairMode::PreferJpeg => "JPG",
                                crate::image_format::RawJpegPairMode::PreferRaw => "RAW",
                            });
                            if let Some(handler) = changed.borrow().as_ref() {
                                handler(next);
                            }
                        });
                        button
                    });
                self.root.put(&button, 0.0, 0.0);
                self.live_pair_buttons
                    .borrow_mut()
                    .insert(section_index, button.clone());
                button
            };
            button.set_label(match self.raw_jpeg_mode.get() {
                crate::image_format::RawJpegPairMode::Both => "BOTH",
                crate::image_format::RawJpegPairMode::PreferJpeg => "JPG",
                crate::image_format::RawJpegPairMode::PreferRaw => "RAW",
            });
            button.set_visible(show_pair_toggle);
            button.set_size_request(64, 30);
            if show_pair_toggle && (!self.reflow_active.get() || !header_was_existing) {
                self.root.move_(
                    &button,
                    heading_x + f64::from((heading_width - 68).max(0)),
                    geometry[section_index].header_y + 20.0,
                );
            }
        }
        drop(geometry);
        drop(ranges);
    }

    fn sync_selection(&self) {
        for (index, tile) in self.live_tiles.borrow().iter() {
            tile.tile.set_manual_selected(self.selection.is_selected(*index));
        }
    }

    fn refresh_model(self: &Rc<Self>) {
        self.clear_strip_layer();
        self.reflow_animation_generation
            .set(self.reflow_animation_generation.get().wrapping_add(1));
        set_grid_zoom_animation_active(false);
        self.reflow_active.set(false);
        self.preserve_headers_during_reflow.set(false);
        // A replacement can keep the same numeric positions while changing
        // PhotoObject metadata. Recycle the bounded realized set so every
        // visible tile is rebound exactly once to the current model.
        let live = std::mem::take(&mut *self.live_tiles.borrow_mut());
        for (_, tile) in live {
            tile.tile.clear_wall_quality();
            self.root.remove(&tile.tile);
            tile.tile.set_opacity(1.0);
            tile.tile.set_presentation_scale(1.0);
            tile.index.set(None);
            let mut pool = self.tile_pool.borrow_mut();
            if pool.len() < SECTIONED_TILE_POOL_CAP {
                pool.push_back(tile);
            }
        }
        // Keep the bounded header/button set alive. Removing the focused
        // RAW/JPEG button makes GTK move focus and scroll while the model is
        // rebuilt. refresh() updates labels and recycles only offscreen rows.
        self.invalidate_geometry();
        self.refresh();
    }

    fn capture_reflow_snapshot(&self) -> SectionedReflowSnapshot {
        let presentation = self.strip_presentation.borrow().clone();
        let mut photos = presentation.as_ref().map(|p| p.photos.clone()).unwrap_or_default();
        let mut visual_px = HashMap::new();
        for (index, entry) in self.live_tiles.borrow().iter() {
            visual_px.insert(
                *index,
                entry.tile.presentation_scale() * self.tile_width.get().max(1) as f32,
            );
            if !photos.contains_key(index) {
                if let Some(photo) = freeze_strip_photo(&entry.tile) { photos.insert(*index, photo); }
            }
        }
        SectionedReflowSnapshot {
            old_columns: self.current_columns.get().max(1),
            old_geometry: self.geometry.borrow().clone(),
            tile_width: self.tile_width.get(),
            tile_height: self.tile_height.get(),
            old_scroll_y: self.scroll_position(),
            old_width: self.geometry_width.get().max(1),
            presentation,
            photos,
            visual_px,
        }
    }

    fn horizontal_grid_metrics(&self, width: i32) -> (f64, f64) {
        Self::grid_metrics_for(width, self.current_columns.get(), self.tile_width.get())
    }

    fn grid_metrics_for(width: i32, columns: u32, tile_width: i32) -> (f64, f64) {
        let columns = columns.max(1);
        let tile_width = f64::from(tile_width.max(1));
        let width = f64::from(width.max(1));
        let available = (width - SECTIONED_SIDE_MARGIN * 2.0).max(tile_width);
        if columns <= 1 {
            return (((width - tile_width) * 0.5).max(SECTIONED_SIDE_MARGIN), 0.0);
        }

        // Keep the grid visually balanced as the sidebar/window changes width.
        // The old fixed 30px stride left all spare width on the right.
        let raw_gap = (available - tile_width * f64::from(columns))
            / f64::from(columns - 1);
        let gap = raw_gap.clamp(18.0, 54.0);
        let used = tile_width * f64::from(columns)
            + gap * f64::from(columns - 1);
        let start_x = ((width - used) * 0.5).max(SECTIONED_SIDE_MARGIN);
        (start_x, gap)
    }

    fn section_index_for_photo(&self, index: u32) -> Option<usize> {
        if self.is_wall() {
            return self
                .wall_state
                .borrow()
                .layout
                .item(index as usize)
                .map(|item| item.section);
        }
        let ranges = self.group_ranges.borrow();
        section_index_for_photo(&ranges, index as usize)
    }

    /// Apply a section reflow as one deterministic layout change.
    ///
    /// Folder headers are ordinary in-flow section headers, not sticky
    /// overlays. When a visible header exists, preserve its viewport-relative
    /// Y exactly while the ordered photo strip underneath it re-wraps. This is
    /// intentionally non-animated: interpolating section/header geometry made
    /// the title drift while thumbnails appeared to fly between rows.
    fn apply_reflow_without_animation(
        self: &Rc<Self>,
        snapshot: SectionedReflowSnapshot,
        anchor: Option<(i64, f64)>,
    ) {
        self.reflow_animation_generation
            .set(self.reflow_animation_generation.get().wrapping_add(1));
        set_grid_zoom_animation_active(false);
        self.clear_strip_layer();
        self.reflow_active.set(false);
        self.preserve_headers_during_reflow.set(false);

        let header_anchor = self
            .scroll
            .borrow()
            .as_ref()
            .and_then(|scroll| {
                let page = scroll.vadjustment().page_size();
                let top = snapshot.old_scroll_y;
                let bottom = top + page;
                snapshot
                    .old_geometry
                    .iter()
                    .enumerate()
                    // A header is visible when any part of its 70 px box
                    // overlaps the viewport. Checking only header_y missed
                    // partially clipped titles at the top edge and caused the
                    // zoom path to fall back to a photo anchor, making the
                    // visible title jump.
                    .filter(|(_, geom)| {
                        geom.header_y < bottom
                            && geom.header_y + f64::from(SECTIONED_HEADER_HEIGHT) > top
                    })
                    .min_by(|(_, a), (_, b)| {
                        (a.header_y - top)
                            .abs()
                            .partial_cmp(&(b.header_y - top).abs())
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(section, geom)| (section, geom.header_y - top))
            });

        self.invalidate_geometry();
        self.refresh();

        let mut restored_header = false;
        if let Some((section, viewport_y)) = header_anchor {
            let new_header_y = self
                .geometry
                .borrow()
                .get(section)
                .map(|geom| geom.header_y);
            if let (Some(new_header_y), Some(scroll)) =
                (new_header_y, self.scroll.borrow().as_ref().cloned())
            {
                let adjustment = scroll.vadjustment();
                let upper =
                    (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
                let target = (new_header_y - viewport_y)
                    .clamp(adjustment.lower(), upper);
                adjustment.set_value(target);
                self.refresh();
                restored_header = true;

                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_SECTIONED_REFLOW header_stable section={} viewport_y={:.1} old_scroll={:.1} new_scroll={:.1} old_columns={} new_columns={} old_tile={}x{} new_tile={}x{}",
                        section,
                        viewport_y,
                        snapshot.old_scroll_y,
                        target,
                        snapshot.old_columns,
                        self.current_columns.get(),
                        snapshot.tile_width,
                        snapshot.tile_height,
                        self.tile_width.get(),
                        self.tile_height.get(),
                    );
                }
            }
        }

        if !restored_header {
            if let Some((photo_id, offset)) = anchor {
                let restored = self.restore_anchor(photo_id, offset);
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_SECTIONED_REFLOW photo_anchor photo_id={} offset={:.1} restored={} old_columns={} new_columns={}",
                        photo_id,
                        offset,
                        restored,
                        snapshot.old_columns,
                        self.current_columns.get(),
                    );
                }
            }
        }

        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_SECTIONED_REFLOW complete animated=false header_anchor={} old_scroll={:.1} final_scroll={:.1} old_columns={} new_columns={}",
                restored_header,
                snapshot.old_scroll_y,
                self.scroll_position(),
                snapshot.old_columns,
                self.current_columns.get(),
            );
        }
    }

    fn animate_reflow(
        self: &Rc<Self>,
        snapshot: SectionedReflowSnapshot,
        anchor: Option<(i64, f64)>,
    ) {
        self.apply_reflow_without_animation(snapshot, anchor);
    }

    fn animate_zoom_reflow(
        self: &Rc<Self>,
        snapshot: SectionedReflowSnapshot,
        anchor: Option<(i64, f64)>,
    ) {
        const ZOOM_SETTLE_US: i64 = 180_000;
        const COLUMN_SETTLE_US: i64 = 220_000;
        const COLUMN_GROW_START: f32 = 0.88;
        const COLUMN_SHRINK_START: f32 = 1.12;

        let old_columns = snapshot.old_columns;
        let old_width = snapshot.tile_width.max(1) as f32;
        let old_visual_px = snapshot.visual_px.clone();
        self.apply_reflow_without_animation(snapshot, anchor);

        self.reflow_animation_generation
            .set(self.reflow_animation_generation.get().wrapping_add(1));
        let generation = self.reflow_animation_generation.get();
        for entry in self.live_tiles.borrow().values() {
            entry.tile.set_presentation_scale(1.0);
        }
        if !self.zoom_animations_enabled.get()
            || !self.root.is_mapped()
            || !self.root.settings().is_gtk_enable_animations()
        {
            return;
        }

        let new_columns = self.current_columns.get().max(1);
        let columns_changed = old_columns != new_columns;
        let new_width = self.tile_width.get().max(1) as f32;
        let normal_start = (old_width / new_width).clamp(0.25, 4.0);
        let column_start = if new_columns > old_columns {
            COLUMN_GROW_START
        } else {
            COLUMN_SHRINK_START
        };
        let known = Rc::new(RefCell::new(HashSet::<u32>::new()));
        for (index, entry) in self.live_tiles.borrow().iter() {
            known.borrow_mut().insert(*index);
            entry.tile.set_presentation_scale(if let Some(visual_px) = old_visual_px.get(index) {
                (*visual_px / new_width).clamp(0.25, 4.0)
            } else if columns_changed {
                column_start
            } else {
                normal_start
            });
        }
        set_grid_zoom_animation_active(true);

        let weak = Rc::downgrade(self);
        let started_at = Cell::new(None);
        self.root.add_tick_callback(move |_, clock| {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if view.reflow_animation_generation.get() != generation
                || !view.zoom_animations_enabled.get()
            {
                return glib::ControlFlow::Break;
            }
            let start_time = started_at.get().unwrap_or_else(|| {
                let now = clock.frame_time();
                started_at.set(Some(now));
                now
            });
            let elapsed = clock.frame_time().saturating_sub(start_time);
            let normal_t = (elapsed as f64 / ZOOM_SETTLE_US as f64).clamp(0.0, 1.0);
            let normal_scale = normal_start
                + (1.0 - normal_start) * ease_in_out_cubic(normal_t) as f32;
            let column_t = (elapsed as f64 / COLUMN_SETTLE_US as f64).clamp(0.0, 1.0);
            let column_scale = column_start
                + (1.0 - column_start) * ease_in_out_cubic(column_t) as f32;

            // Folder virtualization can realize the new edge column after the
            // destination reflow. Enrol those tiles at the current animation
            // scale so they do not pop into view at full size.
            for (index, entry) in view.live_tiles.borrow().iter() {
                let is_new = known.borrow_mut().insert(*index);
                let scale = if let Some(visual_px) = old_visual_px.get(index) {
                    let start = (*visual_px / new_width).clamp(0.25, 4.0);
                    start + (1.0 - start) * ease_in_out_cubic(normal_t) as f32
                } else if columns_changed {
                    column_scale
                } else {
                    normal_scale
                };
                if is_new || (entry.tile.presentation_scale() - 1.0).abs() > 0.001 {
                    entry.tile.set_presentation_scale(scale);
                }
            }

            let duration = if columns_changed {
                ZOOM_SETTLE_US.max(COLUMN_SETTLE_US)
            } else {
                ZOOM_SETTLE_US
            };
            if elapsed >= duration {
                for entry in view.live_tiles.borrow().values() {
                    entry.tile.set_presentation_scale(1.0);
                }
                set_grid_zoom_animation_active(false);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn cancel_zoom_settle(&self) {
        self.reflow_animation_generation
            .set(self.reflow_animation_generation.get().wrapping_add(1));
        for entry in self.live_tiles.borrow().values() {
            entry.tile.set_presentation_scale(1.0);
        }
        set_grid_zoom_animation_active(false);
    }

    fn clear_strip_layer(&self) {
        if let Some(layer) = self.strip_layer.borrow_mut().take() { self.root.remove(&layer); }
        self.strip_presentation.borrow_mut().take();
        for entry in self.live_tiles.borrow().values() { entry.tile.set_opacity(1.0); }
    }

    fn paint_strip(&self, mut presentation: StripPresentation, ranges: &[GroupRange]) {
        for (index, entry) in self.live_tiles.borrow().iter() {
            if !presentation.photos.contains_key(index) {
                if let Some(photo) = freeze_strip_photo(&entry.tile) {
                    presentation.photos.insert(*index, photo);
                }
            }
            entry.tile.set_opacity(0.0);
        }
        let Some(layer) = self.strip_layer.borrow().as_ref().cloned() else { return; };
        let scroll = self.scroll.borrow();
        let Some(scroll) = scroll.as_ref() else { return; };
        let top = (scroll.vadjustment().value() - SECTIONED_OVERSCAN_PX).max(0.0);
        let page = scroll.vadjustment().page_size();
        let height = (page + SECTIONED_OVERSCAN_PX * 2.0)
            .min((self.total_height.get().max(page) - top).max(1.0));
        let slices = strip_slices(&presentation.layout, ranges, top, top + height);
        let draws = slices.into_iter().filter_map(|slice| {
            presentation.photos.get(&slice.index).cloned().map(|photo| (slice, photo))
        }).collect();
        layer.imp().draws.replace(draws);
        layer.imp().size.set((presentation.layout.tile_width, presentation.layout.tile_height));
        layer.imp().top.set(top);
        layer.set_size_request(scroll.width().max(1), height.ceil() as i32);
        self.root.move_(&layer, 0.0, top);
        layer.queue_draw();
        for (section, label) in self.live_headers.borrow().iter() {
            if let Some(frame) = presentation.layout.sections.get(*section) {
                self.root.move_(label, presentation.layout.left, frame.header_y);
            }
        }
        self.strip_presentation.replace(Some(presentation));
    }


    fn capture_center_anchor(&self) -> Option<(i64, f64)> {
        if self.is_wall() {
            return self.wall_center_anchor();
        }
        let scrolled = self.scroll.borrow().as_ref()?.clone();
        let adjustment = scrolled.vadjustment();
        let scroll_y = adjustment.value();
        let lower = adjustment.lower();
        let row_height = f64::from(folder_line_height(self.tile_height.get(), self.show_file_names.get()));
        if let Some((photo_index, offset)) = upper_edge_anchor(
            &self.group_ranges.borrow(),
            &self.geometry.borrow(),
            scroll_y,
            lower,
            row_height,
        ) {
            // The first photo row stays at the same content Y as tile width
            // changes. Near the upper bound, anchoring the viewport center can
            // request a negative scroll value after zoom and get clamped. Use
            // this invariant row so the existing scroll position is preserved.
            if let Some(photo) = self.current_photos.borrow().get(photo_index) {
                return Some((photo.id(), offset));
            }
        }
        let target = adjustment.value() + adjustment.page_size() * 0.5;
        let columns = self.current_columns.get().max(1);
        let ranges = self.group_ranges.borrow();
        let geometry = self.geometry.borrow();
        let section_index = geometry.partition_point(|geom| geom.end_y <= target);
        if let (Some(range), Some(geom)) = (ranges.get(section_index), geometry.get(section_index))
        {
            if range.start == range.end {
                return None;
            }
            let row = if target <= geom.first_photo_y {
                0
            } else {
                ((target - geom.first_photo_y) / row_height)
                    .floor()
                    .max(0.0) as u32
            };
            let local = (row * columns).min(range.end.saturating_sub(range.start) as u32 - 1);
            let index = range.start as u32 + local;
            let photo = self.current_photos.borrow().get(index as usize)?.clone();
            let y = geom.first_photo_y + f64::from(row) * row_height;
            return Some((photo.id(), y - scroll_y));
        }        None
    }

    fn capture_visible_anchors(&self) -> Vec<(i64, f64)> {
        let Some(scrolled) = self.scroll.borrow().as_ref().cloned() else {
            return Vec::new();
        };
        let adjustment = scrolled.vadjustment();
        let scroll_y = adjustment.value();
        let viewport_bottom = scroll_y + adjustment.page_size();
        let row_height = f64::from(folder_line_height(
            self.tile_height.get(),
            self.show_file_names.get(),
        ));
        let mut anchors = self
            .live_tiles
            .borrow()
            .keys()
            .filter_map(|index| {
                let row_y = self.y_for_index(*index)?;
                if row_y + row_height < scroll_y || row_y > viewport_bottom {
                    return None;
                }
                let photo_id = self.current_photos.borrow().get(*index as usize)?.id();
                Some((photo_id, row_y - scroll_y))
            })
            .collect::<Vec<_>>();
        // Keep the photo nearest the viewport center first. If filtering removes
        // every photo in the partially visible top row, restoring the first
        // survivor can otherwise move the viewport by several rows.
        let viewport_center = adjustment.page_size() * 0.5;
        anchors.sort_by(|left, right| {
            (left.1 - viewport_center)
                .abs()
                .total_cmp(&(right.1 - viewport_center).abs())
        });
        anchors
    }

    fn capture_visible_header_anchor(&self) -> Option<(i64, f64)> {
        let scrolled = self.scroll.borrow().as_ref()?.clone();
        let adjustment = scrolled.vadjustment();
        let scroll_y = adjustment.value();
        let page = adjustment.page_size();
        let ranges = self.group_ranges.borrow();
        let geometry = self.geometry.borrow();
        let focused_section = self.live_pair_buttons.borrow().iter()
            .find_map(|(section, button)| button.has_focus().then_some(*section));
        ranges.iter().zip(geometry.iter()).enumerate()
            .filter_map(|(section, (range, geom))| {
                let offset = geom.header_y - scroll_y;
                (offset + SECTIONED_HEADER_HEIGHT > 0.0 && offset < page)
                    .then_some((section, range.folder_id, offset))
            })
            .min_by(|left, right| {
                (focused_section != Some(left.0)).cmp(&(focused_section != Some(right.0)))
                    .then_with(|| (left.2 - page * 0.5).abs()
                        .total_cmp(&(right.2 - page * 0.5).abs()))
            })
            .map(|(_, folder_id, offset)| (folder_id, offset))
    }

    fn restore_header_anchor(self: &Rc<Self>, folder_id: i64, offset: f64) -> bool {
        self.refresh();
        let Some(section) = self.group_ranges.borrow().iter()
            .position(|range| range.folder_id == folder_id) else {
            return false;
        };
        let Some(header_y) = self.geometry.borrow().get(section).map(|geom| geom.header_y) else {
            return false;
        };
        let restored = self.set_scroll_y(header_y - offset);
        if restored && std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!("PIC_NAV viewport_header_restored folder_id={} offset={:.1} scroll_y={:.1}",
                folder_id, offset, self.scroll_position());
        }
        restored
    }

    fn restore_anchor(self: &Rc<Self>, photo_id: i64, offset: f64) -> bool {
        let Some(index) = self
            .current_photos
            .borrow()
            .iter()
            .position(|photo| photo.id() == photo_id)
        else {
            return false;
        };
        self.refresh();
        let Some(y) = self.y_for_index(index as u32) else {
            return false;
        };
        let Some(scrolled) = self.scroll.borrow().as_ref().cloned() else {
            return false;
        };
        let adjustment = scrolled.vadjustment();
        if !sectioned_scroll_extent_is_ready(
            self.total_height.get(),
            adjustment.page_size(),
            adjustment.upper(),
        ) {
            return false;
        }
        let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        let previous = adjustment.value();
        let target = (y - offset).clamp(adjustment.lower(), upper);
        adjustment.set_value(target);
        self.refresh();
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_NAV viewport_anchor_restored photo_id={} index={} offset={:.1} before={:.1} target={:.1} applied={:.1}",
                photo_id,
                index,
                offset,
                previous,
                target,
                adjustment.value(),
            );
        }
        true
    }

    fn y_for_index(&self, index: u32) -> Option<f64> {
        if self.is_wall() {
            return self
                .wall_state
                .borrow()
                .layout
                .item(index as usize)
                .map(|item| item.y);
        }
        let columns = self.current_columns.get().max(1);
        let row_height = f64::from(folder_line_height(self.tile_height.get(), self.show_file_names.get()));
        let section_index = self.section_index_for_photo(index)?;
        let ranges = self.group_ranges.borrow();
        let geometry = self.geometry.borrow();
        let range = ranges.get(section_index)?;
        let geom = geometry.get(section_index)?;
        let local = index as usize - range.start;
        Some(geom.first_photo_y + (local as u32 / columns) as f64 * row_height)
    }

    fn column_for_index(&self, index: u32) -> Option<u32> {
        if self.is_wall() {
            return self
                .wall_state
                .borrow()
                .layout
                .item(index as usize)
                .map(|item| item.x.round() as u32);
        }
        let columns = self.current_columns.get().max(1);
        let ranges = self.group_ranges.borrow();
        let section = section_index_for_photo(&ranges, index as usize)?;
        let range = ranges.get(section)?;
        let local = index as usize - range.start;
        Some((local as u32) % columns)
    }

    fn cancel_scroll_animation(&self) {
        self.scroll_animation_generation
            .set(self.scroll_animation_generation.get().wrapping_add(1));
    }

    fn smooth_keep_index_in_center_zone(self: &Rc<Self>, index: u32) -> bool {
        if self.is_wall() {
            return self.wall_scroll_to(index, false, false);
        }
        self.refresh();
        let Some(row_top) = self.y_for_index(index) else {
            return false;
        };
        let Some(scrolled) = self.scroll.borrow().as_ref().cloned() else {
            return false;
        };
        let adjustment = scrolled.vadjustment();
        let page = adjustment.page_size().max(1.0);
        let row_height = f64::from(folder_line_height(
            self.tile_height.get(),
            self.show_file_names.get(),
        ));
        let row_center = row_top + row_height * 0.5;
        let current_top = adjustment.value();
        let comfort_top = current_top + page * 0.35;
        let comfort_bottom = current_top + page * 0.65;

        if row_center >= comfort_top && row_center <= comfort_bottom {
            return true;
        }

        let lower = adjustment.lower();
        let upper = (adjustment.upper() - page).max(lower);
        let target = (row_center - page * 0.5).clamp(lower, upper);
        self.animate_scroll_to(target, index, "keyboard");
        true
    }

    fn animate_scroll_to(self: &Rc<Self>, target: f64, index: u32, source: &'static str) {
        let Some(scrolled) = self.scroll.borrow().as_ref().cloned() else {
            return;
        };
        let adjustment = scrolled.vadjustment();
        let start = adjustment.value();
        if (target - start).abs() < 0.5 {
            return;
        }

        let generation = self.scroll_animation_generation.get().wrapping_add(1);
        self.scroll_animation_generation.set(generation);
        let weak = Rc::downgrade(self);
        let started = Instant::now();
        const DURATION_MS: f64 = 190.0;

        self.root.add_tick_callback(move |_, _| {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if view.scroll_animation_generation.get() != generation {
                return glib::ControlFlow::Break;
            }

            let t = (started.elapsed().as_secs_f64() * 1000.0 / DURATION_MS).clamp(0.0, 1.0);
            // Cubic ease-in-out: gentle start, quick middle, soft landing.
            let eased = if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            };
            adjustment.set_value(start + (target - start) * eased);
            view.refresh();

            if t >= 1.0 {
                adjustment.set_value(target);
                view.refresh();
                if let Some(photo_id) = view
                    .current_photos
                    .borrow()
                    .get(index as usize)
                    .map(PhotoObject::id)
                {
                    view.focus_photo(photo_id);
                }
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_SECTIONED_SCROLL source={} index={} generation={} from_y={:.1} target_y={:.1} eased=true",
                        source,
                        index,
                        generation,
                        start,
                        target
                    );
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn reveal_index_if_needed(self: &Rc<Self>, index: u32) -> bool {
        if self.is_wall() {
            return self.wall_scroll_to(index, false, false);
        }
        self.refresh();
        let section_index = self.section_index_for_photo(index);
        let ranges = self.group_ranges.borrow();
        let geometry = self.geometry.borrow();
        let row_height = f64::from(folder_line_height(
            self.tile_height.get(),
            self.show_file_names.get(),
        ));
        let row_top = section_index.and_then(|section_index| {
            let range = ranges.get(section_index)?;
            let geom = geometry.get(section_index)?;
            let local = index as usize - range.start;
            let row = local as u32 / self.current_columns.get().max(1);
            Some(geom.first_photo_y + f64::from(row) * row_height)
        });
        drop(geometry);
        drop(ranges);

        let Some(row_top) = row_top else {
            return false;
        };
        let Some(scrolled) = self.scroll.borrow().as_ref().cloned() else {
            return false;
        };
        let adjustment = scrolled.vadjustment();
        let page = adjustment.page_size().max(1.0);
        let current_top = adjustment.value();
        let current_bottom = current_top + page;
        let row_bottom = row_top + row_height;

        let target = if row_top < current_top {
            Some(row_top)
        } else if row_bottom > current_bottom {
            Some(row_bottom - page)
        } else {
            None
        };

        if let Some(target) = target {
            let lower = adjustment.lower();
            let upper = (adjustment.upper() - page).max(lower);
            adjustment.set_value(target.clamp(lower, upper));
            self.refresh();
        }
        true
    }

    fn scroll_to_index(self: &Rc<Self>, index: u32, header: bool) -> bool {
        if self.is_wall() {
            return self.wall_scroll_to(index, header, false);
        }
        self.refresh();
        let section_index = self.section_index_for_photo(index);
        let ranges = self.group_ranges.borrow();
        let geometry = self.geometry.borrow();
        let row = section_index.and_then(|section_index| {
            let range = ranges.get(section_index)?;
            let geom = geometry.get(section_index)?;
            Some(if header {
                (geom.header_y, SECTIONED_HEADER_HEIGHT)
            } else {
                let local = index as usize - range.start;
                let row_index = local as u32 / self.current_columns.get().max(1);
                let row_height = f64::from(folder_line_height(
                    self.tile_height.get(),
                    self.show_file_names.get(),
                ));
                (
                    geom.first_photo_y + f64::from(row_index) * row_height,
                    row_height,
                )
            })
        });
        drop(geometry);
        drop(ranges);
        let Some((row_top, row_height)) = row else {
            return false;
        };
        let Some(scrolled) = self.scroll.borrow().as_ref().cloned() else {
            return false;
        };
        // Folder navigation can switch the Stack page and replace the model in
        // the same main-loop turn. At that point the ScrolledWindow may still
        // have its old (often zero) adjustment bounds. A clamped set_value()
        // used to look like success and stop Open in Folder's retry loop even
        // though the requested row remained thousands of pixels away.
        if !self.root.is_mapped() || !scrolled.is_mapped() {
            return false;
        }
        let adjustment = scrolled.vadjustment();
        let page = adjustment.page_size();
        if page <= 0.0 {
            return false;
        }
        let upper = (adjustment.upper() - page).max(adjustment.lower());
        adjustment.set_value(row_top.clamp(adjustment.lower(), upper));
        self.refresh();
        // The adjustment's upper bound can lag one allocation behind the new
        // section geometry. Report success only after the row is actually in
        // the viewport so the caller retries after GTK updates its bounds.
        viewport_intersects_row(
            row_top,
            row_height,
            adjustment.value(),
            adjustment.page_size(),
        )
    }

    fn scroll_to_index_smooth(self: &Rc<Self>, index: u32, header: bool) -> bool {
        if self.is_wall() {
            return self.wall_scroll_to(index, header, false);
        }
        self.scroll_animation_generation
            .set(self.scroll_animation_generation.get().wrapping_add(1));
        self.refresh();

        let section_index = self.section_index_for_photo(index);
        let ranges = self.group_ranges.borrow();
        let geometry = self.geometry.borrow();
        let target = section_index.and_then(|section_index| {
            let range = ranges.get(section_index)?;
            let geom = geometry.get(section_index)?;
            Some(if header {
                geom.header_y
            } else {
                let local = index as usize - range.start;
                let row = local as u32 / self.current_columns.get().max(1);
                geom.first_photo_y
                    + f64::from(row) * f64::from(folder_line_height(self.tile_height.get(), self.show_file_names.get()))
            })
        });
        drop(geometry);
        drop(ranges);

        let Some(target) = target else {
            return false;
        };
        let Some(scrolled) = self.scroll.borrow().as_ref().cloned() else {
            return false;
        };

        let adjustment = scrolled.vadjustment();
        let lower = adjustment.lower();
        let upper = (adjustment.upper() - adjustment.page_size()).max(lower);
        adjustment.set_value(target.clamp(lower, upper));
        self.refresh();

        if let Some(photo_id) = self
            .current_photos
            .borrow()
            .get(index as usize)
            .map(PhotoObject::id)
        {
            self.focus_photo(photo_id);
        }

        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_SECTIONED_JUMP index={} target_y={:.1} animated=false",
                index,
                target
            );
        }
        true
    }

    fn scroll_to_index_centered(self: &Rc<Self>, index: u32) -> bool {
        if self.is_wall() {
            return self.wall_scroll_to(index, false, true);
        }
        self.refresh();
        let Some(row_top) = self.y_for_index(index) else {
            return false;
        };
        let Some(scrolled) = self.scroll.borrow().as_ref().cloned() else {
            return false;
        };

        let adjustment = scrolled.vadjustment();
        let lower = adjustment.lower();
        let page = adjustment.page_size().max(1.0);
        if !sectioned_scroll_extent_is_ready(self.total_height.get(), page, adjustment.upper()) {
            return false;
        }
        let upper = (adjustment.upper() - page).max(lower);
        let row_height = f64::from(folder_line_height(
            self.tile_height.get(),
            self.show_file_names.get(),
        ));
        let row_center = row_top + row_height * 0.5;
        let target = (row_center - page * 0.5).clamp(lower, upper);

        // Lightbox return owns the viewport. Explicitly invalidate any older
        // keyboard/folder-scroll callback before starting the centered return.
        self.cancel_scroll_animation();
        self.animate_scroll_to(target, index, "lightbox_return");

        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_SECTIONED_CENTER index={} target_y={:.1} eased=true",
                index,
                target
            );
        }
        true
    }

    fn scroll_to_index_centered_now(self: &Rc<Self>, index: u32) -> bool {
        if self.is_wall() {
            return self.wall_scroll_to(index, false, true);
        }
        self.refresh();
        let target_photo = self
            .current_photos
            .borrow()
            .get(index as usize)
            .map(|photo| (photo.id(), photo.folder_id(), photo.path()));
        let Some(row_top) = self.y_for_index(index) else {
            return false;
        };
        let Some(scrolled) = self.scroll.borrow().as_ref().cloned() else {
            return false;
        };
        if !self.root.is_mapped() || !scrolled.is_mapped() {
            return false;
        }

        let adjustment = scrolled.vadjustment();
        let page = adjustment.page_size();
        if page <= 0.0 {
            return false;
        }
        let content_height = self.total_height.get();
        // Wait for GTK to allocate the new Folder content. Otherwise its old
        // Photos-page upper bound can clamp a deep target to the top and make
        // the caller believe a centered reveal succeeded.
        if !sectioned_scroll_extent_is_ready(content_height, page, adjustment.upper()) {
            return false;
        }

        let row_height = f64::from(folder_line_height(
            self.tile_height.get(),
            self.show_file_names.get(),
        ));
        let row_center = row_top + row_height * 0.5;
        let lower = adjustment.lower();
        let upper = (adjustment.upper() - page).max(lower);
        let target = (row_center - page * 0.5).clamp(lower, upper);
        self.cancel_scroll_animation();
        adjustment.set_value(target);
        self.refresh();
        let centered = (adjustment.value() - target).abs() <= 1.0;
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_SECTIONED_CENTER source=open_in_folder index={} photo={:?} target_y={:.1} applied_y={:.1} page={:.1} centered={}",
                index,
                target_photo,
                target,
                adjustment.value(),
                page,
                centered
            );
        }
        centered
            && viewport_intersects_row(row_top, row_height, adjustment.value(), page)
    }

    fn scroll_position(&self) -> f64 {
        self.scroll
            .borrow()
            .as_ref()
            .map(|scroll| scroll.vadjustment().value())
            .unwrap_or(0.0)
    }

    fn photo_for_scroll_position(&self, scroll_y: f64) -> Option<PhotoObject> {
        if self.is_wall() {
            return self.wall_photo_for_y(scroll_y);
        }
        let columns = self.current_columns.get().max(1);
        let row_height = f64::from(folder_line_height(self.tile_height.get(), self.show_file_names.get()));
        let ranges = self.group_ranges.borrow();
        let geometry = self.geometry.borrow();
        for (range, geom) in ranges.iter().zip(geometry.iter()) {
            if scroll_y >= geom.end_y || range.start == range.end {
                continue;
            }
            let row = if scroll_y <= geom.first_photo_y {
                0
            } else {
                ((scroll_y - geom.first_photo_y) / row_height)
                    .floor()
                    .max(0.0) as u32
            };
            let local = (row * columns).min(range.end.saturating_sub(range.start) as u32 - 1);
            return self
                .current_photos
                .borrow()
                .get(range.start + local as usize)
                .cloned();
        }
        None
    }

    fn viewport_center_photo(&self) -> Option<PhotoObject> {
        let scrolled = self.scroll.borrow().as_ref()?.clone();
        let adjustment = scrolled.vadjustment();
        self.photo_for_scroll_position(adjustment.value() + adjustment.page_size() * 0.5)
    }


    fn set_scroll_y(self: &Rc<Self>, scroll_y: f64) -> bool {
        self.scroll_animation_generation
            .set(self.scroll_animation_generation.get().wrapping_add(1));
        let Some(scrolled) = self.scroll.borrow().as_ref().cloned() else {
            return false;
        };
        self.refresh();
        let adjustment = scrolled.vadjustment();
        if !sectioned_scroll_extent_is_ready(
            self.total_height.get(),
            adjustment.page_size(),
            adjustment.upper(),
        ) {
            return false;
        }
        let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        adjustment.set_value(scroll_y.clamp(adjustment.lower(), upper));
        self.refresh();
        true
    }
    fn focus_photo(&self, photo_id: i64) {
        // Focusing can synchronously scroll GTK's viewport and refresh tiles.
        // Release the map borrow before calling into GTK.
        let tile = self
            .live_tiles
            .borrow()
            .values()
            .find(|entry| {
                entry
                    .tile
                    .photo()
                    .as_ref()
                    .is_some_and(|photo| photo.id() == photo_id)
            })
            .map(|entry| entry.tile.clone());
        if let Some(tile) = tile {
            tile.grab_focus();
        }
    }
}


impl Gallery {
    pub fn attach_sectioned_folder_scroll(self: &Rc<Self>, scrolled: &gtk::ScrolledWindow) {
        self.sectioned_folder.attach_scroll(scrolled);
    }

    pub fn cancel_sectioned_folder_scroll_animation(&self) {
        if self.using_virtual_photo_surface() {
            self.sectioned_folder.cancel_scroll_animation();
            if std::env::var_os("PICASA_TRACE").is_some() {
                eprintln!("PIC_SECTIONED_SCROLL_CANCEL source=lightbox_open");
            }
        }
    }

    pub fn refresh_sectioned_folder(self: &Rc<Self>) {
        if self.layout() == PhotoLayout::PhotoWall {
            self.sectioned_folder.refresh();
            return;
        }
        self.sectioned_folder.refresh_model();
    }

    pub fn using_sectioned_folder_view(&self) -> bool {
        self.using_virtual_photo_surface()
    }

    pub fn capture_sectioned_folder_anchors(&self) -> Vec<(i64, f64)> {
        self.sectioned_folder.capture_visible_anchors()
    }

    pub fn capture_sectioned_folder_header_anchor(&self) -> Option<(i64, f64)> {
        self.sectioned_folder.capture_visible_header_anchor()
    }

    pub fn restore_sectioned_folder_header_anchor(self: &Rc<Self>, anchor: (i64, f64)) -> bool {
        self.sectioned_folder.restore_header_anchor(anchor.0, anchor.1)
    }

    pub fn restore_sectioned_folder_anchor(
        self: &Rc<Self>,
        anchor: Option<(i64, f64)>,
    ) -> bool {
        self.sectioned_folder.invalidate_geometry();
        self.sectioned_folder.refresh();
        if let Some((photo_id, offset)) = anchor {
            return self.sectioned_folder.restore_anchor(photo_id, offset);
        }
        false
    }

    pub fn sectioned_folder_scroll_position(&self) -> f64 {
        self.sectioned_folder.scroll_position()
    }

    pub fn set_sectioned_folder_scroll_position(self: &Rc<Self>, y: f64) -> bool {
        self.sectioned_folder.set_scroll_y(y)
    }

    fn sectioned_sync_selection(&self) {
        self.sectioned_folder.sync_selection();
    }
}

#[cfg(test)]
mod section_lookup_tests {
    use super::*;

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn sectioned_focus_and_removal_release_tile_map_before_gtk_callbacks() {
        gtk::init().unwrap();
        let gallery = Rc::new(Gallery::new(
            &[],
            120,
            |_| {},
            |_, _, _| {},
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
        ));
        gallery.group_mode.set(GroupMode::Folder);
        gallery.current_photos.replace(
            (0..400_i64)
                .map(|id| {
                    glib::Object::builder::<PhotoObject>()
                        .property("id", id + 1)
                        .property("path", format!("/focus-regression/photo-{id}.jpg"))
                        .property("filename", format!("photo-{id}.jpg"))
                        .property("folder-id", 7_i64)
                        .property("folder-path", "/focus-regression")
                        .property("original-available", false)
                        .build()
                })
                .collect(),
        );
        gallery.rebuild_group_ranges();
        gallery.sectioned_folder.refresh_model();
        let scroll = gtk::ScrolledWindow::builder()
            .child(&gallery.folder_sectioned_root)
            .build();
        gallery.attach_sectioned_folder_scroll(&scroll);
        let window = gtk::Window::builder()
            .default_width(900)
            .default_height(650)
            .child(&scroll)
            .build();
        window.present();
        let context = glib::MainContext::default();
        let until = Instant::now() + std::time::Duration::from_millis(250);
        while Instant::now() < until {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let view = &gallery.sectioned_folder;
        let tile = view.live_tiles.borrow().get(&1).unwrap().tile.clone();
        let focus_borrow_available = Rc::new(Cell::new(None));
        let result = focus_borrow_available.clone();
        let weak = Rc::downgrade(view);
        let adjustment = scroll.vadjustment();
        let scroll_callback_ran = Rc::new(Cell::new(false));
        let callback_ran = scroll_callback_ran.clone();
        adjustment.connect_value_changed(move |_| callback_ran.set(true));
        tile.connect_has_focus_notify(move |tile| {
            if tile.has_focus() {
                if let Some(view) = weak.upgrade() {
                    let available = view.live_tiles.try_borrow_mut().is_ok();
                    result.set(Some(available));
                    // Re-enter refresh through GTK's synchronous scroll signal.
                    // Record a failed borrow without panicking across GTK FFI.
                    if available {
                        adjustment.set_value(300.0);
                    }
                }
            }
        });
        view.focus_photo(2);
        assert_eq!(focus_borrow_available.get(), Some(true));
        assert!(scroll_callback_ran.get());

        // Removing an on-screen tile emits unmap synchronously, just as focus
        // can synchronously scroll the viewport and invoke refresh again.
        let removal_borrow_available = Rc::new(Cell::new(None));
        let result = removal_borrow_available.clone();
        let weak = Rc::downgrade(view);
        tile.connect_unmap(move |_| {
            if let Some(view) = weak.upgrade() {
                result.set(Some(view.live_tiles.try_borrow_mut().is_ok()));
            }
        });
        gtk::prelude::GtkWindowExt::set_focus(&window, gtk::Widget::NONE);
        scroll.vadjustment().set_value(10_000.0);
        view.refresh();
        assert_eq!(removal_borrow_available.get(), Some(true));
        window.close();
    }

    #[test]
    fn deep_folder_jump_waits_for_the_new_scroll_extent() {
        assert!(!sectioned_scroll_extent_is_ready(387_058.0, 700.0, 1_400.0));
        assert!(sectioned_scroll_extent_is_ready(387_058.0, 700.0, 387_058.0));
        assert!(sectioned_scroll_extent_is_ready(500.0, 700.0, 500.0));
    }

    fn range(start: usize, end: usize) -> GroupRange {
        GroupRange {
            start,
            end,
            label: format!("folder-{start}"),
            folder_id: start as i64,
        }
    }

    #[test]
    fn binary_photo_section_lookup_matches_contiguous_ranges() {
        let ranges = [range(0, 3), range(3, 8), range(8, 9)];
        assert_eq!(section_index_for_photo(&ranges, 0), Some(0));
        assert_eq!(section_index_for_photo(&ranges, 2), Some(0));
        assert_eq!(section_index_for_photo(&ranges, 3), Some(1));
        assert_eq!(section_index_for_photo(&ranges, 8), Some(2));
        assert_eq!(section_index_for_photo(&ranges, 9), None);
    }

    #[test]
    fn binary_visible_section_span_keeps_exact_boundary_semantics() {
        let geometry = [
            SectionedFolderGeometry {
                header_y: 0.0,
                first_photo_y: 70.0,
                end_y: 370.0,
            },
            SectionedFolderGeometry {
                header_y: 370.0,
                first_photo_y: 440.0,
                end_y: 740.0,
            },
            SectionedFolderGeometry {
                header_y: 740.0,
                first_photo_y: 810.0,
                end_y: 1110.0,
            },
        ];
        assert_eq!(visible_section_span(&geometry, 370.0, 740.0), 0..3);
        assert_eq!(visible_section_span(&geometry, 371.0, 739.0), 1..2);
        assert_eq!(visible_section_span(&geometry, 1200.0, 1300.0), 3..3);
    }

    #[test]
    fn sectioned_reveal_rejects_a_stale_viewport_that_cannot_reach_the_row() {
        assert!(!viewport_intersects_row(20_000.0, 100.0, 0.0, 0.0));
        assert!(!viewport_intersects_row(20_000.0, 100.0, 0.0, 700.0));
        assert!(viewport_intersects_row(20_000.0, 100.0, 19_950.0, 700.0));
        assert!(viewport_intersects_row(20_000.0, 100.0, 20_099.0, 700.0));
        assert!(!viewport_intersects_row(20_000.0, 100.0, 20_100.0, 700.0));
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn sectioned_reveal_retries_after_the_folder_scroller_is_mapped() {
        fn settle(milliseconds: u64) {
            let context = glib::MainContext::default();
            let until = Instant::now() + std::time::Duration::from_millis(milliseconds);
            while Instant::now() < until {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        gtk::init().unwrap();
        let gallery = Rc::new(Gallery::new(
            &[],
            120,
            |_| {},
            |_, _, _| {},
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
        ));
        gallery.group_mode.set(GroupMode::Folder);
        gallery.current_photos.replace((0..4_000_i64).map(|id| {
            glib::Object::builder::<PhotoObject>()
                .property("id", id + 1)
                .property("path", format!("/reveal-test/parent/child/photo-{id}.jpg"))
                .property("filename", format!("photo-{id:04}.jpg"))
                .property("folder-id", 7_i64)
                .property("folder-path", "/reveal-test/parent/child")
                .property("original-available", true)
                .build()
        }).collect());
        gallery.rebuild_group_ranges();
        gallery.sectioned_folder.refresh_model();

        let scroll = gtk::ScrolledWindow::builder()
            .child(&gallery.folder_sectioned_root)
            .build();
        gallery.attach_sectioned_folder_scroll(&scroll);
        let window = gtk::Window::builder()
            .title("open-in-folder-allocation-regression")
            .default_width(900)
            .default_height(650)
            .child(&scroll)
            .build();

        // A navigation attempt made before the new Stack page is mapped must
        // remain retryable instead of treating a clamped adjustment as success.
        assert!(!gallery.sectioned_folder.scroll_to_index_centered_now(3_500));
        window.present();
        settle(200);
        assert!(gallery
            .sectioned_folder
            .scroll_to_index_centered_now(3_500));

        let section = gallery.sectioned_folder.section_index_for_photo(3_500).unwrap();
        let range = gallery.sectioned_folder.group_ranges.borrow()[section].clone();
        let geom = gallery.sectioned_folder.geometry.borrow()[section].clone();
        let row_index = (3_500 - range.start) as u32 / gallery.current_columns.get().max(1);
        let row_height = f64::from(folder_line_height(
            gallery.tile_height.get(),
            gallery.show_file_names.get(),
        ));
        let row_top = geom.first_photo_y + f64::from(row_index) * row_height;
        let adjustment = scroll.vadjustment();
        assert!(viewport_intersects_row(
            row_top,
            row_height,
            adjustment.value(),
            adjustment.page_size(),
        ));
        let actual_center = adjustment.value() + adjustment.page_size() * 0.5;
        let expected_center = (row_top + row_height * 0.5)
            .clamp(adjustment.lower() + adjustment.page_size() * 0.5,
                (adjustment.upper() - adjustment.page_size()).max(adjustment.lower())
                    + adjustment.page_size() * 0.5);
        assert!((actual_center - expected_center).abs() <= 1.0);
        window.close();
    }

    #[test]
    fn upper_edge_anchor_keeps_scroll_value_when_zoom_changes_rows() {
        let ranges = [range(0, 4), range(4, 10)];
        let geometry = [
            SectionedFolderGeometry {
                header_y: 0.0,
                first_photo_y: SECTIONED_HEADER_HEIGHT,
                end_y: 300.0,
            },
            SectionedFolderGeometry {
                header_y: 300.0,
                first_photo_y: 370.0,
                end_y: 670.0,
            },
        ];
        let (photo_index, offset) = upper_edge_anchor(&ranges, &geometry, 0.0, 0.0, 280.0).unwrap();
        assert_eq!(photo_index, 0);
        let zoomed_first_row_y = geometry[0].first_photo_y;
        assert_eq!(zoomed_first_row_y - offset, 0.0);
        assert!(upper_edge_anchor(&ranges, &geometry, 281.0, 0.0, 280.0).is_none());
    }

    #[test]
    fn strip_wrap_preserves_every_pixel_and_model_order_in_both_directions() {
        // The clip partition must conserve a whole photo at every intermediate
        // capacity. A photo may occupy only two adjacent rows at a wrap.
        for step in 0..=100 {
            for columns in [3.0 - step as f64 / 100.0, 2.0 + step as f64 / 100.0] {
                let layout = StripLayout {
                    columns, tile_width: 90.0, tile_height: 60.0, pitch: 100.0, left: 0.0,
                    sections: vec![StripSection {
                        first_index: 0.0, first_y: 70.0, row_height: 80.0,
                        header_y: 0.0, end_y: 2000.0,
                    }],
                };
                let slices = strip_slices(&layout, &[range(0, 9)], 0.0, 2000.0);
                for index in 0..9 {
                    let pieces = slices.iter().filter(|p| p.index == index).collect::<Vec<_>>();
                    assert!(!pieces.is_empty() && pieces.len() <= 2);
                    assert!((pieces.iter().map(|p| p.clip_width).sum::<f64>() - 90.0).abs() < 1e-7);
                    if pieces.len() == 2 { assert_eq!(pieces[1].row, pieces[0].row + 1); }
                }
                for row in 0..6 {
                    let pieces = slices.iter().filter(|p| p.row == row).collect::<Vec<_>>();
                    for pair in pieces.windows(2) {
                        assert!(pair[0].index < pair[1].index);
                        assert!(pair[0].clip_x + pair[0].clip_width <= pair[1].clip_x + 1e-7);
                        assert_eq!(pair[0].y, pair[1].y);
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn strip_zoom_resize_and_retarget_keep_photos_and_headers_together() {
        fn settle(milliseconds: u64) {
            let context = glib::MainContext::default();
            let until = Instant::now() + std::time::Duration::from_millis(milliseconds);
            while Instant::now() < until {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        gtk::init().unwrap();
        let gallery = Rc::new(Gallery::new(&[], 120, |_| {}, |_, _, _| {}, |_, _, _, _| {}, |_, _| {}, |_| {}));
        gallery.group_mode.set(GroupMode::Folder);
        gallery.current_columns.set(5);
        let sample_root = std::env::current_dir().unwrap().join("screenshots");
        gallery.current_photos.replace((1..=100_i64).map(|id| {
            let path = sample_root.join(if id % 2 == 0 { "all photos.jpg" } else { "portrait photo screen.jpg" });
            let folder_id = if id <= 12 { 1_i64 } else { 2_i64 };
            let folder_path = if folder_id == 1 { "/zoom-test/first" } else { "/zoom-test/second" };
            glib::Object::builder::<PhotoObject>()
                .property("id", id)
                .property("path", path.to_string_lossy().to_string())
                .property("cached-thumbnail-path", path.to_string_lossy().to_string())
                .property("thumbnail-available", true)
                .property("filename", format!("photo-{id:03}.jpg"))
                .property("folder-id", folder_id)
                .property("folder-path", folder_path)
                .property("original-available", true)
                .build()
        }).collect());
        let mut cached = HashSet::new();
        for photo in gallery.current_photos.borrow().iter() {
            if let Some(key) = photo_presentation_key(photo) {
                if cached.insert(key.clone()) {
                    let file = gtk::gio::File::for_path(photo.path());
                    let texture = gtk::gdk::Texture::from_file(&file).unwrap();
                    folder_thumbnail_cache_insert(key, texture.upcast());
                }
            }
        }
        gallery.rebuild_group_ranges();

        let scroll = gtk::ScrolledWindow::builder()
            .child(&gallery.folder_sectioned_root)
            .build();
        gallery.attach_sectioned_folder_scroll(&scroll);
        let window = gtk::Window::builder()
            .title("rc6-grid-reflow-test")
            .default_width(800)
            .default_height(500)
            .child(&scroll)
            .build();
        window.present();
        if std::env::var_os("PICASA_VISUAL_TEST").is_some() {
            settle(2500);
        }
        settle(150);
        scroll.vadjustment().set_value(0.0);
        settle(100);
        gallery.sectioned_folder.refresh();
        let before = gallery.sectioned_folder.live_tiles.borrow()
            .keys().copied().collect::<HashSet<_>>();
        assert!(!before.is_empty());

        let view = &gallery.sectioned_folder;
        let snapshot = view.capture_reflow_snapshot();
        let anchor = view.capture_center_anchor();
        gallery.current_columns.set(4);
        gallery.tile_width.set(137);
        gallery.tile_height.set(91);
        view.animate_zoom_reflow(snapshot, anchor);
        let mut frames = 0;
        let mut saw_split = false;
        let until = Instant::now() + std::time::Duration::from_secs(2);
        while view.reflow_active.get() && Instant::now() < until {
            if let Some(presentation) = view.strip_presentation.borrow().as_ref() {
                let slices = strip_slices(&presentation.layout, &view.group_ranges.borrow(), 0.0, 2000.0);
                for slice in &slices {
                    // Only viewport/overscan photos are realized.
                    if slice.y < 800.0 {
                        assert!(presentation.photos.contains_key(&slice.index), "missing photo {}", slice.index);
                    }
                }
                let mut seen = HashSet::new();
                saw_split |= slices.iter().any(|slice| !seen.insert(slice.index));
                for (section, header) in view.live_headers.borrow().iter() {
                    let (x, y) = view.root.child_position(header);
                    // Headings track the centered tile column, never the margin.
                    assert_eq!(x, presentation.layout.left);
                    assert!((y - presentation.layout.sections[*section].header_y).abs() < 0.01);
                }
                assert!(before.is_subset(&view.live_tiles.borrow().keys().copied().collect()));
                frames += 1;
            }
            settle(12);
        }
        assert!(!view.reflow_active.get());
        assert!(frames > 3 && saw_split, "the recorded transition never split a wrapping photo");
        assert!(view.strip_layer.borrow().is_none());
        assert!(view.live_tiles.borrow().values().all(|entry| entry.tile.opacity() == 1.0));

        // Reverse, then change direction again while the first animation is
        // still visible. The new animation must start at that exact strip state.
        let snapshot = view.capture_reflow_snapshot();
        let anchor = view.capture_center_anchor();
        gallery.current_columns.set(5);
        gallery.tile_width.set(120);
        gallery.tile_height.set(80);
        view.animate_reflow(snapshot, anchor);
        settle(120);
        let old = view.strip_presentation.borrow().as_ref().unwrap().layout.clone();
        let old_scroll = view.scroll_position();
        let snapshot = view.capture_reflow_snapshot();
        let anchor = view.capture_center_anchor();
        gallery.current_columns.set(4);
        gallery.tile_width.set(137);
        gallery.tile_height.set(91);
        view.animate_zoom_reflow(snapshot, anchor);
        let current = view.strip_presentation.borrow().as_ref().unwrap().layout.clone();
        assert_eq!(old.columns, current.columns);
        assert_eq!(old.pitch, current.pitch);
        assert_eq!(old.left, current.left);
        let scroll_delta = view.scroll_position() - old_scroll;
        for (a, b) in old.sections.iter().zip(&current.sections) {
            assert_eq!(a.first_index, b.first_index);
            assert!((a.first_y + scroll_delta - b.first_y).abs() < 0.001);
        }
        settle(650);
        assert!(!view.reflow_active.get());
        assert!(view.strip_layer.borrow().is_none());
        assert!(view.live_tiles.borrow().values().all(|entry| entry.tile.opacity() == 1.0));
        // A scrolled section must start at the same screen coordinates, even
        // when its first realized photo is far from the section's first row.
        scroll.vadjustment().set_value(1200.0);
        settle(80);
        let old_scroll = view.scroll_position();
        let old_positions = view.live_tiles.borrow().iter().map(|(index, entry)| {
            (*index, view.root.child_position(&entry.tile))
        }).collect::<HashMap<_, _>>();
        let snapshot = view.capture_reflow_snapshot();
        let anchor = view.capture_center_anchor();
        gallery.current_columns.set(5);
        gallery.tile_width.set(120);
        gallery.tile_height.set(80);
        view.animate_reflow(snapshot, anchor);
        let presentation = view.strip_presentation.borrow().as_ref().unwrap().clone();
        let slices = strip_slices(&presentation.layout, &view.group_ranges.borrow(), 0.0, 10000.0);
        let delta = view.scroll_position() - old_scroll;
        let mut matched = 0;
        for slice in &slices {
            if let Some((x, y)) = old_positions.get(&slice.index) {
                assert!((slice.x - x).abs() < 1.0);
                assert!((slice.y - y - delta).abs() < 1.0);
                matched += 1;
            }
        }
        assert!(matched > 5);
        view.refresh_model();
        assert!(view.strip_layer.borrow().is_none());
        assert!(view.live_tiles.borrow().values().all(|entry| entry.tile.opacity() == 1.0));

        // A tiny folder must not gain a scrollbar from the temporary layer.
        gallery.current_photos.borrow_mut().truncate(3);
        gallery.rebuild_group_ranges();
        view.refresh_model();
        settle(80);
        let snapshot = view.capture_reflow_snapshot();
        gallery.current_columns.set(3);
        view.animate_zoom_reflow(snapshot, None);
        settle(80);
        assert!(scroll.vadjustment().upper() <= scroll.vadjustment().page_size() + 1.0);
        view.refresh_model();
        assert!(view.strip_layer.borrow().is_none());
        window.close();
    }

    /// Issue #104: on a fullscreen-sized surface the tile grid centers itself
    /// (the smallest zoom levels clamp the column count and leave real spare
    /// width). Headings used to stay pinned to `SECTIONED_SIDE_MARGIN`, so they
    /// slid away from their own first row as the zoom level changed.
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn section_headings_sit_above_the_first_tile_column() {
        fn settle(milliseconds: u64) {
            let context = glib::MainContext::default();
            let until = Instant::now() + std::time::Duration::from_millis(milliseconds);
            while Instant::now() < until {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        gtk::init().unwrap();
        let gallery = Rc::new(Gallery::new(&[], 120, |_| {}, |_, _, _| {}, |_, _, _, _| {}, |_, _| {}, |_| {}));
        gallery.group_mode.set(GroupMode::Folder);
        let sample_root = std::env::current_dir().unwrap().join("screenshots");
        gallery.current_photos.replace((1..=96_i64).map(|id| {
            let path = sample_root.join(if id % 2 == 0 { "all photos.jpg" } else { "portrait photo screen.jpg" });
            let folder_id = if id <= 48 { 1_i64 } else { 2_i64 };
            glib::Object::builder::<PhotoObject>()
                .property("id", id)
                .property("path", path.to_string_lossy().to_string())
                .property("cached-thumbnail-path", path.to_string_lossy().to_string())
                .property("thumbnail-available", true)
                .property("filename", format!("photo-{id:03}.jpg"))
                .property("folder-id", folder_id)
                .property("folder-path", format!("/zoom-align-test/folder-{folder_id}"))
                .property("original-available", true)
                .build()
        }).collect());
        gallery.rebuild_group_ranges();

        let scroll = gtk::ScrolledWindow::builder()
            .child(&gallery.folder_sectioned_root)
            .build();
        gallery.attach_sectioned_folder_scroll(&scroll);
        let window = gtk::Window::builder()
            .title("issue-104-heading-alignment-test")
            .default_width(2400)
            .default_height(900)
            .child(&scroll)
            .build();
        window.present();
        settle(200);
        scroll.vadjustment().set_value(0.0);

        let view = &gallery.sectioned_folder;
        let mut exercised_centered_grid = false;
        for (columns, tile) in [(12_u32, 100_i32), (8, 219), (6, 300)] {
            gallery.current_columns.set(columns);
            gallery.tile_width.set(tile);
            gallery.tile_height.set(tile * 2 / 3);

            // gtk::Fixed only reports a queued move once the next layout pass
            // has run, so remap the window and read until the positions stop
            // changing instead of trusting the first value.
            let mut last = (f64::NAN, f64::NAN);
            for _ in 0..8 {
                view.refresh();
                window.hide();
                window.present();
                settle(150);
                let tiles_x = view
                    .live_tiles
                    .borrow()
                    .values()
                    .map(|entry| view.root.child_position(&entry.tile).0)
                    .fold(f64::INFINITY, f64::min);
                let heading_x = view
                    .live_headers
                    .borrow()
                    .values()
                    .map(|header| view.root.child_position(header).0)
                    .fold(f64::INFINITY, f64::min);
                let current = (tiles_x, heading_x);
                if current == last {
                    break;
                }
                last = current;
            }
            let (first_column_x, heading_x) = last;

            assert!(first_column_x.is_finite(), "columns={columns}: no realized tiles");
            assert!(heading_x.is_finite(), "columns={columns}: no realized heading");
            // The grid has to be centered (off the side margin) for this case
            // to say anything about the fullscreen drift from issue #104.
            exercised_centered_grid |=
                first_column_x > SECTIONED_SIDE_MARGIN + f64::from(FOLDER_ITEM_MARGIN);
            assert_eq!(
                heading_x, first_column_x,
                "columns={columns} tile={tile}: heading x={heading_x}, first column x={first_column_x}"
            );
        }
        assert!(
            exercised_centered_grid,
            "every case landed on the side margin, so this run never exercised the centered grid"
        );
        window.close();
    }

}
