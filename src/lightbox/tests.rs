#[cfg(test)]
mod viewer_presentation_tests {
    use super::*;

    #[test]
    fn arrow_steps_remain_single_photo_and_clamp_at_collection_edges() {
        assert_eq!(navigation_step(4, 1, 10), 5);
        assert_eq!(navigation_step(4, -1, 10), 3);
        assert_eq!(navigation_step(0, -1, 10), 0);
        assert_eq!(navigation_step(9, 1, 10), 9);
    }

    #[test]
    fn wheel_steps_accumulate_without_dispatching_intermediate_targets() {
        let mut wheel = WheelNavigationState::default();
        wheel.begin(1);
        assert_eq!(wheel.queue_step(4, 1, 10), Some(5));
        assert_eq!(wheel.queue_step(4, 1, 10), Some(6));
        assert_eq!(wheel.queue_step(4, 1, 10), Some(7));
        assert_eq!(wheel.take_pending_target(), Some(7));
        assert_eq!(wheel.active_direction, 0);
    }

    #[test]
    fn arrow_clears_pending_wheel_target_without_changing_its_step() {
        let mut wheel = WheelNavigationState::default();
        wheel.begin(1);
        wheel.queue_step(4, 1, 10);
        wheel.queue_step(4, 1, 10);

        wheel.cancel();
        assert_eq!(wheel.take_pending_target(), None);
        assert_eq!(navigation_step(4, -1, 10), 3);
    }

    #[test]
    fn wheel_target_clamps_and_reversal_drops_old_pending_target() {
        let mut wheel = WheelNavigationState::default();
        wheel.begin(1);
        assert_eq!(wheel.queue_step(8, 1, 10), Some(9));
        assert_eq!(wheel.queue_step(8, 1, 10), None);

        wheel.cancel();
        wheel.begin(-1);
        assert_eq!(wheel.queue_step(8, -1, 10), Some(7));
        assert_eq!(wheel.take_pending_target(), Some(7));
    }

    #[test]
    fn cancelled_wheel_request_releases_the_controller() {
        let mut wheel = WheelNavigationState::default();
        wheel.begin(1);
        wheel.queue_step(4, 1, 10);

        wheel.cancel();
        assert_eq!(wheel.active_direction, 0);
        assert_eq!(wheel.take_pending_target(), None);
    }

    fn request_key(name: &str) -> ViewerRequestKey {
        ViewerRequestKey {
            path: format!("/test/{name}.jpg"),
            mtime: 1,
            size_bytes: 1,
            rotation: 0,
            edit_recipe: String::new(),
            target_width: 800,
            target_height: 600,
        }
    }

    #[test]
    fn foreground_promotes_prefetch_without_losing_its_work() {
        let key = request_key("promote");
        let (prefetch_request, prefetch, first) = claim_viewer_request(&key, false);
        assert!(matches!(first, ViewerRequestClaim::New));
        let (foreground_request, foreground, joined) = claim_viewer_request(&key, true);
        assert!(Arc::ptr_eq(&prefetch_request, &foreground_request));
        assert!(matches!(joined, ViewerRequestClaim::PromotedPrefetch));

        // Replacing the old prefetch lease leaves the promoted foreground
        // lease alive, so the shared worker remains eligible to finish.
        prefetch.cancel();
        assert!(foreground_request.has_consumers());
        assert!(foreground_request.foreground.load(Ordering::Acquire));

        finish_viewer_request(&key, &foreground_request, Err(Arc::from("test complete")));
        foreground.release();
    }

    #[test]
    fn identical_foregrounds_join_one_inflight_request() {
        let key = request_key("foreground-join");
        let (first_request, first, created) = claim_viewer_request(&key, true);
        assert!(matches!(created, ViewerRequestClaim::New));
        let (second_request, second, joined) = claim_viewer_request(&key, true);
        assert!(Arc::ptr_eq(&first_request, &second_request));
        assert!(matches!(joined, ViewerRequestClaim::JoinedForeground));
        assert_eq!(first_request.consumers.load(Ordering::Acquire), 2);

        first.cancel();
        assert!(second_request.has_consumers());
        finish_viewer_request(&key, &second_request, Err(Arc::from("test complete")));
        second.release();
    }

    #[test]
    fn older_generation_is_not_current() {
        assert!(viewer_generation_current(9, 9));
        assert!(!viewer_generation_current(10, 9));
    }

    #[test]
    fn large_photo_preview_uses_final_fitted_size() {
        assert_eq!(
            fitted_picture_dimensions(6016, 4016, 320, 214, 1036, 794, 0.0),
            (1036, 692)
        );
    }

    #[test]
    fn oriented_preview_swaps_native_axes_before_fitting() {
        assert_eq!(
            fitted_picture_dimensions(6016, 4016, 214, 320, 1036, 794, 0.0),
            (530, 794)
        );
    }

    #[test]
    fn small_native_photo_is_not_upscaled() {
        assert_eq!(
            fitted_picture_dimensions(226, 320, 226, 320, 1036, 794, 0.0),
            (226, 320)
        );
    }

    #[test]
    fn metadata_unknown_small_photo_keeps_its_native_size_in_fit_mode() {
        assert_eq!(
            fitted_picture_dimensions(0, 0, 240, 320, 1036, 794, 0.0),
            (240, 320)
        );
    }

    #[test]
    fn one_to_one_uses_decoded_raw_preview_dimensions() {
        assert_eq!(
            fitted_picture_dimensions(6016, 4016, 1620, 1080, 1036, 794, -1.0),
            (1620, 1080)
        );
    }

    #[test]
    fn one_to_one_uses_intrinsic_dimensions_when_metadata_is_unknown() {
        assert_eq!(
            fitted_picture_dimensions(0, 0, 3936, 2624, 1036, 794, -1.0),
            (3936, 2624)
        );
    }

    fn center_of(upper: f64, page_size: f64) -> f64 {
        (upper - page_size) * 0.5
    }

    #[test]
    fn scroll_anchor_centers_geometry_that_newly_overflows() {
        // Fitted (child no larger than the viewport, GTK centres it by
        // alignment) growing into a scrollable range on both axes. The values
        // come straight from a traced zoom: 1663x937 page, target sizes
        // stepping up to 200%.
        assert_eq!(centered_scroll_value(2647.0, 1663.0, 0.0), 492.0);
        assert_eq!(centered_scroll_value(3965.0, 937.0, 0.0), 1514.0);
        assert_eq!(centered_scroll_value(5248.0, 1663.0, 0.0), 1792.0);
        assert_eq!(centered_scroll_value(7872.0, 937.0, 0.0), 3468.0);
    }

    #[test]
    fn scroll_anchor_does_not_drift_across_a_zoom_cycle() {
        // 4x up, then all the way back down to fit, on both a page that makes
        // every step land on an exact pixel and one that makes every step land
        // on a half pixel. Neither previous range nor previous value is fed
        // back in, so the only way to drift is for the rule itself to be
        // history-dependent: every step must be exactly the centre of the
        // range it is given.
        for page_size in [937.0_f64, 938.0] {
            let uppers = [
                1035.0, 1230.0, 1425.0, 1816.0, 2304.0, 3086.0, 4063.0, 5174.0, 6265.0,
            ];
            let mut all = Vec::new();
            all.extend(uppers);
            all.extend(uppers.iter().rev().copied());
            all.push(page_size);
            for upper in all {
                let value = centered_scroll_value(upper, page_size, 0.0);
                // The scroll origin is an integer logical pixel, so an exact
                // centre on a half pixel is the worst case, and it is flat in
                // the magnification rather than growing with it.
                assert!(
                    (value - center_of(upper, page_size)).abs() <= 0.5,
                    "page {page_size} child {upper} landed at {value}"
                );
            }
        }

        // Zoomed all the way back out the image is at fit again: nothing to
        // scroll, so the origin is the only legal value.
        assert_eq!(centered_scroll_value(937.0, 937.0, 0.0), 0.0);
    }

    #[test]
    fn scroll_anchor_recenters_a_panned_photo() {
        // Zooming is expected to re-centre, so a photo left scrolled to 4000 by
        // an earlier pan is pulled back to the middle of the new range instead
        // of carrying the pan forward.
        assert_eq!(centered_scroll_value(7930.0, 937.0, 0.0), 3496.0);
    }

    #[test]
    fn scroll_anchor_recenters_when_the_viewport_resizes() {
        // Window resize: the child keeps its size and only the page changes.
        assert_eq!(centered_scroll_value(2000.0, 1500.0, 0.0), 250.0);
        assert_eq!(centered_scroll_value(2000.0, 1000.0, 0.0), 500.0);
    }

    #[test]
    fn scroll_anchor_collapses_to_the_origin_without_overflow() {
        // Nothing to scroll: the origin is the only legal value, and GTK
        // centres the child by alignment anyway.
        assert_eq!(centered_scroll_value(1663.0, 1663.0, 0.0), 0.0);
        assert_eq!(centered_scroll_value(800.0, 1663.0, 0.0), 0.0);
    }

    #[test]
    fn resetting_the_zoom_invalidates_the_applied_native_scale() {
        let zoom = Cell::new(0.0);
        let applied = Cell::new(APPLIED_SCALE_UNKNOWN);

        // The manual zoom path knows the native scale it landed on, so the
        // record is exact and the slider's repeat of it can be dropped.
        applied.set(1.5);

        // Navigating returns to fit. The record must not survive that, or
        // dragging the slider back to the same 1.5 would look like a repeat and
        // be dropped, leaving the image at fit under a slider that claims 1.5.
        set_zoom_state(&zoom, &applied, 0.0);
        assert_eq!(zoom.get(), 0.0);
        assert_eq!(applied.get(), 0.0);
        assert!((applied.get() - 1.5).abs() > f64::EPSILON);

        // Fit is the native scale 0.0, so a repeated fit report is still a
        // genuine repeat and stays droppable.
        assert!((applied.get() - 0.0).abs() <= f64::EPSILON);

        // The reserved 1:1 mode carries no manual scale, so it invalidates.
        set_zoom_state(&zoom, &applied, -1.0);
        assert_eq!(zoom.get(), -1.0);
        assert_eq!(applied.get(), APPLIED_SCALE_UNKNOWN);

        // A fit-relative zoom can only become a native scale where the fit
        // maths runs, so writing it here invalidates rather than guesses.
        set_zoom_state(&zoom, &applied, 1.5);
        assert_eq!(zoom.get(), 1.5);
        assert_eq!(applied.get(), APPLIED_SCALE_UNKNOWN);
    }

    #[test]
    fn fit_scale_and_renderer_agree_on_a_rotated_catalog() {
        // The catalog says 6016x4016 but the decoded texture is portrait, so
        // the source the viewer scales from is the swapped pair. The slider's
        // fit scale has to be the renderer's, or 100% on the slider stops
        // meaning native pixels.
        let native = (6016_i64, 4016_i64);
        let intrinsic = (214_i32, 320_i32);
        let viewport = (1663_i32, 937_i32);

        let (source_width, source_height, _) =
            presentation_source_dimensions(native.0, native.1, intrinsic.0, intrinsic.1, false);
        assert_eq!((source_width, source_height), (4016.0, 6016.0));
        let fit = presentation_fit_scale_from_source(
            source_width,
            source_height,
            viewport.0,
            viewport.1,
        );
        assert_eq!(fit, 937.0 / 6016.0);

        // 100% is fit * 1/fit, so the rendered size must be the native one.
        assert_eq!(
            fitted_picture_dimensions(
                native.0,
                native.1,
                intrinsic.0,
                intrinsic.1,
                viewport.0,
                viewport.1,
                1.0 / fit,
            ),
            (4016, 6016)
        );
    }
}



#[cfg(test)]
mod one_to_one_layout_regression {
    use super::*;

    fn settle() {
        let context = glib::MainContext::default();
        for _ in 0..20 {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn native_picture_is_centered_before_the_first_pointer_update() {
        gtk::init().unwrap();
        let lightbox = Lightbox::new();
        let window = gtk::Window::new();
        window.set_default_size(200, 120);
        window.set_child(Some(&lightbox.root));
        window.present();
        lightbox.root.set_visible(true);
        settle();

        let photo: PhotoObject = glib::Object::builder::<PhotoObject>()
            .property("id", 1_i64)
            .property("width", 400_i64)
            .property("height", 300_i64)
            .build();
        let preview_bytes = glib::Bytes::from_owned(vec![0; 200 * 120 * 4]);
        let preview = gtk::gdk::MemoryTexture::new(
            200,
            120,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &preview_bytes,
            200 * 4,
        );
        lightbox.picture.set_paintable(Some(&preview));
        lightbox.picture.set_can_shrink(true);
        fit_picture(
            &lightbox.picture,
            std::slice::from_ref(&photo),
            0,
            200,
            120,
            0.0,
            "test-fit",
        );
        settle();

        let native_bytes = glib::Bytes::from_owned(vec![0; 400 * 300 * 4]);
        let native = gtk::gdk::MemoryTexture::new(
            400,
            300,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &native_bytes,
            400 * 4,
        );
        lightbox.picture.set_paintable(Some(&native));
        lightbox.picture.set_can_shrink(false);
        fit_centered_picture(
            &lightbox.picture,
            &lightbox.picture_viewport,
            std::slice::from_ref(&photo),
            0,
            200,
            120,
            -1.0,
            "test-one-to-one",
        );
        settle();

        let bounds = lightbox
            .picture
            .compute_bounds(&lightbox.picture_viewport)
            .unwrap();
        let h = lightbox.picture_viewport.hadjustment();
        let v = lightbox.picture_viewport.vadjustment();
        eprintln!(
            "1:1 regression geometry: root={:?} scroll={:?} picture={:?} bounds={bounds:?} request={:?} h={}/{}/{}/{} v={}/{}/{}/{}",
            (lightbox.root.width(), lightbox.root.height()),
            (
                lightbox.picture_viewport.width(),
                lightbox.picture_viewport.height()
            ),
            (lightbox.picture.width(), lightbox.picture.height()),
            lightbox.picture.size_request(),
            h.value(), h.lower(), h.upper(), h.page_size(),
            v.value(), v.lower(), v.upper(), v.page_size(),
        );
        assert!((bounds.x() + h.value() as f32).abs() < 1.0);
        assert!((bounds.y() + v.value() as f32).abs() < 1.0);
        // The window manager and window decorations determine the actual
        // content allocation; set_default_size does not guarantee 200×120.
        assert!(h.page_size() < 400.0, "test must overflow horizontally");
        assert!(v.page_size() < 300.0, "test must overflow vertically");
        let expected_h = ((400.0 - h.page_size()) / 2.0).round_ties_even();
        let expected_v = ((300.0 - v.page_size()) / 2.0).round_ties_even();
        assert!((h.value() - expected_h).abs() < 1.0);
        assert!((v.value() - expected_v).abs() < 1.0);

        // Ctrl+wheel uses positive fit-relative zoom factors. Each step must
        // position the newly sized picture before the next pointer event,
        // including consecutive zoom-in, zoom-out, and return-to-Fit steps.
        lightbox.picture.set_can_shrink(true);
        for zoom in [0.0, 3.0, 4.0, 5.0, 4.0, 3.0, 0.0] {
            fit_centered_picture(
                &lightbox.picture,
                &lightbox.picture_viewport,
                std::slice::from_ref(&photo),
                0,
                lightbox.root.width(),
                lightbox.root.height(),
                zoom,
                "ctrl-wheel",
            );
            settle();
            let bounds = lightbox
                .picture
                .compute_bounds(&lightbox.picture_viewport)
                .unwrap();
            let expected_h = ((f64::from(lightbox.picture.width()) - h.page_size())
                .max(0.0) / 2.0).round_ties_even();
            let expected_v = ((f64::from(lightbox.picture.height()) - v.page_size())
                .max(0.0) / 2.0).round_ties_even();
            assert!((h.value() - expected_h).abs() < 1.0, "zoom={zoom}, h={h:?}");
            assert!((v.value() - expected_v).abs() < 1.0, "zoom={zoom}, v={v:?}");
            assert!(
                (f64::from(bounds.x()) + h.value()).abs() < 1.0,
                "zoom={zoom}, stale horizontal picture origin: bounds={bounds:?}, h={}",
                h.value(),
            );
            assert!(
                (f64::from(bounds.y()) + v.value()).abs() < 1.0,
                "zoom={zoom}, stale vertical picture origin: bounds={bounds:?}, v={}",
                v.value(),
            );
        }
        window.close();
    }
}

#[cfg(test)]
mod one_to_one_quality_regression {
    use super::*;

    #[test]
    #[ignore = "requires a GTK display"]
    fn small_photo_stays_fit_when_one_to_one_is_requested() {
        gtk::init().unwrap();
        let path = std::env::temp_dir().join(format!(
            "pic-small-one-to-one-{}.png", std::process::id()
        ));
        image::RgbaImage::from_pixel(640, 426, image::Rgba([120, 80, 40, 255]))
            .save(&path).unwrap();
        let photo: PhotoObject = glib::Object::builder::<PhotoObject>()
            .property("id", 1_i64)
            .property("path", path.to_string_lossy().as_ref())
            .property("width", 0_i64)
            .property("height", 0_i64)
            .build();
        photo.set_original_available(true);
        let lightbox = Lightbox::new();
        let window = gtk::Window::new();
        window.set_default_size(1380, 1094);
        window.set_child(Some(&lightbox.root));
        window.present();
        lightbox.open(vec![photo], 0);
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        while (picture_intrinsic_dimensions(&lightbox.picture) != (640, 426)
            || lightbox.picture.size_request() != (640, 426))
            && Instant::now() < deadline {
            while context.pending() { context.iteration(false); }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(picture_intrinsic_dimensions(&lightbox.picture), (640, 426));
        assert_eq!(lightbox.picture.size_request(), (640, 426),
            "Fit must not enlarge the recovered preview to fill the lightbox");
        lightbox.set_one_to_one(true);
        assert!(!lightbox.one_to_one_active.get(),
            "1:1 should remain off when the whole photo already fits");
        assert_eq!(picture_intrinsic_dimensions(&lightbox.picture), (640, 426));
        assert_eq!(lightbox.zoom.get(), 0.0);
        window.close();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "requires a GTK display"]
    fn one_to_one_waits_for_native_pixels() {
        gtk::init().unwrap();
        let path = std::env::temp_dir().join(format!(
            "pic-one-to-one-quality-{}.png", std::process::id()
        ));
        image::RgbaImage::from_pixel(800, 600, image::Rgba([120, 80, 40, 255]))
            .save(&path).unwrap();
        let photo: PhotoObject = glib::Object::builder::<PhotoObject>()
            .property("id", 1_i64)
            .property("path", path.to_string_lossy().as_ref())
            .property("width", 800_i64)
            .property("height", 600_i64)
            .build();
        photo.set_original_available(true);
        let lightbox = Lightbox::new();
        let window = gtk::Window::new();
        window.set_default_size(240, 180);
        window.set_child(Some(&lightbox.root));
        window.present();
        lightbox.open(vec![photo], 0);
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        while (lightbox.picture.paintable().is_none()
            || picture_intrinsic_dimensions(&lightbox.picture).0 >= 800)
            && Instant::now() < deadline {
            while context.pending() { context.iteration(false); }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(lightbox.picture.paintable().is_some());
        assert!(picture_intrinsic_dimensions(&lightbox.picture).0 < 800);
        lightbox.set_one_to_one(true);
        assert!(lightbox.picture.paintable().is_none(),
            "a fit-sized texture was exposed as the 1:1 image");
        let deadline = Instant::now() + Duration::from_secs(5);
        while picture_intrinsic_dimensions(&lightbox.picture) != (800, 600)
            && Instant::now() < deadline {
            while context.pending() { context.iteration(false); }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(picture_intrinsic_dimensions(&lightbox.picture), (800, 600));
        window.close();
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod wheel_quality_regression {
    use super::*;

    #[test]
    #[ignore = "requires an X11 GTK display and xdotool; run with --ignored --test-threads=1"]
    fn wheel_zoom_loads_native_pixels_without_one_to_one() {
        gtk::init().unwrap();
        let path =
            std::env::temp_dir().join(format!("pic-wheel-quality-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(800, 600, image::Rgba([120, 80, 40, 255]))
            .save(&path)
            .unwrap();
        let photo: PhotoObject = glib::Object::builder::<PhotoObject>()
            .property("id", 1_i64)
            .property("path", path.to_string_lossy().as_ref())
            .property("width", 800_i64)
            .property("height", 600_i64)
            .build();
        let lightbox = Lightbox::new();
        let window = gtk::Window::new();
        let title = format!("PIC wheel quality regression {}", std::process::id());
        window.set_title(Some(&title));
        window.set_default_size(240, 180);
        window.set_child(Some(&lightbox.root));
        window.present();
        lightbox.open(vec![photo], 0);
        let context = glib::MainContext::default();
        let settle_until = |label: &str, condition: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !condition() && Instant::now() < deadline {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(condition(), "timed out waiting for {label}");
        };
        settle_until("initial preview", &|| {
            lightbox.key_navigation_ready.get() && lightbox.picture.paintable().is_some()
        });
        assert!(
            picture_intrinsic_dimensions(&lightbox.picture).0 < 800,
            "test must start with a reduced fit preview"
        );
        let search = std::process::Command::new("xdotool")
            .args(["search", "--name", &title])
            .output()
            .unwrap();
        assert!(search.status.success());
        let windows = String::from_utf8(search.stdout).unwrap();
        let window_id = windows.lines().last().unwrap();
        let status = std::process::Command::new("xdotool")
            .args([
                "windowfocus",
                "--sync",
                window_id,
                "mousemove",
                "--window",
                window_id,
                "100",
                "90",
                "sleep",
                "0.2",
                "keydown",
                "ctrl",
                "sleep",
                "0.1",
                "click",
                "4",
                "keyup",
                "ctrl",
            ])
            .status()
            .unwrap();
        assert!(status.success());
        settle_until("wheel zoom", &|| lightbox.zoom.get() > 0.0);
        assert!(!lightbox.one_to_one_active.get());
        settle_until("native pixels after wheel zoom", &|| {
            picture_intrinsic_dimensions(&lightbox.picture) == (800, 600)
        });
        assert!(
            lightbox.native_texture.borrow().is_some(),
            "wheel must load the native source"
        );
        window.close();
        std::fs::remove_file(path).unwrap();
    }
}
