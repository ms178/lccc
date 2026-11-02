#!/usr/bin/env python3
"""Pin the labels and values in the Godbolt oracle's timing table (offline)."""
import contextlib
import io
from pathlib import Path
import sys
import unittest
from unittest.mock import mock_open, patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "oracle"))
import godbolt_oracle as oracle


class GodboltTimingColumns(unittest.TestCase):
    def test_local_references_precede_remote_oracles_under_matching_headers(self):
        row = {
            "program": "probe.c",
            "lccc": {"ok": True, "asm_insns": 1, "stdout": "ok\n", "exit": 0},
            "oracles": [
                {"id": "cg162", "ok": True, "asm_insns": 2, "stdout": "ok", "exit": 0},
                {"id": "cclang2310", "ok": True, "asm_insns": 3, "stdout": "ok", "exit": 0},
            ],
            "status": "PASS",
            "why": [],
        }
        local_seconds = {None: 0.2, "gcc": 0.4, "clang": 0.3}
        remote_seconds = {"cg162": 0.1, "cclang2310": 0.15}

        def bench_local(_source, _flags, _reps, cc=None):
            return {
                "id": "lccc" if cc is None else Path(cc).name,
                "vendor": "local",
                "ok": True,
                "sec": local_seconds[cc],
                "stdout": "ok\n",
                "exit": 0,
                "reason": None,
            }

        def bench_remote(cid, _source, _flags, _reps, _timeout):
            return {
                "id": cid,
                "vendor": "test",
                "ok": True,
                "sec": remote_seconds[cid],
                "stdout": "ok",
                "exit": 0,
                "reason": None,
            }

        output = io.StringIO()
        with contextlib.redirect_stdout(output), \
             patch.object(oracle, "standalone_programs", return_value=["probe.c"]), \
             patch.object(oracle, "check", return_value=row), \
             patch.object(oracle, "LCCC", str(Path(__file__))), \
             patch.object(oracle, "open", mock_open(read_data="int main(void) { return 0; }\n"), create=True), \
             patch.object(oracle, "bench_local", side_effect=bench_local), \
             patch.object(oracle, "bench_remote", side_effect=bench_remote), \
             patch.object(oracle, "remote", return_value={"ok": True, "stdout": "ok", "exit": 0}):
            status = oracle.main([
                "--filter", "probe",
                "--oracles", "cg162,cclang2310",
                "--bench", "1",
                "--local-ref", "gcc,clang",
            ])

        self.assertEqual(status, 0)
        line = next(line for line in output.getvalue().splitlines() if line.startswith("probe "))
        columns = line.split()
        self.assertEqual(columns[0], "probe")
        # Column order is the displayed header: LCCC, local GCC, local Clang,
        # remote GCC, remote Clang, LCCC/local-GCC ratio, status.
        self.assertEqual(columns[1:6], ["200.000", "400.000", "300.000", "100.000", "150.000"])
        self.assertEqual(columns[6:], ["0.500", "PASS"])


if __name__ == "__main__":
    unittest.main()
