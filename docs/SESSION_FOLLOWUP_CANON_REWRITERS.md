# Canonical value-rewrite walkers: slot coverage is not reach semantics

**Status:** landed with the consolidation of the eleven private rewriter tables (PR #701 follow-up).
**Audience:** anyone replacing a `match` table with `replace_operand_value` /
`replace_values_in_inst_map` / `replace_terminator_value`.

---

## 1. What the canonical pair guarantees

[`replace_operand_value`](../src/ir/instruction.rs) and its map-valued companion redirect **every
read** of a value: the `Operand` slots (`for_each_operand_mut`) *and* the bare `Value` slots that are
not `Operand`s (`GetElementPtr.base`, `Load.ptr`, `Store.ptr`, `Memcpy`, va-list pointers,
`Intrinsic.dest_ptr`, `InlineAsm` outputs, …) via `for_each_value_use_mut`.  Destinations — the
values an instruction *defines* — are never touched.

Historic hand-rolled tables enumerated *instruction kinds* and silently missed whole positions, which
is how two `VaEnd { .. } => {}` arms survived (a rewrite that redirected a `va_list` pointer left the
old value in place: SSA-valid, silently wrong).  `ir::instruction::value_replacement_tests` pins the
coverage both forms must have in common.

## 2. What the canonical pair does **not** guarantee: reach

A table also encodes **where** a rewrite is applied.  For a blind "redirect every read of `old` to
`replacement`" that reach matters, and two shapes are illegal:

1. **the instruction that defines `replacement`** — the vectorizer's no-remainder reduction is
   `VecHorizontalAddI32x8(acc) -> sum`: it consumes the accumulator *and* defines the replacement.
   Redirecting its operand produces `hadd(sum) -> sum`; the loop feeding it loses its only outside
   reader, is eliminated, and the backend reads an uninitialised spill slot as a pointer
   (`double_reduction` printed `0` instead of `201415680`, then faulted);
2. **any instruction whose result feeds `replacement`** — the remainder path seeds its scalar
   accumulator phi with `hadd(acc) -> 29`, so redirecting the seed closes the cycle
   `29 -> phi(41) -> 42 -> 29` from the other side; the vector loop again disappears
   (`vectorize_i32_sum` returned garbage for `n = 1`).

Both are the same defect: *the rewrite puts the replacement behind itself*.  The general rule is a
data-dependency question, not an instruction-kind question:

```rust
let (def_block, def_idx) = definition_site(func, replacement)?;   // what defines it
let producers = producer_cone(func, replacement);                 // transitive backward slice
for each read of `old` outside the loop in block bi, instruction ii {
    if bi == def_block && ii <= def_idx { continue; }   // not yet defined here
    if inst.dest() ∈ producers { continue; }            // feeds it -> cycle
    redirect;
}
```

`vectorize::rewrite_uses_where_available` is the implementation, `outside_rewrite_availability_tests`
is the regression suite (both shapes, the same-block cursor, and the bare-`Value` coverage that the
migration exists for).

**Block reach is deliberately broad.**  The no-remainder path *must* redirect readers in the original
exit block, whose zero-trip bypass edge exists in the CFG but is proven dead by the trip-count guard.
A dominance filter ("rewrite only where the definition dominates the use") was implemented, measured,
and rejected: it left those readers on the vector accumulator and `double_reduction` printed
`25051136`.  Reach is part of a rewriter's contract; state it and test it, do not infer it from the
instruction kinds a table happened to list.

**Availability guards are the complement, not a replacement.**  *Reach* is the policy of the
no-remainder rewriter above, whose readers must be redirected even off the zero-trip path.  The
outside-loop rewriter in `vectorize` answers a different question — the new block it inserts
executes unconditionally on the fall-through path, so a redirected reader must see its producer
there — and therefore *does* prove availability before rewriting, from the CFG **after** the pass
inserted its blocks:

* each use is classified (`UseSite::{Inst, Terminator, PhiArm}`); a phi incoming is a use at the end
  of its **predecessor** block, mirroring verifier check 8 exactly;
* a def must exist in the rewritten body, its block must be reachable, and it must dominate the use
  (same-block: textual order decides) — the semantics of `ir::analysis::dominates_idom`, which is
  verifier-identical (`unreachable`/malformed/self-cycles are *not* dominance);
* `CCC_DEBUG_VEC_AVAIL=1` prints `[VEC-AVAIL] fn=... redirected=N refused_unavailable=M`, so a guard
  that refuses everything is visible instead of silent.

Both policies now live behind one table-free walk; the difference is the caller's `Availability`,
which is why the guard is a parameter and not an instruction-kind list.

## 3. Verification used when landing the consolidation

* `scripts/check_codegen_refactor_identity.py --before <baseline> --after target/fastbuild/lccc
  --corpus tests/regression` → `IDENTICAL=791 PGO_PID=4 DIFFERENT=28 COMPILE_FAIL=0 OF 823`
  (the 4 are the documented `-fprofile-generate` PID nondeterminism; the 28 are the intended
  canonicalisation deltas), and the same check over `tests/benchmark/programs` → `IDENTICAL=47
  DIFFERENT=4 OF 51`.
* **Assembly identity is not enough for a codegen migration** — the differing set is executed.
  All 36 fixtures whose assembly differs (28 regression + 4 benchmark + 4 shared names) print
  byte-identical stdout/stderr with exit 0 against the baseline on both binaries; one earlier
  revision of the `if_convert` fix rematerialised `ParamRef` and this step is what caught it
  (`bb_slp_nested_ifconv` rc=139, valgrind: invalid write through an unmaterialised parameter).
* `CCC_VERIFY_IR=1` over all 862 corpus sources → **0 availability violations** (one before the fix:
  `tests/regression/minmax_refused.c`, a `GlobalAddr` reused across a 4-incoming join).
* Per-pass IR dumps (`CCC_DUMP_EACH_PASS=1 CCC_DUMP_FUNC=<fn>`) and pass isolation
  (`CCC_DISABLE_PASSES=<pass>`) localise a codegen delta to a pass without rebuilding.
* The unit suite alone does **not** exercise these shapes: the corpus does.  Run the identity check
  before landing a rewriter migration.

## 4. No tables remain

`split_ranges` was the last duplicate: its 135-line `replace_values_in_inst_phi_aware` /
`replace_values_in_terminator_phi_aware` tables were deleted and now delegate to the canonical
`replace_values_in_inst_map` / `replace_values_in_terminator_map`.  The only caller-level difference
is the `rewrite_phi` flag — a consumer that must *suppress* phi-incoming rewrites — so the flag is a
parameter of the delegating wrapper, not a hand-maintained copy of the walk.  `redundant_loads` is
on the canonical walk for terminators too (an `IndirectBranch` whose target value was merged must
follow the rename, and a unit test pins that).
