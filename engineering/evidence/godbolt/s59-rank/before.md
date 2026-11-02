# Codegen rank report — gap to best oracle

Static per-function instruction-gap ranking, worst first, from local LCCC
and the Compiler Explorer oracles. These are screening metrics, not PMU
evidence; verify wins with controlled runtime and hardware counters on the
intended target before making claims.

- flags: `-O2 -march=x86-64-v3`

| Gap | Benchmark | Function | LCCC insns | Best insns | Best compiler | LCCC loads | stores | spills | branches | vectors |
|---:|---|---|---:|---:|---|---:|---:|---:|---:|---:|
| 430 | `linux_rbtree` | `main` | 603 | 173 | gcc16.2 | 126 | 67 | 63 | 119 | 0 |
| 382 | `csv_field_sum` | `main` | 492 | 110 | gcc16.2 | 87 | 27 | 62 | 62 | 0 |
| 284 | `zlib_ng_adler32` | `main` | 357 | 73 | icc | 60 | 18 | 32 | 26 | 79 |
| 204 | `nbody` | `main` | 322 | 118 | gcc16.2 | 130 | 31 | 33 | 17 | 31 |
| 103 | `chacha20_block` | `chacha20_core` | 166 | 63 | icx | 23 | 8 | 16 | 17 | 80 |
| 96 | `sha256_transform` | `main` | 190 | 94 | gcc16.2 | 31 | 37 | 60 | 16 | 23 |
| 95 | `moving_stats` | `main` | 226 | 131 | gcc16.2 | 40 | 2 | 5 | 8 | 0 |
| 88 | `loop_patterns` | `main` | 267 | 179 | gcc16.2 | 43 | 6 | 7 | 28 | 87 |
| 85 | `i686_alu_chains` | `main` | 194 | 109 | icc | 60 | 61 | 75 | 13 | 6 |
| 73 | `glibc_strstr` | `main` | 197 | 124 | gcc16.2 | 35 | 6 | 19 | 26 | 0 |
| 68 | `strlen_bench` | `main` | 202 | 134 | gcc16.2 | 33 | 9 | 19 | 37 | 0 |
| 67 | `struct_copy` | `main` | 143 | 76 | clang | 44 | 31 | 68 | 4 | 2 |
| 63 | `matmul` | `matmul` | 84 | 21 | gcc16.2 | 32 | 3 | 4 | 12 | 12 |
| 62 | `expat_xml_scan` | `main` | 156 | 94 | gcc16.2 | 37 | 7 | 17 | 34 | 0 |
| 53 | `vecreg_new_ops` | `main` | 87 | 34 | gcc16.2 | 6 | 48 | 52 | 9 | 0 |
| 52 | `ring_fifo` | `main` | 65 | 13 | gcc16.2 | 7 | 0 | 0 | 10 | 0 |
| 47 | `double_reduction` | `main` | 139 | 92 | gcc16.2 | 26 | 4 | 7 | 10 | 48 |
| 46 | `conv_u8_3x3` | `main` | 137 | 91 | gcc16.2 | 38 | 1 | 4 | 14 | 0 |
| 45 | `fannkuch` | `main` | 154 | 109 | icx | 40 | 22 | 7 | 22 | 3 |
| 43 | `zstd_count` | `main` | 134 | 91 | gcc16.2 | 18 | 4 | 8 | 22 | 0 |
| 42 | `sha256_transform` | `sha256_transform` | 168 | 126 | clang | 28 | 3 | 8 | 11 | 47 |
| 38 | `sqlite_varint` | `main` | 176 | 138 | clang | 34 | 7 | 22 | 30 | 0 |
| 36 | `libm_round_family` | `trunc` | 37 | 1 | icx | 7 | 0 | 0 | 3 | 0 |
| 34 | `ascii_case_fold` | `main` | 141 | 107 | clang | 26 | 0 | 0 | 9 | 0 |
| 31 | `libm_round_family` | `round_family_pass` | 58 | 27 | gcc16.2 | 14 | 1 | 0 | 3 | 6 |
| 30 | `linux_find_bit` | `main` | 114 | 84 | gcc16.2 | 19 | 6 | 10 | 14 | 0 |
| 26 | `glibc_memcmp` | `main` | 102 | 76 | gcc16.2 | 17 | 7 | 13 | 12 | 4 |
| 26 | `prefix_scan` | `main` | 81 | 55 | gcc16.2 | 25 | 0 | 0 | 8 | 0 |
| 25 | `binary_search` | `main` | 76 | 51 | gcc16.2 | 10 | 0 | 0 | 17 | 0 |
| 25 | `vecreg_new_ops` | `sat_kernel` | 40 | 15 | gcc16.2 | 6 | 3 | 3 | 3 | 5 |
| 24 | `reduction_vecreg` | `main` | 85 | 61 | icx | 15 | 2 | 0 | 11 | 0 |
| 23 | `fir_filter` | `main` | 106 | 83 | gcc16.2 | 24 | 8 | 0 | 8 | 0 |
| 23 | `hash_table` | `main` | 73 | 50 | gcc16.2 | 5 | 0 | 0 | 15 | 0 |
| 22 | `chacha20_block` | `main` | 92 | 70 | clang | 13 | 18 | 29 | 14 | 0 |
| 20 | `base64_enc` | `main` | 113 | 93 | icc | 21 | 0 | 0 | 16 | 0 |
| 18 | `libm_round_family` | `floor` | 19 | 1 | icx | 6 | 0 | 0 | 1 | 1 |
| 18 | `libm_round_family` | `ceil` | 19 | 1 | icx | 6 | 0 | 0 | 1 | 1 |
| 18 | `switch_dispatch` | `main` | 88 | 70 | gcc16.2 | 13 | 0 | 0 | 22 | 0 |
| 17 | `tls_seg_access` | `tls_pass` | 36 | 19 | gcc16.2 | 9 | 3 | 0 | 3 | 0 |
| 16 | `aarch64_select_patterns` | `main` | 50 | 34 | gcc16.2 | 2 | 0 | 0 | 7 | 0 |
| 13 | `sieve` | `count_primes` | 47 | 34 | gcc16.2 | 7 | 2 | 0 | 9 | 0 |
| 11 | `i686_alu_chains` | `k_digits` | 34 | 23 | icc | 2 | 0 | 0 | 5 | 0 |
| 10 | `global_addr_pressure` | `kernel` | 38 | 28 | gcc16.2 | 11 | 0 | 0 | 6 | 0 |
| 9 | `aarch64_select_patterns` | `select_pressure` | 15 | 6 | gcc16.2 | 0 | 0 | 0 | 1 | 0 |
| 9 | `fp_memfold_stencil5` | `main` | 80 | 71 | icx | 15 | 2 | 7 | 12 | 0 |
| 9 | `libm_round_family` | `rint` | 10 | 1 | icx | 5 | 0 | 0 | 1 | 1 |
| 9 | `libm_round_family` | `nearbyint` | 10 | 1 | icx | 5 | 0 | 0 | 1 | 1 |
| 9 | `libm_round_family` | `roundeven` | 10 | 1 | icx | 5 | 0 | 0 | 1 | 1 |
| 8 | `global_addr_pressure` | `main` | 46 | 38 | gcc16.2 | 8 | 0 | 0 | 8 | 0 |
| 8 | `libm_round_family` | `copysign` | 12 | 4 | clang | 0 | 0 | 0 | 1 | 0 |
| 8 | `mandelbrot` | `main` | 55 | 47 | gcc16.2 | 6 | 0 | 0 | 9 | 1 |
| 7 | `binary_trees` | `main` | 82 | 75 | icx | 6 | 1 | 3 | 21 | 0 |
| 7 | `gzip_crc32` | `main` | 57 | 50 | gcc16.2 | 8 | 0 | 0 | 10 | 0 |
| 7 | `tls_seg_access` | `main` | 24 | 17 | gcc16.2 | 1 | 0 | 0 | 5 | 0 |
| 6 | `hash_table` | `insert` | 47 | 41 | gcc16.2 | 5 | 6 | 0 | 6 | 0 |
| 6 | `histogram` | `main` | 82 | 76 | gcc16.2 | 20 | 4 | 1 | 9 | 0 |
| 6 | `sqlite_varint` | `sqlite_get_varint` | 121 | 115 | clang | 10 | 9 | 0 | 17 | 0 |
| 4 | `binary_trees` | `destroy` | 15 | 11 | clang | 2 | 0 | 0 | 5 | 0 |
| 4 | `i686_alu_chains` | `k_udr10` | 21 | 17 | icc | 3 | 0 | 0 | 3 | 0 |
| 4 | `i686_alu_chains` | `k_srem16` | 20 | 16 | gcc16.2 | 3 | 0 | 0 | 3 | 0 |
| 4 | `libm_round_family` | `fma` | 6 | 2 | gcc16.2 | 0 | 0 | 0 | 1 | 0 |
| 4 | `qsort` | `main` | 27 | 23 | gcc16.2 | 7 | 0 | 0 | 5 | 0 |
| 3 | `bitops` | `main` | 93 | 90 | clang | 4 | 0 | 0 | 6 | 0 |
| 3 | `i686_alu_chains` | `k_urem10` | 20 | 17 | icc | 2 | 0 | 0 | 3 | 0 |
| 3 | `i686_alu_chains` | `k_mul45` | 13 | 10 | gcc16.2 | 2 | 0 | 0 | 3 | 0 |
| 2 | `hash_table` | `lookup` | 27 | 25 | gcc16.2 | 4 | 1 | 0 | 5 | 0 |
| 2 | `i686_alu_chains` | `k_srem7` | 22 | 20 | icc | 1 | 0 | 0 | 3 | 0 |
| 2 | `i686_alu_chains` | `k_mul24` | 13 | 11 | gcc16.2 | 1 | 0 | 0 | 3 | 0 |
| 2 | `i686_alu_chains` | `k_mulm3` | 13 | 11 | gcc16.2 | 1 | 0 | 0 | 3 | 0 |
| 1 | `arith_loop` | `main` | 12 | 11 | gcc16.2 | 2 | 1 | 2 | 3 | 0 |
| 1 | `binary_trees` | `make` | 26 | 25 | icx | 0 | 4 | 0 | 6 | 0 |
| 1 | `fib` | `main` | 12 | 11 | clang | 2 | 1 | 2 | 3 | 0 |
| 1 | `i686_alu_chains` | `k_urem7` | 20 | 19 | icc | 1 | 0 | 0 | 3 | 0 |
| 1 | `i686_alu_chains` | `k_udiv7` | 19 | 18 | icc | 2 | 0 | 0 | 3 | 0 |
| 1 | `i686_alu_chains` | `k_sdr7` | 24 | 23 | gcc16.2 | 2 | 0 | 0 | 3 | 0 |
| 1 | `i686_alu_chains` | `k_mul17` | 13 | 12 | gcc16.2 | 0 | 0 | 0 | 3 | 0 |
| 1 | `i686_alu_chains` | `k_mul1000` | 11 | 10 | gcc16.2 | 0 | 0 | 0 | 3 | 0 |
| 1 | `sieve` | `main` | 12 | 11 | gcc16.2 | 2 | 1 | 2 | 3 | 0 |
| 1 | `tce_sum` | `main` | 11 | 10 | gcc16.2 | 2 | 1 | 2 | 2 | 0 |
