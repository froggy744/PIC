//! The sidebar presentation module's one GTK adapter.
//!
//! [`SidebarWidgets::dispatch`] feeds an event to the
//! [`SidebarPresentation`] state machine and applies the resulting scene to
//! the existing sidebar widgets. It is the only writer of the sidebar's
//! layout (compact dock, full sidebar column, resize handle, split view) and
//! the only code that moves the sidebar shell between the split and the
//! immersive overlay. Applying an unchanged scene writes nothing.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use gtk4 as gtk;
use libadwaita as adw;

use super::sidebar_presentation::max_sidebar_width;
use super::sidebar_presentation::{
    SidebarEvent, SidebarMode, SidebarPage, SidebarPlacement, SidebarPresentation, SidebarScene,
    SidebarTimer, SIDEBAR_MODE_SETTING_KEY,
};
use super::{numeric_setting, SIDEBAR_MIN_WIDTH, SIDEBAR_WIDTH_FRACTION_SETTING_KEY};
use crate::db;

/// Hooks the immersive overlay installs once its slide animation exists.
pub(super) struct ImmersiveHost {
    /// The gallery overlay the sidebar shell is presented on.
    pub(super) surface: gtk::Overlay,
    /// Slide the overlay in (`true`) or out; hides the shell when out.
    pub(super) set_shown: Rc<dyn Fn(bool)>,
    /// Stop the slide and put it fully off-screen, without animating.
    pub(super) reset_slide: Rc<dyn Fn()>,
}

/// The sidebar widgets, as `layout.rs` builds them.
pub(super) struct SidebarWidgetParts {
    pub(super) split: adw::OverlaySplitView,
    pub(super) shell: gtk::Overlay,
    pub(super) column: gtk::Box,
    pub(super) sidebar: gtk::ScrolledWindow,
    pub(super) handle: gtk::Box,
    pub(super) dock: gtk::Overlay,
    pub(super) compact_page: gtk::Box,
    pub(super) compact_immersive_toggle: gtk::Button,
}

/// Cells shared with code outside the adapter (drag handlers, the overlay's
/// child-position handler, and readers not yet moved onto the adapter).
pub(super) struct SidebarCells {
    /// Mirror of "the sidebar mode is normal".
    pub(super) expanded: Rc<Cell<bool>>,
    /// Mirror of "the immersive overlay is revealed".
    pub(super) immersive_shown: Rc<Cell<bool>>,
    /// The drawer width while the split is collapsed.
    pub(super) collapsed_width: Rc<Cell<f64>>,
    /// The compact rail width, written live by its resize drag.
    pub(super) compact_width: Rc<Cell<f64>>,
    /// The immersive overlay's slide width.
    pub(super) slide_width: Rc<Cell<i32>>,
    /// The full sidebar's width when immersive started.
    pub(super) normal_width: Rc<Cell<i32>>,
}

pub(super) struct SidebarWidgets {
    state: RefCell<SidebarPresentation>,
    last: RefCell<Option<SidebarScene>>,
    parts: SidebarWidgetParts,
    cells: SidebarCells,
    connection: Rc<RefCell<rusqlite::Connection>>,
    switch_to_normal_button: RefCell<Option<gtk::Button>>,
    hover_strip: RefCell<Option<gtk::Box>>,
    controls_sync: RefCell<Option<Rc<dyn Fn()>>>,
    immersive_host: RefCell<Option<ImmersiveHost>>,
    leave_held: RefCell<Option<Rc<dyn Fn() -> bool>>>,
    peek_hook: RefCell<Option<Rc<dyn Fn()>>>,
    pending: RefCell<std::collections::VecDeque<SidebarEvent>>,
    applying: Cell<bool>,
}

/// `PIC_SIDEBAR_TRACE=1` logs every sidebar event that changes the scene.
fn sidebar_trace_enabled() -> bool {
    thread_local! {
        static ENABLED: bool = std::env::var_os("PIC_SIDEBAR_TRACE").is_some();
    }
    ENABLED.with(|enabled| *enabled)
}

const LEAVE_CHECK_DELAY: std::time::Duration = std::time::Duration::from_millis(140);

impl SidebarWidgets {
    pub(super) fn new(
        parts: SidebarWidgetParts,
        cells: SidebarCells,
        mode: SidebarMode,
        connection: Rc<RefCell<rusqlite::Connection>>,
    ) -> Rc<Self> {
        let state =
            SidebarPresentation::new(mode, cells.compact_width.get(), cells.collapsed_width.get());
        cells.expanded.set(mode.is_normal());
        Rc::new(Self {
            state: RefCell::new(state),
            last: RefCell::new(None),
            parts,
            cells,
            connection,
            switch_to_normal_button: RefCell::new(None),
            hover_strip: RefCell::new(None),
            controls_sync: RefCell::new(None),
            immersive_host: RefCell::new(None),
            leave_held: RefCell::new(None),
            peek_hook: RefCell::new(None),
            pending: RefCell::new(std::collections::VecDeque::new()),
            applying: Cell::new(false),
        })
    }

    pub(super) fn mode(&self) -> SidebarMode {
        self.state.borrow().mode()
    }

    pub(super) fn is_immersive(&self) -> bool {
        self.state.borrow().is_immersive()
    }

    /// The immersive overlay's slide width, shared with the overlay's
    /// child-position handler.
    pub(super) fn slide_width_cell(&self) -> Rc<Cell<i32>> {
        self.cells.slide_width.clone()
    }

    pub(super) fn set_switch_to_normal_button(&self, button: gtk::Button) {
        self.switch_to_normal_button.replace(Some(button));
    }

    pub(super) fn set_hover_strip(&self, strip: gtk::Box) {
        self.hover_strip.replace(Some(strip));
    }

    pub(super) fn set_controls_sync(&self, sync: Rc<dyn Fn()>) {
        self.controls_sync.replace(Some(sync));
    }

    pub(super) fn attach_immersive_host(&self, host: ImmersiveHost) {
        self.immersive_host.replace(Some(host));
    }

    /// Feed one event, apply the new scene, and schedule any follow-up.
    ///
    /// Writing a widget can emit a notify whose handler dispatches again.
    /// Such nested events are queued and handled once the current scene has
    /// been applied, so a scene is never applied half-way.
    pub(super) fn dispatch(self: &Rc<Self>, event: SidebarEvent) {
        self.pending.borrow_mut().push_back(event);
        if self.applying.get() {
            return;
        }
        self.applying.set(true);
        loop {
            // Release the queue before handling: handling may queue more.
            let next = self.pending.borrow_mut().pop_front();
            let Some(event) = next else { break };
            self.handle_and_apply(event);
        }
        self.applying.set(false);
    }

    fn handle_and_apply(self: &Rc<Self>, event: SidebarEvent) {
        // The split's swipe gestures change it without an event. Take its
        // windowed state as the truth before deciding.
        if !self.is_immersive() {
            let split = &self.parts.split;
            self.state
                .borrow_mut()
                .observe_windowed(split.is_collapsed(), split.shows_sidebar());
        }
        let mode_before = self.mode();
        let timer = self.state.borrow_mut().handle(event);
        if sidebar_trace_enabled()
            && (timer.is_some() || Some(self.state.borrow().scene()) != *self.last.borrow())
        {
            eprintln!(
                "PIC_SIDEBAR event={event:?} timer={timer:?} scene={:?}",
                self.state.borrow().scene()
            );
        }
        if matches!(event, SidebarEvent::ModeChosen(_)) && self.mode() != mode_before {
            self.persist_mode();
        }
        self.apply();
        if let Some(timer) = timer {
            self.schedule(timer);
        }
    }

    /// Called just before a hover peek opens the split, so the gallery can
    /// freeze its layout before the split starts changing width.
    pub(super) fn set_peek_hook(&self, hook: Rc<dyn Fn()>) {
        self.peek_hook.replace(Some(hook));
    }

    /// Keep a collapsed drawer's width cap in step with the split's width.
    /// Called every frame; writes only when the cap moved.
    pub(super) fn sync_drawer_cap(&self) {
        let split = &self.parts.split;
        if split.is_collapsed() && self.mode().is_normal() && !self.is_immersive() {
            let cap = max_sidebar_width(true, self.cells.collapsed_width.get(), split.width());
            if (split.max_sidebar_width() - cap).abs() > 0.1 {
                split.set_max_sidebar_width(cap);
            }
        }
    }

    fn set_split_collapsed(&self, collapsed: bool) {
        let split = &self.parts.split;
        if split.is_collapsed() != collapsed {
            split.set_collapsed(collapsed);
        }
        // A collapsed split shows the sidebar as a drawer at its own width.
        split.set_max_sidebar_width(max_sidebar_width(
            collapsed,
            self.cells.collapsed_width.get(),
            split.width(),
        ));
    }

    fn persist_mode(&self) {
        if let Err(error) = db::set_setting(
            &self.connection.borrow(),
            SIDEBAR_MODE_SETTING_KEY,
            self.mode().setting_value(),
        ) {
            eprintln!("Could not save sidebar mode: {error}");
        }
    }

    /// Run a timed follow-up and feed it back with its generation. The
    /// state machine ignores it if anything newer happened meanwhile.
    fn schedule(self: &Rc<Self>, timer: SidebarTimer) {
        let weak = Rc::downgrade(self);
        match timer {
            SidebarTimer::LeaveCheck { generation } => {
                // Let a right-click create its popover before checking.
                glib::timeout_add_local_once(LEAVE_CHECK_DELAY, move || {
                    if let Some(widgets) = weak.upgrade() {
                        let held = widgets
                            .leave_held
                            .borrow()
                            .as_ref()
                            .is_some_and(|held| held());
                        widgets.dispatch(SidebarEvent::LeaveSettled { generation, held });
                    }
                });
            }
            SidebarTimer::BreakpointFrame { generation } => {
                // One real frame with the sidebar visible as an overlay, so
                // collapse and hide get the drawer animation instead of a cut.
                self.parts.split.add_tick_callback(move |_, _| {
                    if let Some(widgets) = weak.upgrade() {
                        widgets.dispatch(SidebarEvent::BreakpointFrameElapsed { generation });
                    }
                    glib::ControlFlow::Break
                });
            }
            SidebarTimer::BreakpointIdle { generation } => {
                glib::idle_add_local_once(move || {
                    if let Some(widgets) = weak.upgrade() {
                        let sidebar = &widgets.parts.sidebar;
                        let pinned = sidebar.is_visible() && crate::sidebar::is_pinned(sidebar);
                        widgets
                            .dispatch(SidebarEvent::BreakpointIdleElapsed { generation, pinned });
                    }
                });
            }
        }
    }

    /// While this returns true a hover peek stays open after the pointer
    /// left (a resize drag or a context menu is in progress).
    pub(super) fn set_leave_held(&self, held: Rc<dyn Fn() -> bool>) {
        self.leave_held.replace(Some(held));
    }

    /// Write the current scene, touching only what changed.
    fn apply(&self) {
        let scene = self.state.borrow().scene();
        let last = *self.last.borrow();
        if last == Some(scene) {
            return;
        }
        self.cells.expanded.set(self.mode().is_normal());
        self.cells.immersive_shown.set(scene.overlay_revealed);

        let entering = scene.placement == SidebarPlacement::ImmersiveOverlay
            && last.map_or(true, |last| last.placement == SidebarPlacement::Split);
        let leaving = scene.placement == SidebarPlacement::Split
            && last.is_some_and(|last| last.placement == SidebarPlacement::ImmersiveOverlay);

        if entering && last.is_some() {
            self.enter_immersive(&scene);
        } else if leaving {
            self.leave_immersive(&scene);
        } else if scene.placement == SidebarPlacement::ImmersiveOverlay {
            if last.map(|last| last.page) != Some(scene.page) {
                self.present_page(scene.page);
            }
            if last.map(|last| last.overlay_revealed) != Some(scene.overlay_revealed) {
                if let Some(host) = self.immersive_host.borrow().as_ref() {
                    (host.set_shown)(scene.overlay_revealed);
                }
            }
        } else {
            self.apply_windowed(&scene, last.as_ref());
        }

        if let Some(button) = self.switch_to_normal_button.borrow().as_ref() {
            button.set_visible(scene.layout.switch_to_normal_button);
        }
        self.last.replace(Some(scene));
        let sync = self.controls_sync.borrow().clone();
        if let Some(sync) = sync {
            sync();
        }
    }

    fn apply_windowed(&self, scene: &SidebarScene, last: Option<&SidebarScene>) {
        let parts = &self.parts;
        let changed = |pick: fn(&SidebarScene) -> bool| last.map(pick) != Some(pick(scene));
        if changed(|s| s.layout.compact_dock) {
            parts.dock.set_visible(scene.layout.compact_dock);
        }
        if changed(|s| s.layout.resize_handle) {
            parts.handle.set_visible(scene.layout.resize_handle);
        }
        if changed(|s| s.layout.full_sidebar) {
            parts.column.set_visible(scene.layout.full_sidebar);
            parts.sidebar.set_visible(scene.layout.full_sidebar);
        }
        // Mirror pin and hover-peek into sidebar.rs before the split moves:
        // show-sidebar notify handlers read them.
        if changed(|s| s.pinned) {
            crate::sidebar::set_pinned(&parts.sidebar, scene.pinned);
        }
        if changed(|s| s.hover_open) {
            if scene.hover_open {
                if let Some(hook) = self.peek_hook.borrow().clone() {
                    hook();
                }
            }
            crate::sidebar::set_hover_open(&parts.sidebar, scene.hover_open);
        }
        // Hide before collapsing or expanding, show after, so the drawer
        // always slides instead of being cut by the layout switch. The Adw
        // sidebar is completely absent in compact mode. Neither its natural
        // minimum nor its animation can enlarge the icon rail.
        if !scene.split_shows_sidebar && parts.split.shows_sidebar() {
            parts.split.set_show_sidebar(false);
        }
        if changed(|s| s.split_collapsed) || parts.split.is_collapsed() != scene.split_collapsed {
            self.set_split_collapsed(scene.split_collapsed);
        }
        if scene.split_shows_sidebar && !parts.split.shows_sidebar() {
            parts.split.set_show_sidebar(true);
        }
        if changed(|s| s.chrome.hover_strip) {
            if let Some(strip) = self.hover_strip.borrow().as_ref() {
                strip.set_visible(scene.chrome.hover_strip);
            }
        }
        if changed(|s| s.layout.split_gestures) {
            parts
                .split
                .set_enable_show_gesture(scene.layout.split_gestures);
            parts
                .split
                .set_enable_hide_gesture(scene.layout.split_gestures);
        }
        let became_normal = scene.layout.full_sidebar && changed(|s| s.layout.full_sidebar);
        if became_normal {
            self.restore_normal_widths();
        }
    }

    fn restore_normal_widths(&self) {
        let split = &self.parts.split;
        split.set_min_sidebar_width(SIDEBAR_MIN_WIDTH);
        split.set_max_sidebar_width(max_sidebar_width(
            split.is_collapsed(),
            self.cells.collapsed_width.get(),
            split.width(),
        ));
        let fraction = numeric_setting::<f64>(
            &self.connection.borrow(),
            SIDEBAR_WIDTH_FRACTION_SETTING_KEY,
        )
        .unwrap_or(0.22)
        .clamp(0.10, 0.70);
        split.set_sidebar_width_fraction(fraction);
    }

    /// Move the existing pages: the full column or the compact page goes
    /// into the shell. Never instantiates a second sidebar.
    fn present_page(&self, page: SidebarPage) {
        let SidebarWidgetParts {
            shell,
            dock,
            column,
            compact_page,
            ..
        } = &self.parts;
        if let Some(root) = shell.root() {
            if root.focus().is_some_and(|focus| focus.is_ancestor(shell)) {
                // The focused button is about to be removed. Prevent GTK from
                // focusing and scrolling an old gallery tile.
                gtk::prelude::RootExt::set_focus(&root, None::<&gtk::Widget>);
            }
        }
        let shell_widget = shell.clone().upcast::<gtk::Widget>();
        match page {
            SidebarPage::Full => {
                // The normal sidebar already paints its own theme background.
                shell.remove_css_class("navigation-sidebar");
                shell.remove_css_class("compact-sidebar-dock");
                if compact_page
                    .parent()
                    .is_some_and(|parent| parent == shell_widget)
                {
                    shell.set_child(None::<&gtk::Widget>);
                }
                if compact_page.parent().is_none() {
                    dock.set_child(Some(compact_page));
                }
                if column.parent().is_none() {
                    shell.set_child(Some(column));
                }
                self.cells
                    .slide_width
                    .set(self.cells.normal_width.get().max(1));
            }
            SidebarPage::Compact => {
                // In compact mode these classes belong to the dock. When the
                // page moves into the shell, the shell must supply the same
                // opaque, theme-aware background.
                shell.add_css_class("navigation-sidebar");
                shell.add_css_class("compact-sidebar-dock");
                if column.parent().is_some_and(|parent| parent == shell_widget) {
                    shell.set_child(None::<&gtk::Widget>);
                }
                if compact_page.parent().is_some() {
                    dock.set_child(None::<&gtk::Widget>);
                }
                shell.set_child(Some(compact_page));
                self.cells
                    .slide_width
                    .set(self.cells.compact_width.get().round() as i32);
            }
        }
        shell.set_width_request(self.cells.slide_width.get());
        // While immersive the shell's parent is the gallery overlay.
        // Reallocate only that presentation parent after changing width.
        if let Some(parent) = shell.parent() {
            parent.queue_allocate();
        }
    }

    fn enter_immersive(&self, scene: &SidebarScene) {
        let parts = &self.parts;
        let split = &parts.split;
        parts.dock.set_visible(scene.layout.compact_dock);
        parts.handle.set_visible(scene.layout.resize_handle);
        parts.column.set_visible(scene.layout.full_sidebar);
        parts.sidebar.set_visible(scene.layout.full_sidebar);
        split.set_show_sidebar(false);
        split.set_pin_sidebar(scene.split_pin);
        split.set_enable_show_gesture(scene.layout.split_gestures);
        split.set_enable_hide_gesture(scene.layout.split_gestures);
        self.set_split_collapsed(scene.split_collapsed);
        // A collapsed-notify handler may change the sidebar state.
        split.set_show_sidebar(false);
        // The existing shell leaves the split and becomes an overlay child of
        // the gallery. Its position is animated by the overlay's
        // child-position handler, which re-allocates only the sidebar, so the
        // gallery allocation, its scroll adjustment and every tile stay put.
        let overlay_width = split
            .sidebar()
            .map(|shell| shell.width())
            .filter(|width| *width > 0)
            .map(f64::from)
            .unwrap_or_else(|| split.max_sidebar_width());
        split.set_sidebar(None::<&gtk::Widget>);
        self.cells
            .normal_width
            .set(overlay_width.round().max(1.0) as i32);
        self.cells.slide_width.set(self.cells.normal_width.get());
        let shell = &parts.shell;
        shell.set_width_request(self.cells.slide_width.get());
        shell.set_hexpand(false);
        shell.set_halign(gtk::Align::Start);
        shell.set_valign(gtk::Align::Fill);
        shell.set_vexpand(true);
        // Every session starts fully hidden and off-screen.
        let host = self.immersive_host.borrow();
        if let Some(host) = host.as_ref() {
            (host.reset_slide)();
        }
        shell.set_visible(false);
        if let Some(strip) = self.hover_strip.borrow().as_ref() {
            strip.set_visible(scene.chrome.hover_strip);
        }
        if let Some(host) = host.as_ref() {
            // Added last so the overlay paints above the gallery chrome,
            // exactly like the split's own collapsed drawer did.
            host.surface.add_overlay(shell);
        }
        drop(host);
        self.present_page(scene.page);
        parts.compact_immersive_toggle.set_visible(true);
        if scene.overlay_revealed {
            if let Some(host) = self.immersive_host.borrow().as_ref() {
                (host.set_shown)(true);
            }
        }
    }

    fn leave_immersive(&self, scene: &SidebarScene) {
        let parts = &self.parts;
        let split = &parts.split;
        split.set_show_sidebar(false);
        // Return the overlay to the split before the windowed layout is
        // restored, and drop every presentation override so the drawer
        // allocates exactly as before.
        let host = self.immersive_host.borrow();
        if let Some(host) = host.as_ref() {
            (host.reset_slide)();
        }
        parts.compact_immersive_toggle.set_visible(false);
        self.present_page(SidebarPage::Full);
        parts.handle.set_visible(scene.layout.resize_handle);
        let shell = &parts.shell;
        if let Some(host) = host.as_ref() {
            host.surface.remove_overlay(shell);
        }
        drop(host);
        shell.set_width_request(-1);
        shell.set_visible(true);
        shell.set_hexpand(true);
        shell.set_halign(gtk::Align::Fill);
        shell.set_valign(gtk::Align::Fill);
        split.set_sidebar(Some(shell));
        crate::sidebar::clear_hover_open(&parts.sidebar);
        crate::sidebar::set_pinned(&parts.sidebar, scene.pinned);
        self.set_split_collapsed(scene.split_collapsed);
        split.set_pin_sidebar(scene.split_pin);
        split.set_enable_show_gesture(scene.layout.split_gestures);
        split.set_enable_hide_gesture(scene.layout.split_gestures);
        // Restore the exact windowed configuration: the compact rail in
        // compact mode, the full sidebar in normal mode.
        parts.column.set_visible(scene.layout.full_sidebar);
        parts.sidebar.set_visible(scene.layout.full_sidebar);
        parts.dock.set_visible(scene.layout.compact_dock);
        split.set_show_sidebar(scene.split_shows_sidebar);
        if let Some(strip) = self.hover_strip.borrow().as_ref() {
            strip.set_visible(
                super::sidebar_presentation::sidebar_chrome(
                    self.mode(),
                    false,
                    split.shows_sidebar(),
                )
                .hover_strip,
            );
        }
    }
}
