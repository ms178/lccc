"""Use the real general runner, not a text check of the alias sidecar."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("alias_regression_runner", ROOT / "tests/regression/run_regression.py")
runner = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = runner
spec.loader.exec_module(runner)


class AliasRunnerContract(unittest.TestCase):
    def test_alias_runs_self_check_without_wrong_optimized_oracle(self):
        case = next(t for t in runner.discover([ROOT / "tests/regression"]) if t.name == "dse_global_alias")
        self.assertTrue(case.no_compare)
        for code in (0, 1, 77):
            with self.subTest(exit=code), tempfile.TemporaryDirectory() as directory:
                work = Path(directory)
                (work / case.name).touch()
                with patch.object(runner, "run_checked", side_effect=[(0, "", ""), (code, "dse-alias 17 29\n", "")]) as run:
                    result = runner.compile_one(Path("lccc"), "gcc", case, work)
                self.assertEqual(result.status, "pass" if code == 0 else "fail")
                self.assertEqual(run.call_count, 2, "compile + execute; never the invalid oracle")


if __name__ == "__main__":
    unittest.main()
