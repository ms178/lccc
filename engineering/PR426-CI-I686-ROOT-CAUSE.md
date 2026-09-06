# PR #426 CI: i686 regression root cause

## Failure observed

GitHub Actions run `34036123710`, job `101494400705`, published two regression
annotations:

- `segment_fill_copy_alias`: `run failed (rc=-11)`
- `i686_fused_mul_add_operand_order`: `run failed (rc=-11)`

The public job API exposed the test names and annotations, but not the raw log.
The failures were reproducible without guessing at compiler output.

## Reproduction

The two tests use `-m32`.  A host can have the i386 dynamic loader
`/lib/ld-linux.so.2` while lacking the 32-bit development/startup files under
`/usr/lib32`.  That is an incomplete i686 toolchain: LCCC's link step still
produces a dynamically linked ELF32 image, but its libc startup image is
malformed and the test dies with `SIGSEGV` before testing the C function.

On the validation host, the exact PR compiler and both tests behaved as follows:

1. Keep the i386 runtime loader and libraries installed, but temporarily hide
   `/usr/lib32` (the state before `gcc-multilib`/`libc6-dev-i386` was installed).
   Both tests compiled and then exited `139`.
2. Restore `/usr/lib32`, containing the 32-bit startup/development objects.
   Both tests compiled and returned `0` repeatedly.

This distinguishes a host-toolchain failure from a codegen failure: the same
compiler and sources pass when the required i686 link contract is present.
The original runner's loader-only host check could not detect this partial
installation, which is why CI surfaced the two SIGSEGVs as compiler regressions.

## Fix

The test job now enables the i386 architecture and installs
`gcc-multilib` plus `libc6-dev-i386` before the compiler build.  The recorded
CI environment also includes both package versions.  The existing two i686
regressions remain in the corpus and therefore continue to execute as real
codegen/runtime coverage instead of being hidden or skipped.

## Independently validated Agent Z improvement

The proposed MachInst narrow-slot promotion was audited separately from the CI
fix.  It is adopted because it has a sound width argument and executable
coverage: an S64 memory substitution cannot read a 4-byte spill slot, so the
vreg is promoted to a window register and reloaded at S32, which zero-extends
and defines the full GPR.  The classifier and allocator receive the same
small-slot set, preventing a resolver/allocator disagreement.

The added structural gate and `machinst_window_alloc_wide_copy.c` regression
verify that the representative window no longer emits `MI-FALLBACK`.  The
unsafe Agent Z CI changes that switched the project gate to release builds or
reduced the test target set were not adopted; this tree keeps the mandatory
fastbuild/all-targets policy.
