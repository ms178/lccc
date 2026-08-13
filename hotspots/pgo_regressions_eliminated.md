# PGO v10/v11 — measured regressions eliminated, devirtualization made cost-aware

Status: `integrated` — correctness, differential testing, real workloads, and
regression policy passed. Hardware (Raptor Lake PMU) follow-up remains.

Source / workload revision:
- `tests/benchmark/programs/zlib_ng_adler32.c` (zlib-ng 2.2.4)
- `tests/benchmark/programs/expat_xml_scan.c` (Expat 2.6.4)
- synthetic `op_dispatch` / `multi_dispatch` indirect-dispatch kernels

LCCC revision and build mode: `ms178/lccc` main, `cargo build --release`
(Rust opt-level 1, two Cargo jobs).

Reference compiler(s), versions, flags: GCC 14.2, `-O2`.

Reproducer command:
```bash
python3 tests/benchmark/run_pgo_ab.py \
  --only expat_xml_scan,zlib_ng_adler32,gzip_crc32 --reps 31 --warmup 3
```

Correctness evidence: every kernel produced identical output in plain and
`-fprofile-use` builds; differential A/B `differential_ok=True` on all three.

Measurement environment: KVM-exposed x86-64 vCPU (no PMU), CPU-pinned
interleaved rounds, bootstrap 95% CI. Screening evidence, not a hardware claim.

Raw result location / artifact hash: `run_pgo_ab.py` JSON report (retained
outside source control).

Assembly observations:
- adler32 pre-fix: the hot NMAX loop spilled its accumulators because a *flat*
  profile (tied hot functions) perturbed the base inliner.
- expat pre-fix: the PGO layout pass placed the multi-byte UTF-8 handling before
  the hot ASCII loop (frame 40→72 B) — a ~2× hot-path regression.
- op_dispatch pre-fix: devirtualizing a loop-invariant single indirect target
  added a per-iteration compare+branch with no accuracy benefit.

PMU observations (or explicit unavailability): none — explicitly *not* a PMU
claim.

Hypothesis (falsifiable):
1. A flat profile (no single dominant hot function) carries no inlining signal;
   reading it perturbs pass iteration. → gate the inliner on `has_spread()`.
2. A stable single-target indirect call is already BTB-predicted; a guarded
   direct call is pure overhead. → only promote multi-valued sites.
3. Reordering a hot loop's blocks changes register allocation; preserve source
   order and get profile value elsewhere.

Candidate optimization:
- `summary::has_spread()` (unique dominant max) + `inline_decisions_active()`.
- `promote.rs` `LCCC_PGO_PROMOTE_STABLE=95` cost-aware devirtualization.
- `layout.rs` conservative preserve-source-order layout + backend branch
  inversion (`cond_fallthrough`).

Microbenchmark / real-workload validation plan: PGO A/B harness; regression
round-trip incl. a single-target site that must NOT be promoted and a
multi-target site that must.

Risk and invalidation conditions: a hardware PMU could overturn the
"screening-only" numbers; a workload whose base inliner is not already aggressive
could make force-inline more valuable.

Result / next decision:
- adler32: 0.827× → 0.997× (regression eliminated).
- expat: 0.718× → 1.014× (now faster than plain).
- op_dispatch (single-target): 1.28× regression → 1.00× (neutral).
- multi_dispatch (50/50): 1.07× (genuine devirtualization win retained).
- Next: repeat on i7-14700KF with PMU (branch-misses, IPC) to confirm the
  branch-prediction model.
