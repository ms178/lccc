# LCCC benchmark report

- **UTC:** `2026-09-27T21:20:44.995231+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `685da054dbd329148238452f29fa8ec2be45d690`
- **LCCC binary SHA-256:** `8786ceb5a99bf3d083afb5e91191591ef25c889fb4fe6eea3f000cf788cb90ee`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 129.49 ms | 124.77 ms | GCC | 1.0600 [0.9477, 1.0845] | pass |
| `fib` | 2.73 ms | 222.90 ms | GCC | 0.0137 [0.0112, 0.0145] | pass |
| `matmul` | 7.89 ms | 7.71 ms | GCC | 1.0307 [0.9918, 1.0677] | pass |
| `qsort` | 133.03 ms | 138.25 ms | GCC | 1.0062 [0.9166, 1.0423] | pass |
| `sieve` | 49.98 ms | 46.34 ms | GCC | 1.1009 [0.8746, 1.2016] | pass |
| `tce_sum` | 2.74 ms | 2.61 ms | GCC | 1.0158 [0.9047, 1.2744] | pass |
| `nbody` | 527.72 ms | 374.41 ms | GCC | 1.4141 [1.4074, 1.5479] | pass |
| `binary_trees` | 1.9162 s | 1.7333 s | GCC | 1.0999 [1.0742, 1.1624] | pass |
| `spectral_norm` | 265.50 ms | 249.17 ms | GCC | 1.0474 [0.9590, 1.0699] | pass |
| `mandelbrot` | 1.4454 s | 1.6011 s | GCC | 0.9033 [0.8829, 0.9219] | pass |
| `hash_table` | 15.8172 s | 13.4127 s | GCC | 1.1761 [1.0761, 1.2229] | pass |
| `strlen_bench` | 309.33 ms | 288.98 ms | GCC | 1.0423 [0.9984, 1.1120] | pass |
| `switch_dispatch` | 579.66 ms | 549.52 ms | GCC | 1.0737 [1.0365, 1.0884] | pass |
| `struct_copy` | 27.18 ms | 31.94 ms | GCC | 0.8087 [0.7744, 0.8906] | pass |
| `loop_patterns` | 75.72 ms | 82.31 ms | GCC | 0.9567 [0.8526, 1.0274] | pass |
| `fannkuch` | 3.0397 s | 2.6815 s | GCC | 1.1463 [1.0811, 1.1633] | pass |
| `ackermann` | 2.56 ms | 186.21 ms | GCC | 0.0145 [0.0130, 0.0152] | pass |
| `constant_recursion` | 2.72 ms | 164.11 ms | GCC | 0.0164 [0.0152, 0.0172] | pass |
| `bitops` | 275.20 ms | 479.22 ms | GCC | 0.5712 [0.5391, 0.6347] | pass |
| `double_reduction` | 101.54 ms | 135.50 ms | GCC | 0.7390 [0.7267, 0.7831] | pass |
| `ascii_case_fold` | 2.80 ms | 2.84 ms | GCC | 0.9223 [0.8473, 1.1543] | pass |
| `binary_search` | 2.62 ms | 2.40 ms | GCC | 1.0595 [0.9473, 1.1521] | pass |
| `ring_fifo` | 3.15 ms | 3.03 ms | GCC | 1.0097 [0.8866, 1.0743] | pass |
| `aarch64_select_patterns` | 176.46 ms | 153.23 ms | GCC | 1.1034 [1.0500, 1.1771] | pass |
| `histogram` | 2.86 ms | 2.84 ms | GCC | 0.9702 [0.8362, 1.0231] | pass |
| `gzip_crc32` | 152.89 ms | 169.04 ms | GCC | 0.9048 [0.9006, 0.9068] | pass |
| `libm_round_family` | 301.31 ms | 1.2740 s | GCC | 0.2310 [0.2211, 0.2435] | pass |
| `tls_seg_access` | 12.93 ms | 12.19 ms | GCC | 1.0588 [1.0334, 1.0639] | pass |
| `zlib_ng_adler32` | 40.28 ms | 40.01 ms | GCC | 1.0125 [0.9950, 1.0161] | pass |
| `expat_xml_scan` | 52.16 ms | 41.52 ms | GCC | 1.2464 [1.0451, 1.3061] | pass |
| `sqlite_varint` | 36.75 ms | 37.52 ms | GCC | 0.9702 [0.8251, 1.0793] | pass |
| `linux_find_bit` | 18.57 ms | 18.90 ms | GCC | 1.0365 [0.9533, 1.7625] | pass |
| `linux_find_bit_scaled` | 1.0580 s | 932.94 ms | GCC | 1.1616 [1.0778, 1.2352] | pass |
| `glibc_memcmp` | 11.78 ms | 11.73 ms | GCC | 1.0285 [0.9046, 1.2411] | pass |
| `chacha20_block` | 252.20 ms | 317.40 ms | GCC | 0.7946 [0.7732, 0.8930] | pass |
| `sha256_transform` | 322.90 ms | 344.14 ms | GCC | 0.9596 [0.8536, 0.9990] | pass |
| `linux_rbtree` | 18.44 ms | 17.97 ms | GCC | 1.0520 [0.9836, 1.2031] | pass |
| `zstd_count` | 15.01 ms | 13.50 ms | GCC | 1.1579 [1.0293, 1.3283] | pass |
| `lz4_compress` | 3.91 ms | 3.99 ms | GCC | 1.0061 [0.9481, 1.0672] | pass |
| `lz4_match_extend` | 2.3320 s | 1.2506 s | GCC | 1.9367 [1.7866, 2.0592] | pass |
| `glibc_strstr` | 4.3951 s | 4.6323 s | GCC | 0.9491 [0.9451, 0.9520] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `0.7239`
- Arithmetic mean ratio: `0.9465`
- Best individual ratio: `fib` = `0.0137`
- Worst individual ratio: `lz4_match_extend` = `1.9367`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `0.7239`
- Arithmetic mean ratio: `0.9465`
- Best individual ratio: `fib` vs `gcc` = `0.0137`
- Worst individual ratio: `lz4_match_extend` vs `gcc` = `1.9367`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
