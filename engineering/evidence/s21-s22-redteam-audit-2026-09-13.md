# S21/S22 Red-Team Audit — 2026-09-13

Auditor: the delivering agent, second pass, adversarial mindset, per the
standing order ("carefully and thoroughly red-team audit your own work …
Do you agree with their choices? Why or why not?").

Scope: (A) the fall-through elision (L1), (B) the static chain block layout
(L2), (C) the dual-layout measurement selection (L3), (D) the measurement
methodology itself, (E) process failures this session.

Method: full-diff re-read; corpus-wide static A/B (102 functions); dynamic
A/B under the deterministic QEMU icount oracle (`icount_ab.py`,
correctness- and determinism-gated); the full local CI battery (34 gates).

---

## A. Fall-through elision (`emit_branch_to_block`) — L1

**Verdict: sound, and it was the only piece that was trivially right.**

- Semantics: a `jmp` to the physically next emitted block is dead weight on
  every block-based ISA the driver lowers (x86-64, i686, AArch64, RISC-V);
  sequential fall-through between adjacent blocks is the ISA contract. The
  driver sets `next_block_label` before every terminator (shared code path),
  so the check is exhaustive at the only emission site that matters.
- Text-level CFG consumers: `FileLiveness` models `Jmp` as *target-only* and
  every other line as fall-through to the next line, and ARM/RISC-V
  peepholes carry explicit `falls_through` models — so an elided `jmp`
  remains exactly modeled. The `family_private_to` oracle mirrors
  FileLiveness's protocol (S19's two-oracle principle). Verified by the
  corpus (746/0) and both liveness-sensitive gates.
- What I checked and could NOT verify statically: whether any
  pattern-matching peephole keyed on "block ends with a control transfer"
  exists outside the passes I read. The full corpus + benchmark oracle
  (204/204 bit-exact outputs) + differential gate is the evidence; a
  pattern-match that silently degrades would show as an output change.
- Honest residual: L1 alone fired on ZERO edges before L2 existed (measured:
  108 unconditional jumps in `linux_rbtree` main, none to the adjacent
  block). L1 is only load-bearing *together with* L2. That is why they
  shipped as one commit, and the commit message says so.

## B. Static chain layout (`relayout_blocks_static_chain`) — L2

**Verdict: the final design is sound; two of my intermediate designs were
wrong, and the record of *why* they were wrong is the useful part.**

1. **The walk.** Plain greedy successor-following with structural edge
   weights (in-loop 9 / entering a loop 8 / straight-line 5 / plain arm 4 /
   loop exit 1), frontier trace-heads with (placed-pred, nesting frequency,
   LIFO recency, index). Deterministic: every choice is a total order. The
   `Reverse(t)` tie-break keeps the TRUE successor first, which is the same
   default the backend's conditional emitter prefers (`pref_true`) — the
   layout and the emitter agree on which edge becomes fall-through instead
   of fighting over it.

2. **The refined-walk dead end (documented, not hidden).** I first tried to
   fix the zstd hot-loop damage *inside* the walk: a LIFO deferred-arm stack
   with pop-on-cold-edge, latch deferral while a loop body had un-entered
   structure, and a loop-header tie-break. Hand-derivation on zstd's CFG
   produced the exact GCC shape — and the same rules *corrupted* every
   simple-loop function (linux_rbtree chain_cost 820 → 1009; corpus net
   +49 instructions). Root cause: heuristics that are locally principled
   interact chaotically; the latch-deferral blocked a loop body's own latch
   (the body's last block), and the deferred-arm pop ping-ponged equal
   arms. I reverted the whole mechanism. **The lesson mirrors S38's: a
   transform whose correctness argument is "each rule is individually
   sound" is not a transform whose *composition* is sound.**

3. **The laziness gate.** The chain fires only at weighted-gain ≥ 4. The
   threshold is calibrated on measurements, not taste: gain-1 reorders are
   pure RA dice re-rolls (sqlite_varint main: gain 1, +12 static, hot-loop
   slot round-trips); winners exist down to gain 1 and losers up to gain
   19, so the gate refuses to pay risk for nothing rather than pretending
   to predict. With the dual selection (C) the threshold mostly decides
   *compile-time spend*, not quality — which is the right division of
   labor.

4. **`-Os`/`-Oz` and `-m16` are untouched** (RPO start, the 32 KiB boot gate
   margin). The throughput pipeline change is opt-out via
   `CCC_NO_STATIC_CHAIN`, and the historical `CCC_NO_BLOCK_RELAYOUT` still
   disables everything.

5. **PGO interaction (the one open risk I could not fully close).** The
   chain runs before the label renumber and before the PGO
   `layout_module`. PGO block layout preserves incoming order (it only
   places promoted blocks and records hints), so the two passes do not
   fight over the order. But the chain changes the function's block ORDER,
   which is part of the profile fingerprint machinery's input: a profile
   generated by a chain-layout binary and consumed by a no-chain binary
   (or vice versa) can trip the drift gate. The drift gate fails CLOSED
   (skips edge-based reordering, keeps section classification) — the
   worst case is a lost optimization, never a miscompile. I judge this
   acceptable and on record; a follow-up should add the chain state to the
   fingerprint input.

## C. Dual-layout measurement selection — L3

**Verdict: this is the piece I would defend hardest, and it is also the
piece with the honest costs.**

- **Why measurement, not modeling.** The RA's response to a reorder is
  chaotic at the static level: the same branch-quality reorder won −42
  static instructions on linux_rbtree and lost +38 on strlen_bench; won
  −3.9% *dynamic* on rbtree while losing +9.5% dynamic on zstd; and
  zstd's damage was invisible in raw emission counts (154 vs 156 — the
  chain looked *better*) because the text peephole's forwarding kills the
  original order's slot round-trips (15→6) but not the chain's (14→11).
  Every static proxy I tried mispredicted at least one case. The only
  honest selector is to compile both orders and count what the peephole
  leaves.
- **Correctness by construction.** The shipped text is always ONE complete
  emission of ONE order — never a splice. The loser's text is truncated
  out of the buffer. Label counters advance across emissions, so the
  winner's re-emission gets fresh, unique labels (verified: corpus +
  benchmark oracle bit-exact).
- **The metric peephole runs on a FRAGMENT.** `peephole_for_metric` feeds a
  single function's region to the backend's own peephole pipeline. If any
  peephole pass were cross-function, the fragment metric could be wrong —
  but the metric only steers a keep/revert decision between two
  *individually correct* compilations; the worst case is keeping the
  larger one, i.e., yesterday's behavior. The shipped output never
  contains fragment-peepholed text.
- **Debug-trace exactness.** Exploratory emissions set
  `EXPLORATORY_EMISSION`, which silences the load-bearing debug helpers
  (demorgan's count-based gate expects exactly one fire per site — it was
  the first gate to catch the doubling). The winner is re-emitted with the
  flag clear, so traces describe exactly the shipped code. Asymmetry note:
  other env-gated debug prints (replay, GLA trace) can still double under
  exploration; no gate counts those (replay redirects stderr to
  /dev/null; GLA assertions are pattern-based) — verified, but this is
  tolerance, not closure. A future pass could route all codegen debug
  prints through one helper that checks the flag.
- **The side-channel leak I found in this audit.** Records were keyed by
  function name and only drained on `take`; functions the emission loop
  *skips* (gnu_inline definitions) left records behind, and a later
  translation unit in the same process could take a stale record by name.
  The block-count check would reject the malformed permutation
  defensively, but that is luck, not design. Fixed by draining at the end
  of `generate_module` (S22). This is exactly the class the user's audit
  order exists for: I found it by re-reading my own code as an enemy, not
  by any test.
- **The compile-time cost, stated without varnish.** Measured: +120% on a
  single-function chain-heavy file (linux_rbtree 103 ms → 227 ms; the cost
  is 3 emissions + 2 fragment-peepholes for the one chain-fired function),
  +15% wall on the mixed regression corpus (122 s → 140 s), 0% with
  `CCC_NO_STATIC_CHAIN=1`. For one-time-compile/forever-run workloads
  (kernels, libraries) this is the right trade; for `-O2` edit-compile
  loops on chain-heavy TUs it is noticeable. GCC spends more than this on
  its own layout/RA iterations at -O2. If compile time becomes a
  complaint, the knob surface (`CCC_NO_DUAL_LAYOUT`,
  `CCC_NO_STATIC_CHAIN`) is the bisect path, and the honest fix would be a
  decisive-gap fast path (skip the metric peephole when raw counts differ
  by a wide margin) — I measured that this would NOT cover rbtree (6%
  raw gap), so I did not build it on speculation.

## D. Measurement methodology

- Static corpus A/B (local-only rank, 102 functions): the screening
  metric, same pipeline as the CE scoreboard — directly comparable to the
  quest's scoreboard.
- Dynamic A/B: `icount_ab.py` under QEMU TCG with `nochain` logging,
  correctness- and determinism-gated, reps=2. This caught what static
  could not (zstd +9.5%, rbtree −3.9%) and the final selection was
  validated against it (zstd 119,145,077 == 119,145,077 — the damage is
  gone; rbtree 24.99M vs 25.97M — the win kept).
- The CE rank: 3156 → 2967 (−189, −6.0%), 78→77 behind, 20→21 ahead. The
  gap reduction is *entirely* the dual-selected chain+elision; nothing
  else changed. Honest framing: 6% of the gap is real progress, not a
  victory; the remaining top gaps (rbtree +446, csv +388, nbody +212,
  adler +168, chacha +105) are the RA-slot-homing and vectorization
  epics scoped in the S19 audit.

## E. Process failures this session (on record)

1. **I nearly shipped a heuristic (the refined walk) that hand-derivation
   "proved" correct on one CFG.** The derivation was right for zstd and
   wrong for rbtree; only the corpus A/B caught it. Every intermediate
   design was measured before being kept — but the *cost* of the detour
   was two builds and a corpus run that a more suspicious engineer would
   have spent on the dual-run design first. The dual-run was the design I
   dismissed as "expensive" before measuring that the alternatives were
   worse.
2. **I wrote a placeholder closure into a file and nearly left it in**
   (the `edge_cost` unreachable stub) — caught by re-reading the diff.
   Small, but it is the exact "half-baked" failure mode the user has
   called out before.
3. **The test I wrote first asserted my *intended* output, not the pass's
   contract** — the round-trip test used a hand-written permutation that
   was not a permutation of anything. Rewritten to consume the pass's
   actual record. Tests that assert the implementation's current behavior
   are fine; tests that assert an imagined behavior are noise with a
   green checkmark.

## Verdict

The deliverable (S22, base 72fa30fd = latest main, APPLIES-CLEAN) is
validated end-to-end locally: full CI suite 34/34 gates green — fast
battery 31/31, cargo-test 2772/0, regression corpus 746/0 (140 s),
benchmark-output-oracle 204/204 (393 s), differential, clippy, rustfmt —
plus new unit tests (5) pinning the walk, the gate, the parking rule, the
permutation round-trip, and the metric counter. The CE rank improved
3156 → 2967 with the dynamic-icount oracle confirming the kept wins and
the elimination of the one dynamic regression the static metric could not
see. One latent cross-module side-channel leak was found in this audit and
closed by construction (S22). The open risks are on record: PGO
fingerprint sensitivity (fails closed), debug-print doubling on
non-load-bearing traces (tolerance, not closure), and the compile-time
cost of the dual selection (documented, knob-gated, and honestly framed).
