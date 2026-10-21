# Scalar count measurements — 2026-10-21

## Experiment

The baseline is the semantic-name/rebased tree before count composition,
`2883e5e`, including the guarded-folds work and current main through
`91bbcda81fbcb9750298f9869fb5b990d9d819f5`. This is a fresh comparison,
not a relabeling of the historical guarded-folds speedups. Immutable compiler
and workload hashes are in `word-counts-correctness.json` and the raw runtime
report. A final comment-only cleanup was rebuilt: compiler and linker compared
byte-identical with the exact artifacts used for correctness and timing.

Intel Xeon 2.60 GHz VM, two visible CPUs, pinned to CPU 0; **not EPYC 7763**.
Active 3 GiB swap; no concurrent local build or regression execution during
timing. All arms use `-O2 -march=x86-64-v3 -fPIE -pie`; the scaled find-bit
fixture retains `-DPASSES=65536`. Exact GCC 16.2 and Clang 23.1.3 references.
ICX has no local timing result. PMU is not probed; no cycle or dynamic
instruction claim is made.

21 randomized interleaved paired rounds, three excluded warm-ups, bootstrap
95% intervals; MAD outliers reported, none removed. No aggregate application
speedup is inferred. `llvm_word_counts` is explicitly a synthetic call bundle,
not evidence of a whole-application gain.

## Candidate/control elapsed-time ratios

Lower is better. Full commands, raw per-round samples, output checksums,
compile times, executable/text sizes and environment: `word-counts-runtime.json`.

| Workload | Rebased baseline (95% CI) | GCC 16.2 (95% CI) | Clang 23.1.3 (95% CI) |
|---|---|---|---|
| llvm_word_counts | 0.8730 [0.8538, 0.8867] | 0.8678 [0.8511, 0.8811] | 1.2674 [1.2457, 1.2859] |
| hot_web_loop | 0.9919 [0.9830, 1.0202] | 3.0078 [2.9635, 3.0848] | 2.8043 [2.7085, 2.8419] |
| switch_dispatch | 1.0106 [1.0035, 1.0466] | 1.1086 [1.1000, 1.1174] | 0.9954 [0.9918, 1.0165] |
| gzip_crc32 | 0.9976 [0.9951, 1.0014] | 0.8758 [0.8460, 0.8820] | 0.8808 [0.8760, 0.8869] |
| byte_load_reassembly | 1.0025 [0.9686, 1.0384] | 1.3971 [1.3390, 1.4550] | 1.6427 [1.5831, 1.6836] |
| linux_find_bit | 0.9974 [0.9867, 1.0059] | 1.0201 [1.0034, 1.0266] | 1.0328 [1.0234, 1.0427] |
| linux_find_bit_scaled | 1.0028 [0.9712, 1.0309] | 1.0175 [0.9996, 1.0474] | 1.0298 [1.0133, 1.0549] |
| sha256_transform | 1.0096 [0.9963, 1.0473] | 0.9573 [0.9477, 0.9873] | 0.8351 [0.8151, 0.8506] |

The count bundle has **12.7% lower median elapsed time** in this VM run, with
its interval below one. The mechanism is independently visible: default-target
min trailing 18→6, min leading 22→7, cap trailing 11→4, cap leading 13→5.
Those disassembly counts use default target settings, not the matched-v3
timing flags; they are static observations including RET, not PMU readings.

All seven retained timing controls are byte-identical between candidate and
baseline under the timing flags. **Switch still measures 1.0106 with interval
[1.0035, 1.0466] despite identical executables.** This demonstrates residual VM/
measurement effects; the interval is not proof of a code-generation regression.
Do not discard that result, count identical-binary fluctuations as gains, or
treat statistical intervals as a universal causal/hardware guarantee.

## Correctness and scope

All **49 workloads** agree across candidate, rebased baseline, GCC and Clang.
Only `llvm_word_counts` changes its executable in the default-`-O2` correctness
sweep: **48/49 are byte-identical**. This includes every original workload,
gzip, SHA, both find-bit sizes, byte reassembly, dispatch and hot-web promotion.

Library units: **5578 passed, 0 failed, 10 ignored**. Strict fastbuild and
all-target Clippy pass with zero warnings; the active swap file is 3 GiB.
The dedicated native/IR/SSA/RA/debug/disable/refusal checks and all renamed
semantic regression suites pass. The root count fixture passes the standalone
corpus runner. Hosted/local invocation parity passes **331 commands**; both
normal and optimized inventory tests pass. Independent count proofs produce
**24 expected outcomes, no UNKNOWN**, not a compiler/machine-code certificate.

See `WORD-COUNTS.md` for the six-report audit: four opportunities remain
explicitly deferred. A stale integer-observation gate was reproduced on the
rebased baseline: disabled Boolean-return shortening tied the NOT-mask count.
The gate now requires the exact mask/equality shape and a NOT in its disabled
control, rather than falsely requiring fewer instructions for that case.
