# Compiler warning cleanup

Branch: `cleanup/compiler-warnings`
Baseline: `main` at `e2a0e73`.

## Results

The app build has **48 Rust warnings**, down from **191** (143 removed, approximately 75%). No blanket lint suppression was added.

| Warning | Before | After |
| --- | ---: | ---: |
| Deprecated APIs | 121 | 20 |
| Dead code | 54 | 28 |
| Unused variables | 13 | 0 |
| Unused imports | 1 | 0 |
| Unnecessary braces | 1 | 0 |
| Duplicate match pattern | 1 | 0 |
| Total | 191 | 48 |

## Changes

- Use supported CSS string loading APIs without changing provider scope or priority.
- Replace the database recovery/confirmation, database creation, photo export and collage export dialogs with libadwaita alert dialogs.
- Replace native file choosers with a shared asynchronous GTK FileDialog helper. Keep selection filters, default filenames, local-path handling, cancellation guards and background export/import/restore work.
- Replace font and RAW/JPEG preference ComboBoxText widgets with DropDown widgets. Preserve saved preference keys and fonts referenced by recipes even when the font is not installed.
- Remove unused imports, callback arguments, clones, an unreachable duplicate theme alias, and obsolete unused wrappers.
- Compile helpers used only by tests under cfg(test), retaining their existing regression coverage.
- Correct native SMB scan buffer sizes so the subnet prefix plus a host suffix cannot be truncated. The legacy smbc_init deprecation remains.

## Verification

- `cargo check --all-targets`: passed.
- `cargo check --bin pic-rs`: passed; warning counts above captured from compiler JSON on both branches using the same dependencies/toolchain.
- Existing suite: **526 passed, 0 failed, 66 ignored**. Run with `--test-threads=1`; dev/test optimization and debug-info overrides were used to reduce verification build cost. No Cargo configuration changes were committed.
- Native SMB source: GCC syntax check with `-O2 -Wall -Wextra -Werror=format-truncation` passed; only the existing smbc_init deprecation remained.
- Changed dialog/dropdown files parse and pass rustfmt checks; diff whitespace check passed, treating the repository's existing CRLF line endings correctly.
- Independent static review completed; reported build blockers were corrected and fresh compiler/test checks passed afterward.

A targeted GTK CSS test could not run: Xvfb cannot create listening sockets in this environment, so GTK initialization fails before the test checks CSS. GUI behavior remains for testing on a desktop. The 66 ignored tests also include display and external-photo-fixture tests.

## Desktop checks before merging

1. Open/create a database and select a backup for restore; verify cancel performs no action.
2. Export a single photo, a photo selection and a collage; verify filenames, formats and cancellation.
3. Import an overlay and edit text font selection, including a recipe with an unavailable font.
4. Change RAW/JPEG pair preference and reopen settings to confirm persistence.
5. Check settings close/reopen, thumbnail rendering and existing grid/wall resize behavior.

The remaining Rust warnings primarily concern allocation/style-context APIs, the network picker, and unused renderer/compatibility code. They are retained for focused follow-up rather than broad renderer changes in this cleanup.
