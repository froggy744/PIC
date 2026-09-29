//! One presentation tween for Library and Folder zoom/resize.
use super::*;

#[derive(Default)]
pub(super) struct InPlaceTween {
    generation: Cell<u64>,
    tiles: RefCell<Vec<(SquareTile, i64)>>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
}

impl InPlaceTween {
    pub(super) fn is_active(&self) -> bool {
        self.tick.borrow().is_some()
    }

    pub(super) fn cancel(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(tick) = self.tick.borrow_mut().take() {
            tick.remove();
        }
        for (tile, _) in self.tiles.borrow_mut().drain(..) {
            tile.reset_presentation_transform();
        }
        set_grid_zoom_animation_active(false);
    }

    pub(super) fn animate_tile_in_place(
        self: &Rc<Self>,
        root: &impl IsA<gtk::Widget>,
        duration_ms: f64,
        prepare: impl FnOnce() -> Vec<SquareTile> + 'static,
    ) {
        self.cancel();
        if !root.is_mapped() || !root.settings().is_gtk_enable_animations() {
            return;
        }
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
                let tiles = prepare()
                    .into_iter()
                    .filter_map(|tile| {
                        if !tile.is_mapped() || !tile.is_visible() {
                            return None;
                        }
                        let id = tile.photo()?.id();
                        tile.set_presentation_scale(0.96, 0.96);
                        tile.set_presentation_opacity(0.85);
                        Some((tile, id))
                    })
                    .collect();
                tween.tiles.replace(tiles);
                started.set(Some(clock.frame_time()));
            }
            let elapsed = (clock.frame_time() - started.get().unwrap()) as f64 / 1000.0;
            let t = (elapsed / duration_ms.max(16.0)).clamp(0.0, 1.0);
            let remaining = (1.0 - t).powi(3) as f32;
            for (tile, id) in tween.tiles.borrow().iter() {
                if tile.photo().is_some_and(|photo| photo.id() == *id) {
                    let scale = 1.0 - 0.04 * remaining;
                    tile.set_presentation_scale(scale, scale);
                    tile.set_presentation_opacity(1.0 - 0.15 * remaining);
                } else {
                    tile.reset_presentation_transform();
                }
            }
            if t >= 1.0 || tween.tiles.borrow().is_empty() {
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
        for (tile, _) in self.tiles.borrow_mut().drain(..) {
            tile.reset_presentation_transform();
        }
        set_grid_zoom_animation_active(false);
    }
}

impl Drop for InPlaceTween {
    fn drop(&mut self) {
        self.cancel();
    }
}
