# NEF export quality investigation

> Follow-up verified on Nikon Z 5 NEFs: thumbnails and bounded Fit use embedded camera JPEGs; full-resolution Edit and Export use sensor-developed pixels. The [viewer/editor quality fix plan](nef-viewer-editor-quality-fix-plan.md) records the original investigation and the follow-up source-render checks. Lightroom parity remains unverified.

## Success criteria

- Export receives the original NEF path, develops sensor pixels, and never reads the thumbnail cache.
- Export resolution matches the sensor-developed source dimensions and cannot silently fall back to a small cached or embedded preview.
- Thumbnails and bounded Lightbox Fit continue to use Nikon's embedded JPEG for fast, camera-colour preview.
- Native-size Lightbox, full-resolution Edit Mode, and Export share the sensor-developed path; a Fit-sized JPEG cache frame cannot satisfy a native request.
- The embedded full-size camera JPEG calibrates the sensor render's global colour response; this does not claim Lightroom or camera-local processing parity.
- Saved Edit Mode adjustments, crop, rotation, and overlays remain applied to export.
- A real local NEF regression proves Export develops the original's full-resolution sensor data even when catalog dimensions are thumbnail-sized.
- NEF, JPEG, PNG, and WebP export regressions pass, and the original NEF remains byte-identical.
- Keep changes scoped; no revert or merge to `main`. Commit and push on the release branch after the investigation and checks finish.

## Design boundary

- Viewer Fit decode accepts display bounds and uses a scaled camera JPEG; native-size decode develops sensor data.
- Export decode accepts an original source and a named quality intent; it does not accept viewer/cache paths or viewport dimensions.
- Recipe rendering remains shared after decode so crop, rotation, tone, drawings, and overlays stay consistent.
- Export size limiting happens once, after rendering, and reports the final dimensions.
- Sensor RAW development is explicitly used for native-size viewing, full-resolution Edit Mode, and Export. It is not used for thumbnails or bounded Fit previews.

## Progress

- [x] Trace gallery and Edit Mode export job creation back to the original `PhotoObject.path()`.
- [x] Trace `render_job` into `render_for_export` and the RAW decoder.
- [x] Confirm export size defaults to Original (`max_edge = 0`).
- [x] Review Git history and V1.0.0 implementation; neither establishes a separate sensor-RAW export path.
- [x] Find a real local NEF fixture and inspect sensor/embedded-JPEG dimensions.
- [x] Reproduce current real-NEF export and capture decode strategy, dimensions, and timing.
- [x] Initial implementation followed the V1.01 embedded-JPEG contract. The follow-up supersedes that behavior for native-size viewing, full-resolution Edit, and Export.
- [x] Inspect the existing `export-test5/DSC_2086.jpg`: its 2048 × 1367 dimensions match the dialog's 2048-pixel size preset, not the thumbnail cache.
- [x] Run Edit Mode preview and export regressions against the exact local `DSC_2086.NEF`.
- [x] Check current rawler and RAW-development references before recommending an implementation.
- [x] Add an explicit unbounded RAW-source decode path for Export, separate from thumbnail cache and viewport-size bounds.
- [x] Add a real-NEF regression that uses production catalog dimensions and simulated 640 × 427 stale dimensions, then checks pixel equality against the full original decode.
- [x] Run focused export, Edit Mode, and relevant RAW regressions.
- [x] Review final diff for unrelated changes; report evidence and remaining limits.

## Evidence so far

- Export jobs use the original record path. The export chain does not open `cached_thumbnail_path`.
- The dedicated RAW Export decoder uses the uncapped sensor-developed viewer path; the embedded JPEG supplies only the global colour calibration reference. Export does not read the disk thumbnail cache.
- The available `DSC_2306.NEF` reports sensor dimensions 6040 × 4032; its largest `JpgFromRaw` is 6016 × 4016 (1,227,120 bytes). Rawler's sensor-developed output is 6016 × 4016.
- The target `DSC_2086.NEF` reports sensor dimensions 6040 × 4032; its largest `JpgFromRaw` is 6016 × 4016 (2,349,817 bytes).
- Ran `real_raw_export_fidelity_and_privacy` against `DSC_2306.NEF`: it developed the original's 6016 × 4016 sensor image, applied saved edits and rotation, and wrote a 3212 × 4812 cropped output at `max_edge=0`. The test passed for JPEG, PNG, and WebP, and verified the original NEF was unchanged.
- Export now routes RAW files through `decode_raw_source_for_export`, which requests the largest available source without applying catalog/viewport bounds. Output size caps still apply afterward in the export writer.
- The strengthened real-NEF test passes both production catalog dimensions (6016 × 4016) and simulated stale cache dimensions (640 × 427); both produce the same 6016 × 4016 unedited export pixels.
- Ran the real Edit Mode zero/exposure/reset regression against the same NEF: preview used the 6016 × 4016 sensor-developed frame; exposure and reset retained full source dimensions and reset pixels exactly.
- Focused tests passed: Export Batch (25 passed, 1 ignored) and Edit preview (2 passed, 1 ignored). The real-NEF export and Edit preview tests passed when explicitly enabled with `DSC_2086.NEF`.
- `export-test5/DSC_2086.jpg` is 2048 × 1367. The Export dialog maps “2048 px” to `max_edge=2048`; “Original size” maps to `max_edge=0`. The dialog currently initializes to “Original size” for each invocation.
- The ignored `real_raw_export_fidelity_and_privacy` test compares Export with the shared full-size viewer source. The follow-up `real_nef_sensor_render_uses_full_source_and_camera_look_reference` additionally proves native output differs from embedded JPEG pixels, checks source dimensions and per-channel colour distance, and verifies the original NEF hash.
- The original `export-test5/DSC_2086.jpg` is 2048 × 1367 because the size option capped the long edge at 2048. This accounts for its smaller dimensions; it was not a thumbnail-cache export.
- V1.0.0 and current `main` use the same broad embedded-preview-first approach. Reverting to V1.0.0 would not add sensor RAW development.
- The V1.01 history explicitly documents that embedded-JPEG export does not demosaic sensor RAW; the GPU-export prototype was excluded to keep the CPU path. A previous commit also records full-resolution CPU fallback as an intentional V1 behavior.
- The Export dialog labels size choices as maximum long-edge limits and states that NEF export uses full-resolution sensor data.
- `rawler` exposes separate thumbnail, preview, full-image and raw-image APIs. Sensor development is a distinct processing pipeline involving demosaicing and color/white-balance rendering; it cannot be treated as a cache-quality toggle.
- Primary-source review: Rawler's `extract_full_pixels` may return a decoder-provided image before falling back to `raw_to_srgb`; preview extraction is a separate API. The implemented Nikon path explicitly calls `raw_image` followed by `RawDevelop`, then verifies sensor-derived native output ([Rawler analyze API](https://docs.rs/rawler/latest/src/rawler/analyze.rs.html)).
- GPU research: Vulkan format documentation defines sRGB formats and the relevant format behavior; this is useful for GPU preview/edit color parity, but it does not affect CPU Export's source-resolution choice ([Vulkan format specification](https://docs.vulkan.org/spec/latest/chapters/formats.html)). No GPU or shader changes were warranted for this export fix.

## Performance review of the proposed NEF optimizations

- The existing decoded-image cache already serves Export: it is keyed by local path, modification time, and file size, has a 128 MiB budget, and rejects cached frames smaller than the requested export dimensions. A cold full-resolution export develops sensor data and inserts the resulting frame; later full-size requests can reuse it.
- The real-file trace measured embedded-JPEG extraction at 1.18 ms, full TurboJPEG decode at 80.45 ms, and the cold path at 129.9 ms including resize/ownership conversion. A warm full-size read hit the decoded-frame cache and took 50.1 ms, almost all of it the owned RGBA copy. Caching extracted JPEG bytes would avoid only the 1.18 ms extraction step when the decoded-frame cache misses; it would not avoid JPEG decode. That is not justified for this local NEF by current measurements.
- Follow-up sensor trace: local `DSC_2306.NEF` demosaic took 936 ms and colour calibration/output mapping took 468 ms on this machine. Bounded Fit retained the 22 ms TurboJPEG path; thumbnail cache generation took 17 ms. These are local measurements, not remote-share timings.
- TurboJPEG already selected scale 1/1 for the full-resolution path. The viewer resizer skips resampling when source and destination dimensions match; the remaining copy creates an owned image for export/edit processing.
- The existing source-frame cache is used by Export, so a second parallel full-resolution cache would duplicate memory and complicate eviction without evidence of a miss. A disk mmap is also deferred: no measurable file-read bottleneck was found in this trace.
- In the edited export, tone/filter/sharpen work took about 615 ms and JPEG encoding about 463 ms, both substantially more than a warm cached decode. These are the larger measured costs if export time needs further work.
- Strengthened the ignored real-NEF regression so Export runs first (cold cache), asserts exact source-preview dimensions, then compares later full-size and stale-catalog requests against that full frame.
