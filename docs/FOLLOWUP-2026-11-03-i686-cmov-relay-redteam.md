# i686 ↔ x86-64 cross-pollination, red-team audit, and CI reconciliation (2026-11-03)

This note records the deep red-team audit of the two i686 peepholes transplanted
from x86-64 in PR #1095 — the **select-diamond → cmov fold**
(`fold_select_diamond_cmov`) and the **load→copy relay**
(`fold_load_copy_relay_i686`) — and the fixes that took the PR's GitHub CI from
red to green. Base: `ms178/lccc` main `9bed91b` (PR #1093 merge). Oracle:
**GCC 16.2** (`gcc-16`), GNU as 2.47 (`--32`) for encoding parity. Host: 2-core
research VM, 4 GiB swap, `target/fastbuild` profile (`-O1`, `-j2`).

Session commits on `arena/01c0bd36-lccc`:
`8d4733a` (the four code/test red-team fixes), `967eeb9` (the CI contract
reconciliation), atop `c6f7209`/`7ffb661` (the original transplant, rebased).

Validation gate (all green before push):
- `cargo test --profile fastbuild --lib`: **7893 passed, 0 failed, 10 ignored**.
- `check_i686_select_cmov.sh` differential corpus: **ALL PASS** at `-O0/-O1/-O2/-Os`,
  8-byte checksum `e096a80ecd171000` with `lccc == gcc-16 == folds-disabled`,
  fold-fires witness `cmov` count on=4 / off=0.
- `check_bit_select_fold`, `check_parity_and_popcount`, `check_affine_value_bound`,
  `check_late_const_gep`, `check_loop_memset`, `check_symver_indirect_fold`: green.

---

## 0. What shipped

| # | Finding | Layer | Severity | Status |
|---|---------|-------|----------|--------|
| 1.1 | i686 encoder rejected every cmov condition alias and `cmovnl` | assembler dispatch | latent miscompile-as-error | fixed (GAS parity) |
| 1.2 | select true-label census was function-bounded | peephole soundness | latent live-label deletion | fixed (module-wide) |
| 1.3 | relay starved `fold_reg_copy_idioms` Pattern 7 | pass interaction | **active miscompile-grade regression** (3 insns vs 1) | fixed (deference guard) |
| 1.4 | implicit-operand oracle completeness for the relay window | audit | none (verified complete) | proven + regression tests |
| 2.1 | `par32` i686 pin stale after the relay win (13 → 10) | test contract | CI red | pin updated |
| 2.2 | bit-select monotonicity row confounded by the relay | test contract | CI red | isolated (passes held constant) |
| 2.3 | freestanding `_start` fixture linked by the hosted corpus | test harness | CI red | moved to a subdirectory |

---

## 1. The red-team findings

### 1.1 The i686 cmov dispatch was incomplete vs. its own condition parser

`cc_from_mnemonic` (the single source of truth shared by jCC/setCC/cmov) accepts
all 30 GAS condition spellings — `pe`/`po`, `nae`/`nb`/`na`/`nbe`,
`nge`/`nl`/`ng`/`nle`, the `z`/`nz` and `c`/`nc` twins. `encode_cmovcc` already
strips the width suffix and validates through that parser, and its suffix guard
only strips a trailing `w`/`l` when the remaining stem is a valid condition, so
the bare ambiguous `cmovl` (less) and `cmovnl` (not-less = ge) keep their `l` as
the condition and infer width from the destination.

But the **dispatch table** in `i686/assembler/encoder/mod.rs` listed only the
canonical spellings. So `cmovpel`, `cmovnael`, `cmovngel`, … and even the
legitimate `cmovnl` fell through to `unhandled i686 instruction`. The x86-64
backend accepts all of these (`mod.rs` "Additional cmov variants with size
suffix" + the suffix-less alias arm), so this was a genuine cross-backend
inconsistency, and the new select fold can emit any condition the codegen's jCC
uses.

**Proof before the fix:** a new encoder test encoding all 30 `l`/`w` alias forms
failed with `unhandled i686 instruction: cmovnael` (and `cmovnaew`, `cmovnl`).
**Oracle:** GNU as `--32` accepts all 30 suffixed aliases; `cmov{cc}l %ebx,%eax`
= `0f (40+cc) c3`, the 16-bit form = `66 0f (40+cc) c3`.

**Fix:** replace the explicit list with one prefix guard
`m if m.starts_with("cmov") => self.encode_cmovcc(ops, m)`. This is purely
additive (turns errors into successes), keeps `cc_from_mnemonic` as the single
validator (a typo like `cmovfool` still fails closed), and is safe for the
ambiguous `cmovl`/`cmovnl` because of the existing suffix guard. `fcmov*` (x87)
is unaffected — it does not start with `cmov`. New tests pin all 30 aliases ×
{l, w} to the GAS bytes, the ambiguous suffix-less forms at both widths, and the
fail-closed rejection of an unknown condition.

### 1.2 The select true-label census must be module-wide

The fold **deletes** the true label (`.Lsel_true_N`). GNU as `.L` labels are
file-scoped and the id is unique across the module (one `next_label_id` counter
on the shared codegen state, `backend/common.rs`). A function-bounded census
would miss a cross-function branch into the true arm — or an address-taken
`.long .Lsel_true_N` in a jump table — and silently drop a live label. The census
now scans `0..infos.len()`. Shape matching stays per-function; only the reference
census widened. New test: a second function `jmp .Lt` into the true arm vetoes
the fold.

### 1.3 The relay starved Pattern 7 (the one active regression)

`fold_reg_copy_idioms` Pattern 7 rewrites
`movl %A,%B; subl $a,%B; cmpl $b,%B; jCC(ZF/SF)` → `cmpl $(a+b),%A`, after which
the memory-operand fold retires the slot load — **one** instruction. It runs in a
late phase (Phase 8). The relay runs in Phase 2/3 and uses exact CFG liveness
(`GprLiveness::dead_after`), so on the range-check shape
`movl 4(%esp),%esi; movl %esi,%eax; subl $48,%eax; cmpl $9,%eax; je …` it proved
`%esi` dead, retargeted the load to `%eax` and deleted the copy — starving
Pattern 7 and stranding `subl $48,%eax` (an immediate ALU op cannot absorb a
memory operand), leaving **three** instructions instead of one.

The older Pattern 2a in `combined_local_pass` does adjacent load→copy retargeting
too, but its *bounded* deadness scan stops at the `je` barrier and refuses, which
is why the shape survived to Pattern 7 before the relay existed. The relay's
exact liveness is what exposed the interaction.

**Fix:** the relay refuses when the copy is immediately followed by
`subl $imm,%D` (`parse_subl_imm_reg`), deferring to Pattern 7. This is the only
*harmful* conflict: Pattern 6 (`movl %A,%B; cmpl X,%B`) reaches the same optimum
through the relay + memory fold, and Pattern 11 (and+jcc) is dormant. New tests:
the relay refuses the copy+subl shape; the full-pipeline `add` win is preserved;
`fold_reg_copy_sub_cmp_fuses_for_equality` (the PR #1093 regression test) is green
again.

### 1.4 The implicit-operand oracle is complete for the relay window

The relay breaks its forward window on any mention of the scratch `%T`
(explicit names ∪ `classify_implicit_operands`). The audit verified the oracle
enumerates **every** i686 instruction that touches a GP register implicitly
without naming it: `cltd`/`cdq`, `mul`/`div`/`imul` one-operand, the string ops
(`movs`/`stos`/`lods`/`scas`/`cmps`/`ins`/`outs`), `cpuid`/`rdtsc`/`rdtscp`,
`pusha`/`pushad` → `(ALL, ESP)`, `popa`/`popad` → `(ESP, ALL & !ESP)`, `leave`,
`enter`, `cmpxchg`, `xlat`, `lahf`/`sahf`, the `syscall`/`sysenter` family. The
`_ => (0, 0)` default therefore only ever fires for explicit-only, XMM-only, or
non-existent mnemonics — never a hidden GP touch. Inline asm is a barrier. New
regression tests feed `cltd` (scratch `%edx`) and `movsl` (scratch `%esi`) into
the window and assert the relay refuses, guarding the oracle's completeness
against future table edits.

---

## 2. CI contract reconciliation

The transplant is a code-quality win, and five contract gates pinned the
pre-win shape or its CI wiring. Each was reconciled, not weakened:

- **`par32` i686 13 → 10.** The relay retired the callee-saved staging copy; the
  surviving `movl %eax,%ecx` feeds a caller-saved scratch that dies in the XOR,
  so no `pushl`/`popl %ecx` pair remains. Runtime parity still agrees with the
  model (`failures=0`). The pin now guards against re-introducing the staging
  pair.
- **bit-select monotonicity.** `cross_length_check` asserts the selected (btl)
  sequence is never longer than the unselected fallback. The relay improves the
  *fallback* independently (btest32 OFF 16 → 10 by retargeting the staging copies
  and shrinking the frame); the btl ON build has no copies to relay and keeps its
  callee-saved frame (12). The row now holds `CCC_NO_I686_SELECT_CMOV` /
  `CCC_NO_I686_LOAD_COPY_RELAY` constant (off) on both sides, measuring the
  bit-select fold alone — the relay has its own differential gate.
- **freestanding `_start` fixture.** Both corpus runners glob
  `tests/regression/*.c` non-recursively and compile each file *hosted*; a
  root-level `_start` TU collides with `crt1.o` (`multiple definition of
  '_start'`). The fixture moved to `tests/regression/i686_select_cmov/`, outside
  the glob, per the convention documented in `run_regression.py`
  (`corpus_contract_hint` / `discover`). The gate script is its only consumer.
- **orphaned gate script (the real shard-b failure).** The first re-run still
  failed `Fast local contracts (shard b)` in 1 m 2 s. The oracle `RuntimeError:
  offline` / `unknown entry ids: ['typo']` lines in the annotation are *expected
  stderr* from `check_oracle_delta_gate.py`'s passing self-test (7 tests OK);
  the actual failing command was `python3 scripts/check_ci_gate_parity.py`.
  That gate refuses any `tests/regression/check_*.sh` wired into neither
  `ci_local.sh` nor a hosted workflow nor the allowlist — and the new
  `check_i686_select_cmov.sh` was an orphan. Fixed by registering it in all
  three mirrors exactly as its siblings: a `gate "i686-select-cmov" fast`
  entry in `scripts/ci_local.sh`, the same `env CCC=target/fastbuild/lccc
  GCC=gcc-16 bash …` command in `ci.yml`'s `backend-contracts-a` run body, and
  a `{measured_s: 1.1, name: …}` record in `ci-lane-plan.json` shard a (measured
  1.07 s; shard a 525.59 → 526.69 s, still within the 5 % balance tolerance of
  shard b's 531.04 s, `predicted_gate_seconds` updated to match). Both
  `check_ci_gate_parity.py` and `check_ci_lane_plan.py` now PASS locally.

---

## 3. Follow-ups (quality opportunities, not blockers)

1. **btl base memory-folding (i686 bit-select ON build).** btest32 ON keeps a
   callee-saved frame because the base load (`movl 16(%esp),%ebx`) is separated
   from `btl %esi,%ebx` by the index load, and `fold_memory_operands` folds only
   *adjacent* load→consumer; the btl index cannot be a memory operand. Folding
   the base across the intervening index load (or a cost-aware bit-select
   suppression when the relayed fallback is shorter) would bring ON to ~9 and
   remove the frame. This is a separate optimization from the cmov/relay work.
2. **Flag-chain producer admission for the clamp diamond.** The first clamp
   select still needs a producer admitted beyond cmp/test (the second folds via
   `cmovnel`).
3. **Frame-to-zero renumber for `add`.** The residual `subl/addl $12,%esp`
   survives because dead-frame elision refuses when param loads mention `%esp`;
   a displacement renumber (−12) would recover GCC's frame-free `add`.
4. **Remaining x86-64 donors** (abs_cmov, return_home, copy_coalesce,
   identical_blocks, lea fuses, narrow_copy_fold, pow2_cmov, relay_and_lea) and
   the i686→x86 `else_hoist_diamonds` port — see `BACKEND-SWOT-2026-11-01.md`.

## 4. Harness notes

The harness wiped the toolchain cache, swap, `target/`, **and reset the branch
HEAD to the original clone commit `dfb902b`** between turns, preserving the
worktree. Recovery: `git fetch origin`, then
`git reset --mixed origin/arena/01c0bd36-lccc` to re-align HEAD with the pushed PR
branch (the worktree already held its content plus the new fixes, so only the
genuine deltas remained). Swap recreated with `LCCC_SWAP_SIZE=4G`, toolchain via
`restore_agent_toolchain_cache.py --only rust,gcc,clang,binutils,mold`, bins via
`cargo build --profile fastbuild --bins` (~3.5 min cold). Snapshot
`S01-s23-pr1095-ci-green`: `/home/user/ms178-1.patch` (59073 B, APPLIES-CLEAN,
UNGATED), base `9bed91b`, head `967eeb9`.
