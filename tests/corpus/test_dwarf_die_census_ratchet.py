#!/usr/bin/env python3
"""Regression tests for the DIE-vocabulary ratchet's integrity.

`tests/regression/check_dwarf_die_census.py` has two halves: a baseline of what
LCCC emits (losing any of it must fail) and a deferral list of what the
reference compilers emit and LCCC does not (a new one must fail until
declared). Anything listed in BOTH halves is invisible to both checks: a
regression that removes it is excused by the deferral while the baseline still
claims it is present.

That was not hypothetical. The gate previously pinned 3 of the 12 tags LCCC
actually emits and pre-declared 15 tags plus 5 attributes as "deferred" that
the compiler had already implemented -- losing the entire C type graph, every
variable DIE or every parameter DIE would have produced a green gate.

These tests pin the invariants, measure the live vocabulary against the
compiled binary, and prove with negative controls that a stale deferral is
rejected. Dynamic checks skip when the compiler/readelf/oracles are absent, so
the corpus stays portable; the static invariants always run.
"""

from __future__ import annotations

import importlib.util
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GATE = ROOT / "tests/regression/check_dwarf_die_census.py"

# The union over -gdwarf-2..5 of the census fixture, measured with binutils
# 2.47 readelf against the rebuilt compiler. A deliberate capability addition
# must update this table AND the gate together, with a reason in the commit.
MEASURED_LCCC_TAGS = frozenset({
    "DW_TAG_array_type", "DW_TAG_base_type", "DW_TAG_compile_unit",
    "DW_TAG_formal_parameter", "DW_TAG_member", "DW_TAG_pointer_type",
    "DW_TAG_structure_type", "DW_TAG_subprogram", "DW_TAG_subrange_type",
    "DW_TAG_typedef", "DW_TAG_unspecified_type", "DW_TAG_variable",
})
MEASURED_LCCC_ATTRIBUTES = frozenset({
    "DW_AT_byte_size", "DW_AT_comp_dir", "DW_AT_data_member_location",
    "DW_AT_decl_column", "DW_AT_decl_file", "DW_AT_decl_line",
    "DW_AT_encoding", "DW_AT_external", "DW_AT_frame_base",
    "DW_AT_high_pc", "DW_AT_language", "DW_AT_language_name",
    "DW_AT_language_version", "DW_AT_location", "DW_AT_low_pc",
    "DW_AT_name", "DW_AT_producer", "DW_AT_stmt_list", "DW_AT_type",
    "DW_AT_upper_bound",
})
MEASURED_DEFERRED_TAGS = frozenset({
    "DW_TAG_enumeration_type", "DW_TAG_enumerator", "DW_TAG_lexical_block",
})
MEASURED_DEFERRED_ATTRIBUTES = frozenset({
    "DW_AT_GNU_all_call_sites", "DW_AT_GNU_all_tail_call_sites",
    "DW_AT_addr_base", "DW_AT_call_all_calls", "DW_AT_call_all_tail_calls",
    "DW_AT_const_value", "DW_AT_count", "DW_AT_prototyped",
    "DW_AT_sibling", "DW_AT_str_offsets_base",
})


def load_gate():
    spec = importlib.util.spec_from_file_location("die_census_gate", GATE)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def optional(command: str) -> str | None:
    found = shutil.which(command)
    if found:
        return found
    path = ROOT / command
    return str(path.resolve()) if path.exists() else None


class DieCensusRatchet(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.gate = load_gate()

    def test_baseline_pins_the_whole_measured_vocabulary(self) -> None:
        self.assertEqual(set(self.gate.BASELINE_TAGS), MEASURED_LCCC_TAGS)
        self.assertEqual(set(self.gate.BASELINE_ATTRIBUTES), MEASURED_LCCC_ATTRIBUTES)
        self.assertEqual(len(self.gate.BASELINE_TAGS), 12)
        self.assertEqual(len(self.gate.BASELINE_ATTRIBUTES), 20)

    def test_deferrals_are_exactly_the_measured_gap(self) -> None:
        self.assertEqual(set(self.gate.DEFERRED_TAGS), MEASURED_DEFERRED_TAGS)
        self.assertEqual(set(self.gate.DEFERRED_ATTRIBUTES),
                         MEASURED_DEFERRED_ATTRIBUTES)

    def test_baseline_and_deferrals_are_disjoint(self) -> None:
        self.assertFalse(set(self.gate.DEFERRED_TAGS) & set(self.gate.BASELINE_TAGS))
        self.assertFalse(set(self.gate.DEFERRED_ATTRIBUTES)
                         & set(self.gate.BASELINE_ATTRIBUTES))

    def test_every_deferral_has_a_nonempty_reason(self) -> None:
        for table in (self.gate.DEFERRED_TAGS, self.gate.DEFERRED_ATTRIBUTES):
            for name, reason in table.items():
                with self.subTest(name=name):
                    self.assertTrue(name.startswith(("DW_TAG_", "DW_AT_")))
                    self.assertTrue(reason.strip())

    def test_a_stale_deferral_is_rejected_at_import(self) -> None:
        """Negative controls: re-adding one already-implemented tag or
        attribute to the deferral list must fail the module's own assertion."""
        source = GATE.read_text()
        for needle, tag in (
            ('DEFERRED_TAGS = {', '"DW_TAG_base_type": "stale",'),
            ('DEFERRED_ATTRIBUTES = {', '"DW_AT_location": "stale",'),
        ):
            with self.subTest(tag=tag):
                mutated = source.replace(
                    needle, needle + "\n    " + tag, 1,
                )
                self.assertNotEqual(mutated, source)
                with tempfile.TemporaryDirectory() as tmp:
                    path = Path(tmp) / "gate.py"
                    path.write_text(mutated)
                    spec = importlib.util.spec_from_file_location("mutated_gate", path)
                    module = importlib.util.module_from_spec(spec)
                    with self.assertRaises(AssertionError):
                        spec.loader.exec_module(module)

    def test_losing_a_baseline_tag_would_fail_the_gate(self) -> None:
        """End-to-end negative control: run a copy of the gate whose census
        reports one fewer LCCC tag. The tightened baseline must turn that into
        a non-zero exit with a `lost` diagnostic. Under the old 3-tag baseline
        the same mutation passed, which is the defect this suite pins."""
        lccc = optional(os.environ.get("CCC", "target/fastbuild/lccc"))
        gcc = optional(os.environ.get("GCC16", "gcc-16"))
        readelf = optional(os.environ.get("READELF", "readelf"))
        if not all((lccc, gcc, readelf)):
            self.skipTest("rebuilt lccc, gcc-16 or readelf unavailable")
        source = GATE.read_text()
        needle = 'ours_tags |= found["tags"]'
        self.assertIn(needle, source)
        mutated = source.replace(
            needle, needle + ' - {"DW_TAG_variable"}', 1,
        )
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "gate.py"
            path.write_text(mutated)
            proc = subprocess.run(
                [shutil.which("python3") or "python3", str(path),
                 "--lccc", lccc, "--gcc", gcc, "--readelf", readelf,
                 "--versions", "5"],
                cwd=ROOT, capture_output=True, text=True, timeout=180,
            )
        self.assertNotEqual(proc.returncode, 0,
                            "a lost DW_TAG_variable was not caught:\n" + proc.stdout)
        self.assertIn("lost", proc.stdout + proc.stderr)

    def test_live_gate_passes(self) -> None:
        lccc = optional(os.environ.get("CCC", "target/fastbuild/lccc"))
        gcc = optional(os.environ.get("GCC16", "gcc-16"))
        clang = optional(os.environ.get("CLANG23", "clang-23"))
        readelf = optional(os.environ.get("READELF", "readelf"))
        if not all((lccc, gcc, clang, readelf)):
            self.skipTest("rebuilt lccc, gcc-16, clang-23 or readelf unavailable")
        proc = subprocess.run(
            [shutil.which("python3") or "python3", str(GATE),
             "--lccc", lccc, "--gcc", gcc, "--clang", clang,
             "--readelf", readelf],
            cwd=ROOT, capture_output=True, text=True, timeout=180,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("DWARF DIE census: PASS", proc.stdout)
        self.assertIn("12 tags, 20 attributes", proc.stdout)


if __name__ == "__main__":
    unittest.main()
