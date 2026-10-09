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
            .selected_photo
            .borrow()
            .as_ref()
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

    let albums = db::albums(&context.connection.borrow()).unwrap_or_default();
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
    let parent_for_response = parent.clone();
    let entry_for_response = entry.clone();
    dialog.connect_response(Some("create"), move |_, _| {
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
        .body("The folder and its indexed photos will be removed from the app. Files on disk will not be deleted.")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("remove", "Remove from Library");
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);

    let parent_for_response = parent.clone();
    dialog.connect_response(Some("remove"), move |_, _| {
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
