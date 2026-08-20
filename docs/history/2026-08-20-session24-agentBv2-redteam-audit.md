# 2026-08-20 (session 24) — red-team audit: Agent B v2 vs this revision + the ULTIMATE revision

**Base:** `origin/main` @ `6d2ac62` (identical for both revisions).
**Method:** both revisions applied on identical clean clones, built with the
same fastbuild profile, measured with IDENTICAL corpus/boot scripts and the
FULL battery (352 regressions + 4 fuzz suites — not the 23-regression subset).

## Head-to-head (identical measurements, this session)

| Metric | THIS revision | Agent B v2 | Winner |
|---|---|---|---|
| Boot corpus (19 files, −Os −m16, same script) | **31 218 B** | 31 218 B | tie |
| Boot object text (setup gate) | **32 579 B** | 32 579 B | tie |
| gcc reference (same script) | 14 669 B | 14 669 B | — |
| Unit tests | 959/959 | (not re-run by them) | — |
| Correctness | 50/50 | (23-reg battery) | this |
| Regression (352-test battery) | **352 + 6 known** | **351 + 7 (fp_die_at_birth BROKEN)** | **this** |
| m32 differential fuzz (900) | 900/900 | 900/900 | tie |
| alias adversarial fuzz (180) | 180/180 | 180/180 | tie |
| slot-RMW fuzz (750) | 750/750 | 750/750 | tie |
| simd_sse2_arith | green | green (by luck of scope) | this (see below) |
| fp_die_at_birth | **green** | **MISCOMPILES** | **this** |

**Verdict: identical code-size outcomes; this revision is strictly better on
soundness and validation coverage.**

## Why the size metrics tie

The size-effective deltas in both revisions are the SAME passes with the same
semantics: redundant_loads (correct-polarity retain), quadratic_sr, the alias
engine, the unary-RMW implicit-writes fix, the folded-index live-extension.
Different implementations, byte-identical output: measured 31 218 / 32 579 on
both trees with the same scripts. Neither revision found size the other
missed.

**Agent B's claimed "Boot corpus: 30003 vs gcc 13327 (2.25×)" is not
comparable** — their gcc reference (13 327) differs from the identical-script
gcc reference (14 669), i.e. a different file set/flags. On the identical
script both revisions measure 31 218 vs 14 669 (2.13×).

## Soundness deltas (where this revision wins)

1. **GVN F32/F64 load CSE.** v2 re-enables it ("verified empirically" —
   against a 23-regression battery that lacks fp_die_at_birth). Measured this
   session: v2's tree miscompiles fp_die_at_birth (chain_div returns
   chain_neg's value, 375.000999497 vs gcc's 2399.999680158). Root cause found
   this session: the CSE-created FP Copies perturb the die-at-birth FP
   register coalescing of unrelated blocks (copies created in the load-heavy
   energy loop steal the accumulator of the pure-computation chain_div loop).
   This revision keeps FP out of CSE with the root cause at the site; the
   proper fix (copy-aware FP chain coalescing) is specified below.

2. **folded_index_uses gating.** v2 gates on
   `available_regs.any(|r| (32..=55).contains(r.0))` — a PhysReg-id sniff.
   x86-64's XMM pool is PhysReg 20..=33, so on any x86-64 function with
   scalar FP the heuristic reads "ARM pool present" and enables the
   ARM-only live-extension on x86-64 (where the indexed GEPs REMATERIALISE —
   the extension only perturbs allocation). It only "fixed" their
   gpr_leaf regression by accident (that function has no FP). This revision
   gates by an explicit per-backend parameter (arm passes the links, x86-64/
   i686/riscv pass empty) — correct by construction, no id sniffing.

3. **Validation battery.** v2's addendum reports "23 regression" tests; this
   revision runs 352 (their battery cannot catch fp_die_at_birth, which is
   exactly what slipped). Coverage is a soundness control, and it caught a
   real regression v2 shipped.

4. **quadratic_sr soundness claim.** v2 still ships the false "bit-identical
   including wraparound" claim (counterexample t=2^16: direct 32768 vs
   recurrence 2147516416). This revision carries the corrected analysis
   (exact below 2^N; legal via signed-overflow UB like GCC; divergent under
   explicit -fwrapv) and the differential regression.

## The ULTIMATE revision advances (new this session, in this tree)

1. **x64 lever-1 expanded from Return-only to a per-class audited set.**
   Implemented `CCC_X64_NOHOME_CLASSES` and audited EVERY consumer class
   against the 352-test battery:
   - **ret, store, copy, cast, unary, binop: 352+6 clean each → ENABLED
     (new default)** — six previously-disabled classes now properly on.
   - **cmp: 351+7 (simd_sse2_arith SIGSEGV/miscompare, nondeterministic
     checks 11/13) → scoped out with root-cause analysis.** Mechanism: cmp
     consumers interact with COMPARE-REPLAY (operands re-read at a later
     consumer) and FLAG-FUSION (boolean materialisation skipped); a home-less
     LHS only lives in %rax at the producer point, and the single-entry acc
     cache cannot prove survival to a deferred read. The cmp emitter paths
     themselves are cache-correct (verified by inspection of all four
     `emit_int_cmp_insn_typed` branches) — the breakage is the deferred-read
     contract, not the read site. Proper fix = multi-entry acc tracking or
     replay-aware home retention; specified below.

2. **GVN FP CSE root cause identified** (see above) with the fix design:
   copy-aware FP chain coalescing (the die-at-birth coalescer must treat
   GVN-created `Copy{F64}` as chain-transparent instead of chain-breaking,
   gated on the copy's src/dest being the same value-number).

3. **v2 audit verdicts recorded** (this doc) with per-item evidence.

## Remaining specified work (precise, no hand-waving)

1. **cmp class for x64 nohome**: make COMPARE-REPLAY home-retention aware —
   when a Cmp enters cmp_replay and its LHS is in the nohome set, either (a)
   replay reads the LHS via a scratch reload captured at the ORIGINAL cmp
   point (emit `movq %rax, %tmp` there), or (b) exclude replayed-cmp LHS
   values from the nohome set at allocation time (detectable: cmp_replay is
   computed in the prologue BEFORE the nohome gating — wire the set through).
   Option (b) is the small, provable step; (a) unlocks the full class.
2. **FP CSE**: copy-aware die-at-birth coalescing (design above), then
   re-enable F32/F64 load CSE with fp_die_at_birth + a new FP-CSE fuzz
   scenario (two FP loops, one load-heavy, one pure-computation) as the gate.
3. **SIB-indexed addressing for i686** (session-23 spec, unchanged): est.
   −300..−500 B boot; the dominant remaining boot gap stays the
   materialisation-policy allocator (multi-session).

## Validation (ULTIMATE tree, this session)

- unit 959/959 · correctness 50/50 · regression 352 + 6 known gcc-14 set
- m32 fuzz 900/900 · alias fuzz 180/180 · slot-RMW fuzz 750/750
- fp_die_at_birth green · simd_sse2_arith green · gpr_leaf green
- corpus 31 218 B · boot objects 32 579 B (identical to v2 by measurement)
