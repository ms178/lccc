#!/usr/bin/env python3
"""Fast hermetic negative controls: never run the slow benchmark corpus here."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
ENTRY = Path(os.environ.get("BENCHMARK_GATE", ROOT / "scripts/check_benchmark_outputs.sh"))
spec = importlib.util.spec_from_file_location("benchmark_gate", ROOT / "scripts/check_benchmark_outputs.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class BenchmarkOutputGateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="benchmark-gate-contract-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def compiler(self, name, output=b"correct\n", status=0, build_status=0):
        executable = self.root / name
        program = ("#!/usr/bin/env python3\nimport os, resource, sys\n"
                   "resource.setrlimit(resource.RLIMIT_CORE, (0, 0))\n"
                   f"sys.stdout.buffer.write({output!r})\n"
                   + ("os.kill(os.getpid(), 11)\n" if status == -11 else f"sys.exit({status})\n"))
        executable.write_text(
            "#!/usr/bin/env python3\nfrom pathlib import Path\nimport sys\n"
            f"if '-print-file-name=include' in sys.argv: print({str(self.root)!r}); sys.exit(0)\n"
            f"if {build_status}: sys.exit({build_status})\n"
            "out = Path(sys.argv[sys.argv.index('-o') + 1])\n"
            f"out.write_text({program!r}); out.chmod(0o755)\n"
        )
        executable.chmod(0o755)
        return str(executable)

    def check(self, gcc=None, lccc=None, selection="byte_load_reassembly", good=False, opt="-O2"):
        env = dict(os.environ, GCC_BIN=gcc or self.compiler("gcc"),
                   LCCC_BIN=lccc or self.compiler("lccc"), OPT=opt, LCCC_REPO=str(ROOT))
        result = subprocess.run(["bash", str(ENTRY), selection], env=env,
                                capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode == 0, good, result.stdout + result.stderr)
        return result

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


if __name__ == "__main__":
    unittest.main()
