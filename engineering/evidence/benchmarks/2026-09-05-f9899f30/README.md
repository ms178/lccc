# Benchmark evidence — 2026-09-05, `rebased` @ `f9899f30` (post #419 rebase)

Screening run on the shared 2-vCPU sandbox VM (no PMU): lccc vs GCC 14.2,
`-O2`, 3 paired timed reps + 1 warmup per kernel, randomized compiler order,
MAD outliers retained, all outputs checksummed against GCC (32/32 identical;
`aarch64_select_patterns` is ARM-only and excluded). Two same-window batches
of 16 kernels each; raw per-round samples in `results.json`.

**Aggregate: geometric mean 0.7390 over 32 pairs. Conventional code
(recursion folds excluded): 1.0416.**

Headline wins: fib 118×, ackermann 55×, constant_recursion 8×,
libm_round_family 2.43×, bitops 1.50×, matmul 1.35×, gzip_crc32 1.14×,
double_reduction 1.08×.

Remaining gap (worst): spectral_norm 1.60×, glibc_memcmp 1.54×,
fannkuch 1.43×, linux_find_bit 1.42×, expat_xml_scan 1.36×,
nbody 1.27×, mandelbrot 1.24×, sqlite_varint 1.22×.

Movement vs the 2026-09-01 report (different window, 3 vs 9 reps —
directional only): expat 2.07→1.36, tls_seg_access 2.20→1.02,
arith_loop 1.41→1.00, struct_copy 1.44→1.05, binary_trees 1.18→1.07,
double_reduction 1.14→0.93 (now faster than GCC). spectral_norm,
glibc_memcmp and fannkuch remain the tracked P0 targets (OPT-40 /
PERF-41 family).

Caveat: ratios are only comparable within one window; this is a screening
refresh, not a replacement for the 9-rep 2026-09-01 study.
