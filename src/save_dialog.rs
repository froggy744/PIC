//! Shared system Save picker for rendered photo and collage exports.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gio::prelude::*;
use gtk4 as gtk;

/// Opens the platform Save picker and returns the selected destination after
/// the picker has handled any overwrite confirmation.
pub fn choose_export_save_path(
    parent: &gtk::Window,
    title: &str,
    action_label: &str,
    initial_name: &str,
    extension: Option<&str>,
    initial_folder: Option<PathBuf>,
    on_selected: Rc<dyn Fn(PathBuf, bool)>,
) {
    let initial_name = name_with_extension(initial_name, extension);
    let dialog = gtk::FileDialog::builder()
        .title(title)
        .accept_label(action_label)
        .initial_name(&initial_name)
        .modal(true)
        .build();
    if let Some(folder) = initial_folder.as_ref().filter(|path| path.is_dir()) {
        dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
    }
    let extension = extension.map(str::to_owned);

    dialog.save(Some(parent), None::<&gio::Cancellable>, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(path) = file.path() else {
            return;
        };
        let path = append_missing_extension(path, extension.as_deref());
        // GTK's system picker completes its overwrite confirmation before
        // invoking this callback. Export code still performs an atomic replace.
        let confirmed_replace = path.is_file();
        on_selected(path, confirmed_replace);
    });
}

fn name_with_extension(name: &str, extension: Option<&str>) -> String {
    let path = Path::new(name);
    match extension {
        Some(extension) if path.extension().is_none() => {
            path.with_extension(extension).to_string_lossy().into_owned()
        }
        _ => name.to_string(),
    }
}

fn append_missing_extension(mut path: PathBuf, extension: Option<&str>) -> PathBuf {
    if path.extension().is_none() {
        if let Some(extension) = extension {
            path.set_extension(extension);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_filename_adds_only_a_missing_extension() {
        assert_eq!(name_with_extension("collage", Some("jpg")), "collage.jpg");
        assert_eq!(name_with_extension("collage.png", Some("jpg")), "collage.png");
        assert_eq!(name_with_extension("collage", None), "collage");
    }

    #[test]
    fn selected_path_adds_only_a_missing_extension() {
        assert_eq!(
            append_missing_extension(PathBuf::from("/tmp/collage"), Some("jpg")),
            PathBuf::from("/tmp/collage.jpg"),
        );
        assert_eq!(
            append_missing_extension(PathBuf::from("/tmp/collage.png"), Some("jpg")),
            PathBuf::from("/tmp/collage.png"),
        );
    }
}
