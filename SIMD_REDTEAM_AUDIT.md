# SIMD Implementation — Red-Team Audit & Fixes

**Date:** 2026-08-13 · **Branch:** `main` @ `7d86fbf0` (rebased against latest `ms178/lccc`)
**Scope:** auto-vectorizer (`src/passes/vectorize.rs`), SIMD intrinsic lowering
(`src/backend/x86/codegen/{intrinsics,intrinsics_simd,emit}.rs`), x86 VEX/EVEX/SSE
assembler encoders, the generic-SIMD builtin table, and zlib-ng as the real-workload
test case.

**Method:** every claim below was verified empirically — a C reproducer was compiled
with lccc (-O2, plus `LCCC_FORCE_SSE2=1` for the legacy path) and cross-checked against
GCC 14.2 / Clang 19.1 and, where applicable, a Python reference. Generated assembly was
disassembled to pin each root cause before fixing.

---

## 1. Executive summary

The SIMD implementation was **correct on its one happy path** (AVX2 4-wide double
matmul with `N % 4 == 0`, single-array F64/I32 reductions) but contained **six
correctness-critical defects** that produced wrong results or illegal instructions on
every other shape it claimed to support, plus a large body of dead code. All were fixed,
regression-tested, and re-validated against GCC. The remaining strategic gap is
**performance, not correctness**: lccc keeps no vector values in registers across
intrinsic boundaries, so hand-written intrinsic code (zlib-ng) is **~5.3× slower** than
GCC despite identical, correct semantics.

### Verified before → after

| Reproducer (N=255 unless noted) | lccc (before) | lccc (after) | GCC/Clang |
|---|---|---|---|
| matmul double (AVX2) | 33959684223.750 ✗ | 33958656000.000 ✓ | 33958656000.000 |
| matmul double (SSE2 path) | 67527162000.000 ✗ | 33958656000.000 ✓ | 33958656000.000 |
| matmul **float** | **SIGSEGV** ✗ | 33958715392.000 ✓ | 33958715392.000 |
| dot product (AVX2 & SSE2) | 203466.250 / 187640.000 ✗ | 699040.000 / 707264.000 ✓ | 699040.000 / 707264.000 |
| `float s += (float)int[i]` | 0.000 ✗ | 32896.000 ✓ | 32896.000 |
| `_mm256_hadd_ps` | **SIGILL** ✗ | 3 7 19 23 11 15 27 31 ✓ | 3 7 19 23 11 15 27 31 |
| `_mm256_blendv_ps` | returns *mask* ✗ | 1 2 3 2 5 2 7 2 ✓ | 1 2 3 2 5 2 7 2 |
| `_mm256_permutevar_ps` | undefined symbol ✗ | 4 3 2 1 8 7 6 5 ✓ | 4 3 2 1 8 7 6 5 |

---

## 2. Confirmed critical defects (fixed)

### D1 — AVX2 matmul tail: OOB write + dead remainder (wrong results)
`transform_to_fma_f64x4` converted the loop to a byte-offset IV stepping 32 with limit
`N*8`. For `N%4 != 0` this runs `ceil(N/4)` iterations: the last one reads/writes up to
3 doubles **past the row end**, and the scalar remainder (start = `IV/8 = ceil(N/4)*4`)
never fires. Reproduced with N=255 (sum 33959684223.75 vs 33958656000.00; element 255
clobbered, 252–254 skipped).
**Fix:** byte limit = `(N/4)*32` (floor). Constant `n → (n/4)*32`; dynamic `N → (N/4)*32`
via hoisted `UDiv` + `Mul`.

### D2 — SSE2 matmul remainder computed on the wrong IV representation
`insert_remainder_loop` always computed `j_rem_start = IV / 8` (byte-offset assumption),
but the SSE2 path keeps an **element-index** IV → the remainder re-processed nearly the
whole array (67527162000.00 vs 33958656000.00).
**Fix:** remainder start is now width-aware: `IV/8` for the byte-offset AVX2 IV, `IV*2`
for the element-index SSE2 IV.

### D3 — Dot-product: second array left at scalar stride
`transform_reduction_{avx2,sse2}` "scale array indexing" had
`break; // Only process first GEP per block`, so a dot product's second GEP was never
scaled and loaded one element per *vector* iteration (dot(255)=203466.25 vs 699040.0).
**Fix:** collect *all* matching GEPs, apply the ×vec_width scaling in reverse order.

### D4 — Matmul pattern had no element-type gate
`analyze_loop_pattern` matched `C += A*B` for *any* element type but the transform
always lowered to `FmaF64x4/FmaF64x2` (packed **double** FMA). A float matmul was
reinterpreted with an 8-byte element stride → **SIGSEGV**.
**Fix:** require the multiply's element type to be `IrType::F64`; float/int matmuls stay
scalar (they already match GCC numerically).

### D5 — Reduction cast-sum path accepted cross-kind casts
The cast path rejected only *widening* (`long += int`), so `float s += (float)int_arr[i]`
was vectorized as packed I32 adds on an F32 accumulator and returned 0.0.
**Fix:** require the cast source type to equal the accumulator type (redundant casts
only); everything else is rejected.

### D6 — 256-bit `vblendvps/vblendvpd` returned the mask operand
The blend arm computed its result in `%ymm1` but `avx_store_dest` stores `%ymm0`, which
held the **mask** — the intrinsic silently returned its mask input.
**Fix:** produce the result in `%ymm0` (mask→`ymm2`, a→`ymm0`, b→`ymm1`).

### D7 — `FmaF64x2` (SSE2 matmul) aliased the B and C pointers
The lowering materialized pointers through the transient value cache, which conflated
two GEPs sharing an offset but not a base — C was loaded and stored through B's address.
**Fix:** mirror the proven `FmaF64x4` discipline: use the register allocator's
assignments (`operand_reg`/`dest_reg`) with explicit fallbacks, then
`reg_cache.invalidate_all()`.

### D8 — Legacy 2-operand SSE instructions emitted in 3-operand form
`unpckhpd` (2×F64 horizontal add, ×2 sites), `psrldq` (4×I32 horizontal add, ×2 sites)
and `paddd` (`VecAddI32x4`) were emitted with a 3-operand VEX-style shape
(`unpckhpd %xmm0, %xmm0, %xmm1`, `psrldq $8, %xmm0, %xmm1`, `paddd %xmm1, %xmm0, %xmm0`).
The assembler rejects them ("SSE op requires 2 operands"), so the entire SSE2 reduction
path failed to assemble. The AVX (VEX) forms were correct.
**Fix:** emit the correct 2-operand sequences (copy + in-place shift/unpack, 2-op add).

### D9 — 256-bit unpack intrinsics were silently dropped
`UnpcklPs256/UnpckhPs256/UnpcklPd256/UnpckhPd256` were routed to
`emit_avx_fp_256_op` but had **no match arm**, falling into `_ => {}` and emitting
nothing — the result alloca stayed uninitialized (garbage).
**Fix:** added the four arms (`vunpcklps/vunpckhps/vunpcklpd/vunpckhpd`).

### D10 — Wrong VEX prefix on `vhaddps/vhsubps/vaddsubps`
These require the **F2** prefix (VEX pp = 3) but were encoded with pp = 2 (**F3**) → an
illegal instruction (**SIGILL**). The `pd` forms (66) were correct.
**Fix:** pp 2 → 3. Same class fixed for the FMA3 scalar forms: `vfmadd*{132,213,231}ss`
need F3 (pp=2) and `…sd` need F2 (pp=3); both were wrongly given 66 (pp=1). Added
`encode_avx_3op_38_pp_w1` for the W=1 (sd) forms.

### D11 — 128-bit vector copies emitted as 256-bit moves (SSE2 reduction corruption)
`emit_copy_value` contained a literal `TODO` and copied *every* vector with a 256-bit
`vmovupd`, reading 16 bytes past the source slot and writing 16 bytes past the
destination. Under `LCCC_FORCE_SSE2=1` the accumulator copies clobbered `main()`'s
locals (the loop bound itself was corrupted).
**Fix:** width-aware copy — 128-bit values (`vector128_values`) use `movupd`/`xmm0`,
256-bit use `vmovupd`/`ymm0`; the 128-bit class is propagated to the copy's dest.

### D12 — Dead/mis-encoding assembler arms
- 6 exact-duplicate `match` arms (vfmadd213/231ps·pd, vmaxps/pd, vminps/pd, vextractf128)
  removed (unreachable-pattern warnings, no behaviour change).
- `vpcmpeq*`/`vpcmpgt*` EVEX arms shadowed the mask-dest alias arm, so `%k`-destination
  forms were mis-encoded as vector-dest compares. Removed the shadowing arms; the alias
  arm now correctly handles mask-dest, rejects zmm vector-dest (like GAS), and xmm/ymm
  falls through to VEX.
- plain `cvtss2si/cvtsd2si` duplicate arms removed (handled by the q-suffixed arms).
- duplicate `Vbroadcastsd` arm removed.

Build warnings: **34 → 15** (the remaining 14 are `upper camel case` naming on
IntrinsicOp variants and one trailing semicolon — cosmetic, deferred deliberately to
avoid a large mechanical rename).

---

## 3. Gaps closed

- `__lccc_simd256_ps_vpermilvarps256` / `…vpermilvarpd256` were declared in
  `include/lcccsimd.h` but had **no backend lowering** and the assembler only knew the
  immediate `vpermilps` form. Added `VpermilvarPs256/VpermilvarPd256` (enum, lookup
  table, `emit_avx_fp_256_op`) and the variable-index encodings (0F38.0C/0D, operand-kind
  dispatch against 0F3A.04/05).

---

## 4. zlib-ng as test case (data)

- lccc **compiles zlib-ng's SIMD hot files cleanly** (`adler32_avx2.c`,
  `chunkset_avx2.c`, `slide_hash_avx2.c`). Other files only fail on missing *generated*
  config headers (`zconf-ng.h`, `zbuild.h` from a real cmake/configure run) — not
  compiler defects.
- **Correctness:** lccc's `adler32_avx2` over 100 KB of data returns `771462159`,
  identical to GCC and to Python's `zlib.adler32`.

| `zng adler32_avx2` (X86_AVX2, -mavx2 -O2) | instructions | runtime (5000 iters × 100 KB, min of 5, pinned) |
|---|---|---|
| **lccc** | **963** | **0.069 s** |
| GCC 14.2 | 674 | 0.013 s |
| Clang 19.1 | 504 | — |

**Root cause (the next big win):** lccc has no XMM/YMM/ZMM register allocation for
intrinsic chains. Every vector op is `vmovdqu %ymm0, N(%rsp)` … `vmovdqu M(%rsp), %ymm0`
round-trips through the stack (72 `vmovdqu` + 274 `movq` + 77 `leaq` in one function),
while GCC keeps ~15 ymm registers live across the loop and folds memory operands
(`vpmaddubsw (%rsi,%rdx), %ymm0, %ymm0`). The single-op `vec_live_regs` peephole is
insufficient. This is **the** highest-leverage improvement: a proper vector register
allocator + memory-operand folding would close most of the ~5× gap.

---

## 5. Remaining gaps & prioritized work items

| # | Work item | Evidence | Expected impact |
|---|---|---|---|
| 1 | Vector register allocation (XMM/YMM/ZMM) + memory-operand folding for intrinsic chains | adler32_avx2: 963 vs 674/504 instrs, **5.3× slower** | highest — every SIMD workload |
| 2 | Auto-vectorizer coverage: it only recognizes matmul-F64 and F64/I32 sum/dot reductions; no F32, no stores-into-loops, no if-converted, no unaligned/alias versioning, no `#pragma`/hint support | static analysis of `analyze_loop_pattern`/`analyze_reduction_pattern` | large for real scalar loops |
| 3 | `BroadcastLoadF64` hoisting is disabled by a `TODO` (register-allocation conflict) — A[i][k] is re-broadcast every inner iteration | comment in `transform_to_fma_f64x4` | matmul inner loop |
| 4 | `LCCC_FORCE_AVX2` env var is documented but never read; vector width selection ignores target feature detection (uses AVX2 by default on all x86-64) | `vectorize_with_analysis` | correctness on non-AVX2 hosts |
| 5 | `cvtss2si/cvtsd2si` (plain) are encoded with REX.W=1 even for a 32-bit dest (`%eax`); harmless for lccc's own codegen (sign-extended low 32 bits) but deviates from GAS for hand-written asm | `encoder/mod.rs` q-suffixed arms | minor compat |
| 6 | Mixed-width vector copies elsewhere should be audited for the D11 class (any other `ymm`-width copy of a 128-bit value) | D11 | correctness hardening |
| 7 | Duplicate logic in `vectorize.rs` (`transform_reduction_avx2`/`_sse2` are near-identical; `label_to_idx` rebuilt repeatedly) | static analysis | maintainability |

---

## 6. Regression tests added (`tests/regression/`)

Self-checking, return 0 on success, and **verified to fail on the pre-fix binary** and
**match GCC**:

- `vectorize_matmul_tail.c` — D1 (sentinel canary catches the row-end OOB write)
- `vectorize_dot_product.c` — D3
- `vectorize_float_matmul.c` — D4
- `vectorize_mixed_sum.c` — D5
- `vectorize_i32_sum.c` — I32 reduction guard
- `vectorize_sse2_path.c` (+ `.env`) — D2/D7/D8/D11 under `LCCC_FORCE_SSE2=1`
- `simd_vhaddps.c` (+ `.flags -mavx`) — D10
- `simd_blendv256.c` (+ `.flags -mavx`) — D6
- `simd_vpermilvar256.c` (+ `.flags -mavx`) — vpermilvar gap

`run_regression.sh` now sources a per-test `<name>.env` so tests can select
`LCCC_FORCE_SSE2=1`.

---

## 7. Validation matrix (post-fix)

| Suite | Result |
|---|---|
| 12 SIMD reproducers × {AVX2, SSE2} vs GCC | **all match** |
| `tests/intrinsics/run_intrin_tests.py` | **3/3** (t256_fp now compiles *and* runs) |
| `tests/regression/run_regression.sh` | **120/120** (5 `--compare-gcc` mismatches are pre-existing on clean main and unrelated: glibc f128/x87/TLS, abs-symbol, clmul256-gcc-SIGILL) |
| `tests/correctness/run_correctness.py` | **50/50** |
| zlib-ng `adler32_avx2` | correct output, matches GCC + zlib reference |

---

## 8. What a super-genius would do next

1. **Vector register allocation.** Give the backend a real XMM/YMM/ZMM allocator
   (e.g. an interval/priority allocator over the `vector_values` live ranges, extending
   the existing `vec_live_regs` idea from a 1-op peephole to whole-function intervals),
   plus memory-operand folding in `emit_avx_binary_256`/`emit_simd_op` so the *first*
   use of a slot can fold it. Target: adler32_avx2 from 963 → ≤ 550 instructions.
2. **Alias-analysis-driven auto-vectorization** for the general store-loop and
   mixed-type cases, with runtime versioning (`if (alias-free && aligned && N%4==0)`),
   and a cost model that consults register pressure (per the project's own §20–§25).
3. **Target-feature gating**: derive vector width from the driver's target features
   (verified via CPUID on the host, per §54) instead of the `LCCC_FORCE_SSE2` env var,
   and honor `LCCC_FORCE_AVX2` or remove it.
4. Turn `FmaF64x4Hoisted`/`BroadcastLoadF64` back on once (1) lands, eliminating the
   per-iteration `vbroadcastsd` from the matmul inner loop.
5. A "why wasn't this vectorized" diagnostic (per §57) wired to `LCCC_DEBUG_VECTORIZE`.

All fixes are committed on branch `simd-redteam-audit`; correctness is backed by the
reproducers, assembly, and the regression/correctness/intrinsics suites above.

---

# v2 (2026-08-13) — follow-up: MISMATCH resolution, project test-compiles, C23, F32 vectorization

## 1. MISMATCH failures resolved (123→124 regression passes, 0 failures)

All five `--compare-gcc` "MISMATCH" entries were **lccc-conformance tests whose
GCC reference is a defective oracle**, not lccc bugs (verified per test):

| Test | lccc | GCC oracle |
|---|---|---|
| glibc_f128_builtins | PASS | cannot compile (no-arg `__builtin_nanf128()`) |
| glibc_gottpoff | PASS | wrong TLS value (stale `@gottpoff` read) |
| glibc_x87_forms | PASS | SIGILL (GAS emits invalid x87 encoding) |
| regr_v2_abs_symbol_value | PASS | wrong value (GNU ld rebases absolute symbols) |
| regr_v5_clmul256 | PASS | cannot compile (256-bit VPCLMULQDQ gated behind AVX-512) |

Added a `LCCC_NO_COMPARE=1` per-test opt-out via `<name>.env`: the test still
runs and must pass under lccc, only the invalid GCC diff is skipped.

## 2. Test-compile results (with ms178/archpkgbuilds custom patches)

| Project | Result |
|---|---|
| zlib-ng (git) | **109/109** translation units compile with lccc |
| gzip 1.14 | **17/17** real build TUs; 130/140 incl. gnulib (the other 10 fail identically under gcc) |
| expat 2.8.2 | **5/5** real lib TUs (xcsinc.c is an #include file, not a TU) |
| glibc 2.44 + ms178-glibc.patch | **5/5 patched files** (e_atan2f, e_powf, s_sincosf, s_tanf, malloc) + 13/15 core sample (2 need generated headers, not lccc) |

**Runtime**: lccc-compiled zlib-ng `adler32_avx2`/`crc32_braid` return bit-exact
values matching Python's zlib (adler=209396577, crc=573088776).

## 3. binutils 2.47 oracle
Built minimal binutils 2.47.20260726 (`as`, `ld`, `objdump`, `readelf`, `nm`, …)
at `/home/user/binutils-oracle/bin` as an assembler/linker oracle.

## 4. C23 `alignas`/`alignof`
gzip 1.14 builds with `-std=gnu23` and uses `alignas (4096)` as a declaration
specifier (via `DECLARE`); lccc only knew `_Alignas`/`_Alignof`. Added the C23
keyword spellings (mapped to the existing token kinds) + `c23_alignas.c`
regression test.

## 5. F32 reduction auto-vectorization (validated 2× win)
lccc detected F32 sum/dot-product reductions and then **rejected** them
("Unsupported type for AVX2: F32"). Added `Vec{Load,Add,Mul,Zero,HorizontalAdd}F32x{8,4}`
intrinsics + lowering, plus three correctness fixes uncovered while wiring it:
- accumulator phi-entry rewiring only matched F64/I32 zero constants (F32 vector
  zero was DCE'd → 8-byte zero vs 32-byte load);
- AVX2 horizontal add dropped 2 lanes; SSE2 horizontal add shuffled a stale register;
- F32 vector values weren't excluded from GPR register allocation (missing
  `non_gpr_values` entries) → pointer-mediated accumulator access, lost stores.

**Benchmark** (F32 dot, 1M floats, min of 5 runs, pinned):
| compiler | time |
|---|---|
| **lccc** | **0.124 s** |
| GCC 14.2 | 0.246 s |
| Clang 19.1 | 0.247 s |

F32 sum/dot match GCC bit-exactly for N ∈ {1..257, 1024} in both AVX2 and SSE2 modes.

## 6. Remaining top-priority work (v3)
1. **Vector register allocation** across the loop backedge (the adler32 5.3× gap
   root cause: loop-carried accumulators are live across iterations, so the v5
   store-defer analysis correctly cannot elide them; a real XMM/YMM interval
   allocator is required). Data: adler32_avx2 = 963 instrs (lccc) vs 674 (gcc) /
   504 (clang).
2. **F32/F64 store-loop vectorization** (the pattern matcher still only handles
   reductions and the double matmul).
3. Memory-operand folding for the first (last-use) reference of a vector slot.
4. matmul `BroadcastLoadF64` hoisting (documented TODO, blocked on register
   allocation).

---

# v3 (2026-08-13) — novel-ideas execution: diagnostics + reduction-loop codegen

## Delivered (all validated: regression 126/126, correctness 50/50, intrinsics 3/3)

1. **"Why not vectorized" diagnostics (goal §57).** The auto-vectorizer records the
   most specific rejection reason at every bail-out site; `LCCC_WHY_NOT_VECTORIZE=1`
   prints a one-line-per-loop summary. Purely diagnostic.

2. **Defer-aware folding of auto-vectorizer Vec* ops.** The Vec{Load,Add,Mul}
   emitters previously bypassed the register-cache / deferred-store machinery and
   round-tripped every vector op through the stack (~17 instrs per F64 dot
   iteration). They now route through `emit_avx_binary_256`/`emit_sse_binary_128`
   (memory-operand folding + deferred single-use loads + register cache), and
   `compute_vector_defer_values` recognizes Vec* SSA `dest` producers.

3. **Commutative rename elimination** in `emit_avx_binary_256`: `op m0, %ymm0, %ymm0`
   instead of `vmovdqa` rename + fold.

4. **Constant-zero offset materialization removed** from VecLoad* (the vectorizer
   passes `(array_gep, 0)`): `vmovupd (%rax), %ymm0` instead of
   `xorl %ecx,%ecx; vmovupd (%rax,%rcx), %ymm0`.

5. **resolve_slot_addr** now treats `vector_values`/`vector128_values` as Direct
   slots (slot holds vector data, not a pointer) — removes leaq+indirect-load
   round-trips.

## Measured (min-of-5, pinned core, warm cache)

| kernel | lccc (before v3) | lccc (v3) | gcc | clang |
|---|---|---|---|---|
| F64 sum 1M | 0.658 s | **0.624 s** | 0.616 s | — |
| F64 dot 1M | 0.262 s | **0.250 s** | 0.247 s | — |
| F32 dot 1M | 0.124 s | **0.124 s** | 0.246 s | 0.247 s |

F64 reductions went from ~6% slower to within measurement noise of GCC; F32 dot
remains ~2× faster than GCC/Clang.

## Remaining top-priority work (v4)
1. **Loop-carried accumulator in a YMM register** — the last 4-5 instr/iter on
   reduction loops (sum copy + acc load/store) need 256-bit vector register
   allocation (YMM PhysReg class + Phase 3c + Vec* copy coalescing). This is the
   adler32 5.3× gap too (hand-written intrinsic chains).
2. **Reduction-loop address strength reduction** (element-index IV + per-iter
   shl/leaq vs the byte-offset IV the matmul path already uses).
3. Store-loop / general loop vectorization (dataflow-driven).
4. matmul `BroadcastLoadF64` hoisting (still blocked on register allocation).

---

# v4 (2026-08-13) — byte-offset IV strength reduction + latent dot-product fix

## Delivered

1. **Byte-offset IV for reduction loops.** F64/F32/I32 sum and dot products (AVX2
   and SSE2) now step a byte-offset induction variable (stride = elem_size ×
   vec_width) instead of an element index, mirroring the matmul path:
   - the GEP offset becomes the byte IV itself (per-iteration shl/leaq/scale
     chain removed),
   - VecLoad takes `(base, byte_iv)` SIB addressing directly,
   - the loop bound becomes `floor(N/vec_width) * byte_stride` (running ceil
     iterations would read/write past the array end — the OOB class fixed in
     the matmul path),
   - the scalar remainder start becomes `byte_iv_final / elem_size`.
   Falls back to the element-index scheme when the offset chain is not the
   canonical `shl/mul(iv_cast, elem_size)` shape.

2. **Latent dot-product bug fixed.** The dot-product transform removed two
   instructions at `accumulator_add_idx + 4`, which only deleted dead scalar
   code while the element-index Step 2 kept inserting per-GEP multiplies that
   shifted indices. Under byte-offset IV the same removal deleted the loop's
   induction-variable increment (infinite loop / undefined IV). It now removes
   only the dead scalar add and lets DCE reclaim the dead multiply.

## Validation

- 5 reduction kernels × N ∈ {0..300} × {AVX2, SSE2}: bit-exact vs scalar
  reference and GCC (edge cases 0, 1, 3, 4, 7, 8 included).
- Large-N (10⁶) exact in both modes.
- regression 126/126, correctness 50/50, intrinsics 3/3.

## Benchmarks (min-of-5, pinned core)

| kernel | lccc v4 | gcc |
|---|---|---|
| F64 sum 1M | 0.620 s | 0.614 s |
| F64 dot 1M | 0.252 s | 0.249 s |
| F32 dot 1M | 0.123 s | 0.245 s |

## Remaining top-priority work (v5)

1. **YMM register allocation for loop-carried accumulators** — closes the
   per-iteration accumulator copy + temp-store/load (3–4 instr/iter on every
   reduction loop) and the adler32 5.3× intrinsic-chain gap. Needs a YMM
   PhysReg class with XMM/YMM aliasing awareness (or width-partitioned pools).
2. Remove the dead strength-reduced pointer that IVSR emits on top of the
   byte-offset SIB addressing (1 instr/iter).
3. Store-loop / general dataflow-driven loop vectorization.
4. matmul `BroadcastLoadF64` hoisting (unblocks once registers land).

---

# v5 (2026-08-13) — DCE gap, reduction-loop codegen, systemic scalar-FP fix

## Root causes found and fixed

1. **The v4 DCE gap (user-reported).** The vectorizer orphaned the scalar
   load/mul/GEP chains and relied on the global DCE pass — which runs AFTER
   IVSR. IVSR strength-reduced the already-dead GEPs into loop-carried pointer
   increments (`leaq 256(%rdi),%rdi`) that DCE cannot remove (the increment and
   its phi only reference each other). The vectorizer now runs DCE on the
   function immediately after its transforms, before IVSR sees the orphaned
   chains. DOT loop 17 -> 11, SUM 11 -> 8 instructions.

2. **Reduction-loop register reuse.** VecLoad now reuses register-allocated
   base/offset GPRs (`vmovupd (%rdi,%rsi),%ymm0`) instead of copying them into
   rax/rcx; and the accumulator phi group {acc, init_zero, vec_sum} coalesces
   to a single home slot (CFG copy-coalescing was blanket-excluding vector
   values). SUM 6, DOT 9 instructions/iteration.

3. **Systemic scalar-FP fix.** Scalar F64 values were register-allocated to
   XMM, but every producer/consumer round-tripped through GPRs
   (`movq %xmm0,%rax; movq %rax,%xmmN`) because only the binop RHS honored the
   assignment. Now: FP binops honor XMM homes for LHS and dest AND compute
   directly into the destination register; int->float casts emit
   `cvtsi2sd/cvtsi2ss` straight into the dest register; F64/F32 loads emit
   `movsd/movss (%ptr),%xmmN`. The F64 pool keeps xmm2 unless the function
   contains an intrinsic that clobbers it (pblendvb/128-bit VNNI/F128).

## Measured (min-of-N, pinned core, vs gcc -O2)

| benchmark | before (v4) | v5 |
|---|---|---|
| spectral_norm | 9.3x | **2.9x** |
| struct_copy | 20.9x | **6.7x** |
| nbody | 3.8x | **4.8x** (11.9x under an earlier broken measurement) |
| mandelbrot | 3.2x | 2.9x (compare/branch bound) |

Correctness: nbody / spectral_norm / mandelbrot bit-exact vs gcc; regression
127/127, correctness 50/50, intrinsics 3/3; reduction difftests 0..300 x
{AVX2,SSE2} bit-exact; zlib-ng adler32 correct.

## Remaining top-priority work (v6)

1. **Parameter/struct-ABI register allocation**: functions spill all GPR/F64
   parameters to stack at entry (the nbody/struct_copy/linux_find_bit prologue
   bloat) and struct-by-value copies use vmovdqu+shuffle. Dominates the
   remaining 4-7x gaps.
2. **Loop-carried scalar accumulator in an XMM register** (allocator reuse
   across the chain + phi-value F64 allocation) — the last 2 movsd/iter.
3. **Dataflow-driven vectorization** of reductions with computed operands
   (spectral_norm's `1.0/f(i,j) * v[j]`), multi-reduction loops, and
   store-loops.
4. Branch/switch codegen (expat 3.2x, sqlite_varint 2.0x, switch_dispatch 1.5x)
   and bit-op selection (bitops 1.9x, linux_find_bit 1.8x).
5. matmul `BroadcastLoadF64` hoisting.

---

# v6 (2026-08-13) — FP register class xmm8-15, scalar-FP XMM codegen, GEP CSE

## Delivered (all validated; regression 127/127, correctness 50/50, intrinsics 3/3)

1. **F64 register class widened xmm2 → xmm2-xmm15 (14 regs).** The allocator
   was spilling FP intervals (nbody has 58) even though xmm8-15 were free and
   codegen-scratch only touches xmm0/1/2. `phys_reg_name`/`is_xmm_reg` extended
   to the REX-extension bank; the assembler already encodes it.

2. **Scalar FP intrinsics now XMM-allocated.** sqrt/fabs results were excluded
   from both the GPR and XMM scans, so their result spilled mid-chain
   (nbody's `sqrtsd` + spill + reload). Now `sqrtsd %xmmN,%xmmN` in place;
   fabs is a single `andpd/andps` against a rodata mask.

3. **FP register-direct loads/stores** — for materialized pointers AND for
   GEP-folded constant offsets (`movsd 8(%r8),%xmm5`): the fold path only had
   an integer register-direct fast path; F64/F32 fell to the GPR round-trip.

4. **GEP CSE re-enabled in GVN.** Two GEPs with the same base+offset collapse
   to one + a coalesced Copy (was disabled for a pre-register-aware stale-
   register defect). nbody recomputed `bodies + i*56` once per field access
   (8 leaq + 8 scratch-slot spills); now once.

## Measured (min-of-5, pinned, vs gcc -O2)

| benchmark | v5 | v6 |
|---|---|---|
| mandelbrot | 2.87x | **2.12x** |
| nbody | 4.79x | **4.25x** |
| spectral_norm | 2.9x | 2.9x |
| struct_copy | 6.7x | 6.7x |

Correctness: nbody/spectral_norm/mandelbrot/struct_copy bit-exact vs gcc.

## Remaining top-priority work (v7) — precisely diagnosed

1. **Load→FP-op register coalescing**: the allocator assigns the load and the
   consuming binop different XMM registers, leaving `movsd %xmm2,%xmm4` before
   every op (the reg_hint field in the linear scan is unused). This plus
   folded-offset store coalescing is most of the remaining nbody gap.
2. **Affine IVSR**: `mul(add(iv,k),stride)` (nbody's `(i+1)*56`) is not
   recognized; only `mul(iv,c)` is. Two imulq per outer iteration survive.
3. **Parameter register allocation**: F64/GPR params spill to stack at entry
   (`movq %xmm0,216(%rsp)`); struct-by-value copies still vmovdqu+shuffle.
4. **F32 XMM allocation**: the XMM scan filters `ty == F64` only; F32 values
   still go through the stack (the emitters already honor XMM homes, so this
   is a one-line filter widening plus validation).
5. Branch-heavy workloads (expat 3.2x, sqlite_varint 2.1x): boolean
   `setcc+movzbl+test` chains instead of fused compare-to-branch.

---

# v7 (2026-08-13) — FP register coalescing, F32 XMM class, affine IVSR

## Delivered (validated; regression 127/127, correctness 50/50, intrinsics 3/3)

1. **Post-allocation F64/F32 producer→consumer register coalescing.** A
   single-use scalar-FP value feeding a binary op is reassigned to its
   consumer's destination register, so loads land directly in the op's register
   and the reg-to-reg copy that preceded every FP operation disappears.
   LHS-only (the emitter loads the LHS into the destination; coalescing the RHS
   there would let the LHS load clobber it — caught in development as
   `subsd %xmm4,%xmm4` = 0), processed in reverse program order so a chain
   folds onto the tail register, with explicit interval-conflict checks. The
   linear scan is untouched.
   During development, an iterated linear-scan-hint approach was tried and
   reverted: it miscompiled `energy()` via a subtle adjacent-interval liveness
   interaction with the `reg_free_until` bookkeeping (end+1 off-by-one vs
   `overlaps_with`'s inclusive-end semantics). The post-pass avoids the
   allocator internals entirely.

2. **F32 scalars now XMM-allocated** (was F64-only). Also fixed a latent panic
   the widening exposed: the register-direct int→float cast read an
   immediately-consumed source via `value_to_reg`, which panics when there is
   no slot/register (accumulator-cached); switched to `operand_to_rax`.

3. **Affine IVSR** — `(iv + k) * stride` now strength-reduces with initial
   offset `(init + k) * stride` (non-constant init + non-zero offset falls
   back conservatively). Synthetic `(i+1)*2` kernel: zero `imulq` remain.

## Measured (min-of-5, pinned, vs gcc -O2)

| benchmark | v6 | v7 |
|---|---|---|
| nbody | 4.25x | **3.48x** (advance copies 34→11) |
| mandelbrot | 2.12x | **1.89x** |
| struct_copy | 6.65x | **6.27x** |

Correctness: 14 benchmarks bit-exact vs gcc; F32/F64 difftests 0..300 exact;
zlib-ng adler32 correct.

## v8 roadmap (diagnosed this session)

1. **Branch/boolean domain** (expat 3.2x, sqlite_varint 2.1x): range-check
   folding `(x>=lo && x<=hi) -> (unsigned)(x-lo) <= hi-lo`, fused
   compare-to-branch for short-circuit booleans, and GPR boolean coalescing
   (redundant `movl %esi,%ebp; movl %ebp,%r15d` copies). GCC's approach for
   expat is direct `cmpb $X,%dl; ja/jle` range branches with zero boolean
   materialization.
2. **Parameter register allocation** (nbody's `movq %xmm0,216(%rsp)` at entry).
3. Loop-carried scalar accumulator in an XMM register (the last 2 movsd/iter in
   reduction loops).
4. nbody's remaining `imulq` are preheader pointer-inits for non-constant IV
   inits (reuse `&bodies[i]+56` for `&bodies[i+1]`).
