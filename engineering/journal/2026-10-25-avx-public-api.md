# AVX public API continuation

## Native VTEST family

The public surface now includes all twelve AVX floating-point sign-bit tests:
128/256 bits, single/double precision, and ZF/CF/neither flag results. Their
GCC builtin spellings accept native vector operands. A shared typed contract
owns width, instruction and condition-code selection; both x86 backends emit
VTESTPS/PD followed by the corresponding SETcc and a zero extension. This is
not integer PTEST: non-sign payload bits must not participate. Explicit
accumulator-cache invalidation follows the scalar register write.

Public and raw names use the same feature gate and operand-count checks.
The vector-input promotion metadata distinguishes 16-byte and 32-byte inputs;
there is no vector result or result-home allocation. Header wrappers are
regenerated, not hand-edited. AVX permission is sufficient without AVX2.

The runtime regression exhausts all 65,536 pairs of eight-lane sign masks,
projects them onto shorter forms, tests public and native-vector builtins, and
checks that signaling-NaN payloads do not raise FP exceptions. The existing
feature gate additionally covers all twelve builtins in both machine modes,
including final-option AVX denial and AVX-without-AVX2 acceptance.

## Evidence and boundaries

Header regeneration, intrinsic-arity derivation, shell syntax and hosted/local
invocation-parity checks passed locally. The regression source compiled to
assembly with the authenticated GCC 16.2.0 cache oracle. Full candidate builds,
runtime comparisons and assertions belong to hosted CI; no local CI suite was
run. Do not interpret this source-level progress as completed runtime acceptance.

This increment does not replace the bundled struct-vector header model with
native GNU vector types, establish the wider per-function AVX ABI, complete
remaining SSE4 APIs, or complete AVX1/AVX2 coverage. It makes no performance or
independent expert-approval claim. Current toolchain restoration is workspace-
private; no multilib packages are extracted over the sandbox runtime.

## Native AVX2 widening family

All twelve signed/unsigned 128-to-256-bit widening conversions have typed
native lowering in both modes. Six previously missing public conversions to
64-bit lanes are supplied; six scalar-loop implementations are replaced.
Input loads are explicitly XMM-sized, independently of the YMM result, and
pointer-promotion metadata records that asymmetry. GNU builtin results carry
the correct signed native vector element type, including the PMOVZX carriers.
The private header ABI returns a pointer; it is not confused with the GNU
native-vector result contract. Public/raw/private spellings share one registry.

Native builtin operand geometry is checked before lowering. VTEST header
wrappers likewise use an explicitly declared private ABI, keeping GNU-native
vector arguments separate from the current struct-based compatibility headers.

The widening regression covers all 65,536 16-bit seed patterns, lane-distinct
payloads, all public and native builtin results, sign extension boundaries and
a 16-byte input immediately before an inaccessible page. Both standalone
VTEST and widening witnesses passed with GCC 16.2.0, GAS 2.47.20260726 and
mold 2.42.1 locally. Candidate acceptance and latest-stable Rust formatting are
tracked by hosted CI; oracle success is not candidate acceptance. No runtime
speedup has been measured or claimed.

## Current instrumentation and compiler oracle

The research lane now bootstraps GAS with Clang 23.1.4's integrated assembler
and the pinned mold 2.42.1 binary, then restores and explicitly verifies GCC
16.2.0 from the authenticated cache. Installer `--print-bin` results are driver
paths, not directories; PATH uses their parent directories explicitly.

Distribution Valgrind is replaced by the current 3.28.0 release, built from a
commit- and digest-pinned upstream mirror in a private prefix with GCC 16.2.0,
GAS 2.47 and mold 2.42.1. Binary hash receipts, source identity, both amd64 and
i386 Callgrind executables, and actual dual-mode smoke execution gate reuse.
A foreign or failed nonempty prefix is never overlaid or removed. Six isolated
provisioning contract tests pass without network or real compiler invocation.
The full source build and performance observations remain hosted evidence,
not fabricated local measurements.
