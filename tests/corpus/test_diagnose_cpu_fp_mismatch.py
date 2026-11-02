"""Contracts for correctness-gated research diagnostics (no compiler needed)."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "fp_diagnostic", Path(__file__).resolve().parents[2] / "scripts/diagnose_cpu_fp_mismatch.py")
fp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fp)


def report():
    good = {"all_runs_successful": True, "stable_within_compiler": True,
            "matches_baseline": True, "output": "42\n"}
    bad = {**good, "matches_baseline": False, "output": "43\n"}
    return {"settings": {"opt": "-O2", "common_cflags": ["-std=gnu17", "-march=native"]},
            "benchmarks": [
                {"name": "float_case", "tags": ["fp"], "baseline_compiler": "gcc",
                 "correctness": {"gcc": good, "icx": bad}},
                {"name": "integer_case", "tags": ["integer"], "baseline_compiler": "gcc",
                 "correctness": {"gcc": good, "icx": bad}},
            ]}


class DiagnoseCpuFpMismatchTests(unittest.TestCase):
    def test_only_stable_fp_mismatches_with_a_healthy_oracle_are_selected(self):
        r = report()
        self.assertEqual(fp.mismatched_fp_cases(r), ["float_case"])
        r["benchmarks"][0]["correctness"]["gcc"]["stable_within_compiler"] = False
        self.assertEqual(fp.mismatched_fp_cases(r), [])

    def test_control_preserves_profile_and_changes_only_contraction(self):
        r = report()
        r["settings"]["common_cflags"].append("-ffp-contract=fast")
        command = fp.control_command(r, ["float_case"], Path("out"), lccc="lccc",
                                     gcc="gcc", clang="clang", icx="icx",
                                     candidates=["before=previous"])
        self.assertIn("--cflag=-std=gnu17", command)
        self.assertIn("--cflag=-march=native", command)
        self.assertIn("--cflag=-ffp-contract=off", command)
        self.assertNotIn("--cflag=-ffp-contract=fast", command)
        self.assertIn("--skip-perf", command)
        self.assertIn("--strict", command)
        self.assertIn("before=previous", command)

    def test_instruction_evidence_comes_from_the_actual_artifact(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            asm = root / "float_case" / "icx" / "assembly.txt"
            asm.parent.mkdir(parents=True)
            asm.write_text("  10: vfmadd213sd %xmm0,%xmm1,%xmm2\n  14: ret\n")
            row = fp.observations(report(), ["float_case"], root)["float_case"]
            self.assertEqual(row["arms"]["icx"]["fused_instructions"], 1)
            self.assertEqual(row["arms"]["icx"]["output"], "43\n")
            self.assertIsNone(row["arms"]["gcc"]["fused_instructions"])


if __name__ == "__main__":
    unittest.main()
