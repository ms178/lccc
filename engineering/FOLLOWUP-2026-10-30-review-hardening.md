# Review hardening: shared contracts and the honest scope of this change

This change lands four contract simplifications and one new coalescing shape.
It does **not** carry a reducer rewrite: the strided-32-bit reducer keeps its
`VectorSchedule` / `build_preheader` / `emit_vector_body` / `emit_reduce` split
and its `fn transform(...) -> Option<usize>` signature, complete with the
decline-before-mutate discipline described below. Anything a review of the
wider work suggested that is not visible in the tree is out of scope here.

## What this commit lands

* **One coalescing topology.** `phi_update_region` is the single definition of a
  destructive-update window, consumed by both the liveness gate and the
  caller-saved clobber window. It adds exactly one shape to the pre-existing
  same-block and single-predecessor-chain forms: the split-latch diamond
  `source -> {middle, copy}` with `middle -> copy`, exact predecessor and
  successor counts, and no outside entry.
* **One clobber walk.** The window walk visits the middle arm even when the
  source has a direct edge to the copy, because a call there destroys a
  caller-saved home just as effectively.
* **One downcount recurrence scan.** The loop-inversion downcount matches the
  guard as a one-instruction slice instead of a constant instruction index, and
  locates `iv` / `next` with `sole_def` inside their home blocks. The global
  validation scan that was already mandatory now also rejects any extra
  definition, so two full-function definition walks disappear without weakening
  a proof.
* **One SIMD location contract.** `has_vector_location_contract` derives the
  contract from central `vector_result_width` metadata plus the explicit
  scalar-result `HorizontalAdd*` exceptions, replacing a hand-maintained list
  that had already drifted (it missed all three strided loads).
* **One implicit-write event stream.** The core and allocating MachInst
  last-write classifiers consume the same `for_each_implicit_write` callback.
  Callback order is the contract: call clobbers first, then the typed return
  move.

## Declining before mutating (existing property, kept)

`transform` in the strided reducer performs no IR mutation until every plan
buffer is built and every scalar-header phi target is validated. It returns
`None` (leaving the scalar function untouched) when the schedule, the resume
constant, or any emit stage declines; the commit point is reached only after
all of those proofs succeed. The `debug_assert!` that keeps the compact-load
invariant loud in test builds is deliberately preserved.

## Classifications stay separate on purpose

Similar-looking intrinsic lists describe different contracts.
`may_read_memory`, `produces_vector_value`, `vector_result_width`, the alias
footprints, and the vec-SSA / raw-reader sets remain distinct. Collapsing them
into one memory or SIMD "super-list" would be wrong even though it would read
more tidily.

The two genuine duplicates that were merged are the ones with a shared meaning:

* the register-allocation helper now consumes central result metadata with only
  the explicit scalar-result `HorizontalAdd*` exceptions kept as a list, so a
  newly classified SIMD producer cannot be forgotten the way the AVX2 strided
  load was;
* the two MachInst last-write classifiers now share one iterator for implicit
  writes, with a parity test that pins both consumers.

## Scope of the widening

The metadata predicate covers roughly nine times as many intrinsics as the old
hand list. That is intentional and narrow in effect: it only feeds the opt-in
scalar-FP stack experiment (`CCC_DISABLE_SCALAR_FP_XMM`), it can only make the
XMM protection *stronger*, and the default register-allocation configuration
never enables that experiment. A test preserves every former positive, the three
strided loads, and store/scalar negative controls.

## Reachability of the diamond, measured

The latch copy of a loop-carried value is a rename of a value that is already
available, so this pipeline schedules it into the **same block** as its source
definition. Every C shape probed (`if` with a store arm, `if` with a call arm,
value defined by a two-way merge, stores and calls combined) produced
`source block == copy block` and therefore took the pre-existing same-block
path.

Consequence for reviewers: the split-latch diamond is a correctness-neutral
capability that no current C input reaches. It is covered by unit tests that
construct the CFG directly (`phi_coalesce_applies_across_split_latch_diamond`,
`phi_coalesce_refuses_caller_saved_home_over_diamond_middle_call`,
`split_latch_diamond_*`), not by the execution fixture. No runtime improvement
is claimed for it. If a future pass starts emitting the shape, the machinery is
already proven; until then it costs one cheap topology test per candidate.

## Gate coverage

The focused strided-reduce and saturating-subtract gates now cover `-O1` and
`-Os` on `x86-64` and `x86-64-v3`. The dedicated reducer is intentionally
inactive at those levels, so the gates assert correct scalar fallback rather
than a packed shape, and `as --64 -march=generic64` provides an independent
baseline-ISA oracle instead of an incomplete opcode denylist.
