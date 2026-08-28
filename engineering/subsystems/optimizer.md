# Optimizer

Tiers: `-O0` skip except asm resolve; `-O1` mem2reg/fold/copy/DCE; `-O2` full; `-O3` +unroll; `-Os`/`-Oz` size. Authoritative: `src/passes/README.md`.

Notable modules: `vectorize.rs`, `inline.rs`, `aggregate_sroa.rs` (copy-out off), `dse.rs` (same-block, `CCC_NO_DSE`), `backedge_pre` (integer recurrences default-on; FP gated, `CCC_BEPRE_FP`), `loop_rotate.rs` (opt-in, `CCC_LOOP_ROTATE=1`, runs after vectorize), and shared `alias.rs` (LICM, redundant_loads, loop_memory_promote consumers). The active vectorizer applies per-natural-loop PGO profitability: exact trip <8 is rejected and bodies >80 instructions require >=32 trips; absent profile leaves static policy unchanged. Vectorizer coverage: reductions (single/multi/secondary accumulator), widening I32→I64, masked conditional sums, stencils, elementwise map trees, plain copy; VEX-only widened bodies (9× AVX-SSE transition penalty).

FP contraction is the tri-state `FpContract { Off, OnExpr, Fast }` threaded cli→pipeline→passes→backend; default Off (GCC `gnu*` parity), `OnExpr` fuses only within tagged statement roots, and inlined/created values fail closed.

GVN holds per-object epochs for disjoint non-escaping allocas and `restrict` params; unknown-base entries invalidate on every store. GlobalAddr CSE/GVN uses three location classes: foldable, must-materialize, and site-local variable-index bases (followed through Copy/Cast/constant-address chains). Placement is oracle-derived: no movement for cold singleton values, loop-preheader placement for repeated execution, reuse only of an existing dominating occurrence outside loops, and no eager merge of mutually-exclusive branches. Intrinsic-bearing loops refuse new preheader homes, while safe same-block and non-loop CSE remains enabled.

Do not copy Clang sieve vectorization. Copy ICX FMA-in-YMM for FP loops. Do not fold `0.0 + x`, `fabs(x) >= 0`, or NaN/Inf float-to-int; volatile is gated in every load/store pass.
