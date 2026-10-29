"""Fail-closed wiring contracts for the artifact-sharing CI DAG.

The pipeline is a fan-out from one producing job to many lanes that restore the
artifact the producer published.  Every lane is a *consumer* in exactly the same
sense -- a shard is not a special case, it is a job that `needs` the build,
downloads the one artifact, and verifies the sha256 and the commit before it
runs anything -- so this file describes the DAG it actually has:

* `build` produces exactly one artifact, named by the commit;
* every consumer restores that artifact, verifies it, and never compiles the
  compiler itself (a consumer that rebuilt would be validating a different
  compiler than the one that shipped);
* the reusable benchmark workflow is called by both shards, is called *only*
  through `workflow_call`, and keeps its own verification inside the callee so
  a shard cannot skip it;
* `test` aggregates every lane, runs with `always()`, and fails when any lane
  did not succeed.

The sharded lanes are pinned by name.  A shard that silently disappears (or a
lane that is renamed into one) is the failure this file exists to catch -- the
coverage audit `tests/regression/check_ci_lane_plan.py` proves the shards are a
partition of the gates, and this one proves the DAG still wires them.
"""
from copy import deepcopy
from pathlib import Path
import importlib.util
import re
import tempfile
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

# Lanes that restore the artifact instead of compiling their own compiler.
CONSUMERS = {
    'corpus', 'unit', 'backend',
    'frontend-a', 'frontend-b', 'frontend-c',
    'codegen-a', 'codegen-b',
    'backend-contracts-a', 'backend-contracts-b',
}
# Jobs that run gates without being artifact consumers.
PRODUCERS = {'build', 'debug', 'clippy', 'msrv', 'icx-oracle'}
# Both reusable benchmark shards keep the same shape as `benchmark`.
BENCH_SHARDS = {'benchmark-a', 'benchmark-b'}
LANES = CONSUMERS | PRODUCERS | BENCH_SHARDS | {'benchmark'}


def validate(workflow, benchmark):
    jobs = workflow['jobs']
    assert set(jobs) == LANES | {'test'}
    aggregate = jobs['test']
    # The aggregate needs the MERGE job, not the shards: the merge job already
    # needs both shards, so listing them twice would only hide a broken edge.
    assert set(aggregate['needs']) == LANES - BENCH_SHARDS
    assert aggregate['if'] == 'always()'
    assert not aggregate.get('continue-on-error', False)
    final = aggregate['steps'][0]['run']
    assert "row['result'] != 'success'" in final and 'assert results and not bad' in final
    producer = jobs['build']['steps']
    uploads = [s for s in producer if s.get('uses', '').startswith('actions/upload-artifact@')]
    assert len(uploads) == 1
    name = uploads[0]['with']['name']
    assert '${{ github.sha }}' in name
    # The unit tests do not run in the producing job: `build` compiles and
    # packages, and the `unit` lane restores the artifact and runs them. What
    # must hold across that split is that the tests execute under the RUSTFLAGS
    # the build recorded, not under whatever the lane happens to inherit --
    # otherwise the suite validates a different configuration than the one
    # shipped in the artifact.
    test_run = next((s['run'] for s in jobs['unit']['steps']
                     if s.get('name') == 'Run tests'), None)
    assert test_run is not None, "the unit lane lost its 'Run tests' step"
    assert 'export RUSTFLAGS="$(cat target/lccc-rustflags)"' in test_run
    # Every benchmark shard is a reusable-workflow edge onto the same file,
    # depends on the build, and hands the callee its own shard identity.
    for shard in BENCH_SHARDS:
        edge = jobs[shard]
        assert edge['uses'] == './.github/workflows/bench.yml', shard
        assert edge['needs'] == ['build'], shard
        assert not edge.get('continue-on-error', False), shard
        assert edge['with']['shard'] == shard.rsplit('-', 1)[1], shard
        assert edge['with']['artifact'] == f'benchmark-shard-{edge["with"]["shard"]}', shard
    # The merge job is a normal job that consumes both shards' artifacts.
    merge = jobs['benchmark']
    assert merge['needs'] == sorted(BENCH_SHARDS, key=list(jobs).index), merge['needs']
    assert 'uses' not in merge
    merge_run = '\n'.join(s.get('run', '') for s in merge['steps'])
    assert '--merge' in merge_run and 'bench-results.json' in merge_run
    assert any(s.get('uses', '').startswith('actions/download-artifact@')
               for s in merge['steps'])
    # PyYAML YAML1.1 reads bare 'on' as True, unlike Actions' YAML1.2.
    assert set(benchmark.get('on', benchmark.get(True))) == {'workflow_call'}
    assert 'concurrency' not in benchmark, 'caller owns cancellation'
    bench_job = benchmark['jobs']['benchmark']
    assert bench_job['runs-on'] == jobs['build']['runs-on']
    bench_scripts = '\n'.join(s.get('run', '') for s in bench_job['steps'])
    assert '--reps 9' in bench_scripts and '--strict' in bench_scripts
    assert '--compilers lccc,gcc' in bench_scripts and '--gcc gcc-16' in bench_scripts
    # The corpus selection is an INPUT of the reusable workflow, so a shard
    # cannot fall back to "everything" and quietly double the corpus: the
    # callee must consume `inputs.only`, and the caller must pass it.
    assert '--only "$ONLY"' in bench_scripts, 'shard selection must come from the caller'
    assert any('inputs.only' in str(step.get('env', {})) for step in bench_job['steps'])
    # ... and it is the ONLY way a selection is spelled: a second, literal
    # `--only` would turn a shard into an unrelated partial corpus.
    for line in bench_scripts.splitlines():
        if line.lstrip().startswith('#'):
            continue
        if '--only' in line:
            assert '--only "$ONLY"' in line, line
    # The callee verifies the artifact it runs, so a shard cannot skip the
    # identity check the caller's DAG test would otherwise pass for it.
    assert 'sha256sum -c target/ci-build.sha256' in bench_scripts
    assert 'git rev-parse HEAD' in bench_scripts and 'target/ci-build.commit' in bench_scripts
    assert 'build_lccc_fast.sh' not in bench_scripts and 'cargo build' not in bench_scripts
    assert any(s.get('uses', '').startswith('actions/upload-artifact@') and s.get('if') == 'always()' for s in bench_job['steps'])
    for lane in CONSUMERS:
        job = jobs[lane]
        assert job['needs'] == ['build'], lane
        downloads = [s for s in job['steps'] if s.get('uses', '').startswith('actions/download-artifact@')]
        assert len(downloads) == 1 and downloads[0]['with']['name'] == name, lane
        scripts = '\n'.join(s.get('run', '') for s in job['steps'])
        assert 'sha256sum -c target/ci-build.sha256' in scripts, lane
        assert 'git rev-parse HEAD' in scripts and 'target/ci-build.commit' in scripts, lane
        assert 'build_lccc_fast.sh' not in scripts and 'cargo build' not in scripts, lane
    debug = '\n'.join(s.get('run', '') for s in jobs['debug']['steps'])
    assert debug.count('profile.fastbuild.debug-assertions=true') == 2
    assert '--gcc gcc-16' in debug
    assert not jobs['debug'].get('needs')
    icx = jobs['icx-oracle']
    assert icx['needs'] == ['build']
    icx_steps = '\n'.join(s.get('run', '') for s in icx['steps'])
    assert 'ensure_icx_2026.sh' in icx_steps
    assert any('LCCC_TEST_REAL_ICX' in s.get('env', {}) for s in icx['steps'])
    assert any(s.get('uses', '').startswith('actions/upload-artifact@') and s.get('if') == 'always()' for s in icx['steps'])


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
        for lane in ('debug', 'benchmark', 'frontend-b', 'frontend-c',
                     'backend-contracts-a'):
            with self.subTest(lane=lane):
                changed = deepcopy(self.workflow)
                changed['jobs']['test']['needs'] = [
                    name for name in changed['jobs']['test']['needs'] if name != lane]
                with self.assertRaises(AssertionError):
                    validate(changed, self.benchmark)

    def test_consumer_without_producer_rejected(self):
        for lane in ('codegen-a', 'frontend-b', 'frontend-c', 'backend'):
            with self.subTest(lane=lane):
                changed = deepcopy(self.workflow)
                changed['jobs'][lane]['needs'] = []
                with self.assertRaises(AssertionError):
                    validate(changed, self.benchmark)

    def test_unverified_artifact_rejected(self):
        for lane in ('frontend-a', 'frontend-c', 'codegen-b',
                     'backend-contracts-a'):
            with self.subTest(lane=lane):
                changed = deepcopy(self.workflow)
                for step in changed['jobs'][lane]['steps']:
                    if 'run' in step:
                        step['run'] = step['run'].replace(
                            'sha256sum -c target/ci-build.sha256', 'true')
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

    def test_shard_identity_must_match_its_artifact(self):
        # Two shards uploading the same artifact name overwrite each other, and
        # the merge job then publishes half the corpus as "complete" -- exactly
        # the failure `merge_reports` cannot see, because the report it reads is
        # internally consistent.
        changed = deepcopy(self.workflow)
        changed['jobs']['benchmark-b']['with']['artifact'] = 'benchmark-shard-a'
        with self.assertRaises(AssertionError):
            validate(changed, self.benchmark)

    def test_research_path_triggers_and_profiles_are_complete(self):
        research = yaml.load((ROOT / '.github/workflows/compiler-research.yml').read_text(),
                             Loader=yaml.BaseLoader)
        self.assertEqual(set(research['on']), {'workflow_dispatch', 'pull_request'})
        self.assertEqual(research['on']['pull_request'], {
            'branches': ['main'],
            'paths': [
                '.github/workflows/compiler-research.yml',
                'src/passes/vec_ssa.rs',
                'src/passes/slp_vectorizer.rs',
                'src/passes/vectorize.rs',
                'src/passes/byte_load.rs',
                'src/passes/join_store_forward.rs',
                'src/backend/global_alignment.rs',
                'src/backend/x86/codegen/peephole/passes/self_zext.rs',
                'scripts/publish_cpu_runtime_evidence.py',
                'include/emmintrin.h',
                'include/avxintrin.h',
                'tests/regression/native_vector_bitcasts.c',
                'tests/regression/check_native_vector_bitcasts.py',
            ],
        })
        cases = research['jobs']['evidence']['env']['RESEARCH_CASES']
        alternatives = [set(value.split(',')) for value in re.findall(r"'([a-z0-9_,]+)'", cases)
                        if ',' in value]
        self.assertEqual(len(alternatives), 2, 'PR and manual fallback case sets')
        pr_cases, manual_cases = alternatives
        fp_cases = {'f32_sse1_map', 'fp32_product_map', 'fp64_product_map'}
        self.assertTrue(fp_cases <= pr_cases)
        self.assertTrue(fp_cases <= manual_cases)
        self.assertTrue({'zlib_ng_slide_hash', 'linux_list_sort', 'sqlite_vdbe_serial',
                         'byte_load_reassembly', 'zlib_ng_compare256', 'arith_loop',
                         'sqlite_varint', 'hot_web_loop', 'sha256_transform',
                         'glibc_strstr', 'binary_trees', 'expat_xml_scan', 'strlen_bench',
                         'loop_patterns', 'mandelbrot', 'aarch64_select_patterns', 'qsort'} <= pr_cases)
        # Five per notice plus two identities must fit the 50-notice budget.
        self.assertLessEqual(len(pr_cases), 20)
        manual_default = research['on']['workflow_dispatch']['inputs']['cases']['default']
        self.assertEqual(manual_cases, set(manual_default.split(',')))
        self.assertIn('&& \'9\'', research['jobs']['evidence']['env']['RESEARCH_ROUNDS'])
        ab = next(step for step in research['jobs']['evidence']['steps']
                  if step.get('name') == 'Same-compiler list-merge join-forwarding experiment')
        self.assertIn('CCC_DISABLE_PASSES=join_store_forward', ab['run'])
        self.assertIn('--reference join_off', ab['run'])
        self.assertIn('--only linux_list_sort', ab['run'])
        self.assertIn('identical_binary_to_reference',
                      (ROOT / 'scripts/benchmark_native_batched.py').read_text())
        self.assertIn('len(text.encode()) > 3900', ab['run'])
        controls = next(step for step in research['jobs']['evidence']['steps']
                        if step.get('name') == 'Isolate strided-load and string-inlining performance')
        self.assertIn('CCC_NO_STRIDED_REDUCE=1', controls['run'])
        self.assertIn('CCC_INLINE_SKIP=two_way_short_needle', controls['run'])
        self.assertIn('--reference disabled', controls['run'])
        self.assertIn('scripts/benchmark_native_batched.py', controls['run'])
        self.assertIn('scripts/callgrind_corpus.py', controls['run'])
        self.assertIn('--target-flags=-march=x86-64-v3', controls['run'])
        zext = next(step for step in research['jobs']['evidence']['steps']
                    if step.get('name') == 'Same-compiler fallthrough zero-extension ablation')
        self.assertIn('CCC_NO_SELF_ZEXT_FALLTHROUGH=1', zext['run'])
        self.assertIn('--only zlib_ng_slide_hash', zext['run'])
        self.assertIn('callgrind-zext', zext['run'])
        self.assertIn('len(text.encode()) <= 3900', zext['run'])
        publisher = next(step for step in research['jobs']['evidence']['steps']
                         if step.get('name') == 'Publish CPU-specific paired results through the checks API')
        self.assertEqual(publisher.get('if'), 'always()')
        self.assertIn('--expected-cases "$RESEARCH_CASES"', publisher['run'])
        self.assertIn('--expected-compilers lccc,before,gcc,clang,icx,rolled', publisher['run'])
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

    def test_declared_inputs_resolve_like_github(self):
        """A `with:` edge is only walkable when the callee declares the input.

        The sharded benchmark lane passes four inputs, so the walker must
        resolve them the way GitHub does -- caller value, else declared
        default, else refuse -- or the gates behind `if: inputs.*` are
        silently unwalkable and "parity" would mean "we stopped looking".
        """
        with tempfile.TemporaryDirectory() as tmp:
            docs = self.documents()
            # One edge, so a required input has exactly one caller to satisfy.
            del docs['ci.yml']['jobs']['b']
            docs['middle.yml']['on'] = {'workflow_call': {'inputs': {
                'mode': {'type': 'string', 'default': 'strict'},
                'shard': {'type': 'string', 'required': True},
                'gate': {'type': 'boolean', 'default': True},
            }}}
            docs['ci.yml']['jobs']['a']['with'] = {'shard': 'a'}
            steps = self.walk(Path(tmp), docs)
            self.assertEqual(len(steps), 1)
            self.assertTrue(all(s.active and not s.soft for s in steps))

        for mutation in ('undeclared', 'missing-required', 'structured'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                docs = self.documents()
                del docs['ci.yml']['jobs']['b']
                docs['middle.yml']['on'] = {'workflow_call': {'inputs': {
                    'shard': {'type': 'string', 'required': True}}}}
                if mutation == 'undeclared':
                    docs['ci.yml']['jobs']['a']['with'] = {'mode': 'strict'}
                elif mutation == 'missing-required':
                    docs['ci.yml']['jobs']['a']['with'] = {}
                else:
                    docs['ci.yml']['jobs']['a']['with'] = {'shard': {'nested': 'map'}}
                with self.assertRaises(self.parity._WorkflowError):
                    self.walk(Path(tmp), docs)


class OracleRetirementPolicy(unittest.TestCase):
    def test_policy_pointer_and_named_scripts_exist(self):
        import re
        workflow=(ROOT/'.github/workflows/ci.yml').read_text()
        self.assertEqual(workflow.splitlines()[0],'# Oracle policy: engineering/CI-ORACLE-POLICY.md')
        policy=(ROOT/'engineering/CI-ORACLE-POLICY.md').read_text()
        named=re.findall(r'`((?:scripts|tests)/[^` ]+\.(?:py|sh))`',policy)
        self.assertGreaterEqual(len(named),4)
        for name in named:self.assertTrue((ROOT/name).is_file(),name)
