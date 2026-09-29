# Photo Wall Implementation Plan

> **For agentic workers:** Use `superpowers:executing-plans` to implement this plan task by task in the current session. Steps use checkboxes for tracking. This document authorizes no implementation by itself.

**Goal:** Add a justified Photo Wall view to PIC using the current photo collection, selection model, thumbnail workers/cache, and recycled tiles, while preserving Grid.

**Architecture:** Keep the library `GtkGridView` unchanged. Extend the existing `SectionedFolderView` virtual surface with a Photo Wall geometry strategy and support for a headerless collection. Use that surface for all Photo Wall sources; Folder Grid continues using its existing geometry strategy. Geometry is pure Rust data, independent of GTK, decoding, and filesystem access.

**Tech stack:** Rust 2021, GTK4 0.10, GLib/GIO 0.21, existing SQLite catalog and thumbnail display workers. No new dependency is planned.

**Spec:** [TODO.photowall.md](../../../TODO.photowall.md). This plan is the requested architecture inspection and planning deliverable; application code has not been changed.

**Review input:** [TODO-PhotoWall-Suggestions.md](../../../TODO-PhotoWall-Suggestions.md). Suggestions are incorporated where consistent with the active checkout and the original requirements. Rechecked `SquareTile::measure`/`size_allocate` in `src/grid/tile.rs` and EXIF axis swapping in `src/scanner.rs`: rectangular support and normalization are already present. Uploaded older copies must not override these findings.

## Architecture findings

- `src/main.rs` declares `mod grid`, which resolves to `src/grid.rs`. That module includes the active `src/grid/{tile,grouping,sectioned_folder,virtualization,view,navigation,selection}.rs` implementation. Root `virtualization.rs` and `src/grid/grid.rs` are not the active module paths; do not modify those copies.
- `Gallery` in `src/grid/view.rs` owns a flat `gio::ListStore`, one `gtk::MultiSelection`, and shared `current_photos`. Library views use a directly scrolled `GtkGridView` with a recycling factory.
- Default Folder mode uses `SectionedFolderView` in `src/grid/sectioned_folder.rs`: a `GtkFixed` surface, spacer defining scroll extent, indexed section geometry, visible tile/header maps, and bounded reuse pools. It renders the viewport plus 320 px overscan. Tile and header pool caps are 180 and 12; these limit retained spare widgets, not the number needed to fill the viewport.
- The sectioned surface already binds `SquareTile`, shares `MultiSelection`, and provides click/Ctrl/Shift selection, keyboard handling, rubberband selection, context menus, activation, folder headers, and RAW/JPEG header controls.
- Folder boundaries are `GroupRange` entries in `src/grid/grouping.rs`. Folder catalog/order and progressive photo replacement are managed in `src/grid/virtualization.rs`. Section geometry currently assumes constant row height and column count.
- `PhotoObject` carries catalog width, height, library rotation, edit recipe, photo ID, and folder identity. `src/scanner.rs` now saves display-oriented dimensions, swapping axes for EXIF orientations 5–8. Library rotation is separate. `src/lightbox/impl.rs` explicitly acknowledges older sensor-oriented catalog dimensions.
- `src/grid/tile.rs` supports independent tile width/height despite the `SquareTile` name. It provides cache-only binding and queues presentation requests through `src/thumbnail_display.rs`; completions update realized tiles in `Gallery::drain_thumbnail_display_completions`.
- The existing eight zoom levels are `[100, 117, 137, 160, 187, 219, 256, 300]`. `InfoBar::grid_zoom` uses indices 0–7, and `Gallery::request_slider_zoom` applies them. Window wiring is in `src/window/build.rs`, not `src/window/toolbar.rs`.
- The bottom-right `view-grid-symbolic` button in `src/infobar.rs` is **Collage**, not a layout switch. Preserve the Collage operation when extending this control.
- `src/window/build.rs` creates separate Grid and Folder scrollers and selects them through `gallery_scroll_stack`. Selection, navigation, thumbnail processing, scroll targeting, and resize observers frequently branch on Folder mode. They need explicit virtual-surface routing for Photo Wall.
- No photo-section collapse handler or photo drag/drop controller was identified in the inspected grid implementation. Preserve existing behavior; do not add either capability as an unrelated feature.

## Approach and trade-offs

**Recommended:** Extend the existing sectioned virtual surface. Its recycled tiles, headers, callbacks, and selection already provide most of the required behavior. Introduce narrow geometry accessors where fixed-column arithmetic currently appears; keep existing Grid formulas and animations on their current branches.

**Alternative:** Build a new custom GTK layout manager around `GtkGridView`. GTK's list virtualization assumes uniform grid cells; making that work with justified rows would require changing the working grid and its scroll behavior.

**Alternative:** Create a separate virtual Photo Wall widget. It could produce the geometry, but would duplicate recycling, input handling, header rendering, and viewport request scheduling. Avoid that duplication.

## Global constraints

- Photo Wall is a view mode, never a Library/sidebar source.
- Use the same photo ordering, IDs, model objects, filters, selected source, and selection model.
- Layout-only changes must retain the exact ListStore, MultiSelection, PhotoObjects, and ordering. Anchor and selection identity are photo IDs; indices are geometry/model positions only. Do not rebuild the model to switch layouts.
- Do not replace existing Grid virtualization or thumbnail loading/caching.
- Preserve original display aspect ratios and apply library rotation once; use `Contain` for Photo Wall so edited presentations cannot distort photographs.
- Catalog width × height means dimensions after embedded EXIF orientation and before PIC user rotation. Photo Wall consumes that invariant; it does not read EXIF or repair legacy records. Existing legacy records require a separate maintenance effort and must remain a reported compatibility limitation until normalized.
- Use a 4 px horizontal and vertical gap, retaining the current 20 px sectioned side margins.
- Justify completed rows; keep incomplete final rows at target height and left aligned. A single image wider than the usable viewport must shrink to fit.
- Flush rows at every folder boundary and preserve existing header names, counts, ordering, and RAW/JPEG controls.
- No filesystem access, EXIF parsing, image decoding, or GTK widget creation in layout calculation.
- Active widgets must follow viewport plus overscan; geometry may cover the entire collection.
- Total retained photo widgets equal viewport-plus-overscan widgets plus a bounded reusable spare pool. The existing 180 spare-tile cap is not a cap on visible tiles, including on ultrawide displays.
- Recompute geometry for meaningful width/zoom/model changes, never for each scroll tick.
- Every effective geometry change increments a generation; deferred Photo Wall restoration/reflow callbacks must reject a stale generation.
- Switching, zooming, resizing, or scrolling Photo Wall must never initiate original-file reads through layout/anchor code. Existing thumbnail/presentation workers retain their current independent loading policy.
- Preserve Grid's current zoom, resize, and lightbox animations. Initial Photo Wall reflow is immediate with photo anchoring; animated mosaic reflow is outside this feature.
- Do not modify originals, add dependencies, or perform unrelated formatting/refactoring.

## Review focus

1. Older catalogs, missing dimensions, and EXIF portrait NEFs: no double axis swap or implicit repair job; missing dimensions use a stable square fallback, and known legacy dimensions are reported separately.
2. Narrow viewports and extreme aspect ratios: positive finite rectangles, no right-edge overflow, and no oversized final row.
3. Deep scrolling during source replacement or resize: stable photo identity, cancelled stale restoration callbacks, and current viewport priority.
4. Hidden surfaces and rapid Grid/Wall/source changes: no duplicate request producer, retained animation offsets, or lost selection.
5. Empty collections, empty folder ranges, and RAW/JPEG filtering: no phantom photo row or row crossing a boundary; preserve existing header policy.

## File map

Create `src/grid/photo_wall_layout.rs` for pure geometry and unit tests, and `src/grid/photo_wall_tests.rs` for GTK integration tests. Modify the active `src/grid.rs` to declare the geometry module and test module.

Modify `src/grid/view.rs`, `src/grid/sectioned_folder.rs`, `src/grid/virtualization.rs`, `src/grid/selection.rs`, `src/grid/grouping.rs`, and `src/grid/navigation.rs` only where layout state, geometry consumers, or active-surface routing require it. Reuse existing tile code; modify `src/grid/tile.rs` only if rectangular allocation reveals an actual constraint.

Modify `src/infobar.rs` and `src/window/build.rs` for the menu/scroller/zoom wiring. Reuse settings accessors through `src/db/settings.rs` and `src/settings.rs`; avoid a schema migration just for the mode preference. Add the metadata-only ratio helper in `src/photo_object.rs`. Scanner production changes and legacy metadata maintenance are outside this plan; orientation tests may extract the existing scanner calculation without changing its behavior.

## Task 1: Pure justified layout engine

**Interfaces:** `PhotoWallSection { photo_range: Range<usize>, header_height: f64 }`; `PhotoWallItem { photo_index: usize, section: usize, row: usize, x: f64, y: f64, width: f64, height: f64 }`; `PhotoWallRow { item_range: Range<usize>, y: f64, image_height: f64, block_height: f64 }`; `PhotoWallLayout` owns ordered rows/items/section bounds and total height.

Produce `PhotoWallLayout::calculate(aspect_ratios: &[f64], sections: &[PhotoWallSection], viewport_width: f64, target_height: f64, caption_height: f64) -> PhotoWallLayout`, `visible_rows(top: f64, bottom: f64) -> Range<usize>`, and `item(photo_index: usize) -> Option<&PhotoWallItem>`. Constants: gap 4 px, side margins 20 px.

- [ ] Add failing unit tests: `completed_rows_fill_width`, `final_row_uses_target_height`, `rows_do_not_cross_sections`, `empty_sections_follow_header_policy`, `narrow_width_and_panorama_fit`, `invalid_ratios_are_finite`, and `visible_rows_match_intersections`. Assert row right edges match usable width within 1 px; final rows do not exceed target; rectangles are positive and bounded; boundary rows reset independently.
- [ ] Run `cargo test photo_wall_layout`; confirm failure is due to the missing implementation.
- [ ] Implement row accumulation around target height. Use preferred justified heights from `target_height * 0.65` to `target_height * 1.20`. When the next photo crosses the target-width threshold, prefer a candidate ending before or after it within this band, then choose the height closest to target. If adding an item makes a multi-item row too short, move that item to the next row when doing so yields a better valid candidate. Recompute the fitted height after changing membership; do not clamp a completed row's height while leaving its membership unchanged, which would break width justification or aspect ratio. Guarantee at least one item and forward progress. Extreme single-image aspect ratios may require heights below the preferred band to fit; an underfilled final row uses `min(target_height, fitted_height)` and remains left aligned. Accumulate edge coordinates so integer allocations distribute rounding error rather than overflowing. Captions add block height below the common image height.
- [ ] Add `portrait_rows_respect_preferred_height_band` and `extreme_ratios_keep_width_and_aspect_ratio` tests. Assert feasible completed rows use heights within 0.65–1.20 times target, final rows never stretch above target, and exceptional panorama rows preserve aspect ratio and viewport bounds. Treat the band as a row-membership preference, not a reason to distort images.
- [ ] Store rows sorted by Y; use binary searches for viewport rows and a photo-index lookup for anchors. A scroll query must inspect only intersecting rows and their photos.
- [ ] Run `cargo test photo_wall_layout`; require all geometry tests to pass, including generated 14,361-photo and 50,000-photo mixed-ratio collections and viewport queries at the start, middle, and end. These tests need no GTK initialization. Keep this module independent of PhotoObject, GTK, SQLite, paths, thumbnails, EXIF, and selection.

## Task 2: Metadata-only aspect ratios and rectangular tile verification

**Interfaces:** Add `PhotoObject::photo_wall_aspect_ratio(&self) -> f64`, based on positive stored dimensions and separate library rotation. Missing/invalid dimensions return 1.0; do not inspect a file in this helper.

- [ ] Add failing tests for landscape, portrait, square, zero/negative dimensions, 90/180/270 degree library rotations, and already EXIF-normalized dimensions. Verify EXIF normalization and library rotation are separate operations.
- [ ] Verify the active checkout still has independent tile width/height and scanner EXIF normalization before implementation. Add rectangular measurement/allocation coverage for landscape and portrait tiles with captions on/off, and retain existing Grid allocation assertions. If the implementation checkout is older and square-only, minimally extend tile measurement/allocation first; do not replace the widget.
- [ ] Test the existing scanner dimension-normalization calculation for EXIF orientations 1–8, extracting a pure helper only if needed. Assert stored dimensions are post-EXIF and pre-user-rotation; thumbnail orientation is not applied again by the ratio helper.
- [ ] Implement metadata-only aspect ratios. Existing normalized catalog dimensions must never receive a second EXIF swap. Keep original aspect ratio geometry even when an edit recipe crops the cached presentation; display that presentation with `Contain`.
- [ ] Test offline/missing metadata and a normalized EXIF-rotated RAW fixture where available. Assert helper calls do not schedule metadata jobs or perform I/O. Valid dimensions are consumed as stored, invalid/missing dimensions return 1.0, and 90/270 degree PIC rotation inverts the ratio once. Never infer original axes from a cropped presentation texture.
- [ ] Record legacy sensor-oriented dimensions as a separate maintenance follow-up with scanner-owned refresh, persistence/provenance, and retry policies. Do not add that pipeline to Photo Wall. If a legacy catalog or RAW fixture prevents satisfying the original orientation acceptance criterion, report that gap explicitly; do not mark it passed based only on normalized synthetic data.

## Task 3: Extend the existing recycled surface

**Interfaces:** Define public `PhotoLayout { Grid, PhotoWall }` in the active grid module. Store mode in Gallery/shared virtual-surface state. Keep `SectionedFolderView` and its existing root rather than creating another tile/cache subsystem. Its geometry adapter supplies visible items, photo bounds, section/header bounds, total height, and neighbor targets for the current strategy.

Add `PhotoWallState { generation: u64, layout: PhotoWallLayout }` to the surface. Increment generation on effective width, target height, collection/order, section boundaries, dimensions, user rotation, or caption visibility changes. Layout switches invalidate outstanding Wall callbacks as well. Deferred callbacks capture the generation and return without changing widgets/adjustments if it differs; existing source-replacement cancellation remains in force.

- [ ] Add failing GTK tests proving the sectioned surface can render a headerless collection and a two-folder justified collection using the same PhotoObjects and `MultiSelection`.
- [ ] For Folder Photo Wall, feed existing GroupRanges and current header heights to the engine. For ungrouped Photo Wall, feed one headerless range covering the current collection. For Day/Month/History grouping, preserve current sticky heading behavior using visible photo indices; do not invent new folder headers.
- [ ] Extend `geometry_for_current_layout` and `refresh` with a Photo Wall branch that uses the pure row/item geometry. Preserve fixed-column Grid branches. Invalidate by actual viewport width, target height, caption visibility, model generation, metadata generation, and section membership.
- [ ] Add generation tests for each invalidation input, unchanged inputs, and ordinary scrolling. Queue restoration for generation N, replace the collection or resize to N+1, and verify the old callback cannot move the current viewport.
- [ ] Reuse `make_tile`, cache-only binding, selected styling, live maps, tile pool, header pool, and RAW/JPEG header buttons. Allocate each image using its calculated rectangle and force `Contain` only in Photo Wall. Preserve filename captions and favorite/rating/offline indicators.
- [ ] Update rubberband hit testing and vertical keyboard neighbors to use rectangles in Photo Wall. Up/down chooses the nearest horizontal center in the adjacent row; left/right keeps collection order. Reuse existing Ctrl/Shift range selection, activation, context-menu callbacks, and collage selection mode.
- [ ] Run `cargo test photo_wall` for pure tests and GTK tests explicitly with `--ignored --test-threads=1` on a display. Verify portrait allocation, click/Ctrl/Shift selection, Ctrl+A, keyboard navigation, context menu, open, and no row crossing a folder boundary.

## Task 4: Active surface, viewport workers, and navigation

**Interfaces:** Add `Gallery::layout(&self) -> PhotoLayout`, `Gallery::set_layout(self: &Rc<Self>, layout: PhotoLayout)`, and `Gallery::using_virtual_photo_surface(&self) -> bool`. The last returns true for Photo Wall or default sectioned Folder Grid; keep source/group identity separate from layout identity.

Use `ViewAnchor { photo_id: i64, viewport_y_offset: f64 }`, `Gallery::capture_view_anchor(&self) -> Option<ViewAnchor>`, and `Gallery::restore_view_anchor(self: &Rc<Self>, anchor: ViewAnchor)` for operations involving Photo Wall: resize, zoom, Grid ↔ Wall, and sidebar width changes. Layout-specific adapters locate the ID and its destination geometry; one restoration path waits for valid scroll extent, clamps the result, and checks generation. Reuse existing Grid realized-tile/anchor helpers and retain Grid-only pointer zoom behavior and animations. Do not refactor unrelated Grid-only transitions. If an anchor photo disappears on a genuine collection change, use the first remaining selected ID or clamp the previous scroll value; never treat the old index as the same photo.

- [ ] Add failing integration tests for switching at a deep scroll position with a multi-selection; assert exact ListStore, MultiSelection, and PhotoObject identity, model order, source, and selected ID set remain identical and a visible photo stays anchored within rounding tolerance. Include an anchor-photo removal test for genuine collection changes.
- [ ] Extend window scroller selection to use the existing virtual surface for Photo Wall in every source. Keep `GtkGridView` directly inside its scroller in Grid. Attach the virtual root to one scroller only and make active scroll routing explicit.
- [ ] Route `visible_root`, focus, viewport-center lookup, photo restore/reveal, folder targeting, scrollbar date lookup, boundary checks, and lightbox return through the active geometry. Use the shared ViewAnchor capture/restore path before and after changing geometry. Cancel stale reflow/zoom/scroll restorations.
- [ ] Extend current visible-priority scheduling, target-scroll prefetch, ahead/behind prefetch, completion painting, availability refresh, ratings/favorites refresh, and recipe/rotation updates to the active surface. Replace fixed-column scroll-target arithmetic only in Photo Wall. Use the existing display queue and RAM LRU.
- [ ] Ensure progressive replace, append, removal, grouping changes, and RAW/JPEG filtering invalidate Photo Wall geometry without cloning full photo collections or leaving old tiles bound to new indices. Geometry may rebuild O(N) on model batches; ordinary scroll queries must not.
- [ ] Test rapid source/layout changes and progressive replacement while scrolling. Assert requests target current visible IDs; widget counts remain proportional to viewport plus overscan and bounded spare pools after scrolling all the way through 14,361 photos. Count real widgets as well as logical visible items.
- [ ] Repeat widget-count checks on an ultrawide surface at minimum zoom where more than 180 photo tiles are legitimately visible. Verify full viewport coverage and bounded spare retention rather than asserting a global 180-widget maximum.
- [ ] Add scoped test tracing/counters at `source::read` and `source::materialize` to detect original-file reads during mode switching, zoom, resize, and scrolling with already-cached fixtures. Require zero calls attributable to layout/anchor work. Run one case with presentation workers paused/drained to isolate geometry, and one through normal cached worker scheduling. Existing independently necessary worker reads are classified separately; layout must neither call these functions nor trigger a metadata maintenance job.

## Task 5: Bottom view menu, shared zoom, and resize

**Interfaces:** Gallery mode is independent of selected source. Mode preference uses the existing settings infrastructure with key `photo_layout` and values `grid`/`photo_wall`; absent or invalid values default to Grid. Use the current zoom ladder values directly as Photo Wall target heights.

- [ ] Extend the existing bottom-right grid-shaped control to a popover with radio choices `Grid` and `Photo Wall`, plus a separated `Create Collage…` action retaining the existing callback. Update InfoBar fields and their consumers together. Keep its visibility/sensitivity rules appropriate in gallery/editor/lightbox contexts.
- [ ] Wire menu mode changes to `Gallery::set_layout` without invoking source loading, navigation, search reset, or filter reset. Restore the saved mode only when gallery setup is ready.
- [ ] Route slider, +/- keys, Ctrl+wheel, and reset to target-height reflow in Photo Wall. Continue storing the existing shared zoom ladder value; switching layouts must not change it. Use the existing reset level and feedback callback so slider and gallery stay synchronized. Preserve lightbox/editor zoom semantics.
- [ ] On effective content-width changes, recompute rows using the shared ViewAnchor path and generation guards. Skip unchanged geometry inputs; coalesce callbacks within a frame only if allocation observations require it. Do not apply Grid's column animations or strip reflow to Photo Wall and do not retain snapshot offsets across mode changes.
- [ ] Add GTK tests for menu selection, unchanged search/filter/source, repeated Grid → Wall → Grid, slider endpoints, narrow/wide resize, sidebar reveal/hide, and mode restoration. Verify the Collage action still opens the existing editor.

## Task 6: Verification and acceptance evidence

- [ ] Before implementation, record baseline results for `cargo fmt --check`, `cargo check`, and `cargo test`; existing handover documents mention formatting debt, so verify the current checkout rather than assuming it passes.
- [ ] After the focused changes, run `cargo fmt --check`, `cargo check`, and `cargo test`. Fix failures introduced by this work. If repository-wide formatting is already failing, report baseline versus changed-file formatting; do not format unrelated files.
- [ ] Run display-dependent Photo Wall tests explicitly and existing Folder/Grid tests relevant to selection, zoom anchors, progressive replacement, and header stability. Record unavailable display/fixture checks instead of claiming they passed.
- [ ] Exercise All Photos, Favourites, Recently Added, History, albums, individual/parent folders, and supported Network Shares. Use mixed orientations, missing metadata, offline cached photos, RAW/JPEG filtering, edited photos, and a 10,000+ collection.
- [ ] Verify wall width, 4 px gaps, final-row sizing, folder boundaries, headers/counts, zoom, resize, selection, opening and return, context menu, and cached-thumbnail reuse. Confirm the old Grid remains visually and behaviorally unchanged.
- [ ] Inspect widget counts and layout/request tracing during deep scrolling: no widget-per-photo growth, no filesystem activity in geometry, and no full layout recalculation on ordinary scroll events. Record timings without inventing a hardware-independent performance threshold.
- [ ] Record the source-read regression tests, stale-generation rejection, ultrawide viewport coverage, and exact model/selection identity checks as hard acceptance evidence. Separate EXIF-normalized catalog validation from outstanding legacy metadata maintenance.
- [ ] Run `git diff --check`, review only intended active files, and map evidence to every acceptance checkbox in `TODO.photowall.md`. Leave changes reviewable; do not push or merge as part of this plan.

## Expected implementation order

Geometry → rectangular tile verification and metadata-only ratio helper → basic recycled Wall rendering → viewport/navigation integration and generation-safe anchors → menu/zoom/resize → regression and visual verification. Legacy metadata repair is a separate scanner maintenance effort, never a stage triggered by entering Photo Wall. Execute inline because these tasks share private Gallery state and tightly coupled geometry interfaces.
