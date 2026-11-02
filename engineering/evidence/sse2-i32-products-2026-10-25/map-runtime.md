# LCCC benchmark report

- **UTC:** `2026-10-24T22:32:55.721865+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `5f354c5e870e20f051626c103317e3e12720c0e0
`
- **LCCC binary SHA-256:** `3871b1c545dd4aeb830b7f0cf20f18a57f656028675b191ddfb613bc3a1040dc`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | CLANG median | LCCC CANDIDATE: BEFORE median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | ---: | --- | ---: | :---: |
| `i32_product_map` | 506.99 ms | 512.66 ms | 429.28 ms | 969.03 ms | Clang | 1.1779 [1.1742, 1.1865] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `0.9979`
- Arithmetic mean ratio: `0.9979`
- Best individual ratio: `i32_product_map` = `0.9979`
- Worst individual ratio: `i32_product_map` = `0.9979`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.1779`
- Arithmetic mean ratio: `1.1779`
- Best individual ratio: `i32_product_map` vs `clang` = `1.1779`
- Worst individual ratio: `i32_product_map` vs `clang` = `1.1779`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
