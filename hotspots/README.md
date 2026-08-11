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
  optimization queue.
- [`expat_xml_scan_static_leaf_inlining.md`](expat_xml_scan_static_leaf_inlining.md)
  — VM-screening candidate for static leaf-function inlining.
