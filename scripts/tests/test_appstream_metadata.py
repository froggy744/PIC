"""Verify store metadata and generated package identity without network access."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from urllib.parse import unquote, urlsplit
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
METAINFO = ROOT / 'resources/io.github.froggy744.PIC.metainfo.xml'
SCRIPT = ROOT / 'scripts/PIC-build-linux-one-script.sh'


class AppStreamMetadataTests(unittest.TestCase):
    def test_store_metadata_has_existing_replaceable_screenshots(self):
        self.assertTrue(METAINFO.is_file(), 'AppStream metadata is missing')
        component = ET.parse(METAINFO).getroot()
        self.assertEqual(component.findtext('id'), 'io.github.froggy744.PIC')
        self.assertTrue(component.findtext('description/p'))
        self.assertTrue(component.findtext('developer/name'))
        self.assertEqual(component.findtext('launchable'), 'io.github.froggy744.PIC.desktop')
        screenshots = component.findall('screenshots/screenshot')
        self.assertEqual(len(screenshots), 3)
        self.assertEqual(screenshots[0].get('type'), 'default')
        for screenshot in screenshots:
            url = urlsplit(screenshot.findtext('image'))
            self.assertEqual(url.scheme, 'https')
            self.assertEqual(url.netloc, 'raw.githubusercontent.com')
            self.assertTrue((ROOT / unquote(url.path.split('/main/', 1)[1])).is_file())

    def test_packaging_overrides_identity_and_version_and_validates(self):
        source = SCRIPT.read_text()
        body = source.split('write_metainfo_file() {', 1)[1].split('\n}\n', 1)[0] if 'write_metainfo_file() {' in source else '\n    :\n'
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'io.github.test.Custom.metainfo.xml'
            command = 'set -euo pipefail\nwrite_metainfo_file() {' + body + '\n}\nwrite_metainfo_file "$1"\n'
            result = subprocess.run(['bash', '-c', command, 'test', str(path)], env=dict(os.environ, SOURCE_DIR=str(ROOT), APP_ID='io.github.test.Custom', VERSION='2.3.4'), text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(path.is_file(), 'packager did not generate metadata')
            component = ET.parse(path).getroot()
            self.assertEqual(component.findtext('id'), 'io.github.test.Custom')
            self.assertEqual(component.findtext('launchable'), 'io.github.test.Custom.desktop')
            self.assertEqual(component.find('releases/release').get('version'), '2.3.4')
            result = subprocess.run(['appstreamcli', 'validate', '--no-net', str(path)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_missing_metadata_stops_before_vendoring_in_conditional_build(self):
        source = SCRIPT.read_text()
        generator = 'write_metainfo_file() {' + source.split('write_metainfo_file() {', 1)[1].split('\n}\n', 1)[0] + '\n}\n'
        build = 'build_flatpak() {' + source.split('build_flatpak() {', 1)[1].split('# -------------------- main --------------------', 1)[0]
        with tempfile.TemporaryDirectory() as tmp:
            marker = Path(tmp) / 'vendoring-started'
            helpers = '''set -euo pipefail
ensure_flatpak_runtime() { :; }
ensure_flatpak_module_sources() { :; }
copy_source_tree() { mkdir -p "$1"; }
write_desktop_file() { touch "$1"; }
write_runtime_launcher() { touch "$1"; }
find_or_make_icon() { ICON_EXT=png; }
log() { :; }
cargo() { touch "$MARKER"; return 1; }
flatpak-builder() { return 1; }
'''
            command = helpers + generator + build + '\nif build_flatpak; then exit 0; else exit 1; fi\n'
            env = dict(os.environ, SOURCE_DIR=tmp, FLATPAK_STATE_DIR=str(Path(tmp)/'state'),
                       APP_ID='io.github.froggy744.PIC', VERSION='1.0.0', BIN_NAME='pic-rs',
                       GNOME_RUNTIME='50', SKIP_TESTS='0', ONLINE='0', MARKER=str(marker))
            result = subprocess.run(['bash', '-c', command], env=env, text=True, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(marker.exists(), 'metadata failure allowed vendoring to start')
