//! One photo-list pipeline: the photos a gallery destination shows, in the
//! order it shows them. The grid refresh, startup, the library switch and
//! collection navigation all build their lists here, so format visibility,
//! RAW+JPEG pairing, the browse scope (rating, metadata filters, Smart
//! Albums), the Recently Added limit, the Files check and the order cannot
//! drift apart between them.
//!
//! The pipeline takes and returns plain values and never touches GTK, so a
//! worker thread can run it; the window applies a [`PhotoList`] to the
//! gallery (pairing state, then the photos) only while its refresh is still
//! current.
use std::collections::HashSet;

use rusqlite::Connection;

use super::{browse_scope, retain_enabled_formats, sort_folder_stream, sort_photos};
use super::{PhotoSort, RatingFilter, SortDirection, SortField};
use crate::db;
use crate::image_format::RawJpegPairMode;
use crate::sidebar::{self, SidebarFilter};

/// What to list.
#[derive(Clone, Debug)]
pub(super) struct PhotoListRequest {
    /// The sidebar destination being shown. It decides the Recently Added
    /// limit, History's own order and the Folder stream.
    pub(super) filter: SidebarFilter,
    /// Which photos belong: destination, search, rating, metadata filters.
    pub(super) scope: db::BrowseScope,
    pub(super) sort: PhotoSort,
    /// Collection navigation's favourites within one folder page: the scope
    /// holds one destination, so the folder is the query's and favourites
    /// are kept right after it.
    pub(super) favorites_only: bool,
}

impl PhotoListRequest {
    /// The window's own view of `filter` and `search`, under the session's
    /// browse scope (the grid refresh, startup).
    pub(super) fn for_view(
        connection: &Connection,
        filter: SidebarFilter,
        search: &str,
        sort: PhotoSort,
    ) -> Self {
        Self {
            filter,
            scope: browse_scope(connection, filter, search),
            sort,
            favorites_only: false,
        }
    }

    fn search_is_empty(&self) -> bool {
        self.scope.search.is_empty()
    }

    /// The continuous Folder stream: Folder mode over the whole library.
    /// Collection navigation's single-folder pages are not one.
    fn folder_stream(&self) -> bool {
        self.search_is_empty()
            && matches!(self.filter, SidebarFilter::Folder(_))
            && self.scope.destination == db::Destination::AllPhotos
    }
}

/// A destination's photos in display order, with the RAW+JPEG pairing state
/// the gallery shows them under.
#[derive(Clone, Debug)]
pub(super) struct PhotoList {
    pub(super) photos: Vec<db::Photo>,
    pub(super) pair_mode: RawJpegPairMode,
    /// Folders whose headers offer the RAW+JPEG toggle (Folder stream only).
    pub(super) pair_folders: HashSet<i64>,
}

impl Default for PhotoList {
    fn default() -> Self {
        Self {
            photos: Vec::new(),
            pair_mode: RawJpegPairMode::Both,
            pair_folders: HashSet::new(),
        }
    }
}

/// The photos `request` shows, in order. `None` for destinations without a
/// photo grid (Albums, Library). Steps, in this order: query, formats,
/// RAW+JPEG pairing, Recently Added limit (no search), Files check, order
/// (Folder stream, History's own order without a search, otherwise `sort`).
pub(super) fn photo_list(connection: &Connection, request: &PhotoListRequest) -> Option<PhotoList> {
    let (mut photos, pair_mode) = members(connection, request)?;
    let stream = request.folder_stream();
    if stream {
        let folders = db::folders(connection).unwrap_or_default();
        let display_mode = sidebar::FolderDisplayMode::from_setting(
            db::setting(connection, sidebar::FOLDER_DISPLAY_MODE_SETTING_KEY)
                .ok()
                .flatten()
                .as_deref(),
        );
        sort_folder_stream(&mut photos, &folders, request.sort, display_mode);
    } else if request.filter != SidebarFilter::History || !request.search_is_empty() {
        sort_photos(&mut photos, request.sort);
    }
    let pair_folders = if stream {
        db::raw_jpeg_pair_folder_ids(connection).unwrap_or_default()
    } else {
        HashSet::new()
    };
    Some(PhotoList { photos, pair_mode, pair_folders })
}

/// Whether `request` shows any photo: the same membership as [`photo_list`]
/// without ordering it (collection navigation's search for the next
/// non-empty page).
pub(super) fn photo_list_has_photos(connection: &Connection, request: &PhotoListRequest) -> bool {
    members(connection, request).is_some_and(|(photos, _)| !photos.is_empty())
}

fn members(
    connection: &Connection,
    request: &PhotoListRequest,
) -> Option<(Vec<db::Photo>, RawJpegPairMode)> {
    let mut photos = collection_photos(connection, request.filter, &request.scope)?;
    if request.favorites_only {
        photos.retain(|photo| photo.favorite);
    }
    retain_enabled_formats(connection, &mut photos);
    let pair_mode = crate::image_format::raw_jpeg_pair_mode(connection);
    crate::image_format::retain_raw_jpeg_pair_mode(&mut photos, pair_mode);
    if request.search_is_empty() {
        limit_recently_added(connection, request.filter, &mut photos);
    }
    apply_files_check(&mut photos, &request.scope);
    Some((photos, pair_mode))
}

/// The photos the database returns for `scope` (taken for `filter`), before
/// format, RAW+JPEG, Recently Added and Files filtering. `None` for
/// destinations that show no photo grid (Albums, Library).
pub(super) fn collection_photos(
    connection: &Connection,
    filter: SidebarFilter,
    scope: &db::BrowseScope,
) -> Option<Vec<db::Photo>> {
    if scope.search.is_empty() && matches!(filter, SidebarFilter::Albums | SidebarFilter::Library) {
        return None;
    }
    Some(db::browse_photos(connection, scope).unwrap_or_default())
}

pub(super) fn limit_recently_added(connection: &Connection, filter: SidebarFilter, photos: &mut Vec<db::Photo>) {
    if filter != SidebarFilter::RecentlyAdded {
        return;
    }
    // Select the newest files first; the order step restores the chosen order.
    sort_photos(
        photos,
        PhotoSort {
            field: SortField::DateAdded,
            direction: SortDirection::Descending,
        },
    );
    photos.truncate(db::recently_added_limit(connection));
}

/// The Files choice (Online, Offline, Corrupt) SQL cannot express, checked on
/// the loaded photos. Stored ratings are already in the query.
pub(super) fn apply_files_check(photos: &mut Vec<db::Photo>, scope: &db::BrowseScope) {
    match scope.files_check() {
        Some(RatingFilter::Online) => {
            photos.retain(|photo| crate::source::folder_available(photo.folder_id));
        }
        Some(RatingFilter::Offline) => {
            photos.retain(|photo| !crate::source::folder_available(photo.folder_id));
        }
        Some(RatingFilter::Corrupt) => {
            photos.retain(|photo| {
                crate::photo_object::confirmed_corrupt_local_jpeg(&photo.path, photo.width, photo.height)
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::RATING_FILTER_SETTING_KEY;

    const BY_NAME: PhotoSort = PhotoSort { field: SortField::Name, direction: SortDirection::Ascending };

    /// Folders /lib and /lib/sub; a RAW+JPEG pair (a.NEF, a.jpg), PNGs, ratings,
    /// favourites, added times, an edit history and a Smart Album of 5 stars.
    fn library() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(db::SCHEMA).unwrap();
        c.execute_batch(
            "INSERT INTO folders(id,path,name,parent_id) VALUES (1,'/lib','lib',NULL),(2,'/lib/sub','sub',1);
             INSERT INTO photos(id,path,folder_id,rating,favorite,added_at,taken_at) VALUES
               (1,'/lib/a.NEF',1,5,1,10,'2024-01-05'),
               (2,'/lib/a.jpg',1,5,0,11,'2024-01-05'),
               (3,'/lib/b.png',1,0,1,12,'2024-01-04'),
               (4,'/lib/sub/c.jpg',2,3,0,13,'2024-01-03'),
               (5,'/lib/sub/d.png',2,0,1,14,'2024-01-02');
             INSERT INTO editing_events(id,action,edited_at) VALUES (1,'Edit',1),(2,'Edit',2);
             INSERT INTO recently_edited(photo_id,event_id,edited_at) VALUES (4,1,1),(3,2,2);",
        )
        .unwrap();
        let columns: Vec<String> = c
            .prepare("PRAGMA table_info(albums)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        if !columns.iter().any(|name| name == "rules") {
            c.execute("ALTER TABLE albums ADD COLUMN rules TEXT", []).unwrap();
        }
        c.execute(
            "INSERT INTO albums(id,name,rules) VALUES (7,'Best','{\"categories\":{},\"rating\":\"5\"}')",
            [],
        )
        .unwrap();
        c
    }

    fn ids(connection: &Connection, request: &PhotoListRequest) -> Vec<i64> {
        photo_list(connection, request).unwrap().photos.iter().map(|photo| photo.id).collect()
    }

    fn view(connection: &Connection, filter: SidebarFilter) -> PhotoListRequest {
        PhotoListRequest::for_view(connection, filter, "", BY_NAME)
    }

    #[test]
    fn every_destination_lists_its_members_in_display_order() {
        let c = library();
        for (filter, expected) in [
            // By name (case-insensitive): a.jpg, a.NEF, b.png, c.jpg, d.png.
            (SidebarFilter::All, vec![2, 1, 3, 4, 5]),
            (SidebarFilter::Favorites, vec![1, 3, 5]),
            (SidebarFilter::Ratings, vec![2, 1, 4]),
            (SidebarFilter::Rating(3), vec![4]),
            // The Smart Album's saved rule, not the toolbar.
            (SidebarFilter::Album(7), vec![2, 1]),
            // History keeps its most recent edit first, whatever the sort.
            (SidebarFilter::History, vec![3, 4]),
            // The Folder stream: sections in folder order (/lib, then /lib/sub).
            (SidebarFilter::Folder(2), vec![2, 1, 3, 4, 5]),
        ] {
            assert_eq!(ids(&c, &view(&c, filter)), expected, "{filter:?}");
        }
        assert!(photo_list(&c, &view(&c, SidebarFilter::Albums)).is_none());
        assert!(photo_list(&c, &view(&c, SidebarFilter::Library)).is_none());
        // A search is library-wide and sorted, History included.
        let search = PhotoListRequest::for_view(&c, SidebarFilter::History, "sub", BY_NAME);
        assert_eq!(ids(&c, &search), [4, 5]);
    }

    #[test]
    fn formats_pairing_and_the_toolbar_rating_apply_everywhere() {
        let c = library();
        crate::image_format::set_enabled(&c, crate::image_format::for_path("x.png").unwrap(), false).unwrap();
        crate::image_format::set_raw_jpeg_pair_mode(&c, RawJpegPairMode::PreferRaw).unwrap();
        db::set_setting(&c, RATING_FILTER_SETTING_KEY, "5").unwrap();
        let list = photo_list(&c, &view(&c, SidebarFilter::All)).unwrap();
        // PNGs hidden, the pair collapsed to its RAW, the toolbar keeps 5 stars.
        assert_eq!(list.photos.iter().map(|photo| photo.id).collect::<Vec<_>>(), [1]);
        assert_eq!(list.pair_mode, RawJpegPairMode::PreferRaw);
        assert!(list.pair_folders.is_empty(), "only the Folder stream offers the toggle");
        crate::image_format::set_raw_jpeg_pair_mode(&c, RawJpegPairMode::PreferJpeg).unwrap();
        assert_eq!(ids(&c, &view(&c, SidebarFilter::Album(7))), [2]);
        // Ratings ignores the toolbar rating (round-three bug 1).
        db::set_setting(&c, RATING_FILTER_SETTING_KEY, "unrated").unwrap();
        assert_eq!(ids(&c, &view(&c, SidebarFilter::Ratings)), [2, 4]);
    }

    #[test]
    fn recently_added_keeps_the_newest_then_sorts_and_only_without_a_search() {
        let c = library();
        db::set_setting(&c, db::RECENTLY_ADDED_LIMIT_SETTING_KEY, "2").unwrap();
        // The two newest (5, then 4), shown by name.
        assert_eq!(ids(&c, &view(&c, SidebarFilter::RecentlyAdded)), [4, 5]);
        let search = PhotoListRequest::for_view(&c, SidebarFilter::RecentlyAdded, "lib", BY_NAME);
        assert_eq!(ids(&c, &search).len(), 5);
    }

    #[test]
    fn navigation_pages_are_one_folder_sorted_flat_with_favourites_kept() {
        let c = library();
        let page = |favorites_only| PhotoListRequest {
            filter: SidebarFilter::Folder(2),
            scope: db::BrowseScope { destination: db::Destination::Folder(2), ..db::BrowseScope::default() },
            sort: PhotoSort { field: SortField::Name, direction: SortDirection::Descending },
            favorites_only,
        };
        // Not the stream: only /lib/sub, in the chosen order.
        assert_eq!(ids(&c, &page(false)), [5, 4]);
        assert_eq!(ids(&c, &page(true)), [5]);
        assert!(photo_list_has_photos(&c, &page(true)));
        let empty = PhotoListRequest {
            scope: db::BrowseScope { rating: RatingFilter::One, ..page(false).scope },
            ..page(false)
        };
        assert!(!photo_list_has_photos(&c, &empty));
    }

    #[test]
    fn folder_stream_offers_the_pair_toggle_for_paired_folders() {
        let c = library();
        c.execute("UPDATE folders SET raw_jpeg_pair_count=1 WHERE id=1", []).unwrap();
        let stream = photo_list(&c, &view(&c, SidebarFilter::Folder(1))).unwrap();
        assert_eq!(stream.pair_folders, HashSet::from([1]));
    }

    #[test]
    fn corrupt_filter_uses_the_same_status_as_the_badge() {
        let directory = std::env::temp_dir().join(format!("pic-corrupt-filter-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let good_path = directory.join("good.jpg");
        let bad_path = directory.join("bad.jpg");
        std::fs::write(&good_path, [0xff, 0xd8, 0xff, 0xe0]).unwrap();
        std::fs::write(&bad_path, b"not image data").unwrap();
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(db::SCHEMA).unwrap();
        for (id, path) in [(1, &good_path), (2, &bad_path)] {
            c.execute("INSERT INTO photos(id,path) VALUES (?1,?2)", rusqlite::params![id, path.to_str().unwrap()])
                .unwrap();
        }
        db::set_setting(&c, RATING_FILTER_SETTING_KEY, "corrupt").unwrap();
        assert_eq!(ids(&c, &view(&c, SidebarFilter::All)), [2]);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
