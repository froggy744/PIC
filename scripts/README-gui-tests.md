# PIC isolated GUI regression harness

Run the complete X11 GUI regression suite from the repository root:

```sh
scripts/test-gui.sh
```

The script presents a numbered menu. Choose the full suite or a focused run ending after Startup, Library and ratings, Import, Gallery and viewer, Editing and collage, or Screensaver. Focused runs are ordered checkpoints in the existing GUI journey: a later checkpoint runs its earlier prerequisite steps first, then stops at the selected area. The report records the selection. To skip the interactive menu, set `PIC_GUI_SUITE` to `all`, `startup`, `library`, `import`, `viewer`, `editing`, or `screensaver`.

The harness always creates a fresh Xvfb display, wraps the app in a private D-Bus session, and uses temporary `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, and generated photos under `reports/gui-tests/<run-id>/`. It never opens the host library database or uses original photos. Screenshots and application/window-manager/Xvfb diagnostics are saved in that run directory; `latest-report.md` is refreshed after every run.

The full suite requires an X11 window manager so it can validate the fullscreen screensaver. Openbox is discovered automatically and starts only on the harness's Xvfb display with fresh config/data directories. `PIC_GUI_WM_BIN` can select another EWMH-compatible X11 window manager. If no window manager is available, the run is reported BLOCKED. Set `PIC_GUI_REQUIRE_WM=0` only for basic smoke exploration; that run cannot complete fullscreen coverage. No live desktop display is used.

## Fedora

```sh
sudo dnf install xorg-x11-server-Xvfb xdotool ImageMagick dbus-daemon python3-pillow openbox wmctrl
```

## Ubuntu

```sh
sudo apt install xvfb xdotool imagemagick dbus-daemon python3-pil openbox wmctrl
```

The preflight checks `Xvfb`, `xdotool`, ImageMagick `import` and `identify`, `dbus-run-session`, Python 3, Pillow, and the window manager/`wmctrl` needed for fullscreen verification. Missing dependencies generate a BLOCKED report and print the matching package guidance. Release mode is built automatically if `target/release/pic-rs` is absent.

To run with a specific Xvfb or window-manager executable:

```sh
XVFB_BIN=/path/to/Xvfb PIC_GUI_WM_BIN=/path/to/openbox PIC_GUI_REQUIRE_WM=1 scripts/test-gui.sh
```

Test artifacts are intentionally retained. Remove a run directory manually only after reviewing its contents.

The import checkpoint also closes and relaunches PIC, then verifies that the imported catalogue rows and generated gallery remain available. Search, album/favourite workflows, malformed-file handling, accessibility, Wayland, and multiple display sizes are not yet exercised by this harness.
