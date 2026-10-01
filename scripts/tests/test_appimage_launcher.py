"""AppImage desktop registration must never shadow the Flatpak app ID."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / 'scripts/PIC-build-linux-one-script.sh'


class AppImageLauncherTests(unittest.TestCase):
    def test_appimage_and_flatpak_launchers_coexist(self):
        source = SCRIPT.read_text()
        function = 'integrate_appimage_gnome() {' + source.split('integrate_appimage_gnome() {', 1)[1].split('\n}\n', 1)[0] + '\n}\n'
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / 'local data'
            applications = data / 'applications'
            applications.mkdir(parents=True)
            flatpak_desktop = applications / 'io.github.you.PicRs.desktop'
            flatpak_desktop.write_text('[Desktop Entry]\nType=Application\nName=PIC Flatpak\nExec=/usr/bin/true\nIcon=io.github.you.PicRs\n')
            original = flatpak_desktop.read_bytes()
            appimage = root / 'PIC with spaces.AppImage'
            appimage.write_text('#!/bin/sh\nexit 0\n')
            appimage.chmod(0o755)
            icon = root / 'icon.png'
            icon.write_bytes((ROOT / 'icon/pic-256.png').read_bytes())
            tools = root / 'tools'
            tools.mkdir()
            flatpak = tools / 'flatpak'
            flatpak.write_text('#!/bin/sh\nexit 0\n')
            flatpak.chmod(0o755)
            helpers = '''set -euo pipefail
ok() { :; }
log() { :; }
warn() { :; }
have() { case "$1" in gio|python3) return 1;; *) command -v "$1" >/dev/null;; esac; }
'''
            command = helpers + function + '\nintegrate_appimage_gnome "$1" "$2"\n'
            env = dict(os.environ, XDG_DATA_HOME=str(data), APP_ID='io.github.you.PicRs', ICON_EXT='png', PATH=f'{tools}:/usr/bin:/bin')
            result = subprocess.run(['bash', '-c', command, 'test', str(appimage), str(icon)], env=env, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(flatpak_desktop.read_bytes(), original)
            launcher = applications / 'io.github.you.PicRs.AppImage.desktop'
            self.assertTrue(launcher.is_file(), 'AppImage launcher does not have a separate desktop ID')
            self.assertIn('Exec="' + str(appimage) + '" %F', launcher.read_text())
            self.assertIn('Icon=io.github.you.PicRs', launcher.read_text())
            # A second run must preserve a user's edited AppImage launcher.
            launcher.write_text(launcher.read_text() + 'X-Test-User-Edit=true\n')
            edited = launcher.read_bytes()
            result = subprocess.run(['bash', '-c', command, 'test', str(appimage), str(icon)], env=env, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(launcher.read_bytes(), edited)
            self.assertEqual(flatpak_desktop.read_bytes(), original)
            appimage.unlink()
            # GNOME/GIO enumeration must still find Flatpak after the file is removed.
            probe = '''from gi.repository import Gio
apps = [a for a in Gio.AppInfo.get_all() if a.get_id() == 'io.github.you.PicRs.desktop']
assert len(apps) == 1 and apps[0].get_name() == 'PIC Flatpak'
assert apps[0].should_show()
print('Flatpak remains visible after AppImage removal')
'''
            gi_available = subprocess.run(['/usr/bin/python3', '-c', 'from gi.repository import Gio'], capture_output=True).returncode == 0
            if gi_available:
                result = subprocess.run(['/usr/bin/python3', '-c', probe], env=dict(env, XDG_DATA_DIRS=str(root/'system-data')), text=True, capture_output=True)
                self.assertEqual(result.returncode, 0, result.stderr)
