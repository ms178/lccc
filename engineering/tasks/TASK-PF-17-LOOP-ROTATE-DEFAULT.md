# TASK-PF-17 — root-cause loop-rotation miscompiles, then default-enable

IDs: PF-17, v15 roadmap item 1–2 · Priority: **P0** · Base: f657de55
· Rotation is the systemic gap: every inner loop pays a double-jump
preheader (~1 branch/iteration across the whole suite).

## Objective

The pass is correctness-clean for the canonical counted-loop shape and
ships opt-in (`CCC_LOOP_ROTATE=1`). Root-cause the 15 remaining
default-enable miscompile shapes, harden, then flip the default to ON at
-O2+.

Known failing shapes (from the v16/v17 default-enable experiments; full
list with the v14 root-cause pattern in DECISIONS.md):
vectorize_sse2_path, vectorize_reduction_dyn, simd_crc_adler, simd_vecreg,
backedge_pre_*, bitops_builtins, adler_inline_tail,
aggregate_dse_soundness, alloca_bare_builtin, alu_peepholes,
arm_vec_load_offset, huft_build_crash, loop_promote_affine_alias,
stmt_expr_asm_typeof, vectorize_iv_dependent_base.

Working hypothesis: exit-merge-phi off-by-one for cross-phi latch operands
used externally (the v14 class), plus cloned-closure header-phi reference
collapse.

## Files

`src/passes/loop_rotate.rs`, `src/ir/mem2reg/phi_eliminate.rs` (self-loop
copy placement), `src/passes/mod.rs` (pass order: rotation stays AFTER
vectorize).

## Acceptance

- Full regression corpus green (current baseline 474 PASS / 0 FAIL /
  11 SKIP) with rotation default-ON.
- The 9-worst-benchmark suite unchanged or better; loop_patterns drops the
  `addl; movslq` double-jump preheader shape.
- `sum_arr` and `loop_patterns` outputs bit-identical to GCC.
- Re-land the phi-elim self-loop copy-placement fix with the narrower gate
  (only rotation-created self-loops — the broad version regressed
  sqlite_varint and was reverted).

## Validation battery

`cargo test --lib` · full regression corpus with `CCC_LOOP_ROTATE=1` AND
default-on builds · 1200 differential fuzz across O0/O2/O3/Os · 360 phi-CFG
fuzz · kernel corpus 15/15 · nbody 5M-step bit-identical to GCC.

## Do not

- Do not run rotation before vectorize (v14 SIGSEGV: rotated forms corrupt
  vectorizer base-dependence analysis).
- Do not rotate bodies containing Call/CallIndirect, volatile loads/stores,
  or intrinsics (current guards — keep them).
- Do not trampoline the self-loop phi copy (it splits the rotation back
  into body+latch; see DECISIONS.md v13 entry).
