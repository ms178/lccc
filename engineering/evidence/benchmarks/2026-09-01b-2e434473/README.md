# Benchmark evidence — 2026-09-01 (second run), `2e434473`

33-kernel paired run, LCCC vs stock GCC 14.2.0, after generalizing
induction-variable widening through the element-scaling chain.

- `run_benchmarks.py --compilers lccc,gcc --reps 9 --warmup 1 --opt=-O2`
- All 33 outputs byte-identical to the GCC baseline.
- Aggregate: geometric mean **0.7431**; conventional subset (30, recursion
  folds excluded) **1.096**; workload subset **1.381**.

**This run states the current position; it does not attribute anything.**
A ratio is only stable when both arms were measured in one window, and GCC's
own times moved between runs on this shared VM. Attribution of the widening
change comes from a paired same-window A/B with `CCC_NO_IV_WIDEN`, reported in
`engineering/FOLLOWUP-2026-09-01k-iv-widen-scaling.md`: `sieve` **−21.6%**,
`nbody` −3.0%, `arith_loop` −1.3%, `sqlite_varint` −1.3%, nothing regressed.
