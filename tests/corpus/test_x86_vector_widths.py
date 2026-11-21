"""Hold scripts/x86_vector_widths.py to account, without an assembler.

The tool writes the table `operand_class::check` refuses statements by, so
what it reads off an encoding and off a statement has to be what the encoder
reads off them: these tests pin the opcode key of every prefix form, the
combination of every statement shape, and that the table in the tree is one
the tool could have written -- in order, complete, every index in range.

(That the table is what GNU as says is `--check`, which needs GNU as 2.47
and runs beside the table sweep.)
"""
from __future__ import annotations

import re
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from scripts import gas_table_sweep as sweep  # noqa: E402
from scripts import x86_vector_widths as widths  # noqa: E402


def key(space: int, vector_map: int, opcode: int, prefix: int) -> int:
    return space << 24 | vector_map << 16 | opcode << 8 | prefix


class Encodings(unittest.TestCase):
    def test_the_opcode_is_read_off_every_prefix_form(self):
        cases = {
            # vaddps %xmm1, %xmm2, %xmm3: two-byte VEX, map 1, no prefix.
            "c5e858d9": key(0, 1, 0x58, 0),
            # vaddpd: `66` is pp 1; vaddss F3 is 2; vaddsd F2 is 3.
            "c5e958d9": key(0, 1, 0x58, 1),
            "c5ea58d9": key(0, 1, 0x58, 2),
            "c5eb58d9": key(0, 1, 0x58, 3),
            # vpermd %ymm1, %ymm2, %ymm3: three-byte VEX, map 2.
            "c4e26d36d9": key(0, 2, 0x36, 1),
            # vpermq $5, %ymm1, %ymm2: map 3.
            "c4e3fd00d105": key(0, 3, 0x00, 1),
            # vaddps %zmm1, %zmm2, %zmm3: EVEX is the same space as VEX.
            "62f16c4858d9": key(0, 1, 0x58, 0),
            # vaddph: EVEX map 5.
            "62f56c4858d9": key(0, 5, 0x58, 0),
            # vpcmov %xmm1, %xmm2, %xmm3, %xmm4: XOP map 8.
            "8fe860a2e210": key(1, 8, 0xA2, 0),
            # A segment override and an address-size prefix in front.
            "6467c5f8280a": key(0, 1, 0x28, 0),
        }
        for code, expected in cases.items():
            self.assertEqual(widths.opcode_key(bytes.fromhex(code)), expected, code)
        # Not a vector encoding: `addps`, `pop (%rax)` (8F /0), too short.
        for code in ("0f58c1", "8f00", "c5f8", ""):
            self.assertIsNone(widths.opcode_key(bytes.fromhex(code)), code)

    def test_the_combination_is_read_off_the_statement(self):
        cases = {
            "vaddps %xmm1, %xmm2, %xmm3": 0b01_01_01,
            "vpsllw %xmm1, %ymm2, %ymm2": 0b01_10_10,
            "vcvtps2pd %ymm1, %zmm2": 0b10_11,
            "vmovaps %zmm3, (%rdx)": 0b11,
            # A register in an address is the address's.
            "vgatherdps %ymm1, (%rdx,%ymm7,2), %ymm2": 0b10_10,
            # Neither are a mask, a general register or an immediate.
            "vpcmpeqd %zmm1, %zmm2, %k3": 0b11_11,
            "vpinsrd $5, %ecx, %xmm2, %xmm3": 0b01_01,
            "vzeroupper": 0,
            "vcvtsi2sd %eax, %xmm1, %xmm2": 0b01_01,
            # A pseudo-prefix is no operand.
            "{evex} {store} vmovaps %xmm1, %xmm2": 0b01_01,
            "vpcmov %ymm1, %ymm2, %ymm3, %ymm4": 0b10_10_10_10,
        }
        for statement, expected in cases.items():
            self.assertEqual(widths.combination_of(statement), expected, statement)
        self.assertIsNone(widths.combination_of("v %xmm1, %xmm2, %xmm3, %xmm4, %xmm5"))
        self.assertEqual(widths.spell(0b01_10_10), "xyy")
        self.assertEqual(widths.spell(0b11), "z")

    def test_every_width_is_asked_for_in_every_vector_position(self):
        table = ("vfoo, 0x6658, AVX, Modrm|Vex128|Space0F|Src1VVVV|NoSuf, "
                 "{ RegXMM|Unspecified|BaseIndex, RegXMM, RegXMM }\n"
                 "vbar, 0x28, AVX, D|Modrm|Vex|Space0F|NoSuf, "
                 "{ RegXMM|RegYMM|Unspecified|BaseIndex, RegXMM|RegYMM }\n"
                 "plain, 0x90, 0, NoSuf, { Reg32 }\n")
        made, skipped = widths.statements(list(sweep.expand_rows(table)))
        self.assertEqual(skipped, 0)
        plain = [statement for statement in made if not statement.startswith("{")]
        foo = [statement for statement in plain if statement.startswith("vfoo ")]
        # Three widths and an address in the first position, three widths in
        # each of the others; a register of its own in each position.
        self.assertEqual(len(foo), 4 * 3 * 3)
        self.assertIn("vfoo %zmm1, %ymm2, %xmm3", foo)
        self.assertIn("vfoo (%rdx), %xmm2, %xmm3", foo)
        # A row with a direction bit is asked about in both directions.
        self.assertIn("vbar %xmm1, %ymm2", plain)
        self.assertIn("vbar %ymm1, (%rdx)", plain)
        self.assertFalse(any(statement.startswith("plain") for statement in made))
        # Every encoding a pseudo-prefix can ask for.
        self.assertEqual(len(widths.ENCODINGS), 9)
        for prefix in ("{evex} ", "{vex} {store} ", "{load} "):
            self.assertIn(prefix + "vfoo %xmm1, %xmm2, %xmm3", made)
        self.assertEqual(len(made), len(plain) * len(widths.ENCODINGS))


class Table(unittest.TestCase):
    def setUp(self):
        self.source = widths.TABLE.read_text()
        self.opcodes = [(int(k.replace("_", ""), 16), int(shape)) for k, shape in
                        re.findall(r"\(0x([0-9A-F_]+), (\d+)\)", self.source)]
        self.shapes = [[int(code) for code in body.split(",") if code.strip()]
                       for body in re.findall(r"^    &\[([0-9, ]*)\],", self.source, re.M)]

    def test_the_table_in_the_tree_is_one_the_tool_writes(self):
        declared = re.search(r"OPCODES: \[\(u32, u8\); (\d+)\]", self.source)
        shapes = re.search(r"SHAPES: \[&\[u8\]; (\d+)\]", self.source)
        self.assertEqual(int(declared.group(1)), len(self.opcodes))
        self.assertEqual(int(shapes.group(1)), len(self.shapes))
        self.assertGreater(len(self.opcodes), 700)
        # In order, for the encoder's binary search; every index in range.
        keys = [k for k, _ in self.opcodes]
        self.assertEqual(keys, sorted(set(keys)))
        self.assertTrue(all(shape < len(self.shapes) for _, shape in self.opcodes))
        self.assertEqual({shape for _, shape in self.opcodes}, set(range(len(self.shapes))))
        for shape in self.shapes:
            self.assertEqual(shape, sorted(set(shape)))
            self.assertTrue(shape)
            for code in shape:
                self.assertRegex(widths.spell(code), r"^[xyz]{1,4}$")
        # Rendering what was read gives the file back.
        found = {k: {code: "" for code in self.shapes[shape]} for k, shape in self.opcodes}
        self.assertEqual(widths.render(found), self.source)

    def shape_of(self, space, vector_map, opcode, prefix):
        index = dict(self.opcodes)[key(space, vector_map, opcode, prefix)]
        return {widths.spell(code) for code in self.shapes[index]}

    def test_the_table_says_what_the_instructions_take(self):
        # vaddps: one width at a time, in any of the three.
        self.assertEqual(self.shape_of(0, 1, 0x58, 0),
                         {"xx", "yy", "zz", "xxx", "yyy", "zzz"})
        # vaddsd: `%xmm` only.
        self.assertEqual(self.shape_of(0, 1, 0x58, 3), {"xx", "xxx"})
        # vpermd: never `%xmm`.
        self.assertEqual(self.shape_of(0, 2, 0x36, 1), {"yy", "zz", "yyy", "zzz"})
        # vpsllw: an `%xmm` count for any width.
        self.assertEqual(self.shape_of(0, 1, 0xF1, 1),
                         {"xx", "yy", "zz", "xxx", "xyy", "xzz"})
        # vpmovsxbw widens by one step; vpmovwb narrows by one.
        self.assertEqual(self.shape_of(0, 2, 0x20, 1), {"x", "y", "z", "xx", "xy", "yz"})
        self.assertEqual(self.shape_of(0, 2, 0x30, 2), {"x", "y", "z", "xx", "yx", "zy"})
        # vmovaps to memory names one register, between registers two.
        self.assertEqual(self.shape_of(0, 1, 0x29, 0), {"x", "y", "z", "xx", "yy", "zz"})
        # vpcmov (XOP): `%xmm` or `%ymm`, three or four of them.
        self.assertEqual(self.shape_of(1, 8, 0xA2, 0), {"xxx", "yyy", "xxxx", "yyyy"})


if __name__ == "__main__":
    unittest.main()
