# LCCC benchmark report

- **UTC:** `2026-09-25T08:13:35.527617+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `5624367b8366cad6e003fa5ff6021e9f01f12b16`
- **LCCC binary SHA-256:** `9bd637058fa2595e6d5a0d839e905b401e592d94f18abbc6171edaf368f74b49`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 114.76 ms | 100.37 ms | GCC | 1.1314 [1.1270, 1.1711] | pass |
| `fib` | 2.47 ms | 172.24 ms | GCC | 0.0144 [0.0122, 0.0152] | pass |
| `matmul` | 5.74 ms | 5.16 ms | GCC | 1.1015 [1.0647, 1.1916] | pass |
| `qsort` | 125.84 ms | 125.78 ms | GCC | 1.0009 [0.9956, 1.0158] | pass |
| `sieve` | 37.60 ms | 35.88 ms | GCC | 1.0432 [1.0304, 1.0538] | pass |
| `tce_sum` | 2.24 ms | 2.14 ms | GCC | 1.0344 [0.9864, 1.0730] | pass |
| `nbody` | 328.90 ms | 235.53 ms | GCC | 1.4164 [1.3417, 1.4630] | pass |
| `binary_trees` | 1.6301 s | 1.4086 s | GCC | 1.1050 [1.0652, 1.1920] | pass |
| `spectral_norm` | 476.48 ms | 203.19 ms | GCC | 2.3444 [2.3049, 2.3784] | pass |
| `mandelbrot` | 1.3804 s | 1.1307 s | GCC | 1.2104 [1.1758, 1.2256] | pass |
| `hash_table` | 12.3663 s | 9.4978 s | GCC | 1.3020 [1.2094, 1.3340] | pass |
| `strlen_bench` | 251.62 ms | 246.74 ms | GCC | 1.0202 [0.9663, 1.0368] | pass |
| `switch_dispatch` | 540.36 ms | 503.31 ms | GCC | 1.0716 [1.0665, 1.0749] | pass |
| `struct_copy` | 22.95 ms | 23.27 ms | GCC | 0.9865 [0.9582, 0.9956] | pass |
| `loop_patterns` | 65.43 ms | 66.86 ms | GCC | 0.9755 [0.9703, 0.9986] | pass |
| `fannkuch` | 2.8281 s | 2.5671 s | GCC | 1.0996 [1.0972, 1.1049] | pass |
| `ackermann` | 2.37 ms | 154.28 ms | GCC | 0.0157 [0.0149, 0.0168] | pass |
| `constant_recursion` | 2.41 ms | 151.45 ms | GCC | 0.0160 [0.0144, 0.0162] | pass |
| `bitops` | 219.69 ms | 339.05 ms | GCC | 0.6510 [0.6430, 0.6785] | pass |
| `double_reduction` | 93.93 ms | 93.19 ms | GCC | 1.0057 [1.0018, 1.0079] | pass |
| `ascii_case_fold` | 2.44 ms | 2.42 ms | GCC | 1.0334 [0.9426, 1.2079] | pass |
| `binary_search` | 2.45 ms | 2.36 ms | GCC | 0.9802 [0.9368, 1.1212] | pass |
| `ring_fifo` | 2.11 ms | 2.17 ms | GCC | 0.9781 [0.9174, 1.0419] | pass |
| `aarch64_select_patterns` | 138.99 ms | 128.91 ms | GCC | 1.0881 [1.0654, 1.1372] | pass |
| `histogram` | 2.87 ms | 2.56 ms | GCC | 1.0116 [0.8907, 1.2628] | pass |
| `gzip_crc32` | 152.70 ms | 168.71 ms | GCC | 0.9054 [0.8961, 0.9071] | pass |
| `libm_round_family` | 246.76 ms | 243.24 ms | GCC | 1.0030 [0.9701, 1.0098] | pass |
| `tls_seg_access` | 12.98 ms | 11.28 ms | GCC | 1.1203 [1.0404, 1.3692] | pass |
| `zlib_ng_adler32` | 39.77 ms | 39.59 ms | GCC | 0.9999 [0.9933, 1.0306] | pass |
| `expat_xml_scan` | 50.19 ms | 40.08 ms | GCC | 1.2464 [1.2242, 1.2658] | pass |
| `sqlite_varint` | 32.95 ms | 25.59 ms | GCC | 1.1154 [1.1085, 1.1403] | pass |
| `linux_find_bit` | 20.49 ms | 17.69 ms | GCC | 1.1641 [1.1367, 1.3754] | pass |
| `glibc_memcmp` | 10.25 ms | 9.06 ms | GCC | 1.1346 [0.9850, 1.1705] | pass |
| `chacha20_block` | 258.77 ms | 308.43 ms | GCC | 0.8593 [0.8335, 0.8760] | pass |
| `sha256_transform` | 327.08 ms | 274.68 ms | GCC | 1.1847 [1.1452, 1.1987] | pass |
| `linux_rbtree` | 16.53 ms | 16.19 ms | GCC | 1.0163 [1.0136, 1.0241] | pass |
| `zstd_count` | 14.25 ms | 11.91 ms | GCC | 1.1951 [1.1648, 1.4836] | pass |
| `lz4_compress` | 3.69 ms | 3.60 ms | GCC | 1.0349 [1.0115, 1.0465] | pass |
| `glibc_strstr` | 4.4569 s | 4.4105 s | GCC | 1.0113 [1.0105, 1.0126] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `0.7781`
- Arithmetic mean ratio: `1.0161`
- Best individual ratio: `fib` = `0.0144`
- Worst individual ratio: `spectral_norm` = `2.3444`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `0.7781`
- Arithmetic mean ratio: `1.0161`
- Best individual ratio: `fib` vs `gcc` = `0.0144`
- Worst individual ratio: `spectral_norm` vs `gcc` = `2.3444`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
