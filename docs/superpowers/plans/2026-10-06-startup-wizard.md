# Startup Wizard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Recommended execution: inline in the existing session; review the whole branch before integration.

**Goal:** Get a new user into their photos through a simple Welcome → Adding photos → Library flow, with background scanning and a persistent automatic-startup preference.

**Architecture:** Put onboarding state and persistence in a focused module, GTK presentation in its own child module, and import/startup integration in a window coordinator. Reuse the existing folder picker, authorised scan queue and progressive gallery updates; bind wizard progress to its own import ticket.

**Tech Stack:** Existing Rust, GTK4/libadwaita, GLib/GIO and rusqlite; no new dependencies.

**Spec:** [Startup wizard design](../specs/2026-10-06-startup-wizard-design.md).

**UX revision:** This plan incorporates the agreed simplification. Where the linked design differs on page structure, copy, preference placement, tips presentation or preferred size, the requirements below take precedence. Keep the original persistence keys, import ownership and recovery protections. Update those UX sections of the design during implementation so executors have one consistent specification.

## Global Constraints

- Preferences are per library, matching existing application settings.
- No schema migration or new dependencies.
- Never show again suppresses automatic wizard launches and automatic onboarding tips; manual Help → Getting started always works.
- No new main-window minimum width.
- No duplicate imports on wizard reopening or restart.
- Preserve existing sidebar pin behaviour, toolbar responsiveness and progressive gallery updates.
- Do not restructure unrelated startup or scanner code.
- Network shares remain in the existing network-share flow outside the initial wizard.
- Present two main screens: Welcome and Adding photos. Recovery, empty results and errors are variations of the import screen, not extra wizard steps. No Next/Back/Finish buttons, step numbers or mandatory tips page.
- Show “Don't show this automatically again” only on Welcome; save changes immediately and restore the saved value if writing fails.
- Enable Open Library as soon as at least one usable indexed photo can be displayed. Scanning and progressive gallery updates continue after the wizard closes.
- Preferred wizard size is approximately 600×400, clamped to parent space, with wrapping and vertical scrolling at narrow widths.

## User-visible flow and copy

**Welcome:** “Welcome to PIC”; “Your photos stay on your computer. Choose a folder and PIC will build your photo library.” Primary action: **Choose Photos Folder**. Secondary: **Skip for now**. Supporting copy: “You can add more folders later.” Include the automatic-startup preference here only.

**Adding photos:** “Adding your photos”; folder name above a smaller grey, wrapped path; “Finding photos…” while discovery is active; a live count such as “327 photos found” and a progress indicator. Do not show a percentage until the total is known. Keep discovered and indexed counts distinct internally; discovery alone must not enable Open Library. Thumbnails appear progressively in the main gallery behind the dialog. **Open Library** becomes available when a usable indexed photo exists; do not wait for scan or thumbnail generation to finish.

**Recovery variation:** “Continue adding photos”; “PIC was adding photos from:” and the saved folder/path. Offer **Continue adding photos**, **Open Library** and **Choose another folder**. Resume only on explicit Continue. If the matching import is already active, show its progress and prevent another enqueue. Open Library remains available to leave recovery even when no usable photos exist yet.

**Empty variation:** “No photos found”; “PIC couldn't find supported photos in this folder.” Offer **Choose another folder** and **Open Library**.

**Error variation:** Keep the folder/path and show a concise inline message, such as “Could not read some files” or “This folder is no longer available.” Nonfatal warnings do not hide usable photos or disable Open Library. For a stopped/failed import, offer **Try again**, **Choose another folder** and **Open Library** as appropriate; retries use the same authorised import path and cannot duplicate an active job. Open Library is an exit action even if the library is empty.

**Library:** Close the wizard when Open Library is chosen. If scanning is still active, use the existing top notification area for “Photos are still being added. You can start browsing now.” Optionally show the dismissible Getting Started card from Task 5; it is never a wizard page.

## Review Focus

- Existing libraries with only trashed photos or offline/empty roots must not look like first-run libraries (Task 1).
- Preference write failure must not claim Never show again was saved (Tasks 1–2).
- Repeated activation and picker completion after closing the wizard must not create duplicate windows/jobs (Tasks 2–3).
- Interleaved maintenance, cancellation and superseded scan generations must not update the wrong import page (Task 3).
- Restart after registering a root but before scan completion must allow recovery without duplicate queueing (Tasks 1 and 4).

## File map

- Create `src/onboarding.rs`: types, startup eligibility, state transitions, settings load/save.
- Create `src/onboarding/tests.rs`: in-memory database/state regressions.
- Create `src/onboarding/view.rs`: wizard dialog, progress rendering, introductory tips.
- Create `src/onboarding/gtk_tests.rs`: individually run GTK integration regressions.
- Create `src/window/onboarding.rs`: coordinator connecting wizard and existing import/scan callbacks.
- Modify `src/main.rs`: declare onboarding module; retain existing window activation entry point.
- Modify `src/window.rs`: declare coordinator module; retain existing scanner types.
- Modify `src/window/build.rs`: install coordinator, import lifecycle hooks, accepted-event notification.
- Modify `src/window/layout.rs`: add compact Help menu/Getting started action.

## Preparation

- [x] Start implementation on a new `feat/startup-wizard` branch from fetched current main; do not reuse previous fix branches. Record the base SHA and task progress in ignored `.superpowers/` notes so outages are recoverable.

## Task 1: Persistent state and first-run eligibility

**Interfaces:** Produce `OnboardingStage::{Welcome, Importing, Tips, Complete}`, `OnboardingPreferences { never_show: bool, stage: Option<OnboardingStage>, root: Option<String>, tips_dismissed: bool }`, `StartupFacts { has_photo_records: bool, has_folder_records: bool }`, `StartupDecision::{Hidden, Welcome, RecoverImport, Tips}`. Provide `load_preferences(&Connection) -> anyhow::Result<OnboardingPreferences>`, `save_never_show(&Connection, bool) -> anyhow::Result<()>`, `save_stage(&Connection, OnboardingStage, Option<&str>) -> anyhow::Result<()>`, `dismiss_tips(&Connection) -> anyhow::Result<()>`, `finish_onboarding(&Connection) -> anyhow::Result<()>`, `startup_facts(&Connection) -> anyhow::Result<StartupFacts>` and `startup_decision(&OnboardingPreferences, StartupFacts) -> StartupDecision`. Preserve these persistent stages; `Tips` means an eligible library card, never a wizard page. Recovery/empty/error are coordinator runtime states and do not require new persisted stages or keys. `dismiss_tips` saves only the existing tips-dismissed setting, preserving stage/root so card dismissal during a scan cannot erase recovery state.

- [x] Write `onboarding::tests` covering fresh → Welcome, existing photos/folders/trashed-only records → Hidden, never-show → Hidden at every stage, complete → Hidden, importing → RecoverImport and tips without remaining photos → RecoverImport. Use temporary/in-memory databases with `db::SCHEMA`.
- [x] Add persistence round-trip tests for the four exact settings keys in the spec, including `save_never_show(true)` surviving reopening and toggling back to false, and tips dismissal during Importing preserving RecoverImport and the saved root after restart. Failed writes must return an error; unknown stage values with existing data must not trigger fresh startup.
- [x] Run `cargo test onboarding::tests -- --test-threads=1`; observe missing-interface/test failure before implementation.
- [x] Implement types and settings helpers in `src/onboarding.rs`; query all photo records for freshness rather than `db::library_counts`, which excludes trashed records. Write stage/root together in a transaction. Finish sets complete and tips dismissed together.
- [x] Run the same test command; require all cases pass. Commit the state model and tests.

## Task 2: Two-screen wizard presentation and preference

**Interfaces:** Produce `WizardPage::{Welcome, Importing}`, `ImportPresentation::{Adding, Recovery, Empty, Error}` and `WizardProgress { root: String, indexed: usize, discovered: Option<usize>, running: bool, warning: Option<String> }`. Produce `WizardCallbacks { choose_folder: Rc<dyn Fn()>, continue_import: Rc<dyn Fn()>, open_library: Rc<dyn Fn()>, skip: Rc<dyn Fn()>, set_never_show: Rc<dyn Fn(bool) -> Result<(), String>> }`. Continue and Try again share `continue_import`; Choose another folder uses `choose_folder`. `StartupWizard::new(parent: &adw::ApplicationWindow, preferences: &OnboardingPreferences, callbacks: WizardCallbacks) -> Rc<StartupWizard>`; methods `present()`, `close()`, `set_page(WizardPage)`, `set_import_presentation(ImportPresentation)`, `set_progress(&WizardProgress)` and `is_visible() -> bool`. Tips completion belongs to the library card, not wizard callbacks.

- [ ] Write GTK tests `wizard_toggle_persists_without_closing`, `wizard_toggle_write_failure_restores_saved_state`, `wizard_picker_cancel_returns_to_welcome`, `wizard_repeated_present_uses_one_dialog`, `wizard_preference_is_welcome_only`, and `wizard_uses_two_main_screens`. Assert the exact preference label, default false, immediate callback, error feedback, retained saved state, preference hidden in every import variation, no Tips page/step controls and close/Escape semantics. Closing/Escape during import closes the dialog without cancelling the background scan.
- [ ] Run each test in its own process: `cargo test TEST_NAME -- --ignored --test-threads=1`; observe failure before UI implementation.
- [ ] Build the scrollable dialog in `src/onboarding/view.rs`, using the flow/copy above. Reuse one import layout for Adding, Recovery, Empty and Error; change messages and actions instead of adding wizard steps. Keep the preference on Welcome only, prevent recursive saves when reverting a failed write, and render unknown-total progress without a percentage.
- [ ] Run all Task 2 GTK cases individually and scoped formatting. Commit the view and tests.

## Task 3: Share folder import and progressive progress

**Interfaces:** Produce `ImportTicket { generation: u64, root: String }` in the coordinator. `OnboardingCoordinator::new(window: &adw::ApplicationWindow, connection: Rc<RefCell<Connection>>, choose_folder: Rc<dyn Fn()>, continue_import: Rc<dyn Fn(String)>, open_library: Rc<dyn Fn()>) -> Rc<OnboardingCoordinator>`; methods `folder_selected(&str) -> Result<(), String>`, `import_started(ImportTicket)`, `picker_cancelled()`, `import_error(&str)`, and `scan_event(generation: u64, event: &scanner::ScanEvent)`.

- [ ] Write coordinator tests `wizard_import_reuses_one_authorized_job`, `wizard_ignores_other_scan_generations`, `wizard_partial_import_can_open_library`, `wizard_empty_import_offers_another_folder`, and `wizard_unknown_total_has_no_percentage`. Assert no duplicate registration/queueing, a usable PhotosIndexed result enables Open Library before scan completion, discovery alone does not enable it, nonfatal warnings persist, and Cancelled is never displayed as running. Verify opening the library leaves the scan active and progressive gallery updates visible.
- [ ] Run the new cases and observe failure before wiring.
- [ ] In `src/window/build.rs`, share the existing local-folder selection/registration/enqueue callback with onboarding rather than create another scanner. Save importing/root before registering a new folder; surface registration errors in the wizard. Hook cancellation/errors from `FileDialog::select_folder`. Retain existing ordinary import behaviour and authorization.
- [ ] Install the coordinator in `src/window/onboarding.rs`. Forward accepted scanner events after existing generation filtering, without consuming them or changing gallery updates. Match generation and root against the current ticket. Use weak references for callbacks that would otherwise create ownership cycles; detach wizard view observation on close while normal scan processing, gallery updates and persistent completion tracking continue. Use the existing top notification area for background-import status after Open Library.
- [ ] Run coordinator cases and existing scan-job tests (`cargo test scan -- --test-threads=1`). Verify existing folder button still imports normally. Commit integration and tests.

## Task 4: Startup presentation, interruption recovery and Help

**Interfaces:** Add coordinator methods `present_on_startup()` and `present_manually()`. They consume Task 1 state and Task 3 callbacks; only the startup method respects suppression. Install `win.getting-started` as a `gio::SimpleAction` on the existing application window.

- [ ] Write tests `wizard_skip_and_never_show_survive_restart`, `wizard_interrupted_import_does_not_enqueue_automatically`, `wizard_recovery_continue_queues_once`, `wizard_help_reopens_when_suppressed`, `wizard_late_picker_completion_after_close_does_not_import`, and `wizard_recovery_reuses_import_screen`. Reconstruct preferences/database between launches; test restored files, missing roots and already-active imports. Assert recovery/empty/error allow Open Library even without indexed photos, retry cannot duplicate an active scan, and manual Help does not reset completion/preferences.
- [ ] Run each GTK case individually and state tests; observe failures before implementation.
- [ ] Schedule automatic presentation once after main-window mapping and import callback installation. Settings/count errors leave startup usable and hide automatic onboarding. Render recovery through `WizardPage::Importing` with `ImportPresentation::Recovery`; show actual current scan state. An idle interrupted import requires Continue adding photos and never automatically queues a duplicate scan. Choosing another folder reuses Task 3; cancelling the picker restores the previous presentation without changing the saved root. Empty/error states reuse the same layout.
- [ ] Add a compact Help menu containing Getting started in `src/window/layout.rs` and register the window action in build. Preserve the current sidebar toggle (the variable named `menu` is a sidebar-pin button). Manual reopening works for suppressed, completed and populated libraries, retains preference state and does not reset completion or claim an unrelated scan. If there is no matching import to display, open Welcome.
- [ ] Run recovery and Help cases individually. Verify existing populated/offline libraries open normally. Commit startup integration.

## Task 5: Optional library card and responsive layout

**Interfaces:** Add `GettingStartedTips::new(on_finish: Rc<dyn Fn()>) -> GettingStartedTips` in `src/onboarding/view.rs`, with `widget() -> gtk::Widget` and `dismiss()`. This is a compact dismissible card mounted in the normal library, not a dialog page. Coordinator `show_tips_if_eligible()` uses saved preferences and the current indexed-photo count; the callback saves dismissal using Task 1 `dismiss_tips` and calls `finish_onboarding` only when the tracked import no longer needs recovery.

- [ ] Write GTK cases `wizard_tips_are_optional_and_never_show_suppresses_them`, `wizard_tips_do_not_block_lightbox_navigation`, `wizard_tips_are_inside_library`, and `wizard_narrow_layout_keeps_controls_reachable`. Assert the card appears only after entering the library, has the three tips below and Got it, causes no selection/edit side effects, persists dismissal, wraps content and never reopens the wizard as a Tips page.
- [ ] Run each GTK case individually and observe failure before implementation.
- [ ] Mount the card after Open Library, with title **Welcome to PIC**, action **Got it**, and three tips: “Double-click a photo to open it.”; “Use the sidebar to browse folders and albums.”; “Right-click photos for more actions.” Keep scanning active. Do not add a guided click sequence or require dismissal before browsing. Dismiss it before lightbox/edit flows. Retain existing offline-preview help outside this compact card rather than adding another onboarding step.
- [ ] Keep stage/root as `Importing` while the tracked scan remains unfinished, even after Open Library or card dismissal. Got it/dismissal persists `tips_dismissed` immediately; persistence failure leaves dismissal retryable. After successful scan completion with usable photos, save `Tips` if an eligible card remains undismissed; otherwise call `finish_onboarding`. Suppressed automatic tips require no card. Cancelled/failed scans retain recovery state. Preserve the never-show setting; manually reopening Help does not reset a completed library. Test dismissal before scan completion followed by restart, and successful background completion after the wizard closes.
- [ ] Verify layout at 1920×1080, 1366×768 and 360 logical pixels wide, using a floating/resizable test window so the tiling desktop cannot invalidate allocation assumptions. Preferred wizard size is approximately 600×400, clamped to parent space; vertical scrolling and wrapping keep every action and long folder path reachable. Test keyboard traversal, Escape and light/dark theme readability.
- [ ] Run tips/layout cases individually and check main-window minimum width/sidebar auto-hide remain unchanged. Commit tips and layout.

## Task 6: Whole-flow verification and review

- [ ] Run every new GTK case separately with a writable isolated `XDG_CACHE_HOME`; GTK initialization across different test threads requires separate processes. Use a temporary test library, never the user's live library.
- [ ] Run `cargo test -- --test-threads=1 --skip history_group_labels_bucket_by_edit_age_not_import_date` and `cargo build`; expect exit 0. Record the known date-sensitive exclusion explicitly; investigate any additional failures.
- [ ] Apply scoped rustfmt to new modules only, run `git diff --check`, inspect the final diff for unrelated changes, and record exact test outcomes in this plan.
- [ ] Update the linked design's UX sections to match this revision, then review against both documents, especially persistence, scan ownership, outages and narrow-window behaviour. Verify the full visible flow is Welcome → Adding photos → Library, with recovery/empty/error inside the import layout and tips inside the library. Resolve material review findings and rerun affected checks.
- [ ] Hand off the implemented branch with test evidence. Commit/push/merge according to the user's integration instructions at execution time; do not infer a new main push from previous feature pushes.

## Execution note

This is a requested planning deliverable, not an implementation start. Recommended next step is inline execution because the view, coordinator and import ticket share interfaces; review the whole feature once integrated. Perform Preparation before Task 1 coding, then execute Tasks 1–6.