//! `-ftrapv` signed integer overflow trapping and
//! `-ftrivial-auto-var-init={uninitialized,zero,pattern}` automatic storage
//! initialization.

use super::lower::{Lowerer, ZERO_INIT_STORE_LADDER_MAX};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::instruction::CallInfo;
use crate::ir::reexports::{
    Instruction, IrBinOp, IrCmpOp, IrConst, IrUnaryOp, Operand, Terminator, Value,
};

/// Policy for `-ftrivial-auto-var-init=<mode>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrivialAutoVarInit {
    #[default]
    Uninitialized,
    Zero,
    Pattern,
}

impl Lowerer {
    /// Emit a cold trap branch (`if (ovf != 0) { abort(); unreachable; }`) for `-ftrapv`.
    ///
    /// Uses inline SSA arithmetic on the hot fall-through path (matching Clang's
    /// inline overflow check performance rather than GCC's out-of-line
    /// `__addvsi3`/`__mulvdi3` libcalls) while calling `abort()` on the cold
    /// overflow block so `SIGABRT` handlers in POSIX / GCC `ftrapv` test suites
    /// observe the exact expected signal.
    fn emit_trapv_guard(&mut self, ovf_cond: Value) {
        let ok_label = self.fresh_label();
        let trap_label = self.fresh_label();
        self.terminate(Terminator::CondBranch {
            cond: Operand::Value(ovf_cond),
            true_label: trap_label,
            false_label: ok_label,
        });
        self.start_block(trap_label);
        if self.trapv {
            self.emit(Instruction::Call {
                func: "abort".to_string(),
                info: CallInfo {
                    dest: None,
                    args: Vec::new(),
                    arg_types: Vec::new(),
                    return_type: IrType::Void,
                    is_variadic: false,
                    num_fixed_args: 0,
                    ..Default::default()
                },
            });
        }
        self.terminate(Terminator::Unreachable);
        self.start_block(ok_label);
    }

    /// Lower signed `Add` / `Sub` / `Mul` with `-ftrapv` overflow detection at
    /// the C semantic width `common_ty` (`I32`, `I64`, or `I128`), returning a
    /// `Value` of `op_ty` (`widened_op_type(common_ty)`).
    pub(super) fn emit_trapv_signed_binop(
        &mut self,
        ir_op: IrBinOp,
        lhs: Operand,
        rhs: Operand,
        common_ty: IrType,
        op_ty: IrType,
    ) -> Value {
        match common_ty {
            IrType::I32 => {
                // Compute exact mathematical result in I64; I32 overflows iff
                // sign_extend_32_to_64(trunc_64_to_32(res64)) != res64.
                let lhs64 = if op_ty == IrType::I64 {
                    lhs
                } else {
                    Operand::Value(self.emit_cast_val(lhs, op_ty, IrType::I64))
                };
                let rhs64 = if op_ty == IrType::I64 {
                    rhs
                } else {
                    Operand::Value(self.emit_cast_val(rhs, op_ty, IrType::I64))
                };
                let res64 = self.emit_binop_val(ir_op, lhs64, rhs64, IrType::I64);
                let trunc32 = self.emit_cast_val(Operand::Value(res64), IrType::I64, IrType::I32);
                let sext64 = self.emit_cast_val(Operand::Value(trunc32), IrType::I32, IrType::I64);
                let ovf = self.emit_cmp_val(
                    IrCmpOp::Ne,
                    Operand::Value(sext64),
                    Operand::Value(res64),
                    IrType::I64,
                );
                self.emit_trapv_guard(ovf);
                if op_ty == IrType::I64 {
                    sext64
                } else {
                    trunc32
                }
            }
            IrType::I64 => match ir_op {
                IrBinOp::Add => {
                    let res = self.emit_binop_val(IrBinOp::Add, lhs, rhs, IrType::I64);
                    // Overflow iff ((res ^ lhs) & (res ^ rhs)) < 0.
                    let xl =
                        self.emit_binop_val(IrBinOp::Xor, Operand::Value(res), lhs, IrType::I64);
                    let xr =
                        self.emit_binop_val(IrBinOp::Xor, Operand::Value(res), rhs, IrType::I64);
                    let both = self.emit_binop_val(
                        IrBinOp::And,
                        Operand::Value(xl),
                        Operand::Value(xr),
                        IrType::I64,
                    );
                    let ovf = self.emit_cmp_val(
                        IrCmpOp::Slt,
                        Operand::Value(both),
                        Operand::Const(IrConst::I64(0)),
                        IrType::I64,
                    );
                    self.emit_trapv_guard(ovf);
                    res
                }
                IrBinOp::Sub => {
                    let res = self.emit_binop_val(IrBinOp::Sub, lhs, rhs, IrType::I64);
                    // Overflow iff ((lhs ^ rhs) & (lhs ^ res)) < 0.
                    let xlr = self.emit_binop_val(IrBinOp::Xor, lhs, rhs, IrType::I64);
                    let xls =
                        self.emit_binop_val(IrBinOp::Xor, lhs, Operand::Value(res), IrType::I64);
                    let both = self.emit_binop_val(
                        IrBinOp::And,
                        Operand::Value(xlr),
                        Operand::Value(xls),
                        IrType::I64,
                    );
                    let ovf = self.emit_cmp_val(
                        IrCmpOp::Slt,
                        Operand::Value(both),
                        Operand::Const(IrConst::I64(0)),
                        IrType::I64,
                    );
                    self.emit_trapv_guard(ovf);
                    res
                }
                IrBinOp::Mul => {
                    // Exact product in I128; overflows I64 iff sext64(trunc64(p)) != p.
                    let lhs128 = self.emit_cast_val(lhs, IrType::I64, IrType::I128);
                    let rhs128 = self.emit_cast_val(rhs, IrType::I64, IrType::I128);
                    let prod128 = self.emit_binop_val(
                        IrBinOp::Mul,
                        Operand::Value(lhs128),
                        Operand::Value(rhs128),
                        IrType::I128,
                    );
                    let low64 =
                        self.emit_cast_val(Operand::Value(prod128), IrType::I128, IrType::I64);
                    let sext128 =
                        self.emit_cast_val(Operand::Value(low64), IrType::I64, IrType::I128);
                    let ovf = self.emit_cmp_val(
                        IrCmpOp::Ne,
                        Operand::Value(sext128),
                        Operand::Value(prod128),
                        IrType::I128,
                    );
                    self.emit_trapv_guard(ovf);
                    low64
                }
                _ => self.emit_binop_val(ir_op, lhs, rhs, op_ty),
            },
            IrType::I128 => match ir_op {
                IrBinOp::Add => {
                    let res = self.emit_binop_val(IrBinOp::Add, lhs, rhs, IrType::I128);
                    let xl =
                        self.emit_binop_val(IrBinOp::Xor, Operand::Value(res), lhs, IrType::I128);
                    let xr =
                        self.emit_binop_val(IrBinOp::Xor, Operand::Value(res), rhs, IrType::I128);
                    let both = self.emit_binop_val(
                        IrBinOp::And,
                        Operand::Value(xl),
                        Operand::Value(xr),
                        IrType::I128,
                    );
                    let ovf = self.emit_cmp_val(
                        IrCmpOp::Slt,
                        Operand::Value(both),
                        Operand::Const(IrConst::I128(0)),
                        IrType::I128,
                    );
                    self.emit_trapv_guard(ovf);
                    res
                }
                IrBinOp::Sub => {
                    let res = self.emit_binop_val(IrBinOp::Sub, lhs, rhs, IrType::I128);
                    let xlr = self.emit_binop_val(IrBinOp::Xor, lhs, rhs, IrType::I128);
                    let xls =
                        self.emit_binop_val(IrBinOp::Xor, lhs, Operand::Value(res), IrType::I128);
                    let both = self.emit_binop_val(
                        IrBinOp::And,
                        Operand::Value(xlr),
                        Operand::Value(xls),
                        IrType::I128,
                    );
                    let ovf = self.emit_cmp_val(
                        IrCmpOp::Slt,
                        Operand::Value(both),
                        Operand::Const(IrConst::I128(0)),
                        IrType::I128,
                    );
                    self.emit_trapv_guard(ovf);
                    res
                }
                IrBinOp::Mul => {
                    let res = self.emit_binop_val(IrBinOp::Mul, lhs, rhs, IrType::I128);
                    // Guard lhs != 0 before checking (lhs == -1 && rhs == MIN) || (res / lhs != rhs).
                    let lhs_nz = self.emit_cmp_val(
                        IrCmpOp::Ne,
                        lhs,
                        Operand::Const(IrConst::I128(0)),
                        IrType::I128,
                    );
                    let check_bb = self.fresh_label();
                    let min_check_bb = self.fresh_label();
                    let div_bb = self.fresh_label();
                    let done_bb = self.fresh_label();
                    let trap_bb = self.fresh_label();
                    self.terminate(Terminator::CondBranch {
                        cond: Operand::Value(lhs_nz),
                        true_label: check_bb,
                        false_label: done_bb,
                    });
                    self.start_block(check_bb);
                    let is_neg1 = self.emit_cmp_val(
                        IrCmpOp::Eq,
                        lhs,
                        Operand::Const(IrConst::I128(-1)),
                        IrType::I128,
                    );
                    self.terminate(Terminator::CondBranch {
                        cond: Operand::Value(is_neg1),
                        true_label: min_check_bb,
                        false_label: div_bb,
                    });
                    self.start_block(min_check_bb);
                    let is_min = self.emit_cmp_val(
                        IrCmpOp::Eq,
                        rhs,
                        Operand::Const(IrConst::I128(i128::MIN)),
                        IrType::I128,
                    );
                    self.terminate(Terminator::CondBranch {
                        cond: Operand::Value(is_min),
                        true_label: trap_bb,
                        false_label: done_bb,
                    });
                    self.start_block(div_bb);
                    let q =
                        self.emit_binop_val(IrBinOp::SDiv, Operand::Value(res), lhs, IrType::I128);
                    let bad = self.emit_cmp_val(IrCmpOp::Ne, Operand::Value(q), rhs, IrType::I128);
                    self.terminate(Terminator::CondBranch {
                        cond: Operand::Value(bad),
                        true_label: trap_bb,
                        false_label: done_bb,
                    });
                    self.start_block(trap_bb);
                    if self.trapv {
                        self.emit(Instruction::Call {
                            func: "abort".to_string(),
                            info: CallInfo {
                                dest: None,
                                args: Vec::new(),
                                arg_types: Vec::new(),
                                return_type: IrType::Void,
                                is_variadic: false,
                                num_fixed_args: 0,
                                ..Default::default()
                            },
                        });
                    }
                    self.terminate(Terminator::Unreachable);
                    self.start_block(done_bb);
                    res
                }
                _ => self.emit_binop_val(ir_op, lhs, rhs, op_ty),
            },
            _ => self.emit_binop_val(ir_op, lhs, rhs, op_ty),
        }
    }

    /// Lower signed unary `-x` with `-ftrapv` overflow detection (`x == T::MIN` traps).
    pub(super) fn emit_trapv_signed_neg(
        &mut self,
        val: Operand,
        promoted_ty: IrType,
        neg_ty: IrType,
    ) -> Value {
        let min_ovf = match promoted_ty {
            IrType::I32 => {
                let min_c = if neg_ty == IrType::I64 {
                    IrConst::I64(i32::MIN as i64)
                } else {
                    IrConst::I32(i32::MIN)
                };
                Some(self.emit_cmp_val(IrCmpOp::Eq, val, Operand::Const(min_c), neg_ty))
            }
            IrType::I64 => Some(self.emit_cmp_val(
                IrCmpOp::Eq,
                val,
                Operand::Const(IrConst::I64(i64::MIN)),
                IrType::I64,
            )),
            IrType::I128 => Some(self.emit_cmp_val(
                IrCmpOp::Eq,
                val,
                Operand::Const(IrConst::I128(i128::MIN)),
                IrType::I128,
            )),
            _ => None,
        };
        if let Some(ovf) = min_ovf {
            self.emit_trapv_guard(ovf);
        }
        let dest = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest,
            op: IrUnaryOp::Neg,
            src: val,
            ty: neg_ty,
        });
        dest
    }

    /// Initialize an uninitialized automatic local variable according to
    /// `-ftrivial-auto-var-init={zero,pattern}`.
    ///
    /// Scalar allocas receive a single typed `Store` so `mem2reg` still
    /// promotes them directly into SSA registers with zero stack traffic.
    pub(super) fn auto_init_local_var(
        &mut self,
        alloca: Value,
        scalar_ty: Option<(IrType, bool)>,
        alloc_size: usize,
        vla_size: Option<Value>,
    ) {
        match self.trivial_auto_var_init {
            TrivialAutoVarInit::Uninitialized => {}
            TrivialAutoVarInit::Zero => {
                if let Some(vla_sz) = vla_size {
                    self.emit_memset_zero_dynamic(alloca, vla_sz);
                } else if let Some((ty, _)) = scalar_ty {
                    self.emit(Instruction::Store {
                        volatile: false,
                        val: Operand::Const(IrConst::zero(ty)),
                        ptr: alloca,
                        ty,
                        seg_override: AddressSpace::Default,
                    });
                } else if alloc_size > 0 {
                    self.zero_init_alloca(alloca, alloc_size);
                }
            }
            TrivialAutoVarInit::Pattern => {
                const BYTE: u8 = 0xFE;
                if let Some(vla_sz) = vla_size {
                    let ptr_int_ty = crate::common::types::target_int_ir_type();
                    let sz = self.emit_cast_val(Operand::Value(vla_sz), ptr_int_ty, ptr_int_ty);
                    self.emit(Instruction::Call {
                        func: "memset".to_string(),
                        info: CallInfo {
                            dest: None,
                            args: vec![
                                Operand::Value(alloca),
                                Operand::Const(IrConst::I32(BYTE as i32)),
                                Operand::Value(sz),
                            ],
                            arg_types: vec![IrType::Ptr, IrType::I32, ptr_int_ty],
                            return_type: IrType::Ptr,
                            is_variadic: false,
                            num_fixed_args: 3,
                            ..Default::default()
                        },
                    });
                    return;
                }
                if let Some((ty, is_bool)) = scalar_ty {
                    let pat_const = if is_bool {
                        Some(IrConst::zero(ty))
                    } else {
                        match ty {
                            IrType::I8 | IrType::U8 => Some(IrConst::I8(BYTE as i8)),
                            IrType::I16 | IrType::U16 => Some(IrConst::I16(0xFEFEu16 as i16)),
                            IrType::I32 | IrType::U32 => Some(IrConst::I32(0xFEFE_FEFEu32 as i32)),
                            IrType::I64 | IrType::U64 => {
                                Some(IrConst::I64(0xFEFE_FEFE_FEFE_FEFEu64 as i64))
                            }
                            IrType::Ptr => {
                                if crate::common::types::target_is_32bit() {
                                    Some(IrConst::I32(0xFEFE_FEFEu32 as i32))
                                } else {
                                    Some(IrConst::I64(0xFEFE_FEFE_FEFE_FEFEu64 as i64))
                                }
                            }
                            IrType::I128 | IrType::U128 => Some(IrConst::I128(
                                0xFEFE_FEFE_FEFE_FEFE_FEFE_FEFE_FEFE_FEFEu128 as i128,
                            )),
                            IrType::F32 => Some(IrConst::F32(f32::from_bits(0xFEFE_FEFE))),
                            IrType::F64 => {
                                Some(IrConst::F64(f64::from_bits(0xFEFE_FEFE_FEFE_FEFE)))
                            }
                            _ => None,
                        }
                    };
                    if let Some(c) = pat_const {
                        self.emit(Instruction::Store {
                            volatile: false,
                            val: Operand::Const(c),
                            ptr: alloca,
                            ty,
                            seg_override: AddressSpace::Default,
                        });
                        return;
                    }
                }
                self.pattern_init_region(alloca, alloc_size, BYTE);
            }
        }
    }

    fn pattern_init_region(&mut self, base: Value, total_size: usize, byte: u8) {
        if total_size == 0 {
            return;
        }
        if total_size > ZERO_INIT_STORE_LADDER_MAX {
            let ptr_int_ty = crate::common::types::target_int_ir_type();
            self.emit(Instruction::Call {
                func: "memset".to_string(),
                info: CallInfo {
                    dest: None,
                    args: vec![
                        Operand::Value(base),
                        Operand::Const(IrConst::I32(byte as i32)),
                        Operand::Const(IrConst::ptr_int(total_size as i64)),
                    ],
                    arg_types: vec![IrType::Ptr, IrType::I32, ptr_int_ty],
                    return_type: IrType::Ptr,
                    is_variadic: false,
                    num_fixed_args: 3,
                    ..Default::default()
                },
            });
            return;
        }
        let pat64 = (byte as u64) * 0x0101_0101_0101_0101u64;
        let pat32 = (byte as u32) * 0x0101_0101u32;
        let pat16 = (byte as u16) * 0x0101u16;
        let mut off = 0usize;
        while off < total_size {
            let rem = total_size - off;
            let (ty, step, val) = if rem >= 8 {
                (IrType::I64, 8, IrConst::I64(pat64 as i64))
            } else if rem >= 4 {
                (IrType::I32, 4, IrConst::I32(pat32 as i32))
            } else if rem >= 2 {
                (IrType::I16, 2, IrConst::I16(pat16 as i16))
            } else {
                (IrType::I8, 1, IrConst::I8(byte as i8))
            };
            let dst = if off == 0 {
                base
            } else {
                let p = self.fresh_value();
                self.emit(Instruction::GetElementPtr {
                    dest: p,
                    base,
                    offset: Operand::Const(IrConst::ptr_int(off as i64)),
                    ty: IrType::I8,
                });
                p
            };
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Const(val),
                ptr: dst,
                ty,
                seg_override: AddressSpace::Default,
            });
            off += step;
        }
    }
}
