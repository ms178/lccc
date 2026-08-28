# LCCC Follow-up / Kontinuität — Session 90 (2026-08-28, Agent-Z audit + miscompile round)

**Scope:** audit and perfect Agent Z's `fuse_setcc_branch` (PR #274 body);
close the audit with red-team regression tests; resolve every corpus
failure down to root cause; keep the deliverable applies-clean on latest
main.

**Base:** `origin/main @ 2b53a6ec` (PR #273 = v7 peephole `9a540781`,
PR #274 = Agent Z `0fbf334a`)
**Session commits:** `7569cdb0` (fuse_setcc_branch hardening + differential
tests), `7df4089c` (two pre-existing miscompiles surfaced by the corpus)
**Deliverable:** ms178-1.patch — APPLIES-CLEAN on fresh `2b53a6ec`.

---

## 1. Validation state (all measured this session)

| suite | result |
|---|---|
| unit (`cargo test --lib`) | **1258 pass / 0 fail / 6 ignored** |
| regression corpus (`run_regression.py`, regression + benchmark dirs) | **506 pass / 0 fail** / 10 skip-compare (GCC cannot build the oracle — lccc self-checks pass) / 0 skip-run |
| compat_test.sh (GCC differential) | **20/20** |
| boot gate (`build_kernel_boot.sh`) | **PASS, `_end=31168`** (headroom 1600 B), flat setup image **byte-identical** to the ld.bfd oracle |
| mulacc/setCC differential (4 new GCC-oracle tests) | byte-exact vs GCC -O2, fusion ON and OFF (`CCC_NO_MULACC`, `CCC_NO_SETCC_BRANCH`) |

## 2. fuse_setcc_branch — audit closed, hardening landed

The full semantic audit of Agent Z's fusion (1,809 lines) closed with two
code-level defects fixed and locked by tests:

1. **movzwl carrier arm deleted.** setCC writes only the low BYTE of its
   destination; `movzwl %ax` imports AH — garbage — into the bool, so ZF
   would not reflect it. The fusion must refuse that carrier
   (`setcc_fuse_refuses_movzwl_carrier`).
2. **Post-jcc guard made classifier-independent.** The reader check must
   run before the barrier check (a CondJmp is both), labels are skipped
   (fallthrough enters the next block), AND — the new part — the reader
   test consults `is_cond_jcc()`, the authoritative mnemonic list. The
   classifier keys `is_conditional_jump` on the *second* character, so
   `jc` classifies as Other while `jnc` classifies as CondJmp; a `jc`
   after the branch escaped the guard and the fusion fired, handing the
   reader the producer's flags (repro: je→jne rewrite in
   `setcc_fuse_refuses_jcc_reader_after_branch`). The deadness scan now
   also stops at any conditional-jump spelling (its target may rejoin and
   read the carrier).
3. Red-team suite (7 tests): dead window, je inversion, movzwl refusal,
   post-branch jcc reader refusal (`jc` spelling), setCC reader behind
   the fallthrough label refusal, writer-before-reader allowance,
   live-bool window-kept shape. cmpl-headed producers make every refusal
   *observable* (no other pass may drop a cmpl-headed setCC test, and a
   wrong fusion's je→jne rewrite is detectable in the output text).
4. Liveness def-point numbering re-verified end-to-end (per-instruction
   points; mulacc `block_start + ii` defs match exactly) — Agent Z's
   def-point gate is sound.

## 3. Corpus failures — all five resolved to root cause

Starting corpus state on `2b53a6ec` was **501/5** (the earlier "474/0/11"
baseline predates the benchmark dir being swept and the i386 loader being
installed). All five:

1. **sqlite_varint — REAL MISCOMPILE, fixed (`7df4089c`).** MachInst
   two-address lowering (`mov lhs,dst; alu rhs,dst`) reads rhs *after*
   writing dst, but the allocator only guarantees IR read-then-write
   semantics; rhs and dest legally shared %edx and the OR became
   `or %edx,%edx` (sqlite3GetVarint `b |= *p`, every 9-byte varint wrong).
   ISel now enforces the machine constraint: commutative ops swap
   operands, Sub/both-alias falls back to the mature emitter
   (`lower_binop` → bool). Bisection: `CCC_MI_DISABLE_KINDS=binop`,
   `CCC_NO_MACHINST`, `CCC_NO_BLOCK_RELAYOUT`, `CCC_NO_LEAF_PARAM_GPR`
   all masked it; the MI stream contained the OR, the final asm did not.
2. **glibc_f128_builtins — REAL MISCOMPILE, fixed (`7df4089c`).**
   `Intrinsic::result_type()` returned None for F128Fabs/Neg/Copysign, so
   their destinations got 8-byte slots while codegen stores 16 (movdqu);
   the store overflowed into the neighbouring slot and corrupted the
   operand for later ops. `result_type()` now reports F128 for the three
   bit-op builtins.
3. **stmt_expr_asm_typeof — TEST defect, fixed (`7569cdb0`).** It returned
   42 (the asserted payload) instead of 0; both lccc and GCC returned 42.
4+5. **i686_fused_mul_add_operand_order, segment_fill_copy_alias —
   environment, resolved.** `-m32` exes were linked without crt1.o (no
   `libc6-dev-i386` on the host), so main's final `ret` popped argc and
   jumped to 1 — *after* the program computed correctly. They were
   skip-run before (no i386 loader; the 11 baseline skips); installing
   the loader (via gdb's lib32 pull-in) unmasked them. With
   `libc6-dev-i386` installed both exit 0 — and the whole -m32 corpus now
   genuinely executes, which is strictly better i686 coverage.

## 4. New coverage

- `mulacc_chain_u64.c`: kstrtoull-shaped chains with overflow gates,
  wraparound, 2^32/2^63 boundaries.
- `mulacc_sext_addend.c`: sext/negative addend feeders, negative constant
  addends, hi-zero boundary base 0xFFFFFFFF vs rejected >=2^32 bases,
  sign-bit-set u32 feeders.
- `mulacc_nop_cast_sandwich.c`: multi-use no-op cast sandwiches, split
  uses, nested chains, self-referential wraparound (the single-use canon
  rule).
- `setcc_bool_gate.c`: window-dead, window-kept (bool stored), sete/je
  inverted shape, chained selects.

All GCC-oracle compared byte-exact at -O2, fusion on and off.

## 5. Lessons that cost time — do not re-learn them

- The runner sweeps `tests/regression/` **and** `tests/benchmark/programs/`;
  a `.flags` file (e.g. `-m32 -O0`) overrides the `-O2` default — hand
  runs without it reproduce nothing.
- `tests/compat_test.sh` needs plain `LCCC_BIN=./target/fastbuild/lccc`
  (it appends `-I` itself; a raw include dir makes the driver fail).
- gdb's install pulls `libc6-i386` (the loader appears → -m32 runs →
  crt-less links crash at exit); install `libc6-dev-i386` in the same
  breath or the -m32 corpus reports phantom segfaults.
- The MI stream (`CCC_MI_STREAM=1`) proves what MachInst emitted; diff
  *that* against the final asm before suspecting the emitter — the peephole
  phase is innocent in this class of bug.
- Test-expectation pitfalls in this round: another pass may normalize a
  redundant test's operands (`testl %eax,%edx` → `%eax,%eax`) or delete a
  branch-to-next; assert on the *observable semantics* (jne present/absent,
  testl family present), not exact spellings.

## 6. Next-session entry points (priority order)

1. **CCC_PEEPHOLE_SKIP plumbing for the i686 text pipeline** (still
   x86-64-only; peephole_ab.py cannot A/B i686 patterns).
2. P0 levers (memset→rep stosl, video family, census-v2 callee-saves) —
   boot gate has 1600 bytes of `_end` headroom and the gate now reports
   PASS with room; per-function insn excess table unchanged from session 89.
3. Unit tests still missing for P5-sym/P13/P14/threading/redundant-jCC/
   propagate (i686 peephole).
4. RA accumulator machine + allocatable ebx/esi/edi (the big per-function
   excesses: set_video +169, vga_set_mode +137, vsprintf +118).

## 7. Snapshot ledger

`S04-az-audit-miscompiles` — base `2b53a6ec`, head `7df4089c`,
deliverable ms178-1.patch APPLIES-CLEAN on fresh `2b53a6ec` (validated
with `git apply --check`), zero garbage (no trace probes; the temporary
`CCC_SETCC_TRACE` eprintlns were removed before commit).
