#!/usr/bin/env python3
"""Capture the active Wayland desktop through the XDG screenshot portal."""

from __future__ import annotations

import sys
import time
from pathlib import Path

from gi.repository import Gio, GLib


def capture(destination: Path) -> None:
    connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    unique_name = connection.get_unique_name()[1:].replace(".", "_")
    token = f"pic_gui_{os_pid()}_{time.monotonic_ns()}"
    request_path = f"/org/freedesktop/portal/desktop/request/{unique_name}/{token}"
    loop = GLib.MainLoop()
    response_data: dict[str, object] = {}

    def on_response(_connection, _sender, _path, _interface, _signal, parameters, _data):
        code, results = parameters.unpack()
        response_data["code"] = code
        response_data["results"] = results
        loop.quit()

    subscription = connection.signal_subscribe(
        "org.freedesktop.portal.Desktop",
        "org.freedesktop.portal.Request",
        "Response",
        request_path,
        None,
        Gio.DBusSignalFlags.NONE,
        on_response,
        None,
    )
    options = {
        "handle_token": GLib.Variant("s", token),
        "interactive": GLib.Variant("b", False),
    }
    connection.call_sync(
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Screenshot",
        "Screenshot",
        GLib.Variant("(sa{sv})", ("", options)),
        GLib.VariantType.new("(o)"),
        Gio.DBusCallFlags.NONE,
        15000,
        None,
    )
    GLib.timeout_add_seconds(20, lambda: (loop.quit(), False)[1])
    loop.run()
    connection.signal_unsubscribe(subscription)

    if response_data.get("code") != 0:
        raise RuntimeError(f"Wayland screenshot portal returned {response_data.get('code')}")
    results = response_data.get("results")
    if not isinstance(results, dict) or not isinstance(results.get("uri"), str):
        raise RuntimeError("Wayland screenshot portal returned no screenshot URI")

    source = Gio.File.new_for_uri(results["uri"])
    target = Gio.File.new_for_path(str(destination))
    if not source.move(target, Gio.FileCopyFlags.OVERWRITE, None, None):
        raise RuntimeError("Could not move the portal screenshot into the test artifact directory")


def os_pid() -> int:
    # Keep the request token unique even if several screenshots start together.
    import os

    return os.getpid()


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: capture_wayland_screenshot.py OUTPUT.png")
    target_path = Path(sys.argv[1])
    target_path.parent.mkdir(parents=True, exist_ok=True)
    capture(target_path)
