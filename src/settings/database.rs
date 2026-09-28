use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use super::DatabaseManagement;

pub(super) fn page(parent: &adw::Window, management: DatabaseManagement) -> gtk::ScrolledWindow {
    let content = super::page_content(
        "Database",
        "Manage separate local PIC libraries and consistent metadata backups. Photos and the shared thumbnail cache are not copied.",
    );
    let current = gtk::Label::new(None);
    current.set_xalign(0.0);
    current.add_css_class("title-4");
    content.append(&current);

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

    let known_title = gtk::Label::new(Some("Known libraries"));
    known_title.set_xalign(0.0);
    known_title.add_css_class("heading");
    content.append(&known_title);
    let known = gtk::ListBox::new();
    known.add_css_class("boxed-list");
    content.append(&known);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let create = gtk::Button::with_label("Create Database");
    let open = gtk::Button::with_label("Open Database…");
    let backup = gtk::Button::with_label("Backup Database");
    let restore = gtk::Button::with_label("Restore Database…");
    for button in [&create, &open, &backup, &restore] {
        actions.append(button);
    }
    content.append(&actions);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("dim-label");
    content.append(&status);

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
            let chooser = gtk::FileChooserNative::new(
                Some("Open PIC Database"),
                Some(&parent),
                gtk::FileChooserAction::Open,
                Some("Open"),
                Some("Cancel"),
            );
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("SQLite databases"));
            filter.add_pattern("*.db");
            chooser.add_filter(&filter);
            let status = status.clone();
            let refresh = refresh.clone();
            let management = management.clone();
            chooser.connect_response(move |chooser, response| {
                if response == gtk::ResponseType::Accept {
                    if let Some(path) = chooser.file().and_then(|file| file.path()) {
                        let display_name = file_stem(&path);
                        match crate::db::add_existing_library(&path, &display_name, "").and_then(
                            |library| {
                                (management.switch_library)(&library.id)
                                    .map_err(anyhow::Error::msg)?;
                                Ok(library)
                            },
                        ) {
                            Ok(library) => {
                                status.set_text(&format!("Opened {}.", library.name));
                                refresh();
                            }
                            Err(error) => {
                                status.set_text(&format!("Could not open database: {error:#}"))
                            }
                        }
                    }
                }
                chooser.destroy();
            });
            chooser.show();
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
    let dialog = gtk::Dialog::new();
    dialog.set_title(Some("Create Database"));
    dialog.set_transient_for(Some(parent));
    dialog.set_modal(true);
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    dialog.add_button("Create", gtk::ResponseType::Accept);
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
    dialog.content_area().append(&form);
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Accept {
            let result = crate::db::suggested_library_path(&name.text())
                .and_then(|path| {
                    crate::db::create_library(&path, &name.text(), &description.text())
                })
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
        }
        dialog.close();
    });
    dialog.show();
}

fn restore_dialog(
    parent: &adw::Window,
    status: gtk::Label,
    refresh: Rc<dyn Fn()>,
    management: DatabaseManagement,
) {
    let chooser = gtk::FileChooserNative::new(
        Some("Restore Database Backup"),
        Some(parent),
        gtk::FileChooserAction::Open,
        Some("Restore"),
        Some("Cancel"),
    );
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("PIC database backups"));
    filter.add_pattern("*.db");
    chooser.add_filter(&filter);
    chooser.connect_response(move |chooser, response| {
        if response == gtk::ResponseType::Accept {
            if let Some(backup_path) = chooser.file().and_then(|file| file.path()) {
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
        chooser.destroy();
    });
    chooser.show();
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("PIC Library")
        .to_owned()
}
