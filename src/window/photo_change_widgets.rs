//! Photo change: the GTK adapter for [`photo_change::plan`].
//!
//! Every place that changes photos writes the change, then calls
//! [`photo_changed`] once with a [`PhotoChange`] carrying the new values.
//! This module gives the values to every photo object on screen (the
//! gallery's, the viewer's own, the photo a menu acted on, the shown one),
//! then does what the plan asks: rewrite or clear the InfoBar and the
//! Properties panel in front, recount the Library and album counts, and
//! keep, trim or rebuild the photos being browsed. It is the only writer of
//! the InfoBar's photo facts and the Library counts after a photo change.

use std::collections::HashSet;

use super::photo_change::{self, ChangeKind, ChangeView, Shown};
use super::photo_marks::{MarkChange, MarkPlan, Membership};
use super::{refresh_grid, AlbumChange, Front, PhotoActionContext, PropertiesSurface};
use crate::db;
use crate::photo_object::PhotoObject;
use crate::sidebar;

/// A change already written to the database, with the values it wrote.
pub(super) enum PhotoChange<'a> {
    /// New marks; `anchor` is the photo the control acted on.
    Marks {
        plan: &'a MarkPlan,
        anchor: &'a PhotoObject,
    },
    /// New edit recipes, by photo id. A bulk edit reports each slice with
    /// `more_coming`, then its end with no recipes.
    Recipes {
        updates: &'a [(i64, String)],
        more_coming: bool,
    },
    /// The rotation was saved (a quarter turn of this photo).
    Rotated(&'a PhotoObject),
    /// The file and its path were renamed.
    Renamed(&'a PhotoObject),
    /// A tag was added to or removed from this photo.
    Tagged(i64),
    /// Moved to the system Trash.
    Trashed(&'a [i64]),
    /// The scanner re-read these photos.
    Indexed(&'a [db::Photo]),
    /// The scanner found these photos gone.
    Removed(&'a [i64]),
}

impl PhotoChange<'_> {
    fn kind(&self) -> ChangeKind {
        match self {
            Self::Marks { plan, .. } => ChangeKind::Marks(plan.membership),
            Self::Recipes { more_coming, .. } => ChangeKind::Recipes {
                more_coming: *more_coming,
            },
            Self::Rotated(_) => ChangeKind::Rotated,
            Self::Renamed(_) => ChangeKind::Renamed,
            Self::Tagged(_) => ChangeKind::Tagged,
            Self::Trashed(_) => ChangeKind::Trashed,
            Self::Indexed(_) => ChangeKind::Indexed,
            Self::Removed(_) => ChangeKind::Removed,
        }
    }

    fn ids(&self) -> Vec<i64> {
        match self {
            Self::Marks { plan, .. } => plan.ids.clone(),
            Self::Recipes { updates, .. } => updates.iter().map(|(id, _)| *id).collect(),
            Self::Rotated(photo) | Self::Renamed(photo) => vec![photo.id()],
            Self::Tagged(id) => vec![*id],
            Self::Trashed(ids) | Self::Removed(ids) => ids.to_vec(),
            Self::Indexed(records) => records.iter().map(|record| record.id).collect(),
        }
    }
}

fn lightbox_shown(context: &PhotoActionContext) -> bool {
    context
        .front
        .upgrade()
        .is_some_and(|front| front.current() == Front::Lightbox)
}

/// Show `change` everywhere: the photo objects first, then what
/// [`photo_change::plan`] asks for.
pub(super) fn photo_changed(context: &PhotoActionContext, change: PhotoChange) {
    let kind = change.kind();
    let ids = change.ids();
    let wanted: HashSet<i64> = ids.iter().copied().collect();
    let lightbox_shown = lightbox_shown(context);
    let shown = context.info.current_photo();
    let fan_out = {
        let connection = context.connection.borrow();
        let metadata = db::filters::Filters::load(&connection);
        let album_rules = match context.filter.get() {
            sidebar::SidebarFilter::Album(id) => db::album_rules(&connection, id).ok().flatten(),
            _ => None,
        };
        photo_change::plan(
            kind,
            &ChangeView {
                lightbox_shown,
                shown_changed: shown
                    .as_ref()
                    .is_some_and(|photo| wanted.contains(&photo.id())),
                sidebar: context.filter.get(),
                sort: context.sort.get().field,
                search_active: !context.search.borrow().is_empty(),
                metadata: &metadata,
                album_rules: album_rules.as_ref(),
            },
        )
    };

    give_values(context, &change, &wanted, shown.as_ref(), lightbox_shown);

    match fan_out.shown {
        Shown::Keep => {}
        Shown::Refresh(surface) => {
            if let Some(shown) = &shown {
                context.info.photo_updated(shown);
                if let Some(properties) = context.properties.upgrade() {
                    properties.show_photo(surface, Some(shown));
                }
            }
        }
        Shown::Clear => {
            context.info.clear_photo();
            if let Some(properties) = context.properties.upgrade() {
                properties.show_photo(PropertiesSurface::Gallery, None);
                properties.show_photo(PropertiesSurface::Lightbox, None);
            }
        }
    }
    if fan_out.album_counts {
        context.albums.changed(AlbumChange::PhotosChanged);
    }
    if fan_out.library_counts {
        if let Some(sidebar) = context.sidebar.borrow().as_ref() {
            if let Ok(counts) = db::sidebar_counts(&context.connection.borrow()) {
                sidebar::update_library_counts(sidebar, counts);
            }
        }
    }

    let gallery = context.gallery.borrow().upgrade();
    let lightbox = context.lightbox.upgrade();
    match fan_out.membership {
        Membership::Keep => {}
        Membership::Remove => {
            if let Some(gallery) = &gallery {
                gallery.remove_photos(&ids);
            }
            if let Some(lightbox) = &lightbox {
                for id in &ids {
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
                    for id in ids.iter().filter(|id| !gallery.contains_photo(**id)) {
                        lightbox.remove_photo(*id);
                    }
                }
            }
        }
    }

    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!(
            "UI TRACE photo_change kind={kind:?} ids={} lightbox_shown={lightbox_shown} fan_out={fan_out:?}",
            ids.len(),
        );
    }
}

/// The photo objects outside the gallery that can hold a changed photo: the
/// viewer's own (while it is shown), the shown photo and the one a menu
/// acted on.
fn other_objects(
    context: &PhotoActionContext,
    wanted: &HashSet<i64>,
    extra: &[Option<&PhotoObject>],
    lightbox_shown: bool,
) -> Vec<PhotoObject> {
    let mut objects = Vec::new();
    if lightbox_shown {
        if let Some(lightbox) = context.lightbox.upgrade() {
            objects.extend(
                lightbox
                    .photos()
                    .into_iter()
                    .filter(|photo| wanted.contains(&photo.id())),
            );
        }
    }
    for photo in extra.iter().flatten() {
        if wanted.contains(&photo.id()) && !objects.iter().any(|object| object == *photo) {
            objects.push((*photo).clone());
        }
    }
    objects
}

/// Give the change's values to every photo object that shows them, and
/// redraw the gallery tiles and the viewer's frame they change.
fn give_values(
    context: &PhotoActionContext,
    change: &PhotoChange,
    wanted: &HashSet<i64>,
    shown: Option<&PhotoObject>,
    lightbox_shown: bool,
) {
    let gallery = context.gallery.borrow().upgrade();
    let lightbox = context.lightbox.upgrade();
    let viewer_shows_a_changed_photo = || {
        lightbox_shown
            && lightbox
                .as_ref()
                .and_then(|lightbox| lightbox.current_photo())
                .is_some_and(|photo| wanted.contains(&photo.id()))
    };
    match change {
        PhotoChange::Marks { plan, anchor } => {
            if let Some(gallery) = &gallery {
                match plan.change {
                    MarkChange::Favorite(favorite) => gallery.update_favorites(&plan.ids, favorite),
                    MarkChange::Rating(rating) => gallery.update_ratings(&plan.ids, rating),
                }
            }
            for photo in other_objects(context, wanted, &[Some(anchor), shown], lightbox_shown) {
                match plan.change {
                    MarkChange::Favorite(favorite) => photo.set_favorite(favorite),
                    MarkChange::Rating(rating) => photo.set_rating(rating),
                }
            }
        }
        PhotoChange::Recipes { updates, .. } => {
            if let Some(gallery) = &gallery {
                gallery.update_edit_recipes_batch(updates);
            }
            if let Some(lightbox) = &lightbox {
                lightbox.update_edit_recipes_batch(updates);
            }
            for photo in other_objects(context, wanted, &[shown], false) {
                if let Some((_, recipe)) = updates.iter().find(|(id, _)| *id == photo.id()) {
                    photo.set_edit_recipe(recipe.clone());
                }
            }
            if viewer_shows_a_changed_photo() {
                if let Some(lightbox) = &lightbox {
                    lightbox.refresh_current_after_edit();
                }
            }
        }
        PhotoChange::Rotated(photo) | PhotoChange::Renamed(photo) => {
            // The saved row has every new fact (rotation, path, file name,
            // thumbnail cache path).
            let Ok(Some(record)) = db::photo(&context.connection.borrow(), photo.id()) else {
                return;
            };
            if let Some(gallery) = &gallery {
                gallery.update_photo(&record);
            }
            for object in other_objects(context, wanted, &[Some(photo), shown], lightbox_shown) {
                object.set_from_photo(&record);
            }
            if matches!(change, PhotoChange::Rotated(_)) {
                if let Some(gallery) = &gallery {
                    gallery.refresh_rotated_photo(&record.path);
                }
                if viewer_shows_a_changed_photo() {
                    if let Some(lightbox) = &lightbox {
                        lightbox.refresh_current();
                    }
                }
            }
        }
        PhotoChange::Indexed(records) => {
            for record in records.iter() {
                if let Some(gallery) = &gallery {
                    gallery.update_photo(record);
                }
            }
            for object in other_objects(context, wanted, &[shown], lightbox_shown) {
                if let Some(record) = records.iter().find(|record| record.id == object.id()) {
                    object.set_from_photo(record);
                }
            }
        }
        PhotoChange::Trashed(_) => {
            // The viewer may be showing a trashed photo.
            if let Some(lightbox) = &lightbox {
                lightbox.close();
            }
        }
        PhotoChange::Tagged(_) | PhotoChange::Removed(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::{photo_changed, PhotoChange};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    fn settle(until: impl Fn() -> bool) {
        let main = glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !until() && std::time::Instant::now() < deadline {
            main.iteration(false);
        }
    }

    /// A scan re-reading photos while the viewer is in front gives the new
    /// facts to the viewer's own photos and rewrites the viewer's Properties.
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn a_scan_update_reaches_the_viewer_and_its_properties() {
        gtk::init().unwrap();
        let connection = Rc::new(RefCell::new(
            rusqlite::Connection::open_in_memory().unwrap(),
        ));
        connection.borrow().execute_batch(db::SCHEMA).unwrap();
        connection
            .borrow()
            .execute_batch(
                "INSERT INTO folders(id,path,name) VALUES (1,'/photos','photos');
                 INSERT INTO photos(id,path,folder_id) VALUES
                    (1,'/photos/a.jpg',1),(2,'/photos/b.jpg',1),(3,'/photos/c.jpg',1);",
            )
            .unwrap();
        let record = |id| db::photo(&connection.borrow(), id).unwrap().unwrap();
        let object = |id| crate::photo_object::PhotoObject::from_photo(&record(id));
        let (gallery_photo, shown, neighbour) = (object(1), object(2), object(3));

        let stack = gtk::Stack::new();
        stack.add_named(
            &gtk::Box::new(gtk::Orientation::Vertical, 0),
            Some("photos"),
        );
        stack.set_visible_child_name("photos");
        let lightbox = Rc::new(crate::lightbox::Lightbox::new());
        lightbox.open(vec![shown.clone(), neighbour.clone()], 0);
        lightbox.root.set_visible(true);
        let front = FrontSurface::new(
            &stack,
            &lightbox,
            Rc::new(Cell::new(false)),
            Rc::new(RefCell::new(None)),
        );
        assert_eq!(front.current(), Front::Lightbox);
        let info = Rc::new(InfoBar::new());
        info.photo_updated(&shown);
        let properties = PropertiesWidgets::new(info.clone());
        properties.show_photo(PropertiesSurface::Gallery, Some(&gallery_photo));
        let context = PhotoActionContext {
            info,
            lightbox: Rc::downgrade(&lightbox),
            front: Rc::downgrade(&front),
            properties: Rc::downgrade(&properties),
            ..PhotoActionContext::for_tests(&connection)
        };

        let (mut shown_record, mut neighbour_record) = (record(2), record(3));
        shown_record.width = Some(4000);
        neighbour_record.width = Some(3000);
        photo_changed(
            &context,
            PhotoChange::Indexed(&[shown_record, neighbour_record]),
        );

        assert_eq!((shown.width(), neighbour.width()), (4000, 3000));
        assert_eq!(
            properties
                .photo_for_test(PropertiesSurface::Lightbox)
                .map(|photo| photo.id()),
            Some(2),
            "the viewer panel shows the re-read photo"
        );
        assert_eq!(
            properties
                .photo_for_test(PropertiesSurface::Gallery)
                .map(|photo| photo.id()),
            Some(1),
            "the gallery panel keeps its own photo"
        );
        lightbox.close();
    }

    /// Removing a tag the metadata filter narrows on takes the photo out of
    /// the gallery.
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn untagging_under_a_tag_filter_drops_the_photo() {
        gtk::init().unwrap();
        let directory = std::env::current_dir().unwrap().join(format!(
            "target/pic-photo-change-{}-{}",
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
            .execute_batch("INSERT INTO photos (id, path) VALUES (1, '/t/a.jpg'), (2, '/t/b.jpg');")
            .unwrap();
        for id in [1, 2] {
            db::scrapbook_add_tag(&connection.borrow(), id, "Trip").unwrap();
        }
        let mut filters = db::filters::Filters::default();
        filters.toggle(db::filters::Field::Tags, "Trip".into(), true);
        filters.save(&connection.borrow()).unwrap();
        let photos = [1, 2]
            .map(|id| db::photo(&connection.borrow(), id).unwrap().unwrap())
            .to_vec();
        let gallery = Rc::new(grid::Gallery::new(
            &photos,
            148,
            |_| {},
            |_, _, _| {},
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
        ));
        let context = PhotoActionContext {
            gallery: Rc::new(RefCell::new(Rc::downgrade(&gallery))),
            ..PhotoActionContext::for_tests(&connection)
        };

        db::scrapbook_remove_tag(&connection.borrow(), 1, "Trip").unwrap();
        photo_changed(&context, PhotoChange::Tagged(1));
        settle(|| !gallery.contains_photo(1) && gallery.contains_photo(2));

        assert!(
            !gallery.contains_photo(1),
            "the untagged photo left the view"
        );
        assert!(gallery.contains_photo(2));
        drop(context);
        settle(|| false);
        let _ = std::fs::remove_dir_all(directory);
    }
}
