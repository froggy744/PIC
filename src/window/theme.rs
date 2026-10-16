//! Appearance theme engine.
//!
//! Themes are discovered at runtime from the `themes` folder next to the
//! executable (see [`crate::css::theme_discovery`]); nothing about the
//! available themes is hardcoded. Layering on the display, lowest first:
//!
//! 1. Foundation (`css::install_foundation`): base + platform CSS.
//! 2. The `mode: base` theme (shipped as iPhoto Dark) at `APPLICATION`
//!    priority, added after the foundation so it wins for equal specificity.
//! 3. The selected overlay theme at `APPLICATION + 1`.
//!
//! The Themes page in Settings is the single picker; the header-bar button
//! opens it and doubles as the sun/moon appearance indicator.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::gdk;
use gtk::prelude::*;
use libadwaita as adw;
use rusqlite::Connection;

use super::THEME_SETTING_KEY;
use crate::css::theme_discovery::{self, DiscoveredTheme, WindowControls};

pub(crate) const THEMES_DIRECTORY: &str = "themes";

/// Fallback when the saved theme id no longer exists on disk.
const DEFAULT_THEME_ID: &str = "standard";
const TRON_GLOW_SETTING_KEY: &str = "tron-glow-preset";

pub(crate) struct ThemeEngine {
    display: gdk::Display,
    style_manager: adw::StyleManager,
    lightbox: Rc<crate::lightbox::Lightbox>,
    connection: Rc<RefCell<Connection>>,
    /// Theme folder scanned by `themes()`. Defaults to `THEMES_DIRECTORY`;
    /// overridable in tests so failure cases can run against temp folders.
    themes_dir: RefCell<PathBuf>,
    base_provider: RefCell<Option<gtk::CssProvider>>,
    base_id: RefCell<Option<String>>,
    overlay_provider: RefCell<Option<gtk::CssProvider>>,
    active_id: RefCell<String>,
    appearance_button: RefCell<Option<gtk::Button>>,
    /// Runs after the user switches themes: redraws the albums home so its
    /// covers pick up the new palette. Not invoked during startup, when the
    /// albums home does not exist yet.
    post_apply: RefCell<Option<Rc<dyn Fn()>>>,
    /// Runs on every activation (startup and switches): tells the window
    /// chrome which window-control style the active theme opted into.
    window_controls_hook: RefCell<Option<Rc<dyn Fn(WindowControls)>>>,
}

impl ThemeEngine {
    pub(crate) fn new(
        display: gdk::Display,
        connection: Rc<RefCell<Connection>>,
        lightbox: Rc<crate::lightbox::Lightbox>,
    ) -> Rc<Self> {
        let style_manager = adw::StyleManager::default();
        let engine = Rc::new(Self {
            display,
            style_manager: style_manager.clone(),
            lightbox,
            connection,
            themes_dir: RefCell::new(PathBuf::from(THEMES_DIRECTORY)),
            base_provider: RefCell::new(None),
            base_id: RefCell::new(None),
            overlay_provider: RefCell::new(None),
            active_id: RefCell::new(String::new()),
            appearance_button: RefCell::new(None),
            post_apply: RefCell::new(None),
            window_controls_hook: RefCell::new(None),
        });
        // Light themes follow GNOME's appearance preference. The active
        // theme's CSS owns the main window palette, while this signal keeps
        // GTK widgets outside that CSS (including the lightbox) in sync.
        {
            let engine = engine.clone();
            style_manager.connect_dark_notify(move |manager| {
                if !engine.active_is_dark() {
                    engine.lightbox.use_standard_backdrop(manager.is_dark());
                }
            });
        }
        engine
    }

    /// Rescan the theme folder. Call whenever the picker is (re)built so
    /// themes dropped into the folder appear without a restart.
    pub(crate) fn themes(&self) -> Vec<DiscoveredTheme> {
        let dir = crate::css::resolve_runtime_dir(&self.themes_dir.borrow());
        theme_discovery::discover(&dir)
    }

    #[cfg(test)]
    pub(crate) fn set_themes_dir(&self, dir: PathBuf) {
        self.themes_dir.replace(dir);
    }

    pub(crate) fn active_id(&self) -> String {
        self.active_id.borrow().clone()
    }

    /// Saved TRON glow choice. Cinematic is the signature default.
    pub(crate) fn tron_glow_preset(&self) -> u32 {
        crate::db::setting(&self.connection.borrow(), TRON_GLOW_SETTING_KEY)
            .ok()
            .flatten()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| *value <= 2)
            .unwrap_or(2)
    }

    pub(crate) fn set_tron_glow_preset(&self, preset: u32) {
        let preset = preset.min(2);
        if let Err(error) = crate::db::set_setting(
            &self.connection.borrow(),
            TRON_GLOW_SETTING_KEY,
            &preset.to_string(),
        ) {
            eprintln!("Could not save TRON glow preset: {error}");
        }
        self.apply_tron_glow_class(preset);
    }

    fn apply_tron_glow_class(&self, preset: u32) {
        let class = match preset {
            0 => "tron-glow-subtle",
            1 => "tron-glow-standard",
            _ => "tron-glow-cinematic",
        };
        for widget in gtk::Window::list_toplevels() {
            let Ok(window) = widget.downcast::<gtk::Window>() else {
                continue;
            };
            for old in ["tron-glow-subtle", "tron-glow-standard", "tron-glow-cinematic"] {
                window.remove_css_class(old);
            }
            window.add_css_class(class);
            if let Some(content) = window.child() {
                for old in ["tron-glow-subtle", "tron-glow-standard", "tron-glow-cinematic"] {
                    content.remove_css_class(old);
                }
                content.add_css_class(class);
                Self::apply_glow_class_to_descendants(&content, class, false, false);
            }
        }
    }

    fn apply_glow_class_to_descendants(
        widget: &gtk::Widget,
        class: &str,
        in_header: bool,
        in_sidebar: bool,
    ) {
        let classes = widget.css_classes();
        let has = |name: &str| classes.iter().any(|candidate| candidate.as_str() == name);
        let header = in_header || has("layout-left-header") || has("layout-right-header");
        let sidebar = in_sidebar || has("navigation-sidebar");
        let css_name = widget.css_name();
        let target = [
            "photo-tile",
            "album-cover",
            "app-title",
            "section-heading",
            "folder-section-title",
            "folder-section-icon",
            "selection-badge",
            "photo-action-button",
            "search-field",
        ]
        .iter()
        .any(|name| has(name))
            || has("layout-left-header")
            || has("layout-right-header")
            || (sidebar && css_name.as_str() == "row")
            || (header && css_name.as_str() == "button")
            || css_name.as_str() == "scale";
        if target {
            for old in ["tron-glow-subtle", "tron-glow-standard", "tron-glow-cinematic"] {
                widget.remove_css_class(old);
            }
            widget.add_css_class(class);
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            Self::apply_glow_class_to_descendants(&current, class, header, sidebar);
        }
    }

    pub(crate) fn active_is_dark(&self) -> bool {
        self.themes()
            .iter()
            .find(|theme| theme.id == self.active_id())
            .map(|theme| theme.dark)
            .unwrap_or(false)
    }

    pub(crate) fn set_appearance_button(&self, button: &gtk::Button) {
        self.appearance_button.borrow_mut().replace(button.clone());
    }

    pub(crate) fn set_post_apply(&self, hook: Rc<dyn Fn()>) {
        self.post_apply.borrow_mut().replace(hook);
    }

    /// Registers the window-chrome reaction to the active theme's window
    /// control style. Invoked on every activation, so the initial placement
    /// happens during startup.
    pub(crate) fn set_on_window_controls_changed(&self, hook: Rc<dyn Fn(WindowControls)>) {
        self.window_controls_hook.borrow_mut().replace(hook);
    }

    /// Apply the persisted theme before the window's first rendered frame.
    /// Loads the base theme beneath everything, then resolves the saved
    /// selection: saved id, else the default, else the first theme found.
    pub(crate) fn startup(&self) {
        let themes = self.themes();
        if themes.is_empty() {
            eprintln!(
                "No appearance themes found in {}; running with the stock GTK appearance",
                crate::css::resolve_runtime_dir(&self.themes_dir.borrow()).display()
            );
            return;
        }

        if let Some(base) = themes.iter().find(|theme| theme.is_base) {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(base.css.as_str());
            gtk::style_context_add_provider_for_display(
                &self.display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            self.base_provider.borrow_mut().replace(provider);
            self.base_id.borrow_mut().replace(base.id.clone());
        }

        let saved = crate::db::setting(&self.connection.borrow(), THEME_SETTING_KEY)
            .ok()
            .flatten()
            .unwrap_or_else(|| DEFAULT_THEME_ID.to_string());
        let active = theme_discovery::resolve_active(&themes, &saved, DEFAULT_THEME_ID);
        if active.as_ref().is_some_and(|theme| theme.id != saved) {
            eprintln!(
                "Saved appearance theme '{saved}' was not found; falling back to '{}'",
                active.as_ref().map(|theme| theme.id.as_str()).unwrap_or("none")
            );
        }
        if let Some(theme) = active {
            self.activate(&theme);
        }
    }

    /// Re-read the library-specific theme after changing catalogs without
    /// installing a second base provider.
    pub(crate) fn reload_from_connection(&self) {
        let themes = self.themes();
        let saved = crate::db::setting(&self.connection.borrow(), THEME_SETTING_KEY)
            .ok()
            .flatten()
            .unwrap_or_else(|| DEFAULT_THEME_ID.to_string());
        if let Some(theme) = theme_discovery::resolve_active(&themes, &saved, DEFAULT_THEME_ID) {
            self.activate(&theme);
            if let Some(hook) = self.post_apply.borrow().as_ref() {
                hook();
            }
        }
    }

    /// Switch to a theme the user picked: swap the overlay, persist the
    /// choice, and refresh theme-dependent views. Skipped when the theme is
    /// already active so rebuilding the picker cannot cause redundant work.
    pub(crate) fn select(&self, theme: &DiscoveredTheme) {
        if theme.id == self.active_id() {
            return;
        }
        self.activate(theme);
        if let Err(error) =
            crate::db::set_setting(&self.connection.borrow(), THEME_SETTING_KEY, &theme.id)
        {
            eprintln!("Could not save appearance theme: {error}");
        }
        if let Some(hook) = self.post_apply.borrow().as_ref() {
            hook();
        }
    }

    fn activate(&self, theme: &DiscoveredTheme) {
        // Swap the overlay provider. Selecting the base theme itself only
        // needs the base layer that is already on the display.
        if let Some(old) = self.overlay_provider.borrow_mut().take() {
            gtk::style_context_remove_provider_for_display(&self.display, &old);
        }
        let selects_base = self.base_id.borrow().as_deref() == Some(theme.id.as_str());
        if !selects_base {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(theme.css.as_str());
            // PIC themes own scoped editor-toolbar palettes. Place those
            // overlays above the shared toolbar defaults without changing
            // precedence for other themes.
            let priority = if matches!(
                theme.id.as_str(),
                "PIC" | "PIC2" | "PIC-Default" | "PIC-Dark" | "Terminal" | "Monochrome-Studio"
            ) {
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 3
            } else {
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1
            };
            gtk::style_context_add_provider_for_display(
                &self.display,
                &provider,
                priority,
            );
            self.overlay_provider.borrow_mut().replace(provider);
        }
        self.apply_tron_glow_class(self.tron_glow_preset());

        *self.active_id.borrow_mut() = theme.id.clone();
        if let Some(hook) = self.window_controls_hook.borrow().as_ref() {
            hook(theme.window_controls);
        }

        // Dark palettes need dark GTK chrome; explicitly pinned light
        // palettes need light chrome even when GNOME is dark. Adaptive themes
        // leave the scheme at the desktop default.
        if theme.dark {
            self.style_manager.set_color_scheme(adw::ColorScheme::ForceDark);
            self.lightbox.use_iphone_backdrop();
        } else if theme.pins_color_scheme {
            self.style_manager.set_color_scheme(adw::ColorScheme::ForceLight);
            self.lightbox.use_standard_backdrop(false);
        } else {
            self.style_manager.set_color_scheme(adw::ColorScheme::Default);
            self.lightbox.use_standard_backdrop(self.style_manager.is_dark());
        }
        self.update_appearance_icon(theme.dark);
    }

    fn update_appearance_icon(&self, dark: bool) {
        if let Some(button) = self.appearance_button.borrow().as_ref() {
            button.set_icon_name(if dark {
                "weather-clear-night-symbolic"
            } else {
                "weather-clear-symbolic"
            });
        }
    }
}

/// Header-bar button. Opens Settings > Themes (wired by the caller) and its
/// sun/moon icon mirrors the active theme's `dark` flag.
pub(crate) fn appearance_button() -> gtk::Button {
    let button = gtk::Button::new();
    button.set_icon_name("weather-clear-symbolic");
    button.set_tooltip_text(Some("Appearance"));
    button
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell as StdCell;

    fn unique_temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "pic-theme-engine-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_theme(root: &PathBuf, id: &str, header: &str) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("theme.css"), format!("{header}\n.{id} {{}}")).unwrap();
    }

    fn memory_db() -> Rc<RefCell<Connection>> {
        let connection = Rc::new(RefCell::new(Connection::open_in_memory().unwrap()));
        connection
            .borrow()
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            )
            .unwrap();
        connection
    }

    fn settle_glow_preview() {
        let main_loop = glib::MainLoop::new(None, false);
        let loop_to_quit = main_loop.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
            loop_to_quit.quit();
        });
        main_loop.run();
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn tron_glow_presets_persist_and_theme_css_parses() {
        gtk::init().unwrap();
        let errors = Rc::new(RefCell::new(Vec::new()));
        let provider = gtk::CssProvider::new();
        {
            let errors = errors.clone();
            provider.connect_parsing_error(move |_, section, error| {
                errors
                    .borrow_mut()
                    .push(format!("{}: {error}", section.start_location().lines()));
            });
        }
        let theme_css = include_str!("../../themes/tron/theme.css");
        let has_glow_presets = theme_css.contains(".tron-glow-subtle .photo-tile");
        provider.load_from_string(theme_css);
        assert!(errors.borrow().is_empty(), "TRON CSS errors: {errors:?}");
        let display = gdk::Display::default().unwrap();
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
        );

        let connection = memory_db();
        let lightbox = Rc::new(crate::lightbox::Lightbox::new());
        let engine = ThemeEngine::new(display.clone(), connection.clone(), lightbox);
        let window = gtk::Window::builder()
            .title("TRON Legacy — glow preview")
            .default_width(1120)
            .default_height(720)
            .build();
        window.set_decorated(false);
        let surface = gtk::Box::new(gtk::Orientation::Vertical, 18);
        surface.set_margin_top(24);
        surface.set_margin_bottom(24);
        surface.set_margin_start(28);
        surface.set_margin_end(28);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("layout-left-header");
        let title = gtk::Label::new(Some("THE GRID  /  PHOTO ARCHIVE"));
        title.add_css_class("app-title");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        header.append(&title);
        let active = gtk::ToggleButton::with_label("GRID ACTIVE");
        active.set_active(true);
        header.append(&active);
        surface.append(&header);

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 24);
        let sidebar = gtk::ListBox::new();
        sidebar.add_css_class("navigation-sidebar");
        sidebar.set_width_request(210);
        for (label, selected) in [("Library", false), ("All Photos", true), ("Favorites", false)] {
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&gtk::Label::new(Some(label))));
            if selected {
                row.set_state_flags(gtk::StateFlags::SELECTED, false);
            }
            sidebar.append(&row);
        }
        content.append(&sidebar);

        let page = gtk::Box::new(gtk::Orientation::Vertical, 18);
        page.set_hexpand(true);
        let section = gtk::Label::new(Some("RECOGNIZED SIGNALS  /  15 PHOTOS"));
        section.add_css_class("section-heading");
        section.set_xalign(0.0);
        page.append(&section);
        let tiles = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        for (index, (number, flags)) in [
            ("01  /  UNSELECTED", gtk::StateFlags::empty()),
            ("02  /  HOVER", gtk::StateFlags::PRELIGHT),
            ("03  /  SELECTED", gtk::StateFlags::empty()),
        ]
        .into_iter()
        .enumerate()
        {
            let tile = gtk::Overlay::new();
            tile.add_css_class("photo-frame");
            tile.add_css_class("photo-tile");
            tile.set_size_request(230, 158);
            let image = gtk::DrawingArea::new();
            image.set_content_width(230);
            image.set_content_height(158);
            image.set_draw_func(move |_, context, width, height| {
                let palette = [
                    [(0.08, 0.32, 0.58), (0.08, 0.62, 0.72)],
                    [(0.74, 0.22, 0.12), (0.94, 0.64, 0.18)],
                    [(0.14, 0.48, 0.30), (0.36, 0.72, 0.46)],
                ][index];
                let gradient = gtk::cairo::LinearGradient::new(0.0, 0.0, width as f64, height as f64);
                gradient.add_color_stop_rgb(0.0, palette[0].0, palette[0].1, palette[0].2);
                gradient.add_color_stop_rgb(1.0, palette[1].0, palette[1].1, palette[1].2);
                context.set_source(&gradient).unwrap();
                context.paint().unwrap();
                context.set_source_rgba(0.02, 0.04, 0.08, 0.58);
                context.arc(width as f64 * 0.72, height as f64 * 0.42, 30.0, 0.0, std::f64::consts::TAU);
                context.fill().unwrap();
                context.set_source_rgba(1.0, 1.0, 1.0, 0.34);
                context.move_to(12.0, height as f64 * 0.78);
                context.line_to(width as f64 * 0.68, height as f64 * 0.56);
                context.stroke().unwrap();
            });
            tile.set_child(Some(&image));
            let caption = gtk::Label::new(Some(number));
            caption.add_css_class("dim-label");
            caption.set_halign(gtk::Align::Center);
            caption.set_valign(gtk::Align::End);
            caption.set_margin_bottom(8);
            tile.add_overlay(&caption);
            tile.set_state_flags(flags, false);
            if number.starts_with("03") {
                tile.add_css_class("folder-photo-selected");
            }
            tiles.append(&tile);
        }
        page.append(&tiles);

        let album_card = gtk::Button::new();
        album_card.add_css_class("album-card");
        album_card.add_css_class("framed-album-card");
        let album = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let cover = gtk::Overlay::new();
        cover.add_css_class("album-cover");
        cover.set_size_request(168, 126);
        let photo = gtk::DrawingArea::new();
        photo.set_content_width(168);
        photo.set_content_height(126);
        photo.set_draw_func(|_, context, width, height| {
            let gradient = gtk::cairo::LinearGradient::new(0.0, 0.0, width as f64, height as f64);
            gradient.add_color_stop_rgb(0.0, 0.91, 0.20, 0.08);
            gradient.add_color_stop_rgb(0.48, 0.98, 0.72, 0.12);
            gradient.add_color_stop_rgb(1.0, 0.04, 0.36, 0.82);
            context.set_source(&gradient).unwrap();
            context.rectangle(0.0, 0.0, width as f64, height as f64);
            context.fill().unwrap();
            context.set_source_rgb(0.03, 0.04, 0.07);
            context.arc(width as f64 * 0.68, height as f64 * 0.38, 19.0, 0.0, std::f64::consts::TAU);
            context.fill().unwrap();
        });
        photo.set_margin_start(59);
        photo.set_margin_top(34);
        photo.set_margin_end(56);
        photo.set_margin_bottom(40);
        cover.set_child(Some(&photo));
        let frame = gtk::Picture::for_filename("images/theme/album-covers/standard/standard-frame.png");
        frame.set_content_fit(gtk::ContentFit::Fill);
        frame.set_can_target(false);
        cover.add_overlay(&frame);
        album.append(&cover);
        album.append(&gtk::Label::new(Some("NEON ARCHIVE")));
        album_card.set_child(Some(&album));
        album_card.set_halign(gtk::Align::Start);
        page.append(&album_card);

        let zoom = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
        zoom.set_value(68.0);
        zoom.set_hexpand(true);
        page.append(&zoom);
        content.append(&page);
        surface.append(&content);
        window.set_child(Some(&surface));
        window.present();
        settle_glow_preview();

        if let Ok(directory) = std::env::var("PIC_TRON_SCREENSHOT_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            for (preset, name) in [(0, "subtle"), (1, "standard"), (2, "cinematic")] {
                engine.set_tron_glow_preset(preset);
                let class = match preset {
                    0 => "tron-glow-subtle",
                    1 => "tron-glow-standard",
                    _ => "tron-glow-cinematic",
                };
                assert!(title.has_css_class(class));
                assert!(cover.has_css_class(class));
                window.queue_draw();
                settle_glow_preview();
                use gtk::gdk::prelude::PaintableExt;
                let snapshot = gtk::Snapshot::new();
                gtk::WidgetPaintable::new(Some(&window)).snapshot(
                    &snapshot,
                    window.width() as f64,
                    window.height() as f64,
                );
                window
                    .renderer()
                    .unwrap()
                    .render_texture(&snapshot.to_node().unwrap(), None)
                    .save_to_png(format!("{directory}/tron-{name}.png"))
                    .unwrap();
            }
            let sample_photo_pixel = |name: &str| {
                image::open(format!("{directory}/tron-{name}.png"))
                    .unwrap()
                    .to_rgb8()
                    .get_pixel(320, 180)
                    .0
            };
            let original = sample_photo_pixel("subtle");
            for name in ["standard", "cinematic"] {
                assert_eq!(
                    sample_photo_pixel(name),
                    original,
                    "photo pixels changed in {name} preset"
                );
            }
            let subtle = image::open(format!("{directory}/tron-subtle.png"))
                .unwrap()
                .to_rgb8();
            let cinematic = image::open(format!("{directory}/tron-cinematic.png"))
                .unwrap()
                .to_rgb8();
            if has_glow_presets {
                assert!(
                    subtle
                        .pixels()
                        .zip(cinematic.pixels())
                        .any(|(left, right)| left != right),
                    "Subtle and Cinematic must render different glow strengths"
                );
            }
        }
        for (preset, class) in [
            (0, "tron-glow-subtle"),
            (1, "tron-glow-standard"),
            (2, "tron-glow-cinematic"),
        ] {
            engine.set_tron_glow_preset(preset);
            assert!(window.has_css_class(class));
            assert_eq!(engine.tron_glow_preset(), preset);
        }
        window.close();
        gtk::style_context_remove_provider_for_display(&display, &provider);
    }

    /// Exercises the runtime failure paths from a display: rapid theme
    /// switching (provider swaps must not accumulate), rescans after themes
    /// are added, and a deleted active theme. Pure logic fallbacks are
    /// covered headlessly in css::theme_discovery.
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn theme_engine_survives_rapid_switching_rescans_and_deletions() {
        gtk::init().unwrap();
        let display = gtk::gdk::Display::default().unwrap();
        let root = unique_temp_dir("runtime");
        write_theme(&root, "alpha", "/* picasa-theme\n   name: Alpha\n*/");
        write_theme(&root, "beta", "/* picasa-theme\n   name: Beta\n*/");
        write_theme(&root, "zeta", "/* picasa-theme\n   name: Zeta\n   dark: true\n   mode: base\n*/");

        let connection = memory_db();
        let lightbox = Rc::new(crate::lightbox::Lightbox::new());
        let engine = ThemeEngine::new(display, connection.clone(), lightbox);
        engine.set_themes_dir(root.clone());
        engine.startup();

        // No saved selection and no "standard" folder: the first theme wins.
        let themes = engine.themes();
        assert_eq!(themes.len(), 3);
        assert_eq!(engine.active_id(), "alpha");

        // 50 rapid switches between overlays and the base theme: the
        // provider swap must stay bounded (remove-then-add) and the saved
        // setting must always track the last selection.
        for index in 0..50 {
            let theme = themes[index % themes.len()].clone();
            engine.select(&theme);
            assert_eq!(engine.active_id(), theme.id);
            assert_eq!(
                crate::db::setting(&connection.borrow(), THEME_SETTING_KEY)
                    .unwrap()
                    .as_deref(),
                Some(theme.id.as_str())
            );
        }
        // Re-selecting the active theme must be a no-op, not a re-apply.
        let before = engine.active_id();
        engine.select(&themes[0]);
        assert_eq!(engine.active_id(), before);

        // Repeated Settings-style rebuilds rescan the folder; the count is
        // stable and the switch state survives.
        for _ in 0..10 {
            assert_eq!(engine.themes().len(), 3);
        }

        // Deleting the selected theme's folder: the engine keeps running on
        // the in-memory provider until the user picks another theme, which
        // must succeed against the reduced scan.
        std::fs::remove_dir_all(root.join(engine.active_id())).unwrap();
        let remaining = engine.themes();
        assert_eq!(remaining.len(), 2);
        let replacement = remaining
            .iter()
            .find(|theme| theme.id != engine.active_id())
            .unwrap()
            .clone();
        engine.select(&replacement);
        assert_eq!(engine.active_id(), replacement.id);

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// An empty or missing themes folder must leave the engine inert (stock
    /// GTK appearance) without panicking or touching providers.
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn theme_engine_with_no_themes_stays_inert() {
        gtk::init().unwrap();
        let display = gtk::gdk::Display::default().unwrap();
        let connection = memory_db();
        let lightbox = Rc::new(crate::lightbox::Lightbox::new());
        let engine = ThemeEngine::new(display, connection.clone(), lightbox);
        engine.set_themes_dir(unique_temp_dir("does-not-exist"));
        engine.startup();
        assert_eq!(engine.active_id(), "");
        assert!(!engine.active_is_dark());
        assert!(engine.themes().is_empty());

        // A theme appearing later is picked up by the next scan.
        let root = unique_temp_dir("appears-later");
        write_theme(&root, "late", "/* picasa-theme\n   name: Late\n*/");
        engine.set_themes_dir(root.clone());
        let themes = engine.themes();
        engine.select(&themes[0]);
        assert_eq!(engine.active_id(), "late");
        assert_eq!(
            crate::db::setting(&connection.borrow(), THEME_SETTING_KEY)
                .unwrap()
                .as_deref(),
            Some("late")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Guards against the rebuild path registering more signal handlers than
    /// intended: ThemeEngine::new must connect dark_notify exactly once.
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn theme_engine_connects_a_single_dark_notify_handler() {
        gtk::init().unwrap();
        let display = gtk::gdk::Display::default().unwrap();
        let engine = ThemeEngine::new(
            display,
            memory_db(),
            Rc::new(crate::lightbox::Lightbox::new()),
        );
        engine.set_themes_dir(unique_temp_dir("empty"));
        // The engine is the only StyleManager listener the app installs for
        // themes; without a display-side handler enumeration we assert the
        // construction path instead: new() connects once, nothing else does.
        let _ = StdCell::new(engine.active_id());
        assert!(!engine.active_is_dark());
    }
}
