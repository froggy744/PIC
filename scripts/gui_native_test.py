#!/usr/bin/env python3
"""Automated native Wayland/Vulkan smoke test using an isolated PIC library."""

from __future__ import annotations

import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import time
import traceback

from gui_native_support import (
    classify_native_result,
    native_input_socket_path,
    native_lightbox_evidence,
    vulkan_renderer_evidence,
)

try:
    import pyatspi
except ImportError:  # The runner reports a clear BLOCKED result if unavailable.
    pyatspi = None


RUN_DIR = Path(os.environ["PIC_GUI_RUN_DIR"])
SCREENSHOTS = RUN_DIR / "screenshots"
APP = Path(os.environ["PIC_GUI_APP"])
APP_LOG = RUN_DIR / "application.log"
DB_PATH = Path(os.environ["XDG_DATA_HOME"]) / "pic-rs" / "library.db"
PHOTO_FOLDER = Path(os.environ["PIC_GUI_NATIVE_PHOTO_DIR"]).resolve()
YDOTool = os.environ.get("PIC_GUI_YDOTOOL_BIN", "ydotool")
YDTOOL_DAEMON = os.environ.get("PIC_GUI_YDOTOOLD_BIN", "ydotoold")
YDTOOL_SOCKET = native_input_socket_path(os.environ["XDG_RUNTIME_DIR"], os.getpid())
COMMIT = os.environ.get("PIC_GUI_COMMIT", "unknown")
APP_PROCESS: subprocess.Popen | None = None
INPUT_PROCESS: subprocess.Popen | None = None
INPUT_LOG_HANDLE = None
LOG_HANDLE = None
STATE = {
    "canceled": False,
    "app_exited_early": False,
    "confirmations": {
        "application_visible": None,
        "gallery": None,
        "lightbox": None,
        "zoom_1_to_1": None,
        "lightbox_closed": None,
    },
    "source_folder": None,
    "imported_photos": 0,
    "persisted_photos": 0,
    "baseline_roots": set(),
    "baseline_photo_count": 0,
    "screenshots": [],
    "screenshot_errors": [],
    "driver_error": None,
}


def command(args: list[str], *, timeout: float = 15, env=None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, text=True, capture_output=True, timeout=timeout, env=env)


def diagnostic_command(label: str, args: list[str], timeout: float = 20) -> str:
    try:
        result = command(args, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as error:
        return f"{label}: unavailable ({error})"
    output = (result.stdout + result.stderr).strip()
    if result.returncode != 0:
        return f"{label}: command exited {result.returncode}\n{output or '(no output)'}"
    return f"{label}:\n{output or '(no output)'}"


def collect_system_diagnostics() -> None:
    lines = [
        f"GDK_BACKEND={os.environ.get('GDK_BACKEND', '')}",
        f"GSK_RENDERER={os.environ.get('GSK_RENDERER', '')}",
        f"GSK_DEBUG={os.environ.get('GSK_DEBUG', '')}",
        f"WAYLAND_DISPLAY={os.environ.get('WAYLAND_DISPLAY', '')}",
        diagnostic_command("GTK version", ["pkg-config", "--modversion", "gtk4"]),
        diagnostic_command("Vulkan devices and driver", ["vulkaninfo", "--summary"], timeout=40),
        diagnostic_command("GPU and kernel driver", ["lspci", "-nnk"]),
        diagnostic_command("Wayland input daemon", [YDTOOL_DAEMON, "--version"]),
        f"AT-SPI available: {pyatspi is not None}",
    ]
    (RUN_DIR / "system-diagnostics.txt").write_text("\n\n".join(lines) + "\n")


def app_alive() -> bool:
    return APP_PROCESS is not None and APP_PROCESS.poll() is None


def start_app() -> None:
    global APP_PROCESS, LOG_HANDLE
    LOG_HANDLE = APP_LOG.open("a", buffering=1)
    APP_PROCESS = subprocess.Popen(
        [str(APP)],
        env=os.environ.copy(),
        stdout=LOG_HANDLE,
        stderr=subprocess.STDOUT,
    )


def stop_app() -> None:
    global APP_PROCESS, LOG_HANDLE
    if APP_PROCESS is not None and APP_PROCESS.poll() is None:
        APP_PROCESS.terminate()
        try:
            APP_PROCESS.wait(timeout=8)
        except subprocess.TimeoutExpired:
            APP_PROCESS.kill()
            APP_PROCESS.wait(timeout=4)
    APP_PROCESS = None
    if LOG_HANDLE is not None:
        LOG_HANDLE.close()
        LOG_HANDLE = None


def start_input_daemon() -> None:
    global INPUT_PROCESS, INPUT_LOG_HANDLE
    socket_dir = YDTOOL_SOCKET.parent
    socket_dir.mkdir(parents=True, exist_ok=True)
    INPUT_LOG_HANDLE = (RUN_DIR / "input-daemon.log").open("a")
    INPUT_PROCESS = subprocess.Popen(
        [YDTOOL_DAEMON, "-p", str(YDTOOL_SOCKET)],
        stdout=subprocess.DEVNULL,
        stderr=INPUT_LOG_HANDLE,
    )
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        if INPUT_PROCESS.poll() is not None:
            raise RuntimeError("ydotoold exited before creating its input socket")
        if YDTOOL_SOCKET.exists():
            os.environ["YDOTOOL_SOCKET"] = str(YDTOOL_SOCKET)
            return
        time.sleep(0.1)
    raise RuntimeError(f"Timed out waiting for ydotoold socket at {YDTOOL_SOCKET}")


def stop_input_daemon() -> None:
    global INPUT_PROCESS, INPUT_LOG_HANDLE
    if INPUT_PROCESS is not None and INPUT_PROCESS.poll() is None:
        INPUT_PROCESS.terminate()
        try:
            INPUT_PROCESS.wait(timeout=4)
        except subprocess.TimeoutExpired:
            INPUT_PROCESS.kill()
            INPUT_PROCESS.wait(timeout=3)
    INPUT_PROCESS = None
    if INPUT_LOG_HANDLE is not None:
        INPUT_LOG_HANDLE.close()
        INPUT_LOG_HANDLE = None
    YDTOOL_SOCKET.unlink(missing_ok=True)


def key(*events: str) -> None:
    result = command([YDOTool, "key", *events], timeout=5)
    if result.returncode != 0:
        raise RuntimeError(f"Wayland key injection failed: {result.stderr.strip()}")


def type_text(value: str) -> None:
    result = command([YDOTool, "type", value], timeout=15)
    if result.returncode != 0:
        raise RuntimeError(f"Wayland text injection failed: {result.stderr.strip()}")


def desktop_app(name: str):
    if pyatspi is None:
        raise RuntimeError("Python module pyatspi is required for native automation")
    desktop = pyatspi.Registry.getDesktop(0)
    for index in range(desktop.childCount):
        try:
            app = desktop.getChildAtIndex(index)
            if app.name == name:
                return app
        except Exception:
            continue
    return None


def descendants(root):
    """Walk the live AT-SPI tree while tolerating stale objects during redraw."""
    pending = [root]
    seen = 0
    while pending and seen < 12000:
        node = pending.pop()
        seen += 1
        yield node
        try:
            pending.extend(node.getChildAtIndex(i) for i in reversed(range(node.childCount)))
        except Exception:
            continue


def find_accessible(root, *, role: str | None = None, name: str | None = None):
    wanted = name.casefold() if name is not None else None
    for node in descendants(root):
        try:
            if role is not None and node.getRoleName() != role:
                continue
            if wanted is not None and (node.name or "").casefold() != wanted:
                continue
            return node
        except Exception:
            continue
    return None


def accessible_action(node, action_name: str = "click") -> bool:
    if node is None:
        return False
    actions = node.queryAction()
    preferred = (action_name, "click", "press", "toggle")
    for name in preferred:
        for index in range(actions.nActions):
            if actions.getName(index) == name:
                return bool(actions.doAction(index))
    return False


def accessible_state(node, state) -> bool:
    try:
        return bool(node.getState().contains(state))
    except Exception:
        return False


def wait_for_accessible(predicate, description: str, timeout: float = 15):
    deadline = time.monotonic() + timeout
    last_error = None
    while time.monotonic() < deadline:
        if not app_alive():
            STATE["app_exited_early"] = True
            raise RuntimeError(f"PIC exited while waiting for {description}")
        try:
            result = predicate()
            if result:
                return result
        except Exception as error:
            last_error = error
        time.sleep(0.2)
    suffix = f" ({last_error})" if last_error else ""
    raise RuntimeError(f"Timed out waiting for {description}{suffix}")


def pic_accessible():
    return desktop_app("pic-rs")


def pic_window():
    app = pic_accessible()
    return find_accessible(app, role="frame", name="PIC") if app else None


def database_snapshot() -> tuple[set[str], int]:
    if not DB_PATH.is_file():
        return set(), 0
    with sqlite3.connect(DB_PATH, timeout=2) as connection:
        roots = {row[0] for row in connection.execute(
            "SELECT path FROM folders WHERE imported_root=1")}
        count = int(connection.execute("SELECT COUNT(*) FROM photos").fetchone()[0])
    return roots, count


def folder_photo_count(path: str) -> int:
    with sqlite3.connect(DB_PATH, timeout=2) as connection:
        return int(connection.execute(
            "WITH RECURSIVE tree(id) AS ("
            " SELECT id FROM folders WHERE path=?"
            " UNION ALL SELECT folders.id FROM folders JOIN tree ON folders.parent_id=tree.id"
            ") SELECT COUNT(*) FROM photos JOIN tree ON photos.folder_id=tree.id",
            (path,),
        ).fetchone()[0])


def wait_for_database(timeout: float = 45) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if not app_alive():
            STATE["app_exited_early"] = True
            return False
        if DB_PATH.is_file():
            try:
                database_snapshot()
                return True
            except sqlite3.Error:
                pass
        time.sleep(0.3)
    return False


def wait_for_import(previous_roots: set[str], previous_count: int, timeout: float = 90) -> tuple[str | None, int]:
    deadline = time.monotonic() + timeout
    selected_path = None
    count = 0
    while time.monotonic() < deadline:
        if not app_alive():
            STATE["app_exited_early"] = True
            return None, 0
        try:
            roots, total = database_snapshot()
            candidates = sorted(roots - previous_roots)
            if not candidates and total > previous_count:
                candidates = sorted(roots)
            ranked = [(folder_photo_count(path), path) for path in candidates]
            if ranked:
                count, selected_path = max(ranked)
                if count > 0:
                    return selected_path, count
        except sqlite3.Error:
            pass
        time.sleep(0.5)
    return selected_path, count


def wait_for_import_complete(timeout: float = 180) -> tuple[str | None, int]:
    """Wait for the requested real folder's recursive photo count to settle."""
    deadline = time.monotonic() + timeout
    last_count = -1
    stable_since = None
    while time.monotonic() < deadline:
        if not app_alive():
            STATE["app_exited_early"] = True
            raise RuntimeError("PIC exited while importing the selected photo folder")
        try:
            roots, _ = database_snapshot()
            if str(PHOTO_FOLDER) in roots:
                count = folder_photo_count(str(PHOTO_FOLDER))
                if count > 0 and count == last_count:
                    stable_since = stable_since or time.monotonic()
                    if time.monotonic() - stable_since >= 4:
                        return str(PHOTO_FOLDER), count
                else:
                    last_count = count
                    stable_since = None
        except sqlite3.Error:
            pass
        time.sleep(0.5)
    raise RuntimeError(f"No completed photo import found for {PHOTO_FOLDER}")


def wait_for_process_start(timeout: float = 15) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if not app_alive():
            return False
        if APP_LOG.exists() and APP_LOG.stat().st_size > 0:
            # Startup logs are continuously captured; allow GTK to create its
            # first window before asking the user to confirm the gallery.
            time.sleep(2)
            return app_alive()
        time.sleep(0.2)
    return app_alive()


def activate_pic_window() -> None:
    window = pic_window()
    if window is None:
        raise RuntimeError("PIC's main Wayland window is not exposed through AT-SPI")
    if not accessible_action(window, "default.activate"):
        # GTK may already have focus; the following accessibility check keeps
        # keyboard injection scoped to PIC instead of guessing coordinates.
        if not accessible_state(window, pyatspi.STATE_ACTIVE):
            raise RuntimeError("AT-SPI could not activate PIC's main window")


def import_photo_folder() -> None:
    app = wait_for_accessible(pic_accessible, "PIC application accessibility")
    window = wait_for_accessible(pic_window, "PIC main window")
    STATE["confirmations"]["application_visible"] = accessible_state(
        window, pyatspi.STATE_SHOWING)
    if not STATE["confirmations"]["application_visible"]:
        raise RuntimeError("PIC main window is not showing on the Wayland desktop")

    activate_pic_window()
    welcome = find_accessible(app, role="button", name="Skip for now")
    if welcome is not None:
        never_show = find_accessible(
            app, role="check box", name="Don’t show this automatically again")
        if never_show is not None and not accessible_state(never_show, pyatspi.STATE_CHECKED):
            for _ in range(20):
                if accessible_state(never_show, pyatspi.STATE_FOCUSED):
                    break
                key("15:1", "15:0")  # Tab through welcome actions to the preference.
                time.sleep(0.05)
            if not accessible_state(never_show, pyatspi.STATE_FOCUSED):
                raise RuntimeError("Could not focus Don’t show this automatically again through keyboard navigation")
            key("57:1", "57:0")  # Space checks Don’t show this automatically again.
            wait_for_accessible(
                lambda: accessible_state(never_show, pyatspi.STATE_CHECKED),
                "Don’t show this automatically again to be checked", timeout=5)
        if not accessible_action(welcome):
            raise RuntimeError("Could not dismiss PIC's welcome panel through AT-SPI")

    import_button = wait_for_accessible(
        lambda: find_accessible(pic_accessible(), role="button", name="Import Folder"),
        "PIC Import Folder button")
    activate_pic_window()
    if not accessible_action(import_button):
        raise RuntimeError("Could not activate PIC's Import Folder button")

    # GTK's native folder chooser uses Ctrl+L to focus its location field.
    # Enter opens the typed folder; the next Enter accepts it for import.
    time.sleep(1.0)
    key("29:1", "38:1", "38:0", "29:0")  # Left Ctrl + L
    time.sleep(0.2)
    type_text(str(PHOTO_FOLDER))
    key("28:1", "28:0")  # Enter to navigate to the folder
    time.sleep(0.5)
    key("28:1", "28:0")  # Enter to select the folder
    STATE["source_folder"] = str(PHOTO_FOLDER)
    print(f"NATIVE AUTO: Importing real photos from {PHOTO_FOLDER}", flush=True)
    source, count = wait_for_import_complete()
    STATE["source_folder"] = source
    STATE["imported_photos"] = count


def gallery_photo_cells(app, filenames: set[str]):
    cells = []
    for node in descendants(app):
        try:
            if (node.getRoleName() == "table cell" and node.name in filenames
                    and accessible_state(node, pyatspi.STATE_SHOWING)):
                cells.append(node)
        except Exception:
            continue
    return cells


def run_automated_smoke() -> None:
    print("NATIVE AUTO 1/5: Starting isolated Wayland/Vulkan session", flush=True)
    start_input_daemon()
    if not wait_for_database() or not wait_for_process_start():
        STATE["app_exited_early"] = True
        raise RuntimeError("PIC did not finish starting in the isolated library")

    STATE["baseline_roots"], STATE["baseline_photo_count"] = database_snapshot()
    import_photo_folder()

    print("NATIVE AUTO 2/5: Checking imported photos in the visible gallery", flush=True)
    with sqlite3.connect(DB_PATH, timeout=2) as connection:
        filenames = {Path(row[0]).name for row in connection.execute("SELECT path FROM photos")}
    app = wait_for_accessible(pic_accessible, "PIC gallery accessibility")
    cells = wait_for_accessible(
        lambda: gallery_photo_cells(pic_accessible(), filenames),
        "visible imported photo thumbnails")
    if not cells:
        raise RuntimeError("PIC indexed photos but exposed no visible photo thumbnails")
    STATE["confirmations"]["gallery"] = True

    print("NATIVE AUTO 3/5: Opening a real photo in the lightbox", flush=True)
    activate_pic_window()
    cell = cells[0]
    table = cell.parent
    selection = table.querySelection()
    position = cell.getIndexInParent()
    if not selection.selectChild(position):
        raise RuntimeError(f"Could not select visible gallery photo {cell.name}")
    try:
        cell.queryComponent().grabFocus()
    except Exception:
        pass
    key("57:1", "57:0")  # Space opens the selected photo in Fit mode.
    opened = False
    try:
        wait_for_accessible(
            lambda: "PIC_LIGHTBOX_LAYOUT stage=opened" in APP_LOG.read_text(errors="replace"),
            "lightbox open transition", timeout=4)
        opened = True
    except RuntimeError:
        # The toolbar 1:1 action follows the same app shortcut contract: its
        # first activation opens the selected image in Fit mode.
        one_to_one = find_accessible(pic_accessible(), role="toggle button", name="1:1")
        if not accessible_action(one_to_one):
            raise RuntimeError("Could not activate the 1:1 control to open the selected photo")
        wait_for_accessible(
            lambda: "PIC_LIGHTBOX_LAYOUT stage=opened" in APP_LOG.read_text(errors="replace"),
            "lightbox open transition", timeout=8)
    STATE["confirmations"]["lightbox"] = True

    print("NATIVE AUTO 4/5: Activating 1:1 and closing the lightbox", flush=True)
    one_to_one = wait_for_accessible(
        lambda: find_accessible(pic_accessible(), role="toggle button", name="1:1"),
        "lightbox 1:1 control")
    if not accessible_action(one_to_one):
        raise RuntimeError("Could not activate the lightbox 1:1 control")
    wait_for_accessible(
        lambda: native_lightbox_evidence(APP_LOG.read_text(errors="replace"))[1],
        "1:1 zoom update", timeout=12)
    STATE["confirmations"]["zoom_1_to_1"] = True

    one_to_one = find_accessible(pic_accessible(), role="toggle button", name="1:1")
    try:
        one_to_one.queryComponent().grabFocus()
    except Exception:
        pass
    activate_pic_window()
    key("1:1", "1:0")  # Escape closes the lightbox.
    try:
        wait_for_accessible(
            lambda: "PIC_LIGHTBOX_LAYOUT stage=hidden" in APP_LOG.read_text(errors="replace"),
            "lightbox close transition", timeout=3)
    except RuntimeError:
        key("14:1", "14:0")  # Backspace is the app's alternate close shortcut.
        wait_for_accessible(
            lambda: "PIC_LIGHTBOX_LAYOUT stage=hidden" in APP_LOG.read_text(errors="replace"),
            "lightbox close transition", timeout=8)
    STATE["confirmations"]["lightbox_closed"] = True

    print("NATIVE AUTO 5/5: Restarting PIC and verifying import persistence", flush=True)
    if not app_alive():
        STATE["app_exited_early"] = True
        raise RuntimeError("PIC exited before the persistence check")
    stop_app()
    start_app()
    if not wait_for_database(timeout=30) or not wait_for_process_start(timeout=15):
        STATE["app_exited_early"] = True
        raise RuntimeError("PIC did not reopen the isolated library")
    STATE["persisted_photos"] = folder_photo_count(STATE["source_folder"])


def current_renderer_evidence() -> tuple[bool, str]:
    if LOG_HANDLE is not None:
        LOG_HANDLE.flush()
    return vulkan_renderer_evidence(APP_LOG.read_text(errors="replace") if APP_LOG.exists() else "")


def write_report(*, status: str, renderer_ok: bool, renderer_detail: str) -> None:
    confirmations = STATE["confirmations"]
    rows = [
        "# PIC automated native Fedora GUI smoke report",
        "",
        f"- Overall: **{status}**",
        f"- Commit: `{COMMIT}`",
        "- Session: native Fedora desktop; no Xvfb/Openbox/Cairo or software renderer overrides",
        f"- Wayland backend requested: `{os.environ.get('GDK_BACKEND', '')}`",
        f"- Wayland display: `{os.environ.get('WAYLAND_DISPLAY', '')}`",
        f"- Vulkan backend requested: `{os.environ.get('GSK_RENDERER', '')}`",
        f"- GTK Vulkan renderer confirmed: `{renderer_ok}` ({renderer_detail})",
        f"- Selected source folder: `{STATE['source_folder'] or 'not selected'}`",
        f"- Photos indexed in isolated database: {STATE['imported_photos']}",
        f"- Photos still indexed after restart: {STATE['persisted_photos']}",
        f"- User canceled: {STATE['canceled']}",
        f"- PIC exited before completion: {STATE['app_exited_early']}",
        f"- Artifact directory: `reports/gui-tests/{RUN_DIR.name}`",
        "- PIC config/database/cache: temporary paths under this run directory",
        "- Original photo files and the normal PIC library were not intentionally modified.",
        "",
        "## Automated native GUI checks",
        "",
        "| Check | Confirmation | Evidence |",
        "|---|---:|---|",
    ]
    for key, title, image in (
        ("application_visible", "PIC window visible", "00-native-startup.png"),
        ("gallery", "Imported gallery visible", "01-native-gallery.png"),
        ("lightbox", "Lightbox photo visible", "02-native-lightbox.png"),
        ("zoom_1_to_1", "Photo at 1:1 actual size", "03-native-zoom-1-to-1.png"),
        ("lightbox_closed", "Lightbox closed back to gallery", "04-native-lightbox-closed.png"),
    ):
        confirmation = confirmations[key]
        evidence = "AT-SPI + GTK runtime log" if confirmation is True else "not confirmed"
        rows.append(f"| {title} | {confirmation if confirmation is not None else 'not confirmed'} | {evidence} |")
    rows.extend(["", "## Runtime diagnostics", "", "- `system-diagnostics.txt`", "- `application.log`", "- `harness.log`", "- `screenshots/`"])
    if STATE["screenshot_errors"]:
        rows.extend(["", "## Screenshot issues", ""])
        rows.extend(f"- {error}" for error in STATE["screenshot_errors"])
    if STATE["driver_error"]:
        rows.extend(["", "## Driver error", "", "```text", STATE["driver_error"], "```"])
    (RUN_DIR / "report.md").write_text("\n".join(rows) + "\n")


def main() -> int:
    SCREENSHOTS.mkdir(parents=True, exist_ok=True)
    collect_system_diagnostics()
    status = "BLOCKED"
    renderer_ok = False
    renderer_detail = "Vulkan renderer initialization not confirmed"
    try:
        start_app()
        if not wait_for_database() or not wait_for_process_start():
            STATE["app_exited_early"] = True
        else:
            try:
                roots, count = database_snapshot()
                STATE["baseline_roots"] = roots
                STATE["baseline_photo_count"] = count
            except sqlite3.Error:
                pass
            run_automated_smoke()
    except KeyboardInterrupt:
        STATE["canceled"] = True
    except Exception:
        STATE["driver_error"] = traceback.format_exc()
    finally:
        renderer_ok, renderer_detail = current_renderer_evidence()
        stop_app()
        stop_input_daemon()

    status = classify_native_result(
        wayland_ready=os.environ.get("PIC_GUI_WAYLAND_READY") == "1",
        vulkan_ready=renderer_ok,
        canceled=bool(STATE["canceled"]),
        app_exited_early=bool(STATE["app_exited_early"]),
        confirmations=STATE["confirmations"],
        imported_photos=int(STATE["imported_photos"]),
        persisted_photos=int(STATE["persisted_photos"]),
    )
    if STATE["driver_error"] and not STATE["canceled"]:
        status = "BLOCKED" if pyatspi is None or not YDTOOL_SOCKET.exists() else "FAIL"
    write_report(status=status, renderer_ok=renderer_ok, renderer_detail=renderer_detail)
    print(f"Native GUI smoke result: {status}", flush=True)
    print(f"Report: {RUN_DIR / 'report.md'}", flush=True)
    return {"PASS": 0, "FAIL": 1, "BLOCKED": 2}[status]


if __name__ == "__main__":
    sys.exit(main())
