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
