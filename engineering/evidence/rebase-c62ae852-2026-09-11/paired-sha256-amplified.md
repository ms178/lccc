# Benchmark Screen: A: lccc_base vs B: lccc_mine
- **Flags**: `-O2 -DPASSES=8 -DBLOCK_COUNT=131072`
- **Rounds**: `11`
- **Aggregate B/A Geomean**: `0.9498`

| Benchmark | A min (ms) | B min (ms) | B/A Ratio | B/A low3 | Note |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `sha256_transform` | 425.45 | 404.09 | 0.950 | 0.949 |  |
