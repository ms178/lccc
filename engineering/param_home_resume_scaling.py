#!/usr/bin/env python3
"""Byte-exact scaling proof for the resumed parameter-home index.

Compiles generated parameter-heavy translation units with a PRISTINE main leg
and the refactored leg at identical optimization levels, asserts the emitted
assembly is BYTE-IDENTICAL (so this is a pure compiler-speed comparison, not a
code-quality trade), and records the ``[TIME] codegen`` / ``[TIME] total
compile`` witness emitted by ``CCC_TIME_PHASES=1``.

Two shapes per size:

* ``aligned``  -- strictly increasing homes, the hash-table-free fast path.
* ``shifted``  -- a volatile local shifts the homes, so the aligned probe
  fails and the general pass runs.

The body is a FLAT list of ``s += pI;`` statements.  A single
``return p0 + p1 + ... + pN;`` builds an N-deep left-leaning AST and makes the
front-end itself quadratic (measured 1.1 s / 6.5 s / 50.2 s at
n=1000/2000/4000), which would measure the expression parser rather than the
index.  This correction is itself a red-team finding about the benchmark.

Wall-clock ratios are noisy on a shared box; the authoritative complexity
proof is the machine-independent ``ParamHomeStats`` witness pinned by unit
tests (``chain_steps <= params`` and
``instructions_examined <= entry_instructions + 1``).  This script exists to
prove there is NO regression on realistic code and that codegen is unchanged.

Usage:
  python3 engineering/param_home_resume_scaling.py \
      --old target/fastbuild-main-baseline/fastbuild \
      --new target/fastbuild --out /tmp/scaling.json
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

SIZES = (500, 1000, 2000, 4000, 8000, 16000)
CODEGEN = re.compile(r"\[TIME\] codegen:\s+([0-9.]+)s")
TOTAL = re.compile(r"\[TIME\] total compile [^:]*:\s+([0-9.]+)s")


def source(n: int, shifted: bool) -> str:
    params = ", ".join(f"int p{i}" for i in range(n))
    args = ", ".join(str(i + 1) for i in range(n))
    body_local = "    volatile int guard = p0;\n" if shifted else ""
    guard_check = "    if (guard != p0) return -1;\n" if shifted else ""
    adds = "".join(f"    s += p{i};\n" for i in range(n))
    return (f"int f({params}) {{\n{body_local}    int s = 0;\n{adds}"
            f"{guard_check}    return s;\n}}\n"
            f"int main(void) {{ return f({args}) != "
            f"{n * (n + 1) // 2}; }}\n")


def compile_one(compiler: Path, src: Path, out: Path, level: str) -> dict:
    env = os.environ.copy()
    env["CCC_TIME_PHASES"] = "1"
    t0 = time.monotonic()
    p = subprocess.run([str(compiler), "-w", level, "-S", str(src), "-o", str(out)],
                       capture_output=True, text=True, env=env, timeout=600)
    wall = time.monotonic() - t0
    if p.returncode != 0:
        raise RuntimeError(f"{compiler.name} rc={p.returncode}: "
                           + (p.stdout + p.stderr)[-400:])
    cg = CODEGEN.findall(p.stderr)
    tot = TOTAL.findall(p.stderr)
    if not cg or not tot:
        raise RuntimeError(f"{compiler.name} did not emit the timing witness: "
                           + p.stderr[-400:])
    asm = out.read_bytes()
    return dict(codegen=float(cg[-1]), total=float(tot[-1]), wall=wall,
                sha256=hashlib.sha256(asm).hexdigest(), asm_bytes=len(asm))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--old", type=Path, required=True)
    ap.add_argument("--new", type=Path, required=True)
    ap.add_argument("--out", type=Path, default=None)
    ap.add_argument("--sizes", default=",".join(map(str, SIZES)))
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        with tempfile.TemporaryDirectory() as td:
            work = Path(td)
            src = work / "t.c"
            src.write_text(source(8, True))
            assert "volatile int guard" in src.read_text()
            assert src.read_text().count("    s += p") == 8
            old = compile_one(args.old / "lccc", src, work / "old.s", "-O2")
            new = compile_one(args.new / "lccc", src, work / "new.s", "-O2")
            assert old["sha256"] == new["sha256"], "self-test: asm diverged"
            assert old["codegen"] >= 0 and new["total"] >= 0
        print("PARAM_HOME_RESUME_SELFTEST_OK (timing witness parsed, asm identical)")
        return 0

    sizes = [int(x) for x in args.sizes.split(",") if x.strip()]
    rows = []
    with tempfile.TemporaryDirectory(prefix="param-home-resume-") as td:
        work = Path(td)
        for shape in ("aligned", "shifted"):
            for n in sizes:
                src = work / f"{shape}-{n}.c"
                src.write_text(source(n, shape == "shifted"))
                old = compile_one(args.old / "lccc", src, work / f"old-{shape}-{n}.s", "-O2")
                new = compile_one(args.new / "lccc", src, work / f"new-{shape}-{n}.s", "-O2")
                identical = old["sha256"] == new["sha256"] and old["asm_bytes"] == new["asm_bytes"]
                rows.append(dict(shape=shape, n=n, identical=identical,
                                 old_codegen=old["codegen"], new_codegen=new["codegen"],
                                 old_total=old["total"], new_total=new["total"],
                                 old_wall=old["wall"], new_wall=new["wall"],
                                 asm_sha256=new["sha256"], asm_bytes=new["asm_bytes"]))
                print(f"{shape:8s} n={n:<6d} "
                      f"codegen {old['codegen']:.3f} -> {new['codegen']:.3f}s "
                      f"({old['codegen'] / new['codegen']:.2f}x)  "
                      f"total {old['total']:.3f} -> {new['total']:.3f}s  "
                      f"asm_identical={identical}")

    report = dict(old=str(args.old), new=str(args.new), shapes=["aligned", "shifted"],
                  sizes=sizes, rows=rows,
                  all_asm_identical=all(r["identical"] for r in rows))
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(report, indent=2) + "\n")
    if not report["all_asm_identical"]:
        print("PARAM_HOME_RESUME_FAIL: assembly diverged", flush=True)
        return 1
    print(f"PARAM_HOME_RESUME_OK ({len(rows)} cases, byte-identical asm)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
