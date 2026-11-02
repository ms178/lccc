# LCCC benchmark report

- **UTC:** `2026-10-25T14:47:07.409957+00:00`
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
| `llvm_scalar_facts` | 56.35 ms | 51.48 ms | 52.06 ms | 56.99 ms | GCC | 1.0946 [1.0782, 1.1066] | pass |
| `hot_web_loop` | 63.86 ms | 66.80 ms | 57.72 ms | 85.23 ms | Clang | 1.1013 [1.0804, 1.1137] | pass |
| `nbody` | 264.49 ms | 266.21 ms | 253.26 ms | 377.51 ms | Clang | 1.0484 [1.0282, 1.0603] | pass |
| `binary_trees` | 1.2103 s | 1.1130 s | 1.1379 s | 1.2077 s | GCC | 1.0806 [1.0637, 1.1152] | pass |
| `switch_dispatch` | 542.90 ms | 466.72 ms | 561.85 ms | 541.30 ms | GCC | 1.1664 [1.1508, 1.1899] | pass |
| `byte_load_reassembly` | 3.88 ms | 3.59 ms | 3.14 ms | 3.93 ms | Clang | 1.2588 [1.2366, 1.2721] | pass |
| `expat_xml_scan` | 44.77 ms | 40.92 ms | 36.21 ms | 46.80 ms | Clang | 1.2569 [1.2137, 1.2754] | pass |
| `sqlite_varint` | 26.97 ms | 24.17 ms | 28.80 ms | 27.38 ms | GCC | 1.1167 [1.1011, 1.1477] | pass |
| `chacha20_block` | 270.26 ms | 254.52 ms | 225.63 ms | 271.05 ms | Clang | 1.1947 [1.1827, 1.2167] | pass |
| `gzip_longest_match` | 160.69 ms | 153.51 ms | 121.82 ms | 162.85 ms | Clang | 1.3225 [1.3024, 1.3495] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.0696`
- Arithmetic mean ratio: `1.0711`
- Best individual ratio: `hot_web_loop` = `0.9627`
- Worst individual ratio: `switch_dispatch` = `1.1664`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.1609`
- Arithmetic mean ratio: `1.1641`
- Best individual ratio: `nbody` vs `clang` = `1.0484`
- Worst individual ratio: `gzip_longest_match` vs `clang` = `1.3225`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
