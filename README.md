# LCCC — Lightning Fast Claude's C Compiler

> An optimized fork of [CCC](https://github.com/anthropics/claudes-c-compiler) based on additions made by Lev Kropp (https://www.levkropp.com/lccc/).

---

## What is LCCC?

CCC (Claude's C Compiler) is a zero-dependency C compiler written entirely in Rust by Claude Opus 4.6 and Arena.ai Agents,
capable of compiling real projects — gzip, zlib-ng, expat, SQLite, the Linux kernel and glibc — for x86-64, AArch64,
RISC-V 64, and i686, with its own assembler and linker.

LCCC is a performance fork and a personal AI agent research project.

---

## Licensing

LCCC uses a dual-license model to separate original contributions from CCC-derived code.

**LCCC contributions** (new files, regalloc changes, benchmarks, docs) —
MIT OR Apache-2.0 OR BSD-2-Clause (your choice). See `LICENSE-MIT`, `LICENSE-APACHE`, `LICENSE-BSD`.

**CCC-derived code** (frontend, SSA IR, optimizer, backends, assembler, linker) —
CC0 1.0 Universal (public domain). CCC was released as CC0 by Anthropic.

**Workload-derived benchmark kernels** — retain the per-file upstream license
and provenance (for example GPL/LGPL/MIT/Zlib/public-domain material); they
are documented in [`tests/benchmark/WORKLOAD_PROVENANCE.md`](tests/benchmark/WORKLOAD_PROVENANCE.md).

See [`LICENSING.md`](LICENSING.md) for the full breakdown and per-file guidance.

# LCCC benchmark report

- **UTC:** `2026-08-11T12:23:52.895739+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'none', 'allowed_cpus': [0, 1], 'applied': False, 'reason': 'affinity disabled by user'}`
- **PMU:** `not probed`
- **LCCC revision:** `a3401812aeaf83b23b79722a1487005e9f61696f`
- **LCCC binary SHA-256:** `297a5f7c4ecc959d11f7ec468b66baa0b2ff6c15e04b5227338dffc57846bd16`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | CLANG median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | --- | ---: | :---: |
| `arith_loop` | 104.38 ms | 99.68 ms | — | GCC | 1.0309 [1.0080, 1.0848] | pass |
| `fib` | 1.43 ms | 148.88 ms | — | GCC | 0.0095 [0.0094, 0.0100] | pass |
| `matmul` | 7.83 ms | 4.01 ms | — | GCC | 1.9966 [1.7774, 2.1227] | pass |
| `qsort` | 137.90 ms | 125.62 ms | — | GCC | 1.1105 [1.1010, 1.1791] | pass |
| `sieve` | 51.58 ms | 35.26 ms | — | GCC | 1.4799 [1.3714, 1.5484] | pass |
| `tce_sum` | 1.15 ms | 1.15 ms | — | GCC | 0.9801 [0.9429, 1.0812] | pass |
| `nbody` | 1.8829 s | 228.88 ms | — | GCC | 8.2145 [8.0949, 8.2796] | pass |
| `binary_trees` | 2.1316 s | 1.3889 s | — | GCC | 1.2933 [1.1552, 1.4806] | pass |
| `spectral_norm` | 3.7202 s | 306.85 ms | — | GCC | 12.6666 [10.2006, 14.2896] | pass |
| `mandelbrot` | 5.3586 s | 1.1228 s | — | GCC | 4.7661 [4.7419, 4.8463] | pass |
| `hash_table` | 14.1032 s | 11.4875 s | — | GCC | 1.2202 [1.2018, 1.3023] | pass |
| `strlen_bench` | 295.86 ms | 262.52 ms | — | GCC | 1.0925 [0.9527, 1.1995] | pass |
| `switch_dispatch` | 746.10 ms | 496.24 ms | — | GCC | 1.5024 [1.4588, 1.5148] | pass |
| `struct_copy` | 264.79 ms | 42.39 ms | — | GCC | 6.2070 [4.0378, 6.3690] | pass |
| `loop_patterns` | 207.95 ms | 115.19 ms | — | GCC | 1.7121 [1.2572, 2.3773] | pass |
| `fannkuch` | 4.5073 s | 4.2267 s | — | GCC | 1.0451 [0.9309, 1.1128] | pass |
| `ackermann` | 1.44 ms | 150.46 ms | — | GCC | 0.0092 [0.0086, 0.0098] | pass |
| `constant_recursion` | 1.41 ms | 150.50 ms | — | GCC | 0.0089 [0.0082, 0.0124] | pass |
| `bitops` | 747.94 ms | 318.24 ms | — | GCC | 2.3383 [2.3228, 2.3584] | pass |
| `gzip_crc32` | 242.57 ms | 167.05 ms | — | GCC | 1.4528 [1.4512, 1.4598] | pass |
| `zlib_ng_adler32` | 66.44 ms | 37.86 ms | — | GCC | 1.7546 [1.7458, 1.7572] | pass |
| `expat_xml_scan` | 130.74 ms | 31.03 ms | — | GCC | 4.2225 [4.1348, 4.2826] | pass |
| `sqlite_varint` | 53.39 ms | 21.98 ms | — | GCC | 2.4334 [2.3998, 2.5050] | pass |
| `linux_find_bit` | 34.82 ms | 12.74 ms | — | GCC | 2.4492 [2.0395, 3.0768] | pass |
| `glibc_memcmp` | 10.59 ms | 9.35 ms | — | GCC | 1.1414 [1.1258, 1.1505] | pass |

## Aggregate LCCC/GCC (correct pairs only)

- Geometric mean ratio: `1.0761`
- Arithmetic mean ratio: `2.4855`
- Best individual ratio: `constant_recursion` = `0.0089`
- Worst individual ratio: `spectral_norm` = `12.6666`

## Aggregate LCCC / fastest available reference (correct pairs only)

- Geometric mean ratio: `1.0761`
- Arithmetic mean ratio: `2.4855`
- Best individual ratio: `constant_recursion` vs `gcc` = `0.0089`
- Worst individual ratio: `spectral_norm` vs `gcc` = `12.6666`

A ratio below 1 means LCCC was faster.  This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
