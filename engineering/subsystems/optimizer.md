# Optimizer

Tiers: `-O0` skip except asm resolve; `-O1` mem2reg/fold/copy/DCE; `-O2` full; `-O3` +unroll; `-Os`/`-Oz` size. Authoritative: `src/passes/README.md`; pipeline and pass inventory: [`../../docs/optimization-passes.md`](../../docs/optimization-passes.md).

Notable modules: `vectorize.rs`, `inline.rs`, `aggregate_sroa/` (copy-out off), `dse.rs` (same-block, `CCC_NO_DSE`), `backedge_pre.rs` (integer recurrences default-on; FP gated, `CCC_BEPRE_FP`), `global_value_numbering.rs` (extended-GVN on scalar + vector/widen families). Cost host: `cost.rs` (IR size/throughput estimates), `func_stats.rs` (function metrics gates), `opt_pipeline.rs` (schedule assembly).

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

## `aggregate_sroa/` (scalar replacement of aggregates)

`mod.rs` drives up to six rounds per function: forwarding through single-writer copies, `split.rs` (an alloca accessed only at constant offsets becomes one scalar per field) and `byval.rs` (by-value aggregate copies coalesced into their source object). Diagnostics, all stderr-only, read once per `run_passes` (`CCC_AGG_BYVAL_DEBUG` once per process): `CCC_DEBUG_SROA=1` (per-function change count, `[SROA]`, and every split, `[SROA-split]`), `CCC_AGG_SPLIT_DEBUG=1` (`split.rs` candidate rejections and the writer-predicate cross-check), `CCC_AGG_BYVAL_DEBUG=1` (every by-value copy it declines, with the reason).

## Do-not / measured notes

- Do not copy Clang sieve vectorization; copy ICX FMA-in-YMM for FP loops.
- Never fold `0.0 + x`, `fabs(x) >= 0`, or NaN/Inf float-to-int; volatile is gated in every load/store pass.
- `movslq`-insertion heuristics in GCC do not justify widening the IV heuristic — verify with same-window A/B (see journal W1 iv-widen entry).
- GlobalAddr CSE/GVN location classes: foldable / must-materialize / site-local variable-index bases. Placement: no movement for cold singleton values; loop-preheader for repeated execution; no eager merge of mutually-exclusive branches; intrinsic-bearing loops refuse new preheader homes.
- Loop-header tail duplication (`jump_thread.rs`), measured Callgrind rules that were rejected -- do not retry: unrestricted (bitops +8.4%, mandelbrot +12%, find_bit +13%, percpu +33%); "two or more latches" (kills the list_sort merge copy); load-fed alone (fannkuch +4.4%); "both successors in the header's loop" (list_sort 1.0042); loop-nest rule without the private-arm check (strlen +2.3%). The final rule gives up rbtree 0.970 / dl_hash 0.990, which came from loop rotation that regresses elsewhere.
- Accumulator recursion elimination runs after `rec2iter`; moved before it, it pre-empts the O(n) Fibonacci rewrite.
- Self-inlining is frame-neutral by construction (no referenced `Alloca`); never relax that -- stack use per recursion level must not grow. Dead parameter-home allocas left by mem2reg do not count.
- Never narrow a compare through an extension that has other readers (see the compare narrowing rule in `docs/optimization-passes.md`).
- Never peel a cast into an indexed address when its source is a single-use load (strstr regression: the fused load-cast is cheaper).
