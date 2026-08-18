# SIMD/FP Compiler Explorer oracle audit

**Date:** 2026-08-18  
**Original audit base:** `1abb407e597f749a96bd851cb671f0c169f61012`

**Current rebased delivery base:** `208bbfaed003a71fd5b1d0ed20885a195f065c01`

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

`simd_fp_oracle.c` contains 53 independently measurable functions.  They cover:

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

### Fifth distilled fix: division-free vector remainder transitions

The rejected remainder-loop experiment was reproduced before changing the
transformation.  Its `n == 17` failure was not caused by shift semantics: the
late signed division had accidentally disabled RDX allocation for the entire
function.  Replacing it with `LShr` made the vector byte IV live in RDX, but the
packed-FMA intrinsic emitter also used RDX as an unmodeled fixed scratch.  The
first vector iteration overwrote the IV with a pointer; the corrupted exit
index then skipped element 16.

The x86 allocation pre-scan now excludes RDX when an intrinsic emitter has a
fixed RDX clobber, independently of whether an unrelated divide happens to be
present.  With that liveness defect repaired, non-negative, zero-based byte IVs
use `LShr` by `log2(element_size)` in both matmul and F32/F64/I32 reduction
remainder transitions.  Generated AVX2 assembly uses `shrl $2/$3` and contains
no tail `idivl`.  The expanded deterministic boundary sweeps cover 0, every
vector-width edge through 65, and larger 127/128/129-style edges; a structural
regression requires both vector instructions and division-free tail shifts.

A clean-base/current static A/B compared 56 functions from the 50-pattern
corpus plus the matmul regression: **8 improved, 48 tied, 0 regressed**, with
aggregate instructions **2310 -> 2281** (-29), arithmetic mean ratio 0.99181,
and geometric mean ratio 0.99158.  Dot F32/F64 each lose five instructions;
matmul and the other affected sums lose three or four.  Fresh live CE output
for the five distilled sum/dot/row kernels is retained under
`artifacts/simd-fp-followup/remainder/live-oracles*`: LCCC moved from 286 to
270 aggregate instructions; GCC 16.2, Clang 22.1, ICC 2021.10, and latest ICX
were 355, 332, 379, and 150 respectively.  A CPU-0-pinned randomized 24-pair
VM screen over small dynamic F64 sum+dot bounds had 22/24 division-free wins,
but a severe scheduler outlier widened the paired new/base geometric ratio to
**0.9970** with bootstrap 95% interval **[0.9110, 1.1633]**.  The timing result
is therefore explicitly unresolved.  Raw samples, compiler hashes, and the
exact runner are retained under `artifacts/simd-fp-followup/remainder/`.
Static counts are deterministic; these noisy wall times are VM screening
rather than PMU-backed Raptor Lake or bare-metal evidence.

### Sixth distilled fix: contract-aware fused dot reductions

The remaining dot-product stack traffic was caused by two sequential vector
loads: loading B forced the deferred A value out of YMM0 into a 32-byte stack
home before multiply.  Under an explicit fast-contraction contract the AVX2
vectorizer now emits one typed fused reduction intrinsic carrying the
accumulator plus both base/offset pairs.  x86 loads A into YMM0, folds B as the
memory operand of `vfmadd231ps/pd`, and updates the coalesced accumulator YMM
register directly.  No transient vector value or stack home is created.

FMA legality is now independent compiler state rather than being inferred from
reassociation: `-ffp-contract=fast` and `off` obey last-option-wins, while
`-fassociative-math -ffp-contract=off` retains separate packed multiply/add.
CLI unit coverage and a two-mode assembly regression lock down that contract.
On the 50-pattern corpus, p17, p18, and p23 each improve by three instructions
and two stack references; **3 improve, 47 tie, 0 regress** (1838 -> 1829).
The live GCC/Clang/ICC/ICX refresh is under `artifacts/simd-fp-four/fma-dot/`;
the five-kernel LCCC aggregate moves 270 -> 264 instructions. These are static
VM/oracle results, not PMU-backed target-hardware claims.

### Seventh distilled fix: multiple scalar FP reductions in registers

Post-phi scalar FP accumulators are represented as multi-def `Copy` values and
therefore no longer carry an explicit result type.  x86's scalar XMM allocator
previously recognized only typed arithmetic producers, leaving p48's four
loop-carried sums in four stack slots despite fourteen available XMM registers.
Typed FP copy-web recovery now runs on x86 as well as AArch64, excludes
copy-only dead webs through real-use propagation, and reuses the existing
same-block destructive-backedge proof.  The GPR and FP coalescing paths also
share revalidation logic instead of maintaining divergent safety checks.

`multiple_fp_reductions.c` sweeps boundary counts for four simultaneous F32 and
F64 sums; its structural companion requires four distinct in-place XMM
accumulators and a kill-switch control with stack homes.  A controlled 50-case
A/B reports **15 improved, 35 tied, 0 regressed**, aggregate 1829 -> 1742,
with p48 at 63 -> 46 instructions and 16 -> 0 classified stack references.
Artifacts are retained under `artifacts/simd-fp-four/multi-reduction/`.

### Eighth distilled fix: width-aware affine store loops

The old map recognizer accepted only the full `I32/F32 load * scale + bias`
shape. It also rebuilt a multiply and add unconditionally in the remainder,
used a fixed four-byte element offset, treated signed dynamic bounds as
unsigned during vector-trip computation, and materialized both packed-loop
GEPs. The resulting path was narrow, had latent F64/negative-bound correctness
gaps, and spent several integer instructions per vector iteration constructing
addresses.

The recognizer now covers the complete one-source affine family—copy, scale,
add, and scale-plus-bias—for F32, F64, I32, and U32 elements with signed or
unsigned 32/64-bit induction. Legality requires a canonical contiguous
`element index * sizeof(element)` address, either disjoint proven object roots
(`restrict`, separate globals/allocas) or exactly identical in-place GEPs.
Shifted overlapping accesses remain scalar. Signed power-of-two trip
quotients use an explicit bias/shift sequence, so negative bounds execute zero
iterations without an unsigned reinterpretation or `idiv` in the loop.

A separate I64 byte-offset phi advances by the packed byte width while the
source IV remains an element counter. x86 therefore emits one indexed load,
one indexed store, and one byte-IV increment instead of two LEAs plus repeated
cast/scale work. Width selection is F32/I32/U32 x8 and F64 x4 on AVX2, x4/x2
on the 128-bit diagnostic/AArch64 paths. Loop-invariant broadcasts receive
narrowly proven XMM/YMM homes and are consumed directly by packed arithmetic;
the store consumes a deferred result without writing its dead SSA home.
`CCC_NO_MAP_VECREG=1` retains a stack-home control. Under an explicit fast
contraction contract, full FP affine maps use `vfmadd132ps/pd`; strict mode
retains separate multiply and add operations.

The boundary regression checks negative, zero, every packed-width edge through
79, F32/F64/I32, unsigned bounds, I64 induction, exact in-place operation, and
a shifted-overlap dependence. Its structural companion checks AVX2 and SSE
widths, copy/scale/add/full-affine shapes, alias rejection, direct broadcast
registers, absence of the dead packed-result spill, both kill switches, and
strict-versus-fast FMA selection. The same F32/F64/I32 intrinsic family is now
lowered on AArch64 rather than silently falling through an x86-only arm.

A clean pre-item/current static A/B over all 50 corpus functions deliberately
shows the code-size cost instead of hiding it. Strict mode is **0 improved, 47
tied, 3 larger**, aggregate **1580 -> 1665**, arithmetic ratio 1.05380 and
geometric ratio 1.05142. Fast mode is **1 improved, 46 tied, 3 larger**,
aggregate **1742 -> 1826**, arithmetic ratio 1.04822 and geometric ratio
1.05106; p12 improves 60 -> 59 through contract-legal packed FMA. The three
larger functions are newly vectorized p01 copy F32 (21 -> 48), p02 copy F64
(21 -> 48), and p36 constant-scale F32 (23 -> 54). GCC, Clang, and ICX lower
the restrict copy to a six-instruction `memcpy` tail call, so LCCC's larger
inline copy is not presented as a code-size win.

An eleven-run alternating `CLOCK_MONOTONIC` VM screen nevertheless gives a
large hot-loop signal: copy F32 6.66x, copy F64 3.27x, constant-scale F32 5.98x,
affine F32 2.51x, and affine I32 2.60x versus the pre-item compiler; arithmetic
and geometric new/old ratios are 0.28140 and 0.25962. These are unpinned,
non-PMU VM wall times and therefore screening evidence only, not a Raptor Lake
superiority claim. Raw source, samples, static A/B, strict/fast outputs, and
assembly are under `artifacts/simd-fp-four/affine-map/`. Final validation on
the rebased tree is 325/325 regression checks, 50/50 correctness tests, and
803 passed Rust library tests (6 ignored).

Fresh live CE comparisons resolve GCC 16.2 `cg162`, Clang 22.1.0
`cclang2210`, ICC 2021.10 `cicc2021100`, and latest ICX `cicxlatest`. LCCC's
strict scale F32 is 53 instructions (GCC 53, Clang 52, ICC 71, ICX 24), strict
affine F64 is 60 (GCC 42, Clang 60, ICC 123, ICX 27), and fast affine F64 is
59 (GCC 42, Clang 60, ICC 123, ICX 27). Whole-function counts include setup
and scalar tails and are triage data, not throughput estimates.

### Ninth distilled fix: profitable fixed-width SLP distances

The first fixed-vector SLP target is deliberately narrow: a one-block return of
an exact sum of squared differences over two contiguous arrays. The proof
recovers each load's common base and constant byte offset, requires every term
to be the square of the corresponding subtraction, and accepts only complete
measured-profitable 256-bit widths (F32x8 or F64x4). F32x4 and partial F64x3
objects remain scalar, avoiding both an out-of-bounds packed read and a known
horizontal-reduction profitability loss. Reassociation is a hard gate, so
strict `-ffp-contract=off` output remains source-ordered scalar code.

After first measuring a generic five-intrinsic implementation, the final x86
lowering uses one proof-carrying fixed-distance intrinsic. It folds the second
array into `vsubps/pd`, squares in YMM0, performs the cross-lane reduction in
XMM0, emits `vzeroupper`, and returns the scalar already in the SysV FP result
register. A scoped direct-result marker is invalidated by clobbering operations
and accepted only for that exact immediate return. This removes every transient
vector home, register relay, and stack frame rather than relying on broad SIMD
allocation policy.

The controlled 53-function A/B is **2 improved, 51 tied, 0 regressed** in fast
mode: p52 F64x4 is 14 -> 9 instructions and p53 F32x8 is 28 -> 11; aggregate
instructions are **1883 -> 1861**, arithmetic ratio 0.98832 and geometric ratio
0.97437. Strict mode is exactly **0 improved, 53 tied, 0 regressed**
(1722 -> 1722). Live CE fast-mode comparisons resolve the same four oracle
families as above: LCCC/GCC/Clang/latest ICX all emit 9 instructions for F64x4
and 11 for F32x8, while classic ICC emits 16 and 32. The implementation was
informed by the common data dependencies, not copied from one oracle.

An eleven-run alternating VM wall-clock screen measured F32x8 at ratio 0.71709
(1.395x faster) and F64x4 at ratio 1.00421 (0.4% slower/noise), for arithmetic
and geometric ratios 0.86065 and 0.84860. The individual flat/slightly negative
F64 result is retained rather than hidden by the aggregate. These unpinned,
non-PMU timings are screening evidence only; target-hardware counters are still
required. Sources, raw samples, strict/fast output, static A/B, and CE assembly
are under `artifacts/simd-fp-four/fixed-slp/`.

The structural regression locks in full-width packing, strict and kill-switch
controls, the narrow/partial scalar decisions, no YMM stack spill, and a static
F32x8 win. The runtime regression sweeps deterministic exact inputs in strict
and fast modes. Concurrent live-oracle runs also exposed and fixed a tooling
race: local assembly temp names now include the process ID, so parallel
comparisons cannot unlink one another's output. Final validation on latest main
is 328/328 regression checks, 50/50 correctness tests, and 807 passed Rust
library tests (6 ignored).

The post-rebase full run also exposed a newly merged peephole correctness defect
outside SLP: never-read stack-store elimination treated `ret` as if it did not
read 0(%rsp), deleted an inline-retpoline target rewrite, and left the runtime
inside the speculation trap. The pass now preserves an immediate
`movq %target,(%rsp); ret`; the existing 64-deep retpoline runtime regression
passes and still verifies that no naked indirect branch is emitted.

### Tenth delivery: pinned GNU gzip 1.14 end to end

The workload-derived `gzip_crc32` reproducer has been graduated to a complete
GNU gzip 1.14 build selected from `packages/gzip/PKGBUILD`. The reproducible
runner verifies the fetched archive digest, builds the full project with LCCC
and GCC at `-O3 -march=x86-64-v3`/`make -j2`, requires both upstream suites to
pass 30/30, checks bit-identical compressed streams at levels 1/6/9 and exact
decompression, and captures deterministic corpora, binaries, size, objdump,
build logs, best/worst/median samples, and arithmetic/geometric ratios.

This graduation exposes a real end-to-end deficit rather than a win: across
five seven-round paired VM cases LCCC/GCC ratios range from 1.74329 to 1.99878,
with arithmetic 1.87943 and geometric 1.87721. LCCC loses every individual
case; its text is 114,738 bytes versus GCC's 106,982. Live CE triage of the
scalar CRC recurrence is LCCC 36 instructions, GCC 15, Clang 49, ICC 35, and
latest ICX 64. GCC's compact pointer/end loop and folded table address are a
plausible contributor, while the references' widely different static forms
show why no one implementation was assumed optimal.

The script and provenance are under `tests/workloads/gzip-1.14/` and raw
results under `artifacts/simd-fp-four/gzip-e2e/`. This is CPU-pinned VM
wall-clock screening, not PMU-backed Raptor Lake evidence. The repeatedly
fetched archive digest still differs from the package recipe checksum and no
signature was verified here; both limitations remain explicit.

## Correctness policy discovered during the audit

Strict IEEE source order is now the default for FP reductions.  Reassociation
and reduction vectorization require an explicit fast-math/reassociation option;
the umbrella `-ffast-math` option defines `__FAST_MATH__`, while individual
reassociation options follow GCC and do not.  Pointer-root aliasing is conservative:
unrestricted parameter roots may overlap and block vectorization; C `restrict`
propagates to IR `noalias` and can prove the transformation legal.  Dedicated
regressions cover strict ordering and overlapping map/matmul inputs.

## Next measured targets (deferred to a new session)

The affine store-loop, first profitable fixed-width SLP, and first full gzip
workload deliveries are complete and isolated. Do not widen either SIMD
legality proof opportunistically. Follow-up work should first root-cause the
measured gzip gap (beginning with redundant CRC/table address formation) as a
separate optimization, then graduate another expat, SQLite, Linux, glibc, or
zlib-ng workload with the same provenance and output discipline.

For every item, retain strict-vs-fast differential execution, local/remote
assembly, individual gains and regressions, and raw randomized paired timing
samples.  VM timing remains screening evidence; PMU-backed bare-metal results
are required before asserting superiority on Raptor Lake.
