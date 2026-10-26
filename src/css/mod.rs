use std::path::{Path, PathBuf};

use gtk4 as gtk;

pub(crate) const BASE: &str = include_str!("base.css");
pub(crate) const EDIT_MODE: &str = include_str!("edit_mode.css");
pub(crate) const PHOTO_WALL: &str = include_str!("photo_wall.css");
pub(crate) const SIDEBAR_RESIZE: &str = include_str!("components/sidebar_resize.css");
pub(crate) const SQUARE_CORNERS: &str = include_str!("square_corners.css");
pub(crate) const ALBUMS: &str = include_str!("components/albums.css");
pub(crate) const PHOTO_CONTEXT_MENU: &str = include_str!("components/photo_context_menu.css");
pub(crate) const SCREENSAVER: &str = include_str!("screensaver.css");

pub(crate) mod theme_discovery;

#[cfg(target_os = "windows")]
pub(crate) const PLATFORM: &str = include_str!("windows.css");
#[cfg(target_os = "macos")]
pub(crate) const PLATFORM: &str = include_str!("macos.css");
#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
pub(crate) const PLATFORM: &str = include_str!("linux.css");

/// Resolve a runtime resource directory such as `themes` or
/// `images/theme/album-covers`.
///
/// Relative paths normally resolve against the working directory: a source
/// checkout running via `cargo run`, or a packaged launcher that starts in
/// the install folder. When the folder does not exist there, fall back to
/// the same path next to the executable — `cargo build` stages a copy there
/// (see `build.rs`) and installers provide one — so the binary keeps finding
/// its resources no matter which directory it was launched from.
pub(crate) fn resolve_runtime_dir(configured: &Path) -> PathBuf {
    if configured.is_dir() {
        return configured.to_path_buf();
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(executable_dir) = executable.parent() {
            let candidate = executable_dir.join(configured);
            if candidate.is_dir() {
                return candidate;
            }
        }
    }
    configured.to_path_buf()
}

pub(crate) fn install_foundation(display: &gtk::gdk::Display) {
    let base = gtk::CssProvider::new();
    base.load_from_string(BASE);
    gtk::style_context_add_provider_for_display(
        display,
        &base,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Metadata controls use consistent native desktop geometry in every theme.
    // Theme palette variables supply colours; corner-style overrides win at +13.
    let metadata = gtk::CssProvider::new();
    metadata.load_from_string(include_str!("components/metadata_filter_layout.css"));
    gtk::style_context_add_provider_for_display(
        display,
        &metadata,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
    );

    // The edit toolbar is shared by every appearance theme. Keep its visual
    // contract in one place and resolve colors from the active theme palette.
    // This scoped provider sits above theme-wide button rules while remaining
    // below the user-selectable corner style.
    let edit_mode = gtk::CssProvider::new();
    edit_mode.load_from_string(EDIT_MODE);
    gtk::style_context_add_provider_for_display(
        display,
        &edit_mode,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
    );

    // Wall geometry must win over theme frames (+1) and corner settings (+4).
    // These rules match Wall tiles only; Grid keeps the selected theme.
    let photo_wall = gtk::CssProvider::new();
    photo_wall.load_from_string(PHOTO_WALL);
    gtk::style_context_add_provider_for_display(
        display,
        &photo_wall,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 5,
    );

    // Keep the wide resize hit target invisible so only the theme's divider
    // is painted, regardless of GTK or theme hover backgrounds.
    let sidebar_resize = gtk::CssProvider::new();
    sidebar_resize.load_from_string(SIDEBAR_RESIZE);
    gtk::style_context_add_provider_for_display(
        display,
        &sidebar_resize,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 5,
    );

    let platform = gtk::CssProvider::new();
    platform.load_from_string(PLATFORM);
    gtk::style_context_add_provider_for_display(
        display,
        &platform,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 10,
    );

    // Structural rules must win over theme entry styling: there is only
    // one Search/activity surface, including its keyboard focus ring.
    let search_status = gtk::CssProvider::new();
    search_status.load_from_string(include_str!("components/search_status.css"));
    gtk::style_context_add_provider_for_display(
        display,
        &search_status,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 6,
    );

    // Theme rows show the radio choice and pointer hover independently of
    // the selected theme's generic boxed-list styling.
    let theme_picker = gtk::CssProvider::new();
    theme_picker.load_from_string(include_str!("components/theme_picker.css"));
    gtk::style_context_add_provider_for_display(
        display,
        &theme_picker,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 6,
    );

    // Screensaver controls live in a separate window from the main/settings
    // window. Keep its headings readable even under overlays with broad
    // heading rules; the selector is scoped to those two labels only.
    let screensaver = gtk::CssProvider::new();
    screensaver.load_from_string(SCREENSAVER);
    gtk::style_context_add_provider_for_display(
        display,
        &screensaver,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 11,
    );

    // A single hover contract for top-bar and InfoBar action buttons must
    // outrank the selected theme (+1/+3) and platform styling (+10).
    // Native window controls and other app buttons remain untouched.
    let toolbar_actions = gtk::CssProvider::new();
    toolbar_actions.load_from_string(include_str!("components/toolbar_action_consistency.css"));
    gtk::style_context_add_provider_for_display(
        display,
        &toolbar_actions,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 12,
    );

    // User-selectable corner geometry is intentionally more specific than
    // per-theme radii (+1/+3) and shared toolbar hover geometry (+12).
    // It is scoped to windows carrying a corner-style-* class.
    let corner_style = gtk::CssProvider::new();
    corner_style.load_from_string(include_str!("components/corner_style.css"));
    gtk::style_context_add_provider_for_display(
        display,
        &corner_style,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 13,
    );

    // Sidebar header actions share a fixed 28px target and 14px glyph, with
    // the same theme-aware hover surface in every section.
    let sidebar_actions = gtk::CssProvider::new();
    sidebar_actions.load_from_string(include_str!("components/sidebar_action_consistency.css"));
    gtk::style_context_add_provider_for_display(
        display,
        &sidebar_actions,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 14,
    );

    // Legacy square-tile rules remain active when Corner style is Square,
    // preserving the Photo Wall/Masonry behaviour and older UI tests.
    let square_corners = gtk::CssProvider::new();
    square_corners.load_from_string(SQUARE_CORNERS);
    gtk::style_context_add_provider_for_display(
        display,
        &square_corners,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 4,
    );
}
