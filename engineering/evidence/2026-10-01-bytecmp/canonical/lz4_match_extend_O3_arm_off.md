# LCCC benchmark report

- **UTC:** `2026-10-01T01:47:06.736512+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `cdcf30de147db72b5d7266a40147639cf4a23f15`
- **LCCC binary SHA-256:** `8b142f0774c7be12ef11bbc3c50c7a4d7e12f2e1459ca1fb39947ea87a174bfd`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `lz4_match_extend` | 1.3236 s | 1.0490 s | GCC | 1.2547 [1.1935, 1.3519] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.2547`
- Arithmetic mean ratio: `1.2547`
- Best individual ratio: `lz4_match_extend` = `1.2547`
- Worst individual ratio: `lz4_match_extend` = `1.2547`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.2547`
- Arithmetic mean ratio: `1.2547`
- Best individual ratio: `lz4_match_extend` vs `gcc` = `1.2547`
- Worst individual ratio: `lz4_match_extend` vs `gcc` = `1.2547`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
