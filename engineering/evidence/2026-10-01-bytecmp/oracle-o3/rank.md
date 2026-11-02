# Codegen oracle report

Static code-size/structure statistics from local LCCC and Compiler Explorer.
These are screening metrics, not PMU evidence; verify wins with controlled
runtime and hardware counters on the intended target before making claims.

| Source | Function | LCCC | Best | Best compiler | LCCC/best | Loads | Stores | Spills | Branches |
|---|---:|---:|---:|---|---:|---:|---:|---:|---:|
| `engineering/evidence/2026-10-01-bytecmp/bytecmp.c` | `` | 130 | 58 | clang | 2.24x | 24 | 0 | 0 | 34 |
