"""Front-surface interaction checks: the gallery, viewer, editor and date
overview, judged from PICASA_TRACE route lines, the shared zoom slider's
range, value and tooltip (AT-SPI) and screenshots. Run through run.sh; it
prints PASS/FAIL per check and a SUMMARY line, and exits 1 on any failure."""
import sys, os, time, traceback
sys.path.insert(0, os.path.dirname(__file__))
from drv import *
from PIL import Image, ImageChops

results = []
def check(name, passed, evidence):
    results.append((name, bool(passed), evidence))
    print(f"{'PASS' if passed else 'FAIL'}: {name} — {evidence}", flush=True)

def slider_name():
    s = zoom_slider()
    return s.name if s else None

def zoom_ui(since, needle):
    return [l for l in log_lines(since, "PIC_ZOOM_UI") if needle in l]

def dump_names():
    with open(S / "names.txt", "w") as f:
        for n in nodes():
            try:
                if showing(n) and n.name:
                    f.write(f"{n.getRoleName()} {n.name!r}\n")
            except Exception:
                pass

def differs(a, b):
    return ImageChops.difference(Image.open(a).convert("RGB"), Image.open(b).convert("RGB")).getbbox() is not None

proc, win = start()
try:
    key("Escape")
    # All Photos (the date browser lives there).
    def pick_all_photos():
        label = find("label", "All Photos")
        row = label
        while row is not None and row.getRoleName() != "list item":
            row = row.parent
        if row is not None:
            return row.parent.querySelection().selectChild(row.getIndexInParent())
    wait(pick_all_photos, "All Photos row")
    time.sleep(2)

    # 1. Gallery: the slider changes the thumbnail size.
    value, lo, hi = slider()
    mark = log_len()
    target = value - 2 if value >= 2 else value + 2
    slider(target)
    applied = wait(lambda: log_lines(mark, "PIC_ZOOM apply old_width="), "gallery zoom applied", 8)
    lines = zoom_ui(mark, "event=value_changed")
    check("Gallery zoom slider resizes thumbnails",
          applied and hi == 7 and any("lightbox_visible=false edit_visible=false" in l for l in lines),
          f"range 0..{hi}; {applied[0].split(' mode=')[0]}; trace: {lines[-1] if lines else 'none'}")

    # 2. Viewer.
    key("Home")
    mark = log_len()
    key("space")
    opened = wait(lambda: log_lines(mark, "PIC_LIGHTBOX_LAYOUT stage=opened"), "viewer opened", 10)
    time.sleep(1)
    _, _, hi = slider()
    check("Space opens the viewer and the slider switches to photo zoom",
          opened and hi == 100 and "Photo zoom" in (slider_name() or ""),
          f"range 0..{hi}; slider name={slider_name()!r}")
    # Step to portrait.jpg (600x900): its Fit is below 100%, so 1:1 differs from Fit.
    for _ in range(20):
        if find("label", "portrait.jpg"):
            break
        key("Right")
        time.sleep(0.4)
    check("Viewer: arrow keys step to the portrait photo", find("label", "portrait.jpg") is not None, "InfoBar shows portrait.jpg")
    time.sleep(1)
    mark = log_len()
    slider(25)
    applied = wait(lambda: log_lines(mark, "PIC_ZOOM_UI frame_apply slider=25"), "viewer zoom applied", 8)
    check("Viewer: zoom slider zooms the photo", applied, f"trace: {applied[0] if applied else 'none'}")
    mark = log_len()
    key("space")
    on = wait(lambda: zoom_ui(mark, "one_to_one enabled=true"), "1:1 on", 6)
    at_native = wait(lambda: abs(slider()[0] - 50) < 0.5, "slider at 100%", 6)
    check("Viewer: Space selects 1:1", on and at_native and "lightbox_visible=true edit_visible=false" in on[-1],
          f"slider={slider()[0]:.1f} (50 = 100%); {on[-1] if on else 'none'}")
    mark = log_len()
    press("toggle button", "1:1")
    off = wait(lambda: zoom_ui(mark, "one_to_one enabled=false"), "1:1 off", 6)
    at_fit = wait(lambda: slider()[0] < 0.5, "slider at Fit", 6)
    check("Viewer: 1:1 button returns to Fit", off and at_fit and "lightbox_visible=true" in off[-1],
          f"slider={slider()[0]:.1f} (0 = Fit); {off[-1] if off else 'none'}")
    mark = log_len()
    key("Escape")
    hidden = wait(lambda: log_lines(mark, "PIC_LIGHTBOX_LAYOUT stage=hidden"), "viewer hidden", 8)
    time.sleep(1)
    _, _, hi = slider()
    check("Viewer: Escape closes it and the slider returns to thumbnails", hidden and hi == 7, f"range 0..{hi}; slider name={slider_name()!r}")

    # 3. Editor.
    mark = log_len()
    press("button", "Open or close photo editor")
    wait(lambda: "Edit zoom" in (slider_name() or ""), "editor slider", 20)
    _, _, hi = slider()
    check("Edit button opens the editor and the slider switches to edit zoom", hi == 100, f"range 0..{hi}; slider name={slider_name()!r}")
    time.sleep(1.5)
    mark = log_len()
    slider(30)
    lines = wait(lambda: zoom_ui(mark, "event=value_changed value=30.000"), "editor value_changed", 6)
    check("Editor: zoom slider routes to the editor", lines and "lightbox_visible=false edit_visible=true" in lines[-1], lines[-1] if lines else "none")
    mark = log_len()
    key("space")
    on = wait(lambda: zoom_ui(mark, "one_to_one enabled=true"), "editor 1:1", 6)
    check("Editor: Space toggles 1:1 in the editor", on and "lightbox_visible=false edit_visible=true" in on[-1], on[-1] if on else "none")
    key("space")
    time.sleep(0.5)
    key("Escape")
    wait(lambda: "Thumbnail size" in (slider_name() or ""), "editor closed", 15)
    _, _, hi = slider()
    check("Editor: Escape leaves the editor", hi == 7, f"range 0..{hi}; slider name={slider_name()!r}")

    # 4. Date browser.
    press("toggle button", "Browse photos by year and month")
    year = wait(lambda: find("button", contains="Unknown Date") or find("button", contains="20"), "overview tile", 10)
    time.sleep(1)
    overview1 = shot("overview-before.png")
    check("Date browser opens its year overview", year is not None, f"tile={year.name!r}")
    mark = log_len()
    value, _, hi = slider()
    slider(value + 1 if value < 7 else value - 1)
    time.sleep(1.5)
    overview2 = shot("overview-after-zoom.png")
    lines = zoom_ui(mark, "event=value_changed")
    check("Date overview: slider resizes its tiles",
          differs(overview1, overview2) and hi == 7 and lines and "lightbox_visible=false edit_visible=false" in lines[-1],
          f"overview changed={differs(overview1, overview2)}; trace: {lines[-1] if lines else 'none'}")
    def tile_order():
        return [n.name for n in nodes() if showing(n) and n.getRoleName() == "button"
                and (n.name or "").endswith("photos") and n.name != "Sort photos"]
    direction_before = setting("photo-sort-direction") or "descending (default)"
    press("button", "Sort photos")
    time.sleep(1)
    menu = find("radio button", "Date Taken") or find("radio button", "Date taken") or find("label", "Group by")
    shot("overview-after-sort.png")
    sort_menu_shown = menu is not None
    def icon(path):
        return Image.open(path).convert("RGB").crop((1320, 8, 1356, 48))
    icon_flipped = ImageChops.difference(icon(S / "shots/overview-after-zoom.png"),
                                         icon(S / "shots/overview-after-sort.png")).getbbox() is not None
    key("Escape")
    time.sleep(1)
    gone = find("button", contains="Unknown Date") is None and find("button", year.name) is None
    check("Date overview: Escape steps back out of the overview", gone, f"overview tile still showing={not gone}")
    shot("after-escape.png")

    problems = [l for l in log_lines(0) if any(k in l for k in ("panicked at", "Gtk-CRITICAL", "GLib-GObject-CRITICAL"))]
    check("No panic or GTK critical in the app log", not problems, problems[:3] or "none")
except Exception as error:
    traceback.print_exc()
    dump_names()
    shot("failure.png")
    check("Script completed", False, f"{type(error).__name__}: {error}")
finally:
    proc.terminate()
    try:
        proc.wait(10)
    except Exception:
        proc.kill()
if "direction_before" in globals():
    after = setting("photo-sort-direction")
    check("Date overview: Sort flips the direction instead of opening the menu",
          not sort_menu_shown and icon_flipped,
          f"sort icon changed={icon_flipped}; sort menu shown={sort_menu_shown}; "
          f"saved photo-sort-direction before={direction_before}, after exit={after}")
print(f"SUMMARY {sum(p for _, p, _ in results)}/{len(results)} passed")
sys.exit(0 if results and all(p for _, p, _ in results) else 1)
