# LCCC benchmark report

- **UTC:** `2026-09-26T00:38:12.693827+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `e5bc19118c6ea5a54d5539d4a6cff52c97ab6baa`
- **LCCC binary SHA-256:** `d071563d5326f5e1d0a8e3f5aa035e0079843868da16abe9cd2ee16484e3f818`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | CCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | --- | ---: | :---: |
| `spectral_norm` | 240.20 ms | 6.5987 s | 230.08 ms | GCC | 1.0275 [1.0179, 1.0490] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.0275`
- Arithmetic mean ratio: `1.0275`
- Best individual ratio: `spectral_norm` = `1.0275`
- Worst individual ratio: `spectral_norm` = `1.0275`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.0275`
- Arithmetic mean ratio: `1.0275`
- Best individual ratio: `spectral_norm` vs `gcc` = `1.0275`
- Worst individual ratio: `spectral_norm` vs `gcc` = `1.0275`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
