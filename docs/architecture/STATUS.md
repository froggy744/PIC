# Architecture review status

The single source of truth for the deepening candidates from the architecture reviews. Open the reports in a browser; their status boxes are history, so update this file instead.

- Round one, candidates 1-8: [`architecture-review-2026-11-22.html`](architecture-review-2026-11-22.html), taken at `main` @ `5356ef20`.
- Round two, re-checks 5-8 and adds 9-12: [`architecture-review-2026-11-23.html`](architecture-review-2026-11-23.html), taken at `main` @ `07cabed4`. The report labels the new candidates N1-N4; they are 9-12 here.

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
| 9 | Photo mark commands (favourite, rating) | Strong | open | none | `src/window/build.rs` (InfoBar favourite, rating buttons, digit keys), `src/window/photo_actions.rs` (context-menu favourite) |
| 10 | Current photo (`selected_photo`) | Strong | open | none | `src/window/build.rs`, `src/window/photo_actions.rs`, `src/window/layout.rs` |
| 11 | Window session state | Worth exploring | open | none | `src/window/build.rs` (restore, library switch, close), `src/window/layout.rs`, `src/window/sidebar_widgets.rs` |
| 12 | Front surface (widens 6) | Worth exploring | open | none | `src/window/build.rs`, `src/window/layout.rs`, `src/window/photo_actions.rs` |

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

### Next: 9 · Photo mark commands (recommended 2026-11-23, round two)
Recommended over 7 because it is the only open candidate with behaviour users can hit today. Verified at `1e627b0a` (`build.rs` unchanged since):
- Favourite toggle: the InfoBar uses `ids.len() > 1 || !photo.favorite()` (`build.rs:4173`); the context menu uses `!favorite_photo.favorite()` (`photo_actions.rs:578`).
- Rating: the button toggles back to 0 on the same rating (`build.rs:4229`); the digit key does not (`:5512`).
- The InfoBar favourite re-runs `refresh_grid` under active filters (`:4192`); the context menu doesn't, and doesn't refresh Properties.
- Sidebar counts: the favourite paths use `refresh_library_counts`; both rating paths use `update_library_counts`.
- The target rule `lightbox visible ? [photo] : gallery.selected_photo_ids(Some(id))` is written 3 times (`:4168, :4224, :5507`). Export (`:5258`) and Print (`:5331`) use the selection without the lightbox guard; not confirmed whether that is intended.

**First step:** decide with the owner which toggle and refresh rules are correct, then a pure module that returns the target set, the new values and a refresh plan, with table tests; one GTK adapter applies the plan.

### 7 · Editor preview plan
Re-checked 2026-11-23 (round two): the wrong reason only mislabels the trace; it never changes the renderer.
- `cpu_fallback_reason` (`src/edit/editor.rs:166`) is used only in the `PIC_EDIT_RENDERER renderer=cpu_fallback` trace (`:1415`, gated on `PICASA_TRACE`), plus a test at `:3018` that locks in the wrong `"clarity"` reason.
- It names clarity, grain, filter and B&W/sepia, which the GPU allows, and never names the real reasons: partial `filter_amount`, crop or straighten with auto, a view that is not 1:1, rotation, or `gpu_failed`. The trace fires inside `if let Some(gpu_canvas)`, so a `gpu_failed` fallback is reported as a recipe reason.
- The renderer choice is `gpu_tone_supported_for_view` (`live_gpu.rs:1600-1632`), called at `editor.rs:1270` (renderer, `decode_rotation`, base recipe via `gpu_base_recipe` at `:1278`) and rebuilt by hand at `:1529` (`has_source_for`). Those two copies can drift.
- `gpu_tone_supported` alone decides the filter tiles (`editor.rs:889`), deliberately without the view and rotation check.

The planning module is still the design: renderer, base recipe, rotation and reason together, with pure recipe-table tests. CI: `editor-quality.yml`.

### 5, 6, 8, 10, 11, 12
Not started. The problem statements are in the review reports. Re-checked 2026-11-23:
- 5: `smooth_scroll.rs` (274 lines) still has no callers; both `install_*_smooth_gallery_scroll` functions remain; the galleries already share `WheelGlide`. Scrub and settle are written twice inline in `build.rs` with different constants: All Photos scrubs at 1.75 pages (≥ 900 px) and settles after 90 ms; Folder scrubs at 1.60 pages and settles after 110 ms. None of it is tested.
- 6: `window/build.rs` still has 43 `is_visible()` checks. The zoom slider has 10 writers of its value or range (2 unguarded: `build.rs:580`, `:7233`), the 0..7 range setup is copied 3 times (`build.rs:4045`, `:4667`, `layout.rs:2465`), and `lightbox_scale_from_slider` / `lightbox_slider_from_scale` are untested nested functions in `build()`. Best done after 12, as its consumer.
- 8: still Speculative: 12 styles, about 63 style matches across `screensaver.rs` and `screensaver/gpu*.rs`.
- 10: `selected_photo` is written in 9 places (`build.rs:248, :632, :2671, :2777, :4074, :7189`; `photo_actions.rs:272, :1369, :1453`) with no precedence rule. None of the 6 `lightbox.root.connect_visible_notify` handlers writes it, so it may go stale when the lightbox closes; needs checking. This is the candidate 4 follow-up (decision 2) and pairs with 9.
- 11: defaults differ: with no saved last view, startup lands on Recently Added (`window.rs:368`) but a library switch lands on Library (`build.rs:7130`). The sidebar default `0.22` is copied at `layout.rs:1255` and `sidebar_widgets.rs:400`, and only `layout.rs` clamps it. Scroll and anchor are saved only on close, not before a library switch.
- 12: "which surface is in front" is re-derived at about 35 sites (28 `lightbox.root.is_visible()` and 7 `visible_child_name() == Some("edit")` in `build.rs`, plus `layout.rs` and `photo_actions.rs`). One owner would serve 6, 9, 10 and the InfoBar's cover rule.
