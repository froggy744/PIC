"""Exercise dependency approval without network access or package installation."""

import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "PIC-build-linux-one-script.sh"
ARCHIVES = re.findall(r'"([^"|]+\.tar\.gz)\|([a-f0-9]{64})\|([^"\n]+)"', SCRIPT.read_text())

STUB = r'''
import os, pathlib, subprocess, sys
tool = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
state = pathlib.Path(os.environ["TEST_STATE"])
with (state / "calls").open("a") as log:
    log.write(tool + " " + " ".join(args) + "\n")
if tool == "cargo":
    if args[0] == "metadata":
        sys.exit(0 if (state / "rust-ready").exists() else 1)
    if args[0] == "fetch":
        (state / "rust-ready").touch()
    else:
        sys.exit("Compilation must not run during dependency-only checks")
elif tool == "pkg-config":
    sys.exit(0 if (state / "host-ready").exists() else 1)
elif tool == "sudo":
    sys.exit(subprocess.call(args))
elif tool in ("dnf", "apt-get"):
    if os.environ.get("TEST_INSTALL_FAIL"):
        sys.exit(1)
    if not os.environ.get("TEST_HOST_OLD"):
        (state / "host-ready").touch()
elif tool == "git":
    if args[0] == "clone":
        dest = pathlib.Path(args[-1])
        dest.mkdir(parents=True)
        (dest / "Cargo.toml").write_text('[package]\nname="pic-rs"\nversion="1.0.0"\n')
        (dest / "Cargo.lock").touch()
elif tool == "uname":
    print(os.environ.get("TEST_ARCH", os.uname().machine))
elif tool == "curl":
    dest = pathlib.Path(args[args.index("-o") + 1])
    dest.parent.mkdir(parents=True, exist_ok=True)
    if ".AppImage" in dest.name:
        dest.write_text("#!/bin/sh\nexit 0\n")
    else:
        dest.write_text("corrupt" if os.environ.get("TEST_CORRUPT") else "archive")
elif tool == "sha256sum":
    dest = pathlib.Path(args[0])
    name = dest.name.removesuffix(".tmp")
    hashes = dict(line.split(" ", 1) for line in (state / "hashes").read_text().splitlines())
    digest = hashes.get(name, "0" * 64) if dest.read_text() == "archive" else "0" * 64
    print(digest + "  " + str(dest))
elif tool == "flatpak":
    if args[0] == "info":
        if not (state / "runtime-ready").exists():
            sys.exit(1)
        if "--show-metadata" in args:
            if "org.gnome.Sdk" in args[-1]:
                version = "24.08" if os.environ.get("TEST_INCOMPATIBLE_SDK") else "25.08"
                print("[Extension org.freedesktop.Platform.GL]\nversions=" + version + ";")
            else:
                print("runtime=org.freedesktop.Sdk/x86_64/25.08")
    elif args[0] == "install":
        (state / "runtime-ready").touch()
'''


class DependencyPreflightTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.project = self.root / "project"
        self.project.mkdir()
        self.script = self.project / SCRIPT.name
        shutil.copyfile(SCRIPT, self.script)
        (self.project / "Cargo.toml").write_text('[package]\nname="pic-rs"\nversion="1.0.0"\n')
        (self.project / "Cargo.lock").touch()
        self.state = self.root / "state"
        self.state.mkdir()
        (self.state / "hashes").write_text("\n".join(f"{name} {sha}" for name, sha, _ in ARCHIVES))
        self.bin = self.root / "bin"
        self.bin.mkdir()
        stub = self.bin / "stub"
        stub.write_text(f"#!{sys.executable}\n" + STUB)
        stub.chmod(0o755)
        for tool in ("cargo", "rustc", "pkg-config", "cmake", "cc", "c++", "make", "file",
                     "nasm", "patchelf", "flatpak", "flatpak-builder", "curl", "sha256sum", "dnf", "sudo", "git", "uname"):
            (self.bin / tool).symlink_to(stub)
        self.env = dict(os.environ, PATH=f"{self.bin}:/usr/bin:/bin", TEST_STATE=str(self.state),
                        PIC_BUILD_CACHE=str(self.root / "cache"),
                        PIC_FLATPAK_STATE_DIR=str(self.root / "flatpak-state"))

    def ready_host(self):
        (self.state / "host-ready").touch()

    def ready_all(self):
        self.ready_host()
        (self.state / "rust-ready").touch()
        (self.state / "runtime-ready").touch()
        arch = self.env.get("TEST_ARCH", os.uname().machine)
        tool = self.root / "cache/tools" / f"linuxdeploy-{arch}.AppImage"
        tool.parent.mkdir(parents=True)
        tool.write_text("#!/bin/sh\nexit 0\n")
        tool.chmod(0o755)
        for name, sha, _ in ARCHIVES:
            dest = self.root / "cache/flatpak-sources" / sha / name
            dest.parent.mkdir(parents=True)
            dest.write_text("archive")

    def run_check(self, answer="", target="both", check_only=True, mode="local"):
        command = ["bash", str(self.script), mode, "--target", target]
        if check_only:
            command.append("--check-dependencies")
        return subprocess.run(command, input=answer, text=True, capture_output=True,
                              env=self.env, timeout=15)

    def calls(self):
        path = self.state / "calls"
        return path.read_text() if path.exists() else ""

    def assert_no_install(self):
        self.assertNotRegex(self.calls(), r"(?m)^(curl |dnf |sudo |flatpak (install|remote-add)|cargo fetch)")

    def test_decline_lists_both_targets_and_does_not_install(self):
        self.ready_host()
        result = self.run_check("n\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("linuxdeploy", result.stdout)
        for name, _, _ in ARCHIVES:
            self.assertIn(name, result.stdout)
        self.assertIn("Rust", result.stdout)
        self.assert_no_install()

    def test_normal_build_checks_before_compilation(self):
        self.ready_host()
        result = self.run_check("n\n", check_only=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Install/download", result.stdout + result.stderr)
        self.assertNotRegex(self.calls(), r"(?m)^cargo (test|build)")
        self.assert_no_install()

    def test_eof_does_not_grant_approval(self):
        self.ready_host()
        result = self.run_check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("approval", (result.stdout + result.stderr).lower())
        self.assert_no_install()

    def test_approval_installs_rechecks_and_keeps_local_source(self):
        result = self.run_check("yes\n")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("All build dependencies are ready", result.stdout)
        self.assertIn("dnf install", self.calls())
        self.assertIn("flatpak install", self.calls())
        self.assertIn("cargo fetch --locked", self.calls())
        self.assertNotIn("git ", self.calls())
        self.assertNotRegex(self.calls(), r"(?m)^cargo (test|build)")

    def test_ready_dependencies_do_not_prompt_or_download(self):
        self.ready_all()
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("[y/N]", result.stdout + result.stderr)
        self.assert_no_install()

    def test_appimage_only_does_not_require_flatpak(self):
        self.ready_all()
        (self.state / "runtime-ready").unlink()
        shutil.rmtree(self.root / "cache/flatpak-sources")
        result = self.run_check(target="appimage")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("flatpak ", self.calls())
        self.assert_no_install()

    def test_flatpak_only_does_not_require_native_libraries_or_linuxdeploy(self):
        self.ready_all()
        (self.state / "host-ready").unlink()
        shutil.rmtree(self.root / "cache/tools")
        result = self.run_check(target="flatpak")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("pkg-config ", self.calls())
        self.assert_no_install()

    def test_bad_archive_checksum_stops_before_build(self):
        self.ready_host()
        self.env["TEST_CORRUPT"] = "1"
        result = self.run_check("yes\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Could not download or verify", result.stdout + result.stderr)
        self.assertNotRegex(self.calls(), r"(?m)^cargo (test|build)")

    def test_blank_answer_defaults_to_no(self):
        self.ready_host()
        result = self.run_check("\n")
        self.assertNotEqual(result.returncode, 0)
        self.assert_no_install()

    def test_host_install_failure_stops_setup(self):
        self.env["TEST_INSTALL_FAIL"] = "1"
        result = self.run_check("yes\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Host dependency installation failed", result.stdout + result.stderr)
        self.assertNotIn("curl ", self.calls())
        self.assertNotRegex(self.calls(), r"(?m)^cargo (test|build)")

    def test_installing_old_host_versions_does_not_allow_build(self):
        self.env["TEST_HOST_OLD"] = "1"
        result = self.run_check("yes\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Dependencies still unavailable", result.stdout + result.stderr)
        self.assertNotRegex(self.calls(), r"(?m)^cargo (test|build)")

    def test_incompatible_installed_sdk_is_rejected(self):
        self.ready_all()
        self.env["TEST_INCOMPATIBLE_SDK"] = "1"
        result = self.run_check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("incompatible", result.stdout + result.stderr)
        self.assert_no_install()

    def test_github_source_does_not_fetch_missing_crates_without_approval(self):
        self.ready_all()
        (self.state / "rust-ready").unlink()
        result = self.run_check("n\n", mode="github")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("git clone", self.calls())
        self.assertNotIn("cargo fetch", self.calls())
        self.assertNotRegex(self.calls(), r"(?m)^cargo (test|build)")
        self.assert_no_install()

    def test_fully_cached_build_does_not_require_a_downloader(self):
        self.ready_all()
        (self.bin / "curl").unlink()
        for tool in ("bash", "dirname", "mkdir", "date", "tee", "mktemp", "rm", "cp", "chmod",
                     "id", "find", "head", "tar", "awk", "sed"):
            (self.bin / tool).symlink_to(shutil.which(tool))
        self.env["PATH"] = str(self.bin)
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assert_no_install()

    def test_packaging_can_use_a_manually_cached_arm_linuxdeploy(self):
        self.env["TEST_ARCH"] = "aarch64"
        self.ready_all()
        # Exercise the packaging resolver itself, rather than just inventory.
        source = self.script.read_text()
        resolver = source[source.index("linuxdeploy_path() {"):source.index("ensure_flatpak_runtime() {")]
        self.env["TOOLS_DIR"] = str(self.root / "cache/tools")
        result = subprocess.run(
            ["bash"], input='have() { command -v "$1" >/dev/null 2>&1; }\n'
            'warn() { echo "$*" >&2; }\nONLINE=0\n' + resolver + '\nlinuxdeploy_path\n',
            text=True, capture_output=True, env=self.env, timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout.strip(), str(self.root / "cache/tools/linuxdeploy-aarch64.AppImage"))
        self.assert_no_install()


if __name__ == "__main__":
    unittest.main()
