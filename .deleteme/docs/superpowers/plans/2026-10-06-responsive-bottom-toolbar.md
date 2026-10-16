# Restore narrow window resizing and responsive sidebar

Branch: `fix/responsive-bottom-toolbar`, based on main `cc38a4c`.

## Cause

The full application reproduced a minimum width of 1080 px. The sidebar enters compact mode at 1050 px, so a native resize could stop before auto-hide ran. The bottom toolbar requested 814 px and the expanded sidebar added 266 px. Temporarily removing the Import control lowered the limit to 1040 px. Commit `c0fa9b1` (4 October 2026, 15:47 SAST) added that control.

The hidden Albums page also requested 747 px through the horizontally homogeneous main stack, preventing narrower resizing even after the sidebar collapsed.

## Fix

- [x] Create a separate branch from current main.
- [x] Reproduce the original stop with a failing full-window GTK regression.
- [x] Below 900 px of toolbar space, move secondary actions into a labelled overflow popover, retaining the original widgets, handlers, sensitivity, and nested Album menu.
- [x] Keep thumbnail zoom, Favourite, Rating and Edit directly accessible; hide filename presentation below 700 px.
- [x] Restore the original action order when the toolbar widens.
- [x] Let the active main-stack page determine horizontal size, so hidden Albums content does not constrain Photos.
- [x] Verify full-window resize cycles at 1920, 960, 600 and 1440 px, including sidebar hide and restore.
- [x] Verify selected/unselected photo actions, overflow activation, nested Album menu and repeated wide/compact transitions.
- [x] Run existing infobar styling and Masonry/Photo Wall resize checks, the default suite, build and whitespace checks.

## Validation setup

The desktop has two 1920×1080 monitors at scale 1. Tests use the real GTK display and an isolated temporary database/cache. The resize tests make bounded repeated nearby size requests while responsive content releases its previous minimum, representing continued native dragging. A permanent size floor still fails.

The full-window regression failed before implementation (`requested 1000, actual 1080`). The first implementation check reached 599 px, hid the sidebar, restored it at 1440 px and narrowed again. Final post-outage checks are recorded below when complete.

## Final verification

- Full application: 1920 → 960 → 600 → 1440 → 960 → 600 px; reached 1920/1440 px expanded and 959/599 px compact. Sidebar hidden at narrow sizes and visible again when widened.
- Toolbar: selected and unselected photos, 960/600/1400 px and repeated reversals. Secondary controls remained mapped in the drawer, Add to Album's nested popover opened, sensitivity followed selection, and the original Import handler fired exactly once per activation after each move.
- Metadata now receives a budget after controls and filename presentation. This also covers the selected-photo case where fixed metadata thresholds alone imposed a larger minimum.
- Existing GTK checks pass: infobar themed full-width background, deep-scrolled Masonry all-column frame synchronization, and Photo Wall width scaling.
- Default suite passes with the previously reproduced main-branch history-date failure excluded: **530 passed, 70 ignored, 1 filtered out**. `cargo build`, scoped formatting and `git diff --check` pass.
- The outage damaged the package's incremental linker artifacts; clearing only generated `pic-rs` build output and rebuilding recovered verification. Dependencies and source work were retained.

Fast native mouse dragging on other themes/scaling configurations remains a manual check. The user authorized committing and pushing this branch and integrating it into main after verification.
