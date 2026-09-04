//! X86Codegen: integer/float arithmetic, unary ops, binop, copy.

use super::emit::{phys_reg_name, shift_mnemonic, X86Codegen};
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

        if let Some(reg) = self.dest_reg(dest).filter(|r| !super::emit::is_xmm_reg(*r)) {
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

    /// 32-bit leading-zero count of %eax without LZCNT: 31 - BSR(x).
    /// BSR leaves its destination undefined for x == 0, so the zero case is
    /// fixed up explicitly — this preserves the IR's defined
    /// Clz(0) == width semantics, which constant folding guarantees.
    fn emit_clz32_rax_baseline(&mut self) {
        let nz = self.state.fresh_label("clz_nz");
        let done = self.state.fresh_label("clz_done");
        self.state.emit("    testl %eax, %eax");
        self.state.out.emit_jcc_label("    jnz", &nz);
        self.state.emit("    movl $32, %eax");
        self.state.out.emit_jmp_label(&done);
        self.state.out.emit_named_label(&nz);
        self.state.emit("    bsrl %eax, %eax");
        self.state.emit("    xorl $31, %eax");
        self.state.out.emit_named_label(&done);
    }

    /// 32-bit trailing-zero count of %eax without TZCNT: BSF(x).
    /// BSF's destination is undefined for x == 0; the fixup keeps the
    /// defined Ctz(0) == 32 semantics.
    fn emit_ctz32_rax_baseline(&mut self) {
        let nz = self.state.fresh_label("ctz_nz");
        let done = self.state.fresh_label("ctz_done");
        self.state.emit("    testl %eax, %eax");
        self.state.out.emit_jcc_label("    jnz", &nz);
        self.state.emit("    movl $32, %eax");
        self.state.out.emit_jmp_label(&done);
        self.state.out.emit_named_label(&nz);
        self.state.emit("    bsfl %eax, %eax");
        self.state.out.emit_named_label(&done);
    }

    pub(super) fn emit_int_clz_impl(&mut self, ty: IrType) {
        if self.lzcnt_enabled {
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
            return;
        }
        // Baseline x86-64 has no LZCNT: the F3 0F BD encoding decodes as BSR
        // on CPUs without ABM and silently yields the MSB *index* instead of
        // the leading-zero *count* (bit-flipped result, no fault). This
        // corrupted the preboot ZSTD decoder's FSE/Huffman table builds when
        // an lccc-built kernel booted on QEMU's default qemu64 TCG CPU.
        match ty {
            IrType::I8 | IrType::U8 => {
                self.state.emit("    movzbl %al, %eax");
                self.emit_clz32_rax_baseline();
                self.state.emit("    subl $24, %eax");
            }
            IrType::I16 | IrType::U16 => {
                self.state.emit("    movzwl %ax, %eax");
                self.emit_clz32_rax_baseline();
                self.state.emit("    subl $16, %eax");
            }
            IrType::I32 | IrType::U32 => self.emit_clz32_rax_baseline(),
            _ => {
                let nz = self.state.fresh_label("clz_nz");
                let done = self.state.fresh_label("clz_done");
                self.state.emit("    testq %rax, %rax");
                self.state.out.emit_jcc_label("    jnz", &nz);
                self.state.emit("    movl $64, %eax");
                self.state.out.emit_jmp_label(&done);
                self.state.out.emit_named_label(&nz);
                self.state.emit("    bsrq %rax, %rax");
                self.state.emit("    xorq $63, %rax");
                self.state.out.emit_named_label(&done);
            }
        }
    }

    pub(super) fn emit_int_ctz_impl(&mut self, ty: IrType) {
        if self.lzcnt_enabled {
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
            return;
        }
        // TZCNT (F3 0F BC) decodes as plain BSF without the feature; the two
        // agree for every nonzero input, but BSF's result is undefined for
        // zero, so the zero case is fixed up to preserve Ctz(0) == width.
        match ty {
            IrType::I8 | IrType::U8 => {
                self.state.emit("    movzbl %al, %eax");
                self.emit_ctz32_rax_baseline();
            }
            IrType::I16 | IrType::U16 => {
                self.state.emit("    movzwl %ax, %eax");
                self.emit_ctz32_rax_baseline();
            }
            IrType::I32 | IrType::U32 => self.emit_ctz32_rax_baseline(),
            _ => {
                let nz = self.state.fresh_label("ctz_nz");
                let done = self.state.fresh_label("ctz_done");
                self.state.emit("    testq %rax, %rax");
                self.state.out.emit_jcc_label("    jnz", &nz);
                self.state.emit("    movl $64, %eax");
                self.state.out.emit_jmp_label(&done);
                self.state.out.emit_named_label(&nz);
                self.state.emit("    bsfq %rax, %rax");
                self.state.out.emit_named_label(&done);
            }
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
        let narrow = matches!(ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16);
        if narrow {
            match ty {
                IrType::I8 | IrType::U8 => self.state.emit("    movzbl %al, %eax"),
                _ => self.state.emit("    movzwl %ax, %eax"),
            }
        }
        if self.popcnt_enabled {
            if narrow {
                self.state.emit("    popcntl %eax, %eax");
            } else if matches!(ty, IrType::I32 | IrType::U32) {
                self.state.emit("    popcntl %eax, %eax");
            } else {
                self.state.emit("    popcntq %rax, %rax");
            }
            return;
        }
        // POPCNT (0F B8) is #UD on pre-Nehalem/Barcelona x86-64 — it is a
        // v2 feature, not v1. Baseline fallback: the classic O(1) SWAR
        // Hamming-weight sequence (the same code GCC emits for `__builtin_
        // popcount` on a baseline target), replacing the previous per-bit
        // `shr/adc` loop that iterated once per set bit (up to 64 trips —
        // the largest single benchmark gap on this compiler, ~3.85x on the
        // bitops kernel).
        //
        // The SWAR form is straight-line and data-independent, so it costs
        // ~1 round trip regardless of input. It needs scratch registers:
        //  - 32-bit path uses only %eax (in/out) + %ecx.
        //  - 64-bit path uses %rax (in/out) + %rcx + %rdx (the full-width
        //    masks 0x55.., 0x33.., 0x0f.. and 0x0101.. do not fit an imm32,
        //    so they must be loaded with movabsq into a scratch GPR).
        // %rcx (and %rdx on the 64-bit path) are NOT free here: the
        // surrounding accumulator-mode code may have staged live values in
        // them (e.g. a 64-bit multiply constant reused by a later `imulq`).
        // Preserve them with push/pop pairs; the sequence contains no call,
        // so the transient 16-byte stack excursion is safe at any alignment.
        if narrow || matches!(ty, IrType::I32 | IrType::U32) {
            self.state.emit("    pushq %rcx");
            self.state.emit("    movl %eax, %ecx"); // c = x
            self.state.emit("    shrl $1, %eax");
            self.state.emit("    andl $0x55555555, %eax"); // (x>>1)&0x55..
            self.state.emit("    subl %eax, %ecx"); // c = x - pairs
            self.state.emit("    movl %ecx, %eax");
            self.state.emit("    shrl $2, %eax");
            self.state.emit("    andl $0x33333333, %eax");
            self.state.emit("    andl $0x33333333, %ecx");
            self.state.emit("    addl %eax, %ecx"); // c = nibble counts
            self.state.emit("    movl %ecx, %eax");
            self.state.emit("    shrl $4, %eax");
            self.state.emit("    addl %ecx, %eax");
            self.state.emit("    andl $0x0f0f0f0f, %eax"); // byte counts
            self.state.emit("    imull $0x01010101, %eax, %eax"); // *0x01010101
            self.state.emit("    shrl $24, %eax"); // top byte = popcount
            self.state.emit("    popq %rcx");
        } else {
            self.state.emit("    pushq %rcx");
            self.state.emit("    pushq %rdx");
            self.state.emit("    movq %rax, %rcx"); // c = x
            self.state.emit("    shrq $1, %rax");
            self.state.emit("    movabsq $0x5555555555555555, %rdx");
            self.state.emit("    andq %rdx, %rax"); // (x>>1)&0x55..
            self.state.emit("    subq %rax, %rcx"); // c = x - pairs
            self.state.emit("    movq %rcx, %rax");
            self.state.emit("    shrq $2, %rax");
            self.state.emit("    movabsq $0x3333333333333333, %rdx");
            self.state.emit("    andq %rdx, %rax");
            self.state.emit("    andq %rdx, %rcx");
            self.state.emit("    addq %rax, %rcx"); // c = nibble counts
            self.state.emit("    movq %rcx, %rax");
            self.state.emit("    shrq $4, %rax");
            self.state.emit("    addq %rcx, %rax");
            self.state.emit("    movabsq $0x0f0f0f0f0f0f0f0f, %rdx");
            self.state.emit("    andq %rdx, %rax"); // byte counts
            self.state.emit("    movabsq $0x0101010101010101, %rdx");
            self.state.emit("    imulq %rdx, %rax"); // *0x0101..0101
            self.state.emit("    shrq $56, %rax"); // top byte = popcount
            self.state.emit("    popq %rdx");
            self.state.emit("    popq %rcx");
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

    /// Store %rdx (division remainder) to a value's location. Mirror of
    /// `store_rax_to` with the source register changed; the accumulator
    /// case cannot arise for remainders (pair tails are excluded from
    /// accumulator chains by the shared RA filter), but a dead remainder
    /// (use count 0) skips the store entirely.
    pub(super) fn store_rdx_to(&mut self, dest: &Value, use_32bit: bool) {
        if self
            .state
            .value_use_counts
            .get(dest.0 as usize)
            .copied()
            .unwrap_or(0)
            == 0
        {
            return;
        }
        let use_small = use_32bit && self.state.is_small_slot(dest.0);
        if let Some(&reg) = self.reg_assignments.get(&dest.0) {
            let reg_name = phys_reg_name(reg);
            if super::emit::is_xmm_reg(reg) {
                // Integer value parked in an XMM home (bit-punned float word).
                self.state
                    .emit_fmt(format_args!("    movq %rdx, %{}", reg_name));
            } else {
                // Always movq for register stores (movl would zero-extend and
                // corrupt negative I32 remainders flowing into 64-bit uses).
                self.state
                    .out
                    .emit_instr_reg_reg("    movq", "rdx", reg_name);
            }
        } else if let Some(slot) = self.state.get_slot(dest.0) {
            if use_small {
                self.state.out.emit_instr_reg_rbp("    movl", "edx", slot.0);
            } else {
                self.state.out.emit_instr_reg_rbp("    movq", "rdx", slot.0);
            }
        } else if self.state.is_accumulator_location(dest.0) {
            // Accumulator location means %rax residency — a remainder can
            // never claim it directly. Route through %rax: the consumer's
            // operand_to_rax finds it there.
            if use_32bit {
                self.state.emit("    movl %edx, %eax");
            } else {
                self.state.emit("    movq %rdx, %rax");
            }
            self.state.reg_cache.set_acc(dest.0, false);
        } else {
            panic!(
                "x86 codegen: live remainder value {} has no assigned location",
                dest.0
            );
        }
    }

    /// Emit one divide serving a same-block div/rem pair and store BOTH
    /// results. Returns false when the home combination is unstoreable
    /// (deadlock) — the caller falls back to standalone divisions for both
    /// sides and the tail is marked broken.
    fn emit_divrem_pair_head(
        &mut self,
        dest: &Value,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        partner_dest: u32,
        _partner_from_eax: bool,
    ) -> bool {
        let signed = matches!(op, IrBinOp::SDiv | IrBinOp::SRem);
        let self_is_div = matches!(op, IrBinOp::UDiv | IrBinOp::SDiv);
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;
        if use_32bit {
            self.operand_to_eax(lhs);
        } else {
            self.operand_to_rax(lhs);
        }
        self.operand_to_rcx(rhs);
        if signed {
            if use_32bit {
                self.state.emit("    cltd");
                self.state.emit("    idivl %ecx");
            } else {
                self.state.emit("    cqto");
                self.state.emit("    idivq %rcx");
            }
        } else {
            self.state.emit("    xorl %edx, %edx");
            if use_32bit {
                self.state.emit("    divl %ecx");
            } else {
                self.state.emit("    divq %rcx");
            }
        }
        self.state.reg_cache.invalidate_acc();

        // Which value takes which output register:
        //   div-flavoured dest -> %rax, rem-flavoured dest -> %rdx.
        let div_dest = if self_is_div {
            *dest
        } else {
            Value(partner_dest)
        };
        let rem_dest = if self_is_div {
            Value(partner_dest)
        } else {
            *dest
        };

        let rem_home = self.dest_reg(&rem_dest);
        let div_home = self.dest_reg(&div_dest);
        // Immediately-consumed (accumulator-flow) values have neither a home
        // nor a slot: their consumer reads %rax directly. The pair tail can
        // never be one (the accumulator analysis excludes tails), so at most
        // the HEAD's own dest is slotless.
        let rem_slotless = rem_home.is_none() && self.state.get_slot(rem_dest.0).is_none();
        let div_slotless = div_home.is_none() && self.state.get_slot(div_dest.0).is_none();
        // store_rdx(rem) writes %rax iff rem is HOMED in %rdx... impossible
        // here: %rdx (PhysReg 16) is excluded from allocation in any function
        // containing division (prologue). store_rax(div) writes %rdx iff the
        // quotient is HOMED in %rdx — same exclusion. Keep the screening
        // shape for defence against future allocator changes.
        let rdx_phys = crate::backend::regalloc::PhysReg(16);
        let rem_in_rdx = rem_home == Some(rdx_phys);
        let div_in_rdx = div_home == Some(rdx_phys);
        let _ = rem_in_rdx; // cannot occur; see comment above

        // Deadlock screening (mirrors the i686 fusion; with %rdx excluded
        // from allocation only the slotless combinations can trigger).
        if (rem_in_rdx && div_in_rdx)
            || (rem_slotless && div_in_rdx)
            || (div_slotless && rem_in_rdx)
        {
            self.divrem_broken_tails.insert(partner_dest);
            return false;
        }
        let div_dead = self
            .state
            .value_use_counts
            .get(div_dest.0 as usize)
            .copied()
            .unwrap_or(0)
            == 0;
        if rem_slotless {
            // Remainder must reach %rax for its acc-flow consumer. Store the
            // quotient first (its store cannot touch %rdx), then move the
            // remainder into %rax.
            self.store_rax_to(&div_dest);
            if use_32bit {
                self.state.emit("    movl %edx, %eax");
            } else {
                self.state.emit("    movq %rdx, %rax");
            }
            self.state.reg_cache.set_acc(rem_dest.0, false);
        } else if div_slotless {
            // Quotient stays in %rax for its acc-flow consumer; store the
            // remainder first (its store cannot touch %rax).
            self.store_rdx_to(&rem_dest, use_32bit);
            self.state.reg_cache.set_acc(div_dest.0, false);
        } else {
            // Canonical order: remainder out of %rdx first (its store cannot
            // touch %rax — see screening), then the quotient store.
            self.store_rdx_to(&rem_dest, use_32bit);
            if !div_dead {
                self.store_rax_to(&div_dest);
            } else {
                self.state.reg_cache.invalidate_acc();
            }
        }
        true
    }

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

        // Same-block div/rem pair fusion (compute_i686_divrem_pairs with the
        // X86_64 target). The TAIL of a pair emits nothing — its result was
        // stored by the HEAD's dual-store emission further up in this block
        // (one divq/idivq instead of two: the remainder is free with the
        // quotient on x86). The HEAD emits one divide and stores its own
        // result AND the partner's.
        if matches!(
            op,
            IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem
        ) {
            if self.divrem_tail_dests.contains(&dest.0)
                && !self.divrem_broken_tails.contains(&dest.0)
            {
                if std::env::var_os("CCC_DEBUG_DIVREM").is_some() {
                    eprintln!("[DIVREM] tail-skip dest={}", dest.0);
                }
                // Tail: nothing to emit. The accumulator cache makes no
                // claim about this dest (it was never staged through %rax),
                // and its home/slot was written at the head.
                return;
            }
            if self.divrem_broken_tails.contains(&dest.0) {
                // Pair was broken at head-emission time (pathological home
                // combination): both sides emit standalone divisions.
                if std::env::var_os("CCC_DEBUG_DIVREM").is_some() {
                    eprintln!("[DIVREM] tail-broken dest={} (standalone)", dest.0);
                }
            } else if let Some(&(partner_dest, partner_from_eax)) =
                self.divrem_head_partners.get(&dest.0)
            {
                if std::env::var_os("CCC_DEBUG_DIVREM").is_some() {
                    eprintln!(
                        "[DIVREM] head-emit dest={} op={:?} partner={} partner_from_eax={}",
                        dest.0, op, partner_dest, partner_from_eax
                    );
                }
                let fused = self.emit_divrem_pair_head(
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                    partner_dest,
                    partner_from_eax,
                );
                if fused {
                    return;
                }
                // Broken pair: fall through to the standalone path; the
                // partner (tail) was marked broken above.
                if std::env::var_os("CCC_DEBUG_DIVREM").is_some() {
                    eprintln!("[DIVREM] head-broken dest={} (standalone)", dest.0);
                }
            }
        }

        // Register-direct path. An XMM-homed dest (integer value the RA
        // parked in an XMM register: bit-punned float words) has no GPR
        // sub-register names — emit_alu_reg_direct would hit
        // phys_reg_name_32's unreachable!(). The accumulator path below
        // handles XMM homes on both loads (operand_to_rax) and the final
        // store (store_rax_to).
        if let Some(dest_phys) = self.dest_reg(dest).filter(|r| !super::emit::is_xmm_reg(*r)) {
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
                            // Width guard: the 64-bit `opq %reg/$imm, mem`
                            // forms read AND write 8 bytes. A 4-byte small
                            // slot must only be touched by the 32-bit form;
                            // fall through to the register path otherwise
                            // (I8/I16 ALU ops have use_32bit == false).
                            let dest_is_small = self.state.is_small_slot(dest.0);
                            if !dest_is_small || use_32bit {
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
                                        self.state.emit_fmt(format_args!(
                                            "    {}l ${}, {}",
                                            mnem, imm, sref
                                        ));
                                    } else {
                                        self.state.emit_fmt(format_args!(
                                            "    {}q ${}, {}",
                                            mnem, imm, sref
                                        ));
                                    }
                                    self.invalidate_cache_for_values(&[dest.0, lhs_val.0]);
                                    return;
                                }
                            } // width guard
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
                // Width guard: the 64-bit `opq mem, %rax` form reads 8 bytes;
                // a 4-byte small slot may only be folded with the 32-bit form.
                if self.dest_reg(&rhs_val).is_none()
                    && !self.state.is_alloca(rhs_val.0)
                    && (use_32bit || !self.state.is_small_slot(rhs_val.0))
                {
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
                    // Width guard mirrors the rhs path: the 64-bit form reads
                    // 8 bytes from the slot, which a small slot cannot serve.
                    if self.dest_reg(&lhs_val).is_none()
                        && !self.state.is_alloca(lhs_val.0)
                        && (use_32bit || !self.state.is_small_slot(lhs_val.0))
                    {
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

        // General case: load lhs to rax, rhs to rcx.
        //
        // Two source-direct shortcuts avoid the rcx staging copy entirely:
        //   1. Commutative-eligible immediates (Add/Sub/And/Or/Xor and the
        //      `imull $imm` form above) emit `op $imm, %eax` directly.
        //   2. A register-homed rhs (not rax itself — lhs just landed there)
        //      emits `op %rhs, %eax` straight from its home register.
        // Before this, every scalar add in a loop paid `movl %rN, %ecx`
        // (loop_patterns prefix-sum remainder: 9 → 7 instructions/element).
        // Div/rem keep the rcx contract (idiv reads it); shifts need %cl.
        let alu_reg_direct: Option<String> = match (op, rhs) {
            (
                IrBinOp::Add
                | IrBinOp::Sub
                | IrBinOp::Mul
                | IrBinOp::And
                | IrBinOp::Or
                | IrBinOp::Xor,
                Operand::Value(v),
            ) => self
                .reg_assignments
                .get(&v.0)
                .copied()
                // Only NON-SCRATCH homes qualify: the emitter uses rax/rcx/rdx
                // as internal scratch (Select's `movq $0,%rcx; cmoveq`,
                // accumulator staging), which can clobber a statically-homed
                // value between its def and this use — `reg_assignments` is a
                // point-insensitive map, so trusting a scratch-register home
                // here miscompiled `(h^v)*K` when K was homed %rcx
                // (bitops_builtins: mix() returned garbage). Callee-saved and
                // rsi/rdi/r8-r11 homes are RA-managed end-to-end and safe.
                .filter(|&r| {
                    let name = super::emit::phys_reg_name(r);
                    !super::emit::is_xmm_reg(r) && name != "rax" && name != "rcx" && name != "rdx"
                })
                .map(|r| {
                    if use_32bit {
                        super::emit::phys_reg_name_32(r).to_string()
                    } else {
                        super::emit::phys_reg_name(r).to_string()
                    }
                }),
            _ => None,
        };
        let alu_imm_direct: Option<i64> = match op {
            IrBinOp::Add | IrBinOp::Sub | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor => match rhs {
                Operand::Const(c) => c
                    .to_i64()
                    .filter(|&imm| imm >= i32::MIN as i64 && imm <= i32::MAX as i64),
                _ => None,
            },
            _ => None,
        };
        if let Some(reg_name) = alu_reg_direct {
            if use_32bit {
                self.operand_to_eax(lhs);
            } else {
                self.operand_to_rax(lhs);
            }
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
                            .emit_fmt(format_args!("    {}l %{}, %eax", mnem, reg_name));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    {}q %{}, %rax", mnem, reg_name));
                    }
                }
                IrBinOp::And => {
                    if use_32bit {
                        self.state
                            .emit_fmt(format_args!("    andl %{}, %eax", reg_name));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    andq %{}, %rax", reg_name));
                    }
                }
                IrBinOp::Or => {
                    if use_32bit {
                        self.state
                            .emit_fmt(format_args!("    orl %{}, %eax", reg_name));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    orq %{}, %rax", reg_name));
                    }
                }
                IrBinOp::Xor => {
                    if use_32bit {
                        self.state
                            .emit_fmt(format_args!("    xorl %{}, %eax", reg_name));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    xorq %{}, %rax", reg_name));
                    }
                }
                _ => unreachable!("register-direct path gated to ALU ops"),
            }
            self.state.reg_cache.invalidate_acc();
            if use_32bit {
                self.store_eax_to(dest);
            } else {
                self.store_rax_to(dest);
            }
            return;
        }
        if let Some(imm) = alu_imm_direct {
            if use_32bit {
                self.operand_to_eax(lhs);
            } else {
                self.operand_to_rax(lhs);
            }
            let mnem = match op {
                IrBinOp::Add => "add",
                IrBinOp::Sub => "sub",
                IrBinOp::And => "and",
                IrBinOp::Or => "or",
                IrBinOp::Xor => "xor",
                _ => unreachable!("imm-direct path gated to ALU ops"),
            };
            if use_32bit {
                self.state
                    .emit_fmt(format_args!("    {}l ${}, %eax", mnem, imm));
            } else {
                self.state
                    .emit_fmt(format_args!("    {}q ${}, %rax", mnem, imm));
            }
            self.state.reg_cache.invalidate_acc();
            if use_32bit {
                self.store_eax_to(dest);
            } else {
                self.store_rax_to(dest);
            }
            return;
        }
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
                // Count already in %rcx, lhs in %rax: on Intel the VEX
                // form is 1 µop against 2–3 for `shl %cl` (uops.info
                // SHL_R64_CL vs SHLX_R64_R64_R64); on Zen both are 1 µop and
                // the legacy encoding is shorter (tune row decides).
                if self.tune.prefer_shlx(self.bmi2_enabled) {
                    let (x32, x64) = super::emit::shiftx_mnemonic(op);
                    if use_32bit {
                        self.state
                            .emit_fmt(format_args!("    {} %ecx, %eax, %eax", x32));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    {} %rcx, %rax, %rax", x64));
                    }
                } else {
                    let (mnem32, mnem64) = shift_mnemonic(op);
                    if use_32bit {
                        self.state
                            .emit_fmt(format_args!("    {} %cl, %eax", mnem32));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    {} %cl, %rax", mnem64));
                    }
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

        // v12 Fix B improvement 1: 3-operand immediate multiply from a
        // register-homed lhs — `imull $imm, %lhs, %eax` — drops the leading
        // `movl %lhs, %eax` entirely. Only fires when lhs is a register-homed
        // GPR (so the 3-operand form reads it in place) and rhs is a constant
        // that fits in imm32. Order matters: try this BEFORE the memory-source
        // and operand_to_eax paths so the register-homed lhs is not staged.
        if let Operand::Value(lhs_val) = mul_lhs {
            if let Some(lhs_reg) = self
                .dest_reg(lhs_val)
                .filter(|r| !super::emit::is_xmm_reg(*r))
            {
                if let Some(imm) = Self::const_as_imm32_typed(mul_rhs, use_32bit) {
                    if use_32bit {
                        let lhs_name = super::emit::phys_reg_name_32(lhs_reg);
                        // If lhs and eax coincide, the 2-operand form is fine;
                        // otherwise use the 3-operand form to keep lhs intact.
                        if lhs_name == "eax" {
                            self.state
                                .emit_fmt(format_args!("    imull ${}, %eax, %eax", imm));
                        } else {
                            self.state
                                .emit_fmt(format_args!("    imull ${}, %{}, %eax", imm, lhs_name));
                        }
                    } else {
                        let lhs_name = super::emit::phys_reg_name(lhs_reg);
                        if lhs_name == "rax" {
                            self.state
                                .emit_fmt(format_args!("    imulq ${}, %rax, %rax", imm));
                        } else {
                            self.state
                                .emit_fmt(format_args!("    imulq ${}, %{}, %rax", imm, lhs_name));
                        }
                    }
                    self.emit_fused_add_acc(acc, add_dest, use_32bit);
                    return;
                }
            }
        }

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
        // alloca guard the plain binop mem-source path has. Width guard: the
        // 64-bit imulq memory form reads 8 bytes; small slots need the 32-bit
        // form (or no folding at all).
        if let Operand::Value(rhs_val) = mul_rhs {
            if self.dest_reg(rhs_val).is_none()
                && !self.state.is_alloca(rhs_val.0)
                && (use_32bit || !self.state.is_small_slot(rhs_val.0))
            {
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
        // v12 Fix B improvement 2: constant accumulator with a REGISTER-HOMED
        // dest. Adding the immediate directly to %eax (`addl $imm, %eax`) then
        // moving to the dest register avoids staging the constant through the
        // dest (`movq $imm, %dest`; 7 bytes for i64-sign-extended constants, 10
        // for true imm64) before the add. Restricted to register-dest so the
        // memory-dest / GEP-dest paths keep the original, well-exercised store
        // sequencing — the LCG loop (the hot beneficiary) is register-homed by
        // Fix A's precise-span seed.
        if let Some(dest_phys) = self
            .dest_reg(add_dest)
            .filter(|r| !super::emit::is_xmm_reg(*r))
        {
            if let Some(imm) = Self::const_as_imm32_typed(acc, use_32bit) {
                if use_32bit {
                    self.state.emit_fmt(format_args!("    addl ${}, %eax", imm));
                } else {
                    self.state.emit_fmt(format_args!("    addq ${}, %rax", imm));
                }
                let dest_name = super::emit::phys_reg_name_32(dest_phys);
                let dest_name_64 = super::emit::phys_reg_name(dest_phys);
                if use_32bit {
                    if dest_name != "eax" {
                        self.state
                            .emit_fmt(format_args!("    movl %eax, %{}", dest_name));
                    }
                } else if dest_name_64 != "rax" {
                    self.state
                        .emit_fmt(format_args!("    movq %rax, %{}", dest_name_64));
                }
                self.state.reg_cache.invalidate_acc();
                return;
            }

            // Register-dest, non-constant acc: stage acc into dest, then add %eax.
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
