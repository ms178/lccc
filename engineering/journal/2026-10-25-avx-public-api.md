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
