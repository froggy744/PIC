use super::*;
use std::time::{Duration, Instant};

fn settle() {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(400);
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn menu_row_visible(menu: &gtk::Box, title: &str) -> Option<bool> {
    let mut child = menu.first_child();
    while let Some(widget) = child {
        if let Some(row) = widget.downcast_ref::<gtk::Box>() {
            let mut item = row.first_child();
            while let Some(item_widget) = item {
                if item_widget
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|label| label.text() == title)
                {
                    return Some(row.get_visible());
                }
                item = item_widget.next_sibling();
            }
        }
        child = widget.next_sibling();
    }
    None
}

fn wait_for_mapped(widgets: &[gtk::Widget]) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(2);
    while widgets.iter().any(|widget| !widget.is_mapped()) && Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::yield_now();
    }
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn compact_toolbar_keeps_actions_usable_and_restores_wide_layout() {
    libadwaita::init().unwrap();
    crate::css::install_foundation(&gtk::gdk::Display::default().unwrap());
    let bar = InfoBar::new();
    bar.set_photo(None);
    let utilities: Vec<_> = ["Screensaver"]
        .into_iter()
        .map(|title| {
            let button = gtk::Button::from_icon_name("help-about-symbolic");
            button.set_tooltip_text(Some(title));
            bar.append_utility(&button);
            button
        })
        .collect();
    let original_parent = bar.favorite.parent().unwrap();
    let clicks = Rc::new(Cell::new(0));
    let clicks_for_import = clicks.clone();
    bar.import_photos
        .connect_clicked(move |_| clicks_for_import.set(clicks_for_import.get() + 1));
    let window = gtk::Window::builder()
        .default_width(1400)
        .default_height(100)
        .child(&bar.root)
        .build();
    window.present();
    settle();
    let overflow = original_parent
        .last_child()
        .and_downcast::<gtk::MenuButton>()
        .unwrap();
    assert!(overflow.is_visible());
    let moved: Vec<gtk::Widget> = vec![
        bar.collage.clone().upcast(),
        bar.add_to_album.clone().upcast(),
        bar.one_to_one.clone().upcast(),
        bar.rotate.clone().upcast(),
        bar.export.clone().upcast(),
        bar.import_photos.clone().upcast(),
        bar.more.clone().upcast(),
        bar.print.clone().upcast(),
    ];
    for selected in [false, true] {
        if selected {
            let photo: PhotoObject = glib::Object::new();
            photo.set_filename("A long selected photo filename.jpg");
            photo.set_taken_at("2026-10-06");
            photo.set_camera("Camera with a long descriptive name");
            photo.set_width(6000);
            photo.set_height(4000);
            photo.set_aperture(5.6);
            bar.set_photo(Some(&photo));
        }
        for width in [960, 700, 600, 1400, 960, 700, 600] {
            // Continue native resize requests while responsive fields release
            // their previous minimum. A permanent size floor cannot pass.
            for attempt in 0..6 {
                window.set_default_size(width - (attempt % 2), 100);
                settle();
                if window.width() <= width + 2 {
                    break;
                }
            }
            eprintln!("TOOLBAR_RESIZE selected={selected} requested={width} actual={} minimum={} content={} actions={}", window.width(), bar.root.measure(gtk::Orientation::Horizontal, -1).0, bar.root.width(), original_parent.measure(gtk::Orientation::Horizontal, -1).0);
            assert!(
                window.width() <= width + 2,
                "selected={selected} requested={width} toolbar trapped at {}, minimum={} content={} actions={}",
                window.width(), bar.root.measure(gtk::Orientation::Horizontal, -1).0, bar.root.width(), original_parent.measure(gtk::Orientation::Horizontal, -1).0
            );
            for button in &utilities {
                assert!(button.is_mapped(), "utility hidden at {width}px");
                assert!(button.is_sensitive());
                assert_eq!(button.parent().as_ref(), Some(bar.utilities.upcast_ref()));
            }
            let visible_count = match bar.root.width() {
                ..=639 => 0,
                640..=759 => 2,
                760..=899 => 4,
                900..=1199 => 6,
                _ => moved.len(),
            };
            assert!(overflow.is_visible());
            assert_eq!(bar.export.is_sensitive(), selected);
            assert_eq!(bar.add_to_album.is_sensitive(), selected);
            assert!(bar.import_photos.is_sensitive());
            if selected && width == 960 {
                assert!(bar.filename.is_visible(), "filename hidden at 960px");
            }
            for (index, control) in moved.iter().enumerate() {
                assert_eq!(
                    control.parent().as_ref() == Some(&original_parent),
                    index != 5 && index < visible_count
                );
            }
            overflow.popup();
            wait_for_mapped(&moved);
            let menu_box = overflow
                .popover()
                .unwrap()
                .child()
                .unwrap()
                .downcast::<gtk::Box>()
                .unwrap();
            for (index, title) in [
                "Collage",
                "Add to Album",
                "100%",
                "Rotate",
                "Export",
                "Import",
                "Settings",
                "Print",
            ]
            .iter()
            .enumerate()
            {
                assert_eq!(
                    menu_row_visible(&menu_box, title),
                    Some(index == 5 || index >= visible_count),
                    "menu visibility mismatch for {title} at {width}px"
                );
            }
            if visible_count < moved.len()
                || bar.import_photos.parent().as_ref() != Some(&original_parent)
            {
                for control in &moved {
                    assert!(
                        control.is_mapped(),
                        "{} hidden in drawer: control={} row={:?} menu={} active={} mapped={}",
                        control.type_().name(),
                        control.get_visible(),
                        control.parent().map(|p| (p.get_visible(), p.is_mapped())),
                        overflow.popover().unwrap().get_visible(),
                        overflow.is_active(),
                        overflow.popover().unwrap().is_mapped()
                    );
                }
                if selected && width == 600 {
                    bar.rotate.emit_clicked();
                    settle();
                    assert!(
                        overflow.popover().unwrap().is_visible(),
                        "Rotate should leave the action menu open"
                    );
                }
                bar.import_photos.emit_clicked();
                settle();
                assert!(!overflow.popover().unwrap().is_visible());
            } else {
                overflow.popdown();
                bar.import_photos.emit_clicked();
            }
        }
    }
    let menu = overflow.popover().unwrap();
    let menu_box = menu.child().unwrap().downcast::<gtk::Box>().unwrap();
    let hide_buttons = menu_box
        .first_child()
        .unwrap()
        .downcast::<gtk::CheckButton>()
        .unwrap();
    overflow.popup();
    settle();
    assert!(
        menu.get_visible(),
        "menu must be open before toggling Menu only"
    );
    hide_buttons.set_active(true);
    settle();
    assert!(overflow.is_visible());
    assert!(
        menu.is_visible(),
        "Menu only must apply without closing the menu"
    );
    assert_ne!(bar.utilities.parent().as_ref(), Some(&original_parent));
    assert!(
        utilities[0].is_mapped(),
        "screensaver must remain available in the menu"
    );
    assert_ne!(bar.grid_zoom.parent().as_ref(), Some(&original_parent));
    assert_ne!(bar.favorite.parent().as_ref(), Some(&original_parent));
    assert_ne!(bar.rating.parent().as_ref(), Some(&original_parent));
    assert_ne!(bar.edit.parent().as_ref(), Some(&original_parent));
    assert!(
        bar.details.is_visible(),
        "metadata should use the released toolbar width"
    );
    hide_buttons.set_active(false);
    settle();
    assert!(
        menu.is_visible(),
        "revealing buttons must keep the menu open"
    );
    assert_eq!(bar.utilities.parent().as_ref(), Some(&original_parent));
    assert!(utilities[0].is_mapped());
    assert_eq!(bar.grid_zoom.parent().as_ref(), Some(&original_parent));
    assert_eq!(bar.favorite.parent().as_ref(), Some(&original_parent));
    assert_eq!(bar.rating.parent().as_ref(), Some(&original_parent));
    assert_eq!(bar.edit.parent().as_ref(), Some(&original_parent));
    assert_eq!(
        clicks.get(),
        14,
        "action handler must survive every move exactly once"
    );
    bar.favorite.set_visible(false);
    utilities[0].set_visible(false);
    hide_buttons.set_active(true);
    settle();
    assert_eq!(menu_row_visible(&menu_box, "Favourites"), Some(false));
    assert_eq!(menu_row_visible(&menu_box, "Screensaver"), Some(false));
    hide_buttons.set_active(false);
    settle();
    assert!(!bar.favorite.is_visible());
    assert!(!utilities[0].is_visible());
    bar.favorite.set_visible(true);
    utilities[0].set_visible(true);
    settle();
    assert!(bar.favorite.is_mapped());
    assert!(utilities[0].is_mapped());
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn adaptive_metadata_fills_space_without_constraining_resize() {
    libadwaita::init().unwrap();
    crate::css::install_foundation(&gtk::gdk::Display::default().unwrap());
    let display = gtk::gdk::Display::default().unwrap();
    let theme = gtk::CssProvider::new();
    theme.load_from_string(&format!(
        "{}\n{}",
        include_str!("../themes/iDark/theme.css"),
        include_str!("../themes/standard/theme.css")
    ));
    gtk::style_context_add_provider_for_display(
        &display,
        &theme,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let bar = InfoBar::new();
    let photo: PhotoObject = glib::Object::new();
    photo.set_filename("A long filename that should use the remaining metadata space.jpg");
    photo.set_taken_at("2026-10-14");
    photo.set_camera("Camera with a long descriptive name");
    photo.set_width(6000);
    photo.set_height(4000);
    photo.set_aperture(5.6);
    photo.set_iso(800);
    photo.set_shutter_speed(0.004);
    photo.set_focal_length(85.0);
    bar.set_photo(Some(&photo));
    let window = gtk::Window::builder()
        .default_width(1920)
        .default_height(100)
        .child(&bar.root)
        .build();
    window.present();
    settle();
    let actions = bar.grid_zoom.parent().unwrap();
    for scheme in [
        libadwaita::ColorScheme::ForceLight,
        libadwaita::ColorScheme::ForceDark,
    ] {
        libadwaita::StyleManager::default().set_color_scheme(scheme);
        window.maximize();
        settle();
        window.unmaximize();
        settle();
        for width in [1920, 1600, 1400, 1200, 1000, 800, 700, 600, 1920, 600] {
            for attempt in 0..6 {
                window.set_default_size(width - attempt % 2, 100);
                settle();
                if window.width() <= width + 2 {
                    break;
                }
            }
            assert!(
                window.width() <= width + 2,
                "resize trapped at {} for {width}",
                window.width()
            );
            let action_bounds = actions.compute_bounds(&bar.root).unwrap();
            assert!(action_bounds.x() >= 0.0);
            assert!(action_bounds.x() + action_bounds.width() <= bar.root.width() as f32 + 1.0);
            // Metadata must pack beside the filename, not be pushed against
            // the right-hand photo actions when there is surplus space.
            if bar.details.get_visible() && bar.filename.parent().unwrap().get_visible() {
                let filename_box = bar.filename.parent().unwrap();
                let name_bounds = filename_box.compute_bounds(&bar.root).unwrap();
                let mut metric = bar.details.first_child();
                while let Some(child) = metric {
                    if child.get_visible() {
                        let bounds = child.compute_bounds(&bar.root).unwrap();
                        let gap = bounds.x() - name_bounds.x() - name_bounds.width();
                        assert!(
                            (0.0..=24.0).contains(&gap),
                            "metadata not left-packed: gap {gap} at {width}"
                        );
                        break;
                    }
                    metric = child.next_sibling();
                }
            }
            let mut child = bar.details.first_child();
            let mut previous_right = -1.0;
            let mut previous_y = None;
            let mut exposure_count = 0;
            while let Some(metric) = child {
                let title = metric
                    .first_child()
                    .unwrap()
                    .downcast::<gtk::Label>()
                    .unwrap()
                    .text();
                let value = metric
                    .last_child()
                    .unwrap()
                    .downcast::<gtk::Label>()
                    .unwrap();
                if metric.get_visible() {
                    let bounds = metric.compute_bounds(&bar.root).unwrap();
                    assert!(bounds.x() >= previous_right, "metadata overlap at {width}");
                    assert!(bounds.x() + bounds.width() <= action_bounds.x());
                    if let Some(y) = previous_y {
                        assert_eq!(bounds.y(), y);
                    }
                    previous_y = Some(bounds.y());
                    previous_right = bounds.x() + bounds.width();
                    if let Some(expected) = match title.as_str() {
                        "Aperture" => Some("f/5.6"),
                        "ISO" => Some("800"),
                        "Shutter Speed" => Some("1/250 s"),
                        "Focal Length" => Some("85 mm"),
                        _ => None,
                    } {
                        assert_eq!(value.text().as_str(), expected);
                        exposure_count += 1;
                    }
                }
                child = metric.next_sibling();
            }
            if width >= 1600 {
                assert_eq!(exposure_count, 4);
            }
            if width == 600 {
                assert!(exposure_count < 4);
            }
            if width >= 600 {
                assert!(bar.filename.is_visible());
                assert!(bar.details.is_visible(), "Taken must survive at {width}");
            }
        }
    }
    photo.set_iso(0);
    photo.set_shutter_speed(0.0);
    photo.set_focal_length(0.0);
    photo.set_aperture(0.0);
    bar.set_photo(Some(&photo));
    window.set_default_size(1920, 100);
    settle();
    let mut child = bar.details.first_child();
    while let Some(metric) = child {
        let title = metric
            .first_child()
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap()
            .text();
        if ["Aperture", "ISO", "Shutter Speed", "Focal Length"].contains(&title.as_str()) {
            assert!(!metric.get_visible(), "missing EXIF must stay hidden");
        }
        child = metric.next_sibling();
    }
    window.close();
    gtk::style_context_remove_provider_for_display(&display, &theme);
}
