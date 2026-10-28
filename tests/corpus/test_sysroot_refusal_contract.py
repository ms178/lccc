"""Unsupported GNU sysroots must not silently compile against host paths."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
LCCC = ROOT / 'target/fastbuild/lccc'
GCC = shutil.which('gcc-16') or shutil.which('gcc')


@unittest.skipUnless(LCCC.is_file() and GCC, 'built LCCC and native GCC required')
class SysrootRefusal(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='lccc-sysroot-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.sysroot = self.root / 'private root'
        include = self.sysroot / 'usr/include'
        include.mkdir(parents=True)
        (include / 'lccc_private.h').write_text('#define PRIVATE_VALUE 123\n')
        self.source = self.root / 'source.c'
        self.source.write_text('int x = 1;\n')
        self.env = dict(os.environ)
        for key in ('LCCC_SYSROOT', 'CPATH', 'C_INCLUDE_PATH', 'CPLUS_INCLUDE_PATH'):
            self.env.pop(key, None)

    def run_cc(self, cc, *args):
        return subprocess.run([str(cc), *map(str, args)], cwd=self.root,
                              text=True, capture_output=True, env=self.env, timeout=30)

    def test_gcc_uses_private_headers_and_lccc_can_use_explicit_paths(self):
        self.source.write_text('#include <lccc_private.h>\nint x = PRIVATE_VALUE;\n')
        gcc = self.run_cc(GCC, f'--sysroot={self.sysroot}', '-E', '-P', self.source)
        self.assertEqual(gcc.returncode, 0, gcc.stderr)
        self.assertIn('123', gcc.stdout)
        lccc = self.run_cc(LCCC, '-nostdinc', '-I', self.sysroot / 'usr/include',
                           '-E', '-P', self.source)
        self.assertEqual(lccc.returncode, 0, lccc.stderr)
        self.assertIn('123', lccc.stdout)

    def test_sysroot_options_refuse_before_any_compilation(self):
        spellings = ((f'--sysroot={self.sysroot}',), ('--sysroot', self.sysroot),
                     (f'-isysroot={self.sysroot}',), ('-isysroot', self.sysroot),
                     ('--sysroot=/',), (f'-Wl,--sysroot={self.sysroot}',),
                     ('-Xlinker', f'--sysroot={self.sysroot}'))
        for target in ('lccc', 'lccc-i686', 'lccc-arm', 'lccc-riscv'):
            compiler = LCCC.parent / target
            self.assertTrue(compiler.is_file(), f'missing built target {compiler}')
            for flags in spellings:
                for mode in (('-E',), ('-fsyntax-only',), ('-c', '-o', self.root / 'out.o')):
                    with self.subTest(target=target, flags=flags, mode=mode):
                        run = self.run_cc(compiler, *flags, *mode, self.source)
                        self.assertNotEqual(run.returncode, 0, run.stdout)
                        self.assertIn('sysroot', run.stderr.lower())
                        self.assertIn('not supported', run.stderr.lower())

    def test_response_file_and_sysroot_dependent_queries_also_refuse(self):
        response = self.root / 'args.rsp'
        response.write_text(f'--sysroot="{self.sysroot}"\n')
        for args in ((f'@{response}', '-E', self.source),
                     (f'@{response}', '-print-libgcc-file-name'),
                     ('--sysroot', self.sysroot, '-print-file-name=crtbegin.o')):
            with self.subTest(args=args):
                run = self.run_cc(LCCC, *args)
                self.assertNotEqual(run.returncode, 0, run.stdout)
                self.assertIn('sysroot', run.stderr.lower())
                self.assertIn('not supported', run.stderr.lower())

    def test_missing_root_operand_is_diagnosed(self):
        for option in ('--sysroot', '-isysroot'):
            run = self.run_cc(LCCC, option)
            self.assertNotEqual(run.returncode, 0, run.stdout)
            self.assertIn('requires a directory', run.stderr.lower())

    def test_option_like_output_operand_is_not_a_sysroot(self):
        run = self.run_cc(LCCC, '-E', '-o', '--sysroot=output', self.source)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertTrue((self.root / '--sysroot=output').is_file())


if __name__ == '__main__':
    unittest.main()
