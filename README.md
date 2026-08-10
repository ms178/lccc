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

See [`LICENSING.md`](LICENSING.md) for the full breakdown and per-file guidance.

# LCCC benchmark report

- **UTC:** `2026-08-10T22:31:03.954433+00:00`
- **CPU model(s):** `Intel(R) Xeon(R) Processor @ 2.60GHz`
- **Hypervisor detected:** `True`
- **CPU pinning:** `{'requested': 'auto', 'allowed_cpus': [0, 1], 'applied': True, 'cpu': 0, 'reason': 'taskset pinning'}`
- **PMU:** `perf is not installed`
- **LCCC revision:** `7121253f4ea4d8a9cdeb6718f4b63cea07a932cc`
- **LCCC binary SHA-256:** `01df03d848fc3b1cf135a8bcbe4610c3818656c2df623840af7b432d3c01c7c1`
- **Method:** randomized compiler order within each paired round; warm-ups excluded; median wall time and paired bootstrap CI; no automatic outlier removal.

| Benchmark | LCCC median | GCC median | CLANG median | Best reference | LCCC/best paired (95% bootstrap CI) | Correct |
| --- | ---: | ---: | ---: | --- | ---: | :---: |
| `gzip_crc32` | 244.64 ms | 168.55 ms | 167.98 ms | Clang | 1.4560 [1.4548, 1.4616] | pass |
| `zlib_ng_adler32` | 81.57 ms | 39.24 ms | 37.08 ms | Clang | 2.1994 [2.1953, 2.2035] | pass |
| `expat_xml_scan` | 131.16 ms | 40.16 ms | 53.83 ms | GCC | 3.2795 [3.2612, 3.2863] | pass |
| `sqlite_varint` | 55.66 ms | 26.91 ms | 28.33 ms | GCC | 2.0770 [2.0596, 2.1045] | pass |
| `linux_find_bit` | 26.53 ms | 14.85 ms | 18.09 ms | GCC | 1.7776 [1.7717, 1.7858] | pass |
| `glibc_memcmp` | 14.56 ms | 9.39 ms | 9.22 ms | Clang | 1.5789 [1.5579, 1.5901] | pass |

## Aggregate LCCC/GCC (correct pairs only)
- Geometric mean ratio: `1.9602`
- Arithmetic mean ratio: `2.0363`
- Best individual ratio: `gzip_crc32` = `1.4519`
- Worst individual ratio: `expat_xml_scan` = `3.2795`

## Aggregate LCCC / fastest available reference (correct pairs only)
- Geometric mean ratio: `1.9852`
- Arithmetic mean ratio: `2.0614`
- Best individual ratio: `gzip_crc32` vs `clang` = `1.4560`
- Worst individual ratio: `expat_xml_scan` vs `gcc` = `3.2795`

A ratio below 1 means LCCC was faster. This report is screening evidence; a VM without a verified PMU is not evidence for a Raptor Lake microarchitectural claim.
