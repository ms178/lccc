# Register allocation

Production: `src/backend/regalloc.rs` (policy) + `src/backend/live_range.rs` (scan).  
Stack homes: `src/backend/stack_layout/` (`graph_coloring.rs` colors **slots**, not GPRs).

## Pipeline

1. `split_ranges` IR rewrite (call / loop-transparent; fail-closed volatile allocas).
2. Liveness: fat `[start,end]` **and** hole-aware `segments`.
3. Eligibility whitelist; `never_materialized`; call-arg / r10 exclusions.
4. Copy-group coalescing (pairwise-disjoint) + phi latch coalescing.
5. Call-spanning iff a call sits **inside a live segment**.
6. Phase 1: callee-saved linear scan for spanning values.
7. Phase 2: caller-saved **waves** (`run_with_seed`): arg regs, indirect target, i686 ecx/edx, rest.
8. Phase 2c: leftover unused callee-saved.
9. i686 2d/2e; AArch64 loop-pin; XMM/NEON second class.

Default eviction **mode 3**. Mode 5 lost gzip. `enable_splitting` is unused; there is no in-scan reload.

`RegAllocConfig` includes `call_arg_regs`, `indirect_target_regs`, `xmm_regs`, `folded_index_uses` (AArch64 only).

PhysReg **(11) = %r10**, (10) = %r11.

## Open quality gaps

Remat of file-scope arrays; next-use (Adler `sum2`); segment scan (xmltok/inflate); second-chance; affinity vs disjoint merge. Catalog: [`../agent/BACKLOG.md`](../agent/BACKLOG.md) items **RA-01 … RA-22** (part of the 150).

## Kill-switches

`CCC_NO_COALESCE`, `CCC_NO_PHI_COALESCE`, `CCC_NO_LEAF_PARAM_GPR`, `CCC_NO_EAX_ALLOC`, `CCC_NO_VECREG`, `CCC_EVICT_MODE`, `CCC_PGO_WEIGHT_MAX` (default 1), `CCC_DEBUG_RA`.
