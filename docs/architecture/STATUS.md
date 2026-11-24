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
| 5 | One gallery scroll-motion module | Worth exploring | awaiting merge (`refactor/gallery-scroll-motion`) | none yet | `src/window/scroll_motion.rs` (pure), `src/window/build.rs` (GTK adapter); wheel glide stays in `src/window/navigation.rs` |
| 6 | Zoom slider router | Worth exploring | merged | merge `9c9c64ff` (`refactor/zoom-slider-router`) | `src/window/zoom_slider.rs` (pure), `src/window/zoom_slider_widgets.rs` (GTK adapter, a front-change listener) |
| 7 | Editor preview plan | Worth exploring | open, lower priority (diagnostic only) | none | `src/edit/live_gpu.rs`, `src/edit/editor.rs` |
| 8 | Screensaver style descriptor | Speculative | open | none | `src/screensaver.rs`, `src/screensaver/gpu.rs` |
| 9 | Photo mark commands (favourite, rating) | Strong | merged | merge `9f056fe3` (`refactor/photo-mark-commands`) | `src/window/photo_marks.rs` (pure), `src/window/photo_mark_commands.rs` (GTK adapter) |
| 10 | Current photo (`selected_photo`) | Strong | merged | merge `49a93dd4` (`refactor/current-photo`) | `src/infobar_presentation.rs` (precedence, pure), `src/infobar.rs` (`InfoBar::current_photo`) |
| 11 | Window session state | Worth exploring | merged | merge `4f47d674` (`refactor/window-session-state`) | `src/window/session_state.rs` (pure: keys, defaults, clamps, load, record; `SessionStore` seam) |
| 12 | Front surface (widens 6) | Worth exploring | merged | merge `16e816e3` (`refactor/front-surface`) | `src/window/front_surface.rs` (pure precedence + `FrontSurface` reader) |
| 13 | Front-surface change event (follow-up to 12, before 6) | Worth exploring | merged | merge `888408e0` (`refactor/front-surface-events`) | `src/window/front_surface.rs` (state, diff, dispatcher), `src/window/front_mirrors.rs` (listeners) |

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
- `selected_photo` in `window/build.rs` is still kept in step by hand beside the InfoBar events (decision 2). Resolved by candidate 10: the InfoBar owns the current photo.
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
- `selected_photo` (candidate 10) is still kept in step by hand; the adapter reads it as the shown photo. Resolved by candidate 10: the adapter reads `InfoBar::current_photo`.

Changes users would notice: the heart can unfavourite a selection; the lightbox photo menu changes one photo; a re-rated photo disappears under a metadata `Rating` filter; a photo menu favourite under a metadata filter rebuilds the grid; the gallery Properties panel updates; failed writes show an error and leave nothing half-written; clicking a selection's shared star rating clears it; the lightbox moves on when a rating change takes the shown photo out of a rating view; in Collage, marks follow the ticked photos.

**Test strategy and scope:** table tests for `plan` (intent × single/multi × mixed values × front × each view rule, roughly 25 rows); a `db` test for `set_favorite_for_photos` (transaction, trashed skipped). Display tests (Xvfb): keep `album_cover_action_is_only_offered_inside_an_album` (it covers the menu favourite toggle) and `rating_indicator_updates_when_lightbox_photo_changes`; add one adapter test that rates and favourites a two-photo selection and checks objects, InfoBar and the Favourites removal. Scope estimate: pure module ~250 lines with tests, adapter ~200, net removal of ~180 lines from `build.rs` and ~50 from `photo_actions.rs`; 4 stages (pure plan, db batch write, adapter and move the 5 entry points, docs and owner rule). Pairs with 10 (`selected_photo`) and 12 (front surface) but does not need them: the adapter reads the front with the existing `lightbox.root.is_visible()` check in one place.

### 10 · Current photo (merged 2026-11-23)
Done by an agent while the owner was away ("start candidate 10 and continue without my input"). The decisions below were taken by the agent, choosing the option closest to today's behaviour. **The owner accepted them on 2026-11-23** after the manual checks under Verification. Branch `refactor/current-photo`.

#### Investigation (at `51923e06`)
`selected_photo: Rc<RefCell<Option<PhotoObject>>>` (built in `window/build.rs`, shared through `PhotoActionContext`) had 10 writers and 14 readers. Every writer sat beside an InfoBar event that already decided the same thing in `InfoBarPresentation` (the shown photo, with a tested precedence):

| Writer | Paired InfoBar event | Same result? |
|---|---|---|
| lightbox photo changed | `show_lightbox_photo` | yes |
| gallery focus (guarded on lightbox/overview) | `show_gallery_focus` (same guard inside) | yes |
| date browser hand-back: first photo of any selection | `show_date_browser_selection`: only a single photo became shown | **no** for 2+ photos |
| date overview shown: `None` | `set_date_overview_shown(true)` | yes |
| editor saved, rename | `photo_updated` | yes |
| bulk recipe (same object, re-stored) | `photo_updated` at finish | yes |
| trash, library switch: `None` | `clear_photo` | yes |
| photo-menu Export: the right-clicked photo, **without** telling the InfoBar | none | **no**: actions then targeted a photo the InfoBar did not show |

Not written at all: closing the lightbox over an empty selection, and returning to the Photos page with an empty selection. In both, the InfoBar clears its photo but `selected_photo` kept the old one, so it **went stale** (confirmed; the restore after close re-focuses the lightbox photo only when the return succeeds). Stale consequences: Space reopened a photo no longer selected; the scanner's `PhotosUpdated`/`PhotosIndexed` paths could call `photo_updated` on the stale object and put it back on the InfoBar; collection navigation picked the stale photo's folder.

#### Decisions (by the agent, accepted by the owner 2026-11-23)
1. **One owner: the InfoBar.** The current photo *is* the photo the InfoBar shows. `InfoBar::current_photo()` returns the written object whose id matches `InfoBarTarget::photo`; the `selected_photo` cell and its 10 writes are deleted. No new module: the precedence already lived, table-tested, in `InfoBarPresentation` (deletion test: the cell only duplicated it). Alternative not taken: a separate `CurrentPhoto` model fed the same events. It would be a second copy of the same precedence.
2. **Date browser multi-selection:** its first selected photo becomes current (what the actions used before). `InfoBarEvent::DateBrowserSelection` now sets the photo for any non-empty selection, not only for one. Visible effect: the strip's actions are enabled after picking several photos in the date browser (before, they could be disabled while the actions silently anchored on the first photo).
3. **Stale cases clear.** Closing the lightbox or returning to Photos over an empty selection leaves no current photo (InfoBar behaviour wins).
4. **Photo-menu and Scrapbook-inspector Export** pass their photo explicitly (`PhotoActionContext::export` is now `Fn(&PhotoObject)`), so they no longer change the current photo.
5. **Owner rule** `current-photo` in `scripts/check-presentation-owners.sh`: no `selected_photo*`/`selected_for*` cell, `.replace(` or `.borrow_mut(` outside `src/infobar.rs`.

#### Verification
- Pure: `infobar::presentation` table tests, two new rows in `selection_changes` (written red first).
- Display (Xvfb 1920×1080, each test in its own process since every test calls `gtk::init`): new `infobar::layout_tests::current_photo_follows_the_shown_photo`; still green: all 7 `infobar::layout_tests` (including the 4 candidate 4 keeps), `photo_mark_commands::tests::marks_change_a_whole_selection_or_nothing`, `photo_actions_tests::{copy_and_paste_edits_menu_persists_the_recipe_for_export, add_to_album_popover_scrolls_when_the_album_list_is_long, album_cover_action_is_only_offered_inside_an_album}`.
- `cargo check`, `cargo check --tests`, owner script and `--self-test` ok. `cargo fmt --check` adds no new diffs (the tree already has 214).
- Checked by hand by the owner on 2026-11-23, all working: Space after closing the viewer over an empty selection, a date browser multi-selection favourite, photo-menu Export leaving the InfoBar's photo as the rating target, and everyday marks, rotate, edit and the Properties toggle.

#### Follow-up
- 12 (front surface) is the natural next step; it would also remove the gallery focus callback's own lightbox/overview guard (candidate 4 follow-up).

#### Original brief (chosen by the owner 2026-11-23)
Chosen over 12 because candidate 9's controls act on `selected_photo`, so a stale value changes the wrong photo; it is also candidate 4's open follow-up (decision 2). Start the same way as 9: investigation and design only, then the owner's decisions one at a time, then a branch.

**First step:** trace every writer and reader of `selected_photo` (the 9 writers are listed under "5, 6, 8, 10, 11, 12" below), confirm whether it goes stale when the lightbox closes, and propose one owner with a precedence rule. 12 (front surface) is the natural follow-on.

### 12 · Front surface (merged 2026-11-23)
Branch `refactor/front-surface`.

#### Investigation (at `6678d9e3`, no code changed)
"Which surface is in front" is read from four pieces of state at about 50 sites:
- the `main_stack` page (`photos`, `library`, `albums`, `collage`, `edit`, `folders-overview`);
- `lightbox.root.is_visible()`. The viewer only shows over `photos`/`library`: leaving those pages closes it (`build.rs:3780`), and `open_edit` closes it synchronously before switching to `edit` (`build.rs:3987`);
- `date_browser_active` plus `browser.is_overview_visible()`, and a second copy of the overview flag, the `date_browser_overview_visible` cell (`build.rs:210`);
- the edit page's Space slot, which can outlive the page (`build.rs:4126`), so it re-checks the page.

Two kinds of site:
- **Routing** (~35): the zoom slider value, wheel step, centre-click reset, 1:1, rotate (`build.rs:4177-4686`); digit, Ctrl+A/D and browse keys (`:5292-5389`); the Escape/Space/arrow chain (`shortcuts.rs`); collection navigation reopening the viewer ×10 (`:1279-1780`); the gallery focus guard (`:622`); the mark target (`photo_mark_commands.rs:60`); sidebar crossfade, search and the Edit/Collage buttons (`layout.rs`); the overview checks in `toolbar.rs`.
- **Change handlers** (~17): 6 `lightbox.root.connect_visible_notify`, 7+ `main_stack.connect_visible_child_notify` (`build.rs`, `layout.rs`, `immersive.rs`), and 4 `info.set_date_overview_shown`.

Disagreements:
1. The precedence is written two ways: the zoom slider and reset check lightbox → editor → date overview → gallery; 1:1 checks editor → lightbox. This is harmless only because the viewer never shows over the editor.
2. The zoom slider and reset trust `date_browser_active` without the page; the Escape/back handler also checks `photos`. The date browser exits on any sidebar change (`layout.rs:152`), so it stays active only under Edit or Collage, which win by page.
3. The zoom range has 3 writers (lightbox visible-notify, `open_edit`, the `layout.rs` page notify). That is candidate 6's problem.
4. The InfoBar (`InfoBarPresentation::covered`, `InfoBarPage`), Properties (`set_lightbox_shown`) and the metadata panel (`set_lightbox_open`) each keep their own copy, fed by change handlers.

#### Decisions (2026-11-23, by the owner)
1. **Query now, events later.** A pure `front()` with one precedence and a `FrontSurface::current()` that reads the live widgets; no cached state. The ~17 change handlers and the viewer internals (`src/lightbox/`) stay untouched. A `connect_changed` comes later, when candidate 6 needs it.
2. **Page first.** Editor, Collage, Albums and Folders overview win by page. The viewer counts only over Photos or Library. Then the date overview, then Gallery (Photos) or Library home. `Front` values: `Lightbox`, `Editor`, `Collage`, `DateOverview`, `Gallery`, `LibraryHome`, `Albums`, `FoldersOverview`. The date browser's day view is `Gallery`, as every caller treats it today.
3. **Overview from the widget.** `FrontSurface` reads `date_browser_active` and `browser.is_overview_visible()`; the `date_browser_overview_visible` cell and its writes are deleted.
4. **Every routing check in `src/window`** moves onto `FrontSurface::current()`, and owner rule `front-surface` in `scripts/check-presentation-owners.sh` forbids new ones. The change handlers' own `root.is_visible()` parameter is allowed until the events stage.
5. **No behaviour change.** Keyboard, zoom, navigation, focus and editing behave as today; the precedence differs only in states that can't occur.
6. **Verification:** table tests for the full precedence, unreachable combinations included, written red first; `cargo check`; owner script and `--self-test`; one run each of `PIC_GUI_SUITE=viewer` and `editing` (`scripts/test-gui.sh`); then 1-2 checks by hand. Failures are reported, not worked around in unrelated code. No merge before the owner approves the results.

Settled by the agent, accepted with the plan: one file `src/window/front_surface.rs` (`mod` in `window.rs` like `photo_marks`), since the adapter only reads widgets; the edit Space slot stays and its guard becomes `current() == Editor`; merged with a merge commit like 2, 3, 4, 9 and 10.

**Stages:**
1. Pure `front()` with table tests. Done: `0154ada9`.
2. `FrontSurface` and the `build.rs` routing sites; delete the overview cell. Done: `f07b0b74`.
3. `shortcuts.rs`, `layout.rs`, `toolbar.rs`, `photo_mark_commands.rs`; owner rule. Done: `c983d95d`.
4. Docs: GLOSSARY term **Front surface**, AUDIT-INDEX rows, this file.

**Found while implementing (for the owner to accept):**
- Two sites ask a different question than `current()`, answered by the same reader (no cached state, pure part table-tested):
  - `gallery_covered()`: the gallery focus guard. Collage can open over the date overview (the InfoBar Collage button doesn't check the date browser), and the guard must keep blocking gallery focus there; `current()` would say `Collage` and let it through.
  - `date_browser()`: the sort controls in `toolbar.rs` follow the date browser's level even while the viewer covers its overview; `current()` would say `Lightbox`.
- Left as they were: the page change handlers and viewer visible-notify handlers (decision 1); `opened_from_library` in `open_edit` (page identity: it must say Library even with the viewer open over it); the page name passed to `set_page_allowed`, `set_navigation_view` and the page-switch trace; the viewer's own `root.is_visible()` before `grab_focus`; `toolbar.rs`'s `is_photo_view` check.
- The owner rule does not match `matches!` over page names (the `ALLOW` table separates fields with `|`).
- `PhotoActionContext` gained `front: Weak<FrontSurface>`, like its `lightbox`.
- A transient difference that no routing site reaches: during a page change's notify dispatch the viewer can still be visible for a moment on a page other than Photos/Library (the handler that closes it is one of several); `current()` already reports the new page. No routing check runs inside those handlers.

**Verification (2026-11-23):**
- Pure: `front_surface::tests` (3 tests: every page × viewer × date browser, unreachable combinations included; `gallery_covered`; stack names), each written red first.
- `cargo check`, `cargo check --tests`: no new warnings (`db::set_favorite` unused predates this branch).
- `scripts/check-presentation-owners.sh` and `--self-test` ok, with 7 new `front-surface` cases.
- Display (Xvfb 1920×1080, one process each): `marks_change_a_whole_selection_or_nothing`, `copy_and_paste_edits_menu_persists_the_recipe_for_export`, `add_to_album_popover_scrolls_when_the_album_list_is_long`, `album_cover_action_is_only_offered_inside_an_album`, `photo_context_menu_uses_visible_lightbox_overlay_not_covered_gallery`: all pass.
- GUI harness: the first runs (`20261123T154312Z-294975`, `20261123T154651Z-296879`) stopped before their own checks on stale fixed click points in `gui_test.py`, failing on `main` too since `7f5a72ec`. Fixed separately on `fix/gui-harness-folder-navigation` (merge `467b3ee1`: controls driven by accessible name through AT-SPI, two pixel checks made layout-independent; on `main`: library 7/7, viewer 19/19, editing 39/39) and merged into this branch (`07c5a43a`). On this branch: `viewer` COMPLETE 19/19 (run `20261123T161240Z-326056`), `editing` COMPLETE 39/39 (run `20261123T161546Z-330946`).
- Interaction checks (2026-11-23), scripted with `cargo run` on a private Xvfb against a copy of a harness library, judged from `PICASA_TRACE` route lines, the zoom slider's range and value, and screenshots; the same script on this branch and on `main`, 16/16 on both: the gallery slider resizes thumbnails (`lightbox_visible=false edit_visible=false`); the viewer opens on Space, its slider zooms, Space selects 1:1 (slider at 100%), the 1:1 button returns to Fit, Escape closes it (all `lightbox_visible=true`); the editor's slider and Space route to the editor (`edit_visible=true`) and Escape leaves it; the date overview opens, the slider resizes its tiles, Sort flips the direction (icon) without opening the menu, Escape leaves the overview; no panic or GTK critical. The script could not read the saved sort direction back reliably (unchanged by the press on both branches); the live result was used.
- Earlier checks rerun after merging `main`: `front_surface` tests, `cargo check` (the 3 dead-code warnings are in untouched files), `cargo check --tests`, owner script and `--self-test`, the 5 display tests: all pass.
- Approved by the owner and merged 2026-11-23 (`16e816e3`).

**Follow-up after merge:**
- Events stage (decision 1): a `FrontSurface::connect_changed` to replace the ~17 viewer, page and date-overview change handlers and the private copies in the InfoBar (`covered`, `InfoBarPage`), Properties (`set_lightbox_shown`) and the metadata panel (`set_lightbox_open`). Candidate 6 (zoom slider router) is its first consumer: the zoom range still has 3 writers.
- The owner rule does not catch `matches!` over page names (see above).
- `scripts/gui_test.py` still clicks fixed points in the `browse-navigation` suite (`click_at(118, 524)` for the folder row) and elsewhere outside the library, viewer and editing paths.

### 6 · Zoom slider router (merged 2026-11-23)
Branch `refactor/zoom-slider-router`. Started on the owner's standing instruction to continue autonomously; decisions by the agent, closest to today's behaviour.

#### Investigation (at `20c05eba`, no code changed)
The InfoBar's shared zoom slider (`info.grid_zoom`) has three modes, written by hand:
- **Thumbnails**: 0..7, steps 1/1, no decimals, tooltip "Thumbnail size (Ctrl + wheel)".
- **Photo** (viewer): 0..100, steps 0.25/5, tooltip "Photo zoom — left: Fit, middle: 100%, right: 200%".
- **Edit** (editor): 0..100, same steps, tooltip "Edit zoom — …".

Range writers (4): the viewer visible-notify (`build.rs:4542`, both directions), `open_edit` (`:4101`, before it switches to the edit page), the editor's return to Photos (`:4038`), and the `layout.rs:2466` page handler (Photos/Library, the Collage → Edit → Photos workaround).
Value writers besides those: startup (`:585`), the gallery zoom completion (`:708`, guarded only against the viewer, so it can write a thumbnail value into the editor's slider), the viewer zoom sync (`:4531`), the editor zoom sync (`:4093`), and the library switch (`:7069`, unguarded).
The conversions `lightbox_slider_from_scale` / `lightbox_scale_from_slider` are nested in `build()`; `grid_zoom_slider_value` / `grid_zoom_width_from_slider` are in `window.rs`. None has a test. The wheel step (20 / 10 / 1) is written inline. `slider_last_value` only feeds the `PIC_ZOOM_UI` trace.

#### Decisions (2026-11-23, by the agent)
1. **One owner of what the slider shows** (mode, range, steps, rounding, tooltip, displayed value): pure `zoom_slider.rs` (modes, specs, conversions, wheel step) and a GTK adapter `zoom_slider_widgets.rs` that is a `FrontSurface::connect_changed` listener (candidate 13). Input routing (value changed, centre reset) stays in `build.rs` and uses the pure conversions.
2. **Mode from the front:** `Lightbox` → Photo, `Editor` → Edit, anything else → Thumbnails. The 4 range writers go. Collage now gets the thumbnail range at once instead of keeping the editor's until Photos (the slider is disabled there, so nothing visible changes).
3. **Value writes go through the adapter and apply only in their mode:** a thumbnail width only in Thumbnails, viewer and editor zoom only in Photo/Edit. This fixes the late gallery-zoom completion writing into the editor's slider, and guards the library switch.
4. **Conversions move unchanged** into the pure module, with worked-example table tests (literal values).
5. **Owner rule `zoom-slider`:** range, steps, rounding, tooltip and value writes of `grid_zoom` only in the adapter; the startup default (`:585`) and the InfoBar's construction stay allowed.
6. **Verification:** table tests written red first; a display test of the adapter with a real stack, viewer and slider; the InfoBar and display tests; GUI `viewer` and `editing`; the scripted interaction checks on this branch and on `main`. Merge and push on success.

**Stages:**
1. Pure `zoom_slider.rs` (modes, specs, wheel step, conversions moved unchanged) with worked-example table tests, written red first. Done: `2a387641`.
2. `ZoomSliderWidgets` as a front-change listener; the 4 range writers removed; value writers routed through it; nested and `window.rs` conversions removed. Display test `the_slider_follows_the_front_and_ignores_other_surfaces_zoom` (written after the adapter, not red first). Done: `9caabee6`.
3. Owner rule `zoom-slider`. Done: `1fe543f4`.
4. Docs: GLOSSARY **Zoom slider**, AUDIT-INDEX rows, this file. Done: `93d63303`.

**Changes a user could notice (all latent cases):** a gallery zoom that finishes after the editor opens no longer moves the editor's slider; a library switch no longer re-requests the same thumbnail size through the slider; in Collage the (disabled) slider holds the thumbnail range instead of the editor's.

**Verification (2026-11-23):**
- Unit: 68 pass (`zoom_slider` 5 table tests, `front_surface`, InfoBar, Properties, metadata panel, sidebar).
- Display (Xvfb, one process each): the adapter test, `front_surface_reports_viewer_page_and_date_browser_changes`, the 7 `infobar::layout_tests`, 2 metadata panel tests and the 4 photo-action/marks tests: all pass. (The 2 metadata panel tests that fail on `main` too are unchanged, see 13.)
- `cargo check` / `--tests`: no new warnings. Owner script and `--self-test` ok (3 new cases).
- GUI harness: `viewer` COMPLETE 19/19 (`20261123T172013Z-410514`), `editing` COMPLETE 39/39 (`20261123T172317Z-415265`).
- Scripted interaction checks with `cargo run` (slider range, tooltip and value per mode; viewer and editor zoom, Space, 1:1, Escape; date overview slider and sort): 16/16 on this branch and on `main`.

Merged 2026-11-23 (`9c9c64ff`) on the owner's standing instruction to merge on success.

**Follow-up after merge:**
- Slider input (value changed, centre reset) still lives in `build.rs`, matching on the front; it could move into the adapter if it grows.
- The InfoBar's construction tooltip ("…; click the middle to reset") is replaced by the mode tooltip after the first mode change, as before.

### 13 · Front-surface change event (merged 2026-11-23)
Branch `refactor/front-surface-events`. Follow-up to 12 (its decision 1); candidate 6 (zoom slider router) is to be the event's first behaviour consumer.

#### Investigation (at `38e89e45`, no code changed)
17 handlers react to the front changing, in connection order:
- **Viewer shown/hidden** (`lightbox.root.connect_visible_notify`): `build.rs:223` Properties `set_lightbox_shown`; `:746` on hide refresh the InfoBar selection, then `info.set_lightbox_shown`; `:3744` `PIC_LIGHTBOX_LAYOUT` trace; `:4289` close-return (on show freeze the folder scroll animation; on hide 1:1 off, Properties shows the returned photo, restore the selection on a timer; order-sensitive); `:4542` zoom range swap; `:5429` `metadata_panel.set_lightbox_open` (plus `close_started` at `:5439`, before the fade).
- **Page changed** (`main_stack.connect_visible_child_notify`): `:3308` Edit/Collage cancel a sidebar fade; `:3699` Properties `set_page_allowed`; `:3781` leaving Photos/Library closes the viewer, and the Library redirect switches the page again inside the notify (re-entrant); `:3895` collage/editor teardown and History/filter refresh; `layout.rs:2447` `info.set_page` + selection refresh + zoom range to 0..7; `:5448` metadata panel `set_navigation_view` (plus a tick callback for filter changes); `immersive.rs:675` ends Immersive off gallery pages.
- **Date overview on/off**: 4 hand calls of `info.set_date_overview_shown` (`build.rs:2689, 2722, 2748, 2767`), its only listener.

Private copies: InfoBar (`InfoBarPresentation` lightbox/overview/collage), Properties (lightbox shown, page allowed), metadata panel (lightbox open, navigation view). The zoom range has 4 writers (`:4542`, `open_edit` `:4101`, the editor's return `:4038`, `layout.rs:2466`).

Hazards: re-entrancy (a page change closes the viewer and may switch the page again, nested); order dependence (`:4289` turns 1:1 off after the viewer is hidden; the InfoBar refresh precedes `set_lightbox_shown`); two closing moments (`close_started` vs hidden); the date overview has no notification of its own (the overview widget becomes visible only after `on_overview` returns).

#### Decisions (2026-11-23; 1-2 by the owner, 3-8 by the agent at the owner's request, closest to today's behaviour)
1. **Scope: the event plus the state mirrors.** Move only the handlers that mirror state: InfoBar (viewer, overview, page with its selection refresh), Properties (viewer, page allowed), metadata panel (viewer open, navigation view). Behaviour handlers (close-return, close-on-page-change, teardown, fade cancel, Immersive, trace) and the zoom range stay; the zoom range moves in candidate 6.
2. **Delivery: synchronous, nested, first.** `FrontSurface` connects to the viewer and stack notifies before any other handler and calls listeners at once, in registration order.
3. **Payload:** `FrontChange { before, after }` of `FrontState { page, lightbox_shown, date_browser }`; emitted when any of the three changes (not only `Front`), decided by a pure diff. Listeners derive `front()` / `gallery_covered()` from it.
4. **Re-entrancy:** a change made during a dispatch is delivered at once (nested); the outer dispatch then stops, because the remaining listeners already received the newer state. No listener is left on a stale state. The dispatcher is plain Rust and table-tested. The last delivered state is kept only to compute `before`; `current()` still reads the widgets.
5. **Date overview:** `FrontSurface` watches the overview widget's visibility and is told where `date_browser_active` changes; the 4 hand calls go.
6. **The mirrors keep their own inputs** (they are the inputs of their table-tested models); this refactor replaces their 8 hand-wired feeds with one. Removing the copies is later work.
7. **Ownership:** the listeners live in `src/window/front_mirrors.rs`; owner rule `front-mirrors` keeps those setters there (the metadata panel's `close_started` call stays allowed: it is a closing moment, not a front change).
8. **Verification:** table tests for the diff and dispatcher (written red first); a display test of `FrontSurface` with a real stack, viewer and date browser, including a nested change; the existing InfoBar and display tests; GUI `viewer` and `editing`; the scripted interaction checks on this branch and on `main`.

**Stages:**
1. `FrontState`, `change`, `FrontEvents` with table tests (diff, order, nested delivery). Done: `2163334b`.
2. `FrontSurface` emits changes; date browser hook; `FrontMirrors` replaces the 8 feeds; display test. Done: `2cb02e47`.
3. Owner rule `front-mirrors` (it also caught candidate 12's allowance for the removed initial metadata call, now dropped). Done: `9f7144c6`.
4. Docs: GLOSSARY **Front change**, AUDIT-INDEX rows, this file. Done: `06aa0aaf`.

**Found while implementing:**
- GTK queues a page change made inside a page notification (the Library redirect) until the notification ends, so listeners see Photos, then Library, as two changes; only a change of another widget (the viewer closed during a page change) arrives nested. Both are covered by the display test; no listener is left on an older state either way.
- The InfoBar now hears the date overview only when it turns on or off. Before, each year/month step inside the overview re-sent "overview shown", which also cleared a photo the scanner had put on the InfoBar meanwhile. The overview already clears it on entry.
- `Page::stack_name` (the reverse of `from_stack_name`) has a round-trip test written after the function.

**Verification (2026-11-23):**
- Unit: `front_surface` (6 table tests), InfoBar, Properties, metadata panel and sidebar unit tests: 63 pass.
- Display (Xvfb 1920×1080, one process each): `front_surface_reports_viewer_page_and_date_browser_changes`, the 7 `infobar::layout_tests`, `opening_flash_choices_loads_indexed_values`, `metadata_panel_navigation_auto_closes_and_allows_manual_reopen`, and the 5 photo-action/marks tests: all pass. `metadata_filters_actual_window_folder_album_and_layouts` (times out at `metadata_panel.rs:1873`) and `metadata_panel_theme_layout_and_close_reopen` (`panel.button.parent()` at `:1800`) fail identically on `main` at `38e89e45`: pre-existing, not investigated.
- `cargo check` / `--tests`: no new warnings. Owner script and `--self-test` ok.
- GUI harness: `viewer` COMPLETE 19/19 (`20261123T170514Z-385101`), `editing` COMPLETE 39/39 (`20261123T170817Z-389860`).
- Scripted interaction checks (gallery/viewer/editor zoom, Space, 1:1, Escape, date overview, sort) with `cargo run`: 16/16 on this branch and on `main`.

Merged 2026-11-23 (`888408e0`) on the owner's standing instruction to merge on success.

**Follow-up after merge:**
- Candidate 6 (zoom slider router) is next: the zoom range (4 writers) becomes a `connect_changed` listener.
- The remaining behaviour handlers (close-return, close-on-page-change, teardown, fade cancel, Immersive, trace) still connect to GTK directly; move them only with a reason.
- The mirrors still keep their own copies of the inputs (decision 6).

### 11 · Window session state (merged 2026-11-23)
Branch `refactor/window-session-state`.

#### Investigation (at `0006dbb4`, no code changed)
Session keys (`src/window.rs:60-70`, plus `compact-sidebar-width-v2` local to `layout.rs:1153`) and where they are read and written:
- **Startup load** (`build.rs:69-120`): window size (default 1440×900, clamped 640..7680 × 480..4320), maximized, last view, view anchor photo, activated photo, gallery scroll and albums scroll (default 0, floored at 0).
- **Last view:** written on every destination change (`layout.rs:162`) and on close (`build.rs:6998`). Decoded by `sidebar_filter_from_setting` (`window.rs:333`): with no saved view or an invalid one, it falls back to Recently Added. The library switch (`build.rs:6827`) instead lands on Library when nothing is saved; its comment says this is deliberate ("A new library has no browsing destination yet").
- **Activated photo:** written by the viewer's photo-changed handler (`build.rs:236`), reloaded on switch (`:6899`).
- **Viewport** (scroll, anchor photo, albums scroll): written only on close (`build.rs:7003-7027`), read only at startup (`:5384-5398`). **The library switch neither saves the outgoing library's viewport nor restores the incoming one's;** after switching away and back, the next startup restores the scroll from the last close of that library.
- **Window size and maximized:** written on close only (`:7029-7046`).
- **Sidebar fraction:** default `0.22`, clamped `0.10..0.70`, read in two places with the same default and clamp (`layout.rs:1252`, `sidebar_widgets.rs:396`; the review's "only `layout.rs` clamps" is out of date); written on drag end (`layout.rs:2068`) and on close when the mode is normal (`build.rs:7048-7058`).
- **Collapsed (drawer) width** (`layout.rs:1263`, written `:2058`), **compact rail width** (`layout.rs:1157`, written `:1232`, `:2045`) and **Properties fraction** (`build.rs:3319`, default 0.25 clamped 0.10..0.55; written `:3434`) follow the same read-clamp / write-on-drag-end pattern.
- Each site formats and parses its own value; write errors are printed at the drag-end sites and ignored on close. None of the defaults or clamps is tested; only the view encoding is (`window.rs:373`).

#### Decisions (2026-11-23, by the agent on the owner's standing instruction; closest to today's behaviour)
1. **Scope:** the 11 keys above. Sort, grouping, rating filter, thumbnail size and sidebar mode are preferences with owners of their own and stay out.
2. **Module:** `src/window/session_state.rs`, pure. It owns the keys, the defaults and clamps, the view encoding (`sidebar_filter_setting` / `sidebar_filter_from_setting` move in with their tests), `SessionState::load`, and the writes. Callers pass plain values; no GTK in the module.
3. **Seam:** a `SessionStore` trait (get / set / delete). Two adapters: `rusqlite::Connection` (the app) and an in-memory map (the table tests). The app keeps its one connection; the module never opens one.
4. **Default view stays different per entry**, deliberately: `Entry::Startup` falls back to Recently Added, `Entry::LibrarySwitch` lands on Library when nothing is saved (an invalid saved value still falls back to Recently Added in both, as today). The rule is now one tested function instead of two call sites.
5. **Leaving a library saves the same snapshot as closing:** view, viewport (scroll and anchor, or albums scroll), window size/maximized, and the sidebar fraction when the mode is normal. The switch saves it to the outgoing connection before opening the new one. This is the one behaviour change.
6. **Restoring the incoming library's viewport after a switch is not in scope.** Today the switched-to gallery starts at the top; restoring needs to wait for the stream build as startup does. Recorded as follow-up.
7. **Clamps stay where they are applied today:** on load, not on save.
8. **Owner rule `session-state`:** the session keys may be named only in `session_state.rs` and the window test fixtures that seed a library.

**Stages:**
1. `session_state.rs` with 14 table tests, written red first. Done: `c6644b04`.
2. Startup, library switch, close, sidebar/drawer/compact widths and the Properties fraction routed through it; one `leave_snapshot` closure in `build.rs` serves close and switch. Done: `629c34c5`.
3. Owner rule `session-state` (its self-test proves a key read, a key literal and a re-declared key outside the module are caught). Done: `9289e1de`.
4. Docs: GLOSSARY **Window session**, AUDIT-INDEX rows, this file. Done: `703322bd`.

**Found while implementing:**
- A non-finite stored length or fraction ("NaN", "inf") now counts as not saved instead of reaching GTK; before, `clamp` passed NaN through.
- Close now prints a write failure (it used to ignore it), like the drag-end sites.
- `numeric_setting` and the view-encoding functions in `window.rs` had no other callers and went with the move.

**Verification (2026-11-23):**
- Unit: `window::session_state` 14 pass; all `window::` unit tests 148 pass.
- Display (Xvfb 1920×1080, one process each): the 30 window tests that build a window from a seeded session (`responsive_window_tests`, `immersive_window_tests`, `photo_action_tests`, `startup_gallery_tests`, `photo_layout`, onboarding reopen): all pass.
- `cargo check` / `--tests`: no new warnings (only the 3 known dead-code ones). Owner script and `--self-test` ok.
- GUI harness: `viewer` 19/19, `editing` 39/39.
- Scripted interaction checks: 16/16 on this branch and on `main`. They stop the app by killing it, so they never reach Exit; **nothing automated covers the close or library-switch save end to end** (the snapshot itself is table-tested).

Merged 2026-11-23 (`4f47d674`) by the owner (the agent's merge was refused by the auto-mode permission check).

**Follow-up after merge:**
- Restore the incoming library's viewport after a switch (decision 6): wait for the stream build as `populate_startup_gallery` does, then apply the activated-photo / anchor rule.
- A scripted check that exits through the Exit confirmation and switches library would cover the wiring.

### 5 · Gallery scroll motion (started 2026-11-24)
Branch `refactor/gallery-scroll-motion`.

#### What "feels off" turned out to be (2026-11-24, no code changed)
The owner felt the wheel glide had changed since it went in (`88eb5d38`). It has not:
- `src/window/navigation.rs` and `src/grid/navigation.rs` are identical between `88eb5d38` and `a4ccba2e`; both galleries install the glide at the same places.
- The only new per-frame work since is the header fold tick (`toolbar.rs:656`, from `9d1a29eb`).
- Both builds were driven in Xvfb on a copy of the FB-All library (6 single clicks, then a 12-click burst; `PIC_WHEEL_GLIDE_TRACE` + `PIC_SCROLL_FRAME_TRACE`). Folders and All Photos produced the same glide durations (224 ms single, 224 to 360 ms in a burst), the same per-click step curve and the same frame pacing within noise.
- The owner's own trace on the real 75 Hz desktop (210 clicks, 866 glide frames): median 13.34 ms, 12 frames over 20 ms. The owner concluded it was their eyes. Nothing from it feeds this candidate.

#### Investigation (at `a4ccba2e`)
- Wheel glide: already one engine (`WheelGlide`, tested) for both galleries. Out of scope.
- Scrub, sampling, settle and the per-frame thumbnail pump are written twice inline in `build.rs` (All Photos `:1979-2209`, Folders `:2211-2468`), with different numbers and slightly different rules:

| Rule | All Photos | Folders |
|---|---|---|
| A move starts a scrub when it is at least | 1.75 pages, and at least 900 px | 1.60 pages |
| Scrub target sampling | in the value-changed handler, at most every 40 ms | in the frame pump, at most every 50 ms (`FolderScrollbarScrub`) |
| Settle | 90 ms after the last move, only while scrubbing | 110 ms after every move |
| Pump frames after a move | 12 | 12 |
| Visible tiles queued per pump frame | 128 | 96 |
| Warm-ahead | every 3rd pump frame, 6 quiet pump frames after a scrub move | every 3rd pump frame, never while scrubbing |
| Warm-ahead tiles | 32 | 24 |
| Scrub target tiles | 192 | 192 |

- `src/smooth_scroll.rs` (274 lines) has no callers. Its `PIC_SMOOTH_SCROLL*` variables are tuning knobs of that dead module and are not in `docs/development/TRACING.md`.
- Only `FolderScrollbarScrub` (`window.rs:559`) has a test.

#### Decisions (2026-11-24, owner)
1. **Move, don't retune.** Each gallery keeps its own numbers and rules from the table above; nothing the owner sees or feels changes. With both specs side by side in one file, the owner compares scrollbar drags by hand and decides later whether to match them.
2. **Module:** `src/window/scroll_motion.rs`, pure. A `ScrollMotionSpec` per gallery (`ALL_PHOTOS`, `FOLDERS`) holds every number in the table; a `ScrollMotion` owns the scrub state, the sample coalescing, the settle ticket, the pump countdown and the warm-ahead cadence. `FolderScrollbarScrub` and its test fold into it.
3. **Seam:** the `ScrollMotion` interface (`moved`, `take_sample`, `settle`, `frame`), tested with table tests on plain values. The GTK work (queues, refreshes, timers, the Folder whole-pixel snap, the settle prefetch loop) stays in `build.rs` as the adapter.
4. **One rule change, invisible:** both galleries start a scrub at "at least" the threshold (Folders used "more than"). They differ only when a move is exactly 1.60 pages to the pixel.
5. **Delete `src/smooth_scroll.rs`.**

#### Stages
1. Pure `scroll_motion.rs`, test-first (9 table tests: scrub start per spec, scrub lasts until settle, newest settle ticket only, settle per gallery, 12-frame pump, warm-ahead cadence, warm-ahead during a scrub, sampling on moves / in the pump). Done: `9a5aeb9c`.
2. Both galleries' value-changed handlers, pump ticks and settle timers ask `ScrollMotion`; `FolderScrollbarScrub` folded in. Done: `e91bc8be`.
3. `src/smooth_scroll.rs` removed. Done: `bb8c85e9`.
4. Owner rule `scroll-motion` (no hand-kept scrub cells, pump constants or scrub sampler outside the module) with 3 self-test cases. Done: `acff6213`.
5. Docs: `AUDIT-INDEX.md` (module row, scroll pipeline, Quick-Edit rows), `GLOSSARY.md` (Scrub, Settle, Gallery scroll motion). Done: `e4ebb222`.

#### Verification (2026-11-24, at `e4ebb222`)
- `cargo check --locked`: only the 3 known dead-code warnings.
- Unit suite once: 996 passed, 0 failed (220 ignored).
- Display (Xvfb 1920×1080, one process each): `grid_wheel_glides_whole_pixels_to_an_exact_stop`, `grid_scrub_settle_preserves_loaded_thumbnail_after_ram_eviction`, `photo_wall_view_toggle_keeps_selection_and_grid_scroll_context`, `sectioned_folder_visibility_uses_scroll_viewport_not_content_height`: all pass.
- Owner script ok, `--self-test` ok.
- GUI harness `viewer` (debug binary via `PIC_GUI_APP`): 19 PASS, 0 FAIL.
- Real app, `main` (`a4ccba2e`) against the branch, Xvfb, a copy of the FB-All library (9,283 photos), Folders and All Photos: 6 wheel clicks, a 12-click burst, then a scrollbar drag down and back. The same glide (durations, 960/1920 px), the same scrub (13 or 22-23 multi-page jumps, the same final position), frame pacing within noise, and every thumbnail loaded after the settle in both builds.
- `scripts/front-surface-checks/run.sh`: 16/16 passed. The first runs aborted before any check: its AT-SPI bus launcher did not bring the bus up, and the clients' D-Bus activation fallback was denied. Fixed by starting the launcher with `ATSPI_DBUS_IMPLEMENTATION=dbus-daemon`, as `scripts/test-gui.sh` does.

**For the owner:** drag the scrollbar in Folders and in All Photos. If one feels better, matching the other is a change to `ALL_PHOTOS` or `FOLDERS` in `src/window/scroll_motion.rs`.

### Next (updated 2026-11-23, after 11 merged)
Done today: 12 (front surface), 13 (front-surface change event), 6 (zoom slider router), 11 (window session state, merge `4f47d674`, merged by the owner); the GUI harness was fixed separately (`467b3ee1`).

Open candidates, in order:
1. **5 · One gallery scroll-motion module.** Awaiting merge (2026-11-24, see its section). The owner is testing glide scrolling because "something feels off"; nothing from 12, 13, 6 or 11 touches scroll code. Start by asking the owner what feels off (which view, which input) and fold that into the investigation. Then the usual: investigation and design, decisions, branch. Facts are under "5, 6, 8, 10, 11, 12" below (two inline copies of scrub/settle in `build.rs` with different constants, unused `smooth_scroll.rs`, no tests).
2. 11's follow-up (restore the incoming library's viewport after a switch; a scripted Exit / library-switch check), when the owner wants it.
3. 7 (diagnostic only) and 8 (speculative) can wait, or be closed with a note.

How the owner works with these (from this session):
- Each candidate: investigation, then decisions recorded here, then a branch, staged commits (pure module test-first, GTK adapter, owner rule, docs), then verification, merge with a merge commit, push.
- Verification each time: unit and display tests (Xvfb, one process each), `cargo check`, owner script and `--self-test`, GUI harness `viewer` and `editing` (`scripts/test-gui.sh`), and the scripted interaction checks `scripts/front-surface-checks/run.sh` (`cargo run`, private display, copy of a harness library) on the branch and on `main` (`PIC_CHECK_REPO=<main worktree>`).
- The owner has allowed deciding open questions autonomously and merging and pushing on success; say what was decided and why. Since 2026-11-23 the agent's `git merge` into `main` is refused by the auto-mode permission check: push the branch and hand the merge to the owner.
- Pre-existing, not caused by today's work: two metadata panel display tests fail on `main` too (see 13); 3 dead-code warnings in `db/photos.rs` and `grid/honeycomb_raster.rs`.

### 7 · Editor preview plan
**Lower priority (2026-11-23):** its disagreement is diagnostic only (a mislabelled trace); 9 goes first.

Re-checked 2026-11-23 (round two): the wrong reason only mislabels the trace; it never changes the renderer.
- `cpu_fallback_reason` (`src/edit/editor.rs:166`) is used only in the `PIC_EDIT_RENDERER renderer=cpu_fallback` trace (`:1415`, gated on `PICASA_TRACE`), plus a test at `:3018` that locks in the wrong `"clarity"` reason.
- It names clarity, grain, filter and B&W/sepia, which the GPU allows, and never names the real reasons: partial `filter_amount`, crop or straighten with auto, a view that is not 1:1, rotation, or `gpu_failed`. The trace fires inside `if let Some(gpu_canvas)`, so a `gpu_failed` fallback is reported as a recipe reason.
- The renderer choice is `gpu_tone_supported_for_view` (`live_gpu.rs:1600-1632`), called at `editor.rs:1270` (renderer, `decode_rotation`, base recipe via `gpu_base_recipe` at `:1278`) and rebuilt by hand at `:1529` (`has_source_for`). Those two copies can drift.
- `gpu_tone_supported` alone decides the filter tiles (`editor.rs:889`), deliberately without the view and rotation check.

The planning module is still the design: renderer, base recipe, rotation and reason together, with pure recipe-table tests. CI: `editor-quality.yml`.

### 5, 6, 8, 10, 11, 12
Not started (10 has since merged, 12 has started; see their sections). The problem statements are in the review reports. Re-checked 2026-11-23:
- 5: `smooth_scroll.rs` (274 lines) still has no callers; both `install_*_smooth_gallery_scroll` functions remain; the galleries already share `WheelGlide`. Scrub and settle are written twice inline in `build.rs` with different constants: All Photos scrubs at 1.75 pages (≥ 900 px) and settles after 90 ms; Folder scrubs at 1.60 pages and settles after 110 ms. None of it is tested.
- 6: `window/build.rs` still has 43 `is_visible()` checks. The zoom slider has 10 writers of its value or range (2 unguarded: `build.rs:580`, `:7233`), the 0..7 range setup is copied 3 times (`build.rs:4045`, `:4667`, `layout.rs:2465`), and `lightbox_scale_from_slider` / `lightbox_slider_from_scale` are untested nested functions in `build()`. Best done after 12, as its consumer.
- 8: still Speculative: 12 styles, about 63 style matches across `screensaver.rs` and `screensaver/gpu*.rs`.
- 10: `selected_photo` is written in 9 places (`build.rs:248, :632, :2671, :2777, :4074, :7189`; `photo_actions.rs:272, :1369, :1453`) with no precedence rule. None of the 6 `lightbox.root.connect_visible_notify` handlers writes it, so it may go stale when the lightbox closes; needs checking. This is the candidate 4 follow-up (decision 2) and pairs with 9.
- 11: defaults differ: with no saved last view, startup lands on Recently Added (`window.rs:368`) but a library switch lands on Library (`build.rs:7130`). The sidebar default `0.22` is copied at `layout.rs:1255` and `sidebar_widgets.rs:400`, and only `layout.rs` clamps it. Scroll and anchor are saved only on close, not before a library switch.
- 12: "which surface is in front" is re-derived at about 35 sites (28 `lightbox.root.is_visible()` and 7 `visible_child_name() == Some("edit")` in `build.rs`, plus `layout.rs` and `photo_actions.rs`). One owner would serve 6, 9, 10 and the InfoBar's cover rule.
