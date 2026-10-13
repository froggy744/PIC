//! GTK4/GSK composited screensaver stage.
//! All photos are decoded on a worker into RGBA8. The UI retains GdkTexture
//! paintables and animates opacity/geometry, never repainting pixels in Cairo.
use super::*;
use gtk::prelude::*;

fn present(picture: &gtk::Picture, slot: &mut Option<(usize, u64)>,
           index: usize, player: &Player) -> bool {
    let Some(image) = player.frames.get(&index) else { return false };
    let version = (index, image.revision);
    if *slot != Some(version) {
        picture.set_paintable(Some(&image.texture));
        *slot = Some(version);
    }
    true
}

struct Tile {
    frame: gtk::Overlay,
    back: gtk::Picture,
    front: gtk::Picture,
    back_version: Option<(usize, u64)>,
    front_version: Option<(usize, u64)>,
}
impl Tile {
    fn new() -> Self {
        let frame = gtk::Overlay::new();
        let back = gtk::Picture::new();
        let front = gtk::Picture::new();
        for picture in [&back, &front] {
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            picture.set_can_shrink(true);
        }
        frame.set_child(Some(&back));
        frame.add_overlay(&front);
        front.set_opacity(0.0);
        frame.add_css_class("screensaver-card");
        Self { frame, back, front, back_version: None, front_version: None }
    }
    fn sync(&mut self, new_id: usize, old: Option<usize>, progress: f64, player: &Player) {
        if let Some(old_id) = old.filter(|old_id| *old_id != new_id && progress < 1.0) {
            // Retain the outgoing texture even if its index was evicted from
            // the LRU. Never expose an empty/black layer during a fade.
            let outgoing_ready = self.back_version.is_some_and(|(id,_)| id == old_id)
                || present(&self.back, &mut self.back_version, old_id, player);
            if outgoing_ready && present(&self.front, &mut self.front_version, new_id, player) {
                self.front.set_opacity(progress);
                return;
            }
        }
        if present(&self.back, &mut self.back_version, new_id, player) {
            self.front.set_opacity(0.0);
            self.front.set_paintable(None::<&gtk::gdk::Paintable>);
            self.front_version = None;
        }
    }
}

struct Cinema {
    back_frame: gtk::Overlay,
    front_frame: gtk::Overlay,
    back: gtk::Picture,
    front: gtk::Picture,
    back_version: Option<(usize, u64)>,
    front_version: Option<(usize, u64)>,
}
impl Cinema {
    fn new(root: &gtk::Fixed) -> Self {
        let back = gtk::Picture::new();
        let front = gtk::Picture::new();
        let back_frame = gtk::Overlay::new();
        let front_frame = gtk::Overlay::new();
        for picture in [&back, &front] {
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            picture.set_can_shrink(true);
        }
        // Crossfade COMPLETE frames, including their letterbox background.
        // Fading only the photograph leaves the previous photo visible in the
        // margins until the final frame, producing a sudden apparent cut.
        for frame in [&back_frame, &front_frame] {
            frame.add_css_class("screensaver-cinema-frame");
        }
        back_frame.set_child(Some(&back));
        front_frame.set_child(Some(&front));
        front_frame.set_opacity(0.0);
        root.put(&back_frame, 0.0, 0.0);
        root.put(&front_frame, 0.0, 0.0);
        Self {
            back_frame, front_frame, back, front,
            back_version: None, front_version: None,
        }
    }
    fn place(root: &gtk::Fixed, frame: &gtk::Overlay, w: f64, h: f64, zoom: f64) {
        let width = (w * zoom).round() as i32;
        let height = (h * zoom).round() as i32;
        if frame.width_request() != width || frame.height_request() != height {
            frame.set_size_request(width, height);
        }
        let x = (w - width as f64) / 2.0;
        let y = (h - height as f64) / 2.0;
        let (old_x, old_y) = root.child_position(frame);
        if (old_x - x).abs() > 0.25 || (old_y - y).abs() > 0.25 {
            root.move_(frame, x, y);
        }
    }
    fn sync(&mut self, root: &gtk::Fixed, player: &Player, w: f64, h: f64) {
        let elapsed = player.switched_at.elapsed().as_secs_f64();
        let duration = player.interval.as_secs_f64().max(1.0);
        let incoming_zoom = cinematic_motion(player.current, elapsed / duration).0;
        let fade = smootherstep(player.progress());
        let transitioning = player.previous.filter(|old| *old != player.current && fade < 1.0);
        if let Some(old) = transitioning {
            // Old frame continues at its last zoom, independently from new.
            let outgoing_ready = self.back_version.is_some_and(|(id, _)| id == old)
                || present(&self.back, &mut self.back_version, old, player);
            if outgoing_ready
                && present(&self.front, &mut self.front_version, player.current, player)
            {
                Self::place(root, &self.back_frame, w, h, cinematic_motion(old, 1.0).0);
                Self::place(root, &self.front_frame, w, h, incoming_zoom);
                self.back_frame.set_opacity(1.0);
                // GPU blends two opaque frames: the old photo AND its margins
                // fade naturally into the new photo AND its margins without a
                // brightness dip in the overlapping image area.
                self.front_frame.set_opacity(fade);
                return;
            }
        }
        // At 100% incoming opacity the back can switch to the same image,
        // then the top can disappear. Both sides of the swap are identical.
        if present(&self.back, &mut self.back_version, player.current, player) {
            Self::place(root, &self.back_frame, w, h, incoming_zoom);
            self.back_frame.set_opacity(1.0);
            self.front_frame.set_opacity(0.0);
            self.front.set_paintable(None::<&gtk::gdk::Paintable>);
            self.front_version = None;
        }
    }
}

pub(super) struct Stage {
    pub root: gtk::Fixed,
    style: Style,
    tiles: Vec<Tile>,
    cinema: Option<Cinema>,
}
impl Stage {
    pub fn new(style: Style, count: usize) -> Self {
        let root = gtk::Fixed::new();
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.add_css_class("screensaver-stage");
        let cinema = if style == Style::IPhoto {
            Some(Cinema::new(&root))
        } else { None };
        let mut tiles = Vec::new();
        if style != Style::IPhoto {
            for slot in 0..MOSAIC_SLOTS.min(count) {
                let tile = Tile::new();
                if style == Style::PicturePile {
                    tile.frame.add_css_class("screensaver-print");
                    tile.frame.add_css_class(&format!("screensaver-angle-{slot}"));
                }
                root.put(&tile.frame, 0.0, 0.0);
                tiles.push(tile);
            }
        }
        Self { root, style, tiles, cinema }
    }
    pub fn sync(&mut self, player: &Player) {
        let w = self.root.width() as f64;
        let h = self.root.height() as f64;
        if w < 2.0 || h < 2.0 { return; }
        if let Some(cinema) = self.cinema.as_mut() {
            cinema.sync(&self.root, player, w, h);
            return;
        }
        let fade = smootherstep(player.progress());
        for (slot, tile) in self.tiles.iter_mut().enumerate() {
            let Some(&new_id) = player.slots.get(slot) else { continue };
            let (x, y, tile_w, tile_h) = if self.style == Style::Mosaic {
                let margin = (w.min(h) * 0.017).max(8.0);
                let tw = ((w - margin * 4.0) / 3.0).max(1.0);
                let th = ((h - margin * 3.0) / 2.0).max(1.0);
                (margin + (slot % 3) as f64 * (tw + margin),
                 margin + (slot / 3) as f64 * (th + margin), tw, th)
            } else {
                let tw = (w * 0.29).min(h * 0.49);
                let th = tw * 0.79;
                let cx = w * (0.18 + 0.32 * (slot % 3) as f64)
                    + (slot as f64 * 13.0).sin() * w * 0.012;
                let cy = h * (0.27 + 0.44 * (slot / 3) as f64)
                    + (slot as f64 * 7.0).cos() * h * 0.013;
                (cx - tw * 0.5, cy - th * 0.5, tw, th)
            };
            let iw = tile_w.round() as i32;
            let ih = tile_h.round() as i32;
            if tile.frame.width_request() != iw || tile.frame.height_request() != ih {
                tile.frame.set_size_request(iw, ih);
            }
            let (old_x, old_y) = self.root.child_position(&tile.frame);
            if (old_x - x).abs() > 0.25 || (old_y - y).abs() > 0.25 {
                self.root.move_(&tile.frame, x, y);
            }
            let old = player.fading_slot
                .and_then(|(change_slot, old)| (change_slot == slot).then_some(old));
            tile.sync(new_id, old, fade, player);
        }
    }
}

pub(super) fn install_css() {
    if let Some(display) = gtk::gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(r#"
          .screensaver-stage { background-color: #090b10; }
          .screensaver-cinema-frame { background-color: #090b10; }
          .screensaver-card { background-color: #171b21; }
          .screensaver-print {
            background-color: #f6f4ee;
            padding: 7px;
            box-shadow: 0 8px 24px rgba(0,0,0,0.5);
          }
          .screensaver-angle-0 { transform: rotate(-5deg); }
          .screensaver-angle-1 { transform: rotate(3deg); }
          .screensaver-angle-2 { transform: rotate(-3deg); }
          .screensaver-angle-3 { transform: rotate(5deg); }
          .screensaver-angle-4 { transform: rotate(-4deg); }
          .screensaver-angle-5 { transform: rotate(4deg); }
        "#);
        gtk::style_context_add_provider_for_display(
            &display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
