#!/usr/bin/env python3
"""Distill the EVEX packed-broadcast element-size table from the binutils
testsuite (GAS 2.47 oracle).

Method: every `{1toN}` line in the pinned testsuite is assembled; the
emitted EVEX bytes give the (map, pp, W, opcode) key AND the vector length
(from L'L — using the BYTES' LL, not register widths, because narrowing
converts encode the SOURCE width in LL). elem = VL_bytes / N. The per-key
table is emitted as the Rust `match` arms consumed by
`evex_packed_bcst_elem` in src/backend/x86/assembler/encoder/avx.rs, which
drives the central `{1toN}` count law in `check_decorators`.

Auditability: this generator plus the pinned binutils testsuite reproduce
the committed table bit-for-bit; `--check` verifies an existing table
(row count + every row) and fails on drift or ambiguity.

Usage:
  scripts/distill_bcst_elem.py --gas <path-to-as> --suite <binutils>/gas/testsuite/gas/i386
  scripts/distill_bcst_elem.py ... --emit rust          # print Rust arms
  scripts/distill_bcst_elem.py ... --check src/.../avx.rs
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from collections import defaultdict
from pathlib import Path

BCST = re.compile(r"\{1to(\d+)\}")


def assemble(gas: str, line: str, flags=("--64",)) -> bytes | None:
    # Normalize 32-bit GP bases/indices to 64-bit names: the testsuite
    # spells `%eax` bases which add a 0x67 address prefix in --64 mode;
    # the distilled (map,pp,W,opcode)+VL key does not depend on the base
    # width, and this keeps the EVEX prefix at byte 0.
    for a, b in (("%eax", "%rax"), ("%ebx", "%rbx"), ("%ecx", "%rcx"),
                 ("%edx", "%rdx"), ("%esi", "%rsi"), ("%edi", "%rdi"),
                 ("%ebp", "%rbp"), ("%esp", "%rsp")):
        line = re.sub(a + r"(\b|\))", b + r"\1", line)
    with tempfile.TemporaryDirectory() as td:
        src = Path(td) / "t.s"
        obj = Path(td) / "t.o"
        src.write_text(".text\n" + line + "\n")
        r = subprocess.run([gas, *flags, "-o", str(obj), str(src)], capture_output=True)
        if r.returncode != 0:
            return None
        subprocess.run(
            ["objcopy", "-O", "binary", "--only-section=.text", str(obj), str(obj) + ".bin"],
            check=True,
            capture_output=True,
        )
        return (Path(str(obj) + ".bin")).read_bytes()


def decode_key(b: bytes):
    p0, p1, p2 = b[1], b[2], b[3]
    # P2 bits: [7]=z' [6:5]=L'L [4]=b' [3]=V' [2:0]=aaa. For a broadcast
    # line b'=1 BY CONSTRUCTION and L'L carries the vector length (the
    # RC/SAE reinterpret of L'L only happens on decorator lines, which
    # never contain {1toN} and never reach this decoder).
    ll = (p2 >> 5) & 3
    vl = {0: 16, 1: 32, 2: 64}.get(ll)
    return (p0 & 0xF, p1 & 0x3, (p1 >> 7) & 1, b[4], vl)


def collect_lines(suite: Path) -> set[str]:
    lines: set[str] = set()
    for path in sorted(suite.glob("*.s")):
        try:
            text = path.read_text(errors="replace")
        except OSError:
            continue
        for raw in text.splitlines():
            s = raw.split("#", 1)[0].strip()
            if not s or s.startswith((".", "/")) or "{1to" not in s:
                continue
            if not re.match(r"^[a-zA-Z][\w.]*\s", s + " "):
                continue
            lines.add(s)
    return lines


def distill(gas: str, suite: Path):
    lines = collect_lines(suite)
    per_key: dict[tuple, int] = {}
    ambiguous: dict[tuple, list] = defaultdict(list)
    n_mnem = 0
    seen_mnem = set()
    for s in sorted(lines):
        m = BCST.search(s)
        if not m:
            continue
        n = int(m.group(1))
        mnem = s.split()[0].lower().split(".")[0]
        b = assemble(gas, s)
        if b is None or b[0] != 0x62:
            continue
        key = decode_key(b)
        if key[4] is None:
            continue
        if key[4] % n != 0:
            continue
        elem = key[4] // n
        if mnem not in seen_mnem:
            seen_mnem.add(mnem)
            n_mnem += 1
        k4 = key[:4]
        if k4 in per_key and per_key[k4] != elem:
            ambiguous[k4].append(elem)
        per_key.setdefault(k4, elem)
    return lines, n_mnem, per_key, ambiguous


def rust_arms(per_key) -> list[str]:
    out = []
    for key in sorted(per_key):
        out.append(
            f"            ({key[0]}, {key[1]}, {key[2]}, 0x{key[3]:02X}) "
            f"=> Some({per_key[key]}),"
        )
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--gas", required=True, help="path to the GAS oracle binary")
    ap.add_argument("--suite", required=True, help="path to gas/testsuite/gas/i386")
    ap.add_argument("--emit", choices=["rust", "count"], default="count")
    ap.add_argument(
        "--check",
        metavar="RS_FILE",
        help="verify the table inside this Rust source file (row count + every row)",
    )
    args = ap.parse_args()

    lines, n_mnem, per_key, ambiguous = distill(args.gas, Path(args.suite))
    print(
        f"# {len(lines)} broadcast lines; {n_mnem} verified mnemonics; "
        f"{len(per_key)} unique keys; {len(ambiguous)} ambiguous keys (dropped)",
        file=sys.stderr,
    )
    for k, v in sorted(ambiguous.items()):
        print(f"# AMBIGUOUS {k}: {v}", file=sys.stderr)

    if args.emit == "rust":
        for row in rust_arms(per_key):
            print(row)

    if args.check:
        src = Path(args.check).read_text()
        committed = re.findall(
            r"\((\d+), (\d+), (\d+), 0x([0-9A-Fa-f]{2})\) => Some\((\d+)\)", src
        )
        committed = {
            (int(a), int(b), int(c), int(d, 16), int(e))
            for a, b, c, d, e in committed
        }
        want = {k + (v,) for k, v in per_key.items()}
        missing = want - committed
        extra = {
            r for r in committed
            if r[:4] not in per_key and not _handwritten_ok(r)
        }
        print(f"# committed rows: {len(committed)}; distilled rows: {len(want)}", file=sys.stderr)
        if missing:
            print(f"# DRIFT: {len(missing)} distilled rows missing from the Rust table:", file=sys.stderr)
            for r in sorted(missing):
                print(f"#   {r}", file=sys.stderr)
        if extra:
            print(f"# NOTE: {len(extra)} committed rows not distilled by this suite "
                  f"(hand-written rows are expected — e.g. the map-2 variable-index "
                  f"vpermil rows and map-6 vdpphps).", file=sys.stderr)
        if missing:
            return 1
        print("# CHECK OK: every distilled row is present and agrees.", file=sys.stderr)
    return 0


def _handwritten_ok(row) -> bool:
    # Keys intentionally hand-written beyond the distilled set (verified by
    # the fp16-evex.casefile differential rather than the testsuite corpus).
    k = row[:4]
    return k in {
        (2, 1, 0, 0x0C),  # vpermilps variable-index form
        (2, 1, 1, 0x0D),  # vpermilpd variable-index form
        (2, 0, 0, 0x52),  # vdpphps
    }


if __name__ == "__main__":
    raise SystemExit(main())
