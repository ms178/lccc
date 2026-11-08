#!/usr/bin/env python3
"""Self-tests for check_simd_commutativity_agreement.

A gate that cannot fail is decoration.  These tests pin both directions:
the real tree passes, and every defect class the gate claims to catch makes
it fail on a mutated copy of the real source.
"""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "check_simd_commutativity_agreement",
    Path(__file__).with_name("check_simd_commutativity_agreement.py"),
)
assert SPEC and SPEC.loader
gate = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = gate
SPEC.loader.exec_module(gate)

INTRINSICS = (ROOT / "src/backend/x86/codegen/intrinsics.rs").read_text()
SIMD = (ROOT / "src/backend/x86/codegen/intrinsics_simd.rs").read_text()


class NormalizationTests(unittest.TestCase):
    def test_vex_prefix_is_stripped(self):
        self.assertEqual(gate.base_mnemonic("vpmaddubsw"), "pmaddubsw")
        self.assertEqual(gate.base_mnemonic("pmaddubsw"), "pmaddubsw")

    def test_evex_bitwise_q_aliases_collapse(self):
        for name, want in (("vpandq", "pand"), ("vporq", "por"), ("vpxorq", "pxor")):
            self.assertEqual(gate.base_mnemonic(name), want)

    def test_unrelated_suffix_is_not_stripped(self):
        # `pmulhuw` must not be confused with `pmulhw`: only a LEADING v and
        # the three known q-aliases are normalized.
        self.assertEqual(gate.base_mnemonic("vpmulhuw"), "pmulhuw")
        self.assertEqual(gate.base_mnemonic("vpmulhw"), "pmulhw")


class ParseTests(unittest.TestCase):
    def test_real_source_parses_and_finds_all_three_widths(self):
        table = gate.parse_tables(INTRINSICS, SIMD)
        widths = {w for row in table.values() for w in row}
        self.assertEqual(widths, {"128", "256", "512"})
        self.assertIn("pmaddubsw", table)
        self.assertIn("pmaddwd", table)

    def test_missing_128_anchor_is_fail_closed(self):
        with self.assertRaises(gate.ParseError):
            gate.parse_tables("no such function here", SIMD)

    def test_missing_256_call_sites_is_fail_closed(self):
        stripped = INTRINSICS.replace("emit_avx_binary_256(", "emit_avx_binary_256_RENAMED(")
        with self.assertRaises(gate.ParseError):
            gate.parse_tables(stripped, SIMD)

    def test_missing_512_call_sites_is_fail_closed(self):
        stripped = SIMD.replace("emit_evex_binary_512(", "emit_evex_binary_512_RENAMED(")
        with self.assertRaises(gate.ParseError):
            gate.parse_tables(INTRINSICS, stripped)

    def test_empty_allowlist_is_fail_closed(self):
        # The function exists but its body has no mnemonic strings.
        broken = INTRINSICS
        start = broken.index("fn sse128_mnemonic_commutative(")
        end = broken.index("\n    }", start) + len("\n    }")
        broken = broken[:start] + "fn sse128_mnemonic_commutative(inst: &str) -> bool {\n        false\n    }" + broken[end:]
        with self.assertRaises(gate.ParseError):
            gate.parse_tables(broken, SIMD)


class CurrentTreeTests(unittest.TestCase):
    def test_real_tree_is_clean(self):
        table = gate.parse_tables(INTRINSICS, SIMD)
        self.assertEqual(gate.check(table), [])

    def test_sa05_maddubsw_is_non_commutative_at_every_width(self):
        table = gate.parse_tables(INTRINSICS, SIMD)
        for width, flags in table["pmaddubsw"].items():
            self.assertEqual(flags, {False}, f"pmaddubsw at {width}")


class MutationTests(unittest.TestCase):
    """Each mutation reintroduces a real defect; the gate MUST go red."""

    def test_reverting_sa05_is_detected(self):
        mutated = SIMD.replace(
            'self.emit_evex_binary_512(dptr, args, "vpmaddubsw", false);',
            'self.emit_evex_binary_512(dptr, args, "vpmaddubsw", true);',
        )
        self.assertTrue(mutated != SIMD, "mutation anchor did not match")
        problems = gate.check(gate.parse_tables(INTRINSICS, mutated))
        self.assertTrue(any("ASYMMETRIC" in p and "pmaddubsw" in p for p in problems), problems)

    def test_reverting_sa36_is_detected(self):
        mutated = INTRINSICS.replace(
            'self.emit_avx_binary_256(d, args, "vpmaddwd", true);',
            'self.emit_avx_binary_256(d, args, "vpmaddwd", false);',
        )
        self.assertTrue(mutated != INTRINSICS, "mutation anchor did not match")
        problems = gate.check(gate.parse_tables(mutated, SIMD))
        self.assertTrue(any("SYMMETRIC" in p and "pmaddwd" in p for p in problems), problems)

    def test_making_a_subtraction_commutative_is_detected(self):
        mutated = SIMD.replace(
            'self.emit_evex_binary_512(dptr, args, "vpsubd", false);',
            'self.emit_evex_binary_512(dptr, args, "vpsubd", true);',
        )
        self.assertTrue(mutated != SIMD, "mutation anchor did not match")
        problems = gate.check(gate.parse_tables(INTRINSICS, mutated))
        self.assertTrue(any("ASYMMETRIC" in p and "psubd" in p for p in problems), problems)

    def test_unlisted_mnemonic_drift_is_still_detected(self):
        # `pmulld` is in neither authority list; a fresh disagreement must
        # still be caught by the generic cross-width rule.
        mutated = SIMD.replace(
            'self.emit_evex_binary_512(dptr, args, "vpmulld", true);',
            'self.emit_evex_binary_512(dptr, args, "vpmulld", false);',
        )
        self.assertTrue(mutated != SIMD, "mutation anchor did not match")
        problems = gate.check(gate.parse_tables(INTRINSICS, mutated))
        self.assertTrue(any("disagrees across widths" in p and "pmulld" in p for p in problems), problems)


class AuthorityTests(unittest.TestCase):
    def test_asymmetric_and_symmetric_sets_are_disjoint(self):
        self.assertEqual(set(gate.ASYMMETRIC) & set(gate.SYMMETRIC), set())

    def test_authority_keys_are_normalized(self):
        for name in list(gate.ASYMMETRIC) + list(gate.SYMMETRIC):
            self.assertEqual(gate.base_mnemonic(name), name, f"{name} is not a base name")

    def test_every_authority_entry_carries_a_reason(self):
        for name, reason in list(gate.ASYMMETRIC.items()) + list(gate.SYMMETRIC.items()):
            self.assertTrue(reason.strip(), name)
            self.assertGreater(len(reason), 15, f"{name}: reason is too terse to audit")


if __name__ == "__main__":
    unittest.main(verbosity=2)
