#!/usr/bin/env python3
"""SysV AMD64 by-value aggregate ABI interop gate (structs, unions, vectors).

Generates a deterministic corpus of aggregate shapes -- curated psABI corner
cases plus seeded random compositions -- and checks every shape across the
call boundary in all three directions:

    oracle callee  <- lccc caller
    lccc callee    <- oracle caller
    lccc callee    <- lccc caller      (self-consistency control)

Per shape the provider TU exports five functions that together exercise
argument classification, register exhaustion/rollback, the return
convention, and va_arg:

    S    get_K(int a)                                    -> make(a)
    S    ret_K(int a, S x, int b)                        -> transform(x, a+b)
    long sum_K(int a, S x, double d, S y, long long z)   -> weighted hash
    long many_K(S, S, S, S, S, S, S, S, int tail)        -> weighted hash
    long va_K(int n, ...)   (S, double, S, int)          -> weighted hash

Expected values are computed by the caller from `static inline` helpers in a
shared header, so both sides agree on the arithmetic and only the calling
convention is under test. Each scalar leaf receives a distinct small integer
value (exact in every floating format), so an eightbyte delivered in the
wrong register, swapped, or truncated changes the result.

Each case runs in its own process: a crash in one shape cannot mask the rest.
Exit status is 0 only if every case passes on every leg.
"""

import argparse
import os
import random
import shutil
import subprocess
import sys
import tempfile

# (C spelling, kind, size) -- kind drives value generation and hashing.
LEAVES = [
    ("char", "int", 1),
    ("unsigned char", "int", 1),
    ("short", "int", 2),
    ("int", "int", 4),
    ("long long", "int", 8),
    ("_Bool", "bool", 1),
    ("void *", "ptr", 8),
    ("float", "fp", 4),
    ("double", "fp", 8),
    ("long double", "fp", 16),
    ("_Complex float", "cplx", 8),
    ("_Complex double", "cplx", 16),
    ("__int128", "int", 16),
    ("_Float128", "fp", 16),
    ("v4c", "vec", 4),
    ("v8c", "vec", 8),
    ("v2s", "vec", 4),
    ("v4s", "vec", 8),
    ("v2i", "vec", 8),
    ("v1ll", "vec", 8),
    ("v2f", "vec", 8),
    ("v1f", "vec", 4),
    ("v1d", "vec", 8),
    ("v4i", "vec", 16),
    ("v4f", "vec", 16),
    ("v2d", "vec", 16),
]
VEC_TYPEDEFS = """\
typedef char v4c __attribute__((vector_size(4)));
typedef char v8c __attribute__((vector_size(8)));
typedef short v2s __attribute__((vector_size(4)));
typedef short v4s __attribute__((vector_size(8)));
typedef int v2i __attribute__((vector_size(8)));
typedef long long v1ll __attribute__((vector_size(8)));
typedef float v2f __attribute__((vector_size(8)));
typedef float v1f __attribute__((vector_size(4)));
typedef double v1d __attribute__((vector_size(8)));
typedef int v4i __attribute__((vector_size(16)));
typedef float v4f __attribute__((vector_size(16)));
typedef double v2d __attribute__((vector_size(16)));
"""
VEC_LANES = {"v4c": 4, "v8c": 8, "v2s": 2, "v4s": 4, "v2i": 2, "v1ll": 1, "v2f": 2,
             "v1f": 1, "v1d": 1, "v4i": 4, "v4f": 4, "v2d": 2}
LEAF_BY_NAME = {name: (name, kind, size) for name, kind, size in LEAVES}


class Shape:
    """A struct/union type: list of members, each (decl_text, leaf_paths)."""

    def __init__(self, tag, is_union, packed, members):
        self.tag = tag
        self.is_union = is_union
        self.packed = packed
        self.members = members  # list of (decl, [(path, kind, ctype)])


def leaf_paths(prefix, ctype, kind):
    """Scalar access paths for one leaf: (lvalue template, kind, C type).

    Templates use `@` for the base object (`@.m0[1]`, `__real__ @.m2`), so a
    nested member's paths compose by substituting `@` -> `@.name`.
    """
    prefix = "@" + prefix
    if kind == "vec":
        return [(f"{prefix}[{i}]", "veclane", ctype) for i in range(VEC_LANES[ctype])]
    if kind == "cplx":
        return [(f"__real__ {prefix}", "fp", ctype), (f"__imag__ {prefix}", "fp", ctype)]
    return [(prefix, kind, ctype)]


class Gen:
    def __init__(self, rng):
        self.rng = rng
        self.defs = []  # nested struct definitions, emitted before users
        self.counter = 0

    def fresh(self, stem):
        self.counter += 1
        return f"{stem}{self.counter}"

    def leaf(self):
        return self.rng.choice(LEAVES)

    def shape(self, depth=0, force=None):
        rng = self.rng
        is_union = rng.random() < 0.12
        packed = rng.random() < 0.22
        tag = self.fresh("U" if is_union else "S")
        members = []
        for i in range(rng.randint(1, 4)):
            name = f"m{i}"
            r = rng.random()
            if r < 0.66:
                ct, kind, _ = self.leaf()
                members.append((f"{ct} {name};", leaf_paths(f".{name}", ct, kind)))
            elif r < 0.78:
                ct, kind, _ = self.leaf()
                n = rng.randint(1, 3)
                paths = []
                for j in range(n):
                    paths += leaf_paths(f".{name}[{j}]", ct, kind)
                members.append((f"{ct} {name}[{n}];", paths))
            elif r < 0.88 and depth < 2:
                inner = self.shape(depth + 1)
                self.defs.append(inner)
                paths = []
                for _, ps in inner.members:
                    paths += [(p.replace("@", f"@.{name}"), k, c) for p, k, c in ps]
                if inner.is_union:
                    paths = paths[:1]
                members.append((f"{'union' if inner.is_union else 'struct'} {inner.tag} {name};", paths))
            else:
                bt = rng.choice(["unsigned int", "unsigned char", "unsigned long long", "unsigned short"])
                maxw = {"unsigned int": 32, "unsigned char": 8, "unsigned long long": 64, "unsigned short": 16}[bt]
                w = rng.randint(1, maxw)
                members.append((f"{bt} {name} : {w};", [(f"@.{name}", f"bf{w}", bt)]))
        if is_union:
            # Only the first member's leaves are read back (one active member).
            members = [(d, ps if i == 0 else []) for i, (d, ps) in enumerate(members)]
        return Shape(tag, is_union, packed, members)


def curated():
    """Shapes the psABI classifies non-trivially; each names its rule."""
    out = []

    def mk(tag, decls, packed=False, is_union=False):
        members = []
        for d in decls:
            ct, name = d.rsplit(" ", 1)
            name = name.rstrip(";")
            n = None
            if "[" in name:
                name, n = name[:-1].split("[")
                n = int(n)
            kind = LEAF_BY_NAME[ct][1]
            if n is None:
                paths = leaf_paths(f".{name}", ct, kind)
            else:
                paths = []
                for j in range(n):
                    paths += leaf_paths(f".{name}[{j}]", ct, kind)
            members.append((d, paths))
        if is_union:
            members = [(d, ps if i == 0 else []) for i, (d, ps) in enumerate(members)]
        out.append(Shape(tag, is_union, packed, members))

    mk("C_p5", ["char c;", "int i;"], packed=True)                 # misaligned int -> MEMORY
    mk("C_p3", ["char c;", "short s;"], packed=True)               # misaligned short -> MEMORY
    mk("C_pal", ["int a;", "int b;"], packed=True)                 # packed but aligned -> INTEGER
    mk("C_p12", ["char c;", "long long l;", "int k;"], packed=True)
    mk("C_ld", ["long double x;"])                                 # X87,X87UP: mem arg, st0 ret
    mk("C_ldu", ["long double x;", "char c;"], is_union=True)      # INTEGER + X87UP -> MEMORY
    mk("C_cf", ["_Complex float z;"])                              # SSE
    mk("C_cd", ["_Complex double z;"])                             # SSE, SSE (two xmm)
    mk("C_f128", ["_Float128 q;"])                                 # SSE, SSEUP (one xmm)
    mk("C_v4i", ["v4i v;"])                                        # SSE, SSEUP
    mk("C_v2d", ["v2d v;"])
    mk("C_v8c", ["v8c v;"])                                        # SSE
    mk("C_v4c", ["v4c v;"])                                        # INTEGER
    mk("C_v1d", ["v1d v;"])                                        # BLKmode leaf -> MEMORY
    mk("C_v1f", ["v1f v;"])
    mk("C_v1fi", ["v1f v;", "int i;"])
    mk("C_fi", ["float f;", "int i;"])                             # SSE+INTEGER merge -> INTEGER
    mk("C_ff", ["float a;", "float b;"])                           # SSE
    mk("C_dl", ["double d;", "long long l;"])                      # SSE, INTEGER
    mk("C_ld2", ["long long l;", "double d;"])                     # INTEGER, SSE
    mk("C_i128", ["__int128 x;"])
    mk("C_f3", ["float f[3];"])                                    # SSE, SSE
    mk("C_c3", ["char c[3];"])
    mk("C_cfi", ["_Complex float z;", "int i;"])
    mk("C_uv", ["v4i v;", "double d;"], is_union=True)             # SSE+SSE, SSEUP -> one xmm
    mk("C_df128", ["double d;", "_Float128 q;"])                   # > 16 -> MEMORY
    return out


def emit_shape_def(sh):
    kw = "union" if sh.is_union else "struct"
    attr = " __attribute__((packed))" if sh.packed else ""
    body = " ".join(d for d, _ in sh.members)
    return f"{kw} {sh.tag} {{ {body} }}{attr};"


def all_paths(sh):
    out = []
    for _, ps in sh.members:
        out += ps
    return out


def value_expr(kind, ctype, v):
    if kind == "bool":
        return "1"
    if kind == "ptr":
        return f"(void *)(unsigned long)({v} * 16)"
    if kind.startswith("bf"):
        w = int(kind[2:])
        return str(v % (1 << w))
    return str(v)


def lv(template):
    return template.replace("@", "x")


def helpers(sh, k):
    """static inline set/transform/hash for shape k."""
    T = f"{'union' if sh.is_union else 'struct'} {sh.tag}"
    paths = all_paths(sh)
    s = [f"static inline {T} mk_{k}(int salt) {{ {T} x; __builtin_memset(&x, 0, sizeof x);"]
    for i, (p, kind, ct) in enumerate(paths):
        base = (k * 7 + i * 3 + 1) % 50 + 1
        if kind.startswith("bf") or kind in ("bool", "ptr"):
            s.append(f"  {lv(p)} = {value_expr(kind, ct, base)};")
        else:
            s.append(f"  {lv(p)} = {base} + salt;")
    s.append("  return x; }")
    s.append(f"static inline {T} xf_{k}({T} x, int d) {{")
    for p, kind, ct in paths:
        if kind == "bool":
            s.append(f"  {lv(p)} = !{lv(p)};")
        elif kind == "ptr":
            s.append(f"  {lv(p)} = (char *){lv(p)} + d;")
        else:
            s.append(f"  {lv(p)} = {lv(p)} + d;")
    s.append("  return x; }")
    s.append(f"static inline long long h_{k}({T} x) {{ long long h = 0;")
    for i, (p, kind, ct) in enumerate(paths):
        if kind == "ptr":
            s.append(f"  h += (long long)(unsigned long){lv(p)} * {i + 3};")
        else:
            s.append(f"  h += (long long)({lv(p)}) * {i + 3};")
    s.append("  return h; }")
    return "\n".join(s)


def build_sources(shapes):
    hdr = ["#include <stdarg.h>", VEC_TYPEDEFS]
    for sh in shapes:
        hdr.append(emit_shape_def(sh))
    prov = ['#include "abi.h"']
    call = ['#include <stdio.h>', '#include <stdlib.h>', '#include "abi.h"']
    for k, sh in enumerate(shapes):
        T = f"{'union' if sh.is_union else 'struct'} {sh.tag}"
        hdr.append(helpers(sh, k))
        sig = (f"{T} get_{k}(int a);\n"
               f"{T} ret_{k}(int a, {T} x, int b);\n"
               f"long long sum_{k}(int a, {T} x, double d, {T} y, long long z);\n"
               f"long long many_{k}({T} x0, {T} x1, {T} x2, {T} x3, {T} x4, {T} x5, {T} x6, {T} x7, int tail);\n"
               f"long long va_{k}(int n, ...);")
        hdr.append(sig)
        # Scalar-only signature: no aggregate argument, so both sides take
        # their plain-call fast paths and only the return convention decides.
        prov.append(f"{T} get_{k}(int a) {{ return mk_{k}(a); }}")
        prov.append(f"{T} ret_{k}(int a, {T} x, int b) {{ return xf_{k}(x, a + b); }}")
        prov.append(f"long long sum_{k}(int a, {T} x, double d, {T} y, long long z) "
                    f"{{ return h_{k}(x) * 3 + h_{k}(y) * 5 + a * 7 + (long long)d * 11 + z * 13; }}")
        prov.append(f"long long many_{k}({T} x0, {T} x1, {T} x2, {T} x3, {T} x4, {T} x5, {T} x6, {T} x7, int tail) "
                    f"{{ return h_{k}(x0) + h_{k}(x1) * 2 + h_{k}(x2) * 3 + h_{k}(x3) * 4 + h_{k}(x4) * 5"
                    f" + h_{k}(x5) * 6 + h_{k}(x6) * 7 + h_{k}(x7) * 8 + tail * 9; }}")
        prov.append(f"long long va_{k}(int n, ...) {{ va_list ap; va_start(ap, n); {T} x = va_arg(ap, {T});"
                    f" double d = va_arg(ap, double); {T} y = va_arg(ap, {T}); int t = va_arg(ap, int); va_end(ap);"
                    f" return h_{k}(x) * 3 + h_{k}(y) * 5 + (long long)d * 7 + t * 11 + n; }}")
    call.append("static int fails;")
    call.append("static volatile int sel_;")
    call.append("#define CHECK(name, got, want) do { long long g_ = (got), w_ = (want); "
                "if (g_ != w_) { fails++; printf(\"FAIL %s got=%lld want=%lld\\n\", name, g_, w_); } } while (0)")
    for k, sh in enumerate(shapes):
        T = f"{'union' if sh.is_union else 'struct'} {sh.tag}"
        call.append(f"static void case_{k}(void) {{")
        call.append(f"  {T} x = mk_{k}(0), y = mk_{k}(5), z[8];")
        call.append(f"  for (int i = 0; i < 8; i++) z[i] = mk_{k}(i * 2);")
        # The result of a scalar-only call consumed as an argument, through
        # an initializer and through an assignment.
        call.append(f"  CHECK(\"get_{k}\", h_{k}(get_{k}(3)), h_{k}(mk_{k}(3)));")
        call.append(f"  {{ {T} v = get_{k}(4); CHECK(\"getinit_{k}\", h_{k}(v), h_{k}(mk_{k}(4))); }}")
        call.append(f"  {{ {T} v; v = get_{k}(6); CHECK(\"getasg_{k}\", h_{k}(v), h_{k}(mk_{k}(6))); }}")
        # A conditional mixing a call result with an lvalue, both selections.
        call.append(f"  sel_ = 1; CHECK(\"tern_{k}\", h_{k}(sel_ ? x : get_{k}(3)), h_{k}(x));")
        call.append(f"  sel_ = 0; CHECK(\"tern_{k}\", h_{k}(sel_ ? x : get_{k}(3)), h_{k}(mk_{k}(3)));")
        call.append(f"  sel_ = 0; CHECK(\"tern_{k}\", h_{k}(sel_ ? get_{k}(5) : y), h_{k}(y));")
        call.append(f"  CHECK(\"ret_{k}\", h_{k}(ret_{k}(3, x, 4)), h_{k}(xf_{k}(x, 7)));")
        call.append(f"  CHECK(\"sum_{k}\", sum_{k}(2, x, 9.0, y, 100), h_{k}(x) * 3 + h_{k}(y) * 5 + 14 + 99 + 1300);")
        call.append(f"  CHECK(\"many_{k}\", many_{k}(z[0], z[1], z[2], z[3], z[4], z[5], z[6], z[7], 17), "
                    f"h_{k}(z[0]) + h_{k}(z[1]) * 2 + h_{k}(z[2]) * 3 + h_{k}(z[3]) * 4 + h_{k}(z[4]) * 5"
                    f" + h_{k}(z[5]) * 6 + h_{k}(z[6]) * 7 + h_{k}(z[7]) * 8 + 153);")
        call.append(f"  CHECK(\"va_{k}\", va_{k}(1, x, 4.0, y, 6), h_{k}(x) * 3 + h_{k}(y) * 5 + 28 + 66 + 1);")
        call.append("}")
    call.append("int main(int argc, char **argv) { int k = atoi(argv[1]); switch (k) {")
    for k in range(len(shapes)):
        call.append(f"  case {k}: case_{k}(); break;")
    call.append("  default: return 2; } return fails ? 1 : 0; }")
    return "\n".join(hdr) + "\n", "\n".join(prov) + "\n", "\n".join(call) + "\n"


def run(cmd, cwd):
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)



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
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lccc", help="LCCC executable (required unless --emit-only)")
    ap.add_argument("--cc", default="gcc-16", help="oracle compiler (GCC is the authority for vector_size)")
    ap.add_argument("--seed", type=int, default=20261017)
    ap.add_argument("--count", type=int, default=160, help="random shapes in addition to the curated set")
    ap.add_argument("--opt", default="-O2")
    ap.add_argument("--keep", help="write the generated sources to this directory")
    ap.add_argument("--emit-only", action="store_true",
                    help="generate sources under --keep without requiring compilers")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    if args.emit_only and not args.keep:
        ap.error("--emit-only requires --keep")
    if not args.emit_only and not args.lccc:
        ap.error("--lccc is required unless --emit-only is used")
    if not args.emit_only:
        err = oracle_error(args.cc)
        if err:
            print(f"check_sysv_aggregate_abi: FAIL: {err}", file=sys.stderr)
            return 2
    rng = random.Random(args.seed)
    gen = Gen(rng)
    tops = curated() + [gen.shape() for _ in range(args.count)]
    shapes = gen.defs + tops  # nested definitions precede their users
    first_top = len(gen.defs)
    hdr, prov, call = build_sources(shapes)

    work = args.keep or tempfile.mkdtemp(prefix="sysvabi-")
    os.makedirs(work, exist_ok=True)
    for name, text in (("abi.h", hdr), ("prov.c", prov), ("call.c", call)):
        with open(os.path.join(work, name), "w") as f:
            f.write(text)
    if args.emit_only:
        print(f"check_sysv_aggregate_abi: emitted {len(tops)} shapes to {work}")
        return 0

    # Compilation runs inside the work directory: resolve path-like compiler
    # arguments now so a relative `--lccc target/.../lccc` keeps working.
    lccc = os.path.abspath(args.lccc) if os.sep in args.lccc else args.lccc
    compilers = {"oracle": args.cc, "lccc": lccc}
    objs = {}
    for role, cc in compilers.items():
        for tu in ("prov", "call"):
            out = f"{tu}.{role}.o"
            r = run([cc, args.opt, "-w", "-c", f"{tu}.c", "-o", out], work)
            if r.returncode != 0:
                print(f"check_sysv_aggregate_abi: {role} failed to compile {tu}.c:\n{r.stderr[-2000:]}")
                return 1
            objs[(role, tu)] = out

    legs = [("lccc", "oracle"), ("oracle", "lccc"), ("lccc", "lccc")]  # (caller, callee)
    total_fail = 0
    for caller, callee in legs:
        exe = f"t.{caller}-calls-{callee}"
        r = run([args.cc, "-o", exe, objs[(callee, "prov")], objs[(caller, "call")]], work)
        if r.returncode != 0:
            print(f"link failed for {exe}:\n{r.stderr[-2000:]}")
            return 1
        failed = []
        for k in range(first_top, len(shapes)):
            p = subprocess.run([os.path.join(work, exe), str(k)], capture_output=True, text=True, timeout=20)
            if p.returncode != 0:
                why = "crash" if p.returncode < 0 else ",".join(
                    line.split()[1].rsplit("_", 1)[0] for line in p.stdout.splitlines() if line.startswith("FAIL"))
                failed.append((k, why))
        n = len(shapes) - first_top
        print(f"{caller:>6} calls {callee:<6}: {n - len(failed)}/{n} shapes pass")
        for k, why in failed:
            print(f"    FAIL {shapes[k].tag:<8} [{why}]  {emit_shape_def(shapes[k])}" if args.verbose
                  else f"    FAIL {shapes[k].tag} [{why}]")
        total_fail += len(failed)
    if not args.keep:
        shutil.rmtree(work, ignore_errors=True)
    print("check_sysv_aggregate_abi: " + ("PASS" if total_fail == 0 else f"FAIL ({total_fail} shape-legs)"))
    return 0 if total_fail == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
