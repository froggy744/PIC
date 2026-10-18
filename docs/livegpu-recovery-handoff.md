# LiveGPU recovery handoff — 2026-10-18

## Location and scope

Worktree: `/home/peet/PIC/.worktrees/livegpu-recovery`
Branch: `feature/gpu-live-edit`, starting HEAD `4fbf931` (matched origin).
The original `/home/peet/PIC` checkout remains clean on `main` at `df0e6b7`.
One agent used. No live desktop keyboard/mouse interaction; GUI tests used isolated Xvfb sessions and generated libraries. User subsequently authorized committing and pushing this recovery checkpoint.

Initial feature/main comparison: 12 files, 2217 insertions, 172 deletions. Window shortcuts/build/layout had no feature-branch changes. Existing main also contains draft-preview logic: do not attribute every quality problem solely to LiveGPU.

## Confirmed causes and changes

- `src/edit/editor.rs`: removed competing window Capture-phase Space handler and pan-held/pan-armed state. Existing `src/window/shortcuts.rs` → `edit_space_slot` → infobar 1:1 action remains authoritative, including its editable-field guard. Drag claims only scrollable canvas without active Crop/Overlay/Text tools.
- Cached GPU fast path previously skipped native picture sizing/scroll bounds. Extracted existing native sizing/anchor restoration into a shared helper used by CPU completion and cached GPU transitions. Pending restoration stops after returning to Fit. Native GTK scrolling remains intact.
- GPU Fit incorrectly used manual zoom's 24px margin baseline. It now matches GtkPicture Contain geometry. CPU/GPU fixture Fit bounds now match.
- `src/edit/preview.rs`, `src/edit/zoom.rs`, `src/edit/editor.rs`: removed 960px draft selection/downsampling and the draft-sized-buffer/full-sized-canvas mapping. All effect jobs use source resolution; the last sharp frame remains visible until CPU work completes. Existing decoded-source/geometry caches and worker coalescing remain. CPU-only effects can still lag; responsiveness is NOT solved for clarity.
- `src/edit/controls.rs`: slider release only ends the undo action; the final value change already requested the full-resolution frame. Removes redundant release processing.
- `src/edit/live_gpu.rs`: restoring a supported recipe re-shows the cached GL layer even when uniforms are unchanged. Editor invalidates pending CPU results/timers on GPU fast-path reuse; does not immediately hide GPU while CPU fallback is pending.
- GPU timing now distinguishes queue-to-render-entry and queue-to-after-paint. After-paint is a GTK milestone, NOT physical display presentation. Added test-only actual renderer identification and optional real-photo benchmark input.
- `scripts/gui_test.py`, `scripts/test-gui.sh`: reproducible `PIC_GUI_SUITE=edit-navigation` bypasses unrelated accessibility-dependent library checks. Tests physical key/mouse events and screenshot marker displacement. Existing full suite also calls the navigation checks. Added fixture marker and report-directory creation.
- Regression tests extend full-source exposure/reset coverage with sharpen and clarity; GPU test covers returning from hidden layer without re-upload. Removed four obsolete tests that endorsed reduced drafts.

No recipe, export, colour-processing or shader algorithms were changed. Auto still intentionally selects CPU.

## Validation completed

- `cargo check` and `cargo build`: passed after production changes. Latest subsequent changes are test-only renderer identification; these compiled and ran via cargo test.
- Editing unit tests: **138 passed, 0 failed, 4 ignored**. Before removing obsolete draft tests, 142 passed. `/tmp/pic-recovery-edit-tests-final.log`.
- CPU and GPU Xvfb navigation: **6 checks passed each**. Space Fit ↔ 1:1, matching restored Fit bounds, 80px drag produces exactly 80px marker movement, wheel-up moves marker down, Ctrl+wheel enlarges image. Infobar state transitions confirmed in traces. `/tmp/pic-recovery-navigation-{cpu,gpu}/report.md` and application.log.
- OpenGL pixel parity passed: neutral, combined adjustments/sharpen, reset, scrolled viewport, 2:1 Fit sampling, cached-layer restore. Native channel delta <=1; neutral reset delta 0. `/tmp/pic-recovery-parity-gl-final.log`.
- **Important correction:** `GSK_RENDERER=vulkan` test passed but actual renderer was `GskGLRenderer`. It fell back. Actual Vulkan compositor compatibility is NOT established. `/tmp/pic-recovery-parity-vulkan-final.log`.
- Actual NFS Nikon `DSC_5791.NEF` CPU regression passed: neutral/exposure -0.03/reset all **3936×2624**, reset byte-identical. GIO reported location not mounted, but PIC's own NFS loader succeeded. `/tmp/pic-recovery-nef.log`.
- Extended editor checks via temporary harness adapting existing tests: CPU exposure, undo/redo, reset, filter-panel opening, save/reopen and source SHA-256 passed. GPU subset **13 passed**, same behaviours plus navigation. `/tmp/pic-recovery-editor-gpu/report.md` and `/tmp/pic-recovery-editor-cpu/harness.log`. Temporary drivers `/tmp/pic-recovery-editor-gui.{py,sh}` skip unrelated library prerequisites; GPU subset stops before collage.
- No live Wayland desktop/hardware validation. Text field Space insertion and Crop/Overlay/Text pointer interactions are not physically verified. A prior commentary overstated crop coverage: filter-panel opening was tested, crop gestures were not.

## Failures/blockers — do not hide these

- Original full GUI suite stopped on accessibility-bus activation failure before Edit. Focused navigation suite avoids this unrelated dependency.
- Broader CPU suite failed gallery multiselection assertion and timed out exporting a collage through the chooser. Not investigated or attributed to this patch; export success is unverified.
- Ignored GTK editor toolbar test exited unsuccessfully under GL. Cairo/session-bus retry reached an assertion failure: `tile_picture.height() >= tile_min_height` at editor.rs around line 2745. Unresolved; no unrelated layout fixes attempted.
- First Ctrl+wheel test required >10% pixel-area growth, but one actual step produced ~6%. Assertion corrected to >2%, consistent with observed zoom geometry; no product change made for this.
- Arbitrary Fit minification parity, HiDPI, rapid interaction races, GPU failure fallback, real Wayland/Vulkan, Fedora packaging, Flatpak and AppImage remain unverified.

## Measurements (observed, not hardware performance claims)

Development optimized build, isolated Xvfb, **llvmpipe LLVM 22.1.8 software OpenGL**, GskGLRenderer; real Nikon NEF from NFS. These are single-run samples, not release benchmarks or AMD RX 9070 XT results.

- CPU neutral decode/job: 223.113 ms; cached -0.03 exposure tone 26.398 ms / whole job 47.028 ms; cached reset job 21.614 ms.
- Full source upload: **26.253 ms**, 3936×2624 / 41,312,256 bytes; upload-ready wall time 150.441 ms.
- Exposure + sharpen 0.35 CPU full frame: **416.151 ms** (sharpen blur 344.405 ms, apply 18.951 ms).
- Same recipe, GPU source retained, **960×640 viewport/framebuffer**: queue-to-render-entry 1.322 ms; draw submission 0.064 ms; queue-to-GTK-after-paint **28.135 ms**; benchmark observation loop 30.250 ms.
- `/tmp/pic-recovery-nef-benchmark.log`. Initial shader/driver work can be expensive; earlier synthetic first draw was ~512 ms. No FPS or physical presentation claim.
- Editor GUI trace showed one texture upload during a session with repeated tone/undo/reset changes; second upload was a newly reopened editor.

## Continue safely

Recommendation: **REWORK** experimental mode; retain opt-in GPU and CPU default. Input recovery and full-source preview policy have concrete passing checks, but unsupported CPU effects still lag and validation gaps prevent declaring the whole editor recovered.

Do not add another Space controller, restore drafts, alter exports, or rewrite zoom/input. Investigate remaining failures independently, compare with stable main before calling them regressions, and stop rather than inventing workarounds. User controls live keyboard/mouse.

Reproduce focused GUI checks from this worktree:

```sh
cargo build
PIC_EDIT_RENDERER=cpu PIC_GUI_SUITE=edit-navigation PIC_GUI_APP="$PWD/target/debug/pic-rs" bash scripts/test-gui.sh
PIC_EDIT_RENDERER=gpu PIC_GUI_SUITE=edit-navigation PIC_GUI_APP="$PWD/target/debug/pic-rs" bash scripts/test-gui.sh
cargo test edit:: -- --test-threads=1
GDK_BACKEND=x11 GSK_RENDERER=gl PICASA_TRACE=1 xvfb-run -a cargo test gtk_gl_area_matches_cpu_reference_at_native_pixel_size -- --ignored --test-threads=1 --nocapture
```

For the real benchmark set `PIC_TEST_EDIT_REFERENCE` to `nfs://DietPi.local/mnt/4TBP/mypics-2026/Tat%20Matric%20Farewell/DCIM/104NCZ_5/DSC_5791.NEF` and run `gtk_gl_area_benchmarks_full_source_upload_and_viewport_sized_draw` similarly. Its default synthetic fixture is 3936×2624 (~10.3MP), despite older documentation calling it 24MP.

Source patch before this handoff/documentation: 7 files, 226 insertions, 382 deletions. Exact final changes are available in the recovery commit; temporary logs are under `/tmp` and may disappear on reboot.
