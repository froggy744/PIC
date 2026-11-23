//! The Properties presentation module's one GTK adapter.
//!
//! [`PropertiesWidgets`] feeds events to the [`PropertiesPresentation`]
//! model, rebuilds the panels it names and applies its target. It is the
//! only writer of the inspector's visibility: the gallery split's Properties
//! sidebar, the viewer's Properties panel, and the InfoBar button's state.
//! The panel contents come from the builders in `photo_actions.rs`.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::glib;
use gtk4 as gtk;
use libadwaita as adw;

use super::properties_presentation::{PropertiesEvent, PropertiesPresentation, PropertiesSurface};
use super::{
    properties_panel_scroll_y, properties_photo_content, properties_placeholder,
    PhotoActionContext, PropertiesContent,
};
use crate::infobar::InfoBar;
use crate::photo_object::PhotoObject;

fn inspector_trace() -> bool {
    std::env::var_os("PIC_INSPECTOR_TRACE").is_some()
}

pub(super) struct PropertiesWidgets {
    this: Weak<Self>,
    presentation: RefCell<PropertiesPresentation>,
    info: Rc<InfoBar>,
    /// The window's photo actions, which the panel contents use.
    context: RefCell<Option<PhotoActionContext>>,
    gallery: RefCell<Option<(gtk::Box, adw::OverlaySplitView)>>,
    lightbox: RefCell<Option<gtk::Box>>,
    /// The photo each panel presents; `None` in the gallery shows the
    /// "Select a photo" placeholder.
    gallery_photo: RefCell<Option<PhotoObject>>,
    lightbox_photo: RefCell<Option<PhotoObject>>,
}

impl PropertiesWidgets {
    /// The panels and the photo actions are attached once the window has
    /// built them; until then only the InfoBar button is kept in step.
    pub(super) fn new(info: Rc<InfoBar>) -> Rc<Self> {
        let widgets = Rc::new_cyclic(|this| Self {
            this: this.clone(),
            presentation: RefCell::new(PropertiesPresentation::new()),
            info,
            context: RefCell::new(None),
            gallery: RefCell::new(None),
            lightbox: RefCell::new(None),
            gallery_photo: RefCell::new(None),
            lightbox_photo: RefCell::new(None),
        });
        widgets.apply();
        widgets
    }

    pub(super) fn set_context(&self, context: PhotoActionContext) {
        self.context.replace(Some(context));
    }

    pub(super) fn attach_gallery(&self, panel: &gtk::Box, split: &adw::OverlaySplitView) {
        self.gallery.replace(Some((panel.clone(), split.clone())));
        self.apply();
    }

    pub(super) fn attach_lightbox(&self, panel: &gtk::Box) {
        self.lightbox.replace(Some(panel.clone()));
        self.apply();
    }

    /// The InfoBar button and metadata right-click: opens or closes the
    /// panel of the surface on screen for `photo`.
    pub(super) fn toggle(&self, photo: &PhotoObject) {
        let (surface, allowed, was_open) = {
            let presentation = self.presentation.borrow();
            let surface = presentation.active_surface();
            (
                surface,
                surface != PropertiesSurface::Gallery || presentation.gallery_allowed(),
                presentation.is_open(surface),
            )
        };
        if !allowed {
            if inspector_trace() {
                eprintln!(
                    "PIC_INSPECTOR_TRACE stage=ignored reason=index_page photo={}",
                    photo.id()
                );
            }
            return;
        }
        if was_open && inspector_trace() {
            eprintln!("PIC_INSPECTOR_TRACE stage=close surface={surface:?}");
        }
        self.set_photo(surface, Some(photo));
        self.dispatch(PropertiesEvent::Toggle);
        if inspector_trace() {
            eprintln!(
                "PIC_INSPECTOR_TRACE stage=infobar_toggle surface={surface:?} action={} photo={}",
                if was_open { "close" } else { "open" },
                photo.id()
            );
        }
    }

    /// The photo menu's Properties item: shows `photo` in the panel of the
    /// surface on screen.
    pub(super) fn open(&self, photo: &PhotoObject) {
        let surface = self.presentation.borrow().active_surface();
        self.set_photo(surface, Some(photo));
        self.dispatch(PropertiesEvent::Open);
    }

    pub(super) fn close(&self, surface: PropertiesSurface) {
        if inspector_trace() {
            eprintln!("PIC_INSPECTOR_TRACE stage=close surface={surface:?}");
        }
        self.dispatch(PropertiesEvent::Close(surface));
    }

    /// The photo `surface` presents changed. An open panel follows it; in
    /// Immersive Mode it follows once the mode ends.
    pub(super) fn show_photo(&self, surface: PropertiesSurface, photo: Option<&PhotoObject>) {
        self.set_photo(surface, photo);
        let (open, allowed, suspended) = {
            let presentation = self.presentation.borrow();
            (
                presentation.is_open(surface),
                surface != PropertiesSurface::Gallery || presentation.gallery_allowed(),
                presentation.is_suspended(),
            )
        };
        if surface == PropertiesSurface::Gallery
            && open
            && allowed
            && std::env::var_os("PICASA_TRACE").is_some()
        {
            eprintln!(
                "PIC_INSPECTOR stage=refresh photo={} has_context=1",
                photo.map(|photo| photo.id()).unwrap_or_default()
            );
        }
        if open && allowed && suspended && inspector_trace() {
            eprintln!(
                "PIC_INSPECTOR_TRACE stage=deferred surface={surface:?} reason=immersive photo={}",
                photo.map(|photo| photo.id()).unwrap_or_default()
            );
        }
        self.dispatch(PropertiesEvent::PhotoChanged(surface));
    }

    /// Only the Photos page allows the gallery panel; leaving it closes it.
    pub(super) fn set_page_allowed(&self, allowed: bool) {
        if !allowed && inspector_trace() {
            eprintln!("PIC_INSPECTOR_TRACE stage=close surface=Gallery");
        }
        self.dispatch(PropertiesEvent::PageChanged {
            allows_gallery: allowed,
        });
        if inspector_trace() {
            eprintln!("PIC_INSPECTOR_TRACE stage=page_allowed allowed={allowed}");
        }
    }

    /// The viewer opened or closed; closing it closes only its own panel.
    pub(super) fn set_lightbox_shown(&self, shown: bool) {
        if !shown && inspector_trace() {
            eprintln!("PIC_INSPECTOR_TRACE stage=close surface=Lightbox");
        }
        self.dispatch(PropertiesEvent::LightboxShown(shown));
    }

    /// Immersive Mode suspends both panels for the session.
    pub(super) fn set_immersive(&self, active: bool) {
        self.dispatch(if active {
            PropertiesEvent::ImmersiveEntered
        } else {
            PropertiesEvent::ImmersiveLeft
        });
    }

    fn set_photo(&self, surface: PropertiesSurface, photo: Option<&PhotoObject>) {
        let slot = match surface {
            PropertiesSurface::Gallery => &self.gallery_photo,
            PropertiesSurface::Lightbox => &self.lightbox_photo,
        };
        slot.replace(photo.cloned());
    }

    fn dispatch(&self, event: PropertiesEvent) {
        let rebuild = self.presentation.borrow_mut().handle(event);
        // Contents first, then visibility: an opening panel slides in with
        // its photo already in place.
        for surface in &rebuild {
            self.rebuild(*surface);
        }
        self.apply();
        if std::env::var_os("PICASA_TRACE").is_some() {
            for surface in &rebuild {
                if let (Some(panel), Some(photo)) = (self.panel(*surface), self.photo(*surface)) {
                    eprintln!(
                        "PIC_INSPECTOR stage=show surface={surface:?} photo={} visible={} panel_width={}",
                        photo.id(),
                        panel.is_visible(),
                        panel.width()
                    );
                }
            }
        }
    }

    fn apply(&self) {
        let target = self.presentation.borrow().target();
        let split = self
            .gallery
            .borrow()
            .as_ref()
            .map(|(_, split)| split.clone());
        if let Some(split) = split {
            if split.shows_sidebar() != target.gallery_shown {
                split.set_show_sidebar(target.gallery_shown);
            }
        }
        let lightbox = self.lightbox.borrow().clone();
        if let Some(panel) = lightbox {
            if panel.is_visible() != target.lightbox_shown {
                panel.set_visible(target.lightbox_shown);
            }
        }
        self.info.set_properties_active(target.button_active);
        self.info.set_properties_enabled(target.button_enabled);
    }

    fn panel(&self, surface: PropertiesSurface) -> Option<gtk::Box> {
        match surface {
            PropertiesSurface::Gallery => self
                .gallery
                .borrow()
                .as_ref()
                .map(|(panel, _)| panel.clone()),
            PropertiesSurface::Lightbox => self.lightbox.borrow().clone(),
        }
    }

    fn photo(&self, surface: PropertiesSurface) -> Option<PhotoObject> {
        match surface {
            PropertiesSurface::Gallery => self.gallery_photo.borrow().clone(),
            PropertiesSurface::Lightbox => self.lightbox_photo.borrow().clone(),
        }
    }

    fn close_handler(&self, surface: PropertiesSurface) -> Rc<dyn Fn()> {
        let this = self.this.clone();
        Rc::new(move || {
            if let Some(widgets) = this.upgrade() {
                widgets.close(surface);
            }
        })
    }

    fn rebuild(&self, surface: PropertiesSurface) {
        let Some(panel) = self.panel(surface) else {
            return;
        };
        let context = self.context.borrow().clone();
        let Some(context) = context else {
            return;
        };
        match self.photo(surface) {
            Some(photo) => self.install_photo_content(surface, &panel, &photo, &context),
            // Changing folders briefly clears the grid selection: keep the
            // panel open without showing the previous photo's metadata.
            None if surface == PropertiesSurface::Gallery => self.install_placeholder(&panel),
            None => {}
        }
    }

    fn install_photo_content(
        &self,
        surface: PropertiesSurface,
        panel: &gtk::Box,
        photo: &PhotoObject,
        context: &PhotoActionContext,
    ) {
        let PropertiesContent {
            content,
            scroll,
            sections,
        } = properties_photo_content(photo, context, surface, self.close_handler(surface));
        let restore = self
            .presentation
            .borrow_mut()
            .begin_photo_content(surface, properties_panel_scroll_y(panel));
        panel.remove_css_class("properties-empty");
        let existing_scroll = panel
            .first_child()
            .and_then(|child| child.first_child())
            .and_then(|child| child.downcast::<gtk::ScrolledWindow>().ok());
        let active_scroll = if let Some(existing) = existing_scroll {
            scroll.set_child(None::<&gtk::Widget>);
            existing.set_child(Some(&sections));
            existing
        } else {
            while let Some(child) = panel.first_child() {
                panel.remove(&child);
            }
            panel.append(&content);
            scroll
        };
        // The adjustment's range is updated during allocation, after the new
        // contents are installed. Restore after layout instead of clamping to
        // the initial empty range.
        let adjustment = active_scroll.vadjustment();
        let lower = adjustment.lower();
        let upper = (adjustment.upper() - adjustment.page_size()).max(lower);
        if upper > lower {
            adjustment.set_value(restore.scroll_y.clamp(lower, upper));
        }
        let this = self.this.clone();
        let frames = Cell::new(0_u8);
        active_scroll.add_tick_callback(move |scroll, _| {
            let Some(widgets) = this.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !widgets
                .presentation
                .borrow()
                .restore_is_current(surface, restore.serial)
            {
                return glib::ControlFlow::Break;
            }
            let count = frames.get() + 1;
            frames.set(count);
            if count < 2 {
                return glib::ControlFlow::Continue;
            }
            let adjustment = scroll.vadjustment();
            let lower = adjustment.lower();
            let upper = (adjustment.upper() - adjustment.page_size()).max(lower);
            let restored = restore.scroll_y.clamp(lower, upper);
            adjustment.set_value(restored);
            widgets
                .presentation
                .borrow_mut()
                .finish_restore(surface, restore.serial, restored);
            glib::ControlFlow::Break
        });
        if inspector_trace() {
            eprintln!(
                "PIC_INSPECTOR_TRACE stage=photo_content surface={surface:?} photo={} panel_width={}",
                photo.id(),
                panel.width()
            );
        }
    }

    fn install_placeholder(&self, panel: &gtk::Box) {
        self.presentation
            .borrow_mut()
            .begin_placeholder(PropertiesSurface::Gallery, properties_panel_scroll_y(panel));
        let empty = properties_placeholder(self.close_handler(PropertiesSurface::Gallery));
        let scroll = panel
            .first_child()
            .and_then(|child| child.first_child())
            .and_then(|child| child.downcast::<gtk::ScrolledWindow>().ok());
        if let Some(scroll) = scroll {
            scroll.set_child(Some(&empty));
        } else {
            while let Some(child) = panel.first_child() {
                panel.remove(&child);
            }
            panel.append(&empty);
        }
        panel.add_css_class("properties-empty");
    }
}
