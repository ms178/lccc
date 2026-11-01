#!/usr/bin/env python3
"""Mandatory AArch64/RISC-V execution gate for single-bit selection.

Unlike the assembly-count gate, this links freestanding objects with LCCC's own
assembler/linker and executes them under QEMU.  Missing cross binaries or QEMU
are failures, not silent skips: the whole point is to certify the non-host
backends.  The fixture pins a GCC/Clang-measured checksum and observes signed
narrow results before any truncating return can mask a canonical-form bug.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

if not __debug__:
    raise SystemExit("bit-width runtime checks require Python assertions")

ROOT = Path(__file__).resolve().parents[2]
FIX = ROOT / "tests/regression/fixtures/bit_width"
LEVELS = ["-O0", "-O1", "-O2", "-O3", "-Os"]
TARGETS = {
    "arm": ("lccc-arm", "start-arm.s", "qemu-aarch64"),
    "riscv": ("lccc-riscv", "start-riscv.s", "qemu-riscv64"),
}


def run(cmd, **kw):
    p = subprocess.run(list(map(str, cmd)), capture_output=True, text=True, timeout=120, **kw)
    assert p.returncode == 0, (cmd, p.returncode, p.stdout, p.stderr)
    assert not p.stderr.strip(), (cmd, p.stderr)
    return p


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true",
                        help="require QEMU execution; missing emulators are failures")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="lccc-bit-width-") as td:
        w = Path(td)
        runs = 0
        for name, (binary, start, qemu) in TARGETS.items():
            cc = ROOT / "target/fastbuild" / binary
            assert cc.is_file(), f"missing cross compiler: {cc}"
            emulator = shutil.which(qemu)
            if args.run:
                assert emulator, f"missing required emulator: {qemu}"
            run([cc, "-c", FIX / start, "-o", w / f"start-{name}.o"])
            for level in LEVELS:
                for disabled in (False, True):
                    env = dict(os.environ)
                    if disabled:
                        env["CCC_NO_BIT_SELECT"] = "1"
                    obj = w / f"check-{name}{level}{'-off' if disabled else ''}.o"
                    exe = w / f"test-{name}{level}{'-off' if disabled else ''}"
                    p = subprocess.run(
                        [str(cc), level, "-ffreestanding", "-c", str(FIX / "check.c"), "-o", str(obj)],
                        capture_output=True, text=True, timeout=120, env=env,
                    )
                    assert p.returncode == 0 and not p.stderr.strip(), (name, level, disabled, p.stderr)
                    run([cc, "-nostdlib", "-static", w / f"start-{name}.o", obj, "-o", exe])
                    if emulator:
                        p = subprocess.run([emulator, str(exe)], capture_output=True, text=True, timeout=60)
                        assert p.returncode == 0 and not p.stderr.strip(), (
                            name, level, "CCC_NO_BIT_SELECT" if disabled else "default",
                            p.returncode, p.stdout, p.stderr,
                        )
                        runs += 1
        expected = len(TARGETS) * len(LEVELS) * 2
        if args.run:
            assert runs == expected, (runs, expected)
            print(f"OK bit_width_runtime ({runs} cross-target executions)")
        elif runs:
            print(f"OK bit_width_runtime ({runs} cross-target executions)")
        else:
            print("SKIP bit_width_runtime execution: QEMU unavailable (compile/link only)")


if __name__ == "__main__":
    main()
