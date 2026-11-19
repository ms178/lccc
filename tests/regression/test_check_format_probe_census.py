#!/usr/bin/env python3
"""Non-vacuity contracts for check_format_probe_census.py.

A ratchet that cannot see the defect it pins is worse than no ratchet; each
contract below feeds the scanner a known-positive or known-negative shape and
requires the verdict to match.  The shapes are the real ones from the tree:
the DHAT-measured scan matcher probe, the i686 equality probe, the compliant
static-spelling and chained-strip forms they were replaced with, and a test
module that must not be counted.
"""
from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_format_probe_census as census_mod  # noqa: E402


def count(source: str) -> int:
    with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as fh:
        fh.write(source)
        path = Path(fh.name)
    try:
        return census_mod.census(path)
    finally:
        path.unlink()


class FormatProbeCensusContracts(unittest.TestCase):
    def test_strip_prefix_format_is_counted(self) -> None:
        # The exact DHAT-measured shape from match_preloaded_scan_copy.
        src = 't[1].strip_prefix(&format!("{mn}{} ", sfx)).map(|o| (mn, q, o))\n'
        self.assertEqual(count(src), 1)

    def test_equality_probe_is_counted_both_ways(self) -> None:
        self.assertEqual(count('if s == format!("testl {rn}, {rn}") {\n'), 1)
        self.assertEqual(count('if format!("jmp {t}") == s {\n'), 1)
        self.assertEqual(count('if mnem != format!("mov{width}") {\n'), 1)

    def test_suffix_and_contains_probes_are_counted(self) -> None:
        self.assertEqual(count('rest.strip_suffix(&format!(", {rn}"))\n'), 1)
        self.assertEqual(count('t.ends_with(&format!(", {x}"))\n'), 1)
        self.assertEqual(count('t.contains(&format!("%{n}"))\n'), 1)

    def test_compliant_replacements_are_not_counted(self) -> None:
        src = (
            'let probe = if q { "bsfq " } else { "bsfl " };\n'
            't[1].strip_prefix(probe)?;\n'
            'rest.strip_suffix(rn).and_then(|r| r.strip_suffix(", "))\n'
            'let owned = format!("mov {k}, x0");  // hoisted, reused\n'
            'lines.iter().any(|l| l.starts_with(&owned))\n'
        )
        self.assertEqual(count(src), 0)

    def test_comments_do_not_count(self) -> None:
        self.assertEqual(count('// s == format!("testl {rn}, {rn}") was the old form\n'), 0)

    def test_test_modules_are_excluded(self) -> None:
        src = (
            'fn production(s: &str) -> bool {\n'
            '    s == format!("jmp {}", 1)\n'
            '}\n'
            '#[cfg(test)]\n'
            'mod tests {\n'
            '    fn f(a: &str) { assert!(a.starts_with(&format!("mov {},", 2))); }\n'
            '}\n'
        )
        self.assertEqual(count(src), 1)

    def test_baseline_matches_tree(self) -> None:
        # The pinned census must equal the live tree: a drift in either
        # direction means someone changed sites without running the gate.
        current = census_mod.collect()
        import json
        baseline = json.loads(census_mod.BASELINE.read_text())
        above = {k: (baseline.get(k, 0), v) for k, v in current.items()
                 if v > baseline.get(k, 0)}
        self.assertEqual(above, {}, "sites above baseline; the main gate must fail")

    def test_hot_sites_stay_fixed(self) -> None:
        # The two DHAT-measured matchers and the i686 window probe must never
        # regain a format!-built probe: their files are pinned at (or below)
        # the post-fix counts.
        import json
        baseline = json.loads(census_mod.BASELINE.read_text())
        current = census_mod.collect()
        for rel in (
            "src/backend/x86/codegen/peephole/passes/local_patterns.rs",
            "src/backend/i686/codegen/peephole.rs",
            "src/backend/riscv/codegen/peephole.rs",
            "src/backend/x86/codegen/peephole/passes/memory_fold.rs",
        ):
            self.assertLessEqual(current.get(rel, 0), baseline.get(rel, 0), rel)


if __name__ == "__main__":
    unittest.main(verbosity=2)
