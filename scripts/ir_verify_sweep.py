#!/usr/bin/env python3
"""Sweep the regression corpus with the IR verifier and attribute each defect.

`CCC_VERIFY_IR=1` reports a structural violation after *every* pass, so a single
malformed-IR defect is re-reported by every pass that runs after the culprit.
Reading the raw stream therefore tells you almost nothing about who to blame:
the pass with the most lines is usually just the one that runs most often.

This script attributes each (test, optimisation level, violation kind) to the
**first** pass that reported it, which is the pass that produced the bad IR, and
prints a ranked table plus one concrete example per bucket so a fix can start
immediately.

Usage:
    scripts/ir_verify_sweep.py                            # -O2 and -O3, whole corpus
    scripts/ir_verify_sweep.py --levels O0 O1 O2 O3 Os Oz  # explicit levels
    scripts/ir_verify_sweep.py --filter loop_             # substring filter on test name
    scripts/ir_verify_sweep.py --baseline b.json     # compare against a saved run
    scripts/ir_verify_sweep.py --save b.json         # save this run for later diffing

Exit status is 1 if any violation is found (0 when clean), so it can gate CI.
With --baseline it is 1 only on *regressions* relative to that baseline, which
is what you want while a known backlog is being worked through.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

# Violation kind -> substring that identifies it in the verifier's message.
# Ordered most-specific first; the first match wins.
KINDS: list[tuple[str, str]] = [
    ("STALE_PRED", "is not a predecessor"),
    ("MISSING_PRED", "no incoming for predecessor"),
    ("DUP_PRED", "more than once"),
    ("BAD_TARGET", "targets unknown block"),
    ("PHI_ORDER", "after a non-phi instruction"),
    ("DUP_LABEL", "duplicate block label"),
]

STAGE_RE = re.compile(r"after `([^`]*)`")


def classify(line: str) -> str:
    for kind, needle in KINDS:
        if needle in line:
            return kind
    return "OTHER"


def run_one(lccc: str, src: Path, flags: list[str], env_extra: dict, level: str,
            timeout: int) -> list[str]:
    env = dict(os.environ)
    env.update(env_extra)
    env["CCC_VERIFY_IR"] = "1"
    try:
        proc = subprocess.run(
            [lccc, str(src), *flags, level, "-o", os.devnull],
            capture_output=True, text=True, env=env, timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return ["[ir-verify] after `<timeout>`: compile timed out"]
    return [l for l in proc.stderr.splitlines() if "[ir-verify]" in l]


def load_sidecars(src: Path) -> tuple[list[str], dict]:
    """Read the `.flags` / `.env` sidecars the regression runner honours."""
    flags: list[str] = []
    fpath = src.with_suffix(".flags")
    if fpath.exists():
        flags = fpath.read_text().split()
    env_extra: dict = {}
    epath = src.with_suffix(".env")
    if epath.exists():
        for line in epath.read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            k, v = line.split("=", 1)
            env_extra[k.strip()] = v.strip()
    return flags, env_extra


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--levels", nargs="+", default=["-O2", "-O3"], metavar="LEVEL",
                    help="optimisation levels to sweep, with or without the "
                         "leading dash (default: -O2 -O3)")
    ap.add_argument("--lccc", default=os.environ.get("LCCC_BIN", "target/fastbuild/lccc"))
    ap.add_argument("--tests", default="tests/regression")
    ap.add_argument("--filter", default="", help="substring filter on the test name")
    ap.add_argument("--timeout", type=int, default=60)
    ap.add_argument("--save", metavar="JSON", help="write the per-bucket counts here")
    ap.add_argument("--baseline", metavar="JSON",
                    help="compare against a saved run; fail only on regressions")
    ap.add_argument("--examples", type=int, default=1,
                    help="examples to print per bucket (default 1)")
    args = ap.parse_args()

    # argparse would treat a bare `-O2` as an unknown option, so accept both
    # `--levels O2` and `--levels -O2` and normalise here.
    args.levels = [l if l.startswith("-") else f"-{l}" for l in args.levels]

    lccc = args.lccc
    if not Path(lccc).exists():
        print(f"error: compiler not found: {lccc}", file=sys.stderr)
        return 2

    sources = sorted(p for p in Path(args.tests).glob("*.c") if args.filter in p.name)
    if not sources:
        print(f"error: no tests matched {args.filter!r} in {args.tests}", file=sys.stderr)
        return 2

    # bucket -> set of "test -Olevel" configs; bucket -> example lines
    buckets: dict[tuple[str, str], set[str]] = defaultdict(set)
    examples: dict[tuple[str, str], list[str]] = defaultdict(list)
    total_configs = 0
    dirty_configs = 0

    for src in sources:
        flags, env_extra = load_sidecars(src)
        for level in args.levels:
            total_configs += 1
            lines = run_one(lccc, src, flags, env_extra, level, args.timeout)
            if not lines:
                continue
            dirty_configs += 1
            cfg = f"{src.stem} {level}"
            seen: set[str] = set()
            for line in lines:
                kind = classify(line)
                if kind in seen:          # only the FIRST report of each kind
                    continue
                seen.add(kind)
                m = STAGE_RE.search(line)
                stage = m.group(1) if m else "<unknown>"
                key = (kind, stage)
                buckets[key].add(cfg)
                if len(examples[key]) < args.examples:
                    examples[key].append(f"{cfg}\n      {line.strip()}")

    counts = {f"{k}|{s}": len(c) for (k, s), c in buckets.items()}

    print()
    print(f"IR verifier sweep: {len(sources)} tests x {len(args.levels)} levels "
          f"= {total_configs} configs, {dirty_configs} with violations")
    print()
    if not buckets:
        print("  clean - no structural violations found")
    else:
        print(f"  {'KIND':<14}{'FIRST-REPORTING PASS':<34}{'CONFIGS':>8}")
        print("  " + "-" * 56)
        for (kind, stage), cfgs in sorted(buckets.items(), key=lambda kv: -len(kv[1])):
            print(f"  {kind:<14}{stage:<34}{len(cfgs):>8}")
        print()
        print("  examples")
        print("  " + "-" * 56)
        for key in sorted(examples, key=lambda k: -len(buckets[k])):
            for ex in examples[key]:
                print(f"    [{key[0]} / {key[1]}] {ex}")

    if args.save:
        Path(args.save).write_text(json.dumps(counts, indent=2, sort_keys=True) + "\n")
        print(f"\n  saved -> {args.save}")

    if args.baseline:
        base = json.loads(Path(args.baseline).read_text())
        regressions = {k: (base.get(k, 0), v) for k, v in counts.items()
                       if v > base.get(k, 0)}
        fixed = {k: (v, counts.get(k, 0)) for k, v in base.items()
                 if counts.get(k, 0) < v}
        print()
        for k, (was, now) in sorted(fixed.items()):
            print(f"  IMPROVED  {k}: {was} -> {now}")
        for k, (was, now) in sorted(regressions.items()):
            print(f"  REGRESSED {k}: {was} -> {now}")
        if regressions:
            print(f"\n  FAIL: {len(regressions)} bucket(s) regressed")
            return 1
        print("\n  OK: no regressions against baseline")
        return 0

    return 1 if buckets else 0


if __name__ == "__main__":
    sys.exit(main())
