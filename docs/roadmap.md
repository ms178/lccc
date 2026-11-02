---
layout: doc
title: Roadmap
description: LCCC phases — what shipped, and the current execution order.
prev_page:
  title: Benchmarks
  url: /docs/benchmarks
next_page:
  title: Licensing
  url: /docs/licensing
---

# Roadmap
{:.doc-subtitle}

The numbered phases (1–20) and PGO8–11 are **complete** and recorded in git
history — this page reflects the current state of `main` only. The live
work management lives in the engineering tree:
[`engineering/agent/BACKLOG.md`](https://github.com/ms178/lccc/blob/main/engineering/agent/BACKLOG.md)
(item catalog), [`engineering/tasks/`](https://github.com/ms178/lccc/tree/main/engineering/tasks)
(active queue), and
[`engineering/agent/SEQUENCE.md`](https://github.com/ms178/lccc/blob/main/engineering/agent/SEQUENCE.md)
(execution order).

## Status

- **Production quality bar:** gzip / zlib-ng / expat / SQLite compile and
  run byte-verified against GCC; the Linux-kernel boot code (i686, -m16)
  compiles + links with the in-tree assembler/linker and passes the 32 KiB
  boot gate; glibc 2.44 compiles end-to-end at `-O2 -march=x86-64-v3 -fPIC`
  with documented capability gates.
- **Performance:** `-O2` geomean ≈ 0.85× vs GCC 14.2 (conventional code
  ≈ 0.95×); wins include fib (109×), libm round family (2.43×), matmul
  (2.17×), bitops (1.7×), gzip CRC (1.16×). See
  [Benchmarks](/docs/benchmarks) for the full table and methodology.

## Execution order (summary; details in the engineering tree)

1. **P0 codegen** — RA-06 reload-at-use + arithmetic-chain copy webs
   (adler 1.63×), RA-06 location pieces — on `sha256_transform` gcc is still
   **44% faster** (measured) and emits 8 frame-relative stack refs against
   LCCC's 62, with the *same* loop count, so the gap is spill traffic and
   redundant moves rather than loop structure; the web-wide in-loop-use supply
   landed (+3.6…+4.3% runtime, hottest slot 17→10 reloads) and RA-06B's
   back-edge-split framing is superseded by it, OP-05b
   multi-store scatter (nbody/spectral), PF-17 loop-rotation hardening →
   default-enable (the systemic ~1 branch/iter gap), RA-01b
   marching-pointer homes.
2. **Platform gates** — LK-19 IFUNC end-to-end, LK-24 external-PIE startup
   SIGSEGV, kernel objtool interop + `net/*` statement-expression typing.
3. **P1 measured gaps** — PF-06 isort secondary IV, IS-11 andn+cmov ffs,
   FE-25 real `-march=native` (Raptor Lake), PERF-3 TLS addressing,
   expat `imul` chains.
4. **Hygiene** — MS-08 env-knob consolidation, MS-10 CI SSA validation,
   MS-11 glibc `make check` triage harness, MS-14 PMU harness for the
   14700KF.

## Hard rules that shape the roadmap

- gzip `longest_match` stack-mem must not rise; adler/gzip/expat oracles
  gate all RA work.
- Loop rotation stays opt-in until the 15 known miscompile shapes are
  root-caused.
- `FpContract::Off` is the default (GCC `gnu*` parity); FMA emission
  requires the FMA3 ISA feature.
- Every optimization ships with a kill switch and a structural or runtime
  gate; measured negatives are recorded and never silently re-tried
  ([`engineering/DECISIONS.md`](https://github.com/ms178/lccc/blob/main/engineering/DECISIONS.md)).
