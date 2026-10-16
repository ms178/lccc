"""Fail-closed wiring contracts for the artifact-sharing CI DAG."""
from copy import deepcopy
from pathlib import Path
import importlib.util
import tempfile
import unittest
import yaml

ROOT = Path(__file__).resolve().parents[2]
CONSUMERS = {'corpus', 'frontend', 'codegen', 'backend'}
LANES = CONSUMERS | {'build', 'debug', 'clippy', 'msrv', 'benchmark'}


def validate(workflow, benchmark):
    jobs = workflow['jobs']
    assert set(jobs) == LANES | {'test'}
    aggregate = jobs['test']
    assert set(aggregate['needs']) == LANES
    assert aggregate['if'] == 'always()'
    assert not aggregate.get('continue-on-error', False)
    final = aggregate['steps'][0]['run']
    assert "row['result'] != 'success'" in final and 'assert results and not bad' in final
    producer = jobs['build']['steps']
    uploads = [s for s in producer if s.get('uses', '').startswith('actions/upload-artifact@')]
    assert len(uploads) == 1
    name = uploads[0]['with']['name']
    assert '${{ github.sha }}' in name
    test_run = next(s['run'] for s in producer if s.get('name') == 'Run tests')
    assert 'export RUSTFLAGS="$(cat target/lccc-rustflags)"' in test_run
    reusable = jobs['benchmark']
    assert reusable['needs'] == ['build']
    assert reusable['uses'] == './.github/workflows/bench.yml'
    assert not reusable.get('continue-on-error', False)
    # PyYAML YAML1.1 reads bare 'on' as True, unlike Actions' YAML1.2.
    assert set(benchmark.get('on', benchmark.get(True))) == {'workflow_call'}
    assert 'concurrency' not in benchmark, 'caller owns cancellation'
    bench_job = benchmark['jobs']['benchmark']
    assert bench_job['runs-on'] == jobs['build']['runs-on']
    bench_scripts = '\n'.join(s.get('run', '') for s in bench_job['steps'])
    assert '--reps 9' in bench_scripts and '--strict' in bench_scripts
    assert '--only' not in '\n'.join(line for line in bench_scripts.splitlines() if not line.lstrip().startswith('#'))
    assert '--compilers lccc,gcc' in bench_scripts and '--gcc gcc-16' in bench_scripts
    assert any(s.get('uses', '').startswith('actions/upload-artifact@') and s.get('if') == 'always()' for s in bench_job['steps'])
    for lane in CONSUMERS | {'benchmark'}:
        job = jobs[lane] if lane != 'benchmark' else dict(bench_job, needs=['build'])
        assert job['needs'] == ['build']
        downloads = [s for s in job['steps'] if s.get('uses', '').startswith('actions/download-artifact@')]
        assert len(downloads) == 1 and downloads[0]['with']['name'] == name
        scripts = '\n'.join(s.get('run', '') for s in job['steps'])
        assert 'sha256sum -c target/ci-build.sha256' in scripts
        assert 'git rev-parse HEAD' in scripts and 'target/ci-build.commit' in scripts
        assert 'build_lccc_fast.sh' not in scripts and 'cargo build' not in scripts
    debug = '\n'.join(s.get('run', '') for s in jobs['debug']['steps'])
    assert debug.count('profile.fastbuild.debug-assertions=true') == 2
    assert '--gcc gcc-16' in debug
    assert not jobs['debug'].get('needs')


class ReusableReachability(unittest.TestCase):
    def test_call_graph_semantics(self):
        import sys
        spec = importlib.util.spec_from_file_location('pr832_parity', ROOT/'scripts/check_ci_gate_parity.py')
        parity = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = parity
        spec.loader.exec_module(parity)
        for mutation in ('valid', 'disabled', 'soft', 'schedule', 'uncalled', 'cycle', 'missing', 'no_call_trigger'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp); parent=root/'ci.yml'; child=root/'bench.yml'
                caller={'on':'pull_request','jobs':{'bench':{'uses':'./.github/workflows/bench.yml'}}}
                callee={'on':'workflow_call','jobs':{'measure':{'steps':[{'run':'python3 scripts/probe.py'}]}}}
                if mutation=='disabled': caller['jobs']['bench']['if']='false'
                elif mutation=='soft': caller['jobs']['bench']['continue-on-error']=True
                elif mutation=='schedule': caller['on']='schedule'
                elif mutation=='uncalled': caller['jobs']={}
                elif mutation=='cycle': callee['jobs']['back']={'uses':'./.github/workflows/bench.yml'}
                elif mutation=='no_call_trigger': callee['on']='workflow_dispatch'
                parent.write_text(yaml.safe_dump(caller))
                if mutation!='missing': child.write_text(yaml.safe_dump(callee))
                if mutation in ('cycle','missing','no_call_trigger'):
                    with self.assertRaises(parity._WorkflowError): list(parity.workflow_run_steps(parent))
                else:
                    steps=list(parity.workflow_run_steps(parent))
                    active=[s for s in steps if s.active and not s.soft]
                    self.assertEqual(len(active), 1 if mutation=='valid' else 0)
                    if child.exists(): self.assertFalse(any(s.active for s in parity.workflow_run_steps(child)))


class ParallelLayout(unittest.TestCase):
    def setUp(self):
        self.workflow = yaml.safe_load((ROOT / '.github/workflows/ci.yml').read_text())
        self.benchmark = yaml.safe_load((ROOT / '.github/workflows/bench.yml').read_text())

    def test_actual_layout(self):
        validate(self.workflow, self.benchmark)

    def test_missing_lane_dependency_rejected(self):
        changed = deepcopy(self.workflow)
        changed['jobs']['test']['needs'].remove('debug')
        with self.assertRaises(AssertionError):
            validate(changed, self.benchmark)

    def test_consumer_without_producer_rejected(self):
        changed = deepcopy(self.workflow)
        changed['jobs']['codegen']['needs'] = []
        with self.assertRaises(AssertionError):
            validate(changed, self.benchmark)

    def test_unverified_artifact_rejected(self):
        changed = deepcopy(self.workflow)
        for step in changed['jobs']['frontend']['steps']:
            if 'run' in step:
                step['run'] = step['run'].replace('sha256sum -c target/ci-build.sha256', 'true')
        with self.assertRaises(AssertionError):
            validate(changed, self.benchmark)

    def test_duplicate_benchmark_trigger_rejected(self):
        changed = deepcopy(self.benchmark)
        changed.get('on', changed.get(True))['pull_request'] = {}
        with self.assertRaises(AssertionError):
            validate(self.workflow, changed)

    def test_benchmark_rebuild_or_lost_verification_rejected(self):
        for mutation in ('rebuild', 'hash', 'commit', 'coverage'):
            with self.subTest(mutation=mutation):
                changed = deepcopy(self.benchmark)
                steps = changed['jobs']['benchmark']['steps']
                if mutation == 'rebuild': steps.append({'run':'cargo build'})
                elif mutation == 'coverage': steps.append({'run':'true --only fib'})
                else:
                    for step in steps:
                        if 'run' in step:
                            target = 'sha256sum -c target/ci-build.sha256' if mutation == 'hash' else 'git rev-parse HEAD'
                            step['run'] = step['run'].replace(target, 'true')
                with self.assertRaises(AssertionError):
                    validate(self.workflow, changed)

    def test_research_does_not_add_automatic_pr_builds(self):
        research = yaml.load((ROOT / '.github/workflows/compiler-research.yml').read_text(),
                             Loader=yaml.BaseLoader)
        self.assertEqual(set(research['on']), {'workflow_dispatch'})
        self.assertTrue(research['jobs'])
        for job in research['jobs'].values():
            self.assertNotIn('continue-on-error', job)
            self.assertNotIn('pull_request.number', job.get('if', ''))
        redteam = yaml.safe_load((ROOT / '.github/workflows/pr826-redteam.yml').read_text())
        self.assertNotIn('causal-comparison', redteam['jobs'])
        self.assertTrue(redteam['jobs'], 'unrelated red-team checks must survive')


class RecursiveWorkflowSemantics(unittest.TestCase):
    def setUp(self):
        import sys
        spec=importlib.util.spec_from_file_location('pr834_parity',ROOT/'scripts/check_ci_gate_parity.py')
        self.parity=importlib.util.module_from_spec(spec);sys.modules[spec.name]=self.parity;spec.loader.exec_module(self.parity)

    def walk(self,root,documents):
        for name,document in documents.items():(root/name).write_text(yaml.safe_dump(document))
        return list(self.parity.workflow_run_steps(root/'ci.yml'))

    def documents(self):
        return {
            'ci.yml':{'on':'pull_request','jobs':{'a':{'uses':'./.github/workflows/middle.yml'},'b':{'uses':'./.github/workflows/middle.yml'}}},
            'middle.yml':{'on':'workflow_call','jobs':{'next':{'uses':'./.github/workflows/leaf.yml'}}},
            'leaf.yml':{'on':'workflow_call','jobs':{'gate':{'steps':[{'run':'python3 scripts/probe.py'}]}}},
        }

    def test_nested_diamond_and_string_boolean_softness(self):
        for value,expected in ((False,2),('false',2),('${{ false }}',2),(True,0),('true',0)):
            with self.subTest(value=value),tempfile.TemporaryDirectory() as tmp:
                docs=self.documents()
                docs['middle.yml']['jobs']['next']['continue-on-error']=value
                steps=self.walk(Path(tmp),docs)
                self.assertEqual(len(steps),2,'diamond is not a recursive cycle')
                self.assertEqual(sum(s.active and not s.soft for s in steps),expected)
        with tempfile.TemporaryDirectory() as tmp:
            docs=self.documents();docs['leaf.yml']['jobs']['gate']['steps'][0]['continue-on-error']='false'
            self.assertTrue(all(s.active and not s.soft for s in self.walk(Path(tmp),docs)))

    def test_needs_skips_propagate_but_always_can_run(self):
        for always in (False,True):
            with self.subTest(always=always),tempfile.TemporaryDirectory() as tmp:
                docs=self.documents();jobs=docs['ci.yml']['jobs']
                jobs['prep']={'if':'false','steps':[{'run':'echo skipped'}]}
                jobs['a']['needs']='prep';jobs['b']['needs']=['a']
                if always:jobs['b']['if']='always()'
                steps=self.walk(Path(tmp),docs)
                gates=[s for s in steps if 'probe.py' in s.body]
                self.assertEqual(sum(s.active and not s.soft for s in gates),int(always))

    def test_always_intermediate_does_not_erase_a_skipped_ancestor(self):
        for after_always in (False,True):
            with self.subTest(after_always=after_always),tempfile.TemporaryDirectory() as tmp:
                docs=self.documents();jobs=docs['ci.yml']['jobs']
                jobs['prep']={'if':'false','steps':[{'run':'echo skipped'}]}
                jobs['a']['needs']='prep'
                jobs['b'].update(needs='a',**{'if':'always()'})
                jobs['after']={'needs':'b','uses':'./.github/workflows/leaf.yml'}
                if after_always:jobs['after']['if']='always()'
                gates=[s for s in self.walk(Path(tmp),docs) if 'probe.py' in s.body]
                self.assertEqual(sum(s.active and not s.soft for s in gates),1+int(after_always))

    def test_unsupported_semantics_and_bad_needs_rejected(self):
        for mutation in ('matrix','required-input','required-secret','with','secrets','expression-soft','unknown-needs','cycle-needs'):
            with self.subTest(mutation=mutation),tempfile.TemporaryDirectory() as tmp:
                docs=self.documents();job=docs['ci.yml']['jobs']['a']
                if mutation=='matrix':job['strategy']={'matrix':{'os':['ubuntu-latest']}}
                elif mutation=='required-input':docs['middle.yml']['on']={'workflow_call':{'inputs':{'mode':{'required':True,'type':'string'}}}}
                elif mutation=='required-secret':docs['middle.yml']['on']={'workflow_call':{'secrets':{'token':{'required':True}}}}
                elif mutation=='with':job['with']={'mode':'strict'}
                elif mutation=='secrets':job['secrets']='inherit'
                elif mutation=='expression-soft':job['continue-on-error']='${{ inputs.soft }}'
                elif mutation=='unknown-needs':job['needs']='missing'
                elif mutation=='cycle-needs':job['needs']='b';docs['ci.yml']['jobs']['b']['needs']='a'
                with self.assertRaises(self.parity._WorkflowError):self.walk(Path(tmp),docs)


class OracleRetirementPolicy(unittest.TestCase):
    def test_policy_pointer_and_named_scripts_exist(self):
        import re
        workflow=(ROOT/'.github/workflows/ci.yml').read_text()
        self.assertEqual(workflow.splitlines()[0],'# Oracle policy: engineering/CI-ORACLE-POLICY.md')
        policy=(ROOT/'engineering/CI-ORACLE-POLICY.md').read_text()
        named=re.findall(r'`((?:scripts|tests)/[^` ]+\.(?:py|sh))`',policy)
        self.assertGreaterEqual(len(named),4)
        for name in named:self.assertTrue((ROOT/name).is_file(),name)
