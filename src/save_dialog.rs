//! Shared destination and overwrite flow for rendered photo and collage exports.
//! The same in-app confirmation offers Replace and Save As together.

use std::path::{Component, Path, PathBuf};
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
    initial_folder: Option<PathBuf>,
    may_replace: Rc<dyn Fn(&Path) -> bool>,
    on_selected: Rc<dyn Fn(PathBuf, bool)>,
) {
    choose_folder(
        parent.clone(),
        title.to_string(),
        action_label.to_string(),
        initial_name.to_string(),
        extension.map(str::to_string),
        initial_folder,
        may_replace,
        on_selected,
    );
}

fn choose_folder(
    parent: gtk::Window,
    title: String,
    action_label: String,
    initial_name: String,
    extension: Option<String>,
    initial_folder: Option<PathBuf>,
    may_replace: Rc<dyn Fn(&Path) -> bool>,
    on_selected: Rc<dyn Fn(PathBuf, bool)>,
) {
    let dialog = gtk::FileDialog::builder()
        .title("Choose export folder")
        .accept_label("Choose")
        .modal(true)
        .build();
    if let Some(folder) = initial_folder.as_ref().filter(|path| path.is_dir()) {
        dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
    }

    let callback_parent = parent.clone();
    dialog.select_folder(Some(&parent), None::<&gio::Cancellable>, move |result| {
        let Some(folder) = result.ok().and_then(|file| file.path()) else {
            return;
        };
        enter_filename(
            callback_parent.clone(),
            title.clone(),
            action_label.clone(),
            folder,
            initial_name.clone(),
            extension.clone(),
            may_replace.clone(),
            on_selected.clone(),
        );
    });
}

fn enter_filename(
    parent: gtk::Window,
    title: String,
    action_label: String,
    folder: PathBuf,
    initial_name: String,
    extension: Option<String>,
    may_replace: Rc<dyn Fn(&Path) -> bool>,
    on_selected: Rc<dyn Fn(PathBuf, bool)>,
) {
    let entry = gtk::Entry::new();
    entry.set_text(&initial_name);
    entry.set_hexpand(true);
    entry.set_activates_default(true);
    entry.set_placeholder_text(Some("File name"));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&gtk::Label::new(Some("File name")));
    row.append(&entry);

    let dialog = adw::AlertDialog::builder()
        .heading(&title)
        .body(format!("Save in {}", folder.display()))
        .extra_child(&row)
        .close_response("cancel")
        .default_response("save")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("folder", "Choose Folder…");
    dialog.add_response("save", &action_label);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);

    let folder_parent = parent.clone();
    let folder_title = title.clone();
    let folder_action = action_label.clone();
    let folder_name = entry.clone();
    let folder_extension = extension.clone();
    let folder_path = folder.clone();
    let folder_policy = may_replace.clone();
    let folder_callback = on_selected.clone();
    dialog.connect_response(Some("folder"), move |_, _| {
        choose_folder(
            folder_parent.clone(),
            folder_title.clone(),
            folder_action.clone(),
            folder_name.text().to_string(),
            folder_extension.clone(),
            Some(folder_path.clone()),
            folder_policy.clone(),
            folder_callback.clone(),
        );
    });

    let save_parent = parent.clone();
    let save_title = title.clone();
    let save_action = action_label.clone();
    let save_folder = folder.clone();
    let save_name = entry.clone();
    let save_extension = extension.clone();
    let save_policy = may_replace.clone();
    let save_callback = on_selected.clone();
    dialog.connect_response(Some("save"), move |_, _| {
        let name = save_name.text().trim().to_string();
        let Some(path) = filename_path(&save_folder, &name, save_extension.as_deref()) else {
            enter_filename(
                save_parent.clone(),
                save_title.clone(),
                save_action.clone(),
                save_folder.clone(),
                name,
                save_extension.clone(),
                save_policy.clone(),
                save_callback.clone(),
            );
            return;
        };
        if path.exists() {
            confirm_existing_path(
                save_parent.clone(),
                save_title.clone(),
                save_action.clone(),
                save_folder.clone(),
                path,
                save_extension.clone(),
                save_policy.clone(),
                save_callback.clone(),
            );
        } else {
            save_callback(path, false);
        }
    });
    dialog.present(Some(&parent));
}

fn confirm_existing_path(
    parent: gtk::Window,
    title: String,
    action_label: String,
    folder: PathBuf,
    path: PathBuf,
    extension: Option<String>,
    may_replace: Rc<dyn Fn(&Path) -> bool>,
    on_selected: Rc<dyn Fn(PathBuf, bool)>,
) {
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

    let save_parent = parent.clone();
    let save_title = title.clone();
    let save_action = action_label.clone();
    let save_folder = folder.clone();
    let save_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let save_extension = extension.clone();
    let save_policy = may_replace.clone();
    let save_callback = on_selected.clone();
    confirm.connect_response(Some("save-as"), move |_, _| {
        enter_filename(
            save_parent.clone(),
            save_title.clone(),
            save_action.clone(),
            save_folder.clone(),
            save_name.clone(),
            save_extension.clone(),
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
    confirm.present(Some(&parent));
}

fn filename_path(folder: &Path, name: &str, extension: Option<&str>) -> Option<PathBuf> {
    let path = Path::new(name);
    if name.is_empty() || !matches!(path.components().next(), Some(Component::Normal(_))) {
        return None;
    }
    if path.components().count() != 1 {
        return None;
    }
    let path = folder.join(path);
    match extension {
        Some(extension) if path.extension().is_none() => Some(path.with_extension(extension)),
        _ => Some(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_destination_adds_only_a_missing_extension() {
        let folder = Path::new("/tmp");
        assert_eq!(
            filename_path(folder, "collage", Some("jpg")),
            Some(PathBuf::from("/tmp/collage.jpg")),
        );
        assert_eq!(
            filename_path(folder, "collage.png", Some("jpg")),
            Some(PathBuf::from("/tmp/collage.png")),
        );
        assert_eq!(
            filename_path(folder, "collage", None),
            Some(folder.join("collage"))
        );
        assert_eq!(filename_path(folder, "", Some("jpg")), None);
        assert_eq!(filename_path(folder, "../outside.jpg", Some("jpg")), None);
    }
}
