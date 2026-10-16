# Photo Wall verification — rc8

Photo Wall is implemented using the existing recycled SectionedFolderView, photo objects, selection model, display queue and RAM thumbnail cache. Grid retains its existing GtkGridView and folder geometry. The bottom view control offers Grid, Photo Wall and Create Collage…; layout preference persists independently of the source. Shared zoom controls set the Wall target row height.

Implementation commits: `cd68f7a` (reviewed plan), `ef28b3e` (geometry), `0aceed5` (oriented metadata), `1eb3bf7` (recycled rendering), `8c0f716` (viewport/anchors), `45ec2a5` (view menu/zoom). The following final commit contains review fixes, regression checks and this report. All commits are local on rc8.

## Verification results

- Baseline: cargo check passed; cargo test: 417 passed, 33 ignored. Repository-wide cargo fmt --check already failed.
- Final cargo check passed. Final cargo test: **431 passed, 38 ignored**.
- All **five Photo Wall GTK tests passed**, run in separate processes with `--ignored --test-threads=1` on DISPLAY=:0. GTK initialization requires the same thread, so these tests should not be run together using the default Rust test runner.
- Existing GTK tests `sectioned_reveal_retries_after_the_folder_scroller_is_mapped` and `section_headings_sit_above_the_first_tile_column` passed.
- Existing `strip_zoom_resize_and_retarget_keep_photos_and_headers_together` failed to observe its required wrapping transition. The same failure was reproduced on pre-feature commit `85d3c25` using an extracted baseline checkout with the same display and sample images. This is an existing failure, left unchanged.
- `rustfmt --check` passes for the three new Rust modules. Changed hunks in existing modules were formatted while preserving unchanged mixed CRLF line endings. Repository-wide cargo fmt --check still reports pre-existing formatting debt, including unrelated collage and theme files.
- git diff --check passed.
- Inspected an actual GTK snapshot of cached, mixed portrait/landscape thumbnails and folder headings. The synthetic offline objects intentionally display unavailable indicators. Snapshot: `/tmp/pic-photo-wall.png` (local test artifact, not a tracked asset).

## Acceptance mapping

| Original checklist item | Evidence and scope |
| --- | --- |
| Existing Grid works | Grid ↔ Wall test retains model/selection and Grid context; existing reveal/header GTK checks pass. Animation check caveat below. |
| Select Wall from view control | InfoBar radio menu test and window wiring; collage callback retained and tested. |
| Justified rows | Pure completed-row, preferred-height and large-collection tests. |
| Preserve original aspect ratios | Catalog dimensions plus user rotation; extreme/missing ratio tests. Cropped cached presentations use Contain. |
| Respect EXIF orientation | Scanner’s existing normalization extracted and tested for orientations 1–8; already-normalized dimensions tested. Real rotated NEF/legacy catalog validation outstanding. |
| Fill available width | Completed-row tests, including extreme ratios and tiny widths. |
| Avoid final-row stretching | Final row is left aligned, with height at most the target; pure tests. |
| Zoom slider changes row height | Slider/key/wheel route to shared immediate Wall reflow; GTK zoom/anchor checks. Exact slider UI endpoints not separately exercised. |
| Resize correctly | Production width callback included in GTK regression; deep-scrolled anchor retained within 3 px after narrowing. |
| Preserve folder headers | Two-folder surface and progressive batch tests; existing heading test. |
| Never cross folder boundaries | Pure section tests and all rows checked in 14,361-photo GTK collection. |
| Selection works | Same MultiSelection and PhotoObjects, multiple selected IDs retained on switch; existing rectangle/range selection handlers reused. Exhaustive input shortcuts not manually exercised. |
| Lightbox/open works | Recycled tile double-click calls existing activation callback with correct photo ID; return/reveal routes through Wall. Full editor/lightbox round trip not manually exercised. |
| Reuse thumbnail caching | In-memory cache paintables bound to actual recycled widgets; no additional cache subsystem. |
| Reuse/extend virtualization | Existing live tile maps and pools; binary visible-row queries. |
| 10,000+ photos without 10,000+ widgets | 14,361-photo viewport remains below 300 live tiles in tested normal viewport; 10,000-photo ultrawide portrait test legitimately exceeds 180 while covering its viewport. |
| No original decoding during layout | Pure engine has no I/O; GTK-thread read/materialize probe stays zero for tested scroll/zoom/resize/switch operations. Worker activity is separate; exhaustive worker deduplication was not measured. |
| Switching does not change source | Switch reuses exact model/objects and selected IDs; no source loading/navigation callbacks in mode change. Source-specific manual checks remain below. |
| cargo check passes | Passed. |
| Existing tests pass | Default suite passed; separately enabled display animation caveat below. |

## Review fixes and rulings

The independent reviewer returned “with fixes.” Each accepted defect had a failing regression followed by a passing check:

1. Pending Grid zoom survived a Wall switch. Cancel pending sources, reset visual/anchor animation state, and guard deferred callbacks by a layout-switch generation. Preserve the latest requested shared zoom level.
2. Progressive replacement published new photos with stale folder ranges. Publish matching ranges before ListStore notifications on Wall replacement, reorder and append paths.
3. Gaps could exceed the entire image budget in extremely narrow viewports. Stop row accumulation before accepting such a gap; guarantee a positive singleton row. Reclassified from minor to important because viewport containment is a geometry invariant.
4. Gallery’s width observer refreshed geometry before the scroller observer captured an anchor. Capture before refresh, and skip duplicate captures when geometry already matches the new width.

Design rulings retained from the plan:

- Use the existing rc8 checkout because the user requested incremental commits there.
- The 0.65–1.20 target-height band guides row membership; extreme ratios may require shorter rows to preserve aspect ratio and width bounds.
- Stored dimensions are post-EXIF and pre-user-rotation. Existing normalized dimensions must not be swapped again.
- Allocation preparation and anchor restoration are separate: wait for the destination surface allocation, then restore with geometry/model generation guards.
- Layout switching cancels asynchronous Grid zoom without losing the latest shared zoom preference.
- Legacy metadata repair belongs to scanner maintenance, with provenance/persistence/retry decisions; Wall does not read originals or schedule repair jobs.

## Remaining validation and declined claims

No actual EXIF-rotated NEF fixture was available. Normalized synthetic dimensions and EXIF normalization calculations pass, but legacy sensor-oriented catalog dimensions can still give incorrect ratios until separately refreshed. This original acceptance criterion is only partially validated.

All gallery sources use the same collection/surface adapter in code. Manual application checks against real All Photos, Favourites, Recently Added, History, albums, individual/parent folders and Network Shares were not performed. Sidebar toggles, preference restoration across process restarts, every keyboard/mouse selection combination, RAW/JPEG header interactions and a full lightbox/editor round trip remain manual validation items. These are not reported as passed by the synthetic collection tests.

Hardware-independent smoothness, filesystem syscall tracing, worker request deduplication and exhaustive source-specific timing were not established. Test runtimes are environment-specific: five display tests completed individually in approximately 0.3–2.2 seconds each. Layout input changes rebuild geometry; ordinary scrolling was verified not to increment geometry generation. No new drag/drop or folder-collapse functionality was added.
