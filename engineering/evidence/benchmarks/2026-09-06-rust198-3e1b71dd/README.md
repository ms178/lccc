# Benchmark evidence — 2026-09-06

This is the fresh full-corpus run on checked-in commit `f7f88be8` after the Rust 1.98.1, benchmark-CI, and documentation pass.

## Reproduction

```text
python3 tests/benchmark/run_benchmarks.py \
  --lccc target/release/lccc --compilers lccc,gcc \
  --reps 9 --warmup 1 --artifact-dir /tmp/bench-rust198-final/artifacts \
  --json results.json --markdown results.md --seed 20260906 --strict
```

The compiler was built with `scripts/build_lccc_o1_j2.sh` (Rust 1.98.1, release profile, compiler `-O1`, Cargo `-j2`). The runner used randomized paired rounds, one excluded warm-up, CPU 0 pinning, and retained every sample. This VM has no usable PMU; these are wall-clock screening measurements, not hardware-counter claims.

## Result

- Corpus: **33** registered programs; all **33** LCCC/GCC pairs were correct.
- Geometric mean LCCC/GCC ratio: **0.7314**; arithmetic mean: **0.9530**.
- Fastest row: `fib` at `0.0156`; slowest row: `expat_xml_scan` at `1.4073`.

| Benchmark | LCCC median | GCC median | LCCC/GCC |
|---|---:|---:|---:|
| `arith_loop` | 86.14 ms | 81.52 ms | 1.042 |
| `fib` | 1.83 ms | 117.39 ms | 0.016 |
| `matmul` | 4.21 ms | 5.36 ms | 0.785 |
| `qsort` | 101.42 ms | 101.65 ms | 0.998 |
| `sieve` | 48.14 ms | 41.84 ms | 1.122 |
| `tce_sum` | 1.73 ms | 1.81 ms | 0.952 |
| `nbody` | 236.05 ms | 187.12 ms | 1.272 |
| `binary_trees` | 1.0895 s | 976.41 ms | 1.128 |
| `spectral_norm` | 208.75 ms | 164.50 ms | 1.281 |
| `mandelbrot` | 974.58 ms | 808.47 ms | 1.232 |
| `hash_table` | 8.5466 s | 8.5082 s | 0.975 |
| `strlen_bench` | 199.97 ms | 190.40 ms | 1.033 |
| `switch_dispatch` | 422.81 ms | 413.84 ms | 1.002 |
| `struct_copy` | 21.82 ms | 20.62 ms | 1.058 |
| `loop_patterns` | 62.70 ms | 54.59 ms | 1.080 |
| `fannkuch` | 2.4536 s | 1.9928 s | 1.219 |
| `ackermann` | 1.83 ms | 56.67 ms | 0.032 |
| `constant_recursion` | 1.91 ms | 57.04 ms | 0.033 |
| `bitops` | 197.15 ms | 277.30 ms | 0.710 |
| `double_reduction` | 107.78 ms | 113.10 ms | 0.951 |
| `ascii_case_fold` | 2.12 ms | 2.13 ms | 0.997 |
| `binary_search` | 1.84 ms | 1.83 ms | 1.004 |
| `ring_fifo` | 1.72 ms | 1.65 ms | 1.026 |
| `aarch64_select_patterns` | 107.48 ms | 94.85 ms | 1.083 |
| `histogram` | 1.98 ms | 1.93 ms | 0.985 |
| `gzip_crc32` | 122.87 ms | 135.88 ms | 0.882 |
| `libm_round_family` | 186.40 ms | 451.22 ms | 0.419 |
| `tls_seg_access` | 9.39 ms | 8.94 ms | 1.023 |
| `zlib_ng_adler32` | 35.56 ms | 35.39 ms | 1.007 |
| `expat_xml_scan` | 44.10 ms | 31.86 ms | 1.407 |
| `sqlite_varint` | 24.13 ms | 18.67 ms | 1.292 |
| `linux_find_bit` | 14.08 ms | 10.20 ms | 1.383 |
| `glibc_memcmp` | 6.31 ms | 6.22 ms | 1.020 |

Raw per-round data is in [`results.json`](results.json); the rendered report is [`results.md`](results.md), and the terminal transcript is [`terminal.txt`](terminal.txt).
