# Codegen rank report — gap to best oracle

Static per-function instruction-gap ranking, worst first, from local LCCC
and the Compiler Explorer oracles. These are screening metrics, not PMU
evidence; verify wins with controlled runtime and hardware counters on the
intended target before making claims.

- flags: `-O2 -march=x86-64-v3`

| Gap | Benchmark | Function | LCCC insns | Best insns | Best compiler | LCCC loads | stores | spills | branches | vectors |
|---:|---|---|---:|---:|---|---:|---:|---:|---:|---:|
| 277 | `zlib_ng_adler32` | `main` | 350 | 73 | icc | 60 | 18 | 32 | 26 | 72 |
| 190 | `nbody` | `main` | 308 | 118 | gcc16.2 | 127 | 31 | 31 | 17 | 29 |
| 63 | `matmul` | `matmul` | 84 | 21 | gcc16.2 | 32 | 3 | 4 | 12 | 12 |
| 60 | `expat_xml_scan` | `main` | 154 | 94 | gcc16.2 | 37 | 7 | 17 | 34 | 0 |
| 60 | `struct_copy` | `main` | 136 | 76 | clang | 36 | 31 | 60 | 4 | 1 |
| 57 | `chacha20_block` | `chacha20_core` | 120 | 63 | icx | 22 | 5 | 6 | 13 | 58 |
| 54 | `sha256_transform` | `main` | 148 | 94 | gcc16.2 | 35 | 37 | 60 | 16 | 19 |
| 43 | `zstd_count` | `main` | 134 | 91 | gcc16.2 | 17 | 4 | 8 | 22 | 0 |
| 27 | `sha256_transform` | `sha256_transform` | 153 | 126 | clang | 30 | 3 | 5 | 9 | 45 |
| 22 | `chacha20_block` | `main` | 92 | 70 | clang | 13 | 18 | 29 | 14 | 0 |
| 22 | `linux_find_bit` | `main` | 106 | 84 | gcc16.2 | 19 | 0 | 0 | 14 | 0 |
| 18 | `hash_table` | `main` | 68 | 50 | gcc16.2 | 4 | 0 | 0 | 15 | 0 |
| 8 | `mandelbrot` | `main` | 55 | 47 | gcc16.2 | 6 | 0 | 0 | 9 | 1 |
| 6 | `hash_table` | `insert` | 47 | 41 | gcc16.2 | 5 | 6 | 0 | 6 | 0 |
| 2 | `hash_table` | `lookup` | 27 | 25 | gcc16.2 | 4 | 1 | 0 | 5 | 0 |
| 1 | `arith_loop` | `main` | 12 | 11 | gcc16.2 | 2 | 1 | 2 | 3 | 0 |
