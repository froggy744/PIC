//! One presentation tween for Library and Folder zoom/resize.
use super::*;

#[derive(Default)]
pub(super) struct InPlaceTween {
    generation: Cell<u64>,
    tiles: RefCell<Vec<TileMotion>>,
    carried: RefCell<Vec<(SquareTile, i64)>>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
}

pub(super) type TileRect = (f64, f64, f64, f64);
pub(super) type TileChange = (SquareTile, TileRect, TileRect);

pub(super) fn visual_rect(tile: &SquareTile, bounds: &gtk::graphene::Rect) -> TileRect {
    let (dx, dy) = tile.presentation_translate();
    let (sx, sy) = tile.presentation_scale();
    let width = f64::from(bounds.width());
    let height = f64::from(bounds.height());
    (
        f64::from(bounds.x()) + f64::from(dx) + width * (1.0 - f64::from(sx)) * 0.5,
        f64::from(bounds.y()) + f64::from(dy) + height * (1.0 - f64::from(sy)) * 0.5,
        width * f64::from(sx),
        height * f64::from(sy),
    )
}

struct TileMotion {
    tile: SquareTile,
    id: i64,
    from_dx: f32,
    from_dy: f32,
    from_sx: f32,
    from_sy: f32,
    delay_ms: f64,
}

impl InPlaceTween {
    pub(super) fn is_active(&self) -> bool {
        self.tick.borrow().is_some()
    }

    pub(super) fn cancel(&self, preserve_tiles: bool) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(tick) = self.tick.borrow_mut().take() {
            tick.remove();
        }
        let current = self
            .tiles
            .borrow_mut()
            .drain(..)
            .map(|motion| (motion.tile, motion.id))
            .collect::<Vec<_>>();
        let carried = self.carried.borrow_mut().drain(..).collect::<Vec<_>>();
        if preserve_tiles {
            self.carried
                .replace(current.into_iter().chain(carried).collect());
        } else {
            for (tile, _) in current.into_iter().chain(carried) {
                tile.reset_presentation_transform();
            }
        }
        set_grid_zoom_animation_active(false);
    }

    pub(super) fn animate_tile_in_place(
        self: &Rc<Self>,
        root: &impl IsA<gtk::Widget>,
        duration_ms: f64,
        preserve_tiles: bool,
        prepare: impl FnOnce() -> Vec<TileChange> + 'static,
    ) {
        if !root.is_mapped() || !root.settings().is_gtk_enable_animations() {
            self.cancel(false);
            return;
        }
        self.cancel(preserve_tiles);
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        let prepare = RefCell::new(Some(prepare));
        let started = Cell::new(None);
        let waiting_for_layout = Cell::new(true);
        set_grid_zoom_animation_active(true);
        let tick = root.add_tick_callback(move |root, clock| {
            let Some(tween) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if tween.generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            if !root.is_mapped() {
                tween.finish();
                return glib::ControlFlow::Break;
            }
            // Tick callbacks run before GTK layout. Let the first layout pass
            // allocate the destination before comparing cells on the next tick.
            // Real tiles remain fully drawn throughout this pass.
            if waiting_for_layout.replace(false) {
                return glib::ControlFlow::Continue;
            }
            if let Some(prepare) = prepare.borrow_mut().take() {
                let changes = prepare();
                let anchor_x = changes
                    .iter()
                    .map(|(_, _, rect)| rect.0)
                    .fold(f64::INFINITY, f64::min);
                let anchor_y = changes
                    .iter()
                    .map(|(_, _, rect)| rect.1)
                    .fold(f64::INFINITY, f64::min);
                let max_distance = changes
                    .iter()
                    .map(|(_, _, rect)| (rect.0 - anchor_x).abs() + (rect.1 - anchor_y).abs())
                    .fold(1.0_f64, f64::max);
                let tiles: Vec<TileMotion> = changes
                    .into_iter()
                    .filter_map(|(tile, old, new)| {
                        if !tile.is_mapped() || !tile.is_visible() {
                            return None;
                        }
                        let id = tile.photo()?.id();
                        let sx = (old.2 / new.2.max(1.0)) as f32;
                        let sy = (old.3 / new.3.max(1.0)) as f32;
                        let dx = (old.0 - new.0 - (new.2 - old.2) * 0.5) as f32;
                        let dy = (old.1 - new.1 - (new.3 - old.3) * 0.5) as f32;
                        if dx.abs() < 0.5
                            && dy.abs() < 0.5
                            && (sx - 1.0).abs() < 0.005
                            && (sy - 1.0).abs() < 0.005
                        {
                            tile.reset_presentation_transform();
                            return None;
                        }
                        let distance =
                            ((new.0 - anchor_x).abs() + (new.1 - anchor_y).abs()) / max_distance;
                        Some(TileMotion {
                            tile,
                            id,
                            from_dx: dx,
                            from_dy: dy,
                            from_sx: sx,
                            from_sy: sy,
                            delay_ms: distance * 40.0,
                        })
                    })
                    .collect();
                let continued = tiles.iter().map(|motion| motion.id).collect::<HashSet<_>>();
                for (tile, id) in tween.carried.borrow_mut().drain(..) {
                    if !continued.contains(&id) {
                        tile.reset_presentation_transform();
                    }
                }
                let animated_tiles = tiles.len();
                tween.tiles.replace(tiles);
                started.set(Some(clock.frame_time()));
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_TILE_TWEEN animated_tiles={animated_tiles} spread_ms=40 duration_ms={:.0} easing=quint_out",
                        duration_ms.max(16.0),
                    );
                }
            }
            let elapsed = (clock.frame_time() - started.get().unwrap()) as f64 / 1000.0;
            let mut complete = true;
            for motion in tween.tiles.borrow().iter() {
                let t =
                    ((elapsed - motion.delay_ms / 1000.0) / duration_ms.max(16.0)).clamp(0.0, 1.0);
                let remaining = (1.0 - t).powi(5) as f32;
                if motion
                    .tile
                    .photo()
                    .is_some_and(|photo| photo.id() == motion.id)
                {
                    motion.tile.set_presentation_translate(
                        motion.from_dx * remaining,
                        motion.from_dy * remaining,
                    );
                    motion.tile.set_presentation_scale(
                        1.0 + (motion.from_sx - 1.0) * remaining,
                        1.0 + (motion.from_sy - 1.0) * remaining,
                    );
                    complete &= t >= 1.0;
                } else {
                    motion.tile.reset_presentation_transform();
                }
            }
            if complete || tween.tiles.borrow().is_empty() {
                tween.finish();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        self.tick.replace(Some(tick));
    }

    fn finish(&self) {
        self.tick.borrow_mut().take();
        for motion in self.tiles.borrow_mut().drain(..) {
            motion.tile.reset_presentation_transform();
        }
        for (tile, _) in self.carried.borrow_mut().drain(..) {
            tile.reset_presentation_transform();
        }
        set_grid_zoom_animation_active(false);
    }
}

impl Drop for InPlaceTween {
    fn drop(&mut self) {
        self.cancel(false);
    }
}
