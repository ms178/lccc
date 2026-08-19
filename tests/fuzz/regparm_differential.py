#!/usr/bin/env python3
"""Differential fuzzer for the i386 `-mregparm=3` ABI (register parameters).

The kernel boot code (arch/x86/boot) is compiled with `-mregparm=3`, but the
base m32_differential_fuzz.py only exercises the stack-argument cdecl ABI.
This variant passes probe(seed, iter) with seed in %eax and iter in %edx and
verifies the folded 8-bit exit oracle against gcc -m32 -mregparm=3.

Usage:
  python3 regparm_differential.py --ccc path/to/lccc [--seeds 0:150] \
      [--levels O0,O2,Os] [--out /tmp/regparmfz]
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

_HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))
from m32_differential_fuzz import gen_c  # noqa: E402

DRIVER_RP = r"""
.globl _start
_start:
    xorl %ebp, %ebp          # accumulator
    movl $SEEDVAL, %esi      # input seed
    movl $17, %edi           # iterations
1:
    movl %esi, %eax          # regparm arg 0
    movl %edi, %edx          # regparm arg 1
    call probe
    xorl %eax, %ebp
    roll $5, %ebp
    imull $0x01000193, %esi, %esi
    addl $0x9e3779b9, %esi
    decl %edi
    jnz 1b
    movl %ebp, %ebx
    andl $255, %ebx
    movl $1, %eax
    int $0x80
"""


def run(cmd, timeout=20):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stderr
    except Exception as e:  # noqa: BLE001
        return "ERR", repr(e)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ccc", required=True)
    ap.add_argument("--gcc", default="/usr/bin/gcc")
    ap.add_argument("--seeds", default="0:150")
    ap.add_argument("--levels", default="O0,O2,Os")
    ap.add_argument("--out", default="/tmp/regparmfz")
    args = ap.parse_args()

    lo, hi = (int(x) for x in args.seeds.split(":"))
    levels = args.levels.split(",")
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    results = {"pass": 0, "mismatch": 0, "lccc_compile_failure": 0, "skip": 0}
    failures = []
    for seed in range(lo, hi):
        src = out / f"s{seed}.c"
        src.write_text(gen_c(seed))
        drv = out / f"d{seed}.s"
        drv.write_text(DRIVER_RP.replace("SEEDVAL", str(0x1234 + seed * 7919)))
        for level in levels:
            common = ["-m32", "-mregparm=3", "-fno-PIE", "-fomit-frame-pointer",
                      f"-{level}", "-w"]
            lo_obj = out / f"s{seed}_{level}_l.o"
            go_obj = out / f"s{seed}_{level}_g.o"
            rc, err = run([args.ccc, *common, "-c", str(src), "-o", str(lo_obj)])
            if rc != 0:
                results["lccc_compile_failure"] += 1
                failures.append({"seed": seed, "level": level, "kind": "ccc_compile",
                                 "err": err[-400:]})
                continue
            rc, _ = run([args.gcc, *common, "-c", str(src), "-o", str(go_obj)])
            if rc != 0:
                results["skip"] += 1
                continue
            lbin = out / f"b{seed}_{level}_l"
            gbin = out / f"b{seed}_{level}_g"
            rc1, _ = run([args.gcc, "-m32", "-nostdlib", "-no-pie", str(drv),
                          str(lo_obj), "-o", str(lbin)])
            rc2, _ = run([args.gcc, "-m32", "-nostdlib", "-no-pie", str(drv),
                          str(go_obj), "-o", str(gbin)])
            if rc1 != 0 or rc2 != 0:
                results["skip"] += 1
                continue
            lrc, _ = run([str(lbin)], timeout=10)
            grc, _ = run([str(gbin)], timeout=10)
            if lrc == grc and isinstance(lrc, int):
                results["pass"] += 1
            else:
                results["mismatch"] += 1
                failures.append({"seed": seed, "level": level, "kind": "mismatch",
                                 "lccc_exit": str(lrc), "gcc_exit": str(grc)})

    summary = {"results": results, "failures": failures[:20],
               "seeds": args.seeds, "levels": levels}
    (out / "summary.json").write_text(json.dumps(summary, indent=1))
    print(json.dumps(results))
    return 0 if results["mismatch"] == 0 and results["lccc_compile_failure"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
