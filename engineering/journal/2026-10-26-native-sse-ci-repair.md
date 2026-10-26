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

## Scalar conversion and permission defects found during follow-up

A live wrong-code witness, `__builtin_ia32_crc32si(0, 1.0)`, returned zero instead
of GCC's 3712330424: builtin dispatch bypassed normal call parameter conversion
and read the double's bits as an integer. Generated static scalar-parameter
contracts now drive numeric conversions in ordinary x86 builtin dispatch;
prototype CTypes supply the public/private helper cases. Generic SIMD dispatch
uses the same conversion entry point. Vector/address operands avoid redundant
type inference and signature-map probes when canonical metadata describes them.
Constant floating immediates fold during conversion even at O0; byte-count
validation also converts to the builtin's C `int` before checking range/units.

The new executable oracle covers volatile floating inputs to raw/public CRC,
BZHI, public/private scalar-to-vector conversion, and floating shift/shuffle
immediates. It matches current GCC/GAS/mold on both x86 modes at O0 and O2.
It is a mandatory zero-skip dual-mode hosted research family, not just a local
witness. It also exposed a pre-existing i686 BMI2 permission mismatch: 32-bit
BZHI had a defined lowering but the global x86-64-only optimization grant was
incorrectly its sole permission. Builtin/header permission is now separate from
optimizer permission, with last-option-order unit cases; unsupported i686 BMI2
optimization is not enabled.

Fresh hosted frontend CI found that early shape validation changed the existing
widening diagnostic wording. The unified message now states the required vector
bit width, preserving the established 128-bit source contract; no rejection was
weakened. The targeted ISA feature gate now passes in full against GCC 16.2.

That gate had intentionally asserted live AUD-CPU-4 debt: YMM floating/integer
intrinsics emitted instructions even when AVX/AVX2 was denied. Explicit
permissions now cover the packed YMM floating arithmetic/load/store/raw mask
families (AVX) and packed integer add/sub/multiply families (AVX2), on both x86
ABIs. The former debt witnesses are actual rejection/native-emission assertions
with GCC verdict comparisons. AVX-only floating operations remain accepted
without AVX2. This closes those witnesses, not an unmeasured assertion that
every remaining opcode permission has been audited.

Final focused checks retain 198-vector output agreement after scalar lowering,
45 negative cases per compiler, unchanged audit budgets and invocation parity.
The local full CI suite was not run. The hardware conversion witness establishes
a wrong-code repair, not a benchmark speedup. Latest SDM verification and the
other remaining scope boundaries above are still not claimed complete.

## Packed scalar shift counts

The native SSE2/AVX2 emitters treated nonconstant scalar counts as immediate
zero; encoding large constants modulo imm8 also made counts such as 256 act as
identity operations. The new paired native packed-count fixture fails against
GCC before the repair. Element shifts now use MOVD plus the register-count
instruction for runtime C-int counts, and an equivalent saturated immediate
(255) for negative or >255 constants. Byte-lane shifts retain their separate
validated bit-count conversion contract. x86-64 scratch-bank claims and pending
stores are handled before overwriting the count register.

The fixture checks 384 vector-result hashes over all eight 16/32/64-bit logical
and arithmetic shift operations at both vector widths, including 21 runtime
counts and large/negative constants. It matches GCC 16.2.0 on i686/x86-64 at
O0/O2. The hosted Research workflow requires both ABI companions without skips.
Scalar packed shifts now require SSE2 (128-bit) or AVX2 (256-bit); the focused
ISA gate passes its GCC-compared positive/negative and last-option cases on
both ABIs. Metadata's ten contracts, census and CI invocation parity pass.
This repairs semantics and native emission; it is not a measured speedup or an
exhaustive API/signature/manual audit.
