# LCCC benchmark report

- **UTC:** `2026-09-26T01:14:05.080265+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `e5bc19118c6ea5a54d5539d4a6cff52c97ab6baa`
- **LCCC binary SHA-256:** `d071563d5326f5e1d0a8e3f5aa035e0079843868da16abe9cd2ee16484e3f818`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | CCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 119.14 ms | 117.85 ms | 121.30 ms | CCC | 1.0213 [0.9746, 1.0386] | pass |
| `fib` | 2.43 ms | 2.69 ms | 176.36 ms | CCC | 0.9571 [0.8536, 1.0114] | pass |
| `matmul` | 5.33 ms | 5.38 ms | 5.32 ms | GCC | 1.0049 [0.9729, 1.0293] | pass |
| `qsort` | 151.44 ms | 149.60 ms | 151.19 ms | CCC | 1.0105 [0.9980, 1.0214] | pass |
| `sieve` | 47.12 ms | 46.86 ms | 43.97 ms | GCC | 1.0780 [1.0549, 1.1045] | pass |
| `tce_sum` | 2.42 ms | 2.45 ms | 2.42 ms | GCC | 1.0198 [0.8292, 1.1290] | pass |
| `nbody` | 285.25 ms | 284.58 ms | 264.68 ms | GCC | 1.0777 [1.0747, 1.0992] | pass |
| `binary_trees` | 1.5182 s | 1.5605 s | 1.3556 s | GCC | 1.1639 [1.0373, 1.2186] | pass |
| `spectral_norm` | 241.16 ms | 6.5429 s | 235.94 ms | GCC | 1.0248 [1.0209, 1.0356] | pass |
| `mandelbrot` | 1.3465 s | 1.3433 s | 1.0504 s | GCC | 1.2984 [1.2743, 1.3132] | pass |
| `hash_table` | 13.3465 s | 13.7179 s | 13.3011 s | GCC | 1.0058 [0.9787, 1.0319] | pass |
| `strlen_bench` | 263.19 ms | 261.40 ms | 257.17 ms | GCC | 1.0176 [0.9864, 1.0282] | pass |
| `switch_dispatch` | 677.86 ms | 686.82 ms | 612.86 ms | GCC | 1.1132 [1.0869, 1.1215] | pass |
| `struct_copy` | 24.71 ms | 24.65 ms | 26.63 ms | CCC | 0.9992 [0.9540, 1.1562] | pass |
| `loop_patterns` | 75.11 ms | 75.35 ms | 77.17 ms | CCC | 0.9967 [0.9701, 1.0423] | pass |
| `fannkuch` | 3.2820 s | 3.2860 s | 3.1746 s | GCC | 1.0302 [1.0169, 1.0441] | pass |
| `ackermann` | 2.44 ms | 2.40 ms | 193.20 ms | CCC | 0.9554 [0.8155, 1.1874] | pass |
| `constant_recursion` | 2.39 ms | 2.48 ms | 194.45 ms | CCC | 0.9630 [0.9098, 1.0755] | pass |
| `bitops` | 196.74 ms | 195.94 ms | 343.27 ms | CCC | 1.0050 [0.9479, 1.0407] | pass |
| `double_reduction` | 110.07 ms | 109.97 ms | 110.98 ms | CCC | 0.9867 [0.9696, 1.0670] | pass |
| `ascii_case_fold` | 2.56 ms | 2.61 ms | 2.60 ms | GCC | 1.0134 [0.9090, 1.0541] | pass |
| `binary_search` | 2.41 ms | 2.56 ms | 2.59 ms | CCC | 0.9307 [0.8879, 1.0023] | pass |
| `ring_fifo` | 2.54 ms | 2.69 ms | 2.88 ms | CCC | 0.9361 [0.7240, 1.0424] | pass |
| `aarch64_select_patterns` | 152.21 ms | 153.07 ms | 149.85 ms | GCC | 1.0186 [1.0000, 1.0521] | pass |
| `histogram` | 2.60 ms | 2.56 ms | 2.50 ms | GCC | 1.0313 [0.9712, 1.0575] | pass |
| `gzip_crc32` | 171.55 ms | 171.60 ms | 196.46 ms | CCC | 0.9997 [0.9950, 1.0015] | pass |
| `libm_round_family` | 245.01 ms | 244.82 ms | 233.71 ms | GCC | 1.0534 [1.0238, 1.0744] | pass |
| `tls_seg_access` | 12.14 ms | 11.98 ms | 11.78 ms | GCC | 1.0281 [1.0101, 1.1002] | pass |
| `zlib_ng_adler32` | 45.04 ms | 45.48 ms | 44.88 ms | GCC | 0.9986 [0.9828, 1.0324] | pass |
| `expat_xml_scan` | 58.21 ms | 59.54 ms | 46.93 ms | GCC | 1.2304 [1.1454, 1.2404] | pass |
| `sqlite_varint` | 32.49 ms | 32.29 ms | 28.95 ms | GCC | 1.1261 [1.0600, 1.1647] | pass |
| `linux_find_bit` | 16.32 ms | 16.40 ms | 12.23 ms | GCC | 1.3343 [1.3155, 1.3427] | pass |
| `linux_find_bit_scaled` | 885.78 ms | 887.36 ms | 630.07 ms | GCC | 1.3990 [1.3838, 1.4530] | pass |
| `glibc_memcmp` | 9.39 ms | 9.32 ms | 8.94 ms | GCC | 1.0740 [0.8823, 1.3314] | pass |
| `chacha20_block` | 292.24 ms | 293.66 ms | 314.35 ms | CCC | 0.9960 [0.9690, 1.0063] | pass |
| `sha256_transform` | 357.15 ms | 353.63 ms | 302.79 ms | GCC | 1.1787 [1.1677, 1.2137] | pass |
| `linux_rbtree` | 18.82 ms | 18.99 ms | 18.56 ms | GCC | 1.0149 [0.9733, 1.0392] | pass |
| `zstd_count` | 12.16 ms | 12.13 ms | 11.05 ms | GCC | 1.0983 [1.0392, 1.1391] | pass |
| `lz4_compress` | 4.11 ms | 4.04 ms | 4.02 ms | GCC | 1.0181 [0.9961, 1.0458] | pass |
| `lz4_match_extend` | 1.4311 s | 1.4512 s | 1.1214 s | GCC | 1.2896 [1.2482, 1.3423] | pass |
| `glibc_strstr` | 5.2825 s | 5.2478 s | 4.9300 s | GCC | 1.0684 [1.0630, 1.0732] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `0.7588`
- Arithmetic mean ratio: `0.9788`
- Best individual ratio: `constant_recursion` = `0.0125`
- Worst individual ratio: `linux_find_bit_scaled` = `1.3990`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.0576`
- Arithmetic mean ratio: `1.0626`
- Best individual ratio: `binary_search` vs `ccc` = `0.9307`
- Worst individual ratio: `linux_find_bit_scaled` vs `gcc` = `1.3990`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
