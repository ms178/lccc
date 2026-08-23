# Session 67 — ARM regression failures: host-header dependency removed

Date: 2026-08-23
Base: `b7ce7d8cbc57c0bbe0db74e582a70fa36168c76c` (latest `ms178/lccc` main at session start).
Toolchain: Rust/Cargo `1.98.0` via rustup; `scripts/build_lccc_fast.sh`.

## Root cause

The pre-existing ARM regression failures in this sandbox were not caused by an
AArch64 codegen defect. They were caused by ARM structural regression sources
including host libc headers (`<stdint.h>`, `<stdio.h>`) while being compiled with
`lccc-arm` in a cross-target structural mode.

`lccc-arm` preprocesses using the host include search path. On this Debian image,
`/usr/include/stdint.h` and `/usr/include/stdio.h` include multiarch-private
headers such as `bits/libc-header-start.h` and `bits/types.h`. Those are not in
the plain `/usr/include` path used by the structural ARM regression compile, so
AArch64 tests failed before reaching the compiler backend:

```text
/usr/include/stdint.h:26:10: error: bits/libc-header-start.h: No such file or directory
/usr/include/stdio.h:28:10: error: bits/libc-header-start.h: No such file or directory
```

These ARM regressions are intended to test generated AArch64 assembly shape and
independent assembler acceptance, not libc ABI integration. Depending on the
host's exact glibc multiarch include layout made them non-hermetic and masked the
actual backend checks.

## Fix

Made the affected ARM regression sources freestanding:

- `tests/regression/arm_csinc_select.c`
  - Replaced `<stdint.h>` with local fixed-width typedefs used by the test.
  - Replaced `<stdio.h>` with `extern int printf(const char *, ...);`.
- `tests/regression/arm_matmul_column_stride.c`
  - Replaced `<stdio.h>` with a local `printf` declaration.
- `tests/regression/arm_vec_load_offset.c`
  - Replaced `<stdio.h>` with a local `printf` declaration.

This is the best technical fix for these regressions: it preserves the exact C
semantics needed by the tests, keeps them runnable in the native regression
harness, and makes the AArch64 structural path independent of accidental host or
cross sysroot header availability.

## Validation

Installed an independent AArch64 GNU assembler/cross GCC in the sandbox for the
assembler-acceptance step:

```bash
sudo apt-get install -y --no-install-recommends \
  gcc-aarch64-linux-gnu binutils-aarch64-linux-gnu
```

### AArch64 compile-only sweep

All ARM regression C files compile to AArch64 assembly with `lccc-arm -O2 -S`:

```text
PASS arm_array_addressing.c
PASS arm_csinc_select.c
PASS arm_fp_homed_int_binop.c
PASS arm_gep_regbase_acc_cache.c
PASS arm_int_madd_msub_fusion.c
PASS arm_matmul_column_stride.c
PASS arm_static_local.c
PASS arm_vec_load_offset.c
```

### AArch64 structural scripts

```text
PASS tests/regression/check_arm_csinc_select.sh
PASS tests/regression/check_arm_direct_select.sh
PASS tests/regression/check_arm_fp_stack_reload.sh
```

`check_arm_csinc_select.sh` also assembles the generated AArch64 assembly with
`aarch64-linux-gnu-gcc -c`, so condition-code/register encoding mistakes are
covered independently of LCCC's own emitter.

### Native regression semantics for edited sources

The edited sources still compile and run natively under `target/fastbuild/lccc -O2`:

```text
arm_csinc_select.c           -> b57b2d2d6ce2db98 9
arm_matmul_column_stride.c   -> expected 4x4 matrix output
arm_vec_load_offset.c        -> 1578496
```

### Rust unit subset

```text
cargo test --profile fastbuild --lib arm --locked -j 2
# 59 passed; 0 failed; 4 ignored
```

## Follow-up

If future ARM tests truly need libc headers, add an explicit target sysroot and
include model to the driver/harness. Structural backend regressions should remain
freestanding by default: it produces clearer failures and avoids hiding backend
regressions behind host distribution header layout.
