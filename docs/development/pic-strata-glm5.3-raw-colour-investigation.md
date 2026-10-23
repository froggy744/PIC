# PIC Strata RAW colour experiment: GLM-5.3 notes

## Purpose

This note records the RAW colour work GLM-5.3 carried out in `/home/peet/pic-strata`, so a different rendering method can be investigated without adopting that implementation. It is a description of the experiment, not a recommendation to merge or reuse its renderer.

## Problem being investigated

Nikon NEF files looked flatter or less camera-like in PIC's full-resolution viewer than their grid thumbnail / Nikon camera rendering. Attempts to make the sensor-developed image resemble Nikon's embedded JPEG also produced coloured edge rims, speckling in hair, or blotchy skin when pixels were mixed between the two images.

The test set was:

- `DSC_1266.NEF` (with `DSC_1267.JPG` as a camera sibling)
- `DSC_4814.NEF`
- `DSC_5791.NEF`
- `DSC_5917.NEF`

Local copies were under `/home/peet/Pictures/test-raw/`. Most visual comparison work focused on 1266, 4814 and 5917.

## What GLM-5.3 tried, in sequence

1. **Global colour-matrix fit.** Fit a small per-image linear RGB transform from the demosaiced sensor render to the embedded camera JPEG. This was simple, but did not fully model Nikon's tone and colour rendering.
2. **Per-channel quantile matching.** Matched channel distributions more closely, but did not preserve local colour relationships reliably.
3. **3D residual colour field.** Tried to capture non-linear colour differences. This was more complex and still did not address the edge mismatch.
4. **Luma/chroma hybrid.** Kept chroma from the camera JPEG and used sensor-derived luminance. Rim errors remained because the luminance renderings differed.
5. **Pixel-selection edge masks.** Feathered masks and then tighter thresholds attempted to choose between sensor pixels and JPEG pixels near edges. User visual review rejected these: skin became mottled, and hair showed white speckles. A density-based mask looked cleaner in Python checks, but the direction was later abandoned.
6. **Latest approach: JPEG base plus sensor detail.** This removed per-pixel switching. It used the camera JPEG as the complete colour/tone base and added a small, neutral sensor-derived high-frequency luminance detail signal.

The first five approaches are historical context only. None is being proposed as the next method.

## Latest approach in detail (for understanding, not reuse)

The latest uncommitted `viewer.rs` change in `pic-strata` does the following at full resolution:

1. Demosaics the NEF sensor data to a 16-bit RGB image.
2. Decodes the NEF's embedded full-size JPEG and colour-manages it (the trace reported no embedded ICC profile and assumed sRGB).
3. Builds a monotone luminance lookup from sampled sensor/JPEG luminance quantiles (sample stride 8; 1024 quantiles / a 65,536-entry lookup table).
4. Applies that lookup to sensor luminance.
5. Computes a high-pass signal by subtracting a blurred sensor-luminance image from the mapped luminance.
6. Reduces that signal near gradients already represented in the JPEG, then adds the remaining scalar equally to R, G and B of the JPEG pixel. The configured gain is 0.5.

Thus, the colour correction is not a standalone sensor colour transform. The output's colour, tone, demosaic, noise reduction and sharpening come from the embedded JPEG; sensor data contributes only a constrained luminance-detail signal. Setting detail to zero would return the embedded JPEG itself.

The code comments say alignment was checked using NEF `CropArea` against JPEG dimensions and phase correlation. This was used to support pixelwise combination. A future approach need not combine the rasters at all.

## Reported visual and test observations

- Earlier mask-based variants were rejected by the user for visible skin blotches and hair speckles.
- Python prototypes of JPEG-base/detail blending at gains 0.35, 0.6 and 1.0 were inspected on 1266 and 4814. Gain 1.0 showed a dark outline on a high-contrast edge; 0.5 was selected for the Rust implementation. Notes also report visual inspection on 5917.
- The OpenCode completion report said nine unit tests passed and full-resolution renders were generated for 1266, 4814 and 5917. This note has not independently rerun those tests.
- The latest code in `pic-strata` is uncommitted: `src/thumbnail/viewer.rs` (reported diff: +485 / -96 lines). No changes from that checkout have been copied into `/home/peet/PIC`.

## Measured opening cost

The available `PICASA_TRACE=1` log includes the full-resolution open of `DSC_5917.NEF` over NFS:

| Stage | Time |
|---|---:|
| NFS source read, about 29.7 MB | 314 ms |
| Sensor demosaic, 6016×4016 | 1,042 ms |
| JPEG-reference/detail matching stage | 3,690 ms |
| Full source decode reported by viewer | 5,191 ms |
| Texture creation after decode | 45 ms |

The 3.69-second matching stage covers the LUT/detail-weight/detail-combine path in addition to decoding the embedded JPEG; it is the clearest measured penalty attributable to the latest renderer. The overall 5.19-second decode also includes demosaicing and I/O. On another test file from a local path, full-size sensor decoding in trace was around 88–91 ms, but that trace did not include this latest renderer path; it is not an apples-to-apples benchmark.

## Why this experiment should not be the next implementation

- It misses the stated latency goal: its own match stage took 3.69 seconds, before texture creation.
- Its exact Nikon-like colour comes from using Nikon's rendered JPEG as the base. That does not provide a sensor-only colour correction.
- It processes and allocates several full-frame buffers (mapped luminance, camera luminance, blur/detail, weights, output), adding CPU and memory traffic at 24 megapixels.
- Removing the detail blend leaves the embedded JPEG as the output. That may be a useful fast display path, but it is a different product decision, not a new RAW colour algorithm.
- It relies on precise alignment of the JPEG and sensor rasters. A method that estimates a camera colour transform from downsampled/reference data and applies it only to the sensor render could avoid image fusion, but its visual quality and timing still need measurement.

## Constraints for a new investigation

Treat this implementation as a baseline/failure case for latency, not as a starting patch. Do not port its JPEG-base/detail-blend pipeline, its masks, or its multi-buffer full-resolution matching stage.

A new method should answer these separately:

1. **Colour target:** Is the goal Nikon-like colour, or exact display of the camera JPEG? Exact colour is available from the JPEG; sensor rendering asks for an approximation.
2. **Open-time budget:** Measure RAW read, demosaic, colour transform, resize and display separately. The colour-correction step should not add seconds.
3. **Sensor-only transform:** If sensor pixels must remain the output, test a compact colour transform (for example, a camera/profile matrix or a small LUT) against the JPEG reference, without blending JPEG pixels into the output. Fit it at reduced resolution, then measure its application cost.
4. **Photographic review:** Compare skin hue/texture, hair, foliage, neutral surfaces and high-contrast edges in 1266, 4814 and 5917. Judge both colour and artifacts at fit view and 1:1.
5. **Avoid overfitting:** A transform fitted to one scene/photo may not generalize. Validate across scenes and exposures before considering it a camera-wide Nikon profile.

## Source and trace pointers

- Experimental checkout: `/home/peet/pic-strata/src/thumbnail/viewer.rs` (uncommitted; do not copy as the new solution).
- Timing evidence: `/tmp/opencode/pic-trace.log`, especially the `DSC_5917.NEF` records around `nef_sensor_demosaic`, `nef_camera_render_match` and `source_decode`.
- Test photos: `/home/peet/Pictures/test-raw/`.
- Main PIC checkout remained clean on `release/v1.0.1-rc1` when this note was prepared.
