// Wheel and slider zoom share the native source request and cache.
fn ensure_native_zoom_texture(
    picture: &gtk::Picture,
    root: &gtk::Overlay,
    photos: Rc<RefCell<Vec<PhotoObject>>>,
    index: Rc<Cell<usize>>,
    zoom: Rc<Cell<f64>>,
    one_to_one: Rc<Cell<bool>>,
    native_texture: Rc<RefCell<Option<NativeTextureCache>>>,
    pending: Rc<Cell<bool>>,
    photo: &PhotoObject,
) {
    let path = photo.path();
    let rotation = photo.rotation();
    let edit_recipe = photo.edit_recipe();

    if let Some(cache) = native_texture.borrow().as_ref() {
        if cache.path == path && cache.rotation == rotation && cache.edit_recipe == edit_recipe {
            let request = picture.size_request();
            picture.set_paintable(Some(&cache.texture));
            if picture.size_request() != request {
                picture.set_size_request(request.0, request.1);
            }
            picture.queue_draw();
            return;
        }
    }

    if !photo.original_available() {
        return;
    }
    if pending.replace(true) {
        return;
    }

    let (target_width, target_height, _, _, _, _) = viewer_decode_target(&root, rotation, true);
    let key = ViewerRequestKey {
        path: path.clone(),
        mtime: photo.mtime(),
        size_bytes: photo.size_bytes(),
        rotation,
        edit_recipe: edit_recipe.clone(),
        target_width,
        target_height,
    };
    let (request, lease, claim) = claim_viewer_request(&key, true);
    viewer_trace(format!(
        "request lane=native-zoom action={} uri={} variant={}x{}",
        match claim {
            ViewerRequestClaim::New => "new",
            ViewerRequestClaim::JoinedForeground => "join",
            ViewerRequestClaim::PromotedPrefetch => "promote",
        },
        viewer_trace_uri(&path),
        target_width,
        target_height,
    ));
    if matches!(claim, ViewerRequestClaim::New) {
        start_viewer_request(key, request.clone());
    }

    let picture = picture.clone();
    let root = root.clone();
    let photos = photos.clone();
    let index = index.clone();
    let zoom = zoom.clone();
    let one_to_one = one_to_one.clone();
    let native_texture = native_texture.clone();
    let pending = pending.clone();
    let expected_path = path.clone();
    let expected_rotation = rotation;
    let expected_recipe = edit_recipe.clone();

    glib::MainContext::default().spawn_local(async move {
        let result = ViewerResultSlot::wait(request.result.clone()).await;
        pending.set(false);

        if lease.cancelled() || !root.is_visible() {
            lease.release();
            return;
        }

        let still_current = photos.borrow().get(index.get()).is_some_and(|current| {
            current.path() == expected_path
                && current.rotation() == expected_rotation
                && current.edit_recipe() == expected_recipe
        });
        if !still_current {
            lease.release();
            return;
        }

        if let Ok(result) = result {
            let bytes = glib::Bytes::from_owned(result.pixels.clone());
            let texture = gtk::gdk::MemoryTexture::new(
                result.width as i32,
                result.height as i32,
                gtk::gdk::MemoryFormat::R8g8b8a8,
                &bytes,
                result.width as usize * 4,
            );
            *native_texture.borrow_mut() = Some(NativeTextureCache {
                path: expected_path.clone(),
                rotation: expected_rotation,
                edit_recipe: expected_recipe.clone(),
                texture: texture.clone(),
            });

            // Replace only the source texture, never the current zoom
            // geometry. This turns the temporary viewport-sized preview
            // into a sharp native source without moving the photo.
            if zoom.get() > 0.0 && !one_to_one.get() {
                let request = picture.size_request();
                picture.set_paintable(Some(&texture));
                if picture.size_request() != request {
                    picture.set_size_request(request.0, request.1);
                }
                picture.queue_draw();
                viewer_trace(format!(
                    "native_zoom_apply uri={} texture={}x{} request={:?}",
                    viewer_trace_uri(&expected_path),
                    texture.width(),
                    texture.height(),
                    request,
                ));
            }
        }
        lease.release();
    });
}
fn notify_photo_changed(handler: &PhotoChangedHandler, photos: &[PhotoObject], index: usize) {
    let Some(photo) = photos.get(index) else {
        return;
    };
    if let Some(handler) = handler.borrow().as_ref() {
        handler(photo.clone());
    }
}

fn show_photo(
    picture: &gtk::Picture,
    photos: &[PhotoObject],
    index: usize,
    root: &gtk::Overlay,
    zoom: Rc<Cell<f64>>,
    generation: Rc<Cell<u64>>,
    expected_generation: u64,
    decode_cancel: Rc<RefCell<Option<Arc<ViewerRequestLease>>>>,
    picture_viewport: &gtk::ScrolledWindow,
    native_texture: Rc<RefCell<Option<NativeTextureCache>>>,
    display_texture_cache: DisplayTextureCache,
    fit_geometry_fixed: bool,
    cache_hit: bool,
    navigation_ready: Option<Rc<Cell<bool>>>,
    navigation_settled: Option<Rc<dyn Fn()>>,
    offline: Rc<OfflinePresentation>,
) {
    let navigation_started = std::time::Instant::now();
    let Some(photo) = photos.get(index) else {
        return;
    };

    // Decode a display-quality image off the GTK thread. Never display a
    // thumbnail for online photos; keep the previous full-size image during
    // navigation and show a neutral backdrop on initial open.
    let path = photo.path();
    if png_uses_theme_background(&path) {
        // PNG alpha should reveal the active theme's photo surface, not the
        // lightbox's black/dark backdrop. Every bundled theme already owns
        // this class, so the matte follows theme changes without duplicating
        // palette values in the viewer.
        picture.add_css_class("photo-grid");
    } else {
        picture.remove_css_class("photo-grid");
    }
    viewer_trace(format!(
        "show_photo_request generation={} index={} uri={}",
        expected_generation,
        index,
        viewer_trace_uri(&path),
    ));
    if let Some(previous) = decode_cancel.borrow_mut().take() {
        previous.cancel();
    }
    offline.track(photo);
    offline.retry_requested.set(false);
    if photo.original_available() {
        // Do not carry a different photo's offline thumbnail into an online
        // load. A matching full-quality RAM cache has already replaced it.
        if offline.badge.is_visible() && !cache_hit {
            picture.set_paintable(gtk::gdk::Paintable::NONE);
            picture.set_filename(Option::<&str>::None);
            picture.set_size_request(1, 1);
        }
        offline.online();
    } else {
        offline.cached(!cache_hit);
        if !cache_hit {
            cancel_lightbox_prefetch_except(None);
            VIEWER_FOREGROUND_GENERATION.store(0, Ordering::Release);
            show_offline_preview(
                picture, photo.clone(), root, zoom, generation,
                expected_generation, offline, true, navigation_ready, navigation_settled,
            );
            return;
        }
    }
    if cache_hit {
        viewer_trace(format!(
            "cache_hit lane=foreground uri={}",
            viewer_trace_uri(&path)
        ));
        viewer_trace(format!(
            "display_done lane=foreground source=texture_cache navigation_ms={} uri={}",
            navigation_started.elapsed().as_millis(),
            viewer_trace_uri(&path)
        ));
        cancel_lightbox_prefetch_except(None);
        VIEWER_FOREGROUND_GENERATION.store(0, Ordering::Release);
        if let Some(navigation_ready) = navigation_ready {
            navigation_ready.set(true);
        }
        if let Some(navigation_settled) = navigation_settled {
            navigation_settled();
        }
        return;
    }
    let rotation = photo.rotation();
    let edit_recipe_text = photo.edit_recipe();
    let (target_width, target_height, _, _, _, _) =
        viewer_decode_target(root, rotation, zoom.get() < 0.0);
    let key = ViewerRequestKey {
        path: path.clone(),
        mtime: photo.mtime(),
        size_bytes: photo.size_bytes(),
        rotation,
        edit_recipe: edit_recipe_text.clone(),
        target_width,
        target_height,
    };
    // Keep a matching prefetch alive: the foreground lease below promotes it.
    // Other speculative requests lose their leases before they can occupy a
    // decode slot ahead of the selected image.
    cancel_lightbox_prefetch_except(Some(&key));
    VIEWER_FOREGROUND_GENERATION.store(expected_generation, Ordering::Release);
    let (request, lease, claim) = claim_viewer_request(&key, true);
    viewer_trace(format!(
        "request lane=foreground action={} uri={} variant={}x{}",
        match claim {
            ViewerRequestClaim::New => "new",
            ViewerRequestClaim::JoinedForeground => "join",
            ViewerRequestClaim::PromotedPrefetch => "promote",
        },
        viewer_trace_uri(&path),
        target_width,
        target_height,
    ));
    if matches!(claim, ViewerRequestClaim::New) {
        start_viewer_request(key, request.clone());
    }
    *decode_cancel.borrow_mut() = Some(lease.clone());

    let picture = picture.clone();
    let root = root.clone();
    let picture_viewport = picture_viewport.clone();
    let cache_path = path.clone();
    let photo = photo.clone();
    let display_texture_cache_for_result = display_texture_cache.clone();
    let navigation_ready_for_result = navigation_ready.clone();
    let navigation_settled_for_result = navigation_settled.clone();
    glib::MainContext::default().spawn_local(async move {
        let result = ViewerResultSlot::wait(request.result.clone()).await;

        let stale =
            !viewer_generation_current(generation.get(), expected_generation) || lease.cancelled();
        if stale {
            viewer_trace(format!(
                "display_discard lane=foreground uri={} reason=stale",
                viewer_trace_uri(&cache_path),
            ));
            lease.release();
            let _ = VIEWER_FOREGROUND_GENERATION.compare_exchange(
                expected_generation,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            if viewer_generation_current(generation.get(), expected_generation) {
                if let Some(navigation_ready) = navigation_ready_for_result {
                    navigation_ready.set(true);
                }
                if let Some(navigation_settled) = navigation_settled_for_result {
                    navigation_settled();
                }
            }
            return;
        }
        let _ = VIEWER_FOREGROUND_GENERATION.compare_exchange(
            expected_generation,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );

        match result {
            Ok(result) => {
                let display_started = std::time::Instant::now();
                let bytes = glib::Bytes::from_owned(result.pixels.clone());
                let texture = gtk::gdk::MemoryTexture::new(
                    result.width as i32,
                    result.height as i32,
                    gtk::gdk::MemoryFormat::R8g8b8a8,
                    &bytes,
                    result.width as usize * 4,
                );

                // Never overwrite PhotoObject's source dimensions with the
                // dimensions of a display-sized viewer decode. The original
                // dimensions are what 1:1 mode needs to request native pixels.
                if zoom.get() < 0.0 {
                    *native_texture.borrow_mut() = Some(NativeTextureCache {
                        path: cache_path.clone(),
                        rotation,
                        edit_recipe: edit_recipe_text.clone(),
                        texture: texture.clone(),
                    });
                }

                picture.set_paintable(Some(&texture));
                viewer_trace(format!(
                    "display_apply elapsed_ms={} uri={}",
                    display_started.elapsed().as_millis(),
                    viewer_trace_uri(&cache_path),
                ));
                viewer_trace(format!(
                    "display_done lane=foreground source=decode navigation_ms={} uri={}",
                    navigation_started.elapsed().as_millis(),
                    viewer_trace_uri(&cache_path),
                ));
                if zoom.get() >= 0.0 {
                    if !fit_geometry_fixed {
                        fit_picture(
                            &picture,
                            std::slice::from_ref(&photo),
                            0,
                            root.width(),
                            root.height(),
                            zoom.get(),
                            "navigate",
                        );
                    }
                    display_texture_cache_insert(
                        &display_texture_cache_for_result,
                        cache_path.clone(),
                        rotation,
                        edit_recipe_text.clone(),
                        target_width,
                        target_height,
                        texture.clone(),
                    );
                }
                if zoom.get() < 0.0 {
                    fit_centered_picture(
                        &picture,
                        &picture_viewport,
                        std::slice::from_ref(&photo),
                        0,
                        root.width(),
                        root.height(),
                        -1.0,
                        "one-to-one-decode",
                    );
                    picture.queue_resize();
                    picture_viewport.queue_resize();
                    trace_lightbox_after_paint(
                        &root,
                        &picture,
                        &picture_viewport,
                        "one_to_one_settled_decode",
                        None,
                    );
                }
            }
            Err(_) => {
                show_offline_preview(
                    &picture, photo, &root, zoom, generation,
                    expected_generation, offline, false,
                    navigation_ready_for_result, navigation_settled_for_result,
                );
                lease.release();
                return;
            }
        }
        if let Some(navigation_ready) = navigation_ready_for_result {
            navigation_ready.set(true);
        }
        if let Some(navigation_settled) = navigation_settled_for_result {
            navigation_settled();
        }
        lease.release();
    });
}

fn png_uses_theme_background(path: &str) -> bool {
    crate::image_format::for_path(path).is_some_and(|format| format.id == "png")
}

#[cfg(test)]
mod png_background_tests {
    use super::png_uses_theme_background;

    #[test]
    fn only_png_uses_the_theme_photo_background() {
        assert!(png_uses_theme_background("/photos/transparent.png"));
        assert!(png_uses_theme_background("/photos/transparent.PNG"));
        assert!(!png_uses_theme_background("/photos/opaque.jpg"));
        assert!(!png_uses_theme_background("/photos/camera.nef"));
    }
}

fn viewer_trace(message: impl std::fmt::Display) {
    if std::env::var_os("PICASA_TRACE").is_some() {
        static TRACE_STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let elapsed = TRACE_STARTED.get_or_init(std::time::Instant::now).elapsed();
        eprintln!(
            "PIC_VIEWER t_ms={} tid={:?} {message}",
            elapsed.as_millis(),
            std::thread::current().id()
        );
    }
}

fn zoom_trace(message: impl std::fmt::Display) {
    if std::env::var_os("PICASA_TRACE").is_some() {
        static TRACE_STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let elapsed = TRACE_STARTED.get_or_init(std::time::Instant::now).elapsed();
        eprintln!(
            "PIC_ZOOM t_ms={} tid={:?} {message}",
            elapsed.as_millis(),
            std::thread::current().id()
        );
    }
}

// Event callbacks can see the old allocation after an adjustment write. Pair
// those samples with after-paint samples to distinguish input math from the
// geometry GTK actually displayed. Only the first two pan updates are sampled.
fn trace_lightbox_geometry(
    root: &gtk::Overlay,
    picture: &gtk::Picture,
    scroll: &gtk::ScrolledWindow,
    stage: &str,
    pointer: Option<(f64, f64)>,
) {
    if std::env::var_os("PICASA_TRACE").is_none() {
        return;
    }
    let h = scroll.hadjustment();
    let v = scroll.vadjustment();
    let bounds = picture.compute_bounds(root);
    let viewport = scroll.child();
    let focus = root.root().and_then(|root| root.focus());
    let focus = focus.map(|widget| {
        format!(
            "{}:{} flags={:?}",
            widget.type_().name(),
            widget.widget_name(),
            widget.state_flags()
        )
    });
    zoom_trace(format!(
        "geometry stage={stage} frame={:?} pointer_viewport={pointer:?} h={:.2}/{:.2}/{:.2}/{:.2} v={:.2}/{:.2}/{:.2}/{:.2} picture_alloc={:?} picture_req={:?} align={:?}/{:?} expand={}/{} shrink={} fit={:?} intrinsic={:?} picture_root={bounds:?} picture_scroll={:?} origin_plus_adjustment={:?} viewport={:?} viewport_alloc={:?} viewport_root={:?} scroll_alloc={:?} scroll_root={:?} root_alloc={:?} focus={focus:?} picture_flags={:?} scroll_flags={:?} scroll_to_focus={:?}",
        root.frame_clock().map(|clock| clock.frame_counter()),
        h.value(), h.lower(), h.upper(), h.page_size(),
        v.value(), v.lower(), v.upper(), v.page_size(),
        picture.allocation(), picture.size_request(), picture.halign(), picture.valign(),
        picture.hexpands(), picture.vexpands(), picture.can_shrink(), picture.content_fit(),
        picture_intrinsic_dimensions(picture), picture.compute_bounds(scroll),
        picture.compute_bounds(scroll).map(|b| (f64::from(b.x()) + h.value(), f64::from(b.y()) + v.value())),
        viewport.as_ref().map(|widget| widget.type_().name()),
        viewport.as_ref().map(|widget| widget.allocation()),
        viewport.as_ref().and_then(|widget| widget.compute_bounds(root)),
        scroll.allocation(), scroll.compute_bounds(root), root.allocation(),
        picture.state_flags(), scroll.state_flags(),
        viewport.and_then(|widget| widget.downcast::<gtk::Viewport>().ok()).map(|viewport| viewport.is_scroll_to_focus()),
    ));
}

fn trace_lightbox_after_paint(
    root: &gtk::Overlay,
    picture: &gtk::Picture,
    scroll: &gtk::ScrolledWindow,
    stage: &'static str,
    pointer: Option<(f64, f64)>,
) {
    if std::env::var_os("PICASA_TRACE").is_none() {
        return;
    }
    let Some(clock) = root.frame_clock() else {
        return;
    };
    let root = root.downgrade();
    let picture = picture.downgrade();
    let scroll = scroll.downgrade();
    let handler = Rc::new(RefCell::new(None));
    let handler_for_callback = handler.clone();
    *handler.borrow_mut() = Some(clock.connect_after_paint(move |clock| {
        if let Some(id) = handler_for_callback.borrow_mut().take() {
            clock.disconnect(id);
        }
        if let (Some(root), Some(picture), Some(scroll)) =
            (root.upgrade(), picture.upgrade(), scroll.upgrade())
        {
            trace_lightbox_geometry(&root, &picture, &scroll, stage, pointer);
        }
    }));
    clock.request_phase(gtk::gdk::FrameClockPhase::AFTER_PAINT);
}

fn viewer_trace_uri(uri: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return uri.to_owned();
    };
    // Stored share URIs normally contain no authority credentials, but trace
    // output must remain safe if a URI was supplied with userinfo.
    let safe_rest = rest
        .rsplit_once('@')
        .map(|(_, value)| value)
        .unwrap_or(rest);
    format!("{scheme}://{safe_rest}")
}

fn viewer_generation_current(current: u64, expected: u64) -> bool {
    current == expected
}

fn start_viewer_request(key: ViewerRequestKey, request: Arc<ViewerRequest>) {
    let request_for_worker = request.clone();
    let worker_key = key.clone();
    if std::thread::Builder::new()
        .name("lightbox-decode".to_string())
        .spawn(move || {
            let queued = std::time::Instant::now();
            let gate = VIEWER_DECODE_GATE
                .get_or_init(|| DecodeSemaphore::new(MAX_CONCURRENT_VIEWER_DECODES));
            let Some(_permit) = gate.acquire_while(|| request_for_worker.has_consumers()) else {
                finish_viewer_request(
                    &worker_key,
                    &request_for_worker,
                    Err(Arc::from("cancelled at decode gate")),
                );
                return;
            };
            if !request_for_worker.has_consumers()
                || (VIEWER_FOREGROUND_GENERATION.load(Ordering::Acquire) != 0
                    && !request_for_worker.foreground.load(Ordering::Acquire))
            {
                finish_viewer_request(
                    &worker_key,
                    &request_for_worker,
                    Err(Arc::from("cancelled before decode")),
                );
                return;
            }
            viewer_trace(format!(
                "decode_start uri={} variant={}x{} queue_ms={}",
                viewer_trace_uri(&worker_key.path),
                worker_key.target_width,
                worker_key.target_height,
                queued.elapsed().as_millis(),
            ));
            let started = std::time::Instant::now();
            let recipe = crate::edit::EditRecipe::decode(&worker_key.edit_recipe);
            let lane_request = request_for_worker.clone();
            let cancelled_request = request_for_worker.clone();
            let read_context = crate::source::ViewerReadContext::new(
                move || {
                    if lane_request.foreground.load(Ordering::Acquire) {
                        crate::source::ViewerReadLane::Foreground
                    } else {
                        crate::source::ViewerReadLane::Prefetch
                    }
                },
                move || !cancelled_request.has_consumers(),
                Some(crate::source::ViewerSourceFingerprint {
                    mtime: worker_key.mtime,
                    size_bytes: worker_key.size_bytes,
                }),
            );
            let result = crate::thumbnail::decode_for_viewer_with_cancel(
                &worker_key.path,
                worker_key.target_width,
                worker_key.target_height,
                Some(&read_context),
                || !request_for_worker.has_consumers(),
            )
            .map(|image| {
                let image = crate::edit::render::apply_recipe(
                    rotate_image(image, worker_key.rotation),
                    &recipe,
                );
                Arc::new(ViewerDecodeResult {
                    width: image.width(),
                    height: image.height(),
                    pixels: image.into_raw(),
                })
            })
            .map_err(|error| Arc::from(error.to_string()));
            viewer_trace(format!(
                "decode_done uri={} variant={}x{} elapsed_ms={} outcome={}",
                viewer_trace_uri(&worker_key.path),
                worker_key.target_width,
                worker_key.target_height,
                started.elapsed().as_millis(),
                if result.is_ok() { "ok" } else { "error" },
            ));
            finish_viewer_request(&worker_key, &request_for_worker, result);
        })
        .is_err()
    {
        finish_viewer_request(
            &key,
            &request,
            Err(Arc::from("could not start lightbox decode thread")),
        );
    }
}

fn display_texture_cache_lookup(
    cache: &DisplayTextureCache,
    path: &str,
    rotation: i32,
    edit_recipe: &str,
    target_width: u32,
    target_height: u32,
) -> Option<gtk::gdk::MemoryTexture> {
    let mut cache = cache.borrow_mut();
    let position = cache.iter().position(|entry| {
        entry.path == path
            && entry.rotation == rotation
            && entry.edit_recipe == edit_recipe
            && entry.target_width == target_width
            && entry.target_height == target_height
    })?;
    let entry = cache.remove(position)?;
    let texture = entry.texture.clone();
    cache.push_front(entry);
    Some(texture)
}

fn display_texture_cache_insert(
    cache: &DisplayTextureCache,
    path: String,
    rotation: i32,
    edit_recipe: String,
    target_width: u32,
    target_height: u32,
    texture: gtk::gdk::MemoryTexture,
) {
    // RGBA8 footprint of the texture. Used by the byte budget so a large or
    // zoomed viewport cannot cache an unbounded amount of pixel memory.
    let bytes = (texture.width().max(0) as usize)
        .saturating_mul(texture.height().max(0) as usize)
        .saturating_mul(4);
    let mut cache = cache.borrow_mut();
    cache.retain(|entry| {
        !(entry.path == path
            && entry.rotation == rotation
            && entry.edit_recipe == edit_recipe
            && entry.target_width == target_width
            && entry.target_height == target_height)
    });
    cache.push_front(DisplayTextureCacheEntry {
        path,
        rotation,
        edit_recipe,
        target_width,
        target_height,
        texture,
        bytes,
    });
    // Evict least-recently-used entries past either the count or the byte
    // budget. Always keep at least one entry so a single oversized texture can
    // still be shown without the cache immediately dropping everything.
    let mut total: usize = cache.iter().map(|entry| entry.bytes).sum();
    while cache.len() > DISPLAY_TEXTURE_CACHE_CAPACITY
        || (total > DISPLAY_TEXTURE_CACHE_BYTE_BUDGET && cache.len() > 1)
    {
        let Some(removed) = cache.pop_back() else {
            break;
        };
        total = total.saturating_sub(removed.bytes);
        viewer_trace(format!(
            "cache_evict uri={} bytes={} remaining_entries={} remaining_bytes={}",
            viewer_trace_uri(&removed.path),
            removed.bytes,
            cache.len(),
            total,
        ));
    }
}

thread_local! {
    // Only one lightbox exists, so the pending prefetch timer and cancel token
    // live in thread-local state rather than on every navigation closure.
    static PREFETCH_SOURCE: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
    static PREFETCH_CANCEL: RefCell<Vec<Arc<ViewerRequestLease>>> = const { RefCell::new(Vec::new()) };
}

/// Cancel the pending prefetch timer and any in-flight prefetch decode.
/// Called on every navigation (so a stale prefetch never competes with the
/// photo the user actually moved to) and when the lightbox closes.
fn cancel_lightbox_prefetch() {
    cancel_lightbox_prefetch_except(None);
}

/// Cancel speculative leases except for the exact request that the foreground
/// is about to claim. Retaining that lease makes foreground promotion safe:
/// cancelling the old prefetch cannot discard the selected image's result.
fn cancel_lightbox_prefetch_except(keep: Option<&ViewerRequestKey>) {
    PREFETCH_SOURCE.with(|slot| {
        if let Some(source) = slot.borrow_mut().take() {
            source.remove();
        }
    });
    PREFETCH_CANCEL.with(|slot| {
        let mut leases = slot.borrow_mut();
        leases.retain(|lease| {
            let retain = keep.is_some_and(|key| {
                // The request itself is the canonical key; compare through
                // the registry's pointer only after the foreground claim.
                // Before that, an exact key match is represented by the
                // matching result request held by this lease.
                let requests = VIEWER_REQUESTS.get_or_init(|| Mutex::new(HashMap::new()));
                requests
                    .lock()
                    .unwrap()
                    .get(key)
                    .is_some_and(|request| Arc::ptr_eq(request, &lease.request))
            });
            if !retain {
                lease.cancel();
            }
            retain
        });
        if keep.is_none() {
            leases.clear();
        }
    });
}

/// After the user settles on a photo, warm both immediate neighbors in the
/// display-texture RAM cache, prioritizing the direction of movement. Delayed
/// so rapid stepping does not schedule a decode per keypress, and cancelled on
/// the next navigation or when the viewer closes.
fn schedule_lightbox_prefetch(
    photos: Rc<RefCell<Vec<PhotoObject>>>,
    current: usize,
    direction: i32,
    root: gtk::Overlay,
    zoom: Rc<Cell<f64>>,
    cache: DisplayTextureCache,
    generation: Rc<Cell<u64>>,
) {
    cancel_lightbox_prefetch();
    if direction == 0 || !root.is_visible() {
        return;
    }
    let expected_generation = generation.get();
    let source = glib::timeout_add_local(Duration::from_millis(250), move || {
        if !root.is_visible() || generation.get() != expected_generation {
            PREFETCH_SOURCE.with(|slot| {
                slot.borrow_mut().take();
            });
            return glib::ControlFlow::Break;
        }
        // Do not launch either neighbor until the requested photo has
        // completed its foreground decode. This timer is NOT a navigation
        // delay: show_photo() started the selected decode immediately.
        if VIEWER_FOREGROUND_GENERATION.load(Ordering::Acquire) != 0 {
            return glib::ControlFlow::Continue;
        }
        PREFETCH_SOURCE.with(|slot| {
            slot.borrow_mut().take();
        });
        let len = photos.borrow().len();
        let previous = current.checked_sub(1);
        let next = current.checked_add(1).filter(|&index| index < len);
        // Prefer the direction of travel, but warm *both* immediate neighbors.
        // No folder-wide prefetch: at most two speculative viewer decodes.
        let targets = if direction < 0 {
            [previous, next]
        } else {
            [next, previous]
        };
        for target in targets.into_iter().flatten() {
            prefetch_display_texture(&photos.borrow(), target, &root, zoom.get(), cache.clone());
        }
        glib::ControlFlow::Break
    });
    PREFETCH_SOURCE.with(|slot| {
        slot.borrow_mut().replace(source);
    });
}

/// Decode a neighbor photo into the display cache without touching the visible
/// picture. Uses the shared decode gate so a burst of prefetches cannot spawn
/// unbounded full-resolution RAW decodes, and bails out as soon as its cancel
/// token is set.
fn prefetch_display_texture(
    photos: &[PhotoObject],
    index: usize,
    root: &gtk::Overlay,
    zoom: f64,
    cache: DisplayTextureCache,
) {
    let Some(photo) = photos.get(index) else {
        return;
    };
    if !photo.original_available() {
        return;
    }
    if zoom < 0.0 {
        return;
    }
    let (target_width, target_height, _, _, _, _) =
        viewer_decode_target(root, photo.rotation(), false);
    let path = photo.path();
    if display_texture_cache_lookup(
        &cache,
        &path,
        photo.rotation(),
        &photo.edit_recipe(),
        target_width,
        target_height,
    )
    .is_some()
    {
        viewer_trace(format!(
            "cache_hit lane=prefetch uri={}",
            viewer_trace_uri(&path)
        ));
        return;
    }

    let rotation = photo.rotation();
    let edit_recipe_text = photo.edit_recipe();
    let key = ViewerRequestKey {
        path: path.clone(),
        mtime: photo.mtime(),
        size_bytes: photo.size_bytes(),
        rotation,
        edit_recipe: edit_recipe_text.clone(),
        target_width,
        target_height,
    };
    let (request, lease, claim) = claim_viewer_request(&key, false);
    viewer_trace(format!(
        "request lane=prefetch action={} uri={} variant={}x{}",
        match claim {
            ViewerRequestClaim::New => "new",
            ViewerRequestClaim::JoinedForeground => "join",
            ViewerRequestClaim::PromotedPrefetch => "promote",
        },
        viewer_trace_uri(&path),
        target_width,
        target_height,
    ));
    PREFETCH_CANCEL.with(|slot| slot.borrow_mut().push(lease.clone()));
    if matches!(claim, ViewerRequestClaim::New) {
        start_viewer_request(key, request.clone());
    }
    let cache_path = path.clone();
    let cache_for_result = cache.clone();

    glib::MainContext::default().spawn_local(async move {
        let result = ViewerResultSlot::wait(request.result.clone()).await;
        if lease.cancelled() {
            viewer_trace(format!(
                "display_discard lane=prefetch uri={} reason=cancelled",
                viewer_trace_uri(&cache_path),
            ));
            lease.release();
            return;
        }
        let Ok(result) = result else {
            lease.release();
            return;
        };
        let bytes = glib::Bytes::from_owned(result.pixels.clone());
        let texture = gtk::gdk::MemoryTexture::new(
            result.width as i32,
            result.height as i32,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            result.width as usize * 4,
        );
        display_texture_cache_insert(
            &cache_for_result,
            cache_path,
            rotation,
            edit_recipe_text,
            target_width,
            target_height,
            texture,
        );
        lease.release();
    });
}

fn prepare_navigation_photo(
    picture: &gtk::Picture,
    photo: Option<&PhotoObject>,
    root: &gtk::Overlay,
    zoom: f64,
    display_cache: &DisplayTextureCache,
) -> (bool, bool) {
    let Some(photo) = photo else {
        return (false, false);
    };

    if zoom < 0.0 {
        return (false, false);
    }

    let (target_width, target_height, _, _, _, _) =
        viewer_decode_target(root, photo.rotation(), false);
    let path = photo.path();
    if let Some(texture) = display_texture_cache_lookup(
        display_cache,
        &path,
        photo.rotation(),
        &photo.edit_recipe(),
        target_width,
        target_height,
    ) {
        set_fit_geometry_from_intrinsic(
            picture,
            photo,
            root,
            zoom,
            texture.width(),
            texture.height(),
        );
        picture.set_paintable(Some(&texture));

        return (true, true);
    }

    // No blurry grid thumbnail in the full-size viewer. On a RAM cache miss,
    // retain the previous full-resolution paintable until show_photo() has
    // decoded the requested image. This applies to JPEG and RAW alike.
    // The generation check in show_photo() prevents stale results appearing.
    (false, false)
}

fn set_fit_geometry_from_intrinsic(
    picture: &gtk::Picture,
    photo: &PhotoObject,
    root: &gtk::Overlay,
    zoom: f64,
    intrinsic_width: i32,
    intrinsic_height: i32,
) {
    let (native_width, native_height) = presentation_native_dimensions(photo);
    let (width, height) = fitted_picture_dimensions(
        native_width,
        native_height,
        intrinsic_width,
        intrinsic_height,
        root.width(),
        root.height(),
        zoom,
    );
    picture.set_size_request(width, height);
}

/// The dimensions the viewer scales from, and whether they came from the
/// catalog. Database dimensions are frequently described in a different
/// orientation than the decoded texture (EXIF rotation applied to the pixels
/// but not to the stored width/height), so the texture's aspect decides which
/// axis is which. Every scale computation in the viewer must agree on this or
/// the slider's 100% mark drifts away from the rendered size.
fn presentation_source_dimensions(
    native_width: i64,
    native_height: i64,
    intrinsic_width: i32,
    intrinsic_height: i32,
    one_to_one: bool,
) -> (f64, f64, bool) {
    let intrinsic_valid = intrinsic_width > 0 && intrinsic_height > 0;
    let native_valid = native_width > 0 && native_height > 0;

    let (mut source_width, mut source_height) = if one_to_one && intrinsic_valid {
        (f64::from(intrinsic_width), f64::from(intrinsic_height))
    } else if native_valid {
        (native_width as f64, native_height as f64)
    } else if intrinsic_valid {
        (f64::from(intrinsic_width), f64::from(intrinsic_height))
    } else {
        (1.0, 1.0)
    };

    if native_valid
        && intrinsic_valid
        && (source_width > source_height) != (intrinsic_width > intrinsic_height)
    {
        std::mem::swap(&mut source_width, &mut source_height);
    }

    (source_width, source_height, native_valid)
}

fn presentation_fit_scale_from_source(
    source_width: f64,
    source_height: f64,
    native_valid: bool,
    viewport_width: i32,
    viewport_height: i32,
) -> f64 {
    let available_width = (viewport_width - VIEWER_PADDING).max(1) as f64;
    let available_height = (viewport_height - VIEWER_PADDING).max(1) as f64;
    let fit_scale =
        (available_width / source_width.max(1.0)).min(available_height / source_height.max(1.0));
    // Known native dimensions remain the hard cap, so small source images are
    // never enlarged. Without metadata the texture is only a cached opening
    // preview and is allowed to fill the viewer while the real decode runs.
    if native_valid {
        fit_scale.min(1.0)
    } else {
        fit_scale
    }
    .max(f64::EPSILON)
}

fn presentation_fit_scale(
    photo: &PhotoObject,
    viewport_width: i32,
    viewport_height: i32,
    intrinsic_width: i32,
    intrinsic_height: i32,
) -> f64 {
    let (native_width, native_height) = presentation_native_dimensions(photo);
    let (source_width, source_height, native_valid) = presentation_source_dimensions(
        native_width,
        native_height,
        intrinsic_width,
        intrinsic_height,
        false,
    );
    presentation_fit_scale_from_source(
        source_width,
        source_height,
        native_valid,
        viewport_width,
        viewport_height,
    )
}

fn presentation_native_dimensions_from_values(
    raw_width: i64,
    raw_height: i64,
    rotation: i32,
    edit_recipe: &str,
) -> (i64, i64) {
    // Preserve "unknown" catalogue dimensions as unknown. The previous
    // max(1) coercion turned a missing 0x0 metadata record into a seemingly
    // valid 1x1 native image. presentation_source_dimensions() would then
    // prefer that bogus 1x1 value over the decoded paintable (for example
    // 4000x3000), collapsing slider geometry to 1x1/2x2 and making the
    // Lightbox jump while zooming.
    if raw_width <= 1 || raw_height <= 1 {
        return (0, 0);
    }

    let (mut width, mut height) = (raw_width as u32, raw_height as u32);
    if matches!(rotation.rem_euclid(360), 90 | 270) {
        std::mem::swap(&mut width, &mut height);
    }
    let recipe = crate::edit::EditRecipe::decode(edit_recipe);
    let (width, height) = crate::edit::render::estimated_output_dimensions(width, height, &recipe);
    (i64::from(width), i64::from(height))
}

fn presentation_native_dimensions(photo: &PhotoObject) -> (i64, i64) {
    presentation_native_dimensions_from_values(
        photo.width(),
        photo.height(),
        photo.rotation(),
        &photo.edit_recipe(),
    )
}

#[cfg(test)]
mod presentation_dimension_tests {
    use super::{
        presentation_native_dimensions_from_values, presentation_source_dimensions,
    };

    #[test]
    fn missing_catalog_dimensions_fall_back_to_decoded_texture() {
        let (native_width, native_height) =
            presentation_native_dimensions_from_values(0, 0, 0, "");
        assert_eq!((native_width, native_height), (0, 0));

        let (source_width, source_height, native_valid) =
            presentation_source_dimensions(native_width, native_height, 4000, 3000, false);
        assert!(!native_valid);
        assert_eq!((source_width, source_height), (4000.0, 3000.0));
    }

    #[test]
    fn one_by_one_sentinel_is_not_treated_as_real_photo_size() {
        let (native_width, native_height) =
            presentation_native_dimensions_from_values(1, 1, 0, "");
        assert_eq!((native_width, native_height), (0, 0));
    }
}

fn reset_viewport(viewport: &gtk::ScrolledWindow) {
    let horizontal = viewport.hadjustment();
    let vertical = viewport.vadjustment();
    horizontal.set_value(horizontal.lower());
    vertical.set_value(vertical.lower());
}

/// Scroll position that centres the child in the viewport after its scroll
/// range changed.
///
/// `GtkViewport` publishes its new range before it allocates the child at
/// `-value`, so writing the corrected value from the range-change handler
/// lands in the same layout pass: the child is placed once, at the right
/// offset, and the frame that shows the new geometry already matches it. No
/// deferred correction, so there is nothing to overshoot and settle.
///
/// The value is derived from the new range alone. Carrying the previous
/// centre-relative offset across steps was tried and is wrong for a zoom
/// control: a pan inherited from an earlier gesture persists for the whole
/// drag, and because the offset is only ever re-expressed in whole pixels it
/// unwinds at a fraction of a pixel per step rather than being corrected, so
/// the photo stays visibly off centre long after the pan. Zooming is expected
/// to re-centre; dragging the image afterwards re-pans it.
///
/// With no carried state there is nothing to accumulate, so the only error
/// left is the snap of the scroll origin to an integer logical pixel, at most
/// half a pixel, at any magnification.
fn centered_scroll_value(upper: f64, page_size: f64, lower: f64) -> f64 {
    let max_scroll = upper - page_size;
    if max_scroll <= lower {
        return lower;
    }
    // Ties are broken to even. An odd overflow puts the exact centre on x.5,
    // and rounding halves away from zero would bias every step the same way.
    (upper * 0.5 - page_size * 0.5)
        .round_ties_even()
        .clamp(lower, max_scroll)
}

/// Install explicit picture geometry and its matching centered scroll ranges before
/// GTK allocates the GtkViewport child. The viewport configures adjustments
/// during allocation, but its `changed` handlers run after it places the
/// child; centering only from that handler leaves stale child bounds until a
/// later adjustment write (such as the first drag) queues another allocation.
fn fit_centered_picture(
    picture: &gtk::Picture,
    viewport: &gtk::ScrolledWindow,
    photos: &[PhotoObject],
    index: usize,
    viewport_width: i32,
    viewport_height: i32,
    zoom: f64,
    source: &str,
) {
    fit_picture(
        picture,
        photos,
        index,
        viewport_width,
        viewport_height,
        zoom,
        source,
    );
    let (picture_width, picture_height) = picture.size_request();
    prime_viewport_for_picture_size(viewport, picture_width, picture_height);
}

/// Wire one viewport axis to the centring rule above. GtkViewport can emit
/// `changed` after it places its child during allocation, so this handler
/// centres range changes only after that placement. Geometry-writing paths
/// prime the new range before their queued allocation instead.
fn prime_viewport_for_picture_size(
    viewport: &gtk::ScrolledWindow,
    picture_width: i32,
    picture_height: i32,
) {
    fn prime_axis(adjustment: &gtk::Adjustment, content_size: i32) {
        let page_size = adjustment.page_size();
        if page_size <= 0.0 {
            return;
        }

        let lower = adjustment.lower();
        let upper = f64::from(content_size.max(1)).max(page_size + lower);
        let value = centered_scroll_value(upper, page_size, lower);

        let range_changed = (adjustment.upper() - upper).abs() > 0.5
            || (adjustment.page_size() - page_size).abs() > f64::EPSILON;
        if range_changed {
            adjustment.configure(
                value,
                lower,
                upper,
                adjustment.step_increment(),
                adjustment.page_increment(),
                page_size,
            );
        } else if (adjustment.value() - value).abs() > 0.5 {
            adjustment.set_value(value);
        }
    }

    prime_axis(&viewport.hadjustment(), picture_width);
    prime_axis(&viewport.vadjustment(), picture_height);
}

fn install_viewport_anchor(adjustment: &gtk::Adjustment) {
    adjustment.connect_changed(move |adjustment| {
        zoom_trace(format!(
            "adjustment_changed before value={:.1} lower={:.1} upper={:.1} page={:.1} step={:.1} page_inc={:.1}",
            adjustment.value(),
            adjustment.lower(),
            adjustment.upper(),
            adjustment.page_size(),
            adjustment.step_increment(),
            adjustment.page_increment(),
        ));
        let value = centered_scroll_value(
            adjustment.upper(),
            adjustment.page_size(),
            adjustment.lower(),
        );
        if (adjustment.value() - value).abs() > f64::EPSILON {
            adjustment.set_value(value);
        }
        zoom_trace(format!(
            "viewport_anchor value={:.1} upper={:.1} page={:.1} target={:.1}",
            adjustment.value(),
            adjustment.upper(),
            adjustment.page_size(),
            value
        ));
        // GtkViewport measures, updates the range, then allocates the child at
        // -value in the same pass, so the child is only ever placed once.
    });
}

fn picture_intrinsic_dimensions(picture: &gtk::Picture) -> (i32, i32) {
    let Some(paintable) = picture.paintable() else {
        return (0, 0);
    };
    (paintable.intrinsic_width(), paintable.intrinsic_height())
}

fn viewer_decode_target(
    root: &gtk::Overlay,
    rotation: i32,
    one_to_one: bool,
) -> (u32, u32, i32, i32, i32, bool) {
    let allocated_width = root.width() - VIEWER_PADDING;
    let allocated_height = root.height() - VIEWER_PADDING;
    let fallback = allocated_width <= 0 || allocated_height <= 0;
    let (logical_width, logical_height) = if fallback {
        // A hidden overlay normally retains its window allocation. This only
        // applies on the very first frame before GTK has allocated the window.
        (1024, 768)
    } else {
        (allocated_width, allocated_height)
    };
    let scale_factor = root.scale_factor().max(1);
    // RAW catalog dimensions are frequently absent and can describe the
    // sensor rather than its embedded display preview. Use an unbounded 1:1
    // request and let decode_for_viewer clamp it to the actual source it
    // discovers. Falling back to the viewport made 1:1 indistinguishable
    // from fit-to-window for DNG and many NEF files.
    let mut target_width = if one_to_one {
        u32::MAX
    } else {
        (logical_width as u32).saturating_mul(scale_factor as u32)
    };
    let mut target_height = if one_to_one {
        u32::MAX
    } else {
        (logical_height as u32).saturating_mul(scale_factor as u32)
    };
    // decode_for_viewer applies EXIF orientation; user rotation happens in
    // this module afterward, so swap its input bounds for a quarter-turn.
    if matches!(rotation.rem_euclid(360), 90 | 270) {
        std::mem::swap(&mut target_width, &mut target_height);
    }
    (
        target_width.max(1),
        target_height.max(1),
        logical_width,
        logical_height,
        scale_factor,
        fallback,
    )
}

fn rotate_image(image: image::RgbaImage, rotation: i32) -> image::RgbaImage {
    match rotation.rem_euclid(360) {
        90 => image::imageops::rotate90(&image),
        180 => image::imageops::rotate180(&image),
        270 => image::imageops::rotate270(&image),
        _ => image,
    }
}

fn fit_picture(
    picture: &gtk::Picture,
    photos: &[PhotoObject],
    index: usize,
    viewport_width: i32,
    viewport_height: i32,
    zoom: f64,
    source: &str,
) {
    let Some(photo) = photos.get(index) else {
        return;
    };

    if viewport_width <= 0 || viewport_height <= 0 {
        return;
    }

    let paintable = picture.paintable();
    let intrinsic_width = paintable
        .as_ref()
        .map(gtk::gdk::Paintable::intrinsic_width)
        .unwrap_or(0);
    let intrinsic_height = paintable
        .as_ref()
        .map(gtk::gdk::Paintable::intrinsic_height)
        .unwrap_or(0);
    // Fit-to-window does not need explicit pixel geometry. Let GtkPicture fill
    // the viewport and let ContentFit::Contain do the presentation scaling.
    // During a live window resize this avoids issuing a new size request for
    // every single pixel of motion, which otherwise makes the texture
    // repeatedly re-rasterise and visibly flicker.
    if zoom == 0.0 {
        zoom_trace(format!(
            "fit_mode source={source} viewport={}x{} picture_req={:?} alloc={}x{} intrinsic={}x{}",
            viewport_width,
            viewport_height,
            picture.size_request(),
            picture.width(),
            picture.height(),
            intrinsic_width,
            intrinsic_height,
        ));
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.set_halign(gtk::Align::Fill);
        picture.set_valign(gtk::Align::Fill);
        if picture.size_request() != (1, 1) {
            picture.set_size_request(1, 1);
        }
        return;
    }

    // Explicit zoom/1:1 presentation may overflow the viewport. Do not
    // centre an overflowing GtkPicture with widget alignment *and* centre it
    // again with the ScrolledWindow adjustments: those are two competing
    // origins and rapid slider updates make the child visibly oscillate.
    //
    // Axes that still fit may use normal GTK centring. Overflowing axes are
    // anchored at Start and the adjustment is the single owner of centring.
    picture.set_hexpand(false);
    picture.set_vexpand(false);

    let (native_width, native_height) = presentation_native_dimensions(photo);
    let (fitted_width, fitted_height) = fitted_picture_dimensions(
        native_width,
        native_height,
        intrinsic_width,
        intrinsic_height,
        viewport_width,
        viewport_height,
        zoom,
    );
    picture.set_halign(if fitted_width > viewport_width {
        gtk::Align::Start
    } else {
        gtk::Align::Center
    });
    picture.set_valign(if fitted_height > viewport_height {
        gtk::Align::Start
    } else {
        gtk::Align::Center
    });

    // A zoom control sitting at one of its stops still reports a value on every
    // tick, and GTK re-queues a layout for any set_size_request even when the
    // request is unchanged. Re-laying out the same geometry makes the picture
    // re-rasterise at the device scale, which flickers for as long as the
    // control is held still, so an already-correct size is left alone.
    let already_applied = picture.size_request() == (fitted_width, fitted_height);

    if std::env::var_os("PICASA_TRACE").is_some() {
        zoom_trace(format!(
            "fit_picture source={source} index={} zoom={zoom:.5} viewport={}x{} intrinsic={}x{} native={}x{} target={}x{} current_alloc={}x{} applied={}",
            index,
            viewport_width,
            viewport_height,
            intrinsic_width,
            intrinsic_height,
            native_width,
            native_height,
            fitted_width,
            fitted_height,
            picture.width(),
            picture.height(),
            !already_applied,
        ));
    }
    if !already_applied {
        zoom_trace(format!(
            "size_request_write source={source} from={:?} to={}x{} alloc={}x{}",
            picture.size_request(),
            fitted_width,
            fitted_height,
            picture.width(),
            picture.height(),
        ));
        picture.set_size_request(fitted_width, fitted_height);
    } else {
        zoom_trace(format!(
            "size_request_skip source={source} request={:?} alloc={}x{}",
            picture.size_request(),
            picture.width(),
            picture.height(),
        ));
    }
}

fn fitted_picture_dimensions(
    native_width: i64,
    native_height: i64,
    intrinsic_width: i32,
    intrinsic_height: i32,
    viewport_width: i32,
    viewport_height: i32,
    zoom: f64,
) -> (i32, i32) {
    let (source_width, source_height, native_valid) = presentation_source_dimensions(
        native_width,
        native_height,
        intrinsic_width,
        intrinsic_height,
        zoom < 0.0,
    );
    let fit_scale = presentation_fit_scale_from_source(
        source_width,
        source_height,
        native_valid,
        viewport_width,
        viewport_height,
    );
    // At 1:1 the decoded texture is the limit, so its pixels are presented
    // unscaled; a RAW embedded preview may differ from the catalog's sensor
    // dimensions. Fit and every slider step are relative to the same fitted
    // size, which is what keeps `zoom` a stable fraction of the real range.
    let scale = if zoom < 0.0 {
        1.0
    } else if zoom == 0.0 {
        fit_scale
    } else {
        fit_scale * zoom
    };

    (
        (source_width * scale).round().max(1.0) as i32,
        (source_height * scale).round().max(1.0) as i32,
    )
}
