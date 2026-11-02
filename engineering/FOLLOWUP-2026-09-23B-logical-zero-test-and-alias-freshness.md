# Follow-up S08 — the zero-test fold, the entry-gate trap, and the workqueue.o ICE

Session date: 2026-09-23. Base: upstream `93596224` (= PR #600's re-push of
`adace81a`; main unchanged this turn). Head after this session: `d253a2b2`
(four commits on top of the S07-rebase tree `076b3370`).

## What landed

### 1. `fold_logical_zero_test` + terminal adjacency sweep (`0f666c8b`)

Deletes the zero-test after a logical op on the same register:

```
and|or|xor{b,w,l}  SRC, %R     followed immediately (skipping nops) by
cmpl $0, %R  |  test{b,w,l} %Rn, %Rn   →   second line deleted
```

- **Flag law (no reader gate — unique in this file):** a logical op clears
  CF/OF and sets ZF/SF/PF from its result; `cmpl $0` over that result
  produces exactly the same flag set. Nothing can observe the difference.
- **Width law:** the test must read exactly the flagged bits — `testl` after
  a 32-bit op, the *same-width* narrow self-test after andw/andb. A wider
  test reads upper bits the logical op left untouched and unflagged
  (refused, pinned). `cmpl $imm≠0` never matches (SF/CF differ).
- **Kill switch:** `CCC_NO_LOGICAL_TEST_FOLD`.

**The placement lesson (an architectural finding, not a one-off).** The
shape's window is opened by *other passes*: the raw stream is
`andl $1,%eax; movl %eax,%esi; testl %esi,%esi` (RA staging copy between);
`fold_reg_copy_idioms` (phase 3.8) owns the copy-crossing rewrite,
`propagate_reg_copies` retargets the reader, phase 4's liveness pass deletes
the dead copy. Wiring the fold only at the global site, only in the changed2
fixpoint, or *inside* the phase-3.8 loop all stayed dormant on the real
pipeline: **phase 3.8's fixpoint has an entry gate and never runs for
functions whose gate folds all decline** — 11 boot-corpus sites survived
exactly this way while hand-written unit tests (final text, adjacent) passed
unconditionally. The general rule now recorded in the journal:

> A pass whose window other passes open must run unconditionally at a
> pipeline point *after the last window-opener* — not only inside another
> pass's gated fixpoint.

Both direct-window folds (`fold_logical_zero_test` + the zext-pair compare
fold) now run unconditionally after the phase-3.8 block and again in a
terminal sweep before Phase 6 renders; neither fold orphans a value, so no
re-fixpoint is needed. Effect: boot corpus 5449→5438 insns on the old tree
baseline, 5032→5021 on the regenerated-tree baseline (−11 = the 11 sites:
a20.s×2, apm.s×4, string.s, video-mode.s, video-vesa.s, video-vga.s,
video.s). Seven new unit pins incl. assembler-routed output.

### 2. The workqueue.o ICE — alias-aware home freshness (`7308462f`)

**Upstream's kernel gate was broken:** every full kernel build from current
main fails at `kernel/workqueue.o`:

```
ccc: internal error: x86 codegen: operand_to_rax: value 103 in function
'llc_populate_cpu_shard_id' cannot be materialised — its register home is
stale (clobbered) with no slot/remat/acc recovery — refusing to fabricate
a value
```

- Reproduced in isolation with the exact kbuild command; proved with a
  worktree build of the pre-change binary that it predates this session
  (introduced with `adace81a`'s RA rewrite). The S07-era "successful" build
  had silently reused a stale `workqueue.o` — make does not track compiler
  changes.
- Source-shrinking (ddmin) fails by design: the trigger is module-order
  dependent (any deletion shifts value ids and hides it). Flag matrix:
  `-O1` clean, `-O2` fires regardless of `-fmodulo-sched`/`-fivopts`.
- `CCC_DEBUG_NOHOME=1` names the whole story in one line:
  `home=Some(3) fresh=false sharers=[107,109,103,265] defined_by=None` —
  a phi-coalesced induction chain (v107→v109→v103) homed slot-less in r13
  (PhysReg 3), whose in-place chain update (`cmovnel %r10d,%r13d`) freshens
  only the newest SSA id and evicts every other live sharer — including the
  older ids of the *same* logical value. The session-26 hard gate refused
  correctly; the freshness bookkeeping simply could not see the equivalence.

**Fix:** the allocator's blessed same-value classes — exactly the set the
overlap verifier unions (slot-coalescing webs + *applied* phi destructive
updates + FP webs; rejected candidates never alias) — are published as
`RegAllocResult::phi_chain` (member → class-minimum representative), and all
six x86-64 read-side freshness gates become alias-aware
(`home_readable_via_alias`): a clobbered mark on `v` is readable when a
non-clobbered member of the *same class* shares the register — coalescing
guarantees both ids denote one value wherever both are live. Write-side
accounting untouched; ARM/RISC-V/i686 receive an ignored empty map and
behave bit-for-bit as before. New pin `phi_chain_published_for_applied_coalesce`.

**Validation:** isolated `workqueue.o` now compiles clean (138720 B, first
time since `adace81a`); full `bzImage` build EXIT=0; QEMU boot 16/16; unit
tests 3228/0; `ci_local.sh --fast` 61/0/3 ALL GATES GREEN.

**Roadmap consequence (RA-3):** any future coalescer — including
RA-3's pressure-aware greedy scan — MUST keep feeding this map with the same
class set the verifier blesses, or the freshness gate starts refusing values
that exist. The gate and the alias map are now one contract.

### 3. Operational notes

- `ci_local.sh` and `build_kernel_vm.sh` must never run concurrently: CI's
  cargo rebuild replaces `target/fastbuild/lccc` while make execs it (the
  first "failure" this session was exactly that). Gates run serially now.
- Kernel-tree regeneration invalidates the boot-census gcc cache; census
  baselines are only comparable within one tree generation (this session's
  baseline: lccc 5021 / gcc 3183, +57.7%).

## State of the i686 RA roadmap (upstream `engineering/subsystems/i686.md`)

| Lever | State |
| --- | --- |
| RA-1 widths-from-context-evidence | landed upstream (−160 B boot) |
| RA-2 phi chaining | landed upstream — **its slot-less shared-home contract now needs the alias map (see §2)** |
| RA-3 pressure-aware greedy scan | open; must publish `phi_chain` from its own coalescing |
| RA-4 `Cmp(and(x,y),0)` and-flag fold | **CLOSED this session** (`0f666c8b`), incl. the copy-crossing windows via the terminal sweep |
| RA-5 −m16 absolute-address stores | open |
| RA-6 copy+subreg-extend fold (`movl %esi,%eax; movzbl %al,%eax` → `movzbl %sil,%eax`) | open — next lever |

## Next steps

1. RA-6 copy+subreg-extend fold on the i686 side (needs an RA-3-style
   residency notion for the narrow source; the terminal-sweep placement law
   applies here too — the window opens when RA stops materialising the
   staging copy).
2. RA-4 slot shape (needs RA-3 pressure model).
3. RA-5 −m16 absolute stores (small, self-contained; do after RA-6).
4. Re-census + upstream PR with `0f666c8b`+`7308462f` rebased onto latest
   ms178/lccc main.

## Verification ledger (this session)

- `cargo test --lib`: 3228 passed / 0 failed / 7 ignored.
- `ci_local.sh --fast`: 61 gates, 0 failed, 3 skipped, ALL GATES GREEN.
- Kernel: full build EXIT=0, bzImage sha `a6a65457e1914bff…`,
  QEMU boot 16/16 PASS.
- Census (regenerated tree): lccc 5021 insns vs gcc 3183 (+57.7%);
  fold-off A/B 5032 (−11 = 11 zero-test sites).
- Snapshot: `/home/user/ms178-1.patch` re-based, head `d253a2b2`
  (see ledger row 12).
