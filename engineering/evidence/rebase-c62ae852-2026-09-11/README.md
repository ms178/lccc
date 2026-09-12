# Re-validation on upstream `c62ae8526` (PR #501 merged mid-flight, 2026-09-11)

A parallel agent merged PR #501 (`fe1170c4b`, "peephole: copy->RMW coalescing + %rdx
return marker; store-forwarding rsp fix; vectorizer dominance fix") while this work was in
flight. It touches `relay_and_lea.rs` (+822), `store_forwarding.rs` (+125), `liveness.rs`,
`flag_peepholes.rs`, `prologue.rs` and `vectorize.rs` — i.e. it changes generated code —
so every measurement was re-derived rather than assumed. No file overlaps this change, and
the six commits replayed onto `c62ae8526` with zero conflicts.

## Blast radius (`scripts/differential_corpus.sh`, base vs candidate)

807 TUs, 805 compiled by both, 0 exit-status diffs, identical 2-TU failure set,
**10 asm diffs**:

`k_adler32_do8`, `k01_adler`, `linux_rbtree`, `sha256_transform`, `strlen_bench`,
`cmp_replay_acc_nohome`, `huft_build_crash`, `iv_widen_float_conversion_closure`,
`vector_defer_multidef_slot`, `vectorize_matmul_n17`.

**`lz4_compress` is not among them** — it is byte-identical to base, so the binding
no-regression constraint holds structurally, not just statistically.

## Runtime (`scripts/perf_ab.py`, A = pristine `c62ae8526`, B = candidate, min metric)

| benchmark | flags | A min (ms) | B min (ms) | B/A | verdict |
|---|---|---|---|---|---|
| `sha256_transform` | `-DPASSES=8 -DBLOCK_COUNT=131072`, 11 reps | 425.45 | **404.09** | **0.950** (low3 0.949) | **B 5.02 % faster** |
| `lz4_compress` | `-DSRC_SIZE=(1UL<<22) -DPASSES=96U`, 21 reps | 183.63 | 183.01 | 0.997 | no measurable difference |
| `linux_rbtree` | harness default, 11 reps | 16.32 | **16.07** | **0.985** | B 1.5 % faster |
| `strlen_bench` | harness default, 11 reps | 227.28 | **223.75** | **0.984** | B 1.6 % faster |

`paired-sha256.md` is the full 37-benchmark harness screen at the same time (aggregate
geomean 0.9984, "no measurable difference" — the expected reading when only 10 of 805 TUs
change: the 27 unaffected benchmarks dilute three real wins into noise, which is exactly
why the asm differential above, not the geomean, is the regression argument).

## Conclusion

The headline win **survives and slightly improves** across the upstream merge
(+4.7 % on `d03ca818` -> **+5.02 %** on `c62ae8526`), `linux_rbtree` and `strlen_bench`
reproduce, and `lz4_compress` remains byte-identical. `cargo test --lib` and
`ci_local.sh --fast` were re-run on the rebased tree; see the S14 ledger row.
