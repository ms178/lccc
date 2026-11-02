# Codegen oracle report

Static code-size/structure statistics from local LCCC and Compiler Explorer.
These are screening metrics, not PMU evidence; verify wins with controlled
runtime and hardware counters on the intended target before making claims.

| Source | Function | LCCC | Best | Best compiler | LCCC/best | Loads | Stores | Stack refs | Branches |
|---|---:|---:|---:|---|---:|---:|---:|---:|---:|
| `tests/benchmark/programs/gzip_longest_match.c` | `` | 410 | 293 | icc | 1.40x | 66 | 45 | 68 | 69 |
| `tests/benchmark/programs/zlib_ng_compare256.c` | `` | 235 | 235 | lccc | 1.00x | 11 | 6 | 6 | 52 |
| `tests/benchmark/programs/expat_xml_scan.c` | `` | 206 | 175 | gcc | 1.18x | 13 | 7 | 9 | 58 |
