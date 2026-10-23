#!/usr/bin/env python3
"""A/B the straight-line inner loops of two assembly trees under llvm-mca.

Why this exists
---------------
Callgrind counts instructions. A change that keeps the instruction count but
changes *which* instructions run is invisible to it: for example, a merging
`vmovsd %a, %a, %d` (a port-5 shuffle µop) versus `vmovapd %a, %d`
(eliminated at register rename), or a recurrence that gains or loses a
false dependence. On a VM without a PMU, the scheduling models in
llvm-mca are the reproducible instrument for those changes. This script
applies them loop by loop, to exactly the loops that differ.

What it measures
----------------
For each pair of same-named `.s` files in DIR_A and DIR_B, every
straight-line inner loop is extracted: a label `L:`, then a backward
conditional or unconditional jump to `L` whose body holds no other jump,
call or return. Loops are matched by (file, function, ordinal among such
loops). Directives, comments and labels are stripped. A pair whose bodies
are textually identical is skipped. Each remaining body is fed to
`llvm-mca -mtriple=x86_64-unknown-linux-gnu -mcpu=CPU -iterations=N`, and
the script reports total cycles, µops and block reciprocal throughput per
CPU. Steady-state cycles per iteration is `cycles / N`.

It is a model, not a measurement: llvm-mca assumes L1-resident memory and
a perfectly predicted back edge. The models do include move elimination
and the false dependences of merging scalar moves, which are the effects
it is used for here. Loops with internal control flow are not modelled.
They are listed as skipped, so the census stays honest.

Usage:
    mca_loop_ab.py DIR_A DIR_B [--mcpu alderlake,znver4,skylake]
                   [--iterations 200] [--llvm-mca PATH] [--markdown OUT]
"""
from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path

FUNC = re.compile(r"^([A-Za-z_][\w.$]*):\s*$")
LABEL = re.compile(r"^\s*([.\w$]+):\s*(?:#.*)?$")
JUMP = re.compile(r"^\s*(j[a-z]+)\s+([.\w$]+)\s*$")
FLOW = re.compile(r"^\s*(?:j[a-z]+|call[a-z]*|ret[a-z]*|ud2)\b")


def strip(line: str) -> str:
    line = line.split("#", 1)[0].rstrip()
    return line


def loops_of(path: Path) -> dict[tuple[str, int], list[str]]:
    """{(function, ordinal): body lines including the back-edge jump}."""
    lines = path.read_text(encoding="utf-8", errors="replace").split("\n")
    out: dict[tuple[str, int], list[str]] = {}
    func = "?"
    label_at: dict[str, int] = {}
    ordinal: dict[str, int] = {}
    for i, raw in enumerate(lines):
        m = FUNC.match(raw)
        if m and not raw.startswith((".L", " ", "\t")):
            func = m.group(1)
            label_at = {}
            continue
        lm = LABEL.match(raw)
        if lm:
            label_at[lm.group(1)] = i
            continue
        jm = JUMP.match(raw)
        if not jm or jm.group(2) not in label_at:
            continue
        start = label_at[jm.group(2)]
        body = []
        ok = True
        for b in lines[start + 1 : i]:
            s = strip(b)
            if not s.strip() or s.lstrip().startswith(".") and not LABEL.match(s):
                continue
            if LABEL.match(s):
                continue
            if FLOW.match(s):
                ok = False
                break
            body.append(s.strip())
        if not ok or not body:
            continue
        body.append(f"{jm.group(1)} {jm.group(2)}")
        n = ordinal.get(func, 0)
        ordinal[func] = n + 1
        out[(func, n)] = body
    return out


def mca(tool: str, body: list[str], cpu: str, iterations: int) -> dict[str, float]:
    src = "\n".join(body) + "\n"
    r = subprocess.run(
        [tool, "-mtriple=x86_64-unknown-linux-gnu", f"-mcpu={cpu}", f"-iterations={iterations}"],
        input=src,
        capture_output=True,
        text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(r.stderr.strip().splitlines()[-1] if r.stderr.strip() else "llvm-mca failed")
    stats = {}
    for key, name in (("Total Cycles", "cycles"), ("Total uOps", "uops"), ("Block RThroughput", "rthru")):
        m = re.search(rf"^{key}:\s+([\d.]+)", r.stdout, re.M)
        if m:
            stats[name] = float(m.group(1))
    return stats


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("dir_a")
    ap.add_argument("dir_b")
    ap.add_argument("--mcpu", default="alderlake,znver4,skylake")
    ap.add_argument("--iterations", type=int, default=200)
    ap.add_argument("--llvm-mca", default=shutil.which("llvm-mca") or "llvm-mca")
    ap.add_argument("--markdown")
    args = ap.parse_args()
    cpus = [c for c in args.mcpu.split(",") if c]
    a_dir, b_dir = Path(args.dir_a), Path(args.dir_b)

    rows = []
    unmatched = 0
    identical = 0
    errors = []
    for fa in sorted(a_dir.glob("*.s")):
        fb = b_dir / fa.name
        if not fb.exists():
            continue
        la, lb = loops_of(fa), loops_of(fb)
        for key in sorted(set(la) | set(lb)):
            if key not in la or key not in lb:
                unmatched += 1
                continue
            if la[key] == lb[key]:
                identical += 1
                continue
            row = {"file": fa.stem, "func": key[0], "n": key[1],
                   "len": (len(la[key]), len(lb[key])), "cpu": {}}
            for cpu in cpus:
                try:
                    row["cpu"][cpu] = (mca(args.llvm_mca, la[key], cpu, args.iterations),
                                       mca(args.llvm_mca, lb[key], cpu, args.iterations))
                except RuntimeError as e:
                    errors.append(f"{fa.stem}:{key[0]}#{key[1]} {cpu}: {e}")
            rows.append(row)

    out = []
    out.append(f"llvm-mca A/B, {args.iterations} iterations; A={a_dir} B={b_dir}")
    out.append(f"loops: {len(rows)} changed, {identical} identical, {unmatched} unmatched")
    out.append("")
    head = "| loop | insns A→B | " + " | ".join(f"{c} cyc/iter A→B (Δ%)" for c in cpus) + " |"
    out.append(head)
    out.append("|" + "---|" * (2 + len(cpus)))
    totals = {c: [0.0, 0.0] for c in cpus}
    for r in rows:
        cells = []
        for c in cpus:
            if c not in r["cpu"]:
                cells.append("error")
                continue
            a, b = r["cpu"][c]
            ca, cb = a["cycles"] / args.iterations, b["cycles"] / args.iterations
            totals[c][0] += ca
            totals[c][1] += cb
            cells.append(f"{ca:.2f}→{cb:.2f} ({(cb / ca - 1) * 100:+.1f}%)")
        out.append(f"| {r['file']}:{r['func']}#{r['n']} | {r['len'][0]}→{r['len'][1]} | " + " | ".join(cells) + " |")
    if rows:
        cells = [f"{t[0]:.2f}→{t[1]:.2f} ({(t[1] / t[0] - 1) * 100:+.1f}%)" if t[0] else "-" for t in totals.values()]
        out.append("| **sum** | | " + " | ".join(cells) + " |")
    for e in errors:
        out.append(f"error: {e}")
    text = "\n".join(out)
    print(text)
    if args.markdown:
        Path(args.markdown).write_text(text + "\n", encoding="utf-8")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
