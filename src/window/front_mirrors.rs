//! Front mirrors: the one place that keeps the InfoBar, the Properties panel
//! and the metadata panel in step with the front surface.
//!
//! Each of them keeps the front inputs its own presentation model needs
//! (the InfoBar: viewer, date overview and page; Properties: viewer and
//! whether the page may show it; the metadata panel: viewer, page and the
//! sidebar destination). They
//! used to be fed by 8 separate viewer, page and date-browser handlers; now
//! a single [`FrontSurface::connect_changed`] listener feeds them, in the
//! order those handlers ran.

use std::cell::Cell;
use std::rc::{Rc, Weak};

use gtk4::glib;

use super::front_surface::{DateBrowser, FrontChange, FrontState, FrontSurface, Page};
use super::metadata_panel::MetadataPanel;
use super::navigation_filter::NavigationFilter;
use super::properties_widgets::PropertiesWidgets;
use crate::infobar::presentation::InfoBarPage;
use crate::infobar::InfoBar;
use crate::grid;

pub(super) struct FrontMirrors {
    pub(super) info: Rc<InfoBar>,
    pub(super) properties: Rc<PropertiesWidgets>,
    pub(super) metadata_panel: Rc<MetadataPanel>,
    pub(super) gallery: Weak<grid::Gallery>,
    pub(super) filter: Rc<NavigationFilter>,
    /// How many photos the InfoBar's selection strip samples.
    pub(super) selection_preview_limit: usize,
}

impl FrontMirrors {
    /// Bring the mirrors that start from the window's state up to date, then
    /// follow every change.
    pub(super) fn connect(self, front: &Rc<FrontSurface>) {
        let now = front.state();
        self.properties
            .set_page_allowed(super::page_allows_gallery_properties(now.page.stack_name()));
        self.metadata_panel.set_lightbox_open(now.lightbox_shown);
        self.metadata_panel
            .set_navigation_view(now.page.stack_name().unwrap_or(""), self.filter.get());
        self.follow_filter(front);
        front.connect_changed(move |change| self.apply(change));
    }

    /// Several sidebar destinations share the Photos page, so the metadata
    /// panel also follows the destination itself. A navigation may set the
    /// filter before or after it switches the page; one update per burst,
    /// before the next layout and paint, sees both (this replaced a tick
    /// callback that polled every frame, candidate 24).
    fn follow_filter(&self, front: &Rc<FrontSurface>) {
        let pending = Rc::new(Cell::new(false));
        let (panel, filter, front) = (
            Rc::downgrade(&self.metadata_panel),
            Rc::downgrade(&self.filter),
            Rc::downgrade(front),
        );
        self.filter.connect_changed(move |_| {
            if pending.replace(true) {
                return;
            }
            let (pending, panel, filter, front) =
                (pending.clone(), panel.clone(), filter.clone(), front.clone());
            glib::idle_add_local_full(glib::Priority::HIGH_IDLE, move || {
                pending.set(false);
                if let (Some(panel), Some(filter), Some(front)) =
                    (panel.upgrade(), filter.upgrade(), front.upgrade())
                {
                    let page = front.state().page;
                    panel.set_navigation_view(page.stack_name().unwrap_or(""), filter.get());
                }
                glib::ControlFlow::Break
            });
        });
    }

    fn apply(&self, change: &FrontChange) {
        let after = change.after;
        if change.viewer_changed() {
            self.properties.set_lightbox_shown(after.lightbox_shown);
            if !after.lightbox_shown {
                // The selection may have changed while the viewer was open.
                self.refresh_selection();
            }
            self.info.set_lightbox_shown(after.lightbox_shown);
            self.metadata_panel.set_lightbox_open(after.lightbox_shown);
        }
        if change.page_changed() {
            self.page_changed(after);
        }
        if change.date_overview_changed() {
            self.info
                .set_date_overview_shown(after.date_browser == DateBrowser::Overview);
        }
    }

    fn page_changed(&self, after: FrontState) {
        let name = after.page.stack_name();
        self.properties
            .set_page_allowed(super::page_allows_gallery_properties(name));
        if std::env::var_os("PIC_INSPECTOR_TRACE").is_some() {
            eprintln!("PIC_INSPECTOR_TRACE stage=page_switch page={name:?}");
        }
        let page = match after.page {
            Page::Photos => InfoBarPage::Photos,
            Page::Collage => InfoBarPage::Collage,
            _ => InfoBarPage::Other,
        };
        if page == InfoBarPage::Photos {
            self.refresh_selection();
        }
        self.info.set_page(page);
        self.metadata_panel
            .set_navigation_view(name.unwrap_or(""), self.filter.get());
    }

    fn refresh_selection(&self) {
        if let Some(gallery) = self.gallery.upgrade() {
            let (count, photos) = gallery.selected_photo_preview(self.selection_preview_limit);
            self.info.show_gallery_selection(count, &photos);
        }
    }
}
