#!/usr/bin/env python3
"""Differential intrinsic test suite: lccc vs GCC.

Each test in tests/intrinsics/cases/ is a C file using intrinsics from the
bundled headers. The runner compiles it with lccc and GCC, runs both, and
compares stdout + exit code. lccc must be given the GCC include path for
<stdint.h> etc., with lccc's own include/ FIRST so its intrinsic headers win.

Usage:
    python3 tests/intrinsics/run_intrin_tests.py [filter]
"""
import argparse
import os
import subprocess
import sys
import re
from pathlib import Path

REPO = Path(__file__).parent.parent.parent
LCCC = Path(os.environ.get("LCCC_BIN", REPO / "target" / "release" / "lccc"))
LCCC_INC = REPO / "include"
GCC = os.environ.get("CC", "gcc")
GCC_INC = subprocess.check_output([GCC, "-print-file-name=include"], text=True).strip()
CASES = Path(__file__).parent / "cases"

COMPILE_TIMEOUT = 60
RUN_TIMEOUT = 30

def run(cmd, timeout=COMPILE_TIMEOUT):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("filter", nargs="?", default="")
    ap.add_argument("--lccc-flags", default="-O2", help="optimization flags for lccc")
    ap.add_argument("--gcc-flags", default="-O2 -mavx512f -mavx512bw -mavx512vl -mavx512dq -mavx512vnni -mavx512vbmi2 -mavx512vpopcntdq -mavx512bitalg -mgfni -mvaes -mvpclmulqdq",
                    help="optimization flags for GCC")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()

    files = sorted(CASES.glob("*.c"))
    if args.filter:
        files = [f for f in files if args.filter in f.name]

    npass = nfail = nskip = 0
    failures = []
    for src in files:
        name = src.stem
        # Per-test flags: lines starting with "// FLAGS: " in the header.
        extra = []
        for line in src.read_text(errors="replace").splitlines()[:10]:
            m = re.match(r"// FLAGS:\s*(.*)", line)
            if m:
                extra = m.group(1).split()
                break
        lccc_flags = (args.lccc_flags + " " + " ".join(extra)).split()
        gcc_flags = (args.gcc_flags + " " + " ".join(extra)).split()

        lbin = REPO / "work" / f"it_{name}_lccc" if (REPO / "work").exists() else Path(f"/home/user/work/it_{name}_lccc")
        gbin = Path(str(lbin).replace("_lccc", "_gcc"))
        lbin.parent.mkdir(parents=True, exist_ok=True)

        # GCC must also find lccc's headers for _mm* availability tests? No:
        # GCC uses its own headers; lccc uses lccc's. lccc needs gcc's std headers.
        lr = run([str(LCCC), f"-I{LCCC_INC}", f"-I{GCC_INC}"] + lccc_flags +
                 [str(src), "-o", str(lbin)])
        gr = run([GCC] + gcc_flags + [str(src), "-o", str(gbin)])
        if lr is None:
            print(f"TIMEOUT {name} (lccc compile)"); nfail += 1; failures.append((name, "lccc compile timeout")); continue
        if gr is None:
            print(f"TIMEOUT {name} (gcc compile)"); nfail += 1; failures.append((name, "gcc compile timeout")); continue
        if lr.returncode != 0:
            err = (lr.stderr or lr.stdout).strip().splitlines()
            print(f"COMPILE-FAIL-LCCC {name}: {err[-1] if err else '?'}")
            nfail += 1; failures.append((name, "lccc compile fail")); continue
        if gr.returncode != 0:
            print(f"SKIP {name} (gcc compile fail)"); nskip += 1; continue

        lout = run([str(lbin)], RUN_TIMEOUT)
        gout = run([str(gbin)], RUN_TIMEOUT)
        if lout is None or gout is None:
            print(f"TIMEOUT {name} (run)"); nfail += 1; failures.append((name, "run timeout")); continue
        if lout.stdout == gout.stdout and lout.returncode == gout.returncode:
            print(f"PASS {name}")
            npass += 1
        else:
            print(f"MISMATCH {name}")
            if args.verbose:
                print(f"  lccc rc={lout.returncode} out={lout.stdout[:300]!r}")
                print(f"  gcc  rc={gout.returncode} out={gout.stdout[:300]!r}")
            nfail += 1
            failures.append((name, "output mismatch"))

    print(f"\n===== {npass} passed, {nfail} failed, {nskip} skipped =====")
    for name, why in failures:
        print(f"  FAIL {name}: {why}")
    return 1 if nfail else 0

if __name__ == "__main__":
    sys.exit(main())
