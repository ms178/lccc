#!/usr/bin/env python3
"""slice_superopt contracts: proven rewrites only, missed alarms impossible.

The exhaustive 8-bit table is the tool's Alive2; these tests pin that (a)
known strength reductions are found and pass the wider-width gate, (b) known
*inequivalent* pairs are never confused (negative controls), and (c) the
truth-table builder agrees with direct evaluation."""
import random
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import slice_superopt as ss


class TableTests(unittest.TestCase):
    def test_table_matches_direct_evaluation(self):
        for text in ("x*7", "(x<<3)-x", "~x+1", "(x&15)*3"):
            node = ss.parse_spec(text)
            tt = ss.tt_of(node, 1)
            for x in (0, 1, 5, 31, 128, 255):
                self.assertEqual(tt[x], ss.ev(node, (x, 0), 8, 255), text)


class SynthesisTests(unittest.TestCase):
    def test_finds_strength_reductions(self):
        for text in ("x*3", "x*5", "x*7", "x*9", "x*15"):
            spec = ss.parse_spec(text)
            best, stats = ss.synthesise(spec)
            self.assertIsNotNone(best, text)
            self.assertLess(ss.cost(best), ss.cost(spec), text)
            self.assertEqual(ss.tt_of(best, 1), ss.tt_of(spec, 1), text)
            self.assertTrue(ss.verify_wider(spec, best, random.Random(4)), text)

    def test_negative_controls_never_accepted(self):
        # The missed-alarm hunt: if any of these tables ever compare equal,
        # the checker would bless a miscompilation.
        for spec_text, bad_text in (("x*3", "x*4"), ("x<<1", "x<<2"),
                                    ("x&15", "x&7"), ("x+1", "x-1")):
            self.assertNotEqual(ss.tt_of(ss.parse_spec(spec_text), 1),
                                ss.tt_of(ss.parse_spec(bad_text), 1),
                                f"{bad_text} vs {spec_text}")

    def test_equivalent_control_recognised(self):
        self.assertEqual(ss.tt_of(ss.parse_spec("x+x"), 1),
                         ss.tt_of(ss.parse_spec("x<<1"), 1))

    def test_two_var_synthesis_proves_adder_identities(self):
        # F-3: 2-var search keys candidates by sampled signature, but every
        # acceptance is still the exhaustive 65536-point proof -- asserted
        # here by comparing full tables, not signatures.
        for text in ("(x&y)+(x|y)", "(x^y)+((x&y)<<1)"):
            spec = ss.parse_spec(text)
            best, stats = ss.synthesise(spec)
            self.assertIsNotNone(best, text)
            self.assertEqual(ss.to_c(best), "(x + y)", text)
            self.assertEqual(ss.tt_of(best, 2), ss.tt_of(spec, 2), text)
            self.assertTrue(stats["sampled_signature"], text)
            self.assertFalse(stats["search_aborted"], text)
            self.assertTrue(ss.verify_wider(spec, best, random.Random(4)), text)

    def test_two_var_negative_control(self):
        # x<y?x:y (min) must never be "proven" equal to its swap, at
        # signature or table level, and synthesis must not invent a
        # cheaper equivalent that is not exhaustively equal.
        spec, bad = ss.parse_spec("x<y?x:y"), ss.parse_spec("x<y?y:x")
        self.assertNotEqual(ss.tt_of(spec, 2), ss.tt_of(bad, 2))
        best, _ = ss.synthesise(spec)
        if best is not None:
            self.assertEqual(ss.tt_of(best, 2), ss.tt_of(spec, 2))


class ParserTests(unittest.TestCase):
    def test_c_precedence_tiers(self):
        # | binds loosest, then ^, then &, then <, then shifts, then +-,
        # then *: unparenthesised specs must mean what C means.
        self.assertEqual(ss.to_c(ss.parse_spec("x|y&3")), "(x | (y & 3))")
        self.assertEqual(ss.to_c(ss.parse_spec("x^y|3")), "((x ^ y) | 3)")
        self.assertEqual(ss.to_c(ss.parse_spec("x&1<<2")), "(x & (1 << 2))")
        self.assertEqual(ss.to_c(ss.parse_spec("x+1<<2")), "((x + 1) << 2)")
        self.assertEqual(ss.to_c(ss.parse_spec("x*2+3")), "((x * 2) + 3)")
        self.assertEqual(ss.to_c(ss.parse_spec("x<<1<5")), "((x << 1) < 5)")

    def test_var_shift_semantics_flagged(self):
        # Variable shift counts are evaluated with masked (hardware)
        # semantics here but are UB in C at >= width: such specs must be
        # flagged so nobody quotes a proof as a C rewrite.
        self.assertTrue(ss.has_var_shift(ss.parse_spec("x<<y")))
        self.assertTrue(ss.has_var_shift(ss.parse_spec("(x>> (y&3))+1")))
        self.assertFalse(ss.has_var_shift(ss.parse_spec("x<<3")))
        self.assertFalse(ss.has_var_shift(ss.parse_spec("(x&y)+((x<<1)&255)")))

    def test_constants_masked_to_uint8(self):
        # The AST, the printed form and the table must agree: 300 is 44.
        node = ss.parse_spec("x*300")
        self.assertEqual(ss.to_c(node), "(x * 44)")
        self.assertEqual(ss.tt_of(node, 1), ss.tt_of(ss.parse_spec("x*44"), 1))


if __name__ == "__main__":
    unittest.main()
