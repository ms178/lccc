//! X86Codegen: cast operations.

use super::emit::X86Codegen;
use crate::backend::generation::is_i128_type;
use crate::backend::regalloc::PhysReg;
use crate::common::types::IrType;
use crate::ir::reexports::{IrConst, Operand, Value};

/// One deferred widening move (PF-15): the Cast was eligible but its
/// emission is postponed until the consuming compare (or any other next
/// instruction) decides. See `X86Codegen::pending_widen` for the full
/// soundness contract.
pub(super) struct PendingWiden {
    /// Cast destination value (single-use; the use is the consumer).
    pub dest: u32,
    /// Cast source operand (always `Operand::Value`, reg- or slot-homed).
    pub src: Operand,
    /// Narrow source type (I8/U8/I16/U16) — the compare's new width.
    pub from_ty: IrType,
    /// Destination register home the widening move would have written.
    pub dest_phys: PhysReg,
}

impl X86Codegen {
    fn pf15_trace(&self, msg: &str) {
        if std::env::var_os("CCC_DEBUG_PF15").is_some() {
            eprintln!("[pf15] {msg}");
        }
    }

    pub(super) fn emit_cast_instrs_impl(&mut self, from_ty: IrType, to_ty: IrType) {
        self.emit_cast_instrs_x86(from_ty, to_ty);
    }

    pub(super) fn emit_cast_impl(
        &mut self,
        dest: &Value,
        src: &Operand,
        from_ty: IrType,
        to_ty: IrType,
    ) {
        // Flag-fusion forwarding: this cast's destination is never read (the
        // fused consumer uses the live Cmp flags), and its source is the
        // never-materialized boolean — emitting it would read a stale register.
        if self.fused_forward_dests.contains(&dest.0) {
            return;
        }
        // W2 Load->Cast fold handshake: skip ONLY when the adjacent load
        // ACTUALLY emitted the redirected load into this value's register
        // (handshake set by emit_load_impl). Any other situation emits the
        // cast normally — the fold is sound by construction.
        if self.fold_skip_cast == Some(dest.0) {
            self.fold_skip_cast = None;
            return;
        }
        self.fold_skip_cast = None;
        // A cast OF an x87 bit-pattern value (LDFabs/LDCopysign result,
        // tracked in f128_direct_slots) is a no-op regardless of the source
        // type in the IR (the value is already the 16-byte x87 format).
        // Without this, the I64->F128 cast path ran fildq on the bit pattern
        // — 0xC000000000000000 (-3.0 x87) became -2^62.
        if to_ty == IrType::F128 && from_ty != IrType::F128 {
            if let Operand::Value(v) = src {
                if self.state.f128_direct_slots.contains(&v.0) {
                    // Pure type relabel: the 16 bytes are already the x87
                    // format. Propagate the marker to dest and copy the
                    // payload; the cast must NOT run the integer (fildq)
                    // conversion below. F128 SSA temps hold the value
                    // directly in their slot (resolve_slot_addr calls
                    // non-allocas "Indirect", but that only applies to
                    // pointer-holding slots), so copy straight to
                    // get_slot(dest) — a Direct-only copy silently dropped
                    // the payload for every non-alloca dest (the return then
                    // fldt'ed the uninitialized slot).
                    self.state.f128_direct_slots.insert(dest.0);
                    if let (Some(src_slot), Some(dest_slot)) =
                        (self.state.get_slot(v.0), self.state.get_slot(dest.0))
                    {
                        if src_slot.0 != dest_slot.0 {
                            self.state
                                .out
                                .emit_instr_rbp_reg("    movdqu", src_slot.0, "xmm0");
                            self.state
                                .out
                                .emit_instr_reg_rbp("    movdqu", "xmm0", dest_slot.0);
                        }
                    }
                    return;
                }
            }
        }
        // Intercept casts TO F128: produce full 80-bit x87 value in dest slot.
        if to_ty == IrType::F128 && from_ty != IrType::F128 && !is_i128_type(from_ty) {
            if let Some(dest_slot) = self.state.get_slot(dest.0) {
                if from_ty == IrType::F64 {
                    self.operand_to_rax(src);
                    self.state.emit("    subq $8, %rsp");
                    self.state.emit("    movq %rax, (%rsp)");
                    self.state.emit("    fldl (%rsp)");
                    self.state.emit("    addq $8, %rsp");
                } else if from_ty == IrType::F32 {
                    self.operand_to_rax(src);
                    self.state.emit("    subq $4, %rsp");
                    self.state.emit("    movl %eax, (%rsp)");
                    self.state.emit("    flds (%rsp)");
                    self.state.emit("    addq $4, %rsp");
                } else if from_ty.is_signed() || (!from_ty.is_float() && !from_ty.is_unsigned()) {
                    self.operand_to_rax(src);
                    if from_ty.size() < 8 {
                        self.emit_cast_instrs_x86(from_ty, IrType::I64);
                    }
                    self.state.emit("    subq $8, %rsp");
                    self.state.emit("    movq %rax, (%rsp)");
                    self.state.emit("    fildq (%rsp)");
                    self.state.emit("    addq $8, %rsp");
                } else {
                    self.operand_to_rax(src);
                    if from_ty.size() < 8 {
                        self.emit_cast_instrs_x86(from_ty, IrType::I64);
                    }
                    let big_label = self.state.fresh_label("u2f128_big");
                    let done_label = self.state.fresh_label("u2f128_done");
                    self.state.emit("    testq %rax, %rax");
                    self.state.out.emit_jcc_label("    js", &big_label);
                    self.state.emit("    subq $8, %rsp");
                    self.state.emit("    movq %rax, (%rsp)");
                    self.state.emit("    fildq (%rsp)");
                    self.state.emit("    addq $8, %rsp");
                    self.state.out.emit_jmp_label(&done_label);
                    self.state.out.emit_named_label(&big_label);
                    self.state.emit("    subq $8, %rsp");
                    self.state.emit("    movq %rax, (%rsp)");
                    self.state.emit("    fildq (%rsp)");
                    self.state.emit("    addq $8, %rsp");
                    self.state.emit("    subq $16, %rsp");
                    self.state.out.emit_instr_imm_reg(
                        "    movabsq",
                        -9223372036854775808i64,
                        "rax",
                    );
                    self.state.emit("    movq %rax, (%rsp)");
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", 0x403Fi64, "rax");
                    self.state.emit("    movq %rax, 8(%rsp)");
                    self.state.emit("    fldt (%rsp)");
                    self.state.emit("    addq $16, %rsp");
                    self.state.emit("    faddp %st, %st(1)");
                    self.state.out.emit_named_label(&done_label);
                }
                self.state.out.emit_instr_rbp("    fstpt", dest_slot.0);
                self.state.out.emit_instr_rbp("    fldt", dest_slot.0);
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.state.reg_cache.set_acc(dest.0, false);
                self.state.f128_direct_slots.insert(dest.0);
                return;
            }
        }

        // Intercept F128 -> F64/F32 casts
        if from_ty == IrType::F128 && (to_ty == IrType::F64 || to_ty == IrType::F32) {
            self.emit_f128_load_to_x87(src);
            if to_ty == IrType::F64 {
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    movq (%rsp), %rax");
                self.state.emit("    addq $8, %rsp");
            } else {
                self.state.emit("    subq $4, %rsp");
                self.state.emit("    fstps (%rsp)");
                self.state.emit("    movl (%rsp), %eax");
                self.state.emit("    addq $4, %rsp");
            }
            self.state.reg_cache.invalidate_acc();
            self.store_rax_to(dest);
            return;
        }

        // Intercept F128 -> integer casts when we know the source's memory location
        if from_ty == IrType::F128 && !to_ty.is_float() && !is_i128_type(to_ty) {
            if let Operand::Value(v) = src {
                if self.state.f128_direct_slots.contains(&v.0) {
                    if let Some(slot) = self.state.get_slot(v.0) {
                        let addr = crate::backend::state::SlotAddr::Direct(slot);
                        self.emit_f128_to_int_from_memory(&addr, to_ty);
                        self.store_rax_to(dest);
                        return;
                    }
                }
                if let Some((ptr_id, _offset, _is_indirect)) = self.state.get_f128_source(v.0) {
                    if let Some(addr) = self.state.resolve_slot_addr(ptr_id) {
                        self.emit_f128_to_int_from_memory(&addr, to_ty);
                        self.store_rax_to(dest);
                        return;
                    }
                }
            }
            if let Operand::Const(IrConst::LongDouble(_, f128_bytes)) = src {
                let x87 = crate::common::long_double::f128_bytes_to_x87_bytes(f128_bytes);
                self.state.emit("    subq $16, %rsp");
                let lo = u64::from_le_bytes(x87[0..8].try_into().unwrap());
                let hi = u16::from_le_bytes(x87[8..10].try_into().unwrap());
                self.state
                    .out
                    .emit_instr_imm_reg("    movabsq", lo as i64, "rax");
                self.state.emit("    movq %rax, (%rsp)");
                self.state
                    .out
                    .emit_instr_imm_reg("    movq", hi as i64, "rax");
                self.state.emit("    movq %rax, 8(%rsp)");
                self.state.emit("    fldt (%rsp)");
                self.state.emit("    addq $16, %rsp");
                self.emit_f128_st0_to_int(to_ty);
                self.store_rax_to(dest);
                return;
            }
        }
        // PF-15 narrowing-widen defer: a single-use widening cast from a
        // byte/half type is NOT emitted here — it is recorded and the
        // consuming compare (if it is one) folds it into a narrow `cmpb`/
        // `cmpw` on the original source. Any other consumer flushes the
        // recorded move first, making the defer invisible. try_record
        // returns false for every non-eligible shape (flushing whatever is
        // deferred — its own emission could clobber a deferred source
        // home) and this cast then emits normally below.
        if self.try_record_pending_widen(dest, src, from_ty, to_ty) {
            return;
        }
        // PF-16b: widening casts of Clz/Ctz/Popcount results (bit31
        // provably zero, upper bits zeroed by the 32-bit write itself)
        // take the UNSIGNED path: zero-extension equals the required
        // sign-extension, and movl/movzbl beats cltq/movslq/movsbq on
        // length and decoded opacity. The rewrite is a pure from_ty
        // reclassification — every downstream unsigned path (register-
        // direct, slot, accumulator) then emits the zero form.
        let from_ty = if from_ty.is_signed()
            && !from_ty.is_float()
            && from_ty.size() <= 4
            && to_ty.size() == 8
            && !to_ty.is_float()
            && matches!(src, Operand::Value(v) if self.bitop_nonneg_values.contains(&v.0))
        {
            match from_ty {
                IrType::I8 => IrType::U8,
                IrType::I16 => IrType::U16,
                _ => IrType::U32,
            }
        } else {
            from_ty
        };

        // Register-direct integer casts: bypass the accumulator when the destination
        // has a physical register. Instead of load→cast→store through %rax, emit the
        // cast instruction directly targeting the destination register.
        if !from_ty.is_float()
            && !to_ty.is_float()
            && !is_i128_type(from_ty)
            && !is_i128_type(to_ty)
        {
            if let Some(dest_phys) = self.dest_reg(dest) {
                if !super::emit::is_xmm_reg(dest_phys) {
                    if self.try_emit_cast_reg_direct(dest, src, from_ty, to_ty, dest_phys) {
                        return;
                    }
                }
            }
        }

        // register-direct integer -> float casts. Emit cvtsi2sd/cvtsi2ss
        // directly into the destination's allocated XMM register instead of the
        // accumulator path's GPR round-trip (cvtsi2sd %rax,%xmm0; movq
        // %xmm0,%rax; movq %rax,%xmmN). U64 needs the shift+round dance and is
        // left to the general scalar path below. This runs BEFORE the general
        // scalar-FP path because targeting the destination register directly
        // beats staging through %xmm0 + store_xmm_to.
        if !from_ty.is_float()
            && (to_ty == IrType::F64 || to_ty == IrType::F32)
            && from_ty != IrType::U64
            // Ptr normalizes to U64: a high-bit address must take the U64
            // shift+round dance below, not the plain signed cvtsi2sdq this
            // fast path emits (`(double)(void*)0xffffffff80000000` was
            // -2^31 instead of 1.8e19). Ptr == U64 semantically, so exclude
            // it here exactly like U64.
            && from_ty != IrType::Ptr
            && !is_i128_type(from_ty)
        {
            if let Some(dest_phys) = self.dest_reg(dest) {
                if super::emit::is_xmm_reg(dest_phys) {
                    let dname = super::emit::phys_reg_name(dest_phys);
                    // operand_to_rax handles the accumulator cache
                    // (immediately-consumed sources have no slot and no
                    // register — value_to_reg/operand_to_reg would panic).
                    self.operand_to_rax(src);
                    if from_ty.is_unsigned() {
                        self.emit_zero_extend_to_rax(from_ty);
                    } else {
                        self.emit_sign_extend_to_rax(from_ty);
                    }
                    if to_ty == IrType::F64 {
                        self.state
                            .emit_fmt(format_args!("    cvtsi2sdq %rax, %{}", dname));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    cvtsi2ssq %rax, %{}", dname));
                    }
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
            }
        }

        // Scalar FP casts: stage FP sources directly in XMM registers and keep
        // FP results in the SSE domain instead of round-tripping through %rax.
        if !is_i128_type(from_ty)
            && !is_i128_type(to_ty)
            && from_ty != IrType::F128
            && to_ty != IrType::F128
            && (from_ty.is_float() || to_ty.is_float())
        {
            if self.try_emit_scalar_fp_cast(dest, src, from_ty, to_ty) {
                return;
            }
        }

        // Fall through to default implementation for all other cases
        crate::backend::traits::emit_cast_default(self, dest, src, from_ty, to_ty);
    }

    /// Try to emit a scalar F64/F32 cast keeping FP values in XMM registers
    /// end-to-end. Returns true if the cast was emitted, false to fall through.
    /// Mirrors the instruction sequences of emit_generic_cast in f128.rs, but
    /// takes FP inputs directly in %xmm0 and stores FP results via store_xmm_to.
    fn try_emit_scalar_fp_cast(
        &mut self,
        dest: &Value,
        src: &Operand,
        from_ty: IrType,
        to_ty: IrType,
    ) -> bool {
        use crate::backend::cast::{classify_cast, CastKind};
        match classify_cast(from_ty, to_ty) {
            CastKind::Noop if from_ty.is_float() && to_ty.is_float() => {
                // Float-to-float copy (e.g. LongDouble-as-F64): just move it.
                self.emit_fp_operand_to_xmm(src, from_ty, "xmm0");
                self.store_xmm_to(dest, "xmm0", to_ty);
                true
            }
            CastKind::FloatToFloat { widen } => {
                self.emit_fp_operand_to_xmm(src, from_ty, "xmm0");
                if widen {
                    self.state.emit("    cvtss2sd %xmm0, %xmm0");
                } else {
                    self.state.emit("    cvtsd2ss %xmm0, %xmm0");
                }
                self.store_xmm_to(dest, "xmm0", to_ty);
                true
            }
            CastKind::SignedToFloat {
                to_f64,
                from_ty: ft,
            } => {
                self.operand_to_rax(src);
                // Sign-extend sub-64-bit sources to 64 bits (Noop for I64).
                self.emit_cast_instrs_x86(ft, IrType::I64);
                if to_f64 {
                    self.state.emit("    cvtsi2sdq %rax, %xmm0");
                } else {
                    self.state.emit("    cvtsi2ssq %rax, %xmm0");
                }
                self.store_xmm_to(dest, "xmm0", to_ty);
                true
            }
            CastKind::UnsignedToFloat {
                to_f64,
                from_ty: ft,
            } => {
                self.operand_to_rax(src);
                if ft == IrType::U64 {
                    // U64: handle values >= 2^63 via shift+round (mirrors
                    // emit_u64_to_float, but the result stays in %xmm0).
                    let big_label = self.state.fresh_label("u2f_big");
                    let done_label = self.state.fresh_label("u2f_done");
                    self.state.emit("    testq %rax, %rax");
                    self.state.out.emit_jcc_label("    js", &big_label);
                    if to_f64 {
                        self.state.emit("    cvtsi2sdq %rax, %xmm0");
                    } else {
                        self.state.emit("    cvtsi2ssq %rax, %xmm0");
                    }
                    self.state.out.emit_jmp_label(&done_label);
                    self.state.out.emit_named_label(&big_label);
                    self.state.emit("    movq %rax, %rcx");
                    self.state.emit("    shrq $1, %rax");
                    self.state.emit("    andq $1, %rcx");
                    self.state.emit("    orq %rcx, %rax");
                    if to_f64 {
                        self.state.emit("    cvtsi2sdq %rax, %xmm0");
                        self.state.emit("    addsd %xmm0, %xmm0");
                    } else {
                        self.state.emit("    cvtsi2ssq %rax, %xmm0");
                        self.state.emit("    addss %xmm0, %xmm0");
                    }
                    self.state.out.emit_named_label(&done_label);
                    self.state.reg_cache.invalidate_sec(); // clobbered %rcx
                    self.store_xmm_to(dest, "xmm0", to_ty);
                } else {
                    // U8/U16/U32: already zero-extended in %rax by the load.
                    if to_f64 {
                        self.state.emit("    cvtsi2sdq %rax, %xmm0");
                    } else {
                        self.state.emit("    cvtsi2ssq %rax, %xmm0");
                    }
                    self.store_xmm_to(dest, "xmm0", to_ty);
                }
                true
            }
            CastKind::FloatToSigned { from_f64 } => {
                self.emit_fp_operand_to_xmm(src, from_ty, "xmm0");
                if from_f64 {
                    self.state.emit("    cvttsd2siq %xmm0, %rax");
                } else {
                    self.state.emit("    cvttss2siq %xmm0, %rax");
                }
                // Sign-extend to the target width (Noop for 64-bit targets).
                self.emit_cast_instrs_x86(IrType::I64, to_ty);
                self.state.reg_cache.invalidate_acc();
                self.store_rax_to(dest);
                true
            }
            CastKind::FloatToUnsigned { from_f64, to_u64 } => {
                self.emit_fp_operand_to_xmm(src, from_ty, "xmm0");
                if from_f64 && to_u64 {
                    // F64 → U64: handle values >= 2^63 (mirrors
                    // emit_float_to_unsigned, but the input is already in %xmm0).
                    let big_label = self.state.fresh_label("f2u_big");
                    let done_label = self.state.fresh_label("f2u_done");
                    self.state.emit("    movabsq $4890909195324358656, %rcx");
                    self.state.emit("    movq %rcx, %xmm1");
                    self.state.emit("    ucomisd %xmm1, %xmm0");
                    self.state.out.emit_jcc_label("    jae", &big_label);
                    self.state.emit("    cvttsd2siq %xmm0, %rax");
                    self.state.out.emit_jmp_label(&done_label);
                    self.state.out.emit_named_label(&big_label);
                    self.state.emit("    subsd %xmm1, %xmm0");
                    self.state.emit("    cvttsd2siq %xmm0, %rax");
                    self.state.emit("    movabsq $9223372036854775808, %rcx");
                    self.state.emit("    addq %rcx, %rax");
                    self.state.out.emit_named_label(&done_label);
                    self.state.reg_cache.invalidate_sec(); // clobbered %rcx
                } else if from_f64 {
                    self.state.emit("    cvttsd2siq %xmm0, %rax");
                } else {
                    self.state.emit("    cvttss2siq %xmm0, %rax");
                }
                if !to_u64 {
                    // Zero-extend/truncate to the target unsigned width.
                    self.emit_cast_instrs_x86(IrType::I64, to_ty);
                }
                self.state.reg_cache.invalidate_acc();
                self.store_rax_to(dest);
                true
            }
            _ => false,
        }
    }

    /// Try to emit an integer cast directly to the destination register.
    /// Returns true if the cast was emitted, false if we should fall through.
    ///
    /// Key optimization: when the source is a stack slot, fuse the load and cast
    /// into a single instruction (e.g., `movslq -N(%rsp), %r12`), saving 2 instructions
    /// vs the accumulator path (`movq -N(%rsp), %rax; cltq; movq %rax, %r12`).
    /// Emit the widening move for an IntWiden cast into a register home,
    /// from either a register or a slot source. This is the SINGLE
    /// definition of the widening sequences: the register-direct cast path
    /// and the PF-15 deferred-widen flush both route through it, so the
    /// deferred form is bit-identical to the immediate form.
    fn emit_int_widen_move(&mut self, src: &Operand, ft: IrType, dest_phys: PhysReg) {
        use super::emit::{phys_reg_name, phys_reg_name_32, typed_phys_reg_name};
        let dest_64 = phys_reg_name(dest_phys);
        let dest_32 = phys_reg_name_32(dest_phys);
        let src_phys = self.operand_reg(src);
        let src_slot = match src {
            Operand::Value(v) => self.state.get_slot(v.0).map(|sl| sl.0),
            _ => None,
        };
        if let Some(src_reg) = src_phys {
            // XMM-homed source (integer bit-punned in the float domain): the
            // extending move forms below name GP sub-registers only, and
            // typed_phys_reg_name has no spelling for xmm indices. Stage
            // through the generic callee-register path, which owns the
            // xmm→GPR move.
            if super::emit::is_xmm_reg(src_reg) {
                self.operand_to_callee_reg(src, dest_phys);
                return;
            }
            // Source is in a register — emit reg-to-reg extending move.
            let src_typed = typed_phys_reg_name(src_reg, ft);
            if ft.is_signed() {
                match ft.size() {
                    1 => self
                        .state
                        .emit_fmt(format_args!("    movsbq %{}, %{}", src_typed, dest_64)),
                    2 => self
                        .state
                        .emit_fmt(format_args!("    movswq %{}, %{}", src_typed, dest_64)),
                    4 => self.state.emit_fmt(format_args!(
                        "    movslq %{}, %{}",
                        phys_reg_name_32(src_reg),
                        dest_64
                    )),
                    _ => {
                        self.operand_to_callee_reg(src, dest_phys);
                    }
                }
            } else {
                match ft.size() {
                    1 => self
                        .state
                        .emit_fmt(format_args!("    movzbl %{}, %{}", src_typed, dest_32)),
                    2 => self
                        .state
                        .emit_fmt(format_args!("    movzwl %{}, %{}", src_typed, dest_32)),
                    4 => {
                        let src_32 = phys_reg_name_32(src_reg);
                        self.state
                            .emit_fmt(format_args!("    movl %{}, %{}", src_32, dest_32));
                    }
                    _ => {
                        self.operand_to_callee_reg(src, dest_phys);
                    }
                }
            }
        } else if let Some(slot_off) = src_slot {
            // Source is a stack slot — emit fused load+extend directly to
            // dest. Uses emit_instr_rbp_reg which handles rbp/rsp
            // addressing automatically.
            if ft.is_signed() {
                match ft.size() {
                    1 => self
                        .state
                        .out
                        .emit_instr_rbp_reg("    movsbq", slot_off, dest_64),
                    2 => self
                        .state
                        .out
                        .emit_instr_rbp_reg("    movswq", slot_off, dest_64),
                    4 => self
                        .state
                        .out
                        .emit_instr_rbp_reg("    movslq", slot_off, dest_64),
                    _ => self
                        .state
                        .out
                        .emit_instr_rbp_reg("    movq", slot_off, dest_64),
                }
            } else {
                match ft.size() {
                    1 => self
                        .state
                        .out
                        .emit_instr_rbp_reg("    movzbl", slot_off, dest_32),
                    2 => self
                        .state
                        .out
                        .emit_instr_rbp_reg("    movzwl", slot_off, dest_32),
                    4 => self
                        .state
                        .out
                        .emit_instr_rbp_reg("    movl", slot_off, dest_32),
                    _ => self
                        .state
                        .out
                        .emit_instr_rbp_reg("    movq", slot_off, dest_64),
                }
            }
        } else {
            // No stable home: stage through the accumulator (the generic
            // cast path would have done the same). Only reachable from the
            // flush side; the record path refuses home-less sources.
            self.operand_to_callee_reg(src, dest_phys);
        }
    }

    /// PF-15 record path. Returns true when the cast's widening move was
    /// DEFERRED (nothing emitted); false when the cast must emit normally.
    ///
    /// Eligibility: widening cast from a byte/half type to a 32/64-bit
    /// type, destination in a GPR home, source a Value with a stable home
    /// (register or slot), destination used exactly once. The consumer
    /// match (is that single use an integer compare of two such casts?)
    /// happens at the compare; any other consumer simply flushes.
    ///
    /// Refusal contract: the caller emits this cast normally, and those
    /// emissions (register moves, slot stores) may legally clobber a
    /// deferred source's home — RA reused it the moment the deferred
    /// source's only modelled use (this cast) stopped covering it. Any
    /// refusal therefore flushes the deferred moves FIRST.
    fn try_record_pending_widen(
        &mut self,
        dest: &Value,
        src: &Operand,
        from_ty: IrType,
        to_ty: IrType,
    ) -> bool {
        macro_rules! refuse {
            ($why:expr) => {{
                self.pf15_trace(concat!("refuse: ", $why));
                self.flush_pending_widen_impl();
                return false;
            }};
        }
        if !matches!(from_ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16) {
            refuse!("from_ty");
        }
        if to_ty.is_float() || !(to_ty.size() == 4 || to_ty.size() == 8) || is_i128_type(to_ty) {
            refuse!("to_ty");
        }
        if self.value_use_counts.get(&dest.0).copied() != Some(1) {
            refuse!("use_count");
        }
        let src_val = match src {
            Operand::Value(v) => v.0,
            _ => refuse!("src_not_value"),
        };
        let Some(dest_phys) = self.dest_reg(dest) else {
            refuse!("no_dest_reg");
        };
        if super::emit::is_xmm_reg(dest_phys) {
            refuse!("xmm_dest");
        }
        let src_reg = self.operand_reg(src);
        let src_slot = self.state.get_slot(src_val).map(|sl| sl.0);
        if src_reg.is_none() && src_slot.is_none() {
            refuse!("no_src_home");
        }
        // A deferred cast must not be shadowed by another deferred cast
        // whose widening move would clobber this cast's source register
        // home before the compare (or a flush) reads it. RA may legally
        // hand this destination the first source's home: the source's only
        // modelled use is this deferred move, so its range appears to end
        // here. Conflict => flush the earlier moves and emit this cast
        // normally (the transform degrades, never miscompiles).
        for slot in 0..2 {
            let pending_src = self.pending_widen[slot]
                .as_ref()
                .map(|p| match &p.src {
                    Operand::Value(v) => Some(v.0),
                    _ => None,
                })
                .flatten();
            if let Some(pv) = pending_src {
                if self.reg_assignments.get(&pv).copied() == Some(dest_phys) {
                    refuse!("conflict with earlier pending src home");
                }
            }
        }
        // Slots are never full here: only a Cmp consumes a full pair and
        // every non-Cmp/Cast dispatch flushes first (generate_instruction).
        // Belt-and-braces: if they somehow are, fall back to normal emit.
        if self.pending_widen.iter().all(|p| p.is_some()) {
            refuse!("both slots full");
        }
        let free = if self.pending_widen[0].is_none() {
            0
        } else {
            1
        };
        self.pending_widen[free] = Some(PendingWiden {
            dest: dest.0,
            src: src.clone(),
            from_ty,
            dest_phys,
        });
        self.pf15_trace("record");
        true
    }

    /// Emit any deferred widening moves, in program order. Called by the
    /// generate_instruction pre-dispatch hook for every non-Cmp/non-Cast
    /// instruction, on every refused cast, by the compare-replay exits,
    /// and on any compare that does not (or only partially) match.
    pub(super) fn flush_pending_widen_impl(&mut self) {
        for slot in 0..2 {
            if let Some(p) = self.pending_widen[slot].take() {
                self.emit_int_widen_move(&p.src, p.from_ty, p.dest_phys);
            }
        }
    }

    /// If `lhs`/`rhs` are exactly the two recorded deferred casts (in
    /// either order), take both records and return
    /// `((lhs_src, lhs_ty, lhs_signed), (rhs_src, rhs_ty, rhs_signed))`.
    /// Leaves the records untouched on any mismatch — a lone pending is
    /// NEVER silently dropped here (its widening move would otherwise
    /// never be emitted and the consumer would read garbage).
    #[allow(clippy::type_complexity)]
    fn take_pending_widen_pair(
        &mut self,
        lhs: &Operand,
        rhs: &Operand,
    ) -> Option<(
        (Operand, IrType, bool, PhysReg),
        (Operand, IrType, bool, PhysReg),
    )> {
        let (lv, rv) = match (lhs, rhs) {
            (Operand::Value(l), Operand::Value(r)) => (l.0, r.0),
            _ => {
                self.pf15_trace("take_pair: operands not both values");
                return None;
            }
        };
        let p0 = self.pending_widen[0].take();
        let p1 = self.pending_widen[1].take();
        if p0.is_none() || p1.is_none() {
            // Partial occupancy: restore whatever was taken and refuse.
            self.pf15_trace("take_pair: partial occupancy");
            if let Some(a) = p0 {
                self.pending_widen[0] = Some(a);
            }
            if let Some(b) = p1 {
                self.pending_widen[1] = Some(b);
            }
            return None;
        }
        let a = p0.unwrap();
        let b = p1.unwrap();
        // Positional match, both orders.
        if a.dest == lv && b.dest == rv {
            let sa = a.from_ty.is_signed();
            let sb = b.from_ty.is_signed();
            return Some((
                (a.src, a.from_ty, sa, a.dest_phys),
                (b.src, b.from_ty, sb, b.dest_phys),
            ));
        }
        if a.dest == rv && b.dest == lv {
            let sa = b.from_ty.is_signed();
            let sb = a.from_ty.is_signed();
            return Some((
                (b.src, b.from_ty, sa, b.dest_phys),
                (a.src, a.from_ty, sb, a.dest_phys),
            ));
        }
        // Mismatch: restore and report.
        self.pf15_trace("take_pair: dest mismatch");
        self.pending_widen[0] = Some(a);
        self.pending_widen[1] = Some(b);
        None
    }

    /// PF-15 consume path, called from the generate_instruction Cmp arm
    /// BEFORE emit_cmp — including when MachInst will lower the compare —
    /// so every possible consumer sees either a folded narrow compare or
    /// fully materialized widening moves, never a half-deferred state.
    ///
    /// Full-pair shape `cmp(cast a, cast b)`: folds to
    /// `(src_a, src_b, narrow_ty, mapped_op)` when the extension/opcode
    /// compatibility matrix accepts; the two widening moves are then never
    /// emitted at all. Single-side shape `cmp(cast a, C)`: folds when the
    /// constant fits the narrow domain exactly under the same matrix.
    /// Everything else: flush the deferred moves and return None (the
    /// caller emits the wide compare from the original operands).
    ///
    /// Flag-exactness matrix (the defer-side contract lives on
    /// `X86Codegen::pending_widen`):
    ///   * sext+sext: sign-extension is order-preserving, so the narrow
    ///     signed compare produces flag-identical answers for the signed
    ///     setcc family {eq, ne, slt, sle, sgt, sge}. Unsigned opcodes on
    ///     sign-extended operands compare negated bit patterns — refused.
    ///   * zext+zext: zero-extension makes both values non-negative, where
    ///     signed and unsigned relational order coincide; every IR opcode
    ///     maps onto the unsigned narrow set {eq, ne, ult, ule, ugt, uge}.
    ///   * mixed sext/zext: the widened values diverge (0xFF -> -1 vs
    ///     255) — refused.
    ///   * const rhs: allowed inside the narrow type's exact domain, with
    ///     the same per-extension opcode sets (a negative constant can
    ///     never compare against a zero-extended value; unsigned opcodes
    ///     on sign-extended operands are refused, as above).
    #[allow(clippy::type_complexity)]
    pub(super) fn narrow_cmp_operands_impl(
        &mut self,
        dest: u32,
        op: crate::ir::reexports::IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
    ) -> Option<(Operand, Operand, IrType, crate::ir::reexports::IrCmpOp)> {
        use crate::ir::reexports::IrCmpOp;
        // Compare-replay dests re-emit the comparison at a DISTANT consumer
        // (cmov/jcc far from here). The adjacency guarantee that keeps the
        // deferred sources' homes intact does not reach there: materialize.
        if self.cmp_replay.contains_key(&dest) {
            self.pf15_trace("narrow: replay dest, flushing");
            self.flush_pending_widen_impl();
            return None;
        }
        self.pf15_trace("narrow: entered");
        let op_map_zext = |op: IrCmpOp| -> IrCmpOp {
            match op {
                IrCmpOp::Slt => IrCmpOp::Ult,
                IrCmpOp::Sle => IrCmpOp::Ule,
                IrCmpOp::Sgt => IrCmpOp::Ugt,
                IrCmpOp::Sge => IrCmpOp::Uge,
                signed_or_unsigned_keep => signed_or_unsigned_keep,
            }
        };
        let signed_only = |op: IrCmpOp| -> Option<IrCmpOp> {
            match op {
                IrCmpOp::Eq
                | IrCmpOp::Ne
                | IrCmpOp::Slt
                | IrCmpOp::Sle
                | IrCmpOp::Sgt
                | IrCmpOp::Sge => Some(op),
                _ => None,
            }
        };
        // Full-pair shape.
        if let Some(((lsrc, lty, lsign, ldest), (rsrc, rty, rsign, rdest))) =
            self.take_pending_widen_pair(lhs, rhs)
        {
            let folded = if lty != rty {
                None
            } else if lsign && rsign {
                signed_only(op).map(|m| (lsrc.clone(), rsrc.clone(), lty, m))
            } else if !lsign && !rsign {
                Some((lsrc.clone(), rsrc.clone(), lty, op_map_zext(op)))
            } else {
                None // mixed sext/zext: widened values diverge
            };
            self.pf15_trace(if folded.is_some() {
                "pair FOLD"
            } else {
                "pair matrix refuse"
            });
            if let Some(f) = folded {
                return Some(f);
            }
            // Refused: the records are already taken OUT of the slots, so
            // re-emit both widening moves explicitly, in program order.
            self.emit_int_widen_move(&lsrc, lty, ldest);
            self.emit_int_widen_move(&rsrc, rty, rdest);
            return None;
        }
        // Single-side const shape: cmp(cast(x), C) with C in the narrow
        // domain. (Cmp with a constant lhs is canonicalized to rhs
        // upstream; a const lhs here simply takes the flush path below.)
        if self.pending_widen[1].is_some() {
            self.flush_pending_widen_impl();
            return None;
        }
        let (src, from_ty, dest_phys) = match (lhs, self.pending_widen[0].take()) {
            (Operand::Value(v), Some(p)) if p.dest == v.0 => (p.src, p.from_ty, p.dest_phys),
            (_, other) => {
                self.pending_widen[0] = other;
                self.flush_pending_widen_impl();
                return None;
            }
        };
        let (lo, hi) = if from_ty.is_signed() {
            (
                -(1i64 << (from_ty.size() * 8 - 1)),
                (1i64 << (from_ty.size() * 8 - 1)) - 1,
            )
        } else {
            (0i64, (1i64 << (from_ty.size() * 8)) - 1)
        };
        let folded = match rhs {
            Operand::Const(c) => match c.clone().to_i64() {
                Some(cv) if (lo..=hi).contains(&cv) => {
                    if from_ty.is_signed() {
                        signed_only(op).map(|m| (src.clone(), rhs.clone(), from_ty, m))
                    } else {
                        Some((src.clone(), rhs.clone(), from_ty, op_map_zext(op)))
                    }
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(f) = folded {
            return Some(f);
        }
        // Refused after take: re-emit the taken widening move explicitly.
        self.emit_int_widen_move(&src, from_ty, dest_phys);
        None
    }

    /// Register-direct cast whose source currently lives in the accumulator
    /// (see the call site in `try_emit_cast_reg_direct`). Returns false for
    /// shapes that have no single-instruction extending/truncating move;
    /// nothing is emitted in that case.
    fn emit_cast_from_accumulator(
        &mut self,
        kind: crate::backend::cast::CastKind,
        to_ty: IrType,
        dest_32: &str,
        dest_64: &str,
    ) -> bool {
        use crate::backend::cast::CastKind;
        match kind {
            CastKind::IntWiden { from_ty: ft, .. } => {
                let (insn, src, dst) = match (ft.is_signed(), ft.size()) {
                    (true, 1) => ("movsbq", "al", dest_64),
                    (true, 2) => ("movswq", "ax", dest_64),
                    (true, 4) => ("movslq", "eax", dest_64),
                    (false, 1) => ("movzbl", "al", dest_32),
                    (false, 2) => ("movzwl", "ax", dest_32),
                    (false, 4) => ("movl", "eax", dest_32),
                    _ => return false,
                };
                self.state
                    .emit_fmt(format_args!("    {insn} %{src}, %{dst}"));
                true
            }
            CastKind::IntNarrow { to_ty: t } => {
                let (insn, src, dst) = match (t.size(), t.is_unsigned()) {
                    (4, true) => ("movl", "eax", dest_32),
                    (4, false) => ("movslq", "eax", dest_64),
                    (2, _) => ("movzwl", "ax", dest_32),
                    (1, _) => ("movzbl", "al", dest_32),
                    _ => return false,
                };
                self.state
                    .emit_fmt(format_args!("    {insn} %{src}, %{dst}"));
                true
            }
            CastKind::Noop | CastKind::UnsignedToSignedSameSize { .. } => {
                if to_ty.size() <= 4 {
                    self.state
                        .emit_fmt(format_args!("    movl %eax, %{dest_32}"));
                } else {
                    self.state
                        .emit_fmt(format_args!("    movq %rax, %{dest_64}"));
                }
                true
            }
            CastKind::SignedToUnsignedSameSize { to_ty: t } if t.size() == 4 => {
                self.state
                    .emit_fmt(format_args!("    movl %eax, %{dest_32}"));
                true
            }
            _ => false,
        }
    }

    fn try_emit_cast_reg_direct(
        &mut self,
        _dest: &Value,
        src: &Operand,
        from_ty: IrType,
        to_ty: IrType,
        dest_phys: crate::backend::regalloc::PhysReg,
    ) -> bool {
        use super::emit::{phys_reg_name, phys_reg_name_32, typed_phys_reg_name};
        use crate::backend::cast::{classify_cast, CastKind};

        let dest_64 = phys_reg_name(dest_phys);
        let dest_32 = phys_reg_name_32(dest_phys);
        let kind = classify_cast(from_ty, to_ty);

        // Resolve source: either a physical register or a stack slot offset.
        let src_phys = self.operand_reg(src);
        let src_slot = match src {
            Operand::Value(v) => self.state.get_slot(v.0).map(|s| s.0),
            _ => None,
        };

        // For constants, fall through to default path (it's already efficient).
        if matches!(src, Operand::Const(_)) {
            return false;
        }

        // Accumulator-staged source (an immediately-consumed producer with
        // no register and no slot: `j = (long)(i - 1)` where `i - 1` was
        // computed straight into %eax). The generic path would emit the
        // accumulator form and then relay the result (`cltq; movq %rax,%r10`
        // / `movl %eax,%eax; movq %rax,%r10`); the extending move can
        // target the destination register directly from %rax's sub-
        // register (`movslq %eax, %r10`): one instruction and no %rax
        // write — %rax keeps holding the SOURCE, so the register cache
        // stays exactly as it was. Bit-identical to the accumulator form:
        // both read the same low `from_ty` bits and produce the same
        // extension. Shapes without a one-instruction form fall through to
        // the generic path (which handles them today).
        if src_phys.is_none() && src_slot.is_none() {
            let Operand::Value(v) = src else { return false };
            if !self.state.reg_cache.acc_has(v.0, self.state.is_alloca(v.0)) {
                return false;
            }
            return self.emit_cast_from_accumulator(kind, to_ty, dest_32, dest_64);
        }
        // An XMM-homed source is an integer bit-punned in the float domain
        // (fp-select/int-select blending keeps the value in the SSE domain).
        // The extending moves below encode GP register names only — route
        // such sources through the accumulator fallback, which performs the
        // xmm→GPR move (same contract as emit_int_cmp_insn_typed).
        if src_phys.is_some_and(super::emit::is_xmm_reg) {
            return false;
        }

        match kind {
            CastKind::Noop | CastKind::UnsignedToSignedSameSize { .. } => {
                // No conversion needed — just move src to dest register.
                self.operand_to_callee_reg(src, dest_phys);
                return true;
            }
            CastKind::IntWiden { from_ty: ft, .. } => {
                // Widen: sign or zero extend from smaller to larger type.
                // (Single definition shared with the PF-15 deferred-widen
                // flush — emit_int_widen_move above.)
                self.emit_int_widen_move(src, ft, dest_phys);
                true
            }
            CastKind::IntNarrow { to_ty: t } => {
                // Narrow: truncate to the target width.
                // For unsigned 32-bit types: movl zero-extends, giving correct U32 semantics.
                // For signed 32-bit types: movslq sign-extends, preserving I32 semantics
                // when the value later flows into 64-bit operations.
                if let Some(src_reg) = src_phys {
                    let src_typed = typed_phys_reg_name(src_reg, t);
                    match t.size() {
                        4 => {
                            if t.is_unsigned() {
                                // U32 narrowing: movl truncates to 32 bits and zero-extends.
                                let src_32 = phys_reg_name_32(src_reg);
                                self.state
                                    .emit_fmt(format_args!("    movl %{}, %{}", src_32, dest_32));
                            } else {
                                // I32 narrowing: movslq truncates to 32 bits and sign-extends
                                // to 64 bits, preserving negative values.
                                let src_32 = phys_reg_name_32(src_reg);
                                self.state
                                    .emit_fmt(format_args!("    movslq %{}, %{}", src_32, dest_64));
                            }
                        }
                        2 => self
                            .state
                            .emit_fmt(format_args!("    movzwl %{}, %{}", src_typed, dest_32)),
                        1 => self
                            .state
                            .emit_fmt(format_args!("    movzbl %{}, %{}", src_typed, dest_32)),
                        _ => {
                            self.operand_to_callee_reg(src, dest_phys);
                        }
                    }
                } else if let Some(slot_off) = src_slot {
                    // For narrowing from stack: just load the narrower size.
                    match t.size() {
                        4 => self
                            .state
                            .out
                            .emit_instr_rbp_reg("    movl", slot_off, dest_32),
                        2 => self
                            .state
                            .out
                            .emit_instr_rbp_reg("    movzwl", slot_off, dest_32),
                        1 => self
                            .state
                            .out
                            .emit_instr_rbp_reg("    movzbl", slot_off, dest_32),
                        _ => self
                            .state
                            .out
                            .emit_instr_rbp_reg("    movq", slot_off, dest_64),
                    }
                } else {
                    return false;
                }
                return true;
            }
            CastKind::SignedToUnsignedSameSize { to_ty: t } => {
                // I32→U32: reinterpret signed as unsigned, zero-extending upper bits.
                // movl truncates to 32 bits and clears upper 32 — correct for unsigned.
                if let Some(src_reg) = src_phys {
                    match t.size() {
                        4 => {
                            let src_32 = phys_reg_name_32(src_reg);
                            self.state
                                .emit_fmt(format_args!("    movl %{}, %{}", src_32, dest_32));
                        }
                        2 => self.state.emit_fmt(format_args!(
                            "    movzwl %{}, %{}",
                            typed_phys_reg_name(src_reg, t),
                            dest_32
                        )),
                        1 => self.state.emit_fmt(format_args!(
                            "    movzbl %{}, %{}",
                            typed_phys_reg_name(src_reg, t),
                            dest_32
                        )),
                        _ => {
                            self.operand_to_callee_reg(src, dest_phys);
                        }
                    }
                } else if let Some(slot_off) = src_slot {
                    match t.size() {
                        4 => self
                            .state
                            .out
                            .emit_instr_rbp_reg("    movl", slot_off, dest_32),
                        2 => self
                            .state
                            .out
                            .emit_instr_rbp_reg("    movzwl", slot_off, dest_32),
                        1 => self
                            .state
                            .out
                            .emit_instr_rbp_reg("    movzbl", slot_off, dest_32),
                        _ => self
                            .state
                            .out
                            .emit_instr_rbp_reg("    movq", slot_off, dest_64),
                    }
                } else {
                    return false;
                }
                return true;
            }
            _ => {
                // Float casts, F128, etc. — not handled here.
                return false;
            }
        }
    }
}
