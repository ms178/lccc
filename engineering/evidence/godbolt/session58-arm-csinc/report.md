# Codegen oracle report

Static code-size/structure statistics from local LCCC and Compiler Explorer.
These are screening metrics, not PMU evidence; verify wins with controlled
runtime and hardware counters on the intended target before making claims.

| Source | Function | LCCC | Best | Best compiler | LCCC/best | Loads | Stores | Spills | Branches |
|---|---:|---:|---:|---|---:|---:|---:|---:|---:|
| `tests/regression/arm_csinc_select.c` | `inc_if_sge_i32` | 16 | 3 | carm64g1610 | 5.33x | 4 | 4 | 8 | 1 |
