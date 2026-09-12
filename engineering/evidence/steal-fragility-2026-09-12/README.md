# RA-SPAN-STEAL-X64: machine-fragility finding and revert (2026-09-12)

Host: Intel Xeon @ 2.60 GHz, 2 vCPU, 4 GB swap. Toolchain: rustc 1.98.1,
lccc fastbuild (-O1, -j2). Method: `scripts/paired_ab.py`, 15 interleaved
order-alternated rounds, identity-hash + correctness screens, sign test.
All ratios are B/A (median, min) with B = second arm named.

## Verdict

The S22 span-steal investigation (rebased onto `a0e03144`, applied clean,
all gates green, byte-identical fire set to the old host) concluded the
rule must NOT ship. It was reverted before merge; this record exists so
the direction is not re-attempted naively.
It regresses `sha256_transform` by **-11.27 %** on this host (p=0.0003,
min agrees) while its wins do not transfer. No subset, budget, heat floor,
or arm gating repairs sha: every non-empty subset harms (-8.5 % to -50 %).
The rule is a packing perturbation whose outcome is invisible to every
decision-time quantity (cost, depth, lock state, cascade, recurrence all
overlap between the harmed and helped cases). It second-guesses the
validated span-pressure valve with an inferior cost model; the structural
project is valve cost-awareness (FOLLOWUP-2026-09-11-valve-cost-blindness),
not a parallel escape hatch.

## Shipping-config results (steal ON vs OFF, precision ON)

| benchmark | med B/A | min B/A | p | verdict |
|---|---|---|---|---|
| sha256_transform | 0.8873 | 0.9044 | 0.0003 | ON slower 11.27 % |
| sqlite_varint | 1.0576 | 1.0564 | 0.0003 | ON faster 5.76 % |
| zlib_ng_adler32 | 1.0072 | 1.0073 | 0.0003 | ON faster 0.72 % |
| csv_field_sum | 1.0075 | 0.9974 | 0.6056 | noise (median/min disagree) |

Old host (same binaries, verified byte-identical fire set): sha +7.0 %,
varint +3.7 %, adler neutral. Same code, opposite verdicts: the sha shape
(spill state words to stack, keep loop temps in registers) is
host-sensitive. Static codegen is IDENTICAL across hosts (census
-1/+11/-22/+1 over the same 5 files; supply-gate ISO arms reproduce the
S22 2x2 record exactly), so this is purely a dynamic/packing effect.

## Attribution lattice (sha256, this host; every arm vs steal-OFF)

| arm | shape | med | verdict |
|---|---|---|---|
| full steal | 8 incomings fire | 0.887 | -11.3 % |
| K=12 (kill cost-11 dead fires) | 3rd shape | 0.915 | -8.5 % |
| K=22 (kill all dead fires; locked only) | 4th shape | 0.500 | **-50.0 %** |
| dead-only (no locked arm) | novel | 0.785 | -21.5 % |
| single-shot (first win only, v392) | novel | 0.501 | **-49.9 %** |

The fires mitigate each other (locked-only -50 %, dead-only -21 %,
together -11 %): the set is violently non-compositional (L3), and no
decision-time separator exists (incoming cost 11..1600 spans varint's 100;
depths 0..2; lock mixed; cascade all 0 because non-span incomings can
never be re-victimized, which also makes the cascade gate vacuous here).
A per-function budget cannot work either: the allocator cannot know future
fires when deciding the first, and the first win alone is already -50 %.

## Controls (exonerations)

* Precision interaction: steal ON vs OFF with precision OFF both sides:
  -12.16 % (same inversion) — #505's `reuse_redundant_loads` precision is
  EXONERATED.
* Symmetric web-wide flag: scratch build with #505's `live_range.rs` hunks
  reverse-applied produces byte-identical sha for steal-ON and steal-OFF —
  #505's output-neutrality claim EXTENDS to the steal context (agrees).
* Supply (S14): ON vs OFF (steal OFF, shipping precision): +6.35 %
  (min 1.0624, p=0.0003) — TRANSFERS across hosts (+6.06 % old). The
  supply is systematic policy (robust); the steal is perturbation
  (fragile). #505's +6.06 % claim independently CONFIRMED.

## Fire traces (shipped rule, CCC_TRACE_ALLOC)

* sha: v392/c61/d1 (5 locked v10 victims), v393/395/396/c21 (dead),
  v399/391/c11 (dead), v398/c21 (dead), v184/c1600/d2 (6 locked
  v100-300 victims: the inner round loop).
* varint: v140/c100/d1 (4 locked v10 victims). adler: v460/v531/c2000/d3
  (dead victims). csv: v14/c80/d1 (6 locked v10-40 victims).
  k21: v80/c20/d1 (3 locked v10 victims).
* adler under first-only collapses to OFF bytes: v460's escapes lose the
  rank (phantom selection) while blocking v531 — selections != wins.

## Follow-ups

1. Valve cost-awareness (FOLLOWUP-2026-09-11-valve-cost-blindness): the
   original v184 complaint (prio 1600 demoted behind prio-100..400 spans)
   should be fixed IN the valve with its turnover model, not beside it.
2. sqlite_varint oracle gap (lccc 121 vs gcc 116 / clang 115) is now a
   peephole/isel project, not an RA gamble.
3. Lesson L6: packing-perturbation rules must prove robustness across
   hosts (or be systematic policy); single-host paired wins do not
   transfer. Lesson L7: never ship an .s-shape whose benefit depends on
   downstream packing — price the packing or revert.
