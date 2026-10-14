"""The real FMA gate must propagate failures through command substitution.

These isolated shell fixtures test gate control flow, never compiler semantics.
"""
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tests/regression/check_fma_matcher_guards.sh"


class FmaGateContract(unittest.TestCase):
    def run_fixture(self, oracle="reference", candidate="reference", want="",
                    compile_failure=False, execute_failure=False):
        text = SCRIPT.read_text()
        function = text[text.index("run_case() {"):text.index("\nprelude() {")]
        harness = '''set -euo pipefail
fail=0
bad() { echo "$*" >&2; fail=1; }
work=/unused
flags="-O2"
GCC=oracle
CCC=candidate
oracle() { return 0; }
'''
        harness += "candidate() { return " + ("37" if compile_failure else "0") + "; }\n"
        harness += "timeout() {\n"
        if execute_failure:
            harness += "return 38\n"
        else:
            harness += f'if [[ $2 == *.ref ]]; then echo "{oracle}"; else echo "{candidate}"; fi\n'
        harness += "}\n" + function
        harness += f'got=$(run_case fixture /unused/source.c "{want}")\necho "UNREACHABLE_ON_FAILURE:$got"\n'
        return subprocess.run(["bash", "-c", harness], capture_output=True, text=True, timeout=5)

    def test_equal_outputs_remain_admitted(self):
        result = self.run_fixture()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("UNREACHABLE_ON_FAILURE:reference", result.stdout)

    def test_compile_execution_and_output_failures_propagate(self):
        for options in ({"compile_failure": True}, {"execute_failure": True},
                        {"candidate": "wrong"}, {"want": "different"}):
            with self.subTest(options=options):
                result = self.run_fixture(**options)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("UNREACHABLE_ON_FAILURE", result.stdout)
                self.assertIn("fixture:", result.stderr)

    def test_reduction_reach_is_not_a_whole_tu_count(self):
        text = SCRIPT.read_text()
        self.assertIn('__attribute__((noinline)) static long sum_lt', text)
        self.assertIn('"$work/sum_${shape}.s"', text)
        self.assertIn('missing noinline function', text)


if __name__ == "__main__":
    unittest.main()
