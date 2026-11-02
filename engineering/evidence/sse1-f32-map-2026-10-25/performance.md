# LCCC benchmark report

- **UTC:** `2026-10-25T00:33:00.876752+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `not probed`
- **LCCC revision:** `bcb344cba291eb56037231ad3a8f42832cc301b7
`
- **LCCC binary SHA-256:** `f69461ca6d6988760a534d955a6c940b4d69cc6d49296fc60e5475862435658b`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | CLANG median | LCCC CANDIDATE: SCALAR median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | ---: | --- | ---: | :---: |
| `f32_sse1_map` | 263.58 ms | 254.06 ms | 260.00 ms | 1.0373 s | GCC | 1.0454 [1.0231, 1.0563] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.0454`
- Arithmetic mean ratio: `1.0454`
- Best individual ratio: `f32_sse1_map` = `1.0454`
- Worst individual ratio: `f32_sse1_map` = `1.0454`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.0454`
- Arithmetic mean ratio: `1.0454`
- Best individual ratio: `f32_sse1_map` vs `gcc` = `1.0454`
- Worst individual ratio: `f32_sse1_map` vs `gcc` = `1.0454`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
