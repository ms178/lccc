//! I686Codegen: 128-bit (i128) and 64-bit pair operations.
//!
//! On i686, "i128" operations actually operate on 64-bit values using eax:edx pairs.
//! This module also contains the i64 bit-manipulation helpers.

use super::emit::I686Codegen;
use crate::backend::state::StackSlot;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{IrBinOp, IrCmpOp, IrConst, Operand, Value};

impl I686Codegen {
    /// Direct 64-bit RHS reference for the i64 fast paths: a constant pair
    /// (immediates) or a wide value's slot pair (memory operands). This is
    /// what lets `res & mask`, `res * base` and 64-bit compares avoid the
    /// i128 stack-staging dance (`pushl` both halves, operate through
    /// `(%esp)`, `addl $8`) — the boot corpus' number parsers paid ~25
    /// extra bytes per 64-bit ALU site for it.
    fn i64_rhs_ref(&self, rhs: &Operand) -> Option<(String, String)> {
        match rhs {
            Operand::Const(c) => {
                let v: u128 = match c {
                    IrConst::I64(v) => *v as u128,
                    IrConst::I128(v) => (*v as u128) & 0xFFFF_FFFF_FFFF_FFFF,
                    IrConst::F64(f) => f.to_bits() as u128,
                    IrConst::Zero => 0,
                    other => other.to_i64()? as u128,
                };
                let lo = (v & 0xFFFF_FFFF) as i32;
                let hi = ((v >> 32) & 0xFFFF_FFFF) as i32;
                Some((format!("${}", lo), format!("${}", hi)))
            }
            Operand::Value(v) => {
                if !self.state.wide_values.contains(&v.0) {
                    return None;
                }
                let slot = self.state.get_slot(v.0)?;
                Some((self.slot_ref(slot), self.slot_ref_offset(slot, 4)))
            }
        }
    }

    /// i64 ALU fast path (i686): load the LHS pair into %eax:%edx, then
    /// apply the RHS through immediates or direct memory operands — GCC's
    /// shape — instead of staging both operands through the stack. Handles
    /// Add/Sub/And/Or/Xor/Mul. Returns false (caller falls back to the i128
    /// path) for shapes it cannot express: `mull` has no immediate form, so
    /// a constant RHS multiplies through %ecx.
    pub(super) fn try_emit_i64_binop_fast(
        &mut self,
        dest: &Value,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
    ) -> bool {
        if std::env::var_os("CCC_NO_I64_FAST").is_some() {
            return false;
        }
        if !matches!(
            op,
            IrBinOp::Add | IrBinOp::Sub | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor | IrBinOp::Mul
        ) {
            return false;
        }
        let Some((rlo, rhi)) = self.i64_rhs_ref(rhs) else {
            return false;
        };
        let rlo_imm = rlo.starts_with('$');
        let rhi_imm = rhi.starts_with('$');
        // Zero-extended RHS (a u8/u16/u32/ptr value widened to 64 bits): the
        // high half is provably zero at the IR level, regardless of what the
        // widening cast's slot machinery wrote there. This is the parser hot
        // shape `res = res * base + val`: the alo*bhi cross product vanishes
        // and the carry becomes an immediate 0 (GCC's imull/mull/addl/adcl
        // $0 chain).
        let rhs_zero_hi = rhi_imm && rhi == "$0"
            || matches!(rhs, Operand::Value(v) if self.zext_wide_values.contains(&v.0));

        match op {
            IrBinOp::Mul => {
                self.emit_load_acc_pair(lhs);
                if rhs_zero_hi {
                    // bhi = 0: alo*bhi and its accumulation vanish.
                    if !rlo_imm {
                        self.state.emit("    movl %edx, %ecx");
                        emit!(self.state, "    imull {}, %ecx", rlo); // ahi*blo
                        emit!(self.state, "    mull {}", rlo); // eax:edx = alo*blo
                        self.state.emit("    addl %ecx, %edx");
                    } else {
                        let blo = rlo.trim_start_matches('$');
                        self.state.emit("    movl %edx, %ecx");
                        emit!(self.state, "    imull ${}, %ecx", blo); // ahi*blo
                        emit!(self.state, "    movl ${}, %edx", blo);
                        self.state.emit("    mull %edx"); // eax:edx = alo*blo
                        self.state.emit("    addl %ecx, %edx");
                    }
                } else if !rlo_imm {
                    // Memory RHS: every product reads its operand directly —
                    // no stack staging at all.
                    self.state.emit("    movl %edx, %ecx");
                    emit!(self.state, "    imull {}, %ecx", rlo); // ahi*blo
                    self.state.emit("    movl %eax, %edx");
                    emit!(self.state, "    imull {}, %edx", rhi); // alo*bhi
                    self.state.emit("    addl %edx, %ecx");
                    emit!(self.state, "    mull {}", rlo); // eax:edx = alo*blo
                    self.state.emit("    addl %ecx, %edx");
                } else {
                    // Constant RHS: `mull` has no immediate form, so the low
                    // factor stages through %edx after the cross terms.
                    let blo = rlo.trim_start_matches('$');
                    let bhi = rhi.trim_start_matches('$');
                    self.state.emit("    movl %edx, %ecx");
                    emit!(self.state, "    imull ${}, %ecx", blo); // ahi*blo
                    self.state.emit("    movl %eax, %edx");
                    emit!(self.state, "    imull ${}, %edx", bhi); // alo*bhi
                    self.state.emit("    addl %edx, %ecx");
                    emit!(self.state, "    movl ${}, %edx", blo);
                    self.state.emit("    mull %edx"); // eax:edx = alo*blo
                    self.state.emit("    addl %ecx, %edx");
                }
            }
            IrBinOp::Add | IrBinOp::Sub => {
                self.emit_load_acc_pair(lhs);
                let (mn, mn_carry) = if op == IrBinOp::Add {
                    ("addl", "adcl")
                } else {
                    ("subl", "sbbl")
                };
                emit!(self.state, "    {} {}, %eax", mn, rlo);
                if rhs_zero_hi {
                    // High half of the RHS is provably 0: carry with the
                    // 3-byte immediate form instead of a memory operand.
                    emit!(self.state, "    {} $0, %edx", mn_carry);
                } else {
                    emit!(self.state, "    {} {}, %edx", mn_carry, rhi);
                }
            }
            IrBinOp::And | IrBinOp::Or | IrBinOp::Xor => {
                self.emit_load_acc_pair(lhs);
                let mn = match op {
                    IrBinOp::And => "andl",
                    IrBinOp::Or => "orl",
                    _ => "xorl",
                };
                // Identity folding: x&0 / x|0 / x^0 = skip (zero-AND writes
                // zero, so emit `xorl reg,reg` for the 2-byte form); x&-1 /
                // x|-1 / x^-1 likewise reduce. A zero-extended VALUE RHS has
                // the same $0 high half by construction.
                let zero_lo = rlo_imm && rlo == "$0";
                let zero_hi = (rhi_imm && rhi == "$0") || rhs_zero_hi;
                let m1_lo = rlo_imm && rlo == "$-1";
                let m1_hi = rhi_imm && rhi == "$-1";
                if op == IrBinOp::And {
                    if zero_lo {
                        self.state.emit("    xorl %eax, %eax");
                    } else if !m1_lo {
                        emit!(self.state, "    andl {}, %eax", rlo);
                    }
                    if zero_hi {
                        self.state.emit("    xorl %edx, %edx");
                    } else if !m1_hi {
                        emit!(self.state, "    andl {}, %edx", rhi);
                    }
                } else if op == IrBinOp::Or {
                    if !zero_lo && !m1_lo {
                        emit!(self.state, "    orl {}, %eax", rlo);
                    } else if m1_lo {
                        self.state.emit("    movl $-1, %eax");
                    }
                    if !zero_hi && !m1_hi {
                        emit!(self.state, "    orl {}, %edx", rhi);
                    } else if m1_hi {
                        self.state.emit("    movl $-1, %edx");
                    }
                } else {
                    if !zero_lo {
                        emit!(self.state, "    xorl {}, %eax", rlo);
                    }
                    if !zero_hi {
                        emit!(self.state, "    xorl {}, %edx", rhi);
                    }
                }
            }
            _ => return false,
        }
        self.emit_store_acc_pair(dest);
        self.state.reg_cache.invalidate_all();
        true
    }

    /// i64 compare fast path (i686): LHS pair in %eax:%edx, RHS applied
    /// through immediates or memory. Eq/Ne collapse to the branchless
    /// xor-normalize-or-test sequence (5 instructions); ordered compares
    /// keep the hi-then-lo label shape but read the RHS directly.
    pub(super) fn try_emit_i64_cmp_fast(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
    ) -> bool {
        if std::env::var_os("CCC_NO_I64_FAST").is_some() {
            return false;
        }
        let Some((rlo, rhi)) = self.i64_rhs_ref(rhs) else {
            return false;
        };
        match op {
            IrCmpOp::Eq | IrCmpOp::Ne => {
                self.emit_load_acc_pair(lhs);
                emit!(self.state, "    xorl {}, %eax", rlo);
                emit!(self.state, "    xorl {}, %edx", rhi);
                self.state.emit("    orl %eax, %edx");
                let set = if op == IrCmpOp::Eq { "sete" } else { "setne" };
                emit!(self.state, "    {} %al", set);
                self.state.emit("    movzbl %al, %eax");
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
                true
            }
            IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge => {
                // Two-word signed compare: when the high words differ they
                // decide in the signed domain; otherwise the low words
                // decide in the unsigned domain. The hi-decided branch keeps
                // the signed setcc, the equal-hi fallthrough takes the
                // unsigned low setcc — the classic i386 sequence, minus the
                // stack staging of the i128 form.
                let label_id = self.state.next_label_id();
                let label_hi = format!(".Li64hidec_{}", label_id);
                let label_done = format!(".Li64done_{}", label_id);
                self.emit_load_acc_pair(lhs);
                emit!(self.state, "    cmpl {}, %edx", rhi);
                emit!(self.state, "    jne {}", label_hi);
                emit!(self.state, "    cmpl {}, %eax", rlo);
                let low_set = match op {
                    IrCmpOp::Slt => "setb",
                    IrCmpOp::Sle => "setbe",
                    IrCmpOp::Sgt => "seta",
                    IrCmpOp::Sge => "setae",
                    _ => unreachable!("signed i64 low-word cmp: {:?}", op),
                };
                emit!(self.state, "    {} %al", low_set);
                emit!(self.state, "    jmp {}", label_done);
                emit!(self.state, "{}:", label_hi);
                let high_set = match op {
                    IrCmpOp::Slt | IrCmpOp::Sle => "setl",
                    IrCmpOp::Sgt | IrCmpOp::Sge => "setg",
                    _ => unreachable!("signed i64 high-word cmp: {:?}", op),
                };
                emit!(self.state, "    {} %al", high_set);
                emit!(self.state, "{}:", label_done);
                self.state.emit("    movzbl %al, %eax");
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
                true
            }
            IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge => {
                let label_id = self.state.next_label_id();
                let label_hi = format!(".Li64u_{}", label_id);
                self.emit_load_acc_pair(lhs);
                emit!(self.state, "    cmpl {}, %edx", rhi);
                emit!(self.state, "    jne {}", label_hi);
                emit!(self.state, "    cmpl {}, %eax", rlo);
                emit!(self.state, "{}:", label_hi);
                // Unsigned: the SAME setcc serves both words (both compares
                // are in the unsigned domain), so the flags at the label
                // decide directly.
                let set = match op {
                    IrCmpOp::Ult => "setb",
                    IrCmpOp::Ule => "setbe",
                    IrCmpOp::Ugt => "seta",
                    IrCmpOp::Uge => "setae",
                    _ => unreachable!(),
                };
                emit!(self.state, "    {} %al", set);
                self.state.emit("    movzbl %al, %eax");
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
                true
            }
        }
    }


    pub(super) fn emit_sign_extend_acc_high_impl(&mut self) {
        self.state.emit("    cltd");
    }

    pub(super) fn emit_zero_acc_high_impl(&mut self) {
        self.state.emit("    xorl %edx, %edx");
    }

    pub(super) fn emit_load_acc_pair_impl(&mut self, op: &Operand) {
        match op {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    let sr0 = self.slot_ref(slot);
                    emit!(self.state, "    movl {}, %eax", sr0);
                    if self.state.wide_values.contains(&v.0) {
                        let sr4 = self.slot_ref_offset(slot, 4);
                        emit!(self.state, "    movl {}, %edx", sr4);
                    } else {
                        self.state.emit("    xorl %edx, %edx");
                    }
                } else if let Some(phys) = self.reg_assignments.get(&v.0).copied() {
                    let reg = super::emit::phys_reg_name(phys);
                    emit!(self.state, "    movl %{}, %eax", reg);
                    self.state.emit("    xorl %edx, %edx");
                }
            }
            Operand::Const(IrConst::I128(v)) => {
                let low = (*v & 0xFFFFFFFF) as i32;
                let high = ((*v >> 32) & 0xFFFFFFFF) as i32;
                emit!(self.state, "    movl ${}, %eax", low);
                emit!(self.state, "    movl ${}, %edx", high);
            }
            Operand::Const(IrConst::I64(v)) => {
                let low = (*v & 0xFFFFFFFF) as i32;
                let high = ((*v >> 32) & 0xFFFFFFFF) as i32;
                emit!(self.state, "    movl ${}, %eax", low);
                emit!(self.state, "    movl ${}, %edx", high);
            }
            Operand::Const(IrConst::F64(f)) => {
                let bits = f.to_bits();
                let low = (bits & 0xFFFFFFFF) as i32;
                let high = (bits >> 32) as i32;
                emit!(self.state, "    movl ${}, %eax", low);
                emit!(self.state, "    movl ${}, %edx", high);
            }
            Operand::Const(IrConst::Zero) => {
                self.state.emit("    xorl %eax, %eax");
                self.state.emit("    xorl %edx, %edx");
            }
            Operand::Const(c)
                if matches!(c, IrConst::I8(_) | IrConst::I16(_) | IrConst::I32(_)) =>
            {
                if let Some(ext) = c.to_i64() {
                    let low = (ext & 0xFFFFFFFF) as i32;
                    let high = ((ext >> 32) & 0xFFFFFFFF) as i32;
                    emit!(self.state, "    movl ${}, %eax", low);
                    emit!(self.state, "    movl ${}, %edx", high);
                }
            }
            _ => {
                self.operand_to_eax(op);
                self.state.emit("    xorl %edx, %edx");
            }
        }
    }

    pub(super) fn emit_store_acc_pair_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr0 = self.slot_ref(slot);
            let sr4 = self.slot_ref_offset(slot, 4);
            emit!(self.state, "    movl %eax, {}", sr0);
            emit!(self.state, "    movl %edx, {}", sr4);
        }
    }

    pub(super) fn emit_store_pair_to_slot_impl(&mut self, slot: StackSlot) {
        let sr0 = self.slot_ref(slot);
        let sr4 = self.slot_ref_offset(slot, 4);
        emit!(self.state, "    movl %eax, {}", sr0);
        emit!(self.state, "    movl %edx, {}", sr4);
    }

    pub(super) fn emit_load_pair_from_slot_impl(&mut self, slot: StackSlot) {
        let sr0 = self.slot_ref(slot);
        let sr4 = self.slot_ref_offset(slot, 4);
        emit!(self.state, "    movl {}, %eax", sr0);
        emit!(self.state, "    movl {}, %edx", sr4);
    }

    pub(super) fn emit_save_acc_pair_impl(&mut self) {
        self.state.emit("    movl %eax, %esi");
        self.state.emit("    movl %edx, %edi");
    }

    pub(super) fn emit_store_pair_indirect_impl(&mut self) {
        self.state.emit("    movl %esi, (%ecx)");
        self.state.emit("    movl %edi, 4(%ecx)");
    }

    pub(super) fn emit_load_pair_indirect_impl(&mut self) {
        self.state.emit("    movl (%ecx), %eax");
        self.state.emit("    movl 4(%ecx), %edx");
    }

    pub(super) fn emit_i128_neg_impl(&mut self) {
        self.state.emit("    notl %eax");
        self.state.emit("    notl %edx");
        self.state.emit("    addl $1, %eax");
        self.state.emit("    adcl $0, %edx");
    }

    pub(super) fn emit_i128_not_impl(&mut self) {
        self.state.emit("    notl %eax");
        self.state.emit("    notl %edx");
    }

    pub(super) fn emit_i128_to_float_call_impl(
        &mut self,
        src: &Operand,
        from_signed: bool,
        to_ty: IrType,
    ) {
        self.emit_load_acc_pair(src);
        if from_signed {
            self.state.emit("    pushl %edx");
            self.state.emit("    pushl %eax");
            self.state.emit("    fildq (%esp)");
            if to_ty == IrType::F32 {
                self.state.emit("    fstps (%esp)");
                self.state.emit("    movl (%esp), %eax");
            } else {
                self.state.emit("    fstpl (%esp)");
                self.state.emit("    movl (%esp), %eax");
            }
            self.state.emit("    addl $8, %esp");
        } else {
            let label_id = self.state.next_label_id();
            let big_label = format!(".Lu64_to_f_big_{}", label_id);
            let done_label = format!(".Lu64_to_f_done_{}", label_id);

            self.state.emit("    pushl %edx");
            self.state.emit("    pushl %eax");
            self.state.emit("    testl %edx, %edx");
            emit!(self.state, "    js {}", big_label);
            self.state.emit("    fildq (%esp)");
            emit!(self.state, "    jmp {}", done_label);
            emit!(self.state, "{}:", big_label);
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    movl 4(%esp), %edx");
            self.state.emit("    shrl $1, %eax");
            self.state.emit("    movl %edx, %ecx");
            self.state.emit("    shrl $1, %edx");
            self.state.emit("    andl $1, %ecx");
            self.state.emit("    shll $31, %ecx");
            self.state.emit("    orl %ecx, %eax");
            self.state.emit("    movl %eax, (%esp)");
            self.state.emit("    movl %edx, 4(%esp)");
            self.state.emit("    fildq (%esp)");
            self.state.emit("    fadd %st(0), %st(0)");
            emit!(self.state, "{}:", done_label);
            if to_ty == IrType::F32 {
                self.state.emit("    fstps (%esp)");
                self.state.emit("    movl (%esp), %eax");
            } else {
                self.state.emit("    fstpl (%esp)");
                self.state.emit("    movl (%esp), %eax");
            }
            self.state.emit("    addl $8, %esp");
        }
    }

    pub(super) fn emit_float_to_i128_call_impl(
        &mut self,
        src: &Operand,
        _to_signed: bool,
        from_ty: IrType,
    ) {
        // Float -> i128 truncation (result low 64 bits in edx:eax; the
        // caller widens). F32 sources travel in a GP register (4 bytes,
        // flds); F64 sources are 8 bytes and MUST be loaded with fldl -
        // the historical flds path read only half the double and produced
        // garbage (GLM audit BUG-3). Unsigned sources beyond i64 range
        // still saturate per fisttpq semantics; the i686 frontend rejects
        // __int128 so this path is only reachable from internal IR.
        if from_ty == IrType::F64 {
            self.state.emit("    subl $8, %esp");
            match src {
                Operand::Const(c) => {
                    let bits = match c {
                        IrConst::F64(f) => f.to_bits(),
                        IrConst::F32(f) => (*f as f64).to_bits(),
                        _ => 0,
                    };
                    self.state
                        .emit(&format!("    movl ${}, (%esp)", bits as u32));
                    self.state
                        .emit(&format!("    movl ${}, 4(%esp)", (bits >> 32) as u32));
                }
                _ => {
                    // 64-bit value pair: low half -> eax, high half -> edx
                    self.emit_load_acc_pair(src);
                    self.state.emit("    movl %eax, (%esp)");
                    self.state.emit("    movl %edx, 4(%esp)");
                }
            }
            self.state.emit("    fldl (%esp)");
            self.state.emit("    fisttpq (%esp)");
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    movl 4(%esp), %edx");
            self.state.emit("    addl $8, %esp");
        } else {
            self.operand_to_eax(src);
            self.state.emit("    subl $8, %esp");
            self.state.emit("    movl %eax, (%esp)");
            self.state.emit("    flds (%esp)");
            self.state.emit("    fisttpq (%esp)");
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    movl 4(%esp), %edx");
            self.state.emit("    addl $8, %esp");
        }
    }

    pub(super) fn emit_i128_prep_binop_impl(&mut self, lhs: &Operand, rhs: &Operand) {
        self.emit_load_acc_pair(rhs);
        self.state.emit("    pushl %edx");
        self.esp_adjust += 4;
        self.state.emit("    pushl %eax");
        self.esp_adjust += 4;
        self.emit_load_acc_pair(lhs);
    }

    pub(super) fn emit_i128_prep_shift_lhs_impl(&mut self, lhs: &Operand) {
        self.emit_load_acc_pair(lhs);
    }

    pub(super) fn emit_i128_add_impl(&mut self) {
        self.state.emit("    addl (%esp), %eax");
        self.state.emit("    adcl 4(%esp), %edx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_sub_impl(&mut self) {
        self.state.emit("    subl (%esp), %eax");
        self.state.emit("    sbbl 4(%esp), %edx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_mul_impl(&mut self) {
        self.state.emit("    movl %edx, %ecx");
        self.state.emit("    imull (%esp), %ecx");
        self.state.emit("    movl %eax, %edx");
        self.state.emit("    imull 4(%esp), %edx");
        self.state.emit("    addl %edx, %ecx");
        self.state.emit("    mull (%esp)");
        self.state.emit("    addl %ecx, %edx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_and_impl(&mut self) {
        self.state.emit("    andl (%esp), %eax");
        self.state.emit("    andl 4(%esp), %edx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_or_impl(&mut self) {
        self.state.emit("    orl (%esp), %eax");
        self.state.emit("    orl 4(%esp), %edx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_xor_impl(&mut self) {
        self.state.emit("    xorl (%esp), %eax");
        self.state.emit("    xorl 4(%esp), %edx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_shl_impl(&mut self) {
        let label_id = self.state.next_label_id();
        let done_label = format!(".Lshl64_done_{}", label_id);
        self.state.emit("    movl (%esp), %ecx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
        self.state.emit("    shldl %cl, %eax, %edx");
        self.state.emit("    shll %cl, %eax");
        self.state.emit("    testb $32, %cl");
        emit!(self.state, "    je {}", done_label);
        self.state.emit("    movl %eax, %edx");
        self.state.emit("    xorl %eax, %eax");
        emit!(self.state, "{}:", done_label);
    }

    pub(super) fn emit_i128_lshr_impl(&mut self) {
        let label_id = self.state.next_label_id();
        let done_label = format!(".Llshr64_done_{}", label_id);
        self.state.emit("    movl (%esp), %ecx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
        self.state.emit("    shrdl %cl, %edx, %eax");
        self.state.emit("    shrl %cl, %edx");
        self.state.emit("    testb $32, %cl");
        emit!(self.state, "    je {}", done_label);
        self.state.emit("    movl %edx, %eax");
        self.state.emit("    xorl %edx, %edx");
        emit!(self.state, "{}:", done_label);
    }

    pub(super) fn emit_i128_ashr_impl(&mut self) {
        let label_id = self.state.next_label_id();
        let done_label = format!(".Lashr64_done_{}", label_id);
        self.state.emit("    movl (%esp), %ecx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
        self.state.emit("    shrdl %cl, %edx, %eax");
        self.state.emit("    sarl %cl, %edx");
        self.state.emit("    testb $32, %cl");
        emit!(self.state, "    je {}", done_label);
        self.state.emit("    movl %edx, %eax");
        self.state.emit("    sarl $31, %edx");
        emit!(self.state, "{}:", done_label);
    }

    pub(super) fn emit_i128_divrem_call_impl(
        &mut self,
        func_name: &str,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        let di_func = match func_name {
            "__divti3" => "__divdi3",
            "__udivti3" => "__udivdi3",
            "__modti3" => "__moddi3",
            "__umodti3" => "__umoddi3",
            _ => func_name,
        };

        self.state.needs_divdi3_helpers = true;

        self.emit_load_acc_pair(rhs);
        self.state.emit("    pushl %edx");
        self.esp_adjust += 4;
        self.state.emit("    pushl %eax");
        self.esp_adjust += 4;
        self.emit_load_acc_pair(lhs);
        self.state.emit("    pushl %edx");
        self.state.emit("    pushl %eax");
        if self.state.needs_plt(di_func) {
            emit!(self.state, "    call {}@PLT", di_func);
        } else {
            emit!(self.state, "    call {}", di_func);
        }
        self.state.emit("    addl $16, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_store_result_impl(&mut self, dest: &Value) {
        self.emit_store_acc_pair(dest);
    }

    pub(super) fn emit_i128_shl_const_impl(&mut self, amount: u32) {
        if amount == 0 {
            return;
        }
        if amount >= 64 {
            self.state.emit("    xorl %eax, %eax");
            self.state.emit("    xorl %edx, %edx");
        } else if amount >= 32 {
            self.state.emit("    movl %eax, %edx");
            self.state.emit("    xorl %eax, %eax");
            if amount > 32 {
                emit!(self.state, "    shll ${}, %edx", amount - 32);
            }
        } else {
            emit!(self.state, "    shldl ${}, %eax, %edx", amount);
            emit!(self.state, "    shll ${}, %eax", amount);
        }
    }

    pub(super) fn emit_i128_lshr_const_impl(&mut self, amount: u32) {
        if amount == 0 {
            return;
        }
        if amount >= 64 {
            self.state.emit("    xorl %eax, %eax");
            self.state.emit("    xorl %edx, %edx");
        } else if amount >= 32 {
            self.state.emit("    movl %edx, %eax");
            self.state.emit("    xorl %edx, %edx");
            if amount > 32 {
                emit!(self.state, "    shrl ${}, %eax", amount - 32);
            }
        } else {
            emit!(self.state, "    shrdl ${}, %edx, %eax", amount);
            emit!(self.state, "    shrl ${}, %edx", amount);
        }
    }

    pub(super) fn emit_i128_ashr_const_impl(&mut self, amount: u32) {
        if amount == 0 {
            return;
        }
        if amount >= 64 {
            self.state.emit("    sarl $31, %edx");
            self.state.emit("    movl %edx, %eax");
        } else if amount >= 32 {
            self.state.emit("    movl %edx, %eax");
            self.state.emit("    sarl $31, %edx");
            if amount > 32 {
                emit!(self.state, "    sarl ${}, %eax", amount - 32);
            }
        } else {
            emit!(self.state, "    shrdl ${}, %edx, %eax", amount);
            emit!(self.state, "    sarl ${}, %edx", amount);
        }
    }

    pub(super) fn emit_i128_cmp_eq_impl(&mut self, is_ne: bool) {
        self.state.emit("    cmpl (%esp), %eax");
        self.state.emit("    sete %al");
        self.state.emit("    cmpl 4(%esp), %edx");
        self.state.emit("    sete %cl");
        self.state.emit("    andb %cl, %al");
        if is_ne {
            self.state.emit("    xorb $1, %al");
        }
        self.state.emit("    movzbl %al, %eax");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_cmp_ordered_impl(&mut self, op: IrCmpOp) {
        let is_signed = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge
        );

        if is_signed {
            let label_id = self.state.next_label_id();
            let label_hi_decided = format!(".Li128_hidec_{}", label_id);
            let label_done = format!(".Li128_done_{}", label_id);

            self.state.emit("    cmpl 4(%esp), %edx");
            emit!(self.state, "    jne {}", label_hi_decided);

            self.state.emit("    cmpl (%esp), %eax");
            let low_set = match op {
                IrCmpOp::Slt => "setb",
                IrCmpOp::Sle => "setbe",
                IrCmpOp::Sgt => "seta",
                IrCmpOp::Sge => "setae",
                _ => unreachable!("signed i64 low-word cmp got non-signed op: {:?}", op),
            };
            emit!(self.state, "    {} %al", low_set);
            emit!(self.state, "    jmp {}", label_done);

            emit!(self.state, "{}:", label_hi_decided);
            let high_set = match op {
                IrCmpOp::Slt => "setl",
                IrCmpOp::Sle => "setl",
                IrCmpOp::Sgt => "setg",
                IrCmpOp::Sge => "setg",
                _ => unreachable!("signed i64 high-word cmp got non-signed op: {:?}", op),
            };
            emit!(self.state, "    {} %al", high_set);

            emit!(self.state, "{}:", label_done);
        } else {
            let label_id = self.state.next_label_id();
            let high_decided = format!(".Li128_high_{}", label_id);

            self.state.emit("    cmpl 4(%esp), %edx");
            emit!(self.state, "    jne {}", high_decided);
            self.state.emit("    cmpl (%esp), %eax");
            emit!(self.state, "{}:", high_decided);

            let set_instr = match op {
                IrCmpOp::Ult => "setb",
                IrCmpOp::Ule => "setbe",
                IrCmpOp::Ugt => "seta",
                IrCmpOp::Uge => "setae",
                _ => unreachable!("unsigned i64 cmp got non-unsigned op: {:?}", op),
            };
            emit!(self.state, "    {} %al", set_instr);
        }
        self.state.emit("    movzbl %al, %eax");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_cmp_store_result_impl(&mut self, dest: &Value) {
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }
}
