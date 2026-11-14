# Codegen rank report — gap to best oracle

Static per-function instruction-gap ranking, worst first, from local LCCC
and the Compiler Explorer oracles. These are screening metrics, not PMU
evidence; verify wins with controlled runtime and hardware counters on the
intended target before making claims.
Equal call counts do not prove equivalent work. Stack refs are not spills.

- flags: `-O2 -march=x86-64-v3`

| Gap | Benchmark | Function | LCCC insns | Best insns | Best compiler | LCCC loads | stores | stack refs | branches | vectors | LCCC calls | Best calls | Comparable |
|---:|---|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|:--:|
| 297 | `gzip_crc32_slice8` | `main` | 446 | 149 | gcc | 60 | 30 | 0 | 45 | 0 | 4 | 5 | **no** |
| 188 | `expat_siphash24` | `expat_siphash24` | 189 | 1 | clang | 18 | 18 | 14 | 12 | 0 | 1 | 0 | **no** |
| 174 | `byte_load_reassembly` | `main` | 237 | 63 | icx | 41 | 36 | 18 | 20 | 81 | 1 | 1 | unproven |
| 105 | `expat_utf8_content` | `expat_content_tok` | 273 | 168 | clang | 40 | 18 | 0 | 85 | 0 | 0 | 0 | unproven |
| 97 | `i686_alu_chains` | `main` | 212 | 115 | clang | 18 | 78 | 88 | 12 | 12 | 7 | 12 | **no** |
| 94 | `loop_patterns` | `main` | 273 | 179 | gcc | 35 | 8 | 7 | 42 | 82 | 1 | 1 | unproven |
| 78 | `binary_trees` | `make` | 103 | 25 | icx | 0 | 28 | 0 | 30 | 0 | 15 | 3 | **no** |
| 77 | `expat_siphash24` | `sip24_final` | 158 | 81 | gcc | 14 | 4 | 0 | 11 | 0 | 0 | 0 | unproven |
| 76 | `expat_utf8_content` | `main` | 264 | 188 | clang | 11 | 26 | 11 | 59 | 0 | 2 | 3 | **no** |
| 72 | `gzip_longest_match` | `main` | 387 | 315 | gcc | 62 | 40 | 56 | 65 | 2 | 1 | 1 | unproven |
| 68 | `conv_u8_3x3` | `main` | 159 | 91 | gcc | 34 | 10 | 23 | 14 | 0 | 1 | 1 | unproven |
| 62 | `glibc_strstr` | `main` | 186 | 124 | gcc | 10 | 16 | 12 | 25 | 12 | 1 | 2 | **no** |
| 60 | `ascii_case_fold` | `main` | 167 | 107 | clang | 25 | 14 | 16 | 11 | 67 | 0 | 0 | unproven |
| 60 | `linux_list_sort` | `main` | 189 | 129 | gcc | 22 | 26 | 31 | 32 | 18 | 3 | 3 | unproven |
| 59 | `binary_trees` | `check` | 82 | 23 | clang | 16 | 6 | 10 | 21 | 0 | 1 | 1 | unproven |
| 55 | `libm_round_family` | `round_family_pass` | 82 | 27 | gcc | 16 | 0 | 0 | 3 | 17 | 0 | 0 | unproven |
| 53 | `adler32_do8_hot` | `main` | 64 | 11 | icx | 10 | 6 | 9 | 9 | 32 | 2 | 1 | **no** |
| 52 | `linux_rbtree` | `main` | 225 | 173 | gcc | 31 | 38 | 4 | 47 | 0 | 1 | 1 | unproven |
| 47 | `adler32_do8_hot` | `adler32_len` | 119 | 72 | gcc | 14 | 1 | 2 | 7 | 34 | 0 | 0 | unproven |
| 46 | `binary_trees` | `destroy` | 57 | 11 | clang | 14 | 0 | 0 | 22 | 0 | 14 | 2 | **no** |
| 44 | `expat_xml_scan` | `main` | 138 | 94 | gcc | 6 | 5 | 3 | 36 | 0 | 4 | 4 | unproven |
| 37 | `fir_filter` | `main` | 120 | 83 | gcc | 27 | 23 | 24 | 8 | 0 | 1 | 1 | unproven |
| 33 | `crc32_nibble_hot` | `main` | 66 | 33 | gcc | 11 | 6 | 10 | 10 | 26 | 1 | 1 | unproven |
| 30 | `chacha20_block` | `chacha20_core` | 93 | 63 | icx | 11 | 7 | 11 | 11 | 54 | 0 | 0 | unproven |
| 28 | `fannkuch` | `main` | 137 | 109 | icx | 14 | 25 | 4 | 22 | 4 | 1 | 2 | **no** |
| 23 | `double_reduction` | `main` | 115 | 92 | gcc | 5 | 3 | 0 | 10 | 45 | 1 | 1 | unproven |
| 23 | `fp_product_map` | `main` | 117 | 94 | gcc | 8 | 5 | 2 | 11 | 0 | 2 | 2 | unproven |
| 22 | `i32_product_map` | `main` | 57 | 35 | gcc | 2 | 3 | 0 | 9 | 0 | 2 | 2 | unproven |
| 19 | `avx_fp_private` | `main` | 61 | 42 | gcc | 5 | 5 | 5 | 7 | 11 | 2 | 2 | unproven |
| 18 | `glibc_memcmp` | `main` | 94 | 76 | gcc | 3 | 11 | 9 | 12 | 6 | 4 | 4 | unproven |
| 18 | `histogram` | `main` | 94 | 76 | gcc | 19 | 10 | 8 | 11 | 34 | 0 | 0 | unproven |
| 16 | `linux_xxhash64` | `kernel_xxh64` | 179 | 163 | gcc | 11 | 3 | 7 | 9 | 0 | 0 | 0 | unproven |
| 14 | `libm_round_family` | `trunc` | 15 | 1 | icx | 7 | 0 | 0 | 3 | 2 | 0 | 0 | unproven |
| 13 | `aarch64_select_patterns` | `main` | 47 | 34 | gcc | 0 | 0 | 0 | 7 | 0 | 4 | 4 | unproven |
| 13 | `chacha20_block` | `main` | 83 | 70 | clang | 8 | 18 | 26 | 13 | 0 | 3 | 3 | unproven |
| 12 | `avx_fp_private` | `avx_private_frame` | 40 | 28 | icx | 9 | 10 | 2 | 1 | 36 | 0 | 0 | unproven |
| 12 | `glibc_dl_hash` | `glibc_swar_strchrnul` | 48 | 36 | clang | 2 | 0 | 0 | 6 | 0 | 0 | 0 | unproven |
| 12 | `gzip_crc32_slice8` | `gzip_crc32_slice8` | 68 | 56 | gcc | 11 | 0 | 0 | 5 | 0 | 0 | 0 | unproven |
| 12 | `hash_table` | `main` | 62 | 50 | gcc | 0 | 0 | 0 | 15 | 0 | 5 | 5 | unproven |
| 11 | `f32_sse1_map` | `main` | 105 | 94 | gcc | 5 | 5 | 2 | 11 | 0 | 2 | 2 | unproven |
| 11 | `global_addr_pressure` | `mix` | 33 | 22 | clang | 12 | 4 | 0 | 3 | 0 | 0 | 0 | unproven |
| 11 | `global_addr_pressure` | `kernel` | 39 | 28 | gcc | 0 | 0 | 0 | 6 | 0 | 3 | 3 | unproven |
| 10 | `fp_memfold_stencil5` | `main` | 81 | 71 | icx | 12 | 3 | 6 | 12 | 4 | 3 | 3 | unproven |
| 10 | `libm_round_family` | `floor` | 11 | 1 | icx | 6 | 0 | 0 | 1 | 2 | 0 | 0 | unproven |
| 10 | `libm_round_family` | `ceil` | 11 | 1 | icx | 6 | 0 | 0 | 1 | 2 | 0 | 0 | unproven |
| 9 | `fib` | `main` | 21 | 12 | clang | 1 | 1 | 2 | 4 | 0 | 1 | 2 | **no** |
| 9 | `mandelbrot` | `main` | 56 | 47 | gcc | 4 | 0 | 0 | 9 | 3 | 1 | 1 | unproven |
| 8 | `linux_xxhash64` | `main` | 75 | 67 | gcc | 0 | 1 | 0 | 14 | 0 | 5 | 5 | unproven |
| 7 | `binary_search` | `main` | 58 | 51 | gcc | 2 | 1 | 0 | 13 | 0 | 0 | 0 | unproven |
| 7 | `f32_sse1_map` | `sse1_map` | 29 | 22 | icx | 9 | 3 | 0 | 7 | 9 | 0 | 0 | unproven |
| 7 | `fp_product_map` | `fp_product_map` | 29 | 22 | icx | 9 | 3 | 0 | 7 | 9 | 0 | 0 | unproven |
| 7 | `linux_find_bit` | `main` | 91 | 84 | gcc | 1 | 5 | 0 | 14 | 0 | 4 | 4 | unproven |
| 6 | `expat_siphash24` | `sip24_update` | 68 | 62 | gcc | 10 | 8 | 0 | 6 | 0 | 0 | 0 | unproven |
| 6 | `libm_round_family` | `rint` | 7 | 1 | icx | 5 | 0 | 0 | 1 | 1 | 0 | 0 | unproven |
| 6 | `libm_round_family` | `nearbyint` | 7 | 1 | icx | 5 | 0 | 0 | 1 | 1 | 0 | 0 | unproven |
| 6 | `libm_round_family` | `roundeven` | 7 | 1 | icx | 5 | 0 | 0 | 1 | 1 | 0 | 0 | unproven |
| 4 | `expat_siphash24` | `main` | 146 | 142 | gcc | 7 | 20 | 24 | 26 | 14 | 10 | 8 | **no** |
| 3 | `binary_trees` | `main` | 78 | 75 | icx | 2 | 1 | 3 | 21 | 0 | 12 | 12 | unproven |
| 3 | `csv_field_sum` | `main` | 113 | 110 | gcc | 1 | 7 | 3 | 17 | 0 | 4 | 4 | unproven |
| 3 | `hash_table` | `insert` | 44 | 41 | gcc | 4 | 5 | 0 | 6 | 0 | 1 | 1 | unproven |
| 2 | `gzip_crc32` | `main` | 52 | 50 | gcc | 1 | 2 | 0 | 10 | 0 | 3 | 1 | **no** |
| 2 | `hot_web_loop` | `main` | 11 | 9 | gcc | 0 | 0 | 0 | 3 | 0 | 2 | 2 | unproven |
| 2 | `i686_alu_chains` | `k_srem16` | 18 | 16 | gcc | 0 | 0 | 0 | 3 | 0 | 0 | 0 | unproven |
| 2 | `i686_alu_chains` | `k_digits` | 29 | 27 | gcc | 0 | 0 | 0 | 5 | 0 | 0 | 0 | unproven |
| 1 | `arith_loop` | `main` | 12 | 11 | gcc | 1 | 1 | 2 | 3 | 0 | 2 | 2 | unproven |
| 1 | `hash_table` | `lookup` | 26 | 25 | gcc | 4 | 0 | 0 | 5 | 0 | 0 | 0 | unproven |
| 1 | `i686_alu_chains` | `k_sdr16` | 23 | 22 | gcc | 0 | 0 | 0 | 3 | 0 | 0 | 0 | unproven |
| 1 | `i686_alu_chains` | `k_mul17` | 13 | 12 | gcc | 0 | 0 | 0 | 3 | 0 | 0 | 0 | unproven |
| 1 | `i686_alu_chains` | `k_mul45` | 11 | 10 | gcc | 0 | 0 | 0 | 3 | 0 | 0 | 0 | unproven |
| 1 | `i686_alu_chains` | `k_mul1000` | 11 | 10 | gcc | 0 | 0 | 0 | 3 | 0 | 0 | 0 | unproven |
| 1 | `linux_list_sort` | `key_cmp` | 11 | 10 | gcc | 4 | 0 | 0 | 3 | 0 | 0 | 0 | unproven |
| 1 | `lz4_compress` | `main` | 293 | 292 | gcc | 36 | 25 | 28 | 54 | 4 | 4 | 4 | unproven |

## 12 rows are not comparable (940 of 2389 of the deficit)

A per-function instruction gap is only a codegen gap if both
compilers put the same work in that function. When the call counts
differ, work may have moved out of line and this
single-function view never measured where it went. On this corpus
that is not a corner case: it is the top of the table.

| Gap | Benchmark | Function | LCCC calls | Best calls |
|---:|---|---|---:|---:|
| 297 | `gzip_crc32_slice8` | `main` | 4 | 5 |
| 188 | `expat_siphash24` | `expat_siphash24` | 1 | 0 |
| 97 | `i686_alu_chains` | `main` | 7 | 12 |
| 78 | `binary_trees` | `make` | 15 | 3 |
| 76 | `expat_utf8_content` | `main` | 2 | 3 |
| 62 | `glibc_strstr` | `main` | 1 | 2 |
| 53 | `adler32_do8_hot` | `main` | 2 | 1 |
| 46 | `binary_trees` | `destroy` | 14 | 2 |
| 28 | `fannkuch` | `main` | 1 | 2 |
| 9 | `fib` | `main` | 1 | 2 |
| 4 | `expat_siphash24` | `main` | 10 | 8 |
| 2 | `gzip_crc32` | `main` | 3 | 1 |

Re-measure these with `--all-functions` before acting on them.
Worked example, method, and the corpus-wide consequences:
`engineering/evidence/ORACLE-METRIC-1/README.md`.
