# Scaled-index addressing: hardening and validation

## Contracts and design decisions

The affine-shift policy preserves small-scale GEP offsets for native addressing.
This follow-up deliberately does **not** extend that profitability veto to
pointer-integer Adds or vector byte-offset triples: those paths have different
costs and existing recurrence machinery. Tests pin their existing eligibility.
AArch64 register-offset scales remain access-width-dependent; no universal
scale-folding guarantee is implied.

### Liveness coordinates

`u32::MAX` is the undefined-point sentinel. Both ordinary liveness and synthetic
vector intervals now preflight the sum of instructions and terminators, before
constructing coordinates. The exclusive endpoint must remain below the sentinel.
Unsupported capacity produces an explicit assertion diagnostic through existing
compiler failure reporting instead of saturating and silently losing intervals.
The check costs O(blocks) time and O(1) auxiliary space; it introduces no runtime
instructions into compiled C. Changing the entire allocator to 64-bit coordinates
would increase storage and touch unrelated interfaces without practical benefit.
Boundary tests operate on block lengths, not enormous allocated IR fixtures.

### Shared-arm duplication

Before mutating IR, validate plan indices, selected edge, merge existence,
exactly one source incoming per phi, fresh-label availability and identifier
capacity. Store the phi repairs, then clone and commit without an ordinary
failure return after mutation. This prevents missing or later malformed phi
inputs from leaving a partially transformed function. Tests compare the complete
function debug representation on ten failure shapes and verify SSA on both edge
orientations after success. This is transactional with respect to validation
failure, not a claim of recovery from process OOM or allocator aborts.

### Local structure and bounds

Restore the copy-bridged load-fusion helper without changing its algorithm or
legality guards. Keep an explicit three-member set-membership threshold and
compute cluster bounds in one seeded reduction. Neither change claims a measured
runtime speedup. The audit ratchet is regenerated from source rather than used
as an optimization objective.

## Regression coverage

- IVSR shifts 0–4: scalar, GEP, Copy/same-width-cast alias with mixed users,
  pointer-integer address and vector byte-offset consumers; fixtures verify SSA,
  initialize label state and restore the compare-recurrence flag using the shared
  panic-safe scoped guard.
- Exact wide-pointer consumers and independent inexact Load, Store, GEP-base and
  wide stored-value refusal cases; refusals preserve instructions and counters.
- Ten invalid-plan/capacity cases and two successful shared-arm edge orientations.
- Program-point empty, ordinary, maximum-valid, sentinel, cumulative-overflow and
  machine-size boundaries without giant allocations.
- Set-membership spans 31/32/63/64, exhaustively compared over every input byte.
- `affine_scaled_index_matrix.c`: five element sizes (1/2/4/8/16 bytes), nine trip
  counts, calls between affine accesses, all destination elements and wide-field
  guards included in hashes. A separate Python integer model computes expected
  results, avoiding agreement between two miscompiled C implementations as the
  only correctness oracle.
- The matrix is wired into local and GitHub contract gates; authenticated ICX CI
  also compiles and executes the new fixture.

## Local evidence

Toolchains were retrieved from the repository's tag cache and checked against
its SHA-256 manifests: Rust 1.97.0 (the declared/tested MSRV), GCC 16.2.0 and Clang
23.1.3. The Rust `stable` label printed by the wrapper does not make this cached
MSRV the latest upstream stable release. An 8 GiB swap file was activated.

Recorded successful checks:

- Warning-denied `fastbuild` of all compiler binaries and `lccc-ld`.
- All-target fastbuild tests: 5,538 library tests and one linker test passed;
  ten explicitly ignored library tests remain ignored.
- Assertion-enabled all-target fastbuild tests: the same 5,538 library tests
  and one linker test passed; ten explicitly ignored tests remain ignored.
- All-target Clippy with `-D warnings`; no lint suppressions added.
- Independent-model matrix: 24 compiler/policy builds, 1,080 checked hashes,
  GCC 16.2 and Clang 23.1.3, four optimization levels and four LCCC policies.
- 135 native compile/run differential cases covering scaled search bounds,
  IV widening and IVSR arithmetic domains at O0/O2/O3 with default/disabled passes.
- 16 emission plus Clang integrated-assembler checks over x86-64, i686, AArch64
  and RISC-V at O0/O2/O3/Os.
- Existing AArch64 decimal32 width/indexed-fold and native decimal64 indexed-fold
  structural/differential gates.
- Trap-mode undefined-behavior instrumentation of the new C fixture with GCC and
  Clang. Ordinary GCC UBSan linking was unavailable because the cached package
  does not include its runtime; trap mode avoids that missing dependency.
- Rust formatting, Python syntax, shell syntax, diff whitespace and audit census.

The first test compile exposed a missing test-only import, and strict Clippy
exposed a useless test-only Vec allocation; both were corrected, then the checks
were rerun successfully. These were development failures, not ignored results.

## Limits

This is bounded validation, not proof of absence of all compiler defects.
Cross-target assembly acceptance is not cross-target execution. ICX is not
installed locally: no ICX package was found in the listed cache tags, and the
sandbox cannot reach Intel's package host; the authenticated vendor CI job is
responsible for that evidence. No statistically controlled runtime benchmark
establishes a speedup or globally optimal instruction selection. No such claim
is made. CI results must be associated with the exact final commit, not inherited
from an earlier merged change.
