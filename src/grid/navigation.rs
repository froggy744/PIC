impl Gallery {
    /// Photo that must stay at the top across a reshape. While a previous
    /// reframe is still settling, keep its photo: the viewport is mid-flight
    /// and no longer describes the user's position. Otherwise capture the
    /// current photo (or a zoom's saved anchor) and mark the reframe pending.
    fn take_reframe_anchor(&self) -> Option<i64> {
        if self.folder_pending_reframe.get() {
            return self.folder_reframe_photo.get();
        }
        let captured = self.zoom_anchor.take().or_else(|| {
            self.photo_for_scroll_position(self.last_scroll_y.get())
                .map(|photo| photo.id())
        });
        if let Some(photo_id) = captured {
            self.folder_reframe_photo.set(Some(photo_id));
            self.folder_pending_reframe.set(true);
        }
        captured
    }

    /// Keep the Folder viewport on `photo_id` after the rows were reshaped by a
    /// zoom/column change. Realizes the target row, then aligns its tile to the
    /// top edge using the tile's computed bounds (independent of the variable
    /// row heights). Waits for allocation and bounds the settling corrections.
    fn scroll_folder_to_photo(&self, photo_id: i64) {
        self.folder_anchor_photo.set(Some(photo_id));
        let Some(row) = self.folder_row_index_for_photo(photo_id) else {
            return;
        };
        let root = self.folder_root.clone();
        let generation = self.folder_scroll_generation.clone();
        let request = generation.get().wrapping_add(1);
        generation.set(request);
        let model_generation = self.replace_generation.clone();
        let model_request = model_generation.get();
        let mode = self.group_mode.clone();
        let pending = self.folder_pending_reframe.clone();
        let reframe_photo = self.folder_reframe_photo.clone();
        let trace = std::env::var_os("PIC_GALLERY_FOLDER_REFRAME_TRACE").is_some();
        glib::idle_add_local_once(move || {
            if generation.get() != request
                || model_generation.get() != model_request
                || mode.get() != GroupMode::Folder
            {
                if trace {
                    eprintln!(
                        "PIC_FOLDER_REFRAME stage=cancel_idle request={} photo={} current_generation={} model_request={} current_model={} mode={:?}",
                        request,
                        photo_id,
                        generation.get(),
                        model_request,
                        model_generation.get(),
                        mode.get(),
                    );
                }
                return;
            }
            // Ask GtkListView to realize the target row using its virtual
            // geometry. Summed model heights drift from GTK's allocated row
            // heights over deep lists, which can leave the target unbound after
            // a column-count change. The frame callback aligns the realized
            // tile once GTK publishes its actual bounds.
            root.scroll_to(row, gtk::ListScrollFlags::NONE, None);
            if trace {
                let adjustment = root.vadjustment();
                eprintln!(
                    "PIC_FOLDER_REFRAME stage=scroll_to request={} photo={} row={} value={:.1} upper={:.1} page={:.1}",
                    request,
                    photo_id,
                    row,
                    adjustment.as_ref().map_or(-1.0, |value| value.value()),
                    adjustment.as_ref().map_or(-1.0, |value| value.upper()),
                    adjustment.as_ref().map_or(-1.0, |value| value.page_size()),
                );
            }
            // An idle after another idle does not guarantee a GTK allocation.
            // Wait for frame ticks, then correct using the mapped tile bounds.
            let frames = Cell::new(0);
            let stable_frames = Cell::new(0);
            root.add_tick_callback(move |root_for_align, _| {
                if generation.get() != request
                    || model_generation.get() != model_request
                    || mode.get() != GroupMode::Folder
                {
                    return glib::ControlFlow::Break;
                }
                frames.set(frames.get() + 1);
                if frames.get() < 2 {
                    return glib::ControlFlow::Continue;
                }
                let mut tiles = Vec::new();
                collect_tiles(root_for_align.upcast_ref(), &mut tiles);
                let tile = tiles.into_iter().find(|tile| {
                    tile.is_mapped()
                        && tile.is_visible()
                        && tile
                            .imp()
                            .photo
                            .borrow()
                            .as_ref()
                            .is_some_and(|photo| photo.id() == photo_id)
                });
                let Some(tile) = tile else {
                    if trace {
                        eprintln!(
                            "PIC_FOLDER_REFRAME stage=retry_missing request={} photo={} frame={}",
                            request,
                            photo_id,
                            frames.get(),
                        );
                    }
                    if frames.get() < 8 {
                        return glib::ControlFlow::Continue;
                    }
                    pending.set(false);
                    reframe_photo.set(None);
                    return glib::ControlFlow::Break;
                };
                let Some(bounds) = tile.compute_bounds(root_for_align) else {
                    if frames.get() < 8 {
                        return glib::ControlFlow::Continue;
                    }
                    pending.set(false);
                    reframe_photo.set(None);
                    return glib::ControlFlow::Break;
                };
                if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
                    if frames.get() < 8 {
                        return glib::ControlFlow::Continue;
                    }
                    pending.set(false);
                    reframe_photo.set(None);
                    return glib::ControlFlow::Break;
                }
                let Some(adjustment) = root_for_align.vadjustment() else {
                    pending.set(false);
                    return glib::ControlFlow::Break;
                };
                // Align the row, not the tile's six-pixel top margin. GTK may
                // refine its estimated row heights for another frame after a
                // splice; allow a bounded correction until the anchor settles.
                // GTK clips a tile that has reached the leading edge, so its
                // reported bounds may be 0 while its top margin is still
                // present. Never keep subtracting that margin from an already
                // clipped tile; doing so walks the scroll position backwards
                // on every frame during an overlapping reflow.
                let visible_delta =
                    (bounds.y() as f64 - f64::from(tile.margin_top())).max(0.0);
                let target = adjustment.value() + visible_delta;
                let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
                let target = target.clamp(adjustment.lower(), upper);
                if trace {
                    eprintln!(
                        "PIC_FOLDER_REFRAME stage=frame request={} photo={} frame={} bounds_y={:.1} target={:.1} value={:.1} upper={:.1} page={:.1} stable={}",
                        request,
                        photo_id,
                        frames.get(),
                        bounds.y(),
                        target,
                        adjustment.value(),
                        adjustment.upper(),
                        adjustment.page_size(),
                        stable_frames.get(),
                    );
                }
                if (target - adjustment.value()).abs() <= 1.0 {
                    stable_frames.set(stable_frames.get() + 1);
                } else {
                    stable_frames.set(0);
                    adjustment.set_value(target);
                }
                if stable_frames.get() >= 2 || frames.get() >= 8 {
                    // Selecting a fresh anchor is allowed again once this
                    // reshape has been positioned.
                    pending.set(false);
                    reframe_photo.set(None);
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        });
    }
}
fn schedule_scroll_restore(
    adjustment: Option<gtk::Adjustment>,
    scroll_y: f64,
    replace_generation: Rc<Cell<u64>>,
    generation: u64,
) {
    glib::idle_add_local_once(move || {
        if replace_generation.get() != generation {
            return;
        }
        if let Some(adjustment) = adjustment {
            let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
            adjustment.set_value(scroll_y.clamp(adjustment.lower(), upper));
        }
    });
}

/// One Folder photo line: the tile block (square thumbnail plus its optional
/// filename row) plus the 6 px top/bottom spacing every tile line carries.
fn folder_line_height(tile_height: i32, filename_visible: bool) -> i32 {
    tile_block_height(tile_height, filename_visible) + TILE_ROW_SPACING
}

fn box_tiles(line: &gtk::Box) -> Vec<SquareTile> {
    let mut tiles = Vec::new();
    let mut child = line.first_child();
    while let Some(current) = child {
        if let Ok(tile) = current.clone().downcast::<SquareTile>() {
            tiles.push(tile);
        }
        child = current.next_sibling();
    }
    tiles
}

fn update_folder_realized_rows(
    widget: &gtk::Widget,
    tile_width: i32,
    tile_height: i32,
    filename_visible: bool,
) {
    if widget.widget_name().as_str() == "picasa-folder-photo-line" {
        if let Some(line) = widget.downcast_ref::<gtk::Box>() {
            for tile in box_tiles(line) {
                tile.set_tile_size(tile_width, tile_height);
                tile.set_filename_visible(filename_visible);
            }
            if line.is_visible() {
                if let Some(row_root) = line.parent().and_downcast::<gtk::Box>() {
                    row_root.set_height_request(folder_model_row_height(
                        FolderRowKind::Photos,
                        tile_height,
                        filename_visible,
                    ));
                }
            }
        }
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        update_folder_realized_rows(&current, tile_width, tile_height, filename_visible);
        child = current.next_sibling();
    }
}

fn collect_tiles(widget: &gtk::Widget, tiles: &mut Vec<SquareTile>) {
    if let Some(tile) = widget.downcast_ref::<SquareTile>() {
        tiles.push(tile.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        collect_tiles(&current, tiles);
        child = current.next_sibling();
    }
}
