#!/usr/bin/env python3
"""Semantic verification for the PR #713 audit fixes.

See ``docs/reviews/PR713-audit-response.md`` for the finding-by-finding
adjudication this script provides evidence for.

Two independent probes, both driven through the real compiler and compared
against ``gcc -O0`` -- a compiler that cannot contract, reassociate or
vectorize, and is therefore the reference for what the source program means:

1. The inclusive-bound envelope of the packed matmul arm, over
   N in {16,17,31,32,33,48,49,64,65} x {"<", "<="} plus a RUNTIME bound
   (``j <= lim``).  The interesting sizes are the ones where the vector body
   stops short of the row and the scalar remainder runs: that is where the
   pre-existing miscompile produced wrong numbers on the default command line.

2. The FMA contract and target gating: the packed arm must emit FMAs under the
   default C contract and under ``-ffp-contract=fast``, and NONE under
   ``-ffp-contract=off`` or ``-mno-fma``.

Both have fast-battery counterparts in ``check_fma_gating.sh`` (contracts
1-5), which pin exact counts and one runtime-bound number.  This script is the
WIDE envelope: it sweeps every size at which the remainder length changes,
which is how the original miscompile was found and why a single-size test
would not have caught it.  Run this when touching the vectorizer; run the
shell gate on every push.

  LCCC=/path/to/lccc python3 tests/regression/verify_pr713_audit.py
"""
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
LCCC = os.environ.get("LCCC", str(ROOT / "target/fastbuild/lccc"))
GCC = os.environ.get("GCC", "gcc")

MATMUL_SRC = """
#include <stdio.h>
#define N %(n)d
static double A[N][N], B[N][N], C[N][N];
void matmul(void){for(int i=0;i<N;i++)for(int k=0;k<N;k++)for(int j=0;j%(op)s%(bound)s;j++)C[i][j]+=A[i][k]*B[k][j];}
int main(void){ for(int i=0;i<N;i++)for(int j=0;j<N;j++){A[i][j]=(double)((i*7+j*3)%%13)/7.0;B[i][j]=(double)((i*5+j)%%11)/5.0;}
  matmul(); double h=0; for(int i=0;i<N;i++)for(int j=0;j<N;j++)h+=C[i][j]; printf("%%.6f\\n",h); return 0;}
"""

RUNTIME_SRC = """
#include <stdio.h>
#define N %(n)d
static double A[N][N], B[N][N], C[N][N];
int lim = %(n)d - 1;
void matmul(void){for(int i=0;i<N;i++)for(int k=0;k<N;k++)for(int j=0;j<=lim;j++)C[i][j]+=A[i][k]*B[k][j];}
int main(void){ for(int i=0;i<N;i++)for(int j=0;j<N;j++){A[i][j]=(double)((i*7+j*3)%%13)/7.0;B[i][j]=(double)((i*5+j)%%11)/5.0;}
  matmul(); double h=0; for(int i=0;i<N;i++)for(int j=0;j<N;j++)h+=C[i][j]; printf("%%.6f\\n",h); return 0;}
"""

SHAPE_SRC = """
#define N 64
static double A[N][N], B[N][N], C[N][N];
void mm(void) { for (int i = 0; i < N; i++) for (int k = 0; k < N; k++) for (int j = 0; j < N; j++) C[i][j] += A[i][k] * B[k][j]; }
"""

SIZES = (16, 17, 31, 32, 33, 48, 49, 64, 65)
LCCC_FLAGS = "-O2 -march=x86-64-v3"


def run(argv):
    return subprocess.run(argv, capture_output=True, text=True)


def build_run(cc, flags, source, tag, work):
    """Compile and run, returning stripped stdout (None if it did not build)."""
    exe = work / f"bin_{tag}"
    r = run([cc, *flags.split(), "-o", str(exe), str(source)])
    if r.returncode != 0:
        return None
    return run([str(exe)]).stdout.strip()


def envelope(work, failures, say):
    """Probe 1: the packed matmul arm against the oracle, everywhere."""
    say("1. inclusive-bound envelope (matmul, j < N and j <= N-1, plus runtime)")
    for n in SIZES:
        for op, bound in (("<", str(n)), ("<=", f"{n}-1")):
            src = work / f"env_{'lt' if op == '<' else 'le'}_{n}.c"
            src.write_text(MATMUL_SRC % dict(n=n, op=op, bound=bound))
            ref = build_run(GCC, "-O0", src, f"ref_{op}_{n}", work)
            got = build_run(LCCC, LCCC_FLAGS, src, f"new_{op}_{n}", work)
            if ref is None or got is None:
                failures.append(f"N={n} j{op}{bound}: build failed")
                say(f"   N={n:<4} j{op}{bound:<6} BUILD FAILED")
                continue
            if ref != got:
                failures.append(f"N={n} {op} {bound}: lccc={got} ref={ref}")
            say(
                f"   N={n:<4} j{op}{bound:<6} lccc={got:<20} ref={ref:<20} "
                f"{'ok' if ref == got else 'MISMATCH'}"
            )

    src = work / "env_rt.c"
    src.write_text(RUNTIME_SRC % dict(n=17))
    ref = build_run(GCC, "-O0", src, "ref_rt", work)
    got = build_run(LCCC, LCCC_FLAGS, src, "new_rt", work)
    ok = ref is not None and ref == got
    if not ok:
        failures.append("runtime inclusive bound (j <= lim)")
    say(f"   N=17   j<=lim   lccc={got}  ref={ref}  {'ok' if ok else 'MISMATCH'}")


def gating(work, failures, say):
    """Probe 2: the packed arm obeys the contract and the target."""
    say("\n2. FMA contract and target gating (64x64 matmul shape)")
    src = work / "shape.c"
    src.write_text(SHAPE_SRC)
    flagsets = [
        (LCCC_FLAGS, True, "default C contract (fast, FMA3 present)"),
        (f"{LCCC_FLAGS} -ffp-contract=off", False, "-ffp-contract=off"),
        (f"{LCCC_FLAGS} -mno-fma", False, "-mno-fma (no FMA3 unit)"),
        (f"{LCCC_FLAGS} -ffp-contract=fast", True, "-ffp-contract=fast"),
    ]
    for flags, want_fma, label in flagsets:
        out = work / "mm.s"
        r = run([LCCC, *flags.split(), "-S", "-o", str(out), str(src)])
        if r.returncode != 0:
            failures.append(f"FMA gating: {label}: compile failed")
            say(f"   {label:<45} COMPILE FAILED")
            continue
        n = out.read_text().count("vfmadd")
        ok = (n > 0) == want_fma
        if not ok:
            failures.append(f"FMA gating: {label} -> vfmadd={n}")
        say(f"   {label:<45} vfmadd={n:<3} {'ok' if ok else 'FAIL'}")


def main():
    quiet = "--quiet" in sys.argv
    say = (lambda *a: None) if quiet else print

    if not os.path.exists(LCCC):
        print(f"verify_pr713_audit: lccc not found at {LCCC}", file=sys.stderr)
        return 1
    if shutil.which(GCC) is None:
        print("verify_pr713_audit: no gcc oracle", file=sys.stderr)
        return 1

    work = pathlib.Path(tempfile.mkdtemp(prefix="lccc-pr713-"))
    failures = []
    try:
        envelope(work, failures, say)
        gating(work, failures, say)
    finally:
        shutil.rmtree(work, ignore_errors=True)

    if failures:
        print("\nFAILURES:")
        for f in failures:
            print(f"   - {f}")
        print("\nRESULT: FAIL")
        return 1
    say("\nRESULT: ALL CHECKS PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
