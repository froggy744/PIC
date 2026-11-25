// Focus mode: in All Photos, every gallery tile except the lit one is dimmed
// so a single photo stands out while culling a burst.
//
// The lit photo follows whichever moved last: the pointer entering a tile,
// or a single-photo selection (arrow keys, clicks). Leaving a tile changes
// nothing, so the last photo stays lit while the pointer crosses the gaps.
//
// Unlit tiles are shaded toward black at `FOCUS_DIM_STRENGTH` ("lights
// down"): a black layer inside the photo frame, which the frame clips to the
// theme's rounded corners, or a hexagon-masked shade in Honeycomb. Dimming
// toward the background instead washed photos out in light themes. Neither
// path needs the per-tile offscreen pass that widget opacity on the whole
// tile would cost.

/// Strength of the black shade over unlit tiles.
const FOCUS_DIM_STRENGTH: f32 = 0.5;
/// A photo lighting up fades in quickly enough to follow the pointer...
const FOCUS_FADE_IN_US: i64 = 180_000;
/// ...while the one left behind dims a little slower, so the hand-over reads
/// as a soft cross-fade rather than a flicker.
const FOCUS_FADE_OUT_US: i64 = 250_000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct FocusModeState {
    /// The user's toggle.
    enabled: bool,
    /// The current view allows focus mode (All Photos only).
    in_scope: bool,
    lit: Option<i64>,
}

impl FocusModeState {
    fn active(&self) -> bool {
        self.enabled && self.in_scope
    }

    /// Whether a tile showing `photo_id` is drawn dimmed.
    fn dims(&self, photo_id: Option<i64>) -> bool {
        self.active() && self.lit.is_some() && photo_id.is_some() && photo_id != self.lit
    }

    /// Light `photo_id` because the pointer or the selection moved to it.
    /// Ignored while focus mode is inactive so normal browsing costs nothing.
    fn follow(&mut self, photo_id: i64) {
        if self.active() {
            self.lit = Some(photo_id);
        }
    }
}

/// Dim level `elapsed_us` into a fade, and whether the fade is finished.
/// Lighting up (target below `from`) uses the fade-in time, dimming the
/// fade-out time; both ease out (fast start, gentle settle).
fn focus_fade_level(from: f32, target: f32, elapsed_us: i64) -> (f32, bool) {
    let duration = if target < from {
        FOCUS_FADE_IN_US
    } else {
        FOCUS_FADE_OUT_US
    };
    let t = (elapsed_us as f32 / duration as f32).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t).powi(3);
    (from + (target - from) * eased, t >= 1.0)
}

thread_local! {
    static FOCUS_MODE: RefCell<FocusModeState> = RefCell::default();
    static FOCUS_TILES: RefCell<Vec<glib::WeakRef<SquareTile>>> = RefCell::default();
}

fn update_focus_mode(change: impl FnOnce(&mut FocusModeState)) {
    let changed = FOCUS_MODE.with(|state| {
        let mut state = state.borrow_mut();
        let before = *state;
        change(&mut state);
        *state != before
    });
    if changed {
        sync_focus_tiles();
    }
}

pub(crate) fn set_focus_mode_enabled(enabled: bool) {
    update_focus_mode(|state| state.enabled = enabled);
}

/// Called whenever the gallery view changes; only All Photos is in scope.
pub(crate) fn set_focus_mode_in_scope(in_scope: bool) {
    update_focus_mode(|state| state.in_scope = in_scope);
}

/// Seed the lit photo, e.g. from the selection when focus mode turns on.
pub(crate) fn set_focus_mode_lit(photo_id: Option<i64>) {
    update_focus_mode(|state| state.lit = photo_id);
}

fn focus_mode_follow(photo_id: i64) {
    update_focus_mode(|state| state.follow(photo_id));
}

fn focus_mode_dims(photo_id: Option<i64>) -> bool {
    FOCUS_MODE.with(|state| state.borrow().dims(photo_id))
}

fn register_focus_tile(tile: &SquareTile) {
    FOCUS_TILES.with(|tiles| {
        let mut tiles = tiles.borrow_mut();
        if tiles.len() >= 64 && tiles.len().is_power_of_two() {
            tiles.retain(|tile| tile.upgrade().is_some());
        }
        tiles.push(tile.downgrade());
    });
}

fn sync_focus_tiles() {
    let tiles: Vec<SquareTile> = FOCUS_TILES.with(|tiles| {
        let mut tiles = tiles.borrow_mut();
        tiles.retain(|tile| tile.upgrade().is_some());
        tiles.iter().filter_map(glib::WeakRef::upgrade).collect()
    });
    for tile in tiles {
        tile.sync_focus_dim(true);
    }
}

#[cfg(test)]
mod focus_mode_tests {
    use super::FocusModeState;

    fn active(lit: Option<i64>) -> FocusModeState {
        FocusModeState {
            enabled: true,
            in_scope: true,
            lit,
        }
    }

    #[test]
    fn lighting_up_takes_180_ms_and_dimming_250_ms() {
        use super::focus_fade_level;
        assert_eq!(focus_fade_level(1.0, 0.0, 0), (1.0, false));
        assert_eq!(focus_fade_level(1.0, 0.0, 180_000), (0.0, true));
        assert_eq!(focus_fade_level(0.0, 1.0, 180_000).1, false);
        assert_eq!(focus_fade_level(0.0, 1.0, 250_000), (1.0, true));
        assert_eq!(focus_fade_level(0.0, 1.0, 900_000), (1.0, true));
    }

    #[test]
    fn fades_ease_out() {
        use super::focus_fade_level;
        // Half the time covers well over half the change.
        let (level, done) = focus_fade_level(0.0, 1.0, 125_000);
        assert!(!done && level > 0.8, "{level}");
    }

    #[test]
    fn only_the_lit_photo_stays_undimmed() {
        let state = active(Some(2));
        assert!(!state.dims(Some(2)));
        assert!(state.dims(Some(1)));
        assert!(state.dims(Some(3)));
    }

    #[test]
    fn nothing_dims_until_a_photo_is_lit() {
        assert!(!active(None).dims(Some(1)));
    }

    #[test]
    fn empty_tiles_never_dim() {
        assert!(!active(Some(2)).dims(None));
    }

    #[test]
    fn views_other_than_all_photos_never_dim() {
        let state = FocusModeState {
            in_scope: false,
            ..active(Some(2))
        };
        assert!(!state.dims(Some(1)));
    }

    #[test]
    fn toggle_off_never_dims() {
        let state = FocusModeState {
            enabled: false,
            ..active(Some(2))
        };
        assert!(!state.dims(Some(1)));
    }

    #[test]
    fn the_last_pointer_or_selection_move_wins() {
        let mut state = active(Some(1));
        state.follow(2);
        assert_eq!(state.lit, Some(2));
        state.follow(3);
        assert_eq!(state.lit, Some(3));
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn unlit_tiles_shade_toward_black_inside_their_frame() {
        use super::*;
        gtk::init().unwrap();
        // A light theme: white gallery, mid-grey photos, rounded frames.
        let css = gtk::CssProvider::new();
        css.load_from_string(&format!(
            "{}
             .focus-test-grid {{ background: rgb(255, 255, 255); }}
             .focus-test-frame {{ border-radius: 16px; }}
             .focus-test-photo {{ background: rgb(200, 200, 200); }}",
            crate::css::BASE
        ));
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().unwrap(),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 6,
        );
        let grid = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        grid.add_css_class("photo-grid");
        grid.add_css_class("focus-test-grid");
        let tiles: Vec<SquareTile> = (1..=2_i64)
            .map(|id| {
                let frame = gtk::Overlay::new();
                frame.set_overflow(gtk::Overflow::Hidden);
                frame.add_css_class("focus-test-frame");
                let photo = gtk::Box::new(gtk::Orientation::Vertical, 0);
                photo.add_css_class("focus-test-photo");
                frame.set_child(Some(&photo));
                let tile = SquareTile::new(64, 64, &frame);
                tile.imp().photo.replace(Some(
                    glib::Object::builder::<PhotoObject>()
                        .property("id", id)
                        .property("path", format!("/virtual/focus-{id}.jpg"))
                        .build(),
                ));
                grid.append(&tile);
                tile
            })
            .collect();
        // Run the real fades, whatever the session's default.
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(true);
        let window = gtk::Window::new();
        window.set_child(Some(&grid));
        window.present();
        let settle = || {
            for _ in 0..100 {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        settle();
        let render = |node: &gtk::gsk::RenderNode, w: i32, h: i32| {
            let texture = window.renderer().unwrap().render_texture(
                node,
                Some(&gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32)),
            );
            let mut pixels = vec![0; texture.width() as usize * texture.height() as usize * 4];
            texture.download(&mut pixels, texture.width() as usize * 4);
            let width = texture.width() as usize;
            move |x: usize, y: usize| pixels[(y * width + x) * 4..(y * width + x) * 4 + 4].to_vec()
        };
        // Pixels of a tile in its own coordinates (grey, so channel order
        // does not matter).
        let pixels = |tile: &SquareTile| {
            let snapshot = gtk::Snapshot::new();
            gtk::subclass::prelude::WidgetImpl::snapshot(tile.imp(), &snapshot);
            render(&snapshot.to_node().unwrap(), tile.width(), tile.height())
        };
        let grey =
            |pixel: &[u8], low: u8, high: u8| pixel[..3].iter().all(|&c| (low..=high).contains(&c));

        set_focus_mode_lit(None);
        set_focus_mode_in_scope(true);
        set_focus_mode_enabled(true);
        // Entering tile 1 lights it through the PRELIGHT hook.
        tiles[0].set_state_flags(gtk::StateFlags::PRELIGHT, false);
        settle();
        let lit = pixels(&tiles[0]);
        let dimmed = pixels(&tiles[1]);
        let (cx, cy) = (32, 32);
        assert!(grey(&lit(cx, cy), 195, 205), "lit tile {:?}", lit(cx, cy));
        // Half way to black: 200 -> 100, not toward the white background.
        assert!(
            grey(&dimmed(cx, cy), 92, 108),
            "dimmed tile {:?}",
            dimmed(cx, cy)
        );
        // The rounded corner stays empty: the shade is clipped by the frame.
        assert_eq!(
            dimmed(1, 1)[3],
            0,
            "shade outside the frame {:?}",
            dimmed(1, 1)
        );

        // Leaving tile 1 keeps it lit; out of All Photos nothing dims.
        tiles[0].unset_state_flags(gtk::StateFlags::PRELIGHT);
        settle();
        assert!(grey(&pixels(&tiles[0])(cx, cy), 195, 205));
        set_focus_mode_in_scope(false);
        settle();
        assert!(grey(&pixels(&tiles[1])(cx, cy), 195, 205));

        // Honeycomb: the shade is masked to the hexagon, so a neighbour's
        // corner is never covered by it.
        set_focus_mode_in_scope(true);
        settle();
        let hex = &tiles[1];
        hex.set_honeycomb(true);
        hex.imp().hex_width.set(hex.width() as f64);
        hex.imp().hex_height.set(hex.height() as f64);
        settle();
        let snapshot = gtk::Snapshot::new();
        hex.snapshot_honeycomb_for_bench(&snapshot);
        let hexagon = render(&snapshot.to_node().unwrap(), hex.width(), hex.height());
        assert_eq!(
            hexagon(1, 1)[3],
            0,
            "shade outside the hexagon {:?}",
            hexagon(1, 1)
        );
        assert!(
            grey(&hexagon(cx, cy), 92, 108),
            "hexagon {:?}",
            hexagon(cx, cy)
        );

        set_focus_mode_enabled(false);
        set_focus_mode_lit(None);
        window.destroy();
    }

    #[test]
    fn moves_while_inactive_do_not_change_the_lit_photo() {
        let mut state = FocusModeState {
            in_scope: false,
            ..active(Some(1))
        };
        state.follow(2);
        assert_eq!(state.lit, Some(1));
    }
}
