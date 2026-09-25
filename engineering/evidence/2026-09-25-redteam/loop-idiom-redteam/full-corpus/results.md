# LCCC benchmark report

- **UTC:** `2026-09-25T21:53:47.012072+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `244696482818927800d4830b1dc7cbc99708a237`
- **LCCC binary SHA-256:** `fbb696f1030fc4c020a6e22beb12bfefc6e849941733533423d57d11c3f80066`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | CCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 123.83 ms | 125.56 ms | CCC | 0.9956 [0.9901, 1.0208] | pass |
| `fib` | 2.87 ms | 2.72 ms | CCC | 1.0768 [0.9784, 1.1052] | pass |
| `matmul` | 6.09 ms | 6.31 ms | CCC | 0.9960 [0.7567, 1.4181] | pass |
| `qsort` | 160.86 ms | 160.79 ms | CCC | 0.9805 [0.9646, 0.9953] | pass |
| `sieve` | 55.87 ms | 54.40 ms | CCC | 1.0448 [0.9419, 1.0679] | pass |
| `tce_sum` | 3.08 ms | 3.00 ms | CCC | 1.0140 [0.9735, 1.0494] | pass |
| `nbody` | 326.00 ms | 341.61 ms | CCC | 0.9704 [0.9535, 1.0308] | pass |
| `binary_trees` | 1.8338 s | 1.9785 s | CCC | 0.9349 [0.8770, 1.0028] | pass |
| `spectral_norm` | 7.0822 s | 7.0863 s | CCC | 0.9941 [0.9794, 1.0154] | pass |
| `mandelbrot` | 1.3834 s | 1.3749 s | CCC | 1.0046 [0.9987, 1.0127] | pass |
| `hash_table` | 14.5583 s | 14.6280 s | CCC | 0.9920 [0.9512, 1.0452] | pass |
| `strlen_bench` | 274.25 ms | 268.53 ms | CCC | 1.0270 [0.9831, 1.0644] | pass |
| `switch_dispatch` | 682.84 ms | 681.58 ms | CCC | 0.9985 [0.9887, 1.0086] | pass |
| `struct_copy` | 24.16 ms | 24.52 ms | CCC | 0.9870 [0.8284, 1.0015] | pass |
| `loop_patterns` | 78.69 ms | 79.87 ms | CCC | 0.9885 [0.9802, 1.0270] | pass |
| `fannkuch` | 3.3165 s | 3.3315 s | CCC | 1.0012 [0.9745, 1.0094] | pass |
| `ackermann` | 2.50 ms | 2.66 ms | CCC | 0.9598 [0.7702, 1.1812] | pass |
| `constant_recursion` | 2.38 ms | 2.47 ms | CCC | 1.0250 [0.8870, 1.0827] | pass |
| `bitops` | 209.19 ms | 204.40 ms | CCC | 1.0505 [0.9767, 1.1175] | pass |
| `double_reduction` | 118.10 ms | 116.17 ms | CCC | 1.0125 [0.9641, 1.0346] | pass |
| `ascii_case_fold` | 3.16 ms | 2.82 ms | CCC | 1.0031 [0.9642, 1.2279] | pass |
| `binary_search` | 2.62 ms | 2.47 ms | CCC | 1.0104 [0.9860, 1.2174] | pass |
| `ring_fifo` | 2.36 ms | 2.35 ms | CCC | 1.0346 [0.9683, 1.0742] | pass |
| `aarch64_select_patterns` | 155.57 ms | 153.61 ms | CCC | 1.0046 [0.9836, 1.0172] | pass |
| `histogram` | 2.62 ms | 2.61 ms | CCC | 1.0160 [0.8160, 1.0743] | pass |
| `gzip_crc32` | 172.26 ms | 171.46 ms | CCC | 1.0041 [1.0009, 1.0086] | pass |
| `libm_round_family` | 244.11 ms | 244.38 ms | CCC | 1.0014 [0.9940, 1.0044] | pass |
| `tls_seg_access` | 11.82 ms | 11.86 ms | CCC | 0.9913 [0.9773, 1.0163] | pass |
| `zlib_ng_adler32` | 45.65 ms | 45.46 ms | CCC | 1.0056 [0.9780, 1.0177] | pass |
| `expat_xml_scan` | 58.52 ms | 56.85 ms | CCC | 1.0044 [0.9830, 1.0468] | pass |
| `sqlite_varint` | 31.74 ms | 32.01 ms | CCC | 0.9856 [0.9751, 0.9983] | pass |
| `linux_find_bit` | 16.32 ms | 16.31 ms | CCC | 1.0033 [0.9731, 1.0249] | pass |
| `glibc_memcmp` | 8.90 ms | 8.74 ms | CCC | 1.0038 [0.9974, 1.0378] | pass |
| `chacha20_block` | 295.59 ms | 293.48 ms | CCC | 1.0076 [0.9841, 1.0132] | pass |
| `sha256_transform` | 362.46 ms | 361.62 ms | CCC | 1.0077 [0.9732, 1.0199] | pass |
| `linux_rbtree` | 20.01 ms | 19.25 ms | CCC | 1.0264 [0.9770, 1.1107] | pass |
| `zstd_count` | 12.32 ms | 12.44 ms | CCC | 0.9871 [0.9828, 1.0200] | pass |
| `lz4_compress` | 4.04 ms | 4.20 ms | CCC | 0.9687 [0.9306, 1.0547] | pass |
| `glibc_strstr` | 5.2697 s | 5.2766 s | CCC | 1.0000 [0.9949, 1.0090] | pass |

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.0028`
- Arithmetic mean ratio: `1.0031`
- Best individual ratio: `binary_trees` vs `ccc` = `0.9349`
- Worst individual ratio: `fib` vs `ccc` = `1.0768`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
