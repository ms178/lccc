//! IEEE 754 binary16 (`_Float16`) and bfloat16 (`__bf16`) IR lowering.
//!
//! Carrier convention:
//! - Both `_Float16` and `__bf16` ride `IrType::U16` (`IrConst::I16(bits as i16)`),
//!   preserving exact 16-bit IEEE bit patterns across memory, struct fields,
//!   unions, and `__builtin_bit_cast`.
//! - Scalar ABI boundaries mark `Float16` / `BFloat16` parameters, arguments,
//!   and return values with `is_f128_sse = true` alongside `IrType::U16`,
//!   which is completely disjoint from 16-byte `_Float128` (`IrType::U128`)
//!   and routes scalar half-floats through target FP registers (`%xmm0..7` on
//!   x86-64, `%xmm0` return + 4B stack param on i686, `s0..7` on AArch64, and
//!   64-bit NaN-boxed `fa0..7` on RISC-V LP64D).
//! - Conversions `_Float16 <-> float` use hardware `fcvt` on AArch64 and
//!   hardware F16C (`vcvtph2ps` / `vcvtps2ph`) on x86 when F16C is enabled;
//!   otherwise they lower to inline branchless IEEE-754 RNE SSA IR with zero
//!   external `libgcc` soft-float calls (`__extendhfsf2` / `__truncsfhf2`).
//! - Conversions `__bf16 <-> float` always lower to 5–8 inline branchless
//!   integer/SSA instructions with SNaN quieting and exact IEEE RNE rounding,
//!   beating GCC's external `__extendbfsf2` / `__truncsfbf2` libcalls.

use super::lower::Lowerer;
use crate::backend::Target;
use crate::common::const_eval as shared_const_eval;
use crate::common::half_float as hf;
use crate::common::types::{AddressSpace, CType, IrType};
use crate::frontend::parser::ast::{BinOp, Expr};
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{
    CallInfo, Instruction, IrBinOp, IrCmpOp, IrConst, IrUnaryOp, Operand, Value,
};

impl Lowerer {
    /// True when the active target + function attributes provide single-instruction
    /// hardware IEEE binary16 <-> binary32 conversion (`fcvt` on AArch64, F16C on x86).
    pub(super) fn has_hw_f16_convert(&self) -> bool {
        match self.target {
            Target::Aarch64 => true,
            Target::X86_64 | Target::I686 => {
                let features = self
                    .func_state
                    .as_ref()
                    .map_or(&[][..], |fs| &fs.target_features);
                crate::backend::x86::isa::function_features(features, self.target == Target::X86_64)
                    .contains(crate::common::x86_features::X86Feature::F16c)
            }
            Target::Riscv64 => false,
        }
    }

    fn emit_bitcast_u32_to_f32(&mut self, bits_u32: Operand) -> Value {
        let slot = self.emit_entry_alloca(IrType::U32, 4, 4, false);
        self.emit(Instruction::Store {
            volatile: false,
            val: bits_u32,
            ptr: slot,
            ty: IrType::U32,
            seg_override: AddressSpace::Default,
        });
        let dst = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: dst,
            ptr: slot,
            ty: IrType::F32,
            seg_override: AddressSpace::Default,
        });
        dst
    }

    fn emit_bitcast_f32_to_u32(&mut self, val_f32: Operand) -> Value {
        let slot = self.emit_entry_alloca(IrType::F32, 4, 4, false);
        self.emit(Instruction::Store {
            volatile: false,
            val: val_f32,
            ptr: slot,
            ty: IrType::F32,
            seg_override: AddressSpace::Default,
        });
        let dst = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: dst,
            ptr: slot,
            ty: IrType::U32,
            seg_override: AddressSpace::Default,
        });
        dst
    }

    /// Widen a `_Float16` (`IrType::U16`) value to `IrType::F32`.
    pub(super) fn emit_f16_to_f32(&mut self, val_u16: Operand) -> Value {
        if let Operand::Const(c) = &val_u16 {
            if let Some(bits) = shared_const_eval::half_bits_of(c)
                && !self.pass_policy.fp_environment_observable
            {
                let f = hf::f16_bits_to_f32(bits);
                return self.operand_to_value(Operand::Const(IrConst::F32(f)));
            }
        }
        if self.has_hw_f16_convert() {
            let dst = self.fresh_value();
            self.emit(Instruction::Intrinsic {
                dest: Some(dst),
                op: IntrinsicOp::F16ToF32,
                dest_ptr: None,
                args: vec![val_u16],
            });
            return dst;
        }
        self.emit_f16_to_f32_inline(val_u16)
    }

    /// Branchless IEEE-754 binary16 -> binary32 SSA lowering (with SNaN quieting).
    fn emit_f16_to_f32_inline(&mut self, val_u16: Operand) -> Value {
        let c32 = |v: u32| Operand::Const(IrConst::I32(v as i32));
        let h = self.emit_cast_val(val_u16, IrType::U16, IrType::U32);
        let sign_m = self.emit_binop_val(IrBinOp::And, Operand::Value(h), c32(0x8000), IrType::U32);
        let sign = self.emit_binop_val(IrBinOp::Shl, Operand::Value(sign_m), c32(16), IrType::U32);
        let exp_s = self.emit_binop_val(IrBinOp::LShr, Operand::Value(h), c32(10), IrType::U32);
        let exp = self.emit_binop_val(IrBinOp::And, Operand::Value(exp_s), c32(0x1f), IrType::U32);
        let mant = self.emit_binop_val(IrBinOp::And, Operand::Value(h), c32(0x03ff), IrType::U32);

        // Normal: sign | ((exp + 112) << 23) | (mant << 13)
        let exp_n = self.emit_binop_val(IrBinOp::Add, Operand::Value(exp), c32(112), IrType::U32);
        let exp_sh = self.emit_binop_val(IrBinOp::Shl, Operand::Value(exp_n), c32(23), IrType::U32);
        let mant_sh = self.emit_binop_val(IrBinOp::Shl, Operand::Value(mant), c32(13), IrType::U32);
        let norm_t = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(sign),
            Operand::Value(exp_sh),
            IrType::U32,
        );
        let norm_u32 = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(norm_t),
            Operand::Value(mant_sh),
            IrType::U32,
        );

        // Inf/NaN (exp == 31): quiet SNaN when mant != 0
        let mant_nz = self.emit_cmp_val(IrCmpOp::Ne, Operand::Value(mant), c32(0), IrType::U32);
        let qbit = self.emit_select_val(
            Operand::Value(mant_nz),
            c32(0x0040_0000),
            c32(0),
            IrType::U32,
        );
        let inf_base = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(sign),
            c32(0x7f80_0000),
            IrType::U32,
        );
        let inf_m = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(inf_base),
            Operand::Value(mant_sh),
            IrType::U32,
        );
        let infnan_u32 = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(inf_m),
            Operand::Value(qbit),
            IrType::U32,
        );

        // Subnormal (exp == 0 && mant != 0): lz in 22..=31 for safe_m in 1..=0x3ff
        let safe_m = self.emit_select_val(
            Operand::Value(mant_nz),
            Operand::Value(mant),
            c32(1),
            IrType::U32,
        );
        let lz = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest: lz,
            op: IrUnaryOp::Clz,
            src: Operand::Value(safe_m),
            ty: IrType::U32,
        });
        let sub_sh = self.emit_binop_val(IrBinOp::Sub, Operand::Value(lz), c32(8), IrType::U32);
        let sub_raw = self.emit_binop_val(
            IrBinOp::Shl,
            Operand::Value(safe_m),
            Operand::Value(sub_sh),
            IrType::U32,
        );
        let sub_m = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(sub_raw),
            c32(0x007f_ffff),
            IrType::U32,
        );
        let sub_exp = self.emit_binop_val(IrBinOp::Sub, c32(134), Operand::Value(lz), IrType::U32);
        let sub_exp_sh =
            self.emit_binop_val(IrBinOp::Shl, Operand::Value(sub_exp), c32(23), IrType::U32);
        let sub_t = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(sign),
            Operand::Value(sub_exp_sh),
            IrType::U32,
        );
        let sub_u32 = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(sub_t),
            Operand::Value(sub_m),
            IrType::U32,
        );
        let zero_or_sub = self.emit_select_val(
            Operand::Value(mant_nz),
            Operand::Value(sub_u32),
            Operand::Value(sign),
            IrType::U32,
        );

        let exp_is_31 = self.emit_cmp_val(IrCmpOp::Eq, Operand::Value(exp), c32(31), IrType::U32);
        let exp_is_0 = self.emit_cmp_val(IrCmpOp::Eq, Operand::Value(exp), c32(0), IrType::U32);
        let non_zero_exp = self.emit_select_val(
            Operand::Value(exp_is_31),
            Operand::Value(infnan_u32),
            Operand::Value(norm_u32),
            IrType::U32,
        );
        let out_u32 = self.emit_select_val(
            Operand::Value(exp_is_0),
            Operand::Value(zero_or_sub),
            Operand::Value(non_zero_exp),
            IrType::U32,
        );
        self.emit_bitcast_u32_to_f32(Operand::Value(out_u32))
    }

    /// Narrow an `IrType::F32` value to `_Float16` (`IrType::U16`) under IEEE-754 RNE.
    pub(super) fn emit_f32_to_f16(&mut self, val_f32: Operand) -> Value {
        if let Operand::Const(IrConst::F32(f)) = &val_f32 {
            if !self.pass_policy.fp_environment_observable {
                let bits = hf::f32_to_f16_bits(*f);
                return self.operand_to_value(Operand::Const(IrConst::I16(bits as i16)));
            }
        }
        if self.has_hw_f16_convert() {
            let dst = self.fresh_value();
            self.emit(Instruction::Intrinsic {
                dest: Some(dst),
                op: IntrinsicOp::F32ToF16,
                dest_ptr: None,
                args: vec![val_f32],
            });
            return dst;
        }
        self.emit_f32_to_f16_inline(val_f32)
    }

    /// Branchless IEEE-754 RNE binary32 -> binary16 SSA lowering.
    fn emit_f32_to_f16_inline(&mut self, val_f32: Operand) -> Value {
        let c32 = |v: u32| Operand::Const(IrConst::I32(v as i32));
        let u = self.emit_bitcast_f32_to_u32(val_f32);
        let sign_s = self.emit_binop_val(IrBinOp::LShr, Operand::Value(u), c32(16), IrType::U32);
        let sign = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(sign_s),
            c32(0x8000),
            IrType::U32,
        );
        let abs = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(u),
            c32(0x7fff_ffff),
            IrType::U32,
        );

        // NaN: sign | 0x7e00 | ((abs >> 13) & 0x01ff)
        let nan_pl_s =
            self.emit_binop_val(IrBinOp::LShr, Operand::Value(abs), c32(13), IrType::U32);
        let nan_pl = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(nan_pl_s),
            c32(0x01ff),
            IrType::U32,
        );
        let nan_base =
            self.emit_binop_val(IrBinOp::Or, Operand::Value(sign), c32(0x7e00), IrType::U32);
        let nan_h = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(nan_base),
            Operand::Value(nan_pl),
            IrType::U32,
        );
        let inf_h =
            self.emit_binop_val(IrBinOp::Or, Operand::Value(sign), c32(0x7c00), IrType::U32);

        // Normal RNE (0x3880_0000 <= abs < 0x4780_0000; 65520.0 ties overflow to 0x7c00):
        let odd_bit =
            self.emit_binop_val(IrBinOp::And, Operand::Value(nan_pl_s), c32(1), IrType::U32);
        let norm_bias = self.emit_binop_val(
            IrBinOp::Sub,
            Operand::Value(abs),
            c32(0x3800_0000 - 0x0fff),
            IrType::U32,
        );
        let norm_rnd = self.emit_binop_val(
            IrBinOp::Add,
            Operand::Value(norm_bias),
            Operand::Value(odd_bit),
            IrType::U32,
        );
        let norm_sh = self.emit_binop_val(
            IrBinOp::LShr,
            Operand::Value(norm_rnd),
            c32(13),
            IrType::U32,
        );
        let norm_h = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(sign),
            Operand::Value(norm_sh),
            IrType::U32,
        );

        // Subnormal RNE (0x3300_0000 <= abs < 0x3880_0000): shift in 14..=24
        let exp32 = self.emit_binop_val(IrBinOp::LShr, Operand::Value(abs), c32(23), IrType::U32);
        let raw_s = self.emit_binop_val(IrBinOp::Sub, c32(126), Operand::Value(exp32), IrType::U32);
        let lt14 = self.emit_cmp_val(IrCmpOp::Ult, Operand::Value(raw_s), c32(14), IrType::U32);
        let s_min = self.emit_select_val(
            Operand::Value(lt14),
            c32(14),
            Operand::Value(raw_s),
            IrType::U32,
        );
        let gt24 = self.emit_cmp_val(IrCmpOp::Ugt, Operand::Value(s_min), c32(24), IrType::U32);
        let shift = self.emit_select_val(
            Operand::Value(gt24),
            c32(24),
            Operand::Value(s_min),
            IrType::U32,
        );

        let m_low = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(abs),
            c32(0x007f_ffff),
            IrType::U32,
        );
        let full_m = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(m_low),
            c32(0x0080_0000),
            IrType::U32,
        );
        let q = self.emit_binop_val(
            IrBinOp::LShr,
            Operand::Value(full_m),
            Operand::Value(shift),
            IrType::U32,
        );
        let one_sh = self.emit_binop_val(IrBinOp::Shl, c32(1), Operand::Value(shift), IrType::U32);
        let rem_mask =
            self.emit_binop_val(IrBinOp::Sub, Operand::Value(one_sh), c32(1), IrType::U32);
        let rem = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(full_m),
            Operand::Value(rem_mask),
            IrType::U32,
        );
        let sm1 = self.emit_binop_val(IrBinOp::Sub, Operand::Value(shift), c32(1), IrType::U32);
        let half = self.emit_binop_val(IrBinOp::Shl, c32(1), Operand::Value(sm1), IrType::U32);
        let gt_half = self.emit_cmp_val(
            IrCmpOp::Ugt,
            Operand::Value(rem),
            Operand::Value(half),
            IrType::U32,
        );
        let eq_half = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(rem),
            Operand::Value(half),
            IrType::U32,
        );
        let q_odd_b = self.emit_binop_val(IrBinOp::And, Operand::Value(q), c32(1), IrType::U32);
        let q_odd = self.emit_cmp_val(IrCmpOp::Ne, Operand::Value(q_odd_b), c32(0), IrType::U32);
        let tie_up = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(eq_half),
            Operand::Value(q_odd),
            IrType::I32,
        );
        let do_inc = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(gt_half),
            Operand::Value(tie_up),
            IrType::I32,
        );
        let inc_u32 = self.emit_cast_val(Operand::Value(do_inc), IrType::I32, IrType::U32);
        let q_rnd = self.emit_binop_val(
            IrBinOp::Add,
            Operand::Value(q),
            Operand::Value(inc_u32),
            IrType::U32,
        );
        let sub_h = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(sign),
            Operand::Value(q_rnd),
            IrType::U32,
        );

        let is_zero = self.emit_cmp_val(
            IrCmpOp::Ult,
            Operand::Value(abs),
            c32(0x3300_0000),
            IrType::U32,
        );
        let is_sub = self.emit_cmp_val(
            IrCmpOp::Ult,
            Operand::Value(abs),
            c32(0x3880_0000),
            IrType::U32,
        );
        let is_ovf = self.emit_cmp_val(
            IrCmpOp::Uge,
            Operand::Value(abs),
            c32(0x4780_0000),
            IrType::U32,
        );
        let is_nan = self.emit_cmp_val(
            IrCmpOp::Ugt,
            Operand::Value(abs),
            c32(0x7f80_0000),
            IrType::U32,
        );

        let sub_or_zero = self.emit_select_val(
            Operand::Value(is_zero),
            Operand::Value(sign),
            Operand::Value(sub_h),
            IrType::U32,
        );
        let fin_h = self.emit_select_val(
            Operand::Value(is_sub),
            Operand::Value(sub_or_zero),
            Operand::Value(norm_h),
            IrType::U32,
        );
        let ovf_or_fin = self.emit_select_val(
            Operand::Value(is_ovf),
            Operand::Value(inf_h),
            Operand::Value(fin_h),
            IrType::U32,
        );
        let res_u32 = self.emit_select_val(
            Operand::Value(is_nan),
            Operand::Value(nan_h),
            Operand::Value(ovf_or_fin),
            IrType::U32,
        );
        self.emit_cast_val(Operand::Value(res_u32), IrType::U32, IrType::U16)
    }

    /// Widen a `__bf16` (`IrType::U16`) value to `IrType::F32` (with SNaN quieting).
    pub(super) fn emit_bf16_to_f32(&mut self, val_u16: Operand) -> Value {
        if let Operand::Const(c) = &val_u16 {
            if let Some(bits) = shared_const_eval::half_bits_of(c)
                && !self.pass_policy.fp_environment_observable
            {
                let f = hf::bf16_bits_to_f32(bits);
                return self.operand_to_value(Operand::Const(IrConst::F32(f)));
            }
        }
        let c32 = |v: u32| Operand::Const(IrConst::I32(v as i32));
        let b = self.emit_cast_val(val_u16, IrType::U16, IrType::U32);
        let abs = self.emit_binop_val(IrBinOp::And, Operand::Value(b), c32(0x7fff), IrType::U32);
        let is_nan = self.emit_cmp_val(IrCmpOp::Ugt, Operand::Value(abs), c32(0x7f80), IrType::U32);
        let b_q = self.emit_binop_val(IrBinOp::Or, Operand::Value(b), c32(0x0040), IrType::U32);
        let b_sel = self.emit_select_val(
            Operand::Value(is_nan),
            Operand::Value(b_q),
            Operand::Value(b),
            IrType::U32,
        );
        let u32_bits =
            self.emit_binop_val(IrBinOp::Shl, Operand::Value(b_sel), c32(16), IrType::U32);
        self.emit_bitcast_u32_to_f32(Operand::Value(u32_bits))
    }

    /// Narrow an `IrType::F32` value to `__bf16` (`IrType::U16`) under IEEE-754 RNE.
    pub(super) fn emit_f32_to_bf16(&mut self, val_f32: Operand) -> Value {
        if let Operand::Const(IrConst::F32(f)) = &val_f32 {
            if !self.pass_policy.fp_environment_observable {
                let bits = hf::f32_to_bf16_bits(*f);
                return self.operand_to_value(Operand::Const(IrConst::I16(bits as i16)));
            }
        }
        let c32 = |v: u32| Operand::Const(IrConst::I32(v as i32));
        let u = self.emit_bitcast_f32_to_u32(val_f32);
        let abs = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(u),
            c32(0x7fff_ffff),
            IrType::U32,
        );
        let top16 = self.emit_binop_val(IrBinOp::LShr, Operand::Value(u), c32(16), IrType::U32);
        let lsb = self.emit_binop_val(IrBinOp::And, Operand::Value(top16), c32(1), IrType::U32);
        let bias = self.emit_binop_val(IrBinOp::Add, Operand::Value(lsb), c32(0x7fff), IrType::U32);
        let sum = self.emit_binop_val(
            IrBinOp::Add,
            Operand::Value(u),
            Operand::Value(bias),
            IrType::U32,
        );
        let rne = self.emit_binop_val(IrBinOp::LShr, Operand::Value(sum), c32(16), IrType::U32);
        let nan_bf =
            self.emit_binop_val(IrBinOp::Or, Operand::Value(top16), c32(0x0040), IrType::U32);
        let is_nan = self.emit_cmp_val(
            IrCmpOp::Ugt,
            Operand::Value(abs),
            c32(0x7f80_0000),
            IrType::U32,
        );
        let sel = self.emit_select_val(
            Operand::Value(is_nan),
            Operand::Value(nan_bf),
            Operand::Value(rne),
            IrType::U32,
        );
        self.emit_cast_val(Operand::Value(sel), IrType::U32, IrType::U16)
    }

    /// Boldo–Melquiond round-to-odd `F64 -> F32` so `double -> _Float16 / __bf16`
    /// never suffers double-rounding on exact halfway ties.
    fn emit_f64_to_f32_rto(&mut self, val_f64: Operand) -> Value {
        let c32 = |v: u32| Operand::Const(IrConst::I32(v as i32));
        let f = self.emit_cast_val(val_f64.clone(), IrType::F64, IrType::F32);
        let d_back = self.emit_cast_val(Operand::Value(f), IrType::F32, IrType::F64);
        let f_bits = self.emit_bitcast_f32_to_u32(Operand::Value(f));

        let abs_f = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(f_bits),
            c32(0x7fff_ffff),
            IrType::U32,
        );
        let finite = self.emit_cmp_val(
            IrCmpOp::Ult,
            Operand::Value(abs_f),
            c32(0x7f80_0000),
            IrType::U32,
        );
        let low1 = self.emit_binop_val(IrBinOp::And, Operand::Value(f_bits), c32(1), IrType::U32);
        let is_even = self.emit_cmp_val(IrCmpOp::Eq, Operand::Value(low1), c32(0), IrType::U32);
        let inexact = self.emit_cmp_val(
            IrCmpOp::Ne,
            val_f64.clone(),
            Operand::Value(d_back),
            IrType::F64,
        );
        let c1 = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(finite),
            Operand::Value(is_even),
            IrType::I32,
        );
        let need_fix = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(c1),
            Operand::Value(inexact),
            IrType::I32,
        );

        // Determine whether |d_back| < |d|: if so f_bits | 1, else f_bits - 1.
        let pos = self.emit_cmp_val(
            IrCmpOp::Sgt,
            val_f64.clone(),
            Operand::Const(IrConst::F64(0.0)),
            IrType::F64,
        );
        let lt_pos = self.emit_cmp_val(
            IrCmpOp::Slt,
            Operand::Value(d_back),
            val_f64.clone(),
            IrType::F64,
        );
        let gt_neg = self.emit_cmp_val(IrCmpOp::Sgt, Operand::Value(d_back), val_f64, IrType::F64);
        let mag_smaller = self.emit_select_val(
            Operand::Value(pos),
            Operand::Value(lt_pos),
            Operand::Value(gt_neg),
            IrType::I32,
        );

        let bits_or1 =
            self.emit_binop_val(IrBinOp::Or, Operand::Value(f_bits), c32(1), IrType::U32);
        let bits_sub1 =
            self.emit_binop_val(IrBinOp::Sub, Operand::Value(f_bits), c32(1), IrType::U32);
        let adjusted = self.emit_select_val(
            Operand::Value(mag_smaller),
            Operand::Value(bits_or1),
            Operand::Value(bits_sub1),
            IrType::U32,
        );
        let final_bits = self.emit_select_val(
            Operand::Value(need_fix),
            Operand::Value(adjusted),
            Operand::Value(f_bits),
            IrType::U32,
        );
        self.emit_bitcast_u32_to_f32(Operand::Value(final_bits))
    }

    /// Widen either `_Float16` or `__bf16` (`src_ct`) to `IrType::F32`.
    pub(super) fn emit_half_to_f32(&mut self, val_u16: Operand, src_ct: &CType) -> Value {
        if *src_ct == CType::BFloat16 {
            self.emit_bf16_to_f32(val_u16)
        } else {
            self.emit_f16_to_f32(val_u16)
        }
    }

    /// Narrow `IrType::F32` to either `_Float16` or `__bf16` (`dst_ct`).
    pub(super) fn emit_f32_to_half(&mut self, val_f32: Operand, dst_ct: &CType) -> Value {
        if *dst_ct == CType::BFloat16 {
            self.emit_f32_to_bf16(val_f32)
        } else {
            self.emit_f32_to_f16(val_f32)
        }
    }

    /// Convert any scalar `src` (`src_ct`) to `_Float16` or `__bf16` (`target_ct`).
    pub(super) fn convert_to_half(
        &mut self,
        src: Operand,
        src_ty: IrType,
        src_ct: &CType,
        target_ct: &CType,
    ) -> Operand {
        if src_ct == target_ct {
            return src;
        }
        if src_ct.is_half_float() {
            let f32_v = self.emit_half_to_f32(src, src_ct);
            return Operand::Value(self.emit_f32_to_half(Operand::Value(f32_v), target_ct));
        }
        let f32_val = match src_ct {
            CType::Float => self.operand_to_value(src),
            CType::Double => self.emit_f64_to_f32_rto(src),
            CType::LongDouble => {
                // Figueroa's bound (53 >= 2*11 + 2): long double -> F64 -> RTO F32 -> half is exact.
                let d = self.emit_cast_val(src, IrType::F128, IrType::F64);
                self.emit_f64_to_f32_rto(Operand::Value(d))
            }
            CType::Float128 => self.emit_f128_to_f32_rto(src),
            _ if src_ct.is_integer() => {
                if src_ty.size() <= 4 {
                    // Every <=32-bit integer is exact in F64 (53 bits), and RTO F64->F32
                    // preserves exact midpoint ordering for both _Float16 and __bf16.
                    let d = self.emit_cast_val(src, src_ty, IrType::F64);
                    self.emit_f64_to_f32_rto(Operand::Value(d))
                } else {
                    self.emit_wide_int_to_f32_rto(src, src_ty, !src_ct.is_unsigned())
                }
            }
            _ => self.emit_cast_val(src, src_ty, IrType::F32),
        };
        Operand::Value(self.emit_f32_to_half(Operand::Value(f32_val), target_ct))
    }

    fn emit_u64_to_f32_rto(&mut self, u: Operand) -> Value {
        let c64 = |v: u64| Operand::Const(IrConst::I64(v as i64));
        let hi32 = self.emit_binop_val(IrBinOp::LShr, u, c64(32), IrType::U64);
        let is_large = self.emit_cmp_val(IrCmpOp::Ne, Operand::Value(hi32), c64(0), IrType::U64);
        let low12 = self.emit_binop_val(IrBinOp::And, u, c64(0xfff), IrType::U64);
        let has_low = self.emit_cmp_val(IrCmpOp::Ne, Operand::Value(low12), c64(0), IrType::U64);
        let has_low_u64 = self.emit_cast_val(Operand::Value(has_low), IrType::I32, IrType::U64);
        let sticky = self.emit_binop_val(
            IrBinOp::Shl,
            Operand::Value(has_low_u64),
            c64(12),
            IrType::U64,
        );
        let cleared = self.emit_binop_val(IrBinOp::And, u, c64(!0xfff), IrType::U64);
        let jammed = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(cleared),
            Operand::Value(sticky),
            IrType::U64,
        );
        let u_rto = self.emit_select_val(
            Operand::Value(is_large),
            Operand::Value(jammed),
            u,
            IrType::U64,
        );
        let d = self.emit_cast_val(Operand::Value(u_rto), IrType::U64, IrType::F64);
        self.emit_f64_to_f32_rto(Operand::Value(d))
    }

    fn emit_wide_int_to_f32_rto(&mut self, src: Operand, src_ty: IrType, signed: bool) -> Value {
        let c64 = |v: u64| Operand::Const(IrConst::I64(v as i64));
        if !src_ty.is_128bit() {
            if !signed {
                let u = self.emit_cast_val(src, src_ty, IrType::U64);
                return self.emit_u64_to_f32_rto(Operand::Value(u));
            }
            let s = self.emit_cast_val(src, src_ty, IrType::I64);
            let is_neg = self.emit_cmp_val(IrCmpOp::Slt, Operand::Value(s), c64(0), IrType::I64);
            let u = self.emit_cast_val(Operand::Value(s), IrType::I64, IrType::U64);
            let neg_u = self.emit_binop_val(IrBinOp::Sub, c64(0), Operand::Value(u), IrType::U64);
            let mag = self.emit_select_val(
                Operand::Value(is_neg),
                Operand::Value(neg_u),
                Operand::Value(u),
                IrType::U64,
            );
            let pos_f = self.emit_u64_to_f32_rto(Operand::Value(mag));
            let neg_f = self.fresh_value();
            self.emit(Instruction::UnaryOp {
                dest: neg_f,
                op: crate::ir::reexports::IrUnaryOp::Neg,
                src: Operand::Value(pos_f),
                ty: IrType::F32,
            });
            return self.emit_select_val(
                Operand::Value(is_neg),
                Operand::Value(neg_f),
                Operand::Value(pos_f),
                IrType::F32,
            );
        }
        let u128_v = self.emit_cast_val(src, src_ty, IrType::U128);
        let lo = self.emit_cast_val(Operand::Value(u128_v), IrType::U128, IrType::U64);
        let sh64 = self.emit_binop_val(
            IrBinOp::LShr,
            Operand::Value(u128_v),
            Operand::Const(IrConst::I128(64)),
            IrType::U128,
        );
        let hi = self.emit_cast_val(Operand::Value(sh64), IrType::U128, IrType::U64);
        let (mag_hi, mag_lo, is_neg_opt) = if signed {
            let sign_bit =
                self.emit_binop_val(IrBinOp::LShr, Operand::Value(hi), c64(63), IrType::U64);
            let is_neg =
                self.emit_cmp_val(IrCmpOp::Ne, Operand::Value(sign_bit), c64(0), IrType::U64);
            let neg128 = self.emit_binop_val(
                IrBinOp::Sub,
                Operand::Const(IrConst::I128(0)),
                Operand::Value(u128_v),
                IrType::U128,
            );
            let nlo = self.emit_cast_val(Operand::Value(neg128), IrType::U128, IrType::U64);
            let nsh = self.emit_binop_val(
                IrBinOp::LShr,
                Operand::Value(neg128),
                Operand::Const(IrConst::I128(64)),
                IrType::U128,
            );
            let nhi = self.emit_cast_val(Operand::Value(nsh), IrType::U128, IrType::U64);
            let mlo = self.emit_select_val(
                Operand::Value(is_neg),
                Operand::Value(nlo),
                Operand::Value(lo),
                IrType::U64,
            );
            let mhi = self.emit_select_val(
                Operand::Value(is_neg),
                Operand::Value(nhi),
                Operand::Value(hi),
                IrType::U64,
            );
            (mhi, mlo, Some(is_neg))
        } else {
            (hi, lo, None)
        };
        let lo_nz = self.emit_cmp_val(IrCmpOp::Ne, Operand::Value(mag_lo), c64(0), IrType::U64);
        let lo_sticky = self.emit_cast_val(Operand::Value(lo_nz), IrType::I32, IrType::U64);
        let hi_sticky = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(mag_hi),
            Operand::Value(lo_sticky),
            IrType::U64,
        );
        let hi_f32 = self.emit_u64_to_f32_rto(Operand::Value(hi_sticky));
        let scale_2p64 = Operand::Const(IrConst::F32(f32::from_bits(0x5f80_0000)));
        let hi_scaled = self.emit_binop_val(
            IrBinOp::Mul,
            Operand::Value(hi_f32),
            scale_2p64,
            IrType::F32,
        );
        let lo_f32 = self.emit_u64_to_f32_rto(Operand::Value(mag_lo));
        let hi_nz = self.emit_cmp_val(IrCmpOp::Ne, Operand::Value(mag_hi), c64(0), IrType::U64);
        let mag_f32 = self.emit_select_val(
            Operand::Value(hi_nz),
            Operand::Value(hi_scaled),
            Operand::Value(lo_f32),
            IrType::F32,
        );
        if let Some(is_neg) = is_neg_opt {
            let neg_f = self.fresh_value();
            self.emit(Instruction::UnaryOp {
                dest: neg_f,
                op: crate::ir::reexports::IrUnaryOp::Neg,
                src: Operand::Value(mag_f32),
                ty: IrType::F32,
            });
            self.emit_select_val(
                Operand::Value(is_neg),
                Operand::Value(neg_f),
                Operand::Value(mag_f32),
                IrType::F32,
            )
        } else {
            mag_f32
        }
    }

    fn emit_f128_to_f32_rto(&mut self, src: Operand) -> Value {
        let c64 = |v: u64| Operand::Const(IrConst::I64(v as i64));
        let slot = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: slot,
            ty: IrType::U128,
            size: 16,
            align: 16,
            volatile: false,
            semantic_volatile: false,
        });
        self.emit(Instruction::Store {
            val: src,
            ptr: slot,
            ty: IrType::U128,
            volatile: false,
            seg_override: crate::common::types::AddressSpace::Default,
        });
        let lo = self.fresh_value();
        self.emit(Instruction::Load {
            dest: lo,
            ptr: slot,
            ty: IrType::U64,
            volatile: false,
            seg_override: crate::common::types::AddressSpace::Default,
        });
        let hi_ptr = self.emit_binop_val(IrBinOp::Add, Operand::Value(slot), c64(8), IrType::Ptr);
        let hi = self.fresh_value();
        self.emit(Instruction::Load {
            dest: hi,
            ptr: hi_ptr,
            ty: IrType::U64,
            volatile: false,
            seg_override: crate::common::types::AddressSpace::Default,
        });
        let exp_sh = self.emit_binop_val(IrBinOp::LShr, Operand::Value(hi), c64(48), IrType::U64);
        let exp15 = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(exp_sh),
            c64(0x7fff),
            IrType::U64,
        );
        let is_fin =
            self.emit_cmp_val(IrCmpOp::Ne, Operand::Value(exp15), c64(0x7fff), IrType::U64);
        let mask60 = (1u64 << 60) - 1;
        let low60 = self.emit_binop_val(IrBinOp::And, Operand::Value(lo), c64(mask60), IrType::U64);
        let has_low = self.emit_cmp_val(IrCmpOp::Ne, Operand::Value(low60), c64(0), IrType::U64);
        let has_u64 = self.emit_cast_val(Operand::Value(has_low), IrType::I32, IrType::U64);
        let sticky =
            self.emit_binop_val(IrBinOp::Shl, Operand::Value(has_u64), c64(60), IrType::U64);
        let cleared =
            self.emit_binop_val(IrBinOp::And, Operand::Value(lo), c64(!mask60), IrType::U64);
        let jammed = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(cleared),
            Operand::Value(sticky),
            IrType::U64,
        );
        let adj_lo = self.emit_select_val(
            Operand::Value(is_fin),
            Operand::Value(jammed),
            Operand::Value(lo),
            IrType::U64,
        );
        self.emit(Instruction::Store {
            val: Operand::Value(adj_lo),
            ptr: slot,
            ty: IrType::U64,
            volatile: false,
            seg_override: crate::common::types::AddressSpace::Default,
        });
        let adj_f128 = self.fresh_value();
        self.emit(Instruction::Load {
            dest: adj_f128,
            ptr: slot,
            ty: IrType::U128,
            volatile: false,
            seg_override: crate::common::types::AddressSpace::Default,
        });
        let d = self.convert_from_f128(Operand::Value(adj_f128), &CType::Double);
        self.emit_f64_to_f32_rto(d)
    }

    /// Convert `_Float16` or `__bf16` (`src_ct`) to any scalar `target_ct`.
    pub(super) fn convert_from_half(
        &mut self,
        src: Operand,
        src_ct: &CType,
        target_ct: &CType,
    ) -> Operand {
        if src_ct == target_ct {
            return src;
        }
        if *target_ct == CType::Bool {
            let truth = self.lower_soft_float_truthiness(src, src_ct);
            return self.emit_implicit_cast(truth, IrType::I32, IrType::U8);
        }
        if target_ct.is_half_float() {
            let f32_v = self.emit_half_to_f32(src, src_ct);
            return Operand::Value(self.emit_f32_to_half(Operand::Value(f32_v), target_ct));
        }
        let f32_v = Operand::Value(self.emit_half_to_f32(src, src_ct));
        if *target_ct == CType::Float {
            return f32_v;
        }
        if *target_ct == CType::Float128 {
            return self.convert_to_f128(f32_v, &CType::Float);
        }
        if target_ct.is_decimal() {
            return self.convert_decimal_scalar(f32_v, IrType::F32, &CType::Float, target_ct);
        }
        let dst_ir = IrType::from_ctype(target_ct);
        self.emit_implicit_cast(f32_v, IrType::F32, dst_ir)
    }

    /// Lower binary arithmetic or relational comparison when at least one side is `_Float16` / `__bf16`.
    pub(super) fn lower_half_binop(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        lhs_ct: &CType,
        rhs: &Expr,
        rhs_ct: &CType,
    ) -> Operand {
        let common_ct = CType::usual_arithmetic_conversion(lhs_ct, rhs_ct);
        let l_raw = self.lower_expr(lhs);
        let r_raw = self.lower_expr(rhs);
        let l_ty = self.value_ir_type(lhs);
        let r_ty = self.value_ir_type(rhs);

        if matches!(
            op,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        ) {
            // If common_ct is half-float, compare in F32 (lossless widening of both operands!).
            let cmp_ct = if common_ct.is_half_float() {
                CType::Float
            } else {
                common_ct
            };
            let l = self.convert_scalar_ctype(l_raw, l_ty, lhs_ct, &cmp_ct);
            let r = self.convert_scalar_ctype(r_raw, r_ty, rhs_ct, &cmp_ct);
            if cmp_ct == CType::Float128 {
                let (helper, cmp_op) = match op {
                    BinOp::Ne => ("__netf2", IrCmpOp::Ne),
                    BinOp::Lt => ("__lttf2", IrCmpOp::Slt),
                    BinOp::Le => ("__letf2", IrCmpOp::Sle),
                    BinOp::Gt => ("__gttf2", IrCmpOp::Sgt),
                    BinOp::Ge => ("__getf2", IrCmpOp::Sge),
                    _ => ("__eqtf2", IrCmpOp::Eq),
                };
                let d = self.emit_f128_cmp_call(helper, l, r);
                let z = self.emit_cmp_val(
                    cmp_op,
                    Operand::Value(d),
                    Operand::Const(IrConst::I64(0)),
                    IrType::I32,
                );
                return Operand::Value(z);
            }
            let cmp_ir = IrType::from_ctype(&cmp_ct);
            let cmp_op = Self::binop_to_cmp(*op, false);
            return Operand::Value(self.emit_cmp_val(cmp_op, l, r, cmp_ir));
        }

        let l = self.convert_scalar_ctype(l_raw, l_ty, lhs_ct, &common_ct);
        let r = self.convert_scalar_ctype(r_raw, r_ty, rhs_ct, &common_ct);
        Operand::Value(self.emit_arith_in_ctype(op, l, r, &common_ct))
    }

    /// Lower all 19 `__builtin_*f16*` intrinsics (`BuiltinIntrinsic::HalfFloat`).
    pub(super) fn lower_half_float_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Option<Operand> {
        let expected_arity = match name {
            "__builtin_inff16" | "__builtin_huge_valf16" => 0,
            "__builtin_nanf16"
            | "__builtin_nansf16"
            | "__builtin_nansf16b"
            | "__builtin_fabsf16"
            | "__builtin_ceilf16"
            | "__builtin_floorf16"
            | "__builtin_truncf16"
            | "__builtin_roundf16"
            | "__builtin_roundevenf16"
            | "__builtin_rintf16"
            | "__builtin_nearbyintf16"
            | "__builtin_sqrtf16" => 1,
            "__builtin_copysignf16"
            | "__builtin_fminf16"
            | "__builtin_fmaxf16"
            | "__builtin_nextafterf16" => 2,
            "__builtin_fmaf16" => 3,
            _ => 1,
        };
        let call_span = args
            .first()
            .map(|a| a.span())
            .unwrap_or_else(crate::common::source::Span::dummy);
        if args.len() != expected_arity {
            self.diagnostics.borrow_mut().error(
                format!(
                    "'{}' requires {} argument{}, got {}",
                    name,
                    expected_arity,
                    if expected_arity == 1 { "" } else { "s" },
                    args.len()
                ),
                call_span,
            );
            return Some(Operand::Const(IrConst::I16(0)));
        }

        if matches!(name, "__builtin_inff16" | "__builtin_huge_valf16") {
            return Some(Operand::Const(IrConst::I16(0x7c00u16 as i16)));
        }

        if matches!(
            name,
            "__builtin_nanf16" | "__builtin_nansf16" | "__builtin_nansf16b"
        ) {
            let arg_ct = self.expr_ctype(&args[0]);
            if !matches!(arg_ct, CType::Pointer(..) | CType::Array(..)) {
                self.diagnostics.borrow_mut().error(
                    format!(
                        "incompatible argument type for '{}': expected 'const char *'",
                        name
                    ),
                    args[0].span(),
                );
                return Some(Operand::Const(IrConst::I16(0)));
            }
            if let Some(c) =
                shared_const_eval::eval_builtin_call(name, args, &|e| self.eval_const_expr(e))
            {
                return Some(Operand::Const(c));
            }
            // Runtime payload: call C99 `nanf(s)` and transplant the payload bits.
            let s_val = self.lower_expr(&args[0]);
            let f_dest = self.fresh_value();
            self.emit(Instruction::Call {
                func: "nanf".to_string(),
                info: CallInfo {
                    dest: Some(f_dest),
                    args: vec![s_val],
                    arg_types: vec![IrType::Ptr],
                    return_type: IrType::F32,
                    is_variadic: false,
                    num_fixed_args: 1,
                    struct_arg_sizes: vec![None],
                    struct_arg_aligns: vec![],
                    struct_arg_classes: Vec::new(),
                    struct_arg_riscv_float_classes: Vec::new(),
                    struct_arg_is_f128_sse: Vec::new(),
                    is_sret: false,
                    calling_convention: crate::common::types::CallingConvention::Default,
                    is_fastcall: false,
                    regparm: None,
                    is_pure: true,
                    is_const: false,
                    assume_library: true,
                    ret_eightbyte_classes: Vec::new(),
                    ret_is_f128_sse: false,
                    cold: false,
                },
            });
            let u32_v = self.emit_bitcast_f32_to_u32(Operand::Value(f_dest));
            let c32 = |v: u32| Operand::Const(IrConst::I32(v as i32));
            let out_u32 = match name {
                "__builtin_nanf16" => {
                    let pl = self.emit_binop_val(
                        IrBinOp::LShr,
                        Operand::Value(u32_v),
                        c32(13),
                        IrType::U32,
                    );
                    let plm = self.emit_binop_val(
                        IrBinOp::And,
                        Operand::Value(pl),
                        c32(0x01ff),
                        IrType::U32,
                    );
                    self.emit_binop_val(IrBinOp::Or, Operand::Value(plm), c32(0x7e00), IrType::U32)
                }
                "__builtin_nansf16" => {
                    let pl = self.emit_binop_val(
                        IrBinOp::LShr,
                        Operand::Value(u32_v),
                        c32(13),
                        IrType::U32,
                    );
                    let plm = self.emit_binop_val(
                        IrBinOp::And,
                        Operand::Value(pl),
                        c32(0x01ff),
                        IrType::U32,
                    );
                    let z =
                        self.emit_cmp_val(IrCmpOp::Eq, Operand::Value(plm), c32(0), IrType::U32);
                    let nz = self.emit_select_val(
                        Operand::Value(z),
                        c32(0x0100),
                        Operand::Value(plm),
                        IrType::U32,
                    );
                    self.emit_binop_val(IrBinOp::Or, Operand::Value(nz), c32(0x7c00), IrType::U32)
                }
                _ => {
                    let pl = self.emit_binop_val(
                        IrBinOp::LShr,
                        Operand::Value(u32_v),
                        c32(16),
                        IrType::U32,
                    );
                    let plm = self.emit_binop_val(
                        IrBinOp::And,
                        Operand::Value(pl),
                        c32(0x003f),
                        IrType::U32,
                    );
                    let z =
                        self.emit_cmp_val(IrCmpOp::Eq, Operand::Value(plm), c32(0), IrType::U32);
                    let nz = self.emit_select_val(
                        Operand::Value(z),
                        c32(0x0020),
                        Operand::Value(plm),
                        IrType::U32,
                    );
                    self.emit_binop_val(IrBinOp::Or, Operand::Value(nz), c32(0x7f80), IrType::U32)
                }
            };
            return Some(Operand::Value(self.emit_cast_val(
                Operand::Value(out_u32),
                IrType::U32,
                IrType::U16,
            )));
        }

        // Validate real-arithmetic scalar arguments for all value builtins.
        for a in args {
            let ct = self.expr_ctype(a);
            if !ct.is_arithmetic() || ct.is_complex() {
                self.diagnostics.borrow_mut().error(
                    format!("incompatible non-floating argument for '{}'", name),
                    a.span(),
                );
                return Some(Operand::Const(IrConst::I16(0)));
            }
        }

        if !self.pass_policy.fp_environment_observable {
            if let Some(c) =
                shared_const_eval::eval_builtin_call(name, args, &|e| self.eval_const_expr(e))
            {
                return Some(Operand::Const(c));
            }
        }

        let mut half_args = Vec::with_capacity(args.len());
        for a in args {
            let ct = self.expr_ctype(a);
            let ty = self.value_ir_type(a);
            let v = self.lower_expr(a);
            half_args.push(self.convert_scalar_ctype(v, ty, &ct, &CType::Float16));
        }

        match name {
            "__builtin_fabsf16" => {
                let dst = self.emit_binop_val(
                    IrBinOp::And,
                    half_args[0].clone(),
                    Operand::Const(IrConst::I16(0x7fff)),
                    IrType::U16,
                );
                Some(Operand::Value(dst))
            }
            "__builtin_copysignf16" => {
                let mag = self.emit_binop_val(
                    IrBinOp::And,
                    half_args[0].clone(),
                    Operand::Const(IrConst::I16(0x7fff)),
                    IrType::U16,
                );
                let sgn = self.emit_binop_val(
                    IrBinOp::And,
                    half_args[1].clone(),
                    Operand::Const(IrConst::I16(0x8000u16 as i16)),
                    IrType::U16,
                );
                let dst = self.emit_binop_val(
                    IrBinOp::Or,
                    Operand::Value(mag),
                    Operand::Value(sgn),
                    IrType::U16,
                );
                Some(Operand::Value(dst))
            }
            "__builtin_sqrtf16" => {
                let f = self.emit_f16_to_f32(half_args[0].clone());
                let sq = self.fresh_value();
                self.emit(Instruction::Intrinsic {
                    dest: Some(sq),
                    op: IntrinsicOp::SqrtF32,
                    dest_ptr: None,
                    args: vec![Operand::Value(f)],
                });
                Some(Operand::Value(self.emit_f32_to_f16(Operand::Value(sq))))
            }
            "__builtin_fmaf16" => {
                // Exact fused multiply-add in IEEE binary64: every finite _Float16 product
                // has <=22 bits and every non-overflowing sum a*b + c spans at most 41 bits
                // (2^-24..2^16), which fits with zero rounding error in 53-bit binary64!
                let fa = self.emit_f16_to_f32(half_args[0].clone());
                let fb = self.emit_f16_to_f32(half_args[1].clone());
                let fc = self.emit_f16_to_f32(half_args[2].clone());
                let da = self.emit_cast_val(Operand::Value(fa), IrType::F32, IrType::F64);
                let db = self.emit_cast_val(Operand::Value(fb), IrType::F32, IrType::F64);
                let dc = self.emit_cast_val(Operand::Value(fc), IrType::F32, IrType::F64);
                let prod = self.emit_binop_val(
                    IrBinOp::Mul,
                    Operand::Value(da),
                    Operand::Value(db),
                    IrType::F64,
                );
                let sum = self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(prod),
                    Operand::Value(dc),
                    IrType::F64,
                );
                let rto = self.emit_f64_to_f32_rto(Operand::Value(sum));
                Some(Operand::Value(self.emit_f32_to_f16(Operand::Value(rto))))
            }
            "__builtin_fminf16" | "__builtin_fmaxf16" => {
                let is_max = name == "__builtin_fmaxf16";
                Some(Operand::Value(self.emit_half_minmax(
                    half_args[0].clone(),
                    half_args[1].clone(),
                    is_max,
                )))
            }
            "__builtin_nextafterf16" => Some(Operand::Value(
                self.emit_nextafterf16_inline(half_args[0].clone(), half_args[1].clone()),
            )),
            "__builtin_roundevenf16" => {
                let f = self.emit_f16_to_f32(half_args[0].clone());
                let r = self.emit_roundeven_f32(Operand::Value(f));
                Some(Operand::Value(self.emit_f32_to_f16(Operand::Value(r))))
            }
            _ => {
                let f32_fn = match name {
                    "__builtin_ceilf16" => "ceilf",
                    "__builtin_floorf16" => "floorf",
                    "__builtin_truncf16" => "truncf",
                    "__builtin_roundf16" => "roundf",
                    "__builtin_rintf16" => "rintf",
                    "__builtin_nearbyintf16" => "nearbyintf",
                    _ => "truncf",
                };
                let f = self.emit_f16_to_f32(half_args[0].clone());
                let f_res = self.fresh_value();
                self.emit(Instruction::Call {
                    func: f32_fn.to_string(),
                    info: CallInfo {
                        dest: Some(f_res),
                        args: vec![Operand::Value(f)],
                        arg_types: vec![IrType::F32],
                        return_type: IrType::F32,
                        is_variadic: false,
                        num_fixed_args: 1,
                        struct_arg_sizes: vec![None],
                        struct_arg_aligns: vec![],
                        struct_arg_classes: Vec::new(),
                        struct_arg_riscv_float_classes: Vec::new(),
                        struct_arg_is_f128_sse: Vec::new(),
                        is_sret: false,
                        calling_convention: crate::common::types::CallingConvention::Default,
                        is_fastcall: false,
                        regparm: None,
                        is_pure: f32_fn != "rintf",
                        is_const: !matches!(f32_fn, "rintf" | "nearbyintf"),
                        assume_library: true,
                        ret_eightbyte_classes: Vec::new(),
                        ret_is_f128_sse: false,
                        cold: false,
                    },
                });
                Some(Operand::Value(self.emit_f32_to_f16(Operand::Value(f_res))))
            }
        }
    }

    fn emit_roundeven_f32(&mut self, val_f32: Operand) -> Value {
        let u = self.emit_bitcast_f32_to_u32(val_f32.clone());
        let c32 = |v: u32| Operand::Const(IrConst::I32(v as i32));
        let abs = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(u),
            c32(0x7fff_ffff),
            IrType::U32,
        );
        let sign = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(u),
            c32(0x8000_0000),
            IrType::U32,
        );
        let magic_u = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(sign),
            c32(0x4b00_0000), // +/- 8388608.0f (2^23)
            IrType::U32,
        );
        let magic = self.emit_bitcast_u32_to_f32(Operand::Value(magic_u));
        let sum = self.emit_binop_val(
            IrBinOp::Add,
            val_f32.clone(),
            Operand::Value(magic),
            IrType::F32,
        );
        let diff = self.emit_binop_val(
            IrBinOp::Sub,
            Operand::Value(sum),
            Operand::Value(magic),
            IrType::F32,
        );
        let diff_u = self.emit_bitcast_f32_to_u32(Operand::Value(diff));
        let diff_mag = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(diff_u),
            c32(0x7fff_ffff),
            IrType::U32,
        );
        let signed_diff_u = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(diff_mag),
            Operand::Value(sign),
            IrType::U32,
        );
        let need_round = self.emit_cmp_val(
            IrCmpOp::Ult,
            Operand::Value(abs),
            c32(0x4b00_0000),
            IrType::U32,
        );
        let orig_v = self.operand_to_value(val_f32);
        let orig_u = self.emit_bitcast_f32_to_u32(Operand::Value(orig_v));
        let out_u = self.emit_select_val(
            Operand::Value(need_round),
            Operand::Value(signed_diff_u),
            Operand::Value(orig_u),
            IrType::U32,
        );
        self.emit_bitcast_u32_to_f32(Operand::Value(out_u))
    }

    fn emit_half_minmax(&mut self, a_h: Operand, b_h: Operand, is_max: bool) -> Value {
        let fa = self.emit_f16_to_f32(a_h.clone());
        let fb = self.emit_f16_to_f32(b_h.clone());
        let a_u = self.emit_cast_val(a_h, IrType::U16, IrType::U32);
        let b_u = self.emit_cast_val(b_h, IrType::U16, IrType::U32);
        let c32 = |v: u32| Operand::Const(IrConst::I32(v as i32));
        let a_abs =
            self.emit_binop_val(IrBinOp::And, Operand::Value(a_u), c32(0x7fff), IrType::U32);
        let b_abs =
            self.emit_binop_val(IrBinOp::And, Operand::Value(b_u), c32(0x7fff), IrType::U32);
        let a_nan = self.emit_cmp_val(
            IrCmpOp::Ugt,
            Operand::Value(a_abs),
            c32(0x7c00),
            IrType::U32,
        );
        let b_nan = self.emit_cmp_val(
            IrCmpOp::Ugt,
            Operand::Value(b_abs),
            c32(0x7c00),
            IrType::U32,
        );

        // Quiet any surviving NaN when both are NaN; return the non-NaN operand when one is NaN.
        let fa_h = self.emit_f32_to_f16(Operand::Value(fa));
        let fb_h = self.emit_f32_to_f16(Operand::Value(fb));
        let fa_hu = self.emit_cast_val(Operand::Value(fa_h), IrType::U16, IrType::U32);
        let fb_hu = self.emit_cast_val(Operand::Value(fb_h), IrType::U16, IrType::U32);

        let cmp_op = if is_max { IrCmpOp::Sgt } else { IrCmpOp::Slt };
        let ord = self.emit_cmp_val(cmp_op, Operand::Value(fa), Operand::Value(fb), IrType::F32);
        let eq = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(fa),
            Operand::Value(fb),
            IrType::F32,
        );
        // Signed zero tie-break: fmin(-0.0, +0.0) = -0.0 (OR), fmax(-0.0, +0.0) = +0.0 (AND).
        let zero_tie = self.emit_binop_val(
            if is_max { IrBinOp::And } else { IrBinOp::Or },
            Operand::Value(fa_hu),
            Operand::Value(fb_hu),
            IrType::U32,
        );
        let ord_sel = self.emit_select_val(
            Operand::Value(ord),
            Operand::Value(fa_hu),
            Operand::Value(fb_hu),
            IrType::U32,
        );
        let num_sel = self.emit_select_val(
            Operand::Value(eq),
            Operand::Value(zero_tie),
            Operand::Value(ord_sel),
            IrType::U32,
        );
        let b_nan_sel = self.emit_select_val(
            Operand::Value(b_nan),
            Operand::Value(fa_hu),
            Operand::Value(num_sel),
            IrType::U32,
        );
        let res = self.emit_select_val(
            Operand::Value(a_nan),
            Operand::Value(fb_hu),
            Operand::Value(b_nan_sel),
            IrType::U32,
        );
        self.emit_cast_val(Operand::Value(res), IrType::U32, IrType::U16)
    }

    fn emit_nextafterf16_inline(&mut self, x_h: Operand, y_h: Operand) -> Value {
        let c32 = |v: u32| Operand::Const(IrConst::I32(v as i32));
        let x = self.emit_cast_val(x_h, IrType::U16, IrType::U32);
        let y = self.emit_cast_val(y_h, IrType::U16, IrType::U32);
        let ax = self.emit_binop_val(IrBinOp::And, Operand::Value(x), c32(0x7fff), IrType::U32);
        let ay = self.emit_binop_val(IrBinOp::And, Operand::Value(y), c32(0x7fff), IrType::U32);
        let x_nan = self.emit_cmp_val(IrCmpOp::Ugt, Operand::Value(ax), c32(0x7c00), IrType::U32);
        let y_nan = self.emit_cmp_val(IrCmpOp::Ugt, Operand::Value(ay), c32(0x7c00), IrType::U32);
        let x_q = self.emit_binop_val(IrBinOp::Or, Operand::Value(x), c32(0x0200), IrType::U32);
        let y_q = self.emit_binop_val(IrBinOp::Or, Operand::Value(y), c32(0x0200), IrType::U32);

        // Value equality including +0.0 == -0.0 returns y directly.
        let both_z = {
            let or_a = self.emit_binop_val(
                IrBinOp::Or,
                Operand::Value(ax),
                Operand::Value(ay),
                IrType::U32,
            );
            self.emit_cmp_val(IrCmpOp::Eq, Operand::Value(or_a), c32(0), IrType::U32)
        };
        let same_bits = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(x),
            Operand::Value(y),
            IrType::U32,
        );
        let val_eq = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(both_z),
            Operand::Value(same_bits),
            IrType::I32,
        );

        // From +/-0 toward non-zero y: (y & 0x8000) | 1
        let y_sgn = self.emit_binop_val(IrBinOp::And, Operand::Value(y), c32(0x8000), IrType::U32);
        let from_zero =
            self.emit_binop_val(IrBinOp::Or, Operand::Value(y_sgn), c32(1), IrType::U32);
        let x_is_z = self.emit_cmp_val(IrCmpOp::Eq, Operand::Value(ax), c32(0), IrType::U32);

        // Compare x < y via float values:
        let x16 = self.emit_cast_val(Operand::Value(x), IrType::U32, IrType::U16);
        let y16 = self.emit_cast_val(Operand::Value(y), IrType::U32, IrType::U16);
        let fx = self.emit_f16_to_f32(Operand::Value(x16));
        let fy = self.emit_f16_to_f32(Operand::Value(y16));
        let x_lt_y = self.emit_cmp_val(
            IrCmpOp::Slt,
            Operand::Value(fx),
            Operand::Value(fy),
            IrType::F32,
        );
        let x_pos = self.emit_cmp_val(IrCmpOp::Ult, Operand::Value(x), c32(0x8000), IrType::U32);
        let mag_inc = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(x_pos),
            Operand::Value(x_lt_y),
            IrType::I32,
        );
        let x_p1 = self.emit_binop_val(IrBinOp::Add, Operand::Value(x), c32(1), IrType::U32);
        let x_m1 = self.emit_binop_val(IrBinOp::Sub, Operand::Value(x), c32(1), IrType::U32);
        let stepped = self.emit_select_val(
            Operand::Value(mag_inc),
            Operand::Value(x_p1),
            Operand::Value(x_m1),
            IrType::U32,
        );

        let nz_step = self.emit_select_val(
            Operand::Value(x_is_z),
            Operand::Value(from_zero),
            Operand::Value(stepped),
            IrType::U32,
        );
        let eq_or_step = self.emit_select_val(
            Operand::Value(val_eq),
            Operand::Value(y),
            Operand::Value(nz_step),
            IrType::U32,
        );
        let y_nan_or = self.emit_select_val(
            Operand::Value(y_nan),
            Operand::Value(y_q),
            Operand::Value(eq_or_step),
            IrType::U32,
        );
        let res = self.emit_select_val(
            Operand::Value(x_nan),
            Operand::Value(x_q),
            Operand::Value(y_nan_or),
            IrType::U32,
        );
        self.emit_cast_val(Operand::Value(res), IrType::U32, IrType::U16)
    }
}
