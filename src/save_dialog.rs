//! Shared destination selection for rendered photo and collage exports.
//! Existing paths always require an explicit choice: replace when permitted,
//! choose another name, or cancel.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gio::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

pub fn choose_export_save_path(
    parent: &gtk::Window,
    title: &str,
    action_label: &str,
    initial_name: &str,
    extension: Option<&str>,
    filters: Option<gio::ListStore>,
    may_replace: Rc<dyn Fn(&Path) -> bool>,
    on_selected: Rc<dyn Fn(PathBuf, bool)>,
) {
    open_save_dialog(
        parent.clone(),
        title.to_string(),
        action_label.to_string(),
        initial_name.to_string(),
        extension.map(str::to_string),
        filters,
        may_replace,
        on_selected,
    );
}

fn open_save_dialog(
    parent: gtk::Window,
    title: String,
    action_label: String,
    initial_name: String,
    extension: Option<String>,
    filters: Option<gio::ListStore>,
    may_replace: Rc<dyn Fn(&Path) -> bool>,
    on_selected: Rc<dyn Fn(PathBuf, bool)>,
) {
    let dialog = gtk::FileDialog::builder()
        .title(&title)
        .accept_label(&action_label)
        .initial_name(&initial_name)
        .modal(true)
        .build();
    if let Some(filters) = filters.as_ref() {
        dialog.set_filters(Some(filters));
    }

    let selected_parent = parent.clone();
    dialog.save(Some(&parent), None::<&gio::Cancellable>, move |result| {
        let Some(path) = result.ok().and_then(|file| file.path()) else {
            return;
        };
        let path = add_extension_if_missing(path, extension.as_deref());
        if !path.exists() {
            on_selected(path, false);
            return;
        }

        let replace_allowed = may_replace(&path);
        let body = if replace_allowed {
            format!(
                "{} already exists. Replace it, choose another name, or cancel?",
                path.display(),
            )
        } else {
            format!(
                "{} already exists and cannot be replaced here. Choose another name to continue.",
                path.display(),
            )
        };
        let confirm = adw::AlertDialog::builder()
            .heading(if replace_allowed {
                "File already exists"
            } else {
                "Choose another filename"
            })
            .body(body)
            .close_response("cancel")
            .default_response("save-as")
            .build();
        confirm.add_response("cancel", "Cancel");
        confirm.add_response("save-as", "Save As…");
        if replace_allowed {
            confirm.add_response("replace", "Replace");
            confirm.set_response_appearance("replace", adw::ResponseAppearance::Destructive);
        }

        let save_parent = selected_parent.clone();
        let save_title = title.clone();
        let save_action_label = action_label.clone();
        let save_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| initial_name.clone());
        let save_extension = extension.clone();
        let save_filters = filters.clone();
        let save_policy = may_replace.clone();
        let save_callback = on_selected.clone();
        confirm.connect_response(Some("save-as"), move |_, _| {
            open_save_dialog(
                save_parent.clone(),
                save_title.clone(),
                save_action_label.clone(),
                save_name.clone(),
                save_extension.clone(),
                save_filters.clone(),
                save_policy.clone(),
                save_callback.clone(),
            );
        });

        if replace_allowed {
            let replace_path = path.clone();
            let replace_callback = on_selected.clone();
            confirm.connect_response(Some("replace"), move |_, _| {
                replace_callback(replace_path.clone(), true);
            });
        }
        confirm.present(Some(&selected_parent));
    });
}

fn add_extension_if_missing(path: PathBuf, extension: Option<&str>) -> PathBuf {
    match extension {
        Some(extension) if path.extension().is_none() => path.with_extension(extension),
        _ => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_destination_adds_only_a_missing_extension() {
        assert_eq!(
            add_extension_if_missing(PathBuf::from("collage"), Some("jpg")),
            PathBuf::from("collage.jpg"),
        );
        assert_eq!(
            add_extension_if_missing(PathBuf::from("collage.png"), Some("jpg")),
            PathBuf::from("collage.png"),
        );
        assert_eq!(
            add_extension_if_missing(PathBuf::from("collage"), None),
            PathBuf::from("collage"),
        );
    }
}
