{
// Included by build.rs after the live window widgets are assembled. This is
// deliberately presentation-only: the existing gallery, lightbox and caches
// stay in place when immersive mode is toggled.
fn immersive_has_popup(widget: &gtk::Widget) -> bool {
    if widget.downcast_ref::<gtk::Popover>().is_some_and(|p| p.is_mapped()) {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if immersive_has_popup(&current) {
            return true;
        }
    }
    false
}

// Top/bottom targets are put OVER the gallery, so revealing them cannot
// recalculate Honeycomb cells or justified Photo Wall rows.
fn immersive_install_chrome_edge(
    surface: &gtk::Overlay,
    panel: &gtk::Widget,
    bottom: bool,
    active: Rc<Cell<bool>>,
) -> gtk::Box {
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    strip.set_height_request(12);
    strip.set_hexpand(true);
    strip.set_halign(gtk::Align::Fill);
    strip.set_valign(if bottom { gtk::Align::End } else { gtk::Align::Start });
    strip.set_visible(false);
    surface.add_overlay(&strip);
    let serial = Rc::new(Cell::new(0_u64));
    let motion = gtk::EventControllerMotion::new();
    {
        let (panel, strip, serial, active) =
            (panel.clone(), strip.clone(), serial.clone(), active.clone());
        motion.connect_enter(move |_, _, _| {
            let token = serial.get().wrapping_add(1);
            serial.set(token);
            let (panel, strip, serial, active) =
                (panel.clone(), strip.clone(), serial.clone(), active.clone());
            glib::timeout_add_local_once(Duration::from_millis(180), move || {
                if active.get() && serial.get() == token {
                    panel.set_visible(true);
                    strip.set_visible(false);
                }
            });
        });
    }
    {
        let serial = serial.clone();
        motion.connect_leave(move |_| serial.set(serial.get().wrapping_add(1)));
    }
    strip.add_controller(motion);
    let motion = gtk::EventControllerMotion::new();
    {
        let serial = serial.clone();
        motion.connect_enter(move |_, _, _| serial.set(serial.get().wrapping_add(1)));
    }
    {
        let (panel, strip, serial, active) =
            (panel.clone(), strip.clone(), serial.clone(), active.clone());
        motion.connect_leave(move |_| {
            let token = serial.get().wrapping_add(1);
            serial.set(token);
            let (panel, strip, serial, active) =
                (panel.clone(), strip.clone(), serial.clone(), active.clone());
            glib::timeout_add_local(Duration::from_millis(700), move || {
                if !active.get() || serial.get() != token {
                    return glib::ControlFlow::Break;
                }
                if immersive_has_popup(&panel) {
                    return glib::ControlFlow::Continue;
                }
                panel.set_visible(false);
                strip.set_visible(true);
                glib::ControlFlow::Break
            });
        });
    }
    panel.add_controller(motion);
    strip
}
{
    // Left-edge reveal geometry for the immersive navigation overlay.
    // The strip is 12px wide and reveals after a short rest, so a pointer
    // that merely crosses the window edge does not flash the sidebar.
    const IMMERSIVE_EDGE_WIDTH: f64 = 12.0;
    const IMMERSIVE_EDGE_REVEAL_DELAY_MS: u64 = 135;
    const IMMERSIVE_SIDEBAR_HIDE_DELAY_MS: u64 = 400;

    // Immersive sidebar slide.
    //
    // The windowed sidebar already animates with libadwaita's spring (damping
    // ratio 1, mass 0.5, stiffness 500) inside AdwOverlaySplitView. The
    // immersive overlay reuses the same spring, so revealing and hiding the
    // navigation feels identical in both layouts.
    //
    // The drawer keeps its existing overlay child, width and full height;
    // only its horizontal position changes. GtkOverlay emits
    // "get-child-position" for overlay children, so the slide writes that
    // allocation. Nothing outside the sidebar is re-measured or re-allocated:
    // the gallery width, its scroll adjustment and every tile position stay
    // untouched. (A GtkRevealer animates the child's width, and an animated
    // AdwOverlaySplitView drawer re-anchors the gallery scroll; neither is
    // usable here.) gtk4-rs 0.10 does not bind this signal, so it is
    // connected through the C API.
    let immersive_slide = Rc::new(Cell::new(0.0_f64));
    let immersive_slide_target = Rc::new(Cell::new(0.0_f64));
    let immersive_slide_animation: Rc<RefCell<Option<adw::SpringAnimation>>> =
        Rc::new(RefCell::new(None));

    struct ImmersiveSlidePosition {
        shell: *mut gtk::ffi::GtkWidget,
        slide: Rc<Cell<f64>>,
        width: Rc<Cell<i32>>,
    }

    #[repr(C)]
    struct ImmersiveAllocation {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }

    unsafe extern "C" fn immersive_slide_position(
        overlay: *mut gtk::ffi::GtkOverlay,
        child: *mut gtk::ffi::GtkWidget,
        allocation: *mut ImmersiveAllocation,
        user_data: glib::ffi::gpointer,
    ) -> glib::ffi::gboolean {
        let state = &*(user_data as *const ImmersiveSlidePosition);
        if child != state.shell {
            // Let GtkOverlay's default handler position every other child.
            return 0;
        }
        let width = state.width.get().max(1);
        let height = gtk::ffi::gtk_widget_get_height(overlay.cast()).max(1);
        let progress = state.slide.get().clamp(0.0, 1.0);
        (*allocation).x = -((f64::from(width) * (1.0 - progress)).round() as i32);
        (*allocation).y = 0;
        (*allocation).width = width;
        (*allocation).height = height;
        1
    }

    unsafe extern "C" fn immersive_slide_destroy(
        data: glib::ffi::gpointer,
        _closure: *mut glib::gobject_ffi::GClosure,
    ) {
        drop(Box::from_raw(data as *mut ImmersiveSlidePosition));
    }

    unsafe {
        let state = Box::new(ImmersiveSlidePosition {
            shell: sidebar_shell.as_ptr().cast(),
            slide: immersive_slide.clone(),
            width: immersive_slide_width.clone(),
        });
        glib::gobject_ffi::g_signal_connect_data(
            gallery_surface.as_ptr().cast(),
            c"get-child-position".as_ptr(),
            Some(std::mem::transmute::<
                unsafe extern "C" fn(
                    *mut gtk::ffi::GtkOverlay,
                    *mut gtk::ffi::GtkWidget,
                    *mut ImmersiveAllocation,
                    glib::ffi::gpointer,
                ) -> glib::ffi::gboolean,
                unsafe extern "C" fn(),
            >(immersive_slide_position)),
            Box::into_raw(state).cast(),
            Some(immersive_slide_destroy),
            glib::gobject_ffi::G_CONNECT_DEFAULT,
        );
    }

    let set_immersive_sidebar_shown: Rc<dyn Fn(bool)> = {
        let shell = sidebar_shell.clone();
        let surface = gallery_surface.clone();
        let slide = immersive_slide.clone();
        let target_value = immersive_slide_target.clone();
        let animation_slot = immersive_slide_animation.clone();
        Rc::new(move |show: bool| {
            let target = if show { 1.0 } else { 0.0 };
            target_value.set(target);
            if show {
                // Entry left the drawer hidden and its slide at 0, so it
                // becomes visible fully off-screen and then slides in.
                shell.set_visible(true);
            }
            if (slide.get() - target).abs() < 1e-6 {
                if !show {
                    shell.set_visible(false);
                }
                return;
            }
            let animation = {
                let mut slot = animation_slot.borrow_mut();
                if slot.is_none() {
                    let surface_for_target = surface.clone();
                    let slide_for_target = slide.clone();
                    let shell_for_target = shell.clone();
                    let target_for_target = target_value.clone();
                    let callback = adw::CallbackAnimationTarget::new(move |value| {
                        slide_for_target.set(value);
                        // Re-run the overlay allocation so
                        // "get-child-position" writes the new x. Only the
                        // overlay and the sidebar are re-allocated; the
                        // gallery keeps its allocation and scroll position.
                        surface_for_target.queue_allocate();
                        // Drop the off-screen drawer once the slide-out has
                        // settled so it cannot pick or paint anymore.
                        if target_for_target.get() <= 0.0 && value <= 1e-4 {
                            shell_for_target.set_visible(false);
                        }
                    });
                    *slot = Some(adw::SpringAnimation::new(
                        &surface,
                        slide.get(),
                        target,
                        adw::SpringParams::new(1.0, 0.5, 500.0),
                        callback,
                    ));
                }
                slot.as_ref().expect("animation was just created").clone()
            };
            animation.set_value_from(slide.get());
            animation.set_value_to(target);
            // Match the windowed drawer: clamp while closing, allow the
            // spring's natural settle while opening.
            animation.set_clamp(!show);
            animation.play();
        })
    };
    let show_immersive_sidebar = {
        let setter = set_immersive_sidebar_shown.clone();
        Rc::new(move || setter(true))
    };
    let hide_immersive_sidebar = {
        let setter = set_immersive_sidebar_shown.clone();
        Rc::new(move || setter(false))
    };

    // Immersive Mode owns the left navigation for the whole session.
    //
    // `immersive_sidebar_shown` is the single authority for the temporary
    // overlay: the responsive breakpoint, the compact rail and the persistent
    // pin flag all defer to it. The saved Compact UI preference is never
    // written while an immersive session is active.
    //
    // The edge detection lives on the window, not on the overlay strip.
    // GTK picks overlay children for rendering, but pointer events are
    // delivered to the widget under the pointer, and an overlay child never
    // becomes that target. A window-level controller sees every motion
    // event of the window, so the edge area is detected reliably in the
    // windowed and in the F11 fullscreen presentation alike. The same
    // controller owns the auto-hide: the sidebar overlay can appear under a
    // stationary pointer, and GTK then never emits the enter/leave pair the
    // sidebar shell would need to notice that the pointer went away.
    let immersive_edge_motion = gtk::EventControllerMotion::new();
    let immersive_edge_serial = Rc::new(Cell::new(0_u64));
    let immersive_edge_armed = Rc::new(Cell::new(false));
    let immersive_edge_pointer_x = Rc::new(Cell::new(f64::MAX));
    {
        // Width of the overlay sidebar in window coordinates. Immersive
        // reveals the existing sidebar shell as a static presentation
        // overlay, so its own allocation is the authoritative width.
        fn overlay_width(shell: &gtk::Overlay, split: &adw::OverlaySplitView) -> f64 {
            let allocated = shell.width();
            if allocated > 0 {
                f64::from(allocated)
            } else {
                split.max_sidebar_width()
            }
        }
        let (split, shell, active, shown, column) = (
            main_split.clone(),
            sidebar_shell.clone(),
            immersive_active.clone(),
            immersive_sidebar_shown.clone(),
            left_column.clone(),
        );
        let (serial, armed, pointer_x) = (
            immersive_edge_serial.clone(),
            immersive_edge_armed.clone(),
            immersive_edge_pointer_x.clone(),
        );
        // Hide the temporary overlay once the pointer left it. The pointer
        // position is re-checked when the timer fires, so a pointer that
        // returned to the sidebar keeps it open.
        let schedule_hide: Rc<dyn Fn()> = {
            let (split, shell, active, shown, hide_sidebar) = (
                split.clone(),
                shell.clone(),
                active.clone(),
                shown.clone(),
                hide_immersive_sidebar.clone(),
            );
            let (serial, armed, pointer_x) = (
                serial.clone(),
                armed.clone(),
                pointer_x.clone(),
            );
            Rc::new(move || {
                if !active.get() || !shown.get() || armed.get() {
                    return;
                }
                armed.set(true);
                let token = serial.get().wrapping_add(1);
                serial.set(token);
                let (split, shell, active, shown, hide_sidebar) = (
                    split.clone(),
                    shell.clone(),
                    active.clone(),
                    shown.clone(),
                    hide_sidebar.clone(),
                );
                let (serial, armed, pointer_x) = (
                    serial.clone(),
                    armed.clone(),
                    pointer_x.clone(),
                );
                glib::timeout_add_local_once(
                    Duration::from_millis(IMMERSIVE_SIDEBAR_HIDE_DELAY_MS),
                    move || {
                        if !active.get() || !shown.get() || serial.get() != token {
                            return;
                        }
                        armed.set(false);
                        // The pointer must still be outside the sidebar.
                        if pointer_x.get() < overlay_width(&shell, &split) {
                            return;
                        }
                        // Slide the presentation overlay out. Moving a fixed
                        // child cannot re-allocate the gallery or move its
                        // scroll adjustment.
                        shown.set(false);
                        hide_sidebar();
                    },
                );
            })
        };
        let schedule_hide_for_motion = schedule_hide.clone();
        // The pointer can leave the whole window, for example towards a
        // popover of the sidebar. A visible popup keeps the overlay open.
        {
            let (active, shown, column, schedule_hide) = (
                active.clone(),
                shown.clone(),
                column.clone(),
                schedule_hide.clone(),
            );
            immersive_edge_motion.connect_leave(move |_| {
                if !active.get() || !shown.get() {
                    return;
                }
                if immersive_has_popup(column.upcast_ref()) {
                    // The pointer moved into a popup of the sidebar.
                    return;
                }
                schedule_hide();
            });
        }
        let reveal_sidebar_for_motion = show_immersive_sidebar.clone();
        let slide_for_motion = immersive_slide.clone();
        immersive_edge_motion.connect_motion(move |_, x, _| {
            if !active.get() {
                return;
            }
            pointer_x.set(x);
            if shown.get() {
                // The overlay is open. A pointer inside it keeps it open; a
                // pointer outside it starts the auto-hide timer.
                if x < overlay_width(&shell, &split) {
                    serial.set(serial.get().wrapping_add(1));
                    armed.set(false);
                    return;
                }
                if immersive_has_popup(column.upcast_ref()) {
                    // A context menu or popover owns the pointer: keep the
                    // sidebar open until the popup closes.
                    serial.set(serial.get().wrapping_add(1));
                    armed.set(false);
                    return;
                }
                schedule_hide_for_motion();
                return;
            }
            if slide_for_motion.get() > 0.0
                && x < overlay_width(&shell, &split) * slide_for_motion.get().clamp(0.0, 1.0)
            {
                serial.set(serial.get().wrapping_add(1));
                armed.set(false);
                shown.set(true);
                reveal_sidebar_for_motion();
                return;
            }
            if x > IMMERSIVE_EDGE_WIDTH {
                // The pointer left the edge area again: cancel a pending
                // reveal and disarm until the pointer comes back.
                serial.set(serial.get().wrapping_add(1));
                armed.set(false);
                return;
            }
            if armed.get() {
                return;
            }
            armed.set(true);
            let token = serial.get().wrapping_add(1);
            serial.set(token);
            let (split, active, shown, reveal_sidebar) = (
                split.clone(),
                active.clone(),
                shown.clone(),
                reveal_sidebar_for_motion.clone(),
            );
            let (serial, armed, pointer_x) = (
                serial.clone(),
                armed.clone(),
                pointer_x.clone(),
            );
            glib::timeout_add_local_once(
                Duration::from_millis(IMMERSIVE_EDGE_REVEAL_DELAY_MS),
                move || {
                    if !active.get() || shown.get() || serial.get() != token {
                        return;
                    }
                    armed.set(false);
                    // The pointer must still rest inside the edge area.
                    if pointer_x.get() > IMMERSIVE_EDGE_WIDTH {
                        return;
                    }
                    // Slide the existing full sidebar in as a temporary
                    // overlay. It lives in a fixed presentation host, so
                    // moving it cannot re-allocate the gallery or move its
                    // scroll adjustment.
                    shown.set(true);
                    split.set_collapsed(true);
                    reveal_sidebar();
                },
            );
        });
    }
    window.add_controller(immersive_edge_motion);

    let immersive_button = gtk::Button::from_icon_name("view-fullscreen-symbolic");
    immersive_button.add_css_class("flat");
    immersive_button.add_css_class("pic-header-action");
    immersive_button.set_tooltip_text(Some("Immersive Mode (I)"));
    right_header.pack_end(&immersive_button);

    let top_strip = immersive_install_chrome_edge(
        &gallery_surface, right_header.upcast_ref(), false, immersive_active.clone()
    );
    let bottom_strip = immersive_install_chrome_edge(
        &gallery_surface, info.root.upcast_ref(), true, immersive_active.clone()
    );

    // Immersive has no right-edge inspector. Guard every existing toggle path.
    {
        let active = immersive_active.clone();
        properties_split.connect_show_sidebar_notify(move |split| {
            if active.get() && split.shows_sidebar() {
                split.set_show_sidebar(false);
            }
        });
    }
    {
        let active = immersive_active.clone();
        lightbox_properties_panel.connect_visible_notify(move |panel| {
            if active.get() && panel.is_visible() {
                panel.set_visible(false);
            }
        });
    }

    let left_was_open = Rc::new(Cell::new(false));
    let right_was_open = Rc::new(Cell::new(false));
    let viewer_right_was_open = Rc::new(Cell::new(false));
    let metadata_was_visible = Rc::new(Cell::new(false));
    let left_was_collapsed = Rc::new(Cell::new(false));
    let right_was_collapsed = Rc::new(Cell::new(false));
    // The persistent sidebar pin and the split's own gestures describe the
    // windowed layout. They are captured on entry and restored on exit so an
    // immersive session never leaks its presentation into the saved state.
    let left_was_pinned = Rc::new(Cell::new(true));
    let left_was_split_pinned = Rc::new(Cell::new(true));
    let left_was_show_gesture = Rc::new(Cell::new(false));
    let left_was_hide_gesture = Rc::new(Cell::new(false));
    let toggle: Rc<dyn Fn()> = {
        let (active, split, properties, viewer_panel) = (
            immersive_active.clone(), main_split.clone(), properties_split.clone(),
            lightbox_properties_panel.clone()
        );
        let (left_column, sidebar, expanded, dock, left_edge) = (
            left_column.clone(), sidebar.clone(), sidebar_expanded.clone(),
            compact_dock.clone(), sidebar_hover_reveal.clone()
        );
        let shell = sidebar_shell.clone();
        let (header, column, surface, info_root, metadata_root) = (
            right_header.clone(), right_column.clone(), gallery_surface.clone(),
            info.root.clone(), metadata_panel.root.clone()
        );
        let (top_strip, bottom_strip, button, lightbox, info_for_mode) = (
            top_strip.clone(), bottom_strip.clone(),
            immersive_button.clone(), lightbox.clone(), info.clone()
        );
        let (left_was_open, right_was_open, viewer_right_was_open, metadata_was_visible) = (
            left_was_open.clone(), right_was_open.clone(),
            viewer_right_was_open.clone(), metadata_was_visible.clone()
        );
        let (left_was_collapsed, right_was_collapsed) = (
            left_was_collapsed.clone(), right_was_collapsed.clone()
        );
        let (left_was_pinned, left_was_split_pinned) = (
            left_was_pinned.clone(), left_was_split_pinned.clone()
        );
        let (left_was_show_gesture, left_was_hide_gesture) = (
            left_was_show_gesture.clone(), left_was_hide_gesture.clone()
        );
        let immersive_shown = immersive_sidebar_shown.clone();
        let resize_handle = sidebar_shell.last_child().expect("sidebar resize handle");
        Rc::new(move || {
            let entering = !active.get();
            active.set(entering);
            if entering {
                left_was_open.set(split.shows_sidebar());
                right_was_open.set(properties.shows_sidebar());
                viewer_right_was_open.set(viewer_panel.is_visible());
                metadata_was_visible.set(metadata_root.is_visible());
                left_was_collapsed.set(split.is_collapsed());
                right_was_collapsed.set(properties.is_collapsed());
                left_was_pinned.set(sidebar::is_pinned(&sidebar));
                left_was_split_pinned.set(split.is_pin_sidebar());
                left_was_show_gesture.set(split.enables_show_gesture());
                left_was_hide_gesture.set(split.enables_hide_gesture());

                // An immersive session owns the left navigation completely.
                // The compact rail and the full sidebar are both hidden, the
                // split stays collapsed so the sidebar can only appear as an
                // overlay over the photos, and the saved Compact UI setting is
                // left untouched.
                immersive_shown.set(false);
                dock.set_visible(false);
                resize_handle.set_visible(true);
                left_column.set_visible(true);
                sidebar.set_visible(true);
                split.set_show_sidebar(false);
                split.set_pin_sidebar(false);
                split.set_enable_show_gesture(false);
                split.set_enable_hide_gesture(false);
                split.set_collapsed(true);
                // A collapsed-notify handler may change the sidebar state.
                split.set_show_sidebar(false);
                // The immersive reveal is a presentation-only overlay: the
                // existing sidebar shell leaves the split and becomes an
                // overlay child of the gallery surface. Its position is
                // animated by the "get-child-position" handler, which only
                // re-allocates the sidebar, so the gallery allocation, its
                // scroll adjustment and every tile stay put. The sidebar
                // model, its widget tree and the gallery are untouched;
                // only the presentation parent changes.
                let overlay_width = split
                    .sidebar()
                    .map(|shell| shell.width())
                    .filter(|width| *width > 0)
                    .map(f64::from)
                    .unwrap_or_else(|| split.max_sidebar_width());
                split.set_sidebar(None::<&gtk::Widget>);
                immersive_normal_width.set(overlay_width.round().max(1.0) as i32);
                immersive_slide_width.set(immersive_normal_width.get());
                shell.set_width_request(immersive_slide_width.get());
                shell.set_hexpand(false);
                shell.set_halign(gtk::Align::Start);
                shell.set_valign(gtk::Align::Fill);
                shell.set_vexpand(true);
                // Every session starts fully hidden and off-screen.
                if let Some(animation) = immersive_slide_animation.borrow().as_ref() {
                    animation.pause();
                }
                immersive_slide.set(0.0);
                immersive_slide_target.set(0.0);
                shell.set_visible(false);
                properties.set_show_sidebar(false);
                properties.set_pin_sidebar(false);
                properties.set_collapsed(true);
                properties.set_show_sidebar(false);
                viewer_panel.set_visible(false);
                info_for_mode.set_properties_enabled(false);
                metadata_root.set_visible(false);
                left_edge.set_visible(true);

                column.remove(&header);
                header.set_visible(false);
                header.set_halign(gtk::Align::Fill);
                header.set_valign(gtk::Align::Start);
                surface.add_overlay(&header);
                column.remove(&info_root);
                info_root.set_visible(false);
                info_root.set_halign(gtk::Align::Fill);
                info_root.set_valign(gtk::Align::End);
                surface.add_overlay(&info_root);
                // Added last so the overlay paints above the gallery chrome,
                // exactly like the split's own collapsed drawer did.
                surface.add_overlay(&shell);
                compact_immersive_toggle.set_visible(true);

                top_strip.set_visible(true);
                bottom_strip.set_visible(true);
                button.set_icon_name("view-restore-symbolic");
                button.set_tooltip_text(Some("Exit Immersive Mode (I / Esc)"));
            } else {
                top_strip.set_visible(false);
                bottom_strip.set_visible(false);
                // Leave the immersive overlay before restoring the layout.
                immersive_shown.set(false);
                split.set_show_sidebar(false);
                // Return the presentation overlay to the split before the
                // windowed layout is restored, and drop every presentation
                // override so the drawer allocates exactly as before.
                if let Some(animation) = immersive_slide_animation.borrow().as_ref() {
                    animation.pause();
                }
                immersive_slide.set(0.0);
                immersive_slide_target.set(0.0);
                // Move the compact page back to its original windowed dock.
                compact_immersive_toggle.set_visible(false);
                immersive_sidebar_page(true);
                resize_handle.set_visible(expanded.get());
                surface.remove_overlay(&shell);
                shell.set_width_request(-1);
                shell.set_visible(true);
                shell.set_hexpand(true);
                shell.set_halign(gtk::Align::Fill);
                shell.set_valign(gtk::Align::Fill);
                split.set_sidebar(Some(&shell));
                properties.set_show_sidebar(false);
                viewer_panel.set_visible(false);
                sidebar::clear_hover_open(&sidebar);
                sidebar::set_pinned(&sidebar, left_was_pinned.get());
                split.set_collapsed(left_was_collapsed.get());
                properties.set_collapsed(right_was_collapsed.get());
                split.set_pin_sidebar(left_was_split_pinned.get());
                split.set_enable_show_gesture(left_was_show_gesture.get());
                split.set_enable_hide_gesture(left_was_hide_gesture.get());
                properties.set_pin_sidebar(true);
                info_for_mode.set_properties_enabled(true);

                surface.remove_overlay(&header);
                column.prepend(&header);
                header.set_visible(true);
                surface.remove_overlay(&info_root);
                column.append(&info_root);
                info_root.set_visible(true);
                metadata_root.set_visible(metadata_was_visible.get());

                // Restore the exact sidebar configuration that was active
                // before the session: the compact rail in compact mode, the
                // full sidebar in normal mode.
                left_column.set_visible(expanded.get());
                sidebar.set_visible(expanded.get());
                dock.set_visible(!expanded.get());
                split.set_show_sidebar(left_was_open.get() && expanded.get());
                properties.set_show_sidebar(right_was_open.get());
                if lightbox.root.is_visible() {
                    viewer_panel.set_visible(viewer_right_was_open.get());
                }
                left_edge.set_visible(expanded.get() && !split.shows_sidebar());
                button.set_icon_name("view-fullscreen-symbolic");
                button.set_tooltip_text(Some("Immersive Mode (I)"));
            }
        })
    };
    {
        let toggle = toggle.clone();
        immersive_button.connect_clicked(move |_| {
            // Reparenting the clicked header during signal dispatch can cause
            // GTK allocation warnings. Perform the transition after dispatch.
            let toggle = toggle.clone();
            glib::idle_add_local_once(move || toggle());
        });
    }
    // A window action exposes the same transition to the automated GUI
    // harness, which drives PIC through its accessibility interface.
    {
        let toggle = toggle.clone();
        let action = gio::ActionEntry::builder("immersive")
            .activate(move |_, _, _| {
                let toggle = toggle.clone();
                glib::idle_add_local_once(move || toggle());
            })
            .build();
        window.add_action_entries([action]);
    }
    // Actions for the automated GUI harness: reveal/hide the immersive
    // sidebar without synthesising pointer motion. They drive the exact same
    // slide as the left-edge hover.
    for (name, show) in [
        ("immersive-show-sidebar", true),
        ("immersive-hide-sidebar", false),
    ] {
        let active = immersive_active.clone();
        let shown = immersive_sidebar_shown.clone();
        let split = main_split.clone();
        let slide: Rc<dyn Fn()> = if show {
            show_immersive_sidebar.clone()
        } else {
            hide_immersive_sidebar.clone()
        };
        let action = gio::ActionEntry::builder(name)
            .activate(move |_, _, _| {
                if !active.get() {
                    return;
                }
                shown.set(show);
                if show {
                    split.set_collapsed(true);
                }
                slide();
            })
            .build();
        window.add_action_entries([action]);
    }
    immersive_toggle_slot.replace(Some(toggle.clone()));
    // The filter toolbar manages its own visibility when changing views.
    // Keep it suppressed for the whole immersive session, not just entry.
    {
        let active = immersive_active.clone();
        metadata_panel.root.connect_visible_notify(move |root| {
            if active.get() && root.is_visible() {
                root.set_visible(false);
            }
        });
    }
    {
        let active = immersive_active.clone();
        main_stack.connect_visible_child_notify(move |stack| {
            if active.get() && !matches!(
                stack.visible_child_name().as_deref(),
                Some("photos" | "library" | "albums")
            ) {
                if let Some(toggle) = immersive_toggle_slot.borrow().as_ref().cloned() {
                    let active = active.clone();
                    // Avoid reparenting chrome during Stack::notify::visible-child.
                    glib::idle_add_local_once(move || {
                        if active.get() { toggle(); }
                    });
                }
            }
        });
    }
}
}
