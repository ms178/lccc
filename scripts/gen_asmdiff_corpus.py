#!/usr/bin/env python3
"""Generate the exhaustive assembler differential corpus for LCCC.

The goal is coverage of the *encoding decision space*, not of the mnemonic
list: bugs live wherever the encoder must CHOOSE something. Every such choice
is enumerated combinatorially:

  * REX presence/absence, and the spl/bpl/sil/dil vs ah/ch/dh/bh conflict
  * ModRM/SIB structure: rsp/r12 base (SIB required), rbp/r13 base (disp
    required), no-base SIB, index-only, every scale
  * displacement and immediate width selection at the exact boundaries
  * the sign-extended imm8 forms and the AL/AX/EAX/RAX short forms
  * RIP-relative addressing and the relocations it generates
  * segment overrides, address/operand-size overrides, LOCK/REP prefixes
  * VEX2 vs VEX3 selection, VEX.W / VEX.L / vvvv
  * branch relaxation at every displacement boundary, including chains where
    relaxing one jump changes another's distance, and relaxation interacting
    with .p2align padding
  * directives, sections, symbol attributes, symbol arithmetic, jump tables

Every generated case is validated against GNU as before being written; a case
GNU as refuses is trimmed line-by-line or dropped, so the corpus is
self-cleaning against a moving oracle and can never contain an invalid input
that would make the differential report a false failure.

Usage:
    scripts/gen_asmdiff_corpus.py --as /path/to/as --out-dir tests/asm-diff
"""
from __future__ import annotations

import argparse
import itertools
import subprocess
import sys
import tempfile
from pathlib import Path

GP64 = ["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi",
        "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15"]
GP32 = ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi",
        "r8d", "r9d", "r10d", "r11d", "r12d", "r13d", "r14d", "r15d"]
GP16 = ["ax", "cx", "dx", "bx", "sp", "bp", "si", "di",
        "r8w", "r9w", "r10w", "r11w", "r12w", "r13w", "r14w", "r15w"]
GP8 = ["al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil",
       "r8b", "r9b", "r10b", "r11b", "r12b", "r13b", "r14b", "r15b"]
GP8L = ["al", "cl", "dl", "bl", "ah", "ch", "dh", "bh"]

CRIT_BASE = ["rax", "rcx", "rsp", "rbp", "rsi", "r12", "r13", "r8", "r15"]
CRIT_IDX = ["rax", "rbx", "rbp", "r12", "r13", "r15"]

DISPS = [0, 1, -1, 127, 128, -128, -129, 255, 256, 32767, 65535,
         2147483647, -2147483648]
IMM32 = [0, 1, -1, 127, 128, -128, -129, 255, 256, 65535,
         2147483647, -2147483648]
IMM64 = IMM32 + [4294967295, 4294967296, 9223372036854775807,
                 -9223372036854775808]


def mem(base=None, index=None, scale=1, disp=None, seg=None, rip=False):
    s = f"%{seg}:" if seg else ""
    if rip:
        return s + f"{'' if disp is None else disp}(%rip)"
    if disp is not None:
        s += str(disp)
    inner = ""
    if base:
        inner += f"%{base}"
    if index:
        inner += f",%{index},{scale}"
    if inner:
        s += f"({inner})"
    elif disp is None:
        s += "0"
    return s


class Emitter:
    def __init__(self):
        self.cases: list[tuple[str, str, str]] = []

    def add(self, name, body, flags=""):
        self.cases.append((name, body, flags))

    def block(self, name, lines, flags="", prologue=".text\n"):
        self.add(name, prologue + "\n".join(lines) + "\n", flags)


# ─── Generators ───────────────────────────────────────────────────────────

def gen_modrm(e: Emitter):
    for base in GP64:
        lines = [f"mov {mem(base=base, disp=d)}, %rax" for d in DISPS]
        lines += [f"mov %rax, {mem(base=base, disp=d)}" for d in DISPS]
        lines += [f"mov {mem(base=base)}, %rax"]
        e.block(f"base_{base}", lines)

    for base, idx in itertools.product(CRIT_BASE, CRIT_IDX):
        lines = []
        for sc in (1, 2, 4, 8):
            for d in (None, 0, 127, 128, -128, -129, 2147483647):
                lines.append(f"mov {mem(base=base, index=idx, scale=sc, disp=d)}, %rdx")
        e.block(f"sib_{base}_{idx}", lines)

    for idx in GP64:
        if idx == "rsp":
            continue
        lines = []
        for sc in (1, 2, 4, 8):
            for d in (0, 4, 127, 128, -1, 2147483647):
                lines.append(f"mov {mem(index=idx, scale=sc, disp=d)}, %rcx")
        e.block(f"nobase_{idx}", lines)

    lines = [f"mov {d}(%rip), %eax" for d in
             (0, 4, 127, 128, 65536, 2147483647, -2147483648)]
    lines += ["mov sym(%rip), %rax", "mov sym+4(%rip), %rax",
              "mov sym-8(%rip), %rax", "lea sym(%rip), %rax",
              "mov %rax, sym(%rip)", "movl $1, sym(%rip)",
              "movq $-1, sym(%rip)", "addl $7, sym(%rip)",
              "cmpq $0, sym(%rip)", "callq *sym(%rip)", "jmpq *sym(%rip)",
              "movb $1, sym(%rip)", "movw $1, sym(%rip)",
              "imull $70000, sym(%rip), %edx", "imull $3, sym(%rip), %edx"]
    e.block("riprel", lines, prologue=".text\n.extern sym\n")

    e.block("absolute", [
        "mov 0x1234, %eax", "mov 0x7fffffff, %eax", "movq 0x10, %rax",
        "movb 0x10, %al", "movabs 0x123456789abcdef0, %rax",
        "movabs %rax, 0x123456789abcdef0", "movabs 0x123456789abcdef0, %al",
        "movabs $0x123456789abcdef0, %rax", "movabs $0, %rax",
        "movabs $-1, %r15"])


def gen_rex(e: Emitter):
    e.block("gp8_new", [f"mov %{a}, %{b}" for a, b in itertools.product(GP8, GP8)])
    e.block("gp8_legacy", [f"mov %{a}, %{b}"
                           for a, b in itertools.product(GP8L, GP8L)])
    # An instruction naming both a high byte and a REX-requiring register is
    # unencodable; both assemblers must refuse it.
    for hi in ("ah", "ch", "dh", "bh"):
        for lo in ("spl", "r8b", "dil", "sil", "bpl", "r15b"):
            e.add(f"rexconflict_{hi}_{lo}", f".text\nmov %{hi}, %{lo}\n", "reject")

    for regs, sfx in ((GP8, "b"), (GP16, "w"), (GP32, "l"), (GP64, "q")):
        lines = [f"mov %{a}, %{b}" for a in regs for b in regs[:4]]
        lines += [f"mov{sfx} {mem(base='r13', disp=0)}, %{regs[0]}"]
        lines += [f"mov %{r}, {mem(base='r12', index='r13', scale=8, disp=100)}"
                  for r in regs]
        e.block(f"width_{sfx}", lines)

    lines = []
    for s8, d in itertools.product(GP8[:8] + GP8[8:12], GP32[:6] + GP64[:6]):
        lines.append(f"{'movzbl' if d in GP32 else 'movzbq'} %{s8}, %{d}")
        lines.append(f"{'movsbl' if d in GP32 else 'movsbq'} %{s8}, %{d}")
    for s16, d in itertools.product(GP16[:6] + GP16[8:11], GP32[:4] + GP64[:4]):
        lines.append(f"{'movzwl' if d in GP32 else 'movzwq'} %{s16}, %{d}")
        lines.append(f"{'movswl' if d in GP32 else 'movswq'} %{s16}, %{d}")
    for s32, d in itertools.product(GP32[:6] + GP32[8:11], GP64[:6]):
        lines.append(f"movslq %{s32}, %{d}")
    e.block("extend", lines)


def gen_imm(e: Emitter):
    for op in ["add", "or", "adc", "sbb", "and", "sub", "xor", "cmp"]:
        lines = []
        for v in IMM32:
            lines += [f"{op}l ${v}, %eax", f"{op}l ${v}, %ecx",
                      f"{op}l ${v}, {mem(base='rdx', disp=8)}"]
            if -2147483648 <= v <= 2147483647:
                lines += [f"{op}q ${v}, %rax", f"{op}q ${v}, %r11",
                          f"{op}q ${v}, {mem(base='r13')}"]
        for v in (0, 1, 127, 128, 255):
            lines += [f"{op}b ${v}, %al", f"{op}b ${v}, %bl", f"{op}b ${v}, %r10b"]
        for v in (0, 1, 127, 128, 32767, 32768, 65535):
            lines += [f"{op}w ${v}, %ax", f"{op}w ${v}, %dx",
                      f"{op}w ${v}, {mem(base='rdi')}"]
        e.block(f"alu_{op}", lines)

    lines = []
    for v in IMM32:
        lines += [f"testl ${v}, %eax", f"testl ${v}, %ebx",
                  f"movl ${v & 0xFFFFFFFF}, %eax",
                  f"movl ${v & 0xFFFFFFFF}, %r14d",
                  f"movl ${v & 0xFFFFFFFF}, {mem(base='rsp', disp=16)}"]
        if -2147483648 <= v <= 2147483647:
            lines += [f"movq ${v}, %rax",
                      f"movq ${v}, {mem(base='rbp', disp=-8)}",
                      f"testq ${v}, %rdx"]
    for v in IMM64:
        lines.append(f"movabsq ${v}, %rdi")
    e.block("mov_test", lines)

    lines = []
    for v in IMM32:
        lines += [f"imull ${v}, %eax, %ecx",
                  f"imull ${v}, {mem(base='rsi', disp=4)}, %edx"]
        if -2147483648 <= v <= 2147483647:
            lines.append(f"imulq ${v}, %rbx, %r9")
        if -32768 <= v <= 65535:
            lines += [f"imulw ${v}, %ax, %cx",
                      f"imulw ${v}, {mem(base='rsi')}, %dx"]
    e.block("imul", lines)

    e.block("push", [f"push ${v}" for v in
                     (0, 1, -1, 127, 128, -128, -129, 65535,
                      2147483647, -2147483648)])


def gen_shift(e: Emitter):
    lines = []
    for op in ("rol", "ror", "rcl", "rcr", "shl", "shr", "sar"):
        for sfx, reg in (("b", "bl"), ("w", "bx"), ("l", "ebx"), ("q", "rbx")):
            lines += [f"{op}{sfx} %{reg}", f"{op}{sfx} $1, %{reg}"]
            lines += [f"{op}{sfx} ${v}, %{reg}" for v in (0, 2, 7, 15, 31, 63, 255)]
            lines += [f"{op}{sfx} %cl, %{reg}",
                      f"{op}{sfx} %cl, {mem(base='r12', index='rax', scale=4, disp=8)}"]
    for v in (0, 1, 31, 63):
        lines += [f"shldl ${v}, %eax, %ecx", f"shrdq ${v}, %rax, %rcx"]
    lines += ["shldl %cl, %eax, %ecx", "shrdq %cl, %rax, %r15"]
    e.block("shifts", lines)


def gen_prefix(e: Emitter):
    lines = []
    for seg in ("fs", "gs", "cs", "ss", "ds", "es"):
        lines += [f"mov {mem(base='rax', disp=8, seg=seg)}, %rbx",
                  f"movl $1, {mem(base='rdi', seg=seg)}",
                  f"mov {mem(disp=0, seg=seg)}, %rax",
                  f"mov {mem(disp=16, seg=seg)}, %rax",
                  f"movw $1, {mem(base='rdi', seg=seg)}",
                  f"mov {mem(base='r13', index='r12', scale=4, disp=8, seg=seg)}, %rax"]
    e.block("segments", lines)

    lines = []
    for op in ("add", "or", "adc", "sbb", "and", "sub", "xor"):
        for sfx in ("b", "w", "l", "q"):
            r = {"b": "al", "w": "ax", "l": "eax", "q": "rax"}[sfx]
            lines += [f"lock {op}{sfx} %{r}, {mem(base='rdi')}",
                      f"lock {op}{sfx} $1, {mem(base='rdi')}"]
    for sfx, r in (("b", "al"), ("w", "ax"), ("l", "eax"), ("q", "rax")):
        lines += [f"lock xadd{sfx} %{r}, {mem(base='rsi')}",
                  f"lock cmpxchg{sfx} %{r}, {mem(base='rsi')}",
                  f"lock inc{sfx} {mem(base='rsi')}",
                  f"lock dec{sfx} {mem(base='rsi')}",
                  f"lock neg{sfx} {mem(base='rsi')}",
                  f"lock not{sfx} {mem(base='rsi')}",
                  f"xchg %{r}, {mem(base='rsi')}"]
    lines += ["lock cmpxchg16b (%rdi)", "lock cmpxchg8b (%rdi)"]
    e.block("lock", lines)

    lines = []
    for rep in ("rep", "repe", "repne"):
        for ins in ("movsb", "movsw", "movsl", "movsq", "stosb", "stosw",
                    "stosl", "stosq", "lodsb", "lodsw", "lodsl", "lodsq",
                    "scasb", "scasw", "scasl", "scasq", "cmpsb", "cmpsw"):
            lines.append(f"{rep} {ins}")
    e.block("string", lines)


def gen_branch(e: Emitter):
    ccs = ["o", "no", "b", "ae", "e", "ne", "be", "a", "s", "ns",
           "p", "np", "l", "ge", "le", "g"]

    # Backward and forward jumps straddling the disp8 boundary exactly.
    for pad in (124, 125, 126, 127, 128, 129, 130, 200):
        for cc in ("e", "ne", "l"):
            e.block(f"back_{pad}_{cc}",
                    [".p2align 4", "0:"] + ["nop"] * pad + [f"j{cc} 0b"])
        e.block(f"back_{pad}_jmp",
                [".p2align 4", "0:"] + ["nop"] * pad + ["jmp 0b"])
    for pad in (124, 125, 126, 127, 128, 129, 300):
        for cc in ("e", "ne", "g"):
            e.block(f"fwd_{pad}_{cc}",
                    [f"j{cc} 1f"] + ["nop"] * pad + ["1:", "ret"])
        e.block(f"fwd_{pad}_jmp", ["jmp 1f"] + ["nop"] * pad + ["1:", "ret"])

    # Mutually dependent jumps: relaxing one changes another's distance. A
    # one-directional relaxer diverges from GNU as here.
    for n in (2, 3, 5, 8, 12):
        body = []
        for i in range(n):
            body.append(f"je {i+1}f")
            body += ["nop"] * 40
        for i in range(n):
            body.append(f"{i+1}:")
            body += ["nop"] * 20
        body.append("ret")
        e.block(f"chain_{n}", body)

    # Relaxation interacting with alignment padding: shrinking a jump changes
    # the padding, which can push a target back out of short range.
    for pad in (100, 118, 120, 122, 124, 126, 128):
        for al in (4, 8, 16, 32):
            e.block(f"align_{pad}_{al}",
                    ["jmp 1f"] + ["nop"] * pad + [f".p2align {al}", "1:", "ret"])
            e.block(f"alignback_{pad}_{al}",
                    ["0:"] + ["nop"] * pad + [f".p2align {al}"] +
                    ["nop"] * 4 + ["jne 0b"])

    lines = [f"j{cc} target" for cc in ccs]
    lines += ["jmp target", "call target", "jmp *%rax", "call *%rax",
              "jmp *(%rax)", "call *(%rax)", "jmp *(%r13,%rax,8)",
              "call *(%r13,%rax,8)", "callq *%r11", "jrcxz 1f", "1: ret"]
    e.block("targets", lines, prologue=".text\n.extern target\n")

    lines = []
    for cc in ccs:
        lines += [f"set{cc} %{r}" for r in GP8[:8] + ["r8b", "r15b", "spl", "sil"]]
        lines.append(f"set{cc} {mem(base='r12', index='rcx', scale=1)}")
        for dst in ("eax", "r9d", "rbx", "r14"):
            src = mem(base="rdi", disp=8) if dst in ("eax", "rbx") else f"%{dst}"
            lines.append(f"cmov{cc} {src}, %{dst}")
    e.block("setcc_cmovcc", lines)


def gen_padding(e: Emitter):
    """Alignment padding shape for every gap size and alignment.

    Executable padding must be multi-byte NOPs, not repeated 0x90: a 15-byte
    gap is 15 decoded uops in the naive form and 2 in the canonical one, and
    PGO block alignment puts that padding on the hottest paths in the program.
    """
    for ap, al in ((2, 4), (3, 8), (4, 16), (5, 32), (6, 64), (7, 128)):
        for n in range(1, min(al, 68)):
            # After real instructions.
            e.block(f"pad_insn_{al}_{n}",
                    ["nop"] * (al - n) + [f".p2align {ap}", "ret"])
            # After raw data: GNU as leads with a plain 0x90 in this case.
            e.block(f"pad_data_{al}_{n}",
                    [f".skip {al - n}, 0xCC", f".p2align {ap}", ".byte 0xC3"])
    for al, ap in ((16, 4), (32, 5)):
        e.block(f"pad_fill_{al}", ["nop", f".p2align {ap}, 0x90", "ret"])
    e.block("pad_org", ["nop", ".org . + 16", "nop", ".p2align 4", "ret"])


def gen_x87(e: Emitter):
    lines = []
    for i in range(8):
        for op in ("fadd", "fsub", "fsubr", "fmul", "fdiv", "fdivr",
                   "fcom", "fcomp", "fucomi", "fcomi", "fxch", "ffree"):
            lines.append(f"{op} %st({i})")
    for op in ("fadds", "faddl", "fmuls", "fmull", "fdivs", "fdivl", "flds",
               "fldl", "fldt", "fsts", "fstl", "fstps", "fstpl", "fstpt",
               "filds", "fildl", "fildll", "fists", "fistl", "fistps",
               "fistpl", "fistpll", "fisttps", "fisttpl", "fisttpll"):
        lines.append(f"{op} {mem(base='rsp', disp=8)}")
    lines += ["fld1", "fldz", "fldpi", "fldl2e", "fldl2t", "fldlg2", "fldln2",
              "fabs", "fchs", "fsqrt", "fsin", "fcos", "fptan", "fpatan",
              "f2xm1", "fyl2x", "fyl2xp1", "fprem", "fprem1", "frndint",
              "fscale", "fxtract", "fnop", "fwait", "fnclex", "fninit",
              "fnstsw %ax", "fnstcw (%rsp)", "fldcw (%rsp)", "fincstp",
              "fdecstp", "ftst", "fxam", "fcompp", "fucompp"]
    e.block("x87", lines)


def gen_sse(e: Emitter):
    groups = {
        "pd": ["addpd", "subpd", "mulpd", "divpd", "minpd", "maxpd", "andpd",
               "andnpd", "orpd", "xorpd", "unpcklpd", "unpckhpd", "sqrtpd"],
        "ps": ["addps", "subps", "mulps", "divps", "minps", "maxps", "andps",
               "andnps", "orps", "xorps", "unpcklps", "unpckhps", "sqrtps",
               "rcpps", "rsqrtps"],
        "sd": ["addsd", "subsd", "mulsd", "divsd", "minsd", "maxsd", "sqrtsd"],
        "ss": ["addss", "subss", "mulss", "divss", "minss", "maxss", "sqrtss",
               "rcpss", "rsqrtss"],
        "int": ["paddb", "paddw", "paddd", "paddq", "psubb", "psubw", "psubd",
                "psubq", "pand", "pandn", "por", "pxor", "pmullw", "pmulhw",
                "pmulhuw", "pmuludq", "pavgb", "pavgw", "pmaxub", "pminub",
                "pmaxsw", "pminsw", "psadbw", "packsswb", "packssdw",
                "packuswb", "punpcklbw", "punpckhbw", "punpckldq",
                "punpcklqdq", "punpckhqdq", "pcmpeqb", "pcmpeqw", "pcmpeqd",
                "pcmpgtb", "pcmpgtw", "pcmpgtd", "pmaddwd"],
    }
    for tag, ops in groups.items():
        lines = []
        for op in ops:
            for a, b in ((0, 1), (7, 8), (8, 15), (15, 0), (3, 12)):
                lines.append(f"{op} %xmm{a}, %xmm{b}")
            lines += [f"{op} {mem(base='rdi', disp=16)}, %xmm5",
                      f"{op} {mem(base='r13', index='r14', scale=8)}, %xmm13"]
        e.block(f"sse_{tag}", lines)

    lines = []
    for op in ("movaps", "movapd", "movups", "movupd", "movdqa", "movdqu",
               "movntps", "movntpd", "movntdq"):
        lines += [f"{op} %xmm1, %xmm9", f"{op} {mem(base='rsp', disp=32)}, %xmm2"]
        if not op.startswith("movnt"):
            lines.append(f"{op} %xmm10, {mem(base='rbp', disp=-64)}")
    lines += [
        "movd %eax, %xmm0", "movd %xmm0, %eax", "movd %r13d, %xmm9",
        "movd %xmm9, %r13d", "movq %rax, %xmm0", "movq %xmm0, %rax",
        "movq %r12, %xmm11", "movq %xmm11, %r12", "movq %xmm1, %xmm2",
        "movq (%rdi), %xmm3", "movq %xmm3, (%rdi)", "movss %xmm1, %xmm2",
        "movss (%rdi), %xmm3", "movss %xmm3, (%rdi)", "movsd %xmm1, %xmm2",
        "movsd (%rdi), %xmm3", "movsd %xmm3, (%rdi)", "movhps (%rdi), %xmm0",
        "movhps %xmm0, (%rdi)", "movlps (%rdi), %xmm0", "movlps %xmm0, (%rdi)",
        "movhpd (%rdi), %xmm0", "movlpd (%rdi), %xmm0", "movhlps %xmm1, %xmm2",
        "movlhps %xmm1, %xmm2", "movmskps %xmm3, %eax", "movmskpd %xmm3, %r8d",
        "pmovmskb %xmm3, %eax", "movddup %xmm1, %xmm2",
        "movshdup %xmm1, %xmm2", "movsldup %xmm1, %xmm2",
        "movntdqa (%rdi), %xmm0"]
    e.block("sse_moves", lines)

    lines = []
    for op in ("psllw", "pslld", "psllq", "psrlw", "psrld", "psrlq",
               "psraw", "psrad"):
        for v in (0, 1, 7, 15, 31, 63, 127, 255):
            lines += [f"{op} ${v}, %xmm4", f"{op} ${v}, %xmm12"]
        lines += [f"{op} %xmm1, %xmm2", f"{op} (%rdi), %xmm2"]
    for op in ("pslldq", "psrldq"):
        for v in (0, 1, 8, 15, 16, 255):
            lines += [f"{op} ${v}, %xmm4", f"{op} ${v}, %xmm12"]
    e.block("sse_shifts", lines)

    lines = []
    for v in (0, 1, 0x1B, 0x4E, 0xE4, 0xFF):
        for op in ("pshufd", "pshufhw", "pshuflw"):
            lines += [f"{op} ${v}, %xmm1, %xmm2", f"{op} ${v}, (%rdi), %xmm10"]
        lines += [f"shufps ${v}, %xmm1, %xmm2", f"shufpd ${v & 3}, %xmm1, %xmm2"]
    for v in range(8):
        for op in ("cmpps", "cmppd", "cmpss", "cmpsd"):
            lines.append(f"{op} ${v}, %xmm1, %xmm2")
    for v in (0, 1, 3, 7):
        lines += [f"pinsrw ${v}, %eax, %xmm1", f"pinsrw ${v}, (%rdi), %xmm1",
                  f"pextrw ${v}, %xmm1, %eax"]
    e.block("sse_imm", lines)

    e.block("sse_cvt", [
        "cvtsi2sd %eax, %xmm0", "cvtsi2sd %rax, %xmm0",
        "cvtsi2sdl (%rdi), %xmm0", "cvtsi2sdq (%rdi), %xmm0",
        "cvtsi2ss %eax, %xmm0", "cvtsi2ss %rax, %xmm0",
        "cvtsd2si %xmm0, %eax", "cvtsd2si %xmm0, %rax",
        "cvttsd2si %xmm0, %eax", "cvttsd2si %xmm0, %rax",
        "cvtss2si %xmm0, %eax", "cvttss2si %xmm0, %rax",
        "cvtsd2ss %xmm1, %xmm2", "cvtss2sd %xmm1, %xmm2",
        "cvtdq2ps %xmm1, %xmm2", "cvtps2dq %xmm1, %xmm2",
        "cvttps2dq %xmm1, %xmm2", "cvtdq2pd %xmm1, %xmm2",
        "cvtpd2dq %xmm1, %xmm2", "cvttpd2dq %xmm1, %xmm2",
        "cvtps2pd %xmm1, %xmm2", "cvtpd2ps %xmm1, %xmm2",
        "comiss %xmm1, %xmm2", "comisd %xmm1, %xmm2",
        "ucomiss %xmm1, %xmm2", "ucomisd %xmm1, %xmm2",
        "comiss (%rdi), %xmm12", "ucomisd (%r13), %xmm12"])

    lines = []
    for op in ("pabsb", "pabsw", "pabsd", "phaddw", "phaddd", "phsubw",
               "phsubd", "psignb", "psignw", "psignd", "pmulhrsw",
               "pmaddubsw", "pshufb", "pmuldq", "pmulld", "pminsb", "pminsd",
               "pminuw", "pminud", "pmaxsb", "pmaxsd", "pmaxuw", "pmaxud",
               "packusdw", "pcmpeqq", "pcmpgtq", "ptest", "phminposuw",
               "pmovsxbw", "pmovsxbd", "pmovsxbq", "pmovsxwd", "pmovsxwq",
               "pmovsxdq", "pmovzxbw", "pmovzxbd", "pmovzxbq", "pmovzxwd",
               "pmovzxwq", "pmovzxdq"):
        lines += [f"{op} %xmm1, %xmm2", f"{op} %xmm9, %xmm14",
                  f"{op} (%rdi), %xmm3"]
    for v in (0, 1, 5, 15, 0xFF):
        lines += [f"palignr ${v}, %xmm1, %xmm2",
                  f"blendps ${v & 15}, %xmm1, %xmm2",
                  f"blendpd ${v & 3}, %xmm1, %xmm2",
                  f"pblendw ${v}, %xmm1, %xmm2", f"dpps ${v}, %xmm1, %xmm2",
                  f"dppd ${v}, %xmm1, %xmm2", f"mpsadbw ${v & 7}, %xmm1, %xmm2",
                  f"roundps ${v & 15}, %xmm1, %xmm2",
                  f"roundsd ${v & 15}, %xmm1, %xmm2"]
    for v in (0, 1, 3):
        lines += [f"insertps ${v}, %xmm1, %xmm2", f"extractps ${v}, %xmm1, %eax",
                  f"pinsrd ${v}, %eax, %xmm1", f"pextrd ${v}, %xmm1, %eax",
                  f"pinsrq ${v & 1}, %rax, %xmm1",
                  f"pextrq ${v & 1}, %xmm1, %rax",
                  f"pinsrb ${v}, %eax, %xmm1", f"pextrb ${v}, %xmm1, %eax"]
    lines += ["blendvps %xmm0, %xmm1, %xmm2", "blendvpd %xmm0, %xmm1, %xmm2",
              "pblendvb %xmm0, %xmm1, %xmm2", "crc32b %al, %eax",
              "crc32w %ax, %eax", "crc32l %eax, %eax", "crc32q %rax, %rax",
              "crc32b (%rdi), %eax", "crc32q (%rdi), %rax",
              "popcntl %eax, %ecx", "popcntq %rax, %rcx", "popcntw %ax, %cx"]
    e.block("sse4", lines)

    lines = []
    for op in ("aesenc", "aesenclast", "aesdec", "aesdeclast", "aesimc"):
        lines += [f"{op} %xmm1, %xmm2", f"{op} (%rdi), %xmm11"]
    for v in (0, 1, 0x10, 0x11, 0xFF):
        lines += [f"aeskeygenassist ${v}, %xmm1, %xmm2",
                  f"pclmulqdq ${v & 0x11}, %xmm1, %xmm2"]
    e.block("aes", lines)


def gen_avx(e: Emitter):
    lines = []
    for op in ["vaddps", "vaddpd", "vsubps", "vsubpd", "vmulps", "vmulpd",
               "vdivps", "vdivpd", "vminps", "vmaxpd", "vandps", "vandnpd",
               "vorps", "vxorpd", "vunpcklps", "vunpckhpd"]:
        for a, b, c in ((0, 1, 2), (7, 8, 9), (8, 0, 1), (15, 15, 15),
                        (1, 2, 12), (12, 1, 2)):
            lines += [f"{op} %xmm{a}, %xmm{b}, %xmm{c}",
                      f"{op} %ymm{a}, %ymm{b}, %ymm{c}"]
        lines += [f"{op} {mem(base='rdi', disp=32)}, %xmm1, %xmm2",
                  f"{op} {mem(base='r13', disp=32)}, %ymm1, %ymm2",
                  f"{op} {mem(base='rax', index='r12', scale=8)}, %ymm1, %ymm2"]
    e.block("avx_arith", lines)

    lines = []
    for op in ("vpaddb", "vpaddw", "vpaddd", "vpaddq", "vpsubb", "vpsubd",
               "vpand", "vpandn", "vpor", "vpxor", "vpmulld", "vpmullw",
               "vpcmpeqb", "vpcmpeqd", "vpcmpgtb", "vpcmpgtd", "vpmaxsd",
               "vpminud", "vpackusdw", "vpshufb", "vpunpcklbw",
               "vpunpckhqdq", "vpavgb", "vpsadbw", "vpmaddwd"):
        for w in ("xmm", "ymm"):
            lines += [f"{op} %{w}1, %{w}2, %{w}3", f"{op} %{w}9, %{w}10, %{w}11",
                      f"{op} (%rdi), %{w}1, %{w}2",
                      f"{op} (%r13,%rax,4), %{w}1, %{w}14"]
    e.block("avx_int", lines)

    lines = []
    for op in ("vmovaps", "vmovapd", "vmovups", "vmovupd", "vmovdqa",
               "vmovdqu", "vmovntps", "vmovntdq"):
        for w in ("xmm", "ymm"):
            lines += [f"{op} %{w}1, %{w}2", f"{op} %{w}9, %{w}0",
                      f"{op} (%rdi), %{w}3", f"{op} %{w}3, (%rdi)",
                      f"{op} 128(%r12,%rbx,2), %{w}15"]
    lines += ["vmovd %eax, %xmm0", "vmovd %xmm0, %eax", "vmovq %rax, %xmm0",
              "vmovq %xmm0, %rax", "vmovq %xmm1, %xmm2", "vmovq (%rdi), %xmm3",
              "vmovss %xmm1, %xmm2, %xmm3", "vmovsd %xmm1, %xmm2, %xmm3",
              "vmovss (%rdi), %xmm3", "vmovsd (%rdi), %xmm3",
              "vmovhlps %xmm1, %xmm2, %xmm3", "vmovlhps %xmm1, %xmm2, %xmm3",
              "vmovmskps %ymm3, %eax", "vpmovmskb %ymm3, %eax",
              "vmovddup %ymm1, %ymm2", "vmovshdup %ymm1, %ymm2",
              "vzeroupper", "vzeroall", "vlddqu (%rdi), %ymm0",
              "vmovntdqa (%rdi), %ymm0"]
    e.block("avx_moves", lines)

    lines = []
    for v in (0, 1, 0x1B, 0x4E, 0xFF):
        lines += [f"vpshufd ${v}, %ymm1, %ymm2",
                  f"vshufps ${v}, %ymm1, %ymm2, %ymm3",
                  f"vpalignr ${v}, %ymm1, %ymm2, %ymm3",
                  f"vpblendw ${v}, %ymm1, %ymm2, %ymm3",
                  f"vblendps ${v}, %ymm1, %ymm2, %ymm3",
                  f"vperm2i128 ${v}, %ymm1, %ymm2, %ymm3",
                  f"vpermq ${v}, %ymm1, %ymm2",
                  f"vpblendd ${v}, %ymm1, %ymm2, %ymm3"]
    for v in (0, 1):
        lines += [f"vextractf128 ${v}, %ymm1, %xmm2",
                  f"vextracti128 ${v}, %ymm1, %xmm2",
                  f"vextractf128 ${v}, %ymm9, (%rdi)",
                  f"vinsertf128 ${v}, %xmm1, %ymm2, %ymm3",
                  f"vinserti128 ${v}, (%rdi), %ymm2, %ymm3"]
    for v in range(0, 32, 5):
        lines += [f"vcmpps ${v}, %ymm1, %ymm2, %ymm3",
                  f"vcmpsd ${v}, %xmm1, %xmm2, %xmm3"]
    e.block("avx_imm", lines)

    lines = []
    for op in ("vbroadcastss", "vbroadcastsd", "vpbroadcastb", "vpbroadcastw",
               "vpbroadcastd", "vpbroadcastq"):
        lines += [f"{op} (%rdi), %ymm1", f"{op} %xmm1, %ymm2"]
    lines += ["vbroadcasti128 (%rdi), %ymm0", "vbroadcastf128 (%rdi), %ymm0",
              "vbroadcastf128 (%r13), %ymm9",
              "vpermd %ymm1, %ymm2, %ymm3", "vpermps %ymm1, %ymm2, %ymm3",
              "vpermilps %xmm1, %xmm2, %xmm3",
              "vpermilps $0x1b, %ymm1, %ymm2",
              "vpsllvd %ymm1, %ymm2, %ymm3", "vpsrlvq %ymm1, %ymm2, %ymm3",
              "vpsravd %ymm1, %ymm2, %ymm3",
              "vmaskmovps (%rdi), %ymm1, %ymm2",
              "vmaskmovps %ymm2, %ymm1, (%rdi)",
              "vgatherdps %ymm2, (%rdi,%ymm1,4), %ymm3",
              "vpgatherdd %ymm2, (%r13,%ymm1,4), %ymm3"]
    for op in ("vpslld", "vpsrld", "vpsraw", "vpsllq", "vpsrlq"):
        lines += [f"{op} ${v}, %ymm1, %ymm2" for v in (0, 1, 7, 31, 63)]
        lines.append(f"{op} %xmm1, %ymm2, %ymm3")
    e.block("avx2", lines)

    lines = []
    for kind in ("132", "213", "231"):
        for base in ("vfmadd", "vfmsub", "vfnmadd", "vfnmsub"):
            for t in ("ps", "pd"):
                for w in ("xmm", "ymm"):
                    lines += [f"{base}{kind}{t} %{w}1, %{w}2, %{w}3",
                              f"{base}{kind}{t} %{w}9, %{w}10, %{w}11",
                              f"{base}{kind}{t} (%rdi), %{w}1, %{w}2"]
            for t in ("ss", "sd"):
                lines += [f"{base}{kind}{t} %xmm1, %xmm2, %xmm3",
                          f"{base}{kind}{t} (%r13), %xmm1, %xmm2"]
        for base in ("vfmaddsub", "vfmsubadd"):
            for t in ("ps", "pd"):
                lines.append(f"{base}{kind}{t} %ymm1, %ymm2, %ymm3")
    e.block("fma", lines)

    e.block("avx_cvt", [
        "vcvtsi2sdl (%rdi), %xmm1, %xmm2", "vcvtsi2sdq (%rdi), %xmm1, %xmm2",
        "vcvtsd2si %xmm0, %eax", "vcvtsd2si %xmm0, %rax",
        "vcvttsd2si %xmm0, %eax", "vcvttss2si %xmm0, %rax",
        "vcvtsd2ss %xmm1, %xmm2, %xmm3", "vcvtss2sd %xmm1, %xmm2, %xmm3",
        "vcvtdq2ps %ymm1, %ymm2", "vcvtps2dq %ymm1, %ymm2",
        "vcvttps2dq %ymm1, %ymm2", "vcvtdq2pd %xmm1, %ymm2",
        "vcvtpd2dq %ymm1, %xmm2", "vcvtps2pd %xmm1, %ymm2",
        "vcomiss %xmm1, %xmm2", "vucomisd (%r13), %xmm12",
        "vroundps $3, %ymm1, %ymm2", "vroundsd $1, %xmm1, %xmm2, %xmm3",
        "vsqrtps %ymm1, %ymm2", "vrsqrtps %ymm1, %ymm2",
        "vdpps $0xff, %ymm1, %ymm2, %ymm3", "vtestps %ymm1, %ymm2",
        "vptest %ymm1, %ymm2", "vpmovsxbw %xmm1, %ymm2",
        "vpmovzxdq (%rdi), %ymm2", "vphaddw %ymm1, %ymm2, %ymm3",
        "vpabsd %ymm1, %ymm2", "vpblendvb %ymm4, %ymm1, %ymm2, %ymm3",
        "vblendvps %ymm4, %ymm1, %ymm2, %ymm3", "vaesenc %xmm1, %xmm2, %xmm3",
        "vpclmulqdq $0x11, %xmm1, %xmm2, %xmm3", "vldmxcsr (%rsp)",
        "vstmxcsr (%rsp)", "vmovntps %ymm3, (%rdi)", "vmovntpd %ymm3, (%r12)",
        "vmovntdq %xmm13, (%r13,%rax,8)"])


def gen_bmi(e: Emitter):
    lines = []
    for op in ("andn", "bextr", "bzhi", "mulx", "pdep", "pext", "sarx",
               "shlx", "shrx"):
        for sfx in ("l", "q"):
            a, b, c = ("eax", "ecx", "edx") if sfx == "l" else ("rax", "rcx", "rdx")
            x, y, z = ("r8d", "r9d", "r10d") if sfx == "l" else ("r8", "r9", "r10")
            lines += [f"{op}{sfx} %{a}, %{b}, %{c}",
                      f"{op}{sfx} %{x}, %{y}, %{z}",
                      f"{op}{sfx} (%rdi), %{b}, %{c}"]
    for v in (0, 1, 7, 31, 63, 255):
        lines += [f"rorxl ${v}, %eax, %ecx", f"rorxq ${v}, %rax, %rcx",
                  f"rorxq ${v}, (%rdi), %r15"]
    for op in ("blsi", "blsmsk", "blsr"):
        for sfx, r in (("l", "eax"), ("q", "rax")):
            lines += [f"{op}{sfx} %{r}, %{r}", f"{op}{sfx} (%rdi), %{r}"]
    for op in ("tzcnt", "lzcnt", "popcnt", "bsf", "bsr"):
        for sfx, a, b in (("w", "ax", "cx"), ("l", "eax", "ecx"),
                          ("q", "rax", "rcx")):
            lines.append(f"{op}{sfx} %{a}, %{b}")
    for op in ("bt", "btc", "btr", "bts"):
        for sfx, r in (("w", "ax"), ("l", "eax"), ("q", "rax")):
            lines.append(f"{op}{sfx} %{r}, %{r}")
            for v in (0, 1, 7, 15, 31, 63):
                lines += [f"{op}{sfx} ${v}, %{r}", f"{op}{sfx} ${v}, (%rdi)"]
            lines.append(f"{op}{sfx} %{r}, (%rdi)")
    lines += ["bswapl %eax", "bswapq %rax", "bswapl %r13d", "bswapq %r13",
              "adcxl %eax, %ecx", "adcxq %rax, %rcx", "adoxl %eax, %ecx",
              "movbew (%rdi), %ax", "movbel (%rdi), %eax",
              "movbeq (%rdi), %rax", "movbeq %rax, (%rdi)"]
    e.block("bmi", lines)


def gen_lea(e: Emitter):
    lines = []
    for base in ("rax", "rsp", "rbp", "r12", "r13", None):
        for idx in ("rcx", "rbx", "r12", "r13", None):
            for sc in (1, 2, 4, 8):
                for d in (None, 0, 1, 127, 128, -128, -129, 65536):
                    if base is None and idx is None:
                        continue
                    if idx is None and sc != 1:
                        continue
                    o = mem(base=base, index=idx, scale=sc, disp=d)
                    lines += [f"leaq {o}, %rdx", f"leal {o}, %edx"]
    e.block("lea", lines)


def gen_misc(e: Emitter):
    e.block("misc", [
        "nop", "nopw (%rax)", "nopl (%rax)", "nopl 0(%rax)",
        "nopw 0(%rax,%rax,1)", "nopl 0(%rax,%rax,1)",
        "nopw %cs:0(%rax,%rax,1)",
        "xchg %ax, %ax", "xchg %eax, %eax", "xchg %rax, %rax",
        "xchg %eax, %ecx", "xchg %rax, %r12", "xchg %r8, %rax",
        "xchg %cl, %dl", "cbw", "cwde", "cdqe", "cwd", "cdq", "cqo",
        "cltq", "cqto", "cltd", "leave", "ret", "retq", "ret $16",
        "int3", "int $0x80", "iretq", "ud2", "hlt", "pause", "cpuid",
        "rdtsc", "rdtscp", "rdpmc", "clc", "stc", "cmc", "cld", "std",
        "cli", "sti", "lahf", "sahf", "pushfq", "popfq", "mfence",
        "lfence", "sfence", "clflush (%rdi)", "clflushopt (%rdi)",
        "clwb (%rdi)", "prefetcht0 (%rdi)", "prefetcht1 (%rdi)",
        "prefetcht2 (%rdi)", "prefetchnta (%rdi)", "prefetchw (%rdi)",
        "endbr64", "endbr32", "syscall", "sysret", "swapgs", "xgetbv",
        "rdrand %eax", "rdrand %rax", "rdseed %r15", "wbinvd",
        "xsave (%rdi)", "xrstor (%rdi)", "fxsave (%rdi)", "fxrstor (%rdi)",
        "stmxcsr (%rsp)", "ldmxcsr (%rsp)"])

    lines = [f"push %{r}" for r in GP64] + [f"pop %{r}" for r in GP64]
    lines += [f"push %{r}" for r in GP16] + [f"pop %{r}" for r in GP16]
    lines += ["push (%rdi)", "pop (%rdi)", "pushq 8(%r13)", "popq 8(%r13)",
              "pushq $0", "pushq $127", "pushq $128", "pushq $-1"]
    e.block("pushpop", lines)

    lines = []
    for v in (0, 1, 0x80, 0xFF):
        for sfx, r in (("b", "al"), ("w", "ax"), ("l", "eax")):
            lines += [f"in{sfx} ${v}, %{r}", f"out{sfx} %{r}, ${v}"]
    for sfx, r in (("b", "al"), ("w", "ax"), ("l", "eax")):
        lines += [f"in{sfx} %dx, %{r}", f"out{sfx} %{r}, %dx"]
    e.block("inout", lines)

    lines = []
    for op in ("mul", "imul", "div", "idiv", "neg", "not", "inc", "dec"):
        for sfx, regs in (("b", GP8), ("w", GP16), ("l", GP32), ("q", GP64)):
            lines += [f"{op}{sfx} %{r}" for r in regs[:6] + regs[8:11]]
            lines.append(
                f"{op}{sfx} {mem(base='r13', index='r12', scale=2, disp=8)}")
    for sfx, a, b in (("w", "ax", "cx"), ("l", "eax", "ecx"),
                      ("q", "rax", "rcx")):
        lines += [f"imul{sfx} %{a}, %{b}", f"imul{sfx} (%rdi), %{b}"]
    e.block("muldiv", lines)


def gen_directives(e: Emitter):
    e.add("data", """\
.data
.globl gd
gd:     .byte 1, 2, 3, 0xff, -1, 127, -128
        .word 1, 0xffff, -1, 32767, -32768
        .long 1, 0xffffffff, -1, 2147483647, -2147483648
        .quad 1, 0xffffffffffffffff, -1
        .ascii "abc"
        .asciz "def"
        .string "ghi"
        .space 8
        .space 4, 0xAA
        .zero 5
        .byte 0
.align 8
        .long 42
.p2align 4
        .long 43
.balign 32
        .long 45
""")

    e.add("floats", """\
.section .rodata
.globl fp
fp:     .float 1.0, -0.0, 3.14159, 1e30, -1e-30
        .double 1.0, -0.0, 3.141592653589793, 1e300, -1e-300
        .single 0.5
        .quad 0x7ff0000000000000
        .long 0x7f800000
""")

    e.add("symbols", """\
.text
.globl g1
.type g1, @function
g1:     ret
.size g1, .-g1
.weak w1
w1:     ret
.globl alias1
.set alias1, g1
.globl alias2
alias2 = g1
.hidden h1
.globl h1
h1:     ret
.protected p1
.globl p1
p1:     ret
.comm c1, 8, 8
.comm c2, 16, 16
""")

    e.add("sections", """\
.section .text.hot,"ax",@progbits
hotf:   ret
.section .text.unlikely,"ax",@progbits
coldf:  ret
.section .rodata.cst16,"aM",@progbits,16
        .quad 1, 2
.section .data.rel.ro,"aw",@progbits
        .quad 0
.section .bss,"aw",@nobits
        .zero 64
.section .tdata,"awT",@progbits
        .long 7
.section .init_array,"aw",@init_array
        .quad hotf
.text
        ret
""", "nosym")

    # Symbol arithmetic: a wrong addend here is a silent data corruption.
    e.add("symdiff", """\
.text
a:      nop
        nop
b:      nop
c:
.data
        .byte b-a
        .word b-a
        .long b-a
        .quad b-a
        .long c-a
        .long a-b
        .long (c-a)*2
        .long (c-a)+7
        .long 2*(c-a)
.text
        .long b-a
""", "nosym")

    e.add("equ_forward", """\
.text
.set  sz, end - beg
beg:    nop
        nop
        nop
end:    ret
.data
        .long sz
        .byte sz
""", "nosym")

    e.add("rept_irp", """\
.text
.rept 4
        nop
.endr
.irp reg, rax, rbx, rcx, rdx
        push %\\reg
.endr
.irp reg, rdx, rcx, rbx, rax
        pop %\\reg
.endr
        ret
""")

    e.add("macro", """\
.macro tri a, b, c
        add %\\a, %\\b
        add %\\b, %\\c
.endm
.text
        tri rax, rbx, rcx
        tri r8, r9, r10
.macro withdef x=7
        .byte \\x
.endm
        withdef
        withdef 9
        ret
""")

    e.add("conditionals", """\
.text
.set  v, 3
.if v == 3
        nop
.else
        ud2
.endif
.ifdef v
        nop
.endif
.ifndef nothere
        nop
.endif
.if v > 1 && v < 10
        nop
.endif
        ret
""")

    e.add("relocs", """\
.text
.extern ext
.globl f
f:
        call ext
        call ext@PLT
        jmp  ext
        mov  ext(%rip), %rax
        mov  ext@GOTPCREL(%rip), %rax
        lea  ext(%rip), %rax
        movq $ext, %rax
        movabs $ext, %rax
        mov  $ext, %eax
        ret
.data
        .quad ext
        .long ext
        .quad ext+8
        .quad f
""", "nosym")

    e.add("cfi", """\
.text
.globl cf
.type cf, @function
cf:
        .cfi_startproc
        push %rbp
        .cfi_def_cfa_offset 16
        .cfi_offset %rbp, -16
        mov %rsp, %rbp
        .cfi_def_cfa_register %rbp
        sub $32, %rsp
        push %rbx
        .cfi_offset %rbx, -24
        pop %rbx
        .cfi_restore %rbx
        leave
        .cfi_def_cfa %rsp, 8
        ret
        .cfi_endproc
.size cf, .-cf
""", "nosym")


def gen_interact(e: Emitter):
    """Relaxation, alignment and symbol arithmetic all interacting.

    This is the configuration in which a layout bug actually manifests: the
    jump distances, the padding sizes and the data that encodes label
    differences all depend on each other.
    """
    for nb in (2, 4, 8):
        for pad in (60, 62, 63, 64, 120, 124, 126):
            body = [".text", ".globl main", "main:"]
            for i in range(nb):
                body += [".p2align 4", f"blk{i}:", f"  cmp ${i}, %eax",
                         f"  je  blk{(i + 1) % nb}"]
                body += ["  nop"] * (pad // 4)
                body.append("  jmp done")
            body += [".p2align 5", "done:", "  ret",
                     ".data", "  .long done-main", "  .long blk1-blk0"]
            e.add(f"blocks_{nb}_{pad}", "\n".join(body) + "\n", "nosym")

    # A jump table whose entries are label differences in .rodata: wrong
    # relaxation silently corrupts the table.
    for pad in (30, 40, 62, 64, 126, 128):
        body = [".text", ".globl jt", "jt:", "  cmpl $3, %edi", "  ja Ldefault",
                "  movl %edi, %eax", "  leaq Ltab(%rip), %rcx",
                "  movslq (%rcx,%rax,4), %rdx", "  addq %rcx, %rdx",
                "  jmp *%rdx"]
        for i in range(4):
            body.append(f"L{i}:")
            body += ["  nop"] * pad
            body += [f"  movl ${i}, %eax", "  jmp Lend"]
        body += ["Ldefault:", "  xorl %eax, %eax", "Lend:", "  ret",
                 ".section .rodata", ".p2align 2", "Ltab:"]
        body += [f"  .long L{i}-Ltab" for i in range(4)]
        e.add(f"jumptable_{pad}", "\n".join(body) + "\n", "nosym")


def gen_reject(e: Emitter):
    """Input GNU as refuses. LCCC must refuse it too.

    Each of these, if silently accepted, encodes as a DIFFERENT valid
    instruction — the worst possible failure mode, because it turns a typo in
    hand-written assembly into a miscompile with no diagnostic.
    """
    bad = [
        "paddb %mm2, %xmm1", "vpaddb %ymm1, %ymm0",
        "vbroadcasti128 %xmm0, %ymm0", "vmovdqu %ymm0, %xmm1",
        "paddb %xmm32, %xmm1", "mov %rax, %eax", "mov %eax, %rax",
        "mov %al, %ax", "add %rax, %ecx", "movl $1, %rax",
        "lea %rax, %rbx", "mov (%rax,%rsp,4), %rbx",
        "mov (%rax,%rbx,3), %rcx", "mov (%rax,%rbx,16), %rcx",
        "push %eax", "pop %esp", "movsbl %ax, %ecx", "movzbq %eax, %rcx",
        "shl $1, $2", "vfmadd132ps %ymm1, %ymm2",
        "vextracti128 %ymm1, %xmm2", "vaddps %ymm1, %xmm2, %ymm3",
        "cmovz %al, %bl", "jmp $1", "call $1", "movq %xmm0, %ymm1",
        "vzeroupper %ymm0", "notaninstruction %rax, %rbx", "add %rax",
        "add %rax, %rbx, %rcx", "mov $1, $2", "bswapw %ax",
    ]
    for i, inst in enumerate(bad):
        e.add(f"r{i:02d}_{inst.split()[0]}", f".text\n{inst}\n", "reject")


GENERATORS = [
    ("modrm", gen_modrm), ("rex", gen_rex), ("imm", gen_imm),
    ("shift", gen_shift), ("prefix", gen_prefix), ("branch", gen_branch),
    ("padding", gen_padding), ("x87", gen_x87), ("sse", gen_sse),
    ("avx", gen_avx), ("bmi", gen_bmi), ("lea", gen_lea),
    ("misc", gen_misc), ("directive", gen_directives),
    ("interact", gen_interact), ("reject", gen_reject),
]


# ─── Oracle validation ────────────────────────────────────────────────────

def gas_ok(gas: str, body: str, tmp: Path) -> bool:
    src = tmp / "probe.s"
    src.write_text(body)
    return subprocess.run([gas, "--64", "-o", str(tmp / "probe.o"), str(src)],
                          capture_output=True, timeout=120).returncode == 0


def trim(gas: str, body: str, tmp: Path) -> str:
    """Drop only the individual lines GNU as refuses.

    A generated matrix inevitably contains a few combinations the current
    binutils spells differently; discarding just those preserves the
    surrounding coverage instead of losing the whole case.
    """
    keep: list[str] = []
    for ln in body.splitlines():
        s = ln.strip()
        if not s or s.startswith((".", "#", "/*")) or s.endswith(":"):
            keep.append(ln)
            continue
        if gas_ok(gas, "\n".join(keep + [ln]) + "\n", tmp):
            keep.append(ln)
    return "\n".join(keep) + "\n"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--as", dest="gas", default="as")
    ap.add_argument("--out-dir", type=Path,
                    default=Path(__file__).resolve().parents[1] / "tests" / "asm-diff")
    ap.add_argument("--only", default="")
    args = ap.parse_args()

    only = set(args.only.split(",")) if args.only else None
    args.out_dir.mkdir(parents=True, exist_ok=True)
    total = kept = 0
    with tempfile.TemporaryDirectory(prefix="gencorpus-") as td:
        tmp = Path(td)
        for group, fn in GENERATORS:
            if only and group not in only:
                continue
            e = Emitter()
            fn(e)
            out: list[str] = []
            for name, body, flags in e.cases:
                total += 1
                want_reject = "reject" in flags
                if gas_ok(args.gas, body, tmp) == (not want_reject):
                    pass
                elif want_reject:
                    print(f"  drop (gas accepts): {group}/{name}", file=sys.stderr)
                    continue
                else:
                    body = trim(args.gas, body, tmp)
                    real = [l for l in body.splitlines()
                            if l.strip() and not l.strip().startswith(".")
                            and not l.strip().endswith(":")]
                    # A case can fail STRUCTURALLY (e.g. a .p2align that makes
                    # an unrelaxable jump too long); trimming instructions
                    # cannot repair that, and such a case is not a valid
                    # oracle input.
                    if not real or not gas_ok(args.gas, body, tmp):
                        print(f"  drop (structural): {group}/{name}", file=sys.stderr)
                        continue
                    print(f"  trim: {group}/{name}", file=sys.stderr)
                kept += 1
                out.append(f";;; {name}" + (f" {flags}" if flags else ""))
                out.append(body.rstrip("\n"))
            path = args.out_dir / f"{group}.casefile"
            path.write_text("\n".join(out) + "\n")
            print(f"wrote {path.name}: {len(e.cases)} cases", file=sys.stderr)
    print(f"total generated={total} kept={kept}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
