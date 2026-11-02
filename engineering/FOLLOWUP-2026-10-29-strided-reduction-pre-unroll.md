# Follow-up: run the strided-I32 reducer before scalar unrolling

## Finding

The attached 2026-10-29 P/E-core report measured `byte_load_reassembly` at
1.343x the GCC median cycles and 1.366x the GCC instruction count on the P
core; on the E core it was 1.861x cycles and 1.366x instructions. The report
also showed a substantial `zlib_ng_slide_hash` cycle regression (2.480x P,
2.017x E). These are report measurements, not measurements of this patch.
The report records GCC 16.2.1 and a physical i7-14700KF, with 9 paired PMU
rounds; PMU counters and wall time are separate metrics.

Auditing the current `byte_load_reassembly` loop against the reducer exposed a
pipeline-ordering miss: the strided-I32 reducer models the original byte-IV
step and a scalar remainder, but the general early scalar unroller runs before
the main vectorizer and can widen that IV step (8 to 128), so the later
reducer declines the loop. The existing `try_transform` is fail-closed; this
change does not broaden its accepted loop shapes or alter its safety proof.

## Change

* Added a narrowly scoped pre-unroll pass entry that tries only the existing
  strided-I32 reducer. It runs only for x86-64 `-O2+`, non-size optimization,
  with vectorization enabled. It uses the existing AVX2/transform legality
  checks and module symbol facts. A declined loop is left untouched for the
  ordinary unroll/vectorize pipeline.
* Added a regression assertion to
  `tests/regression/check_vector_remainder_codegen.sh` that the dedicated pass
  reports a nonzero transformation count for the strided reduction fixture.
  The test still checks packed 256-bit code generation and compares executable
  output with GCC, including the global-object and dynamic-tail safety cases.

This is independent follow-up work to the open PR #1021. It does not claim to
solve that PR's separate unsigned-saturating-subtract or dispatch/trace work,
nor does it claim that these changes make the complete report corpus faster.

## Validation and limits

* `cargo build --profile fastbuild --bin lccc --bin lccc-ld` passed with the
  restored Rust 1.99.1 cache toolchain.
* Focused `check_vector_remainder_codegen.sh` passed with GCC 16.2.0; it
  verified the new pre-unroll pass transformed the fixture, vector code was
  emitted, and output matched the GCC executable.
* Twelve report-relevant benchmark correctness checks passed against LCCC,
  GCC 16.2.0, and Clang 23.1.4. The run used `--skip-perf`; it emitted no
  performance measurements.
* ICX 2026.2 is not installed in the restored toolchain cache or the standard
  oneAPI paths checked in this environment. `valgrind` is also not installed.
* This sandbox exposes no perf PMU, and no physical-target A/B timing was
  performed. Consequently this follow-up makes no Raptor Lake performance
  improvement claim. Hosted GitHub CI is tracked on the new PR and is not
  represented as locally run CI.
