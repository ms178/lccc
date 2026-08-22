# LCCC benchmark report

- **UTC:** `2026-08-22T20:15:55.384912+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `78209d17d2a6e1d025a87e252183ac3507b1eea3`
- **LCCC binary SHA-256:** `4e9b66cdacb356157d76113b002a5da71dd0d4d73c7dcbf990784297b15abe4d`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `aarch64_select_patterns` | 192.67 ms | 138.49 ms | GCC | 1.3939 [1.3563, 1.4153] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.3939`
- Arithmetic mean ratio: `1.3939`
- Best individual ratio: `aarch64_select_patterns` = `1.3939`
- Worst individual ratio: `aarch64_select_patterns` = `1.3939`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.3939`
- Arithmetic mean ratio: `1.3939`
- Best individual ratio: `aarch64_select_patterns` vs `gcc` = `1.3939`
- Worst individual ratio: `aarch64_select_patterns` vs `gcc` = `1.3939`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
