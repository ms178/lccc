//! X86Codegen: integer/float arithmetic, unary ops, binop, copy.

use super::emit::{shift_mnemonic, X86Codegen};
use crate::backend::regalloc::PhysReg;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{IrBinOp, Operand, Value};

impl X86Codegen {
    // ---- Unary ----

    pub(super) fn emit_float_neg_impl(&mut self, ty: IrType) {
        // The value lives in the GPR accumulator as its IEEE-754 bit pattern,
        // so the sign is a single bit flip — no GPR<->XMM domain crossings and
        // no movabsq-to-xmm shuttle. F32: bit 31 of %eax (xorl zero-extends).
        // F64: bit 63 of %rax; xorq has no imm64 form, so the mask is moved
        // through %rcx (the secondary scratch) first.
        if ty == IrType::F32 {
            self.state.emit("    xorl $0x80000000, %eax");
        } else {
            self.state.emit("    movabsq $-9223372036854775808, %rcx");
            self.state.emit("    xorq %rcx, %rax");
            self.state.reg_cache.invalidate_sec();
        }
    }

    pub(super) fn emit_int_neg_impl(&mut self, ty: IrType) {
        // Width-correct: 32-bit forms zero-extend the upper half (the SysV
        // i32-home invariant); negq on a zero-extended U32 would produce
        // 0xFFFFFFFF00000001 instead of 1.
        match ty {
            IrType::I8 | IrType::U8 => self.state.emit("    negb %al"),
            IrType::I16 | IrType::U16 => self.state.emit("    negw %ax"),
            IrType::I32 | IrType::U32 => self.state.emit("    negl %eax"),
            _ => self.state.emit("    negq %rax"),
        }
    }

    pub(super) fn emit_int_not_impl(&mut self, ty: IrType) {
        match ty {
            IrType::I8 | IrType::U8 => self.state.emit("    notb %al"),
            IrType::I16 | IrType::U16 => self.state.emit("    notw %ax"),
            IrType::I32 | IrType::U32 => self.state.emit("    notl %eax"),
            _ => self.state.emit("    notq %rax"),
        }
    }

    pub(super) fn emit_and_not_impl(
        &mut self,
        not_src: &Operand,
        other: &Operand,
        dest: &Value,
        ty: IrType,
        direct_return: bool,
    ) {
        let narrow = matches!(ty, IrType::I32 | IrType::U32);
        let home = |this: &Self, operand: &Operand| -> Option<String> {
            let Operand::Value(value) = operand else {
                return None;
            };
            let reg = this.reg_assignments.get(&value.0).copied()?;
            if super::emit::is_xmm_reg(reg) || this.state.is_alloca(value.0) {
                return None;
            }
            Some(if narrow {
                super::emit::phys_reg_name_32(reg).to_string()
            } else {
                super::emit::phys_reg_name(reg).to_string()
            })
        };
        let not_name = match home(self, not_src) {
            Some(name) => name,
            None => {
                if narrow {
                    self.operand_to_eax(not_src);
                } else {
                    self.operand_to_rax(not_src);
                }
                if narrow {
                    "eax".into()
                } else {
                    "rax".into()
                }
            }
        };
        let other_name = match home(self, other) {
            Some(name) => name,
            None => {
                self.operand_to_rcx(other);
                if narrow {
                    "ecx".into()
                } else {
                    "rcx".into()
                }
            }
        };

        if direct_return {
            let suffix = if narrow { "l" } else { "q" };
            self.state.emit_fmt(format_args!(
                "    andn{} %{}, %{}, %{}",
                suffix,
                other_name,
                not_name,
                if narrow { "eax" } else { "rax" }
            ));
            self.state.reg_cache.set_acc(dest.0, false);
            return;
        }

        if let Some(reg) = self
            .dest_reg(dest)
            .filter(|r| !super::emit::is_xmm_reg(*r))
        {
            let name = if narrow {
                super::emit::phys_reg_name_32(reg)
            } else {
                super::emit::phys_reg_name(reg)
            };
            let suffix = if narrow { "l" } else { "q" };
            self.state.emit_fmt(format_args!(
                "    andn{} %{}, %{}, %{}",
                suffix, other_name, not_name, name
            ));
            self.state.reg_cache.invalidate_acc();
        } else {
            let suffix = if narrow { "l" } else { "q" };
            self.state.emit_fmt(format_args!(
                "    andn{} %{}, %{}, %{}",
                suffix,
                other_name,
                not_name,
                if narrow { "eax" } else { "rax" }
            ));
            self.store_rax_to(dest);
        }
    }

    pub(super) fn emit_int_clz_impl(&mut self, ty: IrType) {
        match ty {
            IrType::I8 | IrType::U8 => {
                // clz8(x) = lzcnt32(zext(x)) - 24.
                self.state.emit("    movzbl %al, %eax");
                self.state.emit("    lzcntl %eax, %eax");
                self.state.emit("    subl $24, %eax");
            }
            IrType::I16 | IrType::U16 => {
                self.state.emit("    movzwl %ax, %eax");
                self.state.emit("    lzcntl %eax, %eax");
                self.state.emit("    subl $16, %eax");
            }
            IrType::I32 | IrType::U32 => self.state.emit("    lzcntl %eax, %eax"),
            _ => self.state.emit("    lzcntq %rax, %rax"),
        }
    }

    pub(super) fn emit_int_ctz_impl(&mut self, ty: IrType) {
        match ty {
            IrType::I8 | IrType::U8 => {
                // Zero-extend so only the real low byte's trailing zeros count.
                self.state.emit("    movzbl %al, %eax");
                self.state.emit("    tzcntl %eax, %eax");
            }
            IrType::I16 | IrType::U16 => {
                self.state.emit("    movzwl %ax, %eax");
                self.state.emit("    tzcntl %eax, %eax");
            }
            IrType::I32 | IrType::U32 => self.state.emit("    tzcntl %eax, %eax"),
            _ => self.state.emit("    tzcntq %rax, %rax"),
        }
    }

    pub(super) fn emit_int_bswap_impl(&mut self, ty: IrType) {
        match ty {
            // Byte-swap of a single byte is the identity.
            IrType::I8 | IrType::U8 => {}
            IrType::I16 | IrType::U16 => self.state.emit("    rolw $8, %ax"),
            IrType::I32 | IrType::U32 => self.state.emit("    bswapl %eax"),
            _ => self.state.emit("    bswapq %rax"),
        }
    }

    pub(super) fn emit_int_popcount_impl(&mut self, ty: IrType) {
        match ty {
            IrType::I8 | IrType::U8 => {
                self.state.emit("    movzbl %al, %eax");
                self.state.emit("    popcntl %eax, %eax");
            }
            IrType::I16 | IrType::U16 => {
                self.state.emit("    movzwl %ax, %eax");
                self.state.emit("    popcntl %eax, %eax");
            }
            IrType::I32 | IrType::U32 => self.state.emit("    popcntl %eax, %eax"),
            _ => self.state.emit("    popcntq %rax, %rax"),
        }
    }

    /// Emit `dest = BT(base, index) & 1` directly to a physical register.
    /// Returns false only when the result register is BT's fixed count
    /// register (%rcx) or is not an ordinary GPR; the accumulator fallback
    /// handles those shapes. SETcc writes an 8-bit GPR directly, and a 32-bit
    /// result is zero-extended with `movzbl`. For 64-bit results the zero
    /// extension from 32-bit MOVL is enough.
    fn emit_bit_test_reg_direct(
        &mut self,
        base: &Operand,
        index: &Operand,
        dest_phys: PhysReg,
        use_32bit: bool,
    ) -> bool {
        let dest64 = super::emit::phys_reg_name(dest_phys);
        if dest64 == "rcx" {
            return false;
        }
        let dest8 = super::emit::typed_phys_reg_name(dest_phys, IrType::U8);
        // Put the base into the destination first; BT reads the base and
        // writes only CF. The index must go to %rcx after this, so it does
        // not matter whether the base and index source registers alias.
        match base {
            Operand::Value(v) => {
                // An XMM-homed base (bit-punned float word) has no GPR
                // sub-register names; route it through the accumulator arm
                // below, whose operand_to_rax performs the xmm->GPR movq
                // (glibc s_nextupf: BitTest on GET_FLOAT_WORD's value).
                if let Some(&reg) = self
                    .reg_assignments
                    .get(&v.0)
                    .filter(|r| !super::emit::is_xmm_reg(**r))
                {
                    let src64 = super::emit::phys_reg_name(reg);
                    if src64 != dest64 {
                        if use_32bit {
                            let src32 = super::emit::phys_reg_name_32(reg);
                            let dest32 = super::emit::phys_reg_name_32(dest_phys);
                            self.state
                                .emit_fmt(format_args!("    movl %{src32}, %{dest32}"));
                        } else {
                            self.state
                                .emit_fmt(format_args!("    movq %{src64}, %{dest64}"));
                        }
                    }
                } else {
                    // An adjacent producer may deliberately live only in the
                    // allocator-owned accumulator location. `value_to_reg`
                    // bypasses that cache and hard-fails for such a value;
                    // consume through the normal operand path first, then copy
                    // into BT's chosen result register. The canonical
                    // `(x & y) >> bit & 1` handoff exercises this at -O1+.
                    self.operand_to_rax(base);
                    if use_32bit {
                        let dest32 = super::emit::phys_reg_name_32(dest_phys);
                        self.state
                            .emit_fmt(format_args!("    movl %eax, %{dest32}"));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    movq %rax, %{dest64}"));
                    }
                }
            }
            Operand::Const(_) => self.operand_to_reg(base, dest64),
        }
        let const_index = match index {
            Operand::Const(c) => c
                .to_i64()
                .filter(|v| *v >= i32::MIN as i64 && *v <= i32::MAX as i64),
            _ => None,
        };
        if let Some(imm) = const_index {
            let width = if use_32bit { 32 } else { 64 };
            let bit = (imm as u32) % width;
            let bt = if use_32bit { "btl" } else { "btq" };
            self.state
                .emit_fmt(format_args!("    {bt} ${bit}, %{dest64}"));
        } else {
            // BT's bit index is an ordinary r/m operand — it has no fixed
            // count register (that restriction belongs to variable shifts,
            // which use %cl).  Consume the index's own register directly when
            // it has one; only a home-less value needs %rcx staging.  The old
            // unconditional `movq %rN, %rcx; btl %ecx, %dest` paid a copy plus
            // a partial-register dependency per classify (Expat name scan).
            let bt = if use_32bit { "btl" } else { "btq" };
            let own_reg = match index {
                Operand::Value(v) => self.reg_assignments.get(&v.0).copied(),
                _ => None,
            };
            match own_reg {
                // The base was just staged into dest, so an index sharing
                // dest's register is clobbered — stage those via %rcx.
                Some(reg) if !super::emit::is_xmm_reg(reg) && reg != dest_phys => {
                    let idx = if use_32bit {
                        super::emit::phys_reg_name_32(reg)
                    } else {
                        super::emit::phys_reg_name(reg)
                    };
                    self.state
                        .emit_fmt(format_args!("    {bt} %{idx}, %{dest64}"));
                }
                _ => {
                    self.operand_to_rcx(index);
                    let src = if use_32bit { "ecx" } else { "rcx" };
                    self.state
                        .emit_fmt(format_args!("    {bt} %{src}, %{dest64}"));
                }
            }
        }
        self.state.emit_fmt(format_args!("    setc %{dest8}"));
        // ALWAYS zero-extend: SETcc writes one byte only, and the
        // destination was staged with the BASE (mask) value. Without the
        // extension the 64-bit result kept mask bits 8..63 and a later
        // `testq %dest,%dest` was nonzero for every in-range index
        // (set_membership two-block form: bytes 59..64 classified as XML
        // name chars). A 32-bit MOVZX zero-extends the full 64-bit
        // register on x86-64, so one form covers both widths.
        let dest32 = super::emit::phys_reg_name_32(dest_phys);
        self.state
            .emit_fmt(format_args!("    movzbl %{dest8}, %{dest32}"));
        true
    }

    // ---- Binop ----

    pub(super) fn emit_int_binop_impl(
        &mut self,
        dest: &Value,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;
        let is_unsigned = ty.is_unsigned();

        // Register-direct path. An XMM-homed dest (integer value the RA
        // parked in an XMM register: bit-punned float words) has no GPR
        // sub-register names — emit_alu_reg_direct would hit
        // phys_reg_name_32's unreachable!(). The accumulator path below
        // handles XMM homes on both loads (operand_to_rax) and the final
        // store (store_rax_to).
        if let Some(dest_phys) = self
            .dest_reg(dest)
            .filter(|r| !super::emit::is_xmm_reg(*r))
        {
            let is_simple_alu = matches!(
                op,
                IrBinOp::Add
                    | IrBinOp::Sub
                    | IrBinOp::And
                    | IrBinOp::Or
                    | IrBinOp::Xor
                    | IrBinOp::Mul
            );
            if is_simple_alu {
                self.emit_alu_reg_direct(op, lhs, rhs, dest_phys, use_32bit, is_unsigned, dest.0);
                return;
            }
            if matches!(op, IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr) {
                self.emit_shift_reg_direct(op, lhs, rhs, dest_phys, use_32bit, is_unsigned, dest.0);
                return;
            }
            if op == IrBinOp::BitTest {
                if self.emit_bit_test_reg_direct(lhs, rhs, dest_phys, use_32bit) {
                    return;
                }
            }
        }

        if op == IrBinOp::BitTest {
            // Use the native BT instruction: base stays in %rax, the index is
            // consumed from its own register when it has one (BT's index is an
            // ordinary r/m operand — only variable shifts are pinned to %cl),
            // SETC materializes CF, and the boolean is stored to the
            // destination.  This is the cross-cutting canonical lower instead
            // of patching text peepholes.
            self.emit_load_operand(lhs);
            if let Some(imm) = Self::const_as_imm32(rhs) {
                let width = if use_32bit { 32 } else { 64 };
                let bit = (imm as u32) % width;
                let bt_line = if use_32bit {
                    format!("    btl ${bit}, %eax")
                } else {
                    format!("    btq ${bit}, %rax")
                };
                self.state.emit(&bt_line);
            } else {
                // %rax is not allocatable on x86-64, so a register-resident
                // index can never be clobbered by the base load above.  A
                // value without a home still stages through %rcx.
                let own_reg = match rhs {
                    Operand::Value(v) => self.reg_assignments.get(&v.0).copied(),
                    _ => None,
                };
                match own_reg {
                    Some(reg) if !super::emit::is_xmm_reg(reg) => {
                        let idx = if use_32bit {
                            super::emit::phys_reg_name_32(reg)
                        } else {
                            super::emit::phys_reg_name(reg)
                        };
                        let dst = if use_32bit { "eax" } else { "rax" };
                        let bt = if use_32bit { "btl" } else { "btq" };
                        self.state.emit_fmt(format_args!("    {bt} %{idx}, %{dst}"));
                    }
                    _ => {
                        self.operand_to_rcx(rhs);
                        self.state.emit(if use_32bit {
                            "    btl %ecx, %eax"
                        } else {
                            "    btq %rcx, %rax"
                        });
                    }
                }
            }
            self.state.emit("    setc %al");
            self.state.emit(if use_32bit {
                "    movzbl %al, %eax"
            } else {
                "    movzbq %al, %rax"
            });
            self.store_rax_to(dest);
            return;
        }

        // Accumulator-based fallback: try immediate optimizations first
        if self.try_emit_acc_immediate(dest, op, lhs, rhs, use_32bit, is_unsigned) {
            return;
        }

        // Memory-destination ALU: for in-place updates (a op= expr) where dest/lhs share
        // a stack slot. Emits `op %reg, mem` or `op $imm, mem` which does NOT modify
        // any register — breaks the serial %rax dependency chain for ILP.
        // Works for Add, Sub, And, Or, Xor (all have reg/imm-to-memory forms on x86).
        let mem_dest_mnem = match op {
            IrBinOp::Add => Some("add"),
            IrBinOp::Sub => Some("sub"),
            IrBinOp::And => Some("and"),
            IrBinOp::Or => Some("or"),
            IrBinOp::Xor => Some("xor"),
            _ => None,
        };
        if let Some(mnem) = mem_dest_mnem {
            if let Operand::Value(lhs_val) = lhs {
                if self.dest_reg(dest).is_none() && self.dest_reg(lhs_val).is_none() {
                    if let (Some(dest_slot), Some(lhs_slot)) =
                        (self.state.get_slot(dest.0), self.state.get_slot(lhs_val.0))
                    {
                        if dest_slot.0 == lhs_slot.0 {
                            let sref = self.slot_ref(dest_slot.0);
                            // Try register source: op %reg, mem
                            if let Some(rhs_phys) = self
                                .operand_reg(rhs)
                                .filter(|r| !super::emit::is_xmm_reg(*r))
                            {
                                if use_32bit {
                                    let rhs_32 = super::emit::phys_reg_name_32(rhs_phys);
                                    self.state.emit_fmt(format_args!(
                                        "    {}l %{}, {}",
                                        mnem, rhs_32, sref
                                    ));
                                } else {
                                    let rhs_64 = super::emit::phys_reg_name(rhs_phys);
                                    self.state.emit_fmt(format_args!(
                                        "    {}q %{}, {}",
                                        mnem, rhs_64, sref
                                    ));
                                }
                                // The MEMORY holding dest/lhs changed; a prior
                                // `store_rax_to` may have cached dest/lhs in the
                                // accumulator, which now holds a stale value.
                                // The register source is untouched, so only the
                                // memory-backed ids are invalidated.
                                self.invalidate_cache_for_values(&[dest.0, lhs_val.0]);
                                return;
                            }
                            // Try immediate source: op $imm, mem
                            if let Some(imm) = Self::const_as_imm32_typed(rhs, use_32bit) {
                                if use_32bit {
                                    self.state
                                        .emit_fmt(format_args!("    {}l ${}, {}", mnem, imm, sref));
                                } else {
                                    self.state
                                        .emit_fmt(format_args!("    {}q ${}, {}", mnem, imm, sref));
                                }
                                self.invalidate_cache_for_values(&[dest.0, lhs_val.0]);
                                return;
                            }
                        }
                    }
                }
            }
        }

        // Memory-operand optimization: for ALU ops with memory source form,
        // use memory source directly instead of loading rhs into %rcx first.
        // Pattern: addq -N(%rbp), %rax  (saves one movq load instruction)
        // For Mul: imull -N(%rbp), %eax  (2-operand form, does NOT use rdx)
        // Extends to And/Or/Xor which have identical instruction forms.
        let mem_op_mnem = match op {
            IrBinOp::Add => Some("add"),
            IrBinOp::Sub => Some("sub"),
            IrBinOp::Mul => Some("imul"),
            IrBinOp::And => Some("and"),
            IrBinOp::Or => Some("or"),
            IrBinOp::Xor => Some("xor"),
            _ => None,
        };
        if let Some(mnem) = mem_op_mnem {
            if let Operand::Value(rhs_val) = rhs {
                // Check if rhs has a stack slot and is NOT register-allocated.
                // An alloca's stack slot IS its data, but the alloca's VALUE is its
                // ADDRESS (a pointer), so folding an alloca as a memory source here
                // (`opq slot, %rax`) would read the array's first element instead of
                // adding the base address. Skip allocas so the general path below
                // materializes them with `lea` (value_to_reg/operand_to_rax).
                if self.dest_reg(&rhs_val).is_none() && !self.state.is_alloca(rhs_val.0) {
                    if let Some(slot) = self.state.get_slot(rhs_val.0) {
                        if use_32bit {
                            self.operand_to_eax(lhs);
                        } else {
                            self.operand_to_rax(lhs);
                        }
                        let sref = self.slot_ref(slot.0);
                        if use_32bit {
                            self.state
                                .emit_fmt(format_args!("    {}l {}, %eax", mnem, sref));
                            self.store_eax_to(dest);
                        } else {
                            self.state
                                .emit_fmt(format_args!("    {}q {}, %rax", mnem, sref));
                            self.store_rax_to(dest);
                        }
                        return;
                    }
                }
            }
            // Also try memory-operand for lhs (swap: rhs to rax, lhs from memory)
            // for commutative ops: Add, Mul, And, Or, Xor (NOT Sub — non-commutative).
            // Same alloca caveat as above: an alloca lhs is a base-address pointer and
            // must NOT be folded as a memory source (that would read its data, not its
            // address).
            let is_commutative = !matches!(op, IrBinOp::Sub);
            if is_commutative {
                if let Operand::Value(lhs_val) = lhs {
                    if self.dest_reg(&lhs_val).is_none() && !self.state.is_alloca(lhs_val.0) {
                        if let Some(slot) = self.state.get_slot(lhs_val.0) {
                            if use_32bit {
                                self.operand_to_eax(rhs);
                            } else {
                                self.operand_to_rax(rhs);
                            }
                            let sref = self.slot_ref(slot.0);
                            if use_32bit {
                                self.state
                                    .emit_fmt(format_args!("    {}l {}, %eax", mnem, sref));
                                self.store_eax_to(dest);
                            } else {
                                self.state
                                    .emit_fmt(format_args!("    {}q {}, %rax", mnem, sref));
                                self.store_rax_to(dest);
                            }
                            return;
                        }
                    }
                }
            }
        }

        // Immediate multiply: `imull $imm, %eax, %eax` (3-operand immediate
        // form) is one instruction and needs no scratch register, versus the
        // two-instruction `movq $imm, %rcx; imull %ecx, %eax` general path.
        // IMUL with imm32 sign-extends the immediate; for the 32-bit form any
        // 32-bit bit pattern yields the correct low 32 bits (N ≡ -1 mod 2^32),
        // so the typed helper's unsigned extension is sound there. The 64-bit
        // form is restricted to the signed i32 range.
        if op == IrBinOp::Mul {
            let imm = Self::const_as_imm32_typed(rhs, use_32bit);
            if let Some(imm) = imm {
                if use_32bit {
                    self.operand_to_eax(lhs);
                    self.state
                        .emit_fmt(format_args!("    imull ${}, %eax, %eax", imm));
                    self.store_eax_to(dest);
                } else {
                    self.operand_to_rax(lhs);
                    self.state
                        .emit_fmt(format_args!("    imulq ${}, %rax, %rax", imm));
                    self.store_rax_to(dest);
                }
                return;
            }
        }

        // General case: load lhs to rax, rhs to rcx
        if use_32bit {
            self.operand_to_eax(lhs);
        } else {
            self.operand_to_rax(lhs);
        }
        self.operand_to_rcx(rhs);

        match op {
            IrBinOp::Add | IrBinOp::Sub | IrBinOp::Mul => {
                let mnem = match op {
                    IrBinOp::Add => "add",
                    IrBinOp::Sub => "sub",
                    IrBinOp::Mul => "imul",
                    _ => unreachable!("unexpected i64 binop: {:?}", op),
                };
                if use_32bit {
                    self.state
                        .emit_fmt(format_args!("    {}l %ecx, %eax", mnem));
                } else {
                    self.state
                        .emit_fmt(format_args!("    {}q %rcx, %rax", mnem));
                }
            }
            IrBinOp::SDiv => {
                if use_32bit {
                    self.state.emit("    cltd");
                    self.state.emit("    idivl %ecx");
                } else {
                    self.state.emit("    cqto");
                    self.state.emit("    idivq %rcx");
                }
            }
            IrBinOp::UDiv => {
                self.state.emit("    xorl %edx, %edx");
                if use_32bit {
                    self.state.emit("    divl %ecx");
                } else {
                    self.state.emit("    divq %rcx");
                }
            }
            IrBinOp::SRem => {
                if use_32bit {
                    self.state.emit("    cltd");
                    self.state.emit("    idivl %ecx");
                    self.state.emit("    movl %edx, %eax");
                } else {
                    self.state.emit("    cqto");
                    self.state.emit("    idivq %rcx");
                    self.state.emit("    movq %rdx, %rax");
                }
            }
            IrBinOp::URem => {
                self.state.emit("    xorl %edx, %edx");
                if use_32bit {
                    self.state.emit("    divl %ecx");
                    self.state.emit("    movl %edx, %eax");
                } else {
                    self.state.emit("    divq %rcx");
                    self.state.emit("    movq %rdx, %rax");
                }
            }
            IrBinOp::And => {
                if use_32bit {
                    self.state.emit("    andl %ecx, %eax");
                } else {
                    self.state.emit("    andq %rcx, %rax");
                }
            }
            IrBinOp::Or => {
                if use_32bit {
                    self.state.emit("    orl %ecx, %eax");
                } else {
                    self.state.emit("    orq %rcx, %rax");
                }
            }
            IrBinOp::Xor => {
                if use_32bit {
                    self.state.emit("    xorl %ecx, %eax");
                } else {
                    self.state.emit("    xorq %rcx, %rax");
                }
            }
            IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr => {
                let (mnem32, mnem64) = shift_mnemonic(op);
                if use_32bit {
                    self.state
                        .emit_fmt(format_args!("    {} %cl, %eax", mnem32));
                } else {
                    self.state
                        .emit_fmt(format_args!("    {} %cl, %rax", mnem64));
                }
            }
            IrBinOp::BitTest => unreachable!("BitTest handled by native BT fallback"),
        }

        self.state.reg_cache.invalidate_acc();
        if use_32bit {
            self.store_eax_to(dest);
        } else {
            self.store_rax_to(dest);
        }
    }

    /// Invalidate acc/sec cache entries that claim to hold one of `ids`. Used
    /// after in-place memory updates (`op %reg/imm, mem`): the registers are
    /// unchanged, but the memory those ids live in is now different, so a
    /// cached register copy of them is stale.
    fn invalidate_cache_for_values(&mut self, ids: &[u32]) {
        if let Some(e) = self.state.reg_cache.acc {
            if ids.contains(&e.value_id) {
                self.state.reg_cache.invalidate_acc();
            }
        }
        if let Some(e) = self.state.reg_cache.sec {
            if ids.contains(&e.value_id) {
                self.state.reg_cache.invalidate_sec();
            }
        }
    }

    /// Fused multiply-add: add_dest = acc + (mul_lhs * mul_rhs).
    ///
    /// Emits a 3-instruction sequence: load one mul operand to %eax, multiply
    /// by the other (memory-source or register-source), then add %eax to the
    /// accumulator (register-dest or memory-dest).
    pub(super) fn emit_fused_mul_add_impl(
        &mut self,
        _mul_dest: &Value,
        mul_lhs: &Operand,
        mul_rhs: &Operand,
        acc: &Operand,
        add_dest: &Value,
        ty: IrType,
    ) {
        if matches!(ty, IrType::F32 | IrType::F64) {
            self.emit_scalar_fma231(mul_lhs, mul_rhs, acc, add_dest, ty);
            return;
        }
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;

        // Step 1: Compute mul_lhs * mul_rhs into %eax.
        // Strategy: load one operand to %eax, imul the other (prefer memory-source).
        if use_32bit {
            self.operand_to_eax(mul_lhs);
        } else {
            self.operand_to_rax(mul_lhs);
        }

        // Try memory-source multiply for rhs. An alloca's VALUE is its ADDRESS
        // (a pointer), so folding it as a memory source would read the array's
        // first element instead of multiplying by the base address — the same
        // alloca guard the plain binop mem-source path has.
        if let Operand::Value(rhs_val) = mul_rhs {
            if self.dest_reg(rhs_val).is_none() && !self.state.is_alloca(rhs_val.0) {
                if let Some(slot) = self.state.get_slot(rhs_val.0) {
                    let sref = self.slot_ref(slot.0);
                    if use_32bit {
                        self.state
                            .emit_fmt(format_args!("    imull {}, %eax", sref));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    imulq {}, %rax", sref));
                    }
                    // Fall through to add
                    self.emit_fused_add_acc(acc, add_dest, use_32bit);
                    return;
                }
            }
        }

        // Immediate multiply: fold the constant into the 3-operand immediate
        // form instead of staging it in %rcx first (`imull $imm, %eax, %eax`).
        if let Some(imm) = Self::const_as_imm32_typed(mul_rhs, use_32bit) {
            if use_32bit {
                self.state
                    .emit_fmt(format_args!("    imull ${}, %eax, %eax", imm));
            } else {
                self.state
                    .emit_fmt(format_args!("    imulq ${}, %rax, %rax", imm));
            }
            self.emit_fused_add_acc(acc, add_dest, use_32bit);
            return;
        }

        // Register-source multiply
        self.operand_to_rcx(mul_rhs);
        if use_32bit {
            self.state.emit("    imull %ecx, %eax");
        } else {
            self.state.emit("    imulq %rcx, %rax");
        }

        self.emit_fused_add_acc(acc, add_dest, use_32bit);
    }

    /// Helper for fused mul-add: add %eax to the accumulator operand and store to dest.
    fn emit_fused_add_acc(&mut self, acc: &Operand, add_dest: &Value, use_32bit: bool) {
        // If add_dest is register-allocated, add %eax to it directly.
        if let Some(dest_phys) = self
            .dest_reg(add_dest)
            .filter(|r| !super::emit::is_xmm_reg(*r))
        {
            // Ensure acc is in the dest register first
            self.operand_to_callee_reg(acc, dest_phys);
            if use_32bit {
                self.state.emit_fmt(format_args!(
                    "    addl %eax, %{}",
                    super::emit::phys_reg_name_32(dest_phys)
                ));
            } else {
                self.state.emit_fmt(format_args!(
                    "    addq %rax, %{}",
                    super::emit::phys_reg_name(dest_phys)
                ));
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }

        // Memory-dest add: if acc and dest share the same stack slot, use addl %eax, mem.
        if let Operand::Value(acc_val) = acc {
            if self.dest_reg(acc_val).is_none() {
                if let (Some(dest_slot), Some(acc_slot)) = (
                    self.state.get_slot(add_dest.0),
                    self.state.get_slot(acc_val.0),
                ) {
                    if dest_slot.0 == acc_slot.0 {
                        let sref = self.slot_ref(dest_slot.0);
                        if use_32bit {
                            self.state.emit_fmt(format_args!("    addl %eax, {}", sref));
                        } else {
                            self.state.emit_fmt(format_args!("    addq %rax, {}", sref));
                        }
                        self.state.reg_cache.invalidate_acc();
                        return;
                    }
                }
            }
        }

        // Fallback: load acc to %ecx, add to %eax, store result.
        self.operand_to_rcx(acc);
        if use_32bit {
            self.state.emit("    addl %ecx, %eax");
            self.store_eax_to(add_dest);
        } else {
            self.state.emit("    addq %rcx, %rax");
            self.store_rax_to(add_dest);
        }
    }

    pub(super) fn emit_copy_i128_impl(&mut self, dest: &Value, src: &Operand) {
        self.operand_to_rax_rdx(src);
        self.store_rax_rdx_to(dest);
    }
}
