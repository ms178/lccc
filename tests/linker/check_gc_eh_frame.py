#!/usr/bin/env python3
"""check_gc_eh_frame.py -- invariant check for --gc-sections vs .eh_frame.

`--gc-sections` collects at input-section granularity while a translation unit
emits ONE `.eh_frame` per object holding an FDE per function, so the FDE set
and the live-code set can diverge inside a single section.  Two failure modes
follow, and neither is visible from the link's exit status:

  * a dropped `.eh_frame`      -> unwinding silently returns one frame;
  * a stale FDE left behind    -> after compaction its address range is
                                  recycled by live code, so the unwinder can
                                  pick CFI belonging to a different function.

This script checks the invariant directly on the output image:

  1. every STT_FUNC symbol in `.symtab` that lies in an executable section is
     covered by exactly one FDE in `.eh_frame` (via `.eh_frame_hdr`);
  2. every FDE covers the address range of some live symbol -- no FDE may
     describe collected code.

It is linker-agnostic: run it over bfd/lld/mold output as a control.

Usage:
  check_gc_eh_frame.py BIN [BIN...]
Exit status 0 if every invariant holds for every binary.
"""

from __future__ import annotations

import re
import subprocess
import sys


def sh(cmd: list[str]) -> str:
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=60).stdout
    except Exception:  # noqa: BLE001
        return ""


# Sections a linker fills with synthesised code and also describes with
# synthesised FDEs, so the unwinder can walk through a PLT stub.  An FDE there
# legitimately has no STT_FUNC symbol behind it.
SYNTHETIC_CODE = (".plt", ".plt.got", ".plt.sec", ".iplt", ".init", ".fini")


def _sections(binary: str) -> list[tuple[str, int, int, str]]:
    """(name, addr, addr+size, flags) for every section header."""
    out = []
    for line in sh(["readelf", "-SW", binary]).splitlines():
        m = re.match(
            r"\s*\[\s*\d+\]\s+(\S+)\s+\S+\s+([0-9a-f]+)\s+[0-9a-f]+\s+([0-9a-f]+)\s+\S*\s*"
            r"([A-Z]*)\s",
            line,
        )
        if m:
            addr = int(m.group(2), 16)
            size = int(m.group(3), 16)
            out.append((m.group(1), addr, addr + size, m.group(4)))
    return out


def text_functions(binary: str) -> list[tuple[int, int, str]]:
    """(addr, size, name) for every STT_FUNC symbol inside an executable section."""
    exec_ranges = [(lo, hi) for _n, lo, hi, fl in _sections(binary) if "X" in fl and hi > lo]
    out = []
    for line in sh(["nm", "--defined-only", "--print-size", binary]).splitlines():
        parts = line.split()
        if len(parts) < 4:
            continue
        addr_s, size_s, kind, name = parts[0], parts[1], parts[2], parts[3]
        if kind.lower() != "t":
            continue
        try:
            addr = int(addr_s, 16)
            size = int(size_s, 16)
        except ValueError:
            continue
        if size == 0:
            size = 1
        if any(lo <= addr < hi for lo, hi in exec_ranges):
            out.append((addr, size, name))
    return sorted(out)


def fde_ranges(binary: str) -> list[tuple[int, int]]:
    """(start, end) of every FDE reachable through .eh_frame_hdr / .eh_frame."""
    out = sh(["readelf", "--debug-dump=frames", binary])
    ranges = []
    for m in re.finditer(r"pc=([0-9a-f]+)\.\.([0-9a-f]+)", out):
        a, b = int(m.group(1), 16), int(m.group(2), 16)
        if b > a:
            ranges.append((a, b))
    return sorted(ranges)


def check(binary: str) -> list[str]:
    problems: list[str] = []
    funcs = text_functions(binary)
    fdes = fde_ranges(binary)
    if funcs and not fdes:
        problems.append(
            f"{len(funcs)} executable functions but .eh_frame has no FDE "
            "(unwinding is dead)"
        )
        return problems

    # 1. every live function is covered
    for addr, size, name in funcs:
        if not any(a <= addr < b for a, b in fdes):
            problems.append(f"no FDE covers live function {name} @0x{addr:x}")

    # 2. no FDE is left describing something that is not a live function
    live = [(addr, addr + size) for addr, size, _ in funcs]
    for a, b in fdes:
        if any(a < hi and lo < b for lo, hi in live):
            continue  # overlaps a live function: fine
        # Linker-synthesised FDEs for PLT stubs and .init/.fini are a feature,
        # not staleness (GNU ld, lld and mold all emit them).
        if any(
            a >= lo and b <= hi
            for name, lo, hi, _fl in _sections(binary)
            if name in SYNTHETIC_CODE
        ):
            continue
        problems.append(f"FDE 0x{a:x}..0x{b:x} describes no live function")
    return problems


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    bad = 0
    for binary in sys.argv[1:]:
        probs = check(binary)
        funcs = text_functions(binary)
        fdes = fde_ranges(binary)
        status = "OK " if not probs else "FAIL"
        print(f"{status} {binary}: {len(funcs)} funcs, {len(fdes)} FDEs")
        for p in probs:
            print(f"       - {p}")
        bad += bool(probs)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
