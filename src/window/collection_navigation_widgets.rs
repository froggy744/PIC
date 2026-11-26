//! Collection navigation: the GTK adapter for [`collection_navigation`].
//!
//! Up/Down in the viewer calls [`CollectionNavigation::navigate`]. It reads
//! the sidebar's visible destinations and the current one on the GTK thread,
//! then a worker asks the plan for the next collection with photos (each
//! checked through the shared photo-list pipeline under its own browse
//! scope) and builds the target's list. Back on the GTK thread a result is
//! dropped when it is no longer current (the destination or search changed,
//! the viewer closed, another library opened); otherwise the destination is
//! entered as a sidebar click enters it, the gallery shows the list, and once
//! the gallery holds it the viewer opens on the landing photo. The viewer
//! keeps showing its photo until then. Presses during a navigation are
//! queued and run one after another.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use super::collection_navigation::{self, landing, FolderRows, Land, Target};
use super::{
    apply_gallery_grouping, photo_list, photo_list_has_photos, show_photo_list_now, Front,
    FrontSurface, PhotoList, PhotoListRequest, PhotoSort,
};
use crate::db;
use crate::grid;
use crate::lightbox::Lightbox;
use crate::sidebar::SidebarFilter;

/// What the adapter reads and drives.
pub(super) struct CollectionNavigationParts {
    pub(super) connection: Rc<RefCell<rusqlite::Connection>>,
    pub(super) filter: Rc<Cell<SidebarFilter>>,
    pub(super) search: Rc<RefCell<String>>,
    pub(super) sort: Rc<Cell<PhotoSort>>,
    pub(super) group_mode: Rc<Cell<grid::GroupMode>>,
    pub(super) gallery: Rc<grid::Gallery>,
    pub(super) lightbox: Rc<Lightbox>,
    pub(super) front: Rc<FrontSurface>,
    /// The sidebar's visible destinations, in order.
    pub(super) sidebar_destinations: Rc<dyn Fn() -> Vec<SidebarFilter>>,
    /// The sidebar click's destination bookkeeping (`enter_destination`).
    pub(super) enter_destination: Rc<dyn Fn(SidebarFilter)>,
    /// Scroll the sidebar to a folder row the viewer moved to.
    pub(super) reveal_folder_row: Rc<dyn Fn(i64)>,
}

pub(super) struct CollectionNavigation {
    inner: Rc<Inner>,
}

struct Inner {
    parts: CollectionNavigationParts,
    /// A navigation is between its press and the viewer opening.
    running: Cell<bool>,
    /// Presses made meanwhile: net steps, positive forward.
    queued: Cell<i32>,
}

/// A plain-value snapshot for the worker.
struct Request {
    database: PathBuf,
    sidebar: Vec<SidebarFilter>,
    current: SidebarFilter,
    forward: bool,
    sort: PhotoSort,
    /// In Folder mode: the viewer's photos (the Folder stream), as
    /// (photo id, folder id) in order, and the shown photo's folder.
    viewer_stream: Option<(Vec<(i64, i64)>, Option<i64>)>,
}

struct Outcome {
    target: Target,
    /// The target's list for the gallery; `None` when the viewer already
    /// holds it (moving between folder rows inside the Folder stream).
    list: Option<PhotoList>,
    /// The photo the viewer opens on.
    land_id: i64,
}

/// Steps queued at most, so a held key cannot run on for long.
const MAX_QUEUED: i32 = 8;

impl CollectionNavigation {
    pub(super) fn new(parts: CollectionNavigationParts) -> Self {
        Self {
            inner: Rc::new(Inner {
                parts,
                running: Cell::new(false),
                queued: Cell::new(0),
            }),
        }
    }

    /// Move the viewer to the previous (`direction < 0`) or next collection.
    pub(super) fn navigate(&self, direction: i32) {
        if direction == 0 {
            return;
        }
        let inner = &self.inner;
        if inner.running.get() {
            inner
                .queued
                .set((inner.queued.get() + direction.signum()).clamp(-MAX_QUEUED, MAX_QUEUED));
            return;
        }
        Inner::start(inner.clone(), direction > 0);
    }
}

impl Inner {
    /// Up/Down moves between collections only in the viewer and never while
    /// searching (every destination then shows the same search results).
    fn may_navigate(&self) -> bool {
        self.parts.search.borrow().is_empty() && self.parts.front.current() == Front::Lightbox
    }

    fn database(&self) -> Option<PathBuf> {
        db::connection_path(&self.parts.connection.borrow()).ok()
    }

    fn start(self: Rc<Self>, forward: bool) {
        if !self.may_navigate() {
            self.queued.set(0);
            return;
        }
        let Some(database) = self.database() else {
            return;
        };
        let current = self.parts.filter.get();
        // In Folder mode the viewer holds the whole Folder stream; where it
        // is in it decides the current folder row.
        let viewer_stream = matches!(current, SidebarFilter::Folder(_)).then(|| {
            let photos = self.parts.lightbox.photos();
            let stream = photos
                .iter()
                .map(|photo| (photo.id(), photo.folder_id()))
                .collect::<Vec<_>>();
            let shown = self
                .parts
                .lightbox
                .current_photo()
                .map(|photo| photo.folder_id());
            (stream, shown)
        });
        let request = Request {
            database: database.clone(),
            sidebar: (self.parts.sidebar_destinations)(),
            current,
            forward,
            sort: self.parts.sort.get(),
            viewer_stream,
        };
        self.running.set(true);
        glib::MainContext::default().spawn_local(async move {
            let outcome = gio::spawn_blocking(move || plan(request))
                .await
                .ok()
                .flatten();
            let still_current = |inner: &Inner| {
                inner.may_navigate()
                    && inner.parts.filter.get() == current
                    && inner.database().as_ref() == Some(&database)
            };
            let Some(outcome) = outcome.filter(|_| still_current(&self)) else {
                self.finish(false);
                return;
            };
            self.show(outcome, database);
        });
    }

    /// Enter the target, show its list, then open the viewer once the
    /// gallery holds it.
    fn show(self: Rc<Self>, outcome: Outcome, database: PathBuf) {
        let parts = &self.parts;
        let Target { page, land } = outcome.target;
        trace(&format!(
            "show target={page:?} land={land:?} photos={:?}",
            outcome.list.as_ref().map(|list| list.photos.len())
        ));
        (parts.enter_destination)(page);
        if let Some(list) = outcome.list {
            apply_gallery_grouping(
                &parts.gallery,
                page,
                parts.sort.get(),
                parts.group_mode.get(),
                true,
            );
            parts.gallery.invalidate_folder_cache();
            show_photo_list_now(&parts.gallery, list);
        }
        if let SidebarFilter::Folder(row) = page {
            (parts.reveal_folder_row)(row);
        }
        let land_id = outcome.land_id;
        let attempts = Cell::new(0_u32);
        glib::timeout_add_local(std::time::Duration::from_millis(25), move || {
            let parts = &self.parts;
            let current = self.may_navigate()
                && parts.filter.get() == page
                && self.database().as_ref() == Some(&database);
            if !current {
                self.finish(false);
                return glib::ControlFlow::Break;
            }
            if parts.gallery.stream_building() {
                attempts.set(attempts.get() + 1);
                if attempts.get() < 1200 {
                    return glib::ControlFlow::Continue;
                }
                self.finish(false);
                return glib::ControlFlow::Break;
            }
            // The gallery's own objects; inside the Folder stream the
            // viewer's are the same photos.
            let objects = [parts.gallery.photo_objects(), parts.lightbox.photos()]
                .into_iter()
                .find(|objects| objects.iter().any(|photo| photo.id() == land_id));
            let Some((objects, index)) = objects.and_then(|objects| {
                let index = objects.iter().position(|photo| photo.id() == land_id)?;
                Some((objects, index))
            }) else {
                self.finish(false);
                return glib::ControlFlow::Break;
            };
            parts.gallery.select_photo(land_id);
            parts.lightbox.open(objects, index);
            self.finish(true);
            glib::ControlFlow::Break
        });
    }

    /// The navigation ended: run the next queued press, or forget the queue
    /// when it did not land.
    fn finish(self: &Rc<Self>, landed: bool) {
        self.running.set(false);
        let queued = self.queued.get();
        if !landed || queued == 0 {
            self.queued.set(0);
            return;
        }
        self.queued.set(queued - queued.signum());
        Inner::start(self.clone(), queued > 0);
    }
}

/// The worker's part: the next collection with photos and its list.
fn plan(request: Request) -> Option<Outcome> {
    let connection = db::open_existing(&request.database).ok()?;
    // Prime folder availability before photo objects are built (as the grid
    // refresh does).
    let folders = db::folders(&connection).unwrap_or_default();
    let page_request =
        |page: SidebarFilter| PhotoListRequest::for_view(&connection, page, "", request.sort);
    let shown_rows = request
        .sidebar
        .iter()
        .filter_map(|page| match page {
            SidebarFilter::Folder(id) => Some(*id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let rows = FolderRows::new(
        &shown_rows,
        &folders
            .iter()
            .map(|folder| (folder.id, folder.parent_id))
            .collect::<Vec<_>>(),
    );
    // The Folder stream as (photo id, folder row), in order: the viewer's
    // when it holds it, else listed once when the walk first reaches a
    // folder row.
    let mut stream: Option<(Vec<(i64, Option<i64>)>, Option<PhotoList>)> =
        request.viewer_stream.as_ref().map(|(photos, _)| {
            let photos = photos
                .iter()
                .map(|(id, folder)| (*id, rows.row_of(*folder)))
                .collect();
            (photos, None)
        });
    let load_stream = || {
        let list =
            photo_list(&connection, &page_request(SidebarFilter::Folder(0))).unwrap_or_default();
        let photos = list
            .photos
            .iter()
            .map(|photo| {
                (
                    photo.id,
                    photo.folder_id.and_then(|folder| rows.row_of(folder)),
                )
            })
            .collect();
        (photos, Some(list))
    };
    let current = match &request.viewer_stream {
        Some((_, Some(folder))) => rows
            .row_of(*folder)
            .map(SidebarFilter::Folder)
            .unwrap_or(request.current),
        _ => request.current,
    };
    let target =
        collection_navigation::next_stop(&request.sidebar, current, request.forward, |page| {
            match page {
                SidebarFilter::Folder(row) => stream
                    .get_or_insert_with(load_stream)
                    .0
                    .iter()
                    .any(|(_, photo_row)| *photo_row == Some(row)),
                page => photo_list_has_photos(&connection, &page_request(page)),
            }
        })?;
    if let SidebarFilter::Folder(row) = target.page {
        let (photos, list) = stream?;
        let rows_in_order = photos.iter().map(|(_, row)| *row).collect::<Vec<_>>();
        let index = landing(&rows_in_order, row, target.land)?;
        return Some(Outcome {
            target,
            list,
            land_id: photos[index].0,
        });
    }
    let list = photo_list(&connection, &page_request(target.page))?;
    let land_id = match target.land {
        Land::First => list.photos.first(),
        Land::Last => list.photos.last(),
    }?
    .id;
    Some(Outcome {
        target,
        list: Some(list),
        land_id,
    })
}

fn trace(message: &str) {
    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!("UI TRACE collection_navigation {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::{CollectionNavigation, CollectionNavigationParts};
    use sidebar::SidebarFilter::{self, *};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    struct Fixture {
        directory: std::path::PathBuf,
        connection: Rc<RefCell<rusqlite::Connection>>,
        filter: Rc<Cell<SidebarFilter>>,
        search: Rc<RefCell<String>>,
        gallery: Rc<grid::Gallery>,
        lightbox: Rc<crate::lightbox::Lightbox>,
        entered: Rc<RefCell<Vec<SidebarFilter>>>,
        navigation: CollectionNavigation,
        _front: Rc<FrontSurface>,
        _stack: gtk::Stack,
    }

    /// A library on disk (the worker opens its own connection) with the
    /// viewer open on All Photos.
    fn fixture(name: &str, setup: &str, sidebar: Vec<SidebarFilter>) -> Fixture {
        gtk::init().unwrap();
        let directory = std::env::current_dir().unwrap().join(format!(
            "target/pic-collection-nav-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let connection = Rc::new(RefCell::new(
            db::open(&directory.join("library.db")).unwrap(),
        ));
        connection.borrow().execute_batch(setup).unwrap();
        let filter = Rc::new(Cell::new(All));
        let search = Rc::new(RefCell::new(String::new()));
        let sort = Rc::new(Cell::new(PhotoSort {
            field: SortField::Name,
            direction: SortDirection::Ascending,
        }));
        let gallery = Rc::new(grid::Gallery::new(
            &[],
            148,
            |_| {},
            |_, _, _| {},
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
        ));
        let list = photo_list(
            &connection.borrow(),
            &PhotoListRequest::for_view(&connection.borrow(), All, "", sort.get()),
        )
        .unwrap();
        show_photo_list_now(&gallery, list);
        let stack = gtk::Stack::new();
        stack.add_named(
            &gtk::Box::new(gtk::Orientation::Vertical, 0),
            Some("photos"),
        );
        stack.set_visible_child_name("photos");
        let lightbox = Rc::new(crate::lightbox::Lightbox::new());
        lightbox.open(gallery.photo_objects(), 0);
        lightbox.root.set_visible(true);
        let front = FrontSurface::new(
            &stack,
            &lightbox,
            Rc::new(Cell::new(false)),
            Rc::new(RefCell::new(None)),
        );
        assert_eq!(front.current(), Front::Lightbox);
        let entered: Rc<RefCell<Vec<SidebarFilter>>> = Rc::default();
        let navigation = CollectionNavigation::new(CollectionNavigationParts {
            connection: connection.clone(),
            filter: filter.clone(),
            search: search.clone(),
            sort,
            group_mode: Rc::new(Cell::new(grid::GroupMode::None)),
            gallery: gallery.clone(),
            lightbox: lightbox.clone(),
            front: front.clone(),
            sidebar_destinations: Rc::new(move || sidebar.clone()),
            enter_destination: {
                let (filter, entered) = (filter.clone(), entered.clone());
                Rc::new(move |page| {
                    filter.set(page);
                    entered.borrow_mut().push(page);
                })
            },
            reveal_folder_row: Rc::new(|_| {}),
        });
        Fixture {
            directory,
            connection,
            filter,
            search,
            gallery,
            lightbox,
            entered,
            navigation,
            _front: front,
            _stack: stack,
        }
    }

    impl Fixture {
        fn settle(&self, until: impl Fn() -> bool) {
            let main = glib::MainContext::default();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !until() && std::time::Instant::now() < deadline {
                main.iteration(false);
            }
        }

        fn shown(&self) -> Option<i64> {
            self.lightbox.current_photo().map(|photo| photo.id())
        }

        /// Press, then wait until the viewer shows `page` on `photo`.
        fn press(&self, direction: i32, page: SidebarFilter, photo: i64) {
            self.navigation.navigate(direction);
            self.settle(|| self.filter.get() == page && self.shown() == Some(photo));
            assert_eq!(
                (self.filter.get(), self.shown()),
                (page, Some(photo)),
                "pressed {direction}"
            );
        }

        /// Let any running navigation finish.
        fn idle(&self) {
            let until = std::time::Instant::now() + std::time::Duration::from_millis(300);
            self.settle(|| std::time::Instant::now() >= until);
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.lightbox.close();
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    const LIBRARY: &str = "
        INSERT INTO folders (id, path, name) VALUES (1, '/n/one', 'one'), (2, '/n/two', 'two');
        INSERT INTO photos (id, path, folder_id, favorite) VALUES
            (1, '/n/one/a.jpg', 1, 0), (2, '/n/one/b.jpg', 1, 1),
            (3, '/n/two/c.jpg', 2, 0), (4, '/n/two/d.jpg', 2, 0);";

    /// Up/Down follows the sidebar, skips empty collections, wraps, lands on
    /// the first photo going forward and the last going back, and opens the
    /// viewer on the gallery's own photos.
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn viewer_up_and_down_walk_the_sidebar_collections() {
        let library = fixture("walk", LIBRARY, vec![All, Favorites, Ratings, Album(1)]);
        let album = db::create_album(&library.connection.borrow(), "Trip").unwrap();
        assert_eq!(album.id, 1);
        db::add_photos_to_album(&library.connection.borrow(), album.id, &[3, 4]).unwrap();

        library.press(1, Favorites, 2);
        // Ratings has no photos: skipped.
        library.press(1, Album(1), 3);
        library.press(1, All, 1);
        library.press(-1, Album(1), 4);
        library.press(-1, Favorites, 2);
        assert_eq!(
            *library.entered.borrow(),
            [Favorites, Album(1), All, Album(1), Favorites]
        );
        let gallery_objects = library.gallery.photo_objects();
        assert!(
            library
                .lightbox
                .photos()
                .iter()
                .zip(&gallery_objects)
                .all(|(a, b)| a == b),
            "the viewer holds the gallery's photo objects"
        );

        // Two quick presses run one after the other.
        library.navigation.navigate(1);
        library.navigation.navigate(1);
        library.settle(|| library.filter.get() == All && library.shown() == Some(1));
        assert_eq!((library.filter.get(), library.shown()), (All, Some(1)));
    }

    /// Nothing moves during a search, with one collection, or when the
    /// destination changes before the result arrives.
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn viewer_up_and_down_drop_stale_and_needless_moves() {
        let library = fixture("stale", LIBRARY, vec![All, Favorites]);

        library.search.replace("a".into());
        library.navigation.navigate(1);
        library.idle();
        library.search.replace(String::new());
        assert_eq!(
            (library.filter.get(), library.shown()),
            (All, Some(1)),
            "no move while searching"
        );

        // The destination changes while the worker runs: its result is dropped.
        library.navigation.navigate(1);
        library.filter.set(Ratings);
        library.idle();
        assert_eq!(library.filter.get(), Ratings);
        assert!(
            library.entered.borrow().is_empty(),
            "a stale result is not shown"
        );

        // Favourites emptied: All Photos is the only collection left.
        library.filter.set(All);
        library
            .connection
            .borrow()
            .execute_batch("UPDATE photos SET favorite = 0;")
            .unwrap();
        library.navigation.navigate(1);
        library.idle();
        assert_eq!((library.filter.get(), library.shown()), (All, Some(1)));
        assert!(library.entered.borrow().is_empty());
    }

    /// Folder rows are sections of the one Folder stream: landing on one
    /// shows the stream on that row's first (or last) photo, Up/Down inside
    /// Folder mode moves between rows in the viewer's stream, a photo in a
    /// folder without a shown row belongs to its nearest shown ancestor, and
    /// the sequence wraps past the last row.
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn viewer_up_and_down_move_through_the_folder_stream() {
        let library = fixture(
            "folders",
            "INSERT INTO folders (id, path, name, parent_id) VALUES
                (1, '/f/one', 'one', NULL), (2, '/f/two', 'two', NULL),
                (3, '/f/two/sub', 'sub', 2);
             INSERT INTO photos (id, path, folder_id) VALUES
                (1, '/f/one/a.jpg', 1), (2, '/f/one/b.jpg', 1),
                (3, '/f/two/c.jpg', 2), (4, '/f/two/d.jpg', 2),
                (5, '/f/two/sub/e.jpg', 3);",
            vec![All, Album(1), Folder(1), Folder(2)],
        );
        let album = db::create_album(&library.connection.borrow(), "Trip").unwrap();
        db::add_photos_to_album(&library.connection.borrow(), album.id, &[3, 4]).unwrap();

        library.press(1, Album(1), 3);
        library.press(1, Folder(1), 1);
        let stream = library
            .lightbox
            .photos()
            .iter()
            .map(|photo| photo.id())
            .collect::<Vec<_>>();
        assert_eq!(
            stream,
            [1, 2, 3, 4, 5],
            "the viewer holds the whole Folder stream"
        );
        library.press(1, Folder(2), 3);
        assert_eq!(
            library.lightbox.photos().len(),
            5,
            "moving inside the stream keeps it"
        );
        library.press(-1, Folder(1), 2);
        library.press(-1, Album(1), 4);
        library.press(1, Folder(1), 1);
        library.press(1, Folder(2), 3);
        // Past the last folder row: back to All Photos.
        library.press(1, All, 1);
        // Back into Folder(2): its last photo is in the hidden child folder.
        library.press(-1, Folder(2), 5);
    }
}
