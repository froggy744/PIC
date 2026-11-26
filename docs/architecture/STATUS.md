# Architecture review status

The single source of truth for the deepening candidates from the architecture reviews. Open the reports in a browser; their status boxes are history, so update this file instead.

- Round one, candidates 1-8: [`architecture-review-2026-11-22.html`](architecture-review-2026-11-22.html), taken at `main` @ `5356ef20`.
- Round two, re-checks 5-8 and adds 9-12: [`architecture-review-2026-11-23.html`](architecture-review-2026-11-23.html), taken at `main` @ `07cabed4`. The report labels the new candidates N1-N4; they are 9-12 here.
- Round three, candidates 14-20: [`architecture-review-2026-11-26.html`](architecture-review-2026-11-26.html), taken at `feature/smart-albums` @ `13855231` (main + Smart Albums). All deferred, see "Round three" below.

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
| 5 | One gallery scroll-motion module | Worth exploring | merged | merge `c169225e` (`refactor/gallery-scroll-motion`) | `src/window/scroll_motion.rs` (pure), `src/window/build.rs` (GTK adapter); wheel glide stays in `src/window/navigation.rs` |
| 6 | Zoom slider router | Worth exploring | merged | merge `9c9c64ff` (`refactor/zoom-slider-router`) | `src/window/zoom_slider.rs` (pure), `src/window/zoom_slider_widgets.rs` (GTK adapter, a front-change listener) |
| 7 | Editor preview plan | Worth exploring | closed 2026-11-24 (diagnostic only; see its section) | none | `src/edit/live_gpu.rs`, `src/edit/editor.rs` |
| 8 | Screensaver style descriptor | Speculative | merged | merge `dece41f2` (`refactor/screensaver-style-spec`) | `src/screensaver.rs`, `src/screensaver/gpu.rs` |
| 9 | Photo mark commands (favourite, rating) | Strong | merged | merge `9f056fe3` (`refactor/photo-mark-commands`) | `src/window/photo_marks.rs` (pure), `src/window/photo_mark_commands.rs` (GTK adapter) |
| 10 | Current photo (`selected_photo`) | Strong | merged | merge `49a93dd4` (`refactor/current-photo`) | `src/infobar_presentation.rs` (precedence, pure), `src/infobar.rs` (`InfoBar::current_photo`) |
| 11 | Window session state | Worth exploring | merged | merge `4f47d674` (`refactor/window-session-state`) | `src/window/session_state.rs` (pure: keys, defaults, clamps, load, record; `SessionStore` seam) |
| 12 | Front surface (widens 6) | Worth exploring | merged | merge `16e816e3` (`refactor/front-surface`) | `src/window/front_surface.rs` (pure precedence + `FrontSurface` reader) |
| 13 | Front-surface change event (follow-up to 12, before 6) | Worth exploring | merged | merge `888408e0` (`refactor/front-surface-events`) | `src/window/front_surface.rs` (state, diff, dispatcher), `src/window/front_mirrors.rs` (listeners) |
| 14 | Browse scope: one value for "what am I browsing" | Strong | merged | merge `16fa04e7` (`refactor/browse-scope`) | `src/db/browse_scope.rs` (pure: `BrowseScope`, `RatingFilter`, `BrowseSnapshot`), `src/window/library.rs` (sidebar mapping, Files check) |
| 15 | One photo-list pipeline (grid, startup, library switch, navigation) | Strong | merged | merge `456325ac` (`refactor/photo-list-pipeline`) | `src/window/photo_list.rs` (pipeline, plain values), `src/window/library.rs` and `build.rs` (GTK adapters) |
| 16 | Photo change: one notification, one fan-out | Strong | merged | merge `41d31b9c` (`refactor/photo-change`) | `src/window/photo_change.rs` (pure: `ChangeKind`, `ChangeView`, `FanOut`), `src/window/photo_change_widgets.rs` (GTK adapter `photo_changed`) |
| 17 | Album list: one owner of the albums as shown | Strong | merged | merge `2d7a5e27` (`refactor/album-list`) | `src/window/album_list.rs` (pure: changes, plan, current results), `src/window/album_list_widgets.rs` (GTK adapter `AlbumList`) |
| 18 | Collection navigation out of `build()` | Strong | merged | merge `ecc8e954` (`refactor/collection-navigation`) | `src/window/collection_navigation.rs` (pure), `src/window/collection_navigation_widgets.rs` (GTK adapter) |
| 19 | Scan events as a reducer | Worth exploring | in progress (`refactor/scan-events`) | none | `src/window/scan_view.rs` (pure reducer), `src/window/scan_view_widgets.rs` (GTK adapter) |
| 20 | PhotoActionContext shrinks (follows 14, 16, 17) | Speculative | deferred | none | `src/window.rs` |

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

**Follow-up after merge:** done 2026-11-24 on `fix/library-switch-viewport`, see "11 follow-up" below.

#### 11 follow-up · Library switch viewport (2026-11-24)
- **Switching back restores the viewport** (decision 6's follow-up): after a switch, `when_gallery_built` waits for the incoming gallery, then `restore_saved_viewport` applies startup's rule (albums scroll; else the last opened photo; else the anchor photo and scroll). Any navigation before then wins (`REFRESH_GENERATION`). Startup uses the same two functions (`window/startup.rs`).
- **Found by the new check, also on `main`:**
  - A restart came back about 200 px off in All Photos: the restore ran after the build but before the split gave the sidebar its width, and the grid then reflowed from 4 to 3 columns. The restore now also waits until the gallery surface keeps one width for 100 ms; on screen it lands about a third of a second later, in the right place.
  - In Folders, focusing the anchor tile scrolled it fully into view when the saved viewport cut it off (up to a tile away). The saved scroll is applied again after focus (`Gallery::restore_view`).
- **Scripted check** `scripts/library-switch-checks/run.sh` (reuses the front-surface driver, which gained a prepare hook, a restart on the same library and a window size): two libraries (A, and B with A's first 8 photos so a switch really rebuilds the gallery), scroll A, switch to B in Settings > Libraries, back to A, one wheel click, Exit through the confirmation, restart, Exit again. The saved scroll is read from A's database: 320 on leaving, 480 (320 restored + 160) after Exit, 480 after the restart.
  - On `main`: All Photos switch-back FAIL (268), restart FAIL (+204 px); Folders switch-back FAIL (333 for 480).
  - On the branch: 5/5 in All Photos and 5/5 in Folders (`PIC_CHECK_VIEW=folder:1`).
- Verification: unit suite 996 passed, 0 failed; startup display tests pass; owner script and `--self-test` ok; GUI harness `viewer` 19/19. GUI harness `editing` 27 PASS, **2 FAIL** (Editor Undo/Redo sequences, Editor Reset; 29 checks, against 39/39 on 2026-11-23). This branch changes no editor code; the run against `main` for comparison was interrupted (BLOCKED), so whether the failures are older is **still open**: run `PIC_GUI_SUITE=editing scripts/test-gui.sh` on `main`.
- The owner checked the switch and restart by hand and merged ("all work").

### 5 · Gallery scroll motion (merged 2026-11-24)
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

### 8 · Screensaver style descriptor (merged 2026-11-24)
Branch `refactor/screensaver-style-spec`. Chosen by the owner after 5 and the 11 follow-up.

#### Investigation (at `146b36e6`)
12 styles (`enum Style`, `screensaver.rs:58`). Each style's *facts* are spread over separate matches:
- in `impl Style`: `default_tiles`, `from_saved`, `key`, `label`, `subtitle`, `single_photo`, `matrix`, `uses_photos`, `available`;
- `default_background_colour` (`:397`), the Settings tile limit (`:1198`), the decode size per style family (`:1510`), the player's slot count (`:1687`);
- which renderer `gpu::Stage::new` builds (`gpu.rs:528-549`: cinema, Cover Flow, flying, pile, or the tile wall).

The rest of the ~200 `Style::` uses are per-style *drawing* (the Settings preview icons, `gpu/pile.rs`, `gpu/cinema.rs`) and stay where they are. Most facts depend on a style's family, not the style:

| Stage | Styles | Tile limit | Player slots | Decode size |
|---|---|---|---|---|
| Wall | Mosaic, Changing Grid (and Surprise Me in Settings) | 10,000 | the tile count | 1536 px down to 48 px by tile count |
| Prints | Picture Pile, Floating Prints, Photo Drift | 64 | the tile count | 1920×1280 down to 640×430 |
| Flying | Flying Photos | 32 | the tile count | 1920×1280, or 1440×960 above 6 |
| Cover Flow | Cover Flow | 10,000 | none | 2560×1600 |
| Cinema | Ken Burns, Fullscreen Fade, Matrix Rain, Matrix Ken Burns | 10,000 | none | 2560×1600 |

43 tests in `screensaver.rs` (plus 58 under `screensaver/`).

#### Decisions (2026-11-24, by the agent on the owner's standing instruction; closest to today's behaviour)
1. **Move, don't change.** Every number and rule above stays; nothing on screen changes.
2. **Module:** `src/screensaver/style.rs`, pure. `Style` moves there with `Style::spec() -> StyleSpec` (key, label, subtitle, stage, default tiles, background, photos, Terminal-only) as the one place a style's facts live, and `Stage` with `tile_limit`, `slots` and `decode_size`. The existing `Style` methods stay as thin readers of the spec, so callers do not change shape.
3. **Seam:** `Style::spec` and the `Stage` functions, tested with table tests over all 12 styles.
4. **Surprise Me** has the Wall stage: it is resolved to a real style before the player starts (`screensaver.rs:1376`) and behaves like Wall in Settings. Its player slot count would change from none to the tile count, but no player ever receives it unresolved.
5. **Per-style drawing stays** in the preview, `gpu/pile.rs` and `gpu/cinema.rs`. Adding a style is then: the variant, its `CHOICES` entry, its spec, and its drawing.
6. **No owner rule.** The rules guard widget writers; here the risk is a new `match` restating a style fact, which no text pattern tells apart from per-style drawing. The spec's table tests and AUDIT-INDEX's "adding a style" line carry it instead.

#### Stages
1. Pure `screensaver/style.rs` (`StyleSpec`, `Stage`), 4 table tests written red first. Done: `97fa06ed`.
2. `Style` moved there; its methods, the background colour, the Settings tile limit, the decode size, the player's slots and `gpu::Stage`'s renderer choice read the spec. Done: `91dca8f9`.
3. Docs: AUDIT-INDEX (`src/screensaver/` entry, Quick-Edit row), GLOSSARY (Screensaver style spec, Screensaver stage). Done: `ace360fa`.

#### Verification (2026-11-24)
- `cargo check`: only the 3 known warnings; rustfmt clean on the touched files.
- Unit suite: 1000 passed, 0 failed (96 under `screensaver`, 4 new).
- Display (Xvfb, one process each): 8 of the 9 screensaver display tests pass. `screensaver_coverflow_renders_in_a_live_gtk_window` fails on `main` too (empty screenshot; it needs real OpenGL, which this Xvfb lacks, with Cairo or GL renderer).
- GUI harness `screensaver`: its report ends at the editing prerequisites (27 PASS, 2 FAIL: the pre-existing editor checks below), so it reached no screensaver step.
- The owner checked Cover Flow and the other styles by hand: working.
- **Pre-existing, found on 2026-11-24:** GUI harness `editing` failed "Editor Undo/Redo sequences" and "Editor Reset" on `954a99db` too (27/29). The owner confirmed undo, redo and Reset work by hand; the cause was the harness clicking moved buttons, fixed on `fix/gui-harness-editor-checks` (see Next). Undo history is per editing session by design; it does not survive closing the photo.

### 16 · Photo change: one notification, one fan-out (merged 2026-11-26)
Merged into `main` 2026-11-26 (merge `41d31b9c`), approved by the owner; branch deleted. Open: the follow-up below (folder favourite Smart Album recount). Next in order: 18.

Started 2026-11-26 at `main` @ `f94fa7e5` on `refactor/photo-change`. Design decisions taken by the agent on the owner's standing instruction (closest to today's behaviour, drift fixed).

#### Before (at `f94fa7e5`)
Eight change sites, each refreshing its own subset (read from the code):

| Site | Gallery tiles | Viewer objects | InfoBar | Properties (front) | Library counts | Album counts | View membership |
|---|---|---|---|---|---|---|---|
| Marks (`photo_mark_commands::run`) | yes | shown photo only | yes | yes | yes | yes | `photo_marks::plan` |
| Editor saved (`build.rs` `saved`) | yes | no (viewer is closed while editing) | yes | no | no | yes | open Smart Album with an Edited rule |
| Bulk edit (`finish_bulk_recipe`) | yes | yes | if shown | no | no | yes | any metadata filter, History, or Reset |
| Rotate (`rotate_current_photo`) | yes | `refresh_current` | yes | yes | no | yes | open Smart Album with an Edited rule |
| Trash (`trash_photos`) | rebuild | closes | cleared | not cleared | yes | yes | rebuild |
| Rename (`show_rename_dialog`) | the clicked object only | no | yes | **no** (file name, path stale) | no | no | **never** (search and Name sort ignored) |
| Tags (Properties chips) | n/a | n/a | n/a | that panel's tag rows only | no | yes | open Smart Album with a Tags rule; **a metadata Tags filter ignored** |
| Scan updates (`PhotosUpdated`, in-place `PhotosIndexed`) | yes | **no** (viewer keeps old facts) | yes | **no** | scanner's own | at IndexingFinished | scan's own gate |

#### Decisions (2026-11-26, agent)
- F1: pure `photo_change.rs`: `ChangeKind` (Marks(Membership), Recipes, Rotated, Renamed, Tagged, Trashed, Indexed, Removed) and a `ChangeView` (front, whether the shown photo changed, sidebar destination, sort, search, metadata filters, open Smart Album rules) go in; a `FanOut` comes out: what the shown photo's surfaces do (keep / refresh the front surface / clear), library counts, album counts, view membership (`photo_marks::Membership`). Table tests.
- F2: GTK adapter `photo_change_widgets.rs`, one function `photo_changed(context, PhotoChange)`; `PhotoChange` carries the new values each site already has (mark plan, recipes, rotated/renamed photo, tagged id, trashed ids, scan records, removed ids). It always writes the values to the gallery's objects and tiles and to the viewer's own objects, then applies the `FanOut`. Callers keep their database writes and their error handling; the fan-out runs only after a write succeeded.
- F3: shown photo: Trashed clears the InfoBar and Properties (the viewer closes, as today); every other kind refreshes the InfoBar and the front surface's Properties when the shown photo is among the changed ids; Removed (scan) keeps them (gallery and viewer move their own selection).
- F4: library counts: Marks and Trashed. Scan kinds keep the scanner's batched counts.
- F5: album counts: every kind except Indexed (the scan recounts once at IndexingFinished, as today).
- F6: membership: Marks pass `photo_marks::plan`'s answer through; Trashed rebuilds; Removed removes; Indexed keeps (the scan's own gate decides what reaches the gallery). Recipes, Rotated, Renamed, Tagged rebuild when a field they change is narrowed: a metadata filter with a selection on it, or the open Smart Album's rules naming it (Recipes, Rotated → Edited; Renamed → FileType; Tagged → Tags), plus Recipes in History, plus Renamed under a search (it matches paths) or the Name sort.
- F7: drift dropped: bulk edit rebuilt under *any* metadata filter and always after Reset; now only by F6 (a recipe changes only Edited; History is kept). A bulk edit reports each slice as `Recipes { more_coming: true }` (values and the shown photo, so tiles still change as it runs; no recount, no rebuild), then its end once with no photos (recount and rebuild once).
- F8: out of scope: the scan's append queue and counts batching, collage tile refresh after the editor, the editor's own state, `photo_marks::plan`. The folder favourite (`layout.rs`) keeps its own grid rebuild; its shown-photo re-read (`reload_shown_photo`) now goes through the adapter as `Indexed`. Follow-up: it never recounts Smart Albums (a Favourite rule goes stale until the next recount).

#### Stages
1. Pure `photo_change.rs` with table tests (4, seen red against a stub first). Done: `de2b0515`.
2. Adapter `photo_change_widgets.rs`; marks, editor save, bulk edit, rotate, trash, rename, tags, scan updates and removals moved onto it; `smart_albums_changed`, `show_changed_photo`, `update_counts`, `Gallery::update_edit_recipe` and bulk's `refresh_grid_when_done` deleted. Display tests `a_scan_update_reaches_the_viewer_and_its_properties` and `untagging_under_a_tag_filter_drops_the_photo`, seen red with the adapter put back to the old sites' behaviour. Done: `71b2acd7`. The owner approved adding `photo_change_widgets.rs` as an owner of the `photo-marks` rule (it writes the gallery's marks).
3. Owner rule `photo-change` (`photo_updated` and `update_library_counts` only in the adapter, the InfoBar, the sidebar and the InfoBar responsive tests; allowed: the scan's batched counts in `build.rs` and four test fixtures; two self-test cases), `photo-marks` gains the adapter as owner; GLOSSARY (**Photo change**), AUDIT-INDEX. Done in the commit after `71b2acd7`.

#### Verification so far (2026-11-26)
- `cargo check --locked` and `--tests`: no warnings.
- Unit: `window::photo` (49 passed, includes `photo_change` 4).
- Display (Xvfb, one process each): the two new tests; existing `copy_and_paste_edits_menu_persists_the_recipe_for_export`, `rotating_in_the_viewer_refreshes_the_viewer_properties`, `trashing_photos_updates_library_counts`, `albums_home_keeps_browsing_filters_after_edits_and_recounts`, `marks_change_a_whole_selection_or_nothing`, `marks_recount_smart_albums_once_per_burst`: all pass.
- `scripts/check-presentation-owners.sh`: only the four known `open_files.rs` hits; `--self-test` ok.
- Not covered by tests: the rename dialog, the editor's `saved` callback and the scan event handler in `build()` (their adapter paths are tested).

### 17 · Album list (merged 2026-11-26)
Merged into `main` 2026-11-26 (merge `2d7a5e27`), approved by the owner. Open: the follow-ups below and the pre-existing album view test failure. Next in order: 16.

Started 2026-11-26 at `main` @ `43b42762`. Decisions E1-E8 approved by the owner.

#### Before (at `43b42762`)
- Albums home: 8 writers (album edits through `refresh_album_ui`, the Smart Album recount, a filter change in Albums, opening Albums, `album_theme_changed`, the Settings theme callback, the startup populate, the library switch), reached through `albums_home_refresh_slot` (6 clones) or `albums_view::refresh` directly; only the recount and the filter change guard stale results.
- Sidebar album rows: 6 writers; startup (`sidebar::build`) and the library switch gave it the filtered `browse_albums` list.
- Add to Album: the photo menu and Properties build their list fresh; the InfoBar button's popover was built once and rebuilt only by `refresh_album_ui`.

#### Bugs found (each reproduced by a test seen red first)
1. Startup's sidebar album counts were narrowed by a toolbar rating left saved by the last session (the list was queried before the session reset). `sidebar_album_counts_ignore_a_rating_saved_by_the_last_session`: count `1` for `2`. The library switch passed the same filtered list (code reading).
2. After a library switch the InfoBar's Add to Album still listed the old library's manual albums (album ids are per library). `add_to_album_lists_the_open_librarys_manual_albums`: `["Old library album", …]`.
3. The library switch's Albums home was counted before the switch reset the rating (code reading; same fix as 1).
4. A recount still running when an album was edited applied its older lists after the edit. `an_older_recount_never_overwrites_a_newer_album_edit`: two home applications for one.

#### Decisions (approved 2026-11-26)
- E1: pure `album_list.rs` + GTK adapter `AlbumList` (`album_list_widgets.rs`); `PhotoActionContext.albums` replaces `refresh_albums_home`; the home and the sidebar are attached later.
- E2: change → surfaces: Edited, Appearance, LibraryOpened: sidebar rows + home (if shown), now; PhotosChanged: both, batched worker, skipped without Smart Albums; BrowseChanged: home (if shown), worker; Shown: home, now.
- E3: album edits refresh the album rows only, not the folder tree and counts; folder removal keeps its full sidebar refresh (unfiltered albums) and tells `AlbumList`.
- E4: the InfoBar Add to Album list is built each time it opens (`set_create_popup_func`); manual albums only.
- E5: a hidden home is not refreshed; opening it reloads it.
- E6: one Appearance rule (the Settings theme callback also refreshes the album rows).
- E7: bugs 1, 3, 4 (and 2, through E4) fixed here.
- E8: collection navigation's album order, membership semantics, `manual_albums` and the album views' internals unchanged.

Owner safeguards: a library switch invalidates batched recounts and running results; results counted under older browsing filters are dropped; Albums home layout, sort, Type filter, cover size and position kept; guards kept (ask before changing one).

#### Stages
1. Pure `album_list.rs`: `AlbumChange`, `plan`, `AlbumListState` (per-surface generations, library epoch, batching) with table tests. Done: `b76ad42f` (dead-code warnings until stage 2).
2. `AlbumList` adapter; every writer moves onto it; the library switch hands over with `LibraryOpened` after its rating reset (bug 3); the InfoBar Add to Album is built on open (E4, bug 2, moved here from stage 3 so no commit leaves that menu stale); slot, field, thread-locals, `refresh_album_ui`, `recount_smart_albums` deleted. Bugs 4 and 2 tests red first. Done: `eada23c5`.
3. Startup and library switch give the sidebar whole albums (bug 1, test red first). Done: `8d633499`.
4. Owner rule `album-list` (`albums_view::refresh`, `sidebar::refresh_album_rows` only in `album_list_widgets.rs`; two self-test cases; no existing rule changed), docs.

#### Verification (2026-11-26)
- `cargo check --locked` and `--tests`: no warnings.
- Unit (one filtered run): `window::album_list` (4, new), `window::photo_list`, `window::photo_action_tests`, `window::photo_marks`, `db::tests`, `db::browse_scope`: 96 passed.
- Display (Xvfb, one process each): new `an_older_recount_never_overwrites_a_newer_album_edit`, `add_to_album_lists_the_open_librarys_manual_albums`, `sidebar_album_counts_ignore_a_rating_saved_by_the_last_session`; existing `albums_home_keeps_browsing_filters_after_edits_and_recounts` (now: a home result counted under older filters is dropped after they change), `marks_recount_smart_albums_once_per_burst`, `trashing_photos_updates_library_counts`, `rotating_in_the_viewer_refreshes_the_viewer_properties`, `toolbar_rating_filter_shows_it_is_off_in_ratings`, `save_as_smart_album_copies_the_active_filters`, `a_gallery_shown_now_drops_an_older_refresh`, `smart_albums_share_the_overview_and_its_type_filter`, `album_layout_control_shows_coverflow_and_persists_it`: all pass.
- **Pre-existing, found 2026-11-26:** `albums_view::tests::album_view_modes_preserve_albums_and_click_navigation` fails on `main` @ `43b42762` too (bookshelf photo-count colour, `albums_view.rs:2236`); not investigated.
- `scripts/check-presentation-owners.sh`: only the four known `open_files.rs` hits; `--self-test` ok.
- Not covered: the library switch's album handover itself (it lives in `build()` and needs a second registered library; its pieces are tested: `another_library_drops_running_results_and_waiting_batches`, the Add to Album list after a connection change). Not run: full suite, GUI harness, by-hand check.

#### Follow-ups
- 20: `PhotoActionContext` lost `refresh_albums_home`; `sidebar`, `create_album`, `import_folder`, `delete_album`, `on_unavailable` remain for folder removal's full sidebar refresh.

### 15 · One photo-list pipeline (merged 2026-11-26)
Merged into `main` 2026-11-26 (merge `456325ac`, branch deleted), approved by the owner. Open: the follow-ups below; collection navigation's test gap is recorded under 18. Next in order: 17.

Started 2026-11-26 at `main` @ `0653bd77`, on top of 14's `BrowseScope`. Decisions D1-D6 approved by the owner.

#### The four entry points before (at `0653bd77`)
| Step | Grid refresh | Startup | Library switch | Collection navigation |
|---|---|---|---|---|
| Query | `collection_photos` | `collection_photos` | own `match`, rating forced to All | 12 hand-written copies, search kept per candidate |
| Formats | yes | yes | yes | yes |
| RAW+JPEG pairing filter | yes | no | no | no |
| Gallery pairing mode / pair folders | set (before the worker) | not set | not set | not set |
| Recently Added limit | without a search | yes | yes | some branches, even while searching |
| Files check | yes | yes | yes | yes |
| Order | Folder stream / History kept / sort | same | same | always `sort_photos`; folder pages are one folder |
| Thread, stale results | worker, generation | GTK, startup generation | GTK, `gallery.replace` without a generation bump | GTK, `gallery.replace` without a generation bump |

Intentional and kept: startup and the switch are synchronous; the switch forces the rating to All and clears the search; navigation keeps its own page shapes (single-folder flat pages) and its search scope (candidate's destination plus the search). Omissions: pairing outside the grid refresh (added only there by `9fc96934`); no generation bump before the switch and navigation replace the gallery (suspected stale-result bug, not reproduced). Also: navigation's `Folder(current_folder_id)` arm is dead (an early return), left for 18.

#### Decisions (approved 2026-11-26)
- D1: startup, the library switch and navigation apply RAW+JPEG pairing and set the gallery's pairing state.
- D2: the switch and navigation invalidate outstanding refresh generations before they switch libraries or replace the gallery; a stale worker cannot update photos or pairing state (the grid refresh now sets pairing state with its result, only while current).
- D3: navigation keeps its single-folder flat pages (the Folder stream for them is 18's decision).
- D4: navigation keeps each candidate's destination plus the search.
- D5: the Recently Added limit applies only without a search, everywhere.
- D6: owner rule: the pipeline steps are called only from `photo_list.rs`; tests stay allowed.

Owner requirements: keep navigation fast (its "next non-empty page" checks skip ordering, `photo_list_has_photos`); Smart Album membership and the five round-three fixes kept; GTK stays outside the pipeline.

#### Stages
1. `photo_list.rs`: `PhotoListRequest`, `PhotoList`, `photo_list`, `photo_list_has_photos`; `collection_photos`, `limit_recently_added`, `apply_files_check` moved in. Table tests. Done: `74e2941c` (builds with dead-code warnings until stage 2).
2. Grid refresh and startup use it (`startup_photos` removed); the grid refresh applies pairing state with its result, only while current; startup gives the new gallery its pairing state. Done: `412912bf`.
3. Library switch uses it, invalidates refreshes before swapping the connection and shows its list through `show_photo_list_now` (invalidates again). Red-first test `a_gallery_shown_now_drops_an_older_refresh` (the older refresh replaced the newer list: `[1, 2, 3]` for `[9]`). Done: `26395d9c`.
4. Collection navigation's 12 lists are `photo_list` requests on each candidate page; the "next non-empty album" checks use `photo_list_has_photos` (no ordering). Done: `e48a2955`.
5. Owner rule `photo-list` (`photo_list.rs`, `image_format.rs`; self-test cases for pairing and the Files check by hand), docs.

#### Verification (2026-11-26)
- `cargo check --locked` and `--tests`: no warnings.
- Unit (one filtered run): `window::photo_list` (6, new), `window::photo_action_tests`, `window::photo_marks`, `db::browse_scope`, `db::tests`, `db::filters`: 104 passed. Includes the round-three unit tests (`sidebar_rating_destinations_are_independent_of_toolbar_and_favourites`, `startup_restores_history_in_history_order` now through the pipeline, `scan_appends_new_photos_only_to_an_unfiltered_view`) and the Smart Album rule tests.
- Display (Xvfb, one process each): `a_gallery_shown_now_drops_an_older_refresh`, `toolbar_rating_filter_shows_it_is_off_in_ratings`, `trashing_photos_updates_library_counts`, `rotating_in_the_viewer_refreshes_the_viewer_properties`, `albums_home_keeps_browsing_filters_after_edits_and_recounts`, `rating_filter_refresh_invalidates_previous_folder_membership`, `raw_jpeg_header_stays_in_place_after_search_reveal`, `rapid_folder_navigation_keeps_superseded_destination_hidden`, `folder_navigation_hides_unrestored_destination_paint`: all pass.
- `scripts/check-presentation-owners.sh`: only the four known `open_files.rs` hits; `--self-test` ok.
- Not covered by a test: collection navigation itself (it lives in `build()`; candidate 18 makes it testable). Its request shape is covered by `navigation_pages_are_one_folder_sorted_flat_with_favourites_kept`. Not run: full suite, GUI harness, by-hand check.

#### Follow-ups
- 18: collection navigation out of `build()` (its dead `Folder(current_folder_id)` arm; whether its folder pages should be the Folder stream).
- 19: the scan's progressive appends still decide "does this photo join" outside the pipeline (`admits` from 14's follow-ups).

### 14 · Browse scope (merged 2026-11-26)
Merged into `main` 2026-11-26 (merge `16fa04e7`, branch deleted). The owner approved the revised decisions 4, 6 and 7 and checked the rating picker, facet counts, Smart Albums, library switching and search by hand: working. Follow-ups below stay open. Next in order: 15.

Started 2026-11-26 at `main` @ `8bd8cd7c`. The owner approved the nine decisions below and asked for the agent to decide open questions itself.

#### Decisions
1. The value holds the destination, the search, the toolbar rating and the saved metadata filters. The sort stays out (it orders, it does not select); photo marks keep taking it.
2. It lives in `src/db/browse_scope.rs` with its own `Destination`; the window maps `SidebarFilter` onto it in one tested function (`browse_destination` in `library.rs`). Rules there: a search is library-wide; the Folder stream, Recently Added, Library and All browse all photos; Ratings and Rating(n) carry their own rating.
3. It replaces the facet `filters::Scope`; `facets` and `count_matching` take a `BrowseScope`.
4. **Revised while implementing.** Planned: the rating cell is the truth, the settings row a write-only mirror. The Albums home, album covers (`albums/card.rs`, `coverflow.rs`) and the search suggestions only hold a connection, and wiring the cell into them belongs to 17 and 20. So `BrowseScope::load` is the single reader of the session rating row (`RATING_SETTING`); the toolbar still writes row and cell together, and the cell still serves photo marks and the offline watch.
5. `RatingFilter` moved into db as the typed rating. One rating rule: `effective_rating` (the destination's for Ratings / Rating(n), else the toolbar's), applied once, in SQL. Only Corrupt, Online and Offline are checked on loaded photos (`apply_files_check`), at the pipeline step the old rating pass used.
6. **Revised.** No `admits(photo)`: no caller needs it yet and the metadata fields are not on `db::Photo`. The Files check stays outside db (`apply_files_check`). Candidate 15 can add `admits` when its reducer needs it.
7. **Revised.** Two questions, not two versions of one: `is_narrowed()` (metadata filters or an applied toolbar rating) answers "may a photo of the destination be missing?" (scan appends, the chosen album cover, the scan's final Folder refresh, which now also rebuilds under a toolbar rating). "Could an edit or a scan's metadata update move a shown photo?" stays metadata-only (in-place scan updates, the History/editor teardown refresh `build.rs`, `photo_actions.rs` after a batch edit): those changes never touch the rating, and refreshing the grid on every editor close under a rating filter would be visible churn.
8. Scope: the value, the mapping, the predicate, `is_narrowed`, every db reader taking `&BrowseScope` (`Filters::for_browsing` deleted), facets, album counts, the card cover, the stale checks, collection navigation's reads (it stays in `build()` until 18). `collection_photos` takes the scope; formats, RAW+JPEG, the Recently Added limit and sorting are unchanged (15).
9. Branch `refactor/browse-scope`, one commit per stage; push allowed, merge only on approval.

Owner safeguards (2026-11-26): scan batching kept (one scope per indexed batch); workers get immutable scope snapshots and drop stale results (`BrowseSnapshot`: library path plus scope; the grid refresh keeps its generation counter); Smart Album rules keep their own saved rating string (`AlbumRules::predicate`, `rating_filter`), every saved spelling (`0`, `unrated`, `stars`, `1`-`5`) still matches and the JSON is not rewritten; the five round-three fixes kept; sorting, RAW+JPEG and the pipeline untouched.

#### Stages
1. `BrowseScope`, `Destination`, `RatingFilter`, `BrowseSnapshot` with table tests. Done: `2375d8d8`.
2. Every browsing query takes the scope; callers build it through the mapping. Done: `e6d38ca1`.
3. Docs: this section, GLOSSARY (Browse scope), AUDIT-INDEX. Done: `f415fc5a`.

#### Behaviour now aligned (was drift)
- Facet counts and the photo count use the grid's search (path, date, album name; before: path only) and its destination (History, Ratings and Rating(n) narrow the counts too).
- A library switch lists the new library's photos without a stale saved toolbar rating (the setting is reset right after; before, the list could be filtered by it while the picker showed Clear).
- Searching from Ratings / Rating(n) is library-wide under the toolbar rating, as the search rule says; before it also kept only rated photos.
- Save as Smart Album from a sidebar rating destination saves the rating shown there.

#### Verification (2026-11-26)
- `cargo check --locked` and `--tests`: no warnings. Stage 1 checked alone (`db::browse_scope` 7/7 on the old tree).
- Unit suite: 1072 passed, 0 failed. New: `db::browse_scope` (effective rating, narrowed, Files choices, rating keys, SQL membership per destination/search/rating, metadata parameters, stale snapshots), `browse_scope_maps_every_sidebar_destination`, `saved_smart_album_rating_spellings_still_match`, a saved `"0"` rule in `smart_album_rules_on_marks_rebuild_the_album`. Tests were written with the module, not seen red first.
- Round-three fixes: `sidebar_rating_destinations_are_independent_of_toolbar_and_favourites`, `startup_restores_history_in_history_order`, `scan_appends_new_photos_only_to_an_unfiltered_view` (unit), and in Xvfb `toolbar_rating_filter_shows_it_is_off_in_ratings`, `trashing_photos_updates_library_counts`, `rotating_in_the_viewer_refreshes_the_viewer_properties`, `albums_home_keeps_browsing_filters_after_edits_and_recounts`: pass.
- Xvfb also: `save_as_smart_album_copies_the_active_filters`, `rating_filter_refresh_invalidates_previous_folder_membership`, `metadata_panel_navigation_auto_closes_and_allows_manual_reopen`: pass.
- `scripts/check-presentation-owners.sh`: only the four known `open_files.rs` hits.
- Not run: GUI harness. The owner's by-hand check replaced the planned smoke check.
- Saved for later: `scripts/browse-scope-checks/run.sh` (rating picker, metadata panel count, Ratings under Unrated, Smart Albums saved as `0` and `unrated`, search, library switch with a stale rating; private Xvfb, copy of a GUI-harness library). Not run yet: its AT-SPI selectors may need adjusting on the first run.

#### Follow-ups
- The rating is still held twice (session row for queries, cell for photo marks and the offline watch); one owner when 17/20 give the views a scope.
- `admits(photo)` for 15's reducer.

### 18 · Collection navigation out of `build()` (merged 2026-11-26)
Merged into `main` 2026-11-26 (merge `ecc8e954`), approved by the owner; branch deleted. Checked by hand by the owner 2026-11-26. Open: a library-switch test.

Started 2026-11-26 at `main` @ `5a39d0a7` on `refactor/collection-navigation`. Decisions N1-N8 approved by the owner 2026-11-26 with the additions below; the agent's own choices are marked (agent).

#### Before (at `5a39d0a7`, `build.rs:1254-1820`)
- One ~340-line closure behind Up/Down in the viewer (`lightbox/impl.rs:1246`, `shortcuts.rs:125`); `collection_navigation_slot` written, never read.
- Hard-coded order All → Favourites → Recent → albums → folders; the sidebar shows All → Recent → Favourites → Ratings → History, albums, folders. Ratings, Rating(n) and failed boundaries fell back to the folder next to the current photo's folder, in database order, sometimes favourites-only or Recent-limited while the sidebar said plain Folder.
- Folder pages were single-folder flat lists, and in Folder mode Up/Down was a no-op (the Folder arm dead): folders were a dead end.
- "Show this page" copied 9 times; none saved the last view, synced the rating button's scope or the date-browser button. Backward landing was the last photo at some boundaries, the first at others. Every candidate page was listed in full on the GTK thread to test emptiness.

#### Decisions
- N1: pure `collection_navigation.rs` (the sequence of stops, the next eligible stop, folder rows, landing) with table tests; GTK adapter `CollectionNavigation` (`collection_navigation_widgets.rs`); the `build()` closure, the dead slot and the dead Folder arm deleted.
- N2: the visible sidebar order: library rows as shown (hidden rows skipped; Rating(n) only while Ratings is expanded), then albums as the sidebar lists them (manual and Smart Albums), then the visible folder rows. A collapsed section heading does not remove its rows. (agent) Library home, Albums home and History are not stops (History opens photos in the editor, not the viewer).
- N3: the current photo's neighbouring-folder fallback and its mislabelled filtered folder pages are gone.
- N4: folders are the continuous Folder stream. Landing on a folder shows the stream with the viewer on that folder's first photo (last going back); in Folder mode Up/Down moves to the previous/next visible folder row that has photos, inside the stream the viewer already holds; past the last folder the sequence wraps on. (agent) A photo in a folder whose row is not shown (a collapsed tree, Imported-only mode) belongs to its nearest shown ancestor row.
- N5: forward lands on the first eligible photo, backward on the last, including within folders.
- N6: a stop is eligible when the shared photo-list pipeline lists a photo for it under its own browse scope (`PhotoListRequest::for_view`: toolbar rating except on rating pages, metadata filters, Smart Album rules, format visibility, RAW+JPEG pairing, the Recent limit, the Files check); folder rows from the one stream list. Only the target's list is built.
- N7: navigation does the sidebar click's destination bookkeeping through one shared step (`enter_destination`, extracted from `layout.rs` unchanged: date-browser resume/exit, filter, date-browser button, rating scope sync, section layout, last view saved, sidebar selection), keeps the viewer open, and selects the landing photo; the sidebar click itself is unchanged.
- N8: triggers unchanged (Up/Down in the viewer only).
- Owner additions: wrap at both ends of the eligible sequence; with only one eligible collection nothing happens; the current viewer image stays until the target is ready; stale results are dropped (a newer press, a destination or search change, the viewer closing, a library switch); presses during a navigation are queued and run one after another.
- Search (owner, 2026-11-26): while a search is active Up/Down does nothing (every page shows the same global search results).

#### Stages
1. Pure `collection_navigation.rs`, table tests first (5, seen red against a stub). Done in the stage 1 commit.
2. `CollectionNavigation` adapter on a worker with stale-result guards; `enter_destination` extracted; the `build()` closure deleted (library rows and albums). `sidebar::navigation_destinations` replaces `visible_folder_ids`; `Gallery::select_last_photo` and `SectionedFolderView::scroll_to_index` lost their last caller and are deleted. Display tests `viewer_up_and_down_walk_the_sidebar_collections`, `viewer_up_and_down_drop_stale_and_needless_moves` (the stale test seen red with the destination check removed). Folder rows are not stops until stage 3 (dead-code warnings for the folder helpers until then). Done in the stage 2 commit.
3. Folder rows: the continuous stream, landing within folders, navigation inside Folder mode. The worker lists the stream once when the walk first reaches a folder row (or uses the viewer's photos in Folder mode, with the shown photo's row as the current one); inside the stream only the viewer moves. Display test `viewer_up_and_down_move_through_the_folder_stream`. Done in the stage 3 commit.
4. Owner rule `collection-navigation` (`set_collection_navigation_handler` only in the adapter, which installs itself with `install`; one self-test case). Done in the stage 4 commit. GLOSSARY and AUDIT-INDEX entries added. Still open: a by-hand check; the library-switch drop is guarded (database path) but not tested.

#### Verification (2026-11-26)
- `cargo check --locked` and `--tests`: no warnings.
- Unit: `window::collection_navigation` 5 passed (red against a stub first).
- Display (Xvfb, one process each): `viewer_up_and_down_walk_the_sidebar_collections`, `viewer_up_and_down_drop_stale_and_needless_moves` (red with the destination check removed), `viewer_up_and_down_move_through_the_folder_stream`: pass.
- `scripts/check-presentation-owners.sh`: only the four known `open_files.rs` hits; `--self-test` ok.

**Test coverage gap (recorded 2026-11-26, after 15).** Collection navigation (the `collection_navigation` closure in `build()`, `src/window/build.rs`, wired to the viewer through `Lightbox::set_collection_navigation_handler`) has no test of its own: no unit or display test drives it, and the GUI harness has no step for it. 15 moved its lists onto the shared pipeline; what is tested is only the request shape its pages use (`navigation_pages_are_one_folder_sorted_flat_with_favourites_kept` in `window::photo_list`) and the stale-result guard it calls (`a_gallery_shown_now_drops_an_older_refresh`). Untested behaviour:
- which page comes next in each direction (folders in visible sidebar order, albums in order, the Recently Added ↔ albums ↔ folders boundaries, library views moving by the current photo's folder);
- skipping empty candidates (`photo_list_has_photos`), including under a toolbar rating, metadata filters, a search and RAW+JPEG pairing;
- favourites within a folder page, the Recently Added limit on folder pages;
- what it updates: the sidebar filter and selection, section layout, grouping, gallery, `select_last_photo`, and reopening the viewer on the new page.

Also seen in 15: the `Folder(current_folder_id)` arm is dead (the closure returns early for Folder), and whether its folder pages should be the Folder stream instead of single-folder flat pages is 18's decision (15 decision D3). Moving it out of `build()` into a pure "next page" decision plus a GTK adapter is what makes those testable; write the table tests first.

### 19 · Scan events as a reducer (in progress)
Started 2026-11-26 at `main` @ `b8e3dda8` on `refactor/scan-events`. Decisions Q1-Q5 approved by the owner 2026-11-26; the agent's refinements are marked (agent).

#### Before (at `b8e3dda8`, `build.rs:5697-6417`)
- One ~720-line 50 ms `timeout_add_local` closure in `build()`, no tests: refresh-prepare events, then the `ScanEvent` match (~430 lines), then the sidebar-count throttle (50 photos or 250 ms), gallery appends (192 per tick) and thumbnail refreshes (128 per tick).
- Decided inline: generation reset and isolation, the events let through after Stop, which new photo joins the view (no search, no metadata filter, no toolbar rating, enabled format, view All or the photo's own folder), in-place rereads, the `IndexingFinished` / `Finished` rebuild rules (grid refresh, Folder-stream rebuild, next queued folder, Maintenance no-op), and the completion messages per job kind.

#### Decisions (approved 2026-11-26)
- Q1: a reducer. `ScanView` (`scan_view.rs`, pure) holds the per-generation state (counters, network flag, failure shown, the pending append queue, pending sidebar counts and their throttle, the dirty-thumbnail queue); it returns `ScanEffect` lists the adapter applies in order. Refresh-prepare events stay outside (job startup, not showing a scan). (agent) The reducer takes the job as `&mut ScanJobState`, a plain value, and makes the `Finished` / `Cancelled` job changes itself, instead of returning job effects: the totals it reports are read back in the same step, and effects would repeat the job arithmetic in the adapter.
- Q2: behaviour-preserving. Suspected drift gets its own test seen red, its own commit, and the owner's approval before behaviour changes (see "Suspected bugs" below).
- Q3: the reducer sees the view as plain values, never SQLite or GTK. (agent) Through a small `ScanViewFacts` trait (sidebar filter, search active, browse scope, format enabled, folder exists, recovery deferred) that the adapter answers lazily, so the same queries run at the same moments as today (the browse scope once per indexed batch, the format per new photo) and an idle tick does no extra work. The test fake is the second adapter.
- Q4: `src/window/scan_view.rs` (pure) and `src/window/scan_view_widgets.rs` (GTK adapter); `build()` only wires the timer.
- Q5: `PIC_WATCH` / `PICASA_TRACE` / `SCAN FAILED` / `SCAN COMPLETE` lines keep their exact text and are printed by the adapter; the reducer returns the numbers.
- Owner constraints: timer interval, per-tick event and time caps, batch sizes, throttles, cancellation, generation isolation and trace output unchanged; uses the existing `BrowseScope`, `PhotoChange` and album-count refresh.

#### Suspected bugs (read from the code, not fixed; need approval)
- Subfolder admission: a live append needs `photo.folder_id == id` for `Folder(id)`, but the Folder destination includes subfolders, so a new photo in a subfolder of the open folder appears only at the final rebuild.
- RAW+JPEG pairing: appended photos skip pairing, so in pair mode the hidden half of a pair may show until the next rebuild.

#### Stages
1. Pure `scan_view.rs`, table and event-sequence tests first (seen red against a stub).
2. `scan_view_widgets.rs` adapter; the tick body leaves `build()`.
3. Docs (GLOSSARY, AUDIT-INDEX) and verification.

### Round three (2026-11-26): deferred, not rejected
The owner deferred all seven candidates: the bugs the review found are fixed one at a time instead of starting another refactoring cycle. The candidates stay valid; revisit them in this order when a refactor is wanted: 14 → 15 → 17 → 16 → 18, then 19 and 20.

Bugs found by the review (read from the code; fix individually):
- **Fixed** (candidate 17, on `feature/smart-albums`): after an album edit or a Smart Album recount, the Albums home got unfiltered counts. `refresh_album_ui` and `recount_smart_albums` (`src/window/albums.rs`) now feed it `browse_albums`, and a recount started under older browsing filters is dropped. Test: `albums_home_keeps_browsing_filters_after_edits_and_recounts`.
- **Fixed** (14/15), handoff bug 4, `d3d2252a` on `fix/round-three-bugs`: a restored History session showed all photos; startup's list had no History arm. Startup's list moved into `startup_photos` (`library.rs`), which queries through `collection_photos` (the grid refresh's per-destination query) and keeps History in its own order. Not candidate 15: the library switch still has its own list. Test: `startup_restores_history_in_history_order`.
- **Fixed** (14), handoff bug 1, `18923e3e` on `fix/round-three-bugs`: sidebar Ratings with the toolbar rating on Unrated was empty: SQL kept `rating=0`, then `apply_collection_rating_filter` kept 1-5. The rating destinations now query without the toolbar rating (`collection_photos`, `toolbar_rating_applies` in `library.rs`); the toolbar rating button is insensitive there with a tooltip saying why, and the metadata row's scope drops the rating so its counts match the grid. Tests: `sidebar_rating_destinations_are_independent_of_toolbar_and_favourites` (extended), `toolbar_rating_filter_shows_it_is_off_in_ratings` (display).
- **Fixed** (14/19), handoff bug 5, `ab33310c` on `fix/round-three-bugs`: the scan append path added new photos to a rating-filtered view; its "narrowed?" check ignored the rating. Appending new photos now also needs `scan_may_append_new_photos` (`library.rs`: metadata filters empty and toolbar rating `""`/`all`); in-place updates of shown photos keep the old metadata-only gate, because a Folder refresh with no new photos has no final rebuild. Test: `scan_appends_new_photos_only_to_an_unfiltered_view` (the decision; the scan handler in `build()` that calls it is not under test).
- **Fixed** (16), handoff bug 3, `5d83b50c` on `fix/round-three-bugs`: rotating in the viewer refreshed the gallery Properties surface. Rotation moved from `build()` into `rotate_current_photo` (`photo_actions.rs`), which shows the change through `photo_mark_commands::show_changed_photo`, the same front-based surface choice photo marks use (viewer panel while the viewer is in front). Test: `rotating_in_the_viewer_refreshes_the_viewer_properties` (display).
- **Fixed** (16), handoff bug 2, `24297e1b` on `fix/round-three-bugs`: trash did not update the Library counts. The trash work moved from the dialog response into `trash_photos` (`photo_actions.rs`), which refreshes through `photo_mark_commands::update_counts` (Smart Album + Library counts), also after a part-way failure. Test: `trashing_photos_updates_library_counts` (display; a private `XDG_DATA_HOME` Trash under `target/`).

### What is left (2026-11-26, after 18)
Candidates 14-18 merged; only `main` remains (local and GitHub).
- 18: checked by hand by the owner 2026-11-26: done. Open only: a test for a library switch during navigation.
- 16: the sidebar folder favourite never recounted Smart Albums: fixed, merged 2026-11-26 (it now asks the album list to recount; compile-checked, no test).
- 17: pre-existing failure `album_view_modes_preserve_albums_and_click_navigation` (bookshelf photo-count colour), also on `main`; not investigated.
- 14: `scripts/browse-scope-checks/run.sh` never run.
- Not started (deferred): 19 scan events as a reducer; 20 `PhotoActionContext` shrinks.
Suggested order: the 16 recount fix, then 19 when a refactor is wanted.

### Next (updated 2026-11-26)
Smart Albums merged into `main` (merge `89b7b79f`); it is a feature, not a candidate. Round three (14-20) is deferred. Current work is the round-three bugs, one at a time, on `fix/round-three-bugs`. Instructions: [`docs/development/HANDOFF-round-three-bugs.md`](../development/HANDOFF-round-three-bugs.md). Mark each bug below **Fixed** with its commit as it lands. Progress (2026-11-26): all five fixed (`18923e3e`, `24297e1b`, `5d83b50c`, `d3d2252a`, `ab33310c`), each with a test seen red first. Merged into `main` 2026-11-26 (merge `4db4685f`), followed by `feature/album-links` (merge `e2c4b980`: one Add to Album list and New Album dialog, the photo menu's In Albums links; issues #266, #267). Both branches deleted. Round three (14-20) stays deferred.

### Next (2026-11-24, history)
Done today: 12 (front surface), 13 (front-surface change event), 6 (zoom slider router), 11 (window session state, merge `4f47d674`, merged by the owner); the GUI harness was fixed separately (`467b3ee1`).

Open candidates, in order:
1. ~~5 · One gallery scroll-motion module.~~ Merged 2026-11-24 (`c169225e`). The owner dragged both scrollbars and is happy with them; the gallery scrollbar also got a browser-style handle (`feat/gallery-scrollbar-look`, merge `92693c80`).
2. ~~11's follow-up~~ (library switch viewport, Exit / library-switch check). Done 2026-11-24, see "11 follow-up" under 11.
3. ~~8 · Screensaver style descriptor.~~ Merged 2026-11-24, see its section.
4. ~~7 · Editor preview plan.~~ Closed 2026-11-24 with a note (see its section).
5. ~~The two GUI harness editor checks.~~ Fixed 2026-11-24: the harness clicked Undo, Redo, Reset and the screensaver Start button at positions the header and dialog changes had moved. It now presses them by AT-SPI label (`fix/gui-harness-editor-checks`): `editing` 39/39, and `screensaver` 45/45 (it now reaches playback, the fullscreen window, the transition and both exits).

All candidates from both review rounds are merged or closed. A new review round (`mattpocock-skills:improve-codebase-architecture`) is the next step when the owner wants one. 7 (diagnostic only) can wait, or be closed with a note.

How the owner works with these (from this session):
- Each candidate: investigation, then decisions recorded here, then a branch, staged commits (pure module test-first, GTK adapter, owner rule, docs), then verification, merge with a merge commit, push.
- Verification each time: unit and display tests (Xvfb, one process each), `cargo check`, owner script and `--self-test`, GUI harness `viewer` and `editing` (`scripts/test-gui.sh`), and the scripted interaction checks `scripts/front-surface-checks/run.sh` (`cargo run`, private display, copy of a harness library) on the branch and on `main` (`PIC_CHECK_REPO=<main worktree>`).
- The owner has allowed deciding open questions autonomously and merging and pushing on success; say what was decided and why. On 2026-11-23 the agent's `git merge` into `main` was refused by the auto-mode permission check; on 2026-11-24 it worked after the owner said "merge push". Ask when unsure.
- Pre-existing, not caused by today's work: two metadata panel display tests fail on `main` too (see 13); 3 dead-code warnings in `db/photos.rs` and `grid/honeycomb_raster.rs`.

### 7 · Editor preview plan (closed 2026-11-24)
**Closed by the owner's choice (2026-11-24)**, who took 11's follow-up and 8 instead. Nothing on screen depends on it: the wrong reason only mislabels the `PIC_EDIT_RENDERER renderer=cpu_fallback` trace. Reopen if that trace misleads a diagnosis, or if the renderer choice (`editor.rs:1270`) and its hand-built copy (`:1529`, `has_source_for`) drift; the facts below still hold at `dece41f2`.

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
