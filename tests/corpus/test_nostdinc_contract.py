"""Default include suppression must work in every driver preprocessing mode."""
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
class NoStandardIncludes(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='lccc-nostdinc-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.src = self.root / 'main.c'
        self.custom = self.root / 'custom'
        self.custom.mkdir()
        (self.custom / 'private.h').write_text('#define VALUE 47\n')
        (self.root / 'sibling.h').write_text('#define SIBLING 83\n')
        self.env = dict(os.environ)
        # No ambient include-path or partial-sysroot additions may make a
        # missing-header control pass by accident.
        for key in ('CPATH', 'C_INCLUDE_PATH', 'CPLUS_INCLUDE_PATH', 'LCCC_SYSROOT'):
            self.env.pop(key, None)

    def run_cc(self, cc, source, *flags):
        self.src.write_text(source)
        return subprocess.run([str(cc), *flags, str(self.src)], text=True,
                              capture_output=True, env=self.env, timeout=30)

    def test_nostdinc_suppresses_standard_and_bundled_headers(self):
        for cc in (GCC, LCCC):
            for header in ('stdio.h', 'stddef.h', 'immintrin.h'):
                for mode in (('-E',), ('-fsyntax-only',), ('-M',)):
                    with self.subTest(cc=cc, header=header, mode=mode):
                        run = self.run_cc(cc, f'#include <{header}>\n', '-nostdinc', *mode)
                        self.assertNotEqual(run.returncode, 0, run.stdout)
                        self.assertIn(header, run.stderr)

    def test_user_paths_and_quoted_current_directory_still_work(self):
        source = '#include <private.h>\n#include "sibling.h"\nint x=VALUE+SIBLING;\n'
        for cc in (GCC, LCCC):
            for option in ('-I', '-isystem', '-idirafter'):
                with self.subTest(cc=cc, option=option):
                    run = self.run_cc(cc, source, '-nostdinc', option, str(self.custom), '-E', '-P')
                    self.assertEqual(run.returncode, 0, run.stderr)
                    self.assertIn('47', run.stdout)
                    self.assertIn('83', run.stdout)
            run = self.run_cc(cc, '#include "private.h"\nint x=VALUE;\n',
                              '-nostdinc', '-iquote', str(self.custom), '-E', '-P')
            self.assertEqual(run.returncode, 0, run.stderr)
            # GCC falls back to the quote chain when *no* angle-search
            # directory survives; a nonexistent -I does not change that.
            for extra in ((), ('-I', str(self.root / 'missing'))):
                run = self.run_cc(cc, '#include <private.h>\nint x=VALUE;\n',
                                  '-nostdinc', '-iquote', str(self.custom), *extra, '-E', '-P')
                self.assertEqual(run.returncode, 0, run.stderr)
                self.assertIn('47', run.stdout)
            empty = self.root / 'empty'
            empty.mkdir(exist_ok=True)
            run = self.run_cc(cc, '#include <private.h>\n',
                              '-nostdinc', '-iquote', str(self.custom), '-I', str(empty), '-E', '-P')
            self.assertNotEqual(run.returncode, 0, run.stdout)

    def test_has_include_uses_the_same_suppressed_search(self):
        source = '''#if __has_include(<stdio.h>)
#error default search leaked
#endif
#if !__has_include("sibling.h")
#error quoted current directory lost
#endif
int x = 1;
'''
        for cc in (GCC, LCCC):
            with self.subTest(cc=cc):
                run = self.run_cc(cc, source, '-nostdinc', '-E', '-P')
                self.assertEqual(run.returncode, 0, run.stderr)
                probe = '#if !__has_include(<private.h>)\n#error missing quote fallback\n#endif\n'
                run = self.run_cc(cc, probe, '-nostdinc', '-iquote', str(self.custom), '-E', '-P')
                self.assertEqual(run.returncode, 0, run.stderr)

    def test_absolute_headers_need_no_search_directory(self):
        header = self.custom / 'private.h'
        source = f'''#if !__has_include(<{header}>)
#error absolute header was not found
#endif
#include <{header}>
int x = VALUE;
'''
        for cc in (GCC, LCCC):
            with self.subTest(cc=cc):
                run = self.run_cc(cc, source, '-nostdinc', '-E', '-P')
                self.assertEqual(run.returncode, 0, run.stderr)
                self.assertIn('47', run.stdout)

    def test_readded_bundled_directory_has_explicit_dependency_class(self):
        header = ROOT / 'include/lcccsimd.h'
        self.assertTrue(header.is_file())
        for cc in (GCC, LCCC):
            for option, expected in (('-I', True), ('-isystem', False)):
                with self.subTest(cc=cc, option=option):
                    run = self.run_cc(cc, '#include <lcccsimd.h>\n', '-nostdinc',
                                      option, str(header.parent), '-MM')
                    self.assertEqual(run.returncode, 0, run.stderr)
                    self.assertEqual('lcccsimd.h' in run.stdout, expected, run.stdout)

    def test_all_builtin_targets_remove_target_specific_defaults(self):
        for name in ('lccc', 'lccc-i686', 'lccc-arm', 'lccc-riscv'):
            compiler = LCCC.parent / name
            if not compiler.is_file():
                self.fail(f'missing built target {compiler}')
            with self.subTest(target=name):
                run = self.run_cc(compiler, '#include <stdio.h>\n', '-nostdinc', '-E')
                self.assertNotEqual(run.returncode, 0, run.stdout)
                self.assertIn('stdio.h', run.stderr)


if __name__ == '__main__':
    unittest.main()
