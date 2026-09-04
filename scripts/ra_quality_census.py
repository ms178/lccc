#!/usr/bin/env python3
"""Register-allocation quality census: LCCC vs GCC vs Clang, per function.

`kernel_count.py` reports one number (instructions).  Register-allocator work
needs the *shape* of the loss, so this tool classifies every instruction of
every function in a corpus into RA-relevant buckets and prints per-function
and per-bucket columns (lccc/gcc/clang) sorted by the instruction delta:

    insns    all instructions in the function body
    rrmov    register-to-register `mov*` (coalescing failures / relays)
    stkref   memory operands through %rsp/%rbp/%esp/%ebp (spill traffic)
    push     `push %reg` (callee-saved pressure)
    acc      accumulator-pinned forms (cltq/cwtl/cqto/cltd/...)

Usage:
    scripts/ra_quality_census.py                  # benchmark programs + kernel corpus
    scripts/ra_quality_census.py FILE.c ...       # explicit files
    scripts/ra_quality_census.py --m32            # i686 (needs gcc-multilib)
    scripts/ra_quality_census.py --json out.json  # machine-readable
    scripts/ra_quality_census.py --top 15         # 15 worst functions only

Environment:
    LCCC / GCC / CLANG   compiler paths (defaults: target/fastbuild/lccc, gcc, clang)
    CFLAGS               extra flags appended for every compiler

A compiler failure on one file skips that file and is listed at the end; the
census never aborts on oracle-only or target-specific programs.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_DIRS = [REPO / "tests" / "benchmark" / "programs", REPO / "tests" / "benchmark" / "kernel_corpus"]
LCCC = os.environ.get("LCCC", str(REPO / "target" / "fastbuild" / "lccc"))
GCC = os.environ.get("GCC", "gcc")
CLANG = os.environ.get("CLANG", "clang")
EXTRA = os.environ.get("CFLAGS", "").split()
BUCKETS = ("insns", "rrmov", "stkref", "push", "acc")

_LABEL = re.compile(r"^([A-Za-z_$][\w.$@]*):")
_DIRECTIVE = re.compile(r"^\s*\.")
_REG_REG_MOV = re.compile(r"^\s*mov[lqwb]?\s+%[a-z0-9]+\s*,\s*%[a-z0-9]+\s*$")
_STKREF = re.compile(r"\(%(?:rsp|rbp|esp|ebp)\b")
_PUSH = re.compile(r"^\s*push[lq]?\s+%")
_ACC = re.compile(r"^\s*(cltq|cwtl|cqto|cltd|cbtw|cwtd|cdqe)\b")


def compile_to_asm(cc: str, src: Path, flags: list[str]) -> str | None:
    fd, out = tempfile.mkstemp(suffix=".s")
    os.close(fd)
    try:
        proc = subprocess.run([cc, *flags, "-S", "-o", out, str(src)], capture_output=True, text=True, timeout=180)
        if proc.returncode != 0:
            return None
        return Path(out).read_text(errors="replace")
    except (subprocess.TimeoutExpired, OSError):
        return None
    finally:
        try:
            os.unlink(out)
        except OSError:
            pass


def census(asm: str) -> dict[str, dict[str, int]]:
    funcs: dict[str, dict[str, int]] = {}
    counts: dict[str, int] | None = None
    in_text = True
    for raw in asm.splitlines():
        line = raw.split("#", 1)[0].rstrip()
        s = line.strip()
        if not s:
            continue
        if s.startswith((".section", ".data", ".bss", ".rodata")):
            in_text = ".text" in s
            continue
        if s == ".text":
            in_text = True
            continue
        m = _LABEL.match(line)
        if m:
            if in_text:
                counts = funcs.setdefault(m.group(1), dict.fromkeys(BUCKETS, 0))
            continue
        if _DIRECTIVE.match(line) or counts is None or not in_text:
            continue
        counts["insns"] += 1
        counts["rrmov"] += bool(_REG_REG_MOV.match(line))
        counts["stkref"] += bool(_STKREF.search(line))
        counts["push"] += bool(_PUSH.match(line))
        counts["acc"] += bool(_ACC.match(line))
    return funcs


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="*", type=Path)
    ap.add_argument("--m32", action="store_true")
    ap.add_argument("--O", default="-O2")
    ap.add_argument("--json", type=Path)
    ap.add_argument("--top", type=int, default=0)
    ap.add_argument("--no-clang", action="store_true")
    args = ap.parse_args()

    files = list(args.files) or [f for d in DEFAULT_DIRS for f in sorted(d.glob("*.c"))]
    flags = [args.O, *EXTRA] + (["-m32"] if args.m32 else [])
    compilers = [("lccc", LCCC), ("gcc", GCC)]
    if not args.no_clang and shutil.which(CLANG):
        compilers.append(("clang", CLANG))
    names = [n for n, _ in compilers]

    table = []
    totals = {n: dict.fromkeys(BUCKETS, 0) for n in names}
    skipped = []
    for src in files:
        per_cc = {}
        for name, cc in compilers:
            asm = compile_to_asm(cc, src, flags)
            if asm is None:
                skipped.append(f"{src.name}({name})")
                break
            per_cc[name] = census(asm)
        if len(per_cc) != len(compilers):
            continue
        for fn in sorted(set(per_cc["lccc"]) & set(per_cc["gcc"])):
            if fn == "main":
                continue
            row = {n: per_cc[n].get(fn, dict.fromkeys(BUCKETS, 0)) for n in names}
            table.append((src.name, fn, row))
            for n in names:
                for b in BUCKETS:
                    totals[n][b] += row[n][b]

    table.sort(key=lambda t: t[2]["lccc"]["insns"] - t[2]["gcc"]["insns"], reverse=True)
    rows = table[: args.top] if args.top else table
    tag = "/".join(names)
    hdr = f"{'file':<26} {'function':<24} " + " ".join(f"{b + ' ' + tag:>20}" for b in BUCKETS)
    print(hdr)
    print("-" * len(hdr))
    for fname, fn, row in rows:
        print(f"{fname[:26]:<26} {fn[:24]:<24} " + " ".join(f"{'/'.join(str(row[n][b]) for n in names):>20}" for b in BUCKETS))
    print("-" * len(hdr))
    print(f"{'TOTAL':<26} {str(len(table)) + ' fns':<24} " + " ".join(f"{'/'.join(str(totals[n][b]) for n in names):>20}" for b in BUCKETS))
    if skipped:
        print("skipped:", ", ".join(skipped))
    if args.json:
        args.json.write_text(json.dumps({"flags": flags, "compilers": dict(compilers), "totals": totals, "skipped": skipped,
            "functions": [{"file": f, "fn": fn, **{n: row[n] for n in names}} for f, fn, row in table]}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
