# LCCC benchmark report

- **UTC:** `2026-08-22T20:57:04.058795+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `eff11d5df2e0f8193da924e1f0c71e75d688539c`
- **LCCC binary SHA-256:** `704c057a6f4b6f0616f7e26153719352220dc7dd64f94178c629816c2e498c98`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `aarch64_select_patterns` | 180.27 ms | 131.81 ms | GCC | 1.3756 [1.3566, 1.5459] | pass |
| `gzip_crc32` | 185.81 ms | 170.13 ms | GCC | 1.0848 [1.0556, 1.0968] | pass |
| `zlib_ng_adler32` | 61.82 ms | 41.31 ms | GCC | 1.4719 [1.3346, 1.5451] | pass |
| `expat_xml_scan` | 88.55 ms | 41.63 ms | GCC | 2.1347 [2.0675, 2.3692] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.4715`
- Arithmetic mean ratio: `1.5167`
- Best individual ratio: `gzip_crc32` = `1.0848`
- Worst individual ratio: `expat_xml_scan` = `2.1347`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.4715`
- Arithmetic mean ratio: `1.5167`
- Best individual ratio: `gzip_crc32` vs `gcc` = `1.0848`
- Worst individual ratio: `expat_xml_scan` vs `gcc` = `2.1347`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
