#!/usr/bin/env python3
"""Calls that may not return: which ones are deleted, moved, or merged.

`pure` and `const` license two transformations that "writes no memory" does
not: deleting a call whose result is unused, and running a call earlier than
written.  Both need the call to come back.  LCCC used to infer the attributes
from memory effects alone, so a bounds check that traps and a function that
parks in `for (;;)` were deleted, and a callee that divides was run above the
test that guards its loop (tests/regression/call_may_not_return.c,
licm_call_not_speculated.c).  The repair withholds the attributes from a
function not known to return -- and has to know that a function which counts
to a bound does return, or every function with a loop loses them
(`passes::loop_termination`).

This gate pins both directions on the generated code, and the values at run
time:

* dropped -- a call with an unused result is gone when its callee is known to
  return (straight-line code and counted loops: `<`, `!=`, `>` with an odd
  step against any bound; `<=` and even steps against a constant bound far
  enough from the end of the range), and KEPT, asserted as kept, when it is
  not (`for (;;)`, a trap, `i <= n` and `i += 2` against a variable bound, a
  list walk, a counter narrower than its bound);
* hoisted -- the same callees called with loop-invariant arguments in a loop
  that runs its body at least once: one call ahead of the loop, or the call
  still inside it;
* merged -- two identical calls are one call either way: the second returns
  what the first did, if the first returned at all;
* position -- a call that divides is hoisted above inline assembly, which
  falls through (GCC's documented assumption), and stays behind an asm goto,
  behind a call that may exit, and inside a branch of the loop body;
* library -- `const` in a header is not "defined for every argument": an
  external function declared so, and glibc's `div`, are not run above the
  `if (b == 0) return -1;` ahead of their loop.

Every expectation is what GCC 16.2 does with the same source at -O2 and -O3,
with three stated exceptions where LCCC keeps a call GCC removes (STRICTER
below).  The reverse -- LCCC removing a call GCC keeps -- has no exceptions.
(Under -fwrapv GCC also keeps `i += 3` against a variable bound; a counter
with an odd step visits every value, so that loop ends there too.)

Where a call sits is decided on the control-flow graph of the assembly: a
call is "in a loop" if its block lies on a cycle.  Block layout does not
matter.

Usage: check_call_return_facts.py --lccc target/fastbuild/lccc [--gcc gcc-16]
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile

# Callees: none writes memory.  RETURNS are known to come back; the others
# are not, each for the reason beside it.
CALLEES = r"""
#define CALLEE __attribute__((noinline)) static
struct node { struct node *next; int v; };
void ext(int);

CALLEE int straight(int a, int b)   { return a * b + 1; }
CALLEE int quot(int a, int b)       { return a / b; }
CALLEE int up_lt(int n)             { int s = 0; for (int i = 0; i < n; i++) s ^= i * 7 + (s >> 3); return s; }
CALLEE int up_ne(int n)             { int s = 0; for (int i = 0; i != n; i++) s ^= i * 7 + (s >> 3); return s; }
CALLEE int up_lt_u(unsigned n)      { int s = 0; for (unsigned i = 0; i < n; i++) s ^= i * 7 + (s >> 3); return s; }
CALLEE int up_lt_3(int n)           { int s = 0; for (int i = 0; i < n; i += 3) s ^= i * 7 + (s >> 3); return s; }
CALLEE int down_gt(int n)           { int s = 0; for (int i = n; i > 0; i--) s ^= i * 7 + (s >> 3); return s; }
CALLEE int down_ne(unsigned n)      { int s = 0; while (n-- != 0) s ^= n * 7 + (s >> 3); return s; }
CALLEE int up_le_c(int k)           { int s = 0; for (int i = 0; i <= 100; i++) s ^= i * k + (s >> 3); return s; }
CALLEE int up_lt_2c(int k)          { int s = 0; for (int i = 0; i < 64; i += 2) s ^= i * k + (s >> 3); return s; }
CALLEE long up_lt_l(long n)         { long s = 0; for (long i = 0; i < n; i++) s ^= i * 7 + (s >> 3); return s; }
CALLEE long up_lt_z(unsigned long n){ long s = 0; for (unsigned long i = 0; i < n; i++) s ^= i * 7 + (s >> 3); return s; }
CALLEE int nested(int n)            { int s = 0; for (int i = 0; i < n; i++) for (int j = 0; j < 8; j++) s ^= i * j + (s >> 3); return s; }
CALLEE int do_while(int n)          { int s = 0, i = 0; do { s ^= i * 7 + (s >> 3); } while (++i < n); return s; }

/* for (;;) has a constant controlling expression: C11 6.8.5p6 does not let
 * an implementation assume it ends. */
CALLEE int forever(int stop)        { if (stop) for (;;) { } return 0; }
CALLEE int traps(int i)             { if (i >= 4) __builtin_trap(); return 0; }
/* n == UINT_MAX: i <= n holds for every i. */
CALLEE int up_le_u(unsigned n)      { int s = 0; for (unsigned i = 0; i <= n; i++) s ^= i * 7 + (s >> 3); return s; }
/* n odd: i steps over it and wraps. */
CALLEE int up_ne_2(unsigned n)      { int s = 0; for (unsigned i = 0; i != n; i += 2) s ^= i * 7 + (s >> 3); return s; }
/* n == UINT_MAX: the last even value is below it, and the next is 0. */
CALLEE int up_lt_2(unsigned n)      { int s = 0; for (unsigned i = 0; i < n; i += 2) s ^= i * 7 + (s >> 3); return s; }
CALLEE int walks(const struct node *p) { int s = 0; while (p) { s ^= p->v; p = p->next; } return s; }
/* n > 255: the counter wraps below it. */
CALLEE int narrow(int n)            { int s = 0; for (unsigned char i = 0; i < n; i++) s ^= i * 7 + (s >> 3); return s; }

/* Ends, and GCC shows it; LCCC does not (see STRICTER). */
CALLEE int halves(unsigned n)       { int s = 0; while (n) { s++; n >>= 1; } return s; }
CALLEE int up_le_s(int n)           { int s = 0; for (int i = 0; i <= n; i++) s ^= i * 7 + (s >> 3); return s; }
CALLEE int up_lt_2s(int n)          { int s = 0; for (int i = 0; i < n; i += 2) s ^= i * 7 + (s >> 3); return s; }
"""

# name -> parameter type of the one-argument callee.
RETURNS = {
    "up_lt": "int", "up_ne": "int", "up_lt_u": "unsigned", "up_lt_3": "int",
    "down_gt": "int", "down_ne": "unsigned", "up_le_c": "int", "up_lt_2c": "int",
    "up_lt_l": "long", "up_lt_z": "unsigned long", "nested": "int", "do_while": "int",
}
MAY_NOT_RETURN = {
    "forever": "int", "traps": "int", "up_le_u": "unsigned", "up_ne_2": "unsigned",
    "up_lt_2": "unsigned", "walks": "const struct node *", "narrow": "int",
}
# GCC 16.2 removes and hoists these calls; LCCC keeps them.
STRICTER = {
    # A shift towards zero ends; the proof here counts additions only.
    "halves": "unsigned",
    # GCC assumes signed overflow does not happen.  The proof assumes nothing
    # of the kind, so it also holds under -fwrapv -- where `i <= INT_MAX`
    # never fails and an even step can pass an odd bound and wrap.
    "up_le_s": "int",
    "up_lt_2s": "int",
}
ONE_ARG = {**RETURNS, **MAY_NOT_RETURN, **STRICTER}
KEPT = {**MAY_NOT_RETURN, **STRICTER}


def callers() -> str:
    out = []
    for name, ty in ONE_ARG.items():
        out.append(f"void drop_{name}({ty} x) {{ (void){name}(x); }}")
        out.append(
            f"int hoist_{name}({ty} x, int trips) "
            f"{{ int s = 0, j = 0; do {{ s += (int){name}(x) ^ j; }} while (++j < trips); return s; }}")
        out.append(f"int twice_{name}({ty} x) {{ return (int){name}(x) + (int){name}(x); }}")
    out.append(r"""
void drop_straight(int a, int b) { (void)straight(a, b); }
void drop_quot(int a, int b) { (void)quot(a, b); }
int hoist_quot(int a, int b, int trips)
    { int s = 0, j = 0; do { s += quot(a, b) ^ j; } while (++j < trips); return s; }
int twice_quot(int a, int b) { return quot(a, b) + quot(a, b); }

int after_asm(int a, int b, int trips)
    { int s = 0, j = 0; do { __asm__ volatile("nop"); s += quot(a, b) ^ j; } while (++j < trips); return s; }
int after_asm_goto(int a, int b, int trips)
    { int s = 0, j = 0; do { __asm__ goto("nop" : : : : out); s += quot(a, b) ^ j; } while (++j < trips); out: return s; }
int after_call(int a, int b, int trips)
    { int s = 0, j = 0; do { ext(b); s += quot(a, b) ^ j; } while (++j < trips); return s; }
int before_call(int a, int b, int trips)
    { int s = 0, j = 0; do { s += quot(a, b) ^ j; ext(b); } while (++j < trips); return s; }
int conditional(int a, int b, int trips)
    { int s = 0, j = 0; do { if (j & 1) s += quot(a, b); } while (++j < trips); return s; }
""")
    return "\n".join(out)


SHAPES = CALLEES + callers()

# What a caller's assembly must show, as a test of (calls to the callee in a
# block on a cycle, calls to it elsewhere).  A compiler may copy a loop, so a
# call that stays in one is "at least one"; a call that leaves is "none".
GONE, ONCE, IN_LOOP, AHEAD = "no call", "one call, not in a loop", "a call in the loop", "a call ahead of the loop"
HOLDS = {
    GONE: lambda inside, outside: inside == 0 and outside == 0,
    ONCE: lambda inside, outside: inside == 0 and outside == 1,
    IN_LOOP: lambda inside, outside: inside >= 1,
    AHEAD: lambda inside, outside: inside == 0 and outside >= 1,
}


def expectations() -> dict[str, tuple[str, str]]:
    table: dict[str, tuple[str, str]] = {}
    for name in RETURNS:
        table[f"drop_{name}"] = (GONE, name)
        table[f"hoist_{name}"] = (AHEAD, name)
    for name in KEPT:
        table[f"drop_{name}"] = (ONCE, name)
        table[f"hoist_{name}"] = (IN_LOOP, name)
    for name in ONE_ARG:
        table[f"twice_{name}"] = (ONCE, name)
    table.update({
        "drop_straight": (GONE, "straight"),
        # A division fault is not a way of not returning that GCC counts, and
        # the call is not speculated by being deleted.
        "drop_quot": (GONE, "quot"),
        "hoist_quot": (AHEAD, "quot"),
        "twice_quot": (ONCE, "quot"),
        "after_asm": (AHEAD, "quot"),
        "after_asm_goto": (IN_LOOP, "quot"),
        "after_call": (IN_LOOP, "quot"),
        "before_call": (AHEAD, "quot"),
        "conditional": (IN_LOOP, "quot"),
    })
    return table


RUNTIME_MAIN = r"""
#include <stdio.h>
void ext(int x) { (void)x; }
int main(void) {
    static volatile int zero;
    static struct node c = {0, 5}, b = {&c, 3}, a = {&b, 2};
    long acc = 0;
#define MIX(v) acc = acc * 31 + (long)(v)
#define ALL(name, x) do { drop_##name(x); MIX(hoist_##name(x, 3 + zero)); MIX(twice_##name(x)); } while (0)
    ALL(up_lt, 9 + zero); ALL(up_ne, 9 + zero); ALL(up_lt_u, 9u + zero); ALL(up_lt_3, 10 + zero);
    ALL(down_gt, 9 + zero); ALL(down_ne, 9u + zero); ALL(up_le_c, 3 + zero); ALL(up_lt_2c, 3 + zero);
    ALL(up_lt_l, 9L + zero); ALL(up_lt_z, 9UL + zero); ALL(nested, 5 + zero); ALL(do_while, 9 + zero);
    ALL(forever, zero); ALL(traps, 1 + zero); ALL(up_le_u, 9u + zero); ALL(up_ne_2, 8u + zero);
    ALL(up_lt_2, 9u + zero); ALL(walks, &a); ALL(narrow, 9 + zero);
    ALL(halves, 77u + zero); ALL(up_le_s, 9 + zero); ALL(up_lt_2s, 9 + zero);
    drop_straight(3 + zero, 4); drop_quot(9 + zero, 3);
    MIX(hoist_quot(9, 3 + zero, 3)); MIX(twice_quot(9, 3 + zero));
    MIX(after_asm(9, 3 + zero, 3)); MIX(after_asm_goto(9, 3 + zero, 3));
    MIX(after_call(9, 3 + zero, 3)); MIX(before_call(9, 3 + zero, 3));
    MIX(conditional(9, 3 + zero, 4));
    printf("%ld\n", acc);
    return 0;
}
"""

# `const` by declaration only: the definitions are in the other file.
LIBRARY_USER = r"""
#include <stdio.h>
#include <stdlib.h>
extern int safe_quot(int a, int b) __attribute__((const));
extern void require_nonzero(int b);

__attribute__((noinline)) int guarded_extern(int a, int b, int n) {
    int s = 0, j = 0;
    if (b == 0) return -1;
    do { s += safe_quot(a, b); } while (++j < n);
    return s;
}
__attribute__((noinline)) int guarded_div(int a, int b, int n) {
    int s = 0, j = 0;
    if (b == 0) return -1;
    do { s += div(a, b).quot; } while (++j < n);
    return s;
}
__attribute__((noinline)) int required_extern(int a, int b, int n) {
    int s = 0, j = 0;
    do { require_nonzero(b); s += safe_quot(a, b); } while (++j < n);
    return s;
}
int main(void) {
    static volatile int zero;
    printf("%d %d %d\n", guarded_extern(9, 3 + zero, 2), guarded_div(9, 3 + zero, 2),
           required_extern(9, 3 + zero, 2));
    fflush(stdout);
    printf("%d\n", guarded_extern(9, zero, 2));
    fflush(stdout);
    printf("%d\n", guarded_div(9, zero, 2));
    fflush(stdout);
    printf("%d\n", required_extern(9, zero, 2));
    return 0;
}
"""
LIBRARY_DEFS = r"""
#include <stdio.h>
#include <stdlib.h>
int safe_quot(int a, int b) { return a / b; }
void require_nonzero(int b) { if (b == 0) { puts("refused"); exit(0); } }
"""
LIBRARY_OUTPUT = "6 6 6\n-1\n-1\nrefused\n"


def fail(message: str) -> None:
    raise SystemExit(f"FAIL: {message}")


def run(cmd: list[str], check: bool = True) -> subprocess.CompletedProcess:
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    if check and proc.returncode != 0:
        fail(f"{' '.join(cmd)} (rc {proc.returncode})\n{proc.stdout}{proc.stderr}")
    return proc


FUNCTION = re.compile(r"^([A-Za-z_][\w.]*):$")
LABEL = re.compile(r"^(\.L[\w.$]+):$")
JUMP = re.compile(r"^j(\w+)\s+(\S+)$")
CALL = re.compile(r"^call\w*\s+\*?([A-Za-z_][\w.]*)")
ENDS = ("ret", "ud2", "hlt")


def functions_of(asm: str) -> dict[str, list[str]]:
    """name -> the instruction and label lines of each function."""
    out: dict[str, list[str]] = {}
    current = None
    for raw in asm.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        m = FUNCTION.match(line)
        if m and not line.startswith(".L"):
            current = out.setdefault(m.group(1), [])
            continue
        if current is None:
            continue
        if line.startswith(".size") or line == ".cfi_endproc":
            current = None
        elif not line.startswith(".") or LABEL.match(line):
            current.append(line)
    return out


def call_sites(name: str, body: list[str], callee: str) -> tuple[int, int]:
    """(calls to `callee` in a block on a cycle, calls to it elsewhere).

    Blocks are split at labels and after every jump; a jump to a symbol that
    is not a label of the function is a tail call.
    """
    labels = {m.group(1) for line in body if (m := LABEL.match(line))}
    blocks: list[dict] = [{"labels": [], "calls": 0, "succs": [], "falls": True}]

    def fresh() -> None:
        if blocks[-1]["labels"] or blocks[-1]["calls"] or blocks[-1]["succs"] or not blocks[-1]["falls"] \
                or blocks[-1].get("lines"):
            blocks.append({"labels": [], "calls": 0, "succs": [], "falls": True})

    for line in body:
        m = LABEL.match(line)
        if m:
            fresh()
            blocks[-1]["labels"].append(m.group(1))
            continue
        block = blocks[-1]
        block["lines"] = block.get("lines", 0) + 1
        m = JUMP.match(line)
        if m:
            kind, target = m.group(1), m.group(2)
            if target.startswith("*"):
                fail(f"{name}: an indirect jump; the gate reads direct control flow only:\n" + "\n".join(body))
            if target in labels:
                block["succs"].append(target)
            elif target.split("@")[0].split(".")[0] == callee:
                block["calls"] += 1
            if kind == "mp":
                block["falls"] = False
            fresh()
            continue
        m = CALL.match(line)
        if m and m.group(1).split("@")[0].split(".")[0] == callee:
            block["calls"] += 1
        if line.split()[0] in ENDS:
            block["falls"] = False
            fresh()
    index = {label: i for i, block in enumerate(blocks) for label in block["labels"]}
    succs = [[index[t] for t in block["succs"]] + ([i + 1] if block["falls"] and i + 1 < len(blocks) else [])
             for i, block in enumerate(blocks)]
    # A block is on a cycle if it can reach itself.
    on_cycle = []
    for start in range(len(blocks)):
        seen, work = set(), list(succs[start])
        while work:
            node = work.pop()
            if node not in seen:
                seen.add(node)
                work.extend(succs[node])
        on_cycle.append(start in seen)
    inside = sum(block["calls"] for block, cyc in zip(blocks, on_cycle) if cyc)
    total = sum(block["calls"] for block in blocks)
    return inside, total - inside


def check_shapes(lccc: str, tmp: str, opt: str) -> int:
    src = os.path.join(tmp, "shapes.c")
    with open(src, "w") as f:
        f.write(SHAPES)
    functions = functions_of(run([lccc, opt, "-S", src, "-o", "-"]).stdout)
    table = expectations()
    wrong = []
    for name, (expected, callee) in sorted(table.items()):
        if name not in functions:
            fail(f"{opt}: no function {name} in the assembly")
        got = call_sites(name, functions[name], callee)
        if not HOLDS[expected](*got):
            wrong.append(f"  {name}: calls to {callee} (in a loop, elsewhere) = {got}, expected {expected}")
    if wrong:
        fail(f"{opt}: {len(wrong)} of {len(table)} call sites differ:\n" + "\n".join(wrong))
    return len(table)


def check_values(lccc: str, gcc: str, tmp: str) -> None:
    src = os.path.join(tmp, "values.c")
    with open(src, "w") as f:
        f.write(SHAPES + RUNTIME_MAIN)
    ref_bin = os.path.join(tmp, "values.gcc")
    run([gcc, "-O2", src, "-o", ref_bin])
    reference = run([ref_bin]).stdout
    if not reference.strip():
        fail("the reference build printed nothing")
    for opt in ("-O0", "-O1", "-O2", "-O3", "-Os"):
        binary = os.path.join(tmp, "values.lccc")
        run([lccc, opt, src, "-o", binary])
        got = run([binary]).stdout
        if got != reference:
            fail(f"{opt}: values differ from {gcc} -O2: {got!r} vs {reference!r}")


def check_library(lccc: str, gcc: str, tmp: str) -> None:
    user, defs = os.path.join(tmp, "user.c"), os.path.join(tmp, "defs.c")
    with open(user, "w") as f:
        f.write(LIBRARY_USER)
    with open(defs, "w") as f:
        f.write(LIBRARY_DEFS)
    for compiler, opts in ((gcc, ("-O2",)), (lccc, ("-O1", "-O2", "-O3", "-Os"))):
        for opt in opts:
            binary = os.path.join(tmp, "library.bin")
            run([compiler, opt, user, defs, "-o", binary])
            proc = run([binary], check=False)
            if proc.returncode != 0 or proc.stdout != LIBRARY_OUTPUT:
                fail(f"{compiler} {opt}: a call declared const ran where the program did not call it "
                     f"(rc {proc.returncode}, printed {proc.stdout!r}, expected {LIBRARY_OUTPUT!r})")


def self_test() -> None:
    """The reader of the assembly, on shapes whose answer is known."""
    looped = ["pushq %rbx", ".L1:", "call f@PLT", "addl $1, %ebx", "cmpl $3, %ebx", "jl .L1", "ret"]
    assert call_sites("t", looped, "f") == (1, 0)
    ahead = ["call f", ".L1:", "addl $1, %ebx", "jne .L1", "ret"]
    assert call_sites("t", ahead, "f") == (0, 1)
    # A loop block laid out after the return is still a loop block.
    out_of_line = [".L1:", "testl %edi, %edi", "jne .L3", ".L2:", "subl $1, %esi", "jne .L1", "ret",
                   ".L3:", "call f", "jmp .L2"]
    assert call_sites("t", out_of_line, "f") == (1, 0)
    # A tail call is a call, and leaves the function.
    tail = [".L1:", "subl $1, %edi", "jne .L1", "jmp f"]
    assert call_sites("t", tail, "f") == (0, 1)
    # Another callee, and a clone's suffix.
    assert call_sites("t", ["call g", "call f.constprop.0", "ret"], "f") == (0, 1)
    assert call_sites("t", ["call g", "ret"], "f") == (0, 0)
    parsed = functions_of("\t.text\nf:\n\tcall g # x\n.L2:\n\tret\n\t.size f, .-f\nh:\n\tret\n")
    assert parsed == {"f": ["call g", ".L2:", "ret"], "h": ["ret"]}, parsed
    # Every caller is expected to show something, and both answers occur.
    table = expectations()
    assert len(table) == 3 * len(ONE_ARG) + 9
    assert sum(1 for want, _ in table.values() if want == GONE) == len(RETURNS) + 2
    assert sum(1 for want, _ in table.values() if want == IN_LOOP) == len(KEPT) + 3


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc")
    ap.add_argument("--gcc", default="gcc-16")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    self_test()
    if args.self_test:
        print("PASS: call-return-facts self-test")
        return 0
    if not args.lccc:
        ap.error("--lccc is required")
    with tempfile.TemporaryDirectory() as tmp:
        sites = 0
        for opt in ("-O2", "-O3"):
            sites = check_shapes(args.lccc, tmp, opt)
        check_values(args.lccc, args.gcc, tmp)
        check_library(args.lccc, args.gcc, tmp)
    print(f"PASS: call return facts ({sites} call sites at -O2 and -O3, "
          f"{len(RETURNS)} counted callees, {len(KEPT)} kept; values and library calls vs {args.gcc})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
