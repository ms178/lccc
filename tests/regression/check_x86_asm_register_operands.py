#!/usr/bin/env python3
"""x86 `asm` operands in `ymm`, `zmm`, mask and MMX registers.

The back ends hand out general registers and `xmm` registers. An operand
that wants another kind used to go wrong in silence or in the assembler:

* a 256- or 512-bit vector under "x" or "v" was loaded eight bytes wide
  into an `xmm` register and printed under the `xmm` name;
* "k" was refused as an impossible constraint, "Yk" got an `xmm` register;
* "y" got a general register.

The lowering now rewrites such a statement to load and store these operands
itself (src/ir/lowering/x86_asm_registers.rs). This gate holds it to:

1. the text of each statement: the moves, the registers and the template,
   line for line;
2. GCC's text for the same statement, where GCC is given: the template's
   instructions with the same register class and width in every place;
3. the diagnostics: what GCC refuses is refused with GCC's words, and the
   differences from GCC are listed here by name;
4. values: a few hundred generated statements, run and compared with the
   same program built by GCC, at -O0 and -O2 and in 32-bit code.

Usage: check_x86_asm_register_operands.py --lccc BIN [--gcc gcc-16]
"""

from __future__ import annotations

import argparse
import os
import random
import re
import subprocess
import sys
import tempfile

PROBLEMS: list[str] = []


def problem(text: str) -> None:
    PROBLEMS.append(text)


def run(command: list[str], **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(command, capture_output=True, text=True, **kwargs)


# ---------------------------------------------------------------- shapes

HEADER = "#include <immintrin.h>\n"

# name, flags, source of a function `f`, the lines lccc prints between #APP
# and #NO_APP (normalised), the index of the template's first line among
# them, the number of lines the template has, and whether GCC's template is
# comparable (not where GCC is free to choose a register and lccc memory).
SHAPES = [
    (
        "three ymm",
        "-mavx2",
        '__m256i f(__m256i a, __m256i b) { __m256i r;'
        ' __asm__("vpaddd %2, %1, %0" : "=x"(r) : "x"(a), "x"(b)); return r; }',
        ["vmovdqu MEM, %ymm14", "vmovdqu MEM, %ymm13",
         "vpaddd %ymm13, %ymm14, %ymm15", "vmovdqu %ymm15, MEM"],
        2, 1, True,
    ),
    (
        "three ymm, 32-bit code",
        "-m32 -msse2 -mavx2",
        '__m256i f(__m256i a, __m256i b) { __m256i r;'
        ' __asm__("vpaddd %2, %1, %0" : "=x"(r) : "x"(a), "x"(b)); return r; }',
        ["vmovdqu MEM, %ymm6", "vmovdqu MEM, %ymm5",
         "vpaddd %ymm5, %ymm6, %ymm7", "vmovdqu %ymm7, MEM"],
        2, 1, True,
    ),
    (
        "zmm under a zeroing mask",
        "-mavx512f",
        '__m512i f(__m512i a, __m512i b, unsigned short m) { __m512i r;'
        ' __asm__("vpaddd %2, %1, %0%{%3%}%{z%}" : "=v"(r) : "v"(a), "v"(b), "Yk"(m));'
        ' return r; }',
        ["vmovdqu64 MEM, %zmm14", "vmovdqu64 MEM, %zmm13", "kmovw %R32, %k7",
         "vpaddd %zmm13, %zmm14, %zmm15{%k7}{z}", "vmovdqu64 %zmm15, MEM"],
        3, 1, True,
    ),
    (
        "zmm merged into a tied input",
        "-mavx512f",
        '__m512i f(__m512i s, __m512i a, __m512i b, unsigned short m) { __m512i r;'
        ' __asm__("vpaddd %3, %2, %0%{%4%}" : "=v"(r) : "0"(s), "v"(a), "v"(b), "Yk"(m));'
        ' return r; }',
        ["vmovdqu64 MEM, %zmm15", "vmovdqu64 MEM, %zmm14", "vmovdqu64 MEM, %zmm13",
         "kmovw %R32, %k7", "vpaddd %zmm13, %zmm14, %zmm15{%k7}", "vmovdqu64 %zmm15, MEM"],
        4, 1, True,
    ),
    (
        "a compare writes a mask",
        "-mavx512f",
        'unsigned short f(__m512i a, __m512i b) { unsigned short m;'
        ' __asm__("vpcmpgtd %2, %1, %0" : "=k"(m) : "v"(a), "v"(b)); return m; }',
        ["vmovdqu64 MEM, %zmm15", "vmovdqu64 MEM, %zmm14",
         "vpcmpgtd %zmm14, %zmm15, %k7", "kmovw %k7, %R32"],
        2, 1, True,
    ),
    (
        "eight bits of mask lose what is above them",
        "-mavx512f",
        'unsigned f(unsigned char m) { unsigned r;'
        ' __asm__("kmovw %1, %0" : "=r"(r) : "k"(m)); return r; }',
        ["kmovw %R32, %k7", "kshiftlw $8, %k7, %k7", "kshiftrw $8, %k7, %k7",
         "kmovw %k7, %R32"],
        3, 1, True,
    ),
    (
        "sixty-four bits of mask",
        "-mavx512bw",
        'unsigned long long f(unsigned long long a, unsigned long long b) {'
        ' unsigned long long r;'
        ' __asm__("kxorq %2, %1, %0" : "=k"(r) : "k"(a), "k"(b)); return r; }',
        ["kmovq %R64, %k6", "kmovq %R64, %k5", "kxorq %k5, %k6, %k7", "kmovq %k7, %R64"],
        2, 1, True,
    ),
    (
        "sixty-four bits of mask, 32-bit code",
        "-m32 -mavx512bw",
        'unsigned long long f(unsigned long long a, unsigned long long b) {'
        ' unsigned long long r;'
        ' __asm__("kxorq %2, %1, %0" : "=k"(r) : "k"(a), "k"(b)); return r; }',
        ["kmovq MEM, %k6", "kmovq MEM, %k5", "kxorq %k5, %k6, %k7", "kmovq %k7, MEM"],
        2, 1, True,
    ),
    (
        "MMX",
        "-mmmx",
        '__m64 f(__m64 a, __m64 b) { __m64 r;'
        ' __asm__("movq %1, %0\\n\\tpaddd %2, %0" : "=&y"(r) : "y"(a), "y"(b)); return r; }',
        ["movq %R64, %mm6", "movq %R64, %mm5", "movq %mm6, %mm7", "paddd %mm5, %mm7",
         "movq %mm7, %R64"],
        2, 2, True,
    ),
    (
        "MMX, 32-bit code",
        "-m32 -mmmx",
        '__m64 f(__m64 a, __m64 b) { __m64 r;'
        ' __asm__("movq %1, %0\\n\\tpaddd %2, %0" : "=&y"(r) : "y"(a), "y"(b)); return r; }',
        ["movq MEM, %mm6", "movq MEM, %mm5", "movq %mm6, %mm7", "paddd %mm5, %mm7",
         "movq %mm7, MEM"],
        2, 2, True,
    ),
    (
        "thirty-two bits in an MMX register",
        "-mmmx",
        'unsigned f(unsigned a) { unsigned r;'
        ' __asm__("movq %1, %0\\n\\tpslld $4, %0" : "=&y"(r) : "y"(a)); return r; }',
        ["movd %R32, %mm6", "movq %mm6, %mm7", "pslld $4, %mm7", "movd %mm7, %R32"],
        1, 2, True,
    ),
    (
        "the three views of a register",
        "-mavx512f",
        '__m512i f(__m256i a) { __m512i r;'
        ' __asm__("vmovdqa %x1, %x0\\n\\tvinserti64x4 $1, %t1, %g0, %0"'
        ' : "=&x"(r) : "x"(a)); return r; }',
        ["vmovdqu MEM, %ymm14", "vmovdqa %xmm14, %xmm15",
         "vinserti64x4 $1, %ymm14, %zmm15, %zmm15", "vmovdqu64 %zmm15, MEM"],
        1, 2, True,
    ),
    (
        "memory beside the register",
        "-mavx2",
        '__m256i f(__m256i a, __m256i b) { __m256i r;'
        ' __asm__("vpaddd %2, %1, %%ymm0\\n\\tvmovdqu %%ymm0, %0"'
        ' : "=xm"(r) : "x"(a), "xm"(b) : "xmm0"); return r; }',
        ["vmovdqu MEM, %ymm15", "vpaddd MEM, %ymm15, %ymm0", "vmovdqu %ymm0, MEM"],
        1, 2, False,
    ),
    (
        "a general operand is memory",
        "-mavx2",
        '__m256i f(__m256i a) { __m256i r;'
        ' __asm__("vmovdqu %1, %%ymm0\\n\\tvmovdqu %%ymm0, %0"'
        ' : "=m"(r) : "g"(a) : "xmm0"); return r; }',
        ["vmovdqu MEM, %ymm0", "vmovdqu %ymm0, MEM"],
        0, 2, True,
    ),
    (
        "a register variable",
        "-mavx2",
        '__m256i f(__m256i a, __m256i b) { register __m256i p __asm__("ymm7") = a; __m256i r;'
        ' __asm__("vpaddd %1, %2, %0" : "=x"(r) : "x"(p), "x"(b)); return r; }',
        ["vmovdqu MEM, %ymm7", "vmovdqu MEM, %ymm14",
         "vpaddd %ymm7, %ymm14, %ymm15", "vmovdqu %ymm15, MEM"],
        2, 1, True,
    ),
    (
        "a register the statement names",
        "-mavx2",
        '__m256i f(__m256i a) { __m256i r;'
        ' __asm__("vpxor %%ymm15, %%ymm15, %%ymm15\\n\\tvpsubd %1, %%ymm15, %0"'
        ' : "=x"(r) : "x"(a) : "xmm15", "xmm14"); return r; }',
        ["vmovdqu MEM, %ymm12", "vpxor %ymm15, %ymm15, %ymm15",
         "vpsubd %ymm12, %ymm15, %ymm13", "vmovdqu %ymm13, MEM"],
        1, 2, True,
    ),
    (
        "read and written",
        "-mavx2",
        '__m256i f(__m256i a, __m256i b) {'
        ' __asm__("vpsubd %1, %0, %0" : "+x"(a) : "x"(b)); return a; }',
        ["vmovdqu MEM, %ymm15", "vmovdqu MEM, %ymm14",
         "vpsubd %ymm14, %ymm15, %ymm15", "vmovdqu %ymm15, MEM"],
        2, 1, True,
    ),
    (
        "the first SSE register",
        "-mavx2",
        '__m256i f(__m256i a) { __m256i r;'
        ' __asm__("vpabsd %1, %0" : "=Yz"(r) : "x"(a)); return r; }',
        ["vmovdqu MEM, %ymm15", "vpabsd %ymm15, %ymm0", "vmovdqu %ymm0, MEM"],
        1, 1, True,
    ),
]

MEMORY = re.compile(r"(?:[-\w.+$]*\([^)]*\)|\b[A-Za-z_.][\w.]*\+?-?\d*\(%rip\))")
GENERAL32 = re.compile(r"%(?:e[a-ds][xip]|e[sd]i|r\d+d)\b")
GENERAL64 = re.compile(r"%(?:r[a-ds][xip]|r[sd]i|r\d+)\b")
NUMBERED = re.compile(r"%(xmm|ymm|zmm|mm|k)\d+")


def normalise(line: str) -> str:
    line = " ".join(line.replace("\t", " ").split())
    line = MEMORY.sub("MEM", line)
    line = GENERAL32.sub("%R32", line)
    return GENERAL64.sub("%R64", line)


def asm_block(text: str, function: str) -> list[str] | None:
    """The lines of the one `asm` statement of `function`."""
    lines = text.split("\n")
    try:
        start = next(i for i, line in enumerate(lines) if line.strip() == f"{function}:")
    except StopIteration:
        return None
    block: list[str] = []
    inside = False
    for line in lines[start:]:
        stripped = line.strip()
        if stripped == "#APP":
            inside = True
            continue
        if stripped == "#NO_APP":
            return block
        if inside and stripped and not stripped.startswith("#"):
            block.append(normalise(stripped))
    return None


def check_shapes(lccc: str, gcc: str | None, tmp: str) -> int:
    for index, (name, flags, body, want, first, count, comparable) in enumerate(SHAPES):
        source = os.path.join(tmp, f"shape{index}.c")
        with open(source, "w") as handle:
            handle.write(HEADER + body + "\n")
        for level in ("-O0", "-O2"):
            out = os.path.join(tmp, f"shape{index}.s")
            done = run([lccc, level, *flags.split(), "-S", source, "-o", out])
            if done.returncode != 0:
                problem(f"shape '{name}' {level}: lccc fails: {done.stderr.strip()[:300]}")
                continue
            got = asm_block(open(out).read(), "f")
            if got != want:
                problem(f"shape '{name}' {level}:\n    got  {got}\n    want {want}")
        if not (gcc and comparable):
            continue
        out = os.path.join(tmp, f"shape{index}.gcc.s")
        done = run([gcc, "-O2", *flags.split(), "-S", source, "-o", out])
        if done.returncode != 0:
            problem(f"shape '{name}': GCC refuses it: {done.stderr.strip()[:300]}")
            continue
        theirs = asm_block(open(out).read(), "f")
        ours = want[first:first + count]
        blank = [NUMBERED.sub(r"%\1#", line) for line in ours]
        if theirs is None or [NUMBERED.sub(r"%\1#", line) for line in theirs] != blank:
            problem(f"shape '{name}': GCC's template is {theirs}, lccc's {ours}")
    return len(SHAPES)


# ----------------------------------------------------------- diagnostics

INCONSISTENT = "inconsistent operand constraints in an 'asm'"

# source of a function, flags, what lccc says (None: accepts), whether GCC
# refuses it too. Where the two differ the reason is the comment.
DIAGNOSTICS = [
    ('void f(double v) { __asm__("nop" : : "k"(v)); }', "-mavx512f", INCONSISTENT, True),
    ('void f(__int128 v) { __asm__("nop" : : "k"(v)); }', "-mavx512f", INCONSISTENT, True),
    ('void f(__m128i v) { __asm__("nop" : : "k"(v)); }', "-mavx512f", INCONSISTENT, True),
    ('void f(__m512i v) { __asm__("nop" : : "Yk"(v)); }', "-mavx512f", INCONSISTENT, True),
    ('void f(__m256i v) { __asm__("nop" : : "r"(v)); }', "-mavx2", INCONSISTENT, True),
    ('void f(__m256i v) { __asm__("nop" : : "y"(v)); }', "-mavx2", INCONSISTENT, True),
    ('void f(__m128i v) { __asm__("nop" : : "y"(v)); }', "-mavx2", INCONSISTENT, True),
    ('void f(short v) { __asm__("nop" : : "y"(v)); }', "-mmmx", INCONSISTENT, True),
    ('void f(double v) { __asm__("nop" : : "y"(v)); }', "-mmmx", INCONSISTENT, True),
    ('void f(int v) { __asm__("nop" : : "y"(v)); }', "-mmmx", None, False),
    ('void f(long long v) { __asm__("nop" : : "y"(v)); }', "-mmmx", None, False),
    ('void f(void *v) { __asm__("nop" : : "k"(v)); }', "-mavx512bw", None, False),
    ('void f(_Bool v) { __asm__("nop" : : "k"(v)); }', "-mavx512f", None, False),
    # GCC moves a float into an MMX register; nothing was found that does.
    ('void f(float v) { __asm__("nop" : : "y"(v)); }', "-mmmx", INCONSISTENT, False),
    # The stores stand behind the template, where a jump does not come by.
    (
        'int f(__m256i v) { __m256i r; __asm__ goto("vmovdqa %1, %0\\n\\tjmp %l2"'
        ' : "=x"(r) : "x"(v) : : out); return 0; out: return 1; }',
        "-mavx2",
        "'asm goto' output operand 0 cannot be in a vector, mask or MMX register",
        False,
    ),
    # The moves go through a general register, which the variable has not.
    (
        'unsigned short f(unsigned short a) { register unsigned short m __asm__("k1") = a;'
        ' unsigned short r; __asm__("kmovw %1, %0" : "=k"(r) : "k"(m)); return r; }',
        "-mavx512f",
        "'asm' operand 1: a register variable cannot be a mask or MMX operand",
        False,
    ),
    # Seventeen operands and sixteen registers.
    (
        "void f(__m256i *p) { __asm__(\"nop\" : : "
        + ", ".join(f'"x"(p[{i}])' for i in range(17))
        + "); }",
        "-mavx2",
        "'asm' operand has impossible constraints or there are not enough registers",
        True,
    ),
    # GCC asks the function's instruction sets whether the class holds the
    # type: 32 and 64 bits of mask want AVX-512BW, a 256-bit vector AVX.
    # lccc does not ask: the registers are named by the statement, whose
    # instructions need the extension whatever the compiler thinks of it.
    ('void f(unsigned long long v) { __asm__("nop" : : "k"(v)); }', "-mavx512f", None, True),
    (
        'typedef int v8 __attribute__((vector_size(32)));'
        ' void f(v8 *p) { __asm__("nop" : : "x"(*p)); }',
        "-msse2",
        None,
        True,
    ),
]


def check_diagnostics(lccc: str, gcc: str | None, tmp: str) -> int:
    for index, (body, flags, want, gcc_refuses) in enumerate(DIAGNOSTICS):
        source = os.path.join(tmp, f"diag{index}.c")
        with open(source, "w") as handle:
            handle.write(HEADER + body + "\n")
        out = os.path.join(tmp, f"diag{index}.s")
        done = run([lccc, "-O2", *flags.split(), "-S", source, "-o", out])
        errors = [line.split("error: ", 1)[1] for line in done.stderr.split("\n")
                  if "error: " in line]
        if want is None:
            if done.returncode != 0:
                problem(f"diagnostic {index}: lccc refuses `{body}`: {errors}")
        elif done.returncode == 0 or want not in errors:
            problem(f"diagnostic {index}: `{body}`: expected '{want}', got {errors}")
        if gcc:
            theirs = run([gcc, "-O2", *flags.split(), "-S", source, "-o", out])
            if (theirs.returncode != 0) != gcc_refuses:
                problem(
                    f"diagnostic {index}: GCC {'accepts' if gcc_refuses else 'refuses'}"
                    f" `{body}`: {theirs.stderr.strip()[:200]}"
                )
            elif gcc_refuses and want == INCONSISTENT and INCONSISTENT not in theirs.stderr:
                problem(f"diagnostic {index}: GCC's words differ: {theirs.stderr.strip()[:200]}")
    return len(DIAGNOSTICS)


# ---------------------------------------------------------------- values

YMM_OPS = ["vpaddd", "vpsubd", "vpxor", "vpand", "vpor", "vpandn", "vpmulld", "vpminsd",
           "vpmaxsd", "vpcmpeqd", "vpcmpgtd", "vpsllvd", "vpsrlvd", "vpminud", "vpmaxud"]
ZMM_OPS = ["vpaddd", "vpsubd", "vpxord", "vpandd", "vpord", "vpandnd", "vpmulld", "vpminsd",
           "vpmaxsd", "vpsllvd", "vpsrlvd", "vpminud", "vpmaxud", "vprolvd", "vprorvd"]
MASK_OPS = {
    16: ("unsigned short", ["kandw", "korw", "kxorw", "kandnw", "kxnorw"]),
    32: ("unsigned", ["kandd", "kord", "kxord", "kandnd", "kxnord"]),
    64: ("unsigned long long", ["kandq", "korq", "kxorq", "kandnq", "kxnorq"]),
}


def vector_function(rng: random.Random, name: str, width: int) -> str:
    """One function `void NAME(int *o, const int *a, const int *b, unsigned m)`."""
    ops = YMM_OPS if width == 256 else ZMM_OPS
    vector = f"__m{width}i"
    load = "_mm256_loadu_si256((const __m256i *)%s)" if width == 256 else "_mm512_loadu_si512(%s)"
    store = ("_mm256_storeu_si256((__m256i *)o, %s);" if width == 256
             else "_mm512_storeu_si512(o, %s);")
    op, second = rng.choice(ops), rng.choice(ops)
    klass = rng.choice(["x", "v"]) if width == 512 else "x"
    forms = ["plain", "tied", "read_write", "memory", "two_steps", "named", "rvalue"]
    if width == 512:
        forms += ["maskz", "merge", "compare", "maskz_rvalue"]
    form = rng.choice(forms)
    head = f"static __attribute__((noinline)) void {name}(int *o, const int *a, const int *b, unsigned m)\n{{\n"
    head += f"    {vector} va = {load % 'a'}, vb = {load % 'b'}, r = va;\n    (void)m;\n"
    if form == "plain":
        asm = f'"{op} %2, %1, %0" : "={klass}"(r) : "{klass}"(va), "{klass}"(vb)'
    elif form == "tied":
        asm = f'"{op} %2, %0, %0" : "={klass}"(r) : "0"(va), "{klass}"(vb)'
    elif form == "read_write":
        asm = f'"{op} %1, %0, %0" : "+{klass}"(r) : "{klass}"(vb)'
    elif form == "memory":
        asm = f'"{op} %2, %1, %0" : "={klass}"(r) : "{klass}"(va), "{klass}m"(vb)'
    elif form == "two_steps":
        asm = (f'"{op} %2, %1, %0\\n\\t{second} %1, %0, %0"'
               f' : "=&{klass}"(r) : "{klass}"(va), "{klass}"(vb)')
    elif form == "named":
        asm = (f'"{op} %[right], %[left], %[out]"'
               f' : [out] "={klass}"(r) : [left] "{klass}"(va), [right] "{klass}"(vb)')
    elif form == "rvalue":
        add = "_mm256_add_epi32" if width == 256 else "_mm512_add_epi32"
        asm = f'"{op} %2, %1, %0" : "={klass}"(r) : "{klass}"({add}(va, vb)), "{klass}"(vb)'
    elif form == "maskz":
        asm = (f'"{op} %2, %1, %0%{{%3%}}%{{z%}}"'
               f' : "={klass}"(r) : "{klass}"(va), "{klass}"(vb), "Yk"((unsigned short)m)')
    elif form == "maskz_rvalue":
        shift = rng.randrange(1, 9)
        asm = (f'"{op} %2, %1, %0%{{%3%}}%{{z%}}"'
               f' : "={klass}"(r) : "{klass}"(va), "{klass}"(vb),'
               f' "Yk"((unsigned short)(m >> {shift}))')
    elif form == "merge":
        asm = (f'"{op} %3, %2, %0%{{%4%}}"'
               f' : "={klass}"(r) : "0"(vb), "{klass}"(va), "{klass}"(vb), "Yk"((unsigned short)m)')
    else:
        predicate = rng.randrange(8)
        body = head + "    unsigned short k;\n"
        body += (f'    __asm__("vpcmpd ${predicate}, %2, %1, %0"'
                 f' : "=k"(k) : "{klass}"(va), "{klass}"(vb));\n')
        body += f"    {store % 'va'}\n    o[0] = k;\n}}\n"
        return body
    return head + f"    __asm__({asm});\n    {store % 'r'}\n}}\n"


def mask_function(rng: random.Random, name: str) -> str:
    bits = rng.choice([16, 16, 32, 64])
    ctype, ops = MASK_OPS[bits]
    op, second = rng.choice(ops), rng.choice(ops)
    form = rng.choice(["plain", "tied", "read_write", "two_steps", "rvalue"])
    head = (f"static __attribute__((noinline)) unsigned long long {name}"
            "(unsigned long long x, unsigned long long y)\n{\n"
            f"    {ctype} a = ({ctype})x, b = ({ctype})y, r = a;\n")
    if form == "plain":
        asm = f'"{op} %2, %1, %0" : "=k"(r) : "k"(a), "k"(b)'
    elif form == "tied":
        asm = f'"{op} %2, %0, %0" : "=k"(r) : "0"(a), "k"(b)'
    elif form == "read_write":
        asm = f'"{op} %1, %0, %0" : "+k"(r) : "k"(b)'
    elif form == "two_steps":
        asm = f'"{op} %2, %1, %0\\n\\t{second} %1, %0, %0" : "=&k"(r) : "k"(a), "k"(b)'
    else:
        asm = f'"{op} %2, %1, %0" : "=k"(r) : "k"(({ctype})(x >> 3)), "k"(({ctype})(y + x))'
    return head + f"    __asm__({asm});\n    return r;\n}}\n"


def value_program(seed: int, count: int, with_zmm: bool) -> str:
    rng = random.Random(seed)
    parts = ["#include <immintrin.h>\n#include <stdio.h>\n"]
    calls: list[str] = []
    for index in range(count):
        parts.append(vector_function(rng, f"y{index}", 256))
        calls.append(f"    if (avx2) {{ y{index}(o, a, b, m); sum(\"y{index}\", o, 8); }}")
        if with_zmm:
            parts.append(vector_function(rng, f"z{index}", 512))
            calls.append(f"    if (avx512) {{ z{index}(o, a, b, m); sum(\"z{index}\", o, 16); }}")
        parts.append(mask_function(rng, f"k{index}"))
        calls.append(f"    if (avx512) printf(\"k{index} %llx\\n\","
                     f" k{index}(0x{rng.getrandbits(64):x}ULL, 0x{rng.getrandbits(64):x}ULL));")
    parts.append(
        "static void sum(const char *name, const int *o, int n)\n{\n"
        "    unsigned long long h = 1469598103934665603ULL;\n"
        "    for (int i = 0; i < n; i++) h = (h ^ (unsigned)o[i]) * 1099511628211ULL;\n"
        "    printf(\"%s %llx\\n\", name, h);\n}\n"
    )
    parts.append(
        "int main(void)\n{\n"
        "    int avx2 = __builtin_cpu_supports(\"avx2\");\n"
        "    int avx512 = __builtin_cpu_supports(\"avx512f\") && __builtin_cpu_supports(\"avx512bw\");\n"
        "    int a[16], b[16], o[16];\n"
        "    unsigned state = 0x2545F491u, m = 0;\n"
        "    for (int i = 0; i < 16; i++) {\n"
        "        state = state * 1664525u + 1013904223u; a[i] = (int)state;\n"
        "        state = state * 1664525u + 1013904223u; b[i] = (int)(state >> (i % 7));\n"
        "        m = m * 31u + state;\n"
        "    }\n"
        + "\n".join(calls)
        + "\n    return 0;\n}\n"
    )
    # `sum` is used before its definition: declare it first.
    parts.insert(1, "static void sum(const char *name, const int *o, int n);\n")
    return "".join(parts)


ZERO_EXTENSION = r"""
#include <stdio.h>
__attribute__((noinline)) static unsigned widen(unsigned long long x)
{
    unsigned r;
    __asm__("kmovw %1, %0" : "=r"(r) : "k"((unsigned char)x));
    return r;
}
__attribute__((noinline)) static unsigned long long junk(unsigned long long x) { return x * 3 + 1; }
int main(void)
{
    if (!__builtin_cpu_supports("avx512f")) return 0;
    unsigned long long x = junk(0x5555AAAA5555AAA9ULL);
    unsigned got = widen(x);
    if (got != (unsigned char)x) { printf("FAIL %x\n", got); return 1; }
    return 0;
}
"""


def can_run_32(gcc: str | None, tmp: str) -> bool:
    if not gcc:
        return False
    source = os.path.join(tmp, "probe32.c")
    with open(source, "w") as handle:
        handle.write("int main(void) { return 0; }\n")
    binary = os.path.join(tmp, "probe32")
    if run([gcc, "-m32", source, "-o", binary]).returncode != 0:
        return False
    try:
        return run([binary]).returncode == 0
    except OSError:
        return False


def check_values(lccc: str, gcc: str | None, tmp: str) -> int:
    statements = 0
    source = os.path.join(tmp, "zero.c")
    with open(source, "w") as handle:
        handle.write(ZERO_EXTENSION)
    for level in ("-O0", "-O2"):
        binary = os.path.join(tmp, "zero")
        done = run([lccc, level, "-mavx512f", source, "-o", binary])
        if done.returncode != 0:
            problem(f"zero extension {level}: lccc fails: {done.stderr.strip()[:300]}")
        else:
            ran = run([binary])
            if ran.returncode != 0:
                problem(f"zero extension {level}: {ran.stdout.strip()} (exit {ran.returncode})")
    if not gcc:
        return statements
    modes = [("", True)]
    if can_run_32(gcc, tmp):
        # The i686 back end has no encodings for the 512-bit intrinsics the
        # program loads and stores with; the 256-bit and mask statements run.
        modes.append(("-m32 -msse2", False))
    for mode, with_zmm in modes:
        for seed in (20261127, 7, 99):
            count = 40
            program = value_program(seed, count, with_zmm)
            source = os.path.join(tmp, f"values{seed}.c")
            with open(source, "w") as handle:
                handle.write(program)
            flags = ["-mavx2", "-mavx512f", "-mavx512bw", *mode.split()]
            reference = os.path.join(tmp, "values.gcc")
            done = run([gcc, "-O2", *flags, source, "-o", reference])
            if done.returncode != 0:
                problem(f"values seed {seed} {mode}: GCC refuses the program:"
                        f" {done.stderr.strip()[:400]}")
                continue
            want = run([reference])
            if want.returncode != 0:
                problem(f"values seed {seed} {mode}: GCC's program fails ({want.returncode})")
                continue
            statements += count * (3 if with_zmm else 2)
            for level in ("-O0", "-O1", "-O2", "-Os"):
                binary = os.path.join(tmp, "values.lccc")
                done = run([lccc, level, *flags, source, "-o", binary])
                if done.returncode != 0:
                    problem(f"values seed {seed} {mode} {level}: lccc fails:"
                            f" {done.stderr.strip()[:400]}")
                    continue
                got = run([binary])
                if got.returncode != 0 or got.stdout != want.stdout:
                    theirs = want.stdout.split("\n")
                    ours = got.stdout.split("\n")
                    wrong = [f"{o} (GCC: {t})" for o, t in zip(ours, theirs) if o != t][:5]
                    problem(f"values seed {seed} {mode} {level}: exit {got.returncode},"
                            f" {len(ours)} lines against {len(theirs)}; first differences: {wrong}")
    return statements


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--lccc", required=True)
    parser.add_argument("--gcc", help="GCC 16.2: the reference for text, words and values")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="lccc-asmreg.") as tmp:
        shapes = check_shapes(args.lccc, args.gcc, tmp)
        diagnostics = check_diagnostics(args.lccc, args.gcc, tmp)
        statements = check_values(args.lccc, args.gcc, tmp)
    if PROBLEMS:
        for text in PROBLEMS:
            print(f"FAIL: {text}", file=sys.stderr)
        print(f"x86 asm register operands: {len(PROBLEMS)} problem(s)", file=sys.stderr)
        return 1
    against = f", {statements} generated statements equal to GCC's" if args.gcc else ""
    print(f"x86 asm register operands: PASS ({shapes} shapes, {diagnostics} diagnostics{against})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
