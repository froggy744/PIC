#!/usr/bin/env bash
# Browse scope smoke checks (candidate 14): the toolbar rating picker, the
# metadata panel's photo count, sidebar Ratings, Smart Albums with older
# rating spellings, search and a library switch, on a copy of a GUI-harness
# library (checks.py says what is checked). Same private display, D-Bus,
# AT-SPI and `cargo run` setup as scripts/front-surface-checks/run.sh, which
# it runs; PIC_CHECK_REPO and PIC_CHECK_LIBRARY work the same. It never
# touches the real library or the desktop. Output:
# reports/browse-scope-checks/<time>/.
#
# Status: first run 2026-11-26, 9/9 passed. Needs one GUI harness run first
# (e.g. PIC_GUI_SUITE=viewer scripts/test-gui.sh) for the library it copies,
# or PIC_CHECK_LIBRARY pointing at a reports/gui-tests/<run> folder.
#
# Usage, from the repository root:
#   scripts/browse-scope-checks/run.sh
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PIC_CHECKS="$here/checks.py" PIC_CHECK_REPORTS=browse-scope-checks \
    exec "$here/../front-surface-checks/run.sh"
