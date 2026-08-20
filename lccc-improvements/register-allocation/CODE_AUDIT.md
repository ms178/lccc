# Code audit — register allocation (2026-08-20)

Scope: `src/backend/{regalloc,live_range,liveness,split_ranges}.rs` and
`src/backend/stack_layout/{mod,graph_coloring,copy_coalescing,regalloc_helpers,slot_assignment}.rs`.

Method: full read of live_range.rs; structured read of allocate_registers
and helpers; header+API of liveness/split_ranges/stack_layout.

## live_range.rs — quality: high

Strengths: documented invariants, steal-safety, cascade, rotation
restriction, unit tests that encode measured miscompiles, saturating
arithmetic, idempotent `run`.

Issues:

- `find_evict_candidate` public API has **no steal filter** unless
  `incoming` passed; easy to misuse.
- `exchange_eviction` field is never set in the `new()` path from
  `regalloc.rs` Phase 2c (comment says Phase 2c sets it; **code does
  not** — 2c uses default mode 3 unless env). Doc/code drift.
- `handled` grows without bound for compile-time; fine for functions,
  not for pathological IR.
- `available_regs.contains` is O(n) per hint.

## regalloc.rs — quality: high policy, high complexity

~3600 lines, many waves. Correctness comments are excellent (printf
clobber, r10 fptr, nbody GlobalAddr, adler32, sqlite deleteTable).

Issues:

- **Complexity budget.** Four Phase-2 waves × rebuild live ranges each
  time. Compile-time: `build_live_ranges` walks the whole function per
  wave. Cache metadata.
- `exclude_every_third_mul_temp` — unexplained magic; document or kill.
- `x86_ordered_param_copies` heuristic (single block + rbx + r10 present)
  is a fingerprint of “this is x86-64”, not a real leaf test.
- XMM vecreg function is huge; belongs in its own module.
- i686 2d/2e should live in `i686/` so x86-64 compile does not carry them
  mentally.
- `collect_vecreg_candidates` fail-closed is good; missing AVX2 256-bit
  homes (YMM) except as poison.

## liveness.rs — quality: high

Hole-aware segments, setjmp, f128, GEP. Dense bitsets.

Issues:

- Loop depth from back-edges only; irreducible CFG / multi-entry may
  under-weight.
- Segment builder vs interval envelope can disagree; RA mixes both
  (span check on segments, scan on envelopes).

## split_ranges.rs — quality: careful, timid

Volatile allocas, no mem2reg, phi-first, fail-closed. Good.

Gaps: no cross-block post-call phis; unique-exit loops only; `max_splits`
cap. This is the **largest unimplemented half of the March “splitting”
promise**.

## stack_layout/graph_coloring.rs

Name oversells: greedy interval coloring with `end <= start` reuse, **not**
Chaitin. Non-8-byte sizes never share. Diamond CFG still depends on
liveness holes; if intervals are fat, coloring cannot share.

## Inefficiencies

1. Repeated `build_live_ranges` full IR walks (Phase 1, 2×i686, 4 waves,
   2c, 2d, 2e, XMM).
2. `FxHashSet` clones of param_restricted / eligible.
3. `register_compatible` scans all `active` for a physreg.
4. Copy-group union-find rebuilt without path compression stats.

## Suggested mechanical refactors (low risk)

1. `regalloc/xmm.rs`, `regalloc/i686.rs`, `regalloc/coalesce.rs`.
2. One `RangeMetadata` per function, reused across waves.
3. Set `exchange_eviction = true` on 2c **or** delete the comment.
4. Teach `verify_no_overlap` about die-at-birth.
5. Rename `graph_coloring.rs` → `slot_interval_color.rs`.

Do **not** refactor and change heuristics in the same patch.
