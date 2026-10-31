# SIMD address-fold red-team audit — 2026-10-31

## Findings and decisions

| Finding | Reproducer / evidence | Repair / reason |
|---|---|---|
| Alias metadata over-credits an offset edge | New `alias_metadata_must_not_double_credit_an_offset_use` failed before repair | Credit only the offset operand of a unique, actual GEP definition. An alias entry is not another IR use. |
| A store alias can need an offset that its original load-only GEP no longer needs | New `fallback_through_alias_keeps_original_offset` failed | Protect fallback offset roots globally, alongside selected machine indices. Looking only at the currently visited GEP is insufficient. |
| Missing use metadata was treated as zero uses | New `incomplete_use_metadata_must_fail_closed` failed | Missing/zero counts cannot justify deletion; require exact equality between real removed edges and total uses. |
| Unexpected effectful producers could be suppressed | New `side_effecting_offset_definition_is_never_suppressed` failed | Whitelist Copy, integer/pointer Cast and integer/pointer Add/Sub/Mul/Shl producers. A dead result never licenses removing an effect. This is defensive against future map expansion, not evidence that today's recognizer selects volatile loads. |
| Conservative store handling left redundant scale/spill work | Golden gate reproduced `glibc_memcmp` at 156 instructions, 18 stack operands; baseline allows 17 | Introduce a default-refusing target capability. Only x86-64 opts in under its existing shared operand/type and reserved staging-register contract. Result: 154 instructions, 17 stack operands. No baseline/tolerance change. |
| Scoreboard signed arithmetic was undefined | GCC 16.2 UBSan stopped on signed sink accumulation | Use defined unsigned wrapping and unsigned left-shift operands. Preserve signed right-shift behavior on the supported targets. |
| FP reduction receipts truncated results to int | Source audit | Hash the complete float/double result. Narrow integer receipts could miss low-bit errors. |
| Matrix hashing walked across a C row subobject | Source audit | Hash each row within its bounds, then combine unsigned hashes. |
| Small repetition counts skipped matmul / divided by zero | New configurable correctness mode | Reject nonpositive counts; execute matmul at least once. Test REPS=1 and 17, not only the default 4000. |
| AoS source emitted an indentation warning | Enabling GCC 16.2 `-Wall -Wextra -Werror` failed | Brace the output loop; require warning-clean compilation on both sides. |
| Killing only a compiler parent does not bound pipe-inheriting descendants | Process-lifetime audit and real parent/child timeout test | Start a POSIX process session, kill the process group on timeout/interruption, drain/reap the parent, then report failure. A real sleeping descendant is terminated within the timeout budget. |

Four additional backend tests demonstrably failed before the repairs; all now
pass. A slow, independently structured edge-count oracle checks **256**
combinations of fold eligibility, alias presence, fallback and ordinary uses.

### Why this data-flow design

1. Every implicit machine-index read is a protected root. Fallback offsets are
   additional roots, including those reached through aliases.
2. Initial credits correspond to real, unique GEP operand edges—not map size.
3. A producer becomes removable only when every actual use has been credited,
   its definition is unique, its opcode is allowed, and it is not protected.
4. Removing it credits each of its operand positions exactly once. Shared
   operands are revisited when their last remaining consumer disappears.
5. Hash iteration order does not select the result. The independent pull-style
   oracle tests this against the worklist result.

The traversal is linear in the scanned function plus credited operand edges;
its temporary maps/sets are per-function, ownership-managed Rust allocations.
No unsafe code, lock protocol, atomics or shared mutable state were introduced.
Counter growth is bounded by the actual use inventory for valid IR: each
unique instruction and operand edge is credited at most once. Missing metadata
and ambiguous definitions fail closed. This is not a claim that all compiler
IR construction elsewhere has been proved correct.

### Target boundary

The x86-64 store emitter shares address construction and accepted access types
with the load emitter. Integer value staging uses reserved rax, symbol staging
reserved rcx, and FP staging SIMD scratch registers: these are not allocatable
SIB homes. Under the existing folded-index liveness/freshness contract, stores
therefore add no value-dependent address refusal. The new capability records
that narrower guarantee instead of assuming it for every architecture.

Tests enumerate **156** accepted x86 type/scale/register profiles, including
rdx/r11 staging-sensitive homes. Separate assertions prove i686, AArch64 and
RISC-V do not inherit the opt-in. The common analysis and all backend library
tests run in the full suite. Native end-to-end measurements in this sandbox
are x86-64 only; no native ARM/RISC-V execution is claimed.

## Validation results

- Hash-verified release-tag cache restored/verified Rust 1.99.1, GCC 16.2 and
  Clang 23.1.4. Fastbuild compiled **lccc and lccc-ld**, one Cargo job, 4 GiB
  active swap, `RUSTFLAGS='-D warnings'`.
- Full library suite: **7,098 passed, 10 ignored, zero failed**. Clippy with
  warnings denied passed. Rustfmt and whitespace checks passed.
- Python suite: **17 passed**, including parser mutations, failed references,
  stale artifacts, signal/timeout handling and a real descendant timeout.
- **3,276** AoS differential comparisons passed with C warnings denied:
  2,268 trip/page-layout checks and 1,008 type/stride/mixed-sign checks, against
  GCC 16.2 and Clang 23.1.4.
- **528** scoreboard checksum comparisons passed: 44 kernels × three candidate
  optimization levels × two reference compilers × REPS={1,17}.
- Default 4000-repeat corpus: GCC 16.2 and Clang 23.1.4 UBSan instrumentation
  each completed all 44 rows without diagnostics. The cached toolchains do not
  bundle the sanitizer runtime: instrumentation was linked to the installed
  system `libubsan.so.1`, rather than falsely claiming a cached vendor runtime.
- Golden codegen gate passed unchanged. Existing scratch-index, RMW/SIB,
  immediate-store/SIB and decimal64 indexed-fold regressions passed.
- CI lane-plan checker/self-tests and codegen-gate self-tests passed. Hosted
  results must be read from the PR checks for the final head, not inferred
  from a local pass or an older successful run.

## Performance screening and limits

`glibc_memcmp` whole-process screening used PASSES=16384, 24 order-balanced
samples per binary, two warmups, CPU 1 affinity, identical output across the
before/after/GCC/Clang executables. Median wall times were 27.036/27.000/26.989/
26.445 ms respectively. The after/before median ratio was **0.9986**: effectively
unchanged at this resolution, **not a significant speedup**. Process startup
is included; frequency, PMU and target-hardware effects are not controlled.
Raw samples and executable digests are retained in the adjacent JSON report.

The offline Godbolt witness remains 41/9/29 static instructions for
lccc/GCC/Clang. This exposes a general-vectorization gap; it does not prove a
runtime ranking. Remote Godbolt and Intel's PPA are outside this sandbox's
network allowlist. The existing authenticated ICX 2026.2 hosted lane now
executes both probe families; local ICX execution is not claimed.

The one-day randomized acceptance, full misalignment/overlap sweep, controlled
paired kernel timing and SV-03–SV-12 implementations remain open. Saxpy's
in-timing reset and sqrt's negative inputs still disqualify their exploratory
numbers from performance claims. No function is asserted to be universally
optimal, nor is the entire repository asserted defect-free on the strength of
these scoped tests.
