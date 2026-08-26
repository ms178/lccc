use super::*;

fn infer_movext_dst_size(ops: &[Operand]) -> Result<u8, String> {
    if ops.len() != 2 {
        return Err("mov extension requires 2 operands".to_string());
    }
    match &ops[1] {
        Operand::Register(dst) => {
            if is_reg64(&dst.name) {
                Ok(8)
            } else if is_reg32(&dst.name) {
                Ok(4)
            } else if is_reg16(&dst.name) {
                Ok(2)
            } else {
                Err(format!(
                    "mov extension destination must be 16/32/64-bit GP register: {}",
                    dst.name
                ))
            }
        }
        _ => Err("mov extension destination must be a register".to_string()),
    }
}

impl super::InstructionEncoder {
    // ---- Instruction-specific encoders ----

    pub(crate) fn encode_mov(&mut self, ops: &[Operand], size: u8) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("mov requires 2 operands, got {}", ops.len()));
        }

        match (&ops[0], &ops[1]) {
            // mov $imm, %reg
            (Operand::Immediate(imm), Operand::Register(dst)) => {
                self.encode_mov_imm_reg(imm, dst, size)
            }
            // mov %reg, %reg
            (Operand::Register(src), Operand::Register(dst)) => self.encode_mov_rr(src, dst, size),
            // mov mem, %reg
            (Operand::Memory(mem), Operand::Register(dst)) => {
                self.encode_mov_mem_reg(mem, dst, size)
            }
            // mov %reg, mem
            (Operand::Register(src), Operand::Memory(mem)) => {
                self.encode_mov_reg_mem(src, mem, size)
            }
            // mov $imm, mem
            (Operand::Immediate(imm), Operand::Memory(mem)) => {
                self.encode_mov_imm_mem(imm, mem, size)
            }
            // mov label, %reg (label as memory reference)
            (Operand::Label(label), Operand::Register(dst)) => {
                let mem = MemoryOperand {
                    segment: None,
                    displacement: Displacement::Symbol(label.clone()),
                    base: None,
                    index: None,
                    scale: None,
                    mask: None,
                    zeroing: false,
                };
                self.encode_mov_mem_reg(&mem, dst, size)
            }
            // mov %reg, label (store to label address)
            (Operand::Register(src), Operand::Label(label)) => {
                let mem = MemoryOperand {
                    segment: None,
                    displacement: Displacement::Symbol(label.clone()),
                    base: None,
                    index: None,
                    scale: None,
                    mask: None,
                    zeroing: false,
                };
                self.encode_mov_reg_mem(src, &mem, size)
            }
            _ => Err(format!("unsupported mov operand combination: {:?}", ops)),
        }
    }

    pub(crate) fn encode_mov_imm_reg(
        &mut self,
        imm: &ImmediateValue,
        dst: &Register,
        size: u8,
    ) -> Result<(), String> {
        let dst_num = reg_num(&dst.name).ok_or_else(|| format!("bad register: {}", dst.name))?;

        match imm {
            ImmediateValue::Integer(val) => {
                let val = *val;
                if size == 8 {
                    // For 64-bit: if value fits in signed 32-bit, use movq $imm32, %reg (sign-extended)
                    if val >= i32::MIN as i64 && val <= i32::MAX as i64 {
                        self.emit_rex_unary(8, &dst.name);
                        self.bytes.push(0xC7);
                        self.bytes.push(self.modrm(3, 0, dst_num));
                        self.bytes.extend_from_slice(&(val as i32).to_le_bytes());
                    } else if (0..=0xFFFF_FFFFi64).contains(&val) {
                        // A value that fits in UNSIGNED 32 bits needs no 64-bit
                        // immediate at all: writing a 32-bit register zeroes
                        // the upper half of its 64-bit parent, so
                        // `movl $imm32, %eax` leaves exactly imm32 in %rax.
                        // That is 5 bytes (6 with REX.B) against 10 for the
                        // movabs form -- the single largest per-instruction
                        // saving in the whole encoder.
                        //
                        // The sign-extended C7 /0 form CANNOT be used here:
                        // it would load 0xFFFFFFFF_FFFFFFFF for 0xFFFFFFFF.
                        // ICC emits exactly that and is wrong; verified against
                        // GAS 2.47, which decodes `48 c7 c0 ff ff ff ff` as
                        // `mov $0xffffffffffffffff,%rax`.  Only the
                        // zero-extending 32-bit form is both short and correct.
                        let b = needs_rex_ext(&dst.name);
                        if b {
                            self.bytes.push(self.rex(false, false, false, true));
                        }
                        self.bytes.push(0xB8 + (dst_num & 7));
                        self.bytes.extend_from_slice(&(val as u32).to_le_bytes());
                    } else {
                        // Need movabsq for a true 64-bit immediate.
                        let b = needs_rex_ext(&dst.name);
                        self.bytes.push(self.rex(true, false, false, b));
                        self.bytes.push(0xB8 + (dst_num & 7));
                        self.bytes.extend_from_slice(&val.to_le_bytes());
                    }
                } else if size == 4 {
                    // Prefer the B8+rd id form (no modrm byte) — matches GAS
                    // and is 1 byte shorter than C7 /0 for r8-r15.
                    let b = needs_rex_ext(&dst.name);
                    if b {
                        self.bytes.push(self.rex(false, false, false, true));
                    }
                    self.bytes.push(0xB8 + (dst_num & 7));
                    self.bytes.extend_from_slice(&(val as i32).to_le_bytes());
                } else if size == 2 {
                    // 66 B8+rd iw (matches GAS; avoids the modrm byte).
                    self.bytes.push(0x66); // operand size prefix
                    let b = needs_rex_ext(&dst.name);
                    if b {
                        self.bytes.push(self.rex(false, false, false, true));
                    }
                    self.bytes.push(0xB8 + (dst_num & 7));
                    self.bytes.extend_from_slice(&(val as i16).to_le_bytes());
                } else {
                    // 8-bit: B0+r8 ib is 2 bytes (vs 3 for C6 /0) for
                    // AL/CL/DL/BL; with REX it matches C6 length. Matches GAS.
                    //
                    // %spl/%bpl/%sil/%dil encode as 4..7 -- the SAME numbers as
                    // %ah/%ch/%dh/%bh -- and are only reachable when a REX
                    // prefix is present. Emitting B0+r without REX therefore
                    // silently assembles `movb $imm, %dil` as `movb $imm, %bh`,
                    // writing bits 8-15 of RBX. That corrupted a live pointer
                    // (`lea 0x54(%rsp),%rbx; mov $0x50,%bh; mov %dil,(%rbx)`)
                    // and made struct_copy SIGSEGV intermittently at -O0.
                    // needs_rex_ext alone only covers r8b-r15b, so the
                    // mandatory-REX set must be tested as well.
                    if needs_rex_ext(&dst.name) || is_rex_required_8bit(&dst.name) {
                        self.bytes
                            .push(self.rex(false, false, false, needs_rex_ext(&dst.name)));
                        self.bytes.push(0xB0 + (dst_num & 7));
                    } else {
                        self.bytes.push(0xB0 + (dst_num & 7));
                    }
                    self.bytes.push(val as u8);
                }
            }
            ImmediateValue::Symbol(sym) | ImmediateValue::SymbolPlusOffset(sym, _) => {
                // movq $symbol, %reg or movq $(symbol+offset), %reg - load address
                let addend = if let ImmediateValue::SymbolPlusOffset(_, a) = imm {
                    *a
                } else {
                    0
                };
                if size == 8 {
                    // REX.W + C7 /0 id: imm32 sign-extended to 64 bits, so the
                    // address must be in the low 2 GiB (R_X86_64_32S enforces
                    // that at link time).
                    self.emit_rex_unary(8, &dst.name);
                    self.bytes.push(0xC7);
                    self.bytes.push(self.modrm(3, 0, dst_num));
                    self.add_relocation(sym, R_X86_64_32S, addend);
                    self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                } else {
                    // B8+rd id — one byte shorter than C7 /0 because the
                    // destination register folds into the opcode. Same
                    // semantics, less I-cache; GAS picks this form too.
                    self.emit_rex_unary(size, &dst.name);
                    self.bytes.push(0xB8 + (dst_num & 7));
                    self.add_relocation(sym, R_X86_64_32, addend);
                    self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                }
            }
            ImmediateValue::SymbolDiff(sym_a, sym_b) => {
                // head_64.S (compressed boot): `movl $(_bss - startup_32),
                // %ecx` — a label difference as a mov immediate. Both labels
                // live in the SAME object (startup_32 in the .code32 part,
                // _bss at the end), so the difference folds to a constant
                // after layout via the diff-reloc path. GAS emits
                // `b9 <imm32>` with the folded value; the 8-byte movabs
                // form never appears for these (kernel-image offsets are
                // far below 4 GiB).
                if size == 4 {
                    let b = needs_rex_ext(&dst.name);
                    if b {
                        self.bytes.push(self.rex(false, false, false, true));
                    }
                    self.bytes.push(0xB8 + (dst_num & 7));
                    self.add_diff_relocation(sym_a, sym_b, R_X86_64_32, 0);
                    self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                } else if size == 8 {
                    // movabs $sym_a - sym_b, %reg — 64-bit symbol-difference
                    // immediate. arch/x86/mm/mem_encrypt_boot.S emits
                    // `movq $(.L__enc_copy_end - __enc_copy), %rcx` to load
                    // the encrypted-memory copy routine's length. Both labels
                    // live in the same object, so the difference resolves at
                    // link time to a constant; a R_X86_64_64 diff-relocation
                    // folds to the absolute 64-bit value. movabs (REX.W +
                    // B8+rd) is the only 64-bit-immediate mov form.
                    let b = needs_rex_ext(&dst.name);
                    // REX.W (0x48) selects 64-bit operand size; REX.B (the
                    // low REX bit) extends the register field for r8-r15.
                    self.bytes.push(self.rex(true, false, false, b));
                    self.bytes.push(0xB8 + (dst_num & 7));
                    self.add_diff_relocation(sym_a, sym_b, R_X86_64_64, 0);
                    self.bytes.extend_from_slice(&[0; 8]);
                } else {
                    return Err(format!(
                        "symbol-difference mov immediate only supported at 32-bit width (got size {})", size));
                }
            }
            ImmediateValue::SymbolMod(_, _) => {
                Err("unsupported immediate type for mov".to_string())?
            }
        }
        Ok(())
    }

    pub(crate) fn encode_mov_rr(
        &mut self,
        src: &Register,
        dst: &Register,
        size: u8,
    ) -> Result<(), String> {
        let src_num = reg_num(&src.name).ok_or_else(|| format!("bad register: {}", src.name))?;
        let dst_num = reg_num(&dst.name).ok_or_else(|| format!("bad register: {}", dst.name))?;

        if size == 2 {
            self.bytes.push(0x66);
        }
        self.emit_rex_rr(size, &src.name, &dst.name);
        if size == 1 {
            self.bytes.push(0x88);
        } else {
            self.bytes.push(0x89);
        }
        self.bytes.push(self.modrm(3, src_num, dst_num));
        Ok(())
    }

    pub(crate) fn encode_mov_mem_reg(
        &mut self,
        mem: &MemoryOperand,
        dst: &Register,
        size: u8,
    ) -> Result<(), String> {
        let dst_num = reg_num(&dst.name).ok_or_else(|| format!("bad register: {}", dst.name))?;

        // Handle segment prefix
        self.emit_segment_prefix(mem)?;

        if size == 2 {
            self.bytes.push(0x66);
        }
        self.emit_rex_rm(size, &dst.name, mem);
        if size == 1 {
            self.bytes.push(0x8A);
        } else {
            self.bytes.push(0x8B);
        }
        self.encode_modrm_mem(dst_num, mem)
    }

    pub(crate) fn encode_mov_reg_mem(
        &mut self,
        src: &Register,
        mem: &MemoryOperand,
        size: u8,
    ) -> Result<(), String> {
        let src_num = reg_num(&src.name).ok_or_else(|| format!("bad register: {}", src.name))?;

        self.emit_segment_prefix(mem)?;

        if size == 2 {
            self.bytes.push(0x66);
        }
        self.emit_rex_rm(size, &src.name, mem);
        if size == 1 {
            self.bytes.push(0x88);
        } else {
            self.bytes.push(0x89);
        }
        self.encode_modrm_mem(src_num, mem)
    }

    pub(crate) fn encode_mov_imm_mem(
        &mut self,
        imm: &ImmediateValue,
        mem: &MemoryOperand,
        size: u8,
    ) -> Result<(), String> {
        if let ImmediateValue::Integer(v) = imm {
            Self::check_imm32s_q("mov", size, *v)?;
        }
        self.emit_segment_prefix(mem)?;
        if size == 2 {
            self.bytes.push(0x66);
        }
        // Use an empty string for REX calculation since the reg field is /0
        self.emit_rex_rm(if size == 8 { 8 } else { size }, "", mem);
        if size == 1 {
            self.bytes.push(0xC6);
        } else {
            self.bytes.push(0xC7);
        }
        let reloc_count = self.relocations.len();
        self.encode_modrm_mem(0, mem)?;

        let trailing = match size {
            1 => 1,
            2 => 2,
            _ => 4,
        };
        match imm {
            ImmediateValue::Integer(val) => match size {
                1 => self.bytes.push(*val as u8),
                2 => self.bytes.extend_from_slice(&(*val as i16).to_le_bytes()),
                4 | 8 => self.bytes.extend_from_slice(&(*val as i32).to_le_bytes()),
                _ => unreachable!(),
            },
            ImmediateValue::Symbol(sym) | ImmediateValue::SymbolPlusOffset(sym, _) => {
                let addend = if let ImmediateValue::SymbolPlusOffset(_, a) = imm {
                    *a
                } else {
                    0
                };
                if size >= 4 {
                    // movq uses R_X86_64_32S because the 32-bit immediate is sign-extended
                    // to 64 bits; movl uses R_X86_64_32 (unsigned, no sign extension).
                    let reloc_type = if size == 8 { R_X86_64_32S } else { R_X86_64_32 };
                    self.add_relocation(sym, reloc_type, addend);
                    self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                } else {
                    return Err(
                        "symbol immediate only supported for 32/64-bit mov to memory".to_string(),
                    );
                }
            }
            _ => return Err("unsupported immediate for mov to memory".to_string()),
        }
        self.adjust_rip_reloc_addend(reloc_count, trailing);
        Ok(())
    }

    pub(crate) fn encode_movabs(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("movabsq requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let b = needs_rex_ext(&dst.name);
                self.bytes.push(self.rex(true, false, false, b));
                self.bytes.push(0xB8 + (dst_num & 7));
                self.bytes.extend_from_slice(&val.to_le_bytes());
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::Symbol(sym)), Operand::Register(dst))
            | (
                Operand::Immediate(ImmediateValue::SymbolPlusOffset(sym, _)),
                Operand::Register(dst),
            ) => {
                let addend = match &ops[0] {
                    Operand::Immediate(ImmediateValue::SymbolPlusOffset(_, a)) => *a,
                    _ => 0,
                };
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let b = needs_rex_ext(&dst.name);
                self.bytes.push(self.rex(true, false, false, b));
                self.bytes.push(0xB8 + (dst_num & 7));
                self.add_relocation(sym, R_X86_64_64, addend);
                self.bytes.extend_from_slice(&[0u8; 8]);
                Ok(())
            }
            // moffs forms: `movabs 0xADDR, %al/%ax/%eax/%rax` (A0/A1) and the
            // store direction `movabs %rax, 0xADDR` (A2/A3).  These are the only
            // instructions that take a full 64-bit absolute address, and they
            // are hard-wired to the accumulator, so no ModRM byte is emitted.
            (Operand::Memory(mem), Operand::Register(dst))
                if mem.base.is_none() && mem.index.is_none() && reg_num(&dst.name) == Some(0) =>
            {
                let addr = match &mem.displacement {
                    Displacement::Integer(v) => *v,
                    _ => return Err("movabs moffs requires an absolute address".to_string()),
                };
                let size = infer_reg_size(&dst.name);
                if size == 2 {
                    self.bytes.push(0x66);
                }
                if size == 8 {
                    self.bytes.push(self.rex(true, false, false, false));
                }
                self.bytes.push(if size == 1 { 0xA0 } else { 0xA1 });
                self.bytes.extend_from_slice(&addr.to_le_bytes());
                Ok(())
            }
            (Operand::Register(src), Operand::Memory(mem))
                if mem.base.is_none() && mem.index.is_none() && reg_num(&src.name) == Some(0) =>
            {
                let addr = match &mem.displacement {
                    Displacement::Integer(v) => *v,
                    _ => return Err("movabs moffs requires an absolute address".to_string()),
                };
                let size = infer_reg_size(&src.name);
                if size == 2 {
                    self.bytes.push(0x66);
                }
                if size == 8 {
                    self.bytes.push(self.rex(true, false, false, false));
                }
                self.bytes.push(if size == 1 { 0xA2 } else { 0xA3 });
                self.bytes.extend_from_slice(&addr.to_le_bytes());
                Ok(())
            }
            _ => Err("unsupported movabsq operands".to_string()),
        }
    }

    pub(crate) fn encode_movsx(
        &mut self,
        ops: &[Operand],
        src_size: u8,
        dst_size: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("movsx requires 2 operands".to_string());
        }

        let opcode = match (src_size, dst_size) {
            (1, _) => vec![0x0F, 0xBE], // movsbq/movsbl/movsbw
            (2, _) => vec![0x0F, 0xBF], // movswq/movswl
            (4, 8) => vec![0x63],       // movslq (movsxd)
            _ => {
                return Err(format!(
                    "unsupported movsx combination: {} -> {}",
                    src_size, dst_size
                ))
            }
        };

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                // 16-bit destination needs operand-size override prefix
                if dst_size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rr(dst_size, &dst.name, &src.name);
                self.bytes.extend_from_slice(&opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                self.emit_segment_prefix(mem)?;
                if dst_size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(dst_size, &dst.name, mem);
                self.bytes.extend_from_slice(&opcode);
                self.encode_modrm_mem(dst_num, mem)?;
            }
            _ => return Err("unsupported movsx operands".to_string()),
        }
        Ok(())
    }

    pub(crate) fn encode_movzx(
        &mut self,
        ops: &[Operand],
        src_size: u8,
        dst_size: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("movzx requires 2 operands".to_string());
        }

        let opcode = match src_size {
            1 => vec![0x0F, 0xB6], // movzbl/movzbq/movzbw
            2 => vec![0x0F, 0xB7], // movzwl/movzwq
            _ => return Err(format!("unsupported movzx src size: {}", src_size)),
        };

        // Note: movzbl zero-extends to 64 bits implicitly (32-bit op clears upper 32)
        // So we use size=4 for REX calculation unless dst is an extended register needing REX.B
        let rex_size = if dst_size == 8 { 8 } else { 4 };

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                // 16-bit destination needs operand-size override prefix
                if dst_size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rr(rex_size, &dst.name, &src.name);
                self.bytes.extend_from_slice(&opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                self.emit_segment_prefix(mem)?;
                if dst_size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(rex_size, &dst.name, mem);
                self.bytes.extend_from_slice(&opcode);
                self.encode_modrm_mem(dst_num, mem)?;
            }
            // Absolute address given as a bare label: `movzwl sym, %eax`.
            // The parser yields Operand::Label rather than a MemoryOperand, so
            // without this arm the instruction was rejected outright. Reuse the
            // existing synthesize-a-MemoryOperand idiom (same as `mov`) so the
            // RIP-relative / absolute decision stays in one place.
            (Operand::Label(label), Operand::Register(dst)) => {
                let mem = MemoryOperand {
                    segment: None,
                    displacement: Displacement::Symbol(label.clone()),
                    base: None,
                    index: None,
                    scale: None,
                    mask: None,
                    zeroing: false,
                };
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                if dst_size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(rex_size, &dst.name, &mem);
                self.bytes.extend_from_slice(&opcode);
                self.encode_modrm_mem(dst_num, &mem)?;
            }
            _ => return Err("unsupported movzx operands".to_string()),
        }
        Ok(())
    }

    /// Encode GAS source-size-only sign-extension aliases such as
    /// `movsb mem, %r10` and `movsw mem, %r10`. GAS infers the destination
    /// width from the destination register.
    pub(crate) fn encode_movsx_infer_dst(
        &mut self,
        ops: &[Operand],
        src_size: u8,
    ) -> Result<(), String> {
        let dst_size = infer_movext_dst_size(ops)?;
        self.encode_movsx(ops, src_size, dst_size)
    }

    /// Encode GAS source-size-only zero-extension aliases such as
    /// `movzb mem, %r10` and `movzw mem, %r10`.
    pub(crate) fn encode_movzx_infer_dst(
        &mut self,
        ops: &[Operand],
        src_size: u8,
    ) -> Result<(), String> {
        let dst_size = infer_movext_dst_size(ops)?;
        self.encode_movzx(ops, src_size, dst_size)
    }

    pub(crate) fn encode_lea(&mut self, ops: &[Operand], size: u8) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("lea requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                // Segment override (TLS initial-exec: `g_tls@TPOFF(%fs:0)`).
                // lea was missing emit_segment_prefix, silently dropping %fs/
                // %gs and miscompiling every TLS read through lea.
                self.emit_segment_prefix(mem)?;
                self.emit_rex_rm(size, &dst.name, mem);
                self.bytes.push(0x8D);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("lea requires memory source and register destination".to_string()),
        }
    }

    pub(crate) fn encode_push(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("push requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                if needs_rex_ext(&reg.name) {
                    self.bytes.push(self.rex(false, false, false, true));
                }
                self.bytes.push(0x50 + (num & 7));
                Ok(())
            }
            Operand::Immediate(ImmediateValue::Integer(val)) => {
                // push imm is a 64-bit operation in long mode: imm32 is
                // sign-extended, so the same faithfulness gate applies.
                Self::check_imm32s_q("push", 8, *val)?;
                if *val >= -128 && *val <= 127 {
                    self.bytes.push(0x6A);
                    self.bytes.push(*val as u8);
                } else {
                    self.bytes.push(0x68);
                    self.bytes.extend_from_slice(&(*val as i32).to_le_bytes());
                }
                Ok(())
            }
            Operand::Immediate(ImmediateValue::Symbol(sym))
            | Operand::Immediate(ImmediateValue::SymbolPlusOffset(sym, _)) => {
                // pushq $symbol or pushq $(symbol+offset)
                let addend =
                    if let Operand::Immediate(ImmediateValue::SymbolPlusOffset(_, a)) = &ops[0] {
                        *a
                    } else {
                        0
                    };
                self.bytes.push(0x68);
                self.add_relocation(sym, R_X86_64_32S, addend);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            Operand::Memory(mem) => {
                self.emit_segment_prefix(mem)?;
                self.emit_rex_rm(0, "", mem);
                self.bytes.push(0xFF);
                self.encode_modrm_mem(6, mem)
            }
            _ => Err("unsupported push operand".to_string()),
        }
    }

    pub(crate) fn encode_pop(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("pop requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                if needs_rex_ext(&reg.name) {
                    self.bytes.push(self.rex(false, false, false, true));
                }
                self.bytes.push(0x58 + (num & 7));
                Ok(())
            }
            Operand::Memory(mem) => {
                // pop to memory: 8F /0
                self.emit_segment_prefix(mem)?;
                self.emit_rex_rm(0, "", mem);
                self.bytes.push(0x8F);
                self.encode_modrm_mem(0, mem)
            }
            _ => Err("unsupported pop operand".to_string()),
        }
    }

    /// Encode ALU operations (add/or/adc/sbb/and/sub/xor/cmp).
    /// `alu_op` is the operation number (0-7).
    pub(crate) fn encode_alu(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
        alu_op: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }

        let size = mnemonic_size_suffix(mnemonic).unwrap_or(8);

        match (&ops[0], &ops[1]) {
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(dst)) => {
                let val = *val;
                Self::check_imm32s_q(mnemonic, size, val)?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;

                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_unary(size, &dst.name);

                if size == 1 {
                    // 8-bit ALU with imm8. Prefer the AL short form
                    // (04+op*8 ib, 2 bytes) when the destination is AL —
                    // matches GAS (e.g. `and $0x80,%al` → `24 80`, not
                    // `80 e0 80`).
                    if dst_num == 0 && !needs_rex_ext(&dst.name) {
                        self.bytes.push(0x04 + alu_op * 8);
                        self.bytes.push(val as u8);
                    } else {
                        self.bytes.push(0x80);
                        self.bytes.push(self.modrm(3, alu_op, dst_num));
                        self.bytes.push(val as u8);
                    }
                } else if fits_imm8(val, size) {
                    // Sign-extended imm8 (3-byte `83` form). The immediate is
                    // canonicalized to the operand width first, so both the
                    // signed and unsigned spellings of the same value pick the
                    // compact encoding: `addw $65535,%ax` and `addw $-1,%ax`
                    // are the same 16-bit value and both become `66 83 c0 ff`,
                    // matching GAS.
                    self.bytes.push(0x83);
                    self.bytes.push(self.modrm(3, alu_op, dst_num));
                    self.bytes.push(canonical_imm(val, size) as u8);
                } else {
                    // imm32
                    if dst_num == 0 && !needs_rex_ext(&dst.name) {
                        // Special short form: op eax/rax, imm32
                        self.bytes
                            .push(if size == 1 { 0x04 } else { 0x05 } + alu_op * 8);
                    } else {
                        self.bytes.push(0x81);
                        self.bytes.push(self.modrm(3, alu_op, dst_num));
                    }
                    if size == 2 {
                        self.bytes.extend_from_slice(&(val as i16).to_le_bytes());
                    } else {
                        self.bytes.extend_from_slice(&(val as i32).to_le_bytes());
                    }
                }
                Ok(())
            }
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;

                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rr(size, &src.name, &dst.name);
                self.bytes
                    .push(if size == 1 { 0x00 } else { 0x01 } + alu_op * 8);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                self.emit_segment_prefix(mem)?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, &dst.name, mem);
                self.bytes
                    .push(if size == 1 { 0x02 } else { 0x03 } + alu_op * 8);
                self.encode_modrm_mem(dst_num, mem)
            }
            (Operand::Label(label), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                let mem = MemoryOperand {
                    segment: None,
                    displacement: Displacement::Symbol(label.clone()),
                    base: None,
                    index: None,
                    scale: None,
                    mask: None,
                    zeroing: false,
                };
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, &dst.name, &mem);
                self.bytes
                    .push(if size == 1 { 0x02 } else { 0x03 } + alu_op * 8);
                self.encode_modrm_mem(dst_num, &mem)
            }
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                self.emit_segment_prefix(mem)?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, &src.name, mem);
                self.bytes
                    .push(if size == 1 { 0x00 } else { 0x01 } + alu_op * 8);
                self.encode_modrm_mem(src_num, mem)
            }
            (Operand::Register(src), Operand::Label(label)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let mem = MemoryOperand {
                    segment: None,
                    displacement: Displacement::Symbol(label.clone()),
                    base: None,
                    index: None,
                    scale: None,
                    mask: None,
                    zeroing: false,
                };
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, &src.name, &mem);
                self.bytes
                    .push(if size == 1 { 0x00 } else { 0x01 } + alu_op * 8);
                self.encode_modrm_mem(src_num, &mem)
            }
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Memory(mem)) => {
                let val = *val;
                Self::check_imm32s_q(mnemonic, size, val)?;
                self.emit_segment_prefix(mem)?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, "", mem);

                if size == 1 {
                    let rc = self.relocations.len();
                    self.bytes.push(0x80);
                    self.encode_modrm_mem(alu_op, mem)?;
                    self.bytes.push(val as u8);
                    self.adjust_rip_reloc_addend(rc, 1);
                } else if fits_imm8(val, size) {
                    // Same width-canonical imm8 rule as the register form.
                    let rc = self.relocations.len();
                    self.bytes.push(0x83);
                    self.encode_modrm_mem(alu_op, mem)?;
                    self.bytes.push(canonical_imm(val, size) as u8);
                    self.adjust_rip_reloc_addend(rc, 1);
                } else {
                    let rc = self.relocations.len();
                    self.bytes.push(0x81);
                    self.encode_modrm_mem(alu_op, mem)?;
                    let trailing: i64 = if size == 2 { 2 } else { 4 };
                    if size == 2 {
                        self.bytes.extend_from_slice(&(val as i16).to_le_bytes());
                    } else {
                        self.bytes.extend_from_slice(&(val as i32).to_le_bytes());
                    }
                    self.adjust_rip_reloc_addend(rc, trailing);
                }
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::Symbol(sym)), Operand::Memory(mem))
            | (
                Operand::Immediate(ImmediateValue::SymbolPlusOffset(sym, _)),
                Operand::Memory(mem),
            ) => {
                let addend = match &ops[0] {
                    Operand::Immediate(ImmediateValue::SymbolPlusOffset(_, a)) => *a,
                    _ => 0,
                };
                self.emit_segment_prefix(mem)?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, "", mem);
                let rc = self.relocations.len();
                self.bytes.push(0x81);
                self.encode_modrm_mem(alu_op, mem)?;
                // Emit 4-byte relocation for the symbol immediate.
                // Use bytes.len() (instruction-relative offset) since
                // elf_writer_common adds the section base offset separately.
                self.add_relocation(sym, R_X86_64_32S, addend);
                self.bytes.extend_from_slice(&[0; 4]);
                self.adjust_rip_reloc_addend(rc, 4);
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::Symbol(sym)), Operand::Register(dst))
            | (
                Operand::Immediate(ImmediateValue::SymbolPlusOffset(sym, _)),
                Operand::Register(dst),
            ) => {
                let addend = match &ops[0] {
                    Operand::Immediate(ImmediateValue::SymbolPlusOffset(_, a)) => *a,
                    _ => 0,
                };
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_unary(size, &dst.name);
                self.bytes.push(0x81);
                self.bytes.push(self.modrm(3, alu_op, dst_num));
                // Use instruction-relative offset; elf_writer_common adds section base.
                self.add_relocation(sym, R_X86_64_32S, addend);
                self.bytes.extend_from_slice(&[0; 4]);
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::SymbolDiff(sym, diff)), Operand::Register(dst)) => {
                // `addq $identity_mapped - 0b, %rsi` (relocate_kernel_64.S):
                // GAS reserves imm32 for a forward-referenced difference and
                // resolves it after layout. Same-section pairs become a plain
                // constant; when `sym` stays external the writer converts to
                // R_X86_64_PC32 against `sym` with the addend adjusted by the
                // local label's position (value = S - addr(diff)) — exactly
                // the reloc GAS emits for this shape.
                //
                // Deviation note: for a BACKWARD-defined same-section pair
                // GAS folds at parse time and may pick the shorter imm8 form;
                // we always emit imm32. Semantically identical, one byte
                // larger — acceptable because the kernel's uses are forward
                // references where GAS also emits imm32.
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_unary(size, &dst.name);
                self.bytes.push(0x81);
                self.bytes.push(self.modrm(3, alu_op, dst_num));
                self.add_diff_relocation(sym, diff, R_X86_64_PC32, 0);
                self.bytes.extend_from_slice(&[0; 4]);
                Ok(())
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    pub(crate) fn encode_test(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }

        let size = mnemonic_size_suffix(mnemonic).unwrap_or(8);

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rr(size, &src.name, &dst.name);
                self.bytes.push(if size == 1 { 0x84 } else { 0x85 });
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(dst)) => {
                let val = *val;
                Self::check_imm32s_q(mnemonic, size, val)?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_unary(size, &dst.name);

                if size == 1 {
                    if dst_num == 0 && !needs_rex_ext(&dst.name) {
                        self.bytes.push(0xA8);
                    } else {
                        self.bytes.push(0xF6);
                        self.bytes.push(self.modrm(3, 0, dst_num));
                    }
                    self.bytes.push(val as u8);
                } else {
                    if dst_num == 0 && !needs_rex_ext(&dst.name) {
                        self.bytes.push(0xA9);
                    } else {
                        self.bytes.push(0xF7);
                        self.bytes.push(self.modrm(3, 0, dst_num));
                    }
                    if size == 2 {
                        self.bytes.extend_from_slice(&(val as i16).to_le_bytes());
                    } else {
                        self.bytes.extend_from_slice(&(val as i32).to_le_bytes());
                    }
                }
                Ok(())
            }
            // test %reg, mem -> TEST mem, reg (AT&T: src=reg, dst=mem)
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                self.emit_segment_prefix(mem)?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, &src.name, mem);
                self.bytes.push(if size == 1 { 0x84 } else { 0x85 });
                self.encode_modrm_mem(src_num, mem)
            }
            // test mem, %reg -- the AT&T source/destination order reversed.
            //
            // TEST has no 8A/8B-style "load" direction: only 84 /r and 85 /r
            // exist, both with the register in ModRM.reg and the memory operand
            // in r/m. TEST is also symmetric (it discards the AND result and
            // only sets flags), so GNU as encodes `testb (%rcx), %dil` and
            // `testb %dil, (%rcx)` to the exact same bytes -- verified against
            // GAS 2.47 for all four operand sizes. Without this arm the valid
            // memory-source form was rejected outright.
            (Operand::Memory(mem), Operand::Register(reg)) => {
                let reg_n = reg_num(&reg.name).ok_or("bad register")?;
                self.emit_segment_prefix(mem)?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, &reg.name, mem);
                self.bytes.push(if size == 1 { 0x84 } else { 0x85 });
                self.encode_modrm_mem(reg_n, mem)
            }
            // test $imm, mem
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Memory(mem)) => {
                Self::check_imm32s_q(mnemonic, size, *val)?;
                let val = *val;
                self.emit_segment_prefix(mem)?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, "", mem);
                self.bytes.push(if size == 1 { 0xF6 } else { 0xF7 });
                let rc = self.relocations.len();
                self.encode_modrm_mem(0, mem)?;
                let trailing: i64 = if size == 1 {
                    1
                } else if size == 2 {
                    2
                } else {
                    4
                };
                if size == 1 {
                    self.bytes.push(val as u8);
                } else if size == 2 {
                    self.bytes.extend_from_slice(&(val as i16).to_le_bytes());
                } else {
                    self.bytes.extend_from_slice(&(val as i32).to_le_bytes());
                }
                self.adjust_rip_reloc_addend(rc, trailing);
                Ok(())
            }
            _ => Err("unsupported test operands".to_string()),
        }
    }

    /// `nop r/m16` / `nop r/m32` — the 0F 1F /0 multi-byte NOP forms.
    pub(crate) fn encode_nop_rm(&mut self, ops: &[Operand], word: bool) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("nop with operand requires exactly 1 operand".to_string());
        }
        if word {
            self.bytes.push(0x66);
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_segment_prefix(mem)?;
                self.emit_rex_rm(if word { 2 } else { 4 }, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0x1F]);
                self.encode_modrm_mem(0, mem)
            }
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                self.emit_rex_unary(if word { 2 } else { 4 }, &reg.name);
                self.bytes.extend_from_slice(&[0x0F, 0x1F]);
                self.bytes.push(self.modrm(3, 0, num));
                Ok(())
            }
            _ => Err("nop operand must be register or memory".to_string()),
        }
    }

    /// Emit the immediate tail of an `imul` (`0x69`) form.
    ///
    /// The immediate is operand-sized: imm16 for a 16-bit operand, imm32
    /// otherwise. Emitting imm32 for a 16-bit operand — as the previous
    /// implementation did — desynchronizes the instruction stream by two
    /// bytes and corrupts everything after it.
    fn push_imul_imm(&mut self, val: i64, size: u8) {
        if size == 2 {
            self.bytes
                .extend_from_slice(&(canonical_imm(val, 2) as i16).to_le_bytes());
        } else {
            self.bytes
                .extend_from_slice(&(canonical_imm(val, 4) as i32).to_le_bytes());
        }
    }

    pub(crate) fn encode_imul(&mut self, ops: &[Operand], size: u8) -> Result<(), String> {
        match ops.len() {
            1 => {
                // The one-operand form is a plain unary r/m. It needs the
                // 0x66 operand-size prefix for 16-bit operands just like
                // mul/div/neg/not do; routing through encode_imul previously
                // skipped it, so `imulw %bx` encoded as the 32-bit `imull`.
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.encode_unary_rm(ops, 5, size)
            }
            2 => match (&ops[0], &ops[1]) {
                (Operand::Register(src), Operand::Register(dst)) => {
                    let src_num = reg_num(&src.name).ok_or("bad register")?;
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    if size == 2 {
                        self.bytes.push(0x66);
                    }
                    self.emit_rex_rr(size, &dst.name, &src.name);
                    self.bytes.extend_from_slice(&[0x0F, 0xAF]);
                    self.bytes.push(self.modrm(3, dst_num, src_num));
                    Ok(())
                }
                (Operand::Memory(mem), Operand::Register(dst)) => {
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    self.emit_segment_prefix(mem)?;
                    if size == 2 {
                        self.bytes.push(0x66);
                    }
                    self.emit_rex_rm(size, &dst.name, mem);
                    self.bytes.extend_from_slice(&[0x0F, 0xAF]);
                    self.encode_modrm_mem(dst_num, mem)
                }
                (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(dst)) => {
                    let val = *val;
                    Self::check_imm32s_q("imul", size, val)?;
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    if size == 2 {
                        self.bytes.push(0x66);
                    }
                    self.emit_rex_rr(size, &dst.name, &dst.name);
                    if fits_imm8(val, size) {
                        self.bytes.push(0x6B);
                        self.bytes.push(self.modrm(3, dst_num, dst_num));
                        self.bytes.push(canonical_imm(val, size) as u8);
                    } else {
                        self.bytes.push(0x69);
                        self.bytes.push(self.modrm(3, dst_num, dst_num));
                        self.push_imul_imm(val, size);
                    }
                    Ok(())
                }
                _ => Err("unsupported imul operands".to_string()),
            },
            3 => {
                let (val, dst) = match (&ops[0], &ops[2]) {
                    (Operand::Immediate(ImmediateValue::Integer(v)), Operand::Register(d)) => {
                        (*v, d)
                    }
                    _ => return Err("unsupported imul operands".to_string()),
                };
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                Self::check_imm32s_q("imul", size, val)?;
                let short = fits_imm8(val, size);
                match &ops[1] {
                    Operand::Register(src) => {
                        let src_num = reg_num(&src.name).ok_or("bad register")?;
                        if size == 2 {
                            self.bytes.push(0x66);
                        }
                        self.emit_rex_rr(size, &dst.name, &src.name);
                        self.bytes.push(if short { 0x6B } else { 0x69 });
                        self.bytes.push(self.modrm(3, dst_num, src_num));
                    }
                    Operand::Memory(mem) => {
                        self.emit_segment_prefix(mem)?;
                        if size == 2 {
                            self.bytes.push(0x66);
                        }
                        self.emit_rex_rm(size, &dst.name, mem);
                        let rc = self.relocations.len();
                        self.bytes.push(if short { 0x6B } else { 0x69 });
                        self.encode_modrm_mem(dst_num, mem)?;
                        let trailing: i64 = if short {
                            1
                        } else if size == 2 {
                            2
                        } else {
                            4
                        };
                        self.adjust_rip_reloc_addend(rc, trailing);
                    }
                    _ => return Err("unsupported imul operands".to_string()),
                }
                if short {
                    self.bytes.push(canonical_imm(val, size) as u8);
                } else {
                    self.push_imul_imm(val, size);
                }
                Ok(())
            }
            _ => Err("imul requires 1-3 operands".to_string()),
        }
    }

    pub(crate) fn encode_unary_rm(
        &mut self,
        ops: &[Operand],
        op_ext: u8,
        size: u8,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("unary op requires 1 operand".to_string());
        }
        // inc (op_ext=0) and dec (op_ext=1) use FE/FF, not F6/F7
        let base_opcode = if op_ext <= 1 {
            if size == 1 {
                0xFE
            } else {
                0xFF
            }
        } else if size == 1 {
            0xF6
        } else {
            0xF7
        };
        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                self.emit_rex_unary(size, &reg.name);
                self.bytes.push(base_opcode);
                self.bytes.push(self.modrm(3, op_ext, num));
                Ok(())
            }
            Operand::Memory(mem) => {
                self.emit_rex_rm(size, "", mem);
                self.bytes.push(base_opcode);
                self.encode_modrm_mem(op_ext, mem)
            }
            _ => Err("unsupported unary operand".to_string()),
        }
    }

    /// Encode INC/DEC using Group 5 opcode (0xFE/0xFF), not Group 3 (0xF6/0xF7).
    pub(crate) fn encode_inc_dec(
        &mut self,
        ops: &[Operand],
        op_ext: u8,
        size: u8,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("inc/dec requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_unary(size, &reg.name);
                self.bytes.push(if size == 1 { 0xFE } else { 0xFF });
                self.bytes.push(self.modrm(3, op_ext, num));
                Ok(())
            }
            Operand::Memory(mem) => {
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, "", mem);
                self.bytes.push(if size == 1 { 0xFE } else { 0xFF });
                self.encode_modrm_mem(op_ext, mem)
            }
            _ => Err("unsupported inc/dec operand".to_string()),
        }
    }

    /// GNU as's shift/rotate immediate acceptance, derived empirically
    /// against binutils 2.4x (insndiff FALSE-ACCEPT / REJECTS-VALID oracle):
    ///   * 0..=255 is accepted everywhere (raw unsigned imm8 field);
    ///   * -128..=-1 is additionally accepted when the immediate fits the
    ///     DESTINATION width as signed — i.e. for every 8-bit-operand form
    ///     (`shlb $-1, %al` is fine) — and for rol/ror at ALL widths (their
    ///     binutils template carries Imm8S; a negative rotate is meaningful
    ///     modulo the width);
    ///   * everything else is "operand type mismatch".
    /// Silently truncating an out-of-range count would encode an
    /// instruction the programmer did not write.
    /// GAS 2.47 parity + wrong-code prevention: a 64-bit operation's 32-bit
    /// immediate field is SIGN-extended by the CPU, so only values
    /// representable as i32 can be encoded faithfully. `$0xffffffff` in
    /// `andq $0xffffffff, %rax` would silently become `$-1`
    /// (0xffffffffffffffff) — a value change, not an encoding choice.
    /// binutils rejects these ("operand type mismatch"); we reject with a
    /// message that says what actually went wrong and what to use instead.
    /// (ICC 2021 SILENTLY mis-encodes exactly this case — the defect class
    /// this check exists to prevent.)
    fn check_imm32s_q(mnemonic: &str, size: u8, val: i64) -> Result<(), String> {
        if size == 8 && (val > i32::MAX as i64 || val < i32::MIN as i64) {
            return Err(format!(
                "{}: immediate 0x{:x} does not fit the sign-extended 32-bit \
                 immediate field of a 64-bit operation (it would silently \
                 change value); use a register, movl zero-extension, or movabs",
                mnemonic, val
            ));
        }
        Ok(())
    }

    fn check_shift_imm(mnemonic: &str, size: u8, count: i64) -> Result<(), String> {
        // 8-bit operand forms: GNU as accepts ANY immediate and masks it to
        // the low byte (silent inside -128..=255, warn-and-truncate outside
        // — but never a hard error). Byte destinations therefore skip the
        // range check entirely; the caller's `as u8` performs the same
        // wrap GAS encodes.
        if size == 1 {
            return Ok(());
        }
        if mnemonic.starts_with("rol") || mnemonic.starts_with("ror") {
            // rol/ror carry Imm8S. GAS's acceptance windows, probed
            // empirically against binutils 2.4x:
            //   * the raw value -128..=255, and
            //   * the top unsigned band of the OPERAND width,
            //     2^bits-128 ..= 2^bits-1 (`rolw $65535` is -1 in 16-bit
            //     unsigned clothing; `rolw $65407` = -129 is refused, and
            //     values whose low bits merely COLLAPSE into range, like
            //     $-2^31 at 16-bit, are refused too — GAS does not mask).
            // 64-bit operands need no extra band: 2^64-128.. wraps negative
            // in the i64 the parser produced.
            let in_basic = (-128..=255).contains(&count);
            let in_top_band = size < 8 && {
                let bits = (size as u32) * 8;
                let top = 1i64 << bits;
                count >= top - 128 && count < top
            };
            if !in_basic && !in_top_band {
                return Err(format!(
                    "operand type mismatch for `{}' (count {})",
                    mnemonic, count
                ));
            }
            return Ok(());
        }
        // shl/shr/sal/sar/rcl/rcr at 16/32/64-bit widths: unsigned imm8 only.
        if !(0..=255).contains(&count) {
            return Err(format!(
                "operand type mismatch for `{}' (count {} outside 0..=255)",
                mnemonic, count
            ));
        }
        Ok(())
    }

    pub(crate) fn encode_shift(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
        shift_op: u8,
    ) -> Result<(), String> {
        let size = mnemonic_size_suffix(mnemonic).unwrap_or(8);

        // Handle 1-operand form: shift by 1 implicitly
        if ops.len() == 1 {
            match &ops[0] {
                Operand::Register(dst) => {
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    if size == 2 {
                        self.bytes.push(0x66);
                    }
                    self.emit_rex_unary(size, &dst.name);
                    self.bytes.push(if size == 1 { 0xD0 } else { 0xD1 });
                    self.bytes.push(self.modrm(3, shift_op, dst_num));
                    return Ok(());
                }
                Operand::Memory(mem) => {
                    if size == 2 {
                        self.bytes.push(0x66);
                    }
                    self.emit_rex_rm(size, "", mem);
                    self.bytes.push(if size == 1 { 0xD0 } else { 0xD1 });
                    return self.encode_modrm_mem(shift_op, mem);
                }
                _ => return Err(format!("unsupported {} operand", mnemonic)),
            }
        }

        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }

        match (&ops[0], &ops[1]) {
            (Operand::Immediate(ImmediateValue::Integer(count)), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                Self::check_shift_imm(mnemonic, size as u8, *count)?;
                let count = *count as u8;

                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_unary(size, &dst.name);

                if count == 1 {
                    self.bytes.push(if size == 1 { 0xD0 } else { 0xD1 });
                    self.bytes.push(self.modrm(3, shift_op, dst_num));
                } else {
                    self.bytes.push(if size == 1 { 0xC0 } else { 0xC1 });
                    self.bytes.push(self.modrm(3, shift_op, dst_num));
                    self.bytes.push(count);
                }
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::Integer(count)), Operand::Memory(mem)) => {
                Self::check_shift_imm(mnemonic, size as u8, *count)?;
                let count = *count as u8;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, "", mem);
                if count == 1 {
                    self.bytes.push(if size == 1 { 0xD0 } else { 0xD1 });
                    self.encode_modrm_mem(shift_op, mem)
                } else {
                    let rc = self.relocations.len();
                    self.bytes.push(if size == 1 { 0xC0 } else { 0xC1 });
                    self.encode_modrm_mem(shift_op, mem)?;
                    self.bytes.push(count);
                    self.adjust_rip_reloc_addend(rc, 1);
                    Ok(())
                }
            }
            (Operand::Register(cl), Operand::Register(dst)) if cl.name == "cl" => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_unary(size, &dst.name);
                self.bytes.push(if size == 1 { 0xD2 } else { 0xD3 });
                self.bytes.push(self.modrm(3, shift_op, dst_num));
                Ok(())
            }
            (Operand::Register(cl), Operand::Memory(mem)) if cl.name == "cl" => {
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, "", mem);
                self.bytes.push(if size == 1 { 0xD2 } else { 0xD3 });
                self.encode_modrm_mem(shift_op, mem)
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    pub(crate) fn encode_double_shift(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        size: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("double shift requires 3 operands".to_string());
        }

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(count)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.emit_rex_rr(size, &src.name, &dst.name);
                self.bytes.extend_from_slice(&[0x0F, opcode]);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                self.bytes.push(*count as u8);
                Ok(())
            }
            (Operand::Register(cl), Operand::Register(src), Operand::Register(dst))
                if cl.name == "cl" =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.emit_rex_rr(size, &src.name, &dst.name);
                self.bytes.extend_from_slice(&[0x0F, opcode + 1]);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            _ => Err("unsupported double shift operands".to_string()),
        }
    }

    pub(crate) fn encode_bswap(&mut self, ops: &[Operand], size: u8) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("bswap requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                self.emit_rex_unary(size, &reg.name);
                self.bytes.extend_from_slice(&[0x0F, 0xC8 + (num & 7)]);
                Ok(())
            }
            _ => Err("bswap requires register operand".to_string()),
        }
    }

    pub(crate) fn encode_bit_count(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }

        let (prefix, opcode) = match mnemonic {
            "lzcntl" | "lzcntq" | "lzcntw" => (0xF3u8, [0x0F, 0xBD]),
            "tzcntl" | "tzcntq" | "tzcntw" => (0xF3, [0x0F, 0xBC]),
            "popcntl" | "popcntq" | "popcntw" => (0xF3, [0x0F, 0xB8]),
            _ => return Err(format!("unknown bit count: {}", mnemonic)),
        };

        let size = mnemonic_size_suffix(mnemonic).unwrap_or(4);

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                } // operand-size override for 16-bit
                self.bytes.push(prefix);
                self.emit_rex_rr(size, &dst.name, &src.name);
                self.bytes.extend_from_slice(&opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.bytes.push(prefix);
                self.emit_rex_rm(size, &dst.name, mem);
                self.bytes.extend_from_slice(&opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    pub(crate) fn encode_setcc(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("setcc requires 1 operand".to_string());
        }

        let cc_str = &mnemonic[3..];
        // Try the condition code as-is first, then strip trailing 'b' suffix
        let cc = cc_from_mnemonic(cc_str).or_else(|_| {
            if let Some(stripped) = cc_str.strip_suffix('b') {
                cc_from_mnemonic(stripped)
            } else {
                Err(format!("unknown condition code: {}", cc_str))
            }
        })?;

        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                if needs_rex_ext(&reg.name) || is_rex_required_8bit(&reg.name) {
                    self.bytes
                        .push(self.rex(false, false, false, needs_rex_ext(&reg.name)));
                }
                self.bytes.extend_from_slice(&[0x0F, 0x90 + cc]);
                self.bytes.push(self.modrm(3, 0, num));
                Ok(())
            }
            Operand::Memory(mem) => {
                self.emit_rex_rm(1, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0x90 + cc]);
                self.encode_modrm_mem(0, mem)
            }
            _ => Err("setcc requires register or memory operand".to_string()),
        }
    }

    pub(crate) fn encode_cmovcc(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("cmovcc requires 2 operands".to_string());
        }

        // Extract condition code: strip "cmov" prefix and size suffix
        let without_prefix = &mnemonic[4..];
        let (cc_str, size) = if let Some(stripped) = without_prefix.strip_suffix('q') {
            (stripped, 8u8)
        } else if let Some(stripped) = without_prefix.strip_suffix('l') {
            (stripped, 4u8)
        } else if let Some(stripped) = without_prefix.strip_suffix('w') {
            (stripped, 2u8)
        } else {
            (without_prefix, 8u8) // default to 64-bit
        };
        let cc = cc_from_mnemonic(cc_str)?;

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rr(size, &dst.name, &src.name);
                self.bytes.extend_from_slice(&[0x0F, 0x40 + cc]);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, &dst.name, mem);
                self.bytes.extend_from_slice(&[0x0F, 0x40 + cc]);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported cmov operands".to_string()),
        }
    }

    pub(crate) fn encode_jmp(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("jmp requires 1 operand".to_string());
        }

        match &ops[0] {
            Operand::Label(label) => {
                // Near jump with 32-bit displacement (will be resolved by linker/relocator)
                // Always use R_X86_64_PLT32 for branch targets, matching modern GCC/binutils.
                // R_X86_64_PC32 is rejected by ld for PIE executables calling shared lib functions.
                self.bytes.push(0xE9);
                let sym = label.strip_suffix("@PLT").unwrap_or(label.as_str());
                let reloc_type = R_X86_64_PLT32;
                self.add_relocation(sym, reloc_type, -4);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            Operand::Indirect(inner) => match inner.as_ref() {
                Operand::Register(reg) => {
                    let num = reg_num(&reg.name).ok_or("bad register")?;
                    if needs_rex_ext(&reg.name) {
                        self.bytes.push(self.rex(false, false, false, true));
                    }
                    self.bytes.push(0xFF);
                    self.bytes.push(self.modrm(3, 4, num));
                    Ok(())
                }
                Operand::Memory(mem) => {
                    self.emit_rex_rm(0, "", mem);
                    self.bytes.push(0xFF);
                    self.encode_modrm_mem(4, mem)
                }
                _ => Err("unsupported indirect jmp target".to_string()),
            },
            _ => Err("unsupported jmp operand".to_string()),
        }
    }

    pub(crate) fn encode_jcc(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("jcc requires 1 operand".to_string());
        }

        let cc = cc_from_mnemonic(&mnemonic[1..])?;

        match &ops[0] {
            Operand::Label(label) => {
                // Near jcc with 32-bit displacement
                // Strip @PLT suffix and use PLT32 relocation (matches GCC behavior)
                self.bytes.extend_from_slice(&[0x0F, 0x80 + cc]);
                let reloc_type = R_X86_64_PLT32;
                let sym = label.strip_suffix("@PLT").unwrap_or(label);
                self.add_relocation(sym, reloc_type, -4);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            _ => Err("jcc requires label operand".to_string()),
        }
    }

    pub(crate) fn encode_call(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("call requires 1 operand".to_string());
        }

        match &ops[0] {
            Operand::Label(label) => {
                self.bytes.push(0xE8);
                // Use PLT32 for external function calls (linker will resolve)
                let reloc_type = R_X86_64_PLT32;
                let sym = label.strip_suffix("@PLT").unwrap_or(label.as_str());
                self.add_relocation(sym, reloc_type, -4);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            Operand::Indirect(inner) => {
                match inner.as_ref() {
                    Operand::Register(reg) => {
                        let num = reg_num(&reg.name).ok_or("bad register")?;
                        if needs_rex_ext(&reg.name) {
                            self.bytes.push(self.rex(false, false, false, true));
                        }
                        self.bytes.push(0xFF);
                        self.bytes.push(self.modrm(3, 2, num));
                        Ok(())
                    }
                    Operand::Memory(mem) => {
                        // call *disp(%base) - FF /2 with memory operand
                        self.emit_rex_rm(0, "", mem);
                        self.bytes.push(0xFF);
                        self.encode_modrm_mem(2, mem)
                    }
                    _ => Err("unsupported indirect call target".to_string()),
                }
            }
            _ => Err("unsupported call operand".to_string()),
        }
    }

    pub(crate) fn encode_xchg(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("xchg requires 2 operands".to_string());
        }
        let size = mnemonic_size_suffix(mnemonic).unwrap_or(8);

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Memory(mem))
            | (Operand::Memory(mem), Operand::Register(src)) => {
                // xchg is symmetric: `xchgl (%rdx), %ecx` == `xchgl %ecx, (%rdx)`.
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, &src.name, mem);
                self.bytes.push(if size == 1 { 0x86 } else { 0x87 });
                self.encode_modrm_mem(src_num, mem)
            }
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;

                // `xchg` with the accumulator has a one-byte form, 0x90+rd, and
                // GAS always prefers it: `xchg %rax,%rcx` -> 48 91 rather than
                // the 3-byte ModRM form.  Two exceptions, both verified against
                // GAS 2.47:
                //   * 8-bit `xchg %al,%cl` has no short form at all (86 c1).
                //   * `xchg %eax,%eax` must NOT become 0x90, because in 64-bit
                //     mode the bare 0x90 is NOP and does not zero-extend EAX
                //     into RAX the way a real 32-bit xchg would.  GAS emits
                //     87 c0.  `xchg %rax,%rax` and `xchg %ax,%ax` are safe
                //     (48 90 -> canonical 90, and 66 90).
                if size != 1 {
                    let other = if src_num == 0 && !needs_rex_ext(&src.name) {
                        Some(&dst.name)
                    } else if dst_num == 0 && !needs_rex_ext(&dst.name) {
                        Some(&src.name)
                    } else {
                        None
                    };
                    if let Some(other) = other {
                        let other_is_eax =
                            size == 4 && reg_num(other) == Some(0) && !needs_rex_ext(other);
                        if !other_is_eax {
                            let onum = reg_num(other).ok_or("bad register")?;
                            if size == 2 {
                                self.bytes.push(0x66);
                            }
                            // `xchg %rax,%rax` needs no REX.W: exchanging the
                            // accumulator with itself is a no-op either way, so
                            // GAS folds it to the canonical one-byte NOP.
                            if !(size == 8 && onum == 0 && !needs_rex_ext(other)) {
                                self.emit_rex_unary(size, other);
                            }
                            self.bytes.push(0x90 + (onum & 7));
                            return Ok(());
                        }
                    }
                }

                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rr(size, &src.name, &dst.name);
                self.bytes.push(if size == 1 { 0x86 } else { 0x87 });
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            _ => Err("unsupported xchg operands".to_string()),
        }
    }

    pub(crate) fn encode_cmpxchg(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("cmpxchg requires 2 operands".to_string());
        }
        let size = mnemonic_size_suffix(mnemonic).unwrap_or(8);

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, &src.name, mem);
                self.bytes
                    .extend_from_slice(&[0x0F, if size == 1 { 0xB0 } else { 0xB1 }]);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("unsupported cmpxchg operands".to_string()),
        }
    }

    pub(crate) fn encode_xadd(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("xadd requires 2 operands".to_string());
        }
        let size = mnemonic_size_suffix(mnemonic).unwrap_or(8);

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, &src.name, mem);
                self.bytes
                    .extend_from_slice(&[0x0F, if size == 1 { 0xC0 } else { 0xC1 }]);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("unsupported xadd operands".to_string()),
        }
    }
    // ---- CET shadow-stack family (Intel CET / SHSTK) ----
    // Encodings verified against GNU binutils 2.44 (AT&T syntax):
    //   rstorssp m64      F3 0F 01 /5
    //   saveprevssp       F3 0F 01 EA        (fixed, no ModRM)
    //   setssbsy          F3 0F 01 E8        (fixed, no ModRM)
    //   clrssbsy m64      F3 0F AE /6        (memory operand, mod != 11)
    //   wrssd r32, m32    0F 38 F6 /r
    //   wrssq r64, m64    REX.W 0F 38 F6 /r
    //   wrussd r32, m32   66 0F 38 F5 /r
    //   wrussq r64, m64   66 REX.W 0F 38 F5 /r

    pub(crate) fn encode_rstorssp(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("rstorssp requires 1 memory operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_segment_prefix(mem)?;
                self.bytes.extend_from_slice(&[0xF3, 0x0F, 0x01]);
                self.encode_modrm_mem(5, mem) // /5
            }
            _ => Err("rstorssp requires a memory operand".to_string()),
        }
    }

    pub(crate) fn encode_clrssbsy(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("clrssbsy requires 1 memory operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_segment_prefix(mem)?;
                self.bytes.extend_from_slice(&[0xF3, 0x0F, 0xAE]);
                self.encode_modrm_mem(6, mem) // /6
            }
            _ => Err("clrssbsy requires a memory operand".to_string()),
        }
    }

    /// WRSSD/WRSSQ/WRUSSD/WRUSSQ: store to shadow stack (reg -> memory).
    /// `is_user` selects the WRUSS variant (66-prefixed, opcode F5 vs F6).
    pub(crate) fn encode_wrss(
        &mut self,
        ops: &[Operand],
        size: u8,
        is_user: bool,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("wrss/wruss requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                self.emit_segment_prefix(mem)?;
                if is_user {
                    self.bytes.push(0x66); // 66 operand-size prefix (WRUSS only)
                }
                self.emit_rex_rm(size, &src.name, mem);
                self.bytes
                    .extend_from_slice(&[0x0F, 0x38, if is_user { 0xF5 } else { 0xF6 }]);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("wrss/wruss requires register, memory operands".to_string()),
        }
    }
}
