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
    let activated = Rc::new(Cell::new(None::<i64>));
    let activated_for_open = activated.clone();
    let context_photo = Rc::new(Cell::new(None::<i64>));
    let context_for_menu = context_photo.clone();
    let gallery = Rc::new(Gallery::new(
        &[],
        100,
        |_| {},
        move |photos, index, _| activated_for_open.set(Some(photos[index].id())),
        move |photo, _, _, _| context_for_menu.set(Some(photo.id())),
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
    // In-memory cached thumbnails exercise cache-only binding without originals.
    let reads_before = crate::source::original_read_count();
    for (index, photo) in objects.iter().take(256).enumerate() {
        photo.set_cached_thumbnail_path(format!("/virtual/photo-wall-cache/{}.png", photo.id()));
        let (width, height) = if index % 2 == 0 { (40, 60) } else { (60, 40) };
        let mut pixels = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[
                    (60 + index % 120) as u8,
                    (80 + x * 2) as u8,
                    (80 + y * 2) as u8,
                    255,
                ]);
            }
        }
        let texture = gtk::gdk::MemoryTexture::new(
            width as i32,
            height as i32,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(pixels),
            width * 4,
        );
        let paintable: gtk::gdk::Paintable = texture.upcast();
        folder_thumbnail_cache_insert(photo_presentation_key(photo).unwrap(), paintable);
    }
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
    let first_index = *surface.live_tiles.borrow().keys().min().unwrap();
    let tile = surface.live_tiles.borrow()[&first_index].tile.clone();
    assert!(
        tile.transition_paintable().is_some(),
        "RAM cache thumbnail was not painted"
    );
    let controllers = tile.observe_controllers();
    for i in 0..controllers.n_items() {
        if let Some(click) = controllers.item(i).and_downcast::<gtk::GestureClick>() {
            if click.button() == 1 {
                click.emit_by_name::<()>("pressed", &[&2_i32, &0.0_f64, &0.0_f64]);
            }
            if click.button() == 3 {
                click.emit_by_name::<()>("pressed", &[&1_i32, &0.0_f64, &0.0_f64]);
            }
        }
    }
    assert_eq!(activated.get(), Some(objects[first_index as usize].id()));
    assert_eq!(context_photo.get(), activated.get());
    gallery.selection.select_item(42, false);
    gallery.selection.select_item(73, false);
    assert_eq!(crate::source::original_read_count(), reads_before);
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
    if let Ok(path) = std::env::var("PIC_PHOTO_WALL_SCREENSHOT") {
        scroll.vadjustment().set_value(0.0);
        settle();
        let snapshot = gtk::Snapshot::new();
        let paintable = gtk::WidgetPaintable::new(Some(&scroll));
        paintable.snapshot(&snapshot, scroll.width() as f64, scroll.height() as f64);
        let node = snapshot.to_node().unwrap();
        window
            .renderer()
            .unwrap()
            .render_texture(&node, None)
            .save_to_png(path)
            .unwrap();
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
    let gallery_for_resize = gallery.clone();
    stack.add_tick_callback(move |surface, _| {
        gallery_for_resize.update_width(surface.width());
        glib::ControlFlow::Continue
    });

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
    let resize_anchor = gallery.capture_view_anchor().unwrap();
    window.set_default_size(700, 700);
    settle();
    let index = objects
        .iter()
        .position(|photo| photo.id() == resize_anchor.photo_id)
        .unwrap();
    let offset = gallery.sectioned_folder.y_for_index(index as u32).unwrap()
        - wall_scroll.vadjustment().value();
    assert!(
        (offset - resize_anchor.viewport_y_offset).abs() < 3.0,
        "resize lost anchor: {offset} vs {}",
        resize_anchor.viewport_y_offset
    );

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

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_mode_switch_cancels_pending_grid_zoom() {
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
    gallery.request_zoom(187);
    gallery.request_zoom(219);
    assert_eq!(gallery.pending_zoom_width.get(), Some(219));
    gallery.set_layout(PhotoLayout::PhotoWall);
    assert!(
        gallery.pending_zoom_width.get().is_none(),
        "queued Grid zoom survived mode switch"
    );
    assert!(gallery.zoom_reflow_source.borrow().is_none());
    settle();
    assert!(!gallery.sectioned_folder.reflow_active.get());
    assert_eq!(
        gallery.current_zoom_width(),
        219,
        "latest chosen zoom was lost"
    );
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_progressive_folder_batches_publish_matching_sections() {
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
    gallery.group_mode.set(GroupMode::Folder);
    let old = (1_i64..=2)
        .map(|id| {
            glib::Object::builder::<PhotoObject>()
                .property("id", id)
                .property("folder-id", id)
                .build()
        })
        .collect::<Vec<_>>();
    gallery.current_photos.replace(old.clone());
    gallery.store.splice(0, 0, &old);
    gallery.rebuild_group_ranges();
    gallery.set_layout(PhotoLayout::PhotoWall);
    settle();
    let valid = Rc::new(Cell::new(true));
    let valid_for_batches = valid.clone();
    let weak = Rc::downgrade(&gallery);
    gallery.store.connect_items_changed(move |_, _, _, _| {
        let gallery = weak.upgrade().unwrap();
        let photos = gallery.current_photos.borrow();
        let ranges = gallery.group_ranges.borrow();
        if ranges.iter().map(|r| r.end - r.start).sum::<usize>() != photos.len() {
            valid_for_batches.set(false);
        }
        for range in ranges.iter() {
            if photos.get(range.start..range.end).is_none_or(|items| {
                items
                    .iter()
                    .any(|photo| photo.folder_id() != range.folder_id)
            }) {
                valid_for_batches.set(false);
            }
        }
    });
    let photos = (1..=5001)
        .map(|id| crate::db::Photo {
            id: id + 10000,
            path: format!("/virtual/photo-wall-progressive/{id}.jpg"),
            folder_id: Some(if id <= 1500 { 3 } else { 4 }),
            folder_path: None,
            taken_at: None,
            camera: None,
            aperture: None,
            lens: None,
            shutter_speed: None,
            iso: None,
            focal_length: None,
            exposure_bias: None,
            width: Some(6000),
            height: Some(4000),
            size_bytes: None,
            mtime: None,
            added_at: 0,
            rotation: 0,
            edit_recipe: String::new(),
            favorite: false,
            rating: 0,
            trashed: false,
            history_caption: None,
            edited_at: 0,
        })
        .collect::<Vec<_>>();
    gallery.replace(&photos);
    settle();
    assert!(
        valid.get(),
        "a published photo batch had stale folder ranges"
    );
    assert_eq!(gallery.group_ranges.borrow().len(), 2);
    assert_eq!(gallery.store.n_items(), 5001);
}
