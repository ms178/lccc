#!/usr/bin/env python3
"""Safety and usability contracts for offline, unprivileged deb extraction."""
from __future__ import annotations

import io
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
EXTRACT = ROOT / 'scripts/offline_deb_extract.sh'


def make_deb(directory: Path, name: str, members: list[tuple[str, str, str]]) -> Path:
    """Build a minimal deb containing explicitly controlled tar members."""
    work = directory / f'{name}-components'
    work.mkdir()
    (work / 'debian-binary').write_text('2.0\n')
    with tarfile.open(work / 'control.tar.gz', 'w:gz') as control:
        data = b'Package: fixture\nVersion: 1\nArchitecture: all\nMaintainer: Test <test@example.invalid>\nDescription: fixture\n'
        info = tarfile.TarInfo('./control')
        info.size = len(data)
        control.addfile(info, io.BytesIO(data))
    with tarfile.open(work / 'data.tar.gz', 'w:gz') as archive:
        for kind, path, value in members:
            info = tarfile.TarInfo(path)
            if kind == 'file':
                data = value.encode()
                info.size = len(data)
                info.mode = 0o644
                archive.addfile(info, io.BytesIO(data))
            elif kind == 'dir':
                info.type = tarfile.DIRTYPE
                info.mode = 0o755
                archive.addfile(info)
            elif kind in ('symlink', 'hardlink'):
                info.type = tarfile.SYMTYPE if kind == 'symlink' else tarfile.LNKTYPE
                info.linkname = value
                archive.addfile(info)
            else:
                raise ValueError(kind)
    deb = directory / f'{name}.deb'
    subprocess.run(['ar', 'qc', str(deb), 'debian-binary', 'control.tar.gz', 'data.tar.gz'],
                   cwd=work, check=True, capture_output=True)
    return deb


class OfflineDebExtractTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix='lccc-deb-test-')
        self.path = Path(self.temp.name)
        self.good = make_deb(self.path, 'good', [
            ('dir', './usr', ''), ('dir', './usr/lib', ''),
            ('file', './usr/lib/item', 'payload'),
            ('symlink', './usr/lib/absolute', '/usr/lib/item'),
            ('symlink', './usr/lib/relative', 'item'),
        ])

    def tearDown(self) -> None:
        self.temp.cleanup()

    def run_extract(self, destination: Path | str, *packages: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(['bash', str(EXTRACT), str(destination), *(str(p) for p in packages)],
                              text=True, capture_output=True, timeout=30)

    def test_multiple_packages_confine_absolute_links_and_print_recipe(self) -> None:
        extra = make_deb(self.path, 'extra', [('file', './usr/lib/second', 'second')])
        destination = self.path / 'root with spaces'
        result = self.run_extract(destination, self.good, extra)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((destination / 'usr/lib/item').read_text(), 'payload')
        self.assertEqual((destination / 'usr/lib/second').read_text(), 'second')
        self.assertEqual(os.readlink(destination / 'usr/lib/absolute'), 'item')
        self.assertEqual((destination / 'usr/lib/absolute').read_text(), 'payload')
        for needle in ('C_INCLUDE_PATH', 'LIBRARY_PATH', 'LD_LIBRARY_PATH', '-B', '--dynamic-linker',
                       'LCCC does not read C_INCLUDE_PATH', 'partial overlay with host fallbacks',
                       'not isolate it'):
            self.assertIn(needle, result.stdout)

    def test_rejects_root_boundaries_and_symlink_escape(self) -> None:
        for destination in ('/', '/usr', '/tmp', '/home/user'):
            with self.subTest(destination=destination):
                result = self.run_extract(destination, self.good)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('strictly below', result.stderr)
        outside = self.path / 'outside'
        outside.symlink_to('/etc', target_is_directory=True)
        result = self.run_extract(outside / 'never-created', self.good)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('strictly below', result.stderr)

    def test_existing_nonempty_destination_is_untouched(self) -> None:
        dest = self.path / 'occupied'
        dest.mkdir()
        marker = dest / 'marker'
        marker.write_text('untouched')
        result = self.run_extract(dest, self.good)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(marker.read_text(), 'untouched')
        self.assertEqual(sorted(p.name for p in dest.iterdir()), ['marker'])

    def test_malicious_archives_fail_before_extraction(self) -> None:
        cases = {
            'traversal': [('file', '../escaped', 'bad')],
            'absolute': [('file', '/tmp/escaped', 'bad')],
            'escape-link': [('symlink', './out', '../../etc/passwd')],
            'hardlink': [('hardlink', './out', '../escaped')],
            'symlink-parent': [('symlink', './usr', '/tmp'), ('file', './usr/escaped', 'bad')],
            'overlapping-kind': [('symlink', './same', '/tmp'), ('file', './same', 'bad')],
        }
        for name, members in cases.items():
            with self.subTest(name=name):
                evil = make_deb(self.path, name, members)
                dest = self.path / f'root-{name}'
                result = self.run_extract(dest, evil)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertFalse(dest.exists())
        self.assertFalse((self.path / 'escaped').exists())

    def test_symlink_parent_across_packages_is_rejected(self) -> None:
        first = make_deb(self.path, 'first-link', [('symlink', './shared', '/tmp')])
        second = make_deb(self.path, 'second-child', [('file', './shared/item', 'bad')])
        dest = self.path / 'combined'
        result = self.run_extract(dest, first, second)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('traverses symlink', result.stderr)
        self.assertFalse(dest.exists())

    def test_chained_symlink_escape_never_publishes_stage(self) -> None:
        evil = make_deb(self.path, 'chained', [
            ('symlink', './a', '.'), ('symlink', './out', 'a/..'),
        ])
        dest = self.path / 'chain-root'
        result = self.run_extract(dest, evil)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('escapes sysroot', result.stderr)
        self.assertFalse(dest.exists())
        self.assertFalse(list(self.path.glob('chain-root.tmp.*')))


if __name__ == '__main__':
    unittest.main()
