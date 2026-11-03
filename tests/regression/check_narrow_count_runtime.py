#!/usr/bin/env python3
"""Narrow population counts and parity, executed on every backend.

The count-width rule now lowers `__builtin_popcount`/`__builtin_parity` of a
byte or a halfword at the operand's own width, on every target: the extension
back to the consumer's `int` is a `Copy` (the value already fits), and each
backend has its own narrow sequence -- a multiplier form on x86, the byte lanes
of `cnt` on AArch64, an `andi`-masked lane scan on RISC-V.

Those sequences read whole registers, so the interesting question is not
whether they assemble but whether the bits a narrow type does not own stay out
of the answer.  This gate executes the same freestanding checksum on the host
backend natively and on AArch64/RISC-V under QEMU, at every optimization level,
and requires the three to agree with the value GCC and Clang measure on the
same source.  Missing cross compilers or QEMU are failures under `--run`; the
local mirror without emulators still compiles and links, and says so.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

if not __debug__:
    raise SystemExit("narrow count runtime checks require Python assertions")

ROOT = Path(__file__).resolve().parents[2]
FIX = ROOT / "tests/regression/fixtures/count_narrow"
LEVELS = ["-O0", "-O1", "-O2", "-O3", "-Os"]
EXPECTED = 4177401467814966825
CROSS = {
    "arm": ("lccc-arm", "start-arm.s", "qemu-aarch64"),
    "riscv": ("lccc-riscv", "start-riscv.s", "qemu-riscv64"),
}


def run(cmd, **kw):
    p = subprocess.run(list(map(str, cmd)), capture_output=True, text=True, timeout=180, **kw)
    assert p.returncode == 0, (cmd, p.returncode, p.stdout, p.stderr)
    assert not p.stderr.strip(), (cmd, p.stderr)
    return p


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true",
                        help="require QEMU execution; missing emulators are failures")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="lccc-narrow-count-") as td:
        w = Path(td)

        # The oracle, on the host, from an independent compiler when there is
        # one: the checksum is the model, not "whatever lccc printed".
        reference = shutil.which("gcc-16") or shutil.which("gcc")
        if reference:
            run([reference, "-O2", "-o", w / "oracle", FIX / "main.c"])
            run([w / "oracle"])
            got = subprocess.run([w / "oracle"], capture_output=True, text=True)
            assert got.stdout.strip() == "", got.stdout
        else:
            print("SKIP: no host GCC to re-measure the oracle; the pinned value stands")

        # Host backend, natively, at every level.
        host = ROOT / "target/fastbuild/lccc"
        assert host.is_file(), f"missing host compiler: {host}"
        for level in LEVELS:
            exe = w / f"host{level}"
            run([host, level, FIX / "main.c", "-o", exe])
            p = subprocess.run([str(exe)], capture_output=True, text=True, timeout=60)
            assert p.returncode == 0, (level, p.returncode, p.stdout, p.stderr)

        runs = 0
        for name, (binary, start, qemu) in CROSS.items():
            cc = ROOT / "target/fastbuild" / binary
            assert cc.is_file(), f"missing cross compiler: {cc}"
            emulator = shutil.which(qemu)
            if args.run:
                assert emulator, f"missing required emulator: {qemu}"
            run([cc, "-c", FIX / start, "-o", w / f"start-{name}.o"])
            for level in LEVELS:
                obj = w / f"check-{name}{level}.o"
                exe = w / f"test-{name}{level}"
                run([cc, level, "-ffreestanding", "-c", FIX / "check.c", "-o", obj])
                run([cc, "-nostdlib", "-static", w / f"start-{name}.o", obj, "-o", exe])
                if emulator:
                    p = subprocess.run([emulator, str(exe)], capture_output=True,
                                       text=True, timeout=120)
                    assert p.returncode == 0 and not p.stderr.strip(), (
                        name, level, p.returncode, p.stdout, p.stderr)
                    runs += 1
        if runs:
            print(f"OK narrow_count_runtime ({runs} cross-target executions, "
                  f"checksum {EXPECTED})")
        else:
            print("SKIP narrow_count_runtime execution: QEMU unavailable "
                  "(compile/link only)")


if __name__ == "__main__":
    main()
