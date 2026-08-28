# TASK-LK-29 — statement-expression + inline-asm type inference

IDs: LK-29 (kernel bring-up B2) · Priority: **P1** · Base: f657de55
· Blocks ALL of `net/*` in the kernel build.

## Objective

lccc's type inference for `xchg`/`instrument_atomic_read_write` macro
expansions (in `include/net/sock.h` etc.) produces `int *` where GCC
computes `struct dst_entry *`. The interim mitigation (non-fatal
`IncompatiblePointerTypes` warning for statement-expr yields, landed in the
kernel-bringup session) is visible but wrong; GCC computes the correct type
through `typeof` of `*ptr` where ptr is `&field`, and through the yield
type of `({ ...; __ret; })`.

1. Fix `__typeof__` inference through deref/address-of chains and
   statement-expression yields.
2. Re-register `IncompatiblePointerTypes` in `from_flag_name` so
   `-Werror=incompatible-pointer-types` promotes to error (GCC-strict),
   and remove the interim warning-only path.

## Files

`src/frontend/sema/` (typeof + statement-expr types),
`src/common/error.rs` (flag registration).

## Acceptance

- Kernel `net/core/*.o` compiles with no incompatible-pointer diagnostics
  other than genuine ones (verify against GCC's output on the same TU).
- `tests/regression/stmt_expr_asm_typeof.c` and the sock.h-shaped macro
  probes type identically to GCC.
- `-Werror=incompatible-pointer-types` promotes (GCC parity), while
  explicit casts stay suppressed (already landed).

## Validation battery

`cargo test --lib` · sema negative corpus (`check_sema_constraints.sh`) ·
kernel `net/` object sample compile · GCC differential on the repro TUs.

## Do not

- Do not keep the warning-only escape hatch once the inference is correct
  (it hides real type errors in user code).
- Do not special-case the `xchg` macro textually — fix the typeof/yield
  semantics.
