"""The select-fusion gate must disable the compiler pass, not the executable.

Uses a compiler protocol stub: no C compiler, assembler or linker is invoked.
The negative test refuses a purported kill switch that leaves fused assembly.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
MOCK = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[1:]
# The gate pins its oracle to GCC 14 by major version (tests/regression/lib/
# gcc_oracle.sh), so the stub answers the version query like GCC does. The
# query is not a compile, so it is not logged.
if '-dumpfullversion' in args:
    print(os.environ.get('MOCK_GCC_VERSION', '14.2.0'))
    sys.exit(0)
with open(os.environ['MOCK_LOG'], 'a') as log:
    log.write(json.dumps({'args': args, 'skip': os.environ.get('CCC_PEEPHOLE_SKIP', '')}) + '\n')
out = pathlib.Path(args[args.index('-o') + 1])
if '-S' in args:
    if any('ascii_case_fold.c' in a for a in args):
        disabled = os.environ.get('CCC_PEEPHOLE_SKIP') == 'setcc_cmov'
        if os.environ.get('MOCK_IGNORE_SKIP'):
            disabled = False
        body = '    setbe %al\n    testl %eax, %eax\n    cmovnel %edx, %ecx\n' if disabled else '    cmovbel %edx, %ecx\n'
        out.write_text('main:\n' + body + '.size main, .-main\n')
    else:
        out.write_text('bool_kept:\n    setg %al\n.size bool_kept, .-bool_kept\n')
else:
    out.write_text('#!/bin/sh\nprintf "verified\\n"\n')
    out.chmod(0o755)
'''


class SelectCompareControlTests(unittest.TestCase):
    def invoke(self, ignore=False, gcc_version=None):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            compiler = directory / 'compiler protocol stub'
            compiler.write_text(MOCK)
            compiler.chmod(0o755)
            log = directory / 'calls.jsonl'
            env = dict(os.environ, CCC=str(compiler), GCC=str(compiler),
                       MOCK_LOG=str(log))
            env.pop('CCC_PEEPHOLE_SKIP', None)
            env.pop('MOCK_IGNORE_SKIP', None)
            if ignore:
                env['MOCK_IGNORE_SKIP'] = '1'
            if gcc_version is not None:
                env['MOCK_GCC_VERSION'] = gcc_version
            result = subprocess.run(
                ['bash', str(ROOT / 'tests/regression/check_select_from_compare.sh')],
                env=env, text=True, capture_output=True, timeout=30)
            text = log.read_text() if log.exists() else ''
            return result, [json.loads(line) for line in text.splitlines()]

    def test_each_optimization_level_has_a_real_compile_time_control(self):
        result, calls = self.invoke()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for opt in ['-O1', '-O2', '-O3']:
            arms = [c for c in calls if opt in c['args'] and '-S' not in c['args']
                    and not c['args'][-1].endswith('/ref')]
            self.assertEqual(sorted(c['skip'] for c in arms), ['', 'setcc_cmov'])
        shapes = [c for c in calls if '-S' in c['args']
                  and any('ascii_case_fold.c' in a for a in c['args'])]
        self.assertEqual(sorted(c['skip'] for c in shapes), ['', 'setcc_cmov'])

    def test_gcc13_oracle_is_refused_before_any_compile(self):
        result, calls = self.invoke(gcc_version='13.3.0')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('the oracle is GCC 14', result.stdout + result.stderr)
        self.assertEqual(calls, [])

    def test_noop_kill_switch_cannot_pass(self):
        result, _ = self.invoke(ignore=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('disabled fusion did not retain', result.stderr)
        self.assertIn('disabled fusion still emitted', result.stderr)


if __name__ == '__main__':
    unittest.main()
