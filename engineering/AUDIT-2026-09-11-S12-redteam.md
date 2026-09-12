# Red-team audit of the S12 deliverable (self-audit, 2026-09-11)

Scope: everything in `d03ca818..HEAD` that reaches the shipping compiler — i.e. the
decoupled web-wide in-loop-use supply in `src/backend/live_range.rs`, its CI gate
`tests/regression/check_ra_web_inloop_use.sh`, and the documentation/evidence around it.
Method: re-read the diff as an adversarial reviewer, then verify every claim by execution
rather than by argument.

## Findings

### F1 — Stale comment misdescribed the shipping mechanism (FIXED)

At the in-loop-useless demotion site the comment read:

> ``mark_loop_spanning` now supplies merged coalesce members into `uses_in_extents`, so
> the flag reflects the whole web …`

That described the **coupled** design, which is precisely the design that caused the
−40.53 % `lz4_compress` regression and was removed. In the shipping code
`uses_in_extents` is written at exactly one place, from `r.value_id` for `r` in `ranges`
— it never receives a merged member. Merged members go into the separate
`member_in_extent` set.

Why this matters and is not cosmetic: the whole point of the decoupling is that the cap's
*counts* stay per-range while only the *boolean* goes web-wide. A comment asserting that
members are folded into the count map tells the next engineer the opposite of the
invariant the design depends on, and would send anyone debugging an `lz4`-shaped
regression to the wrong map. Rewritten to state the actual mechanism and the reason the
two are kept apart.

### F2 — Provably dead disjunct (FIXED)

```rust
if has_range.contains(&member) || uses_in_extents.contains_key(&member) { continue; }
```

`uses_in_extents`' keys are `{r.value_id : r ∈ ranges, n > 0}`, which is a subset of
`has_range = {r.value_id : r ∈ ranges}`. The second disjunct can therefore never be the
deciding one. It is a vestige of the removed `any_use_in_extent` map — the same class of
leftover the previous audit round deleted. Removed, with the subset invariant recorded in
a comment so it is not "restored for safety".

**F1 + F2 verified output-neutral, not argued:** `scripts/differential_corpus.sh` over the
full corpus — 807 TUs, 805 compiled by both, 0 exit-status diffs, identical 2-TU failure
set, **0 asm diffs of 805**. `cargo test --lib` 2394 passed / 0 failed.
`ci_local.sh --fast` 25 passed / 0 failed / 3 skipped.

### F3 — Leader-only asymmetry (NOT FIXED, deliberate)

`web_in_loop_use` is only ever widened by `members_of.get(&range.value_id)`, i.e. only a
**leader** receives its members' in-extent reads. A value that owns a `LiveRange` *and* is
a coalesce member of some leader never receives the reciprocal supply.

I considered fixing this and decided against it. It is pre-existing behaviour, untouched
by this change; making it symmetric would widen the boolean to more ranges, which changes
victim selection across the corpus, and the measured history of widening this flag is one
large regression (`lz4` −40.53 %) against three small wins. Under §30 that needs its own
reproducer, hypothesis and A/B — it does not get to ride along inside an audit cleanup.
Recorded here so the asymmetry is a known quantity rather than a surprise.

## Do I agree with the choices that ship?

**The decoupling itself — yes, strongly.** It is the only variant that satisfies both
halves of the evidence: the boolean *must* be web-wide (coalesced members share one
register, so a read of any member becomes a hot reload if the web is demoted — that is
physics, not tuning), and the counts *must* stay per-range (they threshold
`MAX_SPAN_EXPOSED_USES`, calibrated 2026-09-08 against chacha20's 11–21 vs adler32's
110–1100). Every attempt to unify them measured worse.

**Shipping a fix whose corpus geomean says "no measurable difference" — yes, but only
because the geomean was refused as the verdict.** 1.0019 over 39 benchmarks is what a
3-benchmark blast radius diluted by 31 unchanged ones looks like. Byte-comparing the
emitted asm for all 34 measurable benchmarks (`ATTRIBUTION.md`) is what makes the claim
safe: 31 are identical to base and therefore *cannot* regress, and the 3 that differ all
improve (+4.7 % / +1.6 % / +1.3 %). `hash_table` reporting +4.7 % on identical asm is the
noise-floor calibration that disqualifies reading any identical-arm row as a regression.
A geomean-only report here would have been either falsely reassuring or falsely alarming.

**Honest caveat on the size of the win.** `sha256_transform` is still **44.10 % slower
than gcc −O2** and 198 instructions against clang's 126. This fix recovers a few percent
of a large gap. It is a correct step, not a solution, and it should not be presented as
closing P0-B.

**Not shipping cost-ordered valve victim selection — yes.** Sub-threshold alone (geomean
+0.52 %, `linux_rbtree` −0.8 %) and it does not fix the coupling it was proposed for,
because `v212` never becomes a valve candidate. Trading a demonstrated rbtree win for
noise on 25 TUs is a bad trade.

**Not shipping x86 load→load slot dedup — yes, and this was the hardest call.** It was
implemented properly (policy threaded, not env-read inline, so both arms are unit-testable
without mutating the environment), proven sound by construction and by execution, and it
closes a documented x86-vs-ARM parity gap. It also buys **2 instructions out of 8414**.
Parity with another backend is not a benefit in itself; a function-wide behavioural change
to a soundness-critical pass has to earn its risk, and 0.024 % does not. Reverted.

## Residual risks I am not able to close here

* **Corpus A/B blind spots.** 31 of 34 benchmarks are byte-identical, so the harness
  structurally cannot detect a regression in code the 3 changed benchmarks do not reach.
  The 807-TU asm differential is the real guard, not the timing table.
* **i686 coverage.** Only 292 TUs compile locally (no 32-bit glibc dev headers in the
  sandbox); those are 0-diff. The rest rests on the CI i686 gates.
* **The cap's design defect is still open.** `worth_capping` can veto a victim without
  nominating a replacement, which hands selection to a cost-blind valve. That is the real
  latent bug behind the `lz4` regression, and it is tracked in
  `FOLLOWUP-2026-09-11-valve-cost-blindness.md` (corrected lead: global cost-ranked cap
  admission). This change stops the bleeding; it does not repair the design.

## Verdict

Ship. Two documentation/dead-code defects found in my own work and fixed under an
output-neutrality proof; one asymmetry consciously left alone with its rationale recorded;
no functional change to the compiler relative to the S12 measurement, so every published
number still applies.
