"""Exact private runtime/sysroot wiring without a real compiler or root writes."""
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / 'scripts/ci_export_gcc_runtime.sh'


class GccRuntimeExport(unittest.TestCase):
    def setup(self, root):
        directory = root / 'private compiler/lib/gcc/x86_64-pc-linux-gnu/16.2.0'
        (directory / '32').mkdir(parents=True)
        for arch in (directory, directory / '32'):
            for name in ('crtbegin.o', 'libgcc.a'):
                (arch / name).write_bytes(f'{arch.name}/{name}'.encode())
        driver = root / 'mock compiler'
        driver.write_text('''#!/bin/sh
if [ "$1" = -dumpfullversion ]; then echo "${MOCK_VERSION:-16.2.0}"; exit 0; fi
root=''' + shlex.quote(str(directory)) + '''
if [ "$1" = -m32 ]; then root="$root/32"; shift; fi
case "$1" in
    -print-file-name=crtbegin.o) echo "$root/crtbegin.o" ;;
    -print-libgcc-file-name) echo "$root/libgcc.a" ;;
    *) exit 99 ;;
esac
''')
        driver.chmod(0o755)
        return directory, driver

    def run_helper(self, root, driver, **env):
        return subprocess.run(['bash', str(HELPER)], env={**os.environ,
                              'RUNNER_TEMP': str(root), 'GITHUB_ENV': str(root / 'env'),
                              'GCC_DRIVER': str(driver), **env},
                              text=True, capture_output=True, check=False)

    def test_exports_coherent_dual_mode_runtime_with_hash_receipt(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            directory, driver = self.setup(root)
            result = self.run_helper(root, driver)
            self.assertEqual(result.returncode, 0, result.stderr)
            sysroot = root / 'current-gcc-runtime'
            self.assertEqual(result.stdout.strip(), str(sysroot))
            self.assertEqual((root / 'env').read_text(), f'LCCC_SYSROOT={sysroot}\n')
            link = sysroot / 'usr/lib/gcc/x86_64-linux-gnu/16.2.0'
            self.assertEqual(link.resolve(), directory)
            receipt = json.loads((sysroot / 'runtime.json').read_text())
            self.assertEqual(receipt['gcc_version'], '16.2.0')
            self.assertEqual(len(receipt['files']), 4)
            for item in receipt['files']:
                self.assertEqual(item['sha256'], hashlib.sha256(Path(item['path']).read_bytes()).hexdigest())

    def test_missing_i386_runtime_cannot_be_exported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            directory, driver = self.setup(root)
            (directory / '32/libgcc.a').unlink()
            result = self.run_helper(root, driver)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('missing exact GCC runtime file', result.stderr)
            self.assertFalse((root / 'env').exists())
            self.assertFalse((root / 'current-gcc-runtime').exists())

    def test_wrong_gcc_version_cannot_be_exported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _, driver = self.setup(root)
            result = self.run_helper(root, driver, MOCK_VERSION='16.0.1')
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('GCC 16.2.0 is required', result.stderr)
            self.assertFalse((root / 'env').exists())

    def test_existing_sysroot_is_not_overlaid(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _, driver = self.setup(root)
            sysroot = root / 'current-gcc-runtime'
            sysroot.mkdir()
            sentinel = sysroot / 'keep'
            sentinel.write_text('untouched')
            result = self.run_helper(root, driver)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('refusing an existing runtime sysroot', result.stderr)
            self.assertEqual(sentinel.read_text(), 'untouched')
            self.assertFalse((root / 'env').exists())


if __name__ == '__main__':
    unittest.main()
