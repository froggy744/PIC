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
    // Match application ordering: foundation first, then a theme at +1.
    crate::css::install_foundation(&gtk::gdk::Display::default().unwrap());
    let css = gtk::CssProvider::new();
    css.load_from_data(include_str!("../../themes/standard/theme.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
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
    for (&index, realized) in surface.live_tiles.borrow().iter() {
        let state = surface.wall_state.borrow();
        let item = state.layout.item(index as usize).unwrap();
        let tile = &realized.tile;
        assert!(tile.has_css_class("photo-wall-tile"));
        let frame = tile.first_child().unwrap();
        assert_eq!(frame.width(), tile.width(), "frame inset at {index}");
        assert_eq!(frame.height(), tile.height(), "frame inset at {index}");
        assert_eq!(
            tile.width(),
            ((item.x + item.width).round() - item.x.round()) as i32
        );
        assert_eq!(
            tile.height(),
            ((item.y + item.height).round() - item.y.round()) as i32
        );
        let style = frame.style_context();
        assert_eq!(style.border(), gtk::Border::new());
        assert_eq!(style.padding(), gtk::Border::new());
    }
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
    // Simulate hover with a CSS class so the real mouse cannot clear PRELIGHT
    // during the rendered-pixel checks. Use the production rules unchanged
    // apart from their hover selector, and inspect settled style states.
    let hover_css = gtk::CssProvider::new();
    hover_css.load_from_data(&format!(
        "{}\n.photo-wall-tile .photo-frame.photo-tile {{ transition: none; }}",
        crate::css::PHOTO_WALL.replace(":hover", ".test-hover")
    ));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &hover_css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 6,
    );
    // Render actual GTK hover/selection styling against light and dark photos.
    // The highlight must alter edge pixels, never layout or image interiors.
    let frame = tile.first_child().and_downcast::<gtk::Overlay>().unwrap();
    let picture = frame.child().and_downcast::<gtk::Picture>().unwrap();
    let original_paintable = picture.paintable();
    let allocation = (tile.width(), tile.height());
    let render =
        || {
            tile.queue_draw();
            settle();
            let snapshot = gtk::Snapshot::new();
            surface.root.snapshot_child(&tile, &snapshot);
            let node =
                snapshot.to_node().unwrap_or_else(|| {
                    panic!(
            "empty tile snapshot: mapped={} size={}x{} flags={:?} selected={} scroll={}",
            tile.is_mapped(), tile.width(), tile.height(), tile.state_flags(),
            frame.has_css_class("folder-photo-selected"), scroll.vadjustment().value()
        )
                });
            let texture = window.renderer().unwrap().render_texture(&node, None);
            let mut pixels = vec![0; texture.width() as usize * texture.height() as usize * 4];
            texture.download(&mut pixels, texture.width() as usize * 4);
            let pixel = |x: usize, y: usize| {
                let offset = (y * texture.width() as usize + x) * 4;
                pixels[offset..offset + 4].to_vec()
            };
            (
                pixel(1, texture.height() as usize / 2),
                pixel(2, texture.height() as usize / 2),
                pixel(texture.width() as usize / 2, texture.height() as usize / 2),
            )
        };
    for shade in [0_u8, 255_u8] {
        let mut pixels = vec![shade; 40 * 60 * 4];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        let texture = gtk::gdk::MemoryTexture::new(
            40,
            60,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(pixels),
            40 * 4,
        );
        picture.set_paintable(Some(&texture));
        tile.remove_css_class("test-hover");
        tile.set_manual_selected(false);
        let baseline = render();
        tile.add_css_class("test-hover");
        let hovered = render();
        assert_ne!(hovered.0, baseline.0, "hover invisible on shade {shade}");
        assert_eq!(hovered.2, baseline.2, "hover tinted the photo interior");
        tile.set_manual_selected(true);
        let selected_hovered = render();
        assert_ne!(
            selected_hovered.1, baseline.1,
            "selection too thin on shade {shade}"
        );
        assert_eq!(selected_hovered.2, baseline.2);
        tile.remove_css_class("test-hover");
        assert_eq!(render(), selected_hovered, "hover overrode selection");
        assert_eq!((tile.width(), tile.height()), allocation);
        assert_eq!(frame.style_context().border(), gtk::Border::new());
    }
    gtk::style_context_remove_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &hover_css,
    );
    picture.set_paintable(original_paintable.as_ref());
    tile.set_manual_selected(surface.selection.is_selected(first_index));
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
    let generation = surface.wall_state.borrow().generation;
    let frame_times = Rc::new(RefCell::new(Vec::new()));
    let peak_tiles = Rc::new(Cell::new(surface.live_tiles.borrow().len()));
    let times_for_probe = frame_times.clone();
    let peak_for_probe = peak_tiles.clone();
    let weak = Rc::downgrade(surface);
    let probe = surface.root.add_tick_callback(move |_, clock| {
        let Some(surface) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        times_for_probe.borrow_mut().push(clock.frame_time());
        peak_for_probe.set(peak_for_probe.get().max(surface.live_tiles.borrow().len()));
        glib::ControlFlow::Continue
    });
    for fraction in [0.1, 0.3, 0.7, 0.9, 0.5] {
        scroll.vadjustment().set_value(height * fraction);
        settle();
    }
    probe.remove();
    assert_eq!(
        surface.wall_state.borrow().generation,
        generation,
        "scroll rebuilt geometry"
    );
    assert!(peak_tiles.get() < 300);
    let mut intervals = frame_times
        .borrow()
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) as f64 / 1000.0)
        .collect::<Vec<_>>();
    intervals.sort_by(f64::total_cmp);
    assert!(!intervals.is_empty());
    eprintln!(
        "Synthetic 14,361-photo Wall scroll: peak_tiles={} geometry_rebuilds=0 frame_intervals={} median_ms={:.2} p95_ms={:.2}",
        peak_tiles.get(), intervals.len(), intervals[intervals.len() / 2],
        intervals[(intervals.len() - 1) * 95 / 100]
    );
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
    for tile in surface.live_tiles.borrow().values() {
        assert!(!tile.tile.has_css_class("photo-wall-tile"));
    }
    gtk::style_context_remove_provider_for_display(&gtk::gdk::Display::default().unwrap(), &css);
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
fn photo_wall_view_toggle_keeps_selection_and_grid_scroll_context() {
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
    let view_toggle = info
        .view_toggle
        .clone()
        .upcast::<gtk::Widget>()
        .downcast::<gtk::Button>()
        .expect("view control must toggle directly without a popup");
    assert!(!view_toggle.is::<gtk::ToggleButton>());
    assert!(!view_toggle.has_css_class("photo-action-button"));
    assert_eq!(
        view_toggle.tooltip_text().as_deref(),
        Some("Switch to Photo Wall")
    );
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
    view_toggle.emit_clicked();
    assert!(!view_toggle.state_flags().contains(gtk::StateFlags::CHECKED));
    assert_eq!(
        view_toggle.icon_name().as_deref(),
        Some("view-app-grid-symbolic")
    );
    assert_eq!(
        view_toggle.tooltip_text().as_deref(),
        Some("Switch to Grid")
    );
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

    view_toggle.emit_clicked();
    assert!(!view_toggle.state_flags().contains(gtk::StateFlags::CHECKED));
    assert_eq!(
        view_toggle.icon_name().as_deref(),
        Some("collage-smart-mosaic-symbolic")
    );
    assert_eq!(
        view_toggle.tooltip_text().as_deref(),
        Some("Switch to Photo Wall")
    );
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

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_lightbox_return_waits_for_allocation_and_focuses_current_photo() {
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
    let objects = (0..2000)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property("width", 4000_i64)
                .property("height", 3000_i64)
                .property("folder-id", if i < 1000 { 1_i64 } else { 2_i64 })
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
        .default_width(1000)
        .default_height(600)
        .child(&scroll)
        .build();
    // The lightbox close callback can run before the destination is mapped
    // or has published its new viewport extent. Its current photo can be far
    // from the photo originally opened after navigating inside the viewer.
    let target = 1500;
    gallery.restore_activated_photo(objects[target].id());
    gallery.grab_focus();
    window.present();
    settle();
    assert_eq!(selected_positions(&gallery.selection), vec![target as u32]);
    let surface = &gallery.sectioned_folder;
    let state = surface.wall_state.borrow();
    let item = state.layout.item(target).unwrap();
    let adjustment = scroll.vadjustment();
    assert!(
        (item.y + item.height * 0.5 - adjustment.value() - adjustment.page_size() * 0.5).abs()
            < 2.0,
        "returned photo was not centred: photo_y={} scroll_y={}",
        item.y,
        adjustment.value()
    );
    drop(state);
    let focused = gtk::prelude::RootExt::focus(&window)
        .and_then(|widget| widget.downcast::<SquareTile>().ok())
        .and_then(|tile| tile.photo());
    assert_eq!(focused.map(|photo| photo.id()), Some(objects[target].id()));

    // Repeat a normal, already mapped return in folder-section mode.
    gallery.group_mode.set(GroupMode::Folder);
    gallery.rebuild_group_ranges();
    surface.refresh_model();
    settle();
    let target = 1200;
    gallery.restore_activated_photo(objects[target].id());
    gallery.grab_focus();
    settle();
    assert_eq!(surface.wall_state.borrow().layout.sections.len(), 2);
    let focused = gtk::prelude::RootExt::focus(&window)
        .and_then(|widget| widget.downcast::<SquareTile>().ok())
        .and_then(|tile| tile.photo());
    assert_eq!(focused.map(|photo| photo.id()), Some(objects[target].id()));
    assert_eq!(surface.selection_anchor.get(), Some(target as u32));

    // Returning to an already visible photo must preserve its bottom-left
    // viewport position, including when the row is partially visible.
    for group in [GroupMode::Folder, GroupMode::None] {
        gallery.group_mode.set(group);
        gallery.rebuild_group_ranges();
        surface.refresh_model();
        settle();
        let target = {
            let state = surface.wall_state.borrow();
            let row = &state.layout.rows[state.layout.item(600).unwrap().row];
            state.layout.items[row.item_range.start].photo_index
        };
        let photo_y = surface.y_for_index(target as u32).unwrap();
        let photo_height = surface
            .wall_state
            .borrow()
            .layout
            .item(target)
            .unwrap()
            .height;
        let adjustment = scroll.vadjustment();
        for offset in [
            adjustment.page_size() - photo_height - 12.0,
            adjustment.page_size() - photo_height * 0.5,
        ] {
            adjustment.set_value(photo_y - offset);
            settle();
            let before = adjustment.value();
            gallery.restore_activated_photo(objects[target].id());
            gallery.grab_focus();
            settle();
            assert!(
                (adjustment.value() - before).abs() < 1.0,
                "visible lightbox return shifted the viewport: before={before}, after={}",
                adjustment.value()
            );
            let focused = gtk::prelude::RootExt::focus(&window)
                .and_then(|widget| widget.downcast::<SquareTile>().ok())
                .and_then(|tile| tile.photo());
            assert_eq!(focused.map(|photo| photo.id()), Some(objects[target].id()));
            assert_eq!(surface.selection_anchor.get(), Some(target as u32));
        }
    }

    // Reopening the viewer cancels a pending return even for headerless Wall.
    gallery.group_mode.set(GroupMode::None);
    gallery.rebuild_group_ranges();
    surface.refresh_model();
    settle();
    scroll.vadjustment().set_value(0.0);
    gallery.restore_activated_photo(objects[1800].id());
    gallery.cancel_sectioned_folder_scroll_animation();
    settle();
    assert_eq!(scroll.vadjustment().value(), 0.0);

    // A new user selection must also prevent the old return stealing focus.
    gallery.restore_activated_photo(objects[1700].id());
    gallery.selection.select_item(0, true);
    settle();
    assert_eq!(scroll.vadjustment().value(), 0.0);
    assert_eq!(selected_positions(&gallery.selection), vec![0]);
    window.close();
}

#[test]
fn wall_quality_waits_for_stationary_viewport_and_checks_physical_pixels() {
    let now = Instant::now();
    let mut gate = WallQualityGate::default();
    let signature = (1, 100u64, 700u64, 1);
    assert!(!gate.ready(signature, now));
    assert!(!gate.ready(signature, now + std::time::Duration::from_millis(499)));
    assert!(gate.ready(signature, now + std::time::Duration::from_millis(500)));
    assert!(!gate.ready(
        (2, 100, 700, 1),
        now + std::time::Duration::from_millis(501)
    ));
    assert!(!wall_quality_needed(320, 200, 1));
    assert!(wall_quality_needed(321, 200, 1));
    assert!(wall_quality_needed(200, 200, 2));
    assert!(!wall_quality_needed(0, 0, 2));
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_quality_preserves_fallback_and_rejects_stale_results() {
    gtk::init().unwrap();
    let photo = glib::Object::builder::<PhotoObject>()
        .property("id", 701_i64)
        .property("path", "/offline/photo.jpg")
        .property("width", 1200_i64)
        .property("height", 800_i64)
        .build();
    photo.set_cached_thumbnail_path("/offline/cache/quality-test.jpg");
    let picture = gtk::Picture::new();
    let frame = gtk::Overlay::new();
    frame.set_child(Some(&picture));
    let tile = SquareTile::new(600, 400, &frame);
    tile.add_css_class("photo-wall-tile");
    tile.set_photo_deferred(&photo);
    let texture = |size| -> gtk::gdk::Paintable {
        gtk::gdk::MemoryTexture::new(
            size,
            size,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(vec![200u8; size as usize * size as usize * 4]),
            size as usize * 4,
        )
        .upcast()
    };
    let base = photo_presentation_key(&photo).unwrap();
    let low = texture(320);
    let high = texture(640);
    tile.apply_presentation_paintable(&base, &low);
    let quality =
        crate::thumbnail_display::wall_request(photo_presentation_request(&photo, false).unwrap())
            .key;
    *tile.imp().wall_quality_key.borrow_mut() = Some(quality.clone());
    assert!(tile.apply_wall_quality(&quality, &high));
    assert!(tile.apply_presentation_paintable(&base, &low));
    assert_eq!(picture.paintable().unwrap().intrinsic_width(), 640);
    tile.bind_photo_folder_fast(&photo, 0);
    assert_eq!(picture.paintable().unwrap().intrinsic_width(), 640);
    assert!(!tile.mark_presentation_missing(&base));
    // The normal LRU may have evicted the low preview while quality was shown.
    folder_thumbnail_cache_remove(&base);
    tile.clear_wall_quality();
    assert_eq!(picture.paintable().unwrap().intrinsic_width(), 320);
    assert!(!tile.apply_wall_quality(&quality, &high));
    *tile.imp().wall_quality_key.borrow_mut() = Some(quality.clone());
    assert!(tile.apply_wall_quality(&quality, &high));
    photo.set_rotation(90);
    assert!(!tile.apply_wall_quality(&quality, &high));
    tile.refresh_thumbnail_with_probe(false);
    let rotated_base = photo_presentation_key(&photo).unwrap();
    let rotated_low = texture(213);
    tile.apply_presentation_paintable(&rotated_base, &rotated_low);
    let rotated_quality =
        crate::thumbnail_display::wall_request(photo_presentation_request(&photo, false).unwrap())
            .key;
    tile.clear_wall_quality();
    *tile.imp().wall_quality_key.borrow_mut() = Some(rotated_quality.clone());
    tile.apply_wall_quality(&rotated_quality, &high);
    tile.clear_wall_quality();
    assert_eq!(picture.paintable().unwrap().intrinsic_width(), 213);
    tile.remove_css_class("photo-wall-tile");
    assert!(!tile.apply_wall_quality(&quality, &high));
    FOLDER_THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().clear());
    for i in 0..512 {
        folder_thumbnail_cache_insert(format!("/normal/{i}.jpg"), low.clone());
    }
    for i in 0..33 {
        folder_thumbnail_cache_insert(format!("/quality/{i}-wall640.jpg"), high.clone());
    }
    FOLDER_THUMBNAIL_CACHE.with(|cache| {
        let cache = cache.borrow();
        assert_eq!(
            cache
                .iter()
                .filter(|(key, _)| !crate::thumbnail_display::is_wall_key(key))
                .count(),
            512
        );
        assert_eq!(
            cache
                .iter()
                .filter(|(key, _)| crate::thumbnail_display::is_wall_key(key))
                .count(),
            32
        );
        cache.len()
    });
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_uses_canonical_cache_without_separate_wall_quality_requests() {
    gtk::init().unwrap();
    let gallery = Rc::new(Gallery::new(
        &[],
        300,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    let objects = (0..20)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", 900_i64 + i)
                .property("path", format!("/photos/{i}.jpg"))
                .property("width", 1200_i64)
                .property("height", 800_i64)
                .build()
        })
        .collect::<Vec<_>>();
    gallery.store.splice(0, 0, &objects);
    gallery.current_photos.replace(objects);
    gallery.set_layout(PhotoLayout::PhotoWall);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&scroll);
    let window = gtk::Window::builder()
        .default_width(900)
        .default_height(450)
        .child(&scroll)
        .build();
    window.present();
    settle();

    let surface = &gallery.sectioned_folder;
    surface.poll_wall_quality();
    assert!(surface.wall_state.borrow().quality_attempted.is_empty());
    assert!(surface.live_tiles.borrow().values().all(|live| {
        live.tile.imp().wall_quality_key.borrow().is_none()
    }));

    window.close();
    settle();
}


#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_width_freeze_ignores_intermediate_sidebar_allocations() {
    gtk::init().unwrap();
    let gallery = Rc::new(Gallery::new(
        &[],
        180,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    let objects = (0..80)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property("width", if i % 2 == 0 { 4000_i64 } else { 6000_i64 })
                .property("height", if i % 2 == 0 { 6000_i64 } else { 4000_i64 })
                .build()
        })
        .collect::<Vec<_>>();
    gallery.store.splice(0, 0, &objects);
    gallery.current_photos.replace(objects);
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
    let stable_width = surface.geometry_width.get();
    assert!(stable_width > 100);

    gallery.set_photo_wall_width_frozen(true);
    surface.geometry_for_current_layout(stable_width + 173);
    assert_eq!(
        surface.geometry_width.get(),
        stable_width,
        "Photo Wall followed an intermediate sidebar animation width"
    );

    gallery.set_photo_wall_width_frozen(false);
    surface.geometry_for_current_layout(stable_width + 173);
    assert_eq!(
        surface.geometry_width.get(),
        stable_width + 173,
        "Photo Wall did not resume normal width observation after unfreeze"
    );

    window.close();
    settle();
}
