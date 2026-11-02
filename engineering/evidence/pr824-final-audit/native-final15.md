# LCCC benchmark report

- **UTC:** `2026-10-14T22:04:30.574142+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `not probed`
- **LCCC revision:** `1098be7ae7b7a6fa725f83856ad3d530eb9f3b2d`
- **LCCC binary SHA-256:** `f256cc0507798388764194b1771eaf2cc0fe68ac780078f7954f6369429f2ebc`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | CLANG median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | --- | ---: | :---: |
| `sieve` | 40.08 ms | 37.05 ms | 34.59 ms | Clang | 1.1595 [1.0978, 1.1693] | pass |
| `binary_trees` | 1.2720 s | 1.1194 s | 1.1812 s | GCC | 1.1377 [1.1242, 1.1459] | pass |
| `spectral_norm` | 142.67 ms | 100.53 ms | 194.68 ms | GCC | 1.4068 [1.3767, 1.4333] | pass |
| `mandelbrot` | 1.1095 s | 919.54 ms | 933.78 ms | GCC | 1.1988 [1.1937, 1.2198] | pass |
| `strlen_bench` | 220.32 ms | 213.44 ms | 205.73 ms | Clang | 1.0842 [1.0580, 1.0898] | pass |
| `switch_dispatch` | 603.04 ms | 507.20 ms | 583.80 ms | GCC | 1.1910 [1.1642, 1.2317] | pass |
| `byte_load_reassembly` | 5.82 ms | 3.29 ms | 2.97 ms | Clang | 1.9481 [1.7392, 1.9871] | pass |
| `zlib_ng_adler32_combine` | 569.53 ms | 546.85 ms | 535.38 ms | Clang | 1.0642 [1.0535, 1.0937] | pass |
| `expat_xml_scan` | 50.90 ms | 45.18 ms | 39.31 ms | Clang | 1.2987 [1.2486, 1.3423] | pass |
| `expat_siphash24` | 267.49 ms | 264.62 ms | 251.30 ms | Clang | 1.0651 [0.9647, 1.1457] | pass |
| `sqlite_varint` | 27.59 ms | 25.19 ms | 28.33 ms | GCC | 1.0925 [1.0715, 1.2349] | pass |
| `linux_find_bit` | 17.52 ms | 13.58 ms | 15.05 ms | GCC | 1.2315 [1.1720, 1.2899] | pass |
| `linux_find_bit_scaled` | 770.51 ms | 628.10 ms | 610.35 ms | Clang | 1.2744 [1.2269, 1.3468] | pass |
| `sha256_transform` | 280.03 ms | 251.36 ms | 288.44 ms | GCC | 1.1049 [1.0707, 1.1664] | pass |
| `lz4_match_extend` | 1.0212 s | 940.20 ms | 990.41 ms | GCC | 1.0900 [1.0078, 1.1299] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.1641`
- Arithmetic mean ratio: `1.1750`
- Best individual ratio: `expat_siphash24` = `1.0016`
- Worst individual ratio: `byte_load_reassembly` = `1.7066`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.2080`
- Arithmetic mean ratio: `1.2232`
- Best individual ratio: `zlib_ng_adler32_combine` vs `clang` = `1.0642`
- Worst individual ratio: `byte_load_reassembly` vs `clang` = `1.9481`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
