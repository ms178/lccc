# ordered arithmetic: workload-driven ordered SIMD and exact signed division

## Scope

This is a focused performance batch, not another list of independently counted
LLVM families. It implements one new exact-division analysis, repairs an ISA
permission defect that disabled an existing ordered SIMD transform, and closes
an observable-load legality hole in that transform. It does not claim to solve
all outstanding missed optimizations or bit demand's dispatch acceptance gate.
No new upstream report is marked globally solved by these reduced witnesses.

The branch includes main through `caac07c` (the TLS codegen hardening merge).
The pre-batch compiler used for comparisons is the saved `539b530` executable.

## Production changes and boundaries

### One resolved ISA authority

`run_passes` now uses `X86Isa.ymm` and `X86Isa.sse41` for the ordered reciprocal
prefix, rather than the driver's raw `enable_avx2` / `enable_sse4_1` requests.
Those requests default to false even though the documented default codegen
ceiling is x86-64-v3. Two obsolete Boolean pipeline parameters and their caller
arguments are removed. Explicit target ceilings and feature denials remain
owned by the resolved ISA object; the change does not grant a new ISA.

The existing transform packs four independent integer-denominator calculations,
converts to double, divides and multiplies four lanes, then adds products to the
scalar accumulator in original order. It does not reassociate the sum, contract
a multiply/add, approximate division, or read beyond the four original elements.
It retains the original scalar tail. Observable FP-environment policy still
blocks this reordering, and the existing environment gates are retained.

The structural regression used to call an unqualified build a non-AVX baseline.
That contradicted the documented V3 default. It now separately tests default
V3, explicit V3, explicit SSE2, AVX-only with a V2 ceiling, AVX2 denial, and
SSE4.1 denial. Disabling final VEX promotion and XMM allocation remains covered.

### Exact signed division and remainder

Before ordinary constant strength reduction, `div_by_const_exact` proves low
zero bits for I32/I64 dividends. For a positive representable `2^k`, exact
multiples can use `AShr(x,k)` without a negative rounding bias; their signed
remainder is zero, including negative multiples and INT_MIN. Negative divisors,
unsupported widths and insufficient facts retain the old lowering.

The bounded analysis propagates low-zero facts through constants, equal-width
copy/cast views, AND, OR, XOR, add/subtract, multiplication and constant valid
shifts. Opposite-parity factors have an even modular product: this includes
`x*(x+odd)`, `x*(x-odd)`, `x*(x^odd)` and `x*~x`. This is a bit-vector fact,
**not** an assumption that signed multiplication is nonnegative. It holds under
`-fwrapv` and needs no signed-overflow speculation.

Inlining leaves duplicate `i+j` definitions before GVN. A separate bounded
structural comparison recognizes equal pure, equal-width modular coefficient
DAGs, including commuted operands. It never equates separate loads or calls.
The fact walk has 64 visits, a copy/view walk has 16 steps, and each structural
comparison has 32 visits. Exhaustion only loses an optimization. Duplicate SSA
definitions refuse the sweep before mutation. Completed plans replace only the
consumer, preserve its destination and source span, and allocate no new IDs.
The exact `exact_signed_div` ablation token is reset per translation unit.

On spectral norm this removes two sign/bias shifts and an add from each packed
denominator DAG. Standalone adjacent-product quotient controls were already
optimized later by existing passes; they are not claimed as new scalar wins.
Complement-product quotients, masked exact quotients and exact remainders do
have shorter native sequences. Widths and operand spellings are witnesses,
not independent optimization families.

### Retain every observable load

The ordered reciprocal matcher previously counted only F64 loads while allowing
other loads in the scalar body. An unrelated volatile integer read could then
be omitted for every four-element prefix iteration. The matcher now requires
exactly one load of any type, with the source load's existing nonvolatile,
default-address-space F64 checks unchanged. A volatile-integer-read regression
must retain its read and must not acquire the packed reciprocal prefix.

## Measured workload result

`ordered-arithmetic-runtime.json` retains all raw rounds, CPU/wall statistics, compiler
versions, commands, source/executable hashes, pinning and environment metadata.
It is a 46-workload **local Intel Xeon VM** experiment, not a rerun on the
user's EPYC 7763. Default `-O2` was passed uniformly; sources' registered semantic
flags were retained. There were 12 randomized paired rounds, two warmups, CPU 0
pinning, no discarded outliers and no usable PMU claim.

| Spectral norm | bit demand | ordered arithmetic | GCC 16.2 |
|---|---:|---:|---:|
| Median wall time | 199.01 ms | 118.81 ms | 157.99 ms |
| Median child CPU | 198.45 ms | 118.48 ms | recorded in raw report |

The paired ordered-arithmetic/bit-demand median is **0.60354**, 95% bootstrap interval
**[0.59322, 0.61051]**: approximately **39.6% less runtime**. All 46 workloads
passed correctness. **The other 45 LCCC executables are byte-identical to batch
3**; their small timing shifts are controls/noise, not optimization wins.
The merged-main rebuild reproduced all 46 measured LCCC executable hashes, and
a separate post-merge correctness sweep passed with GCC 16.2 and Clang 23.1.3.

Default LCCC permits V3 whereas default GCC does not. The default comparison
must therefore not be presented as matched-ISA superiority. A second experiment
(`ordered-arithmetic-matched-isa.json`) gives every compiler `-march=x86-64-v3`, with
20 randomized paired rounds and three warmups:

| Spectral norm, matched V3 | Median wall |
|---|---:|
| bit demand | 141.43 ms |
| ordered arithmetic | 121.40 ms |
| GCC 16.2 | 101.85 ms |
| Clang 23.1.3 | 200.69 ms |

ordered-arithmetic/bit-demand is **0.861**, interval **[0.844, 0.870]**. ordered-arithmetic/GCC remains
**1.192**, interval **[1.162, 1.219]**. This is an improvement, not optimality.
No ICX runtime comparison or EPYC replication is claimed here.

## Executed checks

- Post-merge fastbuild builds all compiler/linker binaries with warnings denied.
- Post-merge all-target tests: **5,455 library tests + 1 linker test passed**,
  10 library tests intentionally ignored; strict all-target Clippy passed.
- Exact signed division: **320,160 comparisons per arm**, GCC 16.2, Clang
  23.1.3 and nine LCCC configurations; SSA/IR checks, disabled-mechanism and
  no-register-allocation controls. Scalar and packed bias reductions are checked.
- Ordered packed reciprocal bit oracle: **1,260 cases per arm**, including
  default V3, explicit V3, exact references and scalar/allocator ablations.
- Existing packed-DAG gate: **9 arms x 9,108** output/exception/guard-page
  comparisons, including pinned GCC 16.2's i686 runtime reference.
- Boundary differential fuzzer: **4,533 cases, 160 LCCC builds**, O2/O3,
  SSE2/V3, wrapping/nonwrapping, zero mismatches or generator defects.
- All bit demand native/IR/exact-policy contracts passed again.
- FP environment gates, environment hygiene and CI/local gate parity passed.
- **210 scoped Z3 bit-vector lemmas passed**: every supported power-of-two
  divisor for both widths, even-product identities and representative transfer
  lemmas. These are algebra checks, not a certificate of the Rust pass.

A diagnostic-stream audit also found a pre-existing Clang format warning in
`double_reduction.c`: LP64 `int64_t` is `long`, not `long long`. An explicit
`(long long)` printf argument fixes that mismatch without changing the workload.
All four generated executables (bit-demand, ordered-arithmetic, GCC, Clang) remained byte-identical
after the cast, and their diagnostic streams were empty. The raw timed report
retains the original source hash; this source-only warning fix was checked
separately and does not alter those measured executables.

## Reproduction

```sh
scripts/build_lccc_fast.sh
python3 scripts/check_exact_signed_div.py --gcc gcc-16 --clang clang
CCC=target/fastbuild/lccc bash tests/regression/check_strict_computed_recip_codegen.sh
CCC=target/fastbuild/lccc GCC=gcc-16 bash tests/regression/check_strict_packed_dag.sh
python3 scripts/check_strict_packed_recip.py --gcc gcc-16 --clang clang
python3 scripts/prove_exact_signed_div.py --json exact-proofs.json
python3 tests/fuzz/boundary_loop_fuzz.py --lccc target/fastbuild/lccc \
  --gcc gcc-16 --seeds 0:20 --levels O2,O3 --march x86-64,x86-64-v3
python3 tests/benchmark/run_benchmarks.py --compilers lccc,gcc \
  --gcc gcc-16 --candidate bit-demand=/path/to/saved/bit-demand \
  --reps 12 --warmup 2 --strict --no-pmu-probe --json workload-runtime.json
```

## Remaining acceptance gap

The focused spectral change has a demonstrated runtime win. The wider hard
performance requirement remains **OPEN**: byte-load reassembly, find-bit, SHA,
switch dispatch and other GCC gaps are not fixed by this batch. The original
bit demand across-layout dispatch issue is not resolved. Do not reinterpret the
45 identical executable controls as 45 improvements, or the 210 solver checks
as 210 implementations. PR #885 remains draft pending that broader work.
