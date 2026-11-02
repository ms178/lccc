#!/usr/bin/env python3
"""Distill EVEX/VEX opcode parameters for a mnemonic family from GAS 2.47.

For each mnemonic, assembles a small register-form battery with the pinned
GAS 2.47 oracle and extracts the prefix fields (EVEX: map/pp/W/L; VEX:
mm/pp/L) plus opcode straight from the emitted bytes. The result is the
data-driven opcode table for the encoder — no opcode from memory, every
entry proven against the oracle.

Usage: scripts/distill_evex_opcodes.py --as PATH --file families.txt
Each non-comment line of the input file: <mnemonic> <tab-separated sample operands...>
"""
from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]


def assemble_bytes(gas: str, flags: list[str], text: str, tmp: Path, name: str) -> bytes | None:
    src = tmp / f"{name}.s"
    src.write_text(text)
    obj = tmp / f"{name}.o"
    proc = subprocess.run([gas, *flags, str(src), "-o", str(obj)],
                          capture_output=True, text=True)
    if proc.returncode != 0:
        return None
    out = obj.with_suffix(".bin")
    subprocess.run(["objcopy", "-O", "binary", "--only-section=.text",
                    str(obj), str(out)], check=True)
    return out.read_bytes()


def decode_prefix(b: bytes) -> dict:
    """Extract (kind, map, pp, W, L) from an encoded instruction."""
    if b[0] == 0x62:
        p1 = b[1]
        p2 = b[2]
        p3 = b[3]
        mm = p1 & 3
        pp = p2 & 3
        w = (p2 >> 7) & 1
        l = (p3 >> 5) & 3
        return {"kind": "EVEX", "map": mm, "pp": pp, "W": w, "L": l,
                "opcode": b[4], "bytes": b.hex()}
    if b[0] == 0xC4:
        p1 = b[1]
        p2 = b[2]
        mm = p1 & 0x1F
        pp = p2 & 3
        l = (p2 >> 2) & 1
        w = (p2 >> 7) & 1
        return {"kind": "VEX3", "map": mm, "pp": pp, "W": w, "L": l,
                "opcode": b[3], "bytes": b.hex()}
    if b[0] == 0xC5:
        p2 = b[1]
        pp = p2 & 3
        l = (p2 >> 2) & 1
        return {"kind": "VEX2", "map": 1, "pp": pp, "W": 0, "L": l,
                "opcode": b[2], "bytes": b.hex()}
    return {"kind": "LEGACY", "bytes": b.hex()}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--as", dest="gas", required=True)
    ap.add_argument("--flags", default=["--64"], nargs="*")
    ap.add_argument("--file", required=True)
    args = ap.parse_args()

    with tempfile.TemporaryDirectory(prefix="distill-") as td:
        tmp = Path(td)
        for lineno, raw in enumerate(Path(args.file).read_text().splitlines()):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            mnem = parts[0]
            forms = parts[1:]
            print(f"== {mnem} ==")
            for i, form in enumerate(forms):
                src = f".text\n{form}\n"
                b = assemble_bytes(args.gas, args.flags, src, tmp, f"i{lineno}_{i}")
                if b is None:
                    print(f"   REJECT  {form}")
                    continue
                info = decode_prefix(b)
                print(f"   {info['bytes']:30s} {info}   <- {form}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
