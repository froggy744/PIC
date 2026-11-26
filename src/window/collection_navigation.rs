//! Collection navigation: which collection Up/Down in the viewer moves to,
//! in one place and free of GTK.
//!
//! The collections are the sidebar's visible destinations in their visible
//! order: library rows, albums (manual and Smart), folder rows. [`next_stop`]
//! walks them from the current one, wrapping at both ends, to the next one
//! that has photos under its own browse scope; it lands on the first photo
//! going forward and on the last going back. Folder rows are sections of the
//! one continuous Folder stream: [`FolderRows`] says which shown row a
//! photo's folder belongs to, and [`landing`] finds a row's first or last
//! photo in the stream.

use std::collections::{HashMap, HashSet};

use crate::sidebar::SidebarFilter;

/// Which end of the target collection the viewer opens on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Land {
    First,
    Last,
}

/// Where Up/Down goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub(super) page: SidebarFilter,
    pub(super) land: Land,
}

/// Destinations the viewer can show as a collection. The Library and Albums
/// homes have no photo grid; History opens its photos in the editor.
fn is_stop(page: SidebarFilter) -> bool {
    !matches!(
        page,
        SidebarFilter::Library | SidebarFilter::Albums | SidebarFilter::History
    )
}

/// The next collection from `current` among `sidebar` (the visible
/// destinations in order) that `has_photos`, wrapping at both ends. `None`
/// when `current` is not one of them or no other collection has photos.
/// `has_photos` is asked lazily, in walking order.
pub(super) fn next_stop(
    sidebar: &[SidebarFilter],
    current: SidebarFilter,
    forward: bool,
    mut has_photos: impl FnMut(SidebarFilter) -> bool,
) -> Option<Target> {
    let mut stops: Vec<SidebarFilter> = Vec::new();
    for page in sidebar.iter().copied().filter(|page| is_stop(*page)) {
        if !stops.contains(&page) {
            stops.push(page);
        }
    }
    let position = stops.iter().position(|page| *page == current)?;
    let count = stops.len();
    (1..count)
        .map(|step| {
            if forward {
                (position + step) % count
            } else {
                (position + count - step) % count
            }
        })
        .map(|index| stops[index])
        .find(|page| has_photos(*page))
        .map(|page| Target {
            page,
            land: if forward { Land::First } else { Land::Last },
        })
}

/// The shown folder rows and the folder tree: which row a photo's folder
/// belongs to.
pub(super) struct FolderRows {
    shown: HashSet<i64>,
    parents: HashMap<i64, i64>,
}

impl FolderRows {
    /// `shown`: the visible folder rows; `folders`: every folder with its
    /// parent.
    pub(super) fn new(shown: &[i64], folders: &[(i64, Option<i64>)]) -> Self {
        Self {
            shown: shown.iter().copied().collect(),
            parents: folders
                .iter()
                .filter_map(|(id, parent)| parent.map(|parent| (*id, parent)))
                .collect(),
        }
    }

    /// The row `folder`'s photos show under: its own row when shown, else
    /// its nearest shown ancestor (a collapsed tree, Imported-only mode).
    pub(super) fn row_of(&self, folder: i64) -> Option<i64> {
        let mut folder = folder;
        let mut seen = HashSet::new();
        loop {
            if self.shown.contains(&folder) {
                return Some(folder);
            }
            if !seen.insert(folder) {
                return None;
            }
            folder = *self.parents.get(&folder)?;
        }
    }
}

/// The index in the stream (each photo's row, in stream order) of `row`'s
/// first or last photo.
pub(super) fn landing(rows: &[Option<i64>], row: i64, land: Land) -> Option<usize> {
    let mut matching = rows
        .iter()
        .enumerate()
        .filter(|(_, photo_row)| **photo_row == Some(row))
        .map(|(index, _)| index);
    match land {
        Land::First => matching.next(),
        Land::Last => matching.last(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SidebarFilter::*;

    const SIDEBAR: [SidebarFilter; 9] = [
        All,
        RecentlyAdded,
        Favorites,
        Ratings,
        History,
        Albums,
        Album(1),
        Album(2),
        Folder(10),
    ];

    fn target(page: SidebarFilter, land: Land) -> Option<Target> {
        Some(Target { page, land })
    }

    #[test]
    fn up_and_down_follow_the_sidebar_and_wrap() {
        let all = |_| true;
        for (current, forward, expected) in [
            (All, true, target(RecentlyAdded, Land::First)),
            (RecentlyAdded, true, target(Favorites, Land::First)),
            (Favorites, false, target(RecentlyAdded, Land::Last)),
            // History and the Albums home are not collections.
            (Ratings, true, target(Album(1), Land::First)),
            (Album(1), false, target(Ratings, Land::Last)),
            (Album(2), true, target(Folder(10), Land::First)),
            // Both ends wrap.
            (Folder(10), true, target(All, Land::First)),
            (All, false, target(Folder(10), Land::Last)),
            // Not a collection in the sidebar: nowhere to move from.
            (Album(9), true, None),
            (History, true, None),
        ] {
            assert_eq!(
                next_stop(&SIDEBAR, current, forward, all),
                expected,
                "{current:?} {forward}"
            );
        }
    }

    #[test]
    fn empty_collections_are_skipped_and_a_lone_one_stays() {
        let with_photos = |pages: &'static [SidebarFilter]| move |page| pages.contains(&page);
        assert_eq!(
            next_stop(&SIDEBAR, All, true, with_photos(&[All, Album(2)])),
            target(Album(2), Land::First)
        );
        assert_eq!(
            next_stop(&SIDEBAR, Album(2), true, with_photos(&[All, Album(2)])),
            target(All, Land::First)
        );
        // Only the current collection has photos: nothing happens.
        assert_eq!(
            next_stop(&SIDEBAR, Album(1), true, with_photos(&[Album(1)])),
            None
        );
        // The current one may be empty under new filters; another still is a target.
        assert_eq!(
            next_stop(&SIDEBAR, Album(1), false, with_photos(&[Folder(10)])),
            target(Folder(10), Land::Last)
        );
    }

    #[test]
    fn collections_are_asked_lazily_in_walking_order() {
        let mut asked = Vec::new();
        let found = next_stop(&SIDEBAR, Favorites, true, |page| {
            asked.push(page);
            page == Album(2)
        });
        assert_eq!(found, target(Album(2), Land::First));
        assert_eq!(asked, [Ratings, Album(1), Album(2)]);
    }

    #[test]
    fn a_photo_belongs_to_its_nearest_shown_folder_row() {
        // 1 ─ 2 ─ 3, and 4 under 1; rows shown: 1 and 2 (3 collapsed).
        let rows = FolderRows::new(
            &[1, 2],
            &[
                (1, None),
                (2, Some(1)),
                (3, Some(2)),
                (4, Some(1)),
                (5, None),
            ],
        );
        for (folder, row) in [
            (1, Some(1)),
            (2, Some(2)),
            (3, Some(2)),
            (4, Some(1)),
            (5, None),
            (99, None),
        ] {
            assert_eq!(rows.row_of(folder), row, "folder {folder}");
        }
    }

    #[test]
    fn a_folder_lands_on_its_first_or_last_photo_in_the_stream() {
        let stream = [Some(1), Some(1), Some(2), None, Some(2), Some(3)];
        for (row, land, index) in [
            (1, Land::First, Some(0)),
            (1, Land::Last, Some(1)),
            (2, Land::First, Some(2)),
            (2, Land::Last, Some(4)),
            (3, Land::Last, Some(5)),
            (7, Land::First, None),
        ] {
            assert_eq!(landing(&stream, row, land), index, "row {row} {land:?}");
        }
    }
}
