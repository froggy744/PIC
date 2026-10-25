# Cover Flow and Reflective Bookshelf Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an iTunes style Cover Flow to the Albums index and screensaver, and an iPhoto '08 inspired reflective treatment to the album bookshelf.

**Architecture:** Share the Cover Flow geometry and GSK cover/reflection painter, but keep album navigation and screensaver playback in their existing owners. The Albums index supplies album cover nodes and actions; the screensaver supplies photo textures through its current bounded decode pipeline. Bookshelf reflections reuse the cover painter with theme-specific intensity.

**Tech Stack:** Rust 2021, GTK4/libadwaita, GSK snapshots, existing SQLite settings and thumbnail/decode services. No new dependency planned.

**Spec:** `docs/superpowers/specs/2026-10-25-coverflow-reflective-bookshelf-design.md`

## Global Constraints

- Work in `/home/peet/PIC`; preserve the unrelated untracked `audit-themes.md`.
- Do not switch branches, commit, merge, push, or reset without explicit authorization from the user or a later instruction. The task steps below omit commits for that reason.
- Keep current album grid, bookshelf, cover themes, existing saved appearance choices, and screensaver styles working.
- Decode photos off the GTK thread; use bounded caches and the existing screensaver cancellation behavior.
- Make motion optional under reduced motion and maintain keyboard focus and readable labels.

## Review Focus

1. **Zero and one item:** show an empty explanation or one stable centered item, with no out-of-range navigation. Task 2 and Task 4 tests.
2. **Large libraries:** materialize only nearby visual items and avoid per-frame decoding or bitmap reflection generation. Task 1 and Task 2 tests.
3. **Missing or broken cover/photo:** use a placeholder and keep navigation/automatic advance alive. Task 2 and Task 4 tests.
4. **Appearance changes:** switching light/dark and bookshelf assets updates reflection/shadow without stale cached art. Task 3 test.
5. **Screensaver close during remote decode:** cancel pending work and release the window without a retained reference. Task 4 regression test.

---

## File map

| File | Responsibility |
|------|----------------|
| `src/coverflow.rs` | Pure focus/position geometry and shared GSK cover, shadow, reflection drawing contract. |
| `src/main.rs` | Register the new module. |
| `src/albums/coverflow.rs` | Albums index adapter: cover nodes, navigation, focus, album activation, and bounded visible set. |
| `src/albums_view.rs`, `src/albums/view.rs` | Include and host the new layout, preserve sort and refresh behavior. |
| `src/albums/bookshelf.rs`, `src/css/components/albums.css` | Reflective bookshelf theme metadata and visual treatment. |
| `src/settings.rs`, `src/albums/actions.rs` | Persist layout choice and expose album presentation controls. |
| `src/screensaver.rs`, `src/screensaver/gpu.rs`, `src/screensaver/gpu/coverflow.rs` | Style choice, player mapping, and fullscreen photo Cover Flow. |
| `docs/THEMES.md`, `docs/guides/library.md`, `images/bookshelf-row-template.md` | User and theme author guidance. |

### Task 1: Shared geometry and rendering proof

**Files:** Create `src/coverflow.rs`; modify `src/main.rs`.

**Interfaces:** Define `CoverflowPose { x, y, width, height, yaw, depth, opacity }`, `coverflow_pose(index, focus, count, viewport) -> Option<CoverflowPose>`, and a painter accepting a `gsk::RenderNode`, pose, reflection strength, and `gtk::Snapshot`. Keep geometry independent of GTK state so both callers use the same ordering and bounds.

- [ ] Write pure tests for center, adjacent items, depth order, clamping, zero/one item, narrow viewport, and the maximum visible radius.
- [ ] Run `cargo test --locked coverflow::tests -- --test-threads=1`; confirm the new tests fail before implementation.
- [ ] Implement the geometry and a minimal GSK proof that draws a cover and its vertically mirrored, fading reflection. Reuse a render node per item; do not rasterize a reflection on every frame. If GTK's snapshot API cannot provide the mask/transform directly, use one cached cover texture per visible item and document the fallback.
- [ ] Verify with the targeted tests and one ignored GTK snapshot test under `xvfb-run -a env GSK_RENDERER=cairo`; inspect its output for clipping and a visibly fading reflection.

### Task 2: Albums index Cover Flow

**Files:** Create `src/albums/coverflow.rs`; modify `src/albums_view.rs`, `src/albums/view.rs`, `src/settings.rs`, `src/albums/actions.rs` and focused inline tests in `src/albums_view.rs`/`src/settings.rs`.

**Interfaces:** Add `AlbumLayout { Grid, Bookshelf, CoverFlow }` with a distinct persisted key; keep `AlbumAppearance` cover/frame and shelf choices as independent styling state. `AlbumCoverflow` receives sorted `Album` records, cover render nodes, and `Rc<dyn Fn(i64)>` for opening an album.

- [ ] Add failing tests for layout persistence, existing setting migration/default, sort order, focused album after refresh, empty/single album, and no more than the visible neighbor window materialized.
- [ ] Implement the layout selector in the Albums header and context menu, then connect it to the existing `refresh`/`populate` path. Reuse `album_cover_photo`, frame selection, and thumbnail loading from `src/albums/card.rs`; keep cover generation lazy around focus.
- [ ] Add arrow/Home/End, wheel/touchpad, click-to-focus, click/Enter-to-open, focus outline, and accessible album name/count for the focused item. Preserve focus by album ID across sort and refresh, falling back to the nearest surviving item.
- [ ] Run `cargo test --locked albums_view::tests -- --test-threads=1` and the focused settings tests. Add one ignored GTK interaction test that navigates/open albums in a private Xvfb display.

### Task 3: Theme-aware reflective bookshelf

**Files:** Modify `src/albums/bookshelf.rs`, `src/albums/view.rs`, `src/css/components/albums.css`, one suitable shipped `images/theme/bookshelf/<theme>/theme.conf`, `images/bookshelf-row-template.md`, and focused inline `src/albums_view.rs` tests.

**Interfaces:** Extend `BookshelfTheme` with optional `reflection_strength` from `theme.conf` (bounded `0.0..=1.0`, default `0.0` for legacy or unsuitable art). Apply the shared reflection painter to the visible cover only; shelf background remains untouched. Add soft contact shadows derived from the active style/theme color.

- [ ] Add failing parser/geometry tests for absent, invalid, and bounded reflection values, plus a visual snapshot test for at least one dark and one light shelf theme.
- [ ] Implement a modest reflection where shelf art leaves room; prevent overlap with the board front and text. Enable it in at least one suitable shipped row theme. Use the theme setting and current style colors to tune alpha/shadow; do not bake a fixed dark gradient into album art.
- [ ] Verify theme switching and background cycling update the effect immediately, then run targeted `albums_view::tests` under the required single test thread.

### Task 4: Screensaver photo Cover Flow

**Files:** Create `src/screensaver/gpu/coverflow.rs`; modify `src/screensaver.rs`, `src/screensaver/gpu.rs` and focused inline screensaver tests.

**Interfaces:** Add `Style::CoverFlow` with persisted key `coverflow` and label `Cover Flow`. The screensaver stage uses the shared painter with `Player` photo textures, while the existing source, shuffle, portrait filtering, prefetch, duration, caption, clock, pause, and cancellation paths stay authoritative.

- [ ] Add failing tests for preference round-trip, style availability, zero/one/two photo behavior, automatic focus advance/wrap, skipped bad photo, caption ID, and bounded retained textures.
- [ ] Add the style to the settings picker and stage dispatch. Reuse the existing worker and frame clock; request only nearby photos and release textures outside the active window. Make click/keyboard exit behavior match current screensaver conventions and disable sweeping animation under reduced motion.
- [ ] Run `cargo test --locked screensaver::tests -- --test-threads=1` and a focused ignored Xvfb GUI test. Use `PIC_GUI_SUITE=screensaver scripts/test-gui.sh` once if the targeted test cannot cover window teardown or remote decode cancellation.

### Task 5: Visual polish and documentation

**Files:** Modify `docs/THEMES.md`, `docs/guides/library.md`, `images/bookshelf-row-template.md`, and any directly affected theme `theme.conf` files.

- [ ] Compare screenshots in light/dark app themes at narrow and wide window sizes, with plain and framed covers, and on both Cairo and the normal renderer. Tune only constants or theme metadata needed for legibility, smooth movement, and shelf alignment.
- [ ] Document layout controls, screensaver source behavior, reduced motion behavior, and `reflection_strength` for shelf theme authors.
- [ ] Run `cargo fmt --all -- --check`, `cargo check --locked`, and only the targeted tests affected by final tuning. Report tested renderers and any remaining visual limitations.

## Completion criteria

- Albums can switch among grid, bookshelf, and Cover Flow without losing their existing saved theme choices; keyboard and pointer users can open any album.
- The screensaver Cover Flow shows individual photos from the chosen source, advances automatically, and closes cleanly.
- Reflections fade naturally, shelf shadows follow the chosen theme, and no missing asset or large library causes an unbounded UI or decode workload.
