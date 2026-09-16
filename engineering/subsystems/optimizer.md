# Optimizer

Tiers: `-O0` skip except asm resolve; `-O1` mem2reg/fold/copy/DCE; `-O2` full; `-O3` +unroll; `-Os`/`-Oz` size. Authoritative: `src/passes/README.md`; pipeline and pass inventory: [`../../docs/optimization-passes.md`](../../docs/optimization-passes.md).

Notable modules: `vectorize.rs`, `inline.rs`, `aggregate_sroa.rs` (copy-out off), `dse.rs` (same-block, `CCC_NO_DSE`), `backedge_pre.rs` (integer recurrences default-on; FP gated, `CCC_BEPRE_FP`), `global_value_numbering.rs` (extended-GVN on scalar + vector/widen families). Cost host: `cost.rs` (IR size/throughput estimates), `func_stats.rs` (function metrics gates), `opt_pipeline.rs` (schedule assembly).

## Written-in semantics (additions must respect)

- SV-resolved derived-IV trampolines; shared-url cleanups run in the cleanup phase, not mid-pipeline.
- GVN canonical-map snapshot/replay treats dispatch as a pure shape; reproducibility pinned by classification/tie-break tests, never by iteration order.
- FP contraction is the tri-state `FpContract { Off, OnExpr, Fast }` threaded cli→pipeline→passes→backend. Default Off = GCC `gnu*` parity: no implicit cross-instruction fusion. Do not relax this to win benchmarks — it miscompiles strictness-sensitive code.
- FP min/max chains: every scalar conditional min/max lowering must be proven lane-exact against `MINPS`/`MAXPS` semantics (NaN and ±0.0 differ from the select expansion — `max(x, 0)` and `min(x, +inf)`-style anchors need hash-identical execution differentials). Two interpreter implementations of the shape (`MapEmitCtx` vs `emit_map_scalar_tree`) — never extend one without the other and the `map_expr_interpreter_tests` agreement test.

## Vectorization surface

- Elementwise map trees, widening I32→I64, masked conditional sums, stencils, plain copy, reduction family; 2× interleave (`vec_interleave` = reduction accumulator chains; below).
- VEX-only widened bodies (9× AVX-SSE transition penalty — the 128-bit lane split under 512-bit is gated).
- Per-natural-loop PGO profitability: exact trip <8 rejected, bodies >80 insns require ≥32 trips; absent profile = static policy.
- FP min/max: lane-exact `MINPS`/`MAXPS`; NaN/±0.0 shadow-proofed. New folds: same-protocol + `map_expr_interpreter_tests` (two interpreters of one `MapExpr`) — the canonical hazard of this family.
- BB-SLP (v5): packet folds incl. packed lane shifts/rotates/minmax; no reassociation (the loop-epics own those — see journal W3 09-16).

## `vec_interleave.rs` (reduction accumulator interleaving)

Splits canonical 2-block vector reductions into interleaved accumulator chains. Measured: dot F64 0.16 ns/elem vs GCC 0.30 L1-resident (1.9×); L2/L3 parity (2.6–3.5× vs kill-switch arm). Knobs: `CCC_NO_VEC_INTERLEAVE=1`, factor override `CCC_VEC_INTERLEAVE=2|4|8`, `CCC_DISABLE_PASSES=vec_interleave`. Open: relax the max/multi-acc rejection (seed-reuse correctness proven upstream, needs a workload); `vhaddpd` combine tree; the two `movslq` IV sign-extensions; AArch64 NEON needs emitter-disp support before un-gating.

## Do-not / measured notes

- Do not copy Clang sieve vectorization; copy ICX FMA-in-YMM for FP loops.
- Never fold `0.0 + x`, `fabs(x) >= 0`, or NaN/Inf float-to-int; volatile is gated in every load/store pass.
- `movslq`-insertion heuristics in GCC do not justify widening the IV heuristic — verify with same-window A/B (see journal W1 iv-widen entry).
- GlobalAddr CSE/GVN location classes: foldable / must-materialize / site-local variable-index bases. Placement: no movement for cold singleton values; loop-preheader for repeated execution; no eager merge of mutually-exclusive branches; intrinsic-bearing loops refuse new preheader homes.
