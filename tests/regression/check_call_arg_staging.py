#!/usr/bin/env python3
"""Differential stress gate for call-argument staging versus register homes.

The x86-64 register allocator lets a value consumed as a call argument live
in an ABI argument register whenever no EARLIER argument of any call that
reads it stages into that register (`collect_call_arg_values` /
`arg_reg_hazard` in `src/backend/regalloc.rs`).  This gate generates callers
that forward permuted, duplicated and derived parameters of every SysV
argument class into callees that hash their arguments in an order-sensitive
way, then checks that an LCCC-compiled caller translation unit computes the
same hashes as a GCC-compiled one.  Callees and the driver are always
compiled by GCC, so a wrong home shows up as an ABI-level mismatch, not as a
self-consistent LCCC bug.

Shapes per caller (seeded, reproducible):
  * tail forwards in 1-3 conditional arms (sibling calls),
  * non-tail calls whose result is combined with a parameter afterwards,
  * two calls in sequence sharing arguments,
  * arguments drawn from parameters (any position, duplicates allowed),
    constants and `p + 1` style expressions,
  * argument classes: int, long, pointer, double, float, 2-GP struct,
    INTEGER+SSE mixed struct, __int128, and calls with more than six GP
    arguments (stack overflow).

Usage: check_call_arg_staging.py --lccc PATH [--gcc gcc-16] [--seeds N]
       [--funcs N] [--opts -O1,-O2] [--keep DIR] [--self-test]
Exit status 0 iff every (seed, opt) build matches GCC bit for bit.
"""

import argparse
import os
import random
import shutil
import subprocess
import sys
import tempfile

TYPES = ["int", "long", "ptr", "double", "float", "S2", "MX", "i128"]
CDECL = {
    "int": "int",
    "long": "long",
    "ptr": "const char *",
    "double": "double",
    "float": "float",
    "S2": "struct S2",
    "MX": "struct MX",
    "i128": "__int128",
}
PRELUDE = """\
struct S2 { long a, b; };
struct MX { long i; double d; };
typedef unsigned long u64;
"""


def fold(t, name):
    """C expression reducing an argument of type t to u64."""
    if t in ("int", "long"):
        return f"(u64){name}"
    if t == "ptr":
        return f"(u64)({name} - base)"
    if t == "double":
        return f"(u64)(long)({name} * 8.0)"
    if t == "float":
        return f"(u64)(long)({name} * 4.0f)"
    if t == "S2":
        return f"((u64){name}.a * 3u + (u64){name}.b)"
    if t == "MX":
        return f"((u64){name}.i * 5u + (u64)(long)({name}.d * 2.0))"
    if t == "i128":
        return f"((u64){name} ^ (u64)({name} >> 64))"
    raise ValueError(t)


def const_of(t, rng):
    k = rng.randint(-9, 99)
    if t in ("int", "long"):
        return str(k)
    if t == "ptr":
        return f"base + {abs(k) % 16}"
    if t == "double":
        return f"{k}.25"
    if t == "float":
        return f"{k}.5f"
    if t == "S2":
        return f"(struct S2){{{k}, {k * 7}}}"
    if t == "MX":
        return f"(struct MX){{{k}, {k}.75}}"
    if t == "i128":
        return f"((__int128){k} << 70 | {abs(k)})"
    raise ValueError(t)


def derived(t, name, rng):
    """A cheap expression of parameter `name` of type t (same type)."""
    if t in ("int", "long"):
        return rng.choice([f"{name} + 1", f"{name} ^ 3", f"{name} * 2"])
    if t == "ptr":
        return f"{name} + 1"
    if t in ("double", "float"):
        return f"{name} + 1"
    if t == "i128":
        return f"{name} + 5"
    return name


class Gen:
    def __init__(self, seed, nfuncs):
        self.rng = random.Random(seed)
        self.nfuncs = nfuncs
        self.callees = []  # (name, [types])

    def callee_sig(self):
        rng = self.rng
        # Bias towards scalar GP classes (the hazard model's main subject),
        # with enough SSE/aggregate/i128 mix to shift GP slots.
        n = rng.choice([1, 2, 2, 3, 3, 3, 4, 4, 5, 6, 7, 8])
        weights = [5, 5, 3, 2, 1, 2, 1, 1]
        return [rng.choices(TYPES, weights)[0] for _ in range(n)]

    def gen_callees(self, count):
        for i in range(count):
            self.callees.append((f"c{i}", self.callee_sig()))

    def callee_defs(self):
        out = [PRELUDE, "extern const char base[64];"]
        for name, sig in self.callees:
            params = ", ".join(f"{CDECL[t]} a{i}" for i, t in enumerate(sig))
            body = " ".join(
                f"h = h * 1000003u + {fold(t, f'a{i}')};" for i, t in enumerate(sig)
            )
            out.append(
                f"long {name}({params}) {{ u64 h = {len(sig)}u; {body} return (long)(h ^ (h >> 29)); }}"
            )
        return "\n".join(out) + "\n"

    def callee_protos(self):
        out = []
        for name, sig in self.callees:
            out.append(f"long {name}({', '.join(CDECL[t] for t in sig)});")
        return "\n".join(out) + "\n"

    def arg_for(self, t, params):
        rng = self.rng
        same = [p for p, pt in params if pt == t]
        r = rng.random()
        if same and r < 0.75:
            return rng.choice(same)
        if same and r < 0.88:
            return derived(t, rng.choice(same), rng)
        return const_of(t, rng)

    def call_expr(self, params):
        name, sig = self.rng.choice(self.callees)
        return f"{name}({', '.join(self.arg_for(t, params) for t in sig)})"

    def caller(self, idx):
        rng = self.rng
        n = rng.choice([1, 2, 3, 3, 4, 4, 5, 6, 7])
        ptypes = [rng.choices(TYPES, [6, 6, 3, 2, 1, 2, 1, 1])[0] for _ in range(n)]
        # The arm condition reads the first integral parameter, or a constant.
        params = [(f"p{i}", t) for i, t in enumerate(ptypes)]
        cond_src = next((p for p, t in params if t in ("int", "long")), None)
        cond = (lambda k: f"{cond_src} > {k}") if cond_src else (lambda k: f"sel > {k}")
        shape = rng.choice(["tail", "tail", "tail2", "tail3", "nontail", "seq"])
        decl = ", ".join(f"{CDECL[t]} {p}" for p, t in params)
        if not cond_src:
            decl = (decl + ", " if decl else "") + "int sel"
        live_after = next((p for p, t in params if t in ("int", "long")), None)
        if shape == "tail":
            body = f"return {self.call_expr(params)};"
        elif shape == "tail2":
            body = f"if ({cond(0)}) return {self.call_expr(params)}; return {self.call_expr(params)};"
        elif shape == "tail3":
            body = (
                f"if ({cond(3)}) return {self.call_expr(params)}; "
                f"if ({cond(-3)}) return {self.call_expr(params)}; return {self.call_expr(params)};"
            )
        elif shape == "nontail":
            tail = f" ^ (long){live_after}" if live_after else ""
            body = f"long r = {self.call_expr(params)}; return r * 3{tail};"
        else:
            body = f"long r = {self.call_expr(params)}; return r + {self.call_expr(params)};"
        attr = "__attribute__((noinline)) "
        return f"{attr}long f{idx}({decl}) {{ {body} }}", params, not cond_src

    def build(self):
        self.gen_callees(max(6, self.nfuncs // 4))
        callers, calls = [], []
        for i in range(self.nfuncs):
            text, params, has_sel = self.caller(i)
            callers.append(text)
            for trial in range(3):
                args = [const_of(t, self.rng) for _, t in params]
                if has_sel:
                    args.append(str(self.rng.choice([-5, 0, 2, 7])))
                calls.append(f"    acc(\"f{i}.{trial}\", f{i}({', '.join(args)}));")
        caller_tu = PRELUDE + "extern const char base[64];\n" + self.callee_protos() + "\n".join(callers) + "\n"
        protos = "\n".join(
            c.split(" {", 1)[0].replace("__attribute__((noinline)) ", "") + ";" for c in callers
        )
        main_tu = (
            self.callee_defs()
            + protos
            + "\n#include <stdio.h>\nconst char base[64];\n"
            + "static void acc(const char *k, long v) { printf(\"%s %ld\\n\", k, v); }\n"
            + "int main(void) {\n"
            + "\n".join(calls)
            + "\n    return 0;\n}\n"
        )
        return caller_tu, main_tu


# Pinned shapes, run before the seeds: `__int128` arguments that overflow to
# the stack AFTER the register arguments are staged (SysV: a 16-byte INTEGER
# argument that does not fit in the remaining GP registers goes to memory,
# and later arguments may still take registers). The stack store must not
# use a scratch register an earlier argument was staged in: a `pushq %r11`
# / `%r8` scratch clobbered a staged argument (seeds 11, 14, 16, 26 and 30
# of the generator, 5/40 on the pre-fix compiler). One function per
# emission path: a value (`cqto` + slot pushes), a 64-bit constant
# (`movabsq` into %rax, the one scratch left), small constants
# (`pushq $imm`), and the original seed-11 reduction.
PINNED_CALLER = r"""
typedef __int128 i128;
typedef unsigned __int128 u128;
extern long base_arr[];
long c13(int, int, const long *, float, int, int, int, i128);
long c7(long, long, long, long, long, long, i128, i128);
long c8(long, long, long, long, long, i128, long, u128);
__attribute__((noinline)) long k_seed11(int p0, i128 p1)
{ long r = c13(30, p0, base_arr + 0, 5.5f, 9, p0 ^ 3, 46, p1); return r * 3 ^ (long)p0; }
__attribute__((noinline)) long k_value(long a, long b)
{ return c7(b, a, b, a, b, a, (i128)a * b, (i128)b); }
__attribute__((noinline)) long k_wide_const(long a)
{ return c7(a, a + 1, a + 2, a + 3, a + 4, a + 5, ((i128)0x123456789abcdefLL << 64) | 7, -5); }
__attribute__((noinline)) long k_small_const(long a, long b)
{ return c7(b, a, b, a, b, a, -1, -5); }
__attribute__((noinline)) long k_mixed(long a, long b, u128 u)
{ return c8(a, b, a ^ b, b - a, a + 7, (i128)a << 70, b, u + 1); }
"""

PINNED_MAIN = r"""
#include <stdio.h>
typedef __int128 i128;
typedef unsigned __int128 u128;
typedef unsigned long long u64;
long base_arr[4] = { 11, 22, 33, 44 };
static u64 f128(i128 v) { return (u64)v * 31u + (u64)(v >> 64); }
long c13(int a, int b, const long *p, float f, int c, int d, int e, i128 g)
{ u64 h = 7u; h = h * 1000003u + (u64)a; h = h * 1000003u + (u64)b; h = h * 1000003u + (u64)*p;
  h = h * 1000003u + (u64)(long)(f * 8); h = h * 1000003u + (u64)c; h = h * 1000003u + (u64)d;
  h = h * 1000003u + (u64)e; h = h * 1000003u + f128(g); return (long)(h ^ (h >> 29)); }
long c7(long a, long b, long c, long d, long e, long f, i128 g, i128 k)
{ u64 h = 5u; h = h * 1000003u + (u64)a; h = h * 1000003u + (u64)b; h = h * 1000003u + (u64)c;
  h = h * 1000003u + (u64)d; h = h * 1000003u + (u64)e; h = h * 1000003u + (u64)f;
  h = h * 1000003u + f128(g); h = h * 1000003u + f128(k); return (long)(h ^ (h >> 29)); }
long c8(long a, long b, long c, long d, long e, i128 g, long f, u128 u)
{ u64 h = 3u; h = h * 1000003u + (u64)a; h = h * 1000003u + (u64)b; h = h * 1000003u + (u64)c;
  h = h * 1000003u + (u64)d; h = h * 1000003u + (u64)e; h = h * 1000003u + f128(g);
  h = h * 1000003u + (u64)f; h = h * 1000003u + f128((i128)u); return (long)(h ^ (h >> 29)); }
long k_seed11(int, i128); long k_value(long, long); long k_wide_const(long);
long k_small_const(long, long); long k_mixed(long, long, u128);
int main(void) {
    printf("k_seed11 %ld\n", k_seed11(13, ((i128)-3 << 64) | 11));
    printf("k_value %ld\n", k_value(-9, 1234567));
    printf("k_wide_const %ld\n", k_wide_const(17));
    printf("k_small_const %ld\n", k_small_const(4, -6));
    printf("k_mixed %ld\n", k_mixed(-2, 99, ((u128)5 << 64) | 3));
    return 0;
}
"""


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def build_and_run(cc, opt, caller, main_obj, work, tag):
    obj = os.path.join(work, f"caller.{tag}.o")
    exe = os.path.join(work, f"t.{tag}")
    r = run([cc, opt, "-c", caller, "-o", obj])
    if r.returncode:
        return None, f"compile failed ({cc} {opt}): {r.stderr[-2000:]}"
    r = run(["gcc-16" if shutil.which("gcc-16") else "gcc", obj, main_obj, "-o", exe])
    if r.returncode:
        return None, f"link failed: {r.stderr[-2000:]}"
    r = run([exe], timeout=60)
    if r.returncode:
        return None, f"run failed rc={r.returncode}"
    return r.stdout, None


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--lccc", required=False)
    ap.add_argument("--gcc", default="gcc-16" if shutil.which("gcc-16") else "gcc")
    ap.add_argument("--seeds", type=int, default=6, help="run seeds 1..N")
    ap.add_argument("--seed-list", help="comma-separated seeds (overrides --seeds), for triage")
    ap.add_argument("--funcs", type=int, default=120)
    ap.add_argument("--opts", default="-O1,-O2")
    ap.add_argument("--keep")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()

    if a.self_test:
        # The generator must be deterministic and emit every argument class.
        c1, m1 = Gen(7, 40).build()
        c2, m2 = Gen(7, 40).build()
        assert (c1, m1) == (c2, m2), "generator is not deterministic"
        for t in ("struct S2", "struct MX", "__int128", "double", "float", "const char *"):
            assert t in c1 + m1, f"generator never emits {t}"
        print("PASS: self-test (deterministic, all argument classes emitted)")
        return 0
    if not a.lccc:
        ap.error("--lccc is required")

    work = a.keep or tempfile.mkdtemp(prefix="cas.")
    os.makedirs(work, exist_ok=True)
    failures, cases = 0, 0
    seeds = (
        [int(x) for x in a.seed_list.split(",")] if a.seed_list else range(1, a.seeds + 1)
    )
    for seed in ["pinned", *seeds]:
        if seed == "pinned":
            caller_tu, main_tu = PINNED_CALLER, PINNED_MAIN
        else:
            caller_tu, main_tu = Gen(seed, a.funcs).build()
        caller = os.path.join(work, f"caller{seed}.c")
        mainc = os.path.join(work, f"main{seed}.c")
        with open(caller, "w") as f:
            f.write(caller_tu)
        with open(mainc, "w") as f:
            f.write(main_tu)
        main_obj = os.path.join(work, f"main{seed}.o")
        r = run([a.gcc, "-O1", "-c", mainc, "-o", main_obj])
        if r.returncode:
            print(f"FAIL: seed {seed}: driver compile: {r.stderr[-1500:]}")
            failures += 1
            continue
        ref, err = build_and_run(a.gcc, "-O0", caller, main_obj, work, f"ref{seed}")
        if err:
            print(f"FAIL: seed {seed}: reference: {err}")
            failures += 1
            continue
        for opt in a.opts.split(","):
            cases += 1
            got, err = build_and_run(a.lccc, opt, caller, main_obj, work, f"l{seed}{opt}")
            if err:
                print(f"FAIL: seed {seed} {opt}: {err}")
                failures += 1
                continue
            if got != ref:
                bad = [
                    (x, y)
                    for x, y in zip(ref.splitlines(), got.splitlines())
                    if x != y
                ]
                print(f"FAIL: seed {seed} {opt}: {len(bad)} mismatching calls, first {bad[:3]}")
                failures += 1
            else:
                print(f"PASS: seed {seed} {opt}: {len(ref.splitlines())} calls match gcc")
    if not a.keep:
        shutil.rmtree(work, ignore_errors=True)
    print(f"{'PASS' if failures == 0 else 'FAIL'}: call-arg staging {cases - failures}/{cases}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
