# Benchmark Screen: A: lccc_base vs B: lccc_mine
- **Flags**: `-O2 -DSRC_SIZE=(1UL<<22) -DPASSES=96U`
- **Rounds**: `21`
- **Aggregate B/A Geomean**: `0.9966`

| Benchmark | A min (ms) | B min (ms) | B/A Ratio | B/A low3 | Note |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `lz4_compress` | 183.63 | 183.01 | 0.997 | 0.999 |  |
