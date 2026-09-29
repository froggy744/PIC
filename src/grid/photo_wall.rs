#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PhotoLayout {
    #[default]
    Grid,
    PhotoWall,
}

#[derive(Default)]
struct PhotoWallState {
    generation: u64,
    layout: photo_wall_layout::PhotoWallLayout,
}

impl SectionedFolderView {
    fn is_wall(&self) -> bool {
        self.layout_mode.get() == PhotoLayout::PhotoWall
    }

    fn calculate_wall_geometry(&self, width: i32) {
        let target = self.tile_width.get();
        let caption = filename_caption_height(self.show_file_names.get());
        if self.geometry_width.get() == width && self.geometry_row_height.get() == target + caption
        {
            return;
        }
        let photos = self.current_photos.borrow();
        let ratios = photos
            .iter()
            .map(PhotoObject::photo_wall_aspect_ratio)
            .collect::<Vec<_>>();
        let sections = if self.group_mode.get() == GroupMode::Folder {
            self.group_ranges
                .borrow()
                .iter()
                .map(|range| photo_wall_layout::PhotoWallSection {
                    photo_range: range.start..range.end,
                    header_height: SECTIONED_HEADER_HEIGHT,
                })
                .collect::<Vec<_>>()
        } else {
            vec![photo_wall_layout::PhotoWallSection {
                photo_range: 0..photos.len(),
                header_height: 0.0,
            }]
        };
        let layout = photo_wall_layout::PhotoWallLayout::calculate(
            &ratios,
            &sections,
            width as f64,
            target as f64,
            caption as f64,
        );
        self.geometry.replace(
            layout
                .sections
                .iter()
                .map(|section| SectionedFolderGeometry {
                    header_y: section.header_y,
                    first_photo_y: section.first_photo_y,
                    end_y: section.end_y,
                })
                .collect(),
        );
        self.total_height.set(layout.total_height);
        let mut state = self.wall_state.borrow_mut();
        state.generation = state.generation.wrapping_add(1);
        state.layout = layout;
        self.geometry_width.set(width);
        self.geometry_row_height.set(target + caption);
    }

    fn wall_vertical_neighbor(&self, index: u32, direction: i32) -> Option<u32> {
        let state = self.wall_state.borrow();
        let item = state.layout.item(index as usize)?;
        let row = if direction < 0 {
            item.row.checked_sub(1)?
        } else {
            item.row + 1
        };
        let row = state.layout.rows.get(row)?;
        let center = item.x + item.width * 0.5;
        state.layout.items[row.item_range.clone()]
            .iter()
            .min_by(|a, b| {
                (a.x + a.width * 0.5 - center)
                    .abs()
                    .total_cmp(&(b.x + b.width * 0.5 - center).abs())
            })
            .map(|item| item.photo_index as u32)
    }

    fn wall_photo_for_y(&self, y: f64) -> Option<PhotoObject> {
        let state = self.wall_state.borrow();
        let row = state
            .layout
            .rows
            .partition_point(|row| row.y + row.block_height <= y);
        let row = state
            .layout
            .rows
            .get(row)
            .or_else(|| state.layout.rows.last())?;
        let index = state.layout.items.get(row.item_range.start)?.photo_index;
        self.current_photos.borrow().get(index).cloned()
    }

    fn wall_center_anchor(&self) -> Option<(i64, f64)> {
        let scroll = self.scroll.borrow().as_ref()?.clone();
        let adjustment = scroll.vadjustment();
        let photo = self.wall_photo_for_y(adjustment.value() + adjustment.page_size() * 0.5)?;
        let index = self
            .current_photos
            .borrow()
            .iter()
            .position(|item| item.id() == photo.id())?;
        Some((
            photo.id(),
            self.y_for_index(index as u32)? - adjustment.value(),
        ))
    }

    fn wall_scroll_to(self: &Rc<Self>, index: u32, header: bool, center: bool) -> bool {
        self.refresh();
        let Some(scroll) = self.scroll.borrow().as_ref().cloned() else {
            return false;
        };
        let state = self.wall_state.borrow();
        let Some(item) = state.layout.item(index as usize) else {
            return false;
        };
        let y = if header {
            state.layout.sections[item.section].header_y
        } else {
            item.y
        };
        let height = if header {
            SECTIONED_HEADER_HEIGHT
        } else {
            state.layout.rows[item.row].block_height
        };
        let adjustment = scroll.vadjustment();
        let page = adjustment.page_size();
        if page <= 0.0
            || !sectioned_scroll_extent_is_ready(
                state.layout.total_height,
                page,
                adjustment.upper(),
            )
        {
            return false;
        }
        let target = if center {
            y + height * 0.5 - page * 0.5
        } else if y < adjustment.value() {
            y
        } else if y + height > adjustment.value() + page {
            y + height - page
        } else {
            adjustment.value()
        };
        drop(state);
        self.cancel_scroll_animation();
        adjustment.set_value(target.clamp(
            adjustment.lower(),
            (adjustment.upper() - page).max(adjustment.lower()),
        ));
        self.refresh();
        true
    }
}

impl Gallery {
    pub fn layout(&self) -> PhotoLayout {
        self.sectioned_folder.layout_mode.get()
    }
    pub fn using_virtual_photo_surface(&self) -> bool {
        self.layout() == PhotoLayout::PhotoWall
            || (self.group_mode.get() == GroupMode::Folder && sectioned_folder_view_enabled())
    }
    pub fn set_layout(self: &Rc<Self>, layout: PhotoLayout) {
        if self.layout() == layout {
            return;
        }
        let anchor = self.capture_view_anchor();
        self.cancel_resize_flip();
        self.sectioned_folder.cancel_zoom_settle();
        self.sectioned_folder.cancel_scroll_animation();
        self.sectioned_folder.layout_mode.set(layout);
        self.sectioned_folder.refresh_model();
        if layout == PhotoLayout::Grid {
            self.last_layout_width.set(0);
            self.update_width(
                self.last_layout_width
                    .get()
                    .max(self.folder_sectioned_root.width())
                    .max(self.root.width()),
            );
        }
        if let Some(changed) = self.folder_view_changed.borrow().as_ref() {
            changed(self.group_mode.get() == GroupMode::Folder);
        }
        if let Some(anchor) = anchor {
            self.restore_view_anchor(anchor);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ViewAnchor {
    photo_id: i64,
    viewport_y_offset: f64,
}

impl Gallery {
    fn capture_view_anchor(&self) -> Option<ViewAnchor> {
        if self.using_virtual_photo_surface() {
            return self.sectioned_folder.capture_center_anchor().map(
                |(photo_id, viewport_y_offset)| ViewAnchor {
                    photo_id,
                    viewport_y_offset,
                },
            );
        }
        let photo = self.viewport_center_photo()?;
        let mut tiles = Vec::new();
        collect_tiles(self.root.upcast_ref(), &mut tiles);
        let viewport_y_offset = tiles
            .iter()
            .find(|tile| tile.photo().is_some_and(|p| p.id() == photo.id()))
            .and_then(|tile| tile.compute_bounds(&self.root))
            .map(|bounds| bounds.y() as f64)
            .unwrap_or(0.0);
        Some(ViewAnchor {
            photo_id: photo.id(),
            viewport_y_offset,
        })
    }

    fn restore_view_anchor(self: &Rc<Self>, anchor: ViewAnchor) {
        let generation = self.sectioned_folder.wall_state.borrow().generation;
        let weak = Rc::downgrade(self);
        let attempts = Cell::new(0);
        let root = self.visible_root();
        root.add_tick_callback(move |_, _| {
            let Some(gallery) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if gallery.sectioned_folder.wall_state.borrow().generation != generation {
                return glib::ControlFlow::Break;
            }
            attempts.set(attempts.get() + 1);
            let index = gallery
                .current_photos
                .borrow()
                .iter()
                .position(|photo| photo.id() == anchor.photo_id)
                .or_else(|| {
                    selected_positions(&gallery.selection)
                        .first()
                        .copied()
                        .map(|i| i as usize)
                });
            let Some(index) = index else {
                return glib::ControlFlow::Break;
            };
            let id = gallery.current_photos.borrow()[index].id();
            if gallery.using_virtual_photo_surface() {
                if gallery
                    .sectioned_folder
                    .restore_anchor(id, anchor.viewport_y_offset)
                {
                    return glib::ControlFlow::Break;
                }
            } else {
                let mut tiles = Vec::new();
                collect_tiles(gallery.root.upcast_ref(), &mut tiles);
                if let Some(bounds) = tiles
                    .iter()
                    .find(|tile| tile.photo().is_some_and(|p| p.id() == id))
                    .and_then(|tile| tile.compute_bounds(&gallery.root))
                {
                    if let Some(adjustment) = gallery.root.vadjustment() {
                        let target =
                            adjustment.value() + bounds.y() as f64 - anchor.viewport_y_offset;
                        adjustment.set_value(target.clamp(
                            adjustment.lower(),
                            (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
                        ));
                        return glib::ControlFlow::Break;
                    }
                } else {
                    gallery
                        .root
                        .scroll_to(index as u32, gtk::ListScrollFlags::NONE, None);
                }
            }
            if attempts.get() >= 12 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn apply_wall_zoom(self: &Rc<Self>, width: i32) {
        let width = nearest_zoom_level(width);
        if width == self.tile_width.get() {
            return;
        }
        let anchor = self.capture_view_anchor();
        self.auto_default_zoom.set(false);
        if let Some(source) = self.zoom_reflow_source.borrow_mut().take() {
            source.remove();
        }
        self.pending_zoom_width.set(None);
        self.tile_width.set(width);
        self.tile_height.set(
            (DEFAULT_TILE_HEIGHT as f64 * width as f64 / DEFAULT_TILE_WIDTH as f64).round() as i32,
        );
        (self.on_zoom_changed)(width);
        self.sectioned_folder.invalidate_geometry();
        self.sectioned_folder.refresh();
        if let Some(anchor) = anchor {
            self.restore_view_anchor(anchor);
        }
    }

    fn wall_target_requests(&self, scroll_y: f64, viewport_height: f64, budget: usize) -> usize {
        if budget == 0 {
            return 0;
        }
        let state = self.sectioned_folder.wall_state.borrow();
        let photos = self.current_photos.borrow();
        let mut requests = Vec::new();
        for row in state
            .layout
            .visible_rows(scroll_y, scroll_y + viewport_height)
        {
            for item in &state.layout.items[state.layout.rows[row].item_range.clone()] {
                if requests.len() >= budget {
                    break;
                }
                if let Some(photo) = photos.get(item.photo_index) {
                    if let Some(request) = photo_presentation_request(photo, true) {
                        if folder_thumbnail_cache_get(&request.key).is_none() {
                            requests.push(request);
                        }
                    }
                }
            }
        }
        crate::thumbnail_display::replace_visible_requests(requests)
    }

    fn wall_apply_cached(&self) -> usize {
        let surface = &self.sectioned_folder;
        let Some(scroll) = surface.scroll.borrow().as_ref().cloned() else {
            return 0;
        };
        let adjustment = scroll.vadjustment();
        let state = surface.wall_state.borrow();
        let tiles = surface.live_tiles.borrow();
        let mut applied = 0;
        for row in state.layout.visible_rows(
            adjustment.value(),
            adjustment.value() + adjustment.page_size(),
        ) {
            for item in &state.layout.items[state.layout.rows[row].item_range.clone()] {
                if let Some(tile) = tiles.get(&(item.photo_index as u32)) {
                    if let Some(photo) = tile.tile.photo() {
                        if let Some(key) = photo_presentation_key(&photo) {
                            if let Some(paintable) = folder_thumbnail_cache_get(&key) {
                                tile.tile.apply_presentation_paintable(&key, &paintable);
                                applied += 1;
                            }
                        }
                    }
                }
            }
        }
        applied
    }

    fn wall_prefetch(&self, budget: usize, direction: f64) -> usize {
        let Some(scroll) = self.sectioned_folder.scroll.borrow().as_ref().cloned() else {
            return 0;
        };
        let adjustment = scroll.vadjustment();
        let state = self.sectioned_folder.wall_state.borrow();
        let visible = state.layout.visible_rows(
            adjustment.value(),
            adjustment.value() + adjustment.page_size(),
        );
        let ahead = visible.len().max(1) * 3;
        let band = if direction < 0.0 {
            visible.start.saturating_sub(ahead)..visible.start
        } else {
            visible.end..(visible.end + ahead).min(state.layout.rows.len())
        };
        let photos = self.current_photos.borrow();
        let mut queued = 0;
        for row in band {
            for item in &state.layout.items[state.layout.rows[row].item_range.clone()] {
                if queued >= budget {
                    return queued;
                }
                if let Some(photo) = photos.get(item.photo_index) {
                    if queue_photo_presentation_async(photo, false) {
                        queued += 1;
                    }
                }
            }
        }
        queued
    }
}
