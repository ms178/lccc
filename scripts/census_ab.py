#!/usr/bin/env python3
"""Binary A/B instruction screening: OLD lccc vs NEW lccc over a corpus.

Runs scripts/ra_quality_census.py twice (once per binary, lccc-only columns)
and diffs the per-function RA buckets (insns/rrmov/stkref/push/acc).  This is
the screening gate for every allocator/codegen change: whole-corpus static
evidence before any wall-clock claim.

Gate (HOT code decides, mirroring ra_ab_census): spill traffic (stkref) over
non-main functions, instruction count as tiebreak.  Exit 1 on regression
unless --no-gate.

Usage:
    scripts/census_ab.py OLD_LCCC NEW_LCCC [--opt -O2] [--json out.json]
    scripts/census_ab.py OLD NEW --top 20 -- files...
    (no files: benchmark programs + kernel corpus + patterns oracle)
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CENSUS = REPO / "scripts" / "ra_quality_census.py"
DEFAULT_DIRS = [
    REPO / "tests" / "benchmark" / "programs",
    REPO / "tests" / "benchmark" / "kernel_corpus",
    REPO / "tests" / "benchmark" / "patterns",
]

BUCKETS = ("insns", "rrmov", "stkref", "push", "acc")


def expand_files(args: list[Path]) -> list[Path]:
    if not args:
        args = DEFAULT_DIRS
    out: list[Path] = []
    for p in args:
        if p.is_dir():
            out.extend(sorted(p.glob("*.c")))
        elif p.is_file():
            out.append(p)
        else:
            print(f"census_ab: not found: {p}", file=sys.stderr)
    return out


def compiler_identity(lccc: str) -> dict:
    """Self-validating A/B: record exactly which binary each side ran."""
    ident: dict[str, str] = {"path": lccc}
    try:
        import hashlib
        ident["md5"] = hashlib.md5(Path(lccc).read_bytes()).hexdigest()[:16]
    except OSError:
        ident["md5"] = "?"
    try:
        r = subprocess.run([lccc, "--version"], capture_output=True, text=True,
                           timeout=30)
        ident["version"] = (r.stdout + r.stderr).strip().splitlines()[0][:80] \
            if (r.stdout + r.stderr).strip() else "?"
    except (subprocess.TimeoutExpired, OSError, IndexError):
        ident["version"] = "?"
    return ident


def run_census(lccc: str, files: list[Path], opt: str, cflags: list[str]) -> dict:
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tmp:
        tmp_path = tmp.name
    cmd = [sys.executable, str(CENSUS), "--no-gcc", "--no-clang",
           "--opt", opt, "--json", tmp_path]
    for c in cflags:
        cmd += ["--cflag", c]
    cmd += [str(f) for f in files]
    env = dict(os.environ)
    env["LCCC"] = lccc
    r = subprocess.run(cmd, capture_output=True, text=True, cwd=REPO, timeout=1200)
    if r.returncode != 0:
        print(f"census_ab: census failed for {lccc}:\n{r.stdout}\n{r.stderr}",
              file=sys.stderr)
        raise SystemExit(2)
    with open(tmp_path) as f:
        data = json.load(f)
    os.unlink(tmp_path)
    return data


def key(fn: dict) -> tuple[str, str]:
    return (fn.get("file", ""), fn.get("fn", ""))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("old", help="baseline lccc binary")
    ap.add_argument("new", help="candidate lccc binary")
    ap.add_argument("files", nargs="*", type=Path)
    ap.add_argument("--opt", default="-O2")
    ap.add_argument("--cflag", action="append", default=[])
    ap.add_argument("--json", type=Path)
    ap.add_argument("--top", type=int, default=0,
                    help="show only the N largest (by |stkref|,|insns|) deltas")
    ap.add_argument("--no-gate", action="store_true",
                    help="report-only: never exit 1 on regression")
    args = ap.parse_args()

    files = expand_files(args.files)
    if not files:
        print("census_ab: empty corpus", file=sys.stderr)
        return 2
    old = run_census(args.old, files, args.opt, args.cflag)
    new = run_census(args.new, files, args.opt, args.cflag)

    old_f = {key(f): f.get("lccc", {}) for f in old.get("functions", [])}
    new_f = {key(f): f.get("lccc", {}) for f in new.get("functions", [])}
    names = sorted(set(old_f) | set(new_f))

    deltas = []
    for name in names:
        o, n = old_f.get(name, {}), new_f.get(name, {})
        row = {"file": name[0], "fn": name[1]}
        for b in BUCKETS:
            row[b] = n.get(b, 0) - o.get(b, 0)
        row["old_missing"] = name not in old_f
        row["new_missing"] = name not in new_f
        # Per-side buckets retained: deltas alone cannot diagnose a
        # flipped side or a miscounted function.
        row["o"] = {b: o.get(b, 0) for b in BUCKETS}
        row["n"] = {b: n.get(b, 0) for b in BUCKETS}
        deltas.append(row)

    totals_old = {b: sum(f.get(b, 0) for f in old_f.values()) for b in BUCKETS}
    totals_new = {b: sum(f.get(b, 0) for f in new_f.values()) for b in BUCKETS}
    total_delta = {b: totals_new[b] - totals_old[b] for b in BUCKETS}

    def fmt(v: int) -> str:
        return f"{v:+d}" if v else "0"

    print(f"corpus: {len(files)} files, {len(names)} functions "
          f"({args.opt}{' ' + ' '.join(args.cflag) if args.cflag else ''})")
    print(f"OLD={args.old}\nNEW={args.new}")
    print("-" * 100)
    hdr = f"{'file':32s} {'fn':28s}" + "".join(f"{b:>9s}" for b in BUCKETS)
    print(hdr + "   (NEW-OLD; negative = fewer = better)")
    changed = [d for d in deltas
               if any(d[b] for b in BUCKETS) or d["old_missing"] or d["new_missing"]]
    changed.sort(key=lambda d: (d["stkref"], d["insns"], d["rrmov"],
                                d["file"], d["fn"]))
    show = changed[:args.top] if args.top else changed
    for d in show:
        line = f"{d['file'][:32]:32s} {d['fn'][:28]:28s}" + \
            "".join(f"{fmt(d[b]):>9s}" for b in BUCKETS)
        if d["old_missing"] or d["new_missing"]:
            line += "   [MISSING SIDE]"
        print(line)
    if args.top and len(changed) > args.top:
        print(f"... and {len(changed) - args.top} more changed functions")
    print("-" * 100)
    print(f"{'TOTAL':32s} {len(names):>28d}" +
          "".join(f"{fmt(total_delta[b]):>9s}" for b in BUCKETS))
    print(f"unchanged: {len(deltas) - len(changed)}/{len(deltas)} functions; "
          f"skipped(old)={old.get('skipped', [])} skipped(new)={new.get('skipped', [])}")

    # Gate: HOT (non-main) stkref, insns tiebreak — same policy as ra_ab_census.
    hot_old = sum(v.get("stkref", 0) for k, v in old_f.items() if k[1] != "main")
    hot_new = sum(v.get("stkref", 0) for k, v in new_f.items() if k[1] != "main")
    ins_old = sum(v.get("insns", 0) for k, v in old_f.items() if k[1] != "main")
    ins_new = sum(v.get("insns", 0) for k, v in new_f.items() if k[1] != "main")
    regress = (hot_new > hot_old) or (hot_new == hot_old and ins_new > ins_old)
    print(f"gate: HOT stkref {hot_old}->{hot_new}, insns {ins_old}->{ins_new} "
          f"=> {'REGRESSION' if regress else 'PASS'}")

    if args.json:
        args.json.write_text(json.dumps({
            "old": args.old, "new": args.new, "opt": args.opt,
            "cflags": args.cflag, "n_files": len(files),
            "identity_old": compiler_identity(args.old),
            "identity_new": compiler_identity(args.new),
            "totals_old": totals_old, "totals_new": totals_new,
            "total_delta": total_delta, "gate_regression": regress,
            "functions": deltas,
        }, indent=1))
        print(f"wrote {args.json}")
    return 1 if (regress and not args.no_gate) else 0


if __name__ == "__main__":
    raise SystemExit(main())
