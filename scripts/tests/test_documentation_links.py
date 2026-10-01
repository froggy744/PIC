"""Validate local links in current documentation, excluding historical records."""
from pathlib import Path
import re
import unittest
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[2]


def local_links(text):
    text = re.sub(r'<!--.*?-->', '', text, flags=re.S)
    text = re.sub(r'^\s*(`{3,}|~{3,}).*?^\s*\1\s*$', '', text, flags=re.M | re.S)
    return re.findall(r'\]\(([^)]+)\)', text) + re.findall(r'<img\b[^>]*\bsrc=["\']([^"\']+)', text, re.I)


def broken_links(root):
    files = list(root.glob('*.md')) + list((root / 'docs').rglob('*.md'))
    files += list((root / 'resources').rglob('README.md'))
    broken = []
    for file in files:
        if 'superpowers' in file.relative_to(root).parts:
            continue
        for target in local_links(file.read_text()):
            target = target.strip()
            if target.startswith('<'):
                target = target[1:target.index('>')]
            else:
                target = re.split(r'\s+["\']', target, maxsplit=1)[0]
            url = urlsplit(target)
            if url.scheme or url.netloc or not url.path:
                continue
            if not (file.parent / unquote(url.path)).exists():
                broken.append(f'{file.relative_to(root)}: {target}')
    return broken


class DocumentationLinks(unittest.TestCase):
    def test_active_local_links_resolve(self):
        self.assertEqual(broken_links(ROOT), [])
