# LCCC benchmark report

- **UTC:** `2026-09-06T15:02:17.788708+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `f7f88be87e9d7383d9afff8de6b37f82b1a457e1`
- **LCCC binary SHA-256:** `843708ce0fa9df105ef6937a4acc3bf189e58959720b75d9a19997eca3b2bbc3`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 86.14 ms | 81.52 ms | GCC | 1.0419 [1.0197, 1.0739] | pass |
| `fib` | 1.83 ms | 117.39 ms | GCC | 0.0156 [0.0148, 0.0175] | pass |
| `matmul` | 4.21 ms | 5.36 ms | GCC | 0.7853 [0.7617, 0.7879] | pass |
| `qsort` | 101.42 ms | 101.65 ms | GCC | 0.9978 [0.9707, 1.0108] | pass |
| `sieve` | 48.14 ms | 41.84 ms | GCC | 1.1224 [1.0881, 1.1299] | pass |
| `tce_sum` | 1.73 ms | 1.81 ms | GCC | 0.9517 [0.7943, 1.1594] | pass |
| `nbody` | 236.05 ms | 187.12 ms | GCC | 1.2722 [1.2558, 1.2785] | pass |
| `binary_trees` | 1.0895 s | 976.41 ms | GCC | 1.1278 [1.1040, 1.1533] | pass |
| `spectral_norm` | 208.75 ms | 164.50 ms | GCC | 1.2808 [1.2409, 1.3197] | pass |
| `mandelbrot` | 974.58 ms | 808.47 ms | GCC | 1.2324 [1.1780, 1.2787] | pass |
| `hash_table` | 8.5466 s | 8.5082 s | GCC | 0.9750 [0.8800, 1.0508] | pass |
| `strlen_bench` | 199.97 ms | 190.40 ms | GCC | 1.0326 [1.0201, 1.0527] | pass |
| `switch_dispatch` | 422.81 ms | 413.84 ms | GCC | 1.0021 [0.9975, 1.0346] | pass |
| `struct_copy` | 21.82 ms | 20.62 ms | GCC | 1.0579 [1.0462, 1.0620] | pass |
| `loop_patterns` | 62.70 ms | 54.59 ms | GCC | 1.0800 [1.0584, 1.1631] | pass |
| `fannkuch` | 2.4536 s | 1.9928 s | GCC | 1.2186 [1.2000, 1.2904] | pass |
| `ackermann` | 1.83 ms | 56.67 ms | GCC | 0.0323 [0.0313, 0.0331] | pass |
| `constant_recursion` | 1.91 ms | 57.04 ms | GCC | 0.0327 [0.0318, 0.0337] | pass |
| `bitops` | 197.15 ms | 277.30 ms | GCC | 0.7098 [0.6554, 0.7298] | pass |
| `double_reduction` | 107.78 ms | 113.10 ms | GCC | 0.9510 [0.9344, 1.0474] | pass |
| `ascii_case_fold` | 2.12 ms | 2.13 ms | GCC | 0.9974 [0.7818, 1.0168] | pass |
| `binary_search` | 1.84 ms | 1.83 ms | GCC | 1.0040 [0.9401, 1.0702] | pass |
| `ring_fifo` | 1.72 ms | 1.65 ms | GCC | 1.0262 [0.9562, 1.0975] | pass |
| `aarch64_select_patterns` | 107.48 ms | 94.85 ms | GCC | 1.0827 [1.0412, 1.1662] | pass |
| `histogram` | 1.98 ms | 1.93 ms | GCC | 0.9855 [0.9639, 1.0551] | pass |
| `gzip_crc32` | 122.87 ms | 135.88 ms | GCC | 0.8820 [0.8745, 0.9238] | pass |
| `libm_round_family` | 186.40 ms | 451.22 ms | GCC | 0.4193 [0.3914, 0.4304] | pass |
| `tls_seg_access` | 9.39 ms | 8.94 ms | GCC | 1.0227 [1.0123, 1.0509] | pass |
| `zlib_ng_adler32` | 35.56 ms | 35.39 ms | GCC | 1.0068 [1.0048, 1.0100] | pass |
| `expat_xml_scan` | 44.10 ms | 31.86 ms | GCC | 1.4073 [1.3905, 1.4297] | pass |
| `sqlite_varint` | 24.13 ms | 18.67 ms | GCC | 1.2919 [1.2799, 1.3062] | pass |
| `linux_find_bit` | 14.08 ms | 10.20 ms | GCC | 1.3829 [1.3712, 1.3892] | pass |
| `glibc_memcmp` | 6.31 ms | 6.22 ms | GCC | 1.0201 [0.9835, 1.0695] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `0.7314`
- Arithmetic mean ratio: `0.9530`
- Best individual ratio: `fib` = `0.0156`
- Worst individual ratio: `expat_xml_scan` = `1.4073`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `0.7314`
- Arithmetic mean ratio: `0.9530`
- Best individual ratio: `fib` vs `gcc` = `0.0156`
- Worst individual ratio: `expat_xml_scan` vs `gcc` = `1.4073`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
