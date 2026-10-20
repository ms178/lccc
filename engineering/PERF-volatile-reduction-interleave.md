# Scalar interleaving for volatile unsigned reductions

## Opportunity and scope

Load-to-ALU fusion removes a register relay, but leaves one serial accumulator
and a branch per volatile read. These reads cannot be CSE'd, widened or packed
into vector loads. Instead, the new `volatile_reduce` IR pass creates four
independent scalar chains and one backedge per four reads.

The pass is x86-64-only, at -O3, outside size optimization. It respects the
ordinary `CCC_DISABLE_PASSES=volatile_reduce` and `CCC_DISABLE_PASSES=unroll`
switches and adds no per-candidate environment access. No new ISA is required.
It runs after vectorization and before loop rotation, while the canonical phis
and top-of-loop condition are still explicit.

The intentionally narrow matcher requires:

- A single-entry two-block loop: exactly two header phis and an unsigned `<`
  guard, with a dedicated unconditional preheader.
- A U32/U64 unit-step counter starting at zero, and an invariant bound.
- One U32/U64 volatile read from an invariant pointer, optionally behind one
  constant-offset GEP; no accumulator-dependent addresses.
- Exactly one matching-width unsigned add, multiply, and, or, or xor reduction
  and the IV increment, with no other body work or observable intermediate
  accumulator. Body definitions cannot escape except through the header phis.

Everything else remains unchanged: signed/FP reassociation, non-unit IVs,
noncanonical guards, wider/narrower reads, stores, calls, atomics, dynamic
addresses and side exits. This is not a new vectorizer or a general unroller.

## Correctness argument

For unsigned bound n and group size 4, m = n & ~3 is the greatest multiple of
four not exceeding n. The main loop executes exactly m source iterations in
order; the original loop starts with IV m and processes n-m (0..3) reads.
Inside the main loop, iv <= m-4, so iv+4 <= m <= n: even UINT_MAX cannot wrap
the IV. The scalar tail retains its original exit condition and SSA live-outs.

Each cloned read retains its width, address expression, segment and volatile
flag. An optional GEP is cloned in each lane instead of being speculated on
zero trips. There is no read before a successful guard, and no read beyond the
original count. Arithmetic regrouping changes only unobservable unsigned
modular operations. Four partial values combine with the same associative,
commutative operation; the seed appears exactly once, and other lanes start
at the operation's identity (0, 1, or all bits set). The combine tree is balanced.

No existing body instruction is removed. The original loop is the tail, and
only its preheader phi edges change. Fresh SSA/label ranges are checked before
mutation; stale cached next IDs are cross-checked against actual IR IDs. New
blocks carry no fabricated source locations, and the extended preheader drops
stale instruction-span indexing. Main loops have five phis; tails have a
nonconstant IV initializer. Neither can be interleaved again.

## Evidence

On the Intel Xeon 2.60 GHz virtual host, nine rotating-order samples per compiler
and kernel, after two warm-up rounds, were pinned to CPU 0. Every binary used
-O3 -fno-pic -no-pie. Each kernel ran one billion iterations. Subprocess output
and successful completion were checked for every run.

| Workload | Current-main LCCC | New LCCC | GCC 16.2 | Clang 23.1.3 | LCCC speedup |
| --- | ---: | ---: | ---: | ---: | ---: |
| Existing `tls_hot_loop` sum | 0.314249 s | 0.160568 s | 0.299427 s | 0.305281 s | 1.96x |
| New sum + product workload | 1.224057 s | 0.460838 s | 1.226280 s | 1.240236 s | 2.66x |

These are median wall times, not hardware cycle-counter measurements or claims
about arbitrary C programs. CPU-time medians show the same effect. In the
eligible main loops, four memory ALU operations plus IV increment, compare and
branch replace four copies of a four-instruction rolled iteration: 7 versus 16
architectural instructions for four reads (setup and tail excluded).

Tradeoff: the benchmark's sum function grows from 57 to 155 bytes; product grows
from 58 to 220 bytes, including setup, four chains, combine and cleanup. This
justifies the -O3/non-size gate rather than applying the transform universally.
Four chains hide the multiply dependency without the additional register
pressure of eight chains, and a power-of-two group has a cheap overflow-safe
bound mask. Short calls may not amortize setup. No speed guarantee is made for
other CPUs, memory types or workloads. ICX was not locally available.

Reproduce using cached/exact compiler paths:

```sh
python3 scripts/measure_volatile_reductions.py \
  --baseline /path/to/main/lccc --candidate target/fastbuild/lccc \
  --gcc /path/to/gcc-16 --clang /path/to/clang-23 \
  --trips 1000000000 --samples 9 --output work/perf/measurement.json
```

Timing is diagnostic, never a flaky CI pass/fail threshold.

## Regression design

- An independent SSA interpreter supplies a changing read stream and compares
  results, final IVs and the complete access trace. 3,040 combinations cover five
  operations, both accumulator/counter widths, four seeds and 38 trip counts.
- Another 90 scenarios cover constant address offsets and generic/FS/GS spaces.
  Each rewritten scenario checks SSA validity and idempotence.
- Explicit unsigned-extrema arithmetic checks prove grouping/tail bounds without
  trying to execute UINT64_MAX iterations. Refusal tests assert no IR mutation,
  including allocator-ID exhaustion and non-associative/unsupported semantics.
- A C contract tests 500 input/seed/count combinations through eleven noinline
  pointer/TLS functions (5,500 results per execution), null-pointer zero trips,
  zero/small/large tails, and unsigned wraparound. Six optimization/code-model
  configurations, two disabled controls and an assembly mechanism assertion
  prevent an optimization that silently stops firing from passing unnoticed.
- A dlopen test exercises 33 trip counts through the dynamic TLS descriptor
  model at -O3. Existing TLS model contracts also remain in force.
- The new regression is mirrored in hosted and local CI. The previously missing
  local mirror of the compiler-independent TLS oracle test is repaired too.

Local warning-denied fastbuild and strict all-target Clippy passed. The full
library suite passed 5,361 tests with zero failures and ten existing ignores.
The new C regression/benchmark fixtures compile with -Wall -Wextra -Werror under
GCC 16.2 and Clang 23.1.3. Formatting, CI/local gate parity, benchmark-harness
self-tests (nine), directive guard and complete TLS contracts passed. Remote CI
results are reported on the PR rather than predicted here.
