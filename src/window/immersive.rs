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
    const IMMERSIVE_EDGE_REVEAL_DELAY_MS: u64 = 180;
    const IMMERSIVE_SIDEBAR_HIDE_DELAY_MS: u64 = 700;

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
        // Width of the overlay sidebar in window coordinates. The split is
        // collapsed while immersive, so the sidebar sits at the very left
        // edge of the window and covers this many pixels.
        fn overlay_width(split: &adw::OverlaySplitView) -> f64 {
            let allocated = split.sidebar().map(|sidebar| sidebar.width()).unwrap_or(0);
            if allocated > 0 {
                f64::from(allocated)
            } else {
                split.max_sidebar_width()
            }
        }
        let (split, active, shown, column) = (
            main_split.clone(),
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
            let (split, active, shown) = (split.clone(), active.clone(), shown.clone());
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
                let (split, active, shown) = (split.clone(), active.clone(), shown.clone());
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
                        if pointer_x.get() < overlay_width(&split) {
                            return;
                        }
                        shown.set(false);
                        split.set_show_sidebar(false);
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
        immersive_edge_motion.connect_motion(move |_, x, _| {
            if !active.get() {
                return;
            }
            pointer_x.set(x);
            if shown.get() {
                // The overlay is open. A pointer inside it keeps it open; a
                // pointer outside it starts the auto-hide timer.
                if x < overlay_width(&split) {
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
            let (split, active, shown) = (split.clone(), active.clone(), shown.clone());
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
                    // Reveal the existing full sidebar as a temporary overlay.
                    // The split stays collapsed, so the gallery keeps its
                    // width and the photos are not re-laid out.
                    shown.set(true);
                    split.set_collapsed(true);
                    split.set_show_sidebar(true);
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
                left_column.set_visible(true);
                sidebar.set_visible(true);
                split.set_show_sidebar(false);
                split.set_pin_sidebar(false);
                split.set_enable_show_gesture(false);
                split.set_enable_hide_gesture(false);
                split.set_collapsed(true);
                // A collapsed-notify handler may change the sidebar state.
                split.set_show_sidebar(false);
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
