# Codegen oracle report

Static code-size/structure statistics from local LCCC and Compiler Explorer.
These are screening metrics, not PMU evidence; verify wins with controlled
runtime and hardware counters on the intended target before making claims.

| Source | Function | LCCC | Best | Best compiler | LCCC/best | Loads | Stores | Stack refs | Branches |
|---|---:|---:|---:|---|---:|---:|---:|---:|---:|
| `tests/benchmark/programs/gzip_crc32.c` | `` | 73 | 50 | gcc16.2 | 1.46x | 3 | 2 | 0 | 13 |
| `tests/benchmark/programs/zlib_ng_adler32.c` | `` | 275 | 147 | icx | 1.87x | 30 | 9 | 14 | 21 |
| `tests/benchmark/programs/expat_xml_scan.c` | `` | 221 | 172 | gcc16.2 | 1.28x | 18 | 10 | 17 | 57 |
| `tests/benchmark/programs/sqlite_varint.c` | `` | 395 | 325 | gcc16.2 | 1.22x | 32 | 32 | 22 | 59 |
| `tests/benchmark/programs/histogram.c` | `` | 64 | 64 | lccc | 1.00x | 9 | 5 | 0 | 9 |
