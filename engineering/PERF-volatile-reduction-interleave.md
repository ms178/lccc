# Scalar interleaving for volatile unsigned reductions

## Opportunity and deliberately narrow scope

Load-to-ALU fusion removes a register relay but leaves one serial accumulator
and a branch per volatile read. The `volatile_reduce` pass creates four scalar
chains and one backedge per four reads. It never vectorizes, widens, coalesces or
speculates the volatile accesses.

The pass is x86-64-only, at O3, outside size optimization, after vectorization
and before loop rotation. It respects both `CCC_DISABLE_PASSES=volatile_reduce`
and `CCC_DISABLE_PASSES=unroll` (plus TU-scoped compatibility controls
`CCC_VOLATILE_REDUCE_LANES` and `CCC_VOLATILE_REDUCE_BUDGET`). No new ISA or per-candidate environment read is
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
`n`, computed in the IV's original width. The grouped loop processes exactly
`m` reads and the original scalar loop resumes at IV `m`, performing zero to
three remaining reads. Since `iv <= m-4` before each successful group,
`iv+4 <= m <= n`: even UINT_MAX cannot wrap the main IV. Zero trips perform no
volatile reads, though accumulator initialization/combine overhead remains.

Every cloned load retains the original address, segment, width and volatile
flag. Optional GEPs are cloned inside the guarded body. The seed appears in one
lane once, with neutral elements in the other lanes. Only unobservable modular
unsigned arithmetic is regrouped. A balanced tree combines the partial values.
The original accumulator and IV names remain the live-out interface.

The original phi preheader inputs are replaced with grouped results from the
combine edge. Main loops have five phis and remainder IVs no longer begin at a
constant zero, preventing recursive interleaving. Fresh values (19 or 23 with a
GEP) and three block labels are checked before mutation. Source spans for
untouched and cloned instructions are retained. A compile-time assertion makes
the fixed-four assumption explicit. The upstream 200-block and 32-rewrite
budgets and lane-zero compatibility control remain intact.

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
No short-trip bypass is added: earlier measurements showed a tiny-call benefit
but medium-trip and Initial-Exec TLS regressions. Retaining the upstream CFG is
the simpler evidence-supported choice, not an invented trip-count threshold.

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

## Attribution of this follow-up

The merged main already has four-chain interleaving. Its original approximately
2x TLS-sum and 2.6x combined sum/product gains must **not** be attributed to this
follow-up. This change preserves those loops while fixing attribution, checked
allocation, TLS cost selection and late-backend correctness. The proposed
short-trip dispatch was rejected after its setup/register-placement trade-offs
were measured. Source-level two-chain comparisons remain diagnostic, not an
isolated lane-count experiment with identical CFG and allocation.

Fresh results against an exact current-main binary are recorded in the PR and
attached evidence. The full 24-case matrix records short, medium and long trips,
including cases with no improvement; no universal speedup is claimed.
