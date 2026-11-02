# Codegen oracle report

Static code-size/structure statistics from local LCCC and Compiler Explorer.
These are screening metrics, not PMU evidence; verify wins with controlled
runtime and hardware counters on the intended target before making claims.

| Source | Function | LCCC | Best | Best compiler | LCCC/best | Loads | Stores | Stack refs | Branches |
|---|---:|---:|---:|---|---:|---:|---:|---:|---:|
| `tests/regression/fixtures/late_const_gep/main.c` | `main` | 30 | 30 | lccc | 1.00x | 1 | 4 | 3 | 6 |
