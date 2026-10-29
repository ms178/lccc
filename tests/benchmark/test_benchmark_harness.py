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


class BalancedOrderTests(unittest.TestCase):
    def test_two_arm_blocks_are_balanced_deterministic_and_safe_after_failure(self):
        import random
        def schedule(seed):
            rng = random.Random(seed)
            result, prev = [], None
            for i in range(9):
                prev = bench.balanced_compiler_order(['lccc', 'gcc'], rng, i, prev)
                result.append(prev)
            return result
        orders = schedule(42)
        self.assertEqual(orders, schedule(42))
        for i in range(0, 8, 2):
            self.assertEqual(orders[i + 1], list(reversed(orders[i])))
        self.assertIn(sum(order[0] == 'lccc' for order in orders), (4, 5))
        self.assertEqual(bench.balanced_compiler_order(['gcc'], random.Random(2), 1, ['lccc', 'gcc']), ['gcc'])
        more = bench.balanced_compiler_order(['a', 'b', 'c'], random.Random(3), 1, ['b', 'a'])
        self.assertEqual(sorted(more), ['a', 'b', 'c'])


class HybridPmuProbeTests(unittest.TestCase):
    def test_actual_report_and_cpu_class_selection(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            for name, cpus in (("cpu_core", "0-15"), ("cpu_atom", "16-27")):
                (root / name).mkdir()
                (root / name / "cpus").write_text(cpus)
            for cpu, pmu in ((0, "cpu_core"), (16, "cpu_atom")):
                with self.subTest(cpu=cpu):
                    rows = f"259234,,{pmu}/cycles/u,173437,100.00,,\n162988,,{pmu}/instructions/u,173437,100.00,,\n"
                    result = {"ok": True, "returncode": 0, "stderr": rows, "error": ""}
                    with mock.patch.object(bench.shutil, "which", return_value="/usr/bin/perf"), \
                         mock.patch.object(bench, "invoke", return_value=result) as invoke:
                        status = bench.probe_pmu(True, ["taskset", "-c", str(cpu)], cpu, root)
                    self.assertTrue(status["available"], status)
                    self.assertEqual(status["probe_pmu"], pmu)
                    command = invoke.call_args.args[0]
                    self.assertEqual(command[:3], ["taskset", "-c", str(cpu)])
                    self.assertIn(f"{{{pmu}/cycles/,{pmu}/instructions/}}", command)
                    self.assertIn("--all-user", command)

    def test_bad_hybrid_results_never_become_evidence(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            for name, cpus in (("cpu_core", "0-15"), ("cpu_atom", "16-27")):
                (root / name).mkdir()
                (root / name / "cpus").write_text(cpus)
            good = "123,,cpu_core/cycles/,10,100.00,,\n456,,cpu_core/instructions/,10,100.00,,\n"
            for data in ("", good.replace("123", "<not counted>"), good.replace("123", "0"),
                         good.replace("100.00", "94.0"), good + good, good.replace("456", "nan"),
                         good.replace("cpu_core", "cpu_atom")):
                with self.subTest(data=data), mock.patch.object(bench.shutil, "which", return_value="perf"), \
                     mock.patch.object(bench, "invoke", return_value={"ok": True, "returncode": 0,
                                        "stderr": data, "error": ""}):
                    self.assertFalse(bench.probe_pmu(True, ["taskset", "-c", "0"], 0, root)["available"])
            with mock.patch.object(bench.shutil, "which", return_value="perf"):
                self.assertFalse(bench.probe_pmu(True, [], None, root)["available"])
                (root / "cpu_core/cpus").write_text("0-15,broken")
                self.assertFalse(bench.probe_pmu(True, ["taskset", "-c", "0"], 0, root)["available"])
                (root / "cpu_core/cpus").write_text("0-16")
                self.assertFalse(bench.probe_pmu(True, ["taskset", "-c", "16"], 16, root)["available"])

    def test_generic_and_disabled_probe(self):
        with tempfile.TemporaryDirectory() as td:
            rows = "123,,cycles:u,10,100.00,,\n456,,instructions:u,10,100.00,,\n"
            with mock.patch.object(bench.shutil, "which", return_value="perf"), \
                 mock.patch.object(bench, "invoke", return_value={"ok": True, "returncode": 0,
                                    "stderr": rows, "error": ""}):
                self.assertTrue(bench.probe_pmu(True, ["taskset", "-c", "0"], 0, Path(td))["available"])
                self.assertFalse(bench.probe_pmu(False, sysfs=Path(td))["available"])


class DurationSensitivityTests(unittest.TestCase):
    def test_short_samples_remain_visible_but_not_in_secondary_diagnostic(self):
        def entry(name, lccc, gcc, ratio):
            return {"name": name, "runtime": {"lccc": {"wall": {"median": lccc}},
                    "gcc": {"wall": {"median": gcc}}},
                    "correctness": {"lccc": {"matches_baseline": True}, "gcc": {"matches_baseline": True}},
                    "ratios_to_gcc": {"lccc": {"median": ratio}}}
        entries = [entry("short", .001, .1, .01), entry("long", .10, .1, 1.0)]
        self.assertEqual(bench.aggregate_lccc_vs_gcc(entries)["n_correct_benchmarks"], 2)
        result = bench.duration_sensitivity(entries)
        self.assertEqual(result["included"], ["long"])
        self.assertEqual(result["excluded_short_or_unknown"], ["short"])
        self.assertEqual(result["aggregate"]["geometric_mean_ratio"], 1.0)
        self.assertIn("not a replacement", result["interpretation"])


class BenchmarkHarnessTests(unittest.TestCase):
    def test_instrumentation_merges_identity_with_explicit_size_overrides(self):
        sys.path.insert(0, str(ROOT / 'scripts'))
        import callgrind_corpus as corpus
        sizes = json.loads((ROOT / 'tests/benchmark/callgrind_sizes.json').read_text())
        by_name = {b.name: b for b in bench.BENCHMARKS}
        self.assertEqual(set(sizes), set(by_name), "every registered workload needs bounded instrumentation sizes")
        self.assertEqual(sizes['volatile_reduce'], '-DVOLATILE_REDUCE_TRIPS=4096')
        self.assertEqual(sizes['hot_web_loop'], '-DHOT_WEB_TRIPS=4096')
        self.assertEqual(sizes['llvm_scalar_facts'], '-DSCALAR_FACTS_TRIPS=1024')
        self.assertEqual(sizes['llvm_word_counts'], '-DROUNDS=1024')
        self.assertEqual(sizes['zlib_ng_compare256'], '-DPASSES=32')
        self.assertTrue(bench.source_for(by_name['zlib_ng_compare256']).is_file())
        self.assertEqual(corpus.workload_flags(by_name['llvm_word_counts'], sizes['llvm_word_counts']), ['-DROUNDS=1024'])
        self.assertEqual(corpus.workload_flags(by_name['linux_find_bit_scaled'], sizes['linux_find_bit_scaled']), ['-DPASSES=128'])
        flags = corpus.workload_flags(by_name['lz4_match_extend'], sizes['lz4_match_extend'])
        self.assertEqual(flags.count('-DMATCH_RICH=1'), 1)
        self.assertEqual(sum(f.startswith('-DPASSES=') for f in flags), 1)
        # A future sizing entry need not duplicate a semantic workload flag.
        flags = corpus.workload_flags(by_name['lz4_match_extend'], '-DPASSES=2')
        self.assertIn('-DMATCH_RICH=1', flags)
        self.assertIn('-DPASSES=2', flags)
        self.assertNotIn('-DPASSES=2048', flags)
        for name in ['fp32_product_map', 'fp64_product_map']:
            flags = corpus.workload_flags(by_name[name], sizes[name])
            self.assertIn('-ffp-contract=off', flags)
            self.assertEqual(flags.count('-DFP_MAP_PASSES=128'), 1)
            self.assertEqual('-DFP_MAP_DOUBLE' in flags, name == 'fp64_product_map')

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


class FlagValuedOptionParsing(unittest.TestCase):
    """`--opt`/`--cflag` take a compiler flag, so every legal value begins with
    a hyphen and argparse's default reading of the space-separated form is
    always wrong.  Both spellings must reach the same parsed value."""

    def test_space_separated_opt_flag_parses_like_the_attached_form(self):
        spaced = bench.build_arg_parser().parse_args(
            bench.normalize_flag_valued_options(["--opt", "-O2"])
        )
        attached = bench.build_arg_parser().parse_args(["--opt=-O2"])
        self.assertEqual(spaced.opt, attached.opt)
        self.assertEqual(spaced.opt, "-O2")

    def test_every_optimization_spelling_survives_normalization(self):
        for flag in ["-O0", "-O1", "-O2", "-O3", "-Os", "-Oz", "-march=x86-64-v3"]:
            args = bench.build_arg_parser().parse_args(
                bench.normalize_flag_valued_options(["--opt", flag])
            )
            self.assertEqual(args.opt, flag)

    def test_repeated_cflags_each_keep_their_own_value(self):
        args = bench.build_arg_parser().parse_args(
            bench.normalize_flag_valued_options(
                ["--cflag", "-Wall", "--cflag", "-fno-strict-aliasing"]
            )
        )
        self.assertEqual(args.cflag, ["-Wall", "-fno-strict-aliasing"])

    def test_a_following_option_is_never_swallowed_as_a_value(self):
        # `--opt --strict` must stay an error rather than silently consuming
        # `--strict` as the optimization flag.
        normalized = bench.normalize_flag_valued_options(["--opt", "--strict"])
        self.assertEqual(normalized, ["--opt", "--strict"])
        with self.assertRaises(SystemExit):
            bench.build_arg_parser().parse_args(normalized)

    def test_normalization_leaves_unrelated_arguments_untouched(self):
        argv = ["--reps", "9", "--warmup", "2", "--strict", "--only", "sieve"]
        self.assertEqual(bench.normalize_flag_valued_options(argv), argv)

    def test_a_trailing_flag_without_a_value_still_reports_the_omission(self):
        with self.assertRaises(SystemExit):
            bench.build_arg_parser().parse_args(
                bench.normalize_flag_valued_options(["--reps", "9", "--opt"])
            )

if __name__ == '__main__':
    unittest.main()
