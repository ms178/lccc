# Codegen rank report — gap to best oracle

Static per-function instruction-gap ranking, worst first, from local LCCC
and the Compiler Explorer oracles. These are screening metrics, not PMU
evidence; verify wins with controlled runtime and hardware counters on the
intended target before making claims.

- flags: `-O2 -march=x86-64-v3 -DMATCH_RICH=1 -DPASSES=2048`

| Gap | Benchmark | Function | LCCC insns | Best insns | Best compiler | LCCC loads | stores | spills | branches | vectors |
|---:|---|---|---:|---:|---|---:|---:|---:|---:|---:|
| 20 | `lz4_compress` | `main` | 326 | 306 | gcc | 88 | 40 | 75 | 48 | 0 |
