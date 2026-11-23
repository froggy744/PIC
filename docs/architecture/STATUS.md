# Architecture review status

The single source of truth for the deepening candidates from the 2026-11-22 architecture review. The review itself, with its before/after diagrams and evidence, is [`architecture-review-2026-11-22.html`](architecture-review-2026-11-22.html) (taken at `main` @ `5356ef20`). Open it in a browser. Its Status box is history; update this file instead.

Terms (Thumbnail presentation, Sidebar presentation, Properties presentation, ...) are defined in [`GLOSSARY.md`](../../GLOSSARY.md). Code paths are in [`AUDIT-INDEX.md`](../../AUDIT-INDEX.md).

## When a candidate moves

- **Started:** set the state to `in progress` and give the branch name.
- **Merged:** set the state to `merged` and give the merge commit (or the commit range if it went straight to `main`), the owning modules, and any follow-up work that is still open.
- **Owner rule added:** a merged candidate whose adapter must be the only writer of some widgets gets a rule in `scripts/check-presentation-owners.sh` in the same change.

## Candidates

| # | Candidate | Strength | State | Merged as | Owning modules |
|---|---|---|---|---|---|
| 1 | Deepen gallery thumbnail presentation | Strong | merged | commits `0df3b918`..`62dff881` on `main` (no merge commit; branch `refactor/thumbnail-presentation`) | `src/grid/presentation.rs` |
| 2 | Sidebar presentation state machine | Strong | merged | merge `03a2fcb3` (`refactor/sidebar-presentation`) | `src/window/sidebar_presentation.rs` (pure), `src/window/sidebar_widgets.rs` (GTK adapter) |
| 3 | Properties inspector as one owner | Strong | merged | merge `2da69bae` (`refactor/properties-presentation`) | `src/window/properties_presentation.rs` (pure), `src/window/properties_widgets.rs` (GTK adapter) |
| 4 | InfoBar decides what it shows | Strong | merged | merge `9756c44f` (`refactor/infobar-presentation`) | `src/infobar_presentation.rs` (pure), `src/infobar.rs` (GTK adapter) |
| 5 | One gallery scroll-motion module | Worth exploring | open | none | `src/window/navigation.rs`, `src/window/build.rs` (scrub, settle, pump), `src/smooth_scroll.rs` |
| 6 | Zoom slider router | Worth exploring | open | none | `src/window/build.rs` (zoom slider handlers), `src/lightbox/impl.rs` |
| 7 | Editor preview plan | Worth exploring | open | none | `src/edit/live_gpu.rs`, `src/edit/editor.rs` |
| 8 | Screensaver style descriptor | Speculative | open | none | `src/screensaver.rs`, `src/screensaver/gpu.rs` |

## Outstanding work

### 1 · Thumbnail presentation
None recorded.

### 2 · Sidebar presentation
- `src/window/layout.rs`: the `compact_dock.connect_visible_notify` guard still hides the compact dock directly while Immersive Mode is active. It predates `SidebarWidgets`, and is the one known exception allowed in `scripts/check-presentation-owners.sh` (`dock.set_visible(false);`). Moving it into the adapter removes that allowlist entry.
- `SidebarCells` (`src/window/sidebar_widgets.rs`) still shares mirror cells with "readers not yet moved onto the adapter".

### 3 · Properties presentation
None recorded. `src/window/immersive.rs` still sets the Properties split's pin and collapse while immersive. That is layout, not visibility, so it is outside what `PropertiesWidgets` owns.

### 4 · InfoBar decides what it shows
**Decisions (2026-11-23, by the owner):**
1. **Notify is in scope.** The InfoBar watches `notify` on the photo it shows (rating, favourite, metadata fields; it already watches `thumbnail-available`). This lets the manual refresh calls go, as a later stage after the mode choice has moved.
   **Revised 2026-11-23 (stage 4), by the owner:** no notify handler. Checking showed it could replace only 5 of the 10 refresh calls:
   - `PhotoObject::set_from_photo` (scanner updates) writes fields without emitting notifications.
   - Editor save works on a new object (`PhotoObject::from_photo`).
   - Rename and context-menu favourite change the context-menu object, which may not be the shown one.

   Instead, `InfoBar::photo_updated` stays the one explicit way to report a change, and it rewrites only what changed. Don't re-propose notify without first making `set_from_photo` notify, which is a performance question on large libraries.
2. **`selected_photo` is left alone.** The `Rc<RefCell<Option<PhotoObject>>>` in `window/build.rs`, kept in step by hand, is not changed. Record it here as follow-up work when the candidate merges.
3. **Branch `refactor/infobar-presentation`** from `main`, merged with a merge commit like candidates 2 and 3.

**The problem** (verified at `8a5bddeb`). The InfoBar shows one of three states: empty, one photo's metadata, or the multi-selection strip. Callers choose the state themselves through three methods that must be called in the right order:
- `set_selected_photos(count, photos)`: 0 also clears the photo; 1 only switches to the metadata page and leaves the old photo showing; 2 or more shows the strip. Collage is checked inside.
- `show_photo_metadata()`
- `set_photo(Option<&PhotoObject>)`

Six places each choose the state, with different rules:

| Place | What it does |
|---|---|
| `window/build.rs`, the `grid::Gallery::new` focus callback | Guards on lightbox and date-browser overview; on `None`, calls `show_photo_metadata` first. |
| `window/build.rs`, `connect_selection_preview_changed` | Same guard, then `set_selected_photos`. |
| `window/build.rs`, `lightbox.root.connect_visible_notify` (the second one) | Re-reads the gallery selection when the lightbox hides. |
| `window/build.rs`, `sync_date_browser_photo_info` | Its own `match count`. |
| `window/build.rs`, the date-browser overview callback | `show_photo_metadata` then `set_photo(None)`. |
| `window/layout.rs`, `main_stack.connect_visible_child_notify` | Re-reads the selection on the Photos page; no lightbox guard. |

About 12 refresh calls of `set_photo(Some(&photo))` follow a change to the photo:
- `window/build.rs`: editor `saved`, favourite, rating buttons, rotate, rating shortcut, two scanner `PhotosUpdated`/`PhotosIndexed` paths.
- `window/photo_actions.rs`: bulk recipe finish, favourite, rename.

`set_photo(None)` resets stay in: window build, library switch, and trash in `photo_actions.rs`.

**Planned stages:**
1. A pure model with no GTK and table tests.
2. The InfoBar applies it through events (surface, selection, focus, lightbox photo), writing only what changed. The old `set_selected_photos`, `show_photo_metadata`, `set_photo` and `set_collage_active` stay as thin wrappers over the same write helpers, so callers are unchanged.
3. Move the six callers, the refresh calls and the InfoBar's own tests onto the events, then delete the old wrappers and the temporary `dead_code` allowances.
4. `photo_updated` rewrites only what changed: filename, metrics, rating and favourite are compared through `PhotoSummary`, and the preview is redrawn only when its thumbnail, rotation, edit recipe or object changes (see decision 1).
5. Docs: GLOSSARY term, an InfoBar entry in AUDIT-INDEX (it has none yet), this file, and an owner rule in `scripts/check-presentation-owners.sh`.

**Follow-up after merge:**
- `selected_photo` in `window/build.rs` is still kept in step by hand beside the InfoBar events (decision 2).
- The gallery focus callback in `window/build.rs` keeps its own lightbox/overview guard, because it also guards the Properties panel and `selected_photo`. The InfoBar no longer needs it.
- `infobar::responsive_tests::compact_toolbar_keeps_actions_usable_and_restores_wide_layout` failed at "Rotate should leave the action menu open" (found 2026-11-23, before candidate 4). Fixed 2026-11-23: Rotate never closed the menu; under Xvfb GTK's autohide closed a menu opened right after a resize, with or without the click. The test now makes sure the menu is open before checking Rotate.

**Tests to keep green** (Xvfb 1920×1080, Wayland unset):
- `multiselection_centers_existing_actions_and_restores_toolbar`
- `rating_indicator_updates_when_lightbox_photo_changes`
- `selected_preview_appears_when_grid_thumbnail_finishes`
- `selection_count_and_clear_stay_outside_the_thumbnail_scroller`

Add one display test that rates a photo with no explicit refresh.

### Next: 7 · Editor preview plan (recommended 2026-11-23)
Re-checked at `2745dbd6`; 5-8 still match the code. 7 goes first because CPU/GPU parity is an engineering priority and it has a verified disagreement:
- `cpu_fallback_reason` (`src/edit/editor.rs`) names clarity, grain, filter and B&W/sepia as CPU-fallback reasons.
- The real rule, `gpu_tone_supported` (`src/edit/live_gpu.rs`), allows all four on the GPU. It only forces CPU for auto brightness or enhance, partial or filtered or cropped auto colour/contrast, smooth and vignette.

**First step:** find every caller of `cpu_fallback_reason`, `gpu_tone_supported`, `gpu_tone_supported_for_view` and `gpu_base_recipe`. Work out whether the wrong reason only mislabels traces, or also changes which renderer the preview uses. Then design the one planning module: the renderer choice, base recipe, rotation and reason together, with pure recipe-table tests. CI: `editor-quality.yml`.

### 5, 6, 8
Not started. The problem statements are in the review report.
- 5: `smooth_scroll.rs` still has no callers, and both `install_*_smooth_gallery_scroll` functions remain; the galleries already share `WheelGlide`.
- 6: `window/build.rs` still has 43 `is_visible()` checks.
