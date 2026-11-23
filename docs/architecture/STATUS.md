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
| 4 | InfoBar decides what it shows | Strong | open: approach proposed, not started | none | `src/infobar.rs`; callers in `src/window/build.rs`, `src/window/layout.rs`, `src/window/photo_actions.rs` |
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
The approach was proposed on 2026-11-23 and is waiting on three decisions:
1. Whether the InfoBar watching the shown photo's `notify` signals (replacing about 12 manual `set_photo` refresh calls) is in scope or a follow-up.
2. Whether to leave the hand-synced `selected_photo` cell alone. The recommendation is to leave it.
3. The branch, proposed as `refactor/infobar-presentation`.

Problem summary: six call sites each choose between the photo, multi-selection and empty states and must call `set_selected_photos`, `show_photo_metadata` and `set_photo` in the right order. The AUDIT-INDEX has no InfoBar entry yet; add one when this candidate lands.

### 5-8
Not started. The problem statements are in the review report.
