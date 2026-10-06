use super::*;
use std::time::{Duration, Instant};

fn settle() {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(600);
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn photo(path: &str, id: i64, available: bool) -> PhotoObject {
    glib::Object::builder()
        .property("id", id)
        .property("path", path)
        .property(
            "cached-thumbnail-path",
            crate::thumbnail::cache_path(path, Some(1), Some(2))
                .unwrap()
                .to_string_lossy()
                .as_ref(),
        )
        .property("width", 4000_i64)
        .property("height", 3000_i64)
        .property("mtime", 1_i64)
        .property("size-bytes", 2_i64)
        .property("original-available", available)
        .build()
}

fn texture(lightbox: &Lightbox) -> gtk::gdk::Texture {
    lightbox
        .picture
        .paintable()
        .expect("offline preview should be visible")
        .downcast::<gtk::gdk::Texture>()
        .unwrap()
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn offline_lightbox_shows_cached_thumbnail_instead_of_blank() {
    gtk::init().unwrap();
    let source = format!("/unmounted/pic-offline-test-{}.jpg", std::process::id());
    let cache = crate::thumbnail::cache_path(&source, Some(1), Some(2)).unwrap();
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    image::RgbImage::from_pixel(80, 60, image::Rgb([200, 30, 20]))
        .save_with_format(&cache, image::ImageFormat::Jpeg)
        .unwrap();
    let lightbox = Lightbox::new();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&lightbox.root)
        .build();
    window.present();
    lightbox.open(vec![photo(&source, 1, false)], 0);
    settle();
    assert_eq!(
        (texture(&lightbox).width(), texture(&lightbox).height()),
        (80, 60)
    );
    assert!(
        lightbox.picture.width() > 80,
        "preview should fit the lightbox"
    );
    assert!(
        lightbox.display_texture_cache.borrow().is_empty(),
        "thumbnail must not pollute full-quality cache"
    );
    lightbox.close();
    window.close();
    let _ = std::fs::remove_file(cache);
}

fn window(lightbox: &Lightbox) -> gtk::Window {
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&lightbox.root)
        .build();
    window.present();
    window
}

fn write_cache(source: &str, width: u32, height: u32) -> std::path::PathBuf {
    let cache = crate::thumbnail::cache_path(source, Some(1), Some(2)).unwrap();
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    image::RgbImage::from_pixel(width, height, image::Rgb([200, 30, 20]))
        .save_with_format(&cache, image::ImageFormat::Jpeg)
        .unwrap();
    cache
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn offline_navigation_clears_previous_photo_when_cache_is_missing() {
    gtk::init().unwrap();
    let source = format!("/unmounted/pic-nav-{}.jpg", std::process::id());
    let cache = write_cache(&source, 80, 60);
    let missing = format!("/unmounted/pic-no-cache-{}.jpg", std::process::id());
    let lightbox = Lightbox::new();
    let window = window(&lightbox);
    let reads = crate::source::original_read_count();
    let clicked = Rc::new(Cell::new(0));
    let clicked_for_handler = clicked.clone();
    lightbox.set_unavailable_handler(move |photo, _| clicked_for_handler.set(photo.id()));
    lightbox.open(vec![photo(&source, 1, false), photo(&missing, 2, false)], 0);
    settle();
    assert!(lightbox.offline.badge.is_visible());
    assert!(lightbox.offline.notice.label().contains("Cached thumbnail"));
    lightbox.offline.badge.emit_clicked();
    assert_eq!(clicked.get(), 1);
    lightbox.navigate_photo(1);
    settle();
    assert!(lightbox.picture.paintable().is_none());
    assert_eq!(
        lightbox.offline.notice.label(),
        "Original unavailable — no cached preview"
    );
    assert!(lightbox.key_navigation_ready.get());
    lightbox.offline.badge.emit_clicked();
    assert_eq!(clicked.get(), 2, "badge must refer to the current photo");
    assert_eq!(crate::source::original_read_count(), reads);
    lightbox.navigate_photo(-1);
    settle();
    assert_eq!(texture(&lightbox).width(), 80);
    lightbox.close();
    assert!(!lightbox.offline.badge.is_visible());
    assert!(lightbox.offline.tracked_photo.borrow().is_none());
    window.close();
    let _ = std::fs::remove_file(cache);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn offline_preview_prefers_quality_cache_and_applies_rotation() {
    gtk::init().unwrap();
    let source = format!("/unmounted/pic-quality-{}.jpg", std::process::id());
    let cache = write_cache(&source, 80, 60);
    let quality = crate::thumbnail::wall_cache_path(&cache);
    image::RgbImage::from_pixel(160, 120, image::Rgb([20, 90, 180]))
        .save_with_format(&quality, image::ImageFormat::Jpeg)
        .unwrap();
    let selected = photo(&source, 1, false);
    selected.set_rotation(90);
    let lightbox = Lightbox::new();
    let window = window(&lightbox);
    lightbox.open(vec![selected.clone()], 0);
    settle();
    assert_eq!(
        (texture(&lightbox).width(), texture(&lightbox).height()),
        (120, 160)
    );
    assert_eq!((selected.width(), selected.height()), (4000, 3000));
    // A corrupt high-quality cache must still allow the normal thumbnail.
    std::fs::write(&quality, b"not an image").unwrap();
    lightbox.refresh_current();
    settle();
    assert_eq!(
        (texture(&lightbox).width(), texture(&lightbox).height()),
        (60, 80)
    );
    let mut recipe = crate::edit::EditRecipe::default();
    recipe.crop.right = 0.5;
    selected.set_edit_recipe(recipe.encode());
    lightbox.refresh_current();
    settle();
    assert_eq!(
        (texture(&lightbox).width(), texture(&lightbox).height()),
        (30, 80),
        "cached preview must retain the gallery's saved crop"
    );
    lightbox.close();
    window.close();
    let _ = std::fs::remove_file(cache);
    let _ = std::fs::remove_file(quality);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn offline_preview_stale_completion_cannot_replace_a_new_photo() {
    gtk::init().unwrap();
    let source = format!("/unmounted/pic-stale-{}.jpg", std::process::id());
    let cache = write_cache(&source, 80, 60);
    let lightbox = Lightbox::new();
    let window = window(&lightbox);
    lightbox.root.set_visible(true);
    settle();
    show_offline_preview(
        &lightbox.picture,
        photo(&source, 1, false),
        &lightbox.root,
        lightbox.zoom.clone(),
        lightbox.load_generation.clone(),
        0,
        lightbox.offline.clone(),
        true,
        None,
        None,
    );
    // Invalidate before the worker can publish its result on GTK's context.
    lightbox.load_generation.set(1);
    let sentinel = gtk::gdk::MemoryTexture::new(
        10,
        10,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(vec![255; 400]),
        40,
    );
    lightbox.picture.set_paintable(Some(&sentinel));
    lightbox.offline.online();
    settle();
    assert_eq!(texture(&lightbox).width(), 10);
    assert!(!lightbox.offline.badge.is_visible());
    lightbox.close();
    window.close();
    let _ = std::fs::remove_file(cache);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn offline_lightbox_reloads_original_when_source_reconnects() {
    gtk::init().unwrap();
    let original = std::env::temp_dir().join(format!("pic-reconnect-{}.jpg", std::process::id()));
    let path = original.to_string_lossy().into_owned();
    let cache = write_cache(&path, 80, 60);
    let selected = photo(&path, 1, false);
    let lightbox = Lightbox::new();
    let window = window(&lightbox);
    lightbox.open(vec![selected.clone()], 0);
    settle();
    assert_eq!(texture(&lightbox).width(), 80);
    image::RgbImage::from_pixel(400, 300, image::Rgb([20, 90, 180]))
        .save(&original)
        .unwrap();
    selected.set_original_available(true);
    settle();
    assert!(texture(&lightbox).width() > 80);
    assert!(!lightbox.offline.badge.is_visible());
    assert!(!lightbox.offline.notice.is_visible());
    assert!(!lightbox.display_texture_cache.borrow().is_empty());
    lightbox.close();
    window.close();
    let _ = std::fs::remove_file(cache);
    let _ = std::fs::remove_file(original);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn offline_lightbox_retry_checks_a_restored_individual_file() {
    gtk::init().unwrap();
    let original = std::env::temp_dir().join(format!("pic-retry-{}.jpg", std::process::id()));
    let path = original.to_string_lossy().into_owned();
    let cache = write_cache(&path, 80, 60);
    let selected = photo(&path, 1, false);
    let lightbox = Lightbox::new();
    let window = window(&lightbox);
    lightbox.open(vec![selected.clone()], 0);
    settle();
    image::RgbImage::from_pixel(400, 300, image::Rgb([20, 90, 180]))
        .save(&original)
        .unwrap();
    lightbox.retry_current_original();
    settle();
    assert!(selected.original_available());
    assert!(!lightbox.offline.badge.is_visible());
    assert!(texture(&lightbox).width() > 80);
    lightbox.close();
    window.close();
    let _ = std::fs::remove_file(cache);
    let _ = std::fs::remove_file(original);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn offline_preview_handles_disconnect_during_open_but_not_corrupt_originals() {
    gtk::init().unwrap();
    let absent = std::env::temp_dir().join(format!("pic-absent-{}.jpg", std::process::id()));
    let absent_path = absent.to_string_lossy().into_owned();
    let cache = write_cache(&absent_path, 80, 60);
    let selected = photo(&absent_path, 1, true); // availability is stale
    let lightbox = Lightbox::new();
    let window = window(&lightbox);
    lightbox.open(vec![selected.clone()], 0);
    settle();
    assert_eq!(texture(&lightbox).width(), 80);
    assert!(lightbox.offline.badge.is_visible());
    assert!(!selected.original_available());
    // Existing, readable files with invalid image data are not offline.
    std::fs::write(&absent, b"not an image").unwrap();
    selected.set_original_available(true);
    settle();
    assert!(lightbox.picture.paintable().is_none());
    assert!(!lightbox.offline.badge.is_visible());
    assert!(selected.original_available());
    lightbox.close();
    window.close();
    let _ = std::fs::remove_file(cache);
    let _ = std::fs::remove_file(absent);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn online_offline_online_navigation_never_retains_another_photos_thumbnail() {
    gtk::init().unwrap();
    let original = std::env::temp_dir().join(format!("pic-online-{}.jpg", std::process::id()));
    image::RgbImage::from_pixel(400, 300, image::Rgb([20, 90, 180]))
        .save(&original)
        .unwrap();
    let original_path = original.to_string_lossy().into_owned();
    let absent = format!("/unmounted/pic-offline-nav-{}.jpg", std::process::id());
    let cache = write_cache(&absent, 80, 60);
    let lightbox = Lightbox::new();
    let window = window(&lightbox);
    lightbox.open(
        vec![photo(&original_path, 1, true), photo(&absent, 2, false)],
        0,
    );
    settle();
    assert!(texture(&lightbox).width() > 80);
    assert!(!lightbox.offline.badge.is_visible());
    lightbox.navigate_photo(1);
    settle();
    assert_eq!(texture(&lightbox).width(), 80);
    assert!(lightbox.offline.badge.is_visible());
    // Force an online cache miss to exercise the pending original load.
    lightbox.display_texture_cache.borrow_mut().clear();
    lightbox.navigate_photo(-1);
    assert!(
        lightbox.picture.paintable().is_none(),
        "previous offline thumbnail leaked into online load"
    );
    settle();
    assert!(texture(&lightbox).width() > 80);
    assert!(!lightbox.offline.badge.is_visible());
    lightbox.close();
    window.close();
    let _ = std::fs::remove_file(cache);
    let _ = std::fs::remove_file(original);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn reconnect_before_cached_completion_does_not_restore_offline_state() {
    gtk::init().unwrap();
    let original = std::env::temp_dir().join(format!("pic-race-{}.jpg", std::process::id()));
    image::RgbImage::from_pixel(400, 300, image::Rgb([20, 90, 180]))
        .save(&original)
        .unwrap();
    let path = original.to_string_lossy().into_owned();
    let cache = write_cache(&path, 80, 60);
    let selected = photo(&path, 1, false);
    let lightbox = Lightbox::new();
    let window = window(&lightbox);
    lightbox.photos.replace(vec![selected.clone()]);
    lightbox.offline.track(&selected);
    lightbox.root.set_visible(true);
    settle();
    let request = crate::thumbnail_display::request_for(
        cache.to_string_lossy().into_owned(),
        path.clone(),
        1,
        2,
        0,
        String::new(),
        4000,
        3000,
        true,
    );
    let pending_preview = load_cached_lightbox_preview(&request).unwrap();
    let expected_revision = lightbox.offline.availability_revision.get();
    selected.set_original_available(true);
    apply_offline_preview_result(
        &lightbox.picture,
        &selected,
        &lightbox.root,
        0.0,
        &lightbox.offline,
        true,
        expected_revision,
        Some(Some(pending_preview)),
    );
    assert!(
        selected.original_available(),
        "cached completion reverted reconnection"
    );
    settle();
    assert!(!lightbox.offline.badge.is_visible());
    assert!(texture(&lightbox).width() > 80);
    lightbox.close();
    window.close();
    let _ = std::fs::remove_file(cache);
    let _ = std::fs::remove_file(original);
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn decode_failure_fallback_cannot_undo_a_later_reconnection() {
    gtk::init().unwrap();
    let original =
        std::env::temp_dir().join(format!("pic-unknown-race-{}.jpg", std::process::id()));
    image::RgbImage::from_pixel(400, 300, image::Rgb([20, 90, 180]))
        .save(&original)
        .unwrap();
    let path = original.to_string_lossy().into_owned();
    let cache = write_cache(&path, 80, 60);
    let selected = photo(&path, 1, true);
    let lightbox = Lightbox::new();
    let window = window(&lightbox);
    lightbox.photos.replace(vec![selected.clone()]);
    lightbox.offline.track(&selected);
    lightbox.root.set_visible(true);
    settle();
    let request = crate::thumbnail_display::request_for(
        cache.to_string_lossy().into_owned(),
        path.clone(),
        1,
        2,
        0,
        String::new(),
        4000,
        3000,
        true,
    );
    let pending_preview = load_cached_lightbox_preview(&request).unwrap();
    let expected_revision = lightbox.offline.availability_revision.get();
    // Source probe completed offline; availability then changed before GTK
    // could publish the worker's cached result.
    selected.set_original_available(false);
    selected.set_original_available(true);
    apply_offline_preview_result(
        &lightbox.picture,
        &selected,
        &lightbox.root,
        0.0,
        &lightbox.offline,
        false,
        expected_revision,
        Some(Some(pending_preview)),
    );
    assert!(
        selected.original_available(),
        "decode-failure fallback reverted reconnection"
    );
    settle();
    assert!(!lightbox.offline.badge.is_visible());
    assert!(texture(&lightbox).width() > 80);
    lightbox.close();
    window.close();
    let _ = std::fs::remove_file(cache);
    let _ = std::fs::remove_file(original);
}
