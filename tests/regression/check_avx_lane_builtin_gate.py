#!/usr/bin/env python3
"""Accept/reject gate and codegen-arm check for the AVX1 128-bit lane builtins.

GCC 16.2 declares the raw builtins that move one 128-bit half of a 256-bit
vector:

    __builtin_ia32_vextractf128_{ps,pd}256(v, sel)  -> 128-bit
    __builtin_ia32_vextractf128_si256(v, sel)       -> 128-bit (v8si/v4si)
    __builtin_ia32_vinsertf128_{ps,pd,si}256(v, h, sel) -> 256-bit

Each is an AVX1 instruction, and the selector is a one-bit immediate. This gate
pins, for each case below, the verdict that GCC 16.2 produces (the reference)
and the verdict LCCC must produce. The check fails if either compiler disagrees
with the pinned verdict, so an expectation cannot silently rot.

Cases cover:
  * the ISA gate: without -mavx every lane builtin is rejected (GCC: "target
    specific option mismatch" / undeclared). LCCC previously accepted them;
  * the immediate: selectors 0 and 1 are accepted, 2, -1 and a runtime value
    are rejected ("the last argument must be an 1-bit immediate");
  * AVX2 implies AVX, so -mavx2 accepts the AVX1 forms.

The codegen check compiles one function per insert and extract form to
assembly and requires the AVX1 mnemonic (`vinsertf128` / `vextractf128`), and
no AVX2 integer mnemonic (`vinserti128` / `vextracti128`). An alias to the
AVX2 op would emit the wrong instruction, which the mnemonic check catches.

Usage:
    check_avx_lane_builtin_gate.py --lccc target/fastbuild/lccc [--gcc gcc-16]
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

PRELUDE = """\
typedef float v8sf __attribute__((vector_size(32)));
typedef float v4sf __attribute__((vector_size(16)));
typedef double v4df __attribute__((vector_size(32)));
typedef double v2df __attribute__((vector_size(16)));
typedef int v8si __attribute__((vector_size(32)));
typedef int v4si __attribute__((vector_size(16)));
int s;
volatile long long sink;
void f(v8sf a, v4sf b, v4df d, v2df e, v8si i, v4si j) {
"""

# The result is stored and its bits read back, so the call is live and the
# immediate is checked (a discarded result can be removed before the check).
BODY = """\
    __typeof__(%s) r = %s;
    sink = ((volatile long long *)&r)[0];
}
"""

ACC, REJ = "accept", "reject"

# (expression, flags, pinned verdict)
CASES: list[tuple[str, str, str]] = [
    # Accepted with -mavx: selector 0 or 1, every name.
    ("__builtin_ia32_vextractf128_ps256(a, 1)", "-O2 -mavx", ACC),
    ("__builtin_ia32_vextractf128_ps256(a, 0)", "-O2 -mavx", ACC),
    ("__builtin_ia32_vextractf128_pd256(d, 1)", "-O2 -mavx", ACC),
    ("__builtin_ia32_vextractf128_si256(i, 1)", "-O2 -mavx", ACC),
    ("__builtin_ia32_vinsertf128_ps256(a, b, 0)", "-O2 -mavx", ACC),
    ("__builtin_ia32_vinsertf128_pd256(d, e, 1)", "-O2 -mavx", ACC),
    ("__builtin_ia32_vinsertf128_si256(i, j, 1)", "-O2 -mavx", ACC),
    # Out-of-range or runtime selector: rejected with -mavx.
    ("__builtin_ia32_vextractf128_ps256(a, 2)", "-O2 -mavx", REJ),
    ("__builtin_ia32_vextractf128_pd256(d, 2)", "-O2 -mavx", REJ),
    ("__builtin_ia32_vextractf128_si256(i, 3)", "-O2 -mavx", REJ),
    ("__builtin_ia32_vinsertf128_ps256(a, b, 2)", "-O2 -mavx", REJ),
    ("__builtin_ia32_vinsertf128_pd256(d, e, -1)", "-O2 -mavx", REJ),
    ("__builtin_ia32_vinsertf128_si256(i, j, 4)", "-O2 -mavx", REJ),
    ("__builtin_ia32_vextractf128_si256(i, s)", "-O2 -mavx", REJ),
    ("__builtin_ia32_vinsertf128_pd256(d, e, s)", "-O2 -mavx", REJ),
    # ISA gate: without -mavx none of the lane builtins is available.
    ("__builtin_ia32_vextractf128_ps256(a, 1)", "-O2", REJ),
    ("__builtin_ia32_vextractf128_pd256(d, 1)", "-O2", REJ),
    ("__builtin_ia32_vextractf128_si256(i, 1)", "-O2", REJ),
    ("__builtin_ia32_vinsertf128_ps256(a, b, 1)", "-O2", REJ),
    ("__builtin_ia32_vinsertf128_pd256(d, e, 1)", "-O2", REJ),
    ("__builtin_ia32_vinsertf128_si256(i, j, 1)", "-O2", REJ),
    # AVX2 implies AVX: the AVX1 forms stay available under -mavx2.
    ("__builtin_ia32_vinsertf128_pd256(d, e, 1)", "-O2 -mavx2", ACC),
    ("__builtin_ia32_vextractf128_si256(i, 1)", "-O2 -mavx2", ACC),
]

# (C function, required mnemonic, forbidden mnemonics)
CODEGEN: list[tuple[str, str, tuple[str, ...]]] = [
    ("void g(v8sf a, v4sf b, v8sf *o) { *o = __builtin_ia32_vinsertf128_ps256(a, b, 1); }",
     "vinsertf128", ("vinserti128",)),
    ("void g(v4df d, v2df e, v4df *o) { *o = __builtin_ia32_vinsertf128_pd256(d, e, 0); }",
     "vinsertf128", ("vinserti128",)),
    ("void g(v8si i, v4si j, v8si *o) { *o = __builtin_ia32_vinsertf128_si256(i, j, 1); }",
     "vinsertf128", ("vinserti128",)),
    ("void g(v8sf a, v4sf *o) { *o = __builtin_ia32_vextractf128_ps256(a, 1); }",
     "vextractf128", ("vextracti128",)),
    ("void g(v4df d, v2df *o) { *o = __builtin_ia32_vextractf128_pd256(d, 1); }",
     "vextractf128", ("vextracti128",)),
    ("void g(v8si i, v4si *o) { *o = __builtin_ia32_vextractf128_si256(i, 1); }",
     "vextractf128", ("vextracti128",)),
]


def verdict(cc: str, flags: str, src: Path, out_dir: Path, asm: bool = False) -> tuple[int, str]:
    """Return (exit code, last stderr line) of compiling `src`."""
    target = out_dir / (src.stem + (".s" if asm else ".o"))
    cmd = [cc, *flags.split(), "-S" if asm else "-c", str(src), "-o", str(target)]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    lines = (proc.stderr or "").strip().splitlines()
    return proc.returncode, lines[-1][:160] if lines else ""


def run_case(job: tuple[int, str, str, str, str, str, Path]) -> dict:
    idx, expr, flags, pinned, lccc, gcc, tmp = job
    src = tmp / f"case_{idx}.c"
    src.write_text(PRELUDE + BODY % (expr, expr))
    lc, lerr = verdict(lccc, flags, src, tmp)
    gc, gerr = verdict(gcc, flags, src, tmp)
    return {
        "expr": expr, "flags": flags, "pinned": pinned,
        "lccc": ACC if lc == 0 else REJ, "gcc": ACC if gc == 0 else REJ,
        "lerr": lerr, "gerr": gerr,
    }


def codegen_check(lccc: str, tmp: Path) -> list[str]:
    """Return failure messages for the codegen-arm check."""
    failures: list[str] = []
    for i, (body, must, forbid) in enumerate(CODEGEN):
        src = tmp / f"cg_{i}.c"
        src.write_text(PRELUDE.split("void f(")[0] + body + "\n")
        rc, err = verdict(lccc, "-O2 -mavx", src, tmp, asm=True)
        if rc != 0:
            failures.append(f"codegen case {i} did not compile: {err}")
            continue
        text = (tmp / f"cg_{i}.s").read_text()
        if not re.search(rf"\b{must}\b", text):
            failures.append(f"codegen case {i}: no `{must}` emitted\n  {body}")
        for bad in forbid:
            if re.search(rf"\b{bad}\b", text):
                failures.append(f"codegen case {i}: forbidden `{bad}` emitted\n  {body}")
    return failures


def self_test(lccc: str, gcc: str, tmp: Path) -> None:
    """Negative control: a deliberately wrong pin must be reported.

    Selector 2 is rejected by both compilers. Pinning it as ACC has to produce
    a mismatch; if the harness still reported it clean, the gate would pass
    every verdict and could never fail."""
    probe = run_case((-1, "__builtin_ia32_vinsertf128_ps256(a, b, 2)", "-O2 -mavx",
                      ACC, lccc, gcc, tmp))
    if probe["lccc"] == ACC and probe["gcc"] == ACC:
        raise SystemExit("self-test: harness accepted a selector both compilers reject")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--lccc", type=Path, required=True)
    ap.add_argument("--gcc", default=os.environ.get("GCC_BIN", "gcc-16"))
    args = ap.parse_args()

    with tempfile.TemporaryDirectory(prefix="avx-lane-gate-") as tmp_s:
        tmp = Path(tmp_s)
        self_test(str(args.lccc), args.gcc, tmp)
        jobs = [(i, e, f, p, str(args.lccc), args.gcc, tmp) for i, (e, f, p) in enumerate(CASES)]
        with cf.ThreadPoolExecutor(max_workers=max(1, os.cpu_count() or 2)) as pool:
            results = list(pool.map(run_case, jobs))
        failures = codegen_check(str(args.lccc), tmp)

    bad = []
    for r in results:
        if r["lccc"] != r["pinned"] or r["gcc"] != r["pinned"]:
            bad.append(r)
    for r in bad:
        print(f"MISMATCH {r['flags']!r} pinned={r['pinned']} lccc={r['lccc']} "
              f"gcc={r['gcc']}\n  {r['expr']}\n  lccc: {r['lerr']}\n  gcc:  {r['gerr']}")
    for msg in failures:
        print("CODEGEN", msg)

    accepted = sum(1 for r in results if r["pinned"] == ACC)
    print(f"cases={len(results)} accept={accepted} reject={len(results) - accepted} "
          f"mismatches={len(bad)} codegen_failures={len(failures)}")
    if bad or failures:
        print("check_avx_lane_builtin_gate: FAILED")
        return 1
    print("check_avx_lane_builtin_gate: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
