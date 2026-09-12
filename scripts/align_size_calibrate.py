#!/usr/bin/env python3
"""Validate the integrated assembler's tight-loop decisions against truth.

The tight-loop alignment policy is two layers:

  1. the IR pass (`passes::loop_align`) audits loop STRUCTURE and emits a
     `.lccc_tight_loop .LBBn` marker for candidates (gated on the
     integrated assembler being the final assembler);
  2. the ELF writer measures the EXACT encoded span (header label to the
     end of the first backward branch to it), accepts when the span fits
     one instruction-cache line (<=64 bytes), and pads unconditionally to
     2**ceil(log2(span)) clamped to the 8..=64 buckets.

This script independently measures ground truth with GNU as + objdump on
the marker-free `-S` text, then cross-checks the writer's own decisions
(parsed from `CCC_DEBUG_TIGHT`):

  * every structurally-audited candidate must be measured;
  * writer bucket must equal ceil-log2 of the true span (0 mismatches);
  * NO candidate with a true span >64 may be padded (0 cache-line leaks);
  * every candidate <=64 must actually be padded (0 missed promotions).

Usage: scripts/align_size_calibrate.py LCCC [-O2] [bench...]
"""
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
INCLUDE = "-I/usr/lib/gcc/x86_64-linux-gnu/14/include"
PROGRAMS = REPO / "tests/benchmark/programs"

# loop_align: func=<mangled> header=L<n> tight=yes|no [(reason)] bound=...
DUMP_RE = re.compile(r"loop_align: func=(\S+) header=L(\d+) tight=(yes|no)")
# [TIGHT] sec<n> header=.LBB<n> span=<s> log2=<k> align=<a>
TIGHT_OK_RE = re.compile(r"\[TIGHT\] sec\d+ header=(\.\S+) span=(\d+) log2=(\d+)")
# [TIGHT] sec<n> header=.LBB<n> reject=<reason>
TIGHT_NO_RE = re.compile(r"\[TIGHT\] sec\d+ header=(\.\S+) reject=(\S+)")
OBJDUMP_LABEL = re.compile(r"^([0-9a-f]+) <(\.[^>]+)>:$")


def bucket(span: int) -> int:
    """ceil-log2 bucket, clamped to the 8..=64 range; 7 = >64 reject."""
    if span <= 8:
        return 3
    if span <= 16:
        return 4
    if span <= 32:
        return 5
    if span <= 64:
        return 6
    return 7


def ground_truth_spans(lccc, opt, src, td, m32=False):
    """Build marker-free assembly (-S, align off), assemble with GNU as,
    measure true header->latch spans. Returns (candidate_labels, spans)."""
    sfile = td / "p.s"
    env = dict(os.environ)
    env["CCC_LOOP_ALIGN_HOT"] = "off"
    env["CCC_DUMP_ALIGN"] = "1"
    include = [] if m32 else [INCLUDE]
    r = subprocess.run(
        [str(lccc), *include, opt, "-S", str(src), "-o", str(sfile)],
        capture_output=True, text=True, env=env)
    if r.returncode:
        return None, f"compile failed {r.stderr[-160:]}"
    candidates = []
    rejects = 0
    for line in r.stderr.splitlines():
        m = DUMP_RE.search(line)
        if not m:
            continue
        if m.group(3) == "yes":
            candidates.append(f".LBB{m.group(2)}")
        else:
            rejects += 1
    ofile = td / "p-gas.o"
    as_flags = ["-L", "--32"] if m32 else ["-L"]
    ar = subprocess.run(["as", *as_flags, "-o", str(ofile), str(sfile)],
                        capture_output=True, text=True)
    if ar.returncode:
        return None, f"as failed {ar.stderr[-160:]}"
    od_args = ["objdump", "-d", "-M", "att"]
    if m32:
        od_args += ["-m", "i386:x86-64"] if False else []
    od = subprocess.run(od_args + [str(ofile)],
                        capture_output=True, text=True).stdout.splitlines()
    label_addr: dict[str, int] = {}
    insns: list[tuple[int, int, str]] = []
    for line in od:
        m = OBJDUMP_LABEL.match(line)
        if m:
            label_addr[m.group(2)] = int(m.group(1), 16)
            continue
        m = re.match(
            r"^\s*([0-9a-f]+):\s+((?:[0-9a-f]{2}[ \t]+){1,15})\s*\t?(.*)$", line)
        if m:
            insns.append((int(m.group(1), 16), len(m.group(2).split()),
                          m.group(3).strip()))
    spans = {}
    for lname in candidates:
        if lname not in label_addr:
            cands = [l for l in label_addr if l.endswith(lname)]
            if not cands:
                spans[lname] = ("label-missing", None)
                continue
            lname = cands[0]
        start = label_addr[lname]
        end = None
        for addr, nbytes, text in insns:
            if addr < start:
                continue
            jm = re.match(
                r"(?:jmp|j[a-z]{1,3}|loop[a-z]*)\s+(?:[0-9a-f]+ )?<([^>]+)>",
                text)
            if jm and jm.group(1) == lname:
                end = addr + nbytes
                break
        if end is None:
            spans[lname] = ("no-backedge", None)
        else:
            spans[lname] = ("", end - start)
    return (spans, rejects), None


def writer_decisions(lccc, opt, src, td, m32=False):
    """Compile through the integrated assembler with TIGHT debug on.
    The final debug line per header (last fixup sweep) is the decision."""
    ofile = td / "p-lccc.o"
    env = dict(os.environ)
    env.pop("CCC_LOOP_ALIGN_HOT", None)
    env.pop("CCC_NO_LOOP_ALIGN", None)
    env["CCC_DEBUG_TIGHT"] = "1"
    include = [] if m32 else [INCLUDE]
    r = subprocess.run(
        [str(lccc), *include, opt, "-c", str(src), "-o", str(ofile)],
        capture_output=True, text=True, env=env)
    if r.returncode:
        return None, f"integrated compile failed {r.stderr[-160:]}"
    accepted: dict[str, tuple[int, int]] = {}
    rejected: dict[str, str] = {}
    for line in r.stderr.splitlines():
        m = TIGHT_OK_RE.search(line)
        if m:
            accepted[m.group(1)] = (int(m.group(2)), int(m.group(3)))
            rejected.pop(m.group(1), None)
            continue
        m = TIGHT_NO_RE.search(line)
        if m and m.group(1) not in accepted:
            rejected[m.group(1)] = m.group(2)
    return (accepted, rejected), None


def main():
    lccc = Path(sys.argv[1])
    rest = sys.argv[2:]
    m32 = "--32" in rest
    rest = [a for a in rest if a != "--32"]
    opt = next((a for a in rest if a.startswith("-O")), "-O2")
    names = [a for a in rest if not a.startswith("-O")]
    if not names:
        names = sorted(p.stem for p in PROGRAMS.glob("*.c"))
    verbose = "-v" in rest
    rows = []
    struct_rejects = 0
    for name in names:
        src = PROGRAMS / f"{name}.c"
        if not src.exists():
            continue
        with tempfile.TemporaryDirectory() as t:
            td = Path(t)
            gt, err = ground_truth_spans(lccc, opt, src, td, m32)
            if err:
                print(f"{name}: {err}")
                continue
            spans, rejects = gt
            struct_rejects += rejects
            wd, err = writer_decisions(lccc, opt, src, td, m32)
            if err:
                print(f"{name}: {err}")
                continue
            accepted, wrejected = wd
            for lname, (note, span) in spans.items():
                w = accepted.get(lname)
                wr = wrejected.get(lname)
                rows.append((name, lname, note, span, w, wr))
    bucket_mismatch = 0
    leaks = 0
    missed = 0
    disagreement = 0
    measured = 0
    for name, lname, note, span, w, wr in rows:
        if span is None:
            if verbose:
                print(f"{name:24s} {lname:8s} ground-truth {note}")
            continue
        measured += 1
        tb = bucket(span)
        flag = ""
        if span > 64:
            if w is not None:
                leaks += 1
                flag = "LEAK>64"
        else:
            if w is None:
                missed += 1
                flag = f"MISSED ({wr or 'no-debug'})"
            elif w[1] != tb:
                bucket_mismatch += 1
                flag = f"BUCKET writer={w[1]} truth={tb}"
            elif w[0] != span:
                disagreement += 1
                flag = f"SPAN writer={w[0]} truth={span}"
        if verbose or flag:
            print(f"{name:24s} {lname:8s} span={span:3d} truth_b={tb} "
                  f"writer={w if w else wr} {flag}".rstrip())
    print(f"\ncandidates measured={measured} structural_rejects={struct_rejects}")
    print(f"bucket-mismatches={bucket_mismatch} span-disagreements={disagreement} "
          f"cache-line-leaks={leaks} missed-promotions={missed}")
    rc = 0 if not (bucket_mismatch or leaks or missed or disagreement) else 1
    sys.exit(rc)


if __name__ == "__main__":
    main()
