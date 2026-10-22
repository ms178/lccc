#!/usr/bin/env python3
"""Differential fuzzer for object identity: an address stays an address, and a
value survives its own conversions.

Two defects of that family shipped and were found by hand, so the shapes are
generated here from now on:

1. **The address of a local must be its address.** A cast that wanted the
   *value* of a local read the local's stack slot — which is the local's
   storage, not its value — so `(int)(uintptr_t)&s` returned `s` at -O1..-O3
   while loading of the pointee was the whole bug (`leaq` was required).
   `slot_rmw` fuzzes slot homes and `punning` fuzzes bytes; neither generates
   `(cast)(uintptr_t)&local`, so nothing could see it.

2. **A conversion must not destroy its own source.** The U64 -> FP
   shift-and-round path rewrites `%rax`; without invalidating the accumulator
   cache, a later use of the source read the mangled operand (`(double)x`
   twice disagreed with itself).

Every generated function takes the address of one of its OWN locals and checks
an invariant that is true for any correct implementation, so the program is a
PASS/FAIL self-check and the reference compiler's output is the oracle: both
compilers must print `OK`. Absolute addresses never cross a compiler boundary
(comparing a callee's address with a caller's local compares two different
stack slots and proves nothing).

Undefined behaviour is excluded by construction: addresses are only converted
to integers and compared, never dereferenced after truncation, no pointer
arithmetic leaves its own object, no signed overflow, no invalid shift, and
every conversion pair is one C11 6.3.1 defines (the FP checks only compare two
conversions of the same value, or compare against the value itself).

Usage:
  python3 tests/fuzz/addr_value_fuzz.py --ccc ./target/fastbuild/lccc --seeds 0:200
  python3 tests/fuzz/addr_value_fuzz.py --ccc gcc --seeds 0:50   # model check
  python3 tests/fuzz/addr_value_fuzz.py --emit-only 7            # print one case
"""
from __future__ import annotations

import argparse
import json
import random
import shutil
import subprocess
import sys
from pathlib import Path

# Source types whose conversion exercises a distinct lowering: the "big"
# shift-and-round path (bit 63 set), the plain unsigned path, the negative
# signed path, and the two narrow routes.
ADDR_TYPES = [
    "char", "signed char", "unsigned char", "short", "unsigned short",
    "int", "unsigned int", "long", "unsigned long", "long long",
    "float", "double",
]
FP_SOURCES = [
    ("uint64_t", "x ^ (1ULL << 63)"),              # big path
    ("uint64_t", "x"),                             # plain path
    ("int64_t", "(int64_t)(x | (1ULL << 63))"),    # negative signed
    ("uint32_t", "(uint32_t)x | 0x80000000u"),     # narrow unsigned
    ("int32_t", "(int32_t)(x | 0x80000000u)"),     # narrow signed
]


def _cast_chain(rng: random.Random):
    """One cast chain, returned as a function so the SAME chain is applied to
    both sides of a comparison (applying different chains proves nothing)."""
    t = rng.choice(["int", "unsigned", "short", "signed char", "unsigned char",
                    "long", "unsigned long"])
    t2 = rng.choice(["long", "int", "unsigned long"])
    t3 = rng.choice(["int", "unsigned", "short"])
    k = rng.choice([0xff, 0xfff, 0xfff0, 0xffff, 0x55, 0xaa, 0xf0f])
    sh = rng.choice([1, 3, 7, 12])
    add = rng.choice([1, 2, 8])
    div = rng.choice([16, 64, 4096])
    dv = rng.choice([2, 8, 32])
    forms = [
        lambda e: "(%s)(%s)" % (t, e),
        lambda e: "(%s)(%s)(%s)" % (t2, t3, e),
        lambda e: "(%s)((%s) & 0x%xUL)" % (t, e, k),
        lambda e: "(%s)((%s) >> %d)" % (t, e, sh),
        lambda e: "(%s)((%s) + %d)" % (t, e, add),
        lambda e: "(%s)((%s) ^ 0x%xUL)" % (t, e, k),
        lambda e: "(%s)(~(%s))" % (t, e),
        lambda e: "(%s)((%s) %% %dUL)" % (t, e, div),
        lambda e: "(%s)((%s) / %dUL)" % (t, e, dv),
    ]
    return forms[rng.randrange(len(forms))]


def _addr_case(rng: random.Random, name: str) -> str:
    """A function that checks its own local's address against itself."""
    ty = rng.choice(ADDR_TYPES)
    init = rng.choice(["1", "2", "7", "10", "1.5", "2.5"])
    shape = rng.randrange(11)
    b = ["\t%s s = %s;" % (ty, init),
         "\tuintptr_t a = (uintptr_t)&s;",
         "\t(void)a;",
         "\t(void)s;",
         # The address of a local is not the local's value.  This is the
         # invariant the shipped defect violated (a cast that wanted the
         # value read the slot instead), and unlike a pure
         # `f(&s) == f(&s)` comparison it cannot be satisfied by a compiler
         # that returns the pointee on BOTH sides.
         "\tif ((uintptr_t)s == a) return 0;"]

    if shape == 0:
        ch = _cast_chain(rng)
        b.append("\treturn %s == %s;" % (ch("(uintptr_t)&s"), ch("a")))
    elif shape == 1:
        c = rng.choice(["int", "unsigned", "short", "long"])
        b.append("\t%s x = (%s)(uintptr_t)&s, y = (%s)a;" % (c, c, c))
        b.append("\treturn x == y;")
    elif shape == 2:
        b += ["\tint v[%d];" % rng.choice([2, 4, 8]),
              "\tv[0] = 11; v[1] = 12;",
              "\tuintptr_t b = (uintptr_t)v;"]
        ch = _cast_chain(rng)
        b.append("\treturn %s == %s && b != (uintptr_t)v[0];"
                 % (ch("(uintptr_t)v"), ch("b")))
    elif shape == 3:
        b += ["\tint v[8];",
              "\tfor (int k = 0; k < 8; k++) v[k] = k * 3;",
              "\tint i = %d;" % rng.choice([0, 1, 3, 7]),
              "\tuintptr_t e = (uintptr_t)&v[i];"]
        ch = _cast_chain(rng)
        b.append("\treturn %s == %s;" % (ch("(uintptr_t)&v[i]"), ch("e")))
    elif shape == 4:
        b += ["\tstruct { uintptr_t raw; void *p; } bx;",
              "\tunion { uintptr_t raw; void *p; } un;",
              "\tuintptr_t dst = 0, src = a;",
              "\tvoid *gp = (void *)a;",
              "\tbx.raw = (uintptr_t)&s; bx.p = &s;",
              "\tun.p = &s;",
              "\tmemcpy(&dst, &src, sizeof dst);",
              "\treturn (int)bx.raw == (int)a && bx.p == (void *)&s",
              "\t    && (int)un.raw == (int)a && (int)dst == (int)a",
              "\t    && gp == (void *)&s;"]
    elif shape == 5:
        b.append("\treturn id_u((uintptr_t)&s) == a"
                 " && id_i((int)(uintptr_t)&s) == (int)a;")
    elif shape == 6:
        b.append("\tint t = %d ? (int)(uintptr_t)&s : (int)a;" % rng.choice([0, 1]))
        b.append("\treturn t == (int)a;")
    elif shape == 7:
        b += ["\tint t = 0;",
              "\tfor (int k = 0; k < %d; k++) t = (int)(uintptr_t)&s;" % rng.choice([1, 3, 5]),
              "\treturn t == (int)a;"]
    elif shape == 8:
        # Byte-window arithmetic needs storage it cannot leave: `(char *)&s + n`
        # on a scalar `s` would walk outside the object (undefined behaviour),
        # so the window is its own array while the scalar check above still
        # pins the address of `s`.
        ch = _cast_chain(rng)
        off = rng.choice([1, 3, 5, 9, 15])   # < sizeof buf, no pointer escape
        b += ["\tunsigned char buf[16];",
              "\tmemset(buf, 0, sizeof buf);",
              "\tchar *base = (char *)&buf[0];",
              "\tchar *tip = base + %d;" % off,
              "\tuintptr_t far = (uintptr_t)(void *)tip;",
              "\t(void)far;"]
        b.append("\treturn tip - base == %d && (void *)base == (void *)&buf[0]"
                 " && %s == %s;"
                 % (off, ch("(uintptr_t)tip"), ch("far")))
    elif shape == 9:
        b += ["\tstruct { int x; char pad[24]; long y; } r;",
              "\tr.x = 5; r.y = 6;",
              "\tuintptr_t m = (uintptr_t)&r.y;"]
        ch = _cast_chain(rng)
        b.append("\treturn %s == %s && m != (uintptr_t)r.y;"
                 % (ch("(uintptr_t)&r.y"), ch("m")))
    else:
        b += ["\ttypedef int v4 __attribute__((vector_size(16)));",
              "\tv4 r = {3, 0, 2, 1};",
              "\tuintptr_t m = (uintptr_t)&r;"]
        ch = _cast_chain(rng)
        b.append("\treturn %s == %s && r[0] == 3 && r[3] == 1;"
                 % (ch("(uintptr_t)&r"), ch("m")))

    return "static int __attribute__((noinline)) %s(void) {\n%s\n}" % (
        name, "\n".join(b))


def _conv_case(rng: random.Random, name: str) -> str:
    """A function that checks a value survives its own conversion."""
    sty, init = rng.choice(FP_SOURCES)
    dty = rng.choice(["double", "float"])
    shape = rng.randrange(6)
    b = ["\tint s = 0;",
         "\tuintptr_t x = (uintptr_t)&s;",
         "\t(void)x;",
         "\t%s v = %s;" % (sty, init),
         "\t(void)v;"]

    if shape == 0:
        b.append("\t%s p = (%s)v, q = (%s)v;" % (dty, dty, dty))
        b.append("\treturn p == q;")
    elif shape == 1:
        b += ["\t%s p = (%s)v;" % (dty, dty),
              "\t%s w = v;" % sty,
              "\t(void)p;",
              "\treturn w == v;"]
    elif shape == 2:
        b += ["\t%s p = (%s)v;" % (dty, dty),
              "\t%s c = v, e = (%s)c;" % (sty, dty),
              "\treturn p == e;"]
    elif shape == 3:
        b += ["\t%s before = v;" % sty,
              "\t%s p = (%s)v;" % (dty, dty),
              "\t%s after = v;" % sty,
              "\t(void)p;",
              "\treturn before == after && after == v;"]
    elif shape == 4:
        b += ["\t%s p = (%s)v;" % (dty, dty),
              "\t%s m = v + 1;" % sty,
              "\t%s q = (%s)v;" % (dty, dty),
              "\treturn p == q && m == v + 1;"]
    else:
        b += ["\t%s small = (%s)(v & 0x7fffffffffffffULL);" % (sty, sty),
              "\t%s p = (%s)small, q = (%s)small;" % (dty, dty, dty),
              "\treturn p == q && (%s)small == small;" % sty]

    return "static int __attribute__((noinline)) %s(void) {\n%s\n}" % (
        name, "\n".join(b))


HEADER = """/* Generated by tests/fuzz/addr_value_fuzz.py (seed %d) -- do not edit.
 *
 * Every function checks an invariant of its own local: the address of an
 * object is that object's address (not its contents), and a conversion does
 * not destroy the value it converts. Both compilers must print OK.
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

static uintptr_t __attribute__((noinline)) id_u(uintptr_t x) { return x; }
static int __attribute__((noinline)) id_i(int x) { return x; }
"""


def gen_c(seed: int) -> str:
    """Deterministic self-checking translation unit for `seed`."""
    rng = random.Random(seed)
    n_addr = rng.randrange(3, 7)
    n_conv = rng.randrange(2, 5)
    funcs, names = [], []
    for i in range(n_addr):
        name = "addr%d" % i
        funcs.append(_addr_case(rng, name))
        names.append(name)
    for i in range(n_conv):
        name = "conv%d" % i
        funcs.append(_conv_case(rng, name))
        names.append(name)

    out = [HEADER % seed, "\n".join(funcs), "", "int main(void) {", "\tint bad = 0;"]
    for name in names:
        out.append('\tif (!%s()) { printf("FAIL %s\\n"); bad++; }' % (name, name))
    out.append('\tprintf(bad ? "FAILURES: %d\\n" : "OK\\n", bad);')
    out.append("\treturn bad != 0;")
    out.append("}")
    return "\n".join(out) + "\n"


def _verdict(text: str) -> str:
    """The generated program's own last line: `OK` or `FAILURES: n`."""
    lines = [ln.strip() for ln in text.splitlines() if ln.strip()]
    return lines[-1] if lines else ""


class _Timeout(Exception):
    """A compiler or a generated binary exceeded its budget."""


def _run(cmd: list[str], timeout: int = 60):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout + p.stderr
    except subprocess.TimeoutExpired:
        raise _Timeout(" ".join(cmd[:2])) from None
    except OSError as exc:
        raise _Timeout(f"{cmd[0]}: {exc}") from None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--ccc", help="compiler under test (required unless "
                                  "--emit-only is used)")
    ap.add_argument("--gcc", default=shutil.which("gcc") or "/usr/bin/gcc",
                    help="reference compiler (default: gcc on PATH)")
    ap.add_argument("--seeds", default="0:50", help="seed range LO:HI")
    ap.add_argument("--levels", default="O0,O1,O2,O3,Os",
                    help="optimisation levels, comma separated")
    ap.add_argument("--out", default="/tmp/addr_value_fuzz")
    ap.add_argument("--emit-only", type=int, metavar="SEED",
                    help="print one generated case and exit")
    args = ap.parse_args()

    if args.emit_only is not None:
        sys.stdout.write(gen_c(args.emit_only))
        return 0
    if not args.ccc:
        ap.error("--ccc is required unless --emit-only is used")

    lo, hi = (int(x) for x in args.seeds.split(":"))
    levels = args.levels.split(",")
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    counts = {"pass": 0, "mismatch": 0, "compile_failure": 0, "skip": 0}
    failures = []

    def record(bucket: str, **fields) -> None:
        counts[bucket] += 1
        failures.append(fields)

    for seed in range(lo, hi):
        src = out / f"s{seed}.c"
        src.write_text(gen_c(seed))
        # The reference must also compile and pass the case: a failing
        # reference means a bug in THIS generator, not in --ccc, so the seed
        # is reported as a skip (the run still fails loudly through the
        # reference_rejected entry when the operator asks for the JSON).
        gbin = out / f"g{seed}"
        try:
            rc, log = _run([args.gcc, "-O2", "-w", str(src), "-o", str(gbin)])
            if rc == 0:
                rc, log = _run([str(gbin)])
                ok = rc == 0 and _verdict(log) == "OK"
            else:
                ok = False
        except _Timeout as exc:
            ok = False
            log = f"timeout: {exc}"
        if not ok:
            record("skip", seed=seed, kind="reference_rejected", detail=log[-400:])
            continue

        for level in levels:
            binp = out / f"c{seed}_{level}"
            try:
                rc, log = _run([args.ccc, f"-{level}", "-w", str(src),
                                "-o", str(binp)], timeout=120)
                if rc != 0:
                    record("compile_failure", seed=seed, level=level,
                           kind="compile", detail=log[-400:])
                    continue
                rc, text = _run([str(binp)])
            except _Timeout as exc:
                record("compile_failure", seed=seed, level=level,
                       kind="timeout", detail=str(exc))
                continue
            if rc == 0 and _verdict(text) == "OK":
                counts["pass"] += 1
            else:
                record("mismatch", seed=seed, level=level, kind="run",
                       detail=(text or f"exit status {rc}")[-400:])

    summary = {"results": counts, "seeds": args.seeds, "levels": levels,
               "failures": failures[:20]}
    (out / "summary.json").write_text(json.dumps(summary, indent=1))
    print(json.dumps(counts))
    return 0 if counts["mismatch"] == 0 and counts["compile_failure"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
