//! RiscvCodegen: type conversion/casting operations.

use super::emit::RiscvCodegen;
use crate::backend::cast::{CastKind, classify_cast};
use crate::common::types::IrType;
use crate::ir::reexports::{Operand, Value};

impl RiscvCodegen {
    /// Truncate `ft0` toward zero into the integer type `to_ty` in `t0`,
    /// leaving the backend's register image for that type (signed types
    /// sign-extended, unsigned types zero-extended to 64 bits).
    ///
    /// Targets of at most 32 bits use the W forms. `fcvt.w.{s,d}` and
    /// `fcvt.wu.{s,d}` saturate to the 32-bit range and sign-extend the
    /// 32-bit result to XLEN (RISC-V Unprivileged ISA, "Floating-Point
    /// Conversion and Move Instructions"), so:
    /// * `int` is the conversion alone -- `fcvt.l.d; sext.w` was two
    ///   instructions, and its out-of-range results wrapped where GCC's and
    ///   Clang's `fcvt.w.d` saturate (`(int)3e9 == INT_MAX`);
    /// * 8/16-bit targets extend from the saturated 32-bit value;
    /// * `unsigned` keeps the zero extension: `fcvt.wu` sign-extends bit 31.
    ///
    /// 64-bit targets (and pointers) use the L forms unchanged.
    fn emit_fp_to_int_from_ft0(&mut self, from_f64: bool, to_ty: IrType, signed: bool) {
        let fmt = if from_f64 { "d" } else { "s" };
        let wide = to_ty.size() > 4;
        let op = match (wide, signed) {
            (true, true) => "l",
            (true, false) => "lu",
            (false, true) => "w",
            (false, false) => "wu",
        };
        self.state
            .emit_fmt(format_args!("    fcvt.{op}.{fmt} t0, ft0, rtz"));
        match to_ty {
            IrType::I8 => {
                self.state.emit("    slli t0, t0, 56");
                self.state.emit("    srai t0, t0, 56");
            }
            IrType::I16 => {
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srai t0, t0, 48");
            }
            IrType::U8 => self.state.emit("    andi t0, t0, 0xff"),
            IrType::U16 => {
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srli t0, t0, 48");
            }
            IrType::U32 => {
                self.state.emit("    slli t0, t0, 32");
                self.state.emit("    srli t0, t0, 32");
            }
            _ => {}
        }
    }

    pub(super) fn emit_cast_instrs_impl(&mut self, from_ty: IrType, to_ty: IrType) {
        match classify_cast(from_ty, to_ty) {
            CastKind::Noop => {}

            CastKind::UnsignedToSignedSameSize { to_ty } => {
                // On RISC-V, 32-bit values must be sign-extended in 64-bit registers
                // per the ABI. When converting U32 to I32, we must sign-extend
                // to ensure correct behavior when the value is used in comparisons
                // or passed to functions compiled by other compilers (e.g. glibc).
                if to_ty == IrType::I32 {
                    self.state.emit("    sext.w t0, t0");
                }
                // U8->I8, U16->I16, U64->I64 are true noops
            }

            kind @ (CastKind::FloatToSigned { from_f64 }
            | CastKind::FloatToUnsigned { from_f64, .. }) => {
                if from_f64 {
                    self.state.emit("    fmv.d.x ft0, t0");
                } else {
                    self.state.emit("    fmv.w.x ft0, t0");
                }
                let signed = matches!(kind, CastKind::FloatToSigned { .. });
                self.emit_fp_to_int_from_ft0(from_f64, to_ty, signed);
            }

            CastKind::SignedToFloat { to_f64, from_ty } => {
                match from_ty.size() {
                    1 => {
                        self.state.emit("    slli t0, t0, 56");
                        self.state.emit("    srai t0, t0, 56");
                    }
                    2 => {
                        self.state.emit("    slli t0, t0, 48");
                        self.state.emit("    srai t0, t0, 48");
                    }
                    4 => self.state.emit("    sext.w t0, t0"),
                    _ => {}
                }
                if to_f64 {
                    self.state.emit("    fcvt.d.l ft0, t0");
                    self.state.emit("    fmv.x.d t0, ft0");
                } else {
                    self.state.emit("    fcvt.s.l ft0, t0");
                    self.state.emit("    fmv.x.w t0, ft0");
                }
            }

            CastKind::UnsignedToFloat { to_f64, from_ty } => {
                let from_u64 = from_ty == IrType::U64;
                if from_u64 {
                    if to_f64 {
                        self.state.emit("    fcvt.d.lu ft0, t0");
                        self.state.emit("    fmv.x.d t0, ft0");
                    } else {
                        self.state.emit("    fcvt.s.lu ft0, t0");
                        self.state.emit("    fmv.x.w t0, ft0");
                    }
                } else if to_f64 {
                    self.state.emit("    fcvt.d.wu ft0, t0");
                    self.state.emit("    fmv.x.d t0, ft0");
                } else {
                    self.state.emit("    fcvt.s.wu ft0, t0");
                    self.state.emit("    fmv.x.w t0, ft0");
                }
            }

            CastKind::FloatToFloat { widen } => {
                if widen {
                    self.state.emit("    fmv.w.x ft0, t0");
                    self.state.emit("    fcvt.d.s ft0, ft0");
                    self.state.emit("    fmv.x.d t0, ft0");
                } else {
                    self.state.emit("    fmv.d.x ft0, t0");
                    self.state.emit("    fcvt.s.d ft0, ft0");
                    self.state.emit("    fmv.x.w t0, ft0");
                }
            }

            CastKind::SignedToUnsignedSameSize { to_ty } => match to_ty {
                IrType::U8 => self.state.emit("    andi t0, t0, 0xff"),
                IrType::U16 => {
                    self.state.emit("    slli t0, t0, 48");
                    self.state.emit("    srli t0, t0, 48");
                }
                IrType::U32 => {
                    self.state.emit("    slli t0, t0, 32");
                    self.state.emit("    srli t0, t0, 32");
                }
                _ => {}
            },

            CastKind::IntWiden { from_ty, .. } => {
                if from_ty.is_unsigned() {
                    match from_ty {
                        IrType::U8 => self.state.emit("    andi t0, t0, 0xff"),
                        IrType::U16 => {
                            self.state.emit("    slli t0, t0, 48");
                            self.state.emit("    srli t0, t0, 48");
                        }
                        IrType::U32 => {
                            self.state.emit("    slli t0, t0, 32");
                            self.state.emit("    srli t0, t0, 32");
                        }
                        _ => {}
                    }
                } else {
                    match from_ty {
                        IrType::I8 => {
                            self.state.emit("    slli t0, t0, 56");
                            self.state.emit("    srai t0, t0, 56");
                        }
                        IrType::I16 => {
                            self.state.emit("    slli t0, t0, 48");
                            self.state.emit("    srai t0, t0, 48");
                        }
                        IrType::I32 => self.state.emit("    sext.w t0, t0"),
                        _ => {}
                    }
                }
            }

            CastKind::IntNarrow { to_ty } => match to_ty {
                IrType::I8 => {
                    self.state.emit("    slli t0, t0, 56");
                    self.state.emit("    srai t0, t0, 56");
                }
                IrType::U8 => self.state.emit("    andi t0, t0, 0xff"),
                IrType::I16 => {
                    self.state.emit("    slli t0, t0, 48");
                    self.state.emit("    srai t0, t0, 48");
                }
                IrType::U16 => {
                    self.state.emit("    slli t0, t0, 48");
                    self.state.emit("    srli t0, t0, 48");
                }
                IrType::I32 => self.state.emit("    sext.w t0, t0"),
                IrType::U32 => {
                    self.state.emit("    slli t0, t0, 32");
                    self.state.emit("    srli t0, t0, 32");
                }
                _ => {}
            },

            CastKind::SignedToF128 { .. }
            | CastKind::UnsignedToF128 { .. }
            | CastKind::F128ToSigned { .. }
            | CastKind::F128ToUnsigned { .. }
            | CastKind::FloatToF128 { .. }
            | CastKind::F128ToFloat { .. } => {
                unreachable!("F128 casts should be handled by emit_cast override");
            }
        }
    }

    pub(super) fn emit_cast_impl(
        &mut self,
        dest: &Value,
        src: &Operand,
        from_ty: IrType,
        to_ty: IrType,
    ) {
        if crate::backend::f128_softfloat::f128_emit_cast(self, dest, src, from_ty, to_ty) {
            return;
        }
        // Clz/Ctz/Popcount results are provably in [0, 32] and their emitters
        // build the count from 0 upward in a full 64-bit register — the
        // I32→I64 signed widen's `sext.w` is the identity. The generic `lw`
        // reload of the 4-byte slot sign-extends a non-negative value, which
        // is equally the identity.
        if from_ty == IrType::I32
            && to_ty == IrType::I64
            && matches!(src, Operand::Value(v) if self.bitop_nonneg_values.contains(&v.0))
        {
            self.operand_to_t0(src);
            self.store_t0_to(dest);
            self.state.reg_cache.invalidate_acc();
            return;
        }
        crate::backend::traits::emit_cast_default(self, dest, src, from_ty, to_ty);
    }
}
