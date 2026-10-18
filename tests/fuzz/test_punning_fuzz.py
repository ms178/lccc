"""Self-tests for punning_fuzz.py: the reference model is the oracle, so it is
tested first (arithmetic/encoding), then its agreement with GCC is checked
when a GCC is available."""
from __future__ import annotations

import random
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import punning_fuzz as pf  # noqa: E402


class ModelArithmetic(unittest.TestCase):
    def test_trunc_is_modular_two_complement(self):
        self.assertEqual(pf.trunc(0x80, "i8"), -128)
        self.assertEqual(pf.trunc(0xFF, "i8"), -1)
        self.assertEqual(pf.trunc(-1, "u16"), 0xFFFF)
        self.assertEqual(pf.trunc(0x1_0000_0000, "u32"), 0)
        self.assertEqual(pf.trunc(0x8000_0000, "i32"), -(2**31))
        self.assertEqual(pf.trunc(2**63, "i64"), -(2**63))

    def test_encoding_is_little_endian(self):
        self.assertEqual(pf.encode(0x11223344, 4), b"\x44\x33\x22\x11")
        self.assertEqual(pf.encode(-2, 2), b"\xfe\xff")
        # Reading a wider store through a narrower type sees the low bytes.
        self.assertEqual(pf.decode(pf.encode(0x11223344, 4)[:1], "u8"), 0x44)
        self.assertEqual(pf.decode(pf.encode(0x11223344, 4)[2:], "u16"), 0x1122)

    def test_encode_decode_round_trip_all_types(self):
        rng = random.Random(1)
        for ty, (size, _signed, _c) in pf.TYPES.items():
            for _ in range(200):
                v = pf.trunc(rng.getrandbits(64), ty)
                self.assertEqual(pf.decode(pf.encode(v, size), ty), v, ty)


class FloatAndWideModel(unittest.TestCase):
    """Mutation-sensitive checks for the T7 additions to the model."""

    def test_stored_floats_are_never_inf_or_nan(self):
        rng = random.Random(7)
        for ty in pf.FLOAT_TYPES:
            size = pf.FLOAT_TYPES[ty]
            exp_shift = 23 if size == 4 else 52
            exp_mask = 0xFF if size == 4 else 0x7FF
            for _ in range(2000):
                b = pf.store_bits(rng.getrandbits(64), ty)
                self.assertNotEqual((b >> exp_shift) & exp_mask, exp_mask, (ty, hex(b)))

    def test_finite_bits_is_identity_on_finite_values(self):
        self.assertEqual(pf.finite_bits(0x3F800000, "f32"), 0x3F800000)  # 1.0f
        self.assertEqual(pf.finite_bits(0x7F800000, "f32"), 0x3F800000)  # +inf -> 1.0f (exponent top bit cleared)
        self.assertEqual(pf.finite_bits(0x3FF0000000000000, "f64"), 0x3FF0000000000000)  # 1.0
        self.assertEqual(pf.finite_bits(0x7FF0000000000000, "f64"), 0x3FF0000000000000)

    def test_wide_loads_split_into_two_words(self):
        data = bytes(range(16))
        lo = int.from_bytes(data[:8], "little")
        hi = int.from_bytes(data[8:], "little")
        self.assertEqual(pf.load_words(data, "u128"), [lo, hi])
        self.assertEqual(pf.load_words(data, "i128"), [lo, hi])

    def test_signed_small_loads_are_sign_extended_like_c(self):
        self.assertEqual(pf.load_words(b"\xff", "i8"), [pf.MASK64])
        self.assertEqual(pf.load_words(b"\x01", "u8"), [1])

    def test_model_covers_the_new_types(self):
        seen = set()
        for seed in range(400):
            for ops in pf.gen_case(seed).bodies:
                seen.update(op.ty for op in ops)
        self.assertTrue({"f32", "f64", "u128", "i128"} <= seen, seen)


class GeneratorShape(unittest.TestCase):
    def test_generation_is_deterministic(self):
        for seed in (0, 7, 999):
            self.assertEqual(pf.generate(pf.gen_case(seed)), pf.generate(pf.gen_case(seed)))
            self.assertEqual(pf.run_model(pf.gen_case(seed)), pf.run_model(pf.gen_case(seed)))

    def test_every_access_stays_inside_the_object(self):
        for seed in range(300):
            for ops in pf.gen_case(seed).bodies:
                for op in ops:
                    size = pf.TYPES[op.ty][0]
                    self.assertLessEqual(op.off + size, pf.OBJECT_SIZE, (seed, op))
                    if op.kind in ("uset", "uget"):
                        self.assertEqual(op.off % size, 0, "union arrays are naturally aligned")

    def test_model_output_format(self):
        out = pf.run_model(pf.gen_case(5))
        self.assertRegex(out, r"^[0-9a-f]{16} [0-9a-f]{16}\n$")


@unittest.skipUnless(shutil.which("gcc"), "gcc not available")
class ModelAgreesWithGcc(unittest.TestCase):
    """Oracle self-check: a disagreement here is a model or generator bug."""

    def test_small_corpus_matches_gcc(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            for seed in range(6):
                for level in ("O0", "O2"):
                    result = pf.one_case(pf.Job(seed, "gcc", level, "base", out, 60))
                    self.assertEqual(result.status, "pass",
                                     f"seed {seed} -{level}: {result.detail}")

    def test_v3_lane_matches_gcc(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            for seed in range(3):
                result = pf.one_case(pf.Job(seed, "gcc", "O2", "v3", out, 60))
                self.assertEqual(result.status, "pass",
                                 f"seed {seed} v3: {result.detail}")


if __name__ == "__main__":
    unittest.main()
