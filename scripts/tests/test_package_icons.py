"""Exercise package staging with actual tracked-layout icons and fake build tools."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]

class PackageIcons(unittest.TestCase):
    def test_deb_and_rpm_include_tracked_icons(self):
        for kind in ('deb','rpm'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp)/'checkout with spaces'
                (root/'scripts').mkdir(parents=True)
                shutil.copy2(ROOT/f'scripts/build-{kind}.sh',root/f'scripts/build-{kind}.sh')
                (root/'Cargo.toml').touch()
                (root/'target/release').mkdir(parents=True)
                binary=root/'target/release/pic-rs';binary.write_text('#!/bin/sh\nexit 0\n');binary.chmod(0o755)
                (root/'icon').mkdir()
                (root/'icon/pic-48.png').write_bytes(b'test-png')
                tools=Path(tmp)/'tools';tools.mkdir()
                cargo=tools/'cargo'
                metadata={'packages':[{'id':'pic','name':'pic-rs','version':'1.0.0','targets':[{'name':'pic-rs','kind':['bin']}]}]}
                cargo.write_text('#!/bin/sh\nif [ "$1" = metadata ]; then\ncat <<\'JSON\'\n'+json.dumps(metadata)+'\nJSON\nfi\n');cargo.chmod(0o755)
                for name in ('dpkg-deb','ldd'):
                    p=tools/name;p.write_text('#!/bin/sh\nexit 0\n');p.chmod(0o755)
                p=tools/'rpmbuild';p.write_text('#!/bin/sh\nmkdir -p "$TEST_REPO/target/rpmbuild/RPMS"\ntouch "$TEST_REPO/target/rpmbuild/RPMS/pic.rpm"\n');p.chmod(0o755)
                env=dict(os.environ,PATH=f'{tools}:/usr/bin:/bin',TEST_REPO=str(root))
                result=subprocess.run(['bash',str(root/f'scripts/build-{kind}.sh')],cwd=tmp,env=env,text=True,capture_output=True)
                self.assertEqual(result.returncode,0,result.stdout+result.stderr)
                payload=root/('target/package-deb' if kind=='deb' else 'target/rpmbuild/SOURCES/payload')
                self.assertEqual((payload/'usr/share/icons/hicolor/48x48/apps/io.github.froggy744.PIC.png').read_bytes(),b'test-png')

    def test_generator_requires_explicit_artwork(self):
        result=subprocess.run(['bash',str(ROOT/'scripts/generate-icon.sh')],cwd='/tmp',text=True,capture_output=True)
        self.assertNotEqual(result.returncode,0)
        self.assertIn('Usage:',result.stderr)

    def test_bundle_outputs_resources_and_restores_custom_icons(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)/'checkout with spaces'
            (root/'scripts').mkdir(parents=True)
            (root/'resources/icons').mkdir(parents=True)
            (root/'resources/custom-icons').mkdir()
            (root/'resources/custom-icons/custom.svg').write_text('<svg/>')
            (root/'resources/icons/stale.svg').write_text('<svg/>')
            shutil.copy2(ROOT/'scripts/build-icon-bundle.sh',root/'scripts/build-icon-bundle.sh')
            theme=Path(tmp)/'theme'
            (theme/'symbolic/actions').mkdir(parents=True)
            (theme/'symbolic/actions/sidebar-show-symbolic.svg').write_text('<svg><g fill="#2e3436"></g></svg>')
            tools=Path(tmp)/'tools';tools.mkdir()
            p=tools/'glib-compile-resources';p.write_text('#!/bin/sh\nfor arg do\n case "$arg" in --target=*) touch "${arg#--target=}";; esac\ndone\n');p.chmod(0o755)
            result=subprocess.run(['bash',str(root/'scripts/build-icon-bundle.sh'),str(theme)],cwd=tmp,env=dict(os.environ,PATH=f'{tools}:/usr/bin:/bin'),text=True,capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertTrue((root/'resources/icons.gresource').exists())
            self.assertTrue((root/'resources/icons/custom.svg').exists())
            self.assertFalse((root/'resources/icons/stale.svg').exists())
            self.assertIn('icons/custom.svg',(root/'resources/icons.gresource.xml').read_text())
