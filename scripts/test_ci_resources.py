#!/usr/bin/env python3
"""Exercise the actual CI gate functions, with fake Cargo/free (no build/OOM)."""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SOURCE = (ROOT / "scripts/ci_local.sh").read_text()
NAMES = ("configure_cargo_gate_resources", "cargo_test_repeated", "clippy_gate")
FUNCTIONS = "\n".join(re.search(rf"^{name}\(\) [{{(]\n.*?^[}})]$", SOURCE,
                                re.M | re.S).group() for name in NAMES)
SETUP = r'''
free() { if [ "$MEM" = missing ]; then return 1; fi; printf 'Mem: %s 0 0 0 0 0\n' "$MEM"; }
record() { printf 'RUN|%s|%s|%s|%s|%s\n' "$1" "${CARGO_BUILD_JOBS-unset}" "${CARGO_PROFILE_FASTBUILD_DEBUG-unset}" "${CARGO_INCREMENTAL-unset}" "${RUSTFLAGS-unset}"; }
cargo() { if [ "$*" = "clippy --version" ]; then return "${FAIL_VERSION:-0}"; fi; record "$*"; return "${FAIL_CARGO:-0}"; }
python3() {
 case "$1" in
 scripts/ci_test_report.py)
   if [ "${2:-}" = --self-test ]; then return "${FAIL_REPORTER:-0}"; fi
   shift 2; "$@" ;;
 scripts/check_clippy_policy.py) record clippy; return "${FAIL_CLIPPY:-0}" ;;
 *) return 99 ;;
 esac
}
capture() {
 for name in CARGO_BUILD_JOBS CI_LOCAL_JOBS CARGO_PROFILE_FASTBUILD_DEBUG CARGO_INCREMENTAL RUSTFLAGS incr dbg jobs flags; do
   declare -p "$name" 2>/dev/null || true
 done
}
incr=sentinel
before=$(capture)
'''


class ResourceTests(unittest.TestCase):
    def run_gates(self, call, mem="1982", overrides=None):
        env = {k: v for k, v in os.environ.items() if not k.startswith(("CARGO_", "CI_LOCAL_", "LCCC_TEST_", "FAIL_")) and k != "RUSTFLAGS"}
        env.update(MEM=mem)
        env.update(overrides or {})
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "target").mkdir()
            (Path(tmp) / "target/lccc-rustflags").write_text("--cfg gate_control")
            script = "set -u -o pipefail\n" + FUNCTIONS + "\n" + SETUP + "\n" + call + r'''
rc=$?
after=$(capture)
[ "$before" = "$after" ] || { echo 'CALLER STATE LEAKED' >&2; exit 98; }
exit "$rc"
'''
            p = subprocess.run(["bash", "-c", script], cwd=tmp, env=env,
                               text=True, capture_output=True, timeout=10)
        self.assertNotIn("CALLER STATE LEAKED", p.stderr)
        runs = [s.split("|")[1:] for s in p.stdout.splitlines() if s.startswith("RUN|")]
        return p.returncode, runs

    def test_order_isolation_resource_matrix(self):
        for mem in ["1982", "5999", "6000", "32768", "missing", "garbage",
                    "9223372036854775808", "9" * 100]:
            for overrides in [{}, {"CARGO_PROFILE_FASTBUILD_DEBUG": "0"},
                              {"CARGO_PROFILE_FASTBUILD_DEBUG": "line-tables-only"},
                              {"CARGO_INCREMENTAL": "1"}, {"CI_LOCAL_JOBS": "2"},
                              {"CARGO_BUILD_JOBS": "3"},
                              {"CARGO_BUILD_JOBS": "3", "CI_LOCAL_JOBS": "2"},
                              {"CARGO_PROFILE_FASTBUILD_DEBUG": "", "CARGO_INCREMENTAL": ""}]:
                with self.subTest(mem=mem, overrides=overrides):
                    low = not re.fullmatch(r"[0-9]{1,18}", mem) or int(mem) < 6000
                    jobs = overrides.get("CI_LOCAL_JOBS", overrides.get("CARGO_BUILD_JOBS", "1" if low else "2"))
                    dbg = overrides.get("CARGO_PROFILE_FASTBUILD_DEBUG") or ("0" if low else "unset")
                    inc = overrides.get("CARGO_INCREMENTAL") or ("0" if dbg == "0" else "unset")
                    isolated = []
                    for call in ["cargo_test_repeated", "clippy_gate"]:
                        rc, runs = self.run_gates(call, mem, overrides)
                        self.assertEqual(rc, 0)
                        self.assertEqual(len(runs), 1)
                        self.assertEqual(runs[0][1:], [jobs, dbg, inc, "--cfg gate_control"])
                        isolated.extend(runs)
                    for call, expected in [("cargo_test_repeated; clippy_gate", isolated),
                                           ("clippy_gate; cargo_test_repeated", isolated[::-1])]:
                        rc, runs = self.run_gates(call, mem, overrides)
                        self.assertEqual(rc, 0)
                        self.assertEqual(runs, expected)
                    self.assertIn("--profile fastbuild --all-targets --locked -j " + jobs, isolated[0][0])

    def test_errors_propagate_without_state_leaks(self):
        for call, overrides, expected, count in [
            ("cargo_test_repeated", {"FAIL_REPORTER": "7"}, 7, 0),
            ("cargo_test_repeated", {"FAIL_CARGO": "8"}, 1, 1),
            ("clippy_gate", {"FAIL_CLIPPY": "9"}, 9, 1),
            ("clippy_gate", {"FAIL_VERSION": "6"}, 6, 0),
        ]:
            with self.subTest(call=call, overrides=overrides):
                rc, runs = self.run_gates(call, overrides=overrides)
                self.assertEqual(rc, expected)
                self.assertEqual(len(runs), count)
        for key in ["CI_LOCAL_JOBS", "CARGO_BUILD_JOBS", "LCCC_TEST_REPEATS"]:
            for value in ["0", "-1", "nonsense", "1+1"]:
                for call in (["cargo_test_repeated"] if key == "LCCC_TEST_REPEATS" else ["cargo_test_repeated", "clippy_gate"]):
                    with self.subTest(key=key, value=value, call=call):
                        rc, runs = self.run_gates(call, overrides={key: value})
                        self.assertEqual(rc, 2)
                        self.assertEqual(runs, [])

    def test_repeat_arithmetic_cannot_wrap_into_a_false_pass(self):
        for value in ["2147483648", "9223372036854775808", "18446744073709551616", "9" * 100, "08"]:
            rc, runs = self.run_gates("cargo_test_repeated", overrides={"LCCC_TEST_REPEATS": value})
            self.assertEqual(rc, 2)
            self.assertEqual(runs, [])

    def test_repeats_and_empty_caller_values(self):
        rc, runs = self.run_gates("cargo_test_repeated", overrides={"LCCC_TEST_REPEATS": "3", "CARGO_PROFILE_FASTBUILD_DEBUG": "", "CARGO_INCREMENTAL": ""})
        self.assertEqual(rc, 0)
        self.assertEqual(len(runs), 3)
        self.assertEqual(runs[0], runs[1])
        self.assertEqual(runs[1], runs[2])


if __name__ == "__main__":
    unittest.main()
