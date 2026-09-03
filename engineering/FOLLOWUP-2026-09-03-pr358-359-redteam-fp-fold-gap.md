# Session follow-up — PR #358/#359 red-team, VLA store-width root cause, FP-fold soundness gap

Date: 2026-09-03. Branch `wip/audit358-359` on `ms178/lccc` main `da964ed4`
(tip; re-fetched, zero new upstream commits this session).

## What this session did

1. **Committed the -O2 VLA-store root-cause fix `b10ef6c0`** (narrow MachInst
   stores of large unsigned immediates overran 4-byte VLA elements with an
   8-byte `movq`). Regression artifacts added by the commit: 7 golden MachInst
   emitter tests and the gcc-oracle regression
   `tests/regression/vla_largeconst_u32_fill.c`; the minimal reproducer
   `tests/bugs/o2_vla_fill.c` already exists on upstream main (added by #362
   `c8683317`) and is used as-is. Full history of the finding, fix contract,
   and the emitter facts that justify the register-destination direct sized
   immediate (raw imm field + `movl` zero-extension) is in the commit message
   and now in `engineering/BUG-2026-09-03-O2-vla-store-miscompile.md`.
2. **Red-team audited PR #359** (`ab9c11a7`): confirmed the constant hoist is
   sound; found a **real soundness hole** in the shared FP-fold liveness
   refinement (pure-overwrite "kill" accepted without control-flow awareness →
   a branch between consumer and overwrite leaves a merge reader depending on
   the deleted load's value).
3. **Committed the fix `722f7c3c`**: `is_cf_transfer` straight-line
   requirement in all three fold passes + three diamond adversarial regression
   tests (each fails pre-fix, passes post-fix) + landed
   `scripts/gen_fp_stress.py` (deterministic PR-#359-path differential
   generator).
4. **Red-team audited PR #358** (see `AUDIT-2026-09-03-PR358-tier2.md`): the
   prior session's verdict doc was already in-repo; this session re-validated
   that the current merged state (#360–#362 hull-model Tier-2 + widest-wins
   type map) plus this session's two commits is green across the whole corpus,
   and established the *orthogonality* of the VLA bug to #358/#362's slot-width
   fix (store side vs reload/sizing side — documented in the BUG doc).
5. **Godbolt-informed structural comparison** of the canonical dot8d FP kernel
   vs gcc 16.2 / icc 2021.10 / clang 23.1 / icx-latest (pinned oracle set in
   `scripts/godbolt.py`): lccc is instruction-equal to gcc16.2 and better than
   icc in the scalar-FMA space at -O3 -march=x86-64-v3; clang/icx win by FP
   vectorization (a separate, tracked gap). A/B measured with
   `CCC_PEEPHOLE_SKIP` kill switches on the same binary.
6. **Engineering docs updated**: BUG doc resolved with root cause + evidence
   table; `AUDIT-2026-09-03-PR359-fp-folds.md` written; this follow-up.

## Verification (all on `da964ed4` + `b10ef6c0` + `722f7c3c`)

| Gate | Result |
|---|---|
| `cargo test --profile fastbuild --lib` (full unit suite) | **1636 passed / 0 failed** / 6 ignored |
| `scripts/run_regression_suite.sh` | **PASS=576 FAIL=0 SKIP=15** (AB-diff failures: 0) |
| `scripts/run_slot_stress.sh 1 45 -O0 -O1 -O2 -O3 -Os` | **PASS=900 FAIL=0** (seed 32 now gcc-exact in all 4 layout cells) |
| FP differential (PR #359 paths), seeds 1..60 x {-O1,-O2,-O3} | **PASS=180 FAIL=0** byte-exact vs gcc |
| `tests/bugs/o2_vla_fill.c`, `vla_largeconst_u32_fill.c` (-O2/-O3) | byte-exact vs gcc |
| dot8d -O3 x86-64-v3 insns (external data, FP-strict) | lccc 21–22 / gcc16.2 21 / icc 24 / clang 16 / icx 10 |
| dot8d -O2 SSE2 insns | lccc 28 / gcc 14.2 32 (lccc <= gcc) |

## Evidence recorded for reviewers

- Root cause write-up + oracle lines: `BUG-2026-09-03-O2-vla-store-miscompile.md`.
- PR #359 verdict + fix rationale + residual items: `AUDIT-2026-09-03-PR359-fp-folds.md`.
- PR #358 verdict (prior session) + this session's re-validation:
  `AUDIT-2026-09-03-PR358-tier2.md`.
- Deterministic generator + corpus runs: `scripts/gen_fp_stress.py`
  (180/180 gcc-oracle; corpus regenerated per run, seeds 1..60).

## Open items for the next agent

1. **Upstream delivery (standing rule)**: rebase these two commits
   (`b10ef6c0`, `722f7c3c`) onto the *latest* `ms178/lccc` main at delivery
   time and open fresh PRs; re-run the full gate table after the rebase
   (unit 1636, regression 576, slot stress 900, FP diff 180). Watch for new
   upstream commits — main was at `da964ed4` at session end.
2. **Port-pressure refinement (PR #359 follow-up, see audit doc)**: when a
   pool constant has ≥3 uses and a free register exists, prefer
   materialize-once over the re-read-as-rm-operand form left by the memory
   fold; measure on the dot8bench2 driver (load-port pressure, not
   instruction count). Decide the interaction so the two passes compose
   instead of competing.
3. **Vectorization gap**: clang/icx reach 16/10 insns on dot8d via packed
   lanes; lccc is scalar for FP reductions. This is the largest remaining
   structural gap on the flagship kernel and is *not* a #359 item.
4. **Kernel-boot oracles**: the zstd/QEMU kernel harnesses in
   `AUDIT-2026-09-03-PR358-tier2.md` still need a kernel tree to re-run
   (tree was wiped); the -O1 CallTyped regression noted there should be
   re-checked against current main (the -O1 slot-stress 180/180 pass is a
   good sign but not the boot-gate).
5. Consider widening the committed `run_slot_stress` default range 1..20 →
   1..45 now that seed 32 is fixed and in the regression suite.
6. Run `scripts/lccc-snapshot.sh` after the next committed state and refresh
   the canonical `/home/user/ms178-1.patch` + `/home/user/artifacts/lccc.bundle`.

## Capability/evidence notes for the supervised-session reviewers

- Working tree and commit history are the source of truth
  (`b10ef6c0`, `722f7c3c` on `da964ed4`); `.base_ref` pinned to `da964ed4`.
- 12 GiB swapfile is active (do not remove); builds use `--profile fastbuild
  -j2`; snapshots exclude `target/` — rebuild after any `target/` wipe.

---

## Addendum (same day, second session) — rebase onto PR #363 + new audit

Upstream merged PR #363 (`2c027cb1` → `e2d5ef66`) on top of `da964ed4` before
this branch was delivered. Per the standing rule, the branch was **rebased
onto the new main** and #363 was red-teamed (it touches the same code area and
independently fixed the same wide-immediate-store bug). Three of the four prior
commits were replayed and conflict-resolved onto `e2d5ef66`:

```
e2d5ef66  upstream main (PR #363 merged)
  f016149e  machinst: fix narrow stores of large immediates (was b10ef6c0)
  9236df3d  peephole: close control-flow gap in FP fold liveness (was 722f7c3c)
  f766a4da  docs (was 7f66e5ae)
  5ef2ea30  isel: never split a volatile F64/D64 const store (NEW, red-team of #363)
```

### Is our solution better than upstream's for the same bug? Yes — measured.

#363 fixed the VLA over-wide store with a sized `%rax` relay
(`movabsq $imm,%rax` + `movl %eax,mem`); `b10ef6c0` uses a single direct sized
move (mov imm field is a raw width-truncatable value). A/B on the exact
reproducer (`o2_vla_fill.c` `b[]` 4-store fill): upstream relay = 8
instructions and 4 `%rax` clobbers; our fast path = 4 single `movl $imm`.
Both pass all oracles; ours is structurally shorter and register-free. The
merged tree keeps both (fast path first, sized relay as the S64>i32 path and
defense-in-depth).

### What we adopted from #363 (valid new insights)

- `is_pure_xmm_overwrite` **merge-write rule** (reg-reg scalar movsd/movss
  preserve upper bits; only memory-source forms are full defines) — a real
  second soundness hole in the PR #359 machinery that our control-flow barrier
  is orthogonal to. Both conditions now gate the folds.
- Hoist **site dedup** (in-place rmw double-count) and **movss** materialization.

### Defect found in #363 (fixed in `5ef2ea30`)

The immediate-form float-constant store lowering split a `volatile` F64/D64
store into two 4-byte `movl` halves, violating C11 single-access volatile
semantics (torn state observable by signal handlers/concurrent readers).
gcc/clang/icc emit one pool+movsd. Fix: refuse the split for volatile stores
(single-access immediate forms still allowed). Demonstrated before/after on
`volatile double t; t = 1.5e300;`, pinned by 3 isel unit tests +
`tests/regression/volatile_const_stores.c`.

### Verification (all gates on e2d5ef66 + 4 commits)

- unit suite `cargo test --profile fastbuild --lib`: **1664/0**
- regression suite: **PASS=577 FAIL=0 SKIP=15** (round 1) / re-run with the
  new volatile regression pending at write time
- slot stress seeds 1..45 × {-O0..-Os}: **900/0**
- FP differential (PR #359 paths) seeds 1..60 × {-O1..-O3}: **180/0**
- VLA oracles (`vla_largeconst_u32_fill`, `vla_narrow_const_store_family`,
  `o2_vla_fill`) at -O2/-O3: byte-exact vs gcc

### Open items (carried)

1. Re-run regression suite with the volatile regression; then deliver a fresh
   PR against latest main (this branch), i.e. rebase (already current) and
   push.
2. Classic-path (global / Value-staged) narrow stores of >i32 immediates still
   emit the `movabsq`+`movl` relay (2 insns) where `movl $imm, sym(%rip)` would
   do (1 insn, gcc parity). Correct today; a codegen follow-up.
3. Indirect-call typed staging (#363 §4) left to its area owner; FP
   vectorization gap for dot8d remains the biggest structural item.
4. Refresh snapshot + canonical patch after the final regression run.
