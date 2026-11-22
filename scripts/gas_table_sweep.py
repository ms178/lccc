#!/usr/bin/env python3
"""ISA-wide differential against GNU as, driven by GNU as's own instruction table.

Why another differential
------------------------
`asmdiff.py` and `encdiff.py` compare what somebody thought of writing down:
case files and corpora.  The integrated assembler can only be as complete as
those lists, and a statement nobody listed is checked by nobody.  This tool
takes the list from the reference itself.  Every row of binutils'
`opcodes/i386-opc.tbl` (template blocks expanded) names a mnemonic and the
operand classes it takes, in AT&T order; each row is turned into concrete
statements -- a few registers, memory operands and immediates per operand
class, with and without a size suffix -- the ones GNU as assembles are kept,
and lccc assembles the same statements:

  SAME     identical bytes and identical relocations
  REJECT   GNU as assembles the statement, lccc refuses it with a diagnostic
  CRASH    lccc does anything else with it: a panic, a signal, a timeout,
           an object that cannot be read back
  LONGER   lccc's encoding is longer
  SHORTER  lccc's encoding is shorter
  DIFF     same length, different bytes
  RELOC    same bytes, different relocations

DIFF, RELOC, LONGER and CRASH are defects by definition: the same statement
means something else, resolves to another address, wastes bytes, or takes the
assembler down.  The first run of this tool found the first three kinds (an
operation without a size suffix taking 64 bits where GNU as takes 32, a
RIP-relative address off by the bytes of a trailing immediate, `push %cx`
pushing four bytes).  SHORTER is acceptable only for the audited list in the
expectation file: lccc commutes the sources of a commutative VEX instruction
to reach the 2-byte prefix, which GNU as does under `-O2` only.

Four modes, one expectation file each: 64-bit code, 32-bit code (`--32`), and
16-bit code under `.code16` (`--16`) and `.code16gcc` (`--16gcc`), where the
default operand and address sizes are the other way round and every size
prefix is too.

The other direction
-------------------
The table also says what a mnemonic does *not* take.  For every mnemonic and
operand position, a register of each kind no row has there is put in --
`rdrand %xmm0`, `adc %cl, %mm3`, `addps %ecx, %xmm1` -- and in the vector
positions of every row vector registers of every width, in every combination
-- `vaddps %ymm1, %xmm2, %xmm3`, `vpermd %xmm1, %xmm2, %xmm3`.  And the
statements made from the rows themselves that GNU as refuses are asked too: a
size suffix on a register of another width (`cmovaq (%rsp), %r9w`), general
registers of two widths where a row takes one of several (`shrd %cl, %ebp,
%r13`), a vector register only EVEX reaches on an opcode without one
(`vfrczpd %xmm17, %xmm17`).  Of the statements GNU as refuses, lccc has to
refuse every one:

  ACCEPT   GNU as refuses the statement, lccc assembles it

An encoder that goes by a register's number, takes an instruction's vector
length from one operand and never looks at the others, or takes the operand
size from a suffix and never looks at the register, assembles such a statement
into an instruction on other registers; the first runs found 17,000, 1,200
and 670 of them assembled.  One that is left is recorded by name.  (64-bit,
32-bit and `.code16` code: an operand-size prefix means the other size in
16-bit code, so what an encoding is as wide as is asked there too.
`.code16gcc` differs from it in the size of a few statements without a
suffix.)

The table is GPL-3.0-or-later and is *read*, never copied: it is fetched at a
pinned commit, verified against a pinned SHA-256 (the same pin
`scripts/isa/gas-2.47.manifest.json` records for the cross-reference gate) and
cached outside the repository.

Usage
-----
    # report
    scripts/gas_table_sweep.py --as AS --lccc LCCC [MODE] [--only MNEMONIC...]

    # CI gate: exactly what the expectation file records -- the same number
    # of statements, the audited SHORTER ones, the same rejected count per
    # mnemonic -- and nothing else beside SAME
    scripts/gas_table_sweep.py --as AS --lccc LCCC [MODE] --check

    # after a fix: rewrite the expectation file from this run
    scripts/gas_table_sweep.py --as AS --lccc LCCC [MODE] --update

    MODE is nothing (64-bit), --32, --16 or --16gcc.

Statements are assembled in batches, each between two named labels, so one
`as` run and a handful of lccc runs cover the whole table (about a minute per
mode).  The statement set is a pure function of the table and of this file:
what is sampled is chosen by a SHA-256 of the row and the statement, not by a
generator whose sequence a Python release may change, so the expectation
files mean the same statements on every machine.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import itertools
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXPECT_DIR = REPO / "tests" / "gas-table-sweep"


@dataclass(frozen=True)
class Mode:
    """One way of assembling the table: also the name of its expectation file."""
    name: str
    long: bool                 # 64-bit registers and addresses exist
    gas: tuple[str, ...]       # GNU as's options
    lccc: tuple[str, ...]      # lccc's options
    prelude: str               # the directive the statements stand behind
    real: bool                 # 16-bit code: 16-bit addresses are swept too
    foreign: bool              # the other direction is swept too


MODES = {
    "64": Mode("64", True, ("--64",), (), "", False, True),
    "32": Mode("32", False, ("--32",), ("-m32",), "", False, True),
    "16": Mode("16", False, ("--32",), ("-m32",), ".code16", True, True),
    "16gcc": Mode("16gcc", False, ("--32",), ("-m32",), ".code16gcc", True, False),
}

# Statements GNU as 2.47 assembles into something no processor reads as
# written. They are not compared.
ORACLE_INVALID = {
    "16gcc": {
        # `66 68` and a two-byte immediate: under the prefix `68` takes
        # four, so the next two bytes of the program become the upper half
        # of what is pushed. (lccc writes `66 68` and four bytes, as GNU as
        # does for `pushl $ext`.)
        "push $ext",
    },
}

# A register of each kind, for the positions where no row of a mnemonic
# takes that kind.
FOREIGN = (
    ("Reg8", "%cl"), ("Reg16", "%cx"), ("Reg32", "%ecx"), ("Reg64", "%rcx"),
    ("SReg", "%fs"), ("Control", "%cr3"), ("Debug", "%db7"), ("FloatReg", "%st(2)"),
    ("RegMMX", "%mm3"), ("RegXMM", "%xmm1"), ("RegYMM", "%ymm2"), ("RegZMM", "%zmm3"),
    ("RegMask", "%k2"),
)
VECTOR_REGISTERS = {"RegXMM", "RegYMM", "RegZMM"}
# Of those statements, one in this many is swept: all of them. (It was one
# in three while each took a run of lccc of its own; `assemble_each`.)
FOREIGN_SHARE = 1

# The assembler the expectation files were recorded with. Another release
# assembles another set of statements, so a comparison against it says
# nothing about these files.
GAS_VERSION = re.compile(r"2\.47(\.[0-9]+)*")
# The defaults the expectation files were recorded with.
PER_ROW, SEED = 6, 1
# What a refusal looks like: lccc's exit status for a diagnostic. Anything
# else -- 101 is a Rust panic, a negative status a signal -- is a crash.
REFUSED = 1

# The table, at an immutable commit of the binutils-gdb mirror whose copy is
# byte-identical to the 2.47 release tarball's.
TABLE_REPO = "RTEMS/sourceware-mirror-binutils-gdb"
TABLE_COMMIT = "fd3cf86c638b7fdc0ccb64e178ec43ccd7e9d3f9"
TABLE_BLOB = "7042ca4512ec190463a6f2652c830bff2dc5a535"
TABLE_PATH = "opcodes/i386-opc.tbl"
TABLE_SHA256 = "a48171c05930cbe120a26fdf802689a4865607b3fe42eca784541dbf7ba81179"

DECL = re.compile(r"^<([A-Za-z_][A-Za-z0-9_]*):")
REF = re.compile(r"<([A-Za-z_][A-Za-z0-9_]*)(?::([A-Za-z_][A-Za-z0-9_]*))?>")
SIZES = ("Byte", "Word", "Dword", "Qword")
# The row attribute of an address with a vector index, and an index for it.
VECTOR_INDEX = {"VecSIB128": "%xmm7", "VecSIB256": "%ymm7", "VecSIB512": "%zmm7"}


# ─── The table ────────────────────────────────────────────────────────────

def table_text(path: str | None) -> str:
    """The pinned table: from `path`, the cache, or the network.

    Nothing but the pinned bytes is ever used or kept: a file given with
    `--tbl` that hashes to something else ends the run, a cached copy that
    does is fetched again, and a download that does is one more failed
    attempt. The cache is written under another name and renamed, so the
    two modes may run side by side.
    """
    if path:
        raw = Path(path).read_bytes()
        digest = hashlib.sha256(raw).hexdigest()
        if digest != TABLE_SHA256:
            sys.exit(f"gas_table_sweep: {path} hashes to {digest}, "
                     f"not the pinned {TABLE_SHA256}")
        return raw.decode("utf-8")
    cache = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / "lccc-gas-table"
    cached = cache / f"i386-opc-{TABLE_SHA256[:16]}.tbl"
    try:
        raw = cached.read_bytes()
    except OSError:
        raw = b""
    if hashlib.sha256(raw).hexdigest() != TABLE_SHA256:
        raw = fetch_table()
        cache.mkdir(parents=True, exist_ok=True)
        partial = cache / f".{cached.name}.{os.getpid()}.partial"
        partial.write_bytes(raw)
        os.replace(partial, cached)
    return raw.decode("utf-8")


def fetch_table() -> bytes:
    """The pinned bytes from the mirror: the raw file, else the blob.

    Three rounds over both, a pause between rounds. An answer that is not
    the pinned bytes -- an error page, a truncated body -- is a failure
    like a refused connection.
    """
    urls = [
        f"https://raw.githubusercontent.com/{TABLE_REPO}/{TABLE_COMMIT}/{TABLE_PATH}",
        f"https://api.github.com/repos/{TABLE_REPO}/git/blobs/{TABLE_BLOB}",
    ]
    errors = []
    for attempt in range(3):
        if attempt:
            time.sleep(5 * attempt)
        for url in urls:
            try:
                request = urllib.request.Request(
                    url, headers={"User-Agent": "lccc-gas-table-sweep"})
                token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
                if token and "api.github.com" in url:
                    request.add_header("Authorization", f"Bearer {token}")
                with urllib.request.urlopen(request, timeout=60) as reply:
                    body = reply.read()
                if "api.github.com" in url:
                    body = base64.b64decode(json.loads(body)["content"])
            except Exception as error:  # noqa: BLE001 -- every failure means "try the next"
                errors.append(f"{url}: {error}")
                continue
            digest = hashlib.sha256(body).hexdigest()
            if digest == TABLE_SHA256:
                return body
            errors.append(f"{url}: {len(body)} bytes hashing to {digest}")
    sys.exit("gas_table_sweep: cannot fetch the instruction table "
             f"(sha256 {TABLE_SHA256}; pass --tbl):\n  " + "\n  ".join(errors))


def split_top(text: str, sep: str = ",") -> list[str]:
    """Split on `sep` outside `<>`, `{}` and parentheses."""
    out, angle, brace, paren, start = [], 0, 0, 0, 0
    for i, ch in enumerate(text):
        # `<` opens a template reference only in front of a name: the opcode
        # language also has `<<`.
        if ch == "<" and i + 1 < len(text) and (text[i + 1].isalpha() or text[i + 1] == "_"):
            angle += 1
        elif ch == ">" and angle:
            angle -= 1
        elif ch == "{":
            brace += 1
        elif ch == "}":
            brace -= 1
        elif ch == "(":
            paren += 1
        elif ch == ")":
            paren -= 1
        elif ch == sep and not (angle or brace or paren):
            out.append(text[start:i])
            start = i + 1
    out.append(text[start:])
    return out


def logical_lines(text: str) -> list[str]:
    """The table's lines, with those continued by a trailing `+` joined."""
    out, buf = [], ""
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("//") or line.startswith("#"):
            continue
        continued = line.endswith("+")
        if continued:
            line = line[:-1].rstrip()
        buf = f"{buf} {line}".strip() if buf else line
        if not continued:
            out.append(buf)
            buf = ""
    if buf:
        out.append(buf)
    return out


# The mnemonics whose number the table has in front and a statement behind:
# `out %al, $5` is the row `{ Imm8, Acc }`. GNU as exchanges the two before
# it looks for the row ("OUT, UWRMSR, and WRMSRNS want their immediate to be
# in the canonical place", `md_assemble` in gas/config/tc-i386.c).
NUMBER_LAST = {"out", "uwrmsr", "wrmsrns"}


def expand_rows(text: str):
    """Yield (mnemonic, opcode, cpu, attributes, operand class sets), templates
    expanded and the operands in the order a statement has them."""
    templates: dict[str, list[dict]] = {}
    rows = []
    for line in logical_lines(text):
        match = DECL.match(line)
        if match and line.endswith(">"):
            body = line[line.index(":") + 1: line.rindex(">")]
            parts = split_top(body)
            fields = ["name"] + [f.strip() for f in parts[0].split(":") if f.strip()]
            instances = []
            for item in parts[1:]:
                if not item.strip():
                    continue
                values = [v.strip() for v in item.split(":")]
                instances.append({f: (values[i] if i < len(values) else "")
                                  for i, f in enumerate(fields)})
            templates[match.group(1)] = instances
            continue
        if line.startswith("{"):
            continue
        rows.append(line)

    def substitute(text: str, chosen: dict[str, dict]) -> str:
        def replace(match: re.Match) -> str:
            instance = chosen[match.group(1)]
            if match.group(2) is None:
                # A first field that starts with `$` selects between
                # spellings and contributes no text.
                value = instance.get("name", "")
                return "" if value.startswith("$") else value
            return instance.get(match.group(2), "")
        return REF.sub(replace, text)

    for line in rows:
        refs = list(dict.fromkeys(name for name, _ in REF.findall(line)))
        if any(name not in templates for name in refs):
            continue
        for combo in (itertools.product(*[templates[r] for r in refs]) if refs else [()]):
            row = substitute(line, dict(zip(refs, combo))) if refs else line
            parts = split_top(row)
            if len(parts) < 4:
                continue
            mnemonic = parts[0].strip()
            if not re.fullmatch(r"[a-z][a-z0-9_.]*", mnemonic):
                continue
            attributes = {a.strip() for a in parts[3].split("|") if a.strip()}
            operands_text = ",".join(parts[4:]).strip()
            operands = []
            if operands_text.startswith("{"):
                inner = operands_text[1: operands_text.rindex("}")].strip()
                if inner:
                    operands = [{t.strip() for t in o.split("|") if t.strip()}
                                for o in split_top(inner)]
            if "SSE2AVX" in attributes:
                mnemonic = "v" + mnemonic
            if (mnemonic in NUMBER_LAST and len(operands) > 1
                    and any(c.startswith("Imm") for c in operands[0])):
                operands[0], operands[1] = operands[1], operands[0]
            yield mnemonic, parts[1].strip(), parts[2].strip(), attributes, operands


# ─── Statements ───────────────────────────────────────────────────────────

def operand_choices(classes: set[str], mode: Mode, attributes: set[str]) -> list[str]:
    """Concrete AT&T operands for one operand class set: a few of each kind."""
    long = mode.long
    out: list[str] = []
    if "Acc" in classes or "FloatAcc" in classes:
        if "FloatAcc" in classes:
            out.append("%st")
        for size, name in zip(SIZES, ("%al", "%ax", "%eax", "%rax")):
            if size in classes and (long or size != "Qword"):
                out.append(name)
        if "Acc" in classes and not (classes & set(SIZES)):
            out += ["%al", "%ax", "%eax"] + (["%rax"] if long else [])
    for reg, names in (("RegC", ("%cl", "%cx", "%ecx", "%rcx")),
                       ("RegD", ("%dl", "%dx", "%edx", "%rdx")),
                       ("RegB", ("%bl", "%bx", "%ebx", "%rbx"))):
        if reg in classes:
            for size, name in zip(SIZES, names):
                if size in classes and (long or size != "Qword"):
                    out.append(name)
            if not (classes & set(SIZES)):
                out.append(names[2])
    if "Reg8" in classes:
        out += ["%cl", "%dh"] + (["%sil", "%r9b"] if long else [])
    if "Reg16" in classes:
        out += ["%cx"] + (["%r9w"] if long else [])
    if "Reg32" in classes:
        out += ["%ecx", "%ebp"] + (["%r9d"] if long else [])
    if "Reg64" in classes and long:
        out += ["%rcx", "%r9", "%r13"]
    if "ShiftCount" in classes:
        out.append("%cl")
    if "InOutPortReg" in classes:
        out += ["%dx", "(%dx)"]
    if "SReg" in classes:
        out += ["%fs", "%es", "%ds"]
    if "Control" in classes:
        out += ["%cr3", "%cr0"] + (["%cr8"] if long else [])
    if "Debug" in classes:
        out += ["%db7", "%dr3"]
    if "Test" in classes:
        out.append("%tr3")
    if "FloatReg" in classes:
        out += ["%st(2)", "%st(0)"]
    if "RegMMX" in classes:
        out += ["%mm3", "%mm7"]
    if "RegXMM" in classes:
        out += ["%xmm1"] + (["%xmm9", "%xmm17"] if long else ["%xmm6"])
    if "RegYMM" in classes:
        out += ["%ymm2"] + (["%ymm10", "%ymm18"] if long else ["%ymm6"])
    if "RegZMM" in classes:
        out += ["%zmm3"] + (["%zmm20"] if long else ["%zmm6"])
    if "RegMask" in classes:
        out += ["%k2", "%k5"]
    if "RegBND" in classes:
        out.append("%bnd1")
    if "RegTMM" in classes:
        out.append("%tmm3")
    if "Imm1" in classes:
        out.append("$1")
    if "Imm8" in classes:
        out += ["$5", "$0x90"]
    if "Imm8S" in classes:
        out += ["$-3", "$7"]
    if "Imm16" in classes:
        out.append("$0x1234")
    if "Imm32" in classes:
        out.append("$0x12345678")
    if "Imm32S" in classes:
        out.append("$-0x12345678")
    if "Imm64" in classes and long:
        out.append("$0x123456789abcdef")
    # A symbol where a 16-bit or wider immediate fits (GNU as takes one in an
    # 8-bit field too, as an R_*_8 that nobody writes: not swept).
    if classes & {"Imm16", "Imm32", "Imm32S", "Imm64"}:
        out.append("$ext")
    vector_index = VECTOR_INDEX.get(next((a for a in attributes if a in VECTOR_INDEX), ""))
    if "BaseIndex" in classes and vector_index:
        # A gather or a scatter: its address has a vector register for an index.
        base, other = ("%rdx", "%rbx") if long else ("%edx", "%ebx")
        out += [f"({base},{vector_index},2)", f"0x12({other},{vector_index},4)",
                f"0x1000(,{vector_index},8)"]
    elif "BaseIndex" in classes:
        if long:
            out += ["(%rdx)", "0x12(%rbx,%rsi,4)", "0x1234(%r9,%r10,2)", "(%rsp)", "(%rbp)",
                    "sym(%rip)", "0x1000"]
        elif mode.real:
            # 16-bit addresses first: they are what 16-bit code is written
            # with, and the first choice is in every sample.
            out += ["(%bx)", "0x12(%bx,%si)", "0x1234(%bp)", "(%di)",
                    "(%edx)", "0x12(%ebx,%esi,4)", "sym", "0x1000"]
        else:
            out += ["(%edx)", "0x12(%ebx,%esi,4)", "0x1234(,%edi,2)", "(%esp)", "(%ebp)",
                    "sym", "0x1000"]
    elif classes & {"Disp8", "Disp16", "Disp32", "Disp64"}:
        if any(a.startswith("Jump") for a in attributes):
            out += ["1f", "ext"]
        else:
            out.append("0x1122334455667788" if (long and "Disp64" in classes) else "0x11223344")
    return list(dict.fromkeys(out))


def draw(seed: int, *parts: str) -> int:
    """A number that depends on `seed` and `parts` alone, on any Python."""
    text = "\x1f".join((str(seed),) + parts)
    return int.from_bytes(hashlib.sha256(text.encode()).digest()[:8], "big")


def rows_of(rows, mode: Mode, only: set[str]):
    """The rows of the mode, and of the mnemonics in `only` if there are any."""
    for row in rows:
        mnemonic, _opcode, cpu, _attributes, _operands = row
        if only and mnemonic not in only:
            continue
        cpus = set(re.split(r"[&|()]", cpu))
        if mode.long and "No64" in cpus:
            continue
        if not mode.long and ("x64" in cpus or "APX_F" in cpus):
            continue
        yield row


def orders_of(attributes: set[str], operands: list) -> list[list]:
    """The orders GNU as takes a row's operands in: the row's own, and for a
    row with a direction bit (`D`) the ones with its first operand and its
    second or last exchanged -- `add %ecx, (%rdx)` is a row, `add (%rdx),
    %ecx` is that row backwards. GNU as says which of them are statements."""
    orders = [operands]
    if "D" in attributes and len(operands) >= 2:
        for other in {1, len(operands) - 1}:
            swapped = list(operands)
            swapped[0], swapped[other] = swapped[other], swapped[0]
            if swapped not in orders:
                orders.append(swapped)
    return orders


def foreign_statements(rows, mode: Mode, only: set[str], seed: int,
                       share: int = FOREIGN_SHARE) -> list[str]:
    """Statements with registers a mnemonic does not take; one in `share` of them.

    Of another kind: for each mnemonic and number of operands, the operands
    of its first row, and in each position in turn a register of every kind
    of `FOREIGN` that no row of that mnemonic and length has there.

    Of another width: for each row with vector operands, `%xmm`, `%ymm` and
    `%zmm` in each of them, in every combination (`vaddps %ymm1, %xmm2,
    %xmm3`; `vpermd` on `%xmm` registers) -- a register of its own in each
    position, and one register in all of them (`vfmaddcph %xmm1, %xmm1,
    %xmm1`, whose destination has to be none of its sources).

    Most are invalid; GNU as says which.
    """
    groups: dict[tuple[str, int], list] = defaultdict(list)
    for mnemonic, _opcode, _cpu, attributes, operands in rows_of(rows, mode, only):
        if operands:
            groups[(mnemonic, len(operands))].append((attributes, operands))
    out = set()
    for (mnemonic, arity), group in groups.items():
        attributes, operands = group[0]
        base = [next(iter(operand_choices(classes, mode, attributes)), None)
                for classes in operands]
        if None not in base:
            for position in range(arity):
                taken = set().union(*(operands[position] for _, operands in group))
                for kind, register in FOREIGN:
                    if kind in taken or (kind == "Reg64" and not mode.long):
                        continue
                    out.add(mnemonic + " " + ", ".join(
                        base[:position] + [register] + base[position + 1:]))
        for attributes, operands in group:
            base = [next(iter(operand_choices(classes, mode, attributes)), None)
                    for classes in operands]
            vector = [position for position, classes in enumerate(operands)
                      if classes & VECTOR_REGISTERS]
            if None in base or not vector:
                continue
            # A register of its own in each position: some instructions
            # want their destination apart from their sources.
            for widths in itertools.product("xyz", repeat=len(vector)):
                made = list(base)
                for position, width in zip(vector, widths):
                    made[position] = f"%{width}mm{position + 1}"
                out.add(mnemonic + " " + ", ".join(made))
            # And one register in all of them, for those.
            for width in "xyz" if len(vector) > 1 else "":
                made = list(base)
                for position in vector:
                    made[position] = f"%{width}mm1"
                out.add(mnemonic + " " + ", ".join(made))
    return sorted(statement for statement in out
                  if draw(seed, "foreign", statement) % share == 0)


def statements(rows, mode: Mode, per_row: int, only: set[str], seed: int):
    """statement -> the table row it was made from, and the classes no choice exists for."""
    long = mode.long
    out: dict[str, str] = {}
    unknown: Counter = Counter()
    for mnemonic, opcode, cpu, attributes, row_operands in rows_of(rows, mode, only):
        suffixes = [""]
        if "NoSuf" not in attributes:
            for suffix, flag in (("b", "No_bSuf"), ("w", "No_wSuf"), ("l", "No_lSuf"),
                                 ("q", "No_qSuf"), ("s", "No_sSuf")):
                if flag not in attributes and (long or suffix != "q"
                                               or mnemonic.startswith("f")):
                    suffixes.append(suffix)
        for order, operands in enumerate(orders_of(attributes, row_operands)):
            per_operand = []
            for classes in operands:
                choices = operand_choices(classes, mode, attributes)
                if not choices:
                    if not order:
                        unknown["|".join(sorted(classes))] += 1
                    break
                per_operand.append(choices)
            else:
                combos = list(itertools.product(*per_operand)) if per_operand else [()]
                if len(combos) > per_row:
                    # The first choice of every operand, then a sample.
                    rest = sorted(combos[1:], key=lambda combo: draw(
                        seed, mnemonic, opcode, ", ".join(combo)))
                    combos = [combos[0]] + rest[: per_row - 1]
                for combo in combos:
                    has_memory = any(("(" in o and o != "(%dx)") or o in ("sym", "0x1000")
                                     for o in combo)
                    has_register = any(o.startswith("%") for o in combo)
                    for suffix in suffixes:
                        statement = (mnemonic + suffix + " " + ", ".join(combo)).strip()
                        # A suffix says something new only where no register
                        # fixes the size; sample the rest (about one in seven).
                        if (suffix and has_register and not has_memory
                                and draw(seed, statement) % 100 >= 15):
                            continue
                        out.setdefault(statement, f"{mnemonic} {opcode} [{cpu}]")
    return out, unknown


# ─── Objects ──────────────────────────────────────────────────────────────

class Elf:
    """Sections, symbols and relocations of an ELF32/ELF64 little-endian object."""

    def __init__(self, path: str):
        self.b = b = Path(path).read_bytes()
        if b[:4] != b"\x7fELF":
            raise ValueError(f"{path}: not an ELF object")
        self.wide = b[4] == 2
        if self.wide:
            (shoff,) = struct.unpack_from("<Q", b, 0x28)
            shentsize, shnum, shstrndx = struct.unpack_from("<HHH", b, 0x3A)
        else:
            (shoff,) = struct.unpack_from("<I", b, 0x20)
            shentsize, shnum, shstrndx = struct.unpack_from("<HHH", b, 0x2E)
        self.sections = []
        for i in range(shnum):
            at = shoff + i * shentsize
            if self.wide:
                name, typ, _f, _a, off, size, link, info, _al, entsize = \
                    struct.unpack_from("<IIQQQQIIQQ", b, at)
            else:
                name, typ, _f, _a, off, size, link, info, _al, entsize = \
                    struct.unpack_from("<IIIIIIIIII", b, at)
            self.sections.append(dict(name_off=name, type=typ, off=off, size=size, link=link,
                                      info=info, entsize=entsize, index=i))
        names = self.sections[shstrndx]
        for section in self.sections:
            section["name"] = self._string(names, section["name_off"])
        self.symbols = []
        table = next((s for s in self.sections if s["type"] == 2), None)
        if table:
            strings = self.sections[table["link"]]
            for i in range(table["size"] // table["entsize"]):
                at = table["off"] + i * table["entsize"]
                if self.wide:
                    name, info, _o, shndx, value, _s = struct.unpack_from("<IBBHQQ", b, at)
                else:
                    name, value, _s, info, _o, shndx = struct.unpack_from("<IIIBBH", b, at)
                text = self._string(strings, name)
                if (info & 0xF) == 3 and 0 < shndx < len(self.sections):
                    text = self.sections[shndx]["name"]
                self.symbols.append(dict(name=text, shndx=shndx, value=value))

    def _string(self, section, offset: int) -> str:
        data = self.b[section["off"] + offset:]
        return data[: data.index(b"\0")].decode("latin-1")

    def data(self, section) -> bytes:
        return self.b[section["off"]: section["off"] + section["size"]]

    def relocations(self, index: int):
        """(offset, type, symbol, addend) of the relocations against section `index`."""
        out = []
        for section in self.sections:
            if section["type"] not in (4, 9) or section["info"] != index:
                continue
            rela = section["type"] == 4
            for i in range(section["size"] // section["entsize"]):
                at = section["off"] + i * section["entsize"]
                if self.wide:
                    offset, info = struct.unpack_from("<QQ", self.b, at)
                    addend = struct.unpack_from("<q", self.b, at + 16)[0] if rela else None
                    symbol, typ = info >> 32, info & 0xFFFFFFFF
                else:
                    offset, info = struct.unpack_from("<II", self.b, at)
                    addend = struct.unpack_from("<i", self.b, at + 8)[0] if rela else None
                    symbol, typ = info >> 8, info & 0xFF
                name = self.symbols[symbol]["name"] if symbol < len(self.symbols) else "?"
                out.append((offset, typ, name, addend))
        return sorted(out)


TAG = "q_"


def batch_source(batch: list[str], prelude: str = "") -> str:
    lines = ["\t.text"]
    if prelude:
        lines.append(f"\t{prelude}")
    for i, statement in enumerate(batch):
        lines.append(f"{TAG}{i}:")
        lines.append(f"1:\t{statement}")
    lines.append(f"{TAG}{len(batch)}:")
    # The target of a `1f` in the last statement.
    lines.append("1:\tnop")
    return "\n".join(lines) + "\n"


def statement_of_line(line: int, prelude: str) -> int:
    """Which statement of `batch_source` stands on 1-based `line` (or its label)."""
    first = 4 if prelude else 3
    return (line - first + 1) // 2


def read_batch(path: str, count: int):
    """(bytes, relocations) of each of the `count` statements, or None."""
    try:
        elf = Elf(path)
    except (OSError, ValueError, struct.error, IndexError):
        return None
    text = next((s for s in elf.sections if s["name"] == ".text"), None)
    if text is None:
        return None
    data = elf.data(text)
    at = {}
    for symbol in elf.symbols:
        if symbol["shndx"] == text["index"] and symbol["name"].startswith(TAG):
            rest = symbol["name"][len(TAG):]
            if rest.isdigit():
                at[int(rest)] = symbol["value"]
    if len(at) != count + 1:
        return None
    relocations = elf.relocations(text["index"])
    out = []
    for i in range(count):
        low, high = at[i], at[i + 1]
        mine = tuple((offset - low, typ, "<next>" if name.startswith(TAG) else name, addend)
                     for offset, typ, name, addend in relocations if low <= offset < high)
        out.append((bytes(data[low:high]), mine))
    return out


def run(command: list[str], **kwargs):
    return subprocess.run(command, capture_output=True, text=True, **kwargs)


def gas_version(gas: str) -> str:
    """The version token of `as --version`: the last word of its first line."""
    try:
        first = run([gas, "--version"]).stdout.splitlines()[0]
    except (OSError, IndexError):
        return ""
    return first.split()[-1] if first.split() else ""


def assemble_gas(all_statements: list[str], gas: str, mode: Mode, tmp: Path):
    """{index: (bytes, relocations)} of the statements GNU as assembles."""
    alive = list(range(len(all_statements)))
    for _ in range(8):
        source = tmp / "g.s"
        source.write_text(batch_source([all_statements[i] for i in alive], mode.prelude))
        result = run([gas, *mode.gas, "-o", str(tmp / "g.o"), str(source)])
        if result.returncode == 0:
            got = read_batch(str(tmp / "g.o"), len(alive))
            if got is None:
                sys.exit("gas_table_sweep: cannot read GNU as's object back")
            return dict(zip(alive, got))
        bad = set()
        for match in re.finditer(r"g\.s:(\d+): (?:Error|Fatal error)", result.stderr):
            k = statement_of_line(int(match.group(1)), mode.prelude)
            if 0 <= k < len(alive):
                bad.add(k)
        if not bad:
            sys.exit("gas_table_sweep: GNU as failed without naming a line:\n"
                     + result.stderr[:600])
        alive = [i for k, i in enumerate(alive) if k not in bad]
    sys.exit("gas_table_sweep: GNU as keeps finding errors")


def assemble_lccc(all_statements: list[str], indexes: list[int], lccc: str,
                  mode: Mode, tmp: Path):
    """What lccc makes of the statements GNU as assembles, bisecting on failure.

    Three maps by statement index: (bytes, relocations) of the ones it
    assembles, the first line of the diagnostic of the ones it refuses, and
    what happened to the ones it does neither with.
    """
    good, refused, crashed = {}, {}, {}

    def assemble(part: list[int]) -> None:
        if not part:
            return
        source, obj = tmp / "l.s", tmp / "l.o"
        source.write_text(batch_source([all_statements[i] for i in part], mode.prelude))
        obj.unlink(missing_ok=True)
        try:
            result = run([lccc, *mode.lccc, "-c", "-o", str(obj), str(source)], timeout=300)
            code, message = result.returncode, (result.stderr or result.stdout)
        except subprocess.TimeoutExpired:
            code, message = None, "timeout after 300 s"
        except OSError as error:
            sys.exit(f"gas_table_sweep: cannot run {lccc}: {error}")
        if code == 0:
            got = read_batch(str(obj), len(part)) if obj.exists() else None
            if got is not None:
                good.update(zip(part, got))
                return
            message = "exit status 0 and no object that can be read back"
        if len(part) > 1:
            middle = len(part) // 2
            assemble(part[:middle])
            assemble(part[middle:])
            return
        lines = message.strip().splitlines()
        first = lines[0][:200] if lines else ""
        if code == REFUSED and first and "panicked" not in message:
            refused[part[0]] = first
        else:
            status = "no exit status" if code is None else f"exit status {code}"
            crashed[part[0]] = f"{status}: {first}" if first else status

    for start in range(0, len(indexes), 400):
        assemble(indexes[start:start + 400])
    return good, refused, crashed


# How many statements one run of lccc is given in `assemble_each`, each as a
# file of its own.
EACH_BATCH = 400


def assemble_each(all_statements: list[str], indexes: list[int], lccc: str,
                  mode: Mode, tmp: Path):
    """What lccc makes of each statement on its own: for the ones GNU as refuses.

    Two maps by statement index: the bytes of the ones it assembles, and
    what happened to the ones it neither assembles nor refuses.

    lccc stops a file at the first statement it refuses, so a file says
    nothing about the statements behind that one, and each statement is a
    file of its own. It goes on with the files behind one it refuses, as GCC
    does, so one run takes many of them: an object is there for a statement
    it assembles and not for one it refuses. (With a run for each statement
    the 64-bit sweep took 115 seconds for one statement in three; it takes 36
    for all of them.) A run that ends in anything but a refusal, and a driver
    that does not go on, say nothing about the statements without an object:
    those get a run each.
    """
    def one(i: int):
        source, obj = tmp / f"f{i}.s", tmp / f"f{i}.o"
        source.write_text(batch_source([all_statements[i]], mode.prelude))
        try:
            result = run([lccc, *mode.lccc, "-c", "-o", str(obj), str(source)], timeout=300)
            code, message = result.returncode, (result.stderr or result.stdout)
        except subprocess.TimeoutExpired:
            code, message = None, "timeout after 300 s"
        got = read_batch(str(obj), 1) if code == 0 and obj.exists() else None
        source.unlink(missing_ok=True)
        obj.unlink(missing_ok=True)
        if got is not None:
            return i, "ACCEPT", f"lccc {got[0][0].hex()}"
        lines = message.strip().splitlines()
        first = lines[0][:200] if lines else ""
        if code == REFUSED and first and "panicked" not in message:
            return i, None, first
        status = "no exit status" if code is None else f"exit status {code}"
        return i, "CRASH", f"{status}: {first}" if first else status

    def many(batch: tuple[int, list[int]]):
        """(verdicts of the statements the run settles, the ones it does not)."""
        number, part = batch
        directory = tmp / f"each{number}"
        directory.mkdir()
        for i in part:
            (directory / f"{i}.s").write_text(batch_source([all_statements[i]], mode.prelude))
        try:
            result = run([lccc, *mode.lccc, "-c", *(f"{i}.s" for i in part)],
                         timeout=600, cwd=directory)
            code, message = result.returncode, (result.stderr or result.stdout)
        except subprocess.TimeoutExpired:
            code, message = None, ""
        settled, open_ = [], []
        ended_well = code in (0, REFUSED) and "panicked" not in message \
            and "internal error" not in message
        for i in part:
            obj = directory / f"{i}.o"
            got = read_batch(str(obj), 1) if obj.exists() else None
            if got is not None:
                settled.append((i, "ACCEPT", f"lccc {got[0][0].hex()}"))
            elif obj.exists() or not ended_well:
                # An object that cannot be read back, or no word on this one.
                open_.append(i)
        shutil.rmtree(directory, ignore_errors=True)
        return settled, open_

    accepted, crashed = {}, {}

    def record(i: int, verdict: str | None, detail: str) -> None:
        if verdict == "ACCEPT":
            accepted[i] = detail
        elif verdict == "CRASH":
            crashed[i] = detail

    with ThreadPoolExecutor(os.cpu_count() or 1) as pool:
        alone = list(indexes)
        if goes_on(lccc, mode, tmp):
            batches = [(number, indexes[start:start + EACH_BATCH])
                       for number, start in enumerate(range(0, len(indexes), EACH_BATCH))]
            alone = []
            for settled, open_ in pool.map(many, batches):
                for found in settled:
                    record(*found)
                alone += open_
        for found in pool.map(one, alone):
            record(*found)
    return accepted, crashed


def goes_on(lccc: str, mode: Mode, tmp: Path) -> bool:
    """Whether lccc assembles the input behind one it refuses."""
    directory = tmp / "goes-on"
    directory.mkdir(exist_ok=True)
    (directory / "refused.s").write_text(batch_source(["nosuchinstruction"], mode.prelude))
    (directory / "taken.s").write_text(batch_source(["nop"], mode.prelude))
    try:
        run([lccc, *mode.lccc, "-c", "refused.s", "taken.s"], timeout=300, cwd=directory)
    except (subprocess.TimeoutExpired, OSError):
        return False
    went_on = (directory / "taken.o").exists() and not (directory / "refused.o").exists()
    shutil.rmtree(directory, ignore_errors=True)
    return went_on


# ─── Verdicts and the expectation file ────────────────────────────────────

DEFECTS = ("CRASH", "DIFF", "RELOC", "LONGER")


@dataclass
class Report:
    """One sweep: how much was swept, and every statement that is not SAME."""
    rows: int                  # table rows, templates expanded
    statements: int            # statements made from them
    assembled: int             # of those, the ones GNU as assembles
    same: int                  # of those, the ones lccc assembles identically
    findings: dict             # verdict -> [(statement, detail)]
    foreign: int | None = None  # the other direction: statements made
    refused: int | None = None  # of those, the ones GNU as refuses

    def counts(self) -> dict[str, int]:
        counts = {"rows": self.rows, "statements": self.statements,
                  "assembled": self.assembled, "same": self.same}
        if self.foreign is not None:
            counts.update(foreign=self.foreign, refused=self.refused)
        return counts


def verdict_of(ours, theirs) -> tuple[str, str]:
    """The verdict on one statement both assemble, and what to show for it."""
    if ours == theirs:
        return "SAME", ""
    if len(ours[0]) != len(theirs[0]):
        verdict = "LONGER" if len(ours[0]) > len(theirs[0]) else "SHORTER"
        return verdict, f"lccc {ours[0].hex()} gas {theirs[0].hex()}"
    if ours[0] != theirs[0]:
        return "DIFF", f"lccc {ours[0].hex()} gas {theirs[0].hex()}"
    return "RELOC", f"lccc {ours[1]} gas {theirs[1]}"


def sweep_rows(rows, mode: Mode, gas: str, lccc: str, *, per_row: int, seed: int,
               only: set[str] = frozenset(), foreign: bool = True,
               share: int = FOREIGN_SHARE):
    """The report of one sweep of `rows`, and the operand classes without a choice."""
    candidates, unknown = statements(rows, mode, per_row, only, seed)
    ordered = sorted(set(candidates) - ORACLE_INVALID.get(mode.name, set()))
    findings: dict[str, list[tuple[str, str]]] = defaultdict(list)
    same = 0
    with tempfile.TemporaryDirectory() as directory:
        tmp = Path(directory)
        reference = assemble_gas(ordered, gas, mode, tmp)
        indexes = sorted(reference)
        good, refused, crashed = assemble_lccc(ordered, indexes, lccc, mode, tmp)
        for i in indexes:
            if i in crashed:
                findings["CRASH"].append((ordered[i], crashed[i]))
            elif i in refused:
                findings["REJECT"].append((ordered[i], refused[i]))
            else:
                verdict, detail = verdict_of(good[i], reference[i])
                if verdict == "SAME":
                    same += 1
                else:
                    findings[verdict].append((ordered[i], detail))
        report = Report(len(rows), len(ordered), len(indexes), same, findings)
        if mode.foreign and foreign:
            # The statements with a register no row takes, and of the
            # statements made from the rows above the ones GNU as refuses:
            # a size suffix on a register of another width, registers of
            # two widths where a row takes one of each of several.
            other = sorted(set(foreign_statements(rows, mode, only, seed, share))
                           | {ordered[i] for i in range(len(ordered)) if i not in reference})
            valid = assemble_gas(other, gas, mode, tmp)
            invalid = [i for i in range(len(other)) if i not in valid]
            accepted, crashed = assemble_each(other, invalid, lccc, mode, tmp)
            findings["ACCEPT"] += [(other[i], accepted[i]) for i in sorted(accepted)]
            findings["CRASH"] += [(other[i], crashed[i]) for i in sorted(crashed)]
            report.foreign, report.refused = len(other), len(invalid)
    report.findings = {verdict: items for verdict, items in findings.items() if items}
    return report, unknown


def sweep(args, rows, mode: Mode) -> Report:
    report, unknown = sweep_rows(rows, mode, args.gas, args.lccc, per_row=args.per_row,
                                 seed=args.seed, only=set(args.only or []),
                                 foreign=not args.no_foreign)
    counts = "  ".join(f"{v} {len(items)}" for v, items in sorted(report.findings.items()))
    print(f"{mode.name}: {report.rows} table rows, {report.statements} statements, "
          f"{report.assembled} assembled by GNU as:  SAME {report.same}  {counts}".rstrip())
    if report.foreign is not None:
        print(f"{mode.name}: {report.foreign} statements with a register of another kind "
              f"or width, {report.refused} refused by GNU as")
    if unknown and args.verbose:
        print("operand classes without a choice:", dict(unknown.most_common(12)))
    return report


def expectation_path(mode: Mode) -> Path:
    return EXPECT_DIR / f"expected-{mode.name}.txt"


COUNTS = ("rows", "statements", "assembled", "same", "foreign", "refused")


def per_mnemonic(report: Report, verdict: str) -> dict[str, list[str]]:
    """mnemonic -> the statements of `verdict`, sorted."""
    found = defaultdict(list)
    for statement, _ in report.findings.get(verdict, []):
        found[statement.split()[0]].append(statement)
    return {mnemonic: sorted(items) for mnemonic, items in found.items()}


def digest_of(statements: list[str]) -> str:
    """Eight hex digits that name a set of statements."""
    return hashlib.sha256("\n".join(sorted(statements)).encode()).hexdigest()[:8]


def expectation_text(report: Report) -> str:
    """The expectation file for `report`: an inventory, not a ceiling."""
    lines = [
        "# What scripts/gas_table_sweep.py finds. Written by --update; --check fails",
        "# on any difference from this file, in either direction.",
        "#",
        "# rows, statements, assembled, same: how much was swept -- table rows,",
        "# statements made from them, the ones GNU as assembles, and of those the",
        "# ones lccc assembles to the same bytes and relocations.",
        "# foreign, refused: the other direction -- statements with a register of a",
        "# kind or a width the mnemonic does not take, and the ones GNU as refuses.",
        "# shorter <statement>: lccc's encoding is shorter than GNU as's, and audited.",
        "# accept <statement>: GNU as refuses the statement and lccc assembles it.",
        "# reject <mnemonic> <count> <digest>: lccc refuses that many swept statements",
        "# of the mnemonic that GNU as assembles; the digest names which.",
    ]
    lines += [f"{key} {value}" for key, value in report.counts().items()]
    for verdict, word in (("SHORTER", "shorter"), ("ACCEPT", "accept")):
        lines += [f"{word} {statement}"
                  for statement, _ in sorted(report.findings.get(verdict, []))]
    lines += [f"reject {mnemonic} {len(items)} {digest_of(items)}"
              for mnemonic, items in sorted(per_mnemonic(report, "REJECT").items())]
    return "\n".join(lines) + "\n"


def parse_expectation(text: str):
    """What an expectation file records: the counts, the statements listed
    under each of `shorter` and `accept`, and (count, digest) per mnemonic
    of `reject`."""
    counts, rejected = {}, {}
    listed = {"shorter": set(), "accept": set()}
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        key, _, rest = line.partition(" ")
        try:
            if key in COUNTS and key not in counts:
                counts[key] = int(rest)
            elif key in listed and rest and rest not in listed[key]:
                listed[key].add(rest)
            elif key == "reject":
                mnemonic, count, digest = rest.split()
                if mnemonic in rejected or not re.fullmatch(r"[0-9a-f]{8}", digest):
                    raise ValueError(rest)
                rejected[mnemonic] = (int(count), digest)
            else:
                # An unknown word, a line said twice, a number that is none.
                raise ValueError(key)
        except ValueError:
            raise ValueError(f"line {number}: cannot read `{line}`") from None
    return counts, listed, rejected


def failures_against(report: Report, text: str) -> list[str]:
    """Everything in `report` the expectation `text` does not say, and vice versa."""
    counts, listed, rejected = parse_expectation(text)
    failures = []
    for verdict in DEFECTS:
        for statement, detail in report.findings.get(verdict, []):
            failures.append(f"{verdict}: {statement}   {detail}")
    # How much was swept. A sweep that assembles nothing finds nothing; these
    # numbers are what makes "nothing found" a statement about the table.
    found_counts = report.counts()
    for key in COUNTS:
        if key in found_counts and key not in counts:
            failures.append(f"COUNT: `{key}` is not recorded (found {found_counts[key]})")
        elif key in counts and key not in found_counts:
            failures.append(f"COUNT: `{key}` is recorded ({counts[key]}) and was not swept")
        elif key in counts and counts[key] != found_counts[key]:
            failures.append(f"COUNT: {key} is {found_counts[key]}, recorded {counts[key]}")
    for verdict, word, what in (("SHORTER", "shorter", "SHORTER (not audited)"),
                                ("ACCEPT", "accept", "ACCEPT")):
        found = dict(report.findings.get(verdict, []))
        for statement in sorted(set(found) - listed[word]):
            failures.append(f"{what}: {statement}   {found[statement]}")
        for statement in sorted(listed[word] - set(found)):
            failures.append(f"STALE: `{word} {statement}` is recorded and no longer found")
    found = per_mnemonic(report, "REJECT")
    details = dict(report.findings.get("REJECT", []))
    for mnemonic in sorted(set(found) | set(rejected)):
        now = found.get(mnemonic, [])
        before, digest = rejected.get(mnemonic, (0, ""))
        if len(now) > before:
            failures.append(f"REJECT: `{mnemonic}`: {len(now)} statements, {before} recorded, "
                            f"e.g. {now[0]}   {details[now[0]]}")
        elif len(now) < before:
            failures.append(f"STALE: `reject {mnemonic}`: {len(now)} statements, "
                            f"{before} recorded")
        elif digest_of(now) != digest:
            # As many as recorded and not the same ones: one statement is
            # assembled now and another is refused that was not.
            failures.append(f"REJECT: `{mnemonic}`: {len(now)} statements as recorded, but "
                            f"other ones: {'; '.join(now[:6])}")
    return failures


def summary(report: Report) -> str:
    parts = [f"{report.same} identical",
             f"{len(report.findings.get('SHORTER', []))} shorter",
             f"{len(report.findings.get('REJECT', []))} refused"]
    if report.foreign is not None:
        parts.append(f"{len(report.findings.get('ACCEPT', []))} of {report.refused} "
                     "invalid ones assembled")
    return ", ".join(parts)


def check(report: Report, mode: Mode) -> int:
    path = expectation_path(mode)
    name = path.relative_to(REPO)
    try:
        failures = failures_against(report, path.read_text())
    except FileNotFoundError:
        print(f"gas_table_sweep: {name} does not exist; --update writes it")
        return 1
    except ValueError as error:
        print(f"gas_table_sweep: {name}: {error}")
        return 1
    for failure in failures[:60]:
        print("  " + failure)
    if failures:
        stale = sum(f.startswith(("STALE", "COUNT")) for f in failures)
        print(f"gas_table_sweep: {len(failures)} difference(s) from {name}"
              + (f"; {stale} of them are the record being out of date -- "
                 "run --update and review what it changes" if stale else ""))
        return 1
    print(f"gas_table_sweep: ok ({mode.name}: {summary(report)})")
    return 0


def update(report: Report, mode: Mode) -> int:
    blocking = [v for v in DEFECTS if report.findings.get(v)]
    if blocking:
        print("gas_table_sweep: not updating: " + ", ".join(blocking)
              + " findings are defects, not expectations")
        return 1
    path = expectation_path(mode)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(expectation_text(report))
    print(f"wrote {path.relative_to(REPO)}: {summary(report)}")
    return 0


# ─── The gate, held to a planted defect of every kind ─────────────────────

# An assembler behind lccc's command line: GNU as itself, which with
# `plant` set does to one statement each what every verdict is about. Given
# several files and no `-o` it assembles each into an object beside it, as a
# compiler driver does, and with `stops` set it ends at the first one it
# refuses, as lccc did.
PLANTED = """#!{python}
import os, subprocess, sys
args = sys.argv[1:]
gas = [{gas!r}, "--32" if "-m32" in args else "--64"]
if "-o" in args:
    files = [(args[-1], args[args.index("-o") + 1])]
else:
    files = [(name, name[:-2] + ".o") for name in args if name.endswith(".s")]


def assemble(source, obj):
    lines = open(source).read().split("\\n")
    head = [line for line in lines[:2] if line.startswith("\\t.")]
    for i, line in enumerate(lines):
        if not ({plant} and line.startswith("1:\\t")):
            continue
        statement = line[3:]
        if statement == "cmc":
            print("planted: error: no such instruction", file=sys.stderr)
            return 1                                            # REJECT
        if statement == "sti":
            sys.exit(3)                                         # CRASH
        if statement == "cli":
            open(obj, "wb").write(b"no object")                 # CRASH
            return 0
        if statement == "clc":
            lines[i] = "1:\\tstc"                                # DIFF
        if statement == "cld":
            lines[i] = "1:\\tcld; nop"                           # LONGER
        if statement == "stc":
            lines[i] = "1:"                                     # SHORTER
        if " %" in statement:
            alone = "\\n".join(head + [statement, ""])
            if subprocess.run(gas + ["-o", os.devnull, "-"], input=alone, text=True,
                              capture_output=True).returncode:
                if statement.startswith("nop "):
                    sys.exit(3)                                 # CRASH
                lines[i] = "1:\\tnop"                            # ACCEPT
    open(source, "w").write("\\n".join(lines))
    return subprocess.run(gas + ["-o", obj, source]).returncode


status = 0
for source, obj in files:
    code = assemble(source, obj)
    if code and ({stops} or code != 1):
        sys.exit(code)
    status = status or code
sys.exit(status)
"""
PLANTED_MNEMONICS = {"clc", "cld", "stc", "cmc", "sti", "cli", "nop", "bswap"}


def self_test(rows, mode: Mode, gas: str) -> list[str]:
    """What is wrong with this tool's verdicts, found by planting defects.

    A sweep that finds nothing says something only if it would have found
    something. Two assemblers stand in for lccc: GNU as itself, of which
    every statement has to be SAME, and GNU as with one statement each
    assembled to other bytes, to more, to fewer, refused, crashed on, left
    without an object and -- an invalid one -- assembled. Each has to come
    out as its verdict, and the expectation logic has to fail on each, and
    on its disappearance. The planted one is swept twice, as a driver that
    goes on behind a file it refuses and as one that stops there.
    """
    reports = []
    with tempfile.TemporaryDirectory() as directory:
        for plant, stops in ((False, False), (True, False), (True, True)):
            wrapper = Path(directory) / f"as-planted-{plant}-{stops}"
            wrapper.write_text(PLANTED.format(python=sys.executable, plant=plant, stops=stops,
                                              gas=gas))
            wrapper.chmod(0o755)
            report, _ = sweep_rows(rows, mode, gas, str(wrapper), per_row=PER_ROW, seed=SEED,
                                   only=PLANTED_MNEMONICS, share=1)
            reports.append(report)
    clean, planted, stopping = reports
    wrong = []
    # The statements GNU as refuses are given to an assembler many files at
    # a time when it goes on behind a file it refuses, and one at a time
    # when it does not (`assemble_each`): the same findings either way.
    if stopping.counts() != planted.counts() or stopping.findings != planted.findings:
        wrong.append("an assembler that stops at the first file it refuses: "
                     f"{stopping.counts()} {stopping.findings}, and one that goes on: "
                     f"{planted.counts()} {planted.findings}")
    if clean.findings or not clean.same or clean.same != clean.assembled:
        wrong.append(f"GNU as against itself: {clean.counts()} {clean.findings}")
    found = {verdict: sorted(statement for statement, _ in items)
             for verdict, items in planted.findings.items()}
    expected = {"DIFF": ["clc"], "LONGER": ["cld"], "SHORTER": ["stc"],
                "REJECT": ["cmc"], "CRASH": ["cli", "sti"]}
    if mode.foreign:
        # Every statement GNU as refuses: assembled, or -- `nop` -- crashed on.
        accepted = found.pop("ACCEPT", [])
        crashed = [statement for statement in found.get("CRASH", []) if " " in statement]
        found["CRASH"] = [statement for statement in found.get("CRASH", [])
                          if " " not in statement]
        if not (accepted and crashed
                and len(accepted) + len(crashed) == planted.refused
                and all(statement.startswith("nop %") for statement in crashed)
                and not any(statement.startswith("nop %") for statement in accepted)):
            wrong.append(f"planted ACCEPT and CRASH of {planted.refused} statements GNU as "
                         f"refuses: found {accepted} and {crashed}")
    if found != expected:
        wrong.append(f"planted defects: found {found}, expected {expected}")
    # The expectation logic: nothing to say about a sweep and its own
    # record, and something about every difference, in both directions.
    record = expectation_text(clean)
    if failures_against(clean, record):
        wrong.append(f"a sweep against its own record: {failures_against(clean, record)}")
    words = ["CRASH:", "DIFF:", "LONGER:", "SHORTER (not audited):", "REJECT:", "COUNT:"]
    stale = ["STALE: `shorter stc`", "STALE: `reject cmc`", "COUNT:"]
    if mode.foreign:
        words.append("ACCEPT:")
        stale.append("STALE: `accept bswap %")
    for report, text, starts in ((planted, record, words),
                                 (clean, expectation_text(planted), stale)):
        failures = failures_against(report, text)
        wrong += [f"the expectation logic does not report `{start}`: {failures}"
                  for start in starts if not any(f.startswith(start) for f in failures)]
    # As many refusals of a mnemonic as recorded, and other statements.
    moved = Report(planted.rows, planted.statements, planted.assembled, planted.same,
                   {**planted.findings, "REJECT": [("cmc %eax", "moved")]},
                   planted.foreign, planted.refused)
    if not any("other ones" in failure
               for failure in failures_against(moved, expectation_text(planted))):
        wrong.append("the expectation logic takes another refused statement of a mnemonic "
                     "for the recorded one")
    return wrong


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--tbl", help="opcodes/i386-opc.tbl of binutils 2.47 (default: fetch)")
    parser.add_argument("--as", dest="gas", default="as", help="GNU as 2.47")
    parser.add_argument("--lccc", help="the lccc driver to hold against GNU as")
    which = parser.add_mutually_exclusive_group()
    which.add_argument("--32", dest="mode", action="store_const", const="32",
                       help="32-bit code")
    which.add_argument("--16", dest="mode", action="store_const", const="16",
                       help="16-bit code (.code16)")
    which.add_argument("--16gcc", dest="mode", action="store_const", const="16gcc",
                       help="16-bit code with 32-bit stack operations (.code16gcc)")
    parser.set_defaults(mode="64")
    parser.add_argument("--per-row", type=int, default=PER_ROW,
                        help="operand combinations per table row")
    parser.add_argument("--seed", type=int, default=SEED)
    parser.add_argument("--only", nargs="+", metavar="MNEMONIC",
                        help="sweep these table mnemonics only (no --check/--update)")
    parser.add_argument("--no-foreign", action="store_true",
                        help="skip the other direction (no --check/--update)")
    parser.add_argument("--check", action="store_true",
                        help="fail on any difference from the expectation file")
    parser.add_argument("--update", action="store_true",
                        help="rewrite the expectation file from this run")
    parser.add_argument("--self-test", action="store_true",
                        help="plant a defect of every kind and require that it is found "
                             "(--check does this first)")
    parser.add_argument("--show", type=int, default=30, help="statements to print per verdict")
    parser.add_argument("--json", help="write every non-SAME statement here")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args(argv)
    mode = MODES[args.mode]
    recording = args.check or args.update
    if not (args.lccc or args.self_test):
        parser.error("--lccc is required")
    if recording and (args.only or args.no_foreign):
        parser.error("--only and --no-foreign sweep a part; --check and --update need all of it")
    if recording and (args.per_row, args.seed) != (PER_ROW, SEED):
        parser.error("--check and --update are about the statements of the defaults; "
                     "--per-row and --seed make other statements")
    version = gas_version(args.gas)
    if not GAS_VERSION.fullmatch(version):
        message = (f"{args.gas} is GNU as `{version}`, not 2.47: it assembles another "
                   "set of statements than the expectation files were recorded with")
        if recording:
            sys.exit("gas_table_sweep: " + message)
        print("warning: " + message, file=sys.stderr)

    rows = list(expand_rows(table_text(args.tbl)))
    if args.check or args.self_test:
        wrong = self_test(rows, mode, args.gas)
        for line in wrong:
            print("  " + line)
        if wrong:
            print("gas_table_sweep: self-test failed: this tool does not find what it "
                  "is there to find")
            return 1
        if args.self_test:
            print(f"gas_table_sweep: self-test ok ({mode.name}: a planted defect of every "
                  "kind is found and fails the check)")
            return 0
    report = sweep(args, rows, mode)
    if args.json:
        Path(args.json).write_text(json.dumps(report.findings, indent=0))
    if args.update:
        return update(report, mode)
    if args.check:
        return check(report, mode)
    for verdict in (*DEFECTS, "SHORTER", "ACCEPT", "REJECT"):
        items = report.findings.get(verdict, [])
        if not items:
            continue
        mnemonics = Counter(statement.split()[0] for statement, _ in items)
        print(f"\n== {verdict}: {len(items)} statements, {len(mnemonics)} mnemonics")
        for statement, detail in items[: args.show]:
            print(f"  {statement:<52} {detail}")
        if verdict in ("REJECT", "ACCEPT"):
            print("  mnemonics: " + " ".join(f"{m}({c})" for m, c in sorted(mnemonics.items())))
    return 0


if __name__ == "__main__":
    sys.exit(main())
