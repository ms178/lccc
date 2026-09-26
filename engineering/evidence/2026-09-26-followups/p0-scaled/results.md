# LCCC benchmark report

- **UTC:** `2026-09-26T00:43:50.153735+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `e5bc19118c6ea5a54d5539d4a6cff52c97ab6baa`
- **LCCC binary SHA-256:** `d071563d5326f5e1d0a8e3f5aa035e0079843868da16abe9cd2ee16484e3f818`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | CCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | --- | ---: | :---: |
| `linux_find_bit_scaled` | 880.65 ms | 876.28 ms | 639.21 ms | GCC | 1.3869 [1.2846, 1.4157] | pass |
| `lz4_match_extend` | 1.3786 s | 1.3788 s | 1.0925 s | GCC | 1.2537 [1.2359, 1.3164] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.3186`
- Arithmetic mean ratio: `1.3203`
- Best individual ratio: `lz4_match_extend` = `1.2537`
- Worst individual ratio: `linux_find_bit_scaled` = `1.3869`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.3186`
- Arithmetic mean ratio: `1.3203`
- Best individual ratio: `lz4_match_extend` vs `gcc` = `1.2537`
- Worst individual ratio: `linux_find_bit_scaled` vs `gcc` = `1.3869`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
