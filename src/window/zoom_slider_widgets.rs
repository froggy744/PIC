//! The GTK adapter for [`super::zoom_slider`]: the only writer of the shared
//! zoom slider's range, steps, rounding, tooltip and displayed value.
//!
//! It follows the front surface: when the mode changes (gallery, viewer,
//! editor) it sets the new mode up and shows that surface's current zoom.
//! The surfaces report later zoom changes through `show_*`, which apply only
//! in their own mode, so a late gallery zoom can never move the editor's
//! slider. Every write is wrapped in the `syncing` flag, which tells the
//! slider's value handler that the change did not come from the user.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk4 as gtk;

use super::front_surface::FrontSurface;
use super::zoom_slider::{self, SliderMode};

/// Where the adapter reads each surface's current zoom when its mode starts.
pub(super) struct ZoomSources {
    /// The gallery's thumbnail width.
    pub(super) thumbnail_width: Box<dyn Fn() -> i32>,
    /// The viewer's zoom and fit scale.
    pub(super) photo_zoom: Box<dyn Fn() -> (f64, f64)>,
    /// The editor's zoom and fit scale, while an editor is open.
    pub(super) edit_zoom: Box<dyn Fn() -> Option<(f64, f64)>>,
}

pub(super) struct ZoomSliderWidgets {
    scale: gtk::Scale,
    syncing: Rc<Cell<bool>>,
    mode: Cell<SliderMode>,
    sources: RefCell<Option<ZoomSources>>,
}

impl ZoomSliderWidgets {
    /// The slider starts in Thumbnails mode, showing `thumbnail_width`. Its
    /// range and tooltip are the InfoBar's construction defaults.
    pub(super) fn new(scale: &gtk::Scale, syncing: Rc<Cell<bool>>, thumbnail_width: i32) -> Rc<Self> {
        let slider = Rc::new(Self {
            scale: scale.clone(),
            syncing,
            mode: Cell::new(SliderMode::Thumbnails),
            sources: RefCell::new(None),
        });
        slider.write_value(zoom_slider::thumbnail_value(thumbnail_width));
        slider
    }

    /// Follow the front surface, reading zooms from `sources`.
    pub(super) fn connect(self: &Rc<Self>, front: &FrontSurface, sources: ZoomSources) {
        self.sources.replace(Some(sources));
        let slider = Rc::downgrade(self);
        front.connect_changed(move |change| {
            if let Some(slider) = slider.upgrade() {
                slider.set_mode(zoom_slider::mode_for(change.after.front()));
            }
        });
    }

    /// The gallery's thumbnail width changed (zoom finished, library switch).
    pub(super) fn show_thumbnail_width(&self, width: i32) {
        if self.mode.get() == SliderMode::Thumbnails {
            self.write_value(zoom_slider::thumbnail_value(width));
        }
    }

    /// The viewer's zoom changed (Ctrl+wheel, 1:1).
    pub(super) fn show_photo_zoom(&self, scale: f64, fit_scale: f64) {
        if self.mode.get() == SliderMode::Photo {
            self.write_value(zoom_slider::photo_value(scale, fit_scale));
        }
    }

    /// The editor's zoom changed.
    pub(super) fn show_edit_zoom(&self, scale: f64, fit_scale: f64) {
        if self.mode.get() == SliderMode::Edit {
            self.write_value(zoom_slider::photo_value(scale, fit_scale));
        }
    }

    fn set_mode(&self, mode: SliderMode) {
        if mode == self.mode.get() {
            return;
        }
        self.mode.set(mode);
        let value = {
            let sources = self.sources.borrow();
            let Some(sources) = sources.as_ref() else {
                return;
            };
            match mode {
                SliderMode::Thumbnails => zoom_slider::thumbnail_value((sources.thumbnail_width)()),
                SliderMode::Photo => {
                    let (scale, fit) = (sources.photo_zoom)();
                    zoom_slider::photo_value(scale, fit)
                }
                SliderMode::Edit => (sources.edit_zoom)()
                    .map_or(0.0, |(scale, fit)| zoom_slider::photo_value(scale, fit)),
            }
        };
        let spec = zoom_slider::spec(mode);
        self.syncing.set(true);
        self.scale.set_range(spec.lower, spec.upper);
        self.scale.set_increments(spec.step, spec.page);
        self.scale.set_round_digits(spec.round_digits);
        self.scale.clear_marks();
        self.scale.set_value(value);
        self.scale.set_tooltip_text(Some(spec.tooltip));
        self.syncing.set(false);
    }

    fn write_value(&self, value: f64) {
        self.syncing.set(true);
        self.scale.set_value(value);
        self.syncing.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lightbox::Lightbox;

    #[test]
    #[ignore = "requires a GTK display; run individually"]
    fn the_slider_follows_the_front_and_ignores_other_surfaces_zoom() {
        gtk::init().unwrap();
        let stack = gtk::Stack::new();
        for name in ["photos", "edit", "collage"] {
            stack.add_named(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some(name));
        }
        stack.set_visible_child_name("photos");
        let lightbox = Lightbox::new();
        lightbox.root.set_visible(false);
        let front = FrontSurface::new(&stack, &lightbox, Rc::new(Cell::new(false)), Rc::new(RefCell::new(None)));
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 7.0, 1.0);
        let syncing = Rc::new(Cell::new(false));
        // Every write the adapter makes must be marked as not from the user.
        let unsynced_writes = Rc::new(Cell::new(0));
        {
            let syncing = syncing.clone();
            let unsynced_writes = unsynced_writes.clone();
            scale.connect_value_changed(move |_| {
                if !syncing.get() {
                    unsynced_writes.set(unsynced_writes.get() + 1);
                }
            });
        }
        let slider = ZoomSliderWidgets::new(&scale, syncing.clone(), 160);
        assert_eq!(scale.value(), 3.0, "starts on the saved thumbnail size");
        slider.connect(&front, ZoomSources {
            thumbnail_width: Box::new(|| 187),
            photo_zoom: Box::new(|| (1.0, 0.5)),
            edit_zoom: Box::new(|| Some((1.5, 0.5))),
        });
        let tooltip = || scale.tooltip_text().map(|text| text.to_string()).unwrap_or_default();
        let range = || (scale.adjustment().lower(), scale.adjustment().upper());

        lightbox.root.set_visible(true);
        assert_eq!((range(), scale.value()), ((0.0, 100.0), 50.0), "viewer at 100%");
        assert!(tooltip().starts_with("Photo zoom"));
        slider.show_thumbnail_width(100);
        assert_eq!(scale.value(), 50.0, "a gallery zoom does not move the viewer's slider");
        slider.show_photo_zoom(2.0, 0.5);
        assert_eq!(scale.value(), 100.0);

        lightbox.root.set_visible(false);
        assert_eq!((range(), scale.value()), ((0.0, 7.0), 4.0), "back on thumbnails (187)");
        assert_eq!(tooltip(), "Thumbnail size (Ctrl + wheel)");
        slider.show_photo_zoom(1.0, 0.5);
        assert_eq!(scale.value(), 4.0, "a viewer zoom does not move the gallery's slider");

        stack.set_visible_child_name("edit");
        assert_eq!((range(), scale.value()), ((0.0, 100.0), 75.0), "editor at 150%");
        assert!(tooltip().starts_with("Edit zoom"));
        slider.show_thumbnail_width(300);
        assert_eq!(scale.value(), 75.0, "a late gallery zoom does not move the editor's slider");
        slider.show_edit_zoom(1.0, 0.5);
        assert_eq!(scale.value(), 50.0);

        stack.set_visible_child_name("collage");
        assert_eq!(range(), (0.0, 7.0), "Collage takes the thumbnail range at once");
        slider.show_thumbnail_width(100);
        assert_eq!(scale.value(), 0.0);

        assert_eq!(unsynced_writes.get(), 0, "every adapter write is marked syncing");
        assert!(!syncing.get());
    }
}
