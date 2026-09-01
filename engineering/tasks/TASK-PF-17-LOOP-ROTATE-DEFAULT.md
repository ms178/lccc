# TASK-PF-17 — root-cause loop-rotation miscompiles, then default-enable

IDs: PF-17, v15 roadmap item 1–2 · Priority: **P0** · Base: f657de55
· Rotation is the systemic gap: every inner loop pays a double-jump
preheader (~1 branch/iteration across the whole suite).

## Objective

The pass is correctness-clean for the canonical counted-loop shape and
ships opt-in (`CCC_LOOP_ROTATE=1`). The 15 v16/v17 default-enable
miscompile shapes MATCH GCC on the 21-name A/B (PRs #325 pred-label,
#327 remaining-three). Flip the default to ON at -O2+ only after the
full 474-test corpus is green.

Historical failing shapes (all MATCH as of `453cbea` + post-merge
audit; full v14 pattern in DECISIONS.md):
vectorize_sse2_path, vectorize_reduction_dyn, simd_crc_adler, simd_vecreg,
backedge_pre_*, bitops_builtins, adler_inline_tail,
aggregate_dse_soundness, alloca_bare_builtin, alu_peepholes,
arm_vec_load_offset, huft_build_crash, loop_promote_affine_alias,
stmt_expr_asm_typeof, vectorize_iv_dependent_base.

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
- Do not drop Guard C (multi-preheader header phi), Guard D (`while (--i)`
  latch incoming defined in the header), or Guard E (nested loops).
- Do not revert IVSR pointer IVs that live in a rotated self-loop
  (`univsr` skip); unrotated preheader+latch pointer IVs must still revert.
- Do not put complete-unroll carried-phi Copies of the LAST clone dest in
  the header (Copy-before-def). Copy INIT in the header; rewrite outside
  uses to the last clone dest.

## Session 2026-09-01 — remaining 3 of 15 root-caused

The 19-name rotate-ON vs GCC -O2 A/B is **19 MATCH / 0 FAIL** (the original
15 plus `loop_rotate_seq_loops` and `loop_rotate_while_dec`). Rotation
stays **opt-in** (`CCC_LOOP_ROTATE=1`). Do not flip the default until the
full 474-test corpus is green.

Root causes (distinct; all three FAILs MATCH gcc with rotation OFF):

1. **Pred-label (sequential loops)** — self-loop phi init incoming labeled
   with the original preheader instead of the guard. Option A
   `(pre_op, header_label)` + Guard C. Tests: `loop_rotate_seq_loops`,
   `alloca_bare_builtin`, `bitops_builtins`, …
2. **bepre hoist** — `schedule_eprime_early` used a stale `def_site` after
   header-phi insert. Current-IR `latest_dep` + never insert inside the
   phi cluster. Tests: `backedge_pre_int_recurrence`,
   `backedge_pre_fp_multiuse`.
3. **univsr × rotated pointer IVs (`alu_peepholes`)** — `detect_ivsr_pointer_ivs`
   treated a rotated self-loop pointer phi (`incoming` from the phi's own
   block) as an unrotated IVSR IV and reverted with the wrong index/base.
   Skip self-incoming / self-terminating pointer phis. Unrotated
   preheader+latch IVs still revert (SIB). Test: `alu_peepholes`.
4. **Guard D / `while (--i)` (`huft_build_crash`)** — cloned `i_next = i - 1`
   rewrote `i` to the latch incoming, which is the header Sub itself, so
   the backedge tested the loop-invariant `(g-1)-1`. Infinite pointer walk,
   SIGSEGV. Bail when a header phi's latch incoming is a non-phi header
   inst. Test: `huft_build_crash`, `loop_rotate_while_dec`.
5. **Complete-unroll Copy-before-def + Guard E (`simd_crc_adler`)** —
   two-block complete unroll replaced carried phis with `Copy phi = last_clone`
   in the header (def is in a later clone). GVN/LICM then, after inlining
   the remainder `for (; i < sz; i++)` into `for (sz = 1; …)`, froze `sz`
   to 1 (adler sz=2 returned `00010001`). Fix: rewrite outside uses to the
   last clone dest; Copy INIT in the header. Guard E: do not rotate a loop
   nested inside another (tighter "cond uses outer-header phi" is not
   enough — copy-prop hides the phi dest). Test: `simd_crc_adler`.

Still open before default-ON: full corpus, 9-worst, nbody bit-identical,
kernel 15/15, fuzz. Guard E is a correctness fence; nested inner loops
no longer rotate (perf left on the table — a sound nested-rotate rewrite
is future work).

## Session 2026-09-01 — post-merge audit (`453cbea`, PRs #325–#328)

HEAD = `origin/main`. Merge vs our remaining-three: `loop_rotate.rs`
diff is #325's pred-label comment only (code already used
`(pre_op, header_label)`). Fastbuild + 21-name rot-ON/OFF A/B vs GCC
-O2: **21 MATCH / 0 FAIL** (original 15 + seq/while_dec/stale_phi_pred
+ ra09_selfop_xor).

Polish landed this audit (not a new miscompile):
- `CCC_LOOP_ROTATE` is truthy-only (`1`/`true`/`yes`/`on`); empty/`0`
  no longer silently enables. `CCC_NO_LOOP_ROTATE` is implemented
  (was documented, never checked).
- Guard C comment no longer claims `(pre_op, pre_label)`.
- `godbolt.py` `clang` alias → CE `cclang2310` (Clang 23.1.0).
- univsr `reject_nonzero_init_counter` latch label is BlockId(2).

Do not flip the default. Do not re-try tight Guard E. ZSTD P0 still
needs kernel tree + qemu.
