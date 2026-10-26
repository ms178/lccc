//! I686Codegen: return value operations.

use super::emit::I686Codegen;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{IrConst, Operand, Value};

impl I686Codegen {
    pub(super) fn emit_return_impl(&mut self, val: Option<&Operand>, frame_size: i64) {
        // The callee's FP stack must be empty at `ret`; drop any cached
        // st(0) copy before the return sequence is emitted.
        self.flush_x87_pending_copy();
        if let Some(val) = val {
            let ret_ty = self.current_return_type;
            if self.vector_ret_buffer_bytes > 0 {
                self.operand_to_eax(val);
                let register = match self.vector_ret_buffer_bytes {
                    32 => "ymm0",
                    64 => "zmm0",
                    _ => unreachable!("invalid wide vector return width"),
                };
                emit!(self.state, "    vmovups (%eax), %{}", register);
                self.emit_epilogue_and_ret(frame_size);
                return;
            }
            if ret_ty == IrType::U16 && self.current_return_half_xmm {
                self.operand_to_eax(val);
                self.state.emit("    movzwl %ax, %eax");
                self.state.emit("    movd %eax, %xmm0");
                self.emit_epilogue_and_ret(frame_size);
                return;
            }
            if ret_ty == IrType::I128 && self.current_return_i686_xmm {
                match val {
                    Operand::Value(value) => {
                        let slot = self
                            .state
                            .get_slot(value.0)
                            .expect("i686 XMM vector return must be stack-backed");
                        let sr = self.slot_ref(slot);
                        emit!(self.state, "    movups {}, %xmm0", sr);
                    }
                    Operand::Const(IrConst::Zero) => self.state.emit("    xorps %xmm0, %xmm0"),
                    Operand::Const(IrConst::I128(value)) => {
                        self.state.emit("    subl $16, %esp");
                        for word in 0..4 {
                            emit!(
                                self.state,
                                "    movl ${}, {}(%esp)",
                                (*value >> (32 * word)) as u32 as i32,
                                4 * word
                            );
                        }
                        self.state.emit("    movups (%esp), %xmm0");
                        self.state.emit("    addl $16, %esp");
                    }
                    _ => panic!("unsupported i686 XMM vector return operand: {val:?}"),
                }
                self.emit_epilogue_and_ret(frame_size);
                return;
            }
            if ret_ty == IrType::I128 && self.current_return_i686_ext_vector_gp3 {
                match val {
                    Operand::Value(value) => {
                        let slot = self
                            .state
                            .get_slot(value.0)
                            .expect("i686 three-word ExtVector return must be stack-backed");
                        let sr0 = self.slot_ref(slot);
                        let sr4 = self.slot_ref_offset(slot, 4);
                        let sr8 = self.slot_ref_offset(slot, 8);
                        emit!(self.state, "    movl {}, %eax", sr0);
                        emit!(self.state, "    movl {}, %edx", sr4);
                        emit!(self.state, "    movl {}, %ecx", sr8);
                    }
                    Operand::Const(IrConst::I128(value)) => {
                        emit!(self.state, "    movl ${}, %eax", *value as u32 as i32);
                        emit!(
                            self.state,
                            "    movl ${}, %edx",
                            (*value >> 32) as u32 as i32
                        );
                        emit!(
                            self.state,
                            "    movl ${}, %ecx",
                            (*value >> 64) as u32 as i32
                        );
                    }
                    Operand::Const(IrConst::Zero) => {
                        self.state.emit("    xorl %eax, %eax");
                        self.state.emit("    xorl %edx, %edx");
                        self.state.emit("    xorl %ecx, %ecx");
                    }
                    _ => panic!("unsupported i686 three-word ExtVector return operand: {val:?}"),
                }
                self.emit_epilogue_and_ret(frame_size);
                return;
            }
            if ret_ty == IrType::I64 || ret_ty == IrType::U64 || ret_ty == IrType::D64 {
                self.emit_load_acc_pair(val);
                self.emit_epilogue_and_ret(frame_size);
                return;
            }
            if ret_ty == IrType::F64 {
                self.emit_f64_load_to_x87(val);
                self.emit_epilogue_and_ret(frame_size);
                return;
            }
            if ret_ty.is_long_double() {
                self.emit_f128_load_to_x87(val);
                self.emit_epilogue_and_ret(frame_size);
                return;
            }
        }
        // Delegate all other cases (I128, F32, scalar int) to default
        crate::backend::traits::emit_return_default(self, val, frame_size);
    }

    pub(super) fn emit_return_i128_to_regs_impl(&mut self) {
        // eax:edx already holds the low 64 bits
    }

    pub(super) fn emit_return_f128_to_reg_impl(&mut self) {
        self.state.emit("    pushl %eax");
        self.state.emit("    fildl (%esp)");
        self.state.emit("    addl $4, %esp");
    }

    pub(super) fn emit_return_f32_to_reg_impl(&mut self) {
        self.state.emit("    pushl %eax");
        self.state.emit("    flds (%esp)");
        self.state.emit("    addl $4, %esp");
    }

    pub(super) fn emit_return_f64_to_reg_impl(&mut self) {
        self.state.emit("    pushl %edx");
        self.state.emit("    pushl %eax");
        self.state.emit("    fldl (%esp)");
        self.state.emit("    addl $8, %esp");
    }

    pub(super) fn emit_return_int_to_reg_impl(&mut self) {
        // eax already holds the return value
    }

    pub(super) fn emit_get_return_f64_second_impl(&mut self, dest: &Value) {
        self.store_eax_to(dest);
    }

    pub(super) fn emit_set_return_f64_second_impl(&mut self, src: &Operand) {
        self.operand_to_eax(src);
    }

    pub(super) fn emit_get_return_f32_second_impl(&mut self, dest: &Value) {
        self.store_eax_to(dest);
    }

    pub(super) fn emit_set_return_f32_second_impl(&mut self, src: &Operand) {
        self.operand_to_eax(src);
    }

    pub(super) fn emit_get_return_f128_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    fstpt {}", sr);
            self.state.f128_direct_slots.insert(dest.0);
        }
    }

    pub(super) fn emit_set_return_f128_second_impl(&mut self, src: &Operand) {
        self.emit_f128_load_to_x87(src);
    }
}
