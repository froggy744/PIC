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
        self.cancel_resize_flip();
        self.sectioned_folder.cancel_zoom_settle();
        self.sectioned_folder.cancel_scroll_animation();
        self.sectioned_folder.layout_mode.set(layout);
        self.sectioned_folder.refresh_model();
        if let Some(changed) = self.folder_view_changed.borrow().as_ref() {
            changed(self.group_mode.get() == GroupMode::Folder);
        }
    }
}
