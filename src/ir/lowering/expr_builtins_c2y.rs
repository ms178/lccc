//! GCC 16 / C23 / C2y builtin lowering:
//! - `<stdbit.h>` (`__builtin_stdc_*` 16 type-generic bit/rotate builtins)
//! - `__builtin_counted_by_ref` + `counted_by`-aware `__builtin_dynamic_object_size`
//! - `__builtin_assoc_barrier`
//! - `__builtin_clear_padding`

use super::lower::Lowerer;
use crate::backend::Target;
use crate::common::types::{CType, IrType, StructLayoutProvider, target_is_32bit};
use crate::frontend::parser::ast::{BinOp, Expr};
use crate::frontend::sema::stdbit;
use crate::ir::reexports::{Instruction, IrBinOp, IrCmpOp, IrConst, IrUnaryOp, Operand, Value};

impl Lowerer {
    fn ptr_zero_const() -> IrConst {
        if target_is_32bit() {
            IrConst::I32(0)
        } else {
            IrConst::I64(0)
        }
    }

    fn size_ir_type() -> IrType {
        if target_is_32bit() {
            IrType::U32
        } else {
            IrType::U64
        }
    }

    fn uconst(ty: IrType, v: u64) -> Operand {
        Operand::Const(match ty {
            IrType::I8 | IrType::U8 => IrConst::I8(v as i8),
            IrType::I16 | IrType::U16 => IrConst::I16(v as i16),
            IrType::I32 | IrType::U32 => IrConst::I32(v as i32),
            IrType::I128 | IrType::U128 => IrConst::I128(v as i128),
            _ => IrConst::I64(v as i64),
        })
    }

    pub(super) fn emit_select_val(
        &mut self,
        cond: Operand,
        true_val: Operand,
        false_val: Operand,
        ty: IrType,
    ) -> Value {
        let dest = self.fresh_value();
        self.emit(Instruction::Select {
            dest,
            cond,
            true_val,
            false_val,
            ty,
        });
        dest
    }

    /// Select between two 128-bit values by selecting their low and high 64-bit halves
    /// independently (`Instruction::Select` in backends is single-register `<= 64`-bit).
    fn emit_select_u128(&mut self, cond: Operand, true_val: Operand, false_val: Operand) -> Value {
        let t_lo = self.emit_cast_val(true_val, IrType::U128, IrType::U64);
        let t_hi_sh = self.emit_binop_val(
            IrBinOp::LShr,
            true_val,
            Operand::Const(IrConst::I128(64)),
            IrType::U128,
        );
        let t_hi = self.emit_cast_val(Operand::Value(t_hi_sh), IrType::U128, IrType::U64);

        let f_lo = self.emit_cast_val(false_val, IrType::U128, IrType::U64);
        let f_hi_sh = self.emit_binop_val(
            IrBinOp::LShr,
            false_val,
            Operand::Const(IrConst::I128(64)),
            IrType::U128,
        );
        let f_hi = self.emit_cast_val(Operand::Value(f_hi_sh), IrType::U128, IrType::U64);

        let sel_lo = self.emit_select_val(
            cond,
            Operand::Value(t_lo),
            Operand::Value(f_lo),
            IrType::U64,
        );
        let sel_hi = self.emit_select_val(
            cond,
            Operand::Value(t_hi),
            Operand::Value(f_hi),
            IrType::U64,
        );
        let lo128 = self.emit_cast_val(Operand::Value(sel_lo), IrType::U64, IrType::U128);
        let hi128 = self.emit_cast_val(Operand::Value(sel_hi), IrType::U64, IrType::U128);
        let hi_shifted = self.emit_binop_val(
            IrBinOp::Shl,
            Operand::Value(hi128),
            Operand::Const(IrConst::I128(64)),
            IrType::U128,
        );
        self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(hi_shifted),
            Operand::Value(lo128),
            IrType::U128,
        )
    }

    fn emit_unary_val(&mut self, op: IrUnaryOp, src: Operand, ty: IrType) -> Value {
        let dest = self.fresh_value();
        self.emit(Instruction::UnaryOp { dest, op, src, ty });
        dest
    }

    fn target_supports_hw_rotate(&self, width: u32) -> bool {
        match self.target {
            Target::X86_64 => matches!(width, 8 | 16 | 32 | 64),
            Target::I686 => matches!(width, 8 | 16 | 32),
            Target::Aarch64 | Target::Riscv64 => matches!(width, 32 | 64),
        }
    }

    fn emit_clz_width(&mut self, val: Operand, width: u32, op_ty: IrType, op_bits: u32) -> Value {
        let raw_clz = self.emit_unary_val(IrUnaryOp::Clz, val, op_ty);
        if width < op_bits {
            self.emit_binop_val(
                IrBinOp::Sub,
                Operand::Value(raw_clz),
                Self::uconst(op_ty, (op_bits - width) as u64),
                op_ty,
            )
        } else {
            raw_clz
        }
    }

    /// Branchless sub-word ctz: ORing `(1 << width)` forces bit `width` to 1,
    /// making the operand provably nonzero (`CtzNonZero`) and returning `width`
    /// when `val == 0` while leaving any lower set bit intact.
    fn emit_ctz_width(&mut self, val: Operand, width: u32, op_ty: IrType, op_bits: u32) -> Value {
        if width < op_bits {
            let sentinel = Self::uconst(op_ty, 1u64 << width);
            let bounded = self.emit_binop_val(IrBinOp::Or, val, sentinel, op_ty);
            self.emit_unary_val(IrUnaryOp::CtzNonZero, Operand::Value(bounded), op_ty)
        } else {
            self.emit_unary_val(IrUnaryOp::Ctz, val, op_ty)
        }
    }

    fn emit_not_masked(
        &mut self,
        val: Operand,
        width: u32,
        mask: u64,
        op_ty: IrType,
        op_bits: u32,
    ) -> Operand {
        if width < op_bits {
            Operand::Value(self.emit_binop_val(IrBinOp::Xor, val, Self::uconst(op_ty, mask), op_ty))
        } else {
            Operand::Value(self.emit_unary_val(IrUnaryOp::Not, val, op_ty))
        }
    }

    /// Lower all 16 `__builtin_stdc_*` builtins with branchless selection,
    /// inline popcount, and native hardware rotates where supported.
    pub(super) fn lower_stdc_bit_builtin(&mut self, name: &str, args: &[Expr]) -> Option<Operand> {
        let arity = stdbit::stdc_bit_arity(name)?;
        if args.len() < arity {
            return Some(Operand::Const(IrConst::I32(0)));
        }
        let arg0_ct = self.expr_ctype(&args[0]);
        let ret_ct = stdbit::stdc_bit_return_ctype(name, Some(&arg0_ct)).unwrap_or(CType::UInt);
        let ret_ir_ty = IrType::from_ctype(&ret_ct);

        if let Some(folded) = self.eval_stdc_bit_const(name, args) {
            let v128 = match folded {
                IrConst::I128(v) => v as u128,
                other => other.to_u64().unwrap_or(0) as u128,
            };
            let typed = match ret_ir_ty {
                IrType::I8 | IrType::U8 => IrConst::I8(v128 as i8),
                IrType::I16 | IrType::U16 => IrConst::I16(v128 as i16),
                IrType::I32 | IrType::U32 => IrConst::I32(v128 as i32),
                IrType::I128 | IrType::U128 => IrConst::I128(v128 as i128),
                _ => IrConst::I64(v128 as i64),
            };
            return Some(Operand::Const(typed));
        }

        let width = stdbit::stdc_bit_width_of_ctype(&arg0_ct).unwrap_or(32);
        if width > 64 {
            return self.lower_stdc_bit_u128(name, args, width, ret_ir_ty);
        }
        if matches!(
            name,
            "__builtin_stdc_rotate_left" | "__builtin_stdc_rotate_right"
        ) {
            return Some(self.lower_stdc_rotate(name, args, width, ret_ir_ty));
        }
        self.lower_stdc_bit_u64_unary(name, args, width, ret_ir_ty)
    }

    fn lower_stdc_bit_u64_unary(
        &mut self,
        name: &str,
        args: &[Expr],
        width: u32,
        ret_ir_ty: IrType,
    ) -> Option<Operand> {
        let op_ty = if width <= 32 {
            IrType::U32
        } else {
            IrType::U64
        };
        let op_bits = if width <= 32 { 32u32 } else { 64u32 };
        let mask: u64 = if width == 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };

        let raw = self.lower_expr_with_type(&args[0], op_ty);
        let x = if width < op_bits {
            Operand::Value(self.emit_binop_val(IrBinOp::And, raw, Self::uconst(op_ty, mask), op_ty))
        } else {
            raw
        };

        let out_op_ty: Value = match name {
            "__builtin_stdc_leading_zeros" => self.emit_clz_width(x, width, op_ty, op_bits),
            "__builtin_stdc_leading_ones" => {
                let inv = self.emit_not_masked(x, width, mask, op_ty, op_bits);
                self.emit_clz_width(inv, width, op_ty, op_bits)
            }
            "__builtin_stdc_trailing_zeros" => self.emit_ctz_width(x, width, op_ty, op_bits),
            "__builtin_stdc_trailing_ones" => {
                let inv = self.emit_not_masked(x, width, mask, op_ty, op_bits);
                self.emit_ctz_width(inv, width, op_ty, op_bits)
            }
            "__builtin_stdc_first_leading_one"
            | "__builtin_stdc_first_leading_zero"
            | "__builtin_stdc_first_trailing_one"
            | "__builtin_stdc_first_trailing_zero" => {
                self.lower_stdc_first_bit_u64(name, x, width, mask, op_ty, op_bits)
            }
            "__builtin_stdc_count_ones" => self.emit_unary_val(IrUnaryOp::Popcount, x, op_ty),
            "__builtin_stdc_count_zeros" => {
                let ones = self.emit_unary_val(IrUnaryOp::Popcount, x, op_ty);
                self.emit_binop_val(
                    IrBinOp::Sub,
                    Self::uconst(op_ty, width as u64),
                    Operand::Value(ones),
                    op_ty,
                )
            }
            "__builtin_stdc_has_single_bit" => {
                let xm1 = self.emit_binop_val(IrBinOp::Sub, x, Self::uconst(op_ty, 1), op_ty);
                let cleared = self.emit_binop_val(IrBinOp::And, x, Operand::Value(xm1), op_ty);
                let pow2_or_zero = self.emit_cmp_val(
                    IrCmpOp::Eq,
                    Operand::Value(cleared),
                    Self::uconst(op_ty, 0),
                    op_ty,
                );
                let nonzero = self.emit_cmp_val(IrCmpOp::Ne, x, Self::uconst(op_ty, 0), op_ty);
                let both = self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(nonzero),
                    Operand::Value(pow2_or_zero),
                    IrType::I32,
                );
                return Some(Operand::Value(self.emit_cast_val(
                    Operand::Value(both),
                    IrType::I32,
                    ret_ir_ty,
                )));
            }
            "__builtin_stdc_bit_width" => {
                let lz = self.emit_clz_width(x, width, op_ty, op_bits);
                self.emit_binop_val(
                    IrBinOp::Sub,
                    Self::uconst(op_ty, width as u64),
                    Operand::Value(lz),
                    op_ty,
                )
            }
            "__builtin_stdc_bit_floor" | "__builtin_stdc_bit_ceil" => {
                self.lower_stdc_floor_ceil_u64(name, x, width, mask, op_ty, op_bits)
            }
            _ => return None,
        };

        let final_val = if op_ty == ret_ir_ty {
            out_op_ty
        } else {
            self.emit_cast_val(Operand::Value(out_op_ty), op_ty, ret_ir_ty)
        };
        Some(Operand::Value(final_val))
    }

    fn lower_stdc_first_bit_u64(
        &mut self,
        name: &str,
        x: Operand,
        width: u32,
        mask: u64,
        op_ty: IrType,
        op_bits: u32,
    ) -> Value {
        let (target_val, sentinel, leading) = match name {
            "__builtin_stdc_first_leading_one" => (x, 0u64, true),
            "__builtin_stdc_first_leading_zero" => (
                self.emit_not_masked(x, width, mask, op_ty, op_bits),
                mask,
                true,
            ),
            "__builtin_stdc_first_trailing_one" => (x, 0u64, false),
            _ => (
                self.emit_not_masked(x, width, mask, op_ty, op_bits),
                mask,
                false,
            ),
        };
        let cnt = if leading {
            let safe = self.emit_binop_val(IrBinOp::Or, target_val, Self::uconst(op_ty, 1), op_ty);
            let raw = self.emit_unary_val(IrUnaryOp::ClzNonZero, Operand::Value(safe), op_ty);
            if width < op_bits {
                self.emit_binop_val(
                    IrBinOp::Sub,
                    Operand::Value(raw),
                    Self::uconst(op_ty, (op_bits - width) as u64),
                    op_ty,
                )
            } else {
                raw
            }
        } else {
            let safe = self.emit_binop_val(
                IrBinOp::Or,
                target_val,
                Self::uconst(op_ty, 1u64 << (width - 1)),
                op_ty,
            );
            self.emit_unary_val(IrUnaryOp::CtzNonZero, Operand::Value(safe), op_ty)
        };
        let plus1 = self.emit_binop_val(
            IrBinOp::Add,
            Operand::Value(cnt),
            Self::uconst(op_ty, 1),
            op_ty,
        );
        let is_empty = self.emit_cmp_val(IrCmpOp::Eq, x, Self::uconst(op_ty, sentinel), op_ty);
        self.emit_select_val(
            Operand::Value(is_empty),
            Self::uconst(op_ty, 0),
            Operand::Value(plus1),
            op_ty,
        )
    }

    /// Branchless `bit_floor` and `bit_ceil` using `ClzNonZero(v | 1)`:
    /// `clz(v | 1) == clz(v)` for every `v >= 1`, while `| 1` guarantees a
    /// nonzero input for `x == 0` / `x == 1`, unlocking direct `bsr` emission
    /// (`(op_bits - 1) - ClzNonZero(safe) => bsr(safe)`) with zero branches!
    fn lower_stdc_floor_ceil_u64(
        &mut self,
        name: &str,
        x: Operand,
        width: u32,
        mask: u64,
        op_ty: IrType,
        op_bits: u32,
    ) -> Value {
        if name == "__builtin_stdc_bit_floor" {
            let safe = self.emit_binop_val(IrBinOp::Or, x, Self::uconst(op_ty, 1), op_ty);
            let clz = self.emit_unary_val(IrUnaryOp::ClzNonZero, Operand::Value(safe), op_ty);
            let highbit = self.emit_binop_val(
                IrBinOp::Sub,
                Self::uconst(op_ty, (op_bits - 1) as u64),
                Operand::Value(clz),
                op_ty,
            );
            let shifted = self.emit_binop_val(
                IrBinOp::Shl,
                Self::uconst(op_ty, 1),
                Operand::Value(highbit),
                op_ty,
            );
            let is_zero = self.emit_cmp_val(IrCmpOp::Eq, x, Self::uconst(op_ty, 0), op_ty);
            self.emit_select_val(
                Operand::Value(is_zero),
                Self::uconst(op_ty, 0),
                Operand::Value(shifted),
                op_ty,
            )
        } else {
            let xm1_raw = self.emit_binop_val(IrBinOp::Sub, x, Self::uconst(op_ty, 1), op_ty);
            let xm1 = if width < op_bits {
                self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(xm1_raw),
                    Self::uconst(op_ty, mask),
                    op_ty,
                )
            } else {
                xm1_raw
            };
            let safe = self.emit_binop_val(
                IrBinOp::Or,
                Operand::Value(xm1),
                Self::uconst(op_ty, 1),
                op_ty,
            );
            let clz = self.emit_unary_val(IrUnaryOp::ClzNonZero, Operand::Value(safe), op_ty);
            let highbit = self.emit_binop_val(
                IrBinOp::Sub,
                Self::uconst(op_ty, (op_bits - 1) as u64),
                Operand::Value(clz),
                op_ty,
            );
            let shifted = self.emit_binop_val(
                IrBinOp::Shl,
                Self::uconst(op_ty, 2),
                Operand::Value(highbit),
                op_ty,
            );
            let wrapped = if width < op_bits {
                self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(shifted),
                    Self::uconst(op_ty, mask),
                    op_ty,
                )
            } else {
                shifted
            };
            let le_one = self.emit_cmp_val(IrCmpOp::Ule, x, Self::uconst(op_ty, 1), op_ty);
            self.emit_select_val(
                Operand::Value(le_one),
                Self::uconst(op_ty, 1 & mask),
                Operand::Value(wrapped),
                op_ty,
            )
        }
    }

    fn lower_stdc_rotate(
        &mut self,
        name: &str,
        args: &[Expr],
        width: u32,
        ret_ir_ty: IrType,
    ) -> Operand {
        let is_left = name == "__builtin_stdc_rotate_left";
        if self.target_supports_hw_rotate(width) {
            let rot_ty = match width {
                8 => IrType::U8,
                16 => IrType::U16,
                32 => IrType::U32,
                _ => IrType::U64,
            };
            let val = self.lower_expr_with_type(&args[0], rot_ty);
            let cnt = self.lower_expr_with_type(&args[1], rot_ty);
            let op = if is_left {
                IrBinOp::RotateLeft
            } else {
                IrBinOp::RotateRight
            };
            let res = self.emit_binop_val(op, val, cnt, rot_ty);
            let casted = if rot_ty == ret_ir_ty {
                res
            } else {
                self.emit_cast_val(Operand::Value(res), rot_ty, ret_ir_ty)
            };
            return Operand::Value(casted);
        }

        let op_ty = if width <= 32 {
            IrType::U32
        } else {
            IrType::U64
        };
        let op_bits = if width <= 32 { 32u32 } else { 64u32 };
        let mask = if width == 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        let raw_x = self.lower_expr_with_type(&args[0], op_ty);
        let x = if width < op_bits {
            Operand::Value(self.emit_binop_val(
                IrBinOp::And,
                raw_x,
                Self::uconst(op_ty, mask),
                op_ty,
            ))
        } else {
            raw_x
        };

        let cnt_ct = self.expr_ctype(&args[1]);
        let cnt_signed = !cnt_ct.is_unsigned();
        let cnt_ir = if cnt_signed {
            if op_ty == IrType::U64 {
                IrType::I64
            } else {
                IrType::I32
            }
        } else {
            op_ty
        };
        let raw_cnt = self.lower_expr_with_type(&args[1], cnt_ir);
        let r = if width.is_power_of_two() {
            let cnt_u = if cnt_ir == op_ty {
                raw_cnt
            } else {
                Operand::Value(self.emit_cast_val(raw_cnt, cnt_ir, op_ty))
            };
            self.emit_binop_val(
                IrBinOp::And,
                cnt_u,
                Self::uconst(op_ty, (width - 1) as u64),
                op_ty,
            )
        } else if cnt_signed {
            let w_c = Self::uconst(cnt_ir, width as u64);
            let rem = self.emit_binop_val(IrBinOp::SRem, raw_cnt, w_c, cnt_ir);
            let plus = self.emit_binop_val(IrBinOp::Add, Operand::Value(rem), w_c, cnt_ir);
            let pos = self.emit_binop_val(IrBinOp::SRem, Operand::Value(plus), w_c, cnt_ir);
            self.emit_cast_val(Operand::Value(pos), cnt_ir, op_ty)
        } else {
            self.emit_binop_val(
                IrBinOp::URem,
                raw_cnt,
                Self::uconst(op_ty, width as u64),
                op_ty,
            )
        };

        let w_minus_r = self.emit_binop_val(
            IrBinOp::Sub,
            Self::uconst(op_ty, width as u64),
            Operand::Value(r),
            op_ty,
        );
        let inv_r = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(w_minus_r),
            Self::uconst(op_ty, (op_bits - 1) as u64),
            op_ty,
        );
        let (shl_amt, shr_amt) = if is_left {
            (Operand::Value(r), Operand::Value(inv_r))
        } else {
            (Operand::Value(inv_r), Operand::Value(r))
        };
        let hi = self.emit_binop_val(IrBinOp::Shl, x, shl_amt, op_ty);
        let lo = self.emit_binop_val(IrBinOp::LShr, x, shr_amt, op_ty);
        let combined =
            self.emit_binop_val(IrBinOp::Or, Operand::Value(hi), Operand::Value(lo), op_ty);
        let masked = if width < op_bits {
            self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(combined),
                Self::uconst(op_ty, mask),
                op_ty,
            )
        } else {
            combined
        };
        let is_zero_rot = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(r),
            Self::uconst(op_ty, 0),
            op_ty,
        );
        let sel = self.emit_select_val(
            Operand::Value(is_zero_rot),
            x,
            Operand::Value(masked),
            op_ty,
        );
        let final_val = if op_ty == ret_ir_ty {
            sel
        } else {
            self.emit_cast_val(Operand::Value(sel), op_ty, ret_ir_ty)
        };
        Operand::Value(final_val)
    }

    fn emit_clz_u128(&mut self, hi: Value, lo: Value, hi_bits: u32) -> Value {
        let clz_hi_raw = self.emit_unary_val(IrUnaryOp::Clz, Operand::Value(hi), IrType::U64);
        let clz_hi = if hi_bits < 64 {
            self.emit_binop_val(
                IrBinOp::Sub,
                Operand::Value(clz_hi_raw),
                Self::uconst(IrType::U64, (64 - hi_bits) as u64),
                IrType::U64,
            )
        } else {
            clz_hi_raw
        };
        let clz_lo = self.emit_unary_val(IrUnaryOp::Clz, Operand::Value(lo), IrType::U64);
        let clz_lo_plus = self.emit_binop_val(
            IrBinOp::Add,
            Operand::Value(clz_lo),
            Self::uconst(IrType::U64, hi_bits as u64),
            IrType::U64,
        );
        let hi_z = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(hi),
            Self::uconst(IrType::U64, 0),
            IrType::U64,
        );
        let sel = self.emit_select_val(
            Operand::Value(hi_z),
            Operand::Value(clz_lo_plus),
            Operand::Value(clz_hi),
            IrType::U64,
        );
        self.emit_cast_val(Operand::Value(sel), IrType::U64, IrType::U32)
    }

    fn emit_ctz_u128(&mut self, hi: Value, lo: Value, hi_bits: u32) -> Value {
        let hi_bounded = if hi_bits < 64 {
            Operand::Value(self.emit_binop_val(
                IrBinOp::Or,
                Operand::Value(hi),
                Self::uconst(IrType::U64, 1u64 << hi_bits),
                IrType::U64,
            ))
        } else {
            Operand::Value(hi)
        };
        let ctz_hi = self.emit_unary_val(IrUnaryOp::Ctz, hi_bounded, IrType::U64);
        let ctz_hi_plus = self.emit_binop_val(
            IrBinOp::Add,
            Operand::Value(ctz_hi),
            Self::uconst(IrType::U64, 64),
            IrType::U64,
        );
        let ctz_lo = self.emit_unary_val(IrUnaryOp::Ctz, Operand::Value(lo), IrType::U64);
        let lo_z = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(lo),
            Self::uconst(IrType::U64, 0),
            IrType::U64,
        );
        let sel = self.emit_select_val(
            Operand::Value(lo_z),
            Operand::Value(ctz_hi_plus),
            Operand::Value(ctz_lo),
            IrType::U64,
        );
        self.emit_cast_val(Operand::Value(sel), IrType::U64, IrType::U32)
    }

    /// 128-bit runtime lowering for `unsigned __int128`.
    fn lower_stdc_bit_u128(
        &mut self,
        name: &str,
        args: &[Expr],
        width: u32,
        ret_ir_ty: IrType,
    ) -> Option<Operand> {
        let op_ty = IrType::U128;
        let raw = self.lower_expr_with_type(&args[0], op_ty);
        let mask128 = if width == 128 {
            u128::MAX
        } else {
            (1u128 << width) - 1
        };
        let x = if width < 128 {
            Operand::Value(self.emit_binop_val(
                IrBinOp::And,
                raw,
                Operand::Const(IrConst::I128(mask128 as i128)),
                op_ty,
            ))
        } else {
            raw
        };

        if matches!(
            name,
            "__builtin_stdc_rotate_left" | "__builtin_stdc_rotate_right"
        ) {
            return Some(self.lower_stdc_rotate_u128(name, args, x, width, mask128));
        }
        if matches!(name, "__builtin_stdc_bit_floor" | "__builtin_stdc_bit_ceil") {
            return Some(self.lower_stdc_floor_ceil_u128(name, x, width, mask128));
        }

        let hi_bits = width - 64;
        let lo64 = self.emit_cast_val(x, op_ty, IrType::U64);
        let shifted_hi =
            self.emit_binop_val(IrBinOp::LShr, x, Operand::Const(IrConst::I128(64)), op_ty);
        let hi64 = self.emit_cast_val(Operand::Value(shifted_hi), op_ty, IrType::U64);
        let hi_mask = if hi_bits == 64 {
            u64::MAX
        } else {
            (1u64 << hi_bits) - 1
        };

        match name {
            "__builtin_stdc_leading_zeros" => {
                Some(Operand::Value(self.emit_clz_u128(hi64, lo64, hi_bits)))
            }
            "__builtin_stdc_trailing_zeros" => {
                Some(Operand::Value(self.emit_ctz_u128(hi64, lo64, hi_bits)))
            }
            "__builtin_stdc_count_ones" | "__builtin_stdc_count_zeros" => {
                let p_hi =
                    self.emit_unary_val(IrUnaryOp::Popcount, Operand::Value(hi64), IrType::U64);
                let p_lo =
                    self.emit_unary_val(IrUnaryOp::Popcount, Operand::Value(lo64), IrType::U64);
                let sum = self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(p_hi),
                    Operand::Value(p_lo),
                    IrType::U64,
                );
                let res = if name == "__builtin_stdc_count_zeros" {
                    self.emit_binop_val(
                        IrBinOp::Sub,
                        Self::uconst(IrType::U64, width as u64),
                        Operand::Value(sum),
                        IrType::U64,
                    )
                } else {
                    sum
                };
                Some(Operand::Value(self.emit_cast_val(
                    Operand::Value(res),
                    IrType::U64,
                    IrType::U32,
                )))
            }
            "__builtin_stdc_has_single_bit" => {
                let xm1 =
                    self.emit_binop_val(IrBinOp::Sub, x, Operand::Const(IrConst::I128(1)), op_ty);
                let clr = self.emit_binop_val(IrBinOp::And, x, Operand::Value(xm1), op_ty);
                let z = self.emit_cmp_val(
                    IrCmpOp::Eq,
                    Operand::Value(clr),
                    Operand::Const(IrConst::I128(0)),
                    op_ty,
                );
                let nz = self.emit_cmp_val(IrCmpOp::Ne, x, Operand::Const(IrConst::I128(0)), op_ty);
                let b = self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(nz),
                    Operand::Value(z),
                    IrType::I32,
                );
                Some(Operand::Value(self.emit_cast_val(
                    Operand::Value(b),
                    IrType::I32,
                    ret_ir_ty,
                )))
            }
            "__builtin_stdc_bit_width" => {
                let lz = self.emit_clz_u128(hi64, lo64, hi_bits);
                Some(Operand::Value(self.emit_binop_val(
                    IrBinOp::Sub,
                    Self::uconst(IrType::U32, width as u64),
                    Operand::Value(lz),
                    IrType::U32,
                )))
            }
            "__builtin_stdc_leading_ones" | "__builtin_stdc_first_leading_zero" => {
                let inv_hi = self.emit_binop_val(
                    IrBinOp::Xor,
                    Operand::Value(hi64),
                    Self::uconst(IrType::U64, hi_mask),
                    IrType::U64,
                );
                let inv_lo = self.emit_unary_val(IrUnaryOp::Not, Operand::Value(lo64), IrType::U64);
                let lo_cnt = self.emit_clz_u128(inv_hi, inv_lo, hi_bits);
                if name == "__builtin_stdc_leading_ones" {
                    Some(Operand::Value(lo_cnt))
                } else {
                    let p1 = self.emit_binop_val(
                        IrBinOp::Add,
                        Operand::Value(lo_cnt),
                        Self::uconst(IrType::U32, 1),
                        IrType::U32,
                    );
                    let all1 = self.emit_cmp_val(
                        IrCmpOp::Eq,
                        x,
                        Operand::Const(IrConst::I128(mask128 as i128)),
                        op_ty,
                    );
                    Some(Operand::Value(self.emit_select_val(
                        Operand::Value(all1),
                        Self::uconst(IrType::U32, 0),
                        Operand::Value(p1),
                        IrType::U32,
                    )))
                }
            }
            "__builtin_stdc_trailing_ones" | "__builtin_stdc_first_trailing_zero" => {
                let inv_hi = self.emit_binop_val(
                    IrBinOp::Xor,
                    Operand::Value(hi64),
                    Self::uconst(IrType::U64, hi_mask),
                    IrType::U64,
                );
                let inv_lo = self.emit_unary_val(IrUnaryOp::Not, Operand::Value(lo64), IrType::U64);
                let to_cnt = self.emit_ctz_u128(inv_hi, inv_lo, hi_bits);
                if name == "__builtin_stdc_trailing_ones" {
                    Some(Operand::Value(to_cnt))
                } else {
                    let p1 = self.emit_binop_val(
                        IrBinOp::Add,
                        Operand::Value(to_cnt),
                        Self::uconst(IrType::U32, 1),
                        IrType::U32,
                    );
                    let all1 = self.emit_cmp_val(
                        IrCmpOp::Eq,
                        x,
                        Operand::Const(IrConst::I128(mask128 as i128)),
                        op_ty,
                    );
                    Some(Operand::Value(self.emit_select_val(
                        Operand::Value(all1),
                        Self::uconst(IrType::U32, 0),
                        Operand::Value(p1),
                        IrType::U32,
                    )))
                }
            }
            "__builtin_stdc_first_leading_one" | "__builtin_stdc_first_trailing_one" => {
                let cnt = if name == "__builtin_stdc_first_leading_one" {
                    self.emit_clz_u128(hi64, lo64, hi_bits)
                } else {
                    self.emit_ctz_u128(hi64, lo64, hi_bits)
                };
                let p1 = self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(cnt),
                    Self::uconst(IrType::U32, 1),
                    IrType::U32,
                );
                let z = self.emit_cmp_val(IrCmpOp::Eq, x, Operand::Const(IrConst::I128(0)), op_ty);
                Some(Operand::Value(self.emit_select_val(
                    Operand::Value(z),
                    Self::uconst(IrType::U32, 0),
                    Operand::Value(p1),
                    IrType::U32,
                )))
            }
            _ => None,
        }
    }

    fn lower_stdc_rotate_u128(
        &mut self,
        name: &str,
        args: &[Expr],
        x: Operand,
        width: u32,
        mask128: u128,
    ) -> Operand {
        let op_ty = IrType::U128;
        let is_left = name == "__builtin_stdc_rotate_left";
        let cnt = self.lower_expr_with_type(&args[1], op_ty);
        let r = if width == 128 {
            self.emit_binop_val(IrBinOp::And, cnt, Operand::Const(IrConst::I128(127)), op_ty)
        } else {
            self.emit_binop_val(
                IrBinOp::URem,
                cnt,
                Operand::Const(IrConst::I128(width as i128)),
                op_ty,
            )
        };
        let inv_raw = self.emit_binop_val(
            IrBinOp::Sub,
            Operand::Const(IrConst::I128(width as i128)),
            Operand::Value(r),
            op_ty,
        );
        let inv = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(inv_raw),
            Operand::Const(IrConst::I128(127)),
            op_ty,
        );
        let (shl_a, shr_a) = if is_left {
            (Operand::Value(r), Operand::Value(inv))
        } else {
            (Operand::Value(inv), Operand::Value(r))
        };
        let hi = self.emit_binop_val(IrBinOp::Shl, x, shl_a, op_ty);
        let lo = self.emit_binop_val(IrBinOp::LShr, x, shr_a, op_ty);
        let or = self.emit_binop_val(IrBinOp::Or, Operand::Value(hi), Operand::Value(lo), op_ty);
        let masked = if width < 128 {
            self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(or),
                Operand::Const(IrConst::I128(mask128 as i128)),
                op_ty,
            )
        } else {
            or
        };
        let z = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(r),
            Operand::Const(IrConst::I128(0)),
            op_ty,
        );
        Operand::Value(self.emit_select_u128(Operand::Value(z), x, Operand::Value(masked)))
    }

    fn lower_stdc_floor_ceil_u128(
        &mut self,
        name: &str,
        x: Operand,
        width: u32,
        mask128: u128,
    ) -> Operand {
        let op_ty = IrType::U128;
        let hi_bits = width - 64;
        if name == "__builtin_stdc_bit_floor" {
            let lo64 = self.emit_cast_val(x, op_ty, IrType::U64);
            let sh_hi =
                self.emit_binop_val(IrBinOp::LShr, x, Operand::Const(IrConst::I128(64)), op_ty);
            let hi64 = self.emit_cast_val(Operand::Value(sh_hi), op_ty, IrType::U64);
            let lz = self.emit_clz_u128(hi64, lo64, hi_bits);
            let lz128 = self.emit_cast_val(Operand::Value(lz), IrType::U32, op_ty);
            let sh_raw = self.emit_binop_val(
                IrBinOp::Sub,
                Operand::Const(IrConst::I128((width - 1) as i128)),
                Operand::Value(lz128),
                op_ty,
            );
            let sh = self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(sh_raw),
                Operand::Const(IrConst::I128(127)),
                op_ty,
            );
            let pow = self.emit_binop_val(
                IrBinOp::Shl,
                Operand::Const(IrConst::I128(1)),
                Operand::Value(sh),
                op_ty,
            );
            let z = self.emit_cmp_val(IrCmpOp::Eq, x, Operand::Const(IrConst::I128(0)), op_ty);
            Operand::Value(self.emit_select_u128(
                Operand::Value(z),
                Operand::Const(IrConst::I128(0)),
                Operand::Value(pow),
            ))
        } else {
            let xm1_raw =
                self.emit_binop_val(IrBinOp::Sub, x, Operand::Const(IrConst::I128(1)), op_ty);
            let xm1 = if width < 128 {
                self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(xm1_raw),
                    Operand::Const(IrConst::I128(mask128 as i128)),
                    op_ty,
                )
            } else {
                xm1_raw
            };
            let xm1_lo = self.emit_cast_val(Operand::Value(xm1), op_ty, IrType::U64);
            let xm1_hi_sh = self.emit_binop_val(
                IrBinOp::LShr,
                Operand::Value(xm1),
                Operand::Const(IrConst::I128(64)),
                op_ty,
            );
            let xm1_hi = self.emit_cast_val(Operand::Value(xm1_hi_sh), op_ty, IrType::U64);
            let lz = self.emit_clz_u128(xm1_hi, xm1_lo, hi_bits);
            let lz128 = self.emit_cast_val(Operand::Value(lz), IrType::U32, op_ty);
            let sh_raw = self.emit_binop_val(
                IrBinOp::Sub,
                Operand::Const(IrConst::I128((width - 1) as i128)),
                Operand::Value(lz128),
                op_ty,
            );
            let sh = self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(sh_raw),
                Operand::Const(IrConst::I128(127)),
                op_ty,
            );
            let pow_raw = self.emit_binop_val(
                IrBinOp::Shl,
                Operand::Const(IrConst::I128(2)),
                Operand::Value(sh),
                op_ty,
            );
            let pow = if width < 128 {
                self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(pow_raw),
                    Operand::Const(IrConst::I128(mask128 as i128)),
                    op_ty,
                )
            } else {
                pow_raw
            };
            let le1 = self.emit_cmp_val(IrCmpOp::Ule, x, Operand::Const(IrConst::I128(1)), op_ty);
            Operand::Value(self.emit_select_u128(
                Operand::Value(le1),
                Operand::Const(IrConst::I128(1)),
                Operand::Value(pow),
            ))
        }
    }

    /// Resolve `(count_field_name, count_field_ctype, fam_field_ctype)` when
    /// `expr` accesses a struct member annotated with `counted_by`.
    pub(super) fn resolve_counted_by_member_expr(
        &self,
        expr: &Expr,
    ) -> Option<(String, CType, CType)> {
        let (base_expr, field_name, is_ptr) = match expr {
            Expr::MemberAccess(b, f, _) => (b.as_ref(), f.as_str(), false),
            Expr::PointerMemberAccess(b, f, _) => (b.as_ref(), f.as_str(), true),
            _ => return None,
        };
        let raw_base = self.get_expr_ctype(base_expr)?;
        let record_ct = if is_ptr {
            match raw_base {
                CType::Pointer(inner, _) | CType::Array(inner, _) => *inner,
                _ => return None,
            }
        } else {
            raw_base
        };
        let record_ct = self.resolve_forward_declared_ctype(record_ct);
        let key = match &record_ct {
            CType::Struct(k) => k,
            _ => return None,
        };
        let layouts = self.types.borrow_struct_layouts();
        let layout = layouts.get(key.as_ref())?;
        layout.field_counted_by(field_name, &*layouts)
    }

    pub(super) fn lower_counted_by_ref(&mut self, args: &[Expr]) -> Option<Operand> {
        let arg0 = args.first()?;
        let Some((cnt_member, _, _)) = self.resolve_counted_by_member_expr(arg0) else {
            return Some(Operand::Const(Self::ptr_zero_const()));
        };
        let addr_expr = match arg0 {
            Expr::MemberAccess(base, _, span) => Expr::AddressOf(
                Box::new(Expr::MemberAccess(base.clone(), cnt_member, *span)),
                *span,
            ),
            Expr::PointerMemberAccess(base, _, span) => Expr::AddressOf(
                Box::new(Expr::PointerMemberAccess(base.clone(), cnt_member, *span)),
                *span,
            ),
            _ => return Some(Operand::Const(Self::ptr_zero_const())),
        };
        Some(self.lower_expr(&addr_expr))
    }

    pub(super) fn expr_may_have_side_effects(expr: &Expr) -> bool {
        match expr {
            Expr::IntLiteral(..)
            | Expr::UIntLiteral(..)
            | Expr::LongLiteral(..)
            | Expr::ULongLiteral(..)
            | Expr::LongLongLiteral(..)
            | Expr::ULongLongLiteral(..)
            | Expr::FloatLiteral(..)
            | Expr::FloatLiteralF32(..)
            | Expr::FloatLiteralLongDouble(..)
            | Expr::FloatLiteralF128(..)
            | Expr::FloatLiteralHalf(..)
            | Expr::FloatLiteralDecimal(..)
            | Expr::CharLiteral(..)
            | Expr::StringLiteral(..)
            | Expr::WideStringLiteral(..)
            | Expr::Char16StringLiteral(..)
            | Expr::Identifier(..)
            | Expr::Sizeof(..)
            | Expr::Countof(..)
            | Expr::Alignof(..)
            | Expr::GnuAlignof(..)
            | Expr::AlignofExpr(..)
            | Expr::GnuAlignofExpr(..)
            | Expr::BuiltinTypesCompatibleP(..)
            | Expr::LabelAddr(..) => false,
            Expr::AddressOf(inner, _)
            | Expr::Deref(inner, _)
            | Expr::MemberAccess(inner, _, _)
            | Expr::PointerMemberAccess(inner, _, _)
            | Expr::Cast(_, inner, _, _) => Self::expr_may_have_side_effects(inner),
            Expr::UnaryOp(op, inner, _) => {
                use crate::frontend::parser::ast::UnaryOp;
                if matches!(op, UnaryOp::PreInc | UnaryOp::PreDec) {
                    true
                } else {
                    Self::expr_may_have_side_effects(inner)
                }
            }
            Expr::BinaryOp(_, l, r, _) | Expr::ArraySubscript(l, r, _) => {
                Self::expr_may_have_side_effects(l) || Self::expr_may_have_side_effects(r)
            }
            Expr::Conditional(c, t, e, _) => {
                Self::expr_may_have_side_effects(c)
                    || Self::expr_may_have_side_effects(t)
                    || Self::expr_may_have_side_effects(e)
            }
            _ => true,
        }
    }

    pub(super) fn lower_object_size_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Option<Operand> {
        let is_dynamic = name == "__builtin_dynamic_object_size";
        let obj_type = if args.len() >= 2 {
            self.eval_const_expr(&args[1])
                .and_then(|c| c.to_i64())
                .unwrap_or(0)
        } else {
            0
        };
        let sz_ty = Self::size_ir_type();
        let unknown_op = if obj_type == 2 || obj_type == 3 {
            Self::uconst(sz_ty, 0)
        } else {
            Self::uconst(sz_ty, u64::MAX)
        };

        let Some(arg0) = args.first() else {
            return Some(unknown_op);
        };

        if !is_dynamic && Self::expr_may_have_side_effects(arg0) {
            return Some(unknown_op);
        }

        if is_dynamic {
            if let Some(dyn_bytes) = self.try_lower_counted_by_object_size(arg0, sz_ty) {
                return Some(dyn_bytes);
            }
        }

        let subobj = (obj_type & 1) != 0;
        if let Some(known_sz) = self.try_static_object_size(arg0, subobj) {
            return Some(Self::uconst(sz_ty, known_sz as u64));
        }

        Some(unknown_op)
    }

    fn try_lower_counted_by_object_size(&mut self, arg0: &Expr, sz_ty: IrType) -> Option<Operand> {
        let (
            cnt_member,
            cnt_ct,
            CType::Array(elem_ct, None | Some(0)) | CType::Pointer(elem_ct, _),
        ) = self.resolve_counted_by_member_expr(arg0)?
        else {
            return None;
        };
        let elem_sz = elem_ct.size_ctx(&*self.types.borrow_struct_layouts());
        let cnt_expr = match arg0 {
            Expr::MemberAccess(base, _, span) => {
                Expr::MemberAccess(base.clone(), cnt_member, *span)
            }
            Expr::PointerMemberAccess(base, _, span) => {
                Expr::PointerMemberAccess(base.clone(), cnt_member, *span)
            }
            _ => return None,
        };
        if elem_sz == 0 {
            let _ = self.lower_expr(&cnt_expr);
            return Some(Self::uconst(sz_ty, 0));
        }
        let cnt_op = if cnt_ct.is_unsigned() {
            self.lower_expr_with_type(&cnt_expr, sz_ty)
        } else {
            let signed_sz_ty = if sz_ty == IrType::U64 {
                IrType::I64
            } else {
                IrType::I32
            };
            let raw_cnt = self.lower_expr_with_type(&cnt_expr, signed_sz_ty);
            let neg = self.emit_cmp_val(
                IrCmpOp::Slt,
                raw_cnt,
                Self::uconst(signed_sz_ty, 0),
                signed_sz_ty,
            );
            let clamped = self.emit_select_val(
                Operand::Value(neg),
                Self::uconst(signed_sz_ty, 0),
                raw_cnt,
                signed_sz_ty,
            );
            Operand::Value(self.emit_cast_val(Operand::Value(clamped), signed_sz_ty, sz_ty))
        };
        let bytes = if elem_sz == 1 {
            cnt_op
        } else if elem_sz.is_power_of_two() {
            let sh = elem_sz.trailing_zeros() as u64;
            Operand::Value(self.emit_binop_val(
                IrBinOp::Shl,
                cnt_op,
                Self::uconst(sz_ty, sh),
                sz_ty,
            ))
        } else {
            Operand::Value(self.emit_binop_val(
                IrBinOp::Mul,
                cnt_op,
                Self::uconst(sz_ty, elem_sz as u64),
                sz_ty,
            ))
        };
        Some(bytes)
    }

    pub(super) fn try_static_object_size(&self, arg0: &Expr, subobj: bool) -> Option<usize> {
        if Self::expr_may_have_side_effects(arg0) {
            return None;
        }
        let (total_sz, byte_off) = self.resolve_ptr_object_and_offset(arg0, subobj)?;
        if byte_off < 0 {
            return None;
        }
        total_sz.checked_sub(byte_off as usize)
    }

    /// Resolve a pointer expression to `(enclosing_object_bytes, byte_offset_within_object)`.
    /// When `subobj` is true (`type` 1 or 3), stops at the innermost array/member subobject;
    /// when `subobj` is false (`type` 0 or 2), walks out to the outermost complete object.
    fn resolve_ptr_object_and_offset(&self, expr: &Expr, subobj: bool) -> Option<(usize, i128)> {
        match expr {
            Expr::StringLiteral(s, _) => Some((s.len() + 1, 0)),
            Expr::WideStringLiteral(s, _) => Some(((s.chars().count() + 1) * 4, 0)),
            Expr::Char16StringLiteral(s, _) => Some(((s.encode_utf16().count() + 1) * 2, 0)),
            Expr::Cast(_, inner, _, _) => self.resolve_ptr_object_and_offset(inner, subobj),
            Expr::AddressOf(inner, _) => self.resolve_lvalue_object_and_offset(inner, subobj),
            Expr::Identifier(..) | Expr::MemberAccess(..) | Expr::PointerMemberAccess(..) => {
                match self.get_expr_ctype(expr)? {
                    CType::Array(..) => self.resolve_lvalue_object_and_offset(expr, subobj),
                    _ => None,
                }
            }
            Expr::BinaryOp(op @ (BinOp::Add | BinOp::Sub), lhs, rhs, _) => {
                let (ptr_side, idx_val) = if let Some(c) = self.eval_const_expr(rhs)
                    && let Some(v) = c.to_i64()
                {
                    (
                        lhs.as_ref(),
                        if *op == BinOp::Sub {
                            -(v as i128)
                        } else {
                            v as i128
                        },
                    )
                } else if *op == BinOp::Add
                    && let Some(c) = self.eval_const_expr(lhs)
                    && let Some(v) = c.to_i64()
                {
                    (rhs.as_ref(), v as i128)
                } else {
                    return None;
                };
                let stride = match self.get_expr_ctype(ptr_side)? {
                    CType::Pointer(pointee, _) | CType::Array(pointee, _) => {
                        if matches!(pointee.as_ref(), CType::Void) {
                            1usize
                        } else {
                            pointee.size_ctx(&*self.types.borrow_struct_layouts())
                        }
                    }
                    _ => return None,
                };
                let (tot, base_off) = self.resolve_ptr_object_and_offset(ptr_side, subobj)?;
                let delta = idx_val.checked_mul(stride as i128)?;
                Some((tot, base_off.checked_add(delta)?))
            }
            _ => None,
        }
    }

    fn resolve_lvalue_object_and_offset(&self, expr: &Expr, subobj: bool) -> Option<(usize, i128)> {
        match expr {
            Expr::Identifier(..) => {
                let ct = self.get_expr_ctype(expr)?;
                let sz = ct.size_ctx(&*self.types.borrow_struct_layouts());
                (sz > 0).then_some((sz, 0))
            }
            Expr::MemberAccess(base, field, _) => {
                if subobj {
                    let ct = self.get_expr_ctype(expr)?;
                    let sz = ct.size_ctx(&*self.types.borrow_struct_layouts());
                    return (sz > 0).then_some((sz, 0));
                }
                let base_ct = self.get_expr_ctype(base)?;
                let key = match &base_ct {
                    CType::Struct(k) | CType::Union(k) => k.as_ref(),
                    _ => return None,
                };
                let field_off = {
                    let layouts = self.types.borrow_struct_layouts();
                    let layout = layouts.get(key)?;
                    if layout
                        .fields
                        .last()
                        .is_some_and(|f| matches!(f.ty, CType::Array(_, None | Some(0))))
                    {
                        return None;
                    }
                    let (off, _) = layout.field_offset(field, &*layouts)?;
                    off as i128
                };
                let (tot, base_off) = self.resolve_lvalue_object_and_offset(base, false)?;
                Some((tot, base_off.checked_add(field_off)?))
            }
            Expr::PointerMemberAccess(_, _, _) => {
                if !subobj {
                    return None;
                }
                let ct = self.get_expr_ctype(expr)?;
                let sz = ct.size_ctx(&*self.types.borrow_struct_layouts());
                (sz > 0).then_some((sz, 0))
            }
            Expr::ArraySubscript(base, idx, _) => {
                let idx_val = self.eval_const_expr(idx)?.to_i64()? as i128;
                let elem_sz =
                    self.get_expr_ctype(expr)?
                        .size_ctx(&*self.types.borrow_struct_layouts()) as i128;
                let (tot, base_off) = self.resolve_ptr_object_and_offset(base, subobj)?;
                Some((tot, base_off.checked_add(idx_val.checked_mul(elem_sz)?)?))
            }
            _ => None,
        }
    }

    pub(super) fn lower_assoc_barrier(&mut self, args: &[Expr]) -> Option<Operand> {
        let arg = args.first()?;
        let val = self.lower_expr(arg);
        let dest = self.fresh_value();
        self.emit(Instruction::Copy { dest, src: val });
        Some(Operand::Value(dest))
    }

    pub(super) fn lower_clear_padding(&mut self, args: &[Expr]) -> Option<Operand> {
        let Some(arg0) = args.first() else {
            return Some(Operand::Const(IrConst::I64(0)));
        };
        let ptr_ct = self.expr_ctype(arg0);
        let ptr_op = self.lower_expr(arg0);
        let CType::Pointer(pointee, addr_space) = ptr_ct else {
            return Some(Operand::Const(IrConst::I64(0)));
        };
        let pointee = self.resolve_forward_declared_ctype(*pointee);
        let target = self.target;
        let (live_mask, base_align) = {
            let layouts = self.types.borrow_struct_layouts();
            let total_sz = pointee.size_ctx(&*layouts);
            let align = pointee.align_ctx(&*layouts).max(1);
            if total_sz == 0 || total_sz > 65536 {
                return Some(Operand::Const(IrConst::I64(0)));
            }
            let mut mask = vec![0u8; total_sz];
            Self::mark_type_live_bits(&pointee, 0, &mut mask, &*layouts, target);
            (mask, align)
        };
        if live_mask.iter().all(|&m| m == 0xFF) {
            return Some(Operand::Const(IrConst::I64(0)));
        }

        let base_ptr = self.operand_to_value(ptr_op);
        let off_ty = Self::size_ir_type();
        let mut idx = 0usize;
        while idx < live_mask.len() {
            let m = live_mask[idx];
            if m == 0xFF {
                idx += 1;
                continue;
            }
            if m == 0x00 {
                let start = idx;
                while idx < live_mask.len() && live_mask[idx] == 0x00 {
                    idx += 1;
                }
                self.emit_coalesced_zero_stores(
                    base_ptr,
                    start,
                    idx,
                    base_align,
                    addr_space.space,
                    off_ty,
                );
            } else {
                let byte_ptr = self.gep_byte_offset(base_ptr, idx, off_ty);
                let old_val = self.fresh_value();
                self.emit(Instruction::Load {
                    dest: old_val,
                    ptr: byte_ptr,
                    ty: IrType::U8,
                    seg_override: addr_space.space,
                    volatile: false,
                });
                let masked = self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(old_val),
                    Self::uconst(IrType::U8, m as u64),
                    IrType::U8,
                );
                self.emit(Instruction::Store {
                    val: Operand::Value(masked),
                    ptr: byte_ptr,
                    ty: IrType::U8,
                    seg_override: addr_space.space,
                    volatile: false,
                });
                idx += 1;
            }
        }
        Some(Operand::Const(IrConst::I64(0)))
    }

    fn gep_byte_offset(&mut self, base_ptr: Value, cur_off: usize, off_ty: IrType) -> Value {
        if cur_off == 0 {
            base_ptr
        } else {
            let gep = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: gep,
                base: base_ptr,
                offset: Self::uconst(off_ty, cur_off as u64),
                ty: IrType::U8,
            });
            gep
        }
    }

    fn emit_coalesced_zero_stores(
        &mut self,
        base_ptr: Value,
        mut cur_off: usize,
        end_off: usize,
        base_align: usize,
        addr_space: crate::common::types::AddressSpace,
        off_ty: IrType,
    ) {
        while cur_off < end_off {
            let rem = end_off - cur_off;
            let off_align = if cur_off == 0 {
                8
            } else {
                1usize << cur_off.trailing_zeros().min(3)
            };
            let eff_align = base_align.min(off_align);
            let chunk = if rem >= 8 && eff_align >= 8 && !target_is_32bit() {
                8
            } else if rem >= 4 && eff_align >= 4 {
                4
            } else if rem >= 2 && eff_align >= 2 {
                2
            } else {
                1
            };
            let store_ty = match chunk {
                8 => IrType::U64,
                4 => IrType::U32,
                2 => IrType::U16,
                _ => IrType::U8,
            };
            let dst_ptr = self.gep_byte_offset(base_ptr, cur_off, off_ty);
            self.emit(Instruction::Store {
                val: Self::uconst(store_ty, 0),
                ptr: dst_ptr,
                ty: store_ty,
                seg_override: addr_space,
                volatile: false,
            });
            cur_off += chunk;
        }
    }

    fn mark_type_live_bits(
        ct: &CType,
        base_off: usize,
        live_mask: &mut [u8],
        layouts: &dyn StructLayoutProvider,
        target: Target,
    ) {
        match ct {
            CType::Struct(key) | CType::Union(key) => {
                let Some(layout) = layouts.get_struct_layout(key) else {
                    return;
                };
                for f in &layout.fields {
                    if let (Some(bo), Some(bw)) = (f.bit_offset, f.bit_width) {
                        for b in 0..bw {
                            let abs_bit = f.offset * 8 + (bo as usize) + (b as usize);
                            let byte_idx = base_off + abs_bit / 8;
                            if let Some(slot) = live_mask.get_mut(byte_idx) {
                                *slot |= 1u8 << (abs_bit % 8);
                            }
                        }
                    } else {
                        Self::mark_type_live_bits(
                            &f.ty,
                            base_off + f.offset,
                            live_mask,
                            layouts,
                            target,
                        );
                    }
                }
            }
            CType::Array(elem, Some(n)) if *n > 0 => {
                let elem_sz = elem.size_ctx(layouts);
                for idx in 0..*n {
                    Self::mark_type_live_bits(
                        elem,
                        base_off + idx * elem_sz,
                        live_mask,
                        layouts,
                        target,
                    );
                }
            }
            CType::LongDouble if matches!(target, Target::X86_64 | Target::I686) => {
                let end = (base_off + 10).min(live_mask.len());
                if base_off < end {
                    live_mask[base_off..end].fill(0xFF);
                }
            }
            CType::ComplexLongDouble if matches!(target, Target::X86_64 | Target::I686) => {
                let comp_sz = if target == Target::X86_64 { 16 } else { 12 };
                for comp in [0usize, comp_sz] {
                    let s = base_off + comp;
                    let e = (s + 10).min(live_mask.len());
                    if s < e {
                        live_mask[s..e].fill(0xFF);
                    }
                }
            }
            _ => {
                let sz = ct.size_ctx(layouts);
                let end = (base_off + sz).min(live_mask.len());
                if base_off < end {
                    live_mask[base_off..end].fill(0xFF);
                }
            }
        }
    }
}
