# Codegen rank report — gap to best oracle

Static per-function instruction-gap ranking, worst first, from local LCCC
and the Compiler Explorer oracles. These are screening metrics, not PMU
evidence; verify wins with controlled runtime and hardware counters on the
intended target before making claims.

- flags: `-O2 -march=x86-64-v3`

| Gap | Benchmark | Function | LCCC insns | Best insns | Best compiler | LCCC loads | stores | spills | branches | vectors | LCCC calls | Best calls | Comparable |
|---:|---|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|:--:|
| 203 | `zlib_ng_adler32` | `main` | 276 | 73 | icc | 49 | 8 | 15 | 21 | 36 | 1 | 4 | **no** |
| 184 | `nbody` | `main` | 302 | 118 | gcc16.2 | 126 | 30 | 31 | 17 | 29 | 2 | 4 | **no** |
| 86 | `i686_alu_chains` | `main` | 195 | 109 | icc | 60 | 61 | 75 | 13 | 6 | 7 | 8 | **no** |
| 86 | `moving_stats` | `main` | 217 | 131 | gcc16.2 | 36 | 2 | 5 | 8 | 0 | 1 | 1 | yes |
| 76 | `zstd_count` | `main` | 167 | 91 | gcc16.2 | 28 | 3 | 8 | 30 | 4 | 1 | 1 | yes |
| 70 | `glibc_strstr` | `main` | 194 | 124 | gcc16.2 | 36 | 6 | 19 | 26 | 0 | 1 | 2 | **no** |
| 64 | `linux_rbtree` | `main` | 237 | 173 | gcc16.2 | 43 | 43 | 10 | 47 | 0 | 1 | 1 | yes |
| 62 | `strlen_bench` | `main` | 196 | 134 | gcc16.2 | 31 | 10 | 18 | 37 | 0 | 5 | 5 | yes |
| 60 | `struct_copy` | `main` | 136 | 76 | clang | 36 | 31 | 60 | 4 | 1 | 1 | 0 | **no** |
| 59 | `lz4_compress` | `main` | 351 | 292 | gcc16.2 | 95 | 45 | 81 | 55 | 4 | 4 | 4 | yes |
| 58 | `expat_xml_scan` | `main` | 152 | 94 | gcc16.2 | 35 | 7 | 17 | 34 | 0 | 4 | 4 | yes |
| 53 | `sha256_transform` | `main` | 147 | 94 | gcc16.2 | 35 | 37 | 61 | 16 | 19 | 3 | 3 | yes |
| 52 | `loop_patterns` | `main` | 231 | 179 | gcc16.2 | 34 | 4 | 5 | 26 | 69 | 1 | 1 | yes |
| 46 | `conv_u8_3x3` | `main` | 137 | 91 | gcc16.2 | 38 | 1 | 4 | 14 | 0 | 1 | 1 | yes |
| 45 | `fannkuch` | `main` | 154 | 109 | icx | 41 | 22 | 7 | 22 | 3 | 1 | 0 | **no** |
| 43 | `double_reduction` | `main` | 135 | 92 | gcc16.2 | 26 | 4 | 7 | 10 | 44 | 1 | 1 | yes |
| 43 | `ring_fifo` | `main` | 56 | 13 | gcc16.2 | 5 | 0 | 0 | 10 | 0 | 0 | 0 | yes |
| 43 | `sqlite_varint` | `main` | 181 | 138 | clang | 35 | 7 | 22 | 30 | 0 | 5 | 0 | **no** |
| 38 | `chacha20_block` | `chacha20_core` | 101 | 63 | icx | 14 | 4 | 11 | 11 | 58 | 0 | 0 | yes |
| 26 | `glibc_memcmp` | `main` | 102 | 76 | gcc16.2 | 17 | 7 | 13 | 12 | 4 | 4 | 4 | yes |
| 26 | `prefix_scan` | `main` | 81 | 55 | gcc16.2 | 25 | 0 | 0 | 8 | 0 | 1 | 1 | yes |
| 25 | `matmul` | `matmul` | 46 | 21 | gcc16.2 | 21 | 0 | 1 | 7 | 12 | 0 | 0 | yes |
| 25 | `vecreg_new_ops` | `sat_kernel` | 40 | 15 | gcc16.2 | 6 | 3 | 3 | 3 | 5 | 0 | 0 | yes |
| 23 | `fir_filter` | `main` | 106 | 83 | gcc16.2 | 24 | 8 | 0 | 8 | 0 | 1 | 1 | yes |
| 23 | `libm_round_family` | `round_family_pass` | 50 | 27 | gcc16.2 | 11 | 0 | 0 | 3 | 6 | 0 | 0 | yes |
| 22 | `chacha20_block` | `main` | 92 | 70 | clang | 13 | 18 | 29 | 14 | 0 | 3 | 0 | **no** |
| 20 | `base64_enc` | `main` | 113 | 93 | icc | 21 | 0 | 0 | 16 | 0 | 1 | 2 | **no** |
| 20 | `binary_search` | `main` | 71 | 51 | gcc16.2 | 9 | 0 | 0 | 17 | 0 | 0 | 0 | yes |
| 20 | `csv_field_sum` | `main` | 130 | 110 | gcc16.2 | 21 | 3 | 6 | 19 | 0 | 4 | 4 | yes |
| 19 | `linux_find_bit` | `main` | 103 | 84 | gcc16.2 | 19 | 0 | 0 | 14 | 0 | 4 | 4 | yes |
| 18 | `hash_table` | `main` | 68 | 50 | gcc16.2 | 4 | 0 | 0 | 15 | 0 | 5 | 5 | yes |
| 18 | `switch_dispatch` | `main` | 88 | 70 | gcc16.2 | 13 | 0 | 0 | 22 | 0 | 1 | 1 | yes |
| 17 | `reduction_vecreg` | `main` | 78 | 61 | icx | 15 | 2 | 0 | 11 | 0 | 4 | 0 | **no** |
| 16 | `aarch64_select_patterns` | `main` | 50 | 34 | gcc16.2 | 2 | 0 | 0 | 7 | 0 | 4 | 4 | yes |
| 16 | `libm_round_family` | `trunc` | 17 | 1 | icx | 7 | 0 | 0 | 3 | 2 | 0 | 0 | yes |
| 13 | `tls_seg_access` | `tls_pass` | 32 | 19 | gcc16.2 | 6 | 1 | 0 | 3 | 0 | 0 | 0 | yes |
| 11 | `global_addr_pressure` | `kernel` | 39 | 28 | gcc16.2 | 10 | 0 | 0 | 6 | 0 | 3 | 3 | yes |
| 11 | `i686_alu_chains` | `k_digits` | 34 | 23 | icc | 2 | 0 | 0 | 5 | 0 | 0 | 0 | yes |
| 11 | `libm_round_family` | `floor` | 12 | 1 | icx | 6 | 0 | 0 | 1 | 2 | 0 | 0 | yes |
| 11 | `libm_round_family` | `ceil` | 12 | 1 | icx | 6 | 0 | 0 | 1 | 2 | 0 | 0 | yes |
| 8 | `mandelbrot` | `main` | 55 | 47 | gcc16.2 | 6 | 0 | 0 | 9 | 1 | 1 | 1 | yes |
| 7 | `binary_trees` | `main` | 82 | 75 | icx | 6 | 1 | 3 | 21 | 0 | 12 | 0 | **no** |
| 7 | `fp_memfold_stencil5` | `main` | 78 | 71 | icx | 15 | 2 | 7 | 12 | 0 | 3 | 0 | **no** |
| 7 | `global_addr_pressure` | `main` | 45 | 38 | gcc16.2 | 8 | 0 | 0 | 8 | 0 | 3 | 3 | yes |
| 7 | `tls_seg_access` | `main` | 24 | 17 | gcc16.2 | 1 | 0 | 0 | 5 | 0 | 2 | 2 | yes |
| 6 | `aarch64_select_patterns` | `select_pressure` | 12 | 6 | gcc16.2 | 0 | 0 | 0 | 1 | 0 | 0 | 0 | yes |
| 6 | `gzip_crc32` | `main` | 56 | 50 | gcc16.2 | 8 | 0 | 0 | 10 | 0 | 3 | 1 | **no** |
| 6 | `libm_round_family` | `rint` | 7 | 1 | icx | 5 | 0 | 0 | 1 | 1 | 0 | 0 | yes |
| 6 | `libm_round_family` | `nearbyint` | 7 | 1 | icx | 5 | 0 | 0 | 1 | 1 | 0 | 0 | yes |
| 6 | `libm_round_family` | `roundeven` | 7 | 1 | icx | 5 | 0 | 0 | 1 | 1 | 0 | 0 | yes |
| 6 | `sha256_transform` | `sha256_transform` | 132 | 126 | clang | 24 | 2 | 9 | 7 | 45 | 0 | 0 | yes |
| 6 | `sqlite_varint` | `sqlite_get_varint` | 121 | 115 | clang | 10 | 9 | 0 | 17 | 0 | 0 | 0 | yes |
| 5 | `hash_table` | `insert` | 46 | 41 | gcc16.2 | 6 | 6 | 0 | 6 | 0 | 1 | 1 | yes |
| 5 | `sieve` | `count_primes` | 39 | 34 | gcc16.2 | 6 | 2 | 0 | 9 | 0 | 1 | 1 | yes |
| 4 | `ascii_case_fold` | `main` | 111 | 107 | clang | 26 | 0 | 1 | 9 | 0 | 0 | 0 | yes |
| 4 | `binary_trees` | `destroy` | 15 | 11 | clang | 2 | 0 | 0 | 5 | 0 | 3 | 0 | **no** |
| 4 | `i686_alu_chains` | `k_srem16` | 20 | 16 | gcc16.2 | 3 | 0 | 0 | 3 | 0 | 0 | 0 | yes |
| 4 | `qsort` | `main` | 27 | 23 | gcc16.2 | 7 | 0 | 0 | 5 | 0 | 2 | 2 | yes |
| 4 | `vecreg_new_ops` | `main` | 38 | 34 | gcc16.2 | 6 | 0 | 1 | 9 | 0 | 3 | 3 | yes |
| 3 | `i686_alu_chains` | `k_urem10` | 20 | 17 | icc | 2 | 0 | 0 | 3 | 0 | 0 | 0 | yes |
| 3 | `i686_alu_chains` | `k_udr10` | 20 | 17 | icc | 4 | 0 | 0 | 3 | 0 | 0 | 0 | yes |
| 3 | `i686_alu_chains` | `k_mul45` | 13 | 10 | gcc16.2 | 2 | 0 | 0 | 3 | 0 | 0 | 0 | yes |
| 2 | `hash_table` | `lookup` | 27 | 25 | gcc16.2 | 4 | 1 | 0 | 5 | 0 | 0 | 0 | yes |
| 2 | `i686_alu_chains` | `k_srem7` | 22 | 20 | icc | 1 | 0 | 0 | 3 | 0 | 0 | 0 | yes |
| 2 | `i686_alu_chains` | `k_mul24` | 13 | 11 | gcc16.2 | 1 | 0 | 0 | 3 | 0 | 0 | 0 | yes |
| 2 | `i686_alu_chains` | `k_mulm3` | 13 | 11 | gcc16.2 | 1 | 0 | 0 | 3 | 0 | 0 | 0 | yes |
| 1 | `arith_loop` | `main` | 12 | 11 | gcc16.2 | 2 | 1 | 2 | 3 | 0 | 2 | 2 | yes |
| 1 | `binary_trees` | `make` | 26 | 25 | icx | 0 | 4 | 0 | 6 | 0 | 3 | 0 | **no** |
| 1 | `fib` | `main` | 12 | 11 | clang | 2 | 1 | 2 | 3 | 0 | 2 | 0 | **no** |
| 1 | `i686_alu_chains` | `k_urem7` | 20 | 19 | icc | 1 | 0 | 0 | 3 | 0 | 0 | 0 | yes |
| 1 | `i686_alu_chains` | `k_udiv7` | 19 | 18 | icc | 2 | 0 | 0 | 3 | 0 | 0 | 0 | yes |
| 1 | `i686_alu_chains` | `k_sdr7` | 24 | 23 | gcc16.2 | 2 | 0 | 0 | 3 | 0 | 0 | 0 | yes |
| 1 | `i686_alu_chains` | `k_mul17` | 13 | 12 | gcc16.2 | 0 | 0 | 0 | 3 | 0 | 0 | 0 | yes |
| 1 | `i686_alu_chains` | `k_mul1000` | 11 | 10 | gcc16.2 | 0 | 0 | 0 | 3 | 0 | 0 | 0 | yes |
| 1 | `sieve` | `main` | 12 | 11 | gcc16.2 | 2 | 1 | 2 | 3 | 0 | 2 | 2 | yes |
| 1 | `tce_sum` | `main` | 11 | 10 | gcc16.2 | 2 | 1 | 2 | 2 | 0 | 1 | 1 | yes |

## 16 rows are not comparable (776 of 1897 of the deficit)

A per-function instruction gap is only a codegen gap if both
compilers put the same work in that function. When the call counts
differ, the smaller body has moved work out of line and this
single-function view never measured where it went. On this corpus
that is not a corner case: it is the top of the table.

| Gap | Benchmark | Function | LCCC calls | Best calls |
|---:|---|---|---:|---:|
| 203 | `zlib_ng_adler32` | `main` | 1 | 4 |
| 184 | `nbody` | `main` | 2 | 4 |
| 86 | `i686_alu_chains` | `main` | 7 | 8 |
| 70 | `glibc_strstr` | `main` | 1 | 2 |
| 60 | `struct_copy` | `main` | 1 | 0 |
| 45 | `fannkuch` | `main` | 1 | 0 |
| 43 | `sqlite_varint` | `main` | 5 | 0 |
| 22 | `chacha20_block` | `main` | 3 | 0 |
| 20 | `base64_enc` | `main` | 1 | 2 |
| 17 | `reduction_vecreg` | `main` | 4 | 0 |
| 7 | `fp_memfold_stencil5` | `main` | 3 | 0 |
| 7 | `binary_trees` | `main` | 12 | 0 |
| 6 | `gzip_crc32` | `main` | 3 | 1 |
| 4 | `binary_trees` | `destroy` | 3 | 0 |
| 1 | `fib` | `main` | 2 | 0 |
| 1 | `binary_trees` | `make` | 3 | 0 |

Re-measure these with `--all-functions` before acting on them.
Worked example, method, and the corpus-wide consequences:
`engineering/evidence/ORACLE-METRIC-1/README.md`.
