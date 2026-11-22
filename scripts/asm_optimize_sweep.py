#!/usr/bin/env python3
"""Hold the assemblers' optimizer to GNU as, and run what it does.

lccc replaces a statement by a cheaper instruction with the same effect
(`src/backend/x86/assembler/encoder/optimize.rs`): in everything the
compiler writes, and under `-Wa,-O2` or `-Wa,-Os` in what a person wrote.
GNU as has `-O2` and `-Os` too. This script makes statements out of GNU as
2.47's own instruction table, as `scripts/gas_table_sweep.py` does, with
what an optimizer looks at put in on top -- immediates at the edges of
every width, one register in every position, every form of address, `{nf}`
-- and asks two things of every one of them.

**Is lccc's encoding as short as GNU as's?** (`--check`, `--update`.) lccc
under `-Wa,-O2` against GNU as under `-O2`, and lccc under `-Wa,-Os` against
the shorter of GNU as's `-O2` and `-Os`. A statement lccc encodes longer
fails unless the expectation file names it, and the ones it names are in
`tests/asm-optimize/README.md` with the reason: each is a replacement GNU as
makes that is not the same instruction. Everything else is an inventory,
which `--check` holds in both directions: how many statements were
rewritten and to what, how many came out shorter than GNU as's, how many
the same length in other bytes.

**Is the replacement the same instruction?** (`--execute`.) Every rewritten
statement is run on this processor both ways, as written and as replaced
(and, beside `--check`, every statement lccc encodes as written to other
bytes than GNU as: GNU as's bytes and lccc's), from the same machine state: every general register, the flags, every
vector and mask register, a block of memory the statement's address is
made to point into. Afterwards all of it has to be the same, but for the
flags the written instruction leaves undefined. A statement this processor
does not run (AVX-512 on one without it, APX anywhere yet) is counted and
skipped; one whose replacement it does not run fails, since a replacement
may need nothing the written instruction does not. No assembler but lccc is
needed for this, and no table when the statements come from `--insn` or
`--file`.

`--self-test` plants replacements that are wrong in each way the run can
see -- another value, another flag, a register's upper half, a vector
register's, another byte of memory, another instruction set -- and requires
every one to be found.

    scripts/asm_optimize_sweep.py --check                 # 64-bit code
    scripts/asm_optimize_sweep.py --32 --check
    scripts/asm_optimize_sweep.py --execute [--32]
    scripts/asm_optimize_sweep.py --execute --insn 'movq $1, %rax'
    scripts/asm_optimize_sweep.py --self-test

Exit status: 0 = as expected, 1 = something differs, 2 = usage or tooling.
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import itertools
import os
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gas_table_sweep as table  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
EXPECT_DIR = REPO / "tests" / "asm-optimize"
LEVELS = ("O2", "Os")
PER_ROW, SEED = 6, 1

# Immediates at the edges of each width.
BOUNDARY = {
    "Imm8": ["$0", "$1", "$0x7f", "$0x80", "$0xff"],
    "Imm8S": ["$0", "$1", "$0x7f", "$-0x80", "$-1"],
    "Imm16": ["$0", "$0x7f", "$0x80", "$0xff", "$0x7fff", "$0x8000", "$0xffff"],
    "Imm32": ["$0", "$0x7f", "$0x80", "$0xff", "$0x100", "$0x7fffffff", "$0x80000000",
              "$0xffffffff"],
    "Imm32S": ["$0", "$0x7f", "$0x80", "$0x7fffffff", "$-1", "$-0x80", "$-0x81",
               "$-0x80000000"],
    "Imm64": ["$0", "$0x7fffffff", "$0x80000000", "$0xffffffff", "$0x100000000", "$-1"],
}
# One register in every position that takes it.
SAME = ["%cl", "%cx", "%ecx", "%rcx", "%r9", "%r9d", "%xmm1", "%ymm2", "%zmm3", "%xmm17",
        "%k2", "%mm3"]
# Every way of writing an address an optimizer can write another way, and
# some it must leave alone.
ADDRESSES = {
    True: ["(,%rcx,2)", "0x8(,%rdx,2)", "(%r13,%rcx)", "(%rbp,%rcx)", "(,%rbp,2)", "(,%r13,2)",
           "sym(,%rcx,2)", "%fs:(%rdx)", "(%ecx)", "0x12(%ecx,%edx,2)", "sym(%ecx)",
           "(%rdx,%rcx)", "(,%rcx,1)", "-0x10", "(%r13,%r13)", "sym(%rcx)", "sym", "sym+8"],
    False: ["(,%ecx,2)", "0x8(,%edx,2)", "(%ebp,%ecx)", "(,%ebp,2)", "%fs:(%edx)", "(%bx)",
            "(%bx,%si)", "(%si)", "(%edx,%ecx)", "(,%ecx,1)", "0xfffffff0", "sym(%ecx)", "sym",
            "sym+8"],
}
# Of the rows with an address, one in this many gets every form of it; `lea`
# and the instructions that take an address apart get them all.
ADDRESS_SHARE = 8
EVERY_ADDRESS = ("lea", "bnd", "tile", "mov", "add", "movaps", "vaddps", "vmovdqu32")

WIDTH = {"b": 1, "w": 2, "l": 4, "q": 8}
GENERAL = {}
for number, names in enumerate([
        "al ax eax rax", "cl cx ecx rcx", "dl dx edx rdx", "bl bx ebx rbx", "spl sp esp rsp",
        "bpl bp ebp rbp", "sil si esi rsi", "dil di edi rdi"]
        + [f"r{n}b r{n}w r{n}d r{n}" for n in range(8, 32)]):
    for width, name in zip((1, 2, 4, 8), names.split()):
        GENERAL[name] = (number, width)
for number, name in enumerate(("ah", "ch", "dh", "bh")):
    GENERAL[name] = (number, 1)


# ─── Statements ───────────────────────────────────────────────────────────

def operand_size(statement: str) -> int | None:
    """The width in bytes a statement operates on, as far as its text says:
    the narrowest general register outside an address."""
    widths = [GENERAL[name][1]
              for name in re.findall(r"%([a-z0-9]+)", re.sub(r"\([^)]*\)", "", statement))
              if name in GENERAL]
    return min(widths) if widths else None


def of_each_width(choices: list[str]) -> list[str]:
    """Of the registers among `choices`, two of each width: the first, and
    the first that needs a prefix byte of its own (`%r9d`, `%sil`)."""
    found: dict[tuple, str] = {}
    for choice in choices:
        name = choice.lstrip("%")
        if not choice.startswith("%") or "(" in choice:
            continue
        width = GENERAL[name][1] if name in GENERAL else name.rstrip("0123456789")
        extended = bool(re.fullmatch(r"r\d+[bwd]?|[sd]il|[sb]pl|[xyz]mm(?:[89]|[1-3]\d)", name))
        found.setdefault((width, extended), choice)
    return list(found.values())


def fits(statement: str, suffix: str = "") -> bool:
    """Whether every number in the statement fits the width it operates on:
    what its suffix says, or its narrowest register, or -- with neither --
    32 bits. (A number that does not fit is cut down by both assemblers,
    each its own way.)"""
    size = WIDTH.get(suffix) or operand_size(statement) or 4
    if size == 8:
        return True
    for text in re.findall(r"\$(-?0x[0-9a-f]+|-?\d+)", statement):
        value = int(text, 0)
        if not -(1 << (8 * size - 1)) <= value < (1 << (8 * size)):
            return False
    return True


def statements(rows, mode: table.Mode, per_row: int, seed: int, only: set[str]) -> dict[str, str]:
    """statement -> the table row it was made from."""
    out: dict[str, str] = {}
    for mnemonic, opcode, cpu, attributes, row_operands in table.rows_of(rows, mode, only):
        suffixes = [""]
        if "NoSuf" not in attributes:
            for suffix, flag in (("b", "No_bSuf"), ("w", "No_wSuf"), ("l", "No_lSuf"),
                                 ("q", "No_qSuf")):
                if flag not in attributes and (mode.long or suffix != "q"):
                    suffixes.append(suffix)
        flagless = "NF" in attributes or "EVexNF" in attributes
        row = f"{mnemonic} {opcode} [{cpu}]"
        for operands in table.orders_of(attributes, row_operands):
            per_operand = []
            for classes in operands:
                choices = list(table.operand_choices(classes, mode, attributes))
                for kind, values in BOUNDARY.items():
                    if kind in classes and (kind != "Imm64" or mode.long):
                        choices += [value for value in values if value not in choices]
                if not choices:
                    break
                per_operand.append(choices)
            else:
                combos = list(itertools.product(*per_operand)) if per_operand else [()]
                if len(combos) > per_row:
                    rest = sorted(combos[1:], key=lambda combo: table.draw(
                        seed, mnemonic, opcode, ", ".join(combo)))
                    combos = [combos[0]] + rest[: per_row - 1]
                first = [choices[0] for choices in per_operand]
                # Every immediate at its edges, into each of a few registers.
                for position, choices in enumerate(per_operand):
                    for value in choices:
                        if not value.startswith("$") or value == "$ext":
                            continue
                        for register in [None] + of_each_width(per_operand[-1]):
                            made = list(first)
                            made[position] = value
                            if register and position != len(per_operand) - 1:
                                made[-1] = register
                            combos.append(tuple(made))
                # One register in every position that takes it.
                for register in SAME:
                    where = [p for p, choices in enumerate(per_operand) if register in choices]
                    if len(where) >= 2:
                        made = list(first)
                        for p in where:
                            made[p] = register
                        combos.append(tuple(made))
                # Every form of address.
                every = mnemonic.startswith(EVERY_ADDRESS) or table.draw(
                    seed, "address", mnemonic, opcode) % ADDRESS_SHARE == 0
                for position, choices in enumerate(per_operand):
                    plain = any(c.startswith("(%") for c in choices)
                    vector = any("mm" in c and "(" in c for c in choices)
                    if every and plain and not vector:
                        for form in ADDRESSES[mode.long]:
                            for other in range(2):
                                made = [c[min(other, len(c) - 1)] for c in per_operand]
                                made[position] = form
                                combos.append(tuple(made))
                for combo in dict.fromkeys(combos):
                    for suffix in suffixes:
                        statement = (mnemonic + suffix + " " + ", ".join(combo)).strip()
                        if not fits(statement, suffix):
                            continue
                        out.setdefault(statement, row)
                        if flagless:
                            out.setdefault("{nf} " + statement, row)
    return out


# ─── The comparison with GNU as ───────────────────────────────────────────

@dataclasses.dataclass
class Sweep:
    """What both assemblers made of the statements, at every level."""
    ordered: list[str]
    theirs: dict[str, dict]          # level -> {index: (bytes, relocations)}
    ours: dict[str, dict]
    lost: dict[str, dict]            # level -> {index: why}: assembled as written only
    crashed: dict[int, str]


def assemble_lccc(ordered: list[str], indexes: list[int], lccc: str, mode: table.Mode,
                  tmp: Path, flags: tuple[str, ...] = ()):
    """What lccc makes of the statements `indexes`: {index: (bytes,
    relocations)} of the ones it assembles, {index: diagnostic} of the ones
    it refuses, {index: what happened} of the ones it does neither with.

    lccc stops at the first statement it refuses, and what it refuses is
    mostly every statement of a mnemonic it does not have. So: batches of
    whole mnemonics; of a batch that fails each mnemonic on its own; of a
    mnemonic that fails each statement on its own if the first one fails,
    and halves otherwise. Batches run side by side.
    """
    def attempt(part: list[int], tag: str):
        source, obj = tmp / f"l{tag}.s", tmp / f"l{tag}.o"
        source.write_text(table.batch_source([ordered[i] for i in part], mode.prelude))
        obj.unlink(missing_ok=True)
        try:
            result = table.run([lccc, *mode.lccc, *flags, "-c", "-o", str(obj), str(source)],
                               timeout=300)
            code, message = result.returncode, (result.stderr or result.stdout)
        except subprocess.TimeoutExpired:
            code, message = None, "timeout after 300 s"
        if code == 0:
            got = table.read_batch(str(obj), len(part)) if obj.exists() else None
            if got is not None:
                return got, None
            message = "exit status 0 and no object that can be read back"
        lines = message.strip().splitlines()
        first = lines[0][:200] if lines else ""
        if code == table.REFUSED and first and "panicked" not in message:
            return None, (False, first)
        status = "no exit status" if code is None else f"exit status {code}"
        return None, (True, f"{status}: {first}" if first else status)

    def batch(groups: list[list[int]], tag: str):
        good, refused, crashed = {}, {}, {}

        def each(part: list[int]) -> None:
            for i in part:
                got, why = attempt([i], tag)
                if got is not None:
                    good[i] = got[0]
                else:
                    (crashed if why[0] else refused)[i] = why[1]

        def halves(part: list[int]) -> None:
            got, _why = attempt(part, tag)
            if got is not None:
                good.update(zip(part, got))
            elif len(part) == 1:
                each(part)
            else:
                halves(part[: len(part) // 2])
                halves(part[len(part) // 2:])

        everything = [i for group in groups for i in group]
        got, _why = attempt(everything, tag)
        if got is not None:
            good.update(zip(everything, got))
            return good, refused, crashed
        for group in groups:
            got, _why = attempt(group, tag) if len(groups) > 1 else (None, None)
            if got is not None:
                good.update(zip(group, got))
                continue
            each(group[:1])
            if group[0] in good:
                halves(group[1:]) if len(group) > 1 else None
            else:
                each(group[1:])
        return good, refused, crashed

    # Whole mnemonics, about 400 statements a batch.
    groups: list[list[int]] = []
    for i in indexes:
        if groups and name_of(ordered[groups[-1][0]]) == name_of(ordered[i]):
            groups[-1].append(i)
        else:
            groups.append([i])
    batches: list[list[list[int]]] = [[]]
    for group in groups:
        if batches[-1] and sum(map(len, batches[-1])) + len(group) > 400:
            batches.append([])
        batches[-1].append(group)
    good, refused, crashed = {}, {}, {}
    with ThreadPoolExecutor(os.cpu_count() or 1) as pool:
        for found in pool.map(lambda numbered: batch(numbered[1], str(numbered[0])),
                              enumerate(batches)):
            good.update(found[0])
            refused.update(found[1])
            crashed.update(found[2])
    return good, refused, crashed


def assemble(ordered: list[str], mode: table.Mode, gas: str | None, lccc: str,
             tmp: Path) -> Sweep:
    theirs: dict[str, dict] = {}
    if gas:
        for level, flags in (("O0", ()), ("O2", ("-O2",)), ("Os", ("-Os",))):
            theirs[level] = table.assemble_gas(
                ordered, gas, dataclasses.replace(mode, gas=(*mode.gas, *flags)), tmp)
        indexes = sorted(theirs["O0"])
    else:
        indexes = list(range(len(ordered)))
    ours, lost = {}, {}
    ours["O0"], _refused, crashed = assemble_lccc(ordered, indexes, lccc, mode, tmp)
    for level in LEVELS:
        ours[level], refused, crashed_here = assemble_lccc(
            ordered, sorted(ours["O0"]), lccc, mode, tmp, (f"-Wa,-{level}",))
        lost[level] = {i: refused.get(i) or crashed_here.get(i) or "no object"
                       for i in ours["O0"] if i not in ours[level]}
    return Sweep(ordered, theirs, ours, lost, crashed)


def reference(sweep: Sweep, i: int, level: str):
    """What lccc's encoding at `level` is held to: GNU as's at -O2, and at
    -Os the shorter of GNU as's two."""
    written = sweep.theirs["O0"][i]
    two = sweep.theirs["O2"].get(i, written)
    if level == "O2":
        return two
    small = sweep.theirs["Os"].get(i, written)
    return small if len(small[0]) < len(two[0]) else two


def name_of(statement: str) -> str:
    words = statement.split()
    return " ".join(words[:2]) if words[0] == "{nf}" else words[0]


def digest_of(items) -> str:
    return hashlib.sha256("\n".join(sorted(items)).encode()).hexdigest()[:8]


def disassemble(encodings: list[bytes], mode: table.Mode, objdump: str, tmp: Path) -> list[str]:
    """The mnemonic objdump reads each encoding as."""
    if not encodings:
        return []
    blob = tmp / "kinds.bin"
    out = []
    # One run each would take minutes; one run of all loses its place at the
    # first prefix it reads otherwise than the assembler meant it. So: one
    # run, the encodings apart by fifteen `nop`s, read back by address.
    gap = 15
    data, starts = bytearray(), []
    for encoding in encodings:
        starts.append(len(data))
        data += encoding + b"\x90" * gap
    blob.write_bytes(bytes(data))
    machine = "i386:x86-64" if mode.long else "i386"
    result = table.run([objdump, "-D", "-b", "binary", "-m", machine, "--no-show-raw-insn",
                        str(blob)])
    at = {}
    for line in result.stdout.splitlines():
        match = re.match(r"\s*([0-9a-f]+):\s+(.*)$", line)
        if match:
            at[int(match.group(1), 16)] = match.group(2).strip()
    for start in starts:
        text = at.get(start, "?")
        words = text.split()
        while words and (words[0] in ("{nf}", "{evex}", "lock", "rex", "data16", "addr32")
                         or words[0].startswith("rex.")):
            words = words[1:]
        out.append(words[0] if words else "?")
    return out


@dataclasses.dataclass
class Report:
    statements: int
    assembled: list[str]    # the statements both assemble as written
    levels: dict            # level -> {"rewritten": [(statement, hex)], "equal": int,
    #                                   "longer": [...], "shorter": [...], "other": [...],
    #                                   "kinds": Counter}
    lost: list[str]
    written: list[str]      # as written: longer than GNU as, or other bytes


def report_of(sweep: Sweep, mode: table.Mode, objdump: str | None, tmp: Path) -> Report:
    ordered = sweep.ordered
    compared = sorted(sweep.ours["O0"])
    written = []
    for i in compared:
        mine, theirs = sweep.ours["O0"][i], sweep.theirs["O0"][i]
        if mine != theirs and len(mine[0]) >= len(theirs[0]):
            written.append(f"{ordered[i]}: lccc {mine[0].hex()} {list(mine[1])} "
                           f"gas {theirs[0].hex()} {list(theirs[1])}")
    levels = {}
    for level in LEVELS:
        found = {"rewritten": [], "equal": 0, "longer": [], "shorter": [], "other": []}
        changed = []
        for i in sorted(sweep.ours[level]):
            mine, plain, target = sweep.ours[level][i], sweep.ours["O0"][i], reference(
                sweep, i, level)
            if mine != plain:
                found["rewritten"].append(f"{ordered[i]} = {mine[0].hex()}")
                changed.append(i)
            if mine == target:
                found["equal"] += 1
            elif len(mine[0]) > len(target[0]):
                found["longer"].append(ordered[i])
            elif len(mine[0]) < len(target[0]):
                found["shorter"].append(ordered[i])
            else:
                found["other"].append(ordered[i])
        kinds: Counter = Counter()
        if objdump:
            before = disassemble([sweep.ours["O0"][i][0] for i in changed], mode, objdump, tmp)
            after = disassemble([sweep.ours[level][i][0] for i in changed], mode, objdump, tmp)
            for i, old, new in zip(changed, before, after):
                prefix = "{nf} " if ordered[i].startswith("{nf}") else ""
                kinds[(prefix + old, new)] += 1
        found["kinds"] = kinds
        levels[level] = found
    lost = sorted(f"-{level}: {ordered[i]}: {why}"
                  for level in LEVELS for i, why in sweep.lost[level].items())
    lost += sorted(f"crash: {ordered[i]}: {why}" for i, why in sweep.crashed.items())
    return Report(len(ordered), [ordered[i] for i in compared], levels, lost, written)


HEADER = """\
# What scripts/asm_optimize_sweep.py finds. Written by --update; --check
# fails on any difference from this file, in either direction.
#
# statements, assembled: how much was swept -- the statements made, and the
# ones both GNU as and lccc assemble as written.
#   swept <mnemonic> <count> <digest>: the latter, by mnemonic. A statement
#     that one of the two starts or stops assembling shows here.
# For each of -O2 and -Os (lccc under -Wa,-O2 against GNU as -O2; lccc under
# -Wa,-Os against the shorter of GNU as -O2 and -Os):
#   rewritten <count> <digest>: the statements lccc encodes otherwise than as
#     written, and a digest of what it makes of each.
#   equal <count>: the statements lccc encodes as GNU as does.
#   longer <statement>: lccc's encoding is longer than GNU as's. Each is a
#     replacement lccc does not make, and tests/asm-optimize/README.md says
#     why.
#   shorter <mnemonic> <count> <digest>: lccc's is the shorter.
#   other <mnemonic> <count> <digest>: the same length, other bytes.
#   kind <written> <rewritten> <count>: what lccc rewrites to what, as
#     objdump reads the two encodings.
"""


def per_mnemonic(items: list[str]) -> dict[str, list[str]]:
    found = defaultdict(list)
    for statement in items:
        found[name_of(statement).replace(" ", "")].append(statement)
    return found


def expectation_text(report: Report) -> str:
    lines = [HEADER.rstrip("\n"), f"statements {report.statements}",
             f"assembled {len(report.assembled)}"]
    lines += [f"swept {mnemonic} {len(items)} {digest_of(items)}"
              for mnemonic, items in sorted(per_mnemonic(report.assembled).items())]
    for level in LEVELS:
        found = report.levels[level]
        lines.append(f"{level} rewritten {len(found['rewritten'])} "
                     f"{digest_of(found['rewritten'])}")
        lines.append(f"{level} equal {found['equal']}")
        lines += [f"{level} longer {statement}" for statement in sorted(found["longer"])]
        for word in ("shorter", "other"):
            lines += [f"{level} {word} {mnemonic} {len(items)} {digest_of(items)}"
                      for mnemonic, items in sorted(per_mnemonic(found[word]).items())]
        lines += [f"{level} kind {old.replace(' ', '')} {new} {count}"
                  for (old, new), count in sorted(found["kinds"].items())]
    return "\n".join(lines) + "\n"


def expectation_path(mode: table.Mode) -> Path:
    return EXPECT_DIR / f"expected-{mode.name}.txt"


def failures_of(report: Report, expected: str | None) -> list[str]:
    """What fails whatever the expectation says, then every line that differs."""
    failures = [f"no longer assembled: {line}" for line in report.lost]
    failures += [f"as written, lccc is not GNU as: {line}" for line in report.written]
    if expected is None:
        return failures
    mine = [line for line in expectation_text(report).splitlines() if not line.startswith("#")]
    theirs = [line for line in expected.splitlines() if line and not line.startswith("#")]
    for line in sorted(set(theirs) - set(mine)):
        failures.append(f"expected, not found:  {line}")
    for line in sorted(set(mine) - set(theirs)):
        failures.append(f"found, not expected:  {line}")
    return failures


# ─── Running both encodings ───────────────────────────────────────────────

# The flags an instruction leaves undefined (SDM Vol. 2), by the mnemonic
# objdump reads the written encoding as: bits of EFLAGS.
CF, PF, AF, ZF, SF, OF = 0x1, 0x4, 0x10, 0x40, 0x80, 0x800
STATUS = CF | PF | AF | ZF | SF | OF
UNDEFINED = {
    "and": AF, "or": AF, "xor": AF, "test": AF,
    "shl": AF, "sal": AF,
    "bt": OF | SF | AF | PF, "bts": OF | SF | AF | PF, "btr": OF | SF | AF | PF,
    "btc": OF | SF | AF | PF,
}


def undefined_flags(statement: str) -> int:
    name = name_of(statement)
    for stem, flags in UNDEFINED.items():
        if name == stem or (name.startswith(stem) and name[len(stem):] in WIDTH):
            return flags
    return 0


def address_of(statement: str, long: bool):
    """The address a statement names: ({register number: times}, bits,
    whether the statement reads or writes there), or None if it names none
    or one that cannot be made to point anywhere."""
    name = name_of(statement)
    operands = statement.split(None, 1)[1] if " " in statement else ""
    if name.startswith("{nf}"):
        operands = operands.split(None, 1)[1] if " " in operands else ""
    match = re.search(r"(-?(?:0x[0-9a-f]+|\d+))?\(([^)]*)\)", operands)
    if not match or "%st(" in operands:
        return None
    parts = [part.strip() for part in match.group(2).split(",")]
    terms: Counter = Counter()
    bits = 0
    for position, part in enumerate(parts[:2]):
        if not part:
            continue
        register = GENERAL.get(part.lstrip("%"))
        if register is None:
            return None
        scale = int(parts[2]) if position == 1 and len(parts) > 2 and parts[2] else 1
        terms[register[0]] += scale
        bits = register[1] * 8
    displacement = int(match.group(1), 0) if match.group(1) else 0
    accessed = not name.replace("{nf} ", "").startswith("lea")
    return terms, displacement, bits, accessed


TRAMPOLINE = r"""
#define GPR(n) (lccc_in + WORD * (n))
#define OUT(n) (lccc_out + WORD * (n))
    .text
    .globl lccc_run
    .type lccc_run, @function
lccc_run:
#ifdef __x86_64__
    push %rbx
    push %rbp
    push %r12
    push %r13
    push %r14
    push %r15
    mov %rsp, lccc_saved_sp(%rip)
    cmpl $1, lccc_level(%rip)
    jb 1f
    je 2f
    {load512}
    jmp 3f
1:  {load128}
    jmp 3f
2:  {load256}
3:  fninit
    {loadmm}
    ldmxcsr lccc_in+MXCSR(%rip)
    pushq lccc_in+FLAGS(%rip)
    popfq
    {loadgpr}
    jmp *lccc_target(%rip)
    .globl lccc_finish
lccc_finish:
    {savegpr}
    mov lccc_saved_sp(%rip), %rsp
    pushfq
    popq lccc_out+FLAGS(%rip)
    stmxcsr lccc_out+MXCSR(%rip)
    cmpl $1, lccc_level(%rip)
    jb 1f
    je 2f
    {save512}
    jmp 3f
1:  {save128}
    jmp 3f
2:  {save256}
3:  {savemm}
    emms
    cmpl $1, lccc_level(%rip)
    jb 4f
    vzeroupper
4:  pop %r15
    pop %r14
    pop %r13
    pop %r12
    pop %rbp
    pop %rbx
    ret
#else
    push %ebx
    push %ebp
    push %esi
    push %edi
    mov %esp, lccc_saved_sp
    cmpl $1, lccc_level
    jb 1f
    je 2f
    {load512}
    jmp 3f
1:  {load128}
    jmp 3f
2:  {load256}
3:  fninit
    {loadmm}
    ldmxcsr lccc_in+MXCSR
    pushl lccc_in+FLAGS
    popfl
    {loadgpr}
    jmp *lccc_target
    .globl lccc_finish
lccc_finish:
    {savegpr}
    mov lccc_saved_sp, %esp
    pushfl
    popl lccc_out+FLAGS
    stmxcsr lccc_out+MXCSR
    cmpl $1, lccc_level
    jb 1f
    je 2f
    {save512}
    jmp 3f
1:  {save128}
    jmp 3f
2:  {save256}
3:  {savemm}
    emms
    cmpl $1, lccc_level
    jb 4f
    vzeroupper
4:  pop %edi
    pop %esi
    pop %ebp
    pop %ebx
    ret
#endif
    .size lccc_run, .-lccc_run

    .globl lccc_clean
    .type lccc_clean, @function
lccc_clean:
    emms
#ifdef __x86_64__
    cmpl $1, lccc_level(%rip)
#else
    cmpl $1, lccc_level
#endif
    jb 1f
    vzeroupper
1:  ret
    .size lccc_clean, .-lccc_clean
    .section .note.GNU-stack,"",@progbits
"""

DRIVER = r"""
#define _GNU_SOURCE
#include <setjmp.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>

#define NGPR ((int)(sizeof(void *) == 8 ? 16 : 8))
#define NVEC ((int)(sizeof(void *) == 8 ? 32 : 8))
#define ARENA 8192
#define STATUS 0x8d5

struct state {
    uintptr_t gpr[16];
    uintptr_t flags;
    uint32_t mxcsr, pad;
    uint64_t k[8];
    uint64_t mm[8];
    uint8_t vec[32][64];
};
struct state lccc_in, lccc_out;
uintptr_t lccc_saved_sp, lccc_target;
int lccc_level;
void lccc_run(void);
void lccc_clean(void);

struct test {
    const char *text;
    const unsigned char *written, *replaced;
    unsigned undefined;        /* flags the written instruction leaves undefined */
    signed char reg[2];        /* the registers of its address, -1 for none */
    unsigned char times[2];
    long long disp;
    unsigned char bits, accessed;
};
#include "cases.inc"

static sigjmp_buf back;
static void caught(int sig) { siglongjmp(back, sig); }

static uint64_t seed;
static uint64_t rnd(void) {
    seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
    return seed * 0x2545F4914F6CDD1DULL;
}
static uintptr_t value(void) {
    int bits = 8 * (int)sizeof(uintptr_t);
    switch (rnd() % 10) {
    case 0: return 0;
    case 1: return 1;
    case 2: return (uintptr_t)-1;
    case 3: return (uintptr_t)1 << (bits - 1);
    case 4: return ((uintptr_t)1 << (bits - 1)) - 1;
    case 5: return (uintptr_t)(rnd() & 0xff);
    case 6: return (uintptr_t)(rnd() & 0xffffffffu);
    default: return (uintptr_t)rnd();
    }
}

static unsigned char *arena, *stack;
static unsigned char arena_before[ARENA], arena_after[2][ARENA];
static struct state after[2];

static int level(void) {
    __builtin_cpu_init();
    if (__builtin_cpu_supports("avx512f") && __builtin_cpu_supports("avx512bw")
        && __builtin_cpu_supports("avx512dq"))
        return 2;
    return __builtin_cpu_supports("avx") ? 1 : 0;
}

/* Registers for the statement's address to be `where`. */
static int point(const struct test *t, unsigned char *where) {
    uint64_t want = (uint64_t)(uintptr_t)where - (uint64_t)t->disp;
    uint64_t mask = t->bits == 64 ? ~0ULL : ((1ULL << t->bits) - 1);
    if (t->reg[0] < 0)
        return 0;
    if (t->bits < 8 * sizeof(void *) && ((uintptr_t)where >> t->bits) != 0)
        return 0;                   /* an address this narrow reaches no memory of ours */
    for (unsigned other = 0; other < 64; other++) {
        uint64_t rest = want;
        if (t->reg[1] >= 0)
            rest -= (uint64_t)t->times[1] * other;
        if (rest % t->times[0])
            continue;
        uint64_t first = rest / t->times[0];
        /* What a narrower address does not use is anything. */
        uint64_t high = t->bits == 64 ? 0 : (rnd() & ~mask);
        lccc_in.gpr[(int)t->reg[0]] = (uintptr_t)((first & mask) | high);
        if (t->reg[1] >= 0)
            lccc_in.gpr[(int)t->reg[1]] = (uintptr_t)(other | high);
        return 1;
    }
    return 0;
}

static int run(const unsigned char *code, int which) {
    memcpy(arena, arena_before, ARENA);
    memset(&lccc_out, 0, sizeof lccc_out);
    lccc_target = (uintptr_t)code;
    int sig = sigsetjmp(back, 1);
    if (sig == 0)
        lccc_run();
    else
        lccc_clean();
    after[which] = lccc_out;
    memcpy(arena_after[which], arena, ARENA);
    return sig;
}

int main(int argc, char **argv) {
    int rounds = argc > 1 ? atoi(argv[1]) : 24;
    int flags = MAP_PRIVATE | MAP_ANONYMOUS;
#ifdef MAP_32BIT
    flags |= MAP_32BIT;             /* reachable by a 32-bit address too */
#endif
    arena = mmap(0, ARENA, PROT_READ | PROT_WRITE, flags, -1, 0);
    stack = mmap(0, ARENA, PROT_READ | PROT_WRITE, flags, -1, 0);
    static unsigned char alternate[1 << 16];
    stack_t ss = { .ss_sp = alternate, .ss_size = sizeof alternate };
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = caught;
    sa.sa_flags = SA_ONSTACK | SA_NODEFER;
    if (arena == MAP_FAILED || stack == MAP_FAILED || sigaltstack(&ss, 0))
        return 2;
    sigaction(SIGSEGV, &sa, 0); sigaction(SIGBUS, &sa, 0); sigaction(SIGILL, &sa, 0);
    sigaction(SIGFPE, &sa, 0); sigaction(SIGTRAP, &sa, 0);
    lccc_level = level();
    int nvec = lccc_level == 2 ? NVEC : (NVEC > 16 ? 16 : NVEC);
    int veclen = lccc_level == 2 ? 64 : lccc_level == 1 ? 32 : 16;
    long ran = 0, skipped = 0, failed = 0;
    for (unsigned n = 0; n < sizeof tests / sizeof tests[0]; n++) {
        const struct test *t = &tests[n];
        const char *why = 0;
        int executed = 0;
        seed = 0x9E3779B97F4A7C15ULL ^ ((uint64_t)n << 20);
        for (int round = 0; round < rounds && !why; round++) {
            for (int i = 0; i < NGPR; i++)
                lccc_in.gpr[i] = value();
            lccc_in.gpr[4] = (uintptr_t)(stack + ARENA / 2);
            lccc_in.flags = 0x202 | (rnd() & STATUS);
            lccc_in.mxcsr = 0x1f80;
            for (unsigned i = 0; i < sizeof lccc_in.vec; i++)
                ((unsigned char *)lccc_in.vec)[i] = (unsigned char)rnd();
            for (int i = 0; i < 8; i++) {
                lccc_in.k[i] = rnd();
                lccc_in.mm[i] = rnd();
            }
            for (int i = 0; i < ARENA; i++)
                arena_before[i] = (unsigned char)rnd();
            /* Where it cannot be pointed, both fault or neither does. */
            if (t->accessed)
                point(t, arena + ARENA / 2);
            int a = run(t->written, 0), b = run(t->replaced, 1);
            static char said[256];
            if (a == SIGILL) {
                skipped++;
                goto next;
            }
            if (b == SIGILL) {
                why = "the replacement needs what the written instruction does not";
                break;
            }
            if (a != b) { snprintf(said, sizeof said, "signal %d against %d", a, b); why = said; break; }
            executed = 1;
            if (a)
                continue;
            for (int i = 0; i < NGPR && !why; i++)
                if (after[0].gpr[i] != after[1].gpr[i]) {
                    snprintf(said, sizeof said, "register %d: %#llx against %#llx", i,
                             (unsigned long long)after[0].gpr[i], (unsigned long long)after[1].gpr[i]);
                    why = said;
                }
            uintptr_t mask = STATUS & ~(uintptr_t)t->undefined;
            if (!why && ((after[0].flags ^ after[1].flags) & mask)) {
                snprintf(said, sizeof said, "flags %#llx against %#llx (of %#llx)",
                         (unsigned long long)(after[0].flags & mask),
                         (unsigned long long)(after[1].flags & mask), (unsigned long long)mask);
                why = said;
            }
            if (!why && after[0].mxcsr != after[1].mxcsr) why = "MXCSR";
            for (int i = 0; i < nvec && !why; i++)
                if (memcmp(after[0].vec[i], after[1].vec[i], veclen)) {
                    snprintf(said, sizeof said, "vector register %d", i);
                    why = said;
                }
            if (!why && lccc_level == 2 && memcmp(after[0].k, after[1].k, sizeof after[0].k))
                why = "a mask register";
            if (!why && memcmp(after[0].mm, after[1].mm, sizeof after[0].mm))
                why = "an MMX register";
            if (!why && memcmp(arena_after[0], arena_after[1], ARENA))
                why = "memory";
        }
        if (why) {
            failed++;
            printf("DIFFERENT %s: %s\n", t->text, why);
        } else if (executed) {
            ran++;
        }
next:   ;
    }
    printf("RAN %ld SKIPPED %ld DIFFERENT %ld LEVEL %d\n", ran, skipped, failed, lccc_level);
    return failed ? 1 : 0;
}
"""


def harness(pairs, mode: table.Mode, directory: Path) -> None:
    """Write the sources of the program that runs `pairs`: (statement,
    written bytes, replaced bytes)."""
    word = 8 if mode.long else 4
    registers = (["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi"]
                 + [f"r{n}" for n in range(8, 16)]) if mode.long else [
        "eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi"]
    rip = "(%rip)" if mode.long else ""
    flags_at = 16 * word
    mxcsr_at = flags_at + word
    k_at = mxcsr_at + 8
    mm_at = k_at + 64
    vec_at = mm_at + 64
    vectors = 32 if mode.long else 8

    def lines(items):
        return "\n    ".join(items)

    # The stack pointer last on the way in: everything before it uses it.
    order = [n for n in range(len(registers)) if n != 4] + [4]
    fill = {
        "loadgpr": lines(f"mov lccc_in+{word * n}{rip}, %{registers[n]}" for n in order),
        "savegpr": lines(f"mov %{registers[n]}, lccc_out+{word * n}{rip}"
                         for n in range(len(registers))),
        "loadmm": lines(f"movq lccc_in+{mm_at + 8 * n}{rip}, %mm{n}" for n in range(8)),
        "savemm": lines(f"movq %mm{n}, lccc_out+{mm_at + 8 * n}{rip}" for n in range(8)),
        "load128": lines(f"movdqu lccc_in+{vec_at + 64 * n}{rip}, %xmm{n}"
                         for n in range(min(vectors, 16))),
        "save128": lines(f"movdqu %xmm{n}, lccc_out+{vec_at + 64 * n}{rip}"
                         for n in range(min(vectors, 16))),
        "load256": lines(f"vmovdqu lccc_in+{vec_at + 64 * n}{rip}, %ymm{n}"
                         for n in range(min(vectors, 16))),
        "save256": lines(f"vmovdqu %ymm{n}, lccc_out+{vec_at + 64 * n}{rip}"
                         for n in range(min(vectors, 16))),
        "load512": lines([f"vmovdqu64 lccc_in+{vec_at + 64 * n}{rip}, %zmm{n}"
                          for n in range(vectors)]
                         + [f"kmovq lccc_in+{k_at + 8 * n}{rip}, %k{n}" for n in range(8)]),
        "save512": lines([f"vmovdqu64 %zmm{n}, lccc_out+{vec_at + 64 * n}{rip}"
                          for n in range(vectors)]
                         + [f"kmovq %k{n}, lccc_out+{k_at + 8 * n}{rip}" for n in range(8)]),
    }
    trampoline = TRAMPOLINE
    for key, text in fill.items():
        trampoline = trampoline.replace("{" + key + "}", text)
    defines = (f"#define WORD {word}\n#define FLAGS {flags_at}\n#define MXCSR {mxcsr_at}\n")
    (directory / "trampoline.S").write_text(defines + trampoline)

    stubs = ["    .text"]
    cases = ["static const struct test tests[] = {"]
    declared = []
    for n, (statement, written, replaced) in enumerate(pairs):
        for which, code in (("a", written), ("b", replaced)):
            stubs.append(f"    .globl lccc_case_{n}_{which}\nlccc_case_{n}_{which}:")
            stubs.append("    .byte " + ", ".join(f"0x{byte:02x}" for byte in code))
            stubs.append("    jmp lccc_finish")
        declared.append(f"extern const unsigned char lccc_case_{n}_a[], lccc_case_{n}_b[];")
        address = address_of(statement, mode.long)
        reg, times, disp, bits, accessed = [-1, -1], [1, 1], 0, 8 * word, 0
        if address:
            terms, disp, bits, accessed = address
            for slot, (number, count) in enumerate(sorted(terms.items(), key=lambda t: t[1])):
                reg[slot], times[slot] = number, count
            accessed = int(accessed)
        text = statement.replace("\\", "\\\\").replace('"', '\\"')
        cases.append(f'    {{"{text}", lccc_case_{n}_a, lccc_case_{n}_b, '
                     f"{undefined_flags(statement):#x}, {{{reg[0]}, {reg[1]}}}, "
                     f"{{{times[0]}, {times[1]}}}, {disp}LL, {bits or 8 * word}, {accessed}}},")
    cases.append("};")
    stubs.append('    .section .note.GNU-stack,"",@progbits')
    (directory / "stubs.S").write_text("\n".join(stubs) + "\n")
    (directory / "cases.inc").write_text("\n".join(declared + cases) + "\n")
    (directory / "driver.c").write_text(DRIVER)


def compiler_for(mode: table.Mode, given: str | None) -> list[str] | None:
    """The C compiler that builds the program for `mode`, if there is one."""
    if given:
        return given.split()
    if mode.long:
        found = shutil.which("cc") or shutil.which("gcc")
        return [found] if found else None
    wrapper = os.environ.get("LCCC_M32_CC")
    if wrapper:
        return wrapper.split()
    found = shutil.which("cc") or shutil.which("gcc")
    return [found, "-m32"] if found else None


def execute(pairs, mode: table.Mode, cc: list[str], rounds: int = 24):
    """(ran, skipped, [what differs]) of running `pairs` both ways, or a
    string saying why the program could not be built."""
    if not pairs:
        return 0, 0, []
    with tempfile.TemporaryDirectory() as directory:
        tmp = Path(directory)
        harness(pairs, mode, tmp)
        binary = tmp / "run"
        build = table.run([*cc, "-O1", "-fno-pie", "-no-pie", "-o", str(binary),
                           str(tmp / "driver.c"), str(tmp / "trampoline.S"),
                           str(tmp / "stubs.S")])
        if build.returncode != 0:
            return "cannot build the program that runs them:\n" + build.stderr[-1200:]
        result = table.run([str(binary), str(rounds)], timeout=1800)
    summary = re.search(r"RAN (\d+) SKIPPED (\d+) DIFFERENT (\d+) LEVEL (\d)", result.stdout)
    if not summary:
        return (f"the program that runs them died (status {result.returncode}):\n"
                + (result.stdout + result.stderr)[-1200:])
    different = [line[len("DIFFERENT "):] for line in result.stdout.splitlines()
                 if line.startswith("DIFFERENT ")]
    return int(summary.group(1)), int(summary.group(2)), different


def runnable(statement: str, written, replaced) -> bool:
    """Whether a pair can be run from any state: no relocation in it, no
    address in a segment that is not ours, and nothing APX (`62` with map 4,
    `d5`), which no processor has yet and an older one reads as something
    else in 32-bit code."""
    if written[1] or replaced[1]:
        return False
    if re.search(r"%[fg]s:", statement) and not name_of(statement).startswith("lea"):
        return False
    return True


def pairs_of(sweep: Sweep) -> list[tuple[str, bytes, bytes]]:
    """Every (statement, written, replaced) that can be run, each replacement once."""
    seen, out = set(), []
    for level in LEVELS:
        for i in sorted(sweep.ours[level]):
            written, replaced = sweep.ours["O0"][i], sweep.ours[level][i]
            if written == replaced or not runnable(sweep.ordered[i], written, replaced):
                continue
            key = (written[0], replaced[0])
            if key not in seen:
                seen.add(key)
                out.append((sweep.ordered[i], written[0], replaced[0]))
    return out


def written_pairs_of(sweep: Sweep) -> list[tuple[str, bytes, bytes]]:
    """Every (statement, GNU as's bytes, lccc's bytes) of the statements the
    two encode differently as written, each pair of encodings once: lccc
    folds an index into the base, takes the short immediate, exchanges the
    sources of a commutative instruction -- and each is the same
    instruction, or this finds it."""
    seen, out = set(), []
    for i in sorted(sweep.ours["O0"]):
        theirs, ours = sweep.theirs["O0"].get(i), sweep.ours["O0"][i]
        if theirs is None or theirs[0] == ours[0] or not runnable(sweep.ordered[i], theirs, ours):
            continue
        key = (theirs[0], ours[0])
        if key not in seen:
            seen.add(key)
            out.append((sweep.ordered[i], theirs[0], ours[0]))
    return out


# What a run has to tell apart: a statement, and bytes that are not it.
PLANTED = (
    ("another value", "movl $1, %eax", "b801000000", "b802000000"),
    ("the upper half of a register", "movq $-1, %rax", "48c7c0ffffffff", "b8ffffffff"),
    ("a flag", "addl %ecx, %ecx", "01c9", "8d0c09"),
    # `and` of a 32-bit register with itself clears the upper half; `test` does not.
    ("a register that is no longer written", "andl %ecx, %ecx", "21c9", "85c9"),
    ("the rest of a vector register", "vmovaps %xmm1, %xmm2", "c5f828d1", "0f28d1"),
    ("another byte of memory", "movl %ecx, (%rdx)", "890a", "48890a"),
    ("an instruction this processor does not have", "nop", "90", "0f0b"),
    ("a store into another place", "movq %rcx, (%rdx)", "48890a", "48894a08"),
)


def self_test(cc: list[str]) -> list[str]:
    """What is wrong with the run, found by planting replacements it has to refuse."""
    mode = table.MODES["64"]
    problems = []
    for what, statement, written, replaced in PLANTED:
        got = execute([(statement, bytes.fromhex(written), bytes.fromhex(replaced))], mode, cc)
        if isinstance(got, str):
            return [got]
        # (Skipped: the written instruction is one this processor lacks.)
        if not got[2] and not got[1]:
            problems.append(f"not found: {what} ({statement}: {written} against {replaced})")
    # And two that are the same, which it has to pass.
    same = [("movq $1, %rax", bytes.fromhex("48c7c001000000"), bytes.fromhex("b801000000")),
            ("andq %rcx, %rcx", bytes.fromhex("4821c9"), bytes.fromhex("4885c9"))]
    got = execute(same, mode, cc)
    if isinstance(got, str):
        return [got]
    if got[2] or got[0] != len(same):
        problems.append(f"refused what is the same: ran {got[0]}, {got[2]}")
    return problems


# ─── Command line ─────────────────────────────────────────────────────────

def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--32", dest="m32", action="store_true", help="32-bit code")
    parser.add_argument("--check", action="store_true", help="compare with the expectation file")
    parser.add_argument("--update", action="store_true", help="write the expectation file")
    parser.add_argument("--execute", action="store_true",
                        help="run every rewritten statement both ways on this processor")
    parser.add_argument("--self-test", action="store_true",
                        help="plant wrong replacements and require the run to find each")
    parser.add_argument("--insn", action="append", default=[], help="a statement (repeatable)")
    parser.add_argument("--file", help="statements, one a line")
    parser.add_argument("--only", action="append", help="only this mnemonic's rows (repeatable)")
    parser.add_argument("--per-row", type=int, default=PER_ROW)
    parser.add_argument("--seed", type=int, default=SEED)
    parser.add_argument("--minimum", type=int, default=0,
                        help="with --execute: fail if fewer statements than this were run")
    parser.add_argument("--gas", default=os.environ.get("LCCC_GAS", ""))
    parser.add_argument("--objdump", default=os.environ.get("LCCC_OBJDUMP", ""))
    parser.add_argument("--lccc", default=os.environ.get("LCCC", ""))
    parser.add_argument("--cc", default=os.environ.get("LCCC_EXEC_CC", ""),
                        help="the C compiler for --execute")
    parser.add_argument("--table", help="i386-opc.tbl (default: the pinned one)")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args(argv)

    mode = table.MODES["32" if args.m32 else "64"]
    cc = compiler_for(mode, args.cc)
    if args.self_test:
        if not cc:
            print("asm_optimize_sweep: no C compiler for --self-test", file=sys.stderr)
            return 2
        problems = self_test(compiler_for(table.MODES["64"], args.cc))
        for problem in problems:
            print("self-test:", problem)
        print("self-test:", "FAILED" if problems else f"ok ({len(PLANTED)} planted, all found)")
        return 1 if problems else 0

    lccc = args.lccc or str(REPO / "target" / "release" / ("lccc-i686" if args.m32 else "lccc-x86"))
    if not Path(lccc).exists():
        print(f"asm_optimize_sweep: no lccc at {lccc} (--lccc, or LCCC=)", file=sys.stderr)
        return 2
    given = list(args.insn)
    if args.file:
        given += [line.strip() for line in Path(args.file).read_text().splitlines()
                  if line.strip() and not line.startswith("#")]
    compare = args.check or args.update or not args.execute
    gas = objdump = None
    if compare:
        gas = args.gas or shutil.which("as")
        objdump = args.objdump or shutil.which("objdump")
        if not gas or not table.GAS_VERSION.fullmatch(table.gas_version(gas)):
            print("asm_optimize_sweep: the comparison needs GNU as 2.47 (--gas, or LCCC_GAS=)",
                  file=sys.stderr)
            return 2
    if given:
        made = {statement: "given" for statement in given}
    else:
        rows = list(table.expand_rows(table.table_text(args.table)))
        made = statements(rows, mode, args.per_row, args.seed, set(args.only or []))
    ordered = sorted(made)
    status = 0
    with tempfile.TemporaryDirectory() as directory:
        tmp = Path(directory)
        sweep = assemble(ordered, mode, gas, lccc, tmp)
        if compare:
            report = report_of(sweep, mode, objdump, tmp)
            print(f"{mode.name}: {report.statements} statements, {len(report.assembled)} assembled "
                  f"by both as written")
            for level in LEVELS:
                found = report.levels[level]
                print(f"  -{level}: rewritten {len(found['rewritten'])}, as GNU as "
                      f"{found['equal']}, shorter {len(found['shorter'])}, other bytes "
                      f"{len(found['other'])}, longer {len(found['longer'])}")
                if args.verbose:
                    for word in ("longer", "other"):
                        for statement in found[word][:40]:
                            i = ordered.index(statement)
                            print(f"     {word}: {statement}: lccc "
                                  f"{sweep.ours[level][i][0].hex()} gas "
                                  f"{reference(sweep, i, level)[0].hex()}")
            path = expectation_path(mode)
            pinned = not given and not args.only and (args.per_row, args.seed) == (PER_ROW, SEED)
            if args.update:
                failures = failures_of(report, None)
                if not pinned:
                    print("asm_optimize_sweep: --update takes the whole table at the default "
                          "--per-row and --seed", file=sys.stderr)
                    return 2
                if not failures:
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(expectation_text(report))
                    print(f"wrote {path.relative_to(REPO)}")
            else:
                expected = path.read_text() if (args.check and pinned and path.exists()) else None
                if args.check and pinned and expected is None:
                    print(f"asm_optimize_sweep: no {path.relative_to(REPO)}", file=sys.stderr)
                    return 2
                failures = failures_of(report, expected)
            for failure in failures[:60]:
                print("  " + failure)
            if len(failures) > 60:
                print(f"  ... and {len(failures) - 60} more")
            if failures:
                status = 1
        if args.execute:
            if not cc:
                print("asm_optimize_sweep: no C compiler for --execute (--cc)", file=sys.stderr)
                return 2
            runs = [("replacements", pairs_of(sweep), args.minimum)]
            if gas:
                runs.append(("encodings that are not GNU as's as written",
                             written_pairs_of(sweep), 0))
            for what, pairs, minimum in runs:
                got = execute(pairs, mode, cc)
                if isinstance(got, str):
                    print("asm_optimize_sweep: " + got, file=sys.stderr)
                    return 2
                ran, skipped, different = got
                print(f"{mode.name}: {len(pairs)} {what}; run both ways {ran}, not run by "
                      f"this processor {skipped}, different {len(different)}")
                for line in different[:60]:
                    print("  DIFFERENT " + line)
                if different or ran < minimum:
                    if ran < minimum:
                        print(f"  only {ran} were run; --minimum {minimum}")
                    status = 1
    return status


if __name__ == "__main__":
    sys.exit(main())
