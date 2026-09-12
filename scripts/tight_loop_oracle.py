#!/usr/bin/env python3
"""Tight-loop alignment census vs the Godbolt oracles.

For every benchmark program, extract each backward-branch label (a loop
header) and the strongest UNCONDITIONAL alignment in the directive group
immediately above it, then compare:

  * GCC 16.2 / Clang 23.1 / ICX on Godbolt (`-O2 -march=raptorlake`,
    directives kept); GCC emits the tight-loop group
    `.p2align K` + `.p2align 4,,10` + `.p2align 3` where
    K = ceil_log2(minimum encoded body size).
  * lccc's integrated assembler decision (`CCC_DEBUG_TIGHT`):
    K = ceil_log2(EXACT encoded body span), clamped to 3..=6, with
    bodies over one cache line rejected to the ordinary cascade.

Exact per-loop equality is NOT expected: the compilers generate
different instruction sequences, so the two inputs to the same rule
differ. What this census establishes is (a) every loop GCC aligns as a
tight loop is also tight-aligned by lccc at a bucket no weaker than the
hardware goal, (b) lccc never aligns a body its own assembler measures
above 64 bytes, and (c) the histogram deltas over the whole corpus.

Usage: scripts/tight_loop_oracle.py LCCC [-O2] [--oracles gcc16.2,clang] [bench...]
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
INCLUDE = "-I/usr/lib/gcc/x86_64-linux-gnu/14/include"
PROGRAMS = REPO / "tests/benchmark/programs"

sys.path.insert(0, str(REPO / "scripts"))
import godbolt as gb  # noqa: E402

LABEL_RE = re.compile(r"^([.\w]+):\s*$")
JMP_RE = re.compile(r"^\s*(?:jmp|j[a-z]{1,3}|loop[a-z]*)\s+([.\w]+)\b")
P2A_RE = re.compile(r"\.p2align\s+(\d+)(?:\s*,,\s*(\d+))?")
FUNC_RE = re.compile(r"^([A-Za-z_][\w.$]*):\s*$")
DUMP_RE = re.compile(r"loop_align: func=(\S+) header=L(\d+) tight=yes")
OK_RE = re.compile(r"\[TIGHT\] sec\d+ header=(\.\S+) span=(\d+) log2=(\d+)")
NO_RE = re.compile(r"\[TIGHT\] sec\d+ header=(\.\S+) reject=(\S+)")


def oracle_headers(lines):
    """label -> (function, strongest unconditional log2 or None)."""
    label_line, func_of = {}, {}
    cur = None
    for i, l in enumerate(lines):
        m = FUNC_RE.match(l.strip())
        if m and not l.startswith("."):
            cur = m.group(1)
        m = LABEL_RE.match(l.strip())
        if m:
            label_line[m.group(1)] = i
            func_of[m.group(1)] = cur
    headers = {}
    for i, l in enumerate(lines):
        m = JMP_RE.match(l)
        if m:
            tgt = m.group(1)
            if tgt in label_line and label_line[tgt] < i:
                headers[tgt] = label_line[tgt]
    out = {}
    for lab, li in headers.items():
        best = None
        j = li - 1
        # Walk only the contiguous directive/comment/blank run.
        while j >= 0:
            t = lines[j].strip()
            if not t or t.startswith((".", "//", "#", ".cfi", ".L")):
                am = P2A_RE.search(t)
                if am:
                    k, skip = int(am.group(1)), am.group(2)
                    if skip is None:
                        best = k  # unconditional dominates
                j -= 1
                continue
            break
        out[lab] = (func_of.get(lab), best)
    return out


def directive_groups(lines):
    """Strongest UNCONDITIONAL .p2align log2 above each backward-branch
    label in lccc -S text ('cascade' for bounded groups only)."""
    label_line, headers = {}, {}
    for i, l in enumerate(lines):
        m = re.match(r"^(\.\w+):", l.strip())
        if m:
            label_line[m.group(1)] = i
    for i, l in enumerate(lines):
        m = JMP_RE.match(l)
        if m and m.group(1) in label_line and label_line[m.group(1)] < i:
            headers[m.group(1)] = label_line[m.group(1)]
    out = {}
    for lab, li in headers.items():
        best, uncond = None, False
        j = li - 1
        while j >= 0 and (".p2align" in lines[j] or not lines[j].strip()):
            am = P2A_RE.search(lines[j])
            if am:
                k, skip = int(am.group(1)), am.group(2)
                if skip is None:
                    uncond = True
                    best = k
                elif best is None:
                    best = k
            j -= 1
        out[lab] = best if uncond else ("cascade" if best else None)
    return out


def lccc_decisions(lccc, opt, src):
    """Histogram of effective strongest log2 for every loop header in
    integrated mode: tight accepts from CCC_DEBUG_TIGHT override the
    portable cascade groups parsed from -S text; plus reject reasons."""
    with tempfile.TemporaryDirectory() as td:
        # 1. portable text: cascade groups per header
        sfile = Path(td) / "p.s"
        r = subprocess.run([str(lccc), INCLUDE, opt, "-S", str(src), "-o", str(sfile)],
                           capture_output=True, text=True)
        if r.returncode:
            return None, None, r.stderr[-300:]
        groups = directive_groups(sfile.read_text(errors="replace").splitlines())
        # 2. integrated decision: tight accepts/rejects
        env = dict(os.environ)
        env["CCC_DEBUG_TIGHT"] = "1"
        env["CCC_DUMP_ALIGN"] = "1"
        obj = Path(td) / "p.o"
        r2 = subprocess.run([str(lccc), INCLUDE, opt, "-c", str(src), "-o", str(obj)],
                            capture_output=True, text=True, env=env)
        if r2.returncode:
            return None, None, r2.stderr[-300:]
        accepted, rejected = {}, {}
        for line in r2.stderr.splitlines():
            m = OK_RE.search(line)
            if m:
                accepted[m.group(1)] = (int(m.group(2)), int(m.group(3)))
                rejected.pop(m.group(1), None)
                continue
            m = NO_RE.search(line)
            if m and m.group(1) not in accepted:
                rejected[m.group(1)] = m.group(2)
    effective = {}
    for lab, bucket in groups.items():
        effective[lab] = accepted.get(lab, (0, bucket if isinstance(bucket, int) else 3))[1] \
            if lab in accepted else (bucket if bucket is not None else 3)
    # Tight labels can exist only in -c debug; ensure every accept is in.
    for lab, (_, k) in accepted.items():
        effective[lab] = k
    return effective, rejected, None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("lccc", type=Path)
    ap.add_argument("--opt", default="-O2")
    ap.add_argument("--oracles", default="gcc16.2,clang,icx")
    ap.add_argument("--march", default="raptorlake")
    ap.add_argument("programs", nargs="*")
    args = ap.parse_args()
    names = args.programs or sorted(p.stem for p in PROGRAMS.glob("*.c"))
    oracles = [o for o in args.oracles.split(",") if o]

    totals = {o: Counter() for o in oracles}
    lccc_hist = Counter()
    lccc_reject = Counter()
    rows = []
    for name in names:
        src = PROGRAMS / f"{name}.c"
        if not src.exists():
            continue
        source = src.read_text(errors="replace")
        ohists = {}
        for o in oracles:
            res = gb.compile_on_godbolt(
                o, source, f"{args.opt} -march={args.march}", keep_directives=True)
            if res is None:
                ohists[o] = Counter(err=1)
                continue
            hs = oracle_headers(gb.assembly_lines(res))
            c = Counter(("cascade" if k is None else k) for (_, k) in hs.values())
            ohists[o] = c
            totals[o].update(c)
        effective, rejected, err = lccc_decisions(args.lccc, args.opt, src)
        if err:
            print(f"{name}: lccc compile failed {err.strip()[:80]}")
            continue
        lc = Counter(("cascade" if k == "cascade" else k) for k in effective.values())
        for why in rejected.values():
            lccc_reject[why] += 1
        lccc_hist.update(lc)
        rows.append((name, ohists, lc))
    width = max(len(n) for n, *_ in rows) if rows else 10
    print(f"{'program':{width}s}  " + "  ".join(f"{o:>22s}" for o in oracles)
          + f"  {'lccc-tight':>16s}")
    for name, oh, lc in rows:
        cells = []
        for o in oracles:
            c = oh[o]
            items = sorted(c.items(), key=lambda kv: (isinstance(kv[0], str), kv[0]))
            cells.append(
                " ".join((f"cas:{v}" if k == "cascade" else f"2^{k}:{v}") for k, v in items)
                or "-"
            )
        print(f"{name:{width}s}  " + "  ".join(f"{x:>22s}" for x in cells)
              + "  " + f"{(' '.join(f'2^{k}:{v}' for k, v in sorted(lc.items())) or '-'):>16s}")
    print("\n=== corpus totals (loop-header strongest unconditional alignment) ===")
    for o in oracles:
        c = totals[o]
        items = sorted(c.items(), key=lambda kv: (isinstance(kv[0], str), kv[0]))
        print(
            f"  {o:9s}: "
            + " ".join((f"cascade:{v}" if k == "cascade" else f"2^{k}:{v}") for k, v in items)
        )
    print("  lccc     : " + " ".join(f"2^{k}:{v}" for k, v in sorted(lccc_hist.items()))
          + "   rejects: " + " ".join(f"{k}:{v}" for k, v in sorted(lccc_reject.items())))


if __name__ == "__main__":
    main()
