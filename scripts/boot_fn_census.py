#!/usr/bin/env python3
"""Per-function `.text` size census for the Linux x86 boot objects.

`scripts/boot_size_oracle.sh` answers the per-*object* question ("which of the
23 setup objects is bigger under lccc?").  That is the right granularity for the
32 KiB gate but the wrong one for fixing it: a 1599-byte delta in `printf.o`
is a handful of functions, and the fix is a code-generation pattern, not an
object.

This script narrows the last mile.  It takes two object directories (the lccc
build and a reference-toolchain build, both produced by
`scripts/boot_size_oracle.sh` with byte-identical command lines) and reports,
per object, every function whose `.text` extent differs — sorted by absolute
delta, so the first rows are exactly the functions worth reading.

Function extents come from the ELF symbol table (`st_size` of `STT_FUNC`
symbols in `.text`), not from disassembly: identical numbers, no
disassembler in the loop, and it works for objects whose bytes the two
assemblers laid out differently.  A `--dump` mode prints the disassembly of
the worst offenders side by side, because the delta alone never says *why*.

Usage:
    scripts/boot_size_oracle.sh                     # builds $OUT and $OUT/oracle-gcc
    scripts/boot_fn_census.py                       # census of those two
    scripts/boot_fn_census.py --top 12 --dump 3     # and disassemble the worst 3
    scripts/boot_fn_census.py --object printf       # one object only
Environment:
    OUT        lccc object dir   (default /var/tmp/bootbuild)
    OOUT       oracle object dir (default $OUT/oracle-gcc)
"""
from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

DEFAULT_OUT = Path(os.environ.get("OUT", "/var/tmp/bootbuild"))


def readelf_symbols(obj: Path) -> dict[str, int]:
    """{function name: .text byte extent} from the ELF symbol table."""
    if not obj.exists():
        return {}
    out = subprocess.run(
        ["readelf", "-sW", str(obj)], capture_output=True, text=True, check=True
    ).stdout
    syms: dict[str, int] = {}
    for line in out.splitlines():
        # `   12: 00000000   142 FUNC    GLOBAL DEFAULT    1 puts`
        f = line.split()
        if len(f) < 8 or f[3] != "FUNC":
            continue
        try:
            size = int(f[2])
        except ValueError:
            continue
        name = f[7]
        # Keep the largest extent if a name appears more than once (aliases).
        if size > syms.get(name, -1):
            syms[name] = size
    return syms


def section_text(obj: Path) -> int:
    """Total executable bytes.

    The boot objects are built with `-ffunction-sections`, so the `.text`
    section itself is empty and every function lives in `.text.<name>`;
    summing only `.text` reports 0 for every object.
    """
    out = subprocess.run(
        ["readelf", "-SW", str(obj)], capture_output=True, text=True, check=True
    ).stdout
    total = 0
    for line in out.splitlines():
        f = line.split()
        # `  [ 4] .text.foo PROGBITS 00000000 000034 0002d4 00  AX  0   0  1`
        idx = [i for i, t in enumerate(f) if t == ".text" or t.startswith(".text.")]
        # `[ 4] .text.foo PROGBITS 00000000 000034 0002d4 00  AX  0   0  1`
        #  -> name, type, addr, off, SIZE: the size is the 5th token after the
        #  bracket pair, and `[ 4]` splits into two fields, so index off the
        #  name rather than a fixed column.
        if not idx or len(f) < idx[0] + 5:
            continue
        try:
            total += int(f[idx[0] + 4], 16)
        except ValueError:
            continue
    return total


def disasm_func(obj: Path, func: str, cc_label: str) -> list[str]:
    """Disassemble one function, stripping addresses so two toolchains align."""
    if not obj.exists():
        return [f"({cc_label}: object missing)"]
    try:
        out = subprocess.run(
            ["objdump", "-d", "--no-show-raw-insn", "--no-addresses",
             "--disassemble=" + func, str(obj)],
            capture_output=True, text=True, check=True,
        ).stdout
    except subprocess.CalledProcessError:
        return [f"({cc_label}: no such function)"]
    lines = []
    for line in out.splitlines():
        line = line.rstrip()
        if not line or line.startswith("Disassembly") or line.endswith(">:"):
            continue
        lines.append("    " + line.strip())
    return lines


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT,
                    help="lccc object directory")
    ap.add_argument("--oout", type=Path, default=None,
                    help="oracle object directory (default $OUT/oracle-gcc)")
    ap.add_argument("--top", type=int, default=20,
                    help="rows per object (default 20)")
    ap.add_argument("--dump", type=int, default=0,
                    help="disassemble the N worst functions side by side")
    ap.add_argument("--object", default=None,
                    help="restrict to objects whose name contains this")
    ap.add_argument("--global-top", type=int, default=25,
                    help="worst functions across all objects (default 25)")
    args = ap.parse_args()

    oout = args.oout or (args.out / "oracle-gcc")
    if not args.out.is_dir():
        print(f"no lccc object dir: {args.out}\n"
              f"run: KERNEL_DIR=... OUT={args.out} scripts/boot_size_oracle.sh",
              file=sys.stderr)
        return 2
    if not oout.is_dir():
        print(f"no oracle object dir: {oout}", file=sys.stderr)
        return 2

    objects = sorted(p.stem for p in args.out.glob("*.o"))
    if args.object:
        objects = [o for o in objects if args.object in o]
        if not objects:
            print(f"no object matching {args.object!r}", file=sys.stderr)
            return 2

    rows: list[tuple[int, int, int, str, str]] = []
    print(f"{'OBJECT':22} {'FUNCTION':30} {'LCCC':>7} {'ORACLE':>7} {'DELTA':>7}")
    print("-" * 80)
    for name in objects:
        a = readelf_symbols(args.out / f"{name}.o")
        b = readelf_symbols(oout / f"{name}.o")
        if not a and not b:
            continue
        deltas = []
        for fn in sorted(set(a) | set(b)):
            la, lb = a.get(fn, 0), b.get(fn, 0)
            if la == lb:
                continue
            deltas.append((abs(la - lb), la, lb, fn))
            rows.append((la - lb, la, lb, name, fn))
        deltas.sort(reverse=True)
        printed = 0
        for _absd, la, lb, fn in deltas[: args.top]:
            print(f"{name:22} {fn:30} {la:7d} {lb:7d} {la - lb:+7d}")
            printed += 1
        if len(deltas) > printed:
            rest = sum(d[1] - d[2] for d in deltas[printed:])
            print(f"{name:22} {'<%d more differing functions>' % (len(deltas) - printed):30}"
                  f" {'':>7} {'':>7} {rest:+7d}")
        ta, tb = section_text(args.out / f"{name}.o"), section_text(oout / f"{name}.o")
        print(f"{name:22} {'== .text total':30} {ta:7d} {tb:7d} {ta - tb:+7d}")
        print()

    ta = sum(section_text(args.out / f"{n}.o") for n in objects)
    tb = sum(section_text(oout / f"{n}.o") for n in objects)
    pct = f"   ({100.0 * ta / tb - 100:+.1f}% vs oracle)" if tb else ""
    print(f"{'TOTAL .text':22} {'':30} {ta:7d} {tb:7d} {ta - tb:+7d}{pct}")

    rows.sort(key=lambda r: -abs(r[0]))
    print(f"\nWorst functions overall (top {args.global_top}):")
    for delta, la, lb, name, fn in rows[: args.global_top]:
        print(f"  {delta:+7d}  {name}.{fn}  (lccc {la}, oracle {lb})")

    if args.dump:
        for _delta, _la, _lb, name, fn in rows[: args.dump]:
            print(f"\n{'=' * 78}\n{name}.o :: {fn}\n{'=' * 78}")
            left = disasm_func(args.out / f"{name}.o", fn, "lccc")
            right = disasm_func(oout / f"{name}.o", fn, "oracle")
            width = max(len(left), len(right))
            print(f"{'LCCC':<40} | ORACLE")
            for i in range(width):
                l = left[i] if i < len(left) else ""
                r = right[i] if i < len(right) else ""
                print(f"{l:<40} | {r}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
