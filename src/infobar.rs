use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gtk::prelude::*;
use gtk4 as gtk;

use crate::photo_object::PhotoObject;

pub struct InfoBar {
    pub root: gtk::Box,
    preview: gtk::Image,
    filename: gtk::Label,
    details: gtk::Box,
    aperture_metric: gtk::Box,
    pub favorite: gtk::Button,
    pub rating: gtk::MenuButton,
    pub rating_buttons: Vec<gtk::Button>,
    pub edit: gtk::Button,
    pub collage: gtk::Button,
    pub view_menu: gtk::MenuButton,
    pub view_grid: gtk::CheckButton,
    pub view_wall: gtk::CheckButton,
    pub add_to_album: gtk::MenuButton,
    pub one_to_one: gtk::ToggleButton,
    pub rotate: gtk::Button,
    pub export: gtk::Button,
    pub more: gtk::Button,
    pub print: gtk::Button,
    pub grid_zoom: gtk::Scale,
    pub grid_zoom_reset: gtk::GestureClick,
    has_photo: Rc<Cell<bool>>,
    has_aperture: Rc<Cell<bool>>,
    collage_active: Rc<Cell<bool>>,
}

impl InfoBar {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        root.set_height_request(58);
        root.set_valign(gtk::Align::End);
        root.set_margin_top(0);
        root.set_margin_bottom(0);
        root.set_margin_start(16);
        root.set_margin_end(16);
        root.add_css_class("photo-info-bar");

        let preview = gtk::Image::new();
        preview.set_pixel_size(40);
        preview.set_size_request(40, 40);
        preview.set_width_request(40);
        preview.set_height_request(40);
        preview.set_hexpand(false);
        preview.set_vexpand(false);
        preview.set_halign(gtk::Align::Center);
        preview.set_valign(gtk::Align::Center);
        preview.set_overflow(gtk::Overflow::Hidden);
        preview.add_css_class("info-preview");

        root.append(&preview);

        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_valign(gtk::Align::Center);
        // Keep enough room for a useful filename, but allow the bottom bar to
        // fit restored/narrow windows without pushing action buttons offscreen.
        text.set_width_request(140);
        text.set_hexpand(false);

        // A small heading above the value keeps the file name grouped with the
        // other labelled metrics (Taken/Camera/Dimensions/Size) instead of
        // acting as a title, and frees the second line for the name itself.
        let heading = gtk::Label::new(Some("File name"));
        heading.set_xalign(0.0);
        heading.add_css_class("dim-label");
        heading.add_css_class("metric-key");
        text.append(&heading);

        let filename = gtk::Label::new(Some("No photo selected"));
        filename.set_xalign(0.0);
        filename.set_ellipsize(gtk::pango::EllipsizeMode::End);
        filename.add_css_class("info-title");
        text.append(&filename);

        root.append(&text);

        let details = gtk::Box::new(gtk::Orientation::Horizontal, 28);
        details.set_hexpand(true);
        details.set_valign(gtk::Align::Center);
        details.set_visible(false);

        let mut other_metrics = Vec::<gtk::Box>::new();
        let mut aperture_metric = None;
        for (label, value) in [
            ("Taken", "—"),
            ("Camera", "—"),
            ("Dimensions", "—"),
            ("Size", "—"),
            ("Aperture", "—"),
        ] {
            let metric = gtk::Box::new(gtk::Orientation::Vertical, 1);
            let key = gtk::Label::new(Some(label));
            key.set_xalign(0.0);
            key.add_css_class("dim-label");
            key.add_css_class("metric-key");

            let val = gtk::Label::new(Some(value));
            val.set_xalign(0.0);
            val.set_ellipsize(gtk::pango::EllipsizeMode::End);
            val.add_css_class("metric-val");

            metric.append(&key);
            metric.append(&val);
            details.append(&metric);
            if label == "Taken" {
                // Fixed-width date column: the filename and the other metrics
                // shrink first, so the date is never squeezed or ellipsized.
                metric.set_width_request(110);
            } else if label == "Aperture" {
                aperture_metric = Some(metric.clone());
            } else {
                other_metrics.push(metric.clone());
            }
        }
        root.append(&details);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        // Keep the action cluster anchored to the trailing edge when the
        // optional preview, filename, or metadata is hidden. Without an
        // expanding child, an empty/missing-thumbnail state makes the whole
        // bottom bar's contents drift to the left.
        actions.set_hexpand(true);
        actions.set_halign(gtk::Align::End);
        actions.set_valign(gtk::Align::Center);

        let favorite = gtk::Button::from_icon_name("emote-love-symbolic");
        configure_action_button(&favorite);
        favorite.add_css_class("favorite-btn");
        favorite.set_tooltip_text(Some("Add to Favourites"));

        let rating = gtk::MenuButton::new();
        rating.set_icon_name("non-starred-symbolic");
        rating.set_has_frame(false);
        configure_action_button(&rating);
        rating.add_css_class("rating-btn");
        rating.set_tooltip_text(Some("Rate photo"));
        rating.set_direction(gtk::ArrowType::Up);

        let rating_popover = gtk::Popover::new();
        rating_popover.set_has_arrow(true);
        rating_popover.set_position(gtk::PositionType::Top);
        let rating_box = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        rating_box.set_margin_top(6);
        rating_box.set_margin_bottom(6);
        rating_box.set_margin_start(6);
        rating_box.set_margin_end(6);

        let mut rating_buttons = Vec::with_capacity(5);
        for value in 1..=5 {
            let star = gtk::Button::from_icon_name("non-starred-symbolic");
            star.add_css_class("flat");
            star.set_size_request(30, 30);
            star.set_tooltip_text(Some(&format!("Rate {value} of 5")));
            let popover_for_click = rating_popover.clone();
            star.connect_clicked(move |_| popover_for_click.popdown());
            rating_box.append(&star);
            rating_buttons.push(star);
        }
        rating_popover.set_child(Some(&rating_box));
        rating.set_popover(Some(&rating_popover));

        let rating_hover = gtk::EventControllerMotion::new();
        let rating_for_hover = rating.clone();
        rating_hover.connect_enter(move |_, _, _| rating_for_hover.popup());
        rating.add_controller(rating_hover);

        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
        configure_action_button(&edit);
        edit.set_tooltip_text(Some("Open or close photo editor"));

        let collage = gtk::Button::with_label("Create Collage…");
        collage.add_css_class("flat");
        let view_menu = gtk::MenuButton::new();
        view_menu.set_icon_name("view-grid-symbolic");
        view_menu.set_tooltip_text(Some("View and Collage"));
        view_menu.set_has_frame(false);
        view_menu.set_direction(gtk::ArrowType::Up);
        configure_action_button(&view_menu);
        let view_popover = gtk::Popover::new();
        let view_choices = gtk::Box::new(gtk::Orientation::Vertical, 6);
        view_choices.set_margin_top(10);
        view_choices.set_margin_bottom(10);
        view_choices.set_margin_start(10);
        view_choices.set_margin_end(10);
        let view_grid = gtk::CheckButton::with_label("Grid");
        let view_wall = gtk::CheckButton::with_label("Photo Wall");
        view_wall.set_group(Some(&view_grid));
        view_grid.set_active(true);
        view_choices.append(&view_grid);
        view_choices.append(&view_wall);
        view_choices.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        view_choices.append(&collage);
        view_popover.set_child(Some(&view_choices));
        view_menu.set_popover(Some(&view_popover));
        for button in [&view_grid, &view_wall] {
            let popover = view_popover.clone();
            button.connect_toggled(move |button| {
                if button.is_active() {
                    popover.popdown();
                }
            });
        }
        let popover = view_popover.clone();
        collage.connect_clicked(move |_| popover.popdown());

        let add_to_album = gtk::MenuButton::new();
        add_to_album.set_icon_name("folder-new-symbolic");
        add_to_album.set_has_frame(false);
        configure_action_button(&add_to_album);
        add_to_album.set_tooltip_text(Some("Add to Album"));

        // Direct grid-size slider. Use GtkScale's native centre mark instead
        // of placing a Button over the trough: the overlay button intercepted
        // pointer motion and made dragging appear to stick at the midpoint.
        let grid_zoom = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 7.0, 1.0);
        grid_zoom.set_draw_value(false);
        // Grid has exactly eight canonical thumbnail sizes. Let the Scale
        // itself own that discrete contract instead of emitting fractional
        // values that are immediately rounded by the gallery and echoed back
        // into the same thumb during a drag.
        grid_zoom.set_round_digits(0);
        grid_zoom.set_width_request(128);
        grid_zoom.set_hexpand(false);
        grid_zoom.set_valign(gtk::Align::Center);
        // Symmetric marks keep the trough vertically centred beside the
        // infobar buttons; a bottom-only mark shifts GtkScale's layout upward.
        grid_zoom.add_mark(3.5, gtk::PositionType::Top, None);
        grid_zoom.add_mark(3.5, gtk::PositionType::Bottom, None);
        grid_zoom.set_tooltip_text(Some(
            "Thumbnail size (Ctrl + wheel); click the middle mark to reset",
        ));

        // Observe centre clicks without placing another widget on the trough.
        // The gesture never claims the sequence, so GtkScale owns dragging.
        let grid_zoom_reset = gtk::GestureClick::new();
        grid_zoom_reset.set_button(1);
        grid_zoom_reset.set_propagation_phase(gtk::PropagationPhase::Capture);
        grid_zoom.add_controller(grid_zoom_reset.clone());

        let one_to_one = gtk::ToggleButton::with_label("1:1");
        configure_action_button(&one_to_one);
        one_to_one.add_css_class("one-to-one-btn");
        one_to_one.set_tooltip_text(Some("Show at 100% (1:1)"));

        let rotate = gtk::Button::from_icon_name("object-rotate-right-symbolic");
        configure_action_button(&rotate);
        rotate.set_tooltip_text(Some("Rotate clockwise"));

        let export = gtk::Button::from_icon_name("document-save-symbolic");
        configure_action_button(&export);
        export.set_tooltip_text(Some("Export photo"));

        let more = gtk::Button::from_icon_name("emblem-system-symbolic");
        configure_action_button(&more);
        more.set_tooltip_text(Some("Settings"));

        let print = gtk::Button::from_icon_name("document-print-symbolic");
        configure_action_button(&print);
        print.set_tooltip_text(Some("Print photo"));

        actions.append(&grid_zoom);
        actions.append(&favorite);
        actions.append(&rating);
        actions.append(&edit);
        actions.append(&view_menu);
        actions.append(&add_to_album);
        actions.append(&one_to_one);
        actions.append(&rotate);
        actions.append(&export);
        actions.append(&more);
        actions.append(&print);
        root.append(&actions);

        // The action buttons are the controls that must always remain usable.
        // On narrow windows, reclaim space from optional metadata first, then
        // from filename/preview presentation. This prevents the bar's natural
        // minimum width from making the application appear clipped.
        let has_photo = Rc::new(Cell::new(false));
        let has_aperture = Rc::new(Cell::new(false));
        let collage_active = Rc::new(Cell::new(false));
        let has_photo_for_resize = has_photo.clone();
        let details_for_resize = details.clone();
        let text_for_resize = text.clone();
        let preview_for_resize = preview.clone();
        let other_metrics_for_resize = other_metrics;
        let camera_metric_for_resize = other_metrics_for_resize[0].clone();
        let dimensions_metric_for_resize = other_metrics_for_resize[1].clone();
        let size_metric_for_resize = other_metrics_for_resize[2].clone();
        let aperture_metric_for_resize = aperture_metric.clone().expect("aperture metric exists");
        let has_aperture_for_resize = has_aperture.clone();
        root.add_tick_callback(move |bar, _| {
            let width = bar.width();
            if width > 0 {
                // Reclaim horizontal space progressively as the window narrows.
                // Keep the filename and action controls as the final compact state.
                camera_metric_for_resize.set_visible(width >= 1280);
                dimensions_metric_for_resize.set_visible(width >= 1180);
                size_metric_for_resize.set_visible(width >= 1080);
                aperture_metric_for_resize
                    .set_visible(width >= 980 && has_aperture_for_resize.get());

                let show_taken = has_photo_for_resize.get() && width >= 880;
                details_for_resize.set_visible(show_taken);
                if let Some(taken_metric) = details_for_resize.first_child() {
                    taken_metric.set_visible(show_taken);
                }

                text_for_resize.set_visible(true);
                preview_for_resize.set_visible(width >= 880);
            }
            glib::ControlFlow::Continue
        });

        Self {
            root,
            preview,
            filename,
            details,
            aperture_metric: aperture_metric.expect("aperture metric exists"),
            favorite,
            rating,
            rating_buttons,
            edit,
            collage,
            view_menu,
            view_grid,
            view_wall,
            add_to_album,
            one_to_one,
            rotate,
            export,
            more,
            print,
            grid_zoom,
            grid_zoom_reset,
            has_photo,
            has_aperture,
            collage_active,
        }
    }

    pub fn connect_photo_layout(&self, changed: impl Fn(crate::grid::PhotoLayout) + 'static) {
        let changed = Rc::new(changed);
        let grid_changed = changed.clone();
        self.view_grid.connect_toggled(move |button| {
            if button.is_active() {
                grid_changed(crate::grid::PhotoLayout::Grid);
            }
        });
        self.view_wall.connect_toggled(move |button| {
            if button.is_active() {
                changed(crate::grid::PhotoLayout::PhotoWall);
            }
        });
    }

    pub fn set_collage_active(&self, active: bool) {
        self.collage_active.set(active);
        self.view_grid.set_sensitive(!active);
        self.view_wall.set_sensitive(!active);
        self.edit
            .set_sensitive(edit_button_sensitive(self.has_photo.get(), active));
        // Collage has no presentation zoom. Disable the shared zoom slider
        // instead of letting it fall through and resize the hidden Gallery.
        self.grid_zoom.set_sensitive(!active);
    }

    fn set_rating_presentation(&self, rating: i32) {
        let rating = rating.clamp(0, 5);
        self.rating.set_icon_name(if rating == 0 {
            "non-starred-symbolic"
        } else {
            "starred-symbolic"
        });
        let tooltip = if rating == 0 {
            "Rate photo".to_string()
        } else {
            format!("Rating: {rating} of 5")
        };
        self.rating.set_tooltip_text(Some(&tooltip));
        for (index, button) in self.rating_buttons.iter().enumerate() {
            button.set_icon_name(if (index as i32) < rating {
                "starred-symbolic"
            } else {
                "non-starred-symbolic"
            });
        }
    }

    pub fn set_photo(&self, photo: Option<&PhotoObject>) {
        let Some(photo) = photo else {
            self.has_photo.set(false);
            self.has_aperture.set(false);
            self.filename.set_text("No photo selected");
            self.preview.set_icon_name(Some("image-x-generic-symbolic"));
            self.details.set_visible(false);
            set_metric_values(&self.details, ["—", "—", "—", "—", "—"]);
            self.favorite.set_sensitive(false);
            self.rating.set_sensitive(false);
            self.set_rating_presentation(0);
            self.edit
                .set_sensitive(edit_button_sensitive(false, self.collage_active.get()));
            self.add_to_album.set_sensitive(false);
            self.rotate.set_sensitive(false);
            self.export.set_sensitive(false);
            self.more.set_sensitive(true);
            self.print.set_sensitive(false);
            self.favorite.remove_css_class("active");
            self.favorite.set_icon_name("emote-love-symbolic");
            return;
        };

        self.has_photo.set(true);
        self.filename.set_text(&photo.filename());

        let cached = photo.cached_thumbnail_path();
        let existing = cached.as_deref().filter(|path| Path::new(path).is_file());
        if let Some(thumb_path) = existing {
            if let Some(rotated) = crate::photo_texture::edited_thumbnail(
                thumb_path,
                photo.rotation(),
                &photo.edit_recipe(),
            ) {
                self.preview.set_paintable(Some(&rotated));
            } else {
                self.preview.set_from_file(Some(thumb_path));
            }
        } else {
            self.preview.set_icon_name(Some("image-x-generic-symbolic"));
        }

        self.details.set_visible(self.root.width() >= 880);
        let dimensions = if photo.width() > 0 && photo.height() > 0 {
            format!("{} × {}", photo.width(), photo.height())
        } else {
            "Unknown".to_string()
        };
        let size = format_size(photo.size_bytes());
        let camera = photo
            .camera()
            .unwrap_or_else(|| "Unknown camera".to_string());
        let raw_date = photo
            .taken_at()
            .unwrap_or_else(|| "Unknown date".to_string());
        let formatted_date = format_date(&raw_date);
        let aperture = photo.aperture();
        let has_aperture = aperture.is_finite() && aperture > 0.0;
        self.has_aperture.set(has_aperture);
        self.aperture_metric
            .set_visible(has_aperture && self.root.width() >= 980);

        // The camera stays a metric beside Taken; the filename subtitle slot
        // that previously duplicated it now holds the actual file name.
        set_metric_values(
            &self.details,
            [
                formatted_date,
                camera,
                dimensions,
                size,
                format_aperture(aperture).unwrap_or_default(),
            ],
        );

        self.favorite.set_sensitive(true);
        self.rating.set_sensitive(true);
        self.set_rating_presentation(photo.rating());
        self.edit
            .set_sensitive(edit_button_sensitive(true, self.collage_active.get()));
        self.add_to_album.set_sensitive(true);
        self.rotate.set_sensitive(true);
        self.export.set_sensitive(true);
        self.more.set_sensitive(true);
        self.print.set_sensitive(true);

        if photo.favorite() {
            self.favorite.set_icon_name("emote-love-symbolic");
            self.favorite.add_css_class("active");
            self.favorite
                .set_tooltip_text(Some("Remove from Favourites"));
        } else {
            self.favorite.set_icon_name("emote-love-symbolic");
            self.favorite.remove_css_class("active");
            self.favorite.set_tooltip_text(Some("Add to Favourites"));
        }
    }
}

fn edit_button_sensitive(has_photo: bool, collage_active: bool) -> bool {
    has_photo && !collage_active
}

fn format_aperture(value: f64) -> Option<String> {
    (value.is_finite() && value > 0.0).then(|| {
        if (value - value.round()).abs() < 0.05 {
            format!("f/{}", value.round() as i64)
        } else {
            format!("f/{value:.1}")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{edit_button_sensitive, format_aperture, format_date};

    #[test]
    fn edit_is_disabled_while_collage_is_active() {
        assert!(edit_button_sensitive(true, false));
        assert!(!edit_button_sensitive(true, true));
        assert!(!edit_button_sensitive(false, false));
        assert!(!edit_button_sensitive(false, true));
    }

    #[test]
    fn taken_date_renders_dd_mmm_yyyy_without_time() {
        // mtime fallback (RFC3339)
        assert_eq!(format_date("2026-09-17T14:46:00+02:00"), "17 Sep 2026");
        // canonical scanner format
        assert_eq!(format_date("2026-09-17 14:46:00"), "17 Sep 2026");
        // EXIF-derived format: missing this made camera photos fall back to
        // the raw timestamp in the bottom bar.
        assert_eq!(format_date("2026-09-17 14-46-00"), "17 Sep 2026");
        // date-only value
        assert_eq!(format_date("2026-09-17"), "17 Sep 2026");
        // unparseable values pass through untouched
        assert_eq!(format_date("not a date"), "not a date");
    }

    #[test]
    fn aperture_uses_photography_formatting() {
        assert_eq!(format_aperture(4.0).as_deref(), Some("f/4"));
        assert_eq!(format_aperture(5.6).as_deref(), Some("f/5.6"));
        assert_eq!(format_aperture(0.0), None);
    }
}

fn configure_action_button<W: IsA<gtk::Widget>>(button: &W) {
    let widget = button.upcast_ref::<gtk::Widget>();
    widget.set_size_request(34, 34);
    widget.set_width_request(34);
    widget.set_height_request(34);
    widget.set_hexpand(false);
    widget.set_vexpand(false);
    widget.set_halign(gtk::Align::Center);
    widget.set_valign(gtk::Align::Center);
    // MenuButton draws its own inner button. Giving its outer widget the
    // theme's action-button surface paints a second badge on hover.
    if !widget.is::<gtk::MenuButton>() {
        widget.add_css_class("flat");
        widget.add_css_class("photo-action-button");
    }
}

fn set_metric_values<const N: usize>(details: &gtk::Box, values: [impl AsRef<str>; N]) {
    let mut child = details.first_child();
    for value in values {
        let Some(metric) = child
            .as_ref()
            .and_then(|widget| widget.downcast_ref::<gtk::Box>())
        else {
            break;
        };
        if let Some(label) = metric.last_child().and_downcast::<gtk::Label>() {
            label.set_text(value.as_ref());
        }
        child = child.and_then(|widget| widget.next_sibling());
    }
}

pub(crate) fn format_size(size: i64) -> String {
    if size >= 1_000_000 {
        format!("{:.1} MB", size as f64 / 1_000_000.0)
    } else if size >= 1_000 {
        format!("{:.1} KB", size as f64 / 1_000.0)
    } else {
        format!("{size} B")
    }
}

pub(crate) fn format_date(value: &str) -> String {
    // Dates render as calendar dates only (dd Mmm yyyy). Every stored
    // taken_at shape must be accepted: RFC3339 (mtime fallback),
    // "YYYY-MM-DD HH:MM:SS", and the EXIF-derived "YYYY-MM-DD HH-MM-SS"
    // produced by the scanner's colon replacement. Missing the EXIF shape
    // made the raw timestamp (time included) show for camera photos.
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(value) {
        return parsed.format("%d %b %Y").to_string();
    }
    if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return parsed.format("%d %b %Y").to_string();
    }
    if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H-%M-%S") {
        return parsed.format("%d %b %Y").to_string();
    }
    if let Ok(parsed) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return parsed.format("%d %b %Y").to_string();
    }
    value.to_string()
}
