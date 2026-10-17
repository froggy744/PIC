"""Test renderer evidence and terminal outcomes for the native GUI runner."""

from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
SUPPORT_PATH = ROOT / "scripts" / "gui_native_support.py"
SPEC = spec_from_file_location("gui_native_support", SUPPORT_PATH) if SUPPORT_PATH.is_file() else None
SUPPORT = module_from_spec(SPEC) if SPEC else None
if SPEC and SPEC.loader:
    SPEC.loader.exec_module(SUPPORT)


class NativeRendererEvidenceTests(unittest.TestCase):
    def require_support(self):
        self.assertIsNotNone(SUPPORT, "native GUI support module must be importable")

    def test_environment_request_alone_does_not_prove_vulkan(self):
        self.require_support()
        self.assertEqual(SUPPORT.vulkan_renderer_evidence("GSK_RENDERER=vulkan"),
                         (False, "Vulkan renderer initialization not confirmed"))

    def test_initialized_vulkan_renderer_is_confirmed(self):
        self.require_support()
        self.assertEqual(
            SUPPORT.vulkan_renderer_evidence("GSK: GskVulkanRenderer initialized and realized"),
            (True, "GTK selected GskVulkanRenderer for a surface"),
        )

    def test_gtk_renderer_debug_line_confirms_selected_renderer(self):
        self.require_support()
        self.assertEqual(
            SUPPORT.vulkan_renderer_evidence(
                "Gsk: Using renderer 'GskVulkanRenderer' for surface 'GdkWaylandToplevel'"),
            (True, "GTK selected GskVulkanRenderer for a surface"),
        )

    def test_native_lightbox_trace_requires_open_zoom_and_close_in_order(self):
        self.require_support()
        log = "\n".join((
            "PIC_LIGHTBOX_LAYOUT stage=opened",
            "PIC_ZOOM_UI one_to_one enabled=true lightbox_visible=true",
            "PIC_LIGHTBOX_LAYOUT stage=hidden",
        ))
        self.assertEqual(SUPPORT.native_lightbox_evidence(log), (True, True, True))

    def test_missing_native_lightbox_transition_is_not_counted_as_pass(self):
        self.require_support()
        self.assertEqual(
            SUPPORT.native_lightbox_evidence("PIC_LIGHTBOX_LAYOUT stage=opened"),
            (True, False, False),
        )

    def test_slider_reset_during_viewer_open_is_not_a_one_to_one_zoom(self):
        self.require_support()
        log = "\n".join((
            "PIC_LIGHTBOX_LAYOUT stage=opened",
            "PIC_ZOOM_UI event=value_changed value=0.000 lightbox_visible=true syncing=true",
            "PIC_LIGHTBOX_LAYOUT stage=hidden",
        ))
        self.assertEqual(SUPPORT.native_lightbox_evidence(log), (True, False, True))

    def test_lightbox_close_before_open_does_not_count_as_a_close_check(self):
        self.require_support()
        log = "\n".join((
            "PIC_LIGHTBOX_LAYOUT stage=hidden",
            "PIC_LIGHTBOX_LAYOUT stage=opened",
        ))
        self.assertEqual(SUPPORT.native_lightbox_evidence(log), (True, False, False))

    def test_native_input_socket_uses_a_short_runtime_path(self):
        self.require_support()
        path = SUPPORT.native_input_socket_path("/run/user/1000", 12345)
        self.assertEqual(path.parent.as_posix(), "/run/user/1000")
        self.assertLessEqual(len(str(path).encode()), 107)

    def test_cairo_renderer_is_not_vulkan_evidence(self):
        self.require_support()
        self.assertFalse(SUPPORT.vulkan_renderer_evidence(
            "GSK: GskCairoRenderer initialized and realized")[0])

    def test_vulkan_device_without_renderer_initialization_is_ambiguous(self):
        self.require_support()
        self.assertFalse(SUPPORT.vulkan_renderer_evidence(
            "Vulkan device found: AMD Radeon RX 7800 XT")[0])

    def test_fallback_to_non_vulkan_renderer_invalidates_vulkan_evidence(self):
        self.require_support()
        self.assertFalse(SUPPORT.vulkan_renderer_evidence(
            "GSK: GskVulkanRenderer initialized; falling back to Cairo renderer")[0])

    def test_cairo_surface_renderer_invalidates_vulkan_evidence(self):
        self.require_support()
        self.assertFalse(SUPPORT.vulkan_renderer_evidence(
            "Using renderer 'GskVulkanRenderer' for surface 'main'; "
            "Using renderer 'GskCairoRenderer' for surface 'dialog'")[0])


class NativeOutcomeTests(unittest.TestCase):
    def require_support(self):
        self.assertIsNotNone(SUPPORT, "native GUI support module must be importable")

    def outcome(self, **overrides):
        values = {
            "wayland_ready": True,
            "vulkan_ready": True,
            "canceled": False,
            "app_exited_early": False,
            "confirmations": {
                "application_visible": True,
                "gallery": True,
                "lightbox": True,
                "zoom_1_to_1": True,
                "lightbox_closed": True,
            },
            "imported_photos": 3,
            "persisted_photos": 3,
        }
        values.update(overrides)
        return SUPPORT.classify_native_result(**values)

    def test_complete_session_with_persisted_photos_passes(self):
        self.require_support()
        self.assertEqual(self.outcome(), "PASS")

    def test_unavailable_wayland_or_vulkan_is_blocked(self):
        self.require_support()
        for key in ("wayland_ready", "vulkan_ready"):
            with self.subTest(key=key):
                self.assertEqual(self.outcome(**{key: False}), "BLOCKED")

    def test_cancel_or_early_app_exit_is_blocked(self):
        self.require_support()
        self.assertEqual(self.outcome(canceled=True), "BLOCKED")
        self.assertEqual(self.outcome(app_exited_early=True), "BLOCKED")

    def test_each_missing_or_declined_visual_confirmation_is_blocked(self):
        self.require_support()
        for key in ("application_visible", "gallery", "lightbox", "zoom_1_to_1", "lightbox_closed"):
            for value in (None, False):
                with self.subTest(key=key, value=value):
                    confirmations = {
                        "application_visible": True,
                        "gallery": True,
                        "lightbox": True,
                        "zoom_1_to_1": True,
                        "lightbox_closed": True,
                    }
                    confirmations[key] = value
                    self.assertEqual(self.outcome(confirmations=confirmations), "BLOCKED")
        self.assertEqual(self.outcome(confirmations={}), "BLOCKED")

    def test_confirmed_import_with_no_indexed_photos_fails(self):
        self.require_support()
        self.assertEqual(self.outcome(imported_photos=0, persisted_photos=0), "FAIL")

    def test_lost_rows_after_restart_fails(self):
        self.require_support()
        self.assertEqual(self.outcome(imported_photos=3, persisted_photos=2), "FAIL")


if __name__ == "__main__":
    unittest.main()
