#!/usr/bin/env python3
"""Fail if generated code mixes VEX-256 (ymm) with legacy SSE in one function.

Writing a ymm register leaves its upper 128 bits live. Any legacy-SSE
instruction executed afterwards -- including ordinary scalar FP such as
movsd/mulsd/addsd -- then costs an AVX->SSE state transition of roughly 70
cycles on Intel, unless a `vzeroupper` has retired the upper state in between.

This is invisible to instruction-count metrics: the offending form is one
instruction SHORTER. It cost a 46x slowdown in tests/benchmark/programs/
struct_copy.c, where a 48-byte struct copy used `vmovdqu %ymm0` and the scalar
physics next to it used movsd/mulsd/addsd (5287 ms versus 113 ms after the fix,
gcc -O2: 21 ms).

Usage:
    scripts/check_avx_sse_transitions.py [--lccc PATH] [--flags "..."] FILE.c...

Exit status is non-zero if any function contains a ymm write that is later
followed by a legacy-SSE instruction with no intervening vzeroupper.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

# Legacy (non-VEX) SSE instructions that read/write xmm registers. A VEX-encoded
# form of the same mnemonic starts with 'v' and is therefore excluded by the
# leading word boundary in the pattern below.
LEGACY_SSE = re.compile(
    r"^\s*(?:"
    r"mov(?:ap[sd]|up[sd]|dq[au]|s[sd]|d|q|nt(?:dq|p[sd]|i))|"
    r"(?:add|sub|mul|div|min|max|sqrt|cmp|ucomi|comi)(?:p[sd]|s[sd])|"
    r"(?:and|andn|or|xor)p[sd]|"
    r"p(?:add|sub|mul|and|or|xor|cmp|shuf|unpck|ack|max|min|avg|sad|movmsk|"
    r"insr|extr|blend|test|slld|srld|sllq|srlq)[a-z]*|"
    r"cvt(?:si2s[sd]|s[sd]2si|tp[sd]2dq|dq2p[sd]|p[sd]2p[sd]|s[sd]2s[sd])|"
    r"unpck[lh]p[sd]|shufp[sd]|blendv?p[sd]|roundp?[sd]|dpp[sd]"
    r")\b",
    re.IGNORECASE,
)
YMM_WRITE = re.compile(r"^\s*v[a-z0-9]+\s+.*%ymm\d+", re.IGNORECASE)
# `movq %rax, %rbx` and `movd %eax, -8(%rbp)` share a mnemonic with the SSE
# forms but touch no xmm register; the caller therefore also requires "%xmm"
# to appear on the line before reporting a transition.
VZEROUPPER = re.compile(r"^\s*vzero(?:upper|all)\b", re.IGNORECASE)
LABEL = re.compile(r"^([A-Za-z_][\w.$]*):")


def scan(asm: str) -> list[tuple[str, int, str, int, str]]:
    """Return (func, ymm_line_no, ymm_text, sse_line_no, sse_text) violations."""
    out: list[tuple[str, int, str, int, str]] = []
    func = "?"
    dirty: tuple[int, str] | None = None
    for n, line in enumerate(asm.splitlines(), 1):
        stripped = line.strip()
        if not stripped or stripped.startswith(("#", "//")):
            continue
        m = LABEL.match(line)
        if m and not m.group(1).startswith(".L"):
            func, dirty = m.group(1), None
            continue
        if stripped.startswith(".") and not stripped.startswith(".L"):
            continue
        if VZEROUPPER.match(line):
            dirty = None
            continue
        if YMM_WRITE.match(line):
            dirty = (n, stripped)
            continue
        if dirty and LEGACY_SSE.match(line) and "%xmm" in stripped:
            out.append((func, dirty[0], dirty[1], n, stripped))
            dirty = None  # one report per ymm write keeps the output readable
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("sources", nargs="+", type=Path)
    ap.add_argument("--lccc", default="./target/release/lccc")
    ap.add_argument("--flags", default="-O2 -march=x86-64-v3")
    args = ap.parse_args()

    total = 0
    for src in args.sources:
        with tempfile.NamedTemporaryFile(suffix=".s", delete=False) as tmp:
            out = Path(tmp.name)
        try:
            r = subprocess.run(
                [args.lccc, *args.flags.split(), "-S", str(src), "-o", str(out)],
                capture_output=True, text=True)
            if r.returncode != 0:
                print(f"  ! {src}: compile failed: "
                      f"{(r.stderr or r.stdout).strip().splitlines()[-1:]}")
                continue
            bad = scan(out.read_text())
        finally:
            out.unlink(missing_ok=True)
        for func, yn, yt, sn, st in bad:
            print(f"AVX->SSE transition in {src.name}:{func}\n"
                  f"    line {yn}: {yt}\n"
                  f"    line {sn}: {st}   <- legacy SSE, upper ymm still dirty")
        total += len(bad)

    if total:
        print(f"\n=== avx-sse: {total} transition(s) found ===")
        return 1
    print(f"=== avx-sse: clean over {len(args.sources)} file(s) ===")
    return 0


if __name__ == "__main__":
    sys.exit(main())
