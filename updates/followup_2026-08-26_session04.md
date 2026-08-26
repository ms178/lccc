# LCCC Follow-up / Kontinuität — Session 04 (2026-08-26)

**Scope:** deep red-team audit of "Agent Z"'s Linux-kernel bringup patch
(974 lines / 23 files), re-derivation of every hunk against current
`ms178/lccc` main, and integration of the validated subset with fixes.
**Full audit:** `docs/history/2026-08-26-session85-redteam-audit-agent-z-kernel-bringup.md`.

---

## 0. Session environment (reproducibility)

| Item | Value |
|---|---|
| Host | 2 vCPU VM, no HW PMU |
| RAM / swap | 1.9 GiB RAM + 6 GiB swapfile `/home/user/.swapfile` |
| Disk | 25 GiB volume, 20 GiB free at session start |
| Rust | 1.98.0 (repo pin, rustup minimal) |
| Host GCC / ld | 14.2.0 (Debian) / GNU ld 2.44 |
| mold | 2.37.1 (Debian package; used for cargo links via `-fuse-ld=mold`) |
| LCCC build | fastbuild profile (`-O1`, no LTO, incremental, `-j2`), ~100 s rebuild |

> Sandbox wipe note: the harness wiped the workspace once mid-session (turn
> interruption → no end-of-turn snapshot). Everything below was rebuilt from
> `origin/main @ be227056` and is committed + snapshotted via
> `scripts/lccc-snapshot.sh` before this turn ends.

## 1. Audit verdicts (short form)

| # | Agent-Z hunk | Verdict |
|---|---|---|
| 1 | prepare_kernel_tree.sh tar tolerance | AGREE, strengthened (sentinel files + retry) |
| 2 | `invalidate_text_section()` after PFE section | AGREE — bug REAL on main (bodies in `__patchable_function_entries,"awo"`), fixed |
| 3 | mcount `-pg/-mfentry/-mrecord-mcount/-mnop-mcount` | AGREE w/ 3 fixes (naked guard; classic-mcount frame ABI; refusal test) |
| 4 | Add-fold `Cast(GlobalAddr)+reg → leaq sym(reg)` | AGREE (mirrors upstream GEP fold 804ce8c) |
| 5 | MIN_JUMP_TABLE_CASES 4→5 | AGREE; comment REWRITTEN (GCC-parity measured; objtool rationale was false) |
| 6 | movabs 64-bit symbol-diff immediate | AGREE (mem_encrypt_boot.S; reproduced refusal) |
| 7 | `.long sym - (. + 4)` parser probe | AGREE (silent-zero reproduced) + hardened (see §2) |
| 8 | `LCCC_BEST_EFFORT_NO_HOME` zero-fabrication | **REJECTED** (correctness charter) |
| 9 | `_Pragma` rewrite | OBSOLETE — main already rewrote it (C11 §6.10.9 path) |
| 10 | `no_instrument_function` plumbing | AGREE (required by #3) |
| 11 | struct-literal updates | AGREE (mechanical) |
| 12 | cli test update | DISAGREE as given — the refusal test was left asserting `is_err()` for now-accepted flags; rewritten |

## 2. What this session changed beyond Agent Z (red-team deltas)

1. **`-mnop-mcount` prefix trap.** `-mnop-mcount` starts with the literal
   prefix `-mno`; the permissive disable-flag arm would silently swallow it.
   The three sub-mode arms now sit BEFORE the `-mno-` catch-all.
2. **Classic mcount frame ABI.** Agent Z emitted `call mcount` before the
   prologue in ALL modes. Measured GCC emits it AFTER `push %rbp; mov
   %rsp,%rbp` and rejects `-pg -fomit-frame-pointer`. Implemented a deferred
   prologue site (`pending_classic_mcount_label`) + forced frame pointer in
   classic mode. Classic mode is x86-64-only (`supports_classic_mcount`);
   i686 supports fentry/nop/record; other targets warn once (never silent).
3. **Naked guard.** Agent Z's comment claimed naked functions are skipped but
   the code didn't check `is_naked`. Fixed.
4. **Paren-RHS probe guard (REGRESSION FOUND & FIXED).** Agent Z's
   `parse_sym_addend`-first probe, applied unconditionally, flipped the sign
   of unparenthesized trailing addends: `760b - 770b + 5` parsed as
   `760b - (770b + 5)` and broke `kernel_altinstr_layout`
   (neg_addend=-11 vs expected -1). The probe is now restricted to
   parenthesized RHS (`rhs_full.starts_with('(')`) and rejects digit-only
   "labels" (`a - (1 + 2)` must not diff against "1"). Both directions are
   unit-tested.
5. **No new diagnostic zero-fabrication mode** (#8 rejected); the hard gate
   ("value N has no register home") stays loud.
6. **prepare_kernel_tree.sh:** tolerating tar exit-2 with only a top-level
   Makefile check was weak; added 5 sentinel files spread across the tree +
   one retry before hard-failing.

## 3. Validation (all on this tree)

* `cargo test --lib`: **1197 tests, 1191 passed, 0 failed, 6 ignored**,
  incl. NEW `test_static_call_parenthesized_dot_addend`,
  `test_diff_trailing_addend_sign_preserved`, `mcount_flag_family_parses`,
  updated `unimplemented_hardening_is_refused_not_ignored`.
* `scripts/run_regression_suite.sh`: **PASS=467 FAIL=0 SKIP=11**, AB-diff
  failures 0 (pre-fix intermediate run: 1 FAIL = kernel_altinstr_layout,
  root-caused to the unguarded probe, fixed).
* NEW regression scripts (all PASS):
  `check_pfe_section.sh`, `check_mcount.sh`, `check_static_call_pcrel.sh`,
  `check_movabs_symdiff.sh`, `check_switch_threshold.sh`,
  `check_percpu_add_fold.sh`.
* Reference cross-checks: GCC 14.2 switch threshold measured (4 cases →
  chain, 5 → table); GCC mcount shapes measured for all four flag combos;
  GAS cross-checked `.long target_fn - (. + 4)` → `R_X86_64_PC32 target_fn-4`,
  lccc now emits the identical relocation; lccc-ld runtime reconstruction of
  the static_call destination verified pointer-exact.

## 4. TODO / next sessions (prioritized)

### P0 — kernel build continuation
* Full Cachymod 6.18.46 object-file count progression with `-pg -mfentry
  -mrecord-mcount -mnop-mcount` enabled (this session validated the
  assembler/codegen pieces, not a full kernel link).
* objtool pass over lccc-built objects with `__mcount_loc` sections
  (needs the host objtool built; budget ~15 min). Expected remaining warnings:
  jump-table quirk + fallthrough annotations (see kernel-boot followup P2).
* `recordmcount`-less config matrix: verify CachyOS `.config` selects
  `FTRACE_MCOUNT_USE_CC` vs `USE_OBJTOOL` with lccc as CC (compiler-identity
  probes `cc-version.sh` etc. — lccc reports as GCC 14.2.0 clone).

### P1 — mcount fidelity gaps (documented, deferred)
* Emitted-inline functions that survive to codegen (extern inline / address-
  taken static inline) are skipped by the `is_inline` guard; GCC instruments
  them. Kernel impact: some trace sites missing (non-fatal). Fix requires
  distinguishing "inlined away" from "emitted despite inline keyword" at
  codegen time.
* ARM/RISC-V `-pg` currently warn-and-skip. arm64 kernel uses
  `-fpatchable-function-entry` (already supported), so only riscv/arm32
  ftrace builds are affected.

### P1 — codegen quest (from STATE.md gaps, unchanged)
* gzip `longest_match` RIP-relative window addressing; xmltok/inflate
  stack-memory regression; sieve scalar shape (do NOT copy clang).
* Loop rotation remains opt-in; measure default-enable on the benchmark
  corpus before flipping.

### P2 — infrastructure
* `tests/regression/check_*.sh` scripts are not yet picked up by
  `run_regression_suite.sh` (it runs `*.c` only). Wire a shell-test phase so
  the six new scripts run in CI.
* mold oracle: this session used Debian mold 2.37.1 (sufficient for LINKING
  lccc; NOT the benchmark oracle). For linker benchmarking rebuild git-HEAD
  mold + wild per docs/linker addendum.

## 5. Snapshot ledger
See `/home/user/artifacts/SNAPSHOT_LEDGER.md` and `ms178-1.patch`
(APPLIES-CLEAN against base `be227056`).
