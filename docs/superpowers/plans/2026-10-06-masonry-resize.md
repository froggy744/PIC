# Masonry Resize Implementation Plan

> **For agentic workers:** Use `superpowers:executing-plans` to implement this checklist inline, task by task. Implementation authorized and completed on the investigation branch. Validation results and remaining manual checks are recorded below.

**Goal:** Make Library/Photos Masonry resize with the stable geometry and viewport behavior already established for Photo Wall.

**Architecture:** Preserve Masonry column membership on width changes. Use a common, unrounded column width to calculate cumulative vertical edges, round only the resulting allocations, and map the viewport's logical content point with the geometry. Reuse Photo Wall's existing adjustment/allocation synchronization.

**Tech Stack:** Rust, GTK4, the existing virtual `SectionedFolderView` and pure layout engines.

**Spec:** The requirements, evidence, and acceptance criteria in this document; user instructions on 2026-10-06, including “look how we solved photowall that had the same issue.”

## Global constraints

- Start from freshly fetched `origin/main`, `e2a0e73c52126e20a36a9f4a171d4d9ec0eaffa9`.
- Work on `fix/masonry-resize-investigation`. Do not switch to or cherry-pick `fix/masonry-resize-stutter`.
- Width-only resizing preserves column assignment and the order within each column, including after mouse release. Zoom, collection changes, or a genuinely impossible narrow allocation may repartition.
- Keep Masonry's 8 logical pixel gaps and side margins where the viewport permits them, fixed folder header heights, whole-pixel allocations, and full horizontal coverage.
- Keep existing selection, thumbnail cache, lightbox behavior, and Photo Wall/Folders behavior.
- Do not add resize animation, frozen screenshots, synthetic widths, extra debounce delays, synchronous original-image reads, or a second thumbnail pipeline.

## Review focus

1. Deep-scrolled mixed-aspect libraries: every visible column must move coherently, even when only one integer column width changes.
2. Drag pauses and mouse release: width-only changes must not trigger a delayed shortest-column reassignment.
3. Resize and sidebar allocation changes: geometry, adjustment, and painted viewport translation must agree in the same frame.
4. Multiple folder sections, tall photos, and tiny widths: headers and gaps remain valid; viewport queries stay bounded and no negative allocation occurs.
5. Zoom, source replacement, and layout switching during resize: real invalidation rebuilds once and preserves current identity and selection.

## Investigation completed

- [x] Fetched `origin/main` and created the new branch directly from it; the initial checkout was clean.
- [x] Read current Masonry, Photo Wall, Folders, width observer, adjustment callbacks, and existing resize tests.
- [x] Read GitHub issue discussions and the actual successful Photo Wall commits.
- [x] Reproduced a geometry discontinuity using the production Rust engines without GTK, thumbnails, database access, or image decoding.
- [x] Ran existing pure Masonry tests and separate GTK resize tests for Masonry and Photo Wall.
- [x] Saved a repeatable diagnostic in `scripts/diagnostics/masonry-resize-probe.rs`.

### How Photo Wall was actually solved

The relevant history is newer and more specific than the earlier GridView discussion in `docs/GALLERY_V2.md`:

- [e24d8fb](https://github.com/froggy744/PIC/commit/e24d8fb24cf3f9e91328b1a1f556aae8e0ec6407): preserved row membership on window resize.
- [a4bf6b6](https://github.com/froggy744/PIC/commit/a4bf6b6d707688b21c7caec7f847305c15129cc4): replaced refitting with scaling, mapped scroll immediately using `adjustment.configure()`, and synchronized the allocated viewport before painting.
- [4a570cd](https://github.com/froggy744/PIC/commit/4a570cdb521e40d37f62120a8ff885ea90a9a5a6): retained ideal heights and rounded cumulative shared vertical edges so rounding error cannot accumulate down a long library.
- [Issue #144](https://github.com/froggy744/PIC/issues/144): documents the jitter, reported success, and confusion over which branch contained the fix.
- [Issue #124](https://github.com/froggy744/PIC/issues/124): records the rejected resize animation experience. Do not restart that experiment.

The working reference is `PhotoWallLayout::scale_photo_area()` in `src/grid/photo_wall_layout.rs`, coordinated by `SectionedFolderView::scale_wall_geometry()` and `attach_scroll()`. The current native window gesture tracker is restricted to Photo Wall, but continuous scaling remains active during ordinary window allocation changes. A new gesture tracker is not needed to fix the reproduced geometry defect.

### Confirmed differences in Masonry

**1. Independent integer column widths create deep vertical discontinuities.**

`resize_masonry()` distributes leftover pixels between columns, then computes every tile height as `round(integer_column_width / aspect_ratio)` and adds those rounded heights to column bottoms. A one-pixel viewport change can increase just one column's width. Every preceding image in that column then changes height, so visible photos in that column can jump hundreds of pixels relative to their neighbors. Mapping a single top photo preserves that one anchor while other columns move.

Run the saved diagnostic from the repository root:

```bash
rustc --edition=2021 -C opt-level=1 scripts/diagnostics/masonry-resize-probe.rs -o /tmp/pic-masonry-resize-probe
/tmp/pic-masonry-resize-probe
```

Fixture: ratios repeating `[0.5, 1.0, 1.5, 2.0]`, preferred width 100, viewport 997 → 998, 600 px visible band. Establish the layout first, scroll to the specified depth, then resize. Depth is a fraction of total content height, not of the scrollbar's maximum value.

| Photos | Scroll depth | Largest Masonry visible photo movement | Photo Wall movement |
| --- | ---: | ---: | ---: |
| 3,086 | Top | 5 px | 1 px |
| 3,086 | 40% | 141 px | 1 px |
| 3,086 | 90% | 315 px | 1.405 px |
| 20,000 | Top | 5 px | 1 px |
| 20,000 | 40% | 907 px | 1 px |
| 20,000 | 90% | 2,045 px | 1 px |

A subsequent repack at the same column count reassigns 2,660 of the 3,086 photos and 17,659 of the 20,000 photos. Scrolling down makes the live-resize discontinuity much worse because the changed heights of all preceding photos accumulate above the viewport. A test performed only at the top misses the scale of the defect.

This isolates a layout defect; it does not measure compositor frame time or prove that every native drag symptom has the same cause.

**2. Masonry reintroduces repacking after resize.**

`calculate_wall_geometry()` arms `masonry_resize_pending` after successful scaling. After 150 ms, `poll_masonry_resize()` invalidates geometry and reruns shortest-column placement, even if the preferred column count is unchanged. The reassignment counts above measure that unchanged-count repack. Photo Wall leaves its scaled membership intact after resizing stops.

**3. Masonry maps a photo's fixed top offset, while Photo Wall maps the logical point inside its row.**

The Masonry anchor is selected again from the current globally sorted items on each resize. This is a different contract from Photo Wall's within-row fraction mapping. Anchor changes and adjustment/allocation ordering need frame-level regression coverage; they are not independently established as the source of the measured column jump.

**4. Folders do less vertical work on width-only changes.**

`geometry_for_current_layout()` reuses folder section Y positions while column count and row height are unchanged. Masonry recalculates every photo and calls `index_masonry_items()`, which sorts the entire layout and rebuilds its indexes. This is additional work that scales with library size; its contribution to dropped frames remains to be profiled separately.

### Existing test results

- `cargo test masonry -- --nocapture`: **7 passed, 4 display tests ignored**.
- `masonry_resize_keeps_the_visible_photo_anchor`: **passed** when explicitly enabled on the display.
- `masonry_virtualizes_twenty_thousand_photos_and_resizes`: **passed** when explicitly enabled.
- `photo_wall_width_change_scales_without_refit_or_scroll_jump`: **passed** when explicitly enabled.
- `pure_scaling_keeps_deep_section_row_visible_in_resize_frames`: **passed** when explicitly enabled.

Each GTK test ran in a separate process with `--ignored --test-threads=1`. Existing compiler warnings were present. The Masonry anchor test checks one photo; the virtualization test waits for settling. Neither asserts coherent movement of all visible columns during a sequence of tiny width changes.

## Fix checklist

### Task 1: Apply Photo Wall's ideal-height and shared-edge method to Masonry

**Files:** `src/grid/masonry_layout.rs`, `src/grid/photo_wall_layout.rs`; tests in their existing test modules.

**Interfaces:** Keep `calculate_masonry(ratios, sections, viewport, preferred_width)` and `resize_masonry(&mut self, viewport: i32, anchor_y: f64) -> Option<f64>`. The latter produces geometry plus the mapped logical scroll point without changing column membership.

- [x] Add `masonry_one_pixel_resize_moves_all_visible_columns_coherently`. Use the diagnostic's 3,086 and 20,000 photo fixtures, with scroll depths 0%, 40%, and 90% and a 600 px visible band. Record every initially visible photo by ID. For 997 → 998, require maximum viewport-relative top movement ≤ 3 px, and unchanged column membership. Verify failure on current main (141/907 px at 40%, 315/2,045 px at 90%).
- [x] Add `masonry_repeated_resize_uses_ideal_edges_without_deep_drift`. Exercise every width 997…1103, reverse that sequence, and return to 997 at several deep positions. Require original column order, final geometry within 1 px, and viewport mapping within 2 px. Test multiple sections with 70 px headers and both portrait/landscape ratios.
- [x] Derive a common ideal column width `(content_width - (columns - 1) * 8) / columns` as floating point. Use it for vertical aspect calculations in both initial packing and subsequent width-only scaling. Integer horizontal pixel remainder allocation must not drive independent column height scaling.
- [x] In each section and column, accumulate ideal tile heights plus fixed gaps. Round cumulative top/bottom edges and derive allocated height from their difference, as Photo Wall does. Keep unrounded values or derive them afresh from stored ratios and stable column order; do not scale previously rounded heights.
- [x] Adapt aspect assertions to Photo Wall's existing integer-edge bound: `abs(width - height * ratio) <= 1 + ratio`. Keep exact whole-pixel bounds, 8 px gaps, positive allocations, full-width coverage, and nonoverlap tests. Tiny widths retain the existing safe repartition fallback when positive-width columns cannot fit.
- [x] Map the same logical point within the anchor tile using its fractional vertical position, with explicit section-header/end handling. Test points inside tall images, inside headers, at section ends, and at scroll limits.
- [x] Run `cargo test masonry_layout`, `cargo test masonry_resize_tests`, and the standalone diagnostic. Require the new tests to pass and the diagnostic's visible movement to fall to ≤ 3 px for both sizes.

### Task 2: Preserve membership after window resizing stops

**Files:** `src/grid/photo_wall.rs`, `src/grid/sectioned_folder.rs`, `src/grid/photo_wall_tests.rs`.

**Interfaces:** Width-only updates continue through `calculate_wall_geometry()` and `scale_wall_geometry()`. Explicit model/zoom invalidation remains the way to request a fresh partition.

- [x] Add `masonry_width_change_does_not_repack_after_idle`. Capture column membership and geometry generation, resize by one pixel, then observe GTK frames for at least 400 ms. Require unchanged membership and no idle geometry rebuild. Verify that it fails on the current 150 ms settle path.
- [x] Remove `masonry_resize_pending`, `poll_masonry_resize()`, its tick invocation, and its reset sites. Update the old anchor test, which currently directly sets and polls that timer, to assert the new width-only contract.
- [x] Retain a fixed column count across ordinary resizing, as Photo Wall retains rows. Repartition only for zoom, actual model/section/layout invalidation, initial allocation, or the impossible-width fallback. Saved zoom must not change during resize.
- [ ] Add coverage for resize → zoom, resize → source replacement, and Masonry → Grid → Masonry. Assert the new geometry uses current data, selection IDs survive, and there is no delayed callback restoring stale layout.
- [ ] Run the idle test and existing Masonry anchor, virtualization, and lightbox return tests individually with `--ignored --test-threads=1`.

### Task 3: Verify the same-frame geometry/scroll contract already used by Photo Wall

**Files:** `src/grid/photo_wall_tests.rs`; adjust `src/grid/photo_wall.rs` or `src/grid/sectioned_folder.rs` only if the new test exposes a synchronization defect. Inspect `src/window/build.rs` width observation without adding a second width owner.

**Interfaces:** Reuse `adjustment.configure()` in `scale_wall_geometry()` and the allocated-width handler in `attach_scroll()`. Consume the mapped point produced by Task 1.

- [x] Adapt `pure_scaling_keeps_deep_section_row_visible_in_resize_frames` to Masonry with mixed ratios and 3,086/20,000 photos. First display and settle the window, set its actual vertical adjustment to 40% or 90% depth, wait for the deep viewport to paint, then resize. Sample `connect_after_paint`, geometry width, adjustment value, root translation, and bounds of all tracked visible tiles across both one-pixel and large width changes. Repeat each depth from a fresh initial layout so earlier resize history does not conceal the problem.
- [x] Require geometry width to equal the allocated scroller width, root translation and logical viewport mapping error < 2 px, tile allocation error < 2 px, stable column membership, and unchanged original-file read count on every sampled frame. Also require no column-dependent jumps beyond the geometry test's 3 px small-step bound.
- [ ] Exercise vertical-only resize, simultaneous width/height changes, sidebar reveal/hide, and hover freeze behavior. Preserve existing Photo Wall behavior; verify Masonry against its actual allocated width rather than introducing synthetic/frozen widths.
- [ ] If the test fails, use existing `PICASA_TRACE` scroll-map/allocation logs to identify the first inconsistent frame. Preserve Photo Wall's publish-geometry → configure-adjustment → synchronize-viewport ordering; fix the demonstrated boundary rather than adding a delay.
- [x] Run both existing Photo Wall resize tests and the new Masonry frame test in separate processes. Run `cargo test` once after implementation, recording any unrelated baseline failures separately.

### Task 4: Measure and validate the visible result

**Files:** Update this investigation with measured results; add temporary scoped tracing only if needed.

- [ ] Compare native drag behavior in Library/Photos Masonry, Photo Wall, and Folders on the same cached library at top, 40%, and near bottom, with sidebar shown/hidden and several zoom levels. Include fast reversals and short pauses during dragging.
- [ ] Capture refresh, layout, index rebuilding, tile bind counts, and paint intervals. Distinguish geometry discontinuities from work taking longer than the display's frame budget. The standalone probe is not a frame-time benchmark.
- [ ] If dropped frames remain after Tasks 1–3, target the measured full-library sort/index or duplicate refresh work. Do not add this optimization before evidence identifies it as a remaining bottleneck.
- [x] Run `git diff --check` and formatting checks scoped to changed Rust files; report repository-wide pre-existing formatting debt separately.
- [ ] Record the exact tested commit, source sizes, display setup, test outcomes, and any remaining limitation before declaring the resize issue fixed.

## Acceptance

The automated checks now cover coherent all-column motion during live resizing and stable membership after release. Passing an anchor-only test or observing a settled screenshot is insufficient. Validation with a real cached photo library remains a manual check.

## Implementation results (2026-10-06)

- Initial and resized Masonry now use one floating-point ideal width for all columns, with cumulative edge rounding and fixed 8 px gaps. Horizontal pixel remainder allocation no longer controls cumulative vertical scale.
- Viewport mapping uses the same reference column and a fractional point within the image, with separate header, gap, section-tail and end handling. Repeated resizing returns to the original geometry and logical viewport position.
- Removed the 150 ms idle repack entirely. Width changes preserve membership; zoom/model/layout invalidation still requests a fresh partition.
- Independent review found a final-column tail incorrectly classified as an inter-photo gap. A regression reproduced boundary crossing and round-trip drift; the corrected mapping passes both narrow and normal-window fixtures.
- The production-engine diagnostic now reports **0–1 px** Masonry visible movement for 3,086 and 20,000 photos at 0%, 40%, and 90% depth, compared with the original 5–2,045 px. Its reported repack counts are hypothetical fresh calculations; the application no longer invokes that idle repack.
- **28 standalone pure-engine tests pass**, including Photo Wall tests.
- GTK checks pass: deep-scroll after-paint all-column synchronization; no idle repack; logical viewport anchor; 20,000-photo virtualization; Photo Wall width scaling, deep-section frame synchronization, and sidebar width freeze. Tests run individually on the real desktop display.
- The deep GTK test first scrolls to 40% and 90%, then resizes narrower/wider and vertically, checking every frame's viewport translation, tile bounds, complete visible coverage, stable column membership, selection, and absence of original-file reads.
- Full default suite: **530 passed, 1 failed, 68 ignored**. The failure is the existing history date test expecting “Aug 2025” for a rolling 400-day offset that now resolves to “Sep 2025”; unchanged main reproduced it. Earlier sandbox cache/network/display failures were environmental and disappeared after access was restored and tests used a writable cache.
- Remaining manual checks: fast native dragging with the user's real cached library, measured frame-time profiling if visible pauses remain, and exhaustive resize → zoom/source/layout/sidebar combinations.

- The ignored Masonry lightbox-return centering test fails on both this branch and unchanged main; this remains an unresolved validation limitation.
- Final branch verification: `cargo build` passes; default tests with the known history-date failure excluded pass (**530 passed, 68 ignored, 1 filtered out**). `git diff --check` passes.
