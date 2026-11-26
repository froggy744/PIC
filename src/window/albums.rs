fn selected_photo_ids(context: &PhotoActionContext, fallback_id: Option<i64>) -> Vec<i64> {
    context
        .gallery
        .borrow()
        .upgrade()
        .map(|gallery| gallery.selected_photo_ids(fallback_id))
        .unwrap_or_else(|| fallback_id.into_iter().collect())
}

fn refresh_album_ui(context: &PhotoActionContext) {
    let folders = db::folders(&context.connection.borrow()).unwrap_or_default();
    let albums = db::albums(&context.connection.borrow()).unwrap_or_default();
    let counts = db::sidebar_counts(&context.connection.borrow()).unwrap_or_default();
    if let Some(sidebar) = context.sidebar.borrow().as_ref() {
        sidebar::refresh(
            sidebar,
            &folders,
            &albums,
            counts,
            context.create_album.clone(),
            context.import_folder.clone(),
            context.delete_album.clone(),
            context.on_unavailable.clone(),
        );
    }
    configure_infobar_album_menu(&context.info.add_to_album, context.clone());
    (context.refresh_albums_home)(&albums);
}

fn configure_infobar_album_menu(button: &gtk::MenuButton, context: PhotoActionContext) {
    let selection_context = context.clone();
    let selected_ids: Rc<dyn Fn() -> Vec<i64>> = Rc::new(move || {
        let fallback = selection_context
            .info
            .current_photo()
            .map(|photo| photo.id());
        selected_photo_ids(&selection_context, fallback)
    });
    button.set_popover(Some(&build_album_popover(
        context,
        selected_ids,
        Rc::new(|| {}),
        Rc::new(|| {}),
    )));
}

fn build_album_popover(
    context: PhotoActionContext,
    selected_ids: Rc<dyn Fn() -> Vec<i64>>,
    dismiss_parent: Rc<dyn Fn()>,
    after_existing_album: Rc<dyn Fn()>,
) -> gtk::Popover {
    let popover = gtk::Popover::new();
    let weak_popover = popover.downgrade();
    let dismiss: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(popover) = weak_popover.upgrade() {
            popover.popdown();
        }
        dismiss_parent();
    });
    popover.set_child(Some(&build_album_choices(
        context,
        selected_ids,
        dismiss,
        after_existing_album,
    )));
    popover
}

fn show_album_chooser(
    parent: &gtk::Widget,
    context: PhotoActionContext,
    selected_ids: Rc<dyn Fn() -> Vec<i64>>,
    after_existing_album: Rc<dyn Fn()>,
) {
    let dialog = adw::AlertDialog::builder()
        .heading("Add to Album")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", "Cancel");
    let weak_dialog = dialog.downgrade();
    let dismiss: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(dialog) = weak_dialog.upgrade() {
            dialog.close();
        }
    });
    let choices = build_album_choices(context, selected_ids, dismiss, after_existing_album);
    dialog.set_extra_child(Some(&choices));
    dialog.present(Some(parent));
}

fn build_album_choices(
    context: PhotoActionContext,
    selected_ids: Rc<dyn Fn() -> Vec<i64>>,
    dismiss: Rc<dyn Fn()>,
    after_existing_album: Rc<dyn Fn()>,
) -> gtk::ScrolledWindow {
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
    menu.set_margin_top(6);
    menu.set_margin_bottom(6);
    menu.set_margin_start(6);
    menu.set_margin_end(6);

    let new_album = gtk::Button::with_label("New Album…");
    new_album.set_halign(gtk::Align::Fill);
    new_album.add_css_class("flat");
    let new_context = context.clone();
    let new_selection = selected_ids.clone();
    let dismiss_for_new = dismiss.clone();
    new_album.connect_clicked(move |_| {
        dismiss_for_new();
        let parent: gtk::Widget = new_context.info.root.clone().upcast();
        let photo_ids = new_selection();
        let context = new_context.clone();
        // Present after the chooser has closed and released its focus.
        glib::idle_add_local_once(move || {
            show_create_album_dialog(parent, photo_ids, context);
        });
    });
    menu.append(&new_album);

    // Destinations only: Smart Albums are read-only photo sources.
    let albums = db::manual_albums(&context.connection.borrow()).unwrap_or_default();
    if !albums.is_empty() {
        menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    }
    for album in albums {
        let item = gtk::Button::with_label(&album.name);
        item.set_halign(gtk::Align::Fill);
        item.add_css_class("flat");
        let item_context = context.clone();
        let item_selection = selected_ids.clone();
        let dismiss_for_item = dismiss.clone();
        let after_existing_album_for_item = after_existing_album.clone();
        item.connect_clicked(move |_| {
            dismiss_for_item();
            let ids = item_selection();
            if ids.is_empty() {
                return;
            }
            if let Err(error) =
                db::add_photos_to_album(&item_context.connection.borrow(), album.id, &ids)
            {
                show_error(
                    item_context.info.root.upcast_ref(),
                    "Could not add to album",
                    &error.to_string(),
                );
                return;
            }
            refresh_album_ui(&item_context);
            after_existing_album_for_item();
        });
        menu.append(&item);
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_propagate_natural_width(true);
    scroll.set_propagate_natural_height(true);
    scroll.set_max_content_height(360);
    scroll.set_child(Some(&menu));
    scroll
}

fn show_create_album_dialog(parent: gtk::Widget, photo_ids: Vec<i64>, context: PhotoActionContext) {
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Album name"));
    entry.set_activates_default(true);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    // Album or Smart Album. Opened from Add to Album the photos need a
    // manual album, so the choice is not offered there.
    let smart = gtk::ToggleButton::with_label("Smart Album");
    if photo_ids.is_empty() {
        let kind = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        kind.add_css_class("linked");
        kind.add_css_class("new-album-kind");
        kind.set_halign(gtk::Align::Center);
        kind.set_margin_bottom(4);
        let manual = gtk::ToggleButton::with_label("Album");
        manual.set_active(true);
        smart.set_group(Some(&manual));
        manual.set_hexpand(true);
        smart.set_hexpand(true);
        kind.set_homogeneous(true);
        kind.append(&manual);
        kind.append(&smart);
        content.append(&kind);
    }
    content.append(&gtk::Label::new(Some("Enter a name for the album:")));
    content.append(&entry);
    let dialog = adw::AlertDialog::builder()
        .heading("New Album")
        .extra_child(&content)
        .default_response("create")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    let weak_dialog = dialog.downgrade();
    smart.connect_toggled(move |smart| {
        if let Some(dialog) = weak_dialog.upgrade() {
            dialog.set_response_label("create", if smart.is_active() { "Next" } else { "Create" });
        }
    });
    let parent_for_response = parent.clone();
    let entry_for_response = entry.clone();
    dialog.connect_response(Some("create"), move |_, _| {
        if smart.is_active() {
            let name = entry_for_response.text().trim().to_string();
            if let Err(error) = db::check_new_album_name(&context.connection.borrow(), &name) {
                show_error(&parent_for_response, "Could not create album", &error.to_string());
                return;
            }
            let parent = parent_for_response.clone();
            let context = context.clone();
            // Present after the name dialog has closed and released its focus.
            glib::idle_add_local_once(move || {
                show_new_smart_album_rules(&parent, name, None, None, context);
            });
            return;
        }
        let album = match db::create_album(
            &context.connection.borrow(),
            entry_for_response.text().as_str(),
        ) {
            Ok(album) => album,
            Err(error) => {
                show_error(
                    &parent_for_response,
                    "Could not create album",
                    &error.to_string(),
                );
                return;
            }
        };
        if !photo_ids.is_empty() {
            if let Err(error) =
                db::add_photos_to_album(&context.connection.borrow(), album.id, &photo_ids)
            {
                show_error(
                    &parent_for_response,
                    "Could not add to album",
                    &error.to_string(),
                );
            }
        }
        refresh_album_ui(&context);
    });
    dialog.present(Some(&parent));
    entry.grab_focus();
}

/// The second step of a new Smart Album (and Save as Smart Album): choose
/// its rules, then create it. `name` is `None` when the dialog should ask for
/// one; `rules` prefills the conditions.
fn show_new_smart_album_rules(
    parent: &gtk::Widget,
    name: String,
    rules: Option<&db::filters::AlbumRules>,
    notice: Option<String>,
    context: PhotoActionContext,
) {
    let ask_name = rules.is_some();
    let connection = context.connection.clone();
    crate::albums_view::rules::present_rules_dialog(
        parent,
        connection,
        crate::albums_view::rules::RulesDialogSpec {
            title: if ask_name { "Save as Smart Album".into() } else { "New Smart Album".into() },
            subtitle: (!ask_name).then(|| name.clone()),
            name: ask_name.then(|| name.clone()),
            accept_label: if ask_name { "Save" } else { "Create" },
            notice,
        },
        rules,
        Rc::new(move |entered, rules| {
            let name = if ask_name { entered } else { name.as_str() };
            db::create_smart_album(&context.connection.borrow(), name, rules)?;
            refresh_album_ui(&context);
            Ok(())
        }),
    );
}

/// Open a Smart Album's rules from the sidebar or album card menu
/// (`sidebar::EDIT_ALBUM_RULES_ACTION`). Saving refreshes album counts and,
/// when that album is open, its photos.
fn show_edit_album_rules(parent: &gtk::Widget, album_id: i64, context: PhotoActionContext) {
    let Some(album) = db::albums(&context.connection.borrow())
        .unwrap_or_default()
        .into_iter()
        .find(|album| album.id == album_id && album.is_smart())
    else {
        return;
    };
    let connection = context.connection.clone();
    crate::albums_view::rules::present_edit_rules(
        parent,
        connection,
        &album,
        Rc::new(move || {
            refresh_album_ui(&context);
            if context.filter.get() == sidebar::SidebarFilter::Album(album_id) {
                refresh_photo_actions_grid(&context);
            }
        }),
    );
}

fn show_delete_album_confirmation(parent: gtk::Widget, album_id: i64, context: PhotoActionContext) {
    let name = db::albums(&context.connection.borrow())
        .unwrap_or_default()
        .into_iter()
        .find(|album| album.id == album_id)
        .map(|album| album.name)
        .unwrap_or_else(|| "this album".to_string());
    let dialog = adw::AlertDialog::builder()
        .heading(format!("Delete “{name}”?"))
        .body("Photos will remain in their original folders.")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete Album");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    let parent_for_response = parent.clone();
    dialog.connect_response(Some("delete"), move |_, _| {
        if let Err(error) = db::delete_album(&context.connection.borrow(), album_id) {
            show_error(
                &parent_for_response,
                "Could not delete album",
                &error.to_string(),
            );
            return;
        }
        if context.filter.get() == sidebar::SidebarFilter::Album(album_id) {
            context.filter.set(sidebar::SidebarFilter::All);
            if let Some(gallery) = context.gallery.borrow().upgrade() {
                restore_section_layout(
                    &context.connection.borrow(),
                    &gallery,
                    &context.info,
                    sidebar::SidebarFilter::All,
                );
            }
            refresh_photo_actions_grid(&context);
        }
        refresh_album_ui(&context);
    });
    dialog.present(Some(&parent));
}

fn show_remove_folder_confirmation(
    parent: gtk::Widget,
    folder: db::Folder,
    context: PhotoActionContext,
) {
    let dialog = adw::AlertDialog::builder()
        .heading(format!("Remove “{}” from the library?", folder.name))
        .body("Remove only allows future scans to re-import this folder. Remove & Exclude permanently ignores this folder and its subfolders. Original files are never deleted.")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("remove", "Remove only");
    dialog.add_response("exclude", "Remove & Exclude");
    dialog.set_response_appearance("exclude", adw::ResponseAppearance::Destructive);
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);

    let parent_for_response = parent.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "remove" && response != "exclude" { return; }
        let exclude = response == "exclude";
        // Store the exclusion BEFORE removing the catalog row, so a parent
        // refresh cannot rediscover the folder during the operation.
        if exclude {
            if let Err(error) = db::exclude_folder(&context.connection.borrow(), &folder.path) {
                show_error(&parent_for_response, "Could not exclude folder", &error.to_string());
                return;
            }
        }
        if let Err(error) = db::remove_folder(&context.connection.borrow(), folder.id) {
            show_error(
                &parent_for_response,
                "Could not remove folder",
                &error.to_string(),
            );
            return;
        }

        let current_filter = context.filter.get();
        let current_folder_removed = match current_filter {
            sidebar::SidebarFilter::Folder(current_id) => {
                !db::folder_exists(&context.connection.borrow(), current_id).unwrap_or(true)
            }
            _ => false,
        };
        if current_folder_removed {
            context.filter.set(sidebar::SidebarFilter::All);
            if let Some(gallery) = context.gallery.borrow().upgrade() {
                restore_section_layout(
                    &context.connection.borrow(),
                    &gallery,
                    &context.info,
                    sidebar::SidebarFilter::All,
                );
            }
            if let Some(sidebar) = context.sidebar.borrow().as_ref() {
                sidebar::set_active_filter(sidebar, sidebar::SidebarFilter::All);
            }
        }

        refresh_album_ui(&context);
        if context.filter.get() != sidebar::SidebarFilter::Albums {
            refresh_photo_actions_grid(&context);
        }
    });
    dialog.present(Some(&parent));
}
