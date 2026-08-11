//! X86Codegen: comparison and select operations.

use super::emit::{is_xmm_reg, phys_reg_name, phys_reg_name_32, X86Codegen};
use crate::backend::regalloc::PhysReg;
use crate::common::types::IrType;
use crate::ir::reexports::{BlockId, IrCmpOp, IrConst, Operand, Value};

impl X86Codegen {
    pub(super) fn emit_float_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        let swap_operands = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
        );
        let (first, second) = if swap_operands {
            (rhs, lhs)
        } else {
            (lhs, rhs)
        };

        // Load first operand → %xmm0 (use constant pool for FP constants)
        self.emit_fp_operand_to_xmm(first, ty, "xmm0");
        // Load second operand → %xmm1
        self.emit_fp_operand_to_xmm(second, ty, "xmm1");

        if ty == IrType::F64 {
            self.state.emit("    ucomisd %xmm1, %xmm0");
        } else {
            self.state.emit("    ucomiss %xmm1, %xmm0");
        }
        match op {
            IrCmpOp::Eq => {
                self.state.emit("    setnp %al");
                self.state.emit("    sete %cl");
                self.state.emit("    andb %cl, %al");
            }
            IrCmpOp::Ne => {
                self.state.emit("    setp %al");
                self.state.emit("    setne %cl");
                self.state.emit("    orb %cl, %al");
            }
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => {
                self.state.emit("    seta %al");
            }
            IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => {
                self.state.emit("    setae %al");
            }
        }
        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }

    pub(super) fn emit_f128_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        let swap_x87 = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
        );
        let (first_x87, second_x87) = if swap_x87 { (lhs, rhs) } else { (rhs, lhs) };
        self.emit_f128_load_to_x87(first_x87);
        self.emit_f128_load_to_x87(second_x87);
        self.state.emit("    fucomip %st(1), %st");
        self.state.emit("    fstp %st(0)");
        match op {
            IrCmpOp::Eq => {
                self.state.emit("    setnp %al");
                self.state.emit("    sete %cl");
                self.state.emit("    andb %cl, %al");
            }
            IrCmpOp::Ne => {
                self.state.emit("    setp %al");
                self.state.emit("    setne %cl");
                self.state.emit("    orb %cl, %al");
            }
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => {
                self.state.emit("    seta %al");
            }
            IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => {
                self.state.emit("    setae %al");
            }
        }
        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }

    pub(super) fn emit_int_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        // COMPARE-REPLAY: the Cmp's single use is a non-adjacent Select or
        // CondBranch (recorded in cmp_replay). Skip the ENTIRE instruction —
        // including the cmp itself — because the consumer re-emits the
        // comparison from the recorded operands right before its cmov/jcc.
        // Emitting the cmp here too would double the compare in the hot loop.
        if self.cmp_replay.contains_key(&dest.0) {
            return;
        }
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;
        self.emit_int_cmp_insn_typed(lhs, rhs, use_32bit);

        // FLAG FUSION: when this Cmp's boolean result is consumed ONLY by the
        // immediately-following Select or CondBranch (precomputed in
        // fused_cmp_dests from the IR adjacency), skip the boolean
        // materialization entirely — the flags set by the cmp/test above are
        // still live, and the consumer emits jcc/cmovcc directly. This saves
        // setcc + movzbl + (movq/testq) per comparison in branch-heavy hot
        // loops (gzip's longest_match). The dispatch loop runs instructions
        // strictly in order, so the consumer is guaranteed to be the next
        // emission; if anything else intervenes (cannot happen by
        // construction), the consumer falls back to materializing the bool —
        // but that path is unreachable for fused cmps.
        //
        // SOUNDNESS (v13): flag fusion is DISABLED when a select fallback that
        // materializes the condition via `operand_to_rax(cond)` is forced
        // (CCC_V9_SELECT = legacy v9 select emission; CCC_NO_INPLACE_SELECT =
        // disable the in-place condition test). Those legacy paths READ the
        // materialized boolean from the Cmp's register/slot, so the Cmp MUST
        // emit the setcc+movzbl — otherwise the legacy select reads a stale
        // register (it was never written because the Cmp skipped the
        // materialization). Without this gate, setting either flag produced
        // wrong select results (differential test: got 660 vs expected 1320).
        let fusion_disabled = std::env::var("CCC_NO_FLAG_FUSION").is_ok()
            || std::env::var("CCC_V9_SELECT").is_ok()
            || std::env::var("CCC_NO_INPLACE_SELECT").is_ok();
        if !fusion_disabled {
            if let Some(&chain_end) = self.fused_cmp_dests.get(&dest.0) {
                // The consumer is the next instruction(s) (possibly after a
                // flag-neutral Copy chain); record pending flags under the
                // chain-end value so the consumer's cond matches.
                self.pending_cmp = Some((chain_end, op));
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }

        let set_instr = match op {
            IrCmpOp::Eq => "sete",
            IrCmpOp::Ne => "setne",
            IrCmpOp::Slt => "setl",
            IrCmpOp::Sle => "setle",
            IrCmpOp::Sgt => "setg",
            IrCmpOp::Sge => "setge",
            IrCmpOp::Ult => "setb",
            IrCmpOp::Ule => "setbe",
            IrCmpOp::Ugt => "seta",
            IrCmpOp::Uge => "setae",
        };
        self.state.emit_fmt(format_args!("    {} %al", set_instr));

        // Register-direct: movzbl %al, %dest_reg_32 — skip %rax relay.
        // Safe because %al is part of %rax, never overlaps callee-saved registers.
        if let Some(d_reg) = self.dest_reg(dest) {
            if !is_xmm_reg(d_reg) {
                let d_name = phys_reg_name_32(d_reg);
                self.state
                    .emit_fmt(format_args!("    movzbl %al, %{}", d_name));
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }

        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }

    pub(super) fn emit_fused_cmp_branch_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        true_label: &str,
        false_label: &str,
    ) {
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;
        self.emit_int_cmp_insn_typed(lhs, rhs, use_32bit);

        let jcc = match op {
            IrCmpOp::Eq => "je",
            IrCmpOp::Ne => "jne",
            IrCmpOp::Slt => "jl",
            IrCmpOp::Sle => "jle",
            IrCmpOp::Sgt => "jg",
            IrCmpOp::Sge => "jge",
            IrCmpOp::Ult => "jb",
            IrCmpOp::Ule => "jbe",
            IrCmpOp::Ugt => "ja",
            IrCmpOp::Uge => "jae",
        };
        self.state
            .emit_fmt(format_args!("    {} {}", jcc, true_label));
        self.state.out.emit_jmp_label(false_label);
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_fused_cmp_branch_blocks_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        true_block: BlockId,
        false_block: BlockId,
    ) {
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;
        self.emit_int_cmp_insn_typed(lhs, rhs, use_32bit);

        let jcc = match op {
            IrCmpOp::Eq => "je",
            IrCmpOp::Ne => "jne",
            IrCmpOp::Slt => "jl",
            IrCmpOp::Sle => "jle",
            IrCmpOp::Sgt => "jg",
            IrCmpOp::Sge => "jge",
            IrCmpOp::Ult => "jb",
            IrCmpOp::Ule => "jbe",
            IrCmpOp::Ugt => "ja",
            IrCmpOp::Uge => "jae",
        };
        if self.state.next_block_label == Some(true_block) {
            self.state
                .out
                .emit_jcc_block(Self::invert_jcc(jcc), false_block.0);
        } else {
            self.state.out.emit_jcc_block(jcc, true_block.0);
            if self.state.next_block_label != Some(false_block) {
                self.state.out.emit_jmp_block(false_block.0);
            }
        }
        self.state.reg_cache.invalidate_all();
    }

    /// Map an integer comparison opcode to the jcc mnemonic for the
    /// condition "the comparison is true".
    fn cmp_jcc(op: IrCmpOp) -> &'static str {
        match op {
            IrCmpOp::Eq => "je",
            IrCmpOp::Ne => "jne",
            IrCmpOp::Slt => "jl",
            IrCmpOp::Sle => "jle",
            IrCmpOp::Sgt => "jg",
            IrCmpOp::Sge => "jge",
            IrCmpOp::Ult => "jb",
            IrCmpOp::Ule => "jbe",
            IrCmpOp::Ugt => "ja",
            IrCmpOp::Uge => "jae",
        }
    }

    /// Map an integer comparison opcode to the cmov CONDITION CODE (the part
    /// between "cmov" and the size suffix, e.g. "g" for cmovgq). The caller
    /// composes `cmov{cc}q`.
    fn cmp_cmov(op: IrCmpOp) -> &'static str {
        match op {
            IrCmpOp::Eq => "e",
            IrCmpOp::Ne => "ne",
            IrCmpOp::Slt => "l",
            IrCmpOp::Sle => "le",
            IrCmpOp::Sgt => "g",
            IrCmpOp::Sge => "ge",
            IrCmpOp::Ult => "b",
            IrCmpOp::Ule => "be",
            IrCmpOp::Ugt => "a",
            IrCmpOp::Uge => "ae",
        }
    }

    /// If `cond` is the destination of a fused Cmp whose flags are still
    /// pending, consume the pending flags and return the comparison opcode.
    fn take_pending_cmp(&mut self, cond: &Operand) -> Option<IrCmpOp> {
        if let Operand::Value(v) = cond {
            if let Some((dest, op)) = self.pending_cmp {
                if dest == v.0 {
                    self.pending_cmp = None;
                    return Some(op);
                }
            }
        }
        None
    }

    /// If `cond` is the destination of a Cmp eligible for compare-replay,
    /// consume the replay record (op + operands) so the consumer can re-emit
    /// the comparison and use cmovcc/jcc directly.
    fn take_replay_cmp(&mut self, cond: &Operand) -> Option<(IrCmpOp, Operand, Operand, IrType)> {
        if let Operand::Value(v) = cond {
            if let Some(rec) = self.cmp_replay.remove(&v.0) {
                return Some(rec);
            }
        }
        None
    }

    #[inline]
    fn invert_jcc(cc: &str) -> &'static str {
        match cc {
            "je" => "jne",
            "jne" => "je",
            "jl" => "jge",
            "jge" => "jl",
            "jle" => "jg",
            "jg" => "jle",
            "jb" => "jae",
            "jae" => "jb",
            "jbe" => "ja",
            "ja" => "jbe",
            _ => "jne",
        }
    }

    pub(super) fn emit_cond_branch_blocks_impl(
        &mut self,
        cond: &Operand,
        true_block: BlockId,
        false_block: BlockId,
    ) {
        let next = self.state.next_block_label;
        // If the false edge is the next emitted block, branch only on true.
        // If the true edge is next, invert the condition and branch only on
        // false. This removes the unconditional jump from the hot path.
        let false_fallthrough = next == Some(false_block);
        let true_fallthrough = next == Some(true_block);

        // FLAG FUSION: the condition is the direct result of the immediately
        // preceding Cmp; its flags are live — branch on them directly,
        // skipping the testq of the materialized boolean.
        if let Some(op) = self.take_pending_cmp(cond) {
            let jcc = Self::cmp_jcc(op);
            if true_fallthrough {
                self.state
                    .out
                    .emit_jcc_block(Self::invert_jcc(jcc), false_block.0);
            } else {
                self.state.out.emit_jcc_block(jcc, true_block.0);
                if !false_fallthrough {
                    self.state.out.emit_jmp_block(false_block.0);
                }
            }
            self.state.reg_cache.invalidate_all();
            return;
        }

        // COMPARE-REPLAY: non-adjacent Cmp consumed by this branch. Re-emit
        // the comparison (fresh flags) and branch on the condition code
        // directly — kills the setcc/movzbl + testq chain per branch.
        if let Some((op, lhs, rhs, ty)) = self.take_replay_cmp(cond) {
            self.state.reg_cache.invalidate_acc();
            let use_32bit = ty == IrType::I32 || ty == IrType::U32;
            self.emit_int_cmp_insn_typed(&lhs, &rhs, use_32bit);
            let jcc = Self::cmp_jcc(op);
            if true_fallthrough {
                self.state
                    .out
                    .emit_jcc_block(Self::invert_jcc(jcc), false_block.0);
            } else {
                self.state.out.emit_jcc_block(jcc, true_block.0);
                if !false_fallthrough {
                    self.state.out.emit_jmp_block(false_block.0);
                }
            }
            self.state.reg_cache.invalidate_all();
            return;
        }

        // Register-direct: test the condition register directly, skip %rax relay.
        if let Operand::Value(v) = cond {
            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                if !is_xmm_reg(reg) {
                    let name = phys_reg_name(reg);
                    self.state
                        .emit_fmt(format_args!("    testq %{}, %{}", name, name));
                    if true_fallthrough {
                        self.state.out.emit_jcc_block("je", false_block.0);
                    } else {
                        self.state.out.emit_jcc_block("jne", true_block.0);
                        if !false_fallthrough {
                            self.state.out.emit_jmp_block(false_block.0);
                        }
                    }
                    return;
                }
            }
        }
        self.operand_to_rax(cond);
        self.state.emit("    testq %rax, %rax");
        if true_fallthrough {
            self.state.out.emit_jcc_block("je", false_block.0);
        } else {
            self.state.out.emit_jcc_block("jne", true_block.0);
            if !false_fallthrough {
                self.state.out.emit_jmp_block(false_block.0);
            }
        }
    }

    /// Test the select condition in place, WITHOUT materializing it into a
    /// register and WITHOUT `pushfq`/`popfq`:
    ///
    ///   * register-allocated cond → `testq %reg, %reg` (no clobbers)
    ///   * stack-slot cond         → `cmpl $0, off(%rsp/%rbp)` (no clobbers;
    ///                               bools are stored as zero-extended I32)
    ///   * accumulator-cached cond → `testq %rax, %rax`
    ///
    /// Returns true when the flags were set in place (so the caller can emit a
    /// cmov directly). `pushfq`/`popfq` are serializing (~20-50 cycles each) and
    /// additionally disable the store-forwarding peephole phases, so avoiding
    /// them in the common case is a large hot-loop win.
    fn test_select_cond_in_place(&mut self, cond: &Operand) -> bool {
        if let Operand::Value(v) = cond {
            if !self.state.is_alloca(v.0) {
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    if !is_xmm_reg(reg) {
                        let name = phys_reg_name(reg);
                        self.state
                            .emit_fmt(format_args!("    testq %{}, %{}", name, name));
                        return true;
                    }
                }
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.state.out.emit_cmp_zero_mem(slot.0);
                    return true;
                }
                if self.state.reg_cache.acc_has(v.0, false)
                    || self.state.reg_cache.acc_has(v.0, true)
                {
                    self.state.emit("    testq %rax, %rax");
                    return true;
                }
            }
        }
        false
    }

    /// Materialize a select operand into a register using ONLY flag-neutral
    /// instructions. The generic `operand_to_rcx`/`operand_to_callee_reg`
    /// materialize zero constants as `xorl %reg, %reg`, which clobbers the
    /// condition flags — unacceptable between the in-place condition test and
    /// the cmov (that is exactly why the legacy path needed pushfq/popfq).
    /// Value operands are always flag-neutral in the existing helpers
    /// (mov/movl/movslq/leaq family), so only constants are handled here.
    fn select_operand_to_reg(&mut self, op: &Operand, reg64: &str, reg32: &str) {
        if let Operand::Const(c) = op {
            if c.is_zero() {
                self.state.emit_fmt(format_args!("    movq $0, %{}", reg64));
                return;
            }
            match c {
                IrConst::I8(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", *v as i64, reg64);
                }
                IrConst::I16(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", *v as i64, reg64);
                }
                IrConst::I32(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", *v as i64, reg64);
                }
                IrConst::I64(v) => {
                    if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 {
                        self.state.out.emit_instr_imm_reg("    movq", *v, reg64);
                    } else {
                        self.state.out.emit_instr_imm_reg("    movabsq", *v, reg64);
                    }
                }
                IrConst::F32(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", v.to_bits() as i64, reg64);
                }
                IrConst::F64(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movabsq", v.to_bits() as i64, reg64);
                }
                IrConst::LongDouble(v, _) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movabsq", v.to_bits() as i64, reg64);
                }
                IrConst::I128(v) => {
                    let low = *v as i64;
                    if low >= i32::MIN as i64 && low <= i32::MAX as i64 {
                        self.state.out.emit_instr_imm_reg("    movq", low, reg64);
                    } else {
                        self.state.out.emit_instr_imm_reg("    movabsq", low, reg64);
                    }
                }
                IrConst::Zero => {
                    self.state.emit_fmt(format_args!("    movq $0, %{}", reg64));
                }
            }
            let _ = reg32;
            return;
        }
        if reg64 == "rcx" {
            self.operand_to_rcx(op);
        } else if reg64 == "rax" {
            self.operand_to_rax(op);
        } else {
            // Register-direct destination: find the PhysReg by name.
            let phys = match reg64 {
                "rbx" => Some(PhysReg(1)),
                "r12" => Some(PhysReg(2)),
                "r13" => Some(PhysReg(3)),
                "r14" => Some(PhysReg(4)),
                "r15" => Some(PhysReg(5)),
                "rbp" => Some(PhysReg(6)),
                "r11" => Some(PhysReg(10)),
                "r10" => Some(PhysReg(11)),
                "r8" => Some(PhysReg(12)),
                "r9" => Some(PhysReg(13)),
                "rdi" => Some(PhysReg(14)),
                "rsi" => Some(PhysReg(15)),
                "rdx" => Some(PhysReg(16)),
                _ => None,
            };
            if let Some(p) = phys {
                self.operand_to_callee_reg(op, p);
            } else {
                self.operand_to_rax(op);
            }
        }
    }

    pub(super) fn emit_select_impl(
        &mut self,
        dest: &Value,
        cond: &Operand,
        true_val: &Operand,
        false_val: &Operand,
        _ty: IrType,
    ) {
        // V9-compat path (debugging): exact v9 emission order.
        if std::env::var("CCC_V9_SELECT").is_ok() {
            if let Some(d_reg) = self.dest_reg(dest) {
                if !is_xmm_reg(d_reg) {
                    let d_name = phys_reg_name(d_reg);
                    self.operand_to_rax(cond);
                    self.state.emit("    testq %rax, %rax");
                    self.state.emit("    pushfq");
                    if self.state.out.use_rsp_addressing {
                        self.state.out.rsp_frame_size += 8;
                    }
                    self.operand_to_callee_reg(false_val, d_reg);
                    self.operand_to_rcx(true_val);
                    self.state.emit("    popfq");
                    if self.state.out.use_rsp_addressing {
                        self.state.out.rsp_frame_size -= 8;
                    }
                    self.state
                        .emit_fmt(format_args!("    cmovneq %rcx, %{}", d_name));
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
            }
            self.operand_to_rax(cond);
            self.state.emit("    testq %rax, %rax");
            self.state.emit("    pushfq");
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size += 8;
            }
            self.operand_to_rax(false_val);
            self.operand_to_rcx(true_val);
            self.state.emit("    popfq");
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size -= 8;
            }
            self.state.emit("    cmovneq %rcx, %rax");
            self.state.reg_cache.invalidate_acc();
            self.store_rax_to(dest);
            return;
        }

        // Constant condition: statically select the operand. Both operands are
        // pure SSA values (already computed by earlier instructions), so only
        // the chosen one needs to be loaded — no test/cmov/branch at all.
        if let Operand::Const(c) = cond {
            let chosen = if c.is_zero() { false_val } else { true_val };
            if let Some(d_reg) = self.dest_reg(dest) {
                if !is_xmm_reg(d_reg) {
                    self.operand_to_callee_reg(chosen, d_reg);
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
            }
            self.operand_to_rax(chosen);
            self.store_rax_to(dest);
            return;
        }

        // FLAG FUSION: the condition is the direct result of the immediately
        // preceding Cmp whose flags are live — use the comparison's condition
        // code directly for the cmov, no test needed.
        let fused_op = self.take_pending_cmp(cond);
        // COMPARE-REPLAY: the condition is a Cmp result consumed only by this
        // select but not adjacent to the Cmp (flags clobbered in between).
        // Re-emit the comparison from the recorded operands right before the
        // cmov; the flags are then fresh. The accumulator cache is invalidated
        // first so re-materialized operands are always reloaded from their
        // canonical locations.
        let replay_op = self.take_replay_cmp(cond);
        if let Some((op, lhs, rhs, ty)) = &replay_op {
            self.state.reg_cache.invalidate_acc();
            let use_32bit = *ty == IrType::I32 || *ty == IrType::U32;
            self.emit_int_cmp_insn_typed(lhs, rhs, use_32bit);
        }
        // Test the condition in place (no pushfq). Only when the condition is
        // not directly testable (rare) do we fall back to the legacy
        // materialize + pushfq/popfq path.
        let tested_in_place = if std::env::var("CCC_NO_INPLACE_SELECT").is_ok() {
            false
        } else if fused_op.is_some() || replay_op.is_some() {
            true
        } else {
            self.test_select_cond_in_place(cond)
        };
        let cmov_cc = match (fused_op, replay_op.as_ref()) {
            (Some(op), _) => Self::cmp_cmov(op),
            (None, Some((op, _, _, _))) => Self::cmp_cmov(*op),
            (None, None) => "ne",
        };

        // Register-direct: when dest has a register, operate directly on it.
        if let Some(d_reg) = self.dest_reg(dest) {
            if !is_xmm_reg(d_reg) {
                let d_name = phys_reg_name(d_reg);
                if tested_in_place {
                    // Load order (all alias-safe):
                    //   1. true_val → %rcx   (clobbers rcx only; rcx is never
                    //      register-allocated, and this only READS other regs)
                    //   2. false_val → dest  (clobbers dest; cond was already
                    //      tested, so flags are safe; if true_val lived in dest,
                    //      rcx already holds a copy)
                    //   3. cmovcc — reads flags (set by the in-place test or the
                    //      fused Cmp; not modified by any of the above moves,
                    //      which are flag-neutral) and rcx.
                    // All materialization is flag-neutral (select_operand_to_reg
                    // avoids `xorl` for zero constants, which would clobber the
                    // flags between the test and the cmov).
                    //
                    // Direct-cmov: when the true value already lives in a
                    // (non-XMM, non-dest) register, emit `cmovcc %src, %dest`
                    // straight from it — skipping the dead `movq %src, %rcx`
                    // staging copy (fill_window's prev[] loop had one per
                    // element). The rcx staging is only needed when true_val
                    // aliases dest or isn't register-resident.
                    let true_src_reg = match true_val {
                        Operand::Value(v) => self
                            .reg_assignments
                            .get(&v.0)
                            .copied()
                            .filter(|r| !is_xmm_reg(*r) && *r != d_reg),
                        _ => None,
                    };
                    if let Some(src_reg) = true_src_reg {
                        self.select_operand_to_reg(
                            false_val,
                            phys_reg_name(d_reg),
                            phys_reg_name_32(d_reg),
                        );
                        self.state.emit_fmt(format_args!(
                            "    cmov{}q %{}, %{}",
                            cmov_cc,
                            phys_reg_name(src_reg),
                            d_name
                        ));
                    } else {
                        self.select_operand_to_reg(true_val, "rcx", "ecx");
                        self.select_operand_to_reg(
                            false_val,
                            phys_reg_name(d_reg),
                            phys_reg_name_32(d_reg),
                        );
                        self.state
                            .emit_fmt(format_args!("    cmov{}q %rcx, %{}", cmov_cc, d_name));
                    }
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
                // Legacy path: condition not testable in place.
                self.operand_to_rax(cond);
                self.state.emit("    testq %rax, %rax");
                self.state.emit("    pushfq");
                if self.state.out.use_rsp_addressing {
                    self.state.out.rsp_frame_size += 8;
                }
                self.operand_to_callee_reg(false_val, d_reg);
                self.operand_to_rcx(true_val);
                self.state.emit("    popfq");
                if self.state.out.use_rsp_addressing {
                    self.state.out.rsp_frame_size -= 8;
                }
                self.state
                    .emit_fmt(format_args!("    cmovneq %rcx, %{}", d_name));
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }

        // Accumulator fallback.
        if tested_in_place {
            // false_val → %rax, true_val → %rcx: both loads only clobber
            // rax/rcx and never modify the flags set by the in-place test or
            // the fused Cmp (flag-neutral materialization, see
            // select_operand_to_reg).
            self.select_operand_to_reg(false_val, "rax", "eax");
            self.select_operand_to_reg(true_val, "rcx", "ecx");
            self.state
                .emit_fmt(format_args!("    cmov{}q %rcx, %rax", cmov_cc));
            self.state.reg_cache.invalidate_acc();
            self.store_rax_to(dest);
            return;
        }

        // Legacy pushfq fallback (condition not testable in place).
        self.operand_to_rax(cond);
        self.state.emit("    testq %rax, %rax");
        self.state.emit("    pushfq");
        if self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size += 8;
        }
        self.operand_to_rax(false_val);
        self.operand_to_rcx(true_val);
        self.state.emit("    popfq");
        if self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size -= 8;
        }
        self.state.emit("    cmovneq %rcx, %rax");
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }
}
