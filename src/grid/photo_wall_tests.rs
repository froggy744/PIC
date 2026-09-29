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

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_view_menu_keeps_selection_and_grid_scroll_context() {
    gtk::init().unwrap();
    let gallery = Rc::new(Gallery::new(
        &[],
        160,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    let objects = (0..2000)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property("width", 6000_i64)
                .property("height", 4000_i64)
                .build()
        })
        .collect::<Vec<_>>();
    gallery.current_photos.replace(objects.clone());
    gallery.store.splice(0, 0, &objects);
    let grid_scroll = gtk::ScrolledWindow::builder().child(&gallery.root).build();
    let wall_scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&wall_scroll);
    let stack = gtk::Stack::new();
    stack.add_named(&grid_scroll, Some("grid"));
    stack.add_named(&wall_scroll, Some("wall"));
    let stack_for_mode = stack.clone();
    let gallery_for_mode = gallery.clone();
    gallery.set_folder_view_changed_handler(move |_| {
        stack_for_mode.set_visible_child_name(if gallery_for_mode.using_virtual_photo_surface() {
            "wall"
        } else {
            "grid"
        })
    });
    let info = crate::infobar::InfoBar::new();
    let gallery_for_menu = gallery.clone();
    info.connect_photo_layout(move |mode| gallery_for_menu.set_layout(mode));
    let collage_calls = Rc::new(Cell::new(0));
    let calls = collage_calls.clone();
    info.collage
        .connect_clicked(move |_| calls.set(calls.get() + 1));
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&stack);
    root.append(&info.root);
    let window = gtk::Window::builder()
        .default_width(1000)
        .default_height(700)
        .child(&root)
        .build();
    window.present();
    settle();
    gallery.update_width(1000);
    grid_scroll.vadjustment().set_value(3000.0);
    settle();
    gallery.selection.unselect_all();
    gallery.selection.select_item(42, false);
    gallery.selection.select_item(73, false);
    let anchor = gallery.capture_view_anchor().unwrap();
    let reads = crate::source::original_read_count();
    info.view_wall.set_active(true);
    settle();
    assert_eq!(gallery.layout(), PhotoLayout::PhotoWall);
    assert_eq!(stack.visible_child_name().as_deref(), Some("wall"));
    let index = objects
        .iter()
        .position(|p| p.id() == anchor.photo_id)
        .unwrap();
    let offset = gallery.sectioned_folder.y_for_index(index as u32).unwrap()
        - wall_scroll.vadjustment().value();
    assert!(
        (offset - anchor.viewport_y_offset).abs() < 3.0,
        "Grid to Wall lost anchor: {offset} vs {}",
        anchor.viewport_y_offset
    );
    let first_row = gallery
        .sectioned_folder
        .wall_state
        .borrow()
        .layout
        .visible_rows(
            wall_scroll.vadjustment().value(),
            wall_scroll.vadjustment().value() + 1.0,
        )
        .start;
    let first_index = {
        let state = gallery.sectioned_folder.wall_state.borrow();
        state.layout.items[state.layout.rows[first_row].item_range.start].photo_index
    };
    assert_eq!(
        gallery.index_for_scroll_position(wall_scroll.vadjustment().value()),
        first_index,
        "date heading used Grid column arithmetic"
    );
    gallery.wheel_zoom_in();
    settle();
    assert_eq!(gallery.current_zoom_width(), 187);
    window.set_default_size(700, 700);
    settle();
    info.view_grid.set_active(true);
    settle();
    assert_eq!(gallery.layout(), PhotoLayout::Grid);
    assert_eq!(stack.visible_child_name().as_deref(), Some("grid"));
    assert!(gallery.selection.is_selected(42) && gallery.selection.is_selected(73));
    assert_eq!(
        gallery.store.item(73).unwrap(),
        objects[73].clone().upcast::<glib::Object>()
    );
    assert_eq!(crate::source::original_read_count(), reads);
    info.collage.emit_clicked();
    assert_eq!(collage_calls.get(), 1);
    window.close();
}
