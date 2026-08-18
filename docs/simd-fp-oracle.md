# SIMD/FP Compiler Explorer oracle audit

**Date:** 2026-08-18  
**Base:** `1abb407e597f749a96bd851cb671f0c169f61012`  
**Target used for screening:** x86-64-v3, AT&T syntax

This document records the reproducible static-code audit introduced by
`tests/benchmark/patterns/simd_fp_oracle.c`, `scripts/godbolt.py`, and
`scripts/codegen_scoreboard.py`.  It is deliberately not a claim that LCCC is
faster than another compiler: instruction counts and mnemonic heuristics are
triage signals, not cycle counts.  The research VM exposes no hardware PMU, so
Raptor Lake performance claims must wait for counter-backed measurements on the
target i7-14700KF.

## Live oracles

The scripts query Compiler Explorer's compiler catalogue rather than assuming
that an old ID is still current.  This audit resolved:

| Oracle | Compiler Explorer ID |
|---|---|
| GCC 16.2 | `cg162` |
| Clang 22.1.0 | `cclang2210` |
| classic ICC 2021.10.0 | `cicc2021100` |
| latest ICX | `cicxlatest` |

`godbolt.py compare` records both the requested alias and resolved compiler
metadata.  The scoreboard requests AT&T syntax explicitly; parser/cache schema
versions are included in the cache key so old Intel-syntax or misparsed results
cannot be reused silently.  Quoted ELF function labels are supported.

## Corpus

`simd_fp_oracle.c` contains 50 independently measurable functions.  They cover:

* contiguous F32/F64/I32 maps: add, multiply, scale, triad, clamp, copy, and
  conversion;
* affine/FMA kernels and dot products;
* scalar and multi-accumulator reductions, including sums, products, min/max,
  sum of squares, and four independent sums;
* masks, comparisons, select, sqrt, reciprocal/division, and normalization;
* three- and five-point stencils;
* interleaved/complex arithmetic and small fixed-vector distance;
* strided/gather-like accesses and integer/FP conversions;
* an Adler-like dependent recurrence representative of zlib-family code.

The corpus is standalone C so the identical source reaches LCCC and all four
remote compilers.  Strict and permissive FP modes are separate experiments:

```sh
python3 scripts/codegen_scoreboard.py \
  tests/benchmark/patterns/simd_fp_oracle.c \
  --flags '-O3 -march=x86-64-v3' --json strict.json

python3 scripts/codegen_scoreboard.py \
  tests/benchmark/patterns/simd_fp_oracle.c \
  --flags '-O3 -march=x86-64-v3 -ffast-math -ffp-contract=fast' \
  --json fast.json
```

The local compiler defaults to `target/fastbuild/lccc` when available.  That
binary is built at Rust `-O1 -j2`; `fastbuild` only removes LTO and enables
incremental compilation for the edit/measure loop.

## Baseline findings

For each function, “gap” is LCCC's parsed instruction count minus the smallest
count among the four live oracles.  It is not a throughput estimate.

| Mode | LCCC gap to per-function best | Behind | Tied | Ahead |
|---|---:|---:|---:|---:|
| strict FP | +423 | 29 | 2 | 19 |
| fast FP | +563 | 33 | 3 | 14 |

The largest fast-mode deficits were:

| Pattern | Instruction gap |
|---|---:|
| `p17_dot_f32` | +36 |
| `p18_dot_f64` | +36 |
| `p13_affine_i32` | +35 |
| `p39_stencil5_f32` | +34 |
| `p12_affine_f32` | +33 |
| `p48_four_sums_f32` | +31 |
| `p23_sum_squares_f32` | +30 |

This result does **not** establish that the smallest oracle is optimal.  It
identifies code shapes for root-cause analysis.  For example, LCCC's F64 dot
has 67 instructions versus GCC 96, Clang 70, ICC 90, and ICX 31.  LCCC beats
three references by this metric but remains far behind ICX; copying GCC's
strategy would therefore be counterproductive.

The conservative `vector` metric recognizes packed SIMD mnemonics and YMM/ZMM
registers but excludes scalar VEX instructions such as `vaddsd`.  Loads,
stores, stack spills, and branches are reported independently.  Any future
claim based on these fields must first inspect the saved assembly, because a
text classifier cannot model fused-domain uops, dependencies, cache behavior,
or branch prediction.

## First distilled fix: destructive scalar-FMA coalescing

`p50_distance3_f64` was the first small, high-confidence target.  Baseline
fast-mode counts were LCCC 28, GCC 9, Clang 9, ICC 12, and ICX 9.  LCCC already
recognized both FMAs, but generated each through `%xmm0/%xmm1`:

```asm
movsd accumulator, %xmm0
movsd multiplicand, %xmm1
vfmadd231sd multiplicand, %xmm1, %xmm0
movsd %xmm0, accumulator
```

The x86 scalar-FMA emitter now detects when linear scan coalesced the FMA result
with the accumulator's XMM home.  If neither multiplicand aliases that
destructive destination, it emits one FMA directly into the assigned register.
The alias test is mandatory: loading an aliased multiplicand after overwriting
the accumulator would silently change the expression.  Stack-homed values are
handled by the same guarded path; all uncertain cases retain the old fallback.

Post-fix `p50_distance3_f64` is 22 instructions: six instructions removed, no
new spills, and the core is now:

```asm
vmulsd      %xmm2, %xmm2, %xmm2
vfmadd231sd %xmm4, %xmm4, %xmm2
vfmadd231sd %xmm6, %xmm6, %xmm2
```

`tests/regression/fma_dest_coalesce.c` checks runtime semantics for a two-FMA
distance, F32 chaining, destructive-destination alias fallbacks, and incoming
stack FP parameters.  `check_fma_dest_coalesce_codegen.sh` is run by the main
regression harness and locks in both sides of the contract: direct accumulation
when legal and `%xmm0` fallback when an operand aliases the destination.

The remaining 13-instruction gap in distance3 is separately attributable to
callee-saved pointer copies/prologue overhead, missed scalar memory-operand
folding, and lack of local SLP packing.  Those should be addressed independently
and measured after each change rather than hidden inside a broad rewrite.

### Second distilled fix: one-block leaf parameter homes

The remaining pointer copies and save/restore frame were an allocator policy
problem.  `ParamRef` values were excluded from the caller-saved allocation
phase, so even a one-block call-free function paid RBX/R12 push/pop overhead.
One-block x86-64 leaves now prefer incoming ABI registers as parameter homes
(RDI, RSI, RDX, then the remaining safe caller-saved pool).  Any required
moves are emitted as an ordered parallel copy after stack-backed arguments have
been captured; cycles are broken through reserved RAX.  Multi-block functions
retain the old policy because moving long-lived loop bases into R11/R10 displaced
hotter values and regressed the corpus.

This takes `p50_distance3_f64` from the post-FMA 22 instructions to 14 (28 to 14
cumulatively), with no stack frame, callee-save traffic, or pointer relay.  A
controlled A/B using `CCC_NO_LEAF_PARAM_GPR=1` changed only p50 in the 50-pattern
fast corpus: -8 instructions, zero static regressions.  The new six-argument
regression also exercises the dependency-ordered `r9 -> r11`, `r8 -> r9`,
`rcx -> r8` copy chain and verifies output independently.

A CPU-0-pinned, randomized 20-pair VM timing screen (100 million calls/sample)
measured a paired geometric new/old ratio of **0.9851**, bootstrap 95% interval
**[0.9670, 1.0012]**, with the new binary faster in 15/20 pairs.  The interval
includes no change: this is suggestive screening, not a speedup claim.  Raw
samples are retained in `artifacts/simd-fp-oracle/distance3/leaf-gpr-paired-timing.json`.
Static code size/instruction count is proven; bare-metal PMU validation remains
required.

### Third distilled fix: scalar VEX memory-source folding

The peephole now folds an adjacent single-use `movss`/`movsd` into scalar VEX
add/sub/mul/div.  Its safety proof is intentionally conservative: source and
destructive destination must differ, load and consumer must be adjacent, and
the loaded XMM register must not be mentioned again before the function's
`.size`.  Multi-use and destructive-self cases remain unchanged.  Four focused
unit tests cover the positive case, both rejection cases, and the XMM1/XMM10
register-name boundary.

A skip-controlled 50-pattern A/B (`CCC_PEEPHOLE_SKIP=fp_reg_mem_fold`) improved
12 functions, removed 19 instructions in total, and regressed none.  Largest
wins were stencil5 (-4), distance3 (-3), and both triads (-2 each).  Distance3
is now **11 instructions** versus its original 28 and the best oracle's 9.

Unlike the latency-bound distance3 microbenchmark, the affected loop gives a
clear timing signal.  A CPU-0-pinned randomized 24-pair screen of a 65,536-item
five-point F32 stencil (1,000 calls/sample) measured a paired new/old geometric
ratio of **0.8694**, bootstrap 95% interval **[0.8474, 0.8930]**; the folded
binary won 23/24 pairs.  This is strong VM screening evidence for the
front-end/code-size improvement, still not a substitute for Raptor Lake PMU
measurements.  Raw samples and the exact harness are retained under
`artifacts/simd-fp-oracle/fp-memfold-stencil5-*`.

### Fourth distilled fix: register-resident vector reductions

The x86 allocator now keeps only proven loop-carried F32/F64 reduction webs in
SIMD registers.  The admission proof tracks exact element class and width
(F32x8/F64x4 AVX or F32x4/F64x2 SSE), requires a same-class Copy web, accepts
only zero/load/add/mul/horizontal-reduce operations, and rejects calls, inline
assembly, memcpy, unrelated intrinsics, unsupported uses, and call-spanning
intervals.  Arbitrary SIMD values continue to use protected stack homes.  The
emitter selects XMM or YMM names from the shared physical-register identity,
coalesces the backedge result with its accumulator, and updates it directly.
`CCC_NO_REDUCTION_VECREG=1` preserves a same-binary control.

A controlled 50-pattern A/B improved the five affected reductions (`p15`-`p18`
and `p23`) by one static instruction each with no regression.  More importantly,
each loop removed two accumulator loads and two stores/spill references from
the static function body.  A CPU-0-pinned randomized 24-pair screen of a
65,536-item F32 sum plus dot workload (5,000 calls to each per sample) measured
a paired register/stack geometric ratio of **0.5021**, bootstrap 95% interval
**[0.4589, 0.5908]**; the register version won 23/24 pairs.  This large signal
is still VM screening—not Raptor Lake PMU evidence.  The exact benchmark,
runner, raw samples, and static A/B are retained as
`reduction_vecreg.c`, `benchmark_reduction_vecreg_ab.py`, and under
`artifacts/simd-fp-oracle/reduction-vecreg-*`.

The live post-fix CE refresh resolved GCC 16.2 `cg162`, Clang 22.1.0
`cclang2210`, ICC 2021.10 `cicc2021100`, and latest ICX `cicxlatest`.  Across
the five reduction functions the local static aggregate was 325 instructions,
versus GCC 420, Clang 347, ICC 346, and ICX 157.  LCCC therefore beats three
references on this coarse aggregate but remains far behind ICX; no blanket
superiority claim is made.

A separate attempted remainder-loop optimization was rejected during full-suite
validation.  Replacing the vectorizer's late signed `/ 8` with `UDiv`, `LShr`,
or a second div-by-constant pass made `vectorize_matmul_tail` skip element 16
when `n == 17`.  The correct signed division is retained even though it emits
`idivl`; the underlying SSA/narrowing interaction must be fixed before removing
that cost.  This is recorded to prevent a static-metric win from reintroducing
the miscompile.

## Correctness policy discovered during the audit

Strict IEEE source order is now the default for FP reductions.  Reassociation
and reduction vectorization require an explicit fast-math/reassociation option;
the umbrella `-ffast-math` option defines `__FAST_MATH__`, while individual
reassociation options follow GCC and do not.  Pointer-root aliasing is conservative:
unrestricted parameter roots may overlap and block vectorization; C `restrict`
propagates to IR `noalias` and can prove the transformation legal.  Dedicated
regressions cover strict ordering and overlapping map/matmul inputs.

## Next measured targets (deferred to a new session)

The current delivery stops after the validated register-resident reduction fix;
do not widen its whitelist opportunistically.  A future agent should start from
the retained A/B and live-oracle artifacts, then address in this order:

1. fix the vectorizer's signed remainder-index SSA/narrowing defect before
   replacing the emitted `idiv`, then reduce loop setup/tail overhead;
2. eliminate the remaining dot-product transient stack temporary and evaluate
   vector FMA only under the existing fast-contract legality rules;
3. support multiple independent reductions (`p48`) without accumulator spills;
4. broaden legal store-loop vectorization and add width-aware SLP for fixed
   vectors/stencils;
5. graduate promising zlib-ng/gzip, expat, SQLite, Linux, and glibc kernels to
   pinned end-to-end workloads with provenance.

For every item, retain strict-vs-fast differential execution, local/remote
assembly, individual gains and regressions, and raw randomized paired timing
samples.  VM timing remains screening evidence; PMU-backed bare-metal results
are required before asserting superiority on Raptor Lake.
