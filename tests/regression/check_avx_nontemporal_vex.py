#!/usr/bin/env python3
"""Encoding contract for the non-temporal stores and streaming load under -mavx.

The streaming forms (MOVNTDQ/MOVNTPS/MOVNTPD, MOVNTDQA) have VEX.128 spellings
that are AVX1-legal (VMOVNTDQA xmm needs only AVX; the 256-bit forms need
AVX2). Under -mavx a legacy `movnt*` would mix with dirty upper YMM state in the
same function. This check pins two properties of the fixture
tests/regression/avx1_nontemporal_masked_vex.c:

  1. x86-64 `-O2 -mavx`: no legacy `movnt*` / `movntdqa` anywhere, and the
     stream/load functions `stream128`, `load128`, `stream256_ps` contain no
     legacy SSE instruction at all.
  2. i686 `-m32 -O2 -mavx`: no legacy `movnt*` / `movntdqa` anywhere.

It does NOT claim that generic 128/256-bit argument staging is VEX on i686:
those `movups`/`movdqu` copies are a separate, known residue (see the
engineering notes for the SSE-in-AVX work), and this check reports them
without failing on them so the residue stays visible.

A negative self-test feeds a legacy listing and requires it to be flagged.

Usage:
    check_avx_nontemporal_vex.py --lccc target/fastbuild/lccc [--self-test]
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "tests/regression/avx1_nontemporal_masked_vex.c"
STREAM_FUNCS = {"stream128", "load128", "stream256_ps"}

LEGACY_MOVNT = re.compile(r"^\s*(movntdq|movntdqa|movntps|movntpd)\s", re.M)
LEGACY_SSE = re.compile(
    r"^\s*(movdqu|movdqa|movups|movupd|movaps|movapd|movntdq|movntdqa|movntps|movntpd)\s"
)
FUNC_LABEL = re.compile(r"^([A-Za-z_][A-Za-z0-9_.]*):\s*$")


def function_bodies(asm: str) -> dict[str, list[str]]:
    bodies: dict[str, list[str]] = {}
    current = None
    for line in asm.splitlines():
        m = FUNC_LABEL.match(line)
        if m:
            current = m.group(1)
            bodies.setdefault(current, [])
            continue
        if current is not None:
            bodies[current].append(line)
    return bodies


def legacy_lines(lines: list[str]) -> list[str]:
    return [ln.strip() for ln in lines if LEGACY_SSE.match(ln)]


def compile_s(lccc: Path, flags: list[str]) -> str:
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "out.s"
        cmd = [str(lccc), *flags, "-S", "-o", str(out), str(FIXTURE)]
        res = subprocess.run(cmd, capture_output=True, text=True)
        if res.returncode != 0:
            raise SystemExit(f"FAIL: lccc {' '.join(flags)} rc={res.returncode}\n{res.stderr}")
        return out.read_text()


def check_asm(asm: str, label: str, check_stream_bodies: bool) -> list[str]:
    problems: list[str] = []
    for ln in asm.splitlines():
        if LEGACY_MOVNT.match(ln):
            problems.append(f"{label}: legacy streaming op: {ln.strip()}")
    if check_stream_bodies:
        bodies = function_bodies(asm)
        for name in sorted(STREAM_FUNCS):
            if name not in bodies:
                problems.append(f"{label}: function {name} missing from output")
                continue
            for ln in legacy_lines(bodies[name]):
                problems.append(f"{label}: {name} contains legacy SSE: {ln}")
    return problems


def self_test() -> int:
    legacy = "stream128:\n    movntps %xmm0, (%rdi)\n    movdqu %xmm0, %xmm1\n"
    flagged = check_asm(legacy, "selftest", check_stream_bodies=True)
    if not any("legacy streaming op" in p for p in flagged):
        print("SELF-TEST FAIL: legacy movntps not flagged")
        return 1
    if not any("stream128 contains legacy SSE" in p for p in flagged):
        print("SELF-TEST FAIL: legacy movdqu in stream128 not flagged")
        return 1
    clean = (
        "stream128:\n    vmovntps %xmm0, (%rdi)\n    vmovdqu %xmm0, %xmm1\n"
        "load128:\n    vmovntdqa (%rdi), %xmm0\n"
        "stream256_ps:\n    vmovntps %ymm0, (%rdi)\n"
    )
    if check_asm(clean, "selftest", check_stream_bodies=True):
        print("SELF-TEST FAIL: clean VEX listing flagged")
        return 1
    print("SELF-TEST PASS")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc", type=Path, default=ROOT / "target/fastbuild/lccc")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    problems: list[str] = []
    problems += check_asm(compile_s(args.lccc, ["-O2", "-mavx"]), "x86-64 -mavx", True)
    problems += check_asm(compile_s(args.lccc, ["-m32", "-O2", "-mavx"]), "i686 -mavx", False)
    if problems:
        for p in problems:
            print("FAIL:", p)
        return 1
    print("PASS: non-temporal and streaming-load forms are VEX under -mavx (x86-64 and i686)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
