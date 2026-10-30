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
split.set_sidebar(Gtk.Box())
split.set_content(Gtk.Box())
split.set_show_sidebar(False)

# PIC's compact rail is a fixed-width dock OUTSIDE the Adw split.
dock = Gtk.Overlay()
dock.add_css_class("compact-sidebar-dock")
dock.set_hexpand(False)
dock.set_vexpand(True)
dock.set_size_request(120, -1)
column = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
column.append(Gtk.Label(label="PIC"))
scroll = Gtk.ScrolledWindow()
scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
scroll.set_min_content_width(1)
rail = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
rail.add_css_class("compact-sidebar")
labels = []
for name in ("Library", "Photos", "Recent", "Albums", "Folders"):
    button = Gtk.Button()
    button.add_css_class("flat")
    button.add_css_class("compact-sidebar-button")
    content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
    content.set_halign(Gtk.Align.CENTER)
    content.append(Gtk.Image.new_from_icon_name("folder-symbolic"))
    label = Gtk.Label(label=name)
    content.append(label)
    labels.append(label)
    button.set_child(content)
    rail.append(button)
details = Gtk.Revealer()
details.set_reveal_child(True)
item = Gtk.Button(label="A very long folder or album name that must not widen the rail")
item.add_css_class("flat")
item.add_css_class("compact-sidebar-option")
item.get_child().set_xalign(0.0)
item.get_child().set_ellipsize(3)
item.get_child().set_width_chars(1)
details.set_child(item)
rail.append(details)
scroll.set_child(rail)
column.append(scroll)
dock.set_child(column)

handle = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
handle.set_size_request(12, -1)
handle.set_halign(Gtk.Align.END)
handle.set_vexpand(True)
handle.set_cursor_from_name("col-resize")
dock.add_overlay(handle)

drag = Gtk.GestureDrag()
drag.set_button(1)
drag.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
drag.set_exclusive(True)
seen = {"begin": False, "updates": 0, "smallest": 999.0, "initial": 120.0}
start_local = [0.0, 0.0]
start_x = [0.0]

def on_begin(_gesture, x, y):
    seen["begin"] = True
    seen["initial"] = float(dock.get_width())
    start_local[:] = [x, y]
    point = handle.translate_coordinates(layout, x, y)
    assert point is not None, "could not translate drag origin into split coordinates"
    start_x[0] = float(point[0])

def on_update(_gesture, dx, dy):
    # When the handle moves, raw dx is no longer a stable displacement.
    # Translate the current pointer into the stationary split-view space,
    # exactly like layout.rs does with widget.compute_point().
    x, y = start_local
    point = handle.translate_coordinates(layout, x + dx, y + dy)
    if point is None:
        return
    stable_delta = float(point[0]) - start_x[0]
    width = max(44.0, min(300.0, seen["initial"] + stable_delta))
    dock.set_size_request(round(width), -1)
    for label in labels:
        label.set_visible(width >= 90.0)
    seen["updates"] += 1
    seen["smallest"] = min(seen["smallest"], width)

drag.connect("drag-begin", on_begin)
drag.connect("drag-update", on_update)
handle.add_controller(drag)
layout = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
layout.append(dock)
split.set_hexpand(True)
layout.append(split)
window.set_child(layout)
base = Gtk.CssProvider()
base.load_from_path("src/css/base.css")
Gtk.StyleContext.add_provider_for_display(window.get_display(), base, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
provider = Gtk.CssProvider()
provider.load_from_path("themes/lars/theme.css")
Gtk.StyleContext.add_provider_for_display(window.get_display(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
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
print("X11 pointer drag:", seen, "allocated:", dock.get_width(), flush=True)
assert not failures, failures
assert seen["begin"], "outside divider or GTK did not receive press"
assert seen["updates"] >= 2, "divider received no drag updates"
assert seen["smallest"] <= 80, "real X11 pointer movement failed to narrow sidebar"
assert dock.get_width() <= 85, "compact dock did not resize when dragged"
assert labels[0].get_visible() is False, "icon-only mode must hide only text"
dock.set_size_request(44, -1)
settle = GLib.MainLoop()
GLib.timeout_add(400, lambda: (settle.quit(), False)[1])
settle.run()
assert dock.get_width() <= 44, f"themed rail cannot reach minimum: {dock.get_width()}"

# A context popover is a widget child of the triggering sidebar button.
# A GTK leave event caused by popup grabs must not hide the sidebar.
button = rail.get_first_child()
popover = Gtk.Popover()
popover.set_parent(button)
popover.set_child(Gtk.Button(label="Open folder"))
popover.popup()

def mapped_popover(widget):
    if isinstance(widget, Gtk.Popover) and widget.get_mapped():
        return True
    child = widget.get_first_child()
    while child is not None:
        next_sibling = child.get_next_sibling()
        if mapped_popover(child):
            return True
        child = next_sibling
    return False

def settle(milliseconds=300):
    step = GLib.MainLoop()
    GLib.timeout_add(milliseconds, lambda: (step.quit(), False)[1])
    step.run()

settle()
assert popover.get_mapped(), "test popup failed to map under X11"
assert mapped_popover(column), "open popup not discoverable from sidebar tree"
popover.popdown()
settle()
assert not mapped_popover(column), "closed popup would block sidebar auto-hide"
window.destroy()
