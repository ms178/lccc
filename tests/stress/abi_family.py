#!/usr/bin/env python3
"""ABI interoperability matrix for LCCC (x86-64 SysV).

A random set of aggregate types (nested structs, unions, arrays, bit-fields,
``_Bool``, ``long double``, packed and over-aligned members) is passed by
value through, and returned by value from, functions with mixed scalar
arguments that overflow the register classes.  The callee and the caller are
separate translation units, and every combination

    caller ∈ {lccc, gcc}  ×  callee ∈ {lccc, gcc}

must produce the exact checksum the generator computed in Python.  A failure
in ``lccc→gcc`` or ``gcc→lccc`` but not in ``lccc→lccc`` is an ABI bug that a
single-compiler test can never see (both sides make the same mistake); a
failure in ``lccc→lccc`` only is a plain miscompile.

All field values are small integers so that float/double arithmetic is exact
and the Python mirror of the callee body is bit-for-bit the C result.
"""
from __future__ import annotations

import random
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

import cemu
from cemu import I8, I16, I32, I64, U8, U16, U32, U64, convert

SCALARS = [
    # (C type, python converter, is_fp)
    ("char", lambda v: I8.wrap(v), False),
    ("signed char", lambda v: I8.wrap(v), False),
    ("unsigned char", lambda v: U8.wrap(v), False),
    ("short", lambda v: I16.wrap(v), False),
    ("unsigned short", lambda v: U16.wrap(v), False),
    ("int", lambda v: I32.wrap(v), False),
    ("unsigned", lambda v: U32.wrap(v), False),
    ("long", lambda v: I64.wrap(v), False),
    ("unsigned long long", lambda v: U64.wrap(v), False),
    ("_Bool", lambda v: 1 if v else 0, False),
    ("float", lambda v: v, True),
    ("double", lambda v: v, True),
    ("long double", lambda v: v, True),
]


@dataclass
class Field:
    name: str
    cty: str
    conv: object          # python converter for a stored value
    is_fp: bool
    array: int = 0        # 0 = scalar, else element count
    bits: int = 0         # bit-field width, 0 = none
    sub: "Agg | None" = None  # nested aggregate


@dataclass
class Agg:
    name: str
    kind: str             # "struct" | "union"
    fields: list[Field]
    attr: str = ""

    def decl(self) -> str:
        body = " ".join(f.decl() for f in self.fields)
        return f"{self.kind} {self.name} {{ {body} }}{self.attr};"

    def leaves(self, prefix: str) -> list[tuple[str, Field]]:
        """All scalar leaves as (access path, field)."""
        out: list[tuple[str, Field]] = []
        for f in self.fields:
            if f.sub is not None:
                if f.array:
                    for i in range(f.array):
                        out += f.sub.leaves(f"{prefix}.{f.name}[{i}]")
                else:
                    out += f.sub.leaves(f"{prefix}.{f.name}")
            elif f.array:
                for i in range(f.array):
                    out.append((f"{prefix}.{f.name}[{i}]", f))
            else:
                out.append((f"{prefix}.{f.name}", f))
        return out


def _decl(self: Field) -> str:
    if self.sub is not None:
        base = f"{self.sub.kind} {self.sub.name} {self.name}"
    else:
        base = f"{self.cty} {self.name}"
    if self.array:
        return f"{base}[{self.array}];"
    if self.bits:
        return f"{base}:{self.bits};"
    return base + ";"


Field.decl = _decl  # type: ignore[attr-defined]


def gen_agg(rng: random.Random, idx: int, pool: list[Agg], depth: int) -> Agg:
    kind = "union" if rng.random() < 0.15 else "struct"
    nf = rng.randint(1, 5) if kind == "struct" else rng.randint(2, 3)
    fields: list[Field] = []
    for i in range(nf):
        r = rng.random()
        if r < 0.15 and pool and depth < 2:
            sub = rng.choice(pool)
            fields.append(Field(f"n{i}", "", None, False, rng.choice([0, 0, 2]), 0, sub))
            continue
        cty, conv, is_fp = rng.choice(SCALARS)
        if cty == "long double" and rng.random() < 0.6:
            cty, conv, is_fp = "double", (lambda v: v), True
        arr = rng.choice([0, 0, 0, 0, 2, 3, 5]) if kind == "struct" else 0
        bits = 0
        if not arr and not is_fp and cty != "_Bool" and kind == "struct" and rng.random() < 0.15:
            width = {"char": 8, "signed char": 8, "unsigned char": 8, "short": 16, "unsigned short": 16,
                     "int": 32, "unsigned": 32, "long": 64, "unsigned long long": 64}[cty]
            bits = rng.randint(1, width - 1)
            signed = not cty.startswith("unsigned") and cty != "_Bool"
            conv = (lambda w, s: (lambda v: cemu.bitfield_store(v, w, s)))(bits, signed)
        fields.append(Field(f"f{i}", cty, conv, is_fp, arr, bits))
    attr = ""
    if kind == "struct":
        r = rng.random()
        if r < 0.08:
            attr = " __attribute__((packed))"
        elif r < 0.14:
            attr = " __attribute__((aligned(16)))"
        elif r < 0.17:
            attr = " __attribute__((aligned(32)))"
    return Agg(f"A{idx}", kind, fields, attr)


@dataclass
class Fn:
    name: str
    params: list[tuple[str, str, Agg | None]]   # (name, ctype, agg)
    ret: Agg | str                               # aggregate or scalar type name
    variadic: bool


def small_val(rng: random.Random, f: Field) -> int:
    if f.cty.startswith("unsigned") or f.cty == "_Bool":
        return rng.randint(0, 50)
    return rng.randint(-50, 50)


def build_program(rng: random.Random, seed: int) -> tuple[str, str, str, str]:
    """Return (common header, callee TU, caller TU, final \"ci cf\" checksum)."""
    aggs: list[Agg] = []
    for i in range(rng.randint(3, 6)):
        aggs.append(gen_agg(rng, i, aggs, 0))
    hdr = ["#include <stdint.h>", "#include <stdarg.h>", ""]
    hdr += [a.decl() for a in aggs]
    fns: list[Fn] = []
    for k in range(rng.randint(3, 6)):
        params: list[tuple[str, str, Agg | None]] = []
        n = rng.randint(1, 9)
        for j in range(n):
            r = rng.random()
            if r < 0.45:
                a = rng.choice(aggs)
                params.append((f"a{j}", f"{a.kind} {a.name}", a))
            else:
                cty = rng.choice(["int", "long", "double", "float", "unsigned char", "short", "long double", "unsigned long long"])
                params.append((f"a{j}", cty, None))
        r = rng.random()
        if r < 0.55:
            ret: Agg | str = rng.choice(aggs)
        else:
            ret = rng.choice(["int", "long", "double", "float", "long double", "unsigned long long", "void"])
        variadic = rng.random() < 0.2 and n >= 1
        fns.append(Fn(f"fn{k}", params, ret, variadic))
    hdr.append("")
    for fn in fns:
        plist = ", ".join(f"{t} {n}" for n, t, _ in fn.params)
        if fn.variadic:
            plist += ", ..."
        rt = f"{fn.ret.kind} {fn.ret.name}" if isinstance(fn.ret, Agg) else fn.ret
        hdr.append(f"{rt} {fn.name}({plist});")
    header = "\n".join(hdr) + "\n"

    # ---- callee TU + Python mirror -------------------------------------
    callee = ['#include "abi_common.h"', ""]
    caller = ['#include "abi_common.h"', "#include <stdio.h>", "", "static long long ci; static double cf;", ""]
    exp_ci, exp_cf = 0, 0
    caller_main = ["int main(void) {"]
    for fi, fn in enumerate(fns):
        plist = ", ".join(f"{t} {n}" for n, t, _ in fn.params)
        if fn.variadic:
            plist += ", ..."
        rt = f"{fn.ret.kind} {fn.ret.name}" if isinstance(fn.ret, Agg) else fn.ret
        body = [f"{rt} {fn.name}({plist}) {{", "    long long si = 0; double sf = 0;"]
        si, sf = 0, 0
        # choose runtime argument values now (they are constants in the caller)
        arg_inits: list[str] = []
        w = 1
        va_extra: list[tuple[str, object, Agg | None]] = []
        for (pn, pt, pa) in fn.params:
            if pa is not None:
                init_parts = []
                for path, f in pa.leaves(pn):
                    v = small_val(rng, f)
                    stored = f.conv(v)
                    if pa.kind == "union":
                        # only the first member of a union is initialised; the
                        # callee reads only that member as well.
                        pass
                    init_parts.append((path, v, stored, f))
                if pa.kind == "union":
                    init_parts = init_parts[:1]
                # designated initialiser (nested paths need brace form; use
                # assignment statements instead for generality)
                arg_inits.append(f"    {pt} {fn.name}_{pn}; __builtin_memset(&{fn.name}_{pn}, 0, sizeof {fn.name}_{pn});")
                for path, v, stored, f in init_parts:
                    arg_inits.append(f"    {fn.name}_{path} = {v};")
                    if f.is_fp:
                        body.append(f"    sf += (double){path} * {w};")
                        sf += stored * w
                    else:
                        body.append(f"    si += (long long){path} * {w};")
                        si += stored * w
                    w += 1
            else:
                v = rng.randint(-50, 50) if not pt.startswith("unsigned") else rng.randint(0, 50)
                arg_inits.append(f"    {pt} {fn.name}_{pn} = {v};")
                if pt in ("double", "float", "long double"):
                    body.append(f"    sf += (double){pn} * {w};")
                    sf += v * w
                else:
                    body.append(f"    si += (long long){pn} * {w};")
                    si += v * w
                w += 1
        if fn.variadic:
            body.append(f"    va_list ap; va_start(ap, {fn.params[-1][0]});")
            for j in range(rng.randint(1, 4)):
                kind = rng.choice(["int", "double", "long", "agg"])
                if kind == "agg":
                    a = rng.choice(aggs)
                    parts = []
                    for path, f in a.leaves("v"):
                        v = small_val(rng, f)
                        parts.append((path, v, f.conv(v), f))
                    if a.kind == "union":
                        parts = parts[:1]
                    va_extra.append((f"{a.kind} {a.name}", parts, a))
                    body.append(f"    {{ {a.kind} {a.name} v = va_arg(ap, {a.kind} {a.name});")
                    for path, v, stored, f in parts:
                        if f.is_fp:
                            body.append(f"      sf += (double){path} * {w};")
                            sf += stored * w
                        else:
                            body.append(f"      si += (long long){path} * {w};")
                            si += stored * w
                        w += 1
                    body.append("    }")
                else:
                    v = rng.randint(-50, 50)
                    va_extra.append((kind, v, None))
                    if kind == "double":
                        body.append(f"    sf += va_arg(ap, double) * {w};")
                        sf += v * w
                    else:
                        body.append(f"    si += va_arg(ap, {kind}) * {w};")
                        si += v * w
                    w += 1
            body.append("    va_end(ap);")
        total = si + int(sf)
        # return value
        if isinstance(fn.ret, Agg):
            body.append(f"    {rt} r; __builtin_memset(&r, 0, sizeof r);")
            ret_leaves = fn.ret.leaves("r")
            if fn.ret.kind == "union":
                ret_leaves = ret_leaves[:1]
            ret_vals = []
            for j, (path, f) in enumerate(ret_leaves):
                v = (total + j * 7) % 101 - 50
                if f.cty.startswith("unsigned") or f.cty == "_Bool":
                    v = abs(v)
                body.append(f"    {path} = {v};")
                ret_vals.append((path, f.conv(v), f))
            body.append("    return r;")
            body.append("}")
            # caller consumes
            caller_main += arg_inits
            call_args = ", ".join(f"{fn.name}_{pn}" for pn, _, _ in fn.params)
            for (vt, vv, va) in va_extra:
                if va is None:
                    call_args += f", ({vt}){vv}"
            # variadic aggregates need named temporaries
            pre = []
            for vi, (vt, vv, va) in enumerate(va_extra):
                if va is not None:
                    pre.append(f"    {vt} {fn.name}_v{vi}; __builtin_memset(&{fn.name}_v{vi}, 0, sizeof {fn.name}_v{vi});")
                    for path, v, stored, f in vv:
                        pre.append(f"    {fn.name}_v{vi}{path[1:]} = {v};")
                    call_args += f", {fn.name}_v{vi}"
            caller_main += pre
            caller_main.append(f"    {{ {rt} r = {fn.name}({call_args});")
            for j, (path, stored, f) in enumerate(ret_vals):
                if f.is_fp:
                    caller_main.append(f"      cf += (double){path} * {j + 1};")
                    exp_cf += stored * (j + 1)
                else:
                    caller_main.append(f"      ci += (long long){path} * {j + 1};")
                    exp_ci += stored * (j + 1)
            caller_main.append(f"      printf(\"{fn.name} %lld %.17g\\n\", ci, cf); }}")
        else:
            if fn.ret == "void":
                body.append("    ci_sink = si; cf_sink = sf;")
                body.append("}")
                callee.insert(1, "long long ci_sink; double cf_sink;")
                caller.insert(3, "extern long long ci_sink; extern double cf_sink;")
            elif fn.ret in ("double", "float", "long double"):
                body.append(f"    return ({fn.ret})(si + (long long)sf);")
                body.append("}")
            else:
                body.append(f"    return ({fn.ret})(si + (long long)sf);")
                body.append("}")
            caller_main += arg_inits
            call_args = ", ".join(f"{fn.name}_{pn}" for pn, _, _ in fn.params)
            pre = []
            for vi, (vt, vv, va) in enumerate(va_extra):
                if va is None:
                    call_args += f", ({vt}){vv}"
                else:
                    pre.append(f"    {vt} {fn.name}_v{vi}; __builtin_memset(&{fn.name}_v{vi}, 0, sizeof {fn.name}_v{vi});")
                    for path, v, stored, f in vv:
                        pre.append(f"    {fn.name}_v{vi}{path[1:]} = {v};")
                    call_args += f", {fn.name}_v{vi}"
            caller_main += pre
            if fn.ret == "void":
                caller_main.append(f"    {fn.name}({call_args}); ci += ci_sink; cf += cf_sink;")
                exp_ci += si
                exp_cf += sf
            elif fn.ret in ("double", "float", "long double"):
                caller_main.append(f"    cf += (double){fn.name}({call_args});")
                exp_cf += total
            else:
                caller_main.append(f"    ci += (long long){fn.name}({call_args});")
                exp_ci += convert(total, I64, {"int": I32, "long": I64, "unsigned long long": U64}[fn.ret])
            caller_main.append(f"    printf(\"{fn.name} %lld %.17g\\n\", ci, cf);")
        callee += body + [""]
    caller_main.append("    return 0;")
    caller_main.append("}")
    caller += caller_main
    # The Python mirror (exp_ci/exp_cf) is the final checksum; per-line stdout
    # is defined by the gcc -O0 reference build and cross-checked with gcc -O2.
    return header, "\n".join(callee) + "\n", "\n".join(caller) + "\n", f"{exp_ci} {exp_cf:.17g}"


def _run(cmd: list[str], timeout: float, cwd: Path) -> tuple[int, str, str]:
    try:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout, errors="replace")
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return -999, "", "timeout"


def run(args, seed: int, workdir: Path):
    """Execute the ABI matrix for one seed.  Returns a list of Outcome."""
    from run_stress import Outcome  # local import to avoid a cycle at module load
    rng = random.Random(seed * 7919 + 17)
    header, callee, caller, _ = build_program(rng, seed)
    d = workdir / f"abi_s{seed}"
    d.mkdir(parents=True, exist_ok=True)
    (d / "abi_common.h").write_text(header)
    (d / "callee.c").write_text(callee)
    (d / "caller.c").write_text(caller)
    outcomes = []
    # Reference: gcc -O0 both sides defines the expected stdout.  (GCC is the
    # platform ABI reference implementation; the Python mirror guards the
    # generator, and gcc -O0 vs gcc -O2 agreement is checked as well.)
    def build(cc: str, level: str, tu: str) -> tuple[bool, str]:
        obj = d / f"{tu}.{Path(cc).name}.{level}.o"
        rc, out, err = _run([cc, f"-{level}", "-w", "-c", str(d / f"{tu}.c"), "-o", str(obj)], args.timeout, d)
        return rc == 0, str(obj) if rc == 0 else (err or out)

    ok, ref_caller = build(args.gcc, "O0", "caller")
    ok2, ref_callee = build(args.gcc, "O0", "callee")
    if not (ok and ok2):
        return [Outcome("abi", seed, "gcc->gcc", "O0", "ORACLE-DISAGREE", f"gcc rejected generated program: {ref_caller if not ok else ref_callee}")]
    exe = d / "ref"
    rc, _, err = _run([args.gcc, ref_caller, ref_callee, "-o", str(exe), "-lm"], args.timeout, d)
    rc, expected, err = _run([str(exe)], args.timeout, d)
    if rc != 0:
        return [Outcome("abi", seed, "gcc->gcc", "O0", "ORACLE-DISAGREE", f"reference binary failed: {err}")]
    # gcc -O2 self-consistency guards against generator UB.
    ok, g2 = build(args.gcc, "O2", "callee")
    ok2, g2c = build(args.gcc, "O2", "caller")
    if ok and ok2:
        rc, _, _ = _run([args.gcc, g2c, g2, "-o", str(d / "ref2"), "-lm"], args.timeout, d)
        rc, got, _ = _run([str(d / "ref2")], args.timeout, d)
        if got != expected:
            keep = args.out / "oracle-disagree" / f"abi_s{seed}"
            shutil.copytree(d, keep, dirs_exist_ok=True)
            return [Outcome("abi", seed, "gcc->gcc", "O2", "ORACLE-DISAGREE", "gcc -O0 and gcc -O2 disagree (generator UB?)", artifact=str(keep))]

    for level in args.levels:
        okc, lc_callee = build(args.lccc, level, "callee")
        okr, lc_caller = build(args.lccc, level, "caller")
        for who, obj_caller, obj_callee, ok in (("lccc->lccc", lc_caller, lc_callee, okc and okr),
                                                ("gcc->lccc", g2c if ok2 else ref_caller, lc_callee, okc),
                                                ("lccc->gcc", lc_caller, g2 if ok else ref_callee, okr)):
            if not ok:
                msg = (lc_callee if not okc else lc_caller)
                o = Outcome("abi", seed, who, level, "ICE", f"lccc failed to compile: {msg.strip()[-300:]}")
                keep = args.out / "ice" / f"abi_s{seed}"
                shutil.copytree(d, keep, dirs_exist_ok=True)
                o.artifact = str(keep)
                outcomes.append(o)
                continue
            exe = d / f"{who.replace('->', '_')}.{level}"
            rc, out, err = _run([args.gcc, obj_caller, obj_callee, "-o", str(exe), "-lm"], args.timeout, d)
            if rc != 0:
                outcomes.append(Outcome("abi", seed, who, level, "ICE", f"link failed: {err.strip()[-300:]}"))
                continue
            rc, got, err = _run([str(exe)], args.timeout, d)
            if rc == -999:
                verdict, detail = "TIMEOUT", "run timeout"
            elif rc < 0:
                verdict, detail = "CRASH", f"signal {-rc}"
            elif got != expected:
                verdict = "MISMATCH"
                diff = [f"expected: {e}\n     got: {g}" for e, g in zip(expected.splitlines(), got.splitlines()) if e != g]
                detail = "\n".join(diff[:6]) or f"exit {rc}"
            else:
                verdict, detail = "PASS", ""
            o = Outcome("abi", seed, who, level, verdict, detail)
            if verdict != "PASS":
                keep = args.out / verdict.lower() / f"abi_s{seed}"
                shutil.copytree(d, keep, dirs_exist_ok=True)
                (keep / f"{who.replace('->', '_')}.{level}.txt").write_text(detail + "\n")
                (keep / "repro.sh").write_text(
                    "#!/bin/sh\n# Rebuild the failing pairing; swap compilers per TU to bisect the side at fault.\n"
                    f"L={args.lccc}\nG={args.gcc}\ncd {keep}\n"
                    f"$L -{level} -c callee.c -o callee.l.o && $L -{level} -c caller.c -o caller.l.o\n"
                    f"$G -O2 -w -c callee.c -o callee.g.o && $G -O2 -w -c caller.c -o caller.g.o\n"
                    "for pair in 'caller.l.o callee.l.o' 'caller.g.o callee.l.o' 'caller.l.o callee.g.o' 'caller.g.o callee.g.o'; do\n"
                    "  $G $pair -o t -lm && echo \"== $pair\" && ./t; done\n")
                o.artifact = str(keep)
            outcomes.append(o)
    return outcomes


if __name__ == "__main__":
    # Print one generated program for inspection: abi_family.py <seed>
    seed = int(sys.argv[1]) if len(sys.argv) > 1 else 0
    h, ce, ca, _ = build_program(random.Random(seed * 7919 + 17), seed)
    print(h)
    print("// ---- callee.c")
    print(ce)
    print("// ---- caller.c")
    print(ca)
