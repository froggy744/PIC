use super::*;

fn fixture() -> (std::path::PathBuf, rusqlite::Connection) {
    let path = std::env::temp_dir().join(format!(
        "pic-home-{}-{}.db",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let connection = db::open(&path).unwrap();
    connection.execute("INSERT INTO photos(id,path,favorite) VALUES (1,'smb://offline/share/one.jpg',1),(2,'smb://offline/share/two.jpg',0)", []).unwrap();
    db::set_edit_recipe(&connection, 1, "edited").unwrap();
    db::create_album(&connection, "Family").unwrap();
    (path, connection)
}

#[test]
fn home_worker_refreshes_favorites_from_another_connection() {
    let (path, connection) = fixture();
    let (requests, receiver) = mpsc::sync_channel(1);
    let (sender, snapshots) = mpsc::channel();
    let thread = std::thread::spawn(move || worker(receiver, sender));
    requests
        .send(HomeRequest {
            database: path.clone(),
            force: true,
            scrapbook_album_id: None,
        })
        .unwrap();
    let initial = snapshots
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .1
        .unwrap()
        .unwrap();
    assert_eq!(
        initial
            .data
            .favorites
            .iter()
            .map(|p| p.id)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(initial.data.added.len(), 2);
    assert_eq!(initial.data.edited.len(), 1);
    assert_eq!(initial.data.albums.len(), 1);
    assert!(initial.images.is_empty());
    db::set_favorite(&connection, 1, false).unwrap();
    db::set_favorite(&connection, 2, true).unwrap();
    requests
        .send(HomeRequest {
            database: path.clone(),
            force: false,
            scrapbook_album_id: None,
        })
        .unwrap();
    let refreshed = snapshots
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .1
        .unwrap()
        .unwrap();
    assert_eq!(
        refreshed
            .data
            .favorites
            .iter()
            .map(|p| p.id)
            .collect::<Vec<_>>(),
        vec![2]
    );
    assert_eq!(db::history_photos(&connection).unwrap().len(), 1);
    requests
        .send(HomeRequest {
            database: path.clone(),
            force: false,
            scrapbook_album_id: None,
        })
        .unwrap();
    assert!(snapshots
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .1
        .unwrap()
        .is_none());
    drop(requests);
    thread.join().unwrap();
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn home_cached_thumbnail_does_not_delete_or_regenerate_corrupt_cache() {
    let path = std::env::temp_dir().join(format!("pic-home-cache-{}.jpg", std::process::id()));
    std::fs::write(&path, b"corrupt cached thumbnail").unwrap();
    let mut request = thumbnail_display::request_for(
        path.to_string_lossy().into_owned(),
        "smb://offline/original.jpg".into(),
        0,
        0,
        0,
        String::new(),
        0,
        0,
        false,
    );
    assert!(matches!(
        thumbnail_display::load_cached_display_thumbnail(&request),
        thumbnail_display::DisplayOutcome::Failed
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"corrupt cached thumbnail");
    image::RgbImage::from_pixel(4, 2, image::Rgb([200, 40, 40]))
        .save(&path)
        .unwrap();
    request.rotation = 90;
    match thumbnail_display::load_cached_display_thumbnail(&request) {
        thumbnail_display::DisplayOutcome::Loaded {
            width,
            height,
            pixels,
        } => {
            assert_eq!((width, height), (2, 4));
            assert_eq!(pixels.len(), 32);
        }
        outcome => panic!("Expected cached image for unavailable original, got {outcome:?}"),
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn home_preview_limit_is_twenty() {
    assert_eq!(db::HOME_PREVIEW_LIMIT, 20);
}

fn collect<T: IsA<gtk::Widget> + glib::object::ObjectType>(
    widget: &gtk::Widget,
    result: &mut Vec<T>,
) {
    if let Ok(item) = widget.clone().downcast::<T>() {
        result.push(item);
    }
    let mut child = widget.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        collect(&item, result);
    }
}

fn collect_home_tracks(root: &gtk::Widget) -> Vec<gtk::Box> {
    let mut tracks = Vec::<gtk::Box>::new();
    collect(root, &mut tracks);
    tracks.retain(|track| {
        track
            .css_classes()
            .iter()
            .any(|class| class == "home-section-track")
    });
    tracks
}

fn collect_home_rows(root: &gtk::Widget) -> Vec<gtk::ScrolledWindow> {
    let mut rows = Vec::<gtk::ScrolledWindow>::new();
    collect(root, &mut rows);
    rows.retain(|row| {
        row.css_classes()
            .iter()
            .any(|class| class == "home-section-row")
    });
    rows
}

fn arrow_visibles_from(root: &gtk::Widget) -> Vec<(String, bool, bool)> {
    let mut buttons = Vec::<gtk::Button>::new();
    collect(root, &mut buttons);
    buttons
        .iter()
        .filter(|b| b.icon_name().is_some())
        .map(|b| {
            (
                b.icon_name().map(|n| n.to_string()).unwrap_or_default(),
                b.is_visible(),
                b.is_sensitive(),
            )
        })
        .collect()
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn home_page_sections_navigation_and_favorite_refresh() {
    libadwaita::init().unwrap();
    let (path, connection) = fixture();
    let destinations = Rc::new(std::cell::RefCell::new(Vec::new()));
    let opened = Rc::new(std::cell::RefCell::new(Vec::new()));
    let home = LibraryHome::new(
        path.clone(),
        {
            let destinations = destinations.clone();
            Rc::new(move |filter| destinations.borrow_mut().push(filter))
        },
        {
            let opened = opened.clone();
            Rc::new(move |photos, index, edited| {
                opened.borrow_mut().push((photos[index].id(), edited))
            })
        },
        Rc::new(|_| {}),
        Rc::new(|_| {}),
    );
    let window = gtk::Window::new();
    window.set_default_size(900, 700);
    window.set_child(Some(&home.root));
    window.present();
    let context = glib::MainContext::default();
    let sections = collect_home_tracks(home.root.upcast_ref());
    let rows = collect_home_rows(home.root.upcast_ref());
    assert_eq!(sections.len(), 4);
    assert_eq!(rows.len(), 4);
    for row in &rows {
        let (h, v) = row.policy();
        assert_eq!(h, gtk::PolicyType::Automatic);
        assert_eq!(v, gtk::PolicyType::Never);
        assert!(row.is_kinetic_scrolling());
    }
    let wait_for = |label: &str, ready: &dyn Fn() -> bool| {
        let start = std::time::Instant::now();
        while !ready() && start.elapsed() < Duration::from_secs(8) {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
        if !ready() {
            let mut labels = Vec::<gtk::Label>::new();
            collect(home.root.upcast_ref(), &mut labels);
            let texts: Vec<_> = labels.iter().map(|l| l.text().to_string()).collect();
            let mut tracks = Vec::<gtk::Box>::new();
            collect(home.root.upcast_ref(), &mut tracks);
            let counts: Vec<_> = tracks
                .iter()
                .filter(|t| t.css_classes().iter().any(|c| c == "home-section-track"))
                .map(|t| {
                    let mut n = 0;
                    let mut child = t.first_child();
                    while child.is_some() {
                        n += 1;
                        child = child.and_then(|c| c.next_sibling());
                    }
                    n
                })
                .collect();
            let arrow_visibles = arrow_visibles_from(home.root.upcast_ref());
            let hadj = rows[0].hadjustment();
            panic!(
                "timeout after {label}; track_child_counts={counts:?}; \
                 arrows={arrow_visibles:?}; hadj=({}, {}, {}); labels={texts:?}",
                hadj.upper(),
                hadj.page_size(),
                hadj.value(),
            );
        }
    };
    wait_for("initial populate", &|| sections[0].first_child().is_some());
    // Square preview cards keep a fixed frame size.
    let mut buttons = Vec::<gtk::Button>::new();
    collect(sections[0].upcast_ref(), &mut buttons);
    assert!(!buttons.is_empty());
    wait_for("card allocation", &|| {
        buttons[0].width() > 0 && buttons[0].height() > 0
    });
    let frame = buttons[0]
        .first_child()
        .and_then(|child| child.downcast::<gtk::Box>().ok())
        .and_then(|content| content.first_child())
        .and_then(|child| child.downcast::<gtk::Overlay>().ok())
        .expect("card frame");
    assert_eq!(frame.width(), frame.height());
    assert_eq!(frame.width(), CARD_THUMB);
    assert_eq!(CARD_THUMB, 180);
    // Cards stay left-aligned with a fixed gap; no even distribution.
    let track = &sections[0];
    assert!(track
        .css_classes()
        .iter()
        .any(|class| class == "home-section-track"));
    if let Some(button) = track.first_child() {
        assert_eq!(button.halign(), gtk::Align::Start);
        assert!(!button.hexpands());
    }
    // Recently Added captions are filenames, not history timestamps.
    {
        let mut labels = Vec::<gtk::Label>::new();
        collect(sections[0].upcast_ref(), &mut labels);
        let texts: Vec<String> = labels.iter().map(|l| l.text().to_string()).collect();
        assert!(
            texts.iter().any(|t| t == "two.jpg" || t == "one.jpg"),
            "expected filenames in Recently Added, got {texts:?}"
        );
        assert!(
            !texts.iter().any(|t| t.contains('·')),
            "Recently Added must not show history captions: {texts:?}"
        );
    }
    // Recently Edited: filename primary, history line as smaller secondary.
    {
        let mut labels = Vec::<gtk::Label>::new();
        collect(sections[1].upcast_ref(), &mut labels);
        let texts: Vec<String> = labels.iter().map(|l| l.text().to_string()).collect();
        assert!(
            texts.iter().any(|t| t == "one.jpg"),
            "expected filename primary in Recently Edited, got {texts:?}"
        );
        assert!(
            texts
                .iter()
                .any(|t| t.starts_with("Photo ·") || t.starts_with("Collage ·")),
            "expected history secondary text in Recently Edited, got {texts:?}"
        );
        let secondary: Vec<_> = labels
            .iter()
            .filter(|l| {
                l.css_classes()
                    .iter()
                    .any(|c| c == "home-card-caption-secondary")
            })
            .collect();
        assert!(!secondary.is_empty(), "edited card needs secondary caption");
    }

    // Scroll buttons appear only once a row overflows the viewport.
    let arrow_visibles = || arrow_visibles_from(home.root.upcast_ref());
    let scroll_buttons = arrow_visibles();
    assert!(scroll_buttons
        .iter()
        .any(|(name, _, _)| name == "pan-start-symbolic"));
    assert!(scroll_buttons
        .iter()
        .any(|(name, _, _)| name == "pan-end-symbolic"));
    // Two photos fit in 900px, so no row is scrollable yet.
    assert!(scroll_buttons.iter().all(|(_, visible, _)| !*visible));

    if let Some(path) = std::env::var_os("PIC_HOME_SCREENSHOT") {
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(&window)).snapshot(
            &snapshot,
            window.width() as f64,
            window.height() as f64,
        );
        let node = snapshot.to_node().unwrap();
        window
            .renderer()
            .unwrap()
            .render_texture(&node, None)
            .save_to_png(path)
            .unwrap();
    }

    // Overflow the first row and check horizontal scrolling plus position memory.
    for id in 3..=12 {
        connection
            .execute(
                "INSERT INTO photos(id,path) VALUES (?1,?2)",
                rusqlite::params![id, format!("smb://offline/share/{id}.jpg")],
            )
            .unwrap();
    }
    wait_for("row overflow", &|| {
        rows[0].hadjustment().upper() > rows[0].hadjustment().page_size() + 1.0
    });
    wait_for("scroll buttons visible", &|| {
        // Only the overflowing first row should show its scroll arrows.
        let arrows = arrow_visibles();
        arrows.len() >= 8
            && arrows.iter().take(2).all(|(_, visible, _)| *visible)
            && arrows.iter().skip(2).all(|(_, visible, _)| !*visible)
    });

    let hadj = rows[0].hadjustment();
    hadj.set_value(120.0);
    assert_eq!(hadj.value(), 120.0);
    // A card click in a later section forces a snapshot rebuild.
    db::set_favorite(&connection, 2, true).unwrap();
    wait_for("favorite row content", &|| {
        let mut labels = Vec::<gtk::Label>::new();
        collect(sections[2].upcast_ref(), &mut labels);
        labels
            .iter()
            .any(|label| label.text() == "two.jpg" || label.text() == "one.jpg")
    });
    wait_for("scroll position memory", &|| {
        (hadj.value() - 120.0).abs() < 0.5
    });

    let mut all_buttons = Vec::<gtk::Button>::new();
    collect(home.root.upcast_ref(), &mut all_buttons);
    for button in all_buttons
        .iter()
        .filter(|b| matches!(b.label().as_deref(), Some("View All" | "View History")))
    {
        button.emit_clicked();
    }
    assert_eq!(
        *destinations.borrow(),
        vec![
            SidebarFilter::RecentlyAdded,
            SidebarFilter::History,
            SidebarFilter::Favorites,
            SidebarFilter::Albums
        ]
    );
    let click_card = |section: usize| {
        let mut buttons = Vec::<gtk::Button>::new();
        collect(sections[section].upcast_ref(), &mut buttons);
        buttons[0].emit_clicked();
    };
    click_card(1);
    click_card(2);
    click_card(3);
    assert_eq!(*opened.borrow(), vec![(1, true), (1, false)]);
    assert_eq!(destinations.borrow().last(), Some(&SidebarFilter::Album(1)));
    db::set_favorite(&connection, 1, false).unwrap();
    db::set_favorite(&connection, 2, false).unwrap();
    wait_for("favourites cleared", &|| {
        let mut labels = Vec::<gtk::Label>::new();
        collect(sections[2].upcast_ref(), &mut labels);
        labels
            .iter()
            .any(|label| label.text() == "No favourites yet")
    });
    window.destroy();
    drop(home);
    while context.pending() {
        context.iteration(false);
    }
    drop(connection);
    std::fs::remove_file(path).unwrap();
}


/// Visual regression fixture for the actual GTK Scrapbook layout. The test
/// generates synthetic coloured thumbnails in memory only; it never injects
/// images into the user's photo library. Run under Xvfb with xdotool and
/// ImageMagick to capture screenshots of the rendered widgets.
#[test]
#[ignore = "requires Xvfb, ImageMagick and xdotool"]
fn scrapbook_reference_capture_layout() {
    use std::path::Path;
    gtk::init().expect("GTK test display");
    #[cfg(target_os = "linux")]
    crate::register_scrapbook_fonts();
    let display = gtk::gdk::Display::default().expect("display");
    crate::css::install_foundation(&display);
    let css = gtk::CssProvider::new();
    let theme_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("themes/scrapbook/theme.css");
    css.load_from_path(&theme_path);
    gtk::style_context_add_provider_for_display(
        &display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 7
    );
    let (path, conn) = fixture();
    for id in 3_i64..=25 {
        conn.execute(
            "INSERT INTO photos(id,path,taken_at,width,height,size_bytes,iso,aperture,favorite,rating)
             VALUES (?1,?2,'2026-05-25',600,400,1250000,64,2.2,?3,?4)",
            rusqlite::params![id, format!("/fixture/Coastland_{id:03}.jpg"), id % 4 == 0, (id % 5) as i32],
        ).unwrap();
    }
    let album_one = db::create_album(&conn, "FB-2026 Coastland Sales").unwrap();
    let ids: Vec<i64> = (3..=25).collect();
    db::add_photos_to_album(&conn, album_one.id, &ids).unwrap();
    let album_two = db::create_album(&conn, "Concrete Palisade").unwrap();
    db::add_photos_to_album(&conn, album_two.id, &[4, 5, 6, 7, 8, 9]).unwrap();
    let album_three = db::create_album(&conn, "Creche").unwrap();
    db::add_photos_to_album(&conn, album_three.id, &[10, 11, 12, 13, 14]).unwrap();
    db::scrapbook_add_tag(&conn, 25, "Project").unwrap();
    db::scrapbook_add_tag(&conn, 25, "Exterior").unwrap();
    db::scrapbook_set_note(&conn, 25, "Facade detail for the project summary.").unwrap();
    let data = db::library_home_data(&conn).unwrap();

    let draw = |id: i64| {
        let width = 240_i32;
        let height = 154_i32;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let sky = y < 78;
                let building = y >= 59 && y < 128 && x >= 40 && x < 211;
                let fence = y >= 77 && x % 17 < 3;
                let (mut red, mut green, mut blue) = if sky {
                    (80 + (x % 27) as u8, 143 + (y % 38) as u8, 218)
                } else if building {
                    (163, 169, 169)
                } else {
                    (77, 144, 85)
                };
                if building && x % 36 < 8 {
                    red = 61; green = 72; blue = 85;
                }
                if fence {
                    red = 75; green = 71; blue = 68;
                }
                if y > 134 {
                    red = red.saturating_add(18);
                }
                let pixel = ((y * width + x) * 4) as usize;
                pixels[pixel..pixel + 4].copy_from_slice(&[
                    red.saturating_add((id % 8) as u8 * 3), green, blue, 255
                ]);
            }
        }
        Arc::new(PreviewImage { width, height, pixels })
    };
    let mut images = HashMap::new();
    for (_, cover) in &data.albums {
        if let Some(photo) = cover { images.insert(photo.id, draw(photo.id)); }
    }
    for photo in &data.scrapbook_photos {
        images.insert(photo.id, draw(photo.id));
    }
    let snapshot = Snapshot { data, images };
    let destinations = Rc::new(std::cell::RefCell::new(Vec::<SidebarFilter>::new()));
    let album_requests = Rc::new(std::cell::RefCell::new(Vec::<i64>::new()));
    let navigate: Navigate = Rc::new({
        let destinations = destinations.clone();
        move |destination| destinations.borrow_mut().push(destination)
    });
    let open: OpenPhoto = Rc::new(|_, _, _| {});
    let request_album: Rc<dyn Fn(i64)> = Rc::new({
        let album_requests = album_requests.clone();
        move |id| album_requests.borrow_mut().push(id)
    });
    let dashboard = super::scrapbook::ScrapbookDashboard::new(
        &navigate, &open, path.clone(), request_album,
        Rc::new(|_| {}), Rc::new(|_| {}),
    );
    dashboard.root.set_visible(true);
    dashboard.populate(&snapshot, &navigate, &open);
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_child(Some(&dashboard.root));
    let window = gtk::Window::new();
    window.set_title(Some("PIC Scrapbook Visual Fixture"));
    window.set_default_size(1550, 900);
    window.set_child(Some(&scroller));
    window.present();
    let context = glib::MainContext::default();
    for _ in 0..80 {
        while context.pending() { context.iteration(false); }
        std::thread::sleep(Duration::from_millis(12));
    }
    assert!(dashboard.root.width() > 1000, "scrapbook root needs a wide preview viewport");
    if let Ok(dir) = std::env::var("PIC_SCRAPBOOK_SCREENSHOT_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        let window_id = std::process::Command::new("xdotool")
            .args(["search", "--onlyvisible", "--name", "^PIC Scrapbook Visual Fixture$"])
            .output().expect("xdotool available");
        assert!(window_id.status.success(), "GUI window not found by xdotool");
        let id = String::from_utf8_lossy(&window_id.stdout)
            .lines().last().expect("window id").to_string();
        for (width, height, name) in [
            (1550, 900, "scrapbook-1550x900.png"),
            (1366, 768, "scrapbook-1366x768.png"),
        ] {
            window.set_default_size(width, height);
            // The default Xvfb screen is only 1280 wide. Resize the actual
            // X11 test window, not merely its GTK default-size hint, so
            // the two screenshots validate distinct real window sizes.
            let resized = std::process::Command::new("xdotool")
                .args(["windowsize", "--sync", &id, &width.to_string(), &height.to_string()])
                .status().expect("resize test window");
            assert!(resized.success(), "could not resize GTK screenshot window");
            for _ in 0..35 {
                while context.pending() { context.iteration(false); }
                std::thread::sleep(Duration::from_millis(12));
            }
            let target = Path::new(&dir).join(name);
            let status = std::process::Command::new("import")
                .args(["-window", &id, target.to_str().unwrap()])
                .status().expect("ImageMagick available");
            assert!(status.success(), "GTK screenshot capture failed");
            assert!(target.is_file(), "missing screenshot");
            let output = std::process::Command::new("identify")
                .args(["-format", "%wx%h", target.to_str().unwrap()])
                .output().expect("ImageMagick identify");
            assert!(output.status.success());
            let actual = String::from_utf8_lossy(&output.stdout);
            assert_eq!(actual, format!("{width}x{height}"),
                "GTK screenshot was cropped by the virtual desktop");
        }
    }
    // Clicking an album previews it inside the Library. The Full Albums
    // page is only opened by the explicit View All Albums action.
    fn find_class(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
        if widget.has_css_class(class) {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            if let Some(found) = find_class(&current, class) {
                return Some(found);
            }
        }
        None
    }
    fn find_label(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
            if button.label().as_deref() == Some(label) {
                return Some(button);
            }
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            if let Some(found) = find_label(&current, label) {
                return Some(found);
            }
        }
        None
    }
    let root_widget = dashboard.root.upcast_ref::<gtk::Widget>();
    let album_scroll = find_class(root_widget, "scrapbook-album-scroller")
        .expect("album shelf must have its own scrollbar");
    let photo_scroll = find_class(root_widget, "scrapbook-photo-scroller")
        .expect("album photos must have their own scrollbar");
    assert!(album_scroll.is::<gtk::ScrolledWindow>());
    assert!(photo_scroll.is::<gtk::ScrolledWindow>());
    let photo_track = find_class(root_widget, "scrapbook-photo-grid")
        .unwrap().downcast::<gtk::Box>().unwrap();
    assert_eq!(photo_track.orientation(), gtk::Orientation::Horizontal);
    let album_track = find_class(root_widget, "scrapbook-albums-row")
        .unwrap().downcast::<gtk::Box>().unwrap();
    assert_eq!(album_track.orientation(), gtk::Orientation::Horizontal);
    let first_card = album_track.first_child().unwrap().downcast::<gtk::Button>().unwrap();
    // The album shelf is alphabetically sorted; Concrete Palisade comes first.
    first_card.emit_clicked();
    assert_eq!(album_requests.borrow().as_slice(), &[album_two.id]);
    assert_eq!(dashboard.active_album(), Some(album_two.id));
    assert!(!dashboard.accepts_snapshot(&snapshot));
    assert!(destinations.borrow().is_empty(), "album preview must not navigate");
    let data_two = db::library_home_data_for_album(&conn, Some(album_two.id)).unwrap();
    let mut images_two = HashMap::new();
    for photo in &data_two.scrapbook_photos {
        images_two.insert(photo.id, draw(photo.id));
    }
    dashboard.populate(&Snapshot { data: data_two, images: images_two }, &navigate, &open);
    assert_eq!(photo_track.first_child().is_some(), true);
    assert!(!dashboard.album_is_loading());
    find_label(root_widget, "View all albums")
        .expect("Library must offer explicit navigation to the Albums page")
        .emit_clicked();
    assert_eq!(&*destinations.borrow(), &[SidebarFilter::Albums]);
    window.destroy();
    drop(conn);
    let _ = std::fs::remove_file(path);
}
