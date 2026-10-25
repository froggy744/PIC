use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use rusqlite::Connection;

use crate::db::{self, Album};
use crate::settings::{self, AlbumAppearance};

const DEFAULT_THUMBNAIL_WIDTH: i32 = 136;
const DEFAULT_THUMBNAIL_HEIGHT: i32 = 91;
const BOOKSHELF_ROW_HEIGHT: i32 = 288;
const BOOKSHELF_SURFACE_Y: i32 = 235;
const BOOKSHELF_TOP_INSET: i32 = 28;
const ALBUM_COVER_THEME_DIRECTORY: &str = "images/theme/album-covers";
const BOOKSHELF_THEME_DIRECTORY: &str = "images/theme/bookshelf";
const BOOKSHELF_MAX_COLUMNS: usize = 5;
const BOOKSHELF_MIN_ROWS: usize = 3;
const BOOKSHELF_COLUMN_GAP: i32 = 28;
const BOOKSHELF_SIDE_PADDING: i32 = 28;
const BOOKSHELF_MIN_CARD_WIDTH: i32 = 220;
const BOOKSHELF_MAX_CARD_WIDTH: i32 = 330;
const BOOKSHELF_CARD_FILL_RATIO: f64 = 0.86;
const BOOKSHELF_RUNTIME_KEY: &str = "picasa-bookshelf-runtime";
const ALBUM_SIZE_KEY: &str = "albums-index-size";
const ALBUM_SORT_KEY: &str = "albums-index-sort";
const FRAME_OPENING_KEY: &str = "picasa-album-frame-opening";

// Album UI implementation is split by responsibility but included into this
// module so the refactor does not change callback visibility or state wiring.
include!("albums/bookshelf.rs");
#[path = "albums/coverflow_input.rs"]
mod coverflow_input;
include!("albums/coverflow.rs");
include!("albums/card.rs");
include!("albums/actions.rs");
include!("albums/view.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_covers_follow_photo_orientation_without_excessive_card_height() {
        assert_eq!(plain_cover_height(240, Some((3000, 2000))), 160);
        assert_eq!(plain_cover_height(240, Some((2000, 3000))), 360);
        assert_eq!(plain_cover_height(240, Some((4000, 3000))), 180);
        assert_eq!(plain_cover_height(240, Some((3000, 4000))), 320);
        assert_eq!(plain_cover_height(240, Some((1000, 4000))), 360);
        assert_eq!(plain_cover_height(240, Some((0, 3000))), cover_height(240));
        assert_eq!(plain_cover_height(240, None), cover_height(240));
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn plain_album_card_keeps_a_portrait_cover_and_full_photo() {
        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-portrait-cover-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let photo_path = directory.join("portrait.png");
        image::RgbImage::new(60, 90).save(&photo_path).unwrap();
        let connection = Rc::new(RefCell::new(
            db::open(&directory.join("library.db")).unwrap(),
        ));
        let album = db::create_album(&connection.borrow(), "Portrait").unwrap();
        connection
            .borrow()
            .execute(
                "INSERT INTO photos (path) VALUES (?1)",
                [photo_path.to_string_lossy().as_ref()],
            )
            .unwrap();
        let photo_id = connection.borrow().last_insert_rowid();
        db::set_album_cover_photo(&connection.borrow(), album.id, photo_id).unwrap();
        db::set_setting(
            &connection.borrow(),
            crate::db::THUMBNAIL_FIT_WHOLE_PHOTO_SETTING_KEY,
            "true",
        )
        .unwrap();

        let albums = db::albums(&connection.borrow()).unwrap();
        let opened = Rc::new(Cell::new(None));
        let opened_for_callback = opened.clone();
        let view = build(
            &albums,
            connection.clone(),
            136,
            Rc::new(move |album_id| opened_for_callback.set(Some(album_id))),
            Rc::new(|| {}),
            Rc::new(|| {}),
        );
        let window = gtk::Window::builder()
            .default_width(900)
            .default_height(800)
            .child(&view)
            .build();
        window.present();
        settle_gtk_layout();
        let cover = find_descendant_with_css_class(view.upcast_ref(), "album-cover").unwrap();
        let picture = find_descendant_with_css_class(&cover, "thumbnail")
            .unwrap()
            .downcast::<gtk::Picture>()
            .unwrap();
        assert!(cover.has_css_class("plain-album-cover"));
        assert_eq!(picture.content_fit(), gtk::ContentFit::Contain);
        assert_eq!(picture_dimensions(&picture), Some((60, 90)));
        assert_eq!(cover.height_request(), cover.width_request() * 3 / 2);
        assert!(cover.height() > cover.width());
        find_descendant_with_css_class(view.upcast_ref(), "album-card")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        assert_eq!(opened.get(), Some(album.id));

        db::set_setting(
            &connection.borrow(),
            crate::db::THUMBNAIL_FIT_WHOLE_PHOTO_SETTING_KEY,
            "false",
        )
        .unwrap();
        refresh(
            &view,
            &albums,
            connection.clone(),
            136,
            Rc::new(|_| {}),
            Rc::new(|| {}),
        );
        settle_gtk_layout();
        let cropped_cover =
            find_descendant_with_css_class(view.upcast_ref(), "album-cover").unwrap();
        let cropped_picture = find_descendant_with_css_class(&cropped_cover, "thumbnail")
            .unwrap()
            .downcast::<gtk::Picture>()
            .unwrap();
        assert_eq!(cropped_picture.content_fit(), gtk::ContentFit::Cover);
        assert_eq!(
            cropped_cover.height_request(),
            cropped_cover.width_request() * 2 / 3
        );
        assert!(
            cropped_cover.width() > cropped_cover.height(),
            "cropped album cover allocated {}x{}",
            cropped_cover.width(),
            cropped_cover.height()
        );

        db::set_setting(
            &connection.borrow(),
            crate::db::THUMBNAIL_FIT_WHOLE_PHOTO_SETTING_KEY,
            "true",
        )
        .unwrap();
        refresh(
            &view,
            &albums,
            connection.clone(),
            136,
            Rc::new(|_| {}),
            Rc::new(|| {}),
        );
        settle_gtk_layout();
        let full_cover = find_descendant_with_css_class(view.upcast_ref(), "album-cover").unwrap();
        let full_picture = find_descendant_with_css_class(&full_cover, "thumbnail")
            .unwrap()
            .downcast::<gtk::Picture>()
            .unwrap();
        assert_eq!(full_picture.content_fit(), gtk::ContentFit::Contain);
        assert_eq!(
            full_cover.height_request(),
            full_cover.width_request() * 3 / 2
        );
        assert!(full_cover.height() > full_cover.width());

        window.close();
        drop(view);
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn settle_gtk_layout() {
        let main_loop = gtk::glib::MainLoop::new(None, false);
        let loop_to_quit = main_loop.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
            loop_to_quit.quit();
        });
        main_loop.run();
    }

    fn collect_album_covers(root: &gtk::Widget, covers: &mut Vec<gtk::Widget>) {
        if root.has_css_class("album-cover") {
            covers.push(root.clone());
            return;
        }
        let mut child = root.first_child();
        while let Some(widget) = child {
            collect_album_covers(&widget, covers);
            child = widget.next_sibling();
        }
    }

    fn album_cover_geometry(root: &gtk::Widget) -> Vec<(i32, i32, i32, i32, [i32; 4])> {
        let mut covers = Vec::new();
        collect_album_covers(root, &mut covers);
        covers
            .into_iter()
            .map(|cover| {
                let picture = find_descendant_with_css_class(&cover, "thumbnail").unwrap();
                (
                    cover.width_request(),
                    cover.height_request(),
                    cover.width(),
                    cover.height(),
                    [
                        picture.margin_start(),
                        picture.margin_top(),
                        picture.margin_end(),
                        picture.margin_bottom(),
                    ],
                )
            })
            .collect()
    }

    fn save_album_snapshot(window: &gtk::Window, path: &Path) {
        let id = std::process::Command::new("xdotool")
            .args(["search", "--onlyvisible", "--name", "^PIC Cover Flow Wheel Validation$"])
            .output().expect("find the actual GTK test window");
        assert!(id.status.success(), "could not find the live GTK window");
        let id = String::from_utf8_lossy(&id.stdout).lines().last().unwrap().to_owned();
        let capture = std::process::Command::new("import")
            .args(["-window", &id, path.to_str().unwrap()]).status()
            .expect("capture the actual GTK window, including its GLArea");
        assert!(capture.success(), "ImageMagick could not capture the GTK window");
        assert!(window.width() > 0);
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn album_cover_appearance_matrix_across_supported_themes() {
        gtk::init().unwrap();
        crate::css::install_foundation(&gtk::gdk::Display::default().unwrap());
        let directory = std::env::temp_dir().join(format!(
            "pic-album-cover-matrix-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Rc::new(RefCell::new(
            db::open(&directory.join("library.db")).unwrap(),
        ));
        settings::set_covers_enabled(&connection.borrow(), true).unwrap();
        settings::set_bookshelf_enabled(&connection.borrow(), false).unwrap();
        let frame_root = crate::css::resolve_runtime_dir(Path::new(ALBUM_COVER_THEME_DIRECTORY));
        let frames = album_frame_paths_in(&frame_root);
        assert!(!frames.is_empty(), "shipped album cover frames are required");

        let mut albums = Vec::new();
        for (index, frame_path) in frames.iter().enumerate() {
            let frame_art = image::open(frame_path).unwrap().to_rgba8();
            assert!(
                frame_art.pixels().any(|pixel| pixel.0[3] < 255),
                "cover frame must retain transparent artwork: {}",
                frame_path.display()
            );
            let album = db::create_album(&connection.borrow(), &format!("Cover {:02}", index + 1))
                .unwrap();
            let relative_frame = frame_path.strip_prefix(&frame_root).unwrap();
            db::set_album_cover_frame(
                &connection.borrow(),
                album.id,
                relative_frame.to_str().unwrap(),
            )
            .unwrap();
            let photo_path = directory.join(format!("cover-{index:02}.png"));
            let photo = image::RgbImage::from_fn(960, 640, |x, y| {
                let band = ((x / 80 + y / 80 + index as u32) % 2) == 0;
                if band {
                    image::Rgb([40 + index as u8 * 13, 105, 185])
                } else {
                    image::Rgb([225, 150 + index as u8 * 5, 60])
                }
            });
            photo.save(&photo_path).unwrap();
            connection
                .borrow()
                .execute(
                    "INSERT INTO photos(path, width, height, size_bytes, mtime, added_at) \
                     VALUES (?1, 960, 640, ?2, 0, 0)",
                    rusqlite::params![
                        photo_path.to_string_lossy().as_ref(),
                        std::fs::metadata(&photo_path).unwrap().len() as i64,
                    ],
                )
                .unwrap();
            let photo_id = connection.borrow().last_insert_rowid();
            db::add_photos_to_album(&connection.borrow(), album.id, &[photo_id]).unwrap();
            db::set_album_cover_photo(&connection.borrow(), album.id, photo_id).unwrap();
            albums.push(album);
        }
        albums = db::albums(&connection.borrow()).unwrap();

        let view = build(
            &albums,
            connection.clone(),
            300,
            Rc::new(|_| {}),
            Rc::new(|| {}),
            Rc::new(|| {}),
        );
        let window = gtk::Window::builder()
            .default_width(1600)
            .default_height(1000)
            .child(&view)
            .build();
        window.present();
        settle_gtk_layout();

        let theme_root = crate::css::resolve_runtime_dir(Path::new("themes"));
        let themes = crate::css::theme_discovery::discover(&theme_root);
        assert!(!themes.is_empty(), "supported theme list must not be empty");
        let screenshot_root = std::env::var_os("PICASA_ALBUM_COVER_MATRIX_DIR").map(PathBuf::from);
        if let Some(root) = screenshot_root.as_ref() {
            std::fs::create_dir_all(root).unwrap();
        }
        let display = gtk::gdk::Display::default().unwrap();
        let mut provider: Option<gtk::CssProvider> = None;
        let style_manager = adw::StyleManager::default();
        let base_provider = themes.iter().find(|theme| theme.is_base).map(|theme| {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(&theme.css);
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            provider
        });

        for theme in &themes {
            if let Some(previous) = provider.take() {
                gtk::style_context_remove_provider_for_display(&display, &previous);
            }
            if !theme.is_base {
                let current = gtk::CssProvider::new();
                current.load_from_string(&theme.css);
                gtk::style_context_add_provider_for_display(
                    &display,
                    &current,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
                );
                provider = Some(current);
            }
            style_manager.set_color_scheme(if theme.dark {
                adw::ColorScheme::ForceDark
            } else {
                adw::ColorScheme::ForceLight
            });

            settings::set_bookshelf_enabled(&connection.borrow(), false).unwrap();
            refresh(
                &view,
                &albums,
                connection.clone(),
                300,
                Rc::new(|_| {}),
                Rc::new(|| {}),
            );
            settle_gtk_layout();
            settle_gtk_layout();
            let without_background = album_cover_geometry(view.upcast_ref());
            assert_eq!(without_background.len(), frames.len(), "{}", theme.id);
            if let Some(root) = screenshot_root.as_ref() {
                save_album_snapshot(
                    &window,
                    &root.join(format!("{}-background-off.png", sanitize_component(&theme.id))),
                );
            }

            settings::set_bookshelf_enabled(&connection.borrow(), true).unwrap();
            refresh(
                &view,
                &albums,
                connection.clone(),
                300,
                Rc::new(|_| {}),
                Rc::new(|| {}),
            );
            settle_gtk_layout();
            settle_gtk_layout();
            let with_background = album_cover_geometry(view.upcast_ref());
            if let Some(root) = screenshot_root.as_ref() {
                save_album_snapshot(
                    &window,
                    &root.join(format!("{}-background-on.png", sanitize_component(&theme.id))),
                );
            }
            assert_eq!(
                with_background,
                without_background,
                "bookshelf toggle changed cover dimensions/opening geometry for theme {}",
                theme.id
            );
        }

        if let Some(provider) = provider {
            gtk::style_context_remove_provider_for_display(&display, &provider);
        }
        if let Some(provider) = base_provider {
            gtk::style_context_remove_provider_for_display(&display, &provider);
        }
        window.close();
        drop(view);
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn sanitize_component(value: &str) -> String {
        value
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .split('-')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    }

    fn album(id: i64) -> Album {
        Album {
            id,
            name: format!("Album {id}"),
            created_at: 0,
            photo_count: 0,
            cover_frame: None,
            cover_photo_id: None,
        }
    }

    fn alpha_with_opening(
        width: usize,
        height: usize,
        opening: (usize, usize, usize, usize),
    ) -> Vec<u8> {
        let (x, y, opening_width, opening_height) = opening;
        let mut alpha = vec![255u8; width * height];
        for row in y..y + opening_height {
            for column in x..x + opening_width {
                alpha[row * width + column] = 0;
            }
        }
        alpha
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn standard_index_controls_preserve_size_and_sort_after_refresh() {
        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-index-controls-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Rc::new(RefCell::new(
            db::open(&directory.join("library.db")).unwrap(),
        ));
        db::create_album(&connection.borrow(), "Zebra").unwrap();
        db::create_album(&connection.borrow(), "Alps").unwrap();
        for name in [
            "Family",
            "Garden",
            "Summer holiday with a long album title",
            "Weekend",
        ] {
            db::create_album(&connection.borrow(), name).unwrap();
        }
        let albums = db::albums(&connection.borrow()).unwrap();
        let view = build(
            &albums,
            connection.clone(),
            100,
            Rc::new(|_| {}),
            Rc::new(|| {}),
            Rc::new(|| {}),
        );
        let cover_before = find_descendant_with_css_class(view.upcast_ref(), "album-cover")
            .unwrap()
            .width_request();
        assert!(
            cover_before >= 220,
            "album covers should have their own larger default size"
        );
        refresh(
            &view,
            &albums,
            connection.clone(),
            300,
            Rc::new(|_| {}),
            Rc::new(|| {}),
        );
        assert_eq!(
            find_descendant_with_css_class(view.upcast_ref(), "album-cover")
                .unwrap()
                .width_request(),
            cover_before
        );
        let sort = find_descendant_with_css_class(view.upcast_ref(), "albums-sort")
            .unwrap()
            .downcast::<gtk::DropDown>()
            .unwrap();
        sort.set_selected(1);
        let first = find_descendant_with_css_class(view.upcast_ref(), "album-name")
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        assert_eq!(first.text(), "Zebra");
        refresh(
            &view,
            &albums,
            connection.clone(),
            136,
            Rc::new(|_| {}),
            Rc::new(|| {}),
        );
        assert_eq!(
            find_descendant_with_css_class(view.upcast_ref(), "album-name")
                .unwrap()
                .downcast::<gtk::Label>()
                .unwrap()
                .text(),
            "Zebra"
        );
        let size = find_descendant_with_css_class(view.upcast_ref(), "albums-size")
            .unwrap()
            .downcast::<gtk::DropDown>()
            .unwrap();
        size.set_selected(2);
        assert!(
            find_descendant_with_css_class(view.upcast_ref(), "album-cover")
                .unwrap()
                .width_request()
                > cover_before
        );
        let created = Rc::new(Cell::new(false));
        let created_callback = created.clone();
        connect_create_album(&view, Rc::new(move || created_callback.set(true)));
        find_descendant_with_css_class(view.upcast_ref(), "albums-create")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        assert!(created.get());

        let window = gtk::Window::builder()
            .default_width(1000)
            .default_height(760)
            .child(&view)
            .build();
        window.present();
        settle_gtk_layout();
        settle_gtk_layout();
        let sort_bounds = find_descendant_with_css_class(view.upcast_ref(), "albums-sort")
            .unwrap()
            .compute_bounds(&view)
            .unwrap();
        let size_bounds = find_descendant_with_css_class(view.upcast_ref(), "albums-size")
            .unwrap()
            .compute_bounds(&view)
            .unwrap();
        let create_bounds = find_descendant_with_css_class(view.upcast_ref(), "albums-create")
            .unwrap()
            .compute_bounds(&view)
            .unwrap();
        assert_eq!(sort_bounds.y(), create_bounds.y());
        assert_eq!(size_bounds.y(), create_bounds.y());
        assert!(sort_bounds.x() < size_bounds.x() && size_bounds.x() < create_bounds.x());
        let size = find_descendant_with_css_class(view.upcast_ref(), "albums-size")
            .unwrap()
            .downcast::<gtk::DropDown>()
            .unwrap();
        size.set_selected(0);
        settle_gtk_layout();
        let compact_grid = find_descendant_with_css_class(view.upcast_ref(), "albums-home-grid")
            .unwrap()
            .downcast::<gtk::FlowBox>()
            .unwrap();
        assert!(
            compact_grid.max_children_per_line() >= 5,
            "compact layout should reveal another column"
        );
        let mut compact_cards = Vec::new();
        collect_bookshelf_cards(compact_grid.upcast_ref(), &mut compact_cards);
        for card in &compact_cards {
            let cover = find_descendant_with_css_class(card.upcast_ref(), "album-cover").unwrap();
            assert_eq!(
                card.width(),
                cover.width_request(),
                "compact album card must not grow beyond its cover"
            );
        }
        let compact_columns = compact_grid.max_children_per_line();
        window.set_default_size(1400, 760);
        settle_gtk_layout();
        settle_gtk_layout();
        assert!(
            compact_grid.max_children_per_line() > compact_columns,
            "compact layout should add columns as the app grows"
        );
        size.set_selected(2);
        settle_gtk_layout();
        let grid = find_descendant_with_css_class(view.upcast_ref(), "albums-home-grid")
            .unwrap()
            .downcast::<gtk::FlowBox>()
            .unwrap();
        let wide_columns = grid.max_children_per_line();
        assert!(wide_columns >= 3);
        let mut cards = Vec::new();
        collect_bookshelf_cards(grid.upcast_ref(), &mut cards);
        assert_eq!(cards.len(), 6);
        assert_eq!(
            cards[0].compute_bounds(&view).unwrap().y(),
            cards[2].compute_bounds(&view).unwrap().y(),
            "three covers should fit in the first row at this width"
        );
        for card in &cards {
            let cover = find_descendant_with_css_class(card.upcast_ref(), "album-cover").unwrap();
            assert_eq!(cover.width_request(), cards[0].width_request());
            assert_eq!(cover.height_request(), cover.width_request() * 2 / 3);
            assert_eq!(
                card.width(),
                cover.width_request(),
                "album card must not grow beyond its cover"
            );
        }
        assert!(find_descendant_with_css_class(view.upcast_ref(), "album-cover-action").is_none());
        show_album_context_menu(
            &cards[0],
            12.0,
            12.0,
            connection.clone(),
            albums[0].clone(),
            Rc::new(|| {}),
            true,
        );
        settle_gtk_layout();
        assert!(
            find_descendant_with_css_class(cards[0].upcast_ref(), "rename-album-action").is_some()
        );
        if let Some(popover) = cards[0].last_child().and_downcast::<gtk::Popover>() {
            popover.popdown();
        }
        settle_gtk_layout();
        if let Some(path) = std::env::var_os("PICASA_ALBUM_SCREENSHOT") {
            let snapshot = gtk::Snapshot::new();
            gtk::WidgetPaintable::new(Some(&window)).snapshot(
                &snapshot,
                window.width() as f64,
                window.height() as f64,
            );
            let node = snapshot.to_node().unwrap();
            window
                .renderer()
                .unwrap()
                .render_texture(&node, None)
                .save_to_png(path)
                .unwrap();
        }
        window.set_default_size(520, 760);
        settle_gtk_layout();
        settle_gtk_layout();
        assert!(
            grid.max_children_per_line() < wide_columns,
            "grid should reflow on narrower windows"
        );
        assert!(
            view.hadjustment().upper() <= view.hadjustment().page_size() + 1.0,
            "album page must not overflow horizontally"
        );
        window.close();
        drop(view);
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn discovers_cover_frames_recursively_across_theme_folders() {
        let directory = std::env::temp_dir().join(format!(
            "pic-album-skins-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(album_frame_paths_in(&directory).is_empty());
        std::fs::create_dir(&directory).unwrap();
        assert!(album_frame_paths_in(&directory).is_empty());
        let standard = directory.join("standard");
        let vintage = directory.join("vintage");
        std::fs::create_dir(&standard).unwrap();
        std::fs::create_dir(&vintage).unwrap();
        for path in [
            standard.join("default-frame.png"),
            vintage.join("blue-frame.png"),
            vintage.join("green-frame.png"),
            vintage.join("green-frame.jpg"),
            vintage.join("notes.txt"),
            directory.join("loose-frame.png"),
        ] {
            std::fs::write(path, []).unwrap();
        }
        std::fs::create_dir(vintage.join("directory-frame.png")).unwrap();
        let expected = vec![
            standard.join("default-frame.png"),
            vintage.join("blue-frame.png"),
            vintage.join("green-frame.png"),
        ];
        assert_eq!(album_frame_paths_in(&directory), expected);

        std::fs::write(standard.join("new-frame.png"), []).unwrap();
        let mut expected = expected;
        expected.push(standard.join("new-frame.png"));
        expected.sort();
        assert_eq!(album_frame_paths_in(&directory), expected);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn discovers_folder_backed_row_themes_before_legacy_backgrounds() {
        let directory = std::env::temp_dir().join(format!(
            "pic-bookshelf-backgrounds-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let single = directory.join("single");
        let walnut = directory.join("walnut");
        std::fs::create_dir(&single).unwrap();
        std::fs::create_dir(&walnut).unwrap();
        std::fs::write(single.join("single-row-bookshelf.png"), []).unwrap();
        std::fs::write(walnut.join("row.jpg"), []).unwrap();
        std::fs::write(directory.join("three-panel-bookshelf.jpg"), []).unwrap();
        std::fs::write(directory.join("bookshelf.png"), []).unwrap();

        let themes = bookshelf_themes_in(&directory);
        assert_eq!(themes.len(), 3);
        assert_eq!(themes[0].id, "single");
        assert_eq!(themes[0].kind, BookshelfThemeKind::Row);
        assert_eq!(
            themes[0].image_path,
            single.join("single-row-bookshelf.png")
        );
        assert_eq!(themes[1].id, "walnut");
        assert_eq!(themes[1].kind, BookshelfThemeKind::Row);
        assert_eq!(themes[2].kind, BookshelfThemeKind::Legacy);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn row_theme_geometry_defaults_and_can_be_overridden() {
        let directory = std::env::temp_dir().join(format!(
            "pic-bookshelf-geometry-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        assert_eq!(
            bookshelf_theme_geometry(&directory, &directory.join("row.png")),
            (BOOKSHELF_ROW_HEIGHT, BOOKSHELF_SURFACE_Y)
        );
        std::fs::write(
            directory.join("theme.conf"),
            "row_height=300\nsurface_y=246\n",
        )
        .unwrap();
        assert_eq!(bookshelf_theme_geometry(&directory, &directory.join("row.png")), (300, 246));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn every_shipped_bookshelf_theme_pins_its_own_shelf_surface() {
        let themes = bookshelf_themes_in(Path::new(BOOKSHELF_THEME_DIRECTORY));
        assert_eq!(themes.len(), 7);
        // Shipped artwork is photographic, so each folder ships the conf that
        // states where its drawn shelf edge is; a packaging slip that loses the
        // file has to be visible as missing geometry, not as a silent page-wide
        // default that puts every cover a few pixels off the drawn board.
        assert!(themes.iter().all(|theme|
            theme.kind == BookshelfThemeKind::Row
                && theme.image_path
                    .parent()
                    .map(|directory| directory.join("theme.conf"))
                    .is_some_and(|conf| conf.is_file())
        ));
        // Covers stand on the board, so the pinned line always sits inside the
        // row, above the underside of the board the artwork draws.
        assert!(themes.iter().all(|theme| theme.surface_y > 0 && theme.surface_y < theme.row_height));
        // The pinned numbers are measured at each artwork's own front edge, so
        // a shipped board that is re-cropped or re-exported shows up here.
        for (id, surface_y) in [
            ("concrete", 227),
            ("darkwood", 225),
            ("darkwood-small", 232),
            ("light-wood", 225),
            ("marble", 236),
            ("shelf", 227),
            ("single", 247),
        ] {
            assert!(themes.iter().any(|theme| theme.id == id && theme.surface_y == surface_y));
        }
    }

    #[test]
    fn measures_the_shelf_surface_from_the_artwork() {
        let directory = std::env::temp_dir().join(format!(
            "pic-bookshelf-surface-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let (width, height, surface) = (400_usize, 200_usize, 160_usize);
        // Book spines are one colour per column, so no row above the shelf is
        // flat across the width and no row changes brightness. The board below
        // is flat across the whole width, and its top edge spans the width.
        let pixels: Vec<u8> = (0_usize..height).flat_map(|row| {
            (0_usize..width).map(|column| {
                let spine = if row < surface {
                    40_usize + (column * 7_usize) % 180_usize
                } else {
                    200_usize
                };
                [spine as u8, spine as u8, spine as u8, 255_usize as u8]
            }).flatten().collect::<Vec<u8>>()
        }).collect();
        let artwork = directory.join("row.png");
        let Ok(mut file) = std::fs::File::create(&artwork) else {
            panic!("could not write the test artwork");
        };
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut file).write_image(
            &pixels,
            width as u32,
            height as u32,
            image::ExtendedColorType::Rgba8,
        ).unwrap();

        let (row_height, surface_y) = bookshelf_theme_geometry(&directory, &artwork);
        assert_eq!((row_height, surface_y), (BOOKSHELF_ROW_HEIGHT, 230));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn zero_skins_have_no_selection() {
        let root = Path::new("images/theme/album-covers");
        assert_eq!(selected_frame_index(&album(1), root, &[], 0), None);
    }

    #[test]
    fn album_cover_prefers_the_photo_chosen_from_the_menu() {
        let directory = std::env::temp_dir().join(format!(
            "pic-album-cover-photo-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = db::open(&directory.join("library.db")).unwrap();
        connection
            .execute_batch(
                "INSERT INTO photos (id, path) VALUES
                   (1, '/tmp/one.jpg'),
                   (2, '/tmp/two.jpg'),
                   (3, '/tmp/three.jpg');",
            )
            .unwrap();
        let album = db::create_album(&connection, "Holiday").unwrap();
        let photos: Vec<_> = [1, 2]
            .into_iter()
            .filter_map(|id| db::photo(&connection, id).unwrap())
            .collect();

        // Nothing is cached in this test, so the automatic pick stays empty.
        assert_eq!(
            album_cover_photo(&connection, &album, &photos).map(|photo| photo.id),
            None
        );

        db::set_album_cover_photo(&connection, album.id, 2).unwrap();
        let album = db::albums(&connection).unwrap().remove(0);
        assert_eq!(
            album_cover_photo(&connection, &album, &photos).map(|photo| photo.id),
            Some(2)
        );
        assert_eq!(automatic_album_cover_photo(&photos).map(|photo| photo.id), None);

        // A chosen photo still resolves when it is not an album member.
        db::set_album_cover_photo(&connection, album.id, 3).unwrap();
        let album = db::albums(&connection).unwrap().remove(0);
        assert_eq!(
            album_cover_photo(&connection, &album, &photos).map(|photo| photo.id),
            Some(3)
        );

        // Deleting the photo clears the choice through the schema's
        // ON DELETE SET NULL, so nothing points at a missing photo.
        connection
            .execute("DELETE FROM photos WHERE id = 3", [])
            .unwrap();
        let album = db::albums(&connection).unwrap().remove(0);
        assert_eq!(album.cover_photo_id, None);
        assert_eq!(
            album_cover_photo(&connection, &album, &photos).map(|photo| photo.id),
            None
        );

        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn persisted_frame_is_selected_and_next_theme_wraps() {
        let root = Path::new("images/theme/album-covers");
        let frames = vec![
            root.join("standard/standard-frame.png"),
            root.join("vintage/blue-frame.png"),
            root.join("vintage/green-frame.png"),
        ];
        let mut album = album(7);
        album.cover_frame = Some("vintage/green-frame.png".to_string());

        assert_eq!(selected_frame_index(&album, root, &frames, 0), Some(2));
        // Themes are ordered alphabetically, so the last one wraps to the first.
        assert_eq!(
            next_theme_identifier(&album, root, &frames, 0).as_deref(),
            Some("standard/standard-frame.png")
        );
    }

    #[test]
    fn automatic_frame_index_ignores_album_frame_override() {
        let root = Path::new("images/theme/album-covers");
        let frames = vec![
            root.join("standard/standard-frame.png"),
            root.join("vintage/blue-frame.png"),
            root.join("vintage/green-frame.png"),
        ];
        let album = album(19);
        let automatic = automatic_frame_index(&album, root, &frames, 0).unwrap();
        let custom = (automatic + 1) % frames.len();
        let mut customized = album;
        customized.cover_frame = frame_identifier(root, &frames[custom]);

        assert_eq!(selected_frame_index(&customized, root, &frames, 0), Some(custom));
        assert_eq!(automatic_frame_index(&customized, root, &frames, 0), Some(automatic));
    }

    #[test]
    fn album_layouts_keep_separate_cover_frame_policies() {
        let root = Path::new("images/theme/album-covers");
        let appearance = AlbumAppearance {
            covers_enabled: true,
            ..AlbumAppearance::default()
        };

        assert!(!album_frame_paths_for_layout(root, appearance, settings::AlbumLayout::Grid).is_empty());
        assert_eq!(
            album_frame_paths_for_layout(root, appearance, settings::AlbumLayout::Bookshelf),
            vec![root.join("standard/standard-frame.png")]
        );
        assert!(album_frame_paths_for_layout(root, appearance, settings::AlbumLayout::CoverFlow).is_empty());
    }

    #[test]
    fn missing_persisted_frame_falls_back_to_stable_assignment() {
        let root = Path::new("images/theme/album-covers");
        let frames = vec![
            root.join("standard/standard-frame.png"),
            root.join("vintage/blue-frame.png"),
        ];
        let mut album = album(11);
        album.cover_frame = Some("removed/missing-frame.png".to_string());
        let mut without_stored_frame = album.clone();
        without_stored_frame.cover_frame = None;

        let selected = selected_frame_index(&album, root, &frames, 0);
        assert!(selected.is_some());
        // The disappeared frame leaves the album on the automatic design.
        assert_eq!(
            selected,
            selected_frame_index(&without_stored_frame, root, &frames, 0)
        );
    }

    #[test]
    fn changing_an_album_cover_persists_only_that_albums_next_theme() {
        let directory = std::env::temp_dir().join(format!(
            "pic-next-album-frame-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = db::open(&directory.join("library.db")).unwrap();
        let first = db::create_album(&connection, "First").unwrap();
        let second = db::create_album(&connection, "Second").unwrap();
        db::set_album_cover_frame(&connection, first.id, "vintage/green-frame.png").unwrap();
        let first = db::albums(&connection).unwrap().remove(0);
        let root = Path::new("images/theme/album-covers");
        let frames = vec![
            root.join("standard/standard-frame.png"),
            root.join("vintage/blue-frame.png"),
            root.join("vintage/green-frame.png"),
        ];

        assert_eq!(
            change_to_next_album_cover(&connection, &first, root, &frames, 0)
                .unwrap()
                .as_deref(),
            Some("standard/standard-frame.png")
        );
        let albums = db::albums(&connection).unwrap();
        assert_eq!(
            albums
                .iter()
                .find(|album| album.id == first.id)
                .unwrap()
                .cover_frame
                .as_deref(),
            Some("standard/standard-frame.png")
        );
        assert_eq!(
            albums
                .iter()
                .find(|album| album.id == second.id)
                .unwrap()
                .cover_frame,
            None
        );
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn all_cover_theme_folders_are_available_with_or_without_a_bookshelf() {
        let directory = Path::new("images/theme/album-covers");
        let with_bookshelf = album_frame_paths_for_appearance(
            directory,
            AlbumAppearance {
                bookshelf_enabled: true,
                covers_enabled: true,
                background_index: 0,
                cover_index: 0,
            },
        );
        let without_bookshelf = album_frame_paths_for_appearance(
            directory,
            AlbumAppearance {
                bookshelf_enabled: false,
                covers_enabled: true,
                background_index: 0,
                cover_index: 0,
            },
        );

        assert_eq!(with_bookshelf, without_bookshelf);
        assert!(with_bookshelf.contains(&directory.join("vintage/blue-frame.png")));
    }

    #[test]
    fn independent_appearance_selects_background_and_cover_assets() {
        use crate::settings::AlbumAppearance;

        let directory = std::env::temp_dir().join(format!(
            "pic-album-appearance-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let standard = directory.join("standard");
        let vintage = directory.join("vintage");
        std::fs::create_dir(&standard).unwrap();
        std::fs::create_dir(&vintage).unwrap();
        std::fs::write(standard.join("white-frame.png"), []).unwrap();
        std::fs::write(vintage.join("blue-frame.png"), []).unwrap();

        let bookshelf = AlbumAppearance {
            bookshelf_enabled: true,
            covers_enabled: true,
            background_index: 0,
            cover_index: 0,
        };
        assert_eq!(
            album_frame_paths_for_appearance(&directory, bookshelf),
            vec![
                standard.join("white-frame.png"),
                vintage.join("blue-frame.png")
            ]
        );

        let plain_covers = AlbumAppearance {
            bookshelf_enabled: false,
            covers_enabled: true,
            background_index: 1,
            cover_index: 0,
        };
        assert_eq!(
            album_frame_paths_for_appearance(&directory, plain_covers),
            vec![
                standard.join("white-frame.png"),
                vintage.join("blue-frame.png")
            ]
        );

        let disabled = AlbumAppearance {
            bookshelf_enabled: true,
            covers_enabled: false,
            background_index: 0,
            cover_index: 0,
        };
        assert!(album_frame_paths_for_appearance(&directory, disabled).is_empty());

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn page_cover_index_selects_one_theme_for_every_album() {
        let root = Path::new("images/theme/album-covers");
        let frames = album_frame_paths_in(root);
        let themes = cover_themes(root, &frames);
        assert!(themes.len() > 1);
        let albums: Vec<_> = (0..5).map(album).collect();

        let theme_for = |album: &Album, cover_index: usize| {
            let index = selected_frame_index(album, root, &frames, cover_index).unwrap();
            cover_theme_name(root, &frames[index]).unwrap()
        };
        for cover_index in 0..themes.len() + 1 {
            let names: std::collections::HashSet<_> = albums
                .iter()
                .map(|album| theme_for(album, cover_index))
                .collect();
            // One page shows one theme, no matter which albums it contains.
            assert_eq!(
                names.len(),
                1,
                "page mixed themes at cover index {cover_index}"
            );
            assert!(names.contains(&themes[cover_index % themes.len()].name));
        }

        let first_album = &albums[0];
        // A full turn of the page action restores the same theme and variant.
        assert_eq!(
            theme_for(first_album, themes.len()),
            theme_for(first_album, 0)
        );
        assert_eq!(
            selected_frame_index(first_album, root, &frames, themes.len()).unwrap(),
            selected_frame_index(first_album, root, &frames, 0).unwrap()
        );
    }

    #[test]
    fn next_album_cover_steps_between_theme_folders() {
        let root = Path::new("images/theme/album-covers");
        let frames = album_frame_paths_in(root);
        let names: Vec<_> = cover_themes(root, &frames)
            .into_iter()
            .map(|theme| theme.name)
            .collect();
        assert!(names.len() > 1);
        let album = album(7);

        let current = selected_frame_index(&album, root, &frames, 0).unwrap();
        let current_theme = cover_theme_name(root, &frames[current]).unwrap();
        let expected = names[(names
            .iter()
            .position(|name| name == &current_theme)
            .unwrap()
            + 1)
            % names.len()]
        .clone();

        let next = next_theme_identifier(&album, root, &frames, 0).unwrap();
        assert_eq!(cover_theme_name(root, &root.join(&next)).unwrap(), expected);
        // The album keeps the same variant whenever it returns to that theme.
        assert_eq!(
            next_theme_identifier(&album, root, &frames, 0),
            Some(next.clone())
        );
        // Clicking again moves on to the theme after that one.
        let mut chosen = album.clone();
        chosen.cover_frame = Some(next);
        let following = next_theme_identifier(&chosen, root, &frames, 0).unwrap();
        assert_ne!(
            cover_theme_name(root, &root.join(&following)).unwrap(),
            expected
        );
    }

    #[test]
    fn responsive_bookshelf_geometry_keeps_cover_bottoms_on_the_surface() {
        for (cover_height, expected_top) in [(91, 144), (137, 98), (186, 49)] {
            let top = bookshelf_card_margin_top(235, cover_height);
            assert_eq!(top, expected_top);
            assert_eq!(top + cover_height, 235);
        }

        assert_eq!(BOOKSHELF_ROW_HEIGHT, 288);
        assert!(BOOKSHELF_SURFACE_Y < BOOKSHELF_ROW_HEIGHT);
    }

    #[test]
    fn bookshelf_keeps_at_least_three_visible_rows() {
        assert_eq!(bookshelf_row_count(0, 5), 3);
        assert_eq!(bookshelf_row_count(1, 5), 3);
        assert_eq!(bookshelf_row_count(10, 5), 3);
        assert_eq!(bookshelf_row_count(10, 4), 3);
        assert_eq!(bookshelf_row_count(10, 3), 4);
        assert_eq!(bookshelf_row_count(20, 5), 4);
    }

    #[test]
    fn responsive_bookshelf_columns_follow_available_width() {
        assert_eq!(bookshelf_columns_for_width(2000, 220), 5);
        assert_eq!(bookshelf_columns_for_width(1400, 220), 5);
        assert_eq!(bookshelf_columns_for_width(1120, 220), 4);
        assert_eq!(bookshelf_columns_for_width(850, 220), 3);
        assert_eq!(bookshelf_columns_for_width(300, 220), 1);
        assert_eq!(bookshelf_columns_for_width(0, 220), 5);
    }

    #[test]
    fn responsive_bookshelf_cards_grow_on_wide_windows() {
        assert_eq!(bookshelf_target_card_width(850, 3), 227);
        assert_eq!(bookshelf_target_card_width(1120, 4), 229);
        assert_eq!(bookshelf_target_card_width(1400, 5), 230);
        assert!(bookshelf_target_card_width(1760, 5) > 240);
        assert!(bookshelf_target_card_width(1760, 5) > 196);
        assert_eq!(bookshelf_target_card_width(2200, 5), 330);
    }

    #[test]
    fn bookshelf_background_rules_cover_row_and_legacy_themes() {
        let directory = std::env::temp_dir().join(format!(
            "pic-bookshelf-css-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let single = directory.join("single");
        std::fs::create_dir(&single).unwrap();
        std::fs::write(single.join("row.png"), []).unwrap();
        std::fs::write(directory.join("three-panel-bookshelf.jpg"), []).unwrap();

        assert_eq!(
            bookshelf_background_path(&directory, 0),
            Some(single.join("row.png"))
        );
        let rules = bookshelf_background_rules(&directory);
        assert!(rules.contains(".albums-bookshelf-0 {"));
        assert!(rules.contains(".albums-bookshelf-1 {"));
        assert!(rules.contains("background-size: 100% 288px"));
        assert!(rules.contains("background-position: center top"));
        assert!(rules.contains("background-size: 100% auto"));
        assert!(crate::css::ALBUMS.contains(".albums-bookshelf .album-photo-count"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn context_menu_model_exposes_theme_controls_and_current_state() {
        use crate::settings::AlbumAppearance;

        let items = album_context_menu_items(AlbumAppearance {
            bookshelf_enabled: true,
            covers_enabled: false,
            background_index: 1,
            cover_index: 0,
        });

        assert_eq!(
            items,
            vec![
                AlbumContextMenuItem {
                    label: "Bookshelf",
                    action: AlbumAppearanceAction::ToggleBookshelf,
                    checked: Some(true),
                    sensitive: true,
                },
                AlbumContextMenuItem {
                    label: "Next Background",
                    action: AlbumAppearanceAction::NextBackground,
                    checked: None,
                    sensitive: true,
                },
                AlbumContextMenuItem {
                    label: "Album Covers",
                    action: AlbumAppearanceAction::ToggleCovers,
                    checked: Some(false),
                    sensitive: true,
                },
                AlbumContextMenuItem {
                    label: "Next Album Covers",
                    action: AlbumAppearanceAction::NextCovers,
                    checked: None,
                    sensitive: true,
                },
                AlbumContextMenuItem {
                    label: "Disable All Themes",
                    action: AlbumAppearanceAction::DisableAll,
                    checked: None,
                    sensitive: true,
                },
                AlbumContextMenuItem {
                    label: "Theme Settings…",
                    action: AlbumAppearanceAction::OpenThemeSettings,
                    checked: None,
                    sensitive: true,
                },
            ]
        );

        let disabled = album_context_menu_items(AlbumAppearance::default());
        assert!(
            !disabled
                .iter()
                .find(|item| item.action == AlbumAppearanceAction::DisableAll)
                .unwrap()
                .sensitive
        );
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn context_menu_renders_and_applies_the_next_background_action() {
        fn find_buttons(widget: &gtk::Widget, buttons: &mut Vec<gtk::Button>) {
            if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                buttons.push(button.clone());
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                find_buttons(&widget, buttons);
                child = widget.next_sibling();
            }
        }

        gtk::init().unwrap();
        let connection = Rc::new(RefCell::new(Connection::open_in_memory().unwrap()));
        connection
            .borrow()
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();
        let changed = Rc::new(std::cell::Cell::new(0));
        let changed_for_callback = changed.clone();
        let view = gtk::ScrolledWindow::new();
        let window = gtk::Window::new();
        window.set_child(Some(&view));
        window.present();

        show_context_menu(
            &view,
            20.0,
            20.0,
            connection.clone(),
            Rc::new(move || changed_for_callback.set(changed_for_callback.get() + 1)),
            Rc::new(|| {}),
        );
        let mut buttons = Vec::new();
        find_buttons(view.upcast_ref(), &mut buttons);
        assert_eq!(buttons.len(), 6);
        buttons[1].emit_clicked();
        assert_eq!(changed.get(), 1);
        assert_eq!(
            settings::album_appearance(&connection.borrow()),
            AlbumAppearance {
                bookshelf_enabled: true,
                covers_enabled: false,
                background_index: 1,
                cover_index: 0,
            }
        );

        buttons[3].emit_clicked();
        assert_eq!(changed.get(), 2);
        assert_eq!(
            settings::album_appearance(&connection.borrow()),
            AlbumAppearance {
                bookshelf_enabled: true,
                covers_enabled: true,
                background_index: 1,
                cover_index: 1,
            }
        );

        window.close();
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn album_context_menu_changes_and_persists_that_albums_frame() {
        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-album-frame-menu-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Rc::new(RefCell::new(
            db::open(&directory.join("library.db")).unwrap(),
        ));
        let album = db::create_album(&connection.borrow(), "Menu Album").unwrap();
        let changed = Rc::new(std::cell::Cell::new(0));
        let changed_for_callback = changed.clone();
        let anchor = gtk::Button::new();
        let window = gtk::Window::builder().child(&anchor).build();
        window.present();

        show_album_context_menu(
            &anchor,
            10.0,
            10.0,
            connection.clone(),
            album.clone(),
            Rc::new(move || changed_for_callback.set(changed_for_callback.get() + 1)),
            true,
        );
        let action = find_descendant_with_css_class(anchor.upcast_ref(), "next-album-frame-action")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        // The album has no frame of its own yet, so there is nothing to reset.
        let reset = find_descendant_with_css_class(anchor.upcast_ref(), "reset-album-frame-action")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        assert!(!reset.is_sensitive());
        action.emit_clicked();

        assert_eq!(changed.get(), 1);
        assert!(settings::album_appearance(&connection.borrow()).covers_enabled);
        assert!(db::albums(&connection.borrow()).unwrap()[0]
            .cover_frame
            .is_some());

        window.close();
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn resetting_an_album_cover_restores_the_automatic_design() {
        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-reset-album-frame-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Rc::new(RefCell::new(
            db::open(&directory.join("library.db")).unwrap(),
        ));
        let album = db::create_album(&connection.borrow(), "Styled").unwrap();
        db::set_album_cover_frame(&connection.borrow(), album.id, "vintage/blue-frame.png")
            .unwrap();
        connection
            .borrow()
            .execute_batch("INSERT INTO photos (id, path) VALUES (7, '/tmp/seven.jpg');")
            .unwrap();
        db::set_album_cover_photo(&connection.borrow(), album.id, 7).unwrap();
        let album = db::albums(&connection.borrow()).unwrap().remove(0);
        let changed = Rc::new(std::cell::Cell::new(0));
        let changed_for_callback = changed.clone();
        let anchor = gtk::Button::new();
        let window = gtk::Window::builder().child(&anchor).build();
        window.present();

        show_album_context_menu(
            &anchor,
            10.0,
            10.0,
            connection.clone(),
            album,
            Rc::new(move || changed_for_callback.set(changed_for_callback.get() + 1)),
            true,
        );
        let reset = find_descendant_with_css_class(anchor.upcast_ref(), "reset-album-frame-action")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        assert!(reset.is_sensitive());
        reset.emit_clicked();

        assert_eq!(changed.get(), 1);
        let album = db::albums(&connection.borrow()).unwrap().remove(0);
        assert_eq!(album.cover_frame, None);
        assert_eq!(album.cover_photo_id, None);

        window.close();
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn album_card_draws_the_photo_chosen_as_its_cover() {
        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-chosen-cover-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Rc::new(RefCell::new(
            db::open(&directory.join("library.db")).unwrap(),
        ));
        let album = db::create_album(&connection.borrow(), "Holiday").unwrap();
        connection
            .borrow()
            .execute_batch(
                "INSERT INTO photos (id, path) VALUES
                   (1, 'screenshots/all photos.jpg'),
                   (2, 'screenshots/rations stars.jpg');",
            )
            .unwrap();
        // Photo 2 is only the chosen cover; it never joined the album.
        db::set_album_cover_photo(&connection.borrow(), album.id, 2).unwrap();

        let albums = db::albums(&connection.borrow()).unwrap();
        let view = build(
            &albums,
            connection.clone(),
            136,
            Rc::new(|_| {}),
            Rc::new(|| {}),
            Rc::new(|| {}),
        );
        let window = gtk::Window::builder()
            .default_width(900)
            .default_height(800)
            .child(&view)
            .build();
        window.present();
        settle_gtk_layout();

        let card = find_descendant_with_css_class(view.upcast_ref(), "album-card").unwrap();
        let picture = find_descendant_with_css_class(card.upcast_ref(), "thumbnail")
            .unwrap()
            .downcast::<gtk::Picture>()
            .unwrap();
        let drawn = picture.file().and_then(|file| file.path()).unwrap();
        assert!(
            drawn.ends_with("screenshots/rations stars.jpg"),
            "card drew {drawn:?}"
        );

        window.close();
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn framed_album_photo_uses_its_skin_opening() {
        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-frame-opening-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Rc::new(RefCell::new(
            db::open(&directory.join("library.db")).unwrap(),
        ));
        let album = db::create_album(&connection.borrow(), "Holiday").unwrap();
        settings::set_bookshelf_enabled(&connection.borrow(), true).unwrap();
        settings::set_covers_enabled(&connection.borrow(), true).unwrap();

        let mut selections = Vec::new();
        for skin in [
            "vintage/blue-frame.png",
            "frame-light/frame-white-panel-frame.png",
        ] {
            db::set_album_cover_frame(&connection.borrow(), album.id, skin).unwrap();
            let albums = db::albums(&connection.borrow()).unwrap();
            let view = build(
                &albums,
                connection.clone(),
                136,
                Rc::new(|_| {}),
                Rc::new(|| {}),
                Rc::new(|| {}),
            );
            let window = gtk::Window::builder()
                .default_width(900)
                .default_height(800)
                .child(&view)
                .build();
            window.present();
            settle_gtk_layout();

            let button = find_descendant_with_css_class(view.upcast_ref(), "album-card")
                .unwrap()
                .downcast::<gtk::Button>()
                .unwrap();
            let cover = find_descendant_with_css_class(button.upcast_ref(), "album-cover").unwrap();
            let picture = find_descendant_with_css_class(cover.upcast_ref(), "thumbnail").unwrap();
            let opening = frame_photo_opening(&Path::new(ALBUM_COVER_THEME_DIRECTORY).join(skin));
            let expected = photo_margins(opening, cover.width_request(), cover.height_request());
            assert_eq!(
                (
                    picture.margin_start(),
                    picture.margin_top(),
                    picture.margin_end(),
                    picture.margin_bottom(),
                ),
                expected,
                "{skin}"
            );
            selections.push(expected);
            window.close();
        }

        assert_ne!(
            selections[0], selections[1],
            "different frame designs must not share one shared opening"
        );
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn detects_the_transparent_opening_inside_a_frame() {
        let alpha = alpha_with_opening(100, 100, (35, 27, 33, 43));
        assert_eq!(
            opening_from_alpha(&alpha, 100, 100, 32),
            Some(PhotoOpening {
                x: 0.35,
                y: 0.27,
                width: 0.33,
                height: 0.43,
            })
        );
    }

    #[test]
    fn ignores_transparency_that_leaves_the_frame_border() {
        // A transparent band along the border is the margin around the album,
        // not the opening that shows the cover photo.
        let mut border_band = vec![255u8; 100 * 100];
        for row in 0..100 {
            for column in 0..20 {
                border_band[row * 100 + column] = 0;
            }
        }
        assert_eq!(opening_from_alpha(&border_band, 100, 100, 32), None);

        // A photo opening must be enclosed by the opaque frame on all sides.
        let mut open_around = vec![0u8; 100 * 100];
        for row in 30..70 {
            for column in 30..70 {
                open_around[row * 100 + column] = 255;
            }
        }
        assert_eq!(opening_from_alpha(&open_around, 100, 100, 32), None);
    }

    #[test]
    fn shipped_frames_report_their_own_photo_openings() {
        let root = Path::new("images/theme/album-covers");
        let expected = [
            (
                "standard/standard-frame.png",
                (0.3572, 0.2721, 0.3150, 0.4154),
            ),
            ("vintage/blue-frame.png", (0.3572, 0.2721, 0.3150, 0.4154)),
            (
                "frame-light/frame-white-panel-frame.png",
                (0.2348, 0.0940, 0.5342, 0.8060),
            ),
            (
                "leather/leather-panel-frame.png",
                (0.3168, 0.2180, 0.3863, 0.4620),
            ),
            (
                "clear-transparent/glossy-glass-frame.png",
                (0.1284, 0.1284, 0.7775, 0.6499),
            ),
        ];
        for (relative, (x, y, width, height)) in expected {
            let opening = frame_photo_opening(&root.join(relative));
            assert!(
                (opening.x - x).abs() < 0.002,
                "{relative} left {}",
                opening.x
            );
            assert!(
                (opening.y - y).abs() < 0.002,
                "{relative} top {}",
                opening.y
            );
            assert!(
                (opening.width - width).abs() < 0.002,
                "{relative} width {}",
                opening.width
            );
            assert!(
                (opening.height - height).abs() < 0.002,
                "{relative} height {}",
                opening.height
            );
        }
    }

    #[test]
    fn photo_margins_keep_the_photo_inside_every_frame() {
        let root = Path::new("images/theme/album-covers");
        for relative in [
            "standard/standard-frame.png",
            "frame-light/frame-white-panel-frame.png",
            "clear-transparent/glossy-glass-frame.png",
        ] {
            let opening = frame_photo_opening(&root.join(relative));
            for width in [100, 136, 220, 300] {
                let height = (width as f64 * 500.0 / 805.0).round() as i32;
                let (left, top, right, bottom) = photo_margins(opening, width, height);
                assert!(
                    left >= 0 && top >= 0 && right >= 0 && bottom >= 0,
                    "{relative}"
                );
                assert!(left + right < width && top + bottom < height, "{relative}");

                // The photo must stay inside the frame's window: never over the
                // frame art, and never inset by more than one rounding pixel.
                let opening_left = (opening.x * width as f64).floor() as i32;
                let opening_top = (opening.y * height as f64).floor() as i32;
                let opening_right = (width as f64 * (opening.x + opening.width)).ceil() as i32;
                let opening_bottom = (height as f64 * (opening.y + opening.height)).ceil() as i32;
                assert!(
                    left >= opening_left && top >= opening_top,
                    "photo over the frame at {relative} {width}"
                );
                assert!(
                    width - right <= opening_right && height - bottom <= opening_bottom,
                    "photo over the frame at {relative} {width}"
                );
                assert!(
                    left - opening_left <= 1 && top - opening_top <= 1,
                    "photo inset too far at {relative} {width}"
                );
                assert!(
                    opening_right - (width - right) <= 1 && opening_bottom - (height - bottom) <= 1,
                    "photo inset too far at {relative} {width}"
                );
            }
        }
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn album_view_modes_preserve_albums_and_click_navigation() {
        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-bookshelf-modes-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Rc::new(RefCell::new(
            db::open(&directory.join("library.db")).unwrap(),
        ));
        let album = db::create_album(&connection.borrow(), "Holiday").unwrap();
        let albums = db::albums(&connection.borrow()).unwrap();
        let opened = Rc::new(std::cell::Cell::new(None));
        let opened_for_callback = opened.clone();
        let on_album: Rc<dyn Fn(i64)> = Rc::new(move |id| opened_for_callback.set(Some(id)));
        let view = build(
            &albums,
            connection.clone(),
            136,
            on_album.clone(),
            Rc::new(|| {}),
            Rc::new(|| {}),
        );
        let window = gtk::Window::builder()
            .default_width(900)
            .default_height(800)
            .child(&view)
            .build();
        window.present();
        settle_gtk_layout();
        assert!(!view.has_css_class("albums-bookshelf"));
        for (style, background, skin) in [
            ("default", false, false),
            ("bookshelf", true, true),
            ("album-covers", false, true),
            ("default", false, false),
        ] {
            db::set_setting(
                &connection.borrow(),
                crate::settings::ALBUM_VIEW_STYLE_SETTING_KEY,
                style,
            )
            .unwrap();
            refresh(
                &view,
                &albums,
                connection.clone(),
                136,
                on_album.clone(),
                Rc::new(|| {}),
            );
            settle_gtk_layout();
            assert_eq!(view.has_css_class("albums-bookshelf"), background);
            assert_eq!(
                find_descendant_with_css_class(view.upcast_ref(), "album-skin").is_some(),
                skin,
            );
            let cards = find_descendant_with_css_class(view.upcast_ref(), "albums-home-grid")
                .unwrap()
                .downcast::<gtk::FlowBox>()
                .unwrap();
            let flow_child = cards
                .first_child()
                .and_then(|child| child.downcast::<gtk::FlowBoxChild>().ok());
            assert_eq!(db::albums(&connection.borrow()).unwrap(), albums);
            let button = find_descendant_with_css_class(view.upcast_ref(), "album-card")
                .unwrap()
                .downcast::<gtk::Button>()
                .unwrap();
            if background {
                let cover =
                    find_descendant_with_css_class(button.upcast_ref(), "album-cover").unwrap();
                let rows =
                    find_descendant_with_css_class(view.upcast_ref(), "albums-bookshelf-rows")
                        .unwrap();
                assert!(!cards.is_visible());
                assert!(rows.is_visible());
                assert!(rows.height() >= BOOKSHELF_ROW_HEIGHT);
                assert!(
                    bookshelf_background_css_class(&connection.borrow())
                        .is_some_and(|background_class|
                            rows.has_css_class(&background_class),
                        ),
                    "the selected art must repeat on the rows box, whose top edge is the first row, so its tiles land exactly behind the shelves the rows draw"
                );
                assert!(
                    view.css_classes()
                        .into_iter()
                        .all(|name| !name.starts_with("albums-bookshelf-")),
                    "the repeating artwork must not hang on the scrolled window, whose background stays put while the rows scroll"
                );
                let photo_count = find_descendant_with_css_class(
                    button.upcast_ref(),
                    "albums-bookshelf-photo-count",
                )
                .unwrap()
                .downcast::<gtk::Label>()
                .unwrap();
                let count_color = photo_count.color();
                assert!((count_color.red() - 58.0 / 255.0).abs() < 0.01);
                assert!((count_color.green() - 33.0 / 255.0).abs() < 0.01);
                assert!((count_color.blue() - 15.0 / 255.0).abs() < 0.01);
                let theme = bookshelf_theme(&connection.borrow())
                .unwrap();
                assert_eq!(
                    button.margin_top() + cover.height_request(),
                    theme.surface_y
                );
                let row = button
                    .ancestor(gtk::Overlay::static_type())
                    .unwrap()
                    .downcast::<gtk::Overlay>()
                    .unwrap();
                let cover_bottom = cover
                    .compute_point(&row, &gtk::graphene::Point::new(0.0, cover.height() as f32))
                    .unwrap()
                    .y();
                assert!(
                    (cover_bottom - theme.surface_y as f32).abs() <= 1.0,
                    "allocated cover edge {cover_bottom} misses the shelf surface {}",
                    theme.surface_y
                );
            } else if style == "default" {
                assert_eq!(cards.row_spacing(), 32);
                assert!(!cards.vexpands());
                assert_eq!(cards.valign(), gtk::Align::Start);
                assert!(!cards.has_css_class("albums-bookshelf-0"));
                assert_eq!(flow_child.unwrap().height_request(), -1);
                assert_eq!(button.margin_top(), 0);
            }
            if skin {
                // The cover photo must sit inside the selected frame's own
                // transparent opening, not a single shared rectangle.
                let root = Path::new(ALBUM_COVER_THEME_DIRECTORY);
                let frame_paths = album_frame_paths_in(root);
                let index = selected_frame_index(&album, root, &frame_paths, 0).unwrap();
                let opening = frame_photo_opening(&frame_paths[index]);
                let cover =
                    find_descendant_with_css_class(button.upcast_ref(), "album-cover").unwrap();
                let (left, top, right, bottom) =
                    photo_margins(opening, cover.width_request(), cover.height_request());
                let picture =
                    find_descendant_with_css_class(cover.upcast_ref(), "thumbnail").unwrap();
                let actual = [
                    picture.margin_start(),
                    picture.margin_top(),
                    picture.margin_end(),
                    picture.margin_bottom(),
                ];
                for (actual, expected) in actual.into_iter().zip([left, top, right, bottom]) {
                    assert_eq!(actual, expected, "{}", frame_paths[index].display());
                }
            }
            opened.set(None);
            button.emit_clicked();
            assert_eq!(opened.get(), Some(album.id));
        }

        settings::set_bookshelf_enabled(&connection.borrow(), true).unwrap();
        settings::set_covers_enabled(&connection.borrow(), false).unwrap();
        db::set_setting(
            &connection.borrow(),
            crate::settings::ALBUM_BOOKSHELF_BACKGROUND_SETTING_KEY,
            "0",
        )
        .unwrap();
        refresh(
            &view,
            &albums,
            connection.clone(),
            136,
            on_album.clone(),
            Rc::new(|| {}),
        );
        settle_gtk_layout();
        let unframed_cards = find_descendant_with_css_class(view.upcast_ref(), "albums-home-grid")
            .unwrap()
            .downcast::<gtk::FlowBox>()
            .unwrap();
        assert!(!unframed_cards.is_visible());
        assert!(
            find_descendant_with_css_class(view.upcast_ref(), "albums-bookshelf-rows")
                .unwrap()
                .is_visible()
        );
        assert!(find_descendant_with_css_class(view.upcast_ref(), "album-skin").is_none());

        window.set_default_size(700, 600);
        settle_gtk_layout();
        for (index, theme) in bookshelf_themes_in(Path::new(BOOKSHELF_THEME_DIRECTORY))
            .into_iter()
            .enumerate()
            .filter(|(_, theme)| theme.kind == BookshelfThemeKind::Row)
        {
            db::set_setting(
                &connection.borrow(),
                crate::settings::ALBUM_BOOKSHELF_BACKGROUND_SETTING_KEY,
                &index.to_string(),
            )
            .unwrap();
            refresh(&view, &albums, connection.clone(), 136, Rc::new(|_| {}), Rc::new(|| {}));
            settle_gtk_layout();
            let rows = find_descendant_with_css_class(view.upcast_ref(), "albums-bookshelf-rows")
                .unwrap();
            assert!(rows.has_css_class(&format!("albums-bookshelf-{index}")));
            let cover = find_descendant_with_css_class(&rows, "album-cover").unwrap();
            let button = cover.ancestor(gtk::Button::static_type()).unwrap();
            let row = button.ancestor(gtk::Overlay::static_type()).unwrap();
            let bottom = cover
                .compute_point(&row, &gtk::graphene::Point::new(0.0, cover.height() as f32))
                .unwrap()
                .y();
            assert!(
                (bottom - theme.surface_y as f32).abs() <= 1.0,
                "{} cover edge {bottom} misses shelf {}",
                theme.id,
                theme.surface_y
            );
        }
        let rows = find_descendant_with_css_class(view.upcast_ref(), "albums-bookshelf-rows")
            .unwrap();
        let before = rows.compute_point(&view, &gtk::graphene::Point::new(0.0, 0.0)).unwrap().y();
        let adjustment = view.vadjustment();
        adjustment.set_value((adjustment.value() + 50.0).min(adjustment.upper() - adjustment.page_size()));
        settle_gtk_layout();
        let after = rows.compute_point(&view, &gtk::graphene::Point::new(0.0, 0.0)).unwrap().y();
        assert!((after - before).abs() >= 49.0, "shelf rows must scroll with their artwork");

        refresh(
            &view,
            &[],
            connection.clone(),
            136,
            on_album,
            Rc::new(|| {}),
        );
        settle_gtk_layout();
        let empty_cards = find_descendant_with_css_class(view.upcast_ref(), "albums-home-grid")
            .unwrap()
            .downcast::<gtk::FlowBox>()
            .unwrap();
        assert!(!empty_cards.is_visible());
        assert!(
            find_descendant_with_css_class(view.upcast_ref(), "albums-bookshelf-rows")
                .unwrap()
                .is_visible()
        );

        let legacy_index = bookshelf_themes_in(Path::new(BOOKSHELF_THEME_DIRECTORY))
            .iter()
            .position(|theme| theme.kind == BookshelfThemeKind::Legacy);
        if let Some(legacy_index) = legacy_index {
            db::set_setting(
                &connection.borrow(),
                crate::settings::ALBUM_BOOKSHELF_BACKGROUND_SETTING_KEY,
                &legacy_index.to_string(),
            )
            .unwrap();
            refresh(
                &view,
                &[],
                connection.clone(),
                136,
                Rc::new(|_| {}),
                Rc::new(|| {}),
            );
            settle_gtk_layout();
            assert!(!empty_cards.vexpands());
            assert_eq!(empty_cards.valign(), gtk::Align::Start);
            assert!(empty_cards.is_visible());
            assert!(view.has_css_class(&format!("albums-bookshelf-{legacy_index}")));
        }

        settings::disable_all_album_themes(&connection.borrow()).unwrap();
        refresh(
            &view,
            &[],
            connection.clone(),
            136,
            Rc::new(|_| {}),
            Rc::new(|| {}),
        );
        settle_gtk_layout();
        assert!(!empty_cards.vexpands());
        assert_eq!(empty_cards.valign(), gtk::Align::Start);
        assert_eq!(empty_cards.row_spacing(), 32);
        assert!(!view.has_css_class("albums-bookshelf"));

        window.close();
        drop(view);
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn one_skin_always_selects_zero() {
        let root = Path::new("images/theme/album-covers");
        let frames = vec![root.join("only/only-frame.png")];
        for id in 0..100 {
            assert_eq!(selected_frame_index(&album(id), root, &frames, 0), Some(0));
        }
    }

    #[test]
    fn selection_is_stable_when_album_contents_change() {
        let root = Path::new("images/theme/album-covers");
        let frames = vec![
            root.join("standard/standard-frame.png"),
            root.join("vintage/blue-frame.png"),
            root.join("vintage/green-frame.png"),
        ];
        let mut album = album(42);
        let selected = selected_frame_index(&album, root, &frames, 0);
        for _ in 0..100 {
            assert_eq!(selected_frame_index(&album, root, &frames, 0), selected);
        }
        album.photo_count = 83;
        album.created_at = 12345;
        assert_eq!(selected_frame_index(&album, root, &frames, 0), selected);
    }

    #[test]
    fn theme_variants_stay_inside_the_theme() {
        let root = Path::new("images/theme/album-covers");
        // A theme with one file always shows that file.
        let single = CoverTheme {
            name: "standard".to_string(),
            frames: vec![root.join("standard/standard-frame.png")],
        };
        for id in -100..100 {
            assert_eq!(
                single.frame_for(&album(id)),
                Some(&root.join("standard/standard-frame.png"))
            );
        }

        // A theme with several files spreads them over the albums, but the
        // chosen file always belongs to that theme.
        let variants = CoverTheme {
            name: "vintage".to_string(),
            frames: (0..6)
                .map(|variant| root.join(format!("vintage/{variant}-frame.png")))
                .collect(),
        };
        let mut selected = std::collections::HashSet::new();
        for id in -100..100 {
            let frame = variants.frame_for(&album(id)).unwrap();
            let index = variants
                .frames
                .iter()
                .position(|path| path == frame)
                .unwrap();
            assert_eq!(cover_theme_name(root, frame).as_deref(), Some("vintage"));
            selected.insert(index);
        }
        assert!(selected.len() > 1);
    }

    #[test]
    fn coverflow_visible_window_is_bounded_and_focus_survives_reorder() {
        assert_eq!(visible_album_indices(0, 1000), 0..5);
        assert_eq!(visible_album_indices(500, 1000), 496..505);
        assert_eq!(visible_album_indices(999, 1000), 995..1000);
        assert_eq!(visible_album_indices(0, 0), 0..0);
        let albums = vec![album(3), album(7), album(11)];
        assert_eq!(restored_album_focus(&albums, Some(7), 0), 1);
        assert_eq!(restored_album_focus(&albums, Some(9), 2), 2);
    }

    #[test]
    fn bookshelf_reflection_strength_is_bounded_and_uses_surface_room() {
        let directory = std::env::temp_dir().join(format!(
            "pic-reflection-theme-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("row.png"), []).unwrap();
        std::fs::write(
            directory.join("theme.conf"),
            "row_height=288\nsurface_y=240\nreflection_strength=0.35\n",
        )
        .unwrap();
        let theme = bookshelf_themes_in(directory.parent().unwrap())
            .into_iter()
            .find(|theme| theme.image_path == directory.join("row.png"))
            .unwrap();
        assert_eq!(theme.reflection_strength, 35);
        assert_eq!(bookshelf_reflection_height(&theme), 44);
        std::fs::write(
            directory.join("theme.conf"),
            "row_height=288\nsurface_y=284\nreflection_strength=2.0\n",
        )
        .unwrap();
        let theme = bookshelf_themes_in(directory.parent().unwrap())
            .into_iter()
            .find(|theme| theme.image_path == directory.join("row.png"))
            .unwrap();
        assert_eq!(theme.reflection_strength, 0);
        assert_eq!(bookshelf_reflection_height(&theme), 0);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn shipped_light_and_dark_shelves_enable_reflection() {
        let themes = bookshelf_themes_in(Path::new(BOOKSHELF_THEME_DIRECTORY));
        for id in ["marble", "darkwood"] {
            let theme = themes.iter().find(|theme| theme.id == id).unwrap();
            assert!(theme.reflection_strength > 0);
            assert!(bookshelf_reflection_height(theme) > 0);
        }
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn bookshelf_reflection_fades_below_a_live_album_cover() {
        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-shelf-reflection-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let photo_path = directory.join("red.png");
        image::RgbImage::from_pixel(120, 80, image::Rgb([240, 30, 20]))
            .save(&photo_path)
            .unwrap();
        let connection = Rc::new(RefCell::new(
            db::open(&directory.join("library.db")).unwrap(),
        ));
        let album = db::create_album(&connection.borrow(), "Red").unwrap();
        connection
            .borrow()
            .execute(
                "INSERT INTO photos (path) VALUES (?1)",
                [photo_path.to_string_lossy().as_ref()],
            )
            .unwrap();
        let id = connection.borrow().last_insert_rowid();
        db::add_photos_to_album(&connection.borrow(), album.id, &[id]).unwrap();
        db::set_album_cover_photo(&connection.borrow(), album.id, id).unwrap();
        settings::set_album_layout(&connection.borrow(), settings::AlbumLayout::Bookshelf).unwrap();
        let themes = bookshelf_themes_in(Path::new(BOOKSHELF_THEME_DIRECTORY));
        let marble = themes
            .iter()
            .position(|theme| theme.id == "marble")
            .unwrap();
        db::set_setting(
            &connection.borrow(),
            settings::ALBUM_BOOKSHELF_BACKGROUND_SETTING_KEY,
            &marble.to_string(),
        )
        .unwrap();
        let albums = db::albums(&connection.borrow()).unwrap();
        let view = build(
            &albums,
            connection.clone(),
            136,
            Rc::new(|_| {}),
            Rc::new(|| {}),
            Rc::new(|| {}),
        );
        let window = gtk::Window::builder()
            .default_width(1000)
            .default_height(780)
            .child(&view)
            .build();
        window.present();
        settle_gtk_layout();
        let reflection =
            find_descendant_with_css_class(view.upcast_ref(), "albums-bookshelf-reflection")
                .expect("reflection missing");
        assert!(reflection.height() > 15);
        assert!(reflection.is_mapped());
        let reflection_impl = reflection.clone().downcast::<ShelfReflection>().unwrap();
        let snapshot = gtk::Snapshot::new();
        reflection_impl.imp().snapshot(&snapshot);
        let node = snapshot.to_node().expect("reflection should paint");
        let texture = window.renderer().unwrap().render_texture(&node, None);
        let width = texture.width() as usize;
        let height = texture.height() as usize;
        let mut pixels = vec![0_u8; width * height * 4];
        texture.download(&mut pixels, width * 4);
        let alpha = |x: usize, y: usize| pixels[(y * width + x) * 4 + 3];
        assert!(alpha(width / 2, 2) > alpha(width / 2, height - 3));
        let marble_alpha = alpha(width / 2, 2);
        let darkwood = themes
            .iter()
            .position(|theme| theme.id == "darkwood")
            .unwrap();
        db::set_setting(
            &connection.borrow(),
            settings::ALBUM_BOOKSHELF_BACKGROUND_SETTING_KEY,
            &darkwood.to_string(),
        )
        .unwrap();
        refresh(
            &view,
            &albums,
            connection.clone(),
            136,
            Rc::new(|_| {}),
            Rc::new(|| {}),
        );
        settle_gtk_layout();
        let darker =
            find_descendant_with_css_class(view.upcast_ref(), "albums-bookshelf-reflection")
                .unwrap()
                .downcast::<ShelfReflection>()
                .unwrap();
        let snapshot = gtk::Snapshot::new();
        darker.imp().snapshot(&snapshot);
        let node = snapshot
            .to_node()
            .expect("new shelf reflection should paint");
        let texture = window.renderer().unwrap().render_texture(&node, None);
        let width = texture.width() as usize;
        let mut pixels = vec![0_u8; width * texture.height() as usize * 4];
        texture.download(&mut pixels, width * 4);
        assert!(pixels[(2 * width + width / 2) * 4 + 3] < marble_alpha);
        window.close();
        drop(view);
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn album_layout_control_shows_coverflow_and_persists_it() {
        adw::init().unwrap();
        let path = std::env::temp_dir().join(format!(
            "pic-coverflow-layout-{}-{}.db",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Rc::new(RefCell::new(db::open(&path).unwrap()));
        let mut cached_thumbnails = Vec::new();
        let mut photo_files = Vec::new();
        for index in 0..8 {
            let name = format!("Album {}", char::from(b'A' + index));
            let album = db::create_album(&connection.borrow(), &name).unwrap();
            let photo_path = path.with_file_name(format!("cover-{index}.png"));
            let photo = image::RgbImage::from_fn(320, 320, |x, y| {
                if x < 12 || y < 12 || x >= 308 || y >= 308 {
                    image::Rgb([245, 245, 245])
                } else if x < 54 && y < 54 {
                    image::Rgb([255, 220, 35])
                } else {
                    image::Rgb([
                        35 + index as u8 * 25,
                        80 + index as u8 * 13,
                        190 - index as u8 * 17,
                    ])
                }
            });
            photo.save(&photo_path).unwrap();
            photo_files.push(photo_path.clone());
            let size_bytes = std::fs::metadata(&photo_path).unwrap().len() as i64;
            connection
                .borrow()
                .execute(
                    "INSERT INTO photos(path, width, height, size_bytes, mtime, added_at) \
                 VALUES (?1, 320, 320, ?2, 0, ?3)",
                    rusqlite::params![photo_path.to_string_lossy().as_ref(), size_bytes, index],
                )
                .unwrap();
            let photo_id = connection.borrow().last_insert_rowid();
            db::add_photos_to_album(&connection.borrow(), album.id, &[photo_id]).unwrap();
            db::set_album_cover_photo(&connection.borrow(), album.id, photo_id).unwrap();
            let cache_path = crate::thumbnail::cache_path(
                &photo_path.to_string_lossy(),
                Some(0),
                Some(size_bytes),
            )
            .unwrap();
            std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
            std::fs::copy(&photo_path, &cache_path).unwrap();
            cached_thumbnails.push(cache_path);
        }
        let albums = db::albums(&connection.borrow()).unwrap();
        let view = build(
            &albums,
            connection.clone(),
            136,
            Rc::new(|_| {}),
            Rc::new(|| {}),
            Rc::new(|| {}),
        );
        let screenshot_dir = std::env::temp_dir().join("pic-coverflow-gui-validation");
        std::fs::create_dir_all(&screenshot_dir).unwrap();
        let app = adw::Application::builder().application_id("io.pic.CoverflowRegression").build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let full_app = std::env::var_os("PIC_COVERFLOW_FULL_APP").is_some();
        let window = if full_app {
            db::set_setting(&connection.borrow(), "last-view", "albums").unwrap();
            db::set_setting(&connection.borrow(), "onboarding-never-show", "true").unwrap();
            let window = crate::window::build(&app, db::open(&path).unwrap());
            window.set_title(Some("PIC Cover Flow Wheel Validation"));
            window.upcast::<gtk::Window>()
        } else { gtk::Window::builder()
            .title("PIC Cover Flow Wheel Validation")
            .decorated(false)
            .default_width(1200)
            .default_height(850)
            .child(&view)
            .build() };
        window.present();
        settle_gtk_layout();
        settle_gtk_layout();
        let view = if full_app {
            let grid = find_descendant_with_css_class(window.upcast_ref(), "albums-home-grid").unwrap();
            grid.ancestor(gtk::ScrolledWindow::static_type()).unwrap().downcast::<gtk::ScrolledWindow>().unwrap()
        } else { view };
        let layout = find_descendant_with_css_class(view.upcast_ref(), "albums-layout")
            .unwrap()
            .downcast::<gtk::DropDown>()
            .unwrap();
        layout.set_selected(2);
        settle_gtk_layout();
        assert_eq!(
            settings::album_layout(&connection.borrow()),
            settings::AlbumLayout::CoverFlow
        );
        let flow = find_descendant_with_css_class(view.upcast_ref(), "albums-coverflow-canvas")
            .expect("Cover Flow canvas should be mounted");
        assert!(flow.is_visible());
        let name = find_descendant_with_css_class(view.upcast_ref(), "albums-coverflow-name")
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        assert_eq!(name.text().as_str(), "Album A");
        let canvas_bounds = flow.compute_bounds(&window).unwrap();
        let title_bounds = name.compute_bounds(&window).unwrap();
        assert!(
            ((canvas_bounds.x() + canvas_bounds.width() / 2.0)
                - (title_bounds.x() + title_bounds.width() / 2.0))
                .abs()
                < 2.0,
            "focused album title is centered below the Cover Flow"
        );
        assert!(
            title_bounds.y() >= canvas_bounds.y() + canvas_bounds.height(),
            "focused album title sits below the reflections"
        );
        save_album_snapshot(&window, &screenshot_dir.join("01-focus-a-wide.png"));

        let bounds = flow.compute_bounds(&window).unwrap();
        let x = Cell::new((bounds.x() + bounds.width() / 2.0).round() as i32);
        let y = Cell::new((bounds.y() + 220.0).round() as i32);
        let window_id = std::process::Command::new("xdotool")
            .args([
                "search",
                "--onlyvisible",
                "--name",
                "^PIC Cover Flow Wheel Validation$",
            ])
            .output()
            .expect("xdotool is required for the GTK wheel interaction test");
        assert!(window_id.status.success(), "could not find the test window");
        let window_id = String::from_utf8_lossy(&window_id.stdout)
            .lines()
            .last()
            .expect("xdotool should return a window ID")
            .to_string();
        let wheel = |button: &str| {
            let geometry = std::process::Command::new("xdotool")
                .args(["getwindowgeometry", "--shell", &window_id])
                .output()
                .expect("get GTK window screen origin");
            assert!(geometry.status.success(), "could not get GTK window geometry");
            let geometry = String::from_utf8_lossy(&geometry.stdout);
            let origin = |key: &str| -> i32 {
                geometry
                    .lines()
                    .find_map(|line| line.strip_prefix(key))
                    .and_then(|value| value.parse().ok())
                    .unwrap_or_else(|| panic!("missing {key} in xdotool geometry: {geometry}"))
            };
            let screen_x = origin("X=") + x.get();
            let screen_y = origin("Y=") + y.get();
            let output = std::process::Command::new("xdotool")
                .args(["mousemove", "--sync", &screen_x.to_string(), &screen_y.to_string()])
                .output()
                .expect("position pointer over Cover Flow");
            assert!(output.status.success(), "xdotool pointer positioning failed");
            let location = std::process::Command::new("xdotool")
                .args(["getmouselocation", "--shell"])
                .output()
                .expect("verify pointer location");
            assert!(location.status.success(), "xdotool could not report pointer location");
            let location = String::from_utf8_lossy(&location.stdout);
            eprintln!(
                "Cover Flow pointer widget=({}, {}) screen=({}, {}) actual=[{}]",
                x.get(), y.get(), screen_x, screen_y, location.trim().replace('\n', " ")
            );
            assert!(location.lines().any(|line| line == format!("X={screen_x}")));
            assert!(location.lines().any(|line| line == format!("Y={screen_y}")));
            let local_x = (screen_x - origin("X=")) as f64;
            let local_y = (screen_y - origin("Y=")) as f64;
            let picked = window.pick(local_x, local_y, gtk::PickFlags::DEFAULT);
            eprintln!(
                "Cover Flow GTK pick at ({local_x:.0}, {local_y:.0}): {:?}; canvas_target={} canvas_bounds=({}, {}, {}, {})",
                picked.as_ref().map(|widget| widget.type_().name()),
                picked.as_ref().is_some_and(|widget| *widget == flow || widget.is_ancestor(&flow)),
                bounds.x(), bounds.y(), bounds.width(), bounds.height()
            );
            let click = std::process::Command::new("xdotool")
                .args(["click", button])
                .output()
                .expect("send physical-style X11 wheel button event");
            assert!(click.status.success(), "xdotool mouse wheel failed");
            let context = gtk::glib::MainContext::default();
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(700);
            while std::time::Instant::now() < deadline {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        wheel("5");
        assert_eq!(
            name.text().as_str(),
            "Album B",
            "wheel down advances album focus"
        );
        save_album_snapshot(&window, &screenshot_dir.join("02-focus-b-wheel-down.png"));
        wheel("7");
        assert_eq!(
            name.text().as_str(),
            "Album C",
            "horizontal wheel advances album focus"
        );
        save_album_snapshot(&window, &screenshot_dir.join("03-focus-c-wheel-right.png"));
        for index in 3..8 {
            wheel("5");
            let expected = format!("Album {}", char::from(b'A' + index));
            assert_eq!(
                name.text().as_str(),
                expected,
                "wheel down advances to {expected}"
            );
            save_album_snapshot(
                &window,
                &screenshot_dir.join(format!("focus-{}.png", char::from(b'A' + index))),
            );
        }
        assert_eq!(name.text().as_str(), "Album H", "wheel stops at the final album");
        for index in (0..7).rev() {
            wheel("4");
            let expected = format!("Album {}", char::from(b'A' + index));
            assert_eq!(
                name.text().as_str(),
                expected,
                "wheel up moves back to {expected}"
            );
        }
        wheel("7");
        assert_eq!(
            name.text().as_str(),
            "Album B",
            "horizontal wheel moves album focus right"
        );
        save_album_snapshot(&window, &screenshot_dir.join("wheel-right.png"));
        wheel("6");
        assert_eq!(
            name.text().as_str(),
            "Album A",
            "horizontal wheel moves album focus left"
        );

        let wide_canvas_width = flow.width();
        let resized = std::process::Command::new("xdotool")
            .args(["windowsize", &window_id, "760", "680"])
            .status()
            .expect("resize the GTK test window");
        assert!(resized.success(), "xdotool could not resize the GTK window");
        settle_gtk_layout();
        settle_gtk_layout();
        assert!(
            flow.width() < wide_canvas_width,
            "Cover Flow adapts to a narrower window"
        );
        save_album_snapshot(&window, &screenshot_dir.join("04-focus-a-narrow.png"));
        // Exercise the same event path when the parent scroller has overflow.
        assert!(std::process::Command::new("xdotool")
            .args(["windowsize", &window_id, "760", "480"]).status().unwrap().success());
        settle_gtk_layout();
        settle_gtk_layout();
        view.vadjustment().set_value(0.0);
        settle_gtk_layout();
        // The full application animates its responsive sidebar drawer.
        // Wait until GTK actually picks the gallery, not the modal shade.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let bounds = flow.compute_bounds(&window).unwrap();
            let picked = window.pick((bounds.x() + bounds.width() / 2.0) as f64,
                (bounds.y() + 160.0) as f64, gtk::PickFlags::DEFAULT);
            if picked.as_ref().is_some_and(|w| *w == flow || w.is_ancestor(&flow)) { break; }
            if std::time::Instant::now() >= deadline {
                panic!("responsive sidebar still intercepts gallery pointer: {:?}", picked.map(|w| w.type_().name()));
            }
            settle_gtk_layout();
        }
        let bounds = flow.compute_bounds(&window).unwrap();
        x.set((bounds.x() + bounds.width() * 0.5) as i32);
        y.set((bounds.y() + 160.0) as i32);
        save_album_snapshot(&window, &screenshot_dir.join("short-before-wheel.png"));
        wheel("5");
        assert_eq!(name.text().as_str(), "Album B", "wheel must navigate in a short, overflowing page");
        wheel("4");
        assert_eq!(name.text().as_str(), "Album A");
        x.set((bounds.x() + bounds.width() * 0.8) as i32);
        wheel("5");
        assert_eq!(name.text().as_str(), "Album B", "wheel over side cover");
        x.set((bounds.x() + 12.0) as i32);
        y.set((bounds.y() + 12.0) as i32);
        wheel("4");
        assert_eq!(name.text().as_str(), "Album A", "wheel over empty background");
        wheel("4");
        assert_eq!(name.text().as_str(), "Album A", "first boundary");
        window.close();
        drop(view);
        drop(connection);
        for cache_path in cached_thumbnails {
            let _ = std::fs::remove_file(cache_path);
        }
        for photo_file in photo_files {
            let _ = std::fs::remove_file(photo_file);
        }
        std::fs::remove_file(path).unwrap();
        eprintln!(
            "Cover Flow visual validation screenshots: {}",
            screenshot_dir.display()
        );
    }
}
