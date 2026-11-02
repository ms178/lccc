# 2026-10-23 — the open-PR red team, and the layout tie that was worth 8 %

The four PRs open against `44abd1b` are #923, #926, #929 and #932. Three of them
are the same ~11 000-line SIMD stack at three stages of its life; the fourth is
twenty lines in `src/backend/generation.rs`. The audit that produced this
journal entry is `docs/FOLLOWUP-2026-10-23-open-pr-audit.md`; what follows is
what actually happened to the code.

**The twenty lines were the interesting ones, and not because of the claim
attached to them.** #932 says `expat_xml_scan` goes 55.66 → 51.01 ms. On
`44abd1b` the same benchmark moves 52.29 → 47.80 ms with the rule and 52.29 without
it, so the *direction* is right and the *number* belongs to a tree that no longer
exists: the phi-copy coalescing it was rebased over changed which functions tie
and what the append order already is. This is the second time this month that a
performance number survived its own rebase only in spirit (#930 vs #928 was the
first), and the rule for the next one is now written down: a number is valid
against the tree it was measured on, and nowhere else.

**Why the rule matters at all.** The dual-layout emitter builds both the
static-chain order and the pre-chain order and keeps whichever the metric likes,
and the metric is a static count of what a placement leaves taken. Three
quarters of the decisions it is asked to make are exact ties — 32 of 44 sites
over the corpus — and on a tie the historical rule quietly returned the append
order, which is the order the middle-end passes happened to produce. Handing a
tie to the trace-built order is not a heuristic on top of the model; it is the
model admitting it has no opinion, with the one piece of evidence that survives
that admission.

**The evidence that had to be added,** because a tie cannot be decided by
argument: memory traffic. The existing 10 % band exists to protect a placement
*win* from being bought with spills, and on a tie there is no win to protect. So
the band splits: strict (`chain_stack <= orig_stack`) where the counts tie, the
historical asymmetric band everywhere else. That is the whole difference between
the PR's rule and this one, and it is the difference between a rule that can only
help and a rule that can also license a spill. `CCC_LAYOUT_TIE_ORIG=1` restores
the old rule bit-for-bit, because an A/B surface is how the next person argues
with this decision.

**What was refused, and why it is not laziness.** The SIMD stack (#923, #926,
#929) cannot be measured on this corpus: `matmul`, `nbody`, `spectral_norm`,
`double_reduction` and every other FP benchmark contain *zero* 16-byte XMM stack
operands, and `main`'s peephole does not even classify `MoveSize::DQ`. The
round trip those PRs remove does not occur in the programs the harness runs, so
landing them here would be indistinguishable from landing nothing — while
carrying 10 000 lines of unvalidated SIMD semantics across a hand-resolved
conflict with 30 overlapping files. Their soundness arguments are good and their
time will come when a corpus with SSE-shaped aggregates exists; today they are
reviewed, documented and left open.

**Process notes worth keeping.** `git fetch --deepen=400` was needed before any
three-dot diff was meaningful (the clone is shallow and every `merge-base`
silently returned nothing). ICX 2026.2 cannot be provisioned from this sandbox
(`apt.repos.intel.com` is outside the egress allowlist), so no ICX number
appears in the audit at all rather than being inherited from a CI run and
presented as a local measurement. `valgrind` 3.19, `python3-yaml`, the i686
multilib and `qemu-i386` *were* recoverable from the `harness-cache-*` bundles,
which turned ten "environment-only" gate failures into real local coverage.
