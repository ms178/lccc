# PR1065: main 0238c77 integration and CI contract repairs

Integrated main `0238c77ac2f37fd30c61f4f3e82a4dbefc0c758d` into PR head
`efc571f4dd51bc082a38b16f07d5e126410e349d`.

## Conflict decisions

- ARM and RV64 use main's centralized ABI factories. Both explicitly leave
  `microsoft_x64` false. This preserves RV64's named/unnamed pair-alignment and
  FP-overflow rules rather than restoring the branch's older inline configs.
- Keep the Microsoft x64 positional argument classifier. Update its unit test
  to main's first-unnamed-index API (not a boolean cast).
- The register allocator passes both the real first-unnamed index and the
  calling-convention-aware x86-64 configuration. An unconditional SysV factory
  would silently discard the Microsoft ABI fix.

## Reproduced CI failures and repairs

- Check 126050918185: the explicit local-only benchmark inventory omitted the
  registered AVX stress workload. It is now listed explicitly, retaining the
  exact inventory assertion and the disjoint/complete CI partition checks.
- Check 126050918033: census failed compiling `avx_fp_private.c` without AVX.
  Local reproduction: 131 compiled, one failure, target-option mismatch in
  always-inline `clamp`. The census now reads **only machine target flags**
  from each source's benchmark registration, without raising the generic ISA
  or dropping the TU. Different compile-time workload variants remain outside
  the scope of this source-level census. Conflicting ISA registrations fail
  explicitly instead of being unioned.
- Four new contract tests cover census inclusion, absolute/relative paths,
  generic sources, negative ISA requirements, and unregistered sources.

## Local evidence

Reviewed `harness-cache-7` tools: Rust 1.99.1, GCC 16.2.0, Clang 23.1.4;
active 4 GiB swap. Compiler/linker built with fastbuild and warnings denied.
GCC is used only for non-kernel fixtures/host tooling, never kernel targets.

- Fastbuild: PASS (4m01s).
- ABI units: 14 passed, zero failures.
- Complete Rust library units: 7860 passed, zero failures, 10 ignored.
- Benchmark shard contracts: six passed.
- Census target contracts: four passed.
- Whole census: 132 TUs compiled, zero failures, 121 with rotations;
  499 rotations. Generic baseline ISA remains unchanged.
- Full Python corpus contracts: final pinned-toolchain run pending at commit.
  Earlier local attempts identified missing backports.tarfile and an accidental
  system-GCC fallback; neither is reported as a passing suite. Dependencies
  are installed outside the repository and the final run sources the pinned
  toolchain environment explicitly.

## Hosted kernel evidence and limits

For **previous head efc571f4**, job 126048432328, run 41673676787 succeeded in
all substantive steps: boot gate, full defconfig bzImage, and QEMU boot. This
is not evidence for the newly merged head; rerun its kernel workflow.

All-13 warning-family/option implementation, local full-kernel/QEMU reproduction,
zero-diagnostic proof and ICX 2026.2 differential evidence remain unestablished.
No readiness, universal correctness or maximal-efficiency claim is made here.

## Follow-up: audit-census failure after the earlier gate was repaired

On 6cc167f18 the former loop-census failure passed, but fast-contract shard b
then reached a previously hidden failure: `fns_over_300` was 233 versus a 232
ceiling. The 319-line `emit_avx_binary_256_inner` already existed on efc571f4.
Extract its register-only all-homed fast path into a boolean helper, called at
exactly the original position after pending-fold resolution and the argument
assertion. No baseline is raised and no source is excluded from the census.
The census now holds at 232. Direct tests cover noncommutative operand order,
source/destination aliasing, all three missing-home cases and GP-home rejection;
fallback rejection must not emit assembly or dirty register state.

The pinned Python corpus rerun completed: **1145 tests passed, five skipped**.
Current-head 6cc167f18 hosted authenticated ICX job 126171060755 passed the
native/local-dispatch, callback, SIMD, qualifier, incoming-memory ABI, FP-select,
volatile-reduction and bit-test matrices using ICX2026.2, GCC16.2 and Clang23.1.4.
Local authenticated ICX provisioning was attempted but Intel's HTTPS endpoint
closed TLS under this sandbox's network restrictions; no substitute was used.

Local canonical package-config boot probe: `_end=31088`, headroom 1680 bytes,
GNU ld image equality and all 24 ISA scans passed. SHA256 of setup.bin:
`ea8d15014f3ea5edd988963fb49b2d2dd4880656e631d727ff0de46c286bed7d`.
The full local defconfig build uses the **6cc167f18-era compiler binary**, not
a future refactored binary, and is not final-head evidence. Its real unsupported
option warnings remain visible; zero diagnostics is still NOT established.

After extraction: full warnings-denied fastbuild-profile library test run:
**7862 passed, zero failed, 10 ignored**. Formatting and the unmodified audit
ratchet pass. Kernel compilation was paused for Rust compilation and resumed
by the test process's exit trap; heavy builds were not run concurrently.
