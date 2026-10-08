use std::cell::{Cell, RefCell};
use std::path::Path;
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

    let known_title = gtk::Label::new(Some("Known libraries"));
    known_title.set_xalign(0.0);
    known_title.add_css_class("heading");
    content.append(&known_title);
    let known = gtk::ListBox::new();
    known.add_css_class("boxed-list");
    content.append(&known);

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
        Some("Remove cached thumbnails that no longer match current photos."),
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
            button.set_sensitive(false);
            thumbnail_status.set_text("Cleaning thumbnail cache…");

            let valid = match crate::thumbnail::valid_cache_paths(&connection.borrow()) {
                Ok(valid) => valid,
                Err(error) => {
                    eprintln!("Could not collect thumbnail cache keys: {error:#}");
                    thumbnail_status.set_text("Could not clean the thumbnail cache.");
                    button.set_sensitive(true);
                    cleanup_running.set(false);
                    return;
                }
            };

            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let result = crate::thumbnail::cleanup_cache(&valid);
                let _ = sender.send(result);
            });

            let poll_status = thumbnail_status.clone();
            let poll_button = button.clone();
            let poll_running = cleanup_running.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                match receiver.try_recv() {
                    Ok(Ok(cleanup)) => {
                        poll_status.set_text(&super::cleanup_result_text(&cleanup));
                        poll_button.set_sensitive(true);
                        poll_running.set(false);
                        glib::ControlFlow::Break
                    }
                    Ok(Err(error)) => {
                        eprintln!("Could not clean thumbnail cache: {error:#}");
                        poll_status.set_text("Could not clean the thumbnail cache.");
                        poll_button.set_sensitive(true);
                        poll_running.set(false);
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        poll_status.set_text("Could not clean the thumbnail cache.");
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
        Some("Remove indexed photos and album links. Registered folders remain."),
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
        let maintenance = maintenance.clone();
        let parent = parent.clone();
        let thumbnail_status = thumbnail_status.clone();
        clear_thumbnails_button.connect_clicked(move |_| {
            let maintenance = maintenance.clone();
            let thumbnail_status = thumbnail_status.clone();
            super::confirm_destructive(
                &parent,
                "Clear thumbnails?",
                "Cached thumbnails will be deleted. Your photos and database will remain.",
                Rc::new(move || {
                    (maintenance.clear_thumbnails)();
                    thumbnail_status.set_text("Thumbnail cache cleared. Reopen Stats to refresh cache totals.");
                }),
            );
        });
    }

    {
        let maintenance = maintenance.clone();
        let parent = parent.clone();
        clear_database_button.connect_clicked(move |_| {
            let maintenance = maintenance.clone();
            super::confirm_destructive(
                &parent,
                "Clear database?",
                "Indexed photos and album links will be removed. Registered folders will remain.",
                Rc::new(move || (maintenance.clear_database)()),
            );
        });
    }

    {
        let maintenance = maintenance.clone();
        let parent = parent.clone();
        clear_all_button.connect_clicked(move |_| {
            let maintenance = maintenance.clone();
            super::confirm_destructive(
                &parent,
                "Clear everything?",
                "Indexed photos, albums, registered folders, and cached thumbnails will be deleted.",
                Rc::new(move || (maintenance.clear_all)()),
            );
        });
    }

    let refresh: Rc<dyn Fn()> = {
        let current = current.clone();
        let name = name.clone();
        let description = description.clone();
        let known = known.clone();
        let management = management.clone();
        let status = status.clone();
        Rc::new(move || refresh_page(&current, &name, &description, &known, &status, &management))
    };
    refresh();

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
            chooser.open(Some(&parent), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        let display_name = file_stem(&path);
                        match crate::db::add_existing_library(&path, &display_name, "")
                            .and_then(|library| {
                                (management.switch_library)(&library.id)
                                    .map_err(anyhow::Error::msg)?;
                                Ok(library)
                            })
                        {
                            Ok(library) => {
                                status.set_text(&format!("Opened {}.", library.name));
                                refresh();
                            }
                            Err(error) => status.set_text(&format!("Could not open database: {error:#}")),
                        }
                    }
                }
            });
        });
    }

    {
        let status = status.clone();
        backup.connect_clicked(move |button| {
            let Ok(library) = crate::db::active_library() else {
                status.set_text("Could not identify the current library.");
                return;
            };
            button.set_sensitive(false);
            status.set_text("Backing up database…");
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender
                    .send(crate::db::backup_library(&library).map_err(|error| error.to_string()));
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
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
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
        let switch = gtk::Button::with_label("Open / Switch");
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
        switch.connect_clicked(move |_| match (management.switch_library)(&id) {
            Ok(()) => {
                status.set_text("Library switched.");
                refresh_page(
                    &current_label,
                    &name,
                    &description,
                    &list_for_switch,
                    &status,
                    &management,
                );
            }
            Err(error) => {
                status.set_text(&format!("Could not switch photo library: {error}"));
            }
        });
        row.append(&switch);
        list.append(&row);
    }
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

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("PIC Library")
        .to_owned()
}
