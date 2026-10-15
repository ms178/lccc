"""Offline checkpoint, exact-corpus and alias contracts for the bounded screen."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import subprocess
import platform
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
import callgrind_corpus as screen


class ScreenTests(unittest.TestCase):
    def test_exact_sizes_and_distinct_aliases(self):
        sizes = json.loads((screen.ROOT / "tests/benchmark/callgrind_sizes.json").read_text())
        self.assertEqual(set(sizes), {b.name for b in screen.bench.BENCHMARKS})
        self.assertIn("-DMATCH_RICH=1", sizes["lz4_match_extend"])
        self.assertNotEqual(sizes["linux_find_bit"], sizes["linux_find_bit_scaled"])

    def test_percpu_pointer_walk_is_registered_and_bounded(self):
        cases = {b.name: b for b in screen.bench.BENCHMARKS}
        case = cases["percpu_ptr_walk"]
        self.assertTrue(screen.bench.source_for(case).is_file())
        sizes = json.loads((screen.ROOT / "tests/benchmark/callgrind_sizes.json").read_text())
        passes = int(sizes[case.name].removeprefix("-DPASSES="))
        self.assertGreater(passes, 0)
        self.assertLess(passes, 300000)

    def test_unavailable_gs_is_executed_and_never_a_successful_sample(self):
        if sys.platform != "linux" or platform.machine() not in ("x86_64", "AMD64"):
            self.skipTest("the per-CPU workload requires Linux x86-64 GS")
        source = screen.ROOT / "tests/benchmark/programs/percpu_ptr_walk.c"
        # Deny ARCH_SET_GS in this child only. Exercise the actual benchmark,
        # not a source spelling or a production-only failure-injection flag.
        wrapper = r'''#include <stddef.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <errno.h>
#define main benchmark_main
#include "SOURCE"
#undef main
int main(void) {
    struct sock_filter filter[] = {
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_arch_prctl, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | EPERM),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW)
    };
    struct sock_fprog program = {4, filter};
    if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) ||
        prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &program)) return 90;
    return benchmark_main();
}
'''.replace("SOURCE", str(source))
        with tempfile.TemporaryDirectory() as td:
            src, exe = Path(td) / "denied.c", Path(td) / "denied"
            src.write_text(wrapper)
            subprocess.run(["gcc", "-O2", str(src), "-o", str(exe)], check=True, capture_output=True, timeout=30)
            result = screen.cg._run([str(exe)], 10)
            self.assertEqual(result["returncode"], 77)
            self.assertIn(b"percpu-ptr-walk: no-gs", result["stdout"])
            self.assertIsNotNone(screen.cg.same_output(result, result))

    def test_failures_are_not_ranked(self):
        with tempfile.TemporaryDirectory() as td, patch.object(screen.cg, "main", return_value=1):
            self.assertEqual(screen.main(["--mine", "stub", "--only", "ring_fifo", "--out", td]), 1)
            result = json.loads((Path(td) / "index.json").read_text())
            self.assertTrue(result["complete"])
            self.assertEqual(result["worst_first"], [])
            self.assertEqual(len(result["failures"]), 1)

    def test_header_driver_is_forwarded_in_both_default_and_versioned_runs(self):
        for driver in ("gcc", "gcc-16"):
            with self.subTest(driver=driver), tempfile.TemporaryDirectory() as td, \
                    patch.object(screen.cg, "main", return_value=1) as measure:
                options = [] if driver == "gcc" else ["--gcc-include-command", driver]
                self.assertEqual(screen.main([
                    "--mine", "stub", "--ref", driver, "--only", "ring_fifo",
                    "--out", td, *options,
                ]), 1)
                args = measure.call_args.args[0]
                self.assertEqual(args[1], driver)
                self.assertEqual(args[args.index("--gcc-include-command") + 1], driver)

    def test_interruption_keeps_incomplete_checkpoint(self):
        with tempfile.TemporaryDirectory() as td, patch.object(screen.cg, "main", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                screen.main(["--mine", "stub", "--only", "ring_fifo", "--out", td])
            self.assertFalse(json.loads((Path(td) / "index.json").read_text())["complete"])

    def test_unknown_selection_does_not_measure(self):
        with tempfile.TemporaryDirectory() as td, patch.object(screen.cg, "main") as measure:
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                screen.main(["--mine", "stub", "--only", "not-a-case", "--out", td])
            measure.assert_not_called()


if __name__ == "__main__":
    unittest.main()
