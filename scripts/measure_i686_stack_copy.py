#!/usr/bin/env python3
"""Measure the i686 by-value argument copy idioms on the host CPU.

This is a *standalone* harness: it needs only `as --32` and `ld -m elf_i386`
(binutils) plus a CPU that executes i386 code. It does not build or invoke
lccc, so the copy policy can be audited and re-derived on any host, including
machines where the compiler cannot be built.

What it models is exactly what the backend emits. `emit_copy_bytes`
(`src/backend/i686/codegen/emit.rs`) stages a by-value argument into the
outgoing argument area by copying `size` bytes from a frame object (or from a
pointer in %ecx) to `off(%esp)`. Candidate instruction mixes:

  scalar  : 4-byte `movl` pairs, then 2-byte, then 1-byte        (-mno-sse)
  sse4    : 16-byte `movups` pairs reusing %xmm0, 4/2/1-byte tails
  sse8    : 16-byte pairs, 8-byte `movlps` tail, then 4/2/1-byte tails
  sseT    : 16-byte pairs, final chunk at `size-16` (overlapping)  (shipped)
  rep     : `rep movsl` with the %esi/%edi/%ecx save/restore the backend needs

Modes:
  copy : the copy alone (isolates copy throughput)
  fwd  : copy, then read the first and last dword at once -- the callee's
         worst case, a store-forwarded read straight out of the copy stores
  call : copy, then call/ret, then the same reads -- the real sequence

Results (Intel Xeon @ 2.60 GHz, min of 101 rounds, L1-hot, cycles per copy,
fwd mode -- the regime the call sequence produces):

  size    scalar   sse4    sse8    sseT
    16      3.07   1.93    1.92    1.93
    20      3.08   2.31    2.30    2.31
    21     15.31  14.55   14.55    2.31
    24      3.85  14.55    2.31    2.31
    25     16.08  14.55   16.08    2.31
    28      3.84   3.07    2.69    2.31
    31     16.84   3.07   15.31    2.31
    32      4.61   2.31    2.31    2.31
    33     16.84  16.07   16.07    2.69
    40      5.38  15.31    2.69    2.70
    48      6.13   2.69    2.69    2.69
    64      7.66   3.08    3.07    3.07

Reading of the data, which is what the shipped policy encodes:

* 16-byte chunks are a 1.6-2.6x win over scalar at every size (0.048 vs
  0.120 cyc/byte at 64 bytes). Kept.
* A tail built from two 4-byte `movl` pairs (sse4) stalls the callee field
  read by ~12 cycles at the 8-mod-16 sizes (24, 40): 14.55 vs 2.31. Both
  fixes remove it; sseT is chosen because it needs no extra instruction for
  remainders of 9..15 bytes and no new mnemonic.
* sse8 (a `movlps` tail) is equal wherever it wins and equal-or-worse on
  21..23, 25..27 and 29..31, where it still needs scalar tails.
* `rep movsl` is 25+ cycles up to 256 bytes: rejected for argument-sized
  copies. The large-object path keeps its own `rep movsb`.
* Alternating %xmm0/%xmm1 (not shown) is within the +/-10% noise floor:
  rejected as unmeasurable.

Re-run with `python3 scripts/measure_i686_stack_copy.py`; the numbers above must
be reproduced before a policy change is accepted.
"""
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

SIZES = [16, 17, 20, 21, 24, 25, 28, 31, 32, 33, 40, 48, 64]
REPS = 2000
ROUNDS = 101
VARIANTS = ["scalar", "sse4", "sse8", "sseT"]
MODES = ["copy", "fwd", "call"]


def tails(off: int, size: int, dstbase: int = 0) -> list[str]:
    out = []
    rem = size - off
    if rem >= 4:
        out.append(f"    movl {off}(%esi), %eax")
        out.append(f"    movl %eax, {dstbase + off}(%esp)")
        off += 4
        rem -= 4
    if rem >= 2:
        out.append(f"    movzwl {off}(%esi), %eax")
        out.append(f"    movw %ax, {dstbase + off}(%esp)")
        off += 2
        rem -= 2
    if rem:
        out.append(f"    movb {off}(%esi), %al")
        out.append(f"    movb %al, {dstbase + off}(%esp)")
    return out


def copy_body(variant: str, size: int) -> list[str]:
    if variant == "scalar":
        out, off = [], 0
        while off + 4 <= size:
            out.append(f"    movl {off}(%esi), %eax")
            out.append(f"    movl %eax, {off}(%esp)")
            off += 4
        return out + tails(off, size)

    if variant == "rep":
        # Fair: the backend must save the callee-saved pair it borrows.
        return [
            "    pushl %edi", "    pushl %ecx",
            "    movl %esp, %edi", "    addl $8, %edi",       # dst = pre-push %esp
            "    movl $srcbuf, %esi",                        # rep advances %esi
            f"    movl ${size // 4}, %ecx",
            "    rep movsl",
            "    popl %ecx", "    popl %edi",
        ] + tails((size // 4) * 4, size)

    out, off, i = [], 0, 0
    while off + 16 <= size:
        reg = "%xmm0" if (variant != "sse4x2" or i % 2 == 0) else "%xmm1"
        out.append(f"    movups {off}(%esi), {reg}")
        out.append(f"    movups {reg}, {off}(%esp)")
        off += 16
        i += 1
    if variant == "sseT":
        if size < 16:
            return tails(0, size)
        if off < size:            # overlapping final chunk, wholly inside [0,size)
            o = size - 16
            return out + [f"    movups {o}(%esi), %xmm0",
                          f"    movups %xmm0, {o}(%esp)"]
        return out
    if variant == "sse8" and size - off >= 8:
        out.append(f"    movlps {off}(%esi), %xmm0")
        out.append(f"    movlps %xmm0, {off}(%esp)")
        off += 8
    return out + tails(off, size)


def observe(size: int, mode: str) -> list[str]:
    if mode == "copy":
        return []
    reads = ["    addl 0(%esp), %eax", f"    addl {size - 4}(%esp), %eax"]
    if mode == "call":
        return ["    call Lsink"] + reads
    return reads


def bench_fn(variant: str, size: int, mode: str) -> str:
    name = f"bench_{variant}_{size}_{mode}"
    arena = ((size + 32 + 15) // 16) * 16
    body = copy_body(variant, size)
    lines = [
        f".globl {name}", f".type {name}, @function", f"{name}:",
        "    pushl %ebx", "    pushl %ebp", "    pushl %esi",
        f"    subl ${arena}, %esp",
        "    movl $srcbuf, %esi",
        "    xorl %eax, %eax",
        "    lfence", "    rdtsc", "    movl %eax, %ebx", "    movl %edx, %ebp",
        f"    movl ${REPS}, %ecx",
        f"L{name}:",
        *body,
        *observe(size, mode),
        "    decl %ecx",
        f"    jnz L{name}",
        "    lfence", "    rdtsc",
        "    subl %ebx, %eax", "    sbbl %ebp, %edx",
        "    addl %edx, %eax",
        f"    addl ${arena}, %esp",
        "    popl %esi", "    popl %ebp", "    popl %ebx",
        "    ret",
    ]
    return "\n".join(lines)


def main() -> int:
    order, funcs = [], []
    for variant in VARIANTS:
        for size in SIZES:
            for mode in MODES:
                funcs.append(bench_fn(variant, size, mode))
                order.append((variant, size, mode))

    start = [".globl _start", "_start:", "    andl $-16, %esp",
             "    subl $1024, %esp", "    movl %esp, %edi"]
    for i, _key in enumerate(order):
        start += [f"    call bench_{_key[0]}_{_key[1]}_{_key[2]}",
                  f"    movl %eax, {4 * i}(%edi)"]
    start += ["    movl $4, %eax", "    movl $1, %ebx", "    movl %edi, %ecx",
              f"    movl ${4 * len(order)}, %edx", "    int $0x80",
              "    movl $1, %eax", "    xorl %ebx, %ebx", "    int $0x80"]

    asm = "\n".join([
        ".code32", ".section .bss", ".balign 64",
        "srcbuf: .space 4096", ".text", *funcs,
        "Lsink:", "    ret",
        *start,
        '.section .note.GNU-stack,"",@progbits', ""])

    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        (td / "b.s").write_text(asm)
        subprocess.run(["as", "--32", str(td / "b.s"), "-o", str(td / "b.o")], check=True)
        subprocess.run(["ld", "-m", "elf_i386", str(td / "b.o"), "-o", str(td / "b")], check=True)
        samples = {k: [] for k in order}
        for _ in range(ROUNDS):
            out = subprocess.run([str(td / "b")], check=True, capture_output=True).stdout
            for key, val in zip(order, struct.unpack(f"<{len(order)}I", out)):
                samples[key].append(val)

    med = {k: sorted(v)[0] / REPS for k, v in samples.items()}
    host = "unknown"
    for line in Path("/proc/cpuinfo").read_text().splitlines():
        if line.startswith("model name"):
            host = line.split(":", 1)[1].strip()
            break
    print(f"host: {host}")
    print(f"min-of-{ROUNDS} cycles per copy, {REPS} copies per round, L1-hot, "
          f"dst = outgoing area below %esp, src = 16-aligned frame object")
    for mode in MODES:
        print(f"\n== mode {mode} ==")
        print(f"{'size':>5} " + " ".join(f"{v:>9}" for v in VARIANTS) +
              "   best")
        for size in SIZES:
            cells = [f"{med[(v, size, mode)]:9.2f}" for v in VARIANTS]
            best = min(VARIANTS, key=lambda v: med[(v, size, mode)])
            print(f"{size:5} " + " ".join(cells) + f"   {best}")
        print(f"{'':5} " + " ".join(f"{med[(v, SIZES[-1], mode)] / SIZES[-1]:9.3f}"
                                    for v in VARIANTS) + "   cyc/byte @256")
    return 0


if __name__ == "__main__":
    sys.exit(main())
