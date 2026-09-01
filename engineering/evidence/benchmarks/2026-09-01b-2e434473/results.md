# LCCC benchmark report

- **UTC:** `2026-09-01T19:38:08.335069+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `2e43447352350890cb35119bd09b1f130bb64dfa`
- **LCCC binary SHA-256:** `8ddea4b2b2fbe2b0892e57ecb6c8d14556f22406103950029407865050fdb13e`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 144.40 ms | 100.24 ms | GCC | 1.4063 [1.3844, 1.4465] | pass |
| `fib` | 2.40 ms | 171.83 ms | GCC | 0.0141 [0.0131, 0.0143] | pass |
| `matmul` | 5.50 ms | 7.19 ms | GCC | 0.7643 [0.7528, 0.8447] | pass |
| `qsort` | 128.30 ms | 126.80 ms | GCC | 1.0099 [1.0051, 1.0403] | pass |
| `sieve` | 38.08 ms | 36.62 ms | GCC | 1.0428 [1.0202, 1.1853] | pass |
| `tce_sum` | 2.16 ms | 2.11 ms | GCC | 1.0297 [0.9263, 1.0749] | pass |
| `nbody` | 405.02 ms | 311.03 ms | GCC | 1.3076 [1.2747, 1.3175] | pass |
| `binary_trees` | 1.5765 s | 1.3568 s | GCC | 1.1781 [1.1410, 1.2293] | pass |
| `spectral_norm` | 304.15 ms | 206.24 ms | GCC | 1.4939 [1.4532, 1.5388] | pass |
| `mandelbrot` | 1.5675 s | 1.5055 s | GCC | 1.0397 [1.0275, 1.0527] | pass |
| `hash_table` | 12.9176 s | 11.4504 s | GCC | 1.1235 [1.1082, 1.2569] | pass |
| `strlen_bench` | 256.68 ms | 246.18 ms | GCC | 1.0259 [0.9923, 1.1042] | pass |
| `switch_dispatch` | 541.92 ms | 509.73 ms | GCC | 1.0650 [1.0419, 1.0758] | pass |
| `struct_copy` | 40.60 ms | 27.77 ms | GCC | 1.4435 [1.2528, 1.4856] | pass |
| `loop_patterns` | 86.81 ms | 75.39 ms | GCC | 1.1534 [1.1194, 1.1551] | pass |
| `fannkuch` | 2.9973 s | 2.5859 s | GCC | 1.1669 [1.1507, 1.1771] | pass |
| `ackermann` | 2.47 ms | 151.53 ms | GCC | 0.0162 [0.0147, 0.0166] | pass |
| `constant_recursion` | 2.52 ms | 156.78 ms | GCC | 0.0154 [0.0129, 0.0161] | pass |
| `bitops` | 312.86 ms | 398.61 ms | GCC | 0.7958 [0.7468, 0.8672] | pass |
| `double_reduction` | 96.09 ms | 113.29 ms | GCC | 0.8763 [0.7793, 0.8852] | pass |
| `ascii_case_fold` | 2.48 ms | 2.37 ms | GCC | 1.0482 [1.0138, 1.0848] | pass |
| `binary_search` | 2.26 ms | 2.21 ms | GCC | 1.0293 [0.9548, 1.0952] | pass |
| `ring_fifo` | 2.28 ms | 2.16 ms | GCC | 1.0307 [0.8958, 1.0889] | pass |
| `aarch64_select_patterns` | 168.69 ms | 128.81 ms | GCC | 1.3055 [1.2873, 1.3587] | pass |
| `histogram` | 2.68 ms | 2.51 ms | GCC | 1.0725 [1.0253, 1.0917] | pass |
| `gzip_crc32` | 159.31 ms | 167.66 ms | GCC | 0.9498 [0.9379, 0.9815] | pass |
| `libm_round_family` | 257.26 ms | 1.1180 s | GCC | 0.2270 [0.2240, 0.2422] | pass |
| `tls_seg_access` | 25.49 ms | 11.60 ms | GCC | 2.1947 [2.1852, 2.2157] | pass |
| `zlib_ng_adler32` | 52.19 ms | 40.70 ms | GCC | 1.2942 [1.2497, 1.3103] | pass |
| `expat_xml_scan` | 88.23 ms | 42.70 ms | GCC | 2.0745 [1.9845, 2.0906] | pass |
| `sqlite_varint` | 33.26 ms | 27.36 ms | GCC | 1.2183 [1.2140, 1.2278] | pass |
| `linux_find_bit` | 16.04 ms | 15.08 ms | GCC | 1.0548 [0.9943, 1.0705] | pass |
| `glibc_memcmp` | 12.39 ms | 9.34 ms | GCC | 1.3314 [1.3279, 1.5820] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `0.7431`
- Arithmetic mean ratio: `1.0545`
- Best individual ratio: `fib` = `0.0141`
- Worst individual ratio: `tls_seg_access` = `2.1947`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `0.7431`
- Arithmetic mean ratio: `1.0545`
- Best individual ratio: `fib` vs `gcc` = `0.0141`
- Worst individual ratio: `tls_seg_access` vs `gcc` = `2.1947`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
