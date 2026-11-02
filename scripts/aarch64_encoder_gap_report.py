#!/usr/bin/env python3
"""AArch64 encoder gap report: classify *how* an encoder disagrees with GNU as.

WHY THIS EXISTS
---------------
``aarch64_operand_legality_matrix.py`` answers a boolean question: does our
assembler agree with GNU as on every row of the curated table?  A boolean is
the right gate, but it is the wrong *report*: "41 rows disagree" cannot tell
you whether the encoder is emitting a different word for a valid instruction
(silent wrong code) or refusing an instruction GNU as accepts (a usability
gap), and those two have completely different severities.

This tool runs the same table through one or more encoders and classifies
every disagreement:

  MISENCODE   GNU as and the encoder both accept, but the words differ.
              Silent wrong code: the most severe class.
  FAIL_OPEN   GNU as rejects, the encoder accepts (and emits words).
              An invalid operand became some other valid instruction.
  FAIL_CLOSED GNU as accepts, the encoder rejects.  A coverage gap, not
              wrong code, but still a divergence.

Rows are reported per group and per class so a fix can be measured against
the exact architectural rule it targets.  Multiple encoders can be compared
in one run, which is what turns "my patch fixes things" into a table.

USAGE
-----
    scripts/aarch64_encoder_gap_report.py --lccc BIN [--lccc BIN] \
        [--as PATH] [--objcopy PATH] [--csv OUT] [--only GROUP]

Exit status is 0 when every encoder agrees with GNU as on every row, 1 when
any encoder disagrees, 2 when a tool is missing (mismatch vs cannot-tell).
"""
from __future__ import annotations

import argparse
import csv
import importlib.util
import sys
import tempfile
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MATRIX = REPO / "scripts" / "aarch64_operand_legality_matrix.py"


def _load_matrix():
    spec = importlib.util.spec_from_file_location("aarch64_matrix", MATRIX)
    mod = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(mod)
    return mod


def classify(expected: str, got: str) -> str | None:
    """Disagreement class for one row, or None when the encoder agrees."""
    if expected == got:
        return None
    if expected.startswith("OK ") and got.startswith("OK "):
        return "MISENCODE"
    if expected == "REJECT" and got.startswith("OK "):
        return "FAIL_OPEN"
    if expected.startswith("OK ") and got == "REJECT":
        return "FAIL_CLOSED"
    # REJECT vs REJECT is equality; nothing else is reachable.
    return "OTHER"


def run_encoder(matrix, binary: Path, rows, as_bin: str, objcopy: str, tmp: Path):
    """(insn, gas_words, encoder_words, class) for every disagreement."""
    link = tmp / f"aarch64-linux-gnu-ccc-{binary.name}"
    if link.exists() or link.is_symlink():
        link.unlink()
    link.symlink_to(binary.resolve())
    out = []
    for group, insn, expected in rows:
        got = matrix.assemble(
            insn, [str(link), "-c"], objcopy, tmp, prologue=".text\n"
        )
        cls = classify(expected, got)
        if cls:
            out.append((group, insn, expected, got, cls))
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc", action="append", default=[], metavar="BIN")
    ap.add_argument("--as", dest="as_bin", default=None)
    ap.add_argument("--objcopy", default=None)
    ap.add_argument("--csv", default=None, help="write every row's verdict")
    ap.add_argument("--only", default=None, help="restrict to one group")
    args = ap.parse_args()

    if not args.lccc:
        ap.error("at least one --lccc BIN is required")

    matrix = _load_matrix()
    as_bin = args.as_bin or str(
        Path.home() / ".cache" / "gas-2.47-aarch64-linux-gnu" / "bin" / "as"
    )
    objcopy = args.objcopy or "/usr/bin/aarch64-linux-gnu-objcopy"
    if not Path(as_bin).exists() or not Path(objcopy).exists():
        print(f"gap report: missing oracle ({as_bin} / {objcopy})", file=sys.stderr)
        return 2

    rows = matrix.parse_table(matrix.TABLE)
    if args.only:
        rows = [r for r in rows if r[0] == args.only]
    if not rows:
        print("gap report: no rows selected", file=sys.stderr)
        return 2

    total_rows = len(rows)
    any_bad = False
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        for binary in args.lccc:
            path = Path(binary)
            if not path.exists():
                print(f"gap report: no such encoder: {path}", file=sys.stderr)
                return 2
            bad = run_encoder(matrix, path, rows, as_bin, objcopy, tmp)
            counts = Counter(c for *_, c in bad)
            print(f"=== {path.name} : {len(bad)}/{total_rows} rows disagree ===")
            for cls in ("MISENCODE", "FAIL_OPEN", "FAIL_CLOSED", "OTHER"):
                if counts[cls]:
                    print(f"    {cls:12s} {counts[cls]}")
            per_group: dict[str, Counter] = defaultdict(Counter)
            for group, *_rest in bad:
                per_group[group][_rest[-1]] += 1
            for group in sorted(per_group):
                c = per_group[group]
                detail = ", ".join(f"{k}={v}" for k, v in sorted(c.items()))
                print(f"      [{group}] {sum(c.values())} ({detail})")
            if bad:
                any_bad = True
                print("    --- worst class first ---")
                order = {"MISENCODE": 0, "FAIL_OPEN": 1, "FAIL_CLOSED": 2, "OTHER": 3}
                for group, insn, expected, got, cls in sorted(
                    bad, key=lambda r: order[r[4]]
                )[:60]:
                    print(f"    {cls:10s} [{group}] {insn}")
                    print(f"               gas={expected}")
                    print(f"               lccc={got}")
            if args.csv:
                with open(args.csv, "w", newline="") as fh:
                    w = csv.writer(fh)
                    w.writerow(["encoder", "group", "instruction", "gas", "lccc", "class"])
                    for group, insn, expected, got, cls in bad:
                        w.writerow([path.name, group, insn, expected, got, cls])
    return 1 if any_bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
