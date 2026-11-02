# S13 recovery + F2 red-team (2026-09-28)

Sequel to `FOLLOWUP-2026-09-27-S13-codegen-gate-fix.md` (the audit addendum).
The 09-27 worktree was partially lost in a snapshot restore (the registry
blew the snapshot cap; the tree came back "frankenstein": post-#656 HEAD
plus fragments of uncommitted work, stripped +x modes, and two files with
duplicated-tail corruption). This session recovered, completed, and
red-teamed the S13 follow-up program. All verdicts below are executed, not
testified.

## Recovery ledger (what persisted vs what was re-landed)

Persisted in the restored worktree (verified, kept):
- `emit.rs`: `IndexedArm` + `indexed_arm` shared predicate + relaxed x86-64
  decider (no %rdx/%r11 check) + XMM mirror + write-confinement proof doc.
- `memory.rs`: store-half `indexed_arm` call + F2 SSE bracket + ACCEPT test
  + F1 contract/XMM-mirror + 4 F2 staging-arm tests.
- `generation.rs`: D9 fixed-point loop inside `retain_indexed_ptr_only_uses`
  + `indexed_retain_alias_tests`.
- `tests/regression/check_indexed_fold_scratch_index.sh` (S13 gate).

Re-landed this session (all green: check/clippy/fmt/unit/e2e/mutation):
- `memory.rs`: REFUSE→ACCEPT flip (the restored tree still carried the
  REFUSE test) + F1 contract/XMM-mirror + F2 staging-arm tests re-spliced.
- `common.rs`: F2 classifier (25→28 mnemonics, see below) + window
  methods + classifier tests.
- `generation.rs` B3: `idx_dead_chain` state field + walk-move + skip-site
  + remat `debug_assert!` + soundness tests.
- `traits.rs`: fail-loud `indexed_fold_ok` default + RISC-V/AArch64 N/A
  note + import trim.
- `decimal.rs` F9: `EncodeOutcome` enum (7 sites + 3 callers) + entry
  `debug_assert!` + zero-spelling canonicalization (`int_digits(0)`,
  `const_to_bid` funnel, `digs(0)`) + 5 tests.
- `memory.rs` load half: rewired to `indexed_arm` (the last hand-copied
  type list + shift guard; the emit.rs "three lists are gone" doc is now
  true). Behavior-equivalence proof in session notes; 45 indexed tests
  green across the rewire.
- `tests/regression/check_decimal64_indexed_fold.sh` +
  `tests/regression/decimal64_indexed_fold.c`: RECREATED (untracked pair
  lost in the restore; convergent with the 09-27 spec: scale-8 SIB movsd
  + exact-sum differential vs GCC).
- `scripts/check_decimal_const_agrees_with_gcc.sh`: hardened (mktemp+trap,
  VALUE_CASES single-sourced via env) + 3 F9 e2e cases (z8/s6/z3).
- `ci.yml`: mirror steps for both gates (parity PASS, 79 commands).
- Scratch gate narrowed: the `leaq` ban is now the remat signature
  (`leaq -?[0-9]*\(%(rsp|rbp)\)`) — MUT2 (refuse-all) still trips it with
  the exact pre-fix `leaq 8(%rsp), %rcx`.
- `cargo fmt --all` clean repo-wide (incl. 3 pre-existing
  `ir/lowering` files dirty under rustfmt 1.9.0).

## New catches (red-team, all fixed + tested)

1. **F2 empty-window panic (live bug).** `fp_store_value_xmm` early-returns
   (emits nothing) for XMM-homed values; `split_inclusive` yields one
   empty fragment for an empty window, which tripped the unterminated-line
   panic. Fix: skip empty fragments + `window_empty_is_clean` unit test.
2. **F2 GPR arm unbracketed (doc over-claimed "calls").** Only the SSE arm
   was windowed. Full transitive cone audit (operand_to_rax,
   value_to_reg_inner, emit_imm_to_gpr, emit_global_addr_into_reg,
   rematerialize_stale_into_rax, operand_to_reg, operand_to_rcx,
   operand_to_callee_reg, rematerialize_stale_into,
   emit_fp_operand_to_xmm) before bracketing: cone ⊆ table EXCEPT the
   over-aligned vec-spill triple {movdqu, vmovdqu, vmovdqu64}, which is
   tabled (always `(%rax)`-destined in-cone, hence clean). GPR arm now
   bracketed; stale "same window" test comment corrected.
3. **Window unit tests** (`window_over` harness): empty-clean,
   clean-lines-pass (incl. the vec triple), write-panics, unknown-
   mnemonic-panics, truncated-mark-clean. 9/9 `sib_scratch` green.
4. **Compare-replay is NOT in the staging cone** (audit correction):
   `emit_int_cmp_replay_insn` runs at Select/CondBranch positions, never
   under `rematerialize_stale_into_rax`. No cmp/setcc/jcc can reach the
   window; the table needs no control-flow vocabulary.

## Validation summary

- `cargo check --all-targets`: clean. `cargo clippy --all-targets
  --profile fastbuild --locked -j 2 -- -D warnings`: green (1 pre-existing
  hex-grouping fix). `cargo fmt --all -- --check`: clean.
- Unit: 45 indexed + 28 decimal + 9 sib_scratch + 14 contract (post-restore)
  + 51 staging — all green.
- E2E: S13 scratch gate PASS; D64 gate PASS (`movsd (%rbx, %r14, 8)`
  folded; sum `0x318000000000BB80` agrees with GCC); D64 battery
  PASS=107 FAIL=0 ROD-DIVERGENCES=0 (s6=1.5e-102DF → 0x2 agreed by GCC).
- Mutation: D64-refuse ⇒ D64 gate FAILS structurally (no scale-8 movsd);
  decider refuse-all ⇒ scratch gate FAILS on `leaq 8(%rsp), %rcx` ×2.
  Both restored byte-identical; gates + contract tests re-greened.
- `ci_local.sh --fast`: <VERDICT PENDING — fill before close>.
- `check_ci_gate_parity.py`: PASS (79 commands).

## Tooling lessons (future agents)

- NEVER batch multiple `edit_file` calls to the SAME file: same-file
  batches are LOSSY (1-of-3 survived in state.rs; ~4-of-12 in decimal.rs).
  One file per batch, or better: single python splice scripts with
  `count == 1` assertions + post-verify greps.
- NEVER trust one read after a write: two files showed duplicated-tail
  corruption (decimal.rs, generation.rs). Always re-read + `git diff`
  after edits; suspect the snapshot layer on impossible bytes.
- Tool results RENDER backslashes doubled: `od -c` showing `\\n` means
  `\n`. Verify regex/escapes by execution, never by eyeballing.
- `grep -E 'movsd .+\(...'`: `.+` demands ≥1 char — a no-displacement SIB
  needs `.*`. (Lost 20 minutes to this + the rendering above.)
- Cold `cargo check/test` at default jobs with zero swap wedges the box
  (load 10+, bash dead). Always `ensure_swap.sh` + jobs=1 first;
  `~/.cargo/config.toml` now pins jobs=1 (scripts override explicitly).
- The repo tracks NO +x modes (all 100644); harnesses invoke via `bash`.
  Match it: new `.sh` files stay 644. (CI's `./scripts/...` lines are
  suspect — see TODO.)
- Background `cargo check` overlapping edits yields STALE errors; never
  chase them — recheck after edits settle.

## TODO for future agents

- CC-O0CALL-1 (queued, untouched this session).
- CI `./scripts/build_lccc_fast.sh` vs 644-tracked scripts: determine how
  hosted CI executes these (or fix the invocation); local builds use
  `bash scripts/...` + a temporary +x on `ensure_swap.sh` (restored).
- Debug-assertions e2e for the F2 GPR window: the guard fires only in
  debug builds; fastbuild gates can't trip it. The window unit tests +
  cone audit carry it locally; confirm PR-#639-style debug CI steps
  compile indexed-store-heavy code (stencil/bench corpus) so the bracket
  is exercised hosted-side.
- Seg-override indexed consumers (09-27 item (iii)): still remat
  unconditionally; the B3 assert will fire loud if a dead chain meets
  one. Fix when/if: keep seg-consumed chains live.
- Stale index homes at the access (09-27 item (ii)): dynamic, undecidable
  at skip time; B3 assert is the tripwire. Silent on corpus to date.
