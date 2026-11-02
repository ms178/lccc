# Folded chunk reports — 2026-08-28 run @ 1b3994e7

Reports for the three chunk JSONs (`chunkA.json`, `chunkB.json`, `chunkC.json`) plus the re-verified merge; folded into one file 2026-09-16.


---

<!-- folded from `chunkA.md` -->

# LCCC benchmark report

- **UTC:** `2026-08-28T18:37:35.651815+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `1b3994e7e48808b9637b37a1f004d091b792f1fc`
- **LCCC binary SHA-256:** `c86d62b0f9b1143644d53593ded215edb799d6b41d36edb721a0a16bf4fbc68b`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `nbody` | 263.99 ms | 214.54 ms | GCC | 1.2333 [1.2272, 1.2388] | pass |
| `binary_trees` | 2.0717 s | 1.9536 s | GCC | 1.0570 [1.0457, 1.0596] | pass |
| `spectral_norm` | 237.28 ms | 181.72 ms | GCC | 1.3053 [1.3027, 1.3126] | pass |
| `mandelbrot` | 1.1027 s | 894.41 ms | GCC | 1.2320 [1.2312, 1.2361] | pass |
| `fannkuch` | 2.8804 s | 2.2596 s | GCC | 1.2744 [1.2736, 1.2773] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.2172`
- Arithmetic mean ratio: `1.2204`
- Best individual ratio: `binary_trees` = `1.0570`
- Worst individual ratio: `spectral_norm` = `1.3053`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.2172`
- Arithmetic mean ratio: `1.2204`
- Best individual ratio: `binary_trees` vs `gcc` = `1.0570`
- Worst individual ratio: `spectral_norm` vs `gcc` = `1.3053`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.


---

<!-- folded from `chunkB.md` -->

# LCCC benchmark report

- **UTC:** `2026-08-28T18:42:00.838439+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `1b3994e7e48808b9637b37a1f004d091b792f1fc`
- **LCCC binary SHA-256:** `c86d62b0f9b1143644d53593ded215edb799d6b41d36edb721a0a16bf4fbc68b`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 113.97 ms | 92.57 ms | GCC | 1.2266 [1.2186, 1.2335] | pass |
| `fib` | 1.25 ms | 129.74 ms | GCC | 0.0096 [0.0089, 0.0106] | pass |
| `matmul` | 5.25 ms | 5.64 ms | GCC | 0.9383 [0.9259, 0.9464] | pass |
| `qsort` | 109.34 ms | 112.45 ms | GCC | 0.9740 [0.9649, 0.9760] | pass |
| `sieve` | 49.94 ms | 42.01 ms | GCC | 1.2004 [1.1526, 1.2356] | pass |
| `tce_sum` | 0.76 ms | 0.77 ms | GCC | 1.0029 [0.9458, 1.0240] | pass |
| `strlen_bench` | 217.35 ms | 210.13 ms | GCC | 1.0410 [1.0249, 1.0468] | pass |
| `switch_dispatch` | 466.64 ms | 477.91 ms | GCC | 0.9753 [0.9723, 0.9766] | pass |
| `struct_copy` | 29.69 ms | 21.81 ms | GCC | 1.3465 [1.3381, 1.3719] | pass |
| `loop_patterns` | 45.73 ms | 43.56 ms | GCC | 1.0589 [0.9938, 1.1195] | pass |
| `ackermann` | 1.10 ms | 61.61 ms | GCC | 0.0179 [0.0165, 0.0184] | pass |
| `constant_recursion` | 1.01 ms | 61.30 ms | GCC | 0.0164 [0.0160, 0.0170] | pass |
| `bitops` | 167.75 ms | 299.70 ms | GCC | 0.5582 [0.5567, 0.5598] | pass |
| `double_reduction` | 105.40 ms | 109.76 ms | GCC | 0.9545 [0.9489, 0.9631] | pass |
| `ascii_case_fold` | 0.93 ms | 0.89 ms | GCC | 1.0656 [1.0258, 1.1462] | pass |
| `binary_search` | 1.01 ms | 1.03 ms | GCC | 0.9906 [0.9617, 1.0261] | pass |
| `ring_fifo` | 0.92 ms | 0.91 ms | GCC | 1.0401 [1.0145, 1.0602] | pass |
| `aarch64_select_patterns` | 123.67 ms | 106.31 ms | GCC | 1.1651 [1.1629, 1.1685] | pass |
| `histogram` | 1.58 ms | 1.50 ms | GCC | 1.0479 [1.0397, 1.0656] | pass |
| `gzip_crc32` | 135.47 ms | 155.30 ms | GCC | 0.8732 [0.8664, 0.8755] | pass |
| `libm_round_family` | 202.36 ms | 490.84 ms | GCC | 0.4126 [0.4106, 0.4132] | pass |
| `tls_seg_access` | 19.57 ms | 9.12 ms | GCC | 2.1465 [2.1175, 2.1529] | pass |
| `zlib_ng_adler32` | 56.26 ms | 37.45 ms | GCC | 1.5045 [1.4829, 1.5218] | pass |
| `expat_xml_scan` | 62.80 ms | 34.94 ms | GCC | 1.8025 [1.7979, 1.8062] | pass |
| `sqlite_varint` | 26.14 ms | 21.47 ms | GCC | 1.1921 [1.1718, 1.2393] | pass |
| `linux_find_bit` | 15.29 ms | 10.15 ms | GCC | 1.5083 [1.5062, 1.5134] | pass |
| `glibc_memcmp` | 6.15 ms | 5.91 ms | GCC | 1.0299 [1.0213, 1.0454] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `0.6632`
- Arithmetic mean ratio: `1.0037`
- Best individual ratio: `fib` = `0.0096`
- Worst individual ratio: `tls_seg_access` = `2.1465`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `0.6632`
- Arithmetic mean ratio: `1.0037`
- Best individual ratio: `fib` vs `gcc` = `0.0096`
- Worst individual ratio: `tls_seg_access` vs `gcc` = `2.1465`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.


---

<!-- folded from `chunkC.md` -->

# LCCC benchmark report

- **UTC:** `2026-08-28T18:43:28.068648+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `1b3994e7e48808b9637b37a1f004d091b792f1fc`
- **LCCC binary SHA-256:** `c86d62b0f9b1143644d53593ded215edb799d6b41d36edb721a0a16bf4fbc68b`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | --- | ---: | :---: |
| `hash_table` | 21.1793 s | 19.5491 s | GCC | 1.0899 [1.0890, 1.0992] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.0899`
- Arithmetic mean ratio: `1.0899`
- Best individual ratio: `hash_table` = `1.0899`
- Worst individual ratio: `hash_table` = `1.0899`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.0899`
- Arithmetic mean ratio: `1.0899`
- Best individual ratio: `hash_table` vs `gcc` = `1.0899`
- Worst individual ratio: `hash_table` vs `gcc` = `1.0899`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.


---

<!-- folded from `merged-verified.md` -->

Ratio definition: all ratios are the runner's median of PAIRED per-round
ratios (not quotients of the medians below); aggregates 0.7381 (33) /
1.0963 (30 conventional) / 1.2197 (workload subset).

== run metadata ==
chunkA.json: 2026-08-28T18:37:35.651815+00:00  hv=True  pin={'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning', 'requested': 'auto'}  pmu=perf is not installed
chunkB.json: 2026-08-28T18:42:00.838439+00:00  hv=True  pin={'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning', 'requested': 'auto'}  pmu=perf is not installed
chunkC.json: 2026-08-28T18:43:28.068648+00:00  hv=True  pin={'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning', 'requested': 'auto'}  pmu=perf is not installed

pairs: 33  correct: 33  FAILED: []
geomean ALL (33 pairs): 0.7381
geomean conventional (30 pairs, excluding ['ackermann', 'constant_recursion', 'fib']): 1.0963
geomean codecs/parsers (workload-derived): 1.2197

| Kernel | LCCC (ms) | GCC (ms) | LCCC/GCC | 95% CI | CV | n |
|---|---:|---:|---:|---|---:|---:|
| fib | 1.2 | 129.7 | 0.010 | [0.009, 0.011] | 0.179 | 15 |
| constant_recursion | 1.0 | 61.3 | 0.016 | [0.016, 0.017] | 0.165 | 15 |
| ackermann | 1.1 | 61.6 | 0.018 | [0.017, 0.018] | 0.162 | 15 |
| libm_round_family | 202.4 | 490.8 | 0.413 | [0.411, 0.413] | 0.101 | 15 |
| bitops | 167.7 | 299.7 | 0.558 | [0.557, 0.560] | 0.004 | 15 |
| gzip_crc32 | 135.5 | 155.3 | 0.873 | [0.866, 0.876] | 0.012 | 15 |
| matmul | 5.3 | 5.6 | 0.938 | [0.926, 0.946] | 0.026 | 15 |
| double_reduction | 105.4 | 109.8 | 0.955 | [0.949, 0.963] | 0.019 | 15 |
| qsort | 109.3 | 112.5 | 0.974 | [0.965, 0.976] | 0.006 | 15 |
| switch_dispatch | 466.6 | 477.9 | 0.975 | [0.972, 0.977] | 0.003 | 15 |
| binary_search | 1.0 | 1.0 | 0.991 | [0.962, 1.026] | 0.691 | 15 |
| tce_sum | 0.8 | 0.8 | 1.003 | [0.946, 1.024] | 0.082 | 15 |
| glibc_memcmp | 6.1 | 5.9 | 1.030 | [1.021, 1.045] | 0.044 | 15 |
| ring_fifo | 0.9 | 0.9 | 1.040 | [1.014, 1.060] | 1.093 | 15 |
| strlen_bench | 217.4 | 210.1 | 1.041 | [1.025, 1.047] | 0.015 | 15 |
| histogram | 1.6 | 1.5 | 1.048 | [1.040, 1.066] | 0.032 | 15 |
| binary_trees | 2071.7 | 1953.6 | 1.057 | [1.046, 1.060] | 0.006 | 15 |
| loop_patterns | 45.7 | 43.6 | 1.059 | [0.994, 1.120] | 0.042 | 15 |
| ascii_case_fold | 0.9 | 0.9 | 1.066 | [1.026, 1.146] | 0.095 | 15 |
| hash_table | 21179.3 | 19549.1 | 1.090 | [1.089, 1.099] | 0.055 | 8 |
| aarch64_select_patterns | 123.7 | 106.3 | 1.165 | [1.163, 1.168] | 0.008 | 15 |
| sqlite_varint | 26.1 | 21.5 | 1.192 | [1.172, 1.239] | 0.026 | 15 |
| sieve | 49.9 | 42.0 | 1.200 | [1.153, 1.236] | 0.029 | 15 |
| arith_loop | 114.0 | 92.6 | 1.227 | [1.219, 1.234] | 0.007 | 15 |
| mandelbrot | 1102.7 | 894.4 | 1.232 | [1.231, 1.236] | 0.022 | 15 |
| nbody | 264.0 | 214.5 | 1.233 | [1.227, 1.239] | 0.049 | 15 |
| fannkuch | 2880.4 | 2259.6 | 1.274 | [1.274, 1.277] | 0.007 | 15 |
| spectral_norm | 237.3 | 181.7 | 1.305 | [1.303, 1.313] | 0.012 | 15 |
| struct_copy | 29.7 | 21.8 | 1.346 | [1.338, 1.372] | 0.016 | 15 |
| zlib_ng_adler32 | 56.3 | 37.4 | 1.504 | [1.483, 1.522] | 0.010 | 15 |
| linux_find_bit | 15.3 | 10.1 | 1.508 | [1.506, 1.513] | 0.010 | 15 |
| expat_xml_scan | 62.8 | 34.9 | 1.803 | [1.798, 1.806] | 0.010 | 15 |
| tls_seg_access | 19.6 | 9.1 | 2.146 | [2.118, 2.153] | 0.011 | 15 |

lccc binary sha256: c86d62b0f9b1143644d53593ded215edb799d6b41d36edb721a0a16bf4fbc68b
lccc git revision: 1b3994e7e48808b9637b37a1f004d091b792f1fc
