# Handoff: round-three review bugs

Branch: `fix/round-three-bugs` (from `main` @ `89b7b79f`, after the Smart Albums merge).

Architecture review round three (`docs/architecture/STATUS.md`, candidates 14-20) is **deferred**. The owner wants the bugs it found fixed **one at a time**, as small targeted fixes. Do not start a refactor candidate (BrowseScope, photo-list pipeline, PhotoChanged, AlbumList) to fix them. When a fix would need one, stop and ask.

The bugs were read from the code during the review. None of them has been reproduced yet. Line numbers are at `89b7b79f`.

## Per bug

1. **Reproduce first.** Write the targeted test and watch it fail (red) before you change code. If it will not go red, the bug may not exist: report that and move on.
2. Fix the root cause in the owning code.
3. The test goes green. Run `cargo check --locked`.
4. Make one commit per bug. Commit, push or merge only when the owner says that word.

**Done:** every bug below is either fixed with a green test or reported as not reproducible.

## Progress

| Bug | State | Commit | Test |
|---|---|---|---|
| 1 | Fixed | `18923e3e` | `sidebar_rating_destinations_are_independent_of_toolbar_and_favourites` (extended), `toolbar_rating_filter_shows_it_is_off_in_ratings` (display) |
| 2 | Fixed | `24297e1b` | `trashing_photos_updates_library_counts` (display) |
| 3 | Fixed | `5d83b50c` | `rotating_in_the_viewer_refreshes_the_viewer_properties` (display) |
| 4 | Fixed | `d3d2252a` | `startup_restores_history_in_history_order` |
| 5 | Fixed | `ab33310c` | `scan_appends_new_photos_only_to_an_unfiltered_view` |

**Done** (2026-11-26): all five fixed. The branch is pushed and waits for the owner's merge.

Notes from the fixes:
- Bug 1 also covers the display: on Ratings and Rating(n) the toolbar rating button is insensitive with the tooltip "Rating filter is not used in Ratings", and the metadata row's scope drops the rating so its counts match the grid. The per-destination query is now `collection_photos` (`library.rs`); bug 4 can reuse it.
- Bug 2's display test trashes real files into a private `XDG_DATA_HOME` under `target/` (GIO refuses to trash on tmpfs).
- Each display test was also checked red with its fix removed.
- Bug 4: startup's photo list is now `startup_photos` (`library.rs`), querying through `collection_photos`; History keeps its own order. The library switch still builds its own list (candidate 15).
- Bug 5: only *appending* new photos checks the toolbar rating (`scan_may_append_new_photos`); in-place updates of shown photos keep the metadata-only gate, since a Folder refresh with no new photos has no final rebuild. Its test covers the decision, not the scan handler in `build()`.

## Bugs, in suggested order

### 1. Sidebar Ratings view is empty when the toolbar rating is Unrated
- **Where:** `src/window/library.rs:659` (`apply_collection_rating_filter`).
- **Cause:** the SQL rating predicate comes from the `photo-rating-filter` setting via `Filters::for_browsing` (`src/db/filters.rs`, `rating_predicate`). The SQL keeps only `rating=0`, then the Ratings view keeps only 1-5, so nothing is left.
- **Intended behaviour:** the sidebar rating destinations (Ratings, Rating(n)) ignore the toolbar rating. That is what the existing test name `sidebar_rating_destinations_are_independent_of_toolbar_and_favourites` (`library.rs` tests) promises.
- **Why that test misses it:** it never saves the toolbar setting. Extend it by saving `photo-rating-filter` = `unrated` and then `3`.

### 2. Trashing a photo does not update the library counts
- **Where:** `src/window/photo_actions.rs:1500-1537`, the trash confirmation response.
- **Fix:** after a successful trash, refresh the sidebar library counts the way `update_counts` in `src/window/photo_mark_commands.rs` does: `db::sidebar_counts` + `sidebar::update_library_counts`.

### 3. Rotating in the viewer refreshes the wrong Properties panel
- **Where:** `src/window/build.rs:4674` (`rotate_photo`). It always calls `properties_for_rotate.show_photo(PropertiesSurface::Gallery, ..)`.
- **Fix:** pick the surface from the front, the way `show_updated` in `src/window/photo_mark_commands.rs` does (Lightbox when the viewer is shown, else Gallery).
- **Guard:** Properties is owned by `properties_widgets.rs`. Run `scripts/check-presentation-owners.sh` after the change.

### 4. A restored History view shows all photos
- **Where:** `src/window/build.rs:118-160`, the initial photo list for the saved view. It has no `SidebarFilter::History` arm and falls through to `browse_photos`.
- **Reference:** `refresh_grid_inner` (`src/window/library.rs:176`) has the right per-filter query.
- **Fix:** make startup use that query for History. Do not rebuild the whole pipeline; that is candidate 15.

### 5. Scan appends new photos to a rating-filtered view
- **Where:** `src/window/build.rs:6556` onwards (`ScanEvent::PhotosIndexed`, the append path around 6562-6590).
- **Cause:** the append path decides whether the view is narrowed with `Filters::load(..).is_empty()`, which ignores the toolbar rating.
- **Fix:** when a rating filter is active, rebuild the view instead of appending (or check each photo's rating).
- **Reference:** `history.rs:70` and `albums/card.rs:415` already use the full "narrowed" check (filters empty and rating `""`/`all`).

## Testing

- Use the commands in `AGENTS.md`.
- Run display tests only under Xvfb, one test per process.
- Known pre-existing failures are listed there too; leave them alone.
