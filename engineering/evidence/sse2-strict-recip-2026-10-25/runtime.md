# LCCC benchmark report

- **UTC:** `2026-10-25T12:42:03.776393+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': '0', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `not probed`
- **LCCC revision:** `161ffd0e2aa7fabe2d870cf15d333f92e0b71fb6
`
- **LCCC binary SHA-256:** `310c9aef0fdd13082425884824678aa2e8cacf8faae768dffdc09bd81f49b68d`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | CLANG median | LCCC CANDIDATE: BEFORE median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | ---: | --- | ---: | :---: |
| `fp32_product_map` | 225.70 ms | 224.07 ms | 210.34 ms | 225.63 ms | Clang | 1.0687 [1.0563, 1.0874] | pass |
| `fp64_product_map` | 439.07 ms | 465.79 ms | 421.17 ms | 439.83 ms | Clang | 1.0447 [1.0404, 1.0548] | pass |
| `i32_product_map` | 416.28 ms | 412.26 ms | 412.63 ms | 415.90 ms | GCC | 1.0105 [0.9882, 1.0157] | pass |
| `nbody` | 369.54 ms | 259.14 ms | 246.63 ms | 370.17 ms | Clang | 1.4975 [1.4479, 1.5263] | pass |
| `spectral_norm` | 158.23 ms | 157.92 ms | 196.96 ms | 200.22 ms | GCC | 1.0087 [1.0032, 1.0237] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.0678`
- Arithmetic mean ratio: `1.0795`
- Best individual ratio: `fp64_product_map` = `0.9544`
- Worst individual ratio: `nbody` = `1.4156`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.1125`
- Arithmetic mean ratio: `1.1260`
- Best individual ratio: `spectral_norm` vs `gcc` = `1.0087`
- Worst individual ratio: `nbody` vs `clang` = `1.4975`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
