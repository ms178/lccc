# Microsoft x64 ABI: implementation and validation status

This is a **partial implementation**, not completion of the kernel/QEMU task.
The non-default ABI is represented on function types and call instructions,
including typedefs, function pointers, composite types and indirect calls.

## Implemented scalar path

- Four shared argument positions: RCX/RDX/R8/R9 or XMM0–3 by position.
- Caller-owned 32-byte home area, overflow arguments and stack alignment.
- Floating-point return and variadic *callers*, including GP duplicates of FP
  register arguments. Microsoft variadic function definitions remain rejected.
- Preservation of RDI/RSI and, when SSE is enabled, XMM6–15, with synthesized
  DWARF save records. The current save policy is conservative; no performance
  superiority is claimed.
- Optional written conventions in declaration ASTs distinguish absence (which
  inherits an established ABI) from explicit `sysv_abi` (which can conflict).
- Specifier attributes propagate to comma-separated declarators. Scalar argument
  checking applies ordinary array/function decay.
- SysV-only typed-call, sibling-call and register-allocation optimizations do not
  reinterpret foreign-ABI calls as SysV calls. The general ABI-aware call path
  handles those sites.

## Reproducible evidence (2026-11-01)

`bash tests/regression/check_ms_abi.sh` uses GCC **only for non-kernel oracle
objects**, and LCCC for the implementation under test. Passed with GCC 16.2.0
at `-O0`, `-O2`, `-Os`, and `-O2 -fstack-clash-protection`:

- Both GCC-to-LCCC and LCCC-to-GCC scalar interoperability.
- Mixed positional GP/FP arguments, FP returns and six-argument overflow.
- Direct, volatile indirect, callback parameter, array and struct-field calls.
- Assembly witnesses for home-slot writes, stack alignment and preservation of
  RDI/RSI/XMM6–15 across a Microsoft-to-SysV clobbering call.
- Mixed Microsoft/SysV integer calls in the same function (a probe that exposed
  and now guards the incorrectly admitted SysV `CallTyped` path).
- Variadic FP duplication, string/function decay, declaration inheritance and
  rejection of conflicting explicit conventions.
- 32-KiB frames, page probing, actual libgcc unwinding through Microsoft frames,
  and `readelf` save-record checks for all twelve extra nonvolatile registers.
- Integer-only `-mno-sse -mno-sse2` emission without vector registers.

The tested compiler executable SHA256 is
`883b8b34d4b13e7685de9426729bca9827c76bd8aa99c485e885895200e1a87c`.
The subsequent source-only edits add comments, use equivalent Rust field
shorthand and add a classifier unit test; they do not constitute a new tested
compiler binary. Hosted CI must validate the submitted source revision.

The actual patched CachyMod 6.18.58 default configuration was retained (SHA256
`3d06a10a7773a3ca9fff4f425b8adff0cf373c656b849a7202744924034d1e5d`).
The latest boot-only run passed at **31,024 / 32,768 bytes**, with 24 ISA-scanned
objects and a byte-identical GNU-linker parity check. No GCC target compilation
was used. The full build subsequently compiled `arch/x86/kernel/cpu/common.o`,
the translation unit previously stopped by fourteen `ms_abi` diagnostics.

## Remaining acceptance criteria

- Aggregate/by-reference, wide scalar, complex/vector ABI rules and Microsoft
  variadic definitions require implementation; the current scalar guard rejects
  unsupported signatures rather than pretending to implement them.
- Parser attribute placement and inherited conventions need further adversarial
  coverage, including post-asm attributes and typedef-derived declarations.
- Further kernel compiler/linker defects and unsupported-option warnings must
  be resolved. The full build is still in progress, not a successful kernel.
- A complete kernel link, QEMU boot, latest-main integration and green hosted
  CI remain required before production readiness. No full-kernel success or
  performance improvement is asserted by the scalar tests.
