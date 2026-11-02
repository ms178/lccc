# Codegen oracle report

Static code-size/structure statistics from local LCCC and Compiler Explorer.
These are screening metrics, not PMU evidence; verify wins with controlled
runtime and hardware counters on the intended target before making claims.

| Source | Function | LCCC | Best | Best compiler | LCCC/best | Loads | Stores | Stack refs | Branches |
|---|---:|---:|---:|---|---:|---:|---:|---:|---:|
| `tests/benchmark/programs/gzip_crc32.c` | `` | 73 | 50 | gcc16.2 | 1.46x | 3 | 2 | 0 | 13 |
| `tests/benchmark/programs/zlib_ng_adler32.c` | `` | 276 | 147 | icx | 1.88x | 31 | 9 | 15 | 21 |
| `tests/benchmark/programs/expat_siphash24.c` | `` | 846 | 308 | gcc16.2 | 2.75x | 72 | 75 | 54 | 67 |
