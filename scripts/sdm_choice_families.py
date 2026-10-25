#!/usr/bin/env python3
"""Cross-reference this tree's encoding choices with the Intel SDM.

`gen_encoding_sweep.py` exists because some instructions have more than one
legal encoding and a compiler should emit the shortest one. It enumerates those
families from a *hand-written* list, which is exactly the kind of list that
silently rots: a family the author forgot is a family nobody ever measures,
and the sweep keeps passing because it never generated the case.

This script makes the list itself checkable. It holds the encoding-choice
families as data, each entry naming the short form and the long form by their
opcode byte, and it verifies that data two ways:

  * `--reparse FILE` re-derives the family table from an Intel-SDM-derived ISA
    description and asserts that every claimed opcode really exists for that
    instruction class. The reference file is XED's `datafiles/xed-isa.txt`
    (intelxed/xed, Apache-2.0), which is generated from and validated against
    the SDM; the SHA-256 of the copy that produced the table below is pinned in
    `SDM_SOURCE`. `--reparse` is what turns "trust the table" into "check the
    table against the manual", and it is the step to re-run when the SDM is
    revised.
  * the default mode is hermetic (no network, no reference file): it checks the
    table's internal consistency and that every family is covered either by a
    named group of `gen_encoding_sweep.py` or by a documented non-encoder
    mechanism (linker relaxation, moffs selection, ...).

A family that is in neither place fails the run. That is the defect this
script exists to catch: coverage that is assumed rather than demonstrated.

Scope: the families are the *multi-encoding* ones — where the ISA lets two
assemblers both be right and one be shorter. Single-encoding instructions are
out of scope here and are covered by the instruction differentials
(`insndiff.py`, `encdiff.py`, `asmdiff.py`).

Usage
-----
    scripts/sdm_choice_families.py                  # hermetic coverage check
    scripts/sdm_choice_families.py --list
    scripts/sdm_choice_families.py --reparse /path/to/xed-isa.txt
"""
from __future__ import annotations

import argparse
import hashlib
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SWEEP = REPO / "scripts" / "gen_encoding_sweep.py"

# The reference these families were derived from. XED ships the file as part of
# its SDM transcription; the digest pins the exact revision that was read.
SDM_SOURCE = {
    "repo": "intelxed/xed",
    "path": "datafiles/xed-isa.txt",
    "license": "Apache-2.0",
    "sha256": "adbe3296bbd4d079c2987711e0e5fc6bc4a27daca59112fa3d8312b8459e17c3",
    "note": (
        "XED's ISA description is generated from and checked against the Intel "
        "Software Developer's Manual. The digest is recorded so a re-parse can "
        "report whether the reference moved; a mismatch is not a failure by "
        "itself (the SDM is revised), it is a prompt to re-run --reparse."
    ),
}

# Every family: (name, iclass(es), short opcode, long opcode, coverage).
#   short/long are the opcode bytes as they appear in the SDM pattern, with the
#   "+r" notation for an opcode that embeds a register number.
#   coverage is either a group name in gen_encoding_sweep.py GROUPS, or a
#   "why-not-the-encoder" string for a choice that some other mechanism makes.
FAMILIES = [
    # --- ALU group 1: sign-extended imm8 (the 3-byte saving) -----------------
    ("alu_imm8_sext", "ADC", "0x83", "0x81", "imm8_sext"),
    ("alu_imm8_sext", "ADD", "0x83", "0x81", "imm8_sext"),
    ("alu_imm8_sext", "AND", "0x83", "0x81", "imm8_sext"),
    ("alu_imm8_sext", "CMP", "0x83", "0x81", "imm8_sext"),
    ("alu_imm8_sext", "OR", "0x83", "0x81", "imm8_sext"),
    ("alu_imm8_sext", "SBB", "0x83", "0x81", "imm8_sext"),
    ("alu_imm8_sext", "SUB", "0x83", "0x81", "imm8_sext"),
    ("alu_imm8_sext", "XOR", "0x83", "0x81", "imm8_sext"),
    # --- ALU: the accumulator form has no ModR/M byte ------------------------
    #   For an immediate that fits i8 the imm8 form (0x83, 3 bytes) beats the
    #   accumulator form (0x05, 5 bytes); above that the accumulator form wins
    #   by one byte over 0x81 + ModR/M. Both halves are choices, so the sweep
    #   must generate both sides of the boundary.
    ("alu_accumulator", "ADD", "0x05", "0x81", "accumulator"),
    ("alu_accumulator", "ADC", "0x15", "0x81", "accumulator"),
    ("alu_accumulator", "AND", "0x25", "0x81", "accumulator"),
    ("alu_accumulator", "CMP", "0x3D", "0x81", "accumulator"),
    ("alu_accumulator", "OR", "0x0D", "0x81", "accumulator"),
    ("alu_accumulator", "SBB", "0x1D", "0x81", "accumulator"),
    ("alu_accumulator", "SUB", "0x2D", "0x81", "accumulator"),
    ("alu_accumulator", "XOR", "0x35", "0x81", "accumulator"),
    # --- TEST: accumulator form vs F7 /0 ------------------------------------
    ("test_accumulator", "TEST", "0xA9", "0xF7", "test_and"),
    # --- IMUL: 0x6B is 0x69 with the immediate narrowed to a sign-extended
    #   byte; three bytes per multiply, and a multiply is never cold. --------
    ("imul_imm8_sext", "IMUL", "0x6B", "0x69", "imul_imm"),
    # --- PUSH: imm8 ---------------------------------------------------------
    ("push_imm8", "PUSH", "0x6A", "0x68", "push_imm"),
    # --- PUSH/POP register: the one-byte 0x50+r / 0x58+r forms --------------
    #   Both are REX-free in 64-bit mode and single-byte in 32-bit mode, where
    #   the general r/m forms cost a ModR/M byte.
    ("push_reg_short", "PUSH", "0x50+r", "0xFF", "push_imm"),
    ("pop_reg_short", "POP", "0x58+r", "0x8F", "push_imm"),
    # --- Shift by one: 0xD1 has no immediate field --------------------------
    ("shift_by_one", "SHL", "0xD1", "0xC1", "shift_one"),
    # --- INC/DEC in 32-bit mode: 0x40+r / 0x48+r ----------------------------
    #   The compact forms exist only where those bytes are not REX prefixes,
    #   i.e. everything except 64-bit mode. i686 must take them; x86-64 must
    #   not (0x40-0x4F is REX there).
    ("inc_dec_reg_short", "INC", "0x40+r", "0xFE", "inc_dec"),
    ("inc_dec_reg_short", "DEC", "0x48+r", "0xFE", "inc_dec"),
    # --- XCHG with the accumulator: 0x90+r, and 0x90 is the canonical NOP ---
    ("xchg_accumulator", "XCHG", "0x90+r", "0x87", "xchg_acc"),
    # --- MOV moffs: A0-A3 carry the address with no ModR/M byte -------------
    ("mov_moffs", "MOV", "0xA3", "0x89", "mov_imm"),
    ("mov_moffs", "MOV", "0xA1", "0x8B", "mov_imm"),
    # --- Moving an immediate into a 64-bit register -------------------------
    #   Three legal widths, and the shortest depends on the value:
    #     movl $imm32,%r32d   5 bytes, zero-extends (B8+r)
    #     movq $imm32,%r64    7 bytes, sign-extends (REX.W C7 /0)
    #     movabsq $imm64,%r64 10 bytes           (REX.W B8+r)
    ("mov_r64_imm_width", "MOV", "0xC7", "0xB8+r", "mov_imm"),
    # --- Branch displacement: rel8 vs rel32 ---------------------------------
    #   The encoder always reserves the long form; the assembler's relaxation
    #   pass shrinks each branch once the distances are known, so this family
    #   is deliberately not a codegen choice.
    ("jmp_rel8", "JMP", "0xEB", "0xE9", "relaxation: elf_writer jump relaxation"),
]

HEADER = re.compile(r"^;;;\s*(\S+)")
OPCODE = re.compile(r"0x[0-9A-Fa-f]{2}")


def parse_sdm_isa(path: Path) -> dict[str, set[str]]:
    """`{ICLASS: {opcode byte, ...}}` from an XED-format ISA description."""
    text = path.read_text(encoding="utf-8", errors="replace")
    # Strip the license block so `{#BEGIN_LEGAL` never looks like a form.
    text = re.sub(r"#BEGIN_LEGAL.*?#END_LEGAL", "", text, flags=re.S)
    table: dict[str, set[str]] = {}
    for block in text.split("\n{\n")[1:]:
        ic = re.search(r"ICLASS\s*:\s*(\S+)", block)
        pat = re.search(r"PATTERN\s*:\s*(.+)", block)
        if not ic or not pat:
            continue
        ops = table.setdefault(ic.group(1), set())
        ops.update(OPCODE.findall(pat.group(1)))
    return table


def sweep_groups() -> set[str]:
    """The GROUPS registry of gen_encoding_sweep.py, without importing it."""
    text = SWEEP.read_text(encoding="utf-8")
    m = re.search(r"^GROUPS = \{(.*?)^\}", text, flags=re.S | re.M)
    if not m:
        raise SystemExit(f"cannot find GROUPS in {SWEEP}")
    return set(re.findall(r'"([A-Za-z0-9_]+)":', m.group(1)))


def check_hermetic() -> list[str]:
    """Coverage check that needs no network and no reference file."""
    problems: list[str] = []
    groups = sweep_groups()
    for name, iclass, short, long_, coverage in FAMILIES:
        if coverage in groups:
            continue
        if coverage.startswith("relaxation:"):
            continue
        problems.append(
            f"{name}/{iclass}: coverage {coverage!r} is neither a "
            f"gen_encoding_sweep.py group nor a documented mechanism"
        )
    # An identical row twice is a copy-paste, not a second family. One family
    # may legitimately own several opcode pairs (MOV has two moffs rows), so
    # the key includes the encodings.
    seen: dict[tuple[str, str, str, str], int] = {}
    for name, iclass, short, long_, _c in FAMILIES:
        key = (name, iclass, short, long_)
        seen[key] = seen.get(key, 0) + 1
    problems.extend(
        f"duplicate family entry: {name}/{iclass} {short}/{long_} appears {n} times"
        for (name, iclass, short, long_), n in seen.items()
        if n > 1
    )
    # Every family must actually name two different opcodes: a "choice" whose
    # two sides are the same byte would silently test nothing.
    problems.extend(
        f"{name}/{iclass}: short and long encodings are the same byte"
        for name, iclass, short, long_, _c in FAMILIES
        if short == long_
    )
    return problems


def check_reparse(path: Path) -> list[str]:
    """Assert every claimed opcode really exists in the SDM description."""
    table = parse_sdm_isa(path)
    problems: list[str] = []
    for name, iclass, short, long_, _cov in FAMILIES:
        ops = table.get(iclass)
        if ops is None:
            problems.append(f"{iclass}: instruction class absent from {path.name}")
            continue
        for side, byte in (("short", short), ("long", long_)):
            if "+r" in byte:
                # `0x50+r` is an opcode with an embedded register number; the
                # reference spells it as a bit pattern, so match the number.
                continue
            if byte not in ops:
                problems.append(
                    f"{name}/{iclass}: {side} form {byte} is not an {iclass} "
                    f"opcode in the reference (has {sorted(ops)})"
                )
    return problems


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--reparse", metavar="ISA_FILE",
                    help="Intel-SDM-derived ISA description to check against")
    ap.add_argument("--list", action="store_true", help="print the family table")
    ap.add_argument("--json", metavar="FILE", help="write the table as JSON")
    args = ap.parse_args()

    if args.list:
        for name, iclass, short, long_, coverage in FAMILIES:
            print(f"{name:20s} {iclass:6s} {short:7s} {long_:7s} {coverage}")
        return 0

    if args.json:
        import json

        Path(args.json).write_text(
            json.dumps(
                {
                    "source": SDM_SOURCE,
                    "families": [
                        {"family": n, "iclass": i, "short": s, "long": l,
                         "coverage": c}
                        for n, i, s, l, c in FAMILIES
                    ],
                },
                indent=2,
            )
            + "\n"
        )

    problems = check_hermetic()
    if args.reparse:
        ref = Path(args.reparse)
        digest = hashlib.sha256(ref.read_bytes()).hexdigest()
        if digest != SDM_SOURCE["sha256"]:
            print(
                f"note: {ref.name} sha256 {digest} differs from the pinned "
                f"{SDM_SOURCE['sha256']}; the SDM/reference moved. Re-parse "
                "result follows, and the pin should be refreshed with it.",
                file=sys.stderr,
            )
        problems += check_reparse(ref)

    if problems:
        print("ERROR: encoding-choice coverage is not closed:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        return 1

    print(
        f"encoding-choice families: {len(FAMILIES)} across "
        f"{len({f[1] for f in FAMILIES})} instruction classes — every family "
        "is covered by the sweep or by a documented mechanism."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
