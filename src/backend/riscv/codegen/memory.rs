//! RiscvCodegen: memory operations (load, store, memcpy, GEP, stack).

use super::emit::{callee_saved_name, RiscvCodegen};
use crate::backend::state::{SlotAddr, StackSlot};
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{IrConst, Operand, Value};

impl RiscvCodegen {
    // ---- Store/Load overrides ----

    pub(super) fn emit_store_impl(&mut self, val: &Operand, ptr: &Value, ty: IrType) {
        if ty == IrType::F128 {
            crate::backend::f128_softfloat::f128_emit_store(self, val, ptr);
            return;
        }
        if crate::backend::generation::is_i128_type(ty) {
            // U128-carrier store: when the stored value is a tracked
            // full-precision F128 source (F128Neg/F128Fabs/F128Copysign
            // results consumed by a U128-typed store), the destination slot
            // ends up holding the full 16 bytes — record it so later F128
            // paths read them instead of the f64-extend fallback.
            crate::backend::traits::emit_store_default(self, val, ptr, ty);
            // Tracking is sound only when the destination slot anchors
            // the bytes themselves (Direct alloca/value slot or an
            // over-aligned alloca): an Indirect/Reg home stores THROUGH a
            // pointer, so its own slot does not hold the data.
            let slot_anchored = matches!(
                self.state.resolve_slot_addr(ptr.0),
                Some(SlotAddr::Direct(_) | SlotAddr::OverAligned(_, _))
            );
            let tracked_val = match val {
                Operand::Value(v) => self.state.get_f128_source(v.0).is_some(),
                // A constant stored through a U128-typed store IS the
                // full-precision bit pattern by construction (LongDouble
                // bytes, or the I128 carrier bits): the destination slot is
                // a full-precision F128 source. Without this, a
                // const-initialized local _Float128 forced every later
                // intrinsic/argument staging into the lossy f64-extend
                // fallback (observed: -1.5F128 negated to -0.0 on RISC-V).
                Operand::Const(c) => matches!(c, IrConst::LongDouble(_, _) | IrConst::I128(_)),
                _ => false,
            };
            if tracked_val && slot_anchored {
                self.state.track_f128_self(ptr.0);
            }
            return;
        }
        crate::backend::traits::emit_store_default(self, val, ptr, ty);
    }

    pub(super) fn emit_load_impl(&mut self, dest: &Value, ptr: &Value, ty: IrType) {
        if ty == IrType::F128 {
            crate::backend::f128_softfloat::f128_emit_load(self, dest, ptr);
            return;
        }
        if crate::backend::generation::is_i128_type(ty) {
            // U128-carrier load: the 16 raw bytes land in dest's own slot,
            // so dest is a full-precision F128 source. Without this,
            // F128Neg/F128Fabs/F128Copysign staging on a freshly loaded
            // _Float128 value degraded to the f64-extend fallback and
            // produced -0.0 for -(-1.5F128)-shaped code.
            if let Some(addr) = self.state.resolve_slot_addr(ptr.0) {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr_impl(slot, id);
                        self.emit_load_pair_indirect_impl();
                    }
                    SlotAddr::Direct(slot) => self.emit_load_pair_from_slot_impl(slot),
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot_impl(slot, ptr.0);
                        self.emit_load_pair_indirect_impl();
                    }
                    SlotAddr::Reg(reg) => {
                        self.emit_reg_to_addr(reg);
                        self.emit_load_pair_indirect_impl();
                    }
                }
                self.emit_store_acc_pair_impl(dest);
                self.state.track_f128_self(dest.0);
            }
            return;
        }
        crate::backend::traits::emit_load_default(self, dest, ptr, ty);
    }

    pub(super) fn emit_store_with_const_offset_impl(
        &mut self,
        val: &Operand,
        base: &Value,
        offset: i64,
        ty: IrType,
    ) {
        if ty == IrType::F128 {
            crate::backend::f128_softfloat::f128_emit_store_with_offset(self, val, base, offset);
            return;
        }
        // For non-F128, emit the operand then use the default path
        self.operand_to_t0(val);
        let addr = self.state.resolve_slot_addr(base.0);
        if let Some(addr) = addr {
            let store_instr = Self::store_for_type(ty);
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    self.state.emit("    mv t3, t0");
                    self.emit_alloca_aligned_addr_impl(slot, id);
                    self.emit_add_offset_to_addr_reg_impl(offset);
                    self.state
                        .emit_fmt(format_args!("    {} t3, 0(t5)", store_instr));
                }
                SlotAddr::Direct(slot) => {
                    let folded_slot = StackSlot(slot.0 + offset);
                    self.emit_store_to_s0("t0", folded_slot.0, store_instr);
                }
                SlotAddr::Indirect(slot) => {
                    self.state.emit("    mv t3, t0");
                    self.emit_load_ptr_from_slot_impl(slot, base.0);
                    if offset != 0 {
                        self.emit_add_offset_to_addr_reg_impl(offset);
                    }
                    self.state
                        .emit_fmt(format_args!("    {} t3, 0(t5)", store_instr));
                }
                SlotAddr::Reg(reg) => {
                    self.state.emit("    mv t3, t0");
                    self.emit_reg_to_addr(reg);
                    if offset != 0 {
                        self.emit_add_offset_to_addr_reg_impl(offset);
                    }
                    self.state
                        .emit_fmt(format_args!("    {} t3, 0(t5)", store_instr));
                }
            }
        }
    }

    pub(super) fn emit_load_with_const_offset_impl(
        &mut self,
        dest: &Value,
        base: &Value,
        offset: i64,
        ty: IrType,
    ) {
        if ty == IrType::F128 {
            crate::backend::f128_softfloat::f128_emit_load_with_offset(self, dest, base, offset);
            return;
        }
        // Default path for non-F128
        let addr = self.state.resolve_slot_addr(base.0);
        if let Some(addr) = addr {
            let load_instr = Self::load_for_type(ty);
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    self.emit_alloca_aligned_addr_impl(slot, id);
                    self.emit_add_offset_to_addr_reg_impl(offset);
                    self.state
                        .emit_fmt(format_args!("    {} t0, 0(t5)", load_instr));
                }
                SlotAddr::Direct(slot) => {
                    let folded_slot = StackSlot(slot.0 + offset);
                    self.emit_load_from_s0("t0", folded_slot.0, load_instr);
                }
                SlotAddr::Indirect(slot) => {
                    self.emit_load_ptr_from_slot_impl(slot, base.0);
                    if offset != 0 {
                        self.emit_add_offset_to_addr_reg_impl(offset);
                    }
                    self.state
                        .emit_fmt(format_args!("    {} t0, 0(t5)", load_instr));
                }
                SlotAddr::Reg(reg) => {
                    self.emit_reg_to_addr(reg);
                    if offset != 0 {
                        self.emit_add_offset_to_addr_reg_impl(offset);
                    }
                    self.state
                        .emit_fmt(format_args!("    {} t0, 0(t5)", load_instr));
                }
            }
            self.store_t0_to(dest);
        }
    }

    // ---- Typed store/load to/from slot ----

    pub(super) fn emit_typed_store_to_slot_impl(
        &mut self,
        instr: &'static str,
        _ty: IrType,
        slot: StackSlot,
    ) {
        self.emit_store_to_s0("t0", slot.0, instr);
    }

    pub(super) fn emit_typed_load_from_slot_impl(&mut self, instr: &'static str, slot: StackSlot) {
        self.emit_load_from_s0("t0", slot.0, instr);
    }

    // ---- Pointer/address helpers ----

    pub(super) fn emit_load_ptr_from_slot_impl(&mut self, slot: StackSlot, val_id: u32) {
        // Check register allocation: use callee-saved register if available.
        if let Some(&reg) = self.reg_assignments.get(&val_id) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv t5, {}", reg_name));
        } else {
            self.emit_load_from_s0("t5", slot.0, "ld");
        }
    }

    pub(super) fn emit_add_offset_to_addr_reg_impl(&mut self, offset: i64) {
        if Self::fits_imm12(offset) {
            self.state
                .emit_fmt(format_args!("    addi t5, t5, {}", offset));
        } else {
            self.state.emit_fmt(format_args!("    li t6, {}", offset));
            self.state.emit("    add t5, t5, t6");
        }
    }

    pub(super) fn emit_slot_addr_to_secondary_impl(
        &mut self,
        slot: StackSlot,
        is_alloca: bool,
        val_id: u32,
    ) {
        if is_alloca {
            self.emit_alloca_addr("t1", val_id, slot.0);
        } else if let Some(&reg) = self.reg_assignments.get(&val_id) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv t1, {}", reg_name));
        } else {
            self.emit_load_from_s0("t1", slot.0, "ld");
        }
    }

    // ---- GEP ----

    pub(super) fn emit_gep_direct_const_impl(&mut self, slot: StackSlot, offset: i64) {
        let folded = slot.0 + offset;
        self.emit_addi_s0("t0", folded);
    }

    pub(super) fn emit_gep_indirect_const_impl(
        &mut self,
        slot: StackSlot,
        offset: i64,
        val_id: u32,
    ) {
        if let Some(&reg) = self.reg_assignments.get(&val_id) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
        } else {
            self.emit_load_from_s0("t0", slot.0, "ld");
        }
        if offset != 0 {
            self.emit_add_imm_to_acc_impl(offset);
        }
    }

    pub(super) fn emit_add_imm_to_acc_impl(&mut self, imm: i64) {
        if (-2048..=2047).contains(&imm) {
            self.state
                .emit_fmt(format_args!("    addi t0, t0, {}", imm));
        } else {
            self.state.emit_fmt(format_args!("    li t1, {}", imm));
            self.state.emit("    add t0, t0, t1");
        }
    }

    // ---- Alloca alignment ----

    pub(super) fn emit_alloca_aligned_addr_impl(&mut self, slot: StackSlot, val_id: u32) {
        let align = self
            .state
            .alloca_over_align(val_id)
            .expect("alloca must have over-alignment for aligned addr emission");
        // Compute s0 + slot_offset into t5 (pointer register)
        self.emit_addi_s0("t5", slot.0);
        // Align: t5 = (t5 + align-1) & -align
        self.state
            .emit_fmt(format_args!("    li t6, {}", align - 1));
        self.state.emit("    add t5, t5, t6");
        self.state.emit_fmt(format_args!("    li t6, -{}", align));
        self.state.emit("    and t5, t5, t6");
    }

    pub(super) fn emit_alloca_aligned_addr_to_acc_impl(&mut self, slot: StackSlot, val_id: u32) {
        let align = self
            .state
            .alloca_over_align(val_id)
            .expect("alloca must have over-alignment for aligned addr emission");
        // Compute s0 + slot_offset into t0 (accumulator)
        self.emit_addi_s0("t0", slot.0);
        // Align: t0 = (t0 + align-1) & -align
        self.state
            .emit_fmt(format_args!("    li t6, {}", align - 1));
        self.state.emit("    add t0, t0, t6");
        self.state.emit_fmt(format_args!("    li t6, -{}", align));
        self.state.emit("    and t0, t0, t6");
        // t0 now holds an aligned address, not any previous SSA value.
        self.state.reg_cache.invalidate_acc();
    }

    // ---- Memcpy ----

    pub(super) fn emit_memcpy_load_dest_addr_impl(
        &mut self,
        slot: StackSlot,
        is_alloca: bool,
        val_id: u32,
    ) {
        if is_alloca {
            self.emit_alloca_addr("t1", val_id, slot.0);
        } else if let Some(&reg) = self.reg_assignments.get(&val_id) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv t1, {}", reg_name));
        } else {
            self.emit_load_from_s0("t1", slot.0, "ld");
        }
    }

    pub(super) fn emit_memcpy_load_src_addr_impl(
        &mut self,
        slot: StackSlot,
        is_alloca: bool,
        val_id: u32,
    ) {
        if is_alloca {
            self.emit_alloca_addr("t2", val_id, slot.0);
        } else if let Some(&reg) = self.reg_assignments.get(&val_id) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv t2, {}", reg_name));
        } else {
            self.emit_load_from_s0("t2", slot.0, "ld");
        }
    }

    pub(super) fn emit_memcpy_impl_impl(&mut self, size: usize) {
        let label_id = self.state.next_label_id();
        let loop_label = format!(".Lmemcpy_loop_{}", label_id);
        let done_label = format!(".Lmemcpy_done_{}", label_id);
        self.state.emit_fmt(format_args!("    li t3, {}", size));
        self.state.emit_fmt(format_args!("{}:", loop_label));
        self.state
            .emit_fmt(format_args!("    beqz t3, {}", done_label));
        self.state.emit("    lbu t4, 0(t2)");
        self.state.emit("    sb t4, 0(t1)");
        self.state.emit("    addi t1, t1, 1");
        self.state.emit("    addi t2, t2, 1");
        self.state.emit("    addi t3, t3, -1");
        self.state.emit_fmt(format_args!("    j {}", loop_label));
        self.state.emit_fmt(format_args!("{}:", done_label));
    }

    // ---- DynAlloca helpers ----

    pub(super) fn emit_round_up_acc_to_16_impl(&mut self) {
        self.state.emit("    addi t0, t0, 15");
        self.state.emit("    andi t0, t0, -16");
    }

    pub(super) fn emit_sub_sp_by_acc_impl(&mut self) {
        self.state.emit("    sub sp, sp, t0");
    }

    pub(super) fn emit_mov_sp_to_acc_impl(&mut self) {
        self.state.emit("    mv t0, sp");
    }

    pub(super) fn emit_mov_acc_to_sp_impl(&mut self) {
        self.state.emit("    mv sp, t0");
    }

    pub(super) fn emit_align_acc_impl(&mut self, align: usize) {
        self.state
            .emit_fmt(format_args!("    addi t0, t0, {}", align - 1));
        self.state
            .emit_fmt(format_args!("    andi t0, t0, -{}", align));
    }
}
