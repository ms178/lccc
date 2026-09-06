# LCCC benchmark report

- **UTC:** `2026-09-06T21:20:44.275306+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'none', 'allowed_cpus': [0, 1], 'applied': False, 'reason': 'affinity disabled by user'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `84b48230522da3b117fa6ecbb0ca11357aab7cc3`
- **LCCC binary SHA-256:** `79966146e52a21a6911f1be9a0e9f6e16c568beb245027a0e0e5cafb78d038db`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | CLANG median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 114.62 ms | 98.54 ms | 99.85 ms | GCC | 1.1625 [1.1412, 1.2063] | pass |
| `fib` | 1.41 ms | 171.35 ms | 292.01 ms | GCC | 0.0083 [0.0075, 0.0084] | pass |
| `matmul` | 5.39 ms | 8.98 ms | 7.96 ms | Clang | 0.6847 [0.5964, 0.7012] | pass |
| `qsort` | 124.70 ms | 124.15 ms | 123.40 ms | Clang | 1.0094 [1.0033, 1.0127] | pass |
| `sieve` | 39.59 ms | 37.96 ms | 35.04 ms | Clang | 1.1334 [1.1120, 1.1584] | pass |
| `tce_sum` | 1.18 ms | 1.21 ms | 1.21 ms | Clang | 1.0018 [0.9095, 1.0596] | pass |
| `nbody` | 413.86 ms | 309.78 ms | 332.06 ms | GCC | 1.3489 [1.3206, 1.3557] | pass |
| `binary_trees` | 1.5857 s | 1.4501 s | 1.3441 s | Clang | 1.1191 [1.0203, 1.2282] | pass |
| `spectral_norm` | 253.29 ms | 200.57 ms | 198.97 ms | Clang | 1.2713 [1.2545, 1.3190] | pass |
| `mandelbrot` | 1.5442 s | 1.4892 s | 1.5052 s | GCC | 1.0362 [1.0332, 1.0396] | pass |
| `hash_table` | 11.2746 s | 10.3646 s | 10.3144 s | Clang | 1.0931 [1.0556, 1.1543] | pass |
| `strlen_bench` | 294.85 ms | 285.62 ms | 265.35 ms | Clang | 1.0836 [1.0195, 1.1227] | pass |
| `switch_dispatch` | 529.34 ms | 503.93 ms | 514.64 ms | GCC | 1.0479 [1.0464, 1.0546] | pass |
| `struct_copy` | 28.11 ms | 26.46 ms | 20.53 ms | Clang | 1.3612 [1.3332, 1.3839] | pass |
| `loop_patterns` | 78.16 ms | 75.58 ms | 67.90 ms | Clang | 1.1488 [1.1374, 1.1825] | pass |
| `fannkuch` | 3.1612 s | 2.5449 s | 2.6823 s | GCC | 1.2415 [1.2276, 1.2468] | pass |
| `ackermann` | 1.45 ms | 148.72 ms | 636.24 ms | GCC | 0.0098 [0.0086, 0.0102] | pass |
| `constant_recursion` | 1.50 ms | 149.59 ms | 636.71 ms | GCC | 0.0098 [0.0094, 0.0102] | pass |
| `bitops` | 296.59 ms | 394.37 ms | 347.03 ms | Clang | 0.8608 [0.8420, 0.8705] | pass |
| `double_reduction` | 107.93 ms | 117.19 ms | 118.24 ms | GCC | 0.8494 [0.8311, 0.9173] | pass |
| `ascii_case_fold` | 1.36 ms | 1.34 ms | 1.28 ms | Clang | 1.0932 [1.0305, 1.1551] | pass |
| `binary_search` | 1.30 ms | 1.30 ms | 1.31 ms | GCC | 0.9918 [0.9627, 1.0497] | pass |
| `ring_fifo` | 1.20 ms | 1.17 ms | 1.18 ms | GCC | 1.0242 [0.9644, 1.1418] | pass |
| `aarch64_select_patterns` | 151.08 ms | 128.69 ms | 102.30 ms | Clang | 1.4916 [1.3785, 1.5133] | pass |
| `histogram` | 1.50 ms | 1.56 ms | 1.39 ms | Clang | 1.1156 [1.0657, 1.1710] | pass |
| `gzip_crc32` | 150.36 ms | 167.12 ms | 166.61 ms | Clang | 0.9033 [0.9007, 0.9197] | pass |
| `libm_round_family` | 238.67 ms | 1.0977 s | 919.94 ms | Clang | 0.2593 [0.2537, 0.2733] | pass |
| `tls_seg_access` | 11.77 ms | 13.09 ms | 11.75 ms | Clang | 1.0813 [0.9131, 1.1044] | pass |
| `zlib_ng_adler32` | 40.30 ms | 38.43 ms | 36.24 ms | Clang | 1.1121 [1.1088, 1.1358] | pass |
| `expat_xml_scan` | 67.01 ms | 38.93 ms | 52.38 ms | GCC | 1.7235 [1.7080, 1.7369] | pass |
| `sqlite_varint` | 30.51 ms | 24.75 ms | 25.91 ms | GCC | 1.2314 [1.2273, 1.2357] | pass |
| `linux_find_bit` | 17.81 ms | 13.54 ms | 16.71 ms | GCC | 1.3072 [1.2888, 1.3222] | pass |
| `glibc_memcmp` | 9.42 ms | 8.13 ms | 8.26 ms | GCC | 1.1607 [1.1545, 1.1728] | pass |
| `chacha20_block` | 2.7355 s | 278.14 ms | 242.48 ms | Clang | 11.2733 [11.1431, 11.8040] | pass |
| `sha256_transform` | 790.51 ms | 285.60 ms | 304.28 ms | GCC | 2.7638 [2.7553, 2.7776] | pass |
| `linux_rbtree` | 16.96 ms | 15.19 ms | 15.30 ms | GCC | 1.1184 [1.1037, 1.1339] | pass |
| `zstd_count` | 15.15 ms | 10.61 ms | 11.30 ms | GCC | 1.4008 [1.3880, 1.4281] | pass |
| `lz4_compress` | 10.61 ms | 2.80 ms | 3.18 ms | GCC | 3.7839 [3.7200, 3.8939] | pass |
| `glibc_strstr` | 4.7544 s | 4.4162 s | 4.5481 s | GCC | 1.0758 [1.0729, 1.0785] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `0.8025`
- Arithmetic mean ratio: `1.3232`
- Best individual ratio: `fib` = `0.0083`
- Worst individual ratio: `chacha20_block` = `9.8599`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `0.8342`
- Arithmetic mean ratio: `1.3947`
- Best individual ratio: `fib` vs `gcc` = `0.0083`
- Worst individual ratio: `chacha20_block` vs `clang` = `11.2733`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
