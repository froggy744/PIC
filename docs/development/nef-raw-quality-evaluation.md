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

The diagnostic also reports `PIC_RAW_QA_GAIN` and the three
`PIC_RAW_QA_LIGHT group=exposure_only_...` summaries. A single robust
neutral-region gain is applied **only in memory to the comparison**, never to
PIC's viewer, editor, saved recipes or exports. The optional
`sensor-exposure-only.png` shows the result of that one global brightness
adjustment. Colour-channel ratios in non-clipped pixels should not be
substantially corrected by gain alone: this test is intended to prove whether
brightness and colour must be treated as separate problems. The PNG is
JPEG-reference-assisted diagnostic material, **not** the final RAW rendering.

For DSC_8030.NEF the original rawler output (without any experimental fitting)
measured ~40.6% of the reference JPEG's scene-linear brightness, ~44.4% in
neutral candidates and ~35.2% in warm-colour candidates. Neutral colour shares
were close, while warm-region red share was substantially lower. Camera data:
Nikon Z 5; WB [1.625, 1, 1.5019531, NaN] (the fourth component is unused on
3-channel Bayer); D65 and A matrices present, D65 selected; black 1008,
white 15520. These values do not alone demonstrate broken WB or matrix.

### Illuminant A versus D65 test (optional)

The Nikon Z 5 `DSC_8030.NEF` has camera matrices for both daylight (D65)
and tungsten (A). Existing `rawler 0.8` preferentially selects D65, even
when the photograph was taken under other lighting. To test the alternative
**without changing PIC or copying any third-party code**, run the same ignored
test with `PIC_NEF_EVALUATION_COMPARE_MATRIX_A=1`. The test temporarily removes
the D65 entry from *its decoded metadata clone*, allowing rawler's existing
illuminant A and chromatic adaptation path to run as a second candidate.
It writes `sensor-matrix-A.png` and `sensor-matrix-A-exposure-only.png`
and prints `PIC_RAW_QA_MATRIX` and `PIC_RAW_QA_LIGHT group=matrix_A_...`.

This is *not* dual-illuminant interpolation and may produce worse results.
No profile settings or colour processing are applied to the normal PIC viewer,
editor or export. Compare both matrix candidates visually and using the warm
and neutral channel-share residuals before any integration decision.

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


## 2026-10-23: independent offline cross-check using uploaded NEF

The project owner has run more than enough manual tests and has asked PIC's
agent to own further verification. **Do not request repetitive terminal tests
from the user.** Use prior evidence, existing private uploads and automated
tests first. Request at most a *final* real-device acceptance after an
independently inspected candidate meets basic requirements.

A separate, private analysis session successfully opened the previously
provided `DSC_4814.NEF`, independently developed its sensor data using
a system-installed decoder for *diagnosis only*, and compared a JPEG preview
against Nikon's camera JPEG. No photo or decoded raster was committed, no
new PIC dependency was added, and no source code or implementation from that
decoder was copied into PIC.

- Independent sensor decode produced **6040 × 4032** developed RGB16; camera
  embedded reference was **6016 × 4016**. These images have different borders,
  so there was no claim of pixelwise colour accuracy.
- On matching rescaled scene previews, mean display-space luminance was
  roughly **0.401** (independent developer) versus **0.408** (Nikon camera).
  The test used automatic brightness for this independent diagnostic, not a
  new PIC RAW rendering option.
- Saturation was visually and statistically different despite very similar
  mean brightness: median HSV-style saturation approximately **0.558**
  versus **0.679** (Nikon). This cross-check demonstrates that matching
  average exposure cannot reproduce the camera's picture processing.
- **Visual acceptance failed:** the project owner identified a strong green
  cast in the independent sensor-derived reference. A technically decodable,
  exposed RAW image is not necessarily a correctly colour-developed image.
  The independent render must NOT be treated as a known-good colour baseline.

Interpretation: The real camera JPEG contains a developed appearance including
tone, saturation and potentially local processing, beyond demosaic/WB alone.
The `DSC_8030.NEF` illuminant-A candidate improved warm-colour residuals
but also shifted neutral colour shares and remained substantially too dark.
Do not mistake an improvement on one file for a calibrated or generalizable
Nikon colour profile. Do not apply either candidate matrix globally.

Next development should be a **camera-independent, scene-referred tone and
output colour pipeline** with preserved precision, followed by multi-scene
private validation. Keep the JPEG-fit experimental renderer out of V1.01.
No additional user terminal-run loops are part of the normal development plan.


### Corrective cross-check: DSC_4814.NEF, independent render (same day)

The Nikon Z 5 file's *own* camera as-shot white balance read through the
system-installed RAW decoder was red/green/blue **1.869140625 / 1 /
1.369140625**. Earlier `DSC_8030.NEF` white balance was
**1.625 / 1 / 1.5019531**. These differ naturally between photographs;
copying one scene's multipliers into another was not a legitimate profile.

A private sensor render of `DSC_4814.NEF` with its **actual** as-shot WB
still looked cooler/greenish than the Nikon camera JPEG and did not produce
acceptable colours. The renderer's default daylight-WB look was warmer but
was still visibly unlike the camera JPEG in greens/contrast/saturation.
This rejects the earlier suggestion that merely applying camera WB is a
solution, and rejects that independent system render as a validated target.

**Do not change PIC's default RAW renderer on this evidence.** A real RAW
working-colour pipeline needs sensor decode, valid per-file WB, illuminant-
dependent colour calibration and an intentional output tone/look. The JPEG
is a reference only, not a substitute for preserving sensor-derived data.
No outside implementation, camera profile or colour LUT was copied into PIC.


## Original Nikon V2 scene renderer — controlled candidate

Implementation: `src/thumbnail/nef_scene.rs`, enabled **only** with
`PIC_NEF_SCENE_RENDER=1`. V1.01's default JPEG browsing, 1:1, editing and
export behaviour is unchanged.

The V2 path develops **genuine sensor data**, without ever reading the
embedded camera JPEG in its colour/tone calculations:

1. Decode NEF using PIC's already-linked `rawler 0.8`. Use as-shot camera
   white-balance coefficients.
2. When both Nikon illuminant A and D65 matrices exist and are valid,
   estimate an interpolation weight from the observed camera-neutral ratios.
   Reuse the *existing* library Bradford adaptation to convert the A matrix
   to D65, then interpolate camera calibration in the D65 output space.
   Invalid, absent or ill-conditioned matrices fall back to rawler's existing
   colour calibration unchanged. The interpolation is exploratory, not a
   claim of complete DNG dual-illuminant parity.
3. Develop to **float32 linear RGB** with rawler's established demosaic,
   scale, as-shot white balance, camera calibration and crop, but explicitly
   omit its terminal sRGB transfer function.
4. Select a bounded global exposure from scene luminance median and 85th
   percentile, **not** from Nikon JPEG pixels or any scene-specific colour fit.
5. Apply an original continuous luminance shoulder to protect highlights.
   Use a shared scale for each channel and common gamut reduction to avoid
   channel-specific hard clipping, then apply the standard sRGB display
   transfer function and output **16-bit RGB**.
6. The existing viewer/editor/export still converts the resulting image to
   RGBA8 when entering its legacy render stage. This is a high-precision
   RAW-*development* stage, **not** end-to-end float RAW editing. A later
   controlled project change must address downstream precision separately.

`PICASA_TRACE=1` logs `nef_dual_illuminant_calibration`,
`nef_scene_render` (scene exposure) and `nef_scene_v2` (total time). The
existing `PIC_NEF_EVALUATION_SENSOR_ONLY` and JPEG-fit paths stay distinct.
The new candidate shares the ordinary 1:1, Edit and export decoder call when
explicitly enabled and leaves FIT/thumbnail previews on embedded camera JPEG.

### Validation completed, and limits

The scoped GitHub Actions check at commit `eaad7b5` passed compilation and
synthetic scene-render unit tests: exact neutral preservation, no channel-ratio
changes for unclipped midtones, bounded exposure, continuous monotonic
highlight shoulder, invalid pixel handling and calibration endpoints. Those
tests do **not** prove photographic parity.

A separate private offline simulation applied a mathematically comparable
scene-linear tone stage to a system-decoded `DSC_4814.NEF` sensor image.
That simulation was **not** generated by PIC's rawler renderer and did not
incorporate the new as-shot dual-illuminant calibration. It remained flatter
and greener than Nikon's embedded JPEG, so it is a *failed visual comparison*,
not acceptance of the V2 code. No photograph or sample raster was committed.

**Do not enable in release, merge, or declare Nikon colours fixed.** A real
V2 PIC render must be inspected against the original camera JPEG for skin,
grass, neutrals and highlight behaviour, using more than one real Nikon
lighting/ISO scenario. Do not ask the project owner to run more development
tests; agent-driven verification must precede a final requested acceptance.

**Licence protection:** no additional dependency, camera profile, shader,
LUT or third-party rendering implementation has been copied into PIC. The
existing `rawler` licence obligations still require packaging review.


## 2026-10-23: isolated float-tone and bounded cache follow-up

The experimental `PIC_NEF_SCENE_RENDER=1` path develops Nikon pixels through
`rawler` in floating-point RGB, applies an estimated as-shot interpolation
of the two *embedded* camera illuminant matrices, then performs an original
scene-derived exposure, monotonic highlight shoulder and restrained neutral-
preserving global contrast/chroma processing in linear light before one sRGB
encoding to RGB16. No embedded-JPEG pixel matching, copied camera profile,
third-party shader/LUT, or new dependency is involved. This is an **opt-in
photographic candidate**, not verified Nikon colour fidelity. The downstream
editor still uses RGBA8; end-to-end higher-precision editing is not yet done.

The native-source cache previously discarded `ImageRgb16` entirely. A
Nikon Z 5 `6016×4016` RGB16 frame requires about 145 MiB, above the old
128 MiB maximum even if RGB16 had been supported. The experimental branch
now counts and caches RGB16 buffers by their actual 2-byte samples under a
192 MiB bounded source cache, and separates cache entries by experimental
renderer mode. This should prevent unnecessary native RAW regeneration
between 1:1 and Edit; **end-to-end latency improvement has not been
benchmarked**. The 8-bit and embedded preview paths are unchanged by default.

A private *independent* system RAW-developer simulation with the existing
uploaded `DSC_4814.NEF` demonstrates that restrained global contrast/chroma
can improve a flat-looking preview, but **does not prove that PIC's real
`rawler` output is visually acceptable**. The earlier independently rendered
photo had an unacceptable green tint. None of those private source files or
renders were committed to this repository; no external engine was integrated.

Acceptance requires an actual PIC-generated NEF preview, evaluated against
Nikon's embedded image and natural skin/neutral colours, before this mode may
become default. Do not merge this branch into V1.01 or `main` on the basis
of synthetic tests or simulation alone.


## 2026-10-23: unclipped camera-to-scene calibration candidate

The experimental renderer was extended after inspection showed that the
pre-existing `rawler` calibration stage can clip scene-linear channel values
before PIC's tone shoulder runs. This defeats one purpose of real RAW editing:
highlight headroom. The experimental code now asks existing `rawler` only to
rescale, demosaic and crop Nikon Bayer data. PIC applies an independently
written 3×3 camera-to-linear-sRGB transform in floating point, using only
the file's existing colour matrix and white-balance multipliers. The matrix
is normalized to preserve a D65 neutral response, with a guarded inverse.
Values over 1.0 are retained until PIC's highlight shoulder and final 16-bit
output encoding. The code has mathematical round-trip, neutral and HDR
retention tests.

The published sRGB/XYZ colourimetry constants are mathematical standards;
no outside application algorithm implementation, source code, LUT, profile or
shader is copied. The decoder crate is the same already-included `rawler 0.8`
(LGPL-2.1): third-party licensing and distribution must still be audited.

**Photographic acceptance is still open.** The new conversion has not yet
been validated with a privately rendered PIC output against Nikon's
embedded camera JPEG, nor stress-tested for noise, performance or GPU/CPU
parity. Do not turn on by default or merge for V1.01 solely on code tests.
