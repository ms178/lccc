#!/usr/bin/env python3
"""Whole-corpus census of the loop-inversion pass.

`loop_invert` runs after phi elimination and rotates top-test loops.  Whether a
loop is rotated, and -- when it is refused -- *why*, is only observable through
the pass's own `CCC_DEBUG_LOOP_INVERT` diagnostics, which are emitted during
compilation and thrown away by every other tool in the tree.  This script
turns them into a table.

For every translation unit in the corpus it compiles with `-c` and the debug
flag, then parses the pass's stderr into three record kinds:

    [INV] rotating header=<H> latch=<T>
    [INV] header <H> has a non-duplicable instruction
    [INV] header <H> defines v<V>, used in block <B> (in loop|outside loop)

and reports, per TU and in aggregate:

  * rotations, split by licence (pure vs memory) when the pass reports one
    (the shipped tree reports a rotation as a plain `rotating` line; a memory
    licence, if present, is spelled `rotating header=.. latch=.. (memory)`);
  * refusals by rule, with the escape cases broken down by whether the using
    block is inside the loop or outside it -- the two need different
    compensation, so they are not the same defect.

The corpus is the tree's own: the bench kernels (`tests/bench/k_*.c`), the
program corpus (`tests/benchmark/programs/*.c`) and the kernel corpus
(`tests/benchmark/kernel_corpus/*.c`).  Pass explicit paths to override.

Usage
    scripts/loop_invert_census.py --lccc target/fastbuild/lccc
    scripts/loop_invert_census.py --lccc target/fastbuild/lccc --opt=-O2 \
        --json /tmp/census.json tests/bench/k_*.c
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROTATE = re.compile(r"\[INV\] rotating header=(\d+) latch=(\d+)(.*)$")
NON_DUP = re.compile(r"\[INV\] header (\d+) has a non-duplicable instruction")
ESCAPE = re.compile(
    r"\[INV\] header (\d+) defines v(\d+), used in block (\d+) \((in loop|outside loop)\)"
)
MULTI_LATCH = re.compile(r"\[INV\] header (\d+) has (\d+) latches")
OVERSIZED = re.compile(r"\[INV\] header (\d+) is oversized \((\d+) insns")
NO_EXIT_TEST = re.compile(r"\[INV\] header (\d+) does not decide the exit")
LICENCE_OFF = re.compile(r"\[INV\] header (\d+) walks memory, licence not taken")


def default_corpus(repo: Path) -> list[Path]:
    files = sorted((repo / "tests/bench").glob("k_*.c"))
    files += sorted((repo / "tests/benchmark/programs").glob("*.c"))
    files += sorted((repo / "tests/benchmark/kernel_corpus").glob("*.c"))
    return files


def compile_one(lccc: Path, repo: Path, src: Path, opt: str, extra: list[str]) -> str:
    env = dict(os.environ)
    env["CCC_DEBUG_LOOP_INVERT"] = "1"
    cmd = [str(lccc), opt, "-c", "-w", "-I", str(repo / "tests/bench"), *extra,
           str(src), "-o", os.devnull]
    r = subprocess.run(cmd, capture_output=True, text=True, env=env, cwd=repo)
    if r.returncode != 0:
        return f"!! compile failed: {r.stderr.strip().splitlines()[-1] if r.stderr else ''}"
    return r.stderr


def classify(log: str) -> dict:
    """Fold one TU's diagnostic log into counter buckets."""
    pure_rot = memory_rot = 0
    non_dup = 0
    escape_in = escape_out = 0
    multi_latch = 0
    oversized = 0
    no_exit_test = 0
    licence_off = 0
    for line in log.splitlines():
        if (m := ROTATE.match(line.strip())):
            if "memory" in m.group(3):
                memory_rot += 1
            else:
                pure_rot += 1
        elif NON_DUP.match(line.strip()):
            non_dup += 1
        elif (m := ESCAPE.match(line.strip())):
            if m.group(4) == "in loop":
                escape_in += 1
            else:
                escape_out += 1
        elif MULTI_LATCH.match(line.strip()):
            multi_latch += 1
        elif OVERSIZED.match(line.strip()):
            oversized += 1
        elif NO_EXIT_TEST.match(line.strip()):
            no_exit_test += 1
        elif LICENCE_OFF.match(line.strip()):
            licence_off += 1
    return {
        "rotations": pure_rot + memory_rot,
        "rotations_memory": memory_rot,
        "refused_non_duplicable": non_dup,
        "refused_escape_in_loop": escape_in,
        "refused_escape_outside_loop": escape_out,
        "refused_multi_latch": multi_latch,
        "refused_oversized": oversized,
        "refused_no_exit_test": no_exit_test,
        "refused_memory_licence_off": licence_off,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lccc", default="target/fastbuild/lccc")
    ap.add_argument("--repo", default=str(Path(__file__).resolve().parents[1]))
    ap.add_argument("--opt", default="-O2",
                    help="optimisation level; spell it as --opt=-O3 (argparse reads a"
                         " bare -O3 as a flag)")
    ap.add_argument("--extra", default="", help="extra flags, space separated")
    ap.add_argument("--json", help="write the per-TU records here")
    ap.add_argument("--quiet", action="store_true", help="aggregate only")
    ap.add_argument("paths", nargs="*", help="TUs to census (default: the corpus)")
    # parse_intermixed_args, not parse_args: a `nargs="*"` positional parsed by
    # plain parse_args is starved by an optional that precedes it on CPython
    # 3.12.3 (the hosted runner) -- pinned by
    # tests/corpus/test_callgrind_contracts.py's star-positional ratchet.
    a = ap.parse_intermixed_args()

    repo = Path(a.repo).resolve()
    lccc = Path(a.lccc)
    if not lccc.is_absolute():
        lccc = repo / lccc
    if not lccc.exists():
        sys.exit(f"census: no compiler at {lccc}")
    srcs = [Path(p) for p in a.paths] or default_corpus(repo)
    if not srcs:
        sys.exit("census: empty corpus")

    records = []
    totals = {"rotations": 0, "rotations_memory": 0, "refused_non_duplicable": 0,
              "refused_escape_in_loop": 0, "refused_escape_outside_loop": 0,
              "refused_multi_latch": 0, "refused_oversized": 0,
              "refused_no_exit_test": 0,
              "refused_memory_licence_off": 0}
    failed = 0
    for src in srcs:
        log = compile_one(lccc, repo, src, a.opt, [f for f in a.extra.split() if f])
        if log.startswith("!!"):
            failed += 1
            if not a.quiet:
                print(f"{log}  {src}")
            continue
        rec = classify(log)
        rec["file"] = str(src.relative_to(repo) if src.is_absolute() and repo in src.parents else src)
        records.append(rec)
        interesting = any(v for k, v in rec.items() if k != "file" and k != "rotations")
        if not a.quiet and (interesting or rec["rotations"]):
            print(f"{rec['file']:<58} rot={rec['rotations']:>3}"
                  f"{' (mem ' + str(rec['rotations_memory']) + ')' if rec['rotations_memory'] else '    '}"
                  f"  nondup={rec['refused_non_duplicable']:>2}"
                  f"  esc_in={rec['refused_escape_in_loop']:>2}"
                  f"  esc_out={rec['refused_escape_outside_loop']:>2}"
                  f"  latch={rec['refused_multi_latch']:>2}"
                  f"  big={rec['refused_oversized']:>2}")
        for k in totals:
            totals[k] += rec[k]

    n_tu = len(records)
    n_rot_tu = sum(1 for r in records if r["rotations"])
    print(f"\ncensus: {n_tu} TUs compiled ({failed} failed), {n_rot_tu} with rotations")
    print(f"  rotations            : {totals['rotations']}"
          f" (memory licence: {totals['rotations_memory']})")
    print(f"  refused, non-duplicable: {totals['refused_non_duplicable']}")
    print(f"  refused, value escapes : {totals['refused_escape_in_loop']} in-loop"
          f" + {totals['refused_escape_outside_loop']} outside-loop"
          f" = {totals['refused_escape_in_loop'] + totals['refused_escape_outside_loop']}")
    print(f"  refused, multi-latch   : {totals['refused_multi_latch']}")
    print(f"  refused, oversized     : {totals['refused_oversized']}")
    print(f"  refused, no header exit: {totals['refused_no_exit_test']}")
    print(f"  refused, memory licence off: {totals['refused_memory_licence_off']}"
          f"  (these rotate under CCC_LOOP_INVERT_MEMORY=1)")
    if a.json:
        Path(a.json).write_text(json.dumps({"opt": a.opt, "totals": totals,
                                            "tus": records}, indent=1))
        print(f"  json -> {a.json}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
