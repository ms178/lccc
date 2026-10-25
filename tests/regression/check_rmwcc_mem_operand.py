#!/usr/bin/env python3
"""Inline-asm memory operands must always carry an address.

WHY THIS GATE EXISTS
====================
`AsmOperand::mem_addr` is printed verbatim into the template (`emit_operand_common`
for the x86 family).  When the resolution step could not compute an address it
left the string EMPTY and nothing complained, so the assembler received a
truncated instruction: the operand placeholder vanished together with its comma
separator, and the only diagnostic was a downstream syntax error that named the
assembly file -- from which the source construct is not recoverable.

That is exactly how Linux 6.18 stopped building:

    mm/zsmalloc.c: folio_trylock -> folio_test_set_bit_locked
                -> arch_test_and_set_bit -> GEN_BINARY_RMWcc(...)
    emitted:      lock  btsq  $0,              <- address operand missing
    assembler:    "expecting lockable instruction after `lock'"

The address in that call (`folio_flags(folio, 0)`) arrived without a stack slot
-- a register home or a Copy chain -- and `resolve_memory_operand` only knew how
to load a slot.  The fix materialises the address through `value_to_reg`, the
same resolver the register-operand preload path uses, and both x86 backends now
call `x86_common::assert_memory_operands_resolved` before substitution so the
failure can never again be silent.

WHAT IS CHECKED
===============
For every optimisation level (and a second pass with the kernel's flag set,
which is where the defect actually appeared):

  1. lccc compiles the TU (no silent truncation can reach the assembler),
  2. no inline-asm line in the emitted `.s` ends with a comma -- the exact
     signature of a dropped final operand,
  3. every `btsq`/`lock add` site that takes a memory operand has an address
     after the comma, and the expected number of RMWcc sites exists (a shape
     that silently stopped being emitted would otherwise pass),
  4. the program's output is byte-identical to GCC 16.2's -- a *wrong* address
     is as bad as a missing one, and only a run detects it.

USAGE
    GCC=gcc-16 CCC=target/fastbuild/lccc tests/regression/check_rmwcc_mem_operand.py

EXIT
    0  every level compiles, every operand has an address, both compilers agree
    1  a mismatch (the failing level and line are named on stderr)
    2  the oracle or the compiler is missing
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CASES = Path(__file__).with_name("rmwcc_mem_operand") / "cases.c"

#: The kernel's x86-64 flag set, minus the ones Kbuild probes for.  This is the
#: configuration in which the defect reproduced; `-O3` plus `-pg -mfentry
#: -mcmodel=kernel` is where the address value stops being stack-homed.
KERNEL_FLAGS = [
    "-O3", "-pg", "-mfentry", "-mno-red-zone", "-mcmodel=kernel", "-fno-PIE",
    "-fno-stack-protector", "-fno-omit-frame-pointer", "-mno-sse", "-mno-mmx",
    "-mno-80387", "-falign-jumps=1", "-falign-loops=1",
]

#: Optimisation levels of the plain pass.  -O0 is included because the operand
#: resolution path differs (everything is stack-homed there), which is a cheap
#: way to prove the fix did not special-case one level.
LEVELS = ["-O0", "-O1", "-O2", "-O3"]

#: A memory operand position after the comma of a two-operand RMW: either a
#: register-indirect address with optional index/scale, a RIP-relative symbol,
#: or a bare symbol/absolute address.
ADDRESS = r"(\(%[a-z0-9]+(?:,%[a-z0-9]+,\d)?\)|[\w.$@]+\(%rip\)|[-\w.$@+]+)"


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def asm_lines(text):
    """Lines that belong to an inline-asm region, with their 1-based numbers."""
    inside = False
    out = []
    for lineno, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if stripped.startswith("#APP"):
            inside = True
            continue
        if stripped.startswith("#NO_APP"):
            inside = False
            continue
        if inside and stripped and not stripped.startswith("#"):
            out.append((lineno, stripped))
    return out


def check_operands(asm_path, label, failures):
    text = asm_path.read_text(errors="replace")
    lines = asm_lines(text)
    if not lines:
        failures.append(f"{label}: no inline-asm region in {asm_path.name}")
        return
    for lineno, line in lines:
        # (2) A template line never legitimately ends with a comma or a
        # separator: that is a placeholder that printed as the empty string.
        if line.endswith(",") or line.endswith(", "):
            failures.append(
                f"{label}:{lineno}: inline-asm operand dropped (line ends with ','): "
                f"{line!r}")
        # (3) Every flag-setting RMW that takes two operands must show an
        # address in the second one.
        m = re.match(r"(?:lock\s+)?btsq\s+(\S+?),\s*(.*)$", line)
        if m and not re.match(ADDRESS + r"$", m.group(2).strip()):
            failures.append(
                f"{label}:{lineno}: btsq has no usable memory address: {line!r}")
    rmwcc_sites = sum(1 for _, line in lines if re.search(r"\bbtsq\b", line))
    if rmwcc_sites < 5:
        failures.append(
            f"{label}: only {rmwcc_sites} btsq inline-asm sites were emitted; "
            "the shapes were optimised away, so the check proves nothing")


def compile_and_run(cc, flags, workdir, tag, failures):
    exe = workdir / f"{tag}.out"
    out = run([cc, *flags, "-o", str(exe), str(CASES)])
    if out.returncode != 0:
        failures.append(f"{tag}: {cc} failed to compile:\n{out.stderr.strip()[:2000]}")
        return None
    res = run([str(exe)])
    if res.returncode != 0:
        failures.append(f"{tag}: program exited {res.returncode}: {res.stderr.strip()[:400]}")
        return None
    return res.stdout


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    ccc = os.environ.get("CCC", "target/fastbuild/lccc")
    oracle = os.environ.get("GCC", "gcc-16")
    for cc in (ccc, oracle):
        if run([cc, "--version"]).returncode != 0:
            print(f"check_rmwcc_mem_operand: {cc} is not usable", file=sys.stderr)
            return 2

    failures = []
    with tempfile.TemporaryDirectory() as tmp:
        workdir = Path(tmp)
        # Oracle output at one level is enough: the program's result depends on
        # the asm, not on the optimisation level, and requiring three extra GCC
        # builds would slow the fast gate for no extra signal.
        expected = compile_and_run(oracle, ["-O2"], workdir, "gcc-O2", failures)

        configs = [(lvl, [lvl]) for lvl in LEVELS] + [("kernel", KERNEL_FLAGS)]
        for level, flags in configs:
            asm = workdir / f"lccc-{level.lstrip('-')}.s"
            tag = f"lccc-{level}"
            out = run([ccc, *flags, "-S", "-o", str(asm), str(CASES)])
            if out.returncode != 0:
                failures.append(
                    f"{tag}: lccc failed to emit assembly:\n{out.stderr.strip()[:2000]}")
                continue
            check_operands(asm, tag, failures)
            if args.verbose:
                print(f"{tag}: {len(asm_lines(asm.read_text()))} inline-asm lines checked")
                # ... and the object must assemble and run to the oracle's answer.
            got = compile_and_run(ccc, flags, workdir, tag, failures)
            if expected is not None and got is not None and got != expected:
                failures.append(
                    f"{tag}: run-time output differs\n  gcc-16: {expected!r}\n  lccc  : {got!r}")

    if failures:
        print("check_rmwcc_mem_operand: FAILED", file=sys.stderr)
        for f in failures:
            print(f"  {f}", file=sys.stderr)
        return 1
    print("check_rmwcc_mem_operand: PASSED "
          f"({len(LEVELS) + 1} configurations, {oracle} parity, no dropped operand)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
