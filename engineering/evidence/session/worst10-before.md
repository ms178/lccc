| # | Benchmark | LCCC (ms) | GCC (ms) | LCCC/GCC | Status |
|--:|---|--:|--:|--:|---|
| 1 | `tls_seg_access` | 24.2 | 10.9 | 2.209× | ok |
| 2 | `spectral_norm` | 306.0 | 201.5 | 1.515× | ok |
| 3 | `arith_loop` | 139.8 | 99.3 | 1.409× | ok |
| 4 | `glibc_memcmp` | 12.1 | 8.8 | 1.379× | ok |
| 5 | `expat_xml_scan` | 50.9 | 40.1 | 1.269× | ok |
| 6 | `loop_patterns` | 88.5 | 71.5 | 1.237× | ok |
| 7 | `fannkuch` | 3043.7 | 2473.4 | 1.234× | ok |
| 8 | `aarch64_select_patterns` | 166.1 | 128.6 | 1.216× | ok |
| 9 | `sqlite_varint` | 32.4 | 26.2 | 1.204× | ok |
| 10 | `sieve` | 62.1 | 55.2 | 1.124× | ok |

Geomean LCCC/GCC (all 33 kernels): before 0.692
