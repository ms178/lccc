# LCCC benchmark report

- **UTC:** `2026-09-25T21:08:18.131676+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `244696482818927800d4830b1dc7cbc99708a237`
- **LCCC binary SHA-256:** `4f18a87a051b3acc85d39a97d476f9242534734ce14ec82c5033fa0fc99729b1`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 118.10 ms | 119.67 ms | GCC | 0.9929 [0.9716, 1.0107] | pass |
| `fib` | 2.53 ms | 174.24 ms | GCC | 0.0145 [0.0140, 0.0146] | pass |
| `matmul` | 5.40 ms | 5.37 ms | GCC | 0.9937 [0.8042, 1.0461] | pass |
| `qsort` | 148.87 ms | 148.26 ms | GCC | 1.0048 [0.9984, 1.0277] | pass |
| `sieve` | 47.46 ms | 43.96 ms | GCC | 1.0783 [1.0440, 1.1036] | pass |
| `tce_sum` | 2.37 ms | 2.38 ms | GCC | 0.9837 [0.9689, 1.0574] | pass |
| `nbody` | 283.82 ms | 263.36 ms | GCC | 1.0784 [1.0713, 1.0993] | pass |
| `binary_trees` | 1.6355 s | 1.3608 s | GCC | 1.1859 [1.0911, 1.2257] | pass |
| `spectral_norm` | 6.3958 s | 230.15 ms | GCC | 27.7116 [26.8569, 28.0781] | pass |
| `mandelbrot` | 1.3482 s | 1.0487 s | GCC | 1.2855 [1.2807, 1.2869] | pass |
| `hash_table` | 13.5447 s | 13.5384 s | GCC | 1.0004 [0.9827, 1.0226] | pass |
| `strlen_bench` | 278.40 ms | 276.23 ms | GCC | 1.0254 [0.9791, 1.0504] | pass |
| `switch_dispatch` | 677.10 ms | 610.91 ms | GCC | 1.1091 [1.0819, 1.1319] | pass |
| `struct_copy` | 23.69 ms | 25.14 ms | GCC | 0.9481 [0.9422, 0.9807] | pass |
| `loop_patterns` | 74.45 ms | 74.99 ms | GCC | 0.9999 [0.9648, 1.0286] | pass |
| `fannkuch` | 3.2809 s | 3.1685 s | GCC | 1.0369 [1.0272, 1.0375] | pass |
| `ackermann` | 2.51 ms | 185.59 ms | GCC | 0.0135 [0.0124, 0.0153] | pass |
| `constant_recursion` | 2.45 ms | 186.28 ms | GCC | 0.0131 [0.0121, 0.0136] | pass |
| `bitops` | 195.32 ms | 346.39 ms | GCC | 0.5643 [0.5536, 0.5814] | pass |
| `double_reduction` | 110.47 ms | 107.98 ms | GCC | 1.0099 [1.0083, 1.0249] | pass |
| `ascii_case_fold` | 2.45 ms | 2.47 ms | GCC | 1.0184 [0.8900, 1.0818] | pass |
| `binary_search` | 2.39 ms | 2.45 ms | GCC | 0.9645 [0.9139, 1.0222] | pass |
| `ring_fifo` | 2.27 ms | 2.35 ms | GCC | 0.9771 [0.8880, 1.0258] | pass |
| `aarch64_select_patterns` | 152.14 ms | 151.12 ms | GCC | 1.0011 [0.9909, 1.0174] | pass |
| `histogram` | 2.66 ms | 2.78 ms | GCC | 1.0326 [0.8431, 1.0806] | pass |
| `gzip_crc32` | 171.44 ms | 195.50 ms | GCC | 0.8756 [0.8747, 0.8842] | pass |
| `libm_round_family` | 244.46 ms | 232.34 ms | GCC | 1.0509 [1.0369, 1.0595] | pass |
| `tls_seg_access` | 11.85 ms | 12.07 ms | GCC | 0.9909 [0.9063, 1.0165] | pass |
| `zlib_ng_adler32` | 44.68 ms | 44.76 ms | GCC | 1.0032 [0.9910, 1.0059] | pass |
| `expat_xml_scan` | 59.59 ms | 48.07 ms | GCC | 1.2420 [1.2329, 1.2741] | pass |
| `sqlite_varint` | 31.80 ms | 27.88 ms | GCC | 1.1308 [1.1224, 1.1520] | pass |
| `linux_find_bit` | 16.02 ms | 12.14 ms | GCC | 1.3122 [1.2999, 1.3426] | pass |
| `glibc_memcmp` | 8.55 ms | 8.54 ms | GCC | 1.0003 [0.9420, 1.0160] | pass |
| `chacha20_block` | 291.16 ms | 310.74 ms | GCC | 0.9386 [0.9267, 0.9448] | pass |
| `sha256_transform` | 348.28 ms | 296.21 ms | GCC | 1.1767 [1.1746, 1.1789] | pass |
| `linux_rbtree` | 18.77 ms | 18.52 ms | GCC | 1.0154 [0.9887, 1.0322] | pass |
| `zstd_count` | 11.77 ms | 10.52 ms | GCC | 1.1188 [1.1010, 1.1227] | pass |
| `lz4_compress` | 4.02 ms | 3.87 ms | GCC | 1.0427 [0.9922, 1.0724] | pass |
| `glibc_strstr` | 5.2416 s | 4.9285 s | GCC | 1.0621 [1.0580, 1.0719] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `0.8020`
- Arithmetic mean ratio: `1.6411`
- Best individual ratio: `constant_recursion` = `0.0131`
- Worst individual ratio: `spectral_norm` = `27.7116`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `0.8020`
- Arithmetic mean ratio: `1.6411`
- Best individual ratio: `constant_recursion` vs `gcc` = `0.0131`
- Worst individual ratio: `spectral_norm` vs `gcc` = `27.7116`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
