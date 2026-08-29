//! I686Codegen: function call operations (cdecl calling convention).

use super::emit::I686Codegen;
use crate::backend::call_abi;
use crate::backend::generation::is_i128_type;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{Operand, Value};

impl I686Codegen {
    pub(super) fn call_abi_config_impl(&self) -> call_abi::CallAbiConfig {
        call_abi::CallAbiConfig {
            max_int_regs: self.regparm as usize,
            max_float_regs: 0,
            align_i128_pairs: false,
            f128_in_fp_regs: false,
            f128_in_gp_pairs: false,
            variadic_floats_in_gp: false,
            large_struct_by_ref: false,
            use_sysv_struct_classification: false,
            use_riscv_float_struct_classification: false,
            allow_struct_split_reg_stack: false,
            align_struct_pairs: false,
            sret_uses_dedicated_reg: false,
            gcc_regparm_mode: true,
            // GCC i386 cdecl never aligns stack args beyond the 4-byte slot
            // granularity (GCC 14.2 -m32 oracle: 3-int prefix + aligned(32)
            // struct -> arg offset 12, callee reads 4-granular via %ebp).
            // The caller packs 4-granular and va_arg walks 4-granular, so the
            // callee layout must agree — see CallAbiConfig::stack_arg_align_cap.
            stack_arg_align_cap: 4,
        }
    }

    pub(super) fn emit_call_compute_stack_space_impl(
        &self,
        arg_classes: &[call_abi::CallArgClass],
        arg_types: &[IrType],
        _struct_arg_aligns: &[Option<usize>],
    ) -> usize {
        let mut total = 0;
        for (i, ac) in arg_classes.iter().enumerate() {
            let ty = if i < arg_types.len() {
                arg_types[i]
            } else {
                IrType::I32
            };
            match ac {
                call_abi::CallArgClass::Stack => match ty {
                    IrType::F64 | IrType::I64 | IrType::U64 => total += 8,
                    _ => total += 4,
                },
                call_abi::CallArgClass::F128Stack => total += 12,
                call_abi::CallArgClass::I128Stack => total += 16,
                call_abi::CallArgClass::StructByValStack { size } => total += (*size + 3) & !3,
                call_abi::CallArgClass::LargeStructStack { size } => total += (*size + 3) & !3,
                call_abi::CallArgClass::ZeroSizeSkip => {}
                call_abi::CallArgClass::IntReg { .. } => {} // regparm: in register, no stack space
                call_abi::CallArgClass::I64RegPair { .. } => {} // regparm: register pair
                call_abi::CallArgClass::StructByValReg { .. } => {} // regparm: struct in registers
                _ => total += 4,
            }
        }
        (total + 15) & !15
    }

    pub(super) fn emit_call_f128_pre_convert_impl(
        &mut self,
        _args: &[Operand],
        _arg_classes: &[call_abi::CallArgClass],
        _arg_types: &[IrType],
        _stack_arg_space: usize,
    ) -> usize {
        0 // No F128 pre-conversion needed on i686
    }

    pub(super) fn emit_call_stack_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[call_abi::CallArgClass],
        arg_types: &[IrType],
        stack_arg_space: usize,
        _fptr_spill: usize,
        _f128_temp_space: usize,
        _struct_arg_aligns: &[Option<usize>],
    ) -> i64 {
        if stack_arg_space > 0 {
            emit!(self.state, "    subl ${}, %esp", stack_arg_space);
            self.esp_adjust += stack_arg_space as i64;
        }

        let mut stack_offset: usize = 0;
        for (i, ac) in arg_classes.iter().enumerate() {
            match ac {
                call_abi::CallArgClass::I128Stack => {
                    self.emit_call_i128_stack_arg(&args[i], stack_offset);
                    stack_offset += 16;
                }
                call_abi::CallArgClass::F128Stack => {
                    self.emit_call_f128_stack_arg(&args[i], stack_offset);
                    stack_offset += 12;
                }
                call_abi::CallArgClass::StructByValStack { size }
                | call_abi::CallArgClass::LargeStructStack { size } => {
                    let sz = *size;
                    self.emit_call_struct_stack_arg(&args[i], stack_offset, sz);
                    stack_offset += (sz + 3) & !3;
                }
                call_abi::CallArgClass::Stack => {
                    let ty = arg_types[i];
                    if ty == IrType::F64 || ty == IrType::I64 || ty == IrType::U64 {
                        self.emit_call_8byte_stack_arg(&args[i], ty, stack_offset);
                        stack_offset += 8;
                    } else {
                        self.operand_to_eax(&args[i]);
                        emit!(self.state, "    movl %eax, {}(%esp)", stack_offset);
                        stack_offset += 4;
                    }
                }
                call_abi::CallArgClass::ZeroSizeSkip => {}
                call_abi::CallArgClass::IntReg { .. } => {} // regparm: handled in emit_call_reg_args
                call_abi::CallArgClass::I64RegPair { .. } => {} // regparm: register pair
                call_abi::CallArgClass::StructByValReg { .. } => {} // regparm: struct in registers
                _ => {
                    self.operand_to_eax(&args[i]);
                    emit!(self.state, "    movl %eax, {}(%esp)", stack_offset);
                    stack_offset += 4;
                }
            }
        }

        stack_arg_space as i64
    }

    pub(super) fn emit_call_reg_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[call_abi::CallArgClass],
        _arg_types: &[IrType],
        _total_sp_adjust: i64,
        _f128_temp_space: usize,
        _stack_arg_space: usize,
        _struct_arg_riscv_float_classes: &[Option<crate::common::types::RiscvFloatClass>],
    ) {
        if self.regparm == 0 {
            return; // cdecl: no register args
        }
        // regparm register order: EAX (reg_idx 0), EDX (reg_idx 1), ECX (reg_idx 2).
        //
        // Args are emitted in DESCENDING base-register order so that %eax —
        // the staging register for operand_to_eax and the pointer register
        // for register-struct copies — is written by the very last move.
        // Register targets are disjoint across args, so descending order
        // guarantees no arg's staging clobbers an already-loaded target.
        let regparm_regs: &[&str] = &["%eax", "%edx", "%ecx"];
        let mut items: Vec<(usize, usize)> = Vec::new(); // (base_reg_idx, arg_idx)
        for (i, ac) in arg_classes.iter().enumerate() {
            match ac {
                call_abi::CallArgClass::IntReg { reg_idx } if *reg_idx < 3 => {
                    items.push((*reg_idx, i))
                }
                call_abi::CallArgClass::I64RegPair { base_reg_idx }
                    if *base_reg_idx + 1 < 3 + 1 =>
                {
                    items.push((*base_reg_idx, i))
                }
                call_abi::CallArgClass::StructByValReg { base_reg_idx, .. } => {
                    items.push((*base_reg_idx, i))
                }
                _ => {}
            }
        }
        items.sort_by(|a, b| b.0.cmp(&a.0));
        for &(base, arg_i) in &items {
            match &arg_classes[arg_i] {
                call_abi::CallArgClass::IntReg { reg_idx } => {
                    let dest_reg = regparm_regs[*reg_idx];
                    self.operand_to_eax(&args[arg_i]);
                    if dest_reg != "%eax" {
                        emit!(self.state, "    movl %eax, {}", dest_reg);
                        self.state.reg_cache.invalidate_acc();
                    }
                }
                call_abi::CallArgClass::I64RegPair { base_reg_idx } => {
                    let lo = regparm_regs[*base_reg_idx];
                    let hi = regparm_regs[*base_reg_idx + 1];
                    match &args[arg_i] {
                        Operand::Const(c) => {
                            let v = c.to_i64().unwrap_or(0);
                            emit!(self.state, "    movl ${}, {}", (v & 0xFFFF_FFFF) as i32, lo);
                            emit!(
                                self.state,
                                "    movl ${}, {}",
                                ((v as u64) >> 32) as i32,
                                hi
                            );
                        }
                        Operand::Value(v) => {
                            if let Some(slot) = self.state.get_slot(v.0) {
                                // Direct mem->reg loads: no staging, order-safe.
                                let sr0 = self.slot_ref(slot);
                                let sr4 = self.slot_ref_offset(slot, 4);
                                emit!(self.state, "    movl {}, {}", sr4, hi);
                                emit!(self.state, "    movl {}, {}", sr0, lo);
                            } else {
                                // Wide values are always slotted on i686, but stay
                                // defensive: acc-pair produces eax(lo):edx(hi).
                                self.emit_load_acc_pair(&args[arg_i]);
                                if *base_reg_idx == 1 {
                                    self.state.emit("    movl %edx, %ecx");
                                    self.state.emit("    movl %eax, %edx");
                                }
                            }
                        }
                    }
                    self.state.reg_cache.invalidate_acc();
                }
                call_abi::CallArgClass::StructByValReg { base_reg_idx, size } => {
                    let words = size.div_ceil(4);
                    if let Operand::Value(v) = &args[arg_i] {
                        if self.state.is_alloca(v.0) {
                            if let Some(slot) = self.state.get_slot(v.0) {
                                // Direct slot-word -> reg loads (full-word reads:
                                // slots are 4-byte rounded, GCC reads word_mode
                                // chunks here too).
                                for k in (0..words).rev() {
                                    let sr = self.slot_ref_offset(slot, (k * 4) as i64);
                                    emit!(
                                        self.state,
                                        "    movl {}, {}",
                                        sr,
                                        regparm_regs[base_reg_idx + k]
                                    );
                                }
                            }
                        } else {
                            // Pointer to struct data: pointer staged in %eax;
                            // descending word order means an %eax target (only
                            // possible at word 0 when base==0) is written last.
                            self.operand_to_eax(&args[arg_i]);
                            for k in (0..words).rev() {
                                emit!(
                                    self.state,
                                    "    movl {}(%eax), {}",
                                    k * 4,
                                    regparm_regs[base_reg_idx + k]
                                );
                            }
                        }
                    }
                    self.state.reg_cache.invalidate_acc();
                }
                _ => {}
            }
        }
    }

    pub(super) fn emit_call_instruction_impl(
        &mut self,
        direct_name: Option<&str>,
        func_ptr: Option<&Operand>,
        indirect: bool,
        _stack_arg_space: usize,
    ) {
        // x87 registers are caller-saved: drop any cached st(0) copy first.
        self.flush_x87_pending_copy();
        if let Some(name) = direct_name {
            if self.state.needs_plt(name) {
                emit!(self.state, "    call {}@PLT", name);
            } else {
                emit!(self.state, "    call {}", name);
            }
        } else if indirect {
            if let Some(fptr) = func_ptr {
                if self.regparm > 0 {
                    // Under regparm, %eax/%edx/%ecx carry arguments — staging
                    // the target in %eax would destroy arg 0. Call through
                    // the value's home directly (register or stack slot);
                    // i686 values always have one or the other.
                    if let Operand::Value(v) = fptr {
                        if let Some(&phys) = self.reg_assignments.get(&v.0) {
                            emit!(
                                self.state,
                                "    call *%{}",
                                super::emit::phys_reg_name(phys)
                            );
                            return;
                        }
                        if let Some(slot) = self.state.get_slot(v.0) {
                            let sr = self.slot_ref(slot);
                            emit!(self.state, "    call *{}", sr);
                            return;
                        }
                    }
                    // Constant target (rare): safe only because no register
                    // argument exists if we got here without a home — still,
                    // prefer an absolute call over clobbering %eax.
                    if let Operand::Const(c) = fptr {
                        if let Some(v) = c.to_i64() {
                            emit!(self.state, "    call *${}", v);
                            return;
                        }
                    }
                }
                self.operand_to_eax(fptr);
            }
            self.state.emit("    call *%eax");
        }
    }

    pub(super) fn emit_call_cleanup_impl(
        &mut self,
        stack_arg_space: usize,
        _f128_temp_space: usize,
        _indirect: bool,
    ) {
        if stack_arg_space > 0 {
            emit!(self.state, "    addl ${}, %esp", stack_arg_space);
            self.esp_adjust -= stack_arg_space as i64;
        }
    }

    pub(super) fn emit_call_store_result_impl(&mut self, dest: &Value, return_type: IrType) {
        if return_type == IrType::Void {
            return;
        }
        if return_type == IrType::I64 || return_type == IrType::U64 {
            if let Some(slot) = self.state.get_slot(dest.0) {
                let sr0 = self.slot_ref(slot);
                let sr4 = self.slot_ref_offset(slot, 4);
                emit!(self.state, "    movl %eax, {}", sr0);
                emit!(self.state, "    movl %edx, {}", sr4);
            }
            self.state.reg_cache.invalidate_acc();
        } else if is_i128_type(return_type) {
            self.emit_call_store_i128_result(dest);
        } else if return_type.is_long_double() {
            self.emit_call_store_f128_result(dest);
        } else if return_type == IrType::F32 {
            self.emit_call_move_f32_to_acc();
            self.emit_store_result(dest);
        } else if return_type == IrType::F64 {
            self.emit_f64_store_from_x87(dest);
            self.state.reg_cache.invalidate_acc();
        } else {
            self.emit_store_result(dest);
        }
    }

    pub(super) fn emit_call_store_i128_result_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr0 = self.slot_ref(slot);
            let sr4 = self.slot_ref_offset(slot, 4);
            emit!(self.state, "    movl %eax, {}", sr0);
            emit!(self.state, "    movl %edx, {}", sr4);
        }
    }

    pub(super) fn emit_call_store_f128_result_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    fstpt {}", sr);
            self.state.f128_direct_slots.insert(dest.0);
        }
    }

    pub(super) fn emit_call_move_f32_to_acc_impl(&mut self) {
        self.state.emit("    subl $4, %esp");
        self.state.emit("    fstps (%esp)");
        self.state.emit("    movl (%esp), %eax");
        self.state.emit("    addl $4, %esp");
    }

    pub(super) fn emit_call_move_f64_to_acc_impl(&mut self) {
        self.state.emit("    subl $8, %esp");
        self.state.emit("    fstpl (%esp)");
        self.state.emit("    movl (%esp), %eax");
        self.state.emit("    addl $8, %esp");
    }
}
