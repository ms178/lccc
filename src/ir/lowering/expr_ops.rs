//! Expression operator lowering: binary ops, unary ops, conditional/ternary,
//! short-circuit logical operators, and increment/decrement.
//!
//! Extracted from expr.rs to keep expression lowering manageable.

use super::lower::Lowerer;
use crate::common::types::{widened_op_type, AddressSpace, CType, IrType};
use crate::frontend::parser::ast::{BinOp, Expr, PostfixOp, UnaryOp};
use crate::ir::reexports::{
    Instruction, IrBinOp, IrCmpOp, IrConst, IrUnaryOp, Operand, Terminator, Value,
};

impl Lowerer {
    // -----------------------------------------------------------------------
    // Binary operations
    // -----------------------------------------------------------------------

    pub(super) fn lower_binary_op(&mut self, op: &BinOp, lhs: &Expr, rhs: &Expr) -> Operand {
        match op {
            BinOp::LogicalAnd => return self.lower_short_circuit(lhs, rhs, true),
            BinOp::LogicalOr => return self.lower_short_circuit(lhs, rhs, false),
            _ => {}
        }

        // Vector arithmetic (element-wise) -- check before constant folding since
        // vectors can't be constant-folded and get_expr_type doesn't handle them.
        // Per GCC vector extensions, if either operand is a vector, the operation
        // is element-wise. A scalar operand is splatted (broadcast) to all lanes.
        if matches!(
            op,
            BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Mod
                | BinOp::BitAnd
                | BinOp::BitOr
                | BinOp::BitXor
                | BinOp::Shl
                | BinOp::Shr
        ) {
            let lhs_ct = self.expr_ctype(lhs);
            if lhs_ct.is_vector() {
                return self.lower_vector_binary_op(op, lhs, rhs, &lhs_ct);
            }
            let rhs_ct = self.expr_ctype(rhs);
            if rhs_ct.is_vector() {
                return self.lower_vector_binary_op(op, lhs, rhs, &rhs_ct);
            }
        }

        // Fast path: constant-fold pure integer arithmetic at lowering time.
        // This ensures correct C type semantics (e.g., 32-bit int width for
        // expressions like (1 << 31) / N) which the IR-level fold pass may lose
        // since it operates on 64-bit values without C type information.
        // Only apply to integer-only operations (shifts, bitwise, int arithmetic).
        // Skip float-involving expressions since eval_const_binop_float doesn't
        // correctly handle mixed int/float type promotion (e.g., int - float).
        if matches!(
            op,
            BinOp::Shl
                | BinOp::Shr
                | BinOp::BitAnd
                | BinOp::BitOr
                | BinOp::BitXor
                | BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Mod
        ) {
            let lhs_ty = self.get_expr_type(lhs);
            let rhs_ty = self.get_expr_type(rhs);
            // _Float128 (carrier U128) is NOT an integer: folding its bit
            // pattern as an i128 would compute garbage (e.g. 1.5F128 * 3.125F128
            // multiplied as integers). F128 ops go through libgcc soft-float.
            let lhs_f128 = matches!(self.expr_ctype(lhs), CType::Float128);
            let rhs_f128 = matches!(self.expr_ctype(rhs), CType::Float128);
            if !lhs_ty.is_float() && !rhs_ty.is_float() && !lhs_f128 && !rhs_f128 {
                if let Some(val) = self.eval_const_expr_from_parts(op, lhs, rhs) {
                    return Operand::Const(val);
                }
            }
        }

        // Complex arithmetic
        if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div) {
            let lhs_ct = self.expr_ctype(lhs);
            let rhs_ct = self.expr_ctype(rhs);
            if lhs_ct.is_complex() || rhs_ct.is_complex() {
                return self.lower_complex_binary_op(op, lhs, rhs, &lhs_ct, &rhs_ct);
            }
        }

        // Pointer arithmetic
        if matches!(op, BinOp::Add | BinOp::Sub) {
            if let Some(result) = self.try_lower_pointer_arithmetic(op, lhs, rhs) {
                return result;
            }
        }

        // Complex equality/inequality
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            let lhs_ct = self.expr_ctype(lhs);
            let rhs_ct = self.expr_ctype(rhs);
            if lhs_ct.is_complex() || rhs_ct.is_complex() {
                return self.lower_complex_comparison(op, lhs, rhs, &lhs_ct, &rhs_ct);
            }
        }

        // Pointer comparison
        if matches!(
            op,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        ) && (self.expr_is_pointer(lhs) || self.expr_is_pointer(rhs))
        {
            let lhs_val = self.lower_expr(lhs);
            let rhs_val = self.lower_expr(rhs);
            let cmp_op = Self::binop_to_cmp(*op, true);
            let ptr_ty = crate::common::types::target_int_ir_type();
            let dest = self.emit_cmp_val(cmp_op, lhs_val, rhs_val, ptr_ty);
            return Operand::Value(dest);
        }

        // _Float128 (binary128): arithmetic and comparisons lower to libgcc
        // soft-float helper calls (__addtf3/__eqtf2/...), matching GCC.
        let lhs_ct = self.expr_ctype(lhs);
        let rhs_ct = self.expr_ctype(rhs);
        if matches!(lhs_ct, CType::Float128) || matches!(rhs_ct, CType::Float128) {
            return self.lower_f128_binop(op, lhs, lhs_ct, rhs, rhs_ct);
        }

        self.lower_arithmetic_binop(op, lhs, rhs)
    }

    /// Lower arithmetic/comparison on _Float128 via libgcc soft-float calls.
    fn lower_f128_binop(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        lhs_ct: CType,
        rhs: &Expr,
        rhs_ct: CType,
    ) -> Operand {
        let lhs_val = self.lower_expr(lhs);
        let rhs_val = self.lower_expr(rhs);
        let lhs128 = self.convert_to_f128(lhs_val, &lhs_ct);
        let rhs128 = self.convert_to_f128(rhs_val, &rhs_ct);
        match op {
            BinOp::Add => {
                let d = self.emit_f128_binop_call("__addtf3", lhs128, rhs128);
                Operand::Value(d)
            }
            BinOp::Sub => {
                let d = self.emit_f128_binop_call("__subtf3", lhs128, rhs128);
                Operand::Value(d)
            }
            BinOp::Mul => {
                let d = self.emit_f128_binop_call("__multf3", lhs128, rhs128);
                Operand::Value(d)
            }
            BinOp::Div => {
                let d = self.emit_f128_binop_call("__divtf3", lhs128, rhs128);
                Operand::Value(d)
            }
            BinOp::Eq => {
                let d = self.emit_f128_cmp_call("__eqtf2", lhs128, rhs128);
                let z = self.emit_cmp_val(
                    IrCmpOp::Eq,
                    Operand::Value(d),
                    Operand::Const(IrConst::I64(0)),
                    IrType::I32,
                );
                Operand::Value(z)
            }
            BinOp::Ne => {
                let d = self.emit_f128_cmp_call("__netf2", lhs128, rhs128);
                let z = self.emit_cmp_val(
                    IrCmpOp::Ne,
                    Operand::Value(d),
                    Operand::Const(IrConst::I64(0)),
                    IrType::I32,
                );
                Operand::Value(z)
            }
            BinOp::Lt => {
                let d = self.emit_f128_cmp_call("__lttf2", lhs128, rhs128);
                let z = self.emit_cmp_val(
                    IrCmpOp::Slt,
                    Operand::Value(d),
                    Operand::Const(IrConst::I64(0)),
                    IrType::I32,
                );
                Operand::Value(z)
            }
            BinOp::Le => {
                let d = self.emit_f128_cmp_call("__letf2", lhs128, rhs128);
                let z = self.emit_cmp_val(
                    IrCmpOp::Sle,
                    Operand::Value(d),
                    Operand::Const(IrConst::I64(0)),
                    IrType::I32,
                );
                Operand::Value(z)
            }
            BinOp::Gt => {
                let d = self.emit_f128_cmp_call("__gttf2", lhs128, rhs128);
                let z = self.emit_cmp_val(
                    IrCmpOp::Sgt,
                    Operand::Value(d),
                    Operand::Const(IrConst::I64(0)),
                    IrType::I32,
                );
                Operand::Value(z)
            }
            BinOp::Ge => {
                let d = self.emit_f128_cmp_call("__getf2", lhs128, rhs128);
                let z = self.emit_cmp_val(
                    IrCmpOp::Sge,
                    Operand::Value(d),
                    Operand::Const(IrConst::I64(0)),
                    IrType::I32,
                );
                Operand::Value(z)
            }
            _ => self.lower_arithmetic_binop(op, lhs, rhs),
        }
    }

    fn lower_complex_binary_op(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
        lhs_ct: &CType,
        rhs_ct: &CType,
    ) -> Operand {
        let result_ct = self.common_complex_type(lhs_ct, rhs_ct);

        // Special case: real - complex uses negation for imag part to preserve -0.0
        if *op == BinOp::Sub && !lhs_ct.is_complex() && rhs_ct.is_complex() {
            let lhs_val = self.lower_expr(lhs);
            let rhs_val = self.lower_expr(rhs);
            let rhs_complex = self.convert_to_complex(rhs_val, rhs_ct, &result_ct);
            let rhs_ptr = self.operand_to_value(rhs_complex);
            return self.lower_real_minus_complex(lhs_val, lhs_ct, rhs_ptr, &result_ct);
        }

        let lhs_val = self.lower_expr(lhs);
        let rhs_val = self.lower_expr(rhs);
        let lhs_complex = self.convert_to_complex(lhs_val, lhs_ct, &result_ct);
        let rhs_complex = self.convert_to_complex(rhs_val, rhs_ct, &result_ct);
        let lhs_ptr = self.operand_to_value(lhs_complex);
        let rhs_ptr = self.operand_to_value(rhs_complex);

        match op {
            BinOp::Add => self.lower_complex_add(lhs_ptr, rhs_ptr, &result_ct),
            BinOp::Sub => self.lower_complex_sub(lhs_ptr, rhs_ptr, &result_ct),
            BinOp::Mul => self.lower_complex_mul(lhs_ptr, rhs_ptr, &result_ct),
            BinOp::Div => self.lower_complex_div(lhs_ptr, rhs_ptr, &result_ct),
            _ => unreachable!("unsupported complex binary op: {:?}", op),
        }
    }

    fn try_lower_pointer_arithmetic(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Option<Operand> {
        let lhs_is_ptr = self.expr_is_pointer(lhs);
        let rhs_is_ptr = self.expr_is_pointer(rhs);
        // Use target-appropriate pointer-width type: I32 on ILP32, I64 on LP64
        let ptr_int_ty = crate::common::types::target_int_ir_type();

        if lhs_is_ptr && !rhs_is_ptr {
            let elem_size = self.get_pointer_elem_size_from_expr(lhs);
            let rhs_ty = self.get_expr_type(rhs);
            let lhs_val = self.lower_expr(lhs);
            let rhs_val = self.lower_expr(rhs);
            // Widen the integer index to pointer width (sign-extend for signed types,
            // zero-extend for unsigned) before scaling and adding to the pointer.
            let rhs_val = self.emit_implicit_cast(rhs_val, rhs_ty, ptr_int_ty);
            let scaled_rhs = self.scale_index(rhs_val, elem_size);
            let ir_op = if *op == BinOp::Add {
                IrBinOp::Add
            } else {
                IrBinOp::Sub
            };
            let dest = self.emit_binop_val(ir_op, lhs_val, scaled_rhs, ptr_int_ty);
            Some(Operand::Value(dest))
        } else if rhs_is_ptr && !lhs_is_ptr && *op == BinOp::Add {
            let elem_size = self.get_pointer_elem_size_from_expr(rhs);
            let lhs_ty = self.get_expr_type(lhs);
            let lhs_val = self.lower_expr(lhs);
            // Widen the integer index to pointer width before scaling and adding to the pointer.
            let lhs_val = self.emit_implicit_cast(lhs_val, lhs_ty, ptr_int_ty);
            let rhs_val = self.lower_expr(rhs);
            let scaled_lhs = self.scale_index(lhs_val, elem_size);
            let dest = self.emit_binop_val(IrBinOp::Add, scaled_lhs, rhs_val, ptr_int_ty);
            Some(Operand::Value(dest))
        } else if lhs_is_ptr && rhs_is_ptr && *op == BinOp::Sub {
            let elem_size = self.get_pointer_elem_size_from_expr(lhs);
            let lhs_val = self.lower_expr(lhs);
            let rhs_val = self.lower_expr(rhs);
            let diff = self.emit_binop_val(IrBinOp::Sub, lhs_val, rhs_val, ptr_int_ty);
            if elem_size > 1 {
                let scale = Operand::Const(IrConst::ptr_int(elem_size as i64));
                let dest =
                    self.emit_binop_val(IrBinOp::SDiv, Operand::Value(diff), scale, ptr_int_ty);
                Some(Operand::Value(dest))
            } else {
                Some(Operand::Value(diff))
            }
        } else {
            None
        }
    }

    /// Multiply an index value by a scale factor (for pointer arithmetic).
    pub(super) fn scale_index(&mut self, index: Operand, scale: usize) -> Operand {
        if scale <= 1 {
            return index;
        }
        let ptr_int_ty = crate::common::types::target_int_ir_type();
        let scaled = self.emit_binop_val(
            IrBinOp::Mul,
            index,
            Operand::Const(IrConst::ptr_int(scale as i64)),
            ptr_int_ty,
        );
        Operand::Value(scaled)
    }

    fn lower_arithmetic_binop(&mut self, op: &BinOp, lhs: &Expr, rhs: &Expr) -> Operand {
        let lhs_ty = self.infer_expr_type(lhs);
        let rhs_ty = self.infer_expr_type(rhs);
        let lhs_expr_ty = self.get_expr_type(lhs);
        let rhs_expr_ty = self.get_expr_type(rhs);

        let is_shift = matches!(op, BinOp::Shl | BinOp::Shr);

        let (op_ty, is_unsigned, common_ty) = if lhs_expr_ty.is_float() || rhs_expr_ty.is_float() {
            let ft = if lhs_expr_ty == IrType::F128 || rhs_expr_ty == IrType::F128 {
                IrType::F128
            } else if lhs_expr_ty == IrType::F64 || rhs_expr_ty == IrType::F64 {
                IrType::F64
            } else {
                IrType::F32
            };
            (ft, false, ft)
        } else if is_shift {
            let promoted_lhs = Self::integer_promote(lhs_ty);
            let shift_op_ty = widened_op_type(promoted_lhs);
            (shift_op_ty, promoted_lhs.is_unsigned(), promoted_lhs)
        } else {
            let ct = Self::common_type(lhs_ty, rhs_ty);
            let ot = widened_op_type(ct);
            (ot, ct.is_unsigned(), ct)
        };

        let (lhs_val, rhs_val) = if is_shift {
            let shift_lhs_ty = widened_op_type(op_ty);
            let lhs_val = self.lower_expr_with_type(lhs, shift_lhs_ty);
            // Shift amount is always small; use widened type for consistency
            let shift_rhs_ty = widened_op_type(IrType::I32);
            let rhs_val = self.lower_expr_with_type(rhs, shift_rhs_ty);
            (lhs_val, rhs_val)
        } else {
            let mut lhs_val = self.lower_expr_with_type(lhs, op_ty);
            let mut rhs_val = self.lower_expr_with_type(rhs, op_ty);
            if common_ty == IrType::U32 && !crate::common::types::target_is_32bit() {
                // On 64-bit: zero-extend both operands to ensure correct 64-bit unsigned
                // semantics. All operand types (signed or unsigned, any width <= 32) need
                // truncation to U32 so that 64-bit operations (like divq) see the correct
                // 32-bit unsigned values.
                // On 32-bit: not needed since U32 operations are native width.
                //
                // from_ty must be the operand's ACTUAL type (`op_ty`, which is
                // U32 here: `widened_op_type(U32) == U32`), not the hardcoded
                // I64. The old I64 label made this a type-lying identity cast
                // that simplify could not fold (its chain check requires
                // inner_to == from_ty), so it survived to codegen as a
                // spurious `mov` per comparison operand — the expat/sqlite
                // `mov %esi,%ebp; mov %ebp,%r15d` copy chains. With the
                // correct label the cast is Cast(U32->U32), which simplify
                // folds to a Copy and copy-prop eliminates.
                if lhs_ty.size() <= 4 {
                    let masked = self.emit_cast_val(lhs_val, op_ty, IrType::U32);
                    lhs_val = Operand::Value(masked);
                }
                if rhs_ty.size() <= 4 {
                    let masked = self.emit_cast_val(rhs_val, op_ty, IrType::U32);
                    rhs_val = Operand::Value(masked);
                }
            }
            (lhs_val, rhs_val)
        };
        let dest = self.fresh_value();

        match op {
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let cmp_op = Self::binop_to_cmp(*op, is_unsigned);
                self.emit(Instruction::Cmp {
                    dest,
                    op: cmp_op,
                    lhs: lhs_val,
                    rhs: rhs_val,
                    ty: common_ty,
                });
            }
            _ => {
                let ir_op = Self::binop_to_ir(*op, is_unsigned);
                self.emit(Instruction::BinOp {
                    dest,
                    op: ir_op,
                    lhs: lhs_val,
                    rhs: rhs_val,
                    ty: op_ty,
                });
            }
        }

        self.maybe_narrow_binop_result(dest, op, common_ty)
    }

    /// Integer promotion: types narrower than int are promoted to int.
    pub(super) fn integer_promote(ty: IrType) -> IrType {
        match ty {
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 => IrType::I32,
            _ => ty,
        }
    }

    /// Convert a comparison BinOp to the corresponding IrCmpOp.
    pub(super) fn binop_to_cmp(op: BinOp, is_unsigned: bool) -> IrCmpOp {
        match (op, is_unsigned) {
            (BinOp::Eq, _) => IrCmpOp::Eq,
            (BinOp::Ne, _) => IrCmpOp::Ne,
            (BinOp::Lt, false) => IrCmpOp::Slt,
            (BinOp::Lt, true) => IrCmpOp::Ult,
            (BinOp::Le, false) => IrCmpOp::Sle,
            (BinOp::Le, true) => IrCmpOp::Ule,
            (BinOp::Gt, false) => IrCmpOp::Sgt,
            (BinOp::Gt, true) => IrCmpOp::Ugt,
            (BinOp::Ge, false) => IrCmpOp::Sge,
            (BinOp::Ge, true) => IrCmpOp::Uge,
            _ => unreachable!("non-comparison BinOp in binop_to_cmp: {:?}", op),
        }
    }

    /// Convert an arithmetic/bitwise BinOp to the corresponding IrBinOp.
    pub(super) fn binop_to_ir(op: BinOp, is_unsigned: bool) -> IrBinOp {
        match (op, is_unsigned) {
            (BinOp::Add, _) => IrBinOp::Add,
            (BinOp::Sub, _) => IrBinOp::Sub,
            (BinOp::Mul, _) => IrBinOp::Mul,
            (BinOp::Div, false) => IrBinOp::SDiv,
            (BinOp::Div, true) => IrBinOp::UDiv,
            (BinOp::Mod, false) => IrBinOp::SRem,
            (BinOp::Mod, true) => IrBinOp::URem,
            (BinOp::BitAnd, _) => IrBinOp::And,
            (BinOp::BitOr, _) => IrBinOp::Or,
            (BinOp::BitXor, _) => IrBinOp::Xor,
            (BinOp::Shl, _) => IrBinOp::Shl,
            (BinOp::Shr, false) => IrBinOp::AShr,
            (BinOp::Shr, true) => IrBinOp::LShr,
            _ => unreachable!("non-arithmetic BinOp in binop_to_ir: {:?}", op),
        }
    }

    fn maybe_narrow_binop_result(&mut self, dest: Value, op: &BinOp, common_ty: IrType) -> Operand {
        // Only narrow integer results; float operations use their exact type.
        if common_ty.is_float() || common_ty.is_128bit() || op.is_comparison() {
            return Operand::Value(dest);
        }
        let wt = widened_op_type(common_ty);
        if common_ty != wt {
            let narrowed = self.emit_cast_val(Operand::Value(dest), wt, common_ty);
            Operand::Value(narrowed)
        } else {
            Operand::Value(dest)
        }
    }

    // -----------------------------------------------------------------------
    // Unary operations
    // -----------------------------------------------------------------------

    pub(super) fn lower_unary_op(&mut self, op: UnaryOp, inner: &Expr) -> Operand {
        match op {
            UnaryOp::Plus => {
                let inner_ct = self.expr_ctype(inner);
                if inner_ct.is_complex() {
                    return self.lower_expr(inner);
                }
                let val = self.lower_expr(inner);
                let inner_ty = self.infer_expr_type(inner);
                let promoted_ty = Self::integer_promote(inner_ty);
                if promoted_ty != inner_ty && inner_ty.is_integer() {
                    let dest = self.emit_cast_val(val, inner_ty, promoted_ty);
                    Operand::Value(dest)
                } else {
                    val
                }
            }
            UnaryOp::Neg => {
                let inner_ct = self.expr_ctype(inner);
                if inner_ct.is_complex() {
                    let val = self.lower_expr(inner);
                    let ptr = self.operand_to_value(val);
                    return self.lower_complex_neg(ptr, &inner_ct);
                }
                // _Float128 constants are I128 bit patterns (binary128).
                // Float negation flips the SIGN BIT (bit 127), not the
                // integer value: -0x3fff8000... (1.5) must become
                // 0xbfff8000... (-1.5), NOT 0xc0008000... (-3.0). Fold here
                // so no UnaryOp(Neg) with a U128-typed bit pattern ever
                // reaches the generic i128 negator.
                if inner_ct == CType::Float128 {
                    // _Float128 negation = toggle sign bit 127. Constants fold
                    // directly; variables go through the F128Neg intrinsic
                    // (a generic UnaryOp Neg on the U128 bit pattern would
                    // wrap the integer instead — -1.5 -> -3.0).
                    if let Expr::FloatLiteralF128(_, bytes, _) = inner {
                        let bits = u128::from_le_bytes(*bytes);
                        let toggled = (bits ^ (1u128 << 127)) as i128;
                        return Operand::Const(IrConst::I128(toggled));
                    }
                    let val = self.lower_expr(inner);
                    let dest = self.fresh_value();
                    self.emit(Instruction::Intrinsic {
                        dest: Some(dest),
                        op: crate::ir::intrinsics::IntrinsicOp::F128Neg,
                        dest_ptr: None,
                        args: vec![val],
                    });
                    return Operand::Value(dest);
                }
                let ty = self.get_expr_type(inner);
                let inner_ty = self.infer_expr_type(inner);
                let neg_ty = if ty.is_float() || ty.is_128bit() {
                    ty
                } else {
                    widened_op_type(ty)
                };
                let val = self.lower_expr(inner);
                // C integer promotion (C11 6.3.1.1): sign-extend (or zero-extend
                // for unsigned) sub-int types to the operation width before negating.
                // Without this, signed char -13 (0xF3) is zero-extended to 243
                // instead of sign-extended to -13, producing wrong results.
                let val = if !neg_ty.is_float() && inner_ty.size() < neg_ty.size() {
                    Operand::Value(self.emit_cast_val(val, inner_ty, neg_ty))
                } else {
                    val
                };
                let dest = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest,
                    op: IrUnaryOp::Neg,
                    src: val,
                    ty: neg_ty,
                });
                if !neg_ty.is_float() {
                    let promoted_ty = Self::integer_promote(inner_ty);
                    self.maybe_narrow(dest, promoted_ty)
                } else {
                    Operand::Value(dest)
                }
            }
            UnaryOp::BitNot => {
                let inner_ct = self.expr_ctype(inner);
                if inner_ct.is_complex() {
                    return self.lower_complex_conj(inner);
                }
                let inner_ty = self.infer_expr_type(inner);
                let ty = self.get_expr_type(inner);
                let not_ty = if ty.is_128bit() {
                    ty
                } else {
                    widened_op_type(ty)
                };
                let val = self.lower_expr(inner);
                // C integer promotion: widen sub-int types before bitwise NOT,
                // same as for Neg above. E.g. ~(signed char -13) must sign-extend
                // -13 to int before complementing, giving 12 (not -244).
                let val = if inner_ty.size() < not_ty.size() {
                    Operand::Value(self.emit_cast_val(val, inner_ty, not_ty))
                } else {
                    val
                };
                let dest = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest,
                    op: IrUnaryOp::Not,
                    src: val,
                    ty: not_ty,
                });
                let promoted_ty = Self::integer_promote(inner_ty);
                self.maybe_narrow(dest, promoted_ty)
            }
            UnaryOp::LogicalNot => {
                let int_ty = crate::common::types::target_int_ir_type();
                let zero = if int_ty == IrType::I32 {
                    IrConst::I32(0)
                } else {
                    IrConst::I64(0)
                };
                let inner_ct = self.expr_ctype(inner);
                if inner_ct.is_complex() {
                    // !complex_val => (real == 0) && (imag == 0)
                    let val = self.lower_expr(inner);
                    let ptr = self.operand_to_value(val);
                    let bool_val = self.lower_complex_to_bool(ptr, &inner_ct);
                    // Negate: bool_val is 1 if nonzero, so !complex is (bool_val == 0)
                    let dest =
                        self.emit_cmp_val(IrCmpOp::Eq, bool_val, Operand::Const(zero), int_ty);
                    Operand::Value(dest)
                } else {
                    let inner_ty = self.infer_expr_type(inner);
                    let val = self.lower_expr(inner);
                    let cmp_val = self.mask_float_sign_for_truthiness(val, inner_ty);
                    // Use the inner expression's type for comparison when it is
                    // a wide integer (I64/U64) on a 32-bit target. Otherwise a
                    // 64-bit value like 0x1_0000_0000 would be truncated to 32
                    // bits and incorrectly compare equal to zero. Float types
                    // are excluded because mask_float_sign_for_truthiness already
                    // reduces them to an I32 boolean on 32-bit targets.
                    let cmp_ty = if !inner_ty.is_float() && inner_ty.size() > int_ty.size() {
                        inner_ty
                    } else {
                        int_ty
                    };
                    let cmp_zero = match cmp_ty {
                        IrType::I64 | IrType::U64 => IrConst::I64(0),
                        _ => {
                            if int_ty == IrType::I32 {
                                IrConst::I32(0)
                            } else {
                                IrConst::I64(0)
                            }
                        }
                    };
                    let dest =
                        self.emit_cmp_val(IrCmpOp::Eq, cmp_val, Operand::Const(cmp_zero), cmp_ty);
                    Operand::Value(dest)
                }
            }
            UnaryOp::PreInc | UnaryOp::PreDec => self.lower_pre_inc_dec(inner, op),
            UnaryOp::RealPart => self.lower_complex_real_part(inner),
            UnaryOp::ImagPart => self.lower_complex_imag_part(inner),
        }
    }

    // -----------------------------------------------------------------------
    // Conditional (ternary) operator
    // -----------------------------------------------------------------------

    pub(super) fn lower_conditional(
        &mut self,
        cond: &Expr,
        then_expr: &Expr,
        else_expr: &Expr,
    ) -> Operand {
        // Complex types are represented as pointers to stack-allocated {real, imag}
        // pairs. Both lower_expr() for complex variables and lower_function_call()
        // for complex-returning functions produce Ptr operands. However,
        // get_expr_type() returns the function signature's IR return type (e.g., F64
        // for packed _Complex float on x86-64), creating a mismatch. Force Ptr as
        // the common type when either branch is complex to avoid corrupting pointer
        // values with float casts.
        let then_ct = self.expr_ctype(then_expr);
        let else_ct = self.expr_ctype(else_expr);
        let result_is_complex = then_ct.is_complex() || else_ct.is_complex();

        // Constant-fold the condition at lowering time. If the condition is a
        // compile-time constant (e.g., sizeof(x)==4, __builtin_constant_p(v)),
        // skip generating code for the dead branch entirely. This is critical
        // for kernel patterns like hweight_long() where __builtin_constant_p
        // and sizeof checks gate large macro expansions — without folding, the
        // dead branches bloat the IR beyond inlining limits and cause linker
        // errors when always_inline functions can't be inlined.
        if let Some(const_val) = self.eval_const_expr(cond) {
            let is_true = const_val.is_nonzero();
            if is_true {
                let then_val = self.lower_expr(then_expr);
                if result_is_complex {
                    // Convert to the common complex type if the branch types differ
                    let common_ct = self.common_complex_type(&then_ct, &else_ct);
                    return self.convert_to_complex(then_val, &then_ct, &common_ct);
                }
                let then_ty = if self.expr_is_pointer(then_expr) {
                    IrType::Ptr
                } else {
                    self.get_expr_type(then_expr)
                };
                let else_ty = if self.expr_is_pointer(else_expr) {
                    IrType::Ptr
                } else {
                    self.get_expr_type(else_expr)
                };
                let common_ty = Self::common_type(then_ty, else_ty);
                return self.emit_implicit_cast(then_val, then_ty, common_ty);
            } else {
                let else_val = self.lower_expr(else_expr);
                if result_is_complex {
                    // Convert to the common complex type if the branch types differ
                    let common_ct = self.common_complex_type(&then_ct, &else_ct);
                    return self.convert_to_complex(else_val, &else_ct, &common_ct);
                }
                let then_ty = if self.expr_is_pointer(then_expr) {
                    IrType::Ptr
                } else {
                    self.get_expr_type(then_expr)
                };
                let else_ty = if self.expr_is_pointer(else_expr) {
                    IrType::Ptr
                } else {
                    self.get_expr_type(else_expr)
                };
                let common_ty = Self::common_type(then_ty, else_ty);
                return self.emit_implicit_cast(else_val, else_ty, common_ty);
            }
        }

        // For complex types, both branches produce Ptr values; use Ptr as the
        // alloca/store/load type so we don't emit bogus int-to-float casts.
        // When the branch types differ, convert each to the common complex type.
        if result_is_complex {
            let common_ct = self.common_complex_type(&then_ct, &else_ct);
            return self.emit_ternary_branch_cond(
                cond,
                IrType::Ptr,
                |s| {
                    let v = s.lower_expr(then_expr);
                    s.convert_to_complex(v, &then_ct, &common_ct)
                },
                |s| {
                    let v = s.lower_expr(else_expr);
                    s.convert_to_complex(v, &else_ct, &common_ct)
                },
            );
        }

        let mut then_ty = self.get_expr_type(then_expr);
        let mut else_ty = self.get_expr_type(else_expr);
        if self.expr_is_pointer(then_expr) {
            then_ty = IrType::Ptr;
        }
        if self.expr_is_pointer(else_expr) {
            else_ty = IrType::Ptr;
        }
        let common_ty = Self::common_type(then_ty, else_ty);

        // Detect struct-typed ternary where branches produce inconsistent
        // representations: compound literals and struct derefs produce alloca
        // addresses, while function calls returning small (≤8 byte) structs
        // produce packed data in registers.  When one branch produces packed
        // data and the other produces an address, normalize by loading packed
        // data from address-producing branches so both branches produce data
        // values.
        //
        // Only check for struct-typed expressions (both branches struct/union).
        // expr_produces_packed_struct_data() is not valid for non-struct types
        // (it can return true for non-struct function calls).
        let result_is_struct = then_ct.is_struct_or_union() || else_ct.is_struct_or_union();
        let (needs_struct_load, then_produces_packed) = if result_is_struct {
            let tp = self.expr_produces_packed_struct_data(then_expr);
            let ep = self.expr_produces_packed_struct_data(else_expr);
            (tp != ep, tp)
        } else {
            (false, false)
        };
        // Use target int type for loading packed small-struct data (≤8 bytes)
        let effective_ty = if needs_struct_load {
            crate::common::types::target_int_ir_type()
        } else {
            common_ty
        };

        self.emit_ternary_branch_cond(
            cond,
            effective_ty,
            |s| {
                let then_val = s.lower_expr(then_expr);
                if needs_struct_load && !then_produces_packed {
                    // Address-producing branch (compound literal, deref, etc.);
                    // load the packed data to match the other packed-data branch
                    if let Operand::Value(ptr) = then_val {
                        let loaded = s.fresh_value();
                        s.emit(Instruction::Load {
                            volatile: false,
                            dest: loaded,
                            ptr,
                            ty: effective_ty,
                            seg_override: AddressSpace::Default,
                        });
                        Operand::Value(loaded)
                    } else {
                        then_val
                    }
                } else {
                    s.emit_implicit_cast(then_val, then_ty, effective_ty)
                }
            },
            |s| {
                let else_val = s.lower_expr(else_expr);
                if needs_struct_load && then_produces_packed {
                    // Address-producing branch (compound literal, deref, etc.);
                    // load the packed data to match the other packed-data branch
                    if let Operand::Value(ptr) = else_val {
                        let loaded = s.fresh_value();
                        s.emit(Instruction::Load {
                            volatile: false,
                            dest: loaded,
                            ptr,
                            ty: effective_ty,
                            seg_override: AddressSpace::Default,
                        });
                        Operand::Value(loaded)
                    } else {
                        else_val
                    }
                } else {
                    s.emit_implicit_cast(else_val, else_ty, effective_ty)
                }
            },
        )
    }

    /// Lower GNU conditional expression: `cond ? : else_expr`
    /// The "then" value is the condition value itself (evaluated once).
    pub(super) fn lower_gnu_conditional(&mut self, cond: &Expr, else_expr: &Expr) -> Operand {
        let cond_val = self.lower_expr(cond);
        let cond_ty = self.get_expr_type(cond);
        let else_ty = self.get_expr_type(else_expr);

        // Complex types always lower to Ptr; avoid type mismatch with function
        // call return types (e.g., F64 for packed _Complex float).
        let cond_ct = self.expr_ctype(cond);
        let else_ct = self.expr_ctype(else_expr);
        let result_is_complex = cond_ct.is_complex() || else_ct.is_complex();

        if result_is_complex {
            // Complex condition: convert to bool via (real != 0) || (imag != 0)
            let common_ct = self.common_complex_type(&cond_ct, &else_ct);
            let cond_ptr = self.operand_to_value(cond_val);
            let cond_bool_op = self.lower_complex_to_bool(cond_ptr, &cond_ct);
            let cond_bool_val = self.operand_to_value(cond_bool_op);
            return self.emit_ternary_branch(
                Operand::Value(cond_bool_val),
                IrType::Ptr,
                |s| s.convert_to_complex(cond_val, &cond_ct, &common_ct),
                |s| {
                    let v = s.lower_expr(else_expr);
                    s.convert_to_complex(v, &else_ct, &common_ct)
                },
            );
        }

        let common_ty = Self::common_type(cond_ty, else_ty);

        // Convert condition to boolean for branching
        let int_ty = crate::common::types::target_int_ir_type();
        let zero = if int_ty == IrType::I32 {
            IrConst::I32(0)
        } else {
            IrConst::I64(0)
        };
        let cond_bool = self.fresh_value();
        self.emit(Instruction::Cmp {
            dest: cond_bool,
            op: IrCmpOp::Ne,
            lhs: cond_val,
            rhs: Operand::Const(zero),
            ty: int_ty,
        });

        self.emit_ternary_branch(
            Operand::Value(cond_bool),
            common_ty,
            |s| s.emit_implicit_cast(cond_val, cond_ty, common_ty),
            |s| {
                let else_val = s.lower_expr(else_expr);
                s.emit_implicit_cast(else_val, else_ty, common_ty)
            },
        )
    }

    /// Shared helper for ternary branch patterns (conditional and GNU conditional).
    /// Evaluates `then_fn` in the true branch and `else_fn` in the false branch,
    /// stores both results to an alloca, and returns the loaded result.
    /// ms178: Ternary whose condition is an arbitrary C expression lowered via
    /// `lower_condition_branch` (cmp; jcc branch tree for && / || chains).
    fn emit_ternary_branch_cond(
        &mut self,
        cond_expr: &Expr,
        result_ty: IrType,
        then_fn: impl FnOnce(&mut Self) -> Operand,
        else_fn: impl FnOnce(&mut Self) -> Operand,
    ) -> Operand {
        // Use emit_entry_alloca so the alloca is in the entry block, ensuring
        // mem2reg can promote it to SSA/Phi form.
        let int_ty = crate::common::types::target_int_ir_type();
        let min_alloca_size = int_ty.size();
        let alloca_size = result_ty.size().max(min_alloca_size);
        let alloca_ty = if result_ty.size() > min_alloca_size {
            result_ty
        } else {
            int_ty
        };
        let result_alloca = self.emit_entry_alloca(alloca_ty, alloca_size, 0, false);

        let then_label = self.fresh_label();
        let else_label = self.fresh_label();
        let end_label = self.fresh_label();

        self.lower_condition_branch(cond_expr, then_label, else_label);

        self.start_block(then_label);
        let then_val = then_fn(self);
        self.emit(Instruction::Store {
            volatile: false,
            val: then_val,
            ptr: result_alloca,
            ty: alloca_ty,
            seg_override: AddressSpace::Default,
        });
        self.terminate(Terminator::Branch(end_label));

        self.start_block(else_label);
        let else_val = else_fn(self);
        self.emit(Instruction::Store {
            volatile: false,
            val: else_val,
            ptr: result_alloca,
            ty: alloca_ty,
            seg_override: AddressSpace::Default,
        });
        self.terminate(Terminator::Branch(end_label));

        self.start_block(end_label);
        let result = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: result,
            ptr: result_alloca,
            ty: alloca_ty,
            seg_override: AddressSpace::Default,
        });
        Operand::Value(result)
    }

    fn emit_ternary_branch(
        &mut self,
        cond: Operand,
        result_ty: IrType,
        then_fn: impl FnOnce(&mut Self) -> Operand,
        else_fn: impl FnOnce(&mut Self) -> Operand,
    ) -> Operand {
        // Use emit_entry_alloca so the alloca is in the entry block, ensuring
        // mem2reg can promote it to SSA/Phi form. Previously this used self.emit()
        // which placed the alloca in the current block — if the expression was inside
        // nested control flow (e.g., inside a loop or if body), the alloca would
        // be in a non-entry block and mem2reg would refuse to promote it.
        //
        // Use the actual result type for the alloca size so that wider types like
        // long double (F128, 16 bytes) and __int128 are stored correctly.
        let int_ty = crate::common::types::target_int_ir_type();
        let min_alloca_size = int_ty.size();
        let alloca_size = result_ty.size().max(min_alloca_size);
        let alloca_ty = if result_ty.size() > min_alloca_size {
            result_ty
        } else {
            int_ty
        };
        let result_alloca = self.emit_entry_alloca(alloca_ty, alloca_size, 0, false);

        let then_label = self.fresh_label();
        let else_label = self.fresh_label();
        let end_label = self.fresh_label();

        self.terminate(Terminator::CondBranch {
            cond,
            true_label: then_label,
            false_label: else_label,
        });

        self.start_block(then_label);
        let then_val = then_fn(self);
        self.emit(Instruction::Store {
            volatile: false,
            val: then_val,
            ptr: result_alloca,
            ty: alloca_ty,
            seg_override: AddressSpace::Default,
        });
        self.terminate(Terminator::Branch(end_label));

        self.start_block(else_label);
        let else_val = else_fn(self);
        self.emit(Instruction::Store {
            volatile: false,
            val: else_val,
            ptr: result_alloca,
            ty: alloca_ty,
            seg_override: AddressSpace::Default,
        });
        self.terminate(Terminator::Branch(end_label));

        self.start_block(end_label);
        let result = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: result,
            ptr: result_alloca,
            ty: alloca_ty,
            seg_override: AddressSpace::Default,
        });
        Operand::Value(result)
    }

    // -----------------------------------------------------------------------
    // Short-circuit logical operators
    // -----------------------------------------------------------------------

    fn lower_short_circuit(&mut self, lhs: &Expr, rhs: &Expr, is_and: bool) -> Operand {
        // Use target-appropriate int type: I32 on ILP32 (i686), I64 on LP64
        let int_ty = crate::common::types::target_int_ir_type();
        let int_size = if int_ty == IrType::I32 { 4 } else { 8 };
        let make_int_const = |v: i64| -> IrConst {
            if int_ty == IrType::I32 {
                IrConst::I32(v as i32)
            } else {
                IrConst::I64(v)
            }
        };

        // FLAT short-circuit lowering. `a || b || c` parses as `(a || b) || c`;
        // the old per-link lowering built a right-nested value tree (one
        // alloca + branch diamond per link), so the intermediate links were
        // DATA (selected as the next link's false arm) and materialized as
        // setcc/cmov chains even when the whole result only fed a branch.
        // Flatten left-associative runs of the SAME operator into ONE branch
        // sequence with ONE merge value — GCC/ICX's shape for Expat's
        // `nameLength` (cmp;ja chains, zero boolean materialization).
        let mut ops: Vec<&Expr> = Vec::with_capacity(4);
        collect_flat_sc_operands(lhs, is_and, &mut ops);
        collect_flat_sc_operands(rhs, is_and, &mut ops);

        // Drop constant identity operands (false for `||`, true for `&&`):
        // they are pure (eval_const_expr has no side effects) and contribute
        // nothing to the result. A constant that DECIDES the chain (true for
        // `||`, false for `&&`) short-circuits: evaluate the surviving
        // operands before it for side effects, then return the constant.
        let mut kept: Vec<&Expr> = Vec::with_capacity(ops.len());
        for op in &ops {
            match self.eval_const_expr(op) {
                Some(c) => {
                    let truthy = c.is_nonzero();
                    // `||` short-circuits on true; `&&` on false.
                    if truthy != is_and {
                        for &k in &kept {
                            let _ = self.lower_condition_expr(k);
                        }
                        return Operand::Const(make_int_const(if is_and { 0 } else { 1 }));
                    }
                }
                None => kept.push(op),
            }
        }
        if kept.is_empty() {
            // Every operand was an identity constant (a||b||0 with a=b=0 => 0;
            // a&&b&&1 with a=b=1 => 1).
            return Operand::Const(make_int_const(if is_and { 1 } else { 0 }));
        }

        // The LAST operand's value flows out as data, so it must be a
        // normalized 0/1 boolean. A comparison / logical-NOT already yields
        // 0/1; anything else gets the `!= 0` normalization (same rule as the
        // old RHS handling — avoids a redundant re-compare per condition).
        let last = *kept.last().unwrap();
        let last_is_normalized_bool = matches!(last, Expr::BinaryOp(op, _, _, _) if op.is_comparison())
            || matches!(last, Expr::UnaryOp(UnaryOp::LogicalNot, _, _));
        let lower_last_bool = |slf: &mut Self| -> Operand {
            let val = slf.lower_condition_expr(last);
            if last_is_normalized_bool {
                val
            } else {
                Operand::Value(slf.emit_cmp_val(
                    IrCmpOp::Ne,
                    val,
                    Operand::Const(make_int_const(0)),
                    int_ty,
                ))
            }
        };

        if kept.len() == 1 {
            return lower_last_bool(self);
        }

        // Flat branch chain: store the default, branch on each leading
        // operand, store the normalized last operand, merge and load.
        let result_alloca = self.emit_entry_alloca(int_ty, int_size, 0, false);
        let end_label = self.fresh_label();

        let default_val = if is_and { 0 } else { 1 };
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Const(make_int_const(default_val)),
            ptr: result_alloca,
            ty: int_ty,
            seg_override: AddressSpace::Default,
        });

        let n = kept.len();
        for op in kept.iter().take(n - 1) {
            let next_label = self.fresh_label();
            let (true_label, false_label) = if is_and {
                (next_label, end_label)
            } else {
                (end_label, next_label)
            };
            let cond = self.lower_condition_expr(op);
            self.terminate(Terminator::CondBranch {
                cond,
                true_label,
                false_label,
            });
            self.start_block(next_label);
        }

        let last_bool = lower_last_bool(self);
        self.emit(Instruction::Store {
            volatile: false,
            val: last_bool,
            ptr: result_alloca,
            ty: int_ty,
            seg_override: AddressSpace::Default,
        });
        self.terminate(Terminator::Branch(end_label));

        self.start_block(end_label);
        let result = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: result,
            ptr: result_alloca,
            ty: int_ty,
            seg_override: AddressSpace::Default,
        });
        Operand::Value(result)
    }

    // Increment/decrement operators
    // -----------------------------------------------------------------------

    pub(super) fn lower_pre_inc_dec(&mut self, inner: &Expr, op: UnaryOp) -> Operand {
        let is_inc = op == UnaryOp::PreInc;
        self.lower_inc_dec_impl(inner, is_inc, true)
    }

    pub(super) fn lower_post_inc_dec(&mut self, inner: &Expr, op: PostfixOp) -> Operand {
        let is_inc = op == PostfixOp::PostInc;
        self.lower_inc_dec_impl(inner, is_inc, false)
    }

    pub(super) fn lower_inc_dec_impl(
        &mut self,
        inner: &Expr,
        is_inc: bool,
        return_new: bool,
    ) -> Operand {
        if let Some(result) = self.try_lower_bitfield_inc_dec(inner, is_inc, return_new) {
            return result;
        }

        let ty = self.get_expr_type(inner);
        if let Some(lv) = self.lower_lvalue(inner) {
            let loaded = self.load_lvalue_typed(&lv, ty);
            let loaded_val = self.operand_to_value(loaded);

            // The loaded value is an SSA definition and is therefore already
            // the stable result of a post-inc/dec expression.  Keep it in SSA
            // instead of forcing every evaluated post-inc/dec through a
            // non-promotable stack home.  The latter was an old workaround for
            // backend copy coalescing, but it both obscured the true live range
            // and generated a store/load pair on every loop iteration.
            let (step, binop_ty) = self.inc_dec_step_and_type(ty, inner);
            let ir_op = if is_inc { IrBinOp::Add } else { IrBinOp::Sub };
            let result = self.emit_binop_val(ir_op, Operand::Value(loaded_val), step, binop_ty);
            let store_op = if self.is_bool_lvalue(inner) {
                self.emit_bool_normalize_typed(Operand::Value(result), binop_ty)
            } else if ty != binop_ty && ty.is_integer() && binop_ty.is_integer() {
                self.emit_implicit_cast(Operand::Value(result), binop_ty, ty)
            } else {
                Operand::Value(result)
            };
            self.store_lvalue_typed(&lv, store_op, ty);
            if return_new {
                return store_op;
            }
            // A discarded result needs no old value.  Otherwise the pre-update
            // load is exactly the value required by C post-inc/dec semantics.
            if self.discard_expr_result {
                return store_op;
            }
            return Operand::Value(loaded_val);
        }
        self.lower_expr(inner)
    }

    fn try_lower_bitfield_inc_dec(
        &mut self,
        inner: &Expr,
        is_inc: bool,
        return_new: bool,
    ) -> Option<Operand> {
        let (field_addr, storage_ty, bit_offset, bit_width) =
            self.resolve_bitfield_lvalue(inner)?;

        let current_val =
            self.extract_bitfield_from_addr(field_addr, storage_ty, bit_offset, bit_width);

        let ir_op = if is_inc { IrBinOp::Add } else { IrBinOp::Sub };
        let wt = widened_op_type(IrType::I32);
        let one = if wt == IrType::I32 {
            IrConst::I32(1)
        } else {
            IrConst::I64(1)
        };
        let result = self.emit_binop_val(ir_op, current_val, Operand::Const(one), wt);

        self.store_bitfield(
            field_addr,
            storage_ty,
            bit_offset,
            bit_width,
            Operand::Value(result),
            self.expr_access_is_volatile(inner),
        );

        let ret_val = if return_new {
            Operand::Value(result)
        } else {
            current_val
        };
        Some(self.truncate_to_bitfield_value(ret_val, bit_width, storage_ty.is_signed()))
    }

    fn inc_dec_step_and_type(&self, ty: IrType, expr: &Expr) -> (Operand, IrType) {
        if ty == IrType::Ptr {
            let ptr_int_ty = crate::common::types::target_int_ir_type();
            let elem_size = self.get_pointer_elem_size_from_expr(expr);
            (
                Operand::Const(IrConst::ptr_int(elem_size as i64)),
                ptr_int_ty,
            )
        } else if ty == IrType::F64 {
            (Operand::Const(IrConst::F64(1.0)), IrType::F64)
        } else if ty == IrType::F32 {
            (Operand::Const(IrConst::F32(1.0)), IrType::F32)
        } else if ty == IrType::F128 {
            (Operand::Const(IrConst::long_double(1.0)), IrType::F128)
        } else if ty == IrType::I128 || ty == IrType::U128 {
            (Operand::Const(IrConst::I128(1)), ty)
        } else {
            // For regular integer types, use the widened operation type for arithmetic.
            // On 64-bit, this is I64; on 32-bit, this is I32 (or I64 for I64/U64).
            // The caller truncates back to the variable's type after the operation.
            let wt = widened_op_type(ty);
            let one = if wt == IrType::I32 {
                IrConst::I32(1)
            } else {
                IrConst::I64(1)
            };
            (Operand::Const(one), wt)
        }
    }
}

/// Collect the flat operand list of a left-associative short-circuit chain of
/// ONE operator (`&&` or `||`). A nested chain of the OTHER operator is kept
/// as a single operand (it evaluates as a unit with its own merge).
fn collect_flat_sc_operands<'a>(expr: &'a Expr, is_and: bool, ops: &mut Vec<&'a Expr>) {
    if let Expr::BinaryOp(op, lhs, rhs, _) = expr {
        let same_op = if is_and {
            *op == BinOp::LogicalAnd
        } else {
            *op == BinOp::LogicalOr
        };
        if same_op {
            collect_flat_sc_operands(lhs, is_and, ops);
            collect_flat_sc_operands(rhs, is_and, ops);
            return;
        }
    }
    ops.push(expr);
}
