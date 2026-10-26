//! i686 InlineAsmEmitter implementation: constraint classification, register
//! allocation, operand loading/storing, and template substitution for inline asm.
//!
//! Handles 32-bit x86 registers (eax, ebx, ecx, edx, esi, edi) and i686
//! calling conventions (cdecl, ILP32).

use super::emit::I686Codegen;
use crate::backend::inline_asm::{AsmOperand, AsmOperandKind, InlineAsmEmitter};
use crate::backend::state::CodegenState;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{BlockId, IrConst, Operand, Value};
use std::borrow::Cow;

/// i686 scratch XMM registers (SSE available on most i686 targets).
const I686_XMM_SCRATCH: &[&str] = &[
    "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7",
];

impl InlineAsmEmitter for I686Codegen {
    fn asm_state(&mut self) -> &mut CodegenState {
        &mut self.state
    }

    fn classify_constraint(&self, constraint: &str) -> AsmOperandKind {
        let c = constraint.trim_start_matches(['=', '+', '&', '%']);
        // Explicit register constraint: {regname}
        if c.starts_with('{') && c.ends_with('}') {
            let reg_name = &c[1..c.len() - 1];
            return AsmOperandKind::Specific(reg_name.to_string());
        }
        // GCC condition code output: =@cc<cond>
        if let Some(cond) = c.strip_prefix("@cc") {
            return AsmOperandKind::ConditionCode(cond.to_string());
        }
        // Tied operand (all digits)
        if !c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit()) {
            if let Ok(n) = c.parse::<usize>() {
                return AsmOperandKind::Tied(n);
            }
        }
        // x87 FPU stack constraints: "t" = st(0), "u" = st(1)
        if c == "t" {
            return AsmOperandKind::X87St0;
        }
        if c == "u" {
            return AsmOperandKind::X87St1;
        }

        // Multi-alternative constraint parsing (same logic as x86-64 but with 32-bit registers)
        let mut has_gp = false;
        let mut has_fp = false;
        let mut has_mem = false;
        let mut has_imm = false;
        let mut specific: Option<String> = None;

        for ch in c.chars() {
            match ch {
                'r' | 'q' | 'R' | 'Q' | 'l' => has_gp = true,
                'g' => {
                    has_gp = true;
                    has_mem = true;
                    has_imm = true;
                }
                'x' | 'v' | 'Y' => has_fp = true,
                'm' | 'o' | 'V' | 'p' => has_mem = true,
                'i' | 'I' | 'n' | 'N' | 'e' | 'E' | 'K' | 'M' | 'G' | 'H' | 'J' | 'L' | 'O' => {
                    has_imm = true
                }
                'a' if specific.is_none() => specific = Some("eax".to_string()),
                'b' if specific.is_none() => specific = Some("ebx".to_string()),
                'c' if specific.is_none() => specific = Some("ecx".to_string()),
                'd' if specific.is_none() => specific = Some("edx".to_string()),
                'S' if specific.is_none() => specific = Some("esi".to_string()),
                'D' if specific.is_none() => specific = Some("edi".to_string()),
                // GCC "A" constraint on x86-32: edx:eax register pair (`reg` = eax, `reg_hi` = edx).
                'A' if specific.is_none() => specific = Some("eax".to_string()),
                _ => {}
            }
        }

        if let Some(reg) = specific {
            AsmOperandKind::Specific(reg)
        } else if has_gp {
            AsmOperandKind::GpReg
        } else if has_fp {
            AsmOperandKind::FpReg
        } else if has_mem {
            AsmOperandKind::Memory
        } else if has_imm {
            AsmOperandKind::Immediate
        } else {
            AsmOperandKind::GpReg
        }
    }

    fn setup_operand_metadata(&self, op: &mut AsmOperand, val: &Operand, _is_output: bool) {
        if op
            .constraint
            .trim_start_matches(['=', '+', '&', '%'])
            .contains('A')
        {
            op.reg_hi = "edx".to_string();
        }
        if matches!(op.kind, AsmOperandKind::Memory) {
            if let Operand::Value(v) = val {
                if let Some(slot) = self.state.get_slot(v.0) {
                    if self.state.is_alloca(v.0) {
                        if self.state.alloca_over_align(v.0).is_some() {
                            op.mem_addr = String::new();
                        } else {
                            op.mem_addr = self.slot_ref(slot);
                        }
                    } else {
                        op.mem_addr = String::new();
                    }
                }
            }
        }
        if matches!(op.kind, AsmOperandKind::Immediate) {
            if let Operand::Const(c) = val {
                op.imm_value = c.to_i64();
            }
        }
    }

    fn fallback_scratch_reg(&mut self, kind: &AsmOperandKind) -> String {
        match kind {
            AsmOperandKind::FpReg => "xmm0".to_string(),
            _ => "ecx".to_string(),
        }
    }

    fn resolve_memory_operand(
        &mut self,
        op: &mut AsmOperand,
        val: &Operand,
        excluded: &[String],
    ) -> Option<String> {
        if !op.mem_addr.is_empty() {
            return None;
        }
        if let Some(ref sym) = op.imm_symbol {
            op.mem_addr = sym.clone();
            return None;
        }
        let alloc_gp_tmp = |this: &mut Self| -> String {
            let mut r = this.assign_scratch_reg(&AsmOperandKind::GpReg, excluded);
            if r.is_empty() {
                r = this.fallback_scratch_reg(&AsmOperandKind::GpReg);
            }
            if r == "eax" {
                this.state.reg_cache.invalidate_acc();
            }
            r
        };
        match val {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    let tmp_reg = alloc_gp_tmp(self);
                    if self.state.is_alloca(v.0) {
                        let sr = self.slot_ref(slot);
                        if let Some(align) = self.state.alloca_over_align(v.0) {
                            emit!(self.state, "    leal {}, %{}", sr, tmp_reg);
                            emit!(self.state, "    addl ${}, %{}", align - 1, tmp_reg);
                            emit!(self.state, "    andl ${}, %{}", -(align as i32), tmp_reg);
                        } else {
                            emit!(self.state, "    leal {}, %{}", sr, tmp_reg);
                        }
                    } else if let Some(&phys) = self.reg_assignments.get(&v.0) {
                        let src = super::emit::phys_reg_name(phys);
                        if src != tmp_reg {
                            emit!(self.state, "    movl %{}, %{}", src, tmp_reg);
                        }
                    } else {
                        let sr = self.slot_ref(slot);
                        emit!(self.state, "    movl {}, %{}", sr, tmp_reg);
                    }
                    op.mem_addr = format!("(%{})", tmp_reg);
                    return Some(tmp_reg);
                } else if let Some(&phys) = self.reg_assignments.get(&v.0) {
                    let tmp_reg = alloc_gp_tmp(self);
                    let src = super::emit::phys_reg_name(phys);
                    if src != tmp_reg {
                        emit!(self.state, "    movl %{}, %{}", src, tmp_reg);
                    }
                    op.mem_addr = format!("(%{})", tmp_reg);
                    return Some(tmp_reg);
                } else if self.state.reg_cache.acc_has(v.0, false) {
                    let tmp_reg = alloc_gp_tmp(self);
                    if tmp_reg != "eax" {
                        emit!(self.state, "    movl %eax, %{}", tmp_reg);
                    }
                    op.mem_addr = format!("(%{})", tmp_reg);
                    return Some(tmp_reg);
                } else {
                    assert!(
                        !op.mem_addr.is_empty(),
                        "i686 codegen: inline-asm memory operand value {} has neither \
                         stack slot nor register assignment",
                        v.0
                    );
                }
            }
            Operand::Const(c) => {
                if let Some(addr) = c.to_i64() {
                    let tmp_reg = alloc_gp_tmp(self);
                    emit!(self.state, "    movl ${}, %{}", addr as i32, tmp_reg);
                    op.mem_addr = format!("(%{})", tmp_reg);
                    return Some(tmp_reg);
                }
            }
        }
        None
    }

    fn assign_scratch_reg(&mut self, kind: &AsmOperandKind, excluded: &[String]) -> String {
        if matches!(kind, AsmOperandKind::FpReg) {
            // i686 only has xmm0-xmm7 (no xmm8-xmm15 without 64-bit mode).
            // Skip excluded registers but cap at 8.
            loop {
                let idx = self.asm_xmm_scratch_idx;
                self.asm_xmm_scratch_idx += 1;
                if idx >= I686_XMM_SCRATCH.len() {
                    for r in I686_XMM_SCRATCH {
                        if !excluded.iter().any(|e| e == *r) {
                            return r.to_string();
                        }
                    }
                    return String::new();
                }
                let reg = I686_XMM_SCRATCH[idx].to_string();
                if !excluded.iter().any(|e| e == &reg) {
                    return reg;
                }
            }
        } else {
            // Prefer GP registers that are NOT live RA homes (`esi/edi/ebx`) first;
            // only touch RA-assigned callee-saved registers on a second pass if
            // all unallocated candidates are excluded (note that `collect_inline_asm_callee_saved_i686`
            // already withholds `esi/edi/ebx` from RA whenever scratch demand > avail).
            const ALL_GP: &[&str] = &["ecx", "edx", "esi", "edi", "eax", "ebx"];
            while self.asm_scratch_idx < ALL_GP.len() * 2 {
                let idx = self.asm_scratch_idx;
                self.asm_scratch_idx += 1;
                let allow_ra_home = idx >= ALL_GP.len();
                let reg_name = ALL_GP[idx % ALL_GP.len()];
                // The GOT is a backend-owned live value, not an SSA RA home.
                // Generic asm scratch must honor the same reservation as RA;
                // otherwise an input silently destroys later GOTOFF/PLT uses.
                if self.pic_got_live && reg_name == "ebx" {
                    continue;
                }
                if excluded.iter().any(|e| e == reg_name) {
                    continue;
                }
                let in_ra = self
                    .reg_assignments
                    .values()
                    .any(|&p| super::emit::phys_reg_name(p) == reg_name);
                if in_ra != allow_ra_home {
                    continue;
                }
                return reg_name.to_string();
            }
            // All registers excluded — return empty to signal spill-to-memory.
            String::new()
        }
    }

    fn load_input_to_reg(
        &mut self,
        op: &AsmOperand,
        val: &Operand,
        _constraint: &str,
        _operands: &[AsmOperand],
    ) {
        let reg = &op.reg;
        let ty = op.operand_type;

        // x87 FPU stack: load value from memory onto the x87 stack with fld
        if matches!(op.kind, AsmOperandKind::X87St0 | AsmOperandKind::X87St1) {
            match val {
                Operand::Value(v) => {
                    if let Some(slot) = self.state.get_slot(v.0) {
                        let fld_instr = match ty {
                            IrType::F32 => "flds",
                            IrType::F128 => "fldt",
                            _ => "fldl", // F64 and default
                        };
                        let sr = self.slot_ref(slot);
                        self.state
                            .emit_fmt(format_args!("    {} {}", fld_instr, sr));
                    }
                }
                Operand::Const(c) => {
                    // Materialize float constant via stack scratch space
                    let bits = match ty {
                        IrType::F32 => {
                            let f = c.to_f64().unwrap_or(0.0) as f32;
                            f.to_bits() as u64
                        }
                        _ => {
                            let f = c.to_f64().unwrap_or(0.0);
                            f.to_bits()
                        }
                    };
                    if ty == IrType::F32 {
                        self.state.emit("    subl $4, %esp");
                        self.state
                            .emit_fmt(format_args!("    movl ${}, (%esp)", bits as u32));
                        self.state.emit("    flds (%esp)");
                        self.state.emit("    addl $4, %esp");
                    } else {
                        let lo = bits as u32;
                        let hi = (bits >> 32) as u32;
                        self.state.emit("    subl $8, %esp");
                        self.state
                            .emit_fmt(format_args!("    movl ${}, (%esp)", lo));
                        self.state
                            .emit_fmt(format_args!("    movl ${}, 4(%esp)", hi));
                        self.state.emit("    fldl (%esp)");
                        self.state.emit("    addl $8, %esp");
                    }
                }
            }
            return;
        }

        let is_xmm = reg.starts_with("xmm");

        if is_xmm {
            match val {
                Operand::Value(v) => {
                    if let Some(slot) = self.state.get_slot(v.0) {
                        let sr = self.slot_ref(slot);
                        let load_instr = match ty {
                            IrType::F32 => "movss",
                            IrType::F64 => "movsd",
                            _ => "movdqu",
                        };
                        self.state
                            .emit_fmt(format_args!("    {} {}, %{}", load_instr, sr, reg));
                    }
                }
                Operand::Const(_) => {
                    self.state
                        .emit_fmt(format_args!("    xorpd %{}, %{}", reg, reg));
                }
            }
            return;
        }

        // GP register - check for 64-bit register pair
        let reg_hi = &op.reg_hi;
        let is_pair = !reg_hi.is_empty() && matches!(ty, IrType::I64 | IrType::U64);

        match val {
            Operand::Const(c) => {
                let imm = match c {
                    IrConst::F32(v) => v.to_bits() as i64,
                    IrConst::F64(v) => v.to_bits() as i64,
                    _ => c.to_i64().unwrap_or(0),
                };
                if is_pair {
                    let lo = imm as u32;
                    let hi = (imm as u64 >> 32) as u32;
                    if lo == 0 {
                        self.state
                            .emit_fmt(format_args!("    xorl %{}, %{}", reg, reg));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    movl ${}, %{}", lo as i32, reg));
                    }
                    if hi == 0 {
                        self.state
                            .emit_fmt(format_args!("    xorl %{}, %{}", reg_hi, reg_hi));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    movl ${}, %{}", hi as i32, reg_hi));
                    }
                } else {
                    let imm32 = imm as i32;
                    if imm32 == 0 {
                        self.state.emit_fmt(format_args!(
                            "    xorl %{}, %{}",
                            Self::reg_to_32(reg),
                            Self::reg_to_32(reg)
                        ));
                    } else {
                        self.state.emit_fmt(format_args!(
                            "    movl ${}, %{}",
                            imm32,
                            Self::reg_to_32(reg)
                        ));
                    }
                }
            }
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    let sr = self.slot_ref(slot);
                    if self.state.is_alloca(v.0) {
                        if let Some(align) = self.state.alloca_over_align(v.0) {
                            self.state
                                .emit_fmt(format_args!("    leal {}, %{}", sr, reg));
                            self.state
                                .emit_fmt(format_args!("    addl ${}, %{}", align - 1, reg));
                            self.state.emit_fmt(format_args!(
                                "    andl ${}, %{}",
                                -(align as i32),
                                reg
                            ));
                        } else {
                            self.state
                                .emit_fmt(format_args!("    leal {}, %{}", sr, reg));
                        }
                    } else if is_pair {
                        let sr4 = self.slot_ref_offset(slot, 4);
                        self.state
                            .emit_fmt(format_args!("    movl {}, %{}", sr, reg));
                        self.state
                            .emit_fmt(format_args!("    movl {}, %{}", sr4, reg_hi));
                    } else {
                        let load_instr = Self::i686_mov_load_for_type(ty);
                        let dest = if Self::is_extending_load(load_instr) {
                            Self::reg_to_32(reg)
                        } else {
                            Self::dest_reg_for_type(reg, ty)
                        };
                        self.state
                            .emit_fmt(format_args!("    {} {}, %{}", load_instr, sr, dest));
                    }
                } else if let Some(&phys) = self.reg_assignments.get(&v.0) {
                    // No stack slot, but register-allocated: the i686 linear
                    // scan is authoritative (the register holds the value
                    // throughout its range — see direct_reg_src_ref), so a
                    // direct mov materialises the asm input. Register pairs
                    // have no single home; refuse loudly rather than load
                    // half the value.
                    if is_pair {
                        panic!(
                            "i686 codegen: inline-asm input value {} (64-bit pair) has \
                             no stack slot — cannot materialise from a single home",
                            v.0
                        );
                    }
                    let src = super::emit::phys_reg_name(phys);
                    self.state
                        .emit_fmt(format_args!("    movl %{}, %{}", src, reg));
                } else {
                    // Neither slot nor register: silently skipping the load
                    // feeds the asm whatever the scratch register held (the
                    // x86-64 OPTIMIZER_HIDE_VAR bug class). Fail loudly.
                    panic!(
                        "i686 codegen: inline-asm input value {} has no stack slot or \
                         register assignment",
                        v.0
                    );
                }
            }
        }
    }

    fn preload_readwrite_output(&mut self, op: &AsmOperand, ptr: &Value) {
        // x87 FPU stack
        if matches!(op.kind, AsmOperandKind::X87St0 | AsmOperandKind::X87St1) {
            let ty = op.operand_type;
            if let Some(slot) = self.state.get_slot(ptr.0) {
                let fld_instr = match ty {
                    IrType::F32 => "flds",
                    IrType::F128 => "fldt",
                    _ => "fldl",
                };
                if self.state.is_direct_slot(ptr.0) {
                    let sr = self.slot_ref(slot);
                    self.state
                        .emit_fmt(format_args!("    {} {}", fld_instr, sr));
                } else {
                    self.state.emit("    pushl %ecx");
                    self.esp_adjust += 4;
                    let sr = self.slot_ref(slot);
                    self.state.emit_fmt(format_args!("    movl {}, %ecx", sr));
                    self.state
                        .emit_fmt(format_args!("    {} (%ecx)", fld_instr));
                    self.state.emit("    popl %ecx");
                    self.esp_adjust -= 4;
                }
            }
            return;
        }
        let reg = &op.reg;
        let ty = op.operand_type;
        let reg_hi = &op.reg_hi;
        let is_pair = !reg_hi.is_empty() && matches!(ty, IrType::I64 | IrType::U64);
        let is_xmm = reg.starts_with("xmm");
        if let Some(slot) = self.state.get_slot(ptr.0) {
            let sr = self.slot_ref(slot);
            if self.state.is_direct_slot(ptr.0) {
                if is_xmm {
                    let load_instr = match ty {
                        IrType::F32 => "movss",
                        IrType::F64 => "movsd",
                        _ => "movdqu",
                    };
                    self.state
                        .emit_fmt(format_args!("    {} {}, %{}", load_instr, sr, reg));
                } else if is_pair {
                    let sr4 = self.slot_ref_offset(slot, 4);
                    self.state
                        .emit_fmt(format_args!("    movl {}, %{}", sr, reg));
                    self.state
                        .emit_fmt(format_args!("    movl {}, %{}", sr4, reg_hi));
                } else {
                    let load_instr = Self::i686_mov_load_for_type(ty);
                    let dest = if Self::is_extending_load(load_instr) {
                        Self::reg_to_32(reg)
                    } else {
                        Self::dest_reg_for_type(reg, ty)
                    };
                    self.state
                        .emit_fmt(format_args!("    {} {}, %{}", load_instr, sr, dest));
                }
            } else {
                // Indirect slot: the slot contains a pointer to the live storage.
                if is_pair {
                    self.state
                        .emit_fmt(format_args!("    movl {}, %{}", sr, reg));
                    self.state
                        .emit_fmt(format_args!("    movl 4(%{}), %{}", reg, reg_hi));
                    self.state
                        .emit_fmt(format_args!("    movl (%{}), %{}", reg, reg));
                } else if is_xmm {
                    let load_instr = match ty {
                        IrType::F32 => "movss",
                        IrType::F64 => "movsd",
                        _ => "movdqu",
                    };
                    self.state.emit("    pushl %ecx");
                    self.esp_adjust += 4;
                    self.state.emit_fmt(format_args!("    movl {}, %ecx", sr));
                    self.state
                        .emit_fmt(format_args!("    {} (%ecx), %{}", load_instr, reg));
                    self.state.emit("    popl %ecx");
                    self.esp_adjust -= 4;
                } else {
                    self.state
                        .emit_fmt(format_args!("    movl {}, %{}", sr, reg));
                    let load_instr = Self::i686_mov_load_for_type(ty);
                    let dest = if Self::is_extending_load(load_instr) {
                        Self::reg_to_32(reg)
                    } else {
                        Self::dest_reg_for_type(reg, ty)
                    };
                    self.state
                        .emit_fmt(format_args!("    {} (%{}), %{}", load_instr, reg, dest));
                }
            }
        }
    }

    fn substitute_template_line(
        &self,
        line: &str,
        operands: &[AsmOperand],
        gcc_to_internal: &[usize],
        operand_types: &[IrType],
        goto_labels: &[(String, BlockId)],
    ) -> String {
        crate::backend::x86_common::assert_memory_operands_resolved(operands, "i686");
        let op_regs: Vec<String> = operands.iter().map(|o| o.reg.clone()).collect();
        let op_names: Vec<Option<String>> = operands.iter().map(|o| o.name.clone()).collect();
        let op_is_memory: Vec<bool> = operands
            .iter()
            .map(|o| matches!(o.kind, AsmOperandKind::Memory))
            .collect();
        let op_mem_addrs: Vec<String> = operands
            .iter()
            .map(|o| {
                if o.seg_prefix.is_empty() {
                    o.mem_addr.clone()
                } else {
                    format!("{}{}", o.seg_prefix, o.mem_addr)
                }
            })
            .collect();
        let op_imm_values: Vec<Option<i64>> = operands.iter().map(|o| o.imm_value).collect();
        let op_imm_symbols: Vec<Option<String>> =
            operands.iter().map(|o| o.imm_symbol.clone()).collect();

        let total = operands.len();
        let mut op_types: Vec<IrType> = vec![IrType::I32; total];
        for (i, ty) in operand_types.iter().enumerate() {
            if i < total {
                op_types[i] = *ty;
            }
        }
        for (i, op) in operands.iter().enumerate() {
            if let AsmOperandKind::Tied(tied_to) = &op.kind {
                if *tied_to < op_types.len() && i < op_types.len() {
                    op_types[i] = op_types[*tied_to];
                }
            }
        }

        // i686 code is never compiled with AVX in lccc: GCC's `%dN`
        // duplicate modifier is the plain operand here.
        let line_owned = crate::backend::x86::codegen::strip_duplicate_modifier(line);
        let line = line_owned.as_str();
        Self::substitute_i686_asm_operands(
            line,
            &op_regs,
            &op_names,
            &op_is_memory,
            &op_mem_addrs,
            &op_types,
            gcc_to_internal,
            goto_labels,
            &op_imm_values,
            &op_imm_symbols,
        )
    }

    fn store_output_from_reg(
        &mut self,
        op: &AsmOperand,
        ptr: &Value,
        _constraint: &str,
        _all_output_regs: &[&str],
    ) {
        if matches!(op.kind, AsmOperandKind::Memory) {
            return;
        }
        // x87 FPU stack outputs
        if matches!(op.kind, AsmOperandKind::X87St0 | AsmOperandKind::X87St1) {
            if let Some(slot) = self.state.get_slot(ptr.0) {
                let ty = op.operand_type;
                let fstp_instr = match ty {
                    IrType::F32 => "fstps",
                    IrType::F128 => "fstpt",
                    _ => "fstpl",
                };
                if self.state.is_direct_slot(ptr.0) {
                    let sr = self.slot_ref(slot);
                    self.state
                        .emit_fmt(format_args!("    {} {}", fstp_instr, sr));
                } else {
                    self.state.emit("    pushl %ecx");
                    self.esp_adjust += 4;
                    let sr = self.slot_ref(slot);
                    self.state.emit_fmt(format_args!("    movl {}, %ecx", sr));
                    self.state
                        .emit_fmt(format_args!("    {} (%ecx)", fstp_instr));
                    self.state.emit("    popl %ecx");
                    self.esp_adjust -= 4;
                }
            }
            return;
        }
        // =@cc<cond> condition code outputs
        if let AsmOperandKind::ConditionCode(ref cond) = op.kind {
            let reg = &op.reg;
            let reg8 = Self::reg_to_8l(reg);
            let x86_cond = Self::gcc_cc_to_x86(cond);
            self.state
                .emit_fmt(format_args!("    set{} %{}", x86_cond, reg8));
            self.state.emit_fmt(format_args!(
                "    movzbl %{}, %{}",
                reg8,
                Self::reg_to_32(reg)
            ));
            if let Some(slot) = self.state.get_slot(ptr.0) {
                let ty = op.operand_type;
                if self.state.is_direct_slot(ptr.0) {
                    let sr = self.slot_ref(slot);
                    if self.output_is_value_home(ptr) {
                        // Promoted value: the full zero-extended dword.
                        self.state.emit_fmt(format_args!(
                            "    movl %{}, {}",
                            Self::reg_to_32(reg),
                            sr
                        ));
                    } else {
                        let store_instr = Self::i686_mov_store_for_type(ty);
                        let src = Self::src_reg_for_type(reg, ty);
                        self.state
                            .emit_fmt(format_args!("    {} %{}, {}", store_instr, src, sr));
                    }
                } else {
                    let scratch = if reg != "ecx" { "ecx" } else { "edx" };
                    self.state.emit_fmt(format_args!("    pushl %{}", scratch));
                    self.esp_adjust += 4;
                    let sr = self.slot_ref(slot);
                    self.state
                        .emit_fmt(format_args!("    movl {}, %{}", sr, scratch));
                    let store_instr = Self::i686_mov_store_for_type(ty);
                    let src = Self::src_reg_for_type(reg, ty);
                    self.state
                        .emit_fmt(format_args!("    {} %{}, (%{})", store_instr, src, scratch));
                    self.state.emit_fmt(format_args!("    popl %{}", scratch));
                    self.esp_adjust -= 4;
                }
            }
            return;
        }

        let reg = &op.reg;
        let ty = op.operand_type;
        let reg_hi = &op.reg_hi;
        let is_pair = !reg_hi.is_empty() && matches!(ty, IrType::I64 | IrType::U64);
        let is_xmm = reg.starts_with("xmm");
        if let Some(slot) = self.state.get_slot(ptr.0) {
            if is_xmm {
                if self.state.is_direct_slot(ptr.0) {
                    let sr = self.slot_ref(slot);
                    let store_instr = match ty {
                        IrType::F32 => "movss",
                        IrType::F64 => "movsd",
                        _ => "movdqu",
                    };
                    self.state
                        .emit_fmt(format_args!("    {} %{}, {}", store_instr, reg, sr));
                } else {
                    let store_instr = match ty {
                        IrType::F32 => "movss",
                        IrType::F64 => "movsd",
                        _ => "movdqu",
                    };
                    self.state.emit("    pushl %ecx");
                    self.esp_adjust += 4;
                    let sr = self.slot_ref(slot);
                    self.state.emit_fmt(format_args!("    movl {}, %ecx", sr));
                    self.state
                        .emit_fmt(format_args!("    {} %{}, (%ecx)", store_instr, reg));
                    self.state.emit("    popl %ecx");
                    self.esp_adjust -= 4;
                }
            } else if is_pair {
                if self.state.is_direct_slot(ptr.0) {
                    let sr = self.slot_ref(slot);
                    let sr4 = self.slot_ref_offset(slot, 4);
                    self.state
                        .emit_fmt(format_args!("    movl %{}, {}", reg, sr));
                    self.state
                        .emit_fmt(format_args!("    movl %{}, {}", reg_hi, sr4));
                } else {
                    let scratch = if reg != "ecx" && reg_hi != "ecx" {
                        "ecx"
                    } else if reg != "edx" && reg_hi != "edx" {
                        "edx"
                    } else {
                        "esi"
                    };
                    self.state.emit_fmt(format_args!("    pushl %{}", scratch));
                    self.esp_adjust += 4;
                    let sr = self.slot_ref(slot);
                    self.state
                        .emit_fmt(format_args!("    movl {}, %{}", sr, scratch));
                    self.state
                        .emit_fmt(format_args!("    movl %{}, (%{})", reg, scratch));
                    self.state
                        .emit_fmt(format_args!("    movl %{}, 4(%{})", reg_hi, scratch));
                    self.state.emit_fmt(format_args!("    popl %{}", scratch));
                    self.esp_adjust -= 4;
                }
            } else if self.state.is_direct_slot(ptr.0) {
                let sr = self.slot_ref(slot);
                let src = Self::src_reg_for_type(reg, ty);
                let extend = Self::i686_mov_load_for_type(ty);
                if Self::is_extending_load(extend) && self.output_is_value_home(ptr) {
                    // A promoted narrow VALUE's home is a 4-byte slot that
                    // every 32-bit reader (`cmpl $0, S` for a typeless
                    // CondBranch, `movl S` reloads) reads whole: store the
                    // extension by the IR type, as a narrow Load would.  The
                    // output register is this operand's own and was already
                    // clobbered by the asm.
                    let r32 = Self::reg_to_32(reg);
                    self.state
                        .emit_fmt(format_args!("    {} %{}, %{}", extend, src, r32));
                    self.state
                        .emit_fmt(format_args!("    movl %{}, {}", r32, sr));
                } else {
                    let store_instr = Self::i686_mov_store_for_type(ty);
                    self.state
                        .emit_fmt(format_args!("    {} %{}, {}", store_instr, src, sr));
                }
            } else {
                let scratch = if reg != "ecx" { "ecx" } else { "edx" };
                self.state.emit_fmt(format_args!("    pushl %{}", scratch));
                self.esp_adjust += 4;
                let sr = self.slot_ref(slot);
                self.state
                    .emit_fmt(format_args!("    movl {}, %{}", sr, scratch));
                let store_instr = Self::i686_mov_store_for_type(ty);
                let src = Self::src_reg_for_type(reg, ty);
                self.state
                    .emit_fmt(format_args!("    {} %{}, (%{})", store_instr, src, scratch));
                self.state.emit_fmt(format_args!("    popl %{}", scratch));
                self.esp_adjust -= 4;
            }
        }
    }

    fn setup_memory_fallback(&self, op: &mut AsmOperand, val: &Operand) {
        // When all GP registers are exhausted (only 6 on i686) and the constraint
        // allows memory (e.g., "g"), fall back to referencing the value's stack slot
        // directly. Unlike setup_operand_metadata for Memory (which handles "m"
        // constraints where the value is an address), here we want the VALUE itself,
        // which lives directly in the stack slot at offset(%ebp) or offset(%esp).
        match val {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    op.mem_addr = self.slot_ref(slot);
                }
            }
            Operand::Const(c) => {
                // Constant value — promote to immediate instead of memory.
                op.kind = AsmOperandKind::Immediate;
                op.imm_value = c.to_i64();
            }
        }
    }

    fn needs_register_pair(&self, ty: IrType) -> bool {
        // On i686, 64-bit integer types need two 32-bit GP registers (a register pair).
        matches!(ty, IrType::I64 | IrType::U64)
    }

    fn reset_scratch_state(&mut self) {
        self.asm_scratch_idx = 0;
        self.asm_xmm_scratch_idx = 0;
    }
}

// Helper methods for i686 inline asm register formatting.
// Register conversion and condition code mapping delegate to the shared
// `x86_common` module to avoid duplication with the x86-64 backend.
impl I686Codegen {
    /// Whether an output's destination is a promoted SSA value's own home
    /// (a full 4-byte scalar slot) rather than user memory: an `Alloca` is
    /// the C object itself (a `char` variable is ONE byte — a wider store
    /// would clobber its neighbours), so only non-alloca direct slots may
    /// take a widened store.
    fn output_is_value_home(&self, ptr: &Value) -> bool {
        !self.state.is_alloca(ptr.0)
            && !self.state.wide_values.contains(&ptr.0)
            && !self.state.f128_direct_slots.contains(&ptr.0)
    }

    /// Return the i686 store mnemonic for a given IR type.
    /// Uses movb/movw for sub-32-bit types, movl for everything else.
    fn i686_mov_store_for_type(ty: IrType) -> &'static str {
        match ty {
            IrType::I8 | IrType::U8 => "movb",
            IrType::I16 | IrType::U16 => "movw",
            _ => "movl",
        }
    }

    /// Return the i686 load mnemonic for a given IR type.
    /// Uses sign/zero-extending loads for sub-32-bit types, movl for everything else.
    fn i686_mov_load_for_type(ty: IrType) -> &'static str {
        match ty {
            IrType::I8 => "movsbl",
            IrType::U8 => "movzbl",
            IrType::I16 => "movswl",
            IrType::U16 => "movzwl",
            _ => "movl",
        }
    }

    /// Convert register to 32-bit variant. Delegates to shared x86_common.
    pub(super) fn reg_to_32<'a>(reg: &'a str) -> Cow<'a, str> {
        crate::backend::x86_common::reg_to_32(reg)
    }

    /// Convert register to 16-bit variant. Delegates to shared x86_common.
    pub(super) fn reg_to_16<'a>(reg: &'a str) -> Cow<'a, str> {
        crate::backend::x86_common::reg_to_16(reg)
    }

    /// Convert register to 8-bit low variant. Delegates to shared x86_common.
    pub(super) fn reg_to_8l<'a>(reg: &'a str) -> Cow<'a, str> {
        crate::backend::x86_common::reg_to_8l(reg)
    }

    /// Map GCC condition code suffix to x86 SETcc suffix. Delegates to shared x86_common.
    pub(super) fn gcc_cc_to_x86(cond: &str) -> &'static str {
        crate::backend::x86_common::gcc_cc_to_x86(cond)
    }

    /// Get the appropriately-sized source register name for a type.
    fn src_reg_for_type<'a>(reg: &'a str, ty: IrType) -> Cow<'a, str> {
        match ty {
            IrType::I8 | IrType::U8 => Self::reg_to_8l(reg),
            IrType::I16 | IrType::U16 => Self::reg_to_16(reg),
            _ => Self::reg_to_32(reg),
        }
    }

    /// Check if a load instruction is a zero/sign-extending load.
    /// These instructions always require a 32-bit destination register.
    fn is_extending_load(instr: &str) -> bool {
        matches!(instr, "movzbl" | "movzwl" | "movsbl" | "movswl")
    }

    /// Get the appropriately-sized destination register name for a type.
    fn dest_reg_for_type<'a>(reg: &'a str, ty: IrType) -> Cow<'a, str> {
        match ty {
            IrType::I8 | IrType::U8 => Self::reg_to_8l(reg),
            IrType::I16 | IrType::U16 => Self::reg_to_16(reg),
            _ => Self::reg_to_32(reg),
        }
    }
}
