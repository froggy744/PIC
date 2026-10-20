use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use rusqlite::Connection;

use super::{DatabaseManagement, LibraryMaintenance};

pub(super) fn page(
    parent: &adw::Window,
    connection: Rc<RefCell<Connection>>,
    maintenance: LibraryMaintenance,
    management: DatabaseManagement,
) -> gtk::ScrolledWindow {
    let content = super::page_content(
        "Libraries",
        "Manage PIC libraries, backups, thumbnail storage, and library maintenance.",
    );
    let current = gtk::Label::new(None);
    current.set_xalign(0.0);
    current.set_hexpand(true);
    current.set_wrap(true);
    current.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    current.set_max_width_chars(64);
    current.add_css_class("title-4");
    content.append(&current);

    let details_title = gtk::Label::new(Some("Library details"));
    details_title.set_xalign(0.0);
    details_title.add_css_class("heading");
    content.append(&details_title);

    let details = super::settings_list();
    let name = gtk::Entry::new();
    let description = gtk::Entry::new();
    super::append_row(
        &details,
        "Name",
        Some("A memorable catalog name."),
        Some(name.upcast_ref()),
    );
    super::append_row(
        &details,
        "Description",
        Some("For example, the people or work represented by this library."),
        Some(description.upcast_ref()),
    );
    let save_details = gtk::Button::with_label("Rename Database");
    super::append_row(
        &details,
        "Library details",
        None,
        Some(save_details.upcast_ref()),
    );
    content.append(&details);

    let actions_heading = gtk::Label::new(Some("Database files"));
    actions_heading.set_xalign(0.0);
    actions_heading.add_css_class("heading");
    content.append(&actions_heading);

    let actions = gtk::Grid::new();
    actions.set_column_spacing(8);
    actions.set_row_spacing(8);
    actions.set_hexpand(true);

    let create = gtk::Button::with_label("Create Library");
    let open = gtk::Button::with_label("Open Existing…");
    let backup = gtk::Button::with_label("Back Up");
    let restore = gtk::Button::with_label("Restore Backup…");
    for button in [&create, &open, &backup, &restore] {
        button.set_hexpand(true);
    }
    actions.attach(&create, 0, 0, 1, 1);
    actions.attach(&open, 1, 0, 1, 1);
    actions.attach(&backup, 0, 1, 1, 1);
    actions.attach(&restore, 1, 1, 1, 1);
    content.append(&actions);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("dim-label");
    content.append(&status);

    let known_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    known_header.set_hexpand(true);
    let known_title = gtk::Label::new(Some("Known libraries"));
    known_title.set_xalign(0.0);
    known_title.add_css_class("heading");
    known_title.set_hexpand(true);
    known_header.append(&known_title);
    let show_db_files = gtk::Button::with_label("Show DB Files");
    show_db_files.set_tooltip_text(Some(
        "List database and backup files in PIC’s default data folder.",
    ));
    known_header.append(&show_db_files);
    content.append(&known_header);
    let known = gtk::ListBox::new();
    known.add_css_class("boxed-list");
    content.append(&known);

    let clean_old_libraries_button = gtk::Button::with_label("Clean Old DBs");
    clean_old_libraries_button.set_valign(gtk::Align::Center);
    let old_libraries_actions = super::settings_list();
    super::append_row(
        &old_libraries_actions,
        "Old databases",
        Some("Remove other databases from PIC’s list. Their files stay on disk and can be opened again."),
        Some(clean_old_libraries_button.upcast_ref()),
    );
    content.append(&old_libraries_actions);

    let thumbnail_title = gtk::Label::new(Some("Thumbnail cache"));
    thumbnail_title.set_xalign(0.0);
    thumbnail_title.add_css_class("heading");
    content.append(&thumbnail_title);

    let thumbnail_actions = super::settings_list();

    let clean_button = gtk::Button::with_label("Clean Cache");
    clean_button.set_valign(gtk::Align::Center);
    super::append_row(
        &thumbnail_actions,
        "Thumbnail maintenance",
        Some("Remove thumbnails not referenced by this library. The cache is shared, so other libraries may regenerate them."),
        Some(clean_button.upcast_ref()),
    );

    let clear_thumbnails_button = gtk::Button::with_label("Clear");
    clear_thumbnails_button.set_valign(gtk::Align::Center);
    clear_thumbnails_button.add_css_class("clear-action-button");
    super::append_row(
        &thumbnail_actions,
        "Clear thumbnails",
        Some("Delete every cached thumbnail. Photos and the database remain."),
        Some(clear_thumbnails_button.upcast_ref()),
    );
    content.append(&thumbnail_actions);

    let thumbnail_status = gtk::Label::new(None);
    thumbnail_status.set_xalign(0.0);
    thumbnail_status.set_hexpand(true);
    thumbnail_status.set_wrap(true);
    thumbnail_status.add_css_class("dim-label");
    content.append(&thumbnail_status);

    let cleanup_running = Rc::new(Cell::new(false));
    {
        let connection = connection.clone();
        let thumbnail_status = thumbnail_status.clone();
        let button_for_click = clean_button.clone();
        let cleanup_running = cleanup_running.clone();
        button_for_click.connect_clicked(move |button| {
            if cleanup_running.replace(true) {
                return;
            }
            maintenance_trace("action=clean_cache phase=start");
            button.set_sensitive(false);
            thumbnail_status.set_text("Cleaning thumbnail cache…");

            let database_path = match crate::db::connection_path(&connection.borrow()) {
                Ok(path) => path,
                Err(error) => {
                    eprintln!("Could not identify the active library database: {error:#}");
                    thumbnail_status.set_text(&format!(
                        "Could not identify the active library database: {error:#}"
                    ));
                    button.set_sensitive(true);
                    cleanup_running.set(false);
                    return;
                }
            };

            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let result = crate::db::open_existing(&database_path)
                    .and_then(|database| crate::thumbnail::valid_cache_paths(&database))
                    .and_then(|valid| {
                        if std::env::var_os("PIC_LIBRARY_MAINTENANCE_TRACE").is_some() {
                            eprintln!(
                                "PIC_LIBRARY_MAINTENANCE action=clean_cache phase=keys_ready paths={}",
                                valid.len()
                            );
                        }
                        crate::thumbnail::cleanup_cache(&valid)
                    });
                let _ = sender.send(result);
            });

            let poll_status = thumbnail_status.clone();
            let poll_button = button.clone();
            let poll_running = cleanup_running.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                match receiver.try_recv() {
                    Ok(Ok(cleanup)) => {
                        maintenance_trace(&format!(
                            "action=clean_cache phase=complete valid={} removed={} bytes_freed={}",
                            cleanup.valid, cleanup.removed, cleanup.bytes_freed
                        ));
                        poll_status.set_text(&super::cleanup_result_text(&cleanup));
                        poll_button.set_sensitive(true);
                        poll_running.set(false);
                        glib::ControlFlow::Break
                    }
                    Ok(Err(error)) => {
                        maintenance_trace(&format!("action=clean_cache phase=failed error={error:#}"));
                        eprintln!("Could not clean thumbnail cache: {error:#}");
                        poll_status.set_text(&format!("Could not clean the thumbnail cache: {error:#}"));
                        poll_button.set_sensitive(true);
                        poll_running.set(false);
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        maintenance_trace("action=clean_cache phase=worker_disconnected");
                        poll_status.set_text("Could not clean the thumbnail cache: background operation stopped.");
                        poll_button.set_sensitive(true);
                        poll_running.set(false);
                        glib::ControlFlow::Break
                    }
                }
            });
        });
    }

    let maintenance_title = gtk::Label::new(Some("Library maintenance"));
    maintenance_title.set_xalign(0.0);
    maintenance_title.add_css_class("heading");
    content.append(&maintenance_title);

    let maintenance_list = super::settings_list();

    let clear_database_button = gtk::Button::with_label("Clear");
    clear_database_button.set_valign(gtk::Align::Center);
    clear_database_button.add_css_class("clear-action-button");
    super::append_row(
        &maintenance_list,
        "Clear database",
        Some("Remove indexed photos, album links, and their edit history. Registered folders remain."),
        Some(clear_database_button.upcast_ref()),
    );

    let clear_all_button = gtk::Button::with_label("Clear All");
    clear_all_button.set_valign(gtk::Align::Center);
    clear_all_button.add_css_class("clear-action-button");
    super::append_row(
        &maintenance_list,
        "Clear all",
        Some("Delete indexed photos, albums, registered folders, and cached thumbnails."),
        Some(clear_all_button.upcast_ref()),
    );

    content.append(&maintenance_list);

    {
        let parent = parent.clone();
        let thumbnail_status = thumbnail_status.clone();
        clear_thumbnails_button.connect_clicked(move |button| {
            let button = button.clone();
            let thumbnail_status = thumbnail_status.clone();
            super::confirm_destructive(
                &parent,
                "Clear thumbnails?",
                "The shared thumbnail cache will be deleted. Other libraries may need to regenerate thumbnails. Your photos and database will remain.",
                Rc::new(move || {
                    maintenance_trace("action=clear_thumbnails phase=confirmed");
                    clear_thumbnail_cache_async(
                        button.clone(),
                        thumbnail_status.clone(),
                        "clear_thumbnails",
                        "Thumbnail cache cleared. Reopen Stats to refresh cache totals.",
                        "Could not fully clear thumbnail cache: ",
                    );
                }),
            );
        });
    }

    {
        let maintenance = maintenance.clone();
        let parent = parent.clone();
        let thumbnail_status = thumbnail_status.clone();
        clear_database_button.connect_clicked(move |_| {
            let maintenance = maintenance.clone();
            let thumbnail_status = thumbnail_status.clone();
            super::confirm_destructive(
                &parent,
                "Clear database?",
                "Indexed photos, album links, and their edit history will be removed. Registered folders will remain.",
                Rc::new(move || {
                    maintenance_trace("action=clear_database phase=confirmed");
                    match (maintenance.clear_database)() {
                        Ok(()) => thumbnail_status.set_text(
                            "Indexed photos, album links, and their edit history removed. Registered folders remain.",
                        ),
                        Err(error) => thumbnail_status
                            .set_text(&format!("Could not clear library database: {error}")),
                    }
                }),
            );
        });
    }

    {
        let maintenance = maintenance.clone();
        let parent = parent.clone();
        let thumbnail_status = thumbnail_status.clone();
        clear_all_button.connect_clicked(move |button| {
            let button = button.clone();
            let maintenance = maintenance.clone();
            let thumbnail_status = thumbnail_status.clone();
            super::confirm_destructive(
                &parent,
                "Clear library data?",
                "Indexed photos, albums, registered folders, and the shared thumbnail cache will be cleared. Original photo files and library preferences will remain.",
                Rc::new(move || {
                    maintenance_trace("action=clear_all phase=confirmed");
                    match (maintenance.clear_all)() {
                        Ok(()) => clear_thumbnail_cache_async(
                            button.clone(),
                            thumbnail_status.clone(),
                            "clear_all",
                            "Library records and thumbnail cache cleared.",
                            "Library records were cleared, but thumbnail cache clearing failed: ",
                        ),
                        Err(error) => thumbnail_status
                            .set_text(&format!("Could not clear library: {error}")),
                    }
                }),
            );
        });
    }

    let refresh: Rc<dyn Fn()> = {
        let parent = parent.clone();
        let current = current.clone();
        let name = name.clone();
        let description = description.clone();
        let known = known.clone();
        let management = management.clone();
        let status = status.clone();
        Rc::new(move || {
            refresh_page(
                &parent,
                &current,
                &name,
                &description,
                &known,
                &status,
                &management,
            )
        })
    };
    refresh();

    {
        let parent = parent.clone();
        let status = status.clone();
        let management = management.clone();
        let refresh = refresh.clone();
        show_db_files.connect_clicked(move |_| {
            match crate::db::default_library_database_files() {
                Ok(files) => show_default_database_files(
                    &parent,
                    files,
                    management.clone(),
                    status.clone(),
                    refresh.clone(),
                ),
                Err(error) => status.set_text(&format!(
                    "Could not list database files in PIC’s default data folder: {error:#}"
                )),
            }
        });
    }

    {
        let parent = parent.clone();
        let status = status.clone();
        let current_label = current.clone();
        let name = name.clone();
        let description = description.clone();
        let known = known.clone();
        let management = management.clone();
        clean_old_libraries_button.connect_clicked(move |_| {
            let (active, libraries) = match crate::db::active_library()
                .and_then(|active| crate::db::known_libraries().map(|libraries| (active, libraries)))
            {
                Ok(result) => result,
                Err(error) => {
                    status.set_text(&format!("Could not inspect registered libraries: {error:#}"));
                    return;
                }
            };
            let old_count = libraries
                .iter()
                .filter(|library| library.id != active.id)
                .count();
            if old_count == 0 {
                status.set_text("There are no old databases to remove.");
                return;
            }

            let message = format!(
                "Remove {old_count} other database(s) from PIC’s library list? Their database files will not be deleted. You can add them again with Open Existing. The current library will remain registered."
            );
            let status_for_action = status.clone();
            let parent_for_action = parent.clone();
            let current_for_action = current_label.clone();
            let name_for_action = name.clone();
            let description_for_action = description.clone();
            let known_for_action = known.clone();
            let management_for_action = management.clone();
            super::confirm_destructive(
                &parent,
                "Clean old databases?",
                &message,
                Rc::new(move || match crate::db::remove_old_libraries_from_registry() {
                    Ok(removed) => {
                        status_for_action.set_text(&format!(
                            "Removed {} old database(s) from PIC’s list. Their files remain on disk.",
                            removed.len()
                        ));
                        refresh_page(
                            &parent_for_action,
                            &current_for_action,
                            &name_for_action,
                            &description_for_action,
                            &known_for_action,
                            &status_for_action,
                            &management_for_action,
                        );
                    }
                    Err(error) => status_for_action
                        .set_text(&format!("Could not clean old databases: {error:#}")),
                }),
            );
        });
    }

    {
        let name = name.clone();
        let description = description.clone();
        let status = status.clone();
        let refresh = refresh.clone();
        save_details.connect_clicked(move |_| {
            match crate::db::active_library().and_then(|library| {
                crate::db::update_library_details(&library.id, &name.text(), &description.text())
            }) {
                Ok(_) => {
                    status.set_text("Library details saved.");
                    refresh();
                }
                Err(error) => {
                    status.set_text(&format!("Could not save library details: {error:#}"))
                }
            }
        });
    }

    {
        let parent = parent.clone();
        let status = status.clone();
        let refresh = refresh.clone();
        let management = management.clone();
        create.connect_clicked(move |_| {
            create_dialog(&parent, status.clone(), refresh.clone(), management.clone());
        });
    }

    {
        let parent = parent.clone();
        let status = status.clone();
        let refresh = refresh.clone();
        let management = management.clone();
        open.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("SQLite databases"));
            filter.add_pattern("*.db");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let chooser = gtk::FileDialog::builder()
                .title("Open PIC Database")
                .accept_label("Open")
                .filters(&filters)
                .modal(true)
                .build();
            let status = status.clone();
            let refresh = refresh.clone();
            let management = management.clone();
            let parent_for_result = parent.clone();
            chooser.open(Some(&parent), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        let display_name = file_stem(&path);
                        let library = crate::db::known_libraries().and_then(|libraries| {
                            libraries
                                .into_iter()
                                .find(|library| library.path == path)
                                .map(Ok)
                                .unwrap_or_else(|| {
                                    crate::db::add_existing_library(&path, &display_name, "")
                                })
                        });
                        match library.and_then(|library| {
                            (management.switch_library)(&library.id)
                                .map_err(anyhow::Error::msg)?;
                            Ok(library)
                        })
                        {
                            Ok(library) => {
                                status.set_text(&format!("Opened {}.", library.name));
                                refresh();
                            }
                            Err(error) => show_library_error(
                                &parent_for_result,
                                &status,
                                "Could not open library",
                                &format!("{error:#}"),
                            ),
                        }
                    }
                }
            });
        });
    }

    {
        let parent = parent.clone();
        let status = status.clone();
        backup.connect_clicked(move |button| {
            let Ok(library) = crate::db::active_library() else {
                status.set_text("Could not identify the current library.");
                return;
            };
            let chooser = gtk::FileDialog::builder()
                .title("Back Up Library")
                .accept_label("Back Up")
                .initial_name(&format!(
                    "PIC-library-backup-{}.db",
                    chrono::Local::now().format("%Y-%m-%d-%H%M%S")
                ))
                .modal(true)
                .build();
            if let Some(folder) = library_folder(&library) {
                chooser.set_initial_folder(Some(&gio::File::for_path(folder)));
            }
            let button = button.clone();
            let status = status.clone();
            chooser.save(Some(&parent), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(mut destination) = file.path() {
                        if destination.extension().is_none() {
                            destination.set_extension("db");
                        }
                        button.set_sensitive(false);
                        status.set_text("Backing up database…");
                        let (sender, receiver) = std::sync::mpsc::channel();
                        std::thread::spawn(move || {
                            let _ = sender.send(
                                crate::db::backup_library_to_path(&library, &destination)
                                    .map_err(|error| error.to_string()),
                            );
                        });
                        let button = button.clone();
                        let status = status.clone();
                        glib::timeout_add_local(Duration::from_millis(50), move || {
                            match receiver.try_recv() {
                                Ok(Ok(path)) => {
                                    status.set_text(&format!("Backup saved to {}", path.display()));
                                    button.set_sensitive(true);
                                    glib::ControlFlow::Break
                                }
                                Ok(Err(error)) => {
                                    status.set_text(&format!("Backup failed: {error}"));
                                    button.set_sensitive(true);
                                    glib::ControlFlow::Break
                                }
                                Err(std::sync::mpsc::TryRecvError::Empty) => {
                                    glib::ControlFlow::Continue
                                }
                                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                    button.set_sensitive(true);
                                    glib::ControlFlow::Break
                                }
                            }
                        });
                    }
                }
            });
        });
    }

    {
        let parent = parent.clone();
        let status = status.clone();
        let refresh = refresh.clone();
        let management = management.clone();
        restore.connect_clicked(move |_| {
            restore_dialog(&parent, status.clone(), refresh.clone(), management.clone());
        });
    }

    super::scroll_page(content)
}

fn refresh_page(
    parent: &adw::Window,
    current_label: &gtk::Label,
    name: &gtk::Entry,
    description: &gtk::Entry,
    list: &gtk::ListBox,
    status: &gtk::Label,
    management: &DatabaseManagement,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let active = crate::db::active_library().ok();
    if let Some(active) = active.as_ref() {
        current_label.set_text(&format!(
            "Current: {} — {}",
            active.name,
            active.path.display()
        ));
        name.set_text(&active.name);
        description.set_text(&active.description);
    }
    for library in crate::db::known_libraries().unwrap_or_default() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_hexpand(true);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(12);
        row.set_margin_end(12);
        let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let title = if active
            .as_ref()
            .is_some_and(|active| active.id == library.id)
        {
            format!("{} (Current)", library.name)
        } else {
            library.name.clone()
        };
        let title = gtk::Label::new(Some(&title));
        title.set_xalign(0.0);
        labels.append(&title);
        if !library.description.is_empty() {
            let description = gtk::Label::new(Some(&library.description));
            description.set_xalign(0.0);
            description.add_css_class("dim-label");
            labels.append(&description);
        }
        let path = gtk::Label::new(Some(&library.path.display().to_string()));
        path.set_xalign(0.0);
        path.set_hexpand(true);
        path.set_max_width_chars(48);
        path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        path.add_css_class("dim-label");
        labels.append(&path);
        labels.set_hexpand(true);
        row.append(&labels);
        let switch = gtk::Button::with_label("Open");
        switch.set_sensitive(
            !active
                .as_ref()
                .is_some_and(|active| active.id == library.id),
        );
        let id = library.id.clone();
        let management = management.clone();
        let current_label = current_label.clone();
        let name = name.clone();
        let description = description.clone();
        let list_for_switch = list.clone();
        let status = status.clone();
        let parent = parent.clone();
        let library_name = library.name.clone();
        let library_path = library.path.clone();
        switch.connect_clicked(move |_| match (management.switch_library)(&id) {
            Ok(()) => {
                status.set_text("Library switched.");
                refresh_page(
                    &parent,
                    &current_label,
                    &name,
                    &description,
                    &list_for_switch,
                    &status,
                    &management,
                );
            }
            Err(error) => {
                if !library_path.is_file() {
                    show_library_error(
                        &parent,
                        &status,
                        "Library not found",
                        &format!(
                            "{library_name} could not be found at:\n{}\n\nIf it was moved, use Open Existing… to select its new location.",
                            library_path.display()
                        ),
                    );
                } else {
                    show_library_error(
                        &parent,
                        &status,
                        "Could not switch library",
                        &error,
                    );
                }
            }
        });
        row.append(&switch);
        list.append(&row);
    }
}

fn show_library_error(parent: &adw::Window, status: &gtk::Label, heading: &str, message: &str) {
    status.set_text(message);
    let dialog = adw::AlertDialog::new(Some(heading), Some(message));
    dialog.add_response("close", "Close");
    dialog.set_default_response(Some("close"));
    dialog.set_close_response("close");
    dialog.present(Some(parent));
}

fn show_default_database_files(
    parent: &adw::Window,
    files: Vec<PathBuf>,
    management: DatabaseManagement,
    status: gtk::Label,
    refresh: Rc<dyn Fn()>,
) {
    let window = gtk::Window::builder()
        .title("Library and Backup Databases")
        .transient_for(parent)
        .modal(true)
        .default_width(680)
        .default_height(520)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);

    let root = dirs::data_dir().map(|directory| directory.join(crate::app_paths::APP_DIRECTORY));
    let description = match root {
        Some(path) => format!(
            "{} database file(s) found under {}",
            files.len(),
            path.display()
        ),
        None => format!(
            "{} database file(s) found in PIC’s default data folder",
            files.len()
        ),
    };
    let heading = gtk::Label::new(Some(&description));
    heading.set_xalign(0.0);
    heading.set_wrap(true);
    heading.add_css_class("dim-label");
    content.append(&heading);

    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    if files.is_empty() {
        let empty = gtk::Label::new(Some("No library or backup database files were found."));
        empty.set_xalign(0.0);
        empty.set_margin_top(12);
        empty.set_margin_bottom(12);
        empty.set_margin_start(12);
        empty.set_margin_end(12);
        list.append(&empty);
    } else {
        for path in files {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.set_margin_top(8);
            row.set_margin_bottom(8);
            row.set_margin_start(12);
            row.set_margin_end(12);
            let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
            labels.set_hexpand(true);
            let filename = path.file_name().and_then(|name| name.to_str());
            let name = gtk::Label::new(filename);
            name.set_xalign(0.0);
            name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            labels.append(&name);
            let full_path = gtk::Label::new(Some(&path.display().to_string()));
            full_path.set_xalign(0.0);
            full_path.set_wrap(true);
            full_path.add_css_class("dim-label");
            labels.append(&full_path);
            row.append(&labels);

            let open = gtk::Button::with_label("Open");
            open.set_valign(gtk::Align::Center);
            let path_for_open = path.clone();
            let parent_for_open = parent.clone();
            let status_for_open = status.clone();
            let refresh_for_open = refresh.clone();
            let window_for_open = window.clone();
            let management_for_open = management.clone();
            open.connect_clicked(move |_| {
                let display_name = file_stem(&path_for_open);
                let result = crate::db::known_libraries()
                    .and_then(|libraries| {
                        libraries
                            .into_iter()
                            .find(|library| library.path == path_for_open)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                crate::db::add_existing_library(
                                    &path_for_open,
                                    &display_name,
                                    "",
                                )
                            })
                    })
                    .and_then(|library| {
                        (management_for_open.switch_library)(&library.id)
                            .map_err(anyhow::Error::msg)?;
                        Ok(library)
                    });
                match result {
                    Ok(library) => {
                        status_for_open.set_text(&format!("Opened {}.", library.name));
                        refresh_for_open();
                        window_for_open.close();
                    }
                    Err(error) => show_library_error(
                        &parent_for_open,
                        &status_for_open,
                        "Could not open database",
                        &format!("{error:#}"),
                    ),
                }
            });
            row.append(&open);
            list.append(&row);
        }
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    content.append(&scroll);

    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    let window_for_close = window.clone();
    close.connect_clicked(move |_| window_for_close.close());
    content.append(&close);
    window.set_child(Some(&content));
    window.present();
}

fn create_dialog(
    parent: &adw::Window,
    status: gtk::Label,
    refresh: Rc<dyn Fn()>,
    management: DatabaseManagement,
) {
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    form.set_margin_top(16);
    form.set_margin_bottom(16);
    form.set_margin_start(16);
    form.set_margin_end(16);
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Library name"));
    let description = gtk::Entry::new();
    description.set_placeholder_text(Some("Description (optional)"));
    form.append(&name);
    form.append(&description);
    let dialog = adw::AlertDialog::builder()
        .heading("Create Database")
        .extra_child(&form)
        .close_response("cancel")
        .default_response("create")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.connect_response(Some("create"), move |_, _| {
        let result = crate::db::suggested_library_path(&name.text())
            .and_then(|path| crate::db::create_library(&path, &name.text(), &description.text()))
            .and_then(|library| {
                (management.switch_library)(&library.id).map_err(anyhow::Error::msg)?;
                Ok(library)
            });
        match result {
            Ok(library) => {
                status.set_text(&format!("Created {}.", library.name));
                refresh();
            }
            Err(error) => status.set_text(&format!("Could not create database: {error:#}")),
        }
    });
    dialog.present(Some(parent));
}

fn restore_dialog(
    parent: &adw::Window,
    status: gtk::Label,
    refresh: Rc<dyn Fn()>,
    management: DatabaseManagement,
) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("PIC database backups"));
    filter.add_pattern("*.db");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let chooser = gtk::FileDialog::builder()
        .title("Restore Database Backup")
        .accept_label("Restore")
        .filters(&filters)
        .modal(true)
        .build();
    if let Ok(library) = crate::db::active_library() {
        if let Some(folder) = library_folder(&library) {
            chooser.set_initial_folder(Some(&gio::File::for_path(folder)));
        }
    }
    chooser.open(Some(parent), None::<&gio::Cancellable>, move |result| {
        if let Ok(file) = result {
            if let Some(backup_path) = file.path() {
                let restored_name = format!("Restored {}", file_stem(&backup_path));
                status.set_text("Restoring database…");
                let (sender, receiver) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let result = crate::db::suggested_library_path(&restored_name)
                        .and_then(|destination| {
                            crate::db::restore_as_library(
                                &backup_path,
                                &destination,
                                &restored_name,
                                &format!("Restored from {}", backup_path.display()),
                            )
                        })
                        .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(result);
                });
                let status = status.clone();
                let refresh = refresh.clone();
                let management = management.clone();
                glib::timeout_add_local(Duration::from_millis(50), move || {
                    match receiver.try_recv() {
                        Ok(Ok(library)) => {
                            match (management.switch_library)(&library.id) {
                                Ok(()) => {
                                    status.set_text(&format!("Restored {}.", library.name));
                                    refresh();
                                }
                                Err(error) => status.set_text(&format!(
                                    "Restored the database, but could not open it: {error}"
                                )),
                            }
                            glib::ControlFlow::Break
                        }
                        Ok(Err(error)) => {
                            status.set_text(&format!("Restore failed: {error}"));
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            status.set_text("Restore failed.");
                            glib::ControlFlow::Break
                        }
                    }
                });
            }
        }
    });
}

fn library_folder(library: &crate::db::LibraryEntry) -> Option<PathBuf> {
    library
        .path
        .parent()
        .map(Path::to_path_buf)
        .filter(|path| path.is_dir())
        .or_else(|| crate::db::library_directory().ok().filter(|path| path.is_dir()))
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("PIC Library")
        .to_owned()
}

fn maintenance_trace(event: &str) {
    if std::env::var_os("PIC_LIBRARY_MAINTENANCE_TRACE").is_some() {
        eprintln!("PIC_LIBRARY_MAINTENANCE {event}");
    }
}

fn clear_thumbnail_cache_async(
    button: gtk::Button,
    status: gtk::Label,
    action: &'static str,
    success_message: &'static str,
    failure_prefix: &'static str,
) {
    button.set_sensitive(false);
    status.set_text("Clearing thumbnail cache…");
    maintenance_trace(&format!("action={action} phase=cache_clear_start"));
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = crate::thumbnail::clear_cache().map_err(|error| format!("{error:#}"));
        let _ = sender.send(result);
    });
    glib::timeout_add_local(Duration::from_millis(50), move || {
        match receiver.try_recv() {
            Ok(Ok(())) => {
                maintenance_trace(&format!("action={action} phase=cache_clear_complete"));
                status.set_text(success_message);
                button.set_sensitive(true);
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                maintenance_trace(&format!(
                    "action={action} phase=cache_clear_failed error={error}"
                ));
                eprintln!("{failure_prefix}{error}");
                status.set_text(&format!("{failure_prefix}{error}"));
                button.set_sensitive(true);
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                status.set_text(&format!("{failure_prefix}background operation stopped"));
                button.set_sensitive(true);
                glib::ControlFlow::Break
            }
        }
    });
}
