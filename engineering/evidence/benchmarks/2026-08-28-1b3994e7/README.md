# Canonical screening run — 2026-08-28 — main @ `1b3994e7`

Frozen point-in-time evidence (see `../README.md` retention rule). The root
`README.md` performance table is derived from exactly these files; nothing in
the table is hand-entered.

## Provenance

| Item | Value |
|---|---|
| LCCC revision | `1b3994e7e48808b9637b37a1f004d091b792f1fc` (post PR #277/#278) |
| LCCC binary | `target/fastbuild/lccc`, SHA-256 `c86d62b0f9b1143644d53593ded215edb799d6b41d36edb721a0a16bf4fbc68b` |
| GCC | 14.2.0 (Debian 14.2.0-19), external reference, `-O2` |
| Host | shared 2-vCPU VM (hypervisor detected), no PMU (`perf` not installed), taskset-pinned to CPU 0 of [0,1] |
| Window | 2026-08-28 18:37–18:50 UTC (three consecutive same-window passes) |
| Runner | `tests/benchmark/run_benchmarks.py`, seed `20260828`, randomized compiler order per round, warm-ups excluded, paired medians + bootstrap CI, checksum gating vs GCC |
| Rounds | 15 paired rounds + 2 warm-ups for 32 kernels; `hash_table` 8 rounds + 1 warm-up (VM-window budget; realized CI ±0.5 %, CV 5.5 %, one MAD outlier retained — disclosed in the root README) |

**Ratio definition.** All `LCCC/GCC` ratios are the runner's median of
paired per-round ratios (numerator/denominator within each randomized
round), not quotients of the displayed medians; the definitions agree in
aggregate (0.737 quotient-of-medians vs 0.7381 paired) and differ per row
by at most ~2 %.

## Pass structure (VM constraint: 10-minute foreground windows)

| File | Kernels | UTC |
|---|---|---|
| `chunkA.json` / `chunkA.md` | binary_trees, fannkuch, mandelbrot, nbody, spectral_norm | 18:37 |
| `chunkB.json` / `chunkB.md` | remaining 27 kernels (canonical rounds) | 18:42 |
| `chunkC.json` / `chunkC.md` | hash_table (8 rounds) | 18:43 |

## Result integrity

- 33/33 pairs: LCCC output byte-identical to the GCC baseline (checksum
  gating; a mismatch disqualifies the number).
- Aggregate recomputed independently from the raw JSON (`merged-verified.md`):
  geomean 0.7381 (all 33), 1.0963 (30 conventional-code pairs), cross-checked
  against the runner's own aggregate where emitted.
- Sub-2 ms medians (fib, ackermann, constant_recursion, binary_search,
  tce_sum, ring_fifo, ascii_case_fold, histogram) are wall-timer-limited;
  their ratios are indicative only.
