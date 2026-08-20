//! I686Codegen: memory operations (load, store, memcpy, GEP, stack).

use crate::ir::reexports::{Operand, Value};
use crate::common::types::IrType;
use crate::backend::state::{StackSlot, SlotAddr};
use crate::backend::regalloc::PhysReg;
use crate::backend::generation::is_i128_type;
use crate::backend::traits::ArchCodegen;
use crate::emit;
use super::emit::{I686Codegen, phys_reg_name};

impl I686Codegen {
    // ---- accumulator-bypass store/load helpers -----------------------------
    //
    // The i686 backend is accumulator-centric: %eax is never handed out by the
    // register allocator, so `load %eax …; movl %eax, %dest` and
    // `movl %src, %eax; store %eax …` round-trips cost an instruction on every
    // memory access.  These helpers emit straight into / out of an allocated
    // GPR when one exists.  Because %eax is never allocatable, none of the
    // direct paths touch the accumulator, so the reg_cache stays valid across
    // them (a direct load into %ebx leaves whatever was in %eax in place).

    /// Sub-register name for a *store source*.  %eax is not allocatable, so
    /// every candidate is a full 32-bit GPR.  In 32-bit mode only %ebx/%ecx/
    /// %edx have encodable 8-bit sub-registers (%sil/%dil/%bpl need REX), and
    /// all six GPRs have 16-bit sub-registers.
    fn store_sub_reg(reg: PhysReg, ty: IrType) -> Option<&'static str> {
        match ty {
            IrType::I8 | IrType::U8 => match reg.0 {
                0 => Some("bl"),
                4 => Some("cl"),
                5 => Some("dl"),
                _ => None,
            },
            IrType::I16 | IrType::U16 => match reg.0 {
                0 => Some("bx"),
                1 => Some("si"),
                2 => Some("di"),
                3 => Some("bp"),
                4 => Some("cx"),
                5 => Some("dx"),
                _ => None,
            },
            _ => Some(phys_reg_name(reg)),
        }
    }

    /// Best-effort direct store source for `val` (immediate or register-resident
    /// value), bypassing the `movl …,%eax; store %eax/…` round-trip. Returns
    /// `None` when the value has no register-resident/imm form that can be
    /// stored directly (allocas, wide values, F64/F128, byte stores from
    /// %esi/%edi/%ebp, ...).
    fn direct_store_src(&self, val: &Operand, ty: IrType) -> Option<String> {
        match val {
            Operand::Const(c) => {
                let imm: i64 = match c {
                    crate::ir::reexports::IrConst::I8(v) => *v as i64,
                    crate::ir::reexports::IrConst::I16(v) => *v as i64,
                    crate::ir::reexports::IrConst::I32(v) => *v as i64,
                    crate::ir::reexports::IrConst::I64(v)
                        if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 =>
                    {
                        *v
                    }
                    crate::ir::reexports::IrConst::Zero => 0,
                    _ => return None,
                };
                // Mask to the stored width so the assembler sees a legal
                // imm8/imm16 operand (the byte/word store would drop the high
                // bits anyway).
                let imm = match ty {
                    IrType::I8 | IrType::U8 => imm & 0xff,
                    IrType::I16 | IrType::U16 => imm & 0xffff,
                    _ => imm,
                };
                Some(format!("${}", imm))
            }
            Operand::Value(v) => {
                if self.state.is_alloca(v.0)
                    || self.state.wide_values.contains(&v.0)
                    || self.state.f128_direct_slots.contains(&v.0)
                {
                    return None;
                }
                let phys = self.reg_assignments.get(&v.0).copied()?;
                let sub = Self::store_sub_reg(phys, ty)?;
                Some(format!("%{}", sub))
            }
        }
    }

    /// Emit a scalar integer/pointer load straight into the destination's
    /// allocated register. Returns false for unsupported shapes (no dest
    /// register, i128 payloads) so the caller can fall back to the
    /// accumulator path.
    fn try_emit_load_direct(&mut self, dest: &Value, ptr: &Value, ty: IrType) -> bool {
        if is_i128_type(ty) {
            return false;
        }
        let Some(phys) = self.dest_reg(dest) else { return false };
        let d = phys_reg_name(phys);
        let load_instr = self.load_instr_for_type(ty);
        let Some(addr) = self.state.resolve_slot_addr(ptr.0) else { return false };
        match addr {
            SlotAddr::Direct(slot) => {
                let sr = self.slot_ref(slot);
                emit!(self.state, "    {} {}, %{}", load_instr, sr, d);
            }
            SlotAddr::Indirect(slot) => {
                // Pointer register-resident: dereference it DIRECTLY. The
                // %ecx staging below is only needed for slot-resident
                // pointers; routing a register pointer through %ecx costs an
                // extra mov AND clobbers %ecx (which the register allocator's
                // load-hazard refinement relies on NOT happening — see
                // Phase 2d in regalloc.rs).
                if let Some(phys) = self.reg_assignments.get(&ptr.0).copied() {
                    let p = phys_reg_name(phys);
                    emit!(self.state, "    {} (%{}), %{}", load_instr, p, d);
                } else {
                    self.emit_load_ptr_from_slot(slot, ptr.0);
                    emit!(self.state, "    {} (%ecx), %{}", load_instr, d);
                }
            }
            SlotAddr::OverAligned(slot, id) => {
                self.emit_alloca_aligned_addr(slot, id);
                emit!(self.state, "    {} (%ecx), %{}", load_instr, d);
            }
        }
        true
    }

    /// Emit a store directly from an immediate or register-resident value,
    /// bypassing the `movl …,%eax; store %eax/…` round-trip. `%ecx` is the
    /// indirect-address scratch, so a value held in %ecx only qualifies for
    /// Direct-slot stores. Returns false to fall back to the accumulator path.
    fn try_emit_store_direct(&mut self, val: &Operand, ptr: &Value, addr: SlotAddr, ty: IrType) -> bool {
        let store_instr = self.store_instr_for_type(ty);
        let Some(src) = self.direct_store_src(val, ty) else { return false };
        if src == "%ecx" || src == "%cl" || src == "%cx" {
            if !matches!(addr, SlotAddr::Direct(_)) {
                return false;
            }
        }
        match addr {
            SlotAddr::Direct(slot) => {
                let sr = self.slot_ref(slot);
                emit!(self.state, "    {} {}, {}", store_instr, src, sr);
            }
            SlotAddr::Indirect(slot) => {
                self.emit_load_ptr_from_slot(slot, ptr.0);
                emit!(self.state, "    {} {}, (%ecx)", store_instr, src);
            }
            SlotAddr::OverAligned(slot, id) => {
                self.emit_alloca_aligned_addr(slot, id);
                emit!(self.state, "    {} {}, (%ecx)", store_instr, src);
            }
        }
        true
    }

    // ---- Store/Load overrides ----

    pub(super) fn emit_store_impl(&mut self, val: &Operand, ptr: &Value, ty: IrType) {
        if ty == IrType::F128 {
            self.emit_f128_load_to_x87(val);
            let addr = self.state.resolve_slot_addr(ptr.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr(slot, id);
                        self.state.emit("    fstpt (%ecx)");
                    }
                    SlotAddr::Direct(slot) => {
                        let sr = self.slot_ref(slot);
                        emit!(self.state, "    fstpt {}", sr);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot(slot, ptr.0);
                        self.state.emit("    fstpt (%ecx)");
                    }
                }
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        if ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64 {
            let addr = self.state.resolve_slot_addr(ptr.0);
            self.emit_load_acc_pair(val);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.state.emit("    pushl %edx");
                        self.esp_adjust += 4;
                        self.state.emit("    pushl %eax");
                        self.esp_adjust += 4;
                        self.emit_alloca_aligned_addr(slot, id);
                        self.state.emit("    popl %eax");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %eax, (%ecx)");
                        self.state.emit("    popl %edx");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %edx, 4(%ecx)");
                    }
                    SlotAddr::Direct(slot) => {
                        let sr0 = self.slot_ref(slot);
                        let sr4 = self.slot_ref_offset(slot, 4);
                        emit!(self.state, "    movl %eax, {}", sr0);
                        emit!(self.state, "    movl %edx, {}", sr4);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.state.emit("    pushl %edx");
                        self.esp_adjust += 4;
                        self.state.emit("    pushl %eax");
                        self.esp_adjust += 4;
                        self.emit_load_ptr_from_slot(slot, ptr.0);
                        self.state.emit("    popl %eax");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %eax, (%ecx)");
                        self.state.emit("    popl %edx");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %edx, 4(%ecx)");
                    }
                }
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        if let Some(addr) = self.state.resolve_slot_addr(ptr.0) {
            if self.try_emit_store_direct(val, ptr, addr, ty) {
                return;
            }
        }
        crate::backend::traits::emit_store_default(self, val, ptr, ty);
    }

    pub(super) fn emit_load_impl(&mut self, dest: &Value, ptr: &Value, ty: IrType) {
        if ty == IrType::F128 {
            let addr = self.state.resolve_slot_addr(ptr.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr(slot, id);
                        self.state.emit("    fldt (%ecx)");
                    }
                    SlotAddr::Direct(slot) => {
                        let sr = self.slot_ref(slot);
                        emit!(self.state, "    fldt {}", sr);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot(slot, ptr.0);
                        self.state.emit("    fldt (%ecx)");
                    }
                }
                if let Some(dest_slot) = self.state.get_slot(dest.0) {
                    let sr = self.slot_ref(dest_slot);
                    emit!(self.state, "    fstpt {}", sr);
                    self.state.f128_direct_slots.insert(dest.0);
                }
            }
            return;
        }
        if ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64 {
            let addr = self.state.resolve_slot_addr(ptr.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr(slot, id);
                        self.state.emit("    movl (%ecx), %eax");
                        self.state.emit("    movl 4(%ecx), %edx");
                    }
                    SlotAddr::Direct(slot) => {
                        let sr0 = self.slot_ref(slot);
                        let sr4 = self.slot_ref_offset(slot, 4);
                        emit!(self.state, "    movl {}, %eax", sr0);
                        emit!(self.state, "    movl {}, %edx", sr4);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot(slot, ptr.0);
                        self.state.emit("    movl (%ecx), %eax");
                        self.state.emit("    movl 4(%ecx), %edx");
                    }
                }
                self.emit_store_acc_pair(dest);
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        if self.try_emit_load_direct(dest, ptr, ty) {
            return;
        }
        crate::backend::traits::emit_load_default(self, dest, ptr, ty);
    }

    pub(super) fn emit_store_with_const_offset_impl(&mut self, val: &Operand, base: &Value, offset: i64, ty: IrType) {
        if ty == IrType::F128 {
            self.emit_f128_load_to_x87(val);
            let addr = self.state.resolve_slot_addr(base.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr(slot, id);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    fstpt (%ecx)");
                    }
                    SlotAddr::Direct(slot) => {
                        let folded_slot = StackSlot(slot.0 + offset);
                        let sr = self.slot_ref(folded_slot);
                        emit!(self.state, "    fstpt {}", sr);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot(slot, base.0);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    fstpt (%ecx)");
                    }
                }
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        // Register-resident 64-bit-pair base store: stage the value into the
        // %eax:%edx accumulator pair, then `movl %eax,off(%base);
        // movl %edx,off+4(%base)`.  Must precede the slot-based pair branch,
        // which silently emits nothing for a slot-less base.  The base is
        // callee-saved by the fold predicate, so the staging cannot clobber
        // it.
        if (ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64)
            && !self.state.is_alloca(base.0)
        {
            if let Some(&phys) = self.reg_assignments.get(&base.0) {
                if matches!(phys.0, 0 | 1 | 2 | 3) {
                    let base_name = phys_reg_name(phys);
                    let mem0 = if offset != 0 {
                        format!("{}(%{})", offset, base_name)
                    } else {
                        format!("(%{})", base_name)
                    };
                    let mem4 = format!("{}(%{})", offset + 4, base_name);
                    self.emit_load_acc_pair(val);
                    emit!(self.state, "    movl %eax, {}", mem0);
                    emit!(self.state, "    movl %edx, {}", mem4);
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
            }
        }
        if ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64 {
            self.emit_load_acc_pair(val);
            let addr = self.state.resolve_slot_addr(base.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.state.emit("    pushl %edx");
                        self.esp_adjust += 4;
                        self.state.emit("    pushl %eax");
                        self.esp_adjust += 4;
                        self.emit_alloca_aligned_addr(slot, id);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    popl %eax");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %eax, (%ecx)");
                        self.state.emit("    popl %edx");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %edx, 4(%ecx)");
                    }
                    SlotAddr::Direct(slot) => {
                        let folded_slot = StackSlot(slot.0 + offset);
                        let sr0 = self.slot_ref(folded_slot);
                        let sr4 = self.slot_ref_offset(folded_slot, 4);
                        emit!(self.state, "    movl %eax, {}", sr0);
                        emit!(self.state, "    movl %edx, {}", sr4);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.state.emit("    pushl %edx");
                        self.esp_adjust += 4;
                        self.state.emit("    pushl %eax");
                        self.esp_adjust += 4;
                        self.emit_load_ptr_from_slot(slot, base.0);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    popl %eax");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %eax, (%ecx)");
                        self.state.emit("    popl %edx");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %edx, 4(%ecx)");
                    }
                }
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        // Register-resident base: single-instruction store to offset(%reg).
        // Mirrors the load path (GCC: `movl %eax, 40(%ebx)`); the value is
        // staged in %eax first — the fold predicate keeps the base out of
        // %eax/%edx/%ecx (callee-saved homes only), so staging never
        // destroys the address.
        if let Some(&phys) = self.reg_assignments.get(&base.0) {
            if !self.state.is_alloca(base.0) && matches!(phys.0, 0 | 1 | 2 | 3) {
                let store_instr = self.store_instr_for_type(ty);
                let base_name = phys_reg_name(phys);
                let dst = if offset != 0 {
                    format!("{}(%{})", offset, base_name)
                } else {
                    format!("(%{})", base_name)
                };
                // Accumulator-bypass: an immediate or register-resident value
                // stores in one instruction (`movl $imm, 40(%ebx)` /
                // `movl %esi, 40(%ebx)`) with %eax untouched.
                if let Some(src) = self.direct_store_src(val, ty) {
                    emit!(self.state, "    {} {}, {}", store_instr, src, dst);
                    return;
                }
                self.operand_to_eax(val);
                let src = self.eax_for_type(ty);
                emit!(self.state, "    {} {}, {}", store_instr, src, dst);
                return;
            }
        }
        // Delegate to default for other types
        let addr = self.state.resolve_slot_addr(base.0);
        if let Some(addr) = addr {
            let store_instr = self.store_instr_for_type(ty);
            // Accumulator-bypass: store an immediate / register-resident value
            // in one instruction. Indirect forms use %ecx as the address
            // scratch, so a value held in %ecx only qualifies for Direct slots.
            if let Some(src) = self.direct_store_src(val, ty) {
                let src_is_ecx = src == "%ecx" || src == "%cl" || src == "%cx";
                match addr {
                    SlotAddr::Direct(slot) => {
                        let folded_slot = StackSlot(slot.0 + offset);
                        let sr = self.slot_ref(folded_slot);
                        emit!(self.state, "    {} {}, {}", store_instr, src, sr);
                        return;
                    }
                    SlotAddr::Indirect(slot) if !src_is_ecx => {
                        self.emit_load_ptr_from_slot(slot, base.0);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        emit!(self.state, "    {} {}, (%ecx)", store_instr, src);
                        return;
                    }
                    SlotAddr::OverAligned(slot, id) if !src_is_ecx => {
                        self.emit_alloca_aligned_addr(slot, id);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        emit!(self.state, "    {} {}, (%ecx)", store_instr, src);
                        return;
                    }
                    _ => {}
                }
            }
            // Fallback: accumulator path (value staged in %eax, saved to %edx
            // when the address scratch %ecx must be computed afterwards).
            self.operand_to_eax(val);
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    self.emit_save_acc();
                    self.emit_alloca_aligned_addr(slot, id);
                    self.emit_add_offset_to_addr_reg(offset);
                    self.emit_typed_store_indirect(store_instr, ty);
                }
                SlotAddr::Direct(slot) => {
                    let folded_slot = StackSlot(slot.0 + offset);
                    self.emit_typed_store_to_slot(store_instr, ty, folded_slot);
                }
                SlotAddr::Indirect(slot) => {
                    self.emit_save_acc();
                    self.emit_load_ptr_from_slot(slot, base.0);
                    if offset != 0 {
                        self.emit_add_offset_to_addr_reg(offset);
                    }
                    self.emit_typed_store_indirect(store_instr, ty);
                }
            }
        }
    }

    pub(super) fn emit_load_with_const_offset_impl(&mut self, dest: &Value, base: &Value, offset: i64, ty: IrType) {
        if ty == IrType::F128 {
            let addr = self.state.resolve_slot_addr(base.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr(slot, id);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    fldt (%ecx)");
                    }
                    SlotAddr::Direct(slot) => {
                        let folded_slot = StackSlot(slot.0 + offset);
                        let sr = self.slot_ref(folded_slot);
                        emit!(self.state, "    fldt {}", sr);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot(slot, base.0);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    fldt (%ecx)");
                    }
                }
                if let Some(dest_slot) = self.state.get_slot(dest.0) {
                    let sr = self.slot_ref(dest_slot);
                    emit!(self.state, "    fstpt {}", sr);
                    self.state.f128_direct_slots.insert(dest.0);
                }
            }
            return;
        }
        // Register-resident 64-bit-pair base load: `movl off(%base),%eax;
        // movl off+4(%base),%edx`.  Must precede the slot-based pair branch,
        // which silently emits nothing for a slot-less base.  The fold
        // predicate (const_offset_fold_reg_base_ok) guarantees the base sits
        // in a callee-saved register, never the %eax:%edx accumulator pair.
        if (ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64)
            && !self.state.is_alloca(base.0)
        {
            if let Some(&phys) = self.reg_assignments.get(&base.0) {
                if matches!(phys.0, 0 | 1 | 2 | 3) {
                    let base_name = phys_reg_name(phys);
                    let mem0 = if offset != 0 {
                        format!("{}(%{})", offset, base_name)
                    } else {
                        format!("(%{})", base_name)
                    };
                    let mem4 = format!("{}(%{})", offset + 4, base_name);
                    emit!(self.state, "    movl {}, %eax", mem0);
                    emit!(self.state, "    movl {}, %edx", mem4);
                    self.emit_store_acc_pair(dest);
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
            }
        }
        if ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64 {
            let addr = self.state.resolve_slot_addr(base.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr(slot, id);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    movl (%ecx), %eax");
                        self.state.emit("    movl 4(%ecx), %edx");
                    }
                    SlotAddr::Direct(slot) => {
                        let folded_slot = StackSlot(slot.0 + offset);
                        let sr0 = self.slot_ref(folded_slot);
                        let sr4 = self.slot_ref_offset(folded_slot, 4);
                        emit!(self.state, "    movl {}, %eax", sr0);
                        emit!(self.state, "    movl {}, %edx", sr4);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot(slot, base.0);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    movl (%ecx), %eax");
                        self.state.emit("    movl 4(%ecx), %edx");
                    }
                }
                self.emit_store_acc_pair(dest);
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        // Register-resident base: single-instruction offset(%reg) load.
        // This is the whole point of the GEP fold — GCC emits
        // `movl 40(%ebx), %eax`; the old path re-materialized the address
        // (movl %ebx,%ecx; addl $40,%ecx; movl (%ecx),%eax) every time.
        // The fold predicate (const_offset_fold_reg_base_ok) restricts the
        // base to callee-saved homes (%ebx/%esi/%edi/%ebp): never the
        // %eax/%edx accumulator staging registers and never the %ecx address
        // scratch, and folded-base liveness links keep it live to here.
        if let Some(&phys) = self.reg_assignments.get(&base.0) {
            if !self.state.is_alloca(base.0) && matches!(phys.0, 0 | 1 | 2 | 3) {
                let load_instr = self.load_instr_for_type(ty);
                let base_name = phys_reg_name(phys);
                let mem = if offset != 0 {
                    format!("{}(%{})", offset, base_name)
                } else {
                    format!("(%{})", base_name)
                };
                // Accumulator-bypass: load straight into the destination's
                // register when one exists (i128 payloads need the pair path).
                if !is_i128_type(ty) {
                    if let Some(dphys) = self.dest_reg(dest) {
                        let d = phys_reg_name(dphys);
                        emit!(self.state, "    {} {}, %{}", load_instr, mem, d);
                        return;
                    }
                }
                emit!(self.state, "    {} {}, %eax", load_instr, mem);
                self.state.reg_cache.invalidate_acc();
                self.emit_store_result(dest);
                return;
            }
        }
        // Delegate to default for other types
        let addr = self.state.resolve_slot_addr(base.0);
        if let Some(addr) = addr {
            let load_instr = self.load_instr_for_type(ty);
            // Accumulator-bypass: load straight into the destination's register
            // (i128 payloads need the 8-byte pair path).
            let direct_dest = if is_i128_type(ty) {
                None
            } else {
                self.dest_reg(dest).map(phys_reg_name)
            };
            if let Some(d) = direct_dest {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr(slot, id);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        emit!(self.state, "    {} (%ecx), %{}", load_instr, d);
                    }
                    SlotAddr::Direct(slot) => {
                        let folded_slot = StackSlot(slot.0 + offset);
                        let sr = self.slot_ref(folded_slot);
                        emit!(self.state, "    {} {}, %{}", load_instr, sr, d);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot(slot, base.0);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        emit!(self.state, "    {} (%ecx), %{}", load_instr, d);
                    }
                }
                return;
            }
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    self.emit_alloca_aligned_addr(slot, id);
                    self.emit_add_offset_to_addr_reg(offset);
                    self.emit_typed_load_indirect(load_instr);
                }
                SlotAddr::Direct(slot) => {
                    let folded_slot = StackSlot(slot.0 + offset);
                    self.emit_typed_load_from_slot(load_instr, folded_slot);
                }
                SlotAddr::Indirect(slot) => {
                    self.emit_load_ptr_from_slot(slot, base.0);
                    if offset != 0 {
                        self.emit_add_offset_to_addr_reg(offset);
                    }
                    self.emit_typed_load_indirect(load_instr);
                }
            }
            self.emit_store_result(dest);
        }
    }

    // ---- Typed store/load helpers ----

    pub(super) fn emit_typed_store_to_slot_impl(&mut self, instr: &'static str, ty: IrType, slot: StackSlot) {
        let reg = self.eax_for_type(ty);
        let sr = self.slot_ref(slot);
        emit!(self.state, "    {} {}, {}", instr, reg, sr);
    }

    pub(super) fn emit_typed_load_from_slot_impl(&mut self, instr: &'static str, slot: StackSlot) {
        let sr = self.slot_ref(slot);
        emit!(self.state, "    {} {}, %eax", instr, sr);
    }

    pub(super) fn emit_load_ptr_from_slot_impl(&mut self, slot: StackSlot, val_id: u32) {
        if let Some(phys) = self.reg_assignments.get(&val_id).copied() {
            let reg = phys_reg_name(phys);
            emit!(self.state, "    movl %{}, %ecx", reg);
        } else {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movl {}, %ecx", sr);
        }
    }

    pub(super) fn emit_typed_store_indirect_impl(&mut self, instr: &'static str, ty: IrType) {
        // Store from the ACCUMULATOR (%eax), not %edx.
        //
        // Every caller of this hook loads the value into %eax last:
        //   * emit_store_default (traits.rs) loads the pointer into %ecx
        //     first and then the value into %eax -- there is NO save_acc on
        //     that path, so storing %edx wrote an uninitialized register.
        //     `g = v;` compiled to `movl %edx, (%ecx)` with %edx never set.
        //   * our own emit_store_with_const_offset override calls
        //     emit_save_acc() and then only touches %ecx (slot load /
        //     lea+add+and), so %eax still holds the value there too.
        // The ARM backend fixed the identical bug (store from x0, not x1);
        // x86-64 always stored from %rax.
        let reg = match ty {
            IrType::I8 | IrType::U8 => "%al",
            IrType::I16 | IrType::U16 => "%ax",
            _ => "%eax",
        };
        emit!(self.state, "    {} {}, (%ecx)", instr, reg);
    }

    pub(super) fn emit_typed_load_indirect_impl(&mut self, instr: &'static str) {
        emit!(self.state, "    {} (%ecx), %eax", instr);
    }

    pub(super) fn emit_add_offset_to_addr_reg_impl(&mut self, offset: i64) {
        if offset != 0 {
            emit!(self.state, "    addl ${}, %ecx", offset as i32);
        }
    }

    // ---- GEP primitives ----

    /// Compute the address of an alloca into `reg`, handling over-aligned allocas.
    pub(super) fn emit_alloca_addr_to(&mut self, reg: &str, val_id: u32, slot: StackSlot) {
        let sr = self.slot_ref(slot);
        if let Some(align) = self.state.alloca_over_align(val_id) {
            emit!(self.state, "    leal {}, %{}", sr, reg);
            emit!(self.state, "    addl ${}, %{}", align - 1, reg);
            emit!(self.state, "    andl ${}, %{}", -(align as i32), reg);
        } else {
            emit!(self.state, "    leal {}, %{}", sr, reg);
        }
    }

    pub(super) fn emit_slot_addr_to_secondary_impl(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32) {
        if is_alloca {
            self.emit_alloca_addr_to("ecx", val_id, slot);
        } else if let Some(phys) = self.reg_assignments.get(&val_id).copied() {
            let reg = phys_reg_name(phys);
            emit!(self.state, "    movl %{}, %ecx", reg);
        } else {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movl {}, %ecx", sr);
        }
    }

    pub(super) fn emit_gep_direct_const_impl(&mut self, slot: StackSlot, offset: i64) {
        let folded_slot = StackSlot(slot.0 + offset);
        let sr = self.slot_ref(folded_slot);
        emit!(self.state, "    leal {}, %eax", sr);
    }

    pub(super) fn emit_gep_indirect_const_impl(&mut self, slot: StackSlot, offset: i64, val_id: u32) {
        if let Some(phys) = self.reg_assignments.get(&val_id).copied() {
            let reg = phys_reg_name(phys);
            if offset == 0 {
                emit!(self.state, "    movl %{}, %eax", reg);
            } else {
                emit!(self.state, "    leal {}(%{}), %eax", offset, reg);
            }
        } else {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movl {}, %eax", sr);
            if offset != 0 {
                emit!(self.state, "    addl ${}, %eax", offset as i32);
            }
        }
    }

    // ---- Dynamic alloca ----

    pub(super) fn emit_add_imm_to_acc_impl(&mut self, imm: i64) {
        emit!(self.state, "    addl ${}, %eax", imm as i32);
    }

    pub(super) fn emit_round_up_acc_to_16_impl(&mut self) {
        self.state.emit("    addl $15, %eax");
        self.state.emit("    andl $-16, %eax");
    }

    pub(super) fn emit_sub_sp_by_acc_impl(&mut self) {
        self.state.emit("    subl %eax, %esp");
    }

    pub(super) fn emit_mov_sp_to_acc_impl(&mut self) {
        self.state.emit("    movl %esp, %eax");
        self.state.reg_cache.invalidate_acc();
    }

    pub(super) fn emit_mov_acc_to_sp_impl(&mut self) {
        self.state.emit("    movl %eax, %esp");
    }

    pub(super) fn emit_align_acc_impl(&mut self, align: usize) {
        emit!(self.state, "    addl ${}, %eax", align - 1);
        emit!(self.state, "    andl ${}, %eax", -(align as i32));
    }

    // ---- Alloca aligned addr ----

    pub(super) fn emit_alloca_aligned_addr_impl(&mut self, slot: StackSlot, val_id: u32) {
        let align = self.state.alloca_over_align(val_id)
            .expect("alloca must have over-alignment for aligned addr emission");
        let sr = self.slot_ref(slot);
        emit!(self.state, "    leal {}, %ecx", sr);
        emit!(self.state, "    addl ${}, %ecx", align - 1);
        emit!(self.state, "    andl ${}, %ecx", -(align as i32));
    }

    pub(super) fn emit_alloca_aligned_addr_to_acc_impl(&mut self, slot: StackSlot, val_id: u32) {
        let align = self.state.alloca_over_align(val_id)
            .expect("alloca must have over-alignment for aligned addr emission");
        let sr = self.slot_ref(slot);
        emit!(self.state, "    leal {}, %eax", sr);
        emit!(self.state, "    addl ${}, %eax", align - 1);
        emit!(self.state, "    andl ${}, %eax", -(align as i32));
        self.state.reg_cache.invalidate_acc();
    }

    // ---- Memcpy ----

    pub(super) fn emit_memcpy_load_dest_addr_impl(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32) {
        if is_alloca {
            self.emit_alloca_addr_to("edi", val_id, slot);
        } else if let Some(phys) = self.reg_assignments.get(&val_id).copied() {
            let reg = phys_reg_name(phys);
            emit!(self.state, "    movl %{}, %edi", reg);
        } else {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movl {}, %edi", sr);
        }
    }

    pub(super) fn emit_memcpy_load_src_addr_impl(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32) {
        if is_alloca {
            self.emit_alloca_addr_to("esi", val_id, slot);
        } else if let Some(phys) = self.reg_assignments.get(&val_id).copied() {
            let reg = phys_reg_name(phys);
            emit!(self.state, "    movl %{}, %esi", reg);
        } else {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movl {}, %esi", sr);
        }
    }

    pub(super) fn emit_memcpy_impl_impl(&mut self, size: usize) {
        emit!(self.state, "    movl ${}, %ecx", size);
        self.state.emit("    rep movsb");
    }
}
