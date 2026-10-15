"""Exercise the real shell gate functions, not a parallel reimplementation."""
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = (ROOT / "tests/regression/check_affine_loop_fold.sh").read_text()
STUB = '''#!/usr/bin/env python3
import json, os, pathlib, sys
log = pathlib.Path(os.environ['TRACE'])
keys = ('CCC_LOOP_ROTATE', 'CCC_NO_AFFINE_EXIT_FOLD')
with log.open('a') as f: f.write(json.dumps({'phase':'compile','env':{k:os.environ.get(k) for k in keys}})+'\\n')
if os.environ.get('FAIL_BUILD'):
    print('forced compiler failure', file=sys.stderr); sys.exit(7)
p = pathlib.Path(sys.argv[sys.argv.index('-o')+1])
if str(p) == '/dev/null':
    print('affine exit-compare folds: 1', file=sys.stderr); sys.exit(0)
p.write_text("#!/usr/bin/env python3\\nimport os,json\\nwith open(os.environ['TRACE'],'a') as f: f.write(json.dumps({'phase':'run','rotate':os.environ.get('CCC_LOOP_ROTATE')})+'\\\\n')\\nprint('ok')\\nraise SystemExit(int(os.environ.get('FAIL_RUN','0')))\\n")
p.chmod(0o755)
'''


class AffineEnvironment(unittest.TestCase):
    def invoke(self, function, command, extra=None, script="check_affine_loop_fold.sh"):
        source = (ROOT / "tests/regression" / script).read_text()
        text = re.search(rf"^{function}\(\).*?^}}", source, re.M | re.S).group()
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            stub = tmp / "compiler"
            stub.write_text(STUB); stub.chmod(0o755)
            trace = tmp / "trace"
            env = {k: v for k, v in os.environ.items() if not k.startswith(('CCC_', 'FAIL_'))}
            env.update(TRACE=str(trace), CCC=str(stub), work=str(tmp), corpus="unused.c", expected="ok")
            env.update(extra or {})
            body = "set -u -o pipefail\nfail=0\nnote(){ :; }\nbad(){ fail=1; }\n" + text + "\n" + command
            p = subprocess.run(['bash', '-c', body], env=env, capture_output=True, text=True, timeout=10)
            rows = [json.loads(line) for line in trace.read_text().splitlines()] if trace.exists() else []
            return p, rows

    def test_pass_knobs_reach_compiler_not_program(self):
        for script in ('check_affine_loop_fold.sh', 'check_affine_exit_compare.sh'):
            with self.subTest(script=script):
                p, rows = self.invoke('run_case', 'run_case probe "-O2" "CCC_LOOP_ROTATE=1 CCC_NO_AFFINE_EXIT_FOLD=1"; exit "$fail"', script=script)
                self.assertEqual(p.returncode, 0, p.stderr)
                self.assertEqual(rows, [
                    {'phase': 'compile', 'env': {'CCC_LOOP_ROTATE': '1', 'CCC_NO_AFFINE_EXIT_FOLD': '1'}},
                    {'phase': 'run', 'rotate': None}])

    def test_compile_and_runtime_failures_are_not_parity(self):
        for extra, count in [({'FAIL_BUILD': '1'}, 1), ({'FAIL_RUN': '11'}, 2)]:
            p, rows = self.invoke('run_case', 'run_case probe "-O2" ""; exit "$fail"', extra)
            self.assertEqual(p.returncode, 1, p.stderr)
            self.assertEqual(len(rows), count)

    def test_successful_report_is_counted(self):
        p, rows = self.invoke('count_reports', 'count_reports CCC_LOOP_ROTATE=1')
        self.assertEqual(p.returncode, 0)
        self.assertEqual(p.stdout.strip(), '1')
        self.assertEqual(rows[0]['env']['CCC_LOOP_ROTATE'], '1')

    def test_report_failure_is_not_zero_folds(self):
        p, _ = self.invoke('count_reports', 'count_reports CCC_NO_AFFINE_EXIT_FOLD=1', {'FAIL_BUILD': '1'})
        self.assertNotEqual(p.returncode, 0)
        self.assertIn('forced compiler failure', p.stderr)


if __name__ == '__main__':
    unittest.main()
