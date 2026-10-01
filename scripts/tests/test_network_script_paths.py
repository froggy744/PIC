"""Run diagnostics with compiler/probe doubles, without touching servers."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class NetworkPaths(unittest.TestCase):
    def test_locations_dispatch_and_overrides(self):
        with tempfile.TemporaryDirectory() as temp:
            base = Path(temp)
            repo = base / 'checkout with spaces'
            (repo / 'scripts').mkdir(parents=True)
            for name in ('scripts/test-shares.sh', 'scripts/pic-smb-probe.c', 'pic-nfs-probe.c'):
                shutil.copy2(ROOT / name, repo / name)
            tools = base / 'tools'
            tools.mkdir()
            gcc = tools / 'gcc'
            gcc.write_text('''#!/usr/bin/env python3
import pathlib,sys
args=sys.argv[1:]
source=next(pathlib.Path(a) for a in args if a.endswith('.c'))
assert source.is_file(), source
out=pathlib.Path(args[args.index('-o')+1])
out.write_text('#!/bin/sh\\nprintf "' + source.name + ' %s\\\\n" "$*"\\n')
out.chmod(0o755)
''')
            gcc.chmod(0o755)
            pkg = tools / 'pkg-config'
            pkg.write_text('#!/bin/sh\necho -I/mock/include -L/mock/lib -lmock\n')
            pkg.chmod(0o755)
            env = dict(os.environ, PATH=f'{tools}:/usr/bin:/bin')
            for cwd in (repo, base):
                for args, want in ((['--exports','test-host'],'pic-nfs-probe.c --exports test-host'),
                                   (['--nfs','4','test-host','/photos','/'],'pic-nfs-probe.c --list 4 test-host /photos /'),
                                   (['--smb','smb://test-host/photos'],'pic-smb-probe.c smb://test-host/photos')):
                    result = subprocess.run(['bash',str(repo/'scripts/test-shares.sh'),*args],cwd=cwd,env=env,text=True,capture_output=True)
                    self.assertEqual(result.returncode,0,result.stderr)
                    self.assertIn(want,result.stdout)
            self.assertTrue((repo/'target/diagnostics/pic-nfs-probe').is_file())
            custom = base / 'custom bins'
            custom.mkdir()
            env.update(PIC_DIAGNOSTICS_BIN_DIR=str(custom), PIC_SMB_PROBE_BIN=str(custom/'smb'), PIC_NFS_PROBE_BIN=str(custom/'nfs'))
            result = subprocess.run(['bash',str(repo/'scripts/test-shares.sh'),'--probe','3','test-host','/photos','/known.jpg'],cwd=base,env=env,text=True,capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertIn('pic-nfs-probe.c 3 test-host /photos /known.jpg',result.stdout)
            self.assertTrue((custom/'smb').exists())
            self.assertTrue((custom/'nfs').exists())
