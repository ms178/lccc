#!/usr/bin/env python3
"""Four-way runtime oracle for the shifted parameter-home layout.

The 55 A/B assembly divergences are caused by the mem2reg fix (proved by
LCCC_DUMP_IR showing the IR differs BEFORE codegen), not by ParamAllocaIndex.
This tool proves the resulting code is CORRECT on the host CPU by executing
the shifted-layout witness with four independent compilers and requiring one
identical checksum at every optimisation level:

  * pristine origin/main (the A/B "old" leg)
  * the PR tree ("new" leg)
  * GCC 16.2.0  (pinned oracle)
  * Clang 23.1.5 (pinned oracle)

ICX 2026.2 is unavailable in this sandbox; see ICX-LOCAL-FEASIBILITY.md.
Hosted CI's authenticated icx-oracle lane is the third oracle there.  This
tool is a research witness, not a CI gate (the pinned oracles are only
installed in the Debian lanes), so it never makes a hosted lane depend on a
local cache path.

Usage: python3 engineering/oracle_shifted_layout.py [out.json] \
    [--old <fastbuild-dir-or-binary>] [--new <fastbuild-dir-or-binary>]

The two LCCC legs are explicit (or LCCC_AB_OLD / LCCC_AB_NEW) and fail closed:
a stale baseline path is never silently substituted for the one measured.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
import tempfile

from compiler_legs import add_leg_arguments, resolve

CACHE = pathlib.Path("/home/user/.cache/lccc/agent-toolchains/bin")
COMPILERS = {
    "lccc_old": "",   # bound in main() from --old, fail-closed
    "lccc_new": "",   # bound in main() from --new, fail-closed
    "gcc": str(CACHE / "gcc-16"),
    "clang": str(CACHE / "clang-23"),
}
OPTS = ("-O0", "-O2", "-Os")
PARAMS = 64
EXPECTED = sum(range(PARAMS))

GUARD = b"extern int printf(const char *, ...);\nvolatile int guard;\n"


def shifted_source(prefix: bool) -> bytes:
    args = b",".join(b"int p%d" % i for i in range(PARAMS))
    body = [b"int f(" + args + b") {"]
    if prefix:
        body += [b"  volatile int local = guard;", b"  int sum = local;"]
    else:
        body += [b"  int sum = 0;"]
    body += [b"  sum += p%d;" % i for i in range(PARAMS)]
    body += [b"  return sum;", b"}",
             b"int main(void) {",
             b"  int r = f(" + b",".join(b"%d" % i for i in range(PARAMS)) + b");",
             b'  printf("%d\\n", r);', b"  return 0;", b"}"]
    return b"\n".join(body) + b"\n"


def main() -> int:
    ap = argparse.ArgumentParser(description="four-way shifted-layout oracle")
    add_leg_arguments(ap)
    ap.add_argument("out", nargs="?", type=pathlib.Path,
                    default=pathlib.Path("/home/user/artifacts/shifted_layout_oracle.json"))
    ap.add_argument("--out", dest="out_opt", type=pathlib.Path, default=None)
    a = ap.parse_args()
    COMPILERS["lccc_old"], COMPILERS["lccc_new"] = resolve(a.old, a.new)
    missing = [k for k, v in COMPILERS.items()
               if not v or not pathlib.Path(v).exists()]
    if missing:
        print(f"MISSING: {missing}; restore the pinned toolchain cache",
              file=sys.stderr)
        return 2
    out = {"expected": EXPECTED, "params": PARAMS,
           "legs": {k: COMPILERS[k] for k in ("lccc_old", "lccc_new")},
           "rows": []}
    with tempfile.TemporaryDirectory(prefix="lccc-shifted-oracle-") as td:
        d = pathlib.Path(td)
        for label, prefix in (("aligned", False), ("shifted", True)):
            src = d / f"{label}.c"
            src.write_bytes(GUARD + shifted_source(prefix))
            for opt in OPTS:
                results = {}
                for name, cc in COMPILERS.items():
                    exe = d / f"{label}{opt[1:]}-{name}"
                    r = subprocess.run([cc, "-w", opt, str(src), "-o", str(exe)],
                                       capture_output=True)
                    if r.returncode != 0:
                        print(f"COMPILE FAIL {label} {opt} {name}: "
                              f"{r.stderr.decode('utf-8', 'replace')[-200:]}",
                              file=sys.stderr)
                        return 1
                    p = subprocess.run([str(exe)], capture_output=True, timeout=30)
                    results[name] = (p.returncode, p.stdout.strip().decode())
                verdicts = {k: v[1] for k, v in results.items()}
                ok = (all(rc == 0 for rc, _ in results.values())
                      and set(verdicts.values()) == {str(EXPECTED)})
                out["rows"].append({"layout": label, "opt": opt,
                                    "checksums": verdicts, "pass": ok})
                print(f"{label:8s} {opt:4s} " +
                      " ".join(f"{k}={v}" for k, v in verdicts.items()) +
                      f"  -> {'PASS' if ok else 'FAIL'}", flush=True)
                if not ok:
                    return 1
    path = a.out_opt or a.out
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(out, indent=2) + "\n")
    print(f"SHIFTED_LAYOUT_ORACLE_OK ({len(out['rows'])} cases) -> {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
