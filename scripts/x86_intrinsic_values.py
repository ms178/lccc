#!/usr/bin/env python3
"""Does every x86 intrinsic lccc offers compute what GCC's computes?

Why
---
`x86_intrinsic_census.py` counts the intrinsics a program can *call* under
lccc.  A call that compiles may still be wrong code, and a wrong intrinsic
is the worst kind of defect: nothing is refused, nothing crashes, and the
program takes another path or prints another number.  (`_xgetbv`, which
runtime dispatch reads the operating system's AVX state with, returned 0
for years: every program that asked concluded there is no AVX.)  The
hand-written suites test the intrinsics somebody thought of.  This tool
tests the ones lccc has.

How
---
For every intrinsic of GCC 16.2's headers that both compilers accept for
the host (`-march=native`), a test is written from the header's own types:

    void v7(void) { lv_begin("_mm256_add_ps");
      for (unsigned q = 0; q < 48; q++) {
        __m256 a0; lv_fill(&a0, sizeof a0, 1); __m256 a1; lv_fill(&a1, sizeof a1, 1);
        __m256 r = _mm256_add_ps(a0, a1); lv_mix(&r, sizeof r); }
      lv_end(); }

`lv_fill` writes arguments of the type's kind: integers of every lane width
with the values arithmetic breaks at (0, -1, the largest and smallest of
the lane, small counts), floating point with integers, halves, infinities,
denormals and one quiet NaN.  `lv_mix` hashes what comes back, and what was
written through every pointer that is not to `const`.  The same source is
compiled by GCC and by lccc, linked against the same runtime (compiled by
GCC, so that a miscompiled harness cannot hide in both), and run on this
processor.  A test whose hash differs is a finding.

Which parameter has to be a constant is not guessed: a caller that passes
a variable is offered to GCC, and where GCC refuses it, the smallest set of
constants it accepts is found.  Such a parameter then gets a spread of
values, each a test of its own (`_mm_shuffle_epi32#27`), and GCC is asked
which of them are valid.

Not everything can be compared.  What reads the clock, the random number
generator or an address is run and not compared (`RUN_ONLY`); what changes
the state the program runs in is not run (`NOT_RUN`); a test that GCC's own
code dies in (an instruction this processor lacks or user mode may not
execute) is dropped and counted.  Where the bits are undefined (the upper
half of `_mm256_castps128_ps256`) only the defined part is hashed.

An immediate is an integer constant expression, not a literal: tests of
every intrinsic that takes one spell it `-(-N)` (`_mm_shuffle_epi32#27~`)
and have to compute what the literal computes.

And where GCC wants a constant, lccc is asked for the same call with a value
of the running program (`_mm_shuffle_epi32#27?`). GCC refuses that call, and
lccc may: with a diagnostic. If it takes the call it has to compute what the
constant computes. What it must never do is assemble the instruction with
whatever stands for "no constant" (that was `$0`), or stop with an internal
error.

And where GCC refuses the constant itself (`_mm_blend_ps(a, b, 16)`: "the
last argument must be a 4-bit immediate"), lccc has to refuse it too, where
the call stands. A few such constants are asked per intrinsic: the one next
to the range, 256 and -1. An instruction that ignores the bits it has no
use for would compute something for them, and a program GCC and Clang
refuse would compile here and nowhere else; an assembler that stops at the
operand has no line of the program to name.

An argument is converted to its parameter's type, and in the tests above no
argument needs it: each is an object of exactly that type. So every
intrinsic with a scalar parameter is called twice more with its scalar
arguments in objects of another arithmetic type. `_mm_insert_epi32@` passes
a `double` where the element is an `int` (the value with three quarters
beside it, which the conversion truncates away) and a `long long` where a
parameter is floating; `_mm_insert_epi32&` passes an integer of another
width: a `long long` whose upper half the conversion has to drop where the
parameter is narrower, an `int` or a `signed char` it has to extend where
the parameter is 64 bits. Both compilers read the same line, and the hash
is GCC's. (lccc's headers wrote some intrinsics as macros over a builtin
without a prototype, and what was passed was what was written: the element
of `_mm_insert_epi32(v, d, 1)` was the low half of the double.)

`--also clang-23` adds a second reference: a test the references do not
agree on is reported as theirs, not as lccc's. `--as AS` (GNU as 2.47) holds
what `lccc -S` writes for the tests to the assembler everybody else uses.

    scripts/x86_intrinsic_values.py --lccc target/fastbuild/lccc --check
    scripts/x86_intrinsic_values.py --lccc target/fastbuild/lccc --only avx2intrin.h -v

`--check` fails when a test differs, when lccc's code dies where GCC's does
not, when lccc refuses a test of an intrinsic it has, when lccc stops with an
internal error on any call, when GNU as refuses the assembly lccc writes
for a test, or when lccc takes a constant GCC refuses (or refuses it without
naming the call).

What is tested is what this processor executes, so the count depends on
the machine (1,239 intrinsics on an AVX-512 Xeon, fewer without AVX-512).
The count is printed and is no part of the verdict.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import math
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from scripts import x86_intrinsic_census as census  # noqa: E402

ROUNDS = 48
BUFFER = 256
# What FXSAVE and the XSAVE family write is the state the processor has, not
# an operand's width: 512 bytes for FXSAVE, and for XSAVE whatever the mask
# selects of what CPUID leaf 0DH counts (2,696 bytes with AVX-512, 10,944
# with AMX tiles). Their buffer is larger than any of that.
STATE_AREA = re.compile(r"^_f?x(save|rstor)")
STATE_BUFFER = 16384
TARGET = ["-march=native", "-ffp-contract=off"]
HEAD = ("#include <immintrin.h>\n#include <x86intrin.h>\n"
        "void lv_begin(const char *, const char *); void lv_end(void);\n"
        "void lv_fill(void *, unsigned long, int); void lv_mix(const void *, unsigned long);\n")

# Values an immediate is tried with; GCC says which of them the intrinsic takes.
IMMEDIATES = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 16, 0x1B, 0x20, 0x31, 0x4E,
              0x55, 0x7F, 0x80, 0xAA, 0xE4, 0xFF,
              # Past the byte an immediate is: GCC refuses these for most
              # intrinsics and takes them for some (a shuffle's `int`, a
              # shift's `unsigned` count), and what it takes it defines.
              -1, 256, 0x1E4]
ROUNDINGS = [4, 8, 9, 10, 11]
SCALES = [1, 2, 4, 8]
# How many tests one intrinsic may be when it has more than one immediate.
MOST_VARIANTS = 40
# How many of them are also written with the immediates as expressions.
EXPRESSION_TWINS = 3
# How a scalar argument is written when it is not an object of its
# parameter's type (the two conversion twins of a test, in this order).
CONVERSIONS = ("@", "&")
# Integer parameter types, each with the cast under which every value is one
# a `double` holds exactly with three quarters beside it and that the type
# holds again once the fraction is truncated away.
CARRIED_INTEGERS = {
    "char": "", "short": "", "int": "", "unsigned char": "", "unsigned short": "",
    "unsigned int": "", "unsigned": "", "__mmask8": "", "__mmask16": "", "__mmask32": "",
    "long long": "(int)", "long long int": "(int)",
    "unsigned long long": "(unsigned)", "__mmask64": "(unsigned)",
}
# The same types by what an integer of another width is for each: the
# unsigned type of its own width for the narrow ones (a `long long` carries
# the value under an upper half the conversion drops), and a narrower signed
# type for the 64-bit ones (the conversion extends its sign).
CARRIED_NARROW = {
    "char": "unsigned char", "short": "unsigned short", "int": "unsigned",
    "unsigned char": "unsigned char", "unsigned short": "unsigned short",
    "unsigned int": "unsigned", "unsigned": "unsigned", "__mmask8": "unsigned char",
    "__mmask16": "unsigned short", "__mmask32": "unsigned",
}
CARRIED_WIDE = {
    "long long": "int", "long long int": "int",
    "unsigned long long": "signed char", "__mmask64": "signed char",
}
CARRIED_FLOATS = ("float", "double")
# Constants far from every range: of the choices GCC refuses, the ones with
# these are asked of lccc, beside the one next to the range.
FAR = (256, -1)
# Constants tried where GCC wants one and its place is being looked for: a
# value most selectors, shifts and scales take, and the two rounding modes.
PROBES = (1, 8, 4, 0)
# What a reference says of an intrinsic it has no code for on this target or
# in this mode: a statement about the intrinsic, none about the call written.
ELSEWHERE = re.compile(r"target specific option mismatch|needs isa option"
                       r"|(needs|requires) target feature"
                       r"|implicit declaration of function|undeclared")
# After so many rounds in a row in which the compiler names one line, for the
# target, and stops (GCC ends at its first `always_inline` mismatch), every
# line left is asked on its own.
ONE_AT_A_TIME = 3

# Run, the result not compared: it is the clock, the random number generator,
# an address, or the state of the registers.
RUN_ONLY = re.compile(
    r"^(_rdrand\d+_step|_rdseed\d+_step|__rdtscp?|_rdtscp?|_rdpid_u32|_readfsbase_u(32|64)|"
    r"_readgsbase_u(32|64)|__readeflags|_fxsave(64)?|_xsave(c|opt|s)?(64)?|_get_ssp|__rdsspd|"
    r"__rdsspq|_mm_getexcept)$")
# Not run: it changes the state the program runs in, or is no user-mode instruction.
NOT_RUN = re.compile(
    r"^(_writefsbase_u(32|64)|_writegsbase_u(32|64)|_xsetbv|_wrpkru|_xrstors?(64)?|_fxrstor(64)?|"
    r"__writeeflags|_mm_malloc|_mm_free|_xbegin|_xend|_xabort|_mm_monitorx?|_mm_mwaitx?|"
    r"__rdpmc|_rdpmc|_umonitor|_umwait|_tpause|_hreset|_senduipi|_stui|_clui|_testui|"
    r"_wbinvd|_wbnoinvd|_serialize|_ptwrite(32|64)|_enc(ls|lu|lv)_u32|_pconfig_u32|"
    r"__llwpcb|__slwpcb|__lwpval(32|64)|__lwpins(32|64)|_(u)?(rd|wr)msr\w*|_wrmsrns\w*|"
    r"_xsusldtrk|_xresldtrk|_(inc|set|clr|rstor|saveprev)ssp\w*|_wr(u)?ss(d|q)|_setssbsy|"
    r"_clrssbsy|_rstorssp|_saveprevssp|__incssp(d|q)|__rstorssp|__saveprevssp|__setssbsy|"
    r"__clrssbsy|__wr(u)?ss(d|q)|_tile_\w+|_mm_loadiwkey|_mm_\w*kl_u8|_mm_encodekey\w+|"
    r"_enqcmds?|_movdir64b|_directstoreu_u(32|64)|_cldemote|_m_prefetchit[01]|_mm_clzero)$")
# The upper part of what these return is undefined: bytes that are defined.
DEFINED_BYTES = (
    (re.compile(r"cast(ps|pd|si)128_(ps|pd|si)(256|512)$"), 16),
    (re.compile(r"cast(ps|pd|si)256_(ps|pd|si)512$"), 32),
    (re.compile(r"castph128_ph(256|512)$"), 16),
    (re.compile(r"castph256_ph512$"), 32),
)
UNDEFINED = re.compile(r"_undefined(_|$)")
# Undefined for zero, as BSF and BSR are.
NONZERO = re.compile(r"^(_bit_scan_(forward|reverse)|__bs[fr][dq])$")
# Arguments only some values of which are defined: the statement that makes
# the argument `a0` one of them. (MXCSR faults on a reserved bit, and an
# unmasked exception would deliver a signal to the test.)
ARGUMENT_RULES = {
    "_xgetbv": "a0 = 0;",
    "_mm_setcsr": "a0 = 0x1f80 | ((a0 & 3) << 13) | (a0 & 0x8040);",
    "_MM_SET_EXCEPTION_MASK": "a0 = 0x1f80;",
    "_MM_SET_EXCEPTION_STATE": "a0 &= 0x3f;",
    "_MM_SET_ROUNDING_MODE": "a0 &= 0x6000;",
    "_MM_SET_FLUSH_ZERO_MODE": "a0 &= 0x8000;",
    "_MM_SET_DENORMALS_ZERO_MODE": "a0 &= 0x0040;",
}

RUNTIME = r"""
/* The runtime of scripts/x86_intrinsic_values.py: arguments in, a hash out.
   Compiled by the reference compiler for every program. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <xmmintrin.h>

extern void (*const lv_tests[])(void);
extern const unsigned lv_count;

static unsigned long long lv_state, lv_hash;
static const char *lv_name;

static unsigned long long lv_next(void)
{
    unsigned long long z = (lv_state += 0x9E3779B97F4A7C15ull);
    z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9ull;
    z = (z ^ (z >> 27)) * 0x94D049BB133111EBull;
    return z ^ (z >> 31);
}

void lv_begin(const char *name, const char *seed)
{
    unsigned long long h = 1469598103934665603ull;
    for (const char *p = seed; *p; p++)
        h = (h ^ (unsigned char)*p) * 1099511628211ull;
    lv_state = h;
    lv_hash = 1469598103934665603ull;
    lv_name = name;
    _mm_setcsr(0x1f80);
}

void lv_end(void)
{
    _mm_setcsr(0x1f80);
    printf("%s %016llx\n", lv_name, lv_hash);
    fflush(stdout);
}

void lv_mix(const void *p, unsigned long n)
{
    const unsigned char *b = p;
    for (unsigned long i = 0; i < n; i++)
        lv_hash = (lv_hash ^ b[i]) * 1099511628211ull;
    lv_hash = (lv_hash ^ n) * 1099511628211ull;
}

static unsigned long long lv_integer(unsigned bits)
{
    unsigned long long r = lv_next(), ones = bits == 64 ? ~0ull : (1ull << bits) - 1;
    switch (r >> 60) {
    case 10: return 0;
    case 11: return ones;
    case 12: return 1ull << (bits - 1);
    case 13: return ones >> 1;
    case 14: return (r >> 8) % 67;
    case 15: return 1ull << ((r >> 8) % bits);
    default: return r & ones;
    }
}

static float lv_float(void)
{
    static const unsigned special[] = {
        0x00000000u, 0x80000000u, 0x7f800000u, 0xff800000u, 0x7fc00000u, 0x7f7fffffu,
        0xff7fffffu, 0x00800000u, 0x00000001u, 0x807fffffu, 0x4f000000u, 0xcf000000u,
        0x4effffffu, 0x4f800000u, 0x5f000000u, 0xdf000000u, 0x5f800000u, 0x3f000000u,
        0xbf000000u, 0x3f800000u, 0xbf800000u, 0x4b800000u, 0x3effffffu, 0x4b000000u,
    };
    unsigned long long r = lv_next();
    unsigned bits;
    float f;
    switch (r >> 60) {
    case 7: case 8:
        return (float)((long)((r >> 8) % 2001) - 1000);
    case 9:
        return (float)((long)((r >> 8) % 2001) - 1000) + 0.5f;
    case 10:
        bits = (unsigned)(r >> 8);
        if ((bits & 0x7f800000u) == 0x7f800000u)
            bits &= 0xff800000u;
        break;
    case 11:
        bits = special[(r >> 8) % (sizeof special / sizeof special[0])];
        break;
    default:
        bits = ((unsigned)(r & 1) << 31) | ((107 + (unsigned)((r >> 1) % 41)) << 23)
               | ((unsigned)(r >> 8) & 0x7fffffu);
        break;
    }
    memcpy(&f, &bits, sizeof f);
    return f;
}

static double lv_double(void)
{
    static const unsigned long long special[] = {
        0x0000000000000000ull, 0x8000000000000000ull, 0x7ff0000000000000ull,
        0xfff0000000000000ull, 0x7ff8000000000000ull, 0x7fefffffffffffffull,
        0xffefffffffffffffull, 0x0010000000000000ull, 0x0000000000000001ull,
        0x800fffffffffffffull, 0x41e0000000000000ull, 0xc1e0000000000000ull,
        0x41dfffffffc00000ull, 0x41f0000000000000ull, 0x43e0000000000000ull,
        0xc3e0000000000000ull, 0x43f0000000000000ull, 0x3fe0000000000000ull,
        0xbfe0000000000000ull, 0x3ff0000000000000ull, 0xbff0000000000000ull,
        0x4340000000000000ull, 0x3fdfffffffffffffull, 0x4330000000000000ull,
        0x47efffffe0000000ull, 0x47f0000000000000ull, 0x36a0000000000000ull,
    };
    unsigned long long r = lv_next(), bits;
    double d;
    switch (r >> 60) {
    case 7: case 8:
        return (double)((long)((r >> 8) % 2001) - 1000);
    case 9:
        return (double)((long)((r >> 8) % 2001) - 1000) + 0.5;
    case 10:
        bits = lv_next();
        if ((bits & 0x7ff0000000000000ull) == 0x7ff0000000000000ull)
            bits &= 0xfff0000000000000ull;
        break;
    case 11:
        bits = special[(r >> 8) % (sizeof special / sizeof special[0])];
        break;
    default:
        bits = ((r & 1) << 63) | ((1003 + ((r >> 1) % 41)) << 52)
               | (lv_next() & 0xfffffffffffffull);
        break;
    }
    memcpy(&d, &bits, sizeof d);
    return d;
}

/* kind 0: integers, 1: float lanes, 2: double lanes, 3: small indexes. */
void lv_fill(void *p, unsigned long n, int kind)
{
    unsigned char *b = p;
    unsigned long i = 0;
    if (kind == 1) {
        for (; i + 4 <= n; i += 4) {
            float f = lv_float();
            memcpy(b + i, &f, 4);
        }
    } else if (kind == 2) {
        for (; i + 8 <= n; i += 8) {
            double d = lv_double();
            memcpy(b + i, &d, 8);
        }
    } else if (kind == 3) {
        for (; i + 8 <= n; i += 8) {
            unsigned long long v = lv_next() % 8;
            memcpy(b + i, &v, 8);
        }
    } else {
        unsigned long width = 1ul << (lv_next() % 4);
        while (width > n)
            width >>= 1;
        for (; i + width <= n; i += width) {
            unsigned long long v = lv_integer(8 * (unsigned)width);
            memcpy(b + i, &v, width);
        }
    }
    for (; i < n; i++)
        b[i] = (unsigned char)lv_next();
}

int main(int argc, char **argv)
{
    unsigned char *skip = calloc(lv_count + 1, 1);
    if (!skip)
        return 2;
    if (argc > 1) {
        FILE *f = fopen(argv[1], "r");
        unsigned i;
        if (!f)
            return 2;
        while (fscanf(f, "%u", &i) == 1)
            if (i < lv_count)
                skip[i] = 1;
        fclose(f);
    }
    for (unsigned i = 0; i < lv_count; i++) {
        if (skip[i]) {
            puts("-");
            fflush(stdout);
            continue;
        }
        lv_tests[i]();
    }
    return 0;
}
"""


class Test:
    """One intrinsic with one choice of immediates."""

    def __init__(self, name: str, returns: str, parameters: list[tuple[str, str]],
                 constants: dict[int, int], spelling: str = ""):
        self.name, self.returns, self.parameters, self.constants = (
            name, returns, parameters, constants)
        # How the immediates are written: "" the literal, "~" the expression
        # `-(-N)`, "?" an object the running program reads N from. Or how the
        # scalar arguments are: "@" and "&", objects of another type
        # (`carrier_of`), beside literal immediates.
        self.spelling = spelling

    @property
    def label(self) -> str:
        if not self.constants:
            return self.name + self.spelling
        return (self.name + "#" + ",".join(str(self.constants[i]) for i in sorted(self.constants))
                + self.spelling)

    @property
    def seed(self) -> str:
        """What the arguments are drawn from: the label without the spelling."""
        return self.label.rstrip("~?@&")

    def carried(self) -> list[int]:
        """The positions whose argument this spelling passes in another type."""
        return [position for position, (kind, _) in enumerate(self.parameters)
                if position not in self.constants and "*" not in kind
                and carrier_of(kind, position, self.spelling) is not None]

    def line(self, index: int) -> str:
        """The test as one line of C: a compiler's line number names the test."""
        name = self.name
        setup, passed, after, statics = [], [], [], []
        for position, (kind, spelled) in enumerate(self.parameters):
            if position in self.constants:
                value = self.constants[position]
                if self.spelling == "?":
                    statics.append(f"static volatile {kind.replace('const', '').strip()} "
                                   f"c{position} = {value};")
                    passed.append(f"c{position}")
                else:
                    passed.append(f"-(-{value})" if self.spelling == "~" else str(value))
            elif "*" in kind:
                offset = 1 if re.search(r"loadu|storeu|lddqu", name) else 0
                size = buffer_of(name)
                statics.append(f"static unsigned char b{position}[{size}] "
                               f"__attribute__((aligned(64)));")
                setup.append(f"lv_fill(b{position}, {size}, {fill_kind(kind)});")
                passed.append(f"({kind})(void *)(b{position} + {offset})")
                if not re.search(r"\bconst\b", kind):
                    after.append(f"lv_mix(b{position}, {size});")
            else:
                plain = re.sub(r"\bconst\b", "", kind).strip()
                fill = 3 if spelled == "__index" else fill_kind(plain)
                setup.append(f"{plain} a{position}; lv_fill(&a{position}, sizeof a{position}, "
                             f"{fill});")
                if NONZERO.match(name):
                    setup.append(f"a{position} |= 1;")
                if name in ARGUMENT_RULES and position == 0:
                    setup.append(ARGUMENT_RULES[name])
                # The argument as the rules above left it, in an object of
                # another type: the call has to convert it back.
                carrier = carrier_of(plain, position, self.spelling)
                if carrier is None:
                    passed.append(f"a{position}")
                else:
                    setup.append(carrier)
                    passed.append(f"x{position}")
        call = f"{name}({', '.join(passed)})"
        returns = re.sub(r"\bconst\b", "", self.returns).strip()
        if returns == "void":
            body = f"{call};"
        elif RUN_ONLY.match(name):
            body = f"{returns} r = {call}; (void)r;"
            after = []
        else:
            size = next((str(n) for pattern, n in DEFINED_BYTES if pattern.search(name)),
                        "sizeof r")
            body = f"{returns} r = {call}; lv_mix(&r, {size});"
        return (f"void v{index}(void) {{ {' '.join(statics)} lv_begin(\"{self.label}\", "
                f"\"{self.seed}\"); "
                f"for (unsigned q = 0; q < {ROUNDS}; q++) {{ {' '.join(setup)} {body} "
                f"{' '.join(after)} }} lv_end(); }}")


def buffer_of(name: str) -> int:
    """The bytes behind a pointer argument of `name`."""
    return STATE_BUFFER if STATE_AREA.match(name) else BUFFER


def carrier_of(kind: str, position: int, spelling: str) -> str | None:
    """The statement that puts argument `a<position>` of type `kind` into an
    object `x<position>` of another arithmetic type, or None where the
    spelling has no other type for the parameter.

    "@": an integer in a `double`, three quarters past it away from zero, so
    that only a conversion toward zero gives it back (a 64-bit one by its low
    half, which a `double` holds exactly); a floating value as a `long long`
    (an integer of its own, drawn here: an integer cannot carry a NaN).
    "&": an integer in an integer of another width. A narrow one in a `long
    long` under an upper half of alternating bits, which the conversion
    drops; a 64-bit one as an `int` or a `signed char`, which it extends.
    """
    kind = " ".join(re.sub(r"\bconst\b", " ", kind).split())
    a, x = f"a{position}", f"x{position}"
    if spelling == "@":
        if kind in CARRIED_INTEGERS:
            return (f"double {x} = (double){CARRIED_INTEGERS[kind]}{a}; "
                    f"{x} += {x} < 0 ? -0.75 : 0.75;")
        if kind in CARRIED_FLOATS:
            return (f"short s{position}; lv_fill(&s{position}, sizeof s{position}, 0); "
                    f"long long {x} = s{position};")
    elif spelling == "&":
        if kind in CARRIED_NARROW:
            return (f"long long {x} = (long long)((unsigned long long)({CARRIED_NARROW[kind]}){a}"
                    f" | 0x5a5a5a5a5a5a5a5aull << 8 * sizeof {a});")
        if kind in CARRIED_WIDE:
            return f"{CARRIED_WIDE[kind]} {x} = ({CARRIED_WIDE[kind]}){a};"
    return None


def conversion_twins(test: Test) -> list[Test]:
    """`test` once per way of passing its scalar arguments in another type,
    for the ways that change an argument of it."""
    twins = [Test(test.name, test.returns, test.parameters, dict(test.constants), spelling)
             for spelling in CONVERSIONS]
    return [twin for twin in twins if twin.carried()]


def fill_kind(kind: str) -> int:
    """What `lv_fill` writes for a value of this type."""
    base = " ".join(kind.replace("const", " ").replace("*", " ").split())
    if re.fullmatch(r"__m(128|256|512)d(_u)?|double", base):
        return 2
    if re.fullmatch(r"__m(128|256|512)(_u)?|float", base):
        return 1
    return 0


def split_parameters(parameters: list[str]) -> list[tuple[str, str]]:
    return [census.parameter_of(parameter) for parameter in parameters]


def may_be_immediate(kind: str) -> bool:
    return kind.replace("const", "").strip() in census.IMMEDIATE_TYPES


def candidates_for(name: str, spelled: str) -> list[int]:
    if spelled in ("__R", "__B_R", "__ROUND") or (spelled.endswith("R") and "round" in name):
        return ROUNDINGS
    if spelled == "__scale":
        return SCALES
    return IMMEDIATES


DIAGNOSTIC = re.compile(r": (fatal error|error|warning|note): (.*)")
MENTION = re.compile(r"c\.c:(\d+)[:,)]")
# The lines a compiler writes in front of a diagnostic, to say where it is.
OPENER = re.compile(r"\s*(In file included from\b|from \S|In function\b|\S+: In function\b"
                    r"|\S+: At top level)")


def errors_of(stderr: str) -> dict[int, str]:
    """Line of `c.c` -> the error that is about it, for every error reported.

    A compiler names the line in the error itself, in the lines in front of
    it (GCC: `inlined from 'v7' at c.c:12:5:`) or in the notes behind it
    (`c.c:12:44: note: called from here`, for an error it places in its own
    header).  A note that follows no error is about no error: GCC tells of a
    changed ABI once per file, with warnings off too, at whichever function
    comes first.
    """
    found: dict[int, str] = {}
    leading: list[int] = []     # named in front of the diagnostic to come
    error: str | None = None    # the error whose notes these are
    for line in stderr.splitlines():
        mentions = [int(number) for number in MENTION.findall(line)]
        diagnostic = DIAGNOSTIC.search(line)
        if diagnostic is None:
            if "inlined from" in line:
                error = None
                leading += mentions
            elif OPENER.match(line):
                error, leading = None, []
            continue
        kind, message = diagnostic.groups()
        if kind in ("error", "fatal error"):
            error = message
            for number in leading + mentions:
                found.setdefault(number, message)
        elif kind == "note" and error is not None:
            for number in mentions:
                found.setdefault(number, error)
        else:
            error = None
        leading = []
    return found


def first_error(stderr: str) -> str:
    for line in stderr.splitlines():
        diagnostic = DIAGNOSTIC.search(line)
        if diagnostic and diagnostic.group(1) in ("error", "fatal error"):
            return diagnostic.group(2)
    return (stderr.strip().splitlines() or ["no message"])[0]


def each_alone(command: list[str], lines: list[str], indexes: list[int], tmp: Path,
               head: str) -> dict[int, str]:
    """Of `indexes`, the lines the compiler refuses when each is the whole file.

    Side by side: each compilation has its own source, and its own output
    where `command` names one, so that none writes the file of another.
    """
    def ask(index: int) -> tuple[int, str | None]:
        source, output = tmp / f"alone-{index}.c", tmp / f"alone-{index}.out"
        source.write_text(head + lines[index] + "\n")
        own = [str(output) if at and command[at - 1] == "-o" and argument != os.devnull
               else argument for at, argument in enumerate(command)]
        result = subprocess.run([*own, "-w", str(source)], capture_output=True, text=True,
                                cwd=tmp)
        source.unlink()
        output.unlink(missing_ok=True)
        return index, None if result.returncode == 0 else first_error(result.stderr)

    with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count() or 1) as pool:
        return {index: message for index, message in pool.map(ask, indexes)
                if message is not None}


def refusals(command: list[str], lines: list[str], tmp: Path, head: str = HEAD) -> dict[int, str]:
    """The indexes of `lines` a compiler refuses, each with the error that names it.

    The file is compiled, the lines its errors are about are taken out, and
    it is compiled again until nothing is refused: a compiler reports what it
    sees, and some stop at the first thing they see.  An error that names no
    line of ours (an assembler's, a crash) is found by halves.
    """
    alive, bad = list(range(len(lines))), {}
    source = tmp / "c.c"
    first = head.count("\n") + 1
    stopped = 0
    while alive:
        source.write_text(head + "\n".join(lines[i] for i in alive) + "\n")
        result = subprocess.run([*command, "-w", str(source)], capture_output=True, text=True,
                                cwd=tmp)
        if result.returncode == 0:
            return bad
        named = {alive[number - first]: message
                 for number, message in errors_of(result.stderr).items()
                 if 0 <= number - first < len(alive)}
        if not named:
            if len(alive) == 1:
                bad[alive[0]] = first_error(result.stderr)
                return bad
            half = len(alive) // 2
            for part in (alive[:half], alive[half:]):
                inner = refusals(command, [lines[i] for i in part], tmp, head)
                bad.update({part[i]: message for i, message in inner.items()})
            return bad
        bad.update(named)
        alive = [i for i in alive if i not in named]
        alone = len(named) == 1 and ELSEWHERE.search(next(iter(named.values())))
        stopped = stopped + 1 if alone else 0
        if stopped == ONE_AT_A_TIME:
            bad.update(each_alone(command, lines, alive, tmp, head))
            return bad
    return bad


def implicated(command: list[str], lines: list[str], tmp: Path, head: str = HEAD) -> set[int]:
    """The indexes of `lines` a compiler refuses."""
    return set(refusals(command, lines, tmp, head))


def immediates_of(functions: dict, compile_only: list[str],
                  tmp: Path) -> tuple[dict[str, list[int] | None], set[str]]:
    """(name -> the positions GCC wants a constant at, or None when no choice
    compiles; the names GCC has no intrinsic of on this target or in this mode).

    Offered to GCC: first every parameter a variable.  What it refuses for
    the target is set aside by the message, before anything is tried: on a
    processor without AVX-512 that is every AVX-512 intrinsic, and a search
    for constants in `_mm512_set_epi8` has 64 parameters to search.

    For the others the constants are found in two steps of one file each.
    A constant at every position that may hold one, with each value of
    `PROBES`: the first value GCC takes.  Then each position in turn a
    variable again, the others constant: GCC refuses exactly where it wants
    the constant, because a constant is valid wherever a variable is.  That
    is 4 + k tests for k positions, where every set of positions is 2**k.
    """
    names = sorted(functions)
    shapes = {name: split_parameters(functions[name][1]) for name in names}

    def test(name: str, constants: dict[int, int]) -> Test:
        return Test(name, functions[name][0], shapes[name], constants)

    def lines_of(tests: list[Test]) -> list[str]:
        return [t.line(i) for i, t in enumerate(tests)]

    why = refusals(compile_only, lines_of([test(name, {}) for name in names]), tmp)
    elsewhere = {names[i] for i, message in why.items() if ELSEWHERE.search(message)}
    answer: dict[str, list[int] | None] = {name: [] for i, name in enumerate(names)
                                           if i not in why}
    wanted = [names[i] for i in sorted(why) if names[i] not in elsewhere]
    places = {name: [p for p, (kind, _) in enumerate(shapes[name]) if may_be_immediate(kind)]
              for name in wanted}

    filled = [(name, value) for name in wanted if places[name] for value in PROBES]
    refused = implicated(compile_only, lines_of(
        [test(name, dict.fromkeys(places[name], value)) for name, value in filled]), tmp)
    taken: dict[str, int] = {}
    for i, (name, value) in enumerate(filled):
        if i not in refused:
            taken.setdefault(name, value)

    freed = [(name, position) for name in wanted if name in taken for position in places[name]]
    refused = implicated(compile_only, lines_of(
        [test(name, {p: taken[name] for p in places[name] if p != position})
         for name, position in freed]), tmp)
    for name in wanted:
        answer[name] = [] if name in taken else None
    for i, (name, position) in enumerate(freed):
        if i in refused:
            answer[name].append(position)
    for name in elsewhere:
        answer[name] = None
    return answer, elsewhere


def not_for_this_target(names: list[str], functions: dict, compile_only: list[str],
                        tmp: Path) -> set[str]:
    """The names GCC refuses for the target, not for the call written."""
    found = set()
    source = tmp / "t.c"
    for name in names:
        source.write_text(HEAD + census.caller(0, name, *functions[name]) + "\n")
        result = subprocess.run([*compile_only, "-w", str(source)], capture_output=True,
                                text=True, cwd=tmp)
        if ELSEWHERE.search(result.stderr):
            found.add(name)
    return found


def combination(choices: list[list[int]], index: int) -> tuple[int, ...]:
    """Element `index` of the product of `choices`, the last choice counting
    fastest, without the product: 29 values at 6 positions are 594 million."""
    picked = []
    for choice in reversed(choices):
        index, at = divmod(index, len(choice))
        picked.append(choice[at])
    return tuple(reversed(picked))


def variants_of(name: str, returns: str, shape: list[tuple[str, str]],
                positions: list[int]) -> list[Test]:
    if not positions:
        plain = Test(name, returns, shape, {})
        return [plain] + conversion_twins(plain)
    choices = [candidates_for(name, shape[p][1]) for p in positions]
    count = math.prod(len(choice) for choice in choices)
    picked = range(count)
    if count > MOST_VARIANTS:
        # A spread of the product, the same on every run.
        step = count / MOST_VARIANTS
        picked = [int(k * step) for k in range(MOST_VARIANTS)]
    tests = [Test(name, returns, shape, dict(zip(positions, combination(choices, index))))
             for index in picked]
    # One twin per choice: where the literal is not valid for the intrinsic,
    # GCC drops the twin with it. The conversion twins go with the same
    # choices, so that one of them survives where an expression twin does.
    return (tests + [Test(name, returns, shape, dict(test.constants), "~")
                     for test in tests[:EXPRESSION_TWINS]]
            + [twin for test in tests[:EXPRESSION_TWINS] for twin in conversion_twins(test)])


def refused_constants(tests: list[Test], invalid: set[int]) -> list[Test]:
    """Of the choices of constants GCC refuses, the ones lccc is asked for.

    Per intrinsic GCC compiles some choice of: the refused choice next to
    the range (the least constants), and one with each constant of `FAR`.
    The literal spelling only: what an expression is worth is the business
    of the tests that compile.
    """
    valid = {test.name for i, test in enumerate(tests) if i not in invalid}
    refused: dict[str, list[Test]] = {}
    for i in sorted(invalid):
        test = tests[i]
        if test.name in valid and not test.spelling:
            refused.setdefault(test.name, []).append(test)

    def size(test: Test) -> tuple:
        values = [test.constants[p] for p in sorted(test.constants)]
        return (sum(1 for v in values if v < 0), sum(abs(v) for v in values), values)

    chosen: list[Test] = []
    for name in sorted(refused):
        ordered = sorted(refused[name], key=size)
        picked = [ordered[0]]
        for far in FAR:
            match = next((t for t in ordered if far in t.constants.values()), None)
            if match is not None and match not in picked:
                picked.append(match)
        chosen += picked
    return chosen


def run(program: Path, count: int, tmp: Path) -> tuple[list[str | None], dict[int, int]]:
    """The hash of every test, and the signal of each that dies."""
    died: dict[int, int] = {}
    skip = tmp / "skip"
    while True:
        skip.write_text("".join(f"{i}\n" for i in sorted(died)))
        result = subprocess.run([str(program), str(skip)], capture_output=True, text=True,
                                timeout=600)
        lines = result.stdout.splitlines()
        if result.returncode == 0 and len(lines) == count:
            break
        if len(lines) >= count:
            raise RuntimeError(f"{program}: exit {result.returncode} after every test")
        died[len(lines)] = result.returncode
    hashes: list[str | None] = []
    for index, line in enumerate(lines):
        hashes.append(None if index in died or line == "-" else line.split()[-1])
    return hashes, died


class Outcome:
    def __init__(self):
        self.tested: dict[str, int] = {}          # intrinsic -> tests compared
        self.differ: dict[str, list[str]] = {}    # intrinsic -> labels that differ
        self.dies: dict[str, list[str]] = {}      # lccc's code dies, the reference's not
        self.refused: dict[str, list[str]] = {}   # lccc refuses a test of what it has
        self.not_assembly: dict[str, list[str]] = {}  # GNU as refuses what lccc -S writes
        self.crashes: dict[str, list[str]] = {}   # lccc stops with an internal error
        self.accepts: dict[str, list[str]] = {}   # lccc compiles a constant GCC refuses
        self.late: dict[str, list[str]] = {}      # ... or refuses it without naming the call
        self.refuses_constant: set[str] = set()   # ... or refuses it as GCC does: right
        self.takes_a_value: set[str] = set()      # a value where GCC wants a constant: right
        self.wants_a_constant: set[str] = set()   # ... refused with a diagnostic, as GCC does
        self.not_executable: set[str] = set()     # the reference's code dies
        self.references_differ: set[str] = set()
        self.run_only: set[str] = set()
        self.not_run: set[str] = set()
        self.not_modeled: set[str] = set()        # GCC refuses every caller written
        self.not_here: set[str] = set()           # not for this processor or mode

    def merge(self, other: "Outcome"):
        for mine, theirs in ((self.tested, other.tested),):
            mine.update(theirs)
        for mine, theirs in ((self.differ, other.differ), (self.dies, other.dies),
                             (self.refused, other.refused),
                             (self.not_assembly, other.not_assembly),
                             (self.crashes, other.crashes), (self.accepts, other.accepts),
                             (self.late, other.late)):
            mine.update(theirs)
        for mine, theirs in ((self.not_executable, other.not_executable),
                             (self.references_differ, other.references_differ),
                             (self.run_only, other.run_only), (self.not_run, other.not_run),
                             (self.not_modeled, other.not_modeled),
                             (self.not_here, other.not_here),
                             (self.takes_a_value, other.takes_a_value),
                             (self.wants_a_constant, other.wants_a_constant),
                             (self.refuses_constant, other.refuses_constant)):
            mine |= theirs

    def failed(self) -> bool:
        return bool(self.differ or self.dies or self.refused or self.not_assembly
                    or self.crashes or self.accepts or self.late)


def build(compiler: list[str], lines: list[str], runtime: Path, linker: str, out: Path,
          tmp: Path) -> bool:
    table = (f"void (*const lv_tests[])(void) = {{ {', '.join(f'v{i}' for i in range(len(lines)))} }};\n"
             f"const unsigned lv_count = {len(lines)};\n")
    source = tmp / "c.c"
    source.write_text(HEAD + "\n".join(lines) + "\n" + table)
    obj = out.with_suffix(".o")
    if subprocess.run([*compiler, "-w", "-c", str(source), "-o", str(obj)], capture_output=True,
                      cwd=tmp).returncode != 0:
        return False
    return subprocess.run([linker, str(runtime), str(obj), "-o", str(out)],
                          capture_output=True).returncode == 0


def program_of(compiler: list[str], tests: list[Test], runtime: Path, linker: str, tag: str,
               tmp: Path) -> tuple[list[str | None], dict[int, int], set[int]]:
    """Build and run `tests`: the hash of each, the exit of each that dies,
    and the ones the compiler does not make an object of."""
    lines = [test.line(i) for i, test in enumerate(tests)]
    refused = implicated([*compiler, "-c", "-o", str(tmp / "probe.o")], lines, tmp)
    kept = [i for i in range(len(tests)) if i not in refused]
    full: list[str | None] = [None] * len(tests)
    if not kept:
        return full, {}, refused
    program = tmp / f"program-{tag}"
    if not build(compiler, [tests[i].line(k) for k, i in enumerate(kept)], runtime, linker,
                 program, tmp):
        raise RuntimeError(f"{' '.join(compiler)}: the tests compile one by one and do not "
                           f"link together")
    hashes, died = run(program, len(kept), tmp)
    for k, i in enumerate(kept):
        full[i] = hashes[k]
    return full, {kept[k]: status for k, status in died.items()}, refused


CRASH = re.compile(r"internal error|internal compiler error|panicked at|"
                   r"RUST_BACKTRACE|stack overflow|Segmentation fault")


class Verdict(NamedTuple):
    """What a compiler does with one test alone."""
    accepted: bool   # it makes an object file
    message: str     # its first error, without the place
    crashed: bool    # an internal error, or an end by a signal
    placed: bool     # the error names the line of the program it is about


def verdict_of(compiler: list[str], test: Test, tmp: Path, tag: str = "m") -> Verdict:
    """`tag` names the files, so that verdicts can be asked for side by side."""
    source = tmp / f"{tag}.c"
    source.write_text(HEAD + test.line(0) + "\n")
    result = subprocess.run([*compiler, "-w", "-c", str(source), "-o", str(tmp / f"{tag}.o")],
                            capture_output=True, text=True, cwd=tmp)
    return verdict_from(result.returncode, result.stderr, source.name)


def verdict_from(returncode: int, stderr: str, source: str) -> Verdict:
    text = stderr.strip()
    crashed = returncode < 0 or bool(CRASH.search(text))
    lines = [line for line in text.splitlines() if "error" in line or CRASH.search(line)]
    first = (lines or text.splitlines() or ["no message"])[0]
    place = re.compile(r"^.*?" + re.escape(source) + r":\d+:\d+: ")
    return Verdict(returncode == 0, place.sub("", first)[:160], crashed,
                   bool(place.search(first)))


def refused_by_gnu_as(lccc_compile: list[str], tests: list[Test], gnu_as: str,
                      tmp: Path) -> dict[int, str]:
    """The tests lccc writes assembly for that GNU as refuses, with its message.

    `lccc -S` is a promise that the text is assembly: a statement lccc's own
    assembler takes and GNU as does not is a statement no other tool can
    read (`vpmovsxbw %zmm0, %zmm0`).
    """
    found: dict[int, str] = {}
    lines = [test.line(i) for i, test in enumerate(tests)]
    source, text = tmp / "s.c", tmp / "s.s"
    source.write_text(HEAD + "\n".join(lines) + "\n")
    if subprocess.run([*lccc_compile, "-w", "-S", str(source), "-o", str(text)],
                      capture_output=True, cwd=tmp).returncode != 0:
        return found
    result = subprocess.run([gnu_as, "--64", str(text), "-o", os.devnull], capture_output=True,
                            text=True)
    if result.returncode == 0:
        return found
    # The function a refused line is in: the last `vN:` label above it.
    owner, current = {}, None
    for number, line in enumerate(text.read_text().splitlines(), 1):
        label = re.match(r"v(\d+):", line)
        if label:
            current = int(label.group(1))
        owner[number] = current
    for match in re.finditer(r":(\d+): Error: (.*)", result.stderr):
        index = owner.get(int(match.group(1)))
        if index is not None and index not in found:
            found[index] = match.group(2).strip()
    return found


def examine(header: Path, lccc: str, levels: list[str], refs: list[str], runtime: Path,
            tmp: Path, verbose: bool, gnu_as: str | None) -> Outcome:
    outcome = Outcome()
    functions = {name: value for name, value in
                 census.functions_of(header.read_text(errors="replace")).items()
                 if not UNDEFINED.search(name)}
    for name in list(functions):
        if NOT_RUN.match(name):
            outcome.not_run.add(name)
            del functions[name]
    if not functions:
        return outcome
    gcc = refs[0]
    gcc_compile = [gcc, *TARGET, "-O2", "-S", "-o", os.devnull]

    # What lccc has, for this processor: the callers it compiles.
    names = sorted(functions)
    callers = [census.caller(i, name, *functions[name]) for i, name in enumerate(names)]
    lacking = census.refused([lccc, *TARGET, "-O1"], callers, tmp)
    functions = {name: functions[name] for i, name in enumerate(names) if i not in lacking}
    if not functions:
        return outcome

    # An intrinsic of another processor or mode: lccc declares more than the
    # target has, and GCC says so by name.
    positions, elsewhere = immediates_of(functions, gcc_compile, tmp)
    outcome.not_here |= elsewhere
    tests: list[Test] = []
    for name in sorted(functions):
        if positions[name] is not None:
            tests += variants_of(name, functions[name][0],
                                 split_parameters(functions[name][1]), positions[name])
    invalid = implicated(gcc_compile, [test.line(i) for i, test in enumerate(tests)], tmp)
    rejected = refused_constants(tests, invalid)
    tests = [test for i, test in enumerate(tests) if i not in invalid]
    # What GCC compiles no test of beside those: one this tool writes no
    # valid call of, or one GCC refuses for the target only once the call has
    # its constants.
    untested = sorted(set(functions) - elsewhere - {test.name for test in tests})
    late = not_for_this_target(untested, functions, gcc_compile, tmp)
    outcome.not_here |= late
    outcome.not_modeled |= set(untested) - late
    if not tests:
        return outcome

    reference, reference_died, unbuilt = program_of([gcc, *TARGET, "-O2"], tests, runtime, gcc,
                                                    "gcc", tmp)
    if unbuilt:
        raise RuntimeError(f"{header.name}: {gcc} does not build the tests it compiled")
    for i in reference_died:
        outcome.not_executable.add(tests[i].name)
    for ref in refs[1:]:
        # Another reference has other headers: the tests it compiles.
        hashes, _, _ = program_of([ref, *TARGET, "-O2"], tests, runtime, gcc, "ref", tmp)
        for i, value in enumerate(hashes):
            if value is not None and reference[i] is not None and value != reference[i]:
                outcome.references_differ.add(tests[i].name)

    # The same calls with a value of the running program where GCC wants a
    # constant: one per intrinsic, behind the tests, held to the hash GCC
    # computes for the constant.
    probed: dict[str, int] = {}
    for i, test in enumerate(tests):
        if test.constants and not test.spelling and i not in reference_died:
            probed.setdefault(test.name, i)
    twins = sorted(probed.values())
    asked = tests + [Test(tests[i].name, tests[i].returns, tests[i].parameters,
                          dict(tests[i].constants), "?") for i in twins]
    reference = reference + [reference[i] for i in twins]

    for level in levels:
        # The constants GCC refuses: every one alone, since a compiler that
        # stops at the first error says nothing about the second.
        with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count() or 1) as pool:
            verdicts = list(pool.map(
                lambda numbered: verdict_of([lccc, *TARGET, level], numbered[1], tmp,
                                            f"r{numbered[0]}"),
                enumerate(rejected)))
        for test, verdict in zip(rejected, verdicts):
            said = f"{test.label} {level}"
            if verdict.crashed:
                outcome.crashes.setdefault(test.name, []).append(f"{said}: {verdict.message}")
            elif verdict.accepted:
                outcome.accepts.setdefault(test.name, []).append(said)
            elif not verdict.placed:
                outcome.late.setdefault(test.name, []).append(f"{said}: {verdict.message}")
            else:
                outcome.refuses_constant.add(test.name)

        hashes, died, refused = program_of([lccc, *TARGET, level], asked, runtime, gcc,
                                           f"lccc{level}", tmp)
        for i in sorted(refused):
            test = asked[i]
            verdict = verdict_of([lccc, *TARGET, level], test, tmp)
            if verdict.crashed:
                outcome.crashes.setdefault(test.name, []).append(
                    f"{test.label} {level}: {verdict.message}")
            elif test.spelling == "?":
                outcome.wants_a_constant.add(test.name)
            else:
                outcome.refused.setdefault(test.name, []).append(
                    f"{test.label} {level}: {verdict.message}")
        if gnu_as:
            assembled = [test for i, test in enumerate(asked) if i not in refused]
            for i, message in sorted(refused_by_gnu_as([lccc, *TARGET, level], assembled,
                                                       gnu_as, tmp).items()):
                outcome.not_assembly.setdefault(assembled[i].name, []).append(
                    f"{assembled[i].label} {level}: {message}")
        for i, test in enumerate(asked):
            name = test.name
            if i in refused or name in outcome.references_differ:
                continue
            if i < len(tests) and i in reference_died:
                continue
            if test.spelling == "?" and i not in died and hashes[i] == reference[i]:
                outcome.takes_a_value.add(name)
            if RUN_ONLY.match(name):
                outcome.run_only.add(name)
            if i in died:
                outcome.dies.setdefault(name, []).append(f"{test.label} {level} "
                                                         f"(exit {died[i]})")
                continue
            if RUN_ONLY.match(name):
                continue
            outcome.tested[name] = outcome.tested.get(name, 0) + 1
            if hashes[i] != reference[i]:
                outcome.differ.setdefault(name, []).append(f"{test.label} {level}")
    if verbose:
        for title, names in (("differs", outcome.differ), ("dies", outcome.dies),
                             ("refused", outcome.refused), ("not as", outcome.not_assembly),
                             ("crashes", outcome.crashes), ("accepts", outcome.accepts),
                             ("late", outcome.late)):
            for name, labels in sorted(names.items()):
                print(f"  {title:8} {name}: {' '.join(labels[:6])}"
                      f"{' ...' if len(labels) > 6 else ''}")
    return outcome


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--lccc", default=str(ROOT / "target" / "release" / "lccc"))
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"),
                        help="GCC 16.2: its headers are the list, its code the reference")
    parser.add_argument("--also", default="", metavar="COMPILERS",
                        help="further references (clang-23): where one differs from GCC, the "
                             "test is theirs to settle")
    parser.add_argument("--levels", default="-O2,-O0",
                        help="the optimization levels lccc is tested at")
    parser.add_argument("--only", metavar="HEADER", action="append", default=[])
    parser.add_argument("--as", dest="gnu_as", default=None, metavar="AS",
                        help="GNU as 2.47: what lccc -S writes for the tests has to assemble")
    parser.add_argument("--check", action="store_true",
                        help="fail when a test differs, dies or is refused")
    parser.add_argument("-v", "--verbose", action="store_true")
    args = parser.parse_args(argv)

    if not Path(args.lccc).exists():
        print(f"x86_intrinsic_values: no lccc at {args.lccc} (--lccc)", file=sys.stderr)
        return 2
    # The tests are compiled in a directory of their own.
    args.lccc = str(Path(args.lccc).resolve())
    if args.gnu_as and os.sep in args.gnu_as:
        args.gnu_as = str(Path(args.gnu_as).resolve())
    refs = [args.gcc, *[compiler for compiler in args.also.split(",") if compiler]]
    version = census.gcc_version_of(refs[0])
    if version != census.GCC_VERSION:
        print(f"x86_intrinsic_values: the headers are GCC {census.GCC_VERSION}'s; {refs[0]} is "
              f"{version or 'not runnable'} (--gcc)", file=sys.stderr)
        return 2
    include = Path(subprocess.run([refs[0], "-print-file-name=include"], capture_output=True,
                                  text=True).stdout.strip())
    total = Outcome()
    with tempfile.TemporaryDirectory() as directory:
        tmp = Path(directory)
        (tmp / "runtime.c").write_text(RUNTIME)
        runtime = tmp / "runtime.o"
        subprocess.run([refs[0], "-O2", "-c", str(tmp / "runtime.c"), "-o", str(runtime)],
                       check=True)
        for header in census.headers_of(include):
            if args.only and header.name not in args.only:
                continue
            started = time.monotonic()
            outcome = examine(header, args.lccc, args.levels.split(","), refs, runtime, tmp,
                              args.verbose, args.gnu_as)
            if outcome.tested or outcome.failed():
                print(f"{header.name:28} {len(outcome.tested):5} compared "
                      f"({sum(outcome.tested.values())} tests), {len(outcome.differ)} differ, "
                      f"{len(outcome.dies)} die, {len(outcome.refused)} refused, "
                      f"{len(outcome.crashes)} crash, {len(outcome.not_assembly)} not assembly, "
                      f"{len(outcome.accepts) + len(outcome.late)} bad constants taken"
                      + (f"  [{time.monotonic() - started:.0f} s]" if args.verbose else ""),
                      flush=True)
            total.merge(outcome)

    for title, names in (("differ from GCC", total.differ), ("die in lccc's code", total.dies),
                         ("refused by lccc", total.refused),
                         ("lccc stops with an internal error", total.crashes),
                         ("lccc -S is not assembly", total.not_assembly),
                         ("lccc compiles a constant GCC refuses", total.accepts),
                         ("lccc refuses a constant without naming the call", total.late)):
        for name in sorted(names):
            print(f"{title}: {name}: {' '.join(names[name][:8])}"
                  f"{' ...' if len(names[name]) > 8 else ''}")
    for title, names in (("the references differ", total.references_differ),
                         ("not executable here", total.not_executable),
                         ("not for this processor or mode", total.not_here),
                         ("no call GCC compiles", total.not_modeled)):
        if names:
            print(f"{title} ({len(names)}): {' '.join(sorted(names))}")
    print(f"{len(total.tested)} intrinsics compared with GCC {version} in "
          f"{sum(total.tested.values())} tests at {args.levels}; {len(total.differ)} differ, "
          f"{len(total.dies)} die, {len(total.refused)} refused, {len(total.crashes)} crash, "
          f"{len(total.not_assembly)} not assembly; run and not compared "
          f"{len(total.run_only)}, not run {len(total.not_run)}")
    constant_only = total.wants_a_constant - total.takes_a_value
    print(f"a value where GCC wants a constant: {len(constant_only)} refuse it as GCC does, "
          f"{len(total.takes_a_value)} compute what the constant computes")
    bad_constant = set(total.accepts) | set(total.late)
    print(f"a constant GCC refuses: {len(total.refuses_constant - bad_constant)} intrinsics "
          f"refuse it as GCC does, {len(total.accepts)} compile it, {len(total.late)} refuse "
          f"it without naming the call")
    if args.check and total.failed():
        print("x86_intrinsic_values: an intrinsic lccc has does not do what GCC's does.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
