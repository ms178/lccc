#!/usr/bin/env python3
"""yarpgen_mutpair contracts, tested without a real yarpgen.

The pair classification and hash parsing are pure functions with a pinned
truth table.  The end-to-end pipeline is tested against a stub generator
(a tiny script honouring yarpgen's file contract), so CI exercises the
generate -> filter -> judge plumbing on every host, provisioned or not:
a differential harness that only runs where its generator happens to be
installed is how the yarpgen engine rotted unnoticed in the first place.
"""
import shutil
import stat
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import yarpgen_mutpair as ym

STUB = '''#!/usr/bin/env python3
# Stub yarpgen: honours --std=/--seed=/--max-array-dims=/-o plus the
# --mutate/--mutation-seed pair, emits driver.c/func.c/init.h whose hash
# output is seed-derived; mutation adds 1 unless mutation-seed % 3 == 0
# (textual no-op branch, to exercise the identical-func discard).
import sys
from pathlib import Path
args = sys.argv[1:]
seed = int(next(a.split("=", 1)[1] for a in args if a.startswith("--seed=")))
out = Path(args[args.index("-o") + 1])
mseed = next((int(a.split("=", 1)[1]) for a in args
              if a.startswith("--mutation-seed=")), None)
out.mkdir(parents=True, exist_ok=True)
bump = 1 if (mseed is not None and mseed % 3) else 0
(out / "init.h").write_text("extern unsigned long long g;\\nvoid test(void);\\n")
(out / "func.c").write_text(
    '/*\\nInvocation: stub\\n*/\\n#include "init.h"\\n'
    f"void test(void) {{ g = {1000 + seed}u + {bump}u; }}\\n")
(out / "driver.c").write_text(
    '#include <stdio.h>\\n#include "init.h"\\nunsigned long long g;\\n'
    'int main(void) { test(); printf("%llu\\\\n", g); return 0; }\\n')
'''


class PureFunctionTests(unittest.TestCase):
    def test_classify_truth_table(self):
        self.assertEqual(ym.classify(1, 2, 1, 2), "pass")
        self.assertIn("COLLAPSED", ym.classify(1, 2, 7, 7))
        self.assertEqual(ym.classify(1, 2, 3, 2), "hash mismatch")
        self.assertEqual(ym.classify(1, 2, None, 2), "build/run failure")
        self.assertEqual(ym.classify(1, 2, 1, None), "build/run failure")

    def test_parse_hash(self):
        self.assertEqual(ym.parse_hash("6871103402060576105\n"),
                         6871103402060576105)
        self.assertIsNone(ym.parse_hash(""))
        self.assertIsNone(ym.parse_hash("-5"))
        self.assertIsNone(ym.parse_hash("12 34"))

    def test_func_body_strips_only_invocation(self):
        self.assertEqual(ym.func_body("Invocation: a b c\nx\n"), "x")
        self.assertEqual(ym.func_body("x\nInvocation2: y\n"),
                         "x\nInvocation2: y")


@unittest.skipUnless(shutil.which("gcc"), "gcc required for stub pipeline")
class StubPipelineTests(unittest.TestCase):
    def _stub(self, tmp: Path) -> str:
        stub = tmp / "yarpgen-stub"
        stub.write_text(STUB)
        stub.chmod(stub.stat().st_mode | stat.S_IXUSR)
        return str(stub)

    def test_generate_filter_judge_end_to_end(self):
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            stub = self._stub(tmp)
            gcc = shutil.which("gcc")
            judged, failures, stats = ym.run_pairs(
                stub, 4, 10, {"gcc-as-dut": [gcc]}, [gcc], ["-O0"],
                tmp / "repros", 30.0)
            self.assertEqual((judged, failures), (4, 0), stats)
            # The stub's mseed%3==0 branch must have been exercised or
            # convincingly absent; identical-func discards are legal
            # either way -- what must never happen is a silent skip of
            # every pair (judged==4 above already pins that).

    def test_collapse_simulation_is_reported(self):
        # Judge the mutant in BOTH slots against the true pair's hashes:
        # that is exactly what a pair-collapsing optimiser produces, and
        # the harness must name it.
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            stub = self._stub(tmp)
            gcc = shutil.which("gcc")
            orig, mut = tmp / "o", tmp / "m"
            self.assertTrue(ym.generate(stub, 5, None, orig, 30))
            self.assertTrue(ym.generate(stub, 5, 7, mut, 30))  # 7%3=1: bump
            ro = ym.build_run_hash([gcc], orig, "-O0", tmp / "a", 30)
            rm = ym.build_run_hash([gcc], mut, "-O0", tmp / "b", 30)
            self.assertEqual((ro, rm), (1005, 1006))
            fails = ym.judge_pair(mut, mut, ro, rm, {"gcc": [gcc]},
                                  ["-O0"], tmp, 30)
            self.assertTrue(any("COLLAPSED" in f for f in fails), fails)


if __name__ == "__main__":
    unittest.main()
