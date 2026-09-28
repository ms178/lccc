# S15: indexed decider/emitter mirror + AArch64 D32 (2026-09-28)

Sequel to `FOLLOWUP-2026-09-28-S13-recovery-and-f2-redteam.md`. That session's
tree (base 32363ac) was wiped mid-`ci_local.sh --fast` by a harness restore;
this session recovered onto the S14 lineage (base 563fb0bc, HEAD 9cc775c6)
and re-derived the follow-up program S14-natively. Several 09-28 items turned
out to be 32363ac-specific (the S14 code predates the buggy shapes); the
re-derivation found one genuine silent miscompile (AArch64 D32 indexed) plus
one latent unsoundness (trait default). All verdicts below are executed.

## Recovery ledger

- `arena_session_restore.sh` recovered swap (8G), Rust 1.98.1, and `.git`
  from `artifacts/lccc.bundle` (branch s10-rebase-main, HEAD = S14).
- The worktree was frankenmix: 25 modified files, of which 10 looked like
  09-28 work and 14 were other-lineage churn (passes/, peephole/, asm_expr,
  elf_writer, ci_local.sh, ...). Triaged per file: KEPT decimal.rs (F9),
  const_arith.rs (funnel), the battery rewrite, ir/lowering fmt×3; CHECKED
  OUT the rest to S14 pristine; RESTORED 2 evicted files (scratch gate .sh,
  pointer_const_multi_level_relro.c).
- Post-triage: `cargo fmt` with the pinned 1.98 toolchain reverted the
  ir/lowering×3 reflows byte-for-byte (S14 was already 1.98-clean; the 09-28
  reflows were rustfmt-1.9.0-shaped churn). Final tree: S14 + 7 modified
  files + D64 gate pair + this doc. No ir/ diff remains.
- REGRET: the 14 checkout-discarded files were never saved to a salvage
  patch first. If any contained genuine 09-28 work (the "narrowing" item in
  the old task list is unaccounted for — no record of its content survives),
  it is unrecoverable. LESSON (hard rule): `git diff > /tmp/salvage.patch`
  BEFORE any checkout triage, no exceptions.

## Porting verdicts (09-28 program → S14)

- D9 (retain fixed-point): N/A on S14, with proof. S14's pipeline runs
  `retain_indexed_ptr_only_uses` twice around `propagate_stable_aliases`;
  a retain pass is order-independent (removals apply after the scan) and
  removals are monotone-safe, so one pass after the last map-adding
  mutation is a complete decision procedure. No loop needed.
- F1/F2 (32363ac `emit_indexed_store_pre` + staging window): N/A on S14.
  S14's phase9 store/load are `return false` (deliberately dead, documented);
  the live paths are the IVSR peephole and the generation-map path, both
  covered by S13's write-confinement proof + contract tests.
- D9-deref (`deref_base_plus_offset`): does not exist on S14 (the buggy
  deref path is 32363ac-newer). S14's `indexed_fold_ok` is the conservative
  shift-guard + type-allowlist form. Verified sound as-is.
- ivsr D64/D32 arms: NOT NEEDED. Probed: D64 loop folds via the map path
  today (`movsd (%rbx,%r13,8)` load + store). The (a)-path gap is
  unreachable-behind-(b); adding arms would be dead redundancy.
- B3 (idx_dead_chain): 32363ac-specific walk shape; S14's skip machinery
  (`foldable_folds` gated on `can_indexed_addr_fold`, store folds keep
  chains live) was read end to end and is sound as-is.

## New work (S15 deltas)

1. **Fail-loud `indexed_fold_ok` default** (`backend/traits.rs`). The old
   default accepted all non-wide-int types while the default emitters
   refuse everything — a decider-yes/emitter-no split, the exact shape the
   soundness contract forbids. RISC-V (no override, never consults) is the
   latent witness. Default now refuses; a `debug_assert` fires loud on any
   real candidate reaching it. Tests: quiet-refusal (both modes) +
   `#[should_panic]` loudness (debug-gated).
2. **AArch64 decider type allowlist** (`arm/codegen/emit.rs` + 3 contract
   tests). The decider had NO type check; the emitter's general arms move
   64-bit carriers. D64 folds width-exactly (kept); D32 rode the 64-bit
   arms (refused, see 4); F128/wide refused (map-excluded upstream, mirrored
   for honesty).
3. **AArch64 emitter width gates** (`arm/codegen/memory.rs`, load+store).
   D32 → refuse (unfold). Proved as defense-in-depth: with the OLD decider
   + these gates, D32 still unfolds (second layer catches it).
4. **AArch64 D32 silent corruption (the catch).** Pure-S14 arm backend on a
   reverse-index D32 loop emits `ldr x0, [x19, x22, lsl #2]` — a 64-bit
   over-read whose value flows into `str x0, [x9]` over-stores, corrupting
   the destination array's neighbor elements. Repro: pointer-arg
   `_Decimal32` reverse loop, `-O2 -S` via the aarch64-named binary.
   Post-fix: GEP rematerialised (`lsl`+`add`), no SIB memop.
5. **x86 store-side type pin** (1 unit test). The S13 suite pinned load-side
   acceptance and store-side HOMES; store-side TYPES (D64/D32 included) were
   unpinned. Pinned now; the D64 e2e gate is its companion.
6. **D64 indexed gate pair** (`decimal64_indexed_fold.c` +
   `check_decimal64_indexed_fold.sh`): structural scale-8 SIB movsd
   (load+store) + gcc differential over an INTEGER-domain checksum.
   Bit-exact `_Decimal64` comparison was tried first and REJECTED: quantum
   drift across 64 accumulations is legal (lccc pads, gcc does not), so a
   bit gate would fail a correct compiler. Wired into ci_local.sh + ci.yml;
   parity PASS (79 commands).
7. **Scratch-gate narrowing.** The `leaq` ban is now the remat signature
   (`leaq -?[0-9]*\(%(rsp|rbp)\)`): MUT-B still trips it with the exact
   pre-fix `leaq 8(%rsp), %rcx` ×2, while a future legitimate non-frame LEA
   no longer can.

## Validation summary

- `cargo test --profile fastbuild --lib indexed_fold`: 13/13 (6 new).
- Debug loudness: 2/2 (incl. should-panic).
- Decimal unit: 24/24 (salvaged F9). Battery: PASS=107 FAIL=0 ROD=0.
- `cargo fmt --all -- --check`: clean. Clippy fastbuild `-D warnings`: green.
- MUT-A (D64/D32 out of decider): D64 gate FAILS structurally (unfolded
  `movsd (%r10)`), scratch gate still PASSES (specificity), exactly the 2
  type-accept unit tests FAIL. MUT-B (refuse-all): both gates FAIL, scratch
  on the narrowed signature. Both restored byte-identical; all re-green.
- `ci_local.sh --fast`: GREEN (89 passed, 0 failed, 5 skipped —
  expected --fast skips; cargo-test 3732/0; asm-diff 583/583 vs GAS 2.47;
  log: `/home/user/ci-fast-S15.log`, CI_EXIT=0). Stamp is mode=fast, so the
  final snapshot is ledger-PARTIAL (full --slow deferred to hosted CI per
  validation economy).

## Audit notes (read, deliberately unchanged)

- i686: decider and emitters share ONE predicate (`sib_scalar_ty`) plus
  mirrored shift/home guards — the gold-standard shape. The emitter's
  index-home recheck is provably redundant (`can_indexed_addr_fold`
  guarantees the home first) but harmless. No gap.
- x86-64: complete for D64/D32 on both halves (commons + decider + tests);
  the D64 gate pins it e2e. No gap.
- AArch64 D32 GLOBALLY (queued, NOT fixed): `reg_for_type`,
  `load_instr_for_type_impl`, `str_for_type` all route D32 through 64-bit
  carriers on EVERY path (the fixed binary's unfolded f32r loop still emits
  `ldr x0`/`str x0`). Fixing that is an AArch64-decimal subsystem project
  (homing/ABI/consumers) with ZERO e2e coverage in-repo and no aarch64
  runtime here — unverifiable, so untouched. Needs qemu-aarch64 + an arm
  decimal battery. Evidence: `/tmp/armd3*.s` shapes (see session notes).

## Tooling lessons (future agents)

- Same-file `edit_file` batches are LOSSY even at 2 calls (the arm store
  gate reported success and vanished). One edit per file per batch; always
  post-verify with grep.
- NEVER hand-reflow for a different rustfmt version: always `cargo fmt`
  with the PINNED toolchain and re-check. (1.9.0 vs 1.98 array-breaking
  drift cost a salvage round-trip.)
- `git checkout -- <paths>` is irreversible: save the salvage patch FIRST.
- A stale `running` process handle + successful build ≠ current tree: the
  restore-built binary came from frankenmix and failed the scratch gate;
  rebuild-after-triage fixed it. Always rebuild after tree surgery.
- `grep -E 'movsd .+\(...'`: `.+` needs ≥1 char; no-disp SIB needs `.*`
  (re-learned from 09-28: the D64 gate uses `-?[0-9]*` for the disp).

## TODO for future agents

- CC-O0CALL-1 (queued, untouched).
- AArch64 D32 global width bug (see above): needs qemu-aarch64 + arm
  decimal battery before any fix is verifiable.
- CI `./scripts/build_lccc_fast.sh` vs 644-tracked scripts: still open from
  09-28 (how does hosted CI execute these?).
- Debug-assertions e2e for indexed paths: fastbuild gates can't trip
  debug-only guards; confirm PR-#639-style debug CI steps compile
  indexed-store-heavy code.
- Seg-override indexed consumers + stale index homes at the access (09-28
  items (ii)/(iii)): still open; the remat paths are the tripwires.
