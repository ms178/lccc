#!/usr/bin/env python3
"""Pin the diagnostics that separate the C17 and C23 dialects.

`check_c23_inventory.py` answers "does this translation unit compile in this
dialect, and does the answer agree with GCC".  That is the right question for a
feature matrix, but it says nothing about *how* a rejected program is rejected:
a compiler that fails a `-std=c17` build with

    expected ';' after return statement before character constant

is technically correct and practically useless, and one that rejects a short
macro invocation by substituting an empty token sequence is not even correct --
it compiles a program the user did not write.

This gate therefore pins, for each dialect-sensitive diagnostic this compiler
produces:

  * the verdict (accept/reject) *and* the verdict GCC 16.2 produces, so a pinned
    expectation cannot silently rot;
  * the message text this compiler must emit (a substring, because message tails
    legitimately carry names);
  * the message text GCC 16.2 emits, for the same reason from the other side.

Deliberate divergences are listed in `DIVERGENCES` rather than hidden: GCC's
pluralisation ("passed 1 arguments") is a grammar bug this compiler does not
copy, and GCC's cascade of follow-on errors after a macro-arity error is noise
this compiler does not emit.

The second half pins debug information: a C17 and a C23 object must carry the
`DW_AT_language_version` a debugger needs to know which revision it is looking
at (201710L and 202311L), matching GCC byte for byte in the attributes that
identify the dialect.  Those attributes are read from the object bytes -- ELF
section table, compile-unit header, abbreviation table, DIE -- because no
version of `readelf` renders them dependably, and a check that reads the tool
instead of the object reports "absent" for both compilers and then compares two
absences.  `readelf` is kept as a second opinion that has to agree.

Nothing here is guessed: every expectation was recorded on x86-64 Linux from
GCC 16.2.0, which the gate re-measures on every run, with Clang 23.1.4
consulted as a second opinion where it implements the C23 spelling (it does not
implement `u8'x'`, so it cannot be the oracle for the whole boundary).
"""

from __future__ import annotations

import argparse
import json
import re
import shlex
import struct
import subprocess
import sys
import tempfile
import zlib
from dataclasses import dataclass, field
from pathlib import Path

GATE_DATE = "2026-11-04"


@dataclass(frozen=True)
class Case:
    """One dialect-sensitive diagnostic."""

    name: str
    dialect: str
    source: str
    #: "accept" or "reject" -- the verdict both compilers must produce.
    expect: str
    #: Substrings this compiler's diagnostics must contain.
    lccc_messages: tuple[str, ...] = ()
    #: Substrings GCC's diagnostics must contain (empty = verdict only).
    oracle_messages: tuple[str, ...] = ()
    #: Why this compiler's wording may differ from GCC's, when it does.
    divergence: str = ""


CASES: tuple[Case, ...] = (
    Case(
        name="implicit-int-c89-is-silent",
        dialect="c89",
        # C89 keeps implicit int, and GCC 16.2 neither accepts nor rejects this
        # form loudly: it compiles `static counter = 5;` without a word. Pinned
        # because "implicit int is always an error now" is a tempting and wrong
        # simplification.
        source="static counter = 5;\nint f(void) { return counter; }\n",
        expect="accept",
    ),
    Case(
        name="implicit-int-c17-is-an-error",
        dialect="c17",
        source="static counter = 5;\nint f(void) { return counter; }\n",
        expect="reject",
        lccc_messages=("type defaults to 'int'", "[-Wimplicit-int]"),
        oracle_messages=("type defaults to 'int'",),
    ),
    Case(
        name="implicit-int-gnu17-is-an-error",
        dialect="gnu17",
        source="register local;\nint f(void) { return 0; }\n",
        expect="reject",
        lccc_messages=("type defaults to 'int'", "[-Wimplicit-int]"),
        oracle_messages=("type defaults to 'int'",),
    ),
    Case(
        name="implicit-int-c23-is-an-error",
        dialect="c23",
        source="static counter = 5;\nint f(void) { return counter; }\n",
        expect="reject",
        lccc_messages=("type defaults to 'int'", "[-Wimplicit-int]"),
        oracle_messages=("type defaults to 'int'",),
    ),
    Case(
        name="macro-arity-too-few",
        dialect="c17",
        source="#define PAIR(a, b) ((a) + (b))\nint f(void) { return PAIR(1); }\n",
        expect="reject",
        lccc_messages=("macro 'PAIR' requires 2 arguments, but only 1 given",),
        oracle_messages=("macro 'PAIR' requires 2 arguments, but only 1 given",),
        divergence=(
            "GCC follows the arity error with \"'PAIR' undeclared\", a cascade "
            "from the failed expansion; this compiler stops at the cause."
        ),
    ),
    Case(
        name="macro-arity-too-many",
        dialect="c17",
        source="#define ONE(a) (a)\nint f(void) { return ONE(1, 2); }\n",
        expect="reject",
        lccc_messages=("macro 'ONE' passed 2 arguments, but takes just 1",),
        oracle_messages=("macro 'ONE' passed 2 arguments, but takes just 1",),
    ),
    Case(
        name="macro-arity-zero-parameter",
        dialect="c17",
        source="#define NONE() 7\nint f(void) { return NONE(1); }\n",
        expect="reject",
        lccc_messages=("macro 'NONE' passed 1 argument, but takes just 0",),
        oracle_messages=("macro 'NONE' passed 1 arguments, but takes just 0",),
        divergence=(
            "GCC prints \"1 arguments\"; this compiler prints the singular form. "
            "The wording is the only difference -- the verdict and the counts "
            "agree."
        ),
    ),
    Case(
        name="macro-empty-argument-is-legal",
        dialect="c17",
        source="#define ONE(a) 0\nint f(void) { return ONE(); }\n",
        expect="accept",
    ),
    Case(
        name="macro-variadic-tail-may-be-empty",
        dialect="c17",
        source="#define TAIL(a, ...) (a)\nint f(void) { return TAIL(1) + TAIL(2, 3); }\n",
        expect="accept",
    ),
    Case(
        name="elifdef-is-invalid-before-c23",
        dialect="c17",
        source="#define X 1\n#ifdef X\nint a;\n#elifdef X\nint b;\n#endif\n",
        expect="reject",
        lccc_messages=("invalid preprocessing directive #elifdef",),
        oracle_messages=("invalid preprocessing directive #elifdef",),
    ),
    Case(
        name="elifdef-skipped-group-stays-skipped",
        dialect="c17",
        # The group is already skipped, so GCC says nothing and does not take
        # the branch -- the `#else` runs. A compiler that took the branch would
        # compile a different program than GCC; one that diagnosed the spelling
        # here would reject a translation unit GCC accepts.
        source=(
            "#define X 1\n"
            "#if 0\n"
            "#elifdef X\n"
            "#error the skipped branch must not run\n"
            "#else\n"
            "int skipped_branch_skipped;\n"
            "#endif\n"
            "int f(void) { return 0; }\n"
        ),
        expect="accept",
    ),
    Case(
        name="elifdef-is-a-gnu-extension",
        dialect="gnu17",
        source="#define X 1\n#if 0\n#elifdef X\nint taken;\n#else\n#error\n#endif\n",
        expect="accept",
    ),
    Case(
        name="elifdef-in-c23",
        dialect="c23",
        source="#define X 1\n#if 0\n#elifdef X\nint taken;\n#else\n#error\n#endif\n",
        expect="accept",
    ),
    Case(
        name="va-start-one-argument-needs-c23",
        dialect="c17",
        source=(
            "#include <stdarg.h>\n"
            "static int f(int named, ...) { va_list ap; va_start(ap);\n"
            "  int v = va_arg(ap, int); va_end(ap); return v + named; }\n"
            "int main(void) { return f(0, 9); }\n"
        ),
        expect="reject",
        lccc_messages=("macro 'va_start' requires 2 arguments, but only 1 given",),
        oracle_messages=("macro 'va_start' requires 2 arguments, but only 1 given",),
    ),
    Case(
        name="va-start-one-argument-needs-c23-gnu17",
        dialect="gnu17",
        source=(
            "#include <stdarg.h>\n"
            "static int f(int named, ...) { va_list ap; va_start(ap);\n"
            "  int v = va_arg(ap, int); va_end(ap); return v + named; }\n"
            "int main(void) { return f(0, 9); }\n"
        ),
        expect="reject",
        lccc_messages=("macro 'va_start' requires 2 arguments, but only 1 given",),
        oracle_messages=("macro 'va_start' requires 2 arguments",),
    ),
    Case(
        name="va-start-one-argument-in-c23",
        dialect="c23",
        source=(
            "#include <stdarg.h>\n"
            "static int f(int named, ...) { va_list ap; va_start(ap);\n"
            "  int v = va_arg(ap, int); va_end(ap); return v + named; }\n"
            "int main(void) { return f(0, 9); }\n"
        ),
        expect="accept",
    ),
    Case(
        name="u8-character-constant-needs-c23",
        dialect="c17",
        source="int f(void) { return u8'a'; }\n",
        expect="reject",
        divergence=(
            "Both compilers reject the program; GCC reports \"'u8' undeclared\" "
            "and then a follow-on syntax error where this compiler reports the "
            "syntax error alone. The lexical split is the same -- `u8` is an "
            "identifier and `'a'` a separate constant -- so only the verdict is "
            "pinned here."
        ),
    ),
    Case(
        name="u8-character-constant-in-c23",
        dialect="c23",
        source="int f(void) { return u8'a'; }\n",
        expect="accept",
    ),
    Case(
        name="static-assert-needs-c23",
        dialect="c17",
        source="static_assert(1, \"ok\");\n",
        expect="reject",
        divergence=(
            "GCC reads `static_assert` as a function call and reports a "
            "declaration-specifier error; the verdict is what is claimed here."
        ),
    ),
    Case(
        name="static-assert-in-c23",
        dialect="c23",
        source="static_assert(1, \"ok\");\nint f(void) { return 0; }\n",
        expect="accept",
    ),
    Case(
        name="alignas-identifier-is-legal-in-c17",
        dialect="c17",
        source="int alignas = 4;\nint alignof = 3;\nint f(void) { return alignas + alignof; }\n",
        expect="accept",
    ),
)

#: `(dialect, DW_AT_language, DW_AT_language_version)`, as both compilers emit
#: them.  `DW_AT_language` is the DWARF *language*, not the revision: C89 is 1,
#: C99 is 12, and everything from C11 on is 29 (`DW_LANG_C11`).  The revision
#: itself lives in `DW_AT_language_version`, which DWARF 5 omits when it equals
#: the language's default (199901 for `DW_LANG_C99`, 201112 for `DW_LANG_C11`) --
#: measured on GCC 16.2, which is why a `-std=c11` object carries no version
#: attribute at all while a `-std=c17` one must carry 201710.  A debugger recovers
#: the revision from those two numbers, so both are pinned here per dialect.
DWARF_IDENTITY: tuple[tuple[str, str, str | None], ...] = (
    ("c89", "1", None),
    ("c99", "12", None),
    ("c11", "29", None),
    ("c17", "29", "201710"),
    ("gnu17", "29", "201710"),
    ("c23", "29", "202311"),
    ("gnu23", "29", "202311"),
)


def run(command: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        timeout=120,
    )


def resolve(command: str) -> list[str]:
    """Split a command line and make a relative program path absolute.

    Every case compiles inside its own temporary directory, so a relative
    `target/fastbuild/lccc` would resolve against that directory instead of the
    checkout.
    """
    parts = shlex.split(command)
    if parts and ("/" in parts[0] or Path(parts[0]).exists()):
        parts[0] = str(Path(parts[0]).resolve())
    return parts


def compiler_version(command: list[str]) -> str:
    proc = run([*command, "-dumpfullversion", "-dumpversion"], Path.cwd())
    return (proc.stdout or proc.stderr).strip()


def diagnose(command: list[str], dialect: str, source: Path, cwd: Path) -> dict[str, object]:
    argv = [*command, f"-std={dialect}", "-fsyntax-only", "-x", "c", source.name]
    proc = run(argv, cwd)
    return {
        "verdict": "accept" if proc.returncode == 0 else "reject",
        "exit_code": proc.returncode,
        "stderr": proc.stderr[-4000:],
        "argv": argv,
    }


def _decimal_from_attribute(rest: str) -> str | None:
    """The decimal value of a `DW_AT_*` constant, however readelf spelled it.

    Binutils prints a DWARF constant as `0x313ee\t(201710)` (the encoding and
    the value), older binutils prints just `0x313ee` or just `201710`, and
    llvm-readelf prints `(0x313ee)`. A parser that knows only one of those
    silently reads "attribute absent" on the other toolchain -- which is exactly
    how this gate first failed on a runner whose readelf omits the parenthesised
    value -- so all four renderings are accepted here, preferring the decimal.
    """
    parenthesised = re.search(r"\((\d+)\)", rest)
    if parenthesised:
        return parenthesised.group(1)
    hexadecimal = re.search(r"0x([0-9a-fA-F]+)", rest)
    if hexadecimal:
        return str(int(hexadecimal.group(1), 16))
    decimal = re.search(r"\b(\d+)\b", rest)
    if decimal:
        return decimal.group(1)
    return None


# --- reading the dialect back out of the object -----------------------------
#
# `readelf` is not an oracle for `DW_AT_language_version`.  Its rendering of a
# DWARF constant has changed three times (`0x313ee\t(201710)`, `0x313ee`,
# `(0x313ee)`, plain `201710`), and a binutils that predates DWARF 5 does not
# name the attribute at all -- so a gate that parses readelf's *text* reports
# "attribute absent" for whichever compiler it looks at, and then happily
# compares two absences.  That is precisely how this gate first failed on a
# runner while passing locally: the check was reading the tool, not the object.
#
# The dialect identity of a compile unit is a handful of bytes in
# `.debug_info`, so the gate reads them from the object itself: ELF section
# table, then the compile-unit header, then the abbreviation table and the DIE
# attributes.  `readelf`, when it is installed and its output is understandable,
# is kept as an independent second opinion that has to agree with the bytes.

_DW_AT_LANGUAGE = 0x13
_DW_AT_PRODUCER = 0x25
_DW_AT_LANGUAGE_NAME = 0x90
_DW_AT_LANGUAGE_VERSION = 0x91

# Forms whose payload has a fixed width.  Forms this gate does not interpret
# still have to be *skipped* correctly, or every attribute after them decodes
# from the wrong offset.
_FIXED_FORM_WIDTHS = {
    0x05: 2,  # DW_FORM_data2
    0x06: 4,  # DW_FORM_data4
    0x07: 8,  # DW_FORM_data8
    0x0B: 1,  # DW_FORM_data1
    0x0C: 1,  # DW_FORM_flag
    0x11: 1,  # DW_FORM_ref1
    0x12: 2,  # DW_FORM_ref2
    0x13: 4,  # DW_FORM_ref4
    0x14: 8,  # DW_FORM_ref8
    0x19: 0,  # DW_FORM_flag_present
    0x1C: 4,  # DW_FORM_ref_sup4
    0x1E: 16,  # DW_FORM_data16
    0x20: 8,  # DW_FORM_ref_sig8
    0x24: 8,  # DW_FORM_ref_sup8
    0x25: 1,  # DW_FORM_strx1
    0x26: 2,  # DW_FORM_strx2
    0x27: 3,  # DW_FORM_strx3
    0x28: 4,  # DW_FORM_strx4
    0x29: 1,  # DW_FORM_addrx1
    0x2A: 2,  # DW_FORM_addrx2
    0x2B: 3,  # DW_FORM_addrx3
    0x2C: 4,  # DW_FORM_addrx4
}


_SHF_COMPRESSED = 0x800  # the section holds an ELF compression header


def _decompress(payload: bytes, elf_class: int, order: str, name: str) -> bytes:
    """The contents of a `SHF_COMPRESSED` section.

    An assembler asked for `--compress-debug-sections=zlib` stores `.debug_info`
    and friends as an `Elf64_Chdr` (or `Elf32_Chdr`) followed by a zlib stream.
    A reader that ignores the flag decodes the compression header as if it were
    the start of the section -- which is exactly how this reader first failed on
    GCC's objects while reading this compiler's own uncompressed ones.
    """
    word = order  # "little" or "big"
    if elf_class == 2:
        chunk_type = int.from_bytes(payload[0:4], word)
        chunk_size = int.from_bytes(payload[8:16], word)
        body = payload[24:]
    else:
        chunk_type = int.from_bytes(payload[0:4], word)
        chunk_size = int.from_bytes(payload[4:8], word)
        body = payload[12:]
    if chunk_type != 1:  # ELFCOMPRESS_ZLIB
        raise ValueError(f"{name} uses compression type {chunk_type}, which is not zlib")
    expanded = zlib.decompress(body)
    if len(expanded) != chunk_size:
        raise ValueError(f"{name} expands to {len(expanded)} bytes, header says {chunk_size}")
    return expanded


@dataclass
class _Elf:
    """The named sections of an ELF object, plus its byte order."""

    sections: dict[str, bytes]
    order: str


def _elf_sections(path: Path) -> _Elf:
    blob = path.read_bytes()
    if blob[:4] != b"\x7fELF":
        raise ValueError("not an ELF object")
    elf_class, encoding = blob[4], blob[5]
    if encoding not in (1, 2):
        raise ValueError(f"unknown ELF encoding {encoding}")
    order = "little" if encoding == 1 else "big"
    mark = "<" if order == "little" else ">"
    if elf_class == 2:
        table = int.from_bytes(blob[0x28:0x30], order)
        entry_size = int.from_bytes(blob[0x3A:0x3C], order)
        count = int.from_bytes(blob[0x3C:0x3E], order)
        names_index = int.from_bytes(blob[0x3E:0x40], order)
        header = struct.Struct(mark + "IIQQQQIIQQ")
    elif elf_class == 1:
        table = int.from_bytes(blob[0x20:0x24], order)
        entry_size = int.from_bytes(blob[0x2E:0x30], order)
        count = int.from_bytes(blob[0x30:0x32], order)
        names_index = int.from_bytes(blob[0x32:0x34], order)
        header = struct.Struct(mark + "IIIIIIIIII")
    else:
        raise ValueError(f"unknown ELF class {elf_class}")
    if entry_size < header.size:
        raise ValueError("section header table entry is too small")

    def section(index: int) -> tuple[int, ...]:
        return header.unpack_from(blob, table + index * entry_size)

    if count == 0:  # extended numbering: the real count is in section 0
        count = section(0)[5]
    if names_index == 0xFFFF:  # SHN_XINDEX: the real index is in section 0
        names_index = section(0)[6]
    names_start, names_size = section(names_index)[4], section(names_index)[5]
    names = blob[names_start : names_start + names_size]
    sections: dict[str, bytes] = {}
    for index in range(count):
        fields = section(index)
        name_start = fields[0]
        name_end = names.find(b"\x00", name_start)
        name = names[name_start : name_end if name_end >= 0 else len(names)]
        start, size, flags = fields[4], fields[5], fields[2]
        payload = blob[start : start + size]
        if flags & _SHF_COMPRESSED and size:
            payload = _decompress(payload, elf_class, order, name.decode("ascii", "replace"))
        sections[name.decode("ascii", "replace")] = payload
    return _Elf(sections=sections, order=order)


def _uleb(data: bytes, offset: int) -> tuple[int, int]:
    value = shift = 0
    while True:
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if not byte & 0x80:
            return value, offset


def _sleb(data: bytes, offset: int) -> tuple[int, int]:
    value = shift = 0
    while True:
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if not byte & 0x80:
            if byte & 0x40:
                value -= 1 << shift
            return value, offset


def _encode_uleb(value: int) -> bytes:
    """The inverse of `_uleb`, for the objects the self-test assembles."""
    encoded = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            encoded.append(byte | 0x80)
        else:
            encoded.append(byte)
            return bytes(encoded)


def _abbrev_table(abbrev: bytes, offset: int) -> dict[int, list[tuple[int, int, int | None]]]:
    """Abbreviation declarations: code -> [(attribute, form, implicit value)]."""
    table: dict[int, list[tuple[int, int, int | None]]] = {}
    while True:
        code, offset = _uleb(abbrev, offset)
        if code == 0:
            return table
        _, offset = _uleb(abbrev, offset)  # DW_TAG_*
        offset += 1  # DW_CHILDREN_*
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
        table[code] = attributes


def _byte_string(section: bytes, start: int) -> str:
    if start >= len(section):
        return ""
    end = section.find(b"\x00", start)
    return section[start : end if end >= 0 else len(section)].decode("utf-8", "replace")


def _form_value(
    form: int,
    data: bytes,
    offset: int,
    elf: _Elf,
    address_size: int,
    offset_size: int,
    implicit: int | None,
) -> tuple[object, int]:
    """One attribute payload: `(value, offset after it)`."""
    order = elf.order
    if form == 0x21:  # DW_FORM_implicit_const: the value lives in the abbrev
        return implicit, offset
    width = _FIXED_FORM_WIDTHS.get(form)
    if width is not None:
        payload = data[offset : offset + width]
        return (int.from_bytes(payload, order) if width else 1), offset + width
    if form == 0x01:  # DW_FORM_addr
        payload = data[offset : offset + address_size]
        return int.from_bytes(payload, order), offset + address_size
    if form in (0x0E, 0x1F, 0x17, 0x10, 0x1D):  # strp/line_strp/sec_offset/ref_addr/strp_sup
        start = int.from_bytes(data[offset : offset + offset_size], order)
        offset += offset_size
        if form in (0x0E, 0x1D):
            return _byte_string(elf.sections.get(".debug_str", b""), start), offset
        if form == 0x1F:
            return _byte_string(elf.sections.get(".debug_line_str", b""), start), offset
        return start, offset
    if form == 0x0F:  # DW_FORM_udata
        return _uleb(data, offset)
    if form == 0x0D:  # DW_FORM_sdata
        return _sleb(data, offset)
    if form in (0x15, 0x1A, 0x1B, 0x22, 0x23):  # ref_udata/strx/addrx/loclistx/rnglistx
        return _uleb(data, offset)
    if form == 0x08:  # DW_FORM_string
        end = data.find(b"\x00", offset)
        end = len(data) if end < 0 else end
        return data[offset:end].decode("utf-8", "replace"), end + 1
    if form in (0x09, 0x0A, 0x03, 0x04, 0x18):  # block*/exprloc
        if form == 0x0A:  # DW_FORM_block1
            length = data[offset]
            start = offset + 1
        elif form == 0x03:  # DW_FORM_block2
            length = int.from_bytes(data[offset : offset + 2], order)
            start = offset + 2
        elif form == 0x04:  # DW_FORM_block4
            length = int.from_bytes(data[offset : offset + 4], order)
            start = offset + 4
        else:  # DW_FORM_block / DW_FORM_exprloc
            length, start = _uleb(data, offset)
        return data[start : start + length], start + length
    if form == 0x16:  # DW_FORM_indirect
        actual, start = _uleb(data, offset)
        return _form_value(actual, data, start, elf, address_size, offset_size, implicit)
    raise ValueError(f"DWARF form 0x{form:x} is not implemented")


def _compile_unit(elf: _Elf) -> dict[int, object]:
    """Attributes of the first compile unit in `.debug_info`, by attribute code."""
    info = elf.sections.get(".debug_info")
    if info is None:
        raise ValueError("no .debug_info section")
    order = elf.order
    offset_size = 4
    offset = 0
    if int.from_bytes(info[0:4], order) == 0xFFFFFFFF:  # DWARF64
        offset_size = 8
        offset = 12
    else:
        offset = 4
    version = int.from_bytes(info[offset : offset + 2], order)
    offset += 2
    if version >= 5:
        address_size = info[offset + 1]
        offset += 2  # DW_UT_* and the address size
    else:
        address_size = 0  # read below, after the abbreviation offset
    abbrev_offset = int.from_bytes(info[offset : offset + offset_size], order)
    offset += offset_size
    if version < 5:  # DWARF 2..4 put the address size after the offset
        address_size = info[offset]
        offset += 1
    table = _abbrev_table(elf.sections.get(".debug_abbrev", b""), abbrev_offset)
    code, offset = _uleb(info, offset)
    if code not in table:
        raise ValueError(f"abbreviation code {code} is not declared")
    attributes: dict[int, object] = {}
    for attribute, form, implicit in table[code]:
        value, offset = _form_value(form, info, offset, elf, address_size, offset_size, implicit)
        attributes[attribute] = value
    return attributes


@dataclass
class DwarfIdentity:
    """What one object says about its dialect, and how it was established."""

    language: str
    version: str | None
    producer: str
    second_opinion: str
    error: str | None = None

    def key(self) -> tuple[str, str | None]:
        return (self.language, self.version)

    def cell(self) -> str:
        return f"{self.language} / {self.version or 'default'}"


def _constant(value: object) -> str:
    return str(value) if isinstance(value, int) else "<absent>"


def _readelf_identity(object_file: Path, cwd: Path, readelf: list[str]) -> str:
    """`readelf`'s own answer, when it can be understood, for the cross-check."""
    try:
        dumped = run([*readelf, "--debug-dump=info", object_file.name], cwd)
    except OSError as error:  # no readelf on this host: the bytes are enough
        return f"unavailable ({error.strerror})"
    if dumped.returncode != 0:
        return f"unavailable ({dumped.stderr.strip().splitlines()[:1]})"
    text = dumped.stdout
    language_line = re.search(r"DW_AT_language\s*:\s*(.*)$", text, re.MULTILINE)
    version_line = re.search(r"DW_AT_language_version\s*:\s*(.*)$", text, re.MULTILINE)
    if not language_line or not version_line:
        # Not a disagreement: this readelf does not name the attribute.
        return "silent"
    language = _decimal_from_attribute(language_line.group(1))
    version = _decimal_from_attribute(version_line.group(1))
    if language is None or version is None:
        return "silent"
    return f"{language} / {version}"


def dwarf_identity(
    command: list[str],
    dialect: str,
    source: Path,
    cwd: Path,
    readelf: list[str],
    tag: str,
    extra_flags: tuple[str, ...] = (),
) -> DwarfIdentity:
    """Compile one function with `-gdwarf-5` and read its dialect back."""
    suffix = "-32" if extra_flags else ""
    object_file = cwd / f"dwarf-{tag}{suffix}.o"
    compile_argv = [
        *command,
        *extra_flags,
        f"-std={dialect}",
        "-g",
        "-gdwarf-5",
        "-O0",
        "-c",
        "-x",
        "c",
        source.name,
        "-o",
        object_file.name,
    ]
    built = run(compile_argv, cwd)
    if built.returncode != 0:
        first = (built.stderr.strip().splitlines() or [""])[0]
        return DwarfIdentity("<compile failed>", None, "", "", f"compile failed: {first}")
    try:
        attributes = _compile_unit(_elf_sections(object_file))
    except (ValueError, IndexError, struct.error) as error:
        return DwarfIdentity("<unreadable>", None, "", "", f"{object_file.name}: {error}")
    version = attributes.get(_DW_AT_LANGUAGE_VERSION)
    return DwarfIdentity(
        language=_constant(attributes.get(_DW_AT_LANGUAGE)),
        version=str(version) if isinstance(version, int) else None,
        producer=str(attributes.get(_DW_AT_PRODUCER, "")),
        second_opinion=_readelf_identity(object_file, cwd, readelf),
    )


def _synthetic_object(
    path: Path, language: int, version: int | None, order: str, compress: bool = False
) -> None:
    """Write an ELF object carrying one DWARF 5 compile unit.

    The self-test cannot compile anything -- it has to run where no compiler is
    installed -- so the decoder is exercised on an object this function
    assembles: ELF header, section table, `.debug_abbrev`, `.debug_info` and
    `.debug_str`, in either byte order (a big-endian target's object carries the
    same DWARF the other way round) and optionally with the zlib-compressed
    debug sections an assembler writes for `--compress-debug-sections`.
    """
    byteorder = "little" if order == "<" else "big"
    word = "<" if order == "<" else ">"
    attributes = [
        (0x25, 0x0E),  # DW_AT_producer: DW_FORM_strp
        (0x13, 0x05),  # DW_AT_language: DW_FORM_data2
        (0x90, 0x0B),  # DW_AT_language_name: DW_FORM_data1
    ]
    if version is not None:
        attributes.append((0x91, 0x0F))  # DW_AT_language_version: DW_FORM_udata
    abbrev = bytearray([1, 0x11, 1])  # code 1, DW_TAG_compile_unit, has children
    for attribute, form in attributes:
        # Attribute codes 0x90 and 0x91 are above 0x7f: they need two bytes.
        abbrev.extend(_encode_uleb(attribute) + _encode_uleb(form))
    abbrev.extend((0, 0, 0))  # end of attributes, end of table

    die = bytearray([1])
    die.extend((0).to_bytes(4, byteorder))  # DW_AT_producer -> .debug_str[0]
    die.extend(language.to_bytes(2, byteorder))
    die.append(3)  # DW_LNAME_C
    if version is not None:
        die.extend(_encode_uleb(version))
    unit = bytearray((5).to_bytes(2, byteorder))  # DWARF version
    unit.append(1)  # DW_UT_compile
    unit.append(8)  # address size
    unit.extend((0).to_bytes(4, byteorder))  # abbreviation offset
    unit.extend(die)
    info = bytearray(len(unit).to_bytes(4, byteorder))
    info.extend(unit)

    names = b"\x00.shstrtab\x00.debug_abbrev\x00.debug_info\x00.debug_str\x00"
    payloads = [b"", names, bytes(abbrev), bytes(info), b"synthetic\x00"]
    flags = [0, 0, 0, 0, 0]
    if compress:
        for index in range(2, len(payloads)):
            payload = payloads[index]
            payloads[index] = (
                struct.pack(word + "IIQQ", 1, 0, len(payload), 1) + zlib.compress(payload)
            )
            flags[index] = _SHF_COMPRESSED
    body = bytearray()
    offsets = []
    for payload in payloads:
        offsets.append(64 + len(body))
        body.extend(payload)
        body.extend(bytes(-len(body) % 8))
    table = 64 + len(body)
    ident = b"\x7fELF" + bytes([2, 1 if order == "<" else 2, 1, 0]) + bytes(8)
    header = ident + struct.pack(
        word + "HHIQQQIHHHHHH", 1, 62, 1, 0, 0, table, 0, 64, 0, 0, 64, 5, 1
    )
    section_header = struct.Struct(word + "IIQQQQIIQQ")
    sections = bytearray(64)
    for index, name in enumerate(
        (".shstrtab", ".debug_abbrev", ".debug_info", ".debug_str"), start=1
    ):
        sections.extend(
            section_header.pack(
                names.find(name.encode() + b"\x00"),
                3 if index == 1 else 1,  # SHT_STRTAB, SHT_PROGBITS
                flags[index],
                0,
                offsets[index],
                len(payloads[index]),
                0,
                0,
                8 if flags[index] else 1,
                0,
            )
        )
    path.write_bytes(bytes(header) + bytes(body) + bytes(sections))


def _decoder_self_test() -> None:
    """Check the object reader on objects this file assembles.

    It runs on every invocation, not only under `--self-test`: the reader is
    the part of this gate that can fail silently -- a decoder that reads the
    wrong offset reports `absent` for a compiler that did nothing wrong -- so a
    broken reader has to stop the gate before it measures anything.
    """
    # LEB128, both byte orders, both section-header shapes, compressed and
    # uncompressed debug sections, and the forms a compile unit uses for the
    # dialect constants.
    assert _uleb(b"\x0b", 0) == (11, 1)
    assert _uleb(b"\xe5\x8e\x26", 0) == (624485, 3)
    assert _sleb(b"\x7f", 0) == (-1, 1)
    assert _sleb(b"\x3f", 0) == (63, 1)
    assert _sleb(b"\x80\x7f", 0) == (-128, 2)
    with tempfile.TemporaryDirectory(prefix="lccc-dwarf-selftest-") as temp:
        root = Path(temp)
        for order, endian in (("<", "little"), (">", "big")):
            for compress in (False, True):
                for version in (201710, 202311, None):
                    case = f"synthetic-{endian}-{compress}-{version}.o"
                    written = root / case
                    _synthetic_object(written, 29, version, order, compress)
                    attributes = _compile_unit(_elf_sections(written))
                    detail = (case, attributes)
                    assert attributes[0x13] == 29, detail
                    assert attributes[0x90] == 3, detail
                    assert attributes.get(0x91) == version, detail
                    assert attributes[0x25] == "synthetic", detail


def self_test() -> int:
    names = [case.name for case in CASES]
    assert len(names) == len(set(names)), "case names must be unique"
    for case in CASES:
        assert case.expect in {"accept", "reject"}, case.name
        if case.expect == "accept":
            assert not case.lccc_messages, f"{case.name}: an accepted case cannot pin messages"
    # The pairs that make the boundary readable: for each dialect-sensitive
    # feature there must be at least one accepted and one rejected case.
    assert any(c.dialect == "c23" and c.expect == "accept" for c in CASES)
    assert any(c.dialect == "c17" and c.expect == "reject" for c in CASES)
    assert any("gnu17" == c.dialect for c in CASES), "the GNU dialects must be covered"
    assert any("implicit" in c.name for c in CASES)
    assert any("arity" in c.name for c in CASES)
    assert any("elifdef" in c.name for c in CASES)
    assert any("va_start" in c.name or "va-start" in c.name for c in CASES)
    # readelf renderings of one value, all of which must read as 201710.
    for rendering in (
        "0x313ee\t(201710)",
        "201710",
        "0x313ee",
        "(0x31647)".replace("31647", "313ee"),
    ):
        assert _decimal_from_attribute(rendering) == "201710", rendering
    assert _decimal_from_attribute("<unknown>") is None
    assert _decimal_from_attribute("29\t(C11)") == "29", "language code with a name"
    identity = {dialect: (language, version) for dialect, language, version in DWARF_IDENTITY}
    assert identity["c17"] == ("29", "201710")
    assert identity["c23"] == ("29", "202311")
    _decoder_self_test()
    print(f"C17/C23 diagnostic parity self-test: PASS ({len(CASES)} cases)")
    return 0


def _dwarf_table(rows: dict[str, dict[str, str]]) -> list[str]:
    lines = [
        "| Dialect | LCCC | GCC | Expected | readelf |",
        "|---|---:|---:|---:|---|",
    ]
    for dialect, entry in rows.items():
        lines.append(
            f"| `-std={dialect}` | {entry['lccc']} | {entry['gcc']} | {entry['expected']}"
            f" | {entry['second_opinion']} |"
        )
    return lines


def markdown(report: dict[str, object]) -> str:
    lines = [
        "# C17/C23 diagnostic parity vs GCC 16.2",
        "",
        f"Measured: {report['gate_date']}<br>",
        f"Oracle: GCC {report['gcc']['version']} (`{report['gcc']['command']}`)<br>",  # type: ignore[index]
        f"Compiler: `{report['lccc']['command']}`<br>",  # type: ignore[index]
        f"Cases: {len(report['cases'])}",  # type: ignore[arg-type]
        "",
        "| Case | Dialect | GCC | LCCC | Messages pinned |",
        "|---|---|---:|---:|---|",
    ]
    for entry in report["cases"]:  # type: ignore[index]
        pinned = ", ".join(f"`{text}`" for text in entry["lccc_messages"]) or "verdict only"
        lines.append(
            "| {} | `-std={}` | {} | {} | {} |".format(
                entry["name"], entry["dialect"], entry["gcc"], entry["lccc"], pinned
            )
        )
    mismatches = report["mismatches"]
    lines.extend(["", "## Mismatches", ""])
    if mismatches:
        lines.extend(f"- {item}" for item in mismatches)  # type: ignore[union-attr]
    else:
        lines.append("- None.")
    lines.extend(
        [
            "",
            "## Dialect identity in debug information",
            "",
            "Cells read `DW_AT_language / DW_AT_language_version` out of the object",
            "bytes; `default` means the version attribute is omitted because it equals",
            "the DWARF 5 default for that language.",
            "",
        ]
    )
    lines.extend(_dwarf_table(report["dwarf"]))  # type: ignore[arg-type]
    if report["dwarf32"]:
        lines.extend(["", "### 32-bit objects (`-m32`)", ""])
        lines.extend(_dwarf_table(report["dwarf32"]))  # type: ignore[arg-type]
    else:
        lines.extend(
            [
                "",
                "32-bit objects were not measured: one of the two compilers has no",
                "`-m32` support on this host.",
            ]
        )
    lines.extend(
        [
            "",
            "`readelf` is a cross-check, not the source: it reads the same bytes and",
            "must agree whenever its output names the attribute. `silent` means that",
            "binutils does not: several versions print `Unknown AT value: 91` for the",
            "attribute GCC and this compiler both emit, which is exactly why the gate",
            "decodes the object instead of trusting the tool.",
        ]
    )
    return "\n".join(lines) + "\n"


def _supports_32_bit(command: list[str], source: Path, cwd: Path) -> bool:
    """Whether this compiler can emit a 32-bit object (multilib or its own).

    The probe runs the operation the measurement runs -- compile to an object --
    so a host whose assembler cannot write 32-bit ELF is skipped rather than
    reported as a parity failure.
    """
    probe = cwd / "probe-32.o"
    argv = [*command, "-m32", "-c", "-x", "c", source.name, "-o", probe.name]
    return run(argv, cwd).returncode == 0


def _measure_identities(
    lccc_command: list[str],
    gcc_command: list[str],
    source: Path,
    cwd: Path,
    readelf: list[str],
    extra_flags: tuple[str, ...] = (),
) -> tuple[dict[str, dict[str, str]], list[str]]:
    """The dialect every `-std=` must declare in debug information."""
    rows: dict[str, dict[str, str]] = {}
    problems: list[str] = []
    for dialect, expected_language, expected_version in DWARF_IDENTITY:
        lccc_identity = dwarf_identity(
            lccc_command, dialect, source, cwd, readelf, "lccc", extra_flags
        )
        gcc_identity = dwarf_identity(
            gcc_command, dialect, source, cwd, readelf, "gcc", extra_flags
        )
        expected = (expected_language, expected_version)
        where = f"DWARF/{'32-bit ' if extra_flags else ''}-std={dialect}"
        rows[dialect] = {
            "lccc": lccc_identity.cell(),
            "gcc": gcc_identity.cell(),
            "expected": f"{expected_language} / {expected_version or 'default'}",
            "producer": lccc_identity.producer,
            "second_opinion": lccc_identity.second_opinion,
        }
        for who, identity in (("LCCC", lccc_identity), ("GCC", gcc_identity)):
            if identity.error is not None:
                problems.append(f"{where}: {who} object could not be read ({identity.error})")
        if lccc_identity.error is not None or gcc_identity.error is not None:
            continue
        if lccc_identity.key() != gcc_identity.key():
            problems.append(
                f"{where}: LCCC declares {lccc_identity.cell()}, GCC {gcc_identity.cell()}"
            )
        if lccc_identity.key() != expected:
            problems.append(f"{where}: expected {expected}, found {lccc_identity.key()}")
        for who, identity in (("LCCC", lccc_identity), ("GCC", gcc_identity)):
            opinion = identity.second_opinion
            if opinion == "silent" or opinion.startswith("unavailable"):
                continue
            if opinion != identity.cell():
                problems.append(
                    f"{where}: {who} object bytes say {identity.cell()}"
                    f" but readelf says {opinion}"
                )
    return rows, problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default="target/fastbuild/lccc", help="LCCC compiler command")
    parser.add_argument("--gcc", default="gcc-16", help="pinned GCC 16.2 command")
    parser.add_argument("--readelf", default="readelf", help="readelf command")
    parser.add_argument("--json", type=Path, help="write the machine-readable report")
    parser.add_argument("--markdown", type=Path, help="write a Markdown rendering")
    parser.add_argument("--self-test", action="store_true", help="check the case table and the object reader")
    parser.add_argument(
        "--report-only",
        action="store_true",
        help="print the table and exit 0 even on a mismatch (docs regeneration only)",
    )
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    gcc_command = resolve(args.gcc)
    lccc_command = resolve(args.lccc)
    readelf_command = resolve(args.readelf)
    version = compiler_version(gcc_command)
    if not re.match(r"^16\.2(?:\.|$)", version):
        print(f"C17/C23 diagnostics: ERROR: expected GCC 16.2, found {version!r}", file=sys.stderr)
        return 2

    try:
        _decoder_self_test()
    except AssertionError as error:
        print(
            f"C17/C23 diagnostics: ERROR: the object reader failed its self-test: {error}",
            file=sys.stderr,
        )
        return 2

    mismatches: list[str] = []
    entries: list[dict[str, object]] = []
    with tempfile.TemporaryDirectory(prefix="lccc-cstd-diagnostics-") as temp:
        root = Path(temp)
        for case in CASES:
            case_dir = root / case.name
            case_dir.mkdir()
            source = case_dir / "case.c"
            source.write_text(case.source, encoding="utf-8")
            lccc = diagnose(lccc_command, case.dialect, source, case_dir)
            gcc = diagnose(gcc_command, case.dialect, source, case_dir)
            entries.append(
                {
                    "name": case.name,
                    "dialect": case.dialect,
                    "expect": case.expect,
                    "lccc": lccc["verdict"],
                    "gcc": gcc["verdict"],
                    "lccc_messages": list(case.lccc_messages),
                    "divergence": case.divergence,
                }
            )
            for who, result, expected, messages in (
                ("LCCC", lccc, case.expect, case.lccc_messages),
                ("GCC", gcc, case.expect, case.oracle_messages),
            ):
                if result["verdict"] != expected:
                    mismatches.append(
                        f"{case.name}/{case.dialect}: expected {who} to {expected}"
                        f"(-std={case.dialect}), got {result['verdict']}"
                    )
                stderr = str(result["stderr"])
                for text in messages:
                    if text not in stderr:
                        mismatches.append(
                            f"{case.name}/{case.dialect}: {who} output lacks {text!r}"
                        )
        # Debug information: the dialect must be identifiable in the object.
        dwarf_source = root / "dwarf.c"
        dwarf_source.write_text("int f(int a) { return a + 1; }\n", encoding="utf-8")
        dwarf, dwarf_mismatches = _measure_identities(
            lccc_command, gcc_command, dwarf_source, root, readelf_command
        )
        mismatches.extend(dwarf_mismatches)
        if _supports_32_bit(lccc_command, dwarf_source, root) and _supports_32_bit(
            gcc_command, dwarf_source, root
        ):
            dwarf32, dwarf32_mismatches = _measure_identities(
                lccc_command, gcc_command, dwarf_source, root, readelf_command, ("-m32",)
            )
            mismatches.extend(dwarf32_mismatches)
        else:
            dwarf32 = {}

    report: dict[str, object] = {
        "gate_date": GATE_DATE,
        "purpose": "Diagnostic text and severity parity for the C17/C23 dialect boundary.",
        "gcc": {"command": args.gcc, "version": version},
        "lccc": {"command": args.lccc},
        "cases": entries,
        "dwarf": dwarf,
        "dwarf32": dwarf32,
        "mismatches": mismatches,
    }
    rendered = markdown(report)
    print(rendered)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if args.markdown:
        args.markdown.parent.mkdir(parents=True, exist_ok=True)
        args.markdown.write_text(rendered, encoding="utf-8")
    if mismatches and not args.report_only:
        print(
            f"C17/C23 diagnostics: FAIL: {len(mismatches)} mismatch(es)",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
