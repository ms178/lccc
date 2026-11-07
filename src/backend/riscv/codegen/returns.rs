//! RiscvCodegen: return value handling.

use super::emit::RiscvCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{IrConst, Operand, Value};

impl RiscvCodegen {
    pub(super) fn emit_return_impl(&mut self, val: Option<&Operand>, frame_size: i64) {
        // Sibling call: the trailing call was suppressed; this `Return` is
        // its tail.  Restore the frame and jump: ra holds our caller's
        // return address again, every argument is in its register, and the
        // callee's result is ours.  The tail line carries the call's read
        // annotation (`# lccc-call-reads`), which the peephole consumes and
        // strips like a call's.
        if let Some(crate::backend::state::SibcallPending::Ready { target, .. }) =
            self.state.pending_sibcall.take()
        {
            self.state.flush_epilogue_loc();
            self.emit_epilogue_impl(frame_size);
            let note = self.sibcall_reads.take().map_or_else(String::new, |mask| {
                format!("{}{}", super::peephole::CALL_READS_TAG, mask)
            });
            match target {
                crate::backend::state::SibcallTarget::Direct(name) => {
                    self.state
                        .emit_fmt(format_args!("    tail {}{}", name, note));
                }
                crate::backend::state::SibcallTarget::Reg(reg) => {
                    self.state.emit_fmt(format_args!("    jr {}{}", reg, note));
                }
                // The x86-64 `jmp *slot(%rip)` form: never armed by this
                // backend (`emit_call_instruction_impl` builds `Direct` or
                // `Reg` only).  Were it ever to reach here, the call it
                // replaced is gone -- trap rather than fall off the end of
                // the function.
                crate::backend::state::SibcallTarget::RipSlot(_) => {
                    self.state.emit("    ebreak");
                }
            }
            return;
        }
        if let Some(val) = val {
            // Flattened float struct: the IR returns the hidden buffer
            // pointer; the psABI wants the members in fa0/fa1/a0.
            if let Some((_, class)) = self.fp_ret_buffer {
                self.emit_return_fp_agg(val, class);
                self.emit_epilogue_and_ret_impl(frame_size);
                return;
            }
            let ret_ty = self.current_return_type;
            if ret_ty == IrType::U16 && self.func_ret_is_f128_sse {
                self.operand_to_t0(val);
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srli t0, t0, 48");
                self.state.emit("    lui t1, 0xffff0");
                self.state.emit("    or t0, t0, t1");
                self.state.emit("    fmv.w.x fa0, t0");
                self.emit_epilogue_and_ret_impl(frame_size);
                return;
            }
            if ret_ty.is_long_double() {
                // Full-precision F128 return: load the full 128-bit value into a0:a1
                self.emit_f128_operand_to_a0_a1(val);
                self.emit_epilogue_and_ret_impl(frame_size);
                return;
            }
            // A full-width integer/pointer value already homed in the ABI
            // return register needs no a0 -> t0 -> a0 relay. Restrict this to
            // types requiring no narrow canonicalization.
            if matches!(
                ret_ty,
                crate::common::types::IrType::I64
                    | crate::common::types::IrType::U64
                    | crate::common::types::IrType::Ptr
            ) && matches!(val, Operand::Value(v) if self.reg_assignments.get(&v.0) == Some(&crate::backend::regalloc::PhysReg(12)))
            {
                self.emit_epilogue_and_ret_impl(frame_size);
                return;
            }
        }
        crate::backend::traits::emit_return_default(self, val, frame_size);
    }

    pub(super) fn emit_return_i128_to_regs_impl(&mut self) {
        self.state.emit("    mv a0, t0");
        self.state.emit("    mv a1, t1");
    }

    pub(super) fn emit_return_f128_to_reg_impl(&mut self) {
        self.state.emit("    fmv.d.x fa0, t0");
        self.state.emit("    call __extenddftf2");
    }

    pub(super) fn emit_return_f32_to_reg_impl(&mut self) {
        self.state.emit("    fmv.w.x fa0, t0");
    }

    pub(super) fn emit_return_f64_to_reg_impl(&mut self) {
        self.state.emit("    fmv.d.x fa0, t0");
    }

    pub(super) fn emit_return_int_to_reg_impl(&mut self) {
        // A return value is passed like a named argument (RISC-V psABI): an
        // `unsigned int` result is sign-extended to 64 bits.  The internal
        // U32 form is zero-extended (see `emit_call_reg_args_impl`), and a
        // conforming caller compares the returned register as is.
        if self.current_return_type == IrType::U32 {
            self.state.emit("    sext.w a0, t0");
        } else {
            self.state.emit("    mv a0, t0");
        }
    }

    pub(super) fn emit_get_return_f64_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_s0("fa1", slot.0, "fsd");
        }
    }

    pub(super) fn emit_set_return_f64_second_impl(&mut self, src: &Operand) {
        match src {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.emit_load_from_s0("fa1", slot.0, "fld");
                }
            }
            Operand::Const(IrConst::F64(f)) => {
                let bits = f.to_bits() as i64;
                self.state.emit_fmt(format_args!("    li t0, {}", bits));
                self.state.emit("    fmv.d.x fa1, t0");
            }
            _ => {
                self.operand_to_t0(src);
                self.state.emit("    fmv.d.x fa1, t0");
            }
        }
    }

    pub(super) fn emit_get_return_f32_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_s0("fa1", slot.0, "fsw");
        }
    }

    pub(super) fn emit_set_return_f32_second_impl(&mut self, src: &Operand) {
        match src {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.emit_load_from_s0("fa1", slot.0, "flw");
                }
            }
            Operand::Const(IrConst::F32(f)) => {
                let bits = f.to_bits() as i64;
                self.state.emit_fmt(format_args!("    li t0, {}", bits));
                self.state.emit("    fmv.w.x fa1, t0");
            }
            _ => {
                self.operand_to_t0(src);
                self.state.emit("    fmv.w.x fa1, t0");
            }
        }
    }
}
