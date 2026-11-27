{
    // The window's one keyboard shortcut controller (candidate 23). It runs
    // in the capture phase, before the focused widget, so shortcuts work
    // whether the grid, a sidebar row or the viewer has focus, and so
    // GtkGridView does not take Space as a selection toggle. Which command a
    // key means is decided by `key_dispatch::key_command`; this adapter
    // gathers the context and carries the command out. The gallery's keys
    // arrive through `gallery_keys_slot`, filled once the gallery exists.
    let window_keys = gtk::EventControllerKey::new();
    window_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let lightbox_for_keys = lightbox.clone();
    let front_for_keys = front.clone();
    let space_open_slot_for_keys = space_open_slot.clone();
    let edit_space_slot_for_keys = edit_space_slot.clone();
    let edit_escape_slot_for_keys = edit_escape_slot.clone();
    let date_browser_back_for_keys = date_browser_back_slot.clone();
    let one_to_one_for_keys = info.one_to_one.clone();
    let space_toggle_in_progress_for_keys = space_toggle_in_progress.clone();
    let search_popup_for_keys = search_popup_slot.clone();
    let search_entry_for_keys = search_entry_slot.clone();
    let window_for_keys = window.clone();
    let immersive_active_for_keys = immersive_active.clone();
    let immersive_toggle_for_keys = immersive_toggle_slot.clone();
    let gallery_keys_for_keys = gallery_keys_slot.clone();
    window_keys.connect_key_pressed(move |_, key, _, modifiers| {
        // Walk the focus chain so both entries (Editable) and the editor's
        // TextView (not Editable) count as typing. An empty search field
        // can keep focus after gallery navigation; Space still opens there.
        let focused = gtk::prelude::RootExt::focus(&window_for_keys);
        let chain = || std::iter::successors(focused.clone(), |widget| widget.parent());
        let typing = if !chain().any(|widget| widget.is::<gtk::Editable>() || widget.is::<gtk::TextView>()) {
            key_dispatch::Typing::No
        } else if chain().any(|widget| {
            widget
                .downcast::<gtk::SearchEntry>()
                .ok()
                .is_some_and(|entry| entry.text().is_empty())
        }) {
            key_dispatch::Typing::EmptySearch
        } else {
            key_dispatch::Typing::Text
        };
        let gallery_keys = gallery_keys_for_keys.borrow().clone();
        let context = key_dispatch::KeyContext {
            front: front_for_keys.current(),
            typing,
            editor_open: edit_escape_slot_for_keys.borrow().is_some(),
            context_menu_open: photo_context_menu_is_open(),
            immersive: immersive_active_for_keys.get(),
            search_suggestions_shown: search_popup_for_keys
                .borrow()
                .as_ref()
                .is_some_and(|popup| popup.is_visible()),
            search_active: search_entry_for_keys
                .borrow()
                .as_ref()
                .is_some_and(|entry| !entry.text().is_empty()),
            focus_in_gallery: gallery_keys
                .as_ref()
                .is_some_and(|keys| (keys.focus_in_gallery)()),
        };
        let handled = |done: bool| {
            if done {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        };
        use key_dispatch::KeyCommand as Command;
        let command = key_dispatch::key_command(key, modifiers, &context);
        match command {
            Command::Pass => glib::Propagation::Proceed,
            Command::Consume => glib::Propagation::Stop,
            Command::EditorEscape => {
                // Clone before invoking: closing clears this slot.
                let escape = edit_escape_slot_for_keys.borrow().clone();
                handled(escape.map(|escape| escape()).is_some())
            }
            Command::DismissContextMenu => {
                dismiss_active_photo_context_menu();
                let root = lightbox_for_keys.root.clone();
                glib::idle_add_local_once(move || {
                    if root.is_visible() {
                        root.grab_focus();
                    }
                });
                glib::Propagation::Stop
            }
            Command::CloseViewer => {
                lightbox_for_keys.request_close();
                glib::Propagation::Stop
            }
            Command::ClearSearch => {
                // The entry's own stop-search: closes the suggestions and
                // clears the text, which rebuilds the destination's view.
                if let Some(entry) = search_entry_for_keys.borrow().as_ref() {
                    entry.emit_stop_search();
                }
                glib::Propagation::Stop
            }
            Command::LeaveImmersive | Command::ToggleImmersive => {
                let toggle = immersive_toggle_for_keys.borrow().as_ref().cloned();
                // Switch widget parenting after key dispatch; GTK may still
                // be snapshotting the gallery during a capture-phase handler.
                handled(toggle.map(|toggle| glib::idle_add_local_once(move || toggle())).is_some())
            }
            Command::DateBrowserBack => handled(
                date_browser_back_for_keys
                    .borrow()
                    .as_ref()
                    .is_some_and(|back| back()),
            ),
            Command::ToggleFullscreen => {
                if window_for_keys.is_fullscreen() {
                    window_for_keys.unfullscreen();
                } else {
                    window_for_keys.fullscreen();
                }
                glib::Propagation::Stop
            }
            Command::ViewerPhoto(direction) => {
                lightbox_for_keys.navigate_photo(direction);
                glib::Propagation::Stop
            }
            Command::ViewerCollection(direction) => {
                lightbox_for_keys.navigate_collection(direction);
                glib::Propagation::Stop
            }
            Command::EditorSpace => {
                // The editor's own Space: Fit / 1:1, a no-op in its Text
                // section. Either way Space must not open the viewer.
                let space = edit_space_slot_for_keys.borrow().clone();
                if let Some(space) = space {
                    space();
                }
                glib::Propagation::Stop
            }
            Command::ViewerToggleOneToOne => {
                space_toggle_in_progress_for_keys.set(true);
                one_to_one_for_keys.set_active(!one_to_one_for_keys.is_active());
                glib::Propagation::Stop
            }
            Command::OpenSelected => {
                if let Some(open_selected) = space_open_slot_for_keys.borrow().as_ref() {
                    open_selected();
                }
                glib::Propagation::Stop
            }
            Command::GalleryBrowse { .. }
            | Command::SelectAll
            | Command::ClearSelection
            | Command::SetRating(_) => {
                handled(gallery_keys.is_some_and(|keys| (keys.run)(command, key)))
            }
        }
    });
    window.add_controller(window_keys);
}
