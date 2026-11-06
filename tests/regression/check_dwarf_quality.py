#!/usr/bin/env python3
"""DWARF quality gate: does an object say enough to debug the program?

A debugger answers four questions per address and per name, and each one has a
section behind it:

* *which line is this instruction?* -- `.debug_line`, indexed by the unit's
  `DW_AT_stmt_list`;
* *which function is this, and where does it start?* -- `DW_TAG_subprogram` in
  `.debug_info`;
* *which unit owns this address?* -- `.debug_aranges`, read before the units so
  that the answer costs one binary search;
* *how do I unwind out of it?* -- `.eh_frame`, read before any `.debug_*`.

This gate decodes those four sections with its own reader and checks the
properties a consumer depends on, on programs whose source it wrote itself, so
that a row, a function and a source line can be compared without trusting a
third-party tool. `readelf` is not consulted: it is advisory at best (it prints
`Unknown AT value` for attributes newer than the binutils it came from) and
absent in some environments.

Every check states a measurement, not a preference, and the measurements come
from GCC 16.2.0 which this gate can use as an oracle when it is installed:

* `-O0` objects carry *every* internal definition the source has -- an
  unreferenced `static` function or object is a symbol and a DIE -- while `-O1`
  and above carry only what the reference graph reaches. A compiler that prunes
  at `-O0` turns `asm("call helper")` into an undefined-symbol link error that
  GCC does not have, and hides a function the user wrote from the debugger
  (`--gcc` cross-checks this, and the rule is asserted against lccc alone so the
  gate also runs where no GCC 16 exists).
* an unused `static inline` function is absent at `-O0`, because GCC drops it
  there too (`-fkeep-inline-functions` is what keeps it).

Gaps that are known and not yet closed are listed in `KNOWN_GAPS`: they are
reported in the markdown and the JSON, and they do not fail the gate. A gap that
disappears is reported as such, so the entry can be deleted rather than rotting.

Usage:
    python3 tests/regression/check_dwarf_quality.py --lccc target/fastbuild/lccc
    python3 tests/regression/check_dwarf_quality.py --lccc ... --gcc gcc-16 \\
        --json out.json --markdown out.md
    python3 tests/regression/check_dwarf_quality.py --self-test
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import struct
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

# The ELF section reader, LEB128 decoders and the DWARF form decoder already
# live in the dialect gate's own reader. Importing them keeps one decoder in the
# tree instead of two that drift; the pieces used here are the pure ones that
# take bytes, so nothing of that script's policy leaks in.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_c_std_diagnostics import (  # noqa: E402
    _byte_string,
    _decompress,
    _form_value,
    _sleb,
    _uleb,
)

GATE_DATE = "2026-11-05"

# --- DWARF tags and attributes used below ------------------------------------
DW_TAG_ARRAY_TYPE = 0x01
DW_TAG_ENUMERATION_TYPE = 0x04
DW_TAG_FORMAL_PARAMETER = 0x05
DW_TAG_UNSPECIFIED_PARAMETERS = 0x18
DW_TAG_LEXICAL_BLOCK = 0x0B
DW_TAG_MEMBER = 0x0D
DW_TAG_POINTER_TYPE = 0x0F
DW_TAG_COMPILE_UNIT = 0x11
DW_TAG_STRUCTURE_TYPE = 0x13
DW_TAG_SUBROUTINE_TYPE = 0x15
DW_TAG_TYPEDEF = 0x16
DW_TAG_UNION_TYPE = 0x17
DW_TAG_INLINED_SUBROUTINE = 0x1D
DW_TAG_SUBRANGE_TYPE = 0x21
DW_TAG_BASE_TYPE = 0x24
DW_TAG_CONST_TYPE = 0x26
DW_TAG_ENUMERATOR = 0x28
DW_TAG_SUBPROGRAM = 0x2E
DW_TAG_VARIABLE = 0x34
DW_TAG_VOLATILE_TYPE = 0x35
DW_TAG_RESTRICT_TYPE = 0x37

DW_AT_LOCATION = 0x02
DW_AT_RANGES = 0x55
DW_AT_NAME = 0x03
DW_AT_ABSTRACT_ORIGIN = 0x31
DW_AT_SPECIFICATION = 0x47
DW_AT_STMT_LIST = 0x10
DW_AT_LOW_PC = 0x11
DW_AT_HIGH_PC = 0x12
DW_AT_LANGUAGE = 0x13
DW_AT_COMP_DIR = 0x1B
DW_AT_PRODUCER = 0x25
DW_AT_DECL_COLUMN = 0x39
DW_AT_DECL_FILE = 0x3A
DW_AT_DECL_LINE = 0x3B
DW_AT_EXTERNAL = 0x3F
DW_AT_FRAME_BASE = 0x40
DW_AT_TYPE = 0x49
DW_AT_BYTE_SIZE = 0x0B
DW_AT_ENCODING = 0x3E
DW_AT_DATA_MEMBER_LOCATION = 0x38
DW_AT_BIT_SIZE = 0x0D
DW_AT_BIT_OFFSET = 0x0C
DW_AT_DATA_BIT_OFFSET = 0x6B
DW_AT_UPPER_BOUND = 0x2F
DW_AT_COUNT = 0x37
DW_AT_PROTOTYPED = 0x27
DW_AT_DECLARATION = 0x3C
DW_AT_CONST_VALUE = 0x1C
DW_AT_GNU_VECTOR = 0x2107


#: `DW_OP_addr`: the only location expression this writer emits for an object at
#: a fixed address.
DW_OP_ADDR = 0x03

#: `DW_OP_fbreg`: one signed LEB128 offset from the frame base, which is how a
#: parameter that lives in the frame is described.
DW_OP_FBREG = 0x91

DW_FORM_STRING = 0x08
DW_FORM_ADDR = 0x01
#: Reference forms that are relative to the start of the unit's header, and the
#: one form that is relative to `.debug_info`.
DW_FORM_UNIT_REFS = (0x11, 0x12, 0x13, 0x14)  # ref1, ref2, ref4, ref8
DW_FORM_REF_ADDR = 0x17

#: `.symtab` symbol types and bindings this gate reads.
STT_FUNC = 2
STT_OBJECT = 1
STB_LOCAL = 0
SHN_UNDEF = 0

#: Known, documented gaps. The key is the check id a failure would carry; the
#: value says what a debugger cannot do yet and where the work item lives. A gap
#: that stops reproducing is reported (once) and the entry should be deleted.
KNOWN_GAPS: dict[str, str] = {
    "inlining": (
        "no DW_TAG_inlined_subroutine: an optimized frame shows the caller with "
        "the callee's line, so a debugger cannot report the inline stack (D6)"
    ),
}

#: Hard limits that make a malformed section a failure rather than a traceback.
MAX_UNITS = 4096
MAX_DIES = 1 << 20
MAX_LINE_ROWS = 1 << 21


# --- ELF ----------------------------------------------------------------------


@dataclass
class Symbol:
    name: str
    value: int
    size: int
    info: int
    shndx: int

    @property
    def binding(self) -> int:
        return self.info >> 4

    @property
    def symbol_type(self) -> int:
        return self.info & 0xF

    @property
    def defined(self) -> bool:
        return self.shndx != SHN_UNDEF


@dataclass
class Elf:
    """Sections with their addresses, symbols, and the byte order.

    `sections` and `order` are what `check_c_std_diagnostics._form_value` needs,
    which is why this class carries them under exactly those names.
    """

    sections: dict[str, bytes] = field(default_factory=dict)
    addresses: dict[str, int] = field(default_factory=dict)
    symbols: list[Symbol] = field(default_factory=list)
    order: str = "little"
    address_size: int = 8

    def u32(self, offset: int, data: bytes) -> int:
        return int.from_bytes(data[offset : offset + 4], self.order)

    def u64(self, offset: int, data: bytes) -> int:
        return int.from_bytes(data[offset : offset + 8], self.order)

    def functions(self, prefix: str = "") -> list[Symbol]:
        return [
            symbol
            for symbol in self.symbols
            if symbol.symbol_type == STT_FUNC
            and symbol.defined
            and symbol.size > 0
            and symbol.name.startswith(prefix)
        ]

    def objects(self, prefix: str = "") -> list[Symbol]:
        return [
            symbol
            for symbol in self.symbols
            if symbol.symbol_type == STT_OBJECT
            and symbol.defined
            and symbol.name.startswith(prefix)
        ]


def read_elf(path: Path) -> Elf:
    blob = path.read_bytes()
    if blob[:4] != b"\x7fELF":
        raise ValueError(f"{path} is not an ELF object")
    elf_class, encoding = blob[4], blob[5]
    if encoding not in (1, 2):
        raise ValueError(f"unknown ELF encoding {encoding}")
    order = "little" if encoding == 1 else "big"
    elf = Elf(order=order, address_size=8 if elf_class == 2 else 4)
    if elf_class == 2:
        table = int.from_bytes(blob[0x28:0x30], order)
        entry_size = int.from_bytes(blob[0x3A:0x3C], order)
        count = int.from_bytes(blob[0x3C:0x3E], order)
        names_index = int.from_bytes(blob[0x3E:0x40], order)
        header = struct.Struct(("<>" if order == "big" else "<") + "IIQQQQIIQQ")
    elif elf_class == 1:
        table = int.from_bytes(blob[0x20:0x24], order)
        entry_size = int.from_bytes(blob[0x2E:0x30], order)
        count = int.from_bytes(blob[0x30:0x32], order)
        names_index = int.from_bytes(blob[0x32:0x34], order)
        header = struct.Struct(("<>" if order == "big" else "<") + "IIIIIIIIII")
    else:
        raise ValueError(f"unknown ELF class {elf_class}")
    if entry_size < header.size:
        raise ValueError("section header table entry is too small")

    def section(index: int) -> tuple[int, ...]:
        return header.unpack_from(blob, table + index * entry_size)

    if count == 0:
        count = section(0)[5]
    if names_index == 0xFFFF:
        names_index = section(0)[6]
    names_start, names_size = section(names_index)[4], section(names_index)[5]
    names = blob[names_start : names_start + names_size]
    for index in range(count):
        fields = section(index)
        name_start = fields[0]
        name_end = names.find(b"\x00", name_start)
        name = names[name_start : name_end if name_end >= 0 else len(names)]
        start, size, flags, address = fields[4], fields[5], fields[2], fields[3]
        payload = blob[start : start + size]
        if flags & 0x800 and size:  # SHF_COMPRESSED
            payload = _decompress(payload, elf_class, order, name.decode("ascii", "replace"))
        key = name.decode("ascii", "replace")
        elf.sections[key] = payload
        elf.addresses[key] = address
    _read_symbols(blob, elf, header, entry_size, count, order)
    _apply_relocations(elf, elf_class, order)
    return elf


#: Relocation types this reader applies: the ones that place an absolute value
#: (a section offset or an address) into a debug section. Every DWARF reference in
#: an object file is one of these, and in an object the value on disk is zero
#: until the linker applies it -- which is why a reader that ignores relocations
#: sees every string offset as 0.
_ABSOLUTE_RELOCATIONS = frozenset(
    {
        1,  # R_X86_64_64, R_386_32, R_ARM_ABS32, R_RISCV_32
        10,  # R_X86_64_32
        11,  # R_X86_64_32S
        257,  # R_AARCH64_ABS64
        258,  # R_AARCH64_ABS32
    }
)


def _apply_relocations(elf: "Elf", elf_class: int, order: str) -> None:
    """Patch debug sections with their relocations, as a linker would.

    Without this an *object* file's `.debug_str` offsets all read as zero (the
    value lives in the addend), and every name a DIE states resolves to whatever
    string sits at offset zero. A gate must read an object the way a consumer
    does -- after relocation -- or it will bless a file no debugger can read.
    """
    for name in [section for section in elf.sections if ".rel" in section]:
        if not (name.startswith(".rela") or name.startswith(".rel")):
            continue
        payload = elf.sections[name]
        target_name = name[5:] if name.startswith(".rela.") else name[4:]
        target = elf.sections.get(target_name)
        if target is None or not payload:
            continue
        has_addend = name.startswith(".rela")
        if elf_class == 2:
            stride = 24 if has_addend else 16
            layout = struct.Struct(("<>" if order == "big" else "<") + ("QQq" if has_addend else "QQ"))
        else:
            stride = 12 if has_addend else 8
            layout = struct.Struct(("<>" if order == "big" else "<") + ("IIi" if has_addend else "II"))
        patched = bytearray(target)
        for offset in range(0, len(payload) - stride + 1, stride):
            fields = layout.unpack_from(payload, offset)
            where, info = fields[0], fields[1]
            addend = fields[2] if has_addend else None
            symbol_index = info >> 32 if elf_class == 2 else info >> 8
            kind = info & 0xFFFFFFFF if elf_class == 2 else info & 0xFF
            if kind not in _ABSOLUTE_RELOCATIONS:
                continue
            width = {1: 8, 10: 4, 11: 4, 257: 8, 258: 4}[kind]
            if where + width > len(patched):
                continue
            if addend is None:
                addend = int.from_bytes(patched[where : where + width], order)
            value = addend
            if symbol_index < len(elf.symbols):
                value += elf.symbols[symbol_index].value
            patched[where : where + width] = (value & ((1 << (width * 8)) - 1)).to_bytes(
                width, order
            )
        elf.sections[target_name] = bytes(patched)


def _read_symbols(blob: bytes, elf: Elf, header, entry_size: int, count: int, order: str) -> None:
    """Fill `elf.symbols` from `.symtab`, the table a debugger looks names up in."""
    strings = elf.sections.get(".strtab")
    payload = elf.sections.get(".symtab")
    if strings is None or payload is None:
        return
    mark = "<" if order == "little" else ">"
    if elf.address_size == 8:
        stride, layout = 24, struct.Struct(mark + "IBBHQQ")
    else:
        stride, layout = 16, struct.Struct(mark + "IIIBBH")
    for offset in range(0, len(payload) - stride + 1, stride):
        fields = layout.unpack_from(payload, offset)
        if elf.address_size == 8:
            name, info, _other, shndx, value, size = fields
        else:
            name, value, size, info, _other, shndx = fields
        if name >= len(strings):
            continue
        elf.symbols.append(
            Symbol(
                name=_byte_string(strings, name),
                value=value,
                size=size,
                info=info,
                shndx=shndx,
            )
        )


# --- .debug_info --------------------------------------------------------------


@dataclass
class Die:
    offset: int
    tag: int
    attrs: dict[int, object]
    depth: int
    #: Form each attribute was encoded with. `DW_AT_high_pc` needs it: an
    #: address is a form of its own, a constant is an offset from `low_pc`.
    forms: dict[int, int] = field(default_factory=dict)
    #: The abbreviation this DIE was written with, so a declaration no DIE uses
    #: can be found: an abbreviation costs bytes in every object and a parse for
    #: every consumer, and one that describes nothing describes it for nobody.
    code: int = 0

    def name(self) -> str:
        value = self.attrs.get(DW_AT_NAME, "")
        return value if isinstance(value, str) else ""


def origin_chain(die: Die, unit: "Unit") -> list[Die]:
    """`die`, then every DIE it names through `DW_AT_abstract_origin`.

    An optimizing compiler moves a function's description onto the abstract DIE
    and leaves the concrete one with a range and a reference: `decl_file`,
    `decl_line`, `type` and the parameter list all live on the origin. A consumer
    follows the chain, so the gate does too.
    """
    chain = [die]
    seen = {die.offset}
    current = die
    while True:
        reference = current.attrs.get(DW_AT_ABSTRACT_ORIGIN)
        if reference is None:
            reference = current.attrs.get(DW_AT_SPECIFICATION)
        if not isinstance(reference, int):
            return chain
        target = _die_offset(current, reference, unit)
        if target is None or target in seen:
            return chain
        following = unit.die_at(target)
        if following is None:
            return chain
        chain.append(following)
        seen.add(target)
        current = following


def resolved(die: Die, unit: "Unit", attribute: int) -> object | None:
    """The first value of `attribute` along the origin chain."""
    for candidate in origin_chain(die, unit):
        if attribute in candidate.attrs:
            return candidate.attrs[attribute]
    return None


def die_name(die: Die, unit: "Unit") -> str:
    """The source name of a DIE, following `DW_AT_abstract_origin`.

    An optimizing compiler factors a function's description into an *abstract*
    DIE (name, type, parameters) and concrete DIEs that carry only a range and a
    reference to it -- that is the shape GCC 16.2 emits for an out-of-line copy of
    an inlined function, and the shape D6 of the campaign will introduce here.
    A consumer resolves the reference, so a gate that only reads `DW_AT_name`
    sees an unnamed DIE and reports a function nobody describes.
    """
    name = resolved(die, unit, DW_AT_NAME)
    return name if isinstance(name, str) else ""


def _die_offset(die: Die, reference: int, unit: "Unit") -> int | None:
    """Turn a reference attribute into an offset inside `.debug_info`."""
    form = die.forms.get(DW_AT_ABSTRACT_ORIGIN)
    if DW_AT_SPECIFICATION in die.forms and die.forms.get(DW_AT_SPECIFICATION) == DW_FORM_REF_ADDR:
        form = DW_FORM_REF_ADDR
    if form == DW_FORM_REF_ADDR:
        return reference
    if form in DW_FORM_UNIT_REFS:
        return unit.offset + reference
    return None


@dataclass
class Unit:
    offset: int
    version: int
    address_size: int
    offset_size: int
    dies: list[Die] = field(default_factory=list)
    #: Abbreviation codes this unit's table declares.
    declared: list[int] = field(default_factory=list)

    def root(self) -> Die | None:
        return self.dies[0] if self.dies else None

    def by_tag(self, tag: int) -> list[Die]:
        return [die for die in self.dies if die.tag == tag]

    def die_at(self, offset: int) -> Die | None:
        """The DIE starting at `offset`, or `None` when no DIE starts there."""
        for die in self.dies:
            if die.offset == offset:
                return die
        return None


@dataclass
class Abbrev:
    tag: int
    children: bool
    attributes: list[tuple[int, int, int | None]]


def _abbrevs(abbrev: bytes, offset: int) -> dict[int, Abbrev]:
    """Abbreviation declarations with the tag the shared reader discards."""
    table: dict[int, Abbrev] = {}
    while True:
        code, offset = _uleb(abbrev, offset)
        if code == 0:
            return table
        tag, offset = _uleb(abbrev, offset)
        children = abbrev[offset] != 0
        offset += 1
        attributes: list[tuple[int, int, int | None]] = []
        while True:
            attribute, offset = _uleb(abbrev, offset)
            form, offset = _uleb(abbrev, offset)
            if attribute == 0 and form == 0:
                break
            implicit = None
            if form == 0x21:  # DW_FORM_implicit_const
                implicit, offset = _sleb(abbrev, offset)
            attributes.append((attribute, form, implicit))
        table[code] = Abbrev(tag=tag, children=children, attributes=attributes)


def units(elf: Elf) -> list[Unit]:
    """Every compile unit in `.debug_info`, with its DIE tree flattened."""
    info = elf.sections.get(".debug_info")
    abbrev = elf.sections.get(".debug_abbrev")
    if info is None:
        return []
    if abbrev is None:
        raise ValueError(".debug_info without .debug_abbrev")
    out: list[Unit] = []
    offset = 0
    while offset + 4 <= len(info) and len(out) < MAX_UNITS:
        start = offset
        length = int.from_bytes(info[offset : offset + 4], elf.order)
        offset += 4
        offset_size = 4
        if length == 0xFFFFFFFF:
            offset_size = 8
            length = int.from_bytes(info[offset : offset + 8], elf.order)
            offset += 8
        if length == 0:
            break
        end = offset + length
        if end > len(info):
            raise ValueError(f"unit at {start:#x} runs past .debug_info")
        version = int.from_bytes(info[offset : offset + 2], elf.order)
        offset += 2
        address_size = 0
        if version >= 5:
            # DW_UT_* then address size; only DW_UT_compile/skeleton occur here.
            offset += 1
            address_size = info[offset]
            offset += 1
        abbrev_offset = int.from_bytes(info[offset : offset + offset_size], elf.order)
        offset += offset_size
        if version < 5:
            address_size = info[offset]
            offset += 1
        table = _abbrevs(abbrev, abbrev_offset)
        unit = Unit(offset=start, version=version, address_size=address_size, offset_size=offset_size)
        unit.declared = sorted(table)
        depth = 0
        while offset < end:
            die_offset = offset
            code, offset = _uleb(info, offset)
            if code == 0:
                depth = max(depth - 1, 0)
                continue
            if code not in table:
                raise ValueError(f"unit at {start:#x}: undeclared abbreviation {code}")
            declaration = table[code]
            attrs: dict[int, object] = {}
            forms: dict[int, int] = {}
            for attribute, form, implicit in declaration.attributes:
                value, offset = _form_value(
                    form, info, offset, elf, address_size, offset_size, implicit
                )
                attrs[attribute] = value
                forms[attribute] = form
            if len(unit.dies) < MAX_DIES:
                unit.dies.append(
                    Die(
                        offset=die_offset,
                        tag=declaration.tag,
                        attrs=attrs,
                        depth=depth,
                        forms=forms,
                        code=code,
                    )
                )
            if declaration.children:
                depth += 1
        out.append(unit)
        offset = end
    return out


def subprogram_ranges(elf: Elf, unit: Unit, version: int) -> list[tuple[str, int, int, Die]]:
    """`(name, low, high, die)` for every subprogram DIE that states a range.

    A subprogram with no `DW_AT_low_pc` is a declaration -- a function this unit
    calls but does not define -- and is deliberately not returned.
    """
    out = []
    for die in unit.by_tag(DW_TAG_SUBPROGRAM):
        low = die.attrs.get(DW_AT_LOW_PC)
        high = die.attrs.get(DW_AT_HIGH_PC)
        if not isinstance(low, int) or not isinstance(high, int):
            continue
        # DWARF 4 introduced the size form: `high_pc` is an address only when it
        # is encoded as one, otherwise it is a byte count from `low_pc`.
        if die.forms.get(DW_AT_HIGH_PC, DW_FORM_ADDR) != DW_FORM_ADDR:
            high = low + high
        out.append((die_name(die, unit), low, high, die))
    return out


# --- .debug_line --------------------------------------------------------------


@dataclass
class LineRow:
    address: int
    file: int
    line: int
    column: int
    is_stmt: bool
    end_sequence: bool
    prologue_end: bool
    epilogue_begin: bool
    discriminator: int
    isa: int

    @property
    def zero_width(self) -> bool:
        return False  # filled in by `line_table`: see `LineTable.spans`


@dataclass
class LineTable:
    offset: int
    version: int
    address_size: int
    rows: list[LineRow]
    files: list[str]
    directories: list[str]

    def spans(self) -> list[tuple[LineRow, int]]:
        """Rows paired with their byte width (0 for the last row of a sequence).

        A zero-width row sits on the boundary between two ranges: a consumer
        looking up that exact address lands on the next row, so the row is not
        "outside" anything and must not be read as if it covered code.
        """
        out: list[tuple[LineRow, int]] = []
        for index, row in enumerate(self.rows):
            if row.end_sequence:
                out.append((row, 0))
                continue
            following = self.rows[index + 1] if index + 1 < len(self.rows) else None
            width = 0 if following is None or following.address < row.address else (
                following.address - row.address
            )
            out.append((row, width))
        return out


def _line_header(elf: Elf, offset: int) -> tuple[dict[str, object], int]:
    """Parse a line program header; returns its fields and where the program starts."""
    line = elf.sections.get(".debug_line")
    if line is None:
        raise ValueError("no .debug_line section")
    start = offset
    length = int.from_bytes(line[offset : offset + 4], elf.order)
    offset += 4
    offset_size = 4
    if length == 0xFFFFFFFF:
        offset_size = 8
        length = int.from_bytes(line[offset : offset + 8], elf.order)
        offset += 8
    end = offset + length
    version = int.from_bytes(line[offset : offset + 2], elf.order)
    offset += 2
    header: dict[str, object] = {"version": version, "end": end, "offset_size": offset_size}
    if version >= 5:
        address_size = line[offset]
        header["address_size"] = address_size
        offset += 2  # address size, segment selector size
    else:
        header["address_size"] = elf.address_size
    header_length = int.from_bytes(line[offset : offset + offset_size], elf.order)
    offset += offset_size
    program_start = offset + header_length
    minimum = line[offset]
    offset += 1
    max_ops = line[offset] if version >= 4 else 1
    offset += 1 if version >= 4 else 0
    default_is_stmt = line[offset] != 0
    offset += 1
    line_base = struct.unpack("b", line[offset : offset + 1])[0]
    offset += 1
    line_range = line[offset]
    offset += 1
    opcode_base = line[offset]
    offset += 1
    standard_lengths = list(line[offset : offset + opcode_base - 1])
    offset += opcode_base - 1
    header.update(
        minimum=minimum,
        max_ops=max_ops,
        default_is_stmt=default_is_stmt,
        line_base=line_base,
        line_range=line_range,
        opcode_base=opcode_base,
        standard_lengths=standard_lengths,
        program_start=program_start,
    )
    directories: list[str] = []
    files: list[str] = []
    address_size = int(header["address_size"])
    if version >= 5:
        directory_formats, offset = _line_formats(line, offset)
        count, offset = _uleb(line, offset)
        for _ in range(count):
            entry, offset = _line_entry(line, offset, elf, address_size, offset_size, directory_formats)
            directories.append(entry.get(1, ""))  # DW_LNCT_path
        file_formats, offset = _line_formats(line, offset)
        count, offset = _uleb(line, offset)
        for _ in range(count):
            entry, offset = _line_entry(line, offset, elf, address_size, offset_size, file_formats)
            files.append(entry.get(1, ""))
    else:
        while offset < program_start:
            name = _byte_string(line, offset)
            offset += len(name) + 1
            if not name:
                break
            directories.append(name)
        while offset < program_start:
            name = _byte_string(line, offset)
            offset += len(name.encode()) + 1
            if not name:
                break
            _directory_index, offset = _uleb(line, offset)
            _timestamp, offset = _uleb(line, offset)
            _size, offset = _uleb(line, offset)
            files.append(name)
    header["directories"] = directories
    header["files"] = files
    if program_start < start:
        raise ValueError(f"line header at {start:#x} points backwards")
    return header, program_start


def _line_formats(line: bytes, offset: int) -> tuple[list[tuple[int, int]], int]:
    count = line[offset]
    offset += 1
    formats = []
    for _ in range(count):
        content_type, offset = _uleb(line, offset)
        form, offset = _uleb(line, offset)
        formats.append((content_type, form))
    return formats, offset


def _line_entry(
    line: bytes,
    offset: int,
    elf: Elf,
    address_size: int,
    offset_size: int,
    formats: list[tuple[int, int]],
) -> tuple[dict[int, str], int]:
    entry: dict[int, str] = {}
    for content_type, form in formats:
        value, offset = _form_value(form, line, offset, elf, address_size, offset_size, None)
        entry[content_type] = value if isinstance(value, str) else str(value)
    return entry, offset


def line_table(elf: Elf, offset: int) -> LineTable:
    """Decode the line program at `offset` (the unit's `DW_AT_stmt_list`)."""
    line = elf.sections.get(".debug_line")
    if line is None:
        raise ValueError("no .debug_line section")
    header, offset = _line_header(elf, offset)
    version = int(header["version"])
    minimum = int(header["minimum"])
    max_ops = int(header["max_ops"])
    line_base = int(header["line_base"])
    line_range = int(header["line_range"])
    opcode_base = int(header["opcode_base"])
    standard_lengths = list(header["standard_lengths"])
    address_size = int(header["address_size"])
    files = list(header["files"])
    end = int(header["end"])
    rows: list[LineRow] = []

    address = 0
    op_index = 0
    file_index = 1
    source_line = 1
    column = 0
    is_stmt = bool(header["default_is_stmt"])
    basic_block = False
    prologue_end = False
    epilogue_begin = False
    discriminator = 0
    isa = 0

    def reset() -> None:
        nonlocal address, op_index, file_index, source_line, column, is_stmt
        nonlocal basic_block, prologue_end, epilogue_begin, discriminator, isa
        address = 0
        op_index = 0
        file_index = 1
        source_line = 1
        column = 0
        is_stmt = bool(header["default_is_stmt"])
        basic_block = False
        prologue_end = False
        epilogue_begin = False
        discriminator = 0
        isa = 0

    def emit() -> None:
        nonlocal basic_block, prologue_end, epilogue_begin, discriminator
        if len(rows) >= MAX_LINE_ROWS:
            raise ValueError("line program exceeds the row limit")
        rows.append(
            LineRow(
                address=address,
                file=file_index,
                line=source_line,
                column=column,
                is_stmt=is_stmt,
                end_sequence=False,
                prologue_end=prologue_end,
                epilogue_begin=epilogue_begin,
                discriminator=discriminator,
                isa=isa,
            )
        )
        basic_block = False
        prologue_end = False
        epilogue_begin = False
        discriminator = 0

    def advance(operation_advance: int) -> None:
        nonlocal address, op_index
        if max_ops == 0:
            raise ValueError("maximum_operations_per_instruction is zero")
        total = op_index + operation_advance
        address += minimum * (total // max_ops)
        op_index = total % max_ops

    while offset < end:
        opcode = line[offset]
        offset += 1
        if opcode == 0:
            payload_length, offset = _uleb(line, offset)
            payload_end = offset + payload_length
            if payload_end > end:
                raise ValueError("extended opcode runs past the program")
            sub = line[offset]
            offset += 1
            if sub == 1:  # DW_LNE_end_sequence
                rows.append(
                    LineRow(
                        address=address,
                        file=file_index,
                        line=source_line,
                        column=column,
                        is_stmt=is_stmt,
                        end_sequence=True,
                        prologue_end=prologue_end,
                        epilogue_begin=epilogue_begin,
                        discriminator=discriminator,
                        isa=isa,
                    )
                )
                reset()
            elif sub == 2:  # DW_LNE_set_address
                address = int.from_bytes(line[offset : offset + address_size], elf.order)
                op_index = 0
            elif sub == 3:  # DW_LNE_define_file (DWARF 2..4)
                name = _byte_string(line, offset)
                offset += len(name.encode()) + 1
                files.append(name)
            elif sub == 4:  # DW_LNE_set_discriminator
                discriminator, offset = _uleb(line, offset)
            offset = payload_end
            continue
        if opcode < opcode_base:
            if opcode == 1:  # DW_LNS_copy
                emit()
            elif opcode == 2:  # DW_LNS_advance_pc
                delta, offset = _uleb(line, offset)
                advance(delta)
            elif opcode == 3:  # DW_LNS_advance_line
                delta, offset = _sleb(line, offset)
                source_line += delta
            elif opcode == 4:  # DW_LNS_set_file
                file_index, offset = _uleb(line, offset)
            elif opcode == 5:  # DW_LNS_set_column
                column, offset = _uleb(line, offset)
            elif opcode == 6:  # DW_LNS_negate_stmt
                is_stmt = not is_stmt
            elif opcode == 7:  # DW_LNS_set_basic_block
                basic_block = True
            elif opcode == 8:  # DW_LNS_const_add_pc
                advance((255 - opcode_base) // line_range)
            elif opcode == 9:  # DW_LNS_fixed_advance_pc
                address += int.from_bytes(line[offset : offset + 2], elf.order)
                offset += 2
                op_index = 0
            elif opcode == 10:  # DW_LNS_set_prologue_end
                prologue_end = True
            elif opcode == 11:  # DW_LNS_set_epilogue_begin
                epilogue_begin = True
            elif opcode == 12:  # DW_LNS_set_isa
                isa, offset = _uleb(line, offset)
            else:  # an opcode this gate does not know: skip its arguments
                for _ in range(standard_lengths[opcode - 1]):
                    _value, offset = _uleb(line, offset)
            continue
        adjusted = opcode - opcode_base
        advance(adjusted // line_range)
        source_line += line_base + adjusted % line_range
        emit()
    return LineTable(
        offset=offset,
        version=version,
        address_size=address_size,
        rows=rows,
        files=files,
        directories=list(header["directories"]),
    )


# --- .debug_aranges -----------------------------------------------------------


@dataclass
class ArangeSet:
    info_offset: int
    address_size: int
    tuples: list[tuple[int, int]]


def arange_sets(elf: Elf) -> list[ArangeSet]:
    """Every address-range set in `.debug_aranges`."""
    data = elf.sections.get(".debug_aranges")
    if data is None:
        return []
    out: list[ArangeSet] = []
    offset = 0
    while offset + 4 <= len(data):
        length = int.from_bytes(data[offset : offset + 4], elf.order)
        offset += 4
        if length == 0:
            break
        end = offset + length
        if end > len(data):
            raise ValueError("address range set runs past .debug_aranges")
        version = int.from_bytes(data[offset : offset + 2], elf.order)
        if version != 2:
            raise ValueError(f"address range set version {version}, expected 2")
        offset += 2
        info_offset = int.from_bytes(data[offset : offset + 4], elf.order)
        offset += 4
        address_size = data[offset]
        offset += 2  # address size, segment selector size
        tuple_size = 2 * address_size
        # The first tuple starts at a multiple of the tuple size *counted from
        # the set's own initial length field*, which is four bytes before the
        # body this loop is inside.
        set_start = end - length - 4
        padding = (tuple_size - (offset - set_start) % tuple_size) % tuple_size
        offset += padding
        tuples: list[tuple[int, int]] = []
        while offset + tuple_size <= end:
            address = int.from_bytes(data[offset : offset + address_size], elf.order)
            size = int.from_bytes(
                data[offset + address_size : offset + tuple_size], elf.order
            )
            offset += tuple_size
            if address == 0 and size == 0:
                break
            tuples.append((address, size))
        out.append(ArangeSet(info_offset=info_offset, address_size=address_size, tuples=tuples))
        offset = end
        if len(out) > MAX_UNITS:
            raise ValueError("too many address range sets")
    return out


# --- range lists --------------------------------------------------------------
#
# A unit whose code sits in more than one section cannot be described by one
# low/high pair, so it names a range list instead (DWARF 5 `.debug_rnglists`,
# DWARF 2..4 `.debug_ranges`). That is the shape of every `-ffunction-sections`
# build -- the kernel's and Debian's default -- so a consumer that cannot read it
# cannot debug those programs at all.


@dataclass
class RangeList:
    offset: int
    ranges: list[tuple[int, int]]


def range_lists(elf: Elf, version: int) -> list[RangeList]:
    """Every range list in `.debug_rnglists` (v5) or `.debug_ranges` (v2..4).

    Version 5's section is a table: a twelve-byte header (unit length, version,
    address size, segment selector size, offset-entry count), the offset table,
    then the lists. The older section is a bare sequence of address pairs, where
    `(0, 0)` ends a list and `(-1, base)` rebases the pairs that follow.
    """
    out: list[RangeList] = []
    if version >= 5:
        data = elf.sections.get(".debug_rnglists")
        if data is None:
            return out
        offset = 0
        while offset + 12 <= len(data):
            length = int.from_bytes(data[offset : offset + 4], elf.order)
            if length == 0:
                break
            if length == 0xFFFFFFFF:
                raise ValueError("64-bit .debug_rnglists tables are not emitted by lccc")
            end = offset + 4 + length
            if end > len(data):
                raise ValueError("range list table runs past .debug_rnglists")
            table_version = int.from_bytes(data[offset + 4 : offset + 6], elf.order)
            if table_version != 5:
                raise ValueError(f"range list table version {table_version}, expected 5")
            address_size = data[offset + 6]
            entry_count = int.from_bytes(data[offset + 8 : offset + 12], elf.order)
            cursor = offset + 12 + 4 * entry_count
            if cursor > end:
                raise ValueError("range list offset table runs past its header")
            while cursor < end:
                list_offset = cursor
                ranges: list[tuple[int, int]] = []
                base = 0
                while True:
                    if cursor >= end:
                        raise ValueError("range list without DW_RLE_end_of_list")
                    opcode = data[cursor]
                    cursor += 1
                    if opcode == 0x00:  # DW_RLE_end_of_list
                        break
                    if opcode == 0x07:  # DW_RLE_start_length
                        start = int.from_bytes(data[cursor : cursor + address_size], elf.order)
                        cursor += address_size
                        length, cursor = _uleb(data, cursor)
                        ranges.append((start, start + length))
                    elif opcode == 0x06:  # DW_RLE_base_address
                        base = int.from_bytes(data[cursor : cursor + address_size], elf.order)
                        cursor += address_size
                    elif opcode == 0x05:  # DW_RLE_offset_pair
                        begin, cursor = _uleb(data, cursor)
                        finish, cursor = _uleb(data, cursor)
                        ranges.append((base + begin, base + finish))
                    elif opcode in (0x02, 0x03, 0x04):  # the *_startx family
                        raise ValueError("indexed range list opcodes are not emitted by lccc")
                    else:
                        raise ValueError(f"unknown range list opcode {opcode:#04x}")
                out.append(RangeList(offset=list_offset, ranges=ranges))
            offset = end
        return out
    data = elf.sections.get(".debug_ranges")
    if data is None:
        return out
    address_size = elf.address_size
    pair = 2 * address_size
    maximum = (1 << (8 * address_size)) - 1
    offset = 0
    while offset + pair <= len(data):
        list_offset = offset
        ranges: list[tuple[int, int]] = []
        base = 0
        while offset + pair <= len(data):
            start = int.from_bytes(data[offset : offset + address_size], elf.order)
            finish = int.from_bytes(data[offset + address_size : offset + pair], elf.order)
            offset += pair
            if start == 0 and finish == 0:
                break
            if start == maximum:
                base = finish
                continue
            ranges.append((base + start, base + finish))
        if ranges:
            out.append(RangeList(offset=list_offset, ranges=ranges))
    return out


def ranges_for(elf: Elf, lists: list[RangeList], offset: int) -> list[tuple[int, int]]:
    for candidate in lists:
        if candidate.offset == offset:
            return candidate.ranges
    raise ValueError(f"no range list starts at {offset:#x}")


# --- .eh_frame ----------------------------------------------------------------


@dataclass
class Fde:
    begin: int
    end: int

    @property
    def size(self) -> int:
        return self.end - self.begin


@dataclass
class Cie:
    code_alignment: int
    data_alignment: int
    fde_encoding: int
    has_augmentation_length: bool


def _encoded(
    value_encoding: int, data: bytes, offset: int, section_address: int, address_size: int, order: str
) -> tuple[int, int]:
    """Read one DW_EH_PE-encoded value, applying the pcrel base when present."""
    if value_encoding == 0xFF:
        raise ValueError("DW_EH_PE_omit value was parsed")
    if value_encoding & 0x80:  # DW_EH_PE_indirect: the field holds a pointer
        raise ValueError("indirect pointer encodings are not emitted by lccc")
    field_address = section_address + offset
    format_code = value_encoding & 0x0F
    if format_code == 0x00:  # absptr
        value = int.from_bytes(data[offset : offset + address_size], order)
        offset += address_size
    elif format_code == 0x01:  # uleb128
        value, offset = _uleb(data, offset)
    elif format_code == 0x09:  # sleb128
        value, offset = _sleb(data, offset)
    elif format_code in (0x02, 0x03, 0x04, 0x0A, 0x0B, 0x0C):
        width = {0x02: 2, 0x03: 4, 0x04: 8, 0x0A: 2, 0x0B: 4, 0x0C: 8}[format_code]
        signed = format_code >= 0x0A
        value = int.from_bytes(data[offset : offset + width], order, signed=signed)
        offset += width
    else:
        raise ValueError(f"unsupported DW_EH_PE format {format_code:#x}")
    if value_encoding & 0x10:  # DW_EH_PE_pcrel
        value += field_address
    return value, offset


def fdes(elf: Elf) -> list[Fde]:
    """Every FDE in `.eh_frame`, with absolute addresses and no external help."""
    data = elf.sections.get(".eh_frame")
    if data is None:
        return []
    section_address = elf.addresses.get(".eh_frame", 0)
    cies: dict[int, Cie] = {}
    out: list[Fde] = []
    offset = 0
    while offset + 4 <= len(data):
        entry_start = offset
        length = int.from_bytes(data[offset : offset + 4], elf.order)
        offset += 4
        if length == 0:
            break
        if length == 0xFFFFFFFF:
            raise ValueError("64-bit .eh_frame entries are not emitted by lccc")
        end = offset + length
        if end > len(data):
            raise ValueError(".eh_frame entry runs past the section")
        identity = int.from_bytes(data[offset : offset + 4], elf.order)
        offset += 4
        if identity == 0:  # a CIE
            version = data[offset]
            offset += 1
            augmentation_start = offset
            while data[offset] != 0:
                offset += 1
            augmentation = data[augmentation_start:offset].decode("ascii", "replace")
            offset += 1
            code_alignment, offset = _uleb(data, offset)
            data_alignment, offset = _sleb(data, offset)
            if version == 1:
                offset += 1  # return address register is a single byte
            else:
                _return_register, offset = _uleb(data, offset)
            has_length = augmentation.startswith("z")
            fde_encoding = 0x00
            if has_length:
                augmentation_length, offset = _uleb(data, offset)
                augmentation_end = offset + augmentation_length
                for letter in augmentation[1:]:
                    if letter == "R":
                        fde_encoding = data[offset]
                        offset += 1
                    elif letter == "L":
                        offset += 1
                    elif letter == "P":
                        _personality, offset = _encoded(
                            data[offset], data, offset + 1, section_address, elf.address_size, elf.order
                        )
                    elif letter == "S":
                        continue
                    else:
                        raise ValueError(f"unknown CIE augmentation {letter!r}")
                offset = augmentation_end
            cies[entry_start] = Cie(
                code_alignment=code_alignment,
                data_alignment=data_alignment,
                fde_encoding=fde_encoding,
                has_augmentation_length=has_length,
            )
            offset = end
            continue
        cie_pointer = entry_start + 4 - identity
        cie = cies.get(cie_pointer)
        if cie is None:
            raise ValueError(f"FDE at {entry_start:#x} refers to unknown CIE {cie_pointer:#x}")
        begin, offset = _encoded(
            cie.fde_encoding, data, offset, section_address, elf.address_size, elf.order
        )
        size, offset = _encoded(
            cie.fde_encoding & 0x0F, data, offset, section_address, elf.address_size, elf.order
        )
        out.append(Fde(begin=begin, end=begin + size))
        offset = end
    return out


# --- source model -------------------------------------------------------------


@dataclass
class FunctionSpan:
    name: str
    first_line: int
    last_line: int


def strip_attributes(text: str) -> str:
    """`text` without any `__attribute__((...))` groups.

    An attribute may sit before the return type
    (`__attribute__((noinline)) int f(void) { ... }`) or between the return type
    and the declarator (`int __attribute__((noinline)) f(void) { ... }`), and in
    neither place is it part of the function's name.  The parentheses are counted
    rather than searched, because an unknown attribute's argument list is
    parenthesised too, and an unbalanced group leaves the line untouched rather
    than guessing.
    """
    while (at := text.find("__attribute__")) != -1:
        open_at = text.find("(", at)
        if open_at == -1:
            return text
        depth = 0
        for position in range(open_at, len(text)):
            if text[position] == "(":
                depth += 1
            elif text[position] == ")":
                depth -= 1
                if depth == 0:
                    text = f"{text[:at]} {text[position + 1:]}"
                    break
        else:
            return text
    return text


def function_spans(source: str) -> list[FunctionSpan]:
    """Definitions in a probe: `name` at its declaration line, up to its brace.

    The probes keep every definition at column 0 and every closing brace at
    column 0, which is what makes this scanner enough; it is a property of the
    fixtures this gate wrote, not a C parser.  A definition is a column-0 line
    that spells a name, an argument list and no terminating `;` once any
    leading attribute group is gone -- which covers a typedef'd or attribute-
    bearing return type without listing every type the probes use.
    """
    lines = source.splitlines()
    spans: list[FunctionSpan] = []
    index = 0
    while index < len(lines):
        line = lines[index]
        # Only column-0 lines are definitions (the fixtures' property): an
        # indented `if (x) {` inside a body is never examined, because a
        # definition's body is skipped whole below, but a scanner that reads
        # indentation as a definition would attribute rows to a function named
        # `if`.
        text = strip_attributes(line) if not line[:1].isspace() else ""
        candidate = None
        if "(" in text and not text.rstrip().endswith(";") \
                and not text.startswith(("/*", "*", "#", "}")):
            head = text.split("(")[0].split()
            if head and head[-1].isidentifier():
                candidate = head[-1]
        if candidate is None or text.rstrip().endswith(";"):
            # A prototype has no body: scanning forward for its closing brace
            # would swallow the next definition and mis-attribute its rows.
            index += 1
            continue
        last = index
        while last < len(lines) and lines[last].rstrip() not in ("}", "};"):
            last += 1
        spans.append(FunctionSpan(name=candidate, first_line=index + 1, last_line=last + 1))
        index = last + 1
    return spans


# --- probes -------------------------------------------------------------------

PROBES: dict[str, str] = {
    # Two functions with frames, a call between them, a loop (so several
    # statements share a function), and the internal definitions whose survival
    # differs by optimization tier.
    #
    # `pq_only_sizeof` is the measured exception, and its assertion is that the
    # probe *links*: GCC drops an unused `static inline` function at `-O0`, so a
    # body named only by an unevaluated `sizeof` operand must not drag its call to
    # `pq_never_defined` into the link. A compiler that keeps it fails this
    # build, which is exactly what a regression here looks like.
    "probe": """\
/* No headers: the -m32 lane has no 32-bit C library, and the probe needs
   nothing from one beyond printf. */
int printf(const char *format, ...);
int pq_leaf(int x) { return x + 1; }
static int pq_unused_static(int x) { return x * 2; }
static int pq_used_static(int x) { return x - 1; }
static inline int pq_unused_inline(int x) { return x / 2; }
extern int pq_never_defined(void);
static inline int pq_only_sizeof(void) { return pq_never_defined(); }
static int pq_unused_var = 5;
int pq_global = 3;
struct pq_point { int x; int y; };
typedef struct pq_point pq_point_t;
enum pq_colour { PQ_RED, PQ_GREEN, PQ_BLUE = 300 };
union pq_word { int i; float f; };
struct pq_bits { unsigned int low : 3; unsigned int high : 5; };
int pq_matrix[2][3];
const char *pq_message = "dump";
int (*pq_handler)(int);
/* One object per type, so each type DIE is reachable the way a debugger needs
   it: through an object the program can actually print. */
struct pq_point pq_point_global;
pq_point_t pq_typedef_global;
enum pq_colour pq_colour_global = PQ_BLUE;
union pq_word pq_word_global;
struct pq_bits pq_bits_global;
/* External on purpose: whether an optimizing tier drops an unreachable *static*
   function is a code-generation policy, and this probe asks about DWARF. */
int pq_sum(int count, ...) {
    return count;
}
/* The declaration's own spelling: `pq_index_t` and `pq_point_t *` must survive
   into the parameter DIEs, or `ptype` prints a type the source never wrote.
   `noinline` because this asks about spelling at every tier, not about which
   calls a given tier folds away. */
typedef unsigned long pq_index_t;
typedef int (*pq_fn_t)(int);
pq_fn_t pq_typedef_handler;
pq_point_t pq_point_pair[2];
/* A member's declaration spells its own type too, and a record full of
   `size_t`-style aliases is what a debugger spends its time printing. */
struct pq_holder { pq_index_t count; pq_point_t point; };
struct pq_holder pq_holder_global;
__attribute__((noinline)) int pq_mark(pq_point_t *point, pq_index_t index) {
    return point->x + (int)index;
}
__attribute__((noinline)) pq_index_t pq_next(pq_index_t index) {
    return index + 1;
}
int pq_frame(int n) {
    int total = pq_global;
    for (int i = 0; i < n; i++) {
        total += pq_leaf(i);
    }
    return pq_used_static(total);
}
/* Where each parameter of one function actually lands. The probe stores its own
   parameters' addresses and prints each one's distance from the frame address,
   so the gate compares a DWARF location with the address the running program
   took -- not with another compiler's idea of where it should be. Eleven
   parameters cover both register files (six integer, eight SSE) and the stack,
   and every address is taken, so every parameter is an object in memory at
   every optimization tier. */
static long pq_arg_delta[11];
static long pq_arg_value[11];
static unsigned long pq_frame_record;
__attribute__((noinline)) long pq_args(int a, long b, char c, short d, unsigned e,
                                       float f, double g, void *h, int i, int j, int k) {
    unsigned long frame = (unsigned long)__builtin_frame_address(0);
    pq_frame_record = 2 * sizeof(void *);
    pq_arg_delta[0] = (long)((unsigned long)(void *)&a - frame);
    pq_arg_delta[1] = (long)((unsigned long)(void *)&b - frame);
    pq_arg_delta[2] = (long)((unsigned long)(void *)&c - frame);
    pq_arg_delta[3] = (long)((unsigned long)(void *)&d - frame);
    pq_arg_delta[4] = (long)((unsigned long)(void *)&e - frame);
    pq_arg_delta[5] = (long)((unsigned long)(void *)&f - frame);
    pq_arg_delta[6] = (long)((unsigned long)(void *)&g - frame);
    pq_arg_delta[7] = (long)((unsigned long)(void *)&h - frame);
    pq_arg_delta[8] = (long)((unsigned long)(void *)&i - frame);
    pq_arg_delta[9] = (long)((unsigned long)(void *)&j - frame);
    pq_arg_delta[10] = (long)((unsigned long)(void *)&k - frame);
    /* What the function itself sees, as longs, so a debugger can be asked the
       same question: `print (long)a` must answer what this line stored. */
    pq_arg_value[0] = a;
    pq_arg_value[1] = b;
    pq_arg_value[2] = c;
    pq_arg_value[3] = d;
    pq_arg_value[4] = e;
    pq_arg_value[5] = (long)f;
    pq_arg_value[6] = (long)g;
    pq_arg_value[7] = (long)h;
    pq_arg_value[8] = i;
    pq_arg_value[9] = j;
    pq_arg_value[10] = k;
    return a + b + c + d + (long)e + (long)f + (long)g + (i + j + k) + (h != 0);
}
/* The program's own answers: the checks below compare the DWARF with what this
   code computes, not with a second compiler's opinion about it. */
int main(void) {
    struct pq_point probe_point;
    unsigned long offset_x = (unsigned long)((char *)&probe_point.x - (char *)&probe_point);
    unsigned long offset_y = (unsigned long)((char *)&probe_point.y - (char *)&probe_point);
    printf("sizeof_point %lu\\n", (unsigned long)sizeof(struct pq_point));
    printf("sizeof_int %lu\\n", (unsigned long)sizeof(int));
    printf("sizeof_matrix %lu\\n", (unsigned long)sizeof pq_matrix);
    printf("sizeof_bits %lu\\n", (unsigned long)sizeof(struct pq_bits));
    printf("sizeof_word %lu\\n", (unsigned long)sizeof(union pq_word));
    printf("offsetof_x %lu\\n", offset_x);
    printf("offsetof_y %lu\\n", offset_y);
    printf("addr_global %p\\n", (void *)&pq_global);
    printf("addr_frame %p\\n", (void *)pq_frame);
    printf("value_sum %d\\n", pq_sum(1, 2) + 2);
    printf("sizeof_index %lu\\n", (unsigned long)sizeof(pq_index_t));
    printf("value_next %d\\n", (int)pq_next(2));
    printf("value_mark %d\\n", pq_mark(&pq_point_global, 1));
    printf("value_red %d\\n", PQ_RED);
    printf("value_green %d\\n", PQ_GREEN);
    printf("value_blue %d\\n", PQ_BLUE);
    printf("value_args %ld\\n", pq_args(1, 2, 3, 4, 5, 6.0f, 7.0, &pq_global, 9, 10, 11));
    printf("frame_record %lu\\n", pq_frame_record);
    for (int index = 0; index < 11; index++) {
        printf("arg_delta_%d %ld\\n", index, pq_arg_delta[index]);
    }
    for (int index = 0; index < 11; index++) {
        printf("arg_value_%d %ld\\n", index, pq_arg_value[index]);
    }
    /* `sizeof` of an inline function keeps its DIE reachable without calling it,
       and the exit status stays zero so the gate can run this binary. */
    (void)sizeof(pq_only_sizeof());
    return pq_frame(4) - 12;
}
""",
    # Two units, so `.debug_aranges` must be per unit and each set must name its
    # own `.debug_info` offset.
    "probe_a": """\
int pq_b_caller(void);
int pq_a_work(int x) {
    int doubled = x * 2;
    return doubled + pq_b_caller();
}
int main(void) { return pq_a_work(3) - 9; }
""",
    "probe_b": """\
int pq_b_caller(void) { return 1; }
""",
    # A declaration is described only when the unit's code refers to it, and a
    # type reachable only from an unreferenced declaration is not described at
    # all. Compiled with `-c` (one of the two names has no definition anywhere),
    # which is why the reference sits in a function.
    "probe_decl": """\
extern int pq_decl_used;
extern int pq_decl_unused;
struct pq_decl_rec { int a; int b; };
extern struct pq_decl_rec pq_decl_rec_unused;
int pq_decl_touch(void) { return pq_decl_used; }
""",
    # A unit with no code at all: GCC still emits a unit and a variable DIE.
    "probe_data": """\
int pq_only_data = 42;
static const char pq_only_text[] = "only data";
""",
}


def write_probes(directory: Path) -> dict[str, Path]:
    out = {}
    for name, text in PROBES.items():
        path = directory / f"{name}.c"
        path.write_text(text)
        out[name] = path
    return out


# --- gate ---------------------------------------------------------------------


@dataclass
class Failure:
    check: str
    detail: str

    def __str__(self) -> str:
        return f"[{self.check}] {self.detail}"


@dataclass
class Report:
    failures: list[Failure] = field(default_factory=list)
    known: dict[str, str] = field(default_factory=dict)
    healed: list[str] = field(default_factory=list)
    measurements: dict[str, object] = field(default_factory=dict)
    objects: int = 0

    def fail(self, check: str, detail: str) -> None:
        self.failures.append(Failure(check=check, detail=detail))

    def gap(self, key: str, detail: str) -> None:
        """Record a known gap; never a failure, but reported with its reason."""
        if key in self.known:
            return
        reason = KNOWN_GAPS.get(key, "")
        self.known[key] = f"{detail} -- {reason}" if reason else detail


def build(report: Report, command: list[str], flags: list[str], probe: list[Path], output: Path,
          cwd: Path, link: bool = True) -> bool:
    """Compile (and by default link) one probe; a failure here is a gate failure."""
    argv = [*command, *flags, "-o", str(output), *[str(path) for path in probe]]
    if not link:
        argv.insert(len(command), "-c")
    proc = subprocess.run(
        argv, cwd=cwd, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
        timeout=180,
    )
    report.objects += 1
    if proc.returncode != 0:
        report.fail(
            "build",
            f"{' '.join(flags)} on {[path.name for path in probe]} exited "
            f"{proc.returncode}: {proc.stderr.strip()[:400]}",
        )
        return False
    return True


def check_unit_structure(report: Report, elf: Elf, version: int, label: str) -> list[Unit]:
    """Headers parse, the version is the one that was asked for, names are real."""
    try:
        parsed = units(elf)
    except ValueError as error:
        report.fail("unit_header", f"{label}: {error}")
        return []
    if not parsed:
        report.fail("unit_header", f"{label}: no compile unit in .debug_info")
        return []
    for unit in parsed:
        root = unit.root()
        if root is None or root.tag != DW_TAG_COMPILE_UNIT:
            report.fail("unit_header", f"{label}: unit at {unit.offset:#x} has no CU DIE")
            continue
        if unit.version != version:
            report.fail(
                "unit_version",
                f"{label}: unit at {unit.offset:#x} says version {unit.version}, "
                f"object was asked for {version}",
            )
        for attribute, named in (
            (DW_AT_NAME, "DW_AT_name"),
            (DW_AT_COMP_DIR, "DW_AT_comp_dir"),
            (DW_AT_PRODUCER, "DW_AT_producer"),
        ):
            if not root.attrs.get(attribute):
                report.fail("unit_attribute", f"{label}: CU at {unit.offset:#x} has no {named}")
        comp_dir = root.attrs.get(DW_AT_COMP_DIR)
        if isinstance(comp_dir, str) and comp_dir and not comp_dir.startswith("/"):
            report.fail(
                "unit_attribute",
                f"{label}: DW_AT_comp_dir {comp_dir!r} is not absolute, so a consumer "
                "cannot resolve a relative file name",
            )
        if elf.functions() and DW_AT_STMT_LIST not in root.attrs:
            report.fail(
                "unit_attribute",
                f"{label}: CU at {unit.offset:#x} has functions but no DW_AT_stmt_list, "
                "so no line table can be found",
            )
    return parsed


def check_abbrev_usage(report: Report, elf: Elf, parsed: list[Unit], label: str) -> None:
    """Every declared abbreviation must describe at least one DIE.

    An abbreviation is a record in `.debug_abbrev` and a parse for every consumer
    that walks the table, so one that no DIE uses is paid for by every object and
    read by nobody. The writer therefore declares a shape when the DIE that takes
    it is written, and this asserts the consequence: the table holds exactly the
    shapes the unit uses.

    It used to hold more. The subprogram shapes were declared as a cross product
    of declaration kind, child list and linkage -- eight, of which a `-O0` unit
    used three -- and the parameter shapes were chosen from a static reading of
    the unit, which still carried the shape for an unnamed parameter no function
    had. Measured on the gate's own probe: `.debug_abbrev` 231 -> 174 bytes at
    `-O0`, the same at `-O2` where no parameter is placed at all.
    """
    for unit in parsed:
        used = {die.code for die in unit.dies}
        unused = [code for code in unit.declared if code not in used]
        if unused:
            report.fail(
                "abbrev_usage",
                f"{label}: unit at {unit.offset:#x} declares abbreviation(s) {unused} that no "
                f"DIE uses ({len(unused)} of {len(unit.declared)} declared)",
            )


def is_dropped_die(low: int, high: int, name: str, live: set[str]) -> bool:
    """Whether a DIE describes code a `--gc-sections` link removed.

    A dropped section's address relocation resolves to zero, so the DIE survives
    as `[0, size)`. That is only the linker's doing when the name is no longer a
    symbol: a *live* function at address zero is a defect, and this predicate
    deliberately says so.
    """
    return low == 0 and high > 0 and name not in live


def check_subprograms(
    report: Report, elf: Elf, parsed: list[Unit], label: str, spans: list[FunctionSpan],
    prefix: str = "pq_"
) -> dict[str, tuple[int, int]]:
    """Every defined function is a DIE whose range is inside a code section."""
    text = {
        name: payload
        for name, payload in elf.sections.items()
        if name == ".text" or name.startswith(".text.")
    }
    # A link with `--gc-sections` (the kernel's and Debian's default) drops a
    # function's section but leaves the DIE that described it, whose address
    # relocation then resolves to zero. Both compilers leave those DIEs behind, so
    # a zero address on a name that is *not* a live symbol is the linker's doing,
    # not a range defect -- while a zero address on a live symbol still is one.
    live = {symbol.name for symbol in elf.functions()}
    dropped = 0
    stale_spans: list[tuple[int, int]] = []
    ranges: dict[str, tuple[int, int]] = {}
    for unit in parsed:
        for name, low, high, die in subprogram_ranges(elf, unit, unit.version):
            # Every DIE range is recorded, because the line-table checks need to
            # know which function a row belongs to -- including `main` and any
            # function the probe did not name with the prefix.  The *assertions*
            # below are scoped to the probe's own names.
            probe_owned = name.startswith(prefix)
            if is_dropped_die(low, high, name, live):
                dropped += 1
                stale_spans.append((low, high))
                continue
            if name in ranges:
                report.fail("subprogram_duplicate", f"{label}: {name} has two subprogram DIEs")
                continue
            if high <= low:
                if probe_owned:
                    report.fail(
                        "subprogram_range",
                        f"{label}: {name} states [{low:#x}, {high:#x}), which covers nothing",
                    )
                continue
            inside = any(
                elf.addresses.get(section, 0) <= low
                and high <= elf.addresses.get(section, 0) + len(payload)
                for section, payload in text.items()
            )
            if not inside and probe_owned:
                report.fail(
                    "subprogram_range",
                    f"{label}: {name} states [{low:#x}, {high:#x}), which is outside every "
                    "code section",
                )
            for attribute, named in ((DW_AT_DECL_FILE, "decl_file"), (DW_AT_DECL_LINE, "decl_line")):
                if resolved(die, unit, attribute) is None and probe_owned:
                    report.fail(
                        "subprogram_attribute",
                        f"{label}: {name} has no DW_AT_{named}, here or on its abstract origin",
                    )
            if resolved(die, unit, DW_AT_FRAME_BASE) is None and unit.version >= 3 and probe_owned:
                report.fail(
                    "subprogram_attribute",
                    f"{label}: {name} has no DW_AT_frame_base, so a consumer cannot compute a "
                    "local's address from the frame",
                )
            ranges[name] = (low, high)
    if dropped:
        report.measurements.setdefault("dropped_dies", {})[label] = dropped
        report.measurements.setdefault("dropped_spans", {})[label] = sorted(stale_spans)
    symbols = {symbol.name: symbol for symbol in elf.functions()}
    for name, symbol in symbols.items():
        if name not in ranges and not name.startswith(prefix):
            continue  # a CRT or library function this probe did not write
        if name not in ranges:
            report.fail(
                "subprogram_missing",
                f"{label}: {name} is a defined function in .symtab but has no subprogram DIE",
            )
            continue
        low, high = ranges[name]
        if high - low != symbol.size:
            report.fail(
                "subprogram_range",
                f"{label}: {name} states {high - low} bytes of code, the symbol says "
                f"{symbol.size}",
            )
    expected = {span.name for span in spans if span.name.startswith(prefix)}
    for name in sorted(expected - set(ranges)):
        report.fail(
            "subprogram_missing", f"{label}: the source defines {name} but no DIE describes it"
        )
    report.measurements.setdefault("functions", {})[label] = len(ranges)
    return ranges


def check_lines(
    report: Report, elf: Elf, parsed: list[Unit], ranges: dict[str, tuple[int, int]],
    label: str, spans: list[FunctionSpan], inlining: bool = False
) -> None:
    """Line rows are ordered, attributed to the right function, and resolvable.

    With `inlining` set -- the optimizing tiers, where a callee is folded into
    its caller -- a row inside function F may legitimately come from any source
    function, because that is what a line program says about inlined code: the
    caller's range, the callee's line.  A consumer recovers the mapping from
    `DW_TAG_inlined_subroutine`, which this gate lists as a known gap, so the
    weaker assertion here is "the line belongs to some function of the probe"
    rather than "it belongs to the owner of the address".
    """
    by_line = {span.name: span for span in spans}
    every_span = [(span.first_line, span.last_line) for span in spans]
    for unit in parsed:
        offset = unit.root().attrs.get(DW_AT_STMT_LIST) if unit.root() else None
        if not isinstance(offset, int):
            continue
        try:
            table = line_table(elf, offset)
        except ValueError as error:
            report.fail("line_program", f"{label}: {error}")
            continue
        if not table.rows:
            report.fail("line_program", f"{label}: the line program at {offset:#x} has no rows")
            continue
        if table.version != unit.version and not (table.version == 2 and unit.version >= 2):
            report.fail(
                "line_version",
                f"{label}: unit version {unit.version} references a version {table.version} "
                "line program; a consumer keys the opcode table on the unit's version",
            )
        if not table.files:
            report.fail("line_program", f"{label}: the line program has no file table")
        sequences = 0
        previous = None
        listed = 0
        for row, width in table.spans():
            if row.end_sequence:
                sequences += 1
                previous = None
                continue
            if previous is not None and row.address < previous:
                report.fail(
                    "line_order",
                    f"{label}: a row at {row.address:#x} follows {previous:#x}; a consumer "
                    "treats rows as ascending",
                )
            previous = row.address
            listed += 1
            # DWARF 2..4 index the file table from 1 (0 means "no file"), DWARF
            # 5 from 0; a consumer keys that on the unit's version, so a row the
            # one convention calls valid and the other does not is a real defect.
            if table.version >= 5:
                in_range = 0 <= row.file < len(table.files)
            else:
                in_range = 1 <= row.file <= len(table.files)
            if not in_range:
                report.fail(
                    "line_file",
                    f"{label}: row at {row.address:#x} names file {row.file}, the version "
                    f"{table.version} table has {len(table.files)} entries",
                )
            if row.line <= 0:
                report.fail("line_number", f"{label}: row at {row.address:#x} has line {row.line}")
            # A row may sit exactly on a function's exclusive end: that is the
            # last row of the function it closes, not a row outside everything.
            owner = next(
                (name for name, (low, high) in ranges.items() if low <= row.address < high),
                None,
            ) or next(
                (name for name, (low, high) in ranges.items() if row.address == high),
                None,
            )
            if owner is None and any(
                low <= row.address < high
                for low, high in report.measurements.get("dropped_spans", {}).get(label, ())
            ):
                stale_rows = report.measurements.setdefault("stale_rows", {})
                stale_rows[label] = stale_rows.get(label, 0) + 1
                owner = None
                continue
            if owner is None:
                if width > 0:
                    report.fail(
                        "line_outside_function",
                        f"{label}: a {width}-byte row at {row.address:#x} is inside no "
                        "subprogram's range",
                    )
                continue
            if inlining:
                if not any(first <= row.line <= last for first, last in every_span):
                    report.fail(
                        "line_attribution",
                        f"{label}: the row at {row.address:#x} claims line {row.line}, which is "
                        "outside every function of the probe",
                    )
                continue
            span = by_line.get(owner)
            if span is not None and not (span.first_line <= row.line <= span.last_line):
                report.fail(
                    "line_attribution",
                    f"{label}: {owner} (source lines {span.first_line}..{span.last_line}) "
                    f"claims line {row.line} at {row.address:#x}",
                )
        if sequences == 0:
            report.fail(
                "line_program", f"{label}: {listed} rows and no DW_LNE_end_sequence"
            )
        # One sequence for the whole unit is legal DWARF: a consumer slices by
        # address, not by sequence.  GCC emits one per function, which lets a
        # consumer stop decoding early, so the ratio is recorded as a measurement
        # (docs/DWARF-CAMPAIGN.md, work item D5) rather than failing the gate.
        report.measurements.setdefault("sequences_vs_functions", {})[label] = [
            sequences,
            len(ranges),
        ]
        statement_rows = sum(1 for row in table.rows if row.is_stmt and not row.end_sequence)
        if statement_rows == 0:
            report.fail(
                "line_is_stmt",
                f"{label}: no row carries is_stmt, so a debugger cannot step by statement",
            )
        report.measurements.setdefault("line_rows", {})[label] = listed
        report.measurements.setdefault("line_sequences", {})[label] = sequences


def check_aranges(report: Report, elf: Elf, parsed: list[Unit], ranges: dict[str, tuple[int, int]],
                  label: str) -> None:
    """One set per unit, naming that unit, and between them covering the code."""
    if not parsed:
        return
    try:
        sets = arange_sets(elf)
    except ValueError as error:
        report.fail("aranges", f"{label}: {error}")
        return
    if not sets:
        report.fail(
            "aranges",
            f"{label}: no .debug_aranges, so mapping an address costs a walk over every unit",
        )
        return
    for unit in parsed:
        if not any(entry.info_offset == unit.offset for entry in sets):
            report.fail(
                "aranges",
                f"{label}: no set names the unit at {unit.offset:#x}; a consumer that trusts "
                "the table misses the unit entirely",
            )
    covered: dict[int, int] = {}
    for entry in sets:
        if not entry.tuples:
            report.fail(
                "aranges", f"{label}: the set for unit {entry.info_offset:#x} lists no range"
            )
        for address, size in entry.tuples:
            if size == 0:
                report.fail(
                    "aranges",
                    f"{label}: the set for unit {entry.info_offset:#x} lists an empty range at "
                    f"{address:#x}",
                )
            for existing in range(address, address + max(size, 1), max(size, 1)):
                if existing in covered and covered[existing] != entry.info_offset:
                    report.fail(
                        "aranges_overlap",
                        f"{label}: {existing:#x} is claimed by both unit "
                        f"{covered[existing]:#x} and unit {entry.info_offset:#x}",
                    )
                covered[existing] = entry.info_offset
    for name, (low, high) in ranges.items():
        if not any(
            address <= low and high <= address + size
            for entry in sets
            for address, size in entry.tuples
        ):
            report.fail(
                "aranges",
                f"{label}: no set covers {name} at [{low:#x}, {high:#x})",
            )


def check_range_lists(
    report: Report, elf: Elf, parsed: list[Unit], ranges: dict[str, tuple[int, int]], label: str
) -> None:
    """A multi-section unit describes itself with a range list a consumer can read."""
    if not parsed:
        return
    try:
        lists = range_lists(elf, parsed[0].version)
    except ValueError as error:
        report.fail("range_list", f"{label}: {error}")
        return
    for unit in parsed:
        root = unit.root()
        if root is None:
            continue
        offset = root.attrs.get(DW_AT_RANGES)
        if not isinstance(offset, int):
            continue
        try:
            listed = ranges_for(elf, lists, offset)
        except ValueError as error:
            report.fail("range_list", f"{label}: unit at {unit.offset:#x}: {error}")
            continue
        stale = [entry for entry in listed if entry[0] == 0]
        listed = [entry for entry in listed if entry[0] != 0]
        if stale:
            report.measurements.setdefault("stale_ranges", {})[label] = len(stale)
        if not listed:
            report.fail(
                "range_list",
                f"{label}: unit at {unit.offset:#x} names an empty range list"
                if not stale
                else f"{label}: every range in the unit's list is stale",
            )
            continue
        for start, end in listed:
            if end <= start:
                report.fail(
                    "range_list",
                    f"{label}: unit at {unit.offset:#x} lists [{start:#x}, {end:#x}), which "
                    "covers nothing",
                )
        ordered = sorted(listed)
        for (_, previous_end), (next_start, _) in zip(ordered, ordered[1:]):
            if next_start < previous_end:
                report.fail(
                    "range_list",
                    f"{label}: unit at {unit.offset:#x} lists overlapping ranges at "
                    f"{next_start:#x}",
                )
        low = root.attrs.get(DW_AT_LOW_PC)
        if isinstance(low, int) and low not in (0, ordered[0][0]):
            report.fail(
                "range_list",
                f"{label}: DW_AT_low_pc {low:#x} is neither 0 nor the first range start "
                f"{ordered[0][0]:#x}, so a consumer that offsets ranges by it lands in the "
                "wrong place",
            )
    report.measurements.setdefault("range_lists", {})[label] = len(lists)


def check_eh_frame(report: Report, elf: Elf, ranges: dict[str, tuple[int, int]], label: str) -> None:
    """Every function with code has an FDE, so a consumer can unwind out of it."""
    try:
        entries = fdes(elf)
    except ValueError as error:
        report.fail("eh_frame", f"{label}: {error}")
        return
    if not entries:
        report.fail("eh_frame", f"{label}: no FDE covers any function")
        return
    for name, (low, high) in sorted(ranges.items()):
        if low == 0 and high > 0:  # see is_dropped_die
            continue
        if not any(entry.begin <= low and high <= entry.end for entry in entries):
            report.fail(
                "eh_frame",
                f"{label}: no FDE covers {name} at [{low:#x}, {high:#x}); unwinding out of it "
                "would rely on the frame pointer alone",
            )
    for entry in entries:
        if entry.size <= 0:
            report.fail("eh_frame", f"{label}: an FDE covers {entry.size} bytes at {entry.begin:#x}")
    report.measurements.setdefault("fdes", {})[label] = len(entries)


def check_object_unit(report: Report, elf: Elf, version: int, label: str, prefix: str = "pq_") -> None:
    """The address-free half of the checks, for an object that cannot be linked.

    A 32-bit target on a host without multilib still compiles to an object, and
    an object is enough to check what a consumer reads *by name*: unit headers,
    subprogram DIEs against the symbol table, declaration attributes. The
    address-based checks (line rows, address map, unwind tables) need linked
    addresses and are covered by the 64-bit matrices.
    """
    try:
        parsed = units(elf)
    except ValueError as error:
        report.fail("unit_header", f"{label}: {error}")
        return
    if not parsed:
        report.fail("unit_header", f"{label}: no compile unit in .debug_info")
        return
    for unit in parsed:
        root = unit.root()
        if root is None or root.tag != DW_TAG_COMPILE_UNIT:
            report.fail("unit_header", f"{label}: unit at {unit.offset:#x} has no CU DIE")
            continue
        if unit.version != version:
            report.fail(
                "unit_version",
                f"{label}: unit at {unit.offset:#x} says version {unit.version}, "
                f"object was asked for {version}",
            )
        if not root.attrs.get(DW_AT_PRODUCER):
            report.fail("unit_attribute", f"{label}: CU at {unit.offset:#x} has no DW_AT_producer")
    described: dict[str, Die] = {}
    for unit in parsed:
        for die in unit.by_tag(DW_TAG_SUBPROGRAM):
            name = die_name(die, unit)
            if name.startswith(prefix):
                described.setdefault(name, die)
                for attribute, named in (
                    (DW_AT_DECL_FILE, "decl_file"),
                    (DW_AT_DECL_LINE, "decl_line"),
                ):
                    if resolved(die, unit, attribute) is None:
                        report.fail(
                            "subprogram_attribute", f"{label}: {name} has no DW_AT_{named}"
                        )
    for symbol in elf.functions(prefix):
        if symbol.name not in described:
            report.fail(
                "subprogram_missing",
                f"{label}: {symbol.name} is a defined function in .symtab but has no "
                "subprogram DIE",
            )


def check_static_policy(report: Report, elf: Elf, label: str, tier: str) -> None:
    """`-O0` keeps internal definitions, `-O1` and above drop the unreachable ones.

    `-O0` is a correctness boundary, not a size preference: an inline-asm `call`
    names a static function no C expression references, so a compiler that
    pruned here would fail a link GCC completes (measured on 16.2.0).
    """
    symbols = {symbol.name for symbol in elf.symbols if symbol.defined}
    dies = set()
    try:
        for unit in units(elf):
            dies.update(die_name(die, unit) for die in unit.by_tag(DW_TAG_SUBPROGRAM))
            dies.update(die_name(die, unit) for die in unit.by_tag(DW_TAG_VARIABLE))
    except ValueError as error:
        report.fail("static_policy", f"{label}: {error}")
        return
    if tier == "O0":
        for name in ("pq_unused_static", "pq_used_static"):
            if name not in symbols:
                report.fail(
                    "static_policy",
                    f"{label}: {name} is defined in the source but absent from .symtab; GCC "
                    "16.2 keeps an unreferenced static function at -O0 and drops it at -O1",
                )
            if name not in dies:
                report.fail(
                    "static_policy",
                    f"{label}: {name} has no subprogram DIE at -O0, so a debugger cannot "
                    "set a breakpoint in a function the user wrote",
                )
        if "pq_unused_var" not in symbols:
            report.fail(
                "static_policy",
                f"{label}: the unreferenced static object pq_unused_var is absent at -O0; "
                "GCC 16.2 emits it there",
            )
        if "pq_unused_inline" in symbols:
            report.fail(
                "static_policy",
                f"{label}: pq_unused_inline is an unused static inline function and GCC 16.2 "
                "drops it at -O0; only -fkeep-inline-functions keeps it",
            )
    else:
        for name in ("pq_unused_static", "pq_unused_var", "pq_unused_inline"):
            if name in symbols:
                report.fail(
                    "static_policy",
                    f"{label}: {name} is unreachable and internal but present at -{tier}; GCC "
                    "16.2 keeps it only at -O0",
                )


def check_debug_payload(report: Report, elf: Elf, label: str, optimizing: bool) -> None:
    """The DIEs a debugger needs to name a value; absent ones are known gaps."""
    _ = optimizing  # inlining is only meaningful where a tier inlines; see below
    try:
        parsed = units(elf)
    except ValueError as error:
        report.fail("debug_payload", f"{label}: {error}")
        return
    tags: dict[int, int] = {}
    for unit in parsed:
        for die in unit.dies:
            tags[die.tag] = tags.get(die.tag, 0) + 1
    if not tags.get(DW_TAG_VARIABLE):
        report.gap("variables", f"{label}: 0 DW_TAG_variable DIEs")
    type_tags = (
        DW_TAG_BASE_TYPE,
        DW_TAG_POINTER_TYPE,
        DW_TAG_CONST_TYPE,
        DW_TAG_VOLATILE_TYPE,
        DW_TAG_RESTRICT_TYPE,
        DW_TAG_TYPEDEF,
        DW_TAG_STRUCTURE_TYPE,
        DW_TAG_UNION_TYPE,
        DW_TAG_ENUMERATION_TYPE,
        DW_TAG_ENUMERATOR,
        DW_TAG_ARRAY_TYPE,
        DW_TAG_SUBRANGE_TYPE,
        DW_TAG_SUBROUTINE_TYPE,
        DW_TAG_MEMBER,
    )
    present = [f"0x{tag:02x}={tags[tag]}" for tag in type_tags if tags.get(tag)]
    if not present:
        report.gap("types", f"{label}: no type DIEs")
    # Only a tier that inlines has something to describe: at `-O0` GCC emits no
    # inlined-subroutine DIE either, so the absence is not a gap there.
    if optimizing and not tags.get(DW_TAG_INLINED_SUBROUTINE):
        report.gap(
            "inlining",
            f"{label}: no DW_TAG_inlined_subroutine DIEs although this tier folds callees "
            "into callers, so a frame cannot say which function a line came from",
        )
    located = sum(
        1
        for unit in parsed
        for die in unit.dies
        if die.tag == DW_TAG_VARIABLE and DW_AT_LOCATION in die.attrs
    )
    if not located:
        report.gap("locations", f"{label}: no variable DIE carries DW_AT_location")
    report.measurements.setdefault("tag_counts", {})[label] = {
        f"{tag:#04x}": count for tag, count in sorted(tags.items())
    }

# --- types, locations and the program's own answers --------------------------
#
# The checks below do not ask "is this byte the same as GCC's". They ask whether
# the DWARF is *true*: whether a struct DIE says the size the running program
# says, whether a member sits at the offset `offsetof` reports, whether an
# enumerator has the value the compiler folded into the instruction stream, and
# whether the address in a variable's `DW_AT_location` is the address the program
# prints for that object. A wrong size is a wrong `sizeof` in the debugger and a
# wrong member offset is a debugger reading the neighbouring field; a gate that
# only compares shapes against a second compiler notices neither.

DW_ATE_SIGNED = 0x05
DW_ATE_SIGNED_CHAR = 0x06
DW_ATE_UNSIGNED = 0x07
DW_ATE_UNSIGNED_CHAR = 0x08

UNSIGNED_ENCODINGS = (DW_ATE_UNSIGNED, DW_ATE_UNSIGNED_CHAR)


def program_facts(report: Report, path: Path, label: str) -> dict[str, int] | None:
    """`key value` lines the probe prints about itself, or `None` if it did not run.

    The probe prints addresses, `sizeof`s, `offsetof`s and enumerator values from
    its own compiled code, so every DWARF claim below is checked against the
    program rather than against a second opinion about the program.
    """
    try:
        proc = subprocess.run(
            [str(path)], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            check=False, timeout=120,
        )
    except OSError as error:
        report.fail("program_facts", f"{label}: cannot run the probe: {error}")
        return None
    if proc.returncode != 0:
        report.fail(
            "program_facts",
            f"{label}: the probe exits {proc.returncode}: {proc.stderr.strip()[:200]}",
        )
        return None
    facts: dict[str, int] = {}
    for line in proc.stdout.splitlines():
        key, _, value = line.partition(" ")
        try:
            facts[key] = int(value, 0)
        except ValueError:
            report.fail("program_facts", f"{label}: unreadable probe line {line!r}")
            return None
    return facts


def reference_target(die: Die, unit: Unit, attribute: int) -> Die | None:
    """The DIE an attribute refers to, whichever reference form it used."""
    value = die.attrs.get(attribute)
    if not isinstance(value, int):
        return None
    form = die.forms.get(attribute)
    if form == DW_FORM_REF_ADDR:
        return unit.die_at(value)
    if form in DW_FORM_UNIT_REFS:
        return unit.die_at(unit.offset + value)
    return None


def children(unit: Unit, parent: Die) -> list[Die]:
    """The DIEs directly below `parent` in the flattened tree."""
    out: list[Die] = []
    for die in unit.dies:
        if die.depth <= parent.depth:
            if die.offset > parent.offset and out:
                break
            continue
        if die.depth == parent.depth + 1 and die.offset > parent.offset:
            out.append(die)
    return out


def resolved_type(die: Die, unit: Unit) -> Die | None:
    """The type a DIE names, following `DW_AT_abstract_origin` first.

    An optimizing compiler moves a parameter's type onto the abstract DIE and
    leaves the concrete one with nothing but a reference, so reading only the
    concrete DIE would report a type-less parameter that a consumer resolves.
    """
    for candidate in origin_chain(die, unit):
        if DW_AT_TYPE in candidate.attrs:
            return reference_target(candidate, unit, DW_AT_TYPE)
    return None


def die_by_tag_name(unit: Unit, tag: int, name: str) -> Die | None:
    for die in unit.by_tag(tag):
        if die.name() == name:
            return die
    return None


def type_of(die: Die, unit: Unit) -> Die | None:
    return reference_target(die, unit, DW_AT_TYPE)


def type_size(die: Die, unit: Unit, depth: int = 0) -> int | None:
    """`DW_AT_byte_size` of a DIE or of the type it names, when stated."""
    if depth > 8:
        return None
    size = die.attrs.get(DW_AT_BYTE_SIZE)
    if isinstance(size, int):
        return size
    target = type_of(die, unit)
    return type_size(target, unit, depth + 1) if target is not None else None


def peel_types(die: Die | None, unit: Unit, limit: int = 12) -> list[Die]:
    """`die` and every type reached from it through `DW_AT_type`."""
    chain: list[Die] = []
    while die is not None and len(chain) < limit and die not in chain:
        chain.append(die)
        die = type_of(die, unit)
    return chain


#: `DW_OP_plus_uconst`: how DWARF 2 and 3 spell a constant member offset, and
#: what both compilers actually emit there. A block is the only form those
#: versions allow for a member of a non-contiguous (bit-field) structure.
DW_OP_PLUS_UCONST = 0x23

DW_FORM_BLOCKS = (0x09, 0x0A, 0x03, 0x04)  # block, block1, block2, block4


def member_offset(member: Die) -> int | None:
    """`DW_AT_data_member_location` as a byte offset, in either encoding.

    DWARF 2 and 3 encode it as a location expression (`DW_OP_plus_uconst`);
    DWARF 4 and 5 allow the constant itself. Both spellings are in the wild --
    GCC 16.2 picks the block for 2 and 3 and the constant from 4 on -- and a
    consumer accepts either, so the gate reads either.
    """
    value = member.attrs.get(DW_AT_DATA_MEMBER_LOCATION)
    if isinstance(value, int):
        return value
    if not isinstance(value, bytes) or not value or value[0] != DW_OP_PLUS_UCONST:
        return None
    offset = 0
    shift = 0
    for byte in value[1:]:
        offset |= (byte & 0x7F) << shift
        shift += 7
        if not byte & 0x80:
            break
    return offset


def member_bit_placement(member: Die, version: int) -> tuple[int, int] | None:
    """`(least significant bit, bit size)` of a member, in any encoding.

    DWARF 5 states one absolute `DW_AT_data_bit_offset`, counted from the least
    significant bit of the containing structure. DWARF 4 allows the same
    attribute, or `DW_AT_data_member_location` plus `DW_AT_bit_offset`; 2 and 3
    only the latter, and there `DW_AT_bit_offset` counts from the *most*
    significant bit of the byte_size-wide storage unit. All three are reduced to
    the LSB-relative placement the program's own layout implies.
    """
    size = member.attrs.get(DW_AT_BIT_SIZE)
    if not isinstance(size, int):
        return None
    absolute = member.attrs.get(DW_AT_DATA_BIT_OFFSET)
    if version >= 4 and isinstance(absolute, int):
        return absolute, size
    offset = member.attrs.get(DW_AT_BIT_OFFSET)
    if not isinstance(offset, int):
        return None
    unit = member.attrs.get(DW_AT_BYTE_SIZE)
    if not isinstance(unit, int):
        return None
    container = member_offset(member) or 0
    return container * 8 + unit * 8 - offset - size, size


def check_type_dies(
    report: Report, elf: Elf, parsed: list[Unit], label: str, facts: dict[str, int]
) -> None:
    """Every type a debugger needs must agree with what the program computed."""
    units_with_types = 0
    dangling = 0
    for unit in parsed:
        for die in unit.dies:
            if DW_AT_TYPE not in die.attrs:
                continue
            units_with_types += 1
            if reference_target(die, unit, DW_AT_TYPE) is None:
                dangling += 1
    if units_with_types == 0:
        report.fail("types", f"{label}: no DIE carries DW_AT_type")
    if dangling:
        report.fail(
            "types",
            f"{label}: {dangling} of {units_with_types} DW_AT_type references point at no DIE",
        )

    unit = parsed[0]
    int_type = die_by_tag_name(unit, DW_TAG_BASE_TYPE, "int")
    if int_type is None:
        report.fail("types", f"{label}: no DW_TAG_base_type for int")
    else:
        size = int_type.attrs.get(DW_AT_BYTE_SIZE)
        encoding = int_type.attrs.get(DW_AT_ENCODING)
        if size != facts["sizeof_int"]:
            report.fail(
                "types",
                f"{label}: int is {size} bytes in DWARF and {facts['sizeof_int']} in the program",
            )
        if encoding not in (DW_ATE_SIGNED, DW_ATE_SIGNED_CHAR):
            report.fail(
                "types", f"{label}: int has encoding {encoding!r}, which is not signed"
            )

    def object_type(name: str) -> Die | None:
        """The type DIE of the file-scope object called `name`."""
        variable = unit_root_variable(unit, name)
        if variable is None:
            report.fail("types", f"{label}: no DW_TAG_variable for {name}")
            return None
        target = type_of(variable, unit)
        if target is None:
            report.fail("types", f"{label}: {name} names no type")
        return target

    point = object_type("pq_point_global")
    if point is None:
        pass
    elif point.tag != DW_TAG_STRUCTURE_TYPE or point.name() != "pq_point":
        report.fail(
            "types",
            f"{label}: pq_point_global is typed {point.name()!r} (tag {point.tag:#04x}), not "
            "the pq_point structure",
        )
    else:
        if type_size(point, unit) != facts["sizeof_point"]:
            report.fail(
                "types",
                f"{label}: struct pq_point is {type_size(point, unit)} bytes in DWARF and "
                f"{facts['sizeof_point']} in the program",
            )
        members = {member.name(): member for member in children(unit, point)
                   if member.tag == DW_TAG_MEMBER}
        for name, key in (("x", "offsetof_x"), ("y", "offsetof_y")):
            member = members.get(name)
            if member is None:
                report.fail("types", f"{label}: pq_point has no member DIE for {name}")
                continue
            offset = member_offset(member)
            if offset != facts[key]:
                report.fail(
                    "types",
                    f"{label}: pq_point.{name} is at {offset!r} in DWARF and {facts[key]} "
                    "in the program",
                )
            chain = peel_types(type_of(member, unit), unit)
            if not chain or chain[-1].attrs.get(DW_AT_BYTE_SIZE) != facts["sizeof_int"]:
                report.fail(
                    "types", f"{label}: pq_point.{name} does not resolve to a {facts['sizeof_int']}-byte type"
                )

    typedef = object_type("pq_typedef_global")
    if typedef is not None:
        if typedef.tag != DW_TAG_TYPEDEF or typedef.name() != "pq_point_t":
            report.fail(
                "types",
                f"{label}: pq_typedef_global is typed {typedef.name()!r} (tag "
                f"{typedef.tag:#04x}), so the typedef spelling is lost",
            )
        else:
            target = type_of(typedef, unit)
            if target is None or target.tag != DW_TAG_STRUCTURE_TYPE \
                    or target.name() != "pq_point":
                report.fail(
                    "types",
                    f"{label}: pq_point_t does not refer to the pq_point structure DIE",
                )
            if type_size(typedef, unit) != facts["sizeof_point"]:
                report.fail(
                    "types",
                    f"{label}: pq_point_t is {type_size(typedef, unit)} bytes in DWARF and "
                    f"{facts['sizeof_point']} in the program",
                )

    # A parameter and a return type carry the spelling their declaration wrote.
    # `CType` erases typedefs, so this is the assertion that the alias survived
    # the trip from the front end to the writer: without it both DIEs are
    # structurally correct and still print `long unsigned int`.
    index_size = facts.get("sizeof_index")
    mark = die_by_tag_name(unit, DW_TAG_SUBPROGRAM, "pq_mark")
    if mark is None:
        report.fail(
            "types",
            f"{label}: no DW_TAG_subprogram for pq_mark, so no parameter spelling can be read",
        )
    else:
        described = {child.name(): child for child in children(unit, mark)
                     if child.tag == DW_TAG_FORMAL_PARAMETER}
        point = described.get("point")
        index = described.get("index")
        if point is None or index is None:
            report.fail(
                "types",
                f"{label}: pq_mark's parameters are {sorted(described)}, not {{'index', 'point'}}",
            )
        else:
            pointer = resolved_type(point, unit)
            if pointer is None or pointer.tag != DW_TAG_POINTER_TYPE:
                report.fail(
                    "types",
                    f"{label}: pq_mark's `point` is not a pointer DIE, so `pq_point_t *` "
                    "was collapsed",
                )
            else:
                alias = type_of(pointer, unit)
                if alias is None or alias.tag != DW_TAG_TYPEDEF or alias.name() != "pq_point_t":
                    report.fail(
                        "types",
                        f"{label}: pq_mark's `point` points at "
                        f"{alias.name() if alias is not None else None!r}, not at the "
                        "pq_point_t typedef",
                    )
                elif (target := type_of(alias, unit)) is None or target.tag != DW_TAG_STRUCTURE_TYPE:
                    report.fail(
                        "types", f"{label}: pq_point_t is not a typedef over the pq_point structure"
                    )
            alias = resolved_type(index, unit)
            if alias is None or alias.tag != DW_TAG_TYPEDEF or alias.name() != "pq_index_t":
                report.fail(
                    "types",
                    f"{label}: pq_mark's `index` is typed "
                    f"{alias.name() if alias is not None else None!r}, not the pq_index_t typedef",
                )
            elif index_size is not None and type_size(alias, unit) != index_size:
                report.fail(
                    "types",
                    f"{label}: pq_index_t is {type_size(alias, unit)} bytes in DWARF and "
                    f"{index_size} in the program",
                )
    next_fn = die_by_tag_name(unit, DW_TAG_SUBPROGRAM, "pq_next")
    if next_fn is None:
        report.fail("types", f"{label}: no DW_TAG_subprogram for pq_next")
    else:
        returned = type_of(next_fn, unit)
        if returned is None or returned.tag != DW_TAG_TYPEDEF or returned.name() != "pq_index_t":
            report.fail(
                "types",
                f"{label}: pq_next returns "
                f"{returned.name() if returned is not None else None!r}, not the pq_index_t typedef",
            )

    # An alias can name a pointer itself, and an array can be declared through
    # one: `typedef int (*pq_fn_t)(int); pq_fn_t h;` and `pq_point_t a[2]` must
    # keep the alias at the right depth, which is where a naive "peel every
    # layer, then match" walk puts it at the wrong one.
    handler = object_type("pq_typedef_handler")
    if handler is not None:
        if handler.tag != DW_TAG_TYPEDEF or handler.name() != "pq_fn_t":
            report.fail(
                "types",
                f"{label}: pq_typedef_handler is typed {handler.name()!r} "
                f"(tag {handler.tag:#04x}), so the alias that names a pointer was lost",
            )
        else:
            pointed = type_of(handler, unit)
            if pointed is None or pointed.tag != DW_TAG_POINTER_TYPE:
                report.fail(
                    "types",
                    f"{label}: pq_fn_t is not a typedef over a pointer type",
                )
            elif (signature := type_of(pointed, unit)) is None \
                    or signature.tag != DW_TAG_SUBROUTINE_TYPE:
                report.fail(
                    "types",
                    f"{label}: pq_fn_t's pointer does not name a subroutine type",
                )
    pair = object_type("pq_point_pair")
    if pair is not None:
        if pair.tag != DW_TAG_ARRAY_TYPE:
            report.fail(
                "types",
                f"{label}: pq_point_pair is typed {pair.name()!r} (tag {pair.tag:#04x}), "
                "not an array",
            )
        else:
            element = type_of(pair, unit)
            if element is None or element.tag != DW_TAG_TYPEDEF \
                    or element.name() != "pq_point_t":
                report.fail(
                    "types",
                    f"{label}: pq_point_pair's element is "
                    f"{element.name() if element is not None else None!r}, not the "
                    "pq_point_t typedef",
                )
            elif type_size(element, unit) != facts["sizeof_point"]:
                report.fail(
                    "types",
                    f"{label}: pq_point_pair's element is {type_size(element, unit)} bytes "
                    f"in DWARF and {facts['sizeof_point']} in the program",
                )

    holder = object_type("pq_holder_global")
    if holder is not None:
        if holder.tag != DW_TAG_STRUCTURE_TYPE or holder.name() != "pq_holder":
            report.fail(
                "types",
                f"{label}: pq_holder_global is typed {holder.name()!r} "
                f"(tag {holder.tag:#04x}), not the pq_holder structure",
            )
        else:
            members = {member.name(): member for member in children(unit, holder)
                       if member.tag == DW_TAG_MEMBER}
            for name, alias in (("count", "pq_index_t"), ("point", "pq_point_t")):
                member = members.get(name)
                if member is None:
                    report.fail("types", f"{label}: pq_holder has no member DIE for {name}")
                    continue
                typed = type_of(member, unit)
                if typed is None or typed.tag != DW_TAG_TYPEDEF or typed.name() != alias:
                    report.fail(
                        "types",
                        f"{label}: pq_holder.{name} is typed "
                        f"{typed.name() if typed is not None else None!r}, not the {alias} "
                        "typedef the member declaration wrote",
                    )

    matrix = object_type("pq_matrix")
    chain = peel_types(matrix, unit) if matrix is not None else []
    # A multi-dimensional array has two encodings in the wild and both are
    # correct: one array type whose children are all the dimensions (GCC 16.2),
    # or one array type per dimension (Clang, and this writer). The gate reads
    # the dimensions in order and does not care which shape carries them.
    bounds = []
    for die in chain:
        if die.tag != DW_TAG_ARRAY_TYPE:
            continue
        for child in children(unit, die):
            if child.tag != DW_TAG_SUBRANGE_TYPE:
                continue
            bound = child.attrs.get(DW_AT_COUNT, child.attrs.get(DW_AT_UPPER_BOUND))
            bounds.append(bound + 1 if isinstance(bound, int) else None)
    if bounds != [2, 3]:
        report.fail(
            "types",
            f"{label}: pq_matrix[2][3] is described with dimensions {bounds}; the outermost "
            "dimension must come first",
        )
    if chain and chain[-1].tag != DW_TAG_BASE_TYPE:
        report.fail(
            "types", f"{label}: pq_matrix does not resolve to an element base type"
        )
    element = chain[-1].attrs.get(DW_AT_BYTE_SIZE) if chain else None
    if element != facts["sizeof_int"]:
        report.fail("types", f"{label}: pq_matrix elements are {element!r} bytes, not int")
    elif bounds == [2, 3] and element * 2 * 3 != facts["sizeof_matrix"]:
        report.fail(
            "types",
            f"{label}: the described pq_matrix is {element * 6} bytes and the program's is "
            f"{facts['sizeof_matrix']}",
        )

    bits = object_type("pq_bits_global")
    if bits is not None:
        if bits.tag != DW_TAG_STRUCTURE_TYPE:
            report.fail("types", f"{label}: pq_bits_global is not a structure type")
        else:
            if type_size(bits, unit) != facts["sizeof_bits"]:
                report.fail(
                    "types",
                    f"{label}: struct pq_bits is {type_size(bits, unit)} bytes in DWARF and "
                    f"{facts['sizeof_bits']} in the program",
                )
            placements = {}
            for member in children(unit, bits):
                if member.tag == DW_TAG_MEMBER:
                    placements[member.name()] = member_bit_placement(member, unit.version)
            expected = {"low": (0, 3), "high": (3, 5)}
            for name, placement in expected.items():
                if placements.get(name) != placement:
                    report.fail(
                        "types",
                        f"{label}: bit-field pq_bits.{name} is placed at "
                        f"{placements.get(name)!r}, expected {placement!r}",
                    )

    word = object_type("pq_word_global")
    if word is not None:
        if word.tag != DW_TAG_UNION_TYPE:
            report.fail("types", f"{label}: pq_word_global is not a union type")
        else:
            if type_size(word, unit) != facts["sizeof_word"]:
                report.fail(
                    "types",
                    f"{label}: union pq_word is {type_size(word, unit)} bytes in DWARF and "
                    f"{facts['sizeof_word']} in the program",
                )
            for member in children(unit, word):
                if member.tag == DW_TAG_MEMBER and DW_AT_DATA_MEMBER_LOCATION in member.attrs \
                        and member.attrs[DW_AT_DATA_MEMBER_LOCATION] != 0:
                    report.fail(
                        "types",
                        f"{label}: union member {member.name()} carries a member location, so "
                        "a consumer would look for it outside the union",
                    )

    colour = object_type("pq_colour_global")
    if colour is not None:
        if colour.tag != DW_TAG_ENUMERATION_TYPE:
            report.fail("types", f"{label}: pq_colour_global is not an enumeration type")
        else:
            values = {
                die.name(): die.attrs.get(DW_AT_CONST_VALUE)
                for die in children(unit, colour)
                if die.tag == DW_TAG_ENUMERATOR
            }
            for name, key in (("PQ_RED", "value_red"), ("PQ_GREEN", "value_green"),
                              ("PQ_BLUE", "value_blue")):
                if values.get(name) != facts[key]:
                    report.fail(
                        "types",
                        f"{label}: enumerator {name} is {values.get(name)!r} in DWARF and "
                        f"{facts[key]} in the program",
                    )
            underlying = type_of(colour, unit)
            if underlying is None or underlying.tag != DW_TAG_BASE_TYPE:
                report.fail(
                    "types", f"{label}: pq_colour does not name an underlying integer type"
                )


def unit_root_variable(unit: Unit, name: str) -> Die | None:
    """The file-scope variable DIE called `name`, preferring the outermost one."""
    best = None
    for die in unit.by_tag(DW_TAG_VARIABLE):
        if die_name(die, unit) == name and (best is None or die.depth < best.depth):
            best = die
    return best


def location_address(die: Die, elf: Elf) -> int | None:
    """The address a `DW_OP_addr` location expression names, when that is all it is.

    A file-scope object has a fixed address, so `DW_OP_addr` is the whole
    expression. Anything else -- a register, a frame-relative location -- is a
    local the gate does not have a program-side address for, and is skipped.
    """
    form = die.forms.get(DW_AT_LOCATION)
    expression = die.attrs.get(DW_AT_LOCATION)
    if form != 0x18 and form not in DW_FORM_BLOCKS:
        return None
    if not isinstance(expression, bytes):
        return None
    if len(expression) != 1 + elf.address_size or expression[0] != DW_OP_ADDR:
        return None
    return int.from_bytes(expression[1:], elf.order)


#: The `pq_args` battery's parameters, in declaration order. The probe takes all
#: eleven addresses, so each one is an object in memory at every tier and every
#: one of them must be locatable -- which makes absence a defect and not a tier
#: policy.
PARAMETER_BATTERY = ("a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k")

# The 32-bit target is measured with a freestanding program: a host that can
# compile `-m32` often cannot link a 32-bit libc, and the check needs to *run* on
# the target, not merely decode an object.  `_start` writes with `int $0x80` and
# prints the same two facts the 64-bit probe prints -- each parameter's distance
# from its own frame address and the value the function itself read -- plus a
# checksum of the ten integers, so a call that lost an argument is visible.
PARAMETER_PROBE_32 = r"""
static long delta[11];
static long value[11];
static char out[2048];
static int out_len;

static void put_char(char c) { out[out_len++] = c; }
static void put_int(long v) {
    char buf[24];
    int n = 0;
    unsigned long u;
    if (v < 0) { put_char('-'); u = (unsigned long)(-v); } else { u = (unsigned long)v; }
    do { buf[n++] = (char)('0' + (int)(u % 10)); u /= 10; } while (u);
    while (n) put_char(buf[--n]);
}
static void put_entry(char prefix, int index, long v) {
    put_char(prefix);
    if (index >= 10) put_char((char)('0' + index / 10));
    put_char((char)('0' + index % 10));
    put_char(' ');
    put_int(v);
    put_char('\n');
}

__attribute__((noinline))
long pq_args(int a, int b, int c, int d, int e, float f, double g, void *h, int i, int j, int k) {
    unsigned long frame = (unsigned long)__builtin_frame_address(0);
    delta[0] = (long)((unsigned long)(void *)&a - frame);
    delta[1] = (long)((unsigned long)(void *)&b - frame);
    delta[2] = (long)((unsigned long)(void *)&c - frame);
    delta[3] = (long)((unsigned long)(void *)&d - frame);
    delta[4] = (long)((unsigned long)(void *)&e - frame);
    delta[5] = (long)((unsigned long)(void *)&f - frame);
    delta[6] = (long)((unsigned long)(void *)&g - frame);
    delta[7] = (long)((unsigned long)(void *)&h - frame);
    delta[8] = (long)((unsigned long)(void *)&i - frame);
    delta[9] = (long)((unsigned long)(void *)&j - frame);
    delta[10] = (long)((unsigned long)(void *)&k - frame);
    value[0] = a; value[1] = b; value[2] = c; value[3] = d; value[4] = e;
    value[5] = (long)f; value[6] = (long)g; value[7] = (long)h;
    value[8] = i; value[9] = j; value[10] = k;
    return a + b + c + d + e + (long)f + (long)g + i + j + k;
}

void _start(void) {
    long sum = pq_args(1, 2, 3, 4, 5, 6.0f, 7.0, (void *)0x1000, 9, 10, 11);
    for (int n = 0; n < 11; n++) put_entry('d', n, delta[n]);
    for (int n = 0; n < 11; n++) put_entry('v', n, value[n]);
    put_entry('s', 0, sum);
    __asm__ volatile("int $0x80" :: "a"(4), "b"(1), "c"(out), "d"(out_len));
    __asm__ volatile("int $0x80" :: "a"(1), "b"(0));
    for (;;) { }
}
"""


def frame_offset(die: Die) -> int | None:
    """The `DW_OP_fbreg` offset of a location expression, or `None`.

    `None` means the expression is not one frame-relative offset. For a parameter
    of this writer that is a defect rather than a policy: `DW_OP_fbreg` is the
    only form it emits for a parameter, and a consumer that meets anything else
    either gives up on the variable or computes an address no object has.
    """
    expression = die.attrs.get(DW_AT_LOCATION)
    if not isinstance(expression, bytes) or not expression:
        return None
    if expression[0] != DW_OP_FBREG:
        return None
    # A block that ends mid-LEB is truncated, which is unreadable rather than
    # exceptional: a malformed object must be a reported failure, not a traceback.
    try:
        offset, after = _sleb(expression, 1)
    except IndexError:
        return None
    return offset if after == len(expression) else None


def check_parameter_locations(
    report: Report, elf: Elf, parsed: list[Unit], label: str, facts: dict[str, int]
) -> None:
    """A parameter's stated location must be the address the program took for it.

    The probe prints its own frame address and where each parameter lives, so the
    expected offset is `(address - frame) - frame record`: the `DW_OP_fbreg`
    displacement is measured from the canonical frame address, which is one frame
    record above the frame address the probe recorded.  Spelled as an address the
    check is `frame + record + fbreg == address`, the evaluation a consumer
    performs -- the address the *running* code gave the object, not a second
    compiler's opinion about where it ought to be.  A `DW_OP_fbreg` that
    disagrees sends a debugger's `print` to the wrong stack slot, which is worse
    than the `<optimized out>` a missing location gives, so both a wrong value and
    an unreadable expression are failures.

    Every parameter DIE in the unit is also required to hold a readable
    expression: the writer emits nothing else, and a malformed block anywhere in
    the unit would go unnoticed by a check that only looked at this battery.
    """
    unit = parsed[0]
    subprogram = next(
        (die for die in unit.by_tag(DW_TAG_SUBPROGRAM)
         if die_name(die, unit) == "pq_args" and DW_AT_LOW_PC in die.attrs),
        None,
    )
    if subprogram is None:
        report.fail(
            "parameter_locations",
            f"{label}: pq_args has no subprogram DIE, so no parameter location can be checked",
        )
        return
    parameters = [die for die in children(unit, subprogram)
                  if die.tag == DW_TAG_FORMAL_PARAMETER]
    if len(parameters) != len(PARAMETER_BATTERY):
        report.fail(
            "parameter_locations",
            f"{label}: pq_args takes {len(PARAMETER_BATTERY)} parameters but its subprogram "
            f"has {len(parameters)} DW_TAG_formal_parameter children",
        )
        return
    record = facts["frame_record"]
    checked = 0
    for index, (name, die) in enumerate(zip(PARAMETER_BATTERY, parameters)):
        spelled = die_name(die, unit)
        if spelled != name:
            report.fail(
                "parameter_locations",
                f"{label}: pq_args parameter {index} is {spelled!r} in DWARF and {name!r} in "
                "the source, so every location in the list is off by one",
            )
            continue
        # The displacement is from the canonical frame address, which sits one
        # frame record above the frame address the probe recorded -- not below it.
        expected = facts[f"arg_delta_{index}"] - record
        value = die.attrs.get(DW_AT_LOCATION)
        if value is None:
            report.fail(
                "parameter_locations",
                f"{label}: pq_args.{name} lives at {expected} from the frame base in the "
                "program's own frame but its DIE states no location",
            )
            continue
        offset = frame_offset(die)
        if offset is None:
            report.fail(
                "parameter_locations",
                f"{label}: pq_args.{name} has a location a consumer cannot read as one frame "
                f"offset: {value!r}",
            )
            continue
        if offset != expected:
            report.fail(
                "parameter_locations",
                f"{label}: pq_args.{name} is DW_OP_fbreg {offset}, which names "
                f"{offset + record} from the frame address; the program's own "
                f"frame puts it at {facts[f'arg_delta_{index}']}",
            )
            continue
        checked += 1
    # The rest of the unit: readable or nothing, never unreadable.
    for die in unit.by_tag(DW_TAG_FORMAL_PARAMETER):
        if DW_AT_LOCATION in die.attrs and frame_offset(die) is None:
            report.fail(
                "parameter_locations",
                f"{label}: parameter {die_name(die, unit)!r} has a location a consumer cannot "
                f"read as one frame offset: {die.attrs[DW_AT_LOCATION]!r}",
            )
    report.measurements.setdefault("parameter_locations", {})[label] = checked


def run_debugger_lane(report: Report, lccc: list[str], gdb: str, workdir: Path) -> None:
    """Build the probe at two tiers and ask a real debugger for its parameters.

    `-O0` places every parameter, so GDB must print all eleven; `-O2` may keep
    one in a register, and a parameter GDB calls `<optimized out>` is skipped
    there -- but one it prints must be the value the function itself saw.
    """
    probes = write_probes(workdir)
    for optimization in ("-O0", "-O2"):
        label = f"debugger-{optimization.lstrip('-')}"
        output = workdir / f"{label}.elf"
        if not build(
            report, lccc, ["-g", "-gdwarf-5", optimization, "-no-pie"], [probes["probe"]],
            output, workdir,
        ):
            continue
        facts = program_facts(report, output, label)
        if facts is None:
            continue
        # `-O0` places every parameter, so a parameter the debugger cannot print
        # is the regression this lane exists for; at `-O2` a register home is
        # allowed and only a *wrong* value is a failure.
        check_debugger_agreement(report, gdb, output, label, facts,
                                 required=optimization == "-O0")


def check_debugger_agreement(
    report: Report, gdb: str, program: Path, label: str, facts: dict[str, int], required: bool
) -> None:
    """Ask a real debugger for the parameters and compare with the program.

    Every other check here decodes what the writer produced and compares it with
    what the probe measured -- and both sides are code this repository owns.  That
    is not enough: a sign error in the frame arithmetic made the gate's own
    formula agree with the writer while GDB, the consumer the information exists
    for, printed the caller's saved frame pointer as the first argument.  So the
    value GDB prints is compared with the value the function itself stored,
    parameter by parameter.

    The probe is asked for each parameter with its own `printf`, so a parameter
    GDB cannot place is visibly absent instead of shifting every later answer by
    one.  A GDB that cannot run here at all (no ptrace, no install) is a
    measurement, not a failure: the environment is not the compiler.
    """
    commands = ["-batch", "-nx", "-ex", "break pq_args", "-ex", "run"]
    for index, name in enumerate(PARAMETER_BATTERY):
        commands += ["-ex", f'printf "arg{index}=%ld\\n", (long){name}']
    try:
        proc = subprocess.run(
            [gdb, *commands, str(program)], text=True, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, check=False, timeout=180,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        report.measurements.setdefault("debugger", {})[label] = f"skipped: {error}"
        return
    printed = {int(index): int(value)
               for index, value in re.findall(r"^arg(\d+)=(-?\d+)$", proc.stdout, re.MULTILINE)}
    if not printed:
        reason = next(
            (line.strip() for line in proc.stdout.splitlines()
             if "ptrace" in line or "No such file" in line or "error" in line.lower()),
            "no values",
        )
        report.measurements.setdefault("debugger", {})[label] = f"skipped: {reason}"[:200]
        return
    for index, name in enumerate(PARAMETER_BATTERY):
        if index not in printed:
            if required:
                report.fail(
                    "debugger",
                    f"{label}: gdb cannot print pq_args.{name} and every parameter has a home at "
                    "this tier, so the DIE names no address a consumer can resolve",
                )
            continue
        want = facts[f"arg_value_{index}"]
        if printed[index] != want:
            report.fail(
                "debugger",
                f"{label}: gdb prints pq_args.{name} as {printed[index]} and the function's own "
                f"parameter is {want}, so the location a consumer resolves is not the object",
            )
    report.measurements.setdefault("debugger", {})[label] = len(printed)


def check_parameters(report: Report, elf: Elf, parsed: list[Unit], label: str) -> None:
    """Every parameter of a function this unit defines needs its own DIE.

    A `DW_TAG_formal_parameter` inside a `subroutine_type` describes a *type* --
    it is anonymous and says nothing about a live call. What a debugger needs to
    print `n` while stopped in `pq_frame` is a named child of the subprogram DIE,
    which is a different part of the graph and is what this check requires.
    """
    unit = parsed[0]
    required = {"pq_frame": ("n",), "pq_leaf": ("x",)}
    described = 0
    located = 0
    # A function that takes `...` must say so: a consumer otherwise reads a
    # complete argument list and reports the extra ones as garbage.
    emitted = {symbol.name for symbol in elf.functions()}
    variadic = next(
        (die for die in unit.by_tag(DW_TAG_SUBPROGRAM)
         if die_name(die, unit) == "pq_sum" and DW_AT_LOW_PC in die.attrs),
        None,
    )
    if variadic is None:
        # An optimizing tier may fold the callee into its caller; a function with
        # no symbol of its own has no DIE to describe, which is the same policy
        # the function check below applies.
        if "pq_sum" in emitted:
            report.fail(
                "parameters", f"{label}: pq_sum is defined in .symtab but has no subprogram DIE"
            )
    else:
        markers = [die for die in children(unit, variadic)
                   if die.tag == DW_TAG_UNSPECIFIED_PARAMETERS]
        if len(markers) != 1:
            report.fail(
                "parameters",
                f"{label}: pq_sum takes `...` but its subprogram has {len(markers)} "
                "DW_TAG_unspecified_parameters children",
            )
        if not [die for die in children(unit, variadic)
                if die.tag == DW_TAG_FORMAL_PARAMETER]:
            report.fail(
                "parameters",
                f"{label}: pq_sum's first parameter is missing, so the DIE describes a "
                "function whose arguments cannot be listed at all",
            )
    for name, parameters in required.items():
        subprogram = next(
            (die for die in unit.by_tag(DW_TAG_SUBPROGRAM)
             if die_name(die, unit) == name and DW_AT_LOW_PC in die.attrs),
            None,
        )
        if subprogram is None:
            if name in emitted:
                report.fail(
                    "parameters", f"{label}: {name} is defined in .symtab but has no subprogram DIE"
                )
            continue
        named = {
            die_name(die, unit): die
            for die in children(unit, subprogram)
            if die.tag == DW_TAG_FORMAL_PARAMETER
        }
        for parameter in parameters:
            die = named.get(parameter)
            if die is None:
                report.gap(
                    "parameters",
                    f"{label}: {name} has no DW_TAG_formal_parameter DIE for {parameter} "
                    f"(children: {sorted(named) or 'none'})",
                )
                continue
            described += 1
            located += DW_AT_LOCATION in die.attrs
            target = resolved_type(die, unit)
            if target is None:
                report.fail(
                    "parameters", f"{label}: parameter {name}.{parameter} names no type"
                )
            elif type_size(target, unit) is None:
                report.fail(
                    "parameters",
                    f"{label}: parameter {name}.{parameter} has a type with no byte size",
                )
    if described:
        report.measurements.setdefault("parameters", {})[label] = described
    report.measurements.setdefault("param_located", {})[label] = located


def check_declaration_policy(report: Report, path: Path, label: str) -> None:
    """Describe the declarations the code refers to, and only those.

    `<stdio.h>` declares `stdin`, `stdout` and `stderr`, each naming the whole
    `_IO_FILE` structure; describing them all costs a debug payload the program
    never asks for (measured on `fib.c`: `.debug_info` 578 bytes against GCC
    16.2's 249). GCC 16.2's rule, measured on a pair of `extern` declarations
    where only one is used, is that a declaration gets a DIE exactly when the
    unit's code refers to it -- and a record that only an unreferenced
    declaration names is not described at all.
    """
    try:
        elf = read_elf(path)
    except ValueError as error:
        report.fail("declarations", f"{label}: {error}")
        return
    try:
        parsed = units(elf)
    except ValueError as error:
        report.fail("declarations", f"{label}: {error}")
        return
    if not parsed:
        report.fail("declarations", f"{label}: no compile unit")
        return
    unit = parsed[0]
    variables = {die_name(die, unit): die for die in unit.by_tag(DW_TAG_VARIABLE)
                 if die_name(die, unit)}
    used = variables.get("pq_decl_used")
    if used is None:
        report.fail(
            "declarations",
            f"{label}: pq_decl_used is referred to by the code but has no DIE, so a "
            "debugger cannot print it",
        )
    elif DW_AT_DECLARATION not in used.attrs:
        report.fail(
            "declarations",
            f"{label}: pq_decl_used is only declared here, but its DIE does not say so",
        )
    if "pq_decl_unused" in variables:
        report.fail(
            "declarations",
            f"{label}: pq_decl_unused is declared and never referred to, so its DIE "
            f"costs {len(unit.dies)}-DIE worth of a payload nothing asked for",
        )
    name_only = [die for die in unit.by_tag(DW_TAG_STRUCTURE_TYPE)
                 if die.name() == "pq_decl_rec"]
    if name_only:
        report.fail(
            "declarations",
            f"{label}: the record pq_decl_rec is reachable only from an unreferenced "
            "declaration but is still described, with its members",
        )
    report.measurements.setdefault("declarations", {})[label] = len(unit.dies)


def check_object_locations(
    report: Report, elf: Elf, parsed: list[Unit], label: str, facts: dict[str, int]
) -> None:
    """A variable's location must be its address -- and only where it has one.

    Two defects hide here and neither is visible in a shape comparison. A missing
    `DW_AT_location` makes a debugger print `<optimized out>` for an object the
    program can see; a location naming a symbol the optimizer deleted makes the
    link fail outright, because the relocation has no target. So the rule the
    gate enforces is the one GCC 16.2 follows: the attribute is present exactly
    when `.symtab` defines the object.
    """
    symbols = {symbol.name: symbol for symbol in elf.objects("")}
    described = 0
    for unit in parsed:
        for die in unit.by_tag(DW_TAG_VARIABLE):
            name = die_name(die, unit)
            if not name:
                continue
            described += 1
            address = location_address(die, elf)
            symbol = symbols.get(name)
            declaration = DW_AT_DECLARATION in die.attrs
            if address is None:
                if symbol is not None and not declaration:
                    report.fail(
                        "locations",
                        f"{label}: {name} is defined in .symtab at {symbol.value:#x} but its "
                        "DIE carries no DW_AT_location, so a debugger cannot read it",
                    )
                continue
            if symbol is None:
                report.fail(
                    "locations",
                    f"{label}: {name} has a DW_AT_location but .symtab does not define it, "
                    f"so the {address:#x} address is a relocation with no target",
                )
            elif address != symbol.value:
                report.fail(
                    "locations",
                    f"{label}: {name} is at {symbol.value:#x} but its DW_AT_location says "
                    f"{address:#x}",
                )
    if described == 0:
        report.fail("locations", f"{label}: no variable DIE at all")
    global_die = unit_root_variable(parsed[0], "pq_global")
    if global_die is None:
        report.fail("locations", f"{label}: no DIE for pq_global")
    else:
        address = location_address(global_die, elf)
        if address != facts["addr_global"]:
            report.fail(
                "locations",
                f"{label}: pq_global is at {facts['addr_global']:#x} in the program and "
                f"{address if address is None else hex(address)} in DWARF",
            )


def check_data_only_unit(report: Report, path: Path, label: str) -> None:
    """A unit with no code still needs a unit, a line program and real locations.

    GCC 16.2 emits the same shape for a translation unit that defines only data:
    a compile unit with `DW_AT_stmt_list` and no `DW_AT_low_pc`, a `.debug_line`
    program whose header carries the file table (the file table is what
    `DW_AT_decl_file` indexes, so it is needed even with no statements at all),
    and one `DW_TAG_variable` per defined object.
    """
    try:
        elf = read_elf(path)
    except ValueError as error:
        report.fail("data_only_unit", f"{label}: {error}")
        return
    if not elf.sections.get(".debug_info"):
        report.fail("data_only_unit", f"{label}: no .debug_info section at all")
        return
    try:
        parsed = units(elf)
    except ValueError as error:
        report.fail("data_only_unit", f"{label}: {error}")
        return
    if not parsed:
        report.fail("data_only_unit", f"{label}: .debug_info exists but holds no unit")
        return
    unit = parsed[0]
    root = unit.root()
    if root is None:
        report.fail("data_only_unit", f"{label}: the unit has no root DIE")
        return
    if DW_AT_LOW_PC in root.attrs:
        report.fail(
            "data_only_unit",
            f"{label}: a unit with no code states DW_AT_low_pc {root.attrs[DW_AT_LOW_PC]:#x}",
        )
    if DW_AT_STMT_LIST not in root.attrs:
        report.fail(
            "data_only_unit",
            f"{label}: the unit has no DW_AT_stmt_list, so its DW_AT_decl_files name no "
            "file table",
        )
    line = elf.sections.get(".debug_line")
    files: list[str] = []
    if not line:
        report.fail("data_only_unit", f"{label}: no .debug_line section")
    else:
        try:
            files = line_table(elf, root.attrs.get(DW_AT_STMT_LIST, 0)).files
        except (ValueError, IndexError) as error:
            report.fail("data_only_unit", f"{label}: .debug_line does not parse: {error}")
        else:
            if not files:
                report.fail("data_only_unit", f"{label}: the line program has no file table")
            elif not any(entry.endswith("probe_data.c") for entry in files):
                report.fail(
                    "data_only_unit",
                    f"{label}: the line program lists {files}, not the source file",
                )
    named = {}
    for die in unit.by_tag(DW_TAG_VARIABLE):
        name = die_name(die, unit)
        if name:
            named[name] = die
    for name in ("pq_only_data", "pq_only_text"):
        if name not in named:
            report.fail(
                "data_only_unit",
                f"{label}: the unit names variables {sorted(named) or 'none'}, not {name}",
            )
    # An unlinked object carries `.rela.debug_info`, not addresses; the location's
    # *existence* is what this check can see, and the linked programs above prove
    # what it points at.
    if "pq_only_data" in named and DW_AT_LOCATION not in named["pq_only_data"].attrs:
        report.fail(
            "data_only_unit", f"{label}: pq_only_data has no DW_AT_location to relocate"
        )
    report.measurements.setdefault("data_unit", {})[label] = {
        "unit_length": unit_length(elf, unit),
        "files": len(files),
    }


def code_sections(elf: Elf) -> bytes:
    """Every instruction byte of an object: `.text` and its per-function pieces."""
    return b"".join(
        blob for name, blob in sorted(elf.sections.items())
        if name == ".text" or name.startswith(".text.")
    )


def check_code_unchanged(
    report: Report,
    lccc: list[str],
    flags: list[str],
    probe: Path,
    label: str,
    workdir: Path,
) -> None:
    """`-g` must describe the program, not change it.

    The strongest statement this gate can make about cost is that the debug
    information is *free at run time*: the same translation unit compiled with
    and without the debug flags must contain the same instruction bytes.  A
    change that leaked into code selection would show up here as a difference,
    and the assertion costs two small compiles per configuration.
    """
    plain = [flag for flag in flags if not flag.startswith("-g")]
    built = {}
    for suffix, twin_flags in (("debug", flags), ("plain", plain)):
        output = workdir / f"{label}-{suffix}.o"
        if not build(report, lccc, twin_flags, [probe], output, workdir, link=False):
            return
        try:
            built[suffix] = code_sections(read_elf(output))
        except ValueError as error:
            report.fail("code_unchanged", f"{label}: {error}")
            return
    if not built["debug"]:
        report.fail("code_unchanged", f"{label}: the probe produced no instruction bytes to compare")
        return
    if built["debug"] != built["plain"]:
        report.fail(
            "code_unchanged",
            f"{label}: {len(built['debug'])} bytes of code with -g against "
            f"{len(built['plain'])} without — debug information must not change the program",
        )


def unit_length(elf: Elf, unit: Unit) -> int:
    """The `.debug_info` contribution length of `unit`."""
    info = elf.sections.get(".debug_info", b"")
    if unit.offset + 4 > len(info):
        return 0
    return elf.u32(unit.offset, info)



def run_gate(lccc: list[str], gcc: list[str] | None, workdir: Path) -> Report:
    report = Report()
    probes = write_probes(workdir)
    spans = {name: function_spans(PROBES[name]) for name in PROBES}
    matrices = (("2", "-O0"), ("3", "-O0"), ("4", "-O0"), ("5", "-O0"), ("5", "-O1"), ("5", "-O2"))
    for version, optimization in matrices:
        label = f"dwarf{version}-{optimization}"
        output = workdir / f"{label}.elf"
        flags = ["-g", f"-gdwarf-{version}", optimization, "-no-pie"]
        check_code_unchanged(report, lccc, flags, probes["probe"], label, workdir)
        if not build(report, lccc, flags, [probes["probe"]], output, workdir):
            continue
        try:
            elf = read_elf(output)
        except ValueError as error:
            report.fail("elf", f"{label}: {error}")
            continue
        parsed = check_unit_structure(report, elf, int(version), label)
        if not parsed:
            continue
        ranges = check_subprograms(report, elf, parsed, label, spans["probe"])
        check_abbrev_usage(report, elf, parsed, label)
        check_lines(
            report, elf, parsed, ranges, label, spans["probe"],
            inlining=optimization != "-O0",
        )
        check_aranges(report, elf, parsed, ranges, label)
        check_eh_frame(report, elf, ranges, label)
        check_static_policy(report, elf, label, optimization.lstrip("-"))
        check_debug_payload(report, elf, label, optimizing=optimization != "-O0")
        check_parameters(report, elf, parsed, label)
        facts = program_facts(report, output, label)
        if facts is not None:
            check_type_dies(report, elf, parsed, label, facts)
            check_object_locations(report, elf, parsed, label, facts)
            check_parameter_locations(report, elf, parsed, label, facts)

    # One function per section is the kernel's and Debian's default packaging, and
    # it moves a unit from a low/high pair to a range list -- a different section,
    # a different encoding, and the only path in which `.debug_rnglists` (v5) or
    # `.debug_ranges` (v2..4) is emitted at all.
    for version, function_sections in (("2", False), ("4", True), ("5", True), ("5", False)):
        if not function_sections:
            continue
        label = f"dwarf{version}-sections"
        output = workdir / f"{label}.elf"
        if not build(
            report, lccc, ["-g", f"-gdwarf-{version}", "-O0", "-ffunction-sections", "-no-pie"],
            [probes["probe"]], output, workdir
        ):
            continue
        try:
            elf = read_elf(output)
        except ValueError as error:
            report.fail("elf", f"{label}: {error}")
            continue
        parsed = check_unit_structure(report, elf, int(version), label)
        if not parsed:
            continue
        if DW_AT_RANGES not in (parsed[0].root().attrs if parsed[0].root() else {}):
            report.fail(
                "range_list",
                f"{label}: code in {len({name for name in elf.sections if name.startswith('.text')})} "
                "sections but the unit states one low/high pair",
            )
        ranges = check_subprograms(report, elf, parsed, label, spans["probe"])
        check_lines(report, elf, parsed, ranges, label, spans["probe"])
        check_range_lists(report, elf, parsed, ranges, label)
        check_aranges(report, elf, parsed, ranges, label)
        check_eh_frame(report, elf, ranges, label)

    # `-Wl,--gc-sections` is how the kernel and every Debian package link: it
    # drops whole function sections, and the DIEs that described them survive with
    # zeroed addresses. Both compilers leave those behind, so the checks must
    # recognise them instead of reporting a phantom defect -- and must still fail
    # when a *live* function's address is zero.
    label = "dwarf5-gc-sections"
    output = workdir / f"{label}.elf"
    if build(
        report,
        lccc,
        ["-g", "-gdwarf-5", "-O0", "-ffunction-sections", "-Wl,--gc-sections", "-no-pie"],
        [probes["probe"]],
        output,
        workdir,
    ):
        try:
            elf = read_elf(output)
        except ValueError as error:
            report.fail("elf", f"{label}: {error}")
        else:
            parsed = check_unit_structure(report, elf, 5, label)
            if parsed:
                ranges = check_subprograms(report, elf, parsed, label, spans["probe"])
                check_lines(report, elf, parsed, ranges, label, spans["probe"])
                check_range_lists(report, elf, parsed, ranges, label)
                check_aranges(report, elf, parsed, ranges, label)
                check_eh_frame(report, elf, ranges, label)

    # Two units in one program: every set must name exactly one unit.
    output = workdir / "multi.elf"
    if build(
        report, lccc, ["-g", "-gdwarf-5", "-O0", "-no-pie"],
        [probes["probe_a"], probes["probe_b"]], output, workdir
    ):
        try:
            elf = read_elf(output)
            parsed = check_unit_structure(report, elf, 5, "multi")
            if len(parsed) == 2:
                ranges = check_subprograms(
                    report, elf, parsed, "multi", spans["probe_a"] + spans["probe_b"]
                )
                check_lines(
                    report, elf, parsed, ranges, "multi", spans["probe_a"] + spans["probe_b"]
                )
                check_aranges(report, elf, parsed, ranges, "multi")
                check_eh_frame(report, elf, ranges, "multi")
            else:
                report.fail(
                    "multi_unit", f"multi: {len(parsed)} compile units for two source files"
                )
        except ValueError as error:
            report.fail("multi_unit", f"multi: {error}")

    # The declaration policy, on an object that does not need to link (one of the
    # two declared names has no definition anywhere in the harness).
    output = workdir / "decl.o"
    if build(
        report, lccc, ["-g", "-gdwarf-5", "-O0"], [probes["probe_decl"]], output, workdir,
        link=False,
    ):
        check_declaration_policy(report, output, "decl")

    # A unit that is all data.
    output = workdir / "data.o"
    if build(
        report, lccc, ["-g", "-gdwarf-5", "-O0"], [probes["probe_data"]], output, workdir,
        link=False,
    ):
        check_data_only_unit(report, output, "data")

    if gcc is not None:
        oracle_comparison(report, lccc, gcc, probes, workdir)
    return report


def run_32_bit_parameters(
    report: Report, lccc: list[str], workdir: Path, gcc: list[str] | None = None
) -> None:
    """The parameter-address contract measured on a 32-bit target itself.

    The 64-bit lane compares a `DW_OP_fbreg` with the address the running program
    took and with the value a debugger prints.  Neither is possible for `-m32` on
    a 64-bit host by the same means -- there is no 32-bit GDB and usually no
    32-bit libc -- so the probe is freestanding: `_start` writes with `int $0x80`,
    which needs no runtime at all.  What is checked is the same law with the
    32-bit frame record: the canonical frame address is `%ebp + 8`, so every
    displacement is the parameter's own address minus `%ebp`, less 8.  A `-m32`
    target that cannot execute here is a measurement, not a failure.

    GCC 16.2 is run through the identical check when it is installed.  That is a
    control on the *law*, not on lccc: the same probe, the same arithmetic and an
    independent frame layout (GCC keeps the `double` 8 bytes higher) must satisfy
    it, so a rule that only the compiler under test obeys is caught here rather
    than discovered by a user.
    """
    targets = [("m32-dwarf5--O0", lccc, True)]
    if gcc:
        targets.append(("m32-gcc-oracle--O0", gcc, False))
    for label, compiler, required in targets:
        measure_32_bit_parameters(report, compiler, workdir, label, required)


def measure_32_bit_parameters(
    report: Report, compiler: list[str], workdir: Path, label: str, required: bool
) -> None:
    """Build, run and check the freestanding 32-bit probe with one compiler."""
    source = workdir / "pq_args32.c"
    source.write_text(PARAMETER_PROBE_32)
    output = workdir / "pq_args32.elf"
    argv = [*compiler, "-m32", "-O0", "-g", "-gdwarf-5", "-nostdlib", "-static", "-no-pie",
            "-o", str(output), str(source)]
    proc = subprocess.run(argv, cwd=workdir, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, check=False, timeout=180)
    report.objects += 1
    if proc.returncode != 0:
        detail = f"{label}: the freestanding 32-bit probe did not build: {proc.stderr.strip()[:200]}"
        if required:
            report.fail("parameter_locations", detail)
        else:
            report.measurements.setdefault("parameter_locations", {})[label] = f"skipped: {detail}"
        return
    try:
        run = subprocess.run([str(output)], text=True, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, check=False, timeout=120)
    except OSError as error:
        report.measurements.setdefault("parameter_locations", {})[label] = (
            f"skipped: cannot execute a 32-bit program here ({error})"
        )
        return
    if run.returncode != 0:
        report.measurements.setdefault("parameter_locations", {})[label] = (
            f"skipped: the 32-bit probe exited {run.returncode}"
        )
        return
    facts: dict[str, int] = {}
    for line in run.stdout.splitlines():
        key, _, value = line.partition(" ")
        if key.startswith("d"):
            facts[f"arg_delta_{int(key[1:])}"] = int(value)
        elif key.startswith("v"):
            facts[f"arg_value_{int(key[1:])}"] = int(value)
        elif key == "s0":
            facts["sum"] = int(value)
    if len(facts) != 23:
        report.fail("parameter_locations", f"{label}: the probe printed {len(facts)} of 23 facts")
        return
    # Ten integers, as the function itself read them: 1+2+3+4+5+6+7+9+10+11.
    if facts["sum"] != 58:
        report.fail("parameter_locations",
                    f"{label}: the callee summed its integer parameters to {facts['sum']}, not 58, "
                    "so the call itself did not deliver every argument")
        return
    expected_values = (1, 2, 3, 4, 5, 6, 7, 0x1000, 9, 10, 11)
    for index, expected_value in enumerate(expected_values):
        if facts[f"arg_value_{index}"] != expected_value:
            report.fail(
                "parameter_locations",
                f"{label}: parameter {index} read {facts[f'arg_value_{index}']} in the callee, "
                f"not the argument the caller passed ({expected_value})",
            )
    try:
        elf = read_elf(output)
    except ValueError as error:
        report.fail("parameter_locations", f"{label}: {error}")
        return
    parsed = units(elf)
    if not parsed:
        report.fail("parameter_locations", f"{label}: the object has no unit")
        return
    # The 32-bit frame record is one four-byte return address.
    facts["frame_record"] = 8
    check_parameter_locations(report, elf, parsed, label, facts)
    report.measurements.setdefault("parameter_locations", {})[label] = "checked"


def run_32_bit(
    report: Report, lccc: list[str], probes: dict[str, Path], workdir: Path,
    gcc: list[str] | None = None,
) -> None:
    """The same object-level contract on a 32-bit target, where the host allows it.

    Three of the campaign's rules live in the target-independent half of the
    compiler (lowering, the IR reachability pass, the DIE writer), so a defect
    there would show on every architecture; checking a second target is what
    turns "should be identical" into evidence. The address-based checks are not
    repeated: a host without 32-bit runtime libraries cannot link a *-m32*
    program, and inventing addresses from an unlinked object would test the
    reader, not the compiler.
    """
    capability = workdir / "capability.c"
    capability.write_text("int pq_probe(void) { return 0; }\n")
    probe_output = workdir / "capability.o"
    attempt = subprocess.run(
        [*lccc, "-m32", "-c", "-o", str(probe_output), str(capability)],
        cwd=workdir, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
        timeout=120,
    )
    if attempt.returncode != 0:
        print(
            "note: -m32 is not usable on this host, skipping the 32-bit matrix",
            file=sys.stderr,
        )
        return
    # The address agreement, measured on the target itself.
    run_32_bit_parameters(report, lccc, workdir, gcc)
    for tier in ("-O0", "-O1"):
        label = f"m32-dwarf5-{tier}"
        output = workdir / f"{label}.o"
        if not build(
            report, lccc, ["-m32", "-g", "-gdwarf-5", tier], [probes["probe"]], output, workdir,
            link=False,
        ):
            continue
        try:
            elf = read_elf(output)
        except ValueError as error:
            report.fail("elf", f"{label}: {error}")
            continue
        check_object_unit(report, elf, 5, label)
        check_static_policy(report, elf, label, tier.lstrip("-"))


def oracle_comparison(
    report: Report, lccc: list[str], gcc: list[str], probes: dict[str, Path], workdir: Path
) -> None:
    """Where GCC 16 is installed, compare the tier rule and the DIE shape to it.

    Only structural facts are compared (a symbol's presence, a DIE's presence),
    never text or sizes: two compilers legitimately disagree about those.
    """
    rows: dict[str, object] = {}
    for tier in ("-O0", "-O1"):
        for name, command in (("lccc", lccc), ("gcc", gcc)):
            output = workdir / f"oracle-{name}-{tier.lstrip('-')}.elf"
            if not build(report, command, ["-g", "-gdwarf-5", tier, "-no-pie"], [probes["probe"]],
                         output, workdir):
                continue
            try:
                elf = read_elf(output)
            except ValueError as error:
                report.fail("oracle", f"{name} {tier}: {error}")
                continue
            rows[f"{name}{tier}"] = sorted(
                symbol.name
                for symbol in elf.symbols
                if symbol.defined and symbol.name.startswith("pq_")
                and symbol.symbol_type in (STT_FUNC, STT_OBJECT)
            )
    for tier in ("-O0", "-O1"):
        mine = rows.get(f"lccc{tier}")
        theirs = rows.get(f"gcc{tier}")
        if mine is None or theirs is None:
            continue
        if mine != theirs:
            report.fail(
                "oracle_symbols",
                f"{tier}: lccc emits {mine}, GCC 16.2 emits {theirs}; the internal-definition "
                "policy is a GCC behaviour, not a choice",
            )
    report.measurements["oracle_symbols"] = rows


def self_test() -> int:
    """Prove the decoders on hand-built bytes, so the gate can be trusted."""
    failures: list[str] = []

    def expect(condition: bool, message: str) -> None:
        if not condition:
            failures.append(message)

    # LEB128 round trip, including the width boundaries.
    for value in (0, 1, 63, 64, 127, 128, 300, 624485, 2**32 - 1):
        encoded = bytearray()
        rest = value
        while True:
            byte = rest & 0x7F
            rest >>= 7
            if rest:
                encoded.append(byte | 0x80)
            else:
                encoded.append(byte)
                break
        decoded, after = _uleb(bytes(encoded), 0)
        expect(decoded == value, f"uleb128 {value} decoded as {decoded}")
        expect(after == len(encoded), f"uleb128 {value} consumed {after} bytes")
    negative, _ = _sleb(b"\x7f", 0)
    expect(negative == -1, f"sleb128 0x7f decoded as {negative}")

    # A parameter location is one `DW_OP_fbreg` and a signed offset, and the
    # widths in the corpus reach both the one-byte and the two-byte encoding.
    class _Located:
        def __init__(self, expression: bytes) -> None:
            self.attrs = {DW_AT_LOCATION: expression}

    for expression, wanted in (
        (b"\x91\x70", -16),
        (b"\x91\x00", 0),
        (b"\x91\x08", 8),
        (b"\x91\xb8\x7f", -72),
        (b"\x91\xa8\x7f", -88),
    ):
        decoded = frame_offset(_Located(expression))
        expect(decoded == wanted, f"{expression!r} decoded as {decoded}, wanted {wanted}")
    # Anything that is not exactly one frame offset is not readable, and saying so
    # is what keeps a wrong address out of a debugger.
    for expression in (b"", b"\x91", b"\x91\x80", b"\x03\x00\x00\x00\x00",
                       b"\x91\x08\x91\x10", b"\x9c"):
        decoded = frame_offset(_Located(expression))
        expect(decoded is None, f"{expression!r} decoded as {decoded}, wanted unreadable")

    # A line program with one sequence, one special opcode and an end_sequence.
    header = bytearray()
    header += b"\x00\x00\x00\x00"  # unit_length, patched below
    header += (5).to_bytes(2, "little")
    header += bytes([8, 0])  # address size, segment selector size
    header += b"\x00\x00\x00\x00"  # header_length, patched below
    header += bytes([1, 1, 1])  # minimum, max_ops, default_is_stmt
    header += bytes([0xFB, 14])  # line_base = -5, line_range = 14
    header += bytes([13])  # opcode_base
    header += bytes([0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1])  # standard lengths
    header += bytes([1, 1, 8])  # one directory format pair: path, DW_FORM_string
    header += bytes([1]) + b"src\x00"
    header += bytes([1, 1, 8])  # one file format pair: path, DW_FORM_string
    header += bytes([1]) + b"unit.c\x00"
    program = bytearray()
    program += bytes([0, 9, 2]) + (0x401000).to_bytes(8, "little")  # set_address
    program += bytes([0x02]) + b"\x02"  # advance_pc 2
    program += bytes([13 + (1 - (-5)) % 14])  # special: advance 1 op, line +1
    program += bytes([0, 1, 1])  # end_sequence
    head_length = len(header) - 12
    header[8:12] = head_length.to_bytes(4, "little")
    body = header + program
    body[0:4] = (len(body) - 4).to_bytes(4, "little")
    elf = Elf(sections={".debug_line": bytes(body)}, addresses={".debug_line": 0}, order="little")
    try:
        table = line_table(elf, 0)
        expect(table.version == 5, f"self-test line version {table.version}")
        expect(table.files == ["unit.c"], f"self-test file table {table.files}")
        expect(len(table.rows) == 2, f"self-test rows {len(table.rows)}")
        expect(table.rows[0].address == 0x401002, f"self-test row address {table.rows[0].address:#x}")
        expect(table.rows[0].line == 2, f"self-test row line {table.rows[0].line}")
        expect(table.rows[1].end_sequence, "self-test last row must end the sequence")
        spans = table.spans()
        expect(spans[0][1] == 0, f"self-test first row width {spans[0][1]}")
    except (ValueError, IndexError, struct.error) as error:
        failures.append(f"self-test line program: {error}")

    # An address-range set with one tuple and a terminator.
    arange = bytearray()
    arange += b"\x00\x00\x00\x00"
    arange += (2).to_bytes(2, "little")
    arange += (0x10).to_bytes(4, "little")  # debug_info offset
    arange += bytes([8, 0])
    arange += b"\x00\x00\x00\x00"  # padding to the tuple size
    arange += (0x401000).to_bytes(8, "little") + (0x20).to_bytes(8, "little")
    arange += bytes(16)
    arange[0:4] = (len(arange) - 4).to_bytes(4, "little")
    elf = Elf(sections={".debug_aranges": bytes(arange)}, addresses={}, order="little")
    try:
        sets = arange_sets(elf)
        expect(len(sets) == 1, f"self-test arange sets {len(sets)}")
        expect(sets[0].info_offset == 0x10, "self-test arange unit offset")
        expect(sets[0].tuples == [(0x401000, 0x20)], f"self-test arange tuples {sets[0].tuples}")
    except (ValueError, IndexError) as error:
        failures.append(f"self-test aranges: {error}")

    # One CIE and one FDE, absolute encoding.
    eh = bytearray()
    cie = bytearray()
    cie += b"\x00\x00\x00\x00"  # CIE_id
    cie += bytes([1]) + b"zR\x00"  # version, augmentation "zR"
    cie += b"\x01" + b"\x78"  # code alignment 1, data alignment -8
    cie += b"\x10"  # return address register
    cie += b"\x01" + b"\x1b"  # augmentation length 1, encoding pcrel|sdata4
    cie += b"\x0c\x1f\x04\x00"  # DW_CFA_def_cfa rsp, 8 etc. (skipped by the reader)
    eh += (len(cie)).to_bytes(4, "little") + cie
    cie_start = 0
    fde_start = 4 + len(cie)
    identity_address = fde_start + 4
    fde = bytearray()
    fde += (identity_address - cie_start).to_bytes(4, "little")  # back-pointer to the CIE
    begin_field = len(fde)  # relative to the FDE's own identity field
    fde += bytes(4)  # pc_begin, patched below
    fde += (0x20).to_bytes(4, "little")  # pc_range, sdata4
    fde += b"\x00"  # augmentation length
    eh += (len(fde)).to_bytes(4, "little") + fde
    # pc_begin is pcrel: what is stored is target minus the field's own address
    # inside the section, which is where the reader starts counting.
    target = 0x401000
    absolute_field = identity_address + begin_field
    eh[absolute_field : absolute_field + 4] = (target - absolute_field).to_bytes(
        4, "little", signed=True
    )
    elf = Elf(
        sections={".eh_frame": bytes(eh)},
        addresses={".eh_frame": 0},
        order="little",
        address_size=8,
    )
    try:
        entries = fdes(elf)
        expect(len(entries) == 1, f"self-test FDE count {len(entries)}")
        expect(entries[0].begin == target, f"self-test FDE begin {entries[0].begin:#x}")
        expect(entries[0].end == target + 0x20, f"self-test FDE end {entries[0].end:#x}")
    except (ValueError, IndexError) as error:
        failures.append(f"self-test eh_frame: {error}")

    # Range lists, version 5: a twelve-byte header, then one list of
    # `DW_RLE_start_length` entries.
    rng = bytearray()
    body = bytearray()
    body += (5).to_bytes(2, "little") + bytes([8, 0]) + (0).to_bytes(4, "little")
    for start, length in ((0x401000, 0x20), (0x401040, 0x10)):
        body += bytes([0x07]) + start.to_bytes(8, "little") + bytes([length])
    body += bytes([0x00])
    rng += (len(body)).to_bytes(4, "little") + body
    elf = Elf(sections={".debug_rnglists": bytes(rng)}, addresses={}, order="little")
    try:
        lists = range_lists(elf, 5)
        expect(len(lists) == 1, f"self-test range list count {len(lists)}")
        expect(
            lists[0].ranges == [(0x401000, 0x401020), (0x401040, 0x401050)],
            f"self-test range list {lists[0].ranges}",
        )
        expect(lists[0].offset == 12, f"self-test range list offset {lists[0].offset}")
    except (ValueError, IndexError) as error:
        failures.append(f"self-test range lists (v5): {error}")

    # Range lists, DWARF 2..4: bare pairs, `(0, 0)` terminates, `(-1, base)`
    # rebases what follows.
    old_form = bytearray()
    old_form += (0x100000).to_bytes(8, "little") + (0x100020).to_bytes(8, "little")
    old_form += ((1 << 64) - 1).to_bytes(8, "little") + (0x200000).to_bytes(8, "little")
    old_form += (0x10).to_bytes(8, "little") + (0x18).to_bytes(8, "little")
    old_form += bytes(16)
    elf = Elf(sections={".debug_ranges": bytes(old_form)}, addresses={}, order="little")
    try:
        lists = range_lists(elf, 4)
        expect(len(lists) == 1, f"self-test range list count (v4) {len(lists)}")
        expect(
            lists[0].ranges == [(0x100000, 0x100020), (0x200010, 0x200018)],
            f"self-test range list (v4) {lists[0].ranges}",
        )
    except (ValueError, IndexError) as error:
        failures.append(f"self-test range lists (v4): {error}")

    # A dropped DIE is only dropped when its name is gone from the symbol table.
    expect(is_dropped_die(0, 0x28, "gone", set()), "self-test dropped DIE not detected")
    expect(
        not is_dropped_die(0, 0x28, "live", {"live"}),
        "self-test: a live function at address zero must stay a defect",
    )
    expect(
        not is_dropped_die(0x401000, 0x28, "gone", set()),
        "self-test: a real address is never a dropped DIE",
    )

    # Relocation application: a `.debug_str` reference on disk is zero plus an
    # addend, exactly like a real object.
    target = bytearray(b"\x00" * 12)
    rela = bytearray()
    rela += (4).to_bytes(8, "little")  # r_offset
    rela += ((0 << 32) | 10).to_bytes(8, "little")  # symbol 0, R_X86_64_32
    rela += (8).to_bytes(8, "little", signed=True)  # addend: the real offset
    elf = Elf(
        sections={".debug_str": bytes(target), ".rela.debug_str": bytes(rela)},
        addresses={},
        symbols=[Symbol(name=".debug_str", value=0, size=0, info=0, shndx=1)],
        order="little",
    )
    _apply_relocations(elf, 2, "little")
    resolved_value = int.from_bytes(elf.sections[".debug_str"][4:8], "little")
    expect(resolved_value == 8, f"self-test relocation produced {resolved_value}")

    # The source scanner the attribution check rests on.
    def span_names(source: str) -> list[tuple[str, int, int]]:
        return [(span.name, span.first_line, span.last_line) for span in function_spans(source)]

    basic = span_names("int a(void) {\n    return 1;\n}\nstatic int b(void) {\n    return 2;\n}\n")
    expect(basic == [("a", 1, 3), ("b", 4, 6)], f"self-test spans {basic}")
    # Attributes before and after the return type are not part of the name, and
    # an indented line is never a definition; a prototype is not one either.
    expect(
        span_names(
            "__attribute__((noinline)) int a(void) {\n"
            "    return 1;\n"
            "}\n"
            "int __attribute__((noinline)) b(void) {\n"
            "    if (1) {\n"
            "        return 2;\n"
            "    }\n"
            "    return 3;\n"
            "}\n"
            "int c(void);\n"
        )
        == [("a", 1, 3), ("b", 4, 9)],
        "self-test spans with attributes",
    )

    if failures:
        for failure in failures:
            print(f"SELF-TEST FAILURE: {failure}", file=sys.stderr)
        return 1
    print("self-test: decoder, line program, aranges, eh_frame and source scanner agree")
    return 0


def markdown(report: Report, lccc_version: str, gcc_version: str | None) -> str:
    lines = [
        "# DWARF quality gate",
        "",
        f"- lccc: `{lccc_version}`",
        f"- oracle: {'`' + gcc_version + '`' if gcc_version else 'none (structural checks only)'}",
        f"- objects built: {report.objects}",
        f"- failures: {len(report.failures)}",
        f"- known gaps reproduced: {len(report.known)}",
        "",
    ]
    if report.failures:
        lines.append("## Failures")
        lines.append("")
        lines += [f"- {failure}" for failure in report.failures]
        lines.append("")
    else:
        lines += ["## Failures", "", "None.", ""]
    lines += ["## Known gaps", ""]
    if report.known:
        lines += [f"- **{key}**: {detail}" for key, detail in sorted(report.known.items())]
    else:
        lines.append("None: every gap in `KNOWN_GAPS` is closed, delete the entries.")
    lines.append("")
    if report.healed:
        lines += ["## Gaps that stopped reproducing", ""]
        lines += [f"- {key}" for key in report.healed]
        lines.append("")
    lines += ["## Measurements", "", "```json", json.dumps(report.measurements, indent=2), "```", ""]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default="target/fastbuild/lccc", help="lccc binary to test")
    parser.add_argument("--gcc", default=None, help="GCC 16 to compare tier policy against")
    parser.add_argument("--json", default=None, help="write the report as JSON here")
    parser.add_argument("--markdown", default=None, help="write the report as markdown here")
    parser.add_argument("--self-test", action="store_true", help="test the decoders, not a compiler")
    parser.add_argument("--keep", action="store_true", help="keep the temporary directory")
    arguments = parser.parse_args()
    if arguments.self_test:
        return self_test()

    lccc = [str(Path(arguments.lccc).resolve())]
    if not Path(lccc[0]).exists():
        print(f"error: {lccc[0]} does not exist", file=sys.stderr)
        return 2
    gcc = None
    if arguments.gcc:
        resolved = shutil.which(arguments.gcc)
        if resolved:
            gcc = [resolved]
        else:
            print(f"note: {arguments.gcc} not found, structural checks only", file=sys.stderr)

    workdir = Path(tempfile.mkdtemp(prefix="dwarf-quality-"))
    (workdir / "m32").mkdir()
    try:
        report = run_gate(lccc, gcc, workdir)
        run_32_bit(report, lccc, write_probes(workdir / "m32"), workdir / "m32", gcc)
        gdb = shutil.which("gdb")
        if gdb:
            run_debugger_lane(report, lccc, gdb, workdir)
        else:
            print("note: gdb not found, the debugger lane is skipped", file=sys.stderr)
        versions = {}
        for name, command in (("lccc", lccc), ("gcc", gcc)):
            if command:
                proc = subprocess.run(
                    [*command, "--version"], text=True, stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE, check=False, timeout=60,
                )
                first = (proc.stdout or proc.stderr).strip().splitlines()
                versions[name] = first[0] if first else "unknown"
        document = markdown(report, versions.get("lccc", "unknown"), versions.get("gcc"))
        if arguments.markdown:
            Path(arguments.markdown).write_text(document)
        if arguments.json:
            Path(arguments.json).write_text(
                json.dumps(
                    {
                        "gate_date": GATE_DATE,
                        "lccc": versions.get("lccc"),
                        "gcc": versions.get("gcc"),
                        "objects": report.objects,
                        "failures": [
                            {"check": failure.check, "detail": failure.detail}
                            for failure in report.failures
                        ],
                        "known_gaps": report.known,
                        "healed_gaps": report.healed,
                        "measurements": report.measurements,
                    },
                    indent=2,
                )
                + "\n"
            )
        if report.failures:
            for failure in report.failures:
                print(f"FAIL {failure}", file=sys.stderr)
            print(
                f"{len(report.failures)} DWARF quality failure(s) across {report.objects} objects",
                file=sys.stderr,
            )
            return 1
        print(
            f"DWARF quality gate: {report.objects} objects, 0 failures, "
            f"{len(report.known)} known gap(s)"
        )
        for key, detail in sorted(report.known.items()):
            print(f"  known gap [{key}]: {detail}")
        return 0
    finally:
        if not arguments.keep:
            shutil.rmtree(workdir, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
