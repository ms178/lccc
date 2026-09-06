---
layout: doc
title: Benchmarks
description: Reproducible generated-code measurements for LCCC.
prev_page:
  title: Optimization Passes
  url: /docs/optimization-passes
---

# Benchmarks
{:.doc-subtitle}

LCCC treats performance as a generated-code question. Every claimed result
must have a correctness oracle, an inspectable assembly path, and a repeatable
measurement protocol. A VM result is screening evidence, not a substitute for
bare-metal counters.

## Canonical runner

[`tests/benchmark/run_benchmarks.py`](../tests/benchmark/run_benchmarks.py) is
the authoritative runner. It:

- compiles each source with LCCC and the selected references at the same C
  optimization level;
- compares deterministic stdout and exit status before accepting a timing row;
- runs one excluded warm-up, randomizes compiler order in each paired round,
  pins to one allowed CPU when possible, and retains every sample;
- reports paired medians, bootstrap intervals, coefficient of variation, MAD
  outlier counts, geometric/arithmetic aggregates, and compiler/toolchain
  metadata; and
- optionally retains binaries, commands, disassembly, section sizes, and raw
  JSON evidence.

The default compiler set is LCCC, GCC, Clang, and ICX when available. CI uses
LCCC and GCC for the complete registered corpus through
[`.github/scripts/ci-bench.py`](../.github/scripts/ci-bench.py); it does not use
a reduced smoke list. Reference compilers are not assumed optimal: promising
results should be followed by assembly comparison and controlled A/B runs.

Build LCCC under the project policy first:

```bash
./scripts/build_lccc_o1_j2.sh
```

This means Rust compiler opt-level 1 and exactly two Cargo jobs. The C program
optimization flag is independent and defaults to `-O2`.

## Reproduce the current CI-sized run

```bash
python3 tests/benchmark/run_benchmarks.py \
  --lccc target/release/lccc \
  --compilers lccc,gcc \
  --reps 9 --warmup 1 --cpu auto \
  --artifact-dir results/bench/artifacts \
  --json results/bench/results.json \
  --markdown results/bench/report.md \
  --seed 20260906 --strict
```

The checked-in result of this command is
[`engineering/evidence/benchmarks/2026-09-06-rust198-3e1b71dd/`](../engineering/evidence/benchmarks/2026-09-06-rust198-3e1b71dd/).
It contains the raw per-round JSON, rendered report, terminal transcript, and
reproduction details.

## Current corpus and result

The corpus contains **33 programs**: compiler-focused kernels plus extracts
from gzip, zlib-ng, Expat, SQLite, glibc, and Linux. The workload-derived
sources and licenses are recorded in
[`tests/benchmark/WORKLOAD_PROVENANCE.md`](../tests/benchmark/WORKLOAD_PROVENANCE.md).
Full-project runners are available under [`tests/workloads/`](../tests/workloads/)
and are intentionally separate from the fast CI corpus.

Fresh screening on 2026-09-06, commit `f7f88be8`, with nine paired rounds:

- **33/33** outputs matched the GCC oracle;
- geometric mean LCCC/GCC: **0.7314**;
- arithmetic mean LCCC/GCC: **0.9530**;
- best row: `fib`, **0.016×** GCC;
- slowest row: `expat_xml_scan`, **1.407×** GCC.

A ratio below 1 means LCCC was faster. Selected rows:

| Program | LCCC median | GCC median | LCCC/GCC |
|---|---:|---:|---:|
| `fib` | 1.83 ms | 117.39 ms | 0.016 |
| `constant_recursion` | 1.91 ms | 57.04 ms | 0.033 |
| `libm_round_family` | 186.40 ms | 451.22 ms | 0.419 |
| `gzip_crc32` | 122.87 ms | 135.88 ms | 0.882 |
| `zlib_ng_adler32` | 35.56 ms | 35.39 ms | 1.007 |
| `sqlite_varint` | 24.13 ms | 18.67 ms | 1.292 |
| `expat_xml_scan` | 44.81 ms | 31.87 ms | 1.407 |
| `linux_find_bit` | 14.08 ms | 10.20 ms | 1.383 |

The host was a hypervisor-backed two-vCPU VM. CPU 0 pinning worked, but no
usable PMU was installed; wall-clock values are therefore screening numbers.
Every sample, including noisy or sub-20-ms rows, remains in the raw report.

## Correctness and code-quality gates

Use the fast full-corpus correctness gate during development:

```bash
LCCC_BIN=target/release/lccc ./scripts/check_benchmark_outputs.sh
```

The assembly guardrail is:

```bash
python3 .github/scripts/ci-codegen-gate.py \
  --lccc target/release/lccc --summary
```

It checks instruction, stack-memory, moves, callee-save, and vector metrics for
the golden gzip, zlib-ng, Expat, SQLite, glibc, hash-table, and stencil
workloads. A baseline refresh is valid only when the new assembly has been
inspected and correctness remains green.

For profile-guided work, use
[`tests/benchmark/run_pgo_ab.py`](../tests/benchmark/run_pgo_ab.py). For
compiler-oracle and assembly work, see [`scripts/godbolt.py`](../scripts/godbolt.py),
[`scripts/kernel_count.py`](../scripts/kernel_count.py), and the linker oracle
scripts. The VM cannot provide PMU evidence; bare-metal claims require a
controlled affinity/governor/thermal setup with cycles, instructions, branches,
and cache/TLB counters where available.
