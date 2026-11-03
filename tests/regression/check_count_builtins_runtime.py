#!/usr/bin/env python3
"""Four-target execution gate for the integer count builtins.

`__builtin_clz`/`ctz` and their `l`/`ll` variants have no single-instruction
lowering on any backend this project ships: x86-64 (baseline) scans with
`bsr`/`bsf` and fixes the zero case up, i686 does the same in 32-bit form,
AArch64 has `clz`/`rbit` but still pays the constant subtract, and RISC-V has
only software (the base ISA has no bit scan, and the backend deliberately does
not require Zbb).  Every one of those paths is a hand-written instruction
sequence whose *values* -- not merely its shape -- have to be right, and the
defined-at-zero contract (`Clz(0)`/`Ctz(0)` == the operand width) is exactly
where a scan-based lowering goes wrong.

So the fixture does not pin instruction shapes: it links freestanding kernels
for all six builtins against in-image reference loops and compares the counts
over an exhaustive low half-word, every single-bit operand with its
neighbours and complements, and randomized sweeps.  The program runs natively
on x86-64 and under QEMU for i686, AArch64 and RISC-V, at every optimization
level, because a count lowering that is only checked at the level that
happens to hit one emitter arm is not checked.

Missing cross binaries or emulators are failures under ``--run``, not silent
skips (the local mirror invokes this without ``--run`` when no emulator is
installed, which compiles and links every program but executes none).
"""
import argparse
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile

if not __debug__:
    raise SystemExit("count-builtin runtime checks require Python assertions")

ROOT = Path(__file__).resolve().parents[2]
FIX = ROOT / "tests/regression/fixtures/count_builtins"
# The freestanding entry stubs are shared with the bit-width runtime gate; they
# only set up the psABI entry state and call `test()`.
STUBS = ROOT / "tests/regression/fixtures/bit_width"
SOURCES = ["count_builtins.c"]
LEVELS = ["-O0", "-O1", "-O2", "-O3", "-Os"]
# name: (compiler binary, entry stub, emulator; None = execute on the host)
TARGETS = {
    "x86_64": ("lccc", "start-x86_64.s", None),
    "i686": ("lccc-i686", "start-i686.s", "qemu-i386"),
    "arm": ("lccc-arm", "start-arm.s", "qemu-aarch64"),
    "riscv": ("lccc-riscv", "start-riscv.s", "qemu-riscv64"),
}


def run(cmd, env=None):
    p = subprocess.run(list(map(str, cmd)), capture_output=True, text=True,
                       timeout=300, env=env)
    assert p.returncode == 0, (cmd, p.returncode, p.stdout, p.stderr)
    assert not p.stderr.strip(), (cmd, p.stderr)
    return p


def host_runs_x86_64():
    return platform.system() == "Linux" and platform.machine() in ("x86_64", "AMD64")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true",
                        help="require execution; missing emulators are failures")
    parser.add_argument("--target", action="append", choices=sorted(TARGETS),
                        help="restrict to one target (repeatable; default: all)")
    args = parser.parse_args()
    targets = args.target or list(TARGETS)
    base_env = dict(os.environ)
    expected = runs = 0
    with tempfile.TemporaryDirectory(prefix="lccc-count-builtins-") as td:
        w = Path(td)
        for name in targets:
            binary, start, qemu = TARGETS[name]
            cc = ROOT / "target/fastbuild" / binary
            assert cc.is_file(), f"missing compiler: {cc}"
            if qemu is None:
                launcher = [] if host_runs_x86_64() else None
                if launcher is None and shutil.which("qemu-x86_64"):
                    launcher = [shutil.which("qemu-x86_64")]
            else:
                emulator = shutil.which(qemu)
                launcher = [emulator] if emulator else None
            if args.run:
                assert launcher is not None, (
                    f"cannot execute {name} binaries here (need {qemu or 'an x86-64 host'})")
            entry = w / f"start-{name}.o"
            run([cc, "-c", STUBS / start, "-o", entry], base_env)
            for level in LEVELS:
                tag = f"{name}{level}"
                objs = []
                for src in SOURCES:
                    obj = w / f"{tag}-{Path(src).stem}.o"
                    run([cc, level, "-ffreestanding", "-c", FIX / src, "-o", obj],
                        base_env)
                    objs.append(obj)
                exe = w / f"{tag}.elf"
                run([cc, "-nostdlib", "-static", entry, *objs, "-o", exe], base_env)
                expected += 1
                if launcher is None:
                    continue
                p = subprocess.run([*launcher, str(exe)], capture_output=True,
                                   text=True, timeout=300)
                assert p.returncode == 0 and not p.stdout.strip() \
                    and not p.stderr.strip(), (
                        "count builtin mismatch", name, level,
                        f"class mask 0x{p.returncode:x}",
                        p.stdout, p.stderr)
                runs += 1
    if args.run:
        assert runs == expected, (runs, expected)
    if runs:
        print(f"OK count_builtins_runtime ({runs} executions over "
              f"{', '.join(targets)})")
    else:
        print("SKIP count_builtins_runtime execution: no emulator "
              f"(compile/link only, {expected} programs)")


if __name__ == "__main__":
    main()
