use super::*;

fn settle() {
    let context = glib::MainContext::default();
    for _ in 0..30 {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_reuses_models_and_virtualizes_headerless_and_folder_collections() {
    gtk::init().unwrap();
    let gallery = Rc::new(Gallery::new(
        &[],
        100,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    let objects = (0..14_361)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property("width", if i % 2 == 0 { 4000_i64 } else { 6000_i64 })
                .property("height", if i % 2 == 0 { 6000_i64 } else { 4000_i64 })
                .property("folder-id", if i < 7000 { 1_i64 } else { 2_i64 })
                .build()
        })
        .collect::<Vec<_>>();
    gallery.store.splice(0, 0, &objects);
    gallery.current_photos.replace(objects.clone());
    gallery.selection.select_item(42, false);
    gallery.selection.select_item(73, false);
    gallery.set_layout(PhotoLayout::PhotoWall);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&scroll);
    let window = gtk::Window::builder()
        .default_width(1000)
        .default_height(600)
        .child(&scroll)
        .build();
    window.present();
    settle();
    let surface = &gallery.sectioned_folder;
    assert!(surface.live_headers.borrow().is_empty());
    assert!(surface.live_tiles.borrow().len() > 10 && surface.live_tiles.borrow().len() < 300);
    assert_eq!(
        gallery.store.item(42).unwrap(),
        objects[42].clone().upcast::<glib::Object>()
    );
    assert!(gallery.selection.is_selected(42) && gallery.selection.is_selected(73));
    let height = surface.wall_state.borrow().layout.total_height;
    scroll.vadjustment().set_value(height * 0.5);
    settle();
    assert!(surface
        .live_tiles
        .borrow()
        .keys()
        .any(|index| *index > 1000));
    assert!(surface.live_tiles.borrow().len() < 300);
    gallery.group_mode.set(GroupMode::Folder);
    gallery.rebuild_group_ranges();
    surface.refresh_model();
    settle();
    assert_eq!(surface.wall_state.borrow().layout.sections.len(), 2);
    for row in &surface.wall_state.borrow().layout.rows {
        let layout = surface.wall_state.borrow();
        let section = layout.layout.items[row.item_range.start].section;
        assert!(layout.layout.items[row.item_range.clone()]
            .iter()
            .all(|item| item.section == section));
    }
    gallery.set_layout(PhotoLayout::Grid);
    settle();
    assert!(gallery.selection.is_selected(42) && gallery.selection.is_selected(73));
    assert_eq!(gallery.store.n_items(), 14_361);
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_anchor_generation_and_ultrawide_viewport() {
    gtk::init().unwrap();
    let gallery = Rc::new(Gallery::new(
        &[],
        100,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    let objects = (0..10_000)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property("width", 4000_i64)
                .property("height", 6000_i64)
                .build()
        })
        .collect::<Vec<_>>();
    gallery.store.splice(0, 0, &objects);
    gallery.current_photos.replace(objects.clone());
    gallery.set_layout(PhotoLayout::PhotoWall);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&scroll);
    let window = gtk::Window::builder()
        .default_width(3800)
        .default_height(1100)
        .child(&scroll)
        .build();
    window.present();
    settle();
    assert!(gallery.sectioned_folder.live_tiles.borrow().len() > 180);
    scroll.vadjustment().set_value(6000.0);
    settle();
    let anchor = gallery.capture_view_anchor().unwrap();
    let original_reads = crate::source::original_read_count();
    let generation = gallery.sectioned_folder.wall_state.borrow().generation;
    gallery.request_slider_zoom(160);
    settle();
    let index = objects
        .iter()
        .position(|p| p.id() == anchor.photo_id)
        .unwrap();
    let offset =
        gallery.sectioned_folder.y_for_index(index as u32).unwrap() - scroll.vadjustment().value();
    assert!(
        (offset - anchor.viewport_y_offset).abs() < 2.0,
        "anchor moved: {offset} vs {}",
        anchor.viewport_y_offset
    );
    assert!(gallery.sectioned_folder.wall_state.borrow().generation > generation);
    let generation = gallery.sectioned_folder.wall_state.borrow().generation;
    scroll
        .vadjustment()
        .set_value(scroll.vadjustment().value() + 100.0);
    settle();
    assert_eq!(
        gallery.sectioned_folder.wall_state.borrow().generation,
        generation
    );
    gallery.restore_view_anchor(anchor);
    gallery.sectioned_folder.invalidate_geometry();
    let value = scroll.vadjustment().value();
    settle();
    assert_eq!(
        scroll.vadjustment().value(),
        value,
        "stale callback restored the scroll"
    );
    assert_eq!(
        crate::source::original_read_count(),
        original_reads,
        "Photo Wall geometry read an original"
    );
    window.close();
}
