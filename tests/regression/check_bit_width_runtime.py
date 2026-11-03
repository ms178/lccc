#!/usr/bin/env python3
"""Mandatory four-target execution gate for single-bit selection and testing,
boolean returns and vector lane conversions.

Unlike the assembly-count gate, this links freestanding objects with LCCC's own
assembler/linker and executes them: natively for x86-64, under QEMU for i686,
AArch64 and RISC-V.  Missing cross binaries or emulators are failures under
``--run``, not silent skips: the point is to certify every backend that the
bit folds reach, including the x86 flag fusions and the i686 register paths.

Every program is pinned to a checksum GCC 16.2 and Clang 23.1.4 agree on:

* ``check.c`` (one TU over ``runtime.c``): signed narrow results observed
  before any truncating return can mask a canonical-form bug.
* the browser bit-index kernels (``browser_bits.c``, transcribed from the
  pinned Chromium and Firefox trees) with ``browser_driver.c``, built twice:
  as separate objects (every kernel opaque, its real calling convention) and
  as one TU (``browser_combined.c``: inlined into the driver's loops).
* ``bool_return.c`` with ``bool_return_check.c``: opaque kernels of the
  ``if (c) return 1; return 0;`` shape and its inversion over every compare
  predicate (float NaN rows included), bit tests, zero tests and every return
  width, plus the narrow-extension-then-``long`` chains the x86 peephole
  collapses.
* ``convertvector.c`` with ``convertvector_check.c``: ``__builtin_convertvector``
  over every lane-type pair (signed, unsigned, 8/16/32/64-bit integer, float,
  double) at 8-, 16- and 32-byte widths, including the x86-64 packed pair and
  the per-lane conversions every other pair and target take.

Every program runs at every optimization level with the folds on, with
``CCC_NO_BIT_SELECT`` (the IR folds off), and on the x86 targets also with
every bit-test fusion switched off, so the fallback lowering each fusion
replaces is executed too.
"""
import argparse
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile

if not __debug__:
    raise SystemExit("bit-width runtime checks require Python assertions")

ROOT = Path(__file__).resolve().parents[2]
FIX = ROOT / "tests/regression/fixtures/bit_width"
LEVELS = ["-O0", "-O1", "-O2", "-O3", "-Os"]
# name: (compiler binary, entry stub, emulator; None = execute on the host)
TARGETS = {
    "x86_64": ("lccc", "start-x86_64.s", None),
    "i686": ("lccc-i686", "start-i686.s", "qemu-i386"),
    "arm": ("lccc-arm", "start-arm.s", "qemu-aarch64"),
    "riscv": ("lccc-riscv", "start-riscv.s", "qemu-riscv64"),
}
# name: translation units, each compiled to its own object
PROGRAMS = {
    "bit_width": ["check.c"],
    "browser": ["browser_bits.c", "browser_driver.c", "browser_check.c"],
    "browser_combined": ["browser_combined.c"],
    "bool_return": ["bool_return.c", "bool_return_check.c"],
    "convertvector": ["convertvector.c", "convertvector_check.c"],
}
BIT_TEST_FUSIONS_OFF = {
    "CCC_NO_BT_FLAG_FUSION": "1",
    "CCC_NO_BT_MODIFY_FUSION": "1",
    "CCC_NO_BT_BRANCH_FUSION": "1",
    "CCC_DISABLE_PASSES": "bt_modify_pairs",
}
# name: (environment overrides, targets the switch changes code on)
CONFIGS = {
    "default": ({}, set(TARGETS)),
    "no-bit-select": ({"CCC_NO_BIT_SELECT": "1"}, set(TARGETS)),
    "no-bt-fusion": (BIT_TEST_FUSIONS_OFF, {"x86_64", "i686"}),
}
KNOBS = {"CCC_NO_BIT_SELECT", *BIT_TEST_FUSIONS_OFF}


def run(cmd, env=None):
    p = subprocess.run(list(map(str, cmd)), capture_output=True, text=True,
                       timeout=180, env=env)
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
    base_env = {k: v for k, v in os.environ.items() if k not in KNOBS}
    expected = runs = 0
    with tempfile.TemporaryDirectory(prefix="lccc-bit-width-") as td:
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
                assert launcher is not None, f"cannot execute {name} binaries here (need {qemu or 'an x86-64 host'})"
            entry = w / f"start-{name}.o"
            run([cc, "-c", FIX / start, "-o", entry], base_env)
            for config, (overrides, applies) in CONFIGS.items():
                if name not in applies:
                    continue
                env = dict(base_env, **overrides)
                for level in LEVELS:
                    for program, sources in PROGRAMS.items():
                        tag = f"{name}{level}-{config}-{program}"
                        objs = []
                        for src in sources:
                            obj = w / f"{tag}-{Path(src).stem}.o"
                            run([cc, level, "-ffreestanding", "-c", FIX / src, "-o", obj], env)
                            objs.append(obj)
                        exe = w / f"{tag}.elf"
                        run([cc, "-nostdlib", "-static", entry, *objs, "-o", exe], base_env)
                        expected += 1
                        if launcher is None:
                            continue
                        p = subprocess.run([*launcher, str(exe)], capture_output=True,
                                           text=True, timeout=120)
                        assert p.returncode == 0 and not p.stderr.strip(), (
                            name, level, config, program, p.returncode, p.stdout, p.stderr)
                        runs += 1
    if args.run:
        assert runs == expected, (runs, expected)
    if runs:
        print(f"OK bit_width_runtime ({runs} executions over {', '.join(targets)})")
    else:
        print("SKIP bit_width_runtime execution: no emulator (compile/link only, "
              f"{expected} programs)")


if __name__ == "__main__":
    main()
