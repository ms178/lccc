#!/usr/bin/env python3
"""Gate: a vector must never be reloaded from an over-aligned frame address
that was just stored, while the stored register is still live.

The defect (engineering/SIMD-IMPLEMENTATION-AUDIT-2026-11-08.md, 9c.10.12):
`emit_alloca_addr_to` materialises an over-aligned home as
`leaq K(%rbp), %r; andq $-A, %r` at EVERY use, so a vector copy-out/copy-in
emits

    leaq -193(%rbp), %rax
    andq $-32, %rax
    vmovdqu %ymm0, (%rax)      # spill
    leaq -193(%rbp), %rax      # same address, re-derived
    andq $-32, %rax
    vmovups (%rax), %ymm3      # reload -> store-forwarding stall

`aligned_vector_reload::forward_aligned_vector_reload` turns the reload into
`vmovups %ymm0, %ymm3`.  Measured on an AVX-512 host: -40.0% from the
forwarding alone, -43.6% once the existing copy-propagation and dead-store
passes retire the orphaned spill.

This gate asserts the STRUCTURAL invariant, not a cycle count, so it is
stable across machines and register allocation: inside one function, no
vector load may read a bare `(%r)` that a preceding vector store wrote, when
the only intervening instructions re-derive the same `%rbp`-relative address
and the stored register is not rewritten.  A load NO WIDER than the store
counts (the low-half case); a wider load does not, because it reads bytes the
store never wrote.

Non-vacuity is part of the contract: run with `--expect-defect` against a
pre-fix compiler and the gate must FAIL.  A gate that cannot fail is
decoration.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REPRO = ROOT / "tests/simd/vector_home_call_loop.c"
DIFF = ROOT / "tests/simd/aligned_vector_reload_diff.c"

# Full-width, unmasked moves only.  Partial/merging (movsd/movss) and
# non-temporal forms have different semantics and are not the defect.
MOVES = (
    "movaps",
    "movups",
    "movapd",
    "movupd",
    "movdqa",
    "movdqu",
    "vmovaps",
    "vmovups",
    "vmovapd",
    "vmovupd",
    "vmovdqa",
    "vmovdqu",
    "vmovdqa32",
    "vmovdqa64",
    "vmovdqu32",
    "vmovdqu64",
)
_MOVE_ALT = "|".join(sorted(MOVES, key=len, reverse=True))
VEC = r"%(?:xmm|ymm|zmm)\d+"
STORE = re.compile(rf"^({_MOVE_ALT})\s+({VEC}),\s+\((%[a-z0-9]+)\)$")
LOAD = re.compile(rf"^({_MOVE_ALT})\s+\((%[a-z0-9]+)\),\s+({VEC})$")
LEA = re.compile(r"^leaq\s+(-?\d*)\(%rbp\),\s+(%[a-z0-9]+)$")
AND = re.compile(r"^andq\s+\$(-\d+),\s+(%[a-z0-9]+)$")
LABEL = re.compile(r"^[.\w$]+:")


def width(reg: str) -> int:
    return 16 if reg.startswith("%xmm") else 32 if reg.startswith("%ymm") else 64


def index(reg: str) -> int:
    return int(reg.lstrip("%")[3:])


def find_defects(asm: str) -> list[str]:
    """Return one message per surviving store->reload round-trip."""
    problems: list[str] = []
    # Canonical address per GP register, exactly as the compiler pass tracks
    # it.  Anything unrecognised clears the state, so the gate can never
    # invent a round-trip across a call or a branch.
    addr: dict[str, tuple[int, int]] = {}
    pending: tuple[tuple[int, int], str, bool] | None = None
    func = "?"
    for raw in asm.splitlines():
        line = raw.strip()
        if line.endswith(":"):
            func = line[:-1]
            addr.clear()
            pending = None
            continue
        if not line or line.startswith("#") or line.startswith("."):
            continue
        if m := LEA.match(line):
            disp = int(m.group(1)) if m.group(1) else 0
            reg = m.group(2)
            if reg not in ("%rsp", "%rbp"):
                addr[reg] = (disp, 1)
            else:
                addr.clear()
                pending = None
            continue
        if m := AND.match(line):
            reg = m.group(2)
            mask = int(m.group(1))
            prev = addr.get(reg)
            if prev is not None and prev[1] == 1 and mask < 0 and reg not in ("%rsp", "%rbp"):
                addr[reg] = (prev[0], -mask)
            else:
                addr.pop(reg, None)
            continue
        if m := STORE.match(line):
            mnem, src, base = m.group(1), m.group(2), m.group(3)
            key = addr.get(base)
            pending = None if key is None else (key, src, mnem.startswith("v"))
            continue
        if m := LOAD.match(line):
            mnem, base, dst = m.group(1), m.group(2), m.group(3)
            key = addr.get(base)
            if pending is not None:
                pk, src, vex = pending
                if (
                    key == pk
                    and width(dst) <= width(src)
                    and vex == mnem.startswith("v")
                ):
                    problems.append(
                        f"{func}: {mnem} ({base}), {dst} reloads bytes still "
                        f"live in {src} after a {width(src)}-byte store to the "
                        f"same %rbp-relative address {pk}. It must be a "
                        f"register copy."
                    )
                pending = None if index(dst) == index(src) else pending
            continue
        # Calls, branches, arithmetic, unknown stores: state dies.
        addr.clear()
        pending = None
    return problems


def compile_asm(cc: str, source: Path, isa: list[str], out: Path) -> tuple[int, str]:
    cmd = [cc, "-O2", *isa, "-S", str(source), "-o", str(out)]
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
    except FileNotFoundError:
        return 127, f"compiler not found: {cc}"
    except subprocess.TimeoutExpired:
        return 124, f"compile timed out: {' '.join(cmd)}"
    return p.returncode, p.stderr


ISA_SETS = (
    ("avx2", ["-mavx2", "-mfma"]),
    ("avx512", ["-mavx512f", "-mavx512bw", "-mavx512vl", "-mfma"]),
)


def run(cc: str, expect_defect: bool) -> int:
    for src in (REPRO, DIFF):
        if not src.is_file():
            print(f"ERROR: missing {src}", file=sys.stderr)
            return 2
    total = 0
    checked = 0
    with tempfile.TemporaryDirectory(prefix="avr-gate-") as td:
        for source, oracles in ((REPRO, ("f1:", "vfmadd")), (DIFF, ("main:", "imul"))):
            for tag, isa in ISA_SETS:
                out = Path(td) / f"{source.stem}-{tag}.s"
                rc, err = compile_asm(cc, source, isa, out)
                if rc != 0:
                    print(
                        f"ERROR: {source.name} {tag}: compile rc={rc}: "
                        f"{err.strip()[:400]}",
                        file=sys.stderr,
                    )
                    return 2
                asm = out.read_text()
                # Oracle: prove the kernel was actually emitted before
                # asserting anything about its instructions, so an empty or
                # failed compile can never masquerade as a pass.
                if not all(o in asm for o in oracles):
                    print(
                        f"ERROR: {source.name} {tag}: kernel not emitted "
                        f"(missing one of {oracles})",
                        file=sys.stderr,
                    )
                    return 2
                problems = find_defects(asm)
                checked += 1
                total += len(problems)
                for p in problems:
                    print(f"  - {source.name}/{tag}: {p}")
    if expect_defect:
        if total == 0:
            print(
                "check_aligned_vector_reload: VACUOUS (the gate found no "
                "defect in a compiler that is supposed to have it)"
            )
            return 1
        print(
            f"check_aligned_vector_reload: NON-VACUOUS "
            f"({total} round-trips detected in {checked} configurations)"
        )
        return 0
    if total:
        print(f"check_aligned_vector_reload: FAIL ({total} surviving round-trips)")
        return 1
    print(
        f"check_aligned_vector_reload: PASS "
        f"({checked} configurations, no store->reload round-trip)"
    )
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    ap.add_argument(
        "--expect-defect",
        action="store_true",
        help="invert the verdict: require the defect (non-vacuity proof)",
    )
    args = ap.parse_args()
    return run(args.lccc, args.expect_defect)


if __name__ == "__main__":
    sys.exit(main())
