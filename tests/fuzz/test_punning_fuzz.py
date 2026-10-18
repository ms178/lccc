"""Self-tests for punning_fuzz.py: the reference model is the oracle, so it is
tested first (arithmetic/encoding), then its agreement with GCC is checked
when a GCC is available."""
from __future__ import annotations

import random
import shutil
import subprocess
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
                    if op.kind in pf.BIT_KINDS:
                        width, pos, _ = pf.BIT_FIELDS[op.ty]
                        self.assertLessEqual(pos + width, 32, (seed, op))  # one 32-bit unit
                        continue
                    if op.kind == "pcopy":
                        self.assertLessEqual(pf.PAD_SIZE, pf.OBJECT_SIZE, (seed, op))
                        continue
                    if op.kind in pf.PAD_KINDS:
                        off, ty = pf.PAD_FIELDS[op.ty]
                        self.assertLessEqual(off + pf.TYPES[ty][0], pf.PAD_SIZE, (seed, op))
                        continue
                    size = pf.TYPES[op.ty][0]
                    self.assertLessEqual(op.off + size, pf.OBJECT_SIZE, (seed, op))
                    if op.kind in ("uset", "uget"):
                        self.assertEqual(op.off % size, 0, "union arrays are naturally aligned")

    def test_model_output_format(self):
        out = pf.run_model(pf.gen_case(5))
        self.assertRegex(out, r"^[0-9a-f]{16} [0-9a-f]{16}\n$")

    def test_bitfield_store_load_matches_packed_layout(self):
        mem = bytearray(pf.OBJECT_SIZE)
        pf.bf_set(mem, 3, 7, 0x7F)          # field b (signed, bits 3..9) = all ones
        self.assertEqual(pf.bf_get(mem, "b"), -1)
        self.assertEqual(pf.bf_get(mem, "a"), 0)
        self.assertEqual(pf.bf_get(mem, "c"), 0)
        pf.bf_set(mem, 0, 3, 5)             # neighbour store keeps b intact
        self.assertEqual((pf.bf_get(mem, "a"), pf.bf_get(mem, "b")), (5, -1))

    def test_bitfield_store_truncates_to_width(self):
        mem = bytearray(pf.OBJECT_SIZE)
        pf.bf_set(mem, 10, 14, (1 << 14) + 7)   # field c: modular, keeps low 14 bits
        self.assertEqual(pf.bf_get(mem, "c"), 7)
        pf.bf_set(mem, 3, 7, 0x80)              # 0x80 truncated to 7 bits is 0 -> b == 0
        self.assertEqual(pf.bf_get(mem, "b"), 0)
        self.assertEqual(pf.bf_get(mem, "c"), 7)
        pf.bf_set(mem, 3, 7, 0xFF)              # 0xFF truncated to 7 bits is 0x7F -> -1
        self.assertEqual(pf.bf_get(mem, "b"), -1)

    def test_padding_bytes_are_never_read(self):
        padding = set(range(pf.PAD_SIZE)) - {
            b for off, ty in pf.PAD_FIELDS.values() for b in range(off, off + pf.TYPES[ty][0])}
        self.assertEqual(padding, {1, 2, 3, 11})
        for seed in range(300):
            for ops in pf.gen_case(seed).bodies:
                for op in ops:
                    if op.kind in pf.PAD_KINDS and op.kind != "pcopy":
                        off, ty = pf.PAD_FIELDS[op.ty]
                        self.assertEqual(op.off, off)
                        self.assertFalse(set(range(off, off + pf.TYPES[ty][0])) & padding)


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

    def test_padded_struct_layout_matches_gcc(self):
        # C cannot take the offset of a bit-field, so bit positions are covered by
        # the corpus itself; the byte layout of struct pad is checked directly.
        probe = pf.UNION_DEF.split("union cell")[0] + (
            "#include <stddef.h>\n#include <stdio.h>\n"
            "int main(void) {\n"
            "  printf(\"%zu %zu %zu %zu %zu %zu\\n\", sizeof(struct pad),"
            " offsetof(struct pad, a), offsetof(struct pad, b),"
            " offsetof(struct pad, c), offsetof(struct pad, d), sizeof(struct bits));\n"
            "  return 0;\n}\n")
        with tempfile.TemporaryDirectory() as tmp:
            src, exe = Path(tmp) / "probe.c", Path(tmp) / "probe"
            src.write_text("#include <stdint.h>\n" + probe)
            subprocess.run(["gcc", "-w", str(src), "-o", str(exe)], check=True, timeout=60)
            got = subprocess.run([str(exe)], capture_output=True, text=True, check=True).stdout.split()
        want = [pf.PAD_SIZE] + [pf.PAD_FIELDS[k][0] for k in "abcd"] + [4]
        self.assertEqual([int(x) for x in got], want)

    def test_v3_lane_matches_gcc(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            for seed in range(3):
                result = pf.one_case(pf.Job(seed, "gcc", "O2", "v3", out, 60))
                self.assertEqual(result.status, "pass",
                                 f"seed {seed} v3: {result.detail}")


if __name__ == "__main__":
    unittest.main()
