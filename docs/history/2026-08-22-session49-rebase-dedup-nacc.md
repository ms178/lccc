# Session 49 — re-base onto PR #190; keep shared-load dedup + N-accumulator lift

Base: `ms178/lccc` main `956c8e1` (PR #190). Build: `fastbuild`, Rust `-O1`,
no LTO, two jobs, 8 GB swap. Host: constrained VM, no PMU.

## What happened upstream

While session 48's patch was in flight, PR #190 merged an independent,
superset implementation of its highest-ROI half:

- **`14ac75e` "x86: keep integer reductions in SIMD homes"** — extends
  `collect_x86_reduction_vector_values` with I32x8/I32x4 **and I64x2** classes,
  makes the integer zero / horizontal-reduce / software-I64-multiply emitters
  consume register assignments, fixes the `x86_fp_pool` detection so an
  XMM2-quarantined pool (starting at XMM3) still enables SIMD allocation, and
  models the I64 lane multiply's `%xmm0/%xmm1/%xmm2` scratch clobber by
  quarantining XMM2 when `VecMulI64x2` is present.  That last mechanism makes
  **I64x2 register homes sound** — strictly better than session 48's choice to
  exclude I64x2 from homes (which lost the optimization).  It also fixes the
  two pre-existing regression failures session 48 documented (Tier-2 frame
  gate now accepts an equally sized frame; `i686_fused_mul_add_operand_order`
  is libc-independent).
- **`c053662`** adds a NEON-only `ReductionKind::Max` (i32 `smax/smaxv`)
  reduction for AArch64.

## What this session kept (still unique, re-applied on top)

1. **`deduplicate_vector_loads`** — merges two `VecLoad*` intrinsics of the
   same op and canonical address inside one reduction body (`b += v*w` after
   `a += u*v`, or `sum += x*x`), rewriting uses to the earlier dominating
   load.  Runs on single- and multi-accumulator paths.  The load key
   canonicalizes the base through a **function-wide def map**: distinct
   `GlobalAddr` SSA values naming one symbol (the frontend emits one per
   source use, and GlobalAddr CSE keeps variable-index bases site-local)
   merge to the symbol name, so a *global* shared array is now deduplicated
   too — the initial pass only handled parameter/alloca bases whose SSA id is
   already object-unique.
2. **N-accumulator multi-reduction** — `ReductionPattern.second:
   Option<SecondaryAccumulator>` became `seconds: Vec<SecondaryAccumulator>`;
   the analyzer accepts any number of independent zero-init chains (pairwise
   disjoint via a running `prior_derived` union), the remainder loop allocates
   one scalar chain per extra accumulator (`RemainderAcc`), and both
   transforms rewrite all bodies in descending add-index order.
3. **`tests/regression/vectorize_int_reduction_homes.c`** — runtime
   correctness pin (nested re-zero, odd trip counts, I64 path, three
   accumulators) that the upstream PR #190 does not carry.

## Conflict resolution

`git cherry-pick b7ed979` onto `956c8e1` conflicted in exactly the expected
places:

- **`regalloc.rs` / `intrinsics.rs`**: upstream's versions taken verbatim —
  they are a strict superset (classes 5/6/7 + XMM2 quarantine + pool fix +
  register-aware emitters), and session 48's versions would have discarded the
  I64x2 win and the pool fix.
- **`vectorize.rs`**: six conflict hunks, all the same two patterns (byte-IV
  union loop and the descending-order body `Vec`), resolved to the
  `seconds: Vec` form; the new NEON `Max` constructor's stale `second: None`
  was updated to `seconds: Vec::new()`, and the two stale "second: None"
  comments were corrected.
- Session 48's now-superseded `docs/history/…session48-int-reduction-homes.md`
  was dropped (upstream carries its own session-48 history); the BACKLOG entry
  was replaced with this session-49 record.

## Validation

- `cargo build --profile fastbuild --locked -j2` with `-D warnings`: clean.
- **1076** `cargo test --lib` (upstream 1075 + re-applied work), **402**
  regression (+1 re-applied `vectorize_int_reduction_homes.c`), **50/50**
  correctness, **360/360** O2/O3/Os differential fuzz — all green.
- 36-file benchmark/pattern corpus A/B vs `956c8e1`: byte-identical except
  `double_reduction` (register homes + dedup, the intended win).
- `double_reduction` benchmark: LCCC ~0.87–0.89× GCC (faster), matching the
  upstream PR #190 measured 0.895× plus the dedup gain.

## Follow-up

1. Merge `deduplicate_vector_loads` into the map-vectorizer path too
   (`transform_map_vector` has no shared-load dedup yet).
2. I64x2 register homes are now sound via the XMM2 quarantine; re-measure
   `vectorize_i64_dot`-shaped kernels and confirm the dot product no longer
   pays the GPR-emulation round trip into the accumulator.
3. Hardware revalidation on the i7-14700KF with PMU remains open for every
   figure above.
