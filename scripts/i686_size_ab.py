#!/usr/bin/env python3
"""A/B the i686 `.text` size of two LCCC builds over a C corpus.

The 32 KiB x86 setup gate is measured on 23 hand-picked real-mode objects, and
that is the right *gate* but a narrow *signal*: an i686 peephole can be a clear
win on the regression corpus and invisible in `arch/x86/boot`, or the other way
round.  This script answers "what did this change do to 32-bit code size, in
aggregate and per file?" for any two compiler binaries.

Both sides get byte-identical command lines; only the compiler differs, so a
delta is a code-generation delta.  Sizes come from the ELF section table
(`.text*` under `-ffunction-sections`), never from a disassembly.

    scripts/i686_size_ab.py                       # tests/regression, -Os and -O2
    scripts/i686_size_ab.py --opt -Os --top 15
    scripts/i686_size_ab.py --corpus 'arch/x86/boot/*.c' --kernel-dir /opt/kwork/linux-6.18.52

Environment / defaults:
    NEW   new compiler   (default <repo>/target/fastbuild/lccc)
    OLD   old compiler   (default /tmp/lccc-prev)
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]


def text_bytes(obj: Path) -> int:
    out = subprocess.run(["readelf", "-SW", str(obj)],
                         capture_output=True, text=True, check=True).stdout
    total = 0
    for line in out.splitlines():
        f = line.split()
        idx = [i for i, t in enumerate(f) if t == ".text" or t.startswith(".text.")]
        # `[ 4] .text.foo PROGBITS 00000000 000034 0002d4 ...` — the size is
        # the 5th token after the name (`[ 4]` splits into two fields).
        if not idx or len(f) < idx[0] + 5:
            continue
        try:
            total += int(f[idx[0] + 4], 16)
        except ValueError:
            continue
    return total


def compile_size(cc: Path, src: Path, flags: list[str], out: Path) -> int | None:
    proc = subprocess.run([str(cc), *flags, "-c", str(src), "-o", str(out)],
                          capture_output=True, text=True)
    if proc.returncode != 0:
        return None
    return text_bytes(out)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--new", type=Path,
                    default=Path(os.environ.get("NEW", REPO / "target/fastbuild/lccc")))
    ap.add_argument("--old", type=Path,
                    default=Path(os.environ.get("OLD", "/tmp/lccc-prev")))
    ap.add_argument("--corpus", default="tests/regression/*.c",
                    help="glob, resolved relative to the repo or --kernel-dir")
    ap.add_argument("--kernel-dir", type=Path, default=None,
                    help="cd here and resolve --corpus relative to it")
    ap.add_argument("--opt", default="-Os -O2",
                    help="space-separated optimization levels to sweep")
    ap.add_argument("--extra-flags", default="-m32 -fomit-frame-pointer")
    ap.add_argument("--top", type=int, default=20)
    args = ap.parse_args()

    for cc in (args.new, args.old):
        if not cc.exists():
            print(f"missing compiler: {cc}", file=sys.stderr)
            return 2

    cwd = Path(args.kernel_dir) if args.kernel_dir else REPO
    sources = sorted(cwd.glob(args.corpus))
    if not sources:
        print(f"no sources match {args.corpus} under {cwd}", file=sys.stderr)
        return 2

    base_flags = args.extra_flags.split() + [
        f"-I{subprocess.run(['gcc', '-print-file-name=include'], capture_output=True, text=True).stdout.strip()}"
    ]

    grand = {}
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        for opt in args.opt.split():
            flags = base_flags + [opt, "-ffunction-sections"]
            rows = []
            skipped = 0
            for src in sources:
                a = compile_size(args.new, src, flags, tmp / "new.o")
                b = compile_size(args.old, src, flags, tmp / "old.o")
                if a is None or b is None:
                    skipped += 1
                    continue
                rows.append((a - b, a, b, src.name))
            na = sum(r[1] for r in rows)
            nb = sum(r[2] for r in rows)
            grand[opt] = (na, nb)
            changed = [r for r in rows if r[0]]
            changed.sort(key=lambda r: -abs(r[0]))
            print(f"\n=== {opt}  ({len(sources)} sources, {skipped} not compiled by both) ===")
            print(f"{'delta':>8} {'new':>8} {'old':>8}  file")
            for d, a, b, name in changed[: args.top]:
                print(f"{d:+8d} {a:8d} {b:8d}  {name}")
            if len(changed) > args.top:
                print(f"  ... {len(changed) - args.top} more differing files")
            pct = 100.0 * na / nb - 100 if nb else 0.0
            print(f"TOTAL .text: new={na} old={nb} delta={na - nb:+d} ({pct:+.2f}%) "
                  f"from {len(changed)} changed file(s)")

    print("\nsummary:")
    for opt, (na, nb) in grand.items():
        print(f"  {opt}: {na - nb:+d} bytes ({100.0 * na / nb - 100:+.2f}%)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
