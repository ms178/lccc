# Whole-TU static instruction census — opt-in FP candidate

After means the FP allocation candidate, subsequently quarantined.
Not final default output, cycles, runtime or a ranking of execution speed. Same sources/flags;
ICX is the resolved moving channel, not a verified fixed 2026 version.

| Workload | Before | After | GCC 16.2 | Clang 23.1 | ICC 2021.10 | ICX latest |
|---|---:|---:|---:|---:|---:|---:|
| aarch64_select_patterns | 72 | 72 | 48 | 58 | 74 | 63 |
| ackermann | 8 | 8 | 133 | 31 | 195 | 34 |
| adler32_do8_hot | 159 | 159 | 99 | 149 | 328 | 120 |
| affine_countdown | 101 | 101 | 186 | 230 | 106 | 265 |
| arith_loop | 198 | 198 | 223 | 236 | 486 | 251 |
| ascii_case_fold | 111 | 111 | 159 | 107 | 172 | 121 |
| base64_enc | 113 | 113 | 108 | 914 | 93 | 120 |
| binary_search | 71 | 71 | 51 | 87 | 57 | 101 |
| binary_trees | 145 | 145 | 1478 | 147 | 606 | 134 |
| bitops | 80 | 80 | 99 | 90 | 125 | 112 |
| chacha20_block | 193 | 193 | 269 | 257 | 2340 | 138 |
| constant_recursion | 10 | 10 | 135 | 32 | 196 | 35 |
| conv_u8_3x3 | 137 | 137 | 91 | 314 | 318 | 280 |
| crc32_nibble_hot | 41 | 41 | 48 | 95 | 84 | 134 |
| csv_field_sum | 214 | 214 | 183 | 300 | 262 | 297 |
| double_reduction | 135 | 135 | 92 | 133 | 167 | 150 |
| expat_xml_scan | 222 | 222 | 172 | 222 | 319 | 327 |
| fannkuch | 152 | 152 | 135 | 226 | 207 | 109 |
| fib | 30 | 30 | 217 | 31 | 152 | 34 |
| fir_filter | 106 | 106 | 83 | 117 | 191 | 159 |
| fp_memfold_stencil5 | 119 | 119 | 121 | 154 | 283 | 116 |
| glibc_memcmp | 203 | 203 | 157 | 227 | 322 | 311 |
| glibc_strstr | 194 | 194 | 182 | 319 | 197 | 261 |
| global_addr_pressure | 100 | 100 | 80 | 109 | 207 | 131 |
| gzip_crc32 | 73 | 73 | 50 | 82 | 60 | 100 |
| hash_table | 141 | 141 | 116 | 140 | 158 | 142 |
| histogram | 59 | 59 | 76 | 107 | 125 | 124 |
| i686_alu_chains | 492 | 492 | 403 | 1528 | 374 | 2784 |
| libm_round_family | 167 | 167 | 173 | 183 | 206 | 125 |
| linux_find_bit | 147 | 147 | 156 | 118 | 205 | 121 |
| linux_rbtree | 237 | 237 | 173 | 189 | 189 | 274 |
| loop_patterns | 231 | 231 | 179 | 361 | 283 | 245 |
| lz4_compress | 351 | 351 | 292 | 421 | 361 | 361 |
| mandelbrot | 55 | 55 | 47 | 59 | 64 | 64 |
| matmul | 85 | 85 | 82 | 164 | 149 | 131 |
| moving_stats | 217 | 217 | 131 | 291 | 143 | 250 |
| nbody | 302 | 296 | 215 | 266 | 1192 | 531 |
| prefix_scan | 72 | 72 | 55 | 96 | 67 | 117 |
| qsort | 31 | 31 | 26 | 43 | 41 | 52 |
| reduction_vecreg | 99 | 99 | 107 | 105 | 282 | 125 |
| ring_fifo | 56 | 56 | 13 | 57 | 68 | 90 |
| sha256_transform | 279 | 279 | 231 | 232 | 438 | 862 |
| sieve | 51 | 51 | 45 | 337 | 215 | 376 |
| spectral_norm | 284 | 284 | 174 | 365 | 274 | 253 |
| sqlite_varint | 398 | 398 | 325 | 373 | 461 | 421 |
| strlen_bench | 196 | 196 | 134 | 226 | 201 | 481 |
| struct_copy | 136 | 136 | 100 | 76 | 213 | 82 |
| switch_dispatch | 88 | 88 | 70 | 80 | 80 | 87 |
| tce_sum | 11 | 11 | 10 | 10 | 39 | 13 |
| tls_seg_access | 56 | 56 | 36 | 55 | 59 | 64 |
| vecreg_new_ops | 78 | 78 | 49 | 85 | 72 | 67 |
| vector_remainder | 98 | 98 | 139 | 204 | 268 | 281 |
| zlib_ng_adler32 | 276 | 276 | 156 | 148 | 248 | 147 |
| zlib_ng_adler32_combine | 293 | 293 | 316 | 495 | 271 | 646 |
| zstd_count | 166 | 166 | 91 | 121 | 106 | 152 |
