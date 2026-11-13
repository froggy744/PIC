use super::*;

// Immersive Mode owns the left navigation for the whole session. These
// tests cover the state ownership: entering the mode must hide every piece
// of left navigation (the 44px compact rail included) without touching the
// saved Compact UI preference, and leaving it must restore the exact
// configuration that was active before.
//
// Like the other display tests of the window they need a GTK display and
// must be started one per process, because GTK can only be initialized on
// the thread that owns the display:
//
//   xvfb-run -a env GSK_RENDERER=cairo cargo test --locked -- \
//       --test-threads=1 --exact window::immersive_window_tests::<name> --ignored

const COMPACT_SIDEBAR_MODE_SETTING: &str = "compact-sidebar-enabled";

fn settle() {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(700);
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
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

fn main_split(widget: &gtk::Widget) -> Option<adw::OverlaySplitView> {
    descendants(widget)
        .into_iter()
        .find_map(|widget| widget.downcast::<adw::OverlaySplitView>().ok())
}

fn compact_dock(widget: &gtk::Widget) -> Option<gtk::Widget> {
    descendants(widget)
        .into_iter()
        .find(|widget| widget.has_css_class("compact-sidebar-dock"))
}

fn immersive_window(compact: bool, layout: &str) -> (adw::ApplicationWindow, std::path::PathBuf) {
    // GTK can only be initialized once per process, and the test harness
    // runs every test on its own thread.
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        adw::init().unwrap();
    });
    // Every window needs its own application id: an application can only
    // export its bus name once per process.
    static SEQUENCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let app = adw::Application::builder()
        .application_id(format!("io.pic.ImmersiveSidebarTest{sequence}"))
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let directory = std::env::temp_dir().join(format!(
        "pic-immersive-test-{}-{compact}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("library.db");
    let connection = db::open(&path).unwrap();
    for (key, value) in [
        (LAST_VIEW_SETTING_KEY, "all"),
        ("onboarding-never-show", "true"),
        (
            COMPACT_SIDEBAR_MODE_SETTING,
            if compact { "true" } else { "false" },
        ),
        ("photo_layout.all", layout),
        (WINDOW_WIDTH_SETTING_KEY, "1400"),
        (WINDOW_HEIGHT_SETTING_KEY, "900"),
    ] {
        db::set_setting(&connection, key, value).unwrap();
    }
    let window = build(&app, connection);
    window.present();
    settle();
    (window, path)
}

fn saved_sidebar_mode(path: &std::path::Path) -> Option<String> {
    let connection = db::open_existing(path).unwrap();
    db::setting(&connection, COMPACT_SIDEBAR_MODE_SETTING)
        .ok()
        .flatten()
}

fn enter_immersive(window: &adw::ApplicationWindow) {
    gio::prelude::ActionGroupExt::activate_action(window, "immersive", None);
    settle();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn immersive_mode_hides_the_compact_rail_and_restores_it() {
    let (window, database) = immersive_window(true, "grid");
    let root = window.upcast_ref::<gtk::Widget>();
    let split = main_split(root).unwrap();
    let dock = compact_dock(root).unwrap();

    // Cold start with the compact rail: the rail is visible and the wide
    // sidebar is not part of the layout.
    assert!(dock.is_visible(), "the compact rail must start visible");
    assert!(!split.shows_sidebar());

    let shell = descendants(root).into_iter()
        .find(|widget| widget.has_css_class("layout-left-column"))
        .unwrap().parent().unwrap();
    enter_immersive(&window);
    assert!(descendants(&shell).iter().any(|widget| widget.has_css_class("compact-sidebar")),
        "immersive sidebar must use the compact page on entry");
    let handle = descendants(root).into_iter()
        .find(|widget| widget.has_css_class("sidebar-resize-handle"))
        .expect("shared resize handle");
    assert!(handle.is_visible(), "compact immersive sidebar needs its divider");

    // Immersive Mode hides every piece of left navigation.
    assert!(!dock.is_visible(), "the compact rail must be hidden");
    assert!(!split.shows_sidebar(), "the sidebar must be hidden");
    assert!(
        split.is_collapsed(),
        "the sidebar must stay an overlay that cannot resize the gallery"
    );

    // The saved Compact UI preference is not touched by the session.
    assert_eq!(
        saved_sidebar_mode(&database).as_deref(),
        Some("true"),
        "immersive mode must not rewrite the saved sidebar mode"
    );

    // Leaving the session restores the exact previous configuration.
    enter_immersive(&window);
    assert!(dock.is_visible(), "the compact rail must come back");
    assert!(!handle.is_visible(), "windowed compact rail hides the divider");
    assert!(!split.shows_sidebar());
    assert_eq!(saved_sidebar_mode(&database).as_deref(), Some("true"));

    // Re-entering starts from the hidden state again.
    enter_immersive(&window);
    assert!(!dock.is_visible());
    assert!(!split.shows_sidebar());
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn immersive_compact_switch_reuses_overlay_and_restores_compact_page() {
    let (window, database) = immersive_window(false, "grid");
    let root = window.clone().upcast::<gtk::Widget>();
    let split = main_split(&root).unwrap();
    let dock = compact_dock(&root).unwrap();
    let column = descendants(&root)
        .into_iter().find(|w| w.has_css_class("layout-left-column")).unwrap();
    let shell = column.parent().unwrap().downcast::<gtk::Overlay>().unwrap();

    enter_immersive(&window);
    let gallery_width = split.content().unwrap().width();
    gio::prelude::ActionGroupExt::activate_action(&window, "immersive-show-sidebar", None);
    settle();
    let full_toggle = descendants(&root).into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .find(|b| b.tooltip_text().as_deref() == Some("Toggle compact sidebar"))
        .expect("normal sidebar compact button");
    full_toggle.emit_clicked();
    settle();

    assert!(!dock.is_visible(), "the separate windowed compact dock stays hidden");
    assert!(shell.is_visible(), "the immersive sidebar remains open");
    assert!(descendants(&shell.clone().upcast::<gtk::Widget>())
        .iter().any(|w| w.has_css_class("compact-sidebar")),
        "the original compact page must be inside the immersive overlay");
    assert_eq!(split.content().unwrap().width(), gallery_width);
    assert_eq!(saved_sidebar_mode(&database).as_deref(), Some("true"));

    let expand = descendants(&shell.clone().upcast::<gtk::Widget>())
        .into_iter().filter_map(|w| w.downcast::<gtk::Button>().ok())
        .find(|b| b.tooltip_text().as_deref() == Some("Switch to normal sidebar"))
        .expect("immersive compact expand button");
    expand.emit_clicked();
    settle();
    assert!(descendants(&shell.clone().upcast::<gtk::Widget>())
        .iter().any(|w| w.has_css_class("layout-left-column")));
    assert!(!dock.is_visible());
    assert_eq!(split.content().unwrap().width(), gallery_width);
    assert_eq!(saved_sidebar_mode(&database).as_deref(), Some("false"));

    full_toggle.emit_clicked();
    settle();
    enter_immersive(&window);
    assert!(dock.is_visible(), "compact dock returns when immersive exits");
    assert!(descendants(&dock).iter().any(|w| w.has_css_class("compact-sidebar")),
        "the same compact page must be restored to its original dock");
    assert_eq!(saved_sidebar_mode(&database).as_deref(), Some("true"));
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn immersive_mode_hides_the_normal_sidebar_without_resizing_the_gallery() {
    let (window, database) = immersive_window(false, "grid");
    let root = window.upcast_ref::<gtk::Widget>();
    let split = main_split(root).unwrap();

    // Cold start with the normal sidebar: it is visible and the gallery
    // only gets the remaining width.
    assert!(split.shows_sidebar(), "the sidebar must start visible");
    let gallery_width = split.content().unwrap().width();
    assert!(gallery_width < split.width());

    enter_immersive(&window);

    assert!(!split.shows_sidebar(), "the sidebar must be hidden");
    assert!(split.is_collapsed());
    let immersive_width = split.content().unwrap().width();
    assert!(
        immersive_width > gallery_width,
        "the gallery must take the full width while the sidebar is hidden"
    );

    // The saved Compact UI preference is not touched by the session.
    assert_eq!(saved_sidebar_mode(&database).as_deref(), Some("false"));

    // Leaving the session restores the sidebar and the gallery width.
    enter_immersive(&window);
    assert!(split.shows_sidebar(), "the sidebar must come back");
    assert!(!split.is_collapsed());
    let restored_width = split.content().unwrap().width();
    assert!(
        (restored_width - gallery_width).abs() < 3,
        "the gallery width must be restored: before={gallery_width} after={restored_width}"
    );
    window.close();
}

fn left_column(widget: &gtk::Widget) -> Option<gtk::Widget> {
    descendants(widget)
        .into_iter()
        .find(|widget| widget.has_css_class("layout-left-column"))
}

fn pin_button(widget: &gtk::Widget) -> Option<gtk::Button> {
    descendants(widget)
        .into_iter()
        .find(|widget| widget.has_css_class("sidebar-toggle-button"))
        .and_then(|widget| widget.downcast::<gtk::Button>().ok())
}

fn grid_scroll(widget: &gtk::Widget) -> Option<gtk::ScrolledWindow> {
    descendants(widget)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::ScrolledWindow>().ok())
        .find(|scroll| {
            scroll
                .child()
                .is_some_and(|child| child.has_css_class("photo-grid"))
        })
}

fn pump_for(duration: Duration) {
    let context = glib::MainContext::default();
    let until = Instant::now() + duration;
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ViewportSample {
    width: i32,
    value: f64,
    upper: f64,
    page: f64,
}

fn viewport_sample(content: &gtk::Widget, scroll: &gtk::ScrolledWindow) -> ViewportSample {
    let adjustment = scroll.vadjustment();
    ViewportSample {
        width: content.width(),
        value: adjustment.value(),
        upper: adjustment.upper(),
        page: adjustment.page_size(),
    }
}

fn reveal_sidebar(window: &adw::ApplicationWindow) {
    gio::prelude::ActionGroupExt::activate_action(window, "immersive-show-sidebar", None);
}

fn hide_sidebar(window: &adw::ApplicationWindow) {
    gio::prelude::ActionGroupExt::activate_action(window, "immersive-hide-sidebar", None);
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn pin_button_exits_immersive_and_restores_the_saved_pin_state() {
    let (window, _database) = immersive_window(false, "grid");
    let root = window.clone().upcast::<gtk::Widget>();
    let split = main_split(&root).unwrap();

    assert!(
        split.shows_sidebar(),
        "the normal layout starts pinned open"
    );
    enter_immersive(&window);
    assert!(!split.shows_sidebar(), "immersive hides the split sidebar");
    assert!(split.is_collapsed());

    let pin = pin_button(&root).expect("the sidebar pin button");
    assert!(
        pin.is_visible(),
        "the pin button must be reachable in immersive"
    );
    pin.emit_clicked();
    settle();

    assert!(!split.is_collapsed(), "the pin must leave immersive mode");
    assert!(
        split.shows_sidebar(),
        "exiting must restore the saved open/pinned sidebar, not toggle it"
    );
    assert!(left_column(&root).unwrap().is_visible());

    // Outside immersive mode the button keeps its normal pin/unpin behaviour.
    pin.emit_clicked();
    settle();
    assert!(
        !split.shows_sidebar(),
        "the pin must still hide a pinned sidebar in the windowed layout"
    );
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn pin_button_exits_immersive_from_the_compact_layout() {
    let (window, database) = immersive_window(true, "grid");
    let root = window.clone().upcast::<gtk::Widget>();
    let split = main_split(&root).unwrap();
    let dock = compact_dock(&root).unwrap();

    assert!(dock.is_visible(), "the compact rail starts visible");
    enter_immersive(&window);
    assert!(!dock.is_visible(), "immersive hides the compact rail");

    pin_button(&root)
        .expect("the sidebar pin button")
        .emit_clicked();
    settle();

    assert!(
        dock.is_visible(),
        "the compact rail must return after the pin exits immersive"
    );
    assert!(!split.is_collapsed());
    assert!(!split.shows_sidebar());
    assert_eq!(saved_sidebar_mode(&database).as_deref(), Some("true"));
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn immersive_sidebar_slides_without_moving_the_gallery() {
    for layout in ["grid", "photo_wall", "masonry", "honeycomb"] {
        let (window, _database) = immersive_window(false, layout);
        // Exercise the real slide; a disabled animation setting would make
        // AdwAnimation jump straight to the final value.
        gtk::Settings::default()
            .expect("a display is required")
            .set_gtk_enable_animations(true);
        let root = window.clone().upcast::<gtk::Widget>();
        let split = main_split(&root).unwrap();
        let column = left_column(&root).expect("the immersive sidebar column");
        let shell = column.parent().expect("the sidebar presentation shell");
        let content = split.content().unwrap();
        let scroll = grid_scroll(&content).expect("the gallery scroll");
        let frame = shell.parent().expect("the gallery overlay");

        enter_immersive(&window);
        assert!(
            !split.shows_sidebar(),
            "{layout}: the sidebar must start hidden"
        );
        let baseline = viewport_sample(&content, &scroll);

        let mut slides = Vec::new();
        for cycle in 0..2 {
            reveal_sidebar(&window);
            let started = Instant::now();
            let mut min_x = f64::MAX;
            let mut positions = std::collections::BTreeSet::new();
            let mut drifted = Vec::new();
            while started.elapsed() < Duration::from_millis(750) {
                pump_for(Duration::from_millis(10));
                if let Some(bounds) = shell.compute_bounds(&frame) {
                    let x = f64::from(bounds.x());
                    min_x = min_x.min(x);
                    positions.insert(x.round() as i32);
                }
                let sample = viewport_sample(&content, &scroll);
                if sample != baseline {
                    drifted.push(sample);
                }
            }
            assert!(
                drifted.is_empty(),
                "{layout} cycle {cycle}: the reveal moved the gallery viewport: {drifted:?}"
            );
            assert!(
                shell.is_visible(),
                "{layout} cycle {cycle}: the revealed sidebar must stay visible"
            );
            let final_x = shell
                .compute_bounds(&frame)
                .map(|bounds| f64::from(bounds.x()))
                .unwrap_or(f64::NEG_INFINITY);
            assert!(
                final_x.abs() < 2.0,
                "{layout} cycle {cycle}: the sidebar must settle flush left, x={final_x}"
            );
            assert!(
                positions.len() > 1,
                "{layout} cycle {cycle}: the sidebar must animate, positions={positions:?}"
            );
            assert!(
                min_x < -2.0,
                "{layout} cycle {cycle}: the sidebar must start off-screen, min_x={min_x}"
            );
            slides.push((min_x, final_x, positions.len()));

            hide_sidebar(&window);
            pump_for(Duration::from_millis(750));
            assert!(
                !shell.is_visible(),
                "{layout} cycle {cycle}: the hidden sidebar must stop being allocated"
            );
            let after = viewport_sample(&content, &scroll);
            assert_eq!(
                after, baseline,
                "{layout} cycle {cycle}: hiding the sidebar moved the gallery viewport"
            );
        }
        println!("{layout}: slides={slides:?}");
        window.close();
    }
}

#[test]
#[ignore = "requires GTK display; run with --ignored --test-threads=1"]
fn immersive_search_centers_selected_folder() {
    let (window, database) = immersive_window(false, "grid");
    let directory = database.parent().unwrap();
    let image_path = directory.join("sample.png");
    image::RgbImage::from_pixel(32, 24, image::Rgb([80, 140, 210]))
        .save(&image_path)
        .unwrap();
    let connection = db::open_existing(&database).unwrap();
    for id in 1..=80 {
        let name = if id == 75 {
            "Unique Search Destination".to_string()
        } else {
            format!("Folder {id:03}")
        };
        let folder = directory.join(&name);
        std::fs::create_dir_all(&folder).unwrap();
        let photo = folder.join("photo.png");
        std::fs::hard_link(&image_path, &photo).unwrap();
        connection
            .execute(
                "INSERT INTO folders(id,path,name) VALUES (?1,?2,?3)",
                rusqlite::params![id, folder.to_str().unwrap(), name],
            )
            .unwrap();
        connection.execute("INSERT INTO photos(id,path,folder_id,width,height,added_at) VALUES (?1,?2,?1,32,24,?1)", rusqlite::params![id, photo.to_str().unwrap()]).unwrap();
    }
    drop(connection);
    enter_immersive(&window);
    let root = window.clone().upcast::<gtk::Widget>();
    let search = descendants(&root)
        .into_iter()
        .find(|w| w.has_css_class("unified-search-entry"))
        .unwrap()
        .downcast::<gtk::SearchEntry>()
        .unwrap();
    let revealer = std::iter::successors(search.parent(), |w| w.parent())
        .find_map(|w| w.downcast::<gtk::Revealer>().ok())
        .unwrap();
    revealer.set_reveal_child(true);
    settle();
    assert!(search.grab_focus());
    search.set_text("Unique Search Destination");
    settle();
    search.emit_by_name::<()>("activate", &[]);
    pump_for(Duration::from_secs(3));
    let headings = descendants(&root)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Label>().ok())
        .filter(|label| label.text().contains("Unique Search Destination") && label.is_mapped())
        .collect::<Vec<_>>();
    let heading_visible = headings.iter().any(|label| {
        label
            .compute_bounds(&root)
            .is_some_and(|b| b.y() >= 0.0 && b.y() < root.height() as f32)
    });
    assert!(
        heading_visible,
        "selected folder heading should be in the viewport"
    );
    gio::prelude::ActionGroupExt::activate_action(&window, "immersive-show-sidebar", None);
    settle();
    let surface = descendants(&root).into_iter().find(|widget| widget.has_css_class("sectioned-folder-view") && widget.is_mapped()).unwrap();
    let scroll = std::iter::successors(surface.parent(), |widget| widget.parent()).find_map(|widget| widget.downcast::<gtk::ScrolledWindow>().ok()).expect("gallery scroll");
    let destination = scroll.vadjustment().value();
    assert!(destination > 0.0, "the destination must be away from the top");
    scroll.vadjustment().set_value(destination / 2.0);
    settle();
    let baseline = scroll.vadjustment().value();
    let compact = descendants(&root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| button.tooltip_text().as_deref() == Some("Toggle compact sidebar"))
        .unwrap();
    assert!(compact.grab_focus());
    gio::prelude::ActionGroupExt::activate_action(&window, "immersive-hide-sidebar", None);
    settle();
    assert_eq!(scroll.vadjustment().value(), baseline, "hiding the sidebar moved the gallery");
    gio::prelude::ActionGroupExt::activate_action(&window, "immersive-show-sidebar", None);
    settle();
    assert!(compact.grab_focus());
    compact.emit_clicked();
    settle();
    assert_eq!(scroll.vadjustment().value(), baseline, "compacting moved the gallery");
    let expand = descendants(&root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| button.tooltip_text().as_deref() == Some("Switch to normal sidebar"))
        .unwrap();
    assert!(expand.grab_focus());
    expand.emit_clicked();
    settle();
    assert_eq!(scroll.vadjustment().value(), baseline, "expanding moved the gallery");
    window.close();
}
