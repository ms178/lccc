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
- Branch on a merge with a constant arm (`if (smaller(...)) j++`, `while (i < n && separator(s[i])) i++`): if-conversion leaves it, `jump_thread::thread_decided` threads it behind the rounds in up to four rounds of thread and cleanup, then `range_fold` + `set_membership` rerun on the function. Measured alternatives (Cachegrind instructions, `zlib_ng_huffman_heap` / `expat_xml_scan`; before: 16,406,812 / 1,529,625; kept: 14,027,589 / 1,428,038) -- do not retry: the if-conversion rule with only the late threader (16,267,020 / 1,529,645: the passes behind the rounds read the merge unthreaded); a pass that moves the constant edge of a nested merge to the outer merge ahead of the rounds, with the threads behind them (14,007,425 / 1,459,708: the set fold of the rounds commits the outer chain before the inner one is threaded, and `if (punctuation(s[i])) count++` takes 39 instructions against 27); the same pass alone (14,696,557 / 1,454,297); bringing a value that is constant on the way as a constant to the outer merge (14,027,607, classifier `span_name` 22 -> 32 instructions). One round against four differs by layout only on the corpus (14,007,463 against 14,027,612 on the heap, both 746 instructions) and leaves the second level of a nest to the late position, which no set fold follows (`nest_skip_separators` 24 instructions and 9 branches against 25 and 7 with a bit test).
- Accumulator recursion elimination runs after `rec2iter`; moved before it, it pre-empts the O(n) Fibonacci rewrite.
- Self-inlining is frame-neutral by construction (no referenced `Alloca`); never relax that -- stack use per recursion level must not grow. Dead parameter-home allocas left by mem2reg do not count.
- Never narrow a compare through an extension that has other readers (see the compare narrowing rule in `docs/optimization-passes.md`).
- Never peel a cast into an indexed address when its source is a single-use load (strstr regression: the fused load-cast is cheaper).

## Inliner value model (`inline.rs`)

Two budgets are enforced together per caller, because they price different
things. `MAX_INLINE_BUDGET_PER_CALLER` (350) counts **pasted instructions** and
is the growth bound the kernel's stack-frame safety depends on.
`MAX_INLINE_BODIES_PER_CALLER` (350) counts **distinct bodies**, charged once
per callee at its first clone, so repeating one body at several sites does not
spend the allowance twice. Every caller-side cap, the multisite-loop vetoes
(`multisite_loop_wrapper_should_stay_outlined`,
`nested_loop_multisite_static_should_stay_outlined`,
`repeated_small_loop_clone_should_stay_outlined`) and the -Os vetoes are
unaffected, so repetition is still bounded by code growth, not by the budget.

`fits_normal_inline_limits` gives `static` and `static inline` the SAME medium
window (160 instructions): `inline` is a request, `static` only removes the
linkage cost of cloning, so the stronger hint must never be admitted less
eagerly than the weaker one. Loop-bearing bodies still need the acyclic block
window (`MAX_INLINE_BLOCKS_NO_LOOPS`) or `fits_static_loop_inline_limits`.

The general inlining tier is cost-ordered (cheapest callee first, stable
sort, GCC `big_speed` / LLVM bottom-up order) **only when the candidate set
cannot fit the instruction budget**, so every uncontended decision is
byte-identical to the historical CFG-order scan. Contended callers then spend
the allowance on call elimination per instruction instead of per CFG position:
zlib-ng's heap sift (`zlib_ng_huffman_heap`) had its 70-instruction loop body
admitted at three sites before the 24-instruction comparator it calls on
every level, leaving the comparator outlined at 160,797 sites.

## GVN cross-block classes (`gvn.rs`)

`cross_block_renamable` lists what may be DELETED (not merely copied) when a
dominated duplicate hits a dominator table entry: integer `Mul`, integer
`Sub` of two values, pure non-`ReadWrite` calls, and — gated by
`CCC_CAST_RENAME=1`, default OFF — integer `Cast`. Pointer casts, 128-bit
types and float<->int conversions are excluded by construction. The
occurrence-relative ordering argument (dominance orders blocks, not
instructions) is only sound for a pure total function of its operands; loads,
calls, division, intrinsics and inline asm keep the unconditional `Copy`
form. Measured effect of the cast class on its target shape
(`zlib_ng_huffman_heap`, PASSES=20, n=15): 16.89 ms -> 11.09 ms (2.241 ->
1.472 vs gcc 16.2). It stays opt-in until the paired corpus run shows a net
win at n>=15 across the zstd/gzip workloads, which currently move 1-3 % in
both directions.

The knob is resolved **once per translation unit** in `run_passes`
(`src/passes/mod.rs`) and installed with `gvn::set_cast_rename_enabled`, the
same install-once shape as `loop_idiom::set_config` and
`vectorize::set_strided_reduce_enabled`. The pass body reads a bit, not the
environment: `check_env_test_hygiene.sh` counts every `env::var`/`env::var_os`
call site under `src/passes/` (budget 150, raised only with a documented
measurement), a per-function probe would have pushed it to 151, and a
concurrent unit-test harness must not drive a pass through `env::set_var`.
`gvn::set_cast_rename_enabled(true/false)` is also the supported way for a test
to exercise the class.
