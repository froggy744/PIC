//! Photo mark commands: the GTK adapter for [`photo_marks::plan`].
//!
//! Every control that changes a favourite or rating calls [`run`] with what
//! the user asked for and the photo it acts on. `run` reads the live front,
//! selection and view, asks the plan what to change, writes it in one
//! transaction, and only then reports it as one photo change
//! (`photo_change_widgets`), which refreshes every surface. A failed
//! write changes nothing on screen and shows one self-dismissing notice.
//!
//! No marks are kept here: they are read from the photo objects each time.

use super::photo_marks::{
    self, MarkChange, MarkFront, MarkIntent, MarkPlan, MarkRequest, MarkView, PhotoMarks,
};
use super::{photo_changed, Front, PhotoActionContext, PhotoChange};
use crate::db;
use crate::photo_object::PhotoObject;
use crate::sidebar;

fn marks_of(photo: &PhotoObject) -> PhotoMarks {
    PhotoMarks {
        id: photo.id(),
        favorite: photo.favorite(),
        rating: photo.rating(),
    }
}

fn write(connection: &rusqlite::Connection, plan: &MarkPlan) -> anyhow::Result<()> {
    match plan.change {
        MarkChange::Favorite(favorite) => {
            db::set_favorite_for_photos(connection, &plan.ids, favorite)
        }
        MarkChange::Rating(rating) => db::set_rating_for_photos(connection, &plan.ids, rating),
    }
}

/// What `intent` on `anchor` would change, without writing: the photo
/// menu's favourite label must name what its click does.
pub(super) fn preview(
    context: &PhotoActionContext,
    intent: MarkIntent,
    anchor: &PhotoObject,
) -> Option<MarkChange> {
    plan_for(context, intent, anchor).map(|(plan, _)| plan.change)
}

fn lightbox_shown(context: &PhotoActionContext) -> bool {
    context
        .front
        .upgrade()
        .is_some_and(|front| front.current() == Front::Lightbox)
}

/// Read the live front, selection and view, and ask the plan.
fn plan_for(
    context: &PhotoActionContext,
    intent: MarkIntent,
    anchor: &PhotoObject,
) -> Option<(MarkPlan, MarkFront)> {
    let gallery = context.gallery.borrow().upgrade();
    let front = if lightbox_shown(context) {
        MarkFront::Lightbox
    } else if gallery
        .as_ref()
        .is_some_and(|gallery| gallery.collage_selection_mode())
    {
        MarkFront::Collage
    } else {
        MarkFront::Gallery
    };
    let selection = match (&gallery, front) {
        (Some(gallery), MarkFront::Gallery | MarkFront::Collage) => gallery
            .selected_photos()
            .iter()
            .map(marks_of)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let metadata = db::filters::Filters::load(&context.connection.borrow());
    let album_rules = match context.filter.get() {
        sidebar::SidebarFilter::Album(id) => db::album_rules(&context.connection.borrow(), id)
            .ok()
            .flatten(),
        _ => None,
    };
    let request = MarkRequest {
        intent,
        front,
        anchor: marks_of(anchor),
        selection: &selection,
        view: MarkView {
            sidebar: context.filter.get(),
            rating: context.rating_filter.get(),
            sort: context.sort.get().field,
            metadata: &metadata,
            album_rules: album_rules.as_ref(),
        },
    };
    photo_marks::plan(&request).map(|plan| (plan, front))
}

/// Apply `intent` to `anchor`, the photo the control acts on: the photo on
/// screen for the InfoBar and digit keys, the clicked photo for the photo
/// menu.
pub(super) fn run(context: &PhotoActionContext, intent: MarkIntent, anchor: &PhotoObject) {
    let Some((plan, front)) = plan_for(context, intent, anchor) else {
        return;
    };
    if let Err(error) = write(&context.connection.borrow(), &plan) {
        let what = match plan.change {
            MarkChange::Favorite(_) => "favourite",
            MarkChange::Rating(_) => "rating",
        };
        eprintln!("Could not update {what}: {error:#}");
        // A running export or scan owns the progress bar; never end it.
        if !context.operation_progress.is_running() {
            context.operation_progress.finish(
                "Could not update photos",
                &format!("Could not update {what}: {error}"),
            );
        }
        return;
    }

    photo_changed(context, PhotoChange::Marks { plan: &plan, anchor });

    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!(
            "UI TRACE photo_mark intent={intent:?} front={front:?} ids={} change={:?} membership={:?}",
            plan.ids.len(),
            plan.change,
            plan.membership,
        );
    }
}

/// Marks changed outside [`run`] (a whole folder): re-read the shown photo
/// and show it. The caller rebuilds the gallery.
pub(super) fn reload_shown_photo(context: &PhotoActionContext) {
    let Some(shown) = context.info.current_photo() else {
        return;
    };
    let Ok(Some(record)) = db::photo(&context.connection.borrow(), shown.id()) else {
        return;
    };
    photo_changed(context, PhotoChange::Indexed(&[record]));
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::run;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn marks_recount_smart_albums_once_per_burst() {
        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-smart-recount-{}-{}",
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
            .execute_batch("INSERT INTO photos (id, path) VALUES (1, '/s/a.jpg'), (2, '/s/b.jpg');")
            .unwrap();
        let mut rules = db::filters::AlbumRules::default();
        rules.categories.insert(
            db::filters::Field::Favorite,
            db::filters::Selection { values: vec!["1".into()], ..Default::default() },
        );
        db::create_smart_album(&connection.borrow(), "Loved", &rules).unwrap();
        let photos = (1..=2)
            .map(|id| db::photo(&connection.borrow(), id).unwrap().unwrap())
            .collect::<Vec<_>>();
        let gallery = Rc::new(grid::Gallery::new(
            &photos, 148, |_| {}, |_, _, _| {}, |_, _, _, _| {}, |_, _| {}, |_| {},
        ));
        let select = |id: i64| {
            gallery.set_selected_photo_ids(&[id]);
            gallery.selected_photos().into_iter().find(|photo| photo.id() == id).unwrap()
        };
        let applied: Rc<RefCell<Vec<i64>>> = Rc::default();
        let applied_in = applied.clone();
        let context = PhotoActionContext {
            connection: connection.clone(),
            gallery: Rc::new(RefCell::new(Rc::downgrade(&gallery))),
            filter: Rc::new(Cell::new(sidebar::SidebarFilter::Albums)),
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
            albums: {
                let list = AlbumList::new(
                    connection.clone(),
                    Rc::new(Cell::new(sidebar::SidebarFilter::Albums)),
                );
                list.attach_recorders(Rc::new(|_| {}), Rc::new(move |albums: &[db::Album]| {
                    applied_in.borrow_mut().push(albums[0].photo_count);
                }));
                list
            },
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
        // Two favourites in quick succession share one worker recount, which
        // sees both.
        run(&context, MarkIntent::ToggleFavorite, &select(1));
        run(&context, MarkIntent::ToggleFavorite, &select(2));
        let main = glib::MainContext::default();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while applied.borrow().is_empty() && std::time::Instant::now() < until {
            main.iteration(false);
        }
        let settle = std::time::Instant::now() + std::time::Duration::from_millis(300);
        while std::time::Instant::now() < settle {
            main.iteration(false);
        }
        assert_eq!(*applied.borrow(), vec![2]);
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn albums_home_keeps_browsing_filters_after_edits_and_recounts() {
        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-albums-home-filters-{}-{}",
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
            .execute_batch(
                "INSERT INTO photos (id, path, favorite, rating) VALUES
                 (1, '/h/a.jpg', 1, 5), (2, '/h/b.jpg', 1, 0);",
            )
            .unwrap();
        let trips = db::create_album(&connection.borrow(), "Trips").unwrap();
        db::add_photos_to_album(&connection.borrow(), trips.id, &[1, 2]).unwrap();
        let mut rules = db::filters::AlbumRules::default();
        rules.categories.insert(
            db::filters::Field::Favorite,
            db::filters::Selection { values: vec!["1".into()], ..Default::default() },
        );
        db::create_smart_album(&connection.borrow(), "Loved", &rules).unwrap();
        // Browsing narrowed to five stars: each album holds one matching photo.
        db::set_setting(&connection.borrow(), "photo-rating-filter", "5").unwrap();

        let home: Rc<RefCell<Vec<Vec<(String, i64)>>>> = Rc::default();
        let home_in = home.clone();
        let context = PhotoActionContext {
            connection: connection.clone(),
            gallery: Rc::new(RefCell::new(std::rc::Weak::new())),
            filter: Rc::new(Cell::new(sidebar::SidebarFilter::Albums)),
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
            albums: {
                let list = AlbumList::new(
                    connection.clone(),
                    Rc::new(Cell::new(sidebar::SidebarFilter::Albums)),
                );
                list.attach_recorders(Rc::new(|_| {}), Rc::new(move |albums: &[db::Album]| {
                    home_in
                        .borrow_mut()
                        .push(albums.iter().map(|a| (a.name.clone(), a.photo_count)).collect());
                }));
                list
            },
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
        let filtered = vec![("Loved".to_string(), 1), ("Trips".to_string(), 1)];
        let main = glib::MainContext::default();
        let spin = |done: &dyn Fn() -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while !done() && std::time::Instant::now() < until {
                main.iteration(false);
            }
        };

        // An album edit refreshes the home with filtered counts.
        albums_edited(&context);
        assert_eq!(home.borrow().last(), Some(&filtered));

        // A Smart Album recount (worker) does too.
        refresh_smart_album_counts(&context);
        spin(&|| home.borrow().len() == 2);
        assert_eq!(home.borrow().len(), 2);
        assert_eq!(home.borrow().last(), Some(&filtered));

        // A recount started under the old filters never overwrites a newer
        // filter state.
        context.albums.recount_now();
        db::set_setting(&connection.borrow(), "photo-rating-filter", "").unwrap();
        let settle = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < settle {
            main.iteration(false);
        }
        assert_eq!(home.borrow().len(), 2);

        // The sidebar's unfiltered list and the manual chooser are unchanged.
        let whole = db::albums(&connection.borrow())
            .unwrap()
            .into_iter()
            .map(|a| (a.name, a.photo_count))
            .collect::<Vec<_>>();
        assert_eq!(whole, [("Loved".to_string(), 2), ("Trips".to_string(), 2)]);
        let chooser = db::manual_albums(&connection.borrow()).unwrap();
        assert_eq!(chooser.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Trips"]);
    }

    /// A library file in a fresh temporary directory.
    fn album_library(name: &str) -> Rc<RefCell<rusqlite::Connection>> {
        let directory = std::env::temp_dir().join(format!(
            "pic-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Rc::new(RefCell::new(db::open(&directory.join("library.db")).unwrap()))
    }

    /// A window-free context whose Albums home records what it is given.
    fn album_context(
        connection: Rc<RefCell<rusqlite::Connection>>,
        home: Rc<RefCell<Vec<Vec<(String, i64)>>>>,
    ) -> PhotoActionContext {
        PhotoActionContext {
            connection: connection.clone(),
            gallery: Rc::new(RefCell::new(std::rc::Weak::new())),
            filter: Rc::new(Cell::new(sidebar::SidebarFilter::Albums)),
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
            albums: {
                let list = AlbumList::new(
                    connection.clone(),
                    Rc::new(Cell::new(sidebar::SidebarFilter::Albums)),
                );
                list.attach_recorders(Rc::new(|_| {}), Rc::new(move |albums: &[db::Album]| {
                    home.borrow_mut()
                        .push(albums.iter().map(|a| (a.name.clone(), a.photo_count)).collect());
                }));
                list
            },
            navigate_to_folder: Rc::new(|_, _| {}),
            open_collage: Rc::new(|_| {}),
            open_edit: Rc::new(|_| {}),
            edit_clipboard: Rc::new(RefCell::new(None)),
            export: Rc::new(|_| {}),
            window: glib::WeakRef::new(),
            context_menu_host: Rc::new(RefCell::new(None)),
            operation_progress: OperationProgressUi::new(),
            properties: std::rc::Weak::new(),
        }
    }

    fn spin_for(millis: u64) {
        let main = glib::MainContext::default();
        let until = std::time::Instant::now() + std::time::Duration::from_millis(millis);
        while std::time::Instant::now() < until {
            main.iteration(false);
        }
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn an_older_recount_never_overwrites_a_newer_album_edit() {
        gtk::init().unwrap();
        let connection = album_library("older-recount");
        connection
            .borrow()
            .execute_batch("INSERT INTO photos (id, path, favorite) VALUES (1, '/r/a.jpg', 1), (2, '/r/b.jpg', 1);")
            .unwrap();
        let trips = db::create_album(&connection.borrow(), "Trips").unwrap();
        db::add_photos_to_album(&connection.borrow(), trips.id, &[1, 2]).unwrap();
        let mut rules = db::filters::AlbumRules::default();
        rules.categories.insert(
            db::filters::Field::Favorite,
            db::filters::Selection { values: vec!["1".into()], ..Default::default() },
        );
        db::create_smart_album(&connection.borrow(), "Loved", &rules).unwrap();
        let home: Rc<RefCell<Vec<Vec<(String, i64)>>>> = Rc::default();
        let context = album_context(connection.clone(), home.clone());

        // A recount is counting on its worker when an album is deleted.
        context.albums.recount_now();
        db::delete_album(&connection.borrow(), trips.id).unwrap();
        albums_edited(&context);
        assert_eq!(home.borrow().last(), Some(&vec![("Loved".to_string(), 2)]));
        spin_for(600);
        // The edit is newer: the older recount's lists are dropped.
        assert_eq!(home.borrow().len(), 1, "an older recount landed: {:?}", home.borrow());
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn add_to_album_lists_the_open_librarys_manual_albums() {
        gtk::init().unwrap();
        let first = album_library("add-menu-first");
        db::create_album(&first.borrow(), "Old library album").unwrap();
        let context = album_context(first, Rc::default());
        configure_infobar_album_menu(&context.info.add_to_album, context.clone());

        // Another library is opened behind the same window.
        let second = album_library("add-menu-second");
        db::create_album(&second.borrow(), "New library album").unwrap();
        let mut rules = db::filters::AlbumRules::default();
        rules.rating = "5".into();
        db::create_smart_album(&second.borrow(), "Smart source", &rules).unwrap();
        let second = Rc::try_unwrap(second).ok().unwrap().into_inner();
        context.connection.replace(second);

        let window = gtk::Window::new();
        window.set_default_size(1200, 200);
        window.set_child(Some(&context.info.root));
        window.present();
        spin_for(200);
        context.info.add_to_album.set_visible(true);
        context.info.add_to_album.popup();
        spin_for(200);
        let labels = context
            .info
            .add_to_album
            .popover()
            .map(|popover| {
                let mut labels = Vec::new();
                let mut pending = vec![popover.upcast::<gtk::Widget>()];
                while let Some(widget) = pending.pop() {
                    if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                        if let Some(label) = button.label() {
                            labels.push(label.to_string());
                        }
                    }
                    let mut child = widget.first_child();
                    while let Some(next) = child {
                        child = next.next_sibling();
                        pending.push(next);
                    }
                }
                labels
            })
            .unwrap_or_default();
        context.info.add_to_album.popdown();
        // Manual albums of the open library only: Smart Albums are sources.
        assert!(labels.iter().any(|label| label == "New library album"), "{labels:?}");
        assert!(!labels.iter().any(|label| label == "Old library album"), "{labels:?}");
        assert!(!labels.iter().any(|label| label == "Smart source"), "{labels:?}");
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn marks_change_a_whole_selection_or_nothing() {
        gtk::init().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "pic-photo-marks-{}-{}",
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
            .execute_batch(
                "INSERT INTO photos (id, path, favorite) VALUES
                 (1, '/marks/a.jpg', 1), (2, '/marks/b.jpg', 0), (3, '/marks/c.jpg', 0);",
            )
            .unwrap();
        let photos = (1..=3)
            .map(|id| db::photo(&connection.borrow(), id).unwrap().unwrap())
            .collect::<Vec<_>>();
        let gallery = Rc::new(grid::Gallery::new(
            &photos,
            148,
            |_| {},
            |_, _, _| {},
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
        ));
        gallery.set_selected_photo_ids(&[1, 2]);
        let object = |id| {
            gallery
                .selected_photos()
                .into_iter()
                .find(|photo| photo.id() == id)
        };
        let anchor = object(1).unwrap();
        let second = object(2).unwrap();
        let filter = Rc::new(Cell::new(sidebar::SidebarFilter::All));
        let context = PhotoActionContext {
            connection: connection.clone(),
            gallery: Rc::new(RefCell::new(Rc::downgrade(&gallery))),
            filter: filter.clone(),
            search: Rc::new(RefCell::new(String::new())),
            sort: Rc::new(Cell::new(PhotoSort {
                field: SortField::DateTaken,
                direction: SortDirection::Descending,
            })),
            rating_filter: Rc::new(Cell::new(RatingFilter::All)),
            info: {
                // The InfoBar owns the current photo.
                let info = Rc::new(InfoBar::new());
                info.photo_updated(&anchor);
                info
            },
            lightbox: std::rc::Weak::new(),
            front: std::rc::Weak::new(),
            sidebar: Rc::new(RefCell::new(None)),
            create_album: Rc::new(|| {}),
            import_folder: Rc::new(|| {}),
            delete_album: Rc::new(|_| {}),
            on_unavailable: Rc::new(|| {}),
            albums: AlbumList::new(connection.clone(), Rc::new(Cell::new(sidebar::SidebarFilter::All))),
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
        let stored = |id| {
            let photo = db::photo(&connection.borrow(), id).unwrap().unwrap();
            (photo.favorite, photo.rating)
        };

        // A mixed selection is added, then removed once all are favourites.
        run(&context, MarkIntent::ToggleFavorite, &anchor);
        assert_eq!((stored(1).0, stored(2).0, stored(3).0), (true, true, false));
        assert!(anchor.favorite() && second.favorite());
        run(&context, MarkIntent::ToggleFavorite, &anchor);
        assert_eq!((stored(1).0, stored(2).0), (false, false));
        assert!(!anchor.favorite() && !second.favorite());

        // Stars set a rating, then clear it when the selection shares it.
        run(&context, MarkIntent::ToggleRating(3), &anchor);
        assert_eq!((stored(1).1, stored(2).1, stored(3).1), (3, 3, 0));
        assert_eq!((anchor.rating(), second.rating()), (3, 3));
        run(&context, MarkIntent::ToggleRating(3), &anchor);
        assert_eq!((stored(1).1, stored(2).1), (0, 0));
        run(&context, MarkIntent::SetRating(4), &anchor);
        run(&context, MarkIntent::SetRating(4), &anchor);
        assert_eq!((stored(1).1, stored(2).1), (4, 4), "digits never toggle");

        // A failed write changes neither the library nor the objects.
        connection
            .borrow()
            .execute_batch(
                "CREATE TRIGGER refuse_two BEFORE UPDATE ON photos WHEN NEW.id = 2
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
        run(&context, MarkIntent::ToggleFavorite, &anchor);
        assert_eq!((stored(1).0, stored(2).0), (false, false));
        assert!(!anchor.favorite() && !second.favorite());
        connection
            .borrow()
            .execute_batch("DROP TRIGGER refuse_two;")
            .unwrap();

        // Unfavouriting in Favourites takes the photos out of the gallery.
        run(&context, MarkIntent::ToggleFavorite, &anchor);
        filter.set(sidebar::SidebarFilter::Favorites);
        run(&context, MarkIntent::ToggleFavorite, &anchor);
        assert_eq!((stored(1).0, stored(2).0), (false, false));
        assert!(!gallery.contains_photo(1) && !gallery.contains_photo(2));
        assert!(gallery.contains_photo(3));

        drop(context);
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
