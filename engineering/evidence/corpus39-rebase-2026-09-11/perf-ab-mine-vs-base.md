# Benchmark Screen: A: lccc vs B: lccc
- **Flags**: `-O2`
- **Rounds**: `11`
- **Aggregate B/A Geomean**: `1.0019`

| Benchmark | A min (ms) | B min (ms) | B/A Ratio | B/A low3 | Note |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `aarch64_select_patterns` | 138.41 | 137.88 | 0.996 | 0.999 |  |
| `ackermann` | 1.15 | 1.13 | 0.986 | 0.971 | excluded |
| `arith_loop` | 108.08 | 104.78 | 0.969 | 0.967 |  |
| `ascii_case_fold` | 1.34 | 1.32 | 0.980 | 0.997 | excluded |
| `base64_enc` | 1.07 | 1.07 | 1.000 | 0.998 | excluded |
| `binary_search` | 1.13 | 1.12 | 0.992 | 1.000 | excluded |
| `binary_trees` | 1587.87 | 1574.51 | 0.992 | 0.989 |  |
| `bitops` | 282.13 | 277.91 | 0.985 | 0.976 |  |
| `chacha20_block` | 271.50 | 273.74 | 1.008 | 1.009 |  |
| `constant_recursion` | 1.11 | 1.28 | 1.150 | 1.188 | excluded |
| `conv_u8_3x3` | 1.09 | 1.13 | 1.029 | 1.013 | excluded |
| `csv_field_sum` | 1.09 | 1.08 | 0.988 | 0.997 | excluded |
| `double_reduction` | 89.84 | 90.52 | 1.008 | 1.013 |  |
| `expat_xml_scan` | 47.16 | 47.65 | 1.010 | 1.008 |  |
| `fannkuch` | 2808.65 | 2826.85 | 1.006 | 1.005 |  |
| `fib` | 1.08 | 1.06 | 0.983 | 0.986 | excluded |
| `fir_filter` | 1.09 | 1.09 | 0.995 | 1.002 | excluded |
| `fp_memfold_stencil5` | 17.12 | 17.13 | 1.000 | 1.005 |  |
| `glibc_memcmp` | 9.27 | 9.32 | 1.005 | 1.003 |  |
| `glibc_strstr` | 4598.91 | 4636.33 | 1.008 | 1.004 |  |
| `global_addr_pressure` | 1.55 | 1.58 | 1.018 | 1.025 | excluded |
| `gzip_crc32` | 150.91 | 152.33 | 1.009 | 1.002 |  |
| `hash_table` | 11295.03 | 11826.59 | 1.047 | 1.035 |  |
| `histogram` | 1.48 | 1.45 | 0.980 | 0.986 | excluded |
| `i686_alu_chains` | 732.37 | 733.33 | 1.001 | 1.001 |  |
| `libm_round_family` | 239.07 | 240.47 | 1.006 | 1.007 |  |
| `linux_find_bit` | 15.13 | 15.12 | 1.000 | 1.003 |  |
| `linux_rbtree` | 16.17 | 16.43 | 1.016 | 1.025 |  |
| `loop_patterns` | 76.93 | 77.13 | 1.003 | 1.005 |  |
| `lz4_compress` | 8.07 | 7.95 | 0.985 | 0.989 |  |
| `mandelbrot` | 1729.32 | 1726.20 | 0.998 | 0.997 |  |
| `matmul` | 4.90 | 4.84 | 0.988 | 0.988 |  |
| `moving_stats` | 1.10 | 1.10 | 0.996 | 0.994 | excluded |
| `nbody` | 317.26 | 314.24 | 0.990 | 1.023 |  |
| `prefix_scan` | 1.08 | 1.08 | 0.999 | 1.007 | excluded |
| `qsort` | 124.52 | 125.70 | 1.009 | 1.010 |  |
| `reduction_vecreg` | 797.89 | 797.21 | 0.999 | 0.997 |  |
| `ring_fifo` | 1.08 | 1.08 | 0.996 | 0.992 | excluded |
| `sha256_transform` | 420.41 | 440.26 | 1.047 | 1.048 |  |
| `sieve` | 44.46 | 45.13 | 1.015 | 0.994 |  |
| `spectral_norm` | 257.20 | 250.16 | 0.973 | 0.985 |  |
| `sqlite_varint` | 30.42 | 30.09 | 0.989 | 0.960 |  |
| `strlen_bench` | 231.46 | 234.38 | 1.013 | 1.002 |  |
| `struct_copy` | 24.98 | 25.08 | 1.004 | 1.009 |  |
| `switch_dispatch` | 552.50 | 553.30 | 1.001 | 1.002 |  |
| `tce_sum` | 1.06 | 1.08 | 1.018 | 1.054 | excluded |
| `tls_seg_access` | 11.60 | 11.64 | 1.003 | 0.999 |  |
| `vecreg_new_ops` | 1.10 | 1.07 | 0.972 | 1.059 | excluded |
| `vector_remainder` | 2.23 | 2.20 | 0.987 | 0.990 | excluded |
| `zlib_ng_adler32` | 39.05 | 38.92 | 0.997 | 0.999 |  |
| `zstd_count` | 13.65 | 13.49 | 0.988 | 0.993 |  |
