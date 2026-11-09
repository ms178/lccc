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


COALESCING = (ROOT / "src/backend/stack_layout/copy_coalescing.rs").read_text()


class CrossLayerTests(unittest.TestCase):
    """The IR admission table is the fourth authority; it must agree too."""

    def test_brace_matching_does_not_pair_an_arm_with_a_later_emit(self):
        emitter = gate.parse_emitter_op_flags(INTRINSICS)
        # A greedy regex mapped LoadI32x4 -> vaddpd. Loads/stores never reach
        # emit_avx_binary_256, so any such mapping means the scan is broken.
        for op, (mnemonic, _) in emitter.items():
            self.assertFalse(
                op.startswith(("Load", "Store")),
                f"{op} was paired with {mnemonic}: brace matching is broken",
            )
        self.assertGreater(len(emitter), 40)

    def test_grouped_admission_arms_are_all_recorded(self):
        table = gate.parse_admission(COALESCING, "memfold_consumer_256")
        # VecAndI64x4 | VecOrI64x4 | VecXorI64x4 => Some(true) is one arm.
        for op in ("VecAndI64x4", "VecOrI64x4", "VecXorI64x4"):
            self.assertIs(table.get(op), True, op)
        self.assertIs(table.get("VecSubI64x4"), False)

    def test_real_tree_cross_layer_is_clean(self):
        emitter = gate.parse_emitter_op_flags(INTRINSICS)
        admissions = {
            f: gate.parse_admission(COALESCING, f)
            for f in ("memfold_consumer_128", "memfold_consumer_256")
        }
        self.assertEqual(gate.check_cross_layer(emitter, admissions), [])

    def test_maddwd_admission_matches_the_emitter(self):
        table = gate.parse_admission(COALESCING, "memfold_consumer_256")
        self.assertIs(table["VecMaddwdI16x16"], True, "symmetric: both slots fold")
        self.assertIs(table["VecMaddubsU8x32"], False, "mixed signedness: src2 only")

    def test_reverting_the_admission_fix_is_detected(self):
        mutated = COALESCING.replace(
            "O::VecMaddwdI16x16 => Some(true),", "O::VecMaddwdI16x16 => Some(false),"
        )
        self.assertTrue(mutated != COALESCING, "mutation anchor did not match")
        emitter = gate.parse_emitter_op_flags(INTRINSICS)
        admissions = {"memfold_consumer_256": gate.parse_admission(mutated, "memfold_consumer_256")}
        problems = gate.check_cross_layer(emitter, admissions)
        self.assertTrue(
            any("VecMaddwdI16x16" in p and "admission" in p.lower() or
                "VecMaddwdI16x16" in p and "memfold_consumer_256" in p for p in problems),
            problems,
        )

    def test_missing_admission_table_is_fail_closed(self):
        with self.assertRaises(gate.ParseError):
            gate.parse_admission("no such function", "memfold_consumer_256")


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


class ScanIntegrityTests(unittest.TestCase):
    """The brace scan must refuse to report a mis-paired table.

    `_brace_arms` counts braces without lexing strings or comments, so the
    parser carries an invariant on the real file: a pure data-movement op can
    never map to an arithmetic mnemonic.  These tests prove the invariant is
    not decoration.
    """

    def test_historical_mispairing_is_refused(self):
        # The exact shape the old greedy regex produced.
        bad = (
            'IntrinsicOp::LoadI32x4 => { '
            'emit_avx_binary_256(a, args, "vaddpd", true) }'
        )
        with self.assertRaises(gate.ParseError) as cm:
            gate.parse_emitter_op_flags(bad)
        self.assertIn("mis-paired", str(cm.exception))

    def test_store_and_const_mispairings_are_refused(self):
        for op in ("StoreI32x4", "VecConstI8x32"):
            for mnem in ("vpaddd", "vmulps", "vpmaddwd", "vxorpd"):
                bad = (
                    f'IntrinsicOp::{op} => {{ '
                    f'emit_avx_binary_256(a, args, "{mnem}", true) }}'
                )
                with self.assertRaises(gate.ParseError, msg=f"{op}/{mnem}"):
                    gate.parse_emitter_op_flags(bad)

    def test_a_legitimate_load_mnemonic_is_accepted(self):
        good = (
            'IntrinsicOp::LoadI32x4 => { '
            'emit_avx_binary_256(a, args, "vmovdqu", true) }'
        )
        self.assertEqual(
            gate.parse_emitter_op_flags(good), {"LoadI32x4": ("vmovdqu", True)}
        )

    def test_real_tree_has_no_data_movement_op_on_an_arithmetic_mnemonic(self):
        src = (ROOT / "src/backend/x86/codegen/intrinsics.rs").read_text()
        table = gate.parse_emitter_op_flags(src)
        for op, (mnem, _f) in table.items():
            if "Load" in op or "Store" in op or "Const" in op:
                self.fail(f"{op} -> {mnem}: scan mis-paired the real source")


if __name__ == "__main__":
    unittest.main(verbosity=2)
