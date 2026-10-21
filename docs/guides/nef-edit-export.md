# Nikon NEF Edit and Export

## Implemented behavior

PIC uses Nikon's embedded JPEG as the default rendered source for a NEF. The largest embedded camera JPEG supplies the full-size Lightbox image, Edit Mode source, and Export source. This preserves the camera's white balance, picture control, tone curve, and in-camera noise reduction while keeping the existing edit recipe and GPU preview pipeline.

The fast paths remain:

- Thumbnails use the small embedded Nikon JPEG and the existing thumbnail cache.
- Lightbox Fit uses TurboJPEG to decode the largest embedded JPEG at display size.
- 1:1, Edit Mode, and Export use that same JPEG at its full embedded resolution.
- Sensor development is a fallback for NEFs with no embedded JPEG. A user-selected RAW develop mode remains a future feature.
- Export applies the saved edit recipe and writes a new file; it leaves the NEF unchanged.

This matches the V1 camera-rendered workflow. It does not claim to reproduce Lightroom edits or provide a sensor RAW develop workflow.

## Defects fixed and regression coverage added

1. **The default sensor render produced poor camera colour and tone.** The previous native NEF path demosaiced sensor pixels and applied a global colour fit against the camera JPEG. That fit could not reproduce Nikon's local tone processing and noise reduction. In the tested `DSC_0934.NEF` export, the previous output was visibly darker and noisier than the restored camera-rendered path. The standard viewer, editor, and export now consume the same full-size camera JPEG.
2. **Compressed byte length selected the wrong embedded JPEG.** `DSC_0941.NEF` has a 3936×2624 `JpgFromRaw` that is smaller in compressed bytes than its 1620×1080 `OtherImage`. Ranking by file size selected the lower-resolution image. Selection now ranks by pixel area, reading JPEG SOF dimensions when Nikon omits dimensions from the TIFF IFD. Thumbnail selection keeps its existing smallest-stream behavior.
3. **Copy/Paste Edits now has an end-to-end export regression.** The GTK regression invokes the real context-menu actions, checks that Paste writes the copied recipe to the library and refreshes the selected `PhotoObject`, then exports a fixture using the stored recipe and checks the PNG pixels. This action path passed; no separate Copy/Paste failure reproduced in the isolated test.

## Files changed

- `src/thumbnail/viewer.rs`: use the largest embedded Nikon JPEG for native NEF viewing and keep sensor development as a missing-preview fallback. Keep thumbnail and Fit decode paths bounded.
- `src/thumbnail/nef.rs`: rank native JPEG candidates by pixel area and parse JPEG SOF dimensions when TIFF dimensions are absent. Add a regression where the full-size stream is smaller in bytes.
- `src/edit/export_batch.rs`: let the real-NEF export regression accept the saved recipe and rotation, and compare the Edit Mode render with full-size JPEG, PNG, and WebP export.
- `src/window/photo_actions.rs`: exercise Copy Edits → Paste Edits → persisted recipe → Export in a GTK regression.

## Verification

The read-only `RAW.db` favorite query found 12 NEFs, `DSC_0934.NEF` through `DSC_0945.NEF`. All 12 passed the native viewer check. The first nine resolve to 3936×2624 camera frames; `DSC_0943.NEF` through `DSC_0945.NEF` resolve to 6016×4016.

All 12 also passed the real-NEF Edit/Export fidelity regression using their saved database recipe and rotation. The regression checks full-resolution source dimensions, equality between Edit Mode and Export render, JPEG/PNG/WebP output, and that each original NEF remains byte-identical. The copied recipe was also exercised through the actual GTK menu actions and a lossless PNG export.

Additional checks passed:

- `cargo test --locked --bin pic-rs edit:: -- --test-threads=1`: 177 passed, 11 ignored.
- `cargo test --locked --bin pic-rs thumbnail:: -- --test-threads=1`: 86 passed, 8 ignored.
- GPU editor parity test: passed; native output had zero channel difference in the tested cases, with the existing downscale case reaching a maximum difference of 2. No tolerance was changed.
- Real 6016×4016 NEF GPU sharpness regression: passed; one source upload, exact source-pixel match at native scale.
- Wide-gamut embedded JPEG conversion: passed.
- GUI test harness launched the app through `cargo run` and passed startup, sample gallery, ratings, and folder navigation. The harness then stopped when the container could not execute `org.a11y.Bus` (`Permission denied`), before its remaining editing checks. The GTK Copy/Paste and GPU tests ran separately and passed.

Re-run examples:

```sh
cargo test --locked --bin pic-rs edit:: -- --test-threads=1
xvfb-run -a cargo test --locked --bin pic-rs \
  window::photo_actions_tests::copy_and_paste_edits_menu_persists_the_recipe_for_export \
  -- --ignored --nocapture --test-threads=1
PIC_TEST_RAW='/path/to/photo.NEF' cargo test --locked --bin pic-rs \
  thumbnail::embedded_nef_color_profile_tests::real_nef_native_viewer_matches_full_resolution_embedded_camera_jpeg \
  -- --ignored --nocapture --test-threads=1
PIC_TEST_EDIT_REFERENCE='/path/to/photo.NEF' cargo test --locked --bin pic-rs \
  edit::export_batch::tests::real_raw_export_fidelity_and_privacy \
  -- --ignored --nocapture --test-threads=1
```

## Other findings to track

- A previous debug run reported an assertion at `src/window/library.rs` when folder navigation passed a `SidebarFilter::Folder` to `refresh_grid_for_navigation`. The debug assertion remains in place. This NEF change does not address that separate navigation path; the isolated GUI run's folder-navigation step passed.
- Remote NEF native viewing still has a full-source read in its native-frame path, even when an embedded JPEG has been extracted. That can cost time and network I/O on NFS/SMB and needs its own measured fix.
- Existing files in `/home/peet/Pictures/manny` include 3840×2560 JPEG exports alongside native-size PNGs (3936×2624 or 6016×4016). The JPEG size may reflect an earlier custom export size; no evidence ties it to the NEF source-selection defect.
- The container's GUI harness could not launch the AT-SPI bus service, so its full editing sequence was not completed. GPU logs also include the system's `radv is not a conformant Vulkan implementation, testing use only` warning; the GPU results are testing evidence for this machine.
- No matched Lightroom export was available for a pixel comparison. The fix restores Nikon's embedded camera rendering as the common source; it does not promise Lightroom parity.
