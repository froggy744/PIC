# Issues 191 and 192: missing Favourite icons

Sources: [#191](https://github.com/froggy744/PIC/issues/191) and [#192](https://github.com/froggy744/PIC/issues/192). Both are open bug reports. This is an investigation checklist; neither symptom has been confirmed in a running app.

## Code findings (2026-10-14)

- Sorting replaces the photo model. In the same-set and progressive paths, replacement explicitly clears the selected photo, then selects position 0. A sort that leaves position 0 selected may produce no GTK selection-change signal to restore the InfoBar. The replacement paths now explicitly publish the first selected photo.
- The Favourites filter deliberately hides tile hearts. Previously that visibility choice lived only on attached tiles. Photo Wall and Masonry pool detached tiles, so a pooled tile could carry the previous filter's choice. The gallery now stores the choice and applies it whenever Grid, Folder, Photo Wall or Masonry binds a tile.
- A source review found rating, edited, offline and corrupt badges updated from the bound photo in `SquareTile`; selection styling is reapplied when the sectioned surface binds or refreshes a tile. No second missing-icon path was established from source review alone.
- `cargo check` and `git diff --check` completed successfully after these changes. The visual reproductions and icon audit below remain open.

## Establish exactly what disappears

- [ ] Use a library with at least two favourited photos, including one rated photo. Record app version, theme, view (Grid, Photo Wall, Masonry), active filter, sort field and direction, and whether the missing heart is on a tile or in the bottom InfoBar. Capture before and after screenshots.
- [ ] For #191, enter Favourites, select a photo, change the sort field, then reverse sort direction. Note whether the selected photo, filename, InfoBar Favourite button, and any tile heart disappear immediately or only when the asynchronous refresh finishes. Leave Favourites and return, then compare.
- [ ] For #192, compare the *same photo* in All Photos across Grid, Photo Wall and Masonry. Repeat after visiting Favourites, switching views, scrolling tiles out and back, and restarting. Include an unfavourited control photo.
- [ ] Check the intended Favourites presentation before calling a missing **tile** heart a defect: `apply_gallery_grouping()` deliberately hides tile hearts while the Favourites filter is active. The InfoBar Favourite button should still be available for a selected photo.

## Trace state from the photo to the icon

- [ ] Confirm `db::photos()` returns the expected `favorite` value, then check `PhotoObject::favorite()` for the same ID before and after sort or filter changes. If it changes, trace `refresh_grid_inner()` and `Gallery::replace_owned_while_current()` through the same-set reorder and progressive replacement paths.
- [ ] If #191 concerns the **InfoBar button**, watch the selection callback in `grid/view.rs` and the `selected(None)` calls in `grid/virtualization.rs` during sort. Confirm a new selected photo reaches the callback in `window/build.rs` and `InfoBar::set_photo()` after the refreshed model is installed. Check whether multi-selection changes the InfoBar presentation.
- [ ] If #191 concerns a **tile badge**, check whether the reported behavior matches the Favourites-specific hide rule in `window.rs`. If it does, settle the expected design before changing code.
- [ ] For #192, inspect each realized `SquareTile`: bound photo ID, `favorite()`, `favorite_indicators_visible`, presence and `is_visible()` of the `favorite-badge` overlay. Compare a newly made tile, a pooled tile reused after leaving Favourites, and a tile rebound after sorting. `SquareTile::new()` defaults the flag to true, while `Gallery::set_favorite_indicators_visible()` visits only tiles in the current widget trees; a detached tile pool may retain an older value. Treat this as a hypothesis until measured.
- [ ] If the badge exists and is visible but cannot be seen, inspect overlay allocation, `SquareTile::snapshot()`, Photo Wall/Masonry clipping, and the active theme's `.favorite-badge` colour. Compare with Grid at the same thumbnail size.
- [ ] Repeat the visibility and allocation checks for rating, edited, selection, offline and corrupt indicators, as #192 requests a wider icon audit. Record each icon's expected conditions so absence is not confused with an unset state.

## Fix only the confirmed failing path

- [ ] Write the smallest reproduction around the failing transition: Favourites sort and InfoBar selection for #191; view/filter changes and tile creation/recycling for #192. Confirm it fails before editing behavior.
- [ ] Correct the owner of the stale state or missed refresh, then confirm the same photo displays the right icon immediately after the transition and after scrolling/recycling. Preserve the deliberate Favourites tile-badge rule unless the desired behavior is changed.
- [ ] Verify Grid, Photo Wall and Masonry with default and alternate themes; test sorting, filter and view switches, favourite toggle, rating alongside the heart, multi-selection, and a restart. Check that the other audited icons still appear under their expected conditions.
- [ ] Update each issue with the observed root cause, reproduction, fix location and verification evidence. Close an issue only when its reported symptom is resolved.
