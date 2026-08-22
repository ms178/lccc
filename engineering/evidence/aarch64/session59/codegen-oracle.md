# Codegen oracle report

Static code-size/structure statistics from local LCCC and Compiler Explorer.
These are screening metrics, not PMU evidence; verify wins with controlled
runtime and hardware counters on the intended target before making claims.

| Source | Function | LCCC | Best | Best compiler | LCCC/best | Loads | Stores | Spills | Branches |
|---|---:|---:|---:|---|---:|---:|---:|---:|---:|
| `tests/benchmark/programs/aarch64_select_patterns.c` | `conditional_increment` | 13 | 3 | carm64g1610 | 4.33x | 3 | 3 | 6 | 1 |
| `tests/benchmark/programs/aarch64_select_patterns.c` | `narrow_high_constant` | 14 | 4 | carm64g1610 | 3.50x | 3 | 3 | 6 | 1 |
| `tests/benchmark/programs/aarch64_select_patterns.c` | `select_pressure` | 31 | 6 | carm64g1610 | 5.17x | 6 | 6 | 12 | 1 |
