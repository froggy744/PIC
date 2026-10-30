"""X11 pointer-delivery smoke test for PIC's shared sidebar divider geometry.

This tests a real XTest-driven mouse gesture, unlike the allocation-only test.
The widget is a small GTK reproduction, not a launch of the full PIC app.
"""
import subprocess
import threading
import time

import gi
gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Gtk, Adw, GLib

Adw.init()
Gtk.Settings.get_default().set_property("gtk-enable-animations", False)
window = Gtk.Window(title="PIC X11 pointer divider test")
window.set_default_size(900, 600)
split = Adw.OverlaySplitView()
split.set_min_sidebar_width(48.0)
split.set_max_sidebar_width(120.0)
split.set_sidebar_width_fraction(120.0 / 900.0)

shell = Gtk.Overlay()
shell.set_size_request(120, -1)
column = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
column.append(Gtk.Label(label="PIC"))
scroll = Gtk.ScrolledWindow()
rail = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
for name in ("Library", "Photos", "Recent", "Albums", "Folders"):
    rail.append(Gtk.Button(label=name))
scroll.set_child(rail)
column.append(scroll)
shell.set_child(column)

handle = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
handle.set_size_request(12, -1)
handle.set_halign(Gtk.Align.END)
handle.set_vexpand(True)
handle.set_cursor_from_name("col-resize")
shell.add_overlay(handle)

drag = Gtk.GestureDrag()
drag.set_button(1)
drag.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
drag.set_exclusive(True)
seen = {"begin": False, "updates": 0, "smallest": 999.0, "initial": 120.0}

def on_begin(_gesture, _x, _y):
    seen["begin"] = True
    seen["initial"] = float(shell.get_width())

def on_update(_gesture, dx, _dy):
    # Exercise the same minimum, maximum and split-width properties as PIC.
    width = max(48.0, min(300.0, seen["initial"] + dx))
    shell.set_size_request(round(width), -1)
    split.set_max_sidebar_width(width)
    split.set_sidebar_width_fraction(width / max(1.0, split.get_width()))
    seen["updates"] += 1
    seen["smallest"] = min(seen["smallest"], width)

drag.connect("drag-begin", on_begin)
drag.connect("drag-update", on_update)
handle.add_controller(drag)
split.set_sidebar(shell)
split.set_content(Gtk.Box())
window.set_child(split)
window.present()

loop = GLib.MainLoop()
failures = []

def send_pointer_drag():
    try:
        time.sleep(0.75)
        ids = subprocess.check_output(
            ["xdotool", "search", "--name", "PIC X11 pointer divider test"],
            text=True,
        ).splitlines()
        assert ids, "no X11 test window"
        window_id = ids[-1]

        def move(x):
            subprocess.run(
                ["xdotool", "mousemove", "--window", window_id, str(x), "240"],
                check=True,
            )

        move(114)  # Inside the 12px handle of a 120px sidebar.
        time.sleep(0.1)
        subprocess.run(["xdotool", "mousedown", "1"], check=True)
        for x in (105, 90, 75, 60, 52):
            move(x)
            time.sleep(0.14)
        subprocess.run(["xdotool", "mouseup", "1"], check=True)
        time.sleep(0.5)
    except Exception as exc:
        failures.append(str(exc))
    finally:
        GLib.idle_add(loop.quit)

threading.Thread(target=send_pointer_drag, daemon=True).start()
GLib.timeout_add_seconds(
    8, lambda: (failures.append("timed out"), loop.quit(), False)[2]
)
loop.run()
print("X11 pointer drag:", seen, "allocated:", shell.get_width(), flush=True)
window.destroy()

assert not failures, failures
assert seen["begin"], "outside divider or GTK did not receive press"
assert seen["updates"] >= 2, "divider received no drag updates"
assert seen["smallest"] <= 80, "real X11 pointer movement failed to narrow sidebar"
