# Session 72 — Alternative patch audit: accepted rsp/push peephole and broadened VLA-varargs by-reference ABI

Date: 2026-08-23
Base: `655569d6342950e13e9073485e3401c5b140772a` (latest `ms178/lccc` main at session start).
Build: `scripts/build_lccc_fast.sh` with Rust/Cargo `1.98.0`.

## User-provided alternative

The proposed patch contained two major ideas:

1. x86 `eliminate_redundant_leaq` must treat stack-pointer updates as writes,
   because cached `leaq X(%rsp), %rax` addresses become stale across `push` /
   `pop` / `leave` / `enter`.
2. VLA-containing struct varargs must be passed by reference and
   `va_arg(ap, typeof(d))` must fetch a pointer. The prior PR audit correctly
   fixed AArch64, but missed that x86 `20020412-1.c` still failed without the
   broader by-reference rule.

## Decision

### Accepted and merged: x86 `%rsp` peephole soundness

This is a real root-cause fix and is now included.

`written_families()` now reports:

- `push*` writes `%rsp`;
- `pop*` writes the popped register and `%rsp`;
- `leave` / `enter` write `%rsp` and `%rbp`.

This prevents deleting a second, textually identical `leaq X(%rsp), %rax` after
stack adjustment. The concrete failure mode is an sret call with a by-value
struct argument: the return-buffer address and argument address can share the
same textual displacement while naming different stack locations.

Regression added:

```text
tests/regression/push_rsp_leaq_dedup.c
```

Validation:

```text
scripts/x86_gcc_torture_slice.sh 20040709-1.c 20040709-2.c 20040709-3.c
# 3 pass / 0 fail

push_rsp_leaq_dedup.c at -O0/-O1/-O2
# all PASS
```

### Accepted after re-audit: VLA struct varargs by-reference, all supported ABIs

The user counter-analysis is correct for LCCC's supported C surface:

- A function prototype cannot name a variable-size struct parameter type in the
  ordinary fixed-parameter ABI path, so the practical reachable case is the
  variadic path.
- `va_arg(...)` yields a non-lvalue expression. Valid C consumers can copy from
  it or pass it onward; they cannot assign into the `va_arg` result object.
- LCCC internally represents aggregate rvalues as pointers already. Returning
  the by-reference pointer from `va_arg` is therefore semantically consistent for
  valid programs and avoids the extra callee-side temporary+memcpy.

The implementation now does this:

Callee side (`lower_va_arg_struct`):

```text
if requested type has runtime VLA aggregate size:
    return va_arg(ap, void *)   // pointer to referenced aggregate object
```

Caller side (`lower_call_arguments`):

```text
if call is variadic and argument is a dynamic-size aggregate:
    struct_arg_size = None      // classify/pass as ordinary pointer
```

The caller-side rule remains explicitly `pre_call_variadic`, so non-variadic ABI
classification is not accidentally changed.

Regression added:

```text
tests/regression/vla_struct_varargs.c
```

Validation:

```text
scripts/x86_gcc_torture_slice.sh 20020412-1.c
# PASS

vla_struct_varargs.c at -O0/-O1/-O2
# all PASS
```

The existing AArch64 regression remains green:

```text
20020412-1.c PASS
arm_vla_struct_varargs_byref PASS
```

## AArch64 revalidation

The seven AArch64 torture fixes from current main remain green:

```text
20020412-1.c PASS
20010209-1.c PASS
20010605-1.c PASS
20030501-1.c PASS
20040520-1.c PASS
20090219-1.c PASS
920612-2.c PASS
```

Full first-500 AArch64 slice remains:

```text
SUMMARY FAIL=8 PASS=492
```

## Conclusion

The alternative provided two valid insights. The x86 `%rsp` peephole fix is
adopted directly. The VLA-varargs insight is also adopted, but with the safer
variadic-only caller gating and with the existing `LocalInfo.vla_size` lookup for
`typeof(identifier)` retained instead of mixing local names into the typedef-size
namespace.
