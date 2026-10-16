# Photo Wall thumbnail quality implementation plan

**Goal:** Sharper visible Wall photographs with immediate normal preview fallback.
**Architecture:** Extend existing thumbnail decoding and priority pool with one bounded quality tier; use separate presentation keys and existing GTK RAM cache.
**Tech Stack:** Rust, GTK4, image, TurboJPEG, rawler.
**Spec:** docs/superpowers/specs/2026-09-30-photo-wall-thumbnail-quality.md

## Global Constraints
Work in current rc8 as requested. Preserve unrelated deletions. Preserve normal thumbnail paths, Grid, zoom, geometry, anchors and prefetch. No original I/O on GTK. Do not push implementation without review.

## Task 1: Resolution-aware decoding and cache maintenance
Interfaces: wall_cache_path(Path) -> PathBuf; create_uncached_with_max(path, destination, max_edge); existing create remains 320. Higher tier clamps upscaling. RAW and JPEG decode parameters propagate. Maintenance preserves both variants without changing required-photo counts.
- [x] Add failing generation, no-upscale, identity and maintenance tests.
- [x] Run focused tests; Expected: new interfaces absent or assertions fail.
- [x] Implement shared decoding parameters and cache paths.
- [x] Run focused tests; Expected: pass.

## Task 2: Bounded quality scheduling and presentation
Interfaces: separate low priority requests within existing priority pool, at most two active upgrades; cancellation by destination. Display quality keys include wall640 path; failures preserve small presentation. Existing RAM cache retains ordinary 512 plus 32/64MiB quality entries.
- [x] Add failing queue, presentation and memory-budget tests.
- [x] Run focused tests; Expected: fail for new behavior.
- [x] Implement tier-aware pool and presentation loading.
- [x] Run focused tests; Expected: pass.

## Task 3: Visible stationary Wall integration
Interfaces: existing SectionedFolder tick polls quality scheduler; signature generation/scroll/page/scale must settle 500ms. Only intersecting visible large tile allocations request quality. Tile validates current requested presentation; Grid or recycle clears quality. Normal requests never downgrade a valid quality presentation.
- [x] Add failing scheduling and display regression tests.
- [x] Run focused tests; Expected: fail.
- [x] Implement Wall integration and stale completion protection.
- [x] Run cargo test and practical display regressions plus real NEF fixture; Expected: pass.
- [x] Run cargo fmt --check and git diff --check; Expected: diff clean, report existing formatting debt accurately.

## Review Focus
Check queue lock ordering, normal request precedence, source availability/network rules, offline cache hits, failure retries, recycled tiles/edits, memory eviction, scale changes and unmapped surfaces. Fresh reviewer reviews full change against approved spec before completion.
