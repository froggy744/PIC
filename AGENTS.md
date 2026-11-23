# PIC — Development Rules for Coding Agents

PIC is a native Linux photo manager/editor (Rust, GTK4 `v4_12` + libadwaita `v1_5`, SQLite via `rusqlite` bundled, RAW via `rawler`). Single binary crate `pic-rs`, edition 2021, no workspace. AppImage/Flatpak distribution.

Engineering priorities, in order: photographic image quality, responsiveness on large libraries, non-destructive editing (edits are stored as `EditRecipe` recipes applied at display/export time; originals are never modified or deleted), and CPU/GPU visual parity. Follow native GTK4/libadwaita conventions and preserve working features. **Never assume a feature, dependency, or GPU path exists — inspect the code.**

All facts below were verified from `Cargo.toml`, `build.rs`, `.cargo/config.toml`, `.github/workflows/`, and `scripts/`.

## Build & run

- Linker is forced to `clang` + **mold** by `.cargo/config.toml`; builds fail without both installed.
- `build.rs` **requires** `libsmbclient-dev` and `libnfs-dev` (it `pkg_config::probe`s them and panics otherwise). `turbojpeg` additionally needs `cmake` and `nasm`. Full CI dependency list: `build-essential clang mold cmake nasm patchelf pkg-config libgtk-4-dev libadwaita-1-dev libsmbclient-dev libnfs-dev librsvg2-common libgdk-pixbuf2.0-bin`.
- `build.rs` compiles `native/private_smb.c` + `native/private_nfs.c` into a static lib linked with `-Wl,--exclude-libs,ALL` so static TurboJPEG symbols cannot override GTK's shared JPEG ABI. Do not "fix" this by exporting those symbols; `appimage-release.yml` greps for exactly that regression (`jpeg_*` symbols) and also enforces a GLIBC ceiling (≤ 2.39).
- `build.rs` stages `themes/` and `images/` into the profile dir (`target/debug`) after every build; the app resolves these relative to the working directory (`resolve_runtime_dir` in `src/css/mod.rs`), so **always run from the repository root**.
- `dev` profile is already tuned (`opt-level=1` for the crate, `opt-level=3` for all dependencies). Do not add ad-hoc optimization flags.

## Commands

- Compile check: `cargo check` (CI uses `cargo check --locked`).
- Full test suite: `cargo test --locked -- --test-threads=1`. `--test-threads=1` is **mandatory**: the ~977 inline unit tests share GTK/GLib globals.
- Single module: `cargo test --locked edit::render::tests -- --test-threads=1`.
- Single test: `cargo test --locked -- --test-threads=1 <exact_test_name>`.
- Format check: `cargo fmt --all -- --check`.
- Manual GUI run: `cargo run` from the repo root.
- Presentation-owner guard: `scripts/check-presentation-owners.sh` (no build needed; `--self-test` proves its rules). Run it after changing sidebar or Properties wiring.

## Testing quirks

- All Rust tests are inline `#[cfg(test)]` modules in `src/`. `tests/` holds only `compact_sidebar_x11_drag.py`, a standalone Python X11 drag repro (run it under Xvfb only).
- Tests needing a display, OpenGL, real photos, or hardware are `#[ignore]`d with a reason string. Run explicitly:
  - `xvfb-run -a -s "-screen 0 1920x1080x24" env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GSK_RENDERER=cairo cargo test --locked <name> -- --ignored --test-threads=1`
  - Keep the screen size: Xvfb defaults to 640×480, below the 1050 px sidebar breakpoint, so window tests start with a collapsed sidebar and fail misleadingly. Keep `-u WAYLAND_DISPLAY GDK_BACKEND=x11`: on a Wayland session GTK otherwise ignores Xvfb and opens the test windows on the user's real desktop.
  - Real-photo tests need `PIC_TEST_EDIT_REFERENCE=/path/to/original`.
- Full GUI regression harness: `scripts/test-gui.sh` (see `scripts/README-gui-tests.md`). Private Xvfb display, private D-Bus, temporary `HOME`/`XDG_*` — it never touches your real library. Select a suite via `PIC_GUI_SUITE=all|startup|library|import|viewer|editing|screensaver|native|wayland`. Artifacts land in `reports/gui-tests/` (gitignored). Full-screen coverage needs an EWMH window manager (openbox), otherwise the run reports BLOCKED.
- Known non-issues, do not chase: transient `canvas=0x0` during collage init; obsolete lightbox decodes cancelled during fast scroll; RADV "not conformant" Vulkan warning on Fedora/AMD; pre-existing build warnings from deprecated GTK APIs.

## CI

Workflows in `.github/workflows/` are path-scoped: each runs `cargo check --locked` plus targeted tests when its `paths:` filter matches. Read those filters to see what a change triggers. Inspect a workflow's inline scripts before running any of them locally; some build real GTK windows.

## Tracing & diagnostics

Opt-in environment variables read at runtime. Never remove them. `PICASA_TRACE` is the master switch for diagnosing slowness; the full list is in `docs/development/TRACING.md`.

## Architecture

- Single entrypoint `src/main.rs` (~220 lines) declaring every module; feature areas are directories: `edit/`, `grid/`, `lightbox/`, `thumbnail/`, `db/`, `collage/`, `window/`, `scanner/`, `screensaver/`, `css/`, `settings/`.
- Module layout is mixed and **intentional**: `db`, `grid`, `lightbox`, `thumbnail` use `<name>.rs` + sibling `<name>/` directory; `edit` and `collage` use `<name>/mod.rs`. Match the style you are already in; never convert one style to the other.
- **`edit::render::apply_recipe` (`src/edit/render.rs`) is the single rendering chokepoint.** Thumbnails, lightbox, main viewer, export, print, collage, and filter preview tiles all flow through it, so an edit appears identically everywhere. The editor's live preview deliberately skips the overlay step and draws on a canvas layer. Add rendering here, not in a caller.
- Exports write a new file and rebuild orientation-normalised EXIF with an sRGB ICC profile; a source ICC profile is never copied verbatim.
- Export runs on plain `std::thread`s, so `render_for_export` intentionally accepts only plain Rust values and rejects GTK objects (`PhotoObject` is neither `Send` nor `Sync`).
- Runtime themes are discovered folders under `themes/<id>/` (docs: `docs/THEMES.md`); platform typography lives outside the theme folders.
- Packaging entry point: `./scripts/PIC-build-linux-one-script.sh local` (add `--appimage-only` or `--flatpak-only`). Only run when packaging is actually the task.

## Working rules

- Before editing: `git status --short --branch`, then read the relevant code. Fix root causes, not symptoms. Stay strictly within the requested scope; no unrelated refactoring, cleanup, or new dependencies.
- Never modify, revert, or discard another agent's uncommitted changes without explicit approval. Do not switch branches, commit, merge, push, rebase, or reset without explicit authorisation.
- Prefer `cargo check` for compile verification. Run targeted tests only; run the full suite at most once per task and only when justified. Never run test loops or benchmarks without approval. Report pre-existing failures/warnings instead of investigating them.
- Do not run `cargo build`/`--release`, packaging scripts, or clean rebuilds unless explicitly requested. Do not launch multiple PIC instances.
- Never claim visual parity, export fidelity, or GPU correctness from a successful compile or GUI launch alone — use the smallest targeted test that verifies the behaviour.
- Report concisely: what changed, how it was verified, and the state of the branch/uncommitted changes. Then stop.

## Codebase Index

Before investigating or changing a feature, consult
`AUDIT-INDEX.md` to identify its implementation,
dependencies and relevant tests.

Treat the index as a navigation guide, not as the
source of truth. Verify against current source code.

Architecture refactors (deepening candidates): `docs/architecture/STATUS.md` is the status tracker. Read it before proposing or continuing one, and update it when a candidate starts or merges.

Do not modify unrelated code or settings.

