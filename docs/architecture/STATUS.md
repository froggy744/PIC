# Architecture review status

The single source of truth for the deepening candidates from the architecture reviews. Open the reports in a browser; their status boxes are history, so update this file instead.

- Round one, candidates 1-8: [`architecture-review-2026-11-22.html`](architecture-review-2026-11-22.html), taken at `main` @ `5356ef20`.
- Round two, re-checks 5-8 and adds 9-12: [`architecture-review-2026-11-23.html`](architecture-review-2026-11-23.html), taken at `main` @ `07cabed4`. The report labels the new candidates N1-N4; they are 9-12 here.

Terms (Thumbnail presentation, Sidebar presentation, Properties presentation, ...) are defined in [`GLOSSARY.md`](../../GLOSSARY.md). Code paths are in [`AUDIT-INDEX.md`](../../AUDIT-INDEX.md).

## Starting a candidate

Load these skills (plugin `mattpocock-skills`) with the Skill tool before designing or changing code:

1. `mattpocock-skills:codebase-design`: the vocabulary (module, interface, depth, seam, adapter, leverage, locality) and the deletion test. Use its terms in code comments, commits and this file.
2. `mattpocock-skills:grilling`: walk the candidate's decision tree with the owner (constraints, the shape of the deepened module, what sits behind the seam, which tests survive) before writing code. Record the decisions under the candidate's section here, as candidate 4 did.
3. `mattpocock-skills:domain-modeling`: keep [`GLOSSARY.md`](../../GLOSSARY.md) current. Add a term for the deepened module when it names a new concept. If the owner rejects a candidate for a lasting reason, offer an ADR.
4. `mattpocock-skills:tdd`: build the pure module test-first (table tests), then the GTK adapter.

For a new review round, run `mattpocock-skills:improve-codebase-architecture` and record its candidates here.

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
| 7 | Editor preview plan | Worth exploring | open, lower priority (diagnostic only) | none | `src/edit/live_gpu.rs`, `src/edit/editor.rs` |
| 8 | Screensaver style descriptor | Speculative | open | none | `src/screensaver.rs`, `src/screensaver/gpu.rs` |
| 9 | Photo mark commands (favourite, rating) | Strong | merged | merge `9f056fe3` (`refactor/photo-mark-commands`) | `src/window/photo_marks.rs` (pure), `src/window/photo_mark_commands.rs` (GTK adapter) |
| 10 | Current photo (`selected_photo`) | Strong | next (investigation not started) | none | `src/window/build.rs`, `src/window/photo_actions.rs`, `src/window/layout.rs` |
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

### 9 · Photo mark commands (merged 2026-11-23)
Recommended over 7 because it is the only open candidate with behaviour users can hit today. Verified at `1e627b0a` (`build.rs` unchanged since):
- Favourite toggle: the InfoBar uses `ids.len() > 1 || !photo.favorite()` (`build.rs:4173`); the context menu uses `!favorite_photo.favorite()` (`photo_actions.rs:578`).
- Rating: the button toggles back to 0 on the same rating (`build.rs:4229`); the digit key does not (`:5512`).
- The InfoBar favourite re-runs `refresh_grid` under active filters (`:4192`); the context menu doesn't, and doesn't refresh Properties.
- Sidebar counts: the favourite paths use `refresh_library_counts`; both rating paths use `update_library_counts`.
- The target rule `lightbox visible ? [photo] : gallery.selected_photo_ids(Some(id))` is written 3 times (`:4168, :4224, :5507`). Export (`:5258`) and Print (`:5331`) use the selection without the lightbox guard; not confirmed whether that is intended.

**First step:** decide with the owner which toggle and refresh rules are correct, then a pure module that returns the target set, the new values and a refresh plan, with table tests; one GTK adapter applies the plan.

#### Investigation (2026-11-23, at `ef47462a`, no code changed)
Five entry points write marks. There is no favourite keyboard shortcut, no rating item in the photo menu, no mark control inside the lightbox itself (it uses the InfoBar and photo menu), and no undo for marks (only edit recipes have history).

| | InfoBar heart (`build.rs:4164`) | Photo menu favourite (`photo_actions.rs:572`) | InfoBar stars 1-5 (`build.rs:4219`) | Digit keys 0-5 (`build.rs:5500`) | Sidebar folder favourite (`layout.rs:568`) |
|---|---|---|---|---|---|
| Anchor | `selected_photo` | the right-clicked object | `selected_photo` | `selected_photo` | the folder |
| Targets | lightbox shown ? `[anchor]` : `gallery.selected_photo_ids(Some(anchor))` | `gallery.selected_photo_ids(Some(clicked))`, **no lightbox guard** | as heart | as heart | folder subtree, non-trashed |
| New value | `ids.len() > 1 \|\| !anchor.favorite()` (multi always adds) | `!clicked.favorite()` (multi follows the clicked photo) | single and same rating → 0, else N | always N (0 clears; 6-9 swallowed) | explicit add/remove |
| Write | per-id `db::set_favorite`, no transaction, ignores `trashed` | same | `set_rating_for_photos`, one transaction, `trashed = 0` | same | one statement |
| On failure | `eprintln`, return; earlier ids already written, UI not updated | error dialog, return; same partial write | `eprintln`, nothing written | `eprintln` | error dialog |
| Objects | anchor + `gallery.update_favorites` | clicked + `update_favorites`; swaps `selected_photo` to the clicked object when ids match | anchor + `update_ratings` | same | none |
| InfoBar | `photo_updated` (not on the removal branch) | `photo_updated` only if clicked = `selected_photo` | `photo_updated` | `photo_updated` | none |
| Properties | `show_photo(Lightbox, …)` even from the gallery | none | `show_photo(Lightbox, …)` | `show_photo(Lightbox, …)` | none |
| Leaves view | Favourites + unfavourite → `remove_photos` + `lightbox.remove_photo(anchor)`; any metadata filter → `refresh_grid` | Favourites → `remove_photos` + `lightbox.remove_photo` per id; metadata filters ignored | rating filter, Ratings/Rating(n) sidebar or rating sort → `refresh_grid`; metadata `Rating` filter ignored; lightbox untouched | same | always `refresh_grid` |
| Sidebar counts | `refresh_library_counts` (rebuilds rows) | same | `update_library_counts` (labels only) | same | `availability_refresh` |

**Inconsistencies and what they cost:**
1. Multi-selection favourite: the heart can only add (no way to unfavourite a selection from the InfoBar); the menu follows the clicked photo, so the same selection gives opposite results from the two controls.
2. Photo menu in the lightbox targets the gallery selection, so it can change photos that are not on screen while the heart in the same view changes one (to confirm with a display test: depends on whether the gallery selection still contains the shown photo).
3. Stars toggle back to 0 on the same rating, digits don't; stars toggle only for a single photo. Reasonable as two intents, but undocumented.
4. Favourite failures half-apply: earlier ids are in the database, UI shows the old state, and the InfoBar path tells nobody. Favourite can also mark trashed photos; rating can't.
5. Membership: metadata filters (`db::filters::Filters`, which include `Favorite` and `Rating` fields) trigger a rebuild only on the heart path; a metadata `Rating` filter never does, so a re-rated photo stays visible. Ratings never update the lightbox collection; Favourites removal does.
6. Properties: the docked gallery panel is never refreshed after a mark (all paths pass `PropertiesSurface::Lightbox`); the menu path refreshes neither panel.
7. Collage selection mode: `Gallery::selected_photo_ids` returns the collage ids with no fallback, so an empty collage selection writes nothing while the anchor object (and InfoBar) shows the new value. The InfoBar keeps photo actions enabled in Collage (`photo_actions_enabled`). Not reproduced yet.
8. Two sidebar refreshers for the same count change; `refresh_library_counts` rebuilds rows for no structural reason (row visibility does not depend on the counts).
9. The target rule is written 3 times in `build.rs` and a 4th, different way in the menu.

**Proposed module boundary (Photo mark commands).** Pure module `src/window/photo_marks.rs` (`include!`d like its neighbours, or a `mod` if the owner prefers), no GTK, no state of its own:
```rust
pub(super) enum MarkIntent { ToggleFavorite, ToggleRating(u8 /*1..=5*/), SetRating(u8 /*0..=5*/) }
pub(super) enum Front { Gallery, Lightbox }
pub(super) struct MarkRequest<'a> {
    intent: MarkIntent,
    front: Front,
    anchor: PhotoMarks,                 // id, favorite, rating as read from the anchor object
    selection: &'a [PhotoMarks],        // gallery selection with fallback, read by the adapter
    view: ViewMembership,               // favourites view, rating filter/sidebar/sort, metadata Favorite/Rating fields
}
pub(super) struct MarkPlan { ids: Vec<i64>, change: MarkChange /* Favorite(bool) | Rating(i32) */, membership: Membership /* Keep | Remove(ids) | Rebuild */ }
pub(super) fn plan(request: MarkRequest) -> Option<MarkPlan>;   // None: nothing to do (empty target)
```
One GTK adapter, `PhotoMarkCommands` (window-level `Rc`, cloned into the five handlers): `run(&self, intent, anchor: &PhotoObject) -> Result<()>` reads the live front/selection/view, calls `plan`, writes once (new `db::set_favorite_for_photos`, one transaction, `trashed = 0`), then on success only: updates gallery objects and the anchor, `InfoBar::photo_updated`, the Properties panel on screen, `sidebar::update_library_counts`, and applies `membership` (gallery and lightbox together). On error nothing in the UI changes and one error path reports it. Entry points shrink to `commands.run(MarkIntent::…, &photo)`. The folder favourite keeps its own target (a folder) but can reuse the adapter's post-change step. No cached marks: values are always read from the `PhotoObject`s and the database.

**Decisions needed before implementation:**
1. ~~Multi-selection favourite rule.~~ **Decided 2026-11-23 by the owner:** if every target is already a favourite, remove them all; otherwise add them all. The heart and the photo menu both use this rule. A single photo simply toggles.
2. ~~Stars vs digits.~~ **Decided 2026-11-23 by the owner:** keep two intents. Stars toggle (same rating → 0); digit keys always set (0 clears). For a multi-selection, the stars clear to 0 when every target already has that rating, otherwise set it (same rule as decision 1).
3. ~~Photo menu in the lightbox.~~ **Decided 2026-11-23 by the owner:** in the lightbox every mark control, the photo menu included, changes only the photo on screen.
4. ~~Leaving the view.~~ **Decided 2026-11-23 by the owner:** one membership rule for Favourites, rating filters/sidebar/sort and metadata `Favorite`/`Rating` filters. A photo that no longer matches leaves the gallery, and the lightbox drops it and moves to the next photo (today's Favourites behaviour, now for every view rule).
5. ~~Failure.~~ **Decided 2026-11-23 by the owner:** one transaction for favourites too, so nothing half-applies, and a self-dismissing message on every entry point, no dialog. Reuse `OperationProgress::finish` (`window/operation_progress.rs:408`, hides after `FINISH_HIDE_AFTER`), which bulk edit failures already use; no new widget.
6. ~~Collage selection mode.~~ **Decided 2026-11-23 by the owner:** marks apply to the photos ticked for the collage; with none ticked, `plan` returns nothing and no object or InfoBar changes (fixes the UI-only change).
7. ~~Scope.~~ **Decided 2026-11-23 by the owner:** the sidebar folder favourite stays out except for reusing the adapter's post-change refresh; undo for marks stays out.

All seven decisions are settled. Started 2026-11-23 on branch `refactor/photo-mark-commands`.

**Stages:**
1. Pure `plan` in `src/window/photo_marks.rs` with table tests (targets per front, favourite rule, star/digit rule, membership). Done: `d7e3d0ae`. Membership is precise: the Online/Offline/Corrupt toolbar filters and unrelated metadata filters no longer force a rebuild (today any non-All rating filter, and any metadata filter on the heart path, does).
2. `db::set_favorite_for_photos`: one transaction, skips trashed photos. Done: `5c8be17f`; test `favourites_for_photos_skip_trashed_and_write_all_or_nothing` forces a failure on the last photo with a trigger.
3. GTK adapter `src/window/photo_mark_commands.rs` (`run`, `preview`, `reload_shown_photo`) over `PhotoActionContext`, which gained `rating_filter`. The heart, stars, digit keys and photo menu call `run`; the menu's label comes from `preview`; the folder favourite calls `reload_shown_photo`. Gallery gained `selected_photos`, `collage_selection_mode` and `contains_photo`. A failed write shows a notice through `OperationProgressUi::finish` unless an operation is running (then it is only logged). Display test `marks_change_a_whole_selection_or_nothing`. Done: `1a8a7771`.
4. Docs: GLOSSARY term **Photo marks**, AUDIT-INDEX rows and a Quick-Edit row, this file, and owner rule `photo-marks` in `scripts/check-presentation-owners.sh` (library and gallery mark writes only in `photo_mark_commands.rs`; the four `library.rs` rating-filter test fixtures are allowed). Done: `2a07053c`.

**Follow-up after merge:**
- Not covered by a test, checked by hand by the owner on 2026-11-23 and working: the lightbox moving on when a rating change takes the shown photo out of a rating or metadata view, and the gallery Properties panel refreshing.
- A failed write while an export or scan owns the progress bar is only logged (`eprintln`), so the running job's bar is not ended.
- Other photo-menu actions (album, remove, export) still target the gallery selection inside the lightbox; decision 3 covered marks only.
- `selected_photo` (candidate 10) is still kept in step by hand; the adapter reads it as the shown photo.

Changes users would notice: the heart can unfavourite a selection; the lightbox photo menu changes one photo; a re-rated photo disappears under a metadata `Rating` filter; a photo menu favourite under a metadata filter rebuilds the grid; the gallery Properties panel updates; failed writes show an error and leave nothing half-written; clicking a selection's shared star rating clears it; the lightbox moves on when a rating change takes the shown photo out of a rating view; in Collage, marks follow the ticked photos.

**Test strategy and scope:** table tests for `plan` (intent × single/multi × mixed values × front × each view rule, roughly 25 rows); a `db` test for `set_favorite_for_photos` (transaction, trashed skipped). Display tests (Xvfb): keep `album_cover_action_is_only_offered_inside_an_album` (it covers the menu favourite toggle) and `rating_indicator_updates_when_lightbox_photo_changes`; add one adapter test that rates and favourites a two-photo selection and checks objects, InfoBar and the Favourites removal. Scope estimate: pure module ~250 lines with tests, adapter ~200, net removal of ~180 lines from `build.rs` and ~50 from `photo_actions.rs`; 4 stages (pure plan, db batch write, adapter and move the 5 entry points, docs and owner rule). Pairs with 10 (`selected_photo`) and 12 (front surface) but does not need them: the adapter reads the front with the existing `lightbox.root.is_visible()` check in one place.

### Next: 10 · Current photo (chosen by the owner 2026-11-23)
Chosen over 12 because candidate 9's controls act on `selected_photo`, so a stale value changes the wrong photo; it is also candidate 4's open follow-up (decision 2). Start the same way as 9: investigation and design only, then the owner's decisions one at a time, then a branch.

**First step:** trace every writer and reader of `selected_photo` (the 9 writers are listed under "5, 6, 8, 10, 11, 12" below), confirm whether it goes stale when the lightbox closes, and propose one owner with a precedence rule. 12 (front surface) is the natural follow-on.

### 7 · Editor preview plan
**Lower priority (2026-11-23):** its disagreement is diagnostic only (a mislabelled trace); 9 goes first.

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
