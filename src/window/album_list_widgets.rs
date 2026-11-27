//! The album list's GTK adapter: the only writer of the sidebar's album rows
//! and the Albums home (`window::album_list` decides what to refresh and
//! which results are current).
//!
//! Callers say what changed with [`AlbumList::changed`]. The sidebar always
//! gets whole albums (`db::albums`); the Albums home gets counts narrowed by
//! the browse scope (`db::browse_albums` under `albums_home_scope`). Worker
//! results are applied only while their surface's generation, the library
//! and (for the home) the browse scope are unchanged.
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use gtk4::glib;
use rusqlite::Connection;

use super::album_list::{plan, AlbumChange, AlbumListState, AlbumRefresh, AlbumTicket, Timing};
use super::albums_home_scope;
use crate::db;
use crate::sidebar::{self, SidebarFilter};
use super::navigation_filter::NavigationFilter;

type AlbumSink = Rc<dyn Fn(&[db::Album])>;
/// What a refresh counted: the sidebar's and the home's lists, when asked.
type Counted = (Option<Vec<db::Album>>, Option<Vec<db::Album>>);

/// How long a burst of photo changes shares one Smart Album recount.
const RECOUNT_BATCH: Duration = Duration::from_millis(150);

pub(super) struct AlbumList {
    connection: Rc<RefCell<Connection>>,
    filter: Rc<NavigationFilter>,
    state: RefCell<AlbumListState>,
    sidebar_rows: RefCell<Option<AlbumSink>>,
    home: RefCell<Option<AlbumSink>>,
}

impl AlbumList {
    /// Surfaces are attached later, once they exist.
    pub(super) fn new(connection: Rc<RefCell<Connection>>, filter: Rc<NavigationFilter>) -> Rc<Self> {
        Rc::new(Self {
            connection,
            filter,
            state: RefCell::default(),
            sidebar_rows: RefCell::new(None),
            home: RefCell::new(None),
        })
    }

    /// The sidebar's albums: whole albums, unfiltered. For building or fully
    /// rebuilding the sidebar (startup, library switch, folder changes).
    pub(super) fn sidebar_albums(connection: &Connection) -> Vec<db::Album> {
        db::albums(connection).unwrap_or_default()
    }

    /// Write album rows into the sidebar held by `sidebar` (once built).
    pub(super) fn attach_sidebar(
        &self,
        sidebar: Rc<RefCell<Option<gtk::ScrolledWindow>>>,
        on_delete_album: Rc<dyn Fn(i64)>,
    ) {
        self.sidebar_rows.replace(Some(Rc::new(move |albums: &[db::Album]| {
            if let Some(sidebar) = sidebar.borrow().as_ref() {
                sidebar::refresh_album_rows(sidebar, albums, &on_delete_album);
            }
        })));
    }

    /// Write the Albums home. Its layout, sort, Type filter, cover size and
    /// position are the view's own and survive a refresh.
    pub(super) fn attach_home(
        &self,
        albums_home: gtk::ScrolledWindow,
        thumbnail_width: i32,
        on_album: Rc<RefCell<Option<Rc<dyn Fn(i64)>>>>,
        on_appearance_changed: Rc<dyn Fn()>,
    ) {
        let connection = self.connection.clone();
        self.home.replace(Some(Rc::new(move |albums: &[db::Album]| {
            let on_album = on_album.borrow().as_ref().cloned().unwrap_or_else(|| Rc::new(|_| {}));
            crate::albums_view::refresh(
                &albums_home,
                albums,
                connection.clone(),
                thumbnail_width,
                on_album,
                on_appearance_changed.clone(),
            );
        })));
    }

    /// Tests: record what each surface is given.
    #[cfg(test)]
    pub(super) fn attach_recorders(&self, sidebar_rows: AlbumSink, home: AlbumSink) {
        self.sidebar_rows.replace(Some(sidebar_rows));
        self.home.replace(Some(home));
    }

    /// Tests: start a Smart Album recount's worker now, without the batch
    /// delay (the same refresh a batch runs).
    #[cfg(test)]
    pub(super) fn recount_now(self: &Rc<Self>) {
        if let Some(refresh) = plan(AlbumChange::PhotosChanged, self.home_shown(), true) {
            self.refresh_on_worker(refresh);
        }
    }

    /// Something changed: refresh the surfaces it affects.
    pub(super) fn changed(self: &Rc<Self>, change: AlbumChange) {
        if change == AlbumChange::LibraryOpened {
            self.state.borrow_mut().library_opened();
        }
        let has_smart = change == AlbumChange::PhotosChanged
            && db::has_smart_albums(&self.connection.borrow()).unwrap_or(false);
        let Some(refresh) = plan(change, self.home_shown(), has_smart) else {
            return;
        };
        match refresh.timing {
            Timing::Now => self.refresh_now(refresh),
            Timing::Worker => self.refresh_on_worker(refresh),
            Timing::Batched => {
                let Some(library) = self.state.borrow_mut().request_batch() else {
                    return; // the waiting recount covers this change too
                };
                let list = self.clone();
                glib::timeout_add_local_once(RECOUNT_BATCH, move || {
                    if !list.state.borrow_mut().take_batch(library) {
                        return;
                    }
                    // The home may have been opened or left meanwhile.
                    if let Some(refresh) = plan(AlbumChange::PhotosChanged, list.home_shown(), true) {
                        list.refresh_on_worker(refresh);
                    }
                });
            }
        }
    }

    fn home_shown(&self) -> bool {
        self.filter.get() == SidebarFilter::Albums
    }

    fn refresh_now(&self, refresh: AlbumRefresh) {
        let ticket = self.state.borrow_mut().begin(refresh);
        let counted = {
            let connection = self.connection.borrow();
            (
                refresh.sidebar.then(|| Self::sidebar_albums(&connection)),
                refresh.home.then(|| {
                    db::browse_albums(&connection, &albums_home_scope(&connection)).unwrap_or_default()
                }),
            )
        };
        self.apply(ticket, counted, None);
    }

    fn refresh_on_worker(self: &Rc<Self>, refresh: AlbumRefresh) {
        let ticket = self.state.borrow_mut().begin(refresh);
        let snapshot = refresh.home.then(|| {
            let connection = self.connection.borrow();
            db::BrowseSnapshot::new(&connection, albums_home_scope(&connection))
        });
        let home_scope = snapshot.as_ref().map(|snapshot| snapshot.scope.clone());
        let count = move |connection: &Connection| -> anyhow::Result<Counted> {
            let sidebar = if refresh.sidebar { Some(db::albums(connection)?) } else { None };
            let home = match &home_scope {
                Some(scope) => Some(db::browse_albums(connection, scope)?),
                None => None,
            };
            Ok((sidebar, home))
        };
        let path = db::connection_path(&self.connection.borrow());
        let Ok(path) = path else {
            // No file behind the connection (tests): count in place.
            let counted = count(&self.connection.borrow());
            if let Ok(counted) = counted {
                self.apply(ticket, counted, snapshot.as_ref());
            }
            return;
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_path = path.clone();
        std::thread::spawn(move || {
            let counted = db::open_existing(&worker_path).and_then(|connection| count(&connection));
            let _ = sender.send(counted.ok());
        });
        let list = self.clone();
        glib::timeout_add_local(Duration::from_millis(25), move || match receiver.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Ok(Some(counted))
                if db::connection_path(&list.connection.borrow()).ok().as_ref() == Some(&path) =>
            {
                list.apply(ticket, counted, snapshot.as_ref());
                glib::ControlFlow::Break
            }
            _ => glib::ControlFlow::Break,
        });
    }

    /// Show what is still current. `snapshot`: a worker's home, counted under
    /// that browse scope; it is dropped if the home was left or the scope moved.
    fn apply(&self, ticket: AlbumTicket, (sidebar, home): Counted, snapshot: Option<&db::BrowseSnapshot>) {
        let (sidebar_current, home_current) = {
            let state = self.state.borrow();
            (state.sidebar_current(ticket), state.home_current(ticket))
        };
        let home_current = home_current
            && snapshot.map_or(true, |snapshot| {
                let connection = self.connection.borrow();
                self.home_shown() && snapshot.is_current(&connection, &albums_home_scope(&connection))
            });
        let sidebar_sink = self.sidebar_rows.borrow().clone();
        let home_sink = self.home.borrow().clone();
        if let (Some(albums), true, Some(sink)) = (sidebar, sidebar_current, sidebar_sink) {
            sink(&albums);
        }
        if let (Some(albums), true, Some(sink)) = (home, home_current, home_sink) {
            sink(&albums);
        }
    }
}
