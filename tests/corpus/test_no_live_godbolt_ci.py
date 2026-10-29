"""Live CE is isolated from push/PR gates; scheduled/manual research stays strict."""
from copy import deepcopy
from pathlib import Path
import re
import shlex
import unittest

try:
    import yaml
except ModuleNotFoundError as exc:  # pragma: no cover - host dependency
    # CI installs python3-yaml (scripts/ci_ubuntu_chroot.sh PACKAGES), so this
    # only fires on a developer box.  Importing the workflow contracts without a
    # parser is impossible, and failing during collection reports the missing
    # package as an unrelated test error; a skip states the real cause once.
    raise unittest.SkipTest(
        "PyYAML is required to read the CI workflow contracts "
        "(install python3-yaml, or `pip install pyyaml`)"
    ) from exc

ROOT = Path(__file__).resolve().parents[2]
LIVE_TOOLS = re.compile(r'\bscripts/(godbolt|codegen_oracle|benchmark_codegen|loop_density_oracle|tight_loop_oracle|tune_oracle|oracle_delta_gate)\.py\b')


def live_invocations(run):
    run = run.replace('\\\n', ' ')
    found = []
    matches = list(LIVE_TOOLS.finditer(run))
    for index, match in enumerate(matches):
        tool = match.group(1)
        line_end = run.find('\n', match.end())
        if line_end < 0:
            line_end = len(run)
        # Do not let a local command swallow a second live command on the
        # same shell line; inspect each script invocation independently.
        if index + 1 < len(matches):
            line_end = min(line_end, matches[index + 1].start())
        tokens = shlex.split(run[match.end():line_end], comments=True)
        if tool == 'oracle_delta_gate' and tokens == ['--self-test']:
            continue
        if tool == 'loop_density_oracle' and '--local-only' in tokens:
            continue
        # argparse permits repeated options and the final value wins.  Merely
        # finding one empty spelling is therefore not enough to prove offline.
        oracle_values = []
        for index, token in enumerate(tokens):
            if token == '--oracles':
                oracle_values.append(tokens[index + 1] if index + 1 < len(tokens) else None)
            elif token.startswith('--oracles='):
                oracle_values.append(token.partition('=')[2])
        empty_oracles = oracle_values == ['']
        if tool in {'benchmark_codegen', 'codegen_oracle'} and empty_oracles:
            continue
        if tool == 'godbolt' and tokens[:2] == ['compare', '--offline'] and empty_oracles:
            # The dispatcher enforces --offline before any catalog or local
            # oracle provisioning. Keep it immediately after the subcommand so
            # it cannot be consumed as the value of another option.
            refs = [tokens[i + 1] for i, token in enumerate(tokens[:-1])
                    if token == '--local-ref']
            refs += [token.partition('=')[2] for token in tokens
                     if token.startswith('--local-ref=')]
            if any('=' in ref and all(ref.partition('=')[::2]) for ref in refs):
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
                    'python3 scripts/loop_density_oracle.py --local-only',
                    "python3 scripts/godbolt.py compare --offline input.c --oracles '' --local-ref icx=/opt/icx",
                    "python3 scripts/godbolt.py compare --offline input.c --oracles= --local-ref=icx=/opt/icx"):
            with self.subTest(run=run):
                self.assertEqual(live_invocations(run), [])

    def test_godbolt_offline_exception_fails_closed(self):
        prefix = 'python3 scripts/godbolt.py '
        for invocation in (
            'compare input.c --local-ref icx=/opt/icx',  # defaults include remote
            'compare input.c --oracles icx2026.2',  # may provision from network
            "compare input.c --oracles '' --local-ref icx=/opt/icx",  # no runtime guard
            "compare --offline input.c --oracles ''",  # no actual comparator
            "compare --offline input.c --oracles '' --local-ref icx=",
            "compare --offline input.c --oracles '' --local-ref icx=/opt/icx --oracles gcc16.2",
            "compare --offline input.c --oracles gcc16.2 --oracles= --local-ref icx=/opt/icx",
            "compare input.c --flags --offline --oracles '' --local-ref icx=/opt/icx",
            "compile gcc16.2 input.c --oracles '' --local-ref icx=/opt/icx",
        ):
            with self.subTest(invocation=invocation):
                self.assertEqual(live_invocations(prefix + invocation), ['godbolt'])

    def test_local_command_cannot_hide_a_live_command_on_the_same_line(self):
        local = "python3 scripts/godbolt.py compare --offline in.c --oracles '' --local-ref x=/opt/cc"
        for separator in ('; ', ' && ', ' | '):
            for live in ('python3 scripts/godbolt.py compare remote.c',
                         'python3 scripts/codegen_oracle.py --oracles gcc16.2'):
                with self.subTest(separator=separator, live=live):
                    self.assertEqual(live_invocations(local + separator + live),
                                     ['godbolt' if 'godbolt.py' in live else 'codegen_oracle'])


if __name__ == '__main__':
    unittest.main()
