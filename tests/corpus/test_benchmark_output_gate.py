#!/usr/bin/env python3
"""Fast hermetic negative controls: never run the slow benchmark corpus here."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
ENTRY = Path(os.environ.get("BENCHMARK_GATE", ROOT / "scripts/check_benchmark_outputs.sh"))
spec = importlib.util.spec_from_file_location("benchmark_gate", ROOT / "scripts/check_benchmark_outputs.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class GateHarness(unittest.TestCase):
    """Fixture helpers only: no test methods, so it adds no test cases."""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="benchmark-gate-contract-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def compiler(self, name, output=b"correct\n", status=0, build_status=0, delay=0.0):
        executable = self.root / name
        program = ("#!/usr/bin/env python3\nimport os, resource, sys\n"
                   "resource.setrlimit(resource.RLIMIT_CORE, (0, 0))\n"
                   f"sys.stdout.buffer.write({output!r})\n"
                   + ("os.kill(os.getpid(), 11)\n" if status == -11 else f"sys.exit({status})\n"))
        executable.write_text(
            "#!/usr/bin/env python3\nfrom pathlib import Path\nimport sys, time\n"
            f"if '-print-file-name=include' in sys.argv: print({str(self.root)!r}); sys.exit(0)\n"
            f"if {build_status}: sys.exit({build_status})\n"
            + (f"time.sleep({delay})\n" if delay else "")
            + "out = Path(sys.argv[sys.argv.index('-o') + 1])\n"
            f"out.write_text({program!r}); out.chmod(0o755)\n"
        )
        executable.chmod(0o755)
        return str(executable)

    def check(self, gcc=None, lccc=None, selection="byte_load_reassembly", good=False, opt="-O2",
              jobs=None, extra=()):
        argv = ["bash", str(ENTRY), selection, *extra]
        if jobs is not None:
            argv += ["--jobs", str(jobs)]
        env = dict(os.environ, GCC_BIN=gcc or self.compiler("gcc"),
                   LCCC_BIN=lccc or self.compiler("lccc"), OPT=opt, LCCC_REPO=str(ROOT))
        result = subprocess.run(argv, env=env,
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode == 0, good, result.stdout + result.stderr)
        return result

class BenchmarkOutputGateTests(GateHarness):
    def test_equal_successful_outputs_pass(self):
        self.check(good=True)

    def test_empty_selection_fails(self):
        self.check(selection="no_such_registered_benchmark_829")

    def test_oracle_build_failure_fails(self):
        self.check(gcc=self.compiler("gcc", build_status=1))

    def test_candidate_build_failure_fails(self):
        self.check(lccc=self.compiler("lccc", build_status=1))

    def test_matching_nonzero_exits_fail(self):
        self.check(gcc=self.compiler("gcc", status=1), lccc=self.compiler("lccc", status=1))

    def test_matching_signals_fail(self):
        self.check(gcc=self.compiler("gcc", status=-11), lccc=self.compiler("lccc", status=-11))

    def test_trailing_newline_difference_fails(self):
        self.check(lccc=self.compiler("lccc", output=b"correct\n\n"))

    def test_nul_difference_fails(self):
        self.check(lccc=self.compiler("lccc", output=b"cor\0rect\n"))

    def test_equal_binary_outputs_pass(self):
        self.check(gcc=self.compiler("gcc", output=b"a\0b\n\n"),
                   lccc=self.compiler("lccc", output=b"a\0b\n\n"), good=True)

    def test_empty_levels_fail(self):
        self.check(opt=" ")

    def test_registered_variant_macros_are_not_deduplicated(self):
        for item in gate.bench.BENCHMARKS:
            flags = gate.case_flags(item, "-O2")
            self.assertIn("-fwrapv", flags)
            for macro in item.extra_flags:
                self.assertIn(macro, flags)
        variants = {item.name: item for item in gate.bench.BENCHMARKS}
        for name in ("linux_find_bit_scaled", "lz4_match_extend"):
            self.assertIn(name, variants)

    def test_legacy_unregistered_sources_are_not_lost(self):
        cases = gate.all_cases()
        sources = {gate.bench.source_for(item) for item in cases}
        self.assertEqual(sources, set(gate.bench.PROGRAMS.glob("*.c")))
        self.assertEqual(len(cases), len({item.name for item in cases}))
        self.assertTrue({item.name for item in gate.bench.BENCHMARKS}
                        <= {item.name for item in cases})

    def test_empty_or_duplicate_registry_fails(self):
        first = gate.bench.BENCHMARKS[0]
        for registry in ((), (first, first)):
            with patch.object(gate.bench, "BENCHMARKS", registry):
                with self.assertRaises(ValueError):
                    gate.all_cases()

    def test_empty_header_query_is_not_the_current_directory(self):
        with patch.object(gate, "command", return_value=b""), patch.dict(os.environ, OPT="-O2"):
            with self.assertRaises(ValueError):
                gate.main(["byte_load_reassembly"])

    def test_timeout_is_a_failure_not_empty_success(self):
        with patch.object(gate.subprocess, "run", side_effect=subprocess.TimeoutExpired("probe", 1)):
            with self.assertRaises(subprocess.TimeoutExpired):
                gate.command(["probe"], timeout=1)

    def test_only_unoptimized_tce_is_bounded(self):
        item = next(item for item in gate.bench.BENCHMARKS if item.name == "tce_sum")
        for level in ("-O0", "-O1"):
            self.assertIn("-DTCE_N=4096", gate.case_flags(item, level))
        self.assertNotIn("-DTCE_N=4096", gate.case_flags(item, "-O2"))


class BenchmarkOutputGateConcurrencyTests(GateHarness):
    """`--jobs` must change the schedule, never the verdict.

    The gate compares bytes, so a parallel unit is independent by construction;
    what has to be pinned is that the *verdict* and the diagnostic text do not
    depend on the schedule, and that the pool is actually used (a `--jobs` flag
    that quietly ran serially would be a 4x cost for nothing).
    """

    SLOW = ("tce_sum", "byte_load_reassembly", "lz4_match_extend")

    def test_parallel_and_serial_agree_on_success(self):
        serial = self.check(good=True, jobs=1)
        parallel = self.check(good=True, jobs=4)
        self.assertEqual(serial.stdout.strip().split(" jobs=")[0],
                         parallel.stdout.strip().split(" jobs=")[0])
        self.assertIn("jobs=4", parallel.stdout)

    def test_parallel_and_serial_agree_on_failure(self):
        broken = self.compiler("lccc", output=b"wrong\n")
        serial = self.check(lccc=broken, jobs=1)
        parallel = self.check(lccc=broken, jobs=4)
        self.assertIn("FAIL", parallel.stdout)
        self.assertEqual(serial.stdout.strip(), parallel.stdout.replace(" jobs=4", "").strip())
        self.assertIn("byte_load_reassembly", parallel.stdout)

    def test_failure_diagnostics_follow_the_requested_order_not_completion(self):
        """Eight workers, four failures: the log must not depend on the schedule.

        Diagnostics are emitted in `(case, level)` order -- case order from the
        registry, level order from OPT -- which is exactly what a serial run
        prints.  Completion order would make the same failure look different on
        every run and break the annotation diffing downstream.
        """
        levels = ["-O3", "-O1", "-O2", "-O0"]
        result = self.check(lccc=self.compiler("lccc", output=b"wrong\n"),
                            opt=" ".join(levels), jobs=8)
        lines = [line for line in result.stdout.splitlines() if line.startswith("FAIL")]
        self.assertEqual(len(lines), len(levels), result.stdout)
        self.assertEqual([line.split()[2].rstrip(":") for line in lines], levels)
        self.assertEqual({line.split()[1] for line in lines}, {"byte_load_reassembly"})

    def test_the_pool_overlaps_units(self):
        """Six 0.3 s builds: serial >= 1.8 s, four-wide well under it."""
        slow = self.compiler("gcc", delay=0.3)
        slow_lccc = self.compiler("lccc", delay=0.3)
        # Four optimisation levels of one case = four independent units, each
        # building twice: 2.4 s of sleeping to schedule.
        levels = "-O0 -O1 -O2 -O3"
        started = time.monotonic()
        self.check(gcc=slow, lccc=slow_lccc, good=True, jobs=4, opt=levels)
        parallel = time.monotonic() - started
        started = time.monotonic()
        self.check(gcc=slow, lccc=slow_lccc, good=True, jobs=1, opt=levels)
        serial = time.monotonic() - started
        self.assertLess(parallel, serial * 0.75,
                        f"parallel={parallel:.2f}s serial={serial:.2f}s")

    def test_zero_or_negative_jobs_is_refused(self):
        for jobs in (0, -1):
            result = self.check(jobs=jobs)
            self.assertIn("--jobs must be >= 1", result.stderr)

    def test_list_needs_no_compiler_at_all(self):
        env = dict(os.environ, GCC_BIN="/nonexistent/gcc", LCCC_BIN="/nonexistent/lccc",
                   LCCC_REPO=str(ROOT))
        result = subprocess.run(["bash", str(ENTRY), "--list"], env=env,
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        names = result.stdout.split()
        self.assertEqual(len(names), len(gate.all_cases()))
        self.assertIn("volatile_reduce", names)


if __name__ == "__main__":
    unittest.main()
