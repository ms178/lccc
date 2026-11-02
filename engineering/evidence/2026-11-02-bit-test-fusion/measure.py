#!/usr/bin/env python3
"""Per-function instruction counts of the browser bit-index kernels.

    measure.py [--lccc DIR] [--base DIR] [--icx BIN] [--json OUT] [--notice]

Compiles tests/regression/fixtures/bit_width/browser_bits.c at -O2 with every
available compiler on every target and counts the instructions of each kernel
(every non-blank line after the kernel's label that is not a label, a directive
or a comment; `ret` and the frame are included for every compiler alike).

  x86-64   gcc-16 / clang-23 / icx -O2 -fPIE        lccc     -O2 -fPIE
  i686     gcc-16 / clang-23 -m32 -O2 -fno-pic      lccc-i686 -O2 -fno-pic
  aarch64  clang-23 --target=aarch64-linux-gnu      lccc-arm -O2
  riscv64  clang-23 --target=riscv64-linux-gnu -march=rv64gc   lccc-riscv -O2

`--lccc DIR` holds the candidate lccc binaries (default target/fastbuild),
`--base DIR` an optional second set (the merge base) shown as its own column.
`--notice` prints one GitHub `::notice` per target with every count, which is
how the authenticated-ICX CI lane publishes counts to the check-run API.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
SRC = ROOT / "tests/regression/fixtures/bit_width/browser_bits.c"
LABEL = re.compile(r"^([A-Za-z_][\w.$]*):")
DEF = re.compile(r"^(?:int|void|u32|static int)\s+([a-z_0-9]+)\(", re.M)


def kernels():
    return [n for n in DEF.findall(SRC.read_text()) if n != "ps_mask_test_bit" and n != "hit"]


def counts(asm, names):
    out, cur = {}, None
    for raw in asm.splitlines():
        line = raw.strip()
        if not line:
            continue
        m = LABEL.match(line)
        if m:
            cur = m.group(1) if m.group(1) in names else None
            if cur:
                out.setdefault(cur, 0)
            continue
        if cur and not line.startswith((".", "#", "//", ";")):
            out[cur] += 1
    return out


def compile_asm(cmd, env=None):
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "k.s"
        p = subprocess.run([*cmd, "-S", str(SRC), "-o", str(out)], capture_output=True,
                           text=True, env=env, timeout=300)
        if p.returncode != 0:
            raise SystemExit(f"{' '.join(cmd)} failed:\n{p.stderr}")
        return out.read_text()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lccc", default=str(ROOT / "target/fastbuild"))
    ap.add_argument("--base")
    ap.add_argument("--icx", default=os.environ.get("LCCC_ICX_BIN"))
    ap.add_argument("--gcc", default=shutil.which("gcc-16"))
    ap.add_argument("--clang", default=os.environ.get("CLANG") or shutil.which("clang-23"))
    ap.add_argument("--json")
    ap.add_argument("--notice", action="store_true")
    ap.add_argument("--target", action="append")
    args = ap.parse_args()
    names = kernels()
    env = {k: v for k, v in os.environ.items() if not k.startswith("CCC_")}
    matrix = {
        "x86-64": [("gcc", args.gcc, ["-O2", "-fPIE"]), ("clang", args.clang, ["-O2", "-fPIE"]),
                   ("icx", args.icx, ["-O2", "-fPIE"]), ("base", args.base and f"{args.base}/lccc", ["-O2", "-fPIE"]),
                   ("lccc", f"{args.lccc}/lccc", ["-O2", "-fPIE"])],
        "i686": [("gcc", args.gcc, ["-m32", "-O2", "-fno-pic"]),
                 ("clang", args.clang, ["-m32", "-O2", "-fno-pic"]),
                 ("base", args.base and f"{args.base}/lccc-i686", ["-O2", "-fno-pic"]),
                 ("lccc", f"{args.lccc}/lccc-i686", ["-O2", "-fno-pic"])],
        "aarch64": [("clang", args.clang, ["--target=aarch64-linux-gnu", "-ffreestanding", "-O2"]),
                    ("base", args.base and f"{args.base}/lccc-arm", ["-O2"]),
                    ("lccc", f"{args.lccc}/lccc-arm", ["-O2"])],
        "riscv64": [("clang", args.clang, ["--target=riscv64-linux-gnu", "-march=rv64gc", "-ffreestanding", "-O2"]),
                    ("base", args.base and f"{args.base}/lccc-riscv", ["-O2"]),
                    ("lccc", f"{args.lccc}/lccc-riscv", ["-O2"])],
    }
    result = {}
    for target, compilers in matrix.items():
        if args.target and target not in args.target:
            continue
        cols = {}
        for name, binary, flags in compilers:
            if not binary or not Path(binary).exists() and not shutil.which(binary):
                continue
            cols[name] = counts(compile_asm([binary, *flags], env), set(names))
        result[target] = cols
        header = ["kernel", *cols]
        print(f"\n## {target} (-O2)\n")
        print("| " + " | ".join(header) + " |")
        print("|" + "---|" * len(header))
        for n in names:
            print("| " + " | ".join([f"`{n}`", *(str(cols[c].get(n, "-")) for c in cols)]) + " |")
        totals = [str(sum(cols[c].values())) for c in cols]
        print("| **total** | " + " | ".join(totals) + " |")
        if args.notice:
            for c in cols:
                body = " ".join(f"{n}={cols[c].get(n, '-')}" for n in names)
                print(f"::notice title=browser_bits {target} {c} -O2::{body}")
    if args.json:
        Path(args.json).write_text(json.dumps(result, indent=1, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
