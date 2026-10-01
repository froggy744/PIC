"""Flatpak's state and build tree must share a filesystem, including overrides."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[2] / 'scripts/PIC-build-linux-one-script.sh'


class FlatpakFilesystemTests(unittest.TestCase):
    def test_build_tree_follows_selected_state_filesystem(self):
        source = SCRIPT.read_text()
        metainfo = 'write_metainfo_file() {' + source.split('write_metainfo_file() {', 1)[1].split('\n}\n', 1)[0] + '\n}\n'
        function = 'build_flatpak() {' + source.split('build_flatpak() {', 1)[1].split('# -------------------- main --------------------', 1)[0]
        with tempfile.TemporaryDirectory() as tmp, tempfile.TemporaryDirectory(dir=Path.home()) as cache:
            root = Path(tmp)
            tools = root / 'tools'
            tools.mkdir()
            builder = tools / 'flatpak-builder'
            builder.write_text(f'#!{sys.executable}\n' + '''
import json, os, pathlib, sys
args = sys.argv[1:]
state = pathlib.Path(next(a.split('=', 1)[1] for a in args if a.startswith('--state-dir=')))
build = pathlib.Path(args[-2])
# Real flatpak-builder creates the target before checking its device.
build.mkdir(parents=True, exist_ok=True)
assert state.stat().st_dev == build.stat().st_dev, 'state/build filesystem mismatch'
assert build.is_relative_to(state), 'build tree did not follow selected state'
manifest = json.loads(pathlib.Path(args[-1]).read_text())
assert (pathlib.Path(args[-1]).parent / 'flatpak-src/.cargo/config.toml').exists()
commands = manifest['modules'][-1]['build-commands']
metadata_command = next((c for c in commands if '/app/share/metainfo/' in c), None)
assert metadata_command is not None, 'Flatpak does not install store metadata'
import xml.etree.ElementTree as ET
metadata = pathlib.Path(args[-1]).parent / 'flatpak-src' / metadata_command.split()[2]
component = ET.parse(metadata).getroot()
assert component.findtext('id') == manifest['app-id']
assert component.findtext('launchable') == manifest['app-id'] + '.desktop'
assert component.find('releases/release').get('version') == '2.3.4'
assert '--disable-download' in args
pathlib.Path(os.environ['TEST_RESULT']).write_text(json.dumps({'state': str(state), 'build': str(build)}))
sys.exit(77)  # Stop before compilation; placement is the boundary under test.
''')
            builder.chmod(0o755)
            cargo = tools / 'cargo'
            cargo.write_text('#!/bin/sh\n[ "$1" = vendor ] && [ "$2" = --locked ] && [ "$3" = --offline ]\n')
            cargo.chmod(0o755)
            helpers = '''set -euo pipefail
log() { :; }
ensure_flatpak_runtime() { :; }
ensure_flatpak_module_sources() { :; }
copy_source_tree() { mkdir -p "$1"; }
write_desktop_file() { touch "$1"; }
write_runtime_launcher() { touch "$1"; }
find_or_make_icon() { ICON_EXT=png; touch "$1/$APP_ID.png"; }
'''
            for state in (root / 'checkout/.flatpak-builder', Path(cache) / 'custom state'):
                with self.subTest(state=state):
                    state.mkdir(parents=True)
                    sentinel = state / 'cached-download'
                    sentinel.write_text('preserve')
                    result_path = root / 'result.json'
                    result_path.unlink(missing_ok=True)
                    env = dict(os.environ, PATH=f'{tools}:/usr/bin:/bin',
                               WORK_ROOT=str(Path(cache) / 'work'), FLATPAK_STATE_DIR=str(state),
                               APP_ID='io.github.you.PicRs', BIN_NAME='pic-rs', GNOME_RUNTIME='50',
                               SKIP_TESTS='0', ONLINE='0', TEST_RESULT=str(result_path),
                               SOURCE_DIR=str(SCRIPT.parents[1]), VERSION='2.3.4')
                    result = subprocess.run(['bash', '-c', helpers + metainfo + function + '\nbuild_flatpak\n'],
                                            env=env, text=True, capture_output=True, timeout=10)
                    self.assertTrue(result_path.exists(), result.stderr)
                    self.assertEqual(json.loads(result_path.read_text())['state'], str(state))
                    self.assertEqual(result.returncode, 1)  # Builder failure still propagates.
                    self.assertEqual(sentinel.read_text(), 'preserve')
