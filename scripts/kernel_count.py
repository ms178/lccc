#!/usr/bin/env python3
"""Compatibility wrapper forwarding to scripts/ra_quality_census.py --kernels.

Per-function instruction counts for the kernel corpus: LCCC vs system GCC.
Fast local proxy for the Compiler Explorer oracle (scripts/godbolt.py): it
censuses every kernel in tests/benchmark/kernel_corpus with both compilers
at -O2 and reports the delta.  Official scoreboard numbers still come from
godbolt.py; this harness exists so a peephole change can be measured in
seconds.

The historical positional substring filters (matched against kernel file
names) become --filter values; the LCCC / GCC / CFLAGS environment
interface is unchanged (the unified census reads the same variables).  The
curated hot-function map (KERN_FUNCS) moved into ra_quality_census.py, and
the unified run reports the full RA bucket set for those functions — the
old script reported instructions only — plus the LCCC-wins summary line.
Clang is not consulted, matching the historical lccc-vs-gcc comparison.
"""
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "scripts"))
import ra_quality_census

if __name__ == "__main__":
    argv: list[str] = []
    for arg in sys.argv[1:]:
        if arg.startswith("-"):
            # Flag spelling (--help, --json, ...): pass through unchanged.
            # The historical script had no flags, so this cannot shadow a
            # documented substring filter.
            argv.append(arg)
        else:
            # Historical positional argument: substring filter on file names.
            argv.extend(["--filter", arg])
    sys.argv = [sys.argv[0], "--kernels", "--no-clang", *argv]
    sys.exit(ra_quality_census.main())
