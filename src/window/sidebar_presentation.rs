//! Sidebar presentation: every decision about how the left navigation is
//! shown, in one place and free of GTK.
//!
//! The left navigation has two modes (the full sidebar, or the compact icon
//! rail) and two places it can be presented (the windowed layout, or the
//! immersive overlay). The split view can also be collapsed by the responsive
//! breakpoint, and the full sidebar can be pinned or hover-peeked.
//!
//! This module turns those inputs into a target: which widgets are visible,
//! how wide the drawer may be, when hover may peek it open and when the
//! breakpoint collapses the split. Widget code in `layout.rs`,
//! `immersive.rs` and `build.rs` asks here and only applies the answer.

use super::SIDEBAR_MAX_WIDTH;
use super::SIDEBAR_MIN_WIDTH;

/// Persisted as `"true"` when the compact rail is chosen.
pub(super) const SIDEBAR_MODE_SETTING_KEY: &str = "compact-sidebar-enabled";

/// The compact rail's narrowest width: icons only.
pub(super) const COMPACT_RAIL_MIN_WIDTH: f64 = 44.0;
/// The compact rail's widest width.
pub(super) const COMPACT_RAIL_MAX_WIDTH: f64 = 300.0;
/// From this width the compact rail shows labels and inline details.
pub(super) const COMPACT_RAIL_LABEL_WIDTH: f64 = 90.0;

/// The window narrows past this and the split collapses to an overlay drawer.
pub(super) const BREAKPOINT_COLLAPSE_WIDTH: i32 = 1050;
/// The window widens past this and the split returns to side-by-side. The gap
/// to [`BREAKPOINT_COLLAPSE_WIDTH`] stops chattering around one width.
pub(super) const BREAKPOINT_EXPAND_WIDTH: i32 = 1080;

/// Which left navigation the user chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SidebarMode {
    /// The full sidebar inside the split view.
    Normal,
    /// The compact icon rail beside the split view.
    Compact,
}

impl SidebarMode {
    pub(super) fn from_setting(value: Option<&str>) -> Self {
        if value == Some("true") {
            Self::Compact
        } else {
            Self::Normal
        }
    }

    pub(super) fn from_normal(normal: bool) -> Self {
        if normal {
            Self::Normal
        } else {
            Self::Compact
        }
    }

    pub(super) fn setting_value(self) -> &'static str {
        match self {
            Self::Normal => "false",
            Self::Compact => "true",
        }
    }

    pub(super) fn is_normal(self) -> bool {
        self == Self::Normal
    }
}

/// Which widgets of the left navigation are part of the layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SidebarTarget {
    /// The compact rail's dock beside the split view.
    pub(super) compact_dock: bool,
    /// The full sidebar column and its content.
    pub(super) full_sidebar: bool,
    /// The full sidebar's resize handle.
    pub(super) resize_handle: bool,
    /// The split view's own show/hide swipe gestures.
    pub(super) split_gestures: bool,
    /// The header button that switches back to the full sidebar.
    pub(super) switch_to_normal_button: bool,
}

/// The layout target for `mode`. An immersive session hides the compact dock
/// and keeps the full sidebar's widgets ready, because the presentation
/// overlay shows whichever page the mode selects.
fn sidebar_target(mode: SidebarMode, immersive: bool) -> SidebarTarget {
    let normal = mode.is_normal();
    if immersive {
        return SidebarTarget {
            compact_dock: false,
            full_sidebar: true,
            resize_handle: true,
            split_gestures: false,
            switch_to_normal_button: !normal,
        };
    }
    SidebarTarget {
        compact_dock: !normal,
        full_sidebar: normal,
        resize_handle: normal,
        split_gestures: normal,
        switch_to_normal_button: !normal,
    }
}

/// Whether switching the windowed layout to `mode` shows the full sidebar.
/// A collapsed split waits for the user or hover to open its drawer.
pub(super) fn split_shows_sidebar_after_mode_switch(mode: SidebarMode, collapsed: bool) -> bool {
    mode.is_normal() && !collapsed
}

/// Small controls around the sidebar that follow whether it is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SidebarChrome {
    /// The thin left-edge strip that peeks the hidden sidebar open.
    pub(super) hover_strip: bool,
    /// The header button that shows (pins) the hidden full sidebar.
    pub(super) show_button: bool,
    /// The sidebar's own header hosts the window's start title buttons.
    pub(super) sidebar_hosts_title_buttons: bool,
}

fn sidebar_chrome(mode: SidebarMode, immersive: bool, shows_sidebar: bool) -> SidebarChrome {
    let normal = mode.is_normal();
    SidebarChrome {
        hover_strip: (normal || immersive) && !shows_sidebar,
        show_button: normal && !shows_sidebar,
        sidebar_hosts_title_buttons: normal && shows_sidebar,
    }
}

/// The widest the full sidebar may be. A collapsed split shows it as an
/// overlay drawer at the user's saved drawer width, never wider than 70% of
/// the split, never narrower than the sidebar minimum.
pub(super) fn max_sidebar_width(collapsed: bool, saved_drawer_width: f64, split_width: i32) -> f64 {
    if collapsed {
        saved_drawer_width.min((f64::from(split_width) * 0.70).max(SIDEBAR_MIN_WIDTH))
    } else {
        SIDEBAR_MAX_WIDTH
    }
}

/// Whether hovering the left edge may peek the hidden full sidebar open.
/// Immersive mode has its own edge reveal. A pinned sidebar the user hid
/// stays hidden, except when the breakpoint collapsed it.
pub(super) fn hover_peek_allowed(
    mode: SidebarMode,
    immersive: bool,
    shows_sidebar: bool,
    pinned: bool,
    collapsed: bool,
) -> bool {
    !immersive && mode.is_normal() && !shows_sidebar && (!pinned || collapsed)
}

/// What the responsive breakpoint does at a window width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BreakpointStep {
    /// Collapse the split to an overlay drawer.
    Collapse,
    /// Return the split to side-by-side.
    Expand,
}

pub(super) fn breakpoint_step(
    immersive: bool,
    collapsed: bool,
    in_transition: bool,
    window_width: i32,
) -> Option<BreakpointStep> {
    if immersive || in_transition {
        return None;
    }
    if !collapsed && window_width <= BREAKPOINT_COLLAPSE_WIDTH {
        Some(BreakpointStep::Collapse)
    } else if collapsed && window_width >= BREAKPOINT_EXPAND_WIDTH {
        Some(BreakpointStep::Expand)
    } else {
        None
    }
}

pub(super) fn clamp_compact_rail_width(width: f64) -> f64 {
    width.clamp(COMPACT_RAIL_MIN_WIDTH, COMPACT_RAIL_MAX_WIDTH)
}

/// Whether the compact rail is wide enough for labels and inline details.
pub(super) fn compact_rail_shows_labels(width: f64) -> bool {
    width >= COMPACT_RAIL_LABEL_WIDTH
}

// ---------------------------------------------------------------------------
// The state machine: events in, one complete target out.
// ---------------------------------------------------------------------------

/// Where the left navigation's shell lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SidebarPlacement {
    /// Inside the split view (the windowed layout).
    Split,
    /// On the gallery as the presentation-only immersive overlay.
    ImmersiveOverlay,
}

/// Which page the sidebar shell holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SidebarPage {
    /// The full sidebar column.
    Full,
    /// The compact rail's page (only ever inside the shell while immersive).
    Compact,
}

/// Everything the widget adapter writes, for one state. Applying the same
/// target twice must change nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SidebarScene {
    pub(super) layout: SidebarTarget,
    pub(super) chrome: SidebarChrome,
    pub(super) placement: SidebarPlacement,
    pub(super) page: SidebarPage,
    /// Immersive only: the overlay is slid in.
    pub(super) overlay_revealed: bool,
    pub(super) split_collapsed: bool,
    pub(super) split_shows_sidebar: bool,
    /// The split's own pin-sidebar property (off while immersive).
    pub(super) split_pin: bool,
    /// The persistent pin, mirrored into `sidebar.rs`.
    pub(super) pinned: bool,
    /// A hover peek is open, mirrored into `sidebar.rs`.
    pub(super) hover_open: bool,
    pub(super) compact_width: f64,
    pub(super) drawer_width: f64,
}

/// Timed follow-ups the adapter schedules. When one fires, the adapter feeds
/// it back with the same generation; a newer state makes it a no-op.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SidebarTimer {
    /// About 140 ms after the pointer leaves a hover peek.
    LeaveCheck { generation: u64 },
    /// One frame after the breakpoint collapsed an open sidebar, so the
    /// drawer slides out instead of being cut.
    BreakpointFrame { generation: u64 },
    /// One main-loop turn after the breakpoint expanded the split.
    BreakpointIdle { generation: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SidebarEvent {
    ModeChosen(SidebarMode),
    /// The pin at entry is kept and restored on exit.
    ImmersiveEntered,
    ImmersiveLeft,
    BreakpointCrossed {
        window_width: i32,
    },
    /// Pointer rested on the windowed left-edge strip.
    EdgeHovered,
    ImmersiveRevealRequested,
    ImmersiveHideRequested,
    /// The sidebar's own pin button: pins a peek, unpins a pinned sidebar.
    PinPressed,
    /// The header arrow that shows a hidden sidebar.
    ShowPressed,
    PointerLeftSidebar,
    /// The pointer came back: a pending leave check must not hide.
    PointerEnteredSidebar,
    /// `held` while a resize drag or context menu keeps the peek open.
    LeaveSettled {
        generation: u64,
        held: bool,
    },
    BreakpointFrameElapsed {
        generation: u64,
    },
    /// Reveals the expanded split if the sidebar is pinned.
    BreakpointIdleElapsed {
        generation: u64,
    },
    /// The split changed show-sidebar on its own (swipe gesture).
    SplitShowChanged(bool),
    /// A destination or search was chosen: a collapsed drawer gets out of
    /// the way.
    DrawerDismissed,
    CompactWidthChanged(f64),
    DrawerWidthChanged(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WindowedSnapshot {
    /// The mode at entry; a mode chosen during the session opens like a
    /// windowed mode switch on exit.
    mode: SidebarMode,
    open: bool,
    collapsed: bool,
    pinned: bool,
}

/// The sidebar presentation module. Owns the logical sidebar state; the
/// adapter owns animation progress.
#[derive(Clone, Debug)]
pub(super) struct SidebarPresentation {
    mode: SidebarMode,
    collapsed: bool,
    open: bool,
    pinned: bool,
    hover_open: bool,
    /// `Some` while immersive: the windowed layout to restore.
    immersive: Option<WindowedSnapshot>,
    revealed: bool,
    breakpoint_in_transition: bool,
    compact_width: f64,
    drawer_width: f64,
    /// Invalidates pending leave checks.
    generation: u64,
    /// Invalidates pending breakpoint follow-ups. Separate from
    /// `generation`: the breakpoint's own show/hide must not make its
    /// follow-up stale.
    breakpoint_generation: u64,
}

impl SidebarPresentation {
    pub(super) fn new(mode: SidebarMode, compact_width: f64, drawer_width: f64) -> Self {
        Self {
            mode,
            collapsed: false,
            open: mode.is_normal(),
            pinned: true,
            hover_open: false,
            immersive: None,
            revealed: false,
            breakpoint_in_transition: false,
            compact_width: clamp_compact_rail_width(compact_width),
            drawer_width: drawer_width.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH),
            generation: 0,
            breakpoint_generation: 0,
        }
    }

    pub(super) fn mode(&self) -> SidebarMode {
        self.mode
    }

    pub(super) fn is_immersive(&self) -> bool {
        self.immersive.is_some()
    }

    pub(super) fn is_revealed(&self) -> bool {
        self.immersive.is_some() && self.revealed
    }

    /// Take the split's real windowed state as the truth. Used while some
    /// windowed writers (hover, pin, breakpoint) still drive the split
    /// directly; once they dispatch events this becomes unnecessary.
    pub(super) fn observe_windowed(&mut self, collapsed: bool, open: bool) {
        if self.immersive.is_none() {
            self.collapsed = collapsed;
            self.open = open;
            if !open {
                self.hover_open = false;
            }
        }
    }

    fn next_generation(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn next_breakpoint_generation(&mut self) -> u64 {
        self.breakpoint_generation = self.breakpoint_generation.wrapping_add(1);
        self.breakpoint_generation
    }

    /// Apply one event. Returns a timer to schedule, if any; read the new
    /// state with [`Self::scene`].
    pub(super) fn handle(&mut self, event: SidebarEvent) -> Option<SidebarTimer> {
        use SidebarEvent::*;
        match event {
            ModeChosen(mode) => {
                self.mode = mode;
                if self.immersive.is_none() {
                    self.open = split_shows_sidebar_after_mode_switch(mode, self.collapsed);
                    self.hover_open = false;
                    self.next_generation();
                }
                None
            }
            ImmersiveEntered => {
                if self.immersive.is_none() {
                    self.immersive = Some(WindowedSnapshot {
                        mode: self.mode,
                        open: self.open,
                        collapsed: self.collapsed,
                        pinned: self.pinned,
                    });
                    self.revealed = false;
                    self.hover_open = false;
                    self.breakpoint_in_transition = false;
                    self.next_generation();
                    self.next_breakpoint_generation();
                }
                None
            }
            ImmersiveLeft => {
                if let Some(snapshot) = self.immersive.take() {
                    self.revealed = false;
                    self.collapsed = snapshot.collapsed;
                    self.pinned = snapshot.pinned;
                    self.open = if self.mode == snapshot.mode {
                        snapshot.open && self.mode.is_normal()
                    } else {
                        split_shows_sidebar_after_mode_switch(self.mode, self.collapsed)
                    };
                    self.hover_open = false;
                    self.next_generation();
                }
                None
            }
            ImmersiveRevealRequested | ImmersiveHideRequested => {
                if self.immersive.is_some() {
                    self.revealed = matches!(event, ImmersiveRevealRequested);
                }
                None
            }
            BreakpointCrossed { window_width } => {
                match breakpoint_step(
                    self.immersive.is_some(),
                    self.collapsed,
                    self.breakpoint_in_transition,
                    window_width,
                )? {
                    BreakpointStep::Collapse => {
                        self.collapsed = true;
                        if !self.mode.is_normal() {
                            self.open = false;
                        }
                        let generation = self.next_breakpoint_generation();
                        if self.open {
                            // Keep it visible as an overlay for one frame.
                            self.breakpoint_in_transition = true;
                            Some(SidebarTimer::BreakpointFrame { generation })
                        } else {
                            None
                        }
                    }
                    BreakpointStep::Expand => {
                        // Expand while hidden, reveal on the next turn.
                        self.open = false;
                        self.hover_open = false;
                        self.collapsed = false;
                        self.breakpoint_in_transition = true;
                        let generation = self.next_breakpoint_generation();
                        Some(SidebarTimer::BreakpointIdle { generation })
                    }
                }
            }
            BreakpointFrameElapsed { generation } => {
                if generation == self.breakpoint_generation {
                    self.open = false;
                    self.hover_open = false;
                    self.breakpoint_in_transition = false;
                }
                None
            }
            BreakpointIdleElapsed { generation } => {
                if generation == self.breakpoint_generation {
                    if self.mode.is_normal() && self.pinned {
                        self.open = true;
                    }
                    self.breakpoint_in_transition = false;
                }
                None
            }
            EdgeHovered => {
                if hover_peek_allowed(
                    self.mode,
                    self.immersive.is_some(),
                    self.open,
                    self.pinned,
                    self.collapsed,
                ) {
                    self.open = true;
                    self.hover_open = true;
                    self.next_generation();
                }
                None
            }
            PointerLeftSidebar => {
                if self.immersive.is_none() && self.hover_open {
                    let generation = self.next_generation();
                    Some(SidebarTimer::LeaveCheck { generation })
                } else {
                    None
                }
            }
            PointerEnteredSidebar => {
                if self.hover_open {
                    self.next_generation();
                }
                None
            }
            LeaveSettled { generation, held } => {
                if generation != self.generation || !self.hover_open {
                    return None;
                }
                if held {
                    return Some(SidebarTimer::LeaveCheck { generation });
                }
                self.open = false;
                self.hover_open = false;
                None
            }
            PinPressed => {
                if self.immersive.is_some() {
                    // The pin is the overlay's exit: restore the saved layout.
                    return self.handle(ImmersiveLeft);
                }
                self.next_generation();
                if self.pinned {
                    // Unpin: auto-hide starts with a real slide-out.
                    self.pinned = false;
                    self.open = false;
                } else {
                    // Pin a hover peek so it stays open.
                    self.pinned = true;
                }
                self.hover_open = false;
                None
            }
            ShowPressed => {
                if self.immersive.is_some() {
                    self.revealed = !self.revealed;
                } else {
                    self.next_generation();
                    self.pinned = true;
                    self.hover_open = false;
                    self.open = true;
                }
                None
            }
            SplitShowChanged(open) => {
                if self.immersive.is_none() && self.open != open {
                    self.open = open;
                    if !open {
                        self.hover_open = false;
                    }
                    self.next_generation();
                }
                None
            }
            DrawerDismissed => {
                if self.immersive.is_none() && self.collapsed && self.open {
                    self.open = false;
                    self.hover_open = false;
                    self.next_generation();
                }
                None
            }
            CompactWidthChanged(width) => {
                self.compact_width = clamp_compact_rail_width(width);
                None
            }
            DrawerWidthChanged(width) => {
                self.drawer_width = width.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
                None
            }
        }
    }

    /// The complete target for the current state.
    pub(super) fn scene(&self) -> SidebarScene {
        let immersive = self.immersive.is_some();
        let layout = sidebar_target(self.mode, immersive);
        let (placement, page, split_collapsed, split_shows_sidebar, split_pin) = if immersive {
            let page = if self.mode.is_normal() {
                SidebarPage::Full
            } else {
                SidebarPage::Compact
            };
            (SidebarPlacement::ImmersiveOverlay, page, true, false, false)
        } else {
            (
                SidebarPlacement::Split,
                SidebarPage::Full,
                self.collapsed,
                self.open && self.mode.is_normal(),
                true,
            )
        };
        SidebarScene {
            layout,
            // The immersive overlay is not the split's sidebar: while it is
            // revealed the edge strip and the show button stay, and the
            // window's title buttons stay in the content header.
            chrome: sidebar_chrome(self.mode, immersive, split_shows_sidebar),
            placement,
            page,
            overlay_revealed: self.is_revealed(),
            split_collapsed,
            split_shows_sidebar,
            split_pin,
            pinned: self
                .immersive
                .map_or(self.pinned, |snapshot| snapshot.pinned),
            hover_open: self.hover_open,
            compact_width: self.compact_width,
            drawer_width: self.drawer_width,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_round_trips_through_its_setting() {
        for mode in [SidebarMode::Normal, SidebarMode::Compact] {
            assert_eq!(SidebarMode::from_setting(Some(mode.setting_value())), mode);
        }
        assert_eq!(SidebarMode::from_setting(None), SidebarMode::Normal);
        assert_eq!(
            SidebarMode::from_setting(Some("garbage")),
            SidebarMode::Normal
        );
    }

    #[test]
    fn windowed_layout_shows_exactly_one_navigation() {
        for mode in [SidebarMode::Normal, SidebarMode::Compact] {
            let target = sidebar_target(mode, false);
            assert_ne!(target.compact_dock, target.full_sidebar, "{mode:?}");
            assert_eq!(target.resize_handle, target.full_sidebar);
            assert_eq!(target.split_gestures, mode.is_normal());
            assert_eq!(target.switch_to_normal_button, !mode.is_normal());
        }
    }

    #[test]
    fn immersive_hides_the_dock_and_never_swipes_the_split() {
        for mode in [SidebarMode::Normal, SidebarMode::Compact] {
            let target = sidebar_target(mode, true);
            assert!(!target.compact_dock);
            assert!(target.full_sidebar && target.resize_handle);
            assert!(!target.split_gestures);
            assert_eq!(target.switch_to_normal_button, !mode.is_normal());
        }
    }

    #[test]
    fn mode_switch_opens_the_sidebar_only_side_by_side() {
        assert!(split_shows_sidebar_after_mode_switch(
            SidebarMode::Normal,
            false
        ));
        assert!(!split_shows_sidebar_after_mode_switch(
            SidebarMode::Normal,
            true
        ));
        assert!(!split_shows_sidebar_after_mode_switch(
            SidebarMode::Compact,
            false
        ));
    }

    #[test]
    fn chrome_follows_whether_the_sidebar_is_on_screen() {
        let hidden = sidebar_chrome(SidebarMode::Normal, false, false);
        assert!(hidden.hover_strip && hidden.show_button && !hidden.sidebar_hosts_title_buttons);
        let shown = sidebar_chrome(SidebarMode::Normal, false, true);
        assert!(!shown.hover_strip && !shown.show_button && shown.sidebar_hosts_title_buttons);
        let compact = sidebar_chrome(SidebarMode::Compact, false, false);
        assert!(!compact.hover_strip && !compact.show_button);
        // Immersive keeps its edge strip in either mode.
        assert!(sidebar_chrome(SidebarMode::Compact, true, false).hover_strip);
    }

    #[test]
    fn collapsed_drawer_is_capped_by_the_split() {
        assert_eq!(max_sidebar_width(false, 300.0, 800), SIDEBAR_MAX_WIDTH);
        assert_eq!(max_sidebar_width(true, 300.0, 1000), 300.0);
        assert_eq!(max_sidebar_width(true, 300.0, 400), 280.0);
        assert_eq!(max_sidebar_width(true, 300.0, 100), SIDEBAR_MIN_WIDTH);
    }

    #[test]
    fn hover_peeks_only_an_unpinned_or_collapsed_hidden_sidebar() {
        let normal = SidebarMode::Normal;
        assert!(hover_peek_allowed(normal, false, false, false, false));
        assert!(!hover_peek_allowed(normal, false, false, true, false));
        assert!(hover_peek_allowed(normal, false, false, true, true));
        assert!(!hover_peek_allowed(normal, false, true, false, false));
        assert!(!hover_peek_allowed(normal, true, false, false, true));
        assert!(!hover_peek_allowed(
            SidebarMode::Compact,
            false,
            false,
            false,
            true
        ));
    }

    #[test]
    fn breakpoint_has_hysteresis_and_yields_to_immersive() {
        use BreakpointStep::*;
        assert_eq!(breakpoint_step(false, false, false, 1050), Some(Collapse));
        assert_eq!(breakpoint_step(false, false, false, 1060), None);
        assert_eq!(breakpoint_step(false, true, false, 1060), None);
        assert_eq!(breakpoint_step(false, true, false, 1080), Some(Expand));
        assert_eq!(breakpoint_step(false, false, true, 900), None);
        assert_eq!(breakpoint_step(true, false, false, 900), None);
    }

    #[test]
    fn compact_rail_width_and_labels() {
        assert_eq!(clamp_compact_rail_width(10.0), COMPACT_RAIL_MIN_WIDTH);
        assert_eq!(clamp_compact_rail_width(500.0), COMPACT_RAIL_MAX_WIDTH);
        assert!(!compact_rail_shows_labels(89.9));
        assert!(compact_rail_shows_labels(90.0));
    }

    // ----- transition tests -------------------------------------------------

    use SidebarEvent::*;

    fn normal() -> SidebarPresentation {
        SidebarPresentation::new(SidebarMode::Normal, 44.0, 300.0)
    }

    fn compact() -> SidebarPresentation {
        SidebarPresentation::new(SidebarMode::Compact, 120.0, 300.0)
    }

    fn shows_navigation(scene: &SidebarScene) -> bool {
        scene.split_shows_sidebar || scene.overlay_revealed || scene.layout.compact_dock
    }

    #[test]
    fn normal_compact_normal_round_trip() {
        let mut sidebar = normal();
        let start = sidebar.scene();
        assert!(start.split_shows_sidebar && !start.layout.compact_dock);
        sidebar.handle(ModeChosen(SidebarMode::Compact));
        let scene = sidebar.scene();
        assert!(
            scene.layout.compact_dock && !scene.split_shows_sidebar && !scene.layout.full_sidebar
        );
        sidebar.handle(ModeChosen(SidebarMode::Normal));
        assert_eq!(sidebar.scene(), start);
    }

    #[test]
    fn windowed_immersive_windowed_restores_the_exact_scene() {
        for mut sidebar in [normal(), compact()] {
            let before = sidebar.scene();
            sidebar.handle(ImmersiveEntered);
            let scene = sidebar.scene();
            assert_eq!(scene.placement, SidebarPlacement::ImmersiveOverlay);
            assert!(
                !shows_navigation(&scene),
                "entering immersive hides navigation"
            );
            assert!(scene.split_collapsed && !scene.split_pin && !scene.layout.split_gestures);
            sidebar.handle(ImmersiveLeft);
            assert_eq!(sidebar.scene(), before);
        }
    }

    #[test]
    fn immersive_reveal_hide_cycles_return_to_fully_hidden() {
        let mut sidebar = compact();
        sidebar.handle(ImmersiveEntered);
        let hidden = sidebar.scene();
        for _ in 0..20 {
            sidebar.handle(ImmersiveRevealRequested);
            let shown = sidebar.scene();
            assert!(shown.overlay_revealed);
            assert_eq!(shown.page, SidebarPage::Compact, "the current mode's page");
            assert_eq!(shown.placement, SidebarPlacement::ImmersiveOverlay);
            sidebar.handle(ImmersiveHideRequested);
            assert_eq!(sidebar.scene(), hidden);
        }
    }

    #[test]
    fn mode_change_while_immersive_switches_the_page_only() {
        let mut sidebar = normal();
        sidebar.handle(ImmersiveEntered);
        sidebar.handle(ImmersiveRevealRequested);
        sidebar.handle(ModeChosen(SidebarMode::Compact));
        let scene = sidebar.scene();
        assert!(scene.overlay_revealed && scene.page == SidebarPage::Compact);
        assert!(!scene.layout.compact_dock && scene.split_collapsed);
        sidebar.handle(ImmersiveLeft);
        let scene = sidebar.scene();
        assert!(scene.layout.compact_dock && !scene.split_shows_sidebar);
    }

    #[test]
    fn pin_in_immersive_leaves_and_restores_the_saved_layout() {
        let mut sidebar = normal();
        sidebar.handle(ImmersiveEntered);
        sidebar.handle(ImmersiveRevealRequested);
        sidebar.handle(PinPressed);
        let scene = sidebar.scene();
        assert!(!sidebar.is_immersive());
        assert!(scene.split_shows_sidebar && scene.pinned && !scene.split_collapsed);

        let mut sidebar = compact();
        sidebar.handle(ImmersiveEntered);
        sidebar.handle(PinPressed);
        let scene = sidebar.scene();
        assert!(scene.layout.compact_dock && !scene.split_shows_sidebar && !scene.split_collapsed);
    }

    #[test]
    fn pin_outside_immersive_toggles_the_pin() {
        let mut sidebar = normal();
        sidebar.handle(PinPressed);
        let scene = sidebar.scene();
        assert!(!scene.pinned && !scene.split_shows_sidebar);
        // Hover peek, then pin it so it stays.
        sidebar.handle(EdgeHovered);
        assert!(sidebar.scene().hover_open);
        sidebar.handle(PinPressed);
        let scene = sidebar.scene();
        assert!(scene.pinned && scene.split_shows_sidebar && !scene.hover_open);
        assert_eq!(
            sidebar.handle(PointerLeftSidebar),
            None,
            "a pinned sidebar never auto-hides"
        );
    }

    #[test]
    fn rapid_hover_and_leave_only_the_latest_leave_hides() {
        let mut sidebar = normal();
        sidebar.handle(PinPressed);
        sidebar.handle(EdgeHovered);
        let Some(SidebarTimer::LeaveCheck { generation: first }) =
            sidebar.handle(PointerLeftSidebar)
        else {
            panic!("leaving a peek schedules a check");
        };
        // Back in and out again before the first check fires.
        sidebar.handle(EdgeHovered);
        let Some(SidebarTimer::LeaveCheck { generation: second }) =
            sidebar.handle(PointerLeftSidebar)
        else {
            panic!("second leave");
        };
        sidebar.handle(LeaveSettled {
            generation: first,
            held: false,
        });
        assert!(
            sidebar.scene().split_shows_sidebar,
            "a stale leave check is ignored"
        );
        assert_eq!(
            sidebar.handle(LeaveSettled {
                generation: second,
                held: true
            }),
            Some(SidebarTimer::LeaveCheck { generation: second }),
            "a context menu keeps the peek and checks again"
        );
        sidebar.handle(LeaveSettled {
            generation: second,
            held: false,
        });
        let scene = sidebar.scene();
        assert!(!scene.split_shows_sidebar && !scene.hover_open && scene.chrome.hover_strip);
    }

    #[test]
    fn returning_to_the_sidebar_cancels_the_leave_check() {
        let mut sidebar = normal();
        sidebar.handle(PinPressed);
        sidebar.handle(EdgeHovered);
        let Some(SidebarTimer::LeaveCheck { generation }) = sidebar.handle(PointerLeftSidebar)
        else {
            panic!("leave check");
        };
        sidebar.handle(PointerEnteredSidebar);
        sidebar.handle(LeaveSettled {
            generation,
            held: false,
        });
        assert!(sidebar.scene().split_shows_sidebar && sidebar.scene().hover_open);
    }

    #[test]
    fn breakpoint_collapses_after_one_frame_and_reveals_pinned_on_expand() {
        let mut sidebar = normal();
        let Some(SidebarTimer::BreakpointFrame { generation }) =
            sidebar.handle(BreakpointCrossed { window_width: 1000 })
        else {
            panic!("an open sidebar slides out one frame after collapsing");
        };
        let scene = sidebar.scene();
        assert!(scene.split_collapsed && scene.split_shows_sidebar);
        assert_eq!(
            sidebar.handle(BreakpointCrossed { window_width: 900 }),
            None,
            "mid-transition"
        );
        sidebar.handle(BreakpointFrameElapsed { generation });
        assert!(!sidebar.scene().split_shows_sidebar);
        assert_eq!(
            sidebar.handle(BreakpointCrossed { window_width: 1060 }),
            None,
            "hysteresis"
        );

        let Some(SidebarTimer::BreakpointIdle { generation }) =
            sidebar.handle(BreakpointCrossed { window_width: 1200 })
        else {
            panic!("expanding reveals on the next turn");
        };
        let scene = sidebar.scene();
        assert!(!scene.split_collapsed && !scene.split_shows_sidebar);
        sidebar.handle(BreakpointIdleElapsed { generation });
        assert!(sidebar.scene().split_shows_sidebar);
    }

    #[test]
    fn breakpoint_is_ignored_while_immersive_and_compact_never_opens_the_split() {
        let mut sidebar = normal();
        sidebar.handle(ImmersiveEntered);
        assert_eq!(
            sidebar.handle(BreakpointCrossed { window_width: 600 }),
            None
        );
        let mut sidebar = compact();
        assert_eq!(
            sidebar.handle(BreakpointCrossed { window_width: 600 }),
            None
        );
        let Some(SidebarTimer::BreakpointIdle { generation }) =
            sidebar.handle(BreakpointCrossed { window_width: 1400 })
        else {
            panic!("expand");
        };
        sidebar.handle(BreakpointIdleElapsed { generation });
        assert!(!sidebar.scene().split_shows_sidebar && sidebar.scene().layout.compact_dock);
    }

    #[test]
    fn the_breakpoints_own_hide_does_not_make_its_follow_up_stale() {
        let mut sidebar = normal();
        let Some(SidebarTimer::BreakpointFrame { generation }) =
            sidebar.handle(BreakpointCrossed { window_width: 1000 })
        else {
            panic!("collapse");
        };
        // Writing the split emits show-sidebar notify, fed back as an event.
        sidebar.handle(SplitShowChanged(true));
        sidebar.handle(BreakpointFrameElapsed { generation });
        assert!(!sidebar.scene().split_shows_sidebar);
        let Some(SidebarTimer::BreakpointIdle { generation }) =
            sidebar.handle(BreakpointCrossed { window_width: 1400 })
        else {
            panic!("the breakpoint must not stay stuck mid-transition");
        };
        sidebar.handle(SplitShowChanged(false));
        sidebar.handle(BreakpointIdleElapsed { generation });
        assert!(sidebar.scene().split_shows_sidebar);
        assert!(sidebar
            .handle(BreakpointCrossed { window_width: 1000 })
            .is_some());
    }

    #[test]
    fn stale_breakpoint_callbacks_never_override_newer_state() {
        let mut sidebar = normal();
        let Some(SidebarTimer::BreakpointFrame { generation }) =
            sidebar.handle(BreakpointCrossed { window_width: 1000 })
        else {
            panic!("collapse");
        };
        sidebar.handle(ImmersiveEntered);
        let before = sidebar.scene();
        sidebar.handle(BreakpointFrameElapsed { generation });
        sidebar.handle(BreakpointIdleElapsed { generation });
        assert_eq!(sidebar.scene(), before);
    }

    #[test]
    fn repeated_events_give_identical_scenes() {
        let mut sidebar = compact();
        sidebar.handle(ImmersiveEntered);
        sidebar.handle(ImmersiveRevealRequested);
        let once = sidebar.scene();
        sidebar.handle(ImmersiveEntered);
        sidebar.handle(ImmersiveRevealRequested);
        assert_eq!(sidebar.scene(), once);
        sidebar.handle(ImmersiveLeft);
        let left = sidebar.scene();
        sidebar.handle(ImmersiveLeft);
        assert_eq!(sidebar.scene(), left);
    }

    #[test]
    fn widths_are_kept_and_clamped() {
        let mut sidebar = SidebarPresentation::new(SidebarMode::Compact, 10.0, 9000.0);
        assert_eq!(sidebar.scene().compact_width, COMPACT_RAIL_MIN_WIDTH);
        assert_eq!(sidebar.scene().drawer_width, SIDEBAR_MAX_WIDTH);
        sidebar.handle(CompactWidthChanged(150.0));
        sidebar.handle(ImmersiveEntered);
        sidebar.handle(ImmersiveLeft);
        assert_eq!(sidebar.scene().compact_width, 150.0);
    }

    #[test]
    fn destination_dismisses_only_a_collapsed_drawer() {
        let mut sidebar = normal();
        sidebar.handle(DrawerDismissed);
        assert!(sidebar.scene().split_shows_sidebar, "side-by-side stays");
        sidebar.handle(SplitShowChanged(false));
        sidebar.handle(BreakpointCrossed { window_width: 900 });
        sidebar.handle(EdgeHovered);
        assert!(
            sidebar.scene().split_shows_sidebar,
            "collapsed drawer peeks even when pinned"
        );
        sidebar.handle(DrawerDismissed);
        assert!(!sidebar.scene().split_shows_sidebar);
    }

    #[test]
    fn revealing_the_immersive_overlay_keeps_the_windowed_chrome() {
        let mut sidebar = normal();
        sidebar.handle(ImmersiveEntered);
        let hidden = sidebar.scene().chrome;
        sidebar.handle(ImmersiveRevealRequested);
        assert_eq!(sidebar.scene().chrome, hidden);
        assert!(hidden.hover_strip && hidden.show_button && !hidden.sidebar_hosts_title_buttons);
    }

    #[test]
    fn show_button_toggles_the_overlay_while_immersive() {
        let mut sidebar = normal();
        sidebar.handle(ImmersiveEntered);
        sidebar.handle(ShowPressed);
        assert!(sidebar.scene().overlay_revealed);
        sidebar.handle(ShowPressed);
        assert!(!sidebar.scene().overlay_revealed);
    }

    /// Cross the breakpoint narrow and wide again, running both follow-ups.
    fn narrow_and_widen(sidebar: &mut SidebarPresentation) {
        if let Some(SidebarTimer::BreakpointFrame { generation }) =
            sidebar.handle(BreakpointCrossed { window_width: 900 })
        {
            sidebar.handle(BreakpointFrameElapsed { generation });
        }
        let Some(SidebarTimer::BreakpointIdle { generation }) =
            sidebar.handle(BreakpointCrossed { window_width: 1600 })
        else {
            panic!("widening expands the split");
        };
        sidebar.handle(BreakpointIdleElapsed { generation });
    }

    fn assert_pinned_open(sidebar: &SidebarPresentation, path: &str) {
        let scene = sidebar.scene();
        assert!(scene.pinned, "{path}: the pin must survive");
        assert!(
            scene.split_shows_sidebar && !scene.hover_open,
            "{path}: the pinned sidebar must be open, not peeked"
        );
    }

    #[test]
    fn pin_survives_a_compact_start_across_the_breakpoint() {
        // The window reports width 0 before it is realized, then its real
        // width: the split collapses and expands while the rail is shown.
        let mut sidebar = compact();
        sidebar.handle(BreakpointCrossed { window_width: 0 });
        let Some(SidebarTimer::BreakpointIdle { generation }) =
            sidebar.handle(BreakpointCrossed { window_width: 1625 })
        else {
            panic!("expand");
        };
        sidebar.handle(BreakpointIdleElapsed { generation });
        assert!(sidebar.scene().pinned);
        sidebar.handle(ModeChosen(SidebarMode::Normal));
        assert_pinned_open(&sidebar, "compact start -> normal");
    }

    #[test]
    fn pin_survives_every_mode_and_immersive_round_trip() {
        let mut sidebar = normal();
        sidebar.handle(ModeChosen(SidebarMode::Compact));
        sidebar.handle(ModeChosen(SidebarMode::Normal));
        assert_pinned_open(&sidebar, "normal -> compact -> normal");

        let mut sidebar = normal();
        sidebar.handle(ImmersiveEntered);
        sidebar.handle(ImmersiveLeft);
        assert_pinned_open(&sidebar, "normal -> immersive -> normal");

        let mut sidebar = compact();
        sidebar.handle(ImmersiveEntered);
        sidebar.handle(ModeChosen(SidebarMode::Normal));
        sidebar.handle(ImmersiveLeft);
        assert_pinned_open(&sidebar, "compact -> immersive -> normal");

        let mut sidebar = compact();
        sidebar.handle(ImmersiveEntered);
        sidebar.handle(ImmersiveLeft);
        sidebar.handle(ModeChosen(SidebarMode::Normal));
        assert_pinned_open(&sidebar, "compact -> immersive -> compact -> normal");

        let mut sidebar = normal();
        narrow_and_widen(&mut sidebar);
        assert_pinned_open(&sidebar, "normal across the breakpoint");

        let mut sidebar = compact();
        narrow_and_widen(&mut sidebar);
        sidebar.handle(ModeChosen(SidebarMode::Normal));
        assert_pinned_open(&sidebar, "compact across the breakpoint -> normal");
    }

    #[test]
    fn an_unpinned_sidebar_stays_unpinned_across_the_breakpoint() {
        let mut sidebar = normal();
        sidebar.handle(PinPressed);
        narrow_and_widen(&mut sidebar);
        let scene = sidebar.scene();
        assert!(!scene.pinned && !scene.split_shows_sidebar);
        sidebar.handle(ModeChosen(SidebarMode::Compact));
        sidebar.handle(ModeChosen(SidebarMode::Normal));
        assert!(!sidebar.scene().pinned, "a mode switch does not re-pin");
    }
}
