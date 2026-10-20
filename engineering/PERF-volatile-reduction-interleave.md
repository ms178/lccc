# Scalar interleaving for volatile unsigned reductions

## Opportunity and deliberately narrow scope

Load-to-ALU fusion removes a register relay but leaves one serial accumulator
and a branch per volatile read. The `volatile_reduce` pass creates four scalar
chains and one backedge per four reads. It never vectorizes, widens, coalesces or
speculates the volatile accesses.

The pass is x86-64-only, at O3, outside size optimization, after vectorization
and before loop rotation. It respects both `CCC_DISABLE_PASSES=volatile_reduce`
and `CCC_DISABLE_PASSES=unroll`. No new ISA or per-candidate environment read is
required. i686, AArch64 and RISC-V are excluded and have explicit nonapplication
checks.

Eligibility requires a single-entry two-block loop with a dedicated
unconditional preheader, two header phis, an unsigned U32/U64 unit IV starting
at zero, an invariant unsigned limit, and one invariant-address volatile read.
The body may contain one constant-offset GEP, one matching-width unsigned
add/mul/and/or/xor reduction, and the unit increment; nothing else. Body results
must not escape except through the original header phis. The seed is invariant
and need not be zero. Signed/FP, atomics, stores, additional reads, side exits,
variable addresses and non-unit IVs are not part of this feature.

## Correctness and setup policy

For unsigned `n`, `m = n & ~3` is the greatest multiple of four not exceeding
`n`. The mask is computed in the IV's original width. When `m == 0`, execution
branches directly to the original scalar loop with its original initial values;
there is no four-way initialization or combine. Otherwise the grouped loop
processes exactly `m` reads and the original loop starts at IV `m`, performing
zero to three remaining reads. Since `iv <= m-4` before each successful group,
`iv+4 <= m <= n`: even UINT_MAX cannot wrap the main IV.

Every cloned load retains the original address, segment, width and volatile
flag. Optional GEPs are cloned inside the guarded body. The seed appears in one
lane once, with neutral elements in the other lanes. Only unobservable modular
unsigned arithmetic is regrouped. A balanced tree combines the partial values.
The original accumulator and IV names remain the live-out interface.

The original phi preheader inputs remain valid for the short path; a third
incoming edge carries grouped results. Main loops have five phis and tails have
three predecessors, so neither is interleaved recursively. Fresh values (20 or
24 with a GEP) and three block labels are checked before mutation. Source spans
for untouched and cloned instructions are retained. A compile-time assertion
makes the fixed-four assumption explicit. A named 32-rewrite per-function
budget bounds repeated analysis rather than claiming unlimited loop expansion.

## Profitability decisions

Long U64 pointer/Local-Exec reductions gain from branch amortization and shorter
accumulator dependencies. Four lanes are compared against a two-chain source
alternative rather than selected by intuition alone. Code-size growth is real;
O3/non-size gating remains essential.

The host matrix exposed an important exception: known Global-Dynamic or
Local-Dynamic TLS accesses repeat a descriptor call per folded load in the
current backend. Grouping did not remove that bottleneck and could regress
runtime. Those addresses now retain the original loop. Declaration attributes
override command-line TLS policy, which overrides output defaults; Local-Exec,
Initial-Exec and ordinary pointers remain eligible. Bounded constant-GEP/Copy
tracking recognizes the same address forms as the backend's fold.

This is a cost exclusion, not a correctness requirement or a claim that dynamic
TLS can never be optimized. A backend resolver-hoisting change would need its
own placement, ABI and profitability review. It is not silently bundled here.
The short-trip dispatch similarly avoids unnecessary four-way setup without
inventing an unsupported universal trip-count threshold.

## Reproduction and attribution

Both diagnostic scripts require exact paths for reference compilers and reject
wrong versions: GCC 16.2, Clang 23.1.3, optionally ICX 2026.2. They validate child
success/output, reject warnings, pin a CPU, warm up twice and rotate run order.
Reports contain compiler and generated-binary hashes, source hashes, exact
commands, host/CPU/environment, raw samples and dispersion. An incomplete matrix
is marked explicitly and JSON checkpoints are atomic. Keep raw measurements in
ignored `work/` or attached artifacts, not the source tree.

```sh
python3 scripts/measure_volatile_matrix.py \
  --candidate target/fastbuild/lccc \
  --gcc /path/to/gcc-16 --clang /path/to/clang-23 \
  --samples 5 --budget 5000000 --output work/perf/matrix.json

python3 scripts/measure_volatile_reductions.py \
  --baseline /path/to/baseline/lccc --candidate target/fastbuild/lccc \
  --baseline-revision BASE --candidate-revision CANDIDATE \
  --gcc /path/to/gcc-16 --clang /path/to/clang-23 \
  --trips 1000000000 --samples 9 --output work/perf/long.json
```

The matrix uses opaque shared-object calls to prevent cross-compiler inlining
and internal CPU/wall timers to exclude process/loader startup. Every call has
an independent modular oracle. It covers five operations, both widths, pointer
and IE TLS, GD TLS controls and a register-pressure context, over counts 0–16,
32, 64, 256, 1024 and 65536. Timing is diagnostic, never a flaky CI threshold.
Steady repeated calls are not evidence about cold TLS allocation or arbitrary
mixed application traffic. No universal speed or optimality claim is made.

## Executable assurance

The pass has sixteen focused Rust tests, including 3,040 changing-stream cases,
90 offset/segment cases and 160 high-bit terminal executions through the actual
production-generated bound. Additional tests cover source spans, live-out IVs,
nested/sibling loops, stale caches, exact allocator exhaustion and TLS policy
precedence. Four production-source mutants were caught and restored.

The native C contract checks 6,000 results per execution plus opaque null/zero
probes. Value parity, assembly and pass-output IR establish application and
nonapplication independently across optimization policies and TLS models. Valid
C negative fixtures protect atomics, signed/FP, strided IVs, changing addresses,
intermediate stores and multiple reads. Dlopen checks the retained dynamic TLS
path. The benchmark driver is smoke-tested in ordinary CI, and the combined
sum/product workload is registered in the central benchmark harness.

Checker self-tests run with and without Python optimization. The authenticated
Intel job adds O3 comparisons and a 480-execution ICX matrix. Local host ICX is
not available under the sandbox's allowed-network policy; authenticated remote
results are separate evidence, not fabricated local timings.

See [the adversarial audit](AUDIT-volatile-reduction.md) for per-function proof
obligations, additional pre-existing peephole defects exposed and repaired, and
explicit tool/feature limitations.

## Corrected-driver host results

Final full matrix: Intel Xeon 2.60 GHz virtual host, CPU 0, five rotating-order
samples after two warmups, five million target reads per sample, all 24 cases
and 22 trip counts completed. The following are median **CPU nanoseconds per
call at n=65536**, not end-to-end application times. “Rolled” is this same
compiler with only this pass disabled, isolating the feature from unrelated
backend fixes. Raw JSON also retains the previous candidate and every sample.

| Case | Rolled | Four chains | Two-chain source | GCC 16.2 | Clang 23.1.3 |
| --- | ---: | ---: | ---: | ---: | ---: |
| U32 pointer add | 23707.89 | 11845.00 | 12282.49 | 23612.29 | 23252.30 |
| U32 pointer multiply | 69883.43 | 23663.88 | 35243.18 | 69818.64 | 69601.87 |
| U64 pointer add | 23489.24 | 11882.49 | 12252.03 | 23321.59 | 23696.12 |
| U64 pointer multiply | 70598.34 | 24966.66 | 35774.01 | 70352.82 | 70331.67 |
| U64 IE TLS add | 23480.01 | 24210.91 | 17698.78 | 23387.70 | 23414.29 |
| U64 IE TLS multiply | 69713.68 | 23716.28 | 35153.05 | 70206.21 | 69892.25 |
| U64 pressure multiply | 70354.07 | 23851.63 | 35235.76 | 70080.89 | 70586.41 |
| U64 GD TLS multiply (excluded) | 93910.68 | 93909.33 | 88487.68 | 134449.91 | 69897.76 |

Across eligible pointer/IE/pressure cases, equally weighted geometric means of
per-count speedup over the rolled control are **0.957x for n=0–3, 1.194x for
n=4–16, and 1.748x for n=1024/65536**. Those are diagnostic aggregates, not an
assumed application trip-count distribution. The short guard improves n=0–3 by
12.8% geometrically over the previous unguarded candidate, but costs 12.9% at
n=4–16 and 5.1% over the long-count group. In particular, U64 IE TLS simple
operations lost roughly one quarter versus that candidate; this is not hidden
by the pointer/multiply wins. The U64 IE add case is also 3.1% slower than the
rolled control in this run and the two-chain source is clearly faster.

Four chains remain a deliberately simple O3 policy: pointer and multiply-heavy
contexts favor them, including pressure; a source-level two-chain alternative
is not an isolated lane-count experiment with identical CFG/register allocation.
One virtual host does not justify a new width/TLS/operator-specific dispatch or
an invented application trip threshold. The natural n<4 bypass avoids executing
a combine with no groups, not all setup/prologue cost. Users dominated by tiny
calls or unfavorable IE TLS contexts can retain the rolled policy with the
existing disable switch. This is a documented compromise, not “optimal on every
workload.” Known GD/LD is excluded because both the direct resolver inspection
and paired measurements support doing so.
