# LCCC benchmark report

- **UTC:** `2026-11-13T22:21:15.226455+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `4c3c081731291900b3966037dab48228b5cd329a
`
- **LCCC binary SHA-256:** `ccbe1ebbbc7c8b3732d6b52c1394f3befb76466b5efcce013044559cf6a11fcc`
- **Optimization:** `-O2`; common flags: `[]`
- **Rounds / excluded warm-ups:** `11 / 2`
- **Method:** randomized balanced two-round blocks for two compilers; independent shuffle otherwise; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | CLANG median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | --- | ---: | :---: |
| `gzip_crc32` | 168.05 ms | 186.52 ms | 184.57 ms | Clang | 0.9201 [0.8940, 0.9401] | pass |
| `zlib_ng_adler32` | 43.44 ms | 45.34 ms | 42.17 ms | Clang | 1.0283 [1.0069, 1.0580] | pass |
| `zlib_ng_compare256` | 10.56 ms | 9.62 ms | 8.65 ms | Clang | 1.2054 [1.0979, 1.3497] | pass |
| `expat_xml_scan` | 49.13 ms | 56.38 ms | 45.10 ms | Clang | 1.1184 [1.0202, 1.1458] | pass |
| `sqlite_varint` | 73.21 ms | 67.59 ms | 68.35 ms | GCC | 1.0836 [0.9870, 1.1239] | pass |
| `linux_find_bit` | 30.40 ms | 29.78 ms | 28.83 ms | Clang | 1.0528 [1.0441, 1.0712] | pass |
| `glibc_strstr` | 4.4767 s | 4.4394 s | 4.7255 s | GCC | 1.0009 [0.9911, 1.0084] | pass |
| `gzip_longest_match` | 183.25 ms | 171.45 ms | 157.01 ms | Clang | 1.1849 [1.1254, 1.3546] | pass |
| `zlib_ng_slide_hash` | 4.91 ms | 5.63 ms | 4.90 ms | Clang | 1.0352 [0.9175, 1.0822] | pass |
| `expat_utf8_content` | 57.91 ms | 66.56 ms | 61.77 ms | Clang | 0.9441 [0.9099, 0.9846] | pass |

## Aggregate LCCC/GCC (correct pairs only)

Workload mix and extreme specialized or short tests can dominate; this is not a whole-application score.

- Geometric mean ratio: `1.0029`
- Arithmetic mean ratio: `1.0060`
- Best individual ratio: `expat_utf8_content` = `0.8729`
- Worst individual ratio: `zlib_ng_compare256` = `1.1251`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.0537`
- Arithmetic mean ratio: `1.0574`
- Best individual ratio: `gzip_crc32` vs `clang` = `0.9201`
- Worst individual ratio: `zlib_ng_compare256` vs `clang` = `1.2054`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
