"""Browse scope smoke checks (candidate 14, docs/architecture/STATUS.md).

Drives PIC (started with `cargo run` on run.sh's private display, against a
copy of a GUI-harness library) through the toolbar rating picker, the
metadata panel's photo count, the sidebar Ratings destination, two Smart
Albums, search and a library switch. Each toolbar rating change refreshes
the grid, which logs `PIC_SCAN_UI gallery_replace ... photos=N`
(PICASA_TRACE); the checks compare N with the count the browse rules give on
the library copy (sqlite). Views are opened by restarting PIC on a saved
`last-view`, so no sidebar row has to be found.

- Rating picker: All Photos under 3 stars shows the 3-star photos, and the
  setting is saved; Clear shows every photo again.
- Metadata panel: its "N photos" count matches the grid under the rating.
- Ratings destination with the toolbar on Unrated shows every rated photo
  (round-three bug 1).
- Smart Albums: rules saved as "0" and as "unrated" (older spellings) both
  show the unrated photos.
- Search: a file-name search shows the matching photos.
- Library switch: the second library opens without its stale saved rating.

Run through run.sh; prints PASS/FAIL per check and a SUMMARY line.
First run 2026-11-26: 9/9 passed on a 15-photo harness library, after
adjusting the selectors (GTK 4 radio buttons by keyboard focus, the
"Filters" toggle, the unnamed search entry, the header library switcher)."""
import sys, os, re, json, time, sqlite3, traceback
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "front-surface-checks"))
from drv import *

results = []
ALBUM_ZERO, ALBUM_UNRATED = 9001, 9002


def check(name, passed, evidence):
    results.append((name, bool(passed), evidence))
    print(f"{'PASS' if passed else 'FAIL'}: {name} — {evidence}", flush=True)


def library_a():
    return XDG / "data/pic-rs/library.db"


def library_b():
    return XDG / "data/pic-rs/second.db"


def count(sql, db=None):
    with sqlite3.connect(db or library_a()) as c:
        return c.execute(f"SELECT COUNT(*) FROM photos p WHERE p.trashed=0 AND ({sql})").fetchone()[0]


def set_view(db, view):
    with sqlite3.connect(db) as c:
        c.execute("DELETE FROM settings WHERE key IN ('last-activated-photo-id', 'last-view-photo-id', "
                  "'last-view-scroll', 'photo-metadata-filters')")
        c.execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('last-view', ?)", (view,))


def prepare(xdg):
    """Library A: ratings 0-5 spread over the photos, two Smart Albums for
    the unrated photos (rules saved as "0" and as "unrated"). Library B: a
    copy with A's first 6 photos and a stale saved 5-star rating."""
    a, b = xdg / "data/pic-rs/library.db", xdg / "data/pic-rs/second.db"
    with sqlite3.connect(a) as c:
        c.execute("UPDATE photos SET rating = id % 6")
        columns = [row[1] for row in c.execute("PRAGMA table_info(albums)")]
        if "rules" not in columns:
            c.execute("ALTER TABLE albums ADD COLUMN rules TEXT")
        for album_id, name, rating in ((ALBUM_ZERO, "Smoke zero", "0"), (ALBUM_UNRATED, "Smoke unrated", "unrated")):
            c.execute("INSERT OR REPLACE INTO albums(id, name, rules) VALUES (?, ?, ?)",
                      (album_id, name, json.dumps({"categories": {}, "rating": rating})))
    set_view(a, "all")
    with sqlite3.connect(a) as source, sqlite3.connect(b) as copy:
        source.backup(copy)
    with sqlite3.connect(b) as c:
        c.execute("DELETE FROM photos WHERE id NOT IN (SELECT id FROM photos ORDER BY id LIMIT 6)")
        c.execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('photo-rating-filter', '5')")
    registry = {"version": 1, "current": "aaaaaaaaaaaaaaaa", "libraries": [
        {"id": "aaaaaaaaaaaaaaaa", "name": "First", "description": "", "path": str(a)},
        {"id": "bbbbbbbbbbbbbbbb", "name": "Second", "description": "", "path": str(b)},
    ]}
    (xdg / "config/pic-rs").mkdir(parents=True, exist_ok=True)
    (xdg / "config/pic-rs/libraries.json").write_text(json.dumps(registry))


def grid_count(since, timeout=15):
    """The photo count of the newest grid refresh logged after line `since`."""
    def latest():
        hits = [re.search(r"gallery_replace .* photos=(\d+)", l) for l in log_lines(since, "gallery_replace")]
        hits = [int(m.group(1)) for m in hits if m]
        return hits[-1] if hits else None
    try:
        wait(lambda: latest() is not None, "grid refresh", timeout)
        time.sleep(1)  # a second refresh may follow the first
    except TimeoutError:
        return None
    return latest()


def rating_button():
    """The toolbar rating picker, named by its tooltip: "Filter by rating
    (clear)", "Rating filter: …" or "Filter: …" (Files choices)."""
    for n in nodes():
        try:
            name = n.name or ""
            if n.getRoleName() == "toggle button" and showing(n) and (
                    name == "Filter by rating (clear)" or name.startswith(("Rating filter: ", "Filter: "))):
                return n
        except Exception:
            continue
    return None


def select_row(label_text):
    """Select the list row holding `label_text` (sidebar, Settings)."""
    def pick():
        row = find("label", label_text)
        while row is not None and row.getRoleName() != "list item":
            row = row.parent
        if row is not None:
            return row.parent.querySelection().selectChild(row.getIndexInParent())
    wait(pick, f"{label_text} row")


def focused_name(role):
    for n in nodes():
        try:
            if n.getRoleName() == role and n.getState().contains(pyatspi.STATE_FOCUSED):
                return n.name
        except Exception:
            continue
    return None


def activate_by_focus(role, name, steps=20):
    """GTK 4 radio and check buttons expose no AT-SPI action (as
    scripts/gui_test.py notes): move keyboard focus to the button, then
    press Space."""
    for direction, count in (("Down", steps), ("Up", 2 * steps)):
        for _ in range(count + 1):
            if focused_name(role) == name:
                key("space")
                return
            key(direction)
    raise TimeoutError(f"could not focus {role} {name!r}")


def pick_rating(label):
    """Toolbar rating picker: open it, choose `label`, close it. Returns the
    log position before the choice."""
    since = log_len()
    action = wait(rating_button, "rating picker").queryAction()
    action.doAction(0)
    wait(lambda: find("radio button", "Clear"), "rating popover")
    activate_by_focus("radio button", label)
    time.sleep(0.5)
    key("Escape")
    return since


def quit_app(proc):
    x("windowactivate", "--sync", win, check=False)
    key("alt+F4")
    press("button", "Close Picasa")
    proc.wait(20)


def restart(view):
    global win
    set_view(library_a(), view)
    proc, win = start(fresh=False)
    key("Escape")
    time.sleep(2)
    return proc


def photo_count_label():
    for n in nodes():
        try:
            if n.getRoleName() == "label" and showing(n) and re.fullmatch(r"\d+ photos", n.name or ""):
                return int(n.name.split()[0])
        except Exception:
            continue
    return None


def problems():
    return [l for l in log_lines(0) if any(k in l for k in ("panicked at", "Gtk-CRITICAL", "GLib-GObject-CRITICAL"))]


proc, win = start(prepare)
seen_problems = []
try:
    key("Escape")
    time.sleep(2)

    # Rating picker in All Photos.
    since = pick_rating("★★★ 3")
    shown, expected = grid_count(since), count("p.rating=3")
    check("Rating picker narrows All Photos to 3 stars",
          shown == expected and setting("photo-rating-filter") == "3",
          f"grid {shown}, expected {expected}, setting {setting('photo-rating-filter')}")
    shot("01-three-stars.png")

    # Metadata panel count under the same rating.
    try:
        press("toggle button", "Filters")
        panel = wait(photo_count_label, "metadata panel photo count")
    except Exception as error:
        panel = f"not found ({type(error).__name__})"
    check("Metadata panel photo count matches the grid", panel == expected,
          f"panel {panel}, expected {expected}")
    shot("02-metadata-panel.png")
    key("Escape")

    since = pick_rating("Clear")
    shown, expected = grid_count(since), count("1")
    check("Clear shows every photo again", shown == expected,
          f"grid {shown}, expected {expected}, setting {setting('photo-rating-filter')}")

    # Search by a file name.
    with sqlite3.connect(library_a()) as c:
        path = c.execute("SELECT path FROM photos WHERE trashed=0 ORDER BY id LIMIT 1").fetchone()[0]
    term = os.path.splitext(os.path.basename(path))[0]
    since = log_len()
    # GTK 4 gives the header's search entry no accessible name: take the
    # showing text-entry node (the only one in the gallery view).
    entry = wait(lambda: next((n for n in nodes() if n.getRoleName() in {"entry", "text", "search box"}
                               and showing(n)), None), "search entry")
    try:
        entry.queryComponent().grabFocus()
    except Exception:
        # GTK 4 may refuse AT-SPI focus; typing in the gallery starts a
        # search instead (type-to-search, window/search.rs).
        x("windowactivate", "--sync", win, check=False)
    x("type", "--delay", "40", term)
    shown = grid_count(since)
    expected = count(f"p.path LIKE '%{term}%' OR substr(p.taken_at,1,10) LIKE '%{term}%' OR EXISTS "
                     f"(SELECT 1 FROM album_photos ap JOIN albums a ON a.id=ap.album_id "
                     f"WHERE ap.photo_id=p.id AND a.name LIKE '%{term}%')")
    check(f"Search for {term!r} shows the matching photos", shown == expected, f"grid {shown}, expected {expected}")
    shot("03-search.png")
    x("key", "ctrl+a", "BackSpace")
    key("Escape")
    time.sleep(2)

    # Sidebar Ratings with the toolbar on Unrated (round-three bug 1). The
    # picker is insensitive in Ratings, so choose Unrated first, then go there.
    pick_rating("Unrated")
    since = log_len()
    select_row("Ratings")
    shown, expected = grid_count(since), count("p.rating BETWEEN 1 AND 5")
    check("Ratings ignores the toolbar's Unrated", shown == expected,
          f"grid {shown}, expected {expected}, setting {setting('photo-rating-filter')}")
    shot("04-ratings-unrated.png")
    seen_problems += problems()
    quit_app(proc)

    # Smart Albums saved with the older rating spellings.
    for album_id, spelling in ((ALBUM_ZERO, "0"), (ALBUM_UNRATED, "unrated")):
        proc = restart(f"album:{album_id}")
        since = pick_rating("★ 1")
        narrowed = grid_count(since)
        since = pick_rating("Clear")
        shown, expected = grid_count(since), count("p.rating=0")
        check(f"Smart Album saved as {spelling!r} shows the unrated photos",
              shown == expected and narrowed == 0,
              f"grid {shown}, expected {expected}; under 1 star {narrowed}, expected 0")
        shot(f"05-smart-album-{spelling}.png")
        seen_problems += problems()
        quit_app(proc)

    # Library switch: Second must not open under its stale saved 5 stars.
    proc = restart("all")
    pick_rating("★★★ 3")
    # The header's library switcher: one button per library, named after it
    # (Settings > Libraries lists the others below the fold).
    press("button", "Libraries")
    second = wait(lambda: next((n for n in nodes() if n.getRoleName() == "button" and showing(n)
                                and (n.name or "").startswith("Second")), None), "Second in the switcher")
    act = second.queryAction()
    act.doAction(0)
    wait(lambda: "Second" in x("getwindowname", win, check=False), "switched to Second", 15)
    key("Escape")
    time.sleep(3)
    with sqlite3.connect(library_b()) as c:
        saved = c.execute("SELECT value FROM settings WHERE key='photo-rating-filter'").fetchone()
    # Clear on an already clear rating refreshes nothing: narrow first, so
    # Clear rebuilds the grid of the second library.
    pick_rating("★★★ 3")
    since = pick_rating("Clear")
    shown, expected = grid_count(since), count("1", library_b())
    check("A library switch drops the new library's stale rating",
          saved and saved[0] == "all" and shown == expected,
          f"Second's setting {saved}, grid after Clear {shown}, expected {expected}")
    shot("06-second-library.png")
    seen_problems += problems()
    quit_app(proc)

    check("No panic or GTK critical in the app log", not seen_problems, seen_problems[:3] or "none")
except Exception as error:
    traceback.print_exc()
    shot("failure.png")
    check("Script completed", False, f"{type(error).__name__}: {error}")
finally:
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(10)
        except Exception:
            proc.kill()
print(f"SUMMARY {sum(p for _, p, _ in results)}/{len(results)} passed")
sys.exit(0 if results and all(p for _, p, _ in results) else 1)
