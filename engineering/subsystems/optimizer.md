# Optimizer

Tiers: `-O0` skip except asm resolve; `-O1` mem2reg/fold/copy/DCE; `-O2` full; `-O3` +unroll; `-Os`/`-Oz` size. Authoritative: `src/passes/README.md`.

Notable modules: `vectorize.rs` (~6905), `inline.rs`, `aggregate_sroa.rs` (copy-out off), `alias.rs` (LICM not wired), `licm.rs` GEP TODO, `vectorize_gate` always true (`pgo/unroll_pgo.rs`).

Do not copy Clang sieve vectorization. Copy ICX FMA-in-YMM for FP loops.
