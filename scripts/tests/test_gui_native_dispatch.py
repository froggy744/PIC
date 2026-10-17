"""Exercise native suite dispatch and its no-Wayland preflight."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


class NativeDispatchTests(unittest.TestCase):
    def test_native_runner_streams_on_the_desktop_bus_with_reduced_renderer_logs(self):
        source = (ROOT / "scripts/test-gui-native.sh").read_text()
        self.assertIn('python3 "$repo_root/scripts/gui_native_test.py"', source)
        self.assertIn('tee "$run_dir/harness.log"', source)
        self.assertNotIn("dbus-run-session -- python3", source)
        self.assertIn('export GSK_DEBUG=renderer', source)
        self.assertNotIn('export GSK_DEBUG=renderer,fallback', source)

    def test_native_suite_uses_automatic_wayland_ui_actions(self):
        source = (ROOT / "scripts/gui_native_test.py").read_text()
        self.assertIn("def run_automated_smoke()", source)
        self.assertIn("pyatspi.Registry.getDesktop", source)
        self.assertIn('PHOTO_FOLDER = Path(os.environ["PIC_GUI_NATIVE_PHOTO_DIR"])', source)
        self.assertNotIn("def prompt(", source)
        self.assertNotIn("run_manual_smoke()", source)

    def test_native_runner_refuses_to_reuse_a_running_pic_library(self):
        source = (ROOT / "scripts/test-gui-native.sh").read_text()
        self.assertIn("NameHasOwner", source)
        self.assertIn("PIC_GUI_NATIVE_PHOTO_DIR", source)
        self.assertIn('ydotoold', source)

    def test_native_suite_is_dispatched_before_xvfb(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "project"
            scripts = root / "scripts"
            scripts.mkdir(parents=True)
            shutil.copyfile(ROOT / "scripts/test-gui.sh", scripts / "test-gui.sh")
            marker = Path(temporary) / "native-runner-called"
            runner = scripts / "test-gui-native.sh"
            runner.write_text(f"#!/bin/sh\ntouch '{marker}'\nexit 23\n")
            runner.chmod(0o755)
            environment = dict(os.environ, PIC_GUI_SUITE="native")
            result = subprocess.run(["bash", str(scripts / "test-gui.sh")],
                                    env=environment, text=True, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 23, result.stdout + result.stderr)
            self.assertTrue(marker.exists(), "native selection did not invoke the native runner")
            self.assertFalse((root / "reports/gui-tests/xvfb.log").exists())

    def test_missing_wayland_writes_blocked_report_without_launching_pic(self):
        with tempfile.TemporaryDirectory() as temporary:
            run_dir = Path(temporary) / "run"
            environment = dict(os.environ, PIC_GUI_NATIVE_RUN_DIR=str(run_dir),
                               PIC_GUI_APP="/does/not/exist")
            environment.pop("WAYLAND_DISPLAY", None)
            result = subprocess.run(["bash", str(ROOT / "scripts/test-gui-native.sh")],
                                    env=environment, text=True, capture_output=True, timeout=10)
            report = run_dir / "report.md"
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertTrue(report.is_file())
            self.assertIn("BLOCKED", report.read_text())
            self.assertFalse((run_dir / "application.log").exists())
            self.assertFalse((run_dir / "xvfb.log").exists())


if __name__ == "__main__":
    unittest.main()
