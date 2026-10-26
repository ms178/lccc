# Native SSE follow-up and hosted failure repair — 2026-10-26

## Root causes, not relaxed acceptance

The preceding main CI failed the unchanged audit census: i686 prologue planning
had crossed the 500-line threshold. Parameter-layout setup now has a focused
helper; the original baseline is unchanged and the census passes. Research
required two executable reports per family, but the GNU result/count fixtures
and the public type fixture lacked i686 entry files. Their new entry files
include the canonical sources with identical ISA flags plus `-m32`. A structural
unit test prevents losing that coverage again. No expected count or skip policy
was reduced. This repair was pushed separately before the follow-up below.

## Native result and operand contracts

Seventy-eight unique generated 128-bit floating-point/conversion helpers now
have native `__m128`/`__m128d`/`__m128i` result declarations. Their adapters use
value casts, not pointer dereferences. Scalar helpers remain scalar. Other
legacy private families retain separately named pointer adapters; this is not
an assertion that every private interface has migrated.

The canonical GNU catalog now has 240 signatures: AES/PCLMUL's seven unnamed
BDESC entries were matched by enum against GCC 16.2's own `wmmintrin.h`, not
inferred absent from a literal-name regex. Their wrappers likewise consume native
results by value. Generated C has 240 return-type contracts on x86-64, 235 on
i686, and 198 executable vector-result probes. Twenty post-AVX2 VNNI operations
remain type-only and are not counted as runtime coverage.

A generated static argument-shape table covers the same canonical signatures.
Sema rejects scalar/address arguments in vector positions, wrong-width vectors,
and vectors in scalar/address positions before pointer-backed emission. Widening
uses its 128-bit source width, not its 256-bit destination. The common path reads
existing expression-type annotations and borrows static slices: it constructs
no per-call signature vector. Lane-element compatibility and all scalar/pointer
argument conversion rules are not claimed fully enforced by this shape check.

## Focused evidence and tool provenance

Swap is active. Builds use fastbuild, locked dependencies and two jobs. GCC
16.2.0, mold 2.42.1 and GAS/objdump 2.47.20260726 were restored from the repository
cache with recorded archive hashes; the final focused comparisons below use
that current assembler pair and linker. The cached Rust 1.97 compiler is the
local bootstrap/MSRV, not a claim about latest stable acceptance. Hosted checks
use the moving stable toolchain. The cache's Clang archive is 23.1.3; it was not
used as a substitute for required Clang 23.1.4, which hosted provisioning obtains
from its hash-pinned official release.

With the rebuilt compiler and current GCC/GAS/mold, all 198 result outputs match
GCC on x86-64 at O0/O2 and i686 at O2. The public type/operator fixture, extended
with private helper result-type assertions and nested scalar/packed/widening
conversion values, passes the same modes. The diagnostic oracle agrees on 45
rejections per compiler in both x86 modes. Public-vector subscript checks retain
all nine invalid cases and native-vector runtime agreement at O0/O2. Ten header
contracts, nine GNU metadata contracts, generated synchronization, arity
synchronization (558 entries), census and CI/local invocation parity pass.
The local CI orchestrator and larger validation suites were not run.

## Manual boundary and unclaimed work

These changes alter C type/operand contracts and coverage, not instruction
encodings or hardware floating-point semantics. GCC signatures are primary
compiler-interface evidence, not an Intel/AMD SDM substitute. Accessible primary
manual mirrors still expose Intel 325462-092 and AMD combined APM 4.09, not
verified latest revisions. A GitHub-hosted locator names Intel 325462-093 at
https://cdrdv2-public.intel.com/929349/325462-093-sdm-vol-1-2abcd-3abcd-4.pdf;
that is a locator, not evidence that its contents were audited. Vendor-host
access is unavailable in this sandbox, and the latest AMD revision was not
independently retrieved. No exhaustive latest-2026 manual verification, full
remaining SSE/AVX API completion, new measured runtime speedup or independent
expert approval is claimed. Full acceptance belongs to the fresh hosted runs.
