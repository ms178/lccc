# Register allocation

Production: `src/backend/regalloc.rs` (policy) + `src/backend/live_range.rs` (scan).
Stack homes: `src/backend/stack_layout/`; non-stack scalar homes use
`ExplicitLocation::{Reg,Accumulator}`, and register pointers resolve through
exact `SlotAddr::Reg(PhysReg)` across every backend. **Tier-2 hole-aware
graph coloring is production default** for the eligible subset
(`CCC_NO_TIER2_GRAPH` restores the scan-only path); copy/phi/multi-def/asm
webs stay on the scan by quarantine.

## Pipeline

1. `split_ranges` IR rewrite (call / loop-transparent; fail-closed volatile allocas).
2. Liveness: fat `[start,end]` and hole-aware `segments` (worklist dataflow).
3. **Interference is decided on segments** (`segments_conflict`) with the
   fat model as fallback; coalesce leaders use the member union; empty
   segments fall back safely.
4. Eligibility whitelist; `never_materialized`; hidden folded-index links.
5. Copy groups + phi latch coalescing; ABI physical hints never override
   `follow_value`. Safe call-free x86 CFG leaves with leading ParamRefs and
   ≤6 register arguments use ordered caller/ABI homes
   (`CCC_NO_LEAF_PARAM_GPR`, `CCC_NO_EMPTY_LOCAL_FRAME_ELISION`).
6. Call-spanning iff a call sits inside a live segment (inclusive-left
   boundary rule; merged-leader union).
7. Phase 1: callee-saved scan for spanning/hot-loop values (precise
   `[start,end)` occupancy seeds via `run_with_seed`).
8. Phase 2 caller-saved waves: argument/indirect exclusions, i686 hazards, rest.
9. Phase 2c: leftover unused callee-saved.
10. **Phase 2f residual fill**: no-eviction segment coloring of holes in
    already-saved callee registers, default-on, ranked multi-piece-first
    with a pressure gate (`CCC_NO_SEGMENT_FILL`).
11. i686 load-hazard refinement, EAX homes.
12. AArch64 loop-pin; XMM/NEON second class.
13. Hard verification (`CCC_VERIFY_REGALLOC`): final segment assignments
    plus `handled` physical-register occupancy history and eviction cut
    points (half-open `[start, cut)` for evicted ranges).

Default eviction mode 3. Mode 5 lost gzip and stays opt-in.
`enable_splitting` is retained as the RA-06 gate; in-scan reload-at-next-use
remains the top open item.

**Mode 6 (opt-in): position-relative cost.** Modes 1–3 rank victims by the
global `priority` = `(Σ_uses pgo_weight) × 10^min(max_loop_depth, 4)` — one
scalar per range, so a single inner-loop use prices a range like twenty, and
uses already behind the scan point still count. Mode 6 ranks by
`LiveRange::spill_cost_at(incoming.start)`: per-use block frequency
(`use_weights` / `suffix_cost`, the `MachineBlockFrequencyInfo` analogue),
summed only over uses **strictly after** the scan point, times `cost_boost`
(the policy multiplier the three `bump_*_priority` helpers maintain for reads
invisible in the IR use chain). Same soundness guards as mode 3; only the
currency changes. Ranges with no attached `use_weights` degrade exactly to
the historical unit-weight `future_uses` count, so unenriched scans are
bit-identical.

The position-relative order subsumes the twice-reverted "zero-future-use
dead victim" experiments (see [`../DECISIONS.md`](../DECISIONS.md)): those
bolted a special case onto an unchanged global order, whereas here a dead
victim is simply the cheapest one under one consistent order.

`RegAllocConfig` includes call/indirect exclusions, XMM regs, folded index
uses, and ABI physical hints.

PhysReg **(11) = %r10** (static chain), (10) = %r11.

The static-chain register is **removed from every allocatable pool** in a
function containing `SetStaticChain` (`reserve_static_chain_reg` in
`stack_layout/regalloc_helpers.rs`): that instruction writes `%r10` / `%ecx`
directly with no IR dest, so a value homed there and live across the nested
call was silently clobbered (gcc.c-torture `920501-7.c`). `GetStaticChain`
alone is not a trigger.

## Open quality gaps

RA-06 reload-at-next-use + arithmetic-chain copy webs (adler 1.63× — the
naive leader promotion measured +28 % and was reverted, see
[`../DECISIONS.md`](../DECISIONS.md)); gzip remat/next-use (RA-01/02);
marching-pointer homes (RA-01b); segment-based splitting for vecreg;
residual Briggs on spilled (RA-09).

## Kill switches / diagnostics

`CCC_NO_COALESCE`, `CCC_NO_PHI_COALESCE`, `CCC_NO_LEAF_PARAM_GPR`,
`CCC_NO_EAX_ALLOC`, `CCC_NO_SEGMENT_FILL`, `CCC_NO_INDEX_HOME`,
`CCC_NO_ABI_REG_HINTS`, `CCC_NO_VECREG`, `CCC_EVICT_MODE`,
`CCC_PGO_WEIGHT_MAX`, `CCC_NO_TIER2_GRAPH`, `CCC_RA_EXPLAIN`,
`CCC_TRACE_ALLOCSTATS[=filter]`, `CCC_DEBUG_SEGMENT_FILL`,
`CCC_VERIFY_REGALLOC`.
