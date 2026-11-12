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

    let right_strip = gtk::Box::new(gtk::Orientation::Vertical, 0);
    right_strip.set_width_request(12);
    right_strip.set_vexpand(true);
    right_strip.set_halign(gtk::Align::End);
    right_strip.set_valign(gtk::Align::Fill);
    right_strip.set_visible(false);
    gallery_surface.add_overlay(&right_strip);
    let right_serial = Rc::new(Cell::new(0_u64));
    let motion = gtk::EventControllerMotion::new();
    {
        let (active, serial, strip, split, viewer, viewer_panel) = (
            immersive_active.clone(), right_serial.clone(), right_strip.clone(),
            properties_split.clone(), lightbox.root.clone(),
            lightbox_properties_panel.clone()
        );
        motion.connect_enter(move |_, _, _| {
            let token = serial.get().wrapping_add(1);
            serial.set(token);
            let (active, serial, strip, split, viewer, viewer_panel) = (
                active.clone(), serial.clone(), strip.clone(), split.clone(),
                viewer.clone(), viewer_panel.clone()
            );
            glib::timeout_add_local_once(Duration::from_millis(180), move || {
                if !active.get() || serial.get() != token { return; }
                if viewer.is_visible() {
                    viewer_panel.set_visible(true);
                } else {
                    split.set_show_sidebar(true);
                }
                strip.set_visible(false);
            });
        });
    }
    {
        let serial = right_serial.clone();
        motion.connect_leave(move |_| serial.set(serial.get().wrapping_add(1)));
    }
    right_strip.add_controller(motion);

    for panel in [
        properties_shell.clone().upcast::<gtk::Widget>(),
        lightbox_properties_panel.clone().upcast::<gtk::Widget>(),
    ] {
        let motion = gtk::EventControllerMotion::new();
        {
            let serial = right_serial.clone();
            motion.connect_enter(move |_, _, _| serial.set(serial.get().wrapping_add(1)));
        }
        {
            let (active, serial, strip, split, viewer, viewer_panel) = (
                immersive_active.clone(), right_serial.clone(), right_strip.clone(),
                properties_split.clone(), lightbox.root.clone(),
                lightbox_properties_panel.clone()
            );
            motion.connect_leave(move |_| {
                let token = serial.get().wrapping_add(1);
                serial.set(token);
                let (active, serial, strip, split, viewer, viewer_panel, panel) = (
                    active.clone(), serial.clone(), strip.clone(), split.clone(),
                    viewer.clone(), viewer_panel.clone(), panel.clone()
                );
                glib::timeout_add_local(Duration::from_millis(700), move || {
                    if !active.get() || serial.get() != token {
                        return glib::ControlFlow::Break;
                    }
                    if immersive_has_popup(&panel) {
                        return glib::ControlFlow::Continue;
                    }
                    if viewer.is_visible() {
                        viewer_panel.set_visible(false);
                    } else {
                        split.set_show_sidebar(false);
                    }
                    strip.set_visible(true);
                    glib::ControlFlow::Break
                });
            });
        }
        panel.add_controller(motion);
    }
    // The existing Properties button may also toggle the panel while
    // immersive. Keep the edge target synchronized in that case.
    {
        let (active, strip, viewer) =
            (immersive_active.clone(), right_strip.clone(), lightbox.root.clone());
        properties_split.connect_show_sidebar_notify(move |split| {
            if active.get() && !viewer.is_visible() {
                strip.set_visible(!split.shows_sidebar());
            }
        });
    }
    {
        let (active, strip, viewer) =
            (immersive_active.clone(), right_strip.clone(), lightbox.root.clone());
        lightbox_properties_panel.connect_visible_notify(move |panel| {
            if active.get() && viewer.is_visible() {
                strip.set_visible(!panel.is_visible());
            }
        });
    }
    {
        let (active, strip, split, panel) = (
            immersive_active.clone(), right_strip.clone(), properties_split.clone(),
            lightbox_properties_panel.clone()
        );
        lightbox.root.connect_visible_notify(move |viewer| {
            if active.get() {
                strip.set_visible(if viewer.is_visible() {
                    !panel.is_visible()
                } else {
                    !split.shows_sidebar()
                });
            }
        });
    }

    let left_was_open = Rc::new(Cell::new(false));
    let right_was_open = Rc::new(Cell::new(false));
    let viewer_right_was_open = Rc::new(Cell::new(false));
    let metadata_was_visible = Rc::new(Cell::new(false));
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
        let (top_strip, bottom_strip, right_strip, button, lightbox) = (
            top_strip.clone(), bottom_strip.clone(), right_strip.clone(),
            immersive_button.clone(), lightbox.clone()
        );
        let (left_was_open, right_was_open, viewer_right_was_open, metadata_was_visible) = (
            left_was_open.clone(), right_was_open.clone(),
            viewer_right_was_open.clone(), metadata_was_visible.clone()
        );
        Rc::new(move || {
            let entering = !active.get();
            active.set(entering);
            if entering {
                left_was_open.set(split.shows_sidebar());
                right_was_open.set(properties.shows_sidebar());
                viewer_right_was_open.set(viewer_panel.is_visible());
                metadata_was_visible.set(metadata_root.is_visible());

                dock.set_visible(false);
                left_column.set_visible(true);
                sidebar.set_visible(true);
                split.set_pin_sidebar(false);
                split.set_show_sidebar(false);
                properties.set_pin_sidebar(false);
                properties.set_show_sidebar(false);
                viewer_panel.set_visible(false);
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
                right_strip.set_visible(true);
                button.set_icon_name("view-restore-symbolic");
                button.set_tooltip_text(Some("Exit Immersive Mode (I / Esc)"));
            } else {
                top_strip.set_visible(false);
                bottom_strip.set_visible(false);
                right_strip.set_visible(false);
                split.set_show_sidebar(false);
                properties.set_show_sidebar(false);
                viewer_panel.set_visible(false);
                sidebar::clear_hover_open(&sidebar);
                split.set_pin_sidebar(true);
                properties.set_pin_sidebar(true);

                surface.remove_overlay(&header);
                column.prepend(&header);
                header.set_visible(true);
                surface.remove_overlay(&info_root);
                column.append(&info_root);
                info_root.set_visible(true);
                metadata_root.set_visible(metadata_was_visible.get());

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
        immersive_button.connect_clicked(move |_| toggle());
    }
    immersive_toggle_slot.replace(Some(toggle.clone()));
    {
        let active = immersive_active.clone();
        main_stack.connect_visible_child_notify(move |stack| {
            if active.get() && !matches!(
                stack.visible_child_name().as_deref(),
                Some("photos" | "library" | "albums")
            ) {
                if let Some(toggle) = immersive_toggle_slot.borrow().as_ref() {
                    toggle();
                }
            }
        });
    }
}
