# LCCC benchmark report

- **UTC:** `2026-08-28T18:37:35.651815+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `1b3994e7e48808b9637b37a1f004d091b792f1fc`
- **LCCC binary SHA-256:** `c86d62b0f9b1143644d53593ded215edb799d6b41d36edb721a0a16bf4fbc68b`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `nbody` | 263.99 ms | 214.54 ms | GCC | 1.2333 [1.2272, 1.2388] | pass |
| `binary_trees` | 2.0717 s | 1.9536 s | GCC | 1.0570 [1.0457, 1.0596] | pass |
| `spectral_norm` | 237.28 ms | 181.72 ms | GCC | 1.3053 [1.3027, 1.3126] | pass |
| `mandelbrot` | 1.1027 s | 894.41 ms | GCC | 1.2320 [1.2312, 1.2361] | pass |
| `fannkuch` | 2.8804 s | 2.2596 s | GCC | 1.2744 [1.2736, 1.2773] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.2172`
- Arithmetic mean ratio: `1.2204`
- Best individual ratio: `binary_trees` = `1.0570`
- Worst individual ratio: `spectral_norm` = `1.3053`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.2172`
- Arithmetic mean ratio: `1.2204`
- Best individual ratio: `binary_trees` vs `gcc` = `1.0570`
- Worst individual ratio: `spectral_norm` vs `gcc` = `1.3053`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
