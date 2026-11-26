use gio::prelude::{AppInfoExt, SettingsExt};

thread_local! {
    static ACTIVE_PHOTO_MENU: RefCell<Option<gtk::Widget>> = RefCell::new(None);
    static INSPECTOR_CLOSE_CSS_INSTALLED: Cell<bool> = const { Cell::new(false) };
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
    touched_selected: bool,
    refresh_grid_when_done: bool,
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
    refresh_grid_when_done: bool,
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
        touched_selected: false,
        refresh_grid_when_done,
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
            if let Some(gallery) = work.context.gallery.borrow().upgrade() {
                gallery.update_edit_recipes_batch(&updates);
            }
            if let Some(lightbox) = work.context.lightbox.upgrade() {
                lightbox.update_edit_recipes_batch(&updates);
            }
            if let Some(current) = work.context.info.current_photo() {
                if let Some((_, recipe)) = updates.iter().find(|(id, _)| *id == current.id()) {
                    current.set_edit_recipe(recipe.clone());
                    work.touched_selected = true;
                }
            }
        }
    }

    work.index = end;
}

fn finish_bulk_recipe(work: BulkRecipeState, name: &'static str, total: usize) {
    if let Some(lightbox) = work.context.lightbox.upgrade() {
        lightbox.refresh_current_after_edit();
    }
    if work.touched_selected {
        if let Some(current) = work.context.info.current_photo() {
            work.context.info.photo_updated(&current);
        }
    }
    if work.refresh_grid_when_done || work.context.filter.get() == sidebar::SidebarFilter::History
        || !db::filters::Filters::load(&work.context.connection.borrow()).is_empty() {
        refresh_photo_actions_grid(&work.context);
        refresh_smart_album_counts(&work.context);
    } else {
        smart_albums_changed(&work.context, db::filters::Field::Edited);
    }
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
                false,
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
                false,
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
                true,
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
            refresh_album_ui(&set_cover_context);
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
            refresh_album_ui(&remove_context);
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
            let requests = if photo.is_outside_library() {
                vec![print_request_for_object(&photo)]
            } else {
                photo_print_requests(&context.connection.borrow(), &ids)
            };
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
        context.info.photo_updated(&photo);
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

/// Move the photos to the system Trash and mark them trashed, then show the
/// change: the grid, the Library and Smart Album counts. Stops at the first
/// failure and returns the heading and message to show; photos trashed
/// before it are still shown as gone.
fn trash_photos(
    context: &PhotoActionContext,
    selection: &[i64],
) -> Result<(), (&'static str, String)> {
    let mut result = Ok(());
    let mut trashed = 0;
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
        trashed += 1;
    }

    if result.is_ok() || trashed > 0 {
        if let Some(lightbox) = context.lightbox.upgrade() {
            lightbox.close();
        }
        context.info.clear_photo();
        refresh_photo_actions_grid(context);
        // Smart Album and Library counts, as photo marks refresh them.
        photo_mark_commands::update_counts(context);
    }
    result
}

/// Rotate the current photo a quarter turn and show it everywhere it is on
/// screen: the InfoBar, Properties, the gallery tile, the viewer and the
/// Smart Albums' Edited rule. Returns the photo and its new rotation, for
/// the editor; `None` when there is no current photo or saving failed.
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
    photo.set_rotation(rotation);
    // The inspector is a snapshot of the photo's metadata; refresh the
    // panel in front after the database and PhotoObject rotation are updated.
    photo_mark_commands::show_changed_photo(context, &photo);
    if let Some(gallery) = context.gallery.borrow().upgrade() {
        gallery.refresh_rotated_photo(&photo.path());
    }
    if let Some(lightbox) = context.lightbox.upgrade() {
        lightbox.refresh_current();
    }
    // Rotation counts as an edit for the Edited rule.
    smart_albums_changed(context, db::filters::Field::Edited);
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
    let sections = gtk::Box::new(gtk::Orientation::Vertical, 6);
    sections.add_css_class("scrapbook-inspector-body");
    sections.set_margin_start(12);
    sections.set_margin_end(12);
    sections.set_margin_top(8);
    sections.set_margin_bottom(8);
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

    // Inspector layout: photo preview above compact metadata, never side-by-side.
    // Use a fixed-height base child for the preview. GtkPicture's natural
    // aspect-ratio requisition must NOT determine the inspector row height:
    // portrait screenshots used to stretch it to hundreds of pixels.
    let preview_frame = gtk::Overlay::new();
    preview_frame.add_css_class("scrapbook-inspector-preview");
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
        missing.set_pixel_size(56);
        missing.add_css_class("dim-label");
        preview_frame.add_overlay(&missing);
    }
    let preview_overlay = gtk::Overlay::new();
    preview_overlay.set_child(Some(&preview_frame));
    let close = gtk::Button::new();
    let close_icon = gtk::Image::from_icon_name("window-close-symbolic");
    close_icon.set_pixel_size(12);
    close.set_child(Some(&close_icon));
    close.set_tooltip_text(Some("Close properties"));
    close.set_focus_on_click(false);
    // Avoid libadwaita's circular/osd padding, which makes the background
    // much bigger than the icon even with a small size request.
    close.add_css_class("flat");
    close.add_css_class("pic-inspector-close");
    close.set_size_request(20, 20);
    INSPECTOR_CLOSE_CSS_INSTALLED.with(|installed| {
      if !installed.replace(true) {
        let close_css = gtk::CssProvider::new();
        close_css.load_from_string(
        "button.pic-inspector-close { min-width: 20px; min-height: 20px;          padding: 0; margin: 0; border: 0; border-radius: 999px;          background-color: alpha(black, 0.55); color: white;          box-shadow: none; }          button.pic-inspector-close:hover { background-color: alpha(black, 0.75); }"
    );
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display, &close_css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION
            );
        }
      }
    });
    close.set_halign(gtk::Align::End);
    close.set_valign(gtk::Align::Start);
    close.set_margin_top(5);
    close.set_margin_end(5);
    close.connect_clicked(move |_| on_close());
    preview_overlay.add_overlay(&close);
    sections.append(&preview_overlay);

    let image_group = gtk::Box::new(gtk::Orientation::Vertical, 0);
    image_group.add_css_class("scrapbook-inspector-metadata");
    // No redundant heading above the file metadata rows.
    add_inspector_row(&image_group, "File name", &photo.filename());
    add_inspector_row(&image_group, "Date taken", &date);
    add_inspector_row(&image_group, "Dimensions", &dimensions);
    add_inspector_row(&image_group, "File size", &crate::infobar::format_size(size));
    add_inspector_row(&image_group, "File type", &file_type(&path));
    add_inspector_row(&image_group, "Rotation", &format!("{rotation}°"));
    add_inspector_row(&image_group, "Favourite", if favorite { "Yes" } else { "No" });
    let rating = record.as_ref().map(|item| item.rating).unwrap_or_else(|| photo.rating());
    let rating_text = if rating > 0 {
        format!("{} / 5", rating.clamp(1, 5))
    } else {
        "Unrated".to_string()
    };
    add_inspector_row(&image_group, "Rating", &rating_text);
    sections.append(&image_group);

    let camera_group = gtk::Box::new(gtk::Orientation::Vertical, 0);
    camera_group.add_css_class("scrapbook-inspector-metadata");
    let camera_heading = gtk::Label::new(Some("Camera"));
    camera_heading.set_xalign(0.0);
    camera_heading.add_css_class("properties-section-heading");
    camera_group.append(&camera_heading);
    let mut camera_rows = 0;
    let camera_name = record
        .as_ref()
        .and_then(|item| item.camera.clone())
        .or_else(|| photo.camera());
    let lens_name = record.as_ref().and_then(|item| item.lens.clone());
    // Blank EXIF strings are not meaningful camera metadata.
    let camera_name = camera_name.filter(|name| !name.trim().is_empty());
    let lens_name = lens_name.filter(|name| !name.trim().is_empty());
    if let Some(name) = camera_name.as_deref() {
        add_inspector_row(&camera_group, "Camera", name);
        camera_rows += 1;
    }
    if let Some(name) = lens_name.as_deref() {
        add_inspector_row(&camera_group, "Lens", name);
        camera_rows += 1;
    }
    let camera_values = [
        ("Aperture", record.as_ref().and_then(|item| item.aperture)
            .or_else(|| Some(photo.aperture())).and_then(format_aperture)),
        ("Shutter speed", record.as_ref().and_then(|item| item.shutter_speed)
            .and_then(format_shutter_speed)),
        ("ISO", record.as_ref().and_then(|item| item.iso)
            .filter(|value| *value > 0).map(|value| value.to_string())),
        ("Focal length", record.as_ref().and_then(|item| item.focal_length)
            .and_then(format_focal_length)),
        ("Exposure", record.as_ref().and_then(|item| item.exposure_bias)
            .and_then(format_exposure_bias)),
    ];
    // Exposure bias alone (often a default 0.0) is not proof that a
    // wallpaper or other image has actual camera EXIF data.
    let has_camera_data = camera_rows > 0
        || camera_values[..4].iter().any(|(_, value)| value.is_some());
    if has_camera_data {
        for (name, value) in &camera_values {
            if let Some(value) = value.as_deref() {
                add_inspector_row(&camera_group, name, value);
            }
        }
        sections.append(&camera_group);
    }

    // Compact, unboxed location footer. Display readable spaces but copy
    // the original URI/path exactly (including percent escapes).
    let location_footer = gtk::Box::new(gtk::Orientation::Vertical, 3);
    location_footer.add_css_class("scrapbook-inspector-location");
    location_footer.set_margin_top(4);
    let footer_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let folder_icon = gtk::Image::from_icon_name("folder-symbolic");
    folder_icon.add_css_class("dim-label");
    footer_header.append(&folder_icon);
    let location_title = gtk::Label::new(Some("File Location"));
    location_title.add_css_class("properties-section-heading");
    location_title.add_css_class("caption");
    location_title.set_xalign(0.0);
    location_title.set_hexpand(true);
    footer_header.append(&location_title);
    let copy_path = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy_path.add_css_class("flat");
    copy_path.set_focus_on_click(false);
    copy_path.set_tooltip_text(Some("Copy full file path"));
    let original_path = path.clone();
    copy_path.connect_clicked(move |_| {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&original_path);
        }
    });
    footer_header.append(&copy_path);
    location_footer.append(&footer_header);

    let readable_path = path.replace("%20", " ");
    let path_value = gtk::Label::new(Some(&readable_path));
    path_value.set_xalign(0.0);
    path_value.set_hexpand(true);
    path_value.set_wrap(true);
    path_value.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    path_value.set_width_chars(1);
    path_value.set_selectable(false);
    path_value.add_css_class("caption");
    path_value.add_css_class("dim-label");
    location_footer.append(&path_value);
    sections.append(&location_footer);
    sections.append(&scrapbook_inspector_extra(photo, context, surface));
    PropertiesContent { content, scroll, sections }
}

/// Global photo tags are always available from the Properties inspector,
/// including outside Scrapbook. Only the extra paper Notes/Actions layout
/// is skin-specific. The same SQLite tags power the global metadata filter.
fn populate_properties_tags(tag_rows: &gtk::FlowBox, context: &PhotoActionContext, photo_id: i64) {
    while let Some(child) = tag_rows.first_child() {
        tag_rows.remove(&child);
    }
    let (tags, _) = db::scrapbook_details(&context.connection.borrow(), photo_id)
        .unwrap_or_default();
    for tag in &tags {
        let chip = gtk::Button::with_label(&format!("{tag} ×"));
        chip.add_css_class("scrapbook-tag-chip");
        chip.set_tooltip_text(Some(&format!("Remove tag {tag}")));
        let tag = tag.clone();
        let context_for_refresh = context.clone();
        let rows_for_refresh = tag_rows.downgrade();
        chip.connect_clicked(move |_| {
            if let Err(error) = db::scrapbook_remove_tag(
                &context_for_refresh.connection.borrow(), photo_id, &tag
            ) {
                eprintln!("PIC Scrapbook: could not remove tag: {error:#}");
            } else {
                if let Some(rows) = rows_for_refresh.upgrade() {
                    populate_properties_tags(&rows, &context_for_refresh, photo_id);
                }
                smart_albums_changed(&context_for_refresh, db::filters::Field::Tags);
            }
        });
        tag_rows.insert(&chip, -1);
    }
}

fn properties_add_tag_button(
    tag_rows: &gtk::FlowBox,
    context: &PhotoActionContext,
    photo_id: i64,
) -> gtk::Button {
    let add_tag = gtk::Button::with_label("+ Add tag");
    add_tag.add_css_class("scrapbook-tag-add");
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
                        smart_albums_changed(&context_for_response, db::filters::Field::Tags);
                    }
                }
            });
            let window = context.window.upgrade();
            dialog.present(window.as_ref());
        });
    }
    add_tag
}

/// "+ Album" beside "+ Add tag": the one Add to Album list, for this photo.
fn properties_add_to_album_button(context: &PhotoActionContext, photo_id: i64) -> gtk::Button {
    let add_to_album = gtk::Button::with_label("+ Album");
    add_to_album.add_css_class("scrapbook-tag-add");
    add_to_album.set_tooltip_text(Some("Add to Album"));
    let context = context.clone();
    add_to_album.connect_clicked(move |button| {
        let ids: Rc<dyn Fn() -> Vec<i64>> = Rc::new(move || vec![photo_id]);
        show_album_popover(button.upcast_ref(), None, context.clone(), ids, Rc::new(|| {}));
    });
    add_to_album
}

fn scrapbook_inspector_extra(
    photo: &crate::photo_object::PhotoObject,
    context: &PhotoActionContext,
    _surface: PropertiesSurface,
) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 9);
    root.add_css_class("photo-tags-global");
    root.add_css_class("properties-section");
    root.add_css_class("scrapbook-inspector-extra");
    root.set_visible(true);
    let photo_id = photo.id();
    let (_, note) = db::scrapbook_details(&context.connection.borrow(), photo_id)
        .unwrap_or_default();

    let tags_header = gtk::Label::new(Some("Tags"));
    tags_header.add_css_class("scrapbook-inspector-paper-title");
    tags_header.set_halign(gtk::Align::Start);
    let tag_rows = gtk::FlowBox::new();
    tag_rows.add_css_class("scrapbook-tags");
    tag_rows.set_selection_mode(gtk::SelectionMode::None);
    tag_rows.set_min_children_per_line(1);
    tag_rows.set_max_children_per_line(4);
    tag_rows.set_column_spacing(5);
    tag_rows.set_row_spacing(5);
    populate_properties_tags(&tag_rows, context, photo_id);
    let header_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    tags_header.set_hexpand(true);
    header_row.append(&tags_header);
    header_row.append(&properties_add_tag_button(&tag_rows, context, photo_id));
    header_row.append(&properties_add_to_album_button(context, photo_id));
    root.append(&header_row);
    root.append(&tag_rows);

    let paper_details = gtk::Box::new(gtk::Orientation::Vertical, 9);
    let notes_header = gtk::Label::new(Some("Notes"));
    notes_header.add_css_class("scrapbook-inspector-paper-title");
    notes_header.set_halign(gtk::Align::Start);
    paper_details.add_css_class("properties-section");
    paper_details.append(&notes_header);
    let note_card = gtk::Box::new(gtk::Orientation::Vertical, 7);
    note_card.add_css_class("scrapbook-note-card");
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
    let note_save = gtk::Button::with_label("Save note");
    note_save.add_css_class("scrapbook-note-save");
    note_save.set_halign(gtk::Align::End);
    let status = gtk::Label::new(None);
    status.add_css_class("scrapbook-note-status");
    note_card.append(&status);
    note_card.append(&note_save);
    {
        let connection = context.connection.clone();
        note_save.connect_clicked(move |_| {
            let (start, end) = buffer.bounds();
            let note = buffer.text(&start, &end, true);
            match db::scrapbook_set_note(&connection.borrow(), photo_id, note.as_str()) {
                Ok(()) => status.set_label("Saved ✓"),
                Err(error) => status.set_label(&format!("Could not save: {error}")),
            }
        });
    }
    paper_details.append(&note_card);
    root.append(&paper_details);

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
    key.add_css_class("caption");
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
    detail.add_css_class("caption");
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
            connection: connection.clone(),
            gallery: Rc::new(RefCell::new(std::rc::Weak::new())),
            filter: Rc::new(Cell::new(sidebar::SidebarFilter::All)),
            search: Rc::new(RefCell::new(String::new())),
            sort: Rc::new(Cell::new(PhotoSort {
                field: SortField::DateTaken,
                direction: SortDirection::Descending,
            })),
            rating_filter: Rc::new(Cell::new(RatingFilter::All)),
            info: {
                // The InfoBar owns the current photo.
                let info = Rc::new(InfoBar::new());
                info.photo_updated(&destination);
                info
            },
            lightbox: std::rc::Weak::new(),
            front: std::rc::Weak::new(),
            sidebar: Rc::new(RefCell::new(None)),
            create_album: Rc::new(|| {}),
            import_folder: Rc::new(|| {}),
            delete_album: Rc::new(|_| {}),
            on_unavailable: Rc::new(|| {}),
            refresh_albums_home: Rc::new(|_| {}),
            navigate_to_folder: Rc::new(|_, _| {}),
            open_collage: Rc::new(|_| {}),
            open_edit: Rc::new(|_| {}),
            edit_clipboard: Rc::new(RefCell::new(None)),
            export: Rc::new(|_| {}),
            window: window.upcast_ref::<gtk::Window>().downgrade(),
            context_menu_host: Rc::new(RefCell::new(Some(overlay.clone().downgrade()))),
            operation_progress: OperationProgressUi::new(),
            properties: std::rc::Weak::new(),
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
        let context = PhotoActionContext {
            connection: connection.clone(),
            gallery: Rc::new(RefCell::new(std::rc::Weak::new())),
            filter: Rc::new(Cell::new(sidebar::SidebarFilter::All)),
            search: Rc::new(RefCell::new(String::new())),
            sort: Rc::new(Cell::new(PhotoSort {
                field: SortField::DateTaken,
                direction: SortDirection::Descending,
            })),
            rating_filter: Rc::new(Cell::new(RatingFilter::All)),
            info: Rc::new(InfoBar::new()),
            lightbox: std::rc::Weak::new(),
            front: std::rc::Weak::new(),
            sidebar: Rc::new(RefCell::new(None)),
            create_album: Rc::new(|| {}),
            import_folder: Rc::new(|| {}),
            delete_album: Rc::new(|_| {}),
            on_unavailable: Rc::new(|| {}),
            refresh_albums_home: Rc::new(|_| {}),
            navigate_to_folder: Rc::new(|_, _| {}),
            open_collage: Rc::new(|_| {}),
            open_edit: Rc::new(|_| {}),
            edit_clipboard: Rc::new(RefCell::new(None)),
            export: Rc::new(|_| {}),
            window: glib::WeakRef::new(),
            context_menu_host: Rc::new(RefCell::new(None)),
            operation_progress: OperationProgressUi::new(),
            properties: std::rc::Weak::new(),
        };

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
            connection: connection.clone(),
            gallery: Rc::new(RefCell::new(std::rc::Weak::new())),
            filter: Rc::new(Cell::new(filter)),
            search: Rc::new(RefCell::new(String::new())),
            sort: Rc::new(Cell::new(PhotoSort {
                field: SortField::DateTaken,
                direction: SortDirection::Descending,
            })),
            rating_filter: Rc::new(Cell::new(RatingFilter::All)),
            info: Rc::new(InfoBar::new()),
            lightbox: std::rc::Weak::new(),
            front: std::rc::Weak::new(),
            sidebar: Rc::new(RefCell::new(None)),
            create_album: Rc::new(|| {}),
            import_folder: Rc::new(|| {}),
            delete_album: Rc::new(|_| {}),
            on_unavailable: Rc::new(|| {}),
            refresh_albums_home: Rc::new(|_| {}),
            navigate_to_folder: Rc::new(|_, _| {}),
            open_collage: Rc::new(|_| {}),
            open_edit: Rc::new(|_| {}),
            edit_clipboard: Rc::new(RefCell::new(None)),
            export: Rc::new(|_| {}),
            window: window.upcast_ref::<gtk::Window>().downgrade(),
            context_menu_host: Rc::new(RefCell::new(Some(overlay.clone().downgrade()))),
            operation_progress: OperationProgressUi::new(),
            properties: std::rc::Weak::new(),
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
            connection: connection.clone(),
            gallery: Rc::new(RefCell::new(std::rc::Weak::new())),
            filter: Rc::new(Cell::new(sidebar::SidebarFilter::All)),
            search: Rc::new(RefCell::new(String::new())),
            sort: Rc::new(Cell::new(PhotoSort {
                field: SortField::DateTaken,
                direction: SortDirection::Descending,
            })),
            rating_filter: Rc::new(Cell::new(RatingFilter::All)),
            info: Rc::new(InfoBar::new()),
            lightbox: std::rc::Weak::new(),
            front: std::rc::Weak::new(),
            sidebar: Rc::new(RefCell::new(Some(sidebar.clone()))),
            create_album: Rc::new(|| {}),
            import_folder: Rc::new(|| {}),
            delete_album: Rc::new(|_| {}),
            on_unavailable: Rc::new(|| {}),
            refresh_albums_home: Rc::new(|_| {}),
            navigate_to_folder: Rc::new(|_, _| {}),
            open_collage: Rc::new(|_| {}),
            open_edit: Rc::new(|_| {}),
            edit_clipboard: Rc::new(RefCell::new(None)),
            export: Rc::new(|_| {}),
            window: glib::WeakRef::new(),
            context_menu_host: Rc::new(RefCell::new(None)),
            operation_progress: OperationProgressUi::new(),
            properties: std::rc::Weak::new(),
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
            connection: connection.clone(),
            gallery: Rc::new(RefCell::new(std::rc::Weak::new())),
            filter: Rc::new(Cell::new(sidebar::SidebarFilter::All)),
            search: Rc::new(RefCell::new(String::new())),
            sort: Rc::new(Cell::new(PhotoSort {
                field: SortField::DateTaken,
                direction: SortDirection::Descending,
            })),
            rating_filter: Rc::new(Cell::new(RatingFilter::All)),
            info: info.clone(),
            lightbox: Rc::downgrade(&lightbox),
            front: Rc::downgrade(&front),
            sidebar: Rc::new(RefCell::new(None)),
            create_album: Rc::new(|| {}),
            import_folder: Rc::new(|| {}),
            delete_album: Rc::new(|_| {}),
            on_unavailable: Rc::new(|| {}),
            refresh_albums_home: Rc::new(|_| {}),
            navigate_to_folder: Rc::new(|_, _| {}),
            open_collage: Rc::new(|_| {}),
            open_edit: Rc::new(|_| {}),
            edit_clipboard: Rc::new(RefCell::new(None)),
            export: Rc::new(|_| {}),
            window: glib::WeakRef::new(),
            context_menu_host: Rc::new(RefCell::new(None)),
            operation_progress: OperationProgressUi::new(),
            properties: Rc::downgrade(&properties),
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

    /// Properties' "+ Album" opens the one Add to Album list for its photo:
    /// manual albums as destinations, never a Smart Album.
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
        let context = PhotoActionContext {
            connection: connection.clone(),
            gallery: Rc::new(RefCell::new(std::rc::Weak::new())),
            filter: Rc::new(Cell::new(sidebar::SidebarFilter::All)),
            search: Rc::new(RefCell::new(String::new())),
            sort: Rc::new(Cell::new(PhotoSort {
                field: SortField::DateTaken,
                direction: SortDirection::Descending,
            })),
            rating_filter: Rc::new(Cell::new(RatingFilter::All)),
            info: Rc::new(InfoBar::new()),
            lightbox: std::rc::Weak::new(),
            front: std::rc::Weak::new(),
            sidebar: Rc::new(RefCell::new(None)),
            create_album: Rc::new(|| {}),
            import_folder: Rc::new(|| {}),
            delete_album: Rc::new(|_| {}),
            on_unavailable: Rc::new(|| {}),
            refresh_albums_home: Rc::new(|_| {}),
            navigate_to_folder: Rc::new(|_, _| {}),
            open_collage: Rc::new(|_| {}),
            open_edit: Rc::new(|_| {}),
            edit_clipboard: Rc::new(RefCell::new(None)),
            export: Rc::new(|_| {}),
            window: glib::WeakRef::new(),
            context_menu_host: Rc::new(RefCell::new(None)),
            operation_progress: OperationProgressUi::new(),
            properties: std::rc::Weak::new(),
        };
        let section = scrapbook_inspector_extra(&photo, &context, PropertiesSurface::Gallery);
        let window = gtk::Window::new();
        window.set_child(Some(&section));
        window.present();
        settle_gtk_layout();

        let add = find_action(section.upcast_ref(), "+ Album").expect("+ Album beside + Add tag");
        assert!(find_action(section.upcast_ref(), "+ Add tag").is_some());
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
        window.close();
    }
}
