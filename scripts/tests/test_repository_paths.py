"""Guard repository/standalone defaults and downloader sibling lookup."""
import os
from pathlib import Path
import shutil
import subprocess
import unittest
import test_build_dependencies as dependencies
ARCHIVES = dependencies.ARCHIVES


class RepositoryPaths(unittest.TestCase):
    ready_all = dependencies.DependencyPreflightTests.ready_all
    ready_host = dependencies.DependencyPreflightTests.ready_host
    def setUp(self):
        dependencies.DependencyPreflightTests.setUp(self)
        self.project = self.root / 'checkout with spaces'
        self.project.mkdir()
        (self.project/'Cargo.toml').write_text('[package]\nname="pic-rs"\nversion="1.0.0"\n')
        (self.project/'Cargo.lock').touch()
        scripts = self.project / 'scripts'
        scripts.mkdir()
        original = self.script
        self.script = scripts / original.name
        shutil.copy2(original,self.script)
        self.env.pop('PIC_FLATPAK_STATE_DIR',None)

    def test_repository_default_from_unrelated_directory(self):
        self.ready_all()
        result = subprocess.run(['bash',str(self.script),'local','--check-dependencies'],cwd=self.root,env=self.env,text=True,capture_output=True)
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        self.assertTrue(list((self.project/'build-logs').glob('*.log')))
        self.assertFalse((self.script.parent/'build-logs').exists())

    def test_explicit_project_and_log_override(self):
        self.ready_all()
        logs=self.root/'custom logs'
        result=subprocess.run(['bash',str(self.script),'local','--project',str(self.project),'--log-dir',str(logs),'--check-dependencies'],cwd=self.root,env=self.env,text=True,capture_output=True)
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        self.assertTrue(list(logs.glob('*.log')))

    def test_standalone_github_copy(self):
        self.ready_all()
        standalone=self.root/'standalone'
        standalone.mkdir()
        script=standalone/self.script.name
        shutil.copy2(self.script,script)
        result=subprocess.run(['bash',str(script),'github','--check-dependencies'],cwd=self.root,env=self.env,text=True,capture_output=True)
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        self.assertTrue(list((standalone/'build-logs').glob('*.log')))

    def test_downloader_finds_all_pinned_sources(self):
        self.ready_all()
        origin=Path(__file__).resolve().parents[2]
        source=origin/'scripts/PIC-download-linux-dependencies.sh'
        if not source.exists(): source=origin/source.name
        downloader=self.script.parent/source.name
        shutil.copy2(source,downloader)
        # All files are cached; checksum double accepts our controlled fixture.
        (self.bin/'sha256sum').unlink()
        (self.bin/'sha256sum').write_text('#!/bin/sh\ncat >/dev/null\nexit 0\n')
        (self.bin/'sha256sum').chmod(0o755)
        result=subprocess.run(['bash',str(downloader)],cwd=self.root,env=self.env,text=True,capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)
        for name,_,_ in ARCHIVES: self.assertIn('Cached OK: '+name,result.stdout)
