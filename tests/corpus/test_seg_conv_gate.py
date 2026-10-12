"""The conversion oracle gate must distinguish compiler rejection from failure."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
GATE = ROOT / 'tests/regression/check_seg_conv_diagnostics.sh'
FAKE = '''#!/usr/bin/env python3
import os, sys
from pathlib import Path
name = Path(sys.argv[0]).name
major = '13' if name == 'gcc' or name == 'gcc-13' else '14'
if '-dumpversion' in sys.argv:
    print(major); sys.exit(0)
if '--version' in sys.argv:
    print('fake GCC ' + major); sys.exit(0)
src = next(Path(a).name for a in sys.argv[1:] if a.endswith('.c'))
mode = os.environ.get('BAD_CCC' if name == 'ccc' else 'BAD_GCC', '')
if mode == 'positive' and src == 'control.c':
    print('fatal error: unavailable toolchain', file=sys.stderr); sys.exit(1)
if src.startswith('neg_'):
    if mode == 'accept': sys.exit(0)
    print('unrelated syntax error' if mode == 'unrelated' else 'error: incompatible address space', file=sys.stderr)
    sys.exit(139 if mode == 'crash' else 77 if mode == 'skip' else 1)
sys.exit(0)
'''


class SegmentConversionGateTests(unittest.TestCase):
    def run_gate(self, **overrides):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            for name in ['ccc', 'gcc', 'gcc-13', 'gcc-14']:
                p = d / name
                p.write_text(FAKE)
                p.chmod(0o755)
            env = {**os.environ, 'PATH': str(d) + os.pathsep + os.environ['PATH'],
                   'CCC': str(d / 'ccc'), 'GCC': '', 'BAD_CCC': '', 'BAD_GCC': '',
                   **overrides}
            return subprocess.run(['bash', str(GATE)], cwd=ROOT, env=env,
                                  capture_output=True, text=True)

    def test_versioned_oracle_beats_unversioned_gcc13(self):
        r = self.run_gate()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn('Conversion oracle: gcc-14', r.stdout)
        self.assertIn('two compilers, positive controls', r.stdout)

    def test_explicit_unsupported_or_missing_oracle_is_not_silently_replaced(self):
        for gcc in ['gcc-13', '/does-not-exist/gcc']:
            with self.subTest(gcc=gcc):
                r = self.run_gate(GCC=gcc)
                self.assertNotEqual(r.returncode, 0)
                self.assertIn('requires GCC >= 14', r.stderr)

    def test_both_compilers_must_pass_semantic_positive_control(self):
        for compiler in ['BAD_CCC', 'BAD_GCC']:
            with self.subTest(compiler=compiler):
                r = self.run_gate(**{compiler: 'positive'})
                self.assertNotEqual(r.returncode, 0)
                self.assertIn('positive control', r.stderr)

    def test_negative_rows_must_reject_for_the_right_reason_without_crashing(self):
        for compiler in ['BAD_CCC', 'BAD_GCC']:
            for mode in ['unrelated', 'accept', 'crash', 'skip']:
                with self.subTest(compiler=compiler, mode=mode):
                    r = self.run_gate(**{compiler: mode})
                    self.assertNotEqual(r.returncode, 0, r.stdout + r.stderr)
                    self.assertIn('FAIL:', r.stderr)


if __name__ == '__main__':
    unittest.main()
