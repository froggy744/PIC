"""Library switch and Exit checks (window session, candidate 11 follow-up).

Two libraries (copies of a GUI-harness library), driven through the real UI:
scroll library A, switch to B in Settings > Libraries, switch back, scroll one
more wheel click, then quit through the Exit confirmation. The scroll each
step saves is read back from A's database, so the checks need no screenshots:

- leaving A for B saves A's viewport (the leave snapshot);
- coming back restores it: after one more 160 px wheel click, Exit saves
  the restored scroll + 160 (without the restore, `main` saved 268);
- Exit through the confirmation saves at all;
- a restart comes back there too: quitting again at once saves the same.

Run through run.sh; prints PASS/FAIL per check and a SUMMARY line."""
import sys, os, json, time, sqlite3, traceback
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "front-surface-checks"))
from drv import *

WHEEL_PX = 160  # GALLERY_WHEEL_STEP_PX
# The view both libraries open on: "all" (All Photos) by default, or e.g.
# "folder:1" (the harness sample folder) for the Folder surface.
VIEW = os.environ.get("PIC_CHECK_VIEW", "all")
results = []


def check(name, passed, evidence):
    results.append((name, bool(passed), evidence))
    print(f"{'PASS' if passed else 'FAIL'}: {name} — {evidence}", flush=True)


def library_a():
    return XDG / "data/pic-rs/library.db"


def library_b():
    return XDG / "data/pic-rs/second.db"


def prepare(xdg):
    """Library A: the largest thumbnails (300 px) so the 17 photos scroll, All Photos, no
    saved viewport and no last-opened photo (that would win over the scroll
    on restore). Library B: A's first 8 photos. Both registered, A current."""
    a, b = xdg / "data/pic-rs/library.db", xdg / "data/pic-rs/second.db"
    with sqlite3.connect(a) as c:
        c.execute("DELETE FROM settings WHERE key IN ('last-activated-photo-id', "
                  "'last-view-photo-id', 'last-view-scroll')")
        for key, value in (("last-view", VIEW), ("grid-thumbnail-size", "300")):
            c.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?, ?)", (key, value))
    with sqlite3.connect(a) as source, sqlite3.connect(b) as copy:
        source.backup(copy)
    # B must hold other photos than A: with the same photos the gallery keeps
    # its widgets on a switch and A's scroll survives without any restore.
    # Its first 8 photos fit on one screen, so B always shows from the top.
    with sqlite3.connect(b) as c:
        c.execute("DELETE FROM photos WHERE id NOT IN (SELECT id FROM photos ORDER BY id LIMIT 8)")
    registry = {"version": 1, "current": "aaaaaaaaaaaaaaaa", "libraries": [
        {"id": "aaaaaaaaaaaaaaaa", "name": "First", "description": "", "path": str(a)},
        {"id": "bbbbbbbbbbbbbbbb", "name": "Second", "description": "", "path": str(b)},
    ]}
    (xdg / "config/pic-rs").mkdir(parents=True, exist_ok=True)
    (xdg / "config/pic-rs/libraries.json").write_text(json.dumps(registry))


def saved(db, key):
    with sqlite3.connect(db) as c:
        row = c.execute("SELECT value FROM settings WHERE key=?", (key,)).fetchone()
    return row[0] if row else None


def saved_scroll(db):
    value = saved(db, "last-view-scroll")
    return float(value) if value is not None else None


def sensitive(n):
    try:
        return n.getState().contains(pyatspi.STATE_SENSITIVE)
    except Exception:
        return False


def press_enabled(role, name):
    def pick():
        for n in nodes():
            try:
                if n.getRoleName() == role and (n.name or "") == name and showing(n) and sensitive(n):
                    return n
            except Exception:
                continue
        return None
    n = wait(pick, f"enabled {role} {name}")
    a = n.queryAction()
    for i in range(a.nActions):
        if a.getName(i) in {"click", "press"}:
            a.doAction(i)
            return n
    raise RuntimeError(f"no action on {role} {name}")


def title():
    return x("getwindowname", win, check=False)


def pick_list_row(label_text):
    def pick():
        label = find("label", label_text)
        row = label
        while row is not None and row.getRoleName() != "list item":
            row = row.parent
        if row is not None:
            return row.parent.querySelection().selectChild(row.getIndexInParent())
    wait(pick, f"{label_text} row")


def switch_to(name):
    """Settings > Libraries > Open on the one library that is not current."""
    press("button", "Settings")
    pick_list_row("Libraries")
    time.sleep(1)
    press_enabled("button", "Open")
    wait(lambda: f"PIC — {name}" in title(), f"switched to {name}", 15)
    key("Escape")  # close Settings
    time.sleep(3)  # gallery build, then the viewport restore


def scroll_gallery(clicks):
    x("mousemove", "900", "500")
    for _ in range(clicks):
        x("click", "5")
        time.sleep(0.5)
    time.sleep(1)


# A short window gives the small gallery room to scroll below the restored
# position; the full width keeps Settings out of the InfoBar's overflow menu.
# Both starts use it, as a real restart reopens at the size it was closed at.
SIZE = ("1600", "650")
proc, win = start(prepare, size=SIZE)
try:
    key("Escape")
    time.sleep(2)

    scroll_gallery(2)
    switch_to("Second")
    left_at = saved_scroll(library_a())
    check("Leaving a library saves its viewport", left_at is not None and left_at > WHEEL_PX,
          f"First last-view-scroll={left_at}, anchor={saved(library_a(), 'last-view-photo-id')}")
    shot("second.png")

    switch_to("First")
    shot("first-again.png")
    scroll_gallery(1)

    # Quit through the Exit confirmation; the close handler saves the session.
    x("windowactivate", "--sync", win, check=False)
    key("alt+F4")
    press("button", "Close Picasa")
    proc.wait(20)
    exited_at = saved_scroll(library_a())
    check("Exit through the confirmation saves the viewport", exited_at is not None,
          f"First last-view-scroll after Exit={exited_at}")
    expected = (left_at or 0) + WHEEL_PX
    check("Coming back to a library restores where it was left",
          exited_at is not None and left_at is not None and abs(exited_at - expected) < WHEEL_PX / 2,
          f"left at {left_at}; after return + one wheel click, Exit saved {exited_at} "
          f"(expected about {expected} when restored)")
    problems = [l for l in log_lines(0) if any(k in l for k in ("panicked at", "Gtk-CRITICAL", "GLib-GObject-CRITICAL"))]

    # Restart on the same library and quit at once: startup must restore.
    proc, win = start(fresh=False, size=SIZE)
    key("Escape")
    time.sleep(4)
    key("alt+F4")
    press("button", "Close Picasa")
    proc.wait(20)
    restarted_at = saved_scroll(library_a())
    check("A restart comes back to where the library was left",
          restarted_at is not None and exited_at is not None and abs(restarted_at - exited_at) < WHEEL_PX / 2,
          f"Exit saved {exited_at}; restarting and quitting at once saved {restarted_at}")

    problems += [l for l in log_lines(0) if any(k in l for k in ("panicked at", "Gtk-CRITICAL", "GLib-GObject-CRITICAL"))]
    check("No panic or GTK critical in the app log", not problems, problems[:3] or "none")
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
