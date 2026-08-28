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
