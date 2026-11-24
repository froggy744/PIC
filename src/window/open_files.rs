//! Files handed to PIC by the desktop ("Open With PIC" in the file manager,
//! `pic-rs photo.jpg`, or dragged onto the window) open in the viewer.
//!
//! A single file opens among the other photos of its folder, so the arrow
//! keys browse the folder as they do in an image viewer. Several files open
//! as exactly that set. Photos already in the library open as their library
//! photo; any other file is shown as a photo outside the library
//! (`PhotoObject::outside_library`) and is never added to the catalogue.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gio::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;
use rusqlite::Connection;

use crate::db;
use crate::lightbox::Lightbox;
use crate::photo_object::PhotoObject;

use super::front_surface::{Front, FrontSurface};

/// Window action that opens its `as` (array of paths) parameter in the viewer.
pub(crate) const ACTION: &str = "open-paths";

/// The viewer list for the requested paths and the index to start at.
///
/// One file: the supported photos of its folder, sorted by name, starting at
/// that file (it is kept even when its type is not a library format). One
/// folder: its supported photos. Several paths: those files in the given
/// order, folders expanded. Returns an empty list when nothing can be shown.
pub(crate) fn viewer_paths(requested: &[PathBuf]) -> (Vec<PathBuf>, usize) {
    if let [file] = requested {
        if file.is_file() {
            let siblings = file.parent().map(folder_photos).unwrap_or_default();
            if let Some(index) = siblings.iter().position(|path| path == file) {
                return (siblings, index);
            }
            return (vec![file.clone()], 0);
        }
    }
    let mut paths = Vec::new();
    for path in requested {
        if path.is_dir() {
            paths.extend(folder_photos(path));
        } else if path.is_file() {
            paths.push(path.clone());
        }
    }
    (paths, 0)
}

fn folder_photos(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut photos = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file() && crate::scanner::supported(path))
        .collect::<Vec<_>>();
    photos.sort_by_cached_key(|path| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        (name.to_lowercase(), name.into_owned())
    });
    photos
}

/// A library photo for each catalogued path, an outside-library photo for
/// the rest.
fn photo_objects(connection: &Connection, paths: &[PathBuf]) -> Vec<PhotoObject> {
    paths
        .iter()
        .map(|path| {
            db::photo_by_path(connection, &path.to_string_lossy())
                .ok()
                .flatten()
                .map(|photo| PhotoObject::from_photo(&photo))
                .unwrap_or_else(|| PhotoObject::outside_library(path))
        })
        .collect()
}

/// Read the dimensions and capture facts of a photo outside the library
/// before it is edited: crop, straighten and the preview bounds need its
/// real size, which opening the folder does not read for every file.
pub(crate) fn load_outside_metadata(photo: &PhotoObject) {
    if !photo.is_outside_library() || (photo.width() > 0 && photo.height() > 0) {
        return;
    }
    let path = photo.path();
    let metadata = crate::source::file(&path)
        .query_info(
            "time::modified,standard::size",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .map_err(anyhow::Error::from)
        .and_then(|info| crate::scanner::read_metadata(&path, &info));
    let metadata = match metadata {
        Ok(metadata) => metadata,
        Err(error) => {
            eprintln!("PIC: could not read {path}: {error:#}");
            return;
        }
    };
    photo.set_width(metadata.width.unwrap_or_default());
    photo.set_height(metadata.height.unwrap_or_default());
    if let Some(taken_at) = metadata.taken_at.as_deref() {
        photo.set_taken_at(taken_at);
    }
    if let Some(camera) = metadata.camera.as_deref() {
        photo.set_camera(camera);
    }
    photo.set_aperture(metadata.aperture.unwrap_or_default());
    photo.set_iso(metadata.iso.unwrap_or_default());
    photo.set_shutter_speed(metadata.shutter_speed.unwrap_or_default());
    photo.set_focal_length(metadata.focal_length.unwrap_or_default());
}

/// The viewer to return to after editing photo `id` of `viewer`: on the
/// saved copy, placed right after its original, or else on the photo.
pub(crate) fn viewer_after_edit(
    mut viewer: Vec<PhotoObject>,
    id: i64,
    saved_copy: Option<PathBuf>,
) -> (Vec<PhotoObject>, usize) {
    let index = viewer
        .iter()
        .position(|photo| photo.id() == id)
        .unwrap_or_default();
    match saved_copy {
        Some(path) => {
            let at = (index + 1).min(viewer.len());
            viewer.insert(at, PhotoObject::outside_library(&path));
            (viewer, at)
        }
        None => (viewer, index),
    }
}

/// Whether files dropped on the window open while `front` is in front.
/// The editor and the collage keep their work: a drop there would cover
/// (and, for an outside photo, silently end) the session in progress.
pub(super) fn accepts_drop(front: Front) -> bool {
    !matches!(front, Front::Editor | Front::Collage)
}

/// Install the window's `open-paths` action and its file drop target.
/// Paths that arrive before the window is mapped (a cold start from the
/// file manager) open once it is.
pub(super) fn install(
    window: &adw::ApplicationWindow,
    lightbox: &Rc<Lightbox>,
    connection: &Rc<RefCell<Connection>>,
    front: &Rc<FrontSurface>,
) {
    let pending: Rc<RefCell<Option<Vec<PathBuf>>>> = Rc::new(RefCell::new(None));
    let open: Rc<dyn Fn(Vec<PathBuf>)> = {
        let lightbox = Rc::downgrade(lightbox);
        let connection = connection.clone();
        Rc::new(move |requested| {
            let Some(lightbox) = lightbox.upgrade() else {
                return;
            };
            let (paths, index) = viewer_paths(&requested);
            if paths.is_empty() {
                eprintln!("PIC: nothing to open in {requested:?}");
                return;
            }
            let photos = photo_objects(&connection.borrow(), &paths);
            lightbox.open(photos, index);
        })
    };
    {
        let pending = pending.clone();
        let open = open.clone();
        window.connect_map(move |_| {
            if let Some(paths) = pending.borrow_mut().take() {
                // Let the first layout settle before the viewer covers it.
                let open = open.clone();
                glib::idle_add_local_once(move || open(paths));
            }
        });
    }
    let action = gio::SimpleAction::new(ACTION, Some(glib::VariantTy::STRING_ARRAY));
    let window_for_action = window.downgrade();
    let open_for_action = open.clone();
    action.connect_activate(move |_, parameter| {
        let Some(paths) = parameter.and_then(|parameter| parameter.get::<Vec<String>>()) else {
            return;
        };
        let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
        let mapped = window_for_action
            .upgrade()
            .is_some_and(|window| window.is_mapped());
        if mapped {
            open_for_action(paths);
        } else {
            pending.replace(Some(paths));
        }
    });
    window.add_action(&action);

    // Photos dragged from the file manager (or any app offering files).
    // Drops without a local path, such as a web image, are refused.
    let drop = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::COPY,
    );
    {
        let front = Rc::downgrade(front);
        drop.connect_accept(move |_, _| {
            front
                .upgrade()
                .is_some_and(|front| accepts_drop(front.current()))
        });
    }
    drop.connect_drop(move |_, value, _, _| {
        let Ok(files) = value.get::<gtk::gdk::FileList>() else {
            return false;
        };
        let paths = local_paths(&files.files())
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        if viewer_paths(&paths).0.is_empty() {
            return false;
        }
        open(paths);
        true
    });
    window.add_controller(drop);
}

/// The local paths of files handed to the application; files without a
/// local path (no FUSE mount for a remote URI) are skipped.
pub(crate) fn local_paths(files: &[gio::File]) -> Vec<String> {
    files
        .iter()
        .filter_map(|file| file.path())
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

/// Hand `paths` to `window`'s viewer.
pub(crate) fn open_in(window: &impl IsA<gtk::Widget>, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    let _ = window
        .as_ref()
        .activate_action(&format!("win.{ACTION}"), Some(&paths.to_variant()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary folder holding empty `files`, removed on drop.
    struct Folder(PathBuf);

    impl Folder {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn folder(files: &[&str]) -> Folder {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "pic-open-files-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        for name in files {
            std::fs::write(dir.join(name), b"").expect("write file");
        }
        Folder(dir)
    }

    #[test]
    fn one_file_opens_among_its_folders_photos_sorted_by_name() {
        let dir = folder(&["b.jpg", "A.png", "notes.txt", "c.JPG"]);
        let (paths, index) = viewer_paths(&[dir.path().join("b.jpg")]);
        let names = paths
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, ["A.png", "b.jpg", "c.JPG"]);
        assert_eq!(index, 1);
    }

    #[test]
    fn one_unsupported_file_opens_alone() {
        let dir = folder(&["a.jpg", "notes.txt"]);
        let file = dir.path().join("notes.txt");
        assert_eq!(viewer_paths(&[file.clone()]), (vec![file], 0));
    }

    #[test]
    fn several_files_open_as_given_and_missing_ones_are_dropped() {
        let dir = folder(&["a.jpg", "b.jpg", "c.jpg"]);
        let requested = [
            dir.path().join("c.jpg"),
            dir.path().join("gone.jpg"),
            dir.path().join("a.jpg"),
        ];
        let (paths, index) = viewer_paths(&requested);
        assert_eq!(paths, [requested[0].clone(), requested[2].clone()]);
        assert_eq!(index, 0);
    }

    #[test]
    fn a_folder_opens_its_photos() {
        let dir = folder(&["b.jpg", "a.jpg", "notes.txt"]);
        let (paths, index) = viewer_paths(&[dir.path().to_path_buf()]);
        assert_eq!(paths, [dir.path().join("a.jpg"), dir.path().join("b.jpg")]);
        assert_eq!(index, 0);
    }

    #[test]
    fn after_save_copy_the_viewer_opens_on_the_copy_beside_its_original() {
        let viewer = ["/p/a.png", "/p/b.png", "/p/c.png"]
            .map(|path| PhotoObject::outside_library(Path::new(path)))
            .to_vec();
        let edited = viewer[1].id();
        let (photos, index) =
            viewer_after_edit(viewer.clone(), edited, Some(PathBuf::from("/p/b_edit.png")));
        let paths = photos.iter().map(PhotoObject::path).collect::<Vec<_>>();
        assert_eq!(paths, ["/p/a.png", "/p/b.png", "/p/b_edit.png", "/p/c.png"]);
        assert_eq!(index, 2);
        assert!(photos[2].is_outside_library());
        let (photos, index) = viewer_after_edit(viewer, edited, None);
        assert_eq!(photos.len(), 3);
        assert_eq!(index, 1);
    }

    #[test]
    fn drops_open_the_viewer_except_over_the_editor_and_the_collage() {
        for front in [
            Front::Gallery,
            Front::Lightbox,
            Front::DateOverview,
            Front::LibraryHome,
            Front::Albums,
            Front::FoldersOverview,
            Front::None,
        ] {
            assert!(accepts_drop(front), "{front:?}");
        }
        assert!(!accepts_drop(Front::Editor));
        assert!(!accepts_drop(Front::Collage));
    }

    #[test]
    fn catalogued_paths_open_as_library_photos_and_others_outside_it() {
        let connection = Connection::open_in_memory().expect("database");
        connection.execute_batch(db::SCHEMA).expect("schema");
        connection
            .execute(
                "INSERT INTO photos (path, added_at) VALUES ('/photos/in.jpg', 0)",
                [],
            )
            .expect("insert photo");
        let photos = photo_objects(
            &connection,
            &[
                PathBuf::from("/photos/in.jpg"),
                PathBuf::from("/photos/out.jpg"),
            ],
        );
        assert!(!photos[0].is_outside_library());
        assert_eq!(photos[0].path(), "/photos/in.jpg");
        assert!(photos[1].is_outside_library());
        assert_eq!(photos[1].path(), "/photos/out.jpg");
        assert!(photos[1].original_available());
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;
    use crate::window::{
        LAST_VIEW_SETTING_KEY, WINDOW_HEIGHT_SETTING_KEY, WINDOW_WIDTH_SETTING_KEY,
    };
    use std::time::{Duration, Instant};

    fn settle() {
        let context = glib::MainContext::default();
        let until = Instant::now() + Duration::from_millis(900);
        while Instant::now() < until {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut result = vec![widget.clone()];
        let mut child = widget.first_child();
        while let Some(widget) = child {
            result.extend(descendants(&widget));
            child = widget.next_sibling();
        }
        result
    }

    // xvfb-run -a -s "-screen 0 1920x1080x24" env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
    //   GSK_RENDERER=cairo cargo test --locked -- --test-threads=1 --exact \
    //   window::open_files::window_tests::outside_photo_opens_in_the_viewer_and_saves_an_edited_copy --ignored
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn outside_photo_opens_in_the_viewer_and_saves_an_edited_copy() {
        adw::init().unwrap();
        let app = adw::Application::builder()
            .application_id("io.pic.OpenFilesTest")
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let directory =
            std::env::temp_dir().join(format!("pic-open-files-window-{}", std::process::id()));
        let photos = directory.join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        for name in ["a.jpg", "b.jpg", "c.jpg"] {
            image::RgbImage::from_pixel(64, 48, image::Rgb([200, 120, 40]))
                .save(photos.join(name))
                .unwrap();
        }
        let original = std::fs::read(photos.join("b.jpg")).unwrap();
        let database = directory.join("library.db");
        let connection = db::open(&database).unwrap();
        for (key, value) in [
            (LAST_VIEW_SETTING_KEY, "all"),
            ("onboarding-never-show", "true"),
            (WINDOW_WIDTH_SETTING_KEY, "1400"),
            (WINDOW_HEIGHT_SETTING_KEY, "900"),
        ] {
            db::set_setting(&connection, key, value).unwrap();
        }
        let window = crate::window::build(&app, connection);
        // As `connect_open` does on a cold start: the paths arrive before
        // the window is mapped.
        open_in(
            &window,
            vec![photos.join("b.jpg").to_string_lossy().into_owned()],
        );
        window.present();
        settle();

        let root = window.upcast_ref::<gtk::Widget>();
        let widgets = descendants(root);
        let backdrop = widgets
            .iter()
            .find(|widget| widget.has_css_class("lightbox-backdrop"))
            .expect("lightbox missing");
        assert!(backdrop.is_drawable(), "the viewer did not open");
        assert!(
            widgets.iter().any(|widget| widget
                .downcast_ref::<gtk::Label>()
                .is_some_and(|label| label.text() == "b.jpg" && label.is_drawable())),
            "the opened photo is not the one shown"
        );
        // Only the file name and size are shown for an outside photo.
        let shown_metrics = widgets
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
            .filter(|label| label.has_css_class("metric-key") && label.is_drawable())
            .map(|label| label.text().to_string())
            .collect::<Vec<_>>();
        assert!(
            shown_metrics == ["File name", "Size"],
            "an outside photo shows {shown_metrics:?}"
        );
        let favourite = widgets
            .iter()
            .find(|widget| {
                widget.is::<gtk::Button>()
                    && widget.tooltip_text().as_deref() == Some("Add to Favourites")
            })
            .expect("favourite button missing");
        assert!(
            !favourite.is_sensitive(),
            "library actions stay enabled for a photo outside the library"
        );
        let button = |widgets: &[gtk::Widget], matches: &dyn Fn(&gtk::Widget) -> bool| {
            widgets
                .iter()
                .find(|widget| {
                    widget.is::<gtk::Button>() && widget.is_drawable() && matches(widget)
                })
                .cloned()
                .and_downcast::<gtk::Button>()
        };
        let edit = button(&widgets, &|widget| {
            widget.tooltip_text().as_deref() == Some("Open or close photo editor")
        })
        .expect("Edit button missing");
        assert!(
            edit.is_sensitive(),
            "Edit is disabled for a photo outside the library"
        );
        edit.emit_clicked();
        settle();

        let widgets = descendants(root);
        let save_copy = button(&widgets, &|widget| {
            descendants(widget).iter().any(|child| {
                child
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|label| label.text() == "Save Copy")
            })
        })
        .expect("the editor did not offer Save Copy");
        assert!(
            button(&widgets, &|widget| widget.tooltip_text().as_deref()
                == Some("Copy the edited photo to paste into another app"))
            .is_some(),
            "the editor has no Copy button"
        );
        save_copy.emit_clicked();
        settle();
        settle();

        let copy = photos.join("b_edit.jpg");
        assert!(copy.is_file(), "Save Copy wrote no file");
        assert_eq!(std::fs::read(photos.join("b.jpg")).unwrap(), original);
        let widgets = descendants(root);
        assert!(
            widgets.iter().any(|widget| widget
                .downcast_ref::<gtk::Label>()
                .is_some_and(|label| label.text() == "b_edit.jpg" && label.is_drawable())),
            "the viewer did not return on the saved copy"
        );
        let library = db::open_existing(&database).unwrap();
        let catalogued: i64 = library
            .query_row("SELECT COUNT(*) FROM photos", [], |row| row.get(0))
            .unwrap();
        assert_eq!(catalogued, 0, "editing added the photo to the library");

        // The outside photo's context menu hides whole groups; only the
        // separators between two visible groups stay.
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let item = |visible: bool| {
            let button = gtk::Button::with_label("item");
            button.set_visible(visible);
            menu.append(&button);
        };
        let separator = || menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        item(true);
        separator();
        item(false);
        separator();
        item(true);
        separator();
        item(false);
        crate::window::hide_orphan_separators(&menu);
        let shown = descendants(menu.upcast_ref())
            .into_iter()
            .skip(1)
            .filter(|widget| widget.parent().as_ref() == Some(menu.upcast_ref()))
            .filter(|widget| widget.is_visible())
            .map(|widget| widget.is::<gtk::Separator>())
            .collect::<Vec<_>>();
        assert_eq!(shown, [false, true, false], "item, separator, item");
        window.close();
        let _ = std::fs::remove_dir_all(&directory);
    }
}
