/// Use one progressive replacement rather than repeatedly regrouping and
/// caching the full loaded prefix after every startup append.
fn populate_startup_gallery(
    gallery: Rc<grid::Gallery>,
    photos: Rc<Vec<db::Photo>>,
    is_current: Rc<dyn Fn() -> bool>,
    restore: Rc<dyn Fn()>,
) {
    glib::idle_add_local_once(move || {
        let populate_started = Instant::now();
        crate::startup_trace::mark("gallery_population_start", None);
        if !is_current() {
            return;
        }
        gallery.replace_while_current(&photos, is_current.clone());
        crate::startup_trace::mark("gallery_population_scheduled", Some(populate_started));
        when_gallery_built(
            gallery,
            is_current,
            Rc::new(move || {
                restore();
                crate::startup_trace::mark("gallery_population_restored", Some(populate_started));
            }),
        );
    });
}

/// Run `then` once the gallery has finished building the photos it was just
/// given, unless the view changed first. A viewport restored before the build
/// ends lands on a half-built model and is lost.
fn when_gallery_built(
    gallery: Rc<grid::Gallery>,
    is_current: Rc<dyn Fn() -> bool>,
    then: Rc<dyn Fn()>,
) {
    glib::timeout_add_local(Duration::from_millis(20), move || {
        if !is_current() {
            return glib::ControlFlow::Break;
        }
        if gallery.stream_building() {
            return glib::ControlFlow::Continue;
        }
        then();
        glib::ControlFlow::Break
    });
}

/// A library's saved viewport, as left on close or before a library switch.
#[derive(Clone, Copy)]
struct SavedViewport {
    filter: sidebar::SidebarFilter,
    /// The photo last opened in the viewer.
    activated_photo: Option<i64>,
    /// The photo at the viewport centre, with the gallery scroll.
    anchor_photo: Option<i64>,
    gallery_scroll: f64,
    albums_scroll: f64,
}

/// Bring a saved viewport back on a built gallery: the albums scroll on
/// Albums; otherwise the last opened photo when this view holds it, else the
/// anchor photo and its scroll. Startup and a library switch both use it.
fn restore_saved_viewport(
    gallery: &grid::Gallery,
    albums_home: &gtk::ScrolledWindow,
    saved: SavedViewport,
) {
    if saved.filter == sidebar::SidebarFilter::Albums {
        let adjustment = albums_home.vadjustment();
        glib::timeout_add_local_once(Duration::from_millis(50), move || {
            let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
            adjustment.set_value(saved.albums_scroll.clamp(adjustment.lower(), upper));
        });
    } else if !saved
        .activated_photo
        .is_some_and(|photo_id| gallery.restore_activated_photo(photo_id))
    {
        if let Some(photo_id) = saved.anchor_photo {
            gallery.restore_view(photo_id, saved.gallery_scroll);
        }
    }
}

#[cfg(test)]
mod startup_gallery_tests {
    use super::*;
    fn fixture() -> (Rc<grid::Gallery>, Rc<Vec<db::Photo>>) {
        gtk::init().unwrap();
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(db::SCHEMA).unwrap();
        for id in 1..=4001 {
            db.execute(
                "INSERT INTO photos(id,path) VALUES (?1,?2)",
                rusqlite::params![id, format!("/startup-test/{id}.jpg")],
            )
            .unwrap();
        }
        let photos = Rc::new(db::photos(&db, None, false, None).unwrap());
        let gallery = Rc::new(grid::Gallery::new(
            &[],
            180,
            |_| {},
            |_, _, _| {},
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
        ));
        (gallery, photos)
    }
    #[test]
    #[ignore = "requires a GTK display"]
    fn startup_gallery_restores_only_after_cooperative_model_build() {
        let (gallery, photos) = fixture();
        let restored = Rc::new(Cell::new(0));
        let target = restored.clone();
        populate_startup_gallery(
            gallery.clone(),
            photos.clone(),
            Rc::new(|| true),
            Rc::new(move || target.set(target.get() + 1)),
        );
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        // GTK setup can already have unrelated idle sources queued. Wait for
        // the startup callback instead of assuming the first iteration runs it.
        while !gallery.stream_building() && restored.get() == 0 && Instant::now() < deadline {
            context.iteration(false);
        }
        assert!(gallery.stream_building());
        assert_eq!(restored.get(), 0);
        while restored.get() == 0 && Instant::now() < deadline {
            context.iteration(false);
        }
        assert_eq!(restored.get(), 1);
        assert_eq!(gallery.photo_objects().len(), photos.len());
    }
    #[test]
    #[ignore = "requires a GTK display"]
    fn startup_gallery_navigation_stops_remaining_batches_and_restoration() {
        let (gallery, photos) = fixture();
        let current = Rc::new(Cell::new(true));
        let check = current.clone();
        let restored = Rc::new(Cell::new(false));
        let target = restored.clone();
        populate_startup_gallery(
            gallery.clone(),
            photos,
            Rc::new(move || check.get()),
            Rc::new(move || target.set(true)),
        );
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        while gallery.photo_objects().is_empty() && Instant::now() < deadline {
            context.iteration(false);
        }
        let built = gallery.photo_objects().len();
        assert!(built > 0 && built < 4001);
        current.set(false);
        while gallery.stream_building() && Instant::now() < deadline {
            context.iteration(false);
        }
        assert!(!gallery.stream_building());
        assert_eq!(gallery.photo_objects().len(), built);
        assert!(!restored.get());
    }
}
