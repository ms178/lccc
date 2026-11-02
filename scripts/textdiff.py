#!/usr/bin/env python3
"""Error-text differential: LCCC vs GNU as diagnostics parity.

insndiff.py checks verdicts (accept/reject) and bytes; this checks that rows
where BOTH assemblers reject an instruction also agree on the diagnostic
text. Usage mirrors insndiff.py (same --lccc/--as/--file/--sweep flags).
"""
from __future__ import annotations

import argparse
import os
import shutil
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import insndiff  # noqa: E402


def norm_lccc(err: str) -> str:
    e = err.strip()
    if "error:" in e:
        e = e.split("error:", 1)[1]
    # some diagnostics carry a file:line prefix after the tag
    e = e.strip()
    return e


def norm_gas(err: str) -> str:
    e = err.strip()
    if "Error:" in e:
        e = e.split("Error:", 1)[1]
    return e.strip()


def main() -> int:
    ap = argparse.ArgumentParser(
        description="LCCC vs GNU as error-text differential.")
    ap.add_argument("--lccc", default=str(insndiff.DEFAULT_LCCC))
    ap.add_argument("--as", dest="gas", default=os.environ.get("LCCC_GAS", "as"))
    ap.add_argument("--objcopy", default=os.environ.get("LCCC_OBJCOPY", "objcopy"))
    ap.add_argument("--file", type=Path, help="one instruction per line")
    ap.add_argument("--sweep", action="append", default=[])
    ap.add_argument("--prologue", default=".text")
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args()

    for tool, label in ((args.lccc, "lccc"), (args.gas, "as")):
        if not (shutil.which(tool) or Path(tool).exists()):
            print(f"error: {label} not found at {tool!r}", file=sys.stderr)
            return 2

    insns: list[str] = []
    if args.file:
        insns += [l for l in args.file.read_text().splitlines()
                  if l.strip() and not l.strip().startswith("#")]
    for t in args.sweep:
        insns += insndiff.expand(t)
    if not insns and not sys.stdin.isatty():
        insns += [l for l in sys.stdin.read().splitlines()
                  if l.strip() and not l.strip().startswith("#")]
    if not insns:
        print("error: no instructions given", file=sys.stderr)
        return 2

    both = match = mismatch = internal = 0
    with tempfile.TemporaryDirectory(prefix="textdiff-") as td:
        tmp = Path(td)
        src = tmp / "i.s"
        for i, inst in enumerate(insns):
            before, after = insndiff.local_label_scaffold(inst)
            parts = [args.prologue]
            if before:
                parts.append(before)
            parts.append(inst)
            if after:
                parts.append(after)
            src.write_text("\n".join(parts) + "\n")
            le = insndiff.encode_lccc(args.lccc, src, tmp / f"l{i}.o", args.objcopy)
            ge = insndiff.encode_gas(args.gas, src, tmp / f"g{i}.o", args.objcopy)
            if le.ok or ge.ok:
                continue
            both += 1
            lt, gt = norm_lccc(le.error), norm_gas(ge.error)
            if "internal" in le.error.lower():
                internal += 1
                if not args.quiet:
                    print(f"INTERNAL   {inst}\n    lccc={le.error}\n    gas ={ge.error}")
                continue
            if lt == gt:
                match += 1
            else:
                mismatch += 1
                if not args.quiet:
                    print(f"MISMATCH   {inst}\n    lccc={lt}\n    gas ={gt}")
    print(f"\n=== textdiff: {len(insns)} instruction(s): "
          f"both-reject={both} text-match={match} "
          f"TEXT-MISMATCH={mismatch} INTERNAL={internal} ===")
    return 1 if (mismatch or internal) else 0


if __name__ == "__main__":
    sys.exit(main())
