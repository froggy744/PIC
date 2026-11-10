use super::*;

fn settle() {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn split(widget: &gtk::Widget) -> Option<adw::OverlaySplitView> {
    if let Some(split) = widget.downcast_ref::<adw::OverlaySplitView>() {
        return Some(split.clone());
    }
    let mut child = widget.first_child();
    while let Some(w) = child {
        if let Some(found) = split(&w) {
            return Some(found);
        }
        child = w.next_sibling();
    }
    None
}

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut result = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(widget) = child {
        result.extend(descendants(&widget));
        child = widget.next_sibling();
    }
    result
}

fn check_live_sidebar_drag(layout: &str, destination: &str, compact: bool) {
    adw::init().unwrap();
    let app = adw::Application::builder()
        .application_id("io.pic.SidebarDragTest")
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let directory = std::env::temp_dir().join(format!(
        "pic-sidebar-drag-{}-{layout}-{}-{compact}",
        std::process::id(),
        destination.replace(':', "-")
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("library.db");
    let connection = db::open(&path).unwrap();
    for (key, value) in [
        (LAST_VIEW_SETTING_KEY, destination),
        ("photo_layout", layout),
        (GRID_THUMBNAIL_SIZE_SETTING_KEY, "219"),
        (WINDOW_WIDTH_SETTING_KEY, "1440"),
        (WINDOW_HEIGHT_SETTING_KEY, "900"),
        (SIDEBAR_WIDTH_FRACTION_SETTING_KEY, "0.22"),
        ("onboarding-never-show", "true"),
    ] {
        db::set_setting(&connection, key, value).unwrap();
    }
    connection
        .execute(
            "INSERT INTO folders(id,path,name) VALUES (1,?1,'Resize test')",
            [directory.to_str().unwrap()],
        )
        .unwrap();
    let image_path = directory.join("sample.png");
    image::RgbImage::from_pixel(32, 24, image::Rgb([80, 140, 210]))
        .save(&image_path)
        .unwrap();
    for id in 1..=240 {
        let photo_path = directory.join(format!("photo-{id}.png"));
        std::fs::hard_link(&image_path, &photo_path).unwrap();
        connection.execute(
            "INSERT INTO photos(id,path,folder_id,width,height,added_at) VALUES (?1,?2,1,32,24,?1)",
            rusqlite::params![id, photo_path.to_str().unwrap()],
        ).unwrap();
    }
    let window = build(&app, connection);
    window.present();
    settle();
    let split = split(window.upcast_ref()).unwrap();
    if compact {
        window.set_default_size(900, 900);
        settle();
        assert!(split.is_collapsed());
        split.set_show_sidebar(true);
        settle();
    }
    // The host display may cap the requested 1440px window to its available
    // logical width. Verify the mode GTK actually chose and base the resize
    // expectation on that mode: expanded split view allocates width away from
    // the gallery, while a collapsed sidebar is an overlay.
    let collapsed_overlay = split.is_collapsed();
    eprintln!(
        "sidebar drag mode={}: window={} split={} collapsed={} compact_setup={compact}",
        if collapsed_overlay {
            "overlay"
        } else {
            "split"
        },
        window.width(),
        split.width(),
        collapsed_overlay,
    );
    let sidebar = split.sidebar().unwrap();
    let handle = descendants(window.upcast_ref())
        .into_iter()
        .find(|widget| widget.has_css_class("sidebar-resize-handle"))
        .unwrap();
    let controllers = handle.observe_controllers();
    let drag = (0..controllers.n_items())
        .find_map(|index| controllers.item(index).and_downcast::<gtk::GestureDrag>())
        .unwrap();
    let visible_tiles = || {
        descendants(window.upcast_ref())
            .into_iter()
            .filter_map(|widget| widget.downcast::<grid::SquareTile>().ok())
            .filter(|tile| tile.is_mapped() && tile.photo().is_some())
            .collect::<Vec<_>>()
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    while visible_tiles().is_empty() && Instant::now() < deadline {
        settle();
    }
    let tiles = visible_tiles();
    assert!(!tiles.is_empty(), "{layout}/{destination}: no live tiles");
    let mut ancestor = tiles[0].parent();
    let scroll = loop {
        let widget = ancestor.expect("tile has no scroller");
        if let Ok(scroll) = widget.clone().downcast::<gtk::ScrolledWindow>() {
            break scroll;
        }
        ancestor = widget.parent();
    };
    scroll
        .vadjustment()
        .set_value(scroll.vadjustment().upper() * 0.4);
    settle();
    let original_scroll = scroll.vadjustment().value();

    let sidebar_start = sidebar.width();
    let content_start = split.content().unwrap().width();
    let effective_min_width = f64::from(
        split
            .sidebar()
            .unwrap()
            .measure(gtk::Orientation::Horizontal, -1)
            .0,
    )
    .max(SIDEBAR_MIN_WIDTH) as i32;
    eprintln!(
        "sidebar drag constraints: configured_min={} measured_child_min={} effective_min={} allocated_start={}",
        SIDEBAR_MIN_WIDTH,
        split.sidebar().unwrap().measure(gtk::Orientation::Horizontal, -1).0,
        effective_min_width,
        sidebar_start,
    );
    let press_x = 6.0_f64;
    let origin = handle
        .compute_point(&split, &gtk::graphene::Point::new(0.0, 0.0))
        .unwrap();
    let press_in_split = f64::from(origin.x()) + press_x;
    drag.emit_by_name::<()>("drag-begin", &[&press_x, &150.0_f64]);

    // GTK reports drag offsets in handle-local coordinates. As the handle
    // moves, send the local position of the SAME pointer in the split view.
    // A moving coordinate origin must never make a stationary pointer toggle
    // the sidebar between its old and new widths.
    let mut previous = None::<(
        f64,
        f64,
        std::collections::HashMap<i64, gtk::graphene::Rect>,
    )>;
    let minimum_movement = effective_min_width - sidebar_start;
    for (movement, expected_width) in [
        (80.0_f64, sidebar_start + 80),
        (80.0, sidebar_start + 80),
        (180.0, sidebar_start + 180),
        (180.0, sidebar_start + 180),
        ((600 - sidebar_start) as f64, 600),
        ((600 - sidebar_start) as f64, 600),
        (40.0, sidebar_start + 40),
        (40.0, sidebar_start + 40),
        (minimum_movement as f64, effective_min_width),
        (0.0, sidebar_start),
    ] {
        let origin = handle
            .compute_point(&split, &gtk::graphene::Point::new(0.0, 0.0))
            .unwrap();
        let offset = press_in_split + movement - f64::from(origin.x()) - press_x;
        drag.emit_by_name::<()>("drag-update", &[&offset, &0.0_f64]);
        settle();
        let collapsed_overlay_now = split.is_collapsed();
        assert!((sidebar.width() - expected_width).abs() <= 1,
            "{layout}/{destination}: sidebar did not follow pointer BEFORE release: start={sidebar_start} movement={movement} actual={}", sidebar.width());
        let content_delta = if collapsed_overlay_now {
            0
        } else {
            expected_width - sidebar_start
        };
        assert!(
            (split.content().unwrap().width() - content_start + content_delta).abs() <= 1,
            "{layout}/{destination}: gallery did not resize live: window={} split={} collapsed={} sidebar_start={} sidebar_now={} content_start={} content_now={} expected_delta={}",
            window.width(),
            split.width(),
            split.is_collapsed(),
            sidebar_start,
            sidebar.width(),
            content_start,
            split.content().unwrap().width(),
            content_delta,
        );
        let tiles = visible_tiles();
        assert!(
            !tiles.is_empty(),
            "{layout}/{destination}: tiles disappeared during resize"
        );
        assert!(
            tiles
                .iter()
                .any(|tile| tile.transition_paintable().is_some()),
            "{layout}/{destination}: live tiles lost their images"
        );
        let bounds = tiles
            .iter()
            .filter_map(|tile| Some((tile.photo()?.id(), tile.compute_bounds(&scroll)?)))
            .collect::<std::collections::HashMap<_, _>>();
        if let Some((last_movement, last_scroll, last_bounds)) = &previous {
            if *last_movement == movement {
                assert!(
                    (scroll.vadjustment().value() - last_scroll).abs() < 1.0,
                    "stationary pointer moved the scroll position"
                );
                for (id, rect) in &bounds {
                    let old = last_bounds
                        .get(id)
                        .expect("stationary pointer replaced a tile");
                    assert!(
                        (rect.x() - old.x()).abs() < 1.0
                            && (rect.y() - old.y()).abs() < 1.0
                            && (rect.width() - old.width()).abs() < 1.0
                            && (rect.height() - old.height()).abs() < 1.0,
                        "{layout}/{destination}: tile {id} jittered under a stationary pointer"
                    );
                }
            }
        }
        previous = Some((movement, scroll.vadjustment().value(), bounds));
    }
    drag.emit_by_name::<()>("drag-end", &[&0.0_f64, &0.0_f64]);
    settle();
    assert!(
        (sidebar.width() - sidebar_start).abs() <= 1,
        "release changed the final width"
    );
    if layout != "grid" {
        assert!(
            (scroll.vadjustment().value() - original_scroll).abs() < 4.0,
            "resizing back to the starting width lost the photo wall scroll anchor"
        );
    }
    drag.emit_by_name::<()>("drag-begin", &[&press_x, &150.0_f64]);
    drag.emit_by_name::<()>("drag-end", &[&0.0_f64, &0.0_f64]);
    settle();
    assert!(
        (sidebar.width() - sidebar_start).abs() <= 1,
        "click without motion resized the sidebar"
    );
    window.close();
    settle();
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn sidebar_drag_resizes_photos_grid_live() {
    check_live_sidebar_drag("grid", "all", false);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn sidebar_drag_resizes_folder_grid_live() {
    check_live_sidebar_drag("grid", "folder:1", false);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn sidebar_drag_keeps_photo_wall_stable_under_stationary_pointer() {
    check_live_sidebar_drag("photo_wall", "all", false);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn sidebar_drag_keeps_masonry_stable_under_stationary_pointer() {
    check_live_sidebar_drag("masonry", "all", false);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn sidebar_drag_resizes_compact_sidebar_live() {
    check_live_sidebar_drag("photo_wall", "all", true);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn full_window_can_resize_narrow_and_restore_sidebar() {
    adw::init().unwrap();
    let app = adw::Application::builder()
        .application_id("io.pic.ResponsiveWindowTest")
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let path =
        std::env::temp_dir().join(format!("pic-responsive-window-{}.db", std::process::id()));
    let window = build(&app, db::open(&path).unwrap());
    window.present();
    settle();
    let split = split(window.upcast_ref()).unwrap();
    for (width, height) in [
        (1920, 1000),
        (960, 650),
        (600, 650),
        (1440, 900),
        (960, 650),
        (600, 650),
    ] {
        // Continue native resize requests while responsive fields release
        // their previous minimum. A permanent size floor cannot pass.
        for attempt in 0..6 {
            window.set_default_size(width - (attempt % 2), height);
            settle();
            if window.width() <= width + 2 {
                break;
            }
        }
        assert!(
            window.width() <= width + 2,
            "requested {width}, actual {}, minimum {}",
            window.width(),
            window.measure(gtk::Orientation::Horizontal, -1).0
        );
        eprintln!(
            "resize requested={width} actual={} collapsed={}",
            window.width(),
            split.is_collapsed()
        );
        assert_eq!(split.is_collapsed(), width <= 1050);
        assert_eq!(split.shows_sidebar(), width > 1080);
        if split.is_collapsed() {
            split.set_show_sidebar(true);
            settle();
            assert!(split.sidebar().unwrap().width() < split.width(),
                "sidebar covers the entire window: sidebar={} split={}",
                split.sidebar().unwrap().width(), split.width());
            split.set_show_sidebar(false);
            settle();
        }
    }
    window.close();
    let _ = std::fs::remove_file(path);
}

#[test]
#[ignore = "requires a GTK display; run individually for each PIC_TEST_THEME"]
fn theme_header_edges_align() {
    adw::init().unwrap();
    let theme = std::env::var("PIC_TEST_THEME").unwrap_or_else(|_| "lars".into());
    let app = adw::Application::builder()
        .application_id("io.pic.HeaderAlignmentTest")
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let directory = std::env::temp_dir().join(format!("pic-header-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let connection = db::open(&directory.join("library.db")).unwrap();
    for (key, value) in [
        ("appearance-theme", theme.as_str()),
        ("onboarding-never-show", "true"),
        (WINDOW_WIDTH_SETTING_KEY, "1440"),
        (WINDOW_HEIGHT_SETTING_KEY, "900"),
        (SIDEBAR_WIDTH_FRACTION_SETTING_KEY, "0.22"),
    ] {
        db::set_setting(&connection, key, value).unwrap();
    }
    let compact = std::env::var_os("PIC_TEST_COMPACT").is_some();
    db::set_setting(
        &connection,
        "compact-sidebar-enabled",
        if compact { "true" } else { "false" },
    )
    .unwrap();
    let window = build(&app, connection);
    window.present();
    settle();
    if !compact {
        split(window.upcast_ref()).unwrap().set_show_sidebar(true);
        settle();
    }
    let widgets = descendants(window.upcast_ref());
    let left = widgets
        .iter()
        .find(|w| w.has_css_class("layout-left-header") && w.is_mapped())
        .unwrap();
    let right = widgets
        .iter()
        .find(|w| w.has_css_class("layout-right-header"))
        .unwrap();
    if compact {
        let dock = widgets
            .iter()
            .find(|w| w.has_css_class("compact-sidebar-dock"))
            .unwrap();
        let page = dock.first_child().unwrap();
        let d = dock.compute_bounds(&window).unwrap();
        let p = page.compute_bounds(&window).unwrap();
        assert!((d.y() - p.y()).abs() < 0.1, "compact dock has top padding");
        assert!(
            (d.y() + d.height() - p.y() - p.height()).abs() < 0.1,
            "compact dock has bottom padding"
        );
    }
    for width in [1440, 1100, 1600] {
        window.set_default_size(width, 900);
        settle();
        let l = left.compute_bounds(&window).unwrap();
        let r = right.compute_bounds(&window).unwrap();
        let left_bottom = l.y() + l.height();
        let right_bottom = r.y() + r.height();
        assert!(
            (l.y() - r.y()).abs() < 0.1,
            "{theme}: header top edges differ: left={}, right={}",
            l.y(),
            r.y()
        );
        eprintln!("{theme} width={width}: left bottom={left_bottom}, right bottom={right_bottom}");
        assert!(
            (left_bottom - right_bottom).abs() < 0.1,
            "{theme}: header bottom edges differ: left={left_bottom}, right={right_bottom}"
        );
    }
    if std::env::var_os("PIC_TEST_THEME_SWITCHES").is_some() {
        let appearance = widgets
            .iter()
            .find(|w| w.tooltip_text().as_deref() == Some("Appearance"))
            .unwrap()
            .downcast_ref::<gtk::Button>()
            .unwrap();
        appearance.emit_clicked();
        settle();
        let settings = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
            .find(|w| w.title().as_deref() == Some("Settings"))
            .unwrap();
        let checks: Vec<_> = descendants(settings.upcast_ref())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
            .filter(|w| {
                w.tooltip_text()
                    .is_some_and(|id| std::path::Path::new("themes").join(id.as_str()).is_dir())
            })
            .collect();
        assert_eq!(
            checks.len(),
            crate::css::theme_discovery::discover(std::path::Path::new("themes")).len()
        );
        for check in checks {
            let id = check.tooltip_text().unwrap();
            let row = check.ancestor(gtk::ListBoxRow::static_type()).unwrap();
            assert!(row.grab_focus());
            assert!(std::process::Command::new("xdotool")
                .args([
                    "search",
                    "--name",
                    "^Settings$",
                    "windowfocus",
                    "--sync",
                    "key",
                    "Return"
                ])
                .status()
                .unwrap()
                .success());
            settle();
            let saved = db::open(&directory.join("library.db")).unwrap();
            assert_eq!(
                db::setting(&saved, "appearance-theme").unwrap().as_deref(),
                Some(id.as_str()),
                "Enter did not activate {id}"
            );
            let l = left.compute_bounds(&window).unwrap();
            let r = right.compute_bounds(&window).unwrap();
            eprintln!(
                "switch + Enter {id}: left bottom={}, right bottom={}",
                l.y() + l.height(),
                r.y() + r.height()
            );
            assert!(
                (l.y() + l.height() - r.y() - r.height()).abs() < 0.1,
                "misaligned after switching to {id}"
            );
        }
        settings.close();
    }
    window.close();
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn honeycomb_properties_preserves_visible_photo_through_repeated_toggles() {
    adw::init().unwrap();
    let app = adw::Application::builder()
        .application_id("io.pic.HoneycombInspectorTest")
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let directory = std::env::temp_dir().join(format!("pic-inspector-anchor-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let mut connection = db::open(&directory.join("library.db")).unwrap();
    for (key, value) in [
        (LAST_VIEW_SETTING_KEY, "all"),
        ("photo_layout", "honeycomb"),
        (GRID_THUMBNAIL_SIZE_SETTING_KEY, "180"),
        (WINDOW_WIDTH_SETTING_KEY, "1440"),
        (WINDOW_HEIGHT_SETTING_KEY, "900"),
        ("onboarding-never-show", "true"),
    ] {
        db::set_setting(&connection, key, value).unwrap();
    }
    connection.execute(
        "INSERT INTO folders(id,path,name) VALUES (1,?1,'Inspector test')",
        [directory.to_str().unwrap()],
    ).unwrap();
    let real_library = std::env::var_os("PIC_TEST_INSPECTOR_LIBRARY");
    let photos = if let Some(path) = real_library {
        let source = rusqlite::Connection::open_with_flags(
            path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).unwrap();
        let mut query = source.prepare("SELECT path,width,height FROM photos WHERE trashed=0").unwrap();
        let rows = query.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
        }).unwrap();
        let photos = rows.filter_map(Result::ok)
            .filter(|(path, _, _)| std::path::Path::new(path).is_file())
            .take(4708).collect::<Vec<_>>();
        assert_eq!(photos.len(), 4708, "real library needs 4,708 available photos");
        photos
    } else {
        let image_path = directory.join("sample.jpg");
        image::RgbImage::from_pixel(32, 24, image::Rgb([80, 140, 210]))
            .save(&image_path).unwrap();
        (1..=4708).map(|id| {
            let photo_path = directory.join(format!("photo-{id}.jpg"));
            std::fs::hard_link(&image_path, &photo_path).unwrap();
            (photo_path.to_string_lossy().into_owned(), 32, 24)
        }).collect()
    };
    {
        let transaction = connection.transaction().unwrap();
        for (index, (path, width, height)) in photos.iter().enumerate() {
            let id = index as i64 + 1;
            transaction.execute(
                "INSERT INTO photos(id,path,folder_id,width,height,added_at) VALUES (?1,?2,1,?3,?4,?1)",
                rusqlite::params![id, path, width, height],
            ).unwrap();
        }
        transaction.commit().unwrap();
    }
    let window = build(&app, connection);
    window.present();
    settle();
    let visible_tiles = || {
        descendants(window.upcast_ref()).into_iter()
            .filter_map(|widget| widget.downcast::<grid::SquareTile>().ok())
            .filter(|tile| tile.is_mapped() && tile.photo().is_some())
            .collect::<Vec<_>>()
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while visible_tiles().is_empty() && Instant::now() < deadline { settle(); }
    let tile = visible_tiles().into_iter().next().expect("Honeycomb did not load");
    assert!(tile.has_css_class("honeycomb-tile"));
    let mut ancestor = tile.parent();
    let scroll = loop {
        let widget = ancestor.expect("tile has no scroller");
        if let Ok(scroll) = widget.clone().downcast::<gtk::ScrolledWindow>() { break scroll; }
        ancestor = widget.parent();
    };
    let inspector = descendants(window.upcast_ref()).into_iter()
        .filter_map(|widget| widget.downcast::<adw::OverlaySplitView>().ok())
        .find(|split| split.sidebar_position() == gtk::PackType::End)
        .expect("Properties split missing");
    let capture = |name: &str| {
        let Some(directory) = std::env::var_os("PIC_TEST_INSPECTOR_CAPTURE_DIR") else { return; };
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(&window)).snapshot(
            &snapshot, window.width() as f64, window.height() as f64,
        );
        let node = snapshot.to_node().expect("window screenshot is empty");
        std::fs::create_dir_all(&directory).unwrap();
        window.renderer().unwrap().render_texture(&node, None)
            .save_to_png(std::path::Path::new(&directory).join(format!("{name}.png"))).unwrap();
    };
    let adjustment = scroll.vadjustment();
    adjustment.set_value((adjustment.upper() - adjustment.page_size()) * 0.5);
    settle();
    let zoom = descendants(window.upcast_ref()).into_iter()
        .find_map(|widget| {
            if widget.has_css_class("infobar-zoom") {
                widget.downcast::<gtk::Scale>().ok()
            } else {
                None
            }
        }).expect("gallery zoom slider missing");
    for cycle in 0..10 {
        if cycle == 0 || cycle == 5 {
            let before_width = visible_tiles()[0].width();
            zoom.set_value(if cycle == 0 { 2.0 } else { 5.0 });
            settle();
            settle();
            assert_ne!(visible_tiles()[0].width(), before_width, "zoom did not change tile size");
        }
        if cycle == 3 || cycle == 7 {
            let before_width = window.width();
            window.set_default_size(if cycle == 3 { 1250 } else { 1440 }, 900);
            settle();
            assert_ne!(window.width(), before_width, "window did not resize");
        }
        for open in [true, false] {
            let center = adjustment.page_size() as f32 * 0.5;
            let tiles = visible_tiles();
            let (_, anchor_y) = tiles.iter().filter_map(|tile| {
                let bounds = tile.compute_bounds(&scroll)?;
                (bounds.y() <= center && bounds.y() + bounds.height() > center)
                    .then_some((tile.photo()?.id(), bounds.y()))
            }).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
            // The geometry anchor is the first photo in the row nearest the center.
            let (anchor_id, anchor_y, _) = tiles.iter().filter_map(|tile| {
                let bounds = tile.compute_bounds(&scroll)?;
                ((bounds.y() - anchor_y).abs() < 1.0)
                    .then_some((tile.photo()?.id(), bounds.y(), bounds.x()))
            }).min_by(|left, right| left.2.total_cmp(&right.2)).unwrap();
            if cycle == 0 && open { capture("before"); }
            inspector.set_show_sidebar(open);
            if cycle % 2 == 0 {
                let context = glib::MainContext::default();
                let until = Instant::now() + Duration::from_millis(75);
                while Instant::now() < until {
                    while context.pending() { context.iteration(false); }
                    std::thread::sleep(Duration::from_millis(5));
                }
                inspector.set_show_sidebar(!open);
                inspector.set_show_sidebar(open);
            }
            settle();
            settle();
            let actual = visible_tiles().into_iter()
                .find(|tile| tile.photo().is_some_and(|photo| photo.id() == anchor_id))
                .and_then(|tile| tile.compute_bounds(&scroll))
                .unwrap_or_else(|| panic!("anchor photo {anchor_id} disappeared in cycle {cycle}"));
            assert!((actual.y() - anchor_y).abs() <= 3.0,
                "anchor photo {anchor_id} shifted in cycle {cycle}: {} -> {}", anchor_y, actual.y());
            if cycle == 0 { capture(if open { "open" } else { "closed" }); }
            assert_eq!(scroll.hadjustment().value(), scroll.hadjustment().lower());
            assert_eq!(scroll.hscrollbar_policy(), gtk::PolicyType::Never);
            if cycle == 2 && open {
                let handle = descendants(inspector.upcast_ref()).into_iter()
                    .find(|widget| widget.has_css_class("pic-properties-resize-handle"))
                    .expect("Properties divider missing");
                let controllers = handle.observe_controllers();
                let drag = (0..controllers.n_items())
                    .find_map(|index| controllers.item(index).and_downcast::<gtk::GestureDrag>())
                    .expect("Properties divider gesture missing");
                let width = inspector.sidebar().unwrap().width();
                drag.emit_by_name::<()>("drag-begin", &[&6.0_f64, &150.0_f64]);
                drag.emit_by_name::<()>("drag-update", &[&-60.0_f64, &0.0_f64]);
                drag.emit_by_name::<()>("drag-end", &[&-60.0_f64, &0.0_f64]);
                settle();
                assert_ne!(inspector.sidebar().unwrap().width(), width, "divider did not resize");
            }
        }
    }
    window.close();
    settle();
    std::fs::remove_dir_all(directory).unwrap();
}
