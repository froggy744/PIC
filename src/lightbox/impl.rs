impl Lightbox {
    pub fn new() -> Self {
        let root = gtk::Overlay::new();
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.set_halign(gtk::Align::Fill);
        root.set_valign(gtk::Align::Fill);
        root.set_focusable(true);
        root.set_can_target(true);
        root.set_visible(false);

        let backdrop = gtk::Box::new(gtk::Orientation::Vertical, 0);
        backdrop.set_hexpand(true);
        backdrop.set_vexpand(true);
        backdrop.set_can_target(true);
        backdrop.add_css_class("lightbox-backdrop");
        root.set_child(Some(&backdrop));

        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_hexpand(false);
        picture.set_vexpand(false);
        picture.set_halign(gtk::Align::Center);
        picture.set_valign(gtk::Align::Center);
        picture.set_can_target(true);
        picture.set_focusable(false);
        picture.set_focus_on_click(false);
        picture.set_overflow(gtk::Overflow::Visible);
        picture.set_widget_name("lightbox-zoom-picture");
        picture.add_css_class("lightbox-picture");

        // Keep the image in a viewport so native-size presentation can be
        // larger than the window and remain pannable instead of being forced
        // back into the overlay's allocation.
        let picture_viewport = gtk::ScrolledWindow::new();
        picture_viewport.set_hexpand(true);
        picture_viewport.set_vexpand(true);
        picture_viewport.set_halign(gtk::Align::Fill);
        picture_viewport.set_valign(gtk::Align::Fill);
        picture_viewport.set_can_target(true);
        picture_viewport.set_focusable(false);
        picture_viewport.set_focus_on_click(false);
        // Keep scrollbars hidden while keeping the viewport constrained to
        // the lightbox allocation. External gives us real scroll ranges
        // without drawing normal scrollbar UI.
        picture_viewport.set_policy(gtk::PolicyType::External, gtk::PolicyType::External);
        picture_viewport.set_child(Some(&picture));
        // GtkScrolledWindow wraps non-scrollable children such as GtkPicture
        // in an implicit GtkViewport. Its default focus tracking may scroll a
        // large child when a mouse click changes focus, which appears as a
        // jump before panning starts. The lightbox owns focus at the root, so
        // the image viewport must never reposition itself for focus.
        if let Some(viewport) = picture_viewport
            .child()
            .and_then(|child| child.downcast::<gtk::Viewport>().ok())
        {
            viewport.set_scroll_to_focus(false);
            viewport.set_focusable(false);
            viewport.set_focus_on_click(false);
        }
        root.add_overlay(&picture_viewport);

        let one_to_one_active = Rc::new(Cell::new(false));
        let native_texture: Rc<RefCell<Option<NativeTextureCache>>> = Rc::new(RefCell::new(None));
        let native_quality_pending = Rc::new(Cell::new(false));
        let display_texture_cache: DisplayTextureCache = Rc::new(RefCell::new(VecDeque::new()));

        // Native-size/manual-zoom panning.
        //
        // GestureDrag may not become active on the exact button-down frame.
        // Keep a separate primary-button press tracker so the image follows
        // the pointer during those first few pixels instead of catching up
        // when GTK finally recognizes the drag.
        const PAN_EARLY_MOTION_EPSILON: f64 = 1.0;
        let drag_start_h = Rc::new(Cell::new(0.0));
        let drag_start_v = Rc::new(Cell::new(0.0));
        let drag_press_x = Rc::new(Cell::new(0.0));
        let drag_press_y = Rc::new(Cell::new(0.0));
        let drag_range_h = Rc::new(Cell::new((0.0, 0.0, 0.0)));
        let drag_range_v = Rc::new(Cell::new((0.0, 0.0, 0.0)));
        let pan_pressed = Rc::new(Cell::new(false));
        let pan_active = Rc::new(Cell::new(false));

        // True mouse-down anchor. This runs immediately on press, before a
        // GestureDrag needs to decide whether the sequence is actually a drag.
        let pan_press = gtk::GestureClick::new();
        pan_press.set_button(1);
        pan_press.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let viewport = picture_viewport.clone();
            let drag_start_h = drag_start_h.clone();
            let drag_start_v = drag_start_v.clone();
            let drag_press_x = drag_press_x.clone();
            let drag_press_y = drag_press_y.clone();
            let drag_range_h = drag_range_h.clone();
            let drag_range_v = drag_range_v.clone();
            let pan_pressed = pan_pressed.clone();
            let pan_active = pan_active.clone();
            pan_press.connect_pressed(move |_, _, x, y| {
                let hadj = viewport.hadjustment();
                let vadj = viewport.vadjustment();
                let scrollable =
                    hadj.upper() - hadj.page_size() > 1.0 || vadj.upper() - vadj.page_size() > 1.0;

                pan_pressed.set(scrollable);
                pan_active.set(false);
                if !scrollable {
                    return;
                }

                drag_start_h.set(hadj.value());
                drag_start_v.set(vadj.value());
                drag_press_x.set(x);
                drag_press_y.set(y);
                drag_range_h.set((hadj.lower(), hadj.upper(), hadj.page_size()));
                drag_range_v.set((vadj.lower(), vadj.upper(), vadj.page_size()));
                viewport.set_cursor_from_name(Some("grab"));

                zoom_trace(format!(
                    "pan_mouse_down x={x:.1} y={y:.1} origin={:.1},{:.1}",
                    hadj.value(),
                    vadj.value(),
                ));
            });
        }
        {
            let viewport = picture_viewport.clone();
            let pan_pressed = pan_pressed.clone();
            let pan_active = pan_active.clone();
            pan_press.connect_released(move |_, _, _, _| {
                pan_pressed.set(false);
                if !pan_active.get() {
                    let hadj = viewport.hadjustment();
                    let vadj = viewport.vadjustment();
                    let scrollable =
                        hadj.upper() - hadj.page_size() > 1.0 || vadj.upper() - vadj.page_size() > 1.0;
                    viewport.set_cursor_from_name(if scrollable {
                        Some("grab")
                    } else {
                        None
                    });
                }
            });
        }
        picture_viewport.add_controller(pan_press);

        // Before GestureDrag recognition, follow ordinary pointer motion from
        // the exact mouse-down coordinates. Once the drag gesture activates it
        // takes over with the same origin, so there is no first-frame jump.
        let early_motion = gtk::EventControllerMotion::new();
        early_motion.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let viewport = picture_viewport.clone();
            let drag_start_h = drag_start_h.clone();
            let drag_start_v = drag_start_v.clone();
            let drag_press_x = drag_press_x.clone();
            let drag_press_y = drag_press_y.clone();
            let drag_range_h = drag_range_h.clone();
            let drag_range_v = drag_range_v.clone();
            let pan_pressed = pan_pressed.clone();
            let pan_active = pan_active.clone();
            early_motion.connect_motion(move |_, x, y| {
                if !pan_pressed.get() || pan_active.get() {
                    return;
                }

                let hadj = viewport.hadjustment();
                let vadj = viewport.vadjustment();
                let h_range = (hadj.lower(), hadj.upper(), hadj.page_size());
                let v_range = (vadj.lower(), vadj.upper(), vadj.page_size());
                let old_h = drag_range_h.get();
                let old_v = drag_range_v.get();
                let range_changed = (h_range.0 - old_h.0).abs() > 0.5
                    || (h_range.1 - old_h.1).abs() > 0.5
                    || (h_range.2 - old_h.2).abs() > 0.5
                    || (v_range.0 - old_v.0).abs() > 0.5
                    || (v_range.1 - old_v.1).abs() > 0.5
                    || (v_range.2 - old_v.2).abs() > 0.5;

                if range_changed {
                    drag_start_h.set(hadj.value());
                    drag_start_v.set(vadj.value());
                    drag_press_x.set(x);
                    drag_press_y.set(y);
                    drag_range_h.set(h_range);
                    drag_range_v.set(v_range);
                    return;
                }

                let dx = x - drag_press_x.get();
                let dy = y - drag_press_y.get();
                if dx.hypot(dy) < PAN_EARLY_MOTION_EPSILON {
                    return;
                }

                let max_h = (hadj.upper() - hadj.page_size()).max(hadj.lower());
                let max_v = (vadj.upper() - vadj.page_size()).max(vadj.lower());
                hadj.set_value((drag_start_h.get() - dx).clamp(hadj.lower(), max_h));
                vadj.set_value((drag_start_v.get() - dy).clamp(vadj.lower(), max_v));
            });
        }
        picture_viewport.add_controller(early_motion);

        let pan_drag = gtk::GestureDrag::new();
        pan_drag.set_button(1);
        pan_drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        pan_drag.set_exclusive(true);

        {
            let viewport = picture_viewport.clone();
            let pan_pressed = pan_pressed.clone();
            let pan_active = pan_active.clone();
            let drag_start_h = drag_start_h.clone();
            let drag_start_v = drag_start_v.clone();
            let drag_press_x = drag_press_x.clone();
            let drag_press_y = drag_press_y.clone();
            let drag_range_h = drag_range_h.clone();
            let drag_range_v = drag_range_v.clone();
            pan_drag.connect_drag_begin(move |gesture, x, y| {
                let hadj = viewport.hadjustment();
                let vadj = viewport.vadjustment();
                let scrollable =
                    hadj.upper() - hadj.page_size() > 1.0 || vadj.upper() - vadj.page_size() > 1.0;
                if !scrollable {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    pan_pressed.set(false);
                    pan_active.set(false);
                    return;
                }

                // Normally the GestureClick above already captured the exact
                // press. Keep this fallback for synthetic/non-mouse sequences.
                if !pan_pressed.get() {
                    pan_pressed.set(true);
                    drag_start_h.set(hadj.value());
                    drag_start_v.set(vadj.value());
                    drag_press_x.set(x);
                    drag_press_y.set(y);
                    drag_range_h.set((hadj.lower(), hadj.upper(), hadj.page_size()));
                    drag_range_v.set((vadj.lower(), vadj.upper(), vadj.page_size()));
                }

                pan_active.set(true);
                viewport.set_cursor_from_name(Some("grabbing"));
                zoom_trace(format!(
                    "pan_drag_begin x={x:.1} y={y:.1} origin={:.1},{:.1}",
                    drag_start_h.get(),
                    drag_start_v.get(),
                ));
            });
        }

        {
            let viewport = picture_viewport.clone();
            let drag_start_h = drag_start_h.clone();
            let drag_start_v = drag_start_v.clone();
            let drag_range_h = drag_range_h.clone();
            let drag_range_v = drag_range_v.clone();
            pan_drag.connect_drag_update(move |_, offset_x, offset_y| {
                let hadj = viewport.hadjustment();
                let vadj = viewport.vadjustment();
                let h_range = (hadj.lower(), hadj.upper(), hadj.page_size());
                let v_range = (vadj.lower(), vadj.upper(), vadj.page_size());
                let old_h = drag_range_h.get();
                let old_v = drag_range_v.get();

                let range_changed = (h_range.0 - old_h.0).abs() > 0.5
                    || (h_range.1 - old_h.1).abs() > 0.5
                    || (h_range.2 - old_h.2).abs() > 0.5
                    || (v_range.0 - old_v.0).abs() > 0.5
                    || (v_range.1 - old_v.1).abs() > 0.5
                    || (v_range.2 - old_v.2).abs() > 0.5;
                if range_changed {
                    // Geometry changed mid-drag (native texture/window resize).
                    // Restart from the newly authoritative visible position.
                    drag_start_h.set(hadj.value() + offset_x);
                    drag_start_v.set(vadj.value() + offset_y);
                    drag_range_h.set(h_range);
                    drag_range_v.set(v_range);
                }

                let max_h = (hadj.upper() - hadj.page_size()).max(hadj.lower());
                let max_v = (vadj.upper() - vadj.page_size()).max(vadj.lower());
                hadj.set_value((drag_start_h.get() - offset_x).clamp(hadj.lower(), max_h));
                vadj.set_value((drag_start_v.get() - offset_y).clamp(vadj.lower(), max_v));
            });
        }

        {
            let viewport = picture_viewport.clone();
            let pan_pressed = pan_pressed.clone();
            let pan_active = pan_active.clone();
            pan_drag.connect_drag_end(move |_, offset_x, offset_y| {
                pan_pressed.set(false);
                pan_active.set(false);
                let hadj = viewport.hadjustment();
                let vadj = viewport.vadjustment();
                let scrollable =
                    hadj.upper() - hadj.page_size() > 1.0 || vadj.upper() - vadj.page_size() > 1.0;
                viewport.set_cursor_from_name(if scrollable {
                    Some("grab")
                } else {
                    None
                });
                zoom_trace(format!(
                    "pan_end offset={offset_x:.1},{offset_y:.1} value={:.1},{:.1}",
                    hadj.value(),
                    vadj.value(),
                ));
            });
        }
        let photos = Rc::new(RefCell::new(Vec::<PhotoObject>::new()));
        let index = Rc::new(Cell::new(0usize));
        let last_width = Rc::new(Cell::new(0i32));
        let last_height = Rc::new(Cell::new(0i32));
        let opening_fit_pending = Rc::new(Cell::new(false));
        let zoom = Rc::new(Cell::new(0.0)); // 0 means fit-to-window
        let zoom_before_one_to_one = Rc::new(Cell::new(0.0));
        let applied_native_scale = Rc::new(Cell::new(APPLIED_SCALE_UNKNOWN));
        let load_generation = Rc::new(Cell::new(0u64));
        let decode_cancel: Rc<RefCell<Option<Arc<ViewerRequestLease>>>> =
            Rc::new(RefCell::new(None));
        let key_navigation_ready = Rc::new(Cell::new(true));
        let wheel_navigation = Rc::new(RefCell::new(WheelNavigationState::default()));
        let photo_changed: PhotoChangedHandler = Rc::new(RefCell::new(None));
        let one_to_one_sync: OneToOneSyncHandler = Rc::new(RefCell::new(None));
        let zoom_sync: ZoomSyncHandler = Rc::new(RefCell::new(None));
        let context_menu: ContextMenuHandler = Rc::new(RefCell::new(None));
        let collection_navigation: CollectionNavigationHandler = Rc::new(RefCell::new(None));

        let double_click = gtk::GestureClick::new();
        double_click.set_button(1);
        let root_for_double = root.clone();
        let generation_for_double = load_generation.clone();
        let cancel_for_double = decode_cancel.clone();
        double_click.connect_pressed(move |gesture, n_press, _, _| {
            if n_press == 2 {
                generation_for_double.set(generation_for_double.get().wrapping_add(1));
                if let Some(active) = cancel_for_double.borrow_mut().take() {
                    active.cancel();
                }
                root_for_double.set_visible(false);
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        // Double-click stays on the picture. The pan gesture is attached to
        // the stationary viewport so its coordinates do not move while the
        // ScrolledWindow adjustments pan the image.
        picture.add_controller(double_click);
        picture_viewport.add_controller(pan_drag);

        // Keep the pan cursor in sync with viewport overflow: zoomed in via
        // Ctrl+wheel and 1:1 both show the grab cursor, fit shows none.
        {
            let viewport_for_cursor = picture_viewport.clone();
            let update_cursor: Rc<dyn Fn()> = Rc::new(move || {
                let hadj = viewport_for_cursor.hadjustment();
                let vadj = viewport_for_cursor.vadjustment();
                let scrollable =
                    hadj.upper() - hadj.page_size() > 1.0 || vadj.upper() - vadj.page_size() > 1.0;
                viewport_for_cursor.set_cursor_from_name(if scrollable {
                    Some("grab")
                } else {
                    None
                });
            });
            let update_for_v = update_cursor.clone();
            picture_viewport
                .vadjustment()
                .connect_changed(move |_| update_for_v());
            let update_for_h = update_cursor.clone();
            picture_viewport
                .hadjustment()
                .connect_changed(move |_| update_for_h());
        }

        // Every zoom path writes real GtkPicture geometry, and GTK publishes
        // the resulting scroll range before it places the child. Keeping the
        // point under the viewport centre anchored on that range change is the
        // single recentering rule: slider, Ctrl+wheel, keys, 1:1 and window
        // resizes all keep their centre without timers or deferred fixes.
        install_viewport_anchor(&picture_viewport.hadjustment());
        install_viewport_anchor(&picture_viewport.vadjustment());

        // Some close paths intentionally hide the overlay directly (outside
        // click and double-click). Reset the internal presentation state for
        // those paths as well as for Lightbox::close().
        let one_to_one_for_visibility = one_to_one_active.clone();
        let zoom_for_visibility = zoom.clone();
        let applied_for_visibility = applied_native_scale.clone();
        let picture_for_visibility = picture.clone();
        let viewport_for_visibility = picture_viewport.clone();
        root.connect_visible_notify(move |root| {
            if !root.is_visible() {
                let mut child = root.first_child();
                while let Some(widget) = child {
                    child = widget.next_sibling();
                    if widget.has_css_class("photo-context-menu") {
                        widget.set_visible(false);
                        widget.unparent();
                    }
                }
                one_to_one_for_visibility.set(false);
                set_zoom_state(&zoom_for_visibility, &applied_for_visibility, 0.0);
                picture_for_visibility.set_can_shrink(true);
                viewport_for_visibility.set_cursor_from_name(None);
                reset_viewport(&viewport_for_visibility);
            }
        });

        let outside_click = gtk::GestureClick::new();
        outside_click.set_button(1);
        outside_click.set_propagation_phase(gtk::PropagationPhase::Capture);

        let root_for_outside = root.clone();
        let picture_for_outside = picture.clone();

        outside_click.connect_pressed(move |gesture, n_press, x, y| {
            if n_press > 1 {
                return;
            }

            let inside_picture = picture_for_outside
                .compute_bounds(&root_for_outside)
                .map(|bounds| {
                    x >= bounds.x() as f64
                        && y >= bounds.y() as f64
                        && x < (bounds.x() + bounds.width()) as f64
                        && y < (bounds.y() + bounds.height()) as f64
                })
                .unwrap_or(false);

            // The photo context menu is a normal overlay child of the root.
            // Treat clicks on it as inside the lightbox so the capture-phase
            // backdrop handler does not close the viewer before its buttons
            // receive the click.
            let inside_context_menu = root_for_outside
                .pick(x, y, gtk::PickFlags::DEFAULT)
                .is_some_and(|picked| {
                    let mut current = Some(picked);
                    while let Some(widget) = current {
                        if widget.has_css_class("photo-context-menu") {
                            return true;
                        }
                        current = widget.parent();
                    }
                    false
                });

            if !inside_picture && !inside_context_menu {
                root_for_outside.set_visible(false);
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        root.add_controller(outside_click);

        // Capture secondary clicks on the actual full-size event surface.
        // The ScrolledWindow fills the lightbox and remains under the pointer
        // over the image as well as the centre/bottom-centre area. Using the
        // same widget for both the gesture coordinates and popover anchor
        // avoids the dead zones caused by root/overlay coordinate mismatch.
        let right_click = gtk::GestureClick::new();
        right_click.set_button(3);
        right_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let photos_for_context = photos.clone();
        let index_for_context = index.clone();
        let context_menu_for_context = context_menu.clone();
        let viewport_for_context = picture_viewport.clone();
        right_click.connect_pressed(move |gesture, _, x, y| {
            let Some(photo) = photos_for_context
                .borrow()
                .get(index_for_context.get())
                .cloned()
            else {
                return;
            };
            if let Some(handler) = context_menu_for_context.borrow().as_ref() {
                handler(
                    photo,
                    viewport_for_context.clone().upcast::<gtk::Widget>(),
                    x,
                    y,
                );
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        picture_viewport.add_controller(right_click);

        let picture_for_fit = picture.clone();
        let photos_for_fit = photos.clone();
        let index_for_fit = index.clone();
        let last_width_for_fit = last_width.clone();
        let last_height_for_fit = last_height.clone();
        let opening_fit_pending_for_fit = opening_fit_pending.clone();
        let zoom_for_fit = zoom.clone();
        let applied_native_scale_for_fit = applied_native_scale.clone();

        root.add_tick_callback(move |root, _| {
            if !root.is_visible() {
                return glib::ControlFlow::Continue;
            }
            // The one-shot opening callback owns geometry until it has used
            // the clicked thumbnail's oriented axes. The generic resize path
            // has only catalog dimensions while the decode is pending.
            if opening_fit_pending_for_fit.get() {
                return glib::ControlFlow::Continue;
            }

            let width = root.width();
            let height = root.height();
            let old_width = last_width_for_fit.get();
            let old_height = last_height_for_fit.get();

            if width > 0
                && height > 0
                && (width != old_width || height != old_height)
            {
                // Fit mode follows the window. Manual zoom does not: 125%
                // must remain 125% while only the viewport grows/shrinks.
                //
                // The internal zoom is fit-relative for layout, so preserve
                // the current native-image scale first, then recompute only
                // the fit-relative multiplier for the new viewport.
                let mut resize_zoom = zoom_for_fit.get();
                if resize_zoom > 0.0 {
                    if let Some(photo) = photos_for_fit.borrow().get(index_for_fit.get()).cloned() {
                        let (intrinsic_width, intrinsic_height) =
                            picture_intrinsic_dimensions(&picture_for_fit);

                        let native_scale = if applied_native_scale_for_fit.get() > 0.0 {
                            applied_native_scale_for_fit.get()
                        } else if old_width > 0 && old_height > 0 {
                            let old_fit_scale = presentation_fit_scale(
                                &photo,
                                old_width,
                                old_height,
                                intrinsic_width,
                                intrinsic_height,
                            );
                            old_fit_scale * resize_zoom
                        } else {
                            let current_fit_scale = presentation_fit_scale(
                                &photo,
                                width,
                                height,
                                intrinsic_width,
                                intrinsic_height,
                            );
                            current_fit_scale * resize_zoom
                        };

                        let new_fit_scale = presentation_fit_scale(
                            &photo,
                            width,
                            height,
                            intrinsic_width,
                            intrinsic_height,
                        );
                        resize_zoom = native_scale / new_fit_scale.max(f64::EPSILON);
                        zoom_for_fit.set(resize_zoom);
                        applied_native_scale_for_fit.set(native_scale);
                    }
                }

                last_width_for_fit.set(width);
                last_height_for_fit.set(height);

                zoom_trace(format!(
                    "resize_tick old={}x{} new={}x{} zoom={resize_zoom:.6} applied_native={:.6} picture_req={:?}",
                    old_width,
                    old_height,
                    width,
                    height,
                    applied_native_scale_for_fit.get(),
                    picture_for_fit.size_request(),
                ));
                fit_picture(
                    &picture_for_fit,
                    &photos_for_fit.borrow(),
                    index_for_fit.get(),
                    width,
                    height,
                    resize_zoom,
                    "resize",
                );
            }

            glib::ControlFlow::Continue
        });

        let scroll = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
        );
        // Capture wheel events before GtkScrolledWindow can consume them.
        // Mouse wheel is reserved for previous/next photo navigation in both
        // fit mode and 1:1 mode. Panning in 1:1 is mouse-drag only.
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);

        let photos_for_scroll = photos.clone();
        let index_for_scroll = index.clone();
        let picture_for_scroll = picture.clone();
        let root_for_scroll = root.clone();
        let zoom_for_scroll = zoom.clone();
        let applied_for_scroll = applied_native_scale.clone();
        let one_to_one_for_scroll = one_to_one_active.clone();
        let one_to_one_sync_for_scroll = one_to_one_sync.clone();
        let zoom_sync_for_scroll = zoom_sync.clone();
        let wheel_navigation_for_scroll = wheel_navigation.clone();
        let wheel_dispatch_slot: Rc<RefCell<Option<Rc<dyn Fn(usize, i32)>>>> =
            Rc::new(RefCell::new(None));

        let wheel_dispatch: Rc<dyn Fn(usize, i32)> = {
            let photos = photos.clone();
            let index = index.clone();
            let picture = picture.clone();
            let root = root.clone();
            let zoom = zoom.clone();
            let applied_native_scale = applied_native_scale.clone();
            let generation = load_generation.clone();
            let decode_cancel = decode_cancel.clone();
            let photo_changed = photo_changed.clone();
            let viewport = picture_viewport.clone();
            let native_texture = native_texture.clone();
            let native_quality_pending = native_quality_pending.clone();
            let display_cache = display_texture_cache.clone();
            let one_to_one = one_to_one_active.clone();
            let key_navigation_ready = key_navigation_ready.clone();
            let wheel_navigation = wheel_navigation.clone();
            let dispatch_slot = wheel_dispatch_slot.clone();
            let index_for_settled = index.clone();
            let wheel_navigation_for_settled = wheel_navigation.clone();
            let settled: Rc<dyn Fn()> = Rc::new(move || {
                let Some(target) = wheel_navigation_for_settled
                    .borrow_mut()
                    .take_pending_target()
                else {
                    return;
                };
                let current = index_for_settled.get();
                if target == current {
                    return;
                }
                let direction = if target < current { -1 } else { 1 };
                viewer_trace(format!(
                    "navigation_dispatch source=wheel_coalesced from_index={} to_index={} direction={direction}",
                    current, target
                ));
                if let Some(dispatch) = dispatch_slot.borrow().as_ref() {
                    dispatch(target, direction);
                }
            });
            Rc::new(move |next, direction| {
                // Wheel requests own their completion state. If this replaces
                // an arrow request, restore arrow readiness immediately so a
                // subsequent key press retains its established behavior.
                key_navigation_ready.set(true);
                wheel_navigation.borrow_mut().begin(direction);
                index.set(next);
                set_zoom_state(&zoom, &applied_native_scale, 0.0);
                one_to_one.set(false);
                native_texture.borrow_mut().take();
                // A pending native-quality request belongs to the photo that
                // just left the viewer. The completion path checks identity
                // before applying, so the new photo may start its own request.
                native_quality_pending.set(false);
                reset_viewport(&viewport);
                let (fit_geometry_fixed, cache_hit) = prepare_navigation_photo(
                    &picture,
                    photos.borrow().get(next),
                    &root,
                    zoom.get(),
                    &display_cache,
                );
                viewer_trace(format!(
                    "infobar_update source=wheel index={} uri={}",
                    next,
                    viewer_trace_uri(&photos.borrow()[next].path()),
                ));
                notify_photo_changed(&photo_changed, &photos.borrow(), next);
                let expected_generation = generation.get().wrapping_add(1);
                generation.set(expected_generation);
                show_photo(
                    &picture,
                    &photos.borrow(),
                    next,
                    &root,
                    zoom.clone(),
                    generation.clone(),
                    expected_generation,
                    decode_cancel.clone(),
                    &viewport,
                    native_texture.clone(),
                    display_cache.clone(),
                    fit_geometry_fixed,
                    cache_hit,
                    None,
                    Some(settled.clone()),
                );
                schedule_lightbox_prefetch(
                    photos.clone(),
                    next,
                    direction,
                    root.clone(),
                    zoom.clone(),
                    display_cache.clone(),
                    generation.clone(),
                );
            })
        };
        wheel_dispatch_slot.replace(Some(wheel_dispatch.clone()));

        scroll.connect_scroll(move |controller, _, dy| {
            // Ctrl+wheel zooms the image; ordinary wheel keeps navigation.
            // The controller's modifier state is sampled on the GTK thread.
            if controller
                .current_event_state()
                .contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                // Ctrl+wheel and the InfoBar slider share native-image scale
                // semantics. Fit is the lower bound, 100% is the native-pixel
                // midpoint, and the upper bound matches the slider's right
                // endpoint.
                if one_to_one_for_scroll.get() {
                    one_to_one_for_scroll.set(false);
                    picture_for_scroll.set_can_shrink(true);
                    if let Some(handler) = one_to_one_sync_for_scroll.borrow().as_ref() {
                        handler(false);
                    }
                }

                let Some(photo) = photos_for_scroll.borrow().get(index_for_scroll.get()).cloned()
                else {
                    return glib::Propagation::Stop;
                };
                let (intrinsic_width, intrinsic_height) =
                    picture_intrinsic_dimensions(&picture_for_scroll);
                let fit_scale = presentation_fit_scale(
                    &photo,
                    root_for_scroll.width(),
                    root_for_scroll.height(),
                    intrinsic_width,
                    intrinsic_height,
                );
                let current_scale = if zoom_for_scroll.get() <= 0.0 {
                    fit_scale
                } else {
                    (fit_scale * zoom_for_scroll.get()).max(fit_scale)
                };
                let next_scale = (current_scale * if dy < 0.0 { 1.12 } else { 0.89 })
                    .clamp(fit_scale, LIGHTBOX_MAX_ZOOM_FACTOR);

                zoom_trace(format!(
                    "ctrl_wheel dy={dy:.3} fit={fit_scale:.6} current_native={current_scale:.6} next_native={next_scale:.6} zoom_before={:.6} picture_req={:?} alloc={}x{}",
                    zoom_for_scroll.get(),
                    picture_for_scroll.size_request(),
                    picture_for_scroll.width(),
                    picture_for_scroll.height(),
                ));

                if next_scale <= fit_scale * 1.001 {
                    zoom_for_scroll.set(0.0);
                    applied_for_scroll.set(0.0);
                } else {
                    zoom_for_scroll.set(next_scale / fit_scale.max(f64::EPSILON));
                    // Ctrl+wheel knows the native scale it landed on, so the
                    // record stays exact and a slider reporting the same value
                    // is still correctly treated as a repeat.
                    applied_for_scroll.set(next_scale);
                }

                fit_picture(
                    &picture_for_scroll,
                    &photos_for_scroll.borrow(),
                    index_for_scroll.get(),
                    root_for_scroll.width(),
                    root_for_scroll.height(),
                    zoom_for_scroll.get(),
                    "ctrl-wheel",
                );

                if let Some(handler) = zoom_sync_for_scroll.borrow().as_ref() {
                    handler(if zoom_for_scroll.get() == 0.0 {
                        0.0
                    } else {
                        next_scale
                    });
                }
                return glib::Propagation::Stop;
            }
            if dy == 0.0 || !root_for_scroll.is_visible() {
                return glib::Propagation::Proceed;
            }

            viewer_trace(format!("wheel_event dy={dy}"));

            let len = photos_for_scroll.borrow().len();
            if len == 0 {
                return glib::Propagation::Stop;
            }

            let direction = if dy < 0.0 { -1 } else { 1 };
            let current = index_for_scroll.get();
            if wheel_navigation_for_scroll.borrow().active_direction != 0 {
                let mut state = wheel_navigation_for_scroll.borrow_mut();
                if state.active_direction != 0 && state.active_direction != direction {
                    state.cancel();
                    let next = navigation_step(current, direction, len);
                    drop(state);
                    if next != current {
                        viewer_trace(format!(
                            "navigation_dispatch source=wheel_reversal from_index={} to_index={} direction={direction}",
                            current, next
                        ));
                        wheel_dispatch(next, direction);
                    }
                } else if let Some(target) = state.queue_step(current, direction, len) {
                    viewer_trace(format!(
                        "wheel_accumulate current_index={} target_index={} direction={direction}",
                        current, target
                    ));
                }
                return glib::Propagation::Stop;
            }

            let next = navigation_step(current, direction, len);
            if next != current {
                viewer_trace(format!(
                    "navigation_dispatch source=wheel from_index={} to_index={} direction={direction}",
                    current, next
                ));
                wheel_dispatch(next, direction);
            }

            glib::Propagation::Stop
        });
        root.add_controller(scroll);

        let key = gtk::EventControllerKey::new();
        // Capture before focused children can consume the event, so Escape always closes the lightbox.
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        let root_for_escape = root.clone();
        let picture_for_key = picture.clone();
        let photos_for_key = photos.clone();
        let index_for_key = index.clone();
        let zoom_for_key = zoom.clone();
        let applied_for_key = applied_native_scale.clone();
        let generation_for_key = load_generation.clone();
        let cancel_for_key = decode_cancel.clone();
        let photo_changed_for_key = photo_changed.clone();
        let viewport_for_key = picture_viewport.clone();
        let native_texture_for_key = native_texture.clone();
        let display_cache_for_key = display_texture_cache.clone();
        let one_to_one_for_key = one_to_one_active.clone();
        let collection_navigation_for_key = collection_navigation.clone();
        let key_navigation_ready_for_key = key_navigation_ready.clone();
        let wheel_navigation_for_key = wheel_navigation.clone();

        key.connect_key_pressed(move |_, key, _, _| {
            if (key == gtk::gdk::Key::Escape || key == gtk::gdk::Key::BackSpace)
                && root_for_escape.is_visible()
            {
                root_for_escape.set_visible(false);
                glib::Propagation::Stop
            } else if [gtk::gdk::Key::plus, gtk::gdk::Key::equal].contains(&key) {
                let current = if zoom_for_key.get() <= 0.0 {
                    1.0
                } else {
                    zoom_for_key.get()
                };
                // Cap the internal (fit-relative) zoom at the same native
                // magnification the slider and wheel allow.
                let max_internal = photos_for_key
                    .borrow()
                    .get(index_for_key.get())
                    .map(|photo| {
                        let (intrinsic_width, intrinsic_height) =
                            picture_intrinsic_dimensions(&picture_for_key);
                        let fit_scale = presentation_fit_scale(
                            photo,
                            root_for_escape.width(),
                            root_for_escape.height(),
                            intrinsic_width,
                            intrinsic_height,
                        );
                        LIGHTBOX_MAX_ZOOM_FACTOR / fit_scale.max(f64::EPSILON)
                    })
                    .unwrap_or(4.0);
                set_zoom_state(
                    &zoom_for_key,
                    &applied_for_key,
                    (current * 1.15).min(max_internal),
                );
                fit_picture(
                    &picture_for_key,
                    &photos_for_key.borrow(),
                    index_for_key.get(),
                    root_for_escape.width(),
                    root_for_escape.height(),
                    zoom_for_key.get(),
                    "key",
                );
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::minus {
                let current = if zoom_for_key.get() <= 0.0 {
                    1.0
                } else {
                    zoom_for_key.get()
                };
                set_zoom_state(&zoom_for_key, &applied_for_key, (current * 0.87).max(0.25));
                fit_picture(
                    &picture_for_key,
                    &photos_for_key.borrow(),
                    index_for_key.get(),
                    root_for_escape.width(),
                    root_for_escape.height(),
                    zoom_for_key.get(),
                    "key",
                );
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::_0 {
                set_zoom_state(&zoom_for_key, &applied_for_key, 0.0);
                fit_picture(
                    &picture_for_key,
                    &photos_for_key.borrow(),
                    index_for_key.get(),
                    root_for_escape.width(),
                    root_for_escape.height(),
                    0.0,
                    "key",
                );
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::_1 {
                set_zoom_state(&zoom_for_key, &applied_for_key, 1.0);
                fit_picture(
                    &picture_for_key,
                    &photos_for_key.borrow(),
                    index_for_key.get(),
                    root_for_escape.width(),
                    root_for_escape.height(),
                    1.0,
                    "key",
                );
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::Left || key == gtk::gdk::Key::Right {
                // Arrow navigation never inherits an accumulated wheel target.
                // Its one-step readiness behavior remains otherwise unchanged.
                wheel_navigation_for_key.borrow_mut().cancel();
                if !key_navigation_ready_for_key.get() {
                    return glib::Propagation::Stop;
                }
                let len = photos_for_key.borrow().len();
                if len == 0 {
                    return glib::Propagation::Stop;
                }

                let current = index_for_key.get();
                let direction = if key == gtk::gdk::Key::Left { -1 } else { 1 };
                let next = navigation_step(current, direction, len);

                if next != current {
                    key_navigation_ready_for_key.set(false);
                    index_for_key.set(next);
                    set_zoom_state(&zoom_for_key, &applied_for_key, 0.0);
                    one_to_one_for_key.set(false);
                    native_texture_for_key.borrow_mut().take();
                    reset_viewport(&viewport_for_key);
                    let (fit_geometry_fixed, cache_hit) = prepare_navigation_photo(
                        &picture_for_key,
                        photos_for_key.borrow().get(next),
                        &root_for_escape,
                        zoom_for_key.get(),
                        &display_cache_for_key,
                    );
                    notify_photo_changed(&photo_changed_for_key, &photos_for_key.borrow(), next);
                    let generation = generation_for_key.get().wrapping_add(1);
                    generation_for_key.set(generation);
                    show_photo(
                        &picture_for_key,
                        &photos_for_key.borrow(),
                        next,
                        &root_for_escape,
                        zoom_for_key.clone(),
                        generation_for_key.clone(),
                        generation,
                        cancel_for_key.clone(),
                        &viewport_for_key,
                        native_texture_for_key.clone(),
                        display_cache_for_key.clone(),
                        fit_geometry_fixed,
                        cache_hit,
                        Some(key_navigation_ready_for_key.clone()),
                        None,
                    );
                    schedule_lightbox_prefetch(
                        photos_for_key.clone(),
                        next,
                        direction,
                        root_for_escape.clone(),
                        zoom_for_key.clone(),
                        display_cache_for_key.clone(),
                        generation_for_key.clone(),
                    );
                }
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::Up || key == gtk::gdk::Key::Down {
                let direction = if key == gtk::gdk::Key::Up { -1 } else { 1 };
                if let Some(handler) = collection_navigation_for_key.borrow().as_ref() {
                    handler(direction);
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            } else {
                glib::Propagation::Proceed
            }
        });
        root.add_controller(key);

        Self {
            root,
            backdrop,
            picture,
            picture_viewport,
            photos,
            index,
            last_width,
            last_height,
            opening_fit_pending,
            zoom,
            zoom_before_one_to_one,
            applied_native_scale,
            one_to_one_active,
            native_texture,
            native_quality_pending,
            display_texture_cache,
            load_generation,
            decode_cancel,
            key_navigation_ready,
            wheel_navigation,
            photo_changed,
            one_to_one_sync,
            zoom_sync,
            context_menu,
            collection_navigation,
        }
    }

    pub fn set_photo_changed_handler(&self, handler: impl Fn(PhotoObject) + 'static) {
        self.photo_changed.replace(Some(Box::new(handler)));
    }

    pub fn set_one_to_one_sync_handler(&self, handler: impl Fn(bool) + 'static) {
        self.one_to_one_sync.replace(Some(Box::new(handler)));
    }

    pub fn set_zoom_sync_handler(&self, handler: impl Fn(f64) + 'static) {
        self.zoom_sync.replace(Some(Box::new(handler)));
    }

    /// Reset the viewer's zoom to a fit-relative value and invalidate the record
    /// of the last applied native scale. See `set_zoom_state`.
    fn set_zoom(&self, value: f64) {
        set_zoom_state(&self.zoom, &self.applied_native_scale, value);
    }

    /// Set the lightbox by native-image scale. 0.0 is Fit; 1.0 is true
    /// 100%, and values above 1.0 are magnified.
    pub fn set_manual_zoom_scale(&self, native_scale: f64) {
        self.apply_manual_zoom_scale(native_scale, true, "reset");
    }

    /// Slider input applies the requested scale immediately, exactly like
    /// Ctrl+wheel and the zoom keys: one geometry write per event, coalesced
    /// by GTK into a single layout per frame. There is no separate render
    /// transform, no base scale to remember and no deferred commit, so the
    /// image on screen is always at the scale the slider asked for. The
    /// range-change anchor keeps the visual centre fixed while that geometry
    /// is laid out, in the same frame the geometry changes.
    pub fn request_slider_zoom(&self, native_scale: f64) {
        zoom_trace(format!(
            "request_slider native_req={native_scale:.6} applied_native={:.6} zoom={:.6} root={}x{} picture_req={:?} picture_alloc={}x{} hadj={:.1}/{:.1}/{:.1} vadj={:.1}/{:.1}/{:.1}",
            self.applied_native_scale.get(),
            self.zoom.get(),
            self.root.width(),
            self.root.height(),
            self.picture.size_request(),
            self.picture.width(),
            self.picture.height(),
            self.picture_viewport.hadjustment().value(),
            self.picture_viewport.hadjustment().upper(),
            self.picture_viewport.hadjustment().page_size(),
            self.picture_viewport.vadjustment().value(),
            self.picture_viewport.vadjustment().upper(),
            self.picture_viewport.vadjustment().page_size(),
        ));
        // A GtkScale keeps reporting its value while the pointer is held still
        // on the trough, and each report would otherwise redo the metadata
        // lookup, the fit maths and the size write for a scale that is already
        // on screen. Dropping the repeat here also keeps a held slider from
        // re-queuing work between frames.
        let previous = self.applied_native_scale.get();
        if (previous - native_scale).abs() <= f64::EPSILON {
            return;
        }
        // The slider already displays the requested value, so echoing it back
        // through zoom_sync would only fight the drag.
        self.apply_manual_zoom_scale(native_scale, false, "slider");
    }

    fn ensure_native_zoom_texture(&self, photo: &PhotoObject) {
        let path = photo.path();
        let rotation = photo.rotation();
        let edit_recipe = photo.edit_recipe();

        if let Some(cache) = self.native_texture.borrow().as_ref() {
            if cache.path == path
                && cache.rotation == rotation
                && cache.edit_recipe == edit_recipe
            {
                let request = self.picture.size_request();
                self.picture.set_paintable(Some(&cache.texture));
                if self.picture.size_request() != request {
                    self.picture.set_size_request(request.0, request.1);
                }
                self.picture.queue_draw();
                return;
            }
        }

        if self.native_quality_pending.replace(true) {
            return;
        }

        let (target_width, target_height, _, _, _, _) =
            viewer_decode_target(&self.root, rotation, true);
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

        let picture = self.picture.clone();
        let root = self.root.clone();
        let photos = self.photos.clone();
        let index = self.index.clone();
        let zoom = self.zoom.clone();
        let one_to_one = self.one_to_one_active.clone();
        let native_texture = self.native_texture.clone();
        let pending = self.native_quality_pending.clone();
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

            let still_current = photos
                .borrow()
                .get(index.get())
                .is_some_and(|current| {
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

    fn apply_manual_zoom_scale(&self, native_scale: f64, notify_zoom_sync: bool, source: &str) {
        if !self.root.is_visible() {
            return;
        }
        let commit_started = std::time::Instant::now();

        // Do not enter the expensive special 1:1/native-texture mode merely
        // because a continuous slider crosses 100%. The geometry is still
        // exactly native-size at scale 1.0; the explicit 1:1 button keeps its
        // existing dedicated decode path.
        if self.one_to_one_active.replace(false) {
            self.picture.set_can_shrink(true);
            if let Some(handler) = self.one_to_one_sync.borrow().as_ref() {
                handler(false);
            }
        }

        let Some(photo) = self.photos.borrow().get(self.index.get()).cloned() else {
            return;
        };
        let (intrinsic_width, intrinsic_height) = picture_intrinsic_dimensions(&self.picture);
        let fit_scale = presentation_fit_scale(
            &photo,
            self.root.width(),
            self.root.height(),
            intrinsic_width,
            intrinsic_height,
        );
        zoom_trace(format!(
            "apply_manual source={source} native_req={native_scale:.6} fit={fit_scale:.6} intrinsic={}x{} root={}x{} current_zoom={:.6} applied_native={:.6}",
            intrinsic_width,
            intrinsic_height,
            self.root.width(),
            self.root.height(),
            self.zoom.get(),
            self.applied_native_scale.get(),
        ));

        if native_scale <= 0.0 || native_scale <= fit_scale * 1.001 {
            self.set_zoom(0.0);
            fit_picture(
                &self.picture,
                &self.photos.borrow(),
                self.index.get(),
                self.root.width(),
                self.root.height(),
                0.0,
                source,
            );
            // Fitted geometry never overflows the viewport, so GTK clamps the
            // scroll range to nothing and centres by alignment. Writing the
            // origin here as well would only race the range change that is
            // about to do the same thing.
            if notify_zoom_sync {
                if let Some(handler) = self.zoom_sync.borrow().as_ref() {
                    handler(0.0);
                }
            }
            return;
        }

        let native_scale = native_scale.clamp(fit_scale, LIGHTBOX_MAX_ZOOM_FACTOR);
        self.zoom.set(native_scale / fit_scale.max(f64::EPSILON));
        // This path knows the native scale it landed on, so the record stays
        // exact: only `set_zoom` (fit and 1:1) has to invalidate it.
        self.applied_native_scale.set(native_scale);

        fit_picture(
            &self.picture,
            &self.photos.borrow(),
            self.index.get(),
            self.root.width(),
            self.root.height(),
            self.zoom.get(),
            source,
        );

        // A slider can issue a new picture size every frame. Publish the
        // matching scroll ranges now, before GTK's queued layout, so the
        // rendered frame never mixes the old adjustment range with the new
        // picture allocation. Ctrl+wheel already feels stable and keeps its
        // established path unchanged.
        if source == "slider" {
            let (picture_width, picture_height) = self.picture.size_request();
            prime_viewport_for_picture_size(
                &self.picture_viewport,
                picture_width,
                picture_height,
            );
        }

        self.ensure_native_zoom_texture(&photo);
        zoom_trace(format!(
            "slider_geometry source={source} native={native_scale:.5} fit={fit_scale:.5} zoom={:.5} commit_us={}",
            self.zoom.get(),
            commit_started.elapsed().as_micros(),
        ));
        if notify_zoom_sync {
            if let Some(handler) = self.zoom_sync.borrow().as_ref() {
                handler(native_scale);
            }
        }
    }

    pub fn current_fit_scale(&self) -> f64 {
        let Some(photo) = self.photos.borrow().get(self.index.get()).cloned() else {
            return 1.0;
        };
        let (intrinsic_width, intrinsic_height) = picture_intrinsic_dimensions(&self.picture);
        presentation_fit_scale(
            &photo,
            self.root.width(),
            self.root.height(),
            intrinsic_width,
            intrinsic_height,
        )
    }

    pub fn current_manual_zoom_scale(&self) -> f64 {
        if self.one_to_one_active.get() {
            return 1.0;
        }
        if self.zoom.get() <= 0.0 {
            return 0.0;
        }
        let Some(photo) = self.photos.borrow().get(self.index.get()).cloned() else {
            return 0.0;
        };
        let (intrinsic_width, intrinsic_height) = picture_intrinsic_dimensions(&self.picture);
        presentation_fit_scale(
            &photo,
            self.root.width(),
            self.root.height(),
            intrinsic_width,
            intrinsic_height,
        ) * self.zoom.get()
    }

    pub fn set_context_menu_handler(
        &self,
        handler: impl Fn(PhotoObject, gtk::Widget, f64, f64) + 'static,
    ) {
        self.context_menu.replace(Some(Box::new(handler)));
    }

    pub fn set_collection_navigation_handler(&self, handler: impl Fn(i32) + 'static) {
        self.collection_navigation.replace(Some(Box::new(handler)));
    }

    /// Toggle native-pixel presentation while remembering the previous zoom.
    /// A negative zoom is reserved for this temporary 1:1 mode.
    pub fn set_one_to_one(&self, enabled: bool) {
        if self.one_to_one_active.get() == enabled {
            return;
        }

        self.one_to_one_active.set(enabled);
        if let Some(handler) = self.one_to_one_sync.borrow().as_ref() {
            handler(enabled);
        }
        if let Some(handler) = self.zoom_sync.borrow().as_ref() {
            handler(if enabled { 1.0 } else { 0.0 });
        }
        self.picture.set_can_shrink(!enabled);
        self.picture_viewport
            .set_cursor_from_name(if enabled { Some("grab") } else { None });

        if enabled {
            if self.zoom.get() != -1.0 {
                self.zoom_before_one_to_one.set(self.zoom.get());
                self.set_zoom(-1.0);
            }

            // Reuse the already decoded native texture when possible. This
            // makes repeated 1:1 toggles instantaneous instead of decoding
            // the same source again every time.
            let cached = self.native_texture.borrow().clone();
            let current = self
                .photos
                .borrow()
                .get(self.index.get())
                .map(|photo| (photo.path(), photo.rotation(), photo.edit_recipe()));

            if let (Some(cache), Some((path, rotation, edit_recipe))) = (cached, current) {
                if cache.path == path
                    && cache.rotation == rotation
                    && cache.edit_recipe == edit_recipe
                {
                    self.picture.set_paintable(Some(&cache.texture));
                    fit_picture(
                        &self.picture,
                        &self.photos.borrow(),
                        self.index.get(),
                        self.root.width(),
                        self.root.height(),
                        -1.0,
                        "one-to-one",
                    );
                    self.picture.queue_resize();
                    self.picture_viewport.queue_resize();
                    return;
                }
            }
        } else {
            // 1:1 off always returns to fit-to-window.
            self.set_zoom(0.0);
            fit_picture(
                &self.picture,
                &self.photos.borrow(),
                self.index.get(),
                self.root.width(),
                self.root.height(),
                0.0,
                "one-to-one-off",
            );
            self.picture.queue_resize();
            self.picture_viewport.queue_resize();
            reset_viewport(&self.picture_viewport);
            return;
        }

        if self.root.is_visible() {
            self.refresh_current();
        }
    }

    pub fn use_iphone_backdrop(&self) {
        self.backdrop.remove_css_class("standard-light");
        self.backdrop.remove_css_class("standard-dark");
    }

    pub fn use_standard_backdrop(&self, dark: bool) {
        self.backdrop.remove_css_class(if dark {
            "standard-light"
        } else {
            "standard-dark"
        });
        self.backdrop.add_css_class(if dark {
            "standard-dark"
        } else {
            "standard-light"
        });
    }

    pub fn open(&self, photos: Vec<PhotoObject>, selected: usize) {
        self.open_internal(photos, selected, None);
    }

    pub fn open_from_source(
        &self,
        photos: Vec<PhotoObject>,
        selected: usize,
        source: Option<(gtk::Widget, gtk::gdk::Paintable)>,
    ) {
        self.open_internal(photos, selected, source);
    }

    fn open_internal(
        &self,
        photos: Vec<PhotoObject>,
        selected: usize,
        source: Option<(gtk::Widget, gtk::gdk::Paintable)>,
    ) {
        self.photos.replace(photos);

        let len = self.photos.borrow().len();
        if len == 0 {
            return;
        }

        self.index.set(selected.min(len - 1));
        self.set_zoom(0.0);
        self.zoom_before_one_to_one.set(0.0);
        self.one_to_one_active.set(false);
        if let Some(handler) = self.zoom_sync.borrow().as_ref() {
            handler(0.0);
        }
        self.key_navigation_ready.set(true);
        self.wheel_navigation.borrow_mut().cancel();
        self.native_texture.borrow_mut().take();
        self.native_quality_pending.set(false);
        reset_viewport(&self.picture_viewport);
        notify_photo_changed(&self.photo_changed, &self.photos.borrow(), self.index.get());
        self.last_width.set(0);
        self.last_height.set(0);
        self.opening_fit_pending.set(true);
        let generation = self.load_generation.get().wrapping_add(1);
        self.load_generation.set(generation);

        // Shared-element opens fade the backdrop in under the moving thumbnail.
        // Instant opens keep the established presentation unchanged.
        if source.is_some() {
            self.backdrop.set_opacity(0.0);
            self.picture.set_opacity(0.0);
        } else {
            self.backdrop.set_opacity(1.0);
            self.picture.set_opacity(1.0);
        }

        // Make the overlay allocatable before selecting a decode target. On
        // the first open it was previously still 0x0 here, so the initial
        // photo used a fallback size and appeared smaller until navigation.
        self.root.set_visible(true);

        // The overlay can still be unmapped/unallocated at this exact point,
        // so the immediate focus request is not always enough. That left the
        // underlying GtkGridView owning the arrow keys until another action
        // (such as Enter) happened to move focus. Request focus now and again
        // on the next idle iteration, once GTK has mapped the lightbox.
        self.root.grab_focus();
        let root_for_focus = self.root.clone();
        glib::idle_add_local_once(move || {
            if root_for_focus.is_visible() {
                root_for_focus.grab_focus();
            }
        });

        // An open can reuse the same GtkPicture from the previous lightbox.
        // Clear it so the first image loads against the neutral viewer
        // background instead of flashing a low-resolution cached thumbnail
        // or briefly showing the prior photo.
        self.picture.set_paintable(gtk::gdk::Paintable::NONE);
        self.picture.set_filename(Option::<&str>::None);

        // GTK does not allocate an overlay synchronously when it becomes
        // visible. Start the first full decode on its first allocated frame,
        // otherwise viewer_decode_target sees 0x0 and permanently uses the
        // fallback target until the user navigates.
        let picture = self.picture.clone();
        let photos = self.photos.clone();
        let index = self.index.clone();
        let zoom = self.zoom.clone();
        let current_generation = self.load_generation.clone();
        let decode_cancel = self.decode_cancel.clone();
        let picture_viewport = self.picture_viewport.clone();
        let native_texture = self.native_texture.clone();
        let display_texture_cache = self.display_texture_cache.clone();
        let backdrop_for_transition = self.backdrop.clone();
        let last_width_for_open = self.last_width.clone();
        let last_height_for_open = self.last_height.clone();
        let opening_fit_pending_for_open = self.opening_fit_pending.clone();
        let source_for_transition = Rc::new(RefCell::new(source));
        let source_for_first_frame = source_for_transition.clone();
        self.root.add_tick_callback(move |root, _| {
            if !root.is_visible() || current_generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            if root.width() <= 0 || root.height() <= 0 {
                return glib::ControlFlow::Continue;
            }

            let (_fit_geometry_fixed, cache_hit) = prepare_navigation_photo(
                &picture,
                photos.borrow().get(index.get()),
                root,
                zoom.get(),
                &display_texture_cache,
            );
            show_photo(
                &picture,
                &photos.borrow(),
                index.get(),
                root,
                zoom.clone(),
                current_generation.clone(),
                generation,
                decode_cancel.clone(),
                &picture_viewport,
                native_texture.clone(),
                display_texture_cache.clone(),
                // fit_picture below establishes the same final geometry used
                // by the shared-element transition. Do not let a later decode
                // re-fit from its intrinsic dimensions and make the real
                // picture settle a second time after the 200 ms animation.
                true,
                cache_hit,
                None,
                None,
            );
            fit_picture(
                &picture,
                &photos.borrow(),
                index.get(),
                root.width(),
                root.height(),
                zoom.get(),
                "open",
            );

            // The catalog may still contain pre-migration sensor dimensions
            // for an EXIF-rotated RAW. The clicked thumbnail is already in
            // display orientation, so use its intrinsic axes to choose the
            // destination orientation before the animation begins. The full
            // decode then replaces pixels without changing this geometry.
            if let Some((_, source_paintable)) = source_for_first_frame.borrow().as_ref() {
                if let Some(photo) = photos.borrow().get(index.get()) {
                    set_fit_geometry_from_intrinsic(
                        &picture,
                        photo,
                        root,
                        zoom.get(),
                        source_paintable.intrinsic_width(),
                        source_paintable.intrinsic_height(),
                    );
                }
            }

            // The one-shot opening callback has now fitted the real picture
            // for this exact viewport. Mark that allocation as handled before
            // the persistent resize callback runs. Otherwise that callback
            // performs a second fit from stale catalog axes while the real
            // paintable is still empty, overwriting the source-oriented RAW
            // destination during the 200 ms transition.
            last_width_for_open.set(root.width());
            last_height_for_open.set(root.height());
            opening_fit_pending_for_open.set(false);

            if let Some((source_widget, source_paintable)) =
                source_for_first_frame.borrow_mut().take()
            {
                if let Some(source_bounds) = source_widget.compute_bounds(root) {
                    let dest_width = picture.width_request().max(1);
                    let dest_height = picture.height_request().max(1);
                    if source_bounds.width() > 1.0
                        && source_bounds.height() > 1.0
                        && dest_width > 1
                        && dest_height > 1
                    {
                        // The tile's paintable is the cached JPEG thumbnail,
                        // which lost a PNG's alpha and paints black where the
                        // source is transparent. The viewer decode is RGBA, so
                        // swap the decoded texture into the moving overlay as
                        // soon as it is available; the animated geometry is
                        // untouched.
                        let png_transition = photos
                            .borrow()
                            .get(index.get())
                            .is_some_and(|photo| png_uses_theme_background(&photo.path()));
                        let decoded_swapped = Cell::new(false);
                        let transition = gtk::Picture::for_paintable(&source_paintable);
                        transition.set_can_shrink(true);
                        transition.set_content_fit(gtk::ContentFit::Cover);
                        if picture.has_css_class("photo-grid") {
                            transition.add_css_class("photo-grid");
                        }
                        transition.set_halign(gtk::Align::Start);
                        transition.set_valign(gtk::Align::Start);
                        transition.set_can_target(false);
                        transition.set_size_request(
                            source_bounds.width().round() as i32,
                            source_bounds.height().round() as i32,
                        );
                        transition.set_margin_start(source_bounds.x().round().max(0.0) as i32);
                        transition.set_margin_top(source_bounds.y().round().max(0.0) as i32);
                        root.add_overlay(&transition);
                        if png_transition {
                            if let Some(decoded) = picture.paintable() {
                                decoded_swapped.set(true);
                                transition.set_paintable(Some(&decoded));
                            }
                        }

                        let start_x = source_bounds.x() as f64;
                        let start_y = source_bounds.y() as f64;
                        let start_w = source_bounds.width() as f64;
                        let start_h = source_bounds.height() as f64;
                        let end_w = dest_width as f64;
                        let end_h = dest_height as f64;
                        let end_x = (root.width() as f64 - end_w) * 0.5;
                        let end_y = (root.height() as f64 - end_h) * 0.5;
                        viewer_trace(format!(
                            "open_transition start={start_x:.1},{start_y:.1} {start_w:.1}x{start_h:.1} end={end_x:.1},{end_y:.1} {end_w:.1}x{end_h:.1} root={}x{} picture_request={}x{}",
                            root.width(),
                            root.height(),
                            picture.width_request(),
                            picture.height_request(),
                        ));
                        let started = Instant::now();
                        let picture_for_transition = picture.clone();
                        let backdrop = backdrop_for_transition.clone();
                        let root_for_transition = root.clone();
                        let generation_for_transition = current_generation.clone();

                        transition.add_tick_callback(move |transition, _| {
                            const OPEN_TRANSITION_MS: f64 = 200.0;
                            let linear = (started.elapsed().as_secs_f64() * 1000.0
                                / OPEN_TRANSITION_MS)
                                .clamp(0.0, 1.0);
                            let eased = 1.0 - (1.0 - linear).powi(3);

                            let x = start_x + (end_x - start_x) * eased;
                            let y = start_y + (end_y - start_y) * eased;
                            let w = start_w + (end_w - start_w) * eased;
                            let h = start_h + (end_h - start_h) * eased;
                            transition.set_margin_start(x.round().max(0.0) as i32);
                            transition.set_margin_top(y.round().max(0.0) as i32);
                            transition.set_size_request(
                                w.round().max(1.0) as i32,
                                h.round().max(1.0) as i32,
                            );
                            backdrop.set_opacity(linear);

                            if png_transition
                                && !decoded_swapped.get()
                                && generation_for_transition.get() == generation
                            {
                                if let Some(decoded) = picture_for_transition.paintable() {
                                    decoded_swapped.set(true);
                                    transition.set_paintable(Some(&decoded));
                                }
                            }

                            if linear >= 1.0 {
                                // PNG thumbnails are cached as JPEG and cannot
                                // preserve alpha. Hold the transition overlay at
                                // its final geometry until the real RGBA viewer
                                // texture arrives instead of exposing a blank
                                // lightbox between animation and decode.
                                if png_transition && !decoded_swapped.get() {
                                    if generation_for_transition.get() != generation {
                                        root_for_transition.remove_overlay(transition);
                                        return glib::ControlFlow::Break;
                                    }
                                    return glib::ControlFlow::Continue;
                                }

                                // For non-PNG images, keep the existing source
                                // thumbnail backstop if the full decode is still
                                // pending. show_photo() replaces it when ready.
                                if picture_for_transition.paintable().is_none() && !png_transition {
                                    picture_for_transition
                                        .set_paintable(transition.paintable().as_ref());
                                }
                                picture_for_transition.set_opacity(1.0);
                                backdrop.set_opacity(1.0);
                                viewer_trace(format!(
                                    "open_transition complete picture_alloc={}x{} picture_request={}x{} source_backstop={}",
                                    picture_for_transition.width(),
                                    picture_for_transition.height(),
                                    picture_for_transition.width_request(),
                                    picture_for_transition.height_request(),
                                    picture_for_transition.paintable().is_some(),
                                ));
                                root_for_transition.remove_overlay(transition);
                                glib::ControlFlow::Break
                            } else {
                                glib::ControlFlow::Continue
                            }
                        });
                    } else {
                        picture.set_opacity(1.0);
                        backdrop_for_transition.set_opacity(1.0);
                    }
                } else {
                    picture.set_opacity(1.0);
                    backdrop_for_transition.set_opacity(1.0);
                }
            } else {
                picture.set_opacity(1.0);
                backdrop_for_transition.set_opacity(1.0);
            }
            glib::ControlFlow::Break
        });
        schedule_lightbox_prefetch(
            self.photos.clone(),
            self.index.get(),
            1,
            self.root.clone(),
            self.zoom.clone(),
            self.display_texture_cache.clone(),
            self.load_generation.clone(),
        );
    }

    pub fn navigate_photo(&self, direction: i32) {
        self.wheel_navigation.borrow_mut().cancel();
        if !self.root.is_visible() || !self.key_navigation_ready.get() {
            return;
        }
        let len = self.photos.borrow().len();
        if len == 0 {
            return;
        }

        let current = self.index.get();
        let next = navigation_step(current, direction, len);
        if next == current {
            return;
        }

        self.key_navigation_ready.set(false);
        self.index.set(next);
        self.set_zoom(0.0);
        self.one_to_one_active.set(false);
        self.native_texture.borrow_mut().take();
        self.native_quality_pending.set(false);
        reset_viewport(&self.picture_viewport);

        let (fit_geometry_fixed, cache_hit) = prepare_navigation_photo(
            &self.picture,
            self.photos.borrow().get(next),
            &self.root,
            self.zoom.get(),
            &self.display_texture_cache,
        );

        notify_photo_changed(&self.photo_changed, &self.photos.borrow(), next);

        let generation = self.load_generation.get().wrapping_add(1);
        self.load_generation.set(generation);
        show_photo(
            &self.picture,
            &self.photos.borrow(),
            next,
            &self.root,
            self.zoom.clone(),
            self.load_generation.clone(),
            generation,
            self.decode_cancel.clone(),
            &self.picture_viewport,
            self.native_texture.clone(),
            self.display_texture_cache.clone(),
            fit_geometry_fixed,
            cache_hit,
            Some(self.key_navigation_ready.clone()),
            None,
        );
        schedule_lightbox_prefetch(
            self.photos.clone(),
            next,
            direction,
            self.root.clone(),
            self.zoom.clone(),
            self.display_texture_cache.clone(),
            self.load_generation.clone(),
        );
    }

    /// Remove a photo from the lightbox collection while keeping the viewer
    /// on the nearest remaining photo. This is used when a photo leaves the
    /// collection currently being browsed (for example, removing a Favourite).
    pub fn remove_photo(&self, photo_id: i64) {
        let current = self.index.get();
        let (removed_position, remaining_len) = {
            let mut photos = self.photos.borrow_mut();
            let Some(position) = photos.iter().position(|photo| photo.id() == photo_id) else {
                return;
            };
            photos.remove(position);
            (position, photos.len())
        };

        if remaining_len == 0 {
            self.close();
            return;
        }

        // If an item before the displayed photo disappeared, compensate for
        // the shifted vector index. If the displayed photo itself disappeared,
        // keep the same slot so the next photo replaces it; for the old final
        // item this naturally selects the previous photo instead.
        let next = if removed_position < current {
            current.saturating_sub(1)
        } else if removed_position == current {
            current.min(remaining_len - 1)
        } else {
            current.min(remaining_len - 1)
        };
        self.index.set(next);

        // Removing a photo after the current one does not change what is on
        // screen. Only re-present when the displayed photo was removed.
        if removed_position != current || !self.root.is_visible() {
            return;
        }

        self.set_zoom(0.0);
        self.zoom_before_one_to_one.set(0.0);
        self.one_to_one_active.set(false);
        self.picture.set_can_shrink(true);
        self.picture_viewport.set_cursor_from_name(None);
        self.native_texture.borrow_mut().take();
        self.native_quality_pending.set(false);
        reset_viewport(&self.picture_viewport);

        let (fit_geometry_fixed, cache_hit) = prepare_navigation_photo(
            &self.picture,
            self.photos.borrow().get(next),
            &self.root,
            self.zoom.get(),
            &self.display_texture_cache,
        );

        notify_photo_changed(&self.photo_changed, &self.photos.borrow(), next);

        let generation = self.load_generation.get().wrapping_add(1);
        self.load_generation.set(generation);
        show_photo(
            &self.picture,
            &self.photos.borrow(),
            next,
            &self.root,
            self.zoom.clone(),
            self.load_generation.clone(),
            generation,
            self.decode_cancel.clone(),
            &self.picture_viewport,
            self.native_texture.clone(),
            self.display_texture_cache.clone(),
            fit_geometry_fixed,
            cache_hit,
            None,
            None,
        );
        self.root.grab_focus();
    }

    pub fn navigate_collection(&self, direction: i32) {
        if !self.root.is_visible() || direction == 0 {
            return;
        }
        if let Some(handler) = self.collection_navigation.borrow().as_ref() {
            handler(direction);
        }
    }

    pub fn close(&self) {
        cancel_lightbox_prefetch();
        // Invalidate an in-flight full-resolution decode as well as hiding
        // the viewer. A late worker result must not repopulate a closed view.
        self.load_generation
            .set(self.load_generation.get().wrapping_add(1));
        if let Some(active) = self.decode_cancel.borrow_mut().take() {
            active.cancel();
        }
        self.one_to_one_active.set(false);
        self.native_quality_pending.set(false);
        self.key_navigation_ready.set(true);
        self.wheel_navigation.borrow_mut().cancel();
        self.set_zoom(0.0);
        reset_viewport(&self.picture_viewport);
        self.root.set_visible(false);
    }

    /// Apply a freshly written edit recipe to this lightbox's own photo
    /// objects. Gallery paste handlers update `current_photos` and
    /// `selected_photo`, but `refresh_current` re-decodes from `self.photos`,
    /// which is a separate vec captured at `open()`. Without this, a paste
    /// that lands while the viewer is open re-renders the stale recipe until
    /// the photo is closed and reopened.
    pub fn update_edit_recipe(&self, id: i64, recipe: &str) {
        self.update_edit_recipes_batch(&[(id, recipe.to_string())]);
    }

    pub fn update_edit_recipes_batch(&self, updates: &[(i64, String)]) {
        if updates.is_empty() {
            return;
        }
        let wanted: std::collections::HashMap<i64, &str> = updates
            .iter()
            .map(|(id, recipe)| (*id, recipe.as_str()))
            .collect();
        for photo in self.photos.borrow().iter() {
            if let Some(recipe) = wanted.get(&photo.id()) {
                photo.set_edit_recipe(recipe.to_string());
            }
        }
    }

    /// Re-decode the visible photo after presentation metadata such as the
    /// user's rotation changes. The current full image stays in place until
    /// its correctly rotated replacement is ready.
    pub fn refresh_current(&self) {
        if !self.root.is_visible() || self.photos.borrow().is_empty() {
            return;
        }
        let generation = self.load_generation.get().wrapping_add(1);
        self.load_generation.set(generation);
        show_photo(
            &self.picture,
            &self.photos.borrow(),
            self.index.get(),
            &self.root,
            self.zoom.clone(),
            self.load_generation.clone(),
            generation,
            self.decode_cancel.clone(),
            &self.picture_viewport,
            self.native_texture.clone(),
            self.display_texture_cache.clone(),
            false,
            false,
            None,
            None,
        );
    }
}
