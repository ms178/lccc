# LCCC Follow-up / Kontinuität — Session 88 (2026-08-28, v3 round)

**Scope:** v3 codegen round on the rebased main (PR #268 merged upstream);
four validated optimization families; 32 KiB boot gate wired as CI gate.

**Base:** `origin/main @ a96c684b` (fast-forward rebase — session-87 content
already upstream, `git rebase` dropped all three local commits as duplicates)
**Session commits:** `a680cefe`, `dc3c49a8`, `36db4f08`
**Deliverable:** ms178-1.patch (v3) — 44,862 B, APPLIES-CLEAN on a fresh
clone of `a96c684b`, zero garbage (env-gated traces follow the
CCC_DEBUG_HAZARDS/CCC_DEBUG_RA house convention).

---

## 1. What v3 contains (each individually measured on the corpus)

| # | change | where | corpus Δ.text |
|---|---|---|---:|
| 1 | direct div divisor from register home (incl. reg-homed allocas) | i686 alu.rs / emit.rs | −3 |
| 2 | **IR-level div/rem pair fusion** (one divl serves URem+UDiv) | regalloc.rs + alu.rs + prologue.rs | (part of −79) |
| 3 | birth-skip in the scratch-hazard filters (value spans own def) | regalloc.rs | (part of −79) |
| 4 | peephole: single-operand pure-source rewrites (divl/push/jmp*) | peephole.rs | (part of −79) |
| 5 | zext-RHS i64 fast paths (parser `res = res*base + val`) | i128_ops.rs + prologue.rs | −12 |
| 6 | no-op-cast coalescing from ParamRef sources | regalloc.rs build_coalesce_groups | ±0 (enabler) |
| 7 | **adjacent copy-latch web coalescing** (`flags |= X` → `orl $X,%reg`) | regalloc.rs build_coalesce_groups | −77 |
| 8 | boot-gate CI wiring (pre-pecompat ≤ 24,576) | run_regression_suite.sh | — |
| | **total** | 24,240 → **24,070** | **−170** |

## 2. The div/rem pair fusion (read this before touching it)

`compute_i686_divrem_pairs(func, allow_const_rhs)` — deterministic same-block
analysis shared by emitter and RA (both derive from the same IR, the two
views cannot disagree). A pair is a URem/UDiv (SRem/SDiv) couple with
IDENTICAL operands; the HEAD emits one divl and dual-stores both results, the
TAIL emits nothing. THREE soundness traps were found by the death-signal
torture (11 cases × 20k rounds; fusing on/off; GCC reference):

1. **Tail liveness**: the tail's dest is physically born at the HEAD's
   dual-store. `patch_divrem_tail_intervals` extends tail-dest liveness to
   the head point BEFORE any interval map is derived — otherwise homes/slots
   in the head..tail window get double-assigned (torture cases 2/5/9/10).
2. **Store order**: the two results live in %eax/%edx until stored. Storing
   the %eax side into a %edx home destroys the remainder first. Home-aware
   ordering + pair-BREAKING for the deadlock combos (quo@%edx ∧ rem@%eax;
   slotless-vs-clobber). Broken pairs emit two standalone divisions.
3. **Acc-flow slotless dests**: immediately-consumed values have no home and
   no slot; their consumer reads %eax directly. A slotless pair side must be
   materialised into %eax with a cache entry; the accumulator analysis
   excludes pair tails outright.

Kill switches: `CCC_NO_IR_DIVREM` (table), `CCC_DEBUG_DIVREM` (trace).

## 3. The copy-latch web coalescing (the vsprintf flag parser)

The frontend lowers C `flags |= X` state loops WITHOUT Phi instructions —
the loop-carried variable is a Copy destination with 2+ definitions (the
phi-elim latch form). The phi-congruence machinery only looked at real Phi
instructions, so every `flags |= X` arm paid a 3-instruction relay
(`movl %ebp,%edx; orl $4,%edx; movl %edx,%ebp`) while GCC emits
`orl $4,%ebp`. The fix treats latch Copies whose source is defined by the
IMMEDIATELY preceding instruction as same-value edges. **Adjacency is the
soundness anchor**: with anything between the source's def and the copy, a
use of the OLD incarnation could sit inside the source's live range.
vsprintf's five flag arms are now the 1-instruction form.

## 4. Boot-gate CI (the e12597c7 lesson, mechanized)

`run_regression_suite.sh` now runs `build_kernel_boot.sh` when `KERNEL_DIR`
is prepared and fails on pre-pecompat content end > 24,576. The script's
non-zero exit on a gate FAIL is a measurement, not an error (parsed either
way). Without KERNEL_DIR: honest SKIP. The gate is RED until the .text
budget (≤ 23,380) is met — that is its job.

## 5. Validation

* divrem torture: 11 cases × 20k rounds (fusion on/off + GCC ref) — green
* zext torture: 11 cases × 50k rounds (fast/fallback/GCC ref) — green
* i64 torture: 20k rounds × 27 ops (session-87 harness) — green
* regression suite 467/0, AB-diff 0 · cargo --lib 1205/0/6
* corpus .text reproduced exactly from the deliverable tree

## 6. Environment notes

Unchanged from session 87 (same sandbox class). Torture binaries are
freestanding `-nostdlib -static` (32-bit syscalls blocked by seccomp — the
death-signal verdict: int3=pass, ud2/SIGSEGV=fail-at-case). The torture
sources live in `/home/z/kernel-work/torture/` (divrem_torture.c,
zext_torture.c) — candidates for tests/ in a future round.

## 7. Next-session entry points (priority order)

1. **P0 — i64 mul-accumulate chain fusion**: `res = res*base + val` still
   round-trips the product through slots (4 mem ops/iteration) and stages
   the zext casts (3 dead store insns for the never-read high halves).
   GCC's 9-insn shape is documented in §analysis; needs virtual zext casts
   (never-materialized) + source-liveness extension (folded_index_uses
   pattern) + a mul→add chain table mirroring the divrem design.
   string.c: kstrtoull +154, simple_strtoull +80 vs GCC.
2. **P0 — RA Phase 2h (iterated hazard refinement)**: div points are
   %ecx-clean when the divisor is register-homed (∉ edx/eax); folded
   index-chain GEPs (base=GlobalAddr, offset unhomed) are clean for %edx —
   two fixpoint iterations after Phase 2g would let the number() digit loop
   claim %edx for the remainder and fire the indexed load
   (`movsbl digits(,%edx),%eax` — −5 insns/iteration).
3. **P1 — video.c family** (+363/+224/+169/+160/+114): systematic ~1.5×
   excess, not yet analyzed in depth; likely state-machine + RA pressure.
4. **P1 — value-width map** for Copy/phi dests (byte slots, movb immediates).
5. **P2 — vsprintf body** (+443 excess remains after the flag-loop fix).

## 8. Snapshot ledger

`S02-v3-s88-codegen` — base `a96c684b`, head `36db4f08`, deliverable
APPLIES-CLEAN. Artifacts: /home/z/my-project/lccc-artifacts (+ PolarFS
mirror recommended before handoff).
