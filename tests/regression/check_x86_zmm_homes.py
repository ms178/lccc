#!/usr/bin/env python3
"""512-bit vector objects that live in `zmm16`..`zmm31` (x86-64).

A 512-bit value used to be 64 bytes of stack behind a `leaq`/`andq $-64`,
and every copy the inliner left behind was memory traffic: a loop of six
operations ran 42 times slower than GCC 16.2's (2.57 s against 0.061 s).
`backend::x86_zmm_homes` now keeps an object that is only read and written
whole in a register, and the loop takes 0.060 s.
This gate holds it to:

1. shapes: the vector and mask instructions of a few functions, line for
   line, and nothing in them that touches the stack. A chain of operations
   is one instruction per operation, a copy between two names of a value
   is none, and the body of a loop has no stack traffic in it;
2. the contract the homes rest on: with the homes switched off
   (`CCC_DISABLE_PASSES=zmmhome`) nothing lccc emits names a register above
   15, so nothing but a home ever writes one. This is looked for in the
   generated programs and in every test of the corpus that is built with
   AVX-512 or a `-march=`;
3. the homes that are decided are the homes that are used: no stage behind
   the decision adds a use that makes the code generator give them up
   (it says so under `CCC_VERIFY_IR`);
4. values: generated programs (chains, loops, branches, calls that scribble
   over every register, more than sixteen values alive at once, objects
   whose address is taken, vectors passed and returned by value), built at
   every optimisation level and with the homes off, and run. Every build
   prints the same numbers, and they are GCC's where GCC is given.

The programs run only on a CPU with AVX-512 F, BW, DQ, VL and VNNI and
FMA, which is what they are built for (/proc/cpuinfo): the options are the
compiler's for the whole file, so what a program checks before its first
vector function is no protection. Elsewhere part 4 builds them, with GCC
as well, and says that it did no more.

Usage: check_x86_zmm_homes.py --lccc BIN [--gcc gcc-16] [--programs N] [--seed N]
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
ISA = ["-mavx512f", "-mavx512bw", "-mavx512dq", "-mavx512vl"]
HIGH = re.compile(r"%[xyz]mm(1[6-9]|2[0-9]|3[01])\b")
MEMORY = re.compile(r"-?\d*\([^)]*\)")
STACK = re.compile(r"\(%r[sb]p|andq \$-64|\bpush|\bpop|subq \$\d+, %rsp")


def problem(text: str) -> None:
    PROBLEMS.append(text)


def run(command: list[str], **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(command, capture_output=True, text=True, **kwargs)


def without_homes() -> dict[str, str]:
    env = dict(os.environ)
    env["CCC_DISABLE_PASSES"] = "zmmhome"
    return env


def telling() -> dict[str, str]:
    """The compiler says when it gives a function's homes up."""
    env = dict(os.environ)
    env["CCC_VERIFY_IR"] = "1"
    return env


def cpu_flags() -> set[str]:
    """The feature flags of this CPU, by the names Linux gives them."""
    try:
        with open("/proc/cpuinfo") as handle:
            for line in handle:
                if line.startswith("flags"):
                    return set(line.split(":", 1)[1].split())
    except OSError:
        pass
    return set()


# ---------------------------------------------------------------- shapes

HEADER = "#include <immintrin.h>\n"

# name, source of a function `f`, its vector and mask instructions (the
# lines that begin with `v` or `k`), in order. MEM is a memory operand: how
# an address is formed is not what a shape is about.
SHAPES = [
    (
        "a chain is one instruction per operation",
        "void f(int *d, const int *x, const int *y) {\n"
        "    __m512i a = _mm512_loadu_si512(x), b = _mm512_loadu_si512(y);\n"
        "    _mm512_storeu_si512(d, _mm512_xor_si512(_mm512_mullo_epi32("
        "_mm512_add_epi32(a, b), b), _mm512_sub_epi32(b, a)));\n"
        "}\n",
        [
            "vmovdqu64 MEM, %zmm16",
            "vmovdqu64 MEM, %zmm17",
            "vpaddd %zmm17, %zmm16, %zmm18",
            "vpmulld %zmm17, %zmm18, %zmm18",
            "vpsubd %zmm16, %zmm17, %zmm16",
            "vpxorq %zmm16, %zmm18, %zmm16",
            "vmovdqu64 %zmm16, MEM",
        ],
    ),
    (
        "a copy between two names of one value is no instruction",
        "void f(int *d, const int *x) {\n"
        "    __m512i a = _mm512_loadu_si512(x);\n"
        "    __m512i b = a, c = b;\n"
        "    _mm512_storeu_si512(d, _mm512_add_epi32(c, a));\n"
        "}\n",
        [
            "vmovdqu64 MEM, %zmm16",
            "vpaddd %zmm16, %zmm16, %zmm16",
            "vmovdqu64 %zmm16, MEM",
        ],
    ),
    (
        "a shift, a shuffle and a broadcast name the register",
        "void f(int *d, const int *x, int k) {\n"
        "    __m512i a = _mm512_loadu_si512(x);\n"
        "    a = _mm512_srli_epi32(a, 3);\n"
        "    a = _mm512_shuffle_epi32(a, 0x1b);\n"
        "    a = _mm512_add_epi32(a, _mm512_set1_epi32(k));\n"
        "    _mm512_storeu_si512(d, a);\n"
        "}\n",
        [
            "vmovdqu64 MEM, %zmm16",
            "vpsrld $3, %zmm16, %zmm16",
            "vpshufd $27, %zmm16, %zmm16",
            "vpbroadcastd %edx, %zmm17",
            "vpaddd %zmm17, %zmm16, %zmm16",
            "vmovdqu64 %zmm16, MEM",
        ],
    ),
    (
        "a compare into a mask reads both registers",
        "void f(int *m, const int *x, const int *y) {\n"
        "    __m512i a = _mm512_loadu_si512(x), b = _mm512_loadu_si512(y);\n"
        "    *m = _mm512_cmpeq_epi32_mask(_mm512_add_epi32(a, b), b);\n"
        "}\n",
        [
            "vmovdqu64 MEM, %zmm16",
            "vmovdqu64 MEM, %zmm17",
            "vpaddd %zmm17, %zmm16, %zmm16",
            "vpcmpd $0, %zmm17, %zmm16, %k1",
            "kmovw %k1, %eax",
        ],
    ),
    (
        "three sources: the result replaces the one it is tied to",
        "void f(int *d, const int *x, const int *y, const int *z) {\n"
        "    __m512i a = _mm512_loadu_si512(x), b = _mm512_loadu_si512(y),"
        " c = _mm512_loadu_si512(z);\n"
        "    _mm512_storeu_si512(d, _mm512_ternarylogic_epi32(a, b, c, 0x96));\n"
        "}\n",
        [
            "vmovdqu64 MEM, %zmm16",
            "vmovdqu64 MEM, %zmm17",
            "vmovdqu64 MEM, %zmm18",
            "vpternlogd $150, %zmm18, %zmm17, %zmm16",
            "vmovdqu64 %zmm16, MEM",
        ],
    ),
    (
        "three sources: a tied source that lives on is copied, not lost",
        "void f(int *d, const int *x, const int *y, const int *z) {\n"
        "    __m512i a = _mm512_loadu_si512(x), b = _mm512_loadu_si512(y),"
        " c = _mm512_loadu_si512(z);\n"
        "    __m512i r = _mm512_ternarylogic_epi32(a, b, c, 0x96);\n"
        "    _mm512_storeu_si512(d, _mm512_add_epi32(r, a));\n"
        "}\n",
        [
            "vmovdqu64 MEM, %zmm16",
            "vmovdqu64 MEM, %zmm17",
            "vmovdqu64 MEM, %zmm18",
            "vmovdqa64 %zmm16, %zmm19",
            "vpternlogd $150, %zmm18, %zmm17, %zmm19",
            "vpaddd %zmm16, %zmm19, %zmm16",
            "vmovdqu64 %zmm16, MEM",
        ],
    ),
]

# The loop of the measurement in the module's header: its body, from the
# label the back edge jumps to.
LOOP = (
    "void f(int *restrict dst, const int *restrict x, const int *restrict y, long n) {\n"
    "    for (long i = 0; i + 16 <= n; i += 16) {\n"
    "        __m512i a = _mm512_loadu_si512((const void *)(x + i));\n"
    "        __m512i b = _mm512_loadu_si512((const void *)(y + i));\n"
    "        __m512i s = _mm512_add_epi32(a, b);\n"
    "        __m512i m = _mm512_mullo_epi32(s, b);\n"
    "        __m512i d = _mm512_sub_epi32(b, a);\n"
    "        __m512i r = _mm512_xor_si512(m, d);\n"
    "        r = _mm512_max_epi32(r, _mm512_srli_epi32(a, 3));\n"
    "        _mm512_storeu_si512((void *)(dst + i), r);\n"
    "    }\n"
    "}\n"
)
LOOP_VECTOR = [
    "vmovdqu64", "vmovdqu64", "vpaddd", "vpmulld", "vpsubd", "vpxorq", "vpsrld", "vpmaxsd",
    "vmovdqu64",
]
# Nine of them are the vector instructions. The rest is the loop itself,
# which is not this module's: the index is scaled by hand and the bound is
# added again for the test (GCC 16.2 has three instructions there).
LOOP_LIMIT = 14


def instructions(assembly: str, function: str) -> list[str] | None:
    """The instructions of `function`, labels kept, directives dropped."""
    lines = assembly.splitlines()
    try:
        start = lines.index(f"{function}:")
    except ValueError:
        return None
    body = []
    for line in lines[start + 1:]:
        text = line.strip()
        if text.startswith(".size") or text == ".cfi_endproc":
            break
        if not text or text.startswith("#") or text.startswith(".cfi") or text.startswith(".p2align"):
            continue
        body.append(re.sub(r"\s+", " ", text))
    return body


def compile_text(lccc: str, source: str, flags: list[str], env=None) -> str | None:
    with tempfile.TemporaryDirectory(prefix="zmmhome.") as directory:
        path = os.path.join(directory, "t.c")
        out = os.path.join(directory, "t.s")
        with open(path, "w") as handle:
            handle.write(source)
        done = run([lccc, *flags, "-S", "-o", out, path], env=env)
        if done.returncode != 0:
            problem(f"compile failed ({' '.join(flags)}):\n{done.stderr.strip()}\n{source}")
            return None
        with open(out) as handle:
            return handle.read()


def check_shapes(lccc: str) -> int:
    checked = 0
    for name, source, expected in SHAPES:
        text = compile_text(lccc, HEADER + source, ["-O2", *ISA])
        if text is None:
            continue
        body = instructions(text, "f")
        if body is None:
            problem(f"shape '{name}': no function f in the output")
            continue
        vector = [MEMORY.sub("MEM", line) for line in body if line[0] in "vk"]
        if vector != expected:
            problem(
                f"shape '{name}':\n  expected:\n    " + "\n    ".join(expected)
                + "\n  got:\n    " + "\n    ".join(vector)
            )
        stack = [line for line in body if STACK.search(line)]
        if stack:
            problem(f"shape '{name}': stack traffic:\n    " + "\n    ".join(stack))
        checked += 1

    text = compile_text(lccc, HEADER + LOOP, ["-O2", *ISA])
    if text is not None:
        body = instructions(text, "f") or []
        back = [at for at, line in enumerate(body) if re.match(r"j[a-z]+ \.L", line)]
        labels = {line[:-1]: at for at, line in enumerate(body) if line.endswith(":")}
        loop = None
        for at in back:
            target = body[at].split()[-1]
            if target in labels and labels[target] < at:
                loop = body[labels[target] + 1: at + 1]
        if loop is None:
            problem("loop: no backward jump in:\n    " + "\n    ".join(body))
        else:
            vector = [line.split()[0] for line in loop if line.startswith("v")]
            stack = [line for line in loop if STACK.search(line)]
            if vector != LOOP_VECTOR:
                problem(f"loop: vector instructions {vector}, expected {LOOP_VECTOR}")
            if stack:
                problem("loop: stack traffic in the body:\n    " + "\n    ".join(stack))
            if len(loop) > LOOP_LIMIT:
                problem(f"loop: {len(loop)} instructions in the body, limit {LOOP_LIMIT}:\n    "
                        + "\n    ".join(loop))
        checked += 1
    return checked


# --------------------------------------------------------------- corpus


def outside_asm(assembly: str) -> list[str]:
    """The lines lccc wrote itself: not what stands between #APP and #NO_APP."""
    lines = []
    inside = False
    for line in assembly.splitlines():
        text = line.strip()
        if text.startswith("#APP"):
            inside = True
        elif text.startswith("#NO_APP"):
            inside = False
        elif not inside and text and not text.startswith(("#", ".")):
            lines.append(text)
    return lines


def check_corpus(lccc: str) -> int:
    """With the homes off, no x86-64 test of the corpus that can have EVEX
    code in it has a register above 15 in what lccc wrote."""
    directory = os.path.dirname(os.path.abspath(__file__))
    checked = 0
    with tempfile.TemporaryDirectory(prefix="zmmhome.") as scratch:
        listing = os.path.join(scratch, "t.s")
        for name in sorted(os.listdir(directory)):
            if not name.endswith(".flags"):
                continue
            with open(os.path.join(directory, name)) as handle:
                flags = handle.read().split()
            source = os.path.join(directory, name[:-len(".flags")] + ".c")
            if (not os.path.exists(source) or "-m32" in flags or any("@" in flag for flag in flags)
                    or not any("avx512" in flag or flag.startswith("-march=") for flag in flags)):
                continue
            done = run([lccc, "-O2", source, *flags, "-S", "-o", listing], env=without_homes())
            if done.returncode != 0:
                continue
            with open(listing) as handle:
                high = [line for line in outside_asm(handle.read()) if HIGH.search(line)]
            if high:
                problem(f"corpus {name[:-6]}.c: a register above 15 with the homes off:\n    "
                        + "\n    ".join(high[:6]))
            checked += 1
    return checked


# ------------------------------------------------------------- programs

BINARY = [
    "_mm512_add_epi32", "_mm512_sub_epi32", "_mm512_mullo_epi32", "_mm512_and_si512",
    "_mm512_or_si512", "_mm512_xor_si512", "_mm512_andnot_si512", "_mm512_max_epi32",
    "_mm512_min_epi32", "_mm512_max_epu32", "_mm512_min_epu32", "_mm512_add_epi64",
    "_mm512_sub_epi64", "_mm512_add_epi16", "_mm512_sub_epi8", "_mm512_adds_epi16",
    "_mm512_subs_epu8", "_mm512_avg_epu8", "_mm512_mullo_epi16", "_mm512_mulhi_epi16",
    "_mm512_madd_epi16", "_mm512_maddubs_epi16", "_mm512_sad_epu8", "_mm512_shuffle_epi8",
    "_mm512_unpacklo_epi32", "_mm512_unpackhi_epi16", "_mm512_unpacklo_epi64",
    "_mm512_packs_epi32", "_mm512_packus_epi16", "_mm512_mul_epu32", "_mm512_max_epi64", "_mm512_min_epu64", "_mm512_permutexvar_epi32",
    "_mm512_permutexvar_epi64",
]
UNARY = ["_mm512_abs_epi32", "_mm512_abs_epi8", "_mm512_abs_epi64", "_mm512_abs_epi16"]
SHIFT = [
    "_mm512_slli_epi32", "_mm512_srli_epi32", "_mm512_srai_epi32", "_mm512_slli_epi16",
    "_mm512_srli_epi64", "_mm512_srai_epi64", "_mm512_srai_epi16", "_mm512_slli_epi64",
]
MASK_COMPARE = [
    "_mm512_cmpeq_epi32_mask", "_mm512_cmpeq_epi8_mask", "_mm512_cmpeq_epi16_mask",
    "_mm512_cmpeq_epi64_mask",
]

PRELUDE = r"""
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

typedef unsigned long long u64;
static u64 state = 0x9e3779b97f4a7c15ull;
static u64 next(void) { state ^= state << 13; state ^= state >> 7; state ^= state << 17; return state; }

static int in[1024] __attribute__((aligned(64)));
static int out[1024] __attribute__((aligned(64)));
static float fin[256] __attribute__((aligned(64)));
static float fout[256] __attribute__((aligned(64)));
static int calls;

/* Whoever is called may write every vector register. This one does. */
__attribute__((noinline)) void scribble(int k) {
    calls += k;
    __asm__ volatile(
        "vpternlogd $0xff, %%zmm16, %%zmm16, %%zmm16\n\tvpternlogd $0xff, %%zmm17, %%zmm17, %%zmm17\n\t"
        "vpternlogd $0xff, %%zmm18, %%zmm18, %%zmm18\n\tvpternlogd $0xff, %%zmm19, %%zmm19, %%zmm19\n\t"
        "vpternlogd $0xff, %%zmm20, %%zmm20, %%zmm20\n\tvpternlogd $0xff, %%zmm21, %%zmm21, %%zmm21\n\t"
        "vpternlogd $0xff, %%zmm22, %%zmm22, %%zmm22\n\tvpternlogd $0xff, %%zmm23, %%zmm23, %%zmm23\n\t"
        "vpternlogd $0xff, %%zmm24, %%zmm24, %%zmm24\n\tvpternlogd $0xff, %%zmm25, %%zmm25, %%zmm25\n\t"
        "vpternlogd $0xff, %%zmm26, %%zmm26, %%zmm26\n\tvpternlogd $0xff, %%zmm27, %%zmm27, %%zmm27\n\t"
        "vpternlogd $0xff, %%zmm28, %%zmm28, %%zmm28\n\tvpternlogd $0xff, %%zmm29, %%zmm29, %%zmm29\n\t"
        "vpternlogd $0xff, %%zmm30, %%zmm30, %%zmm30\n\tvpternlogd $0xff, %%zmm31, %%zmm31, %%zmm31\n\t"
        "vpternlogd $0xff, %%zmm0, %%zmm0, %%zmm0\n\tvpternlogd $0xff, %%zmm1, %%zmm1, %%zmm1\n\t"
        "vpternlogd $0xff, %%zmm2, %%zmm2, %%zmm2\n\tvpternlogd $0xff, %%zmm3, %%zmm3, %%zmm3\n\t"
        "vpternlogd $0xff, %%zmm4, %%zmm4, %%zmm4\n\tvpternlogd $0xff, %%zmm5, %%zmm5, %%zmm5\n\t"
        "vpternlogd $0xff, %%zmm6, %%zmm6, %%zmm6\n\tvpternlogd $0xff, %%zmm7, %%zmm7, %%zmm7\n\t"
        "vzeroupper"
        : : : "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7",
              "xmm16", "xmm17", "xmm18", "xmm19", "xmm20", "xmm21", "xmm22", "xmm23",
              "xmm24", "xmm25", "xmm26", "xmm27", "xmm28", "xmm29", "xmm30", "xmm31");
}

/* An object whose address goes somewhere has to be memory. */
__attribute__((noinline)) void touch(__m512i *p, int k) {
    int lanes[16];
    memcpy(lanes, p, sizeof lanes);
    lanes[k & 15] ^= 0x5a5a5a5a + k;
    memcpy(p, lanes, sizeof lanes);
}

/* By value in and out, across a real call. */
__attribute__((noinline)) __m512i mix(__m512i a, __m512i b) {
    return _mm512_sub_epi32(_mm512_xor_si512(a, b), _mm512_srli_epi32(b, 5));
}

/* By value in and out, inlined: the copies are the ones the homes remove. */
static inline __m512i blend(__m512i a, __m512i b, __m512i c) {
    __m512i t = _mm512_and_si512(a, c);
    __m512i u = _mm512_andnot_si512(c, b);
    return _mm512_or_si512(t, u);
}

static u64 sum_out(void) {
    u64 h = 1469598103934665603ull;
    for (int i = 0; i < 1024; i++) { h ^= (unsigned)out[i]; h *= 1099511628211ull; }
    for (int i = 0; i < 256; i++) { unsigned bits; memcpy(&bits, &fout[i], 4); h ^= bits; h *= 1099511628211ull; }
    return h;
}
"""


class Body:
    """One generated function: statements over `count` 512-bit variables."""

    def __init__(self, rng: random.Random, count: int, floats: int):
        self.rng = rng
        self.count = count
        self.floats = floats
        self.lines: list[str] = []
        self.depth = 1
        self.loops = 0
        self.scalars = 0

    def emit(self, text: str) -> None:
        self.lines.append("    " * self.depth + text)

    def v(self) -> str:
        return f"v{self.rng.randrange(self.count)}"

    def f(self) -> str:
        return f"f{self.rng.randrange(self.floats)}"

    def offset(self) -> int:
        return self.rng.randrange(0, 1008)

    def scalar(self) -> str:
        choice = self.rng.randrange(4)
        if choice == 0:
            return f"in[{self.rng.randrange(1024)}]"
        if choice == 1 and self.scalars:
            return f"s{self.rng.randrange(self.scalars)}"
        if choice == 2:
            return f"(int)n * {self.rng.randrange(1, 9)}"
        return str(self.rng.randrange(-70000, 70000))

    def statement(self) -> None:
        rng = self.rng
        kind = rng.choices(
            ["binary", "unary", "shift", "shuffle", "ternary", "alignr", "set1", "zero", "load",
             "store", "copy", "mask", "call", "touch", "mix", "blend", "if", "loop", "float",
             "array", "dot"],
            weights=[30, 4, 8, 3, 5, 2, 5, 1, 8, 8, 6, 5, 3, 2, 3, 4, 5, 4, 5, 3, 2],
        )[0]
        if kind == "binary":
            self.emit(f"{self.v()} = {rng.choice(BINARY)}({self.v()}, {self.v()});")
        elif kind == "unary":
            self.emit(f"{self.v()} = {rng.choice(UNARY)}({self.v()});")
        elif kind == "shift":
            self.emit(f"{self.v()} = {rng.choice(SHIFT)}({self.v()}, {rng.randrange(0, 40)});")
        elif kind == "shuffle":
            name = rng.choice(["_mm512_shuffle_epi32", "_mm512_shufflehi_epi16", "_mm512_shufflelo_epi16"])
            self.emit(f"{self.v()} = {name}({self.v()}, {rng.randrange(256)});")
        elif kind == "ternary":
            name = rng.choice(["_mm512_ternarylogic_epi32", "_mm512_ternarylogic_epi64"])
            self.emit(f"{self.v()} = {name}({self.v()}, {self.v()}, {self.v()}, {rng.randrange(256)});")
        elif kind == "alignr":
            self.emit(f"{self.v()} = _mm512_alignr_epi8({self.v()}, {self.v()}, {rng.randrange(0, 20)});")
        elif kind == "set1":
            name, cast = rng.choice([("_mm512_set1_epi32", "int"), ("_mm512_set1_epi8", "char"),
                                     ("_mm512_set1_epi16", "short")])
            self.emit(f"{self.v()} = {name}(({cast})({self.scalar()}));")
        elif kind == "zero":
            self.emit(f"{self.v()} = _mm512_setzero_si512();")
        elif kind == "load":
            self.emit(f"{self.v()} = _mm512_loadu_si512((const void *)(in + {self.offset()}));")
        elif kind == "store":
            self.emit(f"_mm512_storeu_si512((void *)(out + {self.offset()}), {self.v()});")
        elif kind == "copy":
            self.emit(f"{self.v()} = {self.v()};")
        elif kind == "mask":
            name = f"s{self.scalars}"
            self.scalars += 1
            self.lines.insert(1, f"    int {name} = 0;")
            self.emit(f"{name} = (int){rng.choice(MASK_COMPARE)}({self.v()}, {self.v()});")
        elif kind == "call":
            self.emit(f"scribble({rng.randrange(1, 9)});")
        elif kind == "touch":
            self.emit(f"touch(&{self.v()}, {rng.randrange(64)});")
        elif kind == "mix":
            self.emit(f"{self.v()} = mix({self.v()}, {self.v()});")
        elif kind == "blend":
            self.emit(f"{self.v()} = blend({self.v()}, {self.v()}, {self.v()});")
        elif kind == "float":
            form = rng.randrange(5)
            if form == 0:
                self.emit(f"{self.f()} = _mm512_loadu_ps(fin + {rng.randrange(0, 240)});")
            elif form == 1:
                self.emit(f"_mm512_storeu_ps(fout + {rng.randrange(0, 240)}, {self.f()});")
            elif form == 2:
                name = rng.choice(["_mm512_add_ps", "_mm512_sub_ps", "_mm512_mul_ps"])
                self.emit(f"{self.f()} = {name}({self.f()}, {self.f()});")
            elif form == 3:
                name = rng.choice(["_mm512_fmadd_ps", "_mm512_fmsub_ps", "_mm512_fnmadd_ps",
                                   "_mm512_fnmsub_ps", "_mm512_fmaddsub_ps", "_mm512_fmsubadd_ps"])
                self.emit(f"{self.f()} = {name}({self.f()}, {self.f()}, {self.f()});")
            else:
                self.emit(f"{self.f()} = _mm512_cvtepi32_ps({self.v()});")
        elif kind == "array":
            slot = rng.randrange(4)
            if rng.randrange(2):
                self.emit(f"arr[{slot}] = {self.v()};")
            else:
                self.emit(f"{self.v()} = arr[{slot}];")
        elif kind == "dot":
            name = rng.choice(["_mm512_dpbusd_epi32", "_mm512_dpbusds_epi32"])
            self.emit(f"{self.v()} = {name}({self.v()}, {self.v()}, {self.v()});")
        elif kind == "if" and self.depth < 4:
            condition = rng.choice([
                f"(in[{rng.randrange(1024)}] & {1 << rng.randrange(4)})",
                f"(n & {1 << rng.randrange(3)})",
                f"(_mm512_cmpeq_epi32_mask({self.v()}, {self.v()}) & {1 << rng.randrange(16)})",
            ])
            self.emit(f"if {condition} {{")
            self.depth += 1
            for _ in range(rng.randrange(1, 6)):
                self.statement()
            self.depth -= 1
            if rng.randrange(2):
                self.emit("} else {")
                self.depth += 1
                for _ in range(rng.randrange(1, 6)):
                    self.statement()
                self.depth -= 1
            self.emit("}")
        elif kind == "loop" and self.depth < 3 and self.loops < 3:
            index = f"i{self.loops}"
            self.loops += 1
            self.emit(f"for (long {index} = 0; {index} < {rng.randrange(1, 5)} + (n & 3); {index}++) {{")
            self.depth += 1
            for _ in range(rng.randrange(2, 8)):
                self.statement()
            if rng.randrange(3) == 0:
                self.emit(f"if ((in[{index} & 1023] & 7) == 5) break;")
            self.depth -= 1
            self.emit("}")
        else:
            self.emit(f"{self.v()} = _mm512_add_epi32({self.v()}, {self.v()});")


def program(seed: int) -> str:
    rng = random.Random(seed)
    functions = []
    names = []
    for index in range(rng.randrange(2, 5)):
        # Most functions stay under sixteen values; some go well over.
        count = rng.choice([3, 5, 8, 12, 16, 20, 28])
        floats = rng.choice([1, 2, 4])
        body = Body(rng, count, floats)
        body.lines.append(f"__attribute__((noinline)) void k{index}(long n) {{")
        body.lines.append("    __m512i arr[4];")
        for slot in range(4):
            body.emit(f"arr[{slot}] = _mm512_loadu_si512((const void *)(in + {64 * slot + index}));")
        for variable in range(count):
            body.emit(f"__m512i v{variable} = _mm512_loadu_si512((const void *)(in + {(variable * 16 + index * 7) % 1008}));")
        for variable in range(floats):
            body.emit(f"__m512 f{variable} = _mm512_loadu_ps(fin + {(variable * 16 + index) % 240});")
        for _ in range(rng.randrange(8, 60)):
            body.statement()
        # Everything reaches the output, so nothing is dead.
        for variable in range(count):
            body.emit(f"_mm512_storeu_si512((void *)(out + {(variable * 16 + 3) % 1008}),"
                      f" _mm512_xor_si512(v{variable},"
                      f" _mm512_loadu_si512((const void *)(out + {(variable * 16 + 3) % 1008}))));")
        for variable in range(floats):
            body.emit(f"_mm512_storeu_ps(fout + {variable * 16}, f{variable});")
        for slot in range(4):
            body.emit(f"_mm512_storeu_si512((void *)(out + {900 + 16 * slot}), arr[{slot}]);")
        for scalar in range(body.scalars):
            body.emit(f"out[{scalar % 1024}] += s{scalar};")
        body.lines.append("}")
        functions.append("\n".join(body.lines))
        names.append(f"k{index}")
    main = [
        "int main(void) {",
        "    if (!__builtin_cpu_supports(\"avx512f\") || !__builtin_cpu_supports(\"avx512bw\")",
        "        || !__builtin_cpu_supports(\"avx512dq\") || !__builtin_cpu_supports(\"avx512vnni\")) {",
        "        puts(\"no avx512\");",
        "        return 0;",
        "    }",
        "    for (int i = 0; i < 1024; i++) in[i] = (int)next();",
        "    for (int i = 0; i < 256; i++) fin[i] = (float)((int)(next() % 2001) - 1000) / 8.0f;",
        "    for (long n = 0; n < 6; n++) {",
    ]
    for name in names:
        main.append(f"        {name}(n);")
        main.append(f"        printf(\"{name} %ld %llx %d\\n\", n, sum_out(), calls);")
    main += ["    }", "    return 0;", "}"]
    return PRELUDE + "\n" + "\n\n".join(functions) + "\n\n" + "\n".join(main) + "\n"


CONFIGURATIONS = [
    ("-O0", ["-O0"], False),
    ("-O1", ["-O1"], False),
    ("-O2", ["-O2"], False),
    ("-O3", ["-O3"], False),
    ("-Os", ["-Os"], False),
    ("-O2 -g", ["-O2", "-g"], False),
    ("-O2 -fno-omit-frame-pointer -fstack-protector-all",
     ["-O2", "-fno-omit-frame-pointer", "-fstack-protector-all"], False),
    ("-O2, homes off", ["-O2"], True),
]
PROGRAM_ISA = [*ISA, "-mavx512vnni", "-mfma", "-ffp-contract=off"]
# What a CPU needs to run a program built with PROGRAM_ISA.
PROGRAM_CPU = {"avx512f", "avx512bw", "avx512dq", "avx512vl", "avx512_vnni", "fma"}


def check_programs(lccc: str, gcc: str | None, count: int, seed: int,
                   execute: bool) -> tuple[int, int, int]:
    built = ran = homes = 0
    said = False
    if not execute:
        print("  values: this CPU lacks one of AVX-512 F, BW, DQ, VL, VNNI and FMA; the "
              "programs are built, not run")
        said = True
    with tempfile.TemporaryDirectory(prefix="zmmhome.") as directory:
        source = os.path.join(directory, "p.c")
        binary = os.path.join(directory, "p")
        for index in range(count):
            text = program(seed + index)
            with open(source, "w") as handle:
                handle.write(text)
            keep = os.path.join(directory, f"failed-{seed + index}.c")

            # Part 2 on every program: nothing but a home is above 15. The
            # one function that names them by hand is taken out.
            listing = os.path.join(directory, "p.s")
            done = run([lccc, "-O2", *PROGRAM_ISA, "-S", "-o", listing, source], env=without_homes())
            if done.returncode != 0:
                problem(f"program {seed + index}: compile failed, homes off:\n{done.stderr.strip()}")
                continue
            with open(listing) as handle:
                off = handle.read()
            for name in re.findall(r"^(k\d+):$", off, re.M):
                body = instructions(off, name) or []
                high = [line for line in body if HIGH.search(line)]
                if high:
                    problem(f"program {seed + index}: {name} names a register above 15 with the "
                            f"homes off:\n    " + "\n    ".join(high[:6]))
            # The homes that were decided are the homes that are used: no
            # later stage has added a use that makes the compiler give a
            # function's homes up.
            for level in ("-O1", "-O2", "-O3", "-Os"):
                done = run([lccc, level, *PROGRAM_ISA, "-S", "-o", listing, source], env=telling())
                if done.returncode != 0:
                    continue
                for line in done.stderr.splitlines():
                    if line.startswith("[zmmhome]"):
                        problem(f"program {seed + index} at {level}: {line}")
                        with open(keep, "w") as handle:
                            handle.write(text)
                if level == "-O2":
                    with open(listing) as handle:
                        homes += len(HIGH.findall(handle.read()))

            reference = None
            if gcc:
                done = run([gcc, "-O2", *PROGRAM_ISA, "-o", binary, source])
                if done.returncode != 0:
                    problem(f"program {seed + index}: {gcc} refuses it:\n{done.stderr.strip()[:2000]}")
                    continue
                if execute:
                    result = run([binary])
                    reference = (f"{gcc} -O2", result.stdout)
                    if result.returncode != 0:
                        problem(f"program {seed + index}: {gcc}'s binary exits {result.returncode}")
                        continue
            for name, flags, off_switch in CONFIGURATIONS:
                if os.path.exists(binary):
                    os.remove(binary)
                done = run([lccc, *flags, *PROGRAM_ISA, "-o", binary, source],
                           env=without_homes() if off_switch else None)
                if done.returncode != 0:
                    problem(f"program {seed + index}: compile failed at {name}:\n"
                            f"{done.stderr.strip()[:2000]}")
                    with open(keep, "w") as handle:
                        handle.write(text)
                    continue
                if done.stderr.strip():
                    problem(f"program {seed + index}: diagnostics at {name}:\n{done.stderr.strip()[:800]}")
                built += 1
                if not execute:
                    continue
                result = run([binary])
                if result.returncode != 0:
                    problem(f"program {seed + index}: exit {result.returncode} at {name}")
                    with open(keep, "w") as handle:
                        handle.write(text)
                    continue
                if result.stdout.startswith("no avx512"):
                    if not said:
                        print("  values: this CPU has no AVX-512 F/BW/DQ/VNNI; the programs are "
                              "compiled, not run")
                        said = True
                    continue
                ran += 1
                if reference is None:
                    reference = (name, result.stdout)
                elif result.stdout != reference[1]:
                    want = reference[1].splitlines()
                    got = result.stdout.splitlines()
                    first = next((at for at, (a, b) in enumerate(zip(want, got)) if a != b),
                                 min(len(want), len(got)))
                    problem(
                        f"program {seed + index}: {name} differs from {reference[0]} at line "
                        f"{first + 1}:\n    {reference[0]}: {want[first] if first < len(want) else '(end)'}"
                        f"\n    {name}: {got[first] if first < len(got) else '(end)'}"
                    )
                    with open(keep, "w") as handle:
                        handle.write(text)
        kept = sorted(name for name in os.listdir(directory) if name.startswith("failed-"))
        if kept:
            target = os.path.join(tempfile.gettempdir(), "zmmhome-failures")
            os.makedirs(target, exist_ok=True)
            for name in kept:
                os.replace(os.path.join(directory, name), os.path.join(target, name))
            print(f"  the failing programs are in {target}")
    return built, ran, homes


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    parser.add_argument("--lccc", required=True)
    parser.add_argument("--gcc")
    parser.add_argument("--programs", type=int, default=24)
    parser.add_argument("--seed", type=int, default=20261127)
    parser.add_argument("--cpu-flags", metavar="FLAGS",
                        help="the CPU's feature flags, instead of /proc/cpuinfo's: what part 4 "
                             "does on a machine with these")
    arguments = parser.parse_args()
    cpu = cpu_flags() if arguments.cpu_flags is None else set(arguments.cpu_flags.split())

    shapes = check_shapes(arguments.lccc)
    corpus = check_corpus(arguments.lccc)
    if corpus < 50:
        problem(f"only {corpus} tests of the corpus were looked at: the selection is broken")
    built, ran, homes = check_programs(arguments.lccc, arguments.gcc, arguments.programs,
                                       arguments.seed, PROGRAM_CPU <= cpu)
    if homes == 0:
        problem("no generated program has a register above 15 in it: the homes are not in use")

    if PROBLEMS:
        for text in PROBLEMS:
            print(f"FAIL: {text}")
        print(f"check_x86_zmm_homes: {len(PROBLEMS)} problem(s)")
        return 1
    against = f", equal to {arguments.gcc}'s" if arguments.gcc and ran else ""
    print(f"check_x86_zmm_homes: PASS ({shapes} shapes; {corpus} corpus tests without a register "
          f"above 15; {arguments.programs} programs, {built} builds, {ran} runs{against}; "
          f"{homes} uses of zmm16..zmm31)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
