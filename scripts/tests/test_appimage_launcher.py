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
        function = 'integrate_appimage_xdg() {' + source.split('integrate_appimage_xdg() {', 1)[1].split('\n}\n', 1)[0] + '\n}\n'
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / 'local data'
            applications = data / 'applications'
            applications.mkdir(parents=True)
            flatpak_desktop = applications / 'io.github.froggy744.PIC.desktop'
            flatpak_desktop.write_text('[Desktop Entry]\nType=Application\nName=PIC Flatpak\nExec=/usr/bin/true\nIcon=io.github.froggy744.PIC\n')
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
            command = helpers + function + '\nintegrate_appimage_xdg "$1" "$2"\n'
            env = dict(os.environ, XDG_DATA_HOME=str(data), APP_ID='io.github.froggy744.PIC', ICON_EXT='png', PATH=f'{tools}:/usr/bin:/bin')
            result = subprocess.run(['bash', '-c', command, 'test', str(appimage), str(icon)], env=env, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(flatpak_desktop.read_bytes(), original)
            launcher = applications / 'io.github.froggy744.PIC.AppImage.desktop'
            self.assertTrue(launcher.is_file(), 'AppImage launcher does not have a separate desktop ID')
            self.assertIn('Exec="' + str(appimage) + '" %F', launcher.read_text())
            self.assertIn('Icon=io.github.froggy744.PIC.AppImage', launcher.read_text())
            self.assertTrue((data / 'icons/hicolor/256x256/apps/io.github.froggy744.PIC.AppImage.png').is_file())
            # A second run updates the AppImage-owned launcher while leaving Flatpak alone.
            launcher.write_text(launcher.read_text() + 'X-Test-User-Edit=true\n')
            result = subprocess.run(['bash', '-c', command, 'test', str(appimage), str(icon)], env=env, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotIn('X-Test-User-Edit=true', launcher.read_text())
            self.assertIn('Exec="' + str(appimage) + '" %F', launcher.read_text())
            self.assertEqual(flatpak_desktop.read_bytes(), original)
            appimage.unlink()
            # GNOME/GIO enumeration must still find Flatpak after the file is removed.
            probe = '''from gi.repository import Gio
apps = [a for a in Gio.AppInfo.get_all() if a.get_id() == 'io.github.froggy744.PIC.desktop']
assert len(apps) == 1 and apps[0].get_name() == 'PIC Flatpak'
assert apps[0].should_show()
print('Flatpak remains visible after AppImage removal')
'''
            gi_available = subprocess.run(['/usr/bin/python3', '-c', 'from gi.repository import Gio'], capture_output=True).returncode == 0
            if gi_available:
                result = subprocess.run(['/usr/bin/python3', '-c', probe], env=dict(env, XDG_DATA_DIRS=str(root/'system-data')), text=True, capture_output=True)
                self.assertEqual(result.returncode, 0, result.stderr)


class AppImageUninstallTests(unittest.TestCase):
    def test_uninstall_rejects_non_appimage_path_without_removing_it(self):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp) / 'important.txt'
            app.write_text('keep this file')
            result = subprocess.run(
                ['bash', str(SCRIPT), '--uninstall-appimage', str(app)],
                env=dict(os.environ), text=True, capture_output=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('expects a .AppImage', result.stderr)
            self.assertEqual(app.read_text(), 'keep this file')

    def test_uninstall_removes_only_matching_appimage_registration_and_icon(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / 'local data'
            applications = data / 'applications'
            icons = data / 'icons/hicolor/256x256/apps'
            applications.mkdir(parents=True)
            icons.mkdir(parents=True)
            app_id = 'io.github.froggy744.PIC'
            appimage = root / 'PIC build.AppImage'
            appimage.write_text('#!/bin/sh\nexit 0\n')
            launcher = applications / f'{app_id}.AppImage.desktop'
            launcher.write_text(
                '[Desktop Entry]\n'
                'Name=PIC AppImage\n'
                f'Exec="{appimage}" %F\n'
                f'Icon={app_id}.AppImage\n'
            )
            image_icon = icons / f'{app_id}.AppImage.png'
            image_icon.write_bytes(b'appimage-icon')
            flatpak_launcher = applications / f'{app_id}.desktop'
            flatpak_launcher.write_text(f'Exec=flatpak run {app_id}\nIcon={app_id}\n')
            shared_flatpak_icon = icons / f'{app_id}.png'
            shared_flatpak_icon.write_bytes(b'flatpak-icon')
            catalog = data / 'pic-rs/catalog.sqlite'
            catalog.parent.mkdir()
            catalog.write_bytes(b'photo library')
            env = dict(os.environ, XDG_DATA_HOME=str(data), PIC_APP_ID=app_id)

            result = subprocess.run(
                ['bash', str(SCRIPT), '--uninstall-appimage', str(appimage)],
                env=env, text=True, capture_output=True,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(appimage.exists())
            self.assertFalse(launcher.exists())
            self.assertFalse(image_icon.exists())
            self.assertTrue(flatpak_launcher.exists())
            self.assertEqual(shared_flatpak_icon.read_bytes(), b'flatpak-icon')
            self.assertEqual(catalog.read_bytes(), b'photo library')

    def test_uninstall_keeps_launcher_when_its_exec_points_elsewhere(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / 'data'
            applications = data / 'applications'
            applications.mkdir(parents=True)
            app_id = 'io.github.froggy744.PIC'
            requested = root / 'requested.AppImage'
            other = root / 'other.AppImage'
            requested.touch()
            other.touch()
            launcher = applications / f'{app_id}.AppImage.desktop'
            launcher.write_text(f'Exec="{other}" %F\nIcon={app_id}.AppImage\n')
            icon_dir = data / 'icons/hicolor/256x256/apps'
            icon_dir.mkdir(parents=True)
            icon = icon_dir / f'{app_id}.AppImage.png'
            icon.write_bytes(b'keep icon used by other launcher')
            env = dict(os.environ, XDG_DATA_HOME=str(data), PIC_APP_ID=app_id)

            result = subprocess.run(
                ['bash', str(SCRIPT), '--uninstall-appimage', str(requested)],
                env=env, text=True, capture_output=True,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(requested.exists())
            self.assertTrue(other.exists())
            self.assertTrue(launcher.exists())
            self.assertEqual(icon.read_bytes(), b'keep icon used by other launcher')

    def test_uninstall_cleans_registration_after_appimage_was_manually_deleted(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / 'data'
            applications = data / 'applications'
            icon_dir = data / 'icons/hicolor/256x256/apps'
            applications.mkdir(parents=True)
            icon_dir.mkdir(parents=True)
            app_id = 'io.github.froggy744.PIC'
            appimage = root / 'already deleted.AppImage'
            launcher = applications / f'{app_id}.AppImage.desktop'
            launcher.write_text(f'Exec="{appimage}" %F\nIcon={app_id}.AppImage\n')
            icon = icon_dir / f'{app_id}.AppImage.png'
            icon.write_bytes(b'appimage icon')
            env = dict(os.environ, XDG_DATA_HOME=str(data), PIC_APP_ID=app_id)

            result = subprocess.run(
                ['bash', str(SCRIPT), '--uninstall-appimage', str(appimage)],
                env=env, text=True, capture_output=True,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(launcher.exists())
            self.assertFalse(icon.exists())

    def test_uninstall_cleans_orphaned_icon_when_launcher_was_already_removed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / 'data'
            icon_dir = data / 'icons/hicolor/256x256/apps'
            icon_dir.mkdir(parents=True)
            app_id = 'io.github.froggy744.PIC'
            appimage = root / 'removed.AppImage'
            icon = icon_dir / f'{app_id}.AppImage.png'
            icon.write_bytes(b'orphaned icon')
            env = dict(os.environ, XDG_DATA_HOME=str(data), PIC_APP_ID=app_id)

            result = subprocess.run(
                ['bash', str(SCRIPT), '--uninstall-appimage', str(appimage)],
                env=env, text=True, capture_output=True,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(icon.exists())
