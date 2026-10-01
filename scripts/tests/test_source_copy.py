"""Source archives retain offline/runtime inputs and omit cleanup state."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]

class SourceCopy(unittest.TestCase):
    def test_offline_inputs_survive_without_generated_state(self):
        source=(ROOT/'scripts/PIC-build-linux-one-script.sh').read_text()
        function=source.split('copy_source_tree() {',1)[1].split('\n}',1)[0]
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)/'source with spaces';root.mkdir()
            keep=['Cargo.toml','Cargo.lock','build.rs','src/main.rs','native/private_nfs.c','native/private_smb.c','resources/icons.gresource','scripts/test-shares.sh','scripts/pic-smb-probe.c','pic-nfs-probe.c','tests/20151128_144228.jpg','icon/pic-48.png','vendor/offline/data','user-archive.zip']
            omit=['target/cache','dist/pkg','build-logs/build.log','.flatpak-builder/state','logs/debug.txt','.worktrees/another/src/main.rs','nested/.worktrees/another/file','to-be-deleted/INDEX.md','.superpowers/state','scripts/tests/__pycache__/test.pyc','scripts/build.windows.log']
            for name in keep+omit:
                path=root/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(name)
            dest=Path(tmp)/'copied'
            script='set -euo pipefail\ncopy_source_tree() {'+function+'\n}\ncopy_source_tree "$DEST"\n'
            result=subprocess.run(['bash','-c',script],env=dict(os.environ,SOURCE_DIR=str(root),DEST=str(dest)),text=True,capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr)
            for name in keep: self.assertTrue((dest/name).is_file(),name)
            for name in omit: self.assertFalse((dest/name).exists(),name)
