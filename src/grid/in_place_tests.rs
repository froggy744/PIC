//! GTK regression recording for visibility, cancellation and stationary cells.
use super::*;

fn pump(ms: u64) {
    let until = Instant::now() + std::time::Duration::from_millis(ms);
    let context = glib::MainContext::default();
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
#[ignore = "requires a GTK display; records PNG frames when PICASA_TRANSITION_RECORD_DIR is set"]
fn in_place_transitions_remain_visible_and_stationary() {
    gtk::init().unwrap();
    crate::css::install_foundation(&gtk::gdk::Display::default().unwrap());
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(crate::db::SCHEMA).unwrap();
    for folder in 1..=3 {
        db.execute(
            "INSERT INTO folders(id,path,name) VALUES(?1,?2,?3)",
            rusqlite::params![
                folder,
                format!("/transition/folder-{folder}"),
                format!("Folder {folder}")
            ],
        )
        .unwrap();
    }
    for id in 1..=30 {
        let path = std::env::current_dir()
            .unwrap()
            .join(format!("samples/ZoomOUT-{id}.jpg"));
        db.execute(
            "INSERT INTO photos(id,path,folder_id) VALUES(?1,?2,?3)",
            rusqlite::params![id, path.to_string_lossy(), (id - 1) / 10 + 1],
        )
        .unwrap();
    }
    let photos = crate::db::photos(&db, None, false, None).unwrap();
    for case in ["library-resize", "folder-resize", "folder-zoom"] {
        let gallery = Rc::new(Gallery::new(
            &photos,
            120,
            |_| {},
            |_, _, _| {},
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
        ));
        let folder = case.starts_with("folder");
        let scroll = gtk::ScrolledWindow::new();
        if folder {
            gallery.set_grouping(GroupMode::Folder, GroupDate::Taken);
            gallery.replace(&photos);
            scroll.set_child(Some(&gallery.folder_sectioned_root));
            gallery.attach_sectioned_folder_scroll(&scroll);
        } else {
            scroll.set_child(Some(&gallery.root));
        }
        for photo in gallery.photo_objects() {
            photo.set_property("cached-thumbnail-path", photo.path());
            photo.set_property("thumbnail-available", true);
            photo.set_property("original-available", true);
            let texture = gtk::gdk::Texture::from_file(&gio::File::for_path(photo.path())).unwrap();
            folder_thumbnail_cache_insert(
                photo_presentation_key(&photo).unwrap(),
                texture.upcast(),
            );
        }
        if folder {
            gallery.sectioned_folder.refresh_model();
        }
        let window = gtk::Window::builder()
            .title(case)
            .default_width(850)
            .default_height(600)
            .child(&scroll)
            .build();
        window.settings().set_gtk_enable_animations(true);
        window.present();
        pump(250);
        gallery.update_width_with_reflow(850);
        pump(250);
        let directory = std::env::var_os("PICASA_TRANSITION_RECORD_DIR")
            .map(|dir| std::path::PathBuf::from(dir).join(case));
        if let Some(dir) = &directory {
            std::fs::create_dir_all(dir).unwrap();
        }
        let original_objects = gallery.photo_objects();
        gallery.selection.select_item(3, true);
        let original_selection = gallery.selected_photo_ids(None);
        let mut frame = 0;
        let mut previous = HashMap::new();
        let mut effect_frames = 0;
        let mut capture = |stationary: bool| {
            pump(17);
            let root: gtk::Widget = if folder {
                gallery.folder_sectioned_root.clone().upcast()
            } else {
                gallery.root.clone().upcast()
            };
            let mut tiles = Vec::new();
            collect_tiles(&root, &mut tiles);
            let mut cells = HashMap::new();
            let mut visible = 0;
            for tile in tiles
                .iter()
                .filter(|tile| tile.is_mapped() && tile.photo().is_some())
            {
                assert!(tile.is_visible(), "{case} frame {frame}: hidden tile");
                assert_eq!(tile.opacity(), 1.0, "{case}: widget opacity changed");
                assert!(
                    tile.transition_paintable().is_some(),
                    "{case} frame {frame}: missing photo pixels"
                );
                let bounds = tile.compute_bounds(&root).unwrap();
                let cell = (bounds.x(), bounds.y(), bounds.width(), bounds.height());
                let id = tile.photo().unwrap().id();
                if stationary {
                    if let Some(old) = previous.get(&id) {
                        assert_eq!(
                            *old, cell,
                            "{case} frame {frame}: tile {id} moved during tween"
                        );
                    }
                }
                let viewport = tile.compute_bounds(&scroll).unwrap();
                if viewport.y() + viewport.height() > 0.0 && viewport.y() < scroll.height() as f32 {
                    visible += 1;
                }
                let (scale_x, scale_y) = tile.presentation_scale();
                let (translate_x, translate_y) = tile.presentation_translate();
                if (scale_x - 1.0).abs() > 0.001
                    || (scale_y - 1.0).abs() > 0.001
                    || translate_x.abs() > 0.1
                    || translate_y.abs() > 0.1
                {
                    effect_frames += 1;
                }
                cells.insert(id, cell);
            }
            assert!(visible > 0, "{case} frame {frame}: empty viewport");
            previous = cells;
            if let Some(dir) = &directory {
                let paintable = gtk::WidgetPaintable::new(Some(&window));
                let mut node = None;
                for _ in 0..5 {
                    let snapshot = gtk::Snapshot::new();
                    paintable.snapshot(
                        &snapshot,
                        window.width().max(1) as f64,
                        window.height().max(1) as f64,
                    );
                    node = snapshot.to_node();
                    if node.is_some() {
                        break;
                    }
                    pump(17);
                }
                let node = node.expect("nonblank render node after paint retry");
                window
                    .renderer()
                    .unwrap()
                    .render_texture(
                        &node,
                        Some(&gtk::graphene::Rect::new(
                            0.0,
                            0.0,
                            window.width().max(1) as f32,
                            window.height().max(1) as f32,
                        )),
                    )
                    .save_to_png(dir.join(format!("{frame:04}.png")))
                    .unwrap();
            }
            frame += 1;
        };
        for _ in 0..10 {
            capture(false);
        }
        for target in [920, 760, 1000, 700, 850] {
            if case == "folder-zoom" {
                gallery.request_zoom(if target > 850 { 168 } else { 120 });
            } else {
                window.set_default_size(target, 600);
                pump(25);
                gallery.update_width_with_reflow(target);
            }
            capture(false);
            capture(false);
            for _ in 0..16 {
                capture(true);
            }
        }
        // Repeated changes before the current tween finishes.
        for step in 0..12 {
            if case == "folder-zoom" {
                gallery.request_zoom(if step % 2 == 0 { 144 } else { 168 });
            } else {
                let width = 700 + step * 20;
                window.set_default_size(width, 600);
                pump(20);
                gallery.update_width_with_reflow(width);
            }
            capture(false);
        }
        capture(false);
        for _ in 0..20 {
            capture(true);
        }
        drop(capture);
        assert!(effect_frames > 0, "{case}: tween never ran");
        gallery.request_zoom(gallery.current_zoom_width() + 24);
        pump(35);
        gallery.cancel_zoom_transition();
        let mut tiles = Vec::new();
        let root: gtk::Widget = if folder {
            gallery.folder_sectioned_root.clone().upcast()
        } else {
            gallery.root.clone().upcast()
        };
        collect_tiles(&root, &mut tiles);
        for tile in tiles {
            assert_eq!(tile.presentation_scale(), (1.0, 1.0));
            assert_eq!(tile.presentation_translate(), (0.0, 0.0));
        }
        eprintln!("{case}: checked {frame} frames; {effect_frames} tile effect samples; no blank viewport; FLIP transforms ran while destination allocations stayed stable");
        assert_eq!(gallery.photo_objects(), original_objects);
        assert_eq!(gallery.selected_photo_ids(None), original_selection);
        window.settings().set_gtk_enable_animations(false);
        gallery.request_zoom(120);
        assert_eq!(gallery.current_zoom_width(), 120);
        assert!(!gallery.sectioned_folder.tween.is_active());
        window.settings().set_gtk_enable_animations(true);
        window.close();
        pump(50);
    }
}
