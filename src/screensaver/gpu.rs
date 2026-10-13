//! GTK4/GSK composited screensaver stage.
//! All photos are decoded on a worker into RGBA8. The UI retains GdkTexture
//! paintables and animates opacity/geometry, never repainting pixels in Cairo.
use super::*;

// 60 steps with per-frame CSS transform interpolation gives a smooth
// GSK-composited flip without creating new thumbnails or touching the layout.
const FLIP_STEPS: usize = 60;
const KEN_STEPS: usize = 240;
const FLOAT_STEPS: usize = 120;

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

/// Card-layer lifecycle: the visible base is always the stable current
/// image, while the incoming texture exists only during Crossfade.
struct Tile {
    frame: gtk::Overlay,
    content: gtk::Overlay,
    back: gtk::Picture,
    front: gtk::Picture,
    back_version: Option<(usize, u64)>,
    front_version: Option<(usize, u64)>,
    flip_step: Option<usize>,
    drift_step: Option<(usize, usize)>,
}
impl Tile {
    fn new(square_mosaic: bool) -> Self {
        let frame = gtk::Overlay::new();
        frame.set_overflow(gtk::Overflow::Hidden);
        let content = gtk::Overlay::new();
        content.set_hexpand(true);
        content.set_vexpand(true);
        let back = gtk::Picture::new();
        let front = gtk::Picture::new();
        for picture in [&back, &front] {
            picture.set_content_fit(if square_mosaic {
                gtk::ContentFit::Cover
            } else {
                gtk::ContentFit::Contain
            });
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            picture.set_can_shrink(true);
        }
        content.set_child(Some(&back));
        content.add_overlay(&front);
        frame.set_child(Some(&content));
        front.set_opacity(0.0);
        frame.add_css_class("screensaver-card");
        Self {
            frame, content, back, front,
            back_version: None, front_version: None, flip_step: None,
            drift_step: None,
        }
    }
    fn clear_incoming(&mut self) {
        self.front.set_opacity(0.0);
        if self.front_version.take().is_some() {
            self.front.set_paintable(None::<&gtk::gdk::Paintable>);
        }
    }
    fn set_flip_step(&mut self, step: Option<usize>) {
        if self.flip_step == step { return; }
        if let Some(old) = self.flip_step {
            self.content.remove_css_class(&format!("screensaver-flip-{old}"));
        }
        if let Some(new) = step {
            self.content.add_css_class(&format!("screensaver-flip-{new}"));
        }
        self.flip_step = step;
    }
    fn drift(&mut self, slot: usize, phase: f64, floating: bool) {
        let direction = slot % 6;
        let step = (phase.clamp(0.0, 1.0) * FLOAT_STEPS as f64).round() as usize;
        if self.drift_step == Some((direction, step)) { return; }
        if let Some((old_dir, old_step)) = self.drift_step {
            self.frame.remove_css_class(&format!("screensaver-float-{old_dir}-{old_step}"));
        }
        // Only Floating Prints drift. A resting scrapbook stays legible.
        if floating {
            self.frame.add_css_class(&format!("screensaver-float-{direction}-{step}"));
            self.drift_step = Some((direction, step));
        }
    }
    fn sync(&mut self, new_id: usize, old: Option<usize>, progress: f64,
        player: &Player, transition: PicasaTransition)
    {
        let fade = progress.clamp(0.0, 1.0);
        let changing = old.filter(|old| *old != new_id && fade < 1.0);
        match (transition, changing) {
            (PicasaTransition::Crossfade, Some(outgoing)) => {
                self.set_flip_step(None);
                // Pin the outgoing texture on the lower layer. Both layers
                // remain alive until the new photo is fully opaque.
                let old_ready = self.back_version.is_some_and(|(id, _)| id == outgoing)
                    || present(&self.back, &mut self.back_version, outgoing, player);
                if old_ready && present(&self.front, &mut self.front_version, new_id, player) {
                    self.front.set_opacity(fade);
                    return;
                }
            }
            (PicasaTransition::Flip, Some(outgoing)) => {
                self.clear_incoming();
                // A 180-degree card turn, seen from its narrow edge halfway.
                // The image changes only while edge-on, never on an open card.
                let showing = if fade < 0.5 { outgoing } else { new_id };
                let valid = self.back_version.is_some_and(|(id, _)| id == showing)
                    || present(&self.back, &mut self.back_version, showing, player);
                if valid {
                    let step = (fade * FLIP_STEPS as f64).round() as usize;
                    self.set_flip_step(Some(step.min(FLIP_STEPS)));
                    return;
                }
            }
            _ => {}
        }
        // Cut: replace the base texture immediately when Player advances.
        // Fade/flip: only commit it after the transition has finished.
        if present(&self.back, &mut self.back_version, new_id, player) {
            self.clear_incoming();
            self.set_flip_step(None);
        }
    }
}

/// Cinematic player uses two stable, full-viewport frames. Picture content
/// moves only through GPU compositor CSS transforms, not GtkFixed geometry.
struct Cinema {
    back_frame: gtk::Overlay,
    front_frame: gtk::Overlay,
    back: gtk::Picture,
    front: gtk::Picture,
    back_version: Option<(usize, u64)>,
    front_version: Option<(usize, u64)>,
    back_motion_step: Option<(usize, usize)>,
    front_motion_step: Option<(usize, usize)>,
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
        for frame in [&back_frame, &front_frame] {
            frame.add_css_class("screensaver-cinema-frame");
            frame.set_overflow(gtk::Overflow::Hidden);
        }
        back_frame.set_child(Some(&back));
        front_frame.set_child(Some(&front));
        front_frame.set_opacity(0.0);
        root.put(&back_frame, 0.0, 0.0);
        root.put(&front_frame, 0.0, 0.0);
        Self {
            back_frame, front_frame, back, front,
            back_version: None, front_version: None,
            back_motion_step: None, front_motion_step: None,
        }
    }
    fn stable_viewport(root: &gtk::Fixed, frame: &gtk::Overlay, w: f64, h: f64) {
        // Do this only when the monitor resolution itself changes; never
        // resize a photo container as part of the Ken Burns motion.
        let iw = w.round() as i32;
        let ih = h.round() as i32;
        if frame.width_request() != iw || frame.height_request() != ih {
            frame.set_size_request(iw, ih);
        }
        let (old_x, old_y) = root.child_position(frame);
        if old_x.abs() > 0.25 || old_y.abs() > 0.25 {
            root.move_(frame, 0.0, 0.0);
        }
    }
    fn motion(picture: &gtk::Picture, previous: &mut Option<(usize, usize)>,
        fraction: f64, photo_id: usize)
    {
        let step = (fraction.clamp(0.0, 1.0) * KEN_STEPS as f64).round() as usize;
        let direction = photo_id % 4;
        if *previous == Some((direction, step)) { return; }
        if let Some((old_dir, old_step)) = previous {
            picture.remove_css_class(&format!("screensaver-ken-{old_dir}-{old_step}"));
        }
        picture.add_css_class(&format!("screensaver-ken-{direction}-{step}"));
        *previous = Some((direction, step));
    }
    fn sync(&mut self, root: &gtk::Fixed, player: &Player, w: f64, h: f64, ken_burns: bool) {
        Self::stable_viewport(root, &self.back_frame, w, h);
        Self::stable_viewport(root, &self.front_frame, w, h);

        let elapsed = player.elapsed().as_secs_f64();
        let duration = player.interval.as_secs_f64().max(1.0);
        let current_fraction = (elapsed / duration).clamp(0.0, 1.0);
        let fade = smootherstep(player.progress());
        let outgoing = player.previous.filter(|old| *old != player.current && fade < 1.0);
        if let Some(old) = outgoing {
            let ready = self.back_version.is_some_and(|(id, _)| id == old)
                || present(&self.back, &mut self.back_version, old, player);
            if ready && present(&self.front, &mut self.front_version, player.current, player) {
                // The old image holds its ending position until it disappears.
                // The new image moves continuously from its starting position.
                if ken_burns {
                    Self::motion(&self.back, &mut self.back_motion_step, 1.0, old);
                    Self::motion(&self.front, &mut self.front_motion_step, current_fraction, player.current);
                }
                self.front_frame.set_opacity(fade);
                return;
            }
        }
        // At fade completion, promote an identical current texture and its
        // identical transform before removing the outgoing overlay.
        if present(&self.back, &mut self.back_version, player.current, player) {
            if ken_burns {
                Self::motion(&self.back, &mut self.back_motion_step, current_fraction, player.current);
            }
            self.front_frame.set_opacity(0.0);
            if self.front_version.take().is_some() {
                self.front.set_paintable(None::<&gtk::gdk::Paintable>);
            }
        }
    }
}

/// Fit N square photos onto the current fullscreen monitor, using as much
/// space as possible without ever stretching a tile. Incomplete last rows
/// are centred to avoid a lopsided block.
#[derive(Debug, Clone, Copy)]
struct SquareGrid {
    columns: usize,
    side: f64,
    gap: f64,
    top: f64,
    width: f64,
}
impl SquareGrid {
    fn fit(width: f64, height: f64, count: usize) -> Option<Self> {
        if count == 0 || width < 2.0 || height < 2.0 { return None; }
        // At high tile counts the gutters must shrink rather than consuming
        // all of the viewport before we have enough rows/columns.
        let gap = (width.min(height) * 0.012)
            .min(18.0)
            .min(width.min(height) / ((count as f64).sqrt() * 4.0));
        let mut best: Option<(Self, usize)> = None;
        // Near-square tiles reach maximum size with a column count close to
        // sqrt(number_of_tiles * monitor_aspect_ratio). Checking a small
        // neighbourhood is effectively O(1) even for 100,000+ tiles, unlike
        // the old 1..count exhaustive layout search on every frame.
        let ideal = ((count as f64 * width / height).sqrt() as usize).clamp(1, count);
        let from = ideal.saturating_sub(12).max(1);
        let to = ideal.saturating_add(12).min(count);
        for cols in from..=to {
            let rows = count.div_ceil(cols);
            let side = ((width - (cols + 1) as f64 * gap) / cols as f64)
                .min((height - (rows + 1) as f64 * gap) / rows as f64);
            if side <= 0.0 { continue; }
            let empty = cols * rows - count;
            let candidate = Self {
                columns: cols,
                side,
                gap,
                top: (height - (rows as f64 * side + (rows - 1) as f64 * gap)) * 0.5,
                width,
            };
            let better = best.as_ref().is_none_or(|(previous, previous_empty)| {
                side > previous.side + 0.5
                    || ((side - previous.side).abs() <= 0.5 && empty < *previous_empty)
            });
            if better { best = Some((candidate, empty)); }
        }
        best.map(|(layout, _)| layout)
    }
    fn position(&self, slot: usize, count: usize) -> (f64, f64) {
        let row = slot / self.columns;
        let col = slot % self.columns;
        let in_row = (count - row * self.columns).min(self.columns);
        let row_width = in_row as f64 * self.side
            + in_row.saturating_sub(1) as f64 * self.gap;
        let left = (self.width - row_width) * 0.5;
        (
            left + col as f64 * (self.side + self.gap),
            self.top + row as f64 * (self.side + self.gap),
        )
    }
}

/// Full-screen changing grid: rectangular cover tiles, no grey gutters or
/// blank last row when the collection size does not fit an exact rectangle.
#[derive(Clone, Copy, Debug)]
struct ChangingGrid {
    cols: usize,
    rows: usize,
    gap: f64,
}
impl ChangingGrid {
    fn fit(width: f64, height: f64, count: usize) -> Self {
        let cols = ((count.max(1) as f64 * width / height).sqrt().ceil() as usize)
            .clamp(1, count.max(1));
        let rows = count.max(1).div_ceil(cols);
        Self { cols, rows, gap: (width.min(height) * 0.004).clamp(2.0, 9.0) }
    }
    fn rect(self, slot: usize, count: usize, width: f64, height: f64)
        -> (f64, f64, f64, f64)
    {
        let row = slot / self.cols;
        let remaining = count.saturating_sub(row * self.cols);
        let cols_in_row = remaining.min(self.cols).max(1);
        let cell_w = width / cols_in_row as f64;
        let cell_h = height / self.rows as f64;
        (
            slot.saturating_sub(row * self.cols) as f64 * cell_w + self.gap,
            row as f64 * cell_h + self.gap,
            (cell_w - 2.0 * self.gap).max(1.0),
            (cell_h - 2.0 * self.gap).max(1.0),
        )
    }
}

/// Polaroid/Scrapbook or Floating Prints layouts. All frames stay safely
/// inside the output after rotation. Overlaps are intentional for the pile.
fn print_rect(style: Style, slot: usize, width: f64, height: f64) -> (f64, f64, f64, f64) {
    let landscape = width >= height;
    let (cx, cy) = match style {
        Style::PicturePile if landscape => {
            [(0.34, 0.34), (0.65, 0.36), (0.38, 0.65),
             (0.64, 0.67), (0.50, 0.50)][slot % 5]
        }
        Style::PicturePile => {
            [(0.38, 0.28), (0.62, 0.32), (0.36, 0.62),
             (0.65, 0.66), (0.51, 0.49)][slot % 5]
        }
        _ if landscape => {
            [(0.19, 0.27), (0.50, 0.25), (0.81, 0.29),
             (0.20, 0.73), (0.51, 0.75), (0.81, 0.71)][slot % 6]
        }
        _ => {
            [(0.29, 0.18), (0.71, 0.19), (0.29, 0.50),
             (0.71, 0.51), (0.29, 0.82), (0.71, 0.81)][slot % 6]
        }
    };
    let tile_w = if style == Style::PicturePile {
        (width * if landscape { 0.35 } else { 0.57 }).min(height * 0.53)
    } else {
        (width * if landscape { 0.28 } else { 0.41 }).min(height * 0.39)
    };
    let tile_w = tile_w.max(40.0);
    let tile_h = tile_w * 0.77;
    let margin = 18.0_f64.min(width.min(height) * 0.03);
    let x = (width * cx - tile_w * 0.5).clamp(margin, (width - tile_w - margin).max(margin));
    let y = (height * cy - tile_h * 0.5).clamp(margin, (height - tile_h - margin).max(margin));
    (x, y, tile_w, tile_h)
}

pub(super) struct Stage {
    pub root: gtk::Fixed,
    style: Style,
    transition: PicasaTransition,
    tiles: Vec<Tile>,
    desired_tiles: usize,
    cinema: Option<Cinema>,
    trace: bool,
    traced_change: Option<usize>,
    traced_settle: Option<usize>,
    last_tick: Option<Instant>,
    synced_revision: Option<u64>,
    synced_geometry: Option<(i32, i32, usize)>,
    active_tile: Option<usize>,
    synced_change: Option<usize>,
}
impl Stage {
    pub fn new(style: Style, count: usize, transition: PicasaTransition) -> Self {
        let root = gtk::Fixed::new();
        root.set_hexpand(true);
        root.set_vexpand(true);
        // Clip nested images/rotated prints to this monitor's window bounds.
        root.set_overflow(gtk::Overflow::Hidden);
        root.add_css_class("screensaver-stage");
        let cinema = if style.single_photo() {
            Some(Cinema::new(&root))
        } else { None };
        // Avoid blocking GTK with the creation of thousands of overlays
        // before the fullscreen surface appears. Additional square tiles are
        // materialised in small frame-clock batches.
        let desired_tiles = if style.single_photo() { 0 } else { count };
        let trace = std::env::var("PIC_SCREENSAVER_TRACE").is_ok_and(|v| v == "1");
        if trace {
            eprintln!("PIC_SCREEN init style={style:?} transition={transition:?} count={count}");
        }
        Self {
            root, style, transition, tiles: Vec::new(), desired_tiles, cinema,
            trace, traced_change: None, traced_settle: None, last_tick: None,
            synced_revision: None, synced_geometry: None,
            active_tile: None, synced_change: None,
        }
    }
    fn add_tile(&mut self) {
        let slot = self.tiles.len();
        if slot >= self.desired_tiles { return; }
        let tile = Tile::new(matches!(self.style, Style::Mosaic | Style::Grid));
        if matches!(self.style, Style::PicturePile | Style::Floating) {
            tile.frame.add_css_class("screensaver-print");
            tile.frame.add_css_class(&format!("screensaver-angle-{slot}"));
        }
        self.root.put(&tile.frame, 0.0, 0.0);
        self.tiles.push(tile);
    }
    pub fn sync(&mut self, player: &Player) {
        if self.trace {
            let now = Instant::now();
            if let Some(previous) = self.last_tick {
                let ms = now.duration_since(previous).as_millis();
                if ms >= 90 {
                    eprintln!(
                        "PIC_SCREEN frame_spike_ms={ms} style={:?} tiles={} loaded={} change={}",
                        self.style, self.tiles.len(), player.frames.len(), player.changes,
                    );
                }
            }
            self.last_tick = Some(now);
            if self.traced_change != Some(player.changes) {
                self.traced_change = Some(player.changes);
                eprintln!(
                    "PIC_SCREEN phase=start style={:?} mode={:?} current={} previous={:?} replacing={:?} frames={} change={}",
                    self.style, self.transition, player.current, player.previous,
                    player.fading_slot, player.frames.len(), player.changes,
                );
            }
            if player.progress() >= 1.0 && self.traced_settle != Some(player.changes) {
                self.traced_settle = Some(player.changes);
                eprintln!(
                    "PIC_SCREEN phase=settled style={:?} current={} old_image={:?} old_tile={:?} frames={} change={}",
                    self.style, player.current, player.previous,
                    player.fading_slot, player.frames.len(), player.changes,
                );
            }
        }
        let w = self.root.width() as f64;
        let h = self.root.height() as f64;
        if w < 2.0 || h < 2.0 { return; }
        if let Some(cinema) = self.cinema.as_mut() {
            cinema.sync(&self.root, player, w, h, self.style == Style::IPhoto);
            return;
        }
        let pending = self.desired_tiles.saturating_sub(self.tiles.len());
        for _ in 0..pending.min(48) {
            self.add_tile();
        }
        let fade = smootherstep(player.progress());
        let tile_count = self.desired_tiles;
        let geometry = (w.round() as i32, h.round() as i32, self.tiles.len());
        let full_refresh = self.synced_revision != Some(player.next_revision)
            || self.synced_geometry != Some(geometry);
        let active_tile = player.fading_slot.map(|(slot, _)| slot);
        let previous_active = self.active_tile;
        let change_pending = self.synced_change != Some(player.changes);
        if !full_refresh && !change_pending
            && active_tile.is_none() && previous_active.is_none()
        {
            self.animate_floating(player);
            return;
        }
        self.synced_revision = Some(player.next_revision);
        self.synced_geometry = Some(geometry);
        self.synced_change = Some(player.changes);
        self.active_tile = active_tile;
        let square_grid = if full_refresh && self.style == Style::Mosaic {
            SquareGrid::fit(w, h, tile_count)
        } else { None };
        let changing_grid = if full_refresh && self.style == Style::Grid {
            Some(ChangingGrid::fit(w, h, tile_count))
        } else { None };
        for (slot, tile) in self.tiles.iter_mut().enumerate() {
            if !full_refresh && active_tile != Some(slot) && previous_active != Some(slot) {
                continue; // Only the changing tile needs an opacity update.
            }
            let Some(&new_id) = player.slots.get(slot) else { continue };
            if full_refresh {
            let (x, y, tile_w, tile_h) = if let Some(grid) = square_grid {
                let (x, y) = grid.position(slot, tile_count);
                (x, y, grid.side, grid.side)
            } else if let Some(grid) = changing_grid {
                grid.rect(slot, tile_count, w, h)
            } else {
                print_rect(self.style, slot, w, h)
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
            }
            let old = player.fading_slot
                .and_then(|(change_slot, old)| (change_slot == slot).then_some(old));
            tile.sync(new_id, old, fade, player, self.transition);
            if change_pending && active_tile == Some(slot)
                && self.style == Style::PicturePile
            {
                // Move the newly replaced print to the top of the stack.
                // Without this, lower cards remain invisible behind others.
                let (x, y) = self.root.child_position(&tile.frame);
                self.root.remove(&tile.frame);
                self.root.put(&tile.frame, x, y);
            }
        }
        self.animate_floating(player);
    }
    fn animate_floating(&mut self, player: &Player) {
        if self.style != Style::Floating { return; }
        let seconds = player.interval.as_secs_f64().max(1.0);
        let time = player.elapsed().as_secs_f64();
        for (index, tile) in self.tiles.iter_mut().enumerate() {
            let phase = ((time / seconds) + index as f64 / 6.0).fract();
            tile.drift(index, phase, true);
        }
    }
}

pub(super) fn install_css() {
    if let Some(display) = gtk::gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        let mut css = String::from(r#"
          .screensaver-stage { background-color: #090b10; }
          .screensaver-cinema-frame { background-color: #090b10; }
          .screensaver-card { background-color: #171b21; }
          .screensaver-print {
            background-color: #f6f4ee;
            padding: 9px 9px 20px 9px;
            box-shadow: 0 10px 25px rgba(0,0,0,0.48);
          }
          .screensaver-caption {
            background-color: rgba(0,0,0,0.64);
            color: white;
            padding: 8px 16px;
            border-radius: 12px;
            font-size: 13px;
          }
          .screensaver-clock {
            color: white;
            background-color: rgba(9,11,16,0.62);
            padding: 7px 13px;
            border-radius: 12px;
            font-size: 22px;
            font-weight: 600;
          }
          .screensaver-angle-0 { transform: rotate(-5deg); }
          .screensaver-angle-1 { transform: rotate(3deg); }
          .screensaver-angle-2 { transform: rotate(-3deg); }
          .screensaver-angle-3 { transform: rotate(5deg); }
          .screensaver-angle-4 { transform: rotate(-4deg); }
          .screensaver-angle-5 { transform: rotate(4deg); }
          .screensaver-card > overlay { transform-origin: 50% 50%; }
        "#);
        // Precompile quantised compositor transforms once, never reparse
        // a CSS stylesheet in the frame clock.
        for step in 0..=FLIP_STEPS {
            let factor = PicasaTransition::Flip.flip_width(step as f64 / FLIP_STEPS as f64);
            css.push_str(&format!(
                ".screensaver-flip-{step} {{ transform: scaleX({factor:.6}); }}\n"
            ));
        }
        // Each floating print oscillates gently around its fixed scrapbook
        // position. All transform values are CSS pixels, not percentages.
        let tilt = [-5.0, 3.0, -3.0, 5.0, -4.0, 4.0];
        for direction in 0..6 {
            for step in 0..=FLOAT_STEPS {
                let t = step as f64 / FLOAT_STEPS as f64;
                let x = 9.0 * (std::f64::consts::TAU * t).sin();
                let y = 7.0 * (std::f64::consts::TAU * t).cos();
                let angle = tilt[direction] + 1.0 * (std::f64::consts::TAU * t).sin();
                css.push_str(&format!(
                    ".screensaver-float-{direction}-{step} {{ transform-origin: 50% 50%; transform: translate({x:.3}px, {y:.3}px) rotate({angle:.3}deg); }}\n"
                ));
            }
        }
        // GTK4's CSS transform parser rejects percentage translate() values.
        // Use compositor-only pixel pans and a smooth push-in. The fixed
        // fullscreen frames never relayout as individual photos move.
        for direction in 0..4 {
            for step in 0..=KEN_STEPS {
                let fraction = step as f64 / KEN_STEPS as f64;
                let (scale, x, y) = cinematic_motion(direction, fraction);
                css.push_str(&format!(
                    ".screensaver-ken-{direction}-{step} {{ transform-origin: 50% 50%; transform: translate({x:.3}px, {y:.3}px) scale({scale:.6}); }}\n"
                ));
            }
        }
        provider.load_from_string(&css);
        gtk::style_context_add_provider_for_display(
            &display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_bounds_all_counts_and_aspects() {
        for (w,h) in [(1920.,1080.), (1080.,1920.),(3840.,2160.)] {
            for count in [1,2,3,4,6,12,24,100] {
                let grid = ChangingGrid::fit(w,h,count);
                for slot in 0..count {
                    let (x,y,tw,th) = grid.rect(slot,count,w,h);
                    assert!(x>=0. && y>=0. && tw>0. && th>0.);
                    assert!(x+tw<=w+0.001 && y+th<=h+0.001);
                }
            }
        }
    }

    #[test]
    fn print_cards_stay_on_screen() {
        for style in [Style::PicturePile,Style::Floating] {
            for (w,h) in [(1920.,1080.),(1080.,1920.),(1024.,768.)] {
                for i in 0..6 {
                    let (x,y,tw,th)=print_rect(style,i,w,h);
                    assert!(x>=0. && y>=0. && tw>0. && th>0.);
                    assert!(x+tw<=w && y+th<=h, "{style:?} {w} {h} {i}");
                }
            }
        }
    }
    #[test]
    fn mosaic_tiles_are_square_centered_and_within_monitor_bounds() {
        for (width, height) in [(1920.0, 1080.0), (3840.0, 2160.0), (1080.0, 1920.0)] {
            for count in [1, 3, 6, 9, 12, 18, 24, 30, 100, 1000, 10000] {
                let grid = SquareGrid::fit(width, height, count).unwrap();
                assert!(grid.side > 0.0);
                for slot in 0..count {
                    let (x, y) = grid.position(slot, count);
                    assert!(x >= -0.01 && y >= -0.01, "{width}x{height} {count} {slot}: {x}, {y}");
                    assert!(x + grid.side <= width + 0.01);
                    assert!(y + grid.side <= height + 0.01);
                }
            }
        }
    }

    #[test]
    fn classic_six_tiles_use_three_columns_two_rows_in_landscape() {
        let grid = SquareGrid::fit(1920.0, 1080.0, 6).unwrap();
        assert_eq!(grid.columns, 3);
        assert_eq!(6_usize.div_ceil(grid.columns), 2);
        let (first_x, first_y) = grid.position(0, 6);
        let (last_x, last_y) = grid.position(5, 6);
        assert!(first_x < last_x && first_y < last_y);
    }
}

