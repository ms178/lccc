# Follow-up — 2026-09-03: PR #358 audit implemented & Tier-2 re-enabled (on upstream main 4762fea1)

## Accomplished this session

1. **Rebased the Tier-2 audit work onto the latest upstream main `4762fea1`**
   (was `2ae63f82`; upstream merged PRs #359/#360/#361 in between — peephole
   FP-fold, our ms178-1.patch as #360, BitTest fix + the slot-stress harness as
   #361). Branch `wip/pr358-audit-rebase`. No overlap with our touched files.
2. **Red-team audit verdict implemented** (see `engineering/AUDIT-2026-09-03-PR358-tier2.md`):
   keep PR #358's width veto (hardened to a widest-wins fixpoint type map) and
   DISAGREE with disabling Tier-2 — re-enabled on by default with a
   closed-boundary hull (fat-interval) interference model; per-segment sharing
   is an A/B-only opt-in; setjmp keeps the unique-slot fallback.
3. **New machinery/tests**: width-invariant debug verifier (`CCC_VERIFY_SLOT_WIDTHS`),
   debug colorer accounting (`CCC_DEBUG_SLOTS`), 8 new Rust unit tests
   (fixpoint + colorer, incl. randomized brute-force interference cross-checks),
   3 new C regressions, Tier-2 A/B differential (step 3b) in
   `run_regression_suite.sh`.
4. **Slot-stress harness made deterministic & future-proof**: `region_setjmp`
   locals are `volatile` (indeterminate-after-longjmp oracle fix);
   `run_slot_stress.sh` factorial now uses the real knobs.
5. **Corpus verification on the rebased tree**: unit 1626/0, regression
   575/0 (+Tier-2 A/B, +per-segment whole-corpus, +width-verifier whole-corpus
   all green), slot stress seeds 1..20 → 400/400.

## Open items for the next agent

1. **Fix the pre-existing -O2 miscompile** (`engineering/BUG-2026-09-03-O2-vla-store-miscompile.md`,
   repro `tests/bugs/o2_vla_fill.c`, generator seed 32): -O1/-Os correct, -O2/-O3
   wrong; all four layout configs wrong; DSE/LOOP_INVERT/MERGE_BLOCKS mask the
   seed-32 case but not the minimal one; pass delta -Os→-O2 is vectorize /
   iv_widen / loop_rotate / iter-2 simplify / bit_idioms / if_convert. Then push
   the slot-stress default range beyond seeds 1..20.
2. **Kernel zstd preboot oracle**: tree at `/home/user/kernel-work/linux-6.18.47`
   was truncated by the workspace wipe (1.7 MB, no `arch/x86`). Regenerate with
   `scripts/prepare_kernel_tree.sh` (~3 min; needs `PKGDIR`, 155 MB tarball from
   cdn.kernel.org — HTTP 200 verified) and reinstall qemu, then run
   `scripts/zstd_preboot_oracle.sh` as ground truth for the hull-colorer frame.
3. **Re-run the -O1 oracle regression** (session10 §7.1, upstream CallTyped) once
   the kernel tree is back; it remains open on main.
4. **Upstream the commit**: this branch's diff vs `4762fea1` is the next
   `ms178-1.patch`; ensure the harness diffs (#361 slot-stress changes) are
   acceptable upstream or split the commit.
5. Re-check upstream HEAD before submitting (`git ls-remote`); regen snapshot +
   patch after any further changes.
