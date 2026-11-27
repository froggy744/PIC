# RAW editor performance investigation plan

> Use the existing traces and focused tests to establish the baseline. Record the evidence before choosing a product change.

**Goal** Measure RAW editor opening and warm slider response on PIC's current code.

**Baseline** The branch starts from `ui/search-width-40`. Record the tested commit, binary profile, camera model, source dimensions, file locality, viewport and device scale, GL renderer, and exact recipe with every trace.

## Prior attempts

- [GPU V1 benchmark](../../../reports/benchmarks/pic-v1-gpu-final/README.md) measured 3.5 ms GPU versus 351 ms CPU for NEF Exposure + Sharpen after decode and upload. The source was the embedded JPEG.
- [GPU recovery](../../livegpu-recovery-handoff.md) removed the fixed 960 px draft. [GPU acceptance](../../experimental-live-gpu.md) recorded 2.3–6.4 s CPU renders for expensive fallback recipes on a 24 MP input. Treat these as historical observations.
- [Render pipeline status](../../architecture/STATUS.md#render-pipeline-project-started-2026-11-27-branch-refactorrender-pipeline) records eight D800 NEFs exporting at 2048 px in 8.0 s with half-size develop versus 17.4 s with full develop, with 5.6% lower edge contrast. This path is export-only.
- Commit `14f46267` uses embedded previews for Fit browsing and native detail for sensor develop. Commit `7a46ea6d` already presents the viewer frame or gallery tile during editor opening. Commit `3ef82830` keeps partial-strength automatic adjustment on CPU for parity.

## 1. Open a RAW in the editor

- [ ] Use a local NEF that exhibits the delay, a disposable library, and an existing debug binary built from the recorded revision. Run from the repo root with `PICASA_TRACE=1`.
- [ ] Record the time to the opening stand-in separately from the time to the first completed editor frame. Open once from the gallery and once again after closing the editor. Record cache state from the logs.
- [ ] Correlate `PIC_EDIT_OPEN`, `PIC_DECODE stage=nef_scene_v2`, `PIC_EDIT source_base`, and `PIC_EDIT_GPU` upload and presentation markers. The existing scene-develop timer includes RAW unpack. If decode dominates and unpack cannot be isolated from existing logs, add diagnostic timing around `decoder.raw_image` and `nef_scene::develop` in `src/thumbnail/viewer.rs` before selecting an optimization.
- [ ] Check the Fit to 1:1 transition and record any crop, orientation, highlight or detail differences. Preserve original files and record their hashes before and after the checks.

## 2. Measure warm slider response

- [ ] Wait for the first completed frame. Change Exposure with an otherwise supported recipe, then use an adjustment that the current trace confirms fell back to CPU. Try partial-strength automatic correction if available. Record the actual route and fallback reason.
- [ ] Record slider input to frame presentation, source cache state, and `PIC_EDIT adjustment_buffer` tone and job timings. Observe drag and release, then Undo and Reset. If the log lacks an input or presentation timestamp, report only the measured intervals.
- [ ] Attribute the delay to decode, upload, render or presentation using matching events for the same request. A cache miss and a slow cached adjustment require different changes.

## 3. Run focused regressions and decide from the data

- [ ] Run `cargo test --locked embedded_nef_fit_never_reuses_sensor_cache_frame -- --test-threads=1`.
- [ ] Run `cargo test --locked edit::editor::tests:: -- --test-threads=1`.
- [ ] With an appropriate real NEF fixture, run the ignored `real_nef_sensor_render_uses_full_source_and_camera_look_reference` test with `PIC_TEST_RAW` set to its local path and `-- --ignored --test-threads=1` after the test filter. Record a skip if the fixture is unavailable.
- [ ] If scene develop dominates opening, evaluate half-size develop in a separate proposed change. Compare both paths with the same source and viewport. Check fine detail, crop, colour and highlight rendering, and Fit to 1:1 continuity. If a cached CPU render dominates slider latency, review its expensive effects and preserve the full-quality release frame and existing geometry. Revisit a viewport-sized draft only with evidence addressing the earlier fixed-draft failure.
- [ ] Save timings, verified renderer identity and remaining gaps with the test results. Keep release builds and benchmark loops subject to the repository's approval rules.

## Initial test results

- The NEF Fit cache regression passed.
- The editor preview module passed six tests and skipped one real-photo test.
- The native NEF colour regression reached its colour assertion and failed on an existing Nikon Z 5 fixture under `reports/atspi-probe/`. Red-channel mean absolute delta was 34.13 against a limit of 25.0. No product code changed on this branch. Carry this fixture result into any future before-and-after comparison.
- Live editor traces were captured on the investigation branch (`36d3760f`)
  and `main` (`6fa957c0`) using byte-identical copies of the Z 5 NEF.
  The branches differed only in header search width. Both sessions felt the
  same to the operator.

| Live editor measurement | Investigation branch | `main` |
| --- | ---: | ---: |
| RAW decode and develop | 774 ms | 768 ms |
| GPU texture upload | 16.6 ms | 15.6 ms |
| Median GPU queue to GTK after-paint | 2.3 ms | 2.5 ms |

The GPU frame timing starts at render queueing. Edit click to first correct
RAW frame was not measured. The traces are in
`/tmp/pic-raw-trace.RuvUo8/pic.log` and
`/tmp/pic-raw-main.Llg08K/pic.log`.

## Decode stage measurements

Three fresh test processes used the existing native NEF regression to exercise
the same full-resolution develop function as the editor. The Nikon Z 5 fixture
was 6016 × 4016. The filesystem cache was not flushed. The test profile matched
the earlier debug build's optimization settings. These intervals exclude GPU
upload, GTK paint, and the later viewer RGBA8 conversion.

| Stage | Median (ms) | Range (ms) |
| --- | ---: | ---: |
| rawler decoder setup | 36 | 35–36 |
| Sensor unpack | 148 | 148–154 |
| rawler preparation and demosaic | 437 | 420–439 |
| Colour, tone, RGB16 output and remaining overhead | 193 | 188–194 |
| Total traced decode and develop | 813 | 798–816 |

The colour and output interval is the total minus the other measured stages.
The preparation and demosaic timer covers one rawler call that also clones
the RAW image, rescales sensor samples and handles cropping. The earlier
editor trace measured RGBA8 conversion separately at about 62 ms.

The nine scene-develop unit tests passed. The fixture still fails the same
baseline colour assertion at 34.13 red-channel mean absolute difference.
If further latency reduction is worthwhile, sample the rawler call once to
separate demosaic compute from preparation and copying before choosing an
optimization.

### Reproduce the stage split

Run from the repository root with the same local Nikon Z 5 fixture.

```sh
PICASA_TRACE=1 PIC_TEST_RAW=/path/to/DSC_1240.NEF \
  cargo test --locked real_nef_sensor_render_uses_full_source_and_camera_look_reference \
  -- --ignored --test-threads=1 --nocapture
```

Read `nef_sensor_unpack`, `nef_prepare_demosaic`, and `nef_scene_v2`.
The fixture fails the colour assertion after the timed stages complete. The
initial measurement and two fresh-process repeats are recorded in
`/tmp/pic-nef-stages-build.log`, `/tmp/pic-nef-stages-2.log`, and
`/tmp/pic-nef-stages-3.log`.

### Demosaic timing

A temporary Fedora `perf` executable could not open a user-space CPU clock
event under the current permissions. As a fallback, one GDB run timed the
PPG demosaic call using source breakpoints on either side. Disassembly
confirmed that the breakpoints bracket the call and a small step lookup.
The test binary and fixture were the same as in the stage split.

| Interval | Wall time | Share of rawler stage |
| --- | ---: | ---: |
| PPG demosaic call, with debugger overhead | 116 ms | 27% |
| Remaining rawler work, by subtraction | 312 ms | 73% |
| Total preparation and demosaic | 428 ms | 100% |

This is wall-clock attribution from one run. The remainder includes
preparation, copying, cropping and other rawler work. Their individual
contributions remain unmeasured. The same baseline colour assertion failed
at 34.13. No rendering code changed for this measurement. The GDB log is
`/tmp/pic-profiler.v6TgKd/gdb.log`.

The next narrow experiment is to time the sensor clone, rescale and
float-buffer construction individually before proposing any change.

### Preparation timing

One further GDB run on the same binary and fixture split the 429 ms rawler
stage using source breakpoints. Times are approximate wall-clock intervals
with breakpoint overhead.

| Interval | Wall time |
| --- | ---: |
| Sensor clone | 24 ms |
| Sensor rescale | 51 ms |
| Float-buffer construction | 47 ms |
| PPG demosaic | 112 ms |
| Default RGB crop through return path | 181 ms |
| Other stage work | 14 ms |

The crop-through-return interval was largest. Disassembly confirms the RGB
crop call lies inside that interval. Its standalone cost and the possibility
of eliminating its copy remain unmeasured. A next experiment would verify
whether the same crop can be applied while writing the output, preserving
dimensions and pixel bytes. The fixture again hit the baseline colour
assertion at 34.13. See `/tmp/pic-profiler.v6TgKd/prep-detailed.log`.

### Live editor confirmation

Opening the same Z 5 NEF in the instrumented desktop app measured 809 ms
for decode and develop, including 158 ms for sensor unpack and 440 ms for
rawler preparation and demosaic. The first RGBA8 conversion took 64 ms and
GPU upload took 17 ms. This confirms the earlier test-process breakdown in
the live app. The 181 ms crop-through-return estimate still comes from GDB.
Live trace: `/tmp/pic-raw-live.i5WQ0G/pic.log`.

### First editor RAW frame

A fresh debug-profile desktop process on the RX 9070 XT opened the same
outside-library NEF from Fit. The driver timestamped the Edit button's
AT-SPI activation and the first GPU after-paint trace for the 6016 × 4016
sensor-developed source. The interval was **1,559 ms**. RAW development
started about 346 ms after activation and took 813 ms. Texture upload took
32 ms in this run. This measures activation to GTK after-paint, including
local trace delivery. It does not measure pointer dispatch, monitor scanout,
or when the stand-in fade finishes.

The driver and timestamped trace are `/tmp/pic-crop-exact.RSMvmd/live.py`
and `/tmp/pic-crop-exact.RSMvmd/editor.log`. The release build was compiling
concurrently, so this is one observed latency rather than an idle-system
estimate.

### Exact crop boundaries

The breakpoints bracketed the allocator and the pixel-copy iterator in
rawler 0.8.0 `src/pixarray.rs:476–489`. Call-site breakpoints in
`src/imgop/develop.rs:288–326` also separated the old RGB buffer release
and the cloned RawImage drop. The debug-profile rawler dependency is already
built at optimization level 3.

| Debug-profile interval | Observed wall time |
| --- | ---: |
| Output allocation call | 1.1–1.2 ms |
| Pixel-copy iterator, including page writes | 174–183 ms |
| Whole RGB crop call | 178–187 ms |
| Old RGB buffer release | 19–20 ms |
| Cloned RawImage drop | 8 ms |
| Crop call through actual function return | 210–211 ms |

The first run instrumented the crop only. The final two runs included cleanup.
Debugger stops add overhead, especially to the allocator interval. The newer
return breakpoint is after RawImage cleanup. The earlier 181 ms estimate
stopped at the return expression before this cleanup and came from one run.

The crop reserves 289,923,072 bytes for a packed 6016 × 4016 RGB float
buffer and copies the selected pixels into it. The uncropped buffer
release was 292,239,360 bytes. Both RGB buffers coexist during the copy,
about 555 MiB of buffer capacity before counting sensor data and other
allocations. That is not a process peak RSS measurement. The crop changes
the geometry, and the current return type stores packed pixels. No operation
has been proved redundant. Whether downstream code could consume a crop
without the intermediate copy remains an open design and parity question.

The machine-address breakpoints and raw logs are in
`/tmp/pic-crop-exact.RSMvmd/exact.gdb` and `debug*.log` in the same directory.

### Optimised release confirmation

Two fresh runs on the same NEF used the optimised release test binary with
ThinLTO and debug information. ThinLTO inlined the crop. Breakpoints were
checked against the release disassembly before timing. The interval from
allocator return to the old RGB free includes copy setup and page writes.

| Release interval | Run 1 | Run 2 |
| --- | ---: | ---: |
| Output allocation call | 0.9 ms | 0.8 ms |
| Allocator return through copy and setup | 150.2 ms | 150.1 ms |
| Old RGB buffer release | 20.4 ms | 20.1 ms |
| Cloned RawImage drop | 7.4 ms | 7.4 ms |
| Crop entry through actual function return | 181.5 ms | 180.8 ms |
| Entire rawler preparation and demosaic | 424.7 ms | 418.7 ms |

The release runs reproduce both the dominant copy interval and the crop
output allocation size. They still hit the baseline colour assertion at
34.13. The copy interval does not distinguish iterator instructions from
memory traffic or page faults. The release confirmation is for RAW
development, while the 1,559 ms live editor measurement used a debug build.

Reproduce with `CARGO_PROFILE_RELEASE_DEBUG=1 cargo test --release --locked
--no-run`, then run the same NEF test binary under GDB using the breakpoints
in `/tmp/pic-crop-exact.RSMvmd/release.gdb`. This script uses the specific
binary's instruction addresses. Check them against `objdump -d -l` after a
rebuild. The two logs are `release-1.log` and `release-2.log` in that directory.

### Production acceptance

The full-size Nikon scene path now opts into the in-place RGB crop in the
local rawler 0.8.0 patch. Three fresh release processes per version used the
same Z 5 fixture and trace settings.

| Metric | Allocating crop median | In-place crop median |
| --- | ---: | ---: |
| Preparation and demosaic | 400 ms | 265 ms |
| RAW decode and develop | 774 ms | 655 ms |
| Process peak RSS | 819 MiB | 642 MiB |

The synthetic allocation-reuse test and the real-NEF bitwise intermediate
comparison passed. The test fixture retained 6016 × 4016 output and the
existing colour assertion reported the same red-channel difference of 34.13.
The measurements are for the test process, not live editor latency. Logs
are in `/tmp/pic-nef-crop-accept.vxxsL2/`.

### Test the production patch

Commit `dec797a0` enables the owned crop for full-size Nikon scene
development. The half-size export path and other rawler callers use their
existing paths. The patch's upkeep notes and upstream version are in
[rawler's local patch notes](../../../third-party/rawler/PIC-PATCH.md).

From the repository root, open a disposable copy of a Nikon NEF.

```sh
PICASA_TRACE=1 cargo run --locked -- /path/to/photo.NEF 2>&1 | tee /tmp/pic-nef-editor.log
```

1. Open the photo at Fit, click Edit and wait for the RAW frame. Check that
   framing and orientation stay correct as the stand-in fades away.
2. Toggle 1:1, pan to the edges and return to Fit. Look for a change in
   framing, fine detail, highlight colour or sharpness.
3. Change Exposure, then Undo or Reset. Repeat with a portrait-oriented NEF
   and, if available, a NEF from a different Nikon body. Compare the same
   file, recipe and zoom on the parent commit if anything looks different.
4. For repeatable measurements, use a fresh process per open and the same
   build profile on both commits. Record `nef_scene_v2` and
   `source_upload_ms` separately. The more detailed
   `nef_prepare_demosaic` marker requires the uncommitted investigation
   timers. The release benchmark above measures decode and develop only.

Run the real-file parity test for each additional fixture to compare the
dimensions and every RGB float sample against rawler's allocating crop.

```sh
PIC_TEST_RAW=/path/to/photo.NEF cargo test --locked \
  owned_nef_crop_matches_allocating_crop_pixel_for_pixel \
  -- --ignored --test-threads=1
```

The separate native colour test already fails on the original Z 5 fixture
at 34.13. Record any difference from the baseline for the same fixture.
