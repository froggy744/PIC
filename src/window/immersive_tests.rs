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

fn immersive_window(compact: bool) -> (adw::ApplicationWindow, std::path::PathBuf) {
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
        "pic-immersive-test-{}-{compact}",
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
    let (window, database) = immersive_window(true);
    let root = window.upcast_ref::<gtk::Widget>();
    let split = main_split(root).unwrap();
    let dock = compact_dock(root).unwrap();

    // Cold start with the compact rail: the rail is visible and the wide
    // sidebar is not part of the layout.
    assert!(dock.is_visible(), "the compact rail must start visible");
    assert!(!split.shows_sidebar());

    enter_immersive(&window);

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
fn immersive_mode_hides_the_normal_sidebar_without_resizing_the_gallery() {
    let (window, database) = immersive_window(false);
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
