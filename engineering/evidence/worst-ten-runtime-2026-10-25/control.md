# LCCC benchmark report

- **UTC:** `2026-10-25T14:53:55.451566+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': '0', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `not probed`
- **LCCC revision:** `521befa41d042b1a693bfa28698b38e300e6bb9e
`
- **LCCC binary SHA-256:** `3f636a4e9d78e9744140c6fd3a92de4c7683ad57ec39e34f74cd868667da9aca`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | CLANG median | LCCC CANDIDATE: BEFORE median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | ---: | --- | ---: | :---: |
| `spectral_norm` | 158.75 ms | 159.93 ms | 197.81 ms | 162.61 ms | GCC | 1.0057 [0.9972, 1.0149] | pass |
| `expat_xml_scan` | 44.35 ms | 40.22 ms | 35.94 ms | 44.27 ms | Clang | 1.2493 [1.2314, 1.2770] | pass |
| `glibc_memcmp` | 7.21 ms | 7.20 ms | 7.18 ms | 7.23 ms | Clang | 1.0024 [0.9958, 1.0097] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.0397`
- Arithmetic mean ratio: `1.0410`
- Best individual ratio: `glibc_memcmp` = `1.0004`
- Worst individual ratio: `expat_xml_scan` = `1.1170`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.0799`
- Arithmetic mean ratio: `1.0858`
- Best individual ratio: `glibc_memcmp` vs `clang` = `1.0024`
- Worst individual ratio: `expat_xml_scan` vs `clang` = `1.2493`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
