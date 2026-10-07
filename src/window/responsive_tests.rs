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
    for (movement, expected_width) in [
        (80.0_f64, sidebar_start + 80),
        (80.0, sidebar_start + 80),
        (180.0, sidebar_start + 180),
        (180.0, sidebar_start + 180),
        (1000.0, 600),
        (1000.0, 600),
        (40.0, sidebar_start + 40),
        (40.0, sidebar_start + 40),
        (-1000.0, 200),
        (0.0, sidebar_start),
    ] {
        let origin = handle
            .compute_point(&split, &gtk::graphene::Point::new(0.0, 0.0))
            .unwrap();
        let offset = press_in_split + movement - f64::from(origin.x()) - press_x;
        drag.emit_by_name::<()>("drag-update", &[&offset, &0.0_f64]);
        settle();
        assert!((sidebar.width() - expected_width).abs() <= 1,
            "{layout}/{destination}: sidebar did not follow pointer BEFORE release: start={sidebar_start} movement={movement} actual={}", sidebar.width());
        let content_delta = if compact {
            0
        } else {
            expected_width - sidebar_start
        };
        assert!(
            (split.content().unwrap().width() - content_start + content_delta).abs() <= 1,
            "{layout}/{destination}: gallery did not resize live"
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
    }
    window.close();
    let _ = std::fs::remove_file(path);
}
