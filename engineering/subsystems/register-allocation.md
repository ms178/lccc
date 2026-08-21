# Register allocation

Production: `src/backend/regalloc.rs` (policy) + `src/backend/live_range.rs` (scan).
Stack homes: `src/backend/stack_layout/`; its hole-aware graph colorer is
research-wired but default-off until RA-23 exposes accumulator lifetimes.

## Pipeline

1. `split_ranges` IR rewrite (call / loop-transparent; fail-closed volatile allocas).
2. Liveness: fat `[start,end]` and hole-aware `segments`.
3. Eligibility whitelist; `never_materialized`; hidden folded-index links.
4. Copy groups + phi latch coalescing; ABI physical hints never override `follow_value`. Safe call-free x86 CFG leaves with leading ParamRefs and ≤6 register arguments use ordered caller/ABI homes.
5. Call-spanning iff a call sits inside a live segment.
6. Phase 1: callee-saved scan for spanning/hot-loop values.
7. Phase 2 caller-saved waves: argument/indirect exclusions, i686 hazards, rest.
8. Phase 2c: leftover unused callee-saved.
9. i686 load-hazard refinement, EAX homes, and no-eviction residual segment fill.
10. AArch64 loop-pin; XMM/NEON second class.
11. Optional hard verification: final segment assignments plus `handled`
    physical-register occupancy history and eviction cut points.

Default eviction mode 3. Mode 5 lost gzip. `enable_splitting` is retained as
the RA-06 gate; in-scan reload-at-next-use remains open.

`RegAllocConfig` includes call/indirect exclusions, XMM regs, folded index uses,
and ABI physical hints. Safe byte-mask index homes are weighted for indexed
addressing; broad hidden homes remain blocked on RA-23.

PhysReg **(11) = %r10**, (10) = %r11.

## Open quality gaps

RA-23 explicit accumulator locations; RA-24 `SlotAddr::Reg`; true split scan
and reload-at-next-use; gzip rematerialization/next-use; full cross-backend
segment fill after the location contract is unified.

## Kill switches / diagnostics

`CCC_NO_COALESCE`, `CCC_NO_PHI_COALESCE`, `CCC_NO_LEAF_PARAM_GPR`,
`CCC_NO_EAX_ALLOC`, `CCC_NO_SEGMENT_FILL`, `CCC_NO_INDEX_HOME`,
`CCC_NO_ABI_REG_HINTS`, `CCC_NO_VECREG`, `CCC_EVICT_MODE`,
`CCC_PGO_WEIGHT_MAX`, `CCC_RA_EXPLAIN`, `CCC_TRACE_ALLOCSTATS[=filter]`,
`CCC_DEBUG_SEGMENT_FILL`, `CCC_VERIFY_REGALLOC`.
