# Benchmark Screen: A: lccc vs B: lccc_ship
- **Flags**: `-O2`
- **Rounds**: `9`
- **Aggregate B/A Geomean**: `1.0052`

| Benchmark | A min (ms) | B min (ms) | B/A Ratio | B/A low3 | Note |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `chacha20_block` | 272.43 | 271.69 | 0.997 | 1.002 |  |
| `double_reduction` | 88.25 | 89.25 | 1.011 | 0.997 |  |
| `fannkuch` | 2770.47 | 2804.44 | 1.012 | 1.014 |  |
| `glibc_strstr` | 4576.16 | 4566.63 | 0.998 | 1.000 |  |
| `linux_rbtree` | 15.97 | 15.84 | 0.992 | 0.992 |  |
| `nbody` | 321.26 | 327.96 | 1.021 | 1.017 |  |
