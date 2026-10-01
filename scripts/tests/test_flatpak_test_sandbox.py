"""Verify SDK test metadata and failure handling without building packages."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[2] / "scripts" / "PIC-build-linux-one-script.sh"


class FlatpakTestSandboxTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.build = self.root / "build-dir"
        self.build.mkdir()
        self.metadata = "[Application]\nname=io.github.you.PicRs\nsdk=org.gnome.Sdk/x86_64/50\n"
        (self.build / "metadata").write_text(self.metadata)
        self.source = self.root / "selected-source"
        self.source.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        stub = self.bin / "flatpak"
        stub.write_text(f"#!{sys.executable}\n" + r'''
import json, os, pathlib, sys
args = sys.argv[1:]
root = pathlib.Path(os.environ["TEST_ROOT"])
metadata_arg = next(arg.split("=", 1)[1] for arg in args if arg.startswith("--metadata="))
metadata = (root / "build-dir" / metadata_arg).read_text()
(root / "invocation.json").write_text(json.dumps({"args": args, "metadata": metadata}))
# The actual SVG loader requires a Devel ID in an uninstalled build sandbox.
if "name=io.github.you.PicRs.Devel\n" not in metadata:
    sys.exit(101)
sys.exit(int(os.environ.get("TEST_EXIT", "0")))
''')
        stub.chmod(0o755)
        self.env = dict(os.environ, PATH=f"{self.bin}:/usr/bin:/bin", TEST_ROOT=str(self.root),
                        APP_ID="io.github.you.PicRs")

    def run_tests(self, skip=False):
        source = SCRIPT.read_text()
        if "run_flatpak_tests() {" in source:
            body = source.split("run_flatpak_tests() {", 1)[1].split("\n}\n", 1)[0]
            function = "run_flatpak_tests() {" + body + "\n}\n"
        else:
            function = ""
        command = 'set -euo pipefail\nlog() { echo "$*"; }\nwarn() { echo "$*" >&2; }\n'
        command += 'die() { echo "$*" >&2; exit 1; }\n'
        command += f"SKIP_TESTS={int(skip)}\n" + function + '\nrun_flatpak_tests "$1" "$2"\n'
        return subprocess.run(["bash", "-c", command, "test", str(self.build), str(self.source)],
                              capture_output=True, text=True, env=self.env, timeout=10)

    def test_tests_use_development_metadata_without_changing_package_identity(self):
        result = self.run_tests()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        invocation = json.loads((self.root / "invocation.json").read_text())
        self.assertIn("name=io.github.you.PicRs.Devel\n", invocation["metadata"])
        self.assertIn("/app/libexec/pic-build-tests/*", invocation["args"][-1])
        self.assertIn("--bind-mount=/run/build/pic-rs=" + str(self.source), invocation["args"])
        self.assertIn("--build-dir=/run/build/pic-rs", invocation["args"])
        self.assertEqual((self.build / "metadata").read_text(), self.metadata)
        self.assertEqual(sorted(path.name for path in self.build.iterdir()), ["metadata"])

    def run_installer(self, artifacts):
        installer = SCRIPT.read_text().split("<<'EOF_TEST_INSTALLER'\n", 1)[1].split("\nEOF_TEST_INSTALLER", 1)[0]
        records = self.root / "artifacts.json"
        records.write_text("\n".join(json.dumps(record) for record in artifacts))
        destination = self.root / "cached-tests"
        result = subprocess.run([sys.executable, "-c", installer, str(records), str(destination)],
                                capture_output=True, text=True, timeout=10)
        return result, destination

    def test_cached_tests_come_from_the_current_cargo_artifacts(self):
        executable = self.root / "selected-revision-tests"
        executable.write_text("selected revision test executable")
        result, destination = self.run_installer([
            {"reason": "compiler-artifact", "profile": {"test": False}, "executable": "/old/app"},
            {"reason": "compiler-artifact", "profile": {"test": True}, "executable": str(executable)},
            {"reason": "build-finished", "success": True},
        ])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([path.name for path in destination.iterdir()], [executable.name])
        self.assertEqual((destination / executable.name).read_text(), executable.read_text())
        self.assertTrue(os.access(destination / executable.name, os.X_OK))

    def test_no_test_artifacts_cannot_produce_a_successful_package(self):
        result, _ = self.run_installer([{"reason": "build-finished", "success": True}])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no test executables", result.stderr)

    def test_failed_tests_remain_fatal_and_remove_temporary_metadata(self):
        self.env["TEST_EXIT"] = "101"
        result = self.run_tests()
        self.assertEqual(result.returncode, 101, result.stdout + result.stderr)
        self.assertEqual((self.build / "metadata").read_text(), self.metadata)
        self.assertEqual(sorted(path.name for path in self.build.iterdir()), ["metadata"])

    def test_skip_tests_does_not_start_a_development_sandbox(self):
        result = self.run_tests(skip=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse((self.root / "invocation.json").exists())
        self.assertEqual(sorted(path.name for path in self.build.iterdir()), ["metadata"])


if __name__ == "__main__":
    unittest.main()
