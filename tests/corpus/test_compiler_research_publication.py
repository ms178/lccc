"""Exercise the hosted research publisher against its actual corpus schema."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/compiler-research.yml"


class ResearchPublication(unittest.TestCase):
    def publish(self, mode):
        text = WORKFLOW.read_text()
        script = textwrap.dedent(text.split("python3 - <<'PY'\n", 1)[1].split("\n          PY", 1)[0])
        cases = re.search(r"RESEARCH_CASES: ([a-z0-9_,]+)", text).group(1).split(",")
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "results/research"
            if mode != "missing":
                report = root / "callgrind-main/index.json"
                report.parent.mkdir(parents=True)
                results = {name: {"metrics": {"ir_ratio": 1.125}} for name in cases}
                failures = []
                if mode == "failed":
                    failures = [{"name": cases[0], "error": "instrumentation failed"}]
                    results.pop(cases[0])
                if mode == "wrong_registry":
                    results.pop(cases[0])
                report.write_text(json.dumps({"complete": True, "results": results, "failures": failures}))
            result = subprocess.run([sys.executable, "-c", script], cwd=td,
                                    env={**os.environ, "RESEARCH_CASES": ",".join(cases),
                                         "GITHUB_STEP_SUMMARY": str(Path(td) / "step.md")},
                                    capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertLessEqual(len(result.stdout.encode()), 4096)
            self.assertIn("::notice title=Compiler research evidence::", result.stdout)
            return json.loads((root / "summary.json").read_text())

    def test_missing_evidence_is_not_a_pass(self):
        summary = self.publish("missing")
        self.assertEqual(summary["callgrind"]["main"], {"available": False})
        self.assertIsNone(summary["head"])

    def test_complete_report_retains_ratios_and_hash(self):
        row = self.publish("complete")["callgrind"]["main"]
        self.assertTrue(row["complete"])
        self.assertEqual(len(row["ratios"]), 6)
        self.assertEqual(len(row["report_sha256"]), 64)

    def test_failures_or_missing_cases_cannot_be_reported_complete(self):
        for mode in ("failed", "wrong_registry"):
            with self.subTest(mode=mode):
                self.assertFalse(self.publish(mode)["callgrind"]["main"]["complete"])


if __name__ == "__main__":
    unittest.main()
