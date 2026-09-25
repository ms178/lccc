# Codegen rank report — gap to best oracle

Static per-function instruction-gap ranking, worst first, from local LCCC
and the Compiler Explorer oracles. These are screening metrics, not PMU
evidence; verify wins with controlled runtime and hardware counters on the
intended target before making claims.

- flags: `-O2 -march=x86-64-v3`

| Gap | Benchmark | Function | LCCC insns | Best insns | Best compiler | LCCC loads | stores | spills | branches | vectors |
|---:|---|---|---:|---:|---|---:|---:|---:|---:|---:|
| 190 | `nbody` | `main` | 308 | 118 | gcc16.2 | 127 | 31 | 31 | 17 | 29 |
| 60 | `struct_copy` | `main` | 136 | 76 | clang | 36 | 31 | 60 | 4 | 1 |
| 54 | `sha256_transform` | `main` | 148 | 94 | gcc16.2 | 35 | 37 | 60 | 16 | 19 |
| 27 | `sha256_transform` | `sha256_transform` | 153 | 126 | clang | 30 | 3 | 5 | 9 | 45 |
| 22 | `linux_find_bit` | `main` | 106 | 84 | gcc16.2 | 19 | 0 | 0 | 14 | 0 |
| 8 | `mandelbrot` | `main` | 55 | 47 | gcc16.2 | 6 | 0 | 0 | 9 | 1 |
