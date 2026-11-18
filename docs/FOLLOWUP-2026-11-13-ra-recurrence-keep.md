# RA-RECURRENCE-KEEP: recurrence-victim veto in ordinary eviction

Date: 2026-11-13. Owner: backend/regalloc.

## Problem

`select_evict_victim` (mode-3 ordinary eviction) whole-value demotes its
victim: `try_evict` removes the final home, so every reference of the
victim — including loop-hot reads *behind* the linear-scan point —
becomes slot traffic. Both rank keys underprice exactly that shape for a
loop recurrence: the position-relative remaining cost reads 0 once the
loop has been scanned past, and the static priority's loop weight is
depth-based, not trip-count-based.

Motivating failure (zstd_count): the setup-loop counter (priority 60,
remaining cost 0, fat interval stretched over the later loops because
layout parks the loop's tail block last) was evicted at the second
loop's admission by a priority-100 single-use cast living wholly inside
the counter's liveness hole — ~3M loop-1 register reads became stack
round-trips (+5.3M Ir, 1.15x vs GCC).

## Fix

A resident `span_recurrence` is never an ordinary-eviction victim when
BOTH hold at the decision point:

1. `spill_cost_at(pos) == 0` — the rank key prices the victim as free,
   which for a recurrence is a lie (the behind-the-scan reads are
   repriced by the demotion), and
2. `!conflicts_with_segments(incoming)` — the conflict is a fat-interval
   phantom: the victim is provably dead at every point the incoming
   lives, so the eviction frees nothing the incoming actually needs.

The vetoed shape is not starved: the recurrence only needs to KEEP the
home it already holds, and the hole-contained incoming is served by the
segment-aware hole-filling waves (unit-tested: the incoming shares the
recurrence's register inside the hole).

### Rejected alternatives (all measured, callgrind Ir)

* **Blanket recurrence veto** (no conditions): zstd_count 93.83M (best),
  but zstd_double_fast +6.8% and gzip_longest_match +1.8% — hot
  uses=1100 incomings slotted while cold resident recurrences kept the
  pool. Net loss.
* **Hole-containment only** (condition 2 alone): gzip_longest_match
  +3.5% (2248M vs 2173M) — a costat=50 recurrence was protected from two
  hot prio=1000 hole-contained incomings and the cascade slotted
  uses=1000, 7-8-segment values.
* **Zero-remaining-cost only** (condition 1 alone): gzip_longest_match
  +0.6% — costat=0 recurrences whose segments REALLY overlapped hot
  prio=50000 incomings kept the pool.

### Shipped conjunction, measured matrix (vs main / veto off)

| benchmark          | main Ir      | veto Ir      | delta  |
|--------------------|--------------|--------------|--------|
| zstd_count         | 99,204,196   | 98,024,497   | −1.19% |
| loop_patterns      | 266,295,905  | 266,795,903  | +0.19% |
| gzip_longest_match | bit-identical binary        | 0      |
| zstd_double_fast   | bit-identical binary        | 0      |
| expat_utf8_content | 215,357,010  | 215,357,010  | 0      |
| expat_xml_scan     | 616,153,160  | 616,153,160  | 0      |
| reg_off_window     | 22,168,319   | 22,168,319   | 0      |
| linux_rbtree       | 16,688,237   | 16,688,237   | 0      |
| sha256_transform   | bit-identical outputs, equal Ir | 0  |
| chacha20_block     | bit-identical outputs, equal Ir | 0  |

Known residual: loop_patterns +0.19% (+500K Ir). Two cold recurrences
whose holes contain the unrolled dot-product loop are protected; the
denials reshuffle evictions onto hot loop temps (per-iteration
`movl/movslq` stack round-trips). Root cause is the static cost model's
trip-count blindness — a 1-bit eviction gate cannot price it; follow-up
belongs in the spill-cost model, not in more gate conditions.

## Knob

`CCC_RA_NO_RECURRENCE_KEEP` — disable the veto (default: off, veto
active). Diagnostic/A-B switch only: load-bearing, never default off.
It reproduces main's allocation exactly (gzip_longest_match and
zstd_double_fast binaries are bit-identical under the switch), which is
what made per-benchmark A/B possible without a main worktree.

## Tests

* `ordinary_eviction_never_takes_a_recurrence_for_a_zero_cost_phantom_conflict`
  — the zstd_count shape; also pins that the hole-contained incoming is
  served by hole-filling (shares the register) rather than starving.
* `recurrence_with_remaining_cost_and_real_overlap_stays_an_ordinary_victim`
  — the gzip shape keeps today's eviction semantics.
