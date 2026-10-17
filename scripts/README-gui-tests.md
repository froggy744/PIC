# PIC isolated GUI regression harness

Run the complete X11 GUI regression suite from the repository root:

```sh
scripts/test-gui.sh
```

The script presents a numbered menu. Choose the full suite or a focused run ending after Startup, Library and ratings, Import, Gallery and viewer, Editing and collage, or Screensaver. Choose Complete suite to run the full 41-case Xvfb baseline followed by the native Fedora Wayland/Vulkan real-photo smoke. To skip the interactive menu, set `PIC_GUI_SUITE` to `all`, `startup`, `library`, `import`, `viewer`, `editing`, `screensaver`, `native`, or `wayland`.

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

The import checkpoint also closes and relaunches PIC, then verifies that the imported catalogue rows and generated gallery remain available. Search, album/favourite workflows, malformed-file handling, accessibility, and multiple display sizes are not yet exercised by this harness.

## Automated native Fedora Wayland/Vulkan smoke test

Choose **8) Complete suite** from the menu, or run it directly:

```sh
PIC_GUI_SUITE=wayland scripts/test-gui.sh
```

This runs the full 41-case Xvfb regression suite and then the automated Wayland/Vulkan smoke below. The 41 existing interaction cases still use Xvfb because they depend on X11 input/window-management tools; the second stage exercises PIC on the real Wayland desktop. Each stage has its own report and the combined run writes `reports/gui-tests/latest-wayland-report.md`.

To run only the native Wayland checks, choose **9) Wayland/Vulkan native smoke only** or run:

```sh
PIC_GUI_SUITE=native scripts/test-gui.sh
```

This separate suite runs the native UI checks automatically in the current Wayland desktop and requests GTK's Vulkan renderer. AT-SPI drives PIC's GTK controls, and a private `ydotoold` socket sends keyboard shortcuts. It imports real photos through PIC's folder chooser, opens a photo in the lightbox, activates 1:1, closes the viewer, restarts PIC, and verifies that the photo rows remain. No terminal answers or manual clicks are required.

By default, the suite imports `/mnt/steam/Wickus`. To test another real photo folder, set `PIC_GUI_NATIVE_PHOTO_DIR`:

```sh
PIC_GUI_NATIVE_PHOTO_DIR="$HOME/Pictures" PIC_GUI_SUITE=native scripts/test-gui.sh
```

The test gives PIC temporary HOME/config/data/cache directories and a temporary database. It reads the chosen photo folder and does not modify the normal PIC library. Before launch it checks that another PIC instance does not own the desktop application bus name; if PIC is already running, the test reports BLOCKED rather than risk using the normal library. Reports and diagnostics are written to `reports/gui-tests/native-<run-id>/`, and `latest-native-report.md` is updated. Screenshot capture is not required; the report records AT-SPI state and GTK transition logs as evidence.

The native suite requires an active Fedora Wayland session, Vulkan-capable GTK 4, `vulkaninfo`, `lspci`, `pkg-config`, Python 3 with `pyatspi`, and `ydotool` plus `ydotoold` with access to `/dev/uinput`. Install `python3-pyatspi` and `ydotool` if they are missing. The script builds `target/release/pic-rs` if needed. For an existing binary, set `PIC_GUI_APP=/path/to/pic-rs`. If Vulkan or automated Wayland input is unavailable, the report is BLOCKED and includes system diagnostics, application logs, and harness output. Test artifacts are retained for inspection.
