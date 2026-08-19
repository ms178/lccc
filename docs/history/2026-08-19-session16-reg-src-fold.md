# 2026-08-19 (session 16) — codegen register-source fold + deferred-slot root-cause

**Base:** `origin/main` @ `8cffc688` (Merge PR #129 — session 15 peephole liveness fix)
**Snapshot:** `/home/user/ms178-1.patch`

## Re-base & verification

Sessions 11–15 are all upstream (verified via `ls-remote` + code markers):

| session | content | upstream |
|---|---|---|
| 11 | load→cmp mem fold, sign-ext | PR #127 `5651e8e9` |
| 12+13 | no-op cast coalescing, dead-value regs, load→widen-cast coalesce, acc-cache | PR #128 `43a5ae54` |
| 15 | memory-fold liveness guard | PR #129 `dc02dcfa` |

`main` is reset to `origin/main` and clean; this session's delta is the three
i686 codegen files + the new regression test.

## The structural fix: fold register-source staging in codegen

The i686 general-case binop/cmp path staged a REGISTER-resident rhs through
the %ecx scratch (`movl %reg,%ecx; op %ecx,%eax`) even though the register
assignment is authoritative — the linear scan guarantees %reg holds the value
throughout its range. The new `direct_reg_src_ref` returns the register name
and the emitters use it in place (`op %reg,%eax`), an identical read with one
instruction fewer. Variable shifts (need %cl) and div/idiv with a %edx divisor
(high half of the dividend) still stage, as before.

**Slot values are deliberately NOT folded in codegen** — and that is the
root-cause finding of this session:

## The deferred-slot root cause (why slot folding must stay in the peephole)

I first tried folding slot operands too (`movl SLOT,%ecx; op %ecx,%eax` →
`op SLOT,%eax`), reasoning the memory read is identical. The fuzz caught it
(seeds 34/140/215/236…). Root cause: **deferred/coalesced values get their
slot assigned lazily** (`finalize_deferred_slots`), and a copy-coalesced value
is read through its copy's slot, not its own. A direct `get_slot` read at
emission time can therefore hit a stale, not-yet-materialized location — the
old `operand_to_ecx` staging is resolved at a point where the correct slot is
known, and the *peephole* folds it only after the whole function's memory
operands are materialized. Folding slots in codegen (without replicating the
deferred-slot lifecycle) is unsound. Registers have no such ambiguity, so the
register-source fold is the sound, structural subset.

## Numbers (linux-6.18.44 arch/x86/boot, -m16 -Os)

- total object text **33526 → 33481 (−45)**, no object regressed:
  printf −3, video −18, string −6, early_serial −9, edd −3, cpu −3, main −3.
- 918 unit tests, 50/50 correctness, **343**+6 regression (new
  `check_reg_src_fold.sh`), **750-case** i686 differential fuzz clean,
  x86-64 output byte-identical (only i686 files touched).

## Remaining (documented, not shipped)

1. Slot-operand folding in codegen requires replicating the deferred-slot
   lifecycle (materialize the value's *live* slot before reading it). The
   peephole already does this correctly post-hoc; a codegen version would need
   the copy-coalescing read-redirection map exposed to the emitter.
2. `%eax` allocatability + caller-saved-first Phase 2 ordering (the remaining
   push/pop tax).
3. GEP-address folds (`movl BASE,%ecx; addl %ecx,%eax` → `addl BASE,%eax`) —
   currently peephole-only.
