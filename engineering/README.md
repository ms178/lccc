# Engineering documentation

This directory is the single home of live engineering documentation.
Everything outside it (`docs/`, root `README.md`) is user-facing or
site-facing. History is layered: session-day narratives in
[`journal/`](journal/README.md) (append new session entries to the current
period file — no new per-session markdown files), root causes and
measured negatives in [`DECISIONS.md`](DECISIONS.md), raw numbers in
[`evidence/`](evidence/README.md).

## Read order for an implementation agent

1. [`STATE.md`](STATE.md) — the compiler as it is on `main` right now.
2. [`agent/RULES.md`](agent/RULES.md) — non-negotiable constraints and vetoes.
3. [`agent/BACKLOG.md`](agent/BACKLOG.md) — the full item catalog (RA/IS/OP/FE/AB/PG/LK/MS).
4. [`agent/SEQUENCE.md`](agent/SEQUENCE.md) — execution order for the current phase.
5. [`tasks/README.md`](tasks/README.md) — the active work queue; pick or claim a task.
6. [`DECISIONS.md`](DECISIONS.md) — measured negatives, reverted experiments,
   root causes. **Read before implementing anything that a DECISIONS bullet
   touches.**
7. Re-oracle against [`evidence/godbolt/`](evidence/godbolt/) and
   `scripts/godbolt.py`; measure with `tests/benchmark/run_benchmarks.py`.

## Layout

| Path | What |
|------|------|
| [`STATE.md`](STATE.md) | Current production state, gaps, constraints |
| [`DECISIONS.md`](DECISIONS.md) | Decision & negative-results ledger (distilled from deleted session journals) |
| [`journal/`](journal/README.md) | Period-file session digests, `src:` tagged entries |
| [`agent/`](agent/README.md) | Agent workflow: rules, backlog, sequence |
| [`tasks/`](tasks/README.md) | Active work queue — one file per actionable item |
| [`subsystems/`](subsystems/README.md) | Per-subsystem handbooks (contracts, knobs, defect classes) |
| [`evidence/godbolt/`](evidence/godbolt/) | Compiler Explorer oracle corpus + scoreboard |
| [`evidence/workloads/`](evidence/workloads/) | gzip / zlib-ng / Expat LCCC vs GCC |
| [`evidence/`](evidence/README.md) | Oracle corpus, workload measurements, frozen screening records; `evidence/pmu/` is the future MS-14 home |

Measure with `scripts/godbolt.py` and `tests/benchmark/run_benchmarks.py`.
Build: `scripts/build_lccc_fast.sh` → `target/fastbuild/lccc`.
