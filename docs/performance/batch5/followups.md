# Find-bit and SHA: recovered-source follow-up results

## Scope and baseline

Baseline: pushed PR #886 head `8547b16`. Candidate: `585ca66`.
Both compilers were built from their actual source trees using the fastbuild
preset with warnings denied. The baseline tree was exported with `git archive`;
no other branch was created or checked out. Main remains the PR's existing base
`b5a6b6b9c7568a002198249cf81ec9d70924683e` at the time of these checks.
The workspace has an active 3 GiB swap file (`/swapfile-3g`, 3,221,225,472
bytes) in addition to its existing 1 GiB file. No package extraction into
system paths was repeated.

The source was reconstructed from the recorded edits after a sandbox failure.
These are **fresh checks and measurements of that recovered source**, not just
restatements of the measurements whose ignored workspace files were lost.

## Changes and safety review

### Unsigned scalar comparison recurrence

`scalar_compare_iv` permits a narrow exception to the still-opt-in general
scalar IVSR policy: a small read-only search loop, U64 IV and result domains,
unit positive step, affine offset one, power-of-two scale greater than eight,
and one direct unsigned comparison against a loop-invariant endpoint. A value
that escapes, forms an address, or feeds an additional consumer does not qualify.
Calls, stores, volatile/segment loads and large loop bodies decline the cost rule.

Affine Shl matching reuses the existing affine cast/no-wrap certificates.
Runtime affine initialization is emitted only in an unsigned arithmetic domain;
all additions and products obey the same modular width as the recurrence.
This matters at ULONG_MAX: no comparison is replaced by a division or a shifted
endpoint, which would be wrong when the original scaled value wraps.

General scalar-derived IVSR remains opt-in. The new production policy is enabled
only for x86-64, snapshotted per pipeline invocation. Disable it independently
with `CCC_DISABLE_PASSES=scalar_compare_iv`.

### Packed pair forwarding

`pair_load_carry` recognizes a two-block loop with exactly the accepted counted
header and a definitely-taken first body visit. The pointer phi advances by
exactly eight bytes; one eight-byte pair store at pointer+0 supplies the next
iteration's pair load at pointer-8. The original first load moves into the
preheader only after proving the first visit executes. No maybe-zero-trip load
is speculated, no load is widened, and no store is removed or reordered.

There can be no other writer, call, atomic operation, volatile load or inline
assembly in the body. Packed operations are restricted to independent dword
lanes with proven-zero upper lanes: pair loads, add/xor/or and immediate logical
shifts. Cross-lane consumers, externally used vector results, scalar escapes,
stride mismatches, duplicate definitions and unsupported headers decline before
mutation. The original load's SSA identifier becomes the carried vector phi;
source-span lengths remain synchronized.

The register allocator's I32x4 classification now admits packed OR as producer
and consumer through the existing home-aware emitter, keeping the phi web in
XMM registers. Without this companion change the transformation spilled the
carried pair, eliminating its benefit; the optimized schedule now has no
per-iteration reload of that pair. Disable the IR transformation independently
with `CCC_DISABLE_PASSES=pair_load_carry`.

The review above and targeted tests are not an independent audit or a formal
implementation certificate. No universal zero-defect or optimality claim.

## Fresh local checks

* Warning-denied fastbuild, lccc and lccc-ld: PASS.
* Full Rust suite: 5,472 library tests and one integration test PASS; ignored
  cases/doctests remain ignored, not counted as successful executions.
* Strict all-target Clippy, formatting, whitespace and environment-test hygiene:
  PASS, with no compiler warnings.
* `search_scaled_bound_wrap.c` and strengthened `pair_schedule_carry.c`: PASS
  with IR verification at -O0/-O1/-O2/-O3/-Os, and with GCC16 at
  -O2 -Wall -Wextra -Werror. The latter uses local arrays so schedule64 actually
  exercises pair packing and the new carry path. It includes odd tails, zero
  loops, wraparound data and a protected page after the final external word.
* Complete segment runtime contract, including the complex-secondary-return
  regression that exposed the earlier DCE defect: PASS.
* Three-round, one-warmup 46-workload benchmark screen: all outputs match GCC
  and the previous PR head. Only linux_find_bit, linux_find_bit_scaled and
  sha256_transform executables differ; the other 43 are byte-identical.
  Three rounds are a correctness/regression screen, not a precise speed claim.

Hosted checks are monitored on PR #886 for the final pushed head. The historical
37 local i386 dependency build failures are not relabeled as passes; hosted
provisioned-platform coverage is separate from these local results.

## Repeated runtime measurements

Xeon VM, CPU 0 pinned with taskset, no PMU probe. Fifteen timed paired rounds,
three excluded warmups, randomized compiler order, all timed samples retained.
Every compiler receives `-O2 -march=x86-64-v3 -fPIE -pie`, so this screen uses
explicitly matched ISA and position-independent code settings. This is not EPYC
hardware evidence, and the remaining GCC gaps are not claimed closed.

| Workload | Candidate / previous head, paired median [95% bootstrap CI] | Candidate / GCC16 |
|---|---:|---:|
| SHA-256 | **0.888 [0.879, 0.896]** | 1.024 [1.012, 1.038] |
| Scaled find-bit | **0.954 [0.945, 0.956]** | 1.145 [1.133, 1.157] |
| Switch dispatch | 1.001 [0.998, 1.011] | 1.155 [1.139, 1.172] |
| Byte-load reassembly | 1.001 [0.981, 1.015] | 1.632 [1.595, 1.646] |

That is about 11.2% less SHA runtime and 4.6% less scaled find-bit runtime
relative to the previous PR head. Switch and byte-load executable hashes are
unchanged by these follow-ups; no new gain is claimed for them. The byte-load
workload is below 20 ms and remains startup/noise sensitive. Switch-case
alignment experiments were neutral or slower and were rejected. The previous
indirect-dispatch concern remains unaddressed.

`followup-screening.json` retains the commands, compiler/CPU/ISA metadata,
source and executable hashes, correctness verdicts, timed results and paired
statistics for the focused screen and the entire 46-workload screen.

Example reproduction, with the previous-head compiler path supplied locally:

```sh
python3 tests/benchmark/run_benchmarks.py --compilers lccc,gcc \
  --gcc /path/to/gcc-16 --candidate before=/path/to/pr886-8547b16-lccc \
  --only linux_find_bit_scaled,sha256_transform,switch_dispatch,byte_load_reassembly \
  --cflag=-march=x86-64-v3 --cflag=-fPIE --cflag=-pie \
  --reps 15 --warmup 3 --no-pmu-probe --strict --json focused-runtime.json
```
