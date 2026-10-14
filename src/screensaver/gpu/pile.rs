//! Picture Pile and Floating Prints: prints that are *laid down* on a table.
//! (Floating Prints is the same engine with a calmer look plus a slow, continuous
//! drift driven by the animation clock.)
//!
//! Why this is not built from GtkFixed/GtkOverlay/CSS like the other walls:
//!
//! * `Fixed::child_position()` reports the child's translation *including* its
//!   CSS rotation offset. Re-applying it with `move_()` shifted every print by
//!   several pixels each time it was raised, and a later refresh shifted it
//!   back. That was the visible jitter.
//! * A replacement photo used to resize its frame (new aspect ratio) on the
//!   very frame it was assigned, while the *old* photo was still showing.
//! * Texture revisions (preview -> original) swapped mid-fade and re-laid-out
//!   every tile.
//!
//! The pile is therefore one custom widget that paints every print in a single
//! GSK snapshot. All motion is analytic (no CSS classes, no relayout, no
//! quantised steps), and the logic is a small display-free state machine
//! (`Model`) with these rules:
//!
//! 1. A print's geometry (size, position, resting angle) is decided ONCE, when
//!    the print is placed, from its own aspect ratio. It never changes again
//!    except on a monitor resize.
//! 2. The outgoing print is never touched: it keeps its own texture and
//!    geometry until it has been covered, then fades only at the tail.
//! 3. A better texture (original replacing a cached preview) cross-fades in
//!    place over the old one; it never pops and never moves the print.
//! 4. Animation time advances by the frame clock's delta, clamped, so one slow
//!    frame (texture upload, decode result) cannot make motion jump ahead.
use super::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, graphene, gsk};

/// Paper border as a fraction of the shorter screen edge (clamped to 4..14px).
/// Set to 0.0 for borderless prints.
const PILE_BORDER: f64 = 0.0085;
/// A newly placed print starts this much larger (as if held above the table).
pub(super) const PILE_LAND_SCALE: f64 = 1.04;
/// ...and this many degrees off its resting angle.
pub(super) const PILE_LAND_TURN: f64 = 1.1;
/// Per-print random tilt added to the slot's resting angle (+/- degrees).
pub(super) const PILE_WOBBLE: f64 = 0.6;
/// Initial photos drop in one after another instead of all at once.
const PILE_STAGGER: f64 = 0.16;
/// Original-quality texture cross-fade time.
pub(super) const PILE_UPGRADE_SECS: f64 = 0.35;
/// Longest animation step accepted for one frame; hides one-off stalls.
pub(super) const PILE_MAX_DT: f64 = 0.05;

pub(super) trait Aspect {
    fn aspect(&self) -> f64;
}
impl Aspect for gdk::Texture {
    fn aspect(&self) -> f64 {
        self.width().max(1) as f64 / self.height().max(1) as f64
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 { a + (b - a) * t }

/// Deterministic per-photo tilt so the same photo always lands the same way.
fn wobble(style: Style, id: usize) -> f64 {
    if style != Style::PicturePile { return 0.0; }
    let h = (id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40;
    ((h % 1000) as f64 / 1000.0 - 0.5) * 2.0 * PILE_WOBBLE
}
fn land_direction(id: usize) -> f64 {
    if ((id.wrapping_mul(2_654_435_761) >> 5) & 1) == 0 { 1.0 } else { -1.0 }
}

/// Pose of a print while it is being laid down (progress 0..=1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Pose {
    pub opacity: f64,
    pub scale: f64,
    /// Degrees away from the resting angle.
    pub turn: f64,
    /// 1.0 = held above the table (soft, far shadow), 0.0 = lying flat.
    pub lift: f64,
}
pub(super) fn landing_pose(progress: f64, direction: f64) -> Pose {
    landing_pose_with(progress, direction, PILE_LAND_SCALE, PILE_LAND_TURN)
}
fn landing_pose_with(progress: f64, direction: f64, scale: f64, turn: f64) -> Pose {
    let p = progress.clamp(0.0, 1.0);
    let lift = 1.0 - smootherstep(p);
    Pose {
        opacity: smootherstep(p / 0.5),
        scale: 1.0 + (scale - 1.0) * lift,
        turn: direction * turn * lift,
        lift,
    }
}

/// Printed look per style.
#[derive(Clone, Copy)]
struct Look {
    paper: (f32, f32, f32),
    radius: f32,
    shadow: f32,
    shadow_dy: f32,
    shadow_blur: f32,
}
fn look(style: Style) -> Look {
    if style == Style::Floating {
        Look { paper: (0.09, 0.105, 0.13), radius: 3.0, shadow: 0.28, shadow_dy: 8.0, shadow_blur: 20.0 }
    } else {
        Look { paper: (0.955, 0.945, 0.915), radius: 2.0, shadow: 0.36, shadow_dy: 8.0, shadow_blur: 22.0 }
    }
}
/// Landing amplitude (scale, degrees): Floating Prints settle more gently.
fn landing_amplitude(style: Style) -> (f64, f64) {
    if style == Style::Floating { (1.02, 0.5) } else { (PILE_LAND_SCALE, PILE_LAND_TURN) }
}
/// The outgoing print stays fully opaque until the new one is (almost)
/// opaque on top of it, then fades only where it sticks out.
pub(super) fn leaving_opacity(progress: f64) -> f64 {
    1.0 - smootherstep((progress - 0.55) / 0.45)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Geometry {
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    pub border: f64,
}
/// Outer print rectangle (photo + paper border). The photo area keeps the
/// photograph's own aspect ratio exactly, so nothing is cropped or stretched.
pub(super) fn pile_geometry(slot: usize, count: usize, aspect: f64, width: f64, height: f64)
    -> Geometry
{
    print_geometry(Style::PicturePile, slot, count, aspect, width, height)
}
pub(super) fn print_geometry(
    style: Style, slot: usize, count: usize, aspect: f64, width: f64, height: f64,
) -> Geometry {
    let ratio = aspect.clamp(0.60, 2.0);
    let (x, y, tw, th) = print_rect(style, slot, count, ratio, width, height);
    let border = if style == Style::Floating {
        2.0
    } else if PILE_BORDER > 0.0 {
        (width.min(height) * PILE_BORDER).clamp(4.0, 14.0).round()
    } else { 0.0 };
    let border = border.min(tw * 0.05);
    let inner_w = (tw - 2.0 * border).max(1.0);
    let inner_h = inner_w / ratio;
    Geometry {
        cx: x + tw * 0.5,
        cy: y + th * 0.5,
        w: tw,
        h: inner_h + 2.0 * border,
        border,
    }
}

#[derive(Clone, Debug)]
pub(super) struct DrawCard<T> {
    pub id: usize,
    pub slot: usize,
    pub texture: T,
    /// Sharper texture fading in over `texture`, with its current opacity.
    pub upgrade: Option<(T, f64)>,
    pub geometry: Geometry,
    pub angle: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub opacity: f64,
    pub lift: f64,
    pub shade: f64,
}

struct Card<T> {
    id: usize,
    texture: T,
    quality: bool,
    upgrade: Option<(T, f64)>,
    aspect: f64,
}
struct Slot<T> {
    card: Option<Card<T>>,
    leaving: Option<(Card<T>, u64)>,
    since: f64,
    z: u64,
}
impl<T> Default for Slot<T> {
    fn default() -> Self { Self { card: None, leaving: None, since: 0.0, z: 0 } }
}

/// What the model needs to know about the player at one frame.
pub(super) struct Feed<'a, T> {
    pub slots: &'a [usize],
    pub changes: usize,
    pub replacing: Option<(usize, usize)>,
    pub transition: f64,
    /// Seconds for one full drift cycle (Floating Prints only).
    pub drift_period: f64,
    /// Texture and "is original quality" for a photo, when loaded.
    pub frame: &'a dyn Fn(usize) -> Option<(T, bool)>,
}

pub(super) struct Model<T> {
    style: Style,
    drift_period: f64,
    mode: PicasaTransition,
    slots: Vec<Slot<T>>,
    clock: f64,
    last_frame: Option<i64>,
    seen_changes: usize,
    z_counter: u64,
    size: (f64, f64),
    dirty: bool,
}
impl<T: Clone + Aspect> Model<T> {
    pub fn new(count: usize, mode: PicasaTransition) -> Self {
        Self::with_style(count, mode, Style::PicturePile)
    }
    pub fn with_style(count: usize, mode: PicasaTransition, style: Style) -> Self {
        Self {
            style,
            drift_period: 6.0,
            mode,
            slots: (0..count).map(|_| Slot::default()).collect(),
            clock: 0.0,
            last_frame: None,
            seen_changes: 0,
            z_counter: 0,
            size: (0.0, 0.0),
            dirty: true,
        }
    }

    /// Advance the animation clock by the frame clock's delta (microseconds).
    pub fn advance(&mut self, frame_time_us: i64, paused: bool) {
        let dt = self.last_frame
            .map(|previous| ((frame_time_us - previous) as f64 / 1e6).clamp(0.0, PILE_MAX_DT))
            .unwrap_or(0.0);
        self.last_frame = Some(frame_time_us);
        if !paused { self.clock += dt; }
    }

    fn place(&mut self, slot: usize, id: usize, texture: T, quality: bool, delay: f64) {
        let state = &mut self.slots[slot];
        let old = state.card.take();
        let old_z = state.z;
        state.leaving = match (self.mode, old) {
            (PicasaTransition::Cut, _) | (_, None) => None,
            (_, Some(card)) => Some((card, old_z)),
        };
        let aspect = texture.aspect();
        state.card = Some(Card { id, texture, quality, upgrade: None, aspect });
        state.since = self.clock + delay;
        self.z_counter += 1;
        // A flipping card keeps its place in the stack; a laid-down print
        // goes on top.
        state.z = if self.mode == PicasaTransition::Flip && state.leaving.is_some() {
            old_z
        } else {
            self.z_counter
        };
        self.dirty = true;
    }

    /// Reconcile with the player. Returns the cards to paint when anything
    /// is moving or has changed, `None` while the pile is at rest.
    pub fn update(&mut self, feed: &Feed<T>, size: (f64, f64)) -> Option<Vec<DrawCard<T>>> {
        if size != self.size { self.size = size; self.dirty = true; }
        let duration = feed.transition.max(0.2);
        self.drift_period = feed.drift_period.max(2.0);

        // A change the player just made: the former print stays as it was,
        // the new photo is laid on top of it.
        if feed.changes != self.seen_changes {
            self.seen_changes = feed.changes;
            if let Some((slot, _former)) = feed.replacing {
                if let (Some(&id), true) = (feed.slots.get(slot), slot < self.slots.len()) {
                    let current = self.slots[slot].card.as_ref().map(|card| card.id);
                    if current != Some(id) {
                        if let Some((texture, quality)) = (feed.frame)(id) {
                            self.place(slot, id, texture, quality, 0.0);
                        }
                    }
                }
            }
        }

        // Initial photos, backfilled photos and original-quality upgrades.
        let mut stagger = 0.0;
        for slot in 0..self.slots.len().min(feed.slots.len()) {
            let id = feed.slots[slot];
            let Some((texture, quality)) = (feed.frame)(id) else { continue };
            let clock = self.clock;
            match self.slots[slot].card.as_mut() {
                Some(card) if card.id == id => {
                    if quality && !card.quality && card.upgrade.is_none() {
                        card.upgrade = Some((texture, clock));
                        self.dirty = true;
                    }
                }
                Some(_) => self.place(slot, id, texture, quality, 0.0),
                None => {
                    self.place(slot, id, texture, quality, stagger);
                    stagger += PILE_STAGGER;
                }
            }
        }

        // Retire finished animations.
        for state in &mut self.slots {
            if state.leaving.is_some() && self.clock - state.since >= duration {
                state.leaving = None;
                self.dirty = true;
            }
            if let Some(card) = state.card.as_mut() {
                if card.upgrade.as_ref().is_some_and(|(_, t0)| self.clock - t0 >= PILE_UPGRADE_SECS) {
                    if let Some((texture, _)) = card.upgrade.take() {
                        card.texture = texture;
                        card.quality = true;
                        self.dirty = true;
                    }
                }
            }
        }

        let animating = self.slots.iter().any(|state| {
            state.leaving.is_some()
                || state.card.as_ref().is_some_and(|card| card.upgrade.is_some())
                || (state.card.is_some() && self.clock - state.since < duration)
        });
        // Floating Prints never rest: they drift continuously.
        let drifting = self.style == Style::Floating && self.slots.iter().any(|s| s.card.is_some());
        if !animating && !drifting && !self.dirty { return None; }
        self.dirty = false;
        Some(self.compose(duration))
    }

    fn draw(&self, card: &Card<T>, geometry: Geometry, slot: usize) -> DrawCard<T> {
        let upgrade = card.upgrade.as_ref().map(|(texture, t0)| {
            (texture.clone(), smootherstep((self.clock - t0) / PILE_UPGRADE_SECS))
        });
        DrawCard {
            id: card.id,
            slot,
            texture: card.texture.clone(),
            upgrade,
            geometry,
            angle: base_tilt(self.style, slot) + wobble(self.style, card.id),
            scale_x: 1.0,
            scale_y: 1.0,
            opacity: 1.0,
            lift: 0.0,
            shade: 0.0,
        }
    }

    fn compose(&self, duration: f64) -> Vec<DrawCard<T>> {
        let (width, height) = self.size;
        let count = self.slots.len();
        let mut stack: Vec<(u64, DrawCard<T>)> = Vec::with_capacity(count * 2);
        for (slot, state) in self.slots.iter().enumerate() {
            let Some(card) = state.card.as_ref() else { continue };
            let progress = ((self.clock - state.since) / duration).clamp(0.0, 1.0);
            let resting = print_geometry(self.style, slot, count, card.aspect, width, height);
            match (self.mode, state.leaving.as_ref()) {
                (PicasaTransition::Flip, Some((old, _))) => {
                    // One card turned over about its vertical axis. The photo
                    // changes only while it is edge-on, and size/centre/angle
                    // glide between the two prints so nothing snaps.
                    let before = print_geometry(self.style, slot, count, old.aspect, width, height);
                    let m = smootherstep(progress);
                    let geometry = Geometry {
                        cx: lerp(before.cx, resting.cx, m),
                        cy: lerp(before.cy, resting.cy, m),
                        w: lerp(before.w, resting.w, m),
                        h: lerp(before.h, resting.h, m),
                        border: lerp(before.border, resting.border, m),
                    };
                    let shown = if progress < 0.5 { old } else { card };
                    let mut draw = self.draw(shown, geometry, slot);
                    let width_factor = PicasaTransition::Flip.flip_width(progress);
                    draw.angle = lerp(
                        base_tilt(self.style, slot) + wobble(self.style, old.id),
                        base_tilt(self.style, slot) + wobble(self.style, card.id), m);
                    draw.scale_x = width_factor;
                    draw.shade = 0.35 * (1.0 - width_factor);
                    stack.push((state.z, draw));
                }
                (PicasaTransition::Cut, _) => {
                    stack.push((state.z, self.draw(card, resting, slot)));
                }
                (_, leaving) => {
                    if let Some((old, old_z)) = leaving {
                        let before = print_geometry(self.style, slot, count, old.aspect, width, height);
                        let mut draw = self.draw(old, before, slot);
                        draw.opacity = leaving_opacity(progress);
                        stack.push((*old_z, draw));
                    }
                    let (amp_scale, amp_turn) = landing_amplitude(self.style);
                    let pose = landing_pose_with(progress, land_direction(card.id), amp_scale, amp_turn);
                    let mut draw = self.draw(card, resting, slot);
                    draw.opacity = pose.opacity;
                    draw.scale_x = pose.scale;
                    draw.scale_y = pose.scale;
                    draw.angle += pose.turn;
                    draw.lift = pose.lift;
                    stack.push((state.z, draw));
                }
            }
        }
        if self.style == Style::Floating {
            // One continuous phase per print slot: no reset when photos change.
            for (_, card) in stack.iter_mut() {
                let phase = (self.clock / self.drift_period + card.slot as f64 * 0.618_034).fract();
                let t = std::f64::consts::TAU * phase;
                card.geometry.cx += FLOATING_DRIFT_X * t.sin();
                card.geometry.cy += FLOATING_DRIFT_Y * t.cos();
                card.angle += FLOATING_TILT_SWAY * t.sin();
            }
        }
        stack.sort_by_key(|(z, _)| *z);
        stack.into_iter().map(|(_, card)| card).collect()
    }
}

// ---------------------------------------------------------------------------
// GTK side: one widget, one snapshot.
// ---------------------------------------------------------------------------

mod canvas_imp {
    use super::*;

    #[derive(Default)]
    pub struct PileCanvas {
        pub(super) cards: RefCell<Vec<DrawCard<gdk::Texture>>>,
        pub(super) warm: RefCell<Vec<gdk::Texture>>,
        pub(super) style: Cell<Option<Style>>,
    }
    #[gtk::glib::object_subclass]
    impl ObjectSubclass for PileCanvas {
        const NAME: &'static str = "PicPileCanvas";
        type Type = super::PileCanvas;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for PileCanvas {}
    impl WidgetImpl for PileCanvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            // Make the GPU upload upcoming textures now, in a 1px nearly
            // invisible draw, instead of during the first frames of a landing.
            let warm: Vec<gdk::Texture> = self.warm.take();
            if !warm.is_empty() {
                let obj = self.obj();
                let (w, h) = (obj.width() as f32, obj.height() as f32);
                snapshot.push_opacity(0.01);
                snapshot.push_clip(&graphene::Rect::new((w - 1.0).max(0.0), (h - 1.0).max(0.0), 1.0, 1.0));
                for texture in &warm {
                    snapshot.append_scaled_texture(
                        texture, gsk::ScalingFilter::Trilinear,
                        &graphene::Rect::new((w - 16.0).max(0.0), (h - 9.0).max(0.0), 16.0, 9.0),
                    );
                }
                snapshot.pop();
                snapshot.pop();
            }
            let look = look(self.style.get().unwrap_or(Style::PicturePile));
            for card in self.cards.borrow().iter() {
                paint_card(snapshot, card, look);
            }
        }
    }
}
gtk::glib::wrapper! {
    pub struct PileCanvas(ObjectSubclass<canvas_imp::PileCanvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}
impl PileCanvas {
    fn new(style: Style) -> Self {
        let canvas: Self = gtk::glib::Object::new();
        canvas.imp().style.set(Some(style));
        canvas.set_can_target(false);
        canvas.set_focusable(false);
        canvas
    }
    fn set_cards(&self, cards: Vec<DrawCard<gdk::Texture>>) {
        *self.imp().cards.borrow_mut() = cards;
        self.queue_draw();
    }
    fn set_warm(&self, textures: Vec<gdk::Texture>) {
        *self.imp().warm.borrow_mut() = textures;
        self.queue_draw();
    }
    #[cfg(test)]
    pub(super) fn painted(&self) -> Vec<DrawCard<gdk::Texture>> {
        self.imp().cards.borrow().clone()
    }
}

/// Draw `texture` into `dest`. Photographs whose aspect already matches the
/// frame are scaled exactly (trilinear: a 2 MP photo shown at 500px must be
/// mip-mapped or it shimmers while moving); anything else is cover-cropped.
fn paint_photo(snapshot: &gtk::Snapshot, texture: &gdk::Texture, dest: &graphene::Rect) {
    let filter = gsk::ScalingFilter::Trilinear;
    let texture_aspect = texture.width().max(1) as f32 / texture.height().max(1) as f32;
    let frame_aspect = dest.width() / dest.height().max(0.001);
    if (texture_aspect / frame_aspect - 1.0).abs() < 0.03 {
        snapshot.append_scaled_texture(texture, filter, dest);
        return;
    }
    let (w, h) = if texture_aspect > frame_aspect {
        (dest.height() * texture_aspect, dest.height())
    } else {
        (dest.width(), dest.width() / texture_aspect)
    };
    snapshot.push_clip(dest);
    snapshot.append_scaled_texture(
        texture, filter,
        &graphene::Rect::new(
            dest.x() + (dest.width() - w) * 0.5, dest.y() + (dest.height() - h) * 0.5, w, h),
    );
    snapshot.pop();
}

fn paint_card(snapshot: &gtk::Snapshot, card: &DrawCard<gdk::Texture>, look: Look) {
    if card.opacity <= 0.002 { return; }
    let g = card.geometry;
    let (w, h, b) = (g.w as f32, g.h as f32, g.border as f32);
    snapshot.save();
    snapshot.translate(&graphene::Point::new(g.cx as f32, g.cy as f32));
    snapshot.rotate(card.angle as f32);
    snapshot.scale(card.scale_x as f32, card.scale_y as f32);
    snapshot.translate(&graphene::Point::new(-w * 0.5, -h * 0.5));
    snapshot.push_opacity(card.opacity.clamp(0.0, 1.0));

    let outline = gsk::RoundedRect::from_rect(graphene::Rect::new(0.0, 0.0, w, h), look.radius);
    let lift = card.lift as f32;
    snapshot.append_outset_shadow(
        &outline,
        &gdk::RGBA::new(0.0, 0.0, 0.0, look.shadow - 0.10 * lift),
        0.0, look.shadow_dy + 14.0 * lift, 0.0, look.shadow_blur + 12.0 * lift,
    );
    snapshot.append_color(
        &gdk::RGBA::new(look.paper.0, look.paper.1, look.paper.2, 1.0),
        &graphene::Rect::new(0.0, 0.0, w, h),
    );
    let photo = graphene::Rect::new(b, b, (w - 2.0 * b).max(1.0), (h - 2.0 * b).max(1.0));
    paint_photo(snapshot, &card.texture, &photo);
    if let Some((texture, alpha)) = &card.upgrade {
        snapshot.push_opacity(alpha.clamp(0.0, 1.0));
        paint_photo(snapshot, texture, &photo);
        snapshot.pop();
    }
    if card.shade > 0.001 {
        snapshot.append_color(
            &gdk::RGBA::new(0.0, 0.0, 0.0, card.shade as f32),
            &graphene::Rect::new(0.0, 0.0, w, h),
        );
    }
    snapshot.pop();
    snapshot.restore();
}

pub(super) struct Pile {
    model: Model<gdk::Texture>,
    pub canvas: PileCanvas,
    warmed: HashSet<(usize, u64)>,
    size: (i32, i32),
}
impl Pile {
    pub fn new(count: usize, mode: PicasaTransition, style: Style) -> Self {
        Self {
            model: Model::with_style(count, mode, style),
            canvas: PileCanvas::new(style),
            warmed: HashSet::new(),
            size: (0, 0),
        }
    }
    pub fn sync(&mut self, player: &Player, w: f64, h: f64, frame_time: i64) {
        self.model.advance(frame_time, player.paused_at.is_some());
        let size = (w.round() as i32, h.round() as i32);
        if size != self.size {
            self.size = size;
            self.canvas.set_size_request(size.0, size.1);
        }
        let lookup = |id: usize| player.frames.get(&id)
            .map(|surface| (surface.texture.clone(), surface.high_quality));
        let feed = Feed {
            slots: &player.slots,
            changes: player.changes,
            replacing: player.fading_slot,
            transition: player.transition.as_secs_f64(),
            drift_period: player.interval.as_secs_f64(),
            frame: &lookup,
        };
        if let Some(cards) = self.model.update(&feed, (w, h)) {
            self.canvas.set_cards(cards);
        }
        // Textures that will be promoted next are uploaded ahead of time.
        let mut warm = Vec::new();
        for offset in 0..PREFETCH {
            if player.count == 0 { break; }
            let index = (player.next + offset) % player.count;
            if player.slots.contains(&index) { continue; }
            if let Some(surface) = player.frames.get(&index) {
                if surface.high_quality && self.warmed.insert((index, surface.revision)) {
                    warm.push(surface.texture.clone());
                }
            }
        }
        if self.warmed.len() > 512 { self.warmed.clear(); }
        if !warm.is_empty() { self.canvas.set_warm(warm); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Fake(usize, f64);
    impl Aspect for Fake { fn aspect(&self) -> f64 { self.1 } }

    const ASPECTS: [f64; 6] = [1.5, 0.667, 1.0, 1.78, 0.75, 1.33];
    fn aspect_of(id: usize) -> f64 { ASPECTS[id % ASPECTS.len()] }

    /// Drives a Model at a fixed frame rate against a scripted feed.
    struct Rig {
        model: Model<Fake>,
        slots: Vec<usize>,
        changes: usize,
        replacing: Option<(usize, usize)>,
        quality: HashMap<usize, bool>,
        loaded: HashSet<usize>,
        time_us: i64,
        transition: f64,
        history: Vec<Vec<DrawCard<Fake>>>,
        step_us: i64,
        paused: bool,
    }
    impl Rig {
        fn new(mode: PicasaTransition, count: usize, transition: f64) -> Self {
            Self {
                model: Model::new(count, mode),
                slots: (0..count).collect(),
                changes: 0,
                replacing: None,
                quality: HashMap::new(),
                loaded: HashSet::new(),
                time_us: 1_000_000,
                transition,
                history: Vec::new(),
                step_us: 16_667,
                paused: false,
            }
        }
        fn load(&mut self, id: usize, high_quality: bool) {
            self.loaded.insert(id);
            self.quality.insert(id, high_quality);
        }
        /// Replace `slot` with a new photo exactly as `Player::update` does.
        fn replace(&mut self, slot: usize, id: usize) {
            let former = std::mem::replace(&mut self.slots[slot], id);
            self.replacing = Some((slot, former));
            self.changes += 1;
        }
        fn frame(&mut self) -> Vec<DrawCard<Fake>> {
            self.time_us += self.step_us;
            self.model.advance(self.time_us, self.paused);
            let loaded = self.loaded.clone();
            let quality = self.quality.clone();
            let lookup = move |id: usize| loaded.contains(&id)
                .then(|| (Fake(id, aspect_of(id)), quality[&id]));
            let slots = self.slots.clone();
            let feed = Feed {
                slots: &slots, changes: self.changes, replacing: self.replacing,
                transition: self.transition, drift_period: 6.0, frame: &lookup,
            };
            let out = self.model.update(&feed, (1920.0, 1080.0));
            if let Some(cards) = &out { self.history.push(cards.clone()); }
            out.unwrap_or_default()
        }
        fn run(&mut self, seconds: f64) {
            for _ in 0..(seconds * 60.0) as usize { self.frame(); }
        }
    }

    #[test]
    fn landing_pose_settles_exactly_and_monotonically() {
        let start = landing_pose(0.0, 1.0);
        assert_eq!(start.opacity, 0.0);
        assert!((start.scale - PILE_LAND_SCALE).abs() < 1e-9);
        let end = landing_pose(1.0, -1.0);
        assert_eq!(end, Pose { opacity: 1.0, scale: 1.0, turn: 0.0, lift: 0.0 });
        let mut previous = landing_pose(0.0, 1.0);
        for step in 1..=200 {
            let pose = landing_pose(step as f64 / 200.0, 1.0);
            assert!(pose.opacity >= previous.opacity - 1e-12);
            assert!(pose.scale <= previous.scale + 1e-12);
            assert!(pose.turn.abs() <= previous.turn.abs() + 1e-12);
            assert!((1.0..=PILE_LAND_SCALE).contains(&pose.scale));
            previous = pose;
        }
    }

    #[test]
    fn outgoing_print_is_never_exposed_before_the_new_one_covers_it() {
        // Whenever the incoming print is not yet opaque the old one is fully
        // opaque, so no frame ever shows the table through the pile.
        for step in 0..=1000 {
            let p = step as f64 / 1000.0;
            let incoming = landing_pose(p, 1.0).opacity;
            let outgoing = leaving_opacity(p);
            if incoming < 1.0 { assert_eq!(outgoing, 1.0, "p={p}"); }
            assert!(1.0 - (1.0 - incoming) * (1.0 - outgoing) > 0.999, "p={p}");
        }
        assert_eq!(leaving_opacity(1.0), 0.0);
    }

    #[test]
    fn geometry_stays_on_screen_for_every_aspect_screen_and_landing_scale() {
        for (w, h) in [(1920.0, 1080.0), (1080.0, 1920.0), (1024.0, 768.0), (3840.0, 2160.0)] {
            for count in 1..=PICTURE_PILE_SLOTS {
                for slot in 0..count {
                    for aspect in [0.5, 0.67, 1.0, 1.5, 1.85, 3.0] {
                        let g = pile_geometry(slot, count, aspect, w, h);
                        let tilt = (base_tilt(Style::PicturePile, slot).abs() + PILE_WOBBLE
                            + PILE_LAND_TURN).to_radians();
                        let scale = PILE_LAND_SCALE;
                        // Rotated, landing-scaled bounding box.
                        let half_w = scale * (g.w * 0.5 * tilt.cos() + g.h * 0.5 * tilt.sin());
                        let half_h = scale * (g.w * 0.5 * tilt.sin() + g.h * 0.5 * tilt.cos());
                        assert!(g.cx - half_w >= 0.0 && g.cx + half_w <= w, "{w}x{h} {slot}/{count} {aspect}");
                        assert!(g.cy - half_h >= 0.0 && g.cy + half_h <= h, "{w}x{h} {slot}/{count} {aspect}");
                        let inner = (g.w - 2.0 * g.border) / (g.h - 2.0 * g.border);
                        let expected = aspect.clamp(0.6, 2.0);
                        assert!((inner / expected - 1.0).abs() < 1e-9,
                            "photo area must keep the photograph's aspect: {inner} vs {expected}");
                    }
                }
            }
        }
    }

    /// The core regression: while a print exists its geometry is constant,
    /// no matter what else loads, upgrades or is replaced around it.
    #[test]
    fn print_geometry_never_changes_during_its_life() {
        for mode in [PicasaTransition::Crossfade, PicasaTransition::Cut] {
            let mut rig = Rig::new(mode, 5, 0.9);
            for id in 0..5 { rig.load(id, false); }
            rig.run(1.0);
            let mut next = 5;
            for round in 0..12 {
                // Unrelated frames arriving (prefetch, upgrades) between changes.
                rig.load(next, false);
                rig.frame();
                rig.load(round % 5, true);
                rig.load(next, true);
                rig.replace(round % 5, next);
                next += 1;
                rig.run(1.3);
            }
            let mut seen: HashMap<usize, Geometry> = HashMap::new();
            for frame in &rig.history {
                for card in frame {
                    let first = seen.entry(card.id).or_insert(card.geometry);
                    assert_eq!(*first, card.geometry, "{mode:?}: print {} moved or resized", card.id);
                }
            }
            assert!(seen.len() >= 17);
        }
    }

    #[test]
    fn a_slot_is_never_empty_after_its_first_print_and_motion_is_smooth() {
        let mut rig = Rig::new(PicasaTransition::Crossfade, 5, 0.9);
        for id in 0..12 { rig.load(id, true); }
        rig.run(1.5);
        let first_visible = rig.history.len();
        for round in 0..6 {
            rig.replace(round % 5, 5 + round);
            rig.run(1.2);
        }
        let mut last: HashMap<usize, (f64, f64, f64)> = HashMap::new();
        for frame in rig.history.iter().skip(first_visible) {
            let mut opaque_in_slot: HashMap<usize, bool> = HashMap::new();
            for card in frame {
                let entry = opaque_in_slot.entry(card.slot).or_insert(false);
                *entry |= card.opacity > 0.999;
            }
            assert!(opaque_in_slot.values().all(|covered| *covered), "a hole appeared in the pile");
            let mut current: HashMap<usize, (f64, f64, f64)> = HashMap::new();
            for card in frame {
                if let Some(&(opacity, scale, angle)) = last.get(&card.id) {
                    assert!((card.opacity - opacity).abs() < 0.08, "opacity step too large");
                    assert!((card.scale_x - scale).abs() < 0.004, "scale step too large");
                    assert!((card.angle - angle).abs() < 0.06, "angle step too large");
                }
                current.insert(card.id, (card.opacity, card.scale_x, card.angle));
            }
            last = current;
        }
    }

    #[test]
    fn finished_pile_goes_idle_and_leaving_prints_are_released() {
        let mut rig = Rig::new(PicasaTransition::Crossfade, 5, 0.9);
        for id in 0..6 { rig.load(id, true); }
        rig.run(3.0);
        rig.replace(2, 5);
        rig.run(0.5);
        assert!(rig.history.last().unwrap().len() >= 6, "old print should still be under the new one");
        rig.run(1.0);
        let before = rig.history.len();
        for _ in 0..30 { assert!(rig.frame().is_empty(), "idle pile must not request repaints"); }
        assert_eq!(rig.history.len(), before);
        assert_eq!(rig.history.last().unwrap().len(), 5);
        assert!(rig.history.last().unwrap().iter().all(|c| c.opacity == 1.0 && c.scale_x == 1.0 && c.lift == 0.0));
    }

    #[test]
    fn original_quality_cross_fades_in_place_without_moving_the_print() {
        let mut rig = Rig::new(PicasaTransition::Crossfade, 5, 0.9);
        for id in 0..5 { rig.load(id, false); }
        rig.run(2.0);
        let geometry_before = rig.history.last().unwrap().iter().find(|c| c.id == 1).unwrap().geometry;
        rig.load(1, true);
        rig.run(0.2);
        let mid = rig.history.last().unwrap().iter().find(|c| c.id == 1).unwrap().clone();
        let (_, alpha) = mid.upgrade.clone().expect("upgrade should be fading in");
        assert!(alpha > 0.0 && alpha < 1.0);
        assert_eq!(mid.geometry, geometry_before);
        rig.run(0.4);
        let end = rig.history.last().unwrap().iter().find(|c| c.id == 1).unwrap().clone();
        assert!(end.upgrade.is_none());
        assert_eq!(end.geometry, geometry_before);
    }

    #[test]
    fn replacement_prints_keep_their_own_aspect_not_the_old_ones() {
        let mut rig = Rig::new(PicasaTransition::Crossfade, 5, 0.9);
        for id in 0..6 { rig.load(id, true); }
        rig.run(2.0);
        rig.replace(0, 1); // 1.5 (landscape) -> 0.667 (portrait)... id 1 has aspect 0.667
        rig.run(0.2);
        let frame = rig.history.last().unwrap();
        let old = frame.iter().find(|c| c.id == 0).unwrap();
        let new = frame.iter().find(|c| c.id == 1).unwrap();
        assert!(old.geometry.w > old.geometry.h && new.geometry.h > new.geometry.w);
        // The outgoing print is exactly what it was before the change.
        let rest = pile_geometry(0, 5, aspect_of(0), 1920.0, 1080.0);
        assert_eq!(old.geometry, rest);
    }

    #[test]
    fn initial_prints_drop_in_one_after_another() {
        let mut rig = Rig::new(PicasaTransition::Crossfade, 5, 0.9);
        for id in 0..5 { rig.load(id, true); }
        rig.frame();
        let first = rig.history[0].clone();
        let visible = first.iter().filter(|card| card.opacity > 0.0).count();
        assert!(visible <= 1, "{visible} prints popped in on the first frame");
        rig.run(2.0);
        assert!(rig.history.last().unwrap().iter().all(|c| c.opacity == 1.0));
    }

    #[test]
    fn flip_changes_the_photo_only_while_edge_on() {
        let mut rig = Rig::new(PicasaTransition::Flip, 5, 0.9);
        for id in 0..6 { rig.load(id, true); }
        rig.run(3.0);
        let begin = rig.history.len();
        rig.replace(1, 5);
        rig.run(1.2);
        let mut swaps = 0;
        let mut previous: Option<usize> = None;
        for frame in &rig.history[begin..] {
            assert_eq!(frame.len(), 5, "exactly one card per slot");
            let slot_one = frame.iter().find(|c| c.id == 1 || c.id == 5).unwrap();
            if let Some(id) = previous { if id != slot_one.id {
                swaps += 1;
                assert!(slot_one.scale_x < 0.12, "swapped while visibly open: {}", slot_one.scale_x);
            }}
            previous = Some(slot_one.id);
        }
        assert_eq!(swaps, 1);
    }

    #[test]
    fn cut_replaces_immediately_without_an_outgoing_print() {
        let mut rig = Rig::new(PicasaTransition::Cut, 5, 0.9);
        for id in 0..6 { rig.load(id, true); }
        rig.run(3.0);
        rig.replace(3, 5);
        rig.frame();
        let frame = rig.history.last().unwrap();
        assert_eq!(frame.len(), 5);
        assert!(frame.iter().any(|c| c.id == 5 && c.opacity == 1.0 && c.scale_x == 1.0));
        assert!(frame.iter().all(|c| c.id != 3));
    }

    #[test]
    fn a_slow_frame_cannot_make_the_animation_jump_and_pause_freezes_it() {
        let mut model: Model<Fake> = Model::new(1, PicasaTransition::Crossfade);
        model.advance(1_000_000, false);
        model.advance(1_500_000, false); // 500 ms stall
        assert!((model.clock - PILE_MAX_DT).abs() < 1e-9);
        model.advance(1_516_667, true);
        assert!((model.clock - PILE_MAX_DT).abs() < 1e-9, "paused clock must not advance");
        model.advance(1_500_000, false); // time going backwards is ignored
        assert!((model.clock - PILE_MAX_DT).abs() < 1e-9);
    }

    #[test]
    fn any_tile_count_lays_out_on_screen_with_each_photos_own_aspect() {
        for style in [Style::PicturePile, Style::Floating] {
            for (w, h) in [(1920.0, 1080.0), (1080.0, 1920.0), (1024.0, 768.0), (3840.0, 2160.0)] {
                for count in [1, 2, 5, 6, 7, 9, 12, 20, 36, 64] {
                    let mut prints = Vec::new();
                    for slot in 0..count {
                        for aspect in [0.67, 1.0, 1.5, 1.78] {
                            let g = print_geometry(style, slot, count, aspect, w, h);
                            let tilt = (3.0f64).to_radians();
                            let hw = 1.04 * (g.w * 0.5 * tilt.cos() + g.h * 0.5 * tilt.sin()) + 3.0;
                            let hh = 1.04 * (g.w * 0.5 * tilt.sin() + g.h * 0.5 * tilt.cos()) + 3.0;
                            assert!(g.cx - hw >= -0.5 && g.cx + hw <= w + 0.5, "{style:?} {w}x{h} n={count} slot {slot}");
                            assert!(g.cy - hh >= -0.5 && g.cy + hh <= h + 0.5, "{style:?} {w}x{h} n={count} slot {slot}");
                            let inner = (g.w - 2.0 * g.border) / (g.h - 2.0 * g.border);
                            assert!((inner / aspect - 1.0).abs() < 1e-9);
                        }
                        prints.push(print_geometry(style, slot, count, 1.5, w, h));
                    }
                    if style == Style::Floating && count > FLOATING_SLOTS {
                        for (i, a) in prints.iter().enumerate() {
                            for b in prints.iter().skip(i + 1) {
                                let apart = (a.cx - b.cx).abs() >= (a.w + b.w) * 0.5
                                    || (a.cy - b.cy).abs() >= (a.h + b.h) * 0.5;
                                assert!(apart, "floating prints overlap at n={count} {w}x{h}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn floating_prints_drift_continuously_without_a_reset_at_photo_changes() {
        let mut rig = Rig::new(PicasaTransition::Crossfade, 6, 0.9);
        rig.model = Model::with_style(6, PicasaTransition::Crossfade, Style::Floating);
        for id in 0..12 { rig.load(id, true); }
        rig.run(2.5);
        let begin = rig.history.len();
        rig.run(1.0);
        rig.replace(2, 7);
        rig.run(1.5);
        // Never idle, and the resting position of every slot moves by tiny,
        // smooth steps, including across the moment a photo is replaced.
        assert_eq!(rig.history.len() - begin, 150, "drift must repaint every frame");
        let mut last: HashMap<usize, (f64, f64, f64)> = HashMap::new();
        for frame in rig.history.iter().skip(begin) {
            let mut current = HashMap::new();
            for card in frame.iter() {
                if let Some(&(x, y, a)) = last.get(&card.id) {
                    assert!((card.geometry.cx - x).abs() < 0.25, "drift x jumped");
                    assert!((card.geometry.cy - y).abs() < 0.25, "drift y jumped");
                    assert!((card.angle - a).abs() < 0.02, "drift angle jumped");
                }
                current.insert(card.id, (card.geometry.cx, card.geometry.cy, card.angle));
            }
            last = current;
        }
    }

    #[test]
    fn floating_prints_stay_on_screen_with_drift_and_landing() {
        for (w, h) in [(1920.0, 1080.0), (1080.0, 1920.0), (1024.0, 768.0)] {
            for slot in 0..FLOATING_SLOTS {
                for aspect in [0.6, 1.0, 1.5, 2.0] {
                    let g = print_geometry(Style::Floating, slot, FLOATING_SLOTS, aspect, w, h);
                    let tilt = (FLOATING_TILT.iter().fold(0.0f64, |m, t| m.max(t.abs()))
                        + FLOATING_TILT_SWAY + 0.5).to_radians();
                    let half_w = 1.02 * (g.w * 0.5 * tilt.cos() + g.h * 0.5 * tilt.sin()) + FLOATING_DRIFT_X;
                    let half_h = 1.02 * (g.w * 0.5 * tilt.sin() + g.h * 0.5 * tilt.cos()) + FLOATING_DRIFT_Y;
                    assert!(g.cx - half_w >= 0.0 && g.cx + half_w <= w);
                    assert!(g.cy - half_h >= 0.0 && g.cy + half_h <= h);
                }
            }
        }
    }

    #[test]
    fn a_missing_frame_leaves_the_slot_alone_until_it_arrives() {
        let mut rig = Rig::new(PicasaTransition::Crossfade, 5, 0.9);
        for id in 0..5 { rig.load(id, true); }
        rig.run(2.0);
        rig.replace(2, 9); // photo 9 has not decoded yet
        rig.run(0.5);
        assert!(rig.history.last().unwrap().iter().any(|c| c.id == 2 && c.opacity == 1.0));
        rig.load(9, true);
        rig.run(1.5);
        let last = rig.history.last().unwrap();
        assert!(last.iter().any(|c| c.id == 9) && last.iter().all(|c| c.id != 2));
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    fn synth(w: u32, h: u32, seed: u32) -> Pixels {
        let mut img = image::RgbaImage::new(w, h);
        let hues = [(220u32, 90u32, 70u32), (60, 140, 200), (90, 180, 110), (200, 160, 60), (150, 90, 190), (70, 70, 90)];
        let (r, g, b) = hues[seed as usize % hues.len()];
        for (x, y, p) in img.enumerate_pixels_mut() {
            let t = y as f32 / h as f32;
            let stripe = if ((x / (w / 12).max(1)) + (y / (h / 12).max(1))) % 2 == 0 { 18 } else { 0 };
            *p = image::Rgba([
                ((r as f32 * (1.0 - 0.5 * t)) as u32 + stripe).min(255) as u8,
                ((g as f32 * (1.0 - 0.5 * t)) as u32 + stripe).min(255) as u8,
                ((b as f32 * (1.0 - 0.5 * t)) as u32 + stripe).min(255) as u8, 255]);
        }
        // a "horizon" and a number-ish mark so cards are distinguishable
        for x in 0..w { for y in (h * 3 / 5)..(h * 3 / 5 + 4) { img.put_pixel(x, y, image::Rgba([255, 255, 255, 255])); } }
        for k in 0..=(seed % 5) { for x in (20 + k * 40)..(40 + k * 40) { for y in 20..60 { if x < w && y < h { img.put_pixel(x, y, image::Rgba([255, 255, 255, 255])); } } } }
        texture_pixels(img)
    }
    fn pump() { let c = glib::MainContext::default(); while c.pending() { c.iteration(false); } }

    /// Smoke test + optional PNG dump: PIC_PILE_PNG_DIR=/tmp/out
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn real_canvas_paints_a_full_pile_cycle() {
        libadwaita::init().unwrap();
        install_css();
        let dims = [(1500u32, 1000u32), (1000, 1500), (1200, 1200), (1600, 900), (900, 1400), (1500, 1000),
                    (1000, 1500), (1600, 900), (1200, 1200), (1500, 1000), (1000, 1500), (1600, 900)];
        for (style, mode) in [
            (Style::PicturePile, PicasaTransition::Crossfade), (Style::PicturePile, PicasaTransition::Flip),
            (Style::PicturePile, PicasaTransition::Cut), (Style::Floating, PicasaTransition::Crossfade),
            (Style::Floating, PicasaTransition::Flip),
        ] {
            let wanted: usize = std::env::var("PIC_PILE_TILES").ok().and_then(|v| v.parse().ok())
                .unwrap_or(if style == Style::Floating { 6 } else { 5 });
            let mut player = Player::new(style, 80, 4, false, wanted);
            let slots = player.slots.len();
            player.transition = Duration::from_millis(900);
            let mut stage = Stage::new(style, slots, mode);
            let window = gtk::Window::builder().default_width(1280).default_height(720).child(&stage.root).build();
            window.present(); pump();
            let (tx, rx) = mpsc::channel();
            let mut frame_time = 1_000_000i64;
            let out = std::env::var("PIC_PILE_PNG_DIR").ok();
            let mut shots = 0;
            for frame in 0..420 {
                while let Ok(i) = rx.try_recv() {
                    let (w, h) = dims[i % 12];
                    player.receive(ImageResult { index: i, high_quality: false, pixels: Some(synth(w / 8, h / 8, i as u32)) });
                    player.receive(ImageResult { index: i, high_quality: true, pixels: Some(synth(w / 2, h / 2, i as u32)) });
                }
                if frame == 150 || frame == 330 { player.switched_at -= Duration::from_secs(10); }
                player.update(&tx);
                frame_time += 16_667;
                stage.sync(&player, frame_time);
                pump();
                if let (Some(dir), true) = (&out, [60, 160, 175, 190, 345, 360, 400].contains(&frame)) {
                    let paintable = gtk::WidgetPaintable::new(Some(&stage.root));
                    let snap = gtk::Snapshot::new();
                    paintable.snapshot(&snap, 1280.0, 720.0);
                    if let (Some(node), Some(renderer)) = (snap.to_node(), window.native().and_then(|n| n.renderer())) {
                        let texture = renderer.render_texture(&node, None);
                        let _ = texture.save_to_png(format!("{dir}/{style:?}_{mode:?}_{frame}.png"));
                        shots += 1;
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            // A change may still be landing (one extra outgoing print) on a slow display.
            assert!(stage.pile.as_ref().unwrap().canvas.painted().len() >= slots, "{style:?} {mode:?}");
            if out.is_some() { assert!(shots > 0); }
        }
    }
}
