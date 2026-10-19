# Experimental GPU Edit and safe photo Export

**Scope:** draft PR #212 on `feature/gpu-edit-pipeline` (base
`feature/gpu-live-edit`). The stable `main` branch is unchanged.

## GPU Edit status (verified on the owner’s Fedora RX 9070 XT)

- Photo adjustments on a GTK `GLArea` desktop OpenGL shader: Exposure,
  Contrast, Saturation, Temperature, Fill Light, Shadows, Highlights, Sharpen,
  Clarity, Grain, B&W/Sepia and all fourteen named creative presets.
- Crop, Straighten, library quarter-turn rotations, Fit/1:1, pan and source
  texture caching. Interactive image overlays and rasterized Pango text.
- Fit and other sub-100% views use footprint-selected trilinear mip sampling.
  A neutral source uses a mip pyramid generated with its one source upload.
  For edited frames, the GPU applies geometry and the full-resolution recipe
  into a cached render texture before building the mip pyramid. The cache is
  reused until the source, recipe or geometry changes; no CPU readback or
  repeated source upload is needed. This keeps Sharpen, Clarity and Grain in
  the same order as CPU preview and Export. The 1:1 path uses level-zero
  sampling and retains full source detail.
- Seventeen GPU filter-picker covers from one decoded source preview.
- A failed GL context, shader or draw returns to the CPU render path; image
  originals and the database recipe are never modified by GPU operations.
- Owner tested and approved the GPU Edit milestone at commit
  `35b227e40633c0b38c1ce903eabd2f168121dc13` (tag
  `gpu-edit-hardware-verified-2026-10-18`). This **does not** verify every
  later Export change or every distribution runtime.

**Auto-correction parity status:** Plain Auto Colour, plain Auto Contrast and
their combination match the CPU Export renderer on the synthetic native 1:1
fixtures (maximum channel difference 0–1). The GPU path now keeps deterministic
GPU-side Auto Colour accumulation and full-resolution histogram statistics;
it does not read statistics back to the CPU. Preview selection remains
conservative: GPU Auto corrections are currently used only for identity
geometry at native 1:1. FIT/50% previews and quarter-turn rotations use CPU
because mip-filter rounding or rotated sampling can differ from the Export
reference. Auto corrections with crops, straighten or creative presets also
remain on CPU. The expanded crop/filter matrix still measures up to 16/255
maximum channel difference for Auto Contrast with full-strength Pencil Sketch,
so those combinations are not safe to enable on GPU yet. The test logs maximum and mean
channel differences and the worst pixel for each diagnostic case; tolerances
have not been increased. Auto mode now probes hardware capability by default;
these unverified edit combinations still use the CPU renderer.

No local Nikon NEF/NRW file was available for the Auto parity matrix. The
embedded Nikon JPEG colour-fidelity regression fixture passed, but this does
not establish Auto parity on an actual camera file. `PIC_EDIT_RENDERER=auto`
uses capability-checked GPU selection by default; the unverified Auto recipe
combinations listed above continue to use the CPU renderer.

### Auto-correction investigation on the RX 7600

The opt-in diagnostic (`PIC_GPU_AUTOSTATS_DIAG=1`) compared CPU Export with
GPU output for all 14 creative presets at strengths 0.25, 0.65 and 1.0, using
Auto Colour, Auto Contrast and both together. It used a deterministic 48×32
synthetic photo with a 12.5% crop on each edge. The full 126-row result,
including maximum and mean channel difference, worst pixel/channel and the
filter-stage comparison, is in
[`auto-correction-parity-rx7600.csv`](auto-correction-parity-rx7600.csv).
These test-only reads inspect the GPU filter intermediate; interactive draws
still do not read image or statistic textures back to the CPU.

Across the 13 presets other than Pencil Sketch, the maximum final difference
was 4/255; the creative-filter intermediate differed by at most 1/255.
Full-strength standard presets were exact in this fixture, while low blend
strengths exposed byte-rounding differences. Auto Contrast and the combined
case sometimes amplified those one-level input differences because a changed
8-bit luminance bin can change the clipped percentile bounds. Pencil Sketch
was the outlier: its GPU filter intermediate differed by up to 6/255, and its
full-strength Auto Contrast and combined results differed by 16/255 (mean
1.2378/255; worst at pixel (0, 15), red 245 GPU vs 229 CPU).

The Pencil Sketch mismatch is in the spatial filter before automatic
correction. CPU `image::imageops::blur` carries its separable Gaussian
intermediate in float and quantizes once at the output; the GPU blur writes
each axis pass to an 8-bit R8 texture, quantizing between passes. At larger
source sizes, the GPU also caps the effect buffer at 1200 pixels, unlike the
full-source CPU Export blur. The subsequent Auto Contrast histogram and
correction amplify this filter-stage difference. The CPU fallback remains in
place for all automatic corrections combined with creative presets, crop,
straighten, FIT/50% and rotated views.

The deterministic Auto Colour reduction uses one RGBA32F partial texel for
each 512 sampled pixels and a final reduction pass. For the 3936×2624 RX 7600
benchmark this was 391 texels (about 6.1 KiB), with no extra source upload or
CPU readback. The same-device OpenGL timer-query comparison with the previous
single-texel additive reduction was:

| GPU case | Deterministic reduction, first / cached | Previous reduction, first / cached |
| --- | ---: | ---: |
| First full frame | 10.236 / 0.016 ms | 8.921 / 0.019 ms |
| Auto Colour | 1.245 / 0.017 ms | 1.102 / 0.017 ms |
| Auto Contrast | 1.905 / 0.029 ms | 1.130 / 0.017 ms |
| Both | 1.726 / 0.018 ms | 2.157 / 0.017 ms |

The runs show no consistent latency regression for the deterministic reduction;
the variation across first-frame measurements is larger than the measured
cached difference. On the native identity fixture, deterministic Auto Colour
is exact (0/255), while the previous reduction measured 1/255. Keep the small
reduction for that measured parity improvement, but keep the broader CPU
fallback gates: it does not repair creative-filter or geometry discrepancies.
The same GPU timer-query test measured CPU full-frame Auto Colour, Auto
Contrast and combined renders at 59.045, 66.271 and 85.737 ms with the current
reduction, versus 59.415, 68.686 and 86.447 ms with the previous path. Timer
queries measure GPU execution; those CPU numbers are reference-render time.

Future Auto selection must probe successful GL context/shader initialization
and the OpenGL capabilities required by the renderer. It must not branch on
GPU vendor strings or dedicated-versus-integrated GPU classification. Keep
the explicit GPU mode available for validation, and fall back to CPU after
context, shader or draw errors. Add capability gates only after the release
checks below pass on supported runtime stacks.

## Export status and implementation

Export uses the full decoded source through the CPU recipe reference. It is
independent of the GPU preview framebuffer, preserves transparent PNG/WebP
overlays, and flattens JPEG alpha against white. It does not modify the source
file. Crop, rotate, tone, filters, layers and text are applied before encoding.
JPEG/PNG/WebP save through a staging file alongside the final destination; on
Linux a `RENAME_NOREPLACE` publication prevents concurrent overwrites.

EXIF is rebuilt from a selected set of nonlocation camera/lens/exposure/date
fields. The exported orientation is always 1 and pixel dimensions describe
the rendered file, never the unedited source. GPS, author fields, MakerNotes,
embedded previews and source-side EXIF pointers are intentionally omitted by
default. The output is tagged with a generated sRGB ICC profile, not blindly
with the input file's potentially incompatible profile.

The automated editor quality workflow (including output metadata, Export
pixels, non-destructive originals, concurrency, overlay safety and software
OpenGL pixel parity) passed at `0700ac3`:
https://github.com/froggy744/PIC/actions/runs/39199032850

**Important Nikon distinction:** For DSC_5791.NEF, the available full-size
embedded JPEG is 3936×2624 (other embedded previews are 1620×1080 and 640×424, confirmed by inspecting the user's local NEF without uploading it). Export at those dimensions uses the camera's JPEG
preview; it does **not** demosaic Nikon NEF RAW sensor data. RAW-development
colour, detail and profile fidelity require a separate demosaicing pipeline.
Do not describe the embedded-JPEG export as full RAW development.

## Development and verification

```sh
cd ~/PIC/.worktrees/gpu-edit-test
git fetch origin feature/gpu-edit-pipeline
git switch --detach origin/feature/gpu-edit-pipeline

PIC_EDIT_RENDERER=gpu PICASA_TRACE=1 cargo run --release \
  2>&1 | tee /tmp/pic-gpu-test.log
```

For CPU reference:
```sh
PIC_EDIT_RENDERER=cpu cargo run
```

GPU Fit quality regression (high-frequency checkerboard against an
`image::FilterType::Lanczos3` reference):

```sh
xvfb-run -a cargo test --locked gtk_gl_area_fit_minification_matches_lanczos_checkerboard_reference \
  -- --ignored --test-threads=1 --nocapture
```

The ignored `gtk_gl_area_benchmarks_full_source_upload_and_viewport_sized_draw`
test reports source upload/mipmap construction, CPU full-frame time, GPU draw
callback timing and GL elapsed-time queries for the rendered frame and
automatic-statistics passes. A Mesa 26.2.3 run on the AMD Radeon RX 7600
measured 7.012 ms for the first 3936×2624 frame and 0.017 ms cached; Auto
Colour 1.134 ms, Auto Contrast 1.075 ms and both 1.635 ms on their first
frames. Cached automatic frames measured 0.017–0.018 ms. The corresponding CPU
full-frame recipes took 62.101, 68.781 and 87.842 ms. Auto adjustments caused
zero additional source uploads. These GPU numbers come from OpenGL timer
queries, while the separately reported draw callback is CPU submission time.
They are local hardware results, not a guarantee for other drivers.

## Second Fedora GPU validation — office RX 7600 XT

Once Editor image-quality CI passes on the branch commit, the office Fedora
RX 7600 XT can test directly using `cargo run --release`; **an AppImage build
is not required** for source/graphics testing. AppImage smoke validation runs
separately for packaging changes or on a manually requested release checkpoint.
Do not compare performance to the RX 9070 XT as if the GPUs were identical.

```sh
cd ~/PIC/.worktrees/gpu-edit-test
git fetch origin feature/gpu-edit-pipeline
git switch --detach origin/feature/gpu-edit-pipeline
git log -1 --oneline
PIC_EDIT_RENDERER=gpu PICASA_TRACE=1 cargo run --release \
  2>&1 | tee /tmp/pic-rx7600xt-gpu.log
```

Review one editable NEF at Fit and 1:1, exposure, a creative filter, a
crop/rotation, and text/image layers. Check that no unexpected blur or
visual jump occurs and that the logs show actual GPU draws. Combined
Auto Colour/Auto Contrast with a cropped/filtered recipe intentionally
stays on the CPU reference until the strict GPU parity gate is green.

The privately held real-RAW Export regression needs a local NEF/DNG
path but **does not upload the photo or modify it**. It creates and
removes temporary JPEG, PNG and WebP exports; checks render dimensions,
lossless pixel identity, JPEG quality, normalized EXIF, GPS/author
privacy and the unchanged RAW original:

```sh
PIC_TEST_EDIT_REFERENCE="/path/to/your/photo.NEF" \
  cargo test --locked real_raw_export_fidelity_and_privacy -- \
  --ignored --test-threads=1 --nocapture
```

Do not run the private RAW test against an offline mount or a network
copy that is not currently readable. Do not publish the generated logs
if they contain private source paths. The test proves Export's CPU
render correctness; interactive GPU preview quality still needs a
visual comparison on the actual Fedora monitor.

The experimental Linux packaging workflow builds and smoke-tests **AppImage only**,
without publishing a release. It does not run for ordinary `src/edit/**` or
`src/thumbnail/**` commits. It runs for changes to the packaging script,
workflow or Cargo manifests, and on manually dispatched checkpoints. **Flatpak
is deferred by the owner (2026-10-19)**. Passing package builds cannot replace
hardware testing on Fedora Wayland with the host's Mesa/GLVND drivers.
AppImage must never bundle vendor GPU drivers.

## Phase 5 — automatic live-renderer selection

`PIC_EDIT_RENDERER` defaults to `auto`. Auto creates a GTK GLArea and
selects GPU only after the current context passes capability, shader, and
render-target validation; `PIC_EDIT_RENDERER=cpu` forces CPU and `gpu` makes an
explicit attempt (including under software GL for debugging). No renderer
settings control was added. Filter-picker thumbnails stay on CPU in Auto to
avoid creating a separate GL context per tile; explicit GPU mode keeps their
existing accelerated path.

The probe requires an OpenGL 3.2 context, successful compilation/linking of all
editor programs, at least six fragment texture units, one vertex texture unit,
and a maximum texture size of at least 2048. It then draws a known 2×2 sample to
an RGBA32F framebuffer and verifies the pixels. Successful capability
fingerprints are cached for the process; shader programs still initialize for
each new GLArea. Failed Auto initialization is suppressed for 60 seconds, so a
known-bad driver is not retried for each image; a later editor opening can
retry after that delay. A source larger than the device texture limit falls
back for that editor without poisoning the device-wide capability cache.

Auto classifies known software rasterizers (`llvmpipe`, `softpipe`, `swrast`,
and SwiftShader) as CPU-only. This check does not classify GPU vendors or
integrated versus discrete devices. An Intel or AMD integrated GPU is eligible
when it meets the same GL requirements; no Intel iGPU was available for this
local run. Hardware validation used an AMD Radeon RX 7600 on Mesa 26.2.3. The
2×2 validation readback is one-time and tiny; no photo pixels are read back for
capability selection. The first Auto initialization measured 21.739 ms
(including shader initialization and validation); the first 8×8 draw submitted
in 1.866 ms. The latter is CPU callback time, not a GPU timer-query result.

The CPU `GtkPicture` remains available during GPU initialization and until the
first frame is presented. Runtime errors retain the last valid GPU frame while
the CPU recipe render completes. Unsupported recipes, GL initialization or
draw errors, and oversized sources are traced under `PICASA_TRACE` and use the
CPU renderer. The existing CPU fallback for Auto Colour/Auto Contrast with
unverified zoom, crop, straighten, rotation, and creative-filter combinations
is unchanged. EditRecipe, Export, and saved recipe behavior are unchanged.

Phase 5 local checks: automatic context validation and pixel presentation
passed on the RX 7600; with `LIBGL_ALWAYS_SOFTWARE=1`, llvmpipe was detected
and rejected with a CPU fallback. Existing native-size GPU parity,
crop/straighten/quarter-turn and FIT checkerboard tests passed on the RX 7600.
The isolated Xvfb filter-thumbnail regression now passes with maximum channel
error 1 across its presets. The full Editor quality workflow passed locally,
and hosted Editor CI passed on draft PR #213 into
`feature/gpu-edit-pipeline` (run 39241677602). A Fedora desktop smoke test with
`PIC_EDIT_RENDERER` unset selected the AMD Radeon RX 7600, passed capability
validation, presented GPU frames, and exercised FIT, 1:1, zoom, and adjustments.
The trace showed CPU fallback for unsupported Auto-correction and creative
filter recipes. The renderer-mode regression test for the forced CPU override
passed. Intel integrated graphics was not available for this hardware
validation.

Before a release, retain the strict parity gates for automatic-adjustment
combinations, review full-resolution JPEG/PNG NEF-preview exports with EXIF and
ICC metadata, run the packaging smoke checks when relevant, and obtain owner
approval. Never publish personal test photos.
