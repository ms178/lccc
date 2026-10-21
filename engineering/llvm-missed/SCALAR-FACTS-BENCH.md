# scalar facts measurements — 2026-10-21

## Experiment

Baseline: latest main after PR #887 (`0c4f3929dd75354d80bae881b6d579e2426fe789`).
Candidate Rust sources are unchanged by this evidence-only publication; exact
source-tree and compiler-binary digests are in `scalar-facts-correctness.json`.
The archived baseline was compiled separately and retained immutably. A shared
Cargo output-directory collision was detected by the mechanism gate, rejected
before timing, and resolved by cleaning the compiler package and rebuilding.
Use **separate target directories** when reproducing baseline/candidate builds.

Intel Xeon 2.60 GHz virtual machine, two visible CPUs, pinned to CPU 0; not an
EPYC 7763. Active 3 GiB swap. No concurrent compilation during timing. PMU
events unavailable because perf is not installed; no instruction/cycle claim.

All arms use `-O2 -march=x86-64-v3 -fPIE -pie`, no LTO or fast-math.
Exact references: GCC 16.2 and Clang 23.1.3. ICX 2026.2 was not available in
the local tag caches and has **no local timing result**; authenticated hosted
ICX integration is a separate CI contract, not a replacement timing reference.

21 randomized paired rounds per arm, three excluded warm-ups, bootstrap 95%
confidence intervals, no removed outliers. Raw samples, commands, metadata,
outputs and hashes: `scalar-facts-runtime.json`. The generic raw dashboard also
considers explicitly labelled control arms when selecting its “best reference”;
the table below deliberately reports vendors and controls separately.

## Paired wall-time ratios

Ratios are candidate/control, **lower is better**. These are two synthetic
witnesses, not application speedups or a whole-corpus performance aggregate.

| Workload | Latest-main baseline (95% CI) | Stealing disabled (95% CI) | GCC 16.2 (95% CI) | Clang 23.1.3 (95% CI) |
|---|---|---|---|---|
| llvm_scalar_facts | 0.9240 [0.8927, 0.9671] | 1.0160 [0.9864, 1.0622] | 0.9362 [0.8714, 0.9668] | 0.9784 [0.9433, 1.0338] |
| hot_web_loop | 0.9550 [0.9268, 0.9589] | 0.9568 [0.9485, 0.9673] | 1.7079 [1.6731, 1.7137] | 1.9565 [1.9070, 1.9665] |

The scalar bundle reduces median elapsed time by **7.60%** versus main in
this run. Candidate and `no_steal` executables are byte-identical for that
bundle: their measured difference is noise, not an allocator effect. Its
Clang confidence interval crosses 1, so no statistically resolved Clang win
is claimed.

The hot-web witness reduces median elapsed time by **4.50%** versus main
and **4.32%** versus disabling the new phase. Main and `no_steal` executables
are byte-identical for that witness, isolating the allocator change. Both
intervals exclude 1. Nevertheless it remains **1.71× slower than GCC** and
**1.96× slower than Clang**. This is useful but incomplete tuning, not closure
of the compiler-performance gap or proof that the cap of three is optimal.

## Correctness and non-regression boundaries

- All **48/48** registered workloads agree with GCC 16.2, Clang 23.1.3 and
  the saved latest-main baseline with aborting IR and RA verification.
- The **46 original workload binaries are byte-identical** to latest main,
  including gzip CRC, SHA-256 and both find-bit workloads. This is stronger
  evidence of no generated-code regression for this configuration than noisy
  spot timings; it does not certify every flag, input or target.
- Only the two new synthetic witnesses change. Full per-workload output and
  binary hashes are in `scalar-facts-correctness.json`.
- Strict fastbuild units: **5,504 passed, 0 failed, 10 ignored**, plus one
  binary unit; three documentation examples remain intentionally ignored.
- Strict all-target Clippy, both fastbuild binaries, formatting and diff
  checks pass with zero compiler warnings.
- Native cohort: O0/O1/O2/O3/Os, O3 debug, O2 wrapv, disabled controls,
  independent exact GCC/Clang oracles, real freestanding i686 execution, and
  per-function mechanisms pass. Hot-web pressure/reference comparisons also
  pass with the phase off/on and caps 0/1/2/3.
- Earlier zero-choice, integer and bit demand native gates pass. Existing
  i686 constant-divide/magic-number gate passes. Inventory tests pass in
  normal and optimized Python; nine benchmark-harness tests pass.
- Independent Z3: **72/72** expected results (58 UNSAT laws, 14 SAT negative
  controls), no UNKNOWN. These are algebra checks, not a machine-code proof.

## Static mechanisms

Common `-O3 -march=x86-64-v3 -fPIE -S`; instruction counts include RET,
not bytes or latency. Across the thirteen newly reduced entries, twelve
have fewer instructions and the shared-remainder entry ties at 13 while
eliminating the quotient-dependent multiply. The distinct-semantics mask
control also legally improves with a different clearing mask.

| Entry | Main | Candidate | GCC 16.2 | Clang 23.1.3 |
|---|---:|---:|---:|---:|
| b6_bits | 14 | 5 | 11 | 11 |
| b6_sum | 10 | 9 | 8 | 6 |
| b6_sub | 10 | 9 | 8 | 6 |
| b6_range | 12 | 5 | 9 | 10 |
| b6_even | 9 | 4 | 5 | 4 |
| b6_shared | 13 | 13 | 11 | 12 |
| b6_masked | 9 | 4 | 10 | 10 |
| b6_select | 17 | 4 | 10 | 9 |
| b6_signed_select | 6 | 4 | 4 | 4 |
| b6_inverse | 33 | 33 | 17 | 25 |
| b6_trip | 6 | 4 | 5 | 5 |
| b6_guard | 9 | 6 | 4 | 4 |
| b6_lowzero | 4 | 4 | 4 | 4 |
| b6_lowone | 7 | 4 | 5 | 5 |
| b6_boolor | 8 | 4 | 4 | 7 |
| b6_trip_near | 6 | 6 | 5 | 5 |
| b6_mask_near | 9 | 4 | 10 | 10 |
| b6_select_near | 5 | 5 | 4 | 4 |

The full eighteen-entry table has 13 fewer/5 equal/0 more instructions versus
main, and 8 fewer/3 equal/7 more versus each vendor. It includes already-
handled and distinct-semantics controls; it is not thirteen new report wins.

## Reproduce

Build both compilers with the fastbuild profile and strict warning flags,
using separate Cargo target directories and a swap file of at least 3 GiB.
Run the native gate with the saved baseline and exact reference executables.
Create a labelled no-steal compiler wrapper using the command recorded in
`scalar-facts-correctness.json`, then:

```sh
python3 tests/benchmark/run_benchmarks.py \
  --lccc /path/to/current --gcc /path/to/gcc-16 --clang /path/to/clang-23.1.3 \
  --compilers lccc,gcc,clang --candidate before=/path/to/saved-main \
  --candidate no_steal=/path/to/no-steal-wrapper \
  --only llvm_scalar_facts,hot_web_loop --reps 21 --warmup 3 \
  --opt=-O2 --cflag=-march=x86-64-v3 --cflag=-fPIE --cflag=-pie \
  --strict --json runtime.json
```

Rerun without `--only`, remove the no-steal arm and add `--skip-perf` with
`CCC_VERIFY_IR=abort CCC_VERIFY_REGALLOC=1` for full-corpus correctness.
Hardware-controlled application measurements on the intended EPYC remain
necessary before extending these results beyond the measured witnesses.
