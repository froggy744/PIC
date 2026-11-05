//! Flying Photos: photographs glide out of the distance and past the viewer,
//! in the spirit of the classic Windows flying-images screensaver, using PIC's
//! own library sources, decode workers, caches and edit recipes.
//!
//! Why the perspective is computed in Rust instead of with a GSK matrix:
//! a photograph that faces the camera projects to a *uniformly scaled*
//! rectangle (on-screen size = focal / depth), so once Rust has done the depth
//! maths the projection is exact. The snapshot then needs translate + rotate +
//! scale only, which renders identically through GSK's GL and Cairo renderers.
//! A `gsk::Transform` carrying a perspective matrix is flattened by the Cairo
//! renderer, which would break CPU/GPU parity and the xvfb test harness.
//!
//! One widget, one snapshot, analytic motion (the same rule pile.rs follows):
//!
//! * the frame clock's clamped delta drives the flight, so one slow frame
//!   (decode result, texture upload) cannot make a photograph jump;
//! * every flight owns its textures. A preview -> original upgrade cross-fades
//!   in place and never restarts, resizes or moves the flight;
//! * launches come from the existing `Player` slots, so this style reuses the
//!   source selection, portrait filter, prefetch window, failed-photo backfill
//!   and texture LRU instead of duplicating them;
//! * a photograph is never replaced out of the air: when the player assigns a
//!   new photo to a slot, the print already in flight keeps flying until it
//!   has passed the camera. New prints enter from the distance instead.
use gtk::subclass::prelude::*;
use gtk::{gdk, graphene, gsk};

use super::pile::Aspect;
use super::*;

/// Launch depth in world units.
const FAR_DEPTH: f64 = 22.0;
/// Depth range at which prints pass the camera. Each print gets its own pass
/// depth, and that is where the field's depth variety comes from: a print
/// passing at the near end rushes past the lens enormous, a print passing at
/// the far end stays a medium print.
const NEAR_DEPTH_RANGE: (f64, f64) = (0.40, 1.80);
/// Magnification of the closest-passing print at the end of its flight.
/// Long edge at launch, as a fraction of the shorter screen edge. A print
/// enters as a small distant photograph and ends up several times taller than
/// the monitor, so it passes *through* the viewport instead of floating in it.
const LAUNCH_SPAN: f64 = 0.10;
/// Real print-size variety: not every photograph is the same physical size.
const PRINT_SIZE_RANGE: (f64, f64) = (0.70, 1.30);
/// How far beyond the monitor edge a print travels before its flight ends, as
/// a multiple of the distance from its vanishing point to that edge along the
/// print's own heading. It is derived per print instead of chosen as a launch
/// offset, so every print leaves through a real boundary and the field covers
/// the whole viewport, corners included, instead of clustering on the
/// vanishing point.
/// How much of a print's outward travel follows the perspective law and how
/// much advances steadily with time. Pure perspective drift is proportional to
/// magnification, which bunches every print around the vanishing point until
/// the last quarter of its flight; the blend is what spreads a fly-past across
/// the whole screen while the print is still at medium distance.
/// Resting roll of a print, degrees.
const ROLL_RANGE: (f64, f64) = (-0.5, 0.5);
/// Wobble amplitude (degrees) and period (seconds).
const WOBBLE_AMPLITUDE: (f64, f64) = (0.1, 0.3);
const WOBBLE_PERIOD: (f64, f64) = (5.0, 12.0);
/// Gentle sideways drift, so flights are not straight lines.
const SWAY_PERIOD: (f64, f64) = (4.0, 9.0);
/// Each print travels up to this much faster or slower than its neighbours.
const SPEED_VARIATION: f64 = 0.22;
/// Aerial perspective: distant prints are slightly dimmed, approaching ones
/// reach full brightness.
const HAZE_AT_DISTANCE: f64 = 0.80;
/// Entry fade bounds (seconds) and its ceiling as a share of the whole flight,
/// which is what makes a print fade in while it is still tiny in the distance.
const ENTRY_SECONDS: (f64, f64) = (0.4, 2.5);
const ENTRY_MAX_FRACTION: f64 = 0.30;
/// Tail fade: a print that is still on screen when its flight ends fades out
/// instead of blinking off.
const EXIT_FADE_FRACTION: f64 = 0.14;
/// Longest animation step accepted from one frame.
const FLY_MAX_DT: f64 = 0.05;
/// Preview -> original cross-fade time.
const FLY_UPGRADE_SECONDS: f64 = 0.35;
/// Stagger between launches that refill one slot at a time.
/// Progress spread used to seed a cold flight field, as a fraction of a flight.
const WARM_START: (f64, f64) = (0.05, 0.80);
/// Hard bound on simultaneous flights, and therefore on live textures.
pub(super) const FLY_MAX_FLIGHTS: usize = 40;

/// Perspective magnification of a camera-facing plane.
///
/// Depth falls linearly with time (a constant closing speed), so magnification
/// `FAR_DEPTH / depth` accelerates exactly like a real camera approach: slow
/// while the print is far away, then a rush past the lens. `near` is this
/// print's own pass depth, which is why prints differ in how close they come.
fn magnification(progress: f64, near: f64) -> f64 {
    let p = progress.clamp(0.0, 1.0);
    let depth = FAR_DEPTH - (FAR_DEPTH - near) * p;
    FAR_DEPTH / depth
}

/// Seconds a print takes to travel from the distance to the camera, and so how
/// often a new one enters. It is bounded from both sides: too short and prints
/// never fill the screen, too long and the flight stops being a fly-through.
fn flight_seconds(launch_period: f64, slots: usize) -> f64 {
    (launch_period.max(1.0) * slots.max(1) as f64).clamp(6.0, 30.0)
}

/// Which point in its own flight a seed print starts at. Slots walk the
/// golden-ratio sequence, so a cold field is seeded with prints at many
/// different distances at once instead of a ramp of identical young prints.
fn warm_start_progress(slot: usize) -> f64 {
    let step = ((slot as f64 + 1.0) * 0.618_033_988_749_895).fract();
    WARM_START.0 + (WARM_START.1 - WARM_START.0) * step
}

/// Entry fade length. It borrows the screensaver's existing "Fade (s)" setting
/// and is capped to a fraction of the flight, so a print is always fully
/// visible long before it reaches the viewer.
fn entry_seconds(user_fade: f64, flight: f64) -> f64 {
    user_fade
        .clamp(ENTRY_SECONDS.0, ENTRY_SECONDS.1)
        .min(flight * ENTRY_MAX_FRACTION)
        .max(0.2)
}

fn haze(closeness: f64) -> f64 {
    HAZE_AT_DISTANCE + (1.0 - HAZE_AT_DISTANCE) * closeness.clamp(0.0, 1.0)
}

/// Opacity of a print at one point in its flight: smooth fade in from nothing,
/// a plain middle, and a smooth fade out for prints that never left the frame.
fn flight_opacity(progress: f64, entry: f64) -> f64 {
    let p = progress.clamp(0.0, 1.0);
    let entering = smootherstep(p / entry.max(1e-6));
    let leaving = 1.0 - smootherstep((p - (1.0 - EXIT_FADE_FRACTION)) / EXIT_FADE_FRACTION);
    entering * leaving.max(0.0)
}

/// On-screen print size at one magnification. The long edge is what the world
/// size drives, so portrait photographs grow along their own long axis and
/// nothing is ever stretched or cropped.
fn flight_size(aspect: f64, size_seed: f64, magnification: f64, span: f64) -> (f64, f64) {
    let ratio = aspect.max(0.05);
    let long = LAUNCH_SPAN * span * size_seed * magnification;
    if ratio >= 1.0 {
        (long, long / ratio)
    } else {
        (long * ratio, long)
    }
}

/// Deterministic flight seeds. No RNG: the same photo and launch number always
/// take the same path, which keeps the animation reproducible and testable.
fn seed_for(id: usize, launch: usize) -> u64 {
    let mut x = (id as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((launch as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
        .wrapping_add(0x2545_F491_4F6C_DD1D);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn rand_unit(seed: u64, lane: u64) -> f64 {
    let mut x = seed
        .wrapping_add(lane.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_mul(0xFF51_AF7D_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^= x >> 29;
    (x as f64) / (u64::MAX as f64)
}

fn rand_range(seed: u64, lane: u64, from: f64, to: f64) -> f64 {
    from + (to - from) * rand_unit(seed, lane)
}

/// Golden-angle / golden-ratio steps used to spread headings, sizes and pass
/// depths over consecutive launches, so the field never fills with prints that
/// all entered at the same size or along the same bearing.
const PHI_A: f64 = 0.618_033_988_749_895;
const PHI_B: f64 = 0.754_877_666_246_693;
const PHI_C: f64 = 0.370_436_069_010_204;
const GOLDEN_ANGLE: f64 = 2.399_963_229_728_662;

/// Low-discrepancy sampling of one property of one print: the launch number and
/// the photo both advance a different golden-ratio sequence, then a bounded
/// hash jitter breaks the regularity. Independent flights that together cover
/// the full range, without an RNG.
fn stratified(seed: u64, lane: u64, id: usize, launch: usize, from: f64, to: f64) -> f64 {
    let step = ((id as f64) * PHI_A + (launch as f64) * PHI_B + (lane as f64) * PHI_C).fract();
    let t = (step + rand_unit(seed, lane) * 0.45).min(1.0);
    from + (to - from) * t
}

/// Screen-space anchor coordinates use different irrational steps from the
/// heading and flight phase. This keeps a twenty-photo field from lining up
/// along the same few spokes while still distributing it without a grid.
fn spread(seed: u64, lane: u64, _id: usize, _launch: usize) -> f64 {
    rand_unit(seed, lane)
}

/// Everything that makes one print's flight its own: approach direction from
/// the vanishing point (left, right, top, bottom and the corners are all used),
/// how far beyond the screen edge it flies, its own pass depth and therefore
/// its own maximum size, its physical size, roll, wobble, sway and speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Attitude {
    pub dir: (f64, f64),
    pub vanishing_point: (f64, f64),
    pub near: f64,
    pub mag: f64,
    pub size_seed: f64,
    pub roll: f64,
    pub wobble_amp: f64,
    pub wobble_period: f64,
    pub wobble_phase: f64,
    pub sway_period: f64,
    pub speed: f64,
}

fn attitude(id: usize, launch: usize) -> Attitude {
    let seed = seed_for(id, launch);
    // Headings step by the golden angle, which never repeats a bearing between
    // consecutive launches and lands in every quadrant and every corner, plus a
    // bounded jitter so the field is not a wheel of spokes.
    let angle = ((id as f64 * PHI_A + launch as f64) * GOLDEN_ANGLE
        + (rand_unit(seed, 3) - 0.5) * 0.9)
        .rem_euclid(std::f64::consts::TAU);
    let dir = (angle.cos(), angle.sin());
    let near = stratified(seed, 1, id, launch, NEAR_DEPTH_RANGE.0, NEAR_DEPTH_RANGE.1);
    Attitude {
        dir,
        // A slightly wandering vanishing point breaks up the central pile-up
        // while keeping each photo's path coherent and perspective-like.
        vanishing_point: (
            -0.43 + spread(seed, 10, id, launch) * 0.86,
            -0.38 + spread(seed, 11, id, launch) * 0.76,
        ),
        near,
        mag: magnification(1.0, near),
        size_seed: stratified(seed, 2, id, launch, PRINT_SIZE_RANGE.0, PRINT_SIZE_RANGE.1),
        roll: rand_range(seed, 9, ROLL_RANGE.0, ROLL_RANGE.1),
        wobble_amp: rand_range(seed, 4, WOBBLE_AMPLITUDE.0, WOBBLE_AMPLITUDE.1),
        wobble_period: rand_range(seed, 5, WOBBLE_PERIOD.0, WOBBLE_PERIOD.1),
        wobble_phase: rand_unit(seed, 8) * std::f64::consts::TAU,
        sway_period: rand_range(seed, 6, SWAY_PERIOD.0, SWAY_PERIOD.1),
        speed: rand_range(seed, 7, 1.0 - SPEED_VARIATION, 1.0 + SPEED_VARIATION),
    }
}

/// Where one print is at one moment: projected centre, size, roll, opacity and
/// how close to the camera it is (0 = at the vanishing point, 1 = past it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Pose {
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    pub angle: f64,
    pub opacity: f64,
    pub closeness: f64,
}

/// `entry` is the fade-in length as a fraction of this print's own flight.
fn flight_pose(
    way: &Attitude,
    aspect: f64,
    progress: f64,
    age: f64,
    entry: f64,
    size: (f64, f64),
) -> Pose {
    let p = progress.clamp(0.0, 1.0);
    let zoom = magnification(p, way.near);
    let span = size.0.min(size.1);
    let (w, h) = flight_size(aspect, way.size_seed, zoom, span);
    let origin = (
        size.0 * (0.5 + way.vanishing_point.0),
        size.1 * (0.5 + way.vanishing_point.1),
    );
    // Lateral position runs from beside the vanishing point at launch to beyond
    // the monitor edge at the end of the flight, on the print's own bearing.
    // This is what distributes the field over the whole screen instead of
    // around the centre, and what makes prints leave the frame naturally.
    // A fixed position in the scene projects away from the common camera
    // centre with the same perspective magnification as its size.
    let projection = zoom;
    let wobble =
        way.wobble_amp * (std::f64::consts::TAU * age / way.wobble_period + way.wobble_phase).sin();
    Pose {
        cx: size.0 * 0.5 + (origin.0 - size.0 * 0.5) * projection,
        cy: size.1 * 0.5 + (origin.1 - size.1 * 0.5) * projection,
        w,
        h,
        angle: way.roll + wobble,
        opacity: flight_opacity(p, entry) * haze(p),
        closeness: p,
    }
}

#[derive(Clone)]
struct Flight<T> {
    slot: usize,
    id: usize,
    texture: T,
    upgrade: Option<(T, f64)>,
    quality: bool,
    aspect: f64,
    way: Attitude,
    born: f64,
    duration: f64,
    /// Entry fade as a fraction of this print's own flight.
    entry: f64,
}

/// One print as the canvas must paint it on this frame.
#[derive(Clone, Debug)]
pub(super) struct DrawFlight<T> {
    #[cfg(test)]
    pub id: usize,
    pub slot: usize,
    pub texture: T,
    /// Sharper original fading in over a cached preview, with its opacity.
    pub upgrade: Option<(T, f64)>,
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    pub angle: f64,
    pub opacity: f64,
    /// 0.0 at the vanishing point, 1.0 past the camera. Used to paint distant
    /// prints first and to scale their shadows.
    pub closeness: f64,
}

/// What the flight engine needs from the player at one frame.
pub(super) struct Feed<'a, T> {
    pub slots: &'a [usize],
    pub changes: usize,
    pub replacing: Option<(usize, usize)>,
    /// Seconds between two launches (the player's photo interval).
    pub launch_period: f64,
    /// Entry fade seconds (the player's transition setting).
    pub entry: f64,
    /// Texture and "is original quality" for a photo, when it is loaded.
    pub frame: &'a dyn Fn(usize) -> Option<(T, bool)>,
}

pub(super) struct Model<T> {
    flights: Vec<Flight<T>>,
    seeded_slots: HashSet<usize>,
    clock: f64,
    last_frame: Option<i64>,
    seen_changes: usize,
    launches: usize,
    size: (f64, f64),
}

impl<T: Clone + Aspect> Model<T> {
    pub fn new() -> Self {
        Self {
            flights: Vec::new(),
            seeded_slots: HashSet::new(),
            clock: 0.0,
            last_frame: None,
            seen_changes: 0,
            launches: 0,
            size: (0.0, 0.0),
        }
    }

    /// Advance the flight clock by the frame clock's delta (microseconds).
    pub fn advance(&mut self, frame_time_us: i64, paused: bool) {
        let dt = self
            .last_frame
            .map(|previous| ((frame_time_us - previous) as f64 / 1e6).clamp(0.0, FLY_MAX_DT))
            .unwrap_or(0.0);
        self.last_frame = Some(frame_time_us);
        if !paused {
            self.clock += dt;
        }
    }

    /// Send the photo in a slot off into the distance.
    ///
    /// `overlap` is true only for a slot the player has just re-assigned: its
    /// new print enters from the distance while the previous print carries on
    /// flying. The steady-state refill path uses `overlap = false`, so one slot
    /// can never launch the same photograph twice.
    ///
    /// `warm` seeds a print already part-way through its own flight, which is
    /// how a cold field is populated on the very first frame.
    fn launch(
        &mut self,
        feed: &Feed<T>,
        slot: usize,
        base: f64,
        entry: f64,
        delay: f64,
        warm: f64,
        overlap: bool,
    ) -> bool {
        let Some(&id) = feed.slots.get(slot) else {
            return false;
        };
        if !overlap && self.flights.iter().any(|flight| flight.slot == slot) {
            return false;
        }
        let Some((texture, quality)) = (feed.frame)(id) else {
            return false;
        };
        if self.flights.len() >= FLY_MAX_FLIGHTS {
            // The player is changing slots faster than prints can pass the
            // camera. Retire the print closest to the camera first: it is the
            // one furthest along, so dropping it is the least visible.
            let closest = self.flights.iter().enumerate().min_by_key(|(_, flight)| {
                ((flight.born + flight.duration - self.clock) * 1000.0) as i64
            });
            if let Some(index) = closest.map(|(index, _)| index) {
                self.flights.remove(index);
            }
        }
        let launch = self.launches;
        self.launches += 1;
        let mut way = attitude(id, launch);
        // Seed foreground photos inside the viewport as their textures arrive.
        let projection = 1.0 + (magnification(warm, way.near) - 1.0) * 0.32;
        way.vanishing_point.0 /= projection;
        way.vanishing_point.1 /= projection;
        let duration = (base / way.speed).max(1.0);
        self.flights.push(Flight {
            slot,
            id,
            aspect: texture.aspect(),
            texture,
            upgrade: None,
            quality,
            way,
            born: self.clock + delay - warm * duration,
            duration,
            entry: (entry / duration).clamp(0.02, ENTRY_MAX_FRACTION),
        });
        true
    }

    /// Reconcile with the player and return the prints to paint, far ones
    /// first, so approaching photographs overlap the distant ones naturally.
    pub fn update(&mut self, feed: &Feed<T>, size: (f64, f64)) -> Vec<DrawFlight<T>> {
        self.size = size;
        let base = flight_seconds(feed.launch_period, feed.slots.len());
        let entry = entry_seconds(feed.entry, base);

        // Flights that have passed the camera are recycled.
        self.flights
            .retain(|flight| self.clock - flight.born < flight.duration);

        // A change the player just made: the print already in the air keeps
        // flying, and the new photo starts its approach from the distance.
        if feed.changes != self.seen_changes {
            self.seen_changes = feed.changes;
            if let Some((slot, _former)) = feed.replacing {
                self.launch(feed, slot, base, entry, 0.0, 0.0, true);
            }
        }

        // First fill, and any slot whose flight has finished while the player
        // kept the same photograph: it flies again. This is what makes new
        // prints enter continuously instead of the animation settling.
        //
        // A cold field is seeded mid-flight. Launching everything at the
        // vanishing point would leave the monitor almost empty for the first
        // seconds of the screensaver; spreading the first launches through
        // their own flights puts distant, mid-distance and near-camera prints
        // on screen in the very first frame, at the same cadence they will
        // keep for the rest of the run.
        for slot in 0..feed.slots.len() {
            let warm = if self.seeded_slots.contains(&slot) {
                0.0
            } else {
                warm_start_progress(slot)
            };
            if self.launch(feed, slot, base, entry, 0.0, warm, false) {
                self.seeded_slots.insert(slot);
            }
        }

        // A sharper original fades in over a cached preview in place; the
        // flight itself is never restarted, resized or moved.
        for flight in &mut self.flights {
            if flight.quality || flight.upgrade.is_some() {
                continue;
            }
            if let Some((texture, true)) = (feed.frame)(flight.id) {
                let clock = self.clock;
                flight.upgrade = Some((texture, clock));
            }
        }
        for flight in &mut self.flights {
            if flight
                .upgrade
                .as_ref()
                .is_some_and(|(_, since)| self.clock - since >= FLY_UPGRADE_SECONDS)
            {
                if let Some((texture, _)) = flight.upgrade.take() {
                    flight.texture = texture;
                    flight.quality = true;
                }
            }
        }

        self.compose()
    }

    fn compose(&self) -> Vec<DrawFlight<T>> {
        let mut cards: Vec<DrawFlight<T>> = Vec::with_capacity(self.flights.len());
        for flight in self.flights.iter() {
            let age = self.clock - flight.born;
            if age < 0.0 {
                continue; // launched with a stagger: not on screen yet
            }
            let progress = (age / flight.duration).clamp(0.0, 1.0);
            let pose = flight_pose(
                &flight.way,
                flight.aspect,
                progress,
                age,
                flight.entry,
                self.size,
            );
            if pose.opacity <= 0.002 || pose.w < 1.0 || pose.h < 1.0 {
                continue;
            }
            let upgrade = flight.upgrade.as_ref().map(|(texture, since)| {
                (
                    texture.clone(),
                    smootherstep((self.clock - since) / FLY_UPGRADE_SECONDS),
                )
            });
            cards.push(DrawFlight {
                #[cfg(test)]
                id: flight.id,
                slot: flight.slot,
                texture: flight.texture.clone(),
                upgrade,
                cx: pose.cx,
                cy: pose.cy,
                w: pose.w,
                h: pose.h,
                angle: pose.angle,
                opacity: pose.opacity,
                closeness: pose.closeness,
            });
        }
        cards.sort_by(|a, b| {
            a.closeness
                .total_cmp(&b.closeness)
                .then(a.slot.cmp(&b.slot))
        });
        cards
    }
}

// ---------------------------------------------------------------------------
// GTK side: one widget, one snapshot.
// ---------------------------------------------------------------------------

mod canvas_imp {
    use super::*;

    #[derive(Default)]
    pub struct FlyingCanvas {
        pub(super) background_colour: Cell<Option<gdk::RGBA>>,
        pub(super) cards: RefCell<Vec<DrawFlight<gdk::Texture>>>,
    }
    #[gtk::glib::object_subclass]
    impl ObjectSubclass for FlyingCanvas {
        const NAME: &'static str = "PicFlyingCanvas";
        type Type = super::FlyingCanvas;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for FlyingCanvas {}
    impl WidgetImpl for FlyingCanvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let (w, h) = (obj.width() as f32, obj.height() as f32);
            let bounds = graphene::Rect::new(0.0, 0.0, w, h);
            // Full-screen black: the prints and their shadows are the picture.
            snapshot.append_color(
                &self
                    .background_colour
                    .get()
                    .unwrap_or(gdk::RGBA::new(0.90, 0.90, 0.88, 1.0)),
                &bounds,
            );
            snapshot.push_clip(&bounds);
            for card in self.cards.borrow().iter() {
                paint_flight(snapshot, card);
            }
            snapshot.pop();
        }
    }
}

gtk::glib::wrapper! {
    pub struct FlyingCanvas(ObjectSubclass<canvas_imp::FlyingCanvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FlyingCanvas {
    fn new() -> Self {
        let canvas: Self = gtk::glib::Object::new();
        canvas.set_can_target(false);
        canvas.set_focusable(false);
        canvas
    }
    fn set_cards(&self, cards: Vec<DrawFlight<gdk::Texture>>) {
        *self.imp().cards.borrow_mut() = cards;
        self.queue_draw();
    }
    #[cfg(test)]
    pub(super) fn painted(&self) -> Vec<DrawFlight<gdk::Texture>> {
        self.imp().cards.borrow().clone()
    }
}

/// Draw one print. Its rectangle already matches the photograph's own aspect
/// ratio exactly, so the texture is scaled (mip-mapped, or a moving 2 MP photo
/// shimmers) and never cover-cropped or stretched.
fn paint_flight(snapshot: &gtk::Snapshot, card: &DrawFlight<gdk::Texture>) {
    if card.opacity <= 0.002 {
        return;
    }
    let (w, h) = (card.w.max(1.0) as f32, card.h.max(1.0) as f32);
    snapshot.save();
    snapshot.translate(&graphene::Point::new(card.cx as f32, card.cy as f32));
    snapshot.rotate(card.angle as f32);
    snapshot.translate(&graphene::Point::new(-w * 0.5, -h * 0.5));
    snapshot.push_opacity(card.opacity.clamp(0.0, 1.0));
    let bounds = graphene::Rect::new(0.0, 0.0, w, h);
    snapshot.append_scaled_texture(&card.texture, gsk::ScalingFilter::Trilinear, &bounds);
    if let Some((texture, alpha)) = &card.upgrade {
        snapshot.push_opacity(alpha.clamp(0.0, 1.0));
        snapshot.append_scaled_texture(
            texture,
            gsk::ScalingFilter::Trilinear,
            &graphene::Rect::new(0.0, 0.0, w, h),
        );
        snapshot.pop();
    }
    snapshot.pop();
    snapshot.restore();
}

pub(super) struct Flying {
    capture: Option<(std::path::PathBuf, Instant, usize)>,
    model: Model<gdk::Texture>,
    pub canvas: FlyingCanvas,
    size: (i32, i32),
}

impl Flying {
    pub(super) fn set_background_colour(&self, colour: gdk::RGBA) {
        self.canvas.imp().background_colour.set(Some(colour));
        self.canvas.queue_draw();
    }
    pub fn new() -> Self {
        Self {
            capture: std::env::var_os("PIC_FLYING_CAPTURE_DIR")
                .map(|dir| (dir.into(), Instant::now(), 0)),
            model: Model::new(),
            canvas: FlyingCanvas::new(),
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
        let lookup = |id: usize| {
            player
                .frames
                .get(&id)
                .map(|surface| (surface.texture.clone(), surface.high_quality))
        };
        let feed = Feed {
            slots: &player.slots,
            changes: player.changes,
            replacing: player.fading_slot,
            launch_period: player.interval.as_secs_f64(),
            entry: player.transition.as_secs_f64(),
            frame: &lookup,
        };
        // Motion is continuous, so the widget is repainted on every frame.
        let cards = self.model.update(&feed, (w, h));
        if let Some((dir, started, index)) = self.capture.as_mut() {
            let times = [2, 5, 10];
            if *index < times.len() && started.elapsed().as_secs_f64() >= times[*index] as f64 {
                if let Some(renderer) = self.canvas.native().and_then(|n| n.renderer()) {
                    let snap = gtk::Snapshot::new();
                    let bounds = graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
                    snap.append_color(
                        &self
                            .canvas
                            .imp()
                            .background_colour
                            .get()
                            .unwrap_or(gdk::RGBA::new(0.90, 0.90, 0.88, 1.0)),
                        &bounds,
                    );
                    for card in &cards {
                        paint_flight(&snap, card);
                    }
                    if let Some(node) = snap.to_node() {
                        let texture = renderer.render_texture(&node, Some(&bounds));
                        let _ = std::fs::create_dir_all(&dir);
                        let path = dir.join(format!("live-{}s.png", times[*index]));
                        let result = texture.save_to_png(&path);
                        let visible = cards
                            .iter()
                            .filter(|c| {
                                c.cx + c.w / 2.0 > 0.0
                                    && c.cy + c.h / 2.0 > 0.0
                                    && c.cx - c.w / 2.0 < w
                                    && c.cy - c.h / 2.0 < h
                            })
                            .count();
                        eprintln!("PIC_FLYING_CAPTURE elapsed={:.3} viewport={}x{} slots={} loaded={} flights={} intersecting={} file={:?} result={:?}", started.elapsed().as_secs_f64(), w, h, player.slots.len(), player.frames.len(), cards.len(), visible, path, result);
                        *index += 1;
                    }
                }
            }
        }
        self.canvas.set_cards(cards);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Fake(usize, f64);
    impl Aspect for Fake {
        fn aspect(&self) -> f64 {
            self.1
        }
    }

    const ASPECTS: [f64; 6] = [1.5, 0.667, 1.0, 1.78, 0.75, 1.33];
    fn aspect_of(id: usize) -> f64 {
        ASPECTS[id % ASPECTS.len()]
    }
    const SCREEN: (f64, f64) = (1920.0, 1080.0);
    fn span() -> f64 {
        SCREEN.0.min(SCREEN.1)
    }

    /// Drives a `Model` at 60 fps against a scripted player feed.
    struct Rig {
        model: Model<Fake>,
        slots: Vec<usize>,
        changes: usize,
        replacing: Option<(usize, usize)>,
        loaded: HashSet<usize>,
        quality: HashMap<usize, bool>,
        time_us: i64,
        step_us: i64,
        launch_period: f64,
        entry: f64,
        paused: bool,
        history: Vec<Vec<DrawFlight<Fake>>>,
    }

    impl Rig {
        fn new(slots: usize, launch_period: f64, entry: f64) -> Self {
            Self {
                model: Model::new(),
                slots: (0..slots).collect(),
                changes: 0,
                replacing: None,
                loaded: HashSet::new(),
                quality: HashMap::new(),
                time_us: 1_000_000,
                step_us: 16_667,
                launch_period,
                entry,
                paused: false,
                history: Vec::new(),
            }
        }
        fn load(&mut self, id: usize, high_quality: bool) {
            self.loaded.insert(id);
            self.quality.insert(id, high_quality);
        }
        /// Replace one slot's photograph exactly as `Player::update` does.
        fn replace(&mut self, slot: usize, id: usize) {
            let former = std::mem::replace(&mut self.slots[slot], id);
            self.replacing = Some((slot, former));
            self.changes += 1;
        }
        fn frame(&mut self) -> Vec<DrawFlight<Fake>> {
            self.time_us += self.step_us;
            self.model.advance(self.time_us, self.paused);
            let loaded = self.loaded.clone();
            let quality = self.quality.clone();
            let lookup = move |id: usize| {
                loaded
                    .contains(&id)
                    .then(|| (Fake(id, aspect_of(id)), quality[&id]))
            };
            let slots = self.slots.clone();
            let feed = Feed {
                slots: &slots,
                changes: self.changes,
                replacing: self.replacing,
                launch_period: self.launch_period,
                entry: self.entry,
                frame: &lookup,
            };
            let cards = self.model.update(&feed, SCREEN);
            self.history.push(cards.clone());
            cards
        }
        fn run(&mut self, seconds: f64) {
            for _ in 0..(seconds * 60.0) as usize {
                self.frame();
            }
        }
    }

    #[test]
    fn magnification_accelerates_like_a_real_camera_approach() {
        for near in [NEAR_DEPTH_RANGE.0, 1.2, NEAR_DEPTH_RANGE.1] {
            assert!((magnification(0.0, near) - 1.0).abs() < 1e-12);
            assert!((magnification(1.0, near) - FAR_DEPTH / near).abs() < 1e-9);
            let mut previous = magnification(0.0, near);
            let mut previous_step = 0.0f64;
            for step in 1..=100 {
                let value = magnification(step as f64 / 100.0, near);
                let grown = value - previous;
                assert!(value > previous, "prints must keep approaching");
                assert!(grown > previous_step, "the approach must accelerate");
                previous_step = grown;
                previous = value;
            }
        }
        // Pass depth is what separates a print that stays at medium distance
        // from one that fills the screen, and the field needs both at once.
        assert!(
            magnification(1.0, NEAR_DEPTH_RANGE.0) > 1.8 * magnification(1.0, NEAR_DEPTH_RANGE.1)
        );
    }

    #[test]
    fn prints_enter_far_away_and_grow_past_the_viewer() {
        for aspect in [0.5, 0.75, 1.0, 1.5, 1.78] {
            for id in 0..24 {
                let way = attitude(id, 0);
                let (w, h) =
                    flight_size(aspect, way.size_seed, magnification(0.0, way.near), span());
                let long = w.max(h);
                assert!(long <= 0.20 * span(), "entered too large: {long}");
                assert!(long >= 0.08 * span(), "entered too small to see: {long}");
                let (w, h) =
                    flight_size(aspect, way.size_seed, magnification(1.0, way.near), span());
                let long = w.max(h);
                assert!(long >= 0.9 * span(), "never reached the viewer: {long}");
                if aspect < 1.0 {
                    assert!(h > w, "portrait prints grow along their own axis");
                } else {
                    assert!(w >= h);
                }
            }
        }
    }

    #[test]
    fn prints_leave_through_the_screen_edge_and_fill_the_whole_viewport() {
        // Every flight ends with the print past a real monitor boundary on its
        // own bearing, never fading out in the middle of the screen.
        for id in 0..60 {
            let way = attitude(id, id * 17 + 5);
            let pose = flight_pose(&way, 1.4, 1.0, 10.0, 0.05, SCREEN);
            assert!(
                pose.cx - pose.w * 0.5 < 0.0
                    || pose.cx + pose.w * 0.5 > SCREEN.0
                    || pose.cy - pose.h * 0.5 < 0.0
                    || pose.cy + pose.h * 0.5 > SCREEN.1
                    || pose.w.max(pose.h) > 1.5 * span(),
                "print {id} ended parked inside the frame at {:?}",
                (pose.cx, pose.cy, pose.w, pose.h)
            );
        }
        // A full field seen at one instant must put ink in all nine regions of
        // the screen: corners and edges included, not a column down the middle.
        for slots in [8usize, 12, 20] {
            let flight = flight_seconds(6.0, slots);
            let entry = entry_seconds(0.9, flight) / flight;
            let mut regions = [[false; 3]; 3];
            for index in 0..slots {
                let way = attitude(index, index * 17 + 5);
                let aspect = if index % 2 == 0 { 1.5 } else { 0.667 };
                let progress = (index as f64 + 0.5) / slots as f64;
                let pose = flight_pose(&way, aspect, progress, progress * flight, entry, SCREEN);
                let left = pose.cx - pose.w * 0.5;
                let right = pose.cx + pose.w * 0.5;
                let top = pose.cy - pose.h * 0.5;
                let bottom = pose.cy + pose.h * 0.5;
                for row in 0..3 {
                    let band_top = SCREEN.1 * row as f64 / 3.0;
                    let band_bottom = SCREEN.1 * (row + 1) as f64 / 3.0;
                    if top >= band_bottom || bottom <= band_top {
                        continue;
                    }
                    for column in 0..3 {
                        let band_left = SCREEN.0 * column as f64 / 3.0;
                        let band_right = SCREEN.0 * (column + 1) as f64 / 3.0;
                        if left < band_right && right > band_left {
                            regions[row][column] = true;
                        }
                    }
                }
            }
            let covered = regions
                .iter()
                .flat_map(|row| row.iter())
                .filter(|hit| **hit)
                .count();
            assert!(
                covered >= if slots <= 12 { 3 } else { 7 },
                "{slots} prints reached only {covered} of 9 screen regions: {regions:?}"
            );
        }
    }

    #[test]
    fn a_cold_field_starts_full_instead_of_empty() {
        let mut rig = Rig::new(12, 6.0, 0.9);
        for id in 0..12 {
            rig.load(id, true);
        }
        // One frame in, the screen must already hold prints at several
        // distances: the first screenshots of the screensaver are the ones a
        // user judges.
        let cards = rig.frame();
        assert!(
            cards.len() >= 10,
            "cold start left the field half empty: {}",
            cards.len()
        );
        let sizes: Vec<f64> = cards.iter().map(|c| c.w.max(c.h)).collect();
        let biggest = sizes.iter().copied().fold(0.0f64, f64::max);
        let smallest = sizes.iter().copied().fold(f64::INFINITY, f64::min);
        assert!(
            biggest > 2.0 * smallest,
            "no depth variety at start: {smallest} .. {biggest}"
        );
        assert!(biggest > 0.3 * span(), "no large print at start: {biggest}");
        let closeness: Vec<f64> = cards.iter().map(|c| c.closeness).collect();
        let far = closeness.iter().copied().fold(1.0f64, f64::min);
        let near = closeness.iter().copied().fold(0.0f64, f64::max);
        assert!(near - far > 0.5, "every print at the same distance");
        // And it stays full: no frame in the first minute is a black screen.
        rig.run(30.0);
        assert!(rig.history.iter().all(|frame| frame.len() >= 8));
    }

    #[test]
    fn every_print_keeps_its_own_aspect_ratio_across_the_whole_flight() {
        for aspect in [0.5, 0.667, 1.0, 1.5, 1.78] {
            for step in 0..=100 {
                let zoom = magnification(step as f64 / 100.0, 1.2);
                let (w, h) = flight_size(aspect, 1.0, zoom, span());
                assert!(((w / h) / aspect - 1.0).abs() < 1e-9, "stretched at {step}");
            }
        }
    }

    #[test]
    fn a_print_fades_in_smoothly_while_it_is_still_in_the_distance() {
        let flight = flight_seconds(6.0, 6);
        let entry = entry_seconds(0.9, flight);
        let entry_progress = entry / flight;
        assert!(entry_progress <= ENTRY_MAX_FRACTION + 1e-9);
        // Fully opaque long before the print is anywhere near the lens.
        assert!(
            magnification(entry_progress, NEAR_DEPTH_RANGE.0)
                < 0.25 * FAR_DEPTH / NEAR_DEPTH_RANGE.0
        );
        let mut previous = flight_opacity(0.0, entry_progress);
        assert_eq!(previous, 0.0, "prints must not start visible");
        for step in 1..=240 {
            let progress = entry_progress * (step as f64 / 240.0);
            let opacity = flight_opacity(progress, entry_progress);
            assert!(opacity >= previous - 1e-12, "the fade went backwards");
            assert!(
                opacity - previous < 0.02,
                "opacity jumped: a print would pop into existence"
            );
            previous = opacity;
        }
        assert!((flight_opacity(entry_progress, entry_progress) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_entry_fade_stays_a_short_part_of_every_flight() {
        for period in [3.0, 8.0, 30.0] {
            for slots in 1..=FLY_MAX_SLOTS {
                let flight = flight_seconds(period, slots);
                assert!((6.0..=30.0).contains(&flight));
                for fade in [0.2, 0.9, 2.5] {
                    let entry = entry_seconds(fade, flight);
                    assert!(entry >= 0.2 && entry <= flight * ENTRY_MAX_FRACTION + 1e-9);
                    let progress = entry / flight;
                    assert_eq!(flight_opacity(0.0, progress), 0.0);
                    assert!((flight_opacity(progress, progress) - 1.0).abs() < 1e-9);
                }
            }
        }
    }

    #[test]
    fn a_print_still_on_screen_when_its_flight_ends_fades_out() {
        let progress = 1.0 - EXIT_FADE_FRACTION;
        assert!((flight_opacity(progress, 0.1) - 1.0).abs() < 1e-9);
        assert_eq!(flight_opacity(1.0, 0.1), 0.0);
        let mut previous = flight_opacity(progress, 0.1);
        for step in 1..=140 {
            let at = progress + (1.0 - progress) * (step as f64 / 140.0);
            let opacity = flight_opacity(at, 0.1);
            assert!(opacity <= previous + 1e-12, "it must only fade out");
            assert!(previous - opacity < 0.03, "the print blinked off");
            previous = opacity;
        }
    }

    #[test]
    fn flights_approach_from_the_left_right_top_and_bottom() {
        let mut quadrant = [0usize; 4];
        let mut angles: Vec<f64> = Vec::new();
        for id in 0..400 {
            for launch in 0..2 {
                let way = attitude(id, launch);
                let length = (way.dir.0 * way.dir.0 + way.dir.1 * way.dir.1).sqrt();
                assert!((length - 1.0).abs() < 1e-9);
                angles.push(way.dir.1.atan2(way.dir.0));
                let index = match (way.dir.0 >= 0.0, way.dir.1 >= 0.0) {
                    (true, true) => 0,
                    (true, false) => 1,
                    (false, true) => 2,
                    _ => 3,
                };
                quadrant[index] += 1;
            }
        }
        for count in quadrant {
            assert!(count > 100, "every screen edge must be used: {quadrant:?}");
        }
        angles.sort_by(|a, b| a.total_cmp(b));
        assert!(angles[angles.len() - 1] - angles[0] > std::f64::consts::TAU * 0.99);
    }

    #[test]
    fn each_print_takes_its_own_path_at_its_own_speed() {
        let mut speeds = HashSet::new();
        let mut depths = HashSet::new();
        let mut centres = HashSet::new();
        let mut durations = Vec::new();
        for id in 0..40 {
            let way = attitude(id, 7);
            assert!((1.0 - SPEED_VARIATION..=1.0 + SPEED_VARIATION).contains(&way.speed));
            assert!((PRINT_SIZE_RANGE.0..=PRINT_SIZE_RANGE.1).contains(&way.size_seed));
            depths.insert((way.near * 1000.0).round() as i64);
            speeds.insert((way.speed * 1000.0).round() as i64);
            let pose = flight_pose(&way, 1.5, 0.5, 4.0, 0.2, SCREEN);
            centres.insert((pose.cx.round() as i64, pose.cy.round() as i64));
            durations.push(flight_seconds(6.0, 6) / way.speed);
        }
        assert!(depths.len() > 15, "prints need different pass depths");
        assert!(speeds.len() > 30, "prints need different speeds");
        assert!(centres.len() > 30, "prints need different trajectories");
        let slowest = durations.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let fastest = durations.iter().copied().fold(f64::INFINITY, f64::min);
        assert!(slowest > fastest * 1.15, "one speed for every photo");
    }

    #[test]
    fn prints_wobble_gently_and_never_spin_upside_down() {
        for id in 0..12 {
            let way = attitude(id, 0);
            let mut angles: Vec<f64> = Vec::new();
            for step in 0..600 {
                let age = step as f64 / 60.0;
                let pose = flight_pose(&way, 1.5, 0.5, age, 0.2, SCREEN);
                assert!(
                    pose.angle.abs() <= ROLL_RANGE.1.abs() + WOBBLE_AMPLITUDE.1 + 1e-9,
                    "a print rotated too far: {}",
                    pose.angle
                );
                angles.push(pose.angle);
            }
            let highest = angles.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let lowest = angles.iter().copied().fold(f64::INFINITY, f64::min);
            assert!(highest - lowest >= WOBBLE_AMPLITUDE.0, "no wobble at all");
            for pair in angles.windows(2) {
                assert!((pair[1] - pair[0]).abs() < 0.1, "the wobble is jerky");
            }
        }
    }

    #[test]
    fn the_flight_never_settles_and_every_print_moves_on_every_frame() {
        let mut rig = Rig::new(6, 6.0, 0.9);
        for id in 0..6 {
            rig.load(id, true);
        }
        rig.run(20.0);
        // The first seconds are the field filling up; steady state starts after.
        let steady = &rig.history[180..];
        let frames = steady.len();
        assert!(frames > 100);
        assert!(steady.iter().all(|frame| frame.len() >= 4));
        for index in 1..frames {
            let (previous, next) = (&steady[index - 1], &steady[index]);
            for a in previous {
                if !next.iter().any(|card| card.id == a.id) {
                    assert!(
                        a.opacity < 0.01,
                        "a print disappeared before its fade: {}",
                        a.opacity
                    );
                }
            }
            for a in previous {
                let Some(b) = next.iter().find(|card| card.id == a.id) else {
                    continue;
                };
                assert!(b.closeness > a.closeness, "a print stopped approaching");
                assert!(b.w > a.w, "prints must only grow");
                let moved = (a.cx - b.cx).hypot(a.cy - b.cy);
                assert!(moved > 1e-4, "a print froze in place");
                assert!(moved < 0.03 * SCREEN.0, "a print jumped: {moved}");
            }
        }
        let (first, last) = (&steady[0], &steady[frames - 1]);
        for a in first {
            let Some(b) = last.iter().find(|card| card.id == a.id) else {
                continue; // This flight passed the camera during the sample.
            };
            let travelled = (a.cx - b.cx).hypot(a.cy - b.cy);
            assert!(
                travelled > 0.02 * span(),
                "a print barely moved: {travelled}"
            );
        }
        // Prints overlap each other in depth; this is not a layout.
        let overlapping = steady
            .iter()
            .filter(|frame| {
                frame.iter().enumerate().any(|(i, a)| {
                    frame.iter().skip(i + 1).any(|b| {
                        a.cx - a.w * 0.5 < b.cx + b.w * 0.5
                            && b.cx - b.w * 0.5 < a.cx + a.w * 0.5
                            && a.cy - a.h * 0.5 < b.cy + b.h * 0.5
                            && b.cy - b.h * 0.5 < a.cy + a.h * 0.5
                    })
                })
            })
            .count();
        assert!(
            overlapping * 2 > frames,
            "prints should overlap naturally: {overlapping}/{frames}"
        );
    }

    #[test]
    fn distant_prints_are_painted_first_so_approaching_ones_overlap_them() {
        let mut rig = Rig::new(6, 6.0, 0.9);
        for id in 0..6 {
            rig.load(id, true);
        }
        rig.run(6.0);
        let cards = rig.frame();
        assert!(cards.len() >= 4);
        for pair in cards.windows(2) {
            assert!(pair[0].closeness <= pair[1].closeness + 1e-12);
        }
    }

    #[test]
    fn a_replaced_slot_sends_its_new_photo_off_without_robbing_the_print_in_the_air() {
        let mut rig = Rig::new(4, 6.0, 0.9);
        for id in 0..8 {
            rig.load(id, true);
        }
        rig.run(2.0);
        let before = rig.frame();
        let in_flight = before
            .iter()
            .find(|card| card.id == 0)
            .expect("photo 0 flying");
        rig.replace(0, 7);
        rig.run(0.4);
        let after = rig.frame();
        let carried_on = after
            .iter()
            .find(|card| card.id == 0)
            .expect("flight was robbed");
        let entered = after
            .iter()
            .find(|card| card.id == 7)
            .expect("no new print entered");
        assert!(
            (carried_on.cx - in_flight.cx).abs() < 60.0
                && (carried_on.cy - in_flight.cy).abs() < 60.0,
            "the print in flight was moved"
        );
        assert!(carried_on.closeness > entered.closeness);
        assert!(
            entered.w.max(entered.h) < 0.25 * span(),
            "the new print did not enter from the distance: {}",
            entered.w.max(entered.h)
        );
        assert!(
            entered.opacity < 0.95,
            "the new print popped in: {}",
            entered.opacity
        );
    }

    #[test]
    fn a_sharper_original_fades_in_without_restarting_the_flight() {
        let mut rig = Rig::new(3, 6.0, 0.9);
        for id in 0..3 {
            rig.load(id, false);
        }
        rig.run(2.0);
        let preview = rig.frame();
        let card = preview.iter().find(|card| card.id == 0).unwrap();
        assert!(card.upgrade.is_none());
        rig.load(0, true);
        rig.frame();
        let upgrading = rig.frame();
        let next = upgrading.iter().find(|card| card.id == 0).unwrap();
        assert!(next.upgrade.is_some());
        assert_eq!(next.slot, card.slot);
        assert!(
            (next.cx - card.cx).abs() < 2.0 && (next.w - card.w).abs() < 2.0,
            "the upgrade moved or resized the print"
        );
        rig.run(1.0);
        let settled = rig.frame().into_iter().find(|card| card.id == 0).unwrap();
        assert!(settled.upgrade.is_none(), "the cross-fade never finished");
    }

    #[test]
    fn pause_freezes_the_flight_and_a_stalled_frame_cannot_jump_it() {
        let mut rig = Rig::new(4, 6.0, 0.9);
        for id in 0..4 {
            rig.load(id, true);
        }
        rig.run(2.0);
        let frozen = rig.frame();
        rig.paused = true;
        rig.step_us = 3_000_000; // a stalled decode, a texture upload, a suspend
        for _ in 0..5 {
            rig.frame();
        }
        let still = rig.frame();
        assert_eq!(frozen.len(), still.len());
        for (a, b) in frozen.iter().zip(still.iter()) {
            assert_eq!(
                (a.cx, a.cy, a.w, a.h, a.opacity),
                (b.cx, b.cy, b.w, b.h, b.opacity)
            );
        }
        rig.paused = false;
        rig.step_us = 16_667;
        let resumed = rig.frame();
        let after = rig.frame();
        for (a, b) in resumed.iter().zip(after.iter()) {
            assert!(
                (a.cx - b.cx).hypot(a.cy - b.cy) > 0.001,
                "motion never resumed"
            );
        }
    }

    #[test]
    fn new_prints_keep_entering_even_when_the_library_is_tiny() {
        let mut rig = Rig::new(6, 3.0, 0.9);
        rig.slots = vec![0, 1, 2];
        for id in 0..3 {
            rig.load(id, true);
        }
        let mut seen = HashSet::new();
        for _ in 0..2400 {
            for card in rig.frame() {
                seen.insert(card.id);
            }
        }
        assert_eq!(seen, [0, 1, 2].into_iter().collect::<HashSet<_>>());
        assert!(
            rig.model.launches >= 12,
            "the field went empty instead of refilling: {}",
            rig.model.launches
        );
        assert!(rig.model.flights.len() <= FLY_MAX_FLIGHTS);
    }

    #[test]
    fn a_slot_without_a_decoded_photo_is_left_alone_until_it_arrives() {
        let mut rig = Rig::new(3, 6.0, 0.9);
        rig.load(0, true);
        rig.load(2, true);
        rig.run(2.0);
        let seen: HashSet<usize> = rig.history.iter().flatten().map(|card| card.id).collect();
        assert!(seen.contains(&0) && seen.contains(&2));
        assert!(
            !seen.contains(&1),
            "launched a photo that was never decoded"
        );
        assert_eq!(rig.model.flights.iter().filter(|f| f.slot == 1).count(), 0);
    }

    #[test]
    fn rapid_slot_changes_cannot_grow_the_flight_field_without_bound() {
        let mut rig = Rig::new(12, 3.0, 0.9);
        for id in 0..400 {
            rig.load(id, true);
        }
        for step in 0..900 {
            rig.frame();
            if step % 30 == 0 {
                rig.replace(step % 12, 100 + step);
            }
        }
        assert!(rig.model.flights.len() <= FLY_MAX_FLIGHTS);
        assert!(!rig.history.last().unwrap().is_empty());
    }
    fn synth(w: u32, h: u32, seed: u32) -> Pixels {
        let hues = [
            (220u32, 90u32, 70u32),
            (60, 140, 200),
            (90, 180, 110),
            (200, 160, 60),
            (150, 90, 190),
            (70, 70, 90),
        ];
        let (r, g, b) = hues[seed as usize % hues.len()];
        let mut img = image::RgbaImage::new(w, h);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let t = y as f32 / h as f32;
            let stripe = if ((x / (w / 12).max(1)) + (y / (h / 12).max(1))) % 2 == 0 {
                18
            } else {
                0
            };
            *p = image::Rgba([
                ((r as f32 * (1.0 - 0.5 * t)) as u32 + stripe).min(255) as u8,
                ((g as f32 * (1.0 - 0.5 * t)) as u32 + stripe).min(255) as u8,
                ((b as f32 * (1.0 - 0.5 * t)) as u32 + stripe).min(255) as u8,
                255,
            ]);
        }
        for x in 0..w {
            for y in (h * 3 / 5)..(h * 3 / 5 + 4).min(h) {
                img.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        texture_pixels(img)
    }
    fn pump() {
        let c = glib::MainContext::default();
        while c.pending() {
            c.iteration(false);
        }
    }

    /// Rendered composition check + PNG dump: PIC_FLYING_PNG_DIR=/tmp/out
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn real_canvas_flies_prints_past_the_viewer() {
        libadwaita::init().unwrap();
        install_css();
        let mut player = Player::new(Style::Flying, 40, 2, false, 20);
        player.transition = Duration::from_millis(900);
        let mut flying = Flying::new();
        let root = gtk::Fixed::new();
        root.put(&flying.canvas, 0.0, 0.0);
        let window = gtk::Window::builder()
            .default_width(2560)
            .default_height(1440)
            .child(&root)
            .build();
        window.present();
        pump();
        let (tx, rx) = mpsc::channel();
        let mut frame_time = 1_000_000i64;
        let out = std::env::var("PIC_FLYING_PNG_DIR").ok();
        let mut shots = 0;
        let mut captured = HashSet::new();
        let mut widest = 0.0f64;
        let mut early = 0.0f64;
        let mut aspects = HashSet::new();
        for frame in 0..202 {
            while let Ok(i) = rx.try_recv() {
                let (w, h) = if i % 3 == 0 {
                    (1000u32, 1500)
                } else {
                    (1500, 1000)
                };
                player.receive(ImageResult {
                    index: i,
                    high_quality: false,
                    pixels: Some(synth(w / 8, h / 8, i as u32)),
                });
                player.receive(ImageResult {
                    index: i,
                    high_quality: true,
                    pixels: Some(synth(w / 2, h / 2, i as u32)),
                });
            }
            player.update(&tx);
            frame_time += 50_000;
            flying.sync(&player, 2560.0, 1440.0, frame_time);
            pump();
            let painted = flying.canvas.painted();
            if frame > 30 {
                assert!(
                    !painted.is_empty(),
                    "the flight field went empty at {frame}"
                );
                for card in &painted {
                    aspects.insert(((card.w / card.h) * 100.0).round() as i64);
                    assert!(card.opacity > 0.0 && card.opacity <= 1.0);
                }
                for pair in painted.windows(2) {
                    assert!(pair[0].closeness <= pair[1].closeness, "painted near-first");
                }
            }
            // The Player switches slots on wall-clock time, so which print is
            // where varies run to run; measure growth across the whole run.
            let widest_now = painted
                .iter()
                .map(|card| card.w.max(card.h))
                .fold(0.0f64, f64::max);
            if frame < 100 {
                early = early.max(widest_now);
            }
            widest = widest.max(widest_now);
            if let (Some(dir), Some(target)) = (
                &out,
                [40, 100, 200]
                    .iter()
                    .copied()
                    .find(|target| *target <= frame && !captured.contains(target)),
            ) {
                let paintable = gtk::WidgetPaintable::new(Some(&flying.canvas));
                let snap = gtk::Snapshot::new();
                paintable.snapshot(&snap, 2560.0, 1440.0);
                if let (Some(node), Some(renderer)) =
                    (snap.to_node(), window.native().and_then(|n| n.renderer()))
                {
                    let texture = renderer.render_texture(&node, None);
                    let seconds = target / 20;
                    let _ = texture.save_to_png(format!("{dir}/flying_{seconds}s.png"));
                    shots += 1;
                    captured.insert(target);
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if let (Some(dir), false) = (&out, captured.contains(&200)) {
            let paintable = gtk::WidgetPaintable::new(Some(&flying.canvas));
            let snap = gtk::Snapshot::new();
            paintable.snapshot(&snap, 2560.0, 1440.0);
            if let (Some(node), Some(renderer)) =
                (snap.to_node(), window.native().and_then(|n| n.renderer()))
            {
                let texture = renderer.render_texture(&node, None);
                let _ = texture.save_to_png(format!("{dir}/flying_10s.png"));
                shots += 1;
            }
        }
        assert!(
            widest > early * 2.0,
            "prints never grow as they approach: {early} -> {widest}"
        );
        assert!(
            widest > 0.45 * 1440.0,
            "no print ever reached the viewer: {widest}"
        );
        assert!(
            aspects.iter().count() >= 2,
            "portrait and landscape both keep their shape"
        );
        if out.is_some() {
            assert_eq!(shots, 3, "capture rendered frames at 2, 5 and 10 seconds");
        }
    }
}
