use std::sync::mpsc::TryRecvError;

static REFRESH_GENERATION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

#[derive(Clone)]
struct FolderViewportAnchor {
    header: Option<(i64, f64)>,
    photos: Vec<(i64, f64)>,
    scroll_y: f64,
}

fn surviving_viewport_anchor(
    anchors: &[(i64, f64)],
    photo_ids: &[i64],
) -> Option<(i64, f64)> {
    anchors
        .iter()
        .find(|(id, _)| photo_ids.contains(id))
        .copied()
}

/// Invalidate any asynchronous grid result or delayed folder destination from
/// an older navigation. Folder-to-folder reuse does not start a new database
/// refresh, so it must still advance this generation to prevent an older
/// Albums/Photos -> Folder timer from pulling the view back later.
fn invalidate_pending_grid_navigation() {
    REFRESH_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn refresh_grid(
    connection: &Rc<RefCell<Connection>>,
    filter: sidebar::SidebarFilter,
    search: &str,
    sort: PhotoSort,
    gallery: &Rc<grid::Gallery>,
) {
    let prepare_started = std::time::Instant::now();
    let folder_target = if search.is_empty() {
        if let sidebar::SidebarFilter::Folder(folder_id) = filter {
            db::folders(&connection.borrow())
                .ok()
                .and_then(|folders| folders.into_iter().find(|folder| folder.id == folder_id))
                .map(|folder| (folder.id, folder.path, false))
        } else {
            None
        }
    } else {
        None
    };
    if std::env::var_os("PICASA_TRACE").is_some()
        && prepare_started.elapsed() >= std::time::Duration::from_millis(20)
    {
        eprintln!(
            "PIC_SCAN_UI refresh_prepare filter={filter:?} elapsed_ms={}",
            prepare_started.elapsed().as_millis()
        );
    }
    refresh_grid_inner(connection, filter, search, sort, gallery, folder_target, None);
}

fn refresh_grid_preserving_folder_viewport(
    connection: &Rc<RefCell<Connection>>,
    filter: sidebar::SidebarFilter,
    search: &str,
    sort: PhotoSort,
    gallery: &Rc<grid::Gallery>,
) {
    let mode = crate::image_format::raw_jpeg_pair_mode(&connection.borrow());
    let anchor = gallery.using_sectioned_folder_view().then(|| FolderViewportAnchor {
        header: gallery.capture_sectioned_folder_header_anchor(),
        photos: gallery.capture_sectioned_folder_anchors(),
        scroll_y: gallery.sectioned_folder_scroll_position(),
    });
    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!(
            "PIC_NAV raw_jpeg_refresh mode={} visible_anchors={} scroll_y={:.1}",
            mode.key(),
            anchor.as_ref().map_or(0, |anchor| anchor.photos.len()),
            anchor.as_ref().map_or(0.0, |anchor| anchor.scroll_y),
        );
    }
    refresh_grid_inner(connection, filter, search, sort, gallery, None, anchor);
}

fn refresh_grid_to_folder(
    connection: &Rc<RefCell<Connection>>,
    filter: sidebar::SidebarFilter,
    search: &str,
    sort: PhotoSort,
    gallery: &Rc<grid::Gallery>,
    folder_id: i64,
    folder_path: String,
    center_folder: bool,
) {
    refresh_grid_inner(
        connection,
        filter,
        search,
        sort,
        gallery,
        Some((folder_id, folder_path, center_folder)),
        None,
    );
}

fn refresh_grid_inner(
    connection: &Rc<RefCell<Connection>>,
    filter: sidebar::SidebarFilter,
    search: &str,
    sort: PhotoSort,
    gallery: &Rc<grid::Gallery>,
    folder_target: Option<(i64, String, bool)>,
    viewport_anchor: Option<FolderViewportAnchor>,
) {
    // Cached Folder membership belongs to the previous query/filter.
    gallery.invalidate_folder_cache();
    let rating_filter = RatingFilter::from_key(
        &db::setting(&connection.borrow(), RATING_FILTER_SETTING_KEY)
            .ok()
            .flatten()
            .unwrap_or_default(),
    );
    if filter == sidebar::SidebarFilter::Albums
        || (filter == sidebar::SidebarFilter::Library && search.is_empty())
    {
        return;
    }

    let pair_mode = crate::image_format::raw_jpeg_pair_mode(&connection.borrow());
    gallery.set_raw_jpeg_mode(pair_mode);
    let pair_folders = if search.is_empty()
        && matches!(filter, sidebar::SidebarFilter::Folder(_))
    {
        db::raw_jpeg_pair_folder_ids(&connection.borrow()).unwrap_or_default()
    } else {
        std::collections::HashSet::new()
    };
    gallery.set_raw_jpeg_pair_folders(pair_folders);

    let generation = REFRESH_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let search = search.to_owned();
    let (sender, receiver) = std::sync::mpsc::channel();
    let database = db::connection_path(&connection.borrow()).ok();

    std::thread::spawn(move || {
        let Some(database) = database else {
            let _ = sender.send(None);
            return;
        };
        let Ok(connection) = db::open_existing(&database) else {
            let _ = sender.send(None);
            return;
        };

        // Prime folder-only availability before constructing PhotoObjects on
        // the GTK thread. This checks imported roots once and never checks an
        // individual original photo path.
        let _ = db::folders(&connection);

        let folder_stream = search.is_empty()
            && matches!(filter, sidebar::SidebarFilter::Folder(_));
        let mut photos = if !search.is_empty() {
            // An active search is a library-wide view, regardless of the
            // destination that was selected before typing began.
            db::photos(&connection, None, false, Some(&search)).unwrap_or_default()
        } else if filter == sidebar::SidebarFilter::History {
            db::history_photos(&connection).unwrap_or_default()
        } else if let sidebar::SidebarFilter::Album(album_id) = filter {
            db::photos_in_album(&connection, album_id, None).unwrap_or_default()
        } else {
            let (folder_id, favorites) = match filter {
                sidebar::SidebarFilter::All | sidebar::SidebarFilter::RecentlyAdded => {
                    (None, false)
                }
                sidebar::SidebarFilter::Favorites => (None, true),
                // Picasa-style Folder mode is a single continuous stream.
                // The selected folder is a scroll destination, not a query
                // boundary, so load every indexed folder exactly once.
                sidebar::SidebarFilter::Folder(_) => (None, false),
                sidebar::SidebarFilter::Albums => return,
                sidebar::SidebarFilter::Library => return,
                sidebar::SidebarFilter::Album(_) => unreachable!(),
                sidebar::SidebarFilter::History => unreachable!(),
            };
            db::photos(&connection, folder_id, favorites, None).unwrap_or_default()
        };

        retain_enabled_formats(&connection, &mut photos);
        let raw_jpeg_mode = crate::image_format::raw_jpeg_pair_mode(&connection);
        crate::image_format::retain_raw_jpeg_pair_mode(&mut photos, raw_jpeg_mode);
        limit_recently_added(&connection, filter, &mut photos);
        apply_rating_filter(&mut photos, rating_filter);
        if folder_stream {
            let folders = db::folders(&connection).unwrap_or_default();
            let display_mode = sidebar::FolderDisplayMode::from_setting(
                db::setting(&connection, sidebar::FOLDER_DISPLAY_MODE_SETTING_KEY)
                    .ok()
                    .flatten()
                    .as_deref(),
            );
            sort_folder_stream(&mut photos, &folders, sort, display_mode);
        } else if filter != sidebar::SidebarFilter::History || !search.is_empty() {
            sort_photos(&mut photos, sort);
        }
        let _ = sender.send(Some(photos));
    });

    let gallery = gallery.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(25), move || {
        match receiver.try_recv() {
            Ok(Some(photos)) => {
                if REFRESH_GENERATION.load(std::sync::atomic::Ordering::Relaxed) == generation {
                    let replace_started = std::time::Instant::now();
                    let count = photos.len();
                    let visible_photo_ids = photos.iter().map(|photo| photo.id).collect::<Vec<_>>();
                    let restore_photo_anchor = viewport_anchor.as_ref().and_then(|anchor| {
                        surviving_viewport_anchor(&anchor.photos, &visible_photo_ids)
                    });
                    gallery.replace_owned_while_current(photos, Rc::new(move || {
                        REFRESH_GENERATION.load(std::sync::atomic::Ordering::Relaxed) == generation
                    }));
                    if std::env::var_os("PICASA_TRACE").is_some() {
                        eprintln!(
                            "PIC_SCAN_UI gallery_replace filter={filter:?} generation={generation} photos={count} elapsed_ms={}",
                            replace_started.elapsed().as_millis()
                        );
                    }
                    if let Some((folder_id, folder_path, center_folder)) = folder_target.clone() {
                        if std::env::var_os("PICASA_TRACE").is_some() {
                            eprintln!(
                                "PIC_NAV folder_target_schedule folder_id={} center={} generation={}",
                                folder_id, center_folder, generation
                            );
                        }
                        let gallery = gallery.clone();
                        // replace() may schedule a progressive model build.
                        // Start the scroll helper on the next main-loop turn so
                        // it never succeeds against the previous grid model.
                        glib::idle_add_local_once(move || {
                            scroll_gallery_to_folder_when_ready(
                                gallery,
                                folder_id,
                                folder_path,
                                generation,
                                center_folder,
                            );
                        });
                    }
                    if let Some(anchor) = viewport_anchor.clone() {
                        if std::env::var_os("PICASA_TRACE").is_some() {
                            eprintln!(
                "PIC_NAV folder_viewport_restore header={:?} photo_id={} fallback_scroll_y={:.1}",
                anchor.header,
                                restore_photo_anchor.map_or(0, |(photo_id, _)| photo_id),
                                anchor.scroll_y
                            );
                        }
                        let gallery = gallery.clone();
                        let attempts = Rc::new(Cell::new(0_u32));
                        let attempts_for_timer = attempts.clone();
                        glib::timeout_add_local(std::time::Duration::from_millis(25), move || {
                            if REFRESH_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
                                != generation
                            {
                                return glib::ControlFlow::Break;
                            }
                            let attempt = attempts_for_timer.get() + 1;
                            attempts_for_timer.set(attempt);
                            if gallery.stream_building() {
                                return if attempt < 1200 {
                                    glib::ControlFlow::Continue
                                } else {
                                    glib::ControlFlow::Break
                                };
                            }
                            let restored = if let Some(header_anchor) = anchor.header {
                                gallery.restore_sectioned_folder_header_anchor(header_anchor)
                            } else if let Some(photo_anchor) = restore_photo_anchor {
                                gallery.restore_sectioned_folder_anchor(Some(photo_anchor))
                            } else {
                                gallery.set_sectioned_folder_scroll_position(anchor.scroll_y)
                            };
                            if restored || attempt >= 240 {
                                glib::ControlFlow::Break
                            } else {
                                glib::ControlFlow::Continue
                            }
                        });
                    }
                }
                glib::ControlFlow::Break
            }
            Ok(None) | Err(TryRecvError::Disconnected) => glib::ControlFlow::Break,
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
        }
    });
}

/// Scroll a sidebar folder selection to its section once a potentially
/// progressive Folder-stream replacement has loaded far enough to contain it.
fn scroll_gallery_to_folder_when_ready(
    gallery: Rc<grid::Gallery>,
    folder_id: i64,
    folder_path: String,
    generation: u64,
    center_folder: bool,
) {
    let total_attempts = Rc::new(Cell::new(0u32));
    // Counts only the attempts made after the progressive stream finished.
    // While rows are still being built, the target folder may legitimately not
    // exist yet, so those attempts must not count toward giving up.
    let settled_attempts = Rc::new(Cell::new(0u32));
    let total_for_timer = total_attempts.clone();
    let settled_for_timer = settled_attempts.clone();
    glib::timeout_add_local(Duration::from_millis(25), move || {
        // A newer destination/refresh supersedes this timer. Without this
        // guard, an old Albums/Photos -> Folder transition can fire later and
        // pull the continuous Folder view back to the previous folder.
        if REFRESH_GENERATION.load(std::sync::atomic::Ordering::Relaxed) != generation {
            return glib::ControlFlow::Break;
        }
        total_for_timer.set(total_for_timer.get() + 1);
        // scroll_to_folder scans the whole photo model. Calling it every 25 ms
        // while the progressive Folder stream is still being built starves that
        // very build, so wait for it to finish before scanning at all.
        if gallery.stream_building() {
            if total_for_timer.get() >= 1200 {
                // Hard safety net (30 s) for a build that never completes.
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        } else {
            let revealed = if center_folder {
                gallery.scroll_to_folder_centered(folder_id, &folder_path)
            } else {
                gallery.scroll_to_folder(folder_id, &folder_path)
            };
            if revealed {
                glib::ControlFlow::Break
            } else {
                let settled = settled_for_timer.get() + 1;
                settled_for_timer.set(settled);
                if settled >= 240 {
                    // Six seconds after the Folder stream is fully built is
                    // generous; a missing/empty folder leaves the position as-is.
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            }
        }
    });
}

fn limit_recently_added(
    connection: &Connection,
    filter: sidebar::SidebarFilter,
    photos: &mut Vec<db::Photo>,
) {
    if filter != sidebar::SidebarFilter::RecentlyAdded {
        return;
    }

    // Select the newest files first, then restore the user's chosen display
    // ordering in refresh_grid().
    sort_photos(
        photos,
        PhotoSort {
            field: SortField::DateAdded,
            direction: SortDirection::Descending,
        },
    );
    photos.truncate(db::recently_added_limit(connection));
}

fn retain_enabled_formats(connection: &Connection, photos: &mut Vec<db::Photo>) {
    let enabled = crate::image_format::enabled_ids(connection).unwrap_or_else(|_| {
        crate::image_format::all()
            .iter()
            .map(|format| format.id)
            .collect()
    });
    photos.retain(|photo| crate::image_format::path_is_enabled_in(&enabled, &photo.path));
}

fn apply_rating_filter(photos: &mut Vec<db::Photo>, filter: RatingFilter) {
    if filter == RatingFilter::AllStars {
        photos.retain(|photo| (1..=5).contains(&photo.rating));
        return;
    }
    if let Some(rating) = filter.rating() {
        photos.retain(|photo| photo.rating == rating);
    }
}

fn sort_photos(photos: &mut [db::Photo], sort: PhotoSort) {
    let corrupt = corrupt_photo_ids(photos, sort.field);
    photos.sort_by(|left, right| photo_ordering(left, right, sort, &corrupt));
}

fn corrupt_photo_ids(photos: &[db::Photo], field: SortField) -> std::collections::HashSet<i64> {
    if field != SortField::Corrupt {
        return std::collections::HashSet::new();
    }
    photos.iter()
        .filter(|photo| crate::photo_object::confirmed_corrupt_local_jpeg(
            &photo.path, photo.width, photo.height,
        ))
        .map(|photo| photo.id)
        .collect()
}

fn photo_ordering(
    left: &db::Photo,
    right: &db::Photo,
    sort: PhotoSort,
    corrupt: &std::collections::HashSet<i64>,
) -> Ordering {
    let ordering = match sort.field {
        SortField::DateTaken => compare_optional(
            left.taken_at.as_deref(),
            right.taken_at.as_deref(),
            sort.direction,
        ),
        SortField::Name => directed_ordering(
            crate::source::filename(&left.path)
                .to_lowercase()
                .cmp(&crate::source::filename(&right.path).to_lowercase()),
            sort.direction,
        ),
        SortField::FileSize => compare_optional(left.size_bytes, right.size_bytes, sort.direction),
        SortField::Dimensions => compare_optional(
            pixel_count(left.width, left.height),
            pixel_count(right.width, right.height),
            sort.direction,
        ),
        SortField::DateAdded => directed_ordering(
            left.added_at
                .cmp(&right.added_at)
                .then(left.id.cmp(&right.id)),
            sort.direction,
        ),
        SortField::Rating => directed_ordering(left.rating.cmp(&right.rating), sort.direction),
        SortField::Corrupt => directed_ordering(
            corrupt.contains(&left.id).cmp(&corrupt.contains(&right.id)),
            sort.direction,
        ),
    };

    ordering.then_with(|| left.path.to_lowercase().cmp(&right.path.to_lowercase()))
}

/// Order the Folder view as one stream of direct-folder sections. Photos are
/// sorted normally inside each section, while folder sections follow the same
/// hierarchy users see in the sidebar. Imported-only mode keeps imported roots
/// together and in the same alphabetical root order as that sidebar mode.
fn sort_folder_stream(
    photos: &mut [db::Photo],
    folders: &[db::Folder],
    sort: PhotoSort,
    display_mode: sidebar::FolderDisplayMode,
) {
    let corrupt = corrupt_photo_ids(photos, sort.field);
    let order = folder_stream_order(folders, display_mode);
    let rank = order
        .iter()
        .enumerate()
        .map(|(index, folder_id)| (*folder_id, index))
        .collect::<std::collections::HashMap<_, _>>();
    photos.sort_by(|left, right| {
        let left_rank = rank
            .get(&left.folder_id.unwrap_or_default())
            .copied()
            .unwrap_or(usize::MAX);
        let right_rank = rank
            .get(&right.folder_id.unwrap_or_default())
            .copied()
            .unwrap_or(usize::MAX);
        left_rank
            .cmp(&right_rank)
            .then_with(|| photo_ordering(left, right, sort, &corrupt))
    });
}

/// Folder ids in the canonical order used by the continuous Folder gallery.
///
/// Sidebar Flat/Tree is presentation-only. The gallery must keep one stable
/// section order across that toggle; otherwise GtkListView has to splice and
/// recycle the visible row set just because the sidebar changed shape.
pub(super) fn folder_stream_order(
    folders: &[db::Folder],
    _display_mode: sidebar::FolderDisplayMode,
) -> Vec<i64> {
    folder_tree_order(folders)
}

fn folder_tree_order(folders: &[db::Folder]) -> Vec<i64> {
    let by_id = folders
        .iter()
        .map(|folder| (folder.id, folder))
        .collect::<std::collections::HashMap<_, _>>();
    let mut children = std::collections::HashMap::<Option<i64>, Vec<i64>>::new();
    for folder in folders {
        children.entry(folder.parent_id).or_default().push(folder.id);
    }
    for ids in children.values_mut() {
        ids.sort_by(|left, right| {
            let left = by_id.get(left).expect("folder id exists");
            let right = by_id.get(right).expect("folder id exists");
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.path.to_lowercase().cmp(&right.path.to_lowercase()))
        });
    }

    fn visit(
        folder_id: i64,
        children: &std::collections::HashMap<Option<i64>, Vec<i64>>,
        seen: &mut std::collections::HashSet<i64>,
        order: &mut Vec<i64>,
    ) {
        if !seen.insert(folder_id) {
            return;
        }
        order.push(folder_id);
        if let Some(ids) = children.get(&Some(folder_id)) {
            for child_id in ids {
                visit(*child_id, children, seen, order);
            }
        }
    }

    let mut order = Vec::with_capacity(folders.len());
    let mut seen = std::collections::HashSet::new();
    if let Some(roots) = children.get(&None) {
        for root_id in roots {
            visit(*root_id, &children, &mut seen, &mut order);
        }
    }
    // Corrupt/legacy parent links must not make a folder disappear from the
    // stream. Append any orphaned rows deterministically at the end.
    let mut remaining = folders
        .iter()
        .filter(|folder| !seen.contains(&folder.id))
        .collect::<Vec<_>>();
    remaining.sort_by(|left, right| left.path.to_lowercase().cmp(&right.path.to_lowercase()));
    for folder in remaining {
        visit(folder.id, &children, &mut seen, &mut order);
    }
    order
}

fn compare_optional<T: Ord>(
    left: Option<T>,
    right: Option<T>,
    direction: SortDirection,
) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => directed_ordering(left.cmp(&right), direction),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn directed_ordering(ordering: Ordering, direction: SortDirection) -> Ordering {
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}

fn pixel_count(width: Option<i64>, height: Option<i64>) -> Option<i128> {
    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            Some(i128::from(width) * i128::from(height))
        }
        _ => None,
    }
}

#[cfg(test)]
mod photo_action_tests {
    use super::{
        apply_rating_filter, folder_stream_order, sort_folder_stream, sort_photos,
        valid_file_name, wallpaper_layout, PhotoSort, RatingFilter, SortDirection, SortField,
        WallpaperLayout,
    };
    use crate::db::{Folder, Photo};

    #[test]
    fn viewport_restore_prefers_the_photo_nearest_the_viewport_center() {
        let anchors = [(42, 236.0), (43, 116.0), (44, -4.0)];
        assert_eq!(
            super::surviving_viewport_anchor(&anchors, &[10, 43, 44]),
            Some((43, 116.0))
        );
        assert_eq!(super::surviving_viewport_anchor(&anchors, &[10, 90]), None);
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn raw_jpeg_header_stays_in_place_after_search_reveal() {
        use super::*;
        fn settle(milliseconds: u64) {
            let context = glib::MainContext::default();
            let until = Instant::now() + Duration::from_millis(milliseconds);
            while Instant::now() < until {
                while context.pending() { context.iteration(false); }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        fn header(gallery: &grid::Gallery) -> Option<gtk::Label> {
            let mut child = gallery.folder_sectioned_root.first_child();
            while let Some(widget) = child {
                if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
                    if label.text().contains("DCIM") { return Some(label); }
                }
                child = widget.next_sibling();
            }
            None
        }
        gtk::init().unwrap();
        let test_dir = std::env::temp_dir().join(format!("pic-pair-viewport-{}", std::process::id()));
        std::fs::create_dir_all(&test_dir).unwrap();
        let connection = db::open(&test_dir.join("test.db")).unwrap();
        connection.execute_batch(
            "INSERT INTO folders(id,path,name,parent_id,imported_root,raw_jpeg_pair_count) VALUES
             (1,'/pair-test/Before','Before',NULL,1,0),
             (2,'/pair-test/Wickus','Wickus',NULL,1,0),
             (3,'/pair-test/Wickus/DCIM','DCIM',2,0,53),
             (4,'/pair-test/ZAfter','ZAfter',NULL,1,0);
             WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000)
             INSERT INTO photos(path,folder_id) SELECT printf('/pair-test/Before/%04d.jpg',x),1 FROM n;
             WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<53)
             INSERT INTO photos(path,folder_id) SELECT printf('/pair-test/Wickus/DCIM/%04d.jpg',x),3 FROM n;
             WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<53)
             INSERT INTO photos(path,folder_id) SELECT printf('/pair-test/Wickus/DCIM/%04d.nef',x),3 FROM n;
             WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000)
             INSERT INTO photos(path,folder_id) SELECT printf('/pair-test/ZAfter/%04d.jpg',x),4 FROM n;"
        ).unwrap();
        let connection = Rc::new(RefCell::new(connection));
        let gallery = Rc::new(grid::Gallery::new(&[], 180, |_| {}, |_, _, _| {},
            |_, _, _, _| {}, |_, _| {}, |_| {}));
        let folders = db::folders(&connection.borrow()).unwrap();
        gallery.set_folder_catalog(&folders, &[1,2,3,4]);
        gallery.set_grouping(grid::GroupMode::Folder, grid::GroupDate::Taken);
        let scroll = gtk::ScrolledWindow::builder().child(&gallery.folder_sectioned_root).build();
        gallery.attach_sectioned_folder_scroll(&scroll);
        let window = gtk::Window::builder().default_width(900).default_height(650).child(&scroll).build();
        window.present();
        let sort = PhotoSort { field: SortField::Name, direction: SortDirection::Ascending };
        crate::image_format::set_raw_jpeg_pair_mode(&connection.borrow(), crate::image_format::RawJpegPairMode::Both).unwrap();
        refresh_grid_to_folder(&connection, sidebar::SidebarFilter::Folder(2), "", sort,
            &gallery, 2, "/pair-test/Wickus".into(), true);
        let ready_deadline = Instant::now() + Duration::from_secs(5);
        while (gallery.stream_building() || header(&gallery).is_none()
            || scroll.vadjustment().value() == 0.0) && Instant::now() < ready_deadline {
            settle(25);
        }
        settle(100);
        let before = gallery.folder_sectioned_root.child_position(&header(&gallery).expect("search destination header")).1 - scroll.vadjustment().value();
        let observed = Rc::new(RefCell::new(Vec::new()));
        let observed_for_tick = observed.clone();
        let gallery_for_tick = gallery.clone();
        scroll.add_tick_callback(move |scrolled, _| {
            if let Some(label) = header(&gallery_for_tick) {
                observed_for_tick.borrow_mut().push(
                    gallery_for_tick.folder_sectioned_root.child_position(&label).1
                        - scrolled.vadjustment().value());
            }
            glib::ControlFlow::Continue
        });
        let connection_for_click = connection.clone();
        let gallery_for_click = gallery.clone();
        gallery.set_raw_jpeg_mode_changed_handler(move |mode| {
            crate::image_format::set_raw_jpeg_pair_mode(&connection_for_click.borrow(), mode).unwrap();
            refresh_grid_preserving_folder_viewport(&connection_for_click,
                sidebar::SidebarFilter::Folder(2), "", sort, &gallery_for_click);
        });
        for expected in ["JPG", "RAW", "BOTH"] {
            observed.borrow_mut().clear();
            let mut child = gallery.folder_sectioned_root.first_child();
            let mut clicked = false;
            while let Some(widget) = child {
                if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
                    if button.is_visible() {
                        button.grab_focus();
                        button.emit_clicked();
                        clicked = true;
                        break;
                    }
                }
                child = widget.next_sibling();
            }
            assert!(clicked);
            settle(500);
            let after = gallery.folder_sectioned_root.child_position(&header(&gallery).expect("header after toggle")).1 - scroll.vadjustment().value();
            assert!((after-before).abs() <= 1.0, "{expected} moved the header: {before} -> {after}");
            assert!(observed.borrow().iter().all(|position| (position-before).abs() <= 1.0),
                "{expected} visibly jumped during refresh: {:?}", observed.borrow());
        }
        window.close();
        drop(connection);
        std::fs::remove_dir_all(test_dir).unwrap();
    }

    #[test]
    fn rename_rejects_paths_and_accepts_a_file_name() {
        assert!(valid_file_name("holiday photo.jpg"));
        assert!(!valid_file_name(""));
        assert!(!valid_file_name(".."));
        assert!(!valid_file_name("folder/photo.jpg"));
    }

    #[test]
    fn wallpaper_layout_preserves_portraits_and_panorama_width() {
        assert_eq!(
            wallpaper_layout(3000, 4500, 1920, 1080),
            WallpaperLayout::PortraitBlur
        );
        assert_eq!(
            wallpaper_layout(6000, 4000, 1920, 1080),
            WallpaperLayout::Cover
        );
        assert_eq!(
            wallpaper_layout(8000, 2000, 1920, 1080),
            WallpaperLayout::PanoramaBlur
        );
    }

    fn photo(
        path: &str,
        taken_at: Option<&str>,
        size_bytes: Option<i64>,
        dimensions: Option<(i64, i64)>,
        mtime: Option<i64>,
    ) -> Photo {
        Photo {
            id: 0,
            path: path.to_string(),
            folder_id: None,
            folder_path: None,
            taken_at: taken_at.map(str::to_string),
            camera: None,
            aperture: None,
            lens: None,
            shutter_speed: None,
            iso: None,
            focal_length: None,
            exposure_bias: None,
            width: dimensions.map(|value| value.0),
            height: dimensions.map(|value| value.1),
            size_bytes,
            mtime,
            added_at: mtime.unwrap_or_default(),
            rotation: 0,
            edit_recipe: String::new(),
            favorite: false,
            rating: 0,
            trashed: false,
            history_caption: None,
            edited_at: 0,
        }
    }

    fn folder(
        id: i64,
        path: &str,
        name: &str,
        parent_id: Option<i64>,
        imported_root: bool,
    ) -> Folder {
        Folder {
            id,
            path: path.to_string(),
            name: name.to_string(),
            parent_id,
            imported_root,
            watched: false,
            photo_count: 1,
            subfolder_count: 0,
            available: true,
        }
    }

    fn photo_in_folder(path: &str, folder_id: i64, folder_path: &str) -> Photo {
        let mut value = photo(path, Some("2024-01-01 12:00:00"), None, None, None);
        value.folder_id = Some(folder_id);
        value.folder_path = Some(folder_path.to_string());
        value
    }

    #[test]
    fn folder_stream_uses_tree_section_order_not_global_photo_sort() {
        let folders = vec![
            folder(1, "/root", "root", None, true),
            folder(2, "/root/B", "B", Some(1), false),
            folder(3, "/root/A", "A", Some(1), false),
        ];
        let mut photos = vec![
            photo_in_folder("/root/B/b.jpg", 2, "/root/B"),
            photo_in_folder("/root/A/z.jpg", 3, "/root/A"),
            photo_in_folder("/root/A/a.jpg", 3, "/root/A"),
        ];

        sort_folder_stream(
            &mut photos,
            &folders,
            PhotoSort {
                field: SortField::Name,
                direction: SortDirection::Ascending,
            },
            crate::sidebar::FolderDisplayMode::Tree,
        );

        assert_eq!(
            photos
                .iter()
                .map(|photo| (photo.folder_id, crate::source::filename(&photo.path)))
                .collect::<Vec<_>>(),
            vec![
                (Some(3), "a.jpg".to_string()),
                (Some(3), "z.jpg".to_string()),
                (Some(2), "b.jpg".to_string()),
            ]
        );
    }

    #[test]
    fn sidebar_display_mode_does_not_reorder_folder_gallery() {
        let folders = vec![
            folder(10, "/Pictures", "Pictures", None, true),
            folder(11, "/Pictures/Drone", "Drone", Some(10), false),
            folder(20, "/Data", "Data", None, true),
            folder(21, "/Data/Trips", "Trips", Some(20), false),
        ];
        let mut photos = vec![
            photo_in_folder("/Pictures/Drone/p.jpg", 11, "/Pictures/Drone"),
            photo_in_folder("/Data/Trips/d.jpg", 21, "/Data/Trips"),
        ];

        let tree_order = folder_stream_order(&folders, crate::sidebar::FolderDisplayMode::Tree);
        let flat_order =
            folder_stream_order(&folders, crate::sidebar::FolderDisplayMode::ImportedOnly);
        assert_eq!(tree_order, flat_order);

        sort_folder_stream(
            &mut photos,
            &folders,
            PhotoSort {
                field: SortField::Name,
                direction: SortDirection::Ascending,
            },
            crate::sidebar::FolderDisplayMode::ImportedOnly,
        );

        assert_eq!(
            photos.iter().map(|photo| photo.folder_id).collect::<Vec<_>>(),
            vec![Some(21), Some(11)]
        );
    }

    #[test]
    fn photo_sort_supports_names_dates_sizes_dimensions_and_ratings() {
        let mut source = vec![
            photo("/photos/z.jpg", Some("2022"), Some(20), None, Some(2)),
            photo(
                "/photos/A.jpg",
                Some("2024"),
                Some(10),
                Some((6000, 4000)),
                Some(3),
            ),
            photo("/photos/m.jpg", None, Some(30), Some((3000, 2000)), Some(1)),
        ];
        source[0].rating = 1;
        source[1].rating = 5;
        source[2].rating = 3;

        let sorted_paths = |field, direction| {
            let mut photos = source.clone();
            sort_photos(&mut photos, PhotoSort { field, direction });
            photos
                .into_iter()
                .map(|photo| photo.path)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            sorted_paths(SortField::Name, SortDirection::Ascending),
            ["/photos/A.jpg", "/photos/m.jpg", "/photos/z.jpg"]
        );
        assert_eq!(
            sorted_paths(SortField::DateTaken, SortDirection::Descending),
            ["/photos/A.jpg", "/photos/z.jpg", "/photos/m.jpg"]
        );
        assert_eq!(
            sorted_paths(SortField::FileSize, SortDirection::Descending),
            ["/photos/m.jpg", "/photos/z.jpg", "/photos/A.jpg"]
        );
        assert_eq!(
            sorted_paths(SortField::Dimensions, SortDirection::Descending),
            ["/photos/A.jpg", "/photos/m.jpg", "/photos/z.jpg"]
        );
        assert_eq!(
            sorted_paths(SortField::DateAdded, SortDirection::Ascending),
            ["/photos/m.jpg", "/photos/z.jpg", "/photos/A.jpg"]
        );
        assert_eq!(
            sorted_paths(SortField::Rating, SortDirection::Descending),
            ["/photos/A.jpg", "/photos/m.jpg", "/photos/z.jpg"]
        );
        assert_eq!(
            sorted_paths(SortField::Rating, SortDirection::Ascending),
            ["/photos/z.jpg", "/photos/m.jpg", "/photos/A.jpg"]
        );
    }

    #[test]
    fn corrupt_sort_uses_the_same_status_as_the_badge() {
        let directory = std::env::temp_dir().join(format!("pic-corrupt-sort-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let good_path = directory.join("good.jpg");
        let bad_path = directory.join("bad.jpg");
        std::fs::write(&good_path, [0xff, 0xd8, 0xff, 0xe0]).unwrap();
        std::fs::write(&bad_path, b"not image data").unwrap();
        let mut good = photo(good_path.to_str().unwrap(), None, None, None, None);
        good.id = 1;
        let mut bad = photo(bad_path.to_str().unwrap(), None, None, None, None);
        bad.id = 2;
        let mut photos = vec![good, bad];
        sort_photos(&mut photos, PhotoSort { field: SortField::Corrupt, direction: SortDirection::Descending });
        assert_eq!(photos.iter().map(|photo| photo.id).collect::<Vec<_>>(), [2, 1]);
        sort_photos(&mut photos, PhotoSort { field: SortField::Corrupt, direction: SortDirection::Ascending });
        assert_eq!(photos.iter().map(|photo| photo.id).collect::<Vec<_>>(), [1, 2]);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rating_filter_supports_all_stars_and_clear() {
        let original = [0, 0, 1, 2, 5].map(|rating| {
            let mut photo = photo("/photos/test.jpg", None, None, None, None);
            photo.rating = rating;
            photo
        });
        for (filter, expected) in [
            (RatingFilter::Unrated, vec![0, 0]),
            (RatingFilter::One, vec![1]),
            (RatingFilter::Two, vec![2]),
            (RatingFilter::Three, vec![]),
            (RatingFilter::Four, vec![]),
            (RatingFilter::Five, vec![5]),
            (RatingFilter::AllStars, vec![1, 2, 5]),
            (RatingFilter::All, vec![0, 0, 1, 2, 5]),
        ] {
            let mut photos = original.to_vec();
            apply_rating_filter(&mut photos, filter);
            assert_eq!(
                photos.iter().map(|p| p.rating).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(RatingFilter::from_key(filter.key()), filter);
        }
        assert_eq!(RatingFilter::from_key("0"), RatingFilter::Unrated);
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn rating_filter_refresh_invalidates_previous_folder_membership() {
        use super::*;
        gtk::init().unwrap();
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(db::SCHEMA).unwrap();
        let connection = Rc::new(RefCell::new(connection));
        let mut photos = vec![photo("/photos/unrated.jpg", None, None, None, None),
                              photo("/photos/rated.jpg", None, None, None, None)];
        photos[0].id = 1;
        photos[1].id = 2;
        photos[1].rating = 2;
        let gallery = Rc::new(grid::Gallery::new(&[], 180, |_| {}, |_, _, _| {},
            |_, _, _, _| {}, |_, _| {}, |_| {}));
        gallery.set_grouping(grid::GroupMode::Folder, grid::GroupDate::Taken);
        gallery.replace(&photos);
        gallery.set_grouping(grid::GroupMode::None, grid::GroupDate::Taken);
        assert!(gallery.can_restore_folder_cache());
        db::set_setting(&connection.borrow(), RATING_FILTER_SETTING_KEY, "unrated").unwrap();
        // Changing the filter on a non-photo page must invalidate the hidden cache too.
        refresh_grid(&connection, sidebar::SidebarFilter::Albums, "",
            PhotoSort { field: SortField::Name, direction: SortDirection::Ascending }, &gallery);
        assert!(!gallery.can_restore_folder_cache());
        apply_rating_filter(&mut photos, RatingFilter::Unrated);
        gallery.set_grouping(grid::GroupMode::Folder, grid::GroupDate::Taken);
        gallery.replace(&photos);
        assert_eq!(gallery.photo_objects().iter().map(|p| p.id()).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn rating_filter_refresh_uses_persisted_edits_and_preserves_query_scope() {
        use crate::db;
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch(db::SCHEMA).unwrap();
        connection.execute_batch(
            "INSERT INTO folders(id,path,name) VALUES (1,'/photos','photos');
             INSERT INTO photos(id,path,folder_id,favorite,rating) VALUES
                (1,'/photos/keep.jpg',1,1,0),
                (2,'/photos/keep-rated.jpg',1,1,2),
                (3,'/photos/keep-other-folder.jpg',2,1,0),
                (4,'/photos/keep-not-favorite.jpg',1,0,0),
                (5,'/photos/different.jpg',1,1,0);
             INSERT INTO albums(id,name) VALUES (1,'album');
             INSERT INTO album_photos(album_id,photo_id) VALUES (1,1),(1,2);"
        ).unwrap();
        let refresh = || {
            let mut photos = db::photos(&connection, Some(1), true, Some("keep")).unwrap();
            apply_rating_filter(&mut photos, RatingFilter::Unrated);
            photos.iter().map(|p| p.id).collect::<Vec<_>>()
        };
        assert_eq!(refresh(), [1]);
        for rating in 1..=5 {
            db::set_rating_for_photos(&connection, &[1], rating).unwrap();
            assert!(refresh().is_empty());
            db::set_rating_for_photos(&connection, &[1], 0).unwrap();
            assert_eq!(refresh(), [1]);
        }
        db::set_rating_for_photos(&connection, &[2], 0).unwrap();
        let mut ids = refresh();
        ids.sort_unstable();
        assert_eq!(ids, [1, 2]);
        db::set_rating_for_photos(&connection, &[1], 3).unwrap();
        let mut album = db::photos_in_album(&connection, 1, None).unwrap();
        apply_rating_filter(&mut album, RatingFilter::Unrated);
        assert_eq!(album.iter().map(|p| p.id).collect::<Vec<_>>(), [2]);
    }

}
