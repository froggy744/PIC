// Gallery thumbnail presentation: which tiles are visible, and getting their
// thumbnails onto them.
//
// Every gallery surface answers the same question here -- which tiles are
// placed where, and what is the visible band -- through
// `visible_presentation_tiles`. The queue and apply steps below are written
// once against that snapshot instead of once per surface. See
// AUDIT-INDEX.md, "Gallery thumbnail & scroll pipeline", for the invariants
// this code protects (placement vs allocation, visible-queue replacement).

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
