#!/usr/bin/env python3
"""Dump one function's assembly from every competition oracle AND local LCCC.

Why this exists
---------------
``scripts/godbolt.py compare`` re-issues a Compiler Explorer request per
compiler even when ``scripts/codegen_oracle.py --rank`` has already cached
exactly that (compiler, flags, source) tuple in ``.godbolt-cache``.  On a
shared egress IP the extra requests collide with CE's rate limiter (HTTP 429)
and the deep-dive silently degrades to "all oracles ERROR" -- precisely the
measurement you cannot afford to lose when you are chasing a gap that the
survey just quantified.

This tool therefore goes through :func:`codegen_oracle._compile_remote`, the
SAME cache namespace the survey uses (``att-v2``), so a deep-dive after
``--rank`` is offline: zero network requests, byte-identical assembly to the
survey's numbers, and no 429 risk.  It reuses the survey's own
``_function_body`` / ``_stats`` implementations, so counts printed here match
the rank table exactly (no second metric dialect).

Usage::

    scripts/oracle_asm.py tests/benchmark/programs/matmul.c --function matmul \\
        --out /tmp/matmul --flags '-O2 -march=x86-64-v3'

Writes ``<out>/{lccc,gcc16.2,clang,icc,icx}.s`` (whole function bodies, AT&T)
and prints the shared statistic columns.  Exit status is non-zero only when
the local compile or function extraction fails; a missing oracle is an error
row because a silent gap would understate the competition.
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

import os

# Lives in scripts/, so the checkout is the parent directory. LCCC_REPO stays
# as an override for the case where the tool is invoked through a symlink or a
# copy in a scratch directory (research layouts).
REPO = Path(os.environ.get("LCCC_REPO", Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(REPO / "scripts"))

import codegen_oracle as oracle  # noqa: E402
import godbolt  # noqa: E402

DEFAULT_ORACLES = ("gcc16.2", "clang", "icc", "icx")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("source", type=Path)
    parser.add_argument("--function")
    parser.add_argument("--flags", default="-O2 -march=x86-64-v3")
    parser.add_argument("--local-flags")
    parser.add_argument("--local", default=str(REPO / "target" / "fastbuild" / "lccc"))
    parser.add_argument("--oracles", default=",".join(DEFAULT_ORACLES))
    parser.add_argument("--out", type=Path)
    args = parser.parse_args(argv)

    source_text = args.source.read_text(errors="replace")
    out: Path | None = args.out
    if out:
        out.mkdir(parents=True, exist_ok=True)

    records: list[tuple[str, list[str] | None, str | None]] = []
    try:
        lines = oracle._local_compile(args.local, args.source,
                                      args.local_flags or args.flags)
        body = oracle._function_body(lines, args.function)
        if body is None:
            raise godbolt.GodboltError(
                f"function '{args.function}' was not emitted by local LCCC")
        records.append(("lccc", body, None))
    except Exception as exc:  # noqa: BLE001 - report, never abort the survey
        records.append(("lccc", None, str(exc)))

    for name in [o for o in (s.strip() for s in args.oracles.split(",")) if o]:
        try:
            body = oracle._function_body(oracle._compile_remote(name, source_text,
                                                                args.flags),
                                         args.function)
            records.append((name, body,
                            None if body is not None else "function not emitted"))
        except Exception as exc:  # noqa: BLE001
            records.append((name, None, str(exc)))

    print(f"{'compiler':<12} {'insns':>6} {'load':>5} {'stor':>5} {'spill':>6} "
          f"{'brnch':>6} {'vec':>4}")
    print("-" * 52)
    for key, body, error in records:
        if body is None:
            print(f"{key:<12} ERROR: {error}")
            continue
        stats = oracle._stats(body, "x86_64")
        print(f"{key:<12} {stats.instructions:>6} {stats.loads:>5} {stats.stores:>5} "
              f"{stats.spills:>6} {stats.branches:>6} {stats.vectors:>4}")
        if out:
            (out / f"{key}.s").write_text("\n".join(body) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
