# Benchmark Screen: A: lccc_base vs B: lccc_mine
- **Flags**: `-O2 -DPASSES=8 -DBLOCK_COUNT=131072`
- **Rounds**: `11`
- **Aggregate B/A Geomean**: `0.9984`

| Benchmark | A min (ms) | B min (ms) | B/A Ratio | B/A low3 | Note |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `aarch64_select_patterns` | 137.59 | 137.38 | 0.998 | 0.997 |  |
| `ackermann` | 1.05 | 1.07 | 1.017 | 1.012 | excluded |
| `arith_loop` | 100.75 | 102.56 | 1.018 | 1.012 |  |
| `ascii_case_fold` | 1.24 | 1.26 | 1.013 | 1.002 | excluded |
| `base64_enc` | 1.05 | 1.04 | 0.998 | 1.007 | excluded |
| `binary_search` | 1.08 | 1.10 | 1.011 | 1.010 | excluded |
| `binary_trees` | 1348.70 | 1354.80 | 1.005 | 1.002 |  |
| `bitops` | 276.90 | 278.13 | 1.004 | 1.005 |  |
| `constant_recursion` | 1.05 | 1.05 | 1.006 | 1.014 | excluded |
| `conv_u8_3x3` | 1.07 | 1.07 | 1.001 | 1.008 | excluded |
| `csv_field_sum` | 1.07 | 1.08 | 1.000 | 0.993 | excluded |
| `fannkuch` | 2803.47 | 2795.31 | 0.997 | 0.996 |  |
| `fib` | 1.05 | 1.05 | 0.999 | 1.003 | excluded |
| `fir_filter` | 1.06 | 1.04 | 0.986 | 0.991 | excluded |
| `fp_memfold_stencil5` | 17.17 | 17.00 | 0.990 | 0.992 |  |
| `global_addr_pressure` | 1.56 | 1.58 | 1.014 | 1.002 | excluded |
| `hash_table` | 11142.79 | 10861.26 | 0.975 | 0.979 |  |
| `histogram` | 1.47 | 1.54 | 1.048 | 1.077 | excluded |
| `i686_alu_chains` | 730.67 | 730.82 | 1.000 | 1.000 |  |
| `linux_rbtree` | 16.32 | 16.07 | 0.985 | 0.985 |  |
| `loop_patterns` | 74.88 | 75.70 | 1.011 | 0.996 |  |
| `mandelbrot` | 1719.46 | 1718.57 | 0.999 | 1.000 |  |
| `matmul` | 4.61 | 4.63 | 1.004 | 1.009 |  |
| `moving_stats` | 1.08 | 1.06 | 0.978 | 0.990 | excluded |
| `nbody` | 314.36 | 315.56 | 1.004 | 1.008 |  |
| `prefix_scan` | 1.04 | 1.05 | 1.005 | 1.019 | excluded |
| `qsort` | 124.16 | 124.27 | 1.001 | 1.001 |  |
| `reduction_vecreg` | 793.96 | 795.25 | 1.002 | 1.001 |  |
| `ring_fifo` | 1.08 | 1.07 | 0.997 | 0.991 | excluded |
| `sieve` | 37.65 | 37.02 | 0.983 | 0.990 |  |
| `spectral_norm` | 248.03 | 248.10 | 1.000 | 1.001 |  |
| `strlen_bench` | 227.28 | 223.75 | 0.984 | 0.980 |  |
| `struct_copy` | 25.78 | 26.03 | 1.010 | 1.004 |  |
| `switch_dispatch` | 553.12 | 552.39 | 0.999 | 0.998 |  |
| `tce_sum` | 1.02 | 1.03 | 1.009 | 1.003 | excluded |
| `vecreg_new_ops` | 1.03 | 1.04 | 1.009 | 1.003 | excluded |
| `vector_remainder` | 1.06 | 1.04 | 0.982 | 0.979 | excluded |
