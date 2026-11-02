#!/usr/bin/env python3
"""Compatibility wrapper forwarding to scripts/ra_quality_census.py --ab-env.

A/B a register-allocator configuration by STATIC code-quality census: the
B side is LCCC compiled with the environment variable(s) set, the A side is
the default LCCC — same binary on both sides so nothing but the knob
differs.  Why static counting rather than wall clock: the reference
environment is a 2-core VM with no hardware PMU, where a paired wall-clock
delta below roughly 3 % is not separable from scheduling noise; the census
buckets are exact, deterministic and reproducible, and they measure the
quantity the allocator actually controls.

Historical flag spellings are translated by this wrapper:

    --env KEY=VALUE (repeatable)  ->  --ab-env KEY=VALUE
    --opt -O3 / --cflag ... / --top / --json / positional files pass through

The historical script never consulted GCC/Clang, so the wrapper passes
--no-gcc --no-clang (ask the unified census directly to add those optional
columns).  The exit-1 regression gate is the historical ra_ab_census one —
HOT code decides it: spill traffic (stkref) in non-main functions with the
instruction count (insns) as tiebreak — on by default; pass --no-gate for a
report-only run.  The JSON export keeps the historical top-level keys
(env/flags/total_a/total_b/skipped/functions) plus unified-census additions
(gate/kernel_totals/driver_totals).
"""
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "scripts"))
import ra_quality_census

if __name__ == "__main__":
    argv = sys.argv[1:]
    out: list[str] = []
    env_seen = False
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--env":
            if i + 1 >= len(argv):
                print("error: --env requires a value", file=sys.stderr)
                sys.exit(2)
            out.extend(["--ab-env", argv[i + 1]])
            env_seen = True
            i += 2
            continue
        out.append(a)
        i += 1
    if not env_seen and "--ab-env" not in out and not any(
        a in ("--help", "-h") for a in argv
    ):
        # The historical script hard-required --env (ap.error, exit 2).
        print("error: --env is required (nothing to A/B)", file=sys.stderr)
        sys.exit(2)
    sys.argv = [sys.argv[0], "--no-gcc", "--no-clang", *out]
    sys.exit(ra_quality_census.main())
