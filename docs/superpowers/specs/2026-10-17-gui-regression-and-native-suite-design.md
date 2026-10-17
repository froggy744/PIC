# GUI Regression and Native Fedora Suite Design

## Goal

Keep the existing 41-case isolated Xvfb GUI suite as PIC's baseline regression run, make its Undo/Redo case prove editor history more thoroughly, and add a smaller native Fedora suite for Wayland, Vulkan, and a real photo folder.

## Existing Xvfb Baseline

- Keep the current `all` suite, its 41 named cases, generated photo fixtures, Xvfb environment, and report behavior as the baseline.
- Strengthen the existing `Editor Undo/Redo sequences` case without splitting it into additional cases. Compare rendered preview pixels at the original, one-edit, and two-edit states; verify each undo and redo restores the appropriate state in order; verify history controls reflect the empty undo/redo boundaries; and verify a new edit after undo invalidates the abandoned redo branch.
- Keep the current temporary XDG data and generated photos so these checks cannot affect a user's library.

## Native Fedora Suite

- Add a separate choice in the existing test menu, with an explicit `PIC_GUI_SUITE` value, and a separately identifiable report. Do not route it through Xvfb, Openbox, the Cairo renderer, or software-rendering overrides.
- Run in the active desktop session with `GDK_BACKEND=wayland`, `GSK_RENDERER=vulkan`, and GTK's renderer/fallback diagnostics enabled. Capture the requested environment and complete application logs. A successful visible app window under the forced Wayland backend and GTK diagnostics identifying a realized Vulkan renderer are required for PASS; the requested environment variables alone are insufficient. Capture GTK version, GPU identification, and graphics driver information for diagnosis. Missing Wayland, Vulkan support, or confirmation of the active renderer produces BLOCKED, not PASS.
- Use a unique run directory and temporary `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, and `XDG_CACHE_HOME`. Import a real photo folder chosen manually through PIC's GUI into that disposable library. Do not open the user's existing PIC database or write to the source photo folder.
- After manual import, ask the user to confirm that the expected gallery is visible, then have them open a photo in the lightbox, zoom to 1:1, and close the lightbox. Record confirmation for each visual step. Verify that the temporary database contains the selected folder and at least one indexed photo, record the indexed count, restart PIC against the same temporary state, and verify that the imported rows remain.
- If the user cancels folder selection, closes PIC, declines a visual confirmation, or the app exits before a required step, report BLOCKED. Retain the disposable environment, logs, and screenshots for debugging; never copy it over the user's normal PIC data.
- Save screenshots and native-session logs alongside the report. Keep native-session results separate from the existing Xvfb `latest-report.md`, so running the native suite cannot replace the baseline result.

## Report and Safety

Reports distinguish PASS, FAIL, and BLOCKED. A manually declined or unconfirmed gallery/lightbox/1:1 check is BLOCKED, not PASS. The report states which Wayland/Vulkan evidence was observed, GTK version, GPU/driver diagnostics (or why they were unavailable), which real folder was selected, and how many photo rows were indexed. The source folder is read-only test input; all PIC database/configuration writes stay inside the run directory.

## Verification

- The existing full suite still reports 41 cases, with its baseline checks preserved and the Undo/Redo case expanded.
- The native suite clearly blocks when launched outside a usable Wayland session or when Vulkan is unavailable or falls back.
- Renderer diagnostics confirm GTK initialized Vulkan; the report includes GTK version and available GPU/driver information.
- The native suite records manual confirmation of gallery visibility, lightbox opening, 1:1 zoom, and lightbox closing.
- Cancellation, early app close, and declined confirmation produce BLOCKED and preserve the disposable run directory.
- The native suite imports and persists rows from a manually selected real folder while keeping PIC's normal user data directories and source files untouched.
- Shell/Python syntax checks and the existing isolated suite are run after implementation. The native suite is run only in a real Fedora Wayland session with Vulkan support and an explicitly selected photo folder.
