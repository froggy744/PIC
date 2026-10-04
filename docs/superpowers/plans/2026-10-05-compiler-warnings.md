# Compiler Warning Cleanup Implementation Plan

Goal: Reduce the warnings in the supplied successful build on cleanup/compiler-warnings, preserving PIC behavior.
Architecture: Review each diagnostic against current main. Remove unused bindings and genuinely obsolete helpers, and migrate supported GTK APIs in small batches. Avoid blanket lint suppression and broad UI redesign.
Tech stack: Rust, GTK 4.12+, libadwaita 1.5+, native Samba/NFS.
Spec: User requested most compiler warnings fixed on this branch and will test the app later.

## Constraints and review focus
- Keep persisted settings, exports, authentication and database behavior compatible.
- Preserve allocation coordinates and CSS provider scope.
- Preserve dialog cancel, close, retry and selection behavior.
- Keep test-only helpers under cfg(test), rather than deleting test coverage.
- Never claim a build or test passed unless it ran successfully.

## Tasks
- [x] 1. Prepare branch and capture baseline diagnostics; resolve verification dependencies.
- [x] 2. Remove unused imports, variables and duplicate match arms; replace identical CSS string API calls.
- [x] 3. Migrate dropdowns and dialogs while preserving callbacks and cancellation; audit dead helpers.
- [ ] 4. Compile/check tests, compare warning counts, independently review changes, push cleanup branch and report exact verification limits.

Verification: cargo check --all-targets; cargo test -- --test-threads=1; selected GTK tests under Xvfb; git diff --check. If native dependencies cannot be installed, record the blocker and provide branch checks for the user's environment.

Results and verification record: ../../compiler-warning-cleanup.md. User authorized continuous execution; no approval checkpoint was introduced.
