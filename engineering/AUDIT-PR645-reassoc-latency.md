# Red-team audit: PR #645 (1d3349b5, Agent B) vs the S66 phi-sink overlay

Date: 2026-09-27 · Auditor: session 66 · Method: line-by-line diff review,
whole-corpus static census (`scripts/census_ab.py`, 78 files / 186
functions), gate census, paired interleaved wall clock
(`scripts/paired_ab.py`, 41 rounds), unit-test pins on both sides.

## Verdict

**AGREE with the patch; DISAGREE that it is sufficient.** The two patches
are orthogonal layers over the same CI failure (#638): #645 fixes the
*reassociation latency model* so the pass stops *creating* the regression;
the S66 sink removes the *structural relay cost* that made lccc start one
move behind GCC in the first place. Composed on 705de1ad they dominate
either alone. No conflict: zero file overlap (they: `reassoc_latency.rs`,
flag/vector peepholes, CI mirror infra; S66: `phi_eliminate.rs`, verifier
bounds, parser declarators, gate pin, docs).

## Quantified composition (gate census, identical counting on both trees)

| metric | A = 705de1ad (#645+#646) | AM = A + S66 overlay |
|---|---|---|
| rot() default | 55 insns / 2 stkref | **53 / 2** |
| rot() escape-off | 55 / 3 | **53 / 3** |
| rot() legacy | 72 / 34 | 73 / 34 (unchanged arm) |
| sha256_transform fn (-O2, v3) | 144 / 2 | **142 / 2** |
| spectral_norm (ratchet 287) | 286 | **286** (sink correctly refuses) |
| corpus totals (186 fns) | — | **−5 insns / −5 rrmov**, 183/186 identical |
| sha256 wall clock (41 interleaved) | — | median ratio 0.9844 (−1.56 %),
sign-test p = 0.061 on this shared box: direction consistent, not
individually significant; static evidence is the primary claim |

The gate ratchet in `check_phi_acyclic_order.sh` is tightened to the exact
composed shape (k_i ≤ 53, k_s ≤ 3): the output is deterministic, so any
shift is a real codegen change and must ratchet the pin explicitly.

## Line-by-line findings on #645

1. **Rotation lags (`rotation_lags`)** — CORRECT. lag = 1 + min over
   in-loop pure-copy sources, 0 when fed by a computation, cycles → 0;
   monotone forward propagation terminates in ≤ |phis| sweeps. Semantics
   check: in `h=g; g=f; f=e; e=d+t1`, h holds e's value from 3 iterations
   ago — r = −lag is the right availability. The old all-zero timing made
   `((e+f)+g)+(k+h)` look free and lost the folded `add (mem), reg`
   (55→56); the lag model recovers it. MAX_ROUNDS=8 caps pathological
   inputs while strict per-rewrite improvement bounds the natural loop.
2. **Register-residency guard** — RIGHT AXIS, SAFE BIAS. Refuse reshaping
   when GPR-weighted recurrent liveness exceeds the allocatable budget
   across the whole span. Budgets (i686 6, x86-64 13, AArch64/RISC-V 26)
   are all ≤ the true allocatable counts (8−2, 16−2−1, 31−2−2) — every
   misclassification errs toward "leave the tree alone". Their rejection of
   peak/integral/span-pressure proxies is documented with measured
   misclassified cases; I add: the corpus census on AM shows no new spill
   traffic anywhere (the census tool's "+1 stkref" row is a regex artifact;
   direct function-level count for sha256_transform is 2→2 stack refs).
   Nit: the x86-64 reservation (13 vs 14) should name the reserved register
   in a comment.
3. **lea register-addend fold** — CORRECT, including the load-bearing
   refusal set. `lea` writes no flags, so the fold must (and does) require
   `flags_dead_after`; width matching via `names_family_at_width` rejects
   `%r8b` addends; `plain_gp_operand` rejects immediates, memory operands
   and non-GP families; D-aliased and %rsp addends and the `movq %rsp,%rbp`
   frame idiom are refused; `sub` is refused (no negated index); the
   immediate form keeps its %rbp-destination exclusion. The `leal`
   truncation matches `addl` semantics at the same width.
4. **Sibling-test relaxations** — LEGITIMATE (not masking): the assertions
   now accept the equivalent lea form with `sum-of-counts == 1`, and the
   SIB commutation accepts both operand orders at scale 1. The invariants
   kept their strength.
5. **CI mirror (chroot + reverse parity + stamp modes)** — closes the exact
   divergence that let #638 ship: local Debian GCC 14.2 shapes rot() as
   71/0 (SIMD prologue) while the runner's GCC 13.x goes scalar (55/0), so
   a local full pass was blind to the CI failure. The reverse-direction
   parity check with a can-only-shrink exception list is the right
   process ratchet. NOTE: `ci_ubuntu_chroot.sh` needs root; this sandbox
   has no `sudo`/`swapon`, so the mirror itself is validated on GH CI, not
   here — recorded as an environment limit, not a skip.
6. **Where I disagree**: #645 leaves the two relay `mov`s per rot()
   iteration untouched (their 55 vs composed 53) and does not cover the
   accumulator shape at all — on the pre-overlay tree the unguarded sink
   variant of that idea *regressed* spectral_norm by decomposing
   `vfmadd231sd` (289 > 287 budget). That failure mode is now pinned on
   the S66 side (`dead_home_computed_incoming_is_not_sunk`), the firing
   side likewise (`computed_incoming_folds_to_its_copy_slot`,
   `fold_sinks_definition_without_touching_readers` — the latter rewritten:
   it previously passed *vacuously under refusal* because its initial
   arrangement already satisfied its assertions; a test that survives
   disabling the code under test proves nothing).

## Self-audit of the S66 overlay (this session)

* First predicate draft ("another relay into the same home follows") was
  wrong-homed: rot() itself has its other incoming copies in the *preheader*
  (dominates, not dominated) and would have lost the sink (55/2 measured).
  Replaced by the live-home test (window ∪ strictly-dominated traffic
  touching the phi home), which matches both measured spectra exactly.
* The refusal side errs safe in all cases: it gates only *profitability*
  (coalescer success), never correctness — the RA's disjoint-interval rule
  still governs the union; a wrongly-fired predicate degrades to an
  unfused relay, a wrongly-refused one to a missed fold, both pinned by
  census budgets.
* Session hygiene: the harness wipe silently reverted the earlier gate-pin
  edit (old tree measured at k_i ≤ 56); all pins re-measured and re-applied
  on the composed tree with today's census in the comment.
