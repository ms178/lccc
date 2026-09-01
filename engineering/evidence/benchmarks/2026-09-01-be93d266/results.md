# LCCC benchmark report

- **UTC:** `2026-09-01T17:53:24.559664+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `be93d266c97f615e442b8e0438eb2ddc2ad35630`
- **LCCC binary SHA-256:** `38254264a3c8be8615136c8324305a5bc6399aa5bf5bc06cfe6446f00b1ad12d`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 143.54 ms | 100.08 ms | GCC | 1.4295 [1.3815, 1.5165] | pass |
| `fib` | 2.46 ms | 176.52 ms | GCC | 0.0137 [0.0136, 0.0150] | pass |
| `matmul` | 5.40 ms | 7.35 ms | GCC | 0.7471 [0.7088, 0.7868] | pass |
| `qsort` | 128.74 ms | 126.60 ms | GCC | 1.0114 [0.9861, 1.0437] | pass |
| `sieve` | 40.14 ms | 37.86 ms | GCC | 1.0520 [1.0268, 1.0715] | pass |
| `tce_sum` | 2.17 ms | 2.26 ms | GCC | 1.0127 [0.8938, 1.4145] | pass |
| `nbody` | 396.83 ms | 306.64 ms | GCC | 1.2808 [1.2626, 1.3538] | pass |
| `binary_trees` | 1.4375 s | 1.3170 s | GCC | 1.0869 [1.0721, 1.1098] | pass |
| `spectral_norm` | 298.73 ms | 200.40 ms | GCC | 1.4914 [1.4331, 1.4926] | pass |
| `mandelbrot` | 1.5642 s | 1.5048 s | GCC | 1.0381 [1.0306, 1.0421] | pass |
| `hash_table` | 12.0356 s | 10.3992 s | GCC | 1.1809 [1.0586, 1.2441] | pass |
| `strlen_bench` | 241.32 ms | 237.86 ms | GCC | 1.0407 [0.9875, 1.0682] | pass |
| `switch_dispatch` | 539.84 ms | 510.87 ms | GCC | 1.0510 [1.0446, 1.0741] | pass |
| `struct_copy` | 39.39 ms | 27.36 ms | GCC | 1.4393 [1.4358, 1.4437] | pass |
| `loop_patterns` | 85.36 ms | 71.42 ms | GCC | 1.1954 [1.1787, 1.2047] | pass |
| `fannkuch` | 2.9837 s | 2.5617 s | GCC | 1.1656 [1.1541, 1.1799] | pass |
| `ackermann` | 2.36 ms | 149.68 ms | GCC | 0.0158 [0.0143, 0.0160] | pass |
| `constant_recursion` | 2.31 ms | 149.52 ms | GCC | 0.0155 [0.0141, 0.0164] | pass |
| `bitops` | 299.40 ms | 389.92 ms | GCC | 0.7674 [0.7542, 0.7859] | pass |
| `double_reduction` | 97.52 ms | 111.15 ms | GCC | 0.8740 [0.8392, 0.8967] | pass |
| `ascii_case_fold` | 2.39 ms | 2.25 ms | GCC | 1.0486 [0.9884, 1.1136] | pass |
| `binary_search` | 2.17 ms | 2.19 ms | GCC | 0.9941 [0.9605, 1.0095] | pass |
| `ring_fifo` | 2.11 ms | 2.12 ms | GCC | 1.0011 [0.9416, 1.0221] | pass |
| `aarch64_select_patterns` | 170.50 ms | 129.68 ms | GCC | 1.3099 [1.2920, 1.3589] | pass |
| `histogram` | 2.62 ms | 2.54 ms | GCC | 1.0653 [0.9969, 1.1427] | pass |
| `gzip_crc32` | 161.67 ms | 170.14 ms | GCC | 0.9496 [0.9244, 0.9738] | pass |
| `libm_round_family` | 255.31 ms | 1.1160 s | GCC | 0.2301 [0.2251, 0.2370] | pass |
| `tls_seg_access` | 24.64 ms | 11.37 ms | GCC | 2.1613 [1.8039, 2.2896] | pass |
| `zlib_ng_adler32` | 50.18 ms | 39.62 ms | GCC | 1.2386 [1.2116, 1.2727] | pass |
| `expat_xml_scan` | 86.86 ms | 41.47 ms | GCC | 2.0891 [2.0845, 2.1058] | pass |
| `sqlite_varint` | 34.37 ms | 27.12 ms | GCC | 1.2498 [1.2134, 1.2889] | pass |
| `linux_find_bit` | 15.41 ms | 14.76 ms | GCC | 1.0448 [1.0307, 1.0598] | pass |
| `glibc_memcmp` | 12.24 ms | 9.18 ms | GCC | 1.3303 [1.3172, 1.3659] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `0.7381`
- Arithmetic mean ratio: `1.0491`
- Best individual ratio: `fib` = `0.0137`
- Worst individual ratio: `tls_seg_access` = `2.1613`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `0.7381`
- Arithmetic mean ratio: `1.0491`
- Best individual ratio: `fib` vs `gcc` = `0.0137`
- Worst individual ratio: `tls_seg_access` vs `gcc` = `2.1613`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
