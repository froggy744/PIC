//! Window session state: what the window remembers between runs of one
//! library (last view, viewport, window size and the split widths).
//!
//! This module owns the setting keys, their defaults and clamps, and how a
//! view is encoded. Callers pass plain values and read [`SessionState`]; they
//! never name a key. The store is a seam with two adapters: the library's
//! SQLite connection (the app) and an in-memory map (the tests below).
//! Clamps apply on load, as before, so a stored value is never rewritten
//! just by reading it.

use rusqlite::Connection;

use super::sidebar_presentation::{clamp_compact_rail_width, COMPACT_RAIL_MIN_WIDTH};
use super::{SIDEBAR_COLLAPSED_DEFAULT_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH};
use crate::{db, sidebar::SidebarFilter};

pub(super) const LAST_VIEW_SETTING_KEY: &str = "last-view";
pub(super) const LAST_VIEW_PHOTO_ID_SETTING_KEY: &str = "last-view-photo-id";
pub(super) const LAST_ACTIVATED_PHOTO_ID_SETTING_KEY: &str = "last-activated-photo-id";
pub(super) const LAST_VIEW_SCROLL_SETTING_KEY: &str = "last-view-scroll";
pub(super) const LAST_ALBUMS_SCROLL_SETTING_KEY: &str = "last-albums-scroll";
pub(super) const WINDOW_WIDTH_SETTING_KEY: &str = "window-width";
pub(super) const WINDOW_HEIGHT_SETTING_KEY: &str = "window-height";
pub(super) const WINDOW_MAXIMIZED_SETTING_KEY: &str = "window-maximized";
pub(super) const SIDEBAR_WIDTH_FRACTION_SETTING_KEY: &str = "sidebar-width-fraction";
pub(super) const PROPERTIES_WIDTH_FRACTION_SETTING_KEY: &str = "properties-width-fraction";
pub(super) const SIDEBAR_COLLAPSED_WIDTH_SETTING_KEY: &str = "sidebar-collapsed-width";
// v2 reset the former 120 px default once; thereafter the chosen compact
// width persists independently from the normal sidebar.
const COMPACT_SIDEBAR_WIDTH_SETTING_KEY: &str = "compact-sidebar-width-v2";

const DEFAULT_WINDOW_WIDTH: i32 = 1440;
const DEFAULT_WINDOW_HEIGHT: i32 = 900;
const DEFAULT_SIDEBAR_FRACTION: f64 = 0.22;
const DEFAULT_PROPERTIES_FRACTION: f64 = 0.25;

/// Where session values are kept. Write errors are reported as text so the
/// caller can print them; reads treat a failure as "not saved".
pub(super) trait SessionStore {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// The app's adapter: the library database's settings table.
impl SessionStore for Connection {
    fn get(&self, key: &str) -> Option<String> {
        db::setting(self, key).ok().flatten()
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        db::set_setting(self, key, value).map_err(|error| error.to_string())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        db::delete_setting(self, key).map_err(|error| error.to_string())
    }
}

/// How the window arrived at a library. The two differ only when no view
/// was saved: a fresh start opens Recently Added, while switching to a
/// library that was never browsed opens its Library home page so the
/// switch is visibly complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Startup,
    LibrarySwitch,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WindowSize {
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
}

/// What the visible page was scrolled to when the library was left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Viewport {
    Albums {
        scroll: f64,
    },
    /// `anchor` is the photo at the viewport centre, if any.
    Gallery {
        scroll: f64,
        anchor: Option<i64>,
    },
}

/// Everything saved when the window leaves a library: on close and before
/// a library switch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LeaveSnapshot {
    pub view: SidebarFilter,
    pub viewport: Viewport,
    pub window: WindowSize,
    /// `None` while the sidebar is not in normal mode: a compact rail's
    /// width must not overwrite the normal sidebar's fraction.
    pub sidebar_fraction: Option<f64>,
}

/// One thing to remember.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SessionChange {
    View(SidebarFilter),
    ActivatedPhoto(i64),
    SidebarFraction(f64),
    CollapsedWidth(f64),
    CompactWidth(f64),
    PropertiesFraction(f64),
    Leave(LeaveSnapshot),
}

/// The saved session of one library, with defaults and clamps applied.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SessionState {
    pub window: WindowSize,
    saved_view: Option<String>,
    /// The photo at the viewport centre when the gallery was left.
    pub view_anchor: Option<i64>,
    /// The photo most recently opened in the viewer; separate from the
    /// selection and the viewport.
    pub activated_photo: Option<i64>,
    pub gallery_scroll: f64,
    pub albums_scroll: f64,
    pub sidebar_fraction: f64,
    pub collapsed_width: f64,
    pub compact_width: f64,
    pub properties_fraction: f64,
}

impl SessionState {
    pub(super) fn load(store: &impl SessionStore) -> Self {
        Self {
            window: WindowSize {
                width: parsed::<i32>(store, WINDOW_WIDTH_SETTING_KEY)
                    .unwrap_or(DEFAULT_WINDOW_WIDTH)
                    .clamp(640, 7680),
                height: parsed::<i32>(store, WINDOW_HEIGHT_SETTING_KEY)
                    .unwrap_or(DEFAULT_WINDOW_HEIGHT)
                    .clamp(480, 4320),
                maximized: store.get(WINDOW_MAXIMIZED_SETTING_KEY).as_deref() == Some("true"),
            },
            saved_view: store.get(LAST_VIEW_SETTING_KEY),
            view_anchor: parsed(store, LAST_VIEW_PHOTO_ID_SETTING_KEY),
            activated_photo: parsed(store, LAST_ACTIVATED_PHOTO_ID_SETTING_KEY),
            gallery_scroll: length(store, LAST_VIEW_SCROLL_SETTING_KEY)
                .unwrap_or(0.0)
                .max(0.0),
            albums_scroll: length(store, LAST_ALBUMS_SCROLL_SETTING_KEY)
                .unwrap_or(0.0)
                .max(0.0),
            sidebar_fraction: length(store, SIDEBAR_WIDTH_FRACTION_SETTING_KEY)
                .unwrap_or(DEFAULT_SIDEBAR_FRACTION)
                .clamp(0.10, 0.70),
            collapsed_width: length(store, SIDEBAR_COLLAPSED_WIDTH_SETTING_KEY)
                .unwrap_or(SIDEBAR_COLLAPSED_DEFAULT_WIDTH)
                .clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH),
            compact_width: length(store, COMPACT_SIDEBAR_WIDTH_SETTING_KEY)
                .map(clamp_compact_rail_width)
                .unwrap_or(COMPACT_RAIL_MIN_WIDTH),
            properties_fraction: length(store, PROPERTIES_WIDTH_FRACTION_SETTING_KEY)
                .unwrap_or(DEFAULT_PROPERTIES_FRACTION)
                .clamp(0.10, 0.55),
        }
    }

    /// The destination to open, given the folders and albums that still
    /// exist. A saved folder or album that disappeared never reopens as an
    /// empty, unselectable page.
    pub(super) fn view(
        &self,
        entry: Entry,
        folders: &[db::Folder],
        albums: &[db::Album],
    ) -> SidebarFilter {
        let Some(value) = self.saved_view.as_deref() else {
            return match entry {
                Entry::Startup => SidebarFilter::RecentlyAdded,
                Entry::LibrarySwitch => SidebarFilter::Library,
            };
        };
        match value {
            "library" => SidebarFilter::Library,
            "all" => SidebarFilter::All,
            "favorites" => SidebarFilter::Favorites,
            "ratings" => SidebarFilter::Ratings,
            "albums" => SidebarFilter::Albums,
            "recently-added" => SidebarFilter::RecentlyAdded,
            "history" => SidebarFilter::History,
            value if value.starts_with("rating:") => value[7..]
                .parse::<u8>()
                .ok()
                .filter(|rating| (1..=5).contains(rating))
                .map(SidebarFilter::Rating)
                .unwrap_or(SidebarFilter::Ratings),
            value if value.starts_with("folder:") => value[7..]
                .parse::<i64>()
                .ok()
                .filter(|id| folders.iter().any(|folder| folder.id == *id))
                .map(SidebarFilter::Folder)
                .unwrap_or(SidebarFilter::RecentlyAdded),
            value if value.starts_with("album:") => value[6..]
                .parse::<i64>()
                .ok()
                .filter(|id| albums.iter().any(|album| album.id == *id))
                .map(SidebarFilter::Album)
                .unwrap_or(SidebarFilter::RecentlyAdded),
            _ => SidebarFilter::RecentlyAdded,
        }
    }
}

/// Save one change. A leave snapshot writes every value even if one fails,
/// and returns the first error.
pub(super) fn record(store: &impl SessionStore, change: SessionChange) -> Result<(), String> {
    match change {
        SessionChange::View(filter) => store.set(LAST_VIEW_SETTING_KEY, &view_setting(filter)),
        SessionChange::ActivatedPhoto(id) => {
            store.set(LAST_ACTIVATED_PHOTO_ID_SETTING_KEY, &id.to_string())
        }
        SessionChange::SidebarFraction(fraction) => {
            store.set(SIDEBAR_WIDTH_FRACTION_SETTING_KEY, &fraction.to_string())
        }
        SessionChange::CollapsedWidth(width) => {
            store.set(SIDEBAR_COLLAPSED_WIDTH_SETTING_KEY, &width.to_string())
        }
        SessionChange::CompactWidth(width) => {
            store.set(COMPACT_SIDEBAR_WIDTH_SETTING_KEY, &width.to_string())
        }
        SessionChange::PropertiesFraction(fraction) => {
            store.set(PROPERTIES_WIDTH_FRACTION_SETTING_KEY, &fraction.to_string())
        }
        SessionChange::Leave(leave) => record_leave(store, leave),
    }
}

fn record_leave(store: &impl SessionStore, leave: LeaveSnapshot) -> Result<(), String> {
    let mut results = vec![store.set(LAST_VIEW_SETTING_KEY, &view_setting(leave.view))];
    match leave.viewport {
        Viewport::Albums { scroll } => {
            results.push(store.set(LAST_ALBUMS_SCROLL_SETTING_KEY, &scroll.to_string()));
        }
        Viewport::Gallery { scroll, anchor } => {
            results.push(store.set(LAST_VIEW_SCROLL_SETTING_KEY, &scroll.to_string()));
            results.push(match anchor {
                Some(id) => store.set(LAST_VIEW_PHOTO_ID_SETTING_KEY, &id.to_string()),
                None => store.delete(LAST_VIEW_PHOTO_ID_SETTING_KEY),
            });
        }
    }
    let window = leave.window;
    results.push(store.set(
        WINDOW_MAXIMIZED_SETTING_KEY,
        if window.maximized { "true" } else { "false" },
    ));
    // A maximized window reports the screen size; keep the size it restores to.
    if !window.maximized {
        results.push(store.set(WINDOW_WIDTH_SETTING_KEY, &window.width.to_string()));
        results.push(store.set(WINDOW_HEIGHT_SETTING_KEY, &window.height.to_string()));
    }
    if let Some(fraction) = leave.sidebar_fraction {
        results.push(store.set(SIDEBAR_WIDTH_FRACTION_SETTING_KEY, &fraction.to_string()));
    }
    results.into_iter().collect()
}

fn view_setting(filter: SidebarFilter) -> String {
    match filter {
        SidebarFilter::Library => "library".into(),
        SidebarFilter::All => "all".into(),
        SidebarFilter::Favorites => "favorites".into(),
        SidebarFilter::Ratings => "ratings".into(),
        SidebarFilter::Rating(rating) => format!("rating:{rating}"),
        SidebarFilter::RecentlyAdded => "recently-added".into(),
        SidebarFilter::History => "history".into(),
        SidebarFilter::Albums => "albums".into(),
        SidebarFilter::Folder(id) => format!("folder:{id}"),
        SidebarFilter::Album(id) => format!("album:{id}"),
    }
}

fn parsed<T: std::str::FromStr>(store: &impl SessionStore, key: &str) -> Option<T> {
    store.get(key)?.parse().ok()
}

/// A stored length or fraction. A non-finite value ("NaN", "inf") counts as
/// not saved, so it can never reach a GTK size.
fn length(store: &impl SessionStore, key: &str) -> Option<f64> {
    parsed::<f64>(store, key).filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::*;

    /// The test adapter.
    #[derive(Default)]
    struct MemoryStore {
        values: RefCell<BTreeMap<String, String>>,
        fail_key: Option<&'static str>,
    }

    impl MemoryStore {
        fn with(values: &[(&str, &str)]) -> Self {
            let store = Self::default();
            for (key, value) in values {
                store
                    .values
                    .borrow_mut()
                    .insert((*key).into(), (*value).into());
            }
            store
        }

        fn value(&self, key: &str) -> Option<String> {
            self.values.borrow().get(key).cloned()
        }
    }

    impl SessionStore for MemoryStore {
        fn get(&self, key: &str) -> Option<String> {
            self.value(key)
        }

        fn set(&self, key: &str, value: &str) -> Result<(), String> {
            if self.fail_key == Some(key) {
                return Err(format!("cannot write {key}"));
            }
            self.values.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<(), String> {
            self.values.borrow_mut().remove(key);
            Ok(())
        }
    }

    fn folder(id: i64) -> db::Folder {
        db::Folder {
            id,
            path: "/photos".into(),
            name: "Photos".into(),
            parent_id: None,
            imported_root: true,
            watched: false,
            photo_count: 0,
            subfolder_count: 0,
            available: true,
        }
    }

    fn album(id: i64) -> db::Album {
        db::Album {
            id,
            name: "Holiday".into(),
            created_at: 0,
            photo_count: 0,
            cover_frame: None,
            cover_photo_id: None,
        }
    }

    #[test]
    fn an_empty_store_loads_the_defaults() {
        let state = SessionState::load(&MemoryStore::default());
        assert_eq!(
            state.window,
            WindowSize {
                width: 1440,
                height: 900,
                maximized: false
            }
        );
        assert_eq!(state.view_anchor, None);
        assert_eq!(state.activated_photo, None);
        assert_eq!(state.gallery_scroll, 0.0);
        assert_eq!(state.albums_scroll, 0.0);
        assert_eq!(state.sidebar_fraction, 0.22);
        assert_eq!(state.collapsed_width, SIDEBAR_COLLAPSED_DEFAULT_WIDTH);
        assert_eq!(state.compact_width, COMPACT_RAIL_MIN_WIDTH);
        assert_eq!(state.properties_fraction, 0.25);
    }

    #[test]
    fn saved_values_load_and_are_clamped() {
        // (key, stored, read back through the loaded state)
        type Read = fn(&SessionState) -> f64;
        let cases: &[(&str, &str, Read, f64)] = &[
            (
                WINDOW_WIDTH_SETTING_KEY,
                "1000",
                |s| s.window.width.into(),
                1000.0,
            ),
            (
                WINDOW_WIDTH_SETTING_KEY,
                "10",
                |s| s.window.width.into(),
                640.0,
            ),
            (
                WINDOW_WIDTH_SETTING_KEY,
                "99999",
                |s| s.window.width.into(),
                7680.0,
            ),
            (
                WINDOW_WIDTH_SETTING_KEY,
                "wide",
                |s| s.window.width.into(),
                1440.0,
            ),
            (
                WINDOW_HEIGHT_SETTING_KEY,
                "700",
                |s| s.window.height.into(),
                700.0,
            ),
            (
                WINDOW_HEIGHT_SETTING_KEY,
                "1",
                |s| s.window.height.into(),
                480.0,
            ),
            (
                WINDOW_HEIGHT_SETTING_KEY,
                "99999",
                |s| s.window.height.into(),
                4320.0,
            ),
            (
                LAST_VIEW_SCROLL_SETTING_KEY,
                "812.5",
                |s| s.gallery_scroll,
                812.5,
            ),
            (
                LAST_VIEW_SCROLL_SETTING_KEY,
                "-4",
                |s| s.gallery_scroll,
                0.0,
            ),
            (
                LAST_ALBUMS_SCROLL_SETTING_KEY,
                "120",
                |s| s.albums_scroll,
                120.0,
            ),
            (
                LAST_ALBUMS_SCROLL_SETTING_KEY,
                "-1",
                |s| s.albums_scroll,
                0.0,
            ),
            (
                SIDEBAR_WIDTH_FRACTION_SETTING_KEY,
                "0.3",
                |s| s.sidebar_fraction,
                0.3,
            ),
            (
                SIDEBAR_WIDTH_FRACTION_SETTING_KEY,
                "0.01",
                |s| s.sidebar_fraction,
                0.10,
            ),
            (
                SIDEBAR_WIDTH_FRACTION_SETTING_KEY,
                "0.9",
                |s| s.sidebar_fraction,
                0.70,
            ),
            (
                SIDEBAR_WIDTH_FRACTION_SETTING_KEY,
                "NaN",
                |s| s.sidebar_fraction,
                0.22,
            ),
            (
                PROPERTIES_WIDTH_FRACTION_SETTING_KEY,
                "0.4",
                |s| s.properties_fraction,
                0.4,
            ),
            (
                PROPERTIES_WIDTH_FRACTION_SETTING_KEY,
                "0.01",
                |s| s.properties_fraction,
                0.10,
            ),
            (
                PROPERTIES_WIDTH_FRACTION_SETTING_KEY,
                "0.9",
                |s| s.properties_fraction,
                0.55,
            ),
            (
                SIDEBAR_COLLAPSED_WIDTH_SETTING_KEY,
                "350",
                |s| s.collapsed_width,
                350.0,
            ),
            (
                SIDEBAR_COLLAPSED_WIDTH_SETTING_KEY,
                "50",
                |s| s.collapsed_width,
                SIDEBAR_MIN_WIDTH,
            ),
            (
                SIDEBAR_COLLAPSED_WIDTH_SETTING_KEY,
                "5000",
                |s| s.collapsed_width,
                SIDEBAR_MAX_WIDTH,
            ),
            (
                COMPACT_SIDEBAR_WIDTH_SETTING_KEY,
                "1",
                |s| s.compact_width,
                clamp_compact_rail_width(1.0),
            ),
            (
                COMPACT_SIDEBAR_WIDTH_SETTING_KEY,
                "120",
                |s| s.compact_width,
                clamp_compact_rail_width(120.0),
            ),
        ];
        for (key, stored, read, expected) in cases {
            let state = SessionState::load(&MemoryStore::with(&[(key, stored)]));
            assert_eq!(read(&state), *expected, "{key}={stored}");
        }
    }

    #[test]
    fn photo_ids_and_maximized_load() {
        let state = SessionState::load(&MemoryStore::with(&[
            (LAST_VIEW_PHOTO_ID_SETTING_KEY, "7"),
            (LAST_ACTIVATED_PHOTO_ID_SETTING_KEY, "9"),
            (WINDOW_MAXIMIZED_SETTING_KEY, "true"),
        ]));
        assert_eq!(state.view_anchor, Some(7));
        assert_eq!(state.activated_photo, Some(9));
        assert!(state.window.maximized);

        let state = SessionState::load(&MemoryStore::with(&[
            (LAST_VIEW_PHOTO_ID_SETTING_KEY, "seven"),
            (WINDOW_MAXIMIZED_SETTING_KEY, "yes"),
        ]));
        assert_eq!(state.view_anchor, None);
        assert!(!state.window.maximized);
    }

    #[test]
    fn every_destination_round_trips() {
        let folders = [folder(42)];
        let albums = [album(17)];
        for filter in [
            SidebarFilter::Library,
            SidebarFilter::All,
            SidebarFilter::Favorites,
            SidebarFilter::Ratings,
            SidebarFilter::Rating(3),
            SidebarFilter::RecentlyAdded,
            SidebarFilter::History,
            SidebarFilter::Albums,
            SidebarFilter::Folder(42),
            SidebarFilter::Album(17),
        ] {
            let store = MemoryStore::default();
            record(&store, SessionChange::View(filter)).unwrap();
            for entry in [Entry::Startup, Entry::LibrarySwitch] {
                assert_eq!(
                    SessionState::load(&store).view(entry, &folders, &albums),
                    filter,
                    "{filter:?} via {entry:?}"
                );
            }
        }
    }

    #[test]
    fn the_stored_view_text_is_stable() {
        // Existing libraries hold these strings; they must keep decoding.
        for (filter, text) in [
            (SidebarFilter::Library, "library"),
            (SidebarFilter::All, "all"),
            (SidebarFilter::Favorites, "favorites"),
            (SidebarFilter::Ratings, "ratings"),
            (SidebarFilter::Rating(3), "rating:3"),
            (SidebarFilter::RecentlyAdded, "recently-added"),
            (SidebarFilter::History, "history"),
            (SidebarFilter::Albums, "albums"),
            (SidebarFilter::Folder(42), "folder:42"),
            (SidebarFilter::Album(17), "album:17"),
        ] {
            assert_eq!(view_setting(filter), text);
        }
    }

    #[test]
    fn missing_or_stale_views_fall_back_by_entry() {
        // (stored view, startup lands on, library switch lands on)
        let cases = [
            (None, SidebarFilter::RecentlyAdded, SidebarFilter::Library),
            (
                Some("album:17"),
                SidebarFilter::RecentlyAdded,
                SidebarFilter::RecentlyAdded,
            ),
            (
                Some("folder:nope"),
                SidebarFilter::RecentlyAdded,
                SidebarFilter::RecentlyAdded,
            ),
            (
                Some("unknown"),
                SidebarFilter::RecentlyAdded,
                SidebarFilter::RecentlyAdded,
            ),
            (
                Some("rating:9"),
                SidebarFilter::Ratings,
                SidebarFilter::Ratings,
            ),
        ];
        for (stored, startup, switch) in cases {
            let store = match stored {
                Some(value) => MemoryStore::with(&[(LAST_VIEW_SETTING_KEY, value)]),
                None => MemoryStore::default(),
            };
            let state = SessionState::load(&store);
            assert_eq!(state.view(Entry::Startup, &[], &[]), startup, "{stored:?}");
            assert_eq!(
                state.view(Entry::LibrarySwitch, &[], &[]),
                switch,
                "{stored:?}"
            );
        }
    }

    #[test]
    fn single_changes_write_their_value() {
        let cases: &[(SessionChange, &str, &str)] = &[
            (
                SessionChange::View(SidebarFilter::Folder(3)),
                LAST_VIEW_SETTING_KEY,
                "folder:3",
            ),
            (
                SessionChange::ActivatedPhoto(11),
                LAST_ACTIVATED_PHOTO_ID_SETTING_KEY,
                "11",
            ),
            (
                SessionChange::SidebarFraction(0.3),
                SIDEBAR_WIDTH_FRACTION_SETTING_KEY,
                "0.3",
            ),
            (
                SessionChange::CollapsedWidth(320.0),
                SIDEBAR_COLLAPSED_WIDTH_SETTING_KEY,
                "320",
            ),
            (
                SessionChange::CompactWidth(96.0),
                COMPACT_SIDEBAR_WIDTH_SETTING_KEY,
                "96",
            ),
            (
                SessionChange::PropertiesFraction(0.4),
                PROPERTIES_WIDTH_FRACTION_SETTING_KEY,
                "0.4",
            ),
        ];
        for (change, key, value) in cases {
            let store = MemoryStore::default();
            record(&store, *change).unwrap();
            assert_eq!(store.value(key).as_deref(), Some(*value), "{change:?}");
            assert_eq!(store.values.borrow().len(), 1, "{change:?} wrote one key");
        }
    }

    fn snapshot(viewport: Viewport, window: WindowSize, sidebar: Option<f64>) -> LeaveSnapshot {
        LeaveSnapshot {
            view: SidebarFilter::All,
            viewport,
            window,
            sidebar_fraction: sidebar,
        }
    }

    const RESTORED: WindowSize = WindowSize {
        width: 1200,
        height: 800,
        maximized: false,
    };
    const MAXIMIZED: WindowSize = WindowSize {
        width: 3000,
        height: 2000,
        maximized: true,
    };

    #[test]
    fn leaving_the_gallery_saves_view_viewport_window_and_sidebar() {
        let store = MemoryStore::default();
        let leave = snapshot(
            Viewport::Gallery {
                scroll: 640.5,
                anchor: Some(5),
            },
            RESTORED,
            Some(0.3),
        );
        record(&store, SessionChange::Leave(leave)).unwrap();
        let state = SessionState::load(&store);
        assert_eq!(state.view(Entry::Startup, &[], &[]), SidebarFilter::All);
        assert_eq!(state.gallery_scroll, 640.5);
        assert_eq!(state.view_anchor, Some(5));
        assert_eq!(state.window, RESTORED);
        assert_eq!(state.sidebar_fraction, 0.3);
    }

    #[test]
    fn leaving_without_an_anchor_forgets_the_old_one() {
        let store = MemoryStore::with(&[(LAST_VIEW_PHOTO_ID_SETTING_KEY, "5")]);
        let leave = snapshot(
            Viewport::Gallery {
                scroll: 0.0,
                anchor: None,
            },
            RESTORED,
            None,
        );
        record(&store, SessionChange::Leave(leave)).unwrap();
        assert_eq!(store.value(LAST_VIEW_PHOTO_ID_SETTING_KEY), None);
    }

    #[test]
    fn leaving_albums_saves_only_the_albums_scroll() {
        let store = MemoryStore::with(&[
            (LAST_VIEW_SCROLL_SETTING_KEY, "300"),
            (LAST_VIEW_PHOTO_ID_SETTING_KEY, "5"),
        ]);
        let leave = snapshot(Viewport::Albums { scroll: 90.0 }, RESTORED, None);
        record(&store, SessionChange::Leave(leave)).unwrap();
        let state = SessionState::load(&store);
        assert_eq!(state.albums_scroll, 90.0);
        assert_eq!(state.gallery_scroll, 300.0);
        assert_eq!(state.view_anchor, Some(5));
    }

    #[test]
    fn a_maximized_window_keeps_its_restored_size() {
        let store = MemoryStore::default();
        record(
            &store,
            SessionChange::Leave(snapshot(Viewport::Albums { scroll: 0.0 }, RESTORED, None)),
        )
        .unwrap();
        record(
            &store,
            SessionChange::Leave(snapshot(Viewport::Albums { scroll: 0.0 }, MAXIMIZED, None)),
        )
        .unwrap();
        assert_eq!(
            SessionState::load(&store).window,
            WindowSize {
                maximized: true,
                ..RESTORED
            }
        );
    }

    #[test]
    fn a_compact_sidebar_does_not_overwrite_the_normal_fraction() {
        let store = MemoryStore::with(&[(SIDEBAR_WIDTH_FRACTION_SETTING_KEY, "0.3")]);
        let leave = snapshot(Viewport::Albums { scroll: 0.0 }, RESTORED, None);
        record(&store, SessionChange::Leave(leave)).unwrap();
        assert_eq!(SessionState::load(&store).sidebar_fraction, 0.3);
    }

    #[test]
    fn a_failed_write_still_saves_the_rest_of_a_leave() {
        let store = MemoryStore {
            fail_key: Some(LAST_VIEW_SETTING_KEY),
            ..MemoryStore::default()
        };
        let leave = snapshot(
            Viewport::Gallery {
                scroll: 10.0,
                anchor: Some(2),
            },
            RESTORED,
            Some(0.3),
        );
        let error = record(&store, SessionChange::Leave(leave)).unwrap_err();
        assert!(error.contains(LAST_VIEW_SETTING_KEY), "{error}");
        let state = SessionState::load(&store);
        assert_eq!(state.gallery_scroll, 10.0);
        assert_eq!(state.window, RESTORED);
        assert_eq!(state.sidebar_fraction, 0.3);
    }

    #[test]
    fn the_sqlite_adapter_round_trips() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(db::SCHEMA).unwrap();
        let leave = snapshot(
            Viewport::Gallery {
                scroll: 12.0,
                anchor: Some(4),
            },
            RESTORED,
            Some(0.25),
        );
        record(&connection, SessionChange::Leave(leave)).unwrap();
        let state = SessionState::load(&connection);
        assert_eq!(state.gallery_scroll, 12.0);
        assert_eq!(state.view_anchor, Some(4));
        assert_eq!(state.window, RESTORED);
        assert_eq!(state.sidebar_fraction, 0.25);
    }
}
