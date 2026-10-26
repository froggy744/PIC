//! GTK4/GSK composited screensaver stage.
//! All photos are decoded on a worker into RGBA8. The UI retains GdkTexture
//! paintables and animates opacity/geometry, never repainting pixels in Cairo.
use super::*;

mod pile;
use pile::Pile;
mod cinema;
use cinema::Cinema;
mod coverflow;
use coverflow::CoverFlow;

// 60 steps with per-frame CSS transform interpolation gives a smooth
// GSK-composited flip without creating new thumbnails or touching the layout.
const FLIP_STEPS: usize = 60;
const FLOATING_TILT: [f64; 6] = [-0.8, 0.6, -0.6, 0.8, -0.5, 0.5];
const FLOATING_DRIFT_X: f64 = 3.0;
const FLOATING_DRIFT_Y: f64 = 2.5;
const FLOATING_TILT_SWAY: f64 = 0.25;

fn base_tilt(style: Style, slot: usize) -> f64 {
    match style {
        Style::PicturePile => 0.0,
        Style::Floating => FLOATING_TILT[slot % FLOATING_TILT.len()],
        _ => 0.0,
    }
}

fn present(
    picture: &gtk::Picture,
    slot: &mut Option<(usize, u64)>,
    index: usize,
    player: &Player,
) -> bool {
    let Some(image) = player.frames.get(&index) else {
        return false;
    };
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
}
impl Tile {
    fn new(cover_photo: bool) -> Self {
        let frame = gtk::Overlay::new();
        frame.set_overflow(gtk::Overflow::Hidden);
        let content = gtk::Overlay::new();
        content.set_hexpand(true);
        content.set_vexpand(true);
        let back = gtk::Picture::new();
        let front = gtk::Picture::new();
        for picture in [&back, &front] {
            picture.set_content_fit(if cover_photo {
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
            frame,
            content,
            back,
            front,
            back_version: None,
            front_version: None,
            flip_step: None,
        }
    }
    fn clear_incoming(&mut self) {
        self.front.set_opacity(0.0);
        if self.front_version.take().is_some() {
            self.front.set_paintable(None::<&gtk::gdk::Paintable>);
        }
    }
    fn set_flip_step(&mut self, step: Option<usize>) {
        if self.flip_step == step {
            return;
        }
        if let Some(old) = self.flip_step {
            self.content
                .remove_css_class(&format!("screensaver-flip-{old}"));
        }
        if let Some(new) = step {
            self.content
                .add_css_class(&format!("screensaver-flip-{new}"));
        }
        self.flip_step = step;
    }
    fn sync(
        &mut self,
        new_id: usize,
        old: Option<usize>,
        progress: f64,
        player: &Player,
        transition: PicasaTransition,
    ) {
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
        if count == 0 || width < 2.0 || height < 2.0 {
            return None;
        }
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
            if side <= 0.0 {
                continue;
            }
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
            if better {
                best = Some((candidate, empty));
            }
        }
        best.map(|(layout, _)| layout)
    }
    fn position(&self, slot: usize, count: usize) -> (f64, f64) {
        let row = slot / self.columns;
        let col = slot % self.columns;
        let in_row = (count - row * self.columns).min(self.columns);
        let row_width = in_row as f64 * self.side + in_row.saturating_sub(1) as f64 * self.gap;
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
        let count = count.max(1);
        // Minimise empty positions and keep a photo-like cell aspect ratio.
        // Do not expand the last few photos into giant tiles.
        let ideal =
            ((count as f64 * width / (height * 1.4)).sqrt().round() as usize).clamp(1, count);
        let from = ideal.saturating_sub(16).max(1);
        let to = ideal.saturating_add(16).min(count);
        let mut best = (f64::INFINITY, ideal, count.div_ceil(ideal));
        for cols in from..=to {
            let rows = count.div_ceil(cols);
            let aspect = (width / cols as f64) / (height / rows as f64);
            let empty = cols * rows - count;
            let penalty = (empty as f64 / count as f64) * 8.0 + (aspect / 1.4).ln().abs() * 0.5;
            if penalty < best.0 {
                best = (penalty, cols, rows);
            }
        }
        Self {
            cols: best.1,
            rows: best.2,
            gap: (width.min(height) * 0.004).clamp(2.0, 9.0),
        }
    }
    fn rect(self, slot: usize, count: usize, width: f64, height: f64) -> (f64, f64, f64, f64) {
        let row = slot / self.cols;
        let remaining = count.saturating_sub(row * self.cols);
        let cols_in_row = remaining.min(self.cols).max(1);
        let cell_w = width / self.cols as f64;
        let cell_h = height / self.rows as f64;
        let left = (width - cols_in_row as f64 * cell_w) * 0.5;
        (
            left + (slot % self.cols) as f64 * cell_w + self.gap,
            row as f64 * cell_h + self.gap,
            (cell_w - 2.0 * self.gap).max(1.0),
            (cell_h - 2.0 * self.gap).max(1.0),
        )
    }
}

/// Layout for any number of prints beyond the hand-placed ones: a grid chosen
/// to match the screen, every row centred, each print fitted into its cell by
/// its own aspect ratio. Picture Pile prints are slightly irregular and
/// overlap a little like a scrapbook; Floating Prints keep clear gaps.
fn scatter_rect(
    style: Style,
    slot: usize,
    count: usize,
    image_aspect: f64,
    width: f64,
    height: f64,
) -> (f64, f64, f64, f64) {
    let ratio = image_aspect.clamp(0.60, 2.0);
    let margin = (width.min(height) * 0.035).max(16.0);
    let (area_w, area_h) = (
        (width - 2.0 * margin).max(1.0),
        (height - 2.0 * margin).max(1.0),
    );
    let ideal = ((count as f64 * area_w / area_h).sqrt().round() as usize).clamp(1, count);
    let mut best = (f64::INFINITY, ideal, count.div_ceil(ideal));
    for cols in ideal.saturating_sub(3).max(1)..=(ideal + 3).min(count) {
        let rows = count.div_ceil(cols);
        let cell_aspect = (area_w / cols as f64) / (area_h / rows as f64);
        let empty = (cols * rows - count) as f64 / count as f64;
        let penalty = empty * 6.0 + (cell_aspect / 1.4).ln().abs();
        if penalty < best.0 {
            best = (penalty, cols, rows);
        }
    }
    let (_, cols, rows) = best;
    let (row, col) = (slot / cols, slot % cols);
    let in_row = (count - row * cols).min(cols);
    let (cell_w, cell_h) = (area_w / cols as f64, area_h / rows as f64);
    let left = margin + (area_w - in_row as f64 * cell_w) * 0.5;
    let pile = style == Style::PicturePile;
    let fill = if pile { 0.96 } else { 0.84 };
    let hash = (slot as u64)
        .wrapping_add(1)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let unit = |shift: u32| ((hash >> shift) % 1000) as f64 / 1000.0 - 0.5;
    let wiggle = if pile { 0.10 } else { 0.0 };
    let cx = left + (col as f64 + 0.5) * cell_w + unit(20) * wiggle * cell_w;
    let cy = margin + (row as f64 + 0.5) * cell_h + unit(40) * wiggle * cell_h;
    let tile_w = (cell_w * fill).min(cell_h * fill * ratio);
    let tile_h = tile_w / ratio;
    let x = (cx - tile_w * 0.5).clamp(margin, (width - tile_w - margin).max(margin));
    let y = (cy - tile_h * 0.5).clamp(margin, (height - tile_h - margin).max(margin));
    (x, y, tile_w, tile_h)
}

/// Return a postcard size that follows the image orientation, not a
/// landscape-only rectangle. Oversized white margins with portrait photos
/// were caused by fitting a tall image in a fixed wide GTK Picture.
fn print_rect(
    style: Style,
    slot: usize,
    count: usize,
    image_aspect: f64,
    width: f64,
    height: f64,
) -> (f64, f64, f64, f64) {
    if count
        > match style {
            Style::PicturePile => PICTURE_PILE_SLOTS,
            Style::Floating => FLOATING_SLOTS,
            _ => usize::MAX,
        }
    {
        return scatter_rect(style, slot, count, image_aspect, width, height);
    }
    let landscape = width >= height;
    let ratio = image_aspect.clamp(0.60, 2.0);
    let (cx, cy) = if count <= 1 {
        (0.5, 0.50)
    } else if count == 2 {
        if landscape {
            [(0.33, 0.50), (0.67, 0.50)][slot % 2]
        } else {
            [(0.50, 0.30), (0.50, 0.70)][slot % 2]
        }
    } else if style == Style::PicturePile {
        if count == 3 {
            if landscape {
                [(0.29, 0.48), (0.71, 0.48), (0.50, 0.55)][slot % 3]
            } else {
                [(0.35, 0.34), (0.65, 0.36), (0.50, 0.70)][slot % 3]
            }
        } else if count == 4 {
            [(0.32, 0.30), (0.68, 0.32), (0.33, 0.71), (0.68, 0.70)][slot % 4]
        } else {
            // Four staggered photographs, plus a smaller centre print.
            // Keep a little physical overlap while leaving each photo legible.
            [
                (0.29, 0.30),
                (0.71, 0.30),
                (0.29, 0.72),
                (0.71, 0.72),
                (0.50, 0.52),
            ][slot % 5]
        }
    } else if count == 3 {
        if landscape {
            [(0.18, 0.50), (0.50, 0.50), (0.82, 0.50)][slot % 3]
        } else {
            [(0.50, 0.19), (0.50, 0.50), (0.50, 0.81)][slot % 3]
        }
    } else if count == 4 {
        [(0.27, 0.27), (0.73, 0.27), (0.27, 0.73), (0.73, 0.73)][slot % 4]
    } else if landscape {
        // Floating Prints uses a clean 3x2 arrangement, not the
        // overlapping Polaroid stack.
        [
            (0.18, 0.28),
            (0.50, 0.28),
            (0.82, 0.28),
            (0.18, 0.72),
            (0.50, 0.72),
            (0.82, 0.72),
        ][slot % 6]
    } else {
        [
            (0.28, 0.18),
            (0.72, 0.18),
            (0.28, 0.50),
            (0.72, 0.50),
            (0.28, 0.82),
            (0.72, 0.82),
        ][slot % 6]
    };

    let pile = style == Style::PicturePile;
    let base_w = if pile {
        width * if landscape { 0.27 } else { 0.43 }
    } else if style == Style::Floating {
        width * if landscape { 0.24 } else { 0.35 }
    } else {
        width * if landscape { 0.26 } else { 0.39 }
    };
    let max_h = height
        * if pile {
            0.34
        } else if style == Style::Floating && landscape {
            0.30
        } else if landscape {
            0.34
        } else {
            0.25
        };
    let mut tile_w = base_w.min(max_h * ratio);
    let mut tile_h = tile_w / ratio;
    if pile && slot == 4 && count >= 5 {
        tile_w *= 0.72;
        tile_h *= 0.72;
    }
    // Allow for rotated corners, gentle floating transforms and shadows.
    let margin = (width.min(height) * 0.035).max(16.0);
    tile_w = tile_w.min((width - 2.0 * margin).max(1.0));
    tile_h = tile_h.min((height - 2.0 * margin).max(1.0));
    let x = (width * cx - tile_w * 0.5).clamp(margin, (width - tile_w - margin).max(margin));
    let y = (height * cy - tile_h * 0.5).clamp(margin, (height - tile_h - margin).max(margin));
    (x, y, tile_w, tile_h)
}

/// Prefer the cached preview's native dimensions until the HQ original
/// arrives. Both maintain the photo orientation; a texture replacement
/// cannot force every photograph into a landscape-only picture widget.
fn photo_aspect(player: &Player, index: usize) -> f64 {
    player
        .frames
        .get(&index)
        .map(|image| {
            let h = image.texture.height().max(1) as f64;
            image.texture.width().max(1) as f64 / h
        })
        .unwrap_or(1.5)
}

pub(super) struct Stage {
    pub root: gtk::Fixed,
    style: Style,
    transition: PicasaTransition,
    tiles: Vec<Tile>,
    desired_tiles: usize,
    cinema: Option<Cinema>,
    coverflow: Option<CoverFlow>,
    pile: Option<Pile>,
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
        let cinema = if style.single_photo() && style != Style::CoverFlow {
            Some(Cinema::new(&root))
        } else {
            None
        };
        let coverflow = (style == Style::CoverFlow).then(|| CoverFlow::new(&root));
        // Avoid blocking GTK with the creation of thousands of overlays
        // before the fullscreen surface appears. Additional square tiles are
        // materialised in small frame-clock batches.
        // Picture Pile and Floating Prints paint themselves in one custom
        // widget (see pile.rs).
        let pile = matches!(style, Style::PicturePile | Style::Floating).then(|| {
            let pile = Pile::new(count, transition, style);
            root.put(&pile.canvas, 0.0, 0.0);
            pile
        });
        let desired_tiles = if style.single_photo() || pile.is_some() {
            0
        } else {
            count
        };
        let trace = std::env::var("PIC_SCREENSAVER_TRACE").is_ok_and(|v| v == "1");
        if trace {
            eprintln!("PIC_SCREEN init style={style:?} transition={transition:?} count={count}");
        }
        Self {
            root,
            style,
            transition,
            tiles: Vec::new(),
            desired_tiles,
            cinema,
            coverflow,
            pile,
            trace,
            traced_change: None,
            traced_settle: None,
            last_tick: None,
            synced_revision: None,
            synced_geometry: None,
            active_tile: None,
            synced_change: None,
        }
    }
    fn add_tile(&mut self) {
        let slot = self.tiles.len();
        if slot >= self.desired_tiles {
            return;
        }
        let tile = Tile::new(true);
        self.root.put(&tile.frame, 0.0, 0.0);
        self.tiles.push(tile);
    }
    pub fn sync(&mut self, player: &Player, frame_time: i64) {
        if self.trace {
            let now = Instant::now();
            if let Some(previous) = self.last_tick {
                let ms = now.duration_since(previous).as_millis();
                if ms >= 90 {
                    eprintln!(
                        "PIC_SCREEN frame_spike_ms={ms} style={:?} tiles={} loaded={} change={}",
                        self.style,
                        self.tiles.len(),
                        player.frames.len(),
                        player.changes,
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
        if w < 2.0 || h < 2.0 {
            return;
        }
        if let Some(cinema) = self.cinema.as_mut() {
            cinema.sync(
                player,
                w,
                h,
                frame_time,
                matches!(self.style, Style::IPhoto | Style::MatrixKenBurns),
            );
            return;
        }
        if let Some(coverflow) = self.coverflow.as_mut() {
            coverflow.sync(player, w, h);
            return;
        }
        if let Some(pile) = self.pile.as_mut() {
            pile.sync(player, w, h, frame_time);
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
        if !full_refresh && !change_pending && active_tile.is_none() && previous_active.is_none() {
            return;
        }
        self.synced_revision = Some(player.next_revision);
        self.synced_geometry = Some(geometry);
        self.synced_change = Some(player.changes);
        self.active_tile = active_tile;
        let square_grid = if full_refresh && self.style == Style::Mosaic {
            SquareGrid::fit(w, h, tile_count)
        } else {
            None
        };
        let changing_grid = if full_refresh && self.style == Style::Grid {
            Some(ChangingGrid::fit(w, h, tile_count))
        } else {
            None
        };
        for (slot, tile) in self.tiles.iter_mut().enumerate() {
            if !full_refresh && active_tile != Some(slot) && previous_active != Some(slot) {
                continue; // Only the changing tile needs an opacity update.
            }
            let Some(&new_id) = player.slots.get(slot) else {
                continue;
            };
            if full_refresh {
                let (x, y, tile_w, tile_h) = if let Some(grid) = square_grid {
                    let (x, y) = grid.position(slot, tile_count);
                    (x, y, grid.side, grid.side)
                } else if let Some(grid) = changing_grid {
                    grid.rect(slot, tile_count, w, h)
                } else {
                    let aspect = photo_aspect(player, new_id);
                    print_rect(self.style, slot, tile_count, aspect, w, h)
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
            let old = player
                .fading_slot
                .and_then(|(change_slot, old)| (change_slot == slot).then_some(old));
            tile.sync(new_id, old, fade, player, self.transition);
        }
    }
}

pub(super) fn install_css() {
    if let Some(display) = gtk::gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        let mut css = String::from(
            r#"
          .screensaver-stage { background-color: #090b10; }
          .screensaver-cinema-frame { background-color: #090b10; }
          .screensaver-card { background-color: #171b21; }
          .screensaver-caption {
            background-color: rgba(0,0,0,0.64);
            color: white;
            padding: 8px 16px;
            border-radius: 12px;
            font-size: 13px;
          }
          .screensaver-clock {
            color: rgba(250,251,255,0.96);
            background-color: rgba(12,16,24,0.56);
            border: 1px solid rgba(255,255,255,0.13);
            border-radius: 17px;
            padding: 10px 19px;
            font-size: 34px;
            font-weight: 600;
            box-shadow: 0 7px 22px rgba(0,0,0,0.25);
          }
          .screensaver-card > overlay { transform-origin: 50% 50%; }
        "#,
        );
        // Precompile quantised compositor transforms once, never reparse
        // a CSS stylesheet in the frame clock.
        for step in 0..=FLIP_STEPS {
            let factor = PicasaTransition::Flip.flip_width(step as f64 / FLIP_STEPS as f64);
            css.push_str(&format!(
                ".screensaver-flip-{step} {{ transform: scaleX({factor:.6}); }}\n"
            ));
        }
        provider.load_from_string(&css);
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_bounds_all_counts_and_aspects() {
        for (w, h) in [(1920., 1080.), (1080., 1920.), (3840., 2160.)] {
            for count in [1, 2, 3, 4, 6, 12, 24, 100] {
                let grid = ChangingGrid::fit(w, h, count);
                for slot in 0..count {
                    let (x, y, tw, th) = grid.rect(slot, count, w, h);
                    assert!(x >= 0. && y >= 0. && tw > 0. && th > 0.);
                    assert!(x + tw <= w + 0.001 && y + th <= h + 0.001);
                }
            }
        }
    }

    #[test]
    fn changing_grid_keeps_last_row_tiles_a_consistent_size() {
        for count in [5, 10, 23, 24, 99, 100, 101] {
            let grid = ChangingGrid::fit(1920., 1080., count);
            let first = grid.rect(0, count, 1920., 1080.);
            let last = grid.rect(count - 1, count, 1920., 1080.);
            assert!((first.2 - last.2).abs() < 0.01);
            assert!((first.3 - last.3).abs() < 0.01);
            assert!(last.0 + last.2 <= 1920.);
        }
    }

    #[test]
    fn print_cards_follow_photo_orientation_and_stay_on_screen() {
        for style in [Style::PicturePile, Style::Floating] {
            for (w, h) in [(1920., 1080.), (1080., 1920.), (1024., 768.)] {
                for count in 1..=6 {
                    for slot in 0..count {
                        for ratio in [0.67, 1.0, 1.5, 1.85] {
                            let (x, y, tw, th) = print_rect(style, slot, count, ratio, w, h);
                            assert!(x >= 0. && y >= 0. && tw > 0. && th > 0.);
                            assert!(x + tw <= w && y + th <= h, "{style:?} {w} {h} {slot}");
                            assert!(
                                (tw / th - ratio).abs() < 0.002,
                                "{style:?}: intended {ratio}, actual {}",
                                tw / th
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn photo_prints_are_smaller_than_old_oversized_scrapbook() {
        let (_, _, tw, th) = print_rect(Style::PicturePile, 0, 5, 1.5, 1649., 927.);
        assert!(tw < 550. && th < 370.);
        let (_, _, pw, ph) = print_rect(Style::PicturePile, 4, 5, 0.7, 1649., 927.);
        assert!(pw < 280. && ph < 400.);
        // Portraits are portrait-shaped cards, not a narrow image trapped
        // in a landscape rectangle filled by huge white side margins.
        assert!(pw < ph);
    }

    #[test]
    fn picture_pile_keeps_overlap_small_enough_to_read_each_photo() {
        let (w, h) = (1920., 1080.);
        let photos: Vec<_> = (0..5)
            .map(|slot| print_rect(Style::PicturePile, slot, 5, 1.5, w, h))
            .collect();
        let mut total_overlap = 0.0;
        for (i, a) in photos.iter().enumerate() {
            for b in photos.iter().skip(i + 1) {
                let overlap_w = (a.0 + a.2).min(b.0 + b.2) - a.0.max(b.0);
                let overlap_h = (a.1 + a.3).min(b.1 + b.3) - a.1.max(b.1);
                if overlap_w > 0.0 && overlap_h > 0.0 {
                    let overlap = overlap_w * overlap_h;
                    total_overlap += overlap;
                    assert!(overlap / a.2.mul_add(a.3, 0.0) < 0.05);
                    assert!(overlap / b.2.mul_add(b.3, 0.0) < 0.05);
                }
            }
        }
        assert!(
            total_overlap > 0.0,
            "the pile should still overlap like a scrapbook"
        );
    }

    #[test]
    fn picture_pile_is_level_while_floating_prints_keep_a_gentle_tilt() {
        let pile_max = (0..6)
            .map(|slot| base_tilt(Style::PicturePile, slot).abs())
            .fold(0.0, f64::max);
        let floating_max = (0..6)
            .map(|slot| base_tilt(Style::Floating, slot).abs())
            .fold(0.0, f64::max)
            + FLOATING_TILT_SWAY;
        assert_eq!(pile_max, 0.0);
        assert!(floating_max < 1.1);
        assert!(FLOATING_DRIFT_X < 4.0 && FLOATING_DRIFT_Y < 4.0);
    }

    #[test]
    fn floating_cards_have_distinct_rows_and_no_rectangle_overlap() {
        let (w, h) = (1920., 1080.);
        let cards: Vec<_> = (0..6)
            .map(|slot| print_rect(Style::Floating, slot, 6, 1.5, w, h))
            .collect();
        for (i, a) in cards.iter().enumerate() {
            for b in cards.iter().skip(i + 1) {
                assert!(
                    a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.3 <= b.1 || b.1 + b.3 <= a.1
                );
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
                    assert!(
                        x >= -0.01 && y >= -0.01,
                        "{width}x{height} {count} {slot}: {x}, {y}"
                    );
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
