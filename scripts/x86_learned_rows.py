#!/usr/bin/env python3
"""The instructions no encoder arm knows, learned from GNU as.

The x86 assemblers encode an instruction in the arm that was written for it
(`src/backend/x86/assembler/encoder`), and an instruction nobody wrote an arm
for is "unhandled" however regular its encoding is. GNU as's opcode table has
some 4,800 rows; `scripts/gas_table_sweep.py` asks statements of all of them
and counted hundreds of mnemonics the arms refuse: the AMD extensions (3DNow!,
FMA4, XOP, TBM, LWP), Key Locker, AMX, AVX10.2, SM3, SHA512, the MMX/SSE
conversions, and odd forms of known ones.

This script closes that list with a table instead of an arm per instruction.
For every spelling of a mnemonic on its list it asks GNU as what it assembles
-- each operand a general, MMX, vector, mask or tile register of one width,
memory, or a number -- and from the bytes of two statements that differ in
their registers it works out where each operand goes: the prefix, the map and
the opcode, the `ModRM` fields, `vvvv`, the upper half of a trailing byte.
Further statements say what else the form takes: a mask and zeroing, a
broadcast and its width, a rounding mode, a register of APX, how a one-byte
displacement is scaled, how large a number may be. Nothing is copied from the
table's own columns: its rows only say which statements are worth asking.

  --write   solve every mnemonic of the list and write
            src/backend/x86/assembler/encoder/learned_rows.rs
  --check   solve again and fail unless that file is what comes out
  --verify  assemble with lccc statements of every row that vary what the
            solving held still -- every register of a kind, the addresses,
            the masks, the broadcasts, the rounding modes, the numbers, the
            encoding hints -- and fail unless each is what GNU as makes of it
            (the same bytes and relocations, or refused by both)
  --add M   with --write: put the mnemonic M (as spelled, suffix and all) on
            the list

The list is the header of the written file.
"""

from __future__ import annotations

import argparse
import itertools
import os
import re
import sys
import tempfile
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gas_table_sweep as sweep  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
OUT = REPO / "src" / "backend" / "x86" / "assembler" / "encoder" / "learned_rows.rs"

MODES = ("64", "32", "16")

# ─── Operand kinds ───────────────────────────────────────────────────────────

# A register of a class the table names, by number.
GENERAL = {
    "r8": ("al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil"),
    "r16": ("ax", "cx", "dx", "bx", "sp", "bp", "si", "di"),
    "r32": ("eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi"),
    "r64": ("rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi"),
}
HIGH_SUFFIX = {"r8": "b", "r16": "w", "r32": "d", "r64": ""}
NUMBERED = {"mm": "mm", "x": "xmm", "y": "ymm", "z": "zmm", "k": "k", "t": "tmm"}
CLASS_KIND = (("Reg8", "r8"), ("Reg16", "r16"), ("Reg32", "r32"), ("Reg64", "r64"),
              ("RegMMX", "mm"), ("RegXMM", "x"), ("RegYMM", "y"), ("RegZMM", "z"),
              ("RegMask", "k"), ("RegTMM", "t"))
# Kinds that are one register, which the opcode implies.
FIXED = ("al", "ax", "eax", "rax", "cl", "cx", "ecx", "rcx", "dx", "edx", "rdx",
         "ebx", "rbx", "xmm0")
# The order the kinds are numbered in, here and in `learned.rs`.
KINDS = ("r8", "r16", "r32", "r64", "mm", "x", "y", "z", "k", "t", "m", "i8") + FIXED
SLOTTED = set(GENERAL) | set(NUMBERED)
SIZE_WORDS = {"Byte", "Word", "Dword", "DWord", "Qword", "Fword", "Tbyte", "Oword",
              "Xmmword", "XMMword", "Ymmword", "YMMword", "Zmmword", "ZMMword",
              "Unspecified"}
ACC = {"Byte": "al", "Word": "ax", "Dword": "eax", "Qword": "rax"}
NAMED = {"RegC": {"Byte": "cl", "Word": "cx", "Dword": "ecx", "Qword": "rcx"},
         "RegD": {"Word": "dx", "Dword": "edx", "Qword": "rdx"},
         "RegB": {"Dword": "ebx", "Qword": "rbx"}}


def kinds_of(classes: set[str], attributes: set[str], long: bool) -> list[str] | None:
    """The kinds an operand of the table's `classes` can be; None for a class
    the rows here do not say (a branch target, a segment register, an address
    with a vector for an index, a number wider than a byte)."""
    rest = set(classes) - SIZE_WORDS
    out: list[str] = []
    if "Acc" in rest:
        rest.discard("Acc")
        if classes & {"Xmmword", "XMMword"} and "RegXMM" not in classes:
            out.append("xmm0")
        else:
            out += [name for size, name in ACC.items()
                    if size in classes and (long or size != "Qword")]
            if not out:
                return None
    for name, by_size in NAMED.items():
        if name in rest:
            rest.discard(name)
            out += [reg for size, reg in by_size.items()
                    if size in classes and (long or size != "Qword")]
            if not classes & set(by_size):
                out.append(by_size["Dword"])
    if "ShiftCount" in rest:
        rest.discard("ShiftCount")
        out.append("cl")
    for name, kind in CLASS_KIND:
        if name in rest:
            rest.discard(name)
            if kind != "r64" or long:
                out.append(kind)
    if "BaseIndex" in rest:
        rest.discard("BaseIndex")
        if any(a.startswith("VecSIB") for a in attributes):
            return None
        out.append("m")
    if rest & {"Imm8", "Imm8S"}:
        rest -= {"Imm8", "Imm8S"}
        out.append("i8")
    if rest or not out:
        return None
    return list(dict.fromkeys(out))


def register(kind: str, number: int) -> str:
    if kind in GENERAL:
        if number < 8:
            return "%" + GENERAL[kind][number]
        return f"%r{number}{HIGH_SUFFIX[kind]}"
    return f"%{NUMBERED[kind]}{number}"


# The numbers of the registers of two statements that are asked of a form:
# by position among the operands that have a field, different in each
# position and different between the two.
PROBE_NUMBERS = ((1, 2, 3, 6, 7), (3, 1, 2, 7, 6))


def probe_number(kind: str, position: int, which: int) -> int:
    number = PROBE_NUMBERS[which][position]
    # `%sil` needs a REX prefix and 32-bit code has none.
    return number - 4 if kind == "r8" and number >= 4 else number


def base_register(mode: str) -> str:
    return "%rbx" if mode == "64" else "%ebx"


def statement_of(name: str, kinds: tuple[str, ...], mode: str, which: int = 0, *,
                 hint: str = "", memory: str | None = None, last: str = "",
                 rounding: tuple[int, str] | None = None, numbers: dict | None = None,
                 immediate: str | None = None) -> str:
    """The statement of `name` with operands of `kinds`: the registers of
    probe `which` (or `numbers`, by operand index), `memory` for the address,
    `last` behind the last operand, `rounding` (a position and a word) put in
    front of an operand."""
    operands = []
    position = 0
    for index, kind in enumerate(kinds):
        if kind in SLOTTED:
            number = (numbers or {}).get(index, probe_number(kind, position, which))
            operands.append(register(kind, number))
            position += 1
        elif kind == "m":
            operands.append(memory or f"({base_register(mode)})")
        elif kind == "i8":
            operands.append(immediate or ("$5", "$9")[which])
        else:
            operands.append("%" + kind)
    if last and operands:
        operands[-1] += last
    if rounding:
        operands.insert(rounding[0], rounding[1])
    return (hint + name + " " + ", ".join(operands)).strip()


# ─── Reading an encoding ─────────────────────────────────────────────────────

@dataclass
class Encoding:
    space: str = "L"            # L(egacy), V(EX), E(VEX), X(OP)
    prefixes: bytes = b""       # the 66, F2, F3 in front (legacy space)
    rex: int = 0                # a REX prefix, or 0
    rex2: int = -1              # the payload of a REX2 prefix, or -1
    map: int = 0                # legacy: 0, 1 (0F), 2 (0F 38), 3 (0F 3A), 4 (0F 0F)
    pp: int = 0
    w: int = 0
    l: int = 0
    opcode: int = 0
    modrm: int = -1
    sib: int = -1
    disp: bytes = b""
    rest: bytes = b""           # behind the ModRM byte and its address
    reg: int = 0                # ModRM.reg with its extension bits
    rm: int = 0                 # ModRM.rm (or the base) with its extension bits
    index: int = -1
    vvvv: int = 0
    z: int = 0
    b: int = 0
    aaa: int = 0
    nd_nf: int = 0              # EVEX P2 bits 4 and 2 of an APX (map 4) form

    @property
    def mod(self) -> int:
        return self.modrm >> 6

    def key(self) -> tuple:
        """What a row is: everything but the operands' own bits."""
        return (self.space, self.prefixes, self.map, self.pp, self.w, self.l, self.opcode)


def read(code: bytes, long: bool, modrm: bool) -> Encoding | None:
    """The fields of `code`, an instruction with a ModRM byte or (`modrm`
    false) without one. None when it is not laid out as expected."""
    enc = Encoding()
    at = 0
    try:
        while code[at] in (0x66, 0xF2, 0xF3):
            at += 1
        enc.prefixes = bytes(code[:at])
        first = code[at]
        # An address-size or a segment prefix: an operand chose it, and the
        # rows have no word for that.
        if first in (0x67, 0x26, 0x2E, 0x36, 0x3E, 0x64, 0x65, 0xF0):
            return None
        vector = long or (at + 1 < len(code) and code[at + 1] >= 0xC0)
        if long and 0x40 <= first <= 0x4F:
            enc.rex = first
            at += 1
            first = code[at]
        r = x = b = r4 = x4 = b4 = v4 = 0
        if first == 0xD5 and long:
            payload = code[at + 1]
            enc.rex2 = payload
            at += 2
            enc.map = 1 if payload & 0x80 else 0
            enc.w = (payload >> 3) & 1
            r, x, b = (payload >> 2) & 1, (payload >> 1) & 1, payload & 1
            r4, x4, b4 = (payload >> 6) & 1, (payload >> 5) & 1, (payload >> 4) & 1
            if enc.map == 1 and code[at] == 0x0F:
                # 3DNow!: `0F 0F`, of which REX2 says the first.
                enc.map = 4
            else:
                enc.opcode = code[at]
            at += 1
        elif first == 0x62 and vector:
            p0, p1, p2 = code[at + 1], code[at + 2], code[at + 3]
            enc.space = "E"
            enc.map = p0 & 7
            r, x, b = (~p0 >> 7) & 1, (~p0 >> 6) & 1, (~p0 >> 5) & 1
            r4, b4 = (~p0 >> 4) & 1, (p0 >> 3) & 1
            enc.w = p1 >> 7
            enc.vvvv = (~p1 >> 3) & 15
            x4 = (~p1 >> 2) & 1
            enc.pp = p1 & 3
            enc.z, enc.l, enc.b = p2 >> 7, (p2 >> 5) & 3, (p2 >> 4) & 1
            v4 = (~p2 >> 3) & 1
            enc.aaa = p2 & 7
            enc.nd_nf = p2 & 0x14
            enc.opcode = code[at + 4]
            at += 5
        elif first in (0xC4, 0x8F) and (vector if first == 0xC4 else code[at + 1] & 0x1F >= 8):
            p0, p1 = code[at + 1], code[at + 2]
            enc.space = "V" if first == 0xC4 else "X"
            enc.map = p0 & 0x1F
            r, x, b = (~p0 >> 7) & 1, (~p0 >> 6) & 1, (~p0 >> 5) & 1
            enc.w = p1 >> 7
            enc.vvvv = (~p1 >> 3) & 15
            enc.l, enc.pp = (p1 >> 2) & 1, p1 & 3
            enc.opcode = code[at + 3]
            at += 4
        elif first == 0xC5 and vector:
            p0 = code[at + 1]
            enc.space, enc.map = "V", 1
            r = (~p0 >> 7) & 1
            enc.vvvv = (~p0 >> 3) & 15
            enc.l, enc.pp = (p0 >> 2) & 1, p0 & 3
            enc.opcode = code[at + 2]
            at += 3
        else:
            enc.w = (enc.rex >> 3) & 1
            r, x, b = (enc.rex >> 2) & 1, (enc.rex >> 1) & 1, enc.rex & 1
            if first == 0x0F:
                second = code[at + 1]
                if second in (0x38, 0x3A):
                    enc.map = 2 if second == 0x38 else 3
                    enc.opcode = code[at + 2]
                    at += 3
                elif second == 0x0F:
                    # 3DNow!: the opcode stands behind the operands.
                    enc.map = 4
                    at += 2
                else:
                    enc.map, enc.opcode = 1, second
                    at += 2
            else:
                enc.opcode = first
                at += 1
        if enc.space != "L":
            if enc.prefixes or enc.rex:
                return None
        enc.vvvv |= v4 << 4
        if not modrm:
            enc.rest = bytes(code[at:])
            return enc
        enc.modrm = code[at]
        at += 1
        enc.reg = ((enc.modrm >> 3) & 7) | r << 3 | r4 << 4
        low = enc.modrm & 7
        if enc.mod == 3:
            # A vector register's fifth bit is EVEX.X; a general register's
            # is the B4 of APX.
            enc.rm = low | b << 3 | (x if enc.space == "E" else 0) << 4 | b4 << 4
        else:
            base = low
            if low == 4:
                enc.sib = code[at]
                at += 1
                base = enc.sib & 7
                enc.index = ((enc.sib >> 3) & 7) | x << 3 | x4 << 4
            width = {0: 0, 1: 1, 2: 4}[enc.mod]
            if enc.mod == 0 and base == 5:
                width = 4
            enc.disp = bytes(code[at: at + width])
            at += width
            enc.rm = base | b << 3 | b4 << 4
        enc.rest = bytes(code[at:])
        if enc.map == 4 and enc.space == "L":
            if not enc.rest:
                return None
            enc.opcode, enc.rest = enc.rest[-1], enc.rest[:-1]
        return enc
    except (IndexError, KeyError):
        return None


# ─── Rows ────────────────────────────────────────────────────────────────────

@dataclass
class Row:
    name: str
    kinds: tuple[str, ...]
    slots: tuple[str, ...]      # per operand: reg, rm, vvvv, is4, imm, low4, none
    space: str
    prefixes: bytes
    map: int
    pp: int
    w: int
    l: int
    opcode: int
    modrm: bool                 # a ModRM byte follows the opcode
    digit: int = -1             # ModRM.reg, where no operand has it
    fixed_rm: int = -1          # ModRM.rm (register form), where no operand has it
    literal: bytes = b""        # behind the opcode of a form without ModRM
    tail: int = -1              # a byte behind the operands that is no operand
    sized: bool = False         # 16-bit code inverts the operand-size prefix
    data16: bool = False        # the leading 66 is the operand-size prefix
    force_sib: bool = False
    disp8: int = 0              # log2 of what a one-byte displacement counts
    mask: bool = False
    zero: bool = False
    broadcast: int = 0          # N of {1toN}
    broadcast_disp8: int = 0
    rounding: int = 0           # 0 none, 1 {sae}, 2 the four rounding modes
    rounding_at: int = 0        # the operand the word stands in front of
    sae_l: int = 0              # L'L under {sae}
    egpr: bool = False          # takes r16-r31 as it is encoded here
    immediate: int = 0          # 0 none, 1: -128..255, 2: 0..255, 3: -128..127, 4: 0..15
    distinct: int = 0           # bit i*5+j: the operands i < j are different registers
    no_sp: int = 0              # bit i: operand i cannot be the stack pointer
    addressed: bool = False     # the general register is an address, as wide as the other
    dropped: str = ""           # why the row is not written after all
    pick: int = 0               # 0 by default, 1 where EVEX is needed or asked, 2 under {vex}
    modes: set[str] = field(default_factory=set)
    nd_nf: int = 0

    def encoding(self) -> tuple:
        """Everything but the name, the modes and the choice among encodings."""
        return (self.kinds, self.slots, self.space, self.prefixes, self.map, self.pp,
                self.w, self.l, self.opcode, self.modrm, self.digit, self.fixed_rm,
                self.literal, self.tail, self.sized, self.data16, self.force_sib,
                self.disp8, self.mask, self.zero, self.broadcast, self.broadcast_disp8,
                self.rounding, self.rounding_at, self.sae_l, self.immediate, self.nd_nf,
                self.distinct, self.no_sp, self.addressed)


class Asker:
    """Statements for GNU as, asked together."""

    def __init__(self, gas: str, mode: str):
        self.gas, self.mode = gas, sweep.MODES[mode]
        self.asked: dict[str, int] = {}
        self.answers: dict[int, tuple] = {}
        self.done = 0

    def ask(self, statement: str) -> None:
        self.asked.setdefault(statement, len(self.asked))

    def run(self) -> None:
        todo = list(self.asked)[self.done:]
        if not todo:
            return
        with tempfile.TemporaryDirectory(prefix="learned-") as tmp:
            got = sweep.assemble_gas(todo, self.gas, self.mode, Path(tmp))
        for index, answer in got.items():
            self.answers[self.done + index] = answer
        self.done = len(self.asked)

    def code(self, statement: str, memory: bool = False) -> bytes | None:
        """The bytes of `statement`, or None when GNU as refuses it or
        writes a relocation. `memory`: it has an address, which the probes
        write with a 32-bit register; 16-bit code says that with a prefix,
        and the rows are of 32-bit code."""
        answer = self.answers.get(self.asked[statement])
        if answer is None or answer[1]:
            return None
        code = answer[0]
        if memory and self.mode.real and code[:1] == b"\x67":
            code = code[1:]
        return code


def spellings(rows, mode: str, names: set[str]):
    """(name, kinds, has a ModRM byte) of every form the table's rows allow a
    name of `names`: each operand one of its kinds, in each order."""
    long = mode == "64"
    seen = set()
    for mnemonic, _opcode, _cpu, attributes, row_operands in sweep.rows_of(
            rows, sweep.MODES[mode], set()):
        if attributes & {"IsString", "IsPrefix"} or any(a.startswith("Jump") for a in attributes):
            continue
        suffixes = [""]
        if "NoSuf" not in attributes:
            suffixes += [s for s, flag in (("b", "No_bSuf"), ("w", "No_wSuf"), ("l", "No_lSuf"),
                                           ("q", "No_qSuf"), ("s", "No_sSuf"))
                         if flag not in attributes and (long or s != "q" or mnemonic[0] == "f")]
        spelled = [mnemonic + s for s in suffixes if mnemonic + s in names]
        if not spelled:
            continue
        modrm = bool(attributes & {"Modrm", "Sibmem", "RegMem"})
        for operands in sweep.orders_of(attributes, row_operands):
            if oracle_invalid(mnemonic, operands):
                continue
            per_operand = [kinds_of(classes, attributes, long) for classes in operands]
            if None in per_operand:
                continue
            for kinds in itertools.product(*per_operand):
                if not modrm and any(k in SLOTTED or k == "m" for k in kinds):
                    continue
                if sum(k in SLOTTED for k in kinds) > len(PROBE_NUMBERS[0]):
                    continue
                for name in spelled:
                    if (name, kinds) not in seen:
                        seen.add((name, kinds))
                        yield name, kinds, modrm


def oracle_invalid(mnemonic: str, operands: list) -> bool:
    """Forms GNU as 2.47 assembles into something else than was written:
    they get no rows.

    `movrs` only loads. GNU as takes its operands the other way round as
    well (the table row has the direction attribute), and writes the load
    of a doubleword for `movrs %cl, (%rbx)` and, under EVEX, an opcode that
    is another instruction's (`88` in APX's map is `popcnt`)."""
    return mnemonic == "movrs" and bool(operands) and "BaseIndex" in operands[-1]


HINTS = ("", "{evex} ", "{vex} ")
# The 66, F3, F2 in front of a legacy opcode, as GNU as orders them, and the
# number a VEX prefix has for the last of them.
PREFIXES = {b"": 0, b"\x66": 1, b"\xf3": 2, b"\xf2": 3, b"\x66\xf3": 2, b"\x66\xf2": 3}
# The numbers a form is asked with, and the ranges that take them.
IMMEDIATE_CLASSES = {
    (-128, -1, 15, 16, 127, 128, 255): 1,
    (15, 16, 127, 128, 255): 2,
    (-128, -1, 15, 16, 127): 3,
    (15,): 4,
}
ROUNDINGS = ("{rn-sae}", "{rd-sae}", "{ru-sae}", "{rz-sae}", "{sae}")
IMMEDIATES = ("$-129", "$-128", "$-1", "$15", "$16", "$127", "$128", "$255", "$256")


def slot_of(numbers: tuple[int, int], first: Encoding, second: Encoding,
            taken: set[str]) -> str | None:
    """The field that holds a register numbered `numbers` in the two probes."""
    found = []
    for name in ("reg", "rm", "vvvv", "is4"):
        if name in taken:
            continue
        if name == "is4":
            if not (first.rest and second.rest):
                continue
            values = (first.rest[-1] >> 4, second.rest[-1] >> 4)
        elif name == "rm":
            if first.mod != 3 or second.mod != 3:
                continue
            values = (first.rm, second.rm)
        elif name == "vvvv":
            if first.space == "L":
                continue
            values = (first.vvvv, second.vvvv)
        else:
            values = (first.reg, second.reg)
        if values == numbers:
            found.append(name)
    return found[0] if len(found) == 1 else None


def solve(name: str, kinds: tuple[str, ...], modrm: bool, mode: str, asker: Asker,
          hint: str):
    """The row of `name kinds` as GNU as encodes it under `hint`, or a word
    on why there is none."""
    long = mode == "64"
    texts = [statement_of(name, kinds, mode, which, hint=hint) for which in (0, 1)]
    codes = [asker.code(text, "m" in kinds) for text in texts]
    if None in codes:
        return "refused"
    encodings = [read(code, long, modrm) for code in codes]
    if None in encodings:
        return "not read"
    first, second = encodings
    if first.key() != second.key() or first.rex2 >= 0:
        return "two encodings"
    slots: list[str] = []
    taken: set[str] = set()
    position = 0
    memory = "m" in kinds
    if memory:
        taken.add("rm")
    for kind in kinds:
        if kind in SLOTTED:
            numbers = (probe_number(kind, position, 0), probe_number(kind, position, 1))
            position += 1
            slot = slot_of(numbers, first, second, taken)
            if slot is None:
                return "no field for an operand"
            taken.add(slot)
            slots.append(slot)
        elif kind == "m":
            slots.append("rm")
        elif kind == "i8":
            slots.append("imm")
        else:
            slots.append("none")
    if first.prefixes not in PREFIXES:
        return "prefixes in an order of their own"
    if first.nd_nf & 0x04:
        return "the NF bit is set"
    row = Row(name=name, kinds=kinds, slots=tuple(slots), space=first.space,
              prefixes=first.prefixes, map=first.map, pp=first.pp, w=first.w, l=first.l,
              opcode=first.opcode, modrm=modrm, modes={mode}, nd_nf=first.nd_nf)
    if not modrm:
        if first.rest != second.rest:
            return "bytes that follow no operand"
        row.literal = first.rest
        return row
    if memory:
        if first.mod == 3:
            return "a register where the address is"
        # An address that is one register has a SIB byte only where the
        # instruction wants one.
        row.force_sib = first.sib >= 0
        if first.rm != 3 or first.disp:
            return "an address not as written"
    elif "rm" not in taken:
        if first.mod != 3 or first.rm != second.rm:
            return "no register form"
        row.fixed_rm = first.rm
    if "reg" not in taken:
        if first.reg != second.reg:
            return "ModRM.reg follows no operand"
        row.digit = first.reg
    if "vvvv" not in taken and first.space != "L" and (first.vvvv or second.vvvv):
        return "vvvv follows no operand"
    # What follows the operands' address: a number, half a byte of one, or
    # a byte that is part of the opcode.
    rests = (first.rest, second.rest)
    immediate = "i8" in kinds
    if "is4" in taken:
        if len(first.rest) != 1:
            return "bytes behind the fourth register"
        lows = (first.rest[0] & 15, second.rest[0] & 15)
        if immediate:
            if lows != (5, 9):
                return "the number is not beside the fourth register"
            row.slots = tuple("low4" if s == "imm" else s for s in row.slots)
        elif lows != (0, 0):
            return "bits beside the fourth register"
    elif immediate:
        if not (first.rest and first.rest[-1] == 5 and second.rest[-1] == 9):
            return "the number is not the last byte"
        rests = (first.rest[:-1], second.rest[:-1])
        if rests[0] != rests[1] or len(rests[0]) > 1:
            return "bytes between the operands and the number"
        if rests[0]:
            row.tail = rests[0][0]
    else:
        if rests[0] != rests[1] or len(rests[0]) > 1:
            return "bytes behind the operands"
        if rests[0]:
            row.tail = rests[0][0]
    return row


def unreadable_hint(hint: str, rounding_at: int) -> str:
    """`hint`, unless GNU as cannot read it there: a rounding word in front
    of the first operand stands right behind the mnemonic, and behind a
    pseudo prefix GNU as takes that for one more of them ("invalid character
    '{' in mnemonic"). The word asks for EVEX by itself."""
    return "" if rounding_at == 0 else hint


def refine(row: Row, mode: str, asker: Asker, hint: str, ask_only: bool) -> None:
    """What else the form takes, asked (`ask_only`) and then read."""
    long = mode == "64"
    name, kinds = row.name, row.kinds
    memory = "m" in kinds
    base = base_register(mode)

    def got(text: str) -> Encoding | None:
        if ask_only:
            asker.ask(text)
            return None
        code = asker.code(text, memory)
        return read(code, long, row.modrm) if code is not None else None

    def same_row(enc: Encoding | None) -> bool:
        # The row's own encoding, with whatever prefix carries the register
        # (REX2 in front of a legacy opcode, the bits of APX in an EVEX one).
        return enc is not None and enc.key()[:1] + enc.key()[2:] == (
            row.space, row.map, row.pp, row.w, row.l, row.opcode)

    evex = row.space == "E"
    # How far a one-byte displacement reaches.
    if memory and evex:
        for shift in range(7):
            enc = got(statement_of(name, kinds, mode, hint=hint,
                                   memory=f"{1 << shift}({base})"))
            if enc is not None and enc.mod == 1 and enc.disp == b"\x01":
                row.disp8 = shift
                break
    if evex and row.modrm:
        enc = got(statement_of(name, kinds, mode, hint=hint, last="{%k1}"))
        row.mask = enc is not None and enc.aaa == 1 and enc.z == 0
        enc = got(statement_of(name, kinds, mode, hint=hint, last="{%k1}{z}"))
        row.zero = enc is not None and enc.aaa == 1 and enc.z == 1
        if memory:
            for count in (2, 4, 8, 16, 32):
                decorated = f"({base}){{1to{count}}}"
                # The decoration stands on the address, wherever it is.
                enc = got(statement_of(name, kinds, mode, hint=hint, memory=decorated))
                if enc is not None and enc.b == 1:
                    row.broadcast = count
                for shift in range(7):
                    enc = got(statement_of(name, kinds, mode, hint=hint,
                                           memory=f"{1 << shift}({base}){{1to{count}}}"))
                    if (enc is not None and enc.b == 1 and enc.mod == 1
                            and enc.disp == b"\x01" and row.broadcast == count):
                        row.broadcast_disp8 = shift
                        break
        else:
            # The rounding word stands in front of an operand; GNU as says
            # which one.
            for at in range(len(kinds) + 1):
                found = {}
                for word in ROUNDINGS:
                    enc = got(statement_of(name, kinds, mode, hint=unreadable_hint(hint, at),
                                           rounding=(at, word)))
                    if enc is not None and enc.b == 1:
                        found[word] = enc.l
                if ask_only or not found or row.rounding:
                    continue
                if set(found) == {"{sae}"}:
                    row.rounding, row.rounding_at, row.sae_l = 1, at, found["{sae}"]
                elif [found.get(word) for word in ROUNDINGS[:4]] == [0, 1, 2, 3]:
                    row.rounding, row.rounding_at = 2, at
    if "i8" in kinds:
        taken = []
        for text in IMMEDIATES:
            enc = got(statement_of(name, kinds, mode, hint=hint, immediate=text))
            if enc is not None:
                taken.append(int(text[1:]))
        if not ask_only:
            row.immediate = IMMEDIATE_CLASSES.get(tuple(taken), 0)
            if not row.immediate:
                row.dropped = f"numbers of a range of its own: {taken}"
            elif ("low4" in row.slots) != (row.immediate == 4):
                row.dropped = "a number beside a register that is no nibble"
    # Operands that have to be different registers: the same register in
    # two of them, and GNU as says whether that is a statement.
    slotted = [index for index, kind in enumerate(kinds)
               if kind in SLOTTED and row.slots[index] != "none"]
    positions = {index: position for position, index in enumerate(slotted)}
    for first, second in itertools.combinations(slotted, 2):
        if kinds[first] != kinds[second]:
            continue
        numbers = {index: probe_number(kinds[index], positions[index], 0) for index in slotted}
        numbers[second] = numbers[first]
        text = statement_of(name, kinds, mode, hint=hint, numbers=numbers)
        if ask_only:
            asker.ask(text)
        elif asker.code(text) is None:
            row.distinct |= 1 << (first * 5 + second)
    # A general register that cannot be the stack pointer (`push2`).
    for index in slotted:
        if kinds[index] in ("r16", "r32", "r64"):
            text = statement_of(name, kinds, mode, hint=hint, numbers={index: 4})
            if ask_only:
                asker.ask(text)
            elif asker.code(text, memory) is None:
                row.no_sp |= 1 << index
    # A general register that is an address itself (`movdir64b`, `enqcmd`):
    # the registers of the memory operand are as wide as it is, and GNU as
    # refuses a narrower address.
    if mode != "16" and memory and any(kind in ("r16", "r32", "r64") for kind in kinds):
        narrow = "(%eax)" if long else "(%bx)"
        text = statement_of(name, kinds, mode, hint=hint, memory=narrow)
        if ask_only:
            asker.ask(text)
        else:
            row.addressed = asker.code(text) is None
    # In 16-bit code an address is 16 bits wide unless it says otherwise.
    # A form whose address has to be as wide as a register operand
    # (`enqcmd`) has no row there.
    if mode == "16" and memory:
        text = statement_of(name, kinds, mode, hint=hint, memory="(%bx)")
        if ask_only:
            asker.ask(text)
        else:
            code = asker.code(text)
            if code is None or code[:1] == b"\x67":
                row.dropped = "an address as wide as an operand"
    # A register of APX, as an operand or in the address.
    if long:
        general = next((index for index, kind in enumerate(kinds)
                        if kind in GENERAL and row.slots[index] != "none"), None)
        if memory:
            enc = got(statement_of(name, kinds, mode, hint=hint, memory="(%r17)"))
            row.egpr = same_row(enc) and enc.rm == 17
        elif general is not None:
            enc = got(statement_of(name, kinds, mode, hint=hint, numbers={general: 17}))
            row.egpr = same_row(enc) and getattr(enc, row.slots[general]) == 17


def solve_all(rows, names: set[str], gas: str):
    """{mode: {(name, kinds, pick): Row}} and why forms have no row."""
    solved: dict[str, dict] = {}
    unsolved: Counter = Counter()
    reasons: dict[tuple, str] = {}
    for mode in MODES:
        asker = Asker(gas, mode)
        forms = list(spellings(rows, mode, names))
        for name, kinds, _modrm in forms:
            for hint in HINTS:
                for which in (0, 1):
                    asker.ask(statement_of(name, kinds, mode, which, hint=hint))
        asker.run()
        found: dict[tuple, Row] = {}
        for name, kinds, modrm in forms:
            plain = solve(name, kinds, modrm, mode, asker, "")
            if isinstance(plain, str):
                if plain != "refused":
                    unsolved[plain] += 1
                    reasons[(mode, name, kinds)] = plain
                continue
            found[(name, kinds, 0)] = plain
            for pick, hint in ((1, HINTS[1]), (2, HINTS[2])):
                other = solve(name, kinds, modrm, mode, asker, hint)
                if isinstance(other, str) or other.space == plain.space:
                    continue
                if (pick == 1) != (other.space == "E"):
                    continue
                other.pick = pick
                found[(name, kinds, pick)] = other
        for ask_only in (True, False):
            for (name, kinds, pick), row in found.items():
                refine(row, mode, asker, HINTS[pick], ask_only)
            asker.run()
        for key, row in list(found.items()):
            if row.dropped:
                unsolved[row.dropped.split(":")[0]] += 1
                reasons[(mode, key[0], key[1])] = row.dropped
                del found[key]
        solved[mode] = found
    return solved, unsolved, reasons


def merge(solved: dict[str, dict]) -> list[Row]:
    """One row for a form that is encoded alike in several modes. 16-bit
    code's rows are 32-bit code's with the operand-size prefix the other way
    round, and say only which those are."""
    rows: dict[tuple, Row] = {}
    for mode in ("64", "32"):
        for (name, kinds, pick), row in solved[mode].items():
            key = (name, pick, row.encoding())
            if key in rows:
                rows[key].modes |= row.modes
                rows[key].egpr |= row.egpr
            else:
                rows[key] = row
    real = solved["16"]
    for (name, kinds, pick), row in solved["32"].items():
        other = real.get((name, kinds, pick))
        if other is None:
            continue
        target = rows[(name, pick, row.encoding())]
        mine, theirs = row.prefixes, other.prefixes
        rest_same = (row.encoding()[:3] + row.encoding()[4:]
                     == other.encoding()[:3] + other.encoding()[4:])
        if not rest_same:
            continue
        if mine == theirs:
            pass
        elif mine == b"\x66" + theirs:
            # 16 bits wide: the prefix in 32-bit code, none in 16-bit code.
            target.sized = target.data16 = True
        elif theirs == b"\x66" + mine:
            target.sized = True
        else:
            continue
        target.modes.add("16")
    # A row that 64-bit code alone has may be a sized one too; whether its
    # `66` is the operand size makes no difference there.
    return sorted(rows.values(), key=lambda r: (r.name, [KINDS.index(k) for k in r.kinds],
                                                 r.pick, sorted(r.modes), r.encoding()))


# ─── The range of a one-byte number ──────────────────────────────────────────

@dataclass(frozen=True)
class Number:
    """The range GNU as takes the one-byte number of a form in."""
    name: str
    kinds: tuple[str, ...]      # of one form; empty for every form of the name
    modes: frozenset
    range: int                  # a value of IMMEDIATE_CLASSES


def number_forms(rows, mode: str) -> list[tuple[str, tuple[str, ...]]]:
    """The forms of the table, of any mnemonic, with one number of one byte."""
    names = {row[0] for row in sweep.rows_of(rows, sweep.MODES[mode], set())}
    return sorted({(name, kinds) for name, kinds, _modrm in spellings(rows, mode, names)
                   if kinds.count("i8") == 1})


def solve_numbers(rows, gas: str) -> tuple[list[Number], dict[str, dict]]:
    """The range of the number of every form that takes one in a single byte,
    whatever encodes the form: an arm of the encoders wrote the low byte of
    any number (`bt $256, %ecx` tested bit 0, `int $256` was `int $0`). And
    {mode: {(name, kinds): the numbers of IMMEDIATES it takes}}.

    A form GNU as takes every number in has no range: another row of the
    table has a wider number for it (`add`), or it is a byte instruction,
    whose number GNU as cuts with a warning (`rolb $256, %cl`)."""
    took: dict[str, dict] = {}
    for mode in MODES:
        asker = Asker(gas, mode)
        forms = number_forms(rows, mode)
        for name, kinds in forms:
            for text in IMMEDIATES:
                asker.ask(statement_of(name, kinds, mode, immediate=text))
        asker.run()
        took[mode] = {}
        for name, kinds in forms:
            numbers = tuple(int(text[1:]) for text in IMMEDIATES
                            if asker.code(statement_of(name, kinds, mode, immediate=text),
                                          "m" in kinds) is not None)
            if numbers:
                took[mode][(name, kinds)] = numbers
    # {name: {kinds: {mode: range}}}; range 0 where there is none.
    by_name: dict[str, dict] = defaultdict(lambda: defaultdict(dict))
    for mode, forms in took.items():
        for (name, kinds), numbers in forms.items():
            by_name[name][kinds][mode] = IMMEDIATE_CLASSES.get(numbers, 0)
    out: list[Number] = []
    for name in sorted(by_name):
        forms = by_name[name]
        ranges = {r for by_mode in forms.values() for r in by_mode.values()}
        if ranges == {0}:
            continue
        if len(ranges) == 1:
            modes = frozenset(mode for by_mode in forms.values() for mode in by_mode)
            out.append(Number(name, (), modes, ranges.pop()))
            continue
        # The forms a register by name stands in go first: `%al` is a byte
        # register too.
        for kinds in sorted(forms, key=lambda k: (not any(kind in FIXED for kind in k),
                                                  [KINDS.index(kind) for kind in k])):
            by_range: dict[int, set] = defaultdict(set)
            for mode, found in forms[kinds].items():
                by_range[found].add(mode)
            for found in sorted(by_range):
                if found:
                    out.append(Number(name, kinds, frozenset(by_range[found]), found))
    return out, took


# ─── Verifying ───────────────────────────────────────────────────────────────

# The legacy prefixes in front of an opcode.
LEGACY_PREFIXES = re.compile(rb"^[\x26\x2e\x36\x3e\x64\x65\x66\x67\xf0\xf2\xf3]*")
# Of the statements GNU as refuses, one in this many is given to lccc: each
# takes a run of its own.
REFUSED_SHARE = 8
# How many statements of each kind a report names.
LISTED = int(os.environ.get("LEARNED_LISTED", "40"))
HIGH_BYTES = ("%ah", "%ch", "%dh", "%bh")
VERIFY_HINTS = {"64": ("{evex} ", "{vex} ", "{vex3} ", "{nf} ", "{rex2} ", "lock ", "rep ",
                       "data16 "),
                "32": ("{evex} ", "{vex} ", "{vex3} ", "lock ", "rep ", "data16 "),
                "16": ("{evex} ", "{vex} ", "{vex3} ", "lock ", "rep ")}
# The registers each operand is tried with, by kind: the corners of each
# encoding's range.
REGISTER_NUMBERS = {"64": {"general": (0, 4, 5, 7, 8, 12, 13, 15, 16, 21, 31),
                           "vector": (0, 7, 8, 15, 16, 23, 31), "small": (0, 3, 7)},
                    "32": {"general": (0, 4, 5, 7), "vector": (0, 4, 7), "small": (0, 3, 7)}}
REGISTER_NUMBERS["16"] = REGISTER_NUMBERS["32"]
ADDRESSES = {
    "64": ("(%rdx)", "0x12(%rbx,%rsi,4)", "0x1234(%r9,%r10,2)", "(%rsp)", "(%rbp)", "(%r12)",
           "(%r13)", "-0x80(%rax)", "0x7f(%rax)", "0x80(%rax)", "sym(%rip)", "sym+4(%rip)",
           "0x1000", "sym", "(,%rcx,2)", "(%r17)", "0x40(%r17,%r18,2)", "%fs:(%rax)",
           "(%eax)", "0x12(%r9d,%r10d,8)"),
    "32": ("(%edx)", "0x12(%ebx,%esi,4)", "0x1234(,%edi,2)", "(%esp)", "(%ebp)", "-0x80(%eax)",
           "0x7f(%eax)", "0x80(%eax)", "sym", "sym+4", "0x1000", "%gs:(%eax)", "(%bx)",
           "0x12(%bx,%si)", "0x1234(%bp)"),
    "16": ("(%bx)", "0x12(%bx,%si)", "0x1234(%bp)", "(%di)", "-0x80(%bx)", "0x7f(%si)",
           "0x80(%bp,%di)", "sym", "sym+4", "0x1000", "%gs:(%bx)", "(%edx)",
           "0x12(%ebx,%esi,4)", "(%esp)", "(%ebp)"),
}


def variants(row: Row, mode: str) -> set[str]:
    """Statements of `row` that vary what the solving held still."""
    name, kinds = row.name, row.kinds
    hint = HINTS[row.pick]
    base = base_register(mode)
    out = set()

    def add(**changes) -> None:
        out.add(statement_of(name, kinds, mode, hint=changes.pop("hint", hint), **changes))

    add()
    numbers = REGISTER_NUMBERS[mode]
    slotted = [index for index, kind in enumerate(kinds) if kind in SLOTTED]
    # Each register operand through the corners of its range, the others
    # held; and all of them at once at each corner.
    for index in slotted:
        kind = kinds[index]
        group = ("general" if kind in GENERAL else
                 "vector" if kind in ("x", "y", "z") else "small")
        for number in numbers[group]:
            add(numbers={index: number})
    for corner in (0, 7, 8, 15, 16, 31):
        if corner > 7 and mode != "64":
            break
        add(numbers={index: corner for index in slotted})
    for index in slotted:
        if kinds[index] == "r8":
            for high in HIGH_BYTES[:2]:
                text = statement_of(name, kinds, mode, hint=hint, numbers={index: 1})
                out.add(text.replace("%cl", high, 1))
        # A register of another width, or of another kind, in its place.
        for other in ("r8", "r16", "r32", "r64", "mm", "x", "y", "z", "k"):
            if other == kinds[index] or (other == "r64" and mode != "64"):
                continue
            swapped = kinds[:index] + (other,) + kinds[index + 1:]
            out.add(statement_of(name, swapped, mode, hint=hint))
    # The same register in two operands.
    for first, second in itertools.combinations(slotted, 2):
        if kinds[first] == kinds[second]:
            add(numbers={first: 2, second: 2})
    if "m" in kinds:
        for address in ADDRESSES[mode]:
            add(memory=address)
        # The reach of a one-byte displacement, either side of each end.
        for shift in {row.disp8, row.broadcast_disp8}:
            unit = 1 << shift
            for displacement in (127 * unit, 128 * unit, -128 * unit, -129 * unit, unit + 1, 1):
                add(memory=f"{displacement}({base})")
                if row.broadcast:
                    add(memory=f"{displacement}({base}){{1to{row.broadcast}}}")
        for count in (2, 4, 8, 16, 32):
            add(memory=f"({base}){{1to{count}}}")
            add(memory=f"0x40({base}){{1to{count}}}", last="{%k3}")
    for decoration in ("{%k1}", "{%k7}", "{%k2}{z}", "{z}", "{%k0}"):
        add(last=decoration)
    if len(kinds) > 1 and kinds[0] in SLOTTED:
        # A mask anywhere but on the last operand.
        text = statement_of(name, kinds, mode, hint=hint)
        head, _, tail = text.partition(",")
        out.add(head + "{%k1}," + tail)
    if "m" not in kinds:
        for at in range(len(kinds) + 1):
            for word in ROUNDINGS:
                add(rounding=(at, word), hint=unreadable_hint(hint, at))
        add(rounding=(row.rounding_at, "{rn-sae}"), last="{%k1}",
            hint=unreadable_hint(hint, row.rounding_at))
    if "i8" in kinds:
        for number in (-129, -128, -1, 0, 15, 16, 127, 128, 255, 256):
            add(immediate=f"${number}")
    for word in VERIFY_HINTS[mode]:
        if word.strip() != hint.strip():
            add(hint=word)
    return out


def another_instruction(statement: str, code: bytes) -> str | None:
    """Why lccc refuses a statement GNU as assembles: what GNU as writes is
    not the instruction the statement names. Both are the operand-size
    prefix word in front of an instruction that has no operand size."""
    if not statement.startswith(("data16 ", "data32 ")):
        return None
    if LEGACY_PREFIXES.sub(b"", code)[:1] == b"\x62":
        # No `66` but the bit of the EVEX prefix that stands for it, which
        # makes `push2` an encoding no processor has. lccc refuses the word
        # in front of every EVEX encoding.
        return "the word as a bit of an EVEX prefix"
    if statement.split()[1] in ("vmread", "vmreadq", "vmwrite", "vmwriteq"):
        return "`66 0F 78` and `66 0F 79` are `extrq` and `insertq`"
    return None


def verify(solved: dict[str, dict], merged: list[Row], gas: str, lccc: dict[str, str],
           share: int) -> int:
    """lccc against GNU as over the variants of every written row."""
    written = {(row.name, row.kinds, row.pick, mode) for row in merged for mode in row.modes}
    failures = 0
    for mode in MODES:
        statements = sorted(set().union(*(
            variants(row, mode) for (name, kinds, pick), row in solved[mode].items()
            if (name, kinds, pick, mode) in written)) or set())
        if not statements:
            continue
        sweep_mode = sweep.MODES[mode]
        with tempfile.TemporaryDirectory(prefix="learned-") as tmp:
            theirs = sweep.assemble_gas(statements, gas, sweep_mode, Path(tmp))
            assembled = sorted(theirs)
            good, refused, crashed = sweep.assemble_lccc(statements, assembled, lccc[mode],
                                                         sweep_mode, Path(tmp))
            others = [index for index in range(len(statements)) if index not in theirs
                      and sweep.draw(1, "learned", statements[index]) % share == 0]
            accepted, crashed_alone = sweep.assemble_each(statements, others, lccc[mode],
                                                          sweep_mode, Path(tmp))
        folded = {index for index in assembled
                  if another_instruction(statements[index], theirs[index][0])}
        refused = {index: why for index, why in refused.items() if index not in folded}
        different = [index for index in assembled if index in good and good[index] != theirs[index]]
        same = len(assembled) - len(different) - len(refused) - len(crashed) - len(
            folded - set(good))
        print(f"x86_learned_rows: [{mode}] {len(statements)} statements, GNU as assembles "
              f"{len(assembled)}: same {same}, "
              f"different {len(different)}, refused {len(refused)}, crashed {len(crashed)}, "
              f"another instruction in GNU as's bytes (refused) {len(folded - set(good))}; "
              f"of {len(others)} it refuses, lccc assembles {len(accepted)} and crashes on "
              f"{len(crashed_alone)}")
        for index in different[:LISTED]:
            ours, wanted = good[index], theirs[index]
            print(f"  DIFF   {statements[index]}: lccc {ours[0].hex()} {list(ours[1]) or ''} "
                  f"GNU as {wanted[0].hex()} {list(wanted[1]) or ''}")
        for index, why in sorted(refused.items())[:LISTED]:
            print(f"  REJECT {statements[index]}: {why[:100]} (GNU as {theirs[index][0].hex()})")
        for index, detail in sorted(accepted.items())[:LISTED]:
            print(f"  ACCEPT {statements[index]}: {detail}")
        for index, why in sorted({**crashed, **crashed_alone}.items())[:LISTED]:
            print(f"  CRASH  {statements[index]}: {why[:100]}")
        failures += (len(different) + len(refused) + len(crashed) + len(accepted)
                     + len(crashed_alone))
    if failures:
        print(f"x86_learned_rows: {failures} statements are not what GNU as makes of them",
              file=sys.stderr)
        return 1
    return 0


def verify_numbers(took: dict[str, dict], gas: str, lccc: dict[str, str], share: int) -> int:
    """lccc against GNU as on the numbers of every form with a one-byte one:
    the numbers at the ends of each range, in the forms GNU as takes any of
    them in. A form lccc takes none of is not its instruction; of any other
    it takes the numbers GNU as takes, and writes the same."""
    failures = 0
    for mode in MODES:
        forms = sorted(took[mode])
        if not forms:
            continue
        statements = [statement_of(name, kinds, mode, immediate=text)
                      for name, kinds in forms for text in IMMEDIATES]
        per_form = len(IMMEDIATES)
        sweep_mode = sweep.MODES[mode]
        with tempfile.TemporaryDirectory(prefix="learned-") as tmp:
            theirs = sweep.assemble_gas(statements, gas, sweep_mode, Path(tmp))
            assembled = sorted(theirs)
            good, refused, crashed = sweep.assemble_lccc(statements, assembled, lccc[mode],
                                                         sweep_mode, Path(tmp))
            others = [index for index in range(len(statements)) if index not in theirs
                      and sweep.draw(1, "number", statements[index]) % share == 0]
            accepted, crashed_alone = sweep.assemble_each(statements, others, lccc[mode],
                                                          sweep_mode, Path(tmp))
        different = [index for index in assembled if index in good and good[index] != theirs[index]]
        # Refused where GNU as assembles: of a form lccc assembles with
        # another number.
        known = {index // per_form for index in good}
        cut = {index: why for index, why in refused.items() if index // per_form in known}
        print(f"x86_learned_rows: [{mode}] numbers: {len(forms)} forms, {len(statements)} "
              f"statements, GNU as assembles {len(assembled)}: same "
              f"{len(good) - len(different)}, different {len(different)}, the number refused "
              f"{len(cut)}, the form refused {len(refused) - len(cut)}, crashed {len(crashed)}; "
              f"of {len(others)} it refuses, lccc assembles {len(accepted)} and crashes on "
              f"{len(crashed_alone)}")
        for index in different[:LISTED]:
            print(f"  DIFF   {statements[index]}: lccc {good[index][0].hex()} "
                  f"GNU as {theirs[index][0].hex()}")
        for index, why in sorted(cut.items())[:LISTED]:
            print(f"  REJECT {statements[index]}: {why[:100]} (GNU as {theirs[index][0].hex()})")
        for index, detail in sorted(accepted.items())[:LISTED]:
            print(f"  ACCEPT {statements[index]}: {detail}")
        for index, why in sorted({**crashed, **crashed_alone}.items())[:LISTED]:
            print(f"  CRASH  {statements[index]}: {why[:100]}")
        failures += len(different) + len(cut) + len(accepted) + len(crashed) + len(crashed_alone)
    if failures:
        print(f"x86_learned_rows: {failures} numbers are not taken as GNU as takes them",
              file=sys.stderr)
        return 1
    return 0


# ─── The written file ────────────────────────────────────────────────────────

SLOTS = ("none", "reg", "rm", "vvvv", "is4", "imm", "low4")
SPACES = "LVEX"


def render(rows: list[Row], names: list[str], numbers: list[Number]) -> str:
    out = [
        "//! The instructions no arm of the encoders knows, as GNU as 2.47 encodes",
        "//! them: one row for each form of each mnemonic of the list below.",
        "//!",
        "//! Written by `scripts/x86_learned_rows.py --write`, which asks GNU as;",
        "//! `--check` holds the file to what it answers and `--verify` holds what",
        "//! `learned.rs` makes of the rows to it. Not edited by hand.",
        "//!",
        "//! The list:",
    ]
    line = "//!"
    for name in names:
        if len(line) + len(name) + 1 > 78:
            out.append(line)
            line = "//!"
        line += " " + name
    out.append(line)
    out += [
        "",
        "#[rustfmt::skip]",
        "use super::learned::{",
        "    ADDRESSED, DATA16, EGPR, M16, M32, M64, MASK, MODRM, ND, Number, ROUND, Row, SAE, SIB,",
        "    SIZED, W, WHEN_EVEX, WHEN_VEX, ZERO, number, row,",
        "};",
        "",
        "/// The mnemonics that have rows, in order.",
        "#[rustfmt::skip]",
    ]
    by_name: dict[str, list[Row]] = defaultdict(list)
    for row in rows:
        by_name[row.name].append(row)
    have = sorted(by_name)
    out.append(f"pub(super) static NAMES: [&str; {len(have)}] = [")
    line = "   "
    for name in have:
        item = f' "{name}",'
        if len(line) + len(item) > 100:
            out.append(line)
            line = "   "
        line += item
    out.append(line)
    out.append("];")
    out += ["", "/// Where the rows of each mnemonic of `NAMES` begin in `ROWS`, and the end.",
            "#[rustfmt::skip]"]
    starts = []
    total = 0
    for name in have:
        starts.append(total)
        total += len(by_name[name])
    starts.append(total)
    out.append(f"pub(super) static STARTS: [u16; {len(starts)}] = [")
    line = "   "
    for start in starts:
        item = f" {start},"
        if len(line) + len(item) > 100:
            out.append(line)
            line = "   "
        line += item
    out.append(line)
    out.append("];")
    out += ["", "/// The rows, by mnemonic; of one mnemonic and one form, the encoding GNU as",
            "/// chooses first.",
            "#[rustfmt::skip]",
            f"pub(super) static ROWS: [Row; {total}] = ["]
    for name in have:
        out.append(f"    // {name}")
        for row in by_name[name]:
            out.append(f"    {render_row(row)}, // {' '.join(row.kinds) or '-'}")
    out.append("];")
    out += render_numbers(numbers)
    return "\n".join(out) + "\n"


RANGES = {1: "-128..=255", 2: "0..=255", 3: "-128..=127", 4: "0..=15"}


def render_numbers(numbers: list[Number]) -> list[str]:
    """The table of the ranges of one-byte numbers."""
    by_name: dict[str, list[Number]] = defaultdict(list)
    for entry in numbers:
        by_name[entry.name].append(entry)
    have = sorted(by_name)
    out = ["", "/// The mnemonics of GNU as's table whose one-byte number has a range, in order:",
           "/// of every encoder's instructions, not of the rows above alone.",
           "#[rustfmt::skip]",
           f"pub(super) static NUMBER_NAMES: [&str; {len(have)}] = ["]
    line = "   "
    for name in have:
        item = f' "{name}",'
        if len(line) + len(item) > 100:
            out.append(line)
            line = "   "
        line += item
    out += [line, "];", "",
            "/// Where the ranges of each mnemonic of `NUMBER_NAMES` begin in `NUMBERS`, and the",
            "/// end.", "#[rustfmt::skip]"]
    starts, total = [], 0
    for name in have:
        starts.append(total)
        total += len(by_name[name])
    starts.append(total)
    out.append(f"pub(super) static NUMBER_STARTS: [u16; {len(starts)}] = [")
    line = "   "
    for start in starts:
        item = f" {start},"
        if len(line) + len(item) > 100:
            out.append(line)
            line = "   "
        line += item
    out += [line, "];", "",
            "/// The ranges: of one form of a mnemonic (the kinds of its operands), or of every",
            "/// form it has (no kinds).", "#[rustfmt::skip]",
            f"pub(super) static NUMBERS: [Number; {total}] = ["]
    for name in have:
        for entry in by_name[name]:
            kinds = [KINDS.index(k) + 1 for k in entry.kinds] + [0] * (5 - len(entry.kinds))
            modes = " | ".join(f"M{mode}" for mode in MODES if mode in entry.modes)
            out.append(f"    number({modes}, [{', '.join(map(str, kinds))}], {entry.range}), "
                       f"// {name} {' '.join(entry.kinds) or '*'}: {RANGES[entry.range]}")
    out.append("];")
    return out


def render_row(row: Row) -> str:
    """`row` as an argument list of `learned::row`."""
    kinds = [KINDS.index(k) + 1 for k in row.kinds] + [0] * (5 - len(row.kinds))
    slots = [SLOTS.index(s) for s in row.slots] + [0] * (5 - len(row.slots))
    flags = []
    if row.w:
        flags.append("W")
    if row.modrm:
        flags.append("MODRM")
    if row.sized:
        flags.append("SIZED")
    if row.force_sib:
        flags.append("SIB")
    if row.mask:
        flags.append("MASK")
    if row.zero:
        flags.append("ZERO")
    if row.egpr:
        flags.append("EGPR")
    for mode in MODES:
        if mode in row.modes:
            flags.append(f"M{mode}")
    if row.rounding == 1:
        flags.append("SAE")
    elif row.rounding == 2:
        flags.append("ROUND")
    if row.pick == 1:
        flags.append("WHEN_EVEX")
    elif row.pick == 2:
        flags.append("WHEN_VEX")
    if row.nd_nf & 0x10:
        flags.append("ND")
    if row.addressed:
        flags.append("ADDRESSED")
    if row.space == "L":
        prefix = PREFIXES[row.prefixes]
        # The operand-size prefix stands in front of the mandatory one.
        if len(row.prefixes) == 2:
            flags.append("DATA16")
        elif row.prefixes == b"\x66" and row.data16:
            flags.append("DATA16")
            prefix = 0
    else:
        prefix = row.pp
    literal = list(row.literal) + [0] * (3 - len(row.literal))
    return "row(%s, %d, [%s], [%s], %d, %d, %d, %d, 0x%02X, %d, %d, %d, [%s], %d, [%d, %d, %d, %d, %d, %d, %d])" % (
        " | ".join(flags) if flags else "0", row.distinct,
        ", ".join(map(str, kinds)), ", ".join(map(str, slots)),
        SPACES.index(row.space), row.map, prefix, row.l, row.opcode,
        row.digit, row.fixed_rm, row.tail,
        ", ".join(f"0x{b:02X}" for b in literal), len(row.literal),
        row.disp8, row.broadcast, row.broadcast_disp8, row.rounding_at, row.sae_l,
        row.immediate, row.no_sp)


def listed_names() -> list[str]:
    """The list in the header of the written file."""
    if not OUT.exists():
        return []
    names: list[str] = []
    listing = False
    for line in OUT.read_text().splitlines():
        if line.startswith("//! The list:"):
            listing = True
        elif listing and line.startswith("//!"):
            names += line[3:].split()
        elif listing:
            break
    return names


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    what = parser.add_mutually_exclusive_group(required=True)
    what.add_argument("--write", action="store_true")
    what.add_argument("--check", action="store_true")
    what.add_argument("--verify", action="store_true")
    what.add_argument("--show", action="store_true", help="print the rows of --add")
    parser.add_argument("--add", action="append", default=[], metavar="MNEMONIC")
    parser.add_argument("--add-from", metavar="FILE", help="a file of mnemonics to add")
    parser.add_argument("--as", dest="gas", default="as")
    parser.add_argument("--table", help="i386-opc.tbl of binutils 2.47")
    parser.add_argument("--lccc", default=str(REPO / "target" / "fastbuild" / "lccc-x86"))
    parser.add_argument("--lccc32", default=str(REPO / "target" / "fastbuild" / "lccc-i686"))
    parser.add_argument("--share", type=int, default=REFUSED_SHARE,
                        help="with --verify: of the statements GNU as refuses, give lccc one "
                             f"in this many (default {REFUSED_SHARE}; 1 for all)")
    parser.add_argument("--mode", choices=MODES, action="append",
                        help="with --verify: only this mode")
    args = parser.parse_args(argv)

    version = sweep.gas_version(args.gas)
    if not sweep.GAS_VERSION.fullmatch(version):
        print(f"x86_learned_rows: {args.gas} is GNU as {version or '?'}, not 2.47", file=sys.stderr)
        return 2
    rows = list(sweep.expand_rows(sweep.table_text(args.table)))
    added = list(args.add)
    if args.add_from:
        added += Path(args.add_from).read_text().split()
    names = sorted(set(added) if args.show else set(listed_names()) | set(added))
    solved, unsolved, reasons = solve_all(rows, set(names), args.gas)
    merged = merge(solved)
    numbers, took = solve_numbers(rows, args.gas)
    if args.show:
        for row in merged:
            print(f"{row.name} {' '.join(row.kinds)} | {row.space} {row.prefixes.hex() or '-'} "
                  f"map{row.map} pp{row.pp} W{row.w} L{row.l} {row.opcode:02X} "
                  f"lit={row.literal.hex() or '-'} | {' '.join(row.slots)} | {render_row(row)}")
        for (mode, name, kinds), reason in sorted(reasons.items()):
            print(f"unsolved [{mode}] {name} {' '.join(kinds)}: {reason}")
        return 0
    text = render(merged, names, numbers)
    if args.write:
        OUT.write_text(text)
        print(f"x86_learned_rows: {len(merged)} rows of {len({r.name for r in merged})} "
              f"mnemonics ({len(names)} listed); unsolved: {dict(unsolved)}; "
              f"{len(numbers)} ranges of {len({n.name for n in numbers})} mnemonics' numbers")
        return 0
    if args.check:
        if OUT.read_text() != text:
            print("x86_learned_rows: learned_rows.rs is not what GNU as answers; "
                  "run --write and read the difference", file=sys.stderr)
            return 1
        print(f"x86_learned_rows: ok ({len(merged)} rows)")
        return 0
    if OUT.read_text() != text:
        print("x86_learned_rows: learned_rows.rs is not what GNU as answers; run --write",
              file=sys.stderr)
        return 1
    if args.mode:
        solved = {mode: (rows if mode in args.mode else {}) for mode, rows in solved.items()}
        took = {mode: (forms if mode in args.mode else {}) for mode, forms in took.items()}
    binaries = {"64": args.lccc, "32": args.lccc32, "16": args.lccc32}
    share = max(1, args.share)
    failed = verify(solved, merged, args.gas, binaries, share)
    failed |= verify_numbers(took, args.gas, binaries, share)
    if not failed:
        print("x86_learned_rows: verified")
    return failed


if __name__ == "__main__":
    raise SystemExit(main())
