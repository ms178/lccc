# Session 42 — vector temp-promotion red-team audit (2026-08-21)

Base: `ms178/lccc` main `aa8bc98` (PR #178). Build: `target/fastbuild`
(`opt-level=1`, no LTO, 2 jobs, 8 GB swap). Host: 2-core VM, no PMU.

## TL;DR

Red-team audit of Agent D's rewrite of `src/passes/vector_temp_promotion.rs`
(the late SIMD cleanup: temp promotion, direct-load fusion, alignment
downgrade). The rewrite is a major correctness improvement over the prior
517-line version and was adopted, after fixing four defects found during the
audit. Assembly is byte-identical to the pre-rewrite compiler on the entire
34-file benchmark + SIMD-pattern corpus (A/B diffed), and every test gate is
green.

## What the rewrite gets right (verified, kept)

1. **Alias-aware promotion window.** The old `no_var_access_between` only saw
   *syntactic* uses of the destination alloca; a read through a `GEP`/`Copy`
   alias of the destination slipped through and reordered a store past it.
   `analyze_destination` + `destination_unobserved_between` close this with a
   pointer-alias fixpoint plus a whole-function escape gate. New test
   `derived_alias_read_in_window_blocks_promotion` pins the fix.
2. **Full-width result gating.** Promotion now requires
   `op.vector_result_width() == alloca_size` and `dest: Some`, and rejects
   volatile/semantic-volatile slots — strictly safer than the old code.
3. **Direct-load set is address-correct.** `direct_vector_load_width` only
   admits 1-argument full-width loads (`Loaddqu`, `Loadu256/256/uPs/uPd256`).
   The old code also admitted the 2-argument `VecLoadF64x4(base, offset)` and
   forwarded **base alone**, dropping the offset — a latent miscompile.
4. **Memory-write invalidation exists at all.** The old fuse had *no*
   invalidation: `v = load(p); v = setzero(); use(v)` forwarded `use(v)` to the
   stale `p`. The rewrite tracks writes and kills pending forwards.
5. **`vmovdqu`-only memory accesses verified in the backend.** `Load256`
   emits a register-register `vmovdqa` after `avx_load_arg`'s `vmovdqu`, and
   every slot store/load is `vmovdqu`/`movdqu` — so `_Alignas` on a
   non-escaping vector alloca is genuinely unobservable and the downgrade is
   sound. `Alloca`-vs-`Param`/`Global` disjointness is sound via the
   fresh-object argument (a param is fixed at entry, before the callee frame's
   allocas exist).
6. **Source-span-preserving instruction removal.** `remove_instructions` keeps
   `BasicBlock.source_spans` in lockstep (the old rebuild dropped spans).

## Defects found and fixed

| # | Defect | Severity | Fix |
|---|--------|----------|-----|
| 1 | `invalidate_for_pointer_write` checked only *source-vs-write* disjointness, so a write through the loaded **slot itself** (`Setzero256 -> %v`) left the stale forward alive | **miscompile** | also require *slot-vs-write* disjointness (`roots.get(slot)` vs `write_root`) |
| 2 | `pointer_roots` did not propagate through `Add`/`Sub`, so a load source `a + i*32` (int pointer-arithmetic, folded by LCCC) had **no root** and was treated as "may alias anything" — the second load's write killed the first forward, **un-fusing** `x = a[i]; y = b[i]; acc = op(x,y)` (regression vs the old pass, caught by A/B diff) | perf regression | propagate the root through `Add`/`Sub` when exactly one operand is rooted |
| 3 | `intrinsic_arg_allows_unaligned_alloca` allowed `LoadF64x4/LoadF64x2/LoadI32x8/LoadI32x4 => index < 2`; index 1 is the **byte offset** (an address-foldable integer) | latent unsoundness | `index == 0` |
| 4 | `dest_ptr: None` invalidation exempted `produces_vector_value()` (dead code, foot-gun); load removal ignored the load's `dest` SSA result | fail-closed gaps | `!is_pure → clear`; removal requires both slot and `dest` uses to be zero |

## Validation

- **1022** `cargo test --lib` unit tests (12 in this module, incl. new
  regression tests for defects 1–4).
- **389 + 1 skip** regression suite, **50/50** correctness, **60/60** O2
  differential fuzz — all green.
- **A/B assembly diff**: every file in `tests/benchmark/programs/*.c` +
  `tests/benchmark/patterns/*.c` (34 files) compiles to **byte-identical**
  assembly before/after the rewrite (`-O2 -march=x86-64-v3`), plus a
  hand-written `_mm256` loop.
- **Benchmark gate** (lccc vs gcc, 9 reps, paired median, checksums verified):
  gzip_crc32 1.07×, zlib_ng_adler32 1.56×, expat_xml_scan 1.90×,
  struct_copy 1.53× — unchanged from the pre-rewrite baseline.

## Pre-existing, out of scope (recorded for later)

`tests/intrinsics/cases/{t128_fp,t256_fp,t512_int}.c` fail with **semantic
errors** (`__lccc_simd256_ps_vshufps256` "too few arguments"): the test cases
pass the imm8 in GCC's position, which does not match the bundled
`immintrin.h`'s `__lccc_simd*` signatures. This is a frontend/header mismatch,
unrelated to any optimization pass; it fails identically before this change.
