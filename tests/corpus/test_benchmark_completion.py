"""The real CLI producer must satisfy the report consumer's completion contract."""
from contextlib import redirect_stdout, redirect_stderr
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tests.benchmark import run_benchmarks as bench


class BenchmarkCompletion(unittest.TestCase):
    def invoke(self, root, *, ok=True, interrupted=False, partial=False):
        path = root/'report.json'
        name = bench.BENCHMARKS[0].name
        row = {'name': name, 'compile': {'gcc': {'ok': ok}},
               'runs': {'gcc': [{'ok': ok}]},
               'correctness': {'gcc': {'matches_baseline': ok}}, 'warnings': []}
        case = mock.Mock(return_value=row, side_effect=([row, RuntimeError('interrupted')] if partial
                                                      else RuntimeError('interrupted') if interrupted else None))
        selected = name + ',' + bench.BENCHMARKS[1].name if partial else name
        with mock.patch.object(sys, 'argv', ['benchmark', '--compilers=gcc', '--only='+selected,
                                           '--skip-perf', '--strict', '--json='+str(path)]), \
             mock.patch.multiple(bench,
                 discover_compilers=mock.Mock(return_value=([bench.CompilerSpec('gcc','GCC','/nonexistent-oracle',())], {})),
                 choose_affinity=mock.Mock(return_value=([], {'applied': False}, [])),
                 select_work_root=mock.Mock(return_value=(str(root), {})),
                 probe_pmu=mock.Mock(return_value={}),
                 collect_environment=mock.Mock(return_value={}),
                 run_benchmark=case,
                 aggregate_lccc_vs_gcc=mock.Mock(return_value={}),
                 aggregate_lccc_vs_best_reference=mock.Mock(return_value={}),
                 render_terminal=mock.Mock(return_value='test')), \
             redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            return bench.main()

    def test_finished_cli_marks_completion_without_certifying_wrong_results(self):
        for ok in (True, False):
            with self.subTest(ok=ok), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                self.assertEqual(self.invoke(root, ok=ok), 0 if ok else 1)
                data = json.loads((root/'report.json').read_text())
                self.assertIs(data.get('complete'), True)
                self.assertEqual(data['benchmarks'][0]['compile']['gcc']['ok'], ok)

    def test_interrupted_cli_cannot_publish_a_complete_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaisesRegex(RuntimeError, 'interrupted'):
                self.invoke(root, interrupted=True)
            self.assertIs(json.loads((root/'report.json').read_text())['complete'], False)

    def test_interruption_invalidates_an_old_success_receipt(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root/'report.json').write_text('{"complete": true, "benchmarks": ["stale"]}')
            with self.assertRaisesRegex(RuntimeError, 'interrupted'):
                self.invoke(root, interrupted=True)
            data = json.loads((root/'report.json').read_text())
            self.assertIs(data['complete'], False)
            self.assertEqual(data['benchmarks'], [])

    def test_completed_cases_survive_a_later_interruption(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaisesRegex(RuntimeError, 'interrupted'):
                self.invoke(root, partial=True)
            data = json.loads((root/'report.json').read_text())
            self.assertIs(data['complete'], False)
            self.assertEqual([row['name'] for row in data['benchmarks']], [bench.BENCHMARKS[0].name])
