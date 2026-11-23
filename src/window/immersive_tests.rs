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
    // `is_visible()` also checks the parents, and the shell stays hidden
    // until the overlay is revealed. The divider's own flag is what matters.
    assert!(handle.get_visible(), "compact immersive sidebar needs its divider");

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
    assert!(!handle.get_visible(), "windowed compact rail hides the divider");
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
    settle_over_sidebar(&window);
    let full_toggle = descendants(&root).into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .find(|b| b.tooltip_text().as_deref() == Some("Toggle compact sidebar"))
        .expect("normal sidebar compact button");
    full_toggle.emit_clicked();
    settle_over_sidebar(&window);

    assert!(!dock.is_visible(), "the separate windowed compact dock stays hidden");
    assert!(shell.is_visible(), "the immersive sidebar remains open");
    assert!(descendants(&shell.clone().upcast::<gtk::Widget>())
        .iter().any(|w| w.has_css_class("compact-sidebar")),
        "the original compact page must be inside the immersive overlay");
    assert_eq!(split.content().unwrap().width(), gallery_width);
    assert_eq!(saved_sidebar_mode(&database).as_deref(), Some("true"));
    // The compact header holds the arrow and the title; both stay inside
    // the overlay instead of painting past its right edge.
    let shell_widget = shell.clone().upcast::<gtk::Widget>();
    for widget in descendants(&shell_widget).into_iter().filter(|widget| {
        widget.is_mapped()
            && (widget.downcast_ref::<gtk::Label>().is_some_and(|label| label.text() == "PIC")
                || widget.downcast_ref::<gtk::Button>().is_some_and(|button| {
                    button.tooltip_text().as_deref() == Some("Switch to normal sidebar")
                }))
    }) {
        let bounds = widget.compute_bounds(&shell_widget).expect("header bounds");
        assert!(
            bounds.x() >= -0.5 && bounds.x() + bounds.width() <= shell.width() as f32 + 0.5,
            "compact header item outside the overlay: x={} w={} shell={}",
            bounds.x(),
            bounds.width(),
            shell.width()
        );
    }

    let expand = descendants(&shell.clone().upcast::<gtk::Widget>())
        .into_iter().filter_map(|w| w.downcast::<gtk::Button>().ok())
        .find(|b| b.tooltip_text().as_deref() == Some("Switch to normal sidebar"))
        .expect("immersive compact expand button");
    expand.emit_clicked();
    settle_over_sidebar(&window);
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

/// Tell the immersive edge controller the pointer rests over the revealed
/// sidebar. Under Xvfb the real pointer sits still somewhere over the gallery
/// and GTK keeps reporting it there, so the (correct) auto-hide would close
/// the overlay 400 ms after every reveal.
fn hold_pointer_over_sidebar(window: &adw::ApplicationWindow) {
    let controllers = window.observe_controllers();
    for index in 0..controllers.n_items() {
        if let Some(motion) = controllers
            .item(index)
            .and_downcast::<gtk::EventControllerMotion>()
        {
            motion.emit_by_name::<()>("motion", &[&5.0_f64, &200.0_f64]);
        }
    }
}

/// [`settle`], with the pointer held over the revealed immersive sidebar.
fn settle_over_sidebar(window: &adw::ApplicationWindow) {
    let until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < until {
        hold_pointer_over_sidebar(window);
        pump_for(Duration::from_millis(10));
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

    // Immersive auto-hides the sidebar; the pin is reachable once the
    // overlay is revealed.
    reveal_sidebar(&window);
    settle_over_sidebar(&window);
    let pin = pin_button(&root).expect("the sidebar pin button");
    assert!(
        pin.is_visible(),
        "the pin button must be reachable in the revealed immersive sidebar"
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
        enter_immersive(&window);
        // Entry moves the shell from the split onto the gallery overlay.
        let frame = shell.parent().expect("the gallery overlay");
        assert!(
            !split.shows_sidebar(),
            "{layout}: the sidebar must start hidden"
        );
        let baseline = viewport_sample(&content, &scroll);

        let mut slides = Vec::new();
        // Honeycomb once drifted a little further on every cycle; run it
        // long enough to show progressive drift.
        let cycles = if layout == "honeycomb" { 20 } else { 2 };
        for cycle in 0..cycles {
            reveal_sidebar(&window);
            let started = Instant::now();
            let mut min_x = f64::MAX;
            let mut positions = std::collections::BTreeSet::new();
            let mut drifted = Vec::new();
            while started.elapsed() < Duration::from_millis(750) {
                hold_pointer_over_sidebar(&window);
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

/// One frame-by-frame sample of the windowed sidebar: its x inside the split
/// (while it is mapped) and the gallery's width.
#[derive(Debug, Clone, Copy)]
struct SlideSample {
    sidebar_x: Option<i32>,
    content_width: i32,
}

/// Pump frames for `duration`, sampling after every main-loop turn.
fn sample_slide(split: &adw::OverlaySplitView, duration: Duration) -> Vec<SlideSample> {
    let context = glib::MainContext::default();
    let until = Instant::now() + duration;
    let mut samples = Vec::new();
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        let sidebar = split.sidebar().unwrap();
        samples.push(SlideSample {
            sidebar_x: sidebar
                .is_mapped()
                .then(|| sidebar.compute_bounds(split))
                .flatten()
                .map(|bounds| bounds.x().round() as i32),
            content_width: split.content().unwrap().width(),
        });
        std::thread::sleep(Duration::from_millis(4));
    }
    samples
}

/// True when some sample sits strictly between the first and last state:
/// the sidebar moved through an intermediate position instead of cutting.
fn slid(samples: &[SlideSample], width: i32) -> bool {
    let partial_x = samples
        .iter()
        .filter_map(|sample| sample.sidebar_x)
        .any(|x| x < -2 && x > -(width - 2));
    let first = samples.first().map_or(0, |sample| sample.content_width);
    let last = samples.last().map_or(0, |sample| sample.content_width);
    let (low, high) = (first.min(last), first.max(last));
    let partial_content = high - low > 4
        && samples
            .iter()
            .any(|sample| sample.content_width > low + 2 && sample.content_width < high - 2);
    partial_x || partial_content
}

fn hover_strip(root: &gtk::Widget) -> gtk::Widget {
    descendants(root)
        .into_iter()
        .find(|widget| {
            widget.width_request() == 12
                && widget.parent().is_some_and(|parent| parent.is::<gtk::Overlay>())
                && motion_controller(widget).is_some()
        })
        .expect("the left-edge hover strip")
}

fn motion_controller(widget: &gtk::Widget) -> Option<gtk::EventControllerMotion> {
    let controllers = widget.observe_controllers();
    (0..controllers.n_items())
        .find_map(|index| controllers.item(index).and_downcast::<gtk::EventControllerMotion>())
}

fn button_with_tooltip(root: &gtk::Widget, tooltip: &str) -> gtk::Button {
    descendants(root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| button.is_visible() && button.tooltip_text().as_deref() == Some(tooltip))
        .unwrap_or_else(|| panic!("visible button {tooltip:?}"))
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn windowed_sidebar_slides_for_pin_hover_peek_and_breakpoint() {
    let (window, _database) = immersive_window(false, "grid");
    // A disabled animation setting makes libadwaita jump to the end.
    gtk::Settings::default()
        .expect("a display is required")
        .set_gtk_enable_animations(true);
    let root = window.clone().upcast::<gtk::Widget>();
    let split = main_split(&root).unwrap();
    assert!(split.shows_sidebar() && !split.is_collapsed());
    let width = split.sidebar().unwrap().width();
    assert!(width > 100, "sidebar width {width}");

    // Unpin: a real slide-out.
    pin_button(&root).unwrap().emit_clicked();
    let samples = sample_slide(&split, Duration::from_millis(700));
    assert!(!split.shows_sidebar(), "unpin hides the sidebar");
    assert!(slid(&samples, width), "unpin must slide out: {samples:?}");

    // Hover peek opens with a slide and closes with one after leaving.
    let strip = hover_strip(&root);
    motion_controller(&strip)
        .unwrap()
        .emit_by_name::<()>("enter", &[&4.0_f64, &300.0_f64]);
    let samples = sample_slide(&split, Duration::from_millis(700));
    assert!(split.shows_sidebar(), "hover peeks the unpinned sidebar");
    assert!(slid(&samples, width), "hover peek must slide in: {samples:?}");
    let shell = split.sidebar().unwrap();
    motion_controller(&shell)
        .expect("the sidebar's leave controller")
        .emit_by_name::<()>("leave", &[]);
    let samples = sample_slide(&split, Duration::from_millis(900));
    assert!(!split.shows_sidebar(), "leaving hides the peek");
    assert!(slid(&samples, width), "the peek must slide out: {samples:?}");

    // The header arrow pins it open again with a slide.
    button_with_tooltip(&root, "Show sidebar").emit_clicked();
    let samples = sample_slide(&split, Duration::from_millis(700));
    assert!(split.shows_sidebar(), "show pins the sidebar open");
    assert!(slid(&samples, width), "show must slide in: {samples:?}");

    // Narrowing past the breakpoint turns it into a drawer that slides out.
    window.set_default_size(960, 800);
    let samples = sample_slide(&split, Duration::from_millis(900));
    assert!(split.is_collapsed() && !split.shows_sidebar(), "narrow hides the drawer");
    assert!(
        samples.iter().filter_map(|s| s.sidebar_x).any(|x| x < -2 && x > -(split.sidebar().unwrap().width() - 2)),
        "the drawer must slide out: {samples:?}"
    );
    // Widening expands while hidden, then slides the pinned sidebar back in.
    window.set_default_size(1600, 900);
    let samples = sample_slide(&split, Duration::from_millis(900));
    assert!(!split.is_collapsed() && split.shows_sidebar(), "wide restores the pinned sidebar");
    assert!(slid(&samples, width), "widening must slide in: {samples:?}");
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn compact_start_keeps_the_pin_when_switching_to_normal() {
    let (window, _database) = immersive_window(true, "grid");
    let root = window.clone().upcast::<gtk::Widget>();
    let split = main_split(&root).unwrap();
    assert!(!split.shows_sidebar(), "the compact rail starts in place");
    button_with_tooltip(&root, "Switch to normal sidebar").emit_clicked();
    settle();
    assert!(split.shows_sidebar(), "normal mode opens the sidebar");
    assert!(
        crate::sidebar::is_pinned(&left_column(&root)
            .and_then(|column| descendants(&column).into_iter()
                .find_map(|widget| widget.downcast::<gtk::ScrolledWindow>().ok()))
            .expect("the sidebar scroller")),
        "the sidebar must open pinned, not as a hover peek"
    );
    // Leaving the sidebar must not auto-hide a pinned sidebar.
    motion_controller(&split.sidebar().unwrap())
        .unwrap()
        .emit_by_name::<()>("leave", &[]);
    settle();
    assert!(split.shows_sidebar(), "a pinned sidebar never auto-hides");
    window.close();
}

// The Properties inspector against Immersive Mode, the viewer and page
// navigation, driven through the real window. Immersive Mode suspends the
// inspector: it must stay hidden for the whole session without any
// transient show/hide, and come back afterwards on the current selection.
fn properties_window(photo_count: i64) -> adw::ApplicationWindow {
    let (window, path) = {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            adw::init().unwrap();
        });
        let app = adw::Application::builder()
            .application_id("io.pic.ImmersivePropertiesTest")
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let directory = std::env::temp_dir()
            .join(format!("pic-immersive-properties-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("library.db");
        let mut connection = db::open(&path).unwrap();
        for (key, value) in [
            (LAST_VIEW_SETTING_KEY, "all"),
            ("onboarding-never-show", "true"),
            (COMPACT_SIDEBAR_MODE_SETTING, "false"),
            ("photo_layout.all", "grid"),
            (WINDOW_WIDTH_SETTING_KEY, "1400"),
            (WINDOW_HEIGHT_SETTING_KEY, "900"),
        ] {
            db::set_setting(&connection, key, value).unwrap();
        }
        connection
            .execute(
                "INSERT INTO folders(id,path,name) VALUES (1,?1,'Properties test')",
                [directory.to_str().unwrap()],
            )
            .unwrap();
        let image_path = directory.join("sample.jpg");
        image::RgbImage::from_pixel(32, 24, image::Rgb([80, 140, 210]))
            .save(&image_path)
            .unwrap();
        let transaction = connection.transaction().unwrap();
        for id in 1..=photo_count {
            let photo_path = directory.join(format!("photo-{id}.jpg"));
            let _ = std::fs::remove_file(&photo_path);
            std::fs::hard_link(&image_path, &photo_path).unwrap();
            transaction
                .execute(
                    "INSERT INTO photos(id,path,folder_id,width,height,added_at) VALUES (?1,?2,1,32,24,?1)",
                    rusqlite::params![id, photo_path.to_string_lossy()],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        let window = build(&app, connection);
        window.present();
        settle();
        (window, path)
    };
    let _ = path;
    window
}

fn properties_split(root: &gtk::Widget) -> adw::OverlaySplitView {
    descendants(root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<adw::OverlaySplitView>().ok())
        .find(|split| split.sidebar_position() == gtk::PackType::End)
        .expect("Properties split missing")
}

fn properties_button(root: &gtk::Widget) -> gtk::Button {
    descendants(root)
        .into_iter()
        .filter(|widget| widget.has_css_class("properties-toggle"))
        .find_map(|widget| widget.downcast::<gtk::Button>().ok())
        .expect("InfoBar Properties button missing")
}

fn photo_grid(root: &gtk::Widget) -> gtk::GridView {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let grid = descendants(root)
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::GridView>().ok())
            .find(|grid| grid.is_mapped() && grid.model().is_some_and(|model| model.n_items() > 0));
        if let Some(grid) = grid {
            return grid;
        }
        assert!(Instant::now() < deadline, "All Photos grid did not load");
        settle();
    }
}

/// Selects a grid position like a click and returns its file name.
fn select_grid_photo(grid: &gtk::GridView, position: u32) -> String {
    let model = grid.model().unwrap();
    model.select_item(position, true);
    settle();
    model
        .item(position)
        .and_downcast::<crate::photo_object::PhotoObject>()
        .expect("grid item is a photo")
        .filename()
}

/// The viewer's inspector panel: the right-aligned 300 px overlay that
/// shares the "scrapbook-inspector" class with the docked panel.
fn lightbox_properties_panel(root: &gtk::Widget, split: &adw::OverlaySplitView) -> gtk::Widget {
    let docked = split.sidebar().unwrap();
    descendants(root)
        .into_iter()
        .filter(|widget| widget.has_css_class("scrapbook-inspector"))
        .find(|widget| {
            widget.width_request() == 300
                && widget.halign() == gtk::Align::End
                && !widget.is_ancestor(&docked)
                && widget != &docked
        })
        .expect("viewer Properties panel missing")
}

fn shows_file_name(panel: &gtk::Widget, name: &str) -> bool {
    descendants(panel)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .any(|label| label.label() == name)
}

fn lightbox_root(root: &gtk::Widget) -> gtk::Widget {
    let backdrop = descendants(root)
        .into_iter()
        .find(|widget| widget.has_css_class("lightbox-backdrop"))
        .expect("lightbox missing");
    let mut ancestor = backdrop.parent();
    while let Some(widget) = ancestor {
        let controllers = widget.observe_controllers();
        if (0..controllers.n_items()).any(|index| {
            controllers.item(index).and_downcast::<gtk::EventControllerKey>().is_some()
        }) && widget.is::<gtk::Overlay>()
        {
            return widget;
        }
        ancestor = widget.parent();
    }
    panic!("lightbox root missing")
}

fn close_lightbox(lightbox: &gtk::Widget) {
    let controllers = lightbox.observe_controllers();
    let key = (0..controllers.n_items())
        .find_map(|index| controllers.item(index).and_downcast::<gtk::EventControllerKey>())
        .unwrap();
    let _: bool = key.emit_by_name(
        "key-pressed",
        &[&gtk::glib::translate::IntoGlib::into_glib(gtk::gdk::Key::Escape), &0_u32, &gtk::gdk::ModifierType::empty()],
    );
    settle();
    settle();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn immersive_suspends_properties_without_transient_visibility() {
    let window = properties_window(24);
    let root = window.clone().upcast::<gtk::Widget>();
    let split = properties_split(&root);
    let button = properties_button(&root);
    let grid = photo_grid(&root);
    let viewer_panel = lightbox_properties_panel(&root, &split);
    let docked = split.sidebar().unwrap();

    // Every show/hide of either panel, as (panel, shown).
    let transitions: Rc<RefCell<Vec<(&'static str, bool)>>> = Rc::default();
    {
        let gallery = transitions.clone();
        split.connect_show_sidebar_notify(move |split| {
            gallery.borrow_mut().push(("gallery", split.shows_sidebar()));
        });
        let transitions = transitions.clone();
        viewer_panel.connect_visible_notify(move |panel| {
            transitions.borrow_mut().push(("viewer", panel.is_visible()));
        });
    }

    let first = select_grid_photo(&grid, 0);
    button.emit_clicked();
    settle();
    assert!(split.shows_sidebar(), "the InfoBar button opens the gallery inspector");
    assert!(button.has_css_class("properties-active"));
    assert!(shows_file_name(&docked, &first));

    for session in 0..3 {
        transitions.borrow_mut().clear();
        enter_immersive(&window);
        assert_eq!(transitions.borrow().as_slice(), &[("gallery", false)], "one hide on entry");
        assert!(!split.shows_sidebar(), "Immersive Mode hides the gallery inspector");
        assert!(!button.is_sensitive(), "the InfoBar button is disabled in Immersive Mode");
        transitions.borrow_mut().clear();
        let mut selected = String::new();
        for position in 1..=3 {
            selected = select_grid_photo(&grid, position + session * 3);
        }
        assert_eq!(
            transitions.borrow().as_slice(),
            &[],
            "selection changes in Immersive Mode must not show or hide an inspector (session {session})"
        );
        enter_immersive(&window);
        assert_eq!(transitions.borrow().as_slice(), &[("gallery", true)], "one show on exit");
        assert!(split.shows_sidebar(), "leaving Immersive Mode resumes the inspector (session {session})");
        assert!(button.is_sensitive() && button.has_css_class("properties-active"));
        assert!(
            shows_file_name(&docked, &selected),
            "the resumed inspector shows the current selection {selected} (session {session})"
        );
    }

    // The viewer has its own inspector; closing the viewer forgets only it.
    for cycle in 0..2 {
        grid.emit_by_name::<()>("activate", &[&2_u32]);
        settle();
        settle();
        let lightbox = lightbox_root(&root);
        assert!(lightbox.is_visible(), "the viewer opened (cycle {cycle})");
        assert!(!viewer_panel.is_visible(), "the viewer inspector starts closed (cycle {cycle})");
        assert!(!button.has_css_class("properties-active"), "the button follows the viewer inspector");
        button.emit_clicked();
        settle();
        assert!(viewer_panel.is_visible(), "the button opens the viewer inspector (cycle {cycle})");
        assert!(button.has_css_class("properties-active"));
        assert!(split.shows_sidebar(), "the gallery inspector keeps its own state");

        enter_immersive(&window);
        assert!(!viewer_panel.is_visible() && !split.shows_sidebar());
        transitions.borrow_mut().clear();
        enter_immersive(&window);
        assert!(viewer_panel.is_visible(), "leaving Immersive Mode resumes the viewer inspector");
        assert!(split.shows_sidebar());

        close_lightbox(&lightbox);
        assert!(!lightbox.is_visible(), "the viewer closed (cycle {cycle})");
        assert!(!viewer_panel.is_visible(), "closing the viewer hides its inspector");
        assert!(split.shows_sidebar(), "closing the viewer keeps the gallery inspector");
        assert!(button.has_css_class("properties-active"), "the button follows the gallery inspector");
    }

    // Leaving the Photos page closes the gallery inspector for good.
    let stack = descendants(&root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Stack>().ok())
        .find(|stack| stack.child_by_name("photos").is_some() && stack.child_by_name("albums").is_some())
        .expect("main stack missing");
    stack.set_visible_child_name("albums");
    settle();
    assert!(!split.shows_sidebar(), "an index page closes the inspector");
    stack.set_visible_child_name("photos");
    settle();
    assert!(!split.shows_sidebar(), "returning to Photos keeps the inspector closed");
    assert!(!button.has_css_class("properties-active"));
    transitions.borrow_mut().clear();
    enter_immersive(&window);
    enter_immersive(&window);
    assert_eq!(
        transitions.borrow().as_slice(),
        &[],
        "Immersive Mode never flashes a closed inspector"
    );
    assert!(!split.shows_sidebar(), "Immersive Mode does not reopen a closed inspector");
    button.emit_clicked();
    settle();
    assert!(split.shows_sidebar(), "the inspector reopens on request");
    window.close();
    settle();
}
