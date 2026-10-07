# 2026-10-07 — IVSR-PTRADD-1 performance evidence

Backing data for
[`../../FOLLOWUP-2026-10-07-perf-ivsr-ptradd.md`](../../FOLLOWUP-2026-10-07-perf-ivsr-ptradd.md).

| file | what it is |
|---|---|
| `callgrind-kernel-ab.json` | Callgrind `Ir` per kernel for the landed change, three arms (pre / post / system GCC 14.2 on one driver), plus the wall-clock confirmation for `varint`, the static blast radius and the ILP32 gate measurement |

This file was filed under `2026-10-06-ivsr-domain/` until 2026-10-07: it measures
the October-7 performance round against base `66c24528`, not the October-6
correctness round. The record itself is unchanged apart from `date`, `refile_reason`
and a corrected `why_not_wall_clock` (the old text claimed runtime was unmeasurable
here; wall clock was in fact measured and is the confirmation metric).

Everything here is Callgrind `Ir`, static instruction counts, or VM wall clock.
**No PMU counters, no cycles, no uops, no Raptor Lake.** The host is a Xeon VM
without a usable PMU; see `host` and `not_measured` inside the JSON.
