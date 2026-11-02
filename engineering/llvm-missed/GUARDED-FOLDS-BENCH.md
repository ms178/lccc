# guarded folds measurements — 2026-10-21

## Experiment

Baseline: merged main `cfb2e0a822e2dba42a3d474f5970ab529e0e5e84`.
Immutable warning-denied fastbuild compiler executables; exact executable,
`src/` tree, Cargo manifest, source and workload hashes are recorded in
`guarded-folds-correctness.json`. Runtime compiler/workload hashes match the full
correctness run. No mutable Cargo output is used as a timing arm.

Intel Xeon 2.60 GHz virtual machine, two visible CPUs, pinned to CPU 0.
Active 3 GiB swap. **Not an EPYC 7763 measurement.** No concurrent Rust/C
builds or regression execution during the retained run. An earlier incomplete
attempt overlapped a final native check and was discarded in full.

All arms use `-O2`, compiler default target settings, no forced `-march`, LTO or
fast-math; the scaled find-bit fixture adds its existing `-DPASSES=65536`.
Vendor numbers are default-target comparisons, **not an ISA-normalized ranking**.
Exact references: GCC 16.2 and Clang 23.1.3. ICX 2026.2 was not available in
the inspected local tag caches; no local ICX timing is claimed. Hosted ICX
integration is a separate contract. PMU was not probed and perf is unavailable;
there are no measured cycle/instruction claims.

21 randomized interleaved paired rounds, three excluded warm-ups, bootstrap
95% confidence intervals. MAD outliers are reported, **none removed**. Full
samples, commands, outputs, compile times, executable/text sizes and metadata:
`guarded-folds-runtime.json`. No cross-workload aggregate speedup is asserted.

## Paired wall-time ratios

Candidate/control; **lower is better**. All eight changed executables are
screened, alongside six unchanged hot-workload/cohort controls. Small/short
kernels, particularly Fibonacci and SipHash, have wide VM uncertainty.

| Workload | Main (95% CI) | GCC 16.2 (95% CI) | Clang 23.1.3 (95% CI) |
|---|---|---|---|
| llvm_scalar_facts | 1.0002 [0.9910, 1.0056] | 0.9031 [0.8971, 0.9090] | 1.0360 [1.0253, 1.0415] |
| hot_web_loop | 0.9980 [0.9944, 1.0020] | 2.6072 [2.5667, 2.6306] | 2.3610 [2.3552, 2.3778] |
| arith_loop | 0.9442 [0.9387, 0.9553] | 0.9981 [0.9925, 1.0009] | 0.9967 [0.9935, 1.0011] |
| fib | 1.0042 [0.9358, 1.0619] | 0.0163 [0.0153, 0.0172] | 0.0102 [0.0096, 0.0108] |
| sieve | 0.9984 [0.9913, 1.0001] | 0.9411 [0.9242, 0.9552] | 0.9590 [0.9515, 0.9705] |
| switch_dispatch | 0.9534 [0.9474, 0.9718] | 1.1091 [1.0818, 1.1473] | 0.9341 [0.9262, 0.9634] |
| aarch64_select_patterns | 0.9939 [0.9814, 1.0047] | 1.0600 [1.0473, 1.0732] | 1.0545 [1.0493, 1.0711] |
| gzip_crc32 | 1.0008 [0.9986, 1.0043] | 0.8771 [0.8741, 0.8816] | 0.8813 [0.8781, 0.8966] |
| byte_load_reassembly | 0.9219 [0.9045, 0.9363] | 1.2691 [1.2465, 1.3299] | 1.4504 [1.4278, 1.4857] |
| zlib_ng_adler32_combine | 0.9948 [0.9529, 1.0343] | 1.0106 [0.9465, 1.0351] | 1.0476 [0.9764, 1.0594] |
| expat_siphash24 | 1.0080 [0.9279, 1.1137] | 0.9509 [0.9058, 1.0484] | 1.0117 [0.9391, 1.0316] |
| linux_find_bit | 1.0008 [0.9959, 1.0096] | 1.0194 [1.0036, 1.0260] | 1.0564 [1.0487, 1.0653] |
| linux_find_bit_scaled | 1.0043 [0.9851, 1.0239] | 1.0297 [1.0194, 1.0892] | 1.0759 [1.0688, 1.1336] |
| sha256_transform | 1.0023 [0.9803, 1.0556] | 1.0005 [0.9798, 1.0118] | 0.9544 [0.9378, 0.9973] |

## Interpretation and mechanism

* Standard byte reassembly: **7.8% lower median elapsed time** versus main.
  Three saved-GPR staging copies disappear; memory access widths are unchanged.
* Standard switch dispatch: **4.7% lower median elapsed time**. The masked-index
  MOVSLQ disappears only with the full 64-bit identity proof.
* Existing 32-variable arithmetic-pressure kernel: **5.6% lower median elapsed
  time**. This is a kernel result, not an application-wide percentage.
* These three main comparisons have confidence intervals wholly below one.
  The other changed-workload intervals include one: no gain or regression is
  resolved by this screen. Vendor performance gaps remain visible above.
* **40/48 correctness executables are byte-identical** to main, including gzip,
  SHA, both find-bit sizes, the hot-web loop and scalar facts scalar cohort. Tiny
  measured movements for identical binaries are noise, not newly claimed wins.
* New reduced contracts shrink native instruction counts versus main and the
  disabled pass: cmp32 9→6, cmp64 10→6, range 9→5, null/Boolean PHI 27→17.
  Instruction counts are static disassembly observations, not PMU readings.

## Correctness and scope

All **48 workloads** agree across candidate, main, GCC 16.2 and Clang 23.1.3.
Strict fastbuild and all-target Clippy pass with zero warnings; library tests
are **5549 passed, 0 failed, 10 ignored**; selected bin targets have no unit
tests. Native/i686/guard-page/IR/RA/disable and prior batches 1/2/3/6 pass.
The ICF suite is **18 passed, 0 failed, 0 warnings, 0 skipped** against the bfd
oracle. Different C++ catch types remain distinct; identical unwind still
folds. An archived **interim pre-ICF linker**, not a separately archived main
linker, demonstrates the previous unsafe different-type aliasing.

The independent solver returns **26 expected outcomes, no UNKNOWN**; algebra
checks do not certify the matcher or machine code. Review and refusal scopes
are in `GUARDED-FOLDS.md`, `guarded-folds.json`, and the coverage ledger. Historical
general XMM splitting, vectorization, tail-layout and dispatch-policy work is
not declared solved by these narrower mechanisms.
