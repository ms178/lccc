//! ArmCodegen: cast operations.

use super::emit::{
    ArmCodegen, arm_fp_name, callee_saved_name, callee_saved_name_32, is_arm_fp_phys,
};
use crate::backend::cast::{CastKind, classify_cast};
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{Operand, Value};

impl ArmCodegen {
    pub(super) fn emit_cast_instrs_impl(&mut self, from_ty: IrType, to_ty: IrType) {
        match classify_cast(from_ty, to_ty) {
            CastKind::Noop | CastKind::UnsignedToSignedSameSize { .. } => {}

            kind @ (CastKind::FloatToSigned { from_f64 }
            | CastKind::FloatToUnsigned { from_f64, .. }) => {
                let signed = matches!(kind, CastKind::FloatToSigned { .. });
                let source = if from_f64 {
                    self.state.emit("    fmov d0, x0");
                    "d0"
                } else {
                    self.state.emit("    fmov s0, w0");
                    "s0"
                };
                self.emit_fp_to_int_conversion(("x0", "w0"), source, to_ty, signed);
            }

            CastKind::SignedToFloat { to_f64, from_ty } => {
                match from_ty.size() {
                    1 => self.state.emit("    sxtb x0, w0"),
                    2 => self.state.emit("    sxth x0, w0"),
                    4 => self.state.emit("    sxtw x0, w0"),
                    _ => {}
                }
                if to_f64 {
                    self.state.emit("    scvtf d0, x0");
                    self.state.emit("    fmov x0, d0");
                } else {
                    self.state.emit("    scvtf s0, x0");
                    self.state.emit("    fmov w0, s0");
                }
            }

            CastKind::UnsignedToFloat { to_f64, .. } => {
                if to_f64 {
                    self.state.emit("    ucvtf d0, x0");
                    self.state.emit("    fmov x0, d0");
                } else {
                    self.state.emit("    ucvtf s0, x0");
                    self.state.emit("    fmov w0, s0");
                }
            }

            CastKind::FloatToFloat { widen } => {
                if widen {
                    self.state.emit("    fmov s0, w0");
                    self.state.emit("    fcvt d0, s0");
                    self.state.emit("    fmov x0, d0");
                } else {
                    self.state.emit("    fmov d0, x0");
                    self.state.emit("    fcvt s0, d0");
                    self.state.emit("    fmov w0, s0");
                }
            }

            CastKind::SignedToUnsignedSameSize { to_ty } => match to_ty {
                IrType::U8 => self.state.emit("    and x0, x0, #0xff"),
                IrType::U16 => self.state.emit("    and x0, x0, #0xffff"),
                IrType::U32 => self.state.emit("    mov w0, w0"),
                _ => {}
            },

            CastKind::IntWiden { from_ty, .. } => {
                if from_ty.is_unsigned() {
                    match from_ty {
                        IrType::U8 => self.state.emit("    and x0, x0, #0xff"),
                        IrType::U16 => self.state.emit("    and x0, x0, #0xffff"),
                        IrType::U32 => self.state.emit("    mov w0, w0"),
                        _ => {}
                    }
                } else {
                    match from_ty {
                        IrType::I8 => self.state.emit("    sxtb x0, w0"),
                        IrType::I16 => self.state.emit("    sxth x0, w0"),
                        IrType::I32 => self.state.emit("    sxtw x0, w0"),
                        _ => {}
                    }
                }
            }

            CastKind::IntNarrow { to_ty } => match to_ty {
                IrType::I8 => self.state.emit("    sxtb x0, w0"),
                IrType::U8 => self.state.emit("    and x0, x0, #0xff"),
                IrType::I16 => self.state.emit("    sxth x0, w0"),
                IrType::U16 => self.state.emit("    and x0, x0, #0xffff"),
                IrType::I32 => self.state.emit("    sxtw x0, w0"),
                IrType::U32 => self.state.emit("    mov w0, w0"),
                _ => {}
            },

            CastKind::SignedToF128 { .. }
            | CastKind::UnsignedToF128 { .. }
            | CastKind::F128ToSigned { .. }
            | CastKind::F128ToUnsigned { .. }
            | CastKind::FloatToF128 { .. }
            | CastKind::F128ToFloat { .. } => {
                unreachable!("F128 cast variants not produced by classify_cast()");
            }
        }
    }

    /// Truncate the FP register `source` (an `sN`/`dN` name) toward zero into
    /// the integer type `to_ty`, writing the GPR named by `(x, w)` in the
    /// backend's register convention for that type (signed types
    /// sign-extended, unsigned types zero-extended to 64 bits).
    ///
    /// Targets of at most 32 bits convert with the W form. `FCVTZS Wd` /
    /// `FCVTZU Wd` saturate to the 32-bit range (Arm ARM C7.2, FCVTZS
    /// (scalar, integer)) and the W write zeroes bits 63:32, so:
    /// * `unsigned` needs nothing more (`fcvtzu w0, d0` replaces
    ///   `fcvtzu x0, d0; mov w0, w0`);
    /// * `int` keeps one `sxtw`, now of a value that saturates exactly like
    ///   GCC's and Clang's `fcvtzs w0, d0` (`(int)3e9 == INT_MAX`; the X form
    ///   produced `3000000000` and `sxtw` wrapped it to a negative number);
    /// * 8/16-bit targets extend from the saturated 32-bit value, matching
    ///   the C semantics the mainstream compilers implement for in-range
    ///   values and their behavior on out-of-range ones.
    ///
    /// 64-bit targets (and pointers) use the X form unchanged.
    pub(super) fn emit_fp_to_int_conversion(
        &mut self,
        (x, w): (&str, &str),
        source: &str,
        to_ty: IrType,
        signed: bool,
    ) {
        let mnemonic = if signed { "fcvtzs" } else { "fcvtzu" };
        if to_ty.size() > 4 {
            self.state
                .emit_fmt(format_args!("    {} {}, {}", mnemonic, x, source));
            return;
        }
        self.state
            .emit_fmt(format_args!("    {} {}, {}", mnemonic, w, source));
        match to_ty {
            IrType::I8 => self.state.emit_fmt(format_args!("    sxtb {}, {}", x, w)),
            IrType::I16 => self.state.emit_fmt(format_args!("    sxth {}, {}", x, w)),
            IrType::I32 => self.state.emit_fmt(format_args!("    sxtw {}, {}", x, w)),
            IrType::U8 => self
                .state
                .emit_fmt(format_args!("    and {}, {}, #0xff", w, w)),
            IrType::U16 => self
                .state
                .emit_fmt(format_args!("    and {}, {}, #0xffff", w, w)),
            _ => {}
        }
    }

    /// Conversions with an operand in an FP register, register to register.
    /// The accumulator convention moves the value through x0 and d0 on both
    /// sides -- `(double)f` with both in FP homes was `fmov x0, d16; fmov s0,
    /// w0; fcvt d0, s0; fmov x0, d0; fmov d18, x0` for one `fcvt d18, s16`,
    /// and `(int)d` was `fmov x0, d16; fmov d0, x0; fcvtzs x0, d0` for one
    /// `fcvtzs x0, d16`. An integer result with a GPR home is converted
    /// straight into that register (`fcvtzs x20, d16`, not `fcvtzs x0, d16;
    /// mov x20, x0`); `emit_fp_to_int_conversion` owns the width rules.
    fn emit_fp_register_cast(
        &mut self,
        dest: &Value,
        src: &Operand,
        from_ty: IrType,
        to_ty: IrType,
    ) -> bool {
        // binary128 and 128-bit integers convert through libcalls.
        let scalar = |ty: IrType| {
            matches!(
                ty,
                IrType::F32
                    | IrType::F64
                    | IrType::I8
                    | IrType::I16
                    | IrType::I32
                    | IrType::I64
                    | IrType::U8
                    | IrType::U16
                    | IrType::U32
                    | IrType::U64
                    | IrType::Ptr
            )
        };
        if !scalar(from_ty) || !scalar(to_ty) {
            return false;
        }
        let src_fp = self.operand_reg(src).filter(|r| is_arm_fp_phys(*r));
        let dest_fp = self
            .get_phys_reg_for_value(dest.0)
            .filter(|r| is_arm_fp_phys(*r));
        match classify_cast(from_ty, to_ty) {
            CastKind::FloatToFloat { widen } if src_fp.is_some() || dest_fp.is_some() => {
                let scratch = if widen { "s0" } else { "d0" };
                let source = self.float_operand_reg(src, from_ty, scratch);
                let target = match dest_fp {
                    Some(phys) => arm_fp_name(phys, to_ty),
                    None if widen => "d0".to_string(),
                    None => "s0".to_string(),
                };
                self.state
                    .emit_fmt(format_args!("    fcvt {}, {}", target, source));
                self.store_float_reg(dest, to_ty, &target);
                true
            }
            kind @ (CastKind::FloatToSigned { .. } | CastKind::FloatToUnsigned { .. }) => {
                let Some(phys) = src_fp else {
                    return false;
                };
                let signed = matches!(kind, CastKind::FloatToSigned { .. });
                let source = arm_fp_name(phys, from_ty);
                // `dest_fp` is None here unless the allocator broke its own
                // class rule; a GPR home takes the result directly and leaves
                // the accumulator (and its cache entry) untouched.
                let dest_gp = self
                    .get_phys_reg_for_value(dest.0)
                    .filter(|r| !is_arm_fp_phys(*r));
                match dest_gp {
                    Some(reg) => {
                        let names = (callee_saved_name(reg), callee_saved_name_32(reg));
                        self.emit_fp_to_int_conversion(names, &source, to_ty, signed);
                    }
                    None => {
                        self.emit_fp_to_int_conversion(("x0", "w0"), &source, to_ty, signed);
                        self.store_x0_to(dest);
                    }
                }
                true
            }
            _ => false,
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
        // Integer widening (e.g. array-index I32 -> I64) where both source and
        // dest are register-assigned: extend directly between registers, no
        // x0 round-trip. This is on the hot path of every indexed access.
        if let CastKind::IntWiden { .. } = classify_cast(from_ty, to_ty) {
            let src_phys = self.operand_reg(src).filter(|r| !is_arm_fp_phys(*r));
            let dest_phys = self
                .get_phys_reg_for_value(dest.0)
                .filter(|r| !is_arm_fp_phys(*r));
            if let (Some(sp), Some(dp)) = (src_phys, dest_phys) {
                let s32 = callee_saved_name_32(sp);
                let d64 = callee_saved_name(dp);
                let d32 = callee_saved_name_32(dp);
                let signed = !from_ty.is_unsigned();
                match (signed, from_ty) {
                    (true, IrType::I8) => self
                        .state
                        .emit_fmt(format_args!("    sxtb {}, {}", d64, s32)),
                    (true, IrType::I16) => self
                        .state
                        .emit_fmt(format_args!("    sxth {}, {}", d64, s32)),
                    (true, IrType::I32) => {
                        // Clz/Ctz/Popcount results are provably in [0, 32]
                        // and were written by W-register instructions (upper
                        // half already zero, and sign-extension is the
                        // identity for non-negative values): the `sxtw` is
                        // dead. A same-register W move is also dead.
                        let bitop_src = matches!(src, Operand::Value(v)
                            if self.bitop_nonneg_values.contains(&v.0));
                        if !bitop_src {
                            self.state
                                .emit_fmt(format_args!("    sxtw {}, {}", d64, s32));
                        } else if sp != dp {
                            self.state
                                .emit_fmt(format_args!("    mov {}, {}", d32, s32));
                        }
                    }
                    // An atomic's unsigned result is already zero-extended
                    // (`atomic_zext_values`): a plain X copy at most, which
                    // copy propagation can fold (a W copy would read as a
                    // zero extension it cannot drop).
                    (false, IrType::U8 | IrType::U16 | IrType::U32)
                        if matches!(src, Operand::Value(v)
                            if self.atomic_zext_values.contains(&v.0)) =>
                    {
                        if sp != dp {
                            self.state.emit_fmt(format_args!(
                                "    mov {}, {}",
                                d64,
                                callee_saved_name(sp)
                            ));
                        }
                    }
                    (false, IrType::U8) => self
                        .state
                        .emit_fmt(format_args!("    and {}, {}, #0xff", d32, s32)),
                    (false, IrType::U16) => self
                        .state
                        .emit_fmt(format_args!("    and {}, {}, #0xffff", d32, s32)),
                    (false, IrType::U32) => self
                        .state
                        .emit_fmt(format_args!("    mov {}, {}", d32, s32)),
                    _ => {}
                }
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }
        // Integer-to-float casts whose result has an FP allocation can write
        // that register directly.  The generic accumulator convention would
        // otherwise round-trip through x0 (`scvtf d0; fmov x0,d0; fmov dN,x0`).
        if let Some(&phys) = self.reg_assignments.get(&dest.0) {
            if is_arm_fp_phys(phys) && matches!(to_ty, IrType::F32 | IrType::F64) {
                let kind = classify_cast(from_ty, to_ty);
                if matches!(
                    kind,
                    CastKind::SignedToFloat { .. } | CastKind::UnsignedToFloat { .. }
                ) {
                    let signed = matches!(kind, CastKind::SignedToFloat { .. });
                    let mnemonic = if signed { "scvtf" } else { "ucvtf" };
                    let fp_dest = arm_fp_name(phys, to_ty);
                    // Source already in a register: convert directly from it,
                    // no x0 round-trip. Only whole-register source widths here;
                    // sub-register signed extension still needs a scratch reg.
                    let src_phys = self.operand_reg(src).filter(|r| !is_arm_fp_phys(*r));
                    if let Some(sp) = src_phys {
                        match from_ty.size() {
                            8 => {
                                self.state.emit_fmt(format_args!(
                                    "    {} {}, {}",
                                    mnemonic,
                                    fp_dest,
                                    callee_saved_name(sp)
                                ));
                            }
                            4 => {
                                self.state.emit_fmt(format_args!(
                                    "    {} {}, {}",
                                    mnemonic,
                                    fp_dest,
                                    callee_saved_name_32(sp)
                                ));
                            }
                            _ => {
                                self.emit_load_operand(src);
                                if signed {
                                    if from_ty.size() == 1 {
                                        self.state.emit("    sxtb x0, w0");
                                    } else {
                                        self.state.emit("    sxth x0, w0");
                                    }
                                } else {
                                    if from_ty.size() == 1 {
                                        self.state.emit("    and x0, x0, #0xff");
                                    } else {
                                        self.state.emit("    and x0, x0, #0xffff");
                                    }
                                }
                                self.state
                                    .emit_fmt(format_args!("    {} {}, x0", mnemonic, fp_dest));
                            }
                        }
                        self.state.reg_cache.invalidate_acc();
                        return;
                    }
                    self.emit_load_operand(src);
                    if signed {
                        match from_ty.size() {
                            1 => self.state.emit("    sxtb x0, w0"),
                            2 => self.state.emit("    sxth x0, w0"),
                            4 => self.state.emit("    sxtw x0, w0"),
                            _ => {}
                        }
                    } else {
                        match from_ty.size() {
                            1 => self.state.emit("    and x0, x0, #0xff"),
                            2 => self.state.emit("    and x0, x0, #0xffff"),
                            4 => self.state.emit("    mov w0, w0"),
                            _ => {}
                        }
                    }
                    self.state
                        .emit_fmt(format_args!("    {} {}, x0", mnemonic, fp_dest));
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
            }
        }
        if self.emit_fp_register_cast(dest, src, from_ty, to_ty) {
            return;
        }
        // Relay fallback for bitop sources: an I32→I64 signed widen of a
        // Clz/Ctz/Popcount result needs no `sxtw` — the W-register load of
        // the 4-byte slot (or the bitop's own W write) already leaves the
        // correct zero-extended, sign-extension-identical value in x0.
        if from_ty == IrType::I32
            && to_ty == IrType::I64
            && matches!(src, Operand::Value(v) if self.bitop_nonneg_values.contains(&v.0))
        {
            self.emit_load_operand(src);
            self.store_x0_to_and_release(dest);
            return;
        }
        // Likewise the zero extension of an atomic's unsigned result.
        if matches!(classify_cast(from_ty, to_ty), CastKind::IntWiden { .. })
            && matches!(from_ty, IrType::U8 | IrType::U16 | IrType::U32)
            && matches!(src, Operand::Value(v) if self.atomic_zext_values.contains(&v.0))
        {
            self.emit_load_operand(src);
            self.store_x0_to_and_release(dest);
            return;
        }
        crate::backend::traits::emit_cast_default(self, dest, src, from_ty, to_ty);
    }
}
