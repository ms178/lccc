# Compiler-candidate audit

Date: 2026-10-15. Base: `f07f4c514460e4116a574773cd84b69d185bf65e`.
Compared source trees: PR #835 (`4216f897460be1eef08cc0f24e8b6016cd738283`)
and PR #836 (`3671c7c9a088226b2962ff9712c178da79e7b9d4`), followed by the
changes in this tree. Source archives, not branch switches, isolate references.

## What the experiments establish

The independent, seven-test adversarial comparison contract was replayed
against both original runners and the corrected runner:

| Runner | Tests | Failures | Errors |
|---|---:|---:|---:|
| PR #835 | 7 | 5 | 0 |
| PR #836 input | 7 | 5 | 0 |
| Corrected tree | 7 | 0 | 0 |

The five reproduced defects were:

1. an unstable GCC control could certify a candidate matching its first output;
2. unavailable requested GCC could be replaced by the candidate itself;
3. successful samples could conceal failed candidate executions;
4. the GCC aggregate checked candidate correctness but not control correctness;
5. fastest-reference selection admitted an incorrect reference compiler.

These are **harness defect reproductions**, not compiler speed measurements.
The regression authority is
`tests/corpus/test_benchmark_comparison_safety.py`. The baseline Python corpus
passed 545 tests with five skips; the competing tree passed 582 with four skips.
Test counts are not interchangeable coverage or quality scores. The corrected
tree's full corpus passed 576 tests in 79.628 seconds with five explicit skips;
the focused comparison-report and workflow suite passed all 15 tests.

A native GCC 12.2 smoke run of all 65 semantic cases is also informative:
all 65 independent `-O0` controls passed, but the optimized GCC arm failed four
`fenv_constant_rounding` cases with exit 52: a discarded division no longer
raised the expected exception. No self-check was removed. This is why the
semantic matrix explicitly uses an unoptimized independent GCC control and
records the optimized GCC arm separately. It does **not** establish GCC 16.2
behavior or LCCC correctness. GCC's pragma support must not be assumed merely
because it is the reference compiler.

## Existing hosted screening: evidence, not a new timing experiment

Checks-API annotations from the original Debian jobs are retained in
[`measurements/compiler-comparison-screen.json`](measurements/compiler-comparison-screen.json).
The run-page source identities are the synthetic PR merge commits, recorded in
that file, not silently relabelled as the new compiler.

- [PR #836 Debian job](https://github.com/ms178/lccc/actions/runs/38781829546/job/116583738392)
- [PR #835 Debian check](https://github.com/ms178/lccc/runs/116522202545)

Both native reports are correctness-only (`skip_perf=true`, one run): no timing
ranking can be inferred. Both report Debian GCC 16.2.0-3. There are 44 matching
source hashes; `tce_sum` changed its size macro and is excluded from this
cross-run comparison. The GCC simulated `Ir` control is identical for all 44
matching shapes. Illustrative whole-program Callgrind counts:

| Same-source shape | PR #836 input Ir | PR #835 Ir | Input / competing |
|---|---:|---:|---:|
| spectral_norm | 5,503,480 | 5,170,615 | 1.06438 |
| struct_copy | 395,646 | 389,646 | 1.01540 |
| lz4_compress | 654,048 | 649,093 | 1.00763 |
| zstd_count | 473,614 | 471,574 | 1.00433 |
| zlib_ng_adler32_combine | 21,869,096 | 21,946,318 | 0.99648 |
| fannkuch | 1,275,870 | 1,310,231 | 0.97377 |

These are **separate-job, bounded-size, simulated instruction screens**, not
native speedups, hardware cycles, or results for the edited compiler. They
refute a blanket claim that either input is uniformly better. The large
shared gaps to GCC also remain visible: `byte_load_reassembly` executes
72,125,195 simulated instructions versus GCC's 16,834,535 (4.28436x), and
`ring_fifo` 637,710 versus 265,362 (2.40317x). An attractive aggregate would not
remove these weaknesses or establish a fix.

## Implementation decisions and their proof obligations

| Area | Decision | Regression authority / limitation |
|---|---|---|
| Managed temporary paths | Adopt leased directory ownership and fallible reservation; remove flat-name fallback; preserve `OsStr` at subprocess boundaries | Concurrent/pending/kept lifetimes, failure cleanup, separators/NUL, valid Unix filename characters, symlink and non-UTF-8 driver cases |
| Packed F64 reciprocal denominators | Use the competing tree's correct I32x4/XMM class in all three home collectors, not I32x8/YMM | Six producer shapes; each relevant collector checked; exact-bit and guarded-tail runtime fixture remains required |
| Architectural register renaming | Preserve the competing fixed/implicit-operand checks, add destination-RCX and invalid-ID guards, retain Unicode-safe scanning and opaque-instruction barriers | MUL/DIV/string implicit operands; ten shift/rotate families and five suffixes; ordinary RCX addresses/BMI2 remain positive controls |
| Never-read frame stores | Adopt architectural read extents rather than arbitrary 16/32/64-byte caps; use wide endpoint arithmetic | ZMM, fixed 512-byte FXRSTOR, unknown XSAVE/state images, MXCSR four-byte control, nonoverlapping stores, i32 endpoint boundary |
| FP liveness and branch fusion | Retain this tree's stronger CMOV/source-width, conditional XSAVE, pin, CFG-edge and live-register requirements | Existing executable and Rust regressions; do not trade defined behavior for smaller assembly |
| AArch64 shared ADR | Adopt structural local/allocated/non-TLS section provenance, checked address construction, object identity and valid-zero support | UNDEF/ABS/reserved/discarded/nonallocated targets rejected; named and section-symbol controls; output sentinel preserved on rejection |
| Arithmetic policy | Keep wrapping and observable FP state independent across compilation boundaries | Four policy combinations at O0/O1/O2/O3, not a test that toggles both together |
| Standalone RV64 tests | Keep the pure displacement sweep standalone; test the real instruction encoder in its production module | Exact asymmetric AUIPC bounds and XLEN wrapping retained; no invented alignment restriction |

The AArch64 linker does not yet consistently serialize merged hidden/protected
visibility through dynsym/GOT/PLT. Consequently shared ADR to nonlocal symbols
is explicitly unsupported, even with a visibility declaration; pretending
that a numeric address proves local binding would be incorrect. This is a
capability limitation, not a claim that general ELF visibility is complete.

Competing unsigned-loop normalization, inline-assembly vector traits and CSE
changes were not bulk-copied without independently applicable proofs. The
existing affine proof already owns a kill switch and signed/unsigned,
full-width, optional-exit and modular-sequence tests. Extra optimization is
not justified by source size or by an old green check alone.

## Reproducible hosted comparison

The opt-in `compiler-comparison` job in `pr826-redteam.yml` is enabled either by
its dispatch input or the `compiler-comparison` PR label. It uses actual Debian
GCC 16.2.0 and the current stable Rust/cargo/rustfmt/clippy. It archives exact
same-repository main/competing/prior trees, builds them with one toolchain, and
records source IDs, executable hashes, versions, commands and raw outputs.

1. **Semantic witnesses:** 65 current-tree cases across O0/O1/O2/O3/Os, with
   independent GCC `-O0` controls and separately labelled optimized GCC.
   Historical failures remain failures in the inventory; final and control
   failures fail the job. This is not timing evidence.
2. **Native workload comparison:** all 45 registry IDs, shared O2 target and
   wrapping flags, randomized paired rounds, affinity where available, one
   warm-up and nine retained samples. The main compiler participates in the
   semantic/static inventory rather than the timing aggregate: its old runtime
   discovery is incompatible with a fresh GCC-16-only image.
3. **O3:** all 45 shapes, correctness-only, never a second timing claim.
4. **Static codegen:** `benchmark_codegen.py --oracles ''`, every registry ID
   per compiler; instruction counts are screening, not runtime or spill cause.
5. **Bounded instrumentation:** all 45 IDs using `callgrind_corpus.py`, both a
   direct candidate comparison and a separate GCC control at the same sizes.
   Raw counters, native/instrumented output checks and manifests are retained.

A failure in one measurement does not suppress the other independent
measurements after a successful build. No `continue-on-error` converts it into
success. Bounded Checks annotations make evidence readable without relying on
an artifact-download host. Live Compiler Explorer calls remain outside CI,
including the optional lane; offline helper coverage is not a claim of an
ICX/multi-vendor survey.

The report rejects missing/duplicated shapes and unqualified timing arms.
Primary screening aggregates exclude incorrect or unstable executions, failed
GCC controls, fewer than seven actual pairs, and either median below 20 ms.
Excluded rows remain in the correctness inventory. Hosted wall times are VM
screening, never bare-metal performance or proof of universal superiority.

## Environment and remaining acceptance criteria

The local sandbox is Debian 12 with GCC 12.2; Rust, Clang, Valgrind and hardware
PMU access are unavailable. The repository swap script activated a real 2 GiB
swap file. Network restrictions prevented installing local Debian GCC 16.2 or
Rust; those exact tools must execute in hosted jobs. The cross-target boundary
matrix, which requires GCC 16.2.0 explicitly, belongs in the exact Debian lane,
not a rolling-major Ubuntu PPA lane. Old green checks and
Python contracts do not validate the edited Rust.

Before claiming completion, inspect current-head compiled tests, strict
Clippy/rustfmt, all required cross-target and linker gates, and the named
comparison evidence. Any discovered miscompile must retain its failing case
and independent control. Native performance conclusions require qualified
samples; neither byte counts nor simulated `Ir` alone settle them.
