#!/usr/bin/env python3
"""Mutpair invariants: the execution filter must never mislabel a pair.

The whole method stands on two claims: an "inequivalent" pair really does
differ on some judged input, and an "equivalent" pair really does not.  A
harness that gets either backwards manufactures false miscompile reports (or,
worse, silent passes) -- the missed-alarm failure this tool exists to hunt.
"""
import random
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mutpair


class FilterTests(unittest.TestCase):
    def test_polarity_labels_are_true(self):
        kept = {"inequivalent": 0, "equivalent": 0}
        for seed in range(300):
            pair = mutpair.make_pair(seed)
            if pair is None:
                continue
            kept[pair.polarity] += 1
            if pair.polarity == "inequivalent":
                self.assertNotEqual(pair.expected_orig, pair.expected_mut)
            else:
                self.assertEqual(pair.expected_orig, pair.expected_mut)
        # The filter must not starve either polarity (else CI tests nothing).
        self.assertGreater(kept["inequivalent"], 30)
        self.assertGreater(kept["equivalent"], 30)

    def test_generated_c_is_deterministic(self):
        for seed in (1, 17, 10_000):
            a, b = mutpair.make_pair(seed), mutpair.make_pair(seed)
            self.assertEqual(a is None, b is None)
            if a is not None:
                self.assertEqual(mutpair.pair_to_c(a), mutpair.pair_to_c(b))

    def test_all_shapes_reachable_and_model_never_raises(self):
        # F-2: loop/mem/call shapes must actually occur, and modelling any
        # kept pair (either side, any judged vector) must stay inside the
        # UB-freedom invariant -- the model raises on %-by-zero, so a raise
        # here means generation broke its own contract.
        shapes = {}
        for seed in range(400):
            pair = mutpair.make_pair(90_000 + seed)
            if pair is None:
                continue
            shapes[pair.shape] = shapes.get(pair.shape, 0) + 1
            for v in pair.vectors:
                mutpair.model_program(pair.prog_orig, v)
                mutpair.model_program(pair.prog_mut, v)
        for want in ("expr", "loop", "mem", "call"):
            self.assertGreater(shapes.get(want, 0), 10, f"{want}: {shapes}")

    def test_attribute_decoration_is_emitted(self):
        # F-5: some kept pairs must carry GNU attributes (the suspected
        # blind-spot dimension), identically on both pair members.
        decorated = 0
        for seed in range(200):
            pair = mutpair.make_pair(110_000 + seed)
            if pair is None:
                continue
            src = mutpair.pair_to_c(pair)
            if pair.prog_orig["attrs"] != "plain":
                decorated += 1
                self.assertIn("__attribute__", src)
                self.assertEqual(pair.prog_orig["attrs"], pair.prog_mut["attrs"])
        self.assertGreater(decorated, 20)

    def test_no_shift_count_mutation_breaks_ub_freedom(self):
        # Structural contract: every shift count in every generated or
        # mutated AST is (<expr> & 31), and every %-divisor a non-zero
        # constant.  Checked on the ASTs themselves -- NOT via model
        # evaluation, which masks counts by construction and therefore
        # cannot observe an unmasked count in the emitted C.  (The
        # original model-level version of this test passed while operand
        # swap on shifts was emitting unmasked counts in ~1% of pairs:
        # a missed alarm inside the missed-alarm harness.)
        rng = random.Random(0xBADC0DE)
        checked = 0
        for seed in range(600):
            pair = mutpair.make_pair(50_000 + seed)
            if pair is not None:
                checked += 1  # make_pair already ran validate_*; re-audit
                mutpair.validate_program_ub_free(pair.prog_orig, "orig")
                mutpair.validate_program_ub_free(pair.prog_mut, "mut")
            # Hammer the mutators directly so pairs the execution filter
            # would discard are audited too.
            gen = mutpair.PairGenerator(60_000 + seed)
            tree = gen.function()
            mutpair.validate_ub_free(tree, "gen")
            mutpair.validate_ub_free(
                mutpair.mutate_inequivalent(tree, rng), "mut-ineq")
            mutpair.validate_ub_free(
                mutpair.mutate_equivalent(tree, rng), "mut-eq")
        self.assertGreater(checked, 100)

    def test_ub_validator_rejects_known_bad_trees(self):
        # Negative controls for the validator itself: a checker that has
        # only ever seen good trees has never been tested.
        bad_shift = ("bin", "<<", ("var", 0), ("var", 1))
        bad_shift2 = ("bin", ">>", ("bin", "&", ("var", 0), ("const", 31)),
                      ("var", 2))  # the historical operand-swap shape
        bad_div = ("bin", "%", ("var", 0), ("var", 1))
        bad_div0 = ("bin", "%", ("var", 0),
                    ("bin", "-", ("const", 3), ("const", 3)))
        for bad in (bad_shift, bad_shift2, bad_div, bad_div0):
            with self.assertRaises(AssertionError, msg=repr(bad)):
                mutpair.validate_ub_free(bad, "control")
        # ...and it is not trivially all-no:
        good = ("bin", "<<", ("var", 0),
                ("bin", "&", ("var", 1), ("const", 31)))
        mutpair.validate_ub_free(good, "control")


@unittest.skipUnless(shutil.which("gcc"), "gcc required for oracle cross-check")
class OracleTests(unittest.TestCase):
    def test_python_model_agrees_with_gcc(self):
        repro = Path(tempfile.mkdtemp(prefix="mutpair_test_"))
        checked = 0
        for seed in range(40):
            pair = mutpair.make_pair(70_000 + seed)
            if pair is None:
                continue
            checked += 1
            fails = mutpair.judge_pair(pair, {"gcc": ["gcc"]}, ["-O0", "-O2"], repro)
            self.assertEqual(fails, [], f"seed {pair.seed}: {fails}")
        self.assertGreater(checked, 10)

    def test_negative_control_is_caught(self):
        repro = Path(tempfile.mkdtemp(prefix="mutpair_test_neg_"))
        with tempfile.TemporaryDirectory() as td:
            fake = Path(td) / "fakecc.py"
            fake.write_text(mutpair.FAKE_OPTIMISER)
            orig = ("bin", "+", ("bin", "*", ("var", 0), ("const", 3)), ("var", 1))
            vecs = mutpair.input_vectors(random.Random(3))
            eo = [mutpair.evaluate(orig, v) for v in vecs]
            pair = mutpair.Pair(-1, "equivalent", orig,
                                ("bin", "+", orig, ("const", 0)), vecs, eo, eo)
            fails = mutpair.judge_pair(pair, {"fake": [sys.executable, str(fake)]},
                                       ["-O2"], repro)
            self.assertTrue(fails, "fake optimiser escaped: harness missed-alarm hole")


if __name__ == "__main__":
    unittest.main()
