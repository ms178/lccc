#!/usr/bin/env python3
"""Which widths of vector register each VEX, EVEX and XOP opcode takes together.

`vaddps %ymm1, %xmm2, %xmm3`, `vpermd %xmm1, %xmm1, %xmm1` and `vpmovsxbw
%ymm1, %ymm2` name registers the instruction exists for, at widths it does not
exist in. An encoder that takes the vector length from one operand and the
register numbers from all of them assembles each into some other instruction;
the encoders here did, for 1,203 of the 31,538 such statements GNU as refuses.

What an instruction takes is a fact about its opcode, and GNU as can be asked
for it. This tool puts `%xmm1`, `%ymm2`, `%zmm3` and -- where the row allows
one -- a memory operand in every vector position of every row of GNU as's
instruction table, in every combination, keeps the statements GNU as
assembles, and reads off the bytes it wrote the opcode (map, opcode byte,
mandatory prefix) and off the statement the widths of its vector registers,
in order. The result is

    src/backend/x86/assembler/encoder/vector_widths.rs

a table from opcode to the combinations of widths it takes, which
`operand_class::check` holds every statement against. An opcode the table has
no entry for is not judged.

    scripts/x86_vector_widths.py --as AS --check   # the table is what GNU as says
    scripts/x86_vector_widths.py --as AS --write   # regenerate it
    scripts/x86_vector_widths.py --as AS --report  # the shapes, with examples

Nothing of GNU as's table is copied: it is read (fetched at the pinned commit
and SHA-256 `gas_table_sweep.py` uses), turned into statements, and what is
recorded is how the assembler answers them.
"""
from __future__ import annotations

import argparse
import itertools
import re
import sys
import tempfile
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO))

from scripts import gas_table_sweep as sweep  # noqa: E402

TABLE = REPO / "src/backend/x86/assembler/encoder/vector_widths.rs"
VECTOR_CLASSES = ("RegXMM", "RegYMM", "RegZMM")
# A width is a base-4 digit; a combination is the digits of its registers,
# the first operand's first. No digit is 0, so no two combinations collide.
DIGIT = {"x": 1, "y": 2, "z": 3}
MAX_REGISTERS = 4
# The opcode spaces of the table's key.
VEX, XOP = 0, 1
# Every encoding a statement can ask for by a pseudo-prefix, each of which
# can have an opcode of its own: the EVEX form of an instruction that has a
# VEX form too (`vmpsadbw` has another mandatory prefix there), and the
# load and the store form of a move (which `vmovaps.s` asks for as well).
ENCODINGS = tuple(f"{form}{direction}"
                  for form in ("", "{evex} ", "{vex} ")
                  for direction in ("", "{load} ", "{store} "))


def opcode_key(code: bytes) -> int | None:
    """(space, map, opcode, mandatory prefix) of a VEX, EVEX or XOP encoding
    as one number, in 64-bit code; None for anything else."""
    at = 0
    while at < len(code) and code[at] in (0x26, 0x2E, 0x36, 0x3E, 0x64, 0x65, 0x67):
        at += 1
    if at + 4 > len(code):
        return None
    first, second = code[at], code[at + 1]
    if first == 0x62 and at + 5 <= len(code):
        space, vector_map, prefix, opcode = VEX, second & 0x07, code[at + 2] & 3, code[at + 4]
    elif first == 0xC4:
        space, vector_map, prefix, opcode = VEX, second & 0x1F, code[at + 2] & 3, code[at + 3]
    elif first == 0xC5:
        space, vector_map, prefix, opcode = VEX, 1, second & 3, code[at + 2]
    elif first == 0x8F and second & 0x1F >= 8:
        space, vector_map, prefix, opcode = XOP, second & 0x1F, code[at + 2] & 3, code[at + 3]
    else:
        return None
    return space << 24 | vector_map << 16 | opcode << 8 | prefix


REGISTER = re.compile(r"%([xyz])mm[0-9]+")


def operands_of(statement: str) -> list[str]:
    """The operands of an AT&T statement: split on commas outside brackets."""
    while statement.startswith("{"):
        statement = statement.partition("} ")[2]
    _, _, rest = statement.partition(" ")
    out, depth, start = [], 0, 0
    for i, ch in enumerate(rest):
        if ch in "({":
            depth += 1
        elif ch in ")}":
            depth -= 1
        elif ch == "," and depth == 0:
            out.append(rest[start:i].strip())
            start = i + 1
    out.append(rest[start:].strip())
    return [operand for operand in out if operand]


def combination_of(statement: str) -> int | None:
    """The widths of the statement's vector register operands as one number;
    0 for none, None for more than the table has room for. A register in an
    address is the address's."""
    code, count = 0, 0
    for operand in operands_of(statement):
        match = REGISTER.fullmatch(operand)
        if match:
            code = code * 4 + DIGIT[match.group(1)]
            count += 1
    return code if count <= MAX_REGISTERS else None


def statements(rows) -> tuple[list[str], int]:
    """Every combination of vector registers and memory in the vector
    positions of every row, and the number of rows that have an operand no
    choice exists for (those rows are not asked about)."""
    mode = sweep.MODES["64"]
    out, skipped = set(), 0
    for mnemonic, _opcode, _cpu, attributes, row_operands in sweep.rows_of(rows, mode, set()):
        if not any(classes & set(VECTOR_CLASSES) for classes in row_operands):
            continue
        # A move is a row in one direction and a statement in both.
        for order, operands in enumerate(sweep.orders_of(attributes, row_operands)):
            per_operand = []
            for position, classes in enumerate(operands, 1):
                choices = sweep.operand_choices(classes, mode, attributes)
                if classes & set(VECTOR_CLASSES):
                    # Every width, whatever the row says it takes: GNU as is
                    # asked, not the row. A register of its own in each
                    # position (`vfmaddcph` wants its destination apart from
                    # its sources), and an address where the row has one.
                    memory = [choice for choice in choices if "(" in choice][:1]
                    per_operand.append([f"%{width}mm{position}" for width in "xyz"] + memory)
                elif choices:
                    per_operand.append(choices[:1])
                else:
                    per_operand = None
                    break
            if per_operand is None:
                skipped += not order
                continue
            for combo in itertools.product(*per_operand):
                statement = f"{mnemonic} {', '.join(combo)}"
                out.update(prefix + statement for prefix in ENCODINGS)
    return sorted(out), skipped


def survey(rows, gas: str):
    """{opcode key: {combination: an example statement}} of what GNU as takes."""
    made, skipped = statements(rows)
    with tempfile.TemporaryDirectory() as directory:
        assembled = sweep.assemble_gas(made, gas, sweep.MODES["64"], Path(directory))
    found: dict[int, dict[int, str]] = defaultdict(dict)
    for index, (code, _relocations) in sorted(assembled.items()):
        key, combination = opcode_key(code), combination_of(made[index])
        if key is None or not combination:
            continue
        found[key].setdefault(combination, made[index])
    return found, len(made), len(assembled), skipped


def spell(combination: int) -> str:
    letters = ""
    while combination:
        letters = "?xyz"[combination % 4] + letters
        combination //= 4
    return letters


def render(found: dict[int, dict[int, str]]) -> str:
    """The Rust source of the table."""
    shapes: dict[tuple[int, ...], int] = {}
    keys = []
    for key in sorted(found):
        shape = tuple(sorted(found[key]))
        keys.append((key, shapes.setdefault(shape, len(shapes))))
    lines = [
        "//! Which widths of vector register each VEX, EVEX and XOP opcode takes",
        "//! together: what GNU as 2.47 assembles, asked for every combination.",
        "//!",
        "//! Written by `scripts/x86_vector_widths.py --write`; `--check` holds it to",
        "//! GNU as. Not edited by hand.",
        "//!",
        "//! A width is a base-4 digit -- 1 for `%xmm`, 2 for `%ymm`, 3 for `%zmm` -- and",
        "//! a combination is the digits of a statement's vector register operands, the",
        "//! first operand's first: `vpsllw %xmm1, %ymm2, %ymm2` is 1 2 2, or 26.",
        "",
        "/// The opcodes, in order: `space << 24 | map << 16 | opcode << 8 | prefix`",
        "/// (space 0 is VEX and EVEX, 1 is XOP; the prefix is the encoding's `pp`),",
        "/// each with the index of its shape in [`SHAPES`].",
        "#[rustfmt::skip]",
        f"pub(super) static OPCODES: [(u32, u8); {len(keys)}] = [",
    ]
    for start in range(0, len(keys), 6):
        lines.append("    " + " ".join(f"(0x{key >> 16:04X}_{key & 0xFFFF:04X}, {shape}),"
                                       for key, shape in keys[start:start + 6]))
    lines += [
        "];",
        "",
        "/// The combinations of widths an opcode takes, in order.",
        "#[rustfmt::skip]",
        f"pub(super) static SHAPES: [&[u8]; {len(shapes)}] = [",
    ]
    for shape, index in sorted(shapes.items(), key=lambda item: item[1]):
        spelled = " ".join(spell(c) for c in shape)
        lines.append(f"    &[{', '.join(str(c) for c in shape)}], // {index}: {spelled}")
    lines += ["];", ""]
    if len(shapes) > 256:
        sys.exit("x86_vector_widths: more than 256 shapes; the index no longer fits a byte")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--as", dest="gas", default="as", help="GNU as 2.47")
    parser.add_argument("--tbl", help="opcodes/i386-opc.tbl of binutils 2.47 (default: fetch)")
    what = parser.add_mutually_exclusive_group(required=True)
    what.add_argument("--check", action="store_true",
                      help="fail unless the table in the tree is what GNU as says")
    what.add_argument("--write", action="store_true", help="write the table")
    what.add_argument("--report", action="store_true", help="print the shapes, with examples")
    args = parser.parse_args(argv)
    version = sweep.gas_version(args.gas)
    if not sweep.GAS_VERSION.fullmatch(version):
        sys.exit(f"x86_vector_widths: {args.gas} is GNU as `{version}`, not 2.47: the table "
                 "records what that release assembles")
    rows = list(sweep.expand_rows(sweep.table_text(args.tbl)))
    found, made, assembled, skipped = survey(rows, args.gas)
    summary = (f"{made} statements, {assembled} assembled by GNU as, {len(found)} opcodes, "
               f"{len({tuple(sorted(shape)) for shape in found.values()})} shapes")
    if skipped:
        # A row that is not asked about leaves its combinations out of the
        # table, and a statement of it would be refused for that.
        sys.exit(f"x86_vector_widths: {skipped} row(s) with a vector operand have an operand "
                 "no choice exists for (gas_table_sweep.operand_choices)")
    if not found:
        sys.exit("x86_vector_widths: GNU as assembled no statement with a vector register")
    text = render(found)
    if args.report:
        by_shape = defaultdict(list)
        for key, shape in found.items():
            by_shape[tuple(sorted(shape))].append(key)
        for shape, keys in sorted(by_shape.items(), key=lambda item: -len(item[1])):
            example = found[keys[0]][shape[-1]]
            print(f"{len(keys):5}  {' '.join(spell(c) for c in shape):48} {example}")
        print(summary)
        return 0
    if args.write:
        TABLE.write_text(text)
        print(f"wrote {TABLE.relative_to(REPO)}: {summary}")
        return 0
    try:
        current = TABLE.read_text()
    except FileNotFoundError:
        current = ""
    if current != text:
        print(f"x86_vector_widths: {TABLE.relative_to(REPO)} is not what GNU as "
              f"{version} says ({summary}); --write regenerates it")
        return 1
    print(f"x86_vector_widths: ok ({summary})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
