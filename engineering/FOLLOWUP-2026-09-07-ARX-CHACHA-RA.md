# FOLLOWUP 2026-09-07 (evening) — rebased rotates: soundness fixes, RA-PRESSURE-1 admission cap

Scope: the state after rebasing the session series onto upstream `main`
`ca1c3b34` (PRs #439 loop-rotate pressure gate, #440 native rotates), the
soundness holes found and fixed in the merged-but-unverified rotate PR, and
the loop-span admission cap built for RA-PRESSURE-1. `ms178-1.patch` is
`git diff ca1c3b34..work-v4`.

## 1. Series on `work-v4` (all gates green at each step)

1. `verifier+vectorize: def-dominates-use verification; fix cloned-remainder base dominance`
2. `verifier: model InlineAsm outputs; vectorize: sound alias-guard bases; loop_unroll: exit-phi entry operand`
3. `ci: run the benchmark output gate unconditionally (INF-BENCHGATE-1)`
4. `passes: constant-fold + copy-prop BEFORE the aggregate SROA split; unroll at -O2`
   — chacha20 10.014× → 2.85× vs GCC (the split only models GEPs whose
   offset OPERAND is a literal constant; folding + copy-prop before the
   split is what turns unrolled offsets into literals).
5. `bit_idioms: soundness — idiom identity must not cross integer casts`
   — see §2. Three live miscompiles fixed.
6. `regalloc: loop-span admission cap for block-local ranges (RA-PRESSURE-1)`
   — see §3. Default OFF; opt-in knob with the full measurement record.
7. `ci: drop a stray blank line introduced with the benchmark gate step`

**Gates at HEAD:** cargo test 2094/0; regression suite 656/0 (AB-diff 0);
IR verify sweep clean; benchmark output gate 180/0; cap-off verified
byte-identical to the pre-cap allocator on generated asm.

## 2. Rotate soundness (PR #440 was merged without a single test run)

Upstream's `IrBinOp::RotateLeft/RotateRight` design (variable amounts,
all four targets, MachInst path, width-aware `rotate_within_bits` folding)
replaces this session's earlier `RolN` design and is strictly better. But
the idiom matchers compared operands with **cast-peeling**: `zext(x)`,
`sext(x)` and `trunc(x)` all peel to `x` yet are different values. Three
live -O2 miscompiles on x86-64, all confirmed against GCC:

  * rotate: `Or(Shl(zext(a),16), LShr(sext(a),16))` folded to `rol a,16`,
    dropping the `0xffff` sign half (sum repro: `7f800000` vs `7fffff80`).
  * SWAR popcount: `popcount_network((uint32_t)(uint16_t)x)` folded to
    `Popcount(x)` — counting the bits the truncation removed (260 vs 129).
  * CLZ select chain: `clz_net((uint32_t)(uint16_t)x)` → `Clz(x)` (26 vs 539).

Fix, uniform across rotate/popcount/clz32/ctz64/shift-pair/bswap-network/
masked-swap/bit-reverse: every VALUE-identity check uses `same_value`
(Copy-chain transparency only) and every folded source is `peel_copies`,
so an idiom whose input runs through ONE cast still folds — with the cast
value as the source, at the idiom's own width — while halves through
different casts of one root no longer match. The rotate matcher also
requires both shifts to run at the OR's width (a shift found under a
widening cast computes at a different width than the amounts are checked
against). Cast-peeling remains only for constants and shift AMOUNTS.

Locked by `tests/regression/bit_idiom_soundness.c` (recognition + cast
attacks at -O0/-O2/-O3 against a reference build) and ten matcher unit
tests including both cast shapes as direct IR-level assertions.

**Warning for future PRs:** #440's own message records "No compilation,
test-suite execution, or benchmarks were run, as requested." The three
defects above were all in that unverified delta. An unverified merge is
an unmeasured merge — run at least the suite + output gate before merge.

## 3. RA-PRESSURE-1: loop-span admission cap (implemented, default OFF)

Mechanism (all unit-tested, `CCC_RA_LOOP_SPAN_RESERVE`, default 0 =
byte-identical old behaviour):

  * `LivenessResult::loop_extents`: linearized `[header_start, latch_end]`
    per natural loop.
  * `mark_loop_spanning`: flags fat-envelope spans; measures per loop the
    block-local PEAK concurrency (the register requirement — shorts reuse
    registers; the count is not the requirement) and the shorts' total
    weighted cost.
  * Admission cap: spans allowed `pool − min(measured peak, knob)`
    registers; excess spans spill at admission; a span never evicts.
  * Pigeonhole gate (cap only when spans outnumber the allowance),
    cost bar (demote only below the shorts' per-register value),
    recurrence guard (MIN_SPAN_LOOP_USES = 3, web-wide through the
    coalesce map — a once-per-pass span's spill load sits on the carried
    dependency chain; arith_loop lost 9.7% to that despite winning the
    frequency count).

Measured, paired 7-rep A/B within single windows:
  * Early window (bar + gate only): chacha20 **+29%**, sha256 **+27%**,
    geomean +0.6%, worst arith_loop −9.7%.
  * The GEP-base / coalesce-web priority boosts must NOT join the bundle:
    priority is the scan's secondary SORT key — re-ranking the main waves
    cost expat −30%, adler32 −23%, sha256 −56% in-window. The 2c phase
    keeps its copies (spare-register-scoped).
  * Late windows (full guard stack) on the throttled VM: net −1.2%, ARX
    wins did not reproduce — and a FIXED binary moved 60× between windows
    (0.809 s → 49.8 s), so cross-window attribution is impossible.

**Remaining before the default can flip** (ordered):
1. Stable-machine census: full-corpus paired A/B at knob 1..8 in ONE
   window; the ARX wins and the guard stack have never been measured in
   the same window on a quiet machine.
2. Cost-bar calibration: the bar uses plain weighted-use totals; the
   recurrence guard's web-wide counting and the bar's shorts total must
   be reconciled (both count coalesce members today, the bar through
   wave ranges, the guard through the member map).
3. The real fix remains live-range splitting at the demotion point
   (evict-and-requeue with reload copies; RA-06
   `split_high_pressure_ranges` exists but measured a net LOSS on
   chacha20 as-is: 1.19 s vs 0.81 s — intra-block re-materialisation
   round-trips, no admission policy). Splitting + this cap's measurement
   machinery is the next session's project.

## 4. MI-ROTATE-1 status after the rebase

Done upstream by #440: variable amounts (`rol %cl`), i686 (32-bit),
aarch64 (`extr`/`ror`), riscv64 (portable pair), MachInst path, identity
folds, width-aware constant folding. Done here: the soundness fixes (§2),
the recognition + attack tests. Still open:
  * sub-word rotates whose complements only meet at the NARROW width
    (`(x16 << 8) | (x16 >> 8)` arrives promoted to i32 with counts 8+8;
    folding needs the truncation-aware pattern — match the consuming
    `Cast(i32→u16)`, rewrite to a narrow rotate, DCE the or-chain. The
    complement-at-promoted-width shape `(w << 8) | (w >> 24)` already
    folds correctly as a u32 rotate of the extended value).
  * rotate-count staging polish (`movslq %esi,%rdx; mov %edx,%ecx` in the
    variable form is a harmless but sloppy count materialisation).

## 5. Patch

`ms178-1.patch` = `git diff ca1c3b34..work-v4`: 14 files, source + tests
+ CI only — no repro dumps, no evidence directories, no mode churn. The
stale 90-file artifact copy from the pre-rebase era has been replaced.
