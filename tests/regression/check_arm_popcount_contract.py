#!/usr/bin/env python3
"""Required compile/link/ISA contract; --run also requires and executes QEMU.

The ordinary fast gate needs no cross runtime. The explicit --run mode fails
if QEMU is missing: runtime coverage is never silently reported as a pass.
"""
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

# Assertions are the gate verdicts, not optional debugging checks. Refuse
# -O/-OO and PYTHONOPTIMIZE before any compiler or runtime can be invoked.
if not __debug__:
    raise SystemExit("AArch64 contract checks require Python assertions; disable -O/-OO and PYTHONOPTIMIZE")

ROOT = Path(__file__).resolve().parents[2]
CC = os.environ.get("CCC_ARM", str(ROOT / "target/fastbuild/lccc-arm"))
FIX = ROOT / "tests/regression/fixtures/arm_popcount"


def compile(args, good=True):
    p = subprocess.run([CC, *map(str, args)], capture_output=True, text=True, timeout=45)
    assert (p.returncode == 0 and not p.stderr.strip()) if good else (
        p.returncode > 0 and "-mgeneral-regs-only" in p.stderr), (args, p.returncode, p.stderr)
    return p


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true")
    args = parser.parse_args()
    if args.run and not shutil.which("qemu-aarch64"):
        parser.error("--run requires qemu-aarch64")
    execute = bool(args.run or shutil.which("qemu-aarch64"))
    runs = 0
    with tempfile.TemporaryDirectory(prefix="lccc-arm-popcount-") as td:
        w = Path(td)
        for level in ["-O0", "-O1", "-O2", "-O3", "-Os"]:
            for restricted in [False, True]:
                flags = [level, "-ffreestanding"] + (["-mgeneral-regs-only"] if restricted else [])
                assembly = w / "count.s"
                compile([*flags, "-S", FIX / "count.c", "-o", assembly])
                text = assembly.read_text()
                if restricted:
                    assert not re.search(r"\b[vqdsbh][0-9]+\b", text), text
                    assert "mul w0, w0, w1" in text and "mul x0, x0, x1" in text
                else:
                    assert "cnt v0.8b" in text, "default NEON path changed"
                compile([*flags, "-c", FIX / "count.c", "-o", w / "count.o"])
                compile([*flags, "-c", FIX / "check.c", "-o", w / "check.o"])
                compile(["-c", FIX / "start.s", "-o", w / "start.o"])
                compile(["-nostdlib", w / "start.o", w / "count.o", w / "check.o", "-o", w / "test"])
                if execute:
                    p = subprocess.run(["qemu-aarch64", str(w / "test")], capture_output=True, timeout=60)
                    assert p.returncode == 0 and not p.stderr, (level, restricted, p.returncode, p.stderr)
                    runs += 1
        # Do not silently emit forbidden code for other FP/SIMD paths.
        source = w / "float.c"
        source.write_text("double f(double x) { return x+1.0; }\n")
        compile(["-O2", "-mgeneral-regs-only", "-S", source, "-o", w / "float.s"], False)
        # Register-like C symbols are addresses, not SIMD uses.
        source.write_text("extern int v0; int *address(void) { return &v0; }\n")
        compile(["-O2", "-mgeneral-regs-only", "-S", source, "-o", w / "symbol.s"])
    print(f"PASS: AArch64 popcount O0/O1/O2/O3/Os, default/restricted; FP guard; {runs} QEMU runs (--run requires availability)")


if __name__ == "__main__":
    main()
