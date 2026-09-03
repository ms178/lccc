# Red-team audit of PR #359 (FP memory folds + constant hoist)

Audited body: `ab9c11a7`, merged upstream as `2146d775` (on main `da964ed4`,
the base of this audit branch). PR title: *"peephole: fold FP loads past
register redefinition, hoist repeated pool constants"*. Surface: three scalar-FP
memory folds with a relaxed liveness proof, a new constant-pool hoist pass, and
pass-scheduling wiring (`mod.rs`); the `vex.rs` delta is comment-only.

**Auditor's verdict: mostly correct, one real soundness hole.** The constant
hoist (`hoist_repeated_fp_constant_loads`) is sound as written. The FMA
folds (`fold_fma_memory_src2`, `fold_zero_addend_fma213_to_132`) and the scalar
fold (`fold_fp_register_loads`) share a **control-flow blind "pure overwrite
kills the loaded value" acceptance** that can delete a still-live load when a
branch sits between the consumer and the overwrite. Fixed in `722f7c3c`.
Claims of measured parity are corroborated (lccc dot8d SSE2 = gcc at 28
insns; -O3 x86-64-v3: 21–22 vs gcc 21 / icc 24; scalar-FP strictly better than
gcc 14.2's 32 at -O2 SSE2). Two minor efficiency gaps remain, not defects.

---

## What the PR does

1. **Liveness refinement for three fold passes.** Previously each fold vetoed
   whenever the loaded xmm was mentioned *anywhere later* in the function.
   The PR accepts a first later mention that is a **pure full-width overwrite**
   (`is_pure_xmm_overwrite`: mov-family destination or self-zeroing xor,
   checked so the destination register does not appear among the sources), on
   the theory that the overwrite kills the loaded value and every subsequent
   mention sees the new value.
2. **`hoist_repeated_fp_constant_loads`** — for leaf functions with ≥2 uses of
   the same `.LCFP_*` pool symbol in scalar-FP ops, materialize it once into a
   register that has zero textual mentions in the whole store, place the
   materialization in the latest dead NOP slot before the first use, and
   rewrite uses to the register. Vetoes: any call, any inline-asm region, any
   `blendv` line (implicit `%xmm0` read), no label between slot and last use
   (fall-through dominance), only exact `SYM(%rip)` tokens of a fixed FP-op
   list, multi-line slots and displaced forms skipped.

## Verdict 1 — the hoist: AGREE (sound; two efficiency notes)

Soundness argument holds under scrutiny:

- **Free-register selection** scans the whole store text (NOP'd slots
  included, since later passes may revive text), so a register chosen has zero
  mentions in any line that could execute. A leaf function may clobber any
  caller-saved xmm, and no implicit xmm reads exist outside `blendv` (vetoed),
  inline asm (vetoed) and calls (vetoed) — the implicit-operand space is
  closed.
- **Dominance** of the materialization over all rewritten uses is guaranteed by
  the no-label-in-`(slot, last_use]` rule *plus* the fact that any branch
  whose target lies inside that textual range would require a label there
  (vetoed) and any branch *originating* inside the range that exits it is
  harmless (uses after the branch are still reached only through the slot).
  Backward edges targeting a label before the slot re-execute the
  materialization — redundant but value-identical.
- **Value identity**: the rewrite copies the same pool bytes the memory
  operand read (rodata, immutable, no fault difference in a mapped leaf).
- **Bounds**: token scanner requires operand-boundary whitespace and pure
  `SYM(%rip)`; an in-place rmw mentioning the token twice (`vaddsd .LC0(%rip),
  %x2, %x2`) is handled by whole-word rewrite of every occurrence.

Efficiency notes (not defects):
1. The pass targets FP *arithmetic* pool operands, not `movsd .LCFP → reg`
   loads (movsd is absent from `FP_OPS`). The common `movsd .LCFP, %xmmK;
   vaddsd %xmmK, ...` pair is actually *folded to a memory operand* by the
   other passes first, so the hoist and the folds partially compete: after
   `fold_fp_register_loads` turns `movsd pool; vaddsd %reg` into
   `vaddsd pool(%rip), ...`, hoisting the pool operand back to a register
   would *add* an instruction. On the canonical dot8d SSE2 kernel the hoist
   therefore does not fire and the pool constant is read 4x as an rm operand —
   which is exactly 4 load-port reads of one cache line, while gcc's single
   `pxor` materialization is cheaper on port pressure. Worth a follow-up
   decision: prefer the materialization over rm-operand form when the pool
   symbol has >2 uses and a free register exists (net −3 load-ports at +0
   instructions for the 4-use case when one use is rewritten... measured
   below: current default equals gcc's instruction count, so this is a
   port-pressure refinement, not an instruction-count loss).
2. Loop-invariant *registers* from the hoist inside a loop body are re-
   materialized per iteration only when the slot is inside the loop (label
   rule), and `gpr_hoist`/phase-4 `fp_broadcast` may recover it later; no
   correctness issue.

## Verdict 2 — the fold liveness refinement: DISAGREE (unsound across branches)

**The hole.** All three scans (`fold_fp_register_loads` `later_mention`,
`fold_fma_memory_src2`'s `total == 2` acceptance, and
`fold_zero_addend_fma213_to_132`'s `ok_after`) accept a killing pure overwrite
based on **textual mention order alone**. Textual order is a liveness proof
only when it equals execution order. A branch between the consumer and the
"killing" overwrite lets a path skip the overwrite while a reader textually
past it still observes the *loaded* value at the merge:

```asm
    movsd  32(%rsi), %xmm5
    vfmadd231sd %xmm5, %xmm10, %xmm2    ; consumer (fold deletes the load)
    jne    .Ldone                       ; taken: %xmm5 STILL = loaded value
    vmovsd 40(%rdi), %xmm5              ; "pure overwrite" — fall-through only
.Ldone:
    vaddsd %xmm5, %xmm3, %xmm3          ; reads the loaded value on the jcc path
```

The pre-fix scan sees the `vmovsd` as the first later mention, classifies it a
pure overwrite, and deletes the defining `movsd` — leaving the `.Ldone` reader
with whatever was in `%xmm5`. This is the same *"text says dead, execution
says live"* class as the documented `cltq` implicit-read bug the relay folds
already guard against, so it is exactly the kind of trap the framework is
supposed to refuse.

**Why it has not fired on the corpus:** a correct register allocator will not
reuse `%xmm5` for the fall-through overwrite while the loaded value is still
live to `.Ldone` on the taken path, so real C kernels only rarely produce the
shape. But the pass proves nothing about the allocator; it states a
whole-function liveness theorem and must be self-contained. The hull-model
Tier-2 colorer and the MachInst width fix (this session's `b10ef6c0`) both
demonstrate how liveness-adjacent "obvious" invariants fail in this codebase;
a peephole must not rest on one.

**Fix (`722f7c3c`).** Accept the pure-overwrite kill only when no
control-flow transfer lies between the consumer and the killing mention.
The precise sufficient textual condition is a straight-line fall-through from
consumer to overwrite: then every later mention is dominated by the overwrite
and reads the new value. Implemented as `is_cf_transfer` (Label, CondJmp,
Jmp, JmpIndirect, Ret) checked over the intervening lines, applied identically
in all three passes. Calls are deliberately excluded from the barrier set — a
call clobbers every caller-saved xmm, so the loaded value cannot be observed
across it, and the passes keep their own stricter call veto for the xmm0–xmm7
variadic-argument window. Straight-line register-reuse chains (the dot8d SSE2
shape the PR's own tests pin) keep folding.

**Regression tests:** three diamond-shape unit tests
(`refuses_when_branch_skips_the_killing_overwrite`,
`refuses_when_branch_bypasses_the_killing_overwrite`,
`refuses_when_branch_between_fma_and_redefinition`) — each **FAILS on the
pre-fix logic and PASSES post-fix**, proving the misfold exists at the pass
level and is now refused.

## Verdict 3 — the measured-claim check: AGREE

| dot8d kernel (external arrays, FP-strict) | lccc | gcc | icc | clang | icx |
|---|---|---|---|---|---|
| -O2 SSE2 instructions | **28** | 29–32 (gcc 14.2/16.2) | — | — | — |
| -O3 -march=x86-64-v3 instructions | **21–22** | 21 | 24 | 16* | 10* |

\* clang/icx vectorize the four independent accumulators (vfmadd231pd over
lanes + lane tree), which is strict-legal here because each `s_k` is a separate
accumulator and the final tree matches the source. lccc has no FP auto-
vectorizer yet; within the scalar-FMA design space lccc is at the floor
(equal to gcc16.2, better than icc). A/B with `CCC_PEEPHOLE_SKIP=fp_reg_mem_fold`
confirms the fold refinement contributes the last instruction of the SSE2 win.

## Residual open items for a future session

1. Decide the port-pressure refinement: materialize a ≥3-use pool constant
   into a free register instead of re-reading it as an rm operand after the
   memory fold (Verdict 1 note 1) — measure on a dot8bench2-style driver.
2. lccc is scalar for FP reductions; the 16/10-insn clang/icx results are the
   vectorization gap, tracked separately from #359.
3. The `fma132` zero-elimination family (`eliminate_redundant_xmm0_zeroing`)
   is a block-local forward state machine; it was not part of #359 and was not
   re-audited here beyond reading (its label/call/branch/ret clearing is
   control-flow aware by construction).
