#!/usr/bin/env python3
"""Random multi-word adders: lccc's programs against the reference compiler's.

`carry_chain_adders.c` holds the spellings of an adder that somebody thought
of. This gate writes the ones nobody did. From a seed it generates a C
program of functions that add and subtract in limbs, each put together at
random from the pieces such code is made of:

* the limb: sum first or carry first, the subtrahends added up first, one
  expression with the `t < x || (c && t == x)` carry, a double-width sum,
  `__builtin_add_overflow`, `__builtin_addcll`, `_addcarry_u64`, an
  increment under a branch;
* the comparisons, against either operand, spelled `<`, `>`, `!(>=)`,
  `!(<=)` -- and now and then against an operand that makes it no carry
  at all;
* the join: `|`, `+`, `^`, `||`, `?:`, `!= 0`;
* the carry: a `u64`, a `u32`, an `int`, a byte or a `_Bool`, starting as
  0, 1, a bit computed from an argument, or a whole word;
* around the limbs: a call, arithmetic that clobbers the flags, a branch,
  a store of the carry, a carry that is inverted, masked or set on a
  condition; counted loops and pointer loops, forwards and backwards, two
  limbs a trip, with an early exit;
* the memory: results apart from the operands, over them, and one limb
  ahead of them, so that a trip reads what the trip before it wrote;
* sums kept in memory with their carries added up, the column sum of a
  product (`acc[0] += lo; hi += acc[0] < lo`): the sum stored before its
  carry is read, the carry computed first, a borrow, a store through a
  pointer that may be the sum or the addend in between -- and comparisons
  that are no carry (`<=`, the operands the other way round).

Nothing in a program is undefined: every word is unsigned, and a carry kept
in a signed type is only converted, compared and edited bit by bit. So
what it prints is what C says it prints, and GCC's program is the witness.
lccc compiles the same source at every optimization level and with the
carry pass, the peephole and register allocation switched off in turn;
every program must print what GCC's prints, line by line (a line is one
function, so a difference names it).

Most of what is generated is not an adder the optimizer may rewrite, and
that is the point: the pass has to tell the ones that are from the ones
that only look like it.

The gate first proves that it can fail: one program is compiled with a
carry planted wrong (`-DFUZZ_PLANT`), and that function, alone, has to be
reported. `carry_chain_adders.c` itself goes through the same
configurations.

With `--limbs` a program is eight limb loops and nothing else, as the
libraries write them and as `limb_unroll` splits them into four limbs a
trip and a remainder: one adder a trip over up to 24 limbs; the counter a
`long`, an `int` or unsigned; counted up to `n` with `<` or `!=`, down to
nothing, by `while (m)`, `do ... while (--m)`, `do ... while (m-- != 1)` or
`while (n-- > 0)`, from a start that is not zero and may be behind the
limit, to a constant limit; through an index, through stepped pointers, or
both; the counter, the pointers and the last sum read behind the loop.
Every other loop is plain -- an adder the optimizer knows and nothing else
in the trip, which is the loop the split is for. The rest are any spelling
at all, now and then with a sum of the limbs, a call or an edited carry in
the trip, which the split has to leave alone. These programs go through
the split switched off as well, and the gate proves that they are what
they are meant to be: at -O2 at least `MUST_SPLIT` of the 64 loops of the
run in CI are split (four `adc`/`sbb` in a row with `limb_unroll` and not
without it). `--census` lists every loop of the seeds with what it is made
of and whether it is split.

A run in CI is eight programs through seven configurations, and eight
programs of limb loops through eight. `--full` adds the other
configurations, `--seeds`/`--start` choose the programs, `--keep` saves the
source of whatever differs and `--dump` prints one program.

The eight are seeds 9 to 16, chosen by breaking the compiler: with the
fold of a compare behind a store (`fold_carry_compare`) allowed to name
memory, seeds 13 and 14 differ from GCC; with the registers it names
allowed to be written in between, seeds 10 and 16 do. A change to the
generator changes what a seed is; choose the window again then. The limb
loops are drawn from a stream of their own (the programs without them are
what they were before there were any), and their window is seeds 9 to 16
too: 26 of its 64 loops are split, in each of the ten spellings of a loop
that the pass takes.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
FIXTURE = ROOT / "tests/regression/carry_chain_adders.c"

PREAMBLE = r"""
#include <stdio.h>
#if defined(__x86_64__)
#include <immintrin.h>
#endif
typedef unsigned long long u64;
typedef unsigned int u32;
#define NOINLINE __attribute__((noinline))
enum { LIMBS = 8 };
static u64 g_sink;
static NOINLINE void sink(u64 v) { g_sink = g_sink * 0x100000001b3ull + v; }
static const u64 edge[] = {
    0, 1, 2, 3, 0x7fffffffffffffffull, 0x8000000000000000ull, 0x8000000000000001ull,
    0xffffffffffffffffull, 0xfffffffffffffffeull, 0xffffffffull, 0x100000000ull,
    0xfffffffeull, 0x80000000ull, 0x7fffffffull, 0xffffffff00000000ull, 0xffffffff00000001ull,
};
static u64 state = 0x9e3779b97f4a7c15ull;
static u64 rnd(void)
{
    state = state * 6364136223846793005ull + 1442695040888963407ull;
    u64 x = state ^ (state >> 29);
    /* Edges about as often as not: adders are wrong at the edges. */
    return (x >> 60) < 9 ? edge[(x >> 40) % (sizeof edge / sizeof edge[0])] : x;
}
static u64 fold(u64 h, u64 v) { h = (h ^ v) * 0x100000001b3ull; return h ^ (h >> 31); }
"""

CARRY_TYPES = ["u64", "u32", "int", "unsigned", "unsigned char", "signed char", "_Bool"]
# What happens to a carry between two limbs besides being carried: nothing
# that can overflow an `int` (a carry starts as any word).
CARRY_EDITS = ["c ^= 1;", "c = !c;", "c &= 1;", "c |= (p >> 1) & 1;", "if (p & 4) c = !c;",
               "if (q & 2) c = 1;", "c = c != 0;"]
CONSTANTS = {
    "u64": ["0", "1", "0xffffffffffffffffull", "0x8000000000000000ull", "0xffffffffull",
            "0x123456789abcdef1ull", "0x7fffffffffffffffull"],
    "u32": ["0", "1", "0xffffffffu", "0x80000000u", "0xffffu", "0x12345679u", "0x7fffffffu"],
}


# Limb loops in a program of `--limbs`: every other one plain, the adder
# one of `ADDERS` and nothing else in the trip.
LIMB_LOOPS = 8
# The spellings of `Program.limb` that are a full adder to the optimizer (3
# and 8 test the carry coming in, 9 counts carries).
ADDERS = (0, 1, 2, 4, 5, 6, 7)
# How many of the 64 limb loops of the run in CI (`--limbs`, seeds
# `LIMBS_FROM` and the seven behind it) `limb_unroll` splits at the least:
# what it splits today (`--limbs --census` says which; of the 640 loops of
# seeds 1 to 80 it is 235). The others are loops it has to leave alone (a
# call or a sum in the trip, a carry that is edited, a spelling that is no
# adder, a count that is known and small) and loops it does not take yet
# (`docs/x86-carry-chain.md` lists them). The number goes up with the pass
# and never down. Other windows are counted and not judged.
MUST_SPLIT = 26


class Program:
    """One generated translation unit."""

    def __init__(self, seed: int, functions: int, limb_loops: int = 0):
        self.r = random.Random(seed)
        self.count = 0
        self.functions: list[tuple[str, str, str]] = []  # (name, kind, word type)
        self.shapes: dict[str, str] = {}  # what a limb loop is made of, for a report
        self.spelling = self.extra = ""
        self.text = [PREAMBLE]
        if limb_loops:
            # The numbers of a limb loop are long enough for five trips of
            # four limbs and a remainder.
            self.text.append("enum { LONG = 24 };")
        for index in range(functions):
            # `f0` is a chain in every program: the negative control plants
            # its wrong carry there.
            kind = 0.0 if index == 0 else self.r.random()
            (self.chain if kind < 0.45 else self.loop if kind < 0.80 else self.accumulate)(index)
        # The limb loops, from a stream of their own: what the functions
        # above are does not depend on them. Every other one is plain.
        others, self.r = self.r, random.Random(f"limbs of {seed}")
        for index in range(functions, functions + limb_loops):
            self.limbs(index, plain=(index - functions) % 2 == 0)
        self.r = others
        self.text.append(self.main())

    def source(self) -> str:
        return "\n".join(self.text)

    def fresh(self, stem: str) -> str:
        self.count += 1
        return f"{stem}{self.count}"

    def pick(self, *choices):
        return self.r.choice(choices)

    def less(self, x: str, y: str) -> str:
        return self.pick(f"{x} < {y}", f"{y} > {x}", f"!({x} >= {y})", f"!({y} <= {x})")

    def join(self, c1: str, c2: str) -> str:
        return self.pick(f"{c1} | {c2}", f"{c2} | {c1}", f"{c1} + {c2}", f"{c1} ^ {c2}",
                         f"{c1} || {c2}", f"({c1} ? 1 : {c2})", f"(({c1} | {c2}) != 0)",
                         f"{c2} + {c1}")

    def operand(self, word: str, right: list[str], wrong: list[str]) -> str:
        """What a comparison is made against: one of the operands that make
        it a carry, or, one time in twelve, something that does not."""
        return self.r.choice(wrong) if self.r.random() < 0.085 else self.r.choice(right)

    def limb(self, word: str, sub: bool, x: str, y: str, c: str, out: str,
             among: tuple[int, ...] | None = None) -> list[str]:
        """Statements that set `out` to `x + y + c` (or `x - y - c`) in some
        spelling and `c` to what that spelling takes for the carry. `among`
        narrows the spellings to choose from."""
        bits = 64 if word == "u64" else 32
        kind = self.pick(*CARRY_TYPES, word)
        s, t, c1, c2 = (self.fresh(stem) for stem in ("s", "t", "ca", "cb"))
        cw = f"({word}){c}"
        everything = [x, y, s, t, cw]
        variant = self.r.randrange(10) if among is None else self.r.choice(among)
        self.spelling = f"{'sub' if sub else 'add'} spelling {variant}"
        if variant <= 2 and not sub:
            # Sum first, carry first, or the second word and the carry first.
            first, second, third = [(x, y, cw), (x, cw, y), (y, cw, x)][variant]
            lines = [f"{word} {s} = {first} + {second}, {t} = {s} + {third};",
                     f"{kind} {c1} = {self.less(s, self.operand(word, [first, second], everything))};",
                     f"{kind} {c2} = {self.less(t, self.operand(word, [s, third], everything))};",
                     f"{out} = {t}; {c} = {self.join(c1, c2)};"]
        elif variant <= 2:
            if variant == 2:
                # The subtrahends added up first.
                lines = [f"{word} {s} = {y} + {cw}, {t} = {x} - {s};",
                         f"{kind} {c1} = {self.less(s, self.operand(word, [y, cw], everything))};",
                         f"{kind} {c2} = {self.less(self.operand(word, [x], everything), s)};"]
            else:
                first, second = (y, cw) if variant == 0 else (cw, y)
                lines = [f"{word} {s} = {x} - {first}, {t} = {s} - {second};",
                         f"{kind} {c1} = {self.pick(self.less(x, first), self.less(x, s))};",
                         f"{kind} {c2} = {self.pick(self.less(s, second), self.less(s, t))};"]
                if self.r.random() < 0.085:
                    lines[1] = f"{kind} {c1} = {self.less(self.r.choice(everything), self.r.choice(everything))};"
            lines.append(f"{out} = {t}; {c} = {self.join(c1, c2)};")
        elif variant == 3:
            op, base = ("-", x) if sub else ("+", x)
            edge = f"{x} == {y}" if sub else f"{t} == {x}"
            first = self.less(x, y) if sub else self.less(t, x)
            lines = [f"{word} {t} = {x} {op} {y} {op} {cw};",
                     f"{kind} {c1} = " + self.pick(f"{first} || ({c} && {edge})",
                                                    f"({first}) | (({edge}) & ({c} != 0))",
                                                    f"{first} ? 1 : ({edge} ? {c} != 0 : 0)") + ";",
                     f"{out} = {t}; {c} = {c1};"]
        elif variant == 4:
            wide = "unsigned __int128" if bits == 64 else "u64"
            guard = bits == 64
            op = "-" if sub else "+"
            carry = self.pick(f"({t} >> {bits}) & 1", f"({t} >> {bits}) != 0") if sub else f"({word})({t} >> {bits})"
            lines = [f"{wide} {t} = ({wide}){x} {op} {y} {op} {cw};", f"{out} = ({word}){t}; {c} = {carry};"]
            if guard:
                # No `__int128` in 32-bit code: the same sum in two steps.
                lines = (["#ifdef __SIZEOF_INT128__"] + lines + ["#else"] +
                         [f"{word} {s} = {x} {op} {y}, {t} = {s} {op} {cw};",
                          f"{out} = {t}; {c} = ({self.less(x, y) if sub else self.less(s, x)}) | "
                          f"({self.less(s, cw) if sub else self.less(t, s)});", "#endif"])
        elif variant == 5:
            builtin = "__builtin_sub_overflow" if sub else "__builtin_add_overflow"
            lines = [f"{word} {s}, {t};",
                     f"{kind} {c1} = {builtin}({x}, {y}, &{s});",
                     f"{kind} {c2} = {builtin}({s}, {cw}, &{t});",
                     f"{out} = {t}; {c} = {self.join(c1, c2)};"]
        elif variant == 6:
            builtin = ("__builtin_subc" if sub else "__builtin_addc") + ("ll" if bits == 64 else "")
            lines = [f"{word} {c1};", f"{out} = {builtin}({x}, {y}, {cw}, &{c1}); {c} = {c1};"]
        elif variant == 7:
            intrinsic = ("_subborrow_u" if sub else "_addcarry_u") + str(bits)
            op = "-" if sub else "+"
            lines = ["#if defined(__x86_64__)",
                     f"{word} {t}; {c} = {intrinsic}((unsigned char){c}, {x}, {y}, &{t}); {out} = {t};",
                     "#else",
                     f"{word} {s} = {x} {op} {y}, {t} = {s} {op} ((unsigned char){c} != 0);",
                     f"{out} = {t}; {c} = ({self.less(x, y) if sub else self.less(s, x)}) | "
                     f"({self.less(s, t) if sub else self.less(t, s)});",
                     "#endif"]
        elif variant == 8:
            # The carry in as a condition: one more, or one less.
            op = "-" if sub else "+"
            first = self.less(x, y) if sub else self.less(t, x)
            wrapped = f"{t} == 0" if not sub else f"{t} == ({word})-1"
            lines = [f"{word} {t} = {x} {op} {y}; {kind} {c1} = {first};",
                     f"if ({c}) {{ {t} {op}= 1; {c1} |= {wrapped}; }}",
                     f"{out} = {t}; {c} = {c1};"]
        else:
            # A carry that is counted: three words, no bit among them.
            op = "-" if sub else "+"
            z = self.pick(cw, f"({cw} * 3)", y)
            lines = [f"{word} {s} = {x} {op} {y}, {t} = {s} {op} {z};",
                     f"{out} = {t}; {c} = ({self.less(x, y) if sub else self.less(s, x)}) + "
                     f"({self.less(s, z) if sub else self.less(t, s)});"]
        return lines

    def initial(self, word: str) -> str:
        return self.pick("0", "0", "1", "p & 1", "p != 0", "p > q", "(p >> 63)", "(q < p)",
                         "p", "p & 3", "(q < p) + (p & 1)", "!p")

    def chain(self, index: int) -> None:
        word = self.pick("u64", "u64", "u32")
        kind = self.pick(*CARRY_TYPES, word)
        limbs = self.r.randrange(1, 5)
        sub = self.r.random() < 0.45
        name = f"f{index}"
        body = [f"static NOINLINE u64 {name}(const {word} *a, const {word} *b, u64 p, u64 q, {word} *out)", "{",
                f"    {kind} c = {self.initial(word)}; u64 h = q;"]
        for i in range(limbs):
            if self.r.random() < 0.15:
                sub = not sub
            y = self.pick(f"b[{i}]", f"b[{i}]", f"b[{i}]", self.r.choice(CONSTANTS[word]),
                          f"a[{i}]", f"({word})q")
            x = self.pick(f"a[{i}]", f"a[{i}]", f"a[{i}]", f"({word})p") if i else f"a[{i}]"
            lines = self.limb(word, sub, x, y, "c", f"out[{i}]")
            shape = self.r.random()
            if shape < 0.12:
                other = self.limb(word, sub, x, y, "c", f"out[{i}]")
                lines = ([f"if (p & {1 << i}) {{"] + lines + ["} else {"] + other + ["}"])
            body.append("    {\n      " + "\n      ".join(lines) + "\n    }")
            between = self.r.random()
            if between < 0.10:
                body.append("    sink(c);")
            elif between < 0.20:
                body.append(f"    h = h * 31 + (h >> 7) + out[{i}];")
            elif between < 0.27:
                body.append(f"    out[{i}] ^= ({word})c;")
            elif between < 0.32:
                body.append("    sink(h);")
            elif between < 0.42:
                body.append("    " + self.r.choice(CARRY_EDITS))
        body += ["#ifdef FUZZ_PLANT", "    c = !c;", "#endif"] if index == 0 else []
        body += [f"    out[{limbs}] = ({word})c;", "    return (u64)c + h;", "}"]
        self.text.append("\n".join(body))
        self.functions.append((name, "chain", word))

    def loop(self, index: int) -> None:
        word = self.pick("u64", "u64", "u32")
        kind = self.pick(*CARRY_TYPES, word)
        sub = self.r.random() < 0.45
        name = f"f{index}"
        y_of = lambda i: self.pick(f"b[{i}]", f"b[{i}]", f"b[{i}]", f"b[{i}]",  # noqa: E731
                                   self.r.choice(CONSTANTS[word]), f"({word})p")
        early = "      if (!c && x == y) break;\n" if self.r.random() < 0.08 else ""

        def trip(i: str, pointers: bool = False, in_loop: bool = True) -> str:
            x, y = ("*a++", "*b++") if pointers else (f"a[{i}]", y_of(i))
            store = "*r++" if pointers else f"r[{i}]"
            lines = self.limb(word, sub, "x", "y", "c", "t")
            extra = self.pick("", "", "", "", "      sink(c);\n", "      h += t;\n",
                              "      " + self.r.choice(CARRY_EDITS) + "\n")
            return (f"      {word} x = {x}, y = {y}, t;\n" + (early if in_loop else "") + "      " +
                    "\n      ".join(lines) + f"\n      {store} = t;\n" + extra)

        form = self.r.randrange(9)
        if form == 0:
            loop = "    for (long i = 0; i < n; i++) {\n" + trip("i") + "    }"
        elif form == 1:
            loop = "    for (long i = n - 1; i >= 0; i--) {\n" + trip("i") + "    }"
        elif form == 2:
            loop = "    long i = 0;\n    while (i < n) {\n" + trip("i") + "      i++;\n    }"
        elif form == 3:
            loop = "    if (n > 0) {\n    long i = 0;\n    do {\n" + trip("i") + "    } while (++i < n);\n    }"
        elif form == 4:
            loop = "    while (n-- > 0) {\n" + trip("", True) + "    }"
        elif form == 5:
            loop = "    if (n > 0)\n    do {\n" + trip("", True) + "    } while (--n != 0);"
        elif form == 6:
            loop = ("    long i = 0;\n    for (; i + 1 < n; i += 2) {\n      {\n" + trip("i") + "      }\n      {\n" +
                    trip("i + 1") + "      }\n    }\n    if (i < n) {\n" + trip("i", in_loop=False) + "    }")
        elif form == 7:
            loop = "    for (unsigned i = 0; i < (unsigned)n; i++) {\n" + trip("i") + "    }"
        else:
            loop = "    for (int i = 0; i < (int)n; i++) {\n" + trip("i") + "    }"
        body = [f"static NOINLINE u64 {name}({word} *r, const {word} *a, const {word} *b, long n, u64 p, u64 q)",
                "{", f"    {kind} c = {self.initial(word)}; u64 h = q;", loop,
                "    return (u64)c + h;", "}"]
        self.text.append("\n".join(body))
        self.functions.append((name, "loop", word))

    def limbs(self, index: int, plain: bool) -> None:
        """A limb loop as a library writes it: one adder a trip and a
        counter stepped by one, in the ways a counted loop is spelled. A
        `plain` one is an adder the optimizer knows and nothing else in the
        trip, which is the loop `limb_unroll` is for; the others are any
        spelling at all, and now and then more than a limb."""
        word = self.pick("u64", "u64", "u64", "u32")
        kind = self.pick(word, word, word, *CARRY_TYPES)
        sub = self.r.random() < 0.4
        count = self.pick("long", "long", "unsigned long", "int", "unsigned", "u64", "u32")
        name = f"f{index}"

        def trip(x: str, y: str, store: str) -> str:
            lines = self.limb(word, sub, "x", "y", "c", "t", ADDERS if plain else None)
            # Mostly nothing else in the trip. Now and then a sum of the
            # limbs, the last limb kept, a call, or a carry that is edited:
            # loops the split has to leave alone, but for the limb kept.
            extra = "" if plain else self.pick(
                "", "", "", "", "", "", "", "", "      h += t;\n", "      last = t;\n", "      sink(c);\n",
                "      " + self.r.choice(CARRY_EDITS) + "\n")
            self.extra = extra
            return (f"      {word} x = {x}, y = {y}, t;\n      " + "\n      ".join(lines) +
                    f"\n      {store} = t;\n" + extra)

        def indexed(i: str) -> str:
            return trip(f"a[{i}]", f"b[{i}]", f"r[{i}]")

        def stepped() -> str:
            return trip("*a++", "*b++", "*r++")

        behind = "0"
        form = self.r.randrange(14)
        if form == 0:
            loop = f"    for ({count} i = 0; i < ({count})n; i++) {{\n" + indexed("i") + "    }"
        elif form == 1:
            loop = f"    for ({count} i = 0; i != ({count})n; i++) {{\n" + indexed("i") + "    }"
        elif form == 2:
            loop = f"    for ({count} i = ({count})n; i > 0; i--) {{\n" + indexed("n - (long)i") + "    }"
        elif form == 3:
            loop = f"    for ({count} i = ({count})n; i != 0; i--) {{\n" + stepped() + "    }"
            behind = "(u64)(r - r0)"
        elif form == 4:
            loop = f"    {count} m = ({count})n;\n    while (m) {{\n" + stepped() + "      m--;\n    }"
            behind = "(u64)(a - a0) * 5"
        elif form == 5:
            loop = (f"    if (n > 0) {{\n    {count} m = ({count})n;\n    do {{\n" + stepped() +
                    "    } while (--m != 0);\n    }")
        elif form == 6:
            loop = (f"    if (n > 0) {{\n    {count} m = ({count})n;\n    do {{\n" + stepped() +
                    "    } while (--m);\n    }")
            behind = "(u64)(b - b0)"
        elif form == 7:
            loop = (f"    {count} i = 0;\n    if (n > 0) {{\n    do {{\n" + indexed("i") +
                    f"    }} while (++i < ({count})n);\n    }}")
            behind = "(u64)i * 7"
        elif form == 8:
            # From a start that is not zero, and may be behind the limit.
            loop = (f"    {count} i = ({count})(p & 3);\n    for (; i < ({count})n; i++) {{\n" + indexed("i") +
                    "    }")
            behind = "(u64)i"
        elif form == 9:
            limit = self.r.randrange(3, 12)
            loop = f"    for (int i = 0; i <= {limit}; i++) {{\n" + indexed("i") + "    }"
        elif form == 10:
            limit = self.r.randrange(3, 12)
            loop = f"    for ({count} i = {limit}; i >= 1; i--) {{\n" + indexed(f"{limit} - (long)i") + "    }"
        elif form == 11:
            loop = f"    for ({count} i = 0; i < ({count})n; i++) {{\n" + trip("a[i]", "*b++", "r[i]") + "    }"
            behind = "(u64)(b - b0)"
        elif form == 12:
            # The test reads the counter before it is stepped.
            loop = (f"    if (n > 0) {{\n    {count} m = ({count})n;\n    do {{\n" + stepped() +
                    "    } while (m-- != 1);\n    }")
        else:
            loop = "    while (n-- > 0) {\n" + stepped() + "    }"
            behind = "(u64)n"
        start = self.pick("0", "0", "0", "0", "1", "p & 1", "(p >> 63)", "q < p", self.initial(word))
        # `f0` of a program of limb loops is where the negative control
        # plants its wrong carry.
        plant = ["#ifdef FUZZ_PLANT", "    c = !c;", "#endif"] if index == 0 else []
        body = [f"static NOINLINE u64 {name}({word} *r, const {word} *a, const {word} *b, long n, u64 p, u64 q)",
                "{", f"    {kind} c = {start}; u64 h = q; {word} last = 0;",
                f"    const {word} *a0 = a, *b0 = b; {word} *r0 = r;", "    (void)a0; (void)b0; (void)r0;", loop,
                *plant, f"    return (u64)c + h + last + {behind};", "}"]
        self.text.append("\n".join(body))
        self.functions.append((name, "limbs", word))
        self.shapes[name] = (f"{'plain' if plain else 'any'} loop {form}, {self.spelling}, {word} limbs, "
                             f"{count} counter, {kind} carry from "
                             f"`{start}`" + (f", `{self.extra.strip()}`" if self.extra else ""))

    def column(self, word: str, i: str) -> list[str]:
        """One sum or difference kept in `acc`, and what is done with its
        carry: the forms of a multi-word accumulator."""
        k = self.r.randrange(2)
        word_k = f"acc[{k}]"
        lo = self.pick(f"a[{i}]", f"b[{i}]", "lo", "lo", "lo", f"({word})p", self.r.choice(CONSTANTS[word]))
        s, old, c = (self.fresh(stem) for stem in ("s", "o", "cc"))
        # What may stand between a sum and the reading of its carry. The
        # store through `alias` may be to the sum or to the addend.
        between = self.pick("", "", "", f"acc[2] = {lo};", "sink(hi);", f"acc[{1 - k}] ^= 1;", "h += top;",
                            f"*alias = ({word})h;", f"h = h * 31 + {word_k};")
        form = self.r.randrange(12)
        if form == 0:
            lines = [f"{word_k} += {lo};", between, f"hi += {self.less(word_k, lo)};"]
        elif form == 1:
            lines = [f"{word} {old} = {word_k}; {word_k} = {old} + {lo};", between,
                     f"hi += {self.less(word_k, old)};"]
        elif form == 2:
            lines = [f"{word} {s} = {word_k} + {lo}; {word_k} = {s};", between, f"if ({self.less(s, lo)}) hi++;"]
        elif form == 3:
            lines = [f"{word} {s} = {word_k} + {lo}; acc[2] = {s};", f"hi += {self.less(s, lo)}; {word_k} = {s};"]
        elif form == 4:
            # The carry rippling through three words.
            lines = [f"acc[0] += {lo}; {word} {c} = {self.less('acc[0]', lo)};", between,
                     f"acc[1] += {c}; top += {self.less('acc[1]', c)};"]
        elif form == 5:
            lines = [f"{word_k} += {lo};",
                     f"hi = hi + ({self.less(word_k, lo)}) + {self.pick('top', '1', f'b[{i}]', 'hi')};"]
        elif form == 6:
            # No carry: with equality, the other way round, another word.
            wrong = self.pick(f"{word_k} <= {lo}", f"{lo} < {word_k}", f"{word_k} < hi", f"{word_k} >= {lo}",
                              f"{word_k} < ({word})({lo} + 1)", f"{word_k} < acc[{1 - k}]")
            lines = [f"{word_k} += {lo};", between, f"hi += {wrong};"]
        elif form == 7:
            lines = [f"{word} {old} = {word_k}; {word_k} = {old} - {lo};", between,
                     f"hi -= {self.less(old, lo)};"]
        elif form == 8:
            lines = [f"hi -= {self.less(word_k, lo)};", between, f"{word_k} -= {lo};"]
        elif form == 9:
            lines = [f"{word} {old} = {word_k}; {word_k} -= {lo};", between, f"hi -= {self.less(old, word_k)};"]
        elif form == 10:
            # Both words of a two-word sum, the way `add_ssaaaa` spells it.
            lines = [f"{word} {s} = acc[0] + {lo};", f"acc[1] = acc[1] + hi + ({self.less(s, lo)});", between,
                     f"acc[0] = {s};"]
        else:
            # And of a two-word difference (`sub_ddmmss`).
            lines = [f"{word} {s} = acc[0] - {lo};", f"acc[1] = acc[1] - hi - ({self.less('acc[0]', lo)});",
                     between, f"acc[0] = {s};"]
        return [line for line in lines if line]

    def accumulate(self, index: int) -> None:
        word = self.pick("u64", "u64", "u32")
        high = self.pick(word, word, "u64")
        name = f"f{index}"
        looped = self.r.random() < 0.6
        i = "i" if looped else "0"
        low = self.pick(f"a[{i}] * b[{i}]", f"a[{i}] + ({word})q", f"a[{i}]", f"({word})(p >> 3)",
                        f"a[{i}] ^ b[{i}]")
        steps = [f"{word} lo = {low};"]
        for _ in range(self.r.randrange(1, 4)):
            steps += self.column(word, i)
        indent = "      " if looped else "    "
        inner = "\n".join(indent + line for line in steps)
        if looped:
            inner = "    for (long i = 0; i < n; i++) {\n" + inner + "\n    }"
        body = [f"static NOINLINE u64 {name}({word} *acc, {word} *alias, const {word} *a, const {word} *b, "
                "long n, u64 p, u64 q)", "{",
                f"    {high} hi = ({high})p; {word} top = ({word})q; u64 h = q + (u64)n;", inner,
                "    return h + hi + top;", "}"]
        self.text.append("\n".join(body))
        self.functions.append((name, "column", word))

    def main(self) -> str:
        lines = ["int main(void)", "{"]
        for name, kind, word in self.functions:
            size = "LONG" if kind == "limbs" else "LIMBS"
            lines.append("    {")
            lines.append(f"        u64 h = 0xcbf29ce484222325ull; {word} a[{size}], b[{size}], r[{size} + 1];")
            lines.append("        for (int trial = 0; trial < 96; trial++) {")
            lines.append(f"            for (int i = 0; i < {size}; i++) {{ a[i] = ({word})rnd(); b[i] = ({word})rnd(); "
                         f"r[i] = ({word})(0x5a5a5a5a5a5a5a5aull + i); }}")
            lines.append(f"            r[{size}] = 0x77; u64 p = rnd(), q = rnd();")
            # Now and then in place: the result over either operand, or one
            # limb ahead of the first.
            lines.append(f"            {word} *dst = trial % 5 == 3 ? a : trial % 7 == 5 ? b : "
                         "trial % 11 == 7 ? a + 1 : r;")
            if kind == "chain":
                lines.append(f"            h = fold(h, {name}(a, b, p, q, dst));")
            elif kind == "column":
                # The pointer written through in between: the sum, the word
                # above it, the addends, or nothing that matters.
                lines.append(f"            {word} apart = 0, *alias = trial % 3 == 1 ? dst : trial % 4 == 2 ? dst + 1 : "
                             "trial % 9 == 4 ? a : trial % 13 == 6 ? b + 2 : &apart;")
                lines.append(f"            h = fold(h, {name}(dst, alias, a, b, trial % (LIMBS + 1), p, q));")
                lines.append("            h = fold(h, apart);")
            else:
                lines.append(f"            long n = trial % (dst == a + 1 ? {size} : {size} + 1);")
                lines.append(f"            h = fold(h, {name}(dst, a, b, n, p, q));")
            lines.append(f"            for (int i = 0; i < {size}; i++) h = fold(h, a[i] ^ ((u64)b[i] << 1));")
            lines.append(f"            for (int i = 0; i <= {size}; i++) h = fold(h, r[i]);")
            lines.append("        }")
            lines.append(f'        printf("{name} %016llx %016llx\\n", h, g_sink);')
            lines.append("    }")
        lines += ["    return 0;", "}"]
        return "\n".join(lines)


# (options, environment): every optimization level, the other code models,
# and the carry pass, the split of the limb loops, the peephole and register
# allocation off in turn.
CONFIGS = [
    (("-O0",), {}), (("-O1",), {}), (("-O2",), {}), (("-O3",), {}), (("-Os",), {}), (("-Og",), {}),
    (("-O2", "-march=native"), {}), (("-O2", "-fPIC"), {}), (("-O2", "-m32"), {}),
    (("-O2",), {"CCC_VERIFY_IR": "abort", "CCC_VALIDATE_SSA": "1"}),
    (("-O2",), {"CCC_NO_REGALLOC": "1"}),
    (("-O2",), {"LCCC_NO_PEEPHOLE": "1"}),
    (("-O2",), {"CCC_DISABLE_PASSES": "carry"}),
    (("-O3",), {"CCC_PEEPHOLE_SKIP": "multiword"}),
    (("-O2",), {"CCC_DISABLE_PASSES": "limb_unroll"}),
]
QUICK = [0, 2, 3, 4, 8, 9, 10]
# And for the programs of limb loops: the same, and the split off.
QUICK_LIMBS = QUICK + [14]
# The first of the eight seeds of a run in CI (see the header for how the
# windows were chosen).
ADDERS_FROM = 9
LIMBS_FROM = 9


def build_and_run(compiler: str, options: tuple[str, ...], env: dict[str, str], source: Path,
                  binary: Path) -> tuple[list[str] | None, str]:
    """The program's output lines, or None and why there are none."""
    try:
        built = subprocess.run([compiler, *options, str(source), "-o", str(binary)], capture_output=True,
                               text=True, check=False, env={**os.environ, **env}, timeout=600)
    except subprocess.TimeoutExpired:
        return None, "is still being compiled after ten minutes"
    if built.returncode != 0:
        return None, f"does not compile: {built.stderr.strip()[-300:]}"
    try:
        ran = subprocess.run([str(binary)], capture_output=True, text=True, check=False, timeout=120)
    except subprocess.TimeoutExpired:
        return None, "does not terminate"
    if ran.returncode != 0:
        return None, f"exits with {ran.returncode}"
    return ran.stdout.splitlines(), ""


def check_source(what: str, source: Path, args, configs, tmp: Path, plant: bool = False) -> list[str]:
    """What lccc's programs of `source` print that GCC's does not."""
    findings: list[str] = []
    references: dict[bool, list[str]] = {}
    for narrow in sorted({"-m32" in options for options, _ in configs}):
        options = ("-O2", "-m32") if narrow else ("-O2",)
        lines, why = build_and_run(args.gcc, options, {}, source, tmp / f"ref-{source.stem}{'-m32' if narrow else ''}")
        if lines is None:
            return [f"{what}: the reference program ({' '.join(options)}) {why}"]
        references[narrow] = lines
    for number, (options, env) in enumerate(configs):
        label = " ".join(options) + "".join(f" {key}={value}" for key, value in env.items())
        extra = ("-DFUZZ_PLANT",) if plant else ()
        lines, why = build_and_run(args.lccc, options + extra, env, source, tmp / f"lccc-{source.stem}-{number}")
        want = references["-m32" in options]
        if lines is None:
            findings.append(f"{what}: [{label}] lccc's program {why}")
        elif lines != want:
            names = [w.split()[0] for w, got in zip(want, lines) if w != got] or ["(the number of lines)"]
            findings.append(f"{what}: [{label}] differs from GCC in {', '.join(names)}")
    if findings and args.keep and not plant:
        shutil.copy(source, Path(args.keep) / source.name)
    return findings


def carry_runs(lccc: str, source: Path, env: dict[str, str]) -> dict[str, int] | None:
    """For each function of `source` at -O2, its longest run of `adc`/`sbb`
    -- no label, no branch and no `setc`/`bt` between one and the next.
    None where lccc does not compile for x86-64."""
    built = subprocess.run([lccc, "-O2", "-S", str(source), "-o", "-"], capture_output=True, text=True,
                           check=False, timeout=600, env={**os.environ, **env})
    if built.returncode != 0 or "%rsp" not in built.stdout:
        return None
    longest: dict[str, int] = {}
    current, run = None, 0
    for line in built.stdout.splitlines():
        text = line.strip()
        head = re.match(r"^([A-Za-z_]\w*):", line)
        if head:
            current, run = head.group(1), 0
        elif current is None or not text or text.startswith("#"):
            continue
        elif re.match(r"(adc|sbb)[bwlq]?\s", text):
            run += 1
            longest[current] = max(longest.get(current, 0), run)
        elif text.endswith(":") or re.match(r"(setc|setb|setnae|bt[lq]?)\s|(j[a-z]+|call[q]?|ret[q]?)\b", text):
            run = 0
    return longest


def split_loops(lccc: str, source: Path, names: list[str]) -> list[str] | None:
    """The functions among `names` that `limb_unroll` split: four `adc`/`sbb`
    in a row with the pass and not without it (a loop of a constant few
    trips is unrolled whole, by another pass, and is not counted). None
    where lccc does not compile for x86-64."""
    with_split = carry_runs(lccc, source, {})
    without = carry_runs(lccc, source, {"CCC_DISABLE_PASSES": "limb_unroll"})
    if with_split is None or without is None:
        return None
    return [name for name in names if with_split.get(name, 0) >= 4 > without.get(name, 0)]


def check_seed(seed: int, args, configs, tmp: Path, plant: bool = False
               ) -> tuple[list[str], int, list[str] | None]:
    """(findings, functions compared, which of the limb loops are split)
    for one seed."""
    program = Program(seed, args.functions, args.limb_loops)
    source = tmp / f"seed{seed}.c"
    source.write_text(program.source())
    split = None if plant or not program.shapes else split_loops(args.lccc, source, list(program.shapes))
    return check_source(f"seed {seed}", source, args, configs, tmp, plant), len(program.functions), split


def census(args) -> int:
    """Which limb loops of the seeds are split, one a line: what
    `MUST_SPLIT` is chosen from, and where to look for the loop that ought
    to be split and is not."""
    loops = split = 0
    with tempfile.TemporaryDirectory(prefix="lccc-carry-fuzz-") as tmp:
        for seed in range(args.start, args.start + args.seeds):
            program = Program(seed, args.functions, args.limb_loops)
            source = Path(tmp) / f"seed{seed}.c"
            source.write_text(program.source())
            done = split_loops(args.lccc, source, list(program.shapes))
            if done is None:
                print("FAIL: lccc does not compile this for x86-64")
                return 1
            for name, shape in program.shapes.items():
                print(f"seed {seed} {name} {'split' if name in done else 'whole'}: {shape}")
            loops += len(program.shapes)
            split += len(done)
    print(f"carry-chain-fuzz: seeds {args.start}..{args.start + args.seeds - 1}, {split} of {loops} limb loops "
          f"split at -O2 ({split * 100 // max(1, loops)} in a hundred)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default=None, help="the reference compiler (default: gcc-16, else gcc)")
    parser.add_argument("--seeds", type=int, default=8, help="how many programs")
    parser.add_argument("--start", type=int, default=None,
                        help=f"the first seed (default: {ADDERS_FROM}, with --limbs {LIMBS_FROM})")
    parser.add_argument("--functions", type=int, default=10, help="functions in a program")
    parser.add_argument("--limbs", action="store_true",
                        help=f"programs of {LIMB_LOOPS} limb loops and nothing else, and not the fixture")
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 2)
    parser.add_argument("--full", action="store_true", help="all configurations, not the quick ones")
    parser.add_argument("--keep", default=None, help="a directory for the sources that differ")
    parser.add_argument("--dump", type=int, default=None, help="print the program of this seed and stop")
    parser.add_argument("--census", action="store_true",
                        help="with --limbs: say which limb loops are split, and what each is made of, and stop")
    args = parser.parse_args()
    args.limb_loops = 0
    if args.limbs:
        args.functions, args.limb_loops = 0, LIMB_LOOPS
    if args.start is None:
        args.start = LIMBS_FROM if args.limbs else ADDERS_FROM
    if args.dump is not None:
        print(Program(args.dump, args.functions, args.limb_loops).source())
        return 0
    if args.census:
        if not args.limbs:
            parser.error("--census is a census of limb loops: give --limbs")
        return census(args)
    args.gcc = args.gcc or shutil.which("gcc-16") or shutil.which("gcc")
    if args.gcc is None:
        print("FAIL: no reference compiler (gcc-16 or gcc)")
        return 1
    if args.keep:
        Path(args.keep).mkdir(parents=True, exist_ok=True)
    configs = CONFIGS if args.full else [CONFIGS[i] for i in (QUICK_LIMBS if args.limbs else QUICK)]
    problems: list[str] = []
    compared = split = 0
    x86_64 = True
    with tempfile.TemporaryDirectory(prefix="lccc-carry-fuzz-") as tmp:
        # The gate itself first: a planted wrong carry is found, in `f0` and
        # nowhere else, with and without optimization.
        control = check_seed(args.start, args, [CONFIGS[0], CONFIGS[2]], Path(tmp), plant=True)[0]
        if len(control) != 2 or not all(found.endswith("differs from GCC in f0") for found in control):
            problems.append(f"negative control: a planted carry in f0 was reported as {control}")
        if not args.limbs:
            # The adders somebody did think of, through the same configurations.
            problems += check_source("carry_chain_adders.c", FIXTURE, args, configs, Path(tmp))
        with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
            for findings, functions, split_here in pool.map(
                    lambda seed: check_seed(seed, args, configs, Path(tmp)),
                    range(args.start, args.start + args.seeds)):
                problems += findings
                compared += functions
                split += len(split_here or [])
                x86_64 = x86_64 and (split_here is not None or not args.limbs)
    what = f"seeds {args.start}..{args.start + args.seeds - 1}"
    if args.limbs:
        # The limb loops are the loops the split is for: enough of them split.
        if x86_64 and (args.start, args.seeds) == (LIMBS_FROM, 8) and split < MUST_SPLIT:
            problems.append(f"only {split} of {compared} limb loops are split at -O2 (four adc/sbb in a row "
                            f"with limb_unroll and not without), at least {MUST_SPLIT} expected")
        what += f", {compared} limb loops ({split} split)" if x86_64 else f", {compared} limb loops"
    else:
        what += f", {compared} functions and carry_chain_adders.c"
    for problem in problems:
        print(f"FAIL: {problem}")
    print(f"carry-chain-fuzz: {what}, {len(configs)} configurations each against GCC, "
          f"1 negative control, {'FAIL' if problems else 'PASS'}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
