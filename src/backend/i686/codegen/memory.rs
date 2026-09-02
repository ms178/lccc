//! I686Codegen: memory operations (load, store, memcpy, GEP, stack).

use super::emit::{phys_reg_name, I686Codegen};
use crate::backend::generation::is_i128_type;
use crate::backend::regalloc::PhysReg;
use crate::ir::reexports::IrConst;
use crate::backend::state::{SlotAddr, StackSlot};
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{Operand, Value};

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
    pub(super) fn direct_store_src(&self, val: &Operand, ty: IrType) -> Option<String> {
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
        let Some(phys) = self.dest_reg(dest) else {
            return false;
        };
        let d = phys_reg_name(phys);
        let load_instr = self.load_instr_for_type(ty);
        let Some(addr) = self.state.resolve_slot_addr(ptr.0) else {
            return false;
        };
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
            SlotAddr::Reg(reg) => emit!(
                self.state,
                "    {} (%{}), %{}",
                load_instr,
                phys_reg_name(reg),
                d
            ),
        }
        true
    }

    /// Emit a store directly from an immediate or register-resident value,
    /// bypassing the `movl …,%eax; store %eax/…` round-trip. `%ecx` is the
    /// indirect-address scratch, so a value held in %ecx only qualifies for
    /// Direct-slot stores. Returns false to fall back to the accumulator path.
    fn try_emit_store_direct(
        &mut self,
        val: &Operand,
        ptr: &Value,
        addr: SlotAddr,
        ty: IrType,
    ) -> bool {
        let store_instr = self.store_instr_for_type(ty);
        let Some(src) = self.direct_store_src(val, ty) else {
            return false;
        };
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
            SlotAddr::Reg(reg) => emit!(
                self.state,
                "    {} {}, (%{})",
                store_instr,
                src,
                phys_reg_name(reg)
            ),
        }
        true
    }

    // ---- Store/Load overrides ----

    pub(super) fn emit_store_impl(&mut self, val: &Operand, ptr: &Value, ty: IrType) {
        // 128-bit integer container stores (IrType::I128/U128: the _Float128
        // gp-view, _Decimal128, vector pack results).  The historical
        // "i128 == 64-bit eax:edx pair" model truncated these to 8 bytes,
        // corrupting every _Float128 initialization (f128_softfloat: the
        // 16-byte slot's upper half stayed garbage and fed __addtf3).
        // Full 16-byte semantics; staging is caller-saved-only (%eax value
        // shuttle, %edx src address, %ecx dst address) and never touches
        // %esi/%edi (allocator-owned homes).
        if is_i128_type(ty) {
            let words: Option<[u32; 4]> = match val {
                Operand::Const(IrConst::I128(v)) => {
                    let b = v.to_le_bytes();
                    Some([
                        u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                        u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
                        u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
                        u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
                    ])
                }
                Operand::Const(IrConst::Zero) => Some([0; 4]),
                _ => None,
            };
            match (words, self.state.resolve_slot_addr(ptr.0)) {
                // Constant: four immediate stores through the resolved dest.
                (Some(words), addr) => {
                    match addr {
                        Some(SlotAddr::Direct(slot)) => {
                            for (k, w) in words.iter().enumerate() {
                                let dsr = self.slot_ref_offset(slot, (4 * k) as i64);
                                emit!(self.state, "    movl ${}, {}", w, dsr);
                            }
                        }
                        _ => {
                            self.emit_i128_dest_addr(ptr);
                            for (k, w) in words.iter().enumerate() {
                                if k == 0 {
                                    emit!(self.state, "    movl ${}, (%ecx)", w);
                                } else {
                                    emit!(self.state, "    movl ${}, {}(%ecx)", w, 4 * k);
                                }
                            }
                        }
                    }
                    self.state.reg_cache.invalidate_acc();
                    self.state.reg_cache.invalidate_sec();
                    return;
                }
                // Value copy: both slots direct => eax shuttle, no address regs.
                (None, Some(SlotAddr::Direct(dslot))) => {
                    if let Operand::Value(sv) = val {
                        if let Some(SlotAddr::Direct(sslot)) =
                            self.state.resolve_slot_addr(sv.0)
                        {
                            for k in 0..4i64 {
                                let ssr = self.slot_ref_offset(sslot, 4 * k);
                                let dsr = self.slot_ref_offset(dslot, 4 * k);
                                emit!(self.state, "    movl {}, %eax", ssr);
                                emit!(self.state, "    movl %eax, {}", dsr);
                            }
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                    }
                    // Fall through to the address-based copy below.
                    self.emit_i128_dest_addr(ptr);
                    self.emit_i128_src_addr(val);
                    for k in 0..4 {
                        if k == 0 {
                            self.state.emit("    movl (%edx), %eax");
                            self.state.emit("    movl %eax, (%ecx)");
                        } else {
                            emit!(self.state, "    movl {}(%edx), %eax", 4 * k);
                            emit!(self.state, "    movl %eax, {}(%ecx)", 4 * k);
                        }
                    }
                    self.state.reg_cache.invalidate_acc();
                    self.state.reg_cache.invalidate_sec();
                    return;
                }
                // Non-direct dest: build both addresses in %edx/%ecx.
                (None, _) => {
                    self.emit_i128_dest_addr(ptr);
                    self.emit_i128_src_addr(val);
                    for k in 0..4 {
                        if k == 0 {
                            self.state.emit("    movl (%edx), %eax");
                            self.state.emit("    movl %eax, (%ecx)");
                        } else {
                            emit!(self.state, "    movl {}(%edx), %eax", 4 * k);
                            emit!(self.state, "    movl %eax, {}(%ecx)", 4 * k);
                        }
                    }
                    self.state.reg_cache.invalidate_acc();
                    self.state.reg_cache.invalidate_sec();
                    return;
                }
            }
        }
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
                    SlotAddr::Reg(reg) => emit!(self.state, "    fstpt (%{})", phys_reg_name(reg)),
                }
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        if ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64 || ty == IrType::D64 {
            let mut addr = self.state.resolve_slot_addr(ptr.0);
            // Base homed in %eax/%edx dies when the VALUE pair is staged
            // into the accumulator below. Stash such a base into the %ecx
            // address scratch first and store through it (same hazard as
            // the pair-load ordering fix; store side cannot reorder because
            // both halves of the value occupy %eax:%edx).
            if let Some(SlotAddr::Reg(reg)) = addr {
                let r = phys_reg_name(reg);
                if r == "eax" || r == "edx" {
                    emit!(self.state, "    movl %{}, %ecx", r);
                    self.state.reg_cache.invalidate_sec();
                    addr = Some(SlotAddr::Reg(crate::backend::regalloc::PhysReg(4)));
                }
            }
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
                    SlotAddr::Reg(reg) => {
                        let r = phys_reg_name(reg);
                        emit!(self.state, "    movl %eax, (%{})", r);
                        emit!(self.state, "    movl %edx, 4(%{})", r);
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
        // 128-bit integer container load (IrType::I128/U128: _Float128
        // gp-view, _Decimal128): full 16-byte semantics — the accumulator
        // fallthrough only ever materialized the low 32 bits.
        if is_i128_type(ty) {
            if let (Some(dslot), Some(SlotAddr::Direct(sslot))) = (
                self.state.get_slot(dest.0),
                self.state.resolve_slot_addr(ptr.0),
            ) {
                for k in 0..4i64 {
                    let ssr = self.slot_ref_offset(sslot, 4 * k);
                    let dsr = self.slot_ref_offset(dslot, 4 * k);
                    emit!(self.state, "    movl {}, %eax", ssr);
                    emit!(self.state, "    movl %eax, {}", dsr);
                }
                self.state.reg_cache.invalidate_acc();
                return;
            }
            // Non-direct source: build its address in %edx, dest slot direct.
            if let Some(dslot) = self.state.get_slot(dest.0) {
                self.emit_i128_src_addr(&Operand::Value(*ptr));
                for k in 0..4 {
                    if k == 0 {
                        self.state.emit("    movl (%edx), %eax");
                    } else {
                        emit!(self.state, "    movl {}(%edx), %eax", 4 * k);
                    }
                    let dsr = self.slot_ref_offset(dslot, 4 * k as i64);
                    emit!(self.state, "    movl %eax, {}", dsr);
                }
                self.state.reg_cache.invalidate_acc();
                self.state.reg_cache.invalidate_sec();
                return;
            }
            // Slot-less dest: fall through to the legacy path (the dest of a
            // U128 load always has a 16-byte slot in the i686 frame model).
        }
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
                    SlotAddr::Reg(reg) => emit!(self.state, "    fldt (%{})", phys_reg_name(reg)),
                }
                if let Some(dest_slot) = self.state.get_slot(dest.0) {
                    let sr = self.slot_ref(dest_slot);
                    emit!(self.state, "    fstpt {}", sr);
                    self.state.f128_direct_slots.insert(dest.0);
                }
            }
            return;
        }
        if ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64 || ty == IrType::D64 {
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
                    SlotAddr::Reg(reg) => {
                        // The base may live in %eax (PhysReg 6, Phase 2e
                        // accumulator home) or %edx (PhysReg 5): loading the
                        // word that OVERWRITES the base register first
                        // destroys the address for the second load (nbody
                        // -m32 SIGSEGV: `movl (%eax),%eax; movl 4(%eax),%edx`
                        // dereferenced the low WORD as a pointer). Order the
                        // pair so the base register is written LAST.
                        let r = phys_reg_name(reg);
                        if r == "eax" {
                            emit!(self.state, "    movl 4(%{}), %edx", r);
                            emit!(self.state, "    movl (%{}), %eax", r);
                        } else {
                            emit!(self.state, "    movl (%{}), %eax", r);
                            emit!(self.state, "    movl 4(%{}), %edx", r);
                        }
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

    pub(super) fn emit_store_with_const_offset_impl(
        &mut self,
        val: &Operand,
        base: &Value,
        offset: i64,
        ty: IrType,
    ) {
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
                    SlotAddr::Reg(reg) => {
                        let r = phys_reg_name(reg);
                        if offset != 0 {
                            emit!(self.state, "    fstpt {}(%{})", offset, r);
                        } else {
                            emit!(self.state, "    fstpt (%{})", r);
                        }
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
            let mut addr = self.state.resolve_slot_addr(base.0);
            // Same %eax/%edx-base hazard as the plain pair store: the value
            // staging below overwrites both accumulator halves, so stash
            // such a base into %ecx first.
            if let Some(SlotAddr::Reg(reg)) = addr {
                let r = phys_reg_name(reg);
                if r == "eax" || r == "edx" {
                    emit!(self.state, "    movl %{}, %ecx", r);
                    self.state.reg_cache.invalidate_sec();
                    addr = Some(SlotAddr::Reg(crate::backend::regalloc::PhysReg(4)));
                }
            }
            self.emit_load_acc_pair(val);
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
                    SlotAddr::Reg(reg) => {
                        let r = phys_reg_name(reg);
                        if offset != 0 {
                            emit!(self.state, "    movl %eax, {}(%{})", offset, r);
                            emit!(self.state, "    movl %edx, {}(%{})", offset + 4, r);
                        } else {
                            emit!(self.state, "    movl %eax, (%{})", r);
                            emit!(self.state, "    movl %edx, 4(%{})", r);
                        }
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
                SlotAddr::Reg(reg) => {
                    let r = phys_reg_name(reg);
                    let src = self.eax_for_type(ty);
                    if offset != 0 {
                        emit!(
                            self.state,
                            "    {} {}, {}(%{})",
                            store_instr,
                            src,
                            offset,
                            r
                        );
                    } else {
                        emit!(self.state, "    {} {}, (%{})", store_instr, src, r);
                    }
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
                    SlotAddr::Reg(reg) => {
                        let r = phys_reg_name(reg);
                        if offset != 0 {
                            emit!(self.state, "    fldt {}(%{})", offset, r);
                        } else {
                            emit!(self.state, "    fldt (%{})", r);
                        }
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
                    SlotAddr::Reg(reg) => {
                        // Same base-clobber hazard as the plain pair load
                        // above: write the base register (possibly %eax or
                        // %edx) last.
                        let r = phys_reg_name(reg);
                        let (m0, m4) = if offset != 0 {
                            (
                                format!("{}(%{})", offset, r),
                                format!("{}(%{})", offset + 4, r),
                            )
                        } else {
                            (format!("(%{})", r), format!("4(%{})", r))
                        };
                        if r == "eax" {
                            emit!(self.state, "    movl {}, %edx", m4);
                            emit!(self.state, "    movl {}, %eax", m0);
                        } else {
                            emit!(self.state, "    movl {}, %eax", m0);
                            emit!(self.state, "    movl {}, %edx", m4);
                        }
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
                    SlotAddr::Reg(reg) => {
                        let r = phys_reg_name(reg);
                        if offset != 0 {
                            emit!(self.state, "    {} {}(%{}), %{}", load_instr, offset, r, d);
                        } else {
                            emit!(self.state, "    {} (%{}), %{}", load_instr, r, d);
                        }
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
                SlotAddr::Reg(reg) => {
                    self.emit_reg_to_addr(reg);
                    if offset != 0 {
                        self.emit_add_offset_to_addr_reg(offset);
                    }
                    self.emit_typed_load_indirect(load_instr);
                }
            }
            self.emit_store_result(dest);
        }
    }

    // ---- SIB indexed addressing (session 27): mem(,%idx,scale) ----
    //
    // Soundness contract: both the base and the index registers are consumed
    // at the Load/Store position — the prologue wires
    // collect_folded_gep_links_all into register allocation, so both live
    // intervals extend to the access.  Every path below either emits a
    // single instruction (no staging → any base/index registers are safe) or
    // stages the value through %eax first, in which case base and index must
    // sit in callee-saved GPRs (staging never touches ebx/esi/edi/ebp).

    /// Types addressable by a single SIB load/store (no pair/x87 handling).
    pub(super) fn sib_scalar_ty(ty: IrType) -> bool {
        matches!(
            ty,
            IrType::I8
                | IrType::U8
                | IrType::I16
                | IrType::U16
                | IrType::I32
                | IrType::U32
                | IrType::Ptr
        )
    }

    fn sib_mem(base_reg: &str, index_reg: &str, shift: u8, disp: i64) -> String {
        let d = if disp == 0 {
            String::new()
        } else {
            format!("{}", disp)
        };
        if shift == 0 {
            format!("{}(%{}, %{})", d, base_reg, index_reg)
        } else {
            format!("{}(%{}, %{}, {})", d, base_reg, index_reg, 1u32 << shift)
        }
    }

    fn sib_mem_sym(sym: &str, index_reg: &str, shift: u8, disp: i64) -> String {
        // Same AT&T rule as x86-64: `sym+disp`, never `sym` concatenated
        // with a bare decimal (that would invent a different symbol).
        let head = if disp == 0 {
            sym.to_string()
        } else if disp > 0 {
            format!("{}+{}", sym, disp)
        } else {
            format!("{}{}", sym, disp)
        };
        if shift == 0 {
            format!("{}(, %{})", head, index_reg)
        } else {
            format!("{}(, %{}, {})", head, index_reg, 1u32 << shift)
        }
    }

    /// Frame-SIB memory operand for an indexed access whose base is a plain
    /// Direct alloca slot: `disp(SLOT, %idx, scale)`.  This is the i686 port
    /// of x86-64's `frame_sib_for_alloca_base`, and it closes a
    /// can-fold/emitter disagreement: `can_indexed_addr_fold` accepts
    /// alloca-Direct bases (the frame slot re-computes at every access),
    /// but the i686 emitters only implemented the register-base form.  With
    /// the fold "guaranteed", the dead-offset-producer walk skipped the
    /// offset chain (`idx_dead_producers`) and the emitter's rematerialise
    /// fallback then reloaded the offset from a slot no producer ever wrote
    /// — gcc.c-torture/execute 20080122-1 read `0(%esp)` as the `2*i+1`
    /// offset, then stored through the garbage address (PR 34628 shape).
    ///
    /// Anchors at %esp (omit_frame_pointer — same accounting as `slot_ref`:
    /// `slot.0 + frame_base_offset + esp_adjust`) or %ebp.  The combined
    /// displacement is ALWAYS printed, even when zero: mod=00 with base=%ebp
    /// would relocate the base to disp32, and an esp-based SIB with no
    /// displacement is the index-only encoding — an explicit displacement
    /// keeps both encodings unambiguous.
    fn frame_sib_for_alloca_base(
        &self,
        base: &Value,
        index: &Value,
        shift: u8,
        disp: i64,
    ) -> Option<String> {
        let idx = self.reg_assignments.get(&index.0).copied()?;
        // GPR index only (PhysReg 0..=6 are the i686 GPRs; %eax=6 is a legal
        // index for the single-instruction forms, which read all inputs
        // before writing any output).
        if idx.0 > 6 {
            return None;
        }
        let Some(SlotAddr::Direct(slot)) = self.state.resolve_slot_addr(base.0) else {
            return None;
        };
        let (frame, off) = if self.omit_frame_pointer {
            ("esp", slot.0 + self.frame_base_offset + self.esp_adjust + disp)
        } else {
            ("ebp", slot.0 + disp)
        };
        // Frame slots and PF-06 displacements are i32 by construction; keep
        // the guard so a pathological frame cannot emit an unencodable
        // disp32 operand (silently wrong would be worse than refusing).
        if off < i32::MIN as i64 || off > i32::MAX as i64 {
            return None;
        }
        let idx_name = phys_reg_name(idx);
        if shift == 0 {
            Some(format!("{}(%{}, %{})", off, frame, idx_name))
        } else {
            Some(format!(
                "{}(%{}, %{}, {})",
                off,
                frame,
                idx_name,
                1u32 << shift
            ))
        }
    }

    pub(super) fn emit_load_indexed_impl(
        &mut self,
        dest: &Value,
        base: &Value,
        index: &Value,
        shift: u8,
        disp: i64,
        ty: IrType,
    ) -> bool {
        if !Self::sib_scalar_ty(ty) || shift > 3 {
            return false;
        }
        let Some(&x) = self.reg_assignments.get(&index.0) else {
            return false;
        };
        let mem = match self.reg_assignments.get(&base.0) {
            Some(&b) => Self::sib_mem(phys_reg_name(b), phys_reg_name(x), shift, disp),
            // Alloca-base indexed addressing: `disp(%esp/%ebp,%idx,scale)`.
            // Must mirror can_indexed_addr_fold's Direct-slot arm — a
            // refusal here after the fold was deemed guaranteed would make
            // rematerialise_skipped_indexed read a skipped producer.
            None => {
                let Some(m) = self.frame_sib_for_alloca_base(base, index, shift, disp) else {
                    return false;
                };
                m
            }
        };
        let load_instr = self.load_instr_for_type(ty);
        // Single instruction — no staging, so dest may alias base/index
        // (x86 computes the address before writing the destination).
        if let Some(&d) = self.reg_assignments.get(&dest.0) {
            emit!(
                self.state,
                "    {} {}, %{}",
                load_instr,
                mem,
                phys_reg_name(d)
            );
            self.state.reg_cache.invalidate_acc();
            return true;
        }
        emit!(self.state, "    {} {}, %eax", load_instr, mem);
        self.state.reg_cache.invalidate_acc();
        self.emit_store_result(dest);
        true
    }

    pub(super) fn emit_store_indexed_impl(
        &mut self,
        val: &Operand,
        base: &Value,
        index: &Value,
        shift: u8,
        disp: i64,
        ty: IrType,
    ) -> bool {
        if !Self::sib_scalar_ty(ty) || shift > 3 {
            return false;
        }
        let Some(&x) = self.reg_assignments.get(&index.0) else {
            return false;
        };
        // Frame-anchored base: the SIB anchors at %esp/%ebp, which value
        // staging through %eax can never touch.
        let (mem, frame_base, b) = match self.reg_assignments.get(&base.0) {
            Some(&b) => (
                Self::sib_mem(phys_reg_name(b), phys_reg_name(x), shift, disp),
                false,
                b,
            ),
            None => {
                let Some(m) = self.frame_sib_for_alloca_base(base, index, shift, disp) else {
                    return false;
                };
                (m, true, PhysReg(0))
            }
        };
        let store_instr = self.store_instr_for_type(ty);
        // Immediate or register-resident value: single instruction, no
        // staging.  A register source equal to the index register can only
        // be the SAME value (`a[i] = i`) — the single instruction reads it
        // for the address and the data consistently.
        if let Some(src) = self.direct_store_src(val, ty) {
            emit!(self.state, "    {} {}, {}", store_instr, src, mem);
            return true;
        }
        // Value must be staged through %eax: require every REGISTER member
        // of the address to sit in a callee-saved GPR (PhysReg 0..=3 =
        // ebx/esi/edi/ebp), which the staging paths never touch.  A
        // frame-anchored base is immune by construction, so only the index
        // register needs to qualify there.
        if (!frame_base && !matches!(b.0, 0 | 1 | 2 | 3)) || !matches!(x.0, 0 | 1 | 2 | 3) {
            return false;
        }
        self.operand_to_eax(val);
        let src = self.eax_for_type(ty);
        emit!(self.state, "    {} {}, {}", store_instr, src, mem);
        self.state.reg_cache.invalidate_acc();
        true
    }

    pub(super) fn emit_load_indexed_sym_impl(
        &mut self,
        dest: &Value,
        sym: &str,
        index: &Value,
        shift: u8,
        disp: i64,
        ty: IrType,
    ) -> bool {
        if self.state.pic_mode || !Self::sib_scalar_ty(ty) || shift > 3 {
            return false;
        }
        let Some(&x) = self.reg_assignments.get(&index.0) else {
            return false;
        };
        let mem = Self::sib_mem_sym(sym, phys_reg_name(x), shift, disp);
        let load_instr = self.load_instr_for_type(ty);
        if let Some(&d) = self.reg_assignments.get(&dest.0) {
            emit!(
                self.state,
                "    {} {}, %{}",
                load_instr,
                mem,
                phys_reg_name(d)
            );
            self.state.reg_cache.invalidate_acc();
            return true;
        }
        emit!(self.state, "    {} {}, %eax", load_instr, mem);
        self.state.reg_cache.invalidate_acc();
        self.emit_store_result(dest);
        true
    }

    pub(super) fn emit_store_indexed_sym_impl(
        &mut self,
        val: &Operand,
        sym: &str,
        index: &Value,
        shift: u8,
        disp: i64,
        ty: IrType,
    ) -> bool {
        if self.state.pic_mode || !Self::sib_scalar_ty(ty) || shift > 3 {
            return false;
        }
        let Some(&x) = self.reg_assignments.get(&index.0) else {
            return false;
        };
        let mem = Self::sib_mem_sym(sym, phys_reg_name(x), shift, disp);
        let store_instr = self.store_instr_for_type(ty);
        if let Some(src) = self.direct_store_src(val, ty) {
            emit!(self.state, "    {} {}, {}", store_instr, src, mem);
            return true;
        }
        // Staging through %eax: the index must survive it.
        if !matches!(x.0, 0 | 1 | 2 | 3) {
            return false;
        }
        self.operand_to_eax(val);
        let src = self.eax_for_type(ty);
        emit!(self.state, "    {} {}, {}", store_instr, src, mem);
        self.state.reg_cache.invalidate_acc();
        true
    }

    // ---- Typed store/load helpers ----

    pub(super) fn emit_typed_store_to_slot_impl(
        &mut self,
        instr: &'static str,
        ty: IrType,
        slot: StackSlot,
    ) {
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

    pub(super) fn emit_slot_addr_to_secondary_impl(
        &mut self,
        slot: StackSlot,
        is_alloca: bool,
        val_id: u32,
    ) {
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

    pub(super) fn emit_gep_indirect_const_impl(
        &mut self,
        slot: StackSlot,
        offset: i64,
        val_id: u32,
    ) {
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
        let align = self
            .state
            .alloca_over_align(val_id)
            .expect("alloca must have over-alignment for aligned addr emission");
        let sr = self.slot_ref(slot);
        emit!(self.state, "    leal {}, %ecx", sr);
        emit!(self.state, "    addl ${}, %ecx", align - 1);
        emit!(self.state, "    andl ${}, %ecx", -(align as i32));
    }

    pub(super) fn emit_alloca_aligned_addr_to_acc_impl(&mut self, slot: StackSlot, val_id: u32) {
        let align = self
            .state
            .alloca_over_align(val_id)
            .expect("alloca must have over-alignment for aligned addr emission");
        let sr = self.slot_ref(slot);
        emit!(self.state, "    leal {}, %eax", sr);
        emit!(self.state, "    addl ${}, %eax", align - 1);
        emit!(self.state, "    andl ${}, %eax", -(align as i32));
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
            self.emit_alloca_addr_to("edi", val_id, slot);
        } else if let Some(phys) = self.reg_assignments.get(&val_id).copied() {
            let reg = phys_reg_name(phys);
            emit!(self.state, "    movl %{}, %edi", reg);
        } else {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movl {}, %edi", sr);
        }
    }

    pub(super) fn emit_memcpy_load_src_addr_impl(
        &mut self,
        slot: StackSlot,
        is_alloca: bool,
        val_id: u32,
    ) {
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

    /// Materialize a 128-bit container's DESTINATION address into %ecx.
    /// Handles Direct (leal), Indirect (pointer load), OverAligned (alloca
    /// re-align) and Reg bases.
    ///
    /// SCRATCH DISCIPLINE (see emit_i128_src_addr): this runs FIRST in the
    /// store sequence, while %edx still holds nothing the sequence needs, so
    /// every arm may read register homes freely — the RA classifies 128-bit
    /// Store/Copy/Intrinsic points as %ecx/%edx-dirty, meaning no LIVE value
    /// is ever homed there across the point.
    fn emit_i128_dest_addr(&mut self, ptr: &Value) {
        match self.state.resolve_slot_addr(ptr.0) {
            Some(SlotAddr::Direct(slot)) => {
                let sr = self.slot_ref(slot);
                emit!(self.state, "    leal {}, %ecx", sr);
            }
            Some(SlotAddr::Indirect(slot)) => {
                self.emit_load_ptr_from_slot(slot, ptr.0);
            }
            Some(SlotAddr::OverAligned(slot, id)) => {
                self.emit_alloca_aligned_addr(slot, id);
            }
            Some(SlotAddr::Reg(reg)) => {
                let r = phys_reg_name(reg);
                if r != "ecx" {
                    emit!(self.state, "    movl %{}, %ecx", r);
                }
            }
            None => {
                self.emit_load_ptr_from_slot_generic(ptr.0);
            }
        }
        self.state.reg_cache.invalidate_sec();
    }

    /// Materialize a 128-bit container's SOURCE address into %edx.
    ///
    /// SCRATCH DISCIPLINE: %ecx already holds the DESTINATION address
    /// (emit_i128_dest_addr ran first), so every arm here targets %edx
    /// DIRECTLY — the historic form routed the Indirect/OverAligned pointer
    /// through %ecx (`movl {slot}, %ecx; movl %ecx, %edx`), destroying the
    /// destination and turning the 4-word copy loop into a self-copy through
    /// an arbitrary stack value (SIGSEGV; the peephole's copy propagation
    /// then faithfully amplified the wreckage).
    fn emit_i128_src_addr(&mut self, val: &Operand) {
        if let Operand::Value(sv) = val {
            match self.state.resolve_slot_addr(sv.0) {
                Some(SlotAddr::Direct(slot)) => {
                    let sr = self.slot_ref(slot);
                    emit!(self.state, "    leal {}, %edx", sr);
                    return;
                }
                Some(SlotAddr::Indirect(slot)) => {
                    // Load the pointer value straight into %edx: register
                    // home when one exists (never %ecx/%edx for a live value
                    // at this RA-dirty point), else the esp-relative slot.
                    if let Some(phys) = self.reg_assignments.get(&sv.0).copied() {
                        let r = phys_reg_name(phys);
                        if r != "edx" {
                            emit!(self.state, "    movl %{}, %edx", r);
                        }
                    } else {
                        let sr = self.slot_ref(slot);
                        emit!(self.state, "    movl {}, %edx", sr);
                    }
                    return;
                }
                Some(SlotAddr::OverAligned(slot, id)) => {
                    // Inline the leal/addl/andl alignment ladder against %edx
                    // (emit_alloca_aligned_addr targets %ecx = the dest).
                    let align = self
                        .state
                        .alloca_over_align(sv.0)
                        .expect("alloca must have over-alignment for aligned addr emission");
                    let sr = self.slot_ref(slot);
                    emit!(self.state, "    leal {}, %edx", sr);
                    emit!(self.state, "    addl ${}, %edx", align - 1);
                    emit!(self.state, "    andl ${}, %edx", -(align as i32));
                    return;
                }
                Some(SlotAddr::Reg(reg)) => {
                    let r = phys_reg_name(reg);
                    if r != "edx" {
                        emit!(self.state, "    movl %{}, %edx", r);
                    }
                    return;
                }
                None => {
                    if let Some(phys) = self.reg_assignments.get(&sv.0).copied() {
                        let r = phys_reg_name(phys);
                        if r != "edx" {
                            emit!(self.state, "    movl %{}, %edx", r);
                        }
                    } else {
                        self.operand_to_eax(&Operand::Value(*sv));
                        self.state.emit("    movl %eax, %edx");
                        self.state.reg_cache.invalidate_acc();
                    }
                    return;
                }
            }
        }
        // Constants with no 16-byte word form (should not occur): stage via
        // the accumulator.
        self.emit_load_operand(val);
        self.state.emit("    movl %eax, %edx");
        self.state.reg_cache.invalidate_acc();
        self.state.reg_cache.invalidate_sec();
    }

    /// Last-resort pointer materialization for values without a slot address
    /// (stack spills of pointer values): stage through %ecx via the value's
    /// register home or an accumulator load.
    fn emit_load_ptr_from_slot_generic(&mut self, v: u32) {
        if let Some(phys) = self.reg_assignments.get(&v).copied() {
            let r = phys_reg_name(phys);
            if r != "ecx" {
                emit!(self.state, "    movl %{}, %ecx", r);
            }
            return;
        }
        // No slot address and no register home: the value was spilled.
        // `value_to_reg` stages it through the accumulator (%eax); move it
        // into the %ecx address scratch.
        self.operand_to_eax(&Operand::Value(Value(v)));
        self.state.emit("    movl %eax, %ecx");
        self.state.reg_cache.invalidate_acc();
        self.state.reg_cache.invalidate_sec();
    }

    pub(super) fn emit_memcpy_impl_impl(&mut self, size: usize) {
        emit!(self.state, "    movl ${}, %ecx", size);
        self.state.emit("    rep movsb");
    }
}
