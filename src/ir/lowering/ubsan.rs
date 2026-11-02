//! Trap-mode UndefinedBehaviorSanitizer (`-fsanitize=undefined` +
//! `-fsanitize-trap=...` / `-fsanitize-undefined-trap-on-error`, `MB-07c`).
//!
//! All checks emit inline SSA branch guards to cold `Terminator::Unreachable`
//! blocks (`ud2` on x86-64/i686, `brk #0` on AArch64, `unimp` on RISC-V) with
//! zero runtime library calls (`__ubsan_handle_*` is never referenced), making
//! trap-mode UBSan safe in freestanding / Linux kernel builds as well as
//! hosted userland.

use super::lower::Lowerer;
use crate::common::types::{AddressSpace, CType, IrType, target_int_ir_type};
use crate::frontend::parser::ast::Expr;
use crate::ir::reexports::{IrBinOp, IrCmpOp, IrConst, Operand, Terminator, Value};

/// Bitmask of UBSan checks supported by `lccc`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UbsanFlags(pub u32);

impl UbsanFlags {
    pub const NONE: Self = Self(0);
    pub const SIGNED_OVERFLOW: Self = Self(1 << 0);
    pub const INT_DIV_ZERO: Self = Self(1 << 1);
    pub const SHIFT_BASE: Self = Self(1 << 2);
    pub const SHIFT_EXPONENT: Self = Self(1 << 3);
    pub const BOUNDS: Self = Self(1 << 4);
    pub const BOUNDS_STRICT: Self = Self(1 << 5);
    pub const NULL: Self = Self(1 << 6);
    pub const BOOL: Self = Self(1 << 7);
    pub const ENUM: Self = Self(1 << 8);
    pub const VLA_BOUND: Self = Self(1 << 9);
    pub const UNREACHABLE: Self = Self(1 << 10);
    pub const RETURN: Self = Self(1 << 11);
    pub const FLOAT_DIV_ZERO: Self = Self(1 << 12);

    pub const SHIFT_GROUP: Self = Self(Self::SHIFT_BASE.0 | Self::SHIFT_EXPONENT.0);

    /// Exact GCC 16.2 `-fsanitize=undefined` group (note: `float-divide-by-zero`
    /// and `bounds-strict` are NOT included in `undefined`, matching GCC 16.2).
    pub const UNDEFINED_GROUP: Self = Self(
        Self::SIGNED_OVERFLOW.0
            | Self::INT_DIV_ZERO.0
            | Self::SHIFT_BASE.0
            | Self::SHIFT_EXPONENT.0
            | Self::BOUNDS.0
            | Self::NULL.0
            | Self::BOOL.0
            | Self::ENUM.0
            | Self::VLA_BOUND.0
            | Self::UNREACHABLE.0
            | Self::RETURN.0,
    );

    pub const ALL_KNOWN: Self =
        Self(Self::UNDEFINED_GROUP.0 | Self::BOUNDS_STRICT.0 | Self::FLOAT_DIV_ZERO.0);

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[inline]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    #[inline]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Parse one comma-separated sanitizer list item.
    /// `allow_all` is true for `-fno-sanitize=`, `-fsanitize-trap=`, and
    /// `-fno-sanitize-trap=` (GCC 16.2 rejects `-fsanitize=all`).
    /// When `lenient_unsupported` is true (`-fno-sanitize=` and `-[no-]fsanitize-trap=`),
    /// known non-UBSan or runtime-only GCC sanitizer names map to `NONE` instead of failing.
    pub fn parse_item_mode(
        item: &str,
        allow_all: bool,
        lenient_unsupported: bool,
    ) -> Result<Self, String> {
        match item {
            "none" => Ok(Self::NONE),
            "all" if allow_all => Ok(Self::ALL_KNOWN),
            "all" => Err("'-fsanitize=all' option is not valid".to_string()),
            "undefined" => Ok(Self::UNDEFINED_GROUP),
            "shift" => Ok(Self::SHIFT_GROUP),
            "shift-base" => Ok(Self::SHIFT_BASE),
            "shift-exponent" => Ok(Self::SHIFT_EXPONENT),
            "integer-divide-by-zero" => Ok(Self::INT_DIV_ZERO),
            "signed-integer-overflow" => Ok(Self::SIGNED_OVERFLOW),
            "bounds" => Ok(Self::BOUNDS),
            "bounds-strict" => Ok(Self(Self::BOUNDS.0 | Self::BOUNDS_STRICT.0)),
            "null" => Ok(Self::NULL),
            "bool" => Ok(Self::BOOL),
            "enum" => Ok(Self::ENUM),
            "vla-bound" => Ok(Self::VLA_BOUND),
            "unreachable" => Ok(Self::UNREACHABLE),
            "return" => Ok(Self::RETURN),
            "float-divide-by-zero" => Ok(Self::FLOAT_DIV_ZERO),
            "address"
            | "kernel-address"
            | "hwaddress"
            | "kernel-hwaddress"
            | "thread"
            | "memory"
            | "kernel-memory"
            | "leak"
            | "shadow-call-stack"
            | "cfi"
            | "kcfi"
            | "alignment"
            | "object-size"
            | "pointer-compare"
            | "pointer-subtract"
            | "nonnull-attribute"
            | "returns-nonnull-attribute"
            | "float-cast-overflow"
            | "builtin"
            | "vptr"
            | "function" => {
                if lenient_unsupported {
                    Ok(Self::NONE)
                } else {
                    Err(format!(
                        "-fsanitize={item} is not supported (requires sanitizer runtime library)"
                    ))
                }
            }
            other => Err(format!(
                "unrecognized argument to '-fsanitize=' option: '{other}'"
            )),
        }
    }

    pub fn parse_item(item: &str, allow_all: bool) -> Result<Self, String> {
        Self::parse_item_mode(item, allow_all, false)
    }

    /// Parse a comma-separated sanitizer list (`undefined,float-divide-by-zero`).
    pub fn parse_list(list: &str, allow_all: bool) -> Result<Self, String> {
        Self::parse_list_mode(list, allow_all, false)
    }

    pub fn parse_list_mode(
        list: &str,
        allow_all: bool,
        lenient_unsupported: bool,
    ) -> Result<Self, String> {
        if list.is_empty() {
            return Err("empty argument to '-fsanitize=' option".to_string());
        }
        let mut acc = Self::NONE;
        for part in list.split(',') {
            acc = acc.union(Self::parse_item_mode(
                part.trim(),
                allow_all,
                lenient_unsupported,
            )?);
        }
        Ok(acc)
    }

    /// Leniently parse a `no_sanitize("...")` attribute list (ignoring unknown
    /// sanitizer names such as `"address"` or `"thread"` the way GCC/Clang do).
    pub fn parse_no_sanitize_attr(list: &str) -> Self {
        let mut acc = Self::NONE;
        for part in list.split(',') {
            if let Ok(flags) = Self::parse_item(part.trim(), true) {
                acc = acc.union(flags);
            }
        }
        acc
    }
}

impl Lowerer {
    /// Check whether `flag` is enabled in the current function's effective
    /// trap-mode UBSan mask (respecting `no_sanitize` attributes and skipping
    /// unevaluated contexts).
    #[inline]
    pub(super) fn ubsan_active(&self, flag: UbsanFlags) -> bool {
        self.func_state.is_some()
            && self
                .ubsan_trap
                .difference(self.func_no_sanitize)
                .contains(flag)
    }

    /// Whether signed integer `+`, `-`, `*`, unary `-`, `++`, `--` must trap on overflow.
    #[inline]
    pub(super) fn signed_overflow_traps(&self) -> bool {
        self.func_state.is_some()
            && (self.trapv
                || (!self.wrap_signed_overflow && self.ubsan_active(UbsanFlags::SIGNED_OVERFLOW)))
    }

    /// Emit a cold hardware trap branch (`if (bad != 0) unreachable;`).
    pub(super) fn emit_ubsan_trap_if(&mut self, bad_cond: Value) {
        let ok_label = self.fresh_label();
        let trap_label = self.fresh_label();
        self.terminate(Terminator::CondBranch {
            cond: Operand::Value(bad_cond),
            true_label: trap_label,
            false_label: ok_label,
        });
        self.start_block(trap_label);
        self.terminate(Terminator::Unreachable);
        self.start_block(ok_label);
    }

    /// Check integer division/modulo for `rhs == 0` (`integer-divide-by-zero`)
    /// and signed `lhs == MIN && rhs == -1` (`signed-integer-overflow`).
    pub(super) fn emit_ubsan_int_div_checks(
        &mut self,
        lhs: Operand,
        rhs: Operand,
        common_ty: IrType,
        op_ty: IrType,
        is_unsigned: bool,
    ) {
        if self.ubsan_active(UbsanFlags::INT_DIV_ZERO) {
            let rhs_narrow = if common_ty != op_ty {
                Operand::Value(self.emit_cast_val(rhs, op_ty, common_ty))
            } else {
                rhs
            };
            let is_zero = self.emit_cmp_val(
                IrCmpOp::Eq,
                rhs_narrow,
                Operand::Const(IrConst::zero(common_ty)),
                common_ty,
            );
            self.emit_ubsan_trap_if(is_zero);
        }

        if !is_unsigned
            && common_ty.is_signed()
            && !self.wrap_signed_overflow
            && self.ubsan_active(UbsanFlags::SIGNED_OVERFLOW)
        {
            let (min_const, neg_one) = match common_ty {
                IrType::I32 => (
                    Operand::Const(IrConst::I32(i32::MIN)),
                    Operand::Const(IrConst::I32(-1)),
                ),
                IrType::I64 => (
                    Operand::Const(IrConst::I64(i64::MIN)),
                    Operand::Const(IrConst::I64(-1)),
                ),
                IrType::I128 => (
                    Operand::Const(IrConst::I128(i128::MIN)),
                    Operand::Const(IrConst::I128(-1)),
                ),
                _ => return,
            };
            let lhs_c = if common_ty != op_ty {
                Operand::Value(self.emit_cast_val(lhs, op_ty, common_ty))
            } else {
                lhs
            };
            let rhs_c = if common_ty != op_ty {
                Operand::Value(self.emit_cast_val(rhs, op_ty, common_ty))
            } else {
                rhs
            };
            let is_min = self.emit_cmp_val(IrCmpOp::Eq, lhs_c, min_const, common_ty);
            let is_neg1 = self.emit_cmp_val(IrCmpOp::Eq, rhs_c, neg_one, common_ty);
            let both = self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(is_min),
                Operand::Value(is_neg1),
                IrType::I32,
            );
            self.emit_ubsan_trap_if(both);
        }
    }

    /// Check floating-point division `lhs / rhs` for `rhs == 0.0` (`float-divide-by-zero`).
    pub(super) fn emit_ubsan_float_div_check(&mut self, rhs: Operand, op_ty: IrType) {
        if !self.ubsan_active(UbsanFlags::FLOAT_DIV_ZERO) {
            return;
        }
        let zero = match op_ty {
            IrType::F32 => Operand::Const(IrConst::F32(0.0)),
            IrType::F64 => Operand::Const(IrConst::F64(0.0)),
            IrType::F128 => Operand::Const(IrConst::long_double(0.0)),
            _ => return,
        };
        let is_zero = self.emit_cmp_val(IrCmpOp::Eq, rhs, zero, op_ty);
        self.emit_ubsan_trap_if(is_zero);
    }

    /// Check integer shifts (`<<`, `>>`, `<<=`, `>>=`) for:
    /// - `shift-exponent`: `rhs < 0 || rhs >= bit_width(promoted_lhs)`
    ///   (single unsigned compare `(rhs as unsigned) >= W`).
    /// - `shift-base` (C11 6.5.7p4, signed `<<` when `!wrap_signed_overflow`):
    ///   `lhs < 0` or `lhs` shifted left overflows the signed bit-width `W`,
    ///   equivalent to `((lhs as unsigned_W) >> ((W - 1) - rhs)) != 0`.
    pub(super) fn emit_ubsan_shift_checks(
        &mut self,
        is_shl: bool,
        lhs: Operand,
        lhs_op_ty: IrType,
        promoted_lhs_ty: IrType,
        rhs_full: Operand,
        rhs_promoted_ty: IrType,
    ) {
        let width_bits: u32 = match promoted_lhs_ty {
            IrType::I32 | IrType::U32 => 32,
            IrType::I64 | IrType::U64 => 64,
            IrType::I128 | IrType::U128 => 128,
            _ => return,
        };

        if self.ubsan_active(UbsanFlags::SHIFT_EXPONENT) {
            let cmp_ty = match rhs_promoted_ty {
                IrType::I128 | IrType::U128 => IrType::U128,
                IrType::I64 | IrType::U64 => IrType::U64,
                _ => IrType::U32,
            };
            let rhs_u = if rhs_promoted_ty != cmp_ty {
                Operand::Value(self.emit_cast_val(rhs_full, rhs_promoted_ty, cmp_ty))
            } else {
                rhs_full
            };
            let w_const = match cmp_ty {
                IrType::U128 => Operand::Const(IrConst::I128(width_bits as i128)),
                IrType::U64 => Operand::Const(IrConst::I64(width_bits as i64)),
                _ => Operand::Const(IrConst::I32(width_bits as i32)),
            };
            let bad_exp = self.emit_cmp_val(IrCmpOp::Uge, rhs_u, w_const, cmp_ty);
            self.emit_ubsan_trap_if(bad_exp);
        }

        if is_shl
            && promoted_lhs_ty.is_signed()
            && !self.wrap_signed_overflow
            && self.ubsan_active(UbsanFlags::SHIFT_BASE)
        {
            let unsigned_lhs_ty = match promoted_lhs_ty {
                IrType::I32 => IrType::U32,
                IrType::I64 => IrType::U64,
                IrType::I128 => IrType::U128,
                _ => return,
            };
            let lhs_u = Operand::Value(self.emit_cast_val(lhs, lhs_op_ty, unsigned_lhs_ty));
            // Clamp shift count with `& (W - 1)` so `shift-base` stays well-defined
            // even when `shift-exponent` is disabled (`-fno-sanitize=shift-exponent`).
            let rhs_u32 = if rhs_promoted_ty != IrType::U32 {
                Operand::Value(self.emit_cast_val(rhs_full, rhs_promoted_ty, IrType::U32))
            } else {
                rhs_full
            };
            let masked_rhs = self.emit_binop_val(
                IrBinOp::And,
                rhs_u32,
                Operand::Const(IrConst::I32((width_bits - 1) as i32)),
                IrType::U32,
            );
            let rev_shift = self.emit_binop_val(
                IrBinOp::Xor,
                Operand::Const(IrConst::I32((width_bits - 1) as i32)),
                Operand::Value(masked_rhs),
                IrType::U32,
            );
            let rev_shift_ty = if unsigned_lhs_ty == IrType::U32 {
                Operand::Value(rev_shift)
            } else {
                Operand::Value(self.emit_cast_val(
                    Operand::Value(rev_shift),
                    IrType::U32,
                    unsigned_lhs_ty,
                ))
            };
            let high_bits =
                self.emit_binop_val(IrBinOp::LShr, lhs_u, rev_shift_ty, unsigned_lhs_ty);
            let bad_base = self.emit_cmp_val(
                IrCmpOp::Ne,
                Operand::Value(high_bits),
                Operand::Const(IrConst::zero(unsigned_lhs_ty)),
                unsigned_lhs_ty,
            );
            self.emit_ubsan_trap_if(bad_base);
        }
    }

    /// Check pointer `ptr` != NULL in `AddressSpace::Default` (`-fsanitize=null`).
    pub(super) fn emit_ubsan_null_check(&mut self, ptr: Value, seg: AddressSpace) {
        if seg != AddressSpace::Default || !self.ubsan_active(UbsanFlags::NULL) {
            return;
        }
        let ptr_int_ty = target_int_ir_type();
        let is_null = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(ptr),
            Operand::Const(IrConst::ptr_int(0)),
            ptr_int_ty,
        );
        self.emit_ubsan_trap_if(is_null);
    }

    /// Check a loaded `_Bool` byte value is `0` or `1` (`-fsanitize=bool`).
    pub(super) fn emit_ubsan_bool_load_check(&mut self, loaded: Operand, load_ty: IrType) {
        if !self.ubsan_active(UbsanFlags::BOOL) || !matches!(load_ty, IrType::I8 | IrType::U8) {
            return;
        }
        let u8_val = if load_ty == IrType::U8 {
            loaded
        } else {
            Operand::Value(self.emit_cast_val(loaded, load_ty, IrType::U8))
        };
        let bad_bool = self.emit_cmp_val(
            IrCmpOp::Ugt,
            u8_val,
            Operand::Const(IrConst::I8(1)),
            IrType::U8,
        );
        self.emit_ubsan_trap_if(bad_bool);
    }

    /// Check VLA declaration dimension `dim_val > 0` (`-fsanitize=vla-bound`).
    pub(super) fn emit_ubsan_vla_bound_check(&mut self, dim_val: Operand, dim_expr: &Expr) {
        if !self.ubsan_active(UbsanFlags::VLA_BOUND) {
            return;
        }
        let expr_ty = self.value_ir_type(dim_expr);
        let is_signed = self.expr_ctype(dim_expr).is_signed() || expr_ty.is_signed();
        let cmp_ty = match (expr_ty.size(), is_signed) {
            (16, true) => IrType::I128,
            (16, false) => IrType::U128,
            (8, true) => IrType::I64,
            (8, false) => IrType::U64,
            (_, true) => IrType::I32,
            (_, false) => IrType::U32,
        };
        let v = self.emit_implicit_cast(dim_val, expr_ty, cmp_ty);
        let zero = Operand::Const(IrConst::zero(cmp_ty));
        let cmp_op = if is_signed { IrCmpOp::Sle } else { IrCmpOp::Eq };
        let bad = self.emit_cmp_val(cmp_op, v, zero, cmp_ty);
        self.emit_ubsan_trap_if(bad);
    }

    /// Check array subscript `actual_base[actual_index]` for:
    /// - Static array bounds (`-fsanitize=bounds` / `-fsanitize=bounds-strict`),
    ///   honoring C11 6.5.6p8 one-past-the-end (`<= N`) under `&a[i]` vs `< N`
    ///   when dereferenced, and skipping trailing struct-hack `arr[0]`/`arr[1]`
    ///   members unless `bounds-strict` is enabled.
    /// - Dynamic VLA bounds (`-fsanitize=vla-bound` or `-fsanitize=bounds`) when
    ///   `actual_base` has a runtime `VmLayout` size.
    pub(super) fn emit_ubsan_subscript_bounds_check(
        &mut self,
        actual_base: &Expr,
        index_ptr_int: Operand,
        vla_stride: Option<Value>,
        elem_size: usize,
        allow_one_past_end: bool,
    ) {
        let ptr_int_ty = target_int_ir_type();
        let unsigned_ptr_ty = if ptr_int_ty == IrType::I32 {
            IrType::U32
        } else {
            IrType::U64
        };

        // 1. Static compile-time array bound check (`CType::Array(_, Some(n))`).
        let base_ct = self.expr_ctype(actual_base);
        if let CType::Array(_, Some(bound)) = base_ct {
            if self.ubsan_active(UbsanFlags::BOUNDS) {
                let strict = self.ubsan_active(UbsanFlags::BOUNDS_STRICT);
                if bound <= 1 && !strict && self.is_trailing_struct_hack_member(actual_base) {
                    return;
                }
                let idx_u =
                    Operand::Value(self.emit_cast_val(index_ptr_int, ptr_int_ty, unsigned_ptr_ty));
                let bound_op = Operand::Const(IrConst::ptr_int(bound as i64));
                let cmp_op = if allow_one_past_end {
                    IrCmpOp::Ugt
                } else {
                    IrCmpOp::Uge
                };
                let bad = self.emit_cmp_val(cmp_op, idx_u, bound_op, unsigned_ptr_ty);
                self.emit_ubsan_trap_if(bad);
            }
            return;
        }

        // 2. Dynamic VLA bound check via `vm_layout_of_expr`.
        if (self.ubsan_active(UbsanFlags::VLA_BOUND) || self.ubsan_active(UbsanFlags::BOUNDS))
            && let Some(layout) = self.vm_layout_of_expr(actual_base)
            && let Some(total_bytes) = layout.size
        {
            let idx_u =
                Operand::Value(self.emit_cast_val(index_ptr_int, ptr_int_ty, unsigned_ptr_ty));
            let bound_elems = if let Some(stride_val) = vla_stride {
                let s_u =
                    self.emit_cast_val(Operand::Value(stride_val), ptr_int_ty, unsigned_ptr_ty);
                let tot_u =
                    self.emit_cast_val(Operand::Value(total_bytes), ptr_int_ty, unsigned_ptr_ty);
                Operand::Value(self.emit_binop_val(
                    IrBinOp::UDiv,
                    Operand::Value(tot_u),
                    Operand::Value(s_u),
                    unsigned_ptr_ty,
                ))
            } else if elem_size <= 1 {
                Operand::Value(self.emit_cast_val(
                    Operand::Value(total_bytes),
                    ptr_int_ty,
                    unsigned_ptr_ty,
                ))
            } else {
                let tot_u =
                    self.emit_cast_val(Operand::Value(total_bytes), ptr_int_ty, unsigned_ptr_ty);
                Operand::Value(self.emit_binop_val(
                    IrBinOp::UDiv,
                    Operand::Value(tot_u),
                    Operand::Const(IrConst::ptr_int(elem_size as i64)),
                    unsigned_ptr_ty,
                ))
            };
            let cmp_op = if allow_one_past_end {
                IrCmpOp::Ugt
            } else {
                IrCmpOp::Uge
            };
            let bad = self.emit_cmp_val(cmp_op, idx_u, bound_elems, unsigned_ptr_ty);
            self.emit_ubsan_trap_if(bad);
        }
    }

    /// True when `expr` accesses the trailing field of a `struct` (or `union`
    /// member), used to exempt C89 `arr[0]` / `arr[1]` struct hacks under
    /// non-strict `-fsanitize=bounds`.
    fn is_trailing_struct_hack_member(&self, expr: &Expr) -> bool {
        let (base, field_name, is_ptr) = match expr {
            Expr::MemberAccess(b, f, _) => (b.as_ref(), f.as_str(), false),
            Expr::PointerMemberAccess(b, f, _) => (b.as_ref(), f.as_str(), true),
            _ => return false,
        };
        let base_ct = self.expr_ctype(base);
        let struct_ct = if is_ptr {
            match base_ct {
                CType::Pointer(inner, _) => inner.as_ref().clone(),
                _ => return false,
            }
        } else {
            base_ct
        };
        let key = match struct_ct {
            CType::Struct(k) => k,
            CType::Union(_) => return true,
            _ => return false,
        };
        let layouts = self.types.borrow_struct_layouts();
        if let Some(layout) = layouts.get(key.as_ref()) {
            if layout.is_union {
                return true;
            }
            return layout
                .fields
                .last()
                .is_some_and(|last| last.name == field_name);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ubsan_undefined_group_matches_gcc16() {
        let undef = UbsanFlags::parse_list("undefined", false).unwrap();
        assert!(undef.contains(UbsanFlags::SIGNED_OVERFLOW));
        assert!(undef.contains(UbsanFlags::INT_DIV_ZERO));
        assert!(undef.contains(UbsanFlags::SHIFT_BASE));
        assert!(undef.contains(UbsanFlags::SHIFT_EXPONENT));
        assert!(undef.contains(UbsanFlags::BOUNDS));
        assert!(undef.contains(UbsanFlags::NULL));
        assert!(undef.contains(UbsanFlags::BOOL));
        assert!(undef.contains(UbsanFlags::VLA_BOUND));
        assert!(!undef.contains(UbsanFlags::FLOAT_DIV_ZERO));
        assert!(!undef.contains(UbsanFlags::BOUNDS_STRICT));
    }

    #[test]
    fn ubsan_rejects_sanitize_all_and_address() {
        assert!(UbsanFlags::parse_list("all", false).is_err());
        assert!(UbsanFlags::parse_list("all", true).is_ok());
        assert!(UbsanFlags::parse_list("address", false).is_err());
    }
}
