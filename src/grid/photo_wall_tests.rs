#![allow(deprecated)] // GTK exposes no supported replacement for CSS border and padding introspection.

use super::*;
use std::time::Duration;

fn settle() {
    let context = glib::MainContext::default();
    for _ in 0..30 {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn wait_for_restore(predicate: impl Fn() -> bool) -> bool {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        while context.pending() {
            context.iteration(false);
        }
        std::thread::yield_now();
    }
    predicate()
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn honeycomb_geometry_updates_preserve_normal_gsk_paintable() {
    gtk::init().unwrap();
    let pixels = glib::Bytes::from_owned(vec![255_u8; 32 * 32 * 4]);
    let paintable: gtk::gdk::Paintable = gtk::gdk::MemoryTexture::new(
        32,
        32,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &pixels,
        32 * 4,
    )
    .upcast();
    let photo = glib::Object::builder::<PhotoObject>()
        .property("id", 90420_i64)
        .property("path", "/virtual/honeycomb-gsk-regression.jpg")
        .property("filename", "honeycomb-gsk-regression.jpg")
        .property("original-available", false)
        .build();
    photo.set_cached_thumbnail_path("/virtual/honeycomb-gsk-regression-thumb.jpg");
    let key = photo_presentation_key(&photo).unwrap();
    folder_thumbnail_cache_insert(key.clone(), paintable.clone());
    let picture = gtk::Picture::new();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&picture));
    let tile = SquareTile::new(180, 208, &overlay);
    // Production order: memory-cache bind first, then set_honeycomb().
    tile.bind_photo_folder_fast(&photo, 0);
    assert!(tile.imp().visual_loaded.get());
    let window = gtk::Window::builder().child(&tile).build();
    window.present();
    settle();

    let assert_preserved = || {
        assert_eq!(
            picture.paintable().as_ref(),
            Some(&paintable),
            "normal GSK thumbnail must survive Honeycomb geometry updates",
        );
    };
    tile.set_honeycomb(true);
    assert_preserved();
    tile.set_hex_dimensions(180.0, 208.0, 5.0);
    assert_preserved();
    tile.set_tile_size(181, 209);
    assert_preserved();
    tile.set_honeycomb(false);
    assert_preserved();
    folder_thumbnail_cache_remove(&key);
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn grid_scrub_settle_preserves_loaded_thumbnail_after_ram_eviction() {
    gtk::init().unwrap();
    set_grid_scrub_active(false);
    let photo = glib::Object::builder::<PhotoObject>()
        .property("id", 91_i64)
        .property("path", "/virtual/scrub-settle.jpg")
        .property("filename", "scrub-settle.jpg")
        .property("original-available", false)
        .build();
    photo.set_cached_thumbnail_path("/virtual/scrub-settle-thumb.jpg");
    let key = photo_presentation_key(&photo).unwrap();
    let paintable: gtk::gdk::Paintable = gtk::gdk::MemoryTexture::new(
        40,
        30,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(vec![128_u8; 40 * 30 * 4]),
        40 * 4,
    )
    .upcast();
    folder_thumbnail_cache_insert(key.clone(), paintable.clone());
    let gallery = Gallery::new(
        &[],
        160,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    );
    gallery.current_photos.replace(vec![photo.clone()]);
    gallery.store.splice(0, 0, &[photo]);
    let scroll = gtk::ScrolledWindow::builder().child(&gallery.root).build();
    let window = gtk::Window::builder()
        .default_width(900)
        .default_height(650)
        .child(&scroll)
        .build();
    window.present();
    settle();
    let mut tiles = Vec::new();
    collect_tiles(gallery.root.upcast_ref(), &mut tiles);
    let tile = tiles
        .into_iter()
        .find(|tile| tile.imp().photo.borrow().is_some())
        .unwrap();
    let frame = tile.first_child().and_downcast::<gtk::Overlay>().unwrap();
    let picture = frame.child().and_downcast::<gtk::Picture>().unwrap();
    assert_eq!(picture.paintable().as_ref(), Some(&paintable));
    assert!(tile.imp().visual_loaded.get());

    // A scrollbar jump can evict the shared cache entry while the visible
    // picture still owns its correct, fully loaded paintable.
    folder_thumbnail_cache_remove(&key);
    let blanked = Rc::new(Cell::new(false));
    let blanked_for_signal = blanked.clone();
    picture.connect_paintable_notify(move |picture| {
        if picture.paintable().is_none() {
            blanked_for_signal.set(true);
        }
    });
    assert!(gallery.refresh_visible_grid_tiles() > 0);
    assert!(
        !blanked.get(),
        "settle cleared an already correct thumbnail"
    );
    assert_eq!(picture.paintable().as_ref(), Some(&paintable));

    // An old image retained by a recycled tile must still be removed when
    // scrubbing ends if it belongs to a different photo.
    let next = glib::Object::builder::<PhotoObject>()
        .property("id", 92_i64)
        .property("path", "/virtual/scrub-next.jpg")
        .property("original-available", false)
        .build();
    next.set_cached_thumbnail_path("/virtual/scrub-next-thumb.jpg");
    set_grid_scrub_active(true);
    tile.set_photo_deferred(&next);
    assert!(picture.paintable().is_some());
    set_grid_scrub_active(false);
    assert!(gallery.refresh_visible_grid_tiles() > 0);
    assert!(
        picture.paintable().is_none(),
        "settle retained a different photo's thumbnail"
    );
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn keyboard_open_uses_the_visible_thumbnail_in_all_gallery_layouts() {
    gtk::init().unwrap();
    let photo = glib::Object::builder::<PhotoObject>()
        .property("id", 17_i64)
        .property("path", "/virtual/keyboard-open.jpg")
        .property("width", 800_i64)
        .property("height", 600_i64)
        .build();
    photo.set_cached_thumbnail_path("/virtual/keyboard-open-thumb.jpg");
    let pixels = vec![128_u8; 40 * 30 * 4];
    let texture = gtk::gdk::MemoryTexture::new(
        40,
        30,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(pixels),
        40 * 4,
    );
    folder_thumbnail_cache_insert(photo_presentation_key(&photo).unwrap(), texture.upcast());

    let gallery = Rc::new(Gallery::new(
        &[],
        180,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    gallery.store.append(&photo);
    gallery.current_photos.replace(vec![photo]);
    let grid_scroll = gtk::ScrolledWindow::builder().child(&gallery.root).build();
    let wall_scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&wall_scroll);
    let pages = gtk::Stack::new();
    pages.add_named(&grid_scroll, Some("grid"));
    pages.add_named(&wall_scroll, Some("wall"));
    let window = gtk::Window::builder()
        .default_width(900)
        .default_height(650)
        .child(&pages)
        .build();
    window.present();
    settle();
    assert!(
        gallery.transition_source_for_photo(17).is_some(),
        "Grid Space should have a thumbnail source"
    );
    assert!(gallery.transition_source_for_photo(18).is_none());

    gallery.set_layout(PhotoLayout::PhotoWall);
    pages.set_visible_child_name("wall");
    settle();
    assert!(
        gallery.transition_source_for_photo(17).is_some(),
        "Photo Wall Space should have a thumbnail source"
    );
    gallery.set_layout(PhotoLayout::Masonry);
    settle();
    assert!(
        gallery.transition_source_for_photo(17).is_some(),
        "Masonry Space should have a thumbnail source"
    );
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_reuses_models_and_virtualizes_headerless_and_folder_collections() {
    gtk::init().unwrap();
    // Match application ordering: foundation first, then a theme at +1.
    crate::css::install_foundation(&gtk::gdk::Display::default().unwrap());
    let css = gtk::CssProvider::new();
    css.load_from_string(include_str!("../../themes/standard/theme.css"));
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
    hover_css.load_from_string(&format!(
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
#[ignore = "measures sustained Photo Wall frame intervals; requires a GTK display"]
fn photo_wall_sustained_scroll_frame_latency() {
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
        .map(|index| {
            glib::Object::builder::<PhotoObject>()
                .property("id", index as i64 + 1)
                .property("path", format!("/virtual/smooth-scroll/{index}.jpg"))
                .property("width", 4000_i64)
                .property("height", 6000_i64)
                .build()
        })
        .collect::<Vec<_>>();
    let pixels = vec![128_u8; 40 * 60 * 4];
    let texture = gtk::gdk::MemoryTexture::new(
        40,
        60,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(pixels),
        40 * 4,
    );
    for photo in objects.iter().take(256) {
        photo.set_cached_thumbnail_path(format!("/virtual/smooth-scroll-cache/{}.png", photo.id()));
        folder_thumbnail_cache_insert(
            photo_presentation_key(photo).unwrap(),
            texture.clone().upcast(),
        );
    }
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
    let renderer_name = window
        .renderer()
        .map(|renderer| renderer.type_().name().to_string())
        .unwrap_or_else(|| "unavailable".to_string());

    let surface = &gallery.sectioned_folder;
    let generation = surface.wall_state.borrow().generation;
    let frame_times = Rc::new(RefCell::new(Vec::new()));
    let peak_tiles = Rc::new(Cell::new(surface.live_tiles.borrow().len()));
    let frame_times_for_tick = frame_times.clone();
    let peak_tiles_for_tick = peak_tiles.clone();
    let weak_surface = Rc::downgrade(surface);
    let tick = surface.root.add_tick_callback(move |_, clock| {
        let Some(surface) = weak_surface.upgrade() else {
            return glib::ControlFlow::Break;
        };
        frame_times_for_tick.borrow_mut().push(clock.frame_time());
        peak_tiles_for_tick.set(
            peak_tiles_for_tick
                .get()
                .max(surface.live_tiles.borrow().len()),
        );
        glib::ControlFlow::Continue
    });

    let adjustment = scroll.vadjustment();
    let start_value = adjustment.value();
    let max_value = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    let distance = ((max_value - start_value) * 0.4).max(0.0);
    eprintln!(
        "Photo Wall scroll probe setup mapped={} tiles={} start={start_value:.1} max={max_value:.1} distance={distance:.1}",
        surface.root.is_mapped(),
        surface.live_tiles.borrow().len(),
    );
    let smooth_started = Instant::now();
    let completed = Rc::new(Cell::new(false));
    let completed_for_timeout = completed.clone();
    let root_for_timeout = surface.root.clone();
    let _timeout = glib::timeout_add_local(std::time::Duration::from_millis(8), move || {
        let progress = (smooth_started.elapsed().as_secs_f64() / 1.5).min(1.0);
        adjustment.set_value(start_value + distance * progress);
        root_for_timeout.queue_draw();
        if progress >= 1.0 {
            completed_for_timeout.set(true);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    let context = glib::MainContext::default();
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    while !completed.get() && Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(completed.get(), "smooth scroll did not complete");
    tick.remove();
    assert_eq!(surface.wall_state.borrow().generation, generation);
    assert!(peak_tiles.get() < 300);

    let mut intervals = frame_times
        .borrow()
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) as f64 / 1000.0)
        .collect::<Vec<_>>();
    intervals.sort_by(f64::total_cmp);
    eprintln!(
        "Photo Wall frame probe captured {} samples",
        intervals.len() + 1
    );
    assert!(intervals.len() > 20, "insufficient frame intervals");
    let stalls_over_50ms = intervals
        .iter()
        .filter(|interval| **interval > 50.0)
        .count();
    eprintln!(
        "Synthetic Photo Wall sustained scroll: renderer={renderer_name} photos=14361 duration_ms=1500 frames={} peak_tiles={} geometry_rebuilds=0 median_ms={:.2} p95_ms={:.2} max_ms={:.2} stalls_over_50ms={}",
        intervals.len(), peak_tiles.get(), intervals[intervals.len() / 2],
        intervals[(intervals.len() - 1) * 95 / 100], intervals[intervals.len() - 1], stalls_over_50ms
    );
    window.close();
}

fn honeycomb_bench_visible_tiles(
    gallery: &Gallery,
    layout: PhotoLayout,
    scroll: &gtk::ScrolledWindow,
) -> Vec<SquareTile> {
    let top = scroll.vadjustment().value();
    let bottom = top + scroll.vadjustment().page_size();
    if layout != PhotoLayout::Grid {
        let wall = gallery.sectioned_folder.wall_state.borrow();
        return gallery
            .sectioned_folder
            .live_tiles
            .borrow()
            .values()
            .filter_map(|entry| {
                let index = entry.index.get()?;
                let item = wall.layout.item(index as usize)?;
                (item.y < bottom && item.y + item.height > top).then(|| entry.tile.clone())
            })
            .collect();
    }
    let mut tiles = Vec::new();
    collect_tiles(gallery.root.upcast_ref(), &mut tiles);
    tiles
        .into_iter()
        .filter(|tile| {
            tile.is_mapped()
                // GridView tiles are positioned in the scrolled viewport.
                // Comparing these bounds to vadjustment.value() counts no
                // tiles after scrolling down, despite visible thumbnails.
                && tile.compute_bounds(scroll).is_some_and(|bounds| {
                    let viewport_bottom = f64::from(scroll.height());
                    f64::from(bounds.y()) < viewport_bottom
                        && f64::from(bounds.y() + bounds.height()) > 0.0
                })
        })
        .collect()
}

fn honeycomb_bench_cpu_us() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if result != 0 {
        return 0;
    }
    let usage = unsafe { usage.assume_init() };
    let user = usage.ru_utime.tv_sec as u64 * 1_000_000 + usage.ru_utime.tv_usec as u64;
    let system = usage.ru_stime.tv_sec as u64 * 1_000_000 + usage.ru_stime.tv_usec as u64;
    user + system
}

fn honeycomb_bench_visible_ready(
    gallery: &Gallery,
    layout: PhotoLayout,
    scroll: &gtk::ScrolledWindow,
    cached: bool,
) -> bool {
    let tiles = honeycomb_bench_visible_tiles(gallery, layout, scroll);
    !tiles.is_empty()
        && tiles.iter().all(|tile| {
            if !tile.imp().visual_loaded.get() {
                return false;
            }
            let photo = tile.imp().photo.borrow();
            let Some(photo) = photo.as_ref() else { return false };
            if photo.corrupt() || !photo.thumbnail_available()
                || tile.imp().applied_visual_key.borrow().as_deref()
                    != photo_presentation_key(photo).as_deref()
                || tile.transition_paintable().is_none()
            {
                return false;
            }
            // Cache misses use the normal GSK fallback by design. Validate the
            // actual photo, not whether every visible tile has been premasked.
            let _ = cached;
            true
        })
}

// Diagnostic-only: never change the readiness predicate to make an invalid
// measurement pass. Capture the state *after* a failed settle deadline.
fn honeycomb_bench_dump_unready(
    gallery: &Gallery,
    layout: PhotoLayout,
    scroll: &gtk::ScrolledWindow,
    cached: bool,
    phase: &str,
) {
    let tiles = honeycomb_bench_visible_tiles(gallery, layout, scroll);
    let mut unready = 0_usize;
    let mut no_photo = 0_usize;
    let mut unloaded = 0_usize;
    let mut unavailable = 0_usize;
    let mut corrupt = 0_usize;
    let mut key_mismatch = 0_usize;
    let mut no_paintable = 0_usize;
    let mut uncached = 0_usize;
    for tile in &tiles {
        let photo = tile.imp().photo.borrow();
        let Some(photo) = photo.as_ref() else {
            unready += 1;
            no_photo += 1;
            continue;
        };
        let loaded = tile.imp().visual_loaded.get();
        let available = photo.thumbnail_available();
        let bad_photo = photo.corrupt();
        let key_ok = tile.imp().applied_visual_key.borrow().as_deref()
            == photo_presentation_key(photo).as_deref();
        let paintable = tile.transition_paintable().is_some();
        let mask_ok = !cached || tile.imp().honeycomb_mask_cached.get();
        if loaded && available && !bad_photo && key_ok && paintable && mask_ok {
            continue;
        }
        unready += 1;
        unloaded += usize::from(!loaded);
        unavailable += usize::from(!available);
        corrupt += usize::from(bad_photo);
        key_mismatch += usize::from(!key_ok);
        no_paintable += usize::from(!paintable);
        uncached += usize::from(!mask_ok);
        if unready <= 12 {
            eprintln!(
                "BENCH_UNREADY_TILE phase={phase} photo_id={} path={:?} loaded={loaded} thumbnail_available={available} corrupt={bad_photo} key_ok={key_ok} paintable={paintable} mask_ok={mask_ok} thumb_path={:?}",
                photo.id(), photo.path(), photo.cached_thumbnail_path(),
            );
        }
    }
    eprintln!(
        "BENCH_UNREADY_SUMMARY phase={phase} visible={} unready={unready} no_photo={no_photo} unloaded={unloaded} unavailable={unavailable} corrupt={corrupt} key_mismatch={key_mismatch} no_paintable={no_paintable} uncached={uncached}",
        tiles.len(),
    );
}

fn honeycomb_bench_capture_tile(
    gallery: &Gallery,
    window: &gtk::Window,
    scroll: &gtk::ScrolledWindow,
    layout: PhotoLayout,
    mode: &str,
) {
    let Ok(directory) = std::env::var("PIC_BENCH_CAPTURE_DIR") else {
        return;
    };
    let Some(tile) = honeycomb_bench_visible_tiles(gallery, layout, scroll)
        .into_iter()
        .find(|tile| tile.imp().visual_loaded.get())
    else {
        return;
    };
    let snapshot = gtk::Snapshot::new();
    tile.snapshot_honeycomb_for_bench(&snapshot);
    let Some(node) = snapshot.to_node() else {
        eprintln!(
            "BENCH_CAPTURE mode={mode} unavailable empty_tile_snapshot size={}x{}",
            tile.width(),
            tile.height()
        );
        return;
    };
    let rendered = window.renderer().unwrap().render_texture(
        &node,
        Some(&gtk::graphene::Rect::new(
            0.0,
            0.0,
            tile.width() as f32,
            tile.height() as f32,
        )),
    );
    let directory = std::path::Path::new(&directory);
    std::fs::create_dir_all(directory).unwrap();
    let path = directory.join(format!("{mode}.png"));
    rendered.save_to_png(&path).unwrap();
    eprintln!(
        "BENCH_CAPTURE mode={mode} photo_id={} path={}",
        tile.photo().unwrap().id(),
        path.display()
    );
}

fn honeycomb_bench_scroll_leg(
    gallery: &Rc<Gallery>,
    window: &gtk::Window,
    scroll: &gtk::ScrolledWindow,
    mode: &str,
    layout: PhotoLayout,
    cached: bool,
    phase: &str,
    distance: f64,
) {
    let adjustment = scroll.vadjustment();
    let from = adjustment.value();
    let max = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    let to = (from + distance).clamp(adjustment.lower(), max);
    let samples = Rc::new(RefCell::new(Vec::<i64>::new()));
    let previous = Rc::new(Cell::new(None::<i64>));
    let samples_after = samples.clone();
    let previous_after = previous.clone();
    let clock = window.frame_clock().unwrap();
    let handler = clock.connect_after_paint(move |clock| {
        let frame_time = clock.frame_time();
        if let Some(old) = previous_after.replace(Some(frame_time)) {
            let interval = frame_time.saturating_sub(old);
            if interval > 0 {
                samples_after.borrow_mut().push(interval);
            }
        }
    });

    let wall_start = Instant::now();
    let cpu_start = honeycomb_bench_cpu_us();
    let reached = Rc::new(Cell::new(false));
    let reached_timeout = reached.clone();
    let adjust_timeout = adjustment.clone();
    let root_timeout = gallery.visible_root();
    let _source = glib::timeout_add_local(Duration::from_millis(8), move || {
        let progress = (wall_start.elapsed().as_secs_f64() / 1.5).min(1.0);
        adjust_timeout.set_value(from + (to - from) * progress);
        root_timeout.queue_draw();
        if progress >= 1.0 {
            reached_timeout.set(true);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(4);
    while !reached.get() && Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(reached.get(), "benchmark scroll leg timed out");
    let scroll_elapsed_us = wall_start.elapsed().as_micros() as u64;
    // Stop sampling at the end of the controlled movement. Texture settling is
    // measured separately and must not contaminate the scrolling frame trace.
    clock.disconnect(handler);
    let settle_start = Instant::now();
    let settle_deadline = settle_start + Duration::from_secs(12);
    while Instant::now() < settle_deadline
        && !honeycomb_bench_visible_ready(gallery, layout, scroll, cached)
    {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let visible_settle_us = settle_start.elapsed().as_micros() as u64;
    let visible_ready = honeycomb_bench_visible_ready(gallery, layout, scroll, cached);
    if !visible_ready {
        honeycomb_bench_dump_unready(gallery, layout, scroll, cached, phase);
    }
    let cpu_us = honeycomb_bench_cpu_us().saturating_sub(cpu_start);
    let rss_kib = {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
            unsafe { usage.assume_init().ru_maxrss as u64 }
        } else {
            0
        }
    };
    let mask = honeycomb_raster::metrics_snapshot();
    let mut intervals = samples
        .borrow()
        .iter()
        .map(|interval| *interval as f64 / 1000.0)
        .collect::<Vec<_>>();
    intervals.sort_by(f64::total_cmp);
    let percentile = |p: f64| -> f64 {
        if intervals.is_empty() {
            0.0
        } else {
            intervals[((intervals.len() - 1) as f64 * p).ceil() as usize]
        }
    };
    let over_16 = intervals.iter().filter(|&&ms| ms > 16.7).count();
    let over_33 = intervals.iter().filter(|&&ms| ms > 33.3).count();
    eprintln!(
        "BENCH_JSON {{\"mode\":\"{mode}\",\"phase\":\"{phase}\",\"kind\":\"{layout:?}\",\"frames\":{},\"median_ms\":{:.3},\"p95_ms\":{:.3},\"p99_ms\":{:.3},\"max_ms\":{:.3},\"over_16_7\":{over_16},\"over_33_3\":{over_33},\"scroll_ms\":{:.1},\"visible_settle_ms\":{:.1},\"visible_ready\":{visible_ready},\"cpu_pct\":{:.1},\"cpu_us\":{cpu_us},\"rss_hwm_kib\":{rss_kib},\"cache_hits\":{},\"cache_misses\":{},\"cache_fallbacks\":{},\"cache_evictions\":{},\"cache_bytes\":{},\"pending_bytes\":{},\"pending_peak_bytes\":{},\"worker_us\":{},\"queue_avg_us\":{},\"queue_max_us\":{},\"main_texture_us\":{},\"estimated_upload_bytes\":{},\"intervals_ms\":[{}]}}",
        intervals.len() + 1,
        percentile(0.50),
        percentile(0.95),
        percentile(0.99),
        intervals.last().copied().unwrap_or_default(),
        scroll_elapsed_us as f64 / 1000.0,
        visible_settle_us as f64 / 1000.0,
        cpu_us as f64 * 100.0 / scroll_elapsed_us.saturating_add(visible_settle_us).max(1) as f64,
        mask.hits,
        mask.misses,
        mask.fallbacks,
        mask.evictions,
        mask.cache_bytes,
        mask.pending_bytes,
        mask.pending_peak_bytes,
        mask.worker_us,
        mask.queue_avg_us,
        mask.queue_max_us,
        mask.main_texture_us,
        mask.estimated_upload_bytes,
        intervals
            .iter()
            .map(|ms| format!("{ms:.3}"))
            .collect::<Vec<_>>()
            .join(","),
    );
    assert!(intervals.len() > 20, "too few native frame samples");
    assert!(visible_ready, "invalid benchmark: visible thumbnails did not become ready in phase {phase}; inspect BENCH_UNREADY_* above");
}

#[test]
#[ignore = "requires native Wayland/Vulkan and PIC_BENCH_LIBRARY_DB pointing to an actual PIC library"]
fn actual_library_honeycomb_cached_scroll_benchmark() {
    libadwaita::init().unwrap();
    let database_path = std::env::var("PIC_BENCH_LIBRARY_DB").unwrap();
    let connection = rusqlite::Connection::open_with_flags(
        &database_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let photos = crate::db::photos(&connection, None, false, None)
        .unwrap()
        .into_iter()
        .filter(|photo| {
            !photo.trashed
                && photo.path.starts_with('/')
                && std::path::Path::new(&photo.path).is_file()
        })
        .map(|photo| PhotoObject::from_photo(&photo))
        .collect::<Vec<_>>();
    assert!(
        photos.len() >= 100,
        "not a representative real PIC library: {} photos",
        photos.len()
    );
    let mode = std::env::var("PIC_BENCH_LAYOUT").unwrap();
    let (layout, phase) = match mode.as_str() {
        "gsk" => (PhotoLayout::Honeycomb, "gsk"),
        "cached" => (PhotoLayout::Honeycomb, "cached"),
        "grid" => (PhotoLayout::Grid, "grid"),
        other => panic!("unknown PIC_BENCH_LAYOUT={other}"),
    };
    assert_eq!(
        honeycomb_raster::cached_render_enabled(),
        mode == "cached",
        "renderer env flag must match the benchmark mode"
    );
    let gallery = Rc::new(Gallery::new(
        &[],
        100,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    gallery.store.splice(0, 0, &photos);
    gallery.current_photos.replace(photos);
    gallery.set_layout(layout);
    let scroll = if layout == PhotoLayout::Grid {
        gtk::ScrolledWindow::builder().child(&gallery.root).build()
    } else {
        gtk::ScrolledWindow::builder()
            .child(&gallery.folder_sectioned_root)
            .build()
    };
    if layout == PhotoLayout::Grid {
        let gallery_for_scroll = gallery.clone();
        scroll.vadjustment().connect_value_changed(move |_| {
            gallery_for_scroll.refresh_visible_grid_tiles();
        });
    } else {
        gallery.attach_sectioned_folder_scroll(&scroll);
    }
    let window = gtk::Window::builder()
        .title("PIC real-library Honeycomb cache benchmark")
        .default_width(1280)
        .default_height(800)
        .child(&scroll)
        .build();
    let gallery_for_tick = gallery.clone();
    gallery.visible_root().add_tick_callback(move |_, _| {
        gallery_for_tick.drain_thumbnail_display_completions();
        glib::ControlFlow::Continue
    });
    window.present();
    settle();
    if std::env::var_os("PIC_BENCH_ONE_PHOTO").is_none() {
        gallery.refresh_thumbnails();
    }
    let renderer_name = window.renderer().unwrap().type_().name().to_string();
    assert_eq!(renderer_name, "GskVulkanRenderer");
    if layout == PhotoLayout::Honeycomb {
        assert_eq!(scroll.hscrollbar_policy(), gtk::PolicyType::Never);
    }
    if std::env::var_os("PIC_BENCH_ONE_PHOTO").is_some() {
        let tile = honeycomb_bench_visible_tiles(&gallery, layout, &scroll)
            .into_iter().find(|tile| tile.is_mapped() && tile.photo().is_some())
            .expect("no mapped photo tile");
        let photo = tile.photo().unwrap();
        eprintln!("SMOKE photo_id={} thumbnail={:?}", photo.id(), photo.cached_thumbnail_path());
        tile.refresh_thumbnail();
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && !tile.imp().honeycomb_mask_cached.get() {
            while context.pending() { context.iteration(false); }
            gallery.drain_thumbnail_display_completions();
            std::thread::sleep(Duration::from_millis(2));
        }
        let metrics = honeycomb_raster::metrics_snapshot();
        eprintln!("SMOKE loaded={} available={} cached={} worker_us={} misses={} hits={} cache_bytes={} paintable={}", tile.imp().visual_loaded.get(), photo.thumbnail_available(), tile.imp().honeycomb_mask_cached.get(), metrics.worker_us, metrics.misses, metrics.hits, metrics.cache_bytes, tile.transition_paintable().is_some());
        assert!(tile.imp().honeycomb_mask_cached.get() && metrics.worker_us > 0, "photo did not reach cached snapshot path");
        settle();
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(&tile)).snapshot(&snapshot, tile.width() as f64, tile.height() as f64);
        let node = snapshot.to_node().expect("cached tile produced no snapshot");
        let texture = window.renderer().unwrap().render_texture(&node, None);
        let mut pixels = vec![0; texture.width() as usize * texture.height() as usize * 4];
        texture.download(&mut pixels, texture.width() as usize * 4);
        let center = ((texture.height() as usize / 2) * texture.width() as usize + texture.width() as usize / 2) * 4;
        assert!(pixels[center + 3] > 240, "cached tile center is transparent");
        eprintln!("SMOKE cached_snapshot={}x{} center_alpha={}", texture.width(), texture.height(), pixels[center + 3]);
        window.close();
        return;
    }
    eprintln!(
        "BENCH_SETUP layout={phase} renderer={renderer_name} photos={} viewport={}x{} zoom_width={} hscroll={:?}",
        gallery.current_photos.borrow().len(),
        scroll.width(),
        scroll.height(),
        gallery.tile_width.get(),
        scroll.hscrollbar_policy(),
    );

    // The prewarm run creates any missing disk thumbnails before measured
    // interleaved runs. Cold/warm below therefore isolates the masked-texture
    // cache while keeping thumbnail decode state equal between layouts.
    let prewarm = std::env::var("PIC_BENCH_PREWARM").as_deref() == Ok("1");
    let adjustment = scroll.vadjustment();
    let distance = 2400.0_f64.min((adjustment.upper() - adjustment.page_size()).max(0.0));
    let context = glib::MainContext::default();
    let initial_deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < initial_deadline
        && !honeycomb_bench_visible_ready(&gallery, layout, &scroll, false)
    {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    if !honeycomb_bench_visible_ready(&gallery, layout, &scroll, false) {
        honeycomb_bench_dump_unready(&gallery, layout, &scroll, false, "initial");
        panic!("invalid benchmark: initial visible thumbnails did not become ready");
    }
    if prewarm {
        honeycomb_bench_scroll_leg(
            &gallery,
            &window,
            &scroll,
            mode.as_str(),
            layout,
            false,
            "prewarm",
            distance,
        );
    } else {
        honeycomb_bench_scroll_leg(
            &gallery,
            &window,
            &scroll,
            mode.as_str(),
            layout,
            mode == "cached",
            "cold",
            distance,
        );
        honeycomb_bench_scroll_leg(
            &gallery,
            &window,
            &scroll,
            mode.as_str(),
            layout,
            false,
            "reverse",
            -distance,
        );
        honeycomb_bench_scroll_leg(
            &gallery,
            &window,
            &scroll,
            mode.as_str(),
            layout,
            mode == "cached",
            "warm",
            distance,
        );
        if layout == PhotoLayout::Honeycomb {
            for tile in honeycomb_bench_visible_tiles(&gallery, layout, &scroll) {
                assert!(
                    (tile.imp().hex_separator.get() - 3.0).abs() < 0.001,
                    "Honeycomb border width changed during scroll"
                );
            }
        }
        honeycomb_bench_capture_tile(&gallery, &window, &scroll, layout, phase);
    }
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
    gallery.set_folder_view_changed_handler(move |_, _| {
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
        Some("view-masonry-symbolic")
    );
    assert_eq!(
        view_toggle.tooltip_text().as_deref(),
        Some("Switch to Masonry")
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
    // Width-only scaling preserves the logical viewport top, rather than
    // keeping a center photo at a fixed pixel offset while tiles scale.
    let (resize_row, resize_fraction, resize_membership) = {
        let state = gallery.sectioned_folder.wall_state.borrow();
        let top = wall_scroll.vadjustment().value();
        let row_index = state.layout.visible_rows(top, top + 1.0).start;
        let row = &state.layout.rows[row_index];
        (
            row_index,
            (top - row.y) / row.block_height,
            row.item_range.clone(),
        )
    };
    window.set_default_size(700, 700);
    settle();
    {
        let state = gallery.sectioned_folder.wall_state.borrow();
        let row = &state.layout.rows[resize_row];
        assert_eq!(row.item_range, resize_membership);
        let expected = row.y + row.block_height * resize_fraction;
        assert!(
            (wall_scroll.vadjustment().value() - expected).abs() < 3.0,
            "resize lost logical viewport position"
        );
    }

    view_toggle.emit_clicked();
    view_toggle.emit_clicked();
    assert_eq!(gallery.layout(), PhotoLayout::Honeycomb);
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
    check_lightbox_return(PhotoLayout::PhotoWall);
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn masonry_lightbox_return_preserves_position_and_focus() {
    check_lightbox_return(PhotoLayout::Masonry);
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn honeycomb_lightbox_return_preserves_position_and_focus() {
    check_lightbox_return(PhotoLayout::Honeycomb);
}

fn check_lightbox_return(layout: PhotoLayout) {
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
                .property(
                    "height",
                    if layout == PhotoLayout::Masonry {
                        [2000_i64, 4000, 6000][i % 3]
                    } else {
                        3000_i64
                    },
                )
                .property("folder-id", if i < 1000 { 1_i64 } else { 2_i64 })
                .build()
        })
        .collect::<Vec<_>>();
    gallery.store.splice(0, 0, &objects);
    gallery.current_photos.replace(objects.clone());
    gallery.set_layout(layout);
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
    window.present();
    let returned_photo_id = objects[target].id();
    assert!(
        wait_for_restore(|| {
            let state = gallery.sectioned_folder.wall_state.borrow();
            let Some(item) = state.layout.item(target) else {
                return false;
            };
            let adjustment = scroll.vadjustment();
            let centered =
                (item.y + item.height * 0.5 - adjustment.value() - adjustment.page_size() * 0.5)
                    .abs()
                    < 2.0;
            let focused = gtk::prelude::RootExt::focus(&window)
                .and_then(|widget| widget.downcast::<SquareTile>().ok())
                .and_then(|tile| tile.photo())
                .is_some_and(|photo| photo.id() == returned_photo_id);
            centered && focused
        }),
        "returned photo {} was not centered and focused after GTK published the scroll extent",
        returned_photo_id,
    );
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

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_width_change_scales_without_refit_or_scroll_jump() {
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
    let objects = (0..200)
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
        .default_width(1400)
        .default_height(600)
        .child(&scroll)
        .build();
    window.present();
    settle();

    let surface = &gallery.sectioned_folder;
    let adjustment = scroll.vadjustment();
    adjustment.set_value((adjustment.upper() - adjustment.page_size()) * 0.4);
    settle();

    let old_width = surface.geometry_width.get();
    let old_value = adjustment.value();
    let old_total = surface.total_height.get();
    let old_rows = surface
        .wall_state
        .borrow()
        .layout
        .rows
        .iter()
        .map(|row| row.item_range.clone())
        .collect::<Vec<_>>();

    let new_width = old_width - 5;
    // Match a real allocation: geometry updates read the allocated width,
    // including synchronous refreshes caused by the vertical adjustment.
    scroll.allocate(new_width, scroll.height(), -1, None);
    surface.geometry_for_current_layout(new_width);

    let factor = f64::from(new_width) / f64::from(old_width);
    let new_rows = surface
        .wall_state
        .borrow()
        .layout
        .rows
        .iter()
        .map(|row| row.item_range.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        old_rows, new_rows,
        "row membership changed on a width-only resize"
    );
    assert_eq!(surface.geometry_width.get(), new_width);
    assert!((surface.total_height.get() - old_total * factor).abs() < 1.0);
    assert!(
        (adjustment.value() - old_value * factor).abs() < 2.0,
        "scroll position did not scale with the wall: {} -> {}",
        old_value,
        adjustment.value()
    );

    window.close();
    settle();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn pure_scaling_keeps_deep_section_row_visible_in_resize_frames() {
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
    gallery.group_mode.set(GroupMode::Folder);
    let photos = (0..1000)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property("folder-id", (i / 50) as i64 + 1)
                .property("width", 4000_i64)
                .property("height", 4000_i64)
                .build()
        })
        .collect::<Vec<_>>();
    gallery.current_photos.replace(photos.clone());
    gallery.store.splice(0, 0, &photos);
    gallery.rebuild_group_ranges();
    gallery.set_layout(PhotoLayout::PhotoWall);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&scroll);
    let window = gtk::Window::builder()
        .default_width(1400)
        .default_height(600)
        .child(&scroll)
        .build();
    window.present();
    settle();
    let surface = &gallery.sectioned_folder;
    let adjustment = scroll.vadjustment();
    let (index, y) = {
        let state = surface.wall_state.borrow();
        let index = state
            .layout
            .rows
            .iter()
            .position(|row| state.layout.items[row.item_range.start].section == 12)
            .unwrap()
            + 2;
        let row = &state.layout.rows[index];
        (index, row.y + row.block_height * 0.5)
    };
    adjustment.set_value(y);
    settle();
    let (fraction, memberships) = {
        let state = surface.wall_state.borrow();
        let row = &state.layout.rows[index];
        (
            (adjustment.value() - row.y) / row.block_height,
            state
                .layout
                .rows
                .iter()
                .map(|row| row.item_range.clone())
                .collect::<Vec<_>>(),
        )
    };
    let reads = crate::source::original_read_count();
    let samples = Rc::new(RefCell::new(Vec::<(i32, i32, f64, bool, f64)>::new()));
    let samples_for_frame = samples.clone();
    let weak_surface = Rc::downgrade(surface);
    let weak_scroll = scroll.downgrade();
    let clock = window.frame_clock().unwrap();
    let handler = clock.connect_after_paint(move |_| {
        let (Some(surface), Some(scroll)) = (weak_surface.upgrade(), weak_scroll.upgrade()) else { return; };
        let state = surface.wall_state.borrow();
        let row = &state.layout.rows[index];
        let wanted = row.y + row.block_height * fraction;
        let bounds = surface.root.compute_bounds(&scroll).unwrap();
        // Checking the adjustment alone misses a viewport that paints its
        // child using an older translation. Measure the visible content too.
        let error = (scroll.vadjustment().value() - wanted).abs()
            .max((f64::from(bounds.y()) + wanted).abs());
        if error > 2.0 {
            eprintln!("PAINT_TRANSLATION width={} geometry={} wanted={wanted:.3} adjustment={:.3} painted_root_y={:.3} page={:.3}",
                scroll.width(), surface.geometry_width.get(), scroll.vadjustment().value(),
                bounds.y(), scroll.vadjustment().page_size());
        }
        let unchanged = state.layout.rows.iter().map(|row| row.item_range.clone()).collect::<Vec<_>>() == memberships;
        let mut tile_error = 0.0_f64;
        for (index, tile) in surface.live_tiles.borrow().iter() {
            let item = state.layout.item(*index as usize).unwrap();
            let bounds = tile.tile.compute_bounds(&surface.root).unwrap();
            let width = (item.x + item.width).round() - item.x.round();
            let height = (item.y + item.height).round() - item.y.round();
            for delta in [
                (f64::from(bounds.x()) - item.x.round()).abs(),
                (f64::from(bounds.y()) - item.y.round()).abs(),
                (f64::from(bounds.width()) - width).abs(),
                (f64::from(bounds.height()) - height).abs(),
            ] {
                tile_error = tile_error.max(delta);
            }
        }
        samples_for_frame.borrow_mut().push((scroll.width(), surface.geometry_width.get(), error, unchanged, tile_error));
    });
    let context = glib::MainContext::default();
    // Native dragging also delivers long runs of tiny allocation changes.
    // Exercise those, including a return to the starting width.
    let widths = [1300, 1000, 1200, 1400]
        .into_iter()
        .chain((1300..1400).rev())
        .chain(1301..=1400);
    for width in widths {
        let first_sample = samples.borrow().len();
        window.set_default_size(width, (f64::from(width) * 0.55).round() as i32);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < until
            && !samples.borrow()[first_sample..]
                .iter()
                .any(|sample| sample.0 == width)
        {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(scroll.width(), width);
    }
    clock.disconnect(handler);
    let painted = samples.borrow();
    assert!(painted.windows(2).any(|pair| pair[1].0 < pair[0].0));
    assert!(painted.windows(2).any(|pair| pair[1].0 > pair[0].0));
    for &(allocated, geometry, error, unchanged, tile_error) in painted.iter() {
        assert!(unchanged, "scaling changed row membership");
        assert_eq!(
            allocated, geometry,
            "painted the new width before scaling old geometry"
        );
        assert!(
            error < 2.0,
            "deep section viewport moved before paint: {error}"
        );
        assert!(
            tile_error < 2.0,
            "painted tiles lagged geometry at width {allocated}: {tile_error}"
        );
    }
    assert_eq!(crate::source::original_read_count(), reads);
    window.close();
    settle();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_integer_allocations_paint_every_pixel_including_final_row() {
    gtk::init().unwrap();
    crate::css::install_foundation(&gtk::gdk::Display::default().unwrap());
    let gallery = Rc::new(Gallery::new(
        &[],
        100,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    let photos = (0..7)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property("width", 40_i64)
                .property("height", 40_i64)
                .build()
        })
        .collect::<Vec<_>>();
    gallery.current_photos.replace(photos.clone());
    gallery.store.splice(0, 0, &photos);
    gallery.set_layout(PhotoLayout::PhotoWall);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&scroll);
    let window = gtk::Window::builder()
        .default_width(799)
        .default_height(400)
        .child(&scroll)
        .build();
    window.present();
    settle();
    let surface = &gallery.sectioned_folder;
    let texture = gtk::gdk::MemoryTexture::new(
        40,
        40,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(vec![255u8; 40 * 40 * 4]),
        40 * 4,
    );
    for width in [799, 801, 997, 799] {
        scroll.allocate(width, 400, -1, None);
        surface.geometry_for_current_layout(width);
        surface.refresh();
        // Allocate synchronously; a window frame would restore the actual
        // window width, defeating this deliberately odd-width fixture.
        for live in surface.live_tiles.borrow().values() {
            let frame = live
                .tile
                .first_child()
                .and_downcast::<gtk::Overlay>()
                .unwrap();
            let picture = frame.child().and_downcast::<gtk::Picture>().unwrap();
            picture.set_paintable(Some(&texture));
            assert_eq!(picture.content_fit(), gtk::ContentFit::Cover);
        }
        surface
            .root
            .allocate(width, surface.total_height.get() as i32, -1, None);
        let state = surface.wall_state.borrow();
        assert_eq!(state.layout.rows.len(), 1);
        assert_eq!(
            state
                .layout
                .items
                .iter()
                .map(|tile| tile.width)
                .sum::<f64>(),
            f64::from(width)
        );
        let height = state.layout.rows[0].image_height as i32;
        let snapshot = gtk::Snapshot::new();
        for live in surface.live_tiles.borrow().values() {
            let item = state
                .layout
                .item(live.index.get().unwrap() as usize)
                .unwrap();
            let bounds = live.tile.compute_bounds(&surface.root).unwrap();
            assert_eq!(
                (bounds.x(), bounds.y(), bounds.width(), bounds.height()),
                (
                    item.x as f32,
                    item.y as f32,
                    item.width as f32,
                    item.height as f32
                )
            );
            surface.root.snapshot_child(&live.tile, &snapshot);
        }
        let node = snapshot.to_node().unwrap();
        let rendered = window.renderer().unwrap().render_texture(
            &node,
            Some(&gtk::graphene::Rect::new(
                0.0,
                0.0,
                width as f32,
                height as f32,
            )),
        );
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        rendered.download(&mut pixels, width as usize * 4);
        assert!(
            pixels.chunks_exact(4).all(|pixel| pixel[3] == 255),
            "background exposed between tiles at width {width}"
        );
    }
    window.close();
    settle();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn wall_and_masonry_zoom_scale_without_gaps_or_flying_tiles() {
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
    let photos = (0..90_i64)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i + 1)
                .property("width", if i % 3 == 0 { 4000_i64 } else { 6000_i64 })
                .property("height", if i % 3 == 0 { 6000_i64 } else { 4000_i64 })
                .build()
        })
        .collect::<Vec<_>>();
    gallery.current_photos.replace(photos.clone());
    gallery.store.splice(0, 0, &photos);

    let scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&scroll);
    let window = gtk::Window::builder()
        .default_width(1000)
        .default_height(670)
        .child(&scroll)
        .build();
    window.present();
    settle();
    gallery.set_zoom_animations_enabled(true);
    assert!(scroll.is_mapped());

    for layout in [PhotoLayout::PhotoWall, PhotoLayout::Masonry] {
        gallery.set_layout(layout);
        settle();
        let surface = &gallery.sectioned_folder;

        gallery.wheel_zoom_in();
        assert!(
            surface.wall_zoom_active.get(),
            "{layout:?} should animate after committing layout"
        );
        assert!(
            (surface.wall_zoom_scale.get() - 1.03).abs() < 0.001,
            "zoom-in must settle from 103% without exposing tile background"
        );
        for live in surface.live_tiles.borrow().values() {
            assert!(
                (live.tile.opacity() - 1.0).abs() < 0.001,
                "zoom must remain fully opaque"
            );
        }
        assert!(
            !surface.reflow_active.get(),
            "wall zoom must not animate photo coordinates"
        );
        for (&index, live) in surface.live_tiles.borrow().iter() {
            let wall = surface.wall_state.borrow();
            let item = wall.layout.item(index as usize).unwrap();
            assert_eq!(live.tile.presentation_offset(), (0.0, 0.0));
            assert_eq!(live.tile.width(), item.width as i32);
            assert_eq!(live.tile.height(), item.height as i32);
        }
        settle();
        assert!(!surface.wall_zoom_active.get());
        for live in surface.live_tiles.borrow().values() {
            assert!((live.tile.presentation_scale() - 1.0).abs() < 0.001);
            assert!((live.tile.opacity() - 1.0).abs() < 0.001);
        }

        gallery.wheel_zoom_out();
        assert!(surface.wall_zoom_active.get());
        assert!(
            (surface.wall_zoom_scale.get() - 1.03).abs() < 0.001,
            "zoom-out must also settle from 103% to keep cells covered"
        );
        for live in surface.live_tiles.borrow().values() {
            assert!(
                (live.tile.opacity() - 1.0).abs() < 0.001,
                "zoom-out must remain fully opaque"
            );
        }
        // Cancel mid-animation by switching off animations, then restore setting.
        gallery.set_zoom_animations_enabled(false);
        assert!(!surface.wall_zoom_active.get());
        for live in surface.live_tiles.borrow().values() {
            assert!((live.tile.presentation_scale() - 1.0).abs() < 0.001);
            assert!((live.tile.opacity() - 1.0).abs() < 0.001);
        }
        gallery.set_zoom_animations_enabled(true);
    }
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_ctrl_wheel_zoom_allocations_match_every_painted_frame() {
    gtk::init().unwrap();
    crate::css::install_foundation(&gtk::gdk::Display::default().unwrap());
    let gallery = Rc::new(Gallery::new(
        &[],
        100,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    let photos = (0..2000)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property("width", [4000_i64, 6000, 4000][i % 3])
                .property("height", [6000_i64, 4000, 4000][i % 3])
                .build()
        })
        .collect::<Vec<_>>();
    gallery.current_photos.replace(photos.clone());
    gallery.store.splice(0, 0, &photos);
    gallery.set_layout(PhotoLayout::PhotoWall);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&scroll);
    let window = gtk::Window::builder()
        .default_width(997)
        .default_height(600)
        .child(&scroll)
        .build();
    window.present();
    settle();
    scroll.vadjustment().set_value(7000.0);
    settle();
    let anchor = gallery.capture_view_anchor().unwrap();
    let expected_anchor = Rc::new(Cell::new((
        gallery
            .current_photos
            .borrow()
            .iter()
            .position(|photo| photo.id() == anchor.photo_id)
            .unwrap(),
        anchor.viewport_y_offset,
    )));
    let expected_for_paint = expected_anchor.clone();
    let errors = Rc::new(RefCell::new(Vec::new()));
    let samples = errors.clone();
    let surface = gallery.sectioned_folder.clone();
    let scroll_for_paint = scroll.clone();
    let handler = window.frame_clock().unwrap().connect_after_paint(move |_| {
        let state = surface.wall_state.borrow();
        let (anchor_index, desired_offset) = expected_for_paint.get();
        let anchor_item = state.layout.item(anchor_index).unwrap();
        let offset = anchor_item.y - scroll_for_paint.vadjustment().value();
        let root_bounds = surface.root.compute_bounds(&scroll_for_paint).unwrap();
        let anchor_error = (offset - desired_offset)
            .abs()
            .max((f64::from(root_bounds.y()) + anchor_item.y - desired_offset).abs());
        if anchor_error > 2.0 {
            samples
                .borrow_mut()
                .push((anchor_index as u32, anchor_error));
        }
        for (&index, live) in surface.live_tiles.borrow().iter() {
            let item = state.layout.item(index as usize).unwrap();
            let tile = &live.tile;
            let bounds = tile.compute_bounds(&surface.root).unwrap();
            let frame = tile.first_child().unwrap();
            let error = [
                (f64::from(bounds.x()) - item.x).abs(),
                (f64::from(bounds.y()) - item.y).abs(),
                (f64::from(bounds.width()) - item.width).abs(),
                (f64::from(bounds.height()) - item.height).abs(),
                (f64::from(frame.width()) - item.width).abs(),
                (f64::from(frame.height()) - item.height).abs(),
            ]
            .into_iter()
            .fold(0.0_f64, f64::max);
            if error > 0.0 {
                samples.borrow_mut().push((index, error));
            }
        }
    });
    let viewport = scroll.clone().upcast::<gtk::Widget>();
    for _ in 0..6 {
        let anchor = gallery.capture_view_anchor().unwrap();
        let index = gallery
            .current_photos
            .borrow()
            .iter()
            .position(|photo| photo.id() == anchor.photo_id)
            .unwrap();
        expected_anchor.set((index, anchor.viewport_y_offset));
        gallery.wheel_zoom_in_at(&viewport, 500.0, 300.0);
        settle();
    }
    for _ in 0..6 {
        let anchor = gallery.capture_view_anchor().unwrap();
        let index = gallery
            .current_photos
            .borrow()
            .iter()
            .position(|photo| photo.id() == anchor.photo_id)
            .unwrap();
        expected_anchor.set((index, anchor.viewport_y_offset));
        gallery.wheel_zoom_out_at(&viewport, 500.0, 300.0);
        settle();
    }
    window.frame_clock().unwrap().disconnect(handler);
    assert!(
        errors.borrow().is_empty(),
        "zoom painted stale geometry or scroll translation: {:?}",
        errors.borrow()
    );
    window.close();
    settle();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn masonry_virtualizes_twenty_thousand_photos_and_resizes() {
    gtk::init().unwrap();
    crate::css::install_foundation(&gtk::gdk::Display::default().unwrap());
    let gallery = Rc::new(Gallery::new(
        &[],
        100,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    assert_eq!(gallery.layout(), PhotoLayout::Grid);
    let photos = (0..20_000)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property("width", if i % 2 == 0 { 4000_i64 } else { 6000_i64 })
                .property("height", if i % 2 == 0 { 6000_i64 } else { 4000_i64 })
                .build()
        })
        .collect::<Vec<_>>();
    gallery.current_photos.replace(photos.clone());
    gallery.store.splice(0, 0, &photos);
    gallery.selection.select_item(42, false);
    let info = crate::infobar::InfoBar::new();
    let gallery_for_mode = gallery.clone();
    info.connect_photo_layout(move |mode| gallery_for_mode.set_layout(mode));
    info.view_toggle.emit_clicked();
    info.view_toggle.emit_clicked();
    assert_eq!(gallery.layout(), PhotoLayout::Masonry);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&scroll);
    let window = gtk::Window::builder()
        .default_width(997)
        .default_height(600)
        .child(&scroll)
        .build();
    window.present();
    settle();
    for width in [997, 701, 1103] {
        window.set_default_size(width, 600);
        settle();
        scroll
            .vadjustment()
            .set_value((scroll.vadjustment().upper() - scroll.vadjustment().page_size()) * 0.4);
        settle();
        let surface = &gallery.sectioned_folder;
        assert!(surface.live_tiles.borrow().len() > 10);
        assert!(surface.live_tiles.borrow().len() < 300);
        assert_eq!(surface.geometry_width.get(), scroll.width());
        let state = surface.wall_state.borrow();
        for (&index, live) in surface.live_tiles.borrow().iter() {
            let item = state.layout.item(index as usize).unwrap();
            assert_eq!(live.tile.photo().unwrap().id(), photos[index as usize].id());
            assert_eq!(live.tile.width(), item.width as i32);
            assert_eq!(live.tile.height(), item.height as i32);
            assert!(item.x + item.width <= f64::from(scroll.width()));
            let frame = live.tile.first_child().unwrap();
            assert_eq!(frame.width(), live.tile.width());
            assert_eq!(frame.style_context().border(), gtk::Border::new());
        }
        assert!(gallery.selection.is_selected(42));
    }
    info.view_toggle.emit_clicked();
    assert_eq!(gallery.layout(), PhotoLayout::Honeycomb);
    info.view_toggle.emit_clicked();
    assert_eq!(gallery.layout(), PhotoLayout::Grid);
    assert!(gallery.selection.is_selected(42));
    window.close();
    settle();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn masonry_corners_follow_settings_and_filenames_stay_disabled() {
    gtk::init().unwrap();
    crate::css::install_foundation(&gtk::gdk::Display::default().unwrap());
    let picture = gtk::Picture::new();
    picture.add_css_class("thumbnail");
    let white = gtk::gdk::MemoryTexture::new(
        100,
        100,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(vec![255u8; 100 * 100 * 4]),
        400,
    );
    picture.set_paintable(Some(&white));
    let frame = gtk::Overlay::new();
    frame.add_css_class("photo-frame");
    frame.add_css_class("photo-tile");
    frame.set_overflow(gtk::Overflow::Hidden);
    frame.set_child(Some(&picture));
    let tile = SquareTile::new(100, 100, &frame);
    tile.add_css_class("photo-wall-tile");
    tile.add_css_class("masonry-tile");
    tile.set_filename_visible(true);
    assert!(
        tile.imp().filename_label.borrow().is_none(),
        "Masonry created a filename label"
    );
    assert_eq!(
        tile.measure(gtk::Orientation::Vertical, -1).0,
        100,
        "Masonry gained a filename row"
    );
    let window = gtk::Window::builder()
        .default_width(100)
        .default_height(100)
        .child(&tile)
        .build();
    window.present();
    settle();
    let render = || {
        let snapshot = gtk::Snapshot::new();
        window.snapshot_child(&tile, &snapshot);
        let node = snapshot.to_node().unwrap();
        let texture = window
            .renderer()
            .unwrap()
            .render_texture(&node, Some(&frame.compute_bounds(&window).unwrap()));
        let mut pixels = vec![0u8; 100 * 100 * 4];
        texture.download(&mut pixels, 400);
        pixels
    };
    let rounded = render();
    assert!(rounded[3] < 255, "default Masonry corner was square");
    window.add_css_class("square-corners");
    settle();
    assert_eq!(
        render()[3],
        255,
        "square-corner setting did not affect Masonry"
    );
    let baseline = render();
    tile.set_manual_selected(true);
    settle();
    let selected = render();
    let edge = (50 * 100 + 1) * 4;
    assert_ne!(
        &selected[edge..edge + 4],
        &baseline[edge..edge + 4],
        "selection outline disappeared"
    );
    assert_eq!(frame.style_context().border(), gtk::Border::new());
    window.close();
    settle();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn masonry_width_change_does_not_repack_after_idle() {
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
    let photos = (0..3_086)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property("width", [2000_i64, 4000, 6000, 8000][i % 4])
                .property("height", 4000_i64)
                .build()
        })
        .collect::<Vec<_>>();
    gallery.current_photos.replace(photos.clone());
    gallery.store.splice(0, 0, &photos);
    gallery.set_layout(PhotoLayout::Masonry);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&scroll);
    let window = gtk::Window::builder()
        .default_width(997)
        .default_height(600)
        .child(&scroll)
        .build();
    window.present();
    settle();
    let surface = &gallery.sectioned_folder;
    let adjustment = scroll.vadjustment();
    adjustment.set_value(surface.total_height.get() * 0.9);
    settle();
    let members = masonry_test_column_members(surface);
    let context = glib::MainContext::default();
    for width in [998, 701] {
        window.set_default_size(width, 600);
        let until = Instant::now() + std::time::Duration::from_secs(2);
        while (scroll.width() != width || surface.geometry_width.get() != width)
            && Instant::now() < until
        {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(scroll.width(), width);
        let generation = surface.wall_state.borrow().generation;
        settle();
        settle();
        assert_eq!(
            surface.wall_state.borrow().generation,
            generation,
            "idle resize rebuilt geometry"
        );
        assert_eq!(
            masonry_test_column_members(surface),
            members,
            "idle resize reassigned photos"
        );
        assert_eq!(gallery.current_zoom_width(), 100);
    }
    window.close();
    settle();
}

fn masonry_test_column_members(surface: &SectionedFolderView) -> Vec<(usize, usize)> {
    let state = surface.wall_state.borrow();
    let positions = state
        .layout
        .items
        .iter()
        .map(|item| item.x as i32)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut members = state
        .layout
        .items
        .iter()
        .map(|item| {
            (
                item.photo_index,
                positions.binary_search(&(item.x as i32)).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    members.sort_unstable();
    members
}

struct MasonryResizeFrame {
    width: i32,
    geometry_width: i32,
    viewport_error: f64,
    tile_error: f64,
    small_step_move: f64,
    visible_count: usize,
    covers_viewport: bool,
    preserves_members: bool,
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn masonry_deep_scroll_resize_keeps_every_painted_column_in_sync() {
    gtk::init().unwrap();
    for count in [3_086, 20_000] {
        for depth in [0.4, 0.9] {
            check_masonry_resize_frames(count, depth);
        }
    }
}

fn check_masonry_resize_frames(count: usize, depth: f64) {
    let gallery = Rc::new(Gallery::new(
        &[],
        100,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    // Cover flat Library/Photos and a collection with fixed folder headings.
    if count == 20_000 {
        gallery.group_mode.set(GroupMode::Folder);
    }
    let photos = (0..count)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property("folder-id", (i / 1000) as i64 + 1)
                .property("width", [2000_i64, 4000, 6000, 8000][i % 4])
                .property("height", 4000_i64)
                .build()
        })
        .collect::<Vec<_>>();
    gallery.current_photos.replace(photos.clone());
    gallery.store.splice(0, 0, &photos);
    gallery.rebuild_group_ranges();
    gallery.set_layout(PhotoLayout::Masonry);
    gallery.selection.select_item(42, false);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&scroll);
    let stack = gtk::Stack::new();
    stack.add_named(&scroll, Some("photos"));
    let gallery_for_width = gallery.clone();
    stack.add_tick_callback(move |stack, _| {
        gallery_for_width.update_width(stack.width());
        glib::ControlFlow::Continue
    });
    let window = gtk::Window::builder()
        .default_width(997)
        .default_height(600)
        .child(&stack)
        .build();
    window.present();
    settle();
    let surface = &gallery.sectioned_folder;
    // Establish and paint the deep viewport before requesting any resize.
    scroll
        .vadjustment()
        .set_value(surface.total_height.get() * depth);
    settle();
    let original = surface.wall_state.borrow().layout.clone();
    let original_top = scroll.vadjustment().value();
    let members = masonry_test_column_members(surface);
    let members_for_frame = members.clone();
    let reads = crate::source::original_read_count();
    let samples = Rc::new(RefCell::new(Vec::<MasonryResizeFrame>::new()));
    let samples_for_frame = samples.clone();
    let previous = RefCell::new(None::<(i32, HashMap<usize, f64>)>);
    let weak_surface = Rc::downgrade(surface);
    let weak_scroll = scroll.downgrade();
    let clock = window.frame_clock().unwrap();
    let handler = clock.connect_after_paint(move |_| {
        let (Some(surface), Some(scroll)) = (weak_surface.upgrade(), weak_scroll.upgrade()) else {
            return;
        };
        let width = scroll.width();
        let mut expected = original.clone();
        let wanted = expected.resize_masonry(width, original_top).unwrap();
        let root_bounds = surface.root.compute_bounds(&scroll).unwrap();
        let viewport_error = (scroll.vadjustment().value() - wanted)
            .abs()
            .max((f64::from(root_bounds.y()) + wanted).abs());
        let state = surface.wall_state.borrow();
        let mut tile_error = 0.0_f64;
        let mut visible = HashMap::new();
        for (&index, tile) in surface.live_tiles.borrow().iter() {
            let item = state.layout.item(index as usize).unwrap();
            let bounds = tile.tile.compute_bounds(&surface.root).unwrap();
            for delta in [
                (f64::from(bounds.x()) - item.x).abs(),
                (f64::from(bounds.y()) - item.y).abs(),
                (f64::from(bounds.width()) - item.width).abs(),
                (f64::from(bounds.height()) - item.height).abs(),
            ] {
                tile_error = tile_error.max(delta);
            }
            let painted = tile.tile.compute_bounds(&scroll).unwrap();
            if painted.y() < scroll.height() as f32 && painted.y() + painted.height() > 0.0 {
                visible.insert(index as usize, f64::from(painted.y()));
            }
        }
        let small_step_move = previous
            .borrow()
            .as_ref()
            .filter(|(old_width, _)| (width - old_width).abs() == 1)
            .map(|(_, positions)| {
                visible
                    .iter()
                    .filter_map(|(id, y)| positions.get(id).map(|old_y| (y - old_y).abs()))
                    .fold(0.0, f64::max)
            })
            .unwrap_or(0.0);
        let covers_viewport = expected
            .visible_row_indices(wanted, wanted + scroll.vadjustment().page_size())
            .into_iter()
            .all(|row| {
                surface
                    .live_tiles
                    .borrow()
                    .contains_key(&(expected.items[row].photo_index as u32))
            });
        let visible_count = visible.len();
        previous.replace(Some((width, visible)));
        samples_for_frame.borrow_mut().push(MasonryResizeFrame {
            width,
            geometry_width: surface.geometry_width.get(),
            viewport_error,
            tile_error,
            small_step_move,
            visible_count,
            covers_viewport,
            preserves_members: masonry_test_column_members(&surface) == members_for_frame,
        });
    });
    let context = glib::MainContext::default();
    for (width, height) in [
        (998, 600),
        (999, 600),
        (1000, 600),
        (999, 600),
        (998, 600),
        (997, 600),
        (997, 700),
        (701, 550),
        (1103, 600),
        (997, 600),
    ] {
        let first_sample = samples.borrow().len();
        window.set_default_size(width, height);
        let until = Instant::now() + std::time::Duration::from_secs(2);
        while Instant::now() < until
            && !samples.borrow()[first_sample..]
                .iter()
                .any(|sample| sample.width == width)
        {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(scroll.width(), width);
        assert!(
            samples.borrow().len() > first_sample,
            "resize did not paint a frame"
        );
    }
    settle();
    clock.disconnect(handler);
    let painted = samples.borrow();
    assert!(painted.windows(2).any(|pair| pair[0].width < pair[1].width));
    assert!(painted.windows(2).any(|pair| pair[0].width > pair[1].width));
    for frame in painted.iter() {
        assert_eq!(
            frame.width, frame.geometry_width,
            "painted width disagrees with geometry"
        );
        assert!(
            frame.visible_count > 0 && frame.covers_viewport,
            "painted an empty or incomplete viewport"
        );
        assert!(
            frame.preserves_members,
            "painted a transient column reassignment"
        );
        assert!(
            frame.viewport_error < 2.0,
            "count={count} depth={depth}: viewport error {}px",
            frame.viewport_error
        );
        assert!(
            frame.tile_error < 2.0,
            "count={count} depth={depth}: tile allocation error {}px",
            frame.tile_error
        );
        assert!(
            frame.small_step_move <= 3.0,
            "count={count} depth={depth}: one-pixel resize moved visible photos {}px",
            frame.small_step_move
        );
    }
    assert_eq!(masonry_test_column_members(surface), members);
    assert_eq!(crate::source::original_read_count(), reads);
    assert!(gallery.selection.is_selected(42));
    window.close();
    settle();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn masonry_resize_keeps_the_visible_photo_anchor() {
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
    let photos = (0..2000)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property("width", 4000_i64)
                .property("height", [2000_i64, 3000, 6000][i % 3])
                .build()
        })
        .collect::<Vec<_>>();
    gallery.current_photos.replace(photos.clone());
    gallery.store.splice(0, 0, &photos);
    gallery.set_layout(PhotoLayout::Masonry);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&scroll);
    let window = gtk::Window::builder()
        .default_width(997)
        .default_height(600)
        .child(&scroll)
        .build();
    window.present();
    settle();
    let surface = &gallery.sectioned_folder;
    let adjustment = scroll.vadjustment();
    adjustment.set_value((adjustment.upper() - adjustment.page_size()) * 0.4);
    settle();
    let members = masonry_test_column_members(surface);
    let original = surface.wall_state.borrow().layout.clone();
    let mut expected = original.clone();
    let old_value = adjustment.value();
    let new_width = scroll.width() + 10;
    let mapped = expected.resize_masonry(new_width, old_value).unwrap();
    window.set_default_size(new_width, 600);
    settle();
    assert!(
        (adjustment.value() - mapped).abs() < 2.0,
        "resize lost the logical viewport position"
    );
    assert_eq!(surface.geometry_width.get(), scroll.width());
    assert_eq!(masonry_test_column_members(surface), members);
    let generation = surface.wall_state.borrow().generation;
    settle();
    assert_eq!(surface.wall_state.borrow().generation, generation);
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn grid_lightbox_return_preserves_visible_photo_position() {
    check_grid_lightbox_return(false);
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn folder_grid_lightbox_return_preserves_visible_photo_position() {
    check_grid_lightbox_return(true);
}

fn check_grid_lightbox_return(folder: bool) {
    gtk::init().unwrap();
    crate::css::install_foundation(&gtk::gdk::Display::default().unwrap());
    let gallery = Rc::new(Gallery::new(
        &[],
        100,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    let photos = (0..2000)
        .map(|i| {
            glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property("width", 4000_i64)
                .property("height", 3000_i64)
                .property("folder-id", if i < 1000 { 1_i64 } else { 2_i64 })
                .build()
        })
        .collect::<Vec<_>>();
    gallery.current_photos.replace(photos.clone());
    gallery.store.splice(0, 0, &photos);
    if folder {
        gallery.group_mode.set(GroupMode::Folder);
        gallery.rebuild_group_ranges();
        gallery.sectioned_folder.refresh_model();
    }
    let root = gallery.visible_root();
    let scroll = gtk::ScrolledWindow::builder().child(&root).build();
    if folder {
        gallery.attach_sectioned_folder_scroll(&scroll);
    }
    let window = gtk::Window::builder()
        .default_width(997)
        .default_height(600)
        .child(&scroll)
        .build();
    let gallery_for_width = gallery.clone();
    scroll.add_tick_callback(move |scroll, _| {
        gallery_for_width.update_width(scroll.width());
        glib::ControlFlow::Continue
    });
    window.present();
    settle();
    let adjustment = scroll.vadjustment();
    adjustment.set_value(3000.0);
    settle();
    let mut tiles = Vec::new();
    collect_tiles(&root, &mut tiles);
    let tile = tiles
        .into_iter()
        .find(|tile| {
            tile.is_mapped()
                && tile.compute_bounds(&scroll).is_some_and(|b| {
                    b.y() > 0.0 && b.y() + b.height() < adjustment.page_size() as f32
                })
        })
        .unwrap();
    let photo = tile.photo().unwrap();
    let position = photos.iter().position(|p| p.id() == photo.id()).unwrap();
    gallery.selection.select_item(position as u32, true);
    for partial in [false, true] {
        let bounds = tile.compute_bounds(&scroll).unwrap();
        let offset = adjustment.page_size()
            - bounds.height() as f64 * if partial { 0.5 } else { 1.0 }
            - 12.0;
        adjustment.set_value(adjustment.value() + bounds.y() as f64 - offset);
        settle();
        let before = adjustment.value();
        gallery.restore_activated_photo(photo.id());
        gallery.grab_focus();
        settle();
        assert!((adjustment.value() - before).abs() < 1.0,
            "Grid lightbox return shifted: folder={folder}, partial={partial}, before={before}, after={}", adjustment.value());
        let focused = gtk::prelude::RootExt::focus(&window).unwrap();
        assert!(
            focused == tile.clone().upcast::<gtk::Widget>() || tile.is_ancestor(&focused),
            "focus did not return to the opened photo: {}",
            focused.type_().name()
        );
        assert_eq!(
            selected_positions(&gallery.selection),
            vec![position as u32]
        );
    }
    // Navigating in the viewer to an offscreen photo must still reveal it.
    let far = 1800;
    assert!(gallery.restore_activated_photo(photos[far].id()));
    gallery.grab_focus();
    settle();
    let mut tiles = Vec::new();
    collect_tiles(&root, &mut tiles);
    let returned = tiles
        .iter()
        .find(|tile| tile.is_mapped() && tile.photo().is_some_and(|p| p.id() == photos[far].id()))
        .unwrap();
    let bounds = returned.compute_bounds(&scroll).unwrap();
    assert!(bounds.y() < adjustment.page_size() as f32 && bounds.y() + bounds.height() > 0.0);
    let focused = gtk::prelude::RootExt::focus(&window).unwrap();
    assert!(focused == returned.clone().upcast::<gtk::Widget>() || returned.is_ancestor(&focused));
    assert_eq!(selected_positions(&gallery.selection), vec![far as u32]);

    // A subsequent user selection must cancel the queued return.
    let before = adjustment.value();
    gallery.restore_activated_photo(photos[0].id());
    gallery.selection.select_item(far as u32, true);
    settle();
    assert_eq!(adjustment.value(), before);
    assert_eq!(selected_positions(&gallery.selection), vec![far as u32]);
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn photo_wall_hidden_model_refresh_rebinds_tiles_on_remap() {
    gtk::init().unwrap();
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection.execute_batch(crate::db::SCHEMA).unwrap();
    connection
        .execute_batch(
            "INSERT INTO photos(id,path,width,height) VALUES
        (1,'/hidden-wall/old.jpg',6000,4000),(2,'/hidden-wall/new.jpg',4000,6000)",
        )
        .unwrap();
    let mut photos = crate::db::photos(&connection, None, false, None).unwrap();
    photos.sort_by_key(|photo| photo.id);
    let gallery = Rc::new(Gallery::new(
        &[],
        180,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&gallery.folder_sectioned_root));
    gallery.attach_sectioned_folder_scroll(&scroll);
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .build();
    window.set_child(Some(&scroll));
    gallery.replace(&photos[..1]);
    gallery.set_layout(PhotoLayout::PhotoWall);
    window.present();
    settle();
    assert!(gallery
        .sectioned_folder
        .live_tiles
        .borrow()
        .values()
        .any(|tile| tile
            .tile
            .imp()
            .photo
            .borrow()
            .as_ref()
            .is_some_and(|p| p.id() == 1)));
    window.set_visible(false);
    settle();
    gallery.replace(&photos[1..]);
    settle();
    window.present();
    settle();
    assert!(gallery
        .sectioned_folder
        .live_tiles
        .borrow()
        .values()
        .any(|tile| tile
            .tile
            .imp()
            .photo
            .borrow()
            .as_ref()
            .is_some_and(|p| p.id() == 2)));
    assert!(gallery
        .sectioned_folder
        .live_tiles
        .borrow()
        .values()
        .all(|tile| tile
            .tile
            .imp()
            .photo
            .borrow()
            .as_ref()
            .is_none_or(|p| p.id() != 1)));
    window.close();
}

/// Opt-in smoke check against a real catalog. Reads photo/folder rows only;
/// foreground preview workers may populate PIC's thumbnail cache.
#[test]
#[ignore = "requires GTK and PIC_TEST_CATALOG pointing to a real catalog"]
fn live_network_wall_browsing_keeps_prefetch_bounded() {
    gtk::init().unwrap();
    let catalog = std::env::var("PIC_TEST_CATALOG").expect("set PIC_TEST_CATALOG");
    let connection =
        rusqlite::Connection::open_with_flags(catalog, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let _ = crate::db::folders(&connection).unwrap();
    let photos = crate::db::photos(&connection, None, false, None).unwrap();
    let count = photos.len();
    let gallery = Rc::new(Gallery::new(
        &[],
        180,
        |_| {},
        |_, _, _| {},
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    let scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    gallery.attach_sectioned_folder_scroll(&scroll);
    gallery.set_layout(PhotoLayout::PhotoWall);
    let window = gtk::Window::builder()
        .default_width(1000)
        .default_height(700)
        .child(&scroll)
        .build();
    gallery.replace_owned_while_current(photos, Rc::new(|| true));
    let context = glib::MainContext::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while gallery.stream_building() && std::time::Instant::now() < deadline {
        context.iteration(false);
    }
    assert!(!gallery.stream_building());
    // Reproduce the window's immediate viewport targeting and persistent
    // completion drain, rather than relying on tile binds alone.
    let gallery_for_scroll = gallery.clone();
    scroll.vadjustment().connect_value_changed(move |_| {
        gallery_for_scroll.queue_visible_folder_cached_tiles_async(96);
    });
    let gallery_for_tick = gallery.clone();
    scroll.add_tick_callback(move |_, _| {
        gallery_for_tick.drain_thumbnail_display_completions();
        glib::ControlFlow::Continue
    });
    window.present();
    settle();
    let adjustment = scroll.vadjustment();
    for fraction in [0.0, 0.25, 0.75, 0.0] {
        adjustment.set_value((adjustment.upper() - adjustment.page_size()).max(0.0) * fraction);
        settle();
        gallery.queue_visible_folder_cached_tiles_async(96);
        gallery.prefetch_folder_cached_tiles(24, if fraction == 0.0 { -1.0 } else { 1.0 });
        let jobs = crate::thumbnail::network_prefetch_queued_count();
        assert!(
            jobs <= 8,
            "remote speculative queue exceeded its limit: {jobs}"
        );
        assert!(gallery.sectioned_folder.live_tiles.borrow().len() < 512);
        for (&index, tile) in gallery.sectioned_folder.live_tiles.borrow().iter() {
            let expected = gallery.current_photos.borrow()[index as usize].id();
            assert_eq!(
                tile.tile.imp().photo.borrow().as_ref().unwrap().id(),
                expected
            );
        }
        eprintln!(
            "LIVE_GALLERY_CHECK photos={} fraction={} realized={} remote_prefetch={}",
            count,
            fraction,
            gallery.sectioned_folder.live_tiles.borrow().len(),
            jobs
        );
    }
    window.close();
}

#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn layout_changes_do_not_generate_higher_quality_thumbnails() {
    gtk::init().unwrap();
    struct RestoreThumbnailQuality(bool);
    impl Drop for RestoreThumbnailQuality {
        fn drop(&mut self) {
            crate::thumbnail::set_high_quality_thumbnails_enabled(self.0);
        }
    }
    let _restore_quality =
        RestoreThumbnailQuality(crate::thumbnail::high_quality_thumbnails_enabled());
    crate::thumbnail::set_high_quality_thumbnails_enabled(false);
    let dir = std::env::temp_dir().join(format!("pic-quality-setting-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.jpg");
    let cache = dir.join("cache.jpg");
    image::RgbImage::from_pixel(1200, 400, image::Rgb([90, 160, 210]))
        .save(&source)
        .unwrap();
    image::RgbImage::from_pixel(320, 107, image::Rgb([90, 160, 210]))
        .save(&cache)
        .unwrap();
    for layout in [
        PhotoLayout::Grid,
        PhotoLayout::PhotoWall,
        PhotoLayout::Masonry,
    ] {
        let photo = glib::Object::builder::<PhotoObject>()
            .property("id", 730_i64)
            .property("path", source.to_str().unwrap())
            .property("width", 1200_i64)
            .property("height", 400_i64)
            .build();
        photo.set_cached_thumbnail_path(cache.to_str().unwrap());
        let gallery = Rc::new(Gallery::new(
            &[],
            300,
            |_| {},
            |_, _, _| {},
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
        ));
        gallery.store.append(&photo);
        gallery.current_photos.replace(vec![photo]);
        gallery.set_layout(layout);
        let child: gtk::Widget = if layout == PhotoLayout::Grid {
            gallery.root.clone().upcast()
        } else {
            gallery.folder_sectioned_root.clone().upcast()
        };
        let scroll = gtk::ScrolledWindow::builder().child(&child).build();
        if layout != PhotoLayout::Grid {
            gallery.attach_sectioned_folder_scroll(&scroll);
        }
        let width = if layout == PhotoLayout::Masonry {
            450
        } else {
            900
        };
        let window = gtk::Window::builder()
            .default_width(width)
            .default_height(500)
            .child(&scroll)
            .build();
        window.present();
        settle();
        window.set_default_size(width + 160, 500);
        settle();
        let high_quality_sibling = cache.with_file_name("cache-wall640.jpg");
        assert!(
            !high_quality_sibling.exists(),
            "changing to or resizing {layout:?} generated a higher-quality thumbnail while the setting was 320 px"
        );
        window.close();
        settle();
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires a GTK display; captures real renderer screenshots"]
fn honeycomb_gui_interactions_virtualization_and_screenshots() {
    libadwaita::init().unwrap();
    libadwaita::StyleManager::default().set_color_scheme(libadwaita::ColorScheme::ForceLight);
    let display = gtk::gdk::Display::default().unwrap();
    crate::css::install_foundation(&display);
    let css = gtk::CssProvider::new();
    css.load_from_string(include_str!("../../themes/standard/theme.css"));
    gtk::style_context_add_provider_for_display(
        &display,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
    let activated = Rc::new(Cell::new(None));
    let opened = activated.clone();
    let gallery = Rc::new(Gallery::new(
        &[],
        100,
        |_| {},
        move |photos, index, _| opened.set(Some(photos[index].id())),
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
    ));
    let paths = std::env::var("PIC_HONEYCOMB_PHOTOS")
        .ok()
        .map(|dir| {
            std::fs::read_dir(dir)
                .unwrap()
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .is_some_and(|e| e == "jpg" || e == "jpeg" || e == "png")
                })
                .take(64)
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            vec![
                "images/onboarding/wiz-edit.jpg".into(),
                "images/onboarding/wiz-browse.jpg".into(),
                "images/onboarding/wiz-albums.jpg".into(),
            ]
        });
    let textures = paths
        .iter()
        .map(|p| {
            gtk::gdk::Texture::from_filename(p)
                .unwrap()
                .upcast::<gtk::gdk::Paintable>()
        })
        .collect::<Vec<_>>();
    let objects = (0..10_001)
        .map(|i| {
            let photo = glib::Object::builder::<PhotoObject>()
                .property("id", i as i64 + 1)
                .property(
                    "taken-at",
                    if i < 5000 {
                        "2025-05-01 12:00:00"
                    } else {
                        "2026-05-01 12:00:00"
                    },
                )
                .property("original-available", i != 5)
                .property("favorite", i == 1)
                .build();
            photo.set_cached_thumbnail_path(format!(
                "/virtual/honeycomb/{}.jpg",
                i % textures.len()
            ));
            folder_thumbnail_cache_insert(
                photo_presentation_key(&photo).unwrap(),
                textures[i % textures.len()].clone(),
            );
            photo
        })
        .collect::<Vec<_>>();
    gallery.store.splice(0, 0, &objects);
    gallery.current_photos.replace(objects);
    gallery.selection.select_item(2, false);
    gallery.set_layout(PhotoLayout::Honeycomb);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&gallery.folder_sectioned_root)
        .build();
    scroll.add_css_class("photo-grid");
    gallery.attach_sectioned_folder_scroll(&scroll);
    let window = gtk::Window::builder()
        .title("PIC Honeycomb Smoke")
        .default_width(1000)
        .default_height(700)
        .child(&scroll)
        .build();
    window.present();
    settle();
    eprintln!(
        "Honeycomb renderer: {}",
        window.renderer().unwrap().type_().name()
    );
    if std::env::var("GSK_RENDERER").as_deref() == Ok("vulkan") {
        assert!(
            window.renderer().unwrap().type_().name().contains("Vulkan"),
            "Vulkan renderer unavailable"
        );
    }
    let surface = &gallery.sectioned_folder;
    assert!(surface.live_tiles.borrow().len() > 10);
    assert!(surface.live_tiles.borrow().len() < 250);
    let tile = surface.live_tiles.borrow()[&0].tile.clone();
    assert!(!tile.accepts_point(1.0, 1.0));
    assert!(tile.accepts_point(tile.width() as f64 / 2.0, tile.height() as f64 / 2.0));
    let state = surface.wall_state.borrow();
    let item = state.layout.item(0).unwrap();
    let point = gtk::graphene::Point::new(
        (item.x + item.width * 0.5) as f32,
        (item.y + item.height * 0.9) as f32,
    );
    let picked = surface
        .root
        .pick(point.x() as f64, point.y() as f64, gtk::PickFlags::DEFAULT)
        .unwrap();
    assert!(
        picked == tile || picked.is_ancestor(&tile),
        "overlapping empty corner intercepted picking"
    );
    drop(state);
    let controllers = tile.observe_controllers();
    let click = (0..controllers.n_items())
        .filter_map(|i| controllers.item(i).and_downcast::<gtk::GestureClick>())
        .find(|g| g.button() == 1)
        .unwrap();
    click.emit_by_name::<()>("pressed", &[&1_i32, &1.0_f64, &1.0_f64]);
    assert!(gallery.selection.is_selected(2));
    let x = tile.width() as f64 / 2.0;
    let y = tile.height() as f64 / 2.0;
    click.emit_by_name::<()>("pressed", &[&2_i32, &x, &y]);
    assert_eq!(activated.get(), Some(1));
    assert!(gallery.selection.is_selected(0));
    settle();
    assert!(
        tile.imp().visual_loaded.get(),
        "cached photo must be visible"
    );
    let hex_snapshot = gtk::Snapshot::new();
    gtk::WidgetPaintable::new(Some(&tile)).snapshot(
        &hex_snapshot,
        tile.width() as f64,
        tile.height() as f64,
    );
    let rendered = window.renderer().unwrap().render_texture(
        &hex_snapshot.to_node().unwrap(),
        Some(&gtk::graphene::Rect::new(
            0.0,
            0.0,
            tile.width() as f32,
            tile.height() as f32,
        )),
    );
    let stride = rendered.width() as usize * 4;
    let mut pixels = vec![0; stride * rendered.height() as usize];
    rendered.download(&mut pixels, stride);
    if std::env::var_os("PIC_HONEYCOMB_TILE_MASK_CACHE_TEST").is_none() {
        assert!(pixels[3] < 8, "hex corner must remain transparent");
        assert!(
            pixels[(rendered.height() as usize / 2) * stride
                + (rendered.width() as usize / 2) * 4
                + 3]
                > 240,
            "photo center must be opaque"
        );
    }
    if std::env::var_os("PIC_HONEYCOMB_TILE_MASK_CACHE_TEST").is_some() {
        let (source_w, source_h) = (31u32, 17u32);
        let mut source = image::RgbaImage::new(source_w, source_h);
        for (x, y, pixel) in source.enumerate_pixels_mut() {
            *pixel = image::Rgba([
                (x * 255 / source_w) as u8,
                (y * 255 / source_h) as u8,
                ((x + y) * 255 / (source_w + source_h)) as u8,
                255,
            ]);
        }
        let source_bytes = glib::Bytes::from_owned(source.clone().into_raw());
        let source_texture = gtk::gdk::MemoryTexture::new(
            source_w as i32,
            source_h as i32,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &source_bytes,
            source_w as usize * 4,
        );
        let frame = tile.first_child().and_downcast::<gtk::Overlay>().unwrap();
        let picture = frame.child().and_downcast::<gtk::Picture>().unwrap();
        let old_paintable = picture.paintable();
        picture.set_paintable(Some(&source_texture));
        settle();
        let render_tile = || {
            let snapshot = gtk::Snapshot::new();
            gtk::WidgetPaintable::new(Some(&tile)).snapshot(
                &snapshot,
                tile.width() as f64,
                tile.height() as f64,
            );
            window.renderer().unwrap().render_texture(
                &snapshot.to_node().unwrap(),
                Some(&gtk::graphene::Rect::new(
                    0.0,
                    0.0,
                    tile.width() as f32,
                    tile.height() as f32,
                )),
            )
        };
        let original = render_tile();
        let mut original_pixels =
            vec![0; original.width() as usize * original.height() as usize * 4];
        original.download(&mut original_pixels, original.width() as usize * 4);
        assert!(
            original_pixels[3] < 8,
            "normal hex corner must remain transparent"
        );
        let center = ((original.height() as usize / 2) * original.width() as usize
            + original.width() as usize / 2)
            * 4
            + 3;
        assert!(
            original_pixels[center] > 240,
            "normal image center must be opaque"
        );
        tile.prepare_honeycomb_mask(source_bytes.clone());
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while !tile.imp().honeycomb_mask_cached.get() && Instant::now() < deadline {
            gallery.drain_thumbnail_display_completions();
            settle();
        }
        assert!(
            tile.imp().honeycomb_mask_cached.get(),
            "worker result was not applied"
        );
        assert!(tile.imp().honeycomb_mask_cached.get());
        let masked = picture
            .paintable()
            .unwrap()
            .downcast::<gtk::gdk::Texture>()
            .unwrap();
        assert_eq!(
            (masked.width(), masked.height()),
            (
                tile.width() * tile.scale_factor(),
                tile.height() * tile.scale_factor()
            )
        );
        let cached = render_tile();
        if let Ok(directory) = std::env::var("PIC_HONEYCOMB_CACHE_COMPARE_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            original
                .save_to_png(std::path::Path::new(&directory).join("normal.png"))
                .unwrap();
            cached
                .save_to_png(std::path::Path::new(&directory).join("cached.png"))
                .unwrap();
        }
        let stride = original.width() as usize * 4;
        let mut cached_pixels = vec![0; stride * cached.height() as usize];
        cached.download(&mut cached_pixels, stride);
        let mean_difference = original_pixels
            .iter()
            .zip(&cached_pixels)
            .map(|(left, right)| left.abs_diff(*right) as u64)
            .sum::<u64>() as f64
            / original_pixels.len() as f64;
        assert!(
            mean_difference < 12.0,
            "cached tile image mean difference {mean_difference:.2}"
        );
        tile.clear_honeycomb_mask_cached();
        assert!(!tile.imp().honeycomb_mask_cached.get());
        assert_eq!(picture.paintable(), Some(source_texture.upcast()));
        assert!(
            tile.try_apply_cached_honeycomb_mask(),
            "warm cache entry was not reused"
        );
        assert!(tile.imp().honeycomb_mask_cached.get());
        tile.clear_honeycomb_mask_cached();
        picture.set_paintable(old_paintable.as_ref());
    }
    let offline = surface.live_tiles.borrow()[&5].tile.clone();
    let frame = offline
        .first_child()
        .and_downcast::<gtk::Overlay>()
        .unwrap();
    let badge = find_overlay_child(&frame, "offline-badge").unwrap();
    let bounds = badge.compute_bounds(&offline).unwrap();
    assert!(
        offline.accepts_point(bounds.x() as f64, bounds.y() as f64),
        "offline badge must be inside polygon"
    );
    // Capture the real X11 window, including the renderer's composited output.
    let xid = if std::env::var_os("PIC_HONEYCOMB_SCREENSHOTS").is_some() {
        let result = std::process::Command::new("xdotool")
            .args(["search", "--name", "^PIC Honeycomb Smoke$"])
            .output()
            .unwrap();
        assert!(result.status.success());
        Some(
            String::from_utf8(result.stdout)
                .unwrap()
                .lines()
                .last()
                .unwrap()
                .to_owned(),
        )
    } else {
        None
    };
    let resize_to = |width: i32, height: i32| {
        window.set_default_size(width, height);
        if let Some(id) = xid.as_ref() {
            assert!(std::process::Command::new("xdotool")
                .args(["windowmove", id, "0", "0"])
                .status()
                .unwrap()
                .success());
            assert!(std::process::Command::new("xdotool")
                .args(["windowsize", id, &width.to_string(), &height.to_string()])
                .status()
                .unwrap()
                .success());
        }
        settle();
    };
    let resize = |width| resize_to(width, 700);
    let save = |name: &str| {
        if let (Ok(dir), Some(id)) = (std::env::var("PIC_HONEYCOMB_SCREENSHOTS"), xid.as_ref()) {
            std::fs::create_dir_all(&dir).unwrap();
            assert!(std::process::Command::new("import")
                .args(["-window", id, &format!("{dir}/{name}.png")])
                .status()
                .unwrap()
                .success());
        }
    };
    let verify_edges = || {
        let state = surface.wall_state.borrow();
        let items = &state.layout.items[state.layout.rows[0].item_range.clone()];
        assert!(items.first().unwrap().x < 0.0);
        assert!(items.last().unwrap().x + items.last().unwrap().width > scroll.width() as f64);
        for item in [items.first().unwrap(), items.last().unwrap()] {
            let live = surface.live_tiles.borrow()[&(item.photo_index as u32)]
                .tile
                .clone();
            assert_eq!(
                live.photo().unwrap().id(),
                gallery.current_photos.borrow()[item.photo_index].id()
            );
            assert!(
                live.imp().visual_loaded.get(),
                "clipped edge must show a real photo"
            );
        }
        assert!(
            scroll.hadjustment().upper() <= scroll.hadjustment().page_size() + 1.0,
            "overhang expanded horizontal extent"
        );
    };
    verify_edges();
    let geometry_before = surface
        .wall_state
        .borrow()
        .layout
        .items
        .iter()
        .map(|item| {
            (
                item.photo_index,
                item.x.to_bits(),
                item.y.to_bits(),
                item.width.to_bits(),
                item.height.to_bits(),
            )
        })
        .collect::<Vec<_>>();
    gallery.set_honeycomb_border_width(16);
    settle();
    let geometry_after = surface
        .wall_state
        .borrow()
        .layout
        .items
        .iter()
        .map(|item| {
            (
                item.photo_index,
                item.x.to_bits(),
                item.y.to_bits(),
                item.width.to_bits(),
                item.height.to_bits(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        geometry_before, geometry_after,
        "border setting changed geometry or photo order"
    );
    save("border-16px");
    let adj = scroll.vadjustment();
    adj.set_value(adj.page_size() * 0.7);
    settle();
    save("border-16px-scrolled");
    adj.set_value(0.0);
    settle();
    gallery.set_honeycomb_border_width(5);
    settle();
    assert_eq!(scroll.hscrollbar_policy(), gtk::PolicyType::Never);
    scroll.hadjustment().set_value(200.0);
    assert_eq!(scroll.hadjustment().value(), scroll.hadjustment().lower());
    save("light");
    libadwaita::StyleManager::default().set_color_scheme(libadwaita::ColorScheme::ForceDark);
    settle();
    save("dark");
    for width in [560, 220, 1200] {
        resize(width);
        assert!(
            (window.width() - width).abs() < 64,
            "window did not resize: requested {width}, actual {}",
            window.width()
        );
        assert!(scroll.hadjustment().upper() <= scroll.hadjustment().page_size() + 1.0);
        scroll.vadjustment().set_value(0.0);
        settle();
        verify_edges();
        save(&format!("width-{width}"));
        gallery.set_honeycomb_borderless(true);
        settle();
        scroll.vadjustment().set_value(0.0);
        settle();
        verify_edges();
        save(&format!("borderless-width-{width}"));
        gallery.set_honeycomb_borderless(false);
        settle();
    }
    let pointer_tile = surface
        .live_tiles
        .borrow()
        .values()
        .filter_map(|entry| {
            entry
                .tile
                .compute_bounds(&scroll)
                .map(|bounds| (entry.tile.clone(), bounds))
        })
        .find(|(_, b)| b.y() > 100.0 && b.y() < 400.0)
        .unwrap();
    let pointer_id = pointer_tile.0.photo().unwrap().id();
    let pointer_y = pointer_tile.1.y() as f64;
    gallery.wheel_zoom_in_at(
        scroll.upcast_ref(),
        (pointer_tile.1.x() + pointer_tile.1.width() / 2.0) as f64,
        (pointer_tile.1.y() + pointer_tile.1.height() / 2.0) as f64,
    );
    settle();
    let pointer_index = gallery
        .current_photos
        .borrow()
        .iter()
        .position(|p| p.id() == pointer_id)
        .unwrap();
    assert!(
        (surface.y_for_index(pointer_index as u32).unwrap()
            - scroll.vadjustment().value()
            - pointer_y)
            .abs()
            < 2.0,
        "pointer anchor moved"
    );
    assert!(
        gallery.stable_zoom_anchor.get().is_none(),
        "wheel anchor must not leak into later slider zoom"
    );
    gallery.apply_wall_zoom(250);
    settle();
    save("zoom");
    let index = surface.wall_state.borrow().layout.rows[4].item_range.start;
    let id = gallery.current_photos.borrow()[index].id();
    assert!(surface.restore_anchor(id, 100.0));
    settle();
    let before = surface.y_for_index(index as u32).unwrap() - scroll.vadjustment().value();
    resize(900);
    let after = surface.y_for_index(index as u32).unwrap() - scroll.vadjustment().value();
    assert!(
        (before - after).abs() < 2.0,
        "resize anchor jumped: {before} -> {after}"
    );
    for fraction in [0.2, 0.7, 0.98] {
        let adj = scroll.vadjustment();
        adj.set_value((adj.upper() - adj.page_size()) * fraction);
        settle();
        assert!(surface.live_tiles.borrow().len() < 250);
    }
    assert!(surface.live_tiles.borrow().keys().any(|i| *i > 9000));
    scroll.vadjustment().set_value(0.0);
    settle();
    for mode in [
        PhotoLayout::Grid,
        PhotoLayout::Honeycomb,
        PhotoLayout::Masonry,
        PhotoLayout::Honeycomb,
    ] {
        gallery.set_layout(mode);
        settle();
        assert!(gallery.selection.is_selected(0));
    }
    gallery.set_grouping(GroupMode::Year, GroupDate::Taken);
    settle();
    assert_eq!(surface.wall_state.borrow().layout.sections.len(), 2);
    assert!(!surface.live_headers.borrow().is_empty());
    save("year");
    gallery.set_grouping(GroupMode::None, GroupDate::Taken);
    settle();
    assert_eq!(surface.wall_state.borrow().layout.sections.len(), 1);
    // Non-overlay scrollbars reduce the actual viewport width; geometry must
    // follow that allocation rather than the outer ScrolledWindow width.
    scroll.set_overlay_scrolling(false);
    for (width, height) in [(1366, 768), (1920, 1080), (2560, 1440)] {
        resize_to(width, height);
        for zoom in [100, 300] {
            gallery.apply_wall_zoom(zoom);
            settle();
            for borderless in [false, true] {
                gallery.set_honeycomb_borderless(borderless);
                settle();
                let adj = scroll.vadjustment();
                adj.set_value(0.0);
                settle();
                verify_edges();
                assert_eq!(
                    surface.geometry_width.get(),
                    surface.honeycomb_clip.borrow().as_ref().unwrap().width()
                );
                assert_eq!(scroll.hscrollbar_policy(), gtk::PolicyType::Never);
                let hadj = scroll.hadjustment();
                hadj.set_value(500.0);
                assert_eq!(hadj.value(), hadj.lower());
                save(&format!(
                    "edge-{width}x{height}-zoom-{zoom}-{}",
                    if borderless { "borderless" } else { "standard" }
                ));
                adj.set_value(adj.page_size() * 0.7);
                settle();
                hadj.set_value(500.0);
                assert_eq!(hadj.value(), hadj.lower());
                save(&format!(
                    "scrolled-{width}x{height}-zoom-{zoom}-{}",
                    if borderless { "borderless" } else { "standard" }
                ));
            }
        }
    }
    window.close();
}
