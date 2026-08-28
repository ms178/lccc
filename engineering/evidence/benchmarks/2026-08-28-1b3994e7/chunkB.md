# LCCC benchmark report

- **UTC:** `2026-08-28T18:42:00.838439+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `1b3994e7e48808b9637b37a1f004d091b792f1fc`
- **LCCC binary SHA-256:** `c86d62b0f9b1143644d53593ded215edb799d6b41d36edb721a0a16bf4fbc68b`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 113.97 ms | 92.57 ms | GCC | 1.2266 [1.2186, 1.2335] | pass |
| `fib` | 1.25 ms | 129.74 ms | GCC | 0.0096 [0.0089, 0.0106] | pass |
| `matmul` | 5.25 ms | 5.64 ms | GCC | 0.9383 [0.9259, 0.9464] | pass |
| `qsort` | 109.34 ms | 112.45 ms | GCC | 0.9740 [0.9649, 0.9760] | pass |
| `sieve` | 49.94 ms | 42.01 ms | GCC | 1.2004 [1.1526, 1.2356] | pass |
| `tce_sum` | 0.76 ms | 0.77 ms | GCC | 1.0029 [0.9458, 1.0240] | pass |
| `strlen_bench` | 217.35 ms | 210.13 ms | GCC | 1.0410 [1.0249, 1.0468] | pass |
| `switch_dispatch` | 466.64 ms | 477.91 ms | GCC | 0.9753 [0.9723, 0.9766] | pass |
| `struct_copy` | 29.69 ms | 21.81 ms | GCC | 1.3465 [1.3381, 1.3719] | pass |
| `loop_patterns` | 45.73 ms | 43.56 ms | GCC | 1.0589 [0.9938, 1.1195] | pass |
| `ackermann` | 1.10 ms | 61.61 ms | GCC | 0.0179 [0.0165, 0.0184] | pass |
| `constant_recursion` | 1.01 ms | 61.30 ms | GCC | 0.0164 [0.0160, 0.0170] | pass |
| `bitops` | 167.75 ms | 299.70 ms | GCC | 0.5582 [0.5567, 0.5598] | pass |
| `double_reduction` | 105.40 ms | 109.76 ms | GCC | 0.9545 [0.9489, 0.9631] | pass |
| `ascii_case_fold` | 0.93 ms | 0.89 ms | GCC | 1.0656 [1.0258, 1.1462] | pass |
| `binary_search` | 1.01 ms | 1.03 ms | GCC | 0.9906 [0.9617, 1.0261] | pass |
| `ring_fifo` | 0.92 ms | 0.91 ms | GCC | 1.0401 [1.0145, 1.0602] | pass |
| `aarch64_select_patterns` | 123.67 ms | 106.31 ms | GCC | 1.1651 [1.1629, 1.1685] | pass |
| `histogram` | 1.58 ms | 1.50 ms | GCC | 1.0479 [1.0397, 1.0656] | pass |
| `gzip_crc32` | 135.47 ms | 155.30 ms | GCC | 0.8732 [0.8664, 0.8755] | pass |
| `libm_round_family` | 202.36 ms | 490.84 ms | GCC | 0.4126 [0.4106, 0.4132] | pass |
| `tls_seg_access` | 19.57 ms | 9.12 ms | GCC | 2.1465 [2.1175, 2.1529] | pass |
| `zlib_ng_adler32` | 56.26 ms | 37.45 ms | GCC | 1.5045 [1.4829, 1.5218] | pass |
| `expat_xml_scan` | 62.80 ms | 34.94 ms | GCC | 1.8025 [1.7979, 1.8062] | pass |
| `sqlite_varint` | 26.14 ms | 21.47 ms | GCC | 1.1921 [1.1718, 1.2393] | pass |
| `linux_find_bit` | 15.29 ms | 10.15 ms | GCC | 1.5083 [1.5062, 1.5134] | pass |
| `glibc_memcmp` | 6.15 ms | 5.91 ms | GCC | 1.0299 [1.0213, 1.0454] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `0.6632`
- Arithmetic mean ratio: `1.0037`
- Best individual ratio: `fib` = `0.0096`
- Worst individual ratio: `tls_seg_access` = `2.1465`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `0.6632`
- Arithmetic mean ratio: `1.0037`
- Best individual ratio: `fib` vs `gcc` = `0.0096`
- Worst individual ratio: `tls_seg_access` vs `gcc` = `2.1465`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
