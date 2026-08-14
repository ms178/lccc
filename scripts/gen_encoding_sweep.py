#!/usr/bin/env python3
"""Generate instructions that have more than one legal encoding.

`gen_asmdiff_corpus.py` builds a corpus for *correctness*: it walks the ModRM,
SIB, REX and VEX decision trees so every structural encoding path is taken at
least once. That corpus is deliberately GAS-shaped, and it is already clean.

This generator targets a different axis: places where the instruction set
offers a CHOICE, so two assemblers can both be right and one can be smaller.
Those are the only places where `encdiff.py` can find an improvement, and most
of them are invisible to a corpus built around structural coverage.

The families, and why each admits a choice:

  accumulator     Many ALU ops have a short `op %eax, imm32` form (opcode
                  0x05/0x2D/...) that saves the ModRM byte. Only valid when
                  the destination is the accumulator.
  imm8_sext       An immediate in [-128,127] can use the sign-extended imm8
                  opcode (0x83) instead of imm32 (0x81): saves 3 bytes.
  redundant_rex   A REX byte is only needed for W, r8-r15, or spl/bpl/sil/dil.
                  Emitting 0x40 otherwise is legal but wasteful.
  mov_imm_zero    `mov $0,%rax` can be `xor %eax,%eax` only if flags are dead,
                  which an assembler cannot know -- but `movq $0` vs
                  `movl $0` into the same register IS an assembler choice, and
                  `movabsq $small` should shrink to the 5-byte form.
  sib_fold        A scale-1 index with no base folds into the base slot; a
                  zero displacement can drop entirely.
  disp_shrink     disp32 that fits in disp8 (and disp8 of zero) should shrink.
  vex_len         VEX3 collapses to VEX2 whenever X, B and W are all clear and
                  the map is 0F: one byte saved on a very common form.
  test_and        `test $imm,%reg` has an accumulator short form; the 8-bit
                  forms have their own opcode.
  push_imm        push imm8 vs imm32.
  shift_one       Shift by 1 has a dedicated opcode (0xD1) shorter than the
                  imm8 form (0xC1 /r ib).
  inc_dec         inc/dec via 0xFE/0xFF vs the (invalid in 64-bit) 0x40+r.
  xchg_acc        xchg with the accumulator has the one-byte 0x90+r form.
  lea_strength    LEA forms that a smarter assembler can shorten.

Output is one instruction per line, ready for `encdiff.py --file`.
"""
from __future__ import annotations

import argparse
import sys

GP64 = ["rax", "rbx", "rcx", "rdx", "rsi", "rdi", "rbp",
        "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15"]
GP32 = ["eax", "ebx", "ecx", "edx", "esi", "edi", "ebp",
        "r8d", "r9d", "r10d", "r11d", "r12d", "r13d", "r14d", "r15d"]
GP16 = ["ax", "bx", "cx", "dx", "si", "di", "bp",
        "r8w", "r9w", "r10w", "r11w", "r12w", "r13w", "r14w", "r15w"]
GP8 = ["al", "bl", "cl", "dl", "sil", "dil", "bpl", "spl",
       "r8b", "r9b", "r10b", "r11b", "r12b", "r13b", "r14b", "r15b"]
XMM = [f"xmm{i}" for i in range(16)]
YMM = [f"ymm{i}" for i in range(16)]

ALU = ["add", "sub", "and", "or", "xor", "cmp", "adc", "sbb"]
# Values that sit on an encoding boundary: imm8 range, imm32 range, and the
# sign-extension edges where a smaller opcode becomes legal or stops being so.
IMM_EDGE = [0, 1, 127, 128, -1, -128, -129, 255, 256, 32767, 65535,
            2147483647, -2147483648]
DISP_EDGE = [0, 1, 127, 128, -1, -128, -129, 255, 1024, 65536]


def accumulator(out):
    """op $imm, %acc -- short form exists only for the accumulator."""
    for op in ALU:
        for imm in IMM_EDGE:
            out.append(f"{op}b ${imm & 0xff}, %al")
            out.append(f"{op}w ${imm & 0xffff}, %ax")
            out.append(f"{op}l ${imm}, %eax")
            out.append(f"{op}q ${imm}, %rax")
            # Same immediate into a non-accumulator: no short form, so this is
            # the control that proves the short form is not applied wrongly.
            out.append(f"{op}l ${imm}, %ecx")
            out.append(f"{op}q ${imm}, %rbx")


def imm8_sext(out):
    """imm in [-128,127] should take the sign-extended imm8 opcode."""
    for op in ALU:
        for imm in [-128, -1, 0, 1, 127, 128, -129]:
            for r in ("ecx", "edx", "r8d", "r13d"):
                out.append(f"{op}l ${imm}, %{r}")
            for r in ("rcx", "rdx", "r8", "r13"):
                out.append(f"{op}q ${imm}, %{r}")
            out.append(f"{op}l ${imm}, (%rdi)")
            out.append(f"{op}q ${imm}, 8(%rsi)")


def redundant_rex(out):
    """Registers that do and do not require a REX byte."""
    for r in GP8:
        out.append(f"movb $1, %{r}")
        out.append(f"addb $1, %{r}")
        out.append(f"testb %{r}, %{r}")
    for a, b in (("al", "cl"), ("sil", "dil"), ("spl", "bpl"),
                 ("r8b", "r9b"), ("al", "r8b"), ("r8b", "al")):
        out.append(f"movb %{a}, %{b}")
        out.append(f"xorb %{a}, %{b}")


def mov_imm(out):
    """Immediate loads: width choice and the movabs shrink."""
    for r64, r32 in zip(GP64, GP32):
        out.append(f"movq $0, %{r64}")
        out.append(f"movl $0, %{r32}")
        out.append(f"movq $1, %{r64}")
        out.append(f"movq $-1, %{r64}")
        out.append(f"movq $4294967295, %{r64}")
        out.append(f"movq $2147483647, %{r64}")
        out.append(f"movq $2147483648, %{r64}")
        out.append(f"movabsq $1, %{r64}")
        out.append(f"movabsq $4294967296, %{r64}")


def sib_fold(out):
    """Scale-1 index folds; zero displacements; every scale."""
    for r in GP64:
        if r == "rsp":
            continue
        for d in (0, 1, -1, 127, 128, -128, -129):
            out.append(f"mov {d}(,%{r},1), %rcx")
            out.append(f"lea {d}(,%{r},1), %rdx")
        for sc in (2, 4, 8):
            out.append(f"mov 8(,%{r},{sc}), %rcx")
        out.append(f"mov (,%{r},1), %rcx")
        out.append(f"mov (%{r}), %rcx")
        out.append(f"mov 0(%{r}), %rcx")


def disp_shrink(out):
    """disp32 that fits disp8, and disp8 that is zero."""
    for r in ("rax", "rbx", "rbp", "r12", "r13", "rsp"):
        for d in DISP_EDGE:
            out.append(f"mov {d}(%{r}), %rcx")
            out.append(f"mov %rcx, {d}(%{r})")
            out.append(f"lea {d}(%{r}), %rdx")
    for b in ("rax", "r12", "rbp", "r13"):
        for i in ("rbx", "r9"):
            for d in (0, 1, 127, 128):
                out.append(f"mov {d}(%{b},%{i},4), %rcx")


def vex_len(out):
    """VEX3 should collapse to VEX2 when X, B and W are clear."""
    pairs = [("xmm0", "xmm1"), ("xmm8", "xmm0"), ("xmm0", "xmm8"),
             ("xmm8", "xmm9"), ("ymm0", "ymm1"), ("ymm8", "ymm0"),
             ("ymm0", "ymm8"), ("ymm8", "ymm9")]
    ops2 = ["vmovaps", "vmovapd", "vmovups", "vmovupd", "vmovdqa", "vmovdqu",
            "vsqrtps", "vrcpps", "vrsqrtps"]
    for op in ops2:
        for a, b in pairs:
            if op.endswith(("ps", "pd")) or op.startswith("vmov"):
                out.append(f"{op} %{a}, %{b}")
    ops3 = ["vaddps", "vsubps", "vmulps", "vdivps", "vandps", "vxorps",
            "vaddpd", "vmulpd", "vpaddb", "vpaddd", "vpaddq", "vpand",
            "vpxor", "vpor", "vpsubd", "vpmullw"]
    for op in ops3:
        for a in ("xmm1", "xmm9"):
            for b in ("xmm2", "xmm10"):
                for c in ("xmm3", "xmm11"):
                    out.append(f"{op} %{a}, %{b}, %{c}")
    for op in ("vaddps", "vpaddd", "vpxor"):
        for m in ("(%rdi)", "(%r13)", "8(%rax,%rbx,4)", "(%r12)"):
            out.append(f"{op} {m}, %xmm2, %xmm3")
            out.append(f"{op} {m}, %ymm2, %ymm3")


def test_and(out):
    for imm in (1, 127, 128, 255, 256, 65535, -1):
        out.append(f"testb ${imm & 0xff}, %al")
        out.append(f"testb ${imm & 0xff}, %cl")
        out.append(f"testw ${imm & 0xffff}, %ax")
        out.append(f"testl ${imm}, %eax")
        out.append(f"testl ${imm}, %ecx")
        out.append(f"testq ${imm}, %rax")
        out.append(f"testq ${imm}, %rbx")
    for a in ("al", "cl", "eax", "ecx", "rax", "rbx", "r8", "r8d"):
        out.append(f"test %{a}, %{a}")


def push_imm(out):
    for imm in (0, 1, 127, 128, -1, -128, -129, 32767, 2147483647):
        out.append(f"pushq ${imm}")
    for r in GP64:
        out.append(f"pushq %{r}")
        out.append(f"popq %{r}")


def shift_one(out):
    for op in ("shl", "shr", "sar", "rol", "ror", "rcl", "rcr"):
        for r in ("eax", "ecx", "r8d"):
            out.append(f"{op}l $1, %{r}")
            out.append(f"{op}l $2, %{r}")
            out.append(f"{op}l %cl, %{r}")
        for r in ("rax", "rcx", "r8"):
            out.append(f"{op}q $1, %{r}")
            out.append(f"{op}q $2, %{r}")
        out.append(f"{op}b $1, %al")
        out.append(f"{op}w $1, %ax")


def inc_dec(out):
    for r32, r64 in zip(GP32, GP64):
        out.append(f"incl %{r32}")
        out.append(f"decl %{r32}")
        out.append(f"incq %{r64}")
        out.append(f"decq %{r64}")
    for m in ("(%rdi)", "8(%rsi)", "(%r12)"):
        out.append(f"incl {m}")
        out.append(f"decq {m}")


def xchg_acc(out):
    for r32, r64 in zip(GP32, GP64):
        out.append(f"xchg %eax, %{r32}")
        out.append(f"xchg %{r32}, %eax")
        out.append(f"xchg %rax, %{r64}")
        out.append(f"xchg %{r64}, %rax")
    out.append("xchg %ecx, %edx")
    out.append("xchg %rcx, %rdx")


def lea_strength(out):
    for r in ("rax", "rbx", "r12", "r13"):
        out.append(f"lea (%{r}), %rcx")
        out.append(f"lea 0(%{r}), %rcx")
        out.append(f"lea (%{r},%rbx,1), %rcx")
        out.append(f"lea 0(%{r},%rbx,1), %rcx")
        out.append(f"lea (,%{r},2), %rcx")
        out.append(f"lea (,%{r},4), %rcx")
        out.append(f"lea (,%{r},8), %rcx")
    out.append("lea (%rax,%rax,1), %rcx")
    out.append("lea (%rax,%rax,2), %rcx")


def movzx_sx(out):
    for src8 in ("al", "cl", "sil", "r8b"):
        for dst in ("eax", "ecx", "r9d"):
            out.append(f"movzbl %{src8}, %{dst}")
            out.append(f"movsbl %{src8}, %{dst}")
        out.append(f"movzbq %{src8}, %rax")
        out.append(f"movsbq %{src8}, %rax")
    for src16 in ("ax", "cx", "r8w"):
        out.append(f"movzwl %{src16}, %eax")
        out.append(f"movswl %{src16}, %eax")
        out.append(f"movzwq %{src16}, %rax")
    for src32 in ("eax", "ecx", "r8d"):
        out.append(f"movslq %{src32}, %rax")


def setcc_cmov(out):
    ccs = ["o", "no", "b", "ae", "e", "ne", "be", "a",
           "s", "ns", "p", "np", "l", "ge", "le", "g"]
    for cc in ccs:
        out.append(f"set{cc} %al")
        out.append(f"set{cc} %r8b")
        out.append(f"set{cc} (%rdi)")
        out.append(f"cmov{cc}l %eax, %ecx")
        out.append(f"cmov{cc}q %rax, %rcx")


def nop_forms(out):
    out.append("nop")
    for n in range(1, 16):
        out.append(f".p2align {n}" if False else "nop")
    for m in ("%ax", "%eax", "(%rax)", "0(%rax)", "0(%rax,%rax,1)"):
        out.append(f"nopw {m}" if m in ("%ax",) else f"nopl {m}")


GROUPS = {
    "accumulator": accumulator,
    "imm8_sext": imm8_sext,
    "redundant_rex": redundant_rex,
    "mov_imm": mov_imm,
    "sib_fold": sib_fold,
    "disp_shrink": disp_shrink,
    "vex_len": vex_len,
    "test_and": test_and,
    "push_imm": push_imm,
    "shift_one": shift_one,
    "inc_dec": inc_dec,
    "xchg_acc": xchg_acc,
    "lea_strength": lea_strength,
    "movzx_sx": movzx_sx,
    "setcc_cmov": setcc_cmov,
    "nop_forms": nop_forms,
}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--groups", nargs="*", default=sorted(GROUPS),
                    help="which families to emit (default: all)")
    ap.add_argument("--list", action="store_true", help="list family names")
    ap.add_argument("-o", "--output", help="write here instead of stdout")
    args = ap.parse_args()

    if args.list:
        for g in sorted(GROUPS):
            print(g)
        return 0

    out: list[str] = []
    for g in args.groups:
        if g not in GROUPS:
            print(f"unknown group: {g}", file=sys.stderr)
            return 2
        GROUPS[g](out)

    seen: set[str] = set()
    uniq = [i for i in out if not (i in seen or seen.add(i))]
    text = "\n".join(uniq) + "\n"
    if args.output:
        with open(args.output, "w") as f:
            f.write(text)
        print(f"{len(uniq)} instructions -> {args.output}", file=sys.stderr)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
