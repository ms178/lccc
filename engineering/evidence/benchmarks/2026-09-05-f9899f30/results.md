# results.md — 2026-09-05, f9899f30 (screening, 3 reps)

ratio = median of paired per-round ratios; < 1 means LCCC faster.

| Kernel | LCCC (ms) | GCC (ms) | LCCC/GCC |
|---|---:|---:|---:|
| `fib` | 1.10 | 130.43 | 0.008 |
| `ackermann` | 1.11 | 61.66 | 0.018 |
| `constant_recursion` | 8.06 | 63.72 | 0.126 |
| `libm_round_family` | 202.22 | 490.73 | 0.412 |
| `bitops` | 201.61 | 302.11 | 0.667 |
| `matmul` | 4.19 | 5.65 | 0.741 |
| `gzip_crc32` | 135.76 | 155.29 | 0.874 |
| `double_reduction` | 110.11 | 118.84 | 0.927 |
| `binary_search` | 0.81 | 0.84 | 0.964 |
| `ring_fifo` | 0.77 | 0.79 | 0.969 |
| `switch_dispatch` | 467.37 | 478.45 | 0.977 |
| `tce_sum` | 0.73 | 0.73 | 0.997 |
| `arith_loop` | 92.90 | 92.67 | 1.002 |
| `qsort` | 112.52 | 111.87 | 1.006 |
| `ascii_case_fold` | 1.04 | 1.03 | 1.009 |
| `zlib_ng_adler32` | 38.02 | 37.25 | 1.021 |
| `tls_seg_access` | 9.26 | 9.07 | 1.021 |
| `histogram` | 1.63 | 1.57 | 1.042 |
| `struct_copy` | 23.01 | 21.85 | 1.053 |
| `strlen_bench` | 223.77 | 212.43 | 1.053 |
| `loop_patterns` | 49.24 | 46.71 | 1.054 |
| `binary_trees` | 2004.21 | 1879.27 | 1.066 |
| `sieve` | 52.06 | 48.13 | 1.082 |
| `hash_table` | 21868.23 | 19590.81 | 1.116 |
| `sqlite_varint` | 25.86 | 21.21 | 1.219 |
| `mandelbrot` | 1105.66 | 893.59 | 1.237 |
| `nbody` | 272.54 | 214.60 | 1.270 |
| `expat_xml_scan` | 47.10 | 34.67 | 1.359 |
| `linux_find_bit` | 14.57 | 10.23 | 1.424 |
| `fannkuch` | 3208.99 | 2252.09 | 1.425 |
| `glibc_memcmp` | 9.11 | 5.91 | 1.542 |
| `spectral_norm` | 291.77 | 181.98 | 1.603 |

**Geometric mean: 0.7390** over 32 pairs. Conventional code (recursion folds excluded): 1.0416.
