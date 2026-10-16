"""Live CE is isolated from push/PR gates; scheduled/manual research stays strict."""
from copy import deepcopy
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


def validate_workflow(workflow, name):
    invocations = [tool for job in workflow.get('jobs', {}).values()
                   for step in job.get('steps', [])
                   for tool in live_invocations(step.get('run', ''))]
    if name != 'oracle-delta.yml':
        assert not invocations, 'live CE in an ordinary workflow'
        return
    assert set(workflow['on']) == {'schedule', 'workflow_dispatch'}
    assert workflow['on']['schedule'], 'research schedule must not be empty'
    assert invocations == ['oracle_delta_gate'], 'one explicit live delta survey'
    jobs = workflow['jobs']
    assert set(jobs) == {'oracle-delta'}
    job = jobs['oracle-delta']
    assert not job.get('continue-on-error'), 'outages must be visible research failures'
    steps = job['steps']
    live = next(step for step in steps if live_invocations(step.get('run', '')))
    argv = shlex.split(live['run'])
    assert '--strict' in argv and '--report' in argv
    assert not live.get('continue-on-error')
    assert '||' not in argv, 'do not hide a research producer failure'
    assert any(step.get('uses', '').startswith('actions/upload-artifact@')
               and step.get('if') == 'always()' for step in steps)


class NoLiveGodboltCI(unittest.TestCase):
    def test_live_oracles_are_isolated_from_required_ci(self):
        for path in (ROOT / '.github/workflows').glob('*.yml'):
            workflow = yaml.load(path.read_text(), Loader=yaml.BaseLoader)
            with self.subTest(workflow=path.name):
                validate_workflow(workflow, path.name)

    def test_research_cannot_become_an_automatic_pr_gate(self):
        original = yaml.load((ROOT / '.github/workflows/oracle-delta.yml').read_text(),
                             Loader=yaml.BaseLoader)
        for event in ('push', 'pull_request', 'pull_request_target', 'workflow_call'):
            changed = deepcopy(original)
            changed['on'][event] = {}
            with self.subTest(event=event), self.assertRaises(AssertionError):
                validate_workflow(changed, 'oracle-delta.yml')

    def test_research_failures_and_missing_artifacts_are_not_silenced(self):
        original = yaml.load((ROOT / '.github/workflows/oracle-delta.yml').read_text(),
                             Loader=yaml.BaseLoader)
        for mutation in ('non-strict', 'allowed-failure', 'lost-artifact'):
            changed = deepcopy(original)
            job = changed['jobs']['oracle-delta']
            if mutation == 'non-strict':
                for step in job['steps']:
                    if 'run' in step: step['run'] = step['run'].replace('--strict', '')
            elif mutation == 'allowed-failure':
                job['continue-on-error'] = 'true'
            else:
                for step in job['steps']: step.pop('if', None)
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                validate_workflow(changed, 'oracle-delta.yml')

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
