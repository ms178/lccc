#!/usr/bin/env python3
"""Differential test: every expression FORM that can produce a by-value
vector / vector-wrapping-struct argument, for every x86-64 SysV carrier
(GP, one SSE eightbyte, one 16-byte XMM, MEMORY), passed directly, through
`...`, and to an unprototyped callee.

The callee is always compiled by the oracle (GCC); the caller is compiled by
the oracle and by lccc, and the two runs must print identical lines.  This
pins the *caller-side* operand representation (value vs. address) that a
shape fuzzer with only lvalue arguments cannot reach: call results, raw
builtins, ternaries, statement expressions, compound literals, casts.

Usage: check_vector_arg_forms.py --lccc PATH [--cc gcc-16] [--opt -O2 ...]
"""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile

# name, element C type, lane count, printf-able lane expression
TYPES = [
    ("v4c", "char", 4),        # 4 B integer lanes: GP
    ("v2i", "int", 2),         # 8 B: one SSE eightbyte
    ("v2f", "float", 2),       # 8 B float: one SSE eightbyte
    ("v4f", "float", 4),       # 16 B: one XMM
    ("v2d", "double", 2),      # 16 B: one XMM
    ("v4i", "int", 4),         # 16 B integer lanes: one XMM
    ("v1d", "double", 1),      # one-lane FP: MEMORY
]

FORMS = [
    # id, expression producing the argument (T = vector type, S = wrapper)
    ("lval_mem", "g_arr[1]"),
    ("lval_ptr", "p[1]"),
    ("local", "loc"),
    ("param", "prm"),
    ("call", "mk(3)"),
    ("call_ptr", "fp_mk(4)"),
    ("arith", "(g_arr[0] + g_arr[1])"),
    ("ternary", "(sel ? g_arr[0] : mk(5))"),
    ("comma", "(sel++, g_arr[1])"),
    ("stmt_expr", "({ T t_ = g_arr[1]; t_ + t_; })"),
    ("compound", "((T){LANES})"),
    ("member", "g_wrap.v"),
    ("call_member", "mkw(6).v"),
    ("assign", "(loc = g_arr[0])"),
]


def lanes_init(elem, n, base):
    return ", ".join(f"({elem})({base} + {i} * 3)" for i in range(n))


def gen(tname, elem, n):
    size = n * {"char": 1, "int": 4, "float": 4, "double": 8}[elem]
    hdr = f"""#include <stdarg.h>
#include <stdio.h>
typedef {elem} T __attribute__((vector_size({size})));
typedef struct {{ T v; }} S;
"""
    callee = hdr + f"""
static long long h(T x) {{ long long r = 0; for (int i = 0; i < {n}; i++) r = r * 131 + (long long)(x[i] * 4); return r; }}
long long take(int a, T x, int b) {{ return h(x) * 7 + a * 3 + b; }}
long long takes(int a, S x, int b) {{ return h(x.v) * 11 + a * 3 + b; }}
long long take_va(int n, ...) {{ va_list ap; va_start(ap, n); T x = va_arg(ap, T); int b = va_arg(ap, int); va_end(ap); return h(x) * 5 + n + b; }}
long long take_vas(int n, ...) {{ va_list ap; va_start(ap, n); S x = va_arg(ap, S); int b = va_arg(ap, int); va_end(ap); return h(x.v) * 13 + n + b; }}
long long take_kr(int a, T x, int b) {{ return h(x) * 17 + a * 3 + b; }}
T mk(int k) {{ T r = {{{lanes_init(elem, n, "k")}}}; return r; }}
S mkw(int k) {{ S r = {{{{{lanes_init(elem, n, "k * 2")}}}}}; return r; }}
"""
    lines = []
    for fid, expr in FORMS:
        e = expr.replace("LANES", lanes_init(elem, n, 9))
        # vector-typed argument
        lines.append(f'  sel = 1; loc = g_arr[1]; printf("{fid} direct %lld\\n", take(1, {e}, 2));')
        lines.append(f'  sel = 0; loc = g_arr[1]; printf("{fid} va %lld\\n", take_va(3, {e}, 4));')
        lines.append(f'  sel = 1; loc = g_arr[1]; printf("{fid} kr %lld\\n", ((long long (*)())take_kr)(5, {e}, 6));')
        # struct-wrapped argument built from the same form
        lines.append(f'  sel = 1; loc = g_arr[1]; {{ S s_ = {{ {e} }}; printf("{fid} wrap %lld\\n", takes(7, s_, 8)); }}')
        lines.append(f'  sel = 1; loc = g_arr[1]; {{ S s_ = {{ {e} }}; printf("{fid} wrapva %lld\\n", take_vas(9, s_, 10)); }}')
    caller = hdr + f"""
long long take(int a, T x, int b);
long long takes(int a, S x, int b);
long long take_va(int n, ...);
long long take_vas(int n, ...);
long long take_kr();
T mk(int k);
S mkw(int k);
T g_arr[2] = {{ {{{lanes_init(elem, n, 1)}}}, {{{lanes_init(elem, n, 2)}}} }};
S g_wrap = {{ {{{lanes_init(elem, n, 7)}}} }};
T (*volatile fp_mk)(int) = mk;
volatile int sel;
static void run(T *p, T prm) {{
  T loc;
{chr(10).join(lines)}
  printf("call_wrap %lld\\n", takes(1, mkw(2), 3));
  printf("call_wrapva %lld\\n", take_vas(1, mkw(2), 3));
}}
int main(void) {{ setvbuf(stdout, NULL, _IOLBF, 0); run(g_arr, g_arr[0]); return 0; }}
"""
    return callee, caller


def sh(cmd, cwd):
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=120)



def oracle_error(cc):
    """None when `cc` is a GCC 16 (the pinned oracle, as in
    tests/regression/lib/gcc_oracle.sh), otherwise why not: a missing or
    different compiler is a configuration error, never a silent substitute."""
    if shutil.which(cc) is None:
        return f"oracle {cc} not found"
    try:
        ver = subprocess.run([cc, "-dumpfullversion"], capture_output=True, text=True, timeout=30).stdout.strip()
    except (OSError, subprocess.SubprocessError) as e:
        return f"oracle {cc} unusable: {e}"
    if ver.split(".")[0] != "16":
        return f"oracle {cc} is GCC {ver or '?'}, GCC 16 required"
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc", required=True)
    ap.add_argument("--cc", default="gcc-16")
    ap.add_argument("--opt", action="append", default=None)
    ap.add_argument("--keep")
    a = ap.parse_args()
    err = oracle_error(a.cc)
    if err:
        print(f"check_vector_arg_forms: FAIL: {err}", file=sys.stderr)
        return 2
    lccc = os.path.abspath(a.lccc)
    opts = a.opt or ["-O0", "-O2"]
    fails = 0
    total = 0
    with tempfile.TemporaryDirectory() as td:
        wd = a.keep or td
        os.makedirs(wd, exist_ok=True)
        for tname, elem, n in TYPES:
            callee, caller = gen(tname, elem, n)
            d = os.path.join(wd, tname)
            os.makedirs(d, exist_ok=True)
            open(os.path.join(d, "callee.c"), "w").write(callee)
            open(os.path.join(d, "caller.c"), "w").write(caller)
            flags = ["-std=gnu17", "-w"]
            r = sh([a.cc, "-O2", *flags, "-c", "callee.c", "-o", "callee.o"], d)
            if r.returncode:
                print(f"ERROR oracle callee {tname}: {r.stderr[:400]}")
                return 2
            r = sh([a.cc, "-O2", *flags, "callee.o", "caller.c", "-o", "ref"], d)
            if r.returncode:
                print(f"ERROR oracle caller {tname}: {r.stderr[:400]}")
                return 2
            ref = sh(["./ref"], d).stdout.splitlines()
            for opt in opts:
                r = sh([lccc, opt, *flags, "-c", "caller.c", "-o", "caller.lccc.o"], d)
                if r.returncode:
                    print(f"FAIL {tname} {opt}: lccc compile error: {r.stderr.strip()[:300]}")
                    fails += len(ref)
                    total += len(ref)
                    continue
                r = sh([a.cc, "callee.o", "caller.lccc.o", "-o", "t"], d)
                if r.returncode:
                    print(f"ERROR link {tname} {opt}: {r.stderr[:300]}")
                    return 2
                run = sh(["./t"], d)
                got = run.stdout.splitlines()
                if run.returncode:
                    print(f"FAIL {tname} {opt}: exit {run.returncode} after {len(got)} lines")
                for i, want in enumerate(ref):
                    total += 1
                    have = got[i] if i < len(got) else "<missing>"
                    if have != want:
                        fails += 1
                        print(f"FAIL {tname} {opt}: want '{want}' got '{have}'")
    print(f"check_vector_arg_forms: {total - fails}/{total} pass")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
