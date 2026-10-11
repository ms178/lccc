"""Offline checkpoint, exact-corpus and alias contracts for the bounded screen."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
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

    def test_failures_are_not_ranked(self):
        with tempfile.TemporaryDirectory() as td, patch.object(screen.cg, "main", return_value=1):
            self.assertEqual(screen.main(["--mine", "stub", "--only", "ring_fifo", "--out", td]), 1)
            result = json.loads((Path(td) / "index.json").read_text())
            self.assertTrue(result["complete"])
            self.assertEqual(result["worst_first"], [])
            self.assertEqual(len(result["failures"]), 1)

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
