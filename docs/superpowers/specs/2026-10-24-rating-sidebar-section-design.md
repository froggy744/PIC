# Ratings Sidebar Collection

## Behavior

- Ratings sits below Favourites in the Library list and uses the same row styling, hover feedback, and selection highlight.
- A single click on Ratings opens a library-wide collection of all nontrashed photos rated 1–5 stars. It does not change the toolbar metadata rating filter.
- A double-click on the Ratings title or a click on its arrow expands or collapses five child rows. A single click on a child opens the library-wide collection for that exact star rating.
- The child rows show library-wide counts, update after rating changes and scans, and align with the Ratings title without a nested frame.
- Ratings and its child rows are saved sidebar destinations. Switching to Favourites, albums, folders, or other sections uses their own collection membership. The toolbar metadata filter remains independent and keeps its existing behavior in those sections.

## Scope

- There is no Unrated child row.
- Photo ratings, sort order, and edit recipes are unchanged.

## Verification

- Test rating counts with mixed ratings and trashed photos.
- Test the sidebar rows, disclosure, navigation selection, and count updates under GTK.
- Test that Ratings and an exact star row ignore the toolbar filter while Favourites retains all its photos after navigation.
- Test saved destination keys and run `cargo check --locked` and `git diff --check`.
