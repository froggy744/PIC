#!/usr/bin/env bash
# Library switch and Exit checks: two copies of a GUI-harness library, driven
# through Settings > Libraries and the Exit confirmation (checks.py says
# what is checked). Same private display, D-Bus, AT-SPI and `cargo run` setup
# as scripts/front-surface-checks/run.sh, which it runs; PIC_CHECK_REPO and
# PIC_CHECK_LIBRARY work the same. Output: reports/library-switch-checks/<time>/.
#
# Usage, from the repository root:
#   scripts/library-switch-checks/run.sh
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PIC_CHECKS="$here/checks.py" PIC_CHECK_REPORTS=library-switch-checks \
    exec "$here/../front-surface-checks/run.sh"
