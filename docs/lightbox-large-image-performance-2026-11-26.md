# Lightbox large-image performance (2026-11-26)

Goal: fast lightbox open, 1:1 zoom and navigation for 4000+ px photos,
especially large PNGs (e.g. a 6072×9000 RGB PNG, 101 MB, and an 8640×8640
RGBA PNG), without a disk cache.

Branch: `lightbox-large-image-speed`, merged into `main`.
Commits: `bd172978`, `ef6786bd`, `17c8159d`, `55c6147b`, `53ffef98`.

## Causes found (PICASA_TRACE)

| Cause | Cost before |
|---|---|
| Local reads used GIO `load_contents` plus a copy | 224 ms for a 101 MB PNG |
| Viewer pixels copied into `glib::Bytes` on the GTK thread | 105–145 ms UI stall per 1:1 texture |
| 1:1 zoom re-read and re-decoded a PNG already fully decoded for FIT | ~600 ms |
| `fast_image_resize` premultiplied alpha of fully opaque RGBA (single-threaded full-size copy) | 100–235 ms per resize |
| Same-size RGBA "resize" at 1:1 copied the source | ~150 ms, +284 MB |

Not reducible without a disk cache: PNG inflate, ~280–400 ms for 40–75 MP files.

## Changes

- `source::read`: one sized `std::fs::read` for local paths; GIO for URIs.
- Decode worker builds `glib::Bytes`; the GTK thread creates the texture without copying.
- `thumbnail::decode_for_viewer_retaining`: one retained full-size decode for
  formats without reduced-size decoding.
  - Owned by the request of the photo in front (`viewer_retain_front`, request
    `id`); publish and take happen under the ownership lock, so cancelled,
    superseded or post-close workers cannot publish or remove a newer slot.
  - Dropped on navigation to any other photo and on close, freed on a
    background thread; a promoted prefetch publishes when it finishes.
  - Limit 320 MiB of decoded buffer; the owner's 1:1 decode takes the pixels
    (no second copy), and a same-size RGBA source is moved, not copied.
- RGBA resize skips premultiply when every alpha byte is 255 (pixel-identical).

## Results

Before/after, Xvfb, 1600×900 window, files in page cache:

| Measurement | `main` | After |
|---|---|---|
| Open 6072×9000 PNG (FIT) | 711 ms | 635 ms |
| FIT → 100%, RGB PNG | 844 ms | 50–56 ms |
| FIT → 100%, RGBA PNG (8640²) | 689 ms | 1 ms |
| Memory at 100%, RGB PNG (RSS / peak) | 553 / 762 MB | 336 / 545 MB |
| Memory at 100%, RGBA PNG (RSS / peak) | 708 / 994 MB | 415 / 702 MB |
| Opaque RGBA resize to fit (6426×6384) | 124–132 ms | 32–34 ms |

Final validation, `scripts/bench-lightbox-large-images.sh` (median / p95 / max ms):

| Scenario | Xvfb/cairo, 3 runs | Wayland/Vulkan, 1 run |
|---|---|---|
| Cold FIT open | 626 / 630 / 630 | 629 |
| Repeated 100% presented, RGB | 69 / 79 / 135 | 12 / 13 / 13 |
| Repeated 100% presented, RGBA | 85 / 96 / 168 | 12 / 13 / 13 |
| Repeated FIT presented | 13 / 18 / 24 | 13 / 18 / 18 |
| PNG → cached photo | 0 / 17 / 17 | 0 |
| Rapid navigation presented (400 ms steps) | 270 / 399 / 408 | 310 / 390 / 390 |
| Sustained navigation presented | 15 / 281 / 400 | 12 / 276 / 387 |

- Sustained navigation (19 large PNGs, 4 sweeps): RSS fell from ~524 to
  ~301 MB across the sweeps (Wayland 479 → 291 MB); peak 604 MB (576 MB).
- No `viewer-retain-drop` threads observed alive after a step or after close;
  thread count stable. No anomalies in any run.
- Rapid-navigation peak memory is ~130–160 MB above `main` (the retained
  decode of the photo in front while neighbours prefetch).

Known, not addressed: ~336 MB RSS remains after closing on every version
including `main` (pre-existing); a photo shown from the RAM texture cache
has no retained decode, so its first 1:1 still decodes.

## Reproduce

```bash
export PIC_BENCH_LIGHTBOX_PNG=... PIC_BENCH_LIGHTBOX_RGBA_PNG=... \
  PIC_BENCH_LIGHTBOX_JPEG=... PIC_BENCH_LIGHTBOX_RAW=... \
  PIC_BENCH_LIGHTBOX_SET='a.png|b.png|...' PIC_BENCH_LIGHTBOX_SUSTAINED='...'
REPS=3 scripts/bench-lightbox-large-images.sh xvfb      # private Xvfb
REPS=1 scripts/bench-lightbox-large-images.sh desktop   # opens windows on the desktop
```

Logs land in `reports/bench-lightbox/` (gitignored).
