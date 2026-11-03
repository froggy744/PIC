"""The packaging version prompt keeps Cargo's manifest and lockfile aligned."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / 'scripts/PIC-build-linux-one-script.sh'


class BuildVersion(unittest.TestCase):
    def run_prompt(self, answer):
        source = SCRIPT.read_text()
        function = source.split('prompt_project_version() {', 1)[1].split('\n}', 1)[0]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'Cargo.toml').write_text('[package]\nname = "pic-rs"\nversion = "1.0.1"\n')
            (root / 'Cargo.lock').write_text('[[package]]\nname = "pic-rs"\nversion = "1.0.1"\n')
            script = 'set -e\ndie() { echo "$*" >&2; exit 1; }\nprompt_project_version() {' + function + '\n}\nprompt_project_version\n'
            result = subprocess.run(['bash', '-c', script], input=answer, text=True, capture_output=True,
                                    env=dict(os.environ, SOURCE_DIR=tmp))
            return result, (root / 'Cargo.toml').read_text(), (root / 'Cargo.lock').read_text()

    def test_new_version_updates_both_files(self):
        result, manifest, lock = self.run_prompt('1.0.2\n')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('version = "1.0.2"', manifest)
        self.assertIn('version = "1.0.2"', lock)

    def test_blank_keeps_version(self):
        result, manifest, lock = self.run_prompt('\n')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('version = "1.0.1"', manifest)
        self.assertIn('version = "1.0.1"', lock)

    def test_invalid_version_does_not_change_files(self):
        result, manifest, lock = self.run_prompt('v1.02\n')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('version = "1.0.1"', manifest)
        self.assertIn('version = "1.0.1"', lock)

if __name__ == '__main__':
    unittest.main()
