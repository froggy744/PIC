//! Photo mark commands: the GTK adapter for [`photo_marks::plan`].
//!
//! Every control that changes a favourite or rating calls [`run`] with what
//! the user asked for and the photo it acts on. `run` reads the live front,
//! selection and view, asks the plan what to change, writes it in one
//! transaction, and only then refreshes the gallery, the lightbox, the
//! InfoBar, the Properties panel on screen and the sidebar counts. A failed
//! write changes nothing on screen and shows one self-dismissing notice.
//!
//! No marks are kept here: they are read from the photo objects each time.

use super::photo_marks::{
    self, MarkChange, MarkFront, MarkIntent, MarkPlan, MarkRequest, MarkView, Membership,
    PhotoMarks,
};
use super::{refresh_grid, Front, PhotoActionContext, PropertiesSurface};
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

fn set_marks(photo: &PhotoObject, change: MarkChange) {
    match change {
        MarkChange::Favorite(favorite) => photo.set_favorite(favorite),
        MarkChange::Rating(rating) => photo.set_rating(rating),
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
    let gallery = context.gallery.borrow().upgrade();
    let lightbox = context.lightbox.upgrade();
    let lightbox_shown = lightbox_shown(context);

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

    if let Some(gallery) = &gallery {
        match plan.change {
            MarkChange::Favorite(favorite) => gallery.update_favorites(&plan.ids, favorite),
            MarkChange::Rating(rating) => gallery.update_ratings(&plan.ids, rating),
        }
    }
    // The anchor and the shown photo can be objects the gallery does not own
    // (the lightbox builds its own).
    let shown = context.info.current_photo();
    for photo in [Some(anchor), shown.as_ref()].into_iter().flatten() {
        if plan.ids.contains(&photo.id()) {
            set_marks(photo, plan.change);
        }
    }
    if let Some(shown) = shown.filter(|photo| plan.ids.contains(&photo.id())) {
        show_updated(context, &shown, lightbox_shown);
    }
    update_counts(context);

    match plan.membership {
        Membership::Keep => {}
        Membership::Remove => {
            if let Some(gallery) = &gallery {
                gallery.remove_photos(&plan.ids);
            }
            if let Some(lightbox) = lightbox.as_ref().filter(|_| lightbox_shown) {
                for id in &plan.ids {
                    lightbox.remove_photo(*id);
                }
            }
        }
        Membership::Rebuild => {
            if let Some(gallery) = &gallery {
                refresh_grid(
                    &context.connection,
                    context.filter.get(),
                    &context.search.borrow(),
                    context.sort.get(),
                    gallery,
                );
                if let Some(lightbox) = lightbox.as_ref().filter(|_| lightbox_shown) {
                    for id in plan.ids.iter().filter(|id| !gallery.contains_photo(**id)) {
                        lightbox.remove_photo(*id);
                    }
                }
            }
        }
    }

    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!(
            "UI TRACE photo_mark intent={intent:?} front={front:?} ids={} change={:?} membership={:?}",
            plan.ids.len(),
            plan.change,
            plan.membership,
        );
    }
}

/// Marks changed outside [`run`] (a whole folder): reload the shown photo's
/// marks and refresh what shows them. The caller rebuilds the gallery.
pub(super) fn reload_shown_photo(context: &PhotoActionContext) {
    let Some(shown) = context.info.current_photo() else {
        return;
    };
    let Ok(Some(record)) = db::photo(&context.connection.borrow(), shown.id()) else {
        return;
    };
    shown.set_favorite(record.favorite);
    shown.set_rating(record.rating);
    show_updated(context, &shown, lightbox_shown(context));
}

fn show_updated(context: &PhotoActionContext, shown: &PhotoObject, lightbox_shown: bool) {
    context.info.photo_updated(shown);
    if let Some(properties) = context.properties.upgrade() {
        let surface = if lightbox_shown {
            PropertiesSurface::Lightbox
        } else {
            PropertiesSurface::Gallery
        };
        properties.show_photo(surface, Some(shown));
    }
}

fn update_counts(context: &PhotoActionContext) {
    let Some(sidebar) = context.sidebar.borrow().as_ref().cloned() else {
        return;
    };
    if let Ok(counts) = db::sidebar_counts(&context.connection.borrow()) {
        sidebar::update_library_counts(&sidebar, counts);
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::run;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

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
