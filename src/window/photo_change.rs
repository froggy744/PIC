//! Photo change: every decision about what a change to some photos refreshes,
//! in one place and free of GTK.
//!
//! Eight places change photos: marks, the editor's save, bulk edits, rotate,
//! trash, rename, tags and the scanner's updates. Each writes the change,
//! then reports it to the adapter (`photo_change_widgets`) as one
//! notification. The adapter always gives the new values to the gallery's
//! and the viewer's photo objects; [`plan`] decides the rest from the
//! [`ChangeKind`] and the [`ChangeView`]: whether the shown photo's InfoBar
//! and Properties are rewritten or cleared, whether the Library and album
//! counts are recounted, and whether the photos being browsed change.

use super::photo_marks::Membership;
use super::{PropertiesSurface, SortField};
use crate::db::filters::{AlbumRules, Field, Filters};
use crate::sidebar::SidebarFilter;

/// What changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChangeKind {
    /// A favourite or rating; `photo_marks::plan` already decided the
    /// membership.
    Marks(Membership),
    /// Edit recipes (the editor's save, pasted or reset edits). A bulk edit
    /// reports each slice with `more_coming`, then its end with no photos.
    Recipes { more_coming: bool },
    /// A quarter turn.
    Rotated,
    /// A new file name.
    Renamed,
    /// A tag added or removed.
    Tagged,
    /// Moved to the system Trash by the user.
    Trashed,
    /// The scanner re-read photos already in the library.
    Indexed,
    /// The scanner found photos gone from disk.
    Removed,
}

/// What the plan reads from the window.
#[derive(Clone, Copy, Debug)]
pub(super) struct ChangeView<'a> {
    /// The viewer is in front, so its Properties panel is the one shown.
    pub(super) lightbox_shown: bool,
    /// The shown photo (the InfoBar's) is among the changed photos.
    pub(super) shown_changed: bool,
    pub(super) sidebar: SidebarFilter,
    pub(super) sort: SortField,
    pub(super) search_active: bool,
    /// The Photo filters (metadata) panel's saved filters.
    pub(super) metadata: &'a Filters,
    /// The open Smart Album's rules; `None` outside a Smart Album.
    pub(super) album_rules: Option<&'a AlbumRules>,
}

/// What happens to the shown photo's InfoBar and Properties panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shown {
    /// Untouched (the shown photo did not change, or its surface moves on
    /// by itself).
    Keep,
    /// Rewrite the InfoBar and this Properties panel.
    Refresh(PropertiesSurface),
    /// No photo to show any more: clear the InfoBar and both panels.
    Clear,
}

/// What the adapter does after giving the new values to the photo objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FanOut {
    pub(super) shown: Shown,
    /// Recount the sidebar's Library counts.
    pub(super) library_counts: bool,
    /// Recount the Smart Albums (batched by the album list).
    pub(super) album_counts: bool,
    pub(super) membership: Membership,
}

/// The filter fields a change can move photos across.
fn fields(kind: ChangeKind) -> &'static [Field] {
    match kind {
        ChangeKind::Recipes { .. } | ChangeKind::Rotated => &[Field::Edited],
        ChangeKind::Renamed => &[Field::FileType],
        ChangeKind::Tagged => &[Field::Tags],
        _ => &[],
    }
}

/// Whether the photos being browsed can change: a filter or the open Smart
/// Album narrows on a changed field, or the order depends on it.
fn narrowed(kind: ChangeKind, view: &ChangeView) -> bool {
    let filtered = fields(kind).iter().any(|field| {
        view.metadata
            .categories
            .get(field)
            .is_some_and(|selection| !selection.is_empty())
            || view
                .album_rules
                .is_some_and(|rules| rules.categories.contains_key(field))
    });
    filtered
        || (matches!(kind, ChangeKind::Recipes { .. }) && view.sidebar == SidebarFilter::History)
        || (kind == ChangeKind::Renamed && (view.search_active || view.sort == SortField::Name))
}

pub(super) fn plan(kind: ChangeKind, view: &ChangeView) -> FanOut {
    let front = if view.lightbox_shown {
        PropertiesSurface::Lightbox
    } else {
        PropertiesSurface::Gallery
    };
    let shown = match kind {
        ChangeKind::Trashed => Shown::Clear,
        ChangeKind::Removed => Shown::Keep,
        _ if view.shown_changed => Shown::Refresh(front),
        _ => Shown::Keep,
    };
    let membership = match kind {
        ChangeKind::Marks(membership) => membership,
        ChangeKind::Trashed => Membership::Rebuild,
        ChangeKind::Removed => Membership::Remove,
        ChangeKind::Indexed | ChangeKind::Recipes { more_coming: true } => Membership::Keep,
        _ if narrowed(kind, view) => Membership::Rebuild,
        _ => Membership::Keep,
    };
    FanOut {
        shown,
        library_counts: matches!(kind, ChangeKind::Marks(_) | ChangeKind::Trashed),
        album_counts: !matches!(
            kind,
            ChangeKind::Indexed | ChangeKind::Recipes { more_coming: true }
        ),
        membership,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::filters::Selection;
    use ChangeKind::*;
    use Membership::{Keep as KeepPhotos, Rebuild, Remove};

    const RECIPES: ChangeKind = Recipes { more_coming: false };

    fn view<'a>(metadata: &'a Filters, album_rules: Option<&'a AlbumRules>) -> ChangeView<'a> {
        ChangeView {
            lightbox_shown: false,
            shown_changed: true,
            sidebar: SidebarFilter::All,
            sort: SortField::DateTaken,
            search_active: false,
            metadata,
            album_rules,
        }
    }

    fn on(field: Field) -> Filters {
        let mut filters = Filters::default();
        filters.categories.insert(
            field,
            Selection {
                values: vec!["x".into()],
                ..Default::default()
            },
        );
        filters
    }

    #[test]
    fn the_shown_photo_is_rewritten_on_the_surface_in_front() {
        let none = Filters::default();
        let gallery = view(&none, None);
        let viewer = ChangeView {
            lightbox_shown: true,
            ..gallery
        };
        let elsewhere = ChangeView {
            shown_changed: false,
            ..viewer
        };
        for (kind, change, expected) in [
            (RECIPES, gallery, Shown::Refresh(PropertiesSurface::Gallery)),
            (Renamed, viewer, Shown::Refresh(PropertiesSurface::Lightbox)),
            (Indexed, viewer, Shown::Refresh(PropertiesSurface::Lightbox)),
            (Tagged, elsewhere, Shown::Keep),
            (Marks(KeepPhotos), elsewhere, Shown::Keep),
            // Trash closes the viewer and leaves nothing to show.
            (Trashed, elsewhere, Shown::Clear),
            (Trashed, viewer, Shown::Clear),
            // Gallery and viewer move their own selection off a removed photo.
            (Removed, viewer, Shown::Keep),
        ] {
            assert_eq!(plan(kind, &change).shown, expected, "{kind:?}");
        }
    }

    #[test]
    fn counts_follow_what_the_change_can_move() {
        let none = Filters::default();
        let change = view(&none, None);
        for (kind, library, albums) in [
            (Marks(KeepPhotos), true, true),
            (Trashed, true, true),
            (RECIPES, false, true),
            (Rotated, false, true),
            (Renamed, false, true),
            (Tagged, false, true),
            (Removed, false, true),
            // The scan recounts once when indexing finishes.
            (Indexed, false, false),
        ] {
            let fan_out = plan(kind, &change);
            assert_eq!(
                (fan_out.library_counts, fan_out.album_counts),
                (library, albums),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn photos_leave_the_view_only_when_it_narrows_on_a_changed_field() {
        let none = Filters::default();
        let (edited, tags, file_type, camera) = (
            on(Field::Edited),
            on(Field::Tags),
            on(Field::FileType),
            on(Field::Camera),
        );
        let mut tag_rules = AlbumRules::default();
        tag_rules
            .categories
            .insert(Field::Tags, Selection::default());
        let plain = view(&none, None);
        for (kind, change, expected) in [
            (Marks(Remove), plain, Remove),
            (Marks(KeepPhotos), plain, KeepPhotos),
            (Trashed, plain, Rebuild),
            (Removed, plain, Remove),
            (Indexed, view(&edited, None), KeepPhotos),
            (RECIPES, plain, KeepPhotos),
            (RECIPES, view(&edited, None), Rebuild),
            // A recipe cannot change a photo's camera.
            (RECIPES, view(&camera, None), KeepPhotos),
            (
                RECIPES,
                ChangeView {
                    sidebar: SidebarFilter::History,
                    ..plain
                },
                Rebuild,
            ),
            (Rotated, view(&edited, None), Rebuild),
            (
                Rotated,
                ChangeView {
                    sidebar: SidebarFilter::History,
                    ..plain
                },
                KeepPhotos,
            ),
            (Tagged, plain, KeepPhotos),
            (Tagged, view(&tags, None), Rebuild),
            (Tagged, view(&none, Some(&tag_rules)), Rebuild),
            (RECIPES, view(&none, Some(&tag_rules)), KeepPhotos),
            (Renamed, plain, KeepPhotos),
            (Renamed, view(&file_type, None), Rebuild),
            (
                Renamed,
                ChangeView {
                    search_active: true,
                    ..plain
                },
                Rebuild,
            ),
            (
                Renamed,
                ChangeView {
                    sort: SortField::Name,
                    ..plain
                },
                Rebuild,
            ),
            (
                Tagged,
                ChangeView {
                    search_active: true,
                    ..plain
                },
                KeepPhotos,
            ),
        ] {
            assert_eq!(
                plan(kind, &change).membership,
                expected,
                "{kind:?} {change:?}"
            );
        }
    }

    #[test]
    fn a_bulk_edit_counts_and_rebuilds_once_at_its_end() {
        let edited = on(Field::Edited);
        let change = view(&edited, None);
        let part = plan(Recipes { more_coming: true }, &change);
        assert_eq!(
            (part.shown, part.album_counts, part.membership),
            (
                Shown::Refresh(PropertiesSurface::Gallery),
                false,
                KeepPhotos
            )
        );
        let end = plan(
            RECIPES,
            &ChangeView {
                shown_changed: false,
                ..change
            },
        );
        assert_eq!(
            (end.shown, end.album_counts, end.membership),
            (Shown::Keep, true, Rebuild)
        );
    }
}
