# 2026-08-19 (session 14) — re-base + workspace cleanup + structural-allocator investigation

**Base:** `origin/main` @ `33dd2117` (Merge PR #128 — sessions 12 **and** 13 landed)
**Snapshot:** `/home/user/ms178-1.patch`

## Re-base

The harness again reset the git object store to the session-8 commit while the
artifacts bundle retained the full history. Verified against a fresh
`ls-remote` that upstream `ms178/lccc` main is `33dd2117`:

```
33dd2117 Merge pull request #128
43a5ae54 i686: coalesce no-op casts, deny registers to folded values, sharpen sign-ext tracking
a8844c03 Merge pull request #127   (session 11)
```

PR #128's squash tree (`041b793b`) is **byte-identical** to this line's
session-13 final tree — the user's merge took sessions 12 *and* 13 together.
Every fix is therefore upstream; local `main` is reset to `origin/main` and is
clean. The deliverable patch is the delta `33dd2117..HEAD`.

Validated present in the re-based tree (all now tracked upstream):
`build_folded_value_set` (generation.rs), `unique_load_def_types` (regalloc.rs),
`eliminate_redundant_sign_ext` (peephole.rs), acc-cache `set_acc` (emit.rs),
`check_u32_i32_cast_no_relay.sh`, `check_load_widen_cast_no_relay.sh`, and the
session 9–13 docs.

## Workspace cleanup

Removed obsolete cruft (all superseded or rejected long ago):
- worktrees `/home/user/ws-baseline` (pre-Grok baseline), `/home/user/ws-bagent`
  (Agent B line), and the prunable `/tmp/baseline`, `/tmp/bisect-liveness`,
  `/tmp/pre-grok`.
- `/home/user/uploads/` (Agent B's rejected patch).
- extracted binutils build dirs (`tools/binutils-2.47`, `tools/bu-2.47`),
  keeping only the pinned `binutils-2.47.tar.xz` for the linker oracle.
- superseded per-snapshot patch files in `/home/user/artifacts/`
  (`ms178-1.S*.patch`), keeping the bundle, ledger, tarball, and `.base_ref`.

## Structural investigation (the big-ticket items, taken head-on)

### 1. Within-32-bit Cast hazard relaxation — attempted, measured, reverted

Hypothesis: a within-32-bit integer cast stages its source through %eax and
writes only its own dest register (overlap-protected), so it must be
%ecx/%edx-clean. Measured win: `strlen`/`strchr` loop pointer moved from %esi
into %edx (a push/pop pair dropped; strlen 17→16 insns).

**The fuzz caught it** (`m32_differential_fuzz` seeds 0 & 116 at O2/Os), and I
bisected to a concrete mechanism: a loop-carried value `v8` is a **phi** (phis
are non-`eligible`, so they never coalesce and never register-scan), while the
cast-of-that-phi IS eligible and takes %edx. The select-arm Sub (`v4 - v8`)
then reads a **stale %edx** — an intermediate temp (`v0 + v7`) reuses %edx
between the cast's def and its use. With register coalescing disabled
(`CCC_NO_COALESCE=1`) the code is correct, so the relaxation interacts with the
cast/select coalescing in a way the linear-scan conflict check does not cover.
Reverted; documented here so the fix lands on the right layer (the allocator's
select-arm/phi handling) instead of the hazard whitelist.

### 2. %eax allocatability — plan (the root structural tax)

The correct mechanism is the existing scratch-hazard whitelist extended with an
`%eax` hazard list (a value may live in %eax across only those instructions
provably not touching %eax). The whitelist is small but non-empty (Phi, Alloca,
folded GEP, direct binop/cmp/load/store whose dest is provably non-%eax). The
blocker identified above (select-arm value materialization clobbering a live
caller-saved register) must be fixed **first**, because it will bite %eax even
harder. Work order for the next session:
1. Make select-arm Sub/Cmp materialization register-safe (they read their
   operands only at their own program point; today a temp reuses the register).
2. Add the %eax hazard list + `PhysReg(6)`-style allocation in Phase 2.
3. Validate with the 600-seed fuzz before measuring.

### 3. Porting audit (the code-bloat work on other backends)

- **Load→Cast fold**: x86-64 **already has it** (`load_cast_fold` /
  `folded_cast_dests` / `fold_skip_cast` in x86 emit.rs, the "W2 Load->Cast
  fold" from an earlier session) — the i686 coalescing from session 13 is the
  accumulator-backend analog, so this is effectively ported.
- **Same-width no-op cast coalescing**: correctly 32-bit-gated. On x86-64
  `movl` zero-extends the upper 32 bits, so I32↔U32 is **not** bit-preserving
  there (verified in x86 cast_ops.rs) — the gate must stay.
- **Widen-from-load coalescing**: bit-preserving on every backend (sub-word
  loads sign/zero-extend into the register on x86-64 `movsbq`, AArch64
  `ldrsb`, RISC-V `lb`). Broadening it beyond 32-bit is the remaining port,
  but on x86-64 it must not double-apply with the existing `load_cast_fold`;
  the clean form is a per-backend opt-in like the existing fold flags.

## State

Rebased tree builds warning-free; 180-case i686 fuzz clean at the re-based
commit. No code landed this session beyond the re-base — the structural work
is deliberately deferred to land on the correct layer (per the finding above)
rather than shipping an unsound hazard relaxation.
