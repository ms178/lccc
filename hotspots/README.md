# LCCC hotspot database

This directory is a durable queue of **evidence-backed generated-code gaps**.
It is not a collection of optimistic ideas.  Every entry must distinguish:

- a reproducible source and benchmark command;
- correctness status and compiler revision;
- emitted-code facts (assembly/object-level evidence);
- measurement conditions and counter availability;
- a falsifiable candidate explanation; and
- the next smallest experiment.

## Entry lifecycle

| Status | Meaning |
|---|---|
| `screening` | A controlled VM or preliminary run found a candidate.  No hardware claim. |
| `reproduced` | Stable on a controlled bare-metal target or independently repeated environment. |
| `root-caused` | Assembly and/or PMU evidence identifies the compiler mechanism. |
| `prototype` | An isolated compiler change exists behind a targeted test. |
| `integrated` | Correctness, differential testing, real workloads, and regression policy passed. |
| `rejected` | Measured trade-off was not worthwhile; retain the evidence and rationale. |

## Required entry template

```text
# <workload>/<function> — <short problem>

Status:
Source / workload revision:
LCCC revision and build mode:
Reference compiler(s), versions, flags:
Reproducer command:
Correctness evidence:
Measurement environment:
Raw result location / artifact hash:
Assembly observations:
PMU observations (or explicit unavailability):
Hypothesis (falsifiable):
Candidate optimization:
Microbenchmark / real-workload validation plan:
Risk and invalidation conditions:
Result / next decision:
```

Generated binaries, full disassemblies, raw timings, and JSON reports should
be placed outside source control (or attached as CI artifacts) and referenced
by a content hash/path in the entry.  The source benchmark and enough metadata
to regenerate them must stay in the repository.

## Current entries

- [`RESEARCH_BACKLOG.md`](RESEARCH_BACKLOG.md) — prioritized, evidence-linked
  optimization queue (now including the integrated PGO v10/v11 items and new
  research spikes such as use-def chains, PGO loop-versioned devirtualization,
  and sample-based PGO).
- [`expat_xml_scan_static_leaf_inlining.md`](expat_xml_scan_static_leaf_inlining.md)
  — acyclic static leaf-function inlining (integrated; hardware follow-up).
- [`P0P1_CYCLE_2026_08_11.md`](P0P1_CYCLE_2026_08_11.md) — the 2026-08-11 P0/P1
  cycle: IVSR safety, bounded static-loop inlining, and the volatile-zero fact
  fix, each with a retained incremental patch snapshot.
- [`struct_copy_aggregate_abi.md`](struct_copy_aggregate_abi.md) — aggregate
  by-value ABI & wide-copy lowering; the largest measured gap (21×) on
  `struct_copy`.

## Integrated PGO findings (moved out of "screening")

The PGO red-team audits produced several *measured, integrated* results that are
now part of the baseline. They are recorded here so the institutional memory
keeps the **why** (and the rejected alternatives):

- **Flat-profile gate** — a profile with no single dominant hot function must
  not perturb the inliner (adler32 was -17% under `-fprofile-use`; now neutral).
- **Cost-aware devirtualization** — promoting a stable single-target indirect
  call is a net regression on modern BTB hardware; only multi-valued sites are
  promoted (op_dispatch +28% → neutral; multi_dispatch 1.07×).
- **Conservative (preserve-source-order) block layout** — reordering the hot
  loop can blow up register allocation (expat 131→248 ms); the layout pass now
  preserves source order and gets profile value from sections, switch ordering,
  and branch inversion instead.
