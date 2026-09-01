# Codegen oracle report

Static code-size/structure statistics from local LCCC and Compiler Explorer.
These are screening metrics, not PMU evidence; verify wins with controlled
runtime and hardware counters on the intended target before making claims.

| Source | Function | LCCC | Best | Best compiler | LCCC/best | Loads | Stores | Spills | Branches |
|---|---:|---:|---:|---|---:|---:|---:|---:|---:|
| `engineering/kernels/isel_roi.c` | `popcount32` | 3 | 2 | clang23.1 | 1.50x | 0 | 0 | 0 | 1 |
| `engineering/kernels/isel_roi.c` | `hash_mul` | 3 | 2 | gcc16.2 | 1.50x | 0 | 0 | 0 | 1 |
| `engineering/kernels/isel_roi.c` | `cmp0` | 6 | 4 | gcc16.2 | 1.50x | 0 | 0 | 0 | 1 |
| `engineering/kernels/isel_roi.c` | `andn32` | 2 | 2 | lccc | 1.00x | 0 | 0 | 0 | 1 |
| `engineering/kernels/isel_roi.c` | `mul3` | 3 | 2 | gcc16.2 | 1.50x | 1 | 0 | 0 | 1 |
| `engineering/kernels/isel_roi.c` | `clz32` | 3 | 2 | clang23.1 | 1.50x | 0 | 0 | 0 | 1 |
| `engineering/kernels/isel_roi.c` | `min_u32` | 4 | 4 | lccc | 1.00x | 0 | 0 | 0 | 1 |
| `engineering/kernels/isel_roi.c` | `rotl32` | 16 | 4 | gcc16.2 | 4.00x | 0 | 0 | 0 | 1 |
