//! Gallery scroll motion: what a gallery does while its scroller moves.
//!
//! Every change of a gallery's scroll position goes through one
//! [`ScrollMotion`]. It decides whether the move is a **scrub** (a direct
//! scrollbar drag that jumps more than about a page), how often a scrub may
//! ask for its destination's thumbnails, when scrolling has **settled**, and
//! what the per-frame thumbnail pump does on each frame after a move.
//!
//! The rules are plain values; the GTK adapter in `window/build.rs` turns
//! the answers into thumbnail queues, refreshes and timers. All Photos and
//! Folders each have a [`ScrollMotionSpec`]; their numbers sit side by side
//! here so they can be compared and, if wanted, matched.

use std::time::{Duration, Instant};

/// The numbers one gallery scrolls by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ScrollMotionSpec {
    /// A move of at least this many pages starts a scrub...
    pub scrub_pages: f64,
    /// ...and of at least this many pixels.
    pub scrub_min_px: f64,
    /// Scrolling has settled once this long passes without a move...
    pub settle_after: Duration,
    /// ...counted only while scrubbing (All Photos), or after every move.
    pub settle_only_while_scrubbing: bool,
    /// Each move keeps the per-frame thumbnail pump running this many frames.
    pub pump_frames: u8,
    /// The pump warms thumbnails ahead of the scroll every this many frames...
    pub warm_every: u8,
    /// ...but not straight after a scrub move.
    pub warm_after_scrub: WarmAfterScrub,
    /// A scrub asks for its destination's thumbnails at most this often...
    pub sample_every: Duration,
    /// ...on moves or on pump frames.
    pub sample_on: SampleOn,
    /// Visible tiles a pump frame queues when not scrubbing.
    pub visible_tiles: usize,
    /// Tiles a warm-ahead pass may load.
    pub warm_tiles: usize,
    /// Tiles a scrub target sample (and the settle) may queue.
    pub target_tiles: usize,
}

/// Where a scrub samples its target. GtkAdjustment and GtkListView emit
/// tiny ±1 px corrections during a drag; coalescing keeps the decode queue
/// owned by one destination instead of one per correction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SampleOn {
    /// In the move itself (All Photos).
    Move,
    /// In the next pump frame, after the list has rebound its rows (Folders).
    Frame,
}

/// When warming ahead resumes during a scrub.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum WarmAfterScrub {
    /// After this many pump frames without a scrub move (All Photos).
    QuietFrames(u8),
    /// Only once the scrub has settled (Folders).
    Settled,
}

/// What the thumbnail pump does on one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PumpFrame {
    /// A scrub is under way: the scrub target owns the decode queue, so only
    /// show thumbnails already decoded; otherwise queue the visible tiles.
    pub scrubbing: bool,
    /// Warm thumbnails ahead of the scroll direction this frame.
    pub warm_ahead: bool,
    /// Queue the scrub target (the latest position) now.
    pub sample: bool,
}

/// All Photos (the GridView gallery).
pub(super) const ALL_PHOTOS: ScrollMotionSpec = ScrollMotionSpec {
    scrub_pages: 1.75,
    scrub_min_px: 900.0,
    settle_after: Duration::from_millis(90),
    settle_only_while_scrubbing: true,
    pump_frames: 12,
    warm_every: 3,
    warm_after_scrub: WarmAfterScrub::QuietFrames(6),
    sample_every: Duration::from_millis(40),
    sample_on: SampleOn::Move,
    visible_tiles: 128,
    warm_tiles: 32,
    target_tiles: 192,
};

/// Folders (the sectioned list gallery).
pub(super) const FOLDERS: ScrollMotionSpec = ScrollMotionSpec {
    scrub_pages: 1.60,
    scrub_min_px: 0.0,
    settle_after: Duration::from_millis(110),
    settle_only_while_scrubbing: false,
    pump_frames: 12,
    warm_every: 3,
    warm_after_scrub: WarmAfterScrub::Settled,
    sample_every: Duration::from_millis(50),
    sample_on: SampleOn::Frame,
    visible_tiles: 96,
    warm_tiles: 24,
    target_tiles: 192,
};

/// What a move means for the gallery.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Moved {
    /// This move started a scrub.
    pub scrub_started: bool,
    /// A scrub is under way (started now or earlier).
    pub scrubbing: bool,
    /// Arm a settle check: call [`ScrollMotion::settle`] with this ticket
    /// once `spec.settle_after` has passed.
    pub settle: Option<SettleTicket>,
    /// Queue the scrub target (this position) now.
    pub sample: bool,
}

/// Names one armed settle check. Only the newest one settles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SettleTicket(u64);

pub(super) struct ScrollMotion {
    spec: ScrollMotionSpec,
    scrubbing: bool,
    /// The newest armed settle check.
    settle_ticket: u64,
    /// Pump frames left from the last move.
    pump_left: u8,
    /// Pump frames left before warming may resume after a scrub move.
    scrub_quiet_left: u8,
    /// Counts pump frames, for the warm-ahead cadence.
    phase: u8,
    /// When this scrub last sampled its target.
    last_sample: Option<Instant>,
}

impl ScrollMotion {
    pub(super) fn new(spec: ScrollMotionSpec) -> Self {
        Self {
            spec,
            scrubbing: false,
            settle_ticket: 0,
            pump_left: 0,
            scrub_quiet_left: 0,
            phase: 0,
            last_sample: None,
        }
    }

    /// The settle check `ticket` came due. True (and the scrub ends) when
    /// no move came after it; false for a check a newer move replaced.
    pub(super) fn settle(&mut self, ticket: SettleTicket) -> bool {
        if ticket.0 != self.settle_ticket {
            return false;
        }
        self.scrubbing = false;
        self.last_sample = None;
        true
    }

    /// One frame of the thumbnail pump, or `None` once it has run out.
    pub(super) fn frame(&mut self, now: Instant) -> Option<PumpFrame> {
        if self.pump_left == 0 {
            return None;
        }
        self.pump_left -= 1;
        let after_scrub_move = self.scrub_quiet_left > 0;
        self.scrub_quiet_left = self.scrub_quiet_left.saturating_sub(1);
        self.phase = self.phase.wrapping_add(1);
        let held = match self.spec.warm_after_scrub {
            WarmAfterScrub::QuietFrames(_) => after_scrub_move,
            WarmAfterScrub::Settled => self.scrubbing,
        };
        Some(PumpFrame {
            scrubbing: self.scrubbing,
            warm_ahead: !held && self.phase % self.spec.warm_every == 0,
            sample: self.take_sample(SampleOn::Frame, now),
        })
    }

    /// The scroll position moved by `delta` px with a page of `page` px.
    pub(super) fn moved(&mut self, delta: f64, page: f64, now: Instant) -> Moved {
        let threshold = (page * self.spec.scrub_pages).max(self.spec.scrub_min_px);
        let scrub_started = delta.abs() >= threshold;
        self.scrubbing |= scrub_started;
        self.pump_left = self.spec.pump_frames;
        if let (true, WarmAfterScrub::QuietFrames(frames)) =
            (self.scrubbing, self.spec.warm_after_scrub)
        {
            self.scrub_quiet_left = frames;
        }
        let settle = (self.scrubbing || !self.spec.settle_only_while_scrubbing).then(|| {
            self.settle_ticket = self.settle_ticket.wrapping_add(1);
            SettleTicket(self.settle_ticket)
        });
        let sample = self.take_sample(SampleOn::Move, now);
        Moved {
            scrub_started,
            scrubbing: self.scrubbing,
            settle,
            sample,
        }
    }

    /// True (and recorded) when a scrub may sample its target here and now.
    fn take_sample(&mut self, here: SampleOn, now: Instant) -> bool {
        if !self.scrubbing || self.spec.sample_on != here {
            return false;
        }
        let due = self.last_sample.map_or(true, |last| {
            now.duration_since(last) >= self.spec.sample_every
        });
        if due {
            self.last_sample = Some(now);
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One start time shared by a test's calls.
    fn t0() -> Instant {
        thread_local!(static START: Instant = Instant::now());
        START.with(|start| *start)
    }

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    /// (spec, page px, move px, starts a scrub)
    const SCRUB_STARTS: &[(&str, ScrollMotionSpec, f64, f64, bool)] = &[
        // All Photos: 1.75 pages, but never under 900 px.
        ("photos, one page", ALL_PHOTOS, 800.0, 800.0, false),
        ("photos, 1.75 pages", ALL_PHOTOS, 800.0, 1400.0, true),
        (
            "photos, just under 1.75 pages",
            ALL_PHOTOS,
            800.0,
            1399.0,
            false,
        ),
        ("photos, upwards", ALL_PHOTOS, 800.0, -1400.0, true),
        (
            "photos, small page: 900 px floor",
            ALL_PHOTOS,
            400.0,
            899.0,
            false,
        ),
        (
            "photos, small page at the floor",
            ALL_PHOTOS,
            400.0,
            900.0,
            true,
        ),
        // Folders: 1.60 pages, no floor.
        ("folders, one page", FOLDERS, 800.0, 800.0, false),
        ("folders, 1.60 pages", FOLDERS, 800.0, 1280.0, true),
        ("folders, just under", FOLDERS, 800.0, 1279.0, false),
        ("folders, small page", FOLDERS, 400.0, 640.0, true),
        ("folders, upwards", FOLDERS, 800.0, -1300.0, true),
        // A wheel click (160 px) never scrubs.
        ("photos, wheel click", ALL_PHOTOS, 800.0, 160.0, false),
        ("folders, wheel click", FOLDERS, 800.0, 160.0, false),
    ];

    #[test]
    fn a_move_of_more_than_about_a_page_starts_a_scrub() {
        for (name, spec, page, delta, starts) in SCRUB_STARTS {
            let mut motion = ScrollMotion::new(*spec);
            let moved = motion.moved(*delta, *page, t0());
            assert_eq!(moved.scrub_started, *starts, "{name}");
            assert_eq!(moved.scrubbing, *starts, "{name}");
        }
    }

    /// The 1-based pump frames that warm ahead, over the whole pump.
    fn warm_frames(motion: &mut ScrollMotion) -> Vec<usize> {
        std::iter::from_fn(|| motion.frame(t0()))
            .enumerate()
            .filter(|(_, frame)| frame.warm_ahead)
            .map(|(index, _)| index + 1)
            .collect()
    }

    #[test]
    fn each_move_runs_the_pump_for_twelve_frames() {
        for spec in [ALL_PHOTOS, FOLDERS] {
            let mut motion = ScrollMotion::new(spec);
            assert_eq!(motion.frame(t0()), None, "{spec:?}: no move, no pump");
            motion.moved(160.0, 800.0, t0());
            for _ in 0..5 {
                assert!(motion.frame(t0()).is_some());
            }
            // A move mid-pump starts the count again.
            motion.moved(160.0, 800.0, t0());
            assert_eq!(
                std::iter::from_fn(|| motion.frame(t0())).count(),
                12,
                "{spec:?}"
            );
        }
    }

    #[test]
    fn the_pump_warms_ahead_every_third_frame() {
        for spec in [ALL_PHOTOS, FOLDERS] {
            let mut motion = ScrollMotion::new(spec);
            motion.moved(160.0, 800.0, t0());
            assert_eq!(warm_frames(&mut motion), vec![3, 6, 9, 12], "{spec:?}");
        }
    }

    #[test]
    fn during_a_scrub_all_photos_warms_after_six_quiet_frames_folders_not_at_all() {
        let mut photos = ScrollMotion::new(ALL_PHOTOS);
        photos.moved(3000.0, 800.0, t0());
        assert!(photos.frame(t0()).unwrap().scrubbing);
        // Pump frames 1..=6 follow the scrub move; frames 9 and 12 warm,
        // listed from frame 2 on.
        assert_eq!(warm_frames(&mut photos), vec![8, 11]);

        let mut folders = ScrollMotion::new(FOLDERS);
        let ticket = folders.moved(3000.0, 800.0, t0()).settle.unwrap();
        assert!(folders.frame(t0()).unwrap().scrubbing);
        assert_eq!(warm_frames(&mut folders), Vec::<usize>::new());
        // Settled: the next move warms again (the cadence carries on).
        assert!(folders.settle(ticket));
        folders.moved(160.0, 800.0, t0());
        let next = std::iter::from_fn(|| folders.frame(t0())).collect::<Vec<_>>();
        assert!(next.iter().all(|frame| !frame.scrubbing));
        assert_eq!(next.iter().filter(|frame| frame.warm_ahead).count(), 4);
    }

    #[test]
    fn a_scrub_lasts_until_scrolling_settles() {
        for spec in [ALL_PHOTOS, FOLDERS] {
            let mut motion = ScrollMotion::new(spec);
            motion.moved(3000.0, 800.0, t0());
            // The end of a drag moves the thumb only a little; still the scrub.
            let small = motion.moved(2.0, 800.0, t0());
            assert!(small.scrubbing && !small.scrub_started, "{spec:?}");
            assert!(motion.settle(small.settle.unwrap()), "{spec:?}");
            assert!(!motion.moved(2.0, 800.0, t0()).scrubbing, "{spec:?}");
        }
    }

    #[test]
    fn only_the_newest_settle_check_settles() {
        for spec in [ALL_PHOTOS, FOLDERS] {
            let mut motion = ScrollMotion::new(spec);
            let first = motion.moved(3000.0, 800.0, t0()).settle.unwrap();
            let second = motion.moved(40.0, 800.0, t0()).settle.unwrap();
            assert!(
                !motion.settle(first),
                "{spec:?}: a newer move came after it"
            );
            assert!(
                motion.moved(1.0, 800.0, t0()).scrubbing,
                "{spec:?}: still dragging"
            );
            let third = motion.moved(1.0, 800.0, t0()).settle.unwrap();
            assert!(!motion.settle(second), "{spec:?}");
            assert!(motion.settle(third), "{spec:?}");
        }
    }

    #[test]
    fn all_photos_waits_for_quiet_only_while_scrubbing_folders_after_every_move() {
        // A wheel click or keyboard step, no scrub.
        assert_eq!(
            ScrollMotion::new(ALL_PHOTOS)
                .moved(160.0, 800.0, t0())
                .settle,
            None
        );
        let mut folders = ScrollMotion::new(FOLDERS);
        let ticket = folders
            .moved(160.0, 800.0, t0())
            .settle
            .expect("Folders settles every move");
        assert!(folders.settle(ticket));
        assert_eq!(ALL_PHOTOS.settle_after, Duration::from_millis(90));
        assert_eq!(FOLDERS.settle_after, Duration::from_millis(110));
    }

    #[test]
    fn all_photos_samples_the_scrub_target_on_moves_at_most_every_40_ms() {
        let start = t0();
        let mut photos = ScrollMotion::new(ALL_PHOTOS);
        assert!(
            !photos.moved(160.0, 800.0, at(start, 0)).sample,
            "not scrubbing"
        );
        assert!(
            photos.moved(3000.0, 800.0, at(start, 10)).sample,
            "the jump itself"
        );
        assert!(!photos.moved(1.0, 800.0, at(start, 49)).sample);
        assert!(photos.moved(1.0, 800.0, at(start, 50)).sample);
        // The pump never samples for All Photos.
        assert!(!photos.frame(at(start, 200)).unwrap().sample);
        // Settling forgets the last sample: the next scrub samples at once.
        let ticket = photos.moved(1.0, 800.0, at(start, 60)).settle.unwrap();
        assert!(photos.settle(ticket));
        assert!(photos.moved(3000.0, 800.0, at(start, 70)).sample);
    }

    #[test]
    fn folders_sample_the_scrub_target_in_the_pump_at_most_every_50_ms() {
        let start = t0();
        let mut folders = ScrollMotion::new(FOLDERS);
        folders.moved(160.0, 800.0, at(start, 0));
        assert!(
            !folders.frame(at(start, 1)).unwrap().sample,
            "not scrubbing"
        );
        assert!(
            !folders.moved(3000.0, 800.0, at(start, 10)).sample,
            "Folders sample in the pump"
        );
        assert!(
            folders.frame(at(start, 10)).unwrap().sample,
            "the jump's own frame"
        );
        assert!(!folders.frame(at(start, 59)).unwrap().sample);
        assert!(folders.frame(at(start, 60)).unwrap().sample);
        let ticket = folders.moved(1.0, 800.0, at(start, 70)).settle.unwrap();
        assert!(folders.settle(ticket));
        assert!(!folders.frame(at(start, 200)).unwrap().sample, "settled");
        folders.moved(3000.0, 800.0, at(start, 210));
        assert!(
            folders.frame(at(start, 211)).unwrap().sample,
            "a new scrub samples at once"
        );
    }
}
