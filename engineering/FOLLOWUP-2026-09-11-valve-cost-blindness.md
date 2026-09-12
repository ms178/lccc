# FOLLOWUP — why the admission cap's veto strands cheap spans (and the remedy that did NOT work)

Date: 2026-09-11
Status: OPEN, root-caused, measured. **The originally proposed remedy has been
implemented and experimentally FALSIFIED — do not retry it.**
Found by: the blast-radius screen of the web-wide in-loop-use supply
(`scripts/differential_corpus.sh` over all 807 `.c` files under `tests/`), which
surfaced a **40.53 % regression on `lz4_compress`** that no prior measurement had
looked for.
Evidence: `engineering/evidence/ra-web-inloop-use-2026-09-11/paired-lz4-base-vs-coupled.json`,
`engineering/evidence/corpus39-rebase-2026-09-11/paired-lz4-coupled-plus-valve.json`,
`.../perf-ab-valve-vs-ship.{json,md}`, `.../corpus-differential.txt`

## What was measured

Supplying merged coalesce members' in-loop reads into the admission cap's use
COUNTS (`span_in_loop_uses` / `span_exposed_uses`) — as opposed to only into the
web-wide `span_has_in_loop_use` boolean — costs:

```
lz4_compress, -O2 -DSRC_SIZE=(1UL<<22) -DPASSES=96U, ~186 ms/arm
  base 25ed36de -> count-coupled, 41 rounds
    base    : min 182.71  med 183.94  sd  2.36
    coupled : min 257.22  med 258.49  sd 39.89
    median ratio 1.4053   min ratio 1.4078   p = 0.0000  => base 40.53 % FASTER
```

Reproduced on the rebased base `d03ca818`, 31 rounds: median 1.4262, min 1.4080,
p=0.0000 (**42.62 %**). Both estimators agree, both arms agree on stdout and exit
status, and the instruction count is **identical** — 251 instructions in `main`
for both arms. Mission §22/§23 says an identical-count/runtime-gap result demands
a mechanism, and `CCC_TRACE_ALLOC=1` supplied one.

## The mechanism

```
counts NOT fed: demote v212 @0 in_loop_uses=1 exposed=1 recur=false remcost=100  site=2
counts fed    : v212 is not demoted anywhere
                demote v133 @5 in_loop_uses=3 exposed=3 recur=false remcost=1110 site=5
```

1. Web-wide counting raises v212's `span_exposed_uses` from 1 to 3, past
   `MAX_SPAN_EXPOSED_USES = 2`, so `worth_capping` becomes false and the
   admission cap (site 2) stops demoting it.
2. The pool is still over the cap, so the pressure reaches the span-pressure
   valve (site 5), which spills **v133 — `remaining_cost` 1110 against v212's
   100**, an 11× more expensive victim.
3. `main`'s hot loop goes from 25 to 29 frame-relative references and the
   function from 12 to 14 stores: 40–43 % slower at identical instruction count.

The web-wide count is not lying — coalesced members share one register, so those
three reads really would become hot reloads. The defect is that `worth_capping`
is a **veto with no replacement policy**: when it refuses a cheap span, the
decision does not fall to another cost-aware rule, it falls through to whatever
selector happens to run next. Base gets the right answer here by accident — it
under-counts the web, which keeps the cheap span demotable.

## The remedy that was tried, and why it is falsified

The obvious reading was that the valve picks badly because
`find_span_valve_victim` orders candidates by `future_uses` — an unweighted count
of remaining use points — with a Braun–Hack "farthest next use" tie-break and **no
cost term at all**. So the valve was made cost-ordered: same eligibility window
(`MIN_VALVE_SPAN_FUTURE_USES..=MAX_VALVE_SPAN_FUTURE_USES`, kept because a span
with no future use should expire rather than be evicted — the measured
gzip_crc32 lesson), but among eligible victims prefer the lowest
`remaining_cost`, which is the same weighted quantity the cap already thresholds
via `MAX_SPAN_REMCOST`. The plan was: valve first, then re-feed the counts.

**Both halves were measured and the plan fails.**

*The valve change does not fix lz4.* With cost-ordering in place and the counts
re-coupled, the trace is unchanged in the way that matters:

```
demote v133 @5 in_loop_uses=3 exposed=3 remcost=1110 site=5     <- still chosen
demote v403 @47 in_loop_uses=6 exposed=4 remcost=1410 site=5
(v212 does not appear at any site)
```

and lz4 is still **42.62 % slower** (median 1.4262, min 1.4080, p=0.0000, 31
rounds). The reason is now clear and is the actual lesson: **v212 is never a
valve candidate at all.** Cost-ordering a selector cannot pick a span that does
not reach it — by the time the valve runs, v212 has either already been homed or
fails the valve's own eligibility gates (`spans_loop`, non-recurrence,
`register_steal_is_safe`, and the `fut ∈ [1, 2]` window). Ordering among the
survivors of those gates is not where the 1110-vs-100 choice is made.

*The valve change is also not a win on its own.* Measured against the shipping
build over every benchmark whose code it changes (`perf_ab.py`, `-O2`, 9
interleaved AB/BA rounds, minimum metric):

| benchmark | B/A (valve faster when >1) |
|---|---|
| nbody | 1.021 |
| fannkuch | 1.012 |
| double_reduction | 1.011 |
| glibc_strstr | 0.998 |
| chacha20_block | 0.997 |
| linux_rbtree | 0.992 |

Geomean **1.0052**, which `perf_ab.py` itself reports as *NO MEASURABLE
DIFFERENCE* (below its 1.0 % threshold). It changes 25 of 805 translation units,
including 16 regression reproducers, and it makes `linux_rbtree` slightly worse —
one of only three benchmarks in the corpus whose code this patch series changes at
all, and one of the three attributable wins (+1.6 %). Trading a demonstrated win
for a sub-threshold geomean on core victim selection is a bad trade.

**Decision: the valve cost-ordering is not shipped, and the "valve first, counts
second" sequencing is closed.** Negative result recorded so the direction is not
retried.

## What the corrected lead is

The choice between "spill the 100-cost span" and "spill the 1110-cost span" is
made by the **admission cap**, not by the valve — the cap decides individually,
per span, whether to demote, and a span it vetoes is simply homed. What is
missing is a *ranking* step: when the pool is over the cap and several spans are
simultaneously candidates, the cap should demote the cheapest one(s) until the
pool fits, rather than applying a per-span veto and letting the residual pressure
be resolved later by a selector with different criteria.

Concretely, the lead is to make the cap's decision **global over the candidate
set** — collect the spans that `worth_capping` would demote plus the ones it
vetoes only on the `MAX_SPAN_EXPOSED_USES` count, order by `remaining_cost`, and
demote cheapest-first until `loop_spanned_register_count() <= allowed`. That is
where a 100-vs-1110 comparison can actually be made, and it is the only place the
web-wide counts become safe to feed, because the count would then inform *which*
span to spill rather than *whether* to spill one at all.

This is a substantive change to admission policy, calibrated against chacha20's
ARX webs, zlib_ng_adler32's inlined NMAX loop and glibc_memcmp's address span, so
it needs its own measurement campaign on the full 39-benchmark corpus before it
can land. It is deliberately not bundled into this patch.

## What ships instead, and why that is not a compromise

The member supply feeds the web-wide BOOLEAN and leaves the cap's per-range
counts as calibrated. This is not a retreat from correctness — it is exactly
scoped to where the evidence is:

* `sha256_transform`'s assembly is **byte-identical** with and without count
  coupling, so the boolean is the *whole* of the win and coupling adds nothing to
  it. Verified directly (`cmp` on `-O2 -DPASSES=8 -DBLOCK_COUNT=131072` output).
* Count coupling's only measured effect anywhere in the corpus is the lz4 loss.
* Blast radius drops from 13 to 10 of 805 translation units; `lz4_compress`,
  `divrem_pair_opposite_flavour.c` and `o0_phi_multidef.c` stop being touched.
* All three corpus benchmarks whose code changes are wins: `sha256_transform`
  +4.7 %, `linux_rbtree` +1.6 %, `strlen_bench` +1.3 % (`perf_ab.py`, 11 rounds),
  corroborated by amplified paired A/Bs at +3.63 %/+3.81 %, +1.05 %/+1.06 % and
  +1.18 %/+2.89 % (median/min, p ≤ 0.0012).

## Guards in place

* `tests/regression/check_ra_web_inloop_use.sh` property 5 — `lz4_compress` must
  be byte-identical with the supply on and off. Mutation-verified: against a
  count-coupled build it exits 1 reporting "224 differing lines" while still
  confirming the sha256 mechanism fires.
* `live_range::tests::mark_loop_spanning_member_uses_do_not_inflate_the_cap_counts`
  — a merged member read at four distinct in-extent points must set the boolean
  and leave both counts at the leader's own zero.
* `scripts/differential_corpus.sh` — the screen that found this. Run it against
  any allocator change *before* a runtime A/B: it names exactly which
  translation units moved, and therefore which benchmarks are worth timing.
  Timing only the benchmark you aimed at is how a 40 % regression survived two
  review rounds.
