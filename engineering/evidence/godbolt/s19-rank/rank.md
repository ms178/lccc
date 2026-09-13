# Codegen rank report — gap to best oracle

Static per-function instruction-gap ranking, worst first, from local LCCC
and the Compiler Explorer oracles. These are screening metrics, not PMU
evidence; verify wins with controlled runtime and hardware counters on the
intended target before making claims.

- flags: `-O2 -march=x86-64-v3`

| Gap | Benchmark | Function | LCCC insns | Best insns | Best compiler | LCCC loads | stores | spills | branches | vectors |
|---:|---|---|---:|---:|---|---:|---:|---:|---:|---:|
| 492 | `linux_rbtree` | `main` | 665 | 173 | gcc16.2 | 139 | 66 | 66 | 181 | 0 |
| 422 | `csv_field_sum` | `main` | 532 | 110 | gcc16.2 | 100 | 47 | 90 | 95 | 0 |
| 215 | `nbody` | `main` | 333 | 118 | gcc16.2 | 125 | 30 | 10 | 20 | 2 |
| 169 | `zlib_ng_adler32` | `main` | 242 | 73 | icc | 40 | 6 | 19 | 26 | 0 |
| 111 | `chacha20_block` | `chacha20_core` | 174 | 63 | icx | 24 | 8 | 16 | 23 | 80 |
| 100 | `moving_stats` | `main` | 231 | 131 | gcc16.2 | 40 | 2 | 4 | 10 | 0 |
| 90 | `i686_alu_chains` | `main` | 199 | 109 | icc | 61 | 60 | 75 | 13 | 6 |
| 86 | `glibc_strstr` | `main` | 210 | 124 | gcc16.2 | 42 | 12 | 33 | 29 | 0 |
| 85 | `loop_patterns` | `main` | 264 | 179 | gcc16.2 | 50 | 13 | 16 | 35 | 66 |
| 83 | `sha256_transform` | `main` | 177 | 94 | gcc16.2 | 48 | 54 | 87 | 17 | 0 |
| 80 | `struct_copy` | `main` | 156 | 76 | clang | 52 | 32 | 77 | 4 | 2 |
| 70 | `strlen_bench` | `main` | 204 | 134 | gcc16.2 | 33 | 9 | 19 | 38 | 0 |
| 66 | `expat_xml_scan` | `main` | 160 | 94 | gcc16.2 | 37 | 7 | 17 | 34 | 0 |
| 65 | `matmul` | `matmul` | 86 | 21 | gcc16.2 | 32 | 3 | 4 | 12 | 12 |
| 58 | `conv_u8_3x3` | `main` | 149 | 91 | gcc16.2 | 38 | 1 | 2 | 18 | 0 |
| 57 | `fannkuch` | `main` | 166 | 109 | icx | 44 | 23 | 12 | 26 | 3 |
| 53 | `vecreg_new_ops` | `main` | 87 | 34 | gcc16.2 | 6 | 48 | 52 | 9 | 0 |
| 52 | `ring_fifo` | `main` | 65 | 13 | gcc16.2 | 7 | 0 | 0 | 10 | 0 |
| 51 | `double_reduction` | `main` | 143 | 92 | gcc16.2 | 26 | 4 | 7 | 13 | 48 |
| 44 | `zstd_count` | `main` | 135 | 91 | gcc16.2 | 19 | 4 | 8 | 22 | 0 |
| 43 | `sha256_transform` | `sha256_transform` | 169 | 126 | clang | 52 | 14 | 27 | 14 | 3 |
| 40 | `fir_filter` | `main` | 123 | 83 | gcc16.2 | 24 | 8 | 0 | 11 | 0 |
| 39 | `sqlite_varint` | `main` | 177 | 138 | clang | 36 | 8 | 25 | 30 | 0 |
| 38 | `libm_round_family` | `trunc` | 39 | 1 | icx | 6 | 0 | 0 | 3 | 0 |
| 35 | `linux_find_bit` | `main` | 119 | 84 | gcc16.2 | 20 | 6 | 10 | 17 | 0 |
| 32 | `glibc_memcmp` | `main` | 108 | 76 | gcc16.2 | 21 | 10 | 20 | 14 | 0 |
| 27 | `binary_search` | `main` | 78 | 51 | gcc16.2 | 10 | 0 | 0 | 18 | 0 |
| 27 | `reduction_vecreg` | `main` | 88 | 61 | icx | 16 | 2 | 0 | 11 | 0 |
| 25 | `chacha20_block` | `main` | 95 | 70 | clang | 16 | 20 | 34 | 14 | 0 |
| 25 | `vecreg_new_ops` | `sat_kernel` | 40 | 15 | gcc16.2 | 6 | 2 | 2 | 3 | 5 |
| 24 | `hash_table` | `main` | 74 | 50 | gcc16.2 | 5 | 0 | 0 | 18 | 0 |
| 23 | `base64_enc` | `main` | 116 | 93 | icc | 23 | 0 | 0 | 16 | 0 |
| 20 | `libm_round_family` | `floor` | 21 | 1 | icx | 5 | 0 | 0 | 1 | 1 |
| 20 | `libm_round_family` | `ceil` | 21 | 1 | icx | 5 | 0 | 0 | 1 | 1 |
| 19 | `tls_seg_access` | `tls_pass` | 38 | 19 | gcc16.2 | 9 | 3 | 0 | 3 | 0 |
| 18 | `switch_dispatch` | `main` | 88 | 70 | gcc16.2 | 13 | 0 | 0 | 22 | 0 |
| 16 | `aarch64_select_patterns` | `main` | 50 | 34 | gcc16.2 | 2 | 0 | 0 | 7 | 0 |
| 15 | `sieve` | `count_primes` | 49 | 34 | gcc16.2 | 7 | 2 | 0 | 11 | 0 |
| 14 | `gzip_crc32` | `main` | 64 | 50 | gcc16.2 | 8 | 0 | 0 | 12 | 0 |
| 14 | `mandelbrot` | `main` | 61 | 47 | gcc16.2 | 6 | 0 | 0 | 9 | 1 |
| 11 | `fp_memfold_stencil5` | `main` | 82 | 71 | icx | 15 | 2 | 7 | 12 | 0 |
| 11 | `global_addr_pressure` | `kernel` | 39 | 28 | gcc16.2 | 11 | 0 | 0 | 6 | 0 |
| 11 | `i686_alu_chains` | `k_digits` | 34 | 23 | icc | 2 | 0 | 0 | 5 | 0 |
| 11 | `libm_round_family` | `rint` | 12 | 1 | icx | 4 | 0 | 0 | 1 | 1 |
| 11 | `libm_round_family` | `nearbyint` | 12 | 1 | icx | 4 | 0 | 0 | 1 | 1 |
| 11 | `libm_round_family` | `roundeven` | 12 | 1 | icx | 4 | 0 | 0 | 1 | 1 |
| 10 | `libm_round_family` | `copysign` | 14 | 4 | clang | 0 | 0 | 0 | 1 | 0 |
| 9 | `aarch64_select_patterns` | `select_pressure` | 15 | 6 | gcc16.2 | 0 | 0 | 0 | 1 | 0 |
| 9 | `hash_table` | `insert` | 50 | 41 | gcc16.2 | 6 | 5 | 0 | 7 | 0 |
| 8 | `global_addr_pressure` | `main` | 46 | 38 | gcc16.2 | 8 | 0 | 0 | 8 | 0 |
| 8 | `hash_table` | `lookup` | 33 | 25 | gcc16.2 | 5 | 0 | 0 | 6 | 0 |
| 7 | `binary_trees` | `main` | 82 | 75 | icx | 6 | 1 | 3 | 21 | 0 |
| 7 | `glibc_memcmp` | `glibc_memcmp_common_alignment` | 88 | 81 | gcc16.2 | 12 | 0 | 0 | 23 | 0 |
| 7 | `tls_seg_access` | `main` | 24 | 17 | gcc16.2 | 1 | 0 | 0 | 5 | 0 |
| 6 | `libm_round_family` | `fma` | 8 | 2 | gcc16.2 | 0 | 0 | 0 | 1 | 0 |
| 6 | `sqlite_varint` | `sqlite_get_varint` | 121 | 115 | clang | 10 | 9 | 0 | 17 | 0 |
| 5 | `binary_trees` | `destroy` | 16 | 11 | clang | 2 | 0 | 0 | 5 | 0 |
| 4 | `i686_alu_chains` | `k_udr10` | 21 | 17 | icc | 3 | 0 | 0 | 3 | 0 |
| 4 | `i686_alu_chains` | `k_srem16` | 20 | 16 | gcc16.2 | 3 | 0 | 0 | 3 | 0 |
| 4 | `libm_round_family` | `round_family_pass` | 31 | 27 | gcc16.2 | 7 | 0 | 0 | 3 | 3 |
| 4 | `qsort` | `main` | 27 | 23 | gcc16.2 | 7 | 0 | 0 | 5 | 0 |
| 3 | `bitops` | `main` | 93 | 90 | clang | 4 | 0 | 0 | 6 | 0 |
| 3 | `i686_alu_chains` | `k_urem10` | 20 | 17 | icc | 2 | 0 | 0 | 3 | 0 |
| 3 | `i686_alu_chains` | `k_mul45` | 13 | 10 | gcc16.2 | 2 | 0 | 0 | 3 | 0 |
| 2 | `arith_loop` | `main` | 13 | 11 | gcc16.2 | 2 | 1 | 2 | 3 | 0 |
| 2 | `fib` | `main` | 13 | 11 | clang | 2 | 1 | 2 | 3 | 0 |
| 2 | `i686_alu_chains` | `k_srem7` | 22 | 20 | icc | 1 | 0 | 0 | 3 | 0 |
| 2 | `i686_alu_chains` | `k_mul24` | 13 | 11 | gcc16.2 | 1 | 0 | 0 | 3 | 0 |
| 2 | `i686_alu_chains` | `k_mulm3` | 13 | 11 | gcc16.2 | 1 | 0 | 0 | 3 | 0 |
| 2 | `sieve` | `main` | 13 | 11 | gcc16.2 | 2 | 1 | 2 | 3 | 0 |
| 1 | `binary_trees` | `make` | 26 | 25 | icx | 0 | 4 | 0 | 6 | 0 |
| 1 | `i686_alu_chains` | `k_urem7` | 20 | 19 | icc | 1 | 0 | 0 | 3 | 0 |
| 1 | `i686_alu_chains` | `k_udiv7` | 19 | 18 | icc | 2 | 0 | 0 | 3 | 0 |
| 1 | `i686_alu_chains` | `k_sdr7` | 24 | 23 | gcc16.2 | 2 | 0 | 0 | 3 | 0 |
| 1 | `i686_alu_chains` | `k_mul17` | 13 | 12 | gcc16.2 | 0 | 0 | 0 | 3 | 0 |
| 1 | `i686_alu_chains` | `k_mul1000` | 11 | 10 | gcc16.2 | 0 | 0 | 0 | 3 | 0 |
| 1 | `tce_sum` | `main` | 11 | 10 | gcc16.2 | 2 | 1 | 2 | 2 | 0 |
| 1 | `vector_remainder` | `main` | 78 | 77 | gcc16.2 | 17 | 3 | 6 | 13 | 0 |
