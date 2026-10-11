#!/usr/bin/env python3
"""Unit tests for scripts/oracle_delta_gate.py (BACKLOG MS-01). Offline."""
from __future__ import annotations

import json
import tempfile
from unittest.mock import patch
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import oracle_delta_gate as g  # noqa: E402

COUNT = lambda body: sum(1 for ln in body if ln.strip() and not ln.strip().endswith(":")
                         and not ln.strip().startswith("."))


class GateTests(unittest.TestCase):
    def test_metrics(self):
        body = ["f:", "  pushq %rbp", "  movl %eax, -4(%rbp)", "  movl 8(%rsp,%rax), %ecx",
                "  movq x@GOTPCREL(%rip), %rdx", "  popq %rbp", "  ret"]
        self.assertEqual(g.metrics(body, COUNT),
                         {"insns": 6, "stack": 2, "got": 1, "push": 1})

    def test_within_tolerance_passes(self):
        base = {"entries": {"k": {"insns": 100, "stack": 0, "got": 0, "push": 0}}}
        row = {"id": "k", "lccc": {"insns": 105, "stack": 2, "got": 1, "push": 1}, "oracles": {}}
        self.assertEqual(g.evaluate([row], base, g.TOLERANCE)[0], [])

    def test_each_metric_can_fail(self):
        base = {"entries": {"k": {"insns": 100, "stack": 0, "got": 0, "push": 0}}}
        for key, val in (("insns", 106), ("stack", 3), ("got", 2), ("push", 2)):
            lccc = {"insns": 100, "stack": 0, "got": 0, "push": 0, key: val}
            fails, _ = g.evaluate([{"id": "k", "lccc": lccc, "oracles": {}}], base, g.TOLERANCE)
            self.assertEqual(len(fails), 1, key)

    def test_improvement_never_fails(self):
        base = {"entries": {"k": {"insns": 100, "stack": 9, "got": 4, "push": 6}}}
        row = {"id": "k", "lccc": {"insns": 60, "stack": 0, "got": 0, "push": 1}, "oracles": {}}
        self.assertEqual(g.evaluate([row], base, g.TOLERANCE)[0], [])

    def test_ratio_uses_best_oracle(self):
        row = {"id": "k", "lccc": {"insns": 90, "stack": 0, "got": 0, "push": 0},
               "oracles": {"a": {"insns": 60}, "b": {"insns": 45}}}
        _, notes = g.evaluate([row], {}, g.TOLERANCE)
        self.assertTrue(any("ratio 2.00" in n for n in notes), notes)

    def test_corpus_is_wellformed(self):
        corpus = json.loads(g.CORPUS.read_text())
        ids = [e["id"] for e in corpus["entries"]]
        self.assertEqual(len(ids), len(set(ids)))
        for e in corpus["entries"]:
            src = g.ROOT / e["source"]
            self.assertTrue(src.is_file(), src)
            self.assertIn(e["function"] + "(", src.read_text(), e["id"])


class RefreshTests(unittest.TestCase):
    def test_refresh_is_complete_or_does_not_publish(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            corpus, baseline, local, report = [root / x for x in ("corpus", "baseline", "lccc", "report")]
            corpus.write_text(json.dumps({"flags": "-O2", "oracles": ["gcc"],
                                          "entries": [{"id": "a"}, {"id": "b"}]}))
            local.touch()
            old = json.dumps({"schema": 1, "flags": "-O2", "entries": {"old": {}}}) + "\n"
            metric = {"insns": 4, "stack": 0, "got": 0, "push": 0}
            rows = [{"id": x, "lccc": metric, "oracles": {"gcc": metric}} for x in ("a", "b")]
            argv = ["--corpus", str(corpus), "--baseline", str(baseline),
                    "--local", str(local), "--report", str(report), "--update-baseline"]
            cases = [(rows, ["oracle failure"]), (rows[:1], []), (rows + rows[:1], []),
                     ([dict(r, oracles={}) for r in rows], [])]
            for result in cases:
                with self.subTest(result=result):
                    baseline.write_text(old)
                    with patch.object(g, "measure", return_value=result):
                        self.assertEqual(g.main(argv), 1)
                    self.assertEqual(baseline.read_text(), old)
            baseline.write_text(old)
            with patch.object(g, "measure", side_effect=RuntimeError("offline")):
                self.assertEqual(g.main(argv), 1)
            self.assertEqual(baseline.read_text(), old)
            self.assertTrue(json.loads(report.read_text())["skipped"])
            with patch.object(g, "measure") as measure:
                self.assertEqual(g.main(argv + ["--only", "typo"]), 2)
                measure.assert_not_called()
            with patch.object(g, "measure", return_value=(rows[:1], [])):
                self.assertEqual(g.main(argv + ["--only", "a"]), 0)
            self.assertEqual(set(json.loads(baseline.read_text())["entries"]), {"old", "a"})
            with patch.object(g, "measure", return_value=(rows, [])):
                self.assertEqual(g.main(argv), 0)
            self.assertEqual(set(json.loads(baseline.read_text())["entries"]), {"a", "b"})
            self.assertFalse(list(root.glob("*.tmp.*")))


if __name__ == "__main__":
    unittest.main(verbosity=1)
