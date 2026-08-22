# LCCC benchmark report

- **UTC:** `2026-08-22T21:36:28.001591+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `fdeea471e3f256f7991eea3d18b033d8663fc940`
- **LCCC binary SHA-256:** `70b39c4cec3016072a499d4b1918e4ccdc6d00ab3e073c8c3d2ffa08e5492ae0`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `aarch64_select_patterns` | 175.90 ms | 128.52 ms | GCC | 1.3646 [1.3584, 1.3770] | pass |
| `gzip_crc32` | 179.27 ms | 168.41 ms | GCC | 1.0641 [1.0365, 1.0704] | pass |
| `zlib_ng_adler32` | 59.41 ms | 38.99 ms | GCC | 1.5246 [1.5201, 1.5567] | pass |
| `expat_xml_scan` | 84.71 ms | 42.50 ms | GCC | 2.0562 [1.8611, 2.1116] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.4607`
- Arithmetic mean ratio: `1.5024`
- Best individual ratio: `gzip_crc32` = `1.0641`
- Worst individual ratio: `expat_xml_scan` = `2.0562`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.4607`
- Arithmetic mean ratio: `1.5024`
- Best individual ratio: `gzip_crc32` vs `gcc` = `1.0641`
- Worst individual ratio: `expat_xml_scan` vs `gcc` = `2.0562`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
