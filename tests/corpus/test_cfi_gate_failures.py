"""CFI parity must reject failed or missing twins, not silently omit them."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
FAKE = '''#!/usr/bin/env python3
import os, sys
from pathlib import Path
args = sys.argv[1:]
out = Path(args[args.index('-o') + 1])
plain = '-fno-asynchronous-unwind-tables' in args
if plain and os.environ.get('BAD_TWIN') == 'reject':
    print('synthetic twin rejection', file=sys.stderr)
    sys.exit(23)
if plain and os.environ.get('BAD_TWIN') == 'missing':
    sys.exit(0)
text = '.text\\n.type f, @function\\nf:\\n.cfi_startproc\\nret\\n.cfi_endproc\\n.size f, .-f\\n'
if plain:
    text = ''.join(line for line in text.splitlines(True) if '.cfi_' not in line)
out.write_text(text)
'''

class CfiGateFailures(unittest.TestCase):
    def run_gate(self, mode):
        with tempfile.TemporaryDirectory(prefix='cfi-gate-test-') as td:
            root = Path(td)
            gate = root / 'gate.sh'
            shutil.copyfile(ROOT / 'tests/regression/check_cfi_invariants.sh', gate)
            (root / 'one.c').write_text('void f(void) {}\n')
            cc = root / 'compiler'; cc.write_text(FAKE); cc.chmod(0o700)
            env = {**os.environ, 'CCC': str(cc), 'BAD_TWIN': mode, 'JOBS': '2'}
            return subprocess.run(['bash', str(gate)], env=env, text=True, capture_output=True, timeout=15)

    def test_matching_twin_passes(self):
        result = self.run_gate('')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('2 functions, 2 returns', result.stdout)

    def test_rejected_twin_fails(self):
        result = self.run_gate('reject')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('synthetic twin rejection', result.stderr)

    def test_exit_zero_without_output_fails(self):
        result = self.run_gate('missing')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('no-unwind compile failed or produced no output', result.stderr)

if __name__ == '__main__':
    unittest.main()
