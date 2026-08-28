//! ArmCodegen: return operations.

use super::emit::{callee_saved_name, ArmCodegen};
use crate::backend::state::SlotAddr;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{Operand, Value};

impl ArmCodegen {
    pub(super) fn emit_return_impl(&mut self, val: Option<&Operand>, frame_size: i64) {
        if let Some(val) = val {
            let ret_ty = self.current_return_type;
            if ret_ty.is_long_double() {
                self.emit_f128_operand_to_q0_full(val);
                self.emit_epilogue_and_ret_impl(frame_size);
                return;
            }
            // _Float128 lowered to the IrType::U128 carrier: AAPCS64 still
            // returns it in Q0. The value is 16 slot bytes; load them whole.
            // (The full loader's f64-extend fallback would be wrong here:
            // U128-carrier loads/stores do not feed the f128 source
            // tracker, so the tracker cannot vouch for the slot contents —
            // but the slot IS the value, 16 bytes, by construction.)
            if self.func_ret_is_f128_sse {
                self.emit_f128_carrier_operand_to_q0(val);
                self.emit_epilogue_and_ret_impl(frame_size);
                return;
            }
        }
        crate::backend::traits::emit_return_default(self, val, frame_size);
    }

    /// Load a _Float128-carrier operand into Q0: direct 16-byte slot load
    /// when the value is slot-homed (the normal carrier case), constants
    /// and tracked values through the full loader.
    pub(super) fn emit_f128_carrier_operand_to_q0(&mut self, op: &Operand) {
        if let Operand::Value(v) = op {
            match self.state.resolve_slot_addr(v.0) {
                Some(SlotAddr::Direct(slot)) => {
                    self.emit_load_from_sp("q0", slot.0, "ldr");
                    return;
                }
                Some(SlotAddr::Reg(reg)) => {
                    self.state
                        .emit_fmt(format_args!("    ldr q0, [{}]", callee_saved_name(reg)));
                    return;
                }
                Some(SlotAddr::OverAligned(slot, id)) => {
                    self.emit_alloca_aligned_addr(slot, id);
                    self.state.emit("    ldr q0, [x9]");
                    return;
                }
                _ => {}
            }
        }
        self.emit_f128_operand_to_q0_full(op);
    }

    pub(super) fn emit_return_i128_to_regs_impl(&mut self) {
        // x0:x1 already hold the i128 return value per AAPCS64 -- noop
    }

    pub(super) fn emit_return_f128_to_reg_impl(&mut self) {
        self.state.emit("    fmov d0, x0");
        self.state.emit("    bl __extenddftf2");
    }

    pub(super) fn emit_return_f32_to_reg_impl(&mut self) {
        self.state.emit("    fmov s0, w0");
    }

    pub(super) fn emit_return_f64_to_reg_impl(&mut self) {
        self.state.emit("    fmov d0, x0");
    }

    pub(super) fn emit_return_int_to_reg_impl(&mut self) {
        // x0 already holds the return value per AAPCS64 -- noop
    }

    pub(super) fn current_return_type_impl(&self) -> IrType {
        self.current_return_type
    }

    /// GNU C nested-function static chain for AArch64.
    ///
    /// GCC uses x18 as the static-chain register on AArch64.  The lowerer
    /// emits GetStaticChain at nested-function entry before ordinary code can
    /// clobber it, and emits SetStaticChain immediately before a direct nested
    /// function call.  Trampolines/non-local goto still fail closed via the
    /// trait defaults; this covers the common direct-call torture cluster.
    pub(super) fn emit_get_static_chain_impl(&mut self, dest: &Value) {
        if let Some(&d_reg) = self.reg_assignments.get(&dest.0) {
            if !super::emit::is_arm_fp_phys(d_reg) {
                let name = super::emit::callee_saved_name(d_reg);
                if name != "x18" {
                    self.state.emit_fmt(format_args!("    mov {}, x18", name));
                }
                self.state.reg_cache.invalidate_all();
                return;
            }
        }
        if self.state.get_slot(dest.0).is_none() {
            return;
        }
        self.state.emit("    mov x0, x18");
        self.store_x0_to(dest);
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_set_static_chain_impl(&mut self, src: &Operand) {
        self.operand_to_x0(src);
        self.state.emit("    mov x18, x0");
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_get_return_f64_second_impl(&mut self, dest: &Value) {
        self.store_float_reg(dest, IrType::F64, "d1");
    }

    pub(super) fn emit_set_return_f64_second_impl(&mut self, src: &Operand) {
        self.float_operand_to_reg(src, IrType::F64, "d1");
    }

    pub(super) fn emit_get_return_f32_second_impl(&mut self, dest: &Value) {
        self.store_float_reg(dest, IrType::F32, "s1");
    }

    pub(super) fn emit_set_return_f32_second_impl(&mut self, src: &Operand) {
        self.float_operand_to_reg(src, IrType::F32, "s1");
    }

    pub(super) fn emit_get_return_f128_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_sp("q1", slot.0, "str");
            self.state.track_f128_self(dest.0);
        }
    }

    pub(super) fn emit_set_return_f128_second_impl(&mut self, src: &Operand) {
        self.emit_f128_operand_to_q0_full(src);
        self.state.emit("    mov v1.16b, v0.16b");
    }
}
