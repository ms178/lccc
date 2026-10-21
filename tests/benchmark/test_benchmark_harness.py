#!/usr/bin/env python3
"""Adversarial contracts for the benchmark harness (no compiler build).

run_benchmarks' oracle selection and correctness evaluation, the native
runner, paired timing (paired_ab) and Callgrind workload sizing. The output
gate itself is covered by tests/corpus/test_benchmark_output_gate.py.
"""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tests.benchmark import run_benchmarks as bench  # noqa: E402


def outcome(status=0, stdout=b'checksum\n', failure=None):
    return dict(returncode=status, stdout=stdout, stderr=b'', failure=failure)


class BenchmarkHarnessTests(unittest.TestCase):
    def test_instrumentation_merges_identity_with_explicit_size_overrides(self):
        sys.path.insert(0, str(ROOT / 'scripts'))
        import callgrind_corpus as corpus
        sizes = json.loads((ROOT / 'tests/benchmark/callgrind_sizes.json').read_text())
        by_name = {b.name: b for b in bench.BENCHMARKS}
        self.assertEqual(set(sizes), set(by_name), "every registered workload needs bounded instrumentation sizes")
        self.assertEqual(sizes['volatile_reduce'], '-DVOLATILE_REDUCE_TRIPS=4096')
        self.assertEqual(sizes['hot_web_loop'], '-DHOT_WEB_TRIPS=4096')
        self.assertEqual(sizes['llvm_batch6_scalar'], '-DBATCH6_SCALAR_TRIPS=1024')
        self.assertEqual(corpus.workload_flags(by_name['linux_find_bit_scaled'], sizes['linux_find_bit_scaled']), ['-DPASSES=128'])
        flags = corpus.workload_flags(by_name['lz4_match_extend'], sizes['lz4_match_extend'])
        self.assertEqual(flags.count('-DMATCH_RICH=1'), 1)
        self.assertEqual(sum(f.startswith('-DPASSES=') for f in flags), 1)
        # A future sizing entry need not duplicate a semantic workload flag.
        flags = corpus.workload_flags(by_name['lz4_match_extend'], '-DPASSES=2')
        self.assertIn('-DMATCH_RICH=1', flags)
        self.assertIn('-DPASSES=2', flags)
        self.assertNotIn('-DPASSES=2048', flags)

    def test_paired_timing_never_times_crashes_or_changed_output(self):
        sys.path.insert(0, str(ROOT / 'scripts'))
        import paired_ab
        # A runner failure (timeout, overflow) is never a sample.
        with mock.patch.object(paired_ab.process, 'run', return_value=dict(outcome(failure='timeout'), elapsed=0.1)):
            with self.assertRaises(ValueError):
                paired_ab.time_run('/probe')
        # Crashes and changed output surface as rc/stdout for the caller's
        # per-sample identity check (exercised end to end below and in
        # tests/corpus/test_paired_sample_integrity.py).
        for reply in [outcome(17), outcome(-11), outcome(stdout=b'checksum')]:
            with self.subTest(reply=reply), mock.patch.object(paired_ab.process, 'run', return_value=dict(reply, elapsed=0.1)):
                ms, rc, out = paired_ab.time_run('/probe')
                self.assertTrue(rc != 0 or out != 'checksum\n')
        with mock.patch.object(paired_ab.process, 'run', return_value=dict(outcome(), elapsed=0.1)):
            self.assertEqual(paired_ab.time_run('/probe'), (100.0, 0, 'checksum\n'))

    def test_paired_warmup_rejects_equal_nonzero_before_identity_guard(self):
        sys.path.insert(0, str(ROOT / 'scripts'))
        import paired_ab
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / 'program'
            binary.write_text('#!/bin/sh\nexit 17\n')
            binary.chmod(0o755)
            report = Path(temp) / 'report.json'
            with mock.patch.object(sys, 'argv', ['paired_ab', '--bin-a', str(binary), '--bin-b', str(binary), '--json', str(report)]):
                self.assertEqual(paired_ab.main(), 4)
            self.assertEqual(json.loads(report.read_text())['verdict'], 'correctness-failure')

    def test_native_runner_preserves_bytes_and_requires_success(self):
        hashes = set()
        for output in [b'value', b'value\n', b'value\r\n', b'value\xff']:
            result = bench.invoke([sys.executable, '-c', 'import sys;sys.stdout.buffer.write(' + repr(output) + ')'], timeout_s=3)
            self.assertTrue(result['ok'])
            hashes.add(result['stdout_sha256'])
        self.assertEqual(len(hashes), 4)
        result = bench.invoke([sys.executable, '-c', 'raise SystemExit(17)'], timeout_s=3)
        self.assertFalse(result['ok'])
        self.assertEqual(result['returncode'], 17)

    def test_partial_runs_and_bad_warmups_cannot_enter_performance_aggregate(self):
        good = dict(ok=True, stdout='value\n', stdout_sha256='hash-value')
        bad = dict(good, ok=False)
        changed = dict(good, stdout='value', stdout_sha256='hash-without-lf')
        for runs, warmups in [([good, bad], []), ([good], [bad]),
                              ([good], [changed]), ([good, changed], [])]:
            with self.subTest(runs=runs, warmups=warmups):
                entry = dict(name='probe', runs={'lccc': runs, 'gcc': [good]},
                             warmups={'lccc': warmups}, correctness={},
                             ratios_to_gcc={'lccc': {'median': 0.5}})
                bench.evaluate_correctness(entry, ['lccc', 'gcc'])
                self.assertFalse(entry['correctness']['lccc']['matches_baseline'])
                self.assertIsNone(bench.aggregate_lccc_vs_gcc([entry]))
        entry = dict(runs={'lccc': [good], 'gcc': [good]},
                     warmups={'lccc': [good]}, correctness={})
        bench.evaluate_correctness(entry, ['lccc', 'gcc'])
        self.assertTrue(entry['correctness']['lccc']['matches_baseline'])

    def test_invalid_gcc_reference_cannot_enter_aggregate_or_best_selection(self):
        good = dict(ok=True, stdout='value\n')
        entry = dict(name='probe', runs={'lccc': [good], 'gcc': [good, dict(good, ok=False)],
                                        'clang': [good]}, correctness={},
                     ratios_to_gcc={'lccc': {'median': 0.5}},
                     runtime={'gcc': {'wall': {'median': 0.1}},
                              'clang': {'wall': {'median': 0.2}}})
        bench.evaluate_correctness(entry, ['lccc', 'gcc', 'clang'])
        # A failed GCC oracle is not silently replaced: nothing is certified.
        self.assertEqual(entry['baseline_compiler'], 'gcc')
        self.assertFalse(entry['correctness']['gcc']['matches_baseline'])
        self.assertIsNone(entry['correctness']['lccc']['matches_baseline'])
        self.assertIsNone(entry['correctness']['clang']['matches_baseline'])
        self.assertIsNone(bench.aggregate_lccc_vs_gcc([entry]))
        self.assertIsNone(bench.best_reference_for_entry(entry))

    def test_candidates_never_become_the_oracle(self):
        good = dict(ok=True, stdout='value\n')
        wrong = dict(ok=True, stdout='other\n')
        for keys, expected_baseline in [(['lccc', 'clang'], 'clang'), (['lccc', 'icx'], 'icx'),
                                        (['lccc', 'lev', 'ccc'], None), (['mine', 'lccc'], None),
                                        (['clang', 'gcc', 'lccc'], 'gcc')]:
            with self.subTest(keys=keys):
                runs = {key: [good] for key in keys}
                runs['lccc'] = [wrong]
                entry = dict(name='probe', runs=runs, correctness={})
                bench.evaluate_correctness(entry, keys)
                self.assertEqual(entry['baseline_compiler'], expected_baseline)
                lccc = entry['correctness']['lccc']['matches_baseline']
                # The wrong candidate is rejected by an oracle, never self-certified.
                self.assertIs(lccc, False if expected_baseline else None)
                if expected_baseline:
                    self.assertTrue(entry['correctness'][expected_baseline]['matches_baseline'])

    def test_native_compile_rejects_stale_missing_and_nonexecutable_output(self):
        compiler = bench.CompilerSpec('probe', 'Probe', '/compiler', ())
        benchmark = bench.BENCHMARKS[0]
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / 'a.out'
            source = Path(temp) / 'probe.c'
            source.write_text('int main(void) { return 0; }')
            for kind in ['missing', 'nonexecutable', 'executable']:
                with self.subTest(kind=kind):
                    output.write_bytes(b'stale')
                    output.chmod(0o755)
                    def invoke(*_args, **_kwargs):
                        self.assertFalse(output.exists())
                        if kind != 'missing':
                            output.write_bytes(b'new')
                            output.chmod(0o755 if kind == 'executable' else 0o644)
                        return dict(ok=True, error='', returncode=0)
                    with mock.patch.object(bench, 'invoke', side_effect=invoke), \
                            mock.patch.object(bench, 'text_size', return_value=3):
                        result = bench.compile_one(compiler, benchmark, source, output, None)
                    self.assertEqual(result['ok'], kind == 'executable')
                    if kind == 'executable':
                        self.assertEqual(result['binary_sha256'], bench.sha256_file(output))
                    else:
                        self.assertIsNone(result['binary_sha256'])

    def test_sign_test_is_exact_and_does_not_round_small_p_values_to_zero(self):
        sys.path.insert(0, str(ROOT / 'scripts'))
        import paired_ab
        import math
        self.assertEqual(paired_ab.sign_test_p([]), 1.0)
        self.assertEqual(paired_ab.sign_test_p([0, 0, -1, 1]), 1.0)
        self.assertEqual(paired_ab.sign_test_p([-1, -1]), 0.5)
        self.assertEqual(paired_ab.sign_test_p([-1] * 25), 2 / 2**25)
        self.assertEqual(paired_ab.sign_test_p([-1] * 22 + [1] * 3),
                         2 * sum(math.comb(25, i) for i in range(4)) / 2**25)
        raw = [1.00000001, 1.00000002]
        self.assertEqual(paired_ab.summarise('test', raw)['samples_ms'], raw)


if __name__ == '__main__':
    unittest.main()
