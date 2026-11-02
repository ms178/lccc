# S19: rebase onto upstream main (PR #660) + R2 red-team fixes (2026-09-28)

Sequel to `FOLLOWUP-2026-09-28-S15-indexed-decider-arm-d32.md` (S15, delivered
upstream as PR #659) and the S18 R2 line (AArch64 D32 width fix). This session
re-anchored the R2 delta onto latest upstream main and red-teamed it there.
All verdicts below are executed; appreciations of proof are inline.

## Rebase (executed)

- Start: local branch `s17-rebase-656 @ 5a4e1cd7` (S15 replay `bd658afe` +
  R2 `5a4e1cd7`, 6 files); upstream `main` fetched at `827982c7` (merge of
  PR #660: the x86-64/i686 torture campaign on top of PR #659).
- **Upstream merged S15 verbatim, proven**: `tree(bd658afe) ==
  tree(3897eb15)` (PR #659's merge), and the S15 file set vs
  `upstream/main` differs only in `.github/workflows/ci.yml` +
  `scripts/ci_local.sh` (legitimate torture-gate extensions). The carried
  delta therefore collapses to exactly the R2 commit.
- Frankenmix #5 forensics: the worktree showed 203 diffs after a harness
  wipe (174 mode-strips + 30 content). Salvaged to
  `/home/user/salvage-s19-triage.patch`, applied to a scratch worktree,
  `write-tree` matched NO known commit; per-blob `--find-object`: 14 old +
  8 deleted-ours + 8 novel. Hunk review of all 8 novel blobs: rustfmt
  reflows + pre-S13/S15 reverts (including a bogus `stage_safe`
  resurrection) — zero post-S18 work, discarded. Reusable method:
  `git apply --index` to scratch + `write-tree` + `--find-object` +
  per-hunk review.
- Rebase proper: `s19-rebase-660 @ df63e898` = `cherry-pick 5a4e1cd7` onto
  `827982c7`, applied with zero conflicts; the replayed patch is
  hunk-identical to R2 modulo base blob hashes (6 files, +283/-45).
- Environment: toolchain reinstalled (stable 1.98.1 + fmt + clippy, 19 s);
  8 G swap via `scripts/ensure_swap.sh`; `target/` was empty — full
  fastbuild from scratch in 4 m 12 s (`-O1 -j2`, gcc+bfd link).

## R2 red-team audit (executed)

Method: every R2 claim traced to its producers/consumers with code
references; nothing taken on trust. R2 itself (D32 rides the U32 move
tables) survived intact — the audit's yields are the four fixes below
plus the following closed verdicts:

1. **Decimal zero-register stores are bit-exact.** `IrConst::D32(v)` carries
   the RAW BID bit pattern (`u32::from_le_bytes`, `expr.rs:267`,
   `const_eval.rs:114`), so `matches!(c, IrConst::D32(0))` fires only for
   literal all-zero bits — no quantum confusion is possible by
   construction. `IrConst::Zero` for D32 is the zeroinitializer, likewise
   all-zero. `str wzr` / `str xzr` are exact in all cases.
2. **All move-table callers pair consistently.** `str_for_type` +
   `reg_for_type` are consumed as a pair at every site (arg homing
   `emit.rs:2413-2414`, the typed slot/indirect impls in `memory.rs:649+`,
   prologue `IntReg`/`StackScalar`, the generic const-offset paths in
   `traits.rs`) — D32 ≡ U32 at each. The `reg_for_type("x8", ty)` sret
   branch only ever sees `Ptr` (the hidden sret param), so it is
   R2-neutral.
3. **`store_x0_to`'s full-width `str x0` is sound for D32.** Small (4-byte)
   slots exist only for X86_64|I686 (`pipeline.rs:669`); AArch64 slots are
   uniformly 8 bytes, so the store writes the value's own slot and the
   upper 4 bytes are owned zero padding. Every D32 reader is w-view
   after R2.
4. **"D32 is never FP-homed" is TRUE.** FP-pool candidacy is `is_scalar_fp`
   = F32|F64 only (`regalloc.rs:8565`); decimals cannot enter pools
   32-38/40-55. Defense in depth holds regardless: the FP arms are
   type-gated, the GP arms filter FP phys regs, and the `fmov` fallbacks
   are bit-preserving moves.
5. **x86 D32 const paths left alone (out of scope, validated).**
   `comparison.rs`'s `movq`u32-bits shape mirrors the ancient F32 arm
   (GAS accepts the u32 imm32 range; the whole torture corpus proves the
   twin); S15 never touched those files.
6. **No AArch64 decimal-arithmetic miscompile.** Lowering routes ALL decimal
   arithmetic/comparisons through the GCC-compatible `__bid_*` helper
   family (`expr_ops.rs:82`), so the `emit_float_binop_impl` D32-shaped
   `else d0` branch is dead. Confirmed empirically: the R2 test's `.s`
   contains `__bid_addsd3`/`__bid_mulsd3`/`__bid_fixsddi`/`__bid_fixsdsi`
   calls and no decimal ALU.

## Fixes (commit `ed3e1d0b`, executed + validated)

**F1. Arm D32 large-const materialization (correctness).** `operand_to_x0`
emitted a bare `mov x0, #N` for `IrConst::D32` with no range handling —
unencodable past imm16 (GAS hard error). R2's own test program emitted
THREE unassemblable movs (`#838860815`, `#830472217`, `#838860820` = the
1.5DF/0.25DF/2.0DF BID patterns); the structural gate passed anyway.
Fix mirrors I32: ≤ 65535 stays one `mov`, larger goes through
`emit_load_imm64` (movz + at most one movk — optimal; a single `movn`
can never apply to a 32-bit pattern since it would set the zero upper
halfwords to ones). Post-fix `.s`: zero bare movs past imm16, 6
movz/movk. Tests: `d32_indexed_large_const_store_stays_encodable`
(through the indexed store path) + `operand_const_materialization_tests`
(boundaries 0/5/65535; 0xDEADBEEF exact movz+movk strings; 0xFFFF0000
single shifted movz). Red-team note: the first version of the new test
asserted a single-`movn` form and FAILED — the failure proved the
expectation wrong (upper halfwords zero ⇒ movz strategy) and the test
was corrected to the shifted-movz truth. 12/12 green.

**F2. Gate hermeticity (the R2 gate was red on stock runners).** As landed,
the gate exited 1: `#include <stdio.h>` + `-I$(gcc include)` does not
resolve glibc's multiarch/stubs layout for the AArch64 target. The test C
file now declares `printf` manually and the gate passes zero `-I` flags —
identical `-S` output, zero host dependence.

**F3. Gate teeth.** New assertion fails on any bare `mov xN/wN, #M` outside
the single-mov alias window (`M > 65535` or `M < -65536`). Narrowly scoped
to the proven emitter population (movz/movk/movn carry imm16 by
construction and do not match).

**F4. CI parity.** R2 added the ci_local gate without a hosted twin, so
`ci-gate-parity` failed (`hosted CI is missing standalone ci_local
gates: tests/regression/check_decimal32_indexed_fold_arm.sh`). Added the
`ci.yml` run step mirroring S15's. Parity green (81 commands mirrored).

## Validation (executed)

- `cargo test --lib -- backend::arm::codegen::emit`: **12/12 pass**.
- R2 gate with the new encodability teeth: **PASS**.
- `cargo fmt --check`: clean. `cargo clippy --lib --tests`: zero warnings.
- `scripts/ci_local.sh --fast`: PENDING (running at doc-draft time).
- Shim cross-check: lccc `-O1` ≡ gcc `-O0` checksums on the CC repro
  (A464B6F2), validating the differential harness below.

## CC-O0CALL-1 (in progress — campaign rebuilt from scratch)

The csmith toolchain was wiped; rebuilt minimally for differential work in
`/tmp/cc-shim` (scratch, not shipped): API-compatible `csmith.h`/runtime
(IEEE CRC32), 70 generated safe-math wrappers (`gen_safe_math.py`,
deterministic + UB-free, return-left-operand on would-be-UB by design),
and a prototypes header (kills gcc-14's implicit-declaration hard errors
and the truncation games). `interesting.sh` = lccc `-O0` crashes or
diverges while gcc `-O0` is clean.

- **REPRODUCED on the rebased tree**: gcc `-O0` → `checksum = A464B6F2`,
  exit 0; lccc `-O0` → **SIGSEGV (139)**; lccc `-O1` → A464B6F2, exit 0.
  The bug is `-O0`-specific and live.
- Reduction: `prune_funcs.py` (whole-static-function deletion, crash +
  differential oracle, ~10 s/trial) running; PENDING at doc-draft time.
- Standing debug leads (backlog CC-O0CALL-1): `CCC_DEBUG_NOHOME`
  home-freshness reporting, the `-O0` slot-assignment path in
  `stack_layout/`, the Call-lowering argument-order walk. Suspect shape:
  a call argument's pointer base lingers in `%rax` across the next
  argument's evaluation (stale-`%rax` load `mov (%rax),%r8`).

## Open (next session)

- CC-O0CALL-1: minimized repro + root cause + regression test + full CI.
- Refresh `ms178-1.patch` + snapshot tarball + bundle + ledger; present.
