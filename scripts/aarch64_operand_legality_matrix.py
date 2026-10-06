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
import re
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
    # --- PSTATE fields: which are readable, writable, and with what --------
    # The PSTATE fields split three ways rather than two: some have a read form
    # (MRS), some only a write form (MSR immediate), and the immediate write
    # has its own range.  `mrs x0,pan` is legal and `msr pan,x0` is not, which
    # is why the read table and the write table are not the same table.
    "system pstate": [
        "mrs x0,pan", "mrs x0,ssbs", "mrs x0,dit", "mrs x0,tco",
        "mrs x0,nzcv", "mrs x0,daif", "mrs x0,spsel", "mrs x0,currentel",
        "mrs x0,daifset", "mrs x0,daifclr", "mrs x0,pan_x", "mrs w0,pan",
        "mrs sp,pan", "mrs d0,pan",
        "msr pan,#1", "msr pan,#0", "msr pan,#2", "msr pan,x0", "msr pan,w0",
        "msr pan,xzr", "msr pan,sp", "msr ssbs,x0", "msr dit,x0", "msr tco,x0",
        "msr spsel,x0", "msr currentel,x0", "msr daifset,x0", "msr fpsr,x0",
        "msr ssbs,#1", "msr ssbs,#2", "msr dit,#1", "msr dit,#2",
        "msr tco,#1", "msr tco,#2",
        "msr spsel,#0", "msr spsel,#1", "msr spsel,#2",
        "msr daifset,#0", "msr daifset,#15", "msr daifset,#16",
        "msr daifclr,#0", "msr daifclr,#15", "msr daifclr,#16",
        "msr nzcv,#15", "msr nzcv,#16", "msr nzcv,x0",
        "msr daif,#0", "msr daif,x0",
        # UAO and ALLINT are one-bit fields like PAN, and are not covered by
        # the groups above.
        "mrs x0,uao", "mrs x0,allint",
        "msr uao,#0", "msr uao,#1", "msr uao,#2", "msr uao,x0", "msr uao,w0",
        "msr allint,#0", "msr allint,#1", "msr allint,#2", "msr allint,x0",
        # The SME SVCR fields are immediate-only: one bit, encoded in CRm's low
        # bit (msr svcrsm,#1 is CRm=3), and there is no register spelling at
        # all -- GNU as rejects `mrs x0,svcrsm`.
        "msr svcrsm,#0", "msr svcrsm,#1", "msr svcrsm,#2", "msr svcrsm,x0",
        "msr svcrza,#0", "msr svcrza,#1", "msr svcrza,#2", "msr svcrza,x0",
        "msr svcrsmza,#0", "msr svcrsmza,#1", "msr svcrsmza,#2", "msr svcrsmza,x0",
        "mrs x0,svcrsm", "mrs x0,svcrza", "mrs x0,svcrsmza",
    ],
    # --- BIC, both forms ---------------------------------------------------
    # BIC (immediate) is AND with an inverted bitmask: Rd is <Xd|SP> and Rn is
    # <Xn>. BIC (register) is AND (shifted): register 31 is ZR everywhere.
    # The byte/halfword load-store family, where the MNEMONIC decides the
    # register class: `ldr/str` with a b/h register is the FP byte/halfword
    # access (GNU as: str b9,[x10] = 3d000149), while `ldrb/strb/ldrh/strh` are
    # the general-purpose forms and take a w register only.  An encoder that
    # keys the check on the operand class instead of the mnemonic rejects the
    # first and accepts the second, and both directions have been live bugs.
    # A bare `v` register is not an operand at all: the 128-bit spelling is
    # `q`, and `str v9,[x10]` is an error in GNU as.
    "ldr/str byte forms": [
        "str b9,[x10]", "str h9,[x10]", "str s9,[x10]", "str d9,[x10]",
        "str q9,[x10]", "str v9,[x10]", "str v0,[x0,#16]",
        "ldr b9,[x10]", "ldr h9,[x10]", "ldr v9,[x10]", "ldr q9,[x10]",
        "str b0,[x1],#1", "ldr b0,[x0],#1", "str h0,[x1],#2", "ldr h0,[x0],#2",
        "str b0,[x1,#1]!", "ldr b0,[x0,#1]!", "str b0,[x1,#1]", "ldr b0,[x0,#1]",
        "strb w9,[x10]", "strb x9,[x10]", "strb b9,[x10]", "strb h9,[x10]",
        "strb s9,[x10]", "strb d9,[x10]", "strb q9,[x10]", "strb wzr,[x10]",
        "strb sp,[x10]", "strb w9,[x10],#1", "strb w9,[x10,#1]!",
        "strh w9,[x10]", "strh x9,[x10]", "strh b9,[x10]", "strh h9,[x10]",
        "strh s9,[x10]", "strh d9,[x10]", "strh w9,[x10],#2",
        "ldrb w9,[x10]", "ldrb x9,[x10]", "ldrb b9,[x10]", "ldrb h9,[x10]",
        "ldrb s9,[x10]", "ldrb d9,[x10]", "ldrb q9,[x10]", "ldrb w9,[x10],#1",
        "ldrh w9,[x10]", "ldrh x9,[x10]", "ldrh h9,[x10]", "ldrh b9,[x10]",
        "ldrh s9,[x10]", "ldrh d9,[x10]",
        # The sign-extending trio: ldrsb/ldrsh widen into an x register (and a w
        # destination is the non-widening of the same instruction), ldrsw is
        # x-only, and an FP register is never one of them.
        "ldrsb x9,[x10]", "ldrsb w9,[x10]", "ldrsb b9,[x10]", "ldrsb s9,[x10]",
        "ldrsh x9,[x10]", "ldrsh w9,[x10]", "ldrsh h9,[x10]",
        "ldrsw x9,[x10]", "ldrsw w9,[x10]", "ldrsw s9,[x10]",
    ],
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
    # === Session-19 extension: families the first matrix revision did not ===
    # === reach (NEON, load/store, system, dp immediates).  Same contract:  ===
    # === every expectation below is GNU as's verdict, never ours.         ===
    #
    # --- NEON MOVI/MVNI: the shift kind and the arrangement decide cmode ---
    # MOVI/MVNI's immediate is an 8-bit field whose *per-lane* meaning is
    # chosen by cmode, and the shift operand selects cmode.  The 16-bit-lane
    # forms use cmode=10xx (lsl) or cmode=110x (msl) -- a `msl` that is
    # dropped leaves cmode at the unshifted encoding, so the instruction
    # assembles with a different immediate than the programmer wrote.
    "neon movi imm8": [
        "movi v0.16b,#0", "movi v0.16b,#255", "movi v0.16b,#256",
        "movi v0.16b,#-1", "movi v0.16b,#-128", "movi v0.16b,#-129",
        "movi v0.8b,#1", "movi v0.4h,#1", "movi v0.8h,#1",
        "movi v0.2s,#1", "movi v0.4s,#1", "movi v0.1d,#1", "movi v0.2d,#0",
        "movi v0.2d,#0xffffffffffffffff", "movi v0.2d,#0x00ff00ff00ff00ff",
        "movi v0.2d,#1", "movi v0.2d,#0x0101010101010101",
    ],
    "neon movi shift": [
        "movi v0.4h,#1,lsl #0", "movi v0.4h,#1,lsl #8", "movi v0.4h,#1,lsl #16",
        "movi v0.8h,#1,lsl #8", "movi v0.2s,#1,lsl #0", "movi v0.2s,#1,lsl #8",
        "movi v0.2s,#1,lsl #16", "movi v0.2s,#1,lsl #24", "movi v0.4s,#1,lsl #8",
        "movi v0.4s,#1,lsl #16", "movi v0.4s,#1,lsl #24", "movi v0.4s,#1,lsl #32",
        "movi v0.4h,#1,msl #8", "movi v0.4h,#1,msl #16", "movi v0.4h,#1,msl #0",
        "movi v0.8h,#1,msl #8", "movi v0.2s,#1,msl #8", "movi v0.2s,#1,msl #16",
        "movi v0.4s,#1,msl #8", "movi v0.4s,#1,msl #16", "movi v0.4s,#1,msl #24",
        "movi v0.8b,#1,lsl #8", "movi v0.16b,#1,lsl #8", "movi v0.2d,#0,msl #8",
    ],
    "neon mvni": [
        "mvni v0.4h,#1", "mvni v0.8h,#1", "mvni v0.2s,#1", "mvni v0.4s,#1",
        "mvni v0.4h,#1,lsl #8", "mvni v0.4h,#1,lsl #16", "mvni v0.8h,#1,lsl #8",
        "mvni v0.2s,#1,lsl #8", "mvni v0.2s,#1,lsl #24", "mvni v0.4s,#1,lsl #8",
        "mvni v0.4h,#1,msl #8", "mvni v0.4h,#1,msl #16", "mvni v0.8h,#1,msl #8",
        "mvni v0.2s,#1,msl #8", "mvni v0.4s,#1,msl #16",
        "mvni v0.16b,#1", "mvni v0.2d,#1", "mvni v0.1d,#1",
    ],
    # --- NEON EXT: index bound depends on the arrangement ------------------
    # EXT's imm4 is an index into the concatenation of the two sources and is
    # only defined for the byte arrangements; 16-byte vectors index 0..15,
    # 8-byte ones 0..7.  A wrapped index silently extracts a different slice.
    "neon ext": [
        "ext v0.16b,v1.16b,v2.16b,#0", "ext v0.16b,v1.16b,v2.16b,#15",
        "ext v0.16b,v1.16b,v2.16b,#16", "ext v0.16b,v1.16b,v2.16b,#31",
        "ext v0.8b,v1.8b,v2.8b,#0", "ext v0.8b,v1.8b,v2.8b,#7",
        "ext v0.8b,v1.8b,v2.8b,#8", "ext v0.8b,v1.8b,v2.8b,#15",
        "ext v0.4h,v1.4h,v2.4h,#1", "ext v0.2s,v1.2s,v2.2s,#1",
        "ext v0.2d,v1.2d,v2.2d,#1", "ext v0.16b,v1.16b,v2.16b,#-1",
        "ext v0.16b,v1.8b,v2.16b,#1",
    ],
    # --- NEON across-lane reductions --------------------------------------
    "neon addv maxv minv": [
        "addv b0,v1.8b", "addv b0,v1.16b", "addv h0,v1.4h", "addv h0,v1.8h",
        "addv s0,v1.4s", "addv d0,v1.2d", "addv s0,v1.2s", "addv b0,v1.4h",
        "addv d0,v1.1d", "saddlv h0,v1.8b", "saddlv s0,v1.4h",
        "uaddlv h0,v1.8b", "smaxv b0,v1.8b", "smaxv b0,v1.16b",
        "sminv s0,v1.4s", "smaxv d0,v1.2d", "addp d0,v1.2d", "addp s0,v1.2s",
        "fmaxv s0,v1.4s", "fmaxv d0,v1.2d", "fminv s0,v1.2s", "fmaxnmv s0,v1.4s",
    ],
    # --- NEON table lookup: 1..4 registers, brace arrangement, empty list --
    "neon tbl tbx": [
        "tbl v0.16b,{v1.16b},v2.16b", "tbl v0.8b,{v1.8b},v2.8b",
        "tbl v0.16b,{v1.16b,v2.16b},v3.16b",
        "tbl v0.16b,{v1.16b,v2.16b,v3.16b},v4.16b",
        "tbl v0.16b,{v1.16b,v2.16b,v3.16b,v4.16b},v5.16b",
        "tbl v0.16b,{},v2.16b", "tbl v0.16b,{v1.16b,v2.16b,v3.16b,v4.16b,v5.16b},v6.16b",
        "tbl v0.16b,{v1.16b,v3.16b},v2.16b", "tbl v0.8b,{v1.16b},v2.8b",
        "tbx v0.16b,{v1.16b},v2.16b", "tbx v0.16b,{v1.16b,v2.16b},v3.16b",
        "tbx v0.16b,{},v2.16b", "tbx v0.8b,{v1.8b,v2.8b},v3.8b",
    ],
    # --- NEON load-and-replicate: R/S bits encode the structure count ------
    "neon ld1r ld2r ld4r": [
        "ld1r {v0.16b},[x0]", "ld1r {v0.8b},[x0]", "ld1r {v0.4h},[x0]",
        "ld1r {v0.2d},[x0]", "ld1r {v0.16b},[x0],#1", "ld1r {v0.16b},[x0],#16",
        "ld1r {v0.16b},[x0],x1", "ld1r {v0.16b,v1.16b},[x0]",
        "ld2r {v0.16b,v1.16b},[x0]", "ld2r {v0.8b,v1.8b},[x0]",
        "ld2r {v0.16b},[x0]", "ld2r {v0.4h,v1.4h},[x0]",
        "ld2r {v0.16b,v1.16b},[x0],#2", "ld2r {v0.16b,v1.16b},[x0],x1",
        "ld3r {v0.16b,v1.16b,v2.16b},[x0]",
        "ld3r {v0.16b,v1.16b},[x0]", "ld3r {v0.4s,v1.4s,v2.4s},[x0]",
        "ld4r {v0.16b,v1.16b,v2.16b,v3.16b},[x0]",
        "ld4r {v0.4s,v1.4s,v2.4s,v3.4s},[x0]",
        "ld4r {v0.16b,v1.16b,v2.16b},[x0]",
        "ld4r {v0.16b,v1.16b,v2.16b,v3.16b},[x0],#4",
    ],
    # --- NEON scalar by-element: the lane bound is per element size --------
    "neon by-element": [
        "fmul s0,s1,v2.s[0]", "fmul s0,s1,v2.s[3]", "fmul s0,s1,v2.s[4]",
        "fmul d0,d1,v2.d[1]", "fmul d0,d1,v2.d[0]", "fmul d0,d1,v2.d[2]",
        "fmul h0,h1,v2.h[7]", "fmul h0,h1,v2.h[8]", "fmul h0,h1,v2.h[0]",
        "fmulx s0,s1,v2.s[0]", "fmulx s0,s1,v2.s[3]", "fmulx s0,s1,v2.s[4]",
        "fmulx d0,d1,v2.d[1]", "fmulx d0,d1,v2.d[2]", "fmulx h0,h1,v2.h[7]",
        "fmla s0,s1,v2.s[0]", "fmla d0,d1,v2.d[1]", "fmls h0,h1,v2.h[7]",
        "fmla v0.4s,v1.4s,v2.s[0]", "fmla v0.2d,v1.2d,v2.d[1]",
        "fmla v0.4s,v1.4s,v2.d[1]",
    ],
    # --- load/store: which register class may be the data register --------
    # Rt's field-31 alias is XZR in every one of these encodings, so `sp` is
    # not an operand -- it is a different instruction that stores zero.
    "ldst Rt class": [
        "str sp,[x0]", "str wsp,[x0]", "ldr sp,[x0]", "ldr wsp,[x0]",
        "str xzr,[x0]", "str wzr,[x0]", "str x0,[sp]", "str x0,[x0]",
        "str q0,[x0]", "str d0,[x0]", "ldr q0,[x0]", "ldr d0,[x0]",
        "ldur sp,[x0,#8]", "stur wsp,[x0,#8]", "ldur d0,[x0,#8]",
        "strb sp,[x0]", "strh wsp,[x0]", "ldrb sp,[x0]",
        "ldrsw x0,[x1]", "ldrsw w0,[x1]", "ldrsw d0,[x1]",
        "ldrsb x0,[x1]", "ldrsb w0,[x1]", "ldrsb q0,[x1]", "ldrsb d0,[x1]",
        "ldrsh x0,[x1]", "ldrh w0,[x1]", "ldrh x0,[x1]", "ldrh q0,[x1]",
        "ldp x0,x1,[x2]", "ldp sp,x1,[x2]", "ldp x0,xzr,[x2]",
        "stp xzr,x1,[x2]", "ldp q0,q1,[x2]", "stp d0,d1,[x2]",
        "ldp x0,w1,[x2]", "ldp w0,w1,[x2]",
    ],
    # --- load/store: imm9 ranges of the unscaled forms ---------------------
    "ldst imm9 range": [
        "ldur x0,[x1,#255]", "ldur x0,[x1,#256]", "ldur x0,[x1,#-256]",
        "ldur x0,[x1,#-257]", "stur x0,[x1,#256]", "stur x0,[x1,#-257]",
        "ldtr x0,[x1,#255]", "ldtr x0,[x1,#256]", "ldtr x0,[x1,#-257]",
        "sttr x0,[x1,#256]", "ldur x0,[x1,#4096]", "ldur x0,[x1,#-4096]",
        "ldurb w0,[x1,#256]", "ldurh w0,[x1,#256]", "ldursb x0,[x1,#256]",
        "str x0,[x1,#32760]", "str x0,[x1,#32768]", "ldr x0,[x1,#32760]",
        "str q0,[x0,#65520]", "str q0,[x0,#65536]",
    ],
    # --- exclusives and atomics: no offset field at all --------------------
    "ldst exclusives": [
        "ldxr x0,[x1]", "ldxrb w0,[x1]", "ldxrh w0,[x1]", "ldxr w0,[x1]",
        "stxr w0,x1,[x2]", "stxrb w0,w1,[x2]", "stxr w0,w1,[x2]",
        "ldaxr x0,[x1]", "stlxr w0,x1,[x2]", "ldxr x0,[x1,#8]",
        "stxr w0,x1,[x2,#8]", "stxr x0,x1,[x2]", "ldxr sp,[x1]",
        "cas x0,x1,[x2]", "casa x0,x1,[x2]", "casal x0,x1,[x2]",
        "cas x0,w1,[x2]", "cas w0,w1,[x2]", "cas x0,x1,[x2,#8]",
        "casp x0,x1,x2,x3,[x4]", "casp w0,w1,w2,w3,[x4]",
        "casb w0,w1,[x2]", "cash w0,w1,[x2]",
        "swp x0,x1,[x2]", "swpa x0,x1,[x2]", "swp x0,x1,[x2,#8]",
        "swp w0,w1,[x2]", "swp x0,w1,[x2]",
        "ldadd x0,x1,[x2]", "ldadda x0,x1,[x2]", "ldadd x0,x1,[x2,#8]",
        "stadd x0,[x1]", "staddl x0,[x1]", "ldset x0,x1,[x2]",
        "ldsmax x0,x1,[x2]", "ldumin x0,x1,[x2]", "ldclr x0,x1,[x2]",
        "steor x0,[x1]", "swpl x0,x1,[x2]",
    ],
    # --- prefetch: the register offset form is a different opcode ---------
    "ldst prfm": [
        "prfm pldl1keep,[x0]", "prfm pldl1keep,[x0,#32760]",
        "prfm pldl1keep,[x0,#32768]", "prfm pldl1keep,[x0,#8]!",
        "prfm pldl1keep,[x0],#8", "prfm pldl1keep,[x0,x1]",
        "prfm pldl1keep,[x0,x1,lsl #3]", "prfm pldl1keep,[x0,w1,uxtw]",
        "prfm pldl1strm,[x0]", "prfm pstl1keep,[x0]", "prfm pldl2strm,[x0]",
        "prfum pldl1keep,[x0,#8]", "prfum pldl1keep,[x0,#-8]",
        "prfum pldl1keep,[x0,#256]", "prfm pldl1keep,#255",
        "prfm #15,[x0]", "prfm #16,[x0]", "prfm pldl1keep,[sp]",
    ],
    # --- system: immediate fields that are not free-form ------------------
    "system immediates": [
        "svc #0", "svc #1", "svc #65535", "svc #65536", "svc #-1",
        "hvc #65535", "hvc #65536", "smc #65535", "smc #65536",
        "brk #0", "brk #65535", "brk #65536", "brk #-1",
        "hlt #65535", "hlt #65536",
        "hint #0", "hint #127", "hint #128", "hint #255", "hint #256",
        "msr daifset,#0", "msr daifset,#15", "msr daifset,#16",
        "msr daifclr,#15", "msr daifclr,#16",
        "msr spsel,#0", "msr spsel,#1", "msr spsel,#2", "msr spsel,#3",
        "msr pan,#0", "msr pan,#1", "msr pan,#2",
        "msr ssbs,#0", "msr ssbs,#1", "msr ssbs,#2",
        "msr dit,#0", "msr dit,#1", "msr tco,#0", "msr tco,#1",
    ],
    # --- system: Rt is GP-only, and width matters -------------------------
    "system Rt": [
        "mrs x0,currentel", "mrs w0,currentel", "mrs d0,currentel",
        "mrs xzr,currentel", "mrs sp,currentel",
        "msr nzcv,x0", "msr nzcv,w0", "msr nzcv,d0", "msr nzcv,sp",
        "msr nzcv,xzr", "msr sp_el0,x0", "msr sp_el0,w0",
        "dc zva,x0", "dc zva,w0", "dc zva,sp", "dc ivac,x0",
        "ic iallu", "ic ialluis", "ic iallu,x0", "ic ivau,x0", "ic ivau,w0",
        "tlbi vmalle1", "tlbi vmalle1is", "tlbi vmalle1,x0", "tlbi vae1,x0",
        "tlbi vae1,w0", "at s1e1r,x0", "at s1e1r,w0", "at s1e1r",
    ],
    # --- system: sys/sysl field ranges ------------------------------------
    "system sys": [
        "sys #0,#0,#0,#0,#0", "sys #0,#7,#15,#15,#7", "sys #0,#8,#0,#0,#0",
        "sys #0,#0,#16,#0,#0", "sys #0,#0,#0,#16,#0", "sys #0,#0,#0,#0,#8",
        "sys #1,#0,#0,#0,#0", "sys #3,#0,#0,#0,#0", "sys #0,#7,#15,#15,#7,x0",
        "sys #0,#0,#0,#0,#0,x0", "sys #0,#0,#0,#0,#0,d0",
        "sysl x0,#0,#0,#0,#0,#0", "sysl x0,#0,#0,#0,#0,#0,#7",
    ],
    # --- data processing: wide immediates and bit numbers -----------------
    "dp wide immediates": [
        "movz x0,#0x1234", "movz w0,#0xffff", "movz w0,#0x10000",
        "movz x0,#0xffff", "movz x0,#0x10000", "movz x0,#0x10000,lsl #16",
        "movz x0,#0x10000,lsl #32", "movz x0,#0x10000,lsl #48",
        "movz x0,#0x1,lsl #64", "movz x0,#0x1,lsl #4", "movz d0,#1",
        "movz sp,#1", "movz xzr,#1", "movk x0,#1", "movk x0,#1,lsl #48",
        "movk w0,#1,lsl #16", "movk x0,#0x10000,lsl #16",
        "movn x0,#1", "movn w0,#0xffff", "movn x0,#0x10000",
        "mov x0,#0x1234,lsl #16", "mov x0,#0x1234", "mov x0,#65535",
        "mov x0,#0x10000", "mov x0,#0xffff0000", "mov w0,#0x1234,lsl #16",
    ],
    "dp bit test": [
        "tbz x0,#0,.", "tbz x0,#63,.", "tbz x0,#64,.", "tbz x0,#-1,.",
        "tbz w0,#31,.", "tbz w0,#32,.", "tbnz x0,#63,.", "tbnz w0,#32,.",
        "tbz x0,#63,x1", "tbz sp,#0,.", "tbz d0,#0,.",
    ],
    # --- branch operand shapes --------------------------------------------
    "branch operands": [
        "b.eq .", "b.eq x0", "b x0", "b #0", "b .+4", "bl .", "bl x0",
        "br x0", "br sp", "br w0", "br xzr", "blr x0", "blr w0", "ret",
        "ret x0", "ret sp", "cbz x0,.", "cbz x0,x1", "cbz x0", "cbnz w0,.",
        "cbz d0,.", "tbz x0,#1", "b.eq .+8, x0",
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


# --------------------------------------------------------------- sweeps ----
# The curated rows above answer "may this operand combination be written at
# all".  They are the wrong instrument for the other half of an encoder bug --
# whether a multi-bit field lands where the oracle puts it -- because a
# permutation of such a field is invisible at its endpoints.  Lane 0 and lane 3
# (and every other bit-palindrome) encode identically under any rotation of the
# index bits, so a matrix that lists only 0, 3 and 7 agrees with an encoder that
# has H and L exchanged, and *disagrees with the architecture*.
#
# So every index, shift and selector field is enumerated instead of sampled,
# including the first out-of-range value, which is the row that catches an
# encoder that masks a field instead of rejecting the operand (`umov w0,v1.b[16]`
# encoded lane 0 silently).
#
# These are generated rather than pasted so the coverage is derived from the
# architecture's element counts and cannot quietly decay when a group is edited.
def _sweep_by_element() -> list[str]:
    rows: list[str] = []
    for mn in ("fmul", "fmulx", "fmla", "fmls"):
        for arr, n in (("2s", 4), ("4s", 4), ("4h", 8), ("8h", 8), ("2d", 2)):
            el = arr[-1]
            for i in range(n + 1):
                rows.append(f"{mn} v0.{arr},v1.{arr},v2.{el}[{i}]")
        for el, n in (("h", 8), ("s", 4), ("d", 2)):
            for i in range(n + 1):
                rows.append(f"{mn} {el}0,{el}1,v2.{el}[{i}]")
    return rows


def _sweep_lane() -> list[str]:
    """DUP / INS / MOV / UMOV / SMOV / EXT: every lane of every arrangement."""
    rows: list[str] = []
    for arr, el, n in (("16b", "b", 16), ("8b", "b", 8), ("8h", "h", 8),
                       ("4h", "h", 4), ("4s", "s", 4), ("2s", "s", 4),
                       ("2d", "d", 2)):
        for i in range(n + 1):
            rows.append(f"dup v0.{arr},v1.{el}[{i}]")
            rows.append(f"ins v0.{el}[{i}],v1.{el}[{i}]")
            rows.append(f"ins v0.{arr}[{i}],v1.{el}[{i}]")
            rows.append(f"mov v0.{el}[{i}],v1.{el}[{i}]")
            rows.append(f"mov v0.{arr}[{i}],v1.{arr}[{i}]")
    for arr, el, n in (("16b", "b", 16), ("8b", "b", 8), ("8h", "h", 8),
                       ("4h", "h", 4), ("4s", "s", 4), ("2s", "s", 4),
                       ("2d", "d", 2)):
        for i in range(n + 1):
            for dest in ("w0", "x0"):
                rows.append(f"umov {dest},v1.{el}[{i}]")
                rows.append(f"smov {dest},v1.{el}[{i}]")
    for i in range(16 + 1):
        rows.append(f"ext v0.16b,v1.16b,v2.16b,#{i}")
    return rows


def _sweep_shift() -> list[str]:
    """SHL/USHR/SSHR/SLI/SRI/SHLL/SHRN: every shift of every arrangement."""
    rows: list[str] = []
    lanes = (("16b", 8), ("8b", 8), ("8h", 16), ("4h", 16), ("4s", 32),
             ("2s", 32), ("2d", 64))
    for arr, w in lanes:
        for s in range(w + 1):
            rows.append(f"shl v0.{arr},v1.{arr},#{s}")
        for s in range(w + 1):
            rows.append(f"ushr v0.{arr},v1.{arr},#{s}")
            rows.append(f"sshr v0.{arr},v1.{arr},#{s}")
        for s in range(0, w + 1, max(1, w // 4)):
            rows.append(f"shl v0.{arr},v1.{arr},#{s}") if False else None
            rows.append(f"sli v0.{arr},v1.{arr},#{s}")
            rows.append(f"sri v0.{arr},v1.{arr},#{s}")
    rows.append("ushr v0.16b,v1.16b,#8")
    rows.append("sshr v0.16b,v1.16b,#8")
    rows.append("shl v0.16b,v1.16b,#8")
    for dst, src, w in (("8h", "8b", 8), ("4s", "4h", 16), ("2d", "2s", 32)):
        rows.append(f"shll v0.{dst},v1.{src},#{w}")
        rows.append(f"shll v0.{dst},v1.{src},#{w - 1}")
        rows.append(f"shll v0.{dst},v1.{src},#{w + 1}")
    for dst, src, w in (("8b", "8h", 8), ("4h", "4s", 16), ("2s", "2d", 32)):
        for s in (0, 1, w // 2, w, w + 1):
            rows.append(f"shrn v0.{dst},v1.{src},#{s}")
            rows.append(f"sqshrn v0.{dst},v1.{src},#{s}")
            hi_dst = {"8b": "16b", "4h": "8h", "2s": "4s"}[dst]
            rows.append(f"sqshrn2 v0.{hi_dst},v1.{src},#{s}")
            rows.append(f"shrn2 v0.{hi_dst},v1.{src},#{s}")
    # The long shifts: SSHLL/USHLL take an amount (0..esize-1), the SHLL
    # aliases fold it in (exactly esize) and live in their own encoding space.
    for i in (0, 1):
        suffix = "2" if i else ""
        for dst, src, w in (("8h", "8b", 8), ("4s", "4h", 16), ("2d", "2s", 32)):
            hi = "16b" if src == "8b" else ("8h" if src == "4h" else "4s")
            for mn in ("ushll", "sshll"):
                for sh in (0, 1, w // 2, w - 1, w, w + 1):
                    rows.append(f"{mn}{suffix} v0.{dst},v1.{src if not i else hi},#{sh}")
            for sh in (0, 1, w - 1, w, w + 1):
                rows.append(f"shll{suffix} v0.{dst},v1.{src if not i else hi},#{sh}")
    rows.append("shll v0.8h,v1.16b,#8")
    rows.append("shll2 v0.8h,v1.8b,#8")
    rows.append("shll v0.8h,v1.8b,#8")
    rows.append("shll2 v0.8h,v1.16b,#8")
    rows.append("shll v0.4s,v1.4h,#16")
    rows.append("shll2 v0.4s,v1.8h,#16")
    rows.append("shll v0.2d,v1.2s,#32")
    rows.append("shll2 v0.2d,v1.4s,#32")
    for dst, src in (("8h", "8b"), ("8h", "16b"), ("4s", "4h"), ("4s", "8h"), ("2d", "2s"), ("2d", "4s")):
        rows.append(f"uxtl v0.{dst},v1.{src}")
        rows.append(f"sxtl v0.{dst},v1.{src}")
        rows.append(f"uxtl2 v0.{dst},v1.{src}")
        rows.append(f"sxtl2 v0.{dst},v1.{src}")
    return rows


def _sweep_elem_gp() -> list[str]:
    """The element<->GPR moves: DUP/INS/MOV/UMOV/SMOV with both widths."""
    rows: list[str] = []
    for arr, n in (("16b", 16), ("8b", 8), ("8h", 8), ("4h", 4), ("4s", 4),
                   ("2s", 4), ("2d", 2)):
        el = arr[-1]
        for gp in ("w1", "x1", "wzr", "xzr", "sp"):
            rows.append(f"dup v0.{arr},{gp}")
        for i in (0, n - 1, n):
            for gp in ("w1", "x1"):
                rows.append(f"ins v0.{el}[{i}],{gp}")
                rows.append(f"mov v0.{el}[{i}],{gp}")
                rows.append(f"umov {gp},v1.{el}[{i}]")
                rows.append(f"smov {gp},v1.{el}[{i}]")
    for gp in ("w0", "x0", "sp", "xzr", "wzr"):
        for el, i in (("b", 3), ("h", 2), ("s", 1), ("d", 1)):
            rows.append(f"mov {gp},v1.{el}[{i}]")
    return rows


def _sweep_movi() -> list[str]:
    """MOVI/MVNI: every immediate byte and every legal shift."""
    rows: list[str] = []
    for imm in (0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x81, 0x18,
                0x24, 0x42, 0x0F, 0xF0, 0x55, 0xAA, 0x3C, 0xC3, 0xFF):
        for arr in ("16b", "8b"):
            rows.append(f"movi v0.{arr},#{imm}")
            rows.append(f"mvni v0.{arr},#{imm}")
    for arr in ("16b", "8b", "8h", "4h", "4s", "2s", "2d"):
        for s in (0, 8, 16, 24):
            rows.append(f"movi v0.{arr},#0x1,lsl #{s}")
            rows.append(f"mvni v0.{arr},#0x1,lsl #{s}")
            rows.append(f"movi v0.{arr},#0x0f,msl #{s}")
    return rows


def _sweep_ld_lane() -> list[str]:
    """LD1-LD4 single-structure lane forms: every lane of every arrangement."""
    rows: list[str] = []
    for arr, el, n in (("16b", "b", 16), ("8b", "b", 8), ("8h", "h", 8),
                       ("4h", "h", 4), ("4s", "s", 4), ("2s", "s", 4),
                       ("2d", "d", 2), ("1d", "d", 1)):
        for i in range(n + 1):
            rows.append(f"ld1 {{v0.{el}}}[{i}],[x1]")
    for arr, el, n in (("16b", "b", 16), ("8h", "h", 8), ("4s", "s", 4),
                       ("2d", "d", 2)):
        for i in range(n + 1):
            regs = ",".join(f"v{k}.{el}" for k in range(2))
            rows.append(f"ld2 {{{regs}}}[{i}],[x1]")
            regs = ",".join(f"v{k}.{el}" for k in range(3))
            rows.append(f"ld3 {{{regs}}}[{i}],[x1]")
            regs = ",".join(f"v{k}.{el}" for k in range(4))
            rows.append(f"ld4 {{{regs}}}[{i}],[x1]")
    return rows


def _sweep_sysreg() -> list[str]:
    """Every named system register, in both directions, plus the families.

    The name list is parsed out of the encoder itself rather than duplicated
    here: a register added to `SYSREGS` (or a family the resolver synthesises)
    becomes a matrix row on the next regeneration, so a new register cannot be
    added without its GNU-as verdict being pinned.  Both directions are emitted
    because the two used to be separate hand-written tables that drifted --
    `mrs x0,pan` was rejected while `msr pan,#1` was accepted, and 28 registers
    were readable but not writable.  The parse is fail-closed: no rows means the
    table moved, and silently emitting nothing would retire the coverage.
    """
    source = (Path(__file__).resolve().parent.parent
              / "src/backend/arm/assembler/encoder/sysreg_table.rs").read_text()
    names = re.findall(r'^\s*\("([a-z0-9_]+)", 0x[0-9a-f]{4}\),\s*$',
                       source, re.MULTILINE)
    if len(names) < 100:
        raise SystemExit(
            f"aarch64_operand_legality_matrix: parsed only {len(names)} system "
            "registers out of encoder/sysreg_table.rs; the generated table must "
            "have moved, refusing to regenerate a smaller matrix"
        )
    out: list[str] = []
    for name in names:
        out.append(f"mrs x0,{name}")
        out.append(f"msr {name},x0")
    # The synthesised families (numbered debug/performance registers and the
    # raw op0:op1:CRn:CRm:op2 spelling), sampled exhaustively at their ends --
    # the numbering is arithmetic, so 0 and the last index exercise the carries.
    for pre in ("dbgbcr", "dbgbvr", "dbgwcr", "dbgwvr"):
        for n in (0, 1, 7, 14, 15):
            out.append(f"mrs x0,{pre}{n}_el1")
            out.append(f"msr {pre}{n}_el1,x0")
    for pre in ("pmevcntr", "pmevtyper"):
        for n in (0, 7, 8, 15, 30, 31):
            out.append(f"mrs x0,{pre}{n}_el0")
    out.append("mrs x0,dbgbcr16_el1")
    out.append("msr dbgwvr15_el1,x0")
    for op0, op1, crn, crm, op2 in ((3, 0, 1, 0, 1), (3, 3, 4, 2, 2),
                                    (2, 0, 0, 7, 5), (3, 7, 15, 15, 7),
                                    (1, 0, 2, 3, 4)):
        out.append(f"mrs x0,s{op0}_{op1}_c{crn}_c{crm}_{op2}")
    # The raw spelling has one legal field width per position, and every
    # out-of-range digit must be rejected rather than masked into a different
    # register: op0 is 2 bits, op1/op2 are 3, CRn/CRm are 4.  Leading zeros in
    # a field and a lower-case-only prefix are the other side of the same
    # question, and GNU as accepts both (as `S3_0_C1_C0_1` too).
    for spelling in ("s4_0_c1_c0_1", "s3_8_c1_c0_1", "s3_0_c16_c0_1",
                     "s3_0_c1_c16_1", "s3_0_c1_c0_8", "s3_0_c1_c0"):
        out.append(f"mrs x0,{spelling}")
        out.append(f"msr {spelling},x0")
    for spelling in ("s03_0_c1_c0_1", "s3_0_c01_c0_1", "S3_0_C1_C0_1",
                     "S3_0_c1_C0_1", "s0_0_c0_c0_0", "s3_0_c15_c15_7"):
        out.append(f"mrs x0,{spelling}")
        out.append(f"msr {spelling},x0")
    # GNU as is case-insensitive about register NAMES as well as about the
    # prefix letters of the raw spelling: `mrs x0,DBGBVR7_EL1`,
    # `mrs x0,DbgBvr7_El1`, `msr PAN,x0` and `msr PAN,x0` all assemble to the
    # words their lower-case spellings do (measured with 2.47).  Only the
    # instruction encoders lower-cased the operand, so the resolver itself
    # disagreed one layer down; these four rows pin the resolver's contract at
    # the level the assembler actually exercises.  A case fold must never be a
    # way past a range check, so the out-of-range raw spellings above are also
    # probed in upper case below.
    for insn in ("mrs x0,DBGBVR7_EL1", "msr DBGBVR7_EL1,x0",
                 "mrs x0,DbgBvr7_El1", "msr PAN,x0", "mrs x0,PAN"):
        out.append(insn)
    for spelling in ("S4_0_C1_C0_1", "S3_8_C1_C0_1", "S3_0_C16_C0_1",
                     "S3_0_C1_C0_8"):
        out.append(f"mrs x0,{spelling}")
        out.append(f"msr {spelling},x0")
    return out



def _sweep_ldst_family() -> list[str]:
    """The whole load/store family, mnemonic by mnemonic, GAS-derived.

    Three addressing bases (unsigned offset `ldr`/`str`, unscaled `ldur`/`stur`,
    unprivileged `ldtr`/`sttr`) crossed with seven spellings (bare, `b`, `h`,
    `sb`, `sh`, `sw`) is the complete family, and nowhere else in this file is
    the *destination class* pinned per mnemonic.  That omission is exactly how
    five defects lived here at once, all of them silent:

      * `ldtr s0,[x0,#8]` (and d/q/b/h) assembled, though the unprivileged base
        has no FP/SIMD form at all;
      * `ldtrb x0`, `ldtrh x0`, `sttrb x0`, `sttrh x0` assembled as their 32-bit
        forms, though the byte/halfword Rt is w-only;
      * `ldurb`/`ldurh`/`sturb`/`sturh`/`ldursw`/`ldursb`/`ldursh` and
        `ldtrsb`/`ldtrsh`/`ldtrsw` were missing entirely (GNU as accepts all
        ten);
      * the register-offset *amount* was never validated, so `lsl #1` on a
        64-bit load encoded LSL #3 -- a different shift than was written -- and
        a bare `w` index was guessed to be UXTW;
      * `[Xn,Xm]!` and `[Xn],Xm` silently dropped the writeback (the parser lost
        the `!`, the encoder ignored the extra operand).

    The first three are this sweep's mnemonic x register-class grid; the last
    two are its addressing-mode rows.  Every expectation is GNU as's verdict at
    regeneration time, and the `--check-lccc` half pins the encoder to the same
    table, so the family cannot drift apart again without a gate going red.
    """
    out: list[str] = []
    for mn in ("ldr", "str", "ldrb", "strb", "ldrh", "strh", "ldrsb", "ldrsh",
               "ldrsw", "ldur", "stur", "ldurb", "sturb", "ldurh", "sturh",
               "ldursb", "ldursh", "ldursw", "ldtr", "sttr", "ldtrb", "sttrb",
               "ldtrh", "sttrh", "ldtrsb", "ldtrsh", "ldtrsw"):
        for rt in ("w0", "x0", "s0", "d0", "q0", "b0", "h0"):
            out.append(f"{mn} {rt},[x0]")
            out.append(f"{mn} {rt},[x0,#8]")
    # Addressing forms: the writeback-register spellings have no encoding in
    # any of the three bases, the register-offset form exists only for the
    # unsigned-offset base, and the amount must be 0 or log2(width).
    for base in ("ldr", "str"):
        for rt in ("x0", "w0", "q0", "b0"):
            for form in ("[x0],x1", "[x0,x1]!", "[x0],#8", "[x0,#8]!",
                         "[x0,x1]", "[x0, x1, lsl #3]", "[x0, x1, lsl #1]",
                         "[x0, w1]", "[x0, w1, uxtw]", "[x0, w1, uxtw #2]"):
                out.append(f"{base} {rt},{form}")
    for base in ("ldur", "ldtr", "ldurb", "ldtrh"):
        for form in ("[x0],#8", "[x0,#8]!", "[x0,x1]"):
            out.append(f"{base} w0,{form}")
    return out

# ── register-class sweep ───────────────────────────────────────────────────
#
# The audit that produced this group ("GP-class bypasses remain", PR #766
# review gate §1) found four reader functions that checked a couple of
# spellings and then fell back to the permissive `parse_reg_num`, which accepts
# the FP/SIMD width letters: `br d0` assembled as `br x0`, `ccmp x0,d0,#0,eq`
# as `ccmp x0,x0,#0,eq`.  Patching those four functions answers the review;
# sweeping the whole operand surface answers the *class*, which is what the
# audit was actually about.  For every GP-only slot of every GP-only mnemonic
# the family covers, this substitutes each FP/SIMD spelling (and, where the
# slot's role is interesting, the 31-spellings) and pins GNU as's verdict.
#
# The expected outcome is not "everything rejects": `clz xzr, x1` and
# `mov sp, x1` are legal, and the sweep pins those words too, so an over-eager
# class check that refused them would fail here as well.
_GP_SLOT_TEMPLATES: list[str] = [
    # data processing (1 source)
    "clz {r}, x1", "clz x0, {r}", "cls {r}, x1", "rbit x0, {r}", "rev {r}, x1",
    "rev16 w0, {r}", "rev32 x0, {r}", "rev16 {r}, w1",
    # 2-source arithmetic / logical
    "add {r}, x1, x2", "add x0, {r}, x2", "add x0, x1, {r}",
    "sub {r}, x1, x2", "subs x0, {r}, #1", "sub x0, x1, {r}, lsl #2",
    "cmp {r}, x1", "cmp x0, {r}", "cmn w0, {r}",
    "mul {r}, x1, x2", "mneg x0, {r}, x2",
    "udiv {r}, x1, x2", "sdiv x0, {r}, x2",
    "lsl {r}, x1, #3", "lsl x0, {r}, #3", "lsr x0, x1, {r}",
    "and {r}, x1, x2", "orr x0, {r}, x2", "eor x0, x1, {r}",
    "ands {r}, x1, #0xff", "tst {r}, x1", "bic x0, x1, {r}", "orn {r}, x1, x2",
    # 3-source
    "madd {r}, x1, x2, x3", "madd x0, x1, x2, {r}", "msub x0, {r}, x2, x3",
    "smulh {r}, x1, x2", "umulh x0, {r}, x2",
    "smaddl {r}, w1, w2, x3", "umaddl x0, w1, w2, {r}",
    # moves / immediate forms
    "mov {r}, x1", "mov x0, {r}", "movz {r}, #1", "movn x0, {r}",
    "movk {r}, #1, lsl #16", "mov {r}, #1",
    # extends / bitfield / extract
    "sxtb {r}, w1", "uxtb x0, {r}", "sxth {r}, w1", "uxth x0, {r}",
    "sxtw {r}, w1", "uxtw {r}, w1", "ubfx {r}, x1, #1, #2",
    "sbfiz x0, {r}, #1, #2", "bfi x0, {r}, #1, #2", "extr {r}, x1, x2, #3",
    # branches and conditional forms
    "br {r}", "blr {r}", "ret {r}", "cbz {r}, .", "cbnz w0, .",
    "tbz x0, #1, .", "tbnz {r}, #1, .",
    "csel {r}, x1, x2, eq", "csinc x0, x1, {r}, eq", "csinv x0, {r}, x2, eq",
    "csneg x0, x1, {r}, eq", "ccmp {r}, x1, #0, eq", "ccmp x0, {r}, #0, eq",
    "ccmn w0, {r}, #0, ne",
    # load/store transferred registers
    "ldr {r}, [x1]", "str {r}, [x1]", "ldrb {r}, [x1]", "strh {r}, [x1]",
    "ldrsb {r}, [x1]", "ldrsh w0, [x1]", "ldrsw {r}, [x1]", "ldur {r}, [x1, #8]",
    "stur {r}, [x1, #8]", "ldtr {r}, [x1, #8]",
    "ldp {r}, x1, [x2]", "stp x0, {r}, [x1]", "ldnp x0, {r}, [x1]",
    "ldxr {r}, [x1]", "stxr w0, {r}, [x1]", "ldaxr {r}, [x1]",
    "cas x0, {r}, [x1]", "casb {r}, w1, [x2]", "swp {r}, x1, [x2]",
    "ldadd {r}, x1, [x2]", "stadd {r}, [x1]", "ldsmax {r}, x1, [x2]",
    "ldumin x0, {r}, [x2]", "casp {r}, x1, x2, x3, [x4]",
    # address slots
    "ldr x0, [{r}]", "str x0, [{r}, #8]", "ldr x0, [{r}, #8]!",
    "ldr x0, [x1, {r}]", "ldr x0, [x1, {r}, uxtw]", "prfm pldl1keep, [{r}]",
    # system instructions
    "mrs {r}, nzcv", "msr nzcv, {r}", "ic ivau, {r}", "dc cvau, {r}",
    "at s1e1r, {r}", "tlbi vae1, {r}", "sys #3, c1, c0, #1, {r}",
    "sysl {r}, #3, c1, c0, #1",
    # FP/SIMD-only slots, with GP spellings substituted in
    "fmov {r}, d1", "fadd {r}, d1, d2", "fmul {r}, s1, s2", "fcmp {r}, d1",
    "scvtf {r}, w1", "fcvtzs w0, {r}", "fmin {r}, d1, d2",
    "fmadd {r}, d1, d2, d3", "fabs {r}, d1", "fsqrt {r}, d1",
    # The rest of the families the same audit touched, added when their
    # encoders were rewritten: each of these has a width or class rule that is
    # not visible in the family's first template.
    "smull {r}, w1, w2", "smull x0, {r}, w2", "umull x0, w1, {r}",
    "smaddl x0, {r}, w2, x3", "umaddl x0, w1, {r}, x3",
    "madd x0, {r}, x2, x3", "msub {r}, x1, x2, x3", "mneg {r}, x1, x2",
    "udiv x0, {r}, x2", "smulh x0, {r}, x2", "umulh {r}, x1, x2",
    "adc {r}, x1, x2", "sbc x0, x1, {r}",
    "cset {r}, eq", "csetm {r}, ne", "cneg {r}, x1, eq",
    "cinc x0, x1, {r}", "cinv x0, {r}, x1", "cinc {r}, x1, eq",
    "bfxil x0, {r}, #1, #2", "ubfiz x0, {r}, #1, #2", "bfm x0, {r}, #1, #2",
    "ubfm {r}, x1, #1, #2", "sbfm x0, {r}, #1, #2",
    "crc32b w0, w1, {r}", "crc32h {r}, w1, w2", "crc32x w0, w1, {r}",
    "crc32cx {r}, w1, w2", "crc32cw w0, {r}, w1",
    "ldrsh {r}, [x1]", "ldrh {r}, [x1]", "strb {r}, [x1]",
    "ldursb {r}, [x1, #8]", "ldurh {r}, [x1, #8]", "ldursw {r}, [x1, #8]",
    "sturh {r}, [x1, #8]", "ldtrsb {r}, [x1, #8]",
    "ldxrb {r}, [x1]", "ldaxrh {r}, [x1]", "stxrb {r}, x1, [x2]",
    "stxr {r}, x1, [x2]", "ldaxp {r}, x1, [x2]", "ldxp x0, {r}, [x2]",
    "stxp {r}, x1, x2, [x3]", "swpb {r}, w1, [x2]", "ldsmaxh {r}, w1, [x2]",
    "casah {r}, w1, [x2]", "cas x0, {r}, [x1]",
    "movn {r}, #1", "movn x0, {r}",
    "fcvt {r}, s1", "fcvt s0, {r}", "frintn {r}, s1", "fcvtms {r}, s1",
    "fcvtas w0, {r}", "fcvtzu {r}, s1", "fcvtzu w0, {r}", "fcvtns x0, {r}",
    "extr x0, {r}, x2, #3", "extr x0, x1, {r}, #3",
    # The scalar SIMD&FP conversion family: twelve mnemonics, each with a
    # register-file form whose two operands must be spelled at one width
    # (`fcvtms s0,s1`, `scvtf d0,d1`) and, for the four integer-conversion
    # mnemonics, a fixed-point form whose bit count is bounded by the element
    # width (`fcvtzs s0,s1,#fbits`).  Which form a spelling reaches depends only
    # on how its operands are spelled, so the two-operand conversion slot and
    # the fixed-point slot are swept separately -- and the sweep substitutions
    # supply the widths that make the mixed and the same-width cases differ.
    "fcvtms {r}, s1", "fcvtzu d0, {r}", "fcvtas s0, {r}", "fcvtns {r}, d1",
    "fcvt h0, {r}", "fcvt {r}, d1",
    "scvtf {r}, s1", "ucvtf h0, {r}", "scvtf d0, {r}", "ucvtf {r}, s1",
    "fcvtzs s0, {r}, #3", "fcvtzs {r}, s1, #3", "fcvtzu d0, {r}, #7",
    "fcvtzu {r}, d1, #7", "scvtf h0, {r}, #3", "scvtf {r}, w1, #3",
    "ucvtf {r}, s1, #3", "ucvtf d0, {r}, #3",
    "fcvtas {r}, s1, #3", "fcvtmu h0, {r}, #3",
    # The exclusive/acquire-release spellings that carry the W-only rule in
    # their *mnemonic* rather than in a size field argument.
    "stlxrb {r}, w1, [x2]", "ldaxrb {r}, [x1]", "ldarb {r}, [x1]",
    "stlrh {r}, w1, [x2]", "ldarh {r}, [x1]", "stlxp {r}, w1, w2, [x3]",
    "stlxp w0, {r}, w2, [x3]", "ldaxp {r}, w1, [x2]", "ldaxp w0, {r}, [x2]",
    "adcs {r}, x1, x2", "adc x0, x1, {r}", "sbcs x0, {r}, x2",
    # CASP's four register operands are two pairs, and only the first half of
    # each is encoded: the sweep has to reach the *second* half too, because
    # that is where the "contiguous" rule and the zero-register spelling of
    # register 31 (`casp x30,xzr,...`) live.
    "casp x30, {r}, x0, x1, [x4]", "casp x0, {r}, x2, x3, [x4]",
    "casp x0, x1, x30, {r}, [x4]", "casp x2, {r}, x4, x5, [x8]",
    "caspal x30, {r}, x2, x3, [x4]", "caspl w30, {r}, w2, w3, [x4]",
    "casp {r}, x1, x2, x3, [sp]", "casp x0, x1, x2, x3, [{r}]",
]

# The FP/SIMD spellings are the point of the sweep; the 31-spellings and the
# identity aliases are included for the slots where the architectural role of
# encoding 31 is the question.
_GP_SLOT_SUBSTITUTIONS: tuple[str, ...] = (
    "d0", "s0", "h0", "b0", "q0", "v0", "d31", "s31", "v31",
    "sp", "wsp", "xzr", "wzr", "lr",
    # The width dimension.  A slot's *class* and its *width* are separate
    # questions -- `mul x0, x1, x2` and `mul w0, x1, x2` differ only in the
    # second -- and the encoders get them wrong independently: the multiply
    # family ignored widths entirely, `ldur lr, [x1, #8]` asked the spelling
    # (`starts_with('x')`) instead of the register, and the CRC32 forms take a
    # 32-bit destination with a 64-bit source in their `x` variants.
    "x0", "x30", "w0", "w30",
)


def _sweep_register_class() -> list[str]:
    out: list[str] = []
    for tmpl in _GP_SLOT_TEMPLATES:
        for r in _GP_SLOT_SUBSTITUTIONS:
            out.append(tmpl.format(r=r))
    return out


# ── SYS/SYSL field domain ─────────────────────────────────────────────────
#
# The audit called the old coverage "vacuous": thirteen rows, every one of them
# a malformed operand shape, so all thirteen rejected before a single field was
# validated -- the rows proved the parser could say no, never that it could say
# yes or that a field range was enforced.  This generator starts from valid
# controls (which must assemble, and whose *words* are the encoding under
# test), then varies one thing at a time: each field one past its architectural
# maximum, the `c` prefix, the optional Rt, its width and class, and the operand
# count in both directions.
_SYS_FIELDS: list[str] = []


def _sweep_system_sys() -> list[str]:
    out: list[str] = []
    # Valid controls across the field domain (op1 0..7, CRn/CRm 0..15, op2 0..7).
    out += [
        "sys #0,c0,c0,#0",
        "sys #3,c1,c0,#1",
        "sys #7,c15,c15,#7",
        "sys #7,c15,c15,#7,x0",
        "sys #3,c1,c0,#1,xzr",
        "sys #3,c1,c0,#1,x30",
        "sys #7,c15,c15,#7,lr",
        "sys 3,c1,c0,1",          # GNU as: `#` is optional on both imm fields
        "sys #0x7,c15,c15,#0x7",  # ...and the value may be hex
        "sysl x0,#0,c0,c0,#0",
        "sysl x0,#3,c1,c0,#1",
        "sysl xzr,#7,c15,c15,#7",
        "sysl lr,#7,c15,c15,#7",
        # One past each field maximum: the field is 3 or 4 bits wide, so a
        # value that does not fit must be refused, not masked into another
        # system operation.
        "sys #8,c0,c0,#0",
        "sys #3,c16,c0,#1",
        "sys #3,c0,c16,#1",
        "sys #3,c0,c0,#8",
        "sysl x0,#8,c0,c0,#0",
        "sysl x0,#3,c16,c0,#1",
        "sysl x0,#3,c0,c16,#1",
        "sysl x0,#3,c0,c0,#8",
        # The `c` of CRn/CRm is part of the grammar.
        "sys #3,1,c0,#1",
        "sys #3,c1,0,#1",
        "sys #3,c0x1,c0,#1",
        # Negative and non-numeric field values.
        "sys #-1,c0,c0,#0",
        "sys #3,c-1,c0,#1",
        "sys #3,c1,c0,#x",
        # Rt: a register (or XZR) for sys, mandatory X for sysl, never W/SP/FP.
        "sys #3,c1,c0,#1,w0",
        "sys #3,c1,c0,#1,d0",
        "sys #3,c1,c0,#1,sp",
        "sys #3,c1,c0,#1,x31",
        "sys #3,c1,c0,#1,#0",
        "sysl w0,#3,c1,c0,#1",
        "sysl d0,#3,c1,c0,#1",
        "sysl sp,#3,c1,c0,#1",
        "sysl #3,c1,c0,#1",
        "sysl x0,#3,c1,c0,#1,x1",
        # Operand-count boundaries.
        "sys #3,c1,c0",
        "sys #3,c1,c0,#1,x0,x1",
        "sys #3,c1,c0,#1,",
    ]
    return out


# ── atomic arity ───────────────────────────────────────────────────────────
#
# Review gate §4: CAS, SWP, LDOP and STOP used minimum-arity checks and
# ignored trailing operands, so `cas x0,x1,[x2],x3` -- assembly no
# architecture defines -- assembled with the extra operand dropped.  A dropped
# operand is never harmless when the form it was dropped from has an imm9 or a
# width field a reader could disagree about; the family is now exact-arity, and
# these rows pin that for every member.
def _sweep_atomic_arity() -> list[str]:
    out: list[str] = []
    forms = [
        ("cas x0, x1, [x2]", True),
        ("casa x0, x1, [x2]", True),
        ("casb w0, w1, [x2]", True),
        ("cas x0, x1, [x2], x3", False),
        ("swp x0, x1, [x2]", True),
        ("swpl w0, w1, [x2]", True),
        ("swp x0, x1, [x2], x3", False),
        ("ldadd x0, x1, [x2]", True),
        ("ldaddal w0, w1, [x2]", True),
        ("ldsmax x0, x1, [x2]", True),
        ("ldumin x0, x1, [x2]", True),
        ("ldadd x0, x1, [x2], x3", False),
        ("stadd x0, [x1]", True),
        ("stclr x0, [x1]", True),
        ("stadd x0, [x1], x2", False),
        ("stadd x0, [x1], #8", False),
        ("ldxr x0, [x1]", True),
        ("ldxr x0, [x1], x2", False),
        ("stxr w0, x1, [x2]", True),
        ("stxr w0, x1, [x2], x3", False),
        ("ldaxr x0, [x1]", True),
        ("stlxr w0, x1, [x2]", True),
        ("ldxrb w0, [x1]", True),
        ("stxrb w0, w1, [x2]", True),
        ("ldxp x0, x1, [x2]", True),
        ("ldxp x0, x1, [x2], x3", False),
        ("stxp w0, x1, x2, [x3]", True),
        ("stxp w0, x1, x2, [x3], x4", False),
        ("ldar x0, [x1]", True),
        ("ldar w0, [x1], x2", False),
        ("stlr x0, [x1]", True),
        ("ldarb w0, [x1]", True),
        ("ldarh w0, [x1]", True),
        ("ldp x0, x1, [x2]", True),
        ("ldp x0, x1, [x2], x3", False),
        ("stp x0, x1, [x2]", True),
        ("ldnp x0, x1, [x2]", True),
        ("stnp x0, x1, [x2], #8", False),
        ("casp x0, x1, x2, x3, [x4]", True),
        ("casp x0, x1, x2, x3, [x4], x5", False),
        ("prfm pldl1keep, [x0]", True),
        ("prfm pldl1keep, [x0], x1", False),
        ("prfum pldl1keep, [x0, #-8]", True),
        ("prfum pldl1keep, [x0, #-8], x1", False),
        ("ldr x0, [x1]", True),
        ("ldr x0, [x1], x2", False),
        ("ldr x0, [x1], #8", True),
        ("ldrb w0, [x1]", True),
        ("ldrb w0, [x1], #1", True),
        ("ldrsb x0, [x1]", True),
        ("ldrsb x0, [x1], #1", True),
        ("ldrsw x0, [x1]", True),
        ("ldrsw x0, [x1], #4", True),
        ("adr x0, .", True),
        ("adrp x0, .", True),
    ]
    out += [f for f, _ in forms]
    # The FP/SIMD spellings of the same slots (the class dimension of §1/§2).
    for rt in ("d0", "s0", "q0", "h0", "b0", "v0"):
        out += [
            f"cas {rt}, x1, [x2]",
            f"swp x0, {rt}, [x2]",
            f"ldadd {rt}, x1, [x2]",
            f"stadd {rt}, [x1]",
            f"ldxr {rt}, [x1]",
            f"stxr w0, {rt}, [x2]",
            f"ldxr x0, [{rt}]",
            f"stxr w0, x1, [{rt}]",
            f"cas x0, x1, [{rt}]",
            f"ldar {rt}, [x1]",
            f"ldxp {rt}, x1, [x2]",
            f"stxp w0, {rt}, x2, [x3]",
            f"adrp {rt}, .",
            f"adr {rt}, .",
            f"ldp {rt}, x1, [x2]",
            f"stp {rt}, q1, [x2]",
            f"casp {rt}, x1, x2, x3, [x4]",
            f"casp x0, x1, {rt}, x3, [x4]",
        ]
    return out


SWEEPS: dict[str, list[str]] = {
    "sweep neon by-element": _sweep_by_element(),
    "sweep neon elem-gp": _sweep_elem_gp(),
    "sweep neon lane": _sweep_lane(),
    "sweep neon shift": _sweep_shift(),
    "sweep neon movi": _sweep_movi(),
    "sweep neon ld lane": _sweep_ld_lane(),
    "sweep system sysreg": _sweep_sysreg(),
    "sweep load/store family": _sweep_ldst_family(),
    "sweep register class": _sweep_register_class(),
    "sweep system sys": _sweep_system_sys(),
    "sweep atomic arity": _sweep_atomic_arity(),
}


def _dispute_family(insn: str, expected: str, got: str) -> tuple[str, str]:
    """The slot shape a disagreement belongs to: `(mnemonic, where)`.

    A fix is a register-class gate on one operand position, so the useful
    grouping is "which operand is disputed", not "which row".  Operands are
    classified by the *class the row used* on the side that disagrees, and
    disagreeing rows of one shape are contiguous in the matrix by
    construction (the sweeps vary one slot at a time).  Position 0 is the
    destination; later positions are the operands in spelling order, with the
    displacement/offset forms folded into their own bucket.
    """
    parts = [p.strip() for p in insn.split(None, 1)]
    mnemonic = parts[0]
    operands = [o.strip() for o in parts[1].split(",")] if len(parts) > 1 else []

    def klass(op: str) -> str:
        op = op.lstrip("#")
        if not op:
            return "?"
        head = op.split()[0].rstrip("![]")
        m = re.match(r"^([a-z]{1,3})\d*$", head)
        if m and m.group(1) in {"x", "w", "sp", "wsp", "xzr", "wzr"}:
            return "gp"
        if m and m.group(1) in {"d", "s", "h", "b", "q", "v"}:
            return "neon"
        if re.fullmatch(r"-?0x[0-9a-f]+|-?\d+", op):
            return "imm"
        if op.startswith("["):
            return "mem"
        return "other"

    if len(operands) <= 1:
        return (mnemonic, "single-operand")
    where = []
    for i, op in enumerate(operands):
        role = "dst" if i == 0 else f"src{i}"
        where.append(f"{role}:{klass(op)}")
    return (mnemonic, ",".join(where))


def _raw_rows() -> list[tuple[str, str]]:
    """(group, instruction) as the group and sweep tables generate them.

    The generators are exhaustive rather than economical: a sweep enumerates
    every slot of a family, and two slots can spell the *same* instruction
    (``sweep neon lane`` alone repeats 114 of its rows that way).  This is the
    pre-deduplication sequence, kept separate so `_expand`'s invariant --
    one row per instruction, first group wins -- is a property of one function
    instead of a property of every generator.
    """
    rows: list[tuple[str, str]] = []
    for group, spec in GROUPS.items():
        for insn in _labels("\n".join(spec)):
            rows.append((group, insn))
    for group, spec in SWEEPS.items():
        for insn in spec:
            rows.append((group, insn))
    return rows


def _expand() -> list[tuple[str, str]]:
    """(group, instruction) for every row of the matrix, one row per instruction.

    Deduplication is lossless here because the expectation is a function of the
    *instruction text alone*: `encode` hands the instruction to GNU as and reads
    back the encoding, and the group is a label on that verdict, not an input to
    it.  A second row for an instruction therefore re-runs an identical
    assembler invocation to re-assert an identical expectation.

    It is also load-bearing, which is why it is a property of this function
    rather than left to the generators.  Before it, the table carried 415
    duplicate rows (240 of them already in the merged 6,817-row table), and a
    sweep that *repeated* older coverage instead of contributing new keys was
    indistinguishable from one that contributed coverage: both produced rows,
    both re-ran green.  With one row per instruction, a group's row count is
    exactly the number of keys it is the first to claim, so the per-group
    ratchets in `elf_writer.rs` cannot be satisfied by repetition.
    """
    rows: list[tuple[str, str]] = []
    seen: set[str] = set()
    for group, insn in _raw_rows():
        if insn in seen:
            continue
        seen.add(insn)
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
    ap.add_argument(
        "--ratchets",
        action="store_true",
        help="print the Rust ratchet table (per-group row counts) for "
        "elf_writer.rs's operand_legality_matrix_matches_the_encoder",
    )
    ap.add_argument("--regenerate", action="store_true", help="rewrite the table")
    ap.add_argument(
        "--max-diffs",
        type=int,
        default=40,
        metavar="N",
        help="how many disagreeing rows --check-lccc prints (0 = all; "
        "the grouped summary is always printed)",
    )
    ap.add_argument("--as", dest="as_bin", default=None)
    ap.add_argument("--objcopy", default=None)
    args = ap.parse_args()

    if args.dump:
        for g, i, e in parse_table(TABLE):
            print(f"{g:34s} {i:34s} {e}")
        return 0

    if args.ratchets:
        # The ratchet is the *generator's* group sizes, not the table's, so a
        # table that lost rows shows up as a mismatch instead of silently
        # lowering its own bar.
        counts: dict[str, int] = {}
        for g, _ in _expand():
            counts[g] = counts.get(g, 0) + 1
        total = sum(counts.values())
        print(f"            (_, {total}),  // total rows")
        for g in sorted(counts):
            print(f'            ("{g}", {counts[g]}),')
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

        # Integrity before verdicts: the checked-in table must be exactly the
        # generator's output -- same (group, instruction) keys, same order, no
        # duplicates, no strays.  Without this a table could lose a whole
        # family (or gain a hand-written row) and still pass `--check`,
        # because every surviving row would agree with GNU as; the ratchet in
        # the Rust test would only notice if it happened to look at totals.
        expected_keys = _expand()
        got_keys = [(g, i) for g, i, _ in rows]
        if len(set(got_keys)) != len(got_keys):
            seen: set[tuple[str, str]] = set()
            dupes = [k for k in got_keys if k in seen or seen.add(k)]  # type: ignore[func-returns-value]
            print(
                f"operand-legality matrix: the table repeats "
                f"{len(got_keys) - len(set(got_keys))} (group, instruction) "
                f"key(s), first: {dupes[0]}",
                file=sys.stderr,
            )
            return 1
        if got_keys != expected_keys:
            missing = [k for k in expected_keys if k not in set(got_keys)]
            extra = [k for k in got_keys if k not in set(expected_keys)]
            first = next(
                (n for n, (a, b) in enumerate(zip(got_keys, expected_keys)) if a != b),
                min(len(got_keys), len(expected_keys)),
            )
            print(
                f"operand-legality matrix: the table is not the generator's "
                f"output (table {len(got_keys)} rows, generator "
                f"{len(expected_keys)}): {len(missing)} missing, {len(extra)} "
                f"extra, first divergence at row {first}"
                + (f": table has {got_keys[first]!r}, generator {expected_keys[first]!r}"
                   if first < min(len(got_keys), len(expected_keys)) else ""),
                file=sys.stderr,
            )
            for k in (missing[:5] or extra[:5]):
                print(f"  {'missing' if k in missing else 'extra'}: {k}", file=sys.stderr)
            return 1

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
                # Triage first, rows second.  A class-sweep disagreement
                # arrives in families of dozens (every FP/vector spelling of
                # one mnemonic's one GP slot), and a cap of "the first 40"
                # both truncates the families a fix has to cover and hides
                # which families they are.  The summary is grouped by the
                # *slot shape* -- mnemonic plus the operand position of the
                # disputed operand -- so one line stands for one encoder
                # reader to fix, and `--max-diffs 0` prints every row.
                families: dict[tuple[str, str], list[str]] = {}
                for group, insn, expected, got in bad:
                    families.setdefault(_dispute_family(insn, expected, got), []).append(
                        insn
                    )
                print(
                    f"  {len(families)} distinct slot shape(s); "
                    f"`--max-diffs 0` prints every row",
                    file=sys.stderr,
                )
                for (mnemonic, where), members in sorted(
                    families.items(), key=lambda kv: (-len(kv[1]), kv[0])
                ):
                    print(
                        f"  {len(members):4d}x  {mnemonic} {where}  "
                        f"e.g. {members[0]}",
                        file=sys.stderr,
                    )
                limit = args.max_diffs
                shown = bad if limit == 0 else bad[:limit]
                for group, insn, expected, got in shown:
                    print(
                        f"  [{group}] {insn}: gas={expected} lccc={got}",
                        file=sys.stderr,
                    )
                if len(shown) < len(bad):
                    print(
                        f"  ... {len(bad) - len(shown)} more row(s) "
                        f"({len(bad)} total, --max-diffs 0 for all)",
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
