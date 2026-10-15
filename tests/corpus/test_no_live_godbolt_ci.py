"""Live CE research is not a GitHub CI gate; offline helper tests remain."""
from pathlib import Path
import re
import shlex
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]
LIVE_TOOLS = re.compile(r'\bscripts/(godbolt|codegen_oracle|benchmark_codegen|loop_density_oracle|tight_loop_oracle|tune_oracle|oracle_delta_gate)\.py\b([^\n]*)')


def live_invocations(run):
    run = run.replace('\\\n', ' ')
    found = []
    for match in LIVE_TOOLS.finditer(run):
        tool, tail = match.groups()
        tokens = shlex.split(tail, comments=True)
        if tool == 'oracle_delta_gate' and tokens == ['--self-test']:
            continue
        if tool == 'loop_density_oracle' and '--local-only' in tokens:
            continue
        empty_oracles = '--oracles=' in tokens or any(
            a == '--oracles' and b == '' for a, b in zip(tokens, tokens[1:]))
        if tool in {'benchmark_codegen', 'codegen_oracle'} and empty_oracles:
            continue
        found.append(tool)
    return found


class NoLiveGodboltCI(unittest.TestCase):
    def test_actual_workflows_do_not_execute_live_oracles(self):
        for path in (ROOT / '.github/workflows').glob('*.yml'):
            workflow = yaml.load(path.read_text(), Loader=yaml.BaseLoader)
            with self.subTest(workflow=path.name):
                self.assertNotIn('oracle-delta', workflow.get('jobs', {}))
                for job in workflow.get('jobs', {}).values():
                    for step in job.get('steps', []):
                        self.assertEqual(live_invocations(step.get('run', '')), [])

    def test_live_compiler_calls_fail_the_policy_even_in_a_step_with_tests(self):
        for tool in ('godbolt', 'codegen_oracle', 'benchmark_codegen', 'loop_density_oracle',
                     'tight_loop_oracle', 'tune_oracle', 'oracle_delta_gate'):
            with self.subTest(tool=tool):
                self.assertEqual(live_invocations('python3 scripts/test_godbolt_cache.py\n'
                                                 f'python3 scripts/{tool}.py --oracles gcc16.2'), [tool])

    def test_explicit_offline_modes_are_not_live_oracle_gates(self):
        for run in ('python3 scripts/oracle_delta_gate.py --self-test',
                    'python3 scripts/test_godbolt_cache.py',
                    "python3 scripts/benchmark_codegen.py --oracles '' --out results/local",
                    'python3 scripts/loop_density_oracle.py --local-only'):
            with self.subTest(run=run):
                self.assertEqual(live_invocations(run), [])


if __name__ == '__main__':
    unittest.main()
