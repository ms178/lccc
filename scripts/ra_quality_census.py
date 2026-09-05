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

Function boundaries come from `.type NAME,@function`, not from every assembly
label: `.L*` basic-block labels are part of their enclosing function.  This is
important for LCCC, whose first `.LBB*` label otherwise used to truncate a
function's count, and for benchmark drivers where hot static helpers inline
into `main`.

Usage:
    scripts/ra_quality_census.py                  # benchmark programs + kernel corpus
    scripts/ra_quality_census.py FILE.c ...       # explicit files
    scripts/ra_quality_census.py --include-main   # include inlined benchmark drivers
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
DEFAULT_DIRS = [
    REPO / "tests" / "benchmark" / "programs",
    REPO / "tests" / "benchmark" / "kernel_corpus",
]
LCCC = os.environ.get("LCCC", str(REPO / "target" / "fastbuild" / "lccc"))
GCC = os.environ.get("GCC", "gcc")
CLANG = os.environ.get("CLANG", "clang")
EXTRA = os.environ.get("CFLAGS", "").split()
BUCKETS = ("insns", "rrmov", "stkref", "push", "acc")

_SYMBOL = r"[A-Za-z_$][\w.$@]*"
_LABEL = re.compile(rf"^({_SYMBOL}):")
_TYPE_FUNCTION = re.compile(rf"^\s*\.type\s+({_SYMBOL})\s*,\s*@function\b")
_SIZE_FUNCTION = re.compile(rf"^\s*\.size\s+({_SYMBOL})\s*,")
_DIRECTIVE = re.compile(r"^\s*\.")
_REG_REG_MOV = re.compile(r"^\s*mov[lqwb]?\s+%[a-z0-9]+\s*,\s*%[a-z0-9]+\s*$")
_STKREF = re.compile(r"\(%(?:rsp|rbp|esp|ebp)\b")
_PUSH = re.compile(r"^\s*push[lq]?\s+%")
_ACC = re.compile(r"^\s*(cltq|cwtl|cqto|cltd|cbtw|cwtd|cdqe)\b")


def compile_to_asm(cc: str, src: Path, flags: list[str]) -> str | None:
    fd, out = tempfile.mkstemp(suffix=".s")
    os.close(fd)
    try:
        proc = subprocess.run(
            [cc, *flags, "-S", "-o", out, str(src)],
            capture_output=True,
            text=True,
            timeout=180,
        )
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


def is_local_label(name: str) -> bool:
    """Whether `name` is an assembler-local control-flow label."""
    return name.startswith((".L", "$L"))


def census(asm: str) -> dict[str, dict[str, int]]:
    """Count complete function bodies without mistaking basic-block labels for functions."""
    funcs: dict[str, dict[str, int]] = {}
    counts: dict[str, int] | None = None
    current_name: str | None = None
    pending_function: str | None = None
    in_text = True
    for raw in asm.splitlines():
        line = raw.split("#", 1)[0].rstrip()
        s = line.strip()
        if not s:
            continue
        if s.startswith((".section", ".data", ".bss", ".rodata")):
            in_text = ".text" in s
            if not in_text:
                counts = None
                current_name = None
            continue
        if s == ".text":
            in_text = True
            continue

        type_match = _TYPE_FUNCTION.match(line)
        if type_match:
            pending_function = type_match.group(1)
            continue
        size_match = _SIZE_FUNCTION.match(line)
        if size_match and size_match.group(1) == current_name:
            counts = None
            current_name = None
            continue

        label_match = _LABEL.match(line)
        if label_match:
            label = label_match.group(1)
            if in_text and (label == pending_function or (pending_function is None and not is_local_label(label))):
                current_name = label
                counts = funcs.setdefault(label, dict.fromkeys(BUCKETS, 0))
                if label == pending_function:
                    pending_function = None
            # Any other label is an in-function/basic-block/data label.  It
            # must not reset `counts`; doing so silently dropped most bodies.
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
    ap.add_argument(
        "--include-main",
        action="store_true",
        help="include main; useful when benchmark helpers inline into its hot loop",
    )
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
            if fn == "main" and not args.include_main:
                continue
            row = {n: per_cc[n].get(fn, dict.fromkeys(BUCKETS, 0)) for n in names}
            table.append((src.name, fn, row))
            for name in names:
                for bucket in BUCKETS:
                    totals[name][bucket] += row[name][bucket]

    table.sort(key=lambda item: item[2]["lccc"]["insns"] - item[2]["gcc"]["insns"], reverse=True)
    rows = table[: args.top] if args.top else table
    tag = "/".join(names)
    header = f"{'file':<26} {'function':<24} " + " ".join(
        f"{bucket + ' ' + tag:>20}" for bucket in BUCKETS
    )
    print(header)
    print("-" * len(header))
    for filename, function, row in rows:
        print(
            f"{filename[:26]:<26} {function[:24]:<24} "
            + " ".join(
                f"{'/'.join(str(row[name][bucket]) for name in names):>20}"
                for bucket in BUCKETS
            )
        )
    print("-" * len(header))
    print(
        f"{'TOTAL':<26} {str(len(table)) + ' fns':<24} "
        + " ".join(
            f"{'/'.join(str(totals[name][bucket]) for name in names):>20}"
            for bucket in BUCKETS
        )
    )
    if skipped:
        print("skipped:", ", ".join(skipped))
    if args.json:
        args.json.write_text(
            json.dumps(
                {
                    "flags": flags,
                    "compilers": dict(compilers),
                    "include_main": args.include_main,
                    "totals": totals,
                    "skipped": skipped,
                    "functions": [
                        {"file": filename, "fn": function, **{name: row[name] for name in names}}
                        for filename, function, row in table
                    ],
                },
                indent=2,
            )
            + "\n"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
