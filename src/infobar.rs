use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

use gtk::prelude::*;
use gtk4 as gtk;

use crate::photo_object::PhotoObject;

pub struct InfoBar {
    pub root: gtk::Box,
    utilities: gtk::Box,
    preview: gtk::Image,
    metadata: gtk::Box,
    metadata_stack: gtk::Stack,
    selection_count: gtk::Label,
    selection_controls: gtk::Box,
    selection_thumbnails: gtk::Box,
    selection_rendered_ids: RefCell<Vec<i64>>,
    selection_remove: Rc<RefCell<Option<Rc<dyn Fn(i64)>>>>,
    pub selection_clear: gtk::Button,
    filename: gtk::Label,
    details: gtk::Box,
    pub favorite: gtk::Button,
    pub rating: gtk::MenuButton,
    pub rating_buttons: Vec<gtk::Button>,
    pub edit: gtk::Button,
    pub collage: gtk::Button,
    pub view_toggle: gtk::Button,
    photo_layout: Rc<Cell<crate::grid::PhotoLayout>>,
    clear_saved_views: gio::SimpleAction,
    pub add_to_album: gtk::MenuButton,
    pub one_to_one: gtk::ToggleButton,
    pub rotate: gtk::Button,
    pub export: gtk::Button,
    pub import_photos: gtk::Button,
    pub more: gtk::Button,
    pub print: gtk::Button,
    overflow_popover: gtk::Popover,
    pub grid_zoom: gtk::Scale,
    pub grid_zoom_reset: gtk::GestureClick,
    has_photo: Rc<Cell<bool>>,
    collage_active: Rc<Cell<bool>>,
}

impl InfoBar {
    /// Keep the screensaver independent of photo selection.
    /// Open the existing photo properties inspector by right-clicking the
    /// metadata region. Left clicks and action buttons remain unaffected.
    pub fn connect_metadata_right_click(&self, open: impl Fn() + 'static) {
        let gesture = gtk::GestureClick::new();
        gesture.set_button(3);
        gesture.connect_pressed(move |gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            open();
        });
        // Bind to the actual metadata content, not the expandable blank
        // viewport space between metadata and photo action buttons.
        self.metadata.add_controller(gesture);
    }

    pub fn append_utility(&self, button: &gtk::Button) {
        configure_action_button(button);
        self.utilities.append(button);
        self.utilities.set_visible(true);
    }

    pub fn append_menu_action(&self, button: &gtk::Button, title: &str) {
        configure_action_button(button);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("photo-actions-menu-row");
        row.set_height_request(34);
        row.append(button);
        let label = gtk::Label::new(Some(title));
        label.set_xalign(0.0);
        row.append(&label);
        let menu = self
            .overflow_popover
            .child()
            .and_downcast::<gtk::Box>()
            .unwrap();
        menu.append(&row);
        let row_for_visibility = row.downgrade();
        button.connect_visible_notify(move |button| {
            if let Some(row) = row_for_visibility.upgrade() {
                row.set_visible(button.get_visible());
            }
        });
        let popover = self.overflow_popover.downgrade();
        button.connect_clicked(move |_| {
            if let Some(popover) = popover.upgrade() {
                popover.popdown();
            }
        });
    }

    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        root.set_height_request(58);
        root.set_valign(gtk::Align::End);
        root.set_margin_top(0);
        root.set_margin_bottom(0);
        // The themed background spans the full width; CSS insets the contents.
        root.set_margin_start(0);
        root.set_margin_end(0);
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

        // A shrinkable viewport keeps optional metadata from imposing a
        // window minimum. Its allocated width is the exact metadata budget.
        let metadata = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        let metadata_viewport = gtk::ScrolledWindow::new();
        metadata_viewport.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
        metadata_viewport.set_propagate_natural_width(false);
        // The photo/selection content must yield all spare width to the
        // fixed controls and actions, even with dozens of thumbnails.
        metadata_viewport.set_min_content_width(1);
        metadata_viewport.set_size_request(1, -1);
        metadata_viewport.set_hexpand(true);
        let metadata_stack = gtk::Stack::new();
        metadata_stack.set_hexpand(true);
        metadata_stack.set_hhomogeneous(false);
        metadata_stack.add_named(&metadata, Some("photo"));

        // A multi-selection replaces the metadata region, while the existing
        // action toolbar stays in place. Keep a bounded strip of cached thumbs.
        let selection_panel = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        selection_panel.set_hexpand(true);
        selection_panel.set_valign(gtk::Align::Center);
        selection_panel.add_css_class("infobar-selection-mode");
        // The thumbnail strip owns all flexible space. Counter and Clear must
        // be siblings of the outer metadata viewport, never its scrollable
        // children: otherwise long selections can push them out of view.
        let selection_thumbnails = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        selection_thumbnails.set_valign(gtk::Align::Center);
        selection_thumbnails.set_halign(gtk::Align::Start);
        let selection_scroll = gtk::ScrolledWindow::new();
        selection_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
        selection_scroll.set_propagate_natural_width(false);
        selection_scroll.set_hexpand(true);
        selection_scroll.set_min_content_width(1);
        selection_scroll.set_size_request(1, -1);
        selection_scroll.set_child(Some(&selection_thumbnails));
        selection_panel.append(&selection_scroll);
        metadata_stack.add_named(&selection_panel, Some("selection"));
        metadata_stack.set_visible_child_name("photo");
        metadata_viewport.set_child(Some(&metadata_stack));
        root.append(&metadata_viewport);

        // A non-scrolling, fixed-size trailing group remains visible even if
        // the thumbnail strip contains more items than fit the window.
        let selection_controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        selection_controls.set_hexpand(false);
        selection_controls.set_valign(gtk::Align::Center);
        selection_controls.set_visible(false);
        let selection_count = gtk::Label::new(None);
        selection_count.add_css_class("infobar-selection-count");
        selection_count.set_xalign(1.0);
        selection_count.set_single_line_mode(true);
        selection_count.set_hexpand(false);
        selection_controls.append(&selection_count);

        let selection_clear = gtk::Button::from_icon_name("edit-clear-all-symbolic");
        selection_clear.set_tooltip_text(Some("Clear selected photos"));
        selection_clear.add_css_class("flat");
        selection_controls.append(&selection_clear);
        root.append(&selection_controls);
        let selection_remove: Rc<RefCell<Option<Rc<dyn Fn(i64)>>>> =
            Rc::new(RefCell::new(None));
        metadata.append(&preview);

        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_valign(gtk::Align::Center);
        // Keep enough room for a useful filename, but allow the bottom bar to
        // fit restored/narrow windows without pushing action buttons offscreen.
        text.set_width_request(100);
        // Do not consume spare metadata width: the next visible metric should
        // remain directly beside the filename instead of drifting right.
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
        filename.set_max_width_chars(32);
        filename.add_css_class("info-title");
        text.append(&filename);

        metadata.append(&text);

        let details = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        details.set_hexpand(false);
        details.set_valign(gtk::Align::Center);
        details.set_visible(false);

        let mut metrics = Vec::<gtk::Box>::new();
        for (label, value) in [
            ("Taken", "—"),
            ("Camera", "—"),
            ("Dimensions", "—"),
            ("Size", "—"),
            ("Aperture", ""),
            ("ISO", ""),
            ("Shutter Speed", ""),
            ("Focal Length", ""),
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
            if label == "Camera" {
                val.set_max_width_chars(20);
            }

            metric.append(&key);
            metric.append(&val);
            details.append(&metric);
            metrics.push(metric);
        }
        metadata.append(&details);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_hexpand(false);
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
        rating_popover.set_autohide(true);
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

        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
        configure_action_button(&edit);
        edit.set_tooltip_text(Some("Open or close photo editor"));

        let collage = gtk::Button::from_icon_name("view-grid-symbolic");
        configure_action_button(&collage);
        collage.set_tooltip_text(Some("Start a new blank collage"));
        let photo_layout = Rc::new(Cell::new(crate::grid::PhotoLayout::Grid));
        let view_toggle = gtk::Button::from_icon_name("collage-smart-mosaic-symbolic");
        view_toggle.set_tooltip_text(Some("Switch to Photo Wall"));
        let layout_for_toggle = photo_layout.clone();
        view_toggle.connect_clicked(move |button| {
            use crate::grid::PhotoLayout;
            let layout = match layout_for_toggle.get() {
                PhotoLayout::Grid => PhotoLayout::PhotoWall,
                PhotoLayout::PhotoWall => PhotoLayout::Masonry,
                PhotoLayout::Masonry => PhotoLayout::Grid,
            };
            layout_for_toggle.set(layout);
            update_photo_layout_button(button, layout);
        });

        let clear_saved_views = gio::SimpleAction::new("clear-saved-views", None);
        let right_click = gtk::GestureClick::new();
        right_click.set_button(3);
        let clear_for_right_click = clear_saved_views.clone();
        right_click.connect_pressed(move |gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            clear_for_right_click.activate(None);
        });
        view_toggle.add_controller(right_click);

        let add_to_album = gtk::MenuButton::new();
        add_to_album.set_icon_name("folder-new-symbolic");
        add_to_album.set_has_frame(false);
        configure_action_button(&add_to_album);
        add_to_album.set_tooltip_text(Some("Add to Album"));

        // Direct grid-size slider; keep the trough free of overlay widgets
        // so pointer motion and dragging stay with GtkScale.
        let grid_zoom = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 7.0, 1.0);
        grid_zoom.add_css_class("infobar-zoom");
        grid_zoom.set_draw_value(false);
        // Grid has exactly eight canonical thumbnail sizes. Let the Scale
        // itself own that discrete contract instead of emitting fractional
        // values that are immediately rounded by the gallery and echoed back
        // into the same thumb during a drag.
        grid_zoom.set_round_digits(0);
        grid_zoom.set_width_request(64);
        grid_zoom.set_hexpand(false);
        grid_zoom.set_valign(gtk::Align::Center);
        grid_zoom.set_tooltip_text(Some(
            "Thumbnail size (Ctrl + wheel); click the middle to reset",
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
        rotate.set_tooltip_text(Some(
            "Rotate clockwise (right-click to rotate counter-clockwise)",
        ));

        let export = gtk::Button::from_icon_name("document-save-symbolic");
        configure_action_button(&export);
        export.set_tooltip_text(Some("Export photo"));

        let import_photos = gtk::Button::from_icon_name("media-flash-symbolic");
        configure_action_button(&import_photos);
        import_photos.set_tooltip_text(Some("Import photos from camera or SD card"));

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
        actions.append(&collage);
        actions.append(&add_to_album);
        actions.append(&one_to_one);
        actions.append(&rotate);
        actions.append(&export);
        actions.append(&import_photos);
        actions.append(&more);
        actions.append(&print);
        // Move the same controls into a labelled drawer on compact windows.
        // Reusing them preserves their signal handlers, sensitivity and menus.
        let overflow = gtk::MenuButton::new();
        overflow.set_icon_name("view-more-symbolic");
        overflow.set_tooltip_text(Some("More photo actions"));
        overflow.set_direction(gtk::ArrowType::Up);
        overflow.set_has_frame(false);
        configure_action_button(&overflow);
        overflow.add_css_class("photo-actions-overflow");
        let overflow_popover = gtk::Popover::new();
        overflow_popover.set_autohide(true);
        // Rate opens a child popover from inside this drawer. Closing that
        // child (after choosing a star or clicking away) must also close the
        // drawer so the next photo click is not trapped behind it.
        overflow_popover.set_cascade_popdown(true);
        let overflow_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        overflow_box.set_margin_top(6);
        overflow_box.set_margin_bottom(6);
        overflow_box.set_margin_start(11);
        overflow_box.set_margin_end(11);
        overflow_box.add_css_class("photo-actions-menu");
        let show_only_menu = Rc::new(Cell::new(false));
        let hide_buttons_option = gtk::CheckButton::with_label("Menu only");
        let show_only_menu_for_toggle = show_only_menu.clone();
        hide_buttons_option.connect_toggled(move |check| {
            show_only_menu_for_toggle.set(check.is_active());
        });
        overflow_box.append(&hide_buttons_option);
        let overflow_entries: Vec<(gtk::Widget, gtk::Box)> = [
            (grid_zoom.clone().upcast(), "Size"),
            (favorite.clone().upcast(), "Favourites"),
            (rating.clone().upcast(), "Rate"),
            (edit.clone().upcast(), "Edit"),
            (collage.clone().upcast(), "Collage"),
            (add_to_album.clone().upcast(), "Add to Album"),
            (one_to_one.clone().upcast(), "100%"),
            (rotate.clone().upcast(), "Rotate"),
            (export.clone().upcast(), "Export"),
            (import_photos.clone().upcast(), "Import"),
            (more.clone().upcast(), "Settings"),
            (print.clone().upcast(), "Print"),
        ]
        .into_iter()
        .map(|(control, title): (gtk::Widget, &str)| {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.set_height_request(34);
            row.add_css_class("photo-actions-menu-row");
            let label = gtk::Label::new(Some(title));
            label.set_xalign(0.0);
            row.append(&label);
            if title != "Rotate" && title != "Favourites" {
                if let Some(button) = control.downcast_ref::<gtk::Button>() {
                    let popover = overflow_popover.downgrade();
                    button.connect_clicked(move |_| {
                        if let Some(popover) = popover.upgrade() {
                            popover.popdown();
                        }
                    });
                }
            }
            (control, row)
        })
        .collect();

        let mut group_separators = Vec::with_capacity(4);
        let mut append_separator = |box_: &gtk::Box| {
            let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
            separator.set_margin_start(4);
            separator.set_margin_end(4);
            box_.append(&separator);
            group_separators.push(separator);
        };
        // Keep each row attached to its existing control while ordering the
        // popup into compact, easy-to-scan groups.
        overflow_box.append(&overflow_entries[0].1); // Size
        append_separator(&overflow_box);
        for index in [1, 2, 3] {
            // Favourite, Rate, Edit
            overflow_box.append(&overflow_entries[index].1);
        }
        append_separator(&overflow_box);
        for index in [5, 4] {
            // Add to Album, Collage
            overflow_box.append(&overflow_entries[index].1);
        }
        append_separator(&overflow_box);
        for index in [6, 7, 8] {
            // 100%, Rotate, Export
            overflow_box.append(&overflow_entries[index].1);
        }
        append_separator(&overflow_box);
        for index in [9, 11, 10] {
            // Import, Print, Settings
            overflow_box.append(&overflow_entries[index].1);
        }
        drop(append_separator);
        overflow_popover.set_child(Some(&overflow_box));
        overflow_popover.set_size_request(190, -1);
        overflow.set_popover(Some(&overflow_popover));
        // Keep the menu reachable even when every action fits in the row; it
        // also contains the option to collapse the toolbar for metadata.
        overflow.set_visible(true);
        actions.append(&overflow);
        let utilities = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        utilities.set_valign(gtk::Align::Center);
        utilities.set_visible(false);
        actions.insert_child_after(&utilities, Some(&print));
        let utility_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        utility_row.add_css_class("photo-actions-menu-row");
        utility_row.append(&gtk::Label::new(Some("Screensaver")));
        utility_row.set_visible(false);
        overflow_box.append(&utility_row);
        let utilities_for_resize = utilities.clone();
        root.append(&actions);
        let zoom_separator = toolbar_separator();
        actions.insert_child_after(&zoom_separator, Some(&grid_zoom));
        // The action buttons are the controls that must always remain usable.
        // On narrow windows, reclaim space from optional metadata first, then
        // from filename/preview presentation. This prevents the bar's natural
        // minimum width from making the application appear clipped.
        let has_photo = Rc::new(Cell::new(false));
        let collage_active = Rc::new(Cell::new(false));
        let has_photo_for_resize = has_photo.clone();
        let details_for_resize = details.clone();
        let text_for_resize = text.clone();
        let preview_for_resize = preview.clone();
        // Reveal secondary actions progressively as the bar gains room. The
        // old all-or-nothing 900px switch hid every secondary action on a
        // window only slightly narrower than that threshold, even when the
        // wide middle of the bar was visibly empty.
        let visible_action_count = Cell::new((usize::MAX, false));
        let show_only_menu_for_resize = show_only_menu.clone();
        let hide_buttons_option_for_resize = hide_buttons_option.clone();
        let grid_zoom_for_resize = grid_zoom.clone();
        let group_separators_for_resize = group_separators;
        root.add_tick_callback(move |bar, _| {
            // Keep the open drawer stable while interacting with its controls.
            // Reparenting during popup allocation can dismiss nested actions.
            let hide_all = show_only_menu_for_resize.get();
            let mode_changed = visible_action_count.get().1 != hide_all;
            for (control, row) in &overflow_entries {
                row.set_visible(
                    control.get_visible() && control.parent().as_ref() == Some(row.upcast_ref()),
                );
            }
            zoom_separator.set_visible(!hide_all && grid_zoom_for_resize.get_visible());
            let screensaver_visible = utilities_for_resize
                .first_child()
                .is_some_and(|button| button.get_visible());
            utilities_for_resize.set_visible(screensaver_visible);
            utility_row.set_visible(hide_all && screensaver_visible);
            let keep_menu_stable = overflow.is_active() && !mode_changed;
            let width = bar.width();
            if width > 0 {
                // Keep zoom and the three common photo controls directly
                // available. Promote secondary controls in pairs as the bar
                // grows, while preserving the overflow drawer for the rest.
                let visible_count = match width {
                    ..=639 => 0,
                    640..=759 => 2,
                    760..=899 => 4,
                    900..=1199 => 6,
                    _ => overflow_entries.len(),
                };
                let state = (visible_count, hide_all);
                if !keep_menu_stable && visible_action_count.replace(state) != state {
                    if !mode_changed {
                        overflow.popdown();
                    }
                    if hide_all {
                        if utilities_for_resize.parent().as_ref() == Some(actions.upcast_ref()) {
                            actions.remove(&utilities_for_resize);
                            utility_row.prepend(&utilities_for_resize);
                        }
                    } else if utilities_for_resize.parent().as_ref()
                        == Some(utility_row.upcast_ref())
                    {
                        utility_row.remove(&utilities_for_resize);
                        actions.insert_child_after(
                            &utilities_for_resize,
                            overflow.prev_sibling().as_ref(),
                        );
                    }
                    utility_row
                        .set_visible(hide_all && utilities_for_resize.first_child().is_some());
                    let mut previous: Option<gtk::Widget> = None;
                    for (index, (control, row)) in overflow_entries.iter().enumerate() {
                        if let Some(menu) = control.downcast_ref::<gtk::MenuButton>() {
                            if menu.is_active() {
                                menu.popdown();
                            }
                        }
                        // Import stays in the menu even when all photo controls fit.
                        let in_toolbar = !hide_all && index != 9 && index < 4 + visible_count;
                        row.set_visible(!in_toolbar && control.get_visible());
                        let parent = control.parent();
                        if in_toolbar {
                            if parent.as_ref() == Some(row.upcast_ref::<gtk::Widget>()) {
                                row.remove(control);
                                if index == 0 {
                                    grid_zoom_for_resize.set_width_request(64);
                                }
                                actions.insert_child_after(control, previous.as_ref());
                            } else if parent.is_none() {
                                actions.insert_child_after(control, previous.as_ref());
                            }
                            previous = Some(control.clone());
                        } else {
                            if parent.as_ref() == Some(actions.upcast_ref::<gtk::Widget>()) {
                                actions.remove(control);
                                if index == 0 {
                                    grid_zoom_for_resize.set_width_request(50);
                                    row.insert_child_after(control, row.first_child().as_ref());
                                } else {
                                    row.insert_child_after(control, None::<&gtk::Widget>);
                                }
                            } else if parent.is_none() {
                                if index == 0 {
                                    grid_zoom_for_resize.set_width_request(50);
                                    row.insert_child_after(control, row.first_child().as_ref());
                                } else {
                                    row.insert_child_after(control, None::<&gtk::Widget>);
                                }
                            }
                        }
                    }
                    let visible_groups = [
                        true,
                        (1..=3).any(|index| overflow_entries[index].1.get_visible()),
                        [5, 4]
                            .iter()
                            .any(|&index| overflow_entries[index].1.get_visible()),
                        [6, 7, 8]
                            .iter()
                            .any(|&index| overflow_entries[index].1.get_visible()),
                        [9, 11, 10]
                            .iter()
                            .any(|&index| overflow_entries[index].1.get_visible()),
                    ];
                    for (index, separator) in group_separators_for_resize.iter().enumerate() {
                        separator.set_visible(
                            visible_groups[index]
                                && visible_groups[index + 1..].iter().any(|visible| *visible),
                        );
                    }
                    zoom_separator.set_visible(!hide_all && grid_zoom_for_resize.get_visible());
                    if !hide_all {
                        actions.reorder_child_after(&zoom_separator, Some(&grid_zoom_for_resize));
                    }
                    overflow.set_visible(true);
                    hide_buttons_option_for_resize.set_active(hide_all);
                }
                // GTK caches label measurements. Only eight small metrics are
                // considered; no photo I/O or catalog queries run during resize.
                let mut available = metadata_viewport.width();
                let filename_minimum = 100;
                let show_text = available >= filename_minimum;
                text_for_resize.set_visible(show_text);
                if show_text {
                    available -= filename_minimum;
                }
                let mut visible_metrics = 0;
                // Taken is essential, then exposure fields, size/dimensions,
                // and finally the longer camera name. Visual order stays fixed.
                for index in [0, 4, 5, 6, 7, 3, 2, 1] {
                    let metric = &metrics[index];
                    let value = metric.last_child().and_downcast::<gtk::Label>().unwrap();
                    let heading = metric.first_child().unwrap();
                    let required = heading
                        .measure(gtk::Orientation::Horizontal, -1)
                        .1
                        .max(value.measure(gtk::Orientation::Horizontal, -1).1);
                    let gap = if show_text || visible_metrics > 0 {
                        16
                    } else {
                        0
                    };
                    let show = has_photo_for_resize.get()
                        && !value.text().is_empty()
                        && available >= required + gap;
                    metric.set_visible(show);
                    if show {
                        metric.set_width_request(required);
                        available -= required + gap;
                        visible_metrics += 1;
                    }
                }
                details_for_resize.set_visible(visible_metrics > 0);
                // Use surplus space for the thumbnail after useful metadata.
                let show_preview = available >= 56;
                preview_for_resize.set_visible(show_preview);
            }
            glib::ControlFlow::Continue
        });
        Self {
            root,
            utilities,
            preview,
            metadata,
            metadata_stack,
            selection_count,
            selection_controls,
            selection_thumbnails,
            selection_rendered_ids: RefCell::new(Vec::new()),
            selection_remove,
            selection_clear,
            filename,
            details,
            favorite,
            rating,
            rating_buttons,
            edit,
            collage,
            view_toggle,
            photo_layout,
            clear_saved_views,
            add_to_album,
            one_to_one,
            rotate,
            export,
            import_photos,
            more,
            print,
            overflow_popover,
            grid_zoom,
            grid_zoom_reset,
            has_photo,
            collage_active,
        }
    }

    /// Show a bounded strip of selected photos without reserving a fixed
    /// six-thumbnail width. GtkScrolledWindow clips to the width left over
    /// after the toolbar, count and Clear button.
    pub fn set_selected_photos(&self, count: usize, photos: &[PhotoObject]) {
        if count <= 1 {
            self.show_photo_metadata();
            return;
        }

        self.selection_count.set_text(&crate::window::format_count(count));
        self.selection_controls.set_visible(true);

        // A selection of thousands only samples a bounded set of thumbnails.
        // When that sample has not changed, avoid repeated image decodes and
        // Gtk widget churn just because the selected count changed.
        let rendered_ids = photos.iter().map(PhotoObject::id).collect::<Vec<_>>();
        let needs_refresh = self.selection_rendered_ids.borrow().as_slice() != rendered_ids.as_slice();
        if needs_refresh {
            while let Some(child) = self.selection_thumbnails.first_child() {
                self.selection_thumbnails.remove(&child);
            }
            for photo in photos {
                let button = gtk::Button::new();
                button.add_css_class("infobar-selected-thumb");
                button.set_tooltip_text(Some(&format!("Deselect {}", photo.filename())));
                let thumbnail = gtk::Image::new();
                thumbnail.set_pixel_size(38);
                thumbnail.set_size_request(38, 38);
                // GtkImage::pixel-size applies to icons, not file pixbufs.
                // Decode at display size to preserve the InfoBar height.
                let pixbuf = photo.cached_thumbnail_path()
                    .filter(|path| Path::new(path).is_file())
                    .and_then(|path| gdk_pixbuf::Pixbuf::from_file_at_scale(path, 38, 38, true).ok());
                if let Some(pixbuf) = pixbuf {
                    let texture = gtk::gdk::Texture::for_pixbuf(&pixbuf);
                    thumbnail.set_paintable(Some(&texture));
                } else {
                    thumbnail.set_icon_name(Some("image-x-generic-symbolic"));
                }
                button.set_child(Some(&thumbnail));
                let id = photo.id();
                let remove = self.selection_remove.clone();
                button.connect_clicked(move |_| {
                    if let Some(remove) = remove.borrow().as_ref() {
                        remove(id);
                    }
                });
                self.selection_thumbnails.append(&button);
            }
            *self.selection_rendered_ids.borrow_mut() = rendered_ids;
        }
        self.metadata_stack.set_visible_child_name("selection");
    }

    pub fn connect_selected_thumbnail_unselect(&self, remove: impl Fn(i64) + 'static) {
        self.selection_remove.replace(Some(Rc::new(remove)));
    }

    pub fn show_photo_metadata(&self) {
        self.metadata_stack.set_visible_child_name("photo");
        self.selection_controls.set_visible(false);
    }

    /// Synchronize a restored section without emitting a user layout change.
    pub fn set_photo_layout(&self, layout: crate::grid::PhotoLayout) {
        self.photo_layout.set(layout);
        update_photo_layout_button(&self.view_toggle, layout);
    }

    pub fn connect_photo_layout(&self, changed: impl Fn(crate::grid::PhotoLayout) + 'static) {
        let layout = self.photo_layout.clone();
        self.view_toggle
            .connect_clicked(move |_| changed(layout.get()));
    }

    pub fn connect_clear_saved_views(&self, clear: impl Fn() + 'static) {
        self.clear_saved_views.connect_activate(move |_, _| clear());
    }

    pub fn dismiss_action_menus(&self) {
        self.overflow_popover.popdown();
        if let Some(popover) = self.rating.popover() {
            popover.popdown();
        }
    }

    pub fn set_collage_active(&self, active: bool) {
        self.collage_active.set(active);
        self.view_toggle.set_sensitive(!active);
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
            self.filename.set_text("No photo selected");
            self.preview.set_icon_name(Some("image-x-generic-symbolic"));
            self.details.set_visible(false);
            set_metric_values(&self.details, [""; 8]);
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
                if photo.iso() > 0 {
                    photo.iso().to_string()
                } else {
                    String::new()
                },
                crate::window::format_shutter_speed(photo.shutter_speed()).unwrap_or_default(),
                crate::window::format_focal_length(photo.focal_length()).unwrap_or_default(),
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

fn toolbar_separator() -> gtk::Separator {
    let separator = gtk::Separator::new(gtk::Orientation::Vertical);
    separator.add_css_class("infobar-group-separator");
    separator.set_valign(gtk::Align::Center);
    separator.set_height_request(24);
    separator
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
            label.set_tooltip_text(Some(value.as_ref()));
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

fn update_photo_layout_button(button: &gtk::Button, layout: crate::grid::PhotoLayout) {
    use crate::grid::PhotoLayout;
    let (icon, tooltip) = match layout {
        PhotoLayout::Grid => ("collage-smart-mosaic-symbolic", "Switch to Photo Wall"),
        PhotoLayout::PhotoWall => ("view-masonry-symbolic", "Switch to Masonry"),
        PhotoLayout::Masonry => ("view-app-grid-symbolic", "Switch to Grid"),
    };
    button.set_icon_name(icon);
    button.set_tooltip_text(Some(tooltip));
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn themed_infobar_background_spans_parent_width() {
        gtk::init().unwrap();
        let display = gtk::gdk::Display::default().unwrap();
        let base = gtk::CssProvider::new();
        base.load_from_string(&format!(
            "{}\n{}",
            crate::css::BASE,
            include_str!("../themes/iDark/theme.css")
        ));
        gtk::style_context_add_provider_for_display(
            &display,
            &base,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        let overlay = gtk::CssProvider::new();
        overlay.load_from_string(include_str!("../themes/standard/theme.css"));
        gtk::style_context_add_provider_for_display(
            &display,
            &overlay,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        let info = InfoBar::new();
        let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
        parent.append(&info.root);
        parent.allocate(1800, 100, -1, None);
        let bounds = info.root.compute_bounds(&parent).unwrap();
        assert_eq!(bounds.x(), 0.0, "background must reach the left edge");
        assert_eq!(
            bounds.width(),
            1800.0,
            "background must reach the right edge"
        );
        let preview = info.preview.compute_bounds(&parent).unwrap();
        assert!(preview.x() >= 16.0, "contents must retain their inset");
        gtk::style_context_remove_provider_for_display(&display, &overlay);
        gtk::style_context_remove_provider_for_display(&display, &base);
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn selection_count_and_clear_stay_outside_the_thumbnail_scroller() {
        gtk::init().unwrap();
        let info = InfoBar::new();

        // A long selection can never push these controls into the scrolling
        // viewport. They are direct, non-expanding children of the root bar.
        info.set_selected_photos(40, &[]);
        assert_eq!(info.selection_count.text().as_str(), "40");
        assert!(info.selection_controls.is_visible());
        assert!(info.selection_clear.is_visible());
        assert_eq!(
            info.selection_controls.parent().unwrap(),
            info.root.clone().upcast::<gtk::Widget>()
        );
        assert_eq!(
            info.selection_count.parent().unwrap(),
            info.selection_controls.clone().upcast::<gtk::Widget>()
        );
        assert_eq!(
            info.selection_clear.parent().unwrap(),
            info.selection_controls.clone().upcast::<gtk::Widget>()
        );
        assert_eq!(info.metadata_stack.visible_child_name().as_deref(), Some("selection"));

        // Viewer/single selection restores photo metadata and frees the room.
        info.show_photo_metadata();
        assert!(!info.selection_controls.is_visible());
        info.set_selected_photos(2, &[]);
        assert_eq!(info.selection_count.text().as_str(), "2");
        assert!(info.selection_controls.is_visible());
        info.set_selected_photos(1, &[]);
        assert!(!info.selection_controls.is_visible());
        assert_eq!(info.metadata_stack.visible_child_name().as_deref(), Some("photo"));
    }
}

#[cfg(test)]
#[path = "infobar_responsive_tests.rs"]
mod responsive_tests;
