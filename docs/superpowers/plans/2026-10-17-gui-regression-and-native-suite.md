# GUI Regression and Native Fedora Suite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve the 41-case isolated GUI baseline, deepen its Undo/Redo verification, and add a separate manual native Fedora Wayland/Vulkan smoke suite for a real photo folder.

**Architecture:** Keep `scripts/test-gui.sh` as the menu entry point, dispatching native work to a dedicated wrapper and Python driver before any Xvfb setup. Put renderer-evidence and outcome classification in a small pure-Python support module so the critical PASS/BLOCKED/FAIL rules can be unit-tested without a desktop; keep all native-session state under its own retained run directory.

**Tech Stack:** Bash, Python 3 standard library, existing GTK PIC binary and SQLite catalogue, GTK/GSK runtime diagnostics, Python `unittest`.

**Spec:** `docs/superpowers/specs/2026-10-17-gui-regression-and-native-suite-design.md`

## Global Constraints

- The existing full Xvfb suite remains 41 named cases.
- The Undo/Redo strengthening stays within the existing `Editor Undo/Redo sequences` case.
- Native execution uses `GDK_BACKEND=wayland`, `GSK_RENDERER=vulkan`, and GTK renderer/fallback diagnostics; environment variables alone never count as Vulkan evidence.
- Native PIC data/config/cache paths are temporary paths inside a unique retained run directory; the normal PIC data and selected source folder are never written.
- A missing Wayland/Vulkan prerequisite, canceled interaction, early app exit, or unconfirmed manual visual step yields BLOCKED.
- Native reports never overwrite the existing Xvfb `latest-report.md`.

## Review Focus

- A log containing only `GSK_RENDERER=vulkan` must not satisfy renderer verification; Task 1 tests require runtime Vulkan evidence.
- GTK log wording may vary; Task 1 tests cover known Vulkan, fallback, and ambiguous diagnostic samples, with ambiguous output remaining BLOCKED.
- A user canceling import, closing PIC during a prompt, or declining a confirmation must produce BLOCKED and preserve artifacts; Task 1 tests each terminal state.
- A confirmed import that indexes no photos or fails restart persistence is a test failure, not a pass; Task 1 tests these outcomes.
- A new edit after Undo must discard the redo branch, while original, one-edit, and two-edit preview states remain distinct; Task 3 verifies this within the existing 41st-case budget.

---

### Task 1: Native diagnostics and outcome rules

**Files:**
- Create: `scripts/gui_native_support.py`
- Create: `scripts/tests/test_gui_native_support.py`

**Interfaces:**
- Produces: `vulkan_renderer_evidence(log_text: str) -> tuple[bool, str]`, returning whether runtime log evidence identifies an initialized Vulkan renderer and a concise evidence detail.
- Produces: `classify_native_result(*, wayland_ready: bool, vulkan_ready: bool, canceled: bool, app_exited_early: bool, confirmations: dict[str, bool | None], imported_photos: int, persisted_photos: int) -> str`, returning exactly `PASS`, `FAIL`, or `BLOCKED`.
- The driver in Task 2 consumes both functions. Missing prerequisite/runtime evidence, cancellation, early close, and any absent/declined confirmation are `BLOCKED`; a confirmed interaction with zero imported photos or lost persisted rows is `FAIL`; all required evidence and checks yield `PASS`.

- [ ] **Step 1: Write failing unit tests** for environment-only Vulkan requests, explicit Vulkan initialization logs, Cairo/fallback logs, ambiguous logs, each cancellation/unconfirmed path, confirmed empty import, lost restart persistence, and complete success.
- [ ] **Step 2: Run `python3 scripts/tests/test_gui_native_support.py -v`** and confirm tests fail because the support functions do not exist.
- [ ] **Step 3: Implement the two support functions** in `scripts/gui_native_support.py`. Keep log parsing conservative: only an explicit runtime renderer identity/initialization indication is accepted.
- [ ] **Step 4: Re-run `python3 scripts/tests/test_gui_native_support.py -v`** and confirm every outcome test passes.

### Task 2: Native Fedora suite and menu dispatch

**Files:**
- Modify: `scripts/test-gui.sh`
- Create: `scripts/test-gui-native.sh`
- Create: `scripts/gui_native_test.py`
- Modify: `scripts/README-gui-tests.md`
- Create: `scripts/tests/test_gui_native_dispatch.py`
- Test: `scripts/tests/test_gui_native_support.py`

**Interfaces:**
- `scripts/test-gui.sh` maps menu choice 8 and `PIC_GUI_SUITE=native` to `scripts/test-gui-native.sh` before setting up Xvfb.
- `scripts/test-gui-native.sh` preflights the active Wayland environment and PIC binary, creates and retains a separate run directory, sets temporary XDG paths and Vulkan diagnostics, then invokes `gui_native_test.py` without Openbox/Xvfb/Cairo/software-rendering variables.
- `gui_native_test.py` launches PIC, captures application and GTK diagnostics plus GTK/GPU/driver metadata, asks the user to select a real folder in PIC's import UI, verifies catalogue rows, asks for gallery/lightbox/1:1/close confirmations, restarts PIC, verifies persistence, and writes the native-only report.

- [ ] **Step 1: Add `NativeDispatchTests.test_native_suite_is_dispatched_before_xvfb` and `test_missing_wayland_writes_blocked_report_without_launching_pic`.** Run `python3 scripts/tests/test_gui_native_dispatch.py -v`; confirm it fails because there is no native dispatch/runner. The missing-Wayland test runs against a temporary run directory, unsets `WAYLAND_DISPLAY`, and asserts status 2, a BLOCKED report, no application log, and no `xvfb.log`.
- [ ] **Step 2: Add menu dispatch and `test-gui-native.sh`.** Preserve the current path unchanged for every existing Xvfb menu choice. Capture GTK version, `vulkaninfo --summary`, and `lspci -nnk` when available; record unavailable tools in the report rather than silently omitting diagnostics.
- [ ] **Step 3: Implement `gui_native_test.py`** with signal/early-exit handling, GUI prompts whose non-affirmative responses are BLOCKED, SQLite import and restart checks, native screenshots, and retained artifacts. Keep user-library paths out of the temporary XDG tree and do not clean the run directory on completion or cancellation.
- [ ] **Step 4: Update `scripts/README-gui-tests.md`** with Fedora prerequisites, menu/env usage, manual steps, report locations, and the retained-artifact/source-safety behavior.
- [ ] **Step 5: Run `python3 scripts/tests/test_gui_native_dispatch.py -v` and `python3 -m unittest discover -s scripts/tests`;** confirm both dispatch and all script tests pass.
- [ ] **Step 6: Run `bash -n scripts/test-gui.sh scripts/test-gui-native.sh` and `python3 -m py_compile scripts/gui_native_test.py scripts/gui_native_support.py`;** confirm syntax checks pass.

### Task 3: Strengthen the existing Undo/Redo case

**Files:**
- Modify: `scripts/gui_test.py`

**Interfaces:**
- Consumes: the current `click_at`, `screenshot`, `preview_center`, `record`, and `add_case` helpers.
- Produces: stronger checks and report details within the existing `Editor Undo/Redo sequences` case; do not create another case row.

- [ ] **Step 1: Extend the existing sequence assertions** to compare the original → Exposure → Exposure+Contrast states; attempt Undo and Redo at the empty-history boundaries and verify the corresponding controls are disabled and the preview is unchanged; verify one-step Undo/Redo order; and undo once, apply a different edit, then verify Redo is disabled and cannot restore the abandoned branch. Record each measured preview delta and boundary result in the same case details.
- [ ] **Step 2: Run `PIC_GUI_SUITE=all scripts/test-gui.sh`** in the isolated Xvfb environment. Confirm the report still lists 41 cases and the expanded Undo/Redo case passes.
- [ ] **Step 3: Correct any interaction or threshold issues** without weakening state distinctions or adding a 42nd baseline case, then rerun the full baseline suite.

### Task 4: End-to-end native and baseline verification

**Files:**
- Modify: none unless verification exposes a defect in Tasks 1–3.

- [ ] **Step 1: Run `PIC_GUI_SUITE=native scripts/test-gui.sh`** in a real Fedora Wayland/Vulkan session, manually import one real photo folder, confirm the gallery, open a photo, zoom to 1:1, close it, and confirm restart persistence.
- [ ] **Step 2: Inspect the native report** for explicit active Vulkan evidence, GTK/GPU/driver diagnostics, each manual confirmation, imported/persisted counts, screenshots/logs, and a separate native latest-report path.
- [ ] **Step 3: Exercise cancellation** by canceling folder selection and confirm the result is BLOCKED while the normal PIC library is untouched and the run directory remains available.
- [ ] **Step 4: Re-run `PIC_GUI_SUITE=all scripts/test-gui.sh`** and verify the baseline report still contains exactly 41 cases and is not overwritten by the native report.
