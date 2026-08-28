# LCCC benchmark report

- **UTC:** `2026-08-28T18:43:28.068648+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `1b3994e7e48808b9637b37a1f004d091b792f1fc`
- **LCCC binary SHA-256:** `c86d62b0f9b1143644d53593ded215edb799d6b41d36edb721a0a16bf4fbc68b`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `hash_table` | 21.1793 s | 19.5491 s | GCC | 1.0899 [1.0890, 1.0992] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.0899`
- Arithmetic mean ratio: `1.0899`
- Best individual ratio: `hash_table` = `1.0899`
- Worst individual ratio: `hash_table` = `1.0899`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.0899`
- Arithmetic mean ratio: `1.0899`
- Best individual ratio: `hash_table` vs `gcc` = `1.0899`
- Worst individual ratio: `hash_table` vs `gcc` = `1.0899`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
