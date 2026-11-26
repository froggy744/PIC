include!("dialogs.rs");
include!("navigation.rs");

pub fn build(app: &adw::Application, connection: Connection) -> adw::ApplicationWindow {
    let build_started = Instant::now();
    crate::startup_trace::mark("window_build_start", None);
    let window = adw::ApplicationWindow::new(app);
    {
        let first_tick = Rc::new(Cell::new(false));
        let first_tick_for_map = first_tick.clone();
        window.connect_map(move |window| {
            let mapped_at = Instant::now();
            crate::startup_trace::mark("window_map", crate::startup_trace::present_started());
            if let Some(surface) = window.surface() {
                let clock = surface.frame_clock();
                let updated = Rc::new(Cell::new(false));
                let updated_once = updated.clone();
                clock.connect_update(move |_| {
                    if !updated_once.replace(true) {
                        crate::startup_trace::mark("first_frame_update", Some(mapped_at));
                    }
                });
                let laid_out = Rc::new(Cell::new(false));
                let laid_out_once = laid_out.clone();
                clock.connect_layout(move |_| {
                    if !laid_out_once.replace(true) {
                        crate::startup_trace::mark("first_frame_layout", Some(mapped_at));
                    }
                });
                let painted_frame = Rc::new(Cell::new(false));
                let painted_frame_once = painted_frame.clone();
                clock.connect_paint(move |_| {
                    if !painted_frame_once.replace(true) {
                        crate::startup_trace::mark("first_frame_paint", Some(mapped_at));
                    }
                });
                let painted = Rc::new(Cell::new(false));
                let painted_once = painted.clone();
                let weak_window = window.downgrade();
                clock.connect_after_paint(move |_| {
                    if !painted_once.replace(true) {
                        crate::startup_trace::mark("first_after_paint", Some(mapped_at));
                        if std::env::var_os("PIC_STARTUP_TRACE").is_some() {
                            let renderer = weak_window
                                .upgrade()
                                .and_then(|window| window.renderer());
                            match renderer {
                                Some(renderer) => eprintln!(
                                    "PIC_STARTUP renderer={} realized={}",
                                    renderer.type_().name(),
                                    renderer.is_realized(),
                                ),
                                None => eprintln!("PIC_STARTUP renderer=unavailable realized=false"),
                            }
                        }
                    }
                });
            }
            if !first_tick_for_map.replace(true) {
                window.add_tick_callback(move |_, _| {
                    crate::startup_trace::mark("first_frame_tick", Some(mapped_at));
                    glib::ControlFlow::Break
                });
            }
        });
    }
    window.set_title(Some("PIC"));
    let connection = Rc::new(RefCell::new(connection));
    let session = SessionState::load(&*connection.borrow());
    window.set_default_size(session.window.width, session.window.height);
    if session.window.maximized {
        window.maximize();
    }

    let _ = db::delete_setting(&connection.borrow(), db::filters::SETTING);
    let startup_catalog_started = Instant::now();
    let folders = db::folders(&connection.borrow()).unwrap_or_default();
    let folder_cache = Rc::new(RefCell::new(folders.clone()));
    // The sidebar counts whole albums (never under a rating left saved by
    // the last session; it is reset below).
    let albums = AlbumList::sidebar_albums(&connection.borrow());
    let initial_filter = session.view(SessionEntry::Startup, &folders, &albums);
    let saved_view_photo_id = session.view_anchor;
    // "Activated" means the photo most recently opened in the viewer. Keep
    // that identity separate from ordinary selection and viewport scrolling.
    let last_activated_photo_id = Rc::new(Cell::new(session.activated_photo));
    let saved_view_scroll = session.gallery_scroll;
    let saved_albums_scroll = session.albums_scroll;
    let sidebar_counts = db::sidebar_counts(&connection.borrow()).unwrap_or_default();
    crate::startup_trace::mark("startup_catalog_metadata", Some(startup_catalog_started));
    let sort = Rc::new(Cell::new(PhotoSort {
        field: SortField::from_key(
            &db::setting(&connection.borrow(), SORT_FIELD_SETTING_KEY)
                .ok()
                .flatten()
                .unwrap_or_default(),
        ),
        direction: SortDirection::from_key(
            &db::setting(&connection.borrow(), SORT_DIRECTION_SETTING_KEY)
                .ok()
                .flatten()
                .unwrap_or_default(),
        ),
    }));
    // Rating filtering is session-only. Starting PIC with a stale saved
    // star filter is surprising and can make rating shortcuts appear to throw
    // the gallery around or hide the photo immediately.
    let rating_filter = Rc::new(Cell::new(RatingFilter::All));
    if let Err(error) = db::set_setting(
        &connection.borrow(),
        RATING_FILTER_SETTING_KEY,
        RatingFilter::All.key(),
    ) {
        eprintln!("Could not clear photo rating filter at startup: {error}");
    }
    let saved_group_mode = group_mode_from_key(
        &db::setting(&connection.borrow(), GROUP_MODE_SETTING_KEY)
            .ok()
            .flatten()
            .unwrap_or_default(),
    );
    let group_mode = Rc::new(Cell::new(if group_date_for_sort(sort.get()).is_some() {
        saved_group_mode
    } else {
        grid::GroupMode::None
    }));
    let startup_photos_started = Instant::now();
    // The same pipeline as a grid refresh (History in its own order).
    let startup_list = photo_list(
        &connection.borrow(),
        &PhotoListRequest::for_view(&connection.borrow(), initial_filter, "", sort.get()),
    )
    .unwrap_or_default();
    crate::startup_trace::mark("startup_photo_query", Some(startup_photos_started));
    let startup_pairing = (startup_list.pair_mode, startup_list.pair_folders);
    let startup_photos = Rc::new(startup_list.photos);
    crate::startup_trace::mark("database_photos_sorted", Some(build_started));
    if std::env::var_os("PIC_STARTUP_TRACE").is_some() {
        eprintln!(
            "PIC_STARTUP stage=library_counts elapsed_ms={} duration_ms=0 photos={} displayed={} folders={} albums={} scan=disabled rss_mb={}",
            crate::startup_trace::elapsed_ms(),
            sidebar_counts.photos,
            startup_photos.len(),
            folders.len(),
            albums.len(),
            crate::diagnostics::rss_mb()
        );
    }

    let info = Rc::new(InfoBar::new_with_settings(Some(connection.clone())));
    info.clear_photo();
    // The one owner of the Properties inspector's presentation.
    let properties = PropertiesWidgets::new(info.clone());

    let lightbox = Rc::new(Lightbox::new());
    // The front surface is read live from these three; see front_surface.rs.
    let main_stack = gtk::Stack::new();
    let date_browser_slot: Rc<RefCell<Option<date_browser::DateBrowser>>> = Rc::new(RefCell::new(None));
    let date_browser_active = Rc::new(Cell::new(false));
    let front = FrontSurface::new(
        &main_stack,
        &lightbox,
        date_browser_active.clone(),
        date_browser_slot.clone(),
    );
    let properties_for_lightbox = properties.clone();
    let info_for_lightbox = info.clone();
    let last_activated_for_lightbox = last_activated_photo_id.clone();
    let connection_for_lightbox = connection.clone();

    // Appearance themes are discovered from the themes folder at runtime; the
    // engine applies them, persists the choice, and is shared with the
    // Settings → Themes picker (the single theme list in the app).
    let display = gtk::gdk::Display::default().expect("a display is required");
    let theme_engine = crate::window::theme::ThemeEngine::new(
        display.clone(),
        connection.clone(),
        lightbox.clone(),
    );
    lightbox.set_photo_changed_handler(move |photo| {
        // A photo opened from outside the library is not a gallery position
        // to return to, so it never becomes the session's activated photo.
        if !photo.is_outside_library() {
            last_activated_for_lightbox.set(Some(photo.id()));
            if let Err(error) = record_session(
                &*connection_for_lightbox.borrow(),
                SessionChange::ActivatedPhoto(photo.id()),
            ) {
                eprintln!("Could not save last activated photo: {error}");
            }
        }
        info_for_lightbox.show_lightbox_photo(&photo);
        properties_for_lightbox.show_photo(PropertiesSurface::Lightbox, Some(&photo));
        // Navigating to another photo restores the viewer's normal fit state.
        info_for_lightbox.one_to_one.set_active(false);
    });

    // Space opens the fullscreen viewer from the gallery. While the viewer
    // is open, it toggles between fit and 1:1 viewing.
    // The actual open action is installed after Gallery exists.
    let space_open_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let edit_space_slot: Rc<RefCell<Option<Rc<dyn Fn() -> bool>>>> = Rc::new(RefCell::new(None));
    let edit_escape_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    // The sidebar click's destination bookkeeping, filled by layout.rs and
    // shared with collection navigation.
    let enter_destination_slot: Rc<RefCell<Option<Rc<dyn Fn(sidebar::SidebarFilter)>>>> =
        Rc::new(RefCell::new(None));
    let date_browser_exit_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let date_browser_discard_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let date_browser_back_slot: Rc<RefCell<Option<Rc<dyn Fn() -> bool>>>> = Rc::new(RefCell::new(None));
    let date_browser_resume_slot: Rc<RefCell<Option<Rc<dyn Fn(Option<(i32, u32)>, Option<i64>)>>>> = Rc::new(RefCell::new(None));
    let search_popup_slot: Rc<RefCell<Option<gtk::Popover>>> = Rc::new(RefCell::new(None));
    let space_toggle_in_progress = Rc::new(Cell::new(false));
    // Initialized before window shortcuts; wired after the layout is assembled.
    let immersive_active = Rc::new(Cell::new(false));
    // `immersive_sidebar_shown` is the single authority for the temporary
    // navigation overlay of an immersive session. The responsive breakpoint,
    // the compact rail and the persistent pin flag all defer to it while an
    // immersive session is active: the sidebar is either hidden or a
    // temporary overlay that hides again when the pointer leaves it.
    let immersive_sidebar_shown = Rc::new(Cell::new(false));
    let immersive_toggle_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> =
        Rc::new(RefCell::new(None));
    let immersive_reveal_search_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> =
        Rc::new(RefCell::new(None));
    let immersive_hide_search_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> =
        Rc::new(RefCell::new(None));

    include!("shortcuts.rs");

    let info_for_grid = info.clone();
    let lightbox_for_grid = lightbox.clone();
    let front_for_grid_selection = front.clone();
    let properties_for_grid_selection = properties.clone();
    let filter = Rc::new(Cell::new(initial_filter));
    let search_text = Rc::new(RefCell::new(String::new()));
    let search_entry_slot: Rc<RefCell<Option<gtk::SearchEntry>>> = Rc::new(RefCell::new(None));
    let search_suppressed = Rc::new(Cell::new(false));
    let cleared_search_query = Rc::new(RefCell::new(None::<String>));
    let search_debounce: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let search_result_folder_on_close = Rc::new(Cell::new(false));
    let search_close_waiting = Rc::new(Cell::new(false));
    let preserve_lightbox_during_search_return = Rc::new(Cell::new(false));
    let gallery_for_actions: Rc<RefCell<Weak<grid::Gallery>>> = Rc::new(RefCell::new(Weak::new()));
    let sidebar_for_unavailable: Rc<RefCell<Option<gtk::ScrolledWindow>>> =
        Rc::new(RefCell::new(None));
    let sidebar_selection_slot: Rc<RefCell<Option<gtk::ScrolledWindow>>> =
        Rc::new(RefCell::new(None));
    let create_album_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let create_album: Rc<dyn Fn()> = {
        let slot = create_album_slot.clone();
        Rc::new(move || {
            if let Some(callback) = slot.borrow().as_ref() {
                callback();
            }
        })
    };
    let import_folder_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let sd_import_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let edit_open_slot: Rc<RefCell<Option<Rc<dyn Fn(i64)>>>> = Rc::new(RefCell::new(None));
    let scrapbook_export_slot: Rc<RefCell<Option<Rc<dyn Fn(i64)>>>> =
        Rc::new(RefCell::new(None));
    let scrapbook_export_photo: Rc<dyn Fn(i64)> = {
        let slot = scrapbook_export_slot.clone();
        Rc::new(move |id| {
            if let Some(export) = slot.borrow().as_ref() {
                export(id);
            }
        })
    };
    let edit_clipboard: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let open_edit: Rc<dyn Fn(i64)> = {
        let slot = edit_open_slot.clone();
        Rc::new(move |id| {
            if let Some(callback) = slot.borrow().as_ref() {
                callback(id);
            }
        })
    };
    // Filled with the shared export (anchored on a photo) once it is
    // built; the InfoBar button and the context-menu Export item use it.
    type ExportAnchored = Rc<dyn Fn(&crate::photo_object::PhotoObject)>;
    let export_slot: Rc<RefCell<Option<ExportAnchored>>> = Rc::new(RefCell::new(None));
    let export_photos: ExportAnchored = {
        let slot = export_slot.clone();
        Rc::new(move |photo| {
            if let Some(callback) = slot.borrow().as_ref() {
                callback(photo);
            }
        })
    };
    let collage_open_slot: Rc<RefCell<Option<Rc<dyn Fn(Vec<i64>)>>>> =
        Rc::new(RefCell::new(None));
    let collage_add_mode_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> =
        Rc::new(RefCell::new(None));
    let collage_prepare_add_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> =
        Rc::new(RefCell::new(None));
    let collage_close_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> =
        Rc::new(RefCell::new(None));
    // True while the user is on the photos page picking images for the open
    // collage. The collage editor must survive that detour; every other
    // transition away from the collage page tears it down.
    let collage_add_mode: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    // True while the photo editor is editing a photo that belongs to the
    // open collage. The collage editor survives that detour, and the
    // editor's Back/Done return to the collage instead of the photos page.
    let collage_editing: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let open_collage: Rc<dyn Fn(Vec<i64>)> = {
        let slot = collage_open_slot.clone();
        Rc::new(move |ids| {
            if let Some(callback) = slot.borrow().as_ref() {
                callback(ids);
            }
        })
    };
    let refresh_folder_slot: Rc<
        RefCell<Option<Rc<dyn Fn(PhotoScanRequestReason, String)>>>,
    > =
        Rc::new(RefCell::new(None));

    // Keep filesystem monitors alive for exactly the folders the user marked
    // for watching and their catalogued subfolders (GIO directory monitors
    // are not recursive). Raw monitor callbacks only mark the owning imported
    // root dirty; they never start a scan directly. Rebuilding is
    // incremental: monitors that are still wanted keep running, so it is safe
    // to call after every scan, mount change or watch toggle.
    struct FolderWatchState {
        database: Option<std::path::PathBuf>,
        monitors: std::collections::HashMap<String, gio::FileMonitor>,
        watched: std::collections::HashSet<String>,
    }
    let watch_trace = std::env::var_os("PIC_WATCH_TRACE").is_some();
    let folder_watch_state = Rc::new(RefCell::new(FolderWatchState {
        database: None,
        monitors: std::collections::HashMap::new(),
        watched: std::collections::HashSet::new(),
    }));
    let pending_watch_refreshes: Rc<
        RefCell<std::collections::HashMap<String, Instant>>,
    > = Rc::new(RefCell::new(std::collections::HashMap::new()));
    let rebuild_folder_watches: Rc<dyn Fn()> = {
        let connection = connection.clone();
        let state = folder_watch_state.clone();
        let pending = pending_watch_refreshes.clone();
        Rc::new(move || {
            let database = db::active_database_path().ok();
            let enabled = db::folder_watching_enabled(&connection.borrow());
            let targets = if enabled {
                match db::folders(&connection.borrow()) {
                    Ok(folders) => watch_targets(&folders, |folder| {
                        #[cfg(target_os = "linux")]
                        if crate::network_shares::private(&folder.path) {
                            // GIO directory monitors may mount a share in the
                            // desktop. Direct network sources use explicit
                            // refresh, never GVfs.
                            return false;
                        }
                        // The cached availability can lag a mount that just
                        // appeared, and local directories are cheap to probe.
                        if folder.path.contains("://") {
                            folder.available
                        } else {
                            std::path::Path::new(&folder.path).is_dir()
                        }
                    }),
                    Err(error) => {
                        eprintln!("WATCH ERROR could not read folders: {error}");
                        return;
                    }
                }
            } else {
                WatchTargets::default()
            };

            let mut state = state.borrow_mut();
            if state.database != database {
                // Another library: nothing it watches has been observed yet.
                for (_, monitor) in state.monitors.drain() {
                    monitor.cancel();
                }
                state.watched.clear();
                state.database = database;
                pending.borrow_mut().clear();
            }
            let stale = state
                .monitors
                .keys()
                .filter(|path| !targets.monitored.contains_key(*path))
                .cloned()
                .collect::<Vec<_>>();
            for path in &stale {
                if let Some(monitor) = state.monitors.remove(path) {
                    monitor.cancel();
                }
            }
            let mut added = 0usize;
            for (watched_path, scan_root) in &targets.monitored {
                if state.monitors.contains_key(watched_path) {
                    continue;
                }
                let file = crate::source::file(watched_path);
                let monitor = match file.monitor_directory(
                    gio::FileMonitorFlags::NONE,
                    gio::Cancellable::NONE,
                ) {
                    Ok(monitor) => monitor,
                    Err(error) => {
                        eprintln!(
                            "WATCH ERROR monitor path={} error={}",
                            watched_path, error
                        );
                        continue;
                    }
                };
                let pending_for_event = pending.clone();
                let scan_root_for_event = scan_root.clone();
                monitor.connect_changed(move |_, file, _, event| {
                    // Ignore metadata-only monitor noise. Content changes are
                    // coalesced below before they can authorize any scan.
                    if matches!(
                        event,
                        gio::FileMonitorEvent::AttributeChanged
                            | gio::FileMonitorEvent::PreUnmount
                            | gio::FileMonitorEvent::Unmounted
                    ) {
                        return;
                    }
                    if watch_trace {
                        eprintln!(
                            "PIC_WATCH event={event:?} file={} root={}",
                            crate::source::reference(file),
                            scan_root_for_event
                        );
                    }
                    pending_for_event
                        .borrow_mut()
                        .insert(scan_root_for_event.clone(), Instant::now());
                });
                state.monitors.insert(watched_path.clone(), monitor);
                added += 1;
            }

            // Changes made while a watched folder was not observed produce no
            // monitor event, so every newly observed watched folder gets one
            // catch-up refresh through the same debounced queue.
            let mut catch_up = Vec::new();
            {
                let mut pending = pending.borrow_mut();
                pending.retain(|root, _| targets.watched.values().any(|wanted| wanted == root));
                for (path, scan_root) in &targets.watched {
                    if !state.watched.contains(path) && !catch_up.contains(scan_root) {
                        pending.entry(scan_root.clone()).or_insert_with(Instant::now);
                        catch_up.push(scan_root.clone());
                    }
                }
            }
            state.watched = targets.watched.keys().cloned().collect();
            if watch_trace {
                eprintln!(
                    "PIC_WATCH rebuild enabled={enabled} watched={} monitors={} added={added} removed={} catch_up={:?}",
                    state.watched.len(),
                    state.monitors.len(),
                    stale.len(),
                    catch_up
                );
            }
        })
    };
    let import_folder: Rc<dyn Fn()> = {
        let slot = import_folder_slot.clone();
        Rc::new(move || {
            if let Some(callback) = slot.borrow().as_ref() {
                callback();
            }
        })
    };
    let add_network_share_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let add_network_share: Rc<dyn Fn()> = {
        let slot = add_network_share_slot.clone();
        Rc::new(move || {
            if let Some(callback) = slot.borrow().as_ref() {
                callback();
            }
        })
    };
    let delete_album_slot: Rc<RefCell<Option<Rc<dyn Fn(i64)>>>> = Rc::new(RefCell::new(None));
    let delete_album: Rc<dyn Fn(i64)> = {
        let slot = delete_album_slot.clone();
        Rc::new(move |album_id| {
            if let Some(callback) = slot.borrow().as_ref() {
                callback(album_id);
            }
        })
    };
    let availability_refresh_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let availability_refresh: Rc<dyn Fn()> = {
        let connection = connection.clone();
        let folder_cache = folder_cache.clone();
        let gallery = gallery_for_actions.clone();
        let sidebar = sidebar_for_unavailable.clone();
        let slot = availability_refresh_slot.clone();
        let create_album = create_album.clone();
        let import_folder = import_folder.clone();
        let delete_album = delete_album.clone();
        let filter = filter.clone();
        let search = search_text.clone();
        let sort = sort.clone();
        let rating_filter = rating_filter.clone();
        Rc::new(move || {
            let connection_for_filter = connection.clone();
            let gallery_for_filter = gallery.clone();
            let filter = filter.clone();
            let search = search.clone();
            let sort = sort.clone();
            let rating_filter = rating_filter.clone();
            let refresh_filtered_gallery: Rc<dyn Fn()> = Rc::new(move || {
                if matches!(rating_filter.get(), RatingFilter::Offline | RatingFilter::Online) {
                    if let Some(gallery) = gallery_for_filter.borrow().upgrade() {
                        refresh_grid(&connection_for_filter, filter.get(), &search.borrow(), sort.get(), &gallery);
                    }
                }
            });
            debug_assert!(PhotoScanRequestReason::AvailabilityUpdate
                .scan_kind()
                .is_none());
            refresh_availability_ui(
                &connection,
                &folder_cache,
                &gallery,
                &sidebar,
                &slot,
                create_album.clone(),
                import_folder.clone(),
                delete_album.clone(),
                refresh_filtered_gallery,
            );
        })
    };
    availability_refresh_slot.replace(Some(availability_refresh.clone()));
    // The one owner of the album surfaces; the sidebar and the Albums home
    // are attached once they are built.
    let album_list = AlbumList::new(connection.clone(), filter.clone());
    album_list.attach_sidebar(sidebar_for_unavailable.clone(), delete_album.clone());
    let album_home_click_slot: Rc<RefCell<Option<Rc<dyn Fn(i64)>>>> =
        Rc::new(RefCell::new(None));
    let library_navigation_slot: Rc<RefCell<Option<Rc<dyn Fn(sidebar::SidebarFilter)>>>> =
        Rc::new(RefCell::new(None));
    // Set by the toolbar: shows whether its rating filter applies to the
    // current destination. Called wherever the destination changes.
    let rating_filter_scope_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> =
        Rc::new(RefCell::new(None));
    let folder_navigation_slot: Rc<RefCell<Option<Rc<dyn Fn(i64, i64)>>>> =
        Rc::new(RefCell::new(None));
    // Open in Folder keeps this exact target alive briefly so a sidebar
    // Tree-mode reorder cannot replace it with a generic folder-header scroll.
    let open_in_folder_exact_target: Rc<Cell<Option<i64>>> = Rc::new(Cell::new(None));
    // Exact-photo navigation has its own readiness requirement: the new
    // folder model alone is NOT ready until the chosen image is visible.
    let open_in_folder_revealed: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let navigate_to_folder: Rc<dyn Fn(i64, i64)> = {
        let slot = folder_navigation_slot.clone();
        Rc::new(move |folder_id, photo_id| {
            if let Some(callback) = slot.borrow().as_ref() {
                callback(folder_id, photo_id);
            }
        })
    };
    let context_menu_host: Rc<RefCell<Option<glib::WeakRef<gtk::Overlay>>>> =
        Rc::new(RefCell::new(None));
    let operation_progress = OperationProgressUi::new();
    let action_context = PhotoActionContext {
        connection: connection.clone(),
        gallery: gallery_for_actions.clone(),
        filter: filter.clone(),
        search: search_text.clone(),
        sort: sort.clone(),
        rating_filter: rating_filter.clone(),
        info: info.clone(),
        lightbox: Rc::downgrade(&lightbox),
        front: Rc::downgrade(&front),
        sidebar: sidebar_for_unavailable.clone(),
        create_album: create_album.clone(),
        import_folder: import_folder.clone(),
        delete_album: delete_album.clone(),
        on_unavailable: availability_refresh.clone(),
        navigate_to_folder: navigate_to_folder.clone(),
        open_collage: open_collage.clone(),
        open_edit: open_edit.clone(),
        edit_clipboard: edit_clipboard.clone(),
        export: export_photos.clone(),
        albums: album_list.clone(),
        window: window.clone().upcast::<gtk::Window>().downgrade(),
        context_menu_host: context_menu_host.clone(),
        operation_progress: operation_progress.clone(),
        properties: Rc::downgrade(&properties),
    };
    properties.set_context(action_context.clone());
    let saved_grid_thumbnail_size = grid_thumbnail_size_from_setting(&connection.borrow());
    let grid_zoom_syncing = Rc::new(Cell::new(false));
    // The one writer of what the shared zoom slider shows (zoom_slider.rs).
    let zoom_slider = ZoomSliderWidgets::new(
        &info.grid_zoom,
        grid_zoom_syncing.clone(),
        saved_grid_thumbnail_size.unwrap_or(DEFAULT_GRID_THUMBNAIL_SIZE),
    );

    // Clear the entry without rebuilding the previous sidebar view behind
    // the viewer. Its close preparation will load the result's folder there.
    let clear_search_after_result: Rc<dyn Fn()> = {
        let search_entry = search_entry_slot.clone();
        let search_text = search_text.clone();
        let suppressed = search_suppressed.clone();
        let debounce = search_debounce.clone();
        let gallery = gallery_for_actions.clone();
        let folder_on_close = search_result_folder_on_close.clone();
        let cleared_query = cleared_search_query.clone();
        let hide_search = immersive_hide_search_slot.clone();
        Rc::new(move || {
            if let Some(source) = debounce.borrow_mut().take() {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| source.remove()));
            }
            if let Some(gallery) = gallery.borrow().upgrade() {
                gallery.set_search_result_activation(false);
            }
            if let Some(entry) = search_entry.borrow().as_ref() {
                if !entry.text().is_empty() {
                    if let Some(hide) = hide_search.borrow().as_ref() {
                        hide();
                    }
                    folder_on_close.set(true);
                    cleared_query.replace(None);
                    suppressed.set(true);
                    entry.set_text("");
                    suppressed.set(false);
                }
            }
            search_text.replace(String::new());
        })
    };

    let gallery_started = Instant::now();
    let gallery = Rc::new(grid::Gallery::new(
        &[],
        saved_grid_thumbnail_size.unwrap_or(DEFAULT_GRID_THUMBNAIL_SIZE),
        move |photo| {
            // A hidden gallery refresh must not replace the photo currently
            // displayed by the lightbox or its info bar.
            if front_for_grid_selection.gallery_covered() {
                return;
            }
            info_for_grid.show_gallery_focus(photo.as_ref());
            properties_for_grid_selection.show_photo(PropertiesSurface::Gallery, photo.as_ref());
        },
        {
            let availability_refresh = availability_refresh.clone();
            let clear_search_after_result = clear_search_after_result.clone();
            let filter = filter.clone();
            let search_text = search_text.clone();
            let connection = connection.clone();
            let open_edit = open_edit.clone();
            let open_collage = open_collage.clone();
            move |photos, selected_index, source| {
                if filter.get() == sidebar::SidebarFilter::History
                    && search_text.borrow().is_empty()
                {
                    if let Some(photo) = photos.get(selected_index) {
                        let is_collage = db::collage_project(&connection.borrow(), photo.id())
                            .ok()
                            .flatten()
                            .is_some();
                        if is_collage {
                            open_collage(vec![photo.id()]);
                        } else {
                            open_edit(photo.id());
                        }
                    }
                    return;
                }
                if photos
                    .get(selected_index)
                    .is_some_and(|photo| !photo.original_available())
                {
                    availability_refresh();
                }
                lightbox_for_grid.open_from_source(photos, selected_index, source);
                clear_search_after_result();
            }
        },
        {
            let action_context = action_context.clone();
            move |photo, anchor, x, y| {
                show_photo_context_menu(photo, anchor, action_context.clone(), x, y);
            }
        },
        {
            let connection = connection.clone();
            let availability_refresh = availability_refresh.clone();
            move |photo, anchor| {
                show_unavailable_dialog(
                    &anchor,
                    photo,
                    connection.clone(),
                    availability_refresh.clone(),
                );
            }
        },
        {
            let connection = connection.clone();
            let zoom_slider = zoom_slider.clone();
            move |width| {
                if let Err(error) = db::set_setting(
                    &connection.borrow(),
                    GRID_THUMBNAIL_SIZE_SETTING_KEY,
                    &width.to_string(),
                ) {
                    eprintln!("Could not save grid thumbnail size: {error}");
                }

                // A gallery zoom may finish just after the viewer or the
                // editor opens; the slider then shows their zoom, not this.
                zoom_slider.show_thumbnail_width(width);
            }
        },
    ));
    // The startup list's pairing state, before the first photos are shown.
    gallery.set_raw_jpeg_mode(startup_pairing.0);
    gallery.set_raw_jpeg_pair_folders(startup_pairing.1);
    crate::startup_trace::mark("gallery_widgets_done", Some(gallery_started));
    // Multi-photo selection replaces metadata in the existing InfoBar instead
    // of drawing a second, floating count on top of the gallery. This observes
    // the same model for Grid, Folder and Photo Wall.
    const SELECTION_INFOBAR_PREVIEW_LIMIT: usize = 48;
    gallery.connect_selection_preview_changed(SELECTION_INFOBAR_PREVIEW_LIMIT, {
        let info = Rc::downgrade(&info);
        move |count, photos| {
            if let Some(info) = info.upgrade() {
                info.show_gallery_selection(count, &photos);
            }
        }
    });
    info.connect_selected_thumbnail_unselect({
        let gallery = Rc::downgrade(&gallery);
        move |photo_id| {
            if let Some(gallery) = gallery.upgrade() {
                gallery.unselect_photo_by_id(photo_id);
            }
        }
    });
    {
        let gallery = Rc::downgrade(&gallery);
        info.selection_clear.connect_clicked(move |_| {
            if let Some(gallery) = gallery.upgrade() {
                gallery.clear_keyboard_selection();
            }
        });
    }
    // No stored size (or only the untouched legacy default): adopt the
    // ~4-thumbnails-per-row view level on the first real layout.
    if saved_grid_thumbnail_size.is_none() {
        gallery.enable_auto_default_zoom();
    }

    {
        let connection = connection.clone();
        let filter = filter.clone();
        let search_text = search_text.clone();
        let sort = sort.clone();
        let gallery_for_pairs = gallery.clone();
        gallery.set_raw_jpeg_mode_changed_handler(move |mode| {
            if let Err(error) =
                crate::image_format::set_raw_jpeg_pair_mode(&connection.borrow(), mode)
            {
                eprintln!("Could not save RAW + JPEG pair view: {error}");
                return;
            }
            let search = search_text.borrow().clone();
            refresh_grid_preserving_folder_viewport(
                &connection,
                filter.get(),
                &search,
                sort.get(),
                &gallery_for_pairs,
            );
        });
    }

    // Resolved size for views that only need a number (album covers).
    let grid_thumbnail_size = saved_grid_thumbnail_size.unwrap_or(DEFAULT_GRID_THUMBNAIL_SIZE);

    // Corner style belongs to the whole application, not just thumbnails.
    // Balanced is the default; prior square-thumbnail settings migrate.
    crate::settings::CornerStyle::saved(&connection.borrow()).apply(&window);
    gallery.set_fit_whole_photo(
        crate::settings::saved_bool(
            &connection.borrow(),
            crate::db::THUMBNAIL_FIT_WHOLE_PHOTO_SETTING_KEY,
        )
        .unwrap_or(false),
    );
    gallery.set_show_file_names(
        crate::settings::saved_bool(
            &connection.borrow(),
            crate::db::THUMBNAIL_FILE_NAMES_SETTING_KEY,
        )
        .unwrap_or(false),
    );
    crate::thumbnail::set_high_quality_thumbnails_enabled(
        crate::db::high_quality_thumbnails_enabled(&connection.borrow()),
    );
    gallery.set_zoom_animations_enabled(db::zoom_animations_enabled(&connection.borrow()));
    gallery_for_actions.replace(Rc::downgrade(&gallery));
    apply_gallery_grouping(&gallery, filter.get(), sort.get(), group_mode.get(), true);
    crate::startup_trace::mark("gallery_configured", Some(build_started));

    // Reuse the gallery model for the Space shortcut. Re-querying the database
    // and rebuilding the complete library model here blocked the GTK thread.
    {
        let gallery = gallery.clone();
        let info = Rc::downgrade(&info);
        let lightbox = lightbox.clone();
        let availability_refresh = availability_refresh.clone();
        let filter = filter.clone();
        let search_text = search_text.clone();

        space_open_slot.replace(Some(Rc::new(move || {
            if filter.get() == sidebar::SidebarFilter::Library && search_text.borrow().is_empty() {
                return;
            }
            let photos = gallery.photo_objects();
            let selected_id = info
                .upgrade()
                .and_then(|info| info.current_photo())
                .map(|photo| photo.id())
                .filter(|id| photos.iter().any(|photo| photo.id() == *id))
                .or_else(|| gallery.selected_photo_ids(None).into_iter().next());
            let Some(selected_id) = selected_id else {
                return;
            };
            let Some(index) = photos.iter().position(|photo| photo.id() == selected_id) else {
                return;
            };
            if !photos[index].original_available() {
                availability_refresh();
            }
            let source = gallery.transition_source_for_photo(selected_id);
            lightbox.open_from_source(photos, index, source);
        })));
    }

    let create_context = action_context.clone();
    let create_parent = window.clone();
    create_album_slot.replace(Some(Rc::new(move || {
        show_create_album_dialog(
            create_parent.clone().upcast(),
            Vec::new(),
            create_context.clone(),
        );
    })));
    let rules_context = action_context.clone();
    let rules_parent = window.downgrade();
    let edit_rules = gio::SimpleAction::new(
        sidebar::EDIT_ALBUM_RULES_ACTION.trim_start_matches("win."),
        Some(glib::VariantTy::INT64),
    );
    edit_rules.connect_activate(move |_, target| {
        let (Some(album_id), Some(parent)) =
            (target.and_then(|value| value.get::<i64>()), rules_parent.upgrade())
        else {
            return;
        };
        show_edit_album_rules(parent.upcast_ref(), album_id, rules_context.clone());
    });
    window.add_action(&edit_rules);
    let save_context = action_context.clone();
    let save_parent = window.downgrade();
    let save_smart_album = gio::SimpleAction::new(
        metadata_panel::SAVE_FILTERS_AS_SMART_ALBUM_ACTION.trim_start_matches("win."),
        None,
    );
    save_smart_album.connect_activate(move |_, _| {
        if let Some(parent) = save_parent.upgrade() {
            show_save_filters_as_smart_album(parent.upcast_ref(), save_context.clone());
        }
    });
    window.add_action(&save_smart_album);
    let recount_context = action_context.clone();
    let recount = gio::SimpleAction::new(
        REFRESH_SMART_ALBUM_COUNTS_ACTION.trim_start_matches("win."),
        None,
    );
    recount.connect_activate(move |_, _| refresh_smart_album_counts(&recount_context));
    window.add_action(&recount);
    // Open an album as its sidebar row does (the photo menu's In Albums).
    let open_album_slot = album_home_click_slot.clone();
    let open_album_context = action_context.clone();
    let open_album = gio::SimpleAction::new(
        OPEN_ALBUM_ACTION.trim_start_matches("win."),
        Some(glib::VariantTy::INT64),
    );
    open_album.connect_activate(move |_, target| {
        let Some(album_id) = target.and_then(|value| value.get::<i64>()) else {
            return;
        };
        let viewer_in_front = open_album_context
            .front
            .upgrade()
            .is_some_and(|front| front.current() == Front::Lightbox);
        if viewer_in_front {
            if let Some(lightbox) = open_album_context.lightbox.upgrade() {
                lightbox.close();
            }
        }
        if let Some(open) = open_album_slot.borrow().as_ref() {
            open(album_id);
        }
    });
    window.add_action(&open_album);
    let delete_context = action_context.clone();
    let delete_parent = window.clone();
    delete_album_slot.replace(Some(Rc::new(move |album_id| {
        show_delete_album_confirmation(
            delete_parent.clone().upcast(),
            album_id,
            delete_context.clone(),
        );
    })));

    {
        let connection = connection.clone();
        let availability_refresh = availability_refresh.clone();
        let lightbox_for_retry = Rc::downgrade(&lightbox);
        lightbox.set_unavailable_handler(move |photo, anchor| {
            let availability_refresh = availability_refresh.clone();
            let lightbox_for_retry = lightbox_for_retry.clone();
            let retry: Rc<dyn Fn()> = Rc::new(move || {
                availability_refresh();
                if let Some(lightbox) = lightbox_for_retry.upgrade() {
                    lightbox.retry_current_original();
                }
            });
            show_unavailable_dialog(&anchor, photo, connection.clone(), retry);
        });
    }
    configure_infobar_album_menu(&info.add_to_album, action_context.clone());
    // Right-click on InfoBar metadata opens Properties; right-click again
    // closes it, without opening the photo context menu.
    {
        let current = Rc::downgrade(&info);
        let properties = properties.clone();
        info.connect_metadata_right_click(move || {
            if let Some(photo) = current.upgrade().and_then(|info| info.current_photo()) {
                properties.toggle(&photo);
            }
        });
    }

    // Destructive maintenance actions for the Settings → Library page. The
    // settings window owns the buttons and confirmation dialogs; the actual
    // behaviour stays here where the gallery and refresh context live.
    let maintenance_cancel_scans: Rc<RefCell<Option<Rc<dyn Fn()>>>> =
        Rc::new(RefCell::new(None));
    let settings_maintenance = crate::settings::LibraryMaintenance {
        clear_database: {
            let connection = connection.clone();
            let gallery = gallery.clone();
            let filter = filter.clone();
            let search = search_text.clone();
            let sort = sort.clone();
            let availability_refresh = availability_refresh.clone();
            let cancel_scans = maintenance_cancel_scans.clone();
            Rc::new(move || {
                let traced = std::env::var_os("PIC_LIBRARY_MAINTENANCE_TRACE").is_some();
                if let Some(cancel) = cancel_scans.borrow().as_ref() {
                    cancel();
                }
                if traced {
                    eprintln!("PIC_LIBRARY_MAINTENANCE action=clear_database phase=delete_start");
                }
                let result = db::clear_photos(&connection.borrow())
                    .map_err(|error| format!("{error:#}"));
                if traced {
                    match &result {
                        Ok(()) => eprintln!("PIC_LIBRARY_MAINTENANCE action=clear_database phase=delete_complete"),
                        Err(error) => eprintln!("PIC_LIBRARY_MAINTENANCE action=clear_database phase=delete_failed error={error}"),
                    }
                }
                result?;
                refresh_grid(
                    &connection,
                    filter.get(),
                    &search.borrow(),
                    sort.get(),
                    &gallery,
                );
                availability_refresh();
                Ok(())
            })
        },
        clear_all: {
            let connection = connection.clone();
            let gallery = gallery.clone();
            let filter = filter.clone();
            let search = search_text.clone();
            let sort = sort.clone();
            let availability_refresh = availability_refresh.clone();
            let rebuild_folder_watches = rebuild_folder_watches.clone();
            let cancel_scans = maintenance_cancel_scans.clone();
            Rc::new(move || {
                let traced = std::env::var_os("PIC_LIBRARY_MAINTENANCE_TRACE").is_some();
                if let Some(cancel) = cancel_scans.borrow().as_ref() {
                    cancel();
                }
                if traced {
                    eprintln!("PIC_LIBRARY_MAINTENANCE action=clear_all phase=database_delete_start");
                }
                db::clear_all(&connection.borrow()).map_err(|error| format!("{error:#}"))?;
                if traced {
                    eprintln!("PIC_LIBRARY_MAINTENANCE action=clear_all phase=database_delete_complete");
                }
                refresh_grid(
                    &connection,
                    filter.get(),
                    &search.borrow(),
                    sort.get(),
                    &gallery,
                );
                availability_refresh();
                rebuild_folder_watches();
                Ok(())
            })
        },
    };

    let clear_saved_views: Rc<dyn Fn()> = {
        let connection = connection.clone();
        let gallery = gallery.clone();
        let weak_info = Rc::downgrade(&info);
        Rc::new(move || {
            if let Err(error) = clear_saved_layouts(&connection.borrow()) {
                eprintln!("Could not clear saved views: {error}");
                return;
            }
            if let Some(info) = weak_info.upgrade() {
                info.set_photo_layout(grid::PhotoLayout::Grid);
            }
            gallery.set_layout(grid::PhotoLayout::Grid);
        })
    };
    {
        let clear = clear_saved_views.clone();
        info.connect_clear_saved_views(move || clear());
    }

    let switch_library_slot: Rc<RefCell<Option<Rc<dyn Fn(&str) -> Result<(), String>>>>> =
        Rc::new(RefCell::new(None));
    // Installed by layout.rs after the widgets have been constructed.
    let sidebar_mode_changed_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> =
        Rc::new(RefCell::new(None));
    let compact_visibility_changed_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> =
        Rc::new(RefCell::new(None));
    let settings_sidebar_mode_changed = sidebar_mode_changed_slot.clone();
    let settings_compact_visibility_changed = compact_visibility_changed_slot.clone();
    let settings_window = crate::settings::SettingsWindow::default();
    let menu_controls = settings_window.menu_controls.clone();
    let settings_window_for_switch = settings_window.clone();
    let settings_parent = window.clone();
    let settings_surface = window.clone();
    let settings_gallery_for_thumbs = gallery.clone();
    let settings_connection = connection.clone();
    let settings_gallery = gallery.clone();
    let settings_filter = filter.clone();
    let settings_search = search_text.clone();
    let settings_sort = sort.clone();
    let settings_lightbox = lightbox.clone();
    let settings_sidebar = sidebar_for_unavailable.clone();
    let settings_on_unavailable = availability_refresh.clone();
    let settings_album_list = album_list.clone();
    let settings_rebuild_folder_watches = rebuild_folder_watches.clone();
    let settings_theme_engine = theme_engine.clone();
    let settings_switch_library_slot = switch_library_slot.clone();
    let settings_clear_saved_views = clear_saved_views.clone();
    let present_settings: Rc<dyn Fn(Option<&'static str>)> = Rc::new(move |initial_page| {
        let connection = settings_connection.clone();
        let gallery = settings_gallery.clone();
        let filter = settings_filter.clone();
        let search = settings_search.clone();
        let sort = settings_sort.clone();
        let lightbox = settings_lightbox.clone();
        let sidebar = settings_sidebar.clone();
        let on_unavailable = settings_on_unavailable.clone();
        let theme_album_list = settings_album_list.clone();
        let watch_connection = settings_connection.clone();
        let watch_sidebar = settings_sidebar.clone();
        let watch_on_unavailable = settings_on_unavailable.clone();
        let maintenance = settings_maintenance.clone();
        let thumbs_window = settings_surface.clone();
        let thumbs_gallery = settings_gallery_for_thumbs.clone();
        let thumbs_connection = settings_connection.clone();
        let thumbs_theme_engine = settings_theme_engine.clone();
        let visibility_connection = settings_connection.clone();
        let visibility_sidebar = settings_sidebar.clone();
        let switch_library_slot = settings_switch_library_slot.clone();
        let settings_sidebar_mode_changed = settings_sidebar_mode_changed.clone();
        let settings_compact_visibility_changed = settings_compact_visibility_changed.clone();
        settings_window.present(
            &settings_parent,
            settings_connection.clone(),
            Rc::new(move || {
                lightbox.close();
                refresh_grid(
                    &connection,
                    filter.get(),
                    &search.borrow(),
                    sort.get(),
                    &gallery,
                );
                if let Some(sidebar) = sidebar.borrow().as_ref().cloned() {
                    if let Ok(folders) = db::folders(&connection.borrow()) {
                        sidebar::refresh_folder_rows(&sidebar, &folders, &on_unavailable);
                    }
                    if let Ok(counts) = db::sidebar_counts(&connection.borrow()) {
                        sidebar::refresh_library_counts(&sidebar, counts, &on_unavailable);
                    }
                }
            }),
            Rc::new(move || theme_album_list.changed(AlbumChange::Appearance)),
            Rc::new(move || {
                crate::window::debug_log("THUMB SETTINGS: apply callback entered");
                thumbs_gallery.set_honeycomb_border_width(crate::settings::honeycomb_border_width(&thumbs_connection.borrow()));
                let corner_style = thumbs_theme_engine.effective_corner_style();
                let fit = crate::settings::saved_bool(
                    &thumbs_connection.borrow(),
                    crate::db::THUMBNAIL_FIT_WHOLE_PHOTO_SETTING_KEY,
                )
                .unwrap_or(false);
                let show_file_names = crate::settings::saved_bool(
                    &thumbs_connection.borrow(),
                    crate::db::THUMBNAIL_FILE_NAMES_SETTING_KEY,
                )
                .unwrap_or(false);
                let high_quality =
                    crate::db::high_quality_thumbnails_enabled(&thumbs_connection.borrow());
                let quality_changed =
                    crate::thumbnail::high_quality_thumbnails_enabled() != high_quality;
                crate::thumbnail::set_high_quality_thumbnails_enabled(high_quality);
                if quality_changed {
                    // 320 and 640 deliberately share one canonical cache path.
                    // Rebuild in place instead of keeping a second Photo Wall cache.
                    if let Err(error) = crate::thumbnail::clear_cache() {
                        eprintln!("Could not rebuild thumbnail cache for quality change: {error}");
                    }
                    crate::grid::clear_folder_thumbnail_cache();
                    thumbs_gallery.refresh_thumbnails();
                }
                corner_style.apply(&thumbs_window);
                crate::window::debug_log(&format!(
                    "THUMB SETTINGS: corner style applied ({corner_style:?}), deferring fit={fit} to idle, high_quality={high_quality}"
                ));
                // Defer the tile walk out of the switch notification so the
                // settings UI settles before the gallery relayouts.
                let gallery_for_fit = thumbs_gallery.clone();
                glib::idle_add_local_once(move || {
                    crate::window::debug_log("THUMB SETTINGS: set_fit_whole_photo begin");
                    gallery_for_fit.set_fit_whole_photo(fit);
                    crate::window::debug_log("THUMB SETTINGS: set_fit_whole_photo end");
                });
                let gallery_for_names = thumbs_gallery.clone();
                glib::idle_add_local_once(move || {
                    gallery_for_names.set_show_file_names(show_file_names);
                });
                crate::window::debug_log("THUMB SETTINGS: apply callback exit");
            }),
            {
                let gallery = settings_gallery.clone();
                let connection = settings_connection.clone();
                Rc::new(move || {
                    gallery.set_zoom_animations_enabled(db::zoom_animations_enabled(
                        &connection.borrow(),
                    ));
                })
            },
            settings_clear_saved_views.clone(),
            Rc::new(move || {
                if let Some(sidebar) = visibility_sidebar.borrow().as_ref().cloned() {
                    let visibility = sidebar::SidebarVisibility::from_connection(
                        &visibility_connection.borrow(),
                    );
                    sidebar::apply_visibility(&sidebar, visibility);
                }
                if let Some(refresh) = settings_sidebar_mode_changed.borrow().as_ref() {
                    refresh();
                }
                if let Some(refresh) = settings_compact_visibility_changed.borrow().as_ref() {
                    refresh();
                }
            }),
            {
                let rebuild_folder_watches = settings_rebuild_folder_watches.clone();
                Rc::new(move || {
                    rebuild_folder_watches();
                    if let Some(sidebar) = watch_sidebar.borrow().as_ref().cloned() {
                        if let Ok(folders) = db::folders(&watch_connection.borrow()) {
                            sidebar::refresh_folder_rows(
                                &sidebar,
                                &folders,
                                &watch_on_unavailable,
                            );
                        }
                    }
                })
            },
            maintenance,
            crate::settings::DatabaseManagement {
                switch_library: Rc::new(move |id| {
                    switch_library_slot
                        .borrow()
                        .as_ref()
                        .ok_or_else(|| "library switcher is not ready".to_owned())?(id)
                }),
            },
            settings_theme_engine.clone(),
            initial_page,
        );
    });
    let sd_import_slot_for_button = sd_import_slot.clone();
    info.import_photos.connect_clicked(move |_| {
        if let Some(callback) = sd_import_slot_for_button.borrow().as_ref() {
            callback();
        }
    });

    let present_settings_from_more = present_settings.clone();
    info.more.connect_clicked(move |_| {
        present_settings_from_more(None);
    });

    let action_context_for_lightbox = action_context.clone();
    lightbox.set_context_menu_handler(move |photo, anchor, x, y| {
        show_photo_context_menu(photo, anchor, action_context_for_lightbox.clone(), x, y);
    });


    gallery.root.add_css_class("photo-grid");

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);

    // Status/progress is embedded in the header title area (next to the
    // search entry) instead of its own row above the grid: appearing or
    // hiding it must never resize the photo grid.
    let refresh_status_box = operation_progress.root().clone();
    crate::startup_trace::mark("actions_configured", Some(build_started));
    let refresh_status_spinner = operation_progress.spinner().clone();
    let refresh_status_label = operation_progress.label().clone();
    let refresh_status_stop = operation_progress.stop().clone();

    let grid_scroll = gtk::ScrolledWindow::new();
    grid_scroll.set_vexpand(true);
    grid_scroll.add_css_class("pic-gallery-scroller");
    grid_scroll.set_hexpand(true);
    // GtkGridView remains the direct scrollable child for every non-Folder
    // view, preserving its existing virtualization and selection behavior.
    grid_scroll.set_child(Some(&gallery.root));
    crate::startup_trace::mark("grid_scroll_attached", Some(build_started));

    let folder_scroll = gtk::ScrolledWindow::new();
    folder_scroll.set_vexpand(true);
    folder_scroll.add_css_class("pic-gallery-scroller");
    folder_scroll.set_hexpand(true);
    // Folder mode now uses the section-aware flat-photo surface by default.
    // The legacy ListView remains available behind PICASA_LEGACY_FOLDER_LIST=1.
    if crate::grid::sectioned_folder_view_enabled() {
        folder_scroll.set_child(Some(&gallery.folder_sectioned_root));
    } else {
        // Keep the virtual surface available for Photo Wall even in legacy Grid mode.
        folder_scroll.set_child(Some(&gallery.folder_root));
    }
    gallery.attach_sectioned_folder_scroll(&folder_scroll);
    let folder_scroll_overlay = gtk::Overlay::new();
    folder_scroll_overlay.set_hexpand(true);
    folder_scroll_overlay.set_vexpand(true);
    folder_scroll_overlay.set_child(Some(&folder_scroll));
    if !crate::grid::sectioned_folder_view_enabled() {
        // Legacy Folder ListView owns the external rubber-band overlay.
        // SectionedFolderView draws and positions its own band inside GtkFixed.
        folder_scroll_overlay.add_overlay(&gallery.folder_rubberband);
    }

    // A temporary date bubble makes a long chronological All Photos scrollbar
    // usable like a timeline. It is deliberately attached only to the GridView
    // scrollbar: Folder mode is not globally date-sorted.
    let scrub_date_label = gtk::Label::new(None);
    scrub_date_label.set_halign(gtk::Align::End);
    scrub_date_label.set_valign(gtk::Align::Center);
    scrub_date_label.set_margin_end(34);
    scrub_date_label.set_can_target(false);
    scrub_date_label.set_visible(false);
    scrub_date_label.add_css_class("scroll-scrub-date");
    let scrub_dragging = Rc::new(Cell::new(false));
    let scrub_hide_generation = Rc::new(Cell::new(0_u64));
    let update_scrub_date: Rc<dyn Fn(f64)> = Rc::new({
        let gallery = gallery.clone();
        let filter = filter.clone();
        let sort = sort.clone();
        let label = scrub_date_label.clone();
        move |scroll_y: f64| {
            let date = match sort.get().field {
                SortField::DateTaken => grid::GroupDate::Taken,
                SortField::DateAdded => grid::GroupDate::Added,
                _ => {
                    label.set_visible(false);
                    return;
                }
            };
            if filter.get() != sidebar::SidebarFilter::All {
                label.set_visible(false);
                return;
            }
            label.set_text(&gallery.month_label_for_scroll_position(scroll_y, date));
            label.set_visible(true);
        }
    });
    let scrollbar = grid_scroll.vscrollbar();
    let scrub_press = gtk::GestureClick::new();
    scrub_press.set_button(0);
    {
        let dragging = scrub_dragging.clone();
        let hide_generation = scrub_hide_generation.clone();
        let adjustment = grid_scroll.vadjustment();
        let update = update_scrub_date.clone();
        scrub_press.connect_pressed(move |_, _, _, _| {
            hide_generation.set(hide_generation.get().wrapping_add(1));
            dragging.set(true);
            update(adjustment.value());
        });
    }
    {
        let dragging = scrub_dragging.clone();
        let hide_generation = scrub_hide_generation.clone();
        let label = scrub_date_label.clone();
        scrub_press.connect_released(move |_, _, _, _| {
            if !dragging.replace(false) {
                return;
            }
            let generation = hide_generation.get().wrapping_add(1);
            hide_generation.set(generation);
            let label = label.clone();
            let hide_generation = hide_generation.clone();
            glib::timeout_add_local_once(Duration::from_millis(550), move || {
                if hide_generation.get() == generation {
                    label.set_visible(false);
                }
            });
        });
    }
    scrollbar.add_controller(scrub_press);
    {
        let dragging = scrub_dragging.clone();
        let update = update_scrub_date.clone();
        grid_scroll.vadjustment().connect_value_changed(move |adjustment| {
            if dragging.get() {
                update(adjustment.value());
            }
        });
    }

    let gallery_scroll_stack = gtk::Stack::new();
    gallery_scroll_stack.set_hexpand(true);
    gallery_scroll_stack.set_vexpand(true);
    gallery_scroll_stack.add_named(&grid_scroll, Some("grid"));
    gallery_scroll_stack.add_named(&folder_scroll_overlay, Some("folders"));
    {
        let gallery_scroll_stack = gallery_scroll_stack.clone();
        let gallery_for_folder_view = gallery.clone();
        let folder_scroll_for_layout = folder_scroll.clone();
        gallery.set_folder_view_changed_handler(move |folder_mode, layout_changed| {
            crate::nav_transition_trace!(
                "stage=folder_view_present folder_mode={folder_mode} layout_changed={layout_changed} layout={:?} visible_before={:?}",
                gallery_for_folder_view.layout(),
                gallery_scroll_stack.visible_child_name()
            );
            // Capture before switching children: GTK may move focus when a
            // focused renderer is hidden. Search and sidebar focus stay put.
            let had_gallery_focus = gallery_owns_focus(&gallery_scroll_stack);
            let wall = gallery_for_folder_view.layout() != grid::PhotoLayout::Grid;
            let folder_sectioned =
                wall || (crate::grid::sectioned_folder_view_enabled() && folder_mode);
            if folder_sectioned {
                if folder_scroll_for_layout.child().as_ref()
                    != Some(gallery_for_folder_view.folder_sectioned_root.upcast_ref())
                    && (gallery_for_folder_view.layout()!=grid::PhotoLayout::Honeycomb
                        || !gallery_for_folder_view.folder_sectioned_root.is_ancestor(&folder_scroll_for_layout))
                {
                    folder_scroll_for_layout
                        .set_child(Some(&gallery_for_folder_view.folder_sectioned_root));
                }
            } else if !crate::grid::sectioned_folder_view_enabled() {
                folder_scroll_for_layout.set_child(Some(&gallery_for_folder_view.folder_root));
            }
            let folder_grid_experiment = crate::grid::folder_gridview_experiment_enabled()
                && folder_mode
                && !folder_sectioned;
            gallery_scroll_stack.set_visible_child_name(
                if wall || (folder_mode && !folder_grid_experiment) {
                    "folders"
                } else {
                    "grid"
                },
            );
            crate::nav_transition_trace!(
                "stage=folder_view_presented folder_mode={folder_mode} layout_changed={layout_changed} layout={:?} wall={wall} sectioned={folder_sectioned} visible={:?}",
                gallery_for_folder_view.layout(),
                gallery_scroll_stack.visible_child_name()
            );
            if folder_mode || wall {
                if folder_sectioned {
                    if std::env::var_os("PICASA_TRACE").is_some() {
                        eprintln!("PIC_FOLDER_SECTIONED enabled mode=flat_photo_virtualized");
                    }
                    transfer_gallery_focus(
                        had_gallery_focus,
                        &gallery_for_folder_view.folder_sectioned_root,
                    );
                    // In Folder Grid, set_layout() already rebuilt this
                    // model. Rebuilding it again here invalidates the pending
                    // Grid anchor restore, so the layout transition never
                    // reports ready. Other presentation changes still need
                    // the deferred refresh here.
                    let folder_grid_layout_switch = layout_changed
                        && folder_mode
                        && gallery_for_folder_view.layout() == grid::PhotoLayout::Grid;
                    if !folder_grid_layout_switch {
                        let gallery = gallery_for_folder_view.clone();
                        glib::idle_add_local_once(move || {
                            gallery.refresh_sectioned_folder();
                        });
                    }
                    return;
                }
                if folder_grid_experiment {
                    if std::env::var_os("PICASA_TRACE").is_some() {
                        eprintln!("PIC_FOLDER_GRIDVIEW enabled mode=photo_grid folder_indicator=sticky");
                    }
                    transfer_gallery_focus(had_gallery_focus, &gallery_for_folder_view.root);
                    gallery_for_folder_view
                        .update_group_header_for_scroll(gallery_for_folder_view.scroll_position());
                    let gallery = gallery_for_folder_view.clone();
                    glib::timeout_add_local_once(Duration::from_millis(90), move || {
                        gallery.refresh_visible_grid_tiles();
                    });
                    return;
                }
                // Keep keyboard focus on the widget that is actually shown.
                // Tab/search helpers used to target the hidden GridView.
                transfer_gallery_focus(had_gallery_focus, &gallery_for_folder_view.folder_root);
                // The Folder model rebuild is synchronous. This timeout runs
                // after that work returns to GTK and paints only the final
                // visible viewport instead of every intermediate bound row.
                let gallery = gallery_for_folder_view.clone();
                glib::timeout_add_local_once(Duration::from_millis(90), move || {
                    gallery.refresh_visible_folder_tiles();
                });
            }
        });
    }

    // Non-Folder GridView (Library, Favourites, Recently Added, Albums,
    // Search) needs the same current-viewport protection as Folder mode. Fast
    // scrollbar motion can otherwise leave hundreds of stale bind requests in
    // front of the final viewport and make thumbnails appear blank until idle.
    // What All Photos does while it moves: scrub, target sampling, settle
    // and the per-frame thumbnail pump (see `window/scroll_motion.rs`).
    let grid_motion = Rc::new(RefCell::new(scroll_motion::ScrollMotion::new(
        scroll_motion::ALL_PHOTOS,
    )));
    let grid_scroll_direction = Rc::new(Cell::new(0.0_f64));
    let last_grid_scroll_y = Rc::new(Cell::new(0.0_f64));
    // The latest scrub position and page, for the settle's final target.
    let grid_scrub_latest = Rc::new(Cell::new((0.0_f64, 0.0_f64)));

    // Optional native frame-clock trace for real-gallery scroll profiling.
    // The adjustment is driven by GTK; this records presentation intervals
    // without involving screenshot or accessibility polling in the samples.
    if std::env::var_os("PIC_SCROLL_FRAME_TRACE").is_some() {
        for (source, scroller) in [("grid", &grid_scroll), ("folder", &folder_scroll)] {
            let trace_installed = Rc::new(Cell::new(false));
            let trace_installed_for_event = trace_installed.clone();
            let trace_until = Rc::new(Cell::new(None::<Instant>));
            let trace_until_for_event = trace_until.clone();
            let last_frame = Rc::new(Cell::new(None::<Instant>));
            let window_for_scroll_trace = window.clone();
            let gallery_for_scroll_trace = gallery.clone();
            let adjustment_for_scroll_trace = scroller.vadjustment();
            scroller
                .vadjustment()
                .connect_value_changed(move |_| {
                    trace_until_for_event.set(Some(Instant::now() + Duration::from_millis(250)));
                    if trace_installed_for_event.get() {
                        return;
                    }
                    let Some(clock) = window_for_scroll_trace
                        .surface()
                        .map(|surface| surface.frame_clock())
                    else {
                        return;
                    };
                    trace_installed_for_event.set(true);
                    let trace_until = trace_until_for_event.clone();
                    let last_frame = last_frame.clone();
                    let gallery_for_paint = gallery_for_scroll_trace.clone();
                    let window_for_paint = window_for_scroll_trace.clone();
                    let adjustment_for_paint = adjustment_for_scroll_trace.clone();
                    clock.connect_after_paint(move |_| {
                        let now = Instant::now();
                        let Some(until) = trace_until.get() else {
                            return;
                        };
                        if now > until {
                            last_frame.set(None);
                            return;
                        }
                        if let Some(previous) = last_frame.replace(Some(now)) {
                            // Window size and scroll position let frame pacing be
                            // compared per simulated screen size, and motion
                            // smoothness measured as pixels moved per frame.
                            eprintln!(
                                "PIC_SCROLL_FRAME source={source} layout={:?} window={}x{} value={:.1} elapsed_ms={:.3} interval_ms={:.3}",
                                gallery_for_paint.layout(),
                                window_for_paint.width(),
                                window_for_paint.height(),
                                adjustment_for_paint.value(),
                                now.duration_since(until - Duration::from_millis(250)).as_secs_f64()
                                    * 1000.0,
                                now.duration_since(previous).as_secs_f64() * 1000.0,
                            );
                        } else {
                            last_frame.set(Some(now));
                        }
                    });
                });
        }
    }

    let gallery_for_group_scroll = gallery.clone();
    let grid_motion_for_event = grid_motion.clone();
    let grid_scroll_direction_for_event = grid_scroll_direction.clone();
    grid_scroll
        .vadjustment()
        .connect_value_changed(move |adjustment| {
            let value = adjustment.value();
            let previous = last_grid_scroll_y.replace(value);
            let delta = value - previous;
            grid_scroll_direction_for_event.set(delta.signum());
            crate::grid::set_gallery_scrolling(true);

            // A scrollbar scrub can teleport farther than GTK can realize
            // GridView children in the same frame, so a scrub queues its
            // destination from the photo model instead of waiting for widget
            // binds. Once started, every following move belongs to the same
            // drag until it settles: the final thumb movement can be much
            // smaller than the first jump.
            let page = adjustment.page_size();
            let moved = grid_motion_for_event
                .borrow_mut()
                .moved(delta, page, Instant::now());
            let spec = scroll_motion::ALL_PHOTOS;
            if moved.scrub_started {
                crate::grid::set_grid_scrub_active(true);
            }
            if moved.scrubbing {
                grid_scrub_latest.set((value, page));
            }
            if moved.sample {
                gallery_for_group_scroll.queue_grid_scroll_target_cached_tiles_async(
                    value,
                    page,
                    spec.target_tiles,
                );
            }
            if let Some(ticket) = moved.settle {
                let motion = grid_motion_for_event.clone();
                let latest = grid_scrub_latest.clone();
                let gallery = gallery_for_group_scroll.clone();
                glib::timeout_add_local_once(spec.settle_after, move || {
                    let settled = motion.borrow_mut().settle(ticket);
                    if !settled {
                        return;
                    }
                    crate::grid::set_gallery_scrolling(false);
                    // Force the final destination once more: the thumb may
                    // have been released between two samples.
                    let (value, page) = latest.get();
                    gallery.queue_grid_scroll_target_cached_tiles_async(
                        value,
                        page,
                        spec.target_tiles,
                    );
                    crate::grid::set_grid_scrub_active(false);
                    gallery.refresh_visible_grid_tiles();
                });
            }

            gallery_for_group_scroll.update_group_header_for_scroll(value);
        });

    let gallery_for_grid_motion_tick = gallery.clone();
    let grid_motion_for_tick = grid_motion.clone();
    grid_scroll.add_tick_callback(move |_, _| {
        let frame = grid_motion_for_tick.borrow_mut().frame(Instant::now());
        let Some(frame) = frame else {
            return glib::ControlFlow::Continue;
        };
        let spec = scroll_motion::ALL_PHOTOS;

        // During a scrub the model-derived target queued by the adjustment
        // callback is authoritative. Do NOT replace it with widget-derived
        // visibility here: GtkGridView may still expose recycled cells from
        // the previous viewport for several frames. Applying the paintables
        // the workers already finished is still required, though: during
        // fast movement a tile binds once, usually before its decode
        // completes, and the completion drain misses tiles that were recycled
        // again before the decode arrived. Re-checking visible tiles against
        // the RAM cache every frame heals exactly those tiles.
        if frame.scrubbing {
            gallery_for_grid_motion_tick
                .apply_visible_cached_paintables(crate::grid::GalleryScroller::Photos);
        } else {
            gallery_for_grid_motion_tick.queue_visible_cached_tiles_async(
                crate::grid::GalleryScroller::Photos,
                spec.visible_tiles,
            );
        }

        // During a scrollbar teleport all decode capacity belongs to the
        // target viewport; warming ahead resumes once GTK has had several
        // frames to rebind the destination cells.
        if frame.warm_ahead {
            gallery_for_grid_motion_tick.prefetch_cached_tiles(
                crate::grid::GalleryScroller::Photos,
                spec.warm_tiles,
                grid_scroll_direction.get(),
            );
        }

        glib::ControlFlow::Continue
    });

    let gallery_for_folder_scroll = gallery.clone();
    let latest_folder_scroll_y = Rc::new(Cell::new(0.0_f64));
    // Last scroll direction, used to warm thumbnails ahead of the user rather
    // than both sides equally.
    let folder_scroll_direction = Rc::new(Cell::new(0.0_f64));
    let folder_scroll_direction_for_event = folder_scroll_direction.clone();
    // Thumbnail loading is intentionally debounced until Folder motion stops.
    // GtkListView may rebind thousands of intermediate rows during a scrollbar
    // jump; loading thumbnails from each bind is pure wasted main-thread work.
    let folder_thumbnail_debounce: Rc<RefCell<Option<glib::SourceId>>> =
        Rc::new(RefCell::new(None));
    let folder_thumbnail_debounce_for_event = folder_thumbnail_debounce.clone();
    let folder_thumbnail_prefetch: Rc<RefCell<Option<glib::SourceId>>> =
        Rc::new(RefCell::new(None));
    let folder_thumbnail_prefetch_for_event = folder_thumbnail_prefetch.clone();
    // What Folders does while it moves: scrub, target sampling, settle and
    // the per-frame thumbnail pump (see `window/scroll_motion.rs`). Only a
    // real multi-page scrollbar scrub counts; Page Up/Down does not, because
    // Folder row bind submits its own async visible request.
    let folder_motion = Rc::new(RefCell::new(scroll_motion::ScrollMotion::new(
        scroll_motion::FOLDERS,
    )));
    let folder_motion_for_event = folder_motion.clone();
    let latest_folder_scroll_y_for_tick = latest_folder_scroll_y.clone();
    let folder_vadjustment = folder_scroll.vadjustment();
    // The settled loader paints the final viewport after motion stops. During
    // active motion, drive thumbnail work from GTK frame ticks instead of a
    // 16 ms timeout. A timeout can run before GtkListView has rebound/allocated
    // the rows for a large scrollbar jump, which warms the old viewport and
    // leaves the new one blank. The pump keeps retrying long enough for
    // recycled rows and async cache decodes to catch up.
    let gallery_for_thumbnail_motion_tick = gallery.clone();
    let folder_scroll_direction_for_tick = folder_scroll_direction.clone();
    folder_scroll.add_tick_callback(move |_, _| {
        let frame = folder_motion.borrow_mut().frame(Instant::now());
        let Some(frame) = frame else {
            return glib::ControlFlow::Continue;
        };
        let spec = scroll_motion::FOLDERS;

        // Always prioritise the tiles actually visible in the frame GTK is
        // about to paint. queue_visible... uses reserved async capacity, so
        // stale prefetch requests cannot starve a scrollbar jump.
        if frame.scrubbing {
            // The model-derived scrub target is authoritative while the thumb is
            // teleporting. GtkListView may still expose rows from the previous
            // viewport, so never let those recycled widgets replace the target
            // queue during an active direct scrub. Thumbnails the workers have
            // already finished must still be displayed: rows bind once, usually
            // before their decode completes, and the completion drain misses
            // rows recycled again mid-scrub. Applying RAM hits every frame
            // heals those rows without touching the decode queue.
            gallery_for_thumbnail_motion_tick
                .apply_visible_cached_paintables(crate::grid::GalleryScroller::Folder);
        } else {
            gallery_for_thumbnail_motion_tick.queue_visible_cached_tiles_async(
                crate::grid::GalleryScroller::Folder,
                spec.visible_tiles,
            );
        }

        // Scrub decode targets are frame-driven: the first sample fires in the
        // jump frame itself, then at most every `sample_every` while the drag
        // continues.
        if frame.sample {
            let page = folder_vadjustment.page_size().max(1.0);
            gallery_for_thumbnail_motion_tick.queue_folder_scroll_target_cached_tiles_async(
                latest_folder_scroll_y_for_tick.get(),
                page,
                spec.target_tiles,
            );
        }

        // Warming ahead is useful, but doing the larger offscreen scan on every
        // frame is unnecessary, so it runs on every third pump frame.
        if frame.warm_ahead {
            gallery_for_thumbnail_motion_tick.prefetch_cached_tiles(
                crate::grid::GalleryScroller::Folder,
                spec.warm_tiles,
                folder_scroll_direction_for_tick.get(),
            );
        }

        glib::ControlFlow::Continue
    });

    folder_scroll
        .vadjustment()
        .connect_value_changed(move |adjustment| {
            let raw_scroll_y = adjustment.value();
            // GtkListView keeps its scroll anchor on device-pixel boundaries
            // (same reason the wheel path quantizes in
            // install_smooth_gallery_scroll). Fractional scrollbar-drag values
            // make GTK immediately write a rounded value back, which reads as
            // a tiny bounce at drag end. Snap to whole pixels instead; the
            // re-entrant value_changed sees an integral value and no-ops.
            let scroll_y = raw_scroll_y.round();
            if scroll_y != raw_scroll_y {
                adjustment.set_value(scroll_y);
            }
            let previous_y = latest_folder_scroll_y.replace(scroll_y);
            let direction = scroll_y - previous_y;
            if direction != 0.0 {
                folder_scroll_direction_for_event.set(direction);
            }
            crate::grid::set_gallery_scrolling(true);

            // Folder bind submits its own visible-priority async request on
            // every RAM miss. That is sufficient for wheel motion and Page
            // Up/Down; only a multi-page scrollbar scrub uses model-derived
            // target preloading, sampled by the pump.
            let page_size = adjustment.page_size().max(1.0);
            let moved = folder_motion_for_event
                .borrow_mut()
                .moved(direction, page_size, Instant::now());
            if moved.scrub_started {
                // Folder and grid views are never visible at the same time,
                // so reuse the shared scrub thread-local: it keeps recycled
                // folder tiles from being blanked at unbind while rows are
                // rebound faster than thumbnail decodes can land. The folder
                // settle callback clears it again.
                crate::grid::set_grid_scrub_active(true);
            }

            // Keep only the cheap position bookkeeping in the raw adjustment
            // callback. Widget picking and sidebar work are throttled below so
            // wheel/touchpad/scrollbar motion cannot spend a frame walking GTK
            // widgets merely to update a visual location marker.
            gallery_for_folder_scroll.update_group_header_for_scroll(scroll_y);

            // Cancel the previous settle callback and arm a new one. Only the
            // final viewport after the settle delay gets a full visible
            // refresh; the frame pump (re-armed by every move) covers
            // wheel/touchpad motion, kinetic scrolling and scrollbar jumps
            // in between.
            if let Some(source) = folder_thumbnail_debounce_for_event.borrow_mut().take() {
                source.remove();
            }
            if let Some(source) = folder_thumbnail_prefetch_for_event.borrow_mut().take() {
                source.remove();
            }
            let Some(ticket) = moved.settle else {
                return;
            };
            let gallery_for_visible = gallery_for_folder_scroll.clone();
            let debounce_slot = folder_thumbnail_debounce_for_event.clone();
            let prefetch_slot = folder_thumbnail_prefetch_for_event.clone();
            let direction_for_settle = folder_scroll_direction_for_event.get();
            let final_scroll_y = latest_folder_scroll_y.clone();
            let final_page_size = page_size;
            let motion_for_settle = folder_motion_for_event.clone();
            let spec = scroll_motion::FOLDERS;
            let source = glib::timeout_add_local(spec.settle_after, move || {
                debounce_slot.borrow_mut().take();
                let settled = motion_for_settle.borrow_mut().settle(ticket);
                if !settled {
                    return glib::ControlFlow::Break;
                }
                crate::grid::set_gallery_scrolling(false);
                // End the shared scrub window before the visible refresh so
                // tiles that never received their thumbnail drop the stale
                // backstop image and return to the normal placeholder state.
                crate::grid::set_grid_scrub_active(false);
                gallery_for_visible.queue_folder_scroll_target_cached_tiles_async(
                    final_scroll_y.get(),
                    final_page_size,
                    spec.target_tiles,
                );
                // Defer the visible refresh to the next main-loop pass so GTK
                // can finish this frame's post-jump anchor/allocation work
                // before tiles drop their stale backstop paintables. Unloading
                // in the same callback interleaves with the ListView's row
                // re-positioning and reads as a small bounce at drag end.
                // Queueing decodes stays inline; it does not touch widgets.
                let gallery_for_settle_refresh = gallery_for_visible.clone();
                glib::timeout_add_local_once(Duration::ZERO, move || {
                    gallery_for_settle_refresh.refresh_visible_folder_tiles();
                });

                // GtkListView can realize the last destination row a few frames
                // *after* the settle callback above. Give those newly-created
                // tiles a short visible-only catch-up window before spending
                // worker capacity on speculative ahead/behind prefetch. This is
                // deliberately additive: it never replaces queued visible work.
                let gallery_for_prefetch = gallery_for_visible.clone();
                let prefetch_slot_for_tick = prefetch_slot.clone();
                let catchup_frames = Rc::new(Cell::new(6u8));
                let catchup_frames_for_tick = catchup_frames.clone();
                let prefetch_allowance = gallery_for_prefetch.thumbnail_prefetch_allowance();
                let prefetched_total = Cell::new(0usize);
                let prefetch_source = glib::timeout_add_local(
                    Duration::from_millis(16),
                    move || {
                        let frames_left = catchup_frames_for_tick.get();
                        if frames_left > 0 {
                            gallery_for_prefetch.refresh_visible_folder_tiles();
                            catchup_frames_for_tick.set(frames_left - 1);
                            return glib::ControlFlow::Continue;
                        }

                        let remaining =
                            prefetch_allowance.saturating_sub(prefetched_total.get());
                        let loaded = gallery_for_prefetch
                            .prefetch_cached_tiles(
                                crate::grid::GalleryScroller::Folder,
                                remaining.min(24),
                                direction_for_settle,
                            );
                        prefetched_total.set(prefetched_total.get() + loaded);
                        // Stop once everything is loaded, or once this settle
                        // has prefetched what the RAM cache can hold; more
                        // would only evict its own earlier work, forever.
                        if (loaded == 0 || prefetched_total.get() >= prefetch_allowance)
                            && !gallery_for_prefetch.thumbnail_display_work_pending()
                        {
                            prefetch_slot_for_tick.borrow_mut().take();
                            glib::ControlFlow::Break
                        } else {
                            glib::ControlFlow::Continue
                        }
                    },
                );
                prefetch_slot.replace(Some(prefetch_source));
                glib::ControlFlow::Break
            });
            folder_thumbnail_debounce_for_event.replace(Some(source));

        });

    

    install_smooth_gallery_scroll(&grid_scroll, gallery.clone(), true);
    // Folder mode uses a variable-height GtkListView. Use relative wheel
    // easing rather than an absolute spring target so GTK anchor corrections
    // cannot pull the viewport backwards. Precision touchpads remain native.
    install_folder_smooth_gallery_scroll(&folder_scroll, gallery.clone());

    // Divider dragging publishes live allocations in every gallery mode.
    let sidebar_resize_active = Rc::new(Cell::new(false));
    let inspector_resize_active = Rc::new(Cell::new(false));
    let inspector_transition_active = Rc::new(Cell::new(false));
    let inspector_trace_last_width = Rc::new(Cell::new(-1_i32));
    let inspector_trace_frames = Rc::new(Cell::new(0_u16));
    // Temporary hover-autohide is presentation only. While that overlay is
    // sliding in or out, keep the gallery's logical width/column model frozen
    // so Folder mode does not rebuild rows just because the pointer touched
    // the left edge. Pin/unpin remains a real layout change and uses the settle
    // gate below to perform one final responsive update after the animation.
    let sidebar_hover_layout_freeze = Rc::new(Cell::new(false));
    // Generation protects against a delayed hide callback unfreezing a newer
    // hover reveal when the pointer returns to the edge very quickly.
    let sidebar_hover_freeze_generation = Rc::new(Cell::new(0u64));
    let sidebar_layout_settle = Rc::new(RefCell::new(WidthSettleGate::new(3)));
    let gallery_for_resize = gallery.clone();
    let sidebar_resize_active_for_tick = sidebar_resize_active.clone();
    let inspector_resize_active_for_tick = inspector_resize_active.clone();
    let inspector_transition_active_for_tick = inspector_transition_active.clone();
    let inspector_trace_last_width_for_tick = inspector_trace_last_width.clone();
    let inspector_trace_frames_for_tick = inspector_trace_frames.clone();
    let sidebar_hover_layout_freeze_for_tick = sidebar_hover_layout_freeze.clone();
    let sidebar_layout_settle_for_tick = sidebar_layout_settle.clone();
    let inspector_trace_enabled = std::env::var_os("PIC_INSPECTOR_TRACE").is_some();
    gallery_scroll_stack.add_tick_callback(move |surface, _clock| {
        gallery_for_resize.present_frame();
        let live_resize =
            (sidebar_resize_active_for_tick.get() || inspector_resize_active_for_tick.get())
                && !sidebar_hover_layout_freeze_for_tick.get();
        if inspector_trace_enabled
            && (inspector_resize_active_for_tick.get() || inspector_transition_active_for_tick.get())
        {
            let width = surface.width();
            if inspector_trace_last_width_for_tick.replace(width) != width {
                let samples = inspector_trace_frames_for_tick.get();
                if samples < 90 {
                    eprintln!(
                        "PIC_INSPECTOR_TRACE stage=frame phase={} sample={} viewport_width={} gallery_root_width={} layout={:?} virtual={} scroll_y={:.1}",
                        if inspector_resize_active_for_tick.get() { "drag" } else { "slide" },
                        samples,
                        width,
                        gallery_for_resize.root.width(),
                        gallery_for_resize.layout(),
                        gallery_for_resize.using_sectioned_folder_view(),
                        gallery_for_resize.scroll_position()
                    );
                } else if samples == 90 {
                    eprintln!("PIC_INSPECTOR_TRACE stage=frame_limit reached=90");
                }
                inspector_trace_frames_for_tick.set(samples.saturating_add(1));
            }
        }
        // Folder Grid has a sectioned, virtualized row model. Rebuilding it
        // at each intermediary inspector width re-anchors the same photo
        // multiple times (e.g. 5->4->3 columns), causing large visible jumps.
        // Keep that grid's column geometry fixed during the native split slide;
        // the show-sidebar settle handler commits the final width once.
        // Photo Wall/Masonry and ordinary resize gestures are unchanged.
        if should_defer_inspector_folder_grid_reflow(
            inspector_transition_active_for_tick.get(),
            gallery_for_resize.layout() == grid::PhotoLayout::Grid,
            gallery_for_resize.using_sectioned_folder_view(),
        ) && !inspector_resize_active_for_tick.get()
        {
            return glib::ControlFlow::Continue;
        }
        if live_resize
            || should_observe_width(
                sidebar_resize_active_for_tick.get() || inspector_resize_active_for_tick.get(),
                sidebar_hover_layout_freeze_for_tick.get(),
            )
        {
            let width = surface.width();
            if width > 100 {
                if live_resize || gallery_for_resize.using_sectioned_folder_view() {
                    // Publish the current columns during divider dragging too,
                    // so Photos Grid keeps fitting its newly allocated width.
                    gallery_for_resize.update_width(width);
                } else if crate::grid::folder_gridview_experiment_enabled() {
                    // Gallery v2 has no column-sized Folder row model to
                    // rebuild, so react immediately when the window crosses a
                    // column boundary. Delaying until the width settles leaves
                    // GridView's previous min-column request active while the
                    // window is dragged smaller, causing repeated
                    // GtkOverlay/Adwaita minimum-width negotiation.
                    gallery_for_resize.update_width_with_flip(width);
                } else if sidebar_layout_settle_for_tick
                    .borrow_mut()
                    .observe(width)
                {
                    // Legacy Folder ListView still benefits from coalescing
                    // because a column change rebuilds its row model.
                    gallery_for_resize.update_width(width);
                }
            }
        }
        glib::ControlFlow::Continue
    });
    crate::startup_trace::mark("scroll_callbacks_connected", Some(build_started));

    // Restore the startup layout without a transition. The interactive
    // Grid/Wall/Masonry selector is connected after page_overlay exists so
    // layout changes can reuse the same retained-frame crossfade.
    gallery.set_honeycomb_border_width(crate::settings::honeycomb_border_width(
        &connection.borrow(),
    ));
    restore_section_layout(&connection.borrow(), &gallery, &info, initial_filter);
    crate::startup_trace::mark("initial_layout_restored", Some(build_started));

    let grid_surface = gtk::Box::new(gtk::Orientation::Vertical, 0);
    grid_surface.set_hexpand(true);
    grid_surface.set_vexpand(true);
    grid_surface.add_css_class("photo-grid");
    // Date grouping keeps its existing heading. Folder grouping hides this
    // widget and renders its headers inside the folder scroller instead.
    grid_surface.append(&gallery.group_header);
    grid_surface.append(&gallery_scroll_stack);

    let date_browser_button = gtk::ToggleButton::new();
    date_browser_button.set_icon_name("x-office-calendar-symbolic");
    date_browser_button.set_tooltip_text(Some("Browse photos by year and month"));
    date_browser_button.add_css_class("pic-header-action");
    date_browser_button.set_visible(filter.get() == sidebar::SidebarFilter::All);
    let date_browser_snapshot: Rc<RefCell<Option<grid::DateBrowserSnapshot>>> = Rc::new(RefCell::new(None));
    let date_browser_resume = Rc::new(RefCell::new(None::<(Option<(i32, u32)>, Option<i64>)>));
    let sync_date_browser_photo_info: Rc<dyn Fn()> = {
        let gallery = Rc::downgrade(&gallery);
        let info = Rc::downgrade(&info);
        Rc::new(move || {
            let (Some(gallery), Some(info)) = (gallery.upgrade(), info.upgrade()) else {
                return;
            };
            let (count, photos) = gallery.selected_photo_preview(SELECTION_INFOBAR_PREVIEW_LIMIT);
            info.show_date_browser_selection(count, &photos);
        })
    };
    let exit_date_browser: Rc<dyn Fn()> = {
        let front = front.clone();
        let snapshot = date_browser_snapshot.clone();
        let gallery = gallery.clone();
        let browser_slot = Rc::downgrade(&date_browser_slot);
        let active = date_browser_active.clone();
        let button = date_browser_button.downgrade();
        let scroll = gallery_scroll_stack.clone();
        let filter = filter.clone();
        let sort = sort.clone();
        let group_mode = group_mode.clone();
        let search = search_text.clone();
        let sync_info = sync_date_browser_photo_info.clone();
        Rc::new(move || {
            if !active.replace(false) { return; }
            front.date_browser_changed();
            let active_photo_id = gallery.selected_photo_ids(None).first().copied();
            if let Some(snapshot) = snapshot.borrow_mut().take() {
                if gallery.restore_date_browser_snapshot(snapshot) {
                    let selected = active_photo_id
                        .filter(|photo_id| gallery.photo_objects().iter().any(|photo| photo.id() == *photo_id))
                        .into_iter()
                        .collect::<Vec<_>>();
                    // The calendar's temporary subset may have carried a
                    // multi-selection. Returning to All Photos keeps only its
                    // active photo, matching normal gallery focus behavior.
                    gallery.set_selected_photo_ids(&selected);
                }
            }
            if let Some(slot) = browser_slot.upgrade() {
                if let Some(browser) = slot.borrow().as_ref() { browser.root().set_visible(false); }
            }
            if let Some(button) = button.upgrade() { button.set_active(false); }
            scroll.set_visible(true);
            apply_gallery_grouping(&gallery, filter.get(), sort.get(), group_mode.get(), search.borrow().is_empty());
            sync_info();
        })
    };
    date_browser_exit_slot.replace(Some(exit_date_browser.clone()));
    let discard_date_browser: Rc<dyn Fn()> = {
        let front = front.clone();
        let snapshot = date_browser_snapshot.clone();
        let browser_slot = Rc::downgrade(&date_browser_slot);
        let active = date_browser_active.clone();
        let button = date_browser_button.downgrade();
        let scroll = gallery_scroll_stack.clone();
        Rc::new(move || {
            if !active.replace(false) { return; }
            front.date_browser_changed();
            snapshot.borrow_mut().take();
            if let Some(slot) = browser_slot.upgrade() {
                if let Some(browser) = slot.borrow().as_ref() { browser.root().set_visible(false); }
            }
            if let Some(button) = button.upgrade() { button.set_active(false); }
            scroll.set_visible(true);
        })
    };
    date_browser_discard_slot.replace(Some(discard_date_browser.clone()));
    {
        let exit = Rc::downgrade(&exit_date_browser);
        gallery.set_date_browser_invalidate_handler(move || {
            if let Some(exit) = exit.upgrade() { exit(); }
        });
    }
    let browser = date_browser::DateBrowser::new(
        {
            let slot = Rc::downgrade(&date_browser_slot);
            let snapshot = date_browser_snapshot.clone();
            let gallery = gallery.clone();
            let scroll = gallery_scroll_stack.clone();
            let sync_info = sync_date_browser_photo_info.clone();
            let exit = exit_date_browser.clone();
            Rc::new(move |month| {
                let indices = slot.upgrade().and_then(|slot| slot.borrow().as_ref().map(|browser| browser.indices(month)));
                let mut snapshot = snapshot.borrow_mut();
                if let (Some(indices), Some(snapshot)) = (indices, snapshot.as_mut()) {
                    if gallery.show_date_browser_subset(snapshot, &indices) {
                        scroll.set_visible(true);
                        sync_info();
                        return;
                    }
                }
                drop(snapshot);
                exit();
            })
        },
        {
            let scroll = gallery_scroll_stack.clone();
            let heading = gallery.group_header.clone();
            Rc::new(move || {
                scroll.set_visible(false);
                heading.set_visible(false);
            })
        },
        exit_date_browser.clone(),
    );
    browser.root().set_visible(false);
    grid_surface.prepend(browser.root());
    date_browser_slot.replace(Some(browser.clone()));
    {
        let front = Rc::downgrade(&front);
        browser.connect_overview_visible(move || {
            if let Some(front) = front.upgrade() {
                front.date_browser_changed();
            }
        });
    }
    {
        let browser = browser.clone();
        let snapshot = date_browser_snapshot.clone();
        let active = date_browser_active.clone();
        let gallery_for_resume = gallery.clone();
        let button = date_browser_button.downgrade();
        let front = front.clone();
        date_browser_resume_slot.replace(Some(Rc::new(move |bucket, photo_id| {
            let photos = gallery_for_resume.photo_objects();
            browser.set_photos(&photos);
            let Some(saved) = gallery_for_resume.date_browser_snapshot() else { return; };
            browser.set_tile_width(gallery_for_resume.current_zoom_width());
            snapshot.replace(Some(saved));
            active.set(true);
            front.date_browser_changed();
            if let Some(button) = button.upgrade() { button.set_active(true); }
            browser.root().set_visible(true);
            browser.resume_month(bucket);
            if let Some(photo_id) = photo_id { gallery_for_resume.select_photo(photo_id); }
        })));
    }
    {
        let active = date_browser_active.clone();
        let snapshot = date_browser_snapshot.clone();
        let gallery = gallery.clone();
        let browser_slot = date_browser_slot.clone();
        let filter = filter.clone();
        let search = search_text.clone();
        let exit = exit_date_browser.clone();
        let front = front.clone();
        date_browser_button.connect_clicked(move |button| {
            if active.get() { exit(); return; }
            if filter.get() != sidebar::SidebarFilter::All || !search.borrow().is_empty() {
                button.set_active(false);
                return;
            }
            if PENDING_REFRESH_GENERATION.load(std::sync::atomic::Ordering::Relaxed) != 0 {
                button.set_active(false);
                return;
            }
            let Some(saved) = gallery.date_browser_snapshot() else { button.set_active(false); return; };
            let Some(browser) = browser_slot.borrow().clone() else { button.set_active(false); return; };
            browser.set_photos(saved.photos());
            browser.set_tile_width(gallery.current_zoom_width());
            snapshot.replace(Some(saved));
            active.set(true);
            front.date_browser_changed();
            browser.root().set_visible(true);
            browser.show_years();
        });
    }

    let grid_overlay = gtk::Overlay::new();
    grid_overlay.set_hexpand(true);
    grid_overlay.set_vexpand(true);
    grid_overlay.set_child(Some(&grid_surface));
    grid_overlay.add_overlay(&scrub_date_label);

    // The context menu is a normal GtkOverlay child, so give it the
    // autohide behaviour GtkPopover used to provide. Any pointer press
    // outside the active menu dismisses it. Presses on menu buttons are
    // left alone so their normal clicked handlers still run.
    let context_menu_autohide = gtk::GestureClick::new();
    context_menu_autohide.set_button(0);
    context_menu_autohide.set_propagation_phase(gtk::PropagationPhase::Capture);
    let window_for_menu_autohide = window.clone();
    let lightbox_for_menu_autohide = lightbox.clone();
    window.connect_is_active_notify(move |window| {
        if !window.is_active() {
            dismiss_active_photo_context_menu();
        }
    });
    let lightbox_context_menu_click = gtk::GestureClick::new();
    lightbox_context_menu_click.set_button(1);
    lightbox_context_menu_click.set_propagation_phase(gtk::PropagationPhase::Capture);
    // The lightbox now owns its menu above the fullscreen photo. Capture
    // clicks outside that menu, but allow events inside to reach its buttons;
    // the old unconditional dismiss closed the menu before clicked handlers.
    let root_for_context_dismiss = lightbox.root.clone();
    lightbox_context_menu_click.connect_pressed(move |_, _, x, y| {
        let inside_menu = root_for_context_dismiss
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .as_ref()
            .is_some_and(photo_context_menu_contains);
        if !inside_menu {
            dismiss_active_photo_context_menu();
        }
    });
    lightbox.root.add_controller(lightbox_context_menu_click);
    context_menu_autohide.connect_pressed(move |_, _, x, y| {
        let picked = window_for_menu_autohide.pick(x, y, gtk::PickFlags::DEFAULT);
        let inside_menu = picked
            .as_ref()
            .is_some_and(photo_context_menu_contains);
        if !inside_menu && dismiss_active_photo_context_menu() {
            let root = lightbox_for_menu_autohide.root.clone();
            glib::idle_add_local_once(move || {
                if root.is_visible() {
                    root.grab_focus();
                }
            });
        }
    });
    window.add_controller(context_menu_autohide);

    let photo_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    photo_page.set_hexpand(true);
    photo_page.set_vexpand(true);
    photo_page.append(&grid_overlay);

    let album_theme_changed: Rc<dyn Fn()> = {
        let album_list = album_list.clone();
        Rc::new(move || album_list.changed(AlbumChange::Appearance))
    };
    // Clone for the theme engine: the original is captured by the layout.rs
    // include below.
    let theme_post_apply = album_theme_changed.clone();
    let albums_started = Instant::now();
    crate::startup_trace::mark("albums_widgets_start", None);
    let open_album: Rc<dyn Fn(i64)> = {
        let slot = album_home_click_slot.clone();
        Rc::new(move |album_id| {
            if let Some(open) = slot.borrow().as_ref() {
                open(album_id);
            }
        })
    };
    let albums_home = albums_view::build_deferred(
        connection.clone(),
        grid_thumbnail_size,
        open_album.clone(),
        album_theme_changed.clone(),
        {
            let present_settings = present_settings.clone();
            Rc::new(move || present_settings(Some("themes")))
        },
    );
    crate::startup_trace::mark("albums_widgets_done", Some(albums_started));
    if initial_filter == sidebar::SidebarFilter::Albums {
        let initialized = Rc::new(Cell::new(false));
        let populated = Rc::new(Cell::new(false));
        let populate_albums = {
            let album_list = album_list.clone();
            Rc::new(move || {
                if populated.replace(true) {
                    return;
                }
                let started = Instant::now();
                album_list.changed(AlbumChange::Shown);
                crate::startup_trace::mark("initial_albums_populated", Some(started));
            })
        };
        window.connect_map(move |window| {
            if initialized.replace(true) {
                return;
            }
            if let Some(surface) = window.surface() {
                let painted = Rc::new(Cell::new(false));
                let painted_once = painted.clone();
                let populate_albums = populate_albums.clone();
                surface.frame_clock().connect_after_paint(move |_| {
                    if painted_once.replace(true) {
                        return;
                    }
                    let populate_albums = populate_albums.clone();
                    glib::idle_add_local_once(move || {
                        populate_albums();
                    });
                });
            }
            // Keep the deferred first paint fast, but do not leave the Albums
            // page empty if this surface never emits after-paint.
            let populate_albums = populate_albums.clone();
            glib::timeout_add_local_once(Duration::from_millis(750), move || {
                populate_albums();
            });
        });
    }
    // Albums home is a separate ScrolledWindow, so it needs the same wheel
    // easing as the photo views. Ctrl+wheel must not zoom the hidden gallery.
    install_smooth_gallery_scroll(&albums_home, gallery.clone(), false);
    albums_view::connect_create_album(&albums_home, create_album.clone());
    // Hidden pages must not impose a width on the active gallery. In
    // particular, the Albums landing page has a wider introductory toolbar.
    main_stack.set_hhomogeneous(false);
    main_stack.set_hexpand(true);
    main_stack.set_vexpand(true);
    main_stack.add_named(&photo_page, Some("photos"));
    {
        let browser = browser.clone();
        let active = date_browser_active.clone();
        let front = front.clone();
        date_browser_back_slot.replace(Some(Rc::new(move || {
            if !active.get() || !matches!(front.current(), Front::Gallery | Front::DateOverview) {
                return false;
            }
            browser.back()
        })));
    }
    main_stack.add_named(&albums_home, Some("albums"));
    let collage_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    collage_page.set_hexpand(true);
    collage_page.set_vexpand(true);
    main_stack.add_named(&collage_page, Some("collage"));
    let edit_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    edit_page.set_hexpand(true);
    edit_page.set_vexpand(true);
    main_stack.add_named(&edit_page, Some("edit"));
    let library_home = crate::library_home::LibraryHome::new(
        db::active_database_path().expect("The library database is already open"),
        {
            let slot = library_navigation_slot.clone();
            Rc::new(move |destination| {
                if let Some(navigate) = slot.borrow().as_ref() {
                    navigate(destination);
                }
            })
        },
        {
            let lightbox = lightbox.clone();
            let connection = connection.clone();
            let open_edit = open_edit.clone();
            let open_collage = open_collage.clone();
            Rc::new(move |photos, index, edited| {
                let Some(photo) = photos.get(index) else {
                    return;
                };
                if edited {
                    let collage = db::collage_project(&connection.borrow(), photo.id())
                        .ok()
                        .flatten()
                        .is_some();
                    if collage {
                        open_collage(vec![photo.id()]);
                    } else {
                        open_edit(photo.id());
                    }
                } else {
                    lightbox.open(photos, index);
                }
            })
        },
        open_edit.clone(),
        scrapbook_export_photo,
    );
    crate::startup_trace::mark("library_home_widgets_done", Some(build_started));
    main_stack.add_named(&library_home.root, Some("library"));
    main_stack.set_visible_child_name(if initial_filter == sidebar::SidebarFilter::Library {
        "library"
    } else if initial_filter == sidebar::SidebarFilter::Albums {
        "albums"
    } else {
        "photos"
    });
    // The gallery Properties panel reserves width through a right-hand split;
    // only the fullscreen viewer and its own inspector are overlays.
    let page_overlay = gtk::Overlay::new();
    page_overlay.set_hexpand(true);
    page_overlay.set_vexpand(true);
    page_overlay.set_child(Some(&main_stack));
    // Freeze the previously presented gallery before a sidebar destination
    // replaces its model. GtkWidgetPaintable::current_image() is immutable:
    // unlike a live WidgetPaintable it cannot redraw the new model underneath
    // the outgoing image. Only one viewport-sized render node is retained.
    // Keep the WidgetPaintable attached across frames so current_image() has
    // the last GTK snapshot when a sidebar click arrives.
    let nav_fade_source = gtk::WidgetPaintable::new(Some(&main_stack));
    // During rapid supersession, freeze the actual blended viewport rather
    // than restoring the first transition's original picture to full opacity.
    let nav_fade_composite_source = gtk::WidgetPaintable::new(Some(&page_overlay));
    let nav_fade_sequence = Rc::new(Cell::new(0_u64));
    let nav_fade_overlay: Rc<RefCell<Option<gtk::Picture>>> =
        Rc::new(RefCell::new(None));
    // Non-gallery navigation (viewer/edit/collage/library switch) must never
    // leave a snapshot over the new surface. Invalidate its tick generation
    // before removing the GTK overlay.
    let nav_fade_cancel: Rc<dyn Fn()> = {
        let sequence = nav_fade_sequence.clone();
        let active = nav_fade_overlay.clone();
        let overlay = page_overlay.clone();
        Rc::new(move || {
            sequence.set(sequence.get().wrapping_add(1));
            if let Some(previous) = active.borrow_mut().take() {
                overlay.remove_overlay(&previous);
                crate::nav_transition_trace!("stage=crossfade_cancel");
            }
        })
    };
    let nav_fade_begin: Rc<dyn Fn(bool, bool, bool, bool)> = {
        let page_overlay = page_overlay.clone();
        let main_stack = main_stack.clone();
        let front = front.clone();
        let source = nav_fade_source.clone();
        let composite_source = nav_fade_composite_source.clone();
        let gallery = gallery.clone();
        let sequence = nav_fade_sequence.clone();
        let active = nav_fade_overlay.clone();
        let exact_photo_revealed = open_in_folder_revealed.clone();
        Rc::new(move |folder_destination, photo_destination, layout_destination, exact_photo_destination| {
            let next = sequence.get().wrapping_add(1);
            sequence.set(next);
            // Snapshot the full currently presented blend on a rapid
            // supersession. Reusing the original picture at full opacity would
            // visibly jump backward halfway through an earlier crossfade.
            let previous = active.borrow_mut().take();
            let frozen = if layout_destination {
                if let Some(ref previous) = previous {
                    let blended = composite_source.current_image();
                    if blended.intrinsic_width() > 1 && blended.intrinsic_height() > 1 {
                        Some(blended)
                    } else {
                        previous.paintable().map(|image| image.current_image())
                    }
                } else if main_stack.is_mapped()
                    && main_stack.width() > 1
                    && main_stack.height() > 1
                    && matches!(
                        front.current(),
                        Front::Gallery
                            | Front::DateOverview
                            | Front::Lightbox
                            | Front::LibraryHome
                            | Front::Albums
                    )
                {
                    Some(source.current_image())
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(previous) = previous {
                page_overlay.remove_overlay(&previous);
            }
            // GTK's animation setting also reflects the desktop's reduced-
            // motion preference. Do not create or retain transition pictures
            // when animations are disabled.
            if layout_destination
                && gtk::Settings::default().is_some_and(|settings| {
                    !settings.property::<bool>("gtk-enable-animations")
                })
            {
                crate::nav_transition_trace!("stage=crossfade_skip reason=animations_disabled");
                return;
            }
            let outgoing = if layout_destination {
                let Some(frozen) = frozen else {
                    return;
                };
                if frozen.intrinsic_width() <= 1 || frozen.intrinsic_height() <= 1 {
                    return;
                }
                gtk::Picture::for_paintable(&frozen)
            } else {
                // Keep the outgoing album or index from showing while its
                // replacement is loading. The solid texture uses PIC's active
                // theme surface color; Grid and Photo Wall rendering stay as-is.
                let background = crate::coverflow::lookup_theme_color(
                    main_stack.upcast_ref(),
                    "theme_bg_color",
                )
                    .or_else(|| {
                        crate::coverflow::lookup_theme_color(
                            main_stack.upcast_ref(),
                            "window_bg_color",
                        )
                    })
                    .unwrap_or_else(|| {
                        if adw::StyleManager::default().is_dark() {
                            gtk::gdk::RGBA::new(0.12, 0.12, 0.12, 1.0)
                        } else {
                            gtk::gdk::RGBA::new(0.97, 0.97, 0.97, 1.0)
                        }
                    });
                let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                let pixels = glib::Bytes::from_owned(vec![
                    channel(background.red()),
                    channel(background.green()),
                    channel(background.blue()),
                    255,
                ]);
                let texture = gtk::gdk::MemoryTexture::new(
                    1,
                    1,
                    gtk::gdk::MemoryFormat::R8g8b8a8,
                    &pixels,
                    4,
                );
                gtk::Picture::for_paintable(&texture)
            };
            outgoing.set_content_fit(gtk::ContentFit::Fill);
            outgoing.set_hexpand(true);
            outgoing.set_vexpand(true);
            outgoing.set_halign(gtk::Align::Fill);
            outgoing.set_valign(gtk::Align::Fill);
            outgoing.set_can_target(false);
            page_overlay.add_overlay(&outgoing);
            active.replace(Some(outgoing.clone()));

            let previous_model_generation = gallery.navigation_model_generation();
            let previous_layout_generation = gallery.layout_presentation_generation();
            let started = Instant::now();
            let fading = Rc::new(RefCell::new(None::<Instant>));
            let ready_frames = Cell::new(0_u8);
            let sequence_for_tick = sequence.clone();
            let page_for_tick = page_overlay.clone();
            let gallery_for_tick = gallery.clone();
            let active_for_tick = active.clone();
            let exact_photo_revealed_for_tick = exact_photo_revealed.clone();
            outgoing.add_tick_callback(move |image, _clock| {
                if sequence_for_tick.get() != next {
                    return glib::ControlFlow::Break;
                }
                let now = Instant::now();
                // An empty/unavailable destination or failed DB request must
                // not leave the old picture covering the application forever.
                // Normal navigation completes much sooner; this is only an
                // error-path safety net, not an animation delay.
                if fading.borrow().is_none() && started.elapsed() > Duration::from_secs(6) {
                    crate::nav_transition_trace!(
                        "stage=crossfade_timeout generation={next} folder={folder_destination} model={photo_destination} layout={layout_destination}"
                    );
                    page_for_tick.remove_overlay(image);
                    active_for_tick.replace(None);
                    return glib::ControlFlow::Break;
                }
                if fading.borrow().is_none() {
                    let model_ready = !photo_destination
                        || (gallery_for_tick.navigation_model_generation()
                            != previous_model_generation
                            && !gallery_for_tick.stream_building());
                    let folder_ready = !folder_destination
                        || (gallery_for_tick.visible_root().opacity() >= 0.99
                            && !gallery_for_tick.stream_building());
                    let layout_ready = !layout_destination
                        || gallery_for_tick.layout_presentation_ready(previous_layout_generation);
                    // Do not expose the new gallery after its model replacement
                    // but before Open in Folder has reached the exact photo.
                    let exact_ready = !exact_photo_destination || exact_photo_revealed_for_tick.get();
                    if !model_ready || !folder_ready || !layout_ready || !exact_ready {
                        ready_frames.set(0);
                        return glib::ControlFlow::Continue;
                    }
                    // The new model must survive a GTK frame before fading
                    // away the outgoing image. No thumbnail decode is awaited.
                    ready_frames.set(ready_frames.get().saturating_add(1));
                    if ready_frames.get() < 2 {
                        return glib::ControlFlow::Continue;
                    }
                    *fading.borrow_mut() = Some(now);
                    crate::nav_transition_trace!(
                        "stage=crossfade_start generation={next} preparation_ms={} folder={folder_destination} layout={layout_destination}",
                        started.elapsed().as_millis(),
                    );
                }
                let elapsed = fading.borrow().as_ref().unwrap().elapsed().as_secs_f64();
                let t = (elapsed / 0.160).clamp(0.0, 1.0);
                // Smoothstep: ease-in/ease-out, no layout or scroll animation.
                let eased = t * t * (3.0 - 2.0 * t);
                image.set_opacity(1.0 - eased);
                if t >= 1.0 {
                    page_for_tick.remove_overlay(image);
                    active_for_tick.replace(None);
                    crate::nav_transition_trace!(
                        "stage=crossfade_complete generation={next} total_ms={}",
                        started.elapsed().as_millis(),
                    );
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        })
    };
    // The layout selector changes the renderer IN PLACE, including Masonry
    // <-> Photo Wall where GtkStack never changes child. Reuse the same frozen
    // outgoing viewport, then wait for the new layout's anchor restoration
    // before fading it away. No model reload or thumbnail decode is needed.
    {
        let gallery_for_mode = gallery.clone();
        let connection_for_mode = connection.clone();
        let filter_for_mode = filter.clone();
        let fade = nav_fade_begin.clone();
        let cancel = nav_fade_cancel.clone();
        let front = front.clone();
        info.connect_photo_layout(move |mode| {
            if gallery_for_mode.layout() != mode {
                if front.current() == Front::Lightbox {
                    cancel();
                } else {
                    fade(false, false, true, false);
                }
                gallery_for_mode.set_layout(mode);
            }
            if let Err(error) = save_section_layout(
                &connection_for_mode.borrow(),
                filter_for_mode.get(),
                mode,
            ) {
                eprintln!("Could not save section photo layout: {error}");
            }
        });
    }
    // Edit and Collage are not sidebar destinations. An in-flight sidebar
    // fade must never cover either page if the user opens one immediately.
    {
        let cancel = nav_fade_cancel.clone();
        main_stack.connect_visible_child_notify(move |stack| {
            if matches!(stack.visible_child_name().as_deref(), Some("edit" | "collage")) {
                cancel();
            }
        });
    }
    let action_menu_autohide = gtk::GestureClick::new();
    action_menu_autohide.set_button(1);
    action_menu_autohide.set_propagation_phase(gtk::PropagationPhase::Capture);
    let info_for_action_menu_autohide = info.clone();
    action_menu_autohide.connect_pressed(move |_, _, _, _| {
        info_for_action_menu_autohide.dismiss_action_menus();
    });
    page_overlay.add_controller(action_menu_autohide);
    context_menu_host.borrow_mut().replace(page_overlay.downgrade());
    // Match the main left sidebar: a native AdwOverlaySplitView sliding in
    // from the RIGHT. Unlike a GtkOverlay panel it reserves real gallery
    // space, so not a single photo tile can be hidden behind Properties.
    // Keep this expanded; a compact window must still lay out the grid beside
    // the inspector rather than silently switching to overlay mode.
    let properties_split = adw::OverlaySplitView::new();
    properties_split.set_sidebar_position(gtk::PackType::End);
    properties_split.set_pin_sidebar(true);
    properties_split.set_collapsed(false);
    properties_split.set_enable_show_gesture(false);
    properties_split.set_enable_hide_gesture(false);
    properties_split.set_min_sidebar_width(280.0);
    properties_split.set_max_sidebar_width(520.0);
    properties_split.set_sidebar_width_fraction(session.properties_fraction);
    properties_split.set_content(Some(&page_overlay));

    let properties_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    properties_panel.set_width_request(280);
    properties_panel.set_hexpand(true);
    properties_panel.set_vexpand(true);
    properties_panel.set_overflow(gtk::Overflow::Hidden);
    properties_panel.add_css_class("background");
    properties_panel.add_css_class("scrapbook-inspector");
    // Never toggle child visibility: the split owns the slide animation.
    let properties_shell = gtk::Overlay::new();
    properties_shell.set_hexpand(true);
    properties_shell.set_vexpand(true);
    properties_shell.set_child(Some(&properties_panel));

    // Reuse the sidebar's capture-phase, split-coordinate resize pattern.
    // Pointer coordinates are converted to the stationary split, not the
    // moving handle, so resizing never oscillates at a fixed mouse position.
    let properties_resize_handle = gtk::Box::new(gtk::Orientation::Vertical, 0);
    properties_resize_handle.set_width_request(12);
    properties_resize_handle.set_halign(gtk::Align::Start);
    properties_resize_handle.set_valign(gtk::Align::Fill);
    properties_resize_handle.set_vexpand(true);
    properties_resize_handle.set_cursor_from_name(Some("col-resize"));
    properties_resize_handle.add_css_class("pic-properties-resize-handle");
    properties_shell.add_overlay(&properties_resize_handle);
    properties_split.set_sidebar(Some(&properties_shell));
    properties_split.set_show_sidebar(false);
    properties.attach_gallery(&properties_panel, &properties_split);

    let resize_origin = Rc::new(Cell::new(0.0_f64));
    let resize_width = Rc::new(Cell::new(0.0_f64));
    let resize_local = Rc::new(Cell::new((0.0_f64, 0.0_f64)));
    let resize_active = inspector_resize_active.clone();
    let resize_gesture = gtk::GestureDrag::new();
    resize_gesture.set_button(1);
    resize_gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    resize_gesture.set_exclusive(true);
    {
        let split = properties_split.clone();
        let gallery = gallery.clone();
        let panel = properties_panel.clone();
        let origin = resize_origin.clone();
        let width = resize_width.clone();
        let local = resize_local.clone();
        let active = resize_active.clone();
        let last_trace_width = inspector_trace_last_width.clone();
        let trace_frames = inspector_trace_frames.clone();
        resize_gesture.connect_drag_begin(move |gesture, x, y| {
            let Some(point) = gesture.widget().and_then(|handle| {
                handle.compute_point(&split, &gtk::graphene::Point::new(x as f32, y as f32))
            }) else {
                gesture.set_state(gtk::EventSequenceState::Denied);
                return;
            };
            gallery.cancel_resize_flip();
            gallery.set_photo_wall_width_frozen(false);
            active.set(true);
            last_trace_width.set(-1);
            trace_frames.set(0);
            if std::env::var_os("PIC_INSPECTOR_TRACE").is_some() {
                eprintln!(
                    "PIC_INSPECTOR_TRACE stage=drag_begin gallery_width={} panel_width={} scroll_y={:.1}",
                    split.content().map(|child| child.width()).unwrap_or(0),
                    panel.width(),
                    gallery.scroll_position()
                );
            }
            origin.set(f64::from(point.x()));
            width.set(panel.width().max(280) as f64);
            local.set((x, y));
        });
    }
    {
        let split = properties_split.clone();
        let origin = resize_origin.clone();
        let width = resize_width.clone();
        let local = resize_local.clone();
        let active = resize_active.clone();
        resize_gesture.connect_drag_update(move |gesture, dx, dy| {
            if !active.get() {
                return;
            }
            let (x, y) = local.get();
            let Some(point) = gesture.widget().and_then(|handle| {
                handle.compute_point(
                    &split,
                    &gtk::graphene::Point::new((x + dx) as f32, (y + dy) as f32),
                )
            }) else {
                return;
            };
            let total = f64::from(split.width().max(1));
            let target = (width.get() + origin.get() - f64::from(point.x()))
                .clamp(280.0, 520.0)
                .min(total * 0.55);
            split.set_sidebar_width_fraction((target / total).clamp(0.1, 0.7));
        });
    }
    {
        let active = resize_active.clone();
        let split = properties_split.clone();
        let gallery = gallery.clone();
        let connection = connection.clone();
        resize_gesture.connect_drag_end(move |_, _, _| {
            if !active.replace(false) {
                return;
            }
            if let Err(error) = record_session(
                &*connection.borrow(),
                SessionChange::PropertiesFraction(split.sidebar_width_fraction()),
            ) {
                eprintln!("Could not save Properties panel width: {error}");
            }
            if std::env::var_os("PIC_INSPECTOR_TRACE").is_some() {
                eprintln!(
                    "PIC_INSPECTOR_TRACE stage=drag_end sidebar_width={} gallery_width={} scroll_y={:.1}",
                    split.sidebar().map(|child| child.width()).unwrap_or(0),
                    split.content().map(|child| child.width()).unwrap_or(0),
                    gallery.scroll_position()
                );
            }
        });
    }
    {
        let active = resize_active.clone();
        resize_gesture.connect_cancel(move |_, _| active.set(false));
    }
    properties_resize_handle.add_controller(resize_gesture);

    // The Gallery's width observer and WidthSettleGate already handle
    // sidebar-style animated allocation. Freeze justified wall layout
    // during the slide, then consume only the final width/scroll anchor.
    // Do not move focus on reveal: grabbing the GridView triggers GTK's
    // implicit scroll-to-focused-tile and can visibly jump the viewport.
    {
        let gallery = gallery.clone();
        let active = inspector_transition_active.clone();
        let resize_active_for_transition = inspector_resize_active.clone();
        let gallery_scroll_for_settle = gallery_scroll_stack.clone();
        let last_width = inspector_trace_last_width.clone();
        let frames = inspector_trace_frames.clone();
        let generation = Rc::new(Cell::new(0_u64));
        properties_split.connect_show_sidebar_notify(move |split| {
            // Resize transitions are not photo zooms or model changes.
            gallery.cancel_resize_flip();
            let serial = generation.get().wrapping_add(1);
            generation.set(serial);
            active.set(true);
            last_width.set(-1);
            frames.set(0);
            let started = Instant::now();
            let start_scroll = gallery.scroll_position();
            let start_width = split.content().map(|child| child.width()).unwrap_or(0);
            let opening = split.shows_sidebar();
            // Capture BEFORE the first animated width change. The sectioned
            // layout's old photo/row offset is our stable visual reference.
            let grid_anchor = gallery.capture_inspector_grid_anchor();
            if std::env::var_os("PIC_INSPECTOR_TRACE").is_some() {
                eprintln!(
                    "PIC_INSPECTOR_TRACE stage=anchor_capture serial={} photo={:?} offset_y={:?} scroll_y={:.1}",
                    serial, grid_anchor.map(|anchor| anchor.0),
                    grid_anchor.map(|anchor| anchor.1), start_scroll
                );
            }
            gallery.set_photo_wall_width_frozen(true);
            if std::env::var_os("PIC_INSPECTOR_TRACE").is_some() {
                eprintln!(
                    "PIC_INSPECTOR_TRACE stage=begin serial={} open={} split_width={} gallery_width={} panel_width={} scroll_y={:.1} layout={:?} virtual={} collapsed={}",
                    serial,
                    opening,
                    split.width(),
                    start_width,
                    split.sidebar().map(|child| child.width()).unwrap_or(0),
                    start_scroll,
                    gallery.layout(),
                    gallery.using_sectioned_folder_view(),
                    split.is_collapsed()
                );
            }
            if gallery.layout() == grid::PhotoLayout::Honeycomb {
                let gallery = gallery.clone();
                let generation = generation.clone();
                let active = active.clone();
                let resize_active = resize_active_for_transition.clone();
                let last_size = Cell::new((0, 0));
                let stable_frames = Cell::new(0_u8);
                gallery_scroll_for_settle.add_tick_callback(move |surface, _| {
                    if generation.get() != serial {
                        return glib::ControlFlow::Break;
                    }
                    if resize_active.get() || gallery.layout() != grid::PhotoLayout::Honeycomb {
                        gallery.set_photo_wall_width_frozen(false);
                        active.set(false);
                        return glib::ControlFlow::Break;
                    }
                    let size = (surface.width(), gallery.honeycomb_viewport_width().unwrap_or(0));
                    if size.0 <= 100 || size.1 <= 100 || last_size.replace(size) != size {
                        stable_frames.set(0);
                    } else {
                        stable_frames.set(stable_frames.get().saturating_add(1));
                    }
                    if started.elapsed() < Duration::from_millis(400) || stable_frames.get() < 3 {
                        return glib::ControlFlow::Continue;
                    }
                    gallery.set_photo_wall_width_frozen(false);
                    active.set(false);
                    glib::ControlFlow::Break
                });
                return;
            }
            let gallery = gallery.clone();
            let split = split.clone();
            let scroll_surface = gallery_scroll_for_settle.clone();
            let generation = generation.clone();
            let active = active.clone();
            glib::timeout_add_local_once(Duration::from_millis(400), move || {
                if generation.get() != serial {
                    return;
                }
                gallery.set_photo_wall_width_frozen(false);
                if should_defer_inspector_folder_grid_reflow(
                    true,
                    gallery.layout() == grid::PhotoLayout::Grid,
                    gallery.using_sectioned_folder_view(),
                ) {
                    // One deterministic column reflow after the slide. The
                    // sectioned view's centre/header anchor preservation then
                    // runs once, rather than on each intermediate width.
                    let final_width = scroll_surface.width();
                    if final_width > 100 {
                        if std::env::var_os("PIC_INSPECTOR_TRACE").is_some() {
                            eprintln!(
                                "PIC_INSPECTOR_TRACE stage=grid_commit serial={} width={} scroll_before={:.1}",
                                serial, final_width, gallery.scroll_position()
                            );
                        }
                        gallery.update_width(final_width);
                        if let Some(anchor) = grid_anchor {
                            // Existing per-column reflow may have chosen a
                            // different centre row after GTK changed the
                            // viewport width. Restore our *original* photo.
                            let restored = gallery.restore_inspector_grid_anchor(anchor);
                            if std::env::var_os("PIC_INSPECTOR_TRACE").is_some() {
                                eprintln!(
                                    "PIC_INSPECTOR_TRACE stage=anchor_restore serial={} photo={} offset_y={:.1} success={} error_y={:?} scroll_y={:.1}",
                                    serial, anchor.0, anchor.1, restored,
                                    gallery.inspector_grid_anchor_error(anchor),
                                    gallery.scroll_position()
                                );
                            }
                            // The sectioned surface calculates its new geometry
                            // synchronously, but GtkScrolledWindow may not yet
                            // have published the matching adjustment.upper().
                            // A failed restore means "extent not ready", NOT
                            // that it is safe to abandon the captured photo.
                            // Retry for a few allocated frames; cancel stale
                            // retries if the panel is toggled again.
                            let gallery_after_layout = gallery.clone();
                            let generation_after_layout = generation.clone();
                            let mut tries: u8 = 0;
                            glib::timeout_add_local(Duration::from_millis(16), move || {
                                if generation_after_layout.get() != serial
                                    || gallery_after_layout.layout() != grid::PhotoLayout::Grid
                                    || !gallery_after_layout.using_sectioned_folder_view()
                                {
                                    return glib::ControlFlow::Break;
                                }
                                tries = tries.saturating_add(1);
                                let residual = gallery_after_layout
                                    .inspector_grid_anchor_error(anchor);
                                let corrected = if !restored
                                    || residual.is_some_and(|err| err.abs() > 2.0)
                                {
                                    gallery_after_layout.restore_inspector_grid_anchor(anchor)
                                } else {
                                    true
                                };
                                let after = gallery_after_layout.inspector_grid_anchor_error(anchor);
                                let complete = corrected
                                    && after.is_some_and(|err| err.abs() <= 2.0);
                                if std::env::var_os("PIC_INSPECTOR_TRACE").is_some()
                                    && (complete || tries >= 20 || tries == 1)
                                {
                                    eprintln!(
                                        "PIC_INSPECTOR_TRACE stage=anchor_settled serial={} photo={} attempt={} error_before={:?} corrected={} error_after={:?} scroll_y={:.1}",
                                        serial, anchor.0, tries, residual, corrected, after,
                                        gallery_after_layout.scroll_position()
                                    );
                                }
                                if complete || tries >= 20 {
                                    glib::ControlFlow::Break
                                } else {
                                    glib::ControlFlow::Continue
                                }
                            });
                        }
                        if std::env::var_os("PIC_INSPECTOR_TRACE").is_some() {
                            eprintln!(
                                "PIC_INSPECTOR_TRACE stage=grid_committed serial={} width={} scroll_after={:.1}",
                                serial, final_width, gallery.scroll_position()
                            );
                        }
                    }
                }
                active.set(false);
                if std::env::var_os("PIC_INSPECTOR_TRACE").is_some() {
                    let scroll = gallery.scroll_position();
                    let end_width = split.content().map(|child| child.width()).unwrap_or(0);
                    eprintln!(
                        "PIC_INSPECTOR_TRACE stage=end serial={} open={} elapsed_ms={} split_width={} gallery_width={} gallery_delta={} panel_width={} scroll_y={:.1} scroll_delta={:.1} collapsed={}",
                        serial,
                        opening,
                        started.elapsed().as_millis(),
                        split.width(),
                        end_width,
                        end_width - start_width,
                        split.sidebar().map(|child| child.width()).unwrap_or(0),
                        scroll,
                        scroll - start_scroll,
                        split.is_collapsed()
                    );
                }
            });
        });
    }

    let gallery_overlay = gtk::Overlay::new();
    gallery_overlay.set_hexpand(true);
    gallery_overlay.set_vexpand(true);
    gallery_overlay.set_child(Some(&properties_split));

    // The fullscreen viewer covers the actual split, and uses its own
    // independent overlay inspector without altering lightbox zoom or pan.
    gallery_overlay.add_overlay(&lightbox.root);
    let lightbox_properties_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    lightbox_properties_panel.set_width_request(300);
    lightbox_properties_panel.set_halign(gtk::Align::End);
    lightbox_properties_panel.set_valign(gtk::Align::Fill);
    lightbox_properties_panel.set_vexpand(true);
    lightbox_properties_panel.set_overflow(gtk::Overflow::Hidden);
    lightbox_properties_panel.add_css_class("background");
    lightbox_properties_panel.add_css_class("scrapbook-inspector");
    lightbox_properties_panel.set_visible(false);
    properties.attach_lightbox(&lightbox_properties_panel);

    // Index pages and full-surface workspaces are not photo grids: close
    // the inspector on Library Home, Albums Home, Edit and Collage.
    // Folder-to-folder navigation intentionally keeps it open.
    // Only photo-gallery pages can own a Properties pane. Library/Albums
    // index, Edit and Collage cannot reopen it through stale InfoBar selection.
    // (Kept in step by FrontMirrors; see front_mirrors.rs.)

    content.append(&gallery_overlay);

    // One snapshot after the native split animation settles; unlike the
    // previous overlay the gallery must occupy only the remaining width.
    {
        let gallery_for_trace = gallery.clone();
        let page_for_trace = page_overlay.clone();
        let panel_for_trace = properties_panel.clone();
        properties_split.connect_show_sidebar_notify(move |split| {
            if std::env::var_os("PIC_INSPECTOR_TRACE").is_none() {
                return;
            }
            let opened = split.shows_sidebar();
            let gallery = gallery_for_trace.clone();
            let page = page_for_trace.clone();
            let panel = panel_for_trace.clone();
            glib::timeout_add_local_once(Duration::from_millis(420), move || {
                eprintln!(
                    "PIC_INSPECTOR_TRACE stage=settled open={} gallery_width={} panel_width={} scroll_y={:.1}",
                    opened,
                    page.width(),
                    panel.width(),
                    gallery.scroll_position()
                );
            });
        });
    }

    // Low-volume, opt-in diagnostics for lightbox close/return geometry.
    // Logs capture both transition endpoints and the settled viewport.
    {
        let gallery_for_trace = gallery.clone();
        let gallery_content_for_trace = page_overlay.clone();
        let inspector_for_trace = properties_panel.clone();
        let split_for_trace = properties_split.clone();
        let lightbox_inspector_for_trace = lightbox_properties_panel.clone();
        let overlay_for_trace = gallery_overlay.clone();
        lightbox.root.connect_visible_notify(move |root| {
            if std::env::var_os("PICASA_TRACE").is_none() {
                return;
            }
            let stage = if root.is_visible() { "opened" } else { "hidden" };
            eprintln!(
                "PIC_LIGHTBOX_LAYOUT stage={} overlay_width={} gallery_width={} inspector_width={} inspector_visible={} viewer_inspector_visible={} scroll_y={:.1}",
                stage,
                overlay_for_trace.width(),
                gallery_content_for_trace.width(),
                inspector_for_trace.width(),
                split_for_trace.shows_sidebar(),
                lightbox_inspector_for_trace.is_visible(),
                gallery_for_trace.scroll_position()
            );
            if !root.is_visible() {
                let gallery = gallery_for_trace.clone();
                let gallery_content = gallery_content_for_trace.clone();
                let inspector = inspector_for_trace.clone();
                let split = split_for_trace.clone();
                glib::timeout_add_local_once(Duration::from_millis(220), move || {
                    eprintln!(
                        "PIC_LIGHTBOX_LAYOUT stage=settled gallery_width={} inspector_width={} inspector_visible={} scroll_y={:.1}",
                        gallery_content.width(),
                        inspector.width(),
                        split.shows_sidebar(),
                        gallery.scroll_position()
                    );
                });
            }
        });
    }
    {
        let filter = filter.clone();
        let search_text = search_text.clone();
        let collage_add_mode = collage_add_mode.clone();
        let lightbox = lightbox.clone();
        main_stack.connect_visible_child_notify(move |stack| {
            // A viewer handing off to the editor fades out over it on its own.
            if !matches!(stack.visible_child_name().as_deref(), Some("photos" | "library"))
                && !lightbox.is_handing_off()
            {
                lightbox.close();
            }
            if filter.get() == sidebar::SidebarFilter::Library
                && search_text.borrow().is_empty()
                && !collage_add_mode.get()
                && stack.visible_child_name().as_deref() == Some("photos")
            {
                stack.set_visible_child_name("library");
            }
        });
    }

    let collage_editor: Rc<RefCell<Option<crate::collage::CollageEditor>>> =
        Rc::new(RefCell::new(None));
    {
        let parent = window.clone().upcast::<gtk::Window>();
        let connection = connection.clone();
        let main_stack = main_stack.clone();
        let collage_page = collage_page.clone();
        let collage_editor = collage_editor.clone();
        let collage_add_mode_slot = collage_add_mode_slot.clone();
        let collage_close_slot = collage_close_slot.clone();
        let collage_add_mode = collage_add_mode.clone();
        // Opens the photo editor while marking the trip as collage-owned so
        // the collage editor survives and Back/Done return to the collage.
        let open_edit_from_collage = {
            let open_edit = open_edit.clone();
            let collage_editing = collage_editing.clone();
            Rc::new(move |id: i64| {
                collage_editing.set(true);
                open_edit(id);
            }) as Rc<dyn Fn(i64)>
        };
        collage_open_slot.replace(Some(Rc::new(move |ids| {
            let add_mode_slot = collage_add_mode_slot.clone();
            let close_slot = collage_close_slot.clone();
            let collage_add_mode = collage_add_mode.clone();
            crate::collage::open(
                &parent,
                connection.clone(),
                ids,
                Rc::new({
                    let parent = parent.clone();
                    let main_stack = main_stack.clone();
                    let collage_page = collage_page.clone();
                    let collage_editor = collage_editor.clone();
                    let collage_add_mode = collage_add_mode.clone();
                    let open_edit_from_collage = open_edit_from_collage.clone();
                    move |photos, draft| {
                        while let Some(child) = collage_page.first_child() {
                            collage_page.remove(&child);
                        }
                        // A fresh editor starts with no add-photos detour
                        // pending; clear any stale picking state so leaving
                        // the collage page tears it down normally.
                        collage_add_mode.set(false);
                        let add_photos = {
                            let slot = add_mode_slot.clone();
                            Rc::new(move || {
                                if let Some(add_mode) = slot.borrow().as_ref() {
                                    add_mode();
                                }
                            }) as Rc<dyn Fn()>
                        };
                        let close = {
                            let slot = close_slot.clone();
                            Rc::new(move || {
                                if let Some(close) = slot.borrow().as_ref() {
                                    close();
                                }
                            }) as Rc<dyn Fn()>
                        };
                        let editor = crate::collage::build_editor(
                            &parent,
                            photos,
                            add_photos,
                            close,
                            draft,
                            open_edit_from_collage.clone(),
                        );
                        collage_page.append(&editor.root);
                        collage_editor.replace(Some(editor));
                        main_stack.set_visible_child_name("collage");
                    }
                }),
            );
        })));
    }

    let edit_editor: Rc<RefCell<Option<crate::edit::EditEditor>>> = Rc::new(RefCell::new(None));
    let smart_edit_context = action_context.clone();
    // InfoBar Draw: open the editor straight on its Draw tab. In the editor
    // it switches to Draw, or closes like Edit when Draw is already showing.
    {
        let edit_editor = edit_editor.clone();
        let front = front.clone();
        let open_edit = open_edit.clone();
        let info_weak = Rc::downgrade(&info);
        info.draw.connect_clicked(move |_| {
            // Collage photos use the collage editor's own Edit Photo action.
            if front.current() == Front::Collage {
                return;
            }
            let Some(info) = info_weak.upgrade() else { return };
            let show_draw = || {
                if let Ok(editor) = edit_editor.try_borrow() {
                    if let Some(editor) = editor.as_ref() {
                        editor.show_draw();
                    }
                }
            };
            if front.current() == Front::Editor {
                let draw_showing = edit_editor
                    .try_borrow()
                    .ok()
                    .and_then(|editor| editor.as_ref().map(|editor| editor.draw_is_active()))
                    .unwrap_or(false);
                if draw_showing {
                    info.edit.emit_clicked();
                } else {
                    show_draw();
                }
                return;
            }
            // Copy the id out first: open_edit() changes the stack
            // synchronously and may trigger InfoBar events.
            let photo_id = info.editable_photo().map(|photo| photo.id());
            if let Some(id) = photo_id {
                open_edit(id);
                show_draw();
            }
        });
    }
    // Catch-all teardown when the collage/edit pages are left by any path
    // (sidebar navigation, search, opening the editor mid-collage, ...).
    // GtkStack reports the maximum minimum size of all its pages, hidden
    // ones included, so an editor left attached keeps demanding its tall
    // minimum and pushes the shared bottom bar offscreen on short windows.
    // The Exit/Done handlers tear down directly; this runs idempotently for
    // every other transition. The collage editor survives transitions to
    // the photos page while the user is picking images to add.
    {
        let main_stack_for_teardown = main_stack.clone();
        let collage_page = collage_page.clone();
        let collage_editor = collage_editor.clone();
        let edit_page = edit_page.clone();
        let edit_editor = edit_editor.clone();
        let edit_escape_slot = edit_escape_slot.clone();
        let collage_add_mode = collage_add_mode.clone();
        let collage_editing = collage_editing.clone();
        let gallery = gallery.clone();
        let connection_for_teardown = connection.clone();
        let filter_for_history = filter.clone();
        let search_for_history = search_text.clone();
        let sort_for_history = sort.clone();
        main_stack_for_teardown.connect_visible_child_notify(move |stack| {
            let visible = stack.visible_child_name();
            if visible.as_deref() == Some("photos")
                && (filter_for_history.get() == sidebar::SidebarFilter::History
                    || !db::filters::Filters::load(&connection_for_teardown.borrow()).is_empty())
            {
                refresh_grid(
                    &connection_for_teardown,
                    filter_for_history.get(),
                    &search_for_history.borrow(),
                    sort_for_history.get(),
                    &gallery,
                );
            }
            if visible.as_deref() != Some("collage")
                && !collage_add_mode.get()
                && !collage_editing.get()
            {
                // try_borrow: the add-photos handler holds the editor borrow
                // while switching pages; skip rather than panic.
                if collage_editor.try_borrow().map(|editor| editor.is_some()).unwrap_or(false) {
                    // Persist the draft on every path out of the editor.
                    if let Ok(editor_handle) = collage_editor.try_borrow() {
                        if let Some(editor) = editor_handle.as_ref() {
                            let json = editor.draft_json();
                            let guard = connection_for_teardown.borrow();
                            let _ =
                                db::set_setting(&guard, crate::collage::DRAFT_SETTING_KEY, &json);
                        }
                    }
                    // Clear the grid selection so the next toolbar collage
                    // click is a blank start and can offer the resume prompt.
                    gallery.set_selected_photo_ids(&[]);
                    while let Some(child) = collage_page.first_child() {
                        collage_page.remove(&child);
                    }
                    collage_editor.replace(None);
                }
            }
            if visible.as_deref() != Some("edit")
                && edit_editor.try_borrow().map(|editor| editor.is_some()).unwrap_or(false)
            {
                edit_escape_slot.borrow_mut().take();
                // Any exit from the photo editor ends the collage detour.
                collage_editing.set(false);
                detach_photo_editor(stack, &edit_page, &edit_editor);
            }
        });
    }
    {
        let parent = window.clone().upcast::<gtk::Window>();
        let connection = connection.clone();
        let main_stack = main_stack.clone();
        let front = front.clone();
        let zoom_slider = zoom_slider.clone();
        let edit_page = edit_page.clone();
        let edit_editor = edit_editor.clone();
        let gallery = gallery.clone();
        let info = info.clone();
        let lightbox = lightbox.clone();
        let edit_space_slot = edit_space_slot.clone();
        let edit_escape_slot = edit_escape_slot.clone();
        let collage_editing = collage_editing.clone();
        let collage_editor = collage_editor.clone();
        let lightbox_properties_panel = lightbox_properties_panel.clone();
        // Bumped by every editor open and close, so a page switch still
        // waiting for an earlier editor's preview gives up.
        let edit_open_serial = Rc::new(Cell::new(0_u64));
        // Keep the shared slider synchronization handle available to the rest
        // of build.rs. The editor-open move closure owns only this cheap clone.
        // Edit-mode Export uses the exact same dialog → destination → progress
        // pipeline as the info-bar Export button (size + file type + Stop bar).
        let export_for_editor: Rc<dyn Fn(Vec<crate::edit::export_batch::ExportJob>)> = {
            let window = window.clone().upcast::<gtk::Window>();
            let progress = operation_progress.clone();
            Rc::new(move |jobs| {
                if jobs.is_empty() {
                    return;
                }
                show_photo_export_dialog(&window, jobs, progress.clone());
            })
        };
        edit_open_slot.replace(Some(Rc::new(move |id| {
            // A photo outside the library has no row: it is the viewer's
            // photo, and closing the editor returns to that viewer.
            let outside_viewer = (id < 0).then(|| lightbox.photos());
            let photo = match &outside_viewer {
                Some(viewer) => viewer.iter().find(|photo| photo.id() == id).cloned().inspect(|photo| {
                    open_files::load_outside_metadata(photo);
                }),
                None => db::photo(&connection.borrow(), id)
                    .ok()
                    .flatten()
                    .map(|photo| crate::photo_object::PhotoObject::from_photo(&photo)),
            };
            let Some(photo) = photo else {
                // The open failed; drop any pending collage-edit marker so
                // the flag cannot get stuck and spare the collage forever.
                collage_editing.set(false);
                return;
            };
            edit_space_slot.borrow_mut().take();
            edit_escape_slot.borrow_mut().take();
            let open_serial = edit_open_serial.get().wrapping_add(1);
            edit_open_serial.set(open_serial);
            // Only a photo opened from the Library index returns to that
            // index. Photos, Folders and Albums keep their existing route.
            let opened_from_library =
                main_stack.visible_child_name().as_deref() == Some("library");
            // The edited image already on screen (the viewer's frame, else the
            // gallery tile) stands in on the editor canvas until the editor's
            // own first frame is ready, so both fades can start at once.
            let placeholder = lightbox.displayed_paintable(id).or_else(|| {
                gallery
                    .transition_source_for_photo(id)
                    .map(|(_, paintable)| paintable)
                    .filter(|paintable| !paintable.is::<gtk::WidgetPaintable>())
            });
            // Opened from the viewer: its last frame stays over the editor
            // until the editor shows the photo, then fades out, instead of
            // cutting to an empty canvas. Elsewhere this closes at once.
            {
                let edit_editor = edit_editor.clone();
                lightbox.fade_out_to(
                    &[lightbox_properties_panel.clone().upcast()],
                    move || {
                        edit_editor.try_borrow().is_ok_and(|editor| {
                            editor
                                .as_ref()
                                .is_some_and(|editor| {
                                    editor.photo_id() == id
                                        && (editor.first_frame_shown() || editor.has_placeholder())
                                })
                        })
                    },
                );
            }
            let library_scroll_y = gallery.scroll_position();
            while let Some(child) = edit_page.first_child() {
                edit_page.remove(&child);
            }
            let trace_uri = photo.path();
            // Save Copy reports the new file; the viewer then opens on it.
            let saved_copy: Rc<RefCell<Option<std::path::PathBuf>>> = Rc::new(RefCell::new(None));
            crate::thumbnail_trace::event(&trace_uri, "editor", "opened");
            info.one_to_one.set_active(false);
            let close = {
                let main_stack = main_stack.clone();
                let one_to_one = info.one_to_one.clone();
                let gallery = gallery.clone();
                let edit_space_slot = edit_space_slot.clone();
                let edit_escape_slot = edit_escape_slot.clone();
                let edit_page = edit_page.clone();
                let edit_editor = edit_editor.clone();
                let collage_editing = collage_editing.clone();
                let collage_editor = collage_editor.clone();
                let connection = connection.clone();
                let trace_uri = trace_uri.clone();
                let lightbox = lightbox.clone();
                let outside_viewer = outside_viewer.clone();
                let saved_copy = saved_copy.clone();
                let edit_open_serial = edit_open_serial.clone();
                Rc::new(move || {
                    crate::thumbnail_trace::event(&trace_uri, "editor", "closed");
                    edit_open_serial.set(edit_open_serial.get().wrapping_add(1));
                    edit_space_slot.borrow_mut().take();
                    edit_escape_slot.borrow_mut().take();
                    one_to_one.set_active(false);
                    let returning_to_collage = collage_editing.get();
                    if let Some(viewer) = outside_viewer.clone() {
                        main_stack.set_visible_child_name("photos");
                        let (photos, index) =
                            open_files::viewer_after_edit(viewer, id, saved_copy.borrow_mut().take());
                        lightbox.open(photos, index);
                    } else if returning_to_collage {
                        // Bring the collage tile up to date with any edits
                        // saved during this detour.
                        if let Ok(editor_handle) = collage_editor.try_borrow() {
                            if let Some(collage) = editor_handle.as_ref() {
                                if let Some(updated) =
                                    db::photo(&connection.borrow(), id).ok().flatten()
                                {
                                    collage.refresh_photo_metadata(&[
                                        crate::photo_object::PhotoObject::from_photo(&updated),
                                    ]);
                                }
                            }
                        }
                        leave_photo_editor(&main_stack, "collage");
                    } else if opened_from_library {
                        leave_photo_editor(&main_stack, "library");
                    } else {
                        leave_photo_editor(&main_stack, "photos");
                        gallery.restore_view(id, library_scroll_y);
                    }
                    // Detach the editor so it stops contributing to the
                    // stack's size request and its preview memory is
                    // released; the editor is rebuilt on the next open.
                    detach_photo_editor(&main_stack, &edit_page, &edit_editor);
                }) as Rc<dyn Fn()>
            };
            let saved = {
                let trace_uri = trace_uri.clone();
                let change_context = smart_edit_context.clone();
                Rc::new(move |photo: crate::photo_object::PhotoObject| {
                    crate::thumbnail_trace::event(&trace_uri, "editor", "saved");
                    photo_changed(
                        &change_context,
                        PhotoChange::Recipes {
                            updates: &[(photo.id(), photo.edit_recipe())],
                            more_coming: false,
                        },
                    );
                }) as Rc<dyn Fn(crate::photo_object::PhotoObject)>
            };
            let editor = crate::edit::build_editor(
                &parent,
                connection.clone(),
                photo,
                close,
                saved,
                export_for_editor.clone(),
            );
            if let Some(placeholder) = &placeholder {
                editor.show_placeholder(placeholder);
            }
            {
                let one_to_one = info.one_to_one.clone();
                editor.set_one_to_one_sync_handler(move |enabled| {
                    one_to_one.set_active(enabled);
                });
            }
            {
                let saved_copy = saved_copy.clone();
                editor.set_copy_saved_handler(move |path| {
                    saved_copy.replace(Some(path));
                });
            }
            {
                // The slider switches to Edit zoom when the edit page shows.
                let zoom_slider = zoom_slider.clone();
                editor.set_zoom_sync_handler(move |native_scale, fit_scale| {
                    zoom_slider.show_edit_zoom(native_scale, fit_scale);
                });
            }
            // Make the way back explicit when the editor was opened from a
            // collage tile; Done saves and follows the same return path.
            if collage_editing.get() {
                editor.set_back_label("Back to Collage", "Return to the collage");
            } else if opened_from_library {
                editor.set_back_label("Back to Library", "Return to the Library index");
            }
            edit_escape_slot.replace(Some(editor.escape_action_handle()));
            edit_page.append(&editor.root);
            {
                let text_toggle = editor.text_toggle_handle();
                edit_editor.replace(Some(editor));
                let front = front.clone();
                let one_to_one = info.one_to_one.clone();
                let edit_space_slot = edit_space_slot.clone();
                edit_space_slot.replace(Some(Rc::new(move || {
                    if front.current() != Front::Editor {
                        // Editor left via the Edit toggle (slot not cleared);
                        // let Space fall through to lightbox/grid open.
                        return false;
                    }
                    if !text_toggle.is_active() {
                        one_to_one.set_active(!one_to_one.is_active());
                    }
                    true
                }) as Rc<dyn Fn() -> bool>));
            }
            if lightbox.is_handing_off()
                || !main_stack.is_mapped()
                || !main_stack.settings().is_gtk_enable_animations()
            {
                // The fading viewer covers the editor while it loads.
                main_stack.set_visible_child_name("edit");
                return;
            }
            // From a page: keep it on screen until the editor shows the photo
            // (its stand-in, else its preview, up to the viewer handoff's
            // limit), then crossfade over EDITOR_FADE_MS, instead of cutting
            // to an empty canvas.
            let edit_editor = edit_editor.clone();
            let edit_open_serial = edit_open_serial.clone();
            let requested = std::time::Instant::now();
            main_stack.add_tick_callback(move |stack, _| {
                if edit_open_serial.get() != open_serial {
                    return glib::ControlFlow::Break;
                }
                let placeholder_shown = edit_editor
                    .try_borrow()
                    .is_ok_and(|editor| editor.as_ref().is_some_and(|editor| editor.has_placeholder()));
                let ready = edit_editor.try_borrow().is_ok_and(|editor| {
                    editor
                        .as_ref()
                        .is_some_and(|editor| {
                            editor.photo_id() == id
                                && (editor.preview_ready() || editor.has_placeholder())
                        })
                });
                let waited = requested.elapsed();
                if !ready && waited < crate::lightbox::HANDOFF_WAIT_LIMIT {
                    return glib::ControlFlow::Continue;
                }
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_EDIT_OPEN stage=crossfade waited_ms={:.1} ready={} placeholder={}",
                        waited.as_secs_f64() * 1000.0,
                        ready,
                        placeholder_shown
                    );
                }
                stack.set_transition_duration(crate::lightbox::EDITOR_FADE_MS);
                stack.set_visible_child_full("edit", gtk::StackTransitionType::Crossfade);
                glib::ControlFlow::Break
            });
        })));
    }

    // Photo mark commands decide the targets, values and refreshes; the
    // controls only report what the user asked for.
    let marks_for_favorite = action_context.clone();
    info.favorite.connect_clicked(move |_| {
        let Some(photo) = marks_for_favorite.info.current_photo() else {
            return;
        };
        photo_mark_commands::run(&marks_for_favorite, MarkIntent::ToggleFavorite, &photo);
    });

    for (index, button) in info.rating_buttons.iter().enumerate() {
        let marks_for_rating = action_context.clone();
        let stars = (index + 1) as u8;
        button.connect_clicked(move |_| {
            let Some(photo) = marks_for_rating.info.current_photo() else {
                return;
            };
            photo_mark_commands::run(&marks_for_rating, MarkIntent::ToggleRating(stars), &photo);
        });
    }

    let lightbox_for_one_to_one = lightbox.clone();
    {
        // Keep the toolbar 1:1 toggle in sync when the lightbox changes mode
        // itself (e.g. Ctrl+wheel leaves 1:1 for a manual zoom).
        let one_to_one_button = info.one_to_one.clone();
        lightbox.set_one_to_one_sync_handler(move |enabled| {
            one_to_one_button.set_active(enabled);
        });
    }
    let edit_editor_for_one_to_one = edit_editor.clone();
    let front_for_one_to_one = front.clone();
    let space_toggle_in_progress_for_toggle = space_toggle_in_progress.clone();
    let space_open_slot_for_one_to_one = space_open_slot.clone();
    info.one_to_one.connect_toggled(move |button| {
        let enabled = button.is_active();
        match front_for_one_to_one.current() {
            Front::Editor => {
                if let Some(editor) = edit_editor_for_one_to_one.borrow().as_ref() {
                    editor.set_one_to_one(enabled);
                }
            }
            Front::Lightbox => lightbox_for_one_to_one.set_one_to_one(enabled),
            _ => {
                if enabled {
                    // The grid toolbar 1:1 control follows Space: first open the
                    // selected photo in Fit, then toggle native pixels on the next
                    // press while the viewer is visible.
                    if let Some(open_selected) = space_open_slot_for_one_to_one.borrow().as_ref() {
                        open_selected();
                    }
                    // Reset even if no photo could be opened (e.g. no selection).
                    // The photo-changed callback may already have reset it.
                    if button.is_active() {
                        button.set_active(false);
                    }
                }
            }
        }
        if space_toggle_in_progress_for_toggle.get() {
            space_toggle_in_progress_for_toggle.set(false);
        }
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_ZOOM_UI one_to_one enabled={} lightbox_visible={} edit_visible={}",
                button.is_active(),
                front_for_one_to_one.current() == Front::Lightbox,
                front_for_one_to_one.current() == Front::Editor,
            );
        }
    });

    // Prepare the correct grid selection behind an animated close. This lets
    // GTK allocate and paint the destination while the lightbox still covers it.
    // Programmatic closes and pending search refreshes keep the old safe path.
    let lightbox_close_prepared = Rc::new(Cell::new(false));
    {
        let gallery = gallery.clone();
        let lightbox_for_prepare = Rc::downgrade(&lightbox);
        let prepared = lightbox_close_prepared.clone();
        let folder_on_close = search_result_folder_on_close.clone();
        let close_waiting = search_close_waiting.clone();
        let preserve_viewer = preserve_lightbox_during_search_return.clone();
        let navigate_to_folder = navigate_to_folder.clone();
        let inspector_for_prepare = properties_panel.clone();
        let gallery_for_prepare = gallery_overlay.clone();
        lightbox.set_close_prepare_handler(move || {
            prepared.set(false);
            if let Some(photo) = lightbox_for_prepare
                .upgrade()
                .and_then(|lightbox| lightbox.current_photo())
            {
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_LIGHTBOX_LAYOUT stage=prepare photo={} gallery_width={} inspector_width={} inspector_visible={} scroll_y={:.1}",
                        photo.id(),
                        gallery_for_prepare.width(),
                        inspector_for_prepare.width(),
                        inspector_for_prepare.is_visible(),
                        gallery.scroll_position()
                    );
                }
                if folder_on_close.replace(false) && photo.folder_id() > 0 {
                    if std::env::var_os("PICASA_TRACE").is_some() {
                        eprintln!(
                            "PIC_NAV search_close_prepare folder={} photo={}",
                            photo.folder_id(),
                            photo.id()
                        );
                    }
                    close_waiting.set(true);
                    preserve_viewer.set(true);
                    navigate_to_folder(photo.folder_id(), photo.id());
                    preserve_viewer.set(false);
                    prepared.set(true);
                    return;
                }
                let restored = gallery.restore_activated_photo(photo.id());
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_LIGHTBOX_LAYOUT stage=restore_requested photo={} found={} scroll_y={:.1}",
                        photo.id(),
                        restored,
                        gallery.scroll_position()
                    );
                }
                prepared.set(restored);
            }
        });
    }
    {
        let close_waiting = search_close_waiting.clone();
        lightbox.set_close_ready_handler(move || !close_waiting.get());
    }

    // Search results return to the current photo's folder on close. Other
    // viewer closes restore the shared gallery selection and viewport.
    let one_to_one_for_visibility = info.one_to_one.clone();
    let gallery_for_lightbox_close = gallery.clone();
    let lightbox_for_lightbox_close = lightbox.clone();
    let properties_for_lightbox_close = properties.clone();
    let search_entry_for_lightbox_close = search_entry_slot.clone();
    let search_close_waiting_for_lightbox_close = search_close_waiting.clone();
    let filter_for_lightbox_close = filter.clone();
    let search_text_for_lightbox_close = search_text.clone();
    let lightbox_return_generation = Rc::new(Cell::new(0_u64));
    lightbox.root.connect_visible_notify(move |root| {
        let return_generation = lightbox_return_generation.get().wrapping_add(1);
        lightbox_return_generation.set(return_generation);
        if root.is_visible() {
            lightbox_close_prepared.set(false);
            search_close_waiting_for_lightbox_close.set(false);
            // A previous Lightbox return may still be easing the Folder view.
            // Once the viewer opens, freeze the hidden gallery exactly where it
            // is so no stale scroll callback can keep moving it underneath us.
            gallery_for_lightbox_close.cancel_sectioned_folder_scroll_animation();
            return;
        }

        one_to_one_for_visibility.set_active(false);
        // Selection may have been restored while the lightbox still covered
        // the grid. Keep an already-open gallery inspector on the returned
        // photo rather than leaving the metadata from the pre-view selection.
        if let Some(photo) = lightbox_for_lightbox_close.current_photo() {
            properties_for_lightbox_close.show_photo(PropertiesSurface::Gallery, Some(&photo));
        }
        if lightbox_close_prepared.replace(false) {
            // Selection, viewport and tile focus were already staged during
            // the fade; repeating the restore would create a second movement.
            return;
        }

        let gallery = gallery_for_lightbox_close.clone();
        let selected_photo = lightbox_for_lightbox_close.current_photo();
        let search_entry = search_entry_for_lightbox_close.clone();
        let filter = filter_for_lightbox_close.clone();
        let filter_at_close = filter.get();
        let search_text = search_text_for_lightbox_close.clone();
        let query_at_close = search_text.borrow().clone();
        let return_generation_cell = lightbox_return_generation.clone();
        let root = root.downgrade();
        glib::timeout_add_local(Duration::from_millis(25), move || {
            if return_generation_cell.get() != return_generation
                || root.upgrade().is_none_or(|root| root.is_visible())
                || filter.get() != filter_at_close
                || *search_text.borrow() != query_at_close
            {
                return glib::ControlFlow::Break;
            }
            // Keep focus in the entry if the viewer was closed while searching.
            if search_entry
                .borrow()
                .as_ref()
                .is_some_and(|entry| entry.has_focus())
            {
                return glib::ControlFlow::Break;
            }
            let restoration_scheduled = selected_photo
                .as_ref()
                .is_some_and(|photo| gallery.restore_activated_photo(photo.id()));
            // The virtual gallery's restore callback focuses the selected
            // tile after its destination layout is allocated. Focusing the
            // gallery container here makes GTK choose its first tile and
            // scroll toward it, which can race and overwrite the return.
            if !restoration_scheduled || !gallery.using_virtual_photo_surface() {
                gallery.grab_focus();
            }
            glib::ControlFlow::Break
        });
    });


    let gallery_for_zoom_slider = gallery.clone();
    let date_browser_for_zoom_slider = date_browser_slot.clone();
    let lightbox_for_zoom_slider = lightbox.clone();
    let edit_editor_for_zoom_slider = edit_editor.clone();
    let front_for_zoom_slider = front.clone();
    let grid_zoom_syncing_for_slider = grid_zoom_syncing.clone();
    let slider_last_value = Rc::new(Cell::new(info.grid_zoom.value()));
    let slider_last_value_for_change = slider_last_value.clone();
    // Lightbox slider drags can emit many value changes inside one display
    // frame. Applying every one forces GtkPicture + GtkScrolledWindow to
    // renegotiate geometry and recenter their adjustments repeatedly, which
    // becomes visible as shaking once the image overflows the viewport.
    //
    // Apply the first movement immediately for direct response, then keep only
    // the newest absolute slider value and publish at most once per ~60 Hz
    // frame. This is throttling, not a trailing debounce: the control remains
    // live under the pointer and never waits hundreds of milliseconds.
    let lightbox_slider_latest = Rc::new(Cell::new(info.grid_zoom.value()));
    let lightbox_slider_applied = Rc::new(Cell::new(info.grid_zoom.value()));
    let lightbox_slider_tick_running = Rc::new(Cell::new(false));
    let lightbox_slider_latest_for_change = lightbox_slider_latest.clone();
    let lightbox_slider_applied_for_change = lightbox_slider_applied.clone();
    let lightbox_slider_tick_running_for_change = lightbox_slider_tick_running.clone();

    info.grid_zoom.connect_value_changed(move |scale| {
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_ZOOM_UI event=value_changed value={:.3} lightbox_visible={} edit_visible={} syncing={} last={:.3}",
                scale.value(),
                front_for_zoom_slider.current() == Front::Lightbox,
                front_for_zoom_slider.current() == Front::Editor,
                grid_zoom_syncing_for_slider.get(),
                slider_last_value_for_change.get(),
            );
        }
        if grid_zoom_syncing_for_slider.get() {
            slider_last_value_for_change.set(scale.value());
            lightbox_slider_latest_for_change.set(scale.value());
            lightbox_slider_applied_for_change.set(scale.value());
            return;
        }

        let front = front_for_zoom_slider.current();
        if front == Front::Lightbox {
            let value = scale.value();
            lightbox_slider_latest_for_change.set(value);
            if std::env::var_os("PICASA_TRACE").is_some() {
                let fit = lightbox_for_zoom_slider.current_fit_scale();
                eprintln!(
                    "PIC_ZOOM_UI lightbox_input slider={value:.3} fit={fit:.6} native_req={:.6} current_native={:.6}",
                    zoom_slider::photo_scale(value, fit),
                    lightbox_for_zoom_slider.current_manual_zoom_scale(),
                );
            }

            if !lightbox_slider_tick_running_for_change.get() {
                lightbox_slider_tick_running_for_change.set(true);

                // Drive geometry from GTK's real frame clock rather than an
                // approximate 16 ms wall-clock timer. A timer can fire between
                // layout/paint phases and make GtkPicture resize twice around
                // one rendered frame, which shows up as a small flash. The
                // frame callback applies only the newest absolute slider value.
                let lightbox = lightbox_for_zoom_slider.clone();
                let latest = lightbox_slider_latest_for_change.clone();
                let applied = lightbox_slider_applied_for_change.clone();
                let running = lightbox_slider_tick_running_for_change.clone();
                let tick_root = lightbox.root.clone();
                tick_root.add_tick_callback(move |root, _| {
                    if !root.is_visible() {
                        running.set(false);
                        return glib::ControlFlow::Break;
                    }

                    let value = latest.get();
                    if (value - applied.get()).abs() <= f64::EPSILON {
                        running.set(false);
                        return glib::ControlFlow::Break;
                    }

                    let fit_scale = lightbox.current_fit_scale();
                    let requested = zoom_slider::photo_scale(value, fit_scale);
                    if std::env::var_os("PICASA_TRACE").is_some() {
                        eprintln!(
                            "PIC_ZOOM_UI frame_apply slider={value:.3} fit={fit_scale:.6} native_req={requested:.6} previous_slider={:.3}",
                            applied.get(),
                        );
                    }
                    lightbox.request_slider_zoom(requested);
                    applied.set(value);
                    glib::ControlFlow::Continue
                });
            }
        } else if front == Front::Editor {
            if let Some(editor) = edit_editor_for_zoom_slider.borrow().as_ref() {
                let fit_scale = editor.current_fit_scale();
                editor.set_manual_zoom_scale(zoom_slider::photo_scale(
                    scale.value(),
                    fit_scale,
                ));
            }
        } else if front == Front::DateOverview {
            if let Some(browser) = date_browser_for_zoom_slider.borrow().as_ref() {
                let width = zoom_slider::thumbnail_width(scale.value());
                browser.set_tile_width(width);
                gallery_for_zoom_slider.request_slider_zoom(width);
            }
        } else {
            gallery_for_zoom_slider
                .request_slider_zoom(zoom_slider::thumbnail_width(scale.value()));
        }
        slider_last_value_for_change.set(scale.value());
    });

    // Lightbox drives the same slider when Ctrl+wheel or the 1:1 action changes
    // the viewer zoom. The midpoint is always true native 100%.
    {
        let zoom_slider = zoom_slider.clone();
        let lightbox_for_zoom_sync = lightbox.clone();
        lightbox.set_zoom_sync_handler(move |native_scale| {
            // Fit is always the left endpoint and native 100% is always the
            // midpoint, independent of photo dimensions or window size.
            zoom_slider.show_photo_zoom(native_scale, lightbox_for_zoom_sync.current_fit_scale());
        });
    }

    // The slider follows the front surface: thumbnail sizes over the
    // gallery, photo zoom over the viewer, edit zoom in the editor.
    {
        let gallery = Rc::downgrade(&gallery);
        let lightbox = Rc::downgrade(&lightbox);
        let edit_editor = edit_editor.clone();
        zoom_slider.connect(
            &front,
            ZoomSources {
                thumbnail_width: Box::new(move || {
                    gallery.upgrade().map_or(DEFAULT_GRID_THUMBNAIL_SIZE, |gallery| gallery.current_zoom_width())
                }),
                photo_zoom: Box::new(move || {
                    lightbox.upgrade().map_or((0.0, 1.0), |lightbox| {
                        (lightbox.current_manual_zoom_scale(), lightbox.current_fit_scale())
                    })
                }),
                edit_zoom: Box::new(move || {
                    edit_editor.try_borrow().ok().and_then(|editor| {
                        editor
                            .as_ref()
                            .map(|editor| (editor.current_manual_zoom_scale(), editor.current_fit_scale()))
                    })
                }),
            },
        );
    }

    let zoom_scale_scroll =
        gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    zoom_scale_scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
    let zoom_scale_for_scroll = info.grid_zoom.clone();
    let front_for_scale_scroll = front.clone();
    zoom_scale_scroll.connect_scroll(move |controller, dx, dy| {
        crate::scroll_trace::log_scroll_input("zoom_slider", controller, dx, dy);
        if controller
            .current_event_state()
            .contains(gtk::gdk::ModifierType::CONTROL_MASK)
        {
            return glib::Propagation::Proceed;
        }
        if dy == 0.0 {
            return glib::Propagation::Stop;
        }

        let step = zoom_slider::wheel_step(zoom_slider::mode_for(front_for_scale_scroll.current()));
        let direction = if dy > 0.0 { -1.0 } else { 1.0 };
        let adjustment = zoom_scale_for_scroll.adjustment();
        let next = (zoom_scale_for_scroll.value() + direction * step)
            .clamp(adjustment.lower(), adjustment.upper());
        zoom_scale_for_scroll.set_value(next);

        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_ZOOM_UI wheel_over_slider dy={dy:.3} unit={:?} step={step:.3} next={next:.3}",
                controller.unit()
            );
        }
        glib::Propagation::Stop
    });
    info.grid_zoom.add_controller(zoom_scale_scroll);

    let grid_zoom_reset_press_x = Rc::new(Cell::new(f64::NAN));
    let grid_zoom_reset_press_x_for_press = grid_zoom_reset_press_x.clone();
    info.grid_zoom_reset.connect_pressed(move |_, _, x, _| {
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!("PIC_ZOOM_UI gesture=press x={x:.2}");
        }
        grid_zoom_reset_press_x_for_press.set(x);
    });

    let gallery_for_zoom_reset = gallery.clone();
    let date_browser_for_zoom_reset = date_browser_slot.clone();
    let lightbox_for_zoom_reset = lightbox.clone();
    let edit_editor_for_zoom_reset = edit_editor.clone();
    let front_for_zoom_reset = front.clone();
    let grid_zoom_for_reset = info.grid_zoom.clone();
    let grid_zoom_reset_press_x_for_release = grid_zoom_reset_press_x.clone();
    info.grid_zoom_reset
        .connect_released(move |_, n_press, x, _| {
            let press_x = grid_zoom_reset_press_x_for_release.replace(f64::NAN);
            if n_press != 1 || !press_x.is_finite() || grid_zoom_for_reset.width() <= 0 {
                return;
            }

            let centre_x = f64::from(grid_zoom_for_reset.width()) * 0.5;
            let was_click = (x - press_x).abs() <= 4.0;
            if !was_click || (x - centre_x).abs() > 12.0 {
                return;
            }

            let front = front_for_zoom_reset.current();
            if front == Front::Lightbox {
                lightbox_for_zoom_reset.set_manual_zoom_scale(1.0);
            } else if front == Front::Editor {
                if let Some(editor) = edit_editor_for_zoom_reset.borrow().as_ref() {
                    editor.fit();
                }
            } else if front == Front::DateOverview {
                if let Some(browser) = date_browser_for_zoom_reset.borrow().as_ref() {
                    gallery_for_zoom_reset.reset_zoom();
                    browser.set_tile_width(gallery_for_zoom_reset.current_zoom_width());
                }
            } else {
                gallery_for_zoom_reset.reset_zoom();
            }
        });

    let edit_editor_for_rotate = edit_editor.clone();
    let front_for_rotate = front.clone();

    let rotate_context = action_context.clone();
    let rotate_photo: Rc<dyn Fn(bool)> = Rc::new(move |counter_clockwise| {
        let Some((photo, rotation)) = rotate_current_photo(&rotate_context, counter_clockwise)
        else {
            return;
        };
        if front_for_rotate.current() == Front::Editor {
            if let Some(editor) = edit_editor_for_rotate.borrow().as_ref() {
                if editor.photo_id() == photo.id() {
                    editor.set_library_rotation(rotation);
                }
            }
        }
    });
    let rotate_photo_for_click = rotate_photo.clone();
    info.rotate.connect_clicked(move |_| rotate_photo_for_click(false));
    let rotate_right_click = gtk::GestureClick::new();
    rotate_right_click.set_button(3);
    rotate_right_click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let rotate_photo_for_right_click = rotate_photo;
    rotate_right_click.connect_pressed(move |gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        rotate_photo_for_right_click(true);
    });
    info.rotate.add_controller(rotate_right_click);

thread_local! {
    // True while `leave_photo_editor` starts its crossfade. GtkStack reports
    // the new page before the transition counts as running, so the page
    // handlers that detach the editor read this instead.
    static LEAVING_EDITOR_WITH_FADE: Cell<bool> = const { Cell::new(false) };
}

/// Leave the photo editor for `page`, crossfading over EDITOR_FADE_MS like
/// the editor's open does. Without animations the switch
/// is immediate; switching to the page already shown does nothing.
fn leave_photo_editor(stack: &gtk::Stack, page: &str) {
    if stack.is_mapped() && stack.settings().is_gtk_enable_animations() {
        stack.set_transition_duration(crate::lightbox::EDITOR_FADE_MS);
        LEAVING_EDITOR_WITH_FADE.with(|leaving| leaving.set(true));
        stack.set_visible_child_full(page, gtk::StackTransitionType::Crossfade);
        LEAVING_EDITOR_WITH_FADE.with(|leaving| leaving.set(false));
    } else {
        stack.set_visible_child_name(page);
    }
}

/// Detach the photo editor from its page and drop it, so it stops adding to
/// the stack's size request and releases its preview memory. While the stack
/// still crossfades away from the editor, wait for the fade to finish so the
/// editor fades out instead of vanishing; an editor opened meanwhile has
/// already replaced it and is kept.
fn detach_photo_editor(
    stack: &gtk::Stack,
    edit_page: &gtk::Box,
    edit_editor: &Rc<RefCell<Option<crate::edit::EditEditor>>>,
) {
    let fading =
        LEAVING_EDITOR_WITH_FADE.with(Cell::get) || stack.is_transition_running();
    let Some(old_root) = edit_page.first_child().filter(|_| fading) else {
        while let Some(child) = edit_page.first_child() {
            edit_page.remove(&child);
        }
        edit_editor.replace(None);
        return;
    };
    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!("PIC_EDIT_CLOSE detach=after_fade");
    }
    let edit_page = edit_page.clone();
    let edit_editor = edit_editor.clone();
    glib::timeout_add_local_once(
        Duration::from_millis(u64::from(crate::lightbox::EDITOR_FADE_MS) + 60),
        move || {
            if old_root.parent().as_ref() != Some(edit_page.upcast_ref::<gtk::Widget>()) {
                return;
            }
            edit_page.remove(&old_root);
            let same_editor = edit_editor.try_borrow().is_ok_and(|editor| {
                editor
                    .as_ref()
                    .is_some_and(|editor| editor.root.clone().upcast::<gtk::Widget>() == old_root)
            });
            if same_editor {
                edit_editor.replace(None);
            }
            if std::env::var_os("PICASA_TRACE").is_some() {
                eprintln!("PIC_EDIT_CLOSE detached dropped={same_editor}");
            }
        },
    );
}

fn show_photo_export_dialog(
    window: &gtk::Window,
    jobs: Vec<crate::edit::export_batch::ExportJob>,
    progress: Rc<OperationProgressUi>,
) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_spacing(12);
    content.set_margin_top(18);
    content.set_margin_bottom(6);
    content.set_margin_start(18);
    content.set_margin_end(18);

    let grid = gtk::Grid::new();
    grid.set_row_spacing(12);
    grid.set_column_spacing(12);

    if jobs.len() > 1 {
        let count = gtk::Label::new(Some(&format!("{} photos selected", jobs.len())));
        count.set_xalign(0.0);
        grid.attach(&count, 0, 0, 2, 1);
    }

    let size_label = gtk::Label::new(Some("Maximum long edge"));
    size_label.set_xalign(0.0);
    let size = gtk::DropDown::from_strings(&[
        "Original dimensions",
        "3840 px maximum (4K)",
        "2048 px maximum",
        "1920 px maximum (Full HD)",
        "1280 px maximum (HD)",
        "Custom maximum…",
    ]);
    size.set_selected(0);
    let size_row = if jobs.len() > 1 { 1 } else { 0 };
    grid.attach(&size_label, 0, size_row, 1, 1);
    grid.attach(&size, 1, size_row, 1, 1);

    let custom_size_label = gtk::Label::new(Some("Maximum long edge (px)"));
    custom_size_label.set_xalign(0.0);
    let custom_size = gtk::SpinButton::with_range(1.0, 30000.0, 100.0);
    custom_size.set_value(3000.0);
    custom_size.set_numeric(true);
    custom_size.set_digits(0);
    custom_size.set_tooltip_text(Some("Maximum output size for the longest edge"));
    let custom_size_row = size_row + 1;
    grid.attach(&custom_size_label, 0, custom_size_row, 1, 1);
    grid.attach(&custom_size, 1, custom_size_row, 1, 1);
    custom_size_label.set_visible(false);
    custom_size.set_visible(false);
    {
        let custom_size_label = custom_size_label.clone();
        let custom_size = custom_size.clone();
        size.connect_selected_notify(move |dropdown| {
            let visible = dropdown.selected() == 5;
            custom_size_label.set_visible(visible);
            custom_size.set_visible(visible);
        });
    }

    let quality_label = gtk::Label::new(Some("JPEG quality"));
    quality_label.set_xalign(0.0);
    let quality = gtk::Scale::with_range(gtk::Orientation::Horizontal, 60.0, 100.0, 1.0);
    quality.set_value(crate::edit::export_batch::default_export_quality() as f64);
    quality.set_draw_value(false);
    quality.set_hexpand(true);
    quality.set_tooltip_text(Some("JPEG image quality; disabled for PNG and WebP"));
    let quality_value = gtk::Label::new(None);
    quality_value.set_width_chars(5);
    quality_value.set_xalign(1.0);
    quality_value.add_css_class("edit-adjustment-value");
    quality_value.set_text(&format!("{:.0}", quality.value()));
    let quality_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    quality_row.append(&quality);
    quality_row.append(&quality_value);
    let quality_grid_row = size_row + 2;
    grid.attach(&quality_label, 0, quality_grid_row, 1, 1);
    grid.attach(&quality_row, 1, quality_grid_row, 1, 1);
    quality.connect_value_changed({
        let quality_value = quality_value.clone();
        move |scale| quality_value.set_text(&format!("{:.0}", scale.value()))
    });

    let type_label = gtk::Label::new(Some("File type"));
    type_label.set_xalign(0.0);
    let file_type = gtk::DropDown::from_strings(
        &crate::edit::export_batch::ExportFormat::dialog_labels(),
    );
    file_type.set_selected(0);
    let type_row = size_row + 3;
    grid.attach(&type_label, 0, type_row, 1, 1);
    grid.attach(&file_type, 1, type_row, 1, 1);
    {
        let quality = quality.clone();
        let quality_label = quality_label.clone();
        file_type.connect_selected_notify(move |dropdown| {
            let enabled = dropdown.selected() == 0;
            quality.set_sensitive(enabled);
            quality_label.set_sensitive(enabled);
        });
    }
    content.append(&grid);
    let includes_nikon_raw = jobs.iter().any(|job| {
        crate::image_format::for_path(&job.source_path)
            .is_some_and(|format| format.id == "nikon_raw")
    });
    let size_help = gtk::Label::new(Some(if includes_nikon_raw {
        "NEF exports use full-resolution sensor data. Size limits the long edge; JPEG quality controls compression."
    } else {
        "Size limits the long edge; JPEG quality controls compression."
    }));
    size_help.set_xalign(0.0);
    size_help.set_wrap(true);
    size_help.add_css_class("dim-label");
    content.append(&size_help);
    let dialog = adw::AlertDialog::builder()
        .heading(if jobs.len() == 1 {
            "Export Photo"
        } else {
            "Export Photos"
        })
        .extra_child(&content)
        .close_response("cancel")
        .default_response("export")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("export", "Export");
    dialog.set_response_appearance("export", adw::ResponseAppearance::Suggested);

    {
        let window = window.clone();
        let size = size.clone();
        let custom_size = custom_size.clone();
        let quality = quality.clone();
        let file_type = file_type.clone();
        dialog.connect_response(Some("export"), move |_, _| {
            let max_edge = match size.selected() {
                1 => 3840,
                2 => 2048,
                3 => 1920,
                4 => 1280,
                5 => custom_size.value().round() as u32,
                _ => 0,
            };
            let quality = quality.value().round().clamp(60.0, 100.0) as u8;
            let format =
                crate::edit::export_batch::ExportFormat::from_dialog_index(file_type.selected());
            let jobs = jobs
                .iter()
                .map(|job| {
                    let mut job = job.clone();
                    job.file_name =
                        crate::edit::export_batch::file_name_for_format(&job.file_name, format);
                    job
                })
                .collect::<Vec<_>>();
            choose_photo_export_destination(
                &window,
                jobs,
                max_edge,
                quality,
                format,
                progress.clone(),
            );
        });
    }
    dialog.present(Some(window));
}

fn choose_photo_export_destination(
    window: &gtk::Window,
    jobs: Vec<crate::edit::export_batch::ExportJob>,
    max_edge: u32,
    quality: u8,
    format: crate::edit::export_batch::ExportFormat,
    progress: Rc<OperationProgressUi>,
) {
    let multiple = jobs.len() > 1;
    if multiple {
        let dialog = gtk::FileDialog::builder()
            .title("Export Photos to Folder")
            .accept_label("Export")
            .modal(true)
            .build();
        let parent = window.clone();
        dialog.select_folder(Some(window), None::<&gio::Cancellable>, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    if jobs.iter().any(|job| path.join(&job.file_name).exists()) {
                        let confirm = adw::AlertDialog::builder()
                            .heading("Export files already exist")
                            .body("Replace files with matching names, or keep both by adding numbers to the new filenames?")
                            .close_response("cancel")
                            .default_response("keep-both")
                            .build();
                        confirm.add_response("cancel", "Cancel");
                        confirm.add_response("keep-both", "Keep Both");
                        confirm.add_response("replace", "Replace Existing");
                        confirm.set_response_appearance(
                            "replace",
                            adw::ResponseAppearance::Destructive,
                        );
                        let parent = parent.clone();
                        let progress = progress.clone();
                        let jobs = jobs.clone();
                        let path = path.clone();
                        confirm.connect_response(None, move |_, response| {
                            if response == "keep-both" || response == "replace" {
                                start_photo_export_folder(
                                    progress.clone(),
                                    jobs.clone(),
                                    path.clone(),
                                    max_edge,
                                    quality,
                                    format,
                                    response == "replace",
                                );
                            }
                        });
                        confirm.present(Some(&parent));
                    } else {
                        start_photo_export_folder(
                            progress.clone(),
                            jobs.clone(),
                            path,
                            max_edge,
                            quality,
                            format,
                            false,
                        );
                    }
                }
            }
        });
    } else {
        let Some(job) = jobs.first().cloned() else {
            return;
        };
        let initial_name = job.file_name.clone();
        let on_selected = Rc::new(move |path, replace_existing| {
            start_photo_export_single(
                progress.clone(),
                job.clone(),
                path,
                max_edge,
                quality,
                format,
                replace_existing,
            );
        });
        crate::save_dialog::choose_export_save_path(
            window,
            "Export Photo",
            "Export",
            &initial_name,
            Some(format.extension()),
            None,
            on_selected,
        );
    }
}

enum ExportProgressMessage {
    Update {
        done: usize,
        filename: String,
        failed: usize,
    },
    Done(crate::edit::export_batch::BatchExportOutcome),
}

fn watch_export_progress(
    progress: Rc<OperationProgressUi>,
    receiver: std::sync::mpsc::Receiver<ExportProgressMessage>,
    total: usize,
) {
    glib::timeout_add_local(Duration::from_millis(80), move || {
        let mut flow = glib::ControlFlow::Continue;
        while let Ok(message) = receiver.try_recv() {
            match message {
                ExportProgressMessage::Update {
                    done,
                    filename,
                    failed,
                    ..
                } => {
                    progress.update("Exporting photos", done, total, &filename, failed);
                }
                ExportProgressMessage::Done(outcome) => {
                    let cancelled = outcome
                        .errors
                        .iter()
                        .any(|error| error.contains("cancelled"));
                    let summary = if cancelled {
                        format!(
                            "Export stopped — {} exported · {} skipped · {} failed",
                            outcome.exported, outcome.skipped, outcome.failed
                        )
                    } else if outcome.failed == 0 && outcome.skipped == 0 {
                        format!("Export complete — {} exported", outcome.exported)
                    } else {
                        format!(
                            "Export complete — {} exported · {} skipped · {} failed",
                            outcome.exported, outcome.skipped, outcome.failed
                        )
                    };
                    for error in outcome.errors.iter().take(5) {
                        eprintln!("Export issue: {error}");
                    }
                    let title = if cancelled {
                        "Export stopped"
                    } else {
                        "Export complete"
                    };
                    progress.finish(title, &summary);
                    flow = glib::ControlFlow::Break;
                }
            }
        }
        flow
    });
}

fn start_photo_export_folder(
    progress: Rc<OperationProgressUi>,
    jobs: Vec<crate::edit::export_batch::ExportJob>,
    folder: std::path::PathBuf,
    max_edge: u32,
    quality: u8,
    format: crate::edit::export_batch::ExportFormat,
    replace_existing: bool,
) {
    let total = jobs.len();
    progress.begin("Exporting photos", total);
    let cancel = progress.cancel_flag();
    let (sender, receiver) = std::sync::mpsc::channel();
    watch_export_progress(progress, receiver, total);
    std::thread::spawn(move || {
        let outcome = crate::edit::export_batch::run_batch_export_with_workers_and_overwrite(
            &jobs,
            &folder,
            max_edge,
            quality,
            format,
            crate::edit::export_batch::DEFAULT_BATCH_EXPORT_WORKERS,
            replace_existing,
            |job| crate::edit::export_batch::render_job(job),
            |done, _total, filename, failed| {
                let _ = sender.send(ExportProgressMessage::Update {
                    done,
                    filename: filename.to_string(),
                    failed,
                });
            },
            Some(&cancel),
        );
        let _ = sender.send(ExportProgressMessage::Done(outcome));
    });
}

fn start_photo_export_single(
    progress: Rc<OperationProgressUi>,
    job: crate::edit::export_batch::ExportJob,
    destination: std::path::PathBuf,
    max_edge: u32,
    quality: u8,
    format: crate::edit::export_batch::ExportFormat,
    replace_existing: bool,
) {
    progress.begin("Exporting photos", 1);
    let cancel = progress.cancel_flag();
    let (sender, receiver) = std::sync::mpsc::channel();
    watch_export_progress(progress, receiver, 1);
    std::thread::spawn(move || {
        let file_name = job.file_name.clone();
        let outcome = if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            crate::edit::export_batch::BatchExportOutcome {
                errors: vec!["cancelled".into()],
                ..Default::default()
            }
        } else if !crate::source::file_available(&job.source_path) {
            crate::edit::export_batch::BatchExportOutcome {
                skipped: 1,
                errors: vec![format!("skipped (missing): {}", job.source_path)],
                ..Default::default()
            }
        } else {
            let render = |job: &crate::edit::export_batch::ExportJob| {
                crate::edit::export_batch::render_job(job)
            };
            let result = if replace_existing {
                crate::edit::export_batch::export_job_to_confirmed_replace(
                    &job,
                    &destination,
                    max_edge,
                    quality,
                    format,
                    &render,
                )
            } else {
                crate::edit::export_batch::export_job_to(
                    &job,
                    &destination,
                    max_edge,
                    quality,
                    format,
                    &render,
                )
            };
            match result {
                Ok(()) => crate::edit::export_batch::BatchExportOutcome {
                    exported: 1,
                    ..Default::default()
                },
                Err(error) => crate::edit::export_batch::BatchExportOutcome {
                    failed: 1,
                    errors: vec![format!("{}: {error:#}", job.source_path)],
                    ..Default::default()
                },
            }
        };
        let _ = sender.send(ExportProgressMessage::Update {
            done: 1,
            filename: file_name,
            failed: outcome.failed,
        });
        let _ = sender.send(ExportProgressMessage::Done(outcome));
    });
}

    let parent_for_export = window.clone();
    let gallery_for_export = gallery.clone();
    let progress_for_export = operation_progress.clone();

    let export_anchored: ExportAnchored = {
        let parent_for_export = parent_for_export;
        let gallery_for_export = gallery_for_export;
        let progress_for_export = progress_for_export;
        Rc::new(move |photo: &crate::photo_object::PhotoObject| {
            // A photo outside the library is in no gallery list: export it
            // alone, from the viewer's object.
            if photo.is_outside_library() {
                let job = crate::edit::export_batch::ExportJob::from_record(
                    photo.path(),
                    photo.rotation(),
                    photo.edit_recipe(),
                    photo.width(),
                    photo.height(),
                    &photo.filename(),
                );
                show_photo_export_dialog(
                    parent_for_export.upcast_ref::<gtk::Window>(),
                    vec![job],
                    progress_for_export.clone(),
                );
                return;
            }
            let ids = gallery_for_export.selected_photo_ids(Some(photo.id()));
            let known: std::collections::HashMap<i64, _> = gallery_for_export
                .photo_objects()
                .into_iter()
                .map(|object| (object.id(), object))
                .collect();
            let jobs = ids
                .iter()
                .filter_map(|id| known.get(id))
                .map(|object| {
                    crate::edit::export_batch::ExportJob::from_record(
                        object.path(),
                        object.rotation(),
                        object.edit_recipe(),
                        object.width(),
                        object.height(),
                        &object.filename(),
                    )
                })
                .collect::<Vec<_>>();
            if jobs.is_empty() {
                return;
            }
            show_photo_export_dialog(
                parent_for_export.upcast_ref::<gtk::Window>(),
                jobs,
                progress_for_export.clone(),
            );
        })
    };
    info.export.connect_clicked({
        let info = Rc::downgrade(&info);
        let export_anchored = export_anchored.clone();
        move |_| {
            if let Some(photo) = info.upgrade().and_then(|info| info.current_photo()) {
                export_anchored(&photo);
            }
        }
    });
    export_slot.replace(Some(export_anchored));
    // The Scrapbook Library selection is independent of the photo grid;
    // export the exact clicked photo through PIC's normal export dialog.
    scrapbook_export_slot.replace(Some(Rc::new({
        let connection = connection.clone();
        let parent = window.clone().upcast::<gtk::Window>();
        let progress = operation_progress.clone();
        move |id| {
            let Some(record) = db::photo(&connection.borrow(), id).ok().flatten() else {
                return;
            };
            let object = crate::photo_object::PhotoObject::from_photo(&record);
            let job = crate::edit::export_batch::ExportJob::from_record(
                object.path(),
                object.rotation(),
                object.edit_recipe(),
                object.width(),
                object.height(),
                &object.filename(),
            );
            show_photo_export_dialog(&parent, vec![job], progress.clone());
        }
    })));

    let info_for_print = Rc::downgrade(&info);
    let window_for_print = window.clone();
    let connection_for_print = connection.clone();
    let gallery_for_print = gallery.clone();

    info.print.connect_clicked(move |_| {
        let Some(photo) = info_for_print.upgrade().and_then(|info| info.current_photo()) else {
            return;
        };

        // Print every selected photo (the full multi-selection, falling back
        // to the anchor photo when nothing extra is selected). PhotoObject is
        // a GTK object and must stay on the GTK thread: copy only Send-safe
        // scalar/string values into the print requests; the renders happen on
        // a worker thread while the dialog runs here.
        let ids = gallery_for_print.selected_photo_ids(Some(photo.id()));
        let requests = photo_print_requests(&connection_for_print.borrow(), &ids);

        print_photos(&window_for_print, &connection_for_print, requests);
    });

    let layout_started = Instant::now();
    let (
        main_split,
        main_surface,
        sidebar,
        thumbnail_recovery_requested,
        thumbnail_recovery_deferred,
        sidebar_for_events,
        right_column,
        right_header,
        _search,
        left_column,
        sidebar_shell,
        sidebar_widgets,
    ) = include!("layout.rs");
    crate::startup_trace::mark("layout_widgets_done", Some(layout_started));

    // Up/Down in the viewer moves to the previous or next collection in the
    // sidebar's visible order (candidate 18).
    {
        let navigation = CollectionNavigation::new(CollectionNavigationParts {
            connection: connection.clone(),
            filter: filter.clone(),
            search: search_text.clone(),
            sort: sort.clone(),
            group_mode: group_mode.clone(),
            gallery: gallery.clone(),
            lightbox: lightbox.clone(),
            front: front.clone(),
            sidebar_destinations: {
                let sidebar = sidebar_selection_slot.clone();
                Rc::new(move || {
                    sidebar
                        .borrow()
                        .as_ref()
                        .map(sidebar::navigation_destinations)
                        .unwrap_or_default()
                })
            },
            enter_destination: {
                let enter = enter_destination_slot.clone();
                Rc::new(move |page| {
                    if let Some(enter) = enter.borrow().as_ref() {
                        enter(page);
                    }
                })
            },
        });
        lightbox.set_collection_navigation_handler(move |direction| navigation.navigate(direction));
    }

    // Window-level photo shortcuts own keys that must work regardless of
    // whether the GridView, sectioned Folder renderer or Lightbox has focus.
    // Real text editors always keep their normal keyboard behaviour.
    {
        let window_for_keys = window.clone();
        let gallery_for_keys = gallery.clone();
        let filter_for_keys = filter.clone();
        let front_for_keys = front.clone();
        let marks_for_keys = action_context.clone();
        let search_for_keys = search_text.clone();

        let photo_keys = gtk::EventControllerKey::new();
        photo_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        photo_keys.connect_key_pressed(move |_, key, _, modifiers| {
            let typing = std::iter::successors(
                gtk::prelude::RootExt::focus(&window_for_keys),
                |widget| widget.parent(),
            )
            .any(|widget| widget.is::<gtk::Editable>() || widget.is::<gtk::TextView>());
            if typing {
                return glib::Propagation::Proceed;
            }

            let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let disallowed_modifier = modifiers.intersects(
                gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK
                    | gtk::gdk::ModifierType::META_MASK,
            );
            // The Photos gallery (with or without the date overview) owns
            // browse keys; the viewer keeps its own key map.
            let gallery_front =
                || matches!(front_for_keys.current(), Front::Gallery | Front::DateOverview);

            // Browse navigation is handled at the window boundary so it is
            // independent of whether a thumbnail, the grid, or a sidebar row
            // retained focus. Text fields, Edit, Collage, and the Lightbox
            // keep their own key maps.
            if gallery_front() && !disallowed_modifier {
                let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
                let mut browser_command = false;
                if control && !shift {
                    browser_command = matches!(
                        key,
                        gtk::gdk::Key::Home
                            | gtk::gdk::Key::End
                            | gtk::gdk::Key::Up
                            | gtk::gdk::Key::Down
                    );
                } else if !control && !shift {
                    browser_command = matches!(
                        key,
                        gtk::gdk::Key::Left
                            | gtk::gdk::Key::Right
                            | gtk::gdk::Key::Up
                            | gtk::gdk::Key::Down
                            | gtk::gdk::Key::Page_Up
                            | gtk::gdk::Key::Page_Down
                            | gtk::gdk::Key::Home
                            | gtk::gdk::Key::End
                    );
                }
                if browser_command {
                    let focus_is_in_gallery = std::iter::successors(
                        gtk::prelude::RootExt::focus(&window_for_keys),
                        |widget| widget.parent(),
                    )
                    .any(|widget| widget == gallery_for_keys.visible_root());
                    let arrow = matches!(
                        key,
                        gtk::gdk::Key::Left
                            | gtk::gdk::Key::Right
                            | gtk::gdk::Key::Up
                            | gtk::gdk::Key::Down
                    );
                    if !arrow || !focus_is_in_gallery || control {
                        let current_photo_id = marks_for_keys
                            .info
                            .current_photo()
                            .map(|photo| photo.id());
                        if gallery_for_keys.navigate_browser_key(key, control, current_photo_id) {
                            return glib::Propagation::Stop;
                        }
                    }
                }
            }

            if control && !disallowed_modifier && matches!(key, gtk::gdk::Key::a | gtk::gdk::Key::A) {
                if gallery_front() {
                    // Ctrl+A should select the complete visible result set.
                    // A folder normally scopes selection to the active folder,
                    // but search can return matches from elsewhere in the
                    // catalogue, so its filtered gallery is the scope.
                    let folder_id = if search_for_keys.borrow().is_empty() {
                        match filter_for_keys.get() {
                            sidebar::SidebarFilter::Folder(sidebar_folder_id) => {
                                gallery_for_keys.selected_folder_id().or(Some(sidebar_folder_id))
                            }
                            _ => None,
                        }
                    } else {
                        None
                    };
                    if std::env::var_os("PICASA_TRACE").is_some() {
                        eprintln!("UI TRACE ctrl_a folder_id={folder_id:?}");
                    }
                    gallery_for_keys.select_all_keyboard_scope(folder_id);
                    return glib::Propagation::Stop;
                }
                return glib::Propagation::Proceed;
            }

            if control && !disallowed_modifier && matches!(key, gtk::gdk::Key::d | gtk::gdk::Key::D) {
                if gallery_front() {
                    gallery_for_keys.clear_keyboard_selection();
                    return glib::Propagation::Stop;
                }
                return glib::Propagation::Proceed;
            }

            // Bare numeric keys are reserved outside text fields so a year such
            // as 2026 cannot accidentally start type-to-search. 0 clears the
            // rating, 1..5 assign that rating to the complete current selection,
            // and 6..9 are intentionally consumed without an action.
            if !control && !disallowed_modifier {
                if let Some(digit) = key
                    .to_unicode()
                    .and_then(|character| character.to_digit(10))
                {
                    if digit <= 5
                        && matches!(
                            front_for_keys.current(),
                            Front::Gallery | Front::DateOverview | Front::Lightbox
                        )
                    {
                        if let Some(photo) = marks_for_keys.info.current_photo() {
                            photo_mark_commands::run(
                                &marks_for_keys,
                                MarkIntent::SetRating(digit as u8),
                                &photo,
                            );
                        }
                    }
                    return glib::Propagation::Stop;
                }
            }

            glib::Propagation::Proceed
        });
        window.add_controller(photo_keys);
    }

    // Appearance button: opens Settings → Themes, where the theme list is
    // built from the theme folders on disk. toolbar.rs appends the
    // `settings` button to the infobar utilities.
    let settings = crate::window::theme::appearance_button();
    theme_engine.set_appearance_button(&settings);
    // Theme switches redraw the albums home so covers pick up the palette.
    // Window chrome (traffic lights vs native title buttons) is wired inside
    // layout.rs, where the headers and the placement closure live.
    theme_engine.set_post_apply(theme_post_apply);
    {
        let present_settings_for_appearance = present_settings.clone();
        settings.connect_clicked(move |_| {
            present_settings_for_appearance(Some("themes"));
        });
    }
    let (refresh, metadata_panel) = include!("toolbar.rs");
    metadata_panel.set_operation_progress(operation_progress.clone());
    // Suppress the metadata bar while viewing a photo and restore its prior
    // mode (individual filters or ALL columns) when returning to the grid.
    // The InfoBar, Properties and the metadata panel follow the front
    // surface through one listener.
    front_mirrors::FrontMirrors {
        info: info.clone(),
        properties: properties.clone(),
        metadata_panel: metadata_panel.clone(),
        gallery: Rc::downgrade(&gallery),
        filter: filter.clone(),
        selection_preview_limit: SELECTION_INFOBAR_PREVIEW_LIMIT,
    }
    .connect(&front);
    // Restore the panel before preparing the destination grid. The lightbox
    // still covers the combined surface, so GTK can finish the gallery's
    // shorter allocation before the close fade reveals it.
    {
        let panel_for_close = metadata_panel.clone();
        lightbox.set_close_started_handler(move || {
            panel_for_close.set_lightbox_open(false);
        });
    }
    {
        let panel_for_navigation = Rc::downgrade(&metadata_panel);
        let filter_for_navigation = filter.clone();
        // Several sidebar destinations share the photos stack page. Observe
        // their filter changes before painting, too, without closing a panel
        // that the user manually reopened in the same destination.
        main_stack.add_tick_callback(move |stack, _| {
            let Some(panel) = panel_for_navigation.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let page = stack.visible_child_name();
            panel.set_navigation_view(page.as_deref().unwrap_or(""), filter_for_navigation.get());
            glib::ControlFlow::Continue
        });
    }

    // The viewer must include the metadata panel's former space. Parent it
    // above the combined panel and gallery surface, so hiding the panel frees
    // that height to the lightbox while the gallery reflows behind its cover.
    gallery_overlay.remove_overlay(&lightbox.root);
    let gallery_surface = gtk::Overlay::new();
    gallery_surface.set_hexpand(true);
    gallery_surface.set_vexpand(true);
    let gallery_stack = gtk::Box::new(gtk::Orientation::Vertical, 0);
    gallery_stack.add_css_class("gallery-content-surface");
    // The transparent filter panel should reveal the same surface as the grid.
    gallery_stack.add_css_class("photo-grid");
    gallery_stack.set_hexpand(true);
    gallery_stack.set_vexpand(true);
    gallery_stack.append(&metadata_panel.root);
    gallery_stack.append(&content);
    gallery_surface.set_child(Some(&gallery_stack));
    gallery_surface.add_overlay(&lightbox.root);
    // Keep the viewer's Properties panel above the lightbox itself. The
    // shared InfoBar routes toggles here while the lightbox is active.
    gallery_surface.add_overlay(&lightbox_properties_panel);
    right_column.append(&right_header);
    right_column.append(&gallery_surface);
    // Allocate the toolbar outside the viewer. An overlay here made Fit
    // center in a taller area whose bottom was covered by the toolbar.
    info.root.set_hexpand(true);
    info.root.set_valign(gtk::Align::End);
    right_column.append(&info.root);
    // Build the immersive overlays once from PIC's existing controls.
    include!("immersive.rs");
    {
        let panel=metadata_panel.clone();
        window.connect_close_request(move |_| {panel.cancel_index();glib::Propagation::Proceed});
    }

    window.set_content(Some(&main_surface));
    crate::startup_trace::mark("widget_content_and_signals_done", Some(build_started));

    let startup_gallery = gallery.clone();
    let startup_photos_for_idle = startup_photos.clone();
    let startup_view_restored = Rc::new(Cell::new(false));
    let last_activated_for_restore = last_activated_photo_id.clone();
    let restore_startup_view: Rc<dyn Fn()> = {
        let restored = startup_view_restored.clone();
        let gallery = gallery.clone();
        let albums_home = albums_home.clone();
        let recovery_requested = thumbnail_recovery_requested.clone();
        Rc::new(move || {
            if restored.replace(true) {
                return;
            }
            restore_saved_viewport(
                &gallery,
                &albums_home,
                SavedViewport {
                    filter: initial_filter,
                    activated_photo: last_activated_for_restore.get(),
                    anchor_photo: saved_view_photo_id,
                    gallery_scroll: saved_view_scroll,
                    albums_scroll: saved_albums_scroll,
                },
            );
            recovery_requested.set(true);
        })
    };
    let startup_generation = REFRESH_GENERATION.load(std::sync::atomic::Ordering::Relaxed);
    populate_startup_gallery(
        startup_gallery, gallery_scroll_stack.clone().upcast(), startup_photos_for_idle,
        Rc::new(move || REFRESH_GENERATION.load(std::sync::atomic::Ordering::Relaxed) == startup_generation),
        restore_startup_view,
    );

    // Responsive sidebar transition.
    //
    // Do not use an AdwBreakpoint to flip `collapsed` directly: changing
    // collapsed is a structural side-by-side -> overlay switch and therefore
    // looks like the sidebar is cut out before its drawer animation can run.
    //
    // Instead stage the transition:
    //   narrow: slide sidebar out -> then collapse
    //   wide:   expand while hidden -> then slide sidebar in
    //
    // Small hysteresis avoids chattering when the user drags around 1050 px.
    // The widths and hysteresis live in `sidebar_presentation::breakpoint_step`.

    {
        // The sidebar presentation module decides each step (and ignores the
        // breakpoint while immersive, where the sidebar must stay a
        // collapsed overlay); its adapter times the two-phase transitions.
        let widgets = sidebar_widgets.clone();
        window.add_tick_callback(move |window, _| {
            widgets.dispatch(SidebarEvent::BreakpointCrossed {
                window_width: window.width(),
            });
            glib::ControlFlow::Continue
        });
    }

    crate::css::install_foundation(&display);
    // Apply the persisted theme (and the base layer) before the window's
    // first rendered frame. Must run after the foundation so the base theme
    // is added after base.css at the same priority and wins.
    theme_engine.startup();

    let (scan_sender, scan_receiver) = std::sync::mpsc::channel::<ScanUiEvent>();
    let (refresh_prepare_sender, refresh_prepare_receiver) =
        std::sync::mpsc::channel::<RefreshPrepareEvent>();

    let scan_job = Rc::new(RefCell::new(ScanJobState::default()));
    maintenance_cancel_scans.replace(Some({
        let scan_job = scan_job.clone();
        let pending_watch_refreshes = pending_watch_refreshes.clone();
        Rc::new(move || {
            let traced = std::env::var_os("PIC_LIBRARY_MAINTENANCE_TRACE").is_some();
            let mut job = scan_job.borrow_mut();
            if let Some(active) = job.active.take() {
                active.interrupt();
            }
            job.generation = job.generation.wrapping_add(1);
            job.kind = None;
            job.active_root = None;
            job.pending.clear();
            job.resume_roots.clear();
            job.stop_requested = true;
            pending_watch_refreshes.borrow_mut().clear();
            if traced {
                eprintln!("PIC_LIBRARY_MAINTENANCE action=cancel_scans phase=complete");
            }
        })
    }));
    sidebar::bind_refresh_gate(&sidebar, &refresh);
    // Keep cancellation reachable after the notification itself is dismissed.
    let dismissed_stop = gtk::Button::with_label("Stop");
    dismissed_stop.set_tooltip_text(Some("Stop background task"));
    dismissed_stop.add_css_class("flat");
    dismissed_stop.add_css_class("dismissed-task-stop");
    dismissed_stop.set_visible(false);
    right_header.pack_end(&dismissed_stop);
    {
        let job = scan_job.clone();
        let refresh = refresh.downgrade();
        let stop = refresh_status_stop.clone();
        let dismissed_stop = dismissed_stop.clone();
        let progress = operation_progress.clone();
        // Includes preparation and cancellation acknowledgement, not merely
        // the time an individual folder worker is active.
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let Some(refresh) = refresh.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let job = job.borrow();
            let busy = job.kind.is_some();
            let exporting = progress.is_running();
            refresh.set_sensitive(!busy);
            let can_stop = (busy && !job.stop_requested)
                || (exporting && !progress.is_cancelled());
            stop.set_sensitive(can_stop);
            dismissed_stop.set_visible(progress.is_dismissed() && (busy || exporting));
            dismissed_stop.set_sensitive(can_stop);
            glib::ControlFlow::Continue
        });
    }
    // Scans and batch operations share the compact Stop control in the
    // notification panel beside Search.
    let stop_scan = operation_progress.stop().clone();

    let onboarding_slot: Rc<RefCell<Option<Rc<onboarding::OnboardingCoordinator>>>> =
        Rc::new(RefCell::new(None));

    let start_next_scan: Rc<dyn Fn()> = {
        let scan_job = scan_job.clone();
        let scan_sender = scan_sender.clone();
        Rc::new(move || {
            let next = {
                let mut job = scan_job.borrow_mut();
                if job.active.is_some() {
                    None
                } else {
                    job.pending.pop_front().map(|root| {
                        let resume=job.resume_roots.remove(&root);
                        (root,job.generation,resume)
                    })
                }
            };
            let Some((root, generation, resume)) = next else {
                return;
            };
            
            let Ok(database) = db::active_database_path() else {
                scan_job.borrow_mut().kind = None;
                return;
            };
            let control = spawn_tagged_scan(root.clone(), database, generation, scan_sender.clone(),resume);
            let mut job = scan_job.borrow_mut();
            job.active_root = Some(root);
            job.active = Some(control);
        })
    };

    refresh_folder_slot.replace(Some({
        let scan_job = scan_job.clone();
        let refresh_prepare_sender = refresh_prepare_sender.clone();
        let refresh_status_spinner = refresh_status_spinner.clone();
        let refresh_status_label = refresh_status_label.clone();
        let stop_scan = stop_scan.clone();
        let operation_progress = operation_progress.clone();
        let refresh = refresh.clone();
        Rc::new(move |reason: PhotoScanRequestReason, path: String| {
            
            if reason.scan_kind().is_none() {
                
                return;
            }
            if scan_job.borrow().kind.is_some() {
                
                refresh_status_label.set_text("Refresh already running…");
                refresh_status_spinner.set_spinning(true);
                operation_progress.present();
                stop_scan.set_visible(true);
                return;
            }
            // A full Refresh All may still have a background availability/sidebar
            // rebuild pending. Do not let that stale worker mutate the sidebar
            // while a targeted folder refresh is starting.
            refresh.set_sensitive(false);
            invalidate_availability_refreshes();
            refresh_status_label.set_text(&format!("Refreshing {}…", crate::source::filename(&path)));
            refresh_status_spinner.set_spinning(true);
            operation_progress.present();
            stop_scan.set_visible(true);
            let generation = {
                let mut job = scan_job.borrow_mut();
                job.authorize_photo_scan(reason)
                    .expect("folder refresh reason was authorized")
            };
            let sender = refresh_prepare_sender.clone();
            let database = db::active_database_path();
            std::thread::spawn(move || {
                let imported_root = database
                    .and_then(|database| db::open_existing(&database))
                    .and_then(|connection| db::folders(&connection))
                    .map(|folders| {
                        folders
                            .into_iter()
                            .any(|folder| folder.path == path && folder.imported_root)
                    })
                    .map_err(|error| error.to_string());
                let _ = sender.send(RefreshPrepareEvent::FolderReady {
                    generation,
                    path,
                    imported_root,
                });
            });
        })
    }));

    // Debounce/coalesce monitor activity independently from scan authorization.
    // One busy scan cannot be interrupted by a watch event; the dirty root
    // remains pending and is submitted after the current job becomes idle.
    {
        let pending = pending_watch_refreshes.clone();
        let scan_job = scan_job.clone();
        let refresh_folder_slot = refresh_folder_slot.clone();
        glib::timeout_add_local(Duration::from_millis(250), move || {
            if scan_job.borrow().kind.is_some() {
                return glib::ControlFlow::Continue;
            }
            let now = Instant::now();
            let ready = pending
                .borrow()
                .iter()
                .find_map(|(path, changed_at)| {
                    (now.duration_since(*changed_at) >= Duration::from_millis(750))
                        .then(|| path.clone())
                });
            if let Some(path) = ready {
                pending.borrow_mut().remove(&path);
                if watch_trace {
                    eprintln!("PIC_WATCH dispatch root={path}");
                }
                if let Some(callback) = refresh_folder_slot.borrow().as_ref() {
                    callback(PhotoScanRequestReason::DebouncedWatchRefresh, path);
                }
            }
            glib::ControlFlow::Continue
        });
    }
    rebuild_folder_watches();

    let cancel_scan_job: Rc<dyn Fn()> = {
        let scan_job = scan_job.clone();
        let connection = connection.clone();
        let refresh_status_label = refresh_status_label.clone();
        Rc::new(move || {
            let mut job = scan_job.borrow_mut();
            // Stop means the whole current job. In particular, a library refresh
            // must not continue with the next queued folder after cancellation.
            job.stop_requested = true;
            // Signal before SQLite persistence, which can wait for the writer.
            if let Some(control) = job.active.as_ref() { control.cancel(); }
            refresh_status_label.set_text(if matches!(job.kind, Some(ScanJobKind::Refresh | ScanJobKind::FolderRefresh)) {
                "Stopping refresh…"
            } else { "Stopping scan…" });
            for root in job.active_root.iter().chain(job.pending.iter()) {
                if let Err(error)=db::pause_network_import(&connection.borrow(),root) {
                    eprintln!("Could not save stopped import {root}: {error}");
                }
            }
            job.pending.clear();
            job.resume_roots.clear();
        })
    };
    let cancel_current: Rc<dyn Fn()> = {
        let cancel = cancel_scan_job.clone();
        let progress = operation_progress.clone();
        Rc::new(move || {
            if progress.is_running() {
                progress.request_cancel();
            } else {
                cancel();
            }
        })
    };
    {
        let cancel = cancel_current.clone();
        refresh_status_stop.connect_clicked(move |_| cancel());
    }
    {
        let cancel = cancel_current.clone();
        dismissed_stop.connect_clicked(move |_| cancel());
    }

    // Recover existing indexed photos at startup, on mount changes, or after
    // manual Refresh. Offline drives may stay disconnected for hours; leave
    // them idle until one of those events requests another recovery pass.
    let startup_paths = std::sync::Arc::new(
        startup_photos
            .iter()
            .map(|photo| photo.path.clone())
            .collect::<std::collections::HashSet<_>>(),
    );
    let start_thumbnail_recovery: Rc<dyn Fn()> = {
        let connection = connection.clone();
        let scan_job = scan_job.clone();
        let sender = scan_sender.clone();
        let requested = thumbnail_recovery_requested.clone();
        let operation_progress = operation_progress.clone();
        Rc::new(move || {
            if scan_job.borrow().kind.is_some() || !requested.get() {
                return;
            }
            crate::startup_trace::mark("thumbnail_recovery_start", None);
            let recovery_query_started = Instant::now();
            let Ok(mut photos) = db::photos(&connection.borrow(), None, false, None) else {
                return;
            };
            crate::startup_trace::mark(
                "thumbnail_recovery_photo_query",
                Some(recovery_query_started),
            );
            retain_enabled_formats(&connection.borrow(), &mut photos);
            requested.set(false);
            // No enabled photos need no cache checks, worker, or progress UI.
            if photos.is_empty() {
                return;
            }
            let mut job = scan_job.borrow_mut();
            job.generation = job.generation.wrapping_add(1);
            job.kind = Some(ScanJobKind::Maintenance);
            job.imported_total = 0;
            job.failed_total = 0;
            job.stop_requested = false;
            let control = scanner::ScanControl::default();
            job.active_root = None;
            job.active = Some(control.clone());
            operation_progress.label().set_text("Checking previews…");
            operation_progress.spinner().set_spinning(true);
            operation_progress.stop().set_visible(true);
            operation_progress.present();
            spawn_thumbnail_recovery(
                photos,
                startup_paths.clone(),
                job.generation,
                sender.clone(),
                control,
            );
        })
    };
    let restored_for_recovery_fallback = startup_view_restored.clone();
    let recovery_requested_for_fallback = thumbnail_recovery_requested.clone();
    glib::timeout_add_local_once(Duration::from_secs(5), move || {
        if !restored_for_recovery_fallback.get() {
            crate::startup_trace::mark("thumbnail_recovery_fallback_request", None);
            recovery_requested_for_fallback.set(true);
        }
    });

    let parent = window.clone();
    let connection_for_import = connection.clone();
    let sidebar_refresh_for_import = availability_refresh.clone();
    let scan_job_for_import = scan_job.clone();
    let start_next_scan_for_import = start_next_scan.clone();

    sd_import_slot.replace(Some({
        let parent = parent.clone();
        let connection = connection_for_import.clone();
        let sidebar_refresh = sidebar_refresh_for_import.clone();
        let scan_job = scan_job_for_import.clone();
        let start_next_scan = start_next_scan_for_import.clone();
        let operation_progress = operation_progress.clone();
        Rc::new(move || {
            let connection = connection.clone();
            let sidebar_refresh = sidebar_refresh.clone();
            let scan_job = scan_job.clone();
            let start_next_scan = start_next_scan.clone();
            let operation_progress = operation_progress.clone();
            let imported: Rc<dyn Fn(String)> = Rc::new(move |root: String| {
                if let Err(error) = db::mark_import_root(&connection.borrow(), &root) {
                    eprintln!("Could not register imported SD-card destination {root}: {error}");
                    return;
                }
                sidebar_refresh();
                {
                    let mut job = scan_job.borrow_mut();
                    job.authorize_photo_scan(PhotoScanRequestReason::ImportFolder)
                        .expect("SD-card import is an authorized scan reason");
                    job.pending.push_back(root);
                }
                start_next_scan();
            });
            crate::sd_import::present(&parent, imported, operation_progress);
        })
    }));

    #[cfg(target_os = "linux")]
    let scan_job_for_network = scan_job_for_import.clone();
    #[cfg(target_os = "linux")]
    let start_next_scan_for_network = start_next_scan_for_import.clone();
    #[cfg(target_os = "linux")]
    let connection_for_network = connection_for_import.clone();
    #[cfg(target_os = "linux")]
    let sidebar_refresh_for_network = sidebar_refresh_for_import.clone();
    #[cfg(target_os = "linux")]
    let parent_for_network = parent.clone();

    // Local imports and onboarding cross the same registration/scan boundary.
    let queue_explicit_local_import: Rc<dyn Fn(String)> = {
        let owner_slot = Rc::downgrade(&onboarding_slot);
        let connection = connection_for_import.clone();
        let sidebar_refresh = sidebar_refresh_for_import.clone();
        let scan_job = scan_job_for_import.clone();
        let start_next_scan = start_next_scan_for_import.clone();
        Rc::new(move |root: String| {
            let owner = owner_slot.upgrade().and_then(|slot| slot.borrow().clone());
            let owner = owner.as_ref().filter(|owner| owner.is_visible());
            match onboarding::queue_local_import(&connection, &scan_job, &root, owner) {
                Ok(true) => {
                    sidebar_refresh();
                    start_next_scan();
                }
                Ok(false) => {}
                Err(error) => {
                    if let Some(owner) = owner {
                        owner.import_error(&error);
                    }
                    eprintln!("Could not import folder {root}: {error}");
                }
            }
        })
    };
    let selected_local_folder: Rc<dyn Fn(String)> = {
        let connection = connection_for_import.clone();
        let parent = parent.clone();
        let queue = queue_explicit_local_import.clone();
        Rc::new(move |root: String| {
            let exclusions = match db::excluded_folders(&connection.borrow()) {
                Ok(rules) => rules,
                Err(error) => {
                    eprintln!("Could not read excluded folders: {error}");
                    return;
                }
            };
            let blocked = exclusions.into_iter().filter(|excluded| {
                db::path_is_excluded(&root, std::slice::from_ref(excluded))
            }).collect::<Vec<_>>();
            if blocked.is_empty() {
                queue(root);
                return;
            }
            let dialog = adw::AlertDialog::builder()
                .heading("This folder is excluded")
                .body("You previously chose Remove & Exclude. Restore this folder to the library and import its photos?")
                .close_response("cancel")
                .build();
            dialog.add_response("cancel", "Keep Excluded");
            dialog.add_response("restore", "Restore & Import");
            dialog.set_response_appearance("restore", adw::ResponseAppearance::Suggested);
            let connection_for_restore = connection.clone();
            let queue_for_restore = queue.clone();
            dialog.connect_response(Some("restore"), move |_, _| {
                for excluded in &blocked {
                    if let Err(error) = db::restore_excluded_folder(&connection_for_restore.borrow(), excluded) {
                        eprintln!("Could not restore excluded folder {excluded}: {error}");
                        return;
                    }
                }
                queue_for_restore(root.clone());
            });
            dialog.present(Some(&parent));
        })
    };
    let choose_local_folder: Rc<dyn Fn()> = {
        let parent = parent.downgrade();
        let selected = selected_local_folder.clone();
        let owner_slot = Rc::downgrade(&onboarding_slot);
        Rc::new(move || {
            let Some(parent) = parent.upgrade() else {
                return;
            };
            let owner = owner_slot.upgrade().and_then(|slot| slot.borrow().clone());
            let token = owner.as_ref().and_then(|owner| owner.picker_token());
            let selected = selected.clone();
            let dialog = gtk::FileDialog::builder()
                .title("Import Local Folder")
                .accept_label("Import")
                .modal(true)
                .build();
            dialog.select_folder(Some(&parent), None::<&gio::Cancellable>, move |result| {
                if let Some(token) = token {
                    if !owner
                        .as_ref()
                        .is_some_and(|owner| owner.picker_result_is_current(token))
                    {
                        return;
                    }
                }
                match result {
                    Ok(file) => selected(crate::source::reference(&file)),
                    Err(error) => {
                        if let Some(owner) = owner.as_ref().filter(|_| token.is_some()) {
                            owner.picker_cancelled();
                            if !error.matches(gtk::DialogError::Dismissed)
                                && !error.matches(gtk::DialogError::Cancelled)
                            {
                                owner.import_error(&error.to_string());
                            }
                        }
                    }
                }
            });
        })
    };
    import_folder_slot.replace(Some(choose_local_folder.clone()));
    let onboarding = onboarding::OnboardingCoordinator::new(
        &window,
        connection.clone(),
        choose_local_folder,
        selected_local_folder,
        {
            let slot = library_navigation_slot.clone();
            let panel = refresh_status_box.clone();
            let label = refresh_status_label.clone();
            let job = scan_job.clone();
            Rc::new(move || {
                if let Some(navigate) = slot.borrow().as_ref() {
                    navigate(sidebar::SidebarFilter::All);
                }
                if job.borrow().kind == Some(ScanJobKind::Import) {
                    label.set_text("Photos are still being added. You can start browsing now.");
                    panel.set_visible(true);
                }
            })
        },
    );
    onboarding.install_library_surface(&main_stack);
    onboarding.install_library_surface(&info.root);
    onboarding.bind_scan_job(&scan_job);
    onboarding_slot.replace(Some(onboarding.clone()));
    // Only persisted unfinished jobs authorize continuation. A completed root,
    // sidebar selection, or a generic startup signal never requests discovery.
    #[cfg(target_os="linux")]
    match db::resumable_network_imports(&connection.borrow()) {
        Ok(roots) if !roots.is_empty() => {
            for root in roots { scan_job.borrow_mut().enqueue_network_import(root,true); }
            start_next_scan();
        }
        Ok(_) => {},
        Err(error) => eprintln!("Could not recover unfinished network imports: {error}"),
    }
    let getting_started = gio::SimpleAction::new("getting-started", None);
    {
        let owner = Rc::downgrade(&onboarding);
        getting_started.connect_activate(move |_, _| {
            if let Some(owner) = owner.upgrade() {
                owner.present_manually();
            }
        });
    }
    window.add_action(&getting_started);
    open_files::install(&window, &lightbox, &connection, &front);
    {
        let owner = Rc::downgrade(&onboarding);
        let presented = Rc::new(Cell::new(false));
        window.connect_map(move |_| {
            if !presented.replace(true) {
                let owner = owner.clone();
                glib::idle_add_local_once(move || {
                    if let Some(owner) = owner.upgrade() {
                        owner.present_on_startup();
                    }
                });
            }
        });
    }

    #[cfg(target_os="linux")]
    add_network_share_slot.replace(Some(Rc::new(move || {
        let scan_job=scan_job_for_network.clone();
        let start_next_scan=start_next_scan_for_network.clone();
        let connection=connection_for_network.clone();
        let sidebar_refresh=sidebar_refresh_for_network.clone();
        let queue_network_import: Rc<dyn Fn(String)> = Rc::new(move |root: String| {
            if scan_job.borrow().active_root.as_ref()==Some(&root) || scan_job.borrow().pending.contains(&root) { return; }
            if let Err(error)=db::mark_import_root(&connection.borrow(),&root){
                eprintln!("Could not register imported folder {root}: {error}");
                return;
            }
            if let Err(error)=db::register_network_import(&connection.borrow(),&root) {
                eprintln!("Could not save queued network import {root}: {error}");return;
            }
            sidebar_refresh();
            {
                let mut job=scan_job.borrow_mut();
                job.enqueue_network_import(root,true);
            }
            start_next_scan();
        });
        let selected: Rc<dyn Fn(String)> = {
            let connection = connection_for_network.clone();
            let parent = parent_for_network.clone();
            let queue = queue_network_import.clone();
            Rc::new(move |root: String| {
                let rules = match db::excluded_folders(&connection.borrow()) {
                    Ok(rules) => rules,
                    Err(error) => {
                        eprintln!("Could not read excluded folders: {error}");
                        return;
                    }
                };
                let blocked = rules.into_iter().filter(|excluded| {
                    db::path_is_excluded(&root, std::slice::from_ref(excluded))
                }).collect::<Vec<_>>();
                if blocked.is_empty() {
                    queue(root);
                    return;
                }
                let dialog = adw::AlertDialog::builder()
                    .heading("This network folder is excluded")
                    .body("You previously chose Remove & Exclude. Restore this folder and import its photos?")
                    .close_response("cancel")
                    .build();
                dialog.add_response("cancel", "Keep Excluded");
                dialog.add_response("restore", "Restore & Import");
                dialog.set_response_appearance("restore", adw::ResponseAppearance::Suggested);
                let connection_for_restore = connection.clone();
                let queue_for_restore = queue.clone();
                dialog.connect_response(Some("restore"), move |_, _| {
                    for excluded in &blocked {
                        if let Err(error) = db::restore_excluded_folder(&connection_for_restore.borrow(), excluded) {
                            eprintln!("Could not restore excluded folder {excluded}: {error}");
                            return;
                        }
                    }
                    queue_for_restore(root.clone());
                });
                dialog.present(Some(&parent));
            })
        };
        let parent_for_network=parent_for_network.clone();
        crate::network_picker::open(
            parent_for_network.upcast_ref::<gtk::Window>(),selected);
    })));

    let scan_job_for_refresh = scan_job.clone();
    let refresh_prepare_sender_for_click = refresh_prepare_sender.clone();
    let recovery_requested_for_refresh = thumbnail_recovery_requested.clone();
    let refresh_status_box_for_click = refresh_status_box.clone();
    let refresh_status_spinner_for_click = refresh_status_spinner.clone();
    let refresh_status_label_for_click = refresh_status_label.clone();
    let stop_scan_for_refresh_click = stop_scan.clone();
    let operation_progress_for_refresh_click = operation_progress.clone();

    refresh.connect_clicked(move |button| {
        
        let maintenance_was_active = scan_job_for_refresh
            .borrow()
            .kind
            == Some(ScanJobKind::Maintenance);
        if maintenance_was_active {
            scan_job_for_refresh.borrow_mut().preempt_maintenance();
            
        } else if scan_job_for_refresh.borrow().kind.is_some() {
            
            refresh_status_label_for_click.set_text("Refresh already running…");
            refresh_status_spinner_for_click.set_spinning(true);
            refresh_status_box_for_click.set_visible(true);
            stop_scan_for_refresh_click.set_visible(true);
            return;
        }
        button.set_sensitive(false);
        // Do not request a full post-refresh thumbnail recovery pass here.
        // Refresh scans already create thumbnails for changed items; a recovery
        // pass over the entire 66k-photo library immediately after repeated
        // refreshes can monopolize the app. Startup/mount recovery remains.
        recovery_requested_for_refresh.set(false);
        refresh_status_label_for_click.set_text("Refreshing library…");
        refresh_status_spinner_for_click.set_spinning(true);
        operation_progress_for_refresh_click.present();
        stop_scan_for_refresh_click.set_visible(true);
        

        let generation = {
            let mut job = scan_job_for_refresh.borrow_mut();
            job.authorize_photo_scan(PhotoScanRequestReason::ManualLibraryRefresh)
                .expect("manual library refresh is an authorized scan reason")
        };

        let sender = refresh_prepare_sender_for_click.clone();
        let database = db::active_database_path();
        std::thread::spawn(move || {
            
            let roots = database
                .and_then(|database| db::open_existing(&database))
                .and_then(|connection| db::imported_root_paths(&connection))
                .map_err(|error| error.to_string());
            
            let _ = sender.send(RefreshPrepareEvent::LibraryReady {
                generation,
                roots,
            });
        });
    });

    let onboarding_for_switch = onboarding.clone();
    let smart_scan_context = action_context.clone();
    let gallery_for_events = gallery.clone();
    let connection_for_events = connection.clone();
    let info_for_events = info.clone();
    let availability_refresh_for_events = availability_refresh.clone();
    let rebuild_folder_watches_for_events = rebuild_folder_watches.clone();
    let filter_for_events = filter.clone();
    let search_for_events = search_text.clone();
    let sort_for_events = sort.clone();
    let scan_job_for_events = scan_job.clone();
    let start_next_scan_for_events = start_next_scan.clone();
    let stop_scan_for_events = stop_scan.clone();
    let operation_progress_for_events = operation_progress.clone();
    let refresh_status_box_for_events = refresh_status_box.clone();
    let refresh_status_spinner_for_events = refresh_status_spinner.clone();
    let refresh_status_label_for_events = refresh_status_label.clone();
    let mut displayed_generation: Option<u64> = None;
    let mut scan_count: usize = 0;
    let mut pending_photos: VecDeque<db::Photo> = VecDeque::new();
    let mut thumbnail_dirty_paths = VecDeque::<std::path::PathBuf>::new();
    let mut thumbnail_dirty_seen = std::collections::HashSet::<std::path::PathBuf>::new();
    let mut priority_thumbnail_paths = Vec::new();
    let mut failure_message_shown = false;
    let mut thumbnail_total: usize = 0;
    let mut network_scan = false;
    let mut last_progress_update = Instant::now();
    let mut pending_sidebar_counts: Option<db::SidebarCounts> = None;
    let mut photos_since_sidebar_update = 0usize;
    let mut last_sidebar_count_update = Instant::now();

    glib::timeout_add_local(Duration::from_millis(50), move || {
        // Keep the callback slot alive for this window's event processing.
        let _onboarding_lifetime = &onboarding_slot;
        onboarding.scan_generation_changed(scan_job_for_events.borrow().generation);
        let ui_tick_started = Instant::now();
        // Drain event-triggered recovery requests once the current scan ends.
        // With no request, this checks only a flag and performs no disk probes.
        start_thumbnail_recovery();
        for path in crate::thumbnail::take_priority_completions() {
            crate::thumbnail_trace::event(
                &path.to_string_lossy(),
                "generation_ui_handoff",
                "delivered_to_event_loop",
            );
            priority_thumbnail_paths.push(path);
        }
        let priority_completions = priority_thumbnail_paths.len();
        let priority_pending = crate::thumbnail::priority_pending_count();
        
        while let Ok(prepare_event) = refresh_prepare_receiver.try_recv() {
            match prepare_event {
                RefreshPrepareEvent::LibraryReady {
                    generation,
                    roots,
                } => {
                    if generation != scan_job_for_events.borrow().generation {
                        continue;
                    }
                    let roots = match roots {
                        Ok(roots) => roots.into_iter().collect::<VecDeque<_>>(),
                        Err(error) => {
                            eprintln!("Could not refresh library folders: {error}");
                            let mut job = scan_job_for_events.borrow_mut();
                            job.kind = None;
                            job.active = None;
                            job.active_root = None;
                            job.pending.clear();
                            refresh_status_spinner_for_events.set_spinning(false);
                            stop_scan_for_events.set_visible(false);
                            refresh_status_label_for_events.set_text("Could not read library folders");
                            refresh_status_box_for_events.set_visible(true);
                            operation_progress_for_events.hide_after(Duration::from_millis(3000));
                            continue;
                        }
                    };
                    
                    if roots.is_empty() {
                        let mut job = scan_job_for_events.borrow_mut();
                        job.kind = None;
                        job.active = None;
                        job.active_root = None;
                        job.pending.clear();
                        refresh_status_spinner_for_events.set_spinning(false);
                        stop_scan_for_events.set_visible(false);
                        refresh_status_label_for_events.set_text("No library folders to refresh");
                        refresh_status_box_for_events.set_visible(true);
                        operation_progress_for_events.hide_after(Duration::from_millis(2500));
                        continue;
                    }
                    let should_start = {
                        let mut job = scan_job_for_events.borrow_mut();
                        if job.stop_requested {
                            job.kind = None;
                            job.active = None;
                            job.active_root = None;
                            job.pending.clear();
                            false
                        } else {
                            job.pending = roots;
                            true
                        }
                    };
                    if should_start {
                        start_next_scan_for_events();
                    } else {
                        refresh_status_spinner_for_events.set_spinning(false);
                        stop_scan_for_events.set_visible(false);
                        refresh_status_label_for_events.set_text("Library refresh stopped");
                        refresh_status_box_for_events.set_visible(true);
                        operation_progress_for_events.hide_after(Duration::from_millis(2500));
                    }
                }
                RefreshPrepareEvent::FolderReady {
                    generation,
                    path,
                    imported_root,
                } => {
                    if generation != scan_job_for_events.borrow().generation {
                        continue;
                    }
                    
                    let ok = match imported_root {
                        Ok(ok) => ok,
                        Err(error) => {
                            eprintln!("Could not validate refresh folder: {error}");
                            false
                        }
                    };
                    let should_start = {
                        let mut job = scan_job_for_events.borrow_mut();
                        if !ok || job.stop_requested {
                            job.kind = None;
                            job.active = None;
                            job.active_root = None;
                            job.pending.clear();
                            false
                        } else {
                            job.pending.clear();
                            job.pending.push_back(path);
                            true
                        }
                    };
                    if should_start {
                        start_next_scan_for_events();
                    } else {
                        refresh_status_spinner_for_events.set_spinning(false);
                        stop_scan_for_events.set_visible(false);
                        refresh_status_label_for_events.set_text("Folder refresh stopped");
                        refresh_status_box_for_events.set_visible(true);
                        operation_progress_for_events.hide_after(Duration::from_millis(2500));
                    }
                }
            }
        }

        if !network_scan && !scan_job_for_events.borrow().stop_requested && priority_pending > 0 && thumbnail_total == 0 {
            refresh_status_label_for_events
                .set_text(&format!("Creating visible thumbnails ({priority_pending} queued)"));
            refresh_status_box_for_events.set_visible(true);
        } else if !network_scan && !scan_job_for_events.borrow().stop_requested && priority_completions > 0 && thumbnail_total == 0 {
            refresh_status_box_for_events.set_visible(false);
        }
        // Drain committed batches promptly, but cap both event count and wall
        // time so input, redraws, scrolling, and Stop remain responsive.
        const MAX_EVENTS_PER_TICK: usize = 64;
        const MAX_EVENT_TIME_PER_TICK: Duration = Duration::from_millis(8);
        let mut handled_events = 0;
        let mut max_event_queue_wait = Duration::ZERO;
        while handled_events < MAX_EVENTS_PER_TICK
            && ui_tick_started.elapsed() < MAX_EVENT_TIME_PER_TICK
        {
            let Ok(ui_event) = scan_receiver.try_recv() else {
                break;
            };
            handled_events += 1;
            max_event_queue_wait = max_event_queue_wait.max(ui_event.queued_at.elapsed());

            // Events from a cancelled/superseded scan are still allowed to
            // finish in their worker threads, but they must never overwrite
            // the progress UI or completion state of the newer job.
            if ui_event.generation != scan_job_for_events.borrow().generation {
                
                continue;
            }

            if displayed_generation != Some(ui_event.generation) {
                displayed_generation = Some(ui_event.generation);
                operation_progress_for_events.show_if_not_dismissed();
                scan_count = 0;
                thumbnail_total = 0;
                failure_message_shown = false;
                pending_sidebar_counts = None;
                photos_since_sidebar_update = 0;
            }

            let event = ui_event.event;
            if scan_job_for_events.borrow().stop_requested && !matches!(&event,
                scanner::ScanEvent::Cancelled {..} | scanner::ScanEvent::Finished {..}
                | scanner::ScanEvent::PhotosIndexed {..} | scanner::ScanEvent::PhotosUpdated {..}
                | scanner::ScanEvent::FolderStarted {..} | scanner::ScanEvent::PhotosRemoved {..}
                | scanner::ScanEvent::FoldersRemoved | scanner::ScanEvent::LibraryCountsChanged {..}
                | scanner::ScanEvent::ThumbnailCreated {..}) {
                continue;
            }
            onboarding.scan_event(ui_event.generation, &event);
            match &event {
                scanner::ScanEvent::Started { root } => {
                    
                    scan_count = 0;
                    thumbnail_total = 0;
                    network_scan = root.to_str().is_some_and(|path| path.starts_with("nfs://") || path.starts_with("smb://"));
                    let is_user_job = !matches!(
                        scan_job_for_events.borrow().kind,
                        Some(ScanJobKind::Maintenance) | None
                    );
                    stop_scan_for_events.set_visible(is_user_job);

                    let folder_name = root
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_else(|| root.to_str().unwrap_or("folder"));
                    refresh_status_label_for_events.set_text(&format!("Scanning {folder_name}…"));
                    refresh_status_spinner_for_events.set_spinning(true);
                    refresh_status_box_for_events.set_visible(true);
                }

                scanner::ScanEvent::FolderStarted { folder } => {
                    
                    // Only imports should append rows immediately. Refreshing an
                    // existing library/folder after Refresh All was appending
                    // duplicate sidebar rows; right-clicking those stale rows
                    // could start another refresh against duplicated UI state.
                    if scan_job_for_events.borrow().kind == Some(ScanJobKind::Import) {
                        run_ui_guarded("sidebar folder append", || {
                            sidebar::append_folder(
                                &sidebar_for_events,
                                folder,
                                availability_refresh_for_events.clone(),
                            )
                        });
                    }
                }

                scanner::ScanEvent::DiscoveryProgress { found } => {
                    refresh_status_label_for_events
                        .set_text(&format!("Scanning… found {found} photos"));
                    refresh_status_box_for_events.set_visible(true);
                }

                scanner::ScanEvent::NetworkProgress {found,added,metadata_ready: _metadata_ready,previews_ready,catalog_complete} => {
                    let phase=if *catalog_complete {"Finishing previews"}else{"Adding photos"};
                    let (done, total) = if *catalog_complete { (*previews_ready, *found) } else { (*added, *found) };
                    refresh_status_label_for_events.set_text(&format!("{phase} · {} / {}", crate::window::format_count(done), crate::window::format_count(total)));
                    refresh_status_box_for_events.set_visible(true);
                    refresh_status_spinner_for_events.set_spinning(true);
                }

                scanner::ScanEvent::PhotosUpdated {photos} => {
                    for photo in photos {
                        if let Some(queued)=pending_photos.iter_mut().find(|queued|queued.id==photo.id) { *queued=photo.clone(); }
                    }
                    photo_changed(&smart_scan_context, PhotoChange::Indexed(photos));
                }

                scanner::ScanEvent::PhotosIndexed { photos, counts } => {
                    scan_count += photos.len();
                    photos_since_sidebar_update += photos.len();
                    pending_sidebar_counts = Some(*counts);

                    let search_active = !search_for_events.borrow().is_empty();
                    // One scope snapshot per indexed batch.
                    let scan_scope = browse_scope(
                        &connection_for_events.borrow(),
                        filter_for_events.get(),
                        &search_for_events.borrow(),
                    );
                    // In-place updates of shown photos are skipped only when a
                    // metadata filter could drop an updated photo; appending
                    // new ones needs an un-narrowed view (no toolbar rating
                    // either). The final refresh covers both.
                    let filters_active = !scan_scope.filters.is_empty();
                    let appends_new = scan_may_append_new_photos(&scan_scope);
                    if watch_trace {
                        let new = photos.iter().filter(|indexed| indexed.newly_discovered).count();
                        let matching = photos
                            .iter()
                            .filter(|indexed| {
                                indexed.newly_discovered
                                    && (matches!(filter_for_events.get(), sidebar::SidebarFilter::All)
                                        || matches!(filter_for_events.get(), sidebar::SidebarFilter::Folder(id) if Some(id) == indexed.photo.folder_id))
                            })
                            .count();
                        eprintln!(
                            "PIC_WATCH grid indexed kind={:?} filter={:?} photos={} new={new} queued_for_append={} search_active={search_active} filters_active={filters_active}",
                            scan_job_for_events.borrow().kind,
                            filter_for_events.get(),
                            photos.len(),
                            if search_active || !appends_new { 0 } else { matching },
                        );
                    }
                    if !search_active && !filters_active {
                        let mut reread = Vec::new();
                        for indexed in photos {
                            let photo = &indexed.photo;
                            if indexed.newly_discovered
                                && appends_new
                                && crate::image_format::path_is_enabled(
                                    &connection_for_events.borrow(),
                                    &photo.path,
                                )
                                && (matches!(filter_for_events.get(), sidebar::SidebarFilter::All)
                                    || matches!(filter_for_events.get(), sidebar::SidebarFilter::Folder(id) if Some(id) == photo.folder_id))
                            {
                                pending_photos.push_back(photo.clone());
                            } else if !indexed.newly_discovered {
                                // Folder refreshes deliberately avoid a full
                                // gallery rebuild: update the shown photos in place.
                                reread.push(photo.clone());
                            }
                        }
                        if !reread.is_empty() {
                            photo_changed(&smart_scan_context, PhotoChange::Indexed(&reread));
                        }
                    }

                    if !network_scan && last_progress_update.elapsed() >= Duration::from_millis(150) {
                        let text = format!("Indexed {scan_count} photos");
                        refresh_status_label_for_events.set_text(&text);
                        refresh_status_box_for_events.set_visible(true);
                        last_progress_update = Instant::now();
                    }
                }

                scanner::ScanEvent::PhotosRemoved { ids } => {
                    pending_photos.retain(|photo| !ids.contains(&photo.id));
                    photo_changed(&smart_scan_context, PhotoChange::Removed(ids));
                }
                scanner::ScanEvent::FoldersRemoved => {
                    refresh_smart_album_counts(&smart_scan_context);
                    availability_refresh_for_events();
                    if let sidebar::SidebarFilter::Folder(id) = filter_for_events.get() {
                        if !db::folder_exists(&connection_for_events.borrow(), id).unwrap_or(true) {
                            filter_for_events.set(sidebar::SidebarFilter::All);
                            restore_section_layout(
                                &connection_for_events.borrow(),
                                &gallery_for_events,
                                &info_for_events,
                                sidebar::SidebarFilter::All,
                            );
                        }
                        refresh_grid(
                            &connection_for_events,
                            filter_for_events.get(),
                            &search_for_events.borrow(),
                            sort_for_events.get(),
                            &gallery_for_events,
                        );
                    }
                }

                scanner::ScanEvent::LibraryCountsChanged { counts } => {
                    pending_sidebar_counts = Some(*counts);
                    // Reconciliation and final snapshots should be visible on
                    // the next UI tick even when no photos needed reindexing.
                    photos_since_sidebar_update = photos_since_sidebar_update.max(50);
                }

                scanner::ScanEvent::IndexingFinished { imported } => {
                    // Imported and refreshed photos can join Smart Albums.
                    refresh_smart_album_counts(&smart_scan_context);
                    if std::env::var_os("PICASA_TRACE").is_some() {
                        eprintln!(
                            "PIC_SCAN_UI indexing_finished queued_ms={} imported={} filter={:?}",
                            ui_event.queued_at.elapsed().as_millis(),
                            imported,
                            filter_for_events.get()
                        );
                    }
                    // Progressive imports append quickly; refresh jobs do one
                    // final rebuild after the whole serialized multi-folder job.
                    if !matches!(
                        scan_job_for_events.borrow().kind,
                        Some(ScanJobKind::Refresh | ScanJobKind::FolderRefresh)
                    ) {
                        refresh_grid(
                            &connection_for_events,
                            filter_for_events.get(),
                            &search_for_events.borrow(),
                            sort_for_events.get(),
                            &gallery_for_events,
                        );
                    }
                    if !network_scan {
                        refresh_status_label_for_events.set_text(&format!("Indexed {imported} photos"));
                    }
                    refresh_status_box_for_events.set_visible(true);
                }

                scanner::ScanEvent::ThumbnailsDeferred { total } => {
                    thumbnail_recovery_deferred.set(*total > 0);
                    
                }
                scanner::ScanEvent::ThumbnailsStarted { total } => {
                    if std::env::var_os("PICASA_TRACE").is_some() {
                        eprintln!(
                            "PIC_SCAN_UI thumbnails_started queued_ms={} total={total}",
                            ui_event.queued_at.elapsed().as_millis()
                        );
                    }
                    
                    scan_count = 0;
                    thumbnail_total = *total;
                    refresh_status_label_for_events
                        .set_text(&format!("Creating thumbnails 0 / {total}"));
                    refresh_status_spinner_for_events.set_spinning(true);
                    refresh_status_box_for_events.set_visible(true);
                    stop_scan_for_events.set_visible(true);
                }

                scanner::ScanEvent::ThumbnailCreated { path } => {
                    scan_count += 1;
                    crate::thumbnail_trace::event(
                        &path.to_string_lossy(),
                        "generation_ui_handoff",
                        "thumbnail_created_event_received",
                    );
                    
                    // Coalesce source paths received in this timer tick and
                    // refresh only matching realized tiles. Non-realized rows
                    // will discover the new cache entry through the background
                    // presentation loader when they bind later.
                    if thumbnail_dirty_seen.insert(path.clone()) {
                        thumbnail_dirty_paths.push_back(path.clone());
                    }
                    if !network_scan && last_progress_update.elapsed() >= Duration::from_millis(150) {
                        let text = format!("Creating thumbnails {scan_count} / {thumbnail_total}");
                        refresh_status_label_for_events.set_text(&text);
                        refresh_status_box_for_events.set_visible(true);
                        last_progress_update = Instant::now();
                    }
                }

                scanner::ScanEvent::Failed { path, error } => {
                    eprintln!("SCAN FAILED: {}: {}", path.display(), error);
                    if !failure_message_shown {
                        failure_message_shown = true;
                        refresh_status_label_for_events
                            .set_text(&format!("Some files could not be added: {error}"));
                    } else {
                        refresh_status_label_for_events.set_text("Scanning… some files failed");
                    }
                    refresh_status_box_for_events.set_visible(true);
                }

                scanner::ScanEvent::Finished { imported, failed } => {
                    
                    eprintln!(
                        "===== SCAN COMPLETE: imported={} failed={} | progressive gallery updates complete =====",
                        imported,
                        failed
                    );

                    let (kind, has_more, total_imported, total_failed) = {
                        let mut job = scan_job_for_events.borrow_mut();
                        job.active = None;
                        job.active_root = None;
                        job.imported_total += *imported;
                        job.failed_total += *failed;
                        (
                            job.kind,
                            !job.pending.is_empty(),
                            job.imported_total,
                            job.failed_total,
                        )
                    };

                    if has_more && !scan_job_for_events.borrow().stop_requested {
                        // Start exactly one next folder. Because scanner::Finished
                        // arrives after that folder's thumbnail pass, refresh
                        // progress remains serialized end-to-end.
                        start_next_scan_for_events();
                        continue;
                    }

                    // An entirely offline (or already cached) pass did no work.
                    // Report that check briefly, then leave it quiet until the
                    // next reconnect. Clear both active markers so later work
                    // can start normally.
                    if kind == Some(ScanJobKind::Maintenance) && thumbnail_total == 0 {
                        {
                            let mut job = scan_job_for_events.borrow_mut();
                            job.kind = None;
                            job.active = None;
                            job.active_root = None;
                        }
                        refresh_status_label_for_events.set_text("No previews need work");
                        refresh_status_spinner_for_events.set_spinning(false);
                        stop_scan_for_events.set_visible(false);
                        operation_progress_for_events.hide_after(Duration::from_millis(2500));
                        continue;
                    }

                    let finish_refreshes_grid = matches!(kind, Some(ScanJobKind::Refresh | ScanJobKind::FolderRefresh))
                        && (!matches!(filter_for_events.get(), sidebar::SidebarFilter::Folder(_))
                            || browse_scope(
                                &connection_for_events.borrow(),
                                filter_for_events.get(),
                                &search_for_events.borrow(),
                            )
                            .is_narrowed());
                    // Progressive appends land at the end of the Folder stream,
                    // so new photos would form a second, misplaced section for
                    // their folder. Rebuild the stream in place once the
                    // refresh added photos, keeping the visible position.
                    let finish_rebuilds_folder_stream = !finish_refreshes_grid
                        && matches!(kind, Some(ScanJobKind::Refresh | ScanJobKind::FolderRefresh))
                        && total_imported > 0
                        && matches!(filter_for_events.get(), sidebar::SidebarFilter::Folder(_));
                    if watch_trace {
                        eprintln!(
                            "PIC_WATCH grid finished kind={kind:?} imported={total_imported} filter={:?} refresh_grid={finish_refreshes_grid} folder_stream_rebuild={finish_rebuilds_folder_stream} folder_rows_redraw={}",
                            filter_for_events.get(),
                            kind == Some(ScanJobKind::FolderRefresh)
                        );
                    }
                    if finish_rebuilds_folder_stream {
                        refresh_grid_preserving_folder_viewport(
                            &connection_for_events,
                            filter_for_events.get(),
                            &search_for_events.borrow(),
                            sort_for_events.get(),
                            &gallery_for_events,
                        );
                    }
                    if finish_refreshes_grid {
                        refresh_grid(
                            &connection_for_events,
                            filter_for_events.get(),
                            &search_for_events.borrow(),
                            sort_for_events.get(),
                            &gallery_for_events,
                        );
                    }

                    if kind != Some(ScanJobKind::FolderRefresh) {
                        crate::settings::refresh_library_availability_stats(
                            connection_for_events.clone(),
                        );
                    }

                    stop_scan_for_events.set_visible(false);
                    refresh_status_spinner_for_events.set_spinning(false);
                    let message = match kind {
                        Some(ScanJobKind::Import) if thumbnail_total > 0 && total_failed == 0 => {
                            let completed = if total_imported > 0 { total_imported } else { thumbnail_total };
                            format!("✓ Previews complete · {} photos", crate::window::format_count(completed))
                        }
                        Some(ScanJobKind::Refresh) => {
                            if total_failed == 0 {
                                format!("Library refresh complete · {total_imported} photos updated")
                            } else {
                                format!(
                                    "Library refresh complete · {total_imported} updated · {total_failed} failed"
                                )
                            }
                        }
                        Some(ScanJobKind::FolderRefresh) => {
                            if total_failed == 0 {
                                format!("Folder refresh complete · {total_imported} photos updated")
                            } else {
                                format!(
                                    "Folder refresh complete · {total_imported} updated · {total_failed} failed"
                                )
                            }
                        }
                        Some(ScanJobKind::Maintenance) => {
                            if total_failed == 0 {
                                if thumbnail_recovery_deferred.get() {
                                    "Available thumbnails recovered · waiting for offline sources".to_string()
                                } else {
                                    "Thumbnail recovery complete".to_string()
                                }
                            } else {
                                format!("Thumbnail recovery complete · {total_failed} failed")
                            }
                        }
                        _ => {
                            if total_failed == 0 {
                                format!("Added {total_imported} photos")
                            } else {
                                format!("Added {total_imported} photos · {total_failed} failed")
                            }
                        }
                    };
                    scan_job_for_events.borrow_mut().kind = None;
                    if kind != Some(ScanJobKind::FolderRefresh) {
                        availability_refresh_for_events();
                    } else {
                        // A folder refresh skips the availability pass, which is
                        // the normal owner of folder-row counts, so redraw the
                        // rows from the catalog or they keep pre-scan counts.
                        if let Ok(folders) = db::folders(&connection_for_events.borrow()) {
                            sidebar::refresh_folder_rows(
                                &sidebar_for_events,
                                &folders,
                                &availability_refresh_for_events,
                            );
                        }
                    }
                    // A scan may have catalogued new subfolders of a watched
                    // folder; give them monitors too.
                    rebuild_folder_watches_for_events();
                    
                    refresh_status_label_for_events.set_text(&message);
                    refresh_status_box_for_events.set_visible(true);
                    operation_progress_for_events.hide_after(Duration::from_millis(2500));
                }

                scanner::ScanEvent::Cancelled { imported } => {
                    let kind = {
                        let mut job = scan_job_for_events.borrow_mut();
                        job.active = None;
                        job.active_root = None;
                        job.pending.clear();
                        job.imported_total += *imported;
                        let kind = job.kind;
                        job.kind = None;
                        kind
                    };
                    rebuild_folder_watches_for_events();
                    stop_scan_for_events.set_visible(false);
                    refresh_status_spinner_for_events.set_spinning(false);
                    let message = match kind {
                        Some(ScanJobKind::Refresh) => {
                            format!("Library refresh stopped · {imported} photos updated")
                        }
                        Some(ScanJobKind::FolderRefresh) => {
                            format!("Folder refresh stopped · {imported} photos updated")
                        }
                        _ => format!("Import stopped · {imported} photos added"),
                    };
                    
                    refresh_status_label_for_events.set_text(&message);
                    refresh_status_box_for_events.set_visible(true);
                    operation_progress_for_events.hide_after(Duration::from_millis(2500));
                }
            }
        }

        if displayed_generation.is_some()
            && displayed_generation != Some(scan_job_for_events.borrow().generation)
        {
            // A library switch or scan preemption invalidates already-drained
            // photo events as well as events still waiting in the channel.
            pending_photos.clear();
            pending_sidebar_counts = None;
            photos_since_sidebar_update = 0;
            displayed_generation = None;
        }
        if pending_sidebar_counts.is_some()
            && (photos_since_sidebar_update >= 50
                || last_sidebar_count_update.elapsed() >= Duration::from_millis(250))
        {
            if let Some(counts) = pending_sidebar_counts.take() {
                sidebar::update_library_counts(&sidebar_for_events, counts);
            }
            photos_since_sidebar_update = 0;
            last_sidebar_count_update = Instant::now();
        }

        const PHOTO_APPEND_BATCH: usize = 192;
        if !search_for_events.borrow().is_empty() {
            // Search results supersede progressive scan appends. The next
            // debounced refresh will replace the model from the DB.
            pending_photos.clear();
        }
        if !pending_photos.is_empty() {
            let mut batch = Vec::with_capacity(PHOTO_APPEND_BATCH.min(pending_photos.len()));
            for _ in 0..PHOTO_APPEND_BATCH {
                let Some(photo) = pending_photos.pop_front() else {
                    break;
                };
                batch.push(photo);
            }
            let append_started = Instant::now();
            let appended = batch.len();
            run_ui_guarded("photo batch append", || {
                gallery_for_events.append_photos(&batch)
            });
            if watch_trace {
                eprintln!(
                    "PIC_WATCH grid append count={appended} remaining={} filter={:?}",
                    pending_photos.len(),
                    filter_for_events.get()
                );
            }
            if std::env::var_os("PICASA_TRACE").is_some()
                && append_started.elapsed() >= Duration::from_millis(20)
            {
                eprintln!(
                    "PIC_SCAN_UI photo_batch_append count={appended} elapsed_ms={}",
                    append_started.elapsed().as_millis()
                );
            }
            
        }
        for path in priority_thumbnail_paths.drain(..) {
            if thumbnail_dirty_seen.insert(path.clone()) {
                thumbnail_dirty_paths.push_back(path);
            }
        }
        const THUMBNAIL_REFRESH_BATCH: usize = 128;
        if !thumbnail_dirty_paths.is_empty() {
            let mut paths = Vec::with_capacity(THUMBNAIL_REFRESH_BATCH.min(thumbnail_dirty_paths.len()));
            for _ in 0..THUMBNAIL_REFRESH_BATCH {
                let Some(path) = thumbnail_dirty_paths.pop_front() else {
                    break;
                };
                thumbnail_dirty_seen.remove(&path);
                paths.push(path);
            }
            run_ui_guarded("targeted thumbnail refresh", || {
                if std::env::var_os("PICASA_TRACE").is_some(){eprintln!("PIC_THUMBNAIL tile_refresh count={}",paths.len());}
                gallery_for_events.refresh_thumbnails_for_paths(&paths)
            });
            
        }

        if std::env::var_os("PICASA_TRACE").is_some()
            && (ui_tick_started.elapsed() >= Duration::from_millis(100)
                || max_event_queue_wait >= Duration::from_millis(500))
        {
            eprintln!(
                "PIC_SCAN_UI tick elapsed_ms={} events={} max_queue_ms={} pending_photos={} thumbnail_progress={}/{} priority_pending={} filter={:?}",
                ui_tick_started.elapsed().as_millis(),
                handled_events,
                max_event_queue_wait.as_millis(),
                pending_photos.len(),
                scan_count,
                thumbnail_total,
                priority_pending,
                filter_for_events.get()
            );
        }
        glib::ControlFlow::Continue
    });

    // What the window saves when it leaves a library: on close and before a
    // library switch (session_state::LeaveSnapshot).
    let leave_snapshot: Rc<dyn Fn() -> LeaveSnapshot> = {
        let connection = connection.clone();
        let filter = filter.clone();
        let gallery = gallery.clone();
        let albums_home = albums_home.clone();
        let main_split = main_split.clone();
        let window = window.clone();
        Rc::new(move || {
            let view = filter.get();
            let viewport = if view == sidebar::SidebarFilter::Albums {
                Viewport::Albums {
                    scroll: albums_home.vadjustment().value(),
                }
            } else {
                Viewport::Gallery {
                    scroll: gallery.scroll_position(),
                    anchor: gallery.viewport_center_photo().map(|photo| photo.id()),
                }
            };
            // A compact rail uses a different width. Leaving in compact mode
            // must not overwrite the normal sidebar's saved fraction.
            let mode = SidebarMode::from_setting(
                db::setting(&connection.borrow(), SIDEBAR_MODE_SETTING_KEY)
                    .ok()
                    .flatten()
                    .as_deref(),
            );
            LeaveSnapshot {
                view,
                viewport,
                window: WindowSize {
                    width: window.width(),
                    height: window.height(),
                    maximized: window.is_maximized(),
                },
                sidebar_fraction: mode
                    .is_normal()
                    .then(|| main_split.sidebar_width_fraction()),
            }
        })
    };

    switch_library_slot.replace(Some({
        let leave_snapshot = leave_snapshot.clone();
        let exit_date_browser = discard_date_browser.clone();
        let date_browser_button = date_browser_button.clone();
        let rating_filter_scope_for_switch = rating_filter_scope_slot.clone();
        let nav_fade_cancel = nav_fade_cancel.clone();
        let connection = connection.clone();
        let scan_job = scan_job.clone();
        let lightbox = lightbox.clone();
        let info = info.clone();
        let folder_cache = folder_cache.clone();
        let gallery = gallery.clone();
        let sidebar_slot = sidebar_for_unavailable.clone();
        let create_album = create_album.clone();
        let import_folder = import_folder.clone();
        let delete_album = delete_album.clone();
        let on_unavailable = availability_refresh.clone();
        let album_list = album_list.clone();
        let filter = filter.clone();
        let search_text = search_text.clone();
        let search_entry = search_entry_slot.clone();
        let sort = sort.clone();
        let group_mode = group_mode.clone();
        let last_activated_photo_id = last_activated_photo_id.clone();
        let main_stack = main_stack.clone();
        let library_home = library_home;
        let rebuild_folder_watches = rebuild_folder_watches.clone();
        let theme_engine = theme_engine.clone();
        let window = window.clone();
        let refresh_status_box = refresh_status_box.clone();
        let refresh_status_spinner = refresh_status_spinner.clone();
        let stop_scan = stop_scan.clone();
        let settings_window = settings_window_for_switch.clone();
        let metadata_panel_for_switch = Rc::downgrade(&metadata_panel);
        let start_next_scan = start_next_scan.clone();
        let albums_home = albums_home.clone();
        let gallery_surface: gtk::Widget = gallery_scroll_stack.clone().upcast();
        Rc::new(move |id: &str| -> Result<(), String> {
            let switch_started = Instant::now();
            let target = db::known_libraries()
                .map_err(|error| format!("{error:#}"))?
                .into_iter()
                .find(|library| library.id == id)
                .ok_or_else(|| "Library is no longer registered".to_owned())?;
            if !target.path.is_file() {
                return Err(format!(
                    "Library '{}' was not found at {}",
                    target.name,
                    target.path.display()
                ));
            }
            // The outgoing library keeps its view and viewport, as on close.
            if let Err(error) = record_session(
                &*connection.borrow(),
                SessionChange::Leave(leave_snapshot()),
            ) {
                eprintln!("Could not save the session before switching library: {error}");
            }

            {
                let mut job = scan_job.borrow_mut();
                if let Some(active) = job.active.take() {
                    active.interrupt();
                }
                job.generation = job.generation.wrapping_add(1);
                job.kind = None;
                job.active_root = None;
                job.pending.clear();
                job.resume_roots.clear();
                job.stop_requested = true;
            }
            onboarding_for_switch.library_changed();
            nav_fade_cancel();
            invalidate_pending_grid_navigation();
            gallery.cancel_folder_destination_hold();
            invalidate_availability_refreshes();
            refresh_status_spinner.set_spinning(false);
            refresh_status_box.set_visible(false);
            stop_scan.set_visible(false);
            lightbox.close();

            let (library, new_connection) = db::select_library(id)
                .map_err(|error| format!("{error:#}"))?;
            crate::startup_trace::mark("switch_database_open", Some(switch_started));
            db::delete_setting(&new_connection,db::filters::SETTING).map_err(|error|error.to_string())?;
            let folders = db::folders(&new_connection).map_err(|error| error.to_string())?;
            // The sidebar's whole albums; the home is counted after the
            // rating reset (AlbumChange::LibraryOpened below).
            let albums = db::albums(&new_connection).map_err(|error| error.to_string())?;
            let counts = db::sidebar_counts(&new_connection).unwrap_or_default();
            let new_session = SessionState::load(&new_connection);
            let new_filter = new_session.view(SessionEntry::LibrarySwitch, &folders, &albums);
            let new_sort = PhotoSort {
                field: SortField::from_key(
                    &db::setting(&new_connection, SORT_FIELD_SETTING_KEY)
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                ),
                direction: SortDirection::from_key(
                    &db::setting(&new_connection, SORT_DIRECTION_SETTING_KEY)
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                ),
            };
            let new_group = group_mode_from_key(
                &db::setting(&new_connection, GROUP_MODE_SETTING_KEY)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            );
            let switch_photos_started = Instant::now();
            // The new library opens unfiltered by rating (the setting is
            // reset below), so its list must not use a stale saved one.
            let switch_scope = db::BrowseScope {
                rating: RatingFilter::All,
                ..browse_scope(&new_connection, new_filter, "")
            };
            // The grid refresh's pipeline (Albums and Library list nothing).
            let switch_list = photo_list(
                &new_connection,
                &PhotoListRequest {
                    filter: new_filter,
                    scope: switch_scope,
                    sort: new_sort,
                    favorites_only: false,
                },
            )
            .unwrap_or_default();
            crate::startup_trace::mark("switch_photo_query", Some(switch_photos_started));

            // The rating filter is session-only (see startup), but it is stored
            // per library, so the new library may still hold one from earlier.
            // Reset it as startup does, or the grid stays filtered while the
            // rating picker already shows Clear and Clear can no longer fire.
            if let Err(error) = db::set_setting(
                &new_connection,
                RATING_FILTER_SETTING_KEY,
                RatingFilter::All.key(),
            ) {
                eprintln!("Could not clear photo rating filter for library switch: {error}");
            }
            exit_date_browser();
            // No refresh started for the old library may land on the new one.
            invalidate_pending_grid_navigation();
            connection.replace(new_connection);
            settings_window.menu_controls.reload(&connection.borrow());
            folder_cache.replace(folders.clone());
            info.clear_photo();
            filter.set(new_filter);
            date_browser_button.set_visible(new_filter == sidebar::SidebarFilter::All);
            date_browser_button.set_sensitive(true);
            restore_section_layout(&connection.borrow(), &gallery, &info, new_filter);
            sort.set(new_sort);
            group_mode.set(new_group);
            last_activated_photo_id.set(new_session.activated_photo);
            search_text.replace(String::new());
            if let Some(entry) = search_entry.borrow().as_ref() {
                entry.set_text("");
            }
            // Bring the session's rating picker back to Clear too (it only
            // fires when another choice was active).
            if let Some(panel) = metadata_panel_for_switch.upgrade() {
                panel.clear_rating_filter();
            }
            if let Some(sync) = rating_filter_scope_for_switch.borrow().as_ref() {
                sync();
            }
            let display_mode = sidebar::FolderDisplayMode::from_setting(
                db::setting(&connection.borrow(), sidebar::FOLDER_DISPLAY_MODE_SETTING_KEY)
                    .ok()
                    .flatten()
                    .as_deref(),
            );
            gallery.set_folder_catalog(&folders, &folder_stream_order(&folders, display_mode));
            crate::settings::CornerStyle::saved(&connection.borrow()).apply(&window);
            gallery.set_fit_whole_photo(
                crate::settings::saved_bool(
                    &connection.borrow(),
                    crate::db::THUMBNAIL_FIT_WHOLE_PHOTO_SETTING_KEY,
                )
                .unwrap_or(false),
            );
            gallery.set_show_file_names(
                crate::settings::saved_bool(
                    &connection.borrow(),
                    crate::db::THUMBNAIL_FILE_NAMES_SETTING_KEY,
                )
                .unwrap_or(false),
            );
            gallery.set_zoom_animations_enabled(db::zoom_animations_enabled(&connection.borrow()));
            gallery.set_honeycomb_border_width(crate::settings::honeycomb_border_width(
                &connection.borrow(),
            ));
            if let Some(width) = grid_thumbnail_size_from_setting(&connection.borrow()) {
                gallery.request_slider_zoom(width);
                zoom_slider.show_thumbnail_width(width);
            }
            apply_gallery_grouping(&gallery, new_filter, new_sort, new_group, true);
            show_photo_list_now(&gallery, switch_list);
            if let Some(sidebar) = sidebar_slot.borrow().as_ref() {
                sidebar::refresh(
                    sidebar,
                    &folders,
                    &albums,
                    counts,
                    create_album.clone(),
                    import_folder.clone(),
                    delete_album.clone(),
                    on_unavailable.clone(),
                );
                sidebar::set_active_filter(sidebar, new_filter);
                sidebar::apply_visibility(
                    sidebar,
                    sidebar::SidebarVisibility::from_connection(&connection.borrow()),
                );
            }
            // After the connection swap and the rating reset: nothing counted
            // for the old library lands, and the home counts the new one.
            album_list.changed(AlbumChange::LibraryOpened);
            library_home.reload(library.path.clone());
            main_stack.set_visible_child_name(if new_filter == sidebar::SidebarFilter::Library {
                "library"
            } else if new_filter == sidebar::SidebarFilter::Albums {
                "albums"
            } else {
                "photos"
            });
            theme_engine.reload_from_connection();
            rebuild_folder_watches();
            window.set_title(Some(&format!("PIC — {}", library.name)));
            settings_window.reset();
            #[cfg(target_os="linux")]
            {
                let roots=db::resumable_network_imports(&connection.borrow()).map_err(|error|error.to_string())?;
                for root in roots {scan_job.borrow_mut().enqueue_network_import(root,true);}
                start_next_scan();
            }
            // Come back to where this library was left, as startup does, once
            // its gallery is built; any navigation before then wins.
            let saved = SavedViewport {
                filter: new_filter,
                activated_photo: new_session.activated_photo,
                anchor_photo: new_session.view_anchor,
                gallery_scroll: new_session.gallery_scroll,
                albums_scroll: new_session.albums_scroll,
            };
            let generation = REFRESH_GENERATION.load(std::sync::atomic::Ordering::Relaxed);
            let gallery_for_restore = gallery.clone();
            let albums_home = albums_home.clone();
            when_gallery_built(
                gallery.clone(),
                gallery_surface.clone(),
                Rc::new(move || {
                    REFRESH_GENERATION.load(std::sync::atomic::Ordering::Relaxed) == generation
                }),
                Rc::new(move || {
                    restore_saved_viewport(&gallery_for_restore, &albums_home, saved);
                    crate::startup_trace::mark("switch_viewport_restored", Some(switch_started));
                }),
            );
            crate::startup_trace::mark("switch_complete", Some(switch_started));
            Ok(())
        })
    }));

    // Persist the browsing destination and its viewport only after the user
    // confirms Exit. Destination changes are also written immediately in
    // layout.rs, so an abnormal process termination still restores the most
    // important part of the session.
    install_close_confirmation(&window, {
        let connection = connection.clone();
        let scan_job = scan_job.clone();
        Rc::new(move || {
            if let Some(active)=scan_job.borrow().active.as_ref() {active.interrupt();}
            if let Err(error) = record_session(
                &*connection.borrow(),
                SessionChange::Leave(leave_snapshot()),
            ) {
                eprintln!("Could not save the session: {error}");
            }
        })
    });

    crate::startup_trace::mark("window_build_done", Some(build_started));
    window
}

// Renderer updates should only move focus that already belongs to the gallery.
fn gallery_owns_focus(container: &impl IsA<gtk::Widget>) -> bool {
    let container = container.as_ref();
    container.root().and_then(|root| root.focus()).is_some_and(|focus| {
        focus == *container || focus.is_ancestor(container)
    })
}

fn transfer_gallery_focus(had_gallery_focus: bool, target: &impl IsA<gtk::Widget>) {
    if had_gallery_focus {
        target.grab_focus();
    }
}

#[cfg(test)]
mod gallery_search_focus_tests {
    use super::*;

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn gallery_updates_preserve_search_focus() {
        gtk::init().expect("GTK display");
        let window = gtk::Window::new();
        let layout = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let search = gtk::SearchEntry::new();
        let gallery = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let photo = gtk::Button::with_label("Photo");
        let next_photo = gtk::Button::with_label("Next photo");
        gallery.append(&photo);
        gallery.append(&next_photo);
        layout.append(&search);
        layout.append(&gallery);
        window.set_child(Some(&layout));
        window.present();
        search.grab_focus();
        for query in ["f", "fo", "folder"] {
            search.set_text(query);
            transfer_gallery_focus(gallery_owns_focus(&gallery), &photo);
            let focus = gtk::prelude::RootExt::focus(&window).expect("focused widget");
            assert!(focus == search.clone().upcast::<gtk::Widget>() || focus.is_ancestor(&search),
                "gallery update stole focus while typing {query}");
            assert_eq!(search.text(), query);
        }
        photo.grab_focus();
        transfer_gallery_focus(gallery_owns_focus(&gallery), &next_photo);
        assert!(gallery_owns_focus(&next_photo));
        window.close();
    }
}
