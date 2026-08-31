# SCCP adoption audit

`src/passes/sccp.rs` (Wegman–Zadeck sparse conditional constant propagation) and
`src/passes/use_def.rs` (CSR use-def chains) were adapted from
[`CrazyTodd-one/claudes-c-compiler`](https://github.com/CrazyTodd-one/claudes-c-compiler)
(HEAD `561a1f1`), which had optimizer machinery lccc lacked.

The upstream implementation was **not** adopted as-is. It targets a different IR, and
several of its assumptions are wrong for lccc's — in ways that produce miscompiles
rather than missed optimizations. This document records every finding, the fix, and
the test that pins it. Test names refer to `src/passes/sccp/tests.rs` unless noted.

Findings are numbered as they were raised during the audit; the gaps in the sequence
are checks that turned out to be non-issues, recorded in §3 so they are not
re-investigated.

---

## 1. Defects found and fixed

### F20 — `asm goto` edges are invisible to the solver (CRITICAL, lccc-only)

`Instruction::InlineAsm` (`src/ir/instruction.rs:434`) carries
`goto_labels: Vec<(String, BlockId)>`. These are **real CFG edges**, and
`cfg_simplify::build_pred_info` (`:100`) honours them. The upstream solver derives
edges from terminators only, so a block reachable *solely* via `asm goto` is deemed
unreachable: its phi entries are pruned and its code is left unconstrained. The Linux
kernel workload hits this directly.

Fixed in both places that enumerate edges — `visit_block` in the solver, and the
rewrite's live-edge computation. Test: `asm_goto_targets_stay_reachable`.

### F2 / F8 — folded branches must prune the not-taken block's phi entries

`cfg_simplify::fold_constant_cond_branches` (`:264`+) calls
`remove_phi_entries_from(not_taken, cur_label)` (helper at `:128`) and its comment
states that leaving them behind "can cause miscompilation when the not-taken block is
still reachable from other paths." Upstream omits this for `CondBranch` **and**
`Switch`.

Deferring the repair to a later `cfg_simplify` run is not a fix: malformed IR exists
between the two passes, and anything scheduled in between will consume it. Tests:
`folding_a_cond_branch_prunes_the_stale_phi_operand`,
`folding_a_switch_prunes_every_stale_phi_operand`,
`operands_from_unreachable_predecessors_are_pruned`.

### F1 — an open `default` arm fabricates constants

Upstream leaves unmodelled dest-producing opcodes at `Top`. Since `⊤ ⊓ C = C`, a phi
merging a modelled constant with an unmodelled definition yields that constant — a
value the program never computes. The lattice must be *pessimistic* about anything it
cannot model.

Replaced with a closed catch-all keyed on `Instruction::dest()`: any instruction that
defines a value and is not explicitly modelled lowers its dest to `Overdefined`.
Tests: `an_unmodelled_definition_is_overdefined_not_top`,
`a_call_result_is_overdefined_and_the_call_is_never_deleted`.

### F3 — `is_nonzero` disagreed with `cfg_simplify` on `long double`

Upstream's branch-condition test ends in `_ => true`, so a zero `LongDouble` is
treated as non-zero. `cfg_simplify` uses the canonical `IrConst::is_nonzero()`
(`src/ir/constants.rs:252`), which handles `LongDouble(0.0, _)` correctly. Two passes
folding the *same* branch in opposite directions is a miscompile waiting for the
scheduler to pick the wrong order.

SCCP now uses the canonical predicate. Tests:
`a_zero_long_double_condition_takes_the_false_edge`,
`a_nonzero_long_double_condition_takes_the_true_edge`.

### F4 — `long double` folded through `f64` (severe divergent oracle)

Upstream funnels everything satisfying `ty.is_float()` — which includes F128 — through
`f64`, silently truncating to a 53-bit mantissa. lccc has exact folding:
`fold_f128_binop` (x87 80-bit on x86 via `common::long_double`, binary128 elsewhere),
`fold_f128_neg`, `fold_f128_cmp`.

Fixed structurally rather than locally: `constant_fold.rs` grew a shared folding
oracle, and SCCP delegates to it, so the two passes cannot drift again. Test:
`long_double_folding_matches_the_shared_oracle_bit_for_bit`, which compares both
`to_hash_key()` and `long_double_bytes()`.

### F23 — `IsConstant` is phase-dependent and must never resolve negatively early

lccc's folder returns `None` for `IsConstant` when the operand is not *yet* constant,
and `resolve_remaining_is_constant` (`constant_fold.rs:181`) forces the `0` case once,
at the end of the pipeline. Upstream maps `Overdefined → Constant(0)` mid-pipeline,
which permanently pessimizes every `__builtin_constant_p` gate that a later pass would
have made constant.

Decision: `IsConstant(Constant(_)) → Constant(1)`; `⊤ → ⊤`; `⊥ → ⊥`. SCCP never
materializes the `0`. This is monotone and leaves the end-of-pipeline resolution as the
single authority. Test: `builtin_constant_p_resolves_positively_but_never_negatively`.

---

## 2. Hardening added during adoption

### Prune only edges that were *proved* dead

SCCP distinguishes "this edge exists and I proved it non-executable" (prunable) from
"this label was never a predecessor of this block" (malformed IR — keep it, and let
`cfg_simplify`'s stale-label cleanup deal with it). A `real_edges` set of all CFG edges
as written — including `asm goto`, regardless of reachability — is built alongside
`live_edges`, and the predicate is
`live_edges.contains(k) || !real_edges.contains(k)`.

This was added after a `loop_rotate` defect emitted a phi naming a non-predecessor and
SCCP dutifully deleted an induction variable's initialisation. The root cause is fixed,
but malformed input from any *other* pass is now safe too. See
`engineering/FOLLOWUP-2026-08-31-sccp-loop-rotate-ir-verifier.md`, and
`src/passes/verify.rs` for the verifier that now catches the whole class.

### Other invariants worth stating

* **Seeding.** `ParamRef` → `⊥`; every value with `use_count > 0` and no definition
  → `⊥`. Dangling references must not be absorbed by a phi meet.
* **Substitution goes through `for_each_operand_mut` only** — never bare `Value`
  fields (`Load.ptr`, `GetElementPtr.base`, `Memcpy`, `InlineAsm` outputs,
  `Intrinsic.dest_ptr`), which cannot legally hold a constant. Precedent:
  `copy_prop::replace_operands_in_instruction` (`:539`–`:720`).
* **Phi contiguity.** Constant phis are removed and their `Copy` spliced in *after*
  the phi prefix: `loop_rotate.rs:412` requires phis contiguous at block start.
* **`Select` monotonicity.** `update_lattice` meets with the old value, so a `Select`
  whose condition goes `C → ⊥` cannot move back up the lattice.
* SCCP is **idempotent** — verified by test.

---

## 3. Checked and found to be non-issues

Recorded so they are not re-audited:

* **F5** — `IrConst::to_i64()` returning `None` for floats is intended, not a gap.
* **F24** — `IrCmpOp::eval_f64` unordered/NaN semantics are correct.
* **F25** — `-0.0 == 0.0` folding to true is correct.
* **F26** — folding `f32` arithmetic via `f64` is safe here: single rounding is
  guaranteed because `53 ≥ 2·24 + 2`.

---

## 4. Debugging

SCCP performs four independent rewrites, each separately suppressible, so bisecting a
miscompile costs one rebuild instead of a bisect over the pass:

| Variable | Suppresses |
|---|---|
| `CCC_SCCP_NO_PRUNE` | pruning phi operands on provably-dead edges |
| `CCC_SCCP_NO_DEFS` | rewriting definitions to `Copy { dest, Const }` |
| `CCC_SCCP_NO_SUBST` | substituting constants into operands |
| `CCC_SCCP_NO_FOLD` | folding `CondBranch`/`Switch` into `Branch` |
| `CCC_SCCP_TRACE_PRUNE` | prints every pruned phi operand and its edge verdict |

Whole-pass kill switch: `CCC_DISABLE_PASSES=sccp`.

---

## 5. Where SCCP runs

Integrated without adding a twelfth phase index — `NUM_PASSES` stays 11, so no
`should_run!` index renumbering was needed:

* **Main loop, phase 4**, immediately before `constant_fold`, sharing that phase's
  slot and `should_run!(4, 0,1,2,3,7,8)` predicate. Gated `opt_level >= 2`, which
  includes `-Os`/`-Oz` where it is a size win.
* **`run_inline_phase`**, immediately before the cleanup `cfg_simplify`. Inlining has
  just substituted literal arguments, so parameter-dependent branches become
  decidable; `cfg_simplify` alone cannot do this, as it only folds an *already*
  constant condition.
