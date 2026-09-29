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
    tile_rects: HashMap<u32, (f64, f64, f64, f64)>,
    old_columns: u32,
    old_geometry: Vec<SectionedFolderGeometry>,
    tile_width: i32,
    tile_height: i32,
    old_scroll_y: f64,
}

#[derive(Clone)]
struct SectionedFolderTile {
    tile: SquareTile,
    index: Rc<Cell<Option<u32>>>,
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

struct SectionedFolderView {
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
    activate: Rc<dyn Fn(Vec<PhotoObject>, usize, Option<(gtk::Widget, gtk::gdk::Paintable)>)>,
    context_menu: Rc<dyn Fn(PhotoObject, gtk::Widget, f64, f64)>,
    unavailable: Rc<dyn Fn(PhotoObject, gtk::Widget)>,
    collage_mode: Rc<Cell<bool>>,
    collage_ids: Rc<RefCell<HashSet<i64>>>,
    rubberband: gtk::DrawingArea,
    scroll: RefCell<Option<gtk::ScrolledWindow>>,
    geometry: RefCell<Vec<SectionedFolderGeometry>>,
    geometry_width: Cell<i32>,
    geometry_columns: Cell<u32>,
    geometry_row_height: Cell<i32>,
    total_height: Cell<f64>,
    live_tiles: RefCell<HashMap<u32, SectionedFolderTile>>,
    tile_pool: RefCell<VecDeque<SectionedFolderTile>>,
    live_headers: RefCell<HashMap<usize, gtk::Label>>,
    header_pool: RefCell<VecDeque<gtk::Label>>,
    selection_anchor: Rc<Cell<Option<u32>>>,
    keyboard_preferred_column: Rc<Cell<Option<u32>>>,
    scroll_animation_generation: Cell<u64>,
    tween: Rc<InPlaceTween>,
    resize_settle_generation: Cell<u64>,
    resize_settle_origin: RefCell<Option<SectionedReflowSnapshot>>,
    resize_settle_anchor: RefCell<Option<(i64, f64)>>,
}

impl SectionedFolderView {
    fn new(
        current_photos: Rc<RefCell<Vec<PhotoObject>>>,
        group_ranges: Rc<RefCell<Vec<GroupRange>>>,
        selection: gtk::MultiSelection,
        current_columns: Rc<Cell<u32>>,
        tile_width: Rc<Cell<i32>>,
        tile_height: Rc<Cell<i32>>,
        fit_whole_photo: Rc<Cell<bool>>,
        show_file_names: Rc<Cell<bool>>,
        activate: Rc<dyn Fn(Vec<PhotoObject>, usize, Option<(gtk::Widget, gtk::gdk::Paintable)>)>,
        context_menu: Rc<dyn Fn(PhotoObject, gtk::Widget, f64, f64)>,
        unavailable: Rc<dyn Fn(PhotoObject, gtk::Widget)>,
        collage_mode: Rc<Cell<bool>>,
        collage_ids: Rc<RefCell<HashSet<i64>>>,
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
            activate,
            context_menu,
            unavailable,
            collage_mode,
            collage_ids,
            rubberband,
            scroll: RefCell::new(None),
            geometry: RefCell::new(Vec::new()),
            geometry_width: Cell::new(0),
            geometry_columns: Cell::new(0),
            geometry_row_height: Cell::new(0),
            total_height: Cell::new(1.0),
            live_tiles: RefCell::new(HashMap::new()),
            tile_pool: RefCell::new(VecDeque::new()),
            live_headers: RefCell::new(HashMap::new()),
            header_pool: RefCell::new(VecDeque::new()),
            selection_anchor: Rc::new(Cell::new(None)),
            keyboard_preferred_column: Rc::new(Cell::new(None)),
            scroll_animation_generation: Cell::new(0),
            tween: Rc::new(InPlaceTween::default()),
            resize_settle_generation: Cell::new(0),
            resize_settle_origin: RefCell::new(None),
            resize_settle_anchor: RefCell::new(None),
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
                    vertical_navigation_target(
                        &view.group_ranges.borrow(),
                        current,
                        columns,
                        preferred,
                        direction,
                    )
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

    fn attach_scroll(self: &Rc<Self>, scrolled: &gtk::ScrolledWindow) {
        self.scroll.replace(Some(scrolled.clone()));

        let this = self.clone();
        scrolled
            .vadjustment()
            .connect_value_changed(move |_| this.refresh());

        let this = self.clone();
        let scrolled_for_tick = scrolled.clone();
        let last_width = Rc::new(Cell::new(0_i32));
        let last_width_for_tick = last_width.clone();
        scrolled.add_tick_callback(move |_, _| {
            let width = scrolled_for_tick.width();
            if width > 0 && width != last_width_for_tick.get() {
                last_width_for_tick.set(width);
                // Width-only motion with the same column count does not change
                // vertical section geometry. Refresh only to stretch headers;
                // update_layout invalidates geometry when columns actually change.
                this.refresh();
            }
            glib::ControlFlow::Continue
        });

        self.refresh();
    }

    fn invalidate_geometry(&self) {
        self.geometry_width.set(0);
        self.geometry_columns.set(0);
        self.geometry_row_height.set(0);
    }

    fn geometry_for_current_layout(&self, width: i32) {
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
        tile.set_filename_visible(self.show_file_names.get());
        tile.set_content_fit(if self.fit_whole_photo.get() {
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

        // Section geometry is ordered by Y. Jump to the first section that
        // overlaps the overscan band and stop after its final section instead
        // of copying and scanning the complete folder catalog on every scroll.
        let section_span = visible_section_span(&geometry, top, bottom);
        for section_index in section_span {
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

        for index in stale {
            if let Some(tile) = self.live_tiles.borrow_mut().remove(&index) {
                self.root.remove(&tile.tile);
                tile.tile.set_opacity(1.0);
                tile.tile.reset_presentation_transform();
                tile.index.set(None);
                let mut pool = self.tile_pool.borrow_mut();
                if pool.len() < SECTIONED_TILE_POOL_CAP {
                    pool.push_back(tile);
                }
            }
        }

        let photos = self.current_photos.borrow();
        for (index, section_index, row, col) in wanted_tiles {
            let existing = {
                let live = self.live_tiles.borrow();
                live.get(&index).cloned()
            };
            let tile = if let Some(tile) = existing {
                tile
            } else {
                let tile = self
                    .tile_pool
                    .borrow_mut()
                    .pop_front()
                    .unwrap_or_else(|| self.make_tile());
                let Some(photo) = photos.get(index as usize) else {
                    continue;
                };
                tile.tile.reset_presentation_transform();
                tile.index.set(Some(index));
                tile.tile.set_opacity(1.0);
                tile.tile
                    .set_tile_size(self.tile_width.get(), self.tile_height.get());
                tile.tile.set_filename_visible(self.show_file_names.get());
                tile.tile.set_content_fit(if self.fit_whole_photo.get() {
                    gtk::ContentFit::Contain
                } else {
                    gtk::ContentFit::Cover
                });
                tile.tile.bind_photo_folder_fast(photo, index as usize);
                tile.tile
                    .set_manual_selected(self.selection.is_selected(index));
                self.root.put(&tile.tile, 0.0, 0.0);
                self.live_tiles.borrow_mut().insert(index, tile.clone());
                tile
            };

            tile.tile
                .set_manual_selected(self.selection.is_selected(index));

            tile.tile
                .set_tile_size(self.tile_width.get(), self.tile_height.get());
            let (start_x, gap) = self.horizontal_grid_metrics(self.geometry_width.get());
            let x = start_x + f64::from(col) * (f64::from(self.tile_width.get()) + gap);
            let y = geometry[section_index].first_photo_y + f64::from(row) * row_height;
            self.root.move_(&tile.tile, x, y);
        }
        drop(photos);

        let wanted_header_ids = wanted_headers.iter().copied().collect::<HashSet<_>>();

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
        }

        // Headings belong to the first tile column, not to the fixed side
        // margin. The tile grid centers itself whenever spare width remains
        // (the smallest zoom levels clamp the column count, so a lot of width
        // is left over in fullscreen), and a margin-pinned title slid away
        // from its own row as the window or zoom level changed (issue #104).
        let (heading_x, _) = self.horizontal_grid_metrics(width);
        let heading_width =
            (width - heading_x.ceil() as i32 - FOLDER_ITEM_MARGIN - SECTIONED_SIDE_MARGIN as i32)
                .max(1);
        for section_index in wanted_headers {
            let existing = {
                let live = self.live_headers.borrow();
                live.get(&section_index).cloned()
            };
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
            label.set_size_request(heading_width, SECTIONED_HEADER_HEIGHT as i32);

            self.root
                .move_(&label, heading_x, geometry[section_index].header_y);
        }
        drop(geometry);
        drop(ranges);
    }

    fn sync_selection(&self) {
        for (index, tile) in self.live_tiles.borrow().iter() {
            tile.tile
                .set_manual_selected(self.selection.is_selected(*index));
        }
    }

    fn refresh_model(self: &Rc<Self>) {
        self.tween.cancel();
        self.resize_settle_generation
            .set(self.resize_settle_generation.get().wrapping_add(1));
        self.resize_settle_origin.borrow_mut().take();
        self.resize_settle_anchor.borrow_mut().take();
        // A replacement can keep the same numeric positions while changing
        // PhotoObject metadata. Recycle the bounded realized set so every
        // visible tile is rebound exactly once to the current model.
        let live = std::mem::take(&mut *self.live_tiles.borrow_mut());
        for (_, tile) in live {
            self.root.remove(&tile.tile);
            tile.tile.set_opacity(1.0);
            tile.tile.reset_presentation_transform();
            tile.index.set(None);
            let mut pool = self.tile_pool.borrow_mut();
            if pool.len() < SECTIONED_TILE_POOL_CAP {
                pool.push_back(tile);
            }
        }
        let headers = std::mem::take(&mut *self.live_headers.borrow_mut());
        for (_, label) in headers {
            self.root.remove(&label);
            let mut pool = self.header_pool.borrow_mut();
            if pool.len() < SECTIONED_HEADER_POOL_CAP {
                pool.push_back(label);
            }
        }
        self.invalidate_geometry();
        self.refresh();
    }

    fn capture_reflow_snapshot(&self) -> SectionedReflowSnapshot {
        SectionedReflowSnapshot {
            tile_rects: self
                .live_tiles
                .borrow()
                .iter()
                .filter_map(|(index, entry)| {
                    let (x, y) = self.root.child_position(&entry.tile);
                    let width = f64::from(entry.tile.width()).max(1.0);
                    let height = f64::from(entry.tile.height()).max(1.0);
                    let top = self.scroll_position();
                    let bottom = top
                        + self
                            .scroll
                            .borrow()
                            .as_ref()
                            .map(|scroll| scroll.vadjustment().page_size())
                            .unwrap_or(f64::MAX);
                    (y + height >= top && y <= bottom).then_some((*index, (x, y, width, height)))
                })
                .collect(),
            old_columns: self.current_columns.get().max(1),
            old_geometry: self.geometry.borrow().clone(),
            tile_width: self.tile_width.get(),
            tile_height: self.tile_height.get(),
            old_scroll_y: self.scroll_position(),
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
        let raw_gap = (available - tile_width * f64::from(columns)) / f64::from(columns - 1);
        let gap = raw_gap.clamp(18.0, 54.0);
        let used = tile_width * f64::from(columns) + gap * f64::from(columns - 1);
        let start_x = ((width - used) * 0.5).max(SECTIONED_SIDE_MARGIN);
        (start_x, gap)
    }

    fn section_index_for_photo(&self, index: u32) -> Option<usize> {
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
        self.tween.cancel();

        let header_anchor = self.scroll.borrow().as_ref().and_then(|scroll| {
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
                let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
                let target = (new_header_y - viewport_y).clamp(adjustment.lower(), upper);
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

    fn settle_resize(
        self: &Rc<Self>,
        snapshot: SectionedReflowSnapshot,
        anchor: Option<(i64, f64)>,
    ) {
        // Stop any zoom effect as soon as a window resize starts. Resize stays
        // visually steady during the drag and receives its own settle effect.
        for entry in self.live_tiles.borrow().values() {
            entry.tile.reset_presentation_transform();
        }
        // Keep the first visible title/viewport anchor for the full resize
        // burst. Recomputing it from each intermediate layout makes headings
        // drift past sections when several column counts change quickly.
        let (origin, stable_anchor) = {
            let mut origin = self.resize_settle_origin.borrow_mut();
            if origin.is_none() {
                *origin = Some(snapshot.clone());
                *self.resize_settle_anchor.borrow_mut() = anchor;
            }
            (
                origin.as_ref().expect("resize origin set above").clone(),
                *self.resize_settle_anchor.borrow(),
            )
        };
        self.apply_reflow_without_animation(origin.clone(), stable_anchor);
        if !crate::animation_settings::folder_resize_enabled()
            || !self.root.is_mapped()
            || !self.root.settings().is_gtk_enable_animations()
        {
            self.resize_settle_origin.borrow_mut().take();
            self.resize_settle_anchor.borrow_mut().take();
            self.resize_settle_generation
                .set(self.resize_settle_generation.get().wrapping_add(1));
            return;
        }
        self.schedule_resize_settle();
    }

    fn on_resize_frame(self: &Rc<Self>) {
        self.tween.cancel();
        for entry in self.live_tiles.borrow().values() {
            entry.tile.reset_presentation_transform();
        }
        if self.resize_settle_origin.borrow().is_some() {
            self.schedule_resize_settle();
        } else {
            self.resize_settle_generation
                .set(self.resize_settle_generation.get().wrapping_add(1));
        }
    }

    fn schedule_resize_settle(self: &Rc<Self>) {
        self.resize_settle_generation
            .set(self.resize_settle_generation.get().wrapping_add(1));
        let generation = self.resize_settle_generation.get();
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_millis(110), move || {
            let Some(view) = weak.upgrade() else { return };
            if view.resize_settle_generation.get() != generation {
                return;
            }
            let origin = view.resize_settle_origin.borrow_mut().take();
            view.resize_settle_anchor.borrow_mut().take();
            if let Some(origin) = origin {
                view.animate_tile_in_place(origin, crate::animation_settings::folder_resize_ms());
            }
        });
    }

    fn animate_zoom_reflow(
        self: &Rc<Self>,
        snapshot: SectionedReflowSnapshot,
        anchor: Option<(i64, f64)>,
    ) {
        self.resize_settle_generation
            .set(self.resize_settle_generation.get().wrapping_add(1));
        self.resize_settle_origin.borrow_mut().take();
        self.resize_settle_anchor.borrow_mut().take();
        self.apply_reflow_without_animation(snapshot.clone(), anchor);
        if crate::animation_settings::folder_zoom_style() == "in_place" {
            self.animate_tile_in_place(snapshot, crate::animation_settings::folder_zoom_ms());
        }
    }

    fn animate_tile_in_place(self: &Rc<Self>, snapshot: SectionedReflowSnapshot, duration_ms: f64) {
        let weak = Rc::downgrade(self);
        self.tween
            .animate_tile_in_place(&self.root, duration_ms, move || {
                let Some(view) = weak.upgrade() else {
                    return Vec::new();
                };
                let scroll_shift = view.scroll_position() - snapshot.old_scroll_y;
                let tiles = view
                    .live_tiles
                    .borrow()
                    .iter()
                    .filter_map(|(index, entry)| {
                        let &(x, y, width, height) = snapshot.tile_rects.get(index)?;
                        let (new_x, new_y) = view.root.child_position(&entry.tile);
                        let changed = (x - new_x).abs() >= 0.5
                            || (y + scroll_shift - new_y).abs() >= 0.5
                            || (width - f64::from(entry.tile.width())).abs() >= 0.5
                            || (height - f64::from(entry.tile.height())).abs() >= 0.5;
                        changed.then(|| entry.tile.clone())
                    })
                    .collect();
                tiles
            });
    }

    fn capture_center_anchor(&self) -> Option<(i64, f64)> {
        let scrolled = self.scroll.borrow().as_ref()?.clone();
        let adjustment = scrolled.vadjustment();
        let scroll_y = adjustment.value();
        let lower = adjustment.lower();
        let row_height = f64::from(folder_line_height(
            self.tile_height.get(),
            self.show_file_names.get(),
        ));
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
        }
        None
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
        let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        adjustment.set_value((y - offset).clamp(adjustment.lower(), upper));
        self.refresh();
        true
    }

    fn y_for_index(&self, index: u32) -> Option<f64> {
        let columns = self.current_columns.get().max(1);
        let row_height = f64::from(folder_line_height(
            self.tile_height.get(),
            self.show_file_names.get(),
        ));
        let section_index = self.section_index_for_photo(index)?;
        let ranges = self.group_ranges.borrow();
        let geometry = self.geometry.borrow();
        let range = ranges.get(section_index)?;
        let geom = geometry.get(section_index)?;
        let local = index as usize - range.start;
        Some(geom.first_photo_y + (local as u32 / columns) as f64 * row_height)
    }

    fn column_for_index(&self, index: u32) -> Option<u32> {
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
        let duration_ms = crate::animation_settings::folder_scroll_ms();

        self.root.add_tick_callback(move |_, _| {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if view.scroll_animation_generation.get() != generation {
                return glib::ControlFlow::Break;
            }

            let t = (started.elapsed().as_secs_f64() * 1000.0 / duration_ms).clamp(0.0, 1.0);
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
                    + f64::from(row)
                        * f64::from(folder_line_height(
                            self.tile_height.get(),
                            self.show_file_names.get(),
                        ))
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
        let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        adjustment.set_value(target.clamp(adjustment.lower(), upper));
        self.refresh();
        true
    }

    fn scroll_to_index_smooth(self: &Rc<Self>, index: u32, header: bool) -> bool {
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
                    + f64::from(row)
                        * f64::from(folder_line_height(
                            self.tile_height.get(),
                            self.show_file_names.get(),
                        ))
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
                index, target
            );
        }
        true
    }

    fn scroll_to_index_centered(self: &Rc<Self>, index: u32) -> bool {
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
                index, target
            );
        }
        true
    }

    fn scroll_position(&self) -> f64 {
        self.scroll
            .borrow()
            .as_ref()
            .map(|scroll| scroll.vadjustment().value())
            .unwrap_or(0.0)
    }

    fn photo_for_scroll_position(&self, scroll_y: f64) -> Option<PhotoObject> {
        let columns = self.current_columns.get().max(1);
        let row_height = f64::from(folder_line_height(
            self.tile_height.get(),
            self.show_file_names.get(),
        ));
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

    fn set_scroll_y(self: &Rc<Self>, scroll_y: f64) {
        self.scroll_animation_generation
            .set(self.scroll_animation_generation.get().wrapping_add(1));
        let Some(scrolled) = self.scroll.borrow().as_ref().cloned() else {
            return;
        };
        self.refresh();
        let adjustment = scrolled.vadjustment();
        let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        adjustment.set_value(scroll_y.clamp(adjustment.lower(), upper));
        self.refresh();
    }
    fn focus_photo(&self, photo_id: i64) {
        if let Some(tile) = self.live_tiles.borrow().values().find(|entry| {
            entry
                .tile
                .photo()
                .as_ref()
                .is_some_and(|photo| photo.id() == photo_id)
        }) {
            tile.tile.grab_focus();
        }
    }
}

impl Gallery {
    pub fn attach_sectioned_folder_scroll(self: &Rc<Self>, scrolled: &gtk::ScrolledWindow) {
        self.sectioned_folder.attach_scroll(scrolled);
    }

    pub fn cancel_sectioned_folder_scroll_animation(&self) {
        if self.using_sectioned_folder_view() {
            self.sectioned_folder.cancel_scroll_animation();
            if std::env::var_os("PICASA_TRACE").is_some() {
                eprintln!("PIC_SECTIONED_SCROLL_CANCEL source=lightbox_open");
            }
        }
    }

    pub fn refresh_sectioned_folder(self: &Rc<Self>) {
        self.sectioned_folder.refresh_model();
    }

    pub fn using_sectioned_folder_view(&self) -> bool {
        self.group_mode.get() == GroupMode::Folder && crate::grid::sectioned_folder_view_enabled()
    }

    fn sectioned_capture_anchor(&self) -> Option<(i64, f64)> {
        self.sectioned_folder.capture_center_anchor()
    }

    fn sectioned_restore_anchor(self: &Rc<Self>, anchor: Option<(i64, f64)>) {
        self.sectioned_folder.invalidate_geometry();
        self.sectioned_folder.refresh();
        if let Some((photo_id, offset)) = anchor {
            self.sectioned_folder.restore_anchor(photo_id, offset);
        }
    }

    fn sectioned_sync_selection(&self) {
        self.sectioned_folder.sync_selection();
    }
}

#[cfg(test)]
mod section_lookup_tests {
    use super::*;

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
        let sample_root = std::env::current_dir().unwrap().join("samples");
        gallery.current_photos.replace(
            (1..=96_i64)
                .map(|id| {
                    let path = sample_root.join(format!("ZoomOUT-{}.jpg", id % 30 + 1));
                    let folder_id = if id <= 48 { 1_i64 } else { 2_i64 };
                    glib::Object::builder::<PhotoObject>()
                        .property("id", id)
                        .property("path", path.to_string_lossy().to_string())
                        .property("cached-thumbnail-path", path.to_string_lossy().to_string())
                        .property("thumbnail-available", true)
                        .property("filename", format!("photo-{id:03}.jpg"))
                        .property("folder-id", folder_id)
                        .property(
                            "folder-path",
                            format!("/zoom-align-test/folder-{folder_id}"),
                        )
                        .property("original-available", true)
                        .build()
                })
                .collect(),
        );
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

            assert!(
                first_column_x.is_finite(),
                "columns={columns}: no realized tiles"
            );
            assert!(
                heading_x.is_finite(),
                "columns={columns}: no realized heading"
            );
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
