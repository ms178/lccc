# LCCC benchmark report

- **UTC:** `2026-09-25T22:27:18.425202+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `3313ef9caef5bdf910226d94cd9a69a86606c561`
- **LCCC binary SHA-256:** `7b324bff70bd1b0d54b98edf6598fddbfaafe55c486470434f863fb039985973`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | CCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 117.01 ms | 118.09 ms | 117.89 ms | GCC | 0.9905 [0.9741, 1.0006] | pass |
| `fib` | 2.56 ms | 2.53 ms | 194.04 ms | CCC | 1.0326 [0.8419, 1.0840] | pass |
| `matmul` | 5.55 ms | 5.78 ms | 5.61 ms | GCC | 0.9994 [0.8949, 1.0526] | pass |
| `qsort` | 148.75 ms | 149.19 ms | 150.26 ms | CCC | 1.0054 [0.9940, 1.0209] | pass |
| `sieve` | 46.67 ms | 46.91 ms | 42.94 ms | GCC | 1.0869 [1.0686, 1.1376] | pass |
| `tce_sum` | 2.26 ms | 2.36 ms | 2.33 ms | GCC | 0.9750 [0.9020, 0.9938] | pass |
| `nbody` | 282.54 ms | 283.76 ms | 263.17 ms | GCC | 1.0736 [1.0732, 1.0815] | pass |
| `binary_trees` | 1.5293 s | 1.5416 s | 1.2949 s | GCC | 1.1928 [1.1461, 1.2189] | pass |
| `spectral_norm` | 6.3834 s | 6.4082 s | 229.19 ms | GCC | 27.8482 [27.7984, 28.1467] | pass |
| `mandelbrot` | 1.3533 s | 1.3610 s | 1.0527 s | GCC | 1.2860 [1.2744, 1.2965] | pass |
| `hash_table` | 13.4590 s | 13.5873 s | 13.6700 s | CCC | 1.0013 [0.9847, 1.0337] | pass |
| `strlen_bench` | 260.14 ms | 257.16 ms | 254.29 ms | GCC | 1.0187 [1.0105, 1.0337] | pass |
| `switch_dispatch` | 677.44 ms | 676.73 ms | 606.92 ms | GCC | 1.1162 [1.1114, 1.1436] | pass |
| `struct_copy` | 23.96 ms | 23.80 ms | 25.00 ms | CCC | 1.0093 [0.9954, 1.0133] | pass |
| `loop_patterns` | 77.96 ms | 78.32 ms | 79.30 ms | CCC | 0.9906 [0.9676, 1.0699] | pass |
| `fannkuch` | 3.3427 s | 3.3089 s | 3.2167 s | GCC | 1.0247 [0.9030, 1.0823] | pass |
| `ackermann` | 2.44 ms | 2.49 ms | 186.85 ms | CCC | 1.0278 [0.9423, 1.1086] | pass |
| `constant_recursion` | 2.50 ms | 2.55 ms | 186.91 ms | CCC | 0.9627 [0.8913, 1.0894] | pass |
| `bitops` | 197.65 ms | 203.20 ms | 345.88 ms | CCC | 0.9989 [0.7724, 1.0660] | pass |
| `double_reduction` | 109.35 ms | 109.30 ms | 111.92 ms | CCC | 1.0137 [0.9922, 1.0408] | pass |
| `ascii_case_fold` | 2.54 ms | 2.55 ms | 2.52 ms | GCC | 1.0073 [0.9775, 1.1265] | pass |
| `binary_search` | 2.41 ms | 2.37 ms | 2.47 ms | CCC | 1.0172 [0.9514, 1.1572] | pass |
| `ring_fifo` | 3.71 ms | 3.69 ms | 3.69 ms | GCC | 0.9858 [0.9495, 1.1717] | pass |
| `aarch64_select_patterns` | 151.87 ms | 150.41 ms | 150.87 ms | CCC | 1.0076 [0.9996, 1.0300] | pass |
| `histogram` | 2.62 ms | 2.56 ms | 2.53 ms | GCC | 1.0431 [0.9996, 1.0957] | pass |
| `gzip_crc32` | 171.13 ms | 171.23 ms | 195.26 ms | CCC | 1.0005 [0.9986, 1.0043] | pass |
| `libm_round_family` | 244.02 ms | 243.70 ms | 232.08 ms | GCC | 1.0519 [1.0491, 1.0532] | pass |
| `tls_seg_access` | 11.80 ms | 11.91 ms | 11.60 ms | GCC | 1.0187 [0.9849, 1.0244] | pass |
| `zlib_ng_adler32` | 44.51 ms | 44.62 ms | 44.61 ms | GCC | 0.9965 [0.9902, 1.0560] | pass |
| `expat_xml_scan` | 56.81 ms | 56.65 ms | 46.56 ms | GCC | 1.2215 [1.1927, 1.2341] | pass |
| `sqlite_varint` | 32.13 ms | 32.28 ms | 28.38 ms | GCC | 1.1304 [1.1129, 1.1672] | pass |
| `linux_find_bit` | 16.29 ms | 16.26 ms | 12.33 ms | GCC | 1.3183 [1.3063, 1.3275] | pass |
| `glibc_memcmp` | 8.65 ms | 8.61 ms | 8.67 ms | CCC | 1.0065 [0.9914, 1.0094] | pass |
| `chacha20_block` | 291.71 ms | 291.16 ms | 309.79 ms | CCC | 1.0059 [0.9895, 1.0085] | pass |
| `sha256_transform` | 348.94 ms | 350.08 ms | 298.95 ms | GCC | 1.1751 [1.1398, 1.1769] | pass |
| `linux_rbtree` | 18.88 ms | 18.99 ms | 18.67 ms | GCC | 1.0014 [0.9830, 1.0392] | pass |
| `zstd_count` | 12.22 ms | 12.03 ms | 10.75 ms | GCC | 1.1176 [1.1040, 1.1528] | pass |
| `lz4_compress` | 4.03 ms | 3.99 ms | 3.91 ms | GCC | 1.0179 [0.9706, 1.0457] | pass |
| `glibc_strstr` | 5.2596 s | 5.2638 s | 4.9444 s | GCC | 1.0638 [1.0586, 1.0711] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `0.8007`
- Arithmetic mean ratio: `1.6449`
- Best individual ratio: `constant_recursion` = `0.0131`
- Worst individual ratio: `spectral_norm` = `27.8482`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.1414`
- Arithmetic mean ratio: `1.7395`
- Best individual ratio: `constant_recursion` vs `ccc` = `0.9627`
- Worst individual ratio: `spectral_norm` vs `gcc` = `27.8482`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
