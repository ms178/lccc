#!/usr/bin/env python3
"""CI entry point for the complete checked-in benchmark corpus.

The repository's canonical runner owns the corpus, paired timing protocol,
correctness oracle, environment capture, and report schema.  This small
compatibility wrapper keeps the historical CI command name while preventing a
second, permanently smaller benchmark list from drifting away from
``tests/benchmark/run_benchmarks.py``.

With no explicit selection the runner executes every registered synthetic and
workload-derived kernel (currently the full 39-program corpus, including the
six workload kernels added 2026-09-06: chacha20_block, sha256_transform,
linux_rbtree, zstd_count, lz4_compress, glibc_strstr).  Use
``--only`` for a local focused experiment, never in the benchmark workflow.
"""

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO))

from tests.benchmark.run_benchmarks import main as canonical_main  # noqa: E402


def main() -> int:
    args = sys.argv[1:]

    # The old wrapper accepted --gcc-inc, but the canonical runner discovers
    # GCC's builtin include directory from the selected GCC executable and
    # records that compiler in the evidence metadata.  Ignore the deprecated
    # compatibility option rather than maintaining a second discovery path.
    filtered: list[str] = []
    index = 0
    while index < len(args):
        if args[index] == "--gcc-inc":
            index += 2
            continue
        filtered.append(args[index])
        index += 1
    args = filtered

    # Preserve the old --summary spelling as a deterministic Markdown report.
    if "--summary" in args:
        args.remove("--summary")
        if "--markdown" not in args:
            args.extend(["--markdown", "bench-summary.md"])

    if "--compilers" not in args:
        args.extend(["--compilers", "lccc,gcc"])
    if "--reps" not in args:
        args.extend(["--reps", "9"])
    if "--warmup" not in args:
        args.extend(["--warmup", "1"])
    if "--strict" not in args:
        args.append("--strict")

    sys.argv = [sys.argv[0], *args]
    return canonical_main()


if __name__ == "__main__":
    raise SystemExit(main())
