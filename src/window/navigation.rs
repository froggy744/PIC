/// Distance one mouse-wheel notch scrolls a gallery, in All Photos and
/// Folders.
const GALLERY_WHEEL_STEP_PX: f64 = 160.0;

/// One wheel glide: smooth mouse-wheel scrolling as browsers do it.
///
/// Every wheel click retargets a glide that ends at exactly zero speed exactly
/// on its destination after a fixed duration: a cubic Hermite curve from the
/// current position and speed to the destination at rest. A click during a
/// glide starts a new curve from the current position *and speed*, so
/// continuous scrolling has no speed jumps; a reversal starts from rest.
/// The view moves in whole pixels and the curve has no asymptotic tail, so
/// it never crawls 1, 0, 1 px or steps again after it looks still (what the
/// former critically damped spring did).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct WheelGlide {
    active: bool,
    /// Frame-clock time the curve started, in microseconds.
    start_us: i64,
    duration_s: f64,
    /// Whole pixels from the curve's start to its destination.
    distance: f64,
    /// Speed at the start, in px/s.
    start_speed: f64,
    /// Whole pixels already applied.
    shown: f64,
}

impl WheelGlide {
    /// Short hops are quick; long accumulated runs take a little longer.
    fn duration_for(distance: f64) -> f64 {
        (0.16 + 0.0004 * distance.abs()).clamp(0.18, 0.36)
    }

    fn progress(&self, now_us: i64) -> f64 {
        ((now_us - self.start_us) as f64 / 1_000_000.0 / self.duration_s).clamp(0.0, 1.0)
    }

    /// Offset from the start at progress `s`.
    fn offset(&self, s: f64) -> f64 {
        let h10 = s * s * s - 2.0 * s * s + s;
        let h01 = -2.0 * s * s * s + 3.0 * s * s;
        h10 * self.duration_s * self.start_speed + h01 * self.distance
    }

    /// Speed in px/s at progress `s`.
    fn speed(&self, s: f64) -> f64 {
        let h10 = 3.0 * s * s - 4.0 * s + 1.0;
        let h01 = -6.0 * s * s + 6.0 * s;
        h10 * self.start_speed + h01 * self.distance / self.duration_s
    }

    /// Add one wheel click of `delta` px. `room` is how far the view can
    /// still move (negative, positive) from where it is shown now.
    fn notch(&mut self, now_us: i64, delta: f64, room: (f64, f64)) {
        let (left, speed) = if self.active {
            (self.distance - self.shown, self.speed(self.progress(now_us)))
        } else {
            (0.0, 0.0)
        };
        let reversing = left * delta < 0.0 || speed * delta < 0.0;
        let (left, speed) = if reversing { (delta, 0.0) } else { (left + delta, speed) };
        let distance = left.clamp(room.0, room.1).round();
        if distance == 0.0 {
            *self = Self::default();
            return;
        }
        let mut duration_s = Self::duration_for(distance);
        // A cubic from speed v to rest overshoots once v * T > 3 * distance;
        // arrive sooner instead.
        if speed.abs() * duration_s > 3.0 * distance.abs() {
            duration_s = 3.0 * distance.abs() / speed.abs();
        }
        *self = Self {
            active: true,
            start_us: now_us,
            duration_s,
            distance,
            start_speed: speed,
            shown: 0.0,
        };
    }

    /// Whole pixels to move for the frame at `now_us`.
    fn advance(&mut self, now_us: i64) -> f64 {
        if !self.active {
            return 0.0;
        }
        let s = self.progress(now_us);
        let at = if s >= 1.0 {
            self.active = false;
            self.distance
        } else if self.shown == 0.0 {
            // Move on the very first frame, even if the curve has barely
            // started: the write takes over the view, cancelling a reveal
            // still pending in GTK (keyboard navigation), so the newer wheel
            // input wins instead of being undone by it.
            let at = self.offset(s).round();
            if at == 0.0 { self.distance.signum() } else { at }
        } else {
            self.offset(s).round()
        };
        let step = at - self.shown;
        self.shown = at;
        step
    }
}

/// `PIC_WHEEL_GLIDE_TRACE=1` logs every wheel click and glide frame.
fn wheel_glide_trace() -> bool {
    thread_local! {
        static ENABLED: bool = std::env::var_os("PIC_WHEEL_GLIDE_TRACE").is_some();
    }
    ENABLED.with(|enabled| *enabled)
}

/// Drives a [`WheelGlide`] on `scrolled`'s vertical adjustment.
#[derive(Clone)]
struct WheelGlider {
    scrolled: gtk::ScrolledWindow,
    glide: Rc<Cell<WheelGlide>>,
}

impl WheelGlider {
    /// Starts gliding on `scrolled`. A pointer press (a scrollbar drag, a
    /// click) or a programmatic jump of more than half a page stops it; other
    /// adjustment corrections are absorbed, because each frame moves relative
    /// to where the view is.
    fn install(scrolled: &gtk::ScrolledWindow) -> Self {
        let glider = Self {
            scrolled: scrolled.clone(),
            glide: Rc::new(Cell::new(WheelGlide::default())),
        };
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let glider = glider.clone();
            click.connect_pressed(move |_, _, _, _| glider.cancel());
        }
        scrolled.add_controller(click);

        let written = Rc::new(Cell::new(f64::NAN));
        {
            let glide = glider.glide.clone();
            let written = written.clone();
            let adjustment = scrolled.vadjustment();
            scrolled.add_tick_callback(move |_, clock| {
                let mut state = glide.get();
                if !state.active {
                    return glib::ControlFlow::Continue;
                }
                let current = adjustment.value();
                let last = written.get();
                if last.is_finite() && (current - last).abs() > adjustment.page_size() * 0.5 {
                    if wheel_glide_trace() {
                        eprintln!("PIC_WHEEL_GLIDE external value={current} written={last}");
                    }
                    // Someone else scrolled the view (navigation, zoom).
                    glide.set(WheelGlide::default());
                    written.set(f64::NAN);
                    return glib::ControlFlow::Continue;
                }
                let lower = adjustment.lower();
                let upper = (adjustment.upper() - adjustment.page_size()).max(lower);
                let step = state.advance(clock.frame_time());
                let next = (current + step).clamp(lower, upper);
                if next != current + step {
                    // Reached an end of the gallery.
                    state = WheelGlide::default();
                }
                if next != current {
                    adjustment.set_value(next);
                }
                if wheel_glide_trace() {
                    eprintln!(
                        "PIC_WHEEL_GLIDE frame t={} step={step} from={current} to={next} active={}",
                        clock.frame_time(),
                        state.active
                    );
                }
                written.set(if state.active { next } else { f64::NAN });
                glide.set(state);
                glib::ControlFlow::Continue
            });
        }
        glider
    }

    /// Add one wheel click of `delta` px.
    fn notch(&self, delta: f64) {
        let adjustment = self.scrolled.vadjustment();
        let lower = adjustment.lower();
        let upper = (adjustment.upper() - adjustment.page_size()).max(lower);
        let current = adjustment.value();
        let now = self
            .scrolled
            .frame_clock()
            .map_or_else(glib::monotonic_time, |clock| clock.frame_time());
        let mut glide = self.glide.get();
        glide.notch(now, delta, (lower - current, upper - current));
        if wheel_glide_trace() {
            eprintln!(
                "PIC_WHEEL_GLIDE notch t={now} at={current} distance={} speed={:.0} duration_ms={:.0}",
                glide.distance,
                glide.start_speed,
                glide.duration_s * 1000.0
            );
        }
        self.glide.set(glide);
    }

    fn cancel(&self) {
        self.glide.set(WheelGlide::default());
    }
}

pub(crate) fn install_smooth_gallery_scroll(
    scrolled: &gtk::ScrolledWindow,
    gallery: Rc<grid::Gallery>,
    ctrl_zoom: bool,
) {
    // GtkScrolledWindow does not ease mouse-wheel detents (its kinetic
    // scrolling is for touchscreens). Wheel clicks glide (`WheelGlide`);
    // touchpad pixel scrolling stays native.
    let adjustment = scrolled.vadjustment();
    let glider = WheelGlider::install(scrolled);

    // Keep the latest pointer position in the scroller's coordinate space.
    // Ctrl+wheel uses this to choose one realized photo as the stable zoom
    // focus for the whole wheel burst; no device polling or model math is
    // required.
    let last_pointer = Rc::new(Cell::new(None::<(f64, f64)>));
    let motion = gtk::EventControllerMotion::new();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let last_pointer = last_pointer.clone();
        motion.connect_enter(move |_, x, y| {
            last_pointer.set(Some((x, y)));
            if std::env::var_os("PICASA_TRACE").is_some() {
                eprintln!("PIC_ZOOM_TRACE pointer_enter x={:.1} y={:.1}", x, y);
            }
        });
    }
    {
        let last_pointer = last_pointer.clone();
        motion.connect_motion(move |_, x, y| last_pointer.set(Some((x, y))));
    }
    {
        let last_pointer = last_pointer.clone();
        motion.connect_leave(move |_| {
            last_pointer.set(None);
            if std::env::var_os("PICASA_TRACE").is_some() {
                eprintln!("PIC_ZOOM_TRACE pointer_leave");
            }
        });
    }
    scrolled.add_controller(motion);
    let zoom_viewport: gtk::Widget = scrolled.clone().upcast();

    // One wheel notch (dy = +1 down, -1 up). Shared by the real wheel
    // handler and the opt-in scripted scroll benchmark.
    let wheel_notch: Rc<dyn Fn(f64)> = {
        let glider = glider.clone();
        Rc::new(move |dy: f64| glider.notch(dy * GALLERY_WHEEL_STEP_PX))
    };
    // Only the All Photos grid installs this with Ctrl+wheel zoom; Albums
    // home does not.
    if ctrl_zoom && std::env::var("PIC_SCROLL_BENCH").as_deref() == Ok("photos") {
        start_scroll_bench(scrolled, wheel_notch.clone());
    }
    let wheel_notch_for_scroll = wheel_notch.clone();

    let controller = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);

    let adjustment_for_scroll = adjustment.clone();
    let glider_for_scroll = glider.clone();
    let last_pointer_for_scroll = last_pointer.clone();
    let zoom_viewport_for_scroll = zoom_viewport.clone();
    controller.connect_scroll(move |controller, dx, dy| {
        crate::scroll_trace::log_scroll_input("gallery", controller, dx, dy);
        // Cover Flow owns wheel navigation. This ancestor runs before its
        // controller in capture phase, so consuming here silently steals
        // detents even when the page adjustment has no scrollable range.
        if zoom_viewport_for_scroll.has_css_class("albums-coverflow-active") {
            glider_for_scroll.cancel();
            return glib::Propagation::Proceed;
        }
        if controller
            .current_event_state()
            .contains(gtk::gdk::ModifierType::CONTROL_MASK)
        {
            glider_for_scroll.cancel();

            if ctrl_zoom {
                let pointer = last_pointer_for_scroll.get();
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_ZOOM_TRACE input_ctrl_wheel dy={:.3} unit={:?} pointer={:?} scroll_y={:.1} lower={:.1} upper={:.1} page={:.1}",
                        dy,
                        controller.unit(),
                        pointer,
                        adjustment_for_scroll.value(),
                        adjustment_for_scroll.lower(),
                        adjustment_for_scroll.upper(),
                        adjustment_for_scroll.page_size(),
                    );
                }
                if dy < 0.0 {
                    if let Some((x, y)) = pointer {
                        gallery.wheel_zoom_in_at(&zoom_viewport_for_scroll, x, y);
                    } else {
                        gallery.wheel_zoom_in();
                    }
                } else if dy > 0.0 {
                    if let Some((x, y)) = pointer {
                        gallery.wheel_zoom_out_at(&zoom_viewport_for_scroll, x, y);
                    } else {
                        gallery.wheel_zoom_out();
                    }
                }
                return glib::Propagation::Stop;
            }

            // Albums home has no thumbnail-zoom action. Preserve the normal
            // Ctrl+wheel behavior instead of zooming the hidden photo grid.
            return glib::Propagation::Proceed;
        }

        if dy == 0.0 {
            return glib::Propagation::Proceed;
        }

        match controller.unit() {
            gtk::gdk::ScrollUnit::Wheel => {
                wheel_notch_for_scroll(dy);
                glib::Propagation::Stop
            }
            gtk::gdk::ScrollUnit::Surface => {
                // Precision touchpads already provide pixel deltas. Let GTK
                // consume them directly so native touchpad scrolling is kept.
                glider_for_scroll.cancel();
                glib::Propagation::Proceed
            }
            _ => glib::Propagation::Proceed,
        }
    });

    scrolled.add_controller(controller);
}



pub(crate) fn install_folder_smooth_gallery_scroll(
    scrolled: &gtk::ScrolledWindow,
    gallery: Rc<grid::Gallery>,
) {
    // Wheel clicks glide like All Photos (`WheelGlide`). Each frame moves
    // relative to where the view is, so the sectioned surface's own scroll
    // corrections never pull the view back to a stale target.
    let glider = WheelGlider::install(scrolled);

    let controller = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);

    // One wheel notch (dy = +1 down, -1 up). Shared by the real wheel
    // handler and the opt-in scripted scroll benchmark, so the benchmark
    // measures exactly the code path a mouse wheel drives.
    let wheel_notch: Rc<dyn Fn(f64)> = {
        let gallery = gallery.clone();
        let glider = glider.clone();
        Rc::new(move |dy: f64| {
            gallery.cancel_folder_keyboard_scroll_animation();
            glider.notch(dy * GALLERY_WHEEL_STEP_PX);
        })
    };
    if std::env::var("PIC_SCROLL_BENCH").as_deref() == Ok("folder") {
        start_scroll_bench(scrolled, wheel_notch.clone());
    }

    let glider_for_scroll = glider.clone();
    let wheel_notch_for_scroll = wheel_notch.clone();
    controller.connect_scroll(move |controller, dx, dy| {
        crate::scroll_trace::log_scroll_input("folder_gallery", controller, dx, dy);
        if controller
            .current_event_state()
            .contains(gtk::gdk::ModifierType::CONTROL_MASK)
        {
            glider_for_scroll.cancel();
            // Match All Photos: commit each wheel step immediately and let
            // the gallery's frame-clock animation retarget on the next step.
            // The former +/- request path used a 150 ms trailing debounce,
            // making Folder Ctrl+wheel feel delayed or snap on release.
            if dy < 0.0 {
                gallery.wheel_zoom_in();
            } else if dy > 0.0 {
                gallery.wheel_zoom_out();
            }
            return glib::Propagation::Stop;
        }

        if dy == 0.0 {
            return glib::Propagation::Proceed;
        }

        match controller.unit() {
            gtk::gdk::ScrollUnit::Wheel => {
                wheel_notch_for_scroll(dy);
                glib::Propagation::Stop
            }
            gtk::gdk::ScrollUnit::Surface => {
                // Precision touchpads already deliver smooth pixel deltas.
                // Leave those native and cancel any stale wheel easing.
                glider_for_scroll.cancel();
                glib::Propagation::Proceed
            }
            _ => glib::Propagation::Proceed,
        }
    });

    scrolled.add_controller(controller);
}

/// Opt-in scripted gallery scroll (`PIC_SCROLL_BENCH=folder` for the Folder
/// view, `PIC_SCROLL_BENCH=photos` for All Photos), for measuring
/// scroll smoothness on the real app, compositor and GPU without a hand on
/// the wheel. Combine with `PIC_SCROLL_FRAME_TRACE=1`; phases are marked with
/// `PIC_SCROLL_BENCH phase=<name>` lines so the frame trace can be split.
///
/// Script, at `PIC_SCROLL_BENCH_RATE` wheel notches per second (default 12;
/// fast hand scrolling is roughly 30-45):
/// - `cold`: 60 notches down into photos not yet decoded;
/// - `warm`: 4 x (8 up, 8 down) over the screens just visited, whose
///   thumbnails are already in RAM;
/// - `cold2`: 60 more notches down.
///
/// It starts once that view is on screen with content, and closes the
/// window when done. Without the variable, nothing here runs.
fn start_scroll_bench(scrolled: &gtk::ScrolledWindow, wheel_notch: Rc<dyn Fn(f64)>) {
    #[derive(Clone, Copy)]
    enum Step {
        Mark(&'static str),
        Notch(f64),
        Wait,
    }
    let mut script = vec![Step::Mark("cold")];
    script.extend(std::iter::repeat(Step::Notch(1.0)).take(60));
    script.extend(std::iter::repeat(Step::Wait).take(14));
    script.push(Step::Mark("warm"));
    for _ in 0..4 {
        script.extend(std::iter::repeat(Step::Notch(-1.0)).take(8));
        script.extend(std::iter::repeat(Step::Notch(1.0)).take(8));
    }
    script.extend(std::iter::repeat(Step::Wait).take(14));
    script.push(Step::Mark("cold2"));
    script.extend(std::iter::repeat(Step::Notch(1.0)).take(60));
    script.extend(std::iter::repeat(Step::Wait).take(14));
    script.push(Step::Mark("done"));

    let rate = std::env::var("PIC_SCROLL_BENCH_RATE")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|rate| (1..=120).contains(rate))
        .unwrap_or(12);
    let scrolled = scrolled.clone();
    let started = Instant::now();
    let ready_since = Rc::new(Cell::new(None::<Instant>));
    let position = Rc::new(Cell::new(0usize));
    eprintln!("PIC_SCROLL_BENCH rate={rate}");
    glib::timeout_add_local(Duration::from_millis(1000 / rate), move || {
        let adjustment = scrolled.vadjustment();
        let has_content = scrolled.is_mapped()
            && adjustment.upper() > adjustment.page_size() * 12.0;
        if ready_since.get().is_none() {
            if has_content {
                ready_since.set(Some(Instant::now()));
                eprintln!("PIC_SCROLL_BENCH phase=waiting_for_settle");
            } else if started.elapsed() > Duration::from_secs(120) {
                eprintln!("PIC_SCROLL_BENCH phase=skipped reason=view_not_visible_or_too_short");
                return glib::ControlFlow::Break;
            }
            return glib::ControlFlow::Continue;
        }
        // Let startup thumbnail work finish before measuring.
        if ready_since.get().is_some_and(|since| since.elapsed() < Duration::from_secs(4)) {
            return glib::ControlFlow::Continue;
        }
        let index = position.get();
        let Some(step) = script.get(index).copied() else {
            if let Some(window) = scrolled.root().and_downcast::<gtk::Window>() {
                window.close();
            }
            return glib::ControlFlow::Break;
        };
        position.set(index + 1);
        match step {
            Step::Mark(name) => eprintln!(
                "PIC_SCROLL_BENCH phase={name} value={:.1} upper={:.1}",
                adjustment.value(),
                adjustment.upper()
            ),
            Step::Notch(dy) => wheel_notch(dy),
            Step::Wait => {}
        }
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod wheel_glide_tests {
    use super::{WheelGlide, GALLERY_WHEEL_STEP_PX};

    const ROOM: (f64, f64) = (-1.0e6, 1.0e6);

    /// Pixels moved in every frame at `hz`, with a wheel click at each time
    /// in `clicks` (seconds; negative values click upward).
    fn run(clicks: &[f64], hz: f64, seconds: f64) -> Vec<f64> {
        let frame_us = (1_000_000.0 / hz) as i64;
        let mut glide = WheelGlide::default();
        let mut next = 0;
        let mut moves = Vec::new();
        let mut now = 0_i64;
        while now < (seconds * 1_000_000.0) as i64 {
            while next < clicks.len() && (clicks[next].abs() * 1_000_000.0) as i64 <= now {
                let delta = if clicks[next] < 0.0 { -GALLERY_WHEEL_STEP_PX } else { GALLERY_WHEEL_STEP_PX };
                glide.notch(now, delta, ROOM);
                next += 1;
            }
            now += frame_us;
            moves.push(glide.advance(now));
        }
        moves
    }

    fn motion(moves: &[f64]) -> &[f64] {
        let first = moves.iter().position(|step| *step != 0.0).unwrap();
        let last = moves.iter().rposition(|step| *step != 0.0).unwrap();
        &moves[first..=last]
    }

    #[test]
    fn one_click_glides_to_an_exact_stop() {
        for hz in [60.0, 75.0, 120.0, 144.0] {
            let moves = run(&[0.0], hz, 1.0);
            assert_eq!(moves.iter().sum::<f64>(), GALLERY_WHEEL_STEP_PX, "{hz} Hz: {moves:?}");
            let motion = motion(&moves);
            assert!(motion.iter().all(|step| *step >= 0.0), "{hz} Hz went back: {motion:?}");
            // Finite: done within the glide duration, no tail.
            assert!(motion.len() as f64 / hz <= 0.25, "{hz} Hz took too long: {motion:?}");
            // Eases in and out: the speed rises, then only falls.
            let peak = motion.iter().cloned().fold(0.0, f64::max);
            let top = motion.iter().position(|step| *step == peak).unwrap();
            assert!(top > 0 && top < motion.len() - 1, "{hz} Hz: {motion:?}");
            assert!(
                motion[top..].windows(2).all(|pair| pair[1] <= pair[0] + 1.0),
                "{hz} Hz sped up while stopping: {motion:?}"
            );
        }
    }

    #[test]
    fn the_stop_has_no_late_step() {
        // A still frame followed by another move is the "last bit of jump".
        // Below 7 ms (one 144 Hz frame) it cannot be seen.
        for hz in [60.0, 75.0, 120.0, 144.0] {
            let moves = run(&[0.0], hz, 1.0);
            let motion = motion(&moves);
            let mut still = 0;
            for step in motion {
                if *step == 0.0 {
                    still += 1;
                } else {
                    assert!(still as f64 / hz < 0.008, "{hz} Hz stepped after a pause: {motion:?}");
                    still = 0;
                }
            }
        }
    }

    #[test]
    fn continuous_clicks_keep_an_even_speed() {
        for rate in [8.0, 15.0] {
            let clicks: Vec<f64> = (0..(rate * 0.8) as usize).map(|i| i as f64 / rate).collect();
            let moves = run(&clicks, 120.0, 2.0);
            assert_eq!(moves.iter().sum::<f64>(), GALLERY_WHEEL_STEP_PX * clicks.len() as f64);
            // Retargeting keeps the speed, so it never jumps at a click.
            let steady = &moves[20..90];
            let worst = steady.windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0, f64::max);
            assert!(worst <= 3.0, "{rate} clicks/s: speed jumped {worst} px/frame: {steady:?}");
        }
    }

    #[test]
    fn reversing_starts_from_rest_without_overshoot() {
        let moves = run(&[0.0, -0.1], 120.0, 1.0);
        let total: f64 = moves.iter().sum();
        let reversal = moves.iter().position(|step| *step < 0.0).unwrap();
        let before: f64 = moves[..reversal].iter().sum();
        assert_eq!(total, before - GALLERY_WHEEL_STEP_PX, "{moves:?}");
        assert!(moves[reversal..].iter().all(|step| *step <= 0.0), "{moves:?}");
    }

    #[test]
    fn a_new_glide_moves_on_its_first_frame() {
        let mut glide = WheelGlide::default();
        glide.notch(1_000, 160.0, ROOM);
        assert_eq!(glide.advance(1_000), 1.0, "the click's own frame must write");
    }

    #[test]
    fn the_end_of_the_gallery_bounds_the_glide() {
        let mut glide = WheelGlide::default();
        glide.notch(0, 160.0, (-10.0, 40.0));
        let mut moved = 0.0;
        for frame in 1..100 {
            moved += glide.advance(frame * 8_000);
        }
        assert_eq!(moved, 40.0);
    }
}

#[cfg(test)]
mod grid_wheel_fraction_tests {
    use super::*;

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn grid_wheel_glides_whole_pixels_to_an_exact_stop() {
        fn settle(milliseconds: u64) {
            let context = glib::MainContext::default();
            let until = Instant::now() + Duration::from_millis(milliseconds);
            while Instant::now() < until {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        gtk::init().unwrap();
        let gallery = Rc::new(grid::Gallery::new(
            &[],
            160,
            |_| {},
            |_, _, _| {},
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
        ));
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, object| {
            let item = object.downcast_ref::<gtk::ListItem>().unwrap();
            let child = gtk::Box::new(gtk::Orientation::Vertical, 0);
            child.set_size_request(100, 80);
            item.set_child(Some(&child));
        });
        let grid = gtk::GridView::new(
            Some(gtk::NoSelection::new(Some(gtk::StringList::new(
                &vec!["photo"; 1000],
            )))),
            Some(factory),
        );
        grid.set_min_columns(5);
        grid.set_max_columns(5);
        let scroll = gtk::ScrolledWindow::builder().child(&grid).build();
        install_smooth_gallery_scroll(&scroll, gallery, true);
        let window = gtk::Window::builder()
            .default_width(900)
            .default_height(650)
            .child(&scroll)
            .build();
        window.present();
        settle(250);
        let adjustment = scroll.vadjustment();
        adjustment.set_value(1000.0);
        settle(30);
        let start = adjustment.value();
        // Every distinct value the view shows, with when it appeared.
        let changes = Rc::new(RefCell::new(Vec::<(Instant, f64)>::new()));
        let observed = changes.clone();
        adjustment.connect_value_changed(move |adjustment| {
            let value = adjustment.value();
            let mut changes = observed.borrow_mut();
            if changes.last().map_or(true, |(_, last)| *last != value) {
                changes.push((Instant::now(), value));
            }
        });
        let controllers = scroll.observe_controllers();
        let wheel = (0..controllers.n_items())
            .find_map(|i| {
                controllers
                    .item(i)
                    .and_downcast::<gtk::EventControllerScroll>()
                    .filter(|controller| {
                        controller.propagation_phase() == gtk::PropagationPhase::Capture
                    })
            })
            .unwrap();
        let clicked = Instant::now();
        assert!(wheel.emit_by_name::<bool>("scroll", &[&0.0_f64, &1.0_f64]));
        settle(800);
        let distance = adjustment.value() - start;
        window.close();
        let changes = changes.borrow();
        assert!(
            changes.iter().all(|(_, value)| value.fract() == 0.0),
            "the grid must be given whole pixels: {changes:?}"
        );
        assert_eq!(distance, GALLERY_WHEEL_STEP_PX, "{changes:?}");
        // The glide ends on time with no late step after a pause.
        let (last, _) = changes.last().expect("the wheel moved the grid");
        assert!(
            *last - clicked < Duration::from_millis(400),
            "the glide ended {:?} after the click: {changes:?}",
            *last - clicked
        );
        let longest_pause = changes
            .windows(2)
            .map(|pair| pair[1].0 - pair[0].0)
            .max()
            .unwrap_or_default();
        assert!(
            longest_pause < Duration::from_millis(60),
            "a late step followed a {longest_pause:?} pause: {changes:?}"
        );
    }
}

fn open_in_folder_should_stop(building: bool, revealed: bool, attempt: u32) -> bool {
    (!building && revealed) || attempt >= 800
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FolderDestinationPlan {
    Normal,
    ReuseWithoutFolderScroll,
    RefreshWithoutFolderScroll,
}

const SEARCH_DEBOUNCE_MS: u64 = 120;

/// Select the presentation prerequisites for a sidebar transition.
/// First bool: wait for the Folder header/scroll reveal.
/// Second bool: wait for a NEW photo model (false for Folder stream reuse).
/// Keeping these separate is essential: Folder-to-Folder never replaces the
/// model, so waiting for a new generation would freeze the outgoing snapshot.
fn sidebar_crossfade_target(
    previous: sidebar::SidebarFilter,
    destination: sidebar::SidebarFilter,
    exact_photo_target: bool,
    viewer_visible: bool,
    reuse_folder_stream: bool,
) -> Option<(bool, bool)> {
    // Open in Folder is an exact-photo navigation, not a generic sidebar
    // click. It still replaces the visible model when coming from All/Recent
    // or search. Keep the outgoing gallery snapshot until the target photo
    // has actually been scrolled into view (the caller supplies that gate).
    if previous == destination || (viewer_visible && !exact_photo_target) {
        return None;
    }
    let folder_destination = matches!(destination, sidebar::SidebarFilter::Folder(_));
    let new_photo_model = !matches!(
        destination,
        sidebar::SidebarFilter::Library | sidebar::SidebarFilter::Albums
    ) && !reuse_folder_stream;
    Some((folder_destination, new_photo_model))
}

fn folder_destination_plan(
    exact_photo_target: bool,
    reuse_folder_stream: bool,
) -> FolderDestinationPlan {
    if !exact_photo_target {
        FolderDestinationPlan::Normal
    } else if reuse_folder_stream {
        FolderDestinationPlan::ReuseWithoutFolderScroll
    } else {
        FolderDestinationPlan::RefreshWithoutFolderScroll
    }
}

fn can_reuse_folder_stream_for_destination(
    has_folder_target: bool,
    folder_cache_available: bool,
) -> bool {
    has_folder_target && folder_cache_available
}

#[cfg(test)]
fn search_folder_focus_should_stop(
    still_on_target: bool,
    search_is_clear: bool,
    building: bool,
    pending: bool,
    focused: bool,
    attempt: u32,
) -> bool {
    if !still_on_target || !search_is_clear {
        return true;
    }
    if !pending {
        return true;
    }
    if building {
        return attempt >= 1200;
    }
    focused || attempt >= 240
}

fn should_ignore_cleared_search_event(
    suppressed: bool,
    cleared_query: Option<&str>,
    query: &str,
) -> bool {
    suppressed || cleared_query.is_some_and(|cleared| cleared == query)
}

#[cfg(test)]
mod open_in_folder_retry_tests {
    use crate::sidebar;
    use super::{
        can_reuse_folder_stream_for_destination, folder_destination_plan,
        open_in_folder_should_stop, search_folder_focus_should_stop, sidebar_crossfade_target,
        FolderDestinationPlan, should_ignore_cleared_search_event, SEARCH_DEBOUNCE_MS,
    };

    #[test]
    fn stops_immediately_after_photo_is_revealed() {
        assert!(open_in_folder_should_stop(false, true, 1));
    }

    #[test]
    fn keeps_retrying_while_stream_is_building_or_photo_is_not_revealed() {
        assert!(!open_in_folder_should_stop(true, false, 1));
        assert!(!open_in_folder_should_stop(false, false, 25));
    }

    #[test]
    fn hard_limit_still_stops_missing_photo_retry_loop() {
        assert!(open_in_folder_should_stop(false, false, 800));
    }

    #[test]
    fn search_debounce_stays_responsive() {
        assert!(SEARCH_DEBOUNCE_MS <= 150);
    }

    #[test]
    fn folder_destination_reuses_cached_full_stream_even_when_search_was_active() {
        assert!(can_reuse_folder_stream_for_destination(true, true));
        assert!(!can_reuse_folder_stream_for_destination(true, false));
        assert!(!can_reuse_folder_stream_for_destination(false, true));
    }

    #[test]
    fn search_folder_focus_waits_until_the_pending_target_is_consumed() {
        assert!(!search_folder_focus_should_stop(true, true, true, true, false, 1));
        assert!(!search_folder_focus_should_stop(true, true, false, true, false, 1));
        assert!(search_folder_focus_should_stop(true, true, false, false, false, 2));
        assert!(search_folder_focus_should_stop(true, true, false, true, true, 2));
    }

    #[test]
    fn search_folder_focus_stops_if_user_moves_on() {
        assert!(search_folder_focus_should_stop(false, true, true, true, false, 1));
        assert!(search_folder_focus_should_stop(true, false, true, true, false, 1));
    }

    #[test]
    fn stale_event_for_the_query_just_cleared_is_ignored_once() {
        assert!(should_ignore_cleared_search_event(true, None, "marianne"));
        assert!(should_ignore_cleared_search_event(false, Some("marianne"), "marianne"));
        assert!(!should_ignore_cleared_search_event(false, Some("marianne"), "maria"));
    }

    #[test]
    fn sidebar_crossfade_covers_gallery_and_folder_reuse() {
        use sidebar::SidebarFilter as Destination;
        // The regression: navigating within the continuous Folder stream
        // must fade even though no model replacement happens.
        assert_eq!(
            sidebar_crossfade_target(
                Destination::Folder(267), Destination::Folder(1), false, false, true,
            ),
            Some((true, false))
        );
        // Other photo destinations replace the gallery model.
        for dest in [
            Destination::All,
            Destination::Favorites,
            Destination::RecentlyAdded,
            Destination::History,
            Destination::Album(3),
        ] {
            assert_eq!(
                sidebar_crossfade_target(Destination::Folder(1), dest, false, false, false),
                Some((false, true)),
            );
        }
        // The two home surfaces replace GTK Stack pages, not photo models.
        assert_eq!(
            sidebar_crossfade_target(
                Destination::Folder(1), Destination::Library, false, false, false,
            ),
            Some((false, false))
        );
        assert_eq!(
            sidebar_crossfade_target(
                Destination::Library, Destination::Albums, false, false, false,
            ),
            Some((false, false))
        );
        // Entering a Folder from a different page must wait for its
        // model AND the deferred scroll/header restoration.
        assert_eq!(
            sidebar_crossfade_target(
                Destination::History, Destination::Folder(267), false, false, false,
            ),
            Some((true, true))
        );
        assert_eq!(
            sidebar_crossfade_target(
                Destination::Folder(1), Destination::Folder(1), false, false, true,
            ),
            None
        );
        // Open in Folder must keep a snapshot while a new folder stream is
        // built and the exact photo is selected. The caller adds an extra
        // exact-photo readiness gate before revealing the destination.
        assert_eq!(
            sidebar_crossfade_target(
                Destination::Library, Destination::Folder(1), true, false, false,
            ),
            Some((true, true))
        );
        // Existing folder stream: no model replacement, but still wait for
        // the exact photo to reach its destination viewport.
        assert_eq!(
            sidebar_crossfade_target(
                Destination::Folder(1), Destination::Folder(2), true, true, true,
            ),
            Some((true, false))
        );
        assert_eq!(
            sidebar_crossfade_target(
                Destination::Library, Destination::Folder(1), false, true, false,
            ),
            None
        );
    }

    #[test]
    fn exact_photo_navigation_never_schedules_generic_folder_scroll() {
        assert_eq!(
            folder_destination_plan(true, true),
            FolderDestinationPlan::ReuseWithoutFolderScroll
        );
        assert_eq!(
            folder_destination_plan(true, false),
            FolderDestinationPlan::RefreshWithoutFolderScroll
        );
        assert_eq!(
            folder_destination_plan(false, true),
            FolderDestinationPlan::Normal
        );
        assert_eq!(
            folder_destination_plan(false, false),
            FolderDestinationPlan::Normal
        );
    }
}
