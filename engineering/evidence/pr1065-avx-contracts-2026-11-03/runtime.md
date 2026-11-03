# LCCC benchmark report

- **UTC:** `2026-11-03T15:54:54.442521+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `a1151691d0de5f1f9cd75c52acfaf7d73e2a6a5e
`
- **LCCC binary SHA-256:** `a51618781358e13f45f48abed83ee0dbd76c0ced0637a256666ad40d14a3a5f3`
- **Optimization:** `-O2`; common flags: `['-march=x86-64-v3']`
- **Rounds / excluded warm-ups:** `15 / 2`
- **Method:** randomized balanced two-round blocks for two compilers; independent shuffle otherwise; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | CLANG median | LCCC CANDIDATE: BEFORE median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | ---: | --- | ---: | :---: |
| `avx_fp_private` | 265.18 ms | 27.66 ms | 11.03 ms | 327.83 ms | Clang | 22.8532 [21.9536, 24.5587] | pass |
| `gzip_longest_match` | 162.45 ms | 154.20 ms | 151.46 ms | 165.33 ms | Clang | 1.1025 [1.0246, 1.1094] | pass |

## Aggregate LCCC/GCC (correct pairs only)

Workload mix and extreme specialized or short tests can dominate; this is not a whole-application score.

- Geometric mean ratio: `3.2504`
- Arithmetic mean ratio: `5.3762`
- Best individual ratio: `gzip_longest_match` = `1.0939`
- Worst individual ratio: `avx_fp_private` = `9.6586`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `5.0195`
- Arithmetic mean ratio: `11.9779`
- Best individual ratio: `gzip_longest_match` vs `clang` = `1.1025`
- Worst individual ratio: `avx_fp_private` vs `clang` = `22.8532`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
