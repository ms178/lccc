# Register Allocation — Current State (August 2026)

**Status:** Linear-scan GPR allocator **shipped and iterated**. Docs in this
directory dated March 2026 described a *planned* 574-line 3-phase greedy
allocator. That design is obsolete. This file is the index for the **live**
system as of `ms178/lccc` main (audit date 2026-08-20).

**Mission:** generated-code performance on real hardware (i7-14700KF / Raptor
Lake). Correctness is a hard constraint. LCCC must beat GCC, Clang, and ICX
on representative C workloads — register allocation is a first-class lever.

## What actually exists

| Module | Lines (approx) | Role |
|--------|----------------|------|
| `src/backend/regalloc.rs` | 3612 | Policy: eligibility, 2c/2d/2e waves, coalescing, XMM/NEON, i686 hazards, loop-pin |
| `src/backend/live_range.rs` | 1261 | Poletto–Sarkar scan, eviction modes, hints, PGO weights, unit tests |
| `src/backend/liveness.rs` | 2163 | Backward dataflow, hole-aware **segments**, GEP-base / f128 / setjmp |
| `src/backend/split_ranges.rs` | 1377 | IR pre-pass: call-split, loop-transparent split, edge-copy layout |
| `src/backend/stack_layout/` | ~2100 | Slot packing, copy coalescing, greedy interval coloring for *stack* |
| `src/backend/stack_layout/graph_coloring.rs` | ~130 | Stack-slot coloring (not a Chaitin GPR allocator) |

There is **no** `linear_scan.rs` / `register_pool.rs`. The scan lives in
`live_range.rs`; the policy wrapper is `allocate_registers`.

## Algorithm in one paragraph

1. `split_ranges` optionally rewrites IR so fat intervals die at calls / loop
   exits (volatile spill allocas; fail-closed).
2. `compute_live_intervals` produces contiguous intervals **and** hole-aware
   `segments`.
3. Eligibility whitelist (GPR integer/ptr ops) minus never-materialized,
   inline-asm, memcpy pointers, etc.
4. Copy-group coalescing (pairwise-disjoint intervals) + phi latch coalescing.
5. **Hole-aware call-spanning:** a value needs callee-saved only if a call
   sits *inside a live segment*, not merely in a diamond gap.
6. **Phase 1** linear-scan on callee-saved for spanning values.
7. **Phase 2** caller-saved in constraint waves (call-arg regs, indirect
   target, i686 `%ecx`/`%edx` scratch, then leftover).
8. **Phase 2c** leftover callee-saved overflow (optional exchange eviction).
9. **Phase 2d/2e** i686 load-hazard refine + `%eax` homes.
10. AArch64 loop-pin steal for hot phis. XMM/NEON second class for scalars
    and a fail-closed 128-bit vecreg whitelist.
11. Stack layout assigns slots to unallocated values (3-tier packing).

Default eviction mode is **3** (hotter incoming, victim next-use after
incoming end). Mode 5 (exchange) is **not** the default: measured gzip
regression on Raptor Lake (`longest_match`).

## Progress since the March 2026 docs

The old documents claimed:

- 574-line 3-phase allocator, ~5% eligibility, 11 KB frames, Week 2 not started.

**Shipped since then (non-exhaustive):**

- Full `LinearScanAllocator` with die-at-birth sharing, cascade eviction,
  rotation for ILP, `run_with_seed` multi-wave occupancy.
- Copy-group coalescing + group-priority reweight (adler32/memcmp param homes).
- GEP-base priority (folded addressing).
- Hole-aware segments vs fat `spans_any_call`.
- Call-arg / indirect-target register exclusion (printf / fptr clobber bugs).
- i686 `%ecx`/`%edx`/`%eax` hazard model + load-hazard refinement.
- XMM scalar + 128-bit vecreg (fail-closed intrinsic whitelist).
- AArch64 folded-index liveness + loop-pin.
- `split_ranges` IR pre-pass.
- PGO hooks (`CCC_PGO_WEIGHT_MAX` default **1** — 4× cap regressed gzip +4.7%).
- Debug: `CCC_DEBUG_RA`, `CCC_TRACE_ALLOC`, `CCC_EVICT_MODE`, many kill-switches.

The 11 KB / 0-register story is **historical**. Leaf integer functions already
home many values in GPRs. Remaining gaps vs GCC/Clang/ICX are **quality**,
not “does linear scan exist?”.

## Documents in this directory

| File | Role |
|------|------|
| [CURRENT_ALLOCATOR_ANALYSIS.md](CURRENT_ALLOCATOR_ANALYSIS.md) | Live architecture, phases, env knobs |
| [LINEAR_SCAN_DESIGN.md](LINEAR_SCAN_DESIGN.md) | Implemented scan (not a Week-2 plan) |
| [INTEGRATION_POINTS.md](INTEGRATION_POINTS.md) | Call sites, `RegAllocConfig` as it is |
| [WEAKNESSES_AND_BACKLOG.md](WEAKNESSES_AND_BACKLOG.md) | Defects, cost-model holes, ideas |
| [CODE_AUDIT.md](CODE_AUDIT.md) | File-by-file audit 2026-08-20 |
| [RESEARCH_REPORT.md](RESEARCH_REPORT.md) | Literature → LCCC, milestones vs ICX/GCC/Clang |
| [COMPETITIVE_STRATEGY.md](COMPETITIVE_STRATEGY.md) | How to win the next 1% |
| [VALIDATION_ZLIB_GZIP_EXPAT.md](VALIDATION_ZLIB_GZIP_EXPAT.md) | 2026-08-20 compile+asm vs GCC `-O2` |
| [PHASE_1_IMPLEMENTATION_PLAN.md](PHASE_1_IMPLEMENTATION_PLAN.md) | Historical Week 1–3 + **Phase 2+ roadmap** |
| [BASELINE_ANALYSIS.md](BASELINE_ANALYSIS.md) | March baseline, annotated |
| [WEEK_1_COMPLETION_REPORT.md](WEEK_1_COMPLETION_REPORT.md) | Historical; do not treat as current |

## Kill-switches (non-exhaustive)

| Env | Effect |
|-----|--------|
| `CCC_NO_COALESCE` | Disable copy groups |
| `CCC_NO_PHI_COALESCE` | Disable latch coalescing |
| `CCC_NO_LEAF_PARAM_GPR` | Disable x86 leaf param copies |
| `CCC_NO_FOLDED_INDEX_LIVENESS` | Skip AArch64 index interval stretch |
| `CCC_NO_LOAD_HAZARD_REFINE` | Skip i686 2d |
| `CCC_NO_EAX_ALLOC` | Skip i686 `%eax` homes |
| `CCC_NO_LOOP_PIN` | Skip AArch64 phi steal |
| `CCC_NO_VECREG` | Skip NEON/XMM vector homes |
| `CCC_EVICT_MODE` | 0 off, 1–3 greedy windows, 5 exchange |
| `CCC_PGO_WEIGHT_MAX` | Default 1 (neutral) |
| `CCC_DEBUG_RA` / `CCC_TRACE_ALLOC` | Assignment / overlap traces |

## Success criteria (updated)

- [x] Linear scan exists and is the production allocator
- [x] Interval splitting as IR pre-pass (limited, fail-closed)
- [x] Copy coalescing (disjoint groups)
- [x] Loop-weighted spill / eviction
- [x] XMM/NEON second class (scalar + whitelist vec)
- [ ] Live-range **splitting inside the scan** (reload at next use) — **not** implemented (`enable_splitting` is a stub)
- [ ] Chaitin/Briggs or PBQP GPR coloring — **not** implemented
- [ ] Rematerialization of GlobalAddr/const as first-class — **not** implemented
- [ ] Beat GCC/Clang/ICX geomean on zlib-ng, zstd, SQLite, gzip — **open**

---

**Do not** implement from the March 2026 “Week 2 checklist”. Implement from
`regalloc.rs` + `live_range.rs` + the backlog in `WEAKNESSES_AND_BACKLOG.md`.
