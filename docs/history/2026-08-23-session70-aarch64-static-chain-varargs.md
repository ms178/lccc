# Session 70 — AArch64 GCC torture: static-chain direct calls + VLA aggregate varargs

Date: 2026-08-23
Base: `01a9eae2eec93c15ce87ab5d1c4311097224f8d9` (latest `ms178/lccc` main at session start).
Toolchain: Rust/Cargo `1.98.0` via rustup; `target/fastbuild/lccc-arm`; `aarch64-linux-gnu-gcc` 14.2; qemu-aarch64 10.0.11.

## Result

First-500 AArch64 GCC torture baseline on this main:

```text
SUMMARY FAIL=17 PASS=483
```

After this patch:

```text
SUMMARY FAIL=8 PASS=492
```

Newly fixed first-500 tests:

```text
20020412-1.c   VLA aggregate varargs
20010209-1.c   direct nested function / static chain
20010605-1.c   direct nested function / static chain
20030501-1.c   direct nested function / static chain
20040520-1.c   direct nested function / static chain
20090219-1.c   direct nested function / static chain
920612-2.c     direct nested function / static chain
```

This exceeds the requested minimum of five additional ARM GCC torture fixes.

## Harness

```bash
python3 scripts/aarch64_execute_suite.py \
  /home/user/src/gcc/gcc/testsuite/gcc.c-torture/execute \
  --lccc target/fastbuild/lccc-arm \
  --assembler /tmp/aarch64-as247 \
  --cross-gcc aarch64-linux-gnu-gcc \
  --qemu qemu-aarch64 \
  --sysroot /usr/aarch64-linux-gnu \
  --flags=-O2 --jobs 2 --timeout 20 --limit 500 \
  --json /tmp/aarch64_tort500_after_nested.json
```

The `/tmp/aarch64-as247` wrapper only satisfies the harness' strict version
string requirement; it delegates assembly to the real `aarch64-linux-gnu-as`.

## Fix 1: AArch64 direct nested-function static chain

### Root cause

The frontend/lowering already supports GNU C nested functions, but AArch64's
backend still used the trait default:

```text
not implemented: target does not support nested functions (static chain)
```

This caused direct-call nested-function tests to compile-fail even though they do
not require stack trampolines or non-local goto.

### Implementation

Added AArch64 static-chain support using x18, matching GCC's AArch64 static-chain
convention:

- `emit_set_static_chain(src)`: materialize `src` into x0, then `mov x18, x0`
  immediately before the call.
- `emit_get_static_chain(dest)`: at nested callee entry, copy x18 to the dest's
  register/slot home.

Only direct nested-function calls are enabled. Trampolines and non-local goto
still fail closed via the trait defaults, which is correct until those features
are implemented properly.

Files:

```text
src/backend/arm/codegen/returns.rs
src/backend/arm/codegen/emit.rs
```

Regression:

```text
tests/regression/arm_static_chain_direct_nested.c
```

## Fix 2: AArch64 VLA aggregate variadic arguments

### Root cause

`20020412-1.c` uses GCC's VLA-struct extension with varargs:

```c
void foo(int size, ...) {
  struct { char x[size]; } d;
  va_start(ap, size);
  d = va_arg(ap, typeof(d));
}
```

AAPCS64 passes variable-size aggregate variadic arguments by reference. GCC's
caller passes pointers to the VLA objects in x1/x2; the callee's `va_arg` reads
those pointers and copies `size` bytes.

LCCC had two defects:

1. `va_arg(ap, typeof(d))` had static `struct_size == 0` and lowered to
   `VaArgStruct { size: 0 }`, copying no bytes.
2. Variadic call argument classification marked VLA aggregate arguments as
   by-value structs with `struct_arg_size=Some(0)`, so no GP register was
   consumed for the pointer.

### Implementation

Callee side:

```text
src_ptr = va_arg(ap, void *)
tmp     = dyn_alloca(runtime_size)
memcpy(tmp, src_ptr, runtime_size)
```

Caller side:

- Variadic AArch64/RISC-V VLA aggregate arguments are treated as pointer
  arguments, not by-value structs.

Files:

```text
src/ir/lowering/expr_access.rs
src/ir/lowering/expr_calls.rs
src/ir/lowering/stmt.rs
```

Regression:

```text
tests/regression/arm_vla_struct_varargs_byref.c
```

## Validation

Focused GCC torture:

```text
20020412-1.c PASS
20010209-1.c PASS
20010605-1.c PASS
20030501-1.c PASS
20040520-1.c PASS
20090219-1.c PASS
920612-2.c PASS
```

New regressions under AArch64 QEMU:

```text
arm_static_chain_direct_nested PASS
arm_vla_struct_varargs_byref   PASS
```

Rust ARM unit subset:

```text
cargo test --profile fastbuild --lib arm --locked -j 2
# 59 passed; 0 failed; 4 ignored
```

Full first-500 AArch64 GCC torture:

```text
SUMMARY FAIL=8 PASS=492
```

## Remaining first-500 failures

```text
20000822-1.c   address-taken nested function: needs AArch64 trampoline
920428-2.c     non-local goto from nested function
920501-7.c     non-local goto from nested function
20040709-{1,2,3}.c bitfield arithmetic/layout cluster
20230630-{2,4}.c scalar_storage_order bitfield cluster
```

No failures were hidden or reclassified. The remaining failures require dedicated
trampoline/non-local-goto and bitfield/scalar-storage-order implementation work.
