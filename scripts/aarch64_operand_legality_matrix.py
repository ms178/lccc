#!/usr/bin/env python3
"""The AArch64 operand-legality matrix: what GNU as accepts, and what it encodes.

WHY THIS EXISTS
---------------
The 60,000-word differential oracle in ``aarch64_encoder_differential.py``
generates random 32-bit words, has objdump decode them, and re-assembles the
resulting text.  Everything it feeds the assembler is therefore a *valid*
AArch64 encoding by construction.  It can measure "does LCCC encode a legal
instruction the same way GNU as does", but it is structurally blind to "does
LCCC reject what it should reject" -- an invalid operand combination such as
``mov x0, d1`` is not an encoding, so objdump can never emit it and the oracle
can never test it.

Those are different bug classes and they need different instruments.  This is
the instrument for the second one: a curated, executable matrix of operand
combinations -- valid *and* deliberately invalid -- whose expected verdicts are
taken from GNU as rather than from LCCC.  A matrix whose expectations were
derived from the implementation under test would agree with every bug it has.

WHY IT IS A FILE AND NOT A TEST
-------------------------------
The matrix is data, and it is regenerated from the oracle rather than
hand-maintained, so the honest way to store it is a checked-in table plus the
program that produced it, and it is asserted in two directions because one
direction alone proves nothing:

* ``--check`` re-runs every row against GNU as and fails if any expectation has
  drifted, which is what keeps the table from rotting when a new binutils
  changes its mind about a corner.
* ``--check-lccc`` runs the same rows against *our* assembler, which is the
  direction that catches a wrong encoder.  A table can be perfectly correct
  while the encoder disagrees with every row of it.

REGISTER 31
-----------
Most of the matrix exists because register 31 is not one register.  The
encoding of ``add x0, x1, x2`` reads field 31 as XZR, the encoding of
``add x0, x0, #1`` reads it as SP, and some encodings cannot read it at all.
``parse_reg_num`` collapses ``sp``, ``wsp``, ``xzr``, ``wzr`` and the FMOV
spellings onto the single number 31, so a validator that only checks "is this a
general-purpose register" cannot distinguish an instruction that means what the
programmer wrote from one that means something else.  Every row of the
``reg31`` group is a case where that distinction decides the verdict.

USAGE
-----
    scripts/aarch64_operand_legality_matrix.py --check      # verify vs GNU as
    scripts/aarch64_operand_legality_matrix.py --check-lccc target/fastbuild/lccc
    scripts/aarch64_operand_legality_matrix.py --print      # dump the matrix
    scripts/aarch64_operand_legality_matrix.py --regenerate # rewrite the table

``--check`` needs ``aarch64-linux-gnu-as`` and ``aarch64-linux-gnu-objcopy``;
``--check-lccc`` needs ``aarch64-linux-gnu-objcopy`` (to read our own object)
and invokes the compiler through a symlink named ``aarch64-linux-gnu-ccc``,
because LCCC selects its backend from ``argv[0]``.  The same table is also
compiled into the unit-test suite, so the guarantee holds on a host with no
cross-binutils at all.
It exits 0 when every row matches GNU as, 1 on any mismatch, and 2 when the
toolchain is missing (so a caller can distinguish "wrong" from "cannot tell").
The ``aarch64-operand-legality`` gate in ``ci_local.sh`` runs it in ``--check``
mode and is skipped, not passed, when the cross-binutils are absent.
"""
from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TABLE = REPO / "tests" / "aarch64" / "operand-legality.tsv"
PROLOGUE = ".arch armv9.4-a+sme\n.text\n"


def _labels(spec: str) -> list[str]:
    """Expand a matrix group spec into concrete instruction texts."""
    return [s for s in spec.strip().splitlines() if s.strip()]


# The matrix.  Grouped by the architectural question each row answers, so a
# failure names the rule that broke rather than just an instruction.
GROUPS: dict[str, list[str]] = {
    # --- register 31: which identity may each slot carry -------------------
    # Data-processing (1 source) reads 31 as ZR and forbids SP outright.
    "reg31 dp1src": [
        "clz x0,x30", "clz x0,lr", "clz x0,xzr", "clz x0,sp",
        "clz w0,w30", "clz w0,wzr", "clz w0,wsp",
        # width is part of the contract: xzr is not a 64-bit source for a w dest
        "clz x0,wzr", "clz w0,xzr",
        "cls x0,xzr", "cls x0,sp", "rbit x0,xzr", "rbit x0,sp",
        "rev x0,xzr", "rev x0,sp", "rev16 x0,xzr", "rev32 x0,xzr",
        "rev32 w0,w1", "rev32 x0,sp",
        # FP/SIMD in a GP slot
        "clz x0,d1", "rev x0,d1", "rbit x0,s1", "rev16 x0,v1",
    ],
    # MOV is two aliases: ORR (31 is ZR) and ADD (31 is SP).  A pair that
    # mixes the two identities is neither, and GNU as rejects it.
    "reg31 mov": [
        "mov x0,x1", "mov w0,w1", "mov x0,xzr", "mov xzr,x0", "mov xzr,xzr",
        "mov sp,x0", "mov x0,sp", "mov sp,sp", "mov wsp,w0", "mov w0,wsp",
        "mov sp,xzr", "mov xzr,sp", "mov wsp,wzr", "mov wzr,wsp",
        "mov x0,lr", "mov lr,x0", "mov w0,lr", "mov x30,w1",
        "mov d0,x1", "mov x0,d1", "mov d0,lr", "mov d0,sp", "mov v0,v1",
    ],
    # Logical (immediate): Rd is Xd|SP, Rn is Xn (31 = ZR), and the
    # flags-setting form takes no SP anywhere.
    "reg31 logical immediate": [
        "and x0,x1,#15", "and sp,x1,#15", "and xzr,x1,#15", "and x0,sp,#15",
        "and x0,xzr,#15", "orr sp,x1,#1", "orr xzr,x1,#1", "orr x0,xzr,#1",
        "eor sp,x1,#1", "ands x0,x1,#15", "ands sp,x1,#15", "ands x0,sp,#15",
        "ands x0,xzr,#15", "and w0,w1,#15", "and wsp,w1,#15",
        # the flags form has no SP destination, but its Rd=31 is the zero
        # register, which the non-flags form does not offer.
        "ands xzr,x1,#15", "orr wzr,x1,#1",
    ],
    # Logical (register): 31 is ZR, SP is forbidden in every slot.
    "reg31 logical register": [
        "and x0,x1,x2", "and x0,xzr,x1", "and x0,sp,x1", "and sp,x1,x2",
        "orr x0,xzr,x1", "orr x0,sp,x1", "orr sp,x1,x2",
        "eor x0,xzr,x1", "ands x0,xzr,x1", "ands x0,sp,x1",
    ],
    # --- operand width ----------------------------------------------------
    # The shifted-register add/sub and register logical forms require all
    # operands the same width; the *extended* add/sub form is the exception
    # and gets its own group below.
    "width add/sub": [
        "add x0,x1,x2", "add w0,w1,w2", "add x0,w1,w2", "add w0,x1,x2",
        "add x0,w1,x2", "add x0,x1,w2", "add w0,w1,x2",
        "sub x0,x1,x2", "sub w0,w1,w2", "sub x0,w1,x2", "subs x0,x1,x2",
        "subs w0,w1,w2", "adds x0,w1,x2",
        "add x0,sp,x1", "add sp,x0,x1", "add x0,xzr,x1",
    ],
    "width extended add/sub": [
        "add x0,x1,w2,uxtb", "add x0,x1,w2,uxth", "add x0,x1,w2,uxtw",
        "add x0,x1,x2,uxtx", "add x0,x1,w2,sxtb", "add x0,x1,w2,sxtw",
        "add w0,w1,w2,uxtb",
    ],
    "width logical": [
        "and x0,x1,x2", "and w0,w1,w2", "and x0,w1,x2", "and x0,x1,w2",
        "and w0,w1,x2", "orr x0,w1,x2", "eor x0,x1,w2", "bic x0,w1,x2",
    ],
    # --- the GP-only logical siblings -------------------------------------
    "gp only logical siblings": [
        "orn x0,x1,x2", "orn w0,w1,w2", "orn x0,w1,x2", "orn x0,xzr,x1",
        "orn x0,sp,x1", "eon x0,x1,x2", "eon x0,w1,x2", "eon x0,d1,x2",
        "bic x0,x1,x2", "bic w0,w1,w2", "bic x0,w1,x2", "bic x0,xzr,x1",
        "bic x0,sp,x1", "bics x0,x1,x2", "bics x0,w1,x2", "bics x0,s1,x2",
        "orn x0,x1,d2", "eon x0,x1,h2", "bic x0,x1,v2", "bics x0,x1,d2",
    ],
    # --- MOV immediates ---------------------------------------------------
    "mov immediate": [
        "mov x0,#1", "mov w0,#1", "mov sp,#1", "mov xzr,#15", "mov wsp,#1",
        "mov d0,#1", "mov d0,#0", "mov s0,#1", "mov q0,#1", "mov v0,#1",
        "mov x0,#0x100000000", "mov x0,#-1",
        # `mov #imm` is an alias, so it takes no explicit shift: the shifted
        # spelling belongs to movz/movk/movn.
        "mov x0,#1,lsl #16", "mov w0,#1,lsl #16", "mov x0,#7,lsl #17",
    ],
    # --- the extend aliases -----------------------------------------------
    # `SXTB <Xd>, <Wn>`: the destination is Xd|Wd and the source is always W.
    "extend aliases": [
        "sxtb x0,w1", "sxtb w0,w1", "sxth x0,w1", "sxth w0,w1",
        "uxtb x0,w1", "uxtb w0,w1", "uxth x0,w1", "uxth w0,w1",
        "sxtb x0,x1", "sxth x0,x1", "uxtb x0,x1", "uxth x0,x1",
        "sxtb w0,x1", "sxtb x0,sp", "sxtb x0,wsp", "sxtb x0,lr",
        "sxtb x0,xzr", "sxtb x0,wzr", "sxtb x0,d1", "sxtb d0,w1",
        "sxtw x0,w1", "sxtw w0,w1", "sxtw x0,x1", "sxtw w0,x1",
        "sxtw x0,sp", "sxtw x0,d1",
        "uxtw x0,w1", "uxtw w0,w1", "uxtw x0,x1", "uxtw x0,sp", "uxtw x0,d1",
    ],
    # --- FMOV between general-purpose and FP registers ---------------------
    # 31 is XZR here, spelled `xzr`/`wzr`; `x31`, SP and mismatched widths
    # are all rejected.
    "fmov gp": [
        "fmov d0,x0", "fmov d0,x30", "fmov d0,xzr", "fmov xzr,d0",
        "fmov d0,x31", "fmov d0,w31", "fmov d0,sp", "fmov d0,wsp",
        "fmov d0,lr", "fmov lr,d0", "fmov d30,lr", "fmov h0,lr",
        "fmov x30,d0", "fmov s0,lr", "fmov d0,x32", "fmov d0,x007",
        "fmov h0,w1", "fmov h0,x1", "fmov s0,w1", "fmov d0,x1",
        "fmov s0,x1", "fmov d0,w1", "fmov q0,x1",
        "fmov v0,v1", "fmov s0,d1", "fmov d0,s1",
        # H pairs with either GP width, which is what makes wsp reachable in
        # a GP slot at all -- and 31 there silently means WZR.
        "fmov h0,wsp", "fmov h0,sp", "fmov h0,wzr", "fmov h0,xzr",
        "fmov wsp,h0", "fmov xzr,h0",
    ],
    # --- FP/SIMD registers reaching GP-only slots --------------------------
    "gp only arithmetic": [
        "add x0,x1,d2", "add x0,x1,h2", "add x0,x1,v2", "add x0,x1,s2",
        "sub x0,x1,s2", "and x0,x1,d2", "orr x0,s1,x2", "eor x0,x1,h2",
    ],
    # --- add/sub, the two register forms -----------------------------------
    # Which of the two register forms an instruction selects is decided by
    # SP in Rd/Rn (the shifted form reads field 31 as ZR and has no encoding
    # for SP, the extended form reads it as SP), and by an explicit <extend>.
    # They disagree about register 31 in *every* slot, so the same `add x0,
    # sp, x1` text is a shifted ADD that cannot be encoded and an extended ADD
    # that can.
    "add/sub extended form reg31": [
        "add x0,sp,x1", "add sp,x0,x1", "add sp,sp,x1", "add x0,x1,sp",
        "add sp,xzr,x1", "add xzr,sp,x1", "add xzr,xzr,xzr", "add xzr,x1,x2",
        "add x0,x1,xzr", "add x0,sp,xzr", "add x0,sp,wzr",
        "adds x0,sp,x1", "adds sp,x1,x2", "adds sp,sp,x1", "subs sp,x1,x2",
        "adds xzr,x1,x2", "subs x0,xzr,x1", "adds x0,x1,sp", "sub sp,sp,x1",
    ],
    # The extended form's `sf` bit covers Rd and Rn, but Rm is chosen by its
    # own spelling: a 64-bit form accepts a 32-bit source (that is the whole
    # point of the form), a 32-bit form does not accept a 64-bit one.
    "add/sub extended form widths": [
        "add x0,sp,w1", "add x0,wsp,w1", "add w0,wsp,w1", "add sp,x1,w2",
        "add w0,wsp,xzr", "add w0,wsp,wzr", "add w0,wsp,x1", "add x0,wsp,x1",
        "add w0,w1,x2", "add w0,w1,w2", "add wsp,w1,w2",
    ],
    # An explicit extend operand picks the form and sets the option field;
    # GNU as does not cross-check the option against Rm's spelling, but the
    # 32/64 split between Rd/Rn and Rm still holds, and imm3 is 0-4.
    "add/sub explicit extend": [
        "add x0,x1,x2,uxtw", "add x0,x1,x2,uxth", "add x0,x1,x2,sxtb",
        "add x0,x1,x2,uxtx", "add x0,x1,xzr,uxtx", "add x0,x1,w2,uxtw",
        "add x0,x1,w2,uxtx", "add x1,x2,x3,uxth #2", "add x0,x1,w2,uxth #4",
        "add x0,x1,x2,uxtx #5", "add w0,w1,w2,uxtx", "add w0,w1,x2,uxtw",
        "add xzr,x1,x2,uxtx", "add sp,x1,x2,uxtx", "adds sp,x1,x2,uxtx",
        "add x0,sp,x1,uxtw", "add x0,sp,w2,uxtw", "add x0,sp,x2,uxtw #3",
    ],
    # --- shifts ------------------------------------------------------------
    # The shift field is 2 bits of opcode and 6 bits of amount, and neither is
    # checked by masking. `ror` occupies 11 there, which is reserved in the
    # add/sub shifted forms (it is `ror` in the logical ones), and the amount
    # only has room for the element width.
    "shift legality add/sub": [
        "add x0,x1,x2", "add x0,x1,x2,lsl #2", "add x0,x1,x2,lsr #5",
        "add x0,x1,x2,asr #5", "add x0,x1,x2,lsl #63", "add x0,x1,x2,lsl #64",
        "add x0,x1,x2,ror #3", "add x0,x1,x2,lsl #0", "sub x0,x1,x2,lsl #32",
        "add w0,w1,w2,lsl #31", "add w0,w1,w2,lsl #32",
        "neg x0,x1,lsl #3", "neg x0,x1,lsr #3", "neg x0,x1,asr #63",
        "neg x0,x1,ror #3", "neg x0,x1,lsl #64", "negs x0,x1,lsl #3",
        "add x0,sp,x1,lsl #2", "add x0,sp,w1,lsl #2", "add x0,sp,x1,lsl #5",
        "add x0,sp,x1,lsr #2", "add x0,sp,x1,lsl #0", "add w0,wsp,w1,lsl #2",
    ],
    "shift legality logical": [
        "and x0,x1,x2,ror #3", "orr x0,x1,x2,ror #3", "eor x0,x1,x2,ror #3",
        "mvn x0,x1,ror #3", "mvn x0,x1,lsl #32", "mvn x0,x1,lsl #64",
        "mvn w0,w1,lsl #31", "and x0,x1,x2,lsl #63", "and w0,w1,w2,lsl #31",
        "and w0,w1,w2,lsl #32", "bic x0,x1,x2,ror #3", "bic x0,x1,x2,asr #7",
        "bic x0,x1,x2,lsl #63", "bic x0,x1,x2,lsl #64", "bic w0,w1,w2,lsl #31",
        "bic w0,w1,w2,lsl #32", "bics x0,x1,x2,ror #3", "orn x0,x1,x2,ror #3",
        "eon x0,x1,x2,ror #3",
    ],
    # --- mov-wide-immediate halfword selector ------------------------------
    # `lsl #N` on MOVZ/MOVK/MOVN names the 16-bit halfword the immediate
    # lands in, so only multiples of 16 that exist in the width are legal --
    # `lsl #15` is not a shift of the immediate, it is out of range.
    "movw halfword selector": [
        "movz x0,#1", "movz x0,#1,lsl #0", "movz x0,#1,lsl #16",
        "movz x0,#1,lsl #32", "movz x0,#1,lsl #48", "movz x0,#1,lsl #15",
        "movz x0,#1,lsl #17", "movz x0,#1,lsl #64", "movz x0,#1,lsr #16",
        "movz w0,#1,lsl #16", "movz w0,#1,lsl #32",
        "movk x0,#1,lsl #32", "movk w0,#1,lsl #16", "movn x0,#1,lsl #16",
    ],
    # --- BIC, both forms ---------------------------------------------------
    # BIC (immediate) is AND with an inverted bitmask: Rd is <Xd|SP> and Rn is
    # <Xn>. BIC (register) is AND (shifted): register 31 is ZR everywhere.
    "bic forms": [
        "bic x0,x1,#15", "bic w0,w1,#15", "bic sp,x1,#15", "bic sp,x1,#16",
        "bic x0,xzr,#15", "bic x0,x1,#0xffff", "bic x0,sp,#15",
        "bic wzr,w1,#15", "bic x0,w1,#15", "bic x0,x1,d2",
        "bic x0,x1,x2", "bic w0,w1,w2", "bic x0,x1,xzr", "bic x0,x1,w2",
        "bic x0,w1,x2", "bic x0,sp,x1", "bic x0,x1,v2",
    ],
    # --- add/sub immediate, written shift ----------------------------------
    # `lsl #12` is not a shift of the immediate, it is the `sh` bit: the
    # immediate has to fit in 12 bits on its own, and only #0/#12 exist.
    "add/sub immediate shift": [
        "add x0,x1,#1,lsl #12", "add x0,x1,#0,lsl #12", "add x0,x1,#0xfff,lsl #12",
        "add x0,x1,#0x1000,lsl #12", "add w0,w1,#0x1000,lsl #12",
        "add x0,x1,#1,lsl #11", "add x0,x1,#1,lsl #13", "add x0,x1,#0x1000",
        "add x0,x1,#0xfff000", "add x0,x1,#0xfffffff", "add x0,x1,#-1",
        "sub x0,x1,#-1", "add sp,sp,#0x1000",
    ],
    # --- mov-wide-immediate preference -------------------------------------
    # Which of MOVZ / MOVN / ORR-bitmask / movz+movk GNU as chooses is
    # observable in the bytes, so each of these pins a branch of the choice.
    "mov immediate wide forms": [
        "mov x0,#0", "mov x0,#-2", "mov x0,#0xffff", "mov x0,#0x10000",
        "mov x0,#0xffff0000", "mov x0,#0xffffffff", "mov x0,#-65536",
        "mov x0,#0x1fe0", "mov x0,#0x30000000", "mov x0,#0x12340000",
        "mov x0,#0x80000000", "mov x0,#0xffffffff00000000",
        "mov x0,#0x5555555555555555", "mov x0,#0x123456789abcdef0",
        "mov w0,#0", "mov w0,#-1", "mov w0,#0xffff", "mov wsp,#0x12345",
        "mov sp,#0x1234", "mov sp,#-1", "mov xzr,#0x100000000",
    ],
    # --- cmp/cmn/tst: the pinned-destination aliases ------------------------
    # CMP/CMN are SUBS/ADDS with the destination pinned to 11111, and TST is
    # ANDS with the same pinned destination. The pinned slot belongs to the
    # alias, not to the instruction the user wrote, so GNU as does not apply
    # the SUBS/ADDS destination rules to it: `cmp sp, x0` and `cmp sp, #1`
    # assemble where `subs sp, x1, x2` and `adds sp, x1, #1` do not. TST is
    # the opposite case -- ANDS has no SP encoding anywhere, so every SP
    # operand of `tst` is rejected.
    "cmp/cmn/tst aliases": [
        "cmp x0,x1", "cmp w0,w1", "cmp x0,x30", "cmp x0,x1,lsl #2",
        "cmp x0,x1,ror #3", "cmp x0,#1", "cmp x0,#0xfff", "cmp x0,#-1",
        "cmp x0,#0x1000", "cmp x0,#0x1000,lsl #12", "cmp w0,#1",
        "cmp x0,sp", "cmp x0,w1", "cmp x0,xzr", "cmp xzr,x0",
        "cmp sp,x0", "cmp wsp,w0", "cmp sp,w0", "cmp sp,sp", "cmp sp,xzr",
        "cmp sp,#1", "cmp wsp,x0", "cmp sp,x0,lsl #2", "cmp sp,x0,uxtw",
        "cmn x0,#1", "cmn x0,x1", "cmn w0,#0xfff", "cmn x0,sp",
        "cmn sp,x0", "cmn sp,#1",
        "tst x0,x1", "tst w0,w1", "tst x0,#15", "tst x0,sp", "tst sp,x0",
        "tst sp,#15", "tst x0,d1", "tst x0,x1,ror #3", "tst wzr,x0",
        "tst xzr,x1",
    ],
    # --- positive controls -------------------------------------------------
    # If these ever fail, the matrix has stopped measuring anything useful.
    "positive controls": [
        "add x0,x1,x2", "add sp,sp,#16", "mov x0,x1", "mov x0,sp", "mov sp,x1",
        "and x0,x1,#15", "and sp,x1,#15", "clz x0,x1", "rev x0,x1",
        "sxtb x0,w1", "sxtb w0,w1", "sxtw x0,w1", "fmov d0,x0", "fmov x0,d0",
        "fmov d0,x30", "fmov lr,d0", "orr x0,x1,x2", "orr x0,x1,#1",
    ],
}


def _expand() -> list[tuple[str, str]]:
    """(group, instruction) for every row of the matrix."""
    rows: list[tuple[str, str]] = []
    for group, spec in GROUPS.items():
        for insn in _labels("\n".join(spec)):
            rows.append((group, insn))
    return rows


def assemble(
    insn: str,
    command: list[str],
    objcopy: str,
    tmp: Path,
    prologue: str = PROLOGUE,
) -> str:
    """One assembler's verdict for one instruction.

    Returns ``OK <hexwords>`` -- the little-endian bytes of ``.text``, so a
    one-instruction file yields the encoding itself -- or ``REJECT`` when the
    assembler exits non-zero.  ``command`` is the assembler invocation with the
    source and ``-o`` arguments appended by the caller's convention.
    """
    src = tmp / "m.s"
    obj = tmp / "m.o"
    binf = tmp / "m.bin"
    src.write_text(prologue + insn + "\n")
    for f in (obj, binf):
        if f.exists():
            f.unlink()
    r = subprocess.run(
        [*command, str(src), "-o", str(obj)], capture_output=True, text=True
    )
    if r.returncode != 0:
        return "REJECT"
    subprocess.run(
        [objcopy, "-O", "binary", "--only-section=.text", str(obj), str(binf)],
        check=True,
        capture_output=True,
    )
    return "OK " + binf.read_bytes().hex()


def encode(insn: str, as_bin: str, objcopy: str, tmp: Path) -> str:
    """GNU as's verdict for one instruction: ``OK <hexwords>`` or ``REJECT``."""
    return assemble(insn, [as_bin], objcopy, tmp)


def parse_table(path: Path) -> list[tuple[str, str, str]]:
    """Rows of ``group<TAB>instruction<TAB>expectation``."""
    out: list[tuple[str, str, str]] = []
    for line in path.read_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 3:
            raise SystemExit(f"malformed row in {path}: {line!r}")
        out.append((parts[0], parts[1], parts[2]))
    return out


def write_table(rows: list[tuple[str, str, str]], path: Path) -> None:
    header = (
        "# AArch64 operand-legality matrix -- expectations come from GNU as.\n"
        "# Generated by scripts/aarch64_operand_legality_matrix.py; do not\n"
        "# hand-edit.  Every row is re-checked against GNU as by --check, which\n"
        "# is the aarch64-operand-legality gate in ci_local.sh.\n"
        "#\n"
        "# group\tinstruction\texpectation  (OK <hex> | REJECT)\n"
    )
    body = "".join(f"{g}\t{i}\t{e}\n" for g, i, e in rows)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(header + body)


def find_tools(explicit_as: str | None, explicit_objcopy: str | None):
    as_bin = explicit_as or shutil.which("aarch64-linux-gnu-as")
    objcopy = explicit_objcopy or shutil.which("aarch64-linux-gnu-objcopy")
    return as_bin, objcopy


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true", help="verify the table vs GNU as")
    ap.add_argument(
        "--check-lccc",
        dest="lccc",
        metavar="LCCC",
        default=None,
        help="verify the table against the lccc AArch64 assembler (differential)",
    )
    ap.add_argument("--print", dest="dump", action="store_true", help="print the table")
    ap.add_argument("--regenerate", action="store_true", help="rewrite the table")
    ap.add_argument("--as", dest="as_bin", default=None)
    ap.add_argument("--objcopy", default=None)
    args = ap.parse_args()

    if args.dump:
        for g, i, e in parse_table(TABLE):
            print(f"{g:34s} {i:34s} {e}")
        return 0

    as_bin, objcopy = find_tools(args.as_bin, args.objcopy)
    if not as_bin or not objcopy:
        print(
            "operand-legality matrix: cross-binutils not found; cannot verify",
            file=sys.stderr,
        )
        return 2

    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        if args.regenerate:
            rows = [
                (g, i, encode(i, as_bin, objcopy, tmp)) for g, i in _expand()
            ]
            write_table(rows, TABLE)
            n_rej = sum(1 for _, _, e in rows if e == "REJECT")
            print(
                f"operand-legality matrix: wrote {len(rows)} rows "
                f"({len(rows) - n_rej} accepted, {n_rej} rejected) to {TABLE}"
            )
            return 0

        if not (args.check or args.lccc):
            ap.error("choose --check, --check-lccc, --print or --regenerate")

        rows = parse_table(TABLE)

        if args.check:
            drift = []
            for group, insn, expected in rows:
                got = encode(insn, as_bin, objcopy, tmp)
                if got != expected:
                    drift.append((group, insn, expected, got))

            if drift:
                print(
                    f"operand-legality matrix: {len(drift)} row(s) drifted from GNU as",
                    file=sys.stderr,
                )
                for group, insn, expected, got in drift[:40]:
                    print(
                        f"  [{group}] {insn}: table={expected} gas={got}",
                        file=sys.stderr,
                    )
                return 1
            print(
                f"operand-legality matrix: {len(rows)} rows agree with GNU as "
                f"({as_bin})"
            )

        if args.lccc:
            # The differential half.  `--check` proves the table still says
            # what GNU as says; this proves *we* still do too.  Without it the
            # table is documentation: a row can be right and the encoder wrong,
            # which is exactly the shape of every bug this matrix was written
            # for (`clz x0, sp` assembled, `fmov h0, wsp` encoded WZR, `lsl
            # x0, x1, #64` emitted 0xfffffc20).  LCCC picks its backend from
            # argv[0], so it is invoked through a symlink named
            # aarch64-linux-gnu-ccc; it also has no `.arch` directive, so the
            # GAS prologue is dropped on this side.
            lccc = Path(args.lccc)
            if not lccc.exists():
                print(f"operand-legality matrix: no such lccc: {lccc}", file=sys.stderr)
                return 2
            link = tmp / "aarch64-linux-gnu-ccc"
            link.symlink_to(lccc.resolve())
            bad = []
            for group, insn, expected in rows:
                got = assemble(insn, [str(link), "-c"], objcopy, tmp, prologue=".text\n")
                if got != expected:
                    bad.append((group, insn, expected, got))

            if bad:
                print(
                    f"operand-legality matrix: {len(bad)} row(s) disagree with "
                    f"lccc ({lccc})",
                    file=sys.stderr,
                )
                for group, insn, expected, got in bad[:40]:
                    print(
                        f"  [{group}] {insn}: gas={expected} lccc={got}",
                        file=sys.stderr,
                    )
                return 1
            print(
                f"operand-legality matrix: {len(rows)} rows agree with lccc "
                f"({lccc})"
            )
        return 0


if __name__ == "__main__":
    sys.exit(main())
