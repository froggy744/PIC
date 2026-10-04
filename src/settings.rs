use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use rusqlite::Connection;

mod database;

/// Destructive library maintenance actions. The settings window only owns the
/// buttons and their confirmation dialogs; the behaviour lives in the main
/// window (build.rs) where the gallery, filter, and refresh context exist.
#[derive(Clone)]
pub struct LibraryMaintenance {
    pub clear_thumbnails: Rc<dyn Fn()>,
    pub clear_database: Rc<dyn Fn()>,
    pub clear_all: Rc<dyn Fn()>,
}

#[derive(Clone)]
pub struct DatabaseManagement {
    pub switch_library: Rc<dyn Fn(&str) -> Result<(), String>>,
}

#[derive(Clone, Default)]
pub struct SettingsWindow {
    window: Rc<RefCell<glib::WeakRef<adw::Window>>>,
    stack: Rc<RefCell<glib::WeakRef<gtk::Stack>>>,
    /// Rebuilds the Appearance section on the Themes page from a fresh scan
    /// of the themes folder, so themes dropped into the folder appear the next time
    /// the window is presented. Set when the page is first built.
    refresh_appearance: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
}

impl SettingsWindow {
    /// Drop cached pages after a catalog switch so every control is rebuilt
    /// from the newly selected library the next time Settings opens.
    pub fn reset(&self) {
        if let Some(window) = self.window.borrow().upgrade() {
            window.set_visible(false);
            window.set_content(None::<&gtk::Widget>);
        }
        self.window.borrow_mut().set(None::<&adw::Window>);
        self.stack.borrow_mut().set(None::<&gtk::Stack>);
        self.refresh_appearance.borrow_mut().take();
    }

    pub fn present(
        &self,
        parent: &adw::ApplicationWindow,
        connection: Rc<RefCell<Connection>>,
        formats_changed: Rc<dyn Fn()>,
        theme_changed: Rc<dyn Fn()>,
        thumbnail_changed: Rc<dyn Fn()>,
        zoom_animations_changed: Rc<dyn Fn()>,
        clear_saved_views: Rc<dyn Fn()>,
        sidebar_changed: Rc<dyn Fn()>,
        folder_watch_changed: Rc<dyn Fn()>,
        maintenance: LibraryMaintenance,
        database_management: DatabaseManagement,
        theme_engine: Rc<crate::window::theme::ThemeEngine>,
        initial_page: Option<&str>,
    ) {
        if let Some(window) = self.window.borrow().upgrade() {
            if let (Some(page), Some(stack)) = (initial_page, self.stack.borrow().upgrade()) {
                stack.set_visible_child_name(page);
            }
            // The pages are built once and reused; rescan the theme folders
            // so newly dropped themes show up on the next open.
            if let Some(refresh) = self.refresh_appearance.borrow().as_ref() {
                refresh();
            }
            window.present();
            return;
        }

        let window = adw::Window::new();
        window.set_title(Some("Settings"));
        window.set_default_size(860, 620);
        window.set_transient_for(Some(parent));
        window.set_destroy_with_parent(true);
        window.set_modal(false);
        // Hide instead of destroy: closing settings must never depend on
        // widget teardown order, and reopening reuses the built pages.
        {
            window.connect_close_request(move |window| {
                crate::window::debug_log("SETTINGS: close requested -> hiding");
                window.set_visible(false);
                glib::Propagation::Stop
            });
        }

        let layout = gtk::Box::new(gtk::Orientation::Vertical, 0);
        layout.set_hexpand(true);
        layout.set_vexpand(true);
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&gtk::Label::new(Some("Settings"))));
        layout.append(&header);

        let stack = gtk::Stack::new();
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        // GtkStack is homogeneous by default, which makes the widest/tallest
        // hidden page participate in the initial window and Paned allocation.
        // The Database page is wider than the rc7 pages, so that first
        // allocation was clipped until a manual resize triggered another one.
        stack.set_hhomogeneous(false);
        stack.set_vhomogeneous(false);
        stack.set_transition_type(gtk::StackTransitionType::None);

        stack.add_titled(
            &interface_page(
                connection.clone(),
                theme_changed.clone(),
                thumbnail_changed,
                zoom_animations_changed,
                clear_saved_views,
            ),
            Some("interface"),
            "Interface",
        );

        let (themes_page, refresh_appearance) = themes_page(theme_engine);
        self.refresh_appearance
            .borrow_mut()
            .replace(refresh_appearance);
        stack.add_titled(&themes_page, Some("themes"), "Themes");

        stack.add_titled(
            &sidebar_page(connection.clone(), sidebar_changed),
            Some("sidebar"),
            "Sidebar",
        );

        stack.add_titled(
            &albums_page(connection.clone(), theme_changed),
            Some("albums"),
            "Albums",
        );

        stack.add_titled(
            &folders_page(connection.clone(), folder_watch_changed),
            Some("folders"),
            "Folders",
        );

        stack.add_titled(
            &formats_page(connection.clone(), formats_changed.clone()),
            Some("formats"),
            "File Formats",
        );

        stack.add_titled(
            &library_page(connection.clone(), formats_changed),
            Some("library"),
            "Stats",
        );

        stack.add_titled(
            &database::page(
                &window,
                connection.clone(),
                maintenance,
                database_management,
            ),
            Some("database"),
            "Libraries",
        );
        stack.add_titled(&about_page(), Some("about"), "About");
        if let Some(page) = initial_page {
            stack.set_visible_child_name(page);
        }

        let categories = gtk::StackSidebar::new();
        categories.set_stack(&stack);
        categories.set_width_request(190);
        categories.set_vexpand(true);

        let split = gtk::Paned::new(gtk::Orientation::Horizontal);
        split.set_start_child(Some(&categories));
        split.set_end_child(Some(&stack));
        split.set_position(190);
        split.set_resize_start_child(false);
        split.set_shrink_start_child(false);
        // The content pane must be free to shrink into the space the window
        // actually has; otherwise page min-width pushes past the window edge.
        split.set_resize_end_child(true);
        split.set_shrink_end_child(true);
        split.set_wide_handle(true);
        split.set_hexpand(true);
        split.set_vexpand(true);
        layout.append(&split);
        window.set_content(Some(&layout));

        self.window.borrow_mut().set(Some(&window));
        self.stack.borrow_mut().set(Some(&stack));
        window.present();
    }
}

fn formats_page(
    connection: Rc<RefCell<Connection>>,
    formats_changed: Rc<dyn Fn()>,
) -> gtk::ScrolledWindow {
    let content = page_content(
        "File Formats",
        "Choose which indexed image formats are visible in the library.",
    );
    let list = settings_list();
    for format in crate::image_format::all() {
        let extensions = format
            .extensions
            .iter()
            .map(|extension| format!(".{extension}"))
            .collect::<Vec<_>>()
            .join(", ");
        let toggle = gtk::Switch::new();
        toggle.set_valign(gtk::Align::Center);
        toggle.set_active(
            crate::image_format::is_enabled(&connection.borrow(), format).unwrap_or(true),
        );
        let connection = connection.clone();
        let formats_changed = formats_changed.clone();
        toggle.connect_active_notify(move |toggle| {
            if let Err(error) =
                crate::image_format::set_enabled(&connection.borrow(), format, toggle.is_active())
            {
                eprintln!("Could not save {} visibility: {error}", format.name);
                return;
            }
            formats_changed();
        });
        append_row(
            &list,
            format.name,
            Some(&extensions),
            Some(toggle.upcast_ref()),
        );
    }

    let pair_modes = [
        crate::image_format::RawJpegPairMode::Both,
        crate::image_format::RawJpegPairMode::PreferJpeg,
        crate::image_format::RawJpegPairMode::PreferRaw,
    ];
    let pair_mode = gtk::DropDown::from_strings(&["Show both", "Prefer JPEG", "Prefer RAW"]);
    let saved_mode = crate::image_format::raw_jpeg_pair_mode(&connection.borrow());
    pair_mode.set_selected(
        pair_modes
            .iter()
            .position(|mode| *mode == saved_mode)
            .unwrap_or(0) as u32,
    );
    pair_mode.set_valign(gtk::Align::Center);
    {
        let connection = connection.clone();
        let formats_changed = formats_changed.clone();
        pair_mode.connect_selected_notify(move |combo| {
            let mode = pair_modes
                .get(combo.selected() as usize)
                .copied()
                .unwrap_or(crate::image_format::RawJpegPairMode::Both);
            if let Err(error) =
                crate::image_format::set_raw_jpeg_pair_mode(&connection.borrow(), mode)
            {
                eprintln!("Could not save RAW + JPEG pair preference: {error}");
                return;
            }
            formats_changed();
        });
    }
    append_row(
        &list,
        "RAW + JPEG pairs",
        Some("When both files share the same folder and filename stem."),
        Some(pair_mode.upcast_ref()),
    );

    content.append(&list);
    scroll_page(content)
}

fn sidebar_page(
    connection: Rc<RefCell<Connection>>,
    sidebar_changed: Rc<dyn Fn()>,
) -> gtk::ScrolledWindow {
    let content = page_content(
        "Sidebar",
        "Choose which sections and destinations are shown in the sidebar.",
    );
    let list = settings_list();
    for (title, subtitle, key) in [
        (
            "Library",
            "Show the Library section.",
            crate::sidebar::LIBRARY_VISIBLE_SETTING_KEY,
        ),
        (
            "Albums",
            "Show albums and the Albums section.",
            crate::sidebar::ALBUMS_VISIBLE_SETTING_KEY,
        ),
        (
            "Folders",
            "Show imported local folders.",
            crate::sidebar::FOLDERS_VISIBLE_SETTING_KEY,
        ),
        (
            "Network Shares",
            "Show registered network shares.",
            crate::sidebar::NETWORK_SHARES_VISIBLE_SETTING_KEY,
        ),
        (
            "All Photos",
            "Show All Photos in the Library section.",
            crate::sidebar::ALL_PHOTOS_VISIBLE_SETTING_KEY,
        ),
        (
            "Favourites",
            "Show Favourites in the Library section.",
            crate::sidebar::FAVOURITES_VISIBLE_SETTING_KEY,
        ),
        (
            "Recently Added",
            "Show Recently Added in the Library section.",
            crate::sidebar::RECENTLY_ADDED_VISIBLE_SETTING_KEY,
        ),
        (
            "History",
            "Show History in the Library section.",
            crate::sidebar::HISTORY_VISIBLE_SETTING_KEY,
        ),
    ] {
        let toggle = gtk::Switch::new();
        toggle.set_valign(gtk::Align::Center);
        toggle.set_active(saved_bool(&connection.borrow(), key).unwrap_or(true));
        {
            let connection = connection.clone();
            let sidebar_changed = sidebar_changed.clone();
            toggle.connect_active_notify(move |toggle| {
                if let Err(error) = crate::db::set_setting(
                    &connection.borrow(),
                    key,
                    &toggle.is_active().to_string(),
                ) {
                    eprintln!("Could not save sidebar visibility for {key}: {error}");
                    return;
                }
                sidebar_changed();
            });
        }
        append_row(&list, title, Some(subtitle), Some(toggle.upcast_ref()));
    }
    content.append(&list);
    scroll_page(content)
}

fn folders_page(
    connection: Rc<RefCell<Connection>>,
    folder_watch_changed: Rc<dyn Fn()>,
) -> gtk::ScrolledWindow {
    let content = page_content(
        "Folders",
        "Manage registered folders and choose which folders are watched for changes.",
    );

    let options = settings_list();
    let automatic = gtk::Switch::new();
    automatic.set_valign(gtk::Align::Center);
    automatic.set_active(crate::db::folder_watching_enabled(&connection.borrow()));
    {
        let connection = connection.clone();
        let folder_watch_changed = folder_watch_changed.clone();
        automatic.connect_active_notify(move |toggle| {
            if let Err(error) =
                crate::db::set_folder_watching_enabled(&connection.borrow(), toggle.is_active())
            {
                eprintln!("Could not save automatic folder watching setting: {error}");
                return;
            }
            folder_watch_changed();
        });
    }
    append_row(
        &options,
        "Automatic folder watching",
        Some("Detect changes automatically in folders marked for watching."),
        Some(automatic.upcast_ref()),
    );
    content.append(&options);

    let heading = gtk::Label::new(Some("Library folders"));
    heading.set_halign(gtk::Align::Start);
    heading.set_hexpand(true);
    heading.add_css_class("heading");
    content.append(&heading);

    let tree = settings_list();
    content.append(&tree);

    let folders = Rc::new(crate::db::folders(&connection.borrow()).unwrap_or_default());
    let expanded = Rc::new(RefCell::new(HashSet::<i64>::new()));

    rebuild_settings_folder_tree(
        &tree,
        folders.clone(),
        expanded.clone(),
        automatic.clone(),
        connection,
        folder_watch_changed,
    );

    scroll_page(content)
}

fn rebuild_settings_folder_tree(
    list: &gtk::ListBox,
    folders: Rc<Vec<crate::db::Folder>>,
    expanded: Rc<RefCell<HashSet<i64>>>,
    automatic: gtk::Switch,
    connection: Rc<RefCell<Connection>>,
    folder_watch_changed: Rc<dyn Fn()>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    if folders.is_empty() {
        append_empty_state(list, "No library folders", true);
        return;
    }

    let by_id = folders
        .iter()
        .map(|folder| (folder.id, folder))
        .collect::<HashMap<_, _>>();
    let mut children = HashMap::<Option<i64>, Vec<i64>>::new();
    for folder in folders.iter() {
        let parent = folder.parent_id.filter(|parent| by_id.contains_key(parent));
        children.entry(parent).or_default().push(folder.id);
    }

    for ids in children.values_mut() {
        ids.sort_by(|left, right| {
            let left = by_id.get(left).expect("folder id exists");
            let right = by_id.get(right).expect("folder id exists");
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.path.to_lowercase().cmp(&right.path.to_lowercase()))
        });
    }

    let roots = children.get(&None).cloned().unwrap_or_default();
    for id in roots {
        append_settings_folder_branch(
            list,
            id,
            0,
            &by_id,
            &children,
            folders.clone(),
            expanded.clone(),
            automatic.clone(),
            connection.clone(),
            folder_watch_changed.clone(),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn append_settings_folder_branch(
    list: &gtk::ListBox,
    folder_id: i64,
    depth: usize,
    by_id: &HashMap<i64, &crate::db::Folder>,
    children: &HashMap<Option<i64>, Vec<i64>>,
    folders: Rc<Vec<crate::db::Folder>>,
    expanded: Rc<RefCell<HashSet<i64>>>,
    automatic: gtk::Switch,
    connection: Rc<RefCell<Connection>>,
    folder_watch_changed: Rc<dyn Fn()>,
) {
    let Some(folder) = by_id.get(&folder_id).copied() else {
        return;
    };
    let child_ids = children.get(&Some(folder_id)).cloned().unwrap_or_default();
    let has_children = !child_ids.is_empty();
    let is_expanded = expanded.borrow().contains(&folder_id);

    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);

    let horizontal = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    horizontal.set_hexpand(true);
    horizontal.set_margin_top(8);
    horizontal.set_margin_bottom(8);
    horizontal.set_margin_start(12 + (depth as i32 * 18));
    horizontal.set_margin_end(12);

    let disclosure = if has_children {
        let button = gtk::Button::from_icon_name(if is_expanded {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        });
        button.add_css_class("flat");
        button.set_valign(gtk::Align::Center);
        button.set_tooltip_text(Some(if is_expanded {
            "Collapse folder"
        } else {
            "Expand folder"
        }));

        let list = list.clone();
        let folders = folders.clone();
        let expanded = expanded.clone();
        let automatic = automatic.clone();
        let connection = connection.clone();
        let folder_watch_changed = folder_watch_changed.clone();
        button.connect_clicked(move |_| {
            {
                let mut expanded = expanded.borrow_mut();
                if !expanded.insert(folder_id) {
                    expanded.remove(&folder_id);
                }
            }
            rebuild_settings_folder_tree(
                &list,
                folders.clone(),
                expanded.clone(),
                automatic.clone(),
                connection.clone(),
                folder_watch_changed.clone(),
            );
        });
        Some(button)
    } else {
        None
    };

    if let Some(disclosure) = disclosure.as_ref() {
        horizontal.append(disclosure);
    } else {
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_width_request(30);
        horizontal.append(&spacer);
    }

    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);

    let title = gtk::Label::new(Some(&folder.name));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    labels.append(&title);

    let status = if folder.available {
        "Available"
    } else {
        "Unavailable"
    };
    let subtitle = gtk::Label::new(Some(&format!(
        "{}\n{status} · {} photos",
        folder.path, folder.photo_count
    )));
    subtitle.set_xalign(0.0);
    subtitle.set_hexpand(true);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    subtitle.add_css_class("dim-label");
    labels.append(&subtitle);
    horizontal.append(&labels);

    let toggle = gtk::Switch::new();
    toggle.set_valign(gtk::Align::Center);
    toggle.set_active(folder.watched);
    automatic
        .bind_property("active", &toggle, "sensitive")
        .sync_create()
        .build();

    {
        let connection = connection.clone();
        let folder_watch_changed = folder_watch_changed.clone();
        toggle.connect_active_notify(move |toggle| {
            match crate::db::set_folder_watched(&connection.borrow(), folder_id, toggle.is_active())
            {
                Ok(true) => folder_watch_changed(),
                Ok(false) => {
                    eprintln!("Could not change watch state for missing folder {folder_id}")
                }
                Err(error) => eprintln!("Could not change folder watch state: {error}"),
            }
        });
    }

    horizontal.append(&toggle);
    row.set_child(Some(&horizontal));
    list.append(&row);

    if has_children && is_expanded {
        for child_id in child_ids {
            append_settings_folder_branch(
                list,
                child_id,
                depth + 1,
                by_id,
                children,
                folders.clone(),
                expanded.clone(),
                automatic.clone(),
                connection.clone(),
                folder_watch_changed.clone(),
            );
        }
    }
}

fn albums_page(
    connection: Rc<RefCell<Connection>>,
    theme_changed: Rc<dyn Fn()>,
) -> gtk::ScrolledWindow {
    let content = page_content(
        "Albums",
        "Customize album presentation and review albums in the current library.",
    );

    append_album_appearance(&content, connection.clone(), theme_changed);

    let heading = gtk::Label::new(Some("Albums in this library"));
    heading.set_halign(gtk::Align::Start);
    heading.set_hexpand(true);
    heading.add_css_class("heading");
    content.append(&heading);

    let albums = crate::db::albums(&connection.borrow()).unwrap_or_default();
    let list = settings_list();
    for album in &albums {
        append_row(
            &list,
            &album.name,
            Some(&format!("{} photos", album.photo_count)),
            None,
        );
    }
    append_empty_state(&list, "No albums", albums.is_empty());
    content.append(&list);
    scroll_page(content)
}

fn append_album_appearance(
    content: &gtk::Box,
    connection: Rc<RefCell<Connection>>,
    theme_changed: Rc<dyn Fn()>,
) {
    let heading = gtk::Label::new(Some("Appearance"));
    heading.set_halign(gtk::Align::Start);
    heading.set_hexpand(true);
    heading.add_css_class("heading");
    content.append(&heading);

    let list = settings_list();
    let appearance = album_appearance(&connection.borrow());
    let updating = Rc::new(Cell::new(false));

    let bookshelf = gtk::Switch::new();
    bookshelf.set_active(appearance.bookshelf_enabled);
    bookshelf.set_valign(gtk::Align::Center);
    append_row(
        &list,
        "Bookshelf",
        Some("Show the selected wooden background behind albums."),
        Some(bookshelf.upcast_ref()),
    );

    let next_background = gtk::Button::with_label("Next Background");
    next_background.set_valign(gtk::Align::Center);
    append_row(
        &list,
        "Bookshelf background",
        Some("Cycle through the available bookshelf images."),
        Some(next_background.upcast_ref()),
    );

    let album_covers = gtk::Switch::new();
    album_covers.set_active(appearance.covers_enabled);
    album_covers.set_valign(gtk::Align::Center);
    append_row(
        &list,
        "Album Covers",
        Some("Show decorative covers around album thumbnails."),
        Some(album_covers.upcast_ref()),
    );

    let next_covers = gtk::Button::with_label("Next Album Covers");
    next_covers.set_valign(gtk::Align::Center);
    append_row(
        &list,
        "Album cover design",
        Some("Cycle through the available cover themes."),
        Some(next_covers.upcast_ref()),
    );

    let disable_all = gtk::Button::with_label("Disable All Themes");
    disable_all.set_valign(gtk::Align::Center);
    disable_all.set_sensitive(appearance.bookshelf_enabled || appearance.covers_enabled);
    append_row(
        &list,
        "Default appearance",
        Some("Turn off the bookshelf and album covers."),
        Some(disable_all.upcast_ref()),
    );

    let reset_all = gtk::Button::with_label("Reset All Theme Settings");
    reset_all.set_valign(gtk::Align::Center);
    reset_all.add_css_class("reset-all-themes-action");
    append_row(
        &list,
        "Reset themes",
        Some("Clear every saved theme choice, including each album's own cover."),
        Some(reset_all.upcast_ref()),
    );

    {
        let connection = connection.clone();
        let theme_changed = theme_changed.clone();
        let updating = updating.clone();
        let disable_all = disable_all.clone();
        let album_covers = album_covers.clone();
        bookshelf.connect_active_notify(move |bookshelf| {
            if updating.get() {
                return;
            }
            if let Err(error) = set_bookshelf_enabled(&connection.borrow(), bookshelf.is_active()) {
                eprintln!("Could not save bookshelf setting: {error}");
                return;
            }
            disable_all.set_sensitive(bookshelf.is_active() || album_covers.is_active());
            theme_changed();
        });
    }
    {
        let connection = connection.clone();
        let theme_changed = theme_changed.clone();
        let updating = updating.clone();
        let disable_all = disable_all.clone();
        let bookshelf = bookshelf.clone();
        album_covers.connect_active_notify(move |covers| {
            if updating.get() {
                return;
            }
            if let Err(error) = set_covers_enabled(&connection.borrow(), covers.is_active()) {
                eprintln!("Could not save album cover setting: {error}");
                return;
            }
            disable_all.set_sensitive(bookshelf.is_active() || covers.is_active());
            theme_changed();
        });
    }
    {
        let connection = connection.clone();
        let theme_changed = theme_changed.clone();
        let updating = updating.clone();
        let bookshelf = bookshelf.clone();
        let disable_all = disable_all.clone();
        next_background.connect_clicked(move |_| {
            if let Err(error) = next_bookshelf_background(
                &connection.borrow(),
                crate::albums_view::bookshelf_background_count(),
            ) {
                eprintln!("Could not save bookshelf background: {error}");
                return;
            }
            updating.set(true);
            bookshelf.set_active(true);
            updating.set(false);
            disable_all.set_sensitive(true);
            theme_changed();
        });
    }
    {
        let connection = connection.clone();
        let theme_changed = theme_changed.clone();
        let updating = updating.clone();
        let album_covers = album_covers.clone();
        let disable_all = disable_all.clone();
        next_covers.connect_clicked(move |_| {
            if let Err(error) = next_album_covers(
                &connection.borrow(),
                crate::albums_view::album_cover_theme_count(),
            ) {
                eprintln!("Could not save album cover design: {error}");
                return;
            }
            updating.set(true);
            album_covers.set_active(true);
            updating.set(false);
            disable_all.set_sensitive(true);
            theme_changed();
        });
    }
    {
        let connection = connection.clone();
        let theme_changed = theme_changed.clone();
        let updating = updating.clone();
        let bookshelf = bookshelf.clone();
        let album_covers = album_covers.clone();
        disable_all.connect_clicked(move |button| {
            if let Err(error) = disable_all_album_themes(&connection.borrow()) {
                eprintln!("Could not disable album themes: {error}");
                return;
            }
            updating.set(true);
            bookshelf.set_active(false);
            album_covers.set_active(false);
            updating.set(false);
            button.set_sensitive(false);
            theme_changed();
        });
    }
    {
        let connection = connection.clone();
        let theme_changed = theme_changed.clone();
        let updating = updating.clone();
        let bookshelf = bookshelf.clone();
        let album_covers = album_covers.clone();
        let disable_all = disable_all.clone();
        reset_all.connect_clicked(move |_| {
            if let Err(error) = reset_all_album_themes(&connection.borrow()) {
                eprintln!("Could not reset theme settings: {error}");
                return;
            }
            updating.set(true);
            bookshelf.set_active(false);
            album_covers.set_active(false);
            updating.set(false);
            disable_all.set_sensitive(false);
            theme_changed();
        });
    }
    content.append(&list);
}

fn library_page(
    connection: Rc<RefCell<Connection>>,
    recently_added_changed: Rc<dyn Fn()>,
) -> gtk::ScrolledWindow {
    let content = page_content("Stats", "A compact overview of the current library.");

    let options = settings_list();
    let recent_limit = gtk::SpinButton::with_range(1.0, 1_000_000.0, 1.0);
    recent_limit.set_value(crate::db::recently_added_limit(&connection.borrow()) as f64);
    recent_limit.set_numeric(true);
    recent_limit.set_digits(0);
    {
        let connection = connection.clone();
        recent_limit.connect_value_changed(move |spin| {
            let value = spin.value_as_int().max(1);
            if let Err(error) = crate::db::set_setting(
                &connection.borrow(),
                crate::db::RECENTLY_ADDED_LIMIT_SETTING_KEY,
                &value.to_string(),
            ) {
                eprintln!("Could not save Recently Added limit: {error}");
                return;
            }
            recently_added_changed();
        });
    }
    append_row(
        &options,
        "Recently Added limit",
        Some("Maximum number of photos shown in Recently Added."),
        Some(recent_limit.upcast_ref()),
    );
    content.append(&options);

    let counts = crate::db::library_counts(&connection.borrow()).unwrap_or_default();
    let database_size = crate::db::database_size(&connection.borrow()).unwrap_or_default();
    let available = crate::db::setting(
        &connection.borrow(),
        crate::db::LIBRARY_AVAILABLE_SETTING_KEY,
    )
    .ok()
    .flatten()
    .unwrap_or_else(|| "Not calculated yet".to_string());
    let unavailable = crate::db::setting(
        &connection.borrow(),
        crate::db::LIBRARY_UNAVAILABLE_SETTING_KEY,
    )
    .ok()
    .flatten()
    .unwrap_or_else(|| "Not calculated yet".to_string());

    let stats_grid = gtk::Grid::new();
    stats_grid.set_column_spacing(12);
    stats_grid.set_row_spacing(0);
    stats_grid.set_column_homogeneous(true);
    stats_grid.set_hexpand(true);

    let left = settings_list();
    append_row(
        &left,
        "Total photos",
        Some(&format_count(counts.photos.max(0) as u64)),
        None,
    );
    let available_label = append_row(&left, "Originals available", Some(&available), None)
        .expect("availability value row has a value label");
    let unavailable_label = append_row(&left, "Originals unavailable", Some(&unavailable), None)
        .expect("availability value row has a value label");

    let right = settings_list();
    append_row(
        &right,
        "Database size",
        Some(&format_bytes(database_size)),
        None,
    );
    append_row(
        &right,
        "Total albums",
        Some(&format_count(counts.albums.max(0) as u64)),
        None,
    );
    append_row(
        &right,
        "Library folders",
        Some(&format_count(counts.folders.max(0) as u64)),
        None,
    );

    stats_grid.attach(&left, 0, 0, 1, 1);
    stats_grid.attach(&right, 1, 0, 1, 1);
    content.append(&stats_grid);

    let updated = gtk::Label::new(
        crate::db::setting(
            &connection.borrow(),
            crate::db::LIBRARY_STATS_UPDATED_SETTING_KEY,
        )
        .ok()
        .flatten()
        .as_deref(),
    );
    updated.set_xalign(1.0);
    updated.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let update_button = gtk::Button::with_label("Update now");
    update_button.set_valign(gtk::Align::Center);
    let updated_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    updated_row.append(&update_button);
    updated_row.append(&updated);

    let availability = settings_list();
    append_row(
        &availability,
        "Availability stats last updated",
        Some("Recheck original-file availability only when needed."),
        Some(updated_row.upcast_ref()),
    );
    content.append(&availability);

    let thumbnail_heading = gtk::Label::new(Some("Thumbnail cache"));
    thumbnail_heading.set_halign(gtk::Align::Start);
    thumbnail_heading.set_hexpand(true);
    thumbnail_heading.add_css_class("heading");
    content.append(&thumbnail_heading);

    let thumbnail_grid = gtk::Grid::new();
    thumbnail_grid.set_column_spacing(12);
    thumbnail_grid.set_row_spacing(0);
    thumbnail_grid.set_column_homogeneous(true);
    thumbnail_grid.set_hexpand(true);

    let thumbnail_left = settings_list();
    let cached_label = stat_row(&thumbnail_left, "Cached thumbnails");
    let required_label = stat_row(&thumbnail_left, "Required thumbnails");

    let thumbnail_right = settings_list();
    let unused_label = stat_row(&thumbnail_right, "Unused thumbnails");
    let cache_size_label = stat_row(&thumbnail_right, "Thumbnail cache size");

    thumbnail_grid.attach(&thumbnail_left, 0, 0, 1, 1);
    thumbnail_grid.attach(&thumbnail_right, 1, 0, 1, 1);
    content.append(&thumbnail_grid);

    refresh_thumbnail_cache_stats(
        connection.clone(),
        cached_label,
        required_label,
        unused_label,
        cache_size_label,
    );

    let stats_running = Rc::new(Cell::new(false));
    {
        let connection = connection.clone();
        let available_label = available_label.clone();
        let unavailable_label = unavailable_label.clone();
        let updated = updated.clone();
        let button_for_click = update_button.clone();
        let stats_running = stats_running.clone();
        button_for_click.connect_clicked(move |button| {
            if stats_running.replace(true) {
                return;
            }
            button.set_sensitive(false);
            updated.set_text("Updating…");
            schedule_availability_stats(
                connection.clone(),
                available_label.clone(),
                unavailable_label.clone(),
                updated.clone(),
                button.clone(),
                stats_running.clone(),
            );
        });
    }

    scroll_page(content)
}

/// Destructive-action confirmation dialog parented to the settings window.
fn confirm_destructive(parent: &adw::Window, title: &str, message: &str, action: Rc<dyn Fn()>) {
    let dialog = adw::AlertDialog::builder()
        .heading(title)
        .body(message)
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("continue", "Continue");
    dialog.connect_response(Some("continue"), move |_, _| action());
    dialog.present(Some(parent));
}

/// Load the thumbnail cache statistics in the background and fill the Library
/// page labels when the measurement finishes. The key set is built on the main
/// thread (the database connection is not `Send`); the directory scan and byte
/// counting run on a worker so a large cache never blocks the UI.
fn refresh_thumbnail_cache_stats(
    connection: Rc<RefCell<Connection>>,
    cached: gtk::Label,
    required: gtk::Label,
    unused: gtk::Label,
    size: gtk::Label,
) {
    let valid = match crate::thumbnail::valid_cache_paths(&connection.borrow()) {
        Ok(valid) => valid,
        Err(error) => {
            eprintln!("Could not collect thumbnail cache keys: {error:#}");
            for label in [&cached, &required, &unused, &size] {
                label.set_text("Unavailable");
            }
            return;
        }
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(crate::thumbnail::cache_stats(&valid));
    });
    glib::timeout_add_local(
        std::time::Duration::from_millis(50),
        move || match receiver.try_recv() {
            Ok(Ok(stats)) => {
                cached.set_text(&format_count(stats.cached));
                required.set_text(&format_count(stats.required));
                unused.set_text(&format_count(stats.unused()));
                size.set_text(&format_bytes(stats.bytes));
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                eprintln!("Could not measure thumbnail cache: {error:#}");
                for label in [&cached, &required, &unused, &size] {
                    label.set_text("Unavailable");
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        },
    );
}

fn cleanup_result_text(cleanup: &crate::thumbnail::CacheCleanup) -> String {
    let removed = if cleanup.removed == 0 {
        "No unused thumbnails".to_string()
    } else {
        format!(
            "Removed {} unused thumbnail{}",
            format_count(cleanup.removed as u64),
            if cleanup.removed == 1 { "" } else { "s" }
        )
    };
    if cleanup.bytes_freed > 0 {
        format!("{removed} · Freed {}", format_bytes(cleanup.bytes_freed))
    } else {
        removed
    }
}

fn schedule_availability_stats(
    connection: Rc<RefCell<Connection>>,
    available_label: gtk::Label,
    unavailable_label: gtk::Label,
    updated: gtk::Label,
    update_button: gtk::Button,
    stats_running: Rc<Cell<bool>>,
) {
    const PAGE_SIZE: usize = 256;
    let mut offset = 0usize;
    let mut available = 0i64;
    let mut unavailable = 0i64;
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        let page = match crate::db::photo_availability_page(&connection.borrow(), PAGE_SIZE, offset)
        {
            Ok(page) => page,
            Err(error) => {
                eprintln!("Could not update library availability stats: {error}");
                stats_running.set(false);
                update_button.set_sensitive(true);
                return glib::ControlFlow::Break;
            }
        };
        if page.is_empty() {
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
            if let Err(error) = crate::db::set_setting(
                &connection.borrow(),
                crate::db::LIBRARY_AVAILABLE_SETTING_KEY,
                &available.to_string(),
            ) {
                eprintln!("Could not save available library stat: {error}");
            }
            if let Err(error) = crate::db::set_setting(
                &connection.borrow(),
                crate::db::LIBRARY_UNAVAILABLE_SETTING_KEY,
                &unavailable.to_string(),
            ) {
                eprintln!("Could not save unavailable library stat: {error}");
            }
            if let Err(error) = crate::db::set_setting(
                &connection.borrow(),
                crate::db::LIBRARY_STATS_UPDATED_SETTING_KEY,
                &timestamp,
            ) {
                eprintln!("Could not save library stats timestamp: {error}");
            }
            updated.set_text(&timestamp);
            stats_running.set(false);
            update_button.set_sensitive(true);
            return glib::ControlFlow::Break;
        }

        for (path, _folder_path) in &page {
            let is_available = crate::source::cached_file_available(path);
            if is_available {
                available += 1;
            } else {
                unavailable += 1;
            }
        }
        offset += page.len();
        available_label.set_text(&available.to_string());
        unavailable_label.set_text(&unavailable.to_string());
        updated.set_text("Updating…");
        glib::ControlFlow::Continue
    });
}

/// Refresh cached availability values after an import or refresh without
/// requiring the Settings window to be open. Filesystem/GIO probing is done on
/// a worker thread; doing even paginated existence checks on GTK can hang when
/// removable or network sources are slow.
pub fn refresh_library_availability_stats(_connection: Rc<RefCell<Connection>>) {
    let database = crate::db::connection_path(&_connection.borrow()).ok();
    std::thread::spawn(move || {
        const PAGE_SIZE: usize = 512;
        let connection = match database
            .ok_or_else(|| anyhow::anyhow!("active library has no database path"))
            .and_then(|database| crate::db::open_existing(&database))
        {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("Could not open database for library availability stats: {error}");
                return;
            }
        };
        let mut offset = 0usize;
        let mut available = 0i64;
        let mut unavailable = 0i64;
        loop {
            let page = match crate::db::photo_availability_page(&connection, PAGE_SIZE, offset) {
                Ok(page) => page,
                Err(error) => {
                    eprintln!("Could not refresh library availability stats: {error}");
                    return;
                }
            };
            if page.is_empty() {
                break;
            }
            for (path, _folder_path) in &page {
                if crate::source::cached_file_available(path) {
                    available += 1;
                } else {
                    unavailable += 1;
                }
            }
            offset += page.len();
        }
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        if let Err(error) = crate::db::set_setting(
            &connection,
            crate::db::LIBRARY_AVAILABLE_SETTING_KEY,
            &available.to_string(),
        ) {
            eprintln!("Could not save available library stat: {error}");
        }
        if let Err(error) = crate::db::set_setting(
            &connection,
            crate::db::LIBRARY_UNAVAILABLE_SETTING_KEY,
            &unavailable.to_string(),
        ) {
            eprintln!("Could not save unavailable library stat: {error}");
        }
        if let Err(error) = crate::db::set_setting(
            &connection,
            crate::db::LIBRARY_STATS_UPDATED_SETTING_KEY,
            &timestamp,
        ) {
            eprintln!("Could not save library stats timestamp: {error}");
        }
    });
}

// Keep the original key so existing bookshelf preferences survive the theme expansion.
pub(crate) const BOOKSHELF_SETTING_KEY: &str = "iphone-bookshelf-albums";
pub(crate) const ALBUM_VIEW_STYLE_SETTING_KEY: &str = "albums-home-style";
pub(crate) const ALBUM_BOOKSHELF_ENABLED_SETTING_KEY: &str = "albums-bookshelf-enabled";
pub(crate) const ALBUM_COVERS_ENABLED_SETTING_KEY: &str = "albums-covers-enabled";
pub(crate) const ALBUM_BOOKSHELF_BACKGROUND_SETTING_KEY: &str = "albums-bookshelf-background";
pub(crate) const ALBUM_COVER_FRAME_SETTING_KEY: &str = "albums-cover-frame";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AlbumAppearance {
    pub bookshelf_enabled: bool,
    pub covers_enabled: bool,
    pub background_index: usize,
    /// Shifts the design every album without its own frame is drawn with, so
    /// "Next Album Covers" advances the whole page by one design.
    pub cover_index: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AlbumViewStyle {
    #[default]
    Default,
    Bookshelf,
    AlbumCovers,
}

#[cfg(test)]
impl AlbumViewStyle {
    fn setting_value(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Bookshelf => "bookshelf",
            Self::AlbumCovers => "album-covers",
        }
    }
}

pub(crate) fn album_view_style(connection: &Connection) -> AlbumViewStyle {
    match crate::db::setting(connection, ALBUM_VIEW_STYLE_SETTING_KEY)
        .ok()
        .flatten()
        .as_deref()
    {
        Some("bookshelf") => AlbumViewStyle::Bookshelf,
        Some("album-covers") => AlbumViewStyle::AlbumCovers,
        Some("default") => AlbumViewStyle::Default,
        _ if crate::db::setting(connection, BOOKSHELF_SETTING_KEY)
            .ok()
            .flatten()
            .as_deref()
            == Some("true") =>
        {
            AlbumViewStyle::Bookshelf
        }
        _ => AlbumViewStyle::Default,
    }
}

pub(crate) fn saved_bool(connection: &Connection, key: &str) -> Option<bool> {
    crate::db::setting(connection, key)
        .ok()
        .flatten()
        .and_then(|value| match value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
}

pub(crate) fn album_appearance(connection: &Connection) -> AlbumAppearance {
    let legacy_style = album_view_style(connection);
    AlbumAppearance {
        bookshelf_enabled: saved_bool(connection, ALBUM_BOOKSHELF_ENABLED_SETTING_KEY)
            .unwrap_or(legacy_style == AlbumViewStyle::Bookshelf),
        covers_enabled: saved_bool(connection, ALBUM_COVERS_ENABLED_SETTING_KEY)
            .unwrap_or(legacy_style != AlbumViewStyle::Default),
        background_index: crate::db::setting(connection, ALBUM_BOOKSHELF_BACKGROUND_SETTING_KEY)
            .ok()
            .flatten()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0),
        cover_index: crate::db::setting(connection, ALBUM_COVER_FRAME_SETTING_KEY)
            .ok()
            .flatten()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0),
    }
}

pub(crate) fn set_bookshelf_enabled(connection: &Connection, enabled: bool) -> anyhow::Result<()> {
    crate::db::set_setting(
        connection,
        ALBUM_BOOKSHELF_ENABLED_SETTING_KEY,
        if enabled { "true" } else { "false" },
    )
}

pub(crate) fn set_covers_enabled(connection: &Connection, enabled: bool) -> anyhow::Result<()> {
    crate::db::set_setting(
        connection,
        ALBUM_COVERS_ENABLED_SETTING_KEY,
        if enabled { "true" } else { "false" },
    )
}

pub(crate) fn next_bookshelf_background(
    connection: &Connection,
    background_count: usize,
) -> anyhow::Result<usize> {
    if background_count == 0 {
        anyhow::bail!("no bookshelf backgrounds are available");
    }
    let next = (album_appearance(connection).background_index + 1) % background_count;
    crate::db::set_setting(
        connection,
        ALBUM_BOOKSHELF_BACKGROUND_SETTING_KEY,
        &next.to_string(),
    )?;
    set_bookshelf_enabled(connection, true)?;
    Ok(next)
}

pub(crate) fn next_album_covers(
    connection: &Connection,
    frame_count: usize,
) -> anyhow::Result<usize> {
    if frame_count == 0 {
        anyhow::bail!("no album cover frames are available");
    }
    let next = (album_appearance(connection).cover_index + 1) % frame_count;
    crate::db::set_setting(connection, ALBUM_COVER_FRAME_SETTING_KEY, &next.to_string())?;
    set_covers_enabled(connection, true)?;
    Ok(next)
}

pub(crate) fn disable_all_album_themes(connection: &Connection) -> anyhow::Result<()> {
    set_bookshelf_enabled(connection, false)?;
    set_covers_enabled(connection, false)
}

/// Forget every theme choice: the switches, both page indexes, the legacy
/// style keys and each album's own cover, so the albums view falls back to its
/// shipped defaults.
pub(crate) fn reset_all_album_themes(connection: &Connection) -> anyhow::Result<()> {
    for key in [
        ALBUM_BOOKSHELF_ENABLED_SETTING_KEY,
        ALBUM_COVERS_ENABLED_SETTING_KEY,
        ALBUM_BOOKSHELF_BACKGROUND_SETTING_KEY,
        ALBUM_COVER_FRAME_SETTING_KEY,
        ALBUM_VIEW_STYLE_SETTING_KEY,
        BOOKSHELF_SETTING_KEY,
    ] {
        crate::db::delete_setting(connection, key)?;
    }
    crate::db::clear_all_album_cover_frames(connection)?;
    crate::db::clear_all_album_cover_photos(connection)?;
    Ok(())
}

#[cfg(test)]
fn set_album_view_style(connection: &Connection, style: AlbumViewStyle) -> anyhow::Result<()> {
    crate::db::set_setting(
        connection,
        ALBUM_VIEW_STYLE_SETTING_KEY,
        style.setting_value(),
    )
}

fn themes_page(
    theme_engine: Rc<crate::window::theme::ThemeEngine>,
) -> (gtk::ScrolledWindow, Rc<dyn Fn()>) {
    let content = page_content("Themes", "Customize theme options.");

    // Appearance: one radio row per theme folder found in the themes folder. The
    // section is rebuilt from a fresh scan every time the settings window is
    // presented, so new folders appear without a restart.
    let appearance_section = gtk::Box::new(gtk::Orientation::Vertical, 12);
    appearance_section.set_hexpand(true);
    content.append(&appearance_section);
    let rebuild_appearance: Rc<dyn Fn()> = {
        let engine = theme_engine.clone();
        let section = appearance_section.clone();
        Rc::new(move || {
            while let Some(child) = section.first_child() {
                section.remove(&child);
            }

            let heading = gtk::Label::new(Some("Appearance"));
            heading.set_halign(gtk::Align::Start);
            heading.set_hexpand(true);
            heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
            heading.add_css_class("heading");
            section.append(&heading);

            let themes = engine.themes();
            let list = settings_list();
            let mut group_leader: Option<gtk::CheckButton> = None;
            for theme in &themes {
                // Radio indicator only: the row title already shows the name.
                // A second label here doubles the row min-width and overflows
                // the Settings window on long theme names.
                let check = gtk::CheckButton::new();
                check.set_valign(gtk::Align::Center);
                check.set_tooltip_text(Some(&theme.id));
                if let Some(leader) = group_leader.as_ref() {
                    check.set_group(Some(leader));
                } else {
                    group_leader = Some(check.clone());
                }
                // Set the active state before connecting the handler so a
                // rebuild never re-applies the already-active theme.
                check.set_active(theme.id == engine.active_id());
                {
                    let engine = engine.clone();
                    let theme = theme.clone();
                    check.connect_toggled(move |button| {
                        if button.is_active() {
                            engine.select(&theme);
                        }
                    });
                }
                append_row(&list, &theme.name, None, Some(check.upcast_ref()));
            }
            append_empty_state(
                &list,
                "No themes found in the themes folder",
                themes.is_empty(),
            );
            section.append(&list);
        })
    };
    rebuild_appearance();

    (scroll_page(content), rebuild_appearance)
}

fn interface_page(
    connection: Rc<RefCell<Connection>>,
    theme_changed: Rc<dyn Fn()>,
    thumbnail_changed: Rc<dyn Fn()>,
    zoom_animations_changed: Rc<dyn Fn()>,
    clear_saved_views: Rc<dyn Fn()>,
) -> gtk::ScrolledWindow {
    let content = page_content(
        "Interface",
        "Customize thumbnail appearance and interface effects.",
    );

    // Thumbnail appearance toggles apply live through thumbnail_changed and
    // are re-read at startup.
    let thumbnail_heading = gtk::Label::new(Some("Thumbnails"));
    thumbnail_heading.set_halign(gtk::Align::Start);
    thumbnail_heading.set_hexpand(true);
    thumbnail_heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
    thumbnail_heading.add_css_class("heading");
    content.append(&thumbnail_heading);

    let thumbnail_list = settings_list();
    let square_corners = gtk::Switch::new();
    square_corners.set_valign(gtk::Align::Center);
    square_corners.set_active(
        saved_bool(
            &connection.borrow(),
            crate::db::THUMBNAIL_SQUARE_CORNERS_SETTING_KEY,
        )
        .unwrap_or(false),
    );
    {
        let connection = connection.clone();
        let thumbnail_changed = thumbnail_changed.clone();
        square_corners.connect_active_notify(move |toggle| {
            let state = toggle.is_active();
            crate::window::debug_log(&format!("SETTINGS: square corners switch -> {state}"));
            if let Err(error) = crate::db::set_setting(
                &connection.borrow(),
                crate::db::THUMBNAIL_SQUARE_CORNERS_SETTING_KEY,
                &state.to_string(),
            ) {
                eprintln!("Could not save square thumbnail corners: {error}");
                return;
            }
            thumbnail_changed();
        });
    }
    append_row(
        &thumbnail_list,
        "Square thumbnail corners",
        Some("Show thumbnails without rounded corners."),
        Some(square_corners.upcast_ref()),
    );

    let fit_whole_photo = gtk::Switch::new();
    fit_whole_photo.set_valign(gtk::Align::Center);
    fit_whole_photo.set_active(
        saved_bool(
            &connection.borrow(),
            crate::db::THUMBNAIL_FIT_WHOLE_PHOTO_SETTING_KEY,
        )
        .unwrap_or(false),
    );
    {
        let connection = connection.clone();
        let thumbnail_changed = thumbnail_changed.clone();
        let theme_changed = theme_changed.clone();
        fit_whole_photo.connect_active_notify(move |toggle| {
            let state = toggle.is_active();
            crate::window::debug_log(&format!("SETTINGS: fit whole photo switch -> {state}"));
            if let Err(error) = crate::db::set_setting(
                &connection.borrow(),
                crate::db::THUMBNAIL_FIT_WHOLE_PHOTO_SETTING_KEY,
                &state.to_string(),
            ) {
                eprintln!("Could not save thumbnail photo fit: {error}");
                return;
            }
            thumbnail_changed();
            theme_changed();
        });
    }
    append_row(
        &thumbnail_list,
        "Show entire photo in thumbnails",
        Some("Letterbox landscape and portrait photos instead of cropping them to the tile."),
        Some(fit_whole_photo.upcast_ref()),
    );

    let show_file_names = gtk::Switch::new();
    show_file_names.set_valign(gtk::Align::Center);
    show_file_names.set_active(
        saved_bool(
            &connection.borrow(),
            crate::db::THUMBNAIL_FILE_NAMES_SETTING_KEY,
        )
        .unwrap_or(false),
    );
    {
        let connection = connection.clone();
        let thumbnail_changed = thumbnail_changed.clone();
        show_file_names.connect_active_notify(move |toggle| {
            let state = toggle.is_active();
            crate::window::debug_log(&format!("SETTINGS: thumbnail filenames switch -> {state}"));
            if let Err(error) = crate::db::set_setting(
                &connection.borrow(),
                crate::db::THUMBNAIL_FILE_NAMES_SETTING_KEY,
                &state.to_string(),
            ) {
                eprintln!("Could not save thumbnail filename setting: {error}");
                return;
            }
            thumbnail_changed();
        });
    }
    append_row(
        &thumbnail_list,
        "Show filenames below thumbnails",
        Some("Display each photo's filename in a single clipped line."),
        Some(show_file_names.upcast_ref()),
    );

    let high_quality = gtk::Switch::new();
    high_quality.set_valign(gtk::Align::Center);
    high_quality.set_active(crate::db::high_quality_thumbnails_enabled(
        &connection.borrow(),
    ));
    {
        let connection = connection.clone();
        let thumbnail_changed = thumbnail_changed.clone();
        high_quality.connect_active_notify(move |toggle| {
            let state = toggle.is_active();
            crate::window::debug_log(&format!("SETTINGS: high quality thumbnails -> {state}"));
            if let Err(error) = crate::db::set_setting(
                &connection.borrow(),
                crate::db::THUMBNAIL_HIGH_QUALITY_SETTING_KEY,
                &state.to_string(),
            ) {
                eprintln!("Could not save high quality thumbnail setting: {error}");
                return;
            }
            thumbnail_changed();
        });
    }
    append_row(
        &thumbnail_list,
        "High quality thumbnails (640 px)",
        Some(
            "Use optional 640 px Photo Wall previews for high-DPI and 4K displays.              Normal thumbnails remain 320 px.",
        ),
        Some(high_quality.upcast_ref()),
    );
    content.append(&thumbnail_list);

    let effects_heading = gtk::Label::new(Some("Effects"));
    effects_heading.set_halign(gtk::Align::Start);
    effects_heading.set_hexpand(true);
    effects_heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
    effects_heading.add_css_class("heading");
    content.append(&effects_heading);

    let effects_list = settings_list();
    let zoom_animations = gtk::Switch::new();
    zoom_animations.set_valign(gtk::Align::Center);
    zoom_animations.set_active(crate::db::zoom_animations_enabled(&connection.borrow()));
    {
        let connection = connection.clone();
        zoom_animations.connect_active_notify(move |toggle| {
            let enabled = toggle.is_active();
            if let Err(error) = crate::db::set_setting(
                &connection.borrow(),
                crate::db::ZOOM_ANIMATIONS_ENABLED_SETTING_KEY,
                &enabled.to_string(),
            ) {
                eprintln!("Could not save zoom animation setting: {error}");
                return;
            }
            zoom_animations_changed();
        });
    }
    append_row(
        &effects_list,
        "Zoom transition animations",
        Some("Animate thumbnails when zooming in or out."),
        Some(zoom_animations.upcast_ref()),
    );
    content.append(&effects_list);

    let views_list = settings_list();
    let clear_views = gtk::Button::with_label("Clear all saved views");
    clear_views.set_valign(gtk::Align::Center);
    clear_views.connect_clicked(move |_| clear_saved_views());
    append_row(
        &views_list,
        "Saved gallery views",
        Some("Reset the view for every section to Grid."),
        Some(clear_views.upcast_ref()),
    );
    content.append(&views_list);

    scroll_page(content)
}

fn package_type() -> &'static str {
    if std::env::var_os("FLATPAK_ID").is_some() {
        "Flatpak"
    } else if std::env::var_os("APPIMAGE").is_some() {
        "AppImage"
    } else {
        "Native / Cargo"
    }
}

fn about_page() -> gtk::ScrolledWindow {
    let content = page_content(
        "About PIC",
        "Build and package information for this installation of PIC.",
    );
    let list = settings_list();
    append_row(&list, "Version", Some(env!("CARGO_PKG_VERSION")), None);
    append_row(&list, "Build", Some(env!("PIC_BUILD_REVISION")), None);
    append_row(&list, "Build date", Some(env!("PIC_BUILD_DATE")), None);
    append_row(&list, "Package", Some(package_type()), None);
    append_row(&list, "Architecture", Some(std::env::consts::ARCH), None);
    content.append(&list);
    scroll_page(content)
}

fn page_content(title: &str, subtitle: &str) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_hexpand(true);
    content.set_margin_top(28);
    content.set_margin_bottom(28);
    content.set_margin_start(28);
    content.set_margin_end(28);
    let title = gtk::Label::new(Some(title));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.add_css_class("title-1");
    let subtitle = gtk::Label::new(Some(subtitle));
    subtitle.set_xalign(0.0);
    subtitle.set_hexpand(true);
    subtitle.set_wrap(true);
    subtitle.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    subtitle.add_css_class("dim-label");
    content.append(&title);
    content.append(&subtitle);
    content
}

fn settings_list() -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_hexpand(true);
    list.add_css_class("boxed-list");
    list
}

fn append_row(
    list: &gtk::ListBox,
    title: &str,
    subtitle: Option<&str>,
    action: Option<&gtk::Widget>,
) -> Option<gtk::Label> {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    box_.set_hexpand(true);
    box_.set_margin_top(10);
    box_.set_margin_bottom(10);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(title));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    labels.append(&title);
    let subtitle_label = if let Some(subtitle) = subtitle {
        let subtitle = gtk::Label::new(Some(subtitle));
        subtitle.set_xalign(0.0);
        subtitle.set_hexpand(true);
        subtitle.set_wrap(true);
        subtitle.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        subtitle.set_selectable(true);
        subtitle.add_css_class("dim-label");
        labels.append(&subtitle);
        Some(subtitle)
    } else {
        None
    };
    box_.append(&labels);
    if let Some(action) = action {
        action.set_valign(gtk::Align::Center);
        box_.append(action);
    }
    row.set_child(Some(&box_));
    list.append(&row);
    subtitle_label
}

fn append_empty_state(list: &gtk::ListBox, message: &str, empty: bool) {
    if empty {
        append_row(list, message, None, None);
    }
}

/// A statistics row whose value starts as "…" and is filled in once the
/// measurement behind it finishes.
fn stat_row(list: &gtk::ListBox, name: &str) -> gtk::Label {
    append_row(list, name, Some("…"), None).expect("statistic value row has a value label")
}

fn scroll_page(content: gtk::Box) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new();
    // Vertical scrolling only: horizontal space is whatever the pane has.
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&content));
    scroll
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.bytes().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit as char);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::{cleanup_result_text, format_bytes, format_count};

    #[test]
    fn byte_sizes_are_human_readable() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn counts_use_thousands_separators() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1,000");
        assert_eq!(format_count(66_037), "66,037");
        assert_eq!(format_count(71_867_123), "71,867,123");
    }

    #[test]
    fn cleanup_results_are_reported_as_a_receipt() {
        use crate::thumbnail::CacheCleanup;

        assert_eq!(
            cleanup_result_text(&CacheCleanup {
                valid: 66_037,
                removed: 5_830,
                bytes_freed: 84_300_000,
            }),
            "Removed 5,830 unused thumbnails · Freed 80.4 MB"
        );
        assert_eq!(
            cleanup_result_text(&CacheCleanup {
                valid: 1,
                removed: 1,
                bytes_freed: 1_048_576,
            }),
            "Removed 1 unused thumbnail · Freed 1.0 MB"
        );
        assert_eq!(
            cleanup_result_text(&CacheCleanup {
                valid: 66_037,
                removed: 0,
                bytes_freed: 0,
            }),
            "No unused thumbnails"
        );
    }

    #[test]
    fn album_view_style_defaults_and_migrates_the_bookshelf_toggle() {
        use super::*;

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();
        assert_eq!(album_view_style(&connection), AlbumViewStyle::Default);

        crate::db::set_setting(&connection, BOOKSHELF_SETTING_KEY, "true").unwrap();
        assert_eq!(album_view_style(&connection), AlbumViewStyle::Bookshelf);

        set_album_view_style(&connection, AlbumViewStyle::AlbumCovers).unwrap();
        assert_eq!(album_view_style(&connection), AlbumViewStyle::AlbumCovers);
        assert_eq!(
            crate::db::setting(&connection, ALBUM_VIEW_STYLE_SETTING_KEY)
                .unwrap()
                .as_deref(),
            Some("album-covers"),
        );

        set_album_view_style(&connection, AlbumViewStyle::Default).unwrap();
        assert_eq!(album_view_style(&connection), AlbumViewStyle::Default);
    }

    #[test]
    fn album_appearance_migrates_existing_styles_to_independent_options() {
        use super::*;

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();

        crate::db::set_setting(&connection, ALBUM_VIEW_STYLE_SETTING_KEY, "bookshelf").unwrap();
        assert_eq!(
            album_appearance(&connection),
            AlbumAppearance {
                bookshelf_enabled: true,
                covers_enabled: true,
                background_index: 0,
                cover_index: 0,
            }
        );

        crate::db::set_setting(&connection, ALBUM_VIEW_STYLE_SETTING_KEY, "album-covers").unwrap();
        assert_eq!(
            album_appearance(&connection),
            AlbumAppearance {
                bookshelf_enabled: false,
                covers_enabled: true,
                background_index: 0,
                cover_index: 0,
            }
        );
    }

    #[test]
    fn bookshelf_and_covers_can_be_changed_independently() {
        use super::*;

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();

        set_bookshelf_enabled(&connection, true).unwrap();
        set_covers_enabled(&connection, false).unwrap();
        assert_eq!(
            album_appearance(&connection),
            AlbumAppearance {
                bookshelf_enabled: true,
                covers_enabled: false,
                background_index: 0,
                cover_index: 0,
            }
        );

        set_bookshelf_enabled(&connection, false).unwrap();
        set_covers_enabled(&connection, true).unwrap();
        assert_eq!(
            album_appearance(&connection),
            AlbumAppearance {
                bookshelf_enabled: false,
                covers_enabled: true,
                background_index: 0,
                cover_index: 0,
            }
        );
    }

    #[test]
    fn next_bookshelf_background_enables_bookshelf_and_wraps() {
        use super::*;

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();

        assert_eq!(next_bookshelf_background(&connection, 4).unwrap(), 1);
        assert!(album_appearance(&connection).bookshelf_enabled);
        assert_eq!(next_bookshelf_background(&connection, 4).unwrap(), 2);
        assert_eq!(next_bookshelf_background(&connection, 4).unwrap(), 3);
        assert_eq!(next_bookshelf_background(&connection, 4).unwrap(), 0);
    }

    #[test]
    fn next_album_covers_enables_covers_and_wraps() {
        use super::*;

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();

        assert!(!album_appearance(&connection).covers_enabled);
        assert_eq!(next_album_covers(&connection, 3).unwrap(), 1);
        let appearance = album_appearance(&connection);
        assert!(appearance.covers_enabled);
        assert_eq!(appearance.cover_index, 1);
        assert_eq!(next_album_covers(&connection, 3).unwrap(), 2);
        assert_eq!(next_album_covers(&connection, 3).unwrap(), 0);

        assert!(next_album_covers(&connection, 0).is_err());
    }

    #[test]
    fn disabling_all_themes_preserves_the_selected_designs() {
        use super::*;

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();

        set_bookshelf_enabled(&connection, true).unwrap();
        set_covers_enabled(&connection, true).unwrap();
        next_bookshelf_background(&connection, 4).unwrap();
        next_album_covers(&connection, 4).unwrap();
        disable_all_album_themes(&connection).unwrap();

        assert_eq!(
            album_appearance(&connection),
            AlbumAppearance {
                bookshelf_enabled: false,
                covers_enabled: false,
                background_index: 1,
                cover_index: 1,
            }
        );
    }

    #[test]
    fn resetting_all_themes_clears_every_custom_choice() {
        use super::*;

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE albums (
                   id INTEGER PRIMARY KEY,
                   name TEXT NOT NULL,
                   created_at INTEGER NOT NULL,
                   cover_frame TEXT,
                   cover_photo_id INTEGER
                 );
                 INSERT INTO albums (id, name, created_at, cover_frame, cover_photo_id)
                 VALUES (1, 'Styled', 0, 'vintage/blue-frame.png', 4);",
            )
            .unwrap();

        set_bookshelf_enabled(&connection, true).unwrap();
        set_covers_enabled(&connection, true).unwrap();
        next_bookshelf_background(&connection, 4).unwrap();
        next_album_covers(&connection, 4).unwrap();
        set_album_view_style(&connection, AlbumViewStyle::Bookshelf).unwrap();
        crate::db::set_setting(&connection, BOOKSHELF_SETTING_KEY, "true").unwrap();

        reset_all_album_themes(&connection).unwrap();

        assert_eq!(album_appearance(&connection), AlbumAppearance::default());
        assert_eq!(album_view_style(&connection), AlbumViewStyle::Default);
        for key in [
            ALBUM_BOOKSHELF_ENABLED_SETTING_KEY,
            ALBUM_COVERS_ENABLED_SETTING_KEY,
            ALBUM_BOOKSHELF_BACKGROUND_SETTING_KEY,
            ALBUM_COVER_FRAME_SETTING_KEY,
            ALBUM_VIEW_STYLE_SETTING_KEY,
            BOOKSHELF_SETTING_KEY,
        ] {
            assert_eq!(crate::db::setting(&connection, key).unwrap(), None, "{key}");
        }
        assert_eq!(
            connection
                .query_row("SELECT cover_frame FROM albums WHERE id = 1", [], |row| {
                    row.get::<_, Option<String>>(0)
                })
                .unwrap(),
            None
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT cover_photo_id FROM albums WHERE id = 1",
                    [],
                    |row| { row.get::<_, Option<i64>>(0) }
                )
                .unwrap(),
            None
        );
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn album_appearance_controls_persist_and_notify() {
        use super::*;

        fn find_controls(
            widget: &gtk::Widget,
            switches: &mut Vec<gtk::Switch>,
            buttons: &mut Vec<gtk::Button>,
        ) {
            if let Some(switch) = widget.downcast_ref::<gtk::Switch>() {
                switches.push(switch.clone());
            }
            if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                buttons.push(button.clone());
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                find_controls(&widget, switches, buttons);
                child = widget.next_sibling();
            }
        }

        gtk::init().unwrap();
        let path = std::env::temp_dir().join(format!(
            "pic-bookshelf-setting-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Rc::new(RefCell::new(Connection::open(&path).unwrap()));
        connection
            .borrow()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS albums (
                   id INTEGER PRIMARY KEY,
                   name TEXT NOT NULL,
                   created_at INTEGER NOT NULL,
                   cover_frame TEXT,
                   cover_photo_id INTEGER
                 );",
            )
            .unwrap();
        let notified = Rc::new(Cell::new(0));
        let notified_for_callback = notified.clone();
        let animation_notified = Rc::new(Cell::new(0));
        let animation_notified_for_callback = animation_notified.clone();
        let page = interface_page(
            connection.clone(),
            Rc::new(move || notified_for_callback.set(notified_for_callback.get() + 1)),
            Rc::new(|| {}),
            Rc::new(move || {
                animation_notified_for_callback.set(animation_notified_for_callback.get() + 1)
            }),
            Rc::new(|| {}),
        );
        let mut switches = Vec::new();
        let mut buttons = Vec::new();
        find_controls(page.upcast_ref(), &mut switches, &mut buttons);
        assert_eq!(
            buttons
                .iter()
                .filter_map(|button| button.label())
                .collect::<Vec<_>>(),
            [
                "Next Background",
                "Next Album Covers",
                "Disable All Themes",
                "Reset All Theme Settings",
            ],
        );
        // Switch order: three thumbnail toggles, zoom animations, then
        // bookshelf and album covers at the end.
        assert_eq!(switches.len(), 6);
        assert!(switches[3].is_active());
        switches[3].set_active(false);
        assert_eq!(animation_notified.get(), 1);
        assert!(!crate::db::zoom_animations_enabled(&connection.borrow()));
        let album_switches = &switches[switches.len() - 2..];
        assert_eq!(album_switches.len(), 2);
        assert!(!album_switches[0].is_active());
        assert!(!album_switches[1].is_active());

        album_switches[1].set_active(true);
        assert_eq!(notified.get(), 1);
        buttons[0].emit_clicked();
        assert_eq!(notified.get(), 2);
        assert!(album_switches[0].is_active());
        assert_eq!(
            album_appearance(&connection.borrow()),
            AlbumAppearance {
                bookshelf_enabled: true,
                covers_enabled: true,
                background_index: 1,
                cover_index: 0,
            }
        );
        buttons[1].emit_clicked();
        assert_eq!(notified.get(), 3);
        assert_eq!(
            album_appearance(&connection.borrow()),
            AlbumAppearance {
                bookshelf_enabled: true,
                covers_enabled: true,
                background_index: 1,
                cover_index: 1,
            }
        );
        buttons[2].emit_clicked();
        assert_eq!(notified.get(), 4);
        assert!(!buttons[2].is_sensitive());

        buttons[3].emit_clicked();
        assert_eq!(notified.get(), 5);
        assert!(!album_switches[0].is_active());
        assert!(!album_switches[1].is_active());
        assert_eq!(
            album_appearance(&connection.borrow()),
            AlbumAppearance::default()
        );
        drop(page);
        drop(connection);

        let reopened = Connection::open(&path).unwrap();
        assert_eq!(album_appearance(&reopened), AlbumAppearance::default());
        drop(reopened);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn interface_clear_saved_views_invokes_only_the_reset_callback() {
        use super::*;
        fn find_reset(widget: &gtk::Widget) -> Option<gtk::Button> {
            if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                if button.label().as_deref() == Some("Clear all saved views") {
                    return Some(button.clone());
                }
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if let Some(button) = find_reset(&widget) {
                    return Some(button);
                }
                child = widget.next_sibling();
            }
            None
        }
        gtk::init().unwrap();
        let connection = Rc::new(RefCell::new(Connection::open_in_memory().unwrap()));
        connection
            .borrow()
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();
        crate::db::set_setting(&connection.borrow(), "grid-thumbnail-size", "219").unwrap();
        let resets = Rc::new(Cell::new(0));
        let resets_for_callback = resets.clone();
        let page = interface_page(
            connection.clone(),
            Rc::new(|| panic!("reset changed theme settings")),
            Rc::new(|| panic!("reset changed thumbnails")),
            Rc::new(|| panic!("reset changed animations")),
            Rc::new(move || resets_for_callback.set(resets_for_callback.get() + 1)),
        );
        find_reset(page.upcast_ref())
            .expect("Interface reset button missing")
            .emit_clicked();
        assert_eq!(resets.get(), 1);
        assert_eq!(
            crate::db::setting(&connection.borrow(), "grid-thumbnail-size")
                .unwrap()
                .as_deref(),
            Some("219")
        );
    }
}
