//! IR lowering for GCC 16.2 / Clang 23 builtins:
//! - Type-generic bit builtins: `__builtin_clzg`, `__builtin_ctzg`,
//!   `__builtin_popcountg`, `__builtin_parityg`, `__builtin_ffsg`,
//!   `__builtin_clrsbg`
//! - Multi-precision carry/borrow builtins: `__builtin_addcb/s/c/l/ll`,
//!   `__builtin_subcb/s/c/l/ll`
//! - `__builtin_bswap128` and `__builtin_bitreverse{8,16,32,64}`
//! - `__builtin_issignaling`, `__builtin_iseqsig`,
//!   `__builtin_speculation_safe_value`, and `__builtin_bit_cast`

use super::lower::Lowerer;
use crate::backend::Target;
use crate::common::types::{AddressSpace, CType, IrType, target_is_32bit};
use crate::frontend::parser::ast::{Expr, TypeSpecifier};
use crate::frontend::sema::gcc16_builtins;
use crate::ir::reexports::{Instruction, IrBinOp, IrCmpOp, IrConst, IrUnaryOp, Operand, Value};

impl Lowerer {
    fn gcc16_uconst(ty: IrType, v: u64) -> Operand {
        Operand::Const(match ty {
            IrType::I8 | IrType::U8 => IrConst::I8(v as i8),
            IrType::I16 | IrType::U16 => IrConst::I16(v as i16),
            IrType::I32 | IrType::U32 => IrConst::I32(v as i32),
            IrType::I128 | IrType::U128 => IrConst::I128(v as i128),
            _ => IrConst::I64(v as i64),
        })
    }

    fn gcc16_select(&mut self, cond: Operand, t: Operand, f: Operand, ty: IrType) -> Value {
        let dest = self.fresh_value();
        self.emit(Instruction::Select {
            dest,
            cond,
            true_val: t,
            false_val: f,
            ty,
        });
        dest
    }

    /// Lower GCC 16.2 type-generic bit builtins (`clzg`, `ctzg`, `popcountg`,
    /// `parityg`, `ffsg`, `clrsbg`) to branchless IR sequences.
    pub(super) fn lower_gcc16_generic_bit_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Option<Operand> {
        if args.is_empty() || args.len() > 2 {
            return Some(Operand::Const(IrConst::I32(0)));
        }
        let arg0_ct = self
            .get_expr_ctype(&args[0])
            .map(|ct| ct.decayed_value_ctype())
            .unwrap_or(CType::UInt);
        let width = gcc16_builtins::int_bit_width_of_ctype(&arg0_ct).unwrap_or(32);

        // Compile-time constant fast path (when arguments have no side effects).
        if args.iter().all(|a| !Self::expr_may_have_side_effects(a)) {
            if let Some(c) = self.eval_gcc16_generic_bit_const(name, args) {
                return Some(Operand::Const(c));
            }
        }

        let raw = self.lower_expr(&args[0]);
        let arg_ir_ty = self.get_expr_type(&args[0]);
        let fallback_op = if args.len() == 2 {
            let fb_raw = self.lower_expr(&args[1]);
            let fb_ty = self.get_expr_type(&args[1]);
            let fb_i32 = self.emit_cast_val(fb_raw, fb_ty, IrType::I32);
            Some(Operand::Value(fb_i32))
        } else {
            None
        };

        if width == 128 {
            return Some(self.lower_gcc16_generic_bit_128(name, raw, arg_ir_ty, fallback_op));
        }

        let op_ty = if width <= 32 {
            IrType::U32
        } else {
            IrType::U64
        };
        let op_bits = if width <= 32 { 32u32 } else { 64u32 };

        if name == "__builtin_clrsbg" {
            return Some(Operand::Value(
                self.lower_clrsbg_narrow(raw, arg_ir_ty, width, op_ty, op_bits),
            ));
        }

        // Zero-extend `raw` from `width` bits into `op_ty`.
        let val = self.emit_cast_val(raw, arg_ir_ty, op_ty);
        let val = if width < op_bits {
            let mask = (1u64 << width) - 1;
            self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(val),
                Self::gcc16_uconst(op_ty, mask),
                op_ty,
            )
        } else {
            val
        };

        let res_i32 = match name {
            "__builtin_popcountg" => {
                let pc = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest: pc,
                    op: IrUnaryOp::Popcount,
                    src: Operand::Value(val),
                    ty: op_ty,
                });
                self.emit_cast_val(Operand::Value(pc), op_ty, IrType::I32)
            }
            "__builtin_parityg" => {
                let pc = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest: pc,
                    op: IrUnaryOp::Popcount,
                    src: Operand::Value(val),
                    ty: op_ty,
                });
                let pc32 = self.emit_cast_val(Operand::Value(pc), op_ty, IrType::I32);
                self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(pc32),
                    Operand::Const(IrConst::I32(1)),
                    IrType::I32,
                )
            }
            "__builtin_ffsg" => {
                let is_zero = self.emit_cmp_val(
                    IrCmpOp::Eq,
                    Operand::Value(val),
                    Self::gcc16_uconst(op_ty, 0),
                    op_ty,
                );
                let top_bit = 1u64 << (op_bits - 1);
                let safe = self.emit_binop_val(
                    IrBinOp::Or,
                    Operand::Value(val),
                    Self::gcc16_uconst(op_ty, top_bit),
                    op_ty,
                );
                let ctz = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest: ctz,
                    op: IrUnaryOp::CtzNonZero,
                    src: Operand::Value(safe),
                    ty: op_ty,
                });
                let ctz32 = self.emit_cast_val(Operand::Value(ctz), op_ty, IrType::I32);
                let plus1 = self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(ctz32),
                    Operand::Const(IrConst::I32(1)),
                    IrType::I32,
                );
                self.gcc16_select(
                    Operand::Value(is_zero),
                    Operand::Const(IrConst::I32(0)),
                    Operand::Value(plus1),
                    IrType::I32,
                )
            }
            "__builtin_ctzg" => {
                // Sub-word sentinel optimization when fallback == width (`width < 32`):
                // `CtzNonZero(val | (1 << width))` needs zero branches and zero cmovs!
                if width < 32 {
                    if let Some(c_fb) = args
                        .get(1)
                        .and_then(|e| self.eval_const_expr(e))
                        .and_then(|c| c.to_i64())
                    {
                        if c_fb == width as i64 {
                            let sent = self.emit_binop_val(
                                IrBinOp::Or,
                                Operand::Value(val),
                                Self::gcc16_uconst(op_ty, 1u64 << width),
                                op_ty,
                            );
                            let ctz = self.fresh_value();
                            self.emit(Instruction::UnaryOp {
                                dest: ctz,
                                op: IrUnaryOp::CtzNonZero,
                                src: Operand::Value(sent),
                                ty: op_ty,
                            });
                            return Some(Operand::Value(self.emit_cast_val(
                                Operand::Value(ctz),
                                op_ty,
                                IrType::I32,
                            )));
                        }
                    }
                }
                if let Some(fb) = fallback_op {
                    let is_zero = self.emit_cmp_val(
                        IrCmpOp::Eq,
                        Operand::Value(val),
                        Self::gcc16_uconst(op_ty, 0),
                        op_ty,
                    );
                    let top_bit = 1u64 << (op_bits - 1);
                    let safe = self.emit_binop_val(
                        IrBinOp::Or,
                        Operand::Value(val),
                        Self::gcc16_uconst(op_ty, top_bit),
                        op_ty,
                    );
                    let ctz = self.fresh_value();
                    self.emit(Instruction::UnaryOp {
                        dest: ctz,
                        op: IrUnaryOp::CtzNonZero,
                        src: Operand::Value(safe),
                        ty: op_ty,
                    });
                    let ctz32 = self.emit_cast_val(Operand::Value(ctz), op_ty, IrType::I32);
                    self.gcc16_select(
                        Operand::Value(is_zero),
                        fb,
                        Operand::Value(ctz32),
                        IrType::I32,
                    )
                } else {
                    let ctz = self.fresh_value();
                    self.emit(Instruction::UnaryOp {
                        dest: ctz,
                        op: IrUnaryOp::CtzNonZero,
                        src: Operand::Value(val),
                        ty: op_ty,
                    });
                    self.emit_cast_val(Operand::Value(ctz), op_ty, IrType::I32)
                }
            }
            "__builtin_clzg" => {
                let pad = (op_bits - width) as i32;
                if let Some(fb) = fallback_op {
                    let is_zero = self.emit_cmp_val(
                        IrCmpOp::Eq,
                        Operand::Value(val),
                        Self::gcc16_uconst(op_ty, 0),
                        op_ty,
                    );
                    let safe = self.emit_binop_val(
                        IrBinOp::Or,
                        Operand::Value(val),
                        Self::gcc16_uconst(op_ty, 1),
                        op_ty,
                    );
                    let clz = self.fresh_value();
                    self.emit(Instruction::UnaryOp {
                        dest: clz,
                        op: IrUnaryOp::ClzNonZero,
                        src: Operand::Value(safe),
                        ty: op_ty,
                    });
                    let clz32 = self.emit_cast_val(Operand::Value(clz), op_ty, IrType::I32);
                    let adj = if pad == 0 {
                        clz32
                    } else {
                        self.emit_binop_val(
                            IrBinOp::Sub,
                            Operand::Value(clz32),
                            Operand::Const(IrConst::I32(pad)),
                            IrType::I32,
                        )
                    };
                    self.gcc16_select(
                        Operand::Value(is_zero),
                        fb,
                        Operand::Value(adj),
                        IrType::I32,
                    )
                } else {
                    let clz = self.fresh_value();
                    self.emit(Instruction::UnaryOp {
                        dest: clz,
                        op: IrUnaryOp::ClzNonZero,
                        src: Operand::Value(val),
                        ty: op_ty,
                    });
                    let clz32 = self.emit_cast_val(Operand::Value(clz), op_ty, IrType::I32);
                    if pad == 0 {
                        clz32
                    } else {
                        self.emit_binop_val(
                            IrBinOp::Sub,
                            Operand::Value(clz32),
                            Operand::Const(IrConst::I32(pad)),
                            IrType::I32,
                        )
                    }
                }
            }
            _ => return Some(Operand::Const(IrConst::I32(0))),
        };
        Some(Operand::Value(res_i32))
    }

    fn lower_gcc16_generic_bit_128(
        &mut self,
        name: &str,
        raw: Operand,
        arg_ir_ty: IrType,
        fallback_op: Option<Operand>,
    ) -> Operand {
        let val128 = self.emit_cast_val(raw, arg_ir_ty, IrType::U128);
        let lo = self.emit_cast_val(Operand::Value(val128), IrType::U128, IrType::U64);
        let hi_sh = self.emit_binop_val(
            IrBinOp::LShr,
            Operand::Value(val128),
            Operand::Const(IrConst::I128(64)),
            IrType::U128,
        );
        let hi = self.emit_cast_val(Operand::Value(hi_sh), IrType::U128, IrType::U64);

        let res = match name {
            "__builtin_popcountg" => {
                let pc_lo = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest: pc_lo,
                    op: IrUnaryOp::Popcount,
                    src: Operand::Value(lo),
                    ty: IrType::U64,
                });
                let pc_hi = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest: pc_hi,
                    op: IrUnaryOp::Popcount,
                    src: Operand::Value(hi),
                    ty: IrType::U64,
                });
                let sum = self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(pc_lo),
                    Operand::Value(pc_hi),
                    IrType::U64,
                );
                self.emit_cast_val(Operand::Value(sum), IrType::U64, IrType::I32)
            }
            "__builtin_parityg" => {
                // XOR-fold 128 -> 64 before popcount: cuts Popcount work in half!
                let xored = self.emit_binop_val(
                    IrBinOp::Xor,
                    Operand::Value(lo),
                    Operand::Value(hi),
                    IrType::U64,
                );
                let pc = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest: pc,
                    op: IrUnaryOp::Popcount,
                    src: Operand::Value(xored),
                    ty: IrType::U64,
                });
                let pc32 = self.emit_cast_val(Operand::Value(pc), IrType::U64, IrType::I32);
                self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(pc32),
                    Operand::Const(IrConst::I32(1)),
                    IrType::I32,
                )
            }
            "__builtin_clzg" => {
                let hi_nz = self.emit_cmp_val(
                    IrCmpOp::Ne,
                    Operand::Value(hi),
                    Operand::Const(IrConst::I64(0)),
                    IrType::U64,
                );
                let safe_hi = self.emit_binop_val(
                    IrBinOp::Or,
                    Operand::Value(hi),
                    Operand::Const(IrConst::I64(1)),
                    IrType::U64,
                );
                let clz_hi = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest: clz_hi,
                    op: IrUnaryOp::ClzNonZero,
                    src: Operand::Value(safe_hi),
                    ty: IrType::U64,
                });
                let clz_hi32 = self.emit_cast_val(Operand::Value(clz_hi), IrType::U64, IrType::I32);

                let lo_nz = self.emit_cmp_val(
                    IrCmpOp::Ne,
                    Operand::Value(lo),
                    Operand::Const(IrConst::I64(0)),
                    IrType::U64,
                );
                let safe_lo = self.emit_binop_val(
                    IrBinOp::Or,
                    Operand::Value(lo),
                    Operand::Const(IrConst::I64(1)),
                    IrType::U64,
                );
                let clz_lo = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest: clz_lo,
                    op: IrUnaryOp::ClzNonZero,
                    src: Operand::Value(safe_lo),
                    ty: IrType::U64,
                });
                let clz_lo32 = self.emit_cast_val(Operand::Value(clz_lo), IrType::U64, IrType::I32);
                let lo_plus_64 = self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(clz_lo32),
                    Operand::Const(IrConst::I32(64)),
                    IrType::I32,
                );
                let fb = fallback_op.unwrap_or(Operand::Const(IrConst::I32(128)));
                let low_part = self.gcc16_select(
                    Operand::Value(lo_nz),
                    Operand::Value(lo_plus_64),
                    fb,
                    IrType::I32,
                );
                self.gcc16_select(
                    Operand::Value(hi_nz),
                    Operand::Value(clz_hi32),
                    Operand::Value(low_part),
                    IrType::I32,
                )
            }
            "__builtin_ctzg" | "__builtin_ffsg" => {
                let lo_nz = self.emit_cmp_val(
                    IrCmpOp::Ne,
                    Operand::Value(lo),
                    Operand::Const(IrConst::I64(0)),
                    IrType::U64,
                );
                let safe_lo = self.emit_binop_val(
                    IrBinOp::Or,
                    Operand::Value(lo),
                    Operand::Const(IrConst::I64(i64::MIN)),
                    IrType::U64,
                );
                let ctz_lo = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest: ctz_lo,
                    op: IrUnaryOp::CtzNonZero,
                    src: Operand::Value(safe_lo),
                    ty: IrType::U64,
                });
                let ctz_lo32 = self.emit_cast_val(Operand::Value(ctz_lo), IrType::U64, IrType::I32);

                let hi_nz = self.emit_cmp_val(
                    IrCmpOp::Ne,
                    Operand::Value(hi),
                    Operand::Const(IrConst::I64(0)),
                    IrType::U64,
                );
                let safe_hi = self.emit_binop_val(
                    IrBinOp::Or,
                    Operand::Value(hi),
                    Operand::Const(IrConst::I64(i64::MIN)),
                    IrType::U64,
                );
                let ctz_hi = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest: ctz_hi,
                    op: IrUnaryOp::CtzNonZero,
                    src: Operand::Value(safe_hi),
                    ty: IrType::U64,
                });
                let ctz_hi32 = self.emit_cast_val(Operand::Value(ctz_hi), IrType::U64, IrType::I32);
                let hi_plus_64 = self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(ctz_hi32),
                    Operand::Const(IrConst::I32(64)),
                    IrType::I32,
                );
                if name == "__builtin_ffsg" {
                    let any_nz = self.emit_binop_val(
                        IrBinOp::Or,
                        Operand::Value(lo),
                        Operand::Value(hi),
                        IrType::U64,
                    );
                    let is_nz = self.emit_cmp_val(
                        IrCmpOp::Ne,
                        Operand::Value(any_nz),
                        Operand::Const(IrConst::I64(0)),
                        IrType::U64,
                    );
                    let ctz128 = self.gcc16_select(
                        Operand::Value(lo_nz),
                        Operand::Value(ctz_lo32),
                        Operand::Value(hi_plus_64),
                        IrType::I32,
                    );
                    let plus1 = self.emit_binop_val(
                        IrBinOp::Add,
                        Operand::Value(ctz128),
                        Operand::Const(IrConst::I32(1)),
                        IrType::I32,
                    );
                    self.gcc16_select(
                        Operand::Value(is_nz),
                        Operand::Value(plus1),
                        Operand::Const(IrConst::I32(0)),
                        IrType::I32,
                    )
                } else {
                    let fb = fallback_op.unwrap_or(Operand::Const(IrConst::I32(128)));
                    let high_part = self.gcc16_select(
                        Operand::Value(hi_nz),
                        Operand::Value(hi_plus_64),
                        fb,
                        IrType::I32,
                    );
                    self.gcc16_select(
                        Operand::Value(lo_nz),
                        Operand::Value(ctz_lo32),
                        Operand::Value(high_part),
                        IrType::I32,
                    )
                }
            }
            "__builtin_clrsbg" => self.lower_clrsbg_128(lo, hi),
            _ => return Operand::Const(IrConst::I32(0)),
        };
        Operand::Value(res)
    }

    fn lower_clrsbg_narrow(
        &mut self,
        raw: Operand,
        arg_ir_ty: IrType,
        width: u32,
        op_ty: IrType,
        op_bits: u32,
    ) -> Value {
        let s_ty = if width <= 32 {
            IrType::I32
        } else {
            IrType::I64
        };
        let narrow_s_ty = match width {
            8 => IrType::I8,
            16 => IrType::I16,
            32 => IrType::I32,
            _ => IrType::I64,
        };
        let s_narrow = self.emit_cast_val(raw, arg_ir_ty, narrow_s_ty);
        let s_ext = self.emit_cast_val(Operand::Value(s_narrow), narrow_s_ty, s_ty);
        let sign_splat = self.emit_binop_val(
            IrBinOp::AShr,
            Operand::Value(s_ext),
            Self::gcc16_uconst(s_ty, (op_bits - 1) as u64),
            s_ty,
        );
        let m_signed = self.emit_binop_val(
            IrBinOp::Xor,
            Operand::Value(s_ext),
            Operand::Value(sign_splat),
            s_ty,
        );
        let m = self.emit_cast_val(Operand::Value(m_signed), s_ty, op_ty);
        let shl1 = self.emit_binop_val(
            IrBinOp::Shl,
            Operand::Value(m),
            Self::gcc16_uconst(op_ty, 1),
            op_ty,
        );
        let nz = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(shl1),
            Self::gcc16_uconst(op_ty, 1),
            op_ty,
        );
        let clz = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest: clz,
            op: IrUnaryOp::ClzNonZero,
            src: Operand::Value(nz),
            ty: op_ty,
        });
        let clz_i32 = self.emit_cast_val(Operand::Value(clz), op_ty, IrType::I32);
        let pad = (op_bits - width) as i32;
        if pad == 0 {
            clz_i32
        } else {
            self.emit_binop_val(
                IrBinOp::Sub,
                Operand::Value(clz_i32),
                Operand::Const(IrConst::I32(pad)),
                IrType::I32,
            )
        }
    }

    fn lower_clrsbg_128(&mut self, lo: Value, hi: Value) -> Value {
        let hi_s = self.emit_cast_val(Operand::Value(hi), IrType::U64, IrType::I64);
        let sign_s = self.emit_binop_val(
            IrBinOp::AShr,
            Operand::Value(hi_s),
            Operand::Const(IrConst::I64(63)),
            IrType::I64,
        );
        let sign_u = self.emit_cast_val(Operand::Value(sign_s), IrType::I64, IrType::U64);
        let m_hi = self.emit_binop_val(
            IrBinOp::Xor,
            Operand::Value(hi),
            Operand::Value(sign_u),
            IrType::U64,
        );
        let m_lo = self.emit_binop_val(
            IrBinOp::Xor,
            Operand::Value(lo),
            Operand::Value(sign_u),
            IrType::U64,
        );
        let m_hi_nz = self.emit_cmp_val(
            IrCmpOp::Ne,
            Operand::Value(m_hi),
            Operand::Const(IrConst::I64(0)),
            IrType::U64,
        );
        let m_hi_sh = self.emit_binop_val(
            IrBinOp::Shl,
            Operand::Value(m_hi),
            Operand::Const(IrConst::I64(1)),
            IrType::U64,
        );
        let m_hi_safe = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(m_hi_sh),
            Operand::Const(IrConst::I64(1)),
            IrType::U64,
        );
        let clz_hi = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest: clz_hi,
            op: IrUnaryOp::ClzNonZero,
            src: Operand::Value(m_hi_safe),
            ty: IrType::U64,
        });
        let hi_res = self.emit_cast_val(Operand::Value(clz_hi), IrType::U64, IrType::I32);

        let m_lo_nz = self.emit_cmp_val(
            IrCmpOp::Ne,
            Operand::Value(m_lo),
            Operand::Const(IrConst::I64(0)),
            IrType::U64,
        );
        let safe_lo = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(m_lo),
            Operand::Const(IrConst::I64(1)),
            IrType::U64,
        );
        let clz_lo = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest: clz_lo,
            op: IrUnaryOp::ClzNonZero,
            src: Operand::Value(safe_lo),
            ty: IrType::U64,
        });
        let clz_lo32 = self.emit_cast_val(Operand::Value(clz_lo), IrType::U64, IrType::I32);
        let lo_plus_63 = self.emit_binop_val(
            IrBinOp::Add,
            Operand::Value(clz_lo32),
            Operand::Const(IrConst::I32(63)),
            IrType::I32,
        );
        let lo_res = self.gcc16_select(
            Operand::Value(m_lo_nz),
            Operand::Value(lo_plus_63),
            Operand::Const(IrConst::I32(127)),
            IrType::I32,
        );
        self.gcc16_select(
            Operand::Value(m_hi_nz),
            Operand::Value(hi_res),
            Operand::Value(lo_res),
            IrType::I32,
        )
    }

    /// Lower `__builtin_addcb/s/c/l/ll` and `__builtin_subcb/s/c/l/ll`.
    pub(super) fn lower_gcc16_carry_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Option<Operand> {
        let (elem_ct, is_sub, _width) = gcc16_builtins::carry_builtin_info(name)?;
        if args.len() != 4 {
            return Some(Operand::Const(IrConst::I32(0)));
        }
        let op_ty = IrType::from_ctype(&elem_ct);
        let x_raw = self.lower_expr(&args[0]);
        let x_ty = self.get_expr_type(&args[0]);
        let x = self.emit_cast_val(x_raw, x_ty, op_ty);

        let y_raw = self.lower_expr(&args[1]);
        let y_ty = self.get_expr_type(&args[1]);
        let y = self.emit_cast_val(y_raw, y_ty, op_ty);

        let cin_raw = self.lower_expr(&args[2]);
        let cin_ty = self.get_expr_type(&args[2]);
        let cin = self.emit_cast_val(cin_raw, cin_ty, op_ty);

        let ptr_op = self.lower_expr(&args[3]);
        let ptr_val = self.operand_to_value(ptr_op);
        let seg_override = match self.get_expr_ctype(&args[3]) {
            Some(CType::Pointer(_, sp)) => sp,
            _ => AddressSpace::Default,
        };
        let store_ty = match self.get_expr_ctype(&args[3]) {
            Some(CType::Pointer(inner, _)) => IrType::from_ctype(&inner),
            _ => op_ty,
        };

        let (res, carry_bool) = if !is_sub {
            let s1 = self.emit_binop_val(IrBinOp::Add, Operand::Value(x), Operand::Value(y), op_ty);
            let c1 = self.emit_cmp_val(IrCmpOp::Ult, Operand::Value(s1), Operand::Value(x), op_ty);
            let s2 =
                self.emit_binop_val(IrBinOp::Add, Operand::Value(s1), Operand::Value(cin), op_ty);
            let c2 = self.emit_cmp_val(IrCmpOp::Ult, Operand::Value(s2), Operand::Value(s1), op_ty);
            let c_or = self.emit_binop_val(
                IrBinOp::Or,
                Operand::Value(c1),
                Operand::Value(c2),
                IrType::I8,
            );
            (s2, c_or)
        } else {
            let d1 = self.emit_binop_val(IrBinOp::Sub, Operand::Value(x), Operand::Value(y), op_ty);
            let b1 = self.emit_cmp_val(IrCmpOp::Ult, Operand::Value(x), Operand::Value(y), op_ty);
            let d2 =
                self.emit_binop_val(IrBinOp::Sub, Operand::Value(d1), Operand::Value(cin), op_ty);
            let b2 =
                self.emit_cmp_val(IrCmpOp::Ult, Operand::Value(d1), Operand::Value(cin), op_ty);
            let b_or = self.emit_binop_val(
                IrBinOp::Or,
                Operand::Value(b1),
                Operand::Value(b2),
                IrType::I8,
            );
            (d2, b_or)
        };

        let cout_val = self.emit_cast_val(Operand::Value(carry_bool), IrType::U8, store_ty);
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Value(cout_val),
            ptr: ptr_val,
            ty: store_ty,
            seg_override,
        });
        Some(Operand::Value(res))
    }

    /// Lower `__builtin_bswap128`, `__builtin_bitreverse{8,16,32,64}`, and
    /// `__builtin_rotate{left,right}{8,16,32,64}`.
    pub(super) fn lower_gcc16_bswap_bitrev_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Option<Operand> {
        if let Some((is_left, bits)) =
            crate::frontend::sema::gcc16_builtins::rotate_builtin_info(name)
        {
            return self.lower_gcc16_fixed_rotate(name, is_left, bits, args);
        }
        let arg = args.first()?;
        if !Self::expr_may_have_side_effects(arg) {
            if let Some(c) = self.eval_gcc16_bswap_bitrev_const(name, args) {
                return Some(Operand::Const(c));
            }
        }
        let raw = self.lower_expr(arg);
        let arg_ty = self.get_expr_type(arg);
        if name == "__builtin_bswap128" {
            let v128 = self.emit_cast_val(raw, arg_ty, IrType::U128);
            let lo = self.emit_cast_val(Operand::Value(v128), IrType::U128, IrType::U64);
            let hi_sh = self.emit_binop_val(
                IrBinOp::LShr,
                Operand::Value(v128),
                Operand::Const(IrConst::I128(64)),
                IrType::U128,
            );
            let hi = self.emit_cast_val(Operand::Value(hi_sh), IrType::U128, IrType::U64);
            let bswap_lo = self.fresh_value();
            self.emit(Instruction::UnaryOp {
                dest: bswap_lo,
                op: IrUnaryOp::Bswap,
                src: Operand::Value(lo),
                ty: IrType::U64,
            });
            let bswap_hi = self.fresh_value();
            self.emit(Instruction::UnaryOp {
                dest: bswap_hi,
                op: IrUnaryOp::Bswap,
                src: Operand::Value(hi),
                ty: IrType::U64,
            });
            let new_hi128 = self.emit_cast_val(Operand::Value(bswap_lo), IrType::U64, IrType::U128);
            let new_lo128 = self.emit_cast_val(Operand::Value(bswap_hi), IrType::U64, IrType::U128);
            let sh = self.emit_binop_val(
                IrBinOp::Shl,
                Operand::Value(new_hi128),
                Operand::Const(IrConst::I128(64)),
                IrType::U128,
            );
            let res = self.emit_binop_val(
                IrBinOp::Or,
                Operand::Value(sh),
                Operand::Value(new_lo128),
                IrType::U128,
            );
            return Some(Operand::Value(res));
        }

        let (bits, res_ty) = match name {
            "__builtin_bitreverse8" => (8u32, IrType::U8),
            "__builtin_bitreverse16" => (16u32, IrType::U16),
            "__builtin_bitreverse32" => (32u32, IrType::U32),
            "__builtin_bitreverse64" => (64u32, IrType::U64),
            _ => return None,
        };
        let work_ty = if bits == 64 { IrType::U64 } else { IrType::U32 };
        let mut v = self.emit_cast_val(raw, arg_ty, res_ty);
        if bits >= 16 {
            let bs = self.fresh_value();
            self.emit(Instruction::UnaryOp {
                dest: bs,
                op: IrUnaryOp::Bswap,
                src: Operand::Value(v),
                ty: res_ty,
            });
            v = bs;
        }
        v = self.emit_cast_val(Operand::Value(v), res_ty, work_ty);
        // Parallel SWAR bit reversal within each byte: nibbles -> pairs -> bits.
        for (shift, m_hi, m_lo) in [
            (4u64, 0xF0F0_F0F0_F0F0_F0F0u64, 0x0F0F_0F0F_0F0F_0F0Fu64),
            (2u64, 0xCCCC_CCCC_CCCC_CCCCu64, 0x3333_3333_3333_3333u64),
            (1u64, 0xAAAA_AAAA_AAAA_AAAAu64, 0x5555_5555_5555_5555u64),
        ] {
            let hi_part = self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(v),
                Self::gcc16_uconst(work_ty, m_hi),
                work_ty,
            );
            let hi_shr = self.emit_binop_val(
                IrBinOp::LShr,
                Operand::Value(hi_part),
                Self::gcc16_uconst(work_ty, shift),
                work_ty,
            );
            let lo_part = self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(v),
                Self::gcc16_uconst(work_ty, m_lo),
                work_ty,
            );
            let lo_shl = self.emit_binop_val(
                IrBinOp::Shl,
                Operand::Value(lo_part),
                Self::gcc16_uconst(work_ty, shift),
                work_ty,
            );
            v = self.emit_binop_val(
                IrBinOp::Or,
                Operand::Value(hi_shr),
                Operand::Value(lo_shl),
                work_ty,
            );
        }
        let narrowed = self.emit_cast_val(Operand::Value(v), work_ty, res_ty);
        Some(Operand::Value(narrowed))
    }

    fn lower_gcc16_fixed_rotate(
        &mut self,
        name: &str,
        is_left: bool,
        bits: u32,
        args: &[Expr],
    ) -> Option<Operand> {
        if args.len() != 2 {
            return None;
        }
        if !Self::expr_may_have_side_effects(&args[0])
            && !Self::expr_may_have_side_effects(&args[1])
            && let Some(c) = self.eval_gcc16_bswap_bitrev_const(name, args)
        {
            return Some(Operand::Const(c));
        }
        let res_ty = match bits {
            8 => IrType::U8,
            16 => IrType::U16,
            32 => IrType::U32,
            _ => IrType::U64,
        };
        let work_ty = if bits == 64 { IrType::U64 } else { IrType::U32 };
        let raw_x = self.lower_expr(&args[0]);
        let ty_x = self.get_expr_type(&args[0]);
        let raw_r = self.lower_expr(&args[1]);
        let ty_r = self.get_expr_type(&args[1]);

        let x_narrow = self.emit_cast_val(raw_x, ty_x, res_ty);
        let x = self.emit_cast_val(Operand::Value(x_narrow), res_ty, work_ty);
        let r_raw = self.emit_cast_val(raw_r, ty_r, work_ty);
        let mask = (bits - 1) as u64;
        let r = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(r_raw),
            Self::gcc16_uconst(work_ty, mask),
            work_ty,
        );
        let neg_r_raw = self.emit_binop_val(
            IrBinOp::Sub,
            Self::gcc16_uconst(work_ty, 0),
            Operand::Value(r),
            work_ty,
        );
        let inv_r = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(neg_r_raw),
            Self::gcc16_uconst(work_ty, mask),
            work_ty,
        );
        let (shl_amt, shr_amt) = if is_left { (r, inv_r) } else { (inv_r, r) };
        let shl = self.emit_binop_val(
            IrBinOp::Shl,
            Operand::Value(x),
            Operand::Value(shl_amt),
            work_ty,
        );
        let shr = self.emit_binop_val(
            IrBinOp::LShr,
            Operand::Value(x),
            Operand::Value(shr_amt),
            work_ty,
        );
        let combined = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(shl),
            Operand::Value(shr),
            work_ty,
        );
        let out = self.emit_cast_val(Operand::Value(combined), work_ty, res_ty);
        Some(Operand::Value(out))
    }

    /// Lower `__builtin_issignaling(x)` to exact branchless IEEE-754 sNaN bit checks.
    pub(super) fn lower_gcc16_issignaling(&mut self, args: &[Expr]) -> Option<Operand> {
        let arg = args.first()?;
        let ctype = self.get_expr_ctype(arg).unwrap_or(CType::Double);
        let ir_ty = self.get_expr_type(arg);
        let val = self.lower_expr(arg);
        let res_bool = match ir_ty {
            IrType::F32 => {
                let slot = self.fresh_value();
                self.emit(Instruction::Alloca {
                    dest: slot,
                    size: 4,
                    ty: IrType::F32,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                });
                let v = self.operand_to_value(val);
                self.emit(Instruction::Store {
                    volatile: false,
                    val: Operand::Value(v),
                    ptr: slot,
                    ty: IrType::F32,
                    seg_override: AddressSpace::Default,
                });
                let bits = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: bits,
                    ptr: slot,
                    ty: IrType::U32,
                    seg_override: AddressSpace::Default,
                });
                let abs_b = self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(bits),
                    Operand::Const(IrConst::I32(0x7FFF_FFFFu32 as i32)),
                    IrType::U32,
                );
                let sub = self.emit_binop_val(
                    IrBinOp::Sub,
                    Operand::Value(abs_b),
                    Operand::Const(IrConst::I32(0x7F80_0001u32 as i32)),
                    IrType::U32,
                );
                self.emit_cmp_val(
                    IrCmpOp::Ult,
                    Operand::Value(sub),
                    Operand::Const(IrConst::I32(0x003F_FFFF)),
                    IrType::U32,
                )
            }
            IrType::F128 => self.lower_issignaling_f128(val, &ctype),
            _ => {
                let slot = self.fresh_value();
                self.emit(Instruction::Alloca {
                    dest: slot,
                    size: 8,
                    ty: IrType::F64,
                    align: 8,
                    volatile: false,
                    semantic_volatile: false,
                });
                let v = self.operand_to_value(val);
                self.emit(Instruction::Store {
                    volatile: false,
                    val: Operand::Value(v),
                    ptr: slot,
                    ty: IrType::F64,
                    seg_override: AddressSpace::Default,
                });
                let bits = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: bits,
                    ptr: slot,
                    ty: IrType::U64,
                    seg_override: AddressSpace::Default,
                });
                let abs_b = self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(bits),
                    Operand::Const(IrConst::I64(0x7FFF_FFFF_FFFF_FFFF)),
                    IrType::U64,
                );
                let sub = self.emit_binop_val(
                    IrBinOp::Sub,
                    Operand::Value(abs_b),
                    Operand::Const(IrConst::I64(0x7FF0_0000_0000_0001)),
                    IrType::U64,
                );
                self.emit_cmp_val(
                    IrCmpOp::Ult,
                    Operand::Value(sub),
                    Operand::Const(IrConst::I64(0x0007_FFFF_FFFF_FFFF)),
                    IrType::U64,
                )
            }
        };
        let ext = self.emit_cast_val(Operand::Value(res_bool), IrType::U8, IrType::I32);
        Some(Operand::Value(ext))
    }

    fn lower_issignaling_f128(&mut self, val: Operand, ctype: &CType) -> Value {
        let slot = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: slot,
            size: 16,
            ty: IrType::F128,
            align: 16,
            volatile: false,
            semantic_volatile: false,
        });
        let v = self.operand_to_value(val);
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Value(v),
            ptr: slot,
            ty: IrType::F128,
            seg_override: AddressSpace::Default,
        });
        let is_x87 = matches!(ctype, CType::LongDouble)
            && matches!(self.target, Target::X86_64 | Target::I686);
        if is_x87 {
            let mant = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: mant,
                ptr: slot,
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
            });
            let ptr_ty = if target_is_32bit() {
                IrType::I32
            } else {
                IrType::I64
            };
            let exp_ptr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: exp_ptr,
                base: slot,
                offset: Self::gcc16_uconst(ptr_ty, 8),
                ty: IrType::U16,
            });
            let se = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: se,
                ptr: exp_ptr,
                ty: IrType::U16,
                seg_override: AddressSpace::Default,
            });
            let exp = self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(se),
                Operand::Const(IrConst::I16(0x7FFF)),
                IrType::U16,
            );
            let exp_max = self.emit_cmp_val(
                IrCmpOp::Eq,
                Operand::Value(exp),
                Operand::Const(IrConst::I16(0x7FFF)),
                IrType::U16,
            );
            let m_sub = self.emit_binop_val(
                IrBinOp::Sub,
                Operand::Value(mant),
                Operand::Const(IrConst::I64(0x8000_0000_0000_0001u64 as i64)),
                IrType::U64,
            );
            let m_ok = self.emit_cmp_val(
                IrCmpOp::Ult,
                Operand::Value(m_sub),
                Operand::Const(IrConst::I64(0x3FFF_FFFF_FFFF_FFFF)),
                IrType::U64,
            );
            self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(exp_max),
                Operand::Value(m_ok),
                IrType::U8,
            )
        } else {
            let lo = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: lo,
                ptr: slot,
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
            });
            let ptr_ty = if target_is_32bit() {
                IrType::I32
            } else {
                IrType::I64
            };
            let hi_ptr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: hi_ptr,
                base: slot,
                offset: Self::gcc16_uconst(ptr_ty, 8),
                ty: IrType::U64,
            });
            let hi = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: hi,
                ptr: hi_ptr,
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
            });
            let abs_hi = self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(hi),
                Operand::Const(IrConst::I64(0x7FFF_FFFF_FFFF_FFFF)),
                IrType::U64,
            );
            let top16 = self.emit_binop_val(
                IrBinOp::LShr,
                Operand::Value(abs_hi),
                Operand::Const(IrConst::I64(47)),
                IrType::U64,
            );
            // Exponent all 1s (0x7FFF) AND quiet bit 47 == 0 -> top 17 bits (sans sign) == 0xFFFE
            let exp_quiet_ok = self.emit_cmp_val(
                IrCmpOp::Eq,
                Operand::Value(top16),
                Operand::Const(IrConst::I64(0xFFFE)),
                IrType::U64,
            );
            let frac_hi = self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(abs_hi),
                Operand::Const(IrConst::I64(0x0000_7FFF_FFFF_FFFF)),
                IrType::U64,
            );
            let frac = self.emit_binop_val(
                IrBinOp::Or,
                Operand::Value(frac_hi),
                Operand::Value(lo),
                IrType::U64,
            );
            let frac_nz = self.emit_cmp_val(
                IrCmpOp::Ne,
                Operand::Value(frac),
                Operand::Const(IrConst::I64(0)),
                IrType::U64,
            );
            self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(exp_quiet_ok),
                Operand::Value(frac_nz),
                IrType::U8,
            )
        }
    }

    /// Lower `__builtin_iseqsig(x, y)`.
    pub(super) fn lower_gcc16_iseqsig(&mut self, args: &[Expr]) -> Option<Operand> {
        if args.len() != 2 {
            return Some(Operand::Const(IrConst::I32(0)));
        }
        let t0 = self.get_expr_type(&args[0]);
        let t1 = self.get_expr_type(&args[1]);
        let cmp_ty = if t0 == IrType::F128 || t1 == IrType::F128 {
            IrType::F128
        } else if t0 == IrType::F64 || t1 == IrType::F64 {
            IrType::F64
        } else {
            IrType::F32
        };
        let v0_raw = self.lower_expr(&args[0]);
        let v1_raw = self.lower_expr(&args[1]);
        let v0 = self.emit_cast_val(v0_raw, t0, cmp_ty);
        let v1 = self.emit_cast_val(v1_raw, t1, cmp_ty);
        // C23 / TS 18661-1 `iseqsig(x, y)` is equivalent to `(x <= y) && (x >= y)`
        // (raising FE_INVALID on any NaN via ordered comparisons, returning 1 iff equal).
        let le = self.emit_cmp_val(IrCmpOp::Sle, Operand::Value(v0), Operand::Value(v1), cmp_ty);
        let ge = self.emit_cmp_val(IrCmpOp::Sge, Operand::Value(v0), Operand::Value(v1), cmp_ty);
        let eq = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(le),
            Operand::Value(ge),
            IrType::U8,
        );
        let ext = self.emit_cast_val(Operand::Value(eq), IrType::U8, IrType::I32);
        Some(Operand::Value(ext))
    }

    /// Lower `__builtin_speculation_safe_value(val [, failval])`.
    pub(super) fn lower_gcc16_speculation_safe(&mut self, args: &[Expr]) -> Option<Operand> {
        let arg0 = args.first()?;
        let val = self.lower_expr(arg0);
        if let Some(arg1) = args.get(1) {
            let _ = self.lower_expr(arg1);
        }
        Some(val)
    }

    /// Lower `__builtin_bit_cast(T, expr)` across scalars, pointers, vectors, and aggregates.
    pub(super) fn lower_bit_cast(
        &mut self,
        target_spec: &TypeSpecifier,
        src_expr: &Expr,
    ) -> Operand {
        if !Self::expr_may_have_side_effects(src_expr) {
            if let Some(c) = self.eval_bit_cast_const(target_spec, src_expr) {
                return Operand::Const(c);
            }
        }
        let dst_ctype = self.type_spec_to_ctype(target_spec);
        let dst_ir_ty = self.type_spec_to_ir(target_spec);
        let dst_size = self.sizeof_type(target_spec).max(1);
        let dst_align = self.c11_alignof_type(target_spec).max(1);

        let dst_slot = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: dst_slot,
            size: dst_size,
            ty: dst_ir_ty,
            align: dst_align,
            volatile: false,
            semantic_volatile: false,
        });

        let src_ctype = self.get_expr_ctype(src_expr);
        let src_is_agg = src_ctype.as_ref().is_some_and(|ct| {
            matches!(
                ct,
                CType::Struct(..)
                    | CType::Union(..)
                    | CType::Array(..)
                    | CType::Vector(..)
                    | CType::ExtVector(..)
                    | CType::ComplexFloat
                    | CType::ComplexDouble
                    | CType::ComplexLongDouble
            )
        });

        let src_op = self.lower_expr(src_expr);
        if src_is_agg {
            let src_ptr = self.operand_to_value(src_op);
            self.emit(Instruction::Memcpy {
                dest: dst_slot,
                src: src_ptr,
                size: dst_size,
            });
        } else {
            let src_ir_ty = self.get_expr_type(src_expr);
            let src_v = self.operand_to_value(src_op);
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Value(src_v),
                ptr: dst_slot,
                ty: src_ir_ty,
                seg_override: AddressSpace::Default,
            });
        }

        let dst_is_agg = matches!(
            dst_ctype,
            CType::Struct(..)
                | CType::Union(..)
                | CType::Array(..)
                | CType::Vector(..)
                | CType::ExtVector(..)
                | CType::ComplexFloat
                | CType::ComplexDouble
                | CType::ComplexLongDouble
        );
        if dst_is_agg {
            Operand::Value(dst_slot)
        } else {
            let loaded = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: loaded,
                ptr: dst_slot,
                ty: dst_ir_ty,
                seg_override: AddressSpace::Default,
            });
            Operand::Value(loaded)
        }
    }

    pub(super) fn eval_gcc16_generic_bit_const(
        &self,
        name: &str,
        args: &[Expr],
    ) -> Option<IrConst> {
        let arg0 = args.first()?;
        let arg0_ct = self.get_expr_ctype(arg0)?.decayed_value_ctype();
        let width = gcc16_builtins::int_bit_width_of_ctype(&arg0_ct)?;
        let c0 = self.eval_const_expr(arg0)?;
        let raw_val = match c0 {
            IrConst::I128(v) => v as u128,
            other => other.to_u64()? as u128,
        };
        let fb = if args.len() == 2 {
            Some(self.eval_const_expr(&args[1])?.to_i64()?)
        } else {
            None
        };
        let res = gcc16_builtins::eval_generic_bit_builtin(name, raw_val, fb, width)?;
        Some(IrConst::I32(res as i32))
    }

    pub(super) fn eval_gcc16_bswap_bitrev_const(
        &self,
        name: &str,
        args: &[Expr],
    ) -> Option<IrConst> {
        if gcc16_builtins::rotate_builtin_info(name).is_some() {
            if args.len() != 2 {
                return None;
            }
            let v0 = self.eval_const_expr(&args[0])?.to_u64()?;
            let v1 = self.eval_const_expr(&args[1])?.to_u64()?;
            let out = gcc16_builtins::eval_fixed_rotate(name, v0, v1)?;
            return Some(match name {
                "__builtin_rotateleft8" | "__builtin_rotateright8" => IrConst::I8(out as u8 as i8),
                "__builtin_rotateleft16" | "__builtin_rotateright16" => {
                    IrConst::I16(out as u16 as i16)
                }
                "__builtin_rotateleft32" | "__builtin_rotateright32" => {
                    IrConst::I32(out as u32 as i32)
                }
                _ => IrConst::I64(out as i64),
            });
        }
        if args.len() != 1 {
            return None;
        }
        let c0 = self.eval_const_expr(&args[0])?;
        let raw_val = match c0 {
            IrConst::I128(v) => v as u128,
            other => other.to_u64()? as u128,
        };
        let out = gcc16_builtins::eval_bswap_or_bitrev(name, raw_val)?;
        Some(match name {
            "__builtin_bswap128" => IrConst::I128(out as i128),
            "__builtin_bitreverse8" => IrConst::I8(out as u8 as i8),
            "__builtin_bitreverse16" => IrConst::I16(out as u16 as i16),
            "__builtin_bitreverse32" => IrConst::I32(out as u32 as i32),
            "__builtin_bitreverse64" => IrConst::I64(out as u64 as i64),
            _ => return None,
        })
    }

    pub(super) fn eval_bit_cast_const(
        &self,
        target_spec: &TypeSpecifier,
        src_expr: &Expr,
    ) -> Option<IrConst> {
        let dst_ct = self.type_spec_to_ctype(target_spec);
        let src_ct = self.get_expr_ctype(src_expr)?.decayed_value_ctype();
        let sz = self.sizeof_type(target_spec);
        if sz == 0 || sz > 16 || src_ct.size() != sz {
            return None;
        }
        let c = self.eval_const_expr(src_expr)?;
        let mut bytes = [0u8; 16];
        match (&src_ct, c) {
            (CType::Float, IrConst::F32(f)) => {
                bytes[..4].copy_from_slice(&f.to_bits().to_le_bytes())
            }
            (CType::Float, IrConst::F64(d)) => {
                bytes[..4].copy_from_slice(&(d as f32).to_bits().to_le_bytes());
            }
            (CType::Double, IrConst::F64(d)) => {
                bytes[..8].copy_from_slice(&d.to_bits().to_le_bytes())
            }
            (CType::Double, IrConst::F32(f)) => {
                bytes[..8].copy_from_slice(&(f as f64).to_bits().to_le_bytes());
            }
            (ct, IrConst::I128(v)) if ct.is_integer() => bytes.copy_from_slice(&v.to_le_bytes()),
            (ct, other) if ct.is_integer() => {
                let u = other.to_u64()? as u128;
                bytes.copy_from_slice(&u.to_le_bytes());
            }
            _ => return None,
        }
        match dst_ct {
            CType::Float => Some(IrConst::F32(f32::from_bits(u32::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
            ])))),
            CType::Double => Some(IrConst::F64(f64::from_bits(u64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ])))),
            CType::Bool => None,
            ct if ct.is_integer() => {
                let raw = u128::from_le_bytes(bytes);
                Some(match sz {
                    1 => IrConst::I8(if ct.is_signed() {
                        raw as u8 as i8
                    } else {
                        raw as u8 as i8
                    }),
                    2 => IrConst::I16(raw as u16 as i16),
                    4 => IrConst::I32(raw as u32 as i32),
                    8 => IrConst::I64(raw as u64 as i64),
                    16 => IrConst::I128(raw as i128),
                    _ => return None,
                })
            }
            _ => None,
        }
    }
}
