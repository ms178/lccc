# LCCC benchmark report

- **UTC:** `2026-09-25T08:35:02.957964+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `not probed`
- **LCCC revision:** `d80c0c74f4e844760b1af9e04642201a876c2300`
- **LCCC binary SHA-256:** `e634c905d1cd277afa058746e846bc2dd1f9368c1360de338e6ba9679935b449`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 113.40 ms | 101.31 ms | GCC | 1.0883 [1.0441, 1.1597] | pass |
| `fib` | 2.45 ms | 174.39 ms | GCC | 0.0142 [0.0129, 0.0149] | pass |
| `matmul` | 7.84 ms | 7.09 ms | GCC | 1.0188 [0.7698, 1.1539] | pass |
| `qsort` | 126.42 ms | 126.33 ms | GCC | 0.9993 [0.9949, 1.0061] | pass |
| `sieve` | 40.23 ms | 38.30 ms | GCC | 1.0405 [1.0311, 1.0672] | pass |
| `tce_sum` | 2.11 ms | 2.11 ms | GCC | 1.0114 [0.9349, 1.0324] | pass |
| `nbody` | 366.95 ms | 238.44 ms | GCC | 1.4204 [1.4102, 1.6609] | pass |
| `binary_trees` | 1.6806 s | 1.4435 s | GCC | 1.1472 [1.1378, 1.1573] | pass |
| `spectral_norm` | 476.76 ms | 200.81 ms | GCC | 2.3740 [2.3305, 2.3858] | pass |
| `mandelbrot` | 1.3626 s | 1.1291 s | GCC | 1.2072 [1.2004, 1.2178] | pass |
| `hash_table` | 12.8798 s | 10.4350 s | GCC | 1.2638 [1.1905, 1.3723] | pass |
| `strlen_bench` | 228.37 ms | 226.61 ms | GCC | 1.0179 [1.0043, 1.0329] | pass |
| `switch_dispatch` | 539.76 ms | 506.39 ms | GCC | 1.0717 [1.0632, 1.0819] | pass |
| `struct_copy` | 22.90 ms | 23.62 ms | GCC | 0.9899 [0.9405, 1.0575] | pass |
| `loop_patterns` | 71.26 ms | 77.46 ms | GCC | 0.9019 [0.8227, 0.9909] | pass |
| `fannkuch` | 2.8952 s | 2.6065 s | GCC | 1.1176 [1.0761, 1.1344] | pass |
| `ackermann` | 2.53 ms | 151.75 ms | GCC | 0.0167 [0.0146, 0.0178] | pass |
| `constant_recursion` | 2.52 ms | 153.02 ms | GCC | 0.0161 [0.0144, 0.0165] | pass |
| `bitops` | 224.80 ms | 342.99 ms | GCC | 0.6548 [0.6391, 0.6762] | pass |
| `double_reduction` | 92.64 ms | 92.25 ms | GCC | 1.0017 [0.9888, 1.0076] | pass |
| `ascii_case_fold` | 2.43 ms | 2.35 ms | GCC | 1.0418 [0.9606, 1.1363] | pass |
| `binary_search` | 2.18 ms | 2.26 ms | GCC | 0.9980 [0.9048, 1.0485] | pass |
| `ring_fifo` | 2.18 ms | 2.25 ms | GCC | 0.9620 [0.8718, 1.0062] | pass |
| `aarch64_select_patterns` | 144.62 ms | 129.15 ms | GCC | 1.0987 [1.0812, 1.2049] | pass |
| `histogram` | 2.48 ms | 2.56 ms | GCC | 0.9903 [0.8873, 1.1110] | pass |
| `gzip_crc32` | 152.32 ms | 168.29 ms | GCC | 0.9051 [0.9033, 0.9076] | pass |
| `libm_round_family` | 252.06 ms | 240.44 ms | GCC | 1.0341 [1.0089, 1.0490] | pass |
| `tls_seg_access` | 11.45 ms | 10.91 ms | GCC | 1.0539 [1.0321, 1.1187] | pass |
| `zlib_ng_adler32` | 39.20 ms | 38.95 ms | GCC | 1.0058 [1.0012, 1.0226] | pass |
| `expat_xml_scan` | 50.39 ms | 40.32 ms | GCC | 1.2523 [1.2062, 1.2824] | pass |
| `sqlite_varint` | 28.66 ms | 25.54 ms | GCC | 1.1276 [1.1033, 1.1531] | pass |
| `linux_find_bit` | 20.54 ms | 17.60 ms | GCC | 1.1612 [1.1444, 1.3412] | pass |
| `glibc_memcmp` | 10.23 ms | 9.09 ms | GCC | 1.1369 [1.1120, 1.1898] | pass |
| `chacha20_block` | 255.52 ms | 299.83 ms | GCC | 0.8468 [0.8305, 0.8648] | pass |
| `sha256_transform` | 324.82 ms | 273.25 ms | GCC | 1.1887 [1.1677, 1.2032] | pass |
| `linux_rbtree` | 18.73 ms | 16.79 ms | GCC | 1.0347 [1.0179, 1.2454] | pass |
| `zstd_count` | 14.33 ms | 12.50 ms | GCC | 1.2111 [1.1539, 1.4234] | pass |
| `lz4_compress` | 3.73 ms | 3.69 ms | GCC | 1.0000 [0.9705, 1.0228] | pass |
| `glibc_strstr` | 4.4822 s | 4.4269 s | GCC | 1.0106 [1.0076, 1.0197] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `0.7749`
- Arithmetic mean ratio: `1.0111`
- Best individual ratio: `fib` = `0.0142`
- Worst individual ratio: `spectral_norm` = `2.3740`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `0.7749`
- Arithmetic mean ratio: `1.0111`
- Best individual ratio: `fib` vs `gcc` = `0.0142`
- Worst individual ratio: `spectral_norm` vs `gcc` = `2.3740`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
