# PR829 full three-way semantic audit — 2026-10-19

## Verdict

PR #829 (`7110d5af8fc2eb9cf64d7907af4b8ed0bed24acd`, one commit on
merge-base `3ff36e8592b0d0e26645bf37d6685395b1a6eca5`) was audited line by
line against main `4c0d36489235bb1f5febd272c96a4bc315084e6c` — 278 commits
later. The verdict is uniform: **every still-valuable change in PR829 is
already on main, in an equal or strictly superior form.** The 2026-10-15
synthesis pipeline (see `engineering/FOLLOWUP-2026-10-15-pr829-pr830-synthesis.md`
and `engineering/PR829-830-COMPARATIVE-AUDIT.md`) absorbed the PR's
substance, and the hardening that followed exceeded it.

Audit outcome recorded as merge commit
`74238a82c488d117be791b9b0f8c9a6b2acd6d0c`, whose tree is byte-identical
to main:

```
$ git diff origin/main 74238a82 --stat
(empty)
```

Nothing in PR829 remains to port. The only content the merge itself
brought in was excluded by policy (89 archived bulk-evidence files, below)
or rejected as an inferior design (one auto-merged `dce.rs` hunk, below).
A new gate, `scripts/check_archived_bulk_absent.py`, mechanically enforces
the evidence exclusion so the resurrection class of defect cannot recur.

## Method

- Full file-level comparison over the whole tree (not only conflicting
  paths): base→PR touches 125 non-evidence files (`+32,438 / −15,621`) and
  311 evidence files (`+825,309`); PR829 totals 436 files `+857,747/−15,621`.
- Residual main↔PR diff over all 1,005 differing files: 194,224 lines,
  including main-only content, so PR additions missing from main could be
  isolated per file (`missing=N` line sets) and then read as hunks.
- `git merge-tree --write-tree origin/main origin/pr829` → 55 content
  conflicts; each was reviewed hunk by hunk (three-way: base, PR, main),
  never auto-resolved.
- Every candidate "PR addition not present on main" was classified as one
  of: renamed/refactored spelling (e.g. `signed_overflow_wraps()` →
  `pass_policy.wrap_signed_overflow`), re-indentation artefact, superseded
  proof (main's is stricter), or genuinely missing — the last bucket was
  empty after hunk review; see the dce.rs and `.env` entries, which are the
  two places PR829 had line-level content main's tree did not.

## Area-by-area supersession table

| PR829 area | What the PR did | What main has | Verdict |
|---|---|---|---|
| RV64 shared binding | relocation contract tests, oracle | `shared_address_kind` + `relocate_address32` guard (semantic superset); PR830 pinned-SIB fix kept | superseded |
| IV / `-fwrapv` legality | `SIGNED_OVERFLOW_WRAPS` thread-local gating; signed wrapping subsets | per-function `IrFunction.pass_policy` + `src/passes/policy.rs`; counted-range signed-math proofs (`prove_counted_bound`, `normalise_bound_threshold`, `bound_leaves_room`) admitting runtime seeds/bounds the PR refused | superset |
| `-fwrapv` flags | `-O2 -fwrapv` oracle flags | identical flags plus wider matrix (`ivw_wrapv_range.c` f0–f11, sum 373767 vs the PR's f0–f7, sum 206604) | superset |
| Vectorizer legality / FP env | `fp_environment_blocks_reordering` whole-pass fences | same fences plus `local_fold_is_fenv_safe` whitelist, `ReductionDefinitions` fail-closed SSA index, copy-root walk bounded to eight hops, late-minmax phase policy, `map_pointer_domain_fits` pointer-ring domain proofs | superset |
| Strict packed recip-mul-add | `StrictPackedRecipMulAddF64x4` + `strict_i32_pack` | renamed `StrictRecipMulAddPackedF64x4`; planner retained `#[cfg(test)]` for differential tests, production uses strict_recip; `strict_i32_pack::Plan` owns transactional IDs with prefix hoisting to the preheader | superset |
| x86 peephole (dead_code, load_op_fuse, relay_and_lea, store forwarding, VEX flag reads, local patterns) | pattern additions | main's versions include `CfiIndex` import, pr832/pr834 tests, `movq %xmm` matchers matching main's codegen spelling, `vec_read_mention_width` + narrow-merge, FXSAVE/XRSTOR kill model, decorated-dst `{%k` handling, `upper_vector_access_line` | superset |
| FP liveness | textual xmm-absence fallback in `xmm_dead_after` | `ans == Some(false)` only — unknown CFG cannot prove deadness (textual absence misses implicit architectural readers), backed by `has_upper_write` + `f.live_out` | PR fallback is the defect class the synthesis fixed |
| Intrinsics | intrinsic table rows | home-aware `dest_xmm_home_name` vmovd/vpshufd emit, let-else denominator source, renamed strict intrinsic | superseded |
| `dce.rs` FP guard | whole-pass early `return 0` when the FP-environment fence bites | per-instruction rooting of FP-environment-observing instructions (`policy::observes_fp_environment` seeded into `live`) — dead integer work still dies, FP work never does | **main is strictly more precise** |
| `constant_fold` / `simplify` | early returns | `preserve_fp` / `strict_fp_environment` guards with the `local_fold_is_fenv_safe` whitelist | superset |
| Driver / pipeline / CLI / passes/mod.rs / regalloc | thread-locals, options | `pass_policy` plumbing, `configure_cargo_gate_resources`, `loop_bound` scheduling gates, checked `final_iv` arithmetic (`checked_mul`/`checked_add` instead of the PR's potentially overflowing `init + trips * step`) | superset |
| Workflows (ci, bench, compiler-research, debian-oracle) | 14 CI features (swap, gcc-16, clippy/rustfmt components, …) | all 14 present on main, plus refined step splits; bench.yml consumes the `lccc-fast` artifact instead of rebuilding; debian-oracle pins 1.99.1 > the PR's 1.99.0 | superseded |
| Scripts & regression tests | 37 regression tests, gates, oracles | every PR-touched test/script exists on main; main-only extras include `contract_shape.c` and the S28/S47 suites | superset |

## The 55 conflicts, resolved

49 were resolved in batched passes (all confirmed main-supersets, among
them the four workflows, `scripts/check_benchmark_outputs.sh` whose
`.py` wrapper already contains the PR's `gcc_include` guard,
`src/backend/x86/codegen/intrinsics.rs`, the three other peephole passes,
and the full test/script batch). The six residual conflicts were reviewed
hunk by hunk:

- `src/backend/x86/codegen/peephole/passes/fp_liveness.rs` (13 hunks) —
  main supersedes: FXSAVE/XRSTOR kill model and docs, decorated-destination
  handling (`dst.contains('{')` ⊇ PR's `{%k` + trim), i64 offset-overflow
  guards, `upper_vector_access_line`, extra tests. The decisive hunk is
  `xmm_dead_after`: the PR's `Some(live) => !live, None => textual xmm
  scan` fallback is exactly the unsound inference main's doc forbids.
- `src/passes/iv_strength_reduce.rs` (16 hunks) — main adds
  `dominating_global_addr`, the `IvsrCfgShape` cached-dominance
  assertions, the signed-unit recurrence certificate, and the measured
  SIB profitability refusals (`sib_scaled && !from_address_add &&
  !narrow_symbol_index`); the PR's side is the older subset.
- `src/passes/iv_widen.rs` (15 hunks) — main replaces the PR's
  "nonnegative constant seed / positive stride / nonnegative constant
  bound" subset with signed-math trajectory proofs (hunks 7–9: the PR's
  const-nonnegative-threshold refusal is superseded by
  `normalise_bound_threshold` + `bound_leaves_room` operating on signed
  endpoints, plus `latch_closed`/`exit_latch_dead` mirror-theorem
  escapes the PR lacks); tests cover negative seeds (`i32::MIN` rows).
- `src/passes/loop_bound.rs` (4 hunks) — main adds the `scheduled()`
  rollout gates, `affine_fold_debug` accounting, checked final-IV
  arithmetic, and the standalone CFG interpreter test.
- `src/passes/vectorize.rs` (23 hunks) — main supersedes throughout:
  byte-count normalisation kill switch, reduction analysis fail-closed
  index, copy-root identity rules, `map_pointer_domain_fits`, pointer-ring
  versioning tests, and the strict-recip production path (the PR's
  `STRICT_RECIP_PACK_ENABLED` thread-local plumbing became a plan-driven
  `denominator_args` flow).
- `src/passes/vectorize/strict_i32_pack.rs` (9 hunks, add/add) — the PR's
  257-line draft became main's 403-line `Plan`/`Builder` with prefix
  hoisting, depth-capped operand recursion, and transactional refusal.

`src/passes/dce.rs` was the **only** non-evidence delta auto-merged rather
than conflicted (disjoint hunks: the PR added a whole-pass FP early-return
at line 134; main had meanwhile grown per-instruction FP rooting at lines
227–228). It was reverted to main's version: rooting FP-observing
instructions keeps them — and everything they use — live while still
removing unrelated dead work, which the PR's blanket `return 0` forbids.

Two PR-only files remain by name: `tests/regression/{aggregate_split_constant_fields,casts_roundtrip}.c.env`.
Main carries both under its `foo.env` naming convention with byte-identical
contents (the `CCC_AGG_SPLIT_DEBUG=1` row and the `LCCC_NO_COMPARE=1`
x87 precision-control note). Nothing else outside `engineering/evidence/`
exists in PR829 that main lacks.

## Excluded evidence (89 files, 749,607 lines)

The merge staged, then dropped, every PR829 evidence file that main does
not carry:

- 76 files, +714,020 lines — pr-only `engineering/evidence/2026-10-14D-shared-binding/**`
  (callgrind `.cg` profiles, oracle/relocation-contract JSON);
- 13 files, +35,587 lines — `engineering/evidence/pr824-final-audit/**`
  (`.tar.xz` bundles, native/JSON/txt dumps).

**All 89 paths are rows in `engineering/evidence/ARCHIVED-BULK.md`** (0 of
89 missing): the bytes are preserved in
`artifacts/evidence-archive/engineering-evidence-bulk-s59.tar.xz` with
per-file sha256 receipts, and no source, script, test or workflow reads
them. Re-adding them would undo the archive decision and inflate every
future patch, so they are excluded from the salvage with prejudice.

Because a three-way merge resurrects pure additions without a conflict,
the exclusion is now mechanical: `scripts/check_archived_bulk_absent.py`
parses the archive index (fail-closed on an empty table), asserts none of
its 123 paths is tracked, and runs as its own self-test + check pair in
both `.github/workflows/ci.yml` (frontend job, next to the conflict-marker
gate) and `scripts/ci_local.sh` (before `build`, compiler-independent).

## CI status

- **PR829 head**: red — the Test Suite job fails at
  `tests/regression/check_fp_recurrence.sh` (exit 1); annotations pin the
  failure to that gate on runs 38650358802 and successors. The PR's
  fixture set predates main's `fp_recurrence/contract_shape.c` addition.
- **main `4c0d3648`**: green — every recent workflow run on main concludes
  `success`, including the same test-suite job with the evolved FP
  fixtures.

So PR829 cannot be brought to green as a head: its remaining red is a
property of its own frozen tree, while everything it fixed is already
validated on main. The salvage was therefore delivered as an audit merge
(`74238a82`) plus this report and the archive gate — a tree identical to
main plus enforceable evidence governance — and CI runs green from there.

## What this audit did not change

- No compiler source differs from main; there was nothing to fix: every
  defect class the audit found (textual liveness fallback, unchecked
  final-IV overflow, coarse FP pass-disable, over-narrow signed wrapping
  subsets, prescriptive RV64 oracle) is already repaired on main, and every
  missed optimisation the PR opened (signed wrapping, symbol-base reuse,
  pointer-ring addressing, strict packed recip) is already adopted in
  main's stronger form.
- PR830's pinned-SIB repair and the S26 rebase material are out of scope
  here; they were merged separately and are referenced from the 2026-10-15
  synthesis documents.
