# 2026-08-19 (session 19) — critical review of the Grok cast.rs audit

**Base:** `origin/main` @ `6af22ea1` (Merge PR #132 — session 18 div/alu work)
**Snapshot:** `/home/user/ms178-1.patch`

## Verdict on the Grok report — finding by finding (empirically checked)

| # | Finding | Verdict |
|---|---|---|
| 1 | Ptr treated as signed in int→float / F128 paths | **CONFIRMED and fixed** — `IrType::Ptr` is neither `is_signed()` nor `is_unsigned()` nor `is_integer()`, so `Ptr → F64` classified as `SignedToFloat { from_ty: Ptr }` and `Ptr → F128` as `SignedToF128 { Ptr }`. Now `is_unsigned_src = from_ty.is_unsigned() || from_ty == IrType::Ptr` and Ptr normalizes to U64/U32. **But the real manifestation was elsewhere** (see below). |
| 2 | `to_u64` for Ptr ignores ILP32 | **CONFIRMED and fixed** — `to_u64 = to_ty == U64 || to_ty == Ptr` was wrong on 32-bit (Ptr is 32-bit there); now `Ptr && !target_is_32bit()`. |
| 3 | `f128_const_halves` panics on malformed data | **Agree (latent)** — fixed with `?`/length guard. The `LongDouble(f64, [u8;16])` is always 16 bytes, so it never fired in practice, but the robust version is strictly better. |
| 4 | Void/zero-size casts | **Agree (defensive)** — added a Void/zero-size guard. |
| 5 | F128 native path incomplete for Ptr | **CONFIRMED and fixed** — Ptr→F128 and F128→Ptr now normalize to the unsigned pointer-width integer. |
| 6 | Same-size signed↔unsigned comments | Minor; the docs were already essentially correct (RISC-V sext.w documented). No change needed. |
| gaps | cost class / extension-need / libcall-map helpers | **Rejected as dead code** — the F128 libcall names already live in the ARM/RISC-V backends (`__addtf3` etc. in intrinsics.rs/float_ops.rs); adding duplicate maps to cast.rs would create maintenance surface without a consumer. The direct-32-bit-cvt and BMI2 ideas are backend work, not classification work. |

## The actual miscompile (beyond cast.rs — traced to two further layers)

The `Ptr → float` signedness bug turned out to have **three** contributing layers,
all now fixed. `(double)p` (accepted as an extension; GCC errors) printed 0.0:

1. **Frontend** (`src/ir/lowering/expr_access.rs`): `lower_cast` dropped the cast
   entirely because `(from_ty == Ptr && to_ty.size() == ptr_size)` matched
   `Ptr → F64` (both 8 bytes). The cast vanished and the ABI return path
   bitcast the pointer (`movq %reg, %xmm0`). Fixed by requiring
   `to_ty.is_integer()` in that clause.
2. **MachInst gate** (`x86/codegen/emit.rs`): `try_lower_machinst`'s Cast gate
   used a coarse size/sign classification that did NOT exclude float casts
   (unlike `isel::can_lower`, which does), so a same-size `Ptr → F64` was
   lowered to a size-move bitcast. Fixed by excluding float casts.
3. **x86 register-direct fast path** (`cast_ops.rs`): `from_ty != IrType::U64`
   let `Ptr` through to a plain signed `cvtsi2sdq`. Fixed by also excluding
   `Ptr`, so it takes the U64 shift+round dance.

After all three, `(double)p` equals `(double)(uintptr_t)p` for low, high-bit
(`0xffffffff80000000`), and heap addresses — bit-exact.

## Shipped

- `src/backend/cast.rs`: Ptr unsigned normalization (int→float, int→F128,
  F128→int), `to_u64` respecting pointer width, panic-free `f128_const_halves`,
  Void/zero-size guard.
- `src/ir/lowering/expr_access.rs`: Ptr→float no longer elided.
- `src/backend/x86/codegen/emit.rs`: MachInst float-cast exclusion.
- `src/backend/x86/codegen/cast_ops.rs`: register-direct int→float excludes Ptr.
- `tests/regression/check_ptr_to_float_cast.sh`.

## Validation

- 922 unit, 50/50 correctness, **346**+6 regression, **600-case** i686 fuzz,
  boot `-Os` `.text` unchanged at 33445 (only Ptr-float paths affected, which
  the kernel does not use).
- The U64→F64 sequence (testq/js/cvtsi2sdq+addsd) matches the godbolt oracle
  (GCC 16.2 / Clang 22.1 `-m32 -O2` shape for unsigned conversion).
