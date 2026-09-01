# Benchmark evidence — 2026-09-01, `be93d266`

Full 33-kernel paired run, LCCC vs stock GCC 14.2.0 (Debian 14.2.0-19),
**after** the order-preserving block-layout fix.

- `tests/benchmark/run_benchmarks.py --compilers lccc,gcc --reps 9 --warmup 1 --opt=-O2`
- Randomized compiler order every round; MAD outliers retained and reported.
- **All 33 outputs byte-identical to the GCC baseline.**
- Aggregate: geometric mean **0.7381** (n=33); conventional subset excluding
  the three recursion folds **1.090**; workload-derived subset **1.374**.

Shared 2-vCPU VM, no PMU. These rank code-generation work; they are not
microarchitectural claims.

**Do not compare absolute times or ratios with other reports.** A ratio is
only stable when both arms were measured in the same window; VM drift that
hits the two compilers unequally moves it. Attribution of any change must come
from a paired same-window A/B with kill switches — see
`engineering/FOLLOWUP-2026-09-01j-block-layout-order-preserving.md`, where
that method showed two of three previously "reported" regressions did not
exist.
