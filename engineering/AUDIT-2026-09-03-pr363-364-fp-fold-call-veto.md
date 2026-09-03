# Red-team audit of PRs #363/#364 — FP fold call-veto gap (fixed)

Session 2026-09-03.  Scope: the delta `da964ed4..77eb34e1` (PR #363 = the S26/S27
MachInst work landed verbatim — 8 of 12 files byte-identical, the 4 diffs are
PR #364's additions; PR #364 = narrow wide-imm store fast path, volatile
const-store split refusal, FP fold kill proofs requiring straight-line code).

## Verdicts

* PR #363: AGREE — content identical to the audited S26/S27 revision.
* PR #364, fixes 1 (narrow wide-imm store) and 2 (volatile split refusal): AGREE.
  The narrow fast path's `movl $imm, %eax` register form is exact under the
  backend's value convention (narrow register values live in the low bits;
  consumers extend), and the memory forms store exactly `size` bytes.
  The volatile single-access contract matches C11 5.1.2.3 at the 8-byte
  granularity x86 can express in one instruction.
* PR #364, fix 3 (FP fold kill proofs): PARTIALLY DISAGREE.  The straight-line
  (is_cf_transfer) requirement is correct and closes a real dominance gap.
  But its justification for excluding Call lines is WRONG as stated, and two
  of the three folds were missing the call veto the commit message claimed
  "the passes keep".

## The gap: a Call is an opaque READER of %xmm0-%xmm7

"is_cf_transfer excludes Call: a call clobbers every caller-saved xmm, so no
reader can observe the loaded value across one."  The first half of that
sentence is false on SysV for %xmm8-%xmm15 (callee-saved), and both halves
miss the real hazard: the call ITSELF can be the reader.  %xmm0-%xmm7 are the
FP argument registers; when the loaded value is passed to the call, the
argument move is a self-move the CallTyped builder elides, so the read leaves
no textual trace — `call foo` mentions no register.  A kill proof that scans
textually ("first later mention is a pure full-width overwrite") then deletes
a load the call still needs.

  movsd  C(%rip), %xmm0      ; deleted by the fold
  vaddsd %xmm0, %xmm5, %xmm5 ; consumer (src-only: %xmm0 still holds C)
  call   foo                 ; implicitly reads %xmm0 = C  ← invisible
  vmovapd %xmm1, %xmm0       ; first TEXTUAL mention = pure overwrite

The sibling fold fold_fma_memory_src2 already vetoes calls for n <= 7
(precisely for this reason, with wording about the "variadic" window);
fold_fp_register_loads and fold_zero_addend_fma213_to_132 did not.

## Reachability (data-driven)

Unreachable through the current backend: the isel materializes FP pool
constants per use (verified on the sweep corpus: each use re-loads; no CSE),
the CallTyped builder elides the argument move only when the value's home is
the argument register, and hoist_repeated_fp_constant_loads (the one pass that
would create a shared materialization) returns false if ANY Call or InlineAsm
line exists.  An empirical 192-function sweep (4 ops x 4 call shapes x 3
orders x pressure x double/float; -O2 -mfma; peephole ON vs OFF vs gcc -O2,
%.17g outputs) found zero divergence — confirming the latent status.

Fixed anyway as defense in depth: the veto costs nothing on sound shapes,
the sibling folds already pay it, and the isel's per-use materialization is a
performance decision, not a soundness contract.

## The fix

* fold_fp_register_loads: parse the loaded register number once; when
  %xmm0-%xmm7, a Call line in the (consumer, overwrite) stretch sets
  later_mention (fold refused).  %xmm8-%xmm15 keep scanning: never argument
  registers, callee-saved, and compiled code never reads a callee-saved
  register before defining it.
* fold_zero_addend_fma213_to_132: same veto (b <= 7) in the ok_after scan;
  %xmm0 is excluded by construction (it is the zeroing register itself).
* is_cf_transfer doc + fold_fma_memory_src2 comments: precise contract
  wording (calls read %xmm0-%xmm7 implicitly as FP argument registers).

## Tests (red -> green verified)

* refuses_when_call_reads_loaded_arg_register (fp_reg_load_tests)
* refuses_when_call_reads_loaded_multiplier_register (fma132_zero_tests)
  — both verified RED on unfixed upstream (fold fires) and GREEN with the fix.
* still_folds_when_non_call_intervenes / still_folds_when_call_does_not_read_callee_saved_reg
  — positive controls for both folds: the veto is call-specific and
  callee-saved registers stay foldable.

## Validation

units 1670/0/6; regression suite PASS=570 FAIL=0 SKIP=15 AB-diff 0 (ELF32
runner not present in this session's environment; amd64 + units + sweep
cover the change).
