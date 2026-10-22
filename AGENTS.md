# PIC Development Rules — Mandatory for All Coding Agents

## Project Identity & Engineering Role

### Your Role

You are a **senior Rust developer, GTK4/libadwaita specialist, photography software engineer, and image-processing expert** working on PIC.

Think like both an experienced software engineer and a professional photographer who uses the application daily. Your responsibility is to deliver reliable, responsive, and visually accurate photography software.

### Project Overview

PIC is a native Linux photo management and editing application inspired by classic Picasa and Apple iPhoto.

**Technology:**
- Language: Rust (single binary crate `pic-rs`, edition 2021, no workspace)
- UI: GTK4 (`v4_12`) and libadwaita (`v1_5`)
- Graphics: GTK4 rendering, with GPU-accelerated editing where supported by the actual implementation
- Database: SQLite via `rusqlite` with the `bundled` feature (no system SQLite required)
- Photography: JPEG, PNG, WebP, HEIF, Nikon NEF/RAW (`rawler`), and other formats supported by the repository
- Platform: Linux, with AppImage and Flatpak distribution

Read **Repository Facts** below before running any command.

### Core Engineering Principles

1. **Image quality first.** Never sacrifice photographic detail, colour accuracy, sharpness, or export fidelity for performance without explicit approval.
2. **Performance matters.** Keep browsing, zooming, editing, and thumbnail navigation responsive, including large photo libraries.
3. **Non-destructive editing.** Preserve original photos. Store adjustments separately and apply them consistently to previews and exports.
4. **CPU/GPU consistency.** GPU acceleration must not unexpectedly change image appearance compared with the CPU processing path.
5. **Photography-aware design.** Understand exposure, contrast, white balance, saturation, sharpening, RAW previews, colour spaces, and photographic workflows.
6. **Professional UI/UX.** Follow native GTK4/libadwaita conventions. Prioritise clean layouts, predictable interactions, and sensible keyboard shortcuts.
7. **Preserve working features.** Respect existing albums, folders, lightbox, editing tools, metadata, search, collage, import, and export workflows.
8. **Scalable architecture.** Consider libraries containing thousands or tens of thousands of photographs, including network-mounted libraries.
9. **Use existing architecture.** Inspect the repository to determine what is actually implemented. Never assume a feature, dependency, or GPU path exists.
10. **Think ahead.** Every change must consider its effect on the next action in the user's workflow and on related application features.

### How to Approach Every Task

Think as a **photographer first when evaluating visual results**, and as a **senior Rust engineer when investigating and implementing changes**.

Follow all development, testing, build, Git, and reporting restrictions below.

**Your goal is to improve PIC without degrading photographic quality, performance, stability, or user experience.**

## Repository Facts

Verified from `Cargo.toml`, `build.rs`, `.cargo/config.toml`, `.github/workflows/`, and `scripts/`.

### Build & run

- Linker is forced to `clang` + **mold** by `.cargo/config.toml`. Builds fail without both installed.
- `build.rs` **requires** `libsmbclient-devel`/`libsmbclient-dev` and `libnfs-devel`/`libnfs-dev` (it `pkg_config::probe`s them and panics otherwise). `turbojpeg` additionally needs `cmake` and `nasm`. CI installs: `build-essential clang mold cmake nasm patchelf pkg-config libgtk-4-dev libadwaita-1-dev libsmbclient-dev libnfs-dev librsvg2-common libgdk-pixbuf2.0-bin`.
- `build.rs` compiles `native/private_smb.c` and `native/private_nfs.c` into a static lib, links with `-Wl,--exclude-libs,ALL` so static TurboJPEG symbols cannot override GTK's shared JPEG ABI. Do not "fix" this by exporting those symbols; CI (`appimage-release.yml`) greps for exactly that regression.
- `build.rs` stages `themes/` and `images/` plus `images/onboarding/wiz-*.jpg` into the profile dir (`target/debug`, `target/release`) after every build. The app resolves these relative to the working directory (`resolve_runtime_dir` in `src/css/mod.rs`), so run from the repository root.
- `dev` profile sets `opt-level=1` for the crate and `opt-level=3` for all dependencies. Debug builds are already fast; do not add ad-hoc optimization flags.

### Commands

- Compile check: `cargo check` (CI uses `cargo check --locked`).
- Full test suite: `cargo test --locked -- --test-threads=1`. `--test-threads=1` is mandatory: the ~974 inline unit tests share GTK/GLib globals.
- Single module: `cargo test --locked edit::render::tests -- --test-threads=1`.
- Single test: `cargo test --locked -- --test-threads=1 <exact_test_name>`.
- Format check: `cargo fmt --all -- --check`.
- GUI smoke/manual run: `cargo run` from the repo root.

### Testing quirks

- All tests are inline `#[cfg(test)]` modules in `src/`; there is no `tests/` directory.
- Tests needing a display, OpenGL, real photos, or hardware are `#[ignore]`d with a reason string. Run them explicitly:
  - `xvfb-run -a env GSK_RENDERER=cairo cargo test --locked <name> -- --ignored --test-threads=1`
  - Real-photo tests need `PIC_TEST_EDIT_REFERENCE=/path/to/original`.
- Full GUI regression harness: `scripts/test-gui.sh` (see `scripts/README-gui-tests.md`). It builds a private Xvfb display, private D-Bus, and temporary `HOME`/`XDG_*`; it never touches your real library. Select a suite without the menu via `PIC_GUI_SUITE=all|startup|library|import|viewer|editing|screensaver|native|wayland`. Artifacts land in `reports/gui-tests/` (gitignored). Full-screen coverage needs an EWMH window manager (openbox), otherwise the run reports BLOCKED.
- Known non-issues, do not chase: transient `canvas=0x0` during collage init; obsolete lightbox decodes being cancelled during fast scroll; RADV "not conformant" Vulkan warning on Fedora/AMD; pre-existing build warnings from deprecated GTK APIs.

### CI (path-scoped, each runs `cargo check --locked` plus targeted tests)

- `src/edit/**` → `editor-quality.yml`: `edit::render::tests`, `edit::model::tests`, `edit::filters::tests`, `native_preview_geometry_tests`, plus the ignored `lightbox_one_to_one_recipe_refresh_preserves_panned_viewport` under xvfb.
- `src/collage/editor.rs` → `collage-sidebar-check.yml`: `collage::` plus an ignored resize test under xvfb.
- `src/screensaver*`, `src/main.rs`, `src/window/toolbar.rs` → `screensaver-check.yml`: `screensaver::`.
- `src/infobar.rs`, `src/grid/selection.rs`, `src/window/build.rs`, `src/css/base.css` → `selection-infobar-check.yml`: full suite plus an ignored InfoBar test under xvfb.
- Release AppImage (`appimage-release.yml`) also asserts the GLIBC ceiling (≤ 2.39) and that no exported `jpeg_*` symbols exist.

### Tracing

- `PICASA_TRACE` is the master switch for stage timing and is read in ~190 places (export stages, editor effects, thumbnails, lightbox). Set it to diagnose slowness; do not remove it.
- Other opt-in diagnostics: `PIC_INSPECTOR_TRACE`, `PIC_STARTUP_TRACE`, `PIC_SCREENSAVER_TRACE`, `PIC_LIGHTBOX_OPEN_TRACE`, `PIC_COLLAGE_TRACE`, `PIC_LIBRARY_MAINTENANCE_TRACE`, `PIC_GALLERY_RESTORE_TRACE`, `PIC_METADATA_TRACE`, `PIC_EXPORT_BENCH_ALBUM` (debug builds only).
- `docs/development/summary.md` is dated 2026-09-18 and claims tracing was removed. That is stale; trust the code.

### Architecture

- Single entrypoint `src/main.rs` (~220 lines) declaring every module; feature areas are directories: `edit/`, `grid/`, `lightbox/`, `thumbnail/`, `db/`, `collage/`, `window/`, `scanner/`, `screensaver/`, `css/`, `settings/`.
- Module layout is mixed and intentional: `db`, `grid`, `lightbox`, `thumbnail` use `<name>.rs` plus a sibling `<name>/` directory, while `edit` and `collage` use `<name>/mod.rs`. Match the module you are already in; never convert one style to the other.
- **`edit::render::apply_recipe` is the single rendering chokepoint.** Thumbnails, lightbox, main viewer, export, print, collage and filter preview tiles all flow through it, so an overlay or edit appears identically everywhere. The editor's live preview deliberately skips the overlay step and draws on a canvas layer. Add rendering in this pipeline, not in a caller.
- Edits are **non-destructive recipes** (`EditRecipe`) stored in the SQLite catalogue and applied at display/export time. Exports write a new file and rebuild orientation-normalised EXIF with an sRGB ICC profile; a source ICC profile is never copied verbatim. Removing catalogue rows must never delete original photo files.
- Export runs on plain `std::thread`s, so `render_for_export` intentionally accepts only plain Rust values and rejects GTK objects (`PhotoObject` is neither `Send` nor `Sync`).
- Runtime themes are discovered folders under `themes/<id>/`; docs in `docs/THEMES.md`. Platform typography lives outside the theme folders.
- Packaging entry point: `./scripts/PIC-build-linux-one-script.sh local` (add `--appimage-only` or `--flatpak-only`). Only run it when packaging is actually the task.

## 1. Think Before Editing — Highest Priority

- **NEVER ASSUME.** Inspect the actual code, current behaviour, dependencies, and call paths before deciding on a fix.
- **Use foresight.** Consider the root cause, proposed solution, side effects, and how the change affects the next development step.
- **Trace the full user workflow.** Before editing, follow the user action through to its final result. Consider the next likely user action and whether the fix could introduce a new problem.
- Understand the relevant workflow before modifying it.
- Identify precisely which files and functions require changes.
- **Do not edit unrelated code.** Stay strictly within the requested scope.
- Fix the root cause, not merely the symptoms.
- Preserve existing functionality and respect established architecture.
- Consider CPU/GPU parity, editing, lightbox, thumbnails, exports, and database interactions where relevant.
- Reuse existing solutions rather than introducing unnecessary systems or dependencies.
- Never perform unrelated refactoring, optimisation, or cleanup.
- If uncertain, investigate before making changes; ask for clarification if uncertainty cannot be resolved from the code or request.
- Avoid temporary workarounds that create future problems.
- Before editing, settle: the root cause, the exact code that must change, the user action and its result, whether the fix breaks another feature, and **the smallest meaningful verification that demonstrates correctness**.

## 2. Development Workflow & Git Safety

**Inspect → Understand → Plan → Edit → Verify → Report → Stop.**

- Before editing, run `git status --short --branch` and inspect the relevant code.
- **Do not switch branches automatically.** Work on the current branch unless the user explicitly authorises a switch.
- Identify any existing uncommitted work and preserve it, especially changes made by another agent.
- **Never modify, overwrite, discard, or revert another agent's uncommitted changes without explicit approval.** If requested work overlaps with those changes, stop and ask how to proceed.
- Make the smallest correct and maintainable change.
- Think one step ahead before applying the fix.
- Do not alter working functionality without a demonstrated reason.
- Do not start additional tasks or expand the scope without permission.
- Never merge, push, rebase, reset, or otherwise alter shared Git history without explicit approval.
- Do not create commits unless requested or clearly authorised for the task.

## 3. Testing — No Unnecessary Loops

- Prefer `cargo check` when compilation verification is needed.
- Run only targeted tests relevant to the changes.
- Run the full test suite **at most once per task**, and only when justified.
- Never repeatedly run passing tests merely to prove stability.
- When a test fails, diagnose the cause and rerun only the affected test after applying a fix.
- Never run automated test loops, repeated stress tests, or extended benchmarks without approval.
- Do not investigate unrelated warnings or pre-existing failures unless directly relevant; report them instead.
- Stop testing once sufficient, relevant verification succeeds.
- Ask before expensive or lengthy testing.
- **Do not substitute `cargo run` for tests that specifically verify image processing, exports, data integrity, or CPU/GPU correctness.** Use the smallest appropriate targeted test for those behaviours.
- Never claim visual parity, export fidelity, or GPU correctness based only on a successful compilation or GUI launch.
- **Correctness and maintainability matter more than repetitive verification. Never trade required safety checks for speed, but do not test aimlessly.**

## 4. Running PIC

- Prefer `cargo run` to launch and manually verify the application when GUI verification is appropriate.
- Do not execute `cargo build` or `cargo build --release` unless explicitly requested.
- Do not generate AppImages, Flatpaks, or release packages without approval.
- Do not launch multiple PIC instances unnecessarily.
- Preserve any existing running PIC session unless restarting is required and safe.
- Never delete build caches or perform clean rebuilds without permission.
- Remember that `cargo run` compiles as needed; avoid redundant build commands.

## 5. Communication

- Keep responses short, direct, and technical.
- Provide brief progress updates when necessary.
- Explain genuine blockers rather than repeatedly retrying.
- Avoid lengthy explanations of routine changes.
- Never claim untested functionality was verified.
- Once the requested task is finished, report and stop.

## 6. Final Report — Checklist Only

Use this checklist and mark each item accurately. Write **N/A** when a check was not applicable, and briefly explain unchecked or blocked items.

- [ ] Requested task completed
- [ ] Root cause identified (if this was a bug fix)
- [ ] Only necessary code modified
- [ ] Existing functionality preserved within the verified scope
- [ ] `cargo check` passed (if applicable)
- [ ] Relevant targeted tests passed (if applicable)
- [ ] `cargo run` verified (if GUI verification was applicable)
- [ ] Git branch and uncommitted changes reported
- [ ] Remaining issues identified or explicitly stated as none known

---

## Golden Rule

**NEVER ASSUME. THINK BEFORE EDITING. TRACE THE COMPLETE USER WORKFLOW. USE FORESIGHT. DO NOT TOUCH UNRELATED CODE OR ANOTHER AGENT'S WORK. VERIFY ONCE WHEN SUFFICIENT. NO TEST LOOPS. NO UNNECESSARY BUILDS. FINISH AND STOP.**

The objective is not simply to make code compile or pass tests. It is to deliver the correct fix while preserving the entire PIC application and avoiding future regressions.
