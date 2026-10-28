# Benchmark Screen: A: lccc-pristine vs B: lccc
- **Flags**: `-O2`
- **Rounds**: `11`
- **Aggregate B/A Geomean**: `1.0005`

| Benchmark | A min (ms) | B min (ms) | B/A Ratio | B/A low3 | Note |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `aarch64_select_patterns` | 114.73 | 116.21 | 1.013 | 1.025 |  |
| `ackermann` | 2.49 | 2.47 | 0.991 | 0.984 | excluded |
| `adler32_do8_hot` | 2.47 | 2.50 | 1.013 | 1.022 | excluded |
| `affine_countdown` | 4.03 | 4.04 | 1.002 | 0.995 |  |
| `arith_loop` | 116.30 | 116.89 | 1.005 | 1.001 |  |
| `ascii_case_fold` | 2.63 | 2.60 | 0.987 | 0.993 | excluded |
| `base64_enc` | 2.40 | 2.46 | 1.023 | 1.014 | excluded |
| `binary_search` | 2.49 | 2.47 | 0.992 | 0.990 | excluded |
| `binary_trees` | 1371.75 | 1387.78 | 1.012 | 0.999 |  |
| `bitops` | 237.50 | 236.92 | 0.998 | 0.996 |  |
| `byte_load_reassembly` | 4.83 | 4.82 | 0.998 | 1.000 |  |
| `chacha20_block` | 301.47 | 301.88 | 1.001 | 0.998 |  |
| `constant_recursion` | 2.42 | 2.44 | 1.009 | 1.011 | excluded |
| `conv_u8_3x3` | 2.53 | 2.48 | 0.981 | 0.999 | excluded |
| `count_observations` | 50.22 | 50.27 | 1.001 | 1.001 |  |
| `crc32_nibble_hot` | 2.47 | 2.45 | 0.991 | 0.986 | excluded |
| `csv_field_sum` | 2.46 | 2.50 | 1.017 | 1.015 | excluded |
| `double_reduction` | 120.94 | 121.02 | 1.001 | 1.012 |  |
| `expat_siphash24` | 260.61 | 260.99 | 1.001 | 0.998 |  |
| `expat_xml_scan` | 50.95 | 51.11 | 1.003 | 1.002 |  |
| `f32_sse1_map` | 265.74 | 265.83 | 1.000 | 1.003 |  |
| `fannkuch` | 3078.28 | 3088.38 | 1.003 | 1.000 |  |
| `fib` | 2.45 | 2.47 | 1.008 | 1.010 | excluded |
| `fir_filter` | 2.72 | 2.71 | 0.998 | 0.998 | excluded |
| `fp_memfold_stencil5` | 21.76 | 21.31 | 0.979 | 0.986 |  |
| `fp_product_map` | 266.72 | 262.97 | 0.986 | 0.987 |  |
| `glibc_memcmp` | 8.72 | 8.69 | 0.997 | 0.998 |  |
| `glibc_strstr` | 5290.14 | 5293.35 | 1.001 | 1.000 |  |
| `global_addr_pressure` | 2.85 | 2.80 | 0.982 | 0.977 | excluded |
| `gzip_crc32` | 171.17 | 171.24 | 1.000 | 0.999 |  |
| `gzip_longest_match` | 181.90 | 183.93 | 1.011 | 1.005 |  |
| `hash_table` | 11989.78 | 12025.85 | 1.003 | 1.009 |  |
| `histogram` | 2.71 | 2.69 | 0.993 | 0.994 | excluded |
| `hot_web_loop` | 72.02 | 71.53 | 0.993 | 1.000 |  |
| `i32_product_map` | 484.51 | 485.88 | 1.003 | 1.002 |  |
| `i686_alu_chains` | 840.94 | 842.46 | 1.002 | 1.001 |  |
| `libm_round_family` | 612.18 | 610.73 | 0.998 | 0.998 |  |
| `linux_find_bit` | 12.30 | 12.37 | 1.006 | 1.002 |  |
| `linux_list_sort` | 510.37 | 510.15 | 1.000 | 1.000 |  |
| `linux_rbtree` | 18.80 | 18.72 | 0.996 | 0.998 |  |
| `llvm_scalar_facts` | 66.31 | 66.31 | 1.000 | 1.005 |  |
| `llvm_word_counts` | 46.19 | 46.48 | 1.006 | 1.001 |  |
| `loop_patterns` | 79.50 | 79.89 | 1.005 | 1.002 |  |
| `lz4_compress` | 4.00 | 4.02 | 1.007 | 1.013 |  |
| `mandelbrot` | 1236.79 | 1234.71 | 0.998 | 0.999 |  |
| `matmul` | 7.99 | 8.00 | 1.001 | 0.999 |  |
| `moving_stats` | 2.46 | 2.45 | 0.997 | 1.003 | excluded |
| `nbody` | 466.12 | 466.42 | 1.001 | 1.001 |  |
| `percpu_ptr_walk` | 8.44 | 8.49 | 1.006 | 1.005 |  |
| `popcount_loop_density` | 835.64 | 837.61 | 1.002 | 1.003 |  |
| `prefix_scan` | 2.50 | 2.47 | 0.988 | 0.988 | excluded |
| `qsort` | 146.44 | 146.33 | 0.999 | 1.000 |  |
| `ra01_global_match` | 2.65 | 2.65 | 1.002 | 1.003 | excluded |
| `reduction_vecreg` | 467.17 | 468.63 | 1.003 | 1.004 |  |
| `reg_off_window` | 4.24 | 4.20 | 0.990 | 0.995 |  |
| `ring_fifo` | 2.55 | 2.53 | 0.993 | 0.995 | excluded |
| `sha256_transform` | 275.78 | 278.32 | 1.009 | 1.006 |  |
| `sieve` | 40.12 | 39.75 | 0.991 | 0.992 |  |
| `spectral_norm` | 182.77 | 182.12 | 0.996 | 0.997 |  |
| `sqlite_varint` | 29.82 | 29.55 | 0.991 | 0.995 |  |
| `strlen_bench` | 249.90 | 251.17 | 1.005 | 1.005 |  |
| `struct_copy` | 21.77 | 21.78 | 1.001 | 1.002 |  |
| `switch_dispatch` | 618.56 | 618.02 | 0.999 | 1.000 |  |
| `switch_lut_bool_bench` | 17.17 | 17.18 | 1.000 | 1.002 |  |
| `switch_lut_packed_bench` | 16.40 | 16.37 | 0.998 | 0.998 |  |
| `switch_lut_packed_fallback_bench` | 16.46 | 16.48 | 1.001 | 1.002 |  |
| `switch_lut_two_val_bench` | 16.75 | 16.74 | 1.000 | 1.000 |  |
| `tce_sum` | 2.40 | 2.42 | 1.012 | 1.012 | excluded |
| `tls_access` | 2.36 | 2.37 | 1.005 | 1.008 | excluded |
| `tls_hot_loop` | 2.82 | 2.81 | 0.999 | 1.008 | excluded |
| `tls_seg_access` | 11.56 | 11.62 | 1.005 | 1.007 |  |
| `vecreg_new_ops` | 2.44 | 2.47 | 1.013 | 0.997 | excluded |
| `vector_remainder` | 2.45 | 2.43 | 0.995 | 0.996 | excluded |
| `volatile_reduce` | 5.26 | 5.28 | 1.003 | 1.000 |  |
| `zlib_ng_adler32` | 43.66 | 43.77 | 1.003 | 1.002 |  |
| `zlib_ng_adler32_combine` | 593.09 | 592.47 | 0.999 | 0.999 |  |
| `zstd_count` | 11.29 | 11.25 | 0.996 | 0.997 |  |
| `zstd_double_fast` | 507.96 | 509.41 | 1.003 | 1.002 |  |
