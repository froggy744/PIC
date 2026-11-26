//! Photo mark commands: every decision about what a favourite or rating
//! change does, in one place and free of GTK.
//!
//! Five controls change marks: the InfoBar heart and stars, the digit keys,
//! and the photo menu's favourite item. Each reports a [`MarkIntent`] for the
//! photo it acts on (the anchor). [`plan`] turns that into a [`MarkPlan`]:
//! which photos change, their new value, and whether the change takes photos
//! out of the view being browsed. The GTK adapter writes the plan once and
//! refreshes the widgets. Marks are never cached here: the adapter reads them
//! from the photo objects for every request.

use super::{RatingFilter, SortField};
use crate::db::filters::{AlbumRules, Field, Filters};
use crate::sidebar::SidebarFilter;

/// What the user asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MarkIntent {
    /// The heart and the photo menu's favourite item.
    ToggleFavorite,
    /// A star button, 1 to 5.
    ToggleRating(u8),
    /// A digit key, 0 (clear) to 5.
    SetRating(u8),
}

/// A photo's marks as the adapter read them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PhotoMarks {
    pub(super) id: i64,
    pub(super) favorite: bool,
    pub(super) rating: i32,
}

/// The surface the user is acting on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MarkFront {
    /// The gallery: the selection when it holds the anchor, else the anchor.
    Gallery,
    /// The lightbox: only the photo on screen.
    Lightbox,
    /// Picking photos for a collage: the ticked photos, even none.
    Collage,
}

#[derive(Clone, Debug)]
pub(super) struct MarkRequest<'a> {
    pub(super) intent: MarkIntent,
    pub(super) front: MarkFront,
    pub(super) anchor: PhotoMarks,
    /// The gallery selection (or the photos ticked for a collage).
    pub(super) selection: &'a [PhotoMarks],
    pub(super) view: MarkView<'a>,
}

/// What decides the gallery's membership and order.
#[derive(Clone, Copy, Debug)]
pub(super) struct MarkView<'a> {
    pub(super) sidebar: SidebarFilter,
    pub(super) rating: RatingFilter,
    pub(super) sort: SortField,
    /// The Photo filters (metadata) panel's saved filters.
    pub(super) metadata: &'a Filters,
    /// The open Smart Album's saved rules, which decide its membership like
    /// filters do. `None` outside a Smart Album.
    pub(super) album_rules: Option<&'a AlbumRules>,
}

/// What the change does to the photos being browsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Membership {
    /// Membership and order are unchanged: update the marks in place.
    Keep,
    /// Every changed photo leaves the view (unfavourite in Favourites):
    /// remove them without rebuilding, keeping the viewport.
    Remove,
    /// Membership or order may change: rebuild the gallery, then drop the
    /// photos that left it from the lightbox.
    Rebuild,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MarkChange {
    Favorite(bool),
    /// 0 clears the rating.
    Rating(i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MarkPlan {
    pub(super) ids: Vec<i64>,
    pub(super) change: MarkChange,
    pub(super) membership: Membership,
}

/// The plan for `request`, or `None` when there is nothing to change.
pub(super) fn plan(request: &MarkRequest) -> Option<MarkPlan> {
    let targets = targets(request);
    if targets.is_empty() {
        return None;
    }
    let change = match request.intent {
        // Remove only when every target already is one; a single photo toggles.
        MarkIntent::ToggleFavorite => {
            MarkChange::Favorite(!targets.iter().all(|photo| photo.favorite))
        }
        // Clear only when every target already has this rating.
        MarkIntent::ToggleRating(stars @ 1..=5) => {
            let stars = i32::from(stars);
            let shared = targets.iter().all(|photo| photo.rating == stars);
            MarkChange::Rating(if shared { 0 } else { stars })
        }
        MarkIntent::SetRating(stars @ 0..=5) => MarkChange::Rating(i32::from(stars)),
        MarkIntent::ToggleRating(_) | MarkIntent::SetRating(_) => return None,
    };
    Some(MarkPlan {
        ids: targets.iter().map(|photo| photo.id).collect(),
        change,
        membership: membership(change, &request.view),
    })
}

fn membership(change: MarkChange, view: &MarkView) -> Membership {
    let metadata_on = |field| {
        std::iter::once(&view.metadata.categories)
            .chain(view.album_rules.map(|rules| &rules.categories))
            .any(|categories| {
                categories
                    .get(&field)
                    .is_some_and(|selection| !selection.is_empty())
            })
    };
    let album_rating_rule =
        view.album_rules.is_some_and(|rules| rules.rating_filter() != RatingFilter::All);
    match change {
        MarkChange::Favorite(false) if view.sidebar == SidebarFilter::Favorites => {
            Membership::Remove
        }
        MarkChange::Favorite(_) if metadata_on(Field::Favorite) => Membership::Rebuild,
        MarkChange::Favorite(_) => Membership::Keep,
        MarkChange::Rating(_)
            if view.rating == RatingFilter::AllStars
                || view.rating.rating().is_some()
                || matches!(
                    view.sidebar,
                    SidebarFilter::Ratings | SidebarFilter::Rating(_)
                )
                || view.sort == SortField::Rating
                || metadata_on(Field::Rating)
                || album_rating_rule =>
        {
            Membership::Rebuild
        }
        MarkChange::Rating(_) => Membership::Keep,
    }
}

fn targets<'a>(request: &'a MarkRequest) -> &'a [PhotoMarks] {
    let anchor = std::slice::from_ref(&request.anchor);
    match request.front {
        MarkFront::Lightbox => anchor,
        MarkFront::Collage => request.selection,
        MarkFront::Gallery => {
            if request
                .selection
                .iter()
                .any(|photo| photo.id == request.anchor.id)
            {
                request.selection
            } else {
                anchor
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::filters::Selection;

    static NO_FILTERS: Filters = Filters {
        categories: std::collections::BTreeMap::new(),
    };

    fn view() -> MarkView<'static> {
        MarkView {
            sidebar: SidebarFilter::All,
            rating: RatingFilter::All,
            sort: SortField::DateTaken,
            metadata: &NO_FILTERS,
            album_rules: None,
        }
    }

    #[test]
    fn smart_album_rules_on_marks_rebuild_the_album() {
        let favourites = AlbumRules {
            categories: [(Field::Favorite, Selection { values: vec!["1".into()], ..Default::default() })]
                .into_iter()
                .collect(),
            rating: String::new(),
        };
        // A saved rule written as "0" (Unrated) is a rating rule too.
        let rated = AlbumRules { rating: "stars".into(), ..Default::default() };
        let unrated = AlbumRules { rating: "0".into(), ..Default::default() };
        let camera = AlbumRules {
            categories: [(Field::Camera, Selection { values: vec!["x".into()], ..Default::default() })]
                .into_iter()
                .collect(),
            rating: String::new(),
        };
        let cases: [(&str, MarkIntent, &AlbumRules, Membership); 6] = [
            ("favourite rule, heart", MarkIntent::ToggleFavorite, &favourites, Membership::Rebuild),
            ("favourite rule, stars", MarkIntent::SetRating(3), &favourites, Membership::Keep),
            ("rating rule, stars", MarkIntent::SetRating(3), &rated, Membership::Rebuild),
            ("rating rule, heart", MarkIntent::ToggleFavorite, &rated, Membership::Keep),
            ("saved \"0\" rule, stars", MarkIntent::SetRating(2), &unrated, Membership::Rebuild),
            ("camera rule, stars", MarkIntent::SetRating(2), &camera, Membership::Keep),
        ];
        for (name, intent, rules, expected) in cases {
            let anchor = marks(1, false, 5);
            let request = MarkRequest {
                intent,
                front: MarkFront::Gallery,
                anchor,
                selection: &[anchor],
                view: MarkView { sidebar: SidebarFilter::Album(7), album_rules: Some(rules), ..view() },
            };
            assert_eq!(plan(&request).map(|plan| plan.membership), Some(expected), "case: {name}");
        }
    }

    fn marks(id: i64, favorite: bool, rating: i32) -> PhotoMarks {
        PhotoMarks {
            id,
            favorite,
            rating,
        }
    }

    #[test]
    fn a_single_photo_favourite_toggles() {
        for (favorite, expected) in [(false, true), (true, false)] {
            let anchor = marks(7, favorite, 0);
            let request = MarkRequest {
                intent: MarkIntent::ToggleFavorite,
                front: MarkFront::Gallery,
                anchor,
                selection: &[anchor],
                view: view(),
            };
            assert_eq!(
                plan(&request),
                Some(MarkPlan {
                    ids: vec![7],
                    change: MarkChange::Favorite(expected),
                    membership: Membership::Keep,
                }),
                "favourite was {favorite}"
            );
        }
    }

    #[test]
    fn the_front_decides_which_photos_change() {
        let anchor = marks(1, false, 0);
        let other = marks(2, false, 0);
        let outside = marks(3, false, 0);
        let cases: [(&str, MarkFront, &[PhotoMarks], Option<Vec<i64>>); 6] = [
            (
                "gallery selection holding the anchor",
                MarkFront::Gallery,
                &[anchor, other],
                Some(vec![1, 2]),
            ),
            (
                "gallery selection without the anchor",
                MarkFront::Gallery,
                &[other, outside],
                Some(vec![1]),
            ),
            (
                "gallery with nothing selected",
                MarkFront::Gallery,
                &[],
                Some(vec![1]),
            ),
            (
                "lightbox ignores the gallery selection",
                MarkFront::Lightbox,
                &[anchor, other],
                Some(vec![1]),
            ),
            (
                "collage uses the ticked photos",
                MarkFront::Collage,
                &[other, outside],
                Some(vec![2, 3]),
            ),
            ("collage with nothing ticked", MarkFront::Collage, &[], None),
        ];
        for (name, front, selection, expected) in cases {
            let request = MarkRequest {
                intent: MarkIntent::ToggleFavorite,
                front,
                anchor,
                selection,
                view: view(),
            };
            assert_eq!(
                plan(&request).map(|plan| plan.ids),
                expected,
                "case: {name}"
            );
        }
    }

    #[test]
    fn a_selection_is_unfavourited_only_when_every_photo_is_a_favourite() {
        let cases: [(&str, bool, [bool; 3], bool); 4] = [
            ("none are favourites", false, [false, false, false], true),
            ("some are, anchor is", true, [true, false, true], true),
            ("some are, anchor is not", false, [false, true, true], true),
            ("all are favourites", true, [true, true, true], false),
        ];
        for (name, anchor_favorite, favorites, expected) in cases {
            let anchor = marks(1, anchor_favorite, 0);
            let selection = [anchor, marks(2, favorites[1], 0), marks(3, favorites[2], 0)];
            let request = MarkRequest {
                intent: MarkIntent::ToggleFavorite,
                front: MarkFront::Gallery,
                anchor,
                selection: &selection,
                view: view(),
            };
            assert_eq!(
                plan(&request).map(|plan| plan.change),
                Some(MarkChange::Favorite(expected)),
                "case: {name}"
            );
        }
    }

    #[test]
    fn stars_toggle_a_shared_rating_and_digits_always_set() {
        let cases: [(&str, MarkIntent, &[i32], Option<i32>); 9] = [
            (
                "star on another rating",
                MarkIntent::ToggleRating(3),
                &[1],
                Some(3),
            ),
            (
                "star on the same rating clears",
                MarkIntent::ToggleRating(3),
                &[3],
                Some(0),
            ),
            (
                "star on a selection sharing it clears",
                MarkIntent::ToggleRating(3),
                &[3, 3],
                Some(0),
            ),
            (
                "star on a mixed selection sets",
                MarkIntent::ToggleRating(3),
                &[3, 4],
                Some(3),
            ),
            (
                "digit on the same rating keeps it",
                MarkIntent::SetRating(3),
                &[3],
                Some(3),
            ),
            ("digit 0 clears", MarkIntent::SetRating(0), &[4, 2], Some(0)),
            (
                "star 0 is not a rating",
                MarkIntent::ToggleRating(0),
                &[2],
                None,
            ),
            (
                "star 6 is not a rating",
                MarkIntent::ToggleRating(6),
                &[2],
                None,
            ),
            (
                "digit 6 is not a rating",
                MarkIntent::SetRating(6),
                &[2],
                None,
            ),
        ];
        for (name, intent, ratings, expected) in cases {
            let selection = ratings
                .iter()
                .enumerate()
                .map(|(index, rating)| marks(index as i64 + 1, false, *rating))
                .collect::<Vec<_>>();
            let request = MarkRequest {
                intent,
                front: MarkFront::Gallery,
                anchor: selection[0],
                selection: &selection,
                view: view(),
            };
            assert_eq!(
                plan(&request).map(|plan| plan.change),
                expected.map(MarkChange::Rating),
                "case: {name}"
            );
        }
    }

    fn metadata(field: Field) -> Filters {
        let mut filters = Filters::default();
        filters.categories.insert(
            field,
            Selection {
                values: vec!["1".into()],
                ..Selection::default()
            },
        );
        filters
    }

    #[test]
    fn a_change_that_can_move_photos_out_of_the_view_says_so() {
        let none = Filters::default();
        let favorite_filter = metadata(Field::Favorite);
        let rating_filter = metadata(Field::Rating);
        let camera_filter = metadata(Field::Camera);
        let all = (SidebarFilter::All, RatingFilter::All, SortField::DateTaken);
        let cases: [(
            &str,
            MarkIntent,
            bool,
            (SidebarFilter, RatingFilter, SortField),
            &Filters,
            Membership,
        ); 16] = [
            (
                "favourite in All",
                MarkIntent::ToggleFavorite,
                false,
                all,
                &none,
                Membership::Keep,
            ),
            (
                "unfavourite in Favourites",
                MarkIntent::ToggleFavorite,
                true,
                (
                    SidebarFilter::Favorites,
                    RatingFilter::All,
                    SortField::DateTaken,
                ),
                &none,
                Membership::Remove,
            ),
            (
                "favourite in Favourites",
                MarkIntent::ToggleFavorite,
                false,
                (
                    SidebarFilter::Favorites,
                    RatingFilter::All,
                    SortField::DateTaken,
                ),
                &none,
                Membership::Keep,
            ),
            (
                "unfavourite in Favourites under a favourite filter",
                MarkIntent::ToggleFavorite,
                true,
                (
                    SidebarFilter::Favorites,
                    RatingFilter::All,
                    SortField::DateTaken,
                ),
                &favorite_filter,
                Membership::Remove,
            ),
            (
                "favourite under a metadata favourite filter",
                MarkIntent::ToggleFavorite,
                false,
                all,
                &favorite_filter,
                Membership::Rebuild,
            ),
            (
                "favourite under an unrelated metadata filter",
                MarkIntent::ToggleFavorite,
                false,
                all,
                &camera_filter,
                Membership::Keep,
            ),
            (
                "favourite under a rating filter",
                MarkIntent::ToggleFavorite,
                false,
                (SidebarFilter::All, RatingFilter::Five, SortField::Rating),
                &rating_filter,
                Membership::Keep,
            ),
            (
                "rate in All",
                MarkIntent::SetRating(3),
                false,
                all,
                &none,
                Membership::Keep,
            ),
            (
                "rate under a star filter",
                MarkIntent::SetRating(3),
                false,
                (SidebarFilter::All, RatingFilter::Five, SortField::DateTaken),
                &none,
                Membership::Rebuild,
            ),
            (
                "rate under All Stars",
                MarkIntent::SetRating(0),
                false,
                (
                    SidebarFilter::All,
                    RatingFilter::AllStars,
                    SortField::DateTaken,
                ),
                &none,
                Membership::Rebuild,
            ),
            (
                "rate under Unrated",
                MarkIntent::SetRating(2),
                false,
                (
                    SidebarFilter::All,
                    RatingFilter::Unrated,
                    SortField::DateTaken,
                ),
                &none,
                Membership::Rebuild,
            ),
            (
                "rate under the Online filter",
                MarkIntent::SetRating(2),
                false,
                (
                    SidebarFilter::All,
                    RatingFilter::Online,
                    SortField::DateTaken,
                ),
                &none,
                Membership::Keep,
            ),
            (
                "rate in the Ratings sidebar",
                MarkIntent::SetRating(0),
                false,
                (
                    SidebarFilter::Ratings,
                    RatingFilter::All,
                    SortField::DateTaken,
                ),
                &none,
                Membership::Rebuild,
            ),
            (
                "rate in a sidebar rating",
                MarkIntent::SetRating(2),
                false,
                (
                    SidebarFilter::Rating(5),
                    RatingFilter::All,
                    SortField::DateTaken,
                ),
                &none,
                Membership::Rebuild,
            ),
            (
                "rate when sorted by rating",
                MarkIntent::SetRating(2),
                false,
                (SidebarFilter::All, RatingFilter::All, SortField::Rating),
                &none,
                Membership::Rebuild,
            ),
            (
                "rate under a metadata rating filter",
                MarkIntent::SetRating(2),
                false,
                all,
                &rating_filter,
                Membership::Rebuild,
            ),
        ];
        for (name, intent, favorite, (sidebar, rating, sort), filters, expected) in cases {
            let anchor = marks(1, favorite, 5);
            let request = MarkRequest {
                intent,
                front: MarkFront::Gallery,
                anchor,
                selection: &[anchor],
                view: MarkView {
                    sidebar,
                    rating,
                    sort,
                    metadata: filters,
                    album_rules: None,
                },
            };
            assert_eq!(
                plan(&request).map(|plan| plan.membership),
                Some(expected),
                "case: {name}"
            );
        }
    }
}
