# Session 2026-09-04 — Re-base to ms178/lccc main 5649e27 + phi-coalesce hardening

## Outcome

- Upstream main advanced `b77736c -> 5649e27` (merge of PR #393, which also pulled in PR #392).
  Upstream now carries the pieces that were previously lost session work: base integer
  split-latch phi coalescing (`phi_window_clobbers_caller_saved` + 3 tests), the
  `x86_body_implicitly_clobbers_rdx` %rdx param-homing gate, CVP recursive depth-capped
  copy look-through, the vec_load_sink use-site remap, and the dead_writes LEA-folded
  store shape. Those were taken from upstream during conflict resolution (`cvp.rs`,
  `dead_writes.rs` reset to HEAD).
- The surviving layer of the session's work was re-applied and committed on top as
  **commit `7ff5d97`**, base `5649e27`:
  - regalloc: Cast/GEP-only `derived_before` propagation; `uses_excluding` eligibility
    flip (sources with other consumers keep allocator homes); per-use-block
    `src_use_path_clear_of_dest_redefs` two-state DFS; rewritten GEP-derived-escape test +
    `accepts_materialized_binop_escaping_source_block` + split-latch helper battery
    (5 tests); **two NEW fixes found this session** (see below).
  - expr_builtins.rs / sema/builtins.rs: `__builtin_dprintf` / `__builtin___dprintf_chk`
    registration, libc alias map, FortifyChk arm, `__dprintf_chk => 3` fixed arity.
  - prologue.rs: fused-cmp/fused-forward def excluded from `never_materialized`.
  - memory_fold.rs: legacy two-operand scalar-FP folds (gated).
  - vec_load_sink.rs: consumer relocation by value.
- Deliverables: `/home/user/ms178-1.patch` (46 KB, **APPLIES-CLEAN** verified against
  `5649e27`), artifacts in `/home/user/artifacts/` (snapshot S01 ledger, series, tarball,
  bundle; `.base_ref` = `5649e27`). Conflicted `git stash` entry dropped.

## New bugs found and fixed this session

1. **Apply-phase call-span guard** (`apply_phi_coalesce_assignments` in regalloc.rs).
   `tests/regression/loop_rotate_seq_loops.c` (`.env`: `CCC_LOOP_ROTATE=1`,
   `CCC_DISABLE_PASSES=vectorize`) miscompiled: stdout matched gcc (`36`) but exit code
   was 1 vs gcc 0. The rotated second loop's backedge source `s` was re-homed into the
   phi's caller-saved register; the (def, copy) window contains no call, but the exit
   block's `printf` clobbers the inherited home and the post-call `s != 36` Cmp read the
   callee's leftovers. Fix: consult the source's FULL live interval against
   `liveness.call_points` via `spans_any_call` (existing idiom); veto caller-saved homes
   only (callee-saved homes survive calls). Unit tests:
   `apply_refuses_caller_saved_home_across_call_outside_window` (incl. callee-saved
   acceptance leg) and `apply_accepts_caller_saved_home_when_no_call_in_source_range`.
   Note: `CCC_NO_PHI_COALESCE=1` also fixed the repro, isolating phi coalescing as the
   culprit.

2. **Dirty-path DFS traversal-through-use** (`src_use_path_clear_of_dest_redefs`). The
   two-state DFS stopped at a clean use of the source, missing the corrupting path
   use-block -> second latch (re-defines the phi dest) -> header -> use-block: on that
   later pass the use reads the shared home with no fresh source def in between.
   Exploration now continues THROUGH the use block; only *dirty* visits veto. Test
   `rejects_src_use_via_second_backedge_copy` now genuinely exercises this (init copies
   moved to a preheader block so the header never re-defines the phi on entry).

## Validation (all at 5649e27 + 7ff5d97)

- `cargo test --profile fastbuild --locked -j2 --lib`: **1772 passed / 0 failed / 6 ignored**
  (20 of them `phi_coalesce` tests).
- `scripts/run_regression_suite.sh`: **PASS=605 FAIL=0 SKIP=15, AB-diff 0** — includes the
  previously failing `loop_rotate_seq_loops` (now green).
- Bug A repros (`/home/user/repro/`): `b1_no_proto`, `b2_with_stdio` output identical to
  gcc — `f32=[1.5] i16=[-3] u8=[200] c=[65] dbl=[2.250]`, chk `f32=[2.5] c=[-7]`.
- Bug B repros: `b5_runner` output identical to gcc (`adler=63111232 twoacc=63111232
  fpsplit=37.500000`); `adler_split`/`twoacc_split`/`fpsplit` loop bodies copy-free
  (only the byte load; `movq %r8,%rax` epilogue move). `b3_acc` `adlerish` copy-free;
  `b4_split` `adler_break` split-latch selects via cmov, no accumulator copies.

## Divergence from upstream #392 (must be defended in any upstream PR)

Upstream's `rejects_split_latch_when_gp_derived_value_escapes` asserts a **BinOp**-derived
escaping value vetoes coalescing. This contradicts the Cast/GEP-only `derived_before`
refinement: a BinOp destination materialises at its own definition point, so the copy is
a plain register move and legitimately coalescible — a BinOp escape veto would regress
materialised-binop backedge sources (`accepts_materialized_binop_escaping_source_block`
covers this). The test was rewritten to a real GetElementPtr+Load escape, which IS the
unsafe shape (the address chain must be born before the def to be shareable).

## Standing rejections (do not redo)

- Do NOT re-remove eligibility for sources with other consumers — regresses the
  `adler_split` a-chain into slot round-trips.
- Do NOT propagate `derived_before` through all instructions — deadlocks the counted-loop
  cascade via Load's GEP dependency.
- Do NOT make the dirty-path DFS stop at clean uses — re-introduces the two-backedge
  corruption.

## To-do for future agents

1. **fpsplit `movq %r8,%rax` quirk**: `fpsplit`'s loop body carries a useless
   `movq %r8,%rax` (index copy), present even with `CCC_NO_PHI_COALESCE=1` — a
   non-coalescing register-allocation artefact. One instruction per iteration on the
   zlib-ng adler shape.
2. **Loop layout `jmp`**: rotated loops still emit `jae .LBBn / .LBBn+1: jmp .LBBn-1`
   instead of inverted fallthrough — loop layout pass gap (no rotation of the loop body
   into fallthrough position).
3. **Upstream PR**: prepare a PR for ms178/lccc with the surviving layer (regalloc
   refinements + dprintf fortify + fused-cmp prologue gate + memory_fold + vec_load_sink
   by-value), with the #392 test divergence explained (see above).
4. **Benchmarks/oracle**: re-run godbolt.py oracle vs GCC 16.2 / Clang 23.1 / ICX on the
   adler/rotate kernels with the merged tree; update the linker oracle per user's
   build/version preferences (GAS/bfd 2.47, mold 2.42 X86/i686 preset — exact cmake
   option still needs to be looked up).
5. **Kernel boot gate** skipped (KERNEL_DIR not set) — set up and run if the kernel
   build is in scope.
6. `loop_rotate_seq_loops` is worth keeping permanently in the regression suite as the
   e2e guard for the apply-phase call-span fix.
7. `/tmp/rebase/` holds upstream-vs-mine copies of the 4 conflicted files — reference
   material for the PR write-up; not persisted, re-create if needed from git.

## Snapshot state

- Repo: `/home/user/lccc`, HEAD `7ff5d97` on main, base `5649e27`.
- `/home/user/ms178-1.patch` — canonical deliverable (regenerated + verified).
- `/home/user/artifacts/` — S01 ledger, series, tarball, bundle; `.base_ref=5649e27`.
- Stash list empty.
