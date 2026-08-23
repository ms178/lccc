# Session 72 — Alternative patch audit: accepted rsp/push peephole, kept safer VLA-varargs model

Date: 2026-08-23
Base: `655569d6342950e13e9073485e3401c5b140772a` (latest `ms178/lccc` main at session start).
Build: `scripts/build_lccc_fast.sh` with Rust/Cargo `1.98.0`.

## User-provided alternative

The proposed patch contained two major ideas:

1. x86 `eliminate_redundant_leaq` must treat stack-pointer updates as writes,
   because cached `leaq X(%rsp), %rax` addresses become stale across `push` /
   `pop` / `leave` / `enter`.
2. VLA-containing struct varargs must be passed by reference and `typeof(d)`
   must resolve to the runtime size for `va_arg(ap, typeof(d))`.

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

### Kept current model: VLA struct varargs

The alternative's diagnosis is correct and already represented in current main,
but the current implementation is more conservative semantically:

- Callee `lower_va_arg_struct` reads the by-reference pointer, allocates a fresh
  runtime-sized temporary, and copies `runtime_size` bytes before returning the
  expression value. This preserves by-value `va_arg` semantics for all expression
  consumers, not only assignment forms.
- Caller classification is gated on AArch64/RISC-V **and variadic calls** before
  treating dynamic aggregate args as plain pointers. A global
  `dynamic_struct_value_size(a) -> None` rule is too broad and risks changing
  non-variadic ABI classification.
- Runtime `typeof(identifier)` size resolution uses the active local's
  `LocalInfo.vla_size`. Registering local variable names in the typedef-size map
  is workable, but it mixes local identifiers into a typedef-oriented namespace
  and can collide with shadowing in ways the locals table already models.

Validation retained:

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

## Conclusion

The alternative is better only for the x86 `%rsp` peephole hole, which is now
adopted. Its VLA-varargs direction is correct, but the current implementation is
safer and more general because it preserves by-value expression semantics and is
properly target/variadic gated.
