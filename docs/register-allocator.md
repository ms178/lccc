---
layout: doc
title: Register Allocator
description: Current linear-scan register allocator.
prev_page:
  title: Architecture
  url: /docs/architecture
next_page:
  title: Optimization Passes
  url: /docs/optimization-passes
---

**Engineering index:** [`engineering/subsystems/register-allocation.md`](https://github.com/ms178/lccc/blob/main/engineering/subsystems/register-allocation.md) · backlog [`engineering/agent/BACKLOG.md`](https://github.com/ms178/lccc/blob/main/engineering/agent/BACKLOG.md)

# Register Allocator
{:.doc-subtitle}
Production RA is a **policy-heavy linear scan** with hole-aware segment
interference, a Tier-2 graph colorer for the eligible subset, and allocator-
owned accumulator/ABI assignments.

**Canonical write-up:** [`engineering/subsystems/register-allocation.md`](https://github.com/ms178/lccc/blob/main/engineering/subsystems/register-allocation.md). Work items: [`engineering/agent/BACKLOG.md`](https://github.com/ms178/lccc/blob/main/engineering/agent/BACKLOG.md). Negative-results ledger: [`engineering/DECISIONS.md`](https://github.com/ms178/lccc/blob/main/engineering/DECISIONS.md). Measurements: [`engineering/evidence/workloads/gzip-zlib-expat.md`](https://github.com/ms178/lccc/blob/main/engineering/evidence/workloads/gzip-zlib-expat.md).

## Modules

| File | Role |
|------|------|
| `src/backend/regalloc.rs` | Policy: eligibility, copy/phi coalescing, call-arg waves, i686 hazards, XMM/NEON, loop-pin, segment-aware scan wiring |
| `src/backend/live_range.rs` | `LinearScanAllocator`, eviction modes, hints, PGO weights (`enable_splitting` is the RA-06 stub gate) |
| `src/backend/liveness.rs` | Worklist dataflow producing fat `intervals` **and** hole-aware `segments` |
| `src/backend/split_ranges.rs` | IR pre-pass: call-split, loop-transparent split (use-rewriting, phi-safe) |
| `src/backend/stack_layout/` | Homes for **unallocated** values; `graph_coloring.rs` colors stack slots (Tier 2) |

## Algorithm (short)

1. Optional `split_ranges` IR rewrite (fail-closed volatile allocas).
2. Liveness: envelopes + segments. **Interference is decided on hole-aware
   segments** (`segments_conflict`), with the fat model as fallback;
   coalesce leaders use the member union; empty segments fall back safely.
3. Copy-group coalescing (pairwise-disjoint) + phi latch coalescing.
4. **Phase 1** linear scan on callee-saved for spanning values, with
   hot-loop use-site depth seeding and precise `[start,end)` occupancy
   spans (`run_with_seed`).
5. **Phase 2** caller-saved in constraint waves (`call_arg_regs`, indirect
   target, i686 `%ecx`/`%edx`, then leftover), waves cannot alias.
6. **Phase 2f residual fill**: segment coloring fills holes in
   already-saved callee registers without eviction (default-on, ranked:
   multi-piece values first, pressure-gated).
7. **Tier-2 graph coloring** (production default, `CCC_NO_TIER2_GRAPH`)
   for the eligible subset; copy/phi/multi-def/asm webs stay on the scan.
8. i686 2d/2e; AArch64 loop-pin; XMM/NEON second class.

Default eviction is **mode 3** (hotter incoming, victim next-use after
incoming end). Mode 5 (exchange) regressed gzip as a default and stays
opt-in.

### Spill cost model

Two distinct currencies coexist, and the difference is deliberate:

| quantity | definition | used by |
|----------|------------|---------|
| `priority` | `(Σ_uses pgo_weight) × 10^min(max_loop_depth, 4)` | modes 1–3, worklist ordering |
| `spill_cost_at(pos)` | `(Σ_{use > pos} 10^min(depth(block(use)),4) × pgo(use)) × cost_boost` | mode 6 |

`priority` is a **global, single-scalar** estimate: the loop weight is the
maximum depth over the def block and all use sites, applied uniformly to
every use. A range with one inner-loop use is therefore priced like a range
with twenty. `spill_cost_at` is the **position-relative, per-use-frequency**
model that GCC (`REG_FREQ_FROM_BB`) and LLVM (`MachineBlockFrequencyInfo`)
both use: each use is weighted by its own block, and uses at or before the
scan position are excluded because they are sunk cost — they were already
served from a register.

Backing storage on `LiveRange`: `use_weights` (parallel to `uses`) and
`suffix_cost` (suffix sums, so the query is a binary search plus one index).
`cost_boost` is the policy multiplier for reads that are invisible in the IR
use chain (folded SIB indices, GEP bases folded into addressing, coalesce
leaders); `regalloc.rs`'s three `bump_*_priority` helpers maintain it
alongside `priority`.

**Compatibility contract.** When `use_weights` is empty — hand-built unit
test ranges, synthetic vector intervals, Phase-2b span ranges — every
accessor degrades to the unit-weight count, so `remaining_cost(pos)` equals
the historical `future_uses(pos)` pointwise. `set_uses` clears any stale
cost table so a replaced use list can never be mis-priced by a leftover
parallel array.

**Mode 6** keeps every soundness guard of mode 3 (steal-safety, never evict
an ABI-hinted value, victim's next use strictly past `incoming.end`) and
changes only the currency. It is opt-in pending a measured win; see
`engineering/FOLLOWUP-2026-09-05-regalloc-cost-model-and-torture.md` §2.1 for
the exact measurement plan and the guard-rails (gzip and the Adler DO8 loop
are where the previous eviction experiments died).

The position-relative order also subsumes the twice-reverted
"zero-future-use dead victim" experiments. Those failed because they bolted
a special case onto an unchanged *global* order; under a consistent
position-relative order a dead victim is simply the cheapest one, and the
profitability guard is expressed in the same currency as the ranking.

### Reserved registers

The ABI **static-chain** register (`%r10` on x86-64, `%ecx` on i686) is
removed from every allocatable pool in functions that contain a
`SetStaticChain`, because that instruction writes the register directly with
no IR dest — nothing else tells the scan it is redefined before the call.
Functions with only `GetStaticChain` are unaffected. See
`reserve_static_chain_reg` in `stack_layout/regalloc_helpers.rs`.

`CCC_VERIFY_REGALLOC=1` hard-verifies segment interference, final
assignments, and eviction occupancy history over a corpus.

## Interface (not frozen)

```rust
pub struct RegAllocConfig {
    pub available_regs: Vec<PhysReg>,
    pub caller_saved_regs: Vec<PhysReg>,
    pub call_arg_regs: Vec<PhysReg>,
    pub indirect_target_regs: Vec<PhysReg>,
    pub allow_inline_asm_regalloc: bool,
    pub xmm_regs: Vec<PhysReg>,
    pub never_materialized: FxHashSet<u32>,
    pub folded_index_uses: FxHashMap<u32, Vec<u32>>, // AArch64 only
}
```

## Open RA work (top of the catalog)

1. **RA-06 reload-at-next-use + in-place splitting** — the #1 gap (3–5×
   spill traffic); the scan still demotes whole lifetimes.
2. **RA-01/02** — gzip `longest_match` remat + IV homes.
3. **RA-01b** — marching-pointer recurrence homes (nbody stack refs).
4. **RA-03** — adler prefix-sum reassociation shape.

## Screening vs GCC (2026-08-25)

See the root `README.md` performance table for the canonical numbers;
kernel-level per-function counts come from `scripts/kernel_count.py` and
the Godbolt oracle (`scripts/godbolt.py`). gzip CRC is a **win** (0.86×);
adler (1.63×) is the tracked RA-06 gap.
