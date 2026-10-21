# NEF detail and colour: viewer, Edit Mode, and Export

> **Current implementation:** see [Nikon NEF Edit and Export](guides/nef-edit-export.md). This file is a historical record of the sensor-development experiment. Its status and checklists below describe that earlier experiment and are superseded by the current guide.

Historical status: the initial V1 camera-JPEG workflow was followed by an experiment that developed Nikon sensor data for native-size requests. That experiment did not preserve Nikon's camera rendering. The current implementation again uses the largest embedded camera JPEG for native viewing, Edit Mode, and Export, with sensor development only as a missing-preview fallback.

The reported symptoms are soft/pixelated detail and wrong brightness/colours in NEF 1:1 viewing and Edit Mode. The first investigation retained embedded-JPEG rendering. The user then confirmed this still showed only the embedded JPEG and authorized a sensor-rendered native-detail path.

## Follow-up: native Nikon sensor rendering (2026-10-21)

- The previous fix passed its own comparisons because Lightbox, Edit Mode, and Export all used `JpgFromRaw`. That proved source consistency, not sensor-RAW rendering. The real-app trace reproduced the problem: Fit decoded a 1504×1004 camera JPEG, and the original native-size request also entered the embedded-JPEG decoder.
- Nikon NEF thumbnails still use the embedded camera JPEG. Bounded Lightbox Fit requests still decode the largest embedded JPEG with TurboJPEG. Native-size Lightbox, full-resolution Edit Mode, and Export decode the sensor mosaic through Rawler's demosaic/develop path.
- The sensor-developed frame is calibrated with a sampled 3×4 linear colour transform fitted against the embedded full-size camera JPEG. The transform corrects global colour/exposure differences while the rendered image pixels remain derived from sensor data. It does not reproduce Nikon's local tone, sharpening, or Lightroom edits exactly.
- Full-size sensor output is cached in the existing 128 MiB in-process source-frame cache. Fit requests remain eligible for the bounded JPEG path; a small cached Fit frame cannot satisfy a native-size request.
- Verified `DSC_2306.NEF` and `DSC_4728.NEF`, both Nikon Z 5. Each produced a 6016×4016 sensor-developed frame from a 6040×4032 mosaic. At sampled points, the render differs from the camera JPEG while remaining within 25 mean RGB levels per channel. The original files remained byte-identical.
- The native Wayland/Vulkan app trace shows a 17 ms thumbnail cache write, a 22 ms Fit JPEG decode, then a 936 ms sensor demosaic and 6016×4016 Lightbox frame on 1:1. The separate GL Edit regression uses the same sensor frame and matches CPU/GPU readback exactly for the tested neutral and edits.
- The native harness now waits for the sensor-developed frame and its matching full-size Lightbox decode before it reports a NEF 1:1 pass. The first harness attempt had closed the viewer as soon as the 1:1 request was queued, before demosaic completed; that run was not treated as verification.

## Initial V1 phase: camera-JPEG detail path (2026-10-20)

This phase records the earlier implementation and the tests that passed before the user reproduced the remaining source-selection bug. Its camera-JPEG-only detail contract is superseded by the 2026-10-21 follow-up above.

- [x] Used the available original `DSC_2086.NEF` (6040×4032 sensor; Nikon `JpgFromRaw` 6016×4016) without changing it.
- [x] Independent ExifTool inspection found JPEG candidates at 6016×4016, 1620×1080, and 640×424. The largest is the camera-generated `JpgFromRaw`; Nikon metadata declares sRGB and Vivid picture control. The embedded JPEG has no ICC profile.
- [x] Traced Lightbox Fit and 1:1 to the embedded camera JPEG. Native 1:1 decode and texture dimensions are 6016×4016. The thumbnail path still uses bounded embedded-JPEG decode; the real NEF thumbnail took 23 ms and produced a 752×502 source preview.
- [x] Fixed a confirmed CPU Edit Mode quarter-turn defect: decode bounds were swapped before decoding, reducing 90°/270° previews before rotation. Decode bounds now remain in source coordinates.
- [x] Added a synthetic quarter-turn regression and expanded the real-photo CPU regression to require exact embedded-JPEG pixels, full dimensions, adjustment/reset equality, and correct 0°/90°/270° dimensions.
- [x] Fixed Edit Mode and Lightbox native-size geometry at HiDPI and fractional scale. GTK logical sizing now maps 1:1 to one source pixel per physical display pixel; editor overlays and Draw keep logical pointer coordinates and normalized photo positions. Monitor-scale changes relayout and preserve the zoom anchor.
- [x] Fixed embedded NEF JPEG thumbnails skipping ICC conversion. ICC-tagged embedded previews now use the same JPEG-to-sRGB conversion as full viewer/export; added a wide-gamut embedded-NEF regression. `DSC_2086.NEF` has no JPEG ICC and declares sRGB, so it correctly remains untransformed.
- [x] Extended the real-photo OpenGL test to require a neutral 1:1 frame to match the decoded source crop byte-for-byte. On `DSC_2086.NEF`, neutral GPU readback and slight exposure edit match the CPU reference with maximum channel difference 0; the source is 6016×4016 and uploads once. The real NEF test passes at surface scales 1 and 2.
- [x] Ran `real_raw_export_fidelity_and_privacy` against the original `DSC_2086.NEF`. It confirms full-size export matches the full-size camera preview, stale thumbnail-sized catalog bounds do not downscale export, PNG/WebP match the edit render, and the source file stays unchanged.
- [x] Compared the full-size embedded `JpgFromRaw` bytes independently with PIC's decoded source and CPU/GPU neutral output. Pixel values match exactly. Opened the same NEF in the native Wayland/Vulkan app, viewed it in Lightbox at 1:1, entered Edit Mode, and confirmed the GPU editor decoded 6016×4016 and reached native scale.
- [x] Completed the isolated native Wayland/Vulkan harness: import, visible gallery, Lightbox, 1:1, close, and restart persistence all passed. A separate live Edit Mode check selected the same NEF, opened the GPU editor, and activated Edit Mode 1:1. See `reports/gui-tests/native-nef-final/report.md` and `reports/gui-tests/native-nef-final/screenshots/editor-1to1.png`.
- [x] Compared Edit/export outputs through the real-NEF CPU/GPU and export fidelity regressions; the source NEF hash remained unchanged.
- [ ] A matching Lightroom export with its profile, edits, sharpening, and crop settings was not available. No claim is made that the app's sensor rendering reproduces Lightroom's rendering or Nikon's full local processing.

Initial V1 regression runs completed before the follow-up changes: `cargo test --locked edit:: -- --test-threads=1` (177 passed, 11 ignored); `cargo test --locked viewer_presentation_tests -- --test-threads=1` (26 passed); the wide-gamut embedded-NEF colour test (1 passed); scale-2 Lightbox allocation test (1 passed); five established software-GL parity tests; real-NEF CPU rotation/edit/reset, export fidelity/privacy, thumbnail timing, and the native Wayland/Vulkan harness. Follow-up-specific results are recorded above and in the release checklist. GTK OpenGL tests must run in separate processes because GTK cannot be initialized on successive Rust test threads in one process.

## What the previous work established

Commit `0d704eb` removed catalog-size limits from RAW Export. It did not change the viewer or Edit Mode source-selection, colour, or display paths. Its tests established source-path use and pixel dimensions for a camera JPEG extracted from the NEF. They did not establish that the actual displayed image is correct.

The earlier investigation remains useful evidence, but its completion must not be interpreted as acceptance of NEF image quality. Local cache/decode timings also do not establish network-share performance or colour fidelity.

## Concrete leads from code inspection

These are investigation targets, not established causes of the user's photo failure.

| Area | Observed implementation | Check required |
| --- | --- | --- |
| Embedded image selection | `src/thumbnail/nef.rs::nef_jpeg_stream` chooses the largest compressed byte length. For DSC_2086 this is Nikon's 6016×4016 `JpgFromRaw`, ahead of 1620×1080 and 640×424 candidates. | Confirm the same source contract with the second reported NEF, if available. |
| Edit source resolution | `src/edit/preview.rs::preview_target_dimensions` previously swapped bounds for library quarter-turns. That reduced the image before the later rotation. The swap is removed and covered at 90°/270°. | Check stale catalog dimensions separately if a real reproduction demonstrates truncation. |
| Edit diagnostics | `src/edit/editor.rs` sets `source_full = true`; the actual GPU test now checks the 6016×4016 source, one upload, and exact native viewport pixels. The real CPU test checks unedited output against decoded source pixels. | Confirm the full application scheduling/reopen/export sequence on a real NEF. |
| Colour interpretation | `src/thumbnail/viewer.rs::decode_viewer_jpeg` assumes sRGB when the JPEG contains no ICC profile. DSC_2086 declares Nikon sRGB; no double transform was found. | Match the user's Lightroom output with a controlled profile/recipe before attributing its visual difference to PIC. |
| 1:1 presentation | Lightbox already requests unbounded decode; Edit Mode and the GPU also have their own geometry, source caches, and display scaling. | Verify the final pixels presented, including device scale, sampling, and stale frame replacement. An unbounded decode alone does not prove correct presentation. |
| Previous real Edit test | `real_photo_zero_exposure_reset_keep_editor_preview_resolution` requests 3960 × 2640 and compares its own original/edited/reset buffers. | Exercise the real UI with the full source; add an independent reference and verify the frame after switching Fit/1:1 and changing edits. |

## Fix sequence and completion checklist

### 1. Establish an independent reference and reproduce the visible failure

- [x] Use the available `DSC_2086.NEF` as the first fixture. Keep the original file unchanged; use isolated generated-photo GUI fixtures.
- [ ] Repeat the full workflow with a second NEF with different orientation or colour metadata. `DSC_8030.NEF` was not available; `DSC_2086.NEF` was used for the full run.
- [x] Record the running debug binary, source fingerprint, catalog dimensions, active GTK Vulkan renderer, Edit OpenGL renderer, and surface scale in the native harness/app logs.
- [x] Extract the embedded JPEG candidates independently with ExifTool and inspect their dimensions, ICC, and colour declarations. Capture the NEF's corresponding metadata separately.
- [x] Compare PIC's full-size source and neutral Edit output to the independently extracted largest embedded JPEG; native CPU/GPU pixel checks are exact.
- [x] Reproduce Gallery → Lightbox Fit → 1:1 → Edit Mode → Edit 1:1 on the real NEF; logs show the first Fit decode is 752×502 and the completed native decode is 6016×4016. CPU and GPU regressions cover exposure/reset and 90/270-degree rotations; export is covered by the separate real-NEF fidelity test.
- [x] Repeat Fit then native 1:1 with the reduced Fit image cached. The cached frame is rejected as too small and the viewer decodes the full 6016×4016 embedded JPEG. CPU Edit rotation tests cover 90/270 degrees.
- [x] Record the first faulty stages: quarter-turn Edit decode bounds reduced source resolution; HiDPI Lightbox/Edit geometry multiplied native content by display scale; embedded-NEF thumbnails skipped ICC conversion. Full-size source decode and export were already correct for the V1 camera-JPEG contract.

Acceptance for this step: a repeatable failing sequence, aligned reference/app crops, and evidence identifying the first incorrect stage. A file's dimensions or a cache hit alone is insufficient.

The Lightroom JPEG is a useful visual comparison, but its saved edits, output size, and sharpening must be recorded. The existing 2800-pixel Lightroom output and a 6016-pixel embedded JPEG have different magnification at 1:1. Use matched framing and scale to assess detail, while retaining a true native-pixel check for each source.

### 2. Confirm the initial V1 image-source contract (historical)

- [x] Determine whether PIC's selected largest embedded JPEG is the camera-generated image: ExifTool identifies it as Nikon `JpgFromRaw` at 6016×4016, with sRGB/Vivid metadata; exact source bytes and decoded neutral output match.
- [x] The initial phase retained the agreed camera-JPEG architecture and repaired the proven CPU rotation stage.
- [x] The initial phase selected the camera-generated Vivid/sRGB JPEG. The user's later reproduction showed this did not satisfy native-detail expectations; the follow-up now uses sensor-developed pixels for native-size viewing, Edit Mode, and Export.

The starting scope for the first phase was the agreed V1.01 camera-JPEG behavior. The follow-up keeps embedded-JPEG thumbnails and Fit browsing, and switches only native-detail requests, full-resolution Edit Mode, and Export to sensor development so the path is consistent across those uses.

### 3. Repair only the proven source/detail failures

- [x] Confirm candidate selection against the real NEF: byte-length selection returns the 6016×4016 `JpgFromRaw`, independently verified as the largest embedded JPEG. Selection logic did not need a speculative change.
- [x] Give native NEF requests a source-resolution contract across Lightbox, Edit Mode, and Export. Full-size dimensions come from the selected embedded image; orientation and PIC rotation are applied after decode.
- [x] Confirm stale catalog bounds do not truncate Edit or Export. The real export regression explicitly compares full catalog dimensions with simulated thumbnail-sized bounds.
- [x] Confirm a Fit-sized frame cannot satisfy a native request: the actual Fit→1:1 harness run rejects its 752×502 cached frame and replaces it with the 6016×4016 decode.
- [x] Verify that 1:1 maps one source pixel to one physical display pixel. Tests cover GTK surface scales 1, 2, 1.5, and 1.75; actual scale-2 GPU readback is exact. Crop, pan, and overlay coordinates remain in their expected spaces, and Fit remains scaled normally.
- [x] Cover CPU fallback and GPU upload with full-resolution source checks; the real NEF source uploads once and the 1:1 GPU frame matches decoded source pixels. Texture-limit fallback behavior was not changed.

Acceptance for this step: native detail agrees with the independent source in Lightbox and Edit Mode, including pan, rotation, minor adjustment, reset, and reopening. No reduced frame remains presented as the completed native image.

### 4. Repair only the proven colour failures

- [x] Determine source colour space from the actual file evidence. `DSC_2086.NEF` declares sRGB/Vivid; its embedded JPEG has no ICC and is correctly assumed sRGB. A synthetic ICC-tagged embedded NEF verifies wide-gamut conversion to sRGB in thumbnail and full viewer paths.
- [x] Trace source pixels through ICC handling, CPU Edit, GPU texture upload/sampling, Lightbox display geometry, and export. The real NEF is unprofiled sRGB and neutral CPU/GPU pixels match its embedded JPEG exactly; the synthetic ICC NEF covers wide-gamut conversion.
- [x] Apply supported embedded-JPEG source-to-sRGB conversion consistently in thumbnail and full viewer/export decoding. No ICC transform is applied to unprofiled sRGB Nikon JPEGs.
- [x] Check neutral output, slight exposure, Auto Colour/Contrast, filters, clarity, and grain across CPU/GPU parity regressions. No tolerance was increased.
- [x] Exercise native GTK Vulkan Lightbox and the editor's actual OpenGL renderer on the Fedora app; software OpenGL parity also passes separately. The real Edit session reports the AMD Radeon RX 9070 XT renderer.

Acceptance for this step: the no-edit image matches the independently verified reference; matching edits give consistent colour and brightness across viewer, editor, and export. Compare framebuffer and exported pixels in a common colour space; desktop screenshots alone can include an additional monitor transform.

### 5. Strengthen regression coverage and verify the application

- [x] Add a focused quarter-turn regression for the confirmed CPU decode-bound cause.
- [x] Add native-quality assertions against independently extracted embedded-JPEG pixels, including exact neutral crops and GPU framebuffer readback.
- [x] Exercise native requests after Fit-cache warm-up, stale export catalog dimensions, and 90/270-degree library rotation. Portrait EXIF orientation was not present in the available fixture.
- [x] Verify neutral/reset equality, crop/rotation, drawing/overlay geometry, and lossless PNG/WebP export against the verified render. JPEG export remains lossy by design.
- [x] Run the relevant render, editor, export, metadata, colour, geometry, and GPU parity regressions. Existing parity thresholds were retained.
- [x] Run the real-photo GPU sharpness test at scale 1 and scale 2, then open the actual app with the real NEF in an isolated library. Lightbox and Edit Mode both reach 1:1 using the full preview; the native harness confirms Lightbox and a separate live AT-SPI check confirms Edit Mode.
- [ ] Verify a second real NEF, ordinary JPEGs, and an accessible representative NFS/SMB source. Use identical source content for local/network pixel comparison where possible; record unavailable coverage explicitly.
- [ ] Compare thumbnail throughput and first-view/native-view timing with the recorded branch baseline. Revisit caching only if measured performance regresses after correctness is fixed.

Relevant existing test entry points include:

```text
edit::editor::tests
edit::render::tests
edit::export_batch::tests
edit::export_metadata::tests
native_preview_geometry_tests
embedded_nef_jpeg_converts_wide_gamut_to_srgb_for_viewer_and_export
gtk_gl_area_full_resolution_sharpness_regression
gtk_gl_area_matches_cpu_reference_at_native_pixel_size
real_raw_export_fidelity_and_privacy
```

### 6. Release acceptance

- [x] Report the confirmed defects, scoped code changes, source provenance, renderer, dimensions, and regression results. The original `DSC_8030.NEF` was unavailable; `DSC_2086.NEF` and an identical isolated copy were used.
- [x] Confirm detail and camera-JPEG colour through exact source-pixel comparisons in the CPU/GPU Edit pipeline and full-size decode. Lightroom parity remains unverified because its matching export was unavailable.
- [x] Confirm full-size Export independently decodes from the original NEF, applies the tested recipe/crop, and leaves the NEF unchanged.
- [x] Confirm the thumbnail path still uses the largest embedded JPEG and generated the real NEF cache preview in 23 ms; this measurement is local and is not a network-share baseline.
- [x] Capture the live Edit Mode 1:1 view and retain it for review at `reports/gui-tests/native-nef-final/screenshots/editor-1to1.png`. This verifies the current camera-JPEG contract; it is not a Lightroom parity claim.

The follow-up code and local real-NEF workflow are complete. The available `DSC_2306.NEF` and `DSC_4728.NEF` fixtures pass Fit-versus-native source checks, Edit Mode CPU/GPU checks, Export checks, and a native Wayland/Vulkan 1:1 run. Remaining evidence limits are a matched Lightroom export, the unavailable `DSC_8030.NEF`, and remote-share native-render timing.
