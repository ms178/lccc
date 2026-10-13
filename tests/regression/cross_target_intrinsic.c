/* AUD-SILENT-ZERO-1 gate input.
 *
 * `__builtin_ia32_aesenc128` is an x86-only intrinsic: AArch64 and RISC-V have
 * no instruction for it, and their backends cannot compute it.  Before the
 * fix those backends "zeroed dest if present" -- a 16-byte store of zeros, or
 * NOTHING AT ALL when the destination was register-allocated -- so an x86
 * intrinsic compiled for another target silently produced a wrong value with
 * rc=0 and no diagnostic anywhere in the pipeline.
 *
 * The two functions below pin both halves of the contract: the x86-only one
 * must be unable to hand back a value (trap), and the portable ones must keep
 * lowering to real instructions (no collateral trapping).
 */
typedef long long v2di __attribute__((vector_size(16)));

v2di x86_only_enc(v2di a, v2di b) { return __builtin_ia32_aesenc128(a, b); }

double portable_sqrt(double x) { return __builtin_sqrt(x); }

double portable_fabs(double x) { return __builtin_fabs(x); }
