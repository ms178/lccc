# Batch 5: cumulative PR #886 and recovered performance follow-ups

## Find-bit and SHA follow-ups

**Current recovered-source results:** see [followups.md](followups.md) and
[followup-screening.json](followup-screening.json). The fresh 15-round screen
confirms SHA and scaled find-bit gains; all 46 workload outputs match.
The earlier recovery warnings below describe provenance, not a current build
failure. Hosted status is tracked on PR #886.

The cumulative work includes the original PR #886 through 8547b16 plus the
following follow-ups. After the sandbox failure, its restored snapshot only
contained the original PR. The follow-up source and tests were reconstructed
from the recorded tool calls, formatted, and then validated afresh before
being committed to the PR branch. The original PR's 14 green hosted checks
must not be attributed to the follow-ups; current hosted status is tracked on
the PR itself.

* `scalar_compare_iv`: narrowly enable a U64 comparison-bound recurrence in
  small, read-only search loops. General scalar IVSR stays opt-in. Affine Shl
  recognition and unsigned runtime initialization cover `(i + 1) << 6`.
* `pair_load_carry`: forward a single eight-byte SIMD store to its identical
  next-iteration pair load in definitely-entered, two-block counted loops.
  Admission requires a matching eight-byte pointer step, one writer, lane-local
  zero-upper-half provenance and no escaping vector values. I32x4 OR producer
  and consumer register-class support keeps the carried vector register-homed.
* Add unit tests and executable regressions for unsigned wraparound, zero/tail
  loops, protected-page boundaries, alias/barrier/escape refusals and source-span
  preservation. The schedule regression uses local arrays to exercise packing.
* Switch target-alignment experiments were neutral or slower; no alignment
  change is included. The prior indirect-dispatch concern is not claimed fixed.

Before the environment failure, the recorded 15-round VM screens (three
warmups) gave current/previous-PR-head ratios of 0.913 [0.885, 0.932] for
scaled find-bit and 0.896 [0.893, 0.899] for SHA. These are historical screening
results, **not rerun measurements of this recovered export**, not EPYC evidence
and not matched-ISA GCC comparisons. The follow-up Rust library suite recorded
5,472 passes and strict Clippy passed before the final canonical constant-carrier
adjustment and strengthened schedule regression. Those final adjustments built
successfully and their emitted assembly was inspected, but comprehensive final
checks were interrupted. The ignored workspace's raw follow-up runtime samples
were not recovered; the original PR's retained evidence below remains available.

Fresh recovery checks: warning-denied fastbuild of lccc/lccc-ld, 5,472 library
tests plus one integration test, strict all-target Clippy and formatting PASS.
Both new C regressions, including the strengthened local-array schedule case,
PASS at -O0/-O1/-O2/-O3/-Os with inter-pass IR verification, and GCC16
-Wall -Wextra -Werror. The full segment runtime contract PASS was repeated.
A separate 3 GiB swap file was activated alongside the existing 1 GiB file;
no package extraction into system paths was repeated. Current benchmark and
CI results are recorded separately from the historical measurements below.

The patch is a complete cumulative Git diff against base
`b5a6b6b9c7568a002198249cf81ec9d70924683e`, not just an incremental update to an
already-applied PR. Apply it to that base using `git apply ms178-1.patch`.

## Original pushed PR evidence (historical)


This is an initial, narrow improvement, **not completion of the GCC-gap mission**.
Find-bit, SHA and switch dispatch still need profitable production changes.
The earlier indirect-dispatch concern is not claimed fixed.

## Changes and safety review

* On x86-64, `address_form` turns memory-consumed pointer-base plus full-width
  integer Add into the equivalent byte GEP, preserving the result's carrier
  type. It never widens a load, changes volatility or removes an integer cast.
  Copy traversal is bounded, narrow offsets are rejected, duplicate definitions
  decline the function. This exposes the existing shared allocator/codegen SIB
  folding rather than reviving the unsafe late address-folding path.
* `reduction_unroll` adds a load-only exception to the existing exact-trip
  two-block unroller: exactly one unsigned U32/U64 sum and the counted IV.
  Existing trip, divisibility, rotation, clone-safety and size guards remain.
  Prefer four iterations, fall back to two only when exactly divisible; never
  create a remainder, speculative access, extra iteration or wider read.
* Cloned accumulator additions are rearranged into a contribution tree and ONE
  carried addition. Only unsigned wrapping addition is eligible. Intermediate
  sums must be private, including bare pointer uses. A failed proof leaves the
  function and allocation counters unchanged. Original final-result IDs remain.
* Red-team focus: signed overflow, dependent contributions, externally used
  partial sums, duplicate definitions, non-divisible trips, volatile reads,
  protected-page end access, and unsupported factors. Rejected signed/FP forms
  must not be broadened merely to increase coverage. This is code review plus
  targeted executable tests, not an independent audit or formal certificate.

Both production additions have `CCC_DISABLE_PASSES` ablation names as above.
Reduction policy is snapshotted per pipeline invocation, defaults off outside
x86-64, and uses the existing unroll/size routing.

## Measurements

Intel Xeon 2.60 GHz VM, taskset CPU 0, no PMU. Not EPYC evidence. GCC cache is
GCC 16.2; Clang 23.1.3 and ICX 2026.2 are not installed locally. Standard
harness commands use LCCC's default target and GCC's default ISA, so those
GCC ratios **are not matched-ISA comparisons**. No aggregate speedup claim.

All 46 workload outputs match GCC and batch 4 in the three-round broad screen.
Only byte_load_reassembly, expat_siphash24, lz4_compress and lz4_match_extend
executables differ from batch 4. Other 42 executables, including SHA, find-bit,
switch and indirect dispatch, are byte-identical. Repeated 12-round screening
rejects the initial apparent SipHash gain as noise:

| Workload | Current / batch 4 paired median [bootstrap 95% CI] |
|---|---|
| byte_load_reassembly | 0.956 [0.941, 0.967] |
| expat_siphash24 | 1.010 [0.940, 1.141] |
| lz4_compress | 1.012 [0.958, 1.021] |
| lz4_match_extend | 0.986 [0.977, 1.005] |
| linux_find_bit_scaled | 1.001 [0.992, 1.007] |
| sha256_transform | 0.997 [0.993, 1.002] |
| switch_dispatch | 1.001 [0.993, 1.002] |

The normal byte workload is under 20 ms: process overhead is significant.
To isolate its loop, change ONLY `#define ROUNDS 20000` to `2000000` in a
workspace copy of `tests/benchmark/programs/byte_load_reassembly.c`. Compile
with each compiler at -O2; GCC uses `-march=x86-64-v3` in this long experiment.
Keep the immutable pre-change compiler for baseline. Execute six configurations
in shuffled order (Python Random seed 5), 12 rounds, discard the first two
warmups, keep all remaining samples. All six outputs match. `byte-long.json`
contains every retained wall-time sample and output; same-round paired ratios:

| Configuration | Median seconds | Ratio to batch 4 [95% bootstrap CI] |
|---|---:|---:|
| batch 4 | 0.39468 | 1.000 |
| both additions | 0.37006 | 0.935 [0.931, 0.945] |
| address only | 0.39859 | 1.010 [1.004, 1.016] |
| reduction only | 0.44516 | 1.128 [1.120, 1.137] |
| both disabled | 0.39532 | 1.001 [0.992, 1.015] |
| GCC 16, x86-64-v3 | 0.14624 | 0.371 [0.369, 0.372] |

The two additions must be judged together, not described as two independent
wins. The longer byte loop improves about 6.5%, but its GCC gap is still large.
The long variant changes the optimization workload too; do not substitute its
GCC ratio for the standard workload ratio. Screening JSON retains source and
binary hashes, compile commands, correctness and all timed samples. Discarded
warmup samples and full logs remain in the local workspace, not this summary.

Standard reproduction (pre-change compiler path supplied by the operator):

```sh
python3 tests/benchmark/run_benchmarks.py --compilers lccc,gcc \
  --gcc /path/to/gcc-16 --candidate batch4=/path/to/pre-change-lccc \
  --only byte_load_reassembly,expat_siphash24,lz4_compress,lz4_match_extend,linux_find_bit_scaled,sha256_transform,switch_dispatch \
  --reps 12 --warmup 2 --no-pmu-probe --strict --json focused-runtime.json
```

## Checks and outstanding gates

* Warning-denied fastbuild of lccc/lccc-ld with active 1 GiB swap: PASS.
* Warning-denied all-target Clippy, formatting and diff whitespace: PASS.
* Rust suite: 5,464 library tests plus one integration test PASS; ignored tests
  and doctests remain ignored, not counted as passes.
* Added unit cases cover wrapping U32/U64 evaluation (16,384 input witnesses),
  exact factors 2/4, private carried sums, refusal/transactionality boundaries,
  full-width address operand order and result-carrier preservation.
* New regression `unsigned_load_reduction.c`: protected-page boundary,
  unaligned byte reassembly, exact/odd/zero/small trips and unsigned overflow.
  LCCC -O0/-O1/-O2/-O3/-Os with inter-pass IR verification PASS; GCC16
  -O2 -Wall -Wextra -Werror PASS. Included automatically by the regression suite.
* Full regression attempt: PASS=982, FAIL=37, SKIP=10, zero A/B differences.
  All 37 failures are missing local i386 libc headers/libgcc build dependencies;
  this is **not a green full-regression result**. Kernel boot gate skipped
  because no prepared kernel tree. Hosted CI must cover provisioned platforms.
* Broader GCC performance gate remains OPEN. No profitable find-bit, SHA or
  switch change has been established. Hosted checks must be reviewed separately.

## Hosted failure found and corrected

The first hosted run failed `check_seg_gs_runtime.sh`: `argument_imag` in
`seg_gs_complex_rvalue.c` failed at -O2/-O3. Local ablation reproduced the
failure with address normalization enabled and passed with it disabled.
Normalization enabled store/load forwarding of a complex temporary; this made
the primary result of a pure call dead. DCE retained GetReturnF64Second but
removed the producing call, leaving the secondary ABI register stale.

DCE now pins the nearest preceding direct/indirect call in the same block when
seeding an ordered GetReturnF32/F64/F128Second root. The normal worklist then
retains its operands, including indirect targets. This fixes the dependency
rather than hiding the new optimization from segment/complex functions. Unit
coverage includes all three return widths and direct/indirect producers, while
the existing executable segment contract covers the observed failure. The
full segment runtime contract now passes locally. Warning-denied tests/build,
all-target Clippy and formatting were repeated. A fresh 46-workload run passes
and every binary matches the pre-fix candidate, so the retained timing evidence
still describes the final generated benchmark code. Hosted checks are rerun
for this correction; do not treat the earlier failing run as green.
