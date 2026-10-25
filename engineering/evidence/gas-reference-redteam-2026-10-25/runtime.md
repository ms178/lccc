# LCCC benchmark report

- **UTC:** `2026-10-25T11:58:11.081346+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `126b0f75a63530aea544e9e3e42b2b4327f7848c
`
- **LCCC binary SHA-256:** `25562dd38e1024ba6d9d385de83d5652ce665ddd965b5247668a04663a482fd8`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | CLANG median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | --- | ---: | :---: |
| `nbody` | 371.27 ms | 254.16 ms | 249.03 ms | Clang | 1.4948 [1.4835, 1.5229] | pass |
| `lz4_match_extend` | 975.66 ms | 978.32 ms | 986.71 ms | GCC | 1.0007 [0.9647, 1.0164] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.1987`
- Arithmetic mean ratio: `1.2184`
- Best individual ratio: `lz4_match_extend` = `1.0007`
- Worst individual ratio: `nbody` = `1.4361`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.2230`
- Arithmetic mean ratio: `1.2477`
- Best individual ratio: `lz4_match_extend` vs `gcc` = `1.0007`
- Worst individual ratio: `nbody` vs `clang` = `1.4948`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
