# Follow-up: PR-audit verdicts, jmp miscompile fix, loop-memset pass, rebase to 6ec7b727

Date: 2026-09-11 · Base: `ms178/lccc` main `@ 6ec7b727` (PR #494; includes
#490 home-freshness and the merged loop-idiom memcpy pass) · Deliverable:
`/home/user/ms178-1.patch` (ledger S08) · Series rebased from the S01–S03
line (worst-10 baseline, i686-gated span-steal, bool-pair fusion + knobs).

## 1. Review verdicts (both audit findings resolved with evidence)

**Finding 1 — missing tail `jmp` on the bool-pair fused chain: VALID, and
worse than "likely".** Code inspection confirms the fused arm of
`emit_cond_branch_blocks_impl` was the only branch emitter without the
`if !cold_next { jmp cold }` guard: in the else-shape it relied on
fall-through reaching the cold successor, but `next_block_label` comes from
raw IR block order, so the neither-successor-next layout fell into an
unrelated block. Demonstrated as a full wrong-output miscompile on a
targeted TU (`(v & 15) < 6 && i >= 128` chain, neither-next layout):
gcc 2105, pre-fix 2009, post-fix 2105. The two previously-known fused
sites (lz4, zstd) are hot-next-shaped, which is why the corpus never fired
it. Fixed (`a3b0e5cb`); pinned by
`tests/regression/check_bool_pair_tail_jmp.sh` (structural fall-through
invariant + differential output + loud failure if the fusion stops
firing). The test TU exercises the neither-next shape directly.

**Finding 2 — baseline/tests: half right, half stale.** Right: the
patch-as-reviewed carried no regression tests for the new passes. Wrong:
the codegen-baseline job does not go red by design — the gate never fails
on improvements ("A metric that got BETTER never fails"), and the base
`6ec7b727` already carries the refreshed baseline (merged with the
loop-idiom memcpy work). Measured on the rebased tree: the new loop-memset
pass changes NO golden-workload metric (lz4_compress is not a golden
workload; the golden set is unchanged), and the bool-pair knob only
shrinks insn/move counts. Gate green (see §4).

## 2. Rebase account

Workspace restore lost `.git`; history rebuilt from the S04 snapshot
bundle (`artifacts/lccc.bundle`, test-cloned). S01 (baseline),
S02 (span-steal + bool-pair), S03 (fmt/knob/i686-gate) cherry-picked onto
`6ec7b727` cleanly (auto-merge absorbed upstream drift in live_range.rs /
emit.rs / prologue.rs). The S04 loop_idiom.rs is SUPERSEDED by the merged
`3ebe7c2d` (memcpy recognition, early, opt-in, nested-capable preheader
call design) — dropped, not clobbered; this session contributes the
complementary memset pass instead (§3). Swap file re-created and verified
active (4 GiB) per standing order.

## 3. Loop-memset recognition (11e3a2b1)

Constant-byte fill loops → `memset(3)`, structurally disjoint from the
merged memcpy pass (fill census refuses any Load). Both canonical shapes
(while-form 2-block, do-while rotate-guard); byte/wide stores; zero at any
width; nonzero uniform patterns with constants interpreted at the STORE
width; indexed and advancing-pointer forms; exact trip counts; exit-value
reconstruction; loop-defined invariant-global bounds (`i < N` re-loads the
global in the header) allowed only when provably distinct from the fill
object, re-materialized in the preheader guard.

lz4_compress x86-64 on this tree: 101,592,183 → 99,815,658 Ir (−1.75%),
md5 `fe42b7842634228b29755f7e3adfead5` == gcc. Differential corpus sweep:
39/39 workload programs byte-identical to gcc. Tests: 6 unit, 2
differential regression tests, `check_loop_memset.sh` decision pinning
(7 rewrite shapes, 4 near-miss keeps, both kill switches), both new
gates wired into `ci_local.sh --fast`.

Design note carried from measurement: nested COPY loops refuse in the
early pass's absence of call-aware RA planning (call barrier evicts
enclosing-loop homes; +12.6M Ir on lz4's literal copy when forced);
nested FILLS stay allowed (cold init code, net win measured).

## 4. Verification

`ci_local.sh --fast`: 19 passed / 0 failed / 3 skipped (includes the two
new gates), ALL GATES GREEN. `cargo fmt --check` clean; strict clippy
`-D warnings` clean; unit suite 2367 passed / 0 failed. lz4/sha256/
chacha20 md5 equality with gcc re-checked post-rebase.
