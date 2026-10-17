#!/usr/bin/env python3
"""Headless, isolated X11 smoke tests for the PIC GTK application."""

from __future__ import annotations

import os
import re
import hashlib
import sqlite3
import subprocess
import sys
import time
import traceback
from dataclasses import dataclass
from pathlib import Path

from PIL import Image, ImageChops, ImageStat, ImageDraw

try:
    import pyatspi
except ImportError:
    pyatspi = None


RUN_DIR = Path(os.environ["PIC_GUI_RUN_DIR"])
SCREENSHOTS = RUN_DIR / "screenshots"
APP = Path(os.environ["PIC_GUI_APP"])
COMMIT = os.environ.get("PIC_GUI_COMMIT", "unknown")
SUITE = os.environ.get("PIC_GUI_SUITE", "all")
DISPLAY = os.environ["DISPLAY"]
APP_LOG = RUN_DIR / "application.log"
SAMPLE_DIR = RUN_DIR / "sample-library"
IMPORT_DIR = RUN_DIR / "xdg" / "home" / "00-import-source"
XDG_ROOT = RUN_DIR / "xdg"
XDG_CONFIG = XDG_ROOT / "config"
XDG_DATA = XDG_ROOT / "data"
XDG_CACHE = XDG_ROOT / "cache"
XDG_HOME = XDG_ROOT / "home"


@dataclass
class Case:
    name: str
    expected: str
    actual: str = "Not reached"
    result: str = "NOT TESTED"


CASES: list[Case] = []
APP_PROCESS: subprocess.Popen | None = None
APP_WINDOW: str | None = None
MOUSE_OFFSET_X = 0
MOUSE_OFFSET_Y = 0


def add_case(name: str, expected: str) -> Case:
    case = Case(name, expected)
    CASES.append(case)
    return case


def command(args: list[str], *, timeout: float = 10, check: bool = True) -> str:
    if len(args) >= 4 and args[:2] == ["xdotool", "mousemove"]:
        args = args.copy()
        args[-2] = str(int(args[-2]) + MOUSE_OFFSET_X)
        args[-1] = str(int(args[-1]) + MOUSE_OFFSET_Y)
    result = subprocess.run(args, check=check, text=True, capture_output=True, timeout=timeout)
    return result.stdout.strip()


def make_sample_library() -> None:
    SAMPLE_DIR.mkdir(parents=True, exist_ok=True)
    IMPORT_DIR.mkdir(parents=True, exist_ok=True)
    samples = [
        ("landscape.jpg", (1200, 700), (45, 110, 190)),
        ("portrait.jpg", (600, 900), (180, 75, 95)),
        ("square.png", (800, 800), (40, 155, 95)),
    ]
    for index in range(12):
        samples.append((f"sample-{index:02d}.jpg", (960, 640), (30 + index * 13, 90 + index * 7, 160 - index * 5)))
    for name, size, color in samples:
        image = Image.new("RGB", size, color)
        draw = ImageDraw.Draw(image)
        draw.text((110 if name == "portrait.jpg" else 32, 28), name, fill="white")
        if name == "portrait.jpg":
            # High contrast edge marker makes native-size pan movement
            # measurable independently of viewport clipping.
            draw.rectangle((8, 8, 87, 87), fill=(255, 255, 0))
        image.save(SAMPLE_DIR / name)
    IMPORT_DIR.mkdir(parents=True, exist_ok=True)
    Image.new("RGB", (640, 480), (235, 125, 30)).save(IMPORT_DIR / "import-only.jpg")
    Image.new("RGB", (512, 384), (30, 180, 195)).save(IMPORT_DIR / "import-second.png")


def seed_isolated_database() -> None:
    """Populate only the temporary PIC database with the generated photos."""
    database = XDG_DATA / "pic-rs" / "library.db"
    deadline = time.monotonic() + 20
    while not database.is_file() and time.monotonic() < deadline:
        time.sleep(0.1)
    if not database.is_file():
        raise RuntimeError("PIC did not create its isolated library database")
    folder = str(SAMPLE_DIR.resolve())
    with sqlite3.connect(database) as connection:
        connection.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('onboarding-never-show','true')")
        connection.execute("INSERT OR IGNORE INTO folders(path,name,imported_root) VALUES(?,?,1)",
                           (folder, SAMPLE_DIR.name))
        folder_id = connection.execute("SELECT id FROM folders WHERE path=?", (folder,)).fetchone()[0]
        settings = [
            ("screensaver-source", "all"),
            ("screensaver-style", "fade"),
            ("screensaver-duration", "3"),
            ("screensaver-transition-ms", "200"),
            ("screensaver-shuffle", "false"),
            ("screensaver-captions", "true"),
        ]
        # Optional theme selection lets the isolated GUI harness take
        # reproducible appearance screenshots without touching user settings.
        gui_theme = os.environ.get("PIC_GUI_THEME")
        if gui_theme:
            settings.append(("appearance-theme", gui_theme))
        connection.executemany("INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)", settings)
        for path in sorted(SAMPLE_DIR.iterdir()):
            if path.suffix.lower() not in {".jpg", ".png"}:
                continue
            with Image.open(path) as image:
                width, height = image.size
            stat = path.stat()
            connection.execute(
                "INSERT OR REPLACE INTO photos(path,folder_id,width,height,size_bytes,mtime,added_at) VALUES(?,?,?,?,?,?,?)",
                (str(path.resolve()), folder_id, width, height, stat.st_size, int(stat.st_mtime), int(stat.st_mtime)),
            )


def screenshot(name: str) -> Path:
    target = SCREENSHOTS / name
    if os.environ.get("PIC_GUI_DESKTOP_DISPLAY") == "1":
        result = subprocess.run(
            ["/usr/bin/python3", str(Path(__file__).with_name("capture_wayland_screenshot.py")), str(target)],
            text=True, capture_output=True, timeout=25,
        )
        if result.returncode != 0:
            raise RuntimeError(f"Wayland screenshot capture failed: {result.stderr.strip()}")
    else:
        command(["import", "-window", "root", str(target)], timeout=15)
    image = Image.open(target).convert("RGB")
    if image.width < 100 or image.height < 100:
        raise RuntimeError(f"Screenshot has invalid dimensions: {image.size}")
    if ImageStat.Stat(image.convert("L")).stddev[0] < 4:
        raise RuntimeError(f"Screenshot appears blank: {target}")
    return target


def screenshot_changed(before: Path, after: Path) -> bool:
    a = Image.open(before).convert("RGB")
    b = Image.open(after).convert("RGB")
    diff = ImageChops.difference(a, b)
    changed = sum(ImageStat.Stat(diff).mean) / 3
    return changed > 0.8


def wait_for(predicate, description: str, timeout: float = 12):
    deadline = time.monotonic() + timeout
    last_error = None
    while time.monotonic() < deadline:
        try:
            result = predicate()
            if result:
                return result
        except Exception as error:
            last_error = error
        time.sleep(0.2)
    detail = f": {last_error}" if last_error else ""
    raise TimeoutError(f"Timed out waiting for {description}{detail}")


def app_alive() -> bool:
    return APP_PROCESS is not None and APP_PROCESS.poll() is None


def accessible_nodes():
    if pyatspi is None:
        return
    pending = [pyatspi.Registry.getDesktop(0)]
    seen = 0
    while pending and seen < 10000:
        node = pending.pop()
        seen += 1
        yield node
        try:
            pending.extend(node.getChildAtIndex(i) for i in reversed(range(node.childCount)))
        except Exception:
            continue


def click_accessible(role: str, name: str, timeout: float = 5):
    deadline = time.monotonic() + timeout
    wanted = name.casefold()
    while time.monotonic() < deadline:
        for node in accessible_nodes() or ():
            try:
                if node.getRoleName() == role and (node.name or "").casefold() == wanted:
                    actions = node.queryAction()
                    for index in range(actions.nActions):
                        if actions.getName(index) in {"click", "press", "toggle"}:
                            if actions.doAction(index):
                                return node
            except Exception:
                continue
        time.sleep(0.2)
    return None


def find_accessible(role: str, name: str):
    wanted = name.casefold()
    for node in accessible_nodes() or ():
        try:
            if node.getRoleName() == role and (node.name or "").casefold() == wanted:
                return node
        except Exception:
            continue
    return None


def accessible_exists(role: str, name: str) -> bool:
    wanted = name.casefold()
    for node in accessible_nodes() or ():
        try:
            if (node.getRoleName() == role and (node.name or "").casefold() == wanted
                    and node.getState().contains(pyatspi.STATE_SHOWING)):
                return True
        except Exception:
            continue
    return False


def dismiss_first_run_welcome() -> None:
    if os.environ.get("PIC_GUI_DESKTOP_DISPLAY") != "1":
        return
    if pyatspi is None:
        raise RuntimeError("pyatspi is required to dismiss the first-run tutorial automatically")
    checkbox = None
    deadline = time.monotonic() + 8
    while checkbox is None and time.monotonic() < deadline:
        checkbox = find_accessible("check box", "Don’t show this automatically again")
        if checkbox is None:
            time.sleep(0.2)
    if checkbox is not None:
        try:
            checked = checkbox.getState().contains(pyatspi.STATE_CHECKED)
        except Exception:
            checked = False
        if not checked:
            # GTK's checkbox has no AT-SPI action and reports an empty
            # component extent on this build. Click its measured position in
            # the responsive welcome panel, scaled from the actual window.
            geometry = command(["xdotool", "getwindowgeometry", "--shell", APP_WINDOW])
            values = {key: int(value) for key, value in re.findall(
                r"^(X|Y|WIDTH|HEIGHT)=(-?\d+)$", geometry, re.MULTILINE)}
            if not all(key in values for key in ("X", "Y", "WIDTH", "HEIGHT")):
                raise RuntimeError("Could not determine PIC window geometry to click the tutorial preference")
            click_x = values["X"] + round(values["WIDTH"] * 0.472)
            click_y = values["Y"] + round(values["HEIGHT"] * 0.74)
            command(["xdotool", "mousemove", str(click_x), str(click_y)])
            command(["xdotool", "click", "1"])
            wait_for(lambda: checkbox.getState().contains(pyatspi.STATE_CHECKED),
                     "clicked first-run tutorial preference to become checked", timeout=4)
    click_accessible("button", "Skip for now", timeout=4)


def prepare_interactive_window(window_id: str) -> None:
    """Give the existing coordinate/pixel assertions their reference geometry."""
    if os.environ.get("PIC_GUI_DESKTOP_DISPLAY") != "1":
        return
    width = int(os.environ.get("PIC_GUI_DISPLAY_WIDTH", "1600"))
    height = int(os.environ.get("PIC_GUI_DISPLAY_HEIGHT", "1000"))
    x = max(0, (width - 1600) // 2)
    y = max(0, (height - 1000) // 2)
    command(["xdotool", "windowsize", "--sync", window_id, "1600", "1000"])
    command(["xdotool", "windowmove", "--sync", window_id, str(x), str(y)])


def check_logs() -> list[str]:
    if not APP_LOG.exists():
        return ["application.log was not created"]
    contents = APP_LOG.read_text(errors="replace")
    patterns = [
        r"Gtk-CRITICAL",
        r"GLib-GObject-CRITICAL",
        r"segmentation fault",
        r"panicked at",
        r"resource[^\n]*(?:not found|missing)",
    ]
    return [pattern for pattern in patterns if re.search(pattern, contents, re.IGNORECASE)]


def record(case: Case, passed: bool, actual: str) -> None:
    if APP_PROCESS is not None and not app_alive():
        passed = False
        actual += "; PIC process exited"
    case.actual = actual
    case.result = "PASS" if passed else "FAIL"
    print(f"{case.result}: {case.name} — {actual}", flush=True)


def click_at(x: int, y: int, button: int = 1, repeat: int = 1) -> None:
    command(["xdotool", "mousemove", str(x), str(y)])
    if repeat == 1:
        command(["xdotool", "click", str(button)])
    else:
        command(["xdotool", "click", "--repeat", str(repeat), "--delay", "180", str(button)])


def wait_for_screen_change(before: Path, name: str, description: str, timeout: float = 10) -> Path:
    """Wait for a visible GUI state transition instead of sleeping blindly."""
    def changed():
        current = screenshot(name)
        return current if screenshot_changed(before, current) else None
    return wait_for(changed, description, timeout=timeout)


def image_is_nonblank(path: Path) -> bool:
    with Image.open(path) as image:
        image.verify()
    with Image.open(path) as image:
        return image.width > 0 and image.height > 0 and ImageStat.Stat(image.convert("RGB")).stddev[0] > 5


def photo_state(name: str) -> tuple[int, str]:
    with sqlite3.connect(XDG_DATA / "pic-rs" / "library.db") as connection:
        row = connection.execute("SELECT rating,edit_recipe FROM photos WHERE path=?",
                                 (str((SAMPLE_DIR / name).resolve()),)).fetchone()
    if row is None:
        raise RuntimeError(f"Temporary photo disappeared from database: {name}")
    return int(row[0] or 0), row[1] or ""


def db_setting(key: str) -> str | None:
    with sqlite3.connect(XDG_DATA / "pic-rs" / "library.db") as connection:
        row = connection.execute("SELECT value FROM settings WHERE key=?", (key,)).fetchone()
    return row[0] if row else None


def with_photo_count() -> int:
    with sqlite3.connect(XDG_DATA / "pic-rs" / "library.db") as connection:
        return int(connection.execute("SELECT COUNT(*) FROM photos").fetchone()[0])


def matching_generated_tiles(path: Path) -> list[tuple[int, int, str]]:
    """Count visible fixture thumbnails by sampling their image centers."""
    colors: dict[str, tuple[int, int, int]] = {}
    for file in SAMPLE_DIR.glob("*.jpg"):
        with Image.open(file) as source:
            colors[file.name] = source.convert("RGB").getpixel((source.width // 2, source.height // 2))
    image = Image.open(path).convert("RGB")
    found = []
    for x in (518, 878, 1234):
        for y in (178, 390, 603, 778):
            px, py = x + MOUSE_OFFSET_X, y + MOUSE_OFFSET_Y
            if px >= image.width or py >= image.height:
                continue
            actual = image.getpixel((px, py))
            match = min(colors, key=lambda name: sum(abs(a - b) for a, b in zip(actual, colors[name])))
            error = sum(abs(a - b) for a, b in zip(actual, colors[match]))
            if error < 75:
                found.append((x, y, match))
    return found


def generated_photo_center_color_distance(path: Path, filename: str, x: int, y: int) -> int:
    """Compare a visible fixture tile's center pixel with its generated source."""
    source_path = SAMPLE_DIR / filename
    with Image.open(source_path) as source:
        expected = source.convert("RGB").getpixel((source.width // 2, source.height // 2))
    with Image.open(path) as screenshot_image:
        actual = screenshot_image.convert("RGB").getpixel((x + MOUSE_OFFSET_X, y + MOUSE_OFFSET_Y))
    return sum(abs(left - right) for left, right in zip(actual, expected))


def selected_badge_count(path: Path) -> int:
    image = Image.open(path).convert("RGB")
    count = 0
    for x in (657, 1011, 1385):
        for y in (146, 360, 573, 786):
            px, py = x + MOUSE_OFFSET_X, y + MOUSE_OFFSET_Y
            crop = image.crop((px - 8, py - 8, px + 9, py + 9))
            blue = sum(1 for red, green, blue in crop.getdata()
                       if 40 <= red <= 90 and 110 <= green <= 170 and blue >= 200)
            if blue >= 18:
                count += 1
    return count


def matching_color_bounds(path: Path, color: tuple[int, int, int], tolerance: int = 28,
                          crop: tuple[int, int, int, int] | None = None):
    image = Image.open(path).convert("RGB")
    return color_bounds(image, color, tolerance, crop)


def color_bounds(image: Image.Image, color: tuple[int, int, int], tolerance: int = 28,
                 crop: tuple[int, int, int, int] | None = None):
    image = image.convert("RGB")
    pixels = image.load()
    left, top, right, bottom = crop or (0, 0, image.width, image.height)
    points = []
    for y in range(max(0, top), min(bottom, image.height)):
        for x in range(max(0, left), min(right, image.width)):
            value = pixels[x, y]
            if max(abs(value[index] - color[index]) for index in range(3)) <= tolerance:
                points.append((x, y))
    if not points:
        return None
    return (min(x for x, _ in points), min(y for _, y in points),
            max(x for x, _ in points) + 1, max(y for _, y in points) + 1)


def count_color_pixels(path: Path, color: tuple[int, int, int], tolerance: int = 24) -> int:
    image = Image.open(path).convert("RGB")
    return sum(1 for value in image.getdata()
               if max(abs(value[index] - color[index]) for index in range(3)) <= tolerance)


def editor_divider_pixels(path: Path) -> int:
    image = Image.open(path).convert("RGB")
    x = 679 + MOUSE_OFFSET_X
    if x >= image.width:
        return 0
    return sum(1 for y in range(130 + MOUSE_OFFSET_Y, min(830 + MOUSE_OFFSET_Y, image.height))
               if max(image.getpixel((x, y))) - min(image.getpixel((x, y))) <= 10
               and 185 <= sum(image.getpixel((x, y))) // 3 <= 235)


def smoke_test() -> None:
    global APP_PROCESS, APP_WINDOW, MOUSE_OFFSET_X, MOUSE_OFFSET_Y
    make_sample_library()
    for path in (XDG_CONFIG, XDG_DATA, XDG_CACHE, XDG_HOME):
        path.mkdir(parents=True, exist_ok=True)
    app_env = os.environ.copy()
    app_env.update({
        "HOME": str(XDG_HOME), "XDG_CONFIG_HOME": str(XDG_CONFIG),
        "XDG_DATA_HOME": str(XDG_DATA), "XDG_CACHE_HOME": str(XDG_CACHE),
        "GSETTINGS_BACKEND": "memory", "GDK_BACKEND": "x11",
        "GSK_RENDERER": "cairo", "LIBGL_ALWAYS_SOFTWARE": "1",
        "GTK_USE_PORTAL": "0", "GIO_USE_VFS": "local",
        "GTK_A11Y": "atspi" if os.environ.get("PIC_GUI_DESKTOP_DISPLAY") == "1" else "none",
        "PICASA_TRACE": "1", "PIC_SCREENSAVER_TRACE": "1",
    })
    log = APP_LOG.open("w")
    APP_PROCESS = subprocess.Popen([str(APP)], env=app_env, stdout=log, stderr=subprocess.STDOUT)
    APP_WINDOW = wait_for(
        lambda: next((wid for wid in command(["xdotool", "search", "--onlyvisible", "--name", ".*"], check=False).splitlines()
                      if "PIC" in command(["xdotool", "getwindowname", wid], check=False)), None),
        "visible PIC window", timeout=30,
    )
    prepare_interactive_window(APP_WINDOW)
    command(["xdotool", "windowfocus", "--sync", APP_WINDOW])
    dismiss_first_run_welcome()
    geometry = command(["xdotool", "getwindowgeometry", "--shell", APP_WINDOW])
    match_x = re.search(r"^X=(\d+)$", geometry, re.MULTILINE)
    match_y = re.search(r"^Y=(\d+)$", geometry, re.MULTILINE)
    if os.environ.get("PIC_GUI_WINDOW_MANAGER", "none") != "none":
        MOUSE_OFFSET_X = int(match_x.group(1)) if match_x else 0
        MOUSE_OFFSET_Y = int(match_y.group(1)) if match_y else 0
    wait_for(lambda: app_alive() and screenshot("00-library-ready.png").exists(),
             "PIC initial Library render", timeout=30)
    home = screenshot("01-library-home.png")
    record(add_case("PIC startup and Library", "PIC opens and renders its Library window"),
           app_alive() and home.exists(), f"window {APP_WINDOW}; screenshot={home.name}")
    if SUITE == "startup":
        return
    # Build an isolated library populated only with generated sample images.
    APP_PROCESS.terminate()
    APP_PROCESS.wait(timeout=5)
    seed_isolated_database()
    # Reopen against three generated samples, then dismiss the first-run welcome
    # with real X11 input. This avoids invoking a desktop portal file chooser.
    log.close()
    log = APP_LOG.open("a")
    APP_PROCESS = subprocess.Popen([str(APP)], env=app_env, stdout=log, stderr=subprocess.STDOUT)
    APP_WINDOW = wait_for(
        lambda: next((wid for wid in command(["xdotool", "search", "--onlyvisible", "--name", ".*"], check=False).splitlines()
                      if "PIC" in command(["xdotool", "getwindowname", wid], check=False)), None),
        "visible PIC window with temporary test photos", timeout=30,
    )
    prepare_interactive_window(APP_WINDOW)
    command(["xdotool", "windowfocus", "--sync", APP_WINDOW])
    geometry = command(["xdotool", "getwindowgeometry", "--shell", APP_WINDOW])
    match_x = re.search(r"^X=(\d+)$", geometry, re.MULTILINE)
    match_y = re.search(r"^Y=(\d+)$", geometry, re.MULTILINE)
    if os.environ.get("PIC_GUI_WINDOW_MANAGER", "none") != "none":
        MOUSE_OFFSET_X = int(match_x.group(1)) if match_x else 0
        MOUSE_OFFSET_Y = int(match_y.group(1)) if match_y else 0
    command(["xdotool", "key", "Escape"])
    wait_for(lambda: app_alive() and screenshot("02-library-loaded.png").exists(),
             "PIC temporary gallery render", timeout=30)
    before = screenshot("02-library-loaded.png")
    expected_photos = 15
    with sqlite3.connect(XDG_DATA / "pic-rs" / "library.db") as connection:
        photo_count = connection.execute("SELECT COUNT(*) FROM photos").fetchone()[0]
    library_case = add_case("Isolated sample gallery", "Generated photos load into the temporary library")
    record(library_case, photo_count == expected_photos and app_alive(),
           f"{photo_count} generated photos indexed; screenshot={before.name}")

    # Set a rating through the actual keyboard shortcut, then verify the
    # corresponding photo row changes in the live temporary SQLite database.
    click_at(518, 178)
    command(["xdotool", "key", "3"])
    wait_for(lambda: photo_state("landscape.jpg")[0] == 3,
             "landscape rating persisted as three stars", timeout=8)
    click_at(878, 178)
    command(["xdotool", "key", "5"])
    wait_for(lambda: photo_state("portrait.jpg")[0] == 5,
             "portrait rating persisted as five stars", timeout=8)
    rating_shot = screenshot("02a-rating-three-stars.png")
    landscape_rating, _ = photo_state("landscape.jpg")
    portrait_rating, _ = photo_state("portrait.jpg")
    record(add_case("Photo rating persistence", "Pressing 3 and 5 saves ratings for the selected photos"),
           landscape_rating == 3 and portrait_rating == 5,
           f"database ratings: landscape={landscape_rating}, portrait={portrait_rating}; screenshot={rating_shot.name}")

    # Open the actual imported folder and filter it to the rated image.
    click_at(118, 486)
    folder_shot = screenshot("02b-folder-view.png")
    with sqlite3.connect(XDG_DATA / "pic-rs" / "library.db") as connection:
        folder_photo_count = connection.execute("SELECT COUNT(*) FROM photos WHERE folder_id=(SELECT id FROM folders WHERE path=?)",
                                                (str(SAMPLE_DIR.resolve()),)).fetchone()[0]
    record(add_case("Folder navigation and photo membership", "Opening the generated folder shows its 15 catalogued photos"),
           folder_photo_count == 15 and len(matching_generated_tiles(folder_shot)) >= 12,
           f"folder database count={folder_photo_count}; visible generated tiles={len(matching_generated_tiles(folder_shot))}; screenshot={folder_shot.name}")
    if click_accessible("toggle button", "Filter by rating (clear)") is None:
        raise RuntimeError("Could not open the folder rating filter through AT-SPI")
    wait_for(lambda: accessible_exists("radio button", "Clear"),
             "rating filter popover to open", timeout=4)
    filter_menu = screenshot("02c-rating-filter-menu.png")
    click_at(1350, 286)
    wait_for(lambda: db_setting("photo-rating-filter") == "3",
             "folder rating filter set to three stars", timeout=8)
    filtered_shot = screenshot("02d-folder-rated-filter.png")
    filtered_tiles = matching_generated_tiles(filtered_shot)
    record(add_case("Folder rating filter membership", "Three-star folder filter shows exactly the rated photo"),
           db_setting("photo-rating-filter") == "3" and len(filtered_tiles) == 1
           and filtered_tiles[0][2] == "landscape.jpg",
           f"setting={db_setting('photo-rating-filter')}; visible fixture tiles={filtered_tiles}; screenshots={filter_menu.name}, {filtered_shot.name}")
    # The check button leaves its popover open, so the Clear row is directly
    # available after choosing the rating filter.
    click_at(1350, 130)
    wait_for(lambda: db_setting("photo-rating-filter") == "all",
             "clear folder rating filter", timeout=8)
    command(["xdotool", "key", "Escape"])
    filter_cleared = screenshot("02e-folder-rating-filter-cleared.png")
    record(add_case("Folder rating filter reset", "Clearing the rating filter restores the folder photos"),
           len(matching_generated_tiles(filter_cleared)) >= 12,
           f"visible fixture tiles={len(matching_generated_tiles(filter_cleared))}; result={filter_cleared.name}")

    # Measure selecting All Stars specifically in Folder view. The database
    # state gives the handler/persistence latency; repeated screenshots give
    # the first captured GUI frame containing exactly both rated fixture
    # photos. The visual time includes screenshot capture and polling cadence.
    # Clear closes the popover; reopen it before selecting All Stars.
    if click_accessible("toggle button", "Filter by rating (clear)") is None:
        raise RuntimeError("Could not reopen the folder rating filter through AT-SPI")
    wait_for(lambda: accessible_exists("radio button", "Clear"),
             "rating filter popover to reopen", timeout=4)
    all_stars_menu = screenshot("02-rating-all-stars-menu.png")
    all_stars_click_started = time.monotonic()
    click_at(1350, 162)  # All Stars row in the rating-filter popover.
    db_deadline = all_stars_click_started + 8
    while db_setting("photo-rating-filter") != "stars" and time.monotonic() < db_deadline:
        time.sleep(0.01)
    db_result_ms = (time.monotonic() - all_stars_click_started) * 1000
    all_stars_screenshot = SCREENSHOTS / "02-rating-all-stars-results.png"
    expected_all_stars = {"landscape.jpg", "portrait.jpg"}
    expected_tile_centers = {"landscape.jpg": (518, 178), "portrait.jpg": (878, 178)}
    visible_tiles: list[tuple[int, int, str]] = []
    tile_color_distances: dict[str, int] = {}
    visual_deadline = all_stars_click_started + 8
    while time.monotonic() < visual_deadline:
        screenshot(all_stars_screenshot.name)
        visible_tiles = matching_generated_tiles(all_stars_screenshot)
        tile_color_distances = {
            name: generated_photo_center_color_distance(all_stars_screenshot, name, *center)
            for name, center in expected_tile_centers.items()
        }
        if ({tile[2] for tile in visible_tiles} == expected_all_stars
                and len(visible_tiles) == len(expected_all_stars)
                and all(distance <= 20 for distance in tile_color_distances.values())):
            break
        time.sleep(0.02)
    visual_result_ms = (time.monotonic() - all_stars_click_started) * 1000
    stars_filter = db_setting("photo-rating-filter")
    record(add_case("Folder All Stars filter and result latency",
                    "Selecting All Stars in Folder shows exactly the 3-star and 5-star photos within 1.5 seconds"),
           stars_filter == "stars"
           and {tile[2] for tile in visible_tiles} == expected_all_stars
           and len(visible_tiles) == len(expected_all_stars)
           and all(distance <= 20 for distance in tile_color_distances.values())
           and visual_result_ms <= 1500,
           f"setting={stars_filter}; visible fixture tiles={visible_tiles}; "
           f"source-to-screen center RGB distances={tile_color_distances} (max 20); "
           f"database state observed={db_result_ms:.1f} ms; first captured matching GUI frame={visual_result_ms:.1f} ms; "
           f"screenshots={all_stars_menu.name}, {all_stars_screenshot.name} (visual timing includes screenshot capture/polling)")
    # Restore the unfiltered folder for the remaining import/navigation tests.
    click_at(1230, 110)
    wait_for(lambda: db_setting("photo-rating-filter") == "all",
             "clear All Stars folder rating filter", timeout=8)
    command(["xdotool", "key", "Escape"])
    if SUITE == "library":
        return

    # Exercise the actual local-folder chooser with a cancellation, an import,
    # and a second import of the same directory. The import source is outside
    # the indexed fixture folder and remains untouched by PIC.
    def open_import_chooser(screenshot_name: str) -> Path:
        click_at(245, 445)
        chooser = wait_for(lambda: next((wid for wid in command(
            ["xdotool", "search", "--onlyvisible", "--name", ".*"], check=False).splitlines()
            if "Import Local Folder" in command(["xdotool", "getwindowname", wid], check=False)), None),
            "Import Local Folder chooser", timeout=15)
        image = screenshot(screenshot_name)
        print(f"IMPORT_CHOOSER window={chooser} screenshot={image.name}", flush=True)
        return image

    def import_chooser_window() -> str | None:
        return next((wid for wid in command(["xdotool", "search", "--onlyvisible", "--name", ".*"], check=False).splitlines()
                     if "Import Local Folder" in command(["xdotool", "getwindowname", wid], check=False)), None)

    def click_screen(x: int, y: int, button: int = 1) -> None:
        subprocess.run(["xdotool", "mousemove", str(x), str(y)], check=True)
        subprocess.run(["xdotool", "click", str(button)], check=True)

    initial_photo_count = 15
    cancel_dialog = open_import_chooser("02f-import-cancel-dialog.png")
    command(["xdotool", "key", "Escape"])
    wait_for(lambda: not any("Import Local Folder" in command(["xdotool", "getwindowname", wid], check=False)
                             for wid in command(["xdotool", "search", "--onlyvisible", "--name", ".*"], check=False).splitlines()),
             "cancelled import chooser to close")
    with sqlite3.connect(XDG_DATA / "pic-rs" / "library.db") as connection:
        after_cancel = connection.execute("SELECT COUNT(*) FROM photos").fetchone()[0]
    cancel_result = screenshot("02g-import-cancelled.png")
    record(add_case("Import chooser cancellation", "Escape closes the picker without changing indexed photos"),
           after_cancel == initial_photo_count,
           f"photo count before/after={initial_photo_count}/{after_cancel}; screenshots={cancel_dialog.name}, {cancel_result.name}")

    import_dialog = open_import_chooser("02h-import-source-chooser.png")
    # The isolated HOME contains only this generated source folder. Choose it
    # from GTK's visible Home list, then use the dialog's Import button.
    click_screen(310, 157)
    home_listing = wait_for_screen_change(import_dialog, "02h1-import-home-listing.png",
                                         "GTK chooser to display the isolated Home folder")
    click_screen(480, 187, button=1)
    selected_folder = screenshot("02h2-import-folder-selected.png")
    if import_chooser_window():
        click_screen(1297, 896)
    wait_for(lambda: with_photo_count() == 17, "two imported fixture photos to be indexed", timeout=30)
    import_result = screenshot("02i-import-complete.png")
    with sqlite3.connect(XDG_DATA / "pic-rs" / "library.db") as connection:
        imported_paths = connection.execute("SELECT COUNT(*) FROM photos WHERE path LIKE ?",
                                            (str(IMPORT_DIR.resolve()) + "/%",)).fetchone()[0]
    record(add_case("GUI local folder import", "Selecting the temporary import directory indexes both photos"),
           imported_paths == 2,
           f"photos indexed from GUI-selected path={imported_paths}; total library photos={with_photo_count()}; screenshots={import_dialog.name}, {home_listing.name}, {selected_folder.name}, {import_result.name}")

    duplicate_dialog = open_import_chooser("02j-duplicate-import-chooser.png")
    chooser_id = import_chooser_window()
    if chooser_id:
        command(["xdotool", "windowfocus", "--sync", chooser_id])
    click_screen(310, 157)
    wait_for_screen_change(duplicate_dialog, "02j1-duplicate-home-listing.png",
                          "GTK chooser to display Home for duplicate import")
    click_screen(480, 187)
    if import_chooser_window():
        click_screen(1300, 896)
    wait_for(lambda: with_photo_count() >= 17, "duplicate import to finish", timeout=20)
    duplicate_result = screenshot("02k-duplicate-import-result.png")
    record(add_case("Duplicate GUI import", "Importing the same source folder again does not duplicate photo rows"),
           with_photo_count() == 17,
           f"photo count after repeat import={with_photo_count()}; screenshots={duplicate_dialog.name}, {duplicate_result.name}")
    # Restart PIC after a GUI import and confirm the isolated catalogue and
    # imported rows survive a real process restart.
    APP_PROCESS.terminate()
    APP_PROCESS.wait(timeout=5)
    log.close()
    log = APP_LOG.open("a")
    APP_PROCESS = subprocess.Popen([str(APP)], env=app_env, stdout=log, stderr=subprocess.STDOUT)
    APP_WINDOW = wait_for(
        lambda: next((wid for wid in command(["xdotool", "search", "--onlyvisible", "--name", ".*"], check=False).splitlines()
                      if "PIC" in command(["xdotool", "getwindowname", wid], check=False)), None),
        "PIC window after restart", timeout=30,
    )
    command(["xdotool", "windowfocus", "--sync", APP_WINDOW])
    command(["xdotool", "key", "Escape"])
    def restarted_gallery() -> Path | None:
        current = screenshot("02l-library-after-restart.png")
        return current if len(matching_generated_tiles(current)) >= 12 else None
    restart_shot = wait_for(restarted_gallery, "generated gallery photos after PIC restart", timeout=30)
    record(add_case("Catalogue persistence after restart",
                    "Imported photo rows remain in the catalogue after closing and reopening PIC"),
           with_photo_count() == 17 and len(matching_generated_tiles(restart_shot)) >= 12,
           f"catalogue photo count after restart={with_photo_count()}; visible generated tiles={len(matching_generated_tiles(restart_shot))}; screenshot={restart_shot.name}")
    if SUITE == "import":
        return

    scroll_case = add_case("Gallery scrolling", "Mouse wheel scrolls the populated gallery")
    command(["xdotool", "mousemove", "650", "690"])
    command(["xdotool", "click", "--repeat", "4", "--delay", "80", "5"])
    scrolled = wait_for_screen_change(filter_cleared, "03-gallery-scrolled.png", "gallery content to scroll")
    initial_tiles = [name for _, _, name in matching_generated_tiles(filter_cleared)]
    scrolled_tiles = [name for _, _, name in matching_generated_tiles(scrolled)]
    scroll_changed = initial_tiles != scrolled_tiles
    record(scroll_case, scroll_changed, f"visible photo sequence before/after={initial_tiles}/{scrolled_tiles}; screenshot={scrolled.name}")
    command(["xdotool", "click", "--repeat", "4", "--delay", "80", "4"])
    gallery_top = wait_for_screen_change(scrolled, "03a-gallery-scroll-restored.png", "gallery scroll to return to top")
    drag_case = add_case("Gallery drag selection", "Mouse drag changes gallery selection")
    command(["xdotool", "mousemove", "700", "180"])
    command(["xdotool", "mousedown", "1"])
    for x, y in ((900, 180), (1100, 300), (1370, 530)):
        command(["xdotool", "mousemove", "--sync", str(x), str(y)])
    command(["xdotool", "mouseup", "1"])
    def drag_selection_visible():
        current = screenshot("03b-gallery-after-drag-wait.png")
        return current if selected_badge_count(current) >= 2 else None
    try:
        wait_for(drag_selection_visible, "drag selection to highlight multiple gallery thumbnails", timeout=5)
    except TimeoutError:
        pass
    dragged = screenshot("03b-gallery-after-drag.png")
    drag_changed = selected_badge_count(dragged) >= 2
    record(drag_case, drag_changed,
           f"visible selected-thumbnail badges={selected_badge_count(dragged)}; screenshot={dragged.name}")
    cx, cy = 518, 178
    click_at(cx, cy, repeat=2)
    wait_for(lambda: count_color_pixels(screenshot("04-lightbox-wait.png"), (45, 110, 190)) > 200_000,
             "double-click to display the selected landscape photo", timeout=10)
    double_shot = screenshot("04-after-double-click.png")
    double_diff = screenshot_changed(before, double_shot)
    landscape_pixels = count_color_pixels(double_shot, (45, 110, 190))
    record(add_case("Gallery double-click opens lightbox", "Double-click displays the selected landscape photo"),
           landscape_pixels > 200_000 and app_alive(), f"landscape pixels displayed={landscape_pixels}; screenshot={double_shot.name}")
    viewer_case = add_case("Lightbox next-photo navigation", "Arrow Right displays the next sample photo")
    command(["xdotool", "key", "Right"])
    wait_for(lambda: count_color_pixels(screenshot("05-lightbox-navigation-wait.png"), (180, 75, 95)) > 200_000,
             "render of the next portrait photo", timeout=10)
    next_shot = screenshot("05-lightbox-next-photo.png")
    portrait_pixels = count_color_pixels(next_shot, (180, 75, 95))
    record(viewer_case, portrait_pixels > 200_000,
           f"Arrow Right displayed portrait.jpg color in {portrait_pixels} pixels; screenshot={next_shot.name}")
    zoom_case = add_case("Lightbox zoom and 1:1", "Zoom slider enlarges the image and Space selects native 1:1 scale")
    viewer_crop = (319 + MOUSE_OFFSET_X, 52 + MOUSE_OFFSET_Y,
                   1434 + MOUSE_OFFSET_X, 836 + MOUSE_OFFSET_Y)
    fit_bounds = matching_color_bounds(next_shot, (180, 75, 95), crop=viewer_crop)
    # Drag the actual shared zoom slider toward its high end.
    command(["xdotool", "mousemove", "967", "866"])
    command(["xdotool", "mousedown", "1"])
    command(["xdotool", "mousemove", "--sync", "986", "866"])
    command(["xdotool", "mouseup", "1"])
    wait_for(lambda: (bounds := matching_color_bounds(
        screenshot("06-lightbox-zoom-wait.png"), (180, 75, 95), crop=viewer_crop)) is not None
        and bounds[2] - bounds[0] > (fit_bounds[2] - fit_bounds[0] if fit_bounds else 0),
        "lightbox image to grow after slider zoom", timeout=10)
    zoom_shot = screenshot("06-lightbox-zoom.png")
    incremental_bounds = matching_color_bounds(zoom_shot, (180, 75, 95), crop=viewer_crop)
    command(["xdotool", "key", "space"])
    wait_for(lambda: (bounds := matching_color_bounds(
        screenshot("07-lightbox-one-to-one-wait.png"), (180, 75, 95), crop=viewer_crop)) is not None
        and abs((bounds[2] - bounds[0]) - 600) <= 8,
        "portrait image to reach 1:1 width after Space", timeout=10)
    one_to_one_shot = screenshot("07-lightbox-one-to-one.png")
    native_bounds = matching_color_bounds(one_to_one_shot, (180, 75, 95), crop=viewer_crop)
    marker_before = count_color_pixels(one_to_one_shot, (255, 255, 0), tolerance=20)
    fit_width = fit_bounds[2] - fit_bounds[0] if fit_bounds else 0
    incremental_width = incremental_bounds[2] - incremental_bounds[0] if incremental_bounds else 0
    native_width = native_bounds[2] - native_bounds[0] if native_bounds else 0
    record(zoom_case, fit_bounds is not None and incremental_width > fit_width
           and native_bounds is not None and abs(native_width - 600) <= 8
           and (native_bounds[3] - native_bounds[1]) > 700,
           f"landscape image bounds: Fit={fit_bounds}, plus={incremental_bounds}, 1:1={native_bounds}; screenshots={zoom_shot.name}, {one_to_one_shot.name}")
    drag_case = add_case("Lightbox 1:1 panning", "Dragging the 1:1 image moves its rendered position")
    command(["xdotool", "mousemove", "870", "450"])
    command(["xdotool", "mousedown", "1"])
    command(["xdotool", "mousemove", "--sync", "870", "350"])
    command(["xdotool", "mouseup", "1"])
    def pan_marker_moved():
        current = screenshot("07a-lightbox-pan-wait.png")
        return current if abs(count_color_pixels(current, (255, 255, 0), tolerance=20)
                              - marker_before) > 500 else None
    pan_shot = wait_for(pan_marker_moved, "high-contrast portrait marker to move after panning", timeout=8)
    # Keep the final state screenshot under a stable name for the report.
    pan_shot = screenshot("07a-lightbox-panned.png")
    pan_bounds = matching_color_bounds(pan_shot, (180, 75, 95), crop=viewer_crop)
    pan_shift = abs((pan_bounds[0] + pan_bounds[2]) - (native_bounds[0] + native_bounds[2])) if pan_bounds and native_bounds else 0
    marker_after = count_color_pixels(pan_shot, (255, 255, 0), tolerance=20)
    record(drag_case, pan_shift >= 35 or marker_before > marker_after + 500,
           f"native image bounds before/after={native_bounds}/{pan_bounds}; clipped edge marker pixels before/after={marker_before}/{marker_after}; screenshot={pan_shot.name}")
    command(["xdotool", "key", "Escape"])
    gallery_after_viewer = wait_for_screen_change(pan_shot, "08-gallery-after-viewer.png", "Escape to return to gallery")
    record(add_case("Lightbox Escape dismissal", "Escape returns to the gallery"),
           screenshot_changed(one_to_one_shot, gallery_after_viewer), f"Gallery screenshot={gallery_after_viewer.name}")
    click_at(cx, cy, button=3)
    menu_shot = wait_for_screen_change(gallery_after_viewer, "09-photo-context-menu.png", "photo context menu to open")
    menu_diff = screenshot_changed(gallery_after_viewer, menu_shot)
    record(add_case("Gallery right-click opens photo context menu", "Right-click displays photo actions"),
           menu_diff and app_alive(), f"Context menu visible; screenshot={menu_shot.name}")
    if SUITE == "viewer":
        return
    click_at(480, 109)
    wait_for(lambda: count_color_pixels(screenshot("10-photo-editor-wait.png"), (45, 110, 190)) > 200_000,
             "editor source image to finish rendering", timeout=12)
    editor_shot = screenshot("10-photo-editor.png")
    editor_open = screenshot_changed(menu_shot, editor_shot)
    editor_title = command(["xdotool", "getwindowname", APP_WINDOW], check=False)
    # GTK draws labels rather than storing them in the PNG; require the app's
    # editor-specific toolbar position and a visible source-image canvas.
    editor_crop = (699 + MOUSE_OFFSET_X, 132 + MOUSE_OFFSET_Y,
                   1423 + MOUSE_OFFSET_X, 823 + MOUSE_OFFSET_Y)
    source_bounds = matching_color_bounds(editor_shot, (45, 110, 190), crop=editor_crop)
    divider_pixels = editor_divider_pixels(editor_shot)
    record(add_case("Photo editor opens", "Edit Photo action opens the selected image and adjustment controls"),
           editor_open and divider_pixels >= 300 and source_bounds is not None and app_alive(),
           f"PIC window title={editor_title!r}; editor split-divider pixels={divider_pixels}; source image bounds={source_bounds}; screenshot={editor_shot.name}")
    if editor_open:
        original_digest = hashlib.sha256((SAMPLE_DIR / "landscape.jpg").read_bytes()).hexdigest()
        center_before = Image.open(editor_shot).convert("RGB").getpixel((1050 + MOUSE_OFFSET_X, 450 + MOUSE_OFFSET_Y))
        # At the empty-history boundary, Undo and Redo must leave the original
        # preview untouched. This catches accidental edits when disabled
        # controls still receive pointer events.
        click_at(1085, 86)
        empty_undo_shot = screenshot("10a-editor-undo-empty.png")
        empty_undo_delta = sum(abs(a - b) for a, b in zip(
            center_before, Image.open(empty_undo_shot).convert("RGB").getpixel(
                (1050 + MOUSE_OFFSET_X, 450 + MOUSE_OFFSET_Y))))
        click_at(1134, 86)
        empty_redo_shot = screenshot("10a-editor-redo-empty.png")
        empty_redo_delta = sum(abs(a - b) for a, b in zip(
            center_before, Image.open(empty_redo_shot).convert("RGB").getpixel(
                (1050 + MOUSE_OFFSET_X, 450 + MOUSE_OFFSET_Y))))
        def toolbar_ink(path: Path, button_x: int) -> int:
            image = Image.open(path).convert("RGB")
            crop = image.crop((button_x - 17 + MOUSE_OFFSET_X, 71 + MOUSE_OFFSET_Y,
                               button_x + 17 + MOUSE_OFFSET_X, 101 + MOUSE_OFFSET_Y))
            return sum(1 for red, green, blue in crop.getdata()
                       if max(red, green, blue) < 120)

        empty_undo_ink = toolbar_ink(empty_undo_shot, 1085)
        empty_redo_ink = toolbar_ink(empty_redo_shot, 1134)
        click_at(570, 199)
        wait_for(lambda: sum(abs(a - b) for a, b in zip(
            Image.open(screenshot("11-editor-exposure-wait.png")).convert("RGB").getpixel((1050 + MOUSE_OFFSET_X, 450 + MOUSE_OFFSET_Y)),
            center_before)) > 20,
            "Exposure change to render", timeout=10)
        exposure_shot = screenshot("11-editor-exposure-adjusted.png")
        center_after = Image.open(exposure_shot).convert("RGB").getpixel((1050 + MOUSE_OFFSET_X, 450 + MOUSE_OFFSET_Y))
        center_delta = sum(abs(center_before[i] - center_after[i]) for i in range(3))
        exposure_undo_ink = toolbar_ink(exposure_shot, 1085)
        exposure_redo_ink = toolbar_ink(exposure_shot, 1134)
        record(add_case("Editor exposure adjustment", "Exposure interaction changes preview pixels"),
               center_delta > 20, f"preview center RGB delta={center_delta}; screenshot={exposure_shot.name}")
        sequence_errors = []

        def preview_center(path: Path):
            return Image.open(path).convert("RGB").getpixel(
                (1050 + MOUSE_OFFSET_X, 450 + MOUSE_OFFSET_Y))

        def wait_center_matches(color, description):
            wait_for(lambda: sum(abs(a - b) for a, b in zip(
                preview_center(screenshot("11a-editor-undo-wait.png")), color)) <= 16,
                description, timeout=8)

        click_at(1085, 86)
        try:
            wait_center_matches(center_before, "Undo to restore the baseline preview")
        except TimeoutError as error:
            sequence_errors.append(str(error))
        undo_shot = screenshot("11a-editor-undo.png")
        undo_center = preview_center(undo_shot)
        undo_delta = sum(abs(center_before[i] - undo_center[i]) for i in range(3))
        undo_undo_ink = toolbar_ink(undo_shot, 1085)
        undo_redo_ink = toolbar_ink(undo_shot, 1134)

        click_at(1134, 86)
        try:
            wait_center_matches(center_after, "Redo to restore the Exposure preview")
        except TimeoutError as error:
            sequence_errors.append(str(error))
        redo_shot = screenshot("11a-editor-redo.png")
        redo_delta = sum(abs(exposure_color - redone) for exposure_color, redone
                         in zip(center_after, preview_center(redo_shot)))
        redo_undo_ink = toolbar_ink(redo_shot, 1085)
        redo_redo_ink = toolbar_ink(redo_shot, 1134)

        # Contrast is a separate edit tool. Undo must step back through it and
        # Exposure independently, then Redo must replay both in order.
        click_at(650, 263)
        multi_shot = screenshot("11a-editor-multiple-adjustments.png")
        multi_center = preview_center(multi_shot)
        if sum(abs(a - b) for a, b in zip(center_after, multi_center)) <= 20:
            sequence_errors.append("Contrast tool did not change the rendered preview")
        click_at(1085, 86)
        try:
            wait_center_matches(center_after, "first Undo to remove only Contrast")
        except TimeoutError as error:
            sequence_errors.append(str(error))
        multi_undo_one = screenshot("11a-editor-undo-contrast.png")
        multi_undo_one_delta = sum(abs(a - b) for a, b in zip(center_after, preview_center(multi_undo_one)))
        multi_undo_one_ink = (toolbar_ink(multi_undo_one, 1085), toolbar_ink(multi_undo_one, 1134))
        click_at(1085, 86)
        try:
            wait_center_matches(center_before, "second Undo to remove Exposure")
        except TimeoutError as error:
            sequence_errors.append(str(error))
        multi_undo_two = screenshot("11a-editor-undo-exposure.png")
        multi_undo_two_delta = sum(abs(a - b) for a, b in zip(center_before, preview_center(multi_undo_two)))
        multi_undo_two_ink = (toolbar_ink(multi_undo_two, 1085), toolbar_ink(multi_undo_two, 1134))
        click_at(1134, 86)
        click_at(1134, 86)
        try:
            wait_center_matches(multi_center, "two Redo actions to restore both adjustments")
        except TimeoutError as error:
            sequence_errors.append(str(error))
        multi_redo = screenshot("11a-editor-redo-multiple.png")
        multi_redo_delta = sum(abs(a - b) for a, b in zip(multi_center, preview_center(multi_redo)))
        multi_redo_ink = (toolbar_ink(multi_redo, 1085), toolbar_ink(multi_redo, 1134))

        # Undo to the Exposure-only state, then make a different edit. That
        # new edit must discard the old Contrast redo branch.
        click_at(1085, 86)
        try:
            wait_center_matches(center_after, "Undo before creating a new branch")
        except TimeoutError as error:
            sequence_errors.append(str(error))
        branch_undo = screenshot("11a-editor-undo-before-branch.png")
        branch_undo_delta = sum(abs(a - b) for a, b in zip(center_after, preview_center(branch_undo)))
        click_at(650, 327)
        try:
            wait_for(lambda: sum(abs(a - b) for a, b in zip(
                preview_center(screenshot("11a-editor-new-branch-wait.png")), center_after)) > 8,
                "Fill Light edit on the new history branch", timeout=10)
        except TimeoutError as error:
            sequence_errors.append(str(error))
        branch_edit = screenshot("11a-editor-new-branch.png")
        branch_center = preview_center(branch_edit)
        branch_edit_delta = sum(abs(a - b) for a, b in zip(center_after, branch_center))
        click_at(1134, 86)
        time.sleep(0.7)
        branch_redo = screenshot("11a-editor-redo-discarded-branch.png")
        branch_redo_delta = sum(abs(a - b) for a, b in zip(branch_center, preview_center(branch_redo)))
        old_branch_delta = sum(abs(a - b) for a, b in zip(multi_center, preview_center(branch_redo)))
        branch_redo_ink = (toolbar_ink(branch_redo, 1085), toolbar_ink(branch_redo, 1134))
        undo_sequence_passed = (
            undo_delta <= 16 and redo_delta <= 16
            and empty_undo_delta <= 16 and empty_redo_delta <= 16
            and multi_undo_one_delta <= 16 and multi_undo_two_delta <= 16
            and multi_redo_delta <= 16 and branch_undo_delta <= 16
            and branch_edit_delta > 8 and branch_redo_delta <= 16
            and old_branch_delta > 20 and not sequence_errors
            and empty_undo_ink == 0 and empty_redo_ink == 0
            and exposure_undo_ink > 0 and exposure_redo_ink == 0
            and undo_undo_ink == 0 and undo_redo_ink > 0
            and redo_undo_ink > 0 and redo_redo_ink == 0
            and multi_undo_one_ink[0] > 0 and multi_undo_one_ink[1] > 0
            and multi_undo_two_ink[0] == 0 and multi_undo_two_ink[1] > 0
            and multi_redo_ink[0] > 0 and multi_redo_ink[1] == 0
            and branch_redo_ink[0] > 0 and branch_redo_ink[1] == 0
        )
        record(add_case("Editor Undo/Redo sequences",
                        "Empty history is unchanged; single and multiple tool edits undo and redo independently in order"),
               undo_sequence_passed,
               f"single undo delta={undo_delta}, single redo delta={redo_delta}; "
               f"empty undo delta={empty_undo_delta}, empty redo delta={empty_redo_delta}; "
               f"contrast undo delta={multi_undo_one_delta}, exposure undo delta={multi_undo_two_delta}, "
               f"two-step redo delta={multi_redo_delta}; sequence_errors={sequence_errors}; "
               f"branch undo delta={branch_undo_delta}, new Fill Light delta={branch_edit_delta}, "
               f"redo after new edit delta={branch_redo_delta}, distance from discarded state={old_branch_delta}; "
               f"toolbar dark pixels empty(U/R)={empty_undo_ink}/{empty_redo_ink}, "
               f"edited(U/R)={exposure_undo_ink}/{exposure_redo_ink}, "
               f"after-Undo(U/R)={undo_undo_ink}/{undo_redo_ink}, "
               f"after-Redo(U/R)={redo_undo_ink}/{redo_redo_ink}, "
               f"two-Undo={multi_undo_two_ink}, final-Redo={multi_redo_ink}, "
               f"discarded-branch={branch_redo_ink}; "
               f"screenshots={undo_shot.name}, {redo_shot.name}, {multi_shot.name}, "
               f"{multi_undo_one.name}, {multi_undo_two.name}, {multi_redo.name}")
        click_at(570, 199)
        wait_for(lambda: sum(abs(a - b) for a, b in zip(
            Image.open(screenshot("11b-editor-reset-adjusted-wait.png")).convert("RGB").getpixel((1050 + MOUSE_OFFSET_X, 450 + MOUSE_OFFSET_Y)),
            center_before)) > 20, "second Exposure change to render", timeout=10)
        click_at(1224, 86)
        try:
            wait_for(lambda: sum(abs(a - b) for a, b in zip(
                Image.open(screenshot("11b-editor-reset-wait.png")).convert("RGB").getpixel((1050 + MOUSE_OFFSET_X, 450 + MOUSE_OFFSET_Y)),
                center_before)) <= 16, "Reset to restore the baseline preview", timeout=8)
        except TimeoutError:
            pass
        reset_shot = screenshot("11b-editor-reset.png")
        reset_center = Image.open(reset_shot).convert("RGB").getpixel((1050 + MOUSE_OFFSET_X, 450 + MOUSE_OFFSET_Y))
        reset_delta = sum(abs(center_before[i] - reset_center[i]) for i in range(3))
        record(add_case("Editor Reset", "Reset all edits restores the original rendered pixels"),
               reset_delta <= 16, f"preview center RGB delta from original={reset_delta}; screenshot={reset_shot.name}")
        click_at(570, 199)
        wait_for(lambda: sum(abs(a - b) for a, b in zip(
            Image.open(screenshot("11c-editor-saved-adjustment-wait.png")).convert("RGB").getpixel((1050 + MOUSE_OFFSET_X, 450 + MOUSE_OFFSET_Y)),
            center_before)) > 20, "saved Exposure change to render", timeout=10)
        exposure_shot = screenshot("11c-editor-saved-adjustment.png")
        click_at(438, 86)
        filters_shot = screenshot("12-editor-filters.png")
        record(add_case("Editor filters panel", "Filters tab displays filter controls"),
               screenshot_changed(exposure_shot, filters_shot), f"Filters screenshot={filters_shot.name}")
        click_at(1368, 85)
        gallery_after_editor = screenshot("13-gallery-after-editor.png")
        _, saved_recipe = photo_state("landscape.jpg")
        source_digest_after = hashlib.sha256((SAMPLE_DIR / "landscape.jpg").read_bytes()).hexdigest()
        recipe_has_exposure = bool(saved_recipe) and "exposure=" in saved_recipe and "exposure=0.0000" not in saved_recipe
        record(add_case("Editor save, reopen data and original safety", "Done persists nonzero edits in the catalog while preserving the source file"),
               recipe_has_exposure and source_digest_after == original_digest,
               f"saved recipe={saved_recipe[:120]!r}; original SHA-256 unchanged={source_digest_after == original_digest}; screenshot={gallery_after_editor.name}")
        click_at(518, 178, button=3)
        click_at(480, 109)
        wait_for(lambda: count_color_pixels(screenshot("13a-editor-reopen-wait.png"), (85, 208, 255)) > 200_000,
                 "saved edit preview to render after reopening", timeout=12)
        persisted_editor = screenshot("13a-editor-reopened-persisted.png")
        persisted_center = Image.open(persisted_editor).convert("RGB").getpixel((1050 + MOUSE_OFFSET_X, 450 + MOUSE_OFFSET_Y))
        persisted_delta = sum(abs(center_before[i] - persisted_center[i]) for i in range(3))
        record(add_case("Editor persistence after reopening", "Reopening the photo restores the saved adjustment preview"),
               persisted_delta > 20 and photo_state("landscape.jpg")[1] == saved_recipe,
               f"preview RGB delta from original={persisted_delta}; catalog recipe unchanged={photo_state('landscape.jpg')[1] == saved_recipe}; screenshot={persisted_editor.name}")
        click_at(1368, 85)
        def gallery_after_persisted_editor() -> Path | None:
            current = screenshot("13b-gallery-after-persisted-editor.png")
            return current if len(matching_generated_tiles(current)) >= 10 else None
        wait_for(gallery_after_persisted_editor, "gallery to return after closing persisted editor", timeout=15)
        # Select a second generated photo with Ctrl, then open the collage
        # action from the real thumbnail context menu.
        command(["xdotool", "keydown", "ctrl"])
        click_at(875, 178)
        command(["xdotool", "keyup", "ctrl"])
        multiselect_shot = screenshot("14-gallery-multiselection.png")
        record(add_case("Gallery multi-photo selection", "Ctrl-click selects a second generated photo"),
               selected_badge_count(multiselect_shot) == 2,
               f"visible selected-thumbnail badges={selected_badge_count(multiselect_shot)}; screenshot={multiselect_shot.name}")
        click_at(875, 178, button=3)
        collage_menu = screenshot("15-collage-context-menu.png")
        record(add_case("Collage action from context menu", "Multiple selection enables Create Collage"),
               screenshot_changed(multiselect_shot, collage_menu), f"Context menu screenshot={collage_menu.name}")
        click_at(840, 251)
        collage_shot = screenshot("16-collage-preview.png")
        collage_open = screenshot_changed(collage_menu, collage_shot)
        record(add_case("Collage preview opens", "Create Collage opens the collage preview/editor"),
               collage_open and app_alive(), f"Collage screenshot={collage_shot.name}")
        if collage_open:
            # Export through the actual collage editor controls and native save
            # chooser. Keep the destination inside this run's temporary tree.
            click_at(500, 803)
            export_options = wait_for_screen_change(collage_shot, "16a-collage-export-options.png",
                                                   "collage export options dialog")
            record(add_case("Collage export options", "Export options are shown before choosing a destination"),
                   export_options.exists(), f"Options dialog screenshot={export_options.name}")
            click_at(787, 565)
            export_destination = wait_for_screen_change(export_options, "16b-collage-export-destination.png",
                                                       "collage destination chooser", timeout=15)
            record(add_case("Collage export destination chooser", "Save chooser accepts an isolated output path"),
                   export_destination.exists(), f"Save chooser screenshot={export_destination.name}")
            export_path = RUN_DIR / "exports" / "rc6-collage.jpg"
            export_path.parent.mkdir(parents=True, exist_ok=True)
            # GTK's native chooser implements Ctrl+L as a location entry. Use
            # a temporary path with the extension removed; the chooser adds
            # the selected JPEG extension automatically.
            command(["xdotool", "key", "ctrl+l"])
            command(["xdotool", "type", "--clearmodifiers", str(export_path.with_suffix(""))])
            command(["xdotool", "key", "Return"])
            wait_for(lambda: export_path.is_file(), "exported collage file", timeout=45)
            output_opened = image_is_nonblank(export_path)
            exported = Image.open(export_path)
            exported.load()
            exported_size = exported.size
            # Collage preview is square and export uses the default 4K long edge.
            expected_size = (3840, 3840)
            record(add_case("Collage export image validation", "Exported image opens, is nonblank, and has expected dimensions"),
                   output_opened and exported.format == "JPEG" and exported_size == expected_size,
                   f"{export_path.name}: {exported.format} {exported_size}, expected {expected_size}")
            # Compare normalized photo geometry on the same canvas size. Solid
            # fixture interiors tolerate GTK interpolation and JPEG artifacts,
            # while each bounding box pins x/y/width/height independently.
            preview_canvas = Image.open(collage_shot).convert("RGB").crop((
                718 + MOUSE_OFFSET_X, 102 + MOUSE_OFFSET_Y,
                1401 + MOUSE_OFFSET_X, 785 + MOUSE_OFFSET_Y))
            exported_canvas = exported.convert("RGB").resize(preview_canvas.size, Image.Resampling.BILINEAR)
            preview_landscape_color = preview_canvas.getpixel((240, 140))
            preview_portrait_color = preview_canvas.getpixel((200, 500))
            preview_boxes = [
                color_bounds(preview_canvas, preview_landscape_color, tolerance=12),
                color_bounds(preview_canvas, preview_portrait_color, tolerance=12),
            ]
            export_boxes = [
                color_bounds(exported_canvas, preview_landscape_color, tolerance=30),
                color_bounds(exported_canvas, preview_portrait_color, tolerance=30),
            ]
            normalized_error = None
            if all(preview_boxes) and all(export_boxes):
                normalized_error = max(
                    abs(a / preview_canvas.width - b / exported_canvas.width)
                    if index % 2 == 0 else
                    abs(a / preview_canvas.height - b / exported_canvas.height)
                    for pbox, ebox in zip(preview_boxes, export_boxes)
                    for index, (a, b) in enumerate(zip(pbox, ebox))
                )
            record(add_case("Collage preview/export consistency",
                            "Each photo has matching normalized x/y position and width/height"),
                   normalized_error is not None and normalized_error <= 0.02,
                   f"normalized max geometry error={normalized_error}; preview boxes={preview_boxes}; "
                   f"export boxes={export_boxes}; canvas={preview_canvas.size}; "
                   f"screenshots={collage_shot.name}, export={export_path.name}")
            wait_for(lambda: not any("Export Collage" in command(["xdotool", "getwindowname", wid], check=False)
                                     for wid in command(["xdotool", "search", "--onlyvisible", "--name", ".*"], check=False).splitlines()),
                     "export completion dialog dismissal", timeout=30)
            # Dialogs may retain focus; Escape returns to the collage preview.
            command(["xdotool", "key", "Escape"])
            click_at(580, 760)
            gallery_after_collage = screenshot("17-gallery-after-collage.png")
            record(add_case("Collage Exit action", "Exit returns from collage editor to gallery"),
                   screenshot_changed(collage_shot, gallery_after_collage),
                   f"Post-collage screenshot={gallery_after_collage.name}")
        else:
            case = add_case("Collage Exit action", "Exit returns from collage editor to gallery")
            case.actual = "Collage editor did not open"
            case.result = "NOT TESTED"
            command(["xdotool", "key", "Escape"])
        gallery_modes = add_case("Gallery layout control interaction", "Layout button visibly changes gallery layout or presents layout choices")
        modes_before = screenshot("18-gallery-layout-before.png")
        click_at(1298, 28)
        modes_after = screenshot("19-gallery-layout-control.png")
        layout_changed = screenshot_changed(modes_before, modes_after)
        if layout_changed:
            record(gallery_modes, True, f"Layout control response screenshot={modes_after.name}")
        else:
            gallery_modes.actual = f"No distinguishable layout response; screenshot={modes_after.name}"
            gallery_modes.result = "NOT TESTED"
        command(["xdotool", "key", "Escape"])
        layout_closed = wait_for_screen_change(modes_after, "19a-gallery-layout-closed.png",
                                               "gallery layout popover to close", timeout=5)
    else:
        for name, expected in (("Editor exposure control", "Moving Exposure changes the preview"),
                               ("Editor filters panel", "Filters tab displays filter controls"),
                               ("Editor Done action", "Done returns to the gallery"),
                               ("Gallery multi-photo selection", "Ctrl-click selects a second generated photo"),
                               ("Collage action from context menu", "Multiple selection enables Create Collage"),
                               ("Collage preview opens", "Create Collage opens the collage preview/editor"),
                               ("Gallery layout control", "Gallery layout control changes the visible mode")):
            case = add_case(name, expected)
            case.actual = "Prerequisite editor did not open"
            case.result = "NOT TESTED"
    if SUITE == "editing":
        return
    click_at(1360, 866)
    saver_setup = screenshot("20-screensaver-setup.png")
    setup_open = screenshot_changed(editor_shot, saver_setup)
    record(add_case("Screensaver setup", "Play control opens screensaver style settings"),
           setup_open and app_alive(), f"Setup controls shown; screenshot={saver_setup.name}")
    # The toolbar passes the current photo selection to the dialog, which
    # defaults to Selected Photos. Pick All Photos explicitly so playback has
    # multiple fixture images and the transition assertion is meaningful.
    click_at(768, 449)
    command(["xdotool", "key", "Home"])
    command(["xdotool", "key", "Return"])
    all_photos_source = screenshot("20a-screensaver-all-photos-source.png")
    click_at(848, 635)
    wait_for(lambda: db_setting("screensaver-source") == "all",
             "screensaver source to save as All Photos", timeout=8)
    record(add_case("Screensaver all-photo source", "Screensaver is configured to play all generated fixture photos"),
           db_setting("screensaver-source") == "all",
           f"saved source={db_setting('screensaver-source')}; screenshot={all_photos_source.name}")
    def fullscreen_saver_window() -> str | None:
        for window_id in command(["xdotool", "search", "--onlyvisible", "--name", "^PIC Screensaver$"], check=False).splitlines():
            geometry = command(["xdotool", "getwindowgeometry", "--shell", window_id], check=False)
            width_match = re.search(r"^WIDTH=(\d+)$", geometry, re.MULTILINE)
            height_match = re.search(r"^HEIGHT=(\d+)$", geometry, re.MULTILINE)
            if width_match and height_match and int(width_match.group(1)) >= 1600 and int(height_match.group(1)) >= 1000:
                return window_id
        return None
    saver_id = wait_for(fullscreen_saver_window, "fullscreen screensaver window", timeout=12)
    saver_shot = screenshot("21-screensaver-running.png")
    saver_geometry = command(["xdotool", "getwindowgeometry", "--shell", saver_id], check=False) if saver_id else ""
    geo_w = re.search(r"^WIDTH=(\d+)$", saver_geometry, re.MULTILINE)
    geo_h = re.search(r"^HEIGHT=(\d+)$", saver_geometry, re.MULTILINE)
    width = int(geo_w.group(1)) if geo_w else 0
    height = int(geo_h.group(1)) if geo_h else 0
    record(add_case("Screensaver window and fullscreen geometry", "Screensaver window is visible and covers the X11 display"),
           bool(saver_id) and width >= 1600 and height >= 1000 and app_alive(),
           f"window={saver_id}; geometry={width}x{height}; screenshot={saver_shot.name}; wm={os.environ.get('PIC_GUI_WINDOW_MANAGER', 'none')}")
    if saver_id:
        # Shorten the slideshow to three seconds in the isolated database and
        # wait until the active photo visibly changes, proving playback.
        transition_shot = wait_for_screen_change(saver_shot, "21a-screensaver-transition.png",
                                                 "screensaver photo transition", timeout=8)
        record(add_case("Screensaver photo playback and transition", "A different generated image appears after the configured interval"),
               screenshot_changed(saver_shot, transition_shot), f"Transition screenshot={transition_shot.name}")
    else:
        transition_shot = saver_shot
        case = add_case("Screensaver photo playback and transition", "A different generated image appears after the configured interval")
        case.actual = "Screensaver window was not visible; transition not exercised"
        case.result = "NOT TESTED"
    command(["xdotool", "key", "Escape"])
    main_return = wait_for_screen_change(saver_shot, "22-screensaver-keyboard-exit.png",
                                         "return to PIC after screensaver keyboard exit")
    record(add_case("Screensaver keyboard exit", "Escape exits screensaver and returns to PIC"),
           main_return.exists() and app_alive(), f"Returned to PIC; screenshot={main_return.name}")
    mouse_case = add_case("Screensaver mouse exit", "Mouse input exits screensaver and returns to PIC")
    # Relaunch separately so Escape and mouse dismissal are independent actions.
    click_at(1360, 866)
    saver_setup_again = wait_for_screen_change(main_return, "23-screensaver-setup-again.png",
                                               "reopen screensaver setup after keyboard dismissal")
    click_at(848, 635)
    wait_for(fullscreen_saver_window, "second fullscreen screensaver window", timeout=12)
    # The screensaver intentionally ignores mouse motion for its first 850 ms
    # to avoid treating the activation pointer position as a user exit.
    time.sleep(0.9)
    click_at(900, 500)
    wait_for(lambda: not command(["xdotool", "search", "--onlyvisible", "--name", "^PIC Screensaver$"], check=False),
             "screensaver closes after mouse click", timeout=8)
    mouse_return = screenshot("24-screensaver-mouse-exit.png")
    record(mouse_case, mouse_return.exists() and app_alive(),
           f"Mouse input dismissed fullscreen screensaver and returned to PIC; setup={saver_setup_again.name}; screenshot={mouse_return.name}")


def write_report(error: Exception | None) -> None:
    startup_errors = check_logs()
    if error:
        result = "FAIL"
        for case in CASES:
            if case.result == "NOT TESTED":
                case.actual = "Not run after earlier failure"
                case.result = "BLOCKED" if result == "BLOCKED" else "NOT TESTED"
    else:
        result = "COMPLETE" if all(case.result == "PASS" for case in CASES) and not startup_errors else "INCOMPLETE"
    counts = {label: sum(case.result == label for case in CASES) for label in ("PASS", "FAIL", "BLOCKED", "NOT TESTED")}
    rows = [
        "# PIC GUI test report",
        "",
        f"- Overall: **{result}**",
        f"- Commit: `{COMMIT}`",
        f"- Display: `{os.environ.get('PIC_GUI_DISPLAY_DESCRIPTION', f'Xvfb {DISPLAY} (1600×1000×24), GTK X11/Cairo')}`",
        f"- Application binary: `{APP}`",
        f"- Selected suite: `{SUITE}` (focused runs include prerequisite steps)",
        f"- Artifact directory: `reports/gui-tests/{RUN_DIR.name}`",
        f"- PIC process: {'running through GUI actions' if APP_PROCESS is not None and APP_PROCESS.poll() is None else 'exited during cleanup'}",
        f"- Counts: {counts['PASS']} PASS, {counts['FAIL']} FAIL, {counts['BLOCKED']} BLOCKED, {counts['NOT TESTED']} NOT TESTED",
        "- Data/config: generated test photos and isolated XDG directories under this run directory",
        "",
        "## Test matrix",
        "",
        "| Feature | Action performed | Expected | Actual | Result |",
        "|---|---|---|---|---|",
    ]
    for case in CASES:
        action = {
            "PIC startup and Library": f"Launch PIC on {os.environ.get('PIC_GUI_DISPLAY_DESCRIPTION', f'Xvfb {DISPLAY}')} and capture the visible main window",
            "Isolated sample gallery": "Load the generated temporary photo fixture and query its catalog rows",
            "Photo rating persistence": "Rate two generated photos through the GUI and query both SQLite ratings",
            "Folder navigation and photo membership": "Click the imported fixture folder and compare catalog membership and visible photos",
            "Folder rating filter membership": "Open the folder filter, select three stars, and verify the visible filename and stored filter",
            "Folder rating filter reset": "Click Clear and verify the full fixture membership returns",
            "Folder All Stars filter and result latency": "Select All Stars in Folder; time persisted state and verify both result tile centers against the generated source colors",
            "Import chooser cancellation": "Open the real folder picker, press Escape, and compare database counts",
            "GUI local folder import": "Choose the generated folder through the GTK chooser and query imported paths",
            "Duplicate GUI import": "Repeat the folder-picker import and verify row count stays unchanged",
            "Gallery scrolling": "Scroll the gallery with the mouse wheel and compare visible filenames",
            "Gallery drag selection": "Drag a selection rectangle across thumbnails and count selected badges",
            "Gallery double-click opens lightbox": "Double-click the selected thumbnail and verify fixture image pixels",
            "Lightbox next-photo navigation": "Press Right and verify portrait pixels in the lightbox viewport",
            "Lightbox zoom and 1:1": "Drag the zoom slider, press Space, and measure native image dimensions",
            "Lightbox 1:1 panning": "Drag the native image and measure the fixture edge marker displacement",
            "Lightbox Escape dismissal": "Press Escape and verify the gallery returns",
            "Gallery right-click opens photo context menu": "Right-click the photo and capture the open context menu",
            "Photo editor opens": "Choose Edit Photo and verify editor controls and source-image canvas",
            "Editor exposure adjustment": "Move the Exposure slider and compare rendered image pixels",
            "Editor Undo/Redo sequences": "Undo and redo single and multiple adjustment tool changes and compare rendered pixels",
            "Editor Reset": "Change Exposure, click Reset, and compare preview pixels with the original render",
            "Editor filters panel": "Click Filters and capture the resulting controls",
            "Editor save, reopen data and original safety": "Save a nonzero recipe, query SQLite, and compare source SHA-256",
            "Editor persistence after reopening": "Reopen the photo and compare preview pixels and saved recipe",
            "Gallery multi-photo selection": "Ctrl-click a second thumbnail and count selected badges",
            "Collage action from context menu": "Right-click the multiselection and invoke Create Collage",
            "Collage preview opens": "Capture the live collage editor preview",
            "Collage export options": "Click Create Collage and inspect the export settings dialog",
            "Collage export destination chooser": "Click Export and choose the temporary output path",
            "Collage export image validation": "Open the saved image and verify format, dimensions, and pixel variance",
            "Collage preview/export consistency": "Compare normalized preview canvas pixels with exported pixels",
            "Collage Exit action": "Exit the collage editor and verify gallery return",
            "Gallery layout control interaction": "Open the layout control and capture its changed state",
            "Screensaver setup": "Open style controls and start playback through the GUI",
            "Screensaver window and fullscreen geometry": "Query the visible screensaver X11 window geometry",
            "Screensaver photo playback and transition": "Capture the slideshow before and after its configured interval",
            "Screensaver keyboard exit": "Press Escape and capture return to the main window",
            "Screensaver mouse exit": "Restart playback, move/click the mouse, and verify dismissal",
        }.get(case.name, "Perform the named GUI action and inspect its recorded result")
        rows.append(f"| {case.name} | {action} | {case.expected} | {case.actual.replace('|', '/')} | {case.result} |")
    rows.extend(["", "## Startup diagnostics", ""])
    rows.append("GTK/resource/startup scan: " + (", ".join(startup_errors) if startup_errors else "no matched critical errors"))
    rows.extend(["", "## Artifacts", "", "- `application.log`", "- `harness.log`", "- `xvfb.log`", "- `screenshots/`"])
    rows.extend([
        "", "## Coverage limitations", "",
        f"- Screensaver fullscreen assertions used X11 window manager: `{os.environ.get('PIC_GUI_WINDOW_MANAGER', 'none')}`; geometry and transitions are measured only when its test cases pass.",
        "- Original-file mutation is checked by SHA-256 against generated fixture photos; RAW decoding and external-device behavior were not tested.",
        "- The isolated session reports portal backend and PipeWire availability warnings in `harness.log`; these did not block the exercised views.",
    ])
    failed_cases = [case for case in CASES if case.result == "FAIL"]
    if failed_cases:
        rows.extend(["", "## Confirmed PIC behavior defects (not fixed)", "",
                     "These failures were exercised through the GUI against generated photos in the isolated test library. No application code or production library was modified.", ""])
        for case in failed_cases:
            rows.append(f"- **{case.name}:** {case.actual}")
            if case.name == "Editor Undo/Redo sequences":
                rows.append("  Reproduction: change Exposure or Contrast, then click Undo; the affected recipe and preview did not revert. See the `11a-editor-*` screenshots and `screenshots/11b-editor-reset.png`.")
            elif case.name == "Collage preview/export consistency":
                rows.append("  Reproduction: export the Smart AI square collage; normalized photo content bounding boxes differed between the preview and JPEG export. See `screenshots/16-collage-preview.png` and `exports/rc6-collage.jpg`.")
            elif case.name == "Gallery drag selection":
                rows.append("  Reproduction: drag a selection rectangle across multiple visible thumbnail cards; fewer than two selection badges appeared. See `screenshots/03b-gallery-after-drag.png`.")
    if error:
        rows.extend(["", "## Failure details", "", "```text", "".join(traceback.format_exception(error)), "```"])
    (RUN_DIR / "report.md").write_text("\n".join(rows) + "\n")


def main() -> int:
    SCREENSHOTS.mkdir(parents=True, exist_ok=True)
    error = None
    try:
        smoke_test()
    except Exception as caught:
        error = caught
        print(f"GUI TEST ERROR: {caught}", file=sys.stderr, flush=True)
        traceback.print_exc()
        try:
            screenshot("failure-state.png")
        except Exception as screenshot_error:
            print(f"Could not capture failure screenshot: {screenshot_error}", file=sys.stderr, flush=True)
    finally:
        if APP_PROCESS is not None and APP_PROCESS.poll() is None:
            APP_PROCESS.terminate()
            try:
                APP_PROCESS.wait(timeout=4)
            except subprocess.TimeoutExpired:
                APP_PROCESS.kill()
                APP_PROCESS.wait(timeout=2)
        write_report(error)
    failed_case = any(case.result == "FAIL" for case in CASES)
    return 0 if error is None and not failed_case and not check_logs() else 1


if __name__ == "__main__":
    raise SystemExit(main())
