//! Properties presentation: every decision about when the Properties
//! inspector is shown, in one place and free of GTK.
//!
//! The inspector has two panels: the gallery panel docked in its split view,
//! and the viewer panel overlaid on the lightbox. Each remembers whether the
//! user opened it and where its contents were scrolled. Three things can
//! hide an open panel without closing it, and closing is different from
//! hiding:
//!
//! - Only the Photos page allows the gallery panel. Leaving it closes the
//!   gallery panel, so it stays closed on return.
//! - Closing the viewer closes the viewer panel and nothing else.
//! - Immersive Mode *suspends* both panels: they are hidden for the session
//!   and come back afterwards. While suspended no panel content is rebuilt;
//!   a panel whose photo changed meanwhile is rebuilt once on resume.
//!
//! [`PropertiesPresentation::handle`] turns an event into the panels to
//! rebuild, and [`PropertiesPresentation::target`] says what the widgets
//! must show. The GTK adapter in `properties_widgets.rs` applies both.

/// The two Properties panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PropertiesSurface {
    /// Docked beside the photo gallery.
    Gallery,
    /// Overlaid on the lightbox viewer.
    Lightbox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PropertiesEvent {
    /// The InfoBar button or a metadata right-click: open or close the
    /// panel of the surface on screen.
    Toggle,
    /// The photo menu's Properties item: open the panel of the surface on
    /// screen, or refresh it when it is already open.
    Open,
    /// A panel's own close button.
    Close(PropertiesSurface),
    /// The visible main page changed; `true` only for the Photos page.
    PageChanged {
        allows_gallery: bool,
    },
    /// The viewer opened (`true`) or closed (`false`).
    LightboxShown(bool),
    ImmersiveEntered,
    ImmersiveLeft,
    /// The photo a surface presents changed: a new selection, viewer
    /// navigation, or an edit to the photo's metadata.
    PhotoChanged(PropertiesSurface),
}

/// What the inspector widgets show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PropertiesTarget {
    /// The gallery split shows its Properties sidebar.
    pub(super) gallery_shown: bool,
    /// The viewer's Properties panel is visible.
    pub(super) lightbox_shown: bool,
    /// The InfoBar button is highlighted: the panel of the surface on screen
    /// is shown.
    pub(super) button_active: bool,
    /// The InfoBar button and the metadata right-click respond.
    pub(super) button_enabled: bool,
}

/// Where a rebuilt panel scrolls to once its new contents are laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ScrollRestore {
    /// Identifies this rebuild; a later rebuild makes it stale.
    pub(super) serial: u64,
    pub(super) scroll_y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Panel {
    /// The user opened the panel and has not closed it.
    open: bool,
    /// The panel's photo changed while it could not be rebuilt.
    stale: bool,
    /// The panel shows the "Select a photo" placeholder.
    placeholder: bool,
    /// The scroll position the next rebuild restores.
    scroll_y: f64,
    /// Bumped by every rebuild, so an older pending restore gives up.
    generation: u64,
    /// A rebuild's scroll restore has not landed yet; the live scroll
    /// position is not trustworthy until it does.
    restore_pending: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct PropertiesPresentation {
    gallery: Panel,
    lightbox: Panel,
    gallery_allowed: bool,
    lightbox_visible: bool,
    suspended: bool,
}

impl PropertiesPresentation {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// The surface on screen, whose panel the InfoBar controls.
    pub(super) fn active_surface(&self) -> PropertiesSurface {
        if self.lightbox_visible {
            PropertiesSurface::Lightbox
        } else {
            PropertiesSurface::Gallery
        }
    }

    pub(super) fn is_open(&self, surface: PropertiesSurface) -> bool {
        self.panel(surface).open
    }

    pub(super) fn gallery_allowed(&self) -> bool {
        self.gallery_allowed
    }

    pub(super) fn target(&self) -> PropertiesTarget {
        let gallery_shown = self.shown(PropertiesSurface::Gallery);
        let lightbox_shown = self.shown(PropertiesSurface::Lightbox);
        PropertiesTarget {
            gallery_shown,
            lightbox_shown,
            button_active: if self.lightbox_visible {
                lightbox_shown
            } else {
                gallery_shown
            },
            button_enabled: !self.suspended,
        }
    }

    /// Applies `event` and returns the panels whose contents must be
    /// rebuilt for their current photo, gallery first.
    pub(super) fn handle(&mut self, event: PropertiesEvent) -> Vec<PropertiesSurface> {
        match event {
            PropertiesEvent::Toggle => {
                let surface = self.active_surface();
                if !self.can_open(surface) {
                    return Vec::new();
                }
                if self.panel(surface).open {
                    self.close(surface);
                    Vec::new()
                } else {
                    self.open(surface)
                }
            }
            PropertiesEvent::Open => {
                let surface = self.active_surface();
                if self.can_open(surface) {
                    self.open(surface)
                } else {
                    Vec::new()
                }
            }
            PropertiesEvent::Close(surface) => {
                self.close(surface);
                Vec::new()
            }
            PropertiesEvent::PageChanged { allows_gallery } => {
                self.gallery_allowed = allows_gallery;
                if !allows_gallery {
                    self.close(PropertiesSurface::Gallery);
                }
                Vec::new()
            }
            PropertiesEvent::LightboxShown(visible) => {
                self.lightbox_visible = visible;
                if !visible {
                    self.close(PropertiesSurface::Lightbox);
                }
                Vec::new()
            }
            PropertiesEvent::ImmersiveEntered => {
                self.suspended = true;
                Vec::new()
            }
            PropertiesEvent::ImmersiveLeft => {
                self.suspended = false;
                [PropertiesSurface::Gallery, PropertiesSurface::Lightbox]
                    .into_iter()
                    .filter(|surface| {
                        let panel = self.panel_mut(*surface);
                        std::mem::take(&mut panel.stale) && panel.open
                    })
                    .collect()
            }
            PropertiesEvent::PhotoChanged(surface) => {
                if surface == PropertiesSurface::Gallery && !self.gallery_allowed {
                    return Vec::new();
                }
                if !self.panel(surface).open {
                    return Vec::new();
                }
                if self.suspended {
                    self.panel_mut(surface).stale = true;
                    Vec::new()
                } else {
                    vec![surface]
                }
            }
        }
    }

    /// Immersive Mode suspends both panels: nothing is shown or rebuilt.
    pub(super) fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// Starts rebuilding `surface` with a photo. `live_scroll_y` is the
    /// panel's current scroll position, if it has contents. Returns where
    /// the new contents must scroll to.
    pub(super) fn begin_photo_content(
        &mut self,
        surface: PropertiesSurface,
        live_scroll_y: Option<f64>,
    ) -> ScrollRestore {
        let panel = self.panel_mut(surface);
        let pending = std::mem::replace(&mut panel.restore_pending, true);
        panel.generation = panel.generation.wrapping_add(1);
        // Mid-restore the live position is still the old range's clamp, and
        // the placeholder's position belongs to no photo.
        if !pending && !panel.placeholder {
            if let Some(live) = live_scroll_y {
                panel.scroll_y = live;
            }
        }
        panel.placeholder = false;
        ScrollRestore {
            serial: panel.generation,
            scroll_y: panel.scroll_y,
        }
    }

    /// Starts showing the "Select a photo" placeholder in `surface`,
    /// remembering the photo contents' scroll position for the next photo.
    pub(super) fn begin_placeholder(
        &mut self,
        surface: PropertiesSurface,
        live_scroll_y: Option<f64>,
    ) {
        let panel = self.panel_mut(surface);
        let pending = std::mem::replace(&mut panel.restore_pending, false);
        panel.generation = panel.generation.wrapping_add(1);
        if !pending && !panel.placeholder {
            if let Some(live) = live_scroll_y {
                panel.scroll_y = live;
            }
        }
        panel.placeholder = true;
    }

    /// Whether the restore started as `serial` is still the latest one.
    pub(super) fn restore_is_current(&self, surface: PropertiesSurface, serial: u64) -> bool {
        self.panel(surface).generation == serial
    }

    /// The restore started as `serial` landed at `scroll_y`.
    pub(super) fn finish_restore(
        &mut self,
        surface: PropertiesSurface,
        serial: u64,
        scroll_y: f64,
    ) {
        let panel = self.panel_mut(surface);
        if panel.generation != serial {
            return;
        }
        panel.scroll_y = scroll_y;
        panel.restore_pending = false;
    }

    fn shown(&self, surface: PropertiesSurface) -> bool {
        let reachable = match surface {
            PropertiesSurface::Gallery => self.gallery_allowed,
            PropertiesSurface::Lightbox => self.lightbox_visible,
        };
        self.panel(surface).open && reachable && !self.suspended
    }

    fn can_open(&self, surface: PropertiesSurface) -> bool {
        !self.suspended && (surface != PropertiesSurface::Gallery || self.gallery_allowed)
    }

    fn open(&mut self, surface: PropertiesSurface) -> Vec<PropertiesSurface> {
        let panel = self.panel_mut(surface);
        panel.open = true;
        panel.stale = false;
        vec![surface]
    }

    fn close(&mut self, surface: PropertiesSurface) {
        let panel = self.panel_mut(surface);
        panel.open = false;
        panel.stale = false;
    }

    fn panel(&self, surface: PropertiesSurface) -> &Panel {
        match surface {
            PropertiesSurface::Gallery => &self.gallery,
            PropertiesSurface::Lightbox => &self.lightbox,
        }
    }

    fn panel_mut(&mut self, surface: PropertiesSurface) -> &mut Panel {
        match surface {
            PropertiesSurface::Gallery => &mut self.gallery,
            PropertiesSurface::Lightbox => &mut self.lightbox,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PropertiesEvent::*;
    use super::PropertiesSurface::{Gallery, Lightbox};
    use super::*;

    fn on_photos_page() -> PropertiesPresentation {
        let mut model = PropertiesPresentation::new();
        model.handle(PageChanged {
            allows_gallery: true,
        });
        model
    }

    fn shown(model: &PropertiesPresentation) -> (bool, bool) {
        let target = model.target();
        (target.gallery_shown, target.lightbox_shown)
    }

    #[test]
    fn toggle_opens_and_closes_the_gallery_panel() {
        let mut model = on_photos_page();
        assert_eq!(model.handle(Toggle), vec![Gallery]);
        assert_eq!(
            model.target(),
            PropertiesTarget {
                gallery_shown: true,
                lightbox_shown: false,
                button_active: true,
                button_enabled: true,
            }
        );
        assert_eq!(model.handle(Toggle), vec![]);
        assert_eq!(shown(&model), (false, false));
        assert!(!model.target().button_active);
    }

    #[test]
    fn open_refreshes_an_open_panel_and_close_closes_it() {
        let mut model = on_photos_page();
        assert_eq!(model.handle(Open), vec![Gallery]);
        assert_eq!(
            model.handle(Open),
            vec![Gallery],
            "Open never toggles closed"
        );
        assert!(model.target().gallery_shown);
        assert_eq!(model.handle(Close(Gallery)), vec![]);
        assert!(!model.is_open(Gallery));
        assert_eq!(
            model.handle(PhotoChanged(Gallery)),
            vec![],
            "a closed panel is not rebuilt"
        );
    }

    #[test]
    fn index_pages_refuse_and_close_the_gallery_panel() {
        let mut model = PropertiesPresentation::new();
        assert_eq!(
            model.handle(Toggle),
            vec![],
            "no page allows it before the first page change"
        );
        model.handle(PageChanged {
            allows_gallery: true,
        });
        model.handle(Toggle);
        model.handle(PageChanged {
            allows_gallery: false,
        });
        assert!(
            !model.is_open(Gallery),
            "leaving Photos closes the gallery panel"
        );
        assert_eq!(model.handle(Toggle), vec![]);
        assert_eq!(model.handle(Open), vec![]);
        assert_eq!(model.handle(PhotoChanged(Gallery)), vec![]);
        model.handle(PageChanged {
            allows_gallery: true,
        });
        assert_eq!(
            shown(&model),
            (false, false),
            "returning to Photos keeps it closed"
        );
    }

    #[test]
    fn the_viewer_panel_is_independent_and_forgotten_when_the_viewer_closes() {
        let mut model = on_photos_page();
        model.handle(Toggle);
        model.handle(LightboxShown(true));
        assert_eq!(model.active_surface(), Lightbox);
        assert!(
            !model.target().button_active,
            "the button follows the viewer panel"
        );
        assert_eq!(model.handle(Toggle), vec![Lightbox]);
        assert_eq!(shown(&model), (true, true));
        assert!(model.target().button_active);
        assert_eq!(model.handle(PhotoChanged(Lightbox)), vec![Lightbox]);

        model.handle(LightboxShown(false));
        assert_eq!(model.active_surface(), Gallery);
        assert_eq!(shown(&model), (true, false));
        assert!(
            model.target().button_active,
            "the button follows the gallery panel again"
        );
        model.handle(LightboxShown(true));
        assert!(
            !model.is_open(Lightbox),
            "the viewer panel starts closed in a new viewer"
        );
        assert_eq!(model.handle(PhotoChanged(Lightbox)), vec![]);
    }

    #[test]
    fn the_viewer_panel_opens_on_index_pages() {
        let mut model = PropertiesPresentation::new();
        model.handle(LightboxShown(true));
        assert_eq!(model.handle(Toggle), vec![Lightbox]);
        assert!(model.target().lightbox_shown);
    }

    #[test]
    fn immersive_mode_suspends_and_resumes_both_panels() {
        let mut model = on_photos_page();
        model.handle(Toggle);
        model.handle(LightboxShown(true));
        model.handle(Toggle);
        assert_eq!(model.handle(ImmersiveEntered), vec![]);
        assert_eq!(
            model.target(),
            PropertiesTarget {
                gallery_shown: false,
                lightbox_shown: false,
                button_active: false,
                button_enabled: false,
            }
        );
        assert!(
            model.is_open(Gallery) && model.is_open(Lightbox),
            "suspending is not closing"
        );
        assert_eq!(
            model.handle(ImmersiveLeft),
            vec![],
            "nothing changed, nothing to rebuild"
        );
        assert_eq!(shown(&model), (true, true));
        assert!(model.target().button_active && model.target().button_enabled);
    }

    #[test]
    fn suspended_photo_changes_rebuild_once_on_resume() {
        let mut model = on_photos_page();
        model.handle(Toggle);
        model.handle(ImmersiveEntered);
        for _ in 0..3 {
            assert_eq!(
                model.handle(PhotoChanged(Gallery)),
                vec![],
                "no rebuild while suspended"
            );
            assert_eq!(shown(&model), (false, false));
        }
        assert_eq!(model.handle(ImmersiveLeft), vec![Gallery]);
        assert_eq!(model.handle(ImmersiveEntered), vec![]);
        assert_eq!(
            model.handle(ImmersiveLeft),
            vec![],
            "the stale mark was consumed"
        );
    }

    #[test]
    fn requests_while_suspended_are_ignored() {
        let mut model = on_photos_page();
        model.handle(ImmersiveEntered);
        assert_eq!(model.handle(Toggle), vec![]);
        assert_eq!(model.handle(Open), vec![]);
        assert_eq!(model.handle(ImmersiveLeft), vec![]);
        assert_eq!(shown(&model), (false, false), "a closed panel stays closed");
    }

    #[test]
    fn closing_while_suspended_is_not_undone_by_resume() {
        let mut model = on_photos_page();
        model.handle(Toggle);
        model.handle(LightboxShown(true));
        model.handle(Toggle);
        model.handle(ImmersiveEntered);
        model.handle(PhotoChanged(Gallery));
        model.handle(PhotoChanged(Lightbox));
        model.handle(PageChanged {
            allows_gallery: false,
        });
        model.handle(LightboxShown(false));
        model.handle(PageChanged {
            allows_gallery: true,
        });
        assert_eq!(model.handle(ImmersiveLeft), vec![]);
        assert_eq!(shown(&model), (false, false));
    }

    #[test]
    fn resume_rebuilds_only_the_stale_open_panels() {
        let mut model = on_photos_page();
        model.handle(Toggle);
        model.handle(LightboxShown(true));
        model.handle(Toggle);
        model.handle(ImmersiveEntered);
        model.handle(PhotoChanged(Lightbox));
        assert_eq!(model.handle(ImmersiveLeft), vec![Lightbox]);
        model.handle(ImmersiveEntered);
        model.handle(PhotoChanged(Lightbox));
        model.handle(PhotoChanged(Gallery));
        assert_eq!(model.handle(ImmersiveLeft), vec![Gallery, Lightbox]);
    }

    #[test]
    fn rebuilds_keep_the_live_scroll_position() {
        let mut model = on_photos_page();
        let first = model.begin_photo_content(Gallery, None);
        assert_eq!(first.scroll_y, 0.0);
        model.finish_restore(Gallery, first.serial, 0.0);
        let second = model.begin_photo_content(Gallery, Some(140.0));
        assert_eq!(
            second.scroll_y, 140.0,
            "the next photo opens where the user scrolled to"
        );
        assert!(!model.restore_is_current(Gallery, first.serial));
        assert!(model.restore_is_current(Gallery, second.serial));
    }

    #[test]
    fn a_rebuild_during_a_pending_restore_keeps_the_remembered_position() {
        let mut model = on_photos_page();
        let first = model.begin_photo_content(Gallery, None);
        model.finish_restore(Gallery, first.serial, 220.0);
        let second = model.begin_photo_content(Gallery, Some(220.0));
        // The new contents are not laid out yet, so GTK clamps the live
        // position to the short initial range.
        let third = model.begin_photo_content(Gallery, Some(12.0));
        assert_eq!(third.scroll_y, 220.0);
        model.finish_restore(Gallery, second.serial, 12.0);
        assert!(
            model.restore_is_current(Gallery, third.serial),
            "a stale restore changes nothing"
        );
        model.finish_restore(Gallery, third.serial, 220.0);
        assert_eq!(
            model.begin_photo_content(Gallery, Some(220.0)).scroll_y,
            220.0
        );
    }

    #[test]
    fn the_placeholder_keeps_the_photo_scroll_position() {
        let mut model = on_photos_page();
        let first = model.begin_photo_content(Gallery, None);
        model.finish_restore(Gallery, first.serial, 0.0);
        model.begin_placeholder(Gallery, Some(180.0));
        assert!(!model.restore_is_current(Gallery, first.serial));
        // The placeholder does not scroll; its position is not a photo's.
        model.begin_placeholder(Gallery, Some(0.0));
        assert_eq!(
            model.begin_photo_content(Gallery, Some(0.0)).scroll_y,
            180.0
        );
    }

    #[test]
    fn the_two_panels_keep_separate_scroll_positions() {
        let mut model = on_photos_page();
        let gallery = model.begin_photo_content(Gallery, None);
        model.finish_restore(Gallery, gallery.serial, 300.0);
        let viewer = model.begin_photo_content(Lightbox, None);
        model.finish_restore(Lightbox, viewer.serial, 40.0);
        assert_eq!(
            model.begin_photo_content(Gallery, Some(300.0)).scroll_y,
            300.0
        );
        assert_eq!(
            model.begin_photo_content(Lightbox, Some(40.0)).scroll_y,
            40.0
        );
    }
}
