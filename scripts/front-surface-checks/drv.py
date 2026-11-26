"""Driver for the front-surface interaction checks (see run.sh).

Launches PIC with `cargo run` on the private display run.sh provides, against
a copy of a GUI-harness library (never the real one), and offers small
AT-SPI, xdotool, screenshot and log helpers to checks.py.
"""
import os, shutil, subprocess, time, sqlite3
from pathlib import Path
import pyatspi

HERE = Path(__file__).resolve().parent
HOME_REPO = HERE.parent.parent
# The tree to run (another worktree, e.g. main, for comparison) and the
# target dir to build into (shared, so a second tree only rebuilds the crate).
REPO = Path(os.environ.get("PIC_CHECK_REPO", HOME_REPO))
TARGET = os.environ.get("PIC_CHECK_TARGET", str(HOME_REPO / "target"))
S = Path(os.environ["PIC_CHECK_OUT"])
def _library_run():
    """A GUI-harness run whose temporary library is copied for each check run:
    PIC_CHECK_LIBRARY, else the newest viewer/editing run under reports/."""
    if os.environ.get("PIC_CHECK_LIBRARY"):
        return Path(os.environ["PIC_CHECK_LIBRARY"])
    # Harness runs are named by UTC time (20261123T161240Z-326056), so the
    # newest sorts last; other folders under reports/gui-tests are ignored.
    runs = sorted(db.parents[3] for db in (HOME_REPO / "reports/gui-tests").glob("2*/xdg/data/pic-rs/library.db"))
    if not runs:
        raise SystemExit("No GUI-harness library found: run PIC_GUI_SUITE=viewer scripts/test-gui.sh first, "
                         "or set PIC_CHECK_LIBRARY to a reports/gui-tests/<run> directory")
    return runs[-1]

RUN = _library_run()
XDG = S / "xdg"
LOG = S / "app.log"
SHOTS = S / "shots"

def x(*args, check=True):
    return subprocess.run(["xdotool", *args], check=check, capture_output=True, text=True).stdout.strip()

def shot(name):
    SHOTS.mkdir(exist_ok=True)
    path = SHOTS / name
    subprocess.run(["import", "-window", "root", str(path)], check=True)
    return path

def wait(pred, what, timeout=10):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try:
            v = pred()
        except Exception:
            v = None
        if v:
            return v
        time.sleep(0.2)
    raise TimeoutError(what)

def nodes():
    pending = [pyatspi.Registry.getDesktop(0)]
    while pending:
        n = pending.pop()
        yield n
        try:
            pending.extend(n.getChildAtIndex(i) for i in reversed(range(n.childCount)))
        except Exception:
            pass

def showing(n):
    try:
        return n.getState().contains(pyatspi.STATE_SHOWING)
    except Exception:
        return False

def find(role, name=None, contains=None):
    for n in nodes():
        try:
            if n.getRoleName() != role or not showing(n):
                continue
            nm = n.name or ""
            if name is not None and nm != name:
                continue
            if contains is not None and contains not in nm:
                continue
            return n
        except Exception:
            continue
    return None

def press(role, name=None, contains=None):
    n = wait(lambda: find(role, name, contains), f"{role} {name or contains}")
    a = n.queryAction()
    for i in range(a.nActions):
        if a.getName(i) in {"click", "press", "toggle"}:
            a.doAction(i)
            return n
    raise RuntimeError(f"no action on {role} {name or contains}")

ZOOM_NAMES = ("Thumbnail size", "Photo zoom", "Edit zoom")

def zoom_slider():
    for n in nodes():
        try:
            if n.getRoleName() == "slider" and showing(n) and (n.name or "").startswith(ZOOM_NAMES):
                return n
        except Exception:
            continue
    return None

def slider(value=None):
    s = wait(zoom_slider, "zoom slider").queryValue()
    if value is not None:
        s.currentValue = value
    return s.currentValue, s.minimumValue, s.maximumValue

def setting(key):
    with sqlite3.connect(XDG / "data/pic-rs/library.db") as c:
        r = c.execute("SELECT value FROM settings WHERE key=?", (key,)).fetchone()
    return r[0] if r else None

def log_lines(since=0, needle=""):
    text = LOG.read_text(errors="replace").splitlines()[since:]
    return [l for l in text if needle in l]

def log_len():
    return len(LOG.read_text(errors="replace").splitlines()) if LOG.exists() else 0

def point_registry_at_copy(xdg, original):
    """The copied `config/pic-rs/libraries.json` still names the original
    run's databases by absolute path, so PIC would open (and write) the
    harness library instead of the copy. Point every library that lived
    under `original` at the same file under `xdg`."""
    import json
    registry = xdg / "config/pic-rs/libraries.json"
    if not registry.exists():
        return
    data = json.loads(registry.read_text())
    original = str(original)
    for library in data.get("libraries", []):
        path = library.get("path", "")
        if path == original or path.startswith(original + "/"):
            library["path"] = str(xdg) + path[len(original):]
    registry.write_text(json.dumps(data, indent=2))

def start(prepare=None, fresh=True, size=("1600", "1000")):
    """Copy the harness library, let `prepare(XDG)` adjust the copy (e.g.
    register a second library), then launch PIC and size its window. With
    `fresh=False` PIC restarts on the library a previous start left; `size`
    is the window size it is given."""
    if fresh:
        if XDG.exists():
            shutil.rmtree(XDG)
        shutil.copytree(RUN / "xdg", XDG)
        point_registry_at_copy(XDG, RUN / "xdg")
        if prepare is not None:
            prepare(XDG)
    env = os.environ.copy()
    env.update({"HOME": str(XDG / "home"), "XDG_CONFIG_HOME": str(XDG / "config"),
                "XDG_DATA_HOME": str(XDG / "data"), "XDG_CACHE_HOME": str(XDG / "cache"),
                "PICASA_TRACE": "1",
                # cargo/rustup live under the real home; only the app's data is isolated.
                "RUSTUP_HOME": os.path.join(__import__("pwd").getpwuid(os.getuid()).pw_dir, ".rustup"),
                "CARGO_TARGET_DIR": TARGET,
                "CARGO_HOME": os.path.join(__import__("pwd").getpwuid(os.getuid()).pw_dir, ".cargo")})
    log = LOG.open("w")
    proc = subprocess.Popen(["cargo", "run", "--locked", "--quiet"], cwd=REPO, env=env,
                            stdout=log, stderr=subprocess.STDOUT)
    win = None
    end = time.monotonic() + 900
    while win is None and time.monotonic() < end:
        if proc.poll() is not None:
            raise SystemExit(f"cargo run exited early ({proc.returncode}); see {LOG}")
        win = next((w for w in x("search", "--onlyvisible", "--name", ".*", check=False).splitlines()
                    if "PIC" in x("getwindowname", w, check=False)), None)
        if win is None:
            time.sleep(0.5)
    if win is None:
        raise TimeoutError("PIC window")
    x("windowsize", "--sync", win, *size)
    x("windowmove", "--sync", win, "0", "0")
    x("windowfocus", "--sync", win)
    time.sleep(3)
    return proc, win

def key(*keys):
    for k in keys:
        x("key", k)
        time.sleep(0.3)
