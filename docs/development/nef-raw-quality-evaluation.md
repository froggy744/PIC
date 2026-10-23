# PIC Nikon Z 5 RAW quality baseline — independent implementation

**Branch:** `feature/nef-raw-quality-harness` (based on `release/v1.0.1-rc1`).
**Status:** Diagnostic only. No release renderer replacement, no new dependencies.

## No copied RAW engines or code

This work follows the PIC author's instruction: read other editors' publicly
documented *ideas*, but **do not copy** their implementations, shaders,
camera profiles, presets, images or source code into PIC. No LightCraft,
RawTherapee, darktable, or LibRaw code is incorporated. The diagnostic is
original PIC code using libraries **already present** in PIC.

PIC itself is MIT licensed. Existing third-party dependencies have their own
licences: notably `rawler 0.8` declares LGPL-2.1. An audit of existing runtime
and distribution obligations is a distinct, necessary packaging task.
"Free to use" does not mean "without licence obligations"; do not add or vendor
third-party code without checking the complete dependency tree.

## Why this experiment is different

The old `src/thumbnail/viewer.rs` attempted to estimate a per-photo colour
transform by regressing the native sensor-rendered pixels against Nikon's
embedded JPEG. Real photographs `DSC_4814.NEF` and `DSC_8104.NEF` proved that
this can produce severe colour casts. Making another such transform would
not fix the root cause.

The first baseline deliberately:
1. Develops the NEF sensor using PIC's **existing unmodified rawler**
   demosaicing, camera white balance, calibration and sRGB pipeline.
2. Decodes Nikon's actual embedded JPEG as the visual reference.
3. Reports channel residuals on a sparse sampling grid, and the shifts
   on neutral/warm reference candidates. These are *diagnostics*, not a
   claim of photographic equivalence.
4. Optionally exports the sensor's 16-bit TIFF and Nikon's original JPEG
   side by side, without modifying the input file.
5. Does **not** feed the JPEG pixels into the sensor rendering.

The reference is allowed to differ in scene-specific tone, sharpening and
local processing. Pixelwise residuals may also be affected by camera-JPEG
cropping and spatial alignment. They cannot replace visual inspection.

## On-device real NEF quality report

From an isolated checkout/worktree of this branch, with the Nikon RAW file
on the machine:

```sh
mkdir -p /tmp/pic-nef-evaluation
PIC_TEST_RAW="/absolute/path/to/DSC_4814.NEF" \
PIC_TEST_OUTPUT_DIR="/tmp/pic-nef-evaluation" \
cargo test --locked raw_quality::tests::real_nef_reference_report \
  -- --ignored --nocapture --test-threads=1
```

Output:
- `/tmp/pic-nef-evaluation/camera-embedded.jpg` — Nikon's JPEG bytes
- `/tmp/pic-nef-evaluation/rawler-sensor16.tiff` — rawler sensor-derived, developed RGB16 (gamma-encoded; not scene-linear)
- `PIC_RAW_QA` diagnostic residuals, neutral and warm candidates, dimensions, development duration
- `PIC_RAW_QA_LIGHT`: linear-sRGB luminance ratios and normalized RGB channel-share differences, to separate brightness from colour casts (diagnostic only)
- `nikon-preview.png` and `sensor-preview.png`: matching 1400px PNGs for direct visual inspection without sharing large TIFF files

Never commit real portraits, NEFs, or their reports to the public repository.

## Opt-in normal GUI sensor-only comparison

```sh
PIC_NEF_EVALUATION_SENSOR_ONLY=1 PICASA_TRACE=1 cargo run
```

This disables the old JPEG colour fit for native sensor development only.
It is strictly experimental; **the default remains untouched**, FIT still
uses the fast Nikon JPEG, and the editor/export display pipeline currently
quantizes RGB16 to RGBA8. This is NOT genuine 16-bit non-destructive tonal
editing end to end. It only isolates rawler as the likely/less-likely source of
colour errors, including the effects of camera matrices and white balance.

## Next engineering decision based on evidence

- If sensor-only colours are natural but differ in tone: focus on a
  scene-referred highlight/tone and colour-management pipeline, carefully
  preserving the 16-bit / float buffer until output conversion.
- If sensor-only shows the same extreme cast: isolate rawler's Nikon
  white-balance metadata or camera calibration matrix handling first,
  using existing metadata/standards, before changing tone or demosaicing.
- If hair/edges are bad independent of colour: profile Bayer interpolation,
  denoising, and 1:1 sharpening separately.
- If a candidate cannot beat the baseline on real Nikon photos, it does not
  become the release renderer.

**Release/merge gate:** visible skin and neutral fidelity on at least
`DSC_4814.NEF`, `DSC_8104.NEF`, and different ISO/lighting samples, stable
FIT/1:1/Edit/Export behaviour, speed regression, CI and a licence audit.
