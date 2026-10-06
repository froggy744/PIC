/// Use one progressive replacement rather than repeatedly regrouping and
/// caching the full loaded prefix after every startup append.
fn populate_startup_gallery(
    gallery: Rc<grid::Gallery>,
    photos: Rc<Vec<db::Photo>>,
    is_current: Rc<dyn Fn() -> bool>,
    restore: Rc<dyn Fn()>,
) {
    glib::idle_add_local_once(move || {
        if !is_current() {
            return;
        }
        gallery.replace_while_current(&photos, is_current.clone());
        glib::timeout_add_local(Duration::from_millis(20), move || {
            if !is_current() {
                return glib::ControlFlow::Break;
            }
            if gallery.stream_building() {
                return glib::ControlFlow::Continue;
            }
            restore();
            glib::ControlFlow::Break
        });
    });
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
        context.iteration(false);
        assert!(gallery.stream_building());
        assert_eq!(restored.get(), 0);
        let deadline = Instant::now() + Duration::from_secs(5);
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
