"""Fail-closed wiring contracts for the artifact-sharing CI DAG."""
from copy import deepcopy
from pathlib import Path
import unittest
import yaml

ROOT = Path(__file__).resolve().parents[2]
CONSUMERS = {'corpus', 'frontend', 'codegen', 'backend'}
LANES = CONSUMERS | {'build', 'debug', 'clippy', 'msrv'}


def validate(workflow):
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
    for lane in CONSUMERS:
        job = jobs[lane]
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


class ParallelLayout(unittest.TestCase):
    def setUp(self):
        self.workflow = yaml.safe_load((ROOT / '.github/workflows/ci.yml').read_text())

    def test_actual_layout(self):
        validate(self.workflow)

    def test_missing_lane_dependency_rejected(self):
        changed = deepcopy(self.workflow)
        changed['jobs']['test']['needs'].remove('debug')
        with self.assertRaises(AssertionError):
            validate(changed)

    def test_consumer_without_producer_rejected(self):
        changed = deepcopy(self.workflow)
        changed['jobs']['codegen']['needs'] = []
        with self.assertRaises(AssertionError):
            validate(changed)

    def test_unverified_artifact_rejected(self):
        changed = deepcopy(self.workflow)
        for step in changed['jobs']['frontend']['steps']:
            if 'run' in step:
                step['run'] = step['run'].replace('sha256sum -c target/ci-build.sha256', 'true')
        with self.assertRaises(AssertionError):
            validate(changed)

    def test_removed_research_gates_stay_off(self):
        self.assertFalse((ROOT / '.github/workflows/compiler-research.yml').exists())
        redteam = yaml.safe_load((ROOT / '.github/workflows/pr826-redteam.yml').read_text())
        self.assertNotIn('causal-comparison', redteam['jobs'])
        self.assertTrue(redteam['jobs'], 'unrelated red-team checks must survive')
