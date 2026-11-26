# Tracing and diagnostics variables

PIC reads these environment variables at runtime. Each one is opt-in: set it (any value) to turn on extra `stderr` output, and leave the code that reads it in place. Diagnose slowness by setting `PICASA_TRACE` first, then the narrower variable for the area involved.

A passing display test swallows `stderr`, so add `-- --nocapture` when a trace is the evidence.

| Variable | Read in | Traces |
|---|---|---|
| `PICASA_TRACE` | ~40 files across `edit/`, `grid/`, `thumbnail/`, `lightbox/`, `window/`, scanner, imports | Master switch for stage timing: export stages, editor effects, thumbnails, lightbox, gallery. |
| `PICASA_TRACE_WALL_ROWS` | `grid/photo_wall_layout.rs` | Photo Wall row layout. |
| `PICASA_TRACE_SCROLL_STACKS` | `grid/sectioned_folder.rs` | Folder view scroll stacks. |
| `PIC_STARTUP_TRACE` | `startup_trace.rs`, `window/build.rs`, `grid.rs`, `db/photos.rs`, `window/availability.rs` | Startup stages (`PIC_STARTUP stage=...`). |
| `PIC_SCROLL_TRACE` | `scroll_trace.rs` | Gallery scrolling. |
| `PIC_SCROLL_FRAME_TRACE` | `window/build.rs` | Per-frame scroll trace, with window size and scroll position. |
| `PIC_WHEEL_GLIDE_TRACE` | `window/navigation.rs` | Wheel glide (smooth wheel scrolling). |
| `PIC_GALLERY_RESTORE_TRACE` | `grid/sectioned_folder.rs`, `grid/selection.rs` | Gallery position restore. |
| `PIC_GALLERY_FOLDER_REFRAME_TRACE` | `grid/navigation.rs` | Folder reframing on navigation. |
| `PIC_NAV_TRANSITION_TRACE` | `nav_transition_trace.rs` | Navigation transitions. |
| `PIC_THUMBNAIL_TRACE_URI` | `thumbnail_trace.rs` | Events for one photo, named by URI. |
| `PIC_HONEYCOMB_MASK_TRACE` | `grid/tile.rs`, `grid/honeycomb_raster.rs` | Honeycomb tile masks. |
| `PIC_SIDEBAR_TRACE` | `window/sidebar_widgets.rs` | Every sidebar event that changes the scene. |
| `PIC_INSPECTOR_TRACE` | `window/properties_widgets.rs`, `window/build.rs`, `grid/view.rs`, `grid/sectioned_folder.rs` | Properties inspector presentation. |
| `PIC_LIGHTBOX_OPEN_TRACE` | `lightbox/impl.rs`, `lightbox/render.rs`, `lightbox/opening_transition.rs` | Lightbox opening. |
| `PIC_METADATA_TRACE` | `metadata_index.rs`, `window/metadata_panel.rs` | Metadata index and panel. |
| `PIC_OVERLAY_TRACE` | `edit/overlays.rs`, `edit/export_benchmark.rs` | Editor overlays. |
| `PIC_GPU_AUTOSTATS_DIAG` | `edit/live_gpu.rs` | Live-GPU auto statistics. |
| `PIC_COLLAGE_TRACE` | `collage/editor.rs`, `collage/render.rs`, `collage/smart_mosaic.rs` | Collage. |
| `PIC_COVERFLOW_TRACE` | `albums/coverflow.rs` | Album cover flow. |
| `PIC_SCREENSAVER_TRACE` | `screensaver.rs`, `screensaver/gpu.rs` | Screensaver. |
| `PIC_WATCH_TRACE` | `window/build.rs` | Folder watching: monitor rebuilds (watched folders, monitors, catch-up refreshes), raw monitor events, debounced refresh dispatch. |
| `PIC_LIBRARY_MAINTENANCE_TRACE` | `window/build.rs`, `settings/database.rs`, `thumbnail/batch.rs` | Library maintenance. |
| `PIC_EXPORT_BENCH_ALBUM` | `main.rs` (debug builds only, `cfg(debug_assertions)`), `edit/export_benchmark.rs` | Runs the export benchmark on an album instead of starting the app. |

Lightbox large images under `PICASA_TRACE`: `PIC_DECODE stage=full_decode_retain action=publish|skip_not_owner|skip_limit|take_native|release_navigation|release_close`, `stage=full_decode_reuse`, `stage=rgba_resize opaque=`, `stage=rgba_conversion source=rgba_moved`. `scripts/bench-lightbox-large-images.sh` counts cancellations from `decode_done ... outcome=error` and `display_discard` lines.

Test-only variables (`PIC_TEST_*`, `PICASA_TEST_*`, `PIC_BENCH_*`) are read by `#[ignore]`d tests and benchmarks. Find each one's meaning at its read site with `grep -rn '"PIC_TEST_<NAME>"' src`.
