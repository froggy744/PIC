//! Shared asynchronous GTK file picker; cancellation never starts an operation.

use std::path::PathBuf;

use gtk4::{self as gtk, prelude::*};

pub(crate) enum FileChoice {
    Open,
    Save,
    Folder,
}

pub(crate) fn choose(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    accept_label: &str,
    choice: FileChoice,
    initial_name: Option<&str>,
    filter: Option<&gtk::FileFilter>,
    on_selected: impl FnOnce(PathBuf) + 'static,
) {
    let dialog = gtk::FileDialog::builder()
        .title(title)
        .accept_label(accept_label)
        .modal(true)
        .build();
    dialog.set_initial_name(initial_name);
    if let Some(filter) = filter {
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(filter);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(filter));
    }
    let selected = move |result: Result<gio::File, glib::Error>| match result {
        Ok(file) => {
            if let Some(path) = file.path() {
                on_selected(path);
            } else {
                eprintln!("PIC requires a local file or a mounted network path");
            }
        }
        Err(error) => {
            if !error.matches(gtk::DialogError::Dismissed)
                && !error.matches(gtk::DialogError::Cancelled)
            {
                eprintln!("Could not choose a file: {error}");
            }
        }
    };
    match choice {
        FileChoice::Open => dialog.open(Some(parent), gio::Cancellable::NONE, selected),
        FileChoice::Save => dialog.save(Some(parent), gio::Cancellable::NONE, selected),
        FileChoice::Folder => dialog.select_folder(Some(parent), gio::Cancellable::NONE, selected),
    }
}
