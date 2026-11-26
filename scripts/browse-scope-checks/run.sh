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
# Status: written 2026-11-26 after candidate 14 merged and NOT YET EXECUTED;
# its AT-SPI selectors may need adjusting on the first run. Needs one GUI
# harness run first (PIC_GUI_SUITE=viewer scripts/test-gui.sh) for the
# library it copies.
#
# Usage, from the repository root:
#   scripts/browse-scope-checks/run.sh
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PIC_CHECKS="$here/checks.py" PIC_CHECK_REPORTS=browse-scope-checks \
    exec "$here/../front-surface-checks/run.sh"
