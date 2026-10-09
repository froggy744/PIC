# Cargo warning cleanup

Branch: `chore/cargo-warning-cleanup`

Baseline: `cargo check` reported 204 Rust warnings. `cargo check`, `cargo build`, and the test target now compile with zero warnings.

## Done

- [x] Remove unused imports, variables, and a redundant closure capture.
- [x] Remove the duplicate theme metadata match pattern and unnecessary argument braces.
- [x] Replace 12 deprecated `gtk::CssProvider::load_from_data` calls with `load_from_string`.
- [x] Confirm `cargo check` and `cargo build` succeed after both cleanup batches.
- [x] Run focused theme discovery and progressive scanner tests. Theme discovery: 15 passed; progressive scanner: 10 passed.
- [x] Smoke test the app after the first two batches (confirmed by the user).
- [x] Restrict eight test-only helpers to test builds; six focused tests pass.
- [x] Restrict two more test-only helpers to test builds and remove three unreferenced functions; four recovery tests pass.
- [x] Restrict five more test-only helpers to test builds and remove two unreferenced wrappers; four focused tests pass.
- [x] Replace seven deprecated GTK size/visibility calls; `cargo build` and 10 focused Settings tests pass.
- [x] Isolate photo-wall timing and onboarding helpers used only in tests; remove an unused export-progress field. Test build, wall-quality test, and 14 export tests pass.
- [x] Move two static onboarding CSS rules into the shared stylesheet; `cargo build` and 15 theme discovery tests pass.
- [x] Remove five uncalled collage, lightbox, thumbnail, and grid helpers; test compilation and two collage draft tests pass.
- [x] Remove unused Home and editor fields and methods; 17 focused tests pass, with two display tests ignored.
- [x] Remove the unused bulk thumbnail entry point and its unused wait helper; 11 thumbnail queue tests pass.
- [x] Replace startup recovery and Settings confirmation message dialogs with `AdwAlertDialog`; `cargo build`, test compilation, and 10 focused Settings tests pass.
- [x] Replace the Settings RAW/JPEG pair selector with `DropDown`, remove unused SD import progress fields, and move Albums bookshelf CSS to a display provider; `cargo check`, 10 Settings tests, and two image format tests pass.
- [x] Replace the text editor font selector with `DropDown`, remove unused reflow snapshot data and an uncalled zoom handler; `cargo check`, 38 edit model tests, and six section lookup tests pass.
- [x] Replace the Create Database, Collage export options, and Photo export options GTK dialogs with `AdwAlertDialog`; `cargo build`, 14 export tests, 10 Settings tests, and two collage model tests pass.
- [x] Replace the Settings Open/Restore, overlay import, and startup recovery file choosers with `GtkFileDialog`; `cargo build`, 11 overlay tests, and 10 Settings tests pass.
- [x] Replace Collage export and Photo export destination choosers with `GtkFileDialog`; `cargo build`, 14 export tests, and two collage model tests pass.
- [x] Remove uncalled tile, scanner, availability, and decoder wrappers; keep a text placement helper for tests only. `cargo build`, 38 edit model tests, 16 folder stream tests, and nine scanner tests pass.
- [x] Replace four deprecated viewport allocation calls in Folder and Wall with parent-relative bounds, width, height, and baseline; `cargo build` and seven focused grid tests pass.
- [x] Remove unused Folder target and visible-folder lookup code, an uncalled retry drain, and production-only scan reason variants that exist solely for tests; `cargo build`, 16 folder stream tests, and 10 scan authorization tests pass.
- [x] Remove the unused strip transition layer and its obsolete tests; active Folder and Wall reflow stays in place. `cargo build` and five section lookup tests pass.
- [x] Remove unused network import state and dormant source reconnection state; six network import and one availability test pass.
- [x] Replace the network picker GTK dialog with a modal GTK window; preserve its import and cancel actions.
- [x] Replace deprecated SMB initialization, validate network scan prefixes, and enlarge IP buffers. Two SMB prefix tests and five network share tests pass; one network share test is ignored.
- [x] Run final `cargo check` and `cargo build` with a writable ccache directory; both finish with zero warnings.
- [x] Clean the remaining test-target warnings: migrate supported GTK APIs, retain CSS border and padding assertions with a local deprecated-API allowance, and remove an unused test helper. `cargo test --no-default-features --no-run` compiles with zero warnings.
- [x] Support DNG previews in the network viewer; the focused remote DNG viewer tests pass.

## Manual verification completed

- [x] Visually check the welcome camera and Discover PIC tour headings after the CSS move.
- [x] Check that Settings destructive confirmations open and Cancel closes them without running the action. Check startup library recovery if that flow is encountered.
- [x] Check the RAW/JPEG pair selector saves all three choices and the Albums bookshelf background still renders.
- [x] Check the text editor font picker, including an edit whose saved font is not installed.
- [x] Check Create Database and both export options dialogs: Cancel should dismiss, and Create/Export should proceed with the chosen values.
- [x] Check Settings Open/Restore and overlay import file pickers. Check startup recovery's Choose Database picker if that flow appears.
- [x] Check single and batch Photo export destinations and Collage export destination, including filename and extension behavior.
- [x] Check Wall zoom and Folder resize/scroll for visual jumps after the viewport allocation change.
- [x] Check the network picker opens, Cancel closes it, and Import adds selected shares.
- [x] Check live SMB and NFS share connections.
- [x] Run the full test suite with a writable cache, D-Bus, and network interface access.
- [x] Finish the app smoke test after the final warning cleanup.

## Network RAW grid thumbnails (follow-up)

- [x] Route network DNG and other registered RAW formats through in-memory preview decoding, with sensor development as a fallback.
- [x] Preserve Nikon's existing embedded JPEG range reads.
- [x] Read orientation from downloaded RAW bytes and serialize non-Nikon RAW thumbnail downloads/development.
- [x] Change non-Nikon network RAW cache keys so previous failed thumbnails retry.
- [x] Verify DNG preview sizing and orientation, corrupt RAW rejection, and cache invalidation; full suite: 576 passed, 116 ignored.
- [ ] Test DNG and other camera RAW thumbnails in the app over NFS and SMB, including portrait orientation and scrolling.
- [x] Optimize DNGs with embedded JPEG strips: read metadata and the preview via network ranges, and use scaled JPEG decoding.
- [x] Verify the supplied DietPi DNG over live NFS: thumbnail generation fell from 823–1107 ms to 247 ms; the JPEG preview is 5.8 MB versus the 46.5 MB original. Full suite: 577 passed, 117 ignored.
- [x] Fix lightbox DNG quality: select the largest embedded preview and develop sensor pixels when previews are smaller than the sensor image. Live NFS verification: S20 FE 20221225_123929.dng now opens at 3024×4032 instead of 384×512; the S25 DNG still opens at 5712×4284.
