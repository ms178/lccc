# Benchmark Screen: A: lccc_S15_base vs B: lccc_v4
- **Flags**: `-O2 -DPASSES=8 -DBLOCK_COUNT=131072`
- **Rounds**: `11`
- **Aggregate B/A Geomean**: `1.0026`

| Benchmark | A min (ms) | B min (ms) | B/A Ratio | B/A low3 | Note |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `sha256_transform` | 404.95 | 406.01 | 1.003 | 1.002 |  |
