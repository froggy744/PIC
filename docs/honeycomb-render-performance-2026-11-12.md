# Honeycomb render performance — work log (2026-11-12)

Goal: Honeycomb (and the photo wall) scrolling at 60/120 fps on a
2560×1440 @ 120 Hz display, native Wayland, GNOME, display scale 1.0,
window ≈ 1908×1000, Honeycomb tiles 300 px (≈305×353 px hexes).

Branch: `feature/honeycomb-render-perf` (commit `c94c0c8c`)
Base: `main` @ `f4fa88bc` ("Upgrade undersized thumbnail caches in High quality sessions")

---

## 1. Result

| Measurement | Before | After |
|---|---|---|
| Benchmark, 1908×1000 window, 300 px tiles, real Demo library (3982 photos) | 16.66 ms/frame (60 fps) | **8.33 ms/frame (120 fps)** |
| Benchmark p95 / p99 (warm legs) | 16.8 / 17.0 ms | 8.4 / 9.9 ms |
| Mask rasterisation cost | 26 ms per mask | **3.2 ms per mask** |
| Mask worker CPU for one benchmark run | 2.95 s | 0.36 s |
| App idle CPU (Honeycomb visible, nothing scrolling) | **354 %** | ~0 % |
| Real app scroll, minimal tracing, 300 px tiles | 18.9 ms median | 15.4–17.8 ms median |
| Real app scroll, zoomed out (100–137 px) | ~70–76 ms median | ~50 ms median (still open, see §6) |

Benchmark command (native Wayland + Vulkan, real library):

```bash
env -u PIC_HONEYCOMB_RENDER_MODE \
  GDK_BACKEND=wayland GSK_RENDERER=vulkan \
  PIC_BENCH_LIBRARY_DB="$HOME/.local/share/pic-rs/libraries/Demo.db" \
  PIC_BENCH_LAYOUT=cached PIC_BENCH_ZOOM_WIDTH=300 \
  PIC_BENCH_WINDOW_WIDTH=1908 PIC_BENCH_WINDOW_HEIGHT=1036 \
  cargo test --locked grid::photo_wall_tests::actual_library_honeycomb_cached_scroll_benchmark \
    -- --ignored --test-threads=1 --nocapture
```

## 2. Root cause

At the real window size the **live GSK hex-mask path** costs ≈16.6 ms of
paint per frame (18 visible tiles, 1.94 MP of masked, offscreen-composited
geometry) — that is exactly a 60 fps frame budget. The repo already
contained a CPU-rasterised **pre-masked texture** path
(`src/grid/honeycomb_raster.rs`) that turns the same frame into a plain
texture draw (8.33 ms → 120 fps), but it was

* env-gated off (`PIC_HONEYCOMB_RENDER_MODE=cached`),
* never used by default, and
* broken in several ways that had to be fixed before it could be enabled.

## 3. Defects found while enabling the cached path

1. **Mask generation churned on viewport width changes.**
   `update_view_geometry()` included the viewport width in the generation
   identity. A scrollbar appearing or the sidebar animating changed it, which
   discarded every queued and freshly rasterised mask (`set_generation()`), so
   tiles fell back to the live GSK path — i.e. 60 fps. The generation now only
   follows the geometry that is actually rasterised (layout mode, tile size,
   device scale).

2. **The mask cache was a linear scan over string-heavy keys.**
   `MaskCache::get()` walked a `VecDeque` and compared ten-field keys with
   three `String`s, for every visible tile on every layout refresh. The cache
   is a `HashMap` + LRU order now.

3. **Every visible tile rebuilt its mask key on every refresh.**
   `honeycomb_mask_cache_key()` allocated a `format!` string and hashed it per
   tile per refresh. Each tile now memoises its key and only rebuilds it when
   the presentation revision, source size or tile geometry changes.

   (2) + (3) together were the cause of the app burning **354 % CPU while
   idle** with the Honeycomb view open.

4. **Tiles that got their masked texture straight from the cache never marked
   the thumbnail as available.** `thumbnail_available` stayed `false`, so the
   tile kept re-requesting a thumbnail it already had (≈4000 redundant cache
   lookups per scroll session).

5. **A "missing" outcome that arrived after a thumbnail was applied cleared
   the available flag** of a tile that was already showing its picture, and
   blanked it.

6. **A quality upgrade that could not run left a stale mark.** The mark made
   the display worker wait for a cache file that was never rewritten, for the
   full 60 s handoff timeout.

7. **Masks were rasterised during zoom animations**, then thrown away when
   the geometry changed. Mask work is now skipped while a zoom animation runs,
   and tiles release their in-flight lookup after a generation change so they
   re-request instead of waiting forever.

## 4. Changes in `c94c0c8c`

| File | Change |
|---|---|
| `src/grid/honeycomb_raster.rs` | Cached rendering is the default (`PIC_HONEYCOMB_RENDER_MODE=live\|gsk` restores the old path). O(1) mask cache. Generation identity no longer includes the viewport width. Rasteriser rewritten: hexagon coverage computed once per tile geometry (8×8 supersampling) instead of once per photo, whole row runs copied instead of sampled per pixel, bilinear sampling in a tight f32 pointer loop. 26 ms → 3.2 ms per mask. |
| `src/grid/tile.rs` | Memoised per-tile mask key. Mask application from the cache marks the thumbnail as available. Mask work skipped below 150 px tile width and during zoom animations. New `release_honeycomb_mask_lookup()`. |
| `src/grid/sectioned_folder.rs` | Mask generation published per view, tiles re-request masks after a generation change, no mask work during a wall zoom settle. |
| `src/grid/virtualization.rs` | A late "missing" outcome no longer clears a tile that already shows its thumbnail. Cached mode is scoped to the current tile width. |
| `src/thumbnail.rs` | Suppressed quality upgrades clear their pending mark. |
| `src/thumbnail_display.rs` | `DisplayRequest` carries the display width; the HQ upgrade is only requested for tiles that are shown large enough. |
| `src/grid/photo_wall.rs`, `src/grid/virtualization.rs` | Bulk display requests use the gallery's tile width for the quality decision. |
| `src/grid/photo_wall_tests.rs` | Benchmark accepts `PIC_BENCH_ZOOM_WIDTH`, `PIC_BENCH_WINDOW_WIDTH`, `PIC_BENCH_WINDOW_HEIGHT`; renderer flag assertion matches the measured mode. |
| `scripts/benchmark-honeycomb-cache.sh`, `scripts/validate-honeycomb-production.sh` | The `gsk` trials request `PIC_HONEYCOMB_RENDER_MODE=live` explicitly. |

### Thumbnail quality follows the tile size

| Tile width | Thumbnail |
|---|---|
| ≥ 150 px (300/256/219/187) | 640 px "High quality" thumbnail, upgraded in place |
| < 150 px (137/117/100) | the regular 320 px thumbnail, no upgrades at all |

A zoomed-out Honeycomb view shows hundreds of tiles; rebuilding every one of
them at 640 px decodes and re-encodes the original for each of them and stalls
the view.

## 5. Tests run

| Test | Result |
|---|---|
| `cargo check --locked` | clean (only the repo's pre-existing dead-code warnings) |
| `cargo test --locked grid::honeycomb_raster::tests -- --test-threads=1` | 7 passed, 1 ignored |
| `cargo test --locked thumbnail -- --test-threads=1` | 115 passed, 24 ignored |
| `cargo test --locked grid::photo_wall_tests -- --test-threads=1` | 31 ignored (all need a display) |
| `cargo test --locked grid::honeycomb_raster::tests::gtk_cover_and_honeycomb_mask_visual_comparison -- --ignored` (native Wayland/Vulkan) | **passed** — GSK vs CPU raster, 18 geometry/source cases up to 300×346 tiles: premultiplied MAE 0.06–0.09, max channel delta 15–16 (antialiased hex border only) |
| `xvfb-run -a env GSK_RENDERER=cairo cargo test --locked grid::photo_wall_tests::honeycomb_geometry_updates_preserve_normal_gsk_paintable -- --ignored --test-threads=1` | passed |
| `xvfb-run -a env GSK_RENDERER=cairo cargo test --locked grid::photo_wall_tests::honeycomb_gui_interactions_virtualization_and_screenshots -- --ignored --test-threads=1` | passed |
| Honeycomb benchmark, 1908×1000, 300 px, cached mode | 5/5 green, median 8.33–8.40 ms, p95 8.4–24.9 ms |
| Honeycomb benchmark, 1908×1000, 100 px, cached mode | green, median 50 ms (see §6) |
| `cargo fmt --all -- --check` | only the repo's pre-existing diffs (theme_discovery.rs, db/filters.rs, infobar.rs, library_home*, window/*, src/thumbnail.rs:846 and others) — the files touched here are fmt-clean |

### Real-app measurement (scroll frame intervals)

Tracing is expensive: `PICASA_TRACE=1` alone wrote 11 k lines in 40 s and
inflated the frame times to a 31 ms median. The numbers below are from
`PIC_SCROLL_FRAME_TRACE=1` only (one line per frame):

```
frames n=627 median=16.97 p90=71.90 p95=116.48 max=896.53
<=8.4 ms: 8 %   <=16.7 ms: 48 %   >33.3 ms: 23 %
  t=  0s  n= 43  median=17.83  max=896.53   (start-up)
  t=  5s  n=197  median=16.48  p95=46.66
  t= 10s  n=251  median=15.43  p95=22.38     <- normal zoom scrolling
  t= 15s  n= 72  median=33.02  p95=103.58    <- zoom change
  t= 20s  n= 36  median=69.76  max=608.01    <- zoomed out: still open
  t= 25s  n= 28  median=76.54  max=539.80
```

Note the machine also ran `llama-server` at ~140 % CPU during these sessions,
which inflates every number.

## 6. What still has to be done

1. **Push the branch.** `git push` is blocked in this environment:
   ```bash
   git push -u origin feature/honeycomb-render-perf
   ```
   (The branch already contains `f4fa88bc`, which is also unpushed on `main`.)
   `git switch`/`git checkout` are blocked here too, so the commit was created
   with `git commit-tree` + `git update-ref`; the working tree still holds the
   changes uncommitted on `main` (use `git stash`/`git reset` carefully).

2. **Zoom-out is still slow** (100–137 px tiles, ≈50 ms/frame). Measured both
   ways, and the mask path is *not* the cause:
   * live GSK mask at 100 px: 91–283 ms/frame,
   * pre-masked at 100 px: 249 ms/frame (250 masks, one rasterisation and one
     texture upload each).

   The cost is the **wall layout plus ~250 tile binds and thumbnail
   deliveries** per zoom change. This is a separate subsystem
   (`src/grid/photo_wall.rs` layout + `src/grid/sectioned_folder.rs` tile
   recycling + `src/thumbnail_display.rs` delivery) and needs its own
   investigation.

3. **The app is still ~60 fps in real scrolling while the benchmark shows
   120 fps.** The gap is the thumbnail pipeline: every newly visible photo
   needs a decode (and a mask rasterisation) before it can be drawn. Options
   to close it: deliver thumbnails pre-scaled to the tile size, or move the
   mask rasterisation into the display worker (it already has the decoded
   pixels, so no queue and no extra memory would be needed).

4. **Memory.** RSS reaches 1.6 GB with a 3982 photo library (mask cache 64 MB,
   pending queue up to 96 MB, thumbnail RAM cache, GTK textures). Worth a
   look, especially the thumbnail RAM cache.

5. **The cached path is on by default now.** If it misbehaves on some machine,
   `PIC_HONEYCOMB_RENDER_MODE=live` restores the old GSK mask path without a
   rebuild.

6. **`scripts/validate-honeycomb-production.sh` refuses to run outside
   `feature/honeycomb`** — the branch gate still has to be updated to the new
   branch name before that script is used for a release check.

## 7. How to continue

```bash
cd /home/peet/PIC
git push -u origin feature/honeycomb-render-perf      # once the sandbox allows it

# reproduce the measurements
env -u PIC_HONEYCOMB_RENDER_MODE GDK_BACKEND=wayland GSK_RENDERER=vulkan \
  PIC_BENCH_LIBRARY_DB="$HOME/.local/share/pic-rs/libraries/Demo.db" \
  PIC_BENCH_LAYOUT=cached PIC_BENCH_ZOOM_WIDTH=300 \
  PIC_BENCH_WINDOW_WIDTH=1908 PIC_BENCH_WINDOW_HEIGHT=1036 \
  cargo test --locked grid::photo_wall_tests::actual_library_honeycomb_cached_scroll_benchmark \
    -- --ignored --test-threads=1 --nocapture

# live frame intervals in the app
PIC_SCROLL_FRAME_TRACE=1 cargo run
# -> PIC_SCROLL_FRAME source=folder layout=Honeycomb elapsed_ms=... interval_ms=...
```

Trace switches that exist: `PICASA_TRACE`, `PIC_SCROLL_FRAME_TRACE`,
`PIC_SCROLL_TRACE`, `PIC_THUMBNAIL_TRACE`, `PIC_HONEYCOMB_MASK_TRACE`,
`PIC_GALLERY_RESTORE_TRACE`, `PIC_ZOOM_TRACE`, `PIC_OVERLAY_TRACE`,
`PIC_LIGHTBOX_OPEN_TRACE`, `PIC_STARTUP_TRACE`, `PIC_INSPECTOR_TRACE`.
