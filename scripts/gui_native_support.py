"""Pure validation helpers for the native Fedora GUI smoke suite."""

from __future__ import annotations

import re
from pathlib import Path


def vulkan_renderer_evidence(log_text: str) -> tuple[bool, str]:
    """Require GTK runtime evidence that GskVulkanRenderer was initialized."""
    normalized = log_text.casefold()
    initialized_renderer = re.search(
        r"using renderer ['\"]gskvulkanrenderer['\"]"
        r"|gskvulkanrenderer.{0,160}(?:initiali[sz](?:ed|ing)|realiz(?:ed|ing))"
        r"|(?:initiali[sz](?:ed|ing)|realiz(?:ed|ing)).{0,160}gskvulkanrenderer",
        normalized,
    )
    non_vulkan_fallback = re.search(
        r"(?:fall(?:ing)?\s+back|fallback).{0,100}(?:cairo|opengl|glrenderer)"
        r"|(?:cairo|opengl|glrenderer).{0,100}(?:fall(?:ing)?\s+back|fallback)",
        normalized,
    )
    non_vulkan_surface_renderer = re.search(
        r"using renderer ['\"]gsk(?:cairo|gl)renderer['\"]", normalized
    )
    if non_vulkan_surface_renderer:
        non_vulkan_fallback = non_vulkan_fallback or non_vulkan_surface_renderer
    if initialized_renderer and not non_vulkan_fallback:
        return True, "GTK selected GskVulkanRenderer for a surface"
    if non_vulkan_fallback:
        return False, "GTK diagnostics report fallback to a non-Vulkan renderer"
    return False, "Vulkan renderer initialization not confirmed"


def classify_native_result(
    *,
    wayland_ready: bool,
    vulkan_ready: bool,
    canceled: bool,
    app_exited_early: bool,
    confirmations: dict[str, bool | None],
    imported_photos: int,
    persisted_photos: int,
) -> str:
    """Classify environment/manual interruptions separately from app defects."""
    required_confirmations = (
        "application_visible", "gallery", "lightbox", "zoom_1_to_1", "lightbox_closed"
    )
    if not wayland_ready or not vulkan_ready or canceled or app_exited_early:
        return "BLOCKED"
    if any(confirmations.get(step) is not True for step in required_confirmations):
        return "BLOCKED"
    if imported_photos <= 0 or persisted_photos != imported_photos:
        return "FAIL"
    return "PASS"


def native_lightbox_evidence(log_text: str) -> tuple[bool, bool, bool]:
    """Confirm GTK actually showed, zoomed, and closed the native lightbox."""
    lines = log_text.splitlines()
    opened = next((i for i, line in enumerate(lines)
                  if "PIC_LIGHTBOX_LAYOUT stage=opened" in line), None)
    if opened is None:
        return False, False, False
    zoomed = next((i for i, line in enumerate(lines)
                   if i > opened and "PIC_ZOOM_UI one_to_one enabled=true" in line
                   and "lightbox_visible=true" in line), None)
    closed = next((i for i, line in enumerate(lines)
                   if i > opened and "PIC_LIGHTBOX_LAYOUT stage=hidden" in line), None)
    if zoomed is not None and closed is not None and zoomed > closed:
        zoomed = None
    return True, zoomed is not None, closed is not None


def native_input_socket_path(runtime_dir: str, process_id: int) -> Path:
    """Keep the Wayland input daemon's Unix socket within sockaddr_un limits."""
    path = Path(runtime_dir) / f"pic-gui-{process_id}.sock"
    if len(str(path).encode()) > 107:
        raise ValueError(f"Runtime directory is too long for a Unix socket: {path.parent}")
    return path
