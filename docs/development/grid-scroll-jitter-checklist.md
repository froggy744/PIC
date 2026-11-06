# Grid scrolling jitter investigation

Status: the Folder keyboard/wheel interruption conflict is fixed in commit
`8486e08b`. General wheel-scroll jitter in Photo Grid and Folder view remains
unresolved. The temporary `PIC_NATIVE_SCROLL` diagnostic toggle has been removed;
PIC retains its existing custom wheel scrolling and native touchpad handling.
The sections below preserve the investigation history and its evidence limits.

## Scope and test matrix

Use the same catalog, viewport, renderer, thumbnail quality, and wheel input
for all six cases. Start with small (100 px), default (160 px when no saved
preference exists), and large (300 px) tile widths. The application has eight
canonical zoom levels: 100, 117, 137, 160, 187, 219, 256, and 300 px; saved
settings can select another width until zoom input snaps to the ladder. Record
the actual width, viewport dimensions, column count, photo count, thumbnail
quality, cache state, renderer, display scaling and refresh rate in each run.
Avoid interpreting a resize or zoom animation as ordinary scrolling jitter.

| Grid layout | 100 px | 160 px | 300 px |
| --- | --- | --- | --- |
| Photo grid | [ ] | [ ] | [ ] |
| Folder view | [ ] | [ ] | [ ] |

For each cell:

- [ ] Let thumbnail generation and other background work settle; note any work still running.
- [ ] Scroll several single detents down and up, then sustained detents and an immediate direction reversal, away from the ends of the scroll range.
- [ ] Record whether the symptom is uneven frame delivery, backward/oscillating movement, delayed thumbnails, or a combination, and whether it also occurs on warm revisits.
- [ ] Capture timestamped input, adjustment movement and frame intervals with minimal instrumentation; record sample duration and dropped/long frames relative to the actual display refresh rate.
- [ ] Repeat the same movement after changing just one variable when testing a hypothesis.

## Investigation checklist

- [x] Check worktree and recent history. Investigation began on clean `main` at `e874db3b`.
- [x] Map Grid layout and its scroll paths; see findings below.
- [x] Identify existing traces and their limitations.
- [ ] Reproduce the six cases using physical mouse-wheel input on the affected system; capture baseline evidence.
- [ ] Distinguish frame stalls from adjustment reversals or thumbnail presentation delays.
- [ ] Compare recent thumbnail changes against a known good revision under equivalent conditions if the symptom implicates them. The latest commits `8ac9ad10` and `e874db3b` change thumbnail handling; temporal proximity alone does not establish causation.
- [ ] State and test one specific root-cause hypothesis with the smallest useful experiment.
- [ ] Create a focused regression test where feasible, fix the confirmed cause, and run the appropriate targeted checks.
- [ ] Repeat the six cases and document measured results, remaining limitations, and branch state.

## Code map and evidence

- `src/grid.rs` defines tile dimensions and the zoom ladder. The source-level
  shape reference is 180×120; the window's unsaved starting width is 160 px,
  with height scaled proportionally. A saved width takes precedence.
- `src/grid/view.rs` calculates Grid columns as
  `floor(max(width - 48, 200) / (tile_width + 30))`, clamped to 1–12. The
  non-Folder grid is a `GtkGridView` directly inside its scroller. Width-only
  changes with unchanged columns avoid explicit relayout; zoom and column
  changes do require geometry updates.
- `src/window/build.rs` attaches the default Folder view to its sectioned
  surface. `src/grid/sectioned_folder.rs` refreshes it on adjustment changes;
  section geometry is reused when columns, row height and section count have
  not changed. `PICASA_LEGACY_FOLDER_LIST=1` selects a legacy comparison path;
  do not assume that path represents the default.
- `src/window/navigation.rs` handles discrete wheel events with a 120 px
  impulse. The photo grid uses a damped spring; Folder uses a separate
  remaining-distance ease and rounds steps to pixels. Touchpad surface events
  follow a different path. These differences are observations, not a diagnosis.
- `PIC_SCROLL_TRACE=1` logs input source, deltas, unit and modifiers.
  `PIC_SCROLL_FRAME_TRACE=1` logs GTK after-paint intervals while the **photo
  grid** adjustment changes and for 250 ms afterwards; it does not monitor the
  Folder adjustment and does not measure compositor presentation or display
  scanout. Neither trace alone identifies the responsible callback.
- `docs/releases/v1.0.3-checklist.md` records an earlier native real-catalog
  sample: 932 intervals, median 13.410 ms, p95 21.266 ms, maximum 51.809 ms,
  with thumbnail recovery active for part of the session. It predates the last
  two thumbnail commits and does not isolate this matrix, so it is context,
  not a current baseline or proof of a regression cause.
- The GUI harness's existing Gallery scrolling case confirms that visible
  content changes; it does not measure smoothness. The synthetic Photo Wall
  timing probe uses a different layout and is not a substitute for this Grid
  matrix. Xvfb/Cairo results cannot establish native hardware smoothness.

## Results

2026-11-06: `cargo test --locked grid::zoom_ladder_tests -- --test-threads=1`
passed all five tests. These cover the zoom ladder and animation helpers; they
do not exercise wheel scrolling or establish smoothness.

2026-11-06 interactive investigation, debug build on the user's native desktop:

- The user reported jitter in both views at all zoom levels, with maximum zoom
  feeling better. A mixed first capture recorded 165 photo-grid and 100 Folder
  wheel events; it did not label each zoom separately.
- The photo-grid small-tile capture recorded 433 GTK after-paint intervals,
  median 12.727 ms, p95 32.469 ms, maximum 133.151 ms. The large-tile segment
  recorded 212 intervals, median 28.622 ms, p95 108.907 ms, maximum 272.778 ms.
  These are unequal, uncontrolled segments that can include zoom transitions;
  they do not explain why large tiles felt smoother and are not a comparison of
  physical presentation times.
- Temporary opt-in position instrumentation extended the frame trace to the
  Folder adjustment. In one isolated downward Folder run, all 44 wheel events
  were downward and 205 recorded adjustment values were monotonic. Its 271
  after-paint intervals had median 9.678 ms, p95 29.279 ms and maximum
  82.246 ms. This single run does not exclude jitter in other conditions.
- In a photo-grid capture, 857 fractional adjustment values were immediately
  followed by their floored value. A temporary experiment quantizing GridView
  spring requests eliminated fractional adjustment values in its capture
  (883 recorded values, none fractional), but the user reported the result
  felt worse. The experiment and temporary instrumentation were removed.
- On returning to the original code without logging, the user said scrolling
  felt better but was uncertain. Synchronous per-frame/position logging can
  perturb the workload, so these logged intervals are diagnostic observations,
  not an undisturbed performance baseline. Raw temporary logs are at
  `/tmp/pic-grid-scroll-trace.log`, `/tmp/pic-grid-scroll-positions.log` and
  `/tmp/pic-grid-scroll-quantized.log` for this session.
- `cargo test --locked grid::section_lookup_tests -- --test-threads=1` passed
  five tests with three display-dependent tests ignored. These check section
  lookup and geometry, not visual smoothness. An initial incorrect module
  filter selected zero tests and was corrected.

The root cause remains unverified. The six-case matrix still needs controlled,
low-overhead automated measurement and symptom attribution before a fix.

## Native wheel toggle and automated functional comparison

During the comparison, `PIC_NATIVE_SCROLL=1` forwarded ordinary wheel events to
GTK in the photo and Folder scrollers. Unset preserved PIC's custom animation.
Ctrl+wheel zoom and touchpad handling retained their existing paths.
`cargo check --locked` passed. The temporary toggle was subsequently removed.
The user preferred custom scrolling in the manual comparison; this does not
establish the cause of the original jitter.

After the user authorized automation and requested all three layouts, an
external driver used AT-SPI actions and zoom values with XWayland wheel input.
No screenshots, new application instrumentation, or timing benchmarks were used.
The existing native automation approach supplied the accessibility pattern;
the screenshot-based gallery check and synthetic Wall timing probes were not
used as substitutes for this toggle comparison.

| Collection | Layouts | Widths | Custom | Native GTK |
| --- | --- | --- | --- | --- |
| All Photos | Grid, Photo Wall, Masonry | 100, 160, 300 px | 9/9 passed | 9/9 passed |
| Folder `/` | Grid, Photo Wall, Masonry | 100, 160, 300 px | 9/9 passed | 9/9 passed |

Each case verified the requested zoom slider index, sent eight downward wheel
detents and eight upward detents, and checked the accessible scroll value moved
down then up. All 36 functional cases passed. An initial driver attempt stopped
before completing any case because the accessibility tree temporarily lacked a
visible scrollbar during a layout change; waiting for the scrollbar resolved it.

Raw results and the driver are retained in the gitignored directory
`reports/gui-tests/scroll-toggle-comparison/` (`custom-results.json`,
`native-results.json`, `driver.py`). Layouts and zoom were restored by the driver;
the test app and private input daemon were stopped. These checks validate wheel
operation across the matrix, not animation smoothness or native Wayland
presentation. The original jitter remains unresolved.

## Targeted animation-path review (before the regression test)

Reviewed only the custom wheel handlers and their adjustment/frame callbacks.
No verified cause of the reported steady wheel jitter was established, so no
scrolling fix or local commit was made. `cargo check --locked` passed.

- Both custom handlers consume ordinary wheel events after queuing their own
  motion; their touchpad branches forward events. No duplicate native wheel
  application was found on the custom path.
- Both tick callbacks use `frame_time()` with the same elapsed-time clamp.
  The spring and Folder easing remain separate; neither was replaced or tuned.
- Folder's adjustment callback rounds fractional inputs, but its custom wheel
  tick already requests integral positions. That callback alone does not prove
  repeated writes during ordinary custom wheel motion.
- One specific potential conflict remains: the sectioned surface's keyboard
  centering animation writes the adjustment for 190 ms, while the Folder wheel
  handler does not cancel that animation. This could affect wheel input that
  interrupts keyboard navigation. It has not been linked to the reported jitter
  or shown to explain ordinary Photo Grid scrolling.

Precise blocker: there is no captured causal connection between a visible jitter
episode and competing adjustment writers; the existing functional tests do not
exercise overlap with keyboard centering. Next experiment: one focused GTK
regression test that starts sectioned keyboard centering, delivers a downward
wheel event before its 190 ms animation ends, and checks whether the old centering
animation subsequently pulls the adjustment backwards. No new logging or broad
matrix was needed for that experiment. Its result is recorded below.

## Verified Folder keyboard/wheel conflict

The focused display-dependent test
`grid::section_lookup_tests::folder_wheel_interrupts_keyboard_centering_without_backward_motion`
starts upward keyboard centering, interrupts immediately with a downward wheel
signal through the installed Folder controller, then observes adjustment changes.
It failed before the fix because centering moved the viewport backwards, and
passed after cancelling the centering animation on Folder wheel input. It also
checks that the wheel still advances more than 100 px of its 120 px impulse.
The wheel easing, sensitivity, Photo Grid behavior and pending-photo-restore
handling are unchanged.

Verification: the new test alone passed under Xvfb/Cairo with
`--ignored --test-threads=1`; `cargo check --locked` passed. This establishes a
specific Folder interruption bug, not a complete explanation of the longstanding
jitter across all views.

Finalisation: removed the uncommitted native-scroll toggle while preserving
`8486e08b`; `src/window/navigation.rs` now matches that verified commit. After
removal, `cargo check --locked` and the focused regression test under Xvfb/Cairo
both passed again. No additional scrolling changes or broad tests were made.

## Keyboard/wheel handoff verified across layouts and photo mode

Extended the focused test coverage to both ungrouped Photo mode and Folder
mode, each in Grid, Photo Wall and Masonry at 100, 160 and 300 px. These are
18 keyboard-reveal/wheel-interruption cases, distinct from the earlier 36
ordinary wheel checks. Fake catalog objects and Xvfb/Cairo were used; no
screenshots, GUI harness, performance benchmark or application tracing was used.

| Mode and layout | Cases | Result |
| --- | --- | --- |
| Photo Grid (GTK GridView) | 100, 160, 300 px | Passed keyboard reveal followed by downward wheel input |
| Photo Wall and Masonry, Photo mode | Both layouts × 3 sizes | Passed |
| Grid, Photo Wall and Masonry, Folder mode | All layouts × 3 sizes | Passed |

The sectioned paths use `smooth_keep_index_in_center_zone`, the keyboard route
exercised by the original regression. Wall/Masonry reveal the photo immediately
rather than running the Grid-specific 190 ms centering animation. Photo Grid
uses `select_photo`, the reveal path used by browser keyboard navigation, and
its own spring wheel controller. Tests check for a backward reveal after the
wheel interrupt and that more than 100 px of the 120 px impulse is retained.

The initial Photo Grid assertion flagged the already-known whole-pixel GTK
corrections as backwards. Its focused check now tolerates at most 1 px per
correction, without quantizing or changing scrolling itself. Measured largest
backsteps were 0.966–1.000 px; the final downward displacement was 108 px.
The 15 sectioned cases had zero backsteps and advanced 120 px. These results
verify this particular keyboard/wheel handoff; they do not establish general
visual smoothness or eliminate the known fractional correction behavior.

Validation: `photo_grid_keyboard_wheel_handoff` and
`keyboard_wheel_handoff_in_virtual_photo_and_folder_layouts` passed under
Xvfb/Cairo with `--ignored --test-threads=1`; `cargo check --locked` passed.
No further scrolling-behavior change was needed for this verification.

## Photo Grid fractional spring feedback

A focused test using a real GTK GridView with fixed-size synthetic cells
confirmed repeated fractional-request → floored-adjustment pairs. With the
original spring, one 120 px downward wheel impulse travelled only 108 px after
settling. Each tick restarted integration from GTK's corrected position, losing
the fractional progress; the existing near-pin guard then ended the animation
short. This is a verified feedback defect, not proof of the general visual
jitter's sole cause.

The minimal correction retains the previous requested position for integration
when GridView's actual value differs by at most 1 px. Larger changes still use
GTK's actual position and retain the existing external-navigation cancellation.
A fresh wheel animation starts from the current GTK value. Requests are not
rounded, and spring constants, wheel sensitivity, touchpads, Folder scrolling
and Albums home are unchanged. GTK can still correct the displayed adjustment;
those small corrections no longer discard the spring's fractional progress.

`window::grid_wheel_fraction_tests::grid_spring_preserves_wheel_distance_across_pixel_corrections`
failed before the fix (108 px), then passed: repeated real GTK corrections still
occurred, while the settled wheel distance was within 1 px of the intended
120 px. The test ran alone under Xvfb/Cairo with `--ignored --test-threads=1`;
`cargo check --locked` passed. No keyboard-conflict tests, benchmarks, screenshots,
GUI suites or extensive application logging were used for this investigation.

Manual acceptance: launched the modified app against the existing library with
`GDK_BACKEND=wayland GSK_RENDERER=vulkan`, with scroll/stage tracing disabled.
The user was asked to compare Photo Grid wheel scrolling at 100, 160 and 300 px
and chose to keep the change: it felt acceptable, but improvement was uncertain.
This is acceptance of the tested correction, not confirmation that general
visible jitter is eliminated. No spring constants or sensitivity were changed.

## Fast scrollbar release: correct-thumbnail blanking

The user reported some thumbnails flickering when a fast scrollbar-thumb drag
stops. The Photo Grid settle callback ends scrub mode and calls
`refresh_visible_grid_tiles`. That function unconditionally unloaded visible
images before loading them again. If a correct visible paintable survived in its
widget but its shared RAM-cache entry had been evicted during the jump, the
settle pass cleared it and queued an asynchronous reload, producing a blank
interval. This is a separately verified settle-path defect; it has not been
attributed to the fractional spring correction.

The refresh now retains fully loaded thumbnails whose applied presentation key
matches the current photo and whose thumbnail remains available. Stale recycled
images and missing/changed presentations still follow the existing unload/load
path. No wheel behavior, cache sizing, image quality or decode logic changed.

The focused test
`grid_scrub_settle_preserves_loaded_thumbnail_after_ram_eviction` failed before
the fix because the settle refresh cleared a correct paintable. It passed after
the fix, checking both that the correct paintable never becomes empty and that
a recycled image belonging to a different photo is still cleared at settle.
The test ran alone under Xvfb/Cairo with `--ignored --test-threads=1`;
`cargo check --locked` passed. After launching the modified app on native
Wayland with Vulkan requested and tracing disabled, the user repeated the fast
scrollbar-thumb drag and confirmed: "Flicker is gone."
