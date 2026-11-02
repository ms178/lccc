#!/usr/bin/env python3
"""Compiler-free path-enumeration tests for scripts/differential_corpus.sh.

PR #721 review P1-4: the corpus exclusion must hold for every equivalent
spelling of the corpus root (relative, ./-prefixed, absolute, trailing
slash, spaces in names) and the enumeration must never differ between them.
The tests drive the REAL script (`--list` mode) so the tested pipeline is
the shipped pipeline.

Run:  python3 scripts/test_differential_corpus_paths.py
"""
from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SCRIPT = REPO / "scripts" / "differential_corpus.sh"


def enumerate_c(root: str) -> list[str]:
    proc = subprocess.run(["bash", str(SCRIPT), "--list", root],
                          capture_output=True, text=True)
    if proc.returncode != 0:
        raise AssertionError(f"--list {root!r} failed: {proc.stderr}")
    return sorted(l for l in proc.stdout.splitlines() if l)


class CorpusExclusionParityTest(unittest.TestCase):
    def test_equivalent_roots_select_identically(self):
        spellings = ["tests", "./tests", "tests/", str(REPO / "tests"),
                     str(REPO / "tests") + "/"]
        sets = [set(enumerate_c(s)) for s in spellings]
        for s in sets[1:]:
            self.assertEqual(s, sets[0])

    def test_default_root_excludes_every_imported_file(self):
        selected = enumerate_c("tests")
        imported = [p for p in selected if "/corpus/clang-c/" in p]
        self.assertEqual(imported, [])
        self.assertGreater(len(selected), 500)  # the harness corpus itself

    def test_direct_corpus_root_selects_nothing_explicitly(self):
        selected = enumerate_c("tests/corpus")
        self.assertEqual(selected, [])

    def test_spaces_in_root_survive(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "my tests"
            (root / "sub").mkdir(parents=True)
            (root / "sub" / "a b.c").write_text("int x;\n")
            sel = enumerate_c(str(root))
            self.assertEqual(len(sel), 1)
            self.assertTrue(sel[0].endswith("a b.c"))

    def test_missing_root_is_a_usage_error(self):
        proc = subprocess.run(
            ["bash", str(SCRIPT), "--list", "/nonexistent/corpus"],
            capture_output=True, text=True)
        self.assertEqual(proc.returncode, 2)


if __name__ == "__main__":
    unittest.main(verbosity=1)
