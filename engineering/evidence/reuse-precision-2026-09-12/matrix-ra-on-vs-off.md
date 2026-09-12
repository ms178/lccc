# Benchmark Screen: A: w_ON_ON.sh vs B: w_OFF_ON.sh
- **Flags**: `-O2 -DPASSES=8 -DBLOCK_COUNT=131072`
- **Rounds**: `15`
- **Aggregate B/A Geomean**: `1.0606`

| Benchmark | A min (ms) | B min (ms) | B/A Ratio | B/A low3 | Note |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `sha256_transform` | 405.13 | 429.69 | 1.061 | 1.062 |  |
