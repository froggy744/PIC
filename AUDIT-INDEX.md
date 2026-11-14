# PIC Codebase Audit Index

**Purpose:** navigation map for PIC maintainers and coding agents: find a feature, understand its dependencies, then identify focused validation before editing.

| Audit metadata | Value |
|---|---|
| Verified commit | `3f57bf31adad7c2f4587b28c87fe9dbb949f63c1` (branch `main`) |
| Verification date | 2026-11-14 |
| Working tree at verification | Not clean: 26 modified + 6 new untracked source files (Honeycomb: `grid/honeycomb*.rs`, `settings/honeycomb.rs`; Immersive: `window/immersive*.rs`). This index was verified against the working tree, which includes that in-progress work. |
| Authority | Source code and `AGENTS.md` override this index |

**Start here:** use the [Quick-Edit Guide](#6-quick-edit-guide-task---files) for entry points, the [Feature impact and validation matrix](#9-feature-impact-and-validation-matrix) for regression risks, and the [Agent navigation protocol](#10-agent-navigation-protocol) before modifications. Search for symbols in the actual checkout; line counts are exact at the verification date but drift.

`AGENTS.md` defines repository rules, permissions, testing expectations, and Git workflow. This document maps implementation, not policy.

---

## 1. Repo layout (root)

| Path | What it is |
|---|---|
| `src/` | All Rust source (single crate `pic-rs`, ~151k lines). All tests inline (`#[cfg(test)]`); 1,136 `#[test]` fns (216 `#[ignore]`d) at verification. No `tests/` dir for Rust. |
| `Cargo.toml` / `Cargo.lock` | Single binary crate, edition 2021. |
| `build.rs` | Requires `libsmbclient-dev` + `libnfs-dev`; compiles `native/*.c`; stages `themes/` + `images/` into `target/<profile>`. |
| `.cargo/config.toml` | Forces clang + mold linker. |
| `native/private_smb.c`, `native/private_nfs.c` | C shims for userspace SMB/NFS, statically linked (symbols hidden). |
| `resources/` | `icons.gresource` (bundled icon theme), `icons/`, `custom-icons/`, Flatpak metainfo. |
| `themes/<id>/` | Runtime-discovered themes, 27 folders (see `docs/THEMES.md`). |
| `images/` | Bundled art (onboarding, bookshelf row template, album covers). |
| `scripts/` | GUI test harness (`test-gui.sh`, `test-gui-native.sh`, `test-gui-wayland.sh`), packaging (`PIC-build-linux-one-script.sh`), native/share/theme tests. |
| `tests/compact_sidebar_x11_drag.py` | Standalone X11 XTest smoke test for the sidebar divider (small GTK repro, not the app). |
| `.github/workflows/` | 9 path-scoped workflows (see Section 7). |
| `docs/` | Feature docs (`THEMES.md`, `GALLERY_V2.md`, `metadata-filters.md`, `experimental-live-gpu.md`, `livegpu-recovery-handoff.md`, `guides/`, `development/`, ...). |
| `errors/`, `examples/`, `screenshots/`, `logo/`, `icon/`, `reports/` | Artifacts/samples, not code. |
| `audit-themes.md` | Separate theme-specific audit document. |

## 2. Startup flow

`src/main.rs` (263 lines) is the only entrypoint:
1. `startup_trace::start()`; in debug builds `PIC_EXPORT_BENCH_ALBUM` runs `edit::export_benchmark::run` and exits before GTK init.
2. `adw::init()`, `register_bundled_icons()` (`resources/icons.gresource`), `register_scrapbook_fonts()` (Linux only, dlopen libfontconfig, OFL fonts from `themes/scrapbook/fonts`).
3. `db::initialize_library_manager()` (from `db/libraries.rs`, included into `db.rs`) picks the library.
4. `db::open(&library.path)` (from `db/core.rs`); existing libraries get `db::automatic_backup_if_due` on a background thread.
5. `window::build(application, connection)` (defined in `window/build.rs`, included into `window.rs`) builds the whole UI; `window.present()`.
6. DB open failure -> `adw::AlertDialog` recovery window (choose/create library) parented to a tiny hidden `adw::ApplicationWindow`.

## 3. Map: top-level `src/*.rs`

| File | Lines | What it does | Links to |
|---|---|---|---|
| `main.rs` | 263 | Entry point; adw app, icon/font registration, library open + recovery dialog. | `db`, `window`, `startup_trace`, `css` |
| `sidebar.rs` | 4608 | **The sidebar.** `SidebarFilter` enum, `SidebarVisibility`, `sidebar::build()`, refresh/count/row functions, folder tree, pin/hover. | `db` (Folder/Album/SidebarCounts); called by `window/build.rs`; `sidebar-*-visible` keys via `db/settings.rs` |
| `albums_view.rs` | 2930 | Album-browse views (incl. Cover Flow album wall); re-exports `albums/` helpers. | `albums/`, `coverflow.rs`, `db` |
| `app_paths.rs` | 561 | Storage locations (`~/.local/share/pic-rs` etc.) + legacy folder-name migration. | used by `db`, thumbnail cache |
| `coverflow.rs` | 862 | OpenGL perspective Cover Flow (dlopen GL); shared by albums and screensaver. | `albums/`, `screensaver/` |
| `diagnostics.rs` | 13 | RSS MiB reader (`/proc/self/statm`). | |
| `image_format.rs` | 347 | Format registry (JPEG/HEIC/RAW/SVG...), per-format enable settings, RAW+JPEG pair mode. | `scanner`, `thumbnail`, `db` settings |
| `infobar.rs` | 1746 | `InfoBar` struct: bottom info/toolbar (photo metadata, ratings, photo actions row). | `window/build.rs`, `window/albums.rs`, `window/photo_actions.rs` |
| `infobar_responsive_tests.rs` | 506 | Responsive tests for the InfoBar. | `infobar` |
| `library_home.rs` | 824 | `LibraryHome` dashboard (Home cards: recent, favorites, albums, scrapbook entry). | `db/library_home.rs`, `library_home/scrapbook.rs`, `sidebar::SidebarFilter` |
| `library_home_tests.rs` | 654 | Home dashboard tests. | `library_home` |
| `metadata_index.rs` | 213 | Cancellable metadata-only backfill (off GTK thread). | `db`, `scanner` |
| `nav_transition_trace.rs` | 27 | PIC_NAV trace helper for navigation transitions. | |
| `network_picker.rs` | 378 | Direct-share import picker UI (Linux-only, `#[cfg(target_os = "linux")]`). | `network_shares` |
| `network_shares.rs` | 517 | Userspace SMB+NFS facade (`visit_scan`, `Entry`); no kernel/GIO mounts. (Linux-only.) | `smb_transport`, `private_smb`, `private_nfs` |
| `onboarding.rs` | 92 | Onboarding state machine (Welcome/Importing/Tips/Complete) + prefs in settings table. | `onboarding/`, `window/onboarding.rs` |
| `photo_object.rs` | 399 | `PhotoObject` (glib subclass; NOT Send/Sync) - the runtime photo row used by grid/lightbox. | `db::Photo`; used by `grid`, `lightbox`, `collage` |
| `photo_texture.rs` | 103 | Edited-thumbnail paintable helper (`edited_thumbnail`, path+rotation+recipe keyed). | `edit::render`, `thumbnail` |
| `private_nfs.rs` | 576 | Rust side of the userspace NFS client (dlopen libnfs). (Linux-only.) | `native/private_nfs.c`, `network_shares` |
| `private_smb.rs` | 309 | Rust side of the userspace SMB client (dlopen libsmbclient). (Linux-only.) | `native/private_smb.c`, `smb_transport` |
| `save_dialog.rs` | 92 | Shared Save picker for photo/collage export. | `edit/export_batch`, `collage` |
| `sd_import.rs` | 1199 | Camera/SD-card import (mount detect, preview, blake3 dedupe, copy). | `scanner`, `db` |
| `smb_transport.rs` | 1639 | Runtime-dlopened libsmbclient transport (SMB URI parsing, browsing, download). (Linux-only.) | `network_shares`, `private_smb` |
| `smooth_scroll.rs` | 274 | Critically-damped spring wheel scrolling (`attach` to scrollers). | attached from `window/build.rs` |
| `source.rs` | 862 | Viewer read lanes: `ViewerReadContext`/read gate, viewer source cache, folder/file availability cache. | `lightbox`, `thumbnail` |
| `startup_trace.rs` | 31 | `PIC_STARTUP_TRACE` markers. | |
| `thumbnail_display.rs` | 776 | Display-worker pool: decodes thumbnails on background threads, hands paintables to GTK (`DisplayRequest`/`DisplayCompletion`). | `thumbnail`, `grid` |
| `thumbnail_trace.rs` | 39 | `PICASA_TRACE` thumbnail markers. | |
| `scroll_trace.rs` | 25 | Scroll tracing helpers. | |
| `window.rs` | 898 | Window module root: filter<->gallery wiring, sort/group/last-view settings, `PhotoActionContext`, filter persistence; `include!`s `operation_progress`, `build`, `startup`, `search`, `availability`, `albums`, `photo_actions`, `library`, `print`; declares `metadata_panel`, `date_browser`, `onboarding`, `photo_layout`, `layout_settle`, `theme`, and the responsive/immersive test modules. | everything |

## 4. Map: module directories

### `src/window/` - main window assembly (the hub)

| File | Lines | What it does | Links to |
|---|---|---|---|
| `build.rs` | 7470 | `window::build()` - assembles the entire main window: sidebar+gallery split, header, InfoBar, export dialogs, scan/import wiring, lightbox hookup. **Biggest file in the app.** `include!`s `dialogs`, `navigation`, `shortcuts` (in-fn), `layout`, `toolbar`, `immersive`. | everything |
| `layout.rs` | 3087 | Gallery/window layout management: split positions, panel sizing, responsive rules, immersive slide/normal widths. Included from a `window::build` fn (returns the layout widget tuple). | `window/build.rs`, `grid` |
| `photo_actions.rs` | 3019 | Photo context menus, bulk edit-recipe ops, trash/favorite/rate actions. | `infobar`, `db`, `edit`, `window/albums.rs` |
| `metadata_panel.rs` | 2226 | Right-side metadata/facet explorer panel; queries run off-thread. | `db/filters.rs` |
| `library.rs` | 1563 | Folder navigation, grid refresh, viewport anchoring per `SidebarFilter`. | `sidebar`, `grid`, `db` |
| `print.rs` | 1308 | Print dialog + page layout (size presets, extras, oriented paper). | `edit::render` |
| `navigation.rs` | 936 | Navigation helpers: folder wheel, sidebar crossfade, destination planning, search focus. | `sidebar`, `grid` |
| `immersive.rs` | 816 | Immersive (chrome-hidden) mode; **`include!`d inside `build.rs`** after the live widgets exist. Presentation-only: chrome edges are `gtk::Revealer` overlays over the gallery, so revealing never reflows Honeycomb/Photo Wall cells. State cells (`immersive_active`, `immersive_sidebar_shown`, toggle/reveal slots) live in `build.rs`. | `window/build.rs` |
| `onboarding.rs` | 684 | In-app onboarding UI + `ImportTracker` wiring. | `onboarding/`, `scanner` |
| `operation_progress.rs` | 501 | `OperationProgressUi` - status row (scan/export/import progress). | `window/build.rs` |
| `availability.rs` | 331 | Folder availability probes, unmount dialogs, thumbnail recovery triggers. | `source.rs`, `thumbnail`, `db` |
| `albums.rs` | 322 | Album UI actions from the window (create/delete/chooser, cover). | `db/albums.rs`, `infobar` |
| `date_browser.rs` | 406 | Year/month date browser strip. | `grid`, `db` |
| `photo_layout.rs` | 339 | Per-section layout persistence (view layouts saved by filter; includes Honeycomb). | `sidebar::SidebarFilter` |
| `search.rs` | 387 | Search entry wiring + folder/photo suggestion popovers. | `db/filters.rs` |
| `theme.rs` | 834 | Appearance theme engine: loads `themes/<id>/` CSS, corner style, applies to widgets. | `css/theme_discovery.rs`, `settings` |
| `toolbar.rs` | 644 | Header toolbar contents (returned tuple from `window::build`). | `window/build.rs` |
| `dialogs.rs` | 110 | Folder statistics dialog, close confirmation. | `db` |
| `layout_settle.rs` | 135 | Coalesces burst width changes (e.g. animated sidebar) into one layout update. | `window/layout.rs` |
| `startup.rs` | 113 | `populate_startup_gallery` - progressive startup gallery fill. | `grid` |
| `shortcuts.rs` | 156 | Keyboard shortcut controller (included inside `build.rs`). | `window/build.rs` |
| `onboarding/tests.rs` | 1124 | Onboarding test module. | |
| `immersive_tests.rs` | 557 | Immersive-mode tests (declared in `window.rs` as `immersive_window_tests` via `#[path]`). | `immersive.rs` |
| `responsive_tests.rs` | 686 | Responsive split tests (declared in `window.rs` as `responsive_window_tests` via `#[path]`). | |

### `src/grid/` - gallery (root `grid.rs` + `include!` composition)

`src/grid.rs` (1050) is the module root: tile size defaults, `ZOOM_LEVELS` ladder (perceptual zoom steps), `PICASA_LEGACY_FOLDER_LIST` switches, first-visible-thumb trace. It **`include!`s** `grid/tile.rs`, `grid/grouping.rs`, `grid/sectioned_folder.rs`, `grid/photo_wall.rs`, `grid/virtualization.rs`, `grid/view.rs`, `grid/navigation.rs`, `grid/selection.rs` (all become items of the flat `grid` module) and `mod`-declares `honeycomb`, `honeycomb_raster`, `honeycomb_surface`, `masonry_layout`, `metadata_index`, `photo_wall_layout`, plus test modules.

| File | Lines | What it does |
|---|---|---|
| `view.rs` | 1532 | `Gallery` struct - the main gallery widget (zoom anchors, layout transitions). |
| `virtualization.rs` | 3336 | Folder stream virtualization: chunked loading, row model, scroll extent. |
| `sectioned_folder.rs` | 3170 | Sectioned (date-grouped) folder rendering (default `GtkGridView` path) + reflow snapshots. |
| `selection.rs` | 2561 | Multi-selection, keyboard navigation across folders, selection positions. |
| `tile.rs` | 2374 | `SquareTile` photo tile widget, paintable cache, scrub/zoom behaviors, Honeycomb mask application. |
| `photo_wall.rs` | 1020 | `PhotoLayout` enum (**Wall/Masonry/Honeycomb**) state, anchor restore, sidebar-transition freeze. |
| `photo_wall_tests.rs` | 3953 | `PhotoLayout` tests (incl. Honeycomb). |
| `photo_wall_layout.rs` | 1419 | Masonry/photo-wall/Honeycomb geometry (integer column widths, anchors). |
| `honeycomb_raster.rs` | 1225 | Honeycomb hex-mask rasterization workers + budgeted cache (4 workers, 64 MB cache / 96 MB pending). |
| `honeycomb.rs` | 124 | Pointy-top hexagon math: fitted bounds, vertices, point-in-hex. |
| `honeycomb_surface.rs` | 68 | `PicHoneycombClip` widget: width-constrained clip around the native `GtkFixed` photo surface. |
| `grouping.rs` | 551 | `GroupMode`/`GroupDate` - date grouping ranges + group headers. |
| `navigation.rs` | 259 | Scroll restore, row heights, realized-row tracking. |
| `metadata_index.rs` | 99 | Grid-side metadata index view. |
| `masonry_layout.rs` | 233 | Masonry layout math. |
| `history_tests.rs` | 480 | History view tests. |
| `grid.rs` | 528 | **Orphan file** - tracked but referenced by nothing (not compiled); do not treat as live code. |

### `src/edit/` - non-destructive editor (`mod.rs` style)

`mod.rs` (15) declares `auto_develop`, `export_batch`, `export_benchmark` (debug-only), `filters`, `live_gpu`, `model`, `overlays`, `render`, `text_render` and `mod editor`; exports `EditRecipe` and `build_editor`. `editor.rs` **`include!`s** `preview`, `zoom`, `crop`, `controls`, `undo`, `export`, `overlays_ui`, `draw_ui`, `text_ui` (all become items of the `editor` module).

| File | Lines | What it does |
|---|---|---|
| `model.rs` | 2202 | `EditRecipe` - the recipe data model (crop, straighten, overlays, text, drawings, filters). The non-destructive unit of truth, stored in `photos.edit_recipe`. |
| `render.rs` | 2363 | **The rendering chokepoint.** `apply_recipe` / `apply_geometry` / `render_thumbnail` / `render_for_viewer` / `render_for_export` - every CPU surface (thumb, lightbox, export, print, collage) renders through here. Add rendering here. |
| `editor.rs` | 3560 | `EditEditor` - the editor window UI: canvas, pan/zoom, tool wiring, live preview. |
| `live_gpu.rs` | 5960 | GPU live-preview renderer: `EditRendererMode` (env `PIC_EDIT_RENDERER` = auto/gpu/cpu), auto GPU decision with retry, shaders on a `gtk::GLArea` (`LiveGpuCanvas`), CPU `Picture` kept behind the GLArea as fallback. Largest editor file. Doc: `docs/experimental-live-gpu.md`. |
| `overlays_ui.rs` | 1974 | Overlay layer UI: drag/resize/move, layer selection. |
| `export_batch.rs` | 1865 | Batch export: `ExportFormat`, unique naming, per-file export on `std::thread`. |
| `overlays.rs` | 1059 | Overlay asset loading/scaling cache, `PreparedOverlay`. |
| `filters.rs` | 968 | `FilterPreset` enum + CPU filter implementations. |
| `draw_ui.rs` | 1198 | Drawing tools panel UI. |
| `text_ui.rs` | 1040 | Text layer panel UI (position grid, fonts). |
| `auto_develop.rs` | 668 | Auto-correct: region analysis, challenge scoring, auto plan. |
| `controls.rs` | 622 | Sliders/filter-tile controls, filter thumbnail rendering. |
| `export_benchmark.rs` | 658 | `PIC_EXPORT_BENCH_ALBUM` resize-method benchmark (debug builds). |
| `preview.rs` | 574 | Preview base caching for the editor. |
| `zoom.rs` | 304 | Zoom fit/1:1 math for the canvas. |
| `export_metadata.rs` | 257 | EXIF rebuild (orientation-normalised) for exports. |
| `text_render.rs` | 246 | Pango text measurement/render for text layers. |
| `crop.rs` | 220 | Crop overlay + aspect presets. |
| `undo.rs` | 46 | History/undo button wiring. |
| `export.rs` | 22 | Export action hookup. |

### `src/lightbox/` - full-screen viewer (root `lightbox.rs` + `include!`)

`src/lightbox.rs` (474) holds the `Lightbox` struct, the decode counting semaphore/permits, viewer request queue + leases, zoom state. It `include!`s `lightbox/impl.rs`, `render.rs`, `offline.rs`, `tests.rs`; `mod opening_transition;` and `mod offline_tests;`.

| File | Lines | What it does |
|---|---|---|
| `impl.rs` | 2936 | Lightbox implementation: navigation, gestures, opening transition glue. |
| `render.rs` | 2209 | Viewer decode/render: zoom textures, `show_photo`, opening decode results. |
| `tests.rs` | 1313 | Viewer tests. |
| `offline_tests.rs` | 541 | Offline-mode tests. |
| `opening_transition.rs` | 331 | Grid->lightbox open transition paintables. |
| `offline.rs` | 276 | Offline (unavailable folder) presentation from cache. |

### `src/thumbnail/` - thumbnail pipeline (root `thumbnail.rs` + `include!`)

`src/thumbnail.rs` (899): quality settings, priority queue, worker orchestration. It `include!`s `cache`, `maintenance`, `recovery`, `viewer`, `nef`, `dng`, `decoders`, `batch`; `mod color`, `mod nef_scene`, `mod raw_quality`.

| File | Lines | What it does |
|---|---|---|
| `viewer.rs` | 1945 | Decode entry points per format (JPEG/HEIC/RAW/SVG dispatch). |
| `color.rs` | 655 | Embedded ICC validation + RGB->sRGB conversion (liblcms). |
| `nef.rs` | 604 | NEF embedded/uncompressed thumbnail extraction. |
| `raw_quality.rs` | 588 | RAW decode quality options. |
| `cache.rs` | 583 | Disk cache paths, sharding, generation guard. |
| `maintenance.rs` | 550 | Cache stats/cleanup vs DB. |
| `decoders.rs` | 546 | TurboJPEG target-size decoders + generic decode. |
| `nef_scene.rs` | 480 | NEF as-shot calibration (camera matrices, illuminant). |
| `dng.rs` | 477 | DNG embedded preview extraction. |
| `texture.rs` | 239 | **Orphan file** (decoded-texture LRU, `PIC_THUMB_TEXTURE_CACHE_MB`) - tracked but referenced by nothing; not compiled. |
| `batch.rs` | 195 | Bulk cancellable thumbnail creation. |
| `recovery.rs` | 165 | Missing-cached-preview recovery. |

### `src/db/` - SQLite (root `db.rs` + `include!`)

`src/db.rs` (331): full SCHEMA string, row structs (`Photo`, `Folder`, `Album`, `SidebarCounts`, `LibraryCounts`), exclusion helpers. It `include!`s `core`, `libraries`, `albums`, `photos`, `history`, `library_home`, `scrapbook`, `settings`, `overlay_assets`, `tests`, `path_migration`, `network_import`; `pub mod filters; pub mod metadata;`.

| File | Lines | What it does |
|---|---|---|
| `photos.rs` | 979 | Photo/folder inserts, search, raw+JPEG pairing. |
| `filters.rs` | 840 | Metadata filter model: `Field`, `Selection`, `Filters`, `Scope`, facets, counts. |
| `libraries.rs` | 738 | Multi-library registry, create/open/backup (`automatic_backup_if_due`). |
| `tests.rs` | 1423 | DB test helpers + tests. |
| `network_import.rs` | 506 | Network import job state (resumable, pause, catalog). |
| `core.rs` | 347 | `database_path`, `open`/`open_existing`, connection config, legacy schema migration. |
| `migration.rs` | 345 | **Orphan file** (GVfs FUSE-path -> `smb://`/`nfs://` rewrite) - tracked but referenced by nothing; not compiled. |
| `settings.rs` | 237 | Generic settings table + `library_counts`/`sidebar_counts`. |
| `history.rs` | 223 | Edit history, collage project storage. |
| `library_home.rs` | 256 | Home dashboard aggregate queries. |
| `path_migration.rs` | 190 | Stored path rewrite after relocating application storage. |
| `albums.rs` | 190 | Album CRUD + cover frames/photos. |
| `metadata.rs` | 155 | photo_metadata store/normalize/backfill. |
| `scrapbook.rs` | 101 | Scrapbook tags/notes. |
| `overlay_assets.rs` | 69 | Overlay asset persistence. |

### Other directories

- **`src/albums/`** - album wall views: `view.rs` (719, build), `bookshelf.rs` (744, bookshelf theme), `card.rs` (707, album card/photo opening), `coverflow.rs` (372), `coverflow_input.rs` (81), `actions.rs` (408, context menus).
- **`src/collage/`** - `mod.rs` style: `model.rs` (899, `CollageDraft`/`CollageProject`, layout kinds), `editor.rs` (2122, `CollageEditor`), `layout.rs` (696, grid/mosaic apply), `smart_mosaic.rs` (678, mosaic packing), `render.rs` (526, collage export), `persistence.rs` (222).
- **`src/screensaver/`** - root `screensaver.rs` (3399, `Style`, preferences, mosaic/pile orchestration); `mod gpu; mod matrix;`. `gpu.rs` (906, render pipeline) declares `gpu/{pile.rs (1503), flying.rs (1503), cinema.rs (489), coverflow.rs (200)}`; `matrix.rs` (252, Matrix rain easter egg).
- **`src/scanner/`** - root `scanner.rs` (2101, `ScanEvent`, `IndexedPhoto`, `ScanControl`); `progressive.rs` (959, incremental scan incl. network sources) + `progressive/tests.rs` (495); `network_raw.rs` (223, network RAW dimension parsing); `network_dimensions_regression_tests.rs` (457).
- **`src/css/`** - `mod.rs` (196, `BASE` + per-component CSS constants, `resolve_runtime_dir`, `install_foundation`), `theme_discovery.rs` (879, scans `themes/<id>/`, parses theme metadata/CSS), `base.css`, `photo_wall.css`, `edit_mode.css`, `screensaver.css`, `square_corners.css`, platform css (`linux`/`macos`/`windows`), `components/` (11 partial CSS files: sidebar resize, corner style, metadata filter layout, quick filters, ...).
- **`src/settings/`** - root `settings.rs` (2939, `SettingsWindow`, corner style, sidebar page, formats page); `database.rs` (1038, library management UI); `menus.rs` (149, feature toggles); `honeycomb.rs` (119, Honeycomb border-width preference, persisted as `honeycomb.border-width` setting).
- **`src/onboarding/`** - `view.rs` (509, welcome/tips screens), `tour.rs` (204, screenshot tour assets), `gtk_tests.rs` (489), `tests.rs` (82).
- **`src/platform/`** - `mod.rs` + per-OS `linux.rs`/`macos.rs`/`windows.rs`: file reveal + native mount monitors.
- **`src/library_home/`** - `scrapbook.rs` (932, scrapbook dashboard UI), `scrapbook/add_photos.rs` (266, picker).

## 5. Cross-cutting links (the "spine")

1. **Data spine:** `db.rs` (SCHEMA + row structs) -> all `db/*` query files -> consumed by
   `scanner` (writes), `grid`/`lightbox` (reads via `PhotoObject`), `sidebar` (counts/folders/albums).
2. **Render spine:** `edit::render::apply_recipe` (`render.rs`) is the canonical committed CPU
   path, shared by thumbnails, lightbox, export, print, collage, and filter tiles.
   The editor's **live GPU preview is a distinct implementation**: `LiveGpuCanvas` on a
   `gtk::GLArea` in `edit/live_gpu.rs`, gated by `EditRendererMode` (env `PIC_EDIT_RENDERER`,
   auto decision cached with retry, CPU picture kept behind the GLArea as fallback).
   **When adding an effect, inspect both paths and test their visual parity; do not assume
   changing one updates the other.**
3. **Window spine:** `window/build.rs` is the hub. It owns the sidebar (`sidebar::build`), the
   gallery (`grid::view::Gallery`), the `Lightbox`, the `InfoBar`, and wires every callback;
   it `include!`s `dialogs`, `navigation`, `shortcuts`, `layout`, `toolbar`, `immersive`.
   `window.rs` (module root) holds the filter<->gallery wiring and `PhotoActionContext`.
4. **Filter spine:** `sidebar::SidebarFilter` is the navigation currency - produced by the sidebar,
   consumed by `window.rs`, `window/library.rs` (grid refresh), `window/photo_layout.rs`
   (layout persistence), `library_home.rs` (cards), `grid/selection.rs` (nav scope).
5. **Thumbnail spine:** `scanner` -> `thumbnail` (decode workers) -> `thumbnail_display`
   (display workers) -> grid tiles (paintables). `source.rs` gates reads for the lightbox.
6. **Theme spine:** `css/theme_discovery.rs` discovers `themes/<id>/`; `window/theme.rs` applies
   the selected theme CSS; `settings` stores the choice; `sidebar.rs` has its own
   `sidebar-*-visible` setting keys (persisted via `db/settings.rs`).
7. **Network spine:** `network_picker` (UI) -> `network_shares` (facade) ->
   `smb_transport` / `private_smb` / `private_nfs` (userspace, no mounts) -> `scanner` +
   `db/network_import` (resumable jobs).
8. **Export spine:** `edit/export_batch` (batch) and `window/build.rs` export dialogs
   (`show_photo_export_dialog`) -> `edit::render::render_for_export` + `edit/export_metadata`
   -> new file + rebuilt EXIF/sRGB. Runs on plain `std::thread`s (plain Rust values only;
   `PhotoObject` is not Send/Sync).

## 6. Quick-Edit Guide (task -> files)

| I want to change... | Read these first | Then edit |
|---|---|---|
| **Sidebar** (rows, folders, counts, visibility, pin/hover) | `src/sidebar.rs` (`build`, `SidebarFilter`, `SidebarVisibility`) | `src/sidebar.rs`; persistence keys there; counts from `db/settings.rs::sidebar_counts`; wiring in `src/window/build.rs`; divider drag test `tests/compact_sidebar_x11_drag.py`; CSS `navigation-sidebar` + `components/sidebar_resize.css`; CI `compact-sidebar-check.yml` (paths: `window/layout.rs`, `window/build.rs`, `settings.rs`, `css/base.css`, `css/components/sidebar_resize.css`, the drag test) |
| **Immersive Mode** (sidebar edge reveal, pin exit, overlay animation, viewport stability) | `src/window/immersive.rs` (included by `build.rs`; state cells in `build.rs`), `src/window/layout.rs`, `src/window/layout_settle.rs` | Verify `src/window/build.rs` integration and `src/window/immersive_tests.rs`; reuse the existing sidebar; do not animate the gallery/split or alter normal-mode pin behavior unless requested |
| **Honeycomb** (hex layout, border/gap, performance, scrolling) | `src/grid/honeycomb.rs` (hex math), `src/grid/honeycomb_surface.rs` (clip widget), `src/grid/honeycomb_raster.rs` (mask workers), `src/grid/photo_wall.rs` (`PhotoLayout::Honeycomb`), `src/grid/tile.rs` | `src/grid/photo_wall_layout.rs` (geometry), `src/settings/honeycomb.rs` (border-width pref), `src/grid/view.rs` / `src/window/layout.rs` as applicable; test Grid/Wall/Masonry regression alongside Honeycomb (`grid/photo_wall_tests.rs`) |
| **Gallery scroll anchoring / restoration** | `src/grid/view.rs`, `src/grid/navigation.rs`, `src/grid/virtualization.rs` | `src/window/library.rs`, `src/window/layout.rs`, `src/smooth_scroll.rs`; validate scroll position after sidebar toggles and layout changes |
| Header toolbar | `src/window/toolbar.rs` | same; assembled in `window/build.rs` |
| Bottom InfoBar | `src/infobar.rs` (`InfoBar`) | `infobar.rs`; photo rows via `window/photo_actions.rs`, album menu via `window/albums.rs` |
| Gallery grid / tiles / zoom | `src/grid.rs` (size defaults, `ZOOM_LEVELS` at top), `src/grid/view.rs` (`Gallery`), `src/grid/tile.rs` | grid files; virtualization in `grid/virtualization.rs`; layouts in `window/layout.rs` |
| Selection / keyboard nav | `src/grid/selection.rs` | + `window/shortcuts.rs` |
| Lightbox / viewer | `src/lightbox.rs` (`Lightbox`), `src/lightbox/render.rs` | `lightbox/impl.rs`, decode lanes in `source.rs` |
| Editor UI | `src/edit/editor.rs` (`EditEditor`, `build`) | panels in `edit/controls.rs`, `draw_ui.rs`, `text_ui.rs`, `overlays_ui.rs`, `crop.rs` (all `include!`d into `editor.rs`) |
| A new edit effect / recipe field | `src/edit/model.rs` (`EditRecipe`), `src/edit/render.rs` | add to model + `render::apply_recipe`; GPU path in `edit/live_gpu.rs` |
| Export | `src/edit/export_batch.rs`, `src/edit/export_metadata.rs` | + dialogs in `window/build.rs` (`show_photo_export_dialog`) |
| Thumbnails / decode | `src/thumbnail.rs`, `src/thumbnail/viewer.rs` | format decoders in `thumbnail/*.rs`; display workers `thumbnail_display.rs`; CI `nef-quality-harness.yml` for NEF paths |
| Folder scanning / import | `src/scanner.rs` (`ScanEvent`, `ScanControl`), `scanner/progressive.rs` | SD import `sd_import.rs`; network `scanner/network_raw.rs` |
| Database schema / queries | `src/db.rs` (SCHEMA, row structs) | specific `db/<area>.rs`; migrations in `db/core.rs` (schema) + `db/path_migration.rs` (storage relocation) |
| Settings window | `src/settings.rs` (`SettingsWindow`) | `settings/database.rs`, `settings/menus.rs`; generic store `db/settings.rs` |
| Themes (new/edit theme) | `docs/THEMES.md`, `src/css/theme_discovery.rs` | `themes/<id>/` folder (CSS); application logic in `window/theme.rs` |
| Albums views | `src/albums_view.rs`, `src/albums/view.rs` | `albums/bookshelf.rs`, `albums/card.rs`, `albums/coverflow.rs`; data in `db/albums.rs` |
| Collage | `src/collage/mod.rs` (exports) | `collage/editor.rs`, `collage/layout.rs`, `collage/model.rs`; CI `collage-sidebar-check.yml` |
| Screensaver | `src/screensaver.rs` (`Style`) | `screensaver/gpu.rs` + `gpu/{pile,flying,cinema,coverflow}.rs` |
| Print | `src/window/print.rs` | renders via `edit::render` |
| Metadata/facet search | `src/db/filters.rs` (`Field`, `Filters`) | `window/metadata_panel.rs`, `window/search.rs` |
| Onboarding | `src/onboarding.rs` (state) | `onboarding/view.rs`, `onboarding/tour.rs`, `window/onboarding.rs` |
| Home dashboard | `src/library_home.rs` (`LibraryHome`) | `library_home/scrapbook.rs`; data `db/library_home.rs` |
| Network shares | `src/network_shares.rs` | `smb_transport.rs`, `private_smb.rs`, `private_nfs.rs`, `native/*.c`, `network_picker.rs` |
| Window layout/split | `src/window/layout.rs`, `window/layout_settle.rs` | responsive tests `window/responsive_tests.rs` |
| App paths / storage | `src/app_paths.rs` | cache paths in `thumbnail/cache.rs` |

## 7. Diagnostics & CI (see AGENTS.md for full rules)

- Trace env vars (opt-in, read at runtime - never remove): `PICASA_TRACE` (master, 193 refs
  at verification), `PIC_STARTUP_TRACE` (`startup_trace.rs`), `PIC_INSPECTOR_TRACE`,
  `PIC_SCREENSAVER_TRACE`, `PIC_LIGHTBOX_OPEN_TRACE`, `PIC_COLLAGE_TRACE`,
  `PIC_LIBRARY_MAINTENANCE_TRACE`, `PIC_GALLERY_RESTORE_TRACE`, `PIC_METADATA_TRACE`,
  `PIC_EXPORT_BENCH_ALBUM` (debug only). Trace helpers: `nav_transition_trace.rs`,
  `scroll_trace.rs`, `thumbnail_trace.rs`.
- Other behavioral env vars: `PIC_EDIT_RENDERER` (auto/gpu/cpu editor renderer),
  `PICASA_LEGACY_FOLDER_LIST` (force legacy folder ListView path),
  `PIC_TEST_EDIT_REFERENCE` (real-photo tests).
- CI (9 workflows, path-scoped; each runs `cargo check --locked` plus targeted tests):
  - `editor-quality.yml`: `src/edit/**`, `src/window/operation_progress.rs` - `edit::` tests
    (render, export_batch, export_metadata, model, filters), `native_preview_geometry_tests`,
    NEF wide-gamut test, plus ignored GPU-parity tests (`gtk_gl_area_*`) under xvfb.
  - `collage-sidebar-check.yml`: `src/collage/editor.rs` - `collage::` + ignored resize test under xvfb.
  - `screensaver-check.yml`: `src/screensaver.rs`, `src/screensaver/**`, `src/main.rs`, `src/window/toolbar.rs` - `screensaver::`.
  - `selection-infobar-check.yml`: `src/infobar.rs`, `src/grid/selection.rs`, `src/window/build.rs`, `src/css/base.css` - full suite + ignored InfoBar test under xvfb.
  - `compact-sidebar-check.yml`: `src/window/layout.rs`, `src/window/build.rs`, `src/settings.rs`, `src/css/base.css`, `src/css/components/sidebar_resize.css`, `tests/compact_sidebar_x11_drag.py` - static regression guards + drag repro.
  - `nef-quality-harness.yml`: `src/thumbnail.rs`, `src/thumbnail/viewer.rs`, `src/thumbnail/raw_quality.rs`, `src/thumbnail/nef_scene.rs` - `raw_quality::tests`, `nef_scene::tests`, NEF cache tests.
  - `scrapbook-skin-check.yml`: `themes/scrapbook/**`, `src/css/theme_discovery.rs`, `src/window/theme.rs`, `src/library_home.rs`, `src/library_home/**`, `src/db/filters.rs`, `src/window/metadata_panel.rs`, `src/library_home_tests.rs`, `src/db.rs`, `src/db/scrapbook.rs`, `src/window/layout.rs`, `src/db/library_home.rs`, `src/main.rs`, `src/albums/card.rs`, `src/css/base.css`.
  - `gpu-packaging-validation.yml`: `workflow_dispatch` only (expensive AppImage GPU validation).
  - `appimage-release.yml`: on GitHub release / packaging script paths.
- GUI regression: `scripts/test-gui.sh` (suites via `PIC_GUI_SUITE=all|startup|library|import|viewer|editing|edit-navigation|browse-navigation|screensaver|native|wayland`; `native` delegates to `test-gui-native.sh`), artifacts in `reports/gui-tests/`. See `scripts/README-gui-tests.md`.

## 8. Gotchas

- Module layout is mixed intentionally and **`include!`-based**: `db.rs`, `grid.rs`, `lightbox.rs`,
  `thumbnail.rs` are root files that `include!` most sibling files (so e.g. `grid/view.rs` is
  part of the flat `grid` module, not `grid::view`); `window.rs`/`window/build.rs` and
  `edit/editor.rs` use in-function `include!` for code that needs live widgets in scope.
  `edit` and `collage` use `mod.rs`. Never convert between styles.
- `src/grid/grid.rs`, `src/db/migration.rs`, and `src/thumbnail/texture.rs` are **orphan files**:
  tracked in Git but referenced by nothing and not compiled. Do not edit them expecting behavior
  changes; do not "fix" code by copying from them.
- `window/build.rs` (7,470 lines) and `edit/live_gpu.rs` (5,960) are the two monster files -
  search within them rather than reading top to bottom.
- `window/immersive.rs` is `include!`d inside a `window::build` function in `build.rs`
  (presentation-only; needs live widgets in scope).
- Tests needing display/OpenGL/hardware are `#[ignore]`d; run under
  `xvfb-run -a env GSK_RENDERER=cairo cargo test --locked <name> -- --ignored --test-threads=1`.
- All 1,136 tests share GTK/GLib globals: always `--test-threads=1`.

## 9. Feature impact and validation matrix

Likely dependencies and test entry points; inspect the current test definitions and `AGENTS.md` before running. All cited suites below were confirmed to exist at verification.

| Feature / entry point | Neighboring features at risk | Focused validation to locate first |
|---|---|---|
| Immersive overlay / `window/immersive.rs` | Sidebar pin/hover, compact mode, responsive split, gallery width and scroll anchoring | `window/immersive_tests.rs`, `window/responsive_tests.rs`; manual hide/reveal cycles in Grid, Honeycomb, Photo Wall, Masonry |
| Sidebar / `sidebar.rs` | Filters, navigation persistence, compact visibility, immersive overlay | `tests/compact_sidebar_x11_drag.py` (standalone repro only), `compact-sidebar-check.yml`, relevant window tests |
| Window split / `window/layout.rs` | All gallery layouts, sidebar, metadata panel, fullscreen/immersive | `window/responsive_tests.rs`, `window/immersive_tests.rs`, `scripts/test-gui.sh` suites |
| Honeycomb / `grid/honeycomb_raster.rs` | Tile caches, mask edges, thumbnail quality, scroll frame times, memory, Photo Wall anchoring | `grid/photo_wall_tests.rs`, `grid/history_tests.rs`; real-gallery scroll check |
| Grid virtualization / `grid/virtualization.rs` | Selection, folder stream, viewport anchoring, grouped view | `grid` tests, history/sectioned-folder tests, interactive scroll/zoom verification |
| Thumbnail / `thumbnail/*.rs` | Grid, lightbox, RAW/NEF/DNG display, cache eviction, edited previews | Decoder/cache-focused tests; `nef-quality-harness.yml` for NEF/RAW paths |
| Editor recipe / `edit/model.rs` + `edit/render.rs` | GPU live preview, export, printed photos, collage, persistence | `editor-quality.yml` (incl. `gtk_gl_area_*` parity tests under xvfb); compare preview and exported image |
| Database / `db.rs` + `db/*.rs` | Libraries, scanner, filters, metadata, migrations | `db/tests.rs`, migration tests, library open/reopen checks |
| Screensaver / `screensaver.rs` | GPU effects, photo picking, preferences and preview | `screensaver::` tests and `screensaver-check.yml` |
| Network shares / `network_shares.rs` | SMB/NFS transport, progressive scan, offline status, recovery | `scanner` network tests; check on unavailable remote source |

**Manual regression rule:** For UI changes, verify not only the target mode but also normal and compact modes, keyboard behavior, scrolling, window resize, and the gallery layouts touched by the change. A standalone GTK repro is **not** a replacement for testing the real application.

## 10. Agent navigation protocol

1. **Read `AGENTS.md` first.** It is authoritative for tools, build/run instructions, tests, branch discipline, and unrelated working-tree changes.
2. **Locate the task** in Section 6, then inspect the listed source, call sites, and relevant tests. Search by symbol/function name, not by recorded line number.
3. **Map the impact** using Sections 5 and 9. Identify state ownership and which layout, cache, renderer, or persistence path is affected.
4. **Minimize scope.** Reuse existing widgets and implementations. Do not recreate the sidebar, add parallel layout state, change persistent settings, restructure modules, or modify unrelated UI just to fix a localized issue.
5. **Preserve invariants.** For immersive transitions, keep gallery viewport geometry/scroll position stable unless resizing is intentional. For rendering changes, preserve thumbnail quality and preview/export consistency (CPU `apply_recipe` vs GPU `LiveGpuCanvas`). For background work, keep GTK objects on the GTK thread.
6. **Validate at the narrowest useful level first**, then run broader build/tests and GUI checks required by `AGENTS.md`. Check warnings, not just exit status. Run `cargo run` for interactive verification when required and a display is available.
7. **Report results accurately:** files touched, behavior changed, tests actually run, skipped checks, known limitations, and Git status. Do not claim visual, hardware, or performance validation without measurements.
8. **Update this index only when architecture changes.** If a module is renamed, moved, newly owns a feature, or gains a significant dependency, update the relevant map/Quick-Edit/matrix entries together. Do not automatically refresh every line count for small code changes.

## 11. Audit refresh checklist

Completed once on 2026-11-14 against commit `3f57bf31` + uncommitted working-tree changes (see metadata). Re-run this checklist when re-verifying:

- Record `git rev-parse HEAD`, branch name, and audit date in the metadata table; note any dirty working tree.
- Confirm that indexed files actually exist; remove or mark absent/orphan paths (current orphans: `grid/grid.rs`, `db/migration.rs`, `thumbnail/texture.rs`), and find newly declared modules.
- Verify ownership from `mod`/`include!` declarations, constructors, call sites, and tests. Treat any old description as a hypothesis until checked.
- Refresh important symbol names and feature relationships first; exact line counts are optional and should remain labeled approximate.
- Check `AGENTS.md` and `.github/workflows/` for the actual commands, CI paths, and test labels; do not invent a suite.
- Check that `window/immersive.rs` inclusion (inside `window/build.rs`) and its interaction with `window/layout.rs` still match the working code.
- Verify the thumbnail/GPU rendering relationship before declaring a universal render pipeline (`edit::render` CPU path vs `edit/live_gpu.rs` GLArea path).
- Keep documentation-only changes separate from code modifications; preserve unrelated working-tree files.

**Verification status:** fully re-verified against the live checkout on 2026-11-14 (commit `3f57bf31`, branch `main`, dirty tree as noted). All paths, module declarations, key symbols, CI paths, and the Quick-Edit Guide were checked against source. No source code was modified as part of this verification.
