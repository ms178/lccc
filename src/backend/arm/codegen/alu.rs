//! ArmCodegen: ALU operations (integer arithmetic, bitwise, unary).

use super::emit::{
    arm_alu_mnemonic, callee_saved_name, callee_saved_name_32, is_arm_fp_phys, ArmCodegen,
};
use crate::common::types::IrType;
use crate::ir::reexports::{IrBinOp, Operand, Value};

impl ArmCodegen {
    pub(super) fn emit_shifted_logical_impl(
        &mut self,
        shift_op: IrBinOp,
        shift_lhs: &Operand,
        shift_amount: &Operand,
        logical_op: IrBinOp,
        other: &Operand,
        dest: &Value,
        ty: IrType,
    ) {
        let use_32bit = matches!(ty, IrType::I32 | IrType::U32);
        let amount = Self::const_as_imm12(shift_amount)
            .expect("shifted logical fusion requires a constant shift");
        let width = if use_32bit { 32 } else { 64 };
        debug_assert!(amount < width);

        let mut materialize = |this: &mut Self, op: &Operand, scratch: &str| -> String {
            // FP-homed integer values must stage through the scratch GPR:
            // callee_saved_name panics on FP register indices (the session-29
            // ICE class). operand_to_x0 handles the fmov correctly.
            if let Some(reg) = this.operand_reg(op).filter(|r| !is_arm_fp_phys(*r)) {
                if use_32bit {
                    callee_saved_name_32(reg).to_string()
                } else {
                    callee_saved_name(reg).to_string()
                }
            } else {
                this.operand_to_x0(op);
                let acc = if use_32bit { "w0" } else { "x0" };
                this.state
                    .emit_fmt(format_args!("    mov {}, {}", scratch, acc));
                scratch.to_string()
            }
        };
        let other_reg = materialize(self, other, if use_32bit { "w1" } else { "x1" });
        let shifted_reg = materialize(self, shift_lhs, if use_32bit { "w2" } else { "x2" });
        let output = if let Some(reg) = self.dest_reg(dest) {
            if use_32bit {
                callee_saved_name_32(reg)
            } else {
                callee_saved_name(reg)
            }
        } else if use_32bit {
            "w0"
        } else {
            "x0"
        };
        self.state.emit_fmt(format_args!(
            "    {} {}, {}, {}, {} #{}",
            arm_alu_mnemonic(logical_op),
            output,
            other_reg,
            shifted_reg,
            arm_alu_mnemonic(shift_op),
            amount
        ));
        self.state.reg_cache.invalidate_acc();
        if self.dest_reg(dest).is_none() {
            self.store_x0_to(dest);
        }
    }

    pub(super) fn emit_int_fused_mul_add_impl(
        &mut self,
        lhs: &Operand,
        rhs: &Operand,
        acc: &Operand,
        dest: &Value,
        ty: IrType,
    ) {
        let use_32bit = matches!(
            ty,
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 | IrType::I32 | IrType::U32
        );
        let mut materialize = |this: &mut Self, op: &Operand, scratch: &str| -> String {
            if let Some(reg) = this.operand_reg(op) {
                if use_32bit {
                    callee_saved_name_32(reg).to_string()
                } else {
                    callee_saved_name(reg).to_string()
                }
            } else {
                // Not register-assigned: load stack-homed values directly into
                // the scratch register, avoiding the ldr-into-x0 + mov detour.
                if let Operand::Value(v) = op {
                    if !this.state.is_alloca(v.0) {
                        if let Some(slot) = this.state.get_slot(v.0) {
                            this.emit_load_from_sp(scratch, slot.0, "ldr");
                            return scratch.to_string();
                        }
                    }
                }
                this.operand_to_x0(op);
                let acc_name = if use_32bit { "w0" } else { "x0" };
                this.state
                    .emit_fmt(format_args!("    mov {}, {}", scratch, acc_name));
                scratch.to_string()
            }
        };
        let lhs_reg = materialize(self, lhs, if use_32bit { "w1" } else { "x1" });
        let rhs_reg = materialize(self, rhs, if use_32bit { "w2" } else { "x2" });
        let acc_reg = materialize(self, acc, if use_32bit { "w3" } else { "x3" });
        if let Some(dest_phys) = self.dest_reg(dest).filter(|r| !is_arm_fp_phys(*r)) {
            let output = if use_32bit {
                callee_saved_name_32(dest_phys)
            } else {
                callee_saved_name(dest_phys)
            };
            self.state.emit_fmt(format_args!(
                "    madd {}, {}, {}, {}",
                output, lhs_reg, rhs_reg, acc_reg
            ));
            self.state.reg_cache.invalidate_acc();
            return;
        }
        let output = if use_32bit { "w0" } else { "x0" };
        self.state.emit_fmt(format_args!(
            "    madd {}, {}, {}, {}",
            output, lhs_reg, rhs_reg, acc_reg
        ));
        self.store_x0_to(dest);
    }

    /// Integer fused multiply-subtract: dest = acc - lhs*rhs (msub).
    /// Same staging discipline as emit_int_fused_mul_add_impl; msub reads
    /// all three sources before writing, so output aliasing is safe.
    pub(super) fn emit_int_fused_mul_sub_impl(
        &mut self,
        lhs: &Operand,
        rhs: &Operand,
        acc: &Operand,
        dest: &Value,
        ty: IrType,
    ) {
        let use_32bit = matches!(
            ty,
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 | IrType::I32 | IrType::U32
        );
        let mut materialize = |this: &mut Self, op: &Operand, scratch: &str| -> String {
            // FP-homed integer values stage through the scratch GPR
            // (callee_saved_name panics on FP indices; session-29 class).
            if let Some(reg) = this.operand_reg(op).filter(|r| !is_arm_fp_phys(*r)) {
                if use_32bit {
                    callee_saved_name_32(reg).to_string()
                } else {
                    callee_saved_name(reg).to_string()
                }
            } else {
                if let Operand::Value(v) = op {
                    if !this.state.is_alloca(v.0) {
                        if let Some(slot) = this.state.get_slot(v.0) {
                            this.emit_load_from_sp(scratch, slot.0, "ldr");
                            return scratch.to_string();
                        }
                    }
                }
                this.operand_to_x0(op);
                let acc_name = if use_32bit { "w0" } else { "x0" };
                this.state
                    .emit_fmt(format_args!("    mov {}, {}", scratch, acc_name));
                scratch.to_string()
            }
        };
        let lhs_reg = materialize(self, lhs, if use_32bit { "w1" } else { "x1" });
        let rhs_reg = materialize(self, rhs, if use_32bit { "w2" } else { "x2" });
        let acc_reg = materialize(self, acc, if use_32bit { "w3" } else { "x3" });
        if let Some(dest_phys) = self.dest_reg(dest).filter(|r| !is_arm_fp_phys(*r)) {
            let output = if use_32bit {
                callee_saved_name_32(dest_phys)
            } else {
                callee_saved_name(dest_phys)
            };
            self.state.emit_fmt(format_args!(
                "    msub {}, {}, {}, {}",
                output, lhs_reg, rhs_reg, acc_reg
            ));
            self.state.reg_cache.invalidate_acc();
            return;
        }
        let output = if use_32bit { "w0" } else { "x0" };
        self.state.emit_fmt(format_args!(
            "    msub {}, {}, {}, {}",
            output, lhs_reg, rhs_reg, acc_reg
        ));
        self.store_x0_to(dest);
    }

    pub(super) fn emit_float_neg_impl(&mut self, ty: IrType) {
        if ty == IrType::F32 {
            self.state.emit("    fmov s0, w0");
            self.state.emit("    fneg s0, s0");
            self.state.emit("    fmov w0, s0");
            self.state.emit("    mov w0, w0"); // zero-extend
        } else {
            self.state.emit("    fmov d0, x0");
            self.state.emit("    fneg d0, d0");
            self.state.emit("    fmov x0, d0");
        }
    }

    pub(super) fn emit_int_neg_impl(&mut self, ty: IrType) {
        // 32-bit forms operate on w0 and zero-extend into x0, matching the
        // I32 home convention used by clz/rev elsewhere in this backend. A
        // 64-bit `neg x0,x0` on a zero-extended U32 would leave 0xFFFFFFFF
        // in the upper half (x86-64 audit, claim 1 — same bug class).
        match ty {
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 | IrType::I32 | IrType::U32 => {
                self.state.emit("    neg w0, w0");
            }
            _ => self.state.emit("    neg x0, x0"),
        }
    }

    pub(super) fn emit_int_not_impl(&mut self, ty: IrType) {
        match ty {
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 | IrType::I32 | IrType::U32 => {
                self.state.emit("    mvn w0, w0");
            }
            _ => self.state.emit("    mvn x0, x0"),
        }
    }

    pub(super) fn emit_int_clz_impl(&mut self, ty: IrType) {
        if ty == IrType::I32 || ty == IrType::U32 {
            self.state.emit("    clz w0, w0");
        } else {
            self.state.emit("    clz x0, x0");
        }
    }

    pub(super) fn emit_int_ctz_impl(&mut self, ty: IrType) {
        if ty == IrType::I32 || ty == IrType::U32 {
            self.state.emit("    rbit w0, w0");
            self.state.emit("    clz w0, w0");
        } else {
            self.state.emit("    rbit x0, x0");
            self.state.emit("    clz x0, x0");
        }
    }

    pub(super) fn emit_int_bswap_impl(&mut self, ty: IrType) {
        if ty == IrType::I16 || ty == IrType::U16 {
            self.state.emit("    rev w0, w0");
            self.state.emit("    lsr w0, w0, #16");
        } else if ty == IrType::I32 || ty == IrType::U32 {
            self.state.emit("    rev w0, w0");
        } else {
            self.state.emit("    rev x0, x0");
        }
    }

    pub(super) fn emit_int_bitreverse_impl(&mut self, ty: IrType) {
        if ty == IrType::I32 || ty == IrType::U32 {
            self.state.emit("    rbit w0, w0");
        } else {
            self.state.emit("    rbit x0, x0");
        }
    }

    pub(super) fn emit_int_popcount_impl(&mut self, ty: IrType) {
        if ty == IrType::I32 || ty == IrType::U32 {
            self.state.emit("    fmov s0, w0");
        } else {
            self.state.emit("    fmov d0, x0");
        }
        self.state.emit("    cnt v0.8b, v0.8b");
        self.state.emit("    uaddlv h0, v0.8b");
        self.state.emit("    fmov w0, s0");
    }

    pub(super) fn emit_int_binop_impl(
        &mut self,
        dest: &Value,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;

        // Same-block div/rem pair fusion (compute_i686_divrem_pairs with the
        // AArch64 target). The TAIL of a pair emits nothing — its result was
        // stored by the HEAD's sdiv+msub dual-store further up in this block
        // (exactly GCC's shape: one divide, the remainder folded as
        // lhs - q*rhs). Dead tails skip the msub entirely — unlike x86's
        // free dual-output, the msub costs a cycle on AArch64.
        if matches!(
            op,
            IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem
        ) {
            if self.divrem_tail_dests.contains(&dest.0) {
                if std::env::var_os("CCC_DEBUG_DIVREM").is_some() {
                    eprintln!("[DIVREM-ARM] tail-skip dest={}", dest.0);
                }
                return;
            }
            if let Some(&partner_dest) = self.divrem_head_partners.get(&dest.0) {
                if std::env::var_os("CCC_DEBUG_DIVREM").is_some() {
                    eprintln!(
                        "[DIVREM-ARM] head-emit dest={} op={:?} partner={}",
                        dest.0, op, partner_dest
                    );
                }
                self.emit_divrem_pair_head_arm(dest, op, lhs, rhs, use_32bit, partner_dest);
                return;
            }
        }

        // Strength reduction: UDiv/URem by power-of-2 constant
        if let Some(shift) = Self::const_as_power_of_2(rhs) {
            if op == IrBinOp::UDiv {
                self.operand_to_x0(lhs);
                if use_32bit {
                    self.state
                        .emit_fmt(format_args!("    lsr w0, w0, #{}", shift));
                } else {
                    self.state
                        .emit_fmt(format_args!("    lsr x0, x0, #{}", shift));
                }
                self.store_x0_to(dest);
                return;
            }
            if op == IrBinOp::URem {
                self.operand_to_x0(lhs);
                let mask = (1u64 << shift) - 1;
                if use_32bit {
                    self.state
                        .emit_fmt(format_args!("    and w0, w0, #{}", mask));
                } else {
                    self.state
                        .emit_fmt(format_args!("    and x0, x0, #{}", mask));
                }
                self.store_x0_to(dest);
                return;
            }
        }

        // Register-direct path. An FP-homed destination (d8-d14 / d16-d31,
        // possible since call-spanning F64 values allocate to the FP pool)
        // cannot take the GP three-operand form — callee_saved_name would
        // panic ("invalid ARM register index", aarch64_fuzz seed 17: an
        // integer BinOp whose result feeds an F64 conversion got an FP
        // home). Fall through to the accumulator path, which stages via
        // x0 and lets store_x0_to fmov into the FP register.
        if let Some(dest_phys) = self.dest_reg(dest).filter(|r| !is_arm_fp_phys(*r)) {
            let dest_name = callee_saved_name(dest_phys);
            let dest_name_32 = callee_saved_name_32(dest_phys);

            let is_shift = matches!(op, IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr);
            let is_simple_alu = matches!(
                op,
                IrBinOp::Add
                    | IrBinOp::Sub
                    | IrBinOp::And
                    | IrBinOp::Or
                    | IrBinOp::Xor
                    | IrBinOp::Mul
            ) || is_shift;
            if is_simple_alu {
                let mnemonic = arm_alu_mnemonic(op);
                let lhs_phys = self.operand_reg(lhs).filter(|r| !is_arm_fp_phys(*r));
                // Emit `op dest, lhs, rhs` with lhs either its assigned register
                // (three-operand form, no setup mov) or dest preloaded with lhs.
                macro_rules! emit3 {
                    ($rhs:expr) => {
                        if let Some(lp) = lhs_phys {
                            if use_32bit {
                                let l = callee_saved_name_32(lp);
                                self.state.emit_fmt(format_args!(
                                    "    {} {}, {}, {}",
                                    mnemonic,
                                    dest_name_32,
                                    l,
                                    $rhs(lp, true)
                                ));
                            } else {
                                let l = callee_saved_name(lp);
                                self.state.emit_fmt(format_args!(
                                    "    {} {}, {}, {}",
                                    mnemonic,
                                    dest_name,
                                    l,
                                    $rhs(lp, false)
                                ));
                            }
                        } else {
                            self.operand_to_callee_reg(lhs, dest_phys);
                            if use_32bit {
                                self.state.emit_fmt(format_args!(
                                    "    {} {}, {}, {}",
                                    mnemonic,
                                    dest_name_32,
                                    dest_name_32,
                                    $rhs(dest_phys, true)
                                ));
                            } else {
                                self.state.emit_fmt(format_args!(
                                    "    {} {}, {}, {}",
                                    mnemonic,
                                    dest_name,
                                    dest_name,
                                    $rhs(dest_phys, false)
                                ));
                            }
                        }
                    };
                }

                if matches!(op, IrBinOp::Add | IrBinOp::Sub) {
                    if let Some(imm) = Self::const_as_imm12(rhs) {
                        emit3!(|_, _| format!("#{}", imm));
                        self.state.reg_cache.invalidate_acc();
                        return;
                    }
                }

                if matches!(op, IrBinOp::And | IrBinOp::Or | IrBinOp::Xor) {
                    let width = if use_32bit { 32 } else { 64 };
                    if let Some(imm) = Self::const_as_logical_imm(rhs, width) {
                        emit3!(|_, _| format!("#{:#x}", imm));
                        self.state.reg_cache.invalidate_acc();
                        return;
                    }
                }

                // AArch64 encodes a constant shift directly in the instruction.
                // Keeping the value in its assigned register avoids several moves
                // through x0/x1/x2 for the common C bit-manipulation idiom.
                if is_shift {
                    if let Some(imm) = Self::const_as_imm12(rhs) {
                        let width = if use_32bit { 32 } else { 64 };
                        if imm < width {
                            emit3!(|_, _| format!("#{}", imm));
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                    }
                }

                let rhs_phys = self.operand_reg(rhs).filter(|r| !is_arm_fp_phys(*r));
                // Regalloc can assign two values whose lifetime endpoints meet
                // to the same register. If they are then used by the SAME
                // instruction, the later definition has overwritten the older
                // value in that register (AArch64 torture 20120919-1: `s +=
                // pi[i]` became `add w5,w5,w5`).  Repair this rare boundary
                // case locally by reloading one operand from its stack home
                // into x0/w0 and using the three-operand form. This preserves
                // the aggressive allocator's half-open handoff policy while
                // keeping same-instruction uses correct.
                if let (Operand::Value(lv), Operand::Value(rv), Some(lp), Some(rp)) =
                    (lhs, rhs, lhs_phys, rhs_phys)
                {
                    if lv.0 != rv.0 && lp.0 == rp.0 {
                        let scratch = if use_32bit { "w0" } else { "x0" };
                        if let Some(slot) = self.state.get_slot(lv.0) {
                            self.emit_load_from_sp(scratch, slot.0, "ldr");
                            let r = if use_32bit {
                                callee_saved_name_32(rp)
                            } else {
                                callee_saved_name(rp)
                            };
                            let out = if use_32bit { dest_name_32 } else { dest_name };
                            self.state.emit_fmt(format_args!(
                                "    {} {}, {}, {}",
                                mnemonic, out, scratch, r
                            ));
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                        if let Some(slot) = self.state.get_slot(rv.0) {
                            self.emit_load_from_sp(scratch, slot.0, "ldr");
                            let l = if use_32bit {
                                callee_saved_name_32(lp)
                            } else {
                                callee_saved_name(lp)
                            };
                            let out = if use_32bit { dest_name_32 } else { dest_name };
                            self.state.emit_fmt(format_args!(
                                "    {} {}, {}, {}",
                                mnemonic, out, l, scratch
                            ));
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                    }
                }
                if let Some(rp) = rhs_phys {
                    let rhs_name = if use_32bit {
                        callee_saved_name_32(rp)
                    } else {
                        callee_saved_name(rp)
                    };
                    if rp.0 == dest_phys.0 {
                        // rhs lives in dest's home. The three-operand form
                        // `op dest, lhs, rhs` reads rhs AFTER dest is written,
                        // so dest==rhs turns the op into `lhs OP lhs` whenever
                        // lhs is not itself live in that register. This is the
                        // same machine constraint the allocator's half-open
                        // handoff does not model locally; the previous repair
                        // covered only a slotted VALUE lhs — a CONSTANT lhs
                        // (repro: `(int32_t)c - 2` with the result homed over
                        // the addend, arm_csinc_select main preheader:
                        // `mov x23,#-1; sub x23,x23,x23` computed 0) and a
                        // slotless lhs fell through to the broken form. Stage
                        // lhs through x0 — that path handles constants, stack
                        // homes and the accumulator uniformly — then read rhs
                        // from its (dest-named) home in the same instruction.
                        let scratch = if use_32bit { "w0" } else { "x0" };
                        self.operand_to_x0(lhs);
                        let out = if use_32bit { dest_name_32 } else { dest_name };
                        self.state.emit_fmt(format_args!(
                            "    {} {}, {}, {}",
                            mnemonic, out, scratch, rhs_name
                        ));
                        self.state.reg_cache.invalidate_acc();
                        return;
                    }
                    emit3!(|_, is32| if is32 {
                        callee_saved_name_32(rp).to_string()
                    } else {
                        callee_saved_name(rp).to_string()
                    });
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
                // rhs not register-assigned: load it into x0 as scratch.
                // If lhs is also not register-assigned, its load into dest goes
                // through x0 too — so lhs must be loaded FIRST, otherwise it
                // clobbers the rhs already sitting in x0.
                if lhs_phys.is_none() {
                    self.operand_to_callee_reg(lhs, dest_phys);
                    self.operand_to_x0(rhs);
                    if use_32bit {
                        self.state.emit_fmt(format_args!(
                            "    {} {}, {}, w0",
                            mnemonic, dest_name_32, dest_name_32
                        ));
                    } else {
                        self.state.emit_fmt(format_args!(
                            "    {} {}, {}, x0",
                            mnemonic, dest_name, dest_name
                        ));
                    }
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
                self.operand_to_x0(rhs);
                emit3!(|_, is32| if is32 {
                    "w0".to_string()
                } else {
                    "x0".to_string()
                });
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }

        // Immediate accumulator path for spilled/short-lived results.  The
        // previous fallback materialized every constant in x0, copied both
        // operands through x1/x2, then performed a register-register op.  GEP
        // scaling and loop increments commonly land here, so use AArch64's
        // immediate forms even when the result has no physical assignment.
        if matches!(op, IrBinOp::Add | IrBinOp::Sub) {
            if let Some(imm) = Self::const_as_imm12(rhs) {
                self.operand_to_x0(lhs);
                let mnemonic = arm_alu_mnemonic(op);
                if use_32bit {
                    self.state
                        .emit_fmt(format_args!("    {} w0, w0, #{}", mnemonic, imm));
                } else {
                    self.state
                        .emit_fmt(format_args!("    {} x0, x0, #{}", mnemonic, imm));
                }
                self.store_x0_to(dest);
                return;
            }
        }
        if matches!(op, IrBinOp::And | IrBinOp::Or | IrBinOp::Xor) {
            let width = if use_32bit { 32 } else { 64 };
            if let Some(imm) = Self::const_as_logical_imm(rhs, width) {
                self.operand_to_x0(lhs);
                let mnemonic = arm_alu_mnemonic(op);
                if use_32bit {
                    self.state
                        .emit_fmt(format_args!("    {} w0, w0, #{:#x}", mnemonic, imm));
                } else {
                    self.state
                        .emit_fmt(format_args!("    {} x0, x0, #{:#x}", mnemonic, imm));
                }
                self.store_x0_to(dest);
                return;
            }
        }
        if matches!(op, IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr) {
            if let Some(imm) = Self::const_as_imm12(rhs) {
                let width = if use_32bit { 32 } else { 64 };
                if imm < width {
                    self.operand_to_x0(lhs);
                    let mnemonic = arm_alu_mnemonic(op);
                    if use_32bit {
                        self.state
                            .emit_fmt(format_args!("    {} w0, w0, #{}", mnemonic, imm));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    {} x0, x0, #{}", mnemonic, imm));
                    }
                    self.store_x0_to(dest);
                    return;
                }
            }
        }

        // Fallback: accumulator path
        self.operand_to_x0(lhs);
        self.state.emit("    mov x1, x0");
        self.operand_to_x0(rhs);
        self.state.emit("    mov x2, x0");

        if use_32bit {
            match op {
                IrBinOp::Add => {
                    self.state.emit("    add w0, w1, w2");
                }
                IrBinOp::Sub => {
                    self.state.emit("    sub w0, w1, w2");
                }
                IrBinOp::Mul => {
                    self.state.emit("    mul w0, w1, w2");
                }
                IrBinOp::SDiv => {
                    self.state.emit("    sdiv w0, w1, w2");
                }
                IrBinOp::UDiv => self.state.emit("    udiv w0, w1, w2"),
                IrBinOp::SRem => {
                    self.state.emit("    sdiv w3, w1, w2");
                    self.state.emit("    msub w0, w3, w2, w1");
                }
                IrBinOp::URem => {
                    self.state.emit("    udiv w3, w1, w2");
                    self.state.emit("    msub w0, w3, w2, w1");
                }
                IrBinOp::And => self.state.emit("    and w0, w1, w2"),
                IrBinOp::Or => self.state.emit("    orr w0, w1, w2"),
                IrBinOp::Xor => self.state.emit("    eor w0, w1, w2"),
                IrBinOp::Shl => {
                    self.state.emit("    lsl w0, w1, w2");
                }
                IrBinOp::AShr => {
                    self.state.emit("    asr w0, w1, w2");
                }
                IrBinOp::LShr => self.state.emit("    lsr w0, w1, w2"),
                IrBinOp::BitTest => {
                    // AArch64 has no BT. UBFX extracts a 1-bit field when the
                    // index is an in-range immediate; variable and 64-bit cases
                    // use the canonical shift/AND form. The IR recognizer only
                    // creates I32 BitTest, so this path remains in w registers.
                    if let Some(imm) = Self::const_as_imm12(rhs) {
                        if (0..31).contains(&imm) {
                            self.state
                                .emit_fmt(format_args!("    ubfx w0, w1, #{imm}, #1"));
                        } else {
                            self.state.emit_fmt(format_args!("    lsr w0, w1, #{imm}"));
                            self.state.emit("    and w0, w0, #1");
                        }
                    } else {
                        self.state.emit("    lsr w0, w1, w2");
                        self.state.emit("    and w0, w0, #1");
                    }
                }
            }
        } else {
            match op {
                IrBinOp::Add => self.state.emit("    add x0, x1, x2"),
                IrBinOp::Sub => self.state.emit("    sub x0, x1, x2"),
                IrBinOp::Mul => self.state.emit("    mul x0, x1, x2"),
                IrBinOp::SDiv => self.state.emit("    sdiv x0, x1, x2"),
                IrBinOp::UDiv => self.state.emit("    udiv x0, x1, x2"),
                IrBinOp::SRem => {
                    self.state.emit("    sdiv x3, x1, x2");
                    self.state.emit("    msub x0, x3, x2, x1");
                }
                IrBinOp::URem => {
                    self.state.emit("    udiv x3, x1, x2");
                    self.state.emit("    msub x0, x3, x2, x1");
                }
                IrBinOp::And => self.state.emit("    and x0, x1, x2"),
                IrBinOp::Or => self.state.emit("    orr x0, x1, x2"),
                IrBinOp::Xor => self.state.emit("    eor x0, x1, x2"),
                IrBinOp::Shl => self.state.emit("    lsl x0, x1, x2"),
                IrBinOp::AShr => self.state.emit("    asr x0, x1, x2"),
                IrBinOp::LShr => self.state.emit("    lsr x0, x1, x2"),
                IrBinOp::BitTest => {
                    if let Some(imm) = Self::const_as_imm12(rhs) {
                        if (0..63).contains(&imm) {
                            self.state
                                .emit_fmt(format_args!("    ubfx x0, x1, #{imm}, #1"));
                        } else {
                            self.state.emit_fmt(format_args!("    lsr x0, x1, #{imm}"));
                            self.state.emit("    and x0, x0, #1");
                        }
                    } else {
                        self.state.emit("    lsr x0, x1, x2");
                        self.state.emit("    and x0, x0, #1");
                    }
                }
            }
        }

        self.store_x0_to(dest);
    }

    /// Store x3 (the fused pair's second result) to a value's location.
    /// Mirror of `store_x0_to` with the source register changed; no
    /// accumulator claim (the value sits in x3, not the x0 accumulator).
    pub(super) fn store_x3_to(&mut self, dest: &Value, use_32bit: bool) {
        let src = if use_32bit { "w3" } else { "x3" };
        if self
            .state
            .value_use_counts
            .get(dest.0 as usize)
            .copied()
            .unwrap_or(0)
            == 0
        {
            return;
        }
        if let Some(&reg) = self.reg_assignments.get(&dest.0) {
            if is_arm_fp_phys(reg) {
                self.state
                    .emit_fmt(format_args!("    fmov d{}, {}", reg.0 - 24, src));
            } else {
                let reg_name = callee_saved_name(reg);
                self.state
                    .emit_fmt(format_args!("    mov {}, {}", reg_name, src));
            }
        } else if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_sp(src, slot.0, "str");
        }
    }

    /// Emit one divide serving a same-block div/rem pair (GCC's sdiv+msub
    /// shape) and store BOTH results. The head's own result always lands in
    /// x0; the partner's result is materialised in x3 via msub (quotient
    /// heads) or taken directly from x3 (remainder heads).
    fn emit_divrem_pair_head_arm(
        &mut self,
        dest: &Value,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        use_32bit: bool,
        partner_dest: u32,
    ) {
        let signed = matches!(op, IrBinOp::SDiv | IrBinOp::SRem);
        let self_is_div = matches!(op, IrBinOp::UDiv | IrBinOp::SDiv);
        // x1/x2 (operands) and x3 (remainder scratch) are used as FIXED
        // registers here. That is sound without any RA exclusion because
        // the ARM allocator only ever assigns x4, x5, x6, x7, x8, x13, x14
        // (ARM_CALLER_SAVED) and x19-x28 (ARM_CALLEE_SAVED) — x1/x2/x3 are
        // in no pool (audited 2026-08-29; x86's counterpart needs the %rdx
        // exclusion because x86's allocatable pool includes rdx).
        self.operand_to_x0(lhs);
        self.state.emit("    mov x1, x0");
        self.operand_to_x0(rhs);
        self.state.emit("    mov x2, x0");
        let div_mnem = if signed { "sdiv" } else { "udiv" };
        let w = if use_32bit { "w" } else { "x" };
        let tail = Value(partner_dest);
        let tail_dead = self
            .state
            .value_use_counts
            .get(partner_dest as usize)
            .copied()
            .unwrap_or(0)
            == 0;
        if self_is_div {
            // Quotient head: x0 = q directly; remainder = lhs - q*rhs in x3.
            self.state
                .emit_fmt(format_args!("    {} {}0, {}1, {}2", div_mnem, w, w, w));
            if !tail_dead {
                self.state
                    .emit_fmt(format_args!("    msub {}3, {}0, {}2, {}1", w, w, w, w));
                self.store_x3_to(&tail, use_32bit);
            }
            self.store_x0_to(dest);
        } else {
            // Remainder head: x3 = q feeds the msub that produces x0 = r.
            self.state
                .emit_fmt(format_args!("    {} {}3, {}1, {}2", div_mnem, w, w, w));
            self.state
                .emit_fmt(format_args!("    msub {}0, {}3, {}2, {}1", w, w, w, w));
            if !tail_dead {
                self.store_x3_to(&tail, use_32bit);
            }
            self.store_x0_to(dest);
        }
    }

    pub(super) fn emit_copy_i128_impl(&mut self, dest: &Value, src: &Operand) {
        self.operand_to_x0_x1(src);
        self.store_x0_x1_to(dest);
    }
}
