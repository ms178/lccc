# Optimizer

Tiers: `-O0` skip except asm resolve; `-O1` mem2reg/fold/copy/DCE; `-O2` full; `-O3` +unroll; `-Os`/`-Oz` size. Authoritative: `src/passes/README.md`.

Notable modules: `vectorize.rs` (~6905), `inline.rs`, `aggregate_sroa.rs` (copy-out off), and shared `alias.rs`. LICM consumes shared linear forms. The active vectorizer applies per-natural-loop PGO profitability: exact trip <8 is rejected and bodies >80 instructions require >=32 trips; absent profile leaves static policy unchanged.

GlobalAddr CSE/GVN uses three location classes: foldable, must-materialize, and site-local variable-index bases (followed through Copy/Cast/constant-address chains). Placement is oracle-derived: no movement for cold singleton values, loop-preheader placement for repeated execution, reuse only of an existing dominating occurrence outside loops, and no eager merge of mutually-exclusive branches. Intrinsic-bearing loops refuse new preheader homes until RA-23 models hidden locations, while safe same-block and non-loop CSE remains enabled.

Do not copy Clang sieve vectorization. Copy ICX FMA-in-YMM for FP loops.
