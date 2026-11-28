use gio::prelude::{AppInfoExt, SettingsExt};

thread_local! {
    static ACTIVE_PHOTO_MENU: RefCell<Option<gtk::Widget>> = RefCell::new(None);
    static CONTEXT_MENU_CSS_INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Installs the context-menu stylesheet exactly once per process. The menu
/// opens on every right-click, so installing here would add a new display
/// provider each time and accumulate them over a session.
fn ensure_context_menu_css(display: &gtk::gdk::Display) {
    if CONTEXT_MENU_CSS_INSTALLED.with(std::cell::Cell::get) {
        return;
    }
    let css = gtk::CssProvider::new();
    css.load_from_string(crate::css::PHOTO_CONTEXT_MENU);
    gtk::style_context_add_provider_for_display(
        display,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    CONTEXT_MENU_CSS_INSTALLED.with(|cell| cell.set(true));
}

/// Shared menu state for the fullscreen viewer's capture-phase wheel router.
/// Avoid duplicating menu ownership inside Lightbox.
pub(crate) fn photo_context_menu_is_open() -> bool {
    ACTIVE_PHOTO_MENU.with(|active| active.borrow().as_ref().is_some_and(|menu| menu.is_visible()))
}

pub(crate) fn dismiss_active_photo_context_menu() -> bool {
    ACTIVE_PHOTO_MENU.with(|active| {
        let Some(menu) = active.borrow_mut().take() else {
            return false;
        };
        menu.set_visible(false);
        // GtkOverlay owns its overlay children. Unparenting them directly
        // bypasses GtkOverlay's child list, which can leave stale allocations
        // or event targets when reopening the menu in the lightbox.
        if let Some(overlay) = menu.parent().and_then(|parent| parent.downcast::<gtk::Overlay>().ok()) {
            overlay.remove_overlay(&menu);
        } else if menu.parent().is_some() {
            menu.unparent();
        }
        true
    })
}

/// Short, unobtrusive ease-in/ease-out for the shared context-menu surface.
/// Hover dismissal fades away; action-triggered dismissal stays synchronous
/// so a menu cannot intercept or obscure a newly opened viewer/dialog.
fn fade_out_photo_context_menu(menu: &gtk::Widget) {
    let is_active = ACTIVE_PHOTO_MENU.with(|active| {
        active.borrow().as_ref().is_some_and(|current| current == menu)
    });
    if !is_active {
        return;
    }
    let animate = gtk::Settings::default().is_none_or(|settings| {
        settings.property::<bool>("gtk-enable-animations")
    });
    if !animate || !menu.is_mapped() {
        dismiss_active_photo_context_menu();
        return;
    }
    ACTIVE_PHOTO_MENU.with(|active| {
        active.borrow_mut().take();
    });
    menu.set_can_target(false);
    let started = std::time::Instant::now();
    let menu_for_tick = menu.clone();
    menu.add_tick_callback(move |_, _| {
        let progress = (started.elapsed().as_secs_f64() / 0.110).clamp(0.0, 1.0);
        let eased = progress * progress * (3.0 - 2.0 * progress);
        menu_for_tick.set_opacity(1.0 - eased);
        if progress < 1.0 {
            return glib::ControlFlow::Continue;
        }
        if let Some(overlay) = menu_for_tick
            .parent()
            .and_then(|parent| parent.downcast::<gtk::Overlay>().ok())
        {
            overlay.remove_overlay(&menu_for_tick);
        }
        glib::ControlFlow::Break
    });
}

fn photo_context_menu_contains(widget: &gtk::Widget) -> bool {
    ACTIVE_PHOTO_MENU.with(|active| {
        let active = active.borrow();
        let Some(menu) = active.as_ref() else {
            return false;
        };
        let mut current = Some(widget.clone());
        while let Some(item) = current {
            if item == *menu {
                return true;
            }
            current = item.parent();
        }
        false
    })
}

const BULK_RECIPE_CHUNK: usize = 64;

enum BulkRecipePlan {
    Fixed(String),
    LayersOnly(String),
    Reset,
}

impl BulkRecipePlan {
    fn operation_name(&self) -> &'static str {
        match self {
            Self::Fixed(_) => "Applying edits",
            Self::LayersOnly(_) => "Applying text & overlays",
            Self::Reset => "Resetting edits",
        }
    }
}

struct BulkRecipeState {
    context: PhotoActionContext,
    plan: BulkRecipePlan,
    ids: Vec<i64>,
    index: usize,
    failed: usize,
    last_filename: String,
    notify: bool,
    history_event: Option<i64>,
}

/// Apply recipe changes to many photos without blocking the GTK main loop.
///
/// Work is sliced into small chunks; each chunk commits one SQLite
/// transaction and does one batched gallery/lightbox refresh, then yields
/// back to the event loop so GNOME never sees the app as hung. Progress is
/// reported through the shared top-notification bar (throttled there).
fn start_bulk_recipe_update(
    context: &PhotoActionContext,
    plan: BulkRecipePlan,
    ids: Vec<i64>,
) {
    if ids.is_empty() {
        return;
    }
    let name = plan.operation_name();
    let total = ids.len();
    // Small pastes are instant; flashing the notification bar for them is
    // noise. Only selections larger than 9 photos get progress feedback.
    let notify = total > 9;
    if notify {
        context.operation_progress.begin(name, total);
    }
    let state = Rc::new(RefCell::new(Some(BulkRecipeState {
        context: context.clone(),
        plan,
        ids,
        index: 0,
        failed: 0,
        last_filename: String::new(),
        notify,
        history_event: None,
    })));
    glib::timeout_add_local(Duration::from_millis(1), move || {
        let Some(mut work) = state.borrow_mut().take() else {
            return glib::ControlFlow::Break;
        };
        run_bulk_recipe_chunk(&mut work);
        let finished = work.index >= work.ids.len();
        if work.notify {
            work.context.operation_progress.update(
                name,
                work.index.min(total),
                total,
                &work.last_filename,
                work.failed,
            );
        }
        if finished {
            finish_bulk_recipe(work, name, total);
            return glib::ControlFlow::Break;
        }
        *state.borrow_mut() = Some(work);
        glib::ControlFlow::Continue
    });
}

fn run_bulk_recipe_chunk(work: &mut BulkRecipeState) {
    let end = (work.index + BULK_RECIPE_CHUNK).min(work.ids.len());
    let chunk_ids = work.ids[work.index..end].to_vec();
    if chunk_ids.is_empty() {
        work.index = end;
        return;
    }

    let mut updates: Vec<(i64, String)> = Vec::with_capacity(chunk_ids.len());
    match &work.plan {
        BulkRecipePlan::Fixed(recipe) => {
            for id in &chunk_ids {
                updates.push((*id, recipe.clone()));
            }
        }
        BulkRecipePlan::Reset => {
            for id in &chunk_ids {
                updates.push((*id, String::new()));
            }
        }
        BulkRecipePlan::LayersOnly(clipboard) => {
            for id in &chunk_ids {
                // Scope the connection borrow to this statement so a nested
                // RefCell borrow (thumbnail/gallery refresh) cannot panic.
                let photo = {
                    let connection = work.context.connection.borrow();
                    db::photo(&connection, *id)
                };
                match photo {
                    Ok(Some(item)) => {
                        if let Some(path) = std::path::Path::new(&item.path).file_name() {
                            if let Some(name) = path.to_str() {
                                work.last_filename = name.to_string();
                            }
                        }
                        updates.push((
                            *id,
                            crate::edit::model::paste_layers_only(&item.edit_recipe, clipboard),
                        ));
                    }
                    Ok(None) => work.failed += 1,
                    Err(_) => work.failed += 1,
                }
            }
        }
    }

    if !updates.is_empty() {
        let action = match work.plan {
            BulkRecipePlan::Reset => "reset",
            _ if work.ids.len() > 1 => "batch_paste",
            _ => "paste",
        };
        let result = db::commit_edit_recipes(
            &work.context.connection.borrow(),
            &updates,
            action,
            work.history_event,
        );
        if let Err(error) = result {
            work.failed = work.failed.saturating_add(updates.len());
            eprintln!("Could not apply bulk edit recipes: {error:#}");
        } else {
            work.history_event = result.unwrap();
            photo_changed(
                &work.context,
                PhotoChange::Recipes {
                    updates: &updates,
                    more_coming: true,
                },
            );
        }
    }

    work.index = end;
}

fn finish_bulk_recipe(work: BulkRecipeState, name: &'static str, total: usize) {
    // Each slice showed its recipes; the end recounts and rebuilds once.
    photo_changed(
        &work.context,
        PhotoChange::Recipes {
            updates: &[],
            more_coming: false,
        },
    );
    let succeeded = total.saturating_sub(work.failed);
    let summary = if work.failed == 0 {
        match name {
            "Resetting edits" => format!("Edits reset on {total} photos"),
            "Applying text & overlays" => format!("Text & overlays applied to {total} photos"),
            _ => format!("Edits applied to {total} photos"),
        }
    } else {
        format!(
            "{succeeded} / {total} photos updated · {} failed",
            work.failed
        )
    };
    // Small selections skip the bar entirely, but failures still surface so
    // a broken paste never fails silently.
    if work.notify || work.failed > 0 {
        work.context.operation_progress.finish(name, &summary);
    }
}

/// The lightbox is a sibling overlay ABOVE the gallery/inspector split.
/// A menu attached to the gallery's page_overlay still exists but is covered
/// by the fullscreen viewer. Resolve the host from the click's actual widget
/// ancestry; grid clicks continue using the original page overlay.
fn photo_context_menu_host_for_anchor(
    anchor: &gtk::Widget,
    gallery_host: &gtk::Overlay,
    lightbox_host: Option<&gtk::Overlay>,
) -> gtk::Overlay {
    if let Some(lightbox_host) = lightbox_host {
        let lightbox_widget: gtk::Widget = lightbox_host.clone().upcast();
        if lightbox_host.is_visible()
            && std::iter::successors(Some(anchor.clone()), |widget| widget.parent())
                .any(|widget| widget == lightbox_widget)
        {
            return lightbox_host.clone();
        }
    }
    gallery_host.clone()
}

fn show_photo_context_menu(
    photo: crate::photo_object::PhotoObject,
    anchor: gtk::Widget,
    context: PhotoActionContext,
    x: f64,
    y: f64,
) {
    // The menu must belong to the actual topmost presentation surface:
    // gallery for tile clicks, lightbox.root for a viewer right-click.
    dismiss_active_photo_context_menu();

    let Some(gallery_host) = context
        .context_menu_host
        .borrow()
        .as_ref()
        .and_then(glib::WeakRef::upgrade)
    else {
        return;
    };
    let lightbox = context.lightbox.upgrade();
    let host = photo_context_menu_host_for_anchor(
        &anchor,
        &gallery_host,
        lightbox.as_ref().map(|lightbox| &lightbox.root),
    );
    crate::nav_transition_trace!(
        "stage=photo_context_menu_host lightbox={} anchor={} host={} mapped={}",
        host != gallery_host,
        anchor.widget_name(),
        host.widget_name(),
        host.is_mapped(),
    );
    let host_widget = host.clone().upcast::<gtk::Widget>();
    let click_point = if anchor == host_widget {
        gtk::graphene::Point::new(x as f32, y as f32)
    } else {
        let Some(point) =
            anchor.compute_point(&host_widget, &gtk::graphene::Point::new(x as f32, y as f32))
        else {
            return;
        };
        point
    };

    // This is a normal overlay widget rather than GtkPopoverMenu, so the
    // theme would otherwise give every GtkButton regular toolbar/dialog
    // padding.  Apply a small context-menu-specific CSS class so rows look
    // and measure like menu items instead of large push buttons.
    ensure_context_menu_css(&host.display());

    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
    menu.set_margin_top(0);
    menu.set_margin_bottom(0);
    menu.set_margin_start(0);
    menu.set_margin_end(0);

    let add_action = |text: &str| {
        let button = gtk::Button::new();
        button.set_focus_on_click(false);
        button.set_halign(gtk::Align::Fill);
        button.set_margin_top(0);
        button.set_margin_bottom(0);
        button.add_css_class("flat");
        button.add_css_class("photo-context-item");

        let label = gtk::Label::new(Some(text));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        button.set_child(Some(&label));

        menu.append(&button);
        button
    };

    let menu_to_hide: Rc<RefCell<Option<gtk::Widget>>> = Rc::new(RefCell::new(None));
    let dismiss_menu: Rc<dyn Fn()> = {
        let menu_to_hide = menu_to_hide.clone();
        Rc::new(move || {
            if let Some(menu) = menu_to_hide.borrow_mut().take() {
                menu.set_visible(false);
                if menu.parent().is_some() {
                    menu.unparent();
                }
            }
            ACTIVE_PHOTO_MENU.with(|active| {
                active.borrow_mut().take();
            });
        })
    };

    // A photo opened from outside the library has no catalogue row, so the
    // library actions (albums, marks, edit recipes, rename, delete) cannot
    // apply to it; they are hidden for it below.
    let outside_library = photo.is_outside_library();
    let open = add_action("Open");
    let edit = add_action("Edit Photo…");
    let open_with = add_action("Open With…");
    let open_in_folder = (!matches!(context.filter.get(), sidebar::SidebarFilter::Albums))
        .then(|| add_action("Open in Folder"));
    {
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class("photo-context-separator");
        menu.append(&separator);
    }

    let selection_ids = selected_photo_ids(&context, Some(photo.id()));

    let selection_for_provider = selection_ids.clone();
    let selection_provider: Rc<dyn Fn() -> Vec<i64>> =
        Rc::new(move || selection_for_provider.clone());
    let add_to_album = add_action("Add to Album");
    let album_scroll_y = context
        .gallery
        .borrow()
        .upgrade()
        .map(|gallery| gallery.scroll_position())
        .unwrap_or_default();
    let album_photo_id = photo.id();
    let album_gallery = context.gallery.clone();
    let restore_album_view: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(gallery) = album_gallery.borrow().upgrade() {
            gallery.restore_context_view(album_photo_id, album_scroll_y);
        }
    });
    let album_context = context.clone();
    let album_selection = selection_provider.clone();
    let dismiss_for_album = dismiss_menu.clone();
    let album_host = host_widget.clone();
    add_to_album.connect_clicked(move |button| {
        // The same Add to Album list as the InfoBar button, beside this row.
        // Measure the row first: closing the menu unparents it.
        let row = button.compute_bounds(&album_host).map(|bounds| {
            gtk::gdk::Rectangle::new(
                bounds.x().round() as i32,
                bounds.y().round() as i32,
                bounds.width().round() as i32,
                bounds.height().round() as i32,
            )
        });
        dismiss_for_album();
        show_album_popover(
            &album_host,
            row.as_ref(),
            album_context.clone(),
            album_selection.clone(),
            restore_album_view.clone(),
        );
    });

    // The albums and Smart Albums this photo is in, each opening as its
    // sidebar row does. The album being viewed is not listed.
    if !outside_library {
        let viewed = match context.filter.get() {
            sidebar::SidebarFilter::Album(id) => Some(id),
            _ => None,
        };
        let containing = db::albums_containing_photo(&context.connection.borrow(), photo.id())
            .unwrap_or_default()
            .into_iter()
            .filter(|album| Some(album.id) != viewed)
            .collect::<Vec<_>>();
        if !containing.is_empty() {
            let heading = gtk::Label::new(Some("In Albums"));
            heading.set_xalign(0.0);
            heading.set_margin_start(12);
            heading.set_margin_top(4);
            heading.add_css_class("dim-label");
            heading.add_css_class("caption-heading");
            menu.append(&heading);
        }
        for album in containing {
            let row = add_action(&album.name);
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            content.append(&gtk::Image::from_icon_name(if album.is_smart() {
                sidebar::smart_album_icon_name()
            } else {
                sidebar::album_icon_name()
            }));
            let label = gtk::Label::new(Some(&album.name));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            content.append(&label);
            row.set_child(Some(&content));
            row.set_tooltip_text(Some(if album.is_smart() {
                "Open this Smart Album"
            } else {
                "Open this album"
            }));
            let dismiss_for_open = dismiss_menu.clone();
            let open_host = host_widget.clone();
            let album_id = album.id;
            row.connect_clicked(move |_| {
                dismiss_for_open();
                let _ = open_host.activate_action(OPEN_ALBUM_ACTION, Some(&album_id.to_variant()));
            });
        }
    }

    let collage_ids = selection_ids.clone();
    let collage = add_action("Create Collage…");
    collage.set_sensitive(collage_ids.len() >= 2);
    let collage_context = context.clone();
    let dismiss_menu_for_collage = dismiss_menu.clone();
    collage.connect_clicked(move |_| {
        dismiss_menu_for_collage();

        (collage_context.open_collage)(collage_ids.clone());
    });

    {
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class("photo-context-separator");
        menu.append(&separator);
    }
    let copy_edits = add_action("Copy Edits");
    let paste_edits = add_action("Paste Edits");
    let paste_overlays = add_action("Paste Text & Overlays Only");
    let reset_edits = add_action("Reset Edits");
    let clicked_recipe = photo.edit_recipe();
    let clicked_is_edited = !crate::edit::EditRecipe::decode(&clicked_recipe).is_default();
    copy_edits.set_sensitive(clicked_is_edited);
    paste_edits.set_sensitive(context.edit_clipboard.borrow().is_some());
    // Only enable when the clipboard carries at least one overlay or text layer.
    paste_overlays.set_sensitive(context.edit_clipboard.borrow().as_deref().is_some_and(
        |recipe| {
            let decoded = crate::edit::EditRecipe::decode(recipe);
            !decoded.overlays.is_empty() || !decoded.text_layers.is_empty()
        },
    ));
    reset_edits.set_sensitive(
        clicked_is_edited || {
            db::any_edited(&context.connection.borrow(), &selection_ids).unwrap_or(false)
        },
    );

    {
        let clipboard = context.edit_clipboard.clone();
        let recipe = clicked_recipe.clone();
        let dismiss_menu = dismiss_menu.clone();
        copy_edits.connect_clicked(move |_| {
            clipboard.replace(Some(recipe.clone()));
            dismiss_menu();
        });
    }
    {
        let paste_context = context.clone();
        let paste_selection = selection_ids.clone();
        let dismiss_menu = dismiss_menu.clone();
        paste_edits.connect_clicked(move |_| {
            let Some(recipe) = paste_context.edit_clipboard.borrow().clone() else {
                return;
            };
            dismiss_menu();
            start_bulk_recipe_update(
                &paste_context,
                BulkRecipePlan::Fixed(recipe),
                paste_selection.clone(),
            );
        });
    }
    {
        let paste_context = context.clone();
        let paste_selection = selection_ids.clone();
        let dismiss_menu = dismiss_menu.clone();
        paste_overlays.connect_clicked(move |_| {
            let Some(clipboard) = paste_context.edit_clipboard.borrow().clone() else {
                return;
            };
            dismiss_menu();
            start_bulk_recipe_update(
                &paste_context,
                BulkRecipePlan::LayersOnly(clipboard),
                paste_selection.clone(),
            );
        });
    }
    {
        let reset_context = context.clone();
        let reset_selection = selection_ids.clone();
        let dismiss_menu = dismiss_menu.clone();
        reset_edits.connect_clicked(move |_| {
            dismiss_menu();
            start_bulk_recipe_update(
                &reset_context,
                BulkRecipePlan::Reset,
                reset_selection.clone(),
            );
        });
    }

    // Name what the click does: a selection is unfavourited only when every
    // photo in it already is a favourite.
    let favorite_label = match photo_mark_commands::preview(
        &context,
        MarkIntent::ToggleFavorite,
        &photo,
    ) {
        Some(MarkChange::Favorite(false)) => "Remove from Favourites",
        _ => "Add to Favourites",
    };
    let favorite = add_action(favorite_label);
    let favorite_context = context.clone();
    let favorite_photo = photo.clone();
    let dismiss_for_favorite = dismiss_menu.clone();
    favorite.connect_clicked(move |_| {
        dismiss_for_favorite();
        photo_mark_commands::run(&favorite_context, MarkIntent::ToggleFavorite, &favorite_photo);
    });

    if let (false, sidebar::SidebarFilter::Album(album_id)) =
        (outside_library, context.filter.get())
    {
        // The cover only makes sense inside an album, and always applies to
        // the album being viewed: no album list, no cross-album choices.
        let viewed_album = db::albums(&context.connection.borrow())
            .unwrap_or_default()
            .into_iter()
            .find(|album| album.id == album_id);
        let already_cover =
            viewed_album.as_ref().and_then(|album| album.cover_photo_id) == Some(photo.id());
        // A Smart Album's rules decide its photos, so none can be removed by hand.
        let manual = viewed_album.as_ref().is_some_and(|album| !album.is_smart());
        let set_cover = add_action("Set as Album Cover");
        set_cover.set_sensitive(!already_cover);
        let set_cover_context = context.clone();
        let set_cover_photo_id = photo.id();
        let dismiss_menu_for_cover = dismiss_menu.clone();
        set_cover.connect_clicked(move |button| {
            dismiss_menu_for_cover();
            if let Err(error) = db::set_album_cover_photo(
                &set_cover_context.connection.borrow(),
                album_id,
                set_cover_photo_id,
            ) {
                show_error(
                    button.upcast_ref(),
                    "Could not set album cover",
                    &error.to_string(),
                );
                return;
            }
            albums_edited(&set_cover_context);
        });

        let remove = manual.then(|| add_action("Remove from Album"));
        let remove_context = context.clone();
        let remove_selection = selection_provider.clone();
        let dismiss_menu_for_remove = dismiss_menu.clone();
        if let Some(remove) = remove { remove.connect_clicked(move |button| {
            dismiss_menu_for_remove();
            let ids = remove_selection();
            if let Err(error) =
                db::remove_photos_from_album(&remove_context.connection.borrow(), album_id, &ids)
            {
                show_error(
                    button.upcast_ref(),
                    "Could not remove from album",
                    &error.to_string(),
                );
                return;
            }
            if let Some(lightbox) = remove_context.lightbox.upgrade() {
                lightbox.close();
            }
            if let Some(gallery) = remove_context.gallery.borrow().upgrade() {
                gallery.remove_photos(&ids);
            }
            albums_edited(&remove_context);
        }); }
    }
    {
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class("photo-context-separator");
        menu.append(&separator);
    }
    let copy = add_action("Copy");
    let copy_location = add_action("Copy File Location");
    {
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class("photo-context-separator");
        menu.append(&separator);
    }
    let rename = add_action("Rename…");
    let file_manager = add_action("Open in File Manager");
    let wallpaper = add_action("Set as Wallpaper");
    let export = add_action("Export…");
    let print = add_action("Print");
    let properties = add_action("Properties");
    {
        let export_context = context.clone();
        let export_photo = photo.clone();
        let dismiss_menu_for_export = dismiss_menu.clone();
        export.connect_clicked(move |_| {
            dismiss_menu_for_export();
            // Export anchors on the right-clicked photo, so a single-tile
            // export always works (multi-select still comes from the gallery
            // selection). The current photo is left alone.
            (export_context.export)(&export_photo);
        });
    }
    {
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class("photo-context-separator");
        menu.append(&separator);
    }
    let delete = add_action("Delete");
    delete.add_css_class("destructive-action");
    #[cfg(target_os = "linux")]
    if crate::network_shares::private(&photo.path()) {
        for action in [&rename, &file_manager, &delete] {
            action.set_sensitive(false);
            action.set_tooltip_text(Some("Direct network shares are read-only inside PIC; no desktop mount or remote write is attempted"));
        }
    }

    let edit_context = context.clone();
    let edit_id = photo.id();
    let dismiss_menu_for_edit = dismiss_menu.clone();
    edit.connect_clicked(move |_| {
        dismiss_menu_for_edit();
        (edit_context.open_edit)(edit_id);
    });

    let photo_for_open = photo.clone();
    let dismiss_menu_for_open = dismiss_menu.clone();
    let lightbox_for_open = context.lightbox.clone();
    let gallery_for_open = context.gallery.clone();
    open.connect_clicked(move |_| {
        // Remove the menu before opening so unparenting it cannot steal focus
        // from the newly opened lightbox. Keep the gallery's current photo
        // collection so wheel/arrow navigation continues normally.
        dismiss_menu_for_open();
        if let Some(lightbox) = lightbox_for_open.upgrade() {
            let photos = gallery_for_open
                .borrow()
                .upgrade()
                .map(|gallery| gallery.photo_objects())
                .unwrap_or_default();
            if let Some(index) = photos
                .iter()
                .position(|item| item.id() == photo_for_open.id())
            {
                lightbox.open(photos, index);
            } else {
                lightbox.open(vec![photo_for_open.clone()], 0);
            }
        }
    });

    #[cfg(target_os = "linux")]
    if crate::network_shares::private(&photo.path()) {
        open_with.set_sensitive(false);
        open_with.set_tooltip_text(Some("Network originals stay in PIC's private SMB/NFS transport; desktop file-manager mounts are disabled"));
    }
    let file = crate::source::file(&photo.path());
    let open_with_file = file.clone();
    let open_with_window = context.window.clone();
    let dismiss_menu_for_open_with = dismiss_menu.clone();
    open_with.connect_clicked(move |_| {
        // The menu is removed before the chooser is presented, so never use
        // the menu button itself as the dialog parent.  It is unparented at
        // that point and AdwAlertDialog can fall back to a corner placement.
        dismiss_menu_for_open_with();
        if let Some(window) = open_with_window.upgrade() {
            show_open_with_dialog(window.upcast_ref(), &open_with_file);
        }
    });

    if let Some(open_in_folder) = open_in_folder.clone() {
        let folder_id = photo.folder_id();
        let navigate_to_folder = context.navigate_to_folder.clone();
        let dismiss_menu_for_folder = dismiss_menu.clone();
        let photo_id = photo.id();
        let current_filter = context.filter.clone();
        let current_search = context.search.clone();
        let gallery_for_folder = context.gallery.clone();
        let sidebar_for_folder = context.sidebar.clone();
        let photo_path_for_folder = photo.path();
        open_in_folder.connect_clicked(move |_| {
            if folder_id != 0 {
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_OPEN_IN_FOLDER request photo_id={} folder_id={} path={:?} filter={:?} searching={}",
                        photo_id,
                        folder_id,
                        photo_path_for_folder,
                        current_filter.get(),
                        !current_search.borrow().is_empty()
                    );
                }
                // If we are already in the continuous Folder tree/stream and no
                // search filter is active, do not route through destination
                // navigation again. That path can first scroll to the folder
                // header and then re-select the photo, which is fragile for
                // virtualized folder rows. Directly select/scroll the clicked
                // photo instead, and still reveal its folder in the sidebar.
                if matches!(current_filter.get(), sidebar::SidebarFilter::Folder(_))
                    && current_search.borrow().is_empty()
                {
                    if let Some(gallery) = gallery_for_folder.borrow().upgrade() {
                        if gallery.select_photo(photo_id) {
                            if let Some(sidebar) = sidebar_for_folder.borrow().as_ref() {
                                sidebar::scroll_to_folder(sidebar, folder_id);
                            }
                            dismiss_menu_for_folder();
                            return;
                        }
                    }
                }
                navigate_to_folder(folder_id, photo_id);
            }
            dismiss_menu_for_folder();
        });
    }

    let path_for_copy = photo.path();
    let display = anchor.display();
    let dismiss_menu_for_copy = dismiss_menu.clone();
    copy_location.connect_clicked(move |_| {
        display.clipboard().set_text(&path_for_copy);
        dismiss_menu_for_copy();
    });

    let file_for_manager = file.clone();
    let dismiss_menu_for_manager = dismiss_menu.clone();
    file_manager.connect_clicked(move |_| {
        open_file_in_manager(&file_for_manager);
        dismiss_menu_for_manager();
    });

    let wallpaper_path = photo.path();
    let wallpaper_rotation = photo.rotation();
    let wallpaper_recipe = photo.edit_recipe();
    let wallpaper_width = photo.width();
    let wallpaper_height = photo.height();
    let wallpaper_mtime = photo.mtime();
    let wallpaper_size = photo.size_bytes();
    let wallpaper_window = context.window.clone();
    let dismiss_menu_for_wallpaper = dismiss_menu.clone();
    wallpaper.connect_clicked(move |_| {
        dismiss_menu_for_wallpaper();
        let target_size = wallpaper_window
            .upgrade()
            .and_then(|window| {
                let surface = window.surface()?;
                let display = gtk::prelude::WidgetExt::display(&window);
                let monitor = display.monitor_at_surface(&surface)?;
                let geometry = monitor.geometry();
                let scale = f64::from(monitor.scale_factor().max(1));
                Some((
                    (f64::from(geometry.width()) * scale).round() as u32,
                    (f64::from(geometry.height()) * scale).round() as u32,
                ))
            })
            .filter(|(width, height)| *width > 0 && *height > 0)
            .unwrap_or((1920, 1080));
        let path = wallpaper_path.clone();
        let recipe = wallpaper_recipe.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = prepare_wallpaper(
                &path,
                wallpaper_rotation,
                &recipe,
                wallpaper_width,
                wallpaper_height,
                wallpaper_mtime,
                wallpaper_size,
                target_size,
            )
            .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });

        let window = wallpaper_window.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || match receiver
            .try_recv()
        {
            Ok(Ok(path)) => {
                if let Err(error) = apply_wallpaper(&path) {
                    if let Some(window) = window.upgrade() {
                        show_error(
                            window.upcast_ref(),
                            "Could not set wallpaper",
                            &error.to_string(),
                        );
                    }
                }
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                if let Some(window) = window.upgrade() {
                    show_error(window.upcast_ref(), "Could not set wallpaper", &error);
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        });
    });

    // Copy puts the clicked photo, edits applied, on the clipboard as PNG,
    // the same as the editor's Copy button.
    {
        let photo = photo.clone();
        let window = context.window.clone();
        let display = anchor.display();
        let dismiss_menu = dismiss_menu.clone();
        copy.connect_clicked(move |_| {
            dismiss_menu();
            let job = crate::edit::export_batch::ExportJob::from_record(
                photo.path(),
                photo.rotation(),
                photo.edit_recipe(),
                photo.width(),
                photo.height(),
                &photo.filename(),
            );
            let window = window.clone();
            crate::edit::quick_save::copy_to_clipboard(display.clipboard(), job, move |copied| {
                if let (Err(error), Some(window)) = (copied, window.upgrade()) {
                    show_error(window.upcast_ref(), "Could not copy the photo", &error);
                }
            });
        });
    }
    // Print runs the InfoBar's print flow on the same photos the menu acts on.
    {
        let photo = photo.clone();
        let context = context.clone();
        let ids = selection_ids.clone();
        let dismiss_menu = dismiss_menu.clone();
        print.connect_clicked(move |_| {
            dismiss_menu();
            let requests = anchored_print_requests(&context.connection.borrow(), &photo, &ids);
            if let Some(window) = context.window.upgrade() {
                print_photos(&window, &context.connection, requests);
            }
        });
    }

    if outside_library {
        for action in [
            Some(&add_to_album),
            Some(&collage),
            Some(&copy_edits),
            Some(&paste_edits),
            Some(&paste_overlays),
            Some(&reset_edits),
            Some(&favorite),
            open_in_folder.as_ref(),
            Some(&rename),
            Some(&delete),
        ]
        .into_iter()
        .flatten()
        {
            action.set_visible(false);
        }
        hide_orphan_separators(&menu);
    }

    let photo_for_rename = photo.clone();
    let anchor_for_rename = anchor.clone();
    let context_for_rename = context.clone();
    let dismiss_menu_for_rename = dismiss_menu.clone();
    rename.connect_clicked(move |_| {
        dismiss_menu_for_rename();
        show_rename_dialog(
            &anchor_for_rename,
            photo_for_rename.clone(),
            context_for_rename.clone(),
        );
    });

    let photo_for_properties = photo.clone();
    let context_for_properties = context.clone();
    let dismiss_menu_for_properties = dismiss_menu.clone();
    properties.connect_clicked(move |_| {
        dismiss_menu_for_properties();
        if let Some(properties) = context_for_properties.properties.upgrade() {
            properties.open(&photo_for_properties);
        }
    });

    let photo_for_delete = photo;
    let anchor_for_delete = anchor.clone();
    let context_for_delete = context;
    let delete_selection = selection_ids;
    let dismiss_menu_for_delete = dismiss_menu.clone();
    delete.connect_clicked(move |_| {
        dismiss_menu_for_delete();
        show_delete_confirmation(
            &anchor_for_delete,
            photo_for_delete.clone(),
            delete_selection.clone(),
            context_for_delete.clone(),
        );
    });

    // Render the same menu UI for both grid thumbnails and lightbox photos.
    // A normal overlay child is always allocated by GtkOverlay, so there is
    // no GtkPopover 0x0/popup lifecycle to fail.
    let menu_host = gtk::ScrolledWindow::new();
    menu_host.add_css_class("photo-context-menu");
    menu_host.add_css_class("card");
    menu_host.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    menu_host.set_has_frame(false);
    // Height is explicitly bounded below. Propagating the whole menu's
    // natural height makes GtkOverlay allocate every action at once and can
    // prevent the internal scroller from acquiring any vertical scroll range.
    menu_host.set_propagate_natural_height(false);
    // A context menu should never expand into a near full-height panel.
    // Keep enough room for the common actions and scroll only the overflow.
    menu_host.set_max_content_height((host.height() - 32).clamp(1, 460));
    menu_host.set_halign(gtk::Align::Start);
    menu_host.set_valign(gtk::Align::Start);
    menu_host.set_can_target(true);
    menu_host.set_child(Some(&menu));

    // Centre the menu on the pointer instead of treating the pointer as the
    // menu's top-left corner.  Measure after the complete menu has been built
    // so the centring also follows dynamic items such as album actions.
    let (_, natural_width, _, _) = menu_host.measure(gtk::Orientation::Horizontal, -1);
    let measured_width = natural_width.max(1).min(host.width().max(1));
    // Measure the CONTENT, not the GtkScrolledWindow: with
    // propagate_natural_height=false, ScrolledWindow reports only its minimum
    // (roughly two rows). Measuring that and setting it as the height request
    // collapsed the context menu and made it almost impossible to scroll.
    let (_, natural_height, _, _) = menu.measure(gtk::Orientation::Vertical, measured_width);
    let measured_height = natural_height
        .max(1)
        .min(menu_host.max_content_height().max(1));
    menu_host.set_size_request(-1, measured_height);

    const MENU_EDGE_INSET: i32 = 4;
    let max_x = (host.width() - measured_width - MENU_EDGE_INSET).max(MENU_EDGE_INSET);
    let max_y = (host.height() - measured_height - MENU_EDGE_INSET).max(MENU_EDGE_INSET);
    let menu_x =
        (click_point.x().round() as i32 - measured_width / 2).clamp(MENU_EDGE_INSET, max_x);
    let menu_y =
        (click_point.y().round() as i32 - measured_height / 2).clamp(MENU_EDGE_INSET, max_y);
    menu_host.set_margin_start(menu_x);
    menu_host.set_margin_top(menu_y);

    // Dismiss only after the pointer really leaves the shared menu.
    // A short grace period avoids flicker at the edge or when moving over
    // its scrolling content. Re-entering cancels the pending dismissal.
    // Verify menu identity: an old timer must never close a newly opened menu.
    let leave_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let hover = gtk::EventControllerMotion::new();
    let timer_for_enter = leave_timer.clone();
    hover.connect_enter(move |_, _, _| {
        if let Some(timer) = timer_for_enter.borrow_mut().take() {
            timer.remove();
        }
    });
    let timer_for_leave = leave_timer.clone();
    let menu_for_leave = menu_host.clone().upcast::<gtk::Widget>();
    hover.connect_leave(move |_| {
        if let Some(timer) = timer_for_leave.borrow_mut().take() {
            timer.remove();
        }
        let departing = menu_for_leave.clone();
        let timer_slot = timer_for_leave.clone();
        let timer = glib::timeout_add_local_once(Duration::from_millis(180), move || {
            timer_slot.borrow_mut().take();
            let still_active = ACTIVE_PHOTO_MENU.with(|active| {
                active.borrow().as_ref().is_some_and(|current| *current == departing)
            });
            if still_active {
                fade_out_photo_context_menu(&departing);
            }
        });
        timer_for_leave.replace(Some(timer));
    });
    menu_host.add_controller(hover);

    let menu_widget = menu_host.clone().upcast::<gtk::Widget>();
    menu_to_hide.borrow_mut().replace(menu_widget.clone());
    ACTIVE_PHOTO_MENU.with(|active| {
        active.borrow_mut().replace(menu_widget);
    });

    host.add_overlay(&menu_host);
    menu_host.set_visible(true);
    // Fade in the existing menu without reflowing its rows or interfering with
    // its scroller. Smoothstep matches PIC's other navigation transitions.
    if gtk::Settings::default().is_some_and(|settings| {
        settings.property::<bool>("gtk-enable-animations")
    }) {
        menu_host.set_opacity(0.0);
        let started = std::time::Instant::now();
        menu_host.add_tick_callback(move |widget, _| {
            let progress = (started.elapsed().as_secs_f64() / 0.125).clamp(0.0, 1.0);
            let eased = progress * progress * (3.0 - 2.0 * progress);
            widget.set_opacity(eased);
            if progress >= 1.0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
}

/// Print requests for library photos, read fresh from the catalogue.
/// `PhotoObject` stays on the GTK thread; the requests carry plain values
/// for the background renders.
fn photo_print_requests(connection: &Connection, ids: &[i64]) -> Vec<PhotoPrintRequest> {
    ids.iter()
        .filter_map(|id| db::photo(connection, *id).ok().flatten())
        .map(|record| PhotoPrintRequest {
            job_name: record
                .path
                .rsplit('/')
                .next()
                .unwrap_or("photo")
                .to_string(),
            reference: record.path,
            rotation: record.rotation,
            edit_recipe: record.edit_recipe,
            source_width: record.width.unwrap_or(0),
            source_height: record.height.unwrap_or(0),
        })
        .collect()
}

/// The photos an Export or Print anchored on `anchor` acts on: the gallery
/// selection that contains it (`selected_ids`, from `selected_photo_ids`), or
/// the anchor alone. A photo outside the library has no catalogue row, so it
/// is always exported or printed alone, from its own object.
fn anchored_export_jobs(
    anchor: &crate::photo_object::PhotoObject,
    selected_ids: &[i64],
    gallery_objects: Vec<crate::photo_object::PhotoObject>,
) -> Vec<crate::edit::export_batch::ExportJob> {
    if anchor.is_outside_library() {
        return vec![export_job_for_object(anchor)];
    }
    let mut known: std::collections::HashMap<i64, _> = gallery_objects
        .into_iter()
        .map(|object| (object.id(), object))
        .collect();
    // The anchor may not be in the gallery's list (the viewer can show a list
    // the gallery no longer holds); it is still a photo to export.
    known.entry(anchor.id()).or_insert_with(|| anchor.clone());
    selected_ids
        .iter()
        .filter_map(|id| known.get(id))
        .map(export_job_for_object)
        .collect()
}

fn export_job_for_object(
    photo: &crate::photo_object::PhotoObject,
) -> crate::edit::export_batch::ExportJob {
    crate::edit::export_batch::ExportJob::from_record(
        photo.path(),
        photo.rotation(),
        photo.edit_recipe(),
        photo.width(),
        photo.height(),
        &photo.filename(),
    )
}

/// Print's counterpart of [`anchored_export_jobs`]: library photos are read
/// fresh from the catalogue, an outside photo from its own object.
fn anchored_print_requests(
    connection: &Connection,
    anchor: &crate::photo_object::PhotoObject,
    selected_ids: &[i64],
) -> Vec<PhotoPrintRequest> {
    if anchor.is_outside_library() {
        vec![print_request_for_object(anchor)]
    } else {
        photo_print_requests(connection, selected_ids)
    }
}

/// The print request for a photo outside the library (no catalogue row).
fn print_request_for_object(photo: &crate::photo_object::PhotoObject) -> PhotoPrintRequest {
    PhotoPrintRequest {
        job_name: photo.filename(),
        reference: photo.path(),
        rotation: photo.rotation(),
        edit_recipe: photo.edit_recipe(),
        source_width: photo.width(),
        source_height: photo.height(),
    }
}

/// Hide each separator that no longer divides two visible groups.
fn hide_orphan_separators(menu: &gtk::Box) {
    let mut item_before = false;
    let mut pending: Option<gtk::Widget> = None;
    let mut child = menu.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.is::<gtk::Separator>() {
            widget.set_visible(false);
            if item_before {
                pending.get_or_insert(widget);
            }
        } else if widget.is_visible() {
            if let Some(separator) = pending.take() {
                separator.set_visible(true);
            }
            item_before = true;
        }
    }
}

fn open_file_in_manager(file: &gio::File) {
    crate::platform::reveal_file(file);
}

fn prepare_wallpaper(
    reference: &str,
    rotation: i32,
    edit_recipe: &str,
    source_width: i64,
    source_height: i64,
    mtime: i64,
    size_bytes: i64,
    target_size: (u32, u32),
) -> anyhow::Result<std::path::PathBuf> {
    let source_path = crate::source::materialize(reference)?;
    std::fs::canonicalize(&source_path).map_err(|error| {
        anyhow::anyhow!(
            "Could not access {}: {error}",
            source_path.to_string_lossy()
        )
    })?;

    let directory = crate::thumbnail::cache_dir()?.join("wallpaper");
    std::fs::create_dir_all(&directory)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"picasa-wallpaper-v2-fit\0");
    hasher.update(reference.as_bytes());
    hasher.update(b"\0");
    hasher.update(mtime.to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(size_bytes.to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(rotation.to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(edit_recipe.as_bytes());
    hasher.update(b"\0");
    hasher.update(target_size.0.to_string().as_bytes());
    hasher.update(b"x");
    hasher.update(target_size.1.to_string().as_bytes());
    let destination = directory.join(format!("{}.jpg", hasher.finalize().to_hex()));
    if !destination.is_file() {
        let image = crate::edit::render::render_for_export(
            reference,
            rotation,
            edit_recipe,
            source_width,
            source_height,
            0,
        )?;
        let wallpaper = compose_wallpaper(image, target_size.0, target_size.1);
        crate::edit::render::save_jpeg(&wallpaper, &destination, 95)?;
    }
    Ok(destination)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WallpaperLayout {
    Cover,
    PortraitBlur,
    PanoramaBlur,
}

fn wallpaper_layout(
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
) -> WallpaperLayout {
    if source_width < source_height {
        return WallpaperLayout::PortraitBlur;
    }
    let source_ratio = source_width as f64 / source_height.max(1) as f64;
    let target_ratio = target_width.max(1) as f64 / target_height.max(1) as f64;
    if source_ratio > target_ratio * 1.45 {
        WallpaperLayout::PanoramaBlur
    } else {
        WallpaperLayout::Cover
    }
}

fn compose_wallpaper(
    source: image::RgbaImage,
    target_width: u32,
    target_height: u32,
) -> image::RgbaImage {
    let target_width = target_width.max(1);
    let target_height = target_height.max(1);
    let layout = wallpaper_layout(source.width(), source.height(), target_width, target_height);
    let source = image::DynamicImage::ImageRgba8(source);
    if layout == WallpaperLayout::Cover {
        return source
            .resize_to_fill(
                target_width,
                target_height,
                image::imageops::FilterType::Lanczos3,
            )
            .to_rgba8();
    }

    let preview_width = (target_width / 8).max(1);
    let preview_height = (target_height / 8).max(1);
    let background = source
        .resize_to_fill(
            preview_width,
            preview_height,
            image::imageops::FilterType::Triangle,
        )
        .to_rgba8();
    let background = image::imageops::blur(&background, 8.0);
    let mut canvas = image::DynamicImage::ImageRgba8(background)
        .resize_exact(
            target_width,
            target_height,
            image::imageops::FilterType::CatmullRom,
        )
        .to_rgba8();

    let foreground = if layout == WallpaperLayout::PanoramaBlur {
        // Keep at least 80% of a panorama. A small centre crop reduces the
        // letterbox area while retaining the wide composition.
        let crop_width = ((source.width() as f64 * 0.8).round() as u32)
            .max(1)
            .min(source.width());
        let left = source.width().saturating_sub(crop_width) / 2;
        source.crop_imm(left, 0, crop_width, source.height())
    } else {
        source
    };
    let foreground = foreground
        .resize(
            target_width,
            target_height,
            image::imageops::FilterType::Lanczos3,
        )
        .to_rgba8();
    let x = i64::from(target_width.saturating_sub(foreground.width()) / 2);
    let y = i64::from(target_height.saturating_sub(foreground.height()) / 2);
    image::imageops::overlay(&mut canvas, &foreground, x, y);
    canvas
}

fn apply_wallpaper(path: &std::path::Path) -> anyhow::Result<()> {
    let uri = gio::File::for_path(path).uri();

    let schema_source = gio::SettingsSchemaSource::default()
        .ok_or_else(|| anyhow::anyhow!("Desktop wallpaper settings are unavailable."))?;
    let schema = schema_source
        .lookup("org.gnome.desktop.background", true)
        .ok_or_else(|| {
            anyhow::anyhow!("This desktop does not support setting the wallpaper here.")
        })?;
    if !schema.has_key("picture-uri") {
        anyhow::bail!("This desktop does not expose a wallpaper setting.");
    }

    let settings = gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None);
    settings.set_string("picture-uri", uri.as_str())?;
    if schema.has_key("picture-uri-dark") {
        settings.set_string("picture-uri-dark", uri.as_str())?;
    }
    if schema.has_key("picture-options") {
        settings.set_string("picture-options", "zoom")?;
    }
    gio::Settings::sync();
    Ok(())
}

fn show_open_with_dialog(parent: &gtk::Widget, file: &gio::File) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.set_margin_top(6);
    content.set_margin_bottom(6);
    content.set_margin_start(6);
    content.set_margin_end(6);

    let path = file
        .path()
        .unwrap_or_else(|| std::path::PathBuf::from(file.uri().as_str()));
    let (content_type, _) = gio::content_type_guess(Some(&path), None::<&[u8]>);
    let apps = gio::AppInfo::all_for_type(content_type.as_str());
    let mut app_buttons = Vec::new();
    let dialog = if apps.is_empty() {
        adw::AlertDialog::builder()
            .heading("Open With")
            .body("No applications are registered for this file type.")
            .close_response("close")
            .build()
    } else {
        for app in apps {
            let button = gtk::Button::with_label(app.display_name().as_str());
            button.set_halign(gtk::Align::Fill);
            button.add_css_class("flat");
            app_buttons.push((button.clone(), app));
            content.append(&button);
        }
        let dialog = adw::AlertDialog::builder()
            .heading("Open With")
            .body("Choose an application for this file:")
            .extra_child(&content)
            .close_response("close")
            .build();
        for (button, app) in app_buttons {
            let app_for_launch = app.clone();
            let file_for_launch = file.clone();
            let dialog_for_launch = dialog.clone();
            button.connect_clicked(move |_| {
                let _ = app_for_launch.launch(
                    std::slice::from_ref(&file_for_launch),
                    None::<&gio::AppLaunchContext>,
                );
                dialog_for_launch.close();
            });
        }
        dialog
    };
    dialog.add_response("close", "Cancel");
    dialog.present(Some(parent));
}

fn show_rename_dialog(
    parent: &gtk::Widget,
    photo: crate::photo_object::PhotoObject,
    context: PhotoActionContext,
) {
    let entry = gtk::Entry::new();
    entry.set_text(&photo.filename());
    entry.set_activates_default(true);
    entry.select_region(0, -1);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let label = gtk::Label::new(Some("Enter a new file name:"));
    label.set_xalign(0.0);
    content.append(&label);
    content.append(&entry);

    let dialog = adw::AlertDialog::builder()
        .heading("Rename photo")
        .extra_child(&content)
        .close_response("cancel")
        .default_response("rename")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("rename", "Rename");
    dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);

    let parent_for_response = parent.clone();
    dialog.connect_response(Some("rename"), move |_, _| {
        let new_name = entry.text().trim().to_string();
        if !valid_file_name(&new_name) {
            show_error(
                &parent_for_response,
                "Invalid file name",
                "Enter a file name without folder separators.",
            );
            return;
        }
        if new_name == photo.filename() {
            return;
        }

        let old_name = photo.filename();
        let old_cache = photo.cached_thumbnail_path();
        let fingerprint = db::photo(&context.connection.borrow(), photo.id())
            .ok()
            .flatten()
            .map(|record| (record.mtime, record.size_bytes));
        let source = crate::source::file(&photo.path());
        let renamed = match source.set_display_name(&new_name, gio::Cancellable::NONE) {
            Ok(file) => file,
            Err(error) => {
                show_error(
                    &parent_for_response,
                    "Could not rename photo",
                    &error.to_string(),
                );
                return;
            }
        };
        let new_reference = crate::source::reference(&renamed);
        if let Err(error) =
            db::set_photo_path(&context.connection.borrow(), photo.id(), &new_reference)
        {
            // Keep the filesystem and database referring to the same path if
            // the unique-path update unexpectedly fails.
            let _ = renamed.set_display_name(&old_name, gio::Cancellable::NONE);
            show_error(
                &parent_for_response,
                "Could not rename photo",
                &error.to_string(),
            );
            return;
        }

        photo.set_path(new_reference.as_str());
        photo.set_filename(crate::source::filename(&new_reference));
        if let (Some(old_cache), Some((mtime, size_bytes))) = (old_cache, fingerprint) {
            if let Ok(new_cache) = crate::thumbnail::cache_path(&new_reference, mtime, size_bytes) {
                if let Some(parent) = new_cache.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if std::fs::rename(&old_cache, &new_cache).is_ok() || new_cache.is_file() {
                    let new_cache = new_cache.to_string_lossy().into_owned();
                    photo.set_cached_thumbnail_path(new_cache.as_str());
                }
            }
        }
        photo_changed(&context, PhotoChange::Renamed(&photo));
    });
    dialog.present(Some(parent));
}

fn valid_file_name(name: &str) -> bool {
    if name.is_empty() || matches!(name, "." | "..") {
        return false;
    }
    std::path::Path::new(name)
        .file_name()
        .is_some_and(|file_name| file_name == name)
}

fn show_delete_confirmation(
    parent: &gtk::Widget,
    photo: crate::photo_object::PhotoObject,
    selection: Vec<i64>,
    context: PhotoActionContext,
) {
    // The selection always contains the clicked photo (selected_photo_ids
    // falls back to it), so the single-photo heading can stay personalized.
    let single = selection.len() <= 1;
    let heading = if single {
        format!("Move “{}” to Trash?", photo.filename())
    } else {
        format!("Move {} photos to Trash?", selection.len())
    };
    let body = if single {
        "The photo will be removed from the library and moved to the system Trash."
    } else {
        "The photos will be removed from the library and moved to the system Trash."
    };
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .close_response("cancel")
        .default_response("cancel")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Move to Trash");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);

    let parent_for_response = parent.clone();
    dialog.connect_response(Some("delete"), move |_, _| {
        if let Err((heading, message)) = trash_photos(&context, &selection) {
            show_error(&parent_for_response, heading, &message);
        }
    });
    dialog.present(Some(parent));
}

/// Move the photos to the system Trash and mark them trashed, then report
/// the photos trashed as one photo change. Stops at the first
/// failure and returns the heading and message to show; photos trashed
/// before it are still shown as gone.
fn trash_photos(
    context: &PhotoActionContext,
    selection: &[i64],
) -> Result<(), (&'static str, String)> {
    let mut result = Ok(());
    let mut trashed = Vec::new();
    for id in selection {
        // Resolve the on-disk path from the database instead of trusting
        // grid tiles: menu actions must not depend on tile recycling.
        let path = match db::photo(&context.connection.borrow(), *id) {
            Ok(Some(record)) => record.path,
            Ok(None) => continue,
            Err(error) => {
                result = Err(("Could not delete photo", error.to_string()));
                break;
            }
        };

        if let Err(error) = db::set_trashed(&context.connection.borrow(), *id, true) {
            result = Err(("Could not delete photo", error.to_string()));
            break;
        }

        if let Err(error) = crate::source::file(&path).trash(gio::Cancellable::NONE) {
            let _ = db::set_trashed(&context.connection.borrow(), *id, false);
            result = Err(("Could not move photo to Trash", error.to_string()));
            break;
        }
        trashed.push(*id);
    }

    if result.is_ok() || !trashed.is_empty() {
        photo_changed(context, PhotoChange::Trashed(&trashed));
    }
    result
}

/// Rotate the current photo a quarter turn and report it as one photo
/// change, which shows it everywhere. Returns the photo and its new
/// rotation, for the editor; `None` when there is no current photo or saving
/// failed.
fn rotate_current_photo(
    context: &PhotoActionContext,
    counter_clockwise: bool,
) -> Option<(crate::photo_object::PhotoObject, i32)> {
    let photo = context.info.current_photo()?;
    let rotation = (photo.rotation() + if counter_clockwise { 270 } else { 90 }) % 360;

    if let Err(error) = db::set_rotation(&context.connection.borrow(), photo.id(), rotation) {
        eprintln!("Could not rotate photo: {error}");
        return None;
    }
    photo_changed(context, PhotoChange::Rotated(&photo));
    Some((photo, rotation))
}

fn refresh_photo_actions_grid(context: &PhotoActionContext) {
    let Some(gallery) = context.gallery.borrow().upgrade() else {
        return;
    };
    gallery.refresh_favorite_indicators();
    let search = context.search.borrow().clone();
    refresh_grid(
        &context.connection,
        context.filter.get(),
        &search,
        context.sort.get(),
        &gallery,
    );
}

fn properties_panel_scroll_y(panel: &gtk::Box) -> Option<f64> {
    panel
        .first_child()
        .and_then(|content| content.first_child())
        .and_then(|scroll| scroll.downcast::<gtk::ScrolledWindow>().ok())
        .map(|scroll| scroll.vadjustment().value())
}

/// The Library and Albums index pages are not photo-selection surfaces.
/// Keep this gate authoritative for InfoBar and photo-menu calls alike.
pub(crate) fn page_allows_gallery_properties(page: Option<&str>) -> bool {
    page == Some("photos")
}

/// The gallery panel's contents while no photo is selected, such as
/// between folders; `on_close` runs when its close button is clicked.
fn properties_placeholder(on_close: Rc<dyn Fn()>) -> gtk::Box {
    let empty = gtk::Box::new(gtk::Orientation::Vertical, 12);
    empty.set_valign(gtk::Align::Center);
    empty.set_vexpand(true);
    empty.set_margin_start(20);
    empty.set_margin_end(20);
    let title = gtk::Label::new(Some("Properties"));
    title.add_css_class("heading");
    let hint = gtk::Label::new(Some("Select a photo to view its details"));
    hint.add_css_class("dim-label");
    hint.set_wrap(true);
    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.set_tooltip_text(Some("Close properties"));
    close.set_halign(gtk::Align::End);
    close.add_css_class("flat");
    close.connect_clicked(move |_| on_close());
    empty.append(&close);
    empty.append(&title);
    empty.append(&hint);
    empty
}

/// The Properties widgets for one photo: `content` holds `scroll`, which
/// holds `sections`. A panel that already has a scroller keeps it and swaps
/// in only `sections`, so its scroll position survives the rebuild.
struct PropertiesContent {
    content: gtk::Box,
    scroll: gtk::ScrolledWindow,
    sections: gtk::Box,
}

/// Builds the Properties contents for `photo`; `on_close` runs when the
/// preview's close button is clicked.
fn properties_photo_content(
    photo: &crate::photo_object::PhotoObject,
    context: &PhotoActionContext,
    surface: PropertiesSurface,
    on_close: Rc<dyn Fn()>,
) -> PropertiesContent {
    let record = db::photo(&context.connection.borrow(), photo.id())
        .ok()
        .flatten();
    let path = record
        .as_ref()
        .map(|photo| photo.path.clone())
        .unwrap_or_else(|| photo.path());
    let width = record
        .as_ref()
        .and_then(|photo| photo.width)
        .unwrap_or_else(|| photo.width());
    let height = record
        .as_ref()
        .and_then(|photo| photo.height)
        .unwrap_or_else(|| photo.height());
    let dimensions = if width > 0 && height > 0 {
        format!("{width} × {height}")
    } else {
        "Unknown".to_string()
    };
    let size = record
        .as_ref()
        .and_then(|photo| photo.size_bytes)
        .unwrap_or_else(|| photo.size_bytes());
    let date = record
        .as_ref()
        .and_then(|photo| photo.taken_at.as_deref())
        .map(crate::infobar::format_date)
        .unwrap_or_else(|| "Unknown date".to_string());
    let rotation = record
        .as_ref()
        .map(|photo| photo.rotation)
        .unwrap_or_else(|| photo.rotation());
    let favorite = record
        .as_ref()
        .map(|photo| photo.favorite)
        .unwrap_or_else(|| photo.favorite());

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("scrapbook-inspector-page");
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_vexpand(true);
    content.append(&scroll);
    let sections = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sections.add_css_class("scrapbook-inspector-body");
    sections.add_css_class("properties-body");
    // The sidebar's geometry: a 10 px outer inset, then each block's own
    // 10 px alignment inset (`properties_inset`), so text and icons share
    // the sidebar's leading edge and rows can highlight edge to edge.
    sections.set_margin_start(sidebar::SIDEBAR_ROOT_INSET);
    sections.set_margin_end(sidebar::SIDEBAR_ROOT_INSET);
    sections.set_margin_top(10);
    sections.set_margin_bottom(16);
    sections.set_valign(gtk::Align::Start);
    scroll.set_child(Some(&sections));

    // The paper label is only visible in the Scrapbook skin.
    let scrapbook_selected = gtk::Label::new(Some("Selected Photo"));
    scrapbook_selected.add_css_class("scrapbook-only");
    scrapbook_selected.add_css_class("scrapbook-inspector-heading");
    scrapbook_selected.set_visible(crate::window::theme::scrapbook_is_active());
    scrapbook_selected.set_halign(gtk::Align::Start);
    scrapbook_selected.set_can_target(false);
    sections.append(&scrapbook_selected);

    // Header: the panel title and its close button. It lives in `sections`
    // so a rebuild swaps it with the rest and the scroll restore keeps
    // reading `content > scroll`.
    let header = gtk::Box::new(gtk::Orientation::Horizontal, sidebar::SIDEBAR_ITEM_SPACING);
    header.add_css_class("properties-header");
    header.add_css_class("sidebar-primary-section-heading");
    properties_inset(&header);
    // Uppercase like the sidebar's LIBRARY / ALBUMS / FOLDERS headings.
    let title = gtk::Label::new(Some("PROPERTIES"));
    // The sidebar's section-heading type: 12 px, medium weight, dimmed.
    title.add_css_class("sidebar-section-heading-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    header.append(&title);
    let close = sidebar::sidebar_action_button("window-close-symbolic", Some("Close properties"));
    close.set_focusable(true);
    close.set_focus_on_click(false);
    close.connect_clicked(move |_| on_close());
    header.append(&close);
    sections.append(&header);

    // Inspector layout: photo preview above compact metadata, never side-by-side.
    // Use a fixed-height base child for the preview. GtkPicture's natural
    // aspect-ratio requisition must NOT determine the inspector row height:
    // portrait screenshots used to stretch it to hundreds of pixels.
    let preview_frame = gtk::Overlay::new();
    preview_frame.add_css_class("scrapbook-inspector-preview");
    preview_frame.add_css_class("properties-preview");
    preview_frame.set_hexpand(true);
    preview_frame.set_overflow(gtk::Overflow::Hidden);
    let preview_base = gtk::Box::new(gtk::Orientation::Vertical, 0);
    preview_base.set_size_request(-1, 170);
    preview_frame.set_child(Some(&preview_base));
    if let Some(cache_path) = photo
        .cached_thumbnail_path()
        .filter(|path| std::path::Path::new(path).is_file())
    {
        let preview = gtk::Picture::new();
        preview.set_content_fit(gtk::ContentFit::Contain);
        preview.set_can_shrink(true);
        preview.set_hexpand(true);
        preview.set_vexpand(true);
        preview.set_filename(Some(&cache_path));
        preview_frame.add_overlay(&preview);
    } else {
        let missing = gtk::Image::from_icon_name("image-x-generic-symbolic");
        missing.set_pixel_size(40);
        missing.add_css_class("dim-label");
        preview_frame.add_overlay(&missing);
    }
    properties_inset(&preview_frame);
    sections.append(&preview_frame);

    // Summary: name, date, then type · megapixels · size, then the marks.
    let summary = gtk::Box::new(gtk::Orientation::Vertical, 2);
    summary.add_css_class("properties-summary");
    let filename = photo.filename();
    let name = properties_text(&filename, &["properties-filename"]);
    name.set_wrap(false);
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    name.set_tooltip_text(Some(&filename));
    summary.append(&name);
    summary.append(&properties_text(&date, &["dim-label"]));
    let megapixels = (width > 0 && height > 0)
        .then(|| format!("{:.1} MP", (width as f64 * height as f64) / 1_000_000.0));
    let facts: Vec<String> = [
        Some(file_type(&path)),
        megapixels,
        (size > 0).then(|| crate::infobar::format_size(size)),
    ]
    .into_iter()
    .flatten()
    .collect();
    summary.append(&properties_text(&facts.join(" · "), &["dim-label"]));
    let rating = record.as_ref().map(|item| item.rating).unwrap_or_else(|| photo.rating());
    summary.append(&properties_marks(rating, favorite));
    properties_inset(&summary);
    sections.append(&summary);

    let camera_name = record
        .as_ref()
        .and_then(|item| item.camera.clone())
        .or_else(|| photo.camera());
    let lens_name = record.as_ref().and_then(|item| item.lens.clone());
    // Blank EXIF strings are not meaningful camera metadata.
    let camera_name = camera_name
        .filter(|name| !name.trim().is_empty())
        .map(|name| properties_camera_name(&name));
    let lens_name = lens_name.filter(|name| !name.trim().is_empty());
    let aperture = record
        .as_ref()
        .and_then(|item| item.aperture)
        .or_else(|| Some(photo.aperture()))
        .and_then(format_aperture);
    let shutter = record
        .as_ref()
        .and_then(|item| item.shutter_speed)
        .and_then(format_shutter_speed);
    let iso = record
        .as_ref()
        .and_then(|item| item.iso)
        .filter(|value| *value > 0)
        .map(|value| format!("ISO {value}"));
    let focal_length = record
        .as_ref()
        .and_then(|item| item.focal_length)
        .and_then(format_focal_length);
    let exposure_bias = record
        .as_ref()
        .and_then(|item| item.exposure_bias)
        .and_then(format_exposure_bias);
    // Exposure bias alone (often a default 0.0) is not proof that a
    // wallpaper or other image has actual camera EXIF data.
    let has_camera_data = camera_name.is_some()
        || lens_name.is_some()
        || aperture.is_some()
        || shutter.is_some()
        || iso.is_some()
        || focal_length.is_some();

    let exposure: Vec<&str> = [&aperture, &shutter, &iso]
        .into_iter()
        .filter_map(|value| value.as_deref())
        .collect();
    if camera_name.is_some() || lens_name.is_some() || !exposure.is_empty() {
        sections.append(&properties_divider());
        let camera = gtk::Box::new(gtk::Orientation::Vertical, 2);
        camera.add_css_class("scrapbook-inspector-metadata");
        camera.add_css_class("properties-camera");
        if let Some(name) = camera_name.as_deref() {
            camera.append(&properties_text(name, &["properties-camera-name"]));
        }
        if let Some(name) = lens_name.as_deref() {
            camera.append(&properties_text(name, &["dim-label"]));
        }
        if !exposure.is_empty() {
            camera.append(&properties_text(&exposure.join(" · "), &["properties-exposure"]));
        }
        properties_inset(&camera);
        sections.append(&camera);
    }
    sections.append(&properties_divider());

    // Secondary information collapses; the summary above stays visible.
    let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
    details.add_css_class("scrapbook-inspector-metadata");
    properties_inset(&details);
    add_inspector_row(&details, "Dimensions", &dimensions);
    if let Some(value) = focal_length.as_deref() {
        add_inspector_row(&details, "Focal length", value);
    }
    if has_camera_data {
        if let Some(value) = exposure_bias.as_deref() {
            add_inspector_row(&details, "Exposure", value);
        }
    }
    add_inspector_row(&details, "Rotation", &format!("{rotation}°"));
    sections.append(&properties_section(
        &context.connection,
        PropertiesSection::Details,
        "help-about-symbolic",
        "Details",
        &details,
    ));

    // Display readable spaces but copy the original URI/path exactly
    // (including percent escapes).
    let location = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    location.add_css_class("scrapbook-inspector-location");
    properties_inset(&location);
    let readable_path = path.replace("%20", " ");
    let path_value = gtk::Label::new(Some(&readable_path));
    path_value.set_xalign(0.0);
    path_value.set_valign(gtk::Align::Center);
    path_value.set_hexpand(true);
    path_value.set_wrap(true);
    path_value.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    path_value.set_width_chars(1);
    path_value.set_selectable(false);
    path_value.add_css_class("dim-label");
    location.append(&path_value);
    let copy_path = sidebar::sidebar_action_button("edit-copy-symbolic", Some("Copy full file path"));
    copy_path.set_focusable(true);
    copy_path.set_valign(gtk::Align::Start);
    copy_path.set_focus_on_click(false);
    let original_path = path.clone();
    copy_path.connect_clicked(move |_| {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&original_path);
        }
    });
    location.append(&copy_path);
    sections.append(&properties_section(
        &context.connection,
        PropertiesSection::Location,
        "folder-symbolic",
        "Location",
        &location,
    ));

    sections.append(&scrapbook_inspector_extra(photo, context, surface));
    PropertiesContent { content, scroll, sections }
}

/// A left-aligned, wrapping Properties label with `classes`.
fn properties_text(text: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_width_chars(1);
    for class in classes {
        label.add_css_class(class);
    }
    label
}

fn properties_divider() -> gtk::Separator {
    let divider = gtk::Separator::new(gtk::Orientation::Horizontal);
    divider.add_css_class("properties-divider");
    properties_inset(&divider);
    divider
}

/// The sidebar's alignment inset on both sides of a Properties block.
fn properties_inset(widget: &impl IsA<gtk::Widget>) {
    widget.set_margin_start(sidebar::SIDEBAR_ALIGNMENT_INSET);
    widget.set_margin_end(sidebar::SIDEBAR_CONTENT_INSET);
}

/// A sidebar row's contents: the row padding and alignment insets, with
/// the sidebar's icon-to-label spacing.
fn properties_row_content() -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, sidebar::SIDEBAR_ITEM_SPACING);
    content.set_margin_top(sidebar::SIDEBAR_ROW_PADDING_Y);
    content.set_margin_bottom(sidebar::SIDEBAR_ROW_PADDING_Y);
    properties_inset(&content);
    content
}

/// A sidebar navigation glyph: 16 px in the sidebar's 18 px icon slot.
fn properties_row_icon(icon_name: &str) -> gtk::Image {
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(sidebar::SIDEBAR_NAVIGATION_GLYPH_SIZE);
    icon.set_size_request(sidebar::SIDEBAR_ICON_SIZE, sidebar::SIDEBAR_ICON_SIZE);
    icon.set_valign(gtk::Align::Center);
    icon.add_css_class("sidebar-navigation-icon");
    icon
}

/// The photo's rating and favourite as read-only marks; the InfoBar and
/// the photo menu change them.
fn properties_marks(rating: i32, favorite: bool) -> gtk::Box {
    let marks = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    marks.add_css_class("properties-marks");
    let rating = rating.clamp(0, 5);
    let stars = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    stars.set_hexpand(true);
    for star in 1..=5 {
        let lit = star <= rating;
        // The InfoBar's stars: the same two glyphs in the text colour.
        let icon = gtk::Image::from_icon_name(if lit { "starred-symbolic" } else { "non-starred-symbolic" });
        icon.set_pixel_size(sidebar::SIDEBAR_NAVIGATION_GLYPH_SIZE);
        stars.append(&icon);
    }
    let rating_text = if rating > 0 {
        format!("Rating: {rating} / 5")
    } else {
        "Unrated".to_string()
    };
    stars.set_tooltip_text(Some(&rating_text));
    stars.update_property(&[gtk::accessible::Property::Label(&rating_text)]);
    marks.append(&stars);
    // The InfoBar's heart: each theme's favourite red (`favorite-badge`,
    // the colour the InfoBar's active heart uses) or dimmed like the
    // InfoBar's inactive heart (`properties-favorite`, base.css).
    let heart = gtk::Image::from_icon_name("emote-love-symbolic");
    heart.set_pixel_size(sidebar::SIDEBAR_NAVIGATION_GLYPH_SIZE);
    heart.add_css_class(if favorite { "favorite-badge" } else { "properties-favorite" });
    let favorite_text = if favorite { "Favourite" } else { "Not a favourite" };
    heart.set_tooltip_text(Some(favorite_text));
    heart.update_property(&[gtk::accessible::Property::Label(favorite_text)]);
    marks.append(&heart);
    marks
}

/// Display name for a stored camera string. Older imports keep EXIF Make
/// and Model together ("NIKON CORPORATION NIKON Z 5"); show the brand once,
/// in its usual spelling ("Nikon Z 5"). The stored value is unchanged.
fn properties_camera_name(raw: &str) -> String {
    const BRANDS: &[(&str, &str)] = &[
        ("nikon corporation", "Nikon"),
        ("nikon", "Nikon"),
        ("canon inc.", "Canon"),
        ("canon inc", "Canon"),
        ("canon", "Canon"),
        ("sony corporation", "Sony"),
        ("sony", "Sony"),
        ("olympus imaging corp.", "Olympus"),
        ("olympus corporation", "Olympus"),
        ("olympus", "Olympus"),
        ("om digital solutions", "OM System"),
        ("fujifilm corporation", "Fujifilm"),
        ("fujifilm", "Fujifilm"),
        ("panasonic", "Panasonic"),
        ("leica camera ag", "Leica"),
        ("leica", "Leica"),
        ("pentax corporation", "Pentax"),
        ("pentax", "Pentax"),
        ("ricoh imaging company, ltd.", "Ricoh"),
        ("ricoh", "Ricoh"),
        ("samsung", "Samsung"),
        ("hasselblad", "Hasselblad"),
    ];
    let mut rest = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut brand = None;
    // Strip every leading brand spelling: "NIKON CORPORATION NIKON Z 5"
    // carries two.
    'strip: loop {
        let lower = rest.to_lowercase();
        for (prefix, name) in BRANDS {
            let matched = lower == *prefix
                || lower
                    .strip_prefix(prefix)
                    .is_some_and(|tail| tail.starts_with(' '));
            if matched {
                rest = rest[prefix.len()..].trim_start().to_string();
                brand = Some(*name);
                continue 'strip;
            }
        }
        break;
    }
    match brand {
        Some(brand) if rest.is_empty() => brand.to_string(),
        Some(brand) => format!("{brand} {rest}"),
        None => rest,
    }
}

/// The collapsible Properties sections, as bits of the saved setting.
#[derive(Clone, Copy)]
enum PropertiesSection {
    Details = 1,
    Location = 2,
    Tags = 4,
    Notes = 8,
}

/// The library setting that remembers which Properties sections are open.
const PROPERTIES_SECTIONS_SETTING_KEY: &str = "properties-open-sections";

/// Which sections are open; Details and Tags & albums until the user
/// changes it.
fn properties_open_sections(connection: &Connection) -> u8 {
    db::setting(connection, PROPERTIES_SECTIONS_SETTING_KEY)
        .ok()
        .flatten()
        .and_then(|value| value.parse().ok())
        .unwrap_or(PropertiesSection::Details as u8 | PropertiesSection::Tags as u8)
}

/// A collapsible section: a full-width header row (icon, title, chevron)
/// over `body`. Which sections are open is saved in the library, so moving
/// between photos and restarting keep the sections the user opened.
fn properties_section(
    connection: &Rc<RefCell<Connection>>,
    section: PropertiesSection,
    icon_name: &str,
    title: &str,
    body: &impl IsA<gtk::Widget>,
) -> gtk::Box {
    let bit = section as u8;
    let expanded = properties_open_sections(&connection.borrow()) & bit != 0;
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("properties-collapsible");

    let header = gtk::Button::new();
    header.add_css_class("flat");
    header.add_css_class("properties-collapsible-header");
    header.set_margin_top(sidebar::SIDEBAR_ROW_MARGIN_Y);
    header.set_margin_bottom(sidebar::SIDEBAR_ROW_MARGIN_Y);
    let row = properties_row_content();
    row.append(&properties_row_icon(icon_name));
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    label.set_valign(gtk::Align::Center);
    label.set_hexpand(true);
    row.append(&label);
    // The sidebar headings' disclosure: pan-end closed, pan-down open, a
    // 14 px glyph centred in the 28 px action column.
    let chevron_name = |open: bool| if open { "pan-down-symbolic" } else { "pan-end-symbolic" };
    let chevron = gtk::Image::from_icon_name(chevron_name(expanded));
    chevron.set_pixel_size(sidebar::SIDEBAR_ACTION_ICON_SIZE);
    chevron.set_size_request(sidebar::SIDEBAR_CONTROL_SIZE, -1);
    chevron.set_valign(gtk::Align::Center);
    chevron.add_css_class("properties-section-chevron");
    row.append(&chevron);
    header.set_child(Some(&row));
    header.update_state(&[gtk::accessible::State::Expanded(Some(expanded))]);

    let revealer = gtk::Revealer::new();
    revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    revealer.set_reveal_child(expanded);
    body.add_css_class("properties-collapsible-body");
    revealer.set_child(Some(body));

    let revealer_for_click = revealer.clone();
    let connection = connection.clone();
    header.connect_clicked(move |header| {
        let open = !revealer_for_click.reveals_child();
        revealer_for_click.set_reveal_child(open);
        chevron.set_icon_name(Some(chevron_name(open)));
        header.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
        let connection = connection.borrow();
        let sections = properties_open_sections(&connection);
        let sections = if open { sections | bit } else { sections & !bit };
        if let Err(error) =
            db::set_setting(&connection, PROPERTIES_SECTIONS_SETTING_KEY, &sections.to_string())
        {
            eprintln!("Could not save the open Properties sections: {error:#}");
        }
    });
    root.append(&header);
    root.append(&revealer);
    root
}

/// A flat Properties action laid out like a sidebar row: glyph, then label.
fn properties_action_button(icon_name: &str, label: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("properties-action");
    let content = properties_row_content();
    content.append(&properties_row_icon(icon_name));
    content.append(&gtk::Label::new(Some(label)));
    button.set_child(Some(&content));
    button
}

/// The Tags & albums items' spacing: 8 px between items, 4 px between lines.
const PROPERTIES_ITEM_GAP: i32 = 8;
const PROPERTIES_ITEM_LINE_GAP: i32 = 4;

/// Where each visible child of a Tags & albums group goes at `width`: in a
/// row at its natural width (at most the whole width, so a long name
/// ellipsizes), wrapping to the next line when it does not fit. A negative
/// width lays everything out on one line.
fn properties_wrap_items(group: &gtk::Widget, width: i32) -> Vec<(gtk::Widget, gtk::Allocation)> {
    let mut placed = Vec::new();
    let (mut x, mut y, mut line_height) = (0, 0, 0);
    let mut child = group.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if !widget.should_layout() {
            continue;
        }
        let (min, natural, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
        let item_width = if width < 0 {
            natural
        } else {
            natural.min(width).max(min)
        };
        if x > 0 && width >= 0 && x + item_width > width {
            x = 0;
            y += line_height + PROPERTIES_ITEM_LINE_GAP;
            line_height = 0;
        }
        let (_, height, _, _) = widget.measure(gtk::Orientation::Vertical, item_width);
        placed.push((widget, gtk::Allocation::new(x, y, item_width, height)));
        x += item_width + PROPERTIES_ITEM_GAP;
        line_height = line_height.max(height);
    }
    placed
}

mod properties_wrap_layout {
    use super::{gtk, properties_wrap_items, PROPERTIES_ITEM_GAP};
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub(crate) struct PropertiesWrapLayout;

    #[glib::object_subclass]
    impl ObjectSubclass for PropertiesWrapLayout {
        const NAME: &'static str = "PicPropertiesWrapLayout";
        type Type = super::PropertiesWrapLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for PropertiesWrapLayout {}

    impl LayoutManagerImpl for PropertiesWrapLayout {
        fn request_mode(&self, _: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(
            &self,
            group: &gtk::Widget,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                let (mut min, mut natural) = (0, 0);
                let mut child = group.first_child();
                while let Some(widget) = child {
                    child = widget.next_sibling();
                    if widget.should_layout() {
                        let (item_min, item_natural, _, _) =
                            widget.measure(gtk::Orientation::Horizontal, -1);
                        min = min.max(item_min);
                        natural += item_natural + if natural > 0 { PROPERTIES_ITEM_GAP } else { 0 };
                    }
                }
                (min, natural, -1, -1)
            } else {
                let height = properties_wrap_items(group, for_size)
                    .iter()
                    .map(|(_, at)| at.y() + at.height())
                    .max()
                    .unwrap_or(0);
                (height, height, -1, -1)
            }
        }

        fn allocate(&self, group: &gtk::Widget, width: i32, _: i32, _: i32) {
            for (widget, at) in properties_wrap_items(group, width) {
                widget.size_allocate(&at, -1);
            }
        }
    }
}

glib::wrapper! {
    /// Lays a Tags & albums group's items out like words: see
    /// `properties_wrap_items`.
    pub(crate) struct PropertiesWrapLayout(ObjectSubclass<properties_wrap_layout::PropertiesWrapLayout>)
        @extends gtk::LayoutManager;
}

/// A Tags & albums group: content-sized items that wrap like words (GTK
/// 4.12 / libadwaita 1.5 have no wrap box; a FlowBox aligns columns).
fn properties_item_group(css_class: &str) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    group.add_css_class(css_class);
    // The items' own padding sits outside the section's text inset, so
    // their glyphs line up with the action rows' glyphs below.
    group.set_margin_start(sidebar::SIDEBAR_ALIGNMENT_INSET - 6);
    group.set_margin_end(sidebar::SIDEBAR_CONTENT_INSET);
    group.set_layout_manager(Some(glib::Object::new::<PropertiesWrapLayout>()));
    group
}

/// One flat Tags & albums item: a tiny glyph and the name, then (when it
/// can be removed) a small muted remove button. With `open`, the glyph and
/// name are a button that opens it. Both buttons take keyboard focus.
fn properties_item(
    icon_name: &str,
    name: &str,
    open: Option<(&str, Box<dyn Fn(&gtk::Button)>)>,
    remove: Option<(String, Box<dyn Fn()>)>,
) -> gtk::Box {
    let item = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    item.add_css_class("properties-item");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(12);
    icon.set_valign(gtk::Align::Center);
    icon.add_css_class("properties-item-icon");
    content.append(&icon);
    let label = gtk::Label::new(Some(name));
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_xalign(0.0);
    content.append(&label);
    match open {
        Some((tooltip, on_open)) => {
            let button = gtk::Button::new();
            button.add_css_class("flat");
            button.add_css_class("properties-item-open");
            button.set_child(Some(&content));
            button.set_tooltip_text(Some(tooltip));
            button.set_hexpand(true);
            button.connect_clicked(move |button| on_open(button));
            item.append(&button);
        }
        None => {
            label.set_tooltip_text(Some(name));
            content.set_hexpand(true);
            item.append(&content);
        }
    }
    if let Some((tooltip, on_remove)) = remove {
        let button = gtk::Button::from_icon_name("window-close-symbolic");
        button.add_css_class("flat");
        button.add_css_class("properties-item-remove");
        button.set_valign(gtk::Align::Center);
        button.set_tooltip_text(Some(&tooltip));
        button.update_property(&[gtk::accessible::Property::Label(&tooltip)]);
        button.connect_clicked(move |_| on_remove());
        item.append(&button);
    }
    item
}

/// Global photo tags are always available from the Properties inspector,
/// including outside Scrapbook. Only the extra paper Notes/Actions layout
/// is skin-specific. The same SQLite tags power the global metadata filter.
fn populate_properties_tags(tag_rows: &gtk::Box, context: &PhotoActionContext, photo_id: i64) {
    while let Some(child) = tag_rows.first_child() {
        tag_rows.remove(&child);
    }
    let (tags, _) =
        db::scrapbook_details(&context.connection.borrow(), photo_id).unwrap_or_default();
    tag_rows.set_visible(!tags.is_empty());
    for tag in tags {
        let context = context.clone();
        let rows = tag_rows.downgrade();
        let remove_tag = tag.clone();
        let remove: Box<dyn Fn()> = Box::new(move || {
            if let Err(error) =
                db::scrapbook_remove_tag(&context.connection.borrow(), photo_id, &remove_tag)
            {
                eprintln!("PIC Scrapbook: could not remove tag: {error:#}");
            } else {
                if let Some(rows) = rows.upgrade() {
                    populate_properties_tags(&rows, &context, photo_id);
                }
                photo_changed(&context, PhotoChange::Tagged(photo_id));
            }
        });
        tag_rows.append(&properties_item(
            "user-bookmarks-symbolic",
            &tag,
            None,
            Some((format!("Remove tag {tag}"), remove)),
        ));
    }
}

fn properties_add_tag_button(
    tag_rows: &gtk::Box,
    context: &PhotoActionContext,
    photo_id: i64,
) -> gtk::Button {
    let add_tag = properties_action_button("list-add-symbolic", "Add tag");
    {
        let context = context.clone();
        let rows_for_dialog = tag_rows.downgrade();
        add_tag.connect_clicked(move |_| {
            let dialog = adw::AlertDialog::new(Some("Add a photo tag"), Some("Keep your scrapbook organised with short tags."));
            let entry = gtk::Entry::new();
            entry.set_placeholder_text(Some("e.g. Family, Holiday, Project"));
            entry.set_max_length(40);
            let selector = gtk::Box::new(gtk::Orientation::Vertical, 7);
            let existing = db::scrapbook_existing_tags(&context.connection.borrow())
                .unwrap_or_default();
            if !existing.is_empty() {
                let mut labels = vec!["Choose an existing tag…".to_string()];
                labels.extend(existing);
                let names: Vec<&str> = labels.iter().map(String::as_str).collect();
                let dropdown = gtk::DropDown::from_strings(&names);
                dropdown.set_tooltip_text(Some("Use a tag already assigned to other photos"));
                let entry_for_dropdown = entry.clone();
                dropdown.connect_selected_notify(move |dropdown| {
                    let index = dropdown.selected() as usize;
                    if let Some(name) = labels.get(index).filter(|_| index > 0) {
                        entry_for_dropdown.set_text(name);
                    }
                });
                selector.append(&dropdown);
            }
            selector.append(&entry);
            dialog.set_extra_child(Some(&selector));
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("add", "Add tag");
            dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
            let connection = context.connection.clone();
            let context_for_response = context.clone();
            let rows_for_response = rows_for_dialog.clone();
            dialog.connect_response(None, move |_, response| {
                if response == "add" {
                    if let Err(error) = db::scrapbook_add_tag(
                        &connection.borrow(), photo_id, entry.text().as_str()
                    ) {
                        eprintln!("PIC Scrapbook: could not add tag: {error:#}");
                    } else {
                        if let Some(rows) = rows_for_response.upgrade() {
                            populate_properties_tags(&rows, &context_for_response, photo_id);
                        }
                        photo_changed(&context_for_response, PhotoChange::Tagged(photo_id));
                    }
                }
            });
            let window = context.window.upgrade();
            dialog.present(window.as_ref());
        });
    }
    add_tag
}

/// The albums and Smart Albums this photo is in, under its tags. Each
/// opens its album as its sidebar row does (like the photo menu's In Albums
/// rows); a manual album's remove button takes the photo out of it. Smart
/// Albums are read-only photo sources, so theirs has none. The group is
/// hidden when the photo is in none.
fn populate_properties_albums(album_rows: &gtk::Box, context: &PhotoActionContext, photo_id: i64) {
    while let Some(child) = album_rows.first_child() {
        album_rows.remove(&child);
    }
    let containing =
        db::albums_containing_photo(&context.connection.borrow(), photo_id).unwrap_or_default();
    album_rows.set_visible(!containing.is_empty());
    for album in containing {
        let album_id = album.id;
        let open: Box<dyn Fn(&gtk::Button)> = Box::new(move |button| {
            let _ = button.activate_action(OPEN_ALBUM_ACTION, Some(&album_id.to_variant()));
        });
        let remove = (!album.is_smart()).then(|| {
            let context = context.clone();
            let rows = album_rows.downgrade();
            let remove: Box<dyn Fn()> = Box::new(move || {
                remove_photo_from_album_in_properties(&context, &rows, album_id, photo_id);
            });
            (format!("Remove from album {}", album.name), remove)
        });
        album_rows.append(&properties_item(
            if album.is_smart() {
                sidebar::smart_album_icon_name()
            } else {
                sidebar::album_icon_name()
            },
            &album.name,
            Some((
                if album.is_smart() {
                    "Open this Smart Album"
                } else {
                    "Open this album"
                },
                open,
            )),
            remove,
        ));
    }
}

/// An album item's remove button: takes the photo out of that manual
/// album, as the photo menu's Remove from Album does, then refills the
/// albums. While that album is being viewed, the photo also leaves the gallery.
fn remove_photo_from_album_in_properties(
    context: &PhotoActionContext,
    album_rows: &glib::WeakRef<gtk::Box>,
    album_id: i64,
    photo_id: i64,
) {
    let ids = [photo_id];
    if let Err(error) = db::remove_photos_from_album(&context.connection.borrow(), album_id, &ids) {
        if let Some(rows) = album_rows.upgrade() {
            show_error(
                rows.upcast_ref(),
                "Could not remove from album",
                &error.to_string(),
            );
        }
        return;
    }
    if let Some(rows) = album_rows.upgrade() {
        populate_properties_albums(&rows, context, photo_id);
    }
    if context.filter.get() == sidebar::SidebarFilter::Album(album_id) {
        if let Some(lightbox) = context.lightbox.upgrade() {
            lightbox.close();
        }
        if let Some(gallery) = context.gallery.borrow().upgrade() {
            gallery.remove_photos(&ids);
        }
    }
    albums_edited(context);
}

/// "Add to album" beside "Add tag": the one Add to Album list, for this photo.
/// Adding it to an album refreshes the album items.
fn properties_add_to_album_button(
    album_rows: &gtk::Box,
    context: &PhotoActionContext,
    photo_id: i64,
) -> gtk::Button {
    let add_to_album = properties_action_button("folder-new-symbolic", "Add to album");
    add_to_album.set_tooltip_text(Some("Add to Album"));
    let context = context.clone();
    let album_rows = album_rows.downgrade();
    add_to_album.connect_clicked(move |button| {
        let ids: Rc<dyn Fn() -> Vec<i64>> = Rc::new(move || vec![photo_id]);
        let refresh_context = context.clone();
        let refresh_rows = album_rows.clone();
        let refresh: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(rows) = refresh_rows.upgrade() {
                populate_properties_albums(&rows, &refresh_context, photo_id);
            }
        });
        show_album_popover(button.upcast_ref(), None, context.clone(), ids, refresh);
    });
    add_to_album
}

fn scrapbook_inspector_extra(
    photo: &crate::photo_object::PhotoObject,
    context: &PhotoActionContext,
    _surface: PropertiesSurface,
) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("photo-tags-global");
    root.add_css_class("properties-section");
    root.add_css_class("scrapbook-inspector-extra");
    root.set_visible(true);
    let photo_id = photo.id();
    let (_, note) = db::scrapbook_details(&context.connection.borrow(), photo_id)
        .unwrap_or_default();

    let tags = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let tag_rows = properties_item_group("properties-tags");
    populate_properties_tags(&tag_rows, context, photo_id);
    tags.append(&tag_rows);
    let album_rows = properties_item_group("properties-albums");
    populate_properties_albums(&album_rows, context, photo_id);
    tags.append(&album_rows);
    // The action rows are not inset: their own row padding lines their
    // glyphs up with the section icons above.
    let tag_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tag_actions.add_css_class("properties-actions");
    tag_actions.append(&properties_add_tag_button(&tag_rows, context, photo_id));
    tag_actions.append(&properties_add_to_album_button(
        &album_rows,
        context,
        photo_id,
    ));
    tags.append(&tag_actions);
    root.append(&properties_section(
        &context.connection,
        PropertiesSection::Tags,
        "user-bookmarks-symbolic",
        "Tags & albums",
        &tags,
    ));

    let note_card = gtk::Box::new(gtk::Orientation::Vertical, 7);
    note_card.add_css_class("scrapbook-note-card");
    properties_inset(&note_card);
    let text = gtk::TextView::new();
    text.add_css_class("scrapbook-note-edit");
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.set_size_request(-1, 76);
    text.set_top_margin(7);
    text.set_bottom_margin(7);
    text.set_left_margin(8);
    text.set_right_margin(8);
    let buffer = text.buffer();
    buffer.set_text(&note);
    note_card.append(&text);
    // Save note appears only while the text differs from the saved note.
    let note_save = gtk::Button::with_label("Save note");
    note_save.add_css_class("scrapbook-note-save");
    note_save.set_visible(false);
    let status = gtk::Label::new(None);
    status.add_css_class("scrapbook-note-status");
    status.add_css_class("dim-label");
    status.add_css_class("caption");
    status.set_xalign(0.0);
    status.set_hexpand(true);
    let note_footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    note_footer.append(&status);
    note_footer.append(&note_save);
    note_card.append(&note_footer);
    let saved_note = Rc::new(RefCell::new(note.clone()));
    {
        let saved_note = saved_note.clone();
        let note_save = note_save.clone();
        let status = status.clone();
        buffer.connect_changed(move |buffer| {
            let (start, end) = buffer.bounds();
            let changed = buffer.text(&start, &end, true).as_str() != saved_note.borrow().as_str();
            note_save.set_visible(changed);
            if changed {
                status.set_label("");
            }
        });
    }
    {
        let connection = context.connection.clone();
        note_save.connect_clicked(move |button| {
            let (start, end) = buffer.bounds();
            let note = buffer.text(&start, &end, true);
            match db::scrapbook_set_note(&connection.borrow(), photo_id, note.as_str()) {
                Ok(()) => {
                    *saved_note.borrow_mut() = note.to_string();
                    button.set_visible(false);
                    status.set_label("Saved ✓");
                }
                Err(error) => status.set_label(&format!("Could not save: {error}")),
            }
        });
    }
    root.append(&properties_section(
        &context.connection,
        PropertiesSection::Notes,
        "document-edit-symbolic",
        "Notes",
        &note_card,
    ));

    let action_section = gtk::Box::new(gtk::Orientation::Vertical, 9);
    action_section.add_css_class("scrapbook-only");
    action_section.set_visible(crate::window::theme::scrapbook_is_active());
    let actions_header = gtk::Label::new(Some("Actions"));
    actions_header.add_css_class("scrapbook-inspector-paper-title");
    actions_header.set_halign(gtk::Align::Start);
    action_section.append(&actions_header);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    actions.add_css_class("scrapbook-inspector-actions");
    let make_action = |label: &str, icon_name: &str| {
        let button = gtk::Button::new();
        button.add_css_class("scrapbook-inspector-action");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
        content.set_halign(gtk::Align::Center);
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.set_pixel_size(20);
        content.append(&icon);
        content.append(&gtk::Label::new(Some(label)));
        button.set_child(Some(&content));
        button
    };
    let edit = make_action("Edit", "document-edit-symbolic");
    let edit_action = context.open_edit.clone();
    edit.connect_clicked(move |_| edit_action(photo_id));
    actions.append(&edit);
    let new_album = make_action("Album", "folder-new-symbolic");
    let album_context = context.clone();
    new_album.connect_clicked(move |button| {
        let ids: Rc<dyn Fn() -> Vec<i64>> = Rc::new(move || vec![photo_id]);
        show_album_popover(button.upcast_ref(), None, album_context.clone(), ids, Rc::new(|| {}));
    });
    actions.append(&new_album);
    let export = make_action("Export", "document-save-symbolic");
    let export_action = context.export.clone();
    let export_photo = photo.clone();
    export.connect_clicked(move |_| export_action(&export_photo));
    actions.append(&export);
    action_section.append(&actions);
    root.append(&action_section);
    root
}

fn add_inspector_row(group: &gtk::Box, name: &str, value: &str) {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    line.set_margin_start(0);
    line.set_margin_end(0);
    line.set_margin_top(4);
    line.set_margin_bottom(4);

    let key = gtk::Label::new(Some(name));
    key.set_xalign(0.0);
    key.set_valign(gtk::Align::Center);
    // Descriptive Camera/Lens values need a narrow label column; the
    // generic 11-character column leaves an excessive gap at 260–300px.
    if matches!(name, "Camera" | "Lens") {
        key.set_width_chars(0);
        key.set_size_request(60, -1);
    } else {
        key.set_width_chars(11);
    }
    key.add_css_class("dim-label");
    line.append(&key);

    let detail = gtk::Label::new(Some(value));
    detail.set_hexpand(true);
    // Camera and Lens are descriptive text, not right-aligned metrics.
    // Start their values directly after the key and wrap to the left.
    detail.set_xalign(if matches!(name, "Camera" | "Lens") { 0.0 } else { 1.0 });
    detail.set_valign(gtk::Align::Center);
    detail.set_wrap(true);
    detail.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    detail.set_width_chars(1);
    detail.set_max_width_chars(28);
    detail.set_selectable(false);
    line.append(&detail);

    group.append(&line);
}

fn format_aperture(value: f64) -> Option<String> {
    (value.is_finite() && value > 0.0).then(|| {
        format!("f/{value:.1}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    })
}

pub(crate) fn format_shutter_speed(value: f64) -> Option<String> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    Some(if value < 0.5 {
        format!("1/{} s", (1.0 / value).round() as u64)
    } else {
        format!("{} s", format_decimal(value))
    })
}

fn format_decimal(value: f64) -> String {
    format!("{value:.1}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

pub(crate) fn format_focal_length(value: f64) -> Option<String> {
    (value.is_finite() && value > 0.0).then(|| format!("{} mm", format_decimal(value)))
}

fn format_exposure_bias(value: f64) -> Option<String> {
    value.is_finite().then(|| format!("{value:+.1} EV"))
}

fn file_type(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_uppercase())
        .unwrap_or_else(|| "Unknown".to_string())
}

fn show_error(parent: &gtk::Widget, heading: &str, message: &str) {
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(message)
        .close_response("close")
        .build();
    dialog.add_response("close", "Close");
    dialog.present(Some(parent));
}

#[cfg(test)]
mod photo_print_request_tests {
    use super::*;

    #[test]
    fn print_requests_read_library_photos_fresh_and_skip_unknown_ids() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(db::SCHEMA).unwrap();
        connection
            .execute(
                "INSERT INTO photos (id, path, width, height, rotation, edit_recipe)
                 VALUES (7, '/p/IMG_7.jpg', 4000, 3000, 90, 'recipe')",
                [],
            )
            .unwrap();
        let requests = photo_print_requests(&connection, &[7, 99]);
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.job_name, "IMG_7.jpg");
        assert_eq!(request.reference, "/p/IMG_7.jpg");
        assert_eq!((request.source_width, request.source_height), (4000, 3000));
        assert_eq!(request.rotation, 90);
        assert_eq!(request.edit_recipe, "recipe");
    }

    fn library_object(id: i64, path: &str) -> crate::photo_object::PhotoObject {
        glib::Object::builder()
            .property("id", id)
            .property("path", path)
            .build()
    }

    fn paths(jobs: &[crate::edit::export_batch::ExportJob]) -> Vec<&str> {
        jobs.iter().map(|job| job.source_path.as_str()).collect()
    }

    #[test]
    fn export_targets_the_selection_or_the_anchor_even_outside_the_gallery_list() {
        let a = library_object(1, "/p/a.jpg");
        let b = library_object(2, "/p/b.jpg");
        let gallery = || vec![a.clone(), b.clone()];
        // The selection containing the anchor, in selection order.
        assert_eq!(
            paths(&anchored_export_jobs(&a, &[2, 1], gallery())),
            ["/p/b.jpg", "/p/a.jpg"]
        );
        // The anchor is not in the gallery's list (the viewer shows another
        // list): it is exported alone instead of nothing happening.
        let elsewhere = library_object(9, "/p/elsewhere.jpg");
        assert_eq!(
            paths(&anchored_export_jobs(&elsewhere, &[9], gallery())),
            ["/p/elsewhere.jpg"]
        );
        // A photo outside the library is exported alone from its object.
        let outside = crate::photo_object::PhotoObject::outside_library(std::path::Path::new(
            "/elsewhere/outside.jpg",
        ));
        let ids = [outside.id()];
        assert_eq!(
            paths(&anchored_export_jobs(&outside, &ids, gallery())),
            ["/elsewhere/outside.jpg"]
        );
    }

    #[test]
    fn print_targets_an_outside_photo_from_its_object() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(db::SCHEMA).unwrap();
        let outside = crate::photo_object::PhotoObject::outside_library(std::path::Path::new(
            "/elsewhere/outside.jpg",
        ));
        let requests = anchored_print_requests(&connection, &outside, &[outside.id()]);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].reference, "/elsewhere/outside.jpg");
    }
}

#[cfg(test)]
mod photo_actions_tests {
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn photo_context_menu_uses_visible_lightbox_overlay_not_covered_gallery() {
        use gtk::prelude::*;

        gtk::init().expect("GTK display needed");
        let gallery = gtk::Overlay::new();
        let grid_anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
        gallery.set_child(Some(&grid_anchor));

        let lightbox = gtk::Overlay::new();
        let viewer_anchor = gtk::ScrolledWindow::new();
        lightbox.set_child(Some(&viewer_anchor));
        lightbox.set_visible(false);

        // Mirrors PIC: the lightbox is above the gallery split.
        let surface = gtk::Overlay::new();
        surface.set_child(Some(&gallery));
        surface.add_overlay(&lightbox);
        let window = gtk::Window::builder()
            .default_width(640)
            .default_height(480)
            .child(&surface)
            .build();
        window.present();
        settle_gtk_layout();

        let grid_host = photo_context_menu_host_for_anchor(
            grid_anchor.upcast_ref(),
            &gallery,
            Some(&lightbox),
        );
        assert_eq!(grid_host, gallery);
        lightbox.set_visible(true);
        settle_gtk_layout();

        // Crucial regression: a lightbox menu on the gallery overlay is
        // hidden underneath the fullscreen viewer.
        let viewer_host = photo_context_menu_host_for_anchor(
            viewer_anchor.upcast_ref(),
            &gallery,
            Some(&lightbox),
        );
        assert_eq!(viewer_host, lightbox);
        let grid_host = photo_context_menu_host_for_anchor(
            grid_anchor.upcast_ref(),
            &gallery,
            Some(&lightbox),
        );
        assert_eq!(grid_host, gallery);

        lightbox.set_visible(false);
        assert_eq!(
            photo_context_menu_host_for_anchor(
                viewer_anchor.upcast_ref(),
                &gallery,
                Some(&lightbox),
            ),
            gallery
        );
        window.close();
    }

    use super::*;

    #[test]
    fn library_and_album_indexes_do_not_allow_properties() {
        assert!(page_allows_gallery_properties(Some("photos")));
        for page in ["library", "albums", "edit", "collage"] {
            assert!(!page_allows_gallery_properties(Some(page)));
        }
        assert!(!page_allows_gallery_properties(None));
    }

    #[test]
    fn properties_camera_names_show_the_brand_once() {
        for (stored, shown) in [
            ("NIKON CORPORATION NIKON Z 5", "Nikon Z 5"),
            ("NIKON Z 5", "Nikon Z 5"),
            ("Canon EOS R5", "Canon EOS R5"),
            ("Canon Inc. Canon EOS 5D", "Canon EOS 5D"),
            ("SONY ILCE-7M3", "Sony ILCE-7M3"),
            ("FUJIFILM X-T4", "Fujifilm X-T4"),
            ("OLYMPUS IMAGING CORP.  E-M5", "Olympus E-M5"),
            ("NIKON CORPORATION", "Nikon"),
            ("Apple iPhone 15 Pro", "Apple iPhone 15 Pro"),
            ("Nikonos V", "Nikonos V"),
        ] {
            assert_eq!(properties_camera_name(stored), shown, "{stored}");
        }
    }

    #[test]
    fn properties_sections_open_details_and_tags_until_changed() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(db::SCHEMA).unwrap();
        assert_eq!(
            properties_open_sections(&connection),
            PropertiesSection::Details as u8 | PropertiesSection::Tags as u8
        );
        db::set_setting(&connection, PROPERTIES_SECTIONS_SETTING_KEY, "12").unwrap();
        assert_eq!(
            properties_open_sections(&connection),
            PropertiesSection::Tags as u8 | PropertiesSection::Notes as u8
        );
    }

    #[test]
    fn photographic_properties_values() {
        assert_eq!(format_aperture(4.0).as_deref(), Some("f/4"));
        assert_eq!(format_shutter_speed(0.001).as_deref(), Some("1/1000 s"));
        assert_eq!(format_shutter_speed(1.5).as_deref(), Some("1.5 s"));
        assert_eq!(format_shutter_speed(0.7).as_deref(), Some("0.7 s"));
        assert_eq!(format_focal_length(85.0).as_deref(), Some("85 mm"));
        assert_eq!(format_exposure_bias(0.7).as_deref(), Some("+0.7 EV"));
        assert_eq!(format_exposure_bias(0.0).as_deref(), Some("+0.0 EV"));
        assert_eq!(format_aperture(0.0), None);
        assert_eq!(format_shutter_speed(f64::NAN), None);
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn gallery_properties_split_resizes_content_without_covering_thumbnails() {
        adw::init().unwrap();

        // The right-hand inspector uses the same structural animated split
        // as the left sidebar. Opening it must subtract its allocated width
        // from the gallery, not paint over the existing thumbnails.
        let gallery = gtk::Box::new(gtk::Orientation::Vertical, 0);
        gallery.set_hexpand(true);
        gallery.set_vexpand(true);
        let inspector = gtk::Box::new(gtk::Orientation::Vertical, 0);
        inspector.set_width_request(280);
        let split = adw::OverlaySplitView::new();
        split.set_sidebar_position(gtk::PackType::End);
        split.set_pin_sidebar(true);
        split.set_collapsed(false);
        split.set_min_sidebar_width(280.0);
        split.set_max_sidebar_width(520.0);
        split.set_sidebar_width_fraction(0.25);
        split.set_content(Some(&gallery));
        split.set_sidebar(Some(&inspector));
        split.set_show_sidebar(false);

        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&split));
        let viewer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        viewer.set_hexpand(true);
        viewer.set_vexpand(true);
        viewer.set_visible(false);
        overlay.add_overlay(&viewer);
        let viewer_inspector = gtk::Box::new(gtk::Orientation::Vertical, 0);
        viewer_inspector.set_width_request(300);
        viewer_inspector.set_halign(gtk::Align::End);
        viewer_inspector.set_valign(gtk::Align::Fill);
        viewer_inspector.set_visible(false);
        overlay.add_overlay(&viewer_inspector);

        let window = gtk::Window::builder()
            .default_width(1200)
            .default_height(700)
            .child(&overlay)
            .build();
        window.present();
        for _ in 0..10 {
            settle_gtk_layout();
        }
        let full_width = gallery.width();
        assert!(full_width > 700, "gallery must start at full width");

        split.set_show_sidebar(true);
        for _ in 0..10 {
            settle_gtk_layout();
        }
        assert!(split.shows_sidebar());
        assert!(
            gallery.width() < full_width - 200,
            "inspector covered tiles instead of resizing gallery: before={full_width} after={}",
            gallery.width()
        );
        let reduced_width = gallery.width();

        // Like the main sidebar, divider resizing changes the content width
        // without changing the selected destination or overlay allocation.
        split.set_sidebar_width_fraction(0.4);
        for _ in 0..8 {
            settle_gtk_layout();
        }
        assert!(
            gallery.width() < reduced_width - 30,
            "drag-width update failed to resize the content"
        );
        let resized_width = gallery.width();

        // Opening and closing the lightbox must not reallocate the grid.
        viewer.set_visible(true);
        viewer_inspector.set_visible(true);
        settle_gtk_layout();
        assert_eq!(gallery.width(), resized_width);
        assert_eq!(viewer.width(), overlay.width());
        viewer_inspector.set_visible(false);
        viewer.set_visible(false);
        settle_gtk_layout();
        assert_eq!(gallery.width(), resized_width);
        assert!(split.shows_sidebar(), "gallery inspector state was lost");

        split.set_show_sidebar(false);
        for _ in 0..10 {
            settle_gtk_layout();
        }
        assert!(
            (gallery.width() - full_width).abs() < 3,
            "closing inspector did not restore the full gallery width"
        );
        window.close();
    }

    fn settle_gtk_layout() {
        let main_loop = gtk::glib::MainLoop::new(None, false);
        let loop_to_quit = main_loop.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
            loop_to_quit.quit();
        });
        main_loop.run();
    }

    // Set GDK_BACKEND=x11 PIC_TEST_POINTER=1 to exercise real mouse input
    // with xdotool instead of emitting the buttons' clicked signals.
    fn pointer_click(window: &gtk::Window, widget: &gtk::Widget) {
        let point = widget
            .compute_point(
                window,
                &gtk::graphene::Point::new(
                    widget.width() as f32 / 2.0,
                    widget.height() as f32 / 2.0,
                ),
            )
            .unwrap();
        let output = std::process::Command::new("xdotool")
            .args(["search", "--name", "PIC album pointer test"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let id = String::from_utf8(output.stdout).unwrap();
        assert!(std::process::Command::new("xdotool")
            .args([
                "mousemove", "--window", id.trim(),
                &point.x().round().to_string(), &point.y().round().to_string(),
                "click", "1",
            ])
            .status()
            .unwrap()
            .success());
        settle_gtk_layout();
    }

    fn find_action(root: &gtk::Widget, text: &str) -> Option<gtk::Button> {
        if let Some(button) = root.downcast_ref::<gtk::Button>() {
            let label = button.label().map(|label| label.to_string()).or_else(|| {
                let child = button.child()?;
                // A plain label, or an icon row's label (In Albums).
                let label = child.clone().downcast::<gtk::Label>().ok().or_else(|| {
                    let mut item = child.first_child();
                    while let Some(widget) = item {
                        if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
                            return Some(label);
                        }
                        item = widget.next_sibling();
                    }
                    None
                })?;
                Some(label.text().to_string())
            });
            if label.as_deref() == Some(text) {
                return Some(button.clone());
            }
        }
        let mut child = root.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Some(found) = find_action(&widget, text) {
                return Some(found);
            }
            child = next;
        }
        None
    }

    fn menu() -> gtk::Widget {
        ACTIVE_PHOTO_MENU
            .with(|active| active.borrow().clone())
            .expect("photo context menu is showing")
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn copy_and_paste_edits_menu_persists_the_recipe_for_export() {
        use gtk::prelude::*;
        use crate::edit::export_batch::{export_job_to, render_job, ExportFormat, ExportJob};

        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-copy-paste-edits-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let destination_path = directory.join("destination.png");
        let original = image::RgbaImage::from_fn(32, 24, |x, y| {
            image::Rgba([(x * 7) as u8, (y * 9) as u8, ((x + y) * 3) as u8, 255])
        });
        image::DynamicImage::ImageRgba8(original.clone())
            .save(&destination_path)
            .unwrap();
        let connection = Rc::new(RefCell::new(db::open(&directory.join("library.db")).unwrap()));
        connection
            .borrow()
            .execute(
                "INSERT INTO photos (id, path, width, height) VALUES (1, 'source.NEF', 32, 24), (2, ?1, 32, 24)",
                rusqlite::params![destination_path.to_string_lossy()],
            )
            .unwrap();
        let mut recipe = crate::edit::EditRecipe::default();
        recipe.exposure = 0.31;
        recipe.contrast = 0.22;
        recipe.sharpen = 0.15;
        let encoded = recipe.encode();
        db::set_edit_recipe(&connection.borrow(), 1, &encoded).unwrap();
        let source = crate::photo_object::PhotoObject::from_photo(
            &db::photo(&connection.borrow(), 1).unwrap().unwrap(),
        );
        let destination = crate::photo_object::PhotoObject::from_photo(
            &db::photo(&connection.borrow(), 2).unwrap().unwrap(),
        );

        let overlay = gtk::Overlay::new();
        let window = adw::ApplicationWindow::builder()
            .title("PIC copy and paste edits test")
            .default_width(640)
            .default_height(480)
            .content(&overlay)
            .build();
        window.present();
        settle_gtk_layout();

        let context = PhotoActionContext {
            info: {
                // The InfoBar owns the current photo.
                let info = Rc::new(InfoBar::new());
                info.photo_updated(&destination);
                info
            },
            window: window.upcast_ref::<gtk::Window>().downgrade(),
            context_menu_host: Rc::new(RefCell::new(Some(overlay.clone().downgrade()))),
            ..PhotoActionContext::for_tests(&connection)
        };

        show_photo_context_menu(source, overlay.clone().upcast(), context.clone(), 10.0, 10.0);
        let copy = find_action(&menu(), "Copy Edits").unwrap();
        assert!(copy.is_sensitive());
        copy.emit_clicked();
        assert_eq!(context.edit_clipboard.borrow().as_deref(), Some(encoded.as_str()));

        show_photo_context_menu(
            destination.clone(), overlay.clone().upcast(), context.clone(), 10.0, 10.0,
        );
        let paste = find_action(&menu(), "Paste Edits").unwrap();
        assert!(paste.is_sensitive());
        paste.emit_clicked();
        settle_gtk_layout();

        let stored = db::photo(&connection.borrow(), 2).unwrap().unwrap().edit_recipe;
        assert_eq!(stored, encoded, "paste must persist the copied recipe");
        assert_eq!(destination.edit_recipe(), encoded,
            "the selected photo object must refresh so Export sees the pasted recipe");
        assert_eq!(context.info.current_photo().unwrap().edit_recipe(), encoded);

        let record = db::photo(&connection.borrow(), 2).unwrap().unwrap();
        let job = ExportJob::from_record(
            record.path,
            record.rotation,
            record.edit_recipe,
            32,
            24,
            "destination.png",
        );
        let expected = render_job(&job).unwrap();
        let export_path = directory.join("destination_edit.png");
        export_job_to(
            &job,
            &export_path,
            0,
            95,
            ExportFormat::Png,
            &render_job,
        )
        .unwrap();
        let exported = image::open(export_path).unwrap().to_rgba8();
        assert_eq!(exported, expected, "Export must use the pasted saved recipe");
        assert_ne!(exported, original, "the copied adjustments must affect Export");

        window.close();
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn add_to_album_popover_scrolls_when_the_album_list_is_long() {
        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-add-to-album-scroll-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Rc::new(RefCell::new(
            db::open(&directory.join("library.db")).unwrap(),
        ));
        for index in 1..=30 {
            db::create_album(&connection.borrow(), &format!("Album {index:02}")).unwrap();
        }
        let context = PhotoActionContext::for_tests(&connection);

        let popover =
            build_album_popover(context, Rc::new(Vec::new), Rc::new(|| {}), Rc::new(|| {}));
        let scroll = popover
            .child()
            .and_then(|child| child.downcast::<gtk::ScrolledWindow>().ok())
            .expect("a long album chooser must have a scrollable viewport");

        assert_eq!(scroll.vscrollbar_policy(), gtk::PolicyType::Automatic);
        assert!(scroll.propagates_natural_height());
        assert!(scroll.max_content_height() > 0);
        let last_album = find_action(scroll.upcast_ref(), "Album 30")
            .expect("the final album must remain available inside the viewport");
        assert!(last_album.is_sensitive());

        drop(popover);
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn album_cover_action_is_only_offered_inside_an_album() {
        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-photo-context-cover-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Rc::new(RefCell::new(
            db::open(&directory.join("library.db")).unwrap(),
        ));
        connection
            .borrow()
            .execute_batch("INSERT INTO photos (id, path) VALUES (1, 'screenshots/all photos.jpg');")
            .unwrap();
        let album = db::create_album(&connection.borrow(), "Holiday").unwrap();
        db::add_photos_to_album(&connection.borrow(), album.id, &[1]).unwrap();
        let photo = db::photo(&connection.borrow(), 1).unwrap().unwrap();
        let photo_object = crate::photo_object::PhotoObject::from_photo(&photo);

        let overlay = gtk::Overlay::new();
        let window = adw::ApplicationWindow::builder()
            .title("PIC album pointer test")
            .default_width(640)
            .default_height(480)
            .content(&overlay)
            .build();
        window.present();
        settle_gtk_layout();

        let context_for = |filter: sidebar::SidebarFilter| PhotoActionContext {
            filter: NavigationFilter::new(filter),
            albums: AlbumList::new(connection.clone(), NavigationFilter::new(filter)),
            window: window.upcast_ref::<gtk::Window>().downgrade(),
            context_menu_host: Rc::new(RefCell::new(Some(overlay.clone().downgrade()))),
            ..PhotoActionContext::for_tests(&connection)
        };

        // Inside the album the action targets the album being viewed, with no
        // album list and no "Add to Album"-style chooser.
        show_photo_context_menu(
            photo_object.clone(),
            overlay.clone().upcast(),
            context_for(sidebar::SidebarFilter::Album(album.id)),
            10.0,
            10.0,
        );
        let open_menu = menu();
        assert!(find_action(&open_menu, "Export…").is_some());
        assert!(find_action(&open_menu, "Set as Album Cover").is_some());
        assert!(find_action(&open_menu, "Remove from Album").is_some());

        find_action(&open_menu, "Set as Album Cover")
            .unwrap()
            .emit_clicked();
        assert_eq!(
            db::albums(&connection.borrow())
                .unwrap()
                .remove(0)
                .cover_photo_id,
            Some(1)
        );

        // The action greys out once the photo already is the album's cover.
        show_photo_context_menu(
            photo_object.clone(),
            overlay.clone().upcast(),
            context_for(sidebar::SidebarFilter::Album(album.id)),
            10.0,
            10.0,
        );
        assert!(!find_action(&menu(), "Set as Album Cover")
            .unwrap()
            .is_sensitive());
        dismiss_active_photo_context_menu();

        // Outside an album the cover action is not offered at all.
        show_photo_context_menu(
            photo_object,
            overlay.clone().upcast(),
            context_for(sidebar::SidebarFilter::All),
            10.0,
            10.0,
        );
        let open_menu = menu();
        assert!(find_action(&open_menu, "Set as Album Cover").is_none());
        assert!(find_action(&open_menu, "Remove from Album").is_none());

        // Exercise the trigger itself: it must close the context menu and
        // show the InfoBar's Add to Album list as a popover on the menu's
        // host (no separate dialog) before an album can be selected.
        db::remove_photos_from_album(&connection.borrow(), album.id, &[1]).unwrap();
        let trigger = find_action(&open_menu, "Add to Album").unwrap();
        settle_gtk_layout();
        if std::env::var_os("PIC_TEST_POINTER").is_some() {
            pointer_click(window.upcast_ref(), trigger.upcast_ref());
        } else {
            trigger.emit_clicked();
        }
        settle_gtk_layout();
        assert!(ACTIVE_PHOTO_MENU.with(|active| active.borrow().is_none()));
        assert!(window.visible_dialog().is_none(), "no separate album dialog");
        let mut child = overlay.first_child();
        let mut chooser = None;
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Ok(popover) = widget.downcast::<gtk::Popover>() {
                if popover.is_visible() {
                    chooser = Some(popover);
                }
            }
        }
        let chooser = chooser.expect("album list popover is shown");
        assert!(find_action(chooser.upcast_ref(), "New Album…").is_some());
        let album_button = find_action(chooser.upcast_ref(), "Holiday").unwrap();
        if std::env::var_os("PIC_TEST_POINTER").is_some() {
            pointer_click(window.upcast_ref(), album_button.upcast_ref());
        } else {
            album_button.emit_clicked();
        }
        let membership: i64 = connection
            .borrow()
            .query_row(
                "SELECT COUNT(*) FROM album_photos WHERE album_id = ?1 AND photo_id = 1",
                [album.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(membership, 1);
        assert!(ACTIVE_PHOTO_MENU.with(|active| active.borrow().is_none()));
        settle_gtk_layout();

        // The menu now links to the album the photo is in.
        show_photo_context_menu(
            crate::photo_object::PhotoObject::from_photo(
                &db::photo(&connection.borrow(), 1).unwrap().unwrap(),
            ),
            overlay.clone().upcast(),
            context_for(sidebar::SidebarFilter::All),
            10.0,
            10.0,
        );
        let link = find_action(&menu(), "Holiday").expect("In Albums lists Holiday");
        assert_eq!(link.tooltip_text().as_deref(), Some("Open this album"));
        dismiss_active_photo_context_menu();

        // The lightbox can show a photo that is not the gallery selection.
        // The clicked object must still change, and reopening the menu must
        // offer the inverse action after the first click closes it.
        let favorite_photo = crate::photo_object::PhotoObject::from_photo(
            &db::photo(&connection.borrow(), 1).unwrap().unwrap(),
        );
        show_photo_context_menu(
            favorite_photo.clone(),
            overlay.clone().upcast(),
            context_for(sidebar::SidebarFilter::All),
            10.0,
            10.0,
        );
        let favorite_button = find_action(&menu(), "Add to Favourites").unwrap();
        settle_gtk_layout();
        if std::env::var_os("PIC_TEST_POINTER").is_some() {
            pointer_click(window.upcast_ref(), favorite_button.upcast_ref());
        } else {
            favorite_button.emit_clicked();
        }
        assert!(ACTIVE_PHOTO_MENU.with(|active| active.borrow().is_none()));
        assert!(favorite_photo.favorite());
        assert!(db::photo(&connection.borrow(), 1).unwrap().unwrap().favorite);
        show_photo_context_menu(
            favorite_photo,
            overlay.clone().upcast(),
            context_for(sidebar::SidebarFilter::All),
            10.0,
            10.0,
        );
        let remove_favorite = find_action(&menu(), "Remove from Favourites").unwrap();
        settle_gtk_layout();
        if std::env::var_os("PIC_TEST_POINTER").is_some() {
            pointer_click(window.upcast_ref(), remove_favorite.upcast_ref());
        } else {
            remove_favorite.emit_clicked();
        }
        assert!(ACTIVE_PHOTO_MENU.with(|active| active.borrow().is_none()));
        assert!(!db::photo(&connection.borrow(), 1).unwrap().unwrap().favorite);

        window.close();
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// Trashing photos updates the sidebar Library counts, as marking does.
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn trashing_photos_updates_library_counts() {
        // A private Trash: for files on the home filesystem GIO trashes into
        // $XDG_DATA_HOME/Trash, so the user's own Trash is never touched.
        // (It refuses to trash on tmpfs, so the files live under target/.)
        let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
        let directory = target.join(format!(
            "pic-trash-counts-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let photos = directory.join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::env::set_var("XDG_DATA_HOME", directory.join("data"));
        gtk::init().unwrap();
        let connection = Rc::new(RefCell::new(db::open(&directory.join("library.db")).unwrap()));
        for (id, name, favorite) in [(1, "a.jpg", 1), (2, "b.jpg", 1), (3, "c.jpg", 0)] {
            let path = photos.join(name);
            std::fs::write(&path, [0xff, 0xd8, 0xff, 0xd9]).unwrap();
            connection
                .borrow()
                .execute(
                    "INSERT INTO photos (id, path, favorite) VALUES (?1, ?2, ?3)",
                    rusqlite::params![id, path.to_string_lossy(), favorite],
                )
                .unwrap();
        }
        let sidebar = sidebar::library_sidebar_for_test(
            db::sidebar_counts(&connection.borrow()).unwrap(),
        );
        assert_eq!(sidebar::library_count_for_test(&sidebar, sidebar::SidebarFilter::All), "3");
        let context = PhotoActionContext {
            sidebar: Rc::new(RefCell::new(Some(sidebar.clone()))),
            ..PhotoActionContext::for_tests(&connection)
        };

        trash_photos(&context, &[1, 3]).unwrap();
        assert!(!photos.join("a.jpg").exists() && !photos.join("c.jpg").exists());
        assert_eq!(sidebar::library_count_for_test(&sidebar, sidebar::SidebarFilter::All), "1");
        assert_eq!(
            sidebar::library_count_for_test(&sidebar, sidebar::SidebarFilter::Favorites),
            "1"
        );

        // A failure part-way still shows the photos already trashed.
        std::fs::remove_file(photos.join("b.jpg")).unwrap();
        connection
            .borrow()
            .execute(
                "INSERT INTO photos (id, path) VALUES (4, ?1)",
                rusqlite::params![photos.join("d.jpg").to_string_lossy()],
            )
            .unwrap();
        std::fs::write(photos.join("d.jpg"), [0xff, 0xd8, 0xff, 0xd9]).unwrap();
        sidebar::update_library_counts(&sidebar, db::sidebar_counts(&connection.borrow()).unwrap());
        assert_eq!(sidebar::library_count_for_test(&sidebar, sidebar::SidebarFilter::All), "2");
        assert!(trash_photos(&context, &[4, 2]).is_err());
        assert_eq!(sidebar::library_count_for_test(&sidebar, sidebar::SidebarFilter::All), "1");
        // Let the background grid and Smart Album refreshes finish first.
        drop(context);
        let main = glib::MainContext::default();
        let until = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < until {
            main.iteration(false);
        }
        let _ = std::fs::remove_dir_all(directory);
    }

    /// Rotating in the viewer refreshes the viewer's Properties panel and
    /// leaves the gallery panel on its own photo.
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn rotating_in_the_viewer_refreshes_the_viewer_properties() {
        gtk::init().unwrap();
        let connection = Rc::new(RefCell::new(Connection::open_in_memory().unwrap()));
        connection.borrow().execute_batch(db::SCHEMA).unwrap();
        connection
            .borrow()
            .execute_batch(
                "INSERT INTO folders(id,path,name) VALUES (1,'/photos','photos');
                 INSERT INTO photos(id,path,folder_id) VALUES
                    (1,'/photos/gallery.jpg',1),(2,'/photos/viewer.jpg',1);",
            )
            .unwrap();
        let object = |id| {
            crate::photo_object::PhotoObject::from_photo(
                &db::photo(&connection.borrow(), id).unwrap().unwrap(),
            )
        };
        let (gallery_photo, viewer_photo) = (object(1), object(2));

        let stack = gtk::Stack::new();
        stack.add_named(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some("photos"));
        stack.set_visible_child_name("photos");
        let lightbox = Rc::new(crate::lightbox::Lightbox::new());
        lightbox.root.set_visible(true);
        let front = FrontSurface::new(
            &stack,
            &lightbox,
            Rc::new(Cell::new(false)),
            Rc::new(RefCell::new(None)),
        );
        assert_eq!(front.current(), Front::Lightbox);
        let info = Rc::new(InfoBar::new());
        // The viewer's photo is the shown one (front inputs are FrontMirrors').
        info.photo_updated(&viewer_photo);
        let properties = PropertiesWidgets::new(info.clone());
        properties.show_photo(PropertiesSurface::Gallery, Some(&gallery_photo));

        let context = PhotoActionContext {
            info: info.clone(),
            lightbox: Rc::downgrade(&lightbox),
            front: Rc::downgrade(&front),
            properties: Rc::downgrade(&properties),
            ..PhotoActionContext::for_tests(&connection)
        };

        let (rotated, rotation) = rotate_current_photo(&context, false).unwrap();
        assert_eq!((rotated.id(), rotation), (2, 90));
        assert_eq!(db::photo(&connection.borrow(), 2).unwrap().unwrap().rotation, 90);
        assert_eq!(
            properties.photo_for_test(PropertiesSurface::Lightbox).map(|photo| photo.id()),
            Some(2),
            "the viewer panel shows the rotated photo"
        );
        assert_eq!(
            properties.photo_for_test(PropertiesSurface::Gallery).map(|photo| photo.id()),
            Some(1),
            "the gallery panel keeps its own photo"
        );
    }

    /// Properties' "Add to album" opens the one Add to Album list for its photo:
    /// manual albums as destinations, never a Smart Album.
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn properties_tags_and_albums_wrap_as_content_sized_items() {
        gtk::init().unwrap();
        let connection = Rc::new(RefCell::new(Connection::open_in_memory().unwrap()));
        connection.borrow().execute_batch(db::SCHEMA).unwrap();
        connection
            .borrow()
            .execute_batch(
                "INSERT INTO folders(id,path,name) VALUES (1,'/photos','photos');
                 INSERT INTO photos(id,path,folder_id) VALUES (1,'/photos/a.jpg',1);",
            )
            .unwrap();
        for tag in [
            "Food",
            "Sea",
            "A very long tag name that will not fit!!",
            "Kids",
        ] {
            db::scrapbook_add_tag(&connection.borrow(), 1, tag).unwrap();
        }
        for name in ["Trip", "Family"] {
            let album = db::create_album(&connection.borrow(), name).unwrap();
            db::add_photos_to_album(&connection.borrow(), album.id, &[1]).unwrap();
        }
        let photo = crate::photo_object::PhotoObject::from_photo(
            &db::photo(&connection.borrow(), 1).unwrap().unwrap(),
        );
        let context = PhotoActionContext::for_tests(&connection);
        let section = scrapbook_inspector_extra(&photo, &context, PropertiesSurface::Gallery);
        let window = gtk::Window::new();
        window.set_default_size(200, 600);
        window.set_child(Some(&section));
        window.present();
        settle_gtk_layout();

        let group_of = |name: &str| {
            let mut found = None;
            let mut stack = vec![section.clone().upcast::<gtk::Widget>()];
            while let Some(widget) = stack.pop() {
                if widget.has_css_class(name) {
                    found = Some(widget.clone());
                }
                let mut child = widget.first_child();
                while let Some(next) = child {
                    child = next.next_sibling();
                    stack.push(next);
                }
            }
            found.unwrap()
        };
        let tags = group_of("properties-tags");
        let albums = group_of("properties-albums");
        assert!(
            !tags.is_ancestor(&albums) && !albums.is_ancestor(&tags),
            "separate groups"
        );
        let items = |group: &gtk::Widget| {
            let mut items = Vec::new();
            let mut child = group.first_child();
            while let Some(item) = child {
                child = item.next_sibling();
                assert!(item.has_css_class("properties-item"));
                items.push(item.compute_bounds(group).unwrap());
            }
            items
        };
        let tag_items = items(&tags);
        assert_eq!(tag_items.len(), 4);
        // Content-sized: the short tags are narrower than the long one, and
        // every item stays inside the narrow panel (the long name ellipsizes).
        assert!(tag_items[0].width() < tag_items[2].width());
        assert!(tag_items
            .iter()
            .all(|at| at.x() >= 0.0 && at.x() + at.width() <= tags.width() as f32 + 0.5));
        // Food and Sea share the first line; the long tag wraps below them.
        assert_eq!(tag_items[0].y(), tag_items[1].y());
        assert!(tag_items[1].x() > tag_items[0].x() + tag_items[0].width());
        assert!(tag_items[2].y() > tag_items[0].y());
        assert_eq!(items(&albums).len(), 2);
        // Every tag and album has a keyboard-focusable remove button.
        for group in [&tags, &albums] {
            let mut child = group.first_child();
            while let Some(item) = child {
                child = item.next_sibling();
                let remove = item.last_child().and_downcast::<gtk::Button>().unwrap();
                assert!(remove.has_css_class("properties-item-remove") && remove.is_focusable());
            }
        }
        window.close();
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn properties_add_to_album_uses_the_shared_album_list() {
        gtk::init().unwrap();
        let connection = Rc::new(RefCell::new(Connection::open_in_memory().unwrap()));
        connection.borrow().execute_batch(db::SCHEMA).unwrap();
        connection
            .borrow()
            .execute_batch(
                "INSERT INTO folders(id,path,name) VALUES (1,'/photos','photos');
                 INSERT INTO photos(id,path,folder_id) VALUES (1,'/photos/a.jpg',1);",
            )
            .unwrap();
        let trip = db::create_album(&connection.borrow(), "Trip").unwrap();
        let mut loved = db::filters::AlbumRules::default();
        loved.categories.insert(
            db::filters::Field::Favorite,
            db::filters::Selection { values: vec!["1".into()], ..Default::default() },
        );
        db::create_smart_album(&connection.borrow(), "Loved", &loved).unwrap();
        let photo = crate::photo_object::PhotoObject::from_photo(
            &db::photo(&connection.borrow(), 1).unwrap().unwrap(),
        );
        let context = PhotoActionContext::for_tests(&connection);
        let section = scrapbook_inspector_extra(&photo, &context, PropertiesSurface::Gallery);
        let window = gtk::Window::new();
        window.set_child(Some(&section));
        window.present();
        settle_gtk_layout();

        let add = find_action(section.upcast_ref(), "Add to album")
            .expect("Add to album beside Add tag");
        assert!(find_action(section.upcast_ref(), "Add tag").is_some());
        add.emit_clicked();
        settle_gtk_layout();
        let mut child = add.first_child();
        let mut list = None;
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Ok(popover) = widget.downcast::<gtk::Popover>() {
                list = Some(popover);
            }
        }
        let list = list.expect("the Add to Album popover opens on the button");
        assert!(list.is_visible());
        assert!(find_action(list.upcast_ref(), "New Album…").is_some());
        assert!(find_action(list.upcast_ref(), "Loved").is_none(), "no Smart Album destination");
        find_action(list.upcast_ref(), "Trip").unwrap().emit_clicked();
        settle_gtk_layout();
        assert_eq!(
            db::albums_containing_photo(&connection.borrow(), 1)
                .unwrap()
                .into_iter()
                .map(|album| album.id)
                .collect::<Vec<_>>(),
            [trip.id]
        );
        // The photo's albums show as chips under its tags: Trip once added.
        assert!(
            find_action(section.upcast_ref(), "Trip").is_some(),
            "Properties lists the album the photo was just added to"
        );
        assert!(find_action(section.upcast_ref(), "Loved").is_none());
        // Its item ends in a remove button, which takes the photo out again.
        let item = find_action(section.upcast_ref(), "Trip")
            .unwrap()
            .parent()
            .unwrap();
        let rows = item.parent().and_downcast::<gtk::Box>().unwrap();
        assert!(rows.has_css_class("properties-albums"));
        let remove = item.last_child().and_downcast::<gtk::Button>().unwrap();
        assert!(remove.has_css_class("properties-item-remove"));
        assert!(remove.is_focusable(), "removing works from the keyboard");
        remove.emit_clicked();
        settle_gtk_layout();
        assert!(db::albums_containing_photo(&connection.borrow(), 1)
            .unwrap()
            .is_empty());
        assert!(
            find_action(section.upcast_ref(), "Trip").is_none(),
            "the item leaves with it"
        );
        assert!(!rows.is_visible(), "no album row once the photo is in none");
        window.close();
    }
}
