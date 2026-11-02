#!/usr/bin/env python3
"""Census of loop-header alignment decisions across the benchmark corpus.

Compiles each benchmark with `-S` under several CCC_LOOP_ALIGN_HOT tiers
and reports the directive group emitted at every backward-branch target
(loop header), so the structural hot-class selection can be audited
without eyeballing assembly dumps.

Usage: scripts/align_decision_census.py LCCC [-O2] [bench...]
"""
import os
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
INCLUDE = "-I/usr/lib/gcc/x86_64-linux-gnu/14/include"
PROGRAMS = REPO / "tests/benchmark/programs"

LABEL_RE = re.compile(r"^(\.L[A-Za-z0-9_]+):\s*$")
JMP_RE = re.compile(r"^\s*(?:jmp|je|jne|jz|jnz|jg|jge|jl|jle|ja|jae|jb|jbe|js|jns|jo|jno|jp|jnp|jecxz|jrcxz|loop[a-z]*)\s+(\.L[A-Za-z0-9_]+)\b")
ALIGN_RE = re.compile(r"\.p2align\s+(\d+)(?:\s*,,\s*(\d+))?")


def compile_s(lccc, opt, tier, src: Path, out: Path):
    env = dict(os.environ)
    if tier is not None:
        env["CCC_LOOP_ALIGN_HOT"] = tier
    r = subprocess.run([str(lccc), INCLUDE, opt, "-S", str(src), "-o", str(out)],
                       capture_output=True, text=True, env=env)
    if r.returncode != 0:
        return r.stderr[-300:]
    return None


def header_groups(lines):
    """Map loop-header label -> ordered directive group above it."""
    label_line = {}
    for i, l in enumerate(lines):
        m = LABEL_RE.match(l.strip())
        if m:
            label_line[m.group(1)] = i
    headers = {}
    for i, l in enumerate(lines):
        m = JMP_RE.match(l)
        if not m:
            continue
        tgt = m.group(1)
        if tgt in label_line and label_line[tgt] < i:
            headers[tgt] = label_line[tgt]
    groups = {}
    for lab, li in headers.items():
        grp = []
        j = li - 1
        while j >= 0 and ".p2align" in lines[j]:
            grp.append(lines[j].strip())
            j -= 1
        groups[lab] = grp[::-1]
    return groups


def strength(grp):
    """Strongest UNCONDITIONAL directive dominates in GAS; else strongest."""
    best = 0
    for g in grp:
        m = ALIGN_RE.search(g)
        if m:
            log2, skip = int(m.group(1)), m.group(2)
            if skip is None:
                return log2  # unconditional dominates everything after
            best = max(best, log2)
    return best


def main():
    lccc = Path(sys.argv[1])
    opt = sys.argv[2] if len(sys.argv) > 2 and sys.argv[2].startswith("-O") else "-O2"
    names = [a for a in sys.argv[2:] if not a.startswith("-O")]
    if not names:
        names = sorted(p.stem for p in PROGRAMS.glob("*.c"))
    tiers = [None, "off", "5", "6", "5skip"]
    width = max(len(n) for n in names)
    for name in names:
        src = PROGRAMS / f"{name}.c"
        if not src.exists():
            continue
        reports = {}
        for tier in tiers:
            out = Path(f"/tmp/census_{name}_{tier or 'default'}.s")
            err = compile_s(lccc, opt, tier, src, out)
            if err:
                reports[tier] = f"ERR {err.strip()[:60]}"
                continue
            groups = header_groups(out.read_text(errors="replace").splitlines())
            counts = {}
            for lab, grp in groups.items():
                s = strength(grp)
                counts[s] = counts.get(s, 0) + 1
            reports[tier] = " ".join(f"2^{k}:{v}" for k, v in sorted(counts.items())) or "no-loops"
        print(f"{name.ljust(width)}  " + " | ".join(f"{str(t) or 'default':>7}={reports[t]}" for t in tiers))


if __name__ == "__main__":
    main()
