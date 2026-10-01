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
        if self.apx_nf {
            return Err("{nf} unsupported for `mov`".to_string());
        }
        if self.apx_evex {
            return Err("no EVEX encoding for `mov`".to_string());
        }
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
                    broadcast: None,
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
                    broadcast: None,
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
                        // That is 5 bytes (6 with REX.B, 7 with REX2) against
                        // 10/11 for the movabs form -- the single largest
                        // per-instruction saving in the whole encoder.
                        //
                        // GAS 2.47 still emits 11-byte REX2-movabs for
                        // `movq $0xffffffff, %r16`; the zero-extending 32-bit
                        // form is both shorter and correct (ICC's C7 /0
                        // sign-extends 0xffffffff to -1).
                        self.emit_rex_unary(4, &dst.name);
                        self.bytes.push(0xB8 + (dst_num & 7));
                        self.bytes.extend_from_slice(&(val as u32).to_le_bytes());
                    } else {
                        // Need movabsq for a true 64-bit immediate.
                        self.emit_rex_unary(8, &dst.name);
                        self.bytes.push(0xB8 + (dst_num & 7));
                        self.bytes.extend_from_slice(&val.to_le_bytes());
                    }
                } else if size == 4 {
                    // Prefer the B8+rd id form (no modrm byte) — matches GAS
                    // and is 1 byte shorter than C7 /0 for r8-r15.
                    self.emit_rex_unary(4, &dst.name);
                    self.bytes.push(0xB8 + (dst_num & 7));
                    self.bytes.extend_from_slice(&(val as i32).to_le_bytes());
                } else if size == 2 {
                    // 66 B8+rd iw (matches GAS; avoids the modrm byte).
                    self.bytes.push(0x66); // operand size prefix
                    self.emit_rex_unary(2, &dst.name);
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
                    // mandatory-REX set must be tested as well. emit_rex_unary
                    // also emits REX2 for %r16b–%r31b.
                    self.emit_rex_unary(1, &dst.name);
                    self.bytes.push(0xB0 + (dst_num & 7));
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
                } else if size == 2 {
                    // `movw $xtrn, %ax` = 66 b8 <iw> + R_X86_64_16 (GAS
                    // 2.47 byte-probed; the old code fell into the 32-bit
                    // arm and wrote EAX with a 4-byte patch).
                    self.bytes.push(0x66);
                    self.emit_rex_unary(2, &dst.name);
                    self.bytes.push(0xB8 + (dst_num & 7));
                    self.add_relocation(sym, R_X86_64_16, addend);
                    self.bytes.extend_from_slice(&[0, 0]);
                } else if size == 1 {
                    // `movb $xtrn, %al` = b0 <ib> + R_X86_64_8 (GAS-probed;
                    // same opcode as the Integer path, including the
                    // mandatory-REX rule for %spl/%bpl/%sil/%dil documented
                    // there). The old code wrote a full EAX load instead.
                    self.emit_rex_unary(1, &dst.name);
                    self.bytes.push(0xB0 + (dst_num & 7));
                    self.add_relocation(sym, R_X86_64_8, addend);
                    self.bytes.push(0);
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
                    self.emit_rex_unary(4, &dst.name);
                    self.bytes.push(0xB8 + (dst_num & 7));
                    self.add_diff_relocation(sym_a, sym_b, R_X86_64_32, 0);
                    self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                } else if size == 8 {
                    // mov r64, imm32 -- 7 bytes against movabs's 10. A label
                    // difference is section-relative by construction (both
                    // labels live in one object, or the right side is the
                    // position counter), so the value always fits a signed
                    // 32-bit and GAS always uses the C7 form here
                    // (byte-probed: `movq $(b - a), %rcx` = 48 c7 c1 02 00
                    // 00 00; `movq $(xtrn - .), %rax` = 48 c7 c0 + PC32
                    // xtrn+0x3). A same-section pair folds to its disp32; an
                    // external-minus-local pair becomes R_X86_64_PC32 against
                    // the external symbol via the diff path (raw addend 0: the
                    // writer's diff pass adds (reloc_offset - b_off), and PC32
                    // then subtracts the place -- exactly GAS's `PC32 xtrn+0x3`
                    // semantics for `xtrn - .`). movabs stays correct but is
                    // 3 dead bytes of I-cache.
                    self.emit_rex_unary(8, &dst.name);
                    self.bytes.push(0xC7);
                    self.bytes.push(self.modrm(3, 0, dst_num));
                    self.add_diff_relocation(sym_a, sym_b, R_X86_64_PC32, 0);
                    self.bytes.extend_from_slice(&[0; 4]);
                } else {
                    return Err(format!(
                        "symbol-difference mov immediate only supported at 32-bit width (got size {})",
                        size
                    ));
                }
            }
            ImmediateValue::SymbolMod(sym, modifier) => {
                // `sym@SIZE`: the section-size pseudo-value. The B8+rd
                // form with an R_X86_64_32 against the `name@SIZE`
                // pseudo-symbol; the writer folds it to a constant when
                // the section is local (GAS byte-parity: `movl $.data@SIZE
                // + 4, %eax` = b8 04 00 00 00).
                if !modifier.eq_ignore_ascii_case("SIZE") {
                    Err("unsupported immediate type for mov".to_string())?
                }
                if size == 8 {
                    Err("movq of @SIZE is not supported".to_string())?
                }
                self.emit_rex_unary(4, &dst.name);
                self.bytes.push(0xB8 + (dst_num & 7));
                self.add_relocation(&format!("{sym}@SIZE"), R_X86_64_32, 0);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
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
        // `.s` selects the load-direction opcode (8A/8B) with the roles
        // swapped: `movl.s %eax,%ebx` = 8b d8, not 89 c3 (GAS-probed) —
        // the alternate encoding of the same move.
        if self.s_flip {
            self.emit_rex_rr(size, &dst.name, &src.name);
            self.bytes.push(if size == 1 { 0x8A } else { 0x8B });
            self.bytes.push(self.modrm(3, dst_num, src_num));
            return Ok(());
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

    /// Moffs address emission: [0x66] [REX.W] A0-A3 + moffs64.
    /// Integer addresses encode inline; symbols take an R_X86_64_64
    /// relocation (GAS 2.47: `movabs foo,%rax` -> `48 a1 0.. + R64`).
    fn emit_moffs_addr(
        &mut self,
        acc: &str,
        disp: &Displacement,
        store: bool,
    ) -> Result<(), String> {
        let size = infer_reg_size(acc);
        if size == 2 {
            self.bytes.push(0x66);
        }
        if size == 8 {
            self.emit_rex_unary(8, acc);
        }
        self.bytes.push(match (store, size == 1) {
            (false, true) => 0xA0,
            (false, false) => 0xA1,
            (true, true) => 0xA2,
            (true, false) => 0xA3,
        });
        match disp {
            Displacement::Integer(v) => self.bytes.extend_from_slice(&v.to_le_bytes()),
            Displacement::Symbol(sym) => {
                self.add_relocation(sym, R_X86_64_64, 0);
                self.bytes.extend_from_slice(&[0u8; 8]);
            }
            Displacement::SymbolPlusOffset(sym, a) => {
                self.add_relocation(sym, R_X86_64_64, *a);
                self.bytes.extend_from_slice(&[0u8; 8]);
            }
            _ => return Err("movabs moffs requires an absolute address".to_string()),
        }
        Ok(())
    }

    /// `addr32`-forced accumulator moffs rows (GAS 2.47): 67 [66] [REX.W]
    /// A0–A3 + moffs32. The `addr32` prefix word moves the bare-absolute
    /// accumulator moves off the SIB body onto the moffs rows with a
    /// 32-bit address field (`addr32 mov %eax,0x600898` = `67 a3 98 08
    /// 60 00`; the rax forms keep REX.W: `67 48 a1 ...`). Integer
    /// addresses only — a symbolic absolute switches the relocation
    /// class (R_X86_64_32/32S), which the body's SIB form encodes
    /// correctly, so symbols stay on the SIB path.
    pub(crate) fn encode_addr32_moffs(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("mov requires 2 operands".to_string());
        }
        let (acc, disp, store) = match (&ops[0], &ops[1]) {
            (Operand::Memory(mem), Operand::Register(r)) => (&r.name, &mem.displacement, false),
            (Operand::Register(r), Operand::Memory(mem)) => (&r.name, &mem.displacement, true),
            _ => return Err("operand type mismatch for `mov'".to_string()),
        };
        if !registers::is_accum(&acc.to_ascii_lowercase()) {
            return Err("operand type mismatch for `mov'".to_string());
        }
        let Displacement::Integer(v) = disp else {
            return Err("operand type mismatch for `mov'".to_string());
        };
        let size = registers::infer_reg_size(acc);
        if size == 2 {
            self.bytes.push(0x66);
        }
        if size == 8 {
            self.emit_rex_unary(8, acc);
        }
        self.bytes.push(match (store, size == 1) {
            (false, true) => 0xA0,
            (false, false) => 0xA1,
            (true, true) => 0xA2,
            (true, false) => 0xA3,
        });
        self.bytes.extend_from_slice(&(*v as i32).to_le_bytes());
        Ok(())
    }

    pub(crate) fn encode_movabs(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("movabsq requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.emit_rex_unary(8, &dst.name);
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
                self.emit_rex_unary(8, &dst.name);
                self.bytes.push(0xB8 + (dst_num & 7));
                self.add_relocation(sym, R_X86_64_64, addend);
                self.bytes.extend_from_slice(&[0u8; 8]);
                Ok(())
            }
            // moffs forms: `movabs 0xADDR, %al/%ax/%eax/%rax` (A0/A1) and the
            // store direction `movabs %rax, 0xADDR` (A2/A3).  These are the only
            // instructions that take a full 64-bit absolute address, and they
            // are hard-wired to the accumulator, so no ModRM byte is emitted.
            // (Also the redirect target for `mov`-family + huge absolute.)
            (Operand::Memory(mem), Operand::Register(dst))
                if mem.base.is_none() && mem.index.is_none() && is_accum(&dst.name) =>
            {
                self.emit_moffs_addr(&dst.name, &mem.displacement, false)
            }
            (Operand::Register(src), Operand::Memory(mem))
                if mem.base.is_none() && mem.index.is_none() && is_accum(&src.name) =>
            {
                self.emit_moffs_addr(&src.name, &mem.displacement, true)
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
                ));
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
                    broadcast: None,
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

    /// Encode suffix-less `movzx` / `movsx` where both src and dst sizes are
    /// inferred from the register operands (e.g. `movzx %bl, %edi` from
    /// twofish-x86_64-asm_64.S). For reg-reg, src size comes from src reg,
    /// dst size from dst reg. For mem-reg, src size defaults to 1 when dst
    /// is 32/64-bit and the mnemonic is ambiguous — the twofish pattern is
    /// always reg-reg, so mem-reg falls back to byte source.
    pub(crate) fn encode_movzx_infer_both(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("movzx requires 2 operands".to_string());
        }
        let src_size = match &ops[0] {
            Operand::Register(r) => infer_reg_size(&r.name),
            Operand::Memory(_) => 1, // default byte source for ambiguous mem form
            _ => return Err("movzx source must be register or memory".to_string()),
        };
        if src_size != 1 && src_size != 2 {
            return Err(format!(
                "movzx source must be 8 or 16-bit, got {} bytes",
                src_size
            ));
        }
        let dst_size = infer_movext_dst_size(ops)?;
        self.encode_movzx(ops, src_size, dst_size)
    }

    pub(crate) fn encode_movsx_infer_both(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("movsx requires 2 operands".to_string());
        }
        let src_size = match &ops[0] {
            Operand::Register(r) => infer_reg_size(&r.name),
            Operand::Memory(_) => 1,
            _ => return Err("movsx source must be register or memory".to_string()),
        };
        if src_size != 1 && src_size != 2 && src_size != 4 {
            return Err(format!(
                "movsx source must be 8/16/32-bit, got {} bytes",
                src_size
            ));
        }
        let dst_size = infer_movext_dst_size(ops)?;
        // movsx with 32->64 is movslq (0x63), handled by encode_movsx
        self.encode_movsx(ops, src_size, dst_size)
    }

    pub(crate) fn encode_lea(&mut self, ops: &[Operand], size: u8) -> Result<(), String> {
        if self.apx_nf || self.apx_evex {
            return Err("lea has no APX NDD/{nf}/{evex} form".to_string());
        }
        if ops.len() != 2 {
            return Err("lea requires 2 operands (APX NDD lea is not encodable)".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                // (Segment overrides — including the TLS initial-exec
                // `g_tls@TPOFF(%fs:0)` shape — are emitted by the
                // dispatch-level operand scan in `encode`.)
                // 16-bit LEA takes the 0x66 operand-size prefix (GAS:
                // `leaw (%rax),%cx` -> 66 8d 08); emit_rex_rm does not
                // push 0x66 itself (legacy prefixes are each caller's
                // responsibility except the segment byte).
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, &dst.name, mem);
                self.bytes.push(0x8D);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("lea requires memory source and register destination".to_string()),
        }
    }

    /// `size`: 8 (default/`q'/`l') or 2 (`pushw`). The 16-bit immediate
    /// form is `66 68 iw` (GAS 2.47: `pushw $1000` = 66 68 e8 03 — the
    /// imm32 form is never used when the operand size is 16-bit).
    pub(crate) fn encode_push(&mut self, ops: &[Operand], size: u8) -> Result<(), String> {
        if self.apx_nf || self.apx_evex {
            return Err("{nf}/{evex} unsupported for `push'".to_string());
        }
        if ops.len() != 1 {
            return Err("push requires 1 operand".to_string());
        }
        let want16 = size == 2;
        if want16 {
            self.bytes.push(0x66);
        }
        match &ops[0] {
            Operand::Register(reg) => {
                // Segment registers (GAS 2.47, byte-probed): only %fs/%gs
                // are pushable in long mode (`0f a0/a8`); es/cs/ss/ds are
                // rejected verbatim. Formerly the generic r32 path fired
                // and `push %fs` silently encoded as `push %rsp` (0x54) —
                // a wrong-code bug, not a diagnostic.
                match reg.name.to_ascii_lowercase().as_str() {
                    "fs" | "gs" => {
                        self.bytes.push(0x0F);
                        self.bytes.push(if reg.name.eq_ignore_ascii_case("fs") {
                            0xA0
                        } else {
                            0xA8
                        });
                        return Ok(());
                    }
                    "es" | "cs" | "ss" | "ds" => {
                        return Err(format!("you can't `push %{}'", reg.name));
                    }
                    _ => {}
                }
                let num = reg_num(&reg.name).ok_or("bad register")?;
                self.emit_rex_unary(4, &reg.name);
                self.bytes.push(0x50 + (num & 7));
                Ok(())
            }
            Operand::Immediate(ImmediateValue::Integer(val)) => {
                if want16 {
                    // PUSH imm16: GAS 2.47 first truncates the expression
                    // value to the 16-bit operand width, THEN applies the
                    // shortest-form law: a truncated value that fits a
                    // SIGNED byte uses the imm8 form (`pushw $10' =
                    // 66 6a 0a; `pushw $0xffffff90' truncates to 0xff90 =
                    // -112 -> 66 6a 90); only values outside imm8 after
                    // truncation take 66 68 imm16 (`pushw $0x1234' =
                    // 66 68 34 12).  A value that survives neither the
                    // truncation check nor the byte check is out of range.
                    if !(-32768..=65535).contains(val) && !(0..=0xFFFF_FFFF).contains(val) {
                        return Err(format!(
                            "operand type mismatch: pushw immediate out of 16-bit range"
                        ));
                    }
                    let v16 = *val as i16;
                    if (-128..=127).contains(&v16) {
                        self.bytes.push(0x6A);
                        self.bytes.push(v16 as u8);
                    } else {
                        self.bytes.push(0x68);
                        self.bytes.extend_from_slice(&v16.to_le_bytes());
                    }
                    return Ok(());
                }
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
                let addend =
                    if let Operand::Immediate(ImmediateValue::SymbolPlusOffset(_, a)) = &ops[0] {
                        *a
                    } else {
                        0
                    };
                if want16 {
                    // `pushw $early` = 66 68 <iw> + R_X86_64_16 (GAS-probed).
                    self.bytes.push(0x68);
                    self.add_relocation(sym, R_X86_64_16, addend);
                    self.bytes.extend_from_slice(&[0, 0]);
                    return Ok(());
                }
                self.bytes.push(0x68);
                self.add_relocation(sym, R_X86_64_32S, addend);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            Operand::Memory(mem) => {
                self.emit_rex_rm(0, "", mem);
                self.bytes.push(0xFF);
                self.encode_modrm_mem(6, mem)
            }
            _ => Err("unsupported push operand".to_string()),
        }
    }

    pub(crate) fn encode_pop(&mut self, ops: &[Operand]) -> Result<(), String> {
        if self.apx_nf || self.apx_evex {
            return Err("{nf}/{evex} unsupported for `pop'".to_string());
        }
        if ops.len() != 1 {
            return Err("pop requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                // Segment registers: %fs/%gs only in long mode — same
                // treatment as push (GAS 2.47: `0f a1/a9`, es/cs/ss/ds
                // rejected verbatim).
                match reg.name.to_ascii_lowercase().as_str() {
                    "fs" | "gs" => {
                        self.bytes.push(0x0F);
                        self.bytes.push(if reg.name.eq_ignore_ascii_case("fs") {
                            0xA1
                        } else {
                            0xA9
                        });
                        return Ok(());
                    }
                    "es" | "cs" | "ss" | "ds" => {
                        return Err(format!("you can't `pop %{}'", reg.name));
                    }
                    _ => {}
                }
                let num = reg_num(&reg.name).ok_or("bad register")?;
                self.emit_rex_unary(4, &reg.name);
                self.bytes.push(0x58 + (num & 7));
                Ok(())
            }
            Operand::Memory(mem) => {
                // pop to memory: 8F /0
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
        if self.apx_nf && alu_op == 7 {
            return Err("{nf} unsupported for `cmp'".to_string());
        }
        if ops.len() == 3 {
            if alu_op == 7 {
                return Err("cmp does not support APX NDD".to_string());
            }
            return self.encode_alu_apx(ops, mnemonic, alu_op, true);
        }
        if self.apx_wants_evex() {
            return self.encode_alu_apx(ops, mnemonic, alu_op, false);
        }
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
                    if is_accum(&dst.name) {
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
                    if is_accum(&dst.name) {
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
                // `.s` on a register-register ALU pair selects the
                // store-direction encoding (GAS 2.47: `add.s %edx,%ecx'
                // = 03 ca, not 01 d1; `add.s %r8d,%r9d' = 45 03 c8 — the
                // D=1 form even when both extensions are needed).  The
                // roles swap with the opcode; memory/immediate forms
                // ignore `.s` (its direction is forced by the memory
                // operand there), which the other arms below never touch.
                if self.s_flip {
                    self.emit_rex_rr(size, &dst.name, &src.name);
                    self.bytes
                        .push(if size == 1 { 0x02 } else { 0x03 } + alu_op * 8);
                    self.bytes.push(self.modrm(3, dst_num, src_num));
                    return Ok(());
                }
                self.emit_rex_rr(size, &src.name, &dst.name);
                self.bytes
                    .push(if size == 1 { 0x00 } else { 0x01 } + alu_op * 8);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
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
                    broadcast: None,
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
                    broadcast: None,
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
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, "", mem);
                let rc = self.relocations.len();
                // Operand-sized immediate + matching relocation width
                // (GAS 2.47, byte-probed): 8-bit = `80 /0 ib` +
                // R_X86_64_8 (`addb $sym,(%rax)` = `80 00 00`), 16-bit =
                // `66 81 /0 iw` + R_X86_64_16, 32/64-bit = `81 /0 id` +
                // R_X86_64_32S. The old path always emitted opcode 81
                // with a 4-byte R_32S — the 8-bit form is a #UD-adjacent
                // mis-encode (81 has no 8-bit row).
                match size {
                    1 => {
                        self.bytes.push(0x80);
                        self.encode_modrm_mem(alu_op, mem)?;
                        self.add_relocation(sym, R_X86_64_8, addend);
                        self.bytes.push(0);
                        self.adjust_rip_reloc_addend(rc, 1);
                    }
                    2 => {
                        self.bytes.push(0x81);
                        self.encode_modrm_mem(alu_op, mem)?;
                        self.add_relocation(sym, R_X86_64_16, addend);
                        self.bytes.extend_from_slice(&[0; 2]);
                        self.adjust_rip_reloc_addend(rc, 2);
                    }
                    4 => {
                        self.bytes.push(0x81);
                        self.encode_modrm_mem(alu_op, mem)?;
                        // R_X86_64_32 for the 32-bit form (GAS class law;
                        // R_32S is the 64-bit/REX spelling).
                        self.add_relocation(sym, R_X86_64_32, addend);
                        self.bytes.extend_from_slice(&[0; 4]);
                        self.adjust_rip_reloc_addend(rc, 4);
                    }
                    _ => {
                        self.bytes.push(0x81);
                        self.encode_modrm_mem(alu_op, mem)?;
                        // Emit 4-byte relocation for the symbol immediate.
                        // Use bytes.len() (instruction-relative offset) since
                        // elf_writer_common adds the section base offset separately.
                        self.add_relocation(sym, R_X86_64_32S, addend);
                        self.bytes.extend_from_slice(&[0; 4]);
                        self.adjust_rip_reloc_addend(rc, 4);
                    }
                }
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
                if is_accum(&dst.name) {
                    // Accumulator short form with a relocated immediate:
                    // `add $(_GLOBAL_OFFSET_TABLE_ - .), %eax` is `05
                    // disp32`, not `81 /0` — one byte shorter, exactly what
                    // GAS 2.47 emits for the kernel's GOT-base idiom. The
                    // immediate is OPERAND-SIZED (GAS 2.47, byte-probed):
                    // `add $sym,%al` = `04 00` + R_X86_64_8 and
                    // `sub $sym,%ax` = `66 2d 00 00` + R_X86_64_16 — not
                    // the 32-bit form with a 4-byte R_32S.
                    self.bytes
                        .push(if size == 1 { 0x04 } else { 0x05 } + alu_op * 8);
                    match size {
                        1 => {
                            self.add_relocation(sym, R_X86_64_8, addend);
                            self.bytes.push(0);
                        }
                        2 => {
                            self.add_relocation(sym, R_X86_64_16, addend);
                            self.bytes.extend_from_slice(&[0; 2]);
                        }
                        // 32-bit takes R_X86_64_32 (GAS: `add $sym,%eax` =
                        // R_32 — the zero-extending class), 64-bit takes
                        // the sign-extending R_X86_64_32S.
                        4 => {
                            self.add_relocation(sym, R_X86_64_32, addend);
                            self.bytes.extend_from_slice(&[0; 4]);
                        }
                        _ => {
                            self.add_relocation(sym, R_X86_64_32S, addend);
                            self.bytes.extend_from_slice(&[0; 4]);
                        }
                    }
                    return Ok(());
                }
                // Non-accumulator: 8-bit uses the 80 /ib form with an
                // 8-bit relocation (`add $sym,%bl` = `80 c3 00` +
                // R_X86_64_8 — the 81 /0 id form is 16/32-bit only);
                // 16-bit keeps 66 81 with a 2-byte R_16 field.
                match size {
                    1 => {
                        self.bytes.push(0x80);
                        self.bytes.push(self.modrm(3, alu_op, dst_num));
                        self.add_relocation(sym, R_X86_64_8, addend);
                        self.bytes.push(0);
                    }
                    2 => {
                        self.bytes.push(0x81);
                        self.bytes.push(self.modrm(3, alu_op, dst_num));
                        self.add_relocation(sym, R_X86_64_16, addend);
                        self.bytes.extend_from_slice(&[0; 2]);
                    }
                    _ => {
                        self.bytes.push(0x81);
                        self.bytes.push(self.modrm(3, alu_op, dst_num));
                        self.add_relocation(sym, R_X86_64_32S, addend);
                        self.bytes.extend_from_slice(&[0; 4]);
                    }
                }
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
                if is_accum(&dst.name) {
                    // Accumulator short form (see the Symbol arm above).
                    self.bytes
                        .push(if size == 1 { 0x04 } else { 0x05 } + alu_op * 8);
                } else {
                    self.bytes.push(0x81);
                    self.bytes.push(self.modrm(3, alu_op, dst_num));
                }
                self.add_diff_relocation(sym, diff, R_X86_64_PC32, 0);
                self.bytes.extend_from_slice(&[0; 4]);
                Ok(())
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    /// APX EVEX (map-4) encoding of ADD/OR/ADC/SBB/AND/SUB/XOR.
    /// `ndd` is true for the 3-operand NDD form (dest in vvvv).
    fn encode_alu_apx(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
        alu_op: u8,
        ndd: bool,
    ) -> Result<(), String> {
        let size = mnemonic_size_suffix(mnemonic).unwrap_or(8);
        let nf = self.apx_nf;
        let opc_rmr = if size == 1 { 0x00 } else { 0x01 } + alu_op * 8;
        let opc_rrm = if size == 1 { 0x02 } else { 0x03 } + alu_op * 8;

        if ndd {
            if ops.len() != 3 {
                return Err(format!("{} NDD requires 3 operands", mnemonic));
            }
            let ndd_name = match &ops[2] {
                Operand::Register(r) => r.name.as_str(),
                _ => return Err("APX NDD destination must be a register".to_string()),
            };
            return match (&ops[0], &ops[1]) {
                (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(src1)) => {
                    self.encode_alu_apx_imm(*val, size, alu_op, &src1.name, Some(ndd_name), nf)
                }
                (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Memory(mem)) => {
                    self.encode_alu_apx_imm_mem(*val, size, alu_op, mem, Some(ndd_name), nf)
                }
                (Operand::Register(src), Operand::Register(src1)) => {
                    let src_n = reg_num(&src.name).ok_or("bad src register")?;
                    let src1_n = reg_num(&src1.name).ok_or("bad register")?;
                    self.emit_apx_evex_rr(size, &src.name, &src1.name, Some(ndd_name), nf)?;
                    self.bytes.push(opc_rmr);
                    self.bytes.push(self.modrm(3, src_n, src1_n));
                    Ok(())
                }
                (Operand::Register(src), Operand::Memory(mem)) => {
                    let src_n = reg_num(&src.name).ok_or("bad src register")?;
                    self.emit_apx_evex_rm(size, &src.name, mem, Some(ndd_name), nf)?;
                    self.bytes.push(opc_rmr);
                    self.encode_modrm_mem(src_n, mem)
                }
                (Operand::Memory(mem), Operand::Register(src1)) => {
                    let src1_n = reg_num(&src1.name).ok_or("bad register")?;
                    self.emit_apx_evex_rm(size, &src1.name, mem, Some(ndd_name), nf)?;
                    self.bytes.push(opc_rrm);
                    self.encode_modrm_mem(src1_n, mem)
                }
                _ => Err(format!("unsupported {} NDD operands", mnemonic)),
            };
        }

        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }
        match (&ops[0], &ops[1]) {
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(dst)) => {
                self.encode_alu_apx_imm(*val, size, alu_op, &dst.name, None, nf)
            }
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Memory(mem)) => {
                self.encode_alu_apx_imm_mem(*val, size, alu_op, mem, None, nf)
            }
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_n = reg_num(&src.name).ok_or("bad src register")?;
                let dst_n = reg_num(&dst.name).ok_or("bad dst register")?;
                self.emit_apx_evex_rr(size, &src.name, &dst.name, None, nf)?;
                self.bytes.push(opc_rmr);
                self.bytes.push(self.modrm(3, src_n, dst_n));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_n = reg_num(&dst.name).ok_or("bad dst register")?;
                self.emit_apx_evex_rm(size, &dst.name, mem, None, nf)?;
                self.bytes.push(opc_rrm);
                self.encode_modrm_mem(dst_n, mem)
            }
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_n = reg_num(&src.name).ok_or("bad src register")?;
                self.emit_apx_evex_rm(size, &src.name, mem, None, nf)?;
                self.bytes.push(opc_rmr);
                self.encode_modrm_mem(src_n, mem)
            }
            _ => Err(format!("unsupported {} operands for APX EVEX", mnemonic)),
        }
    }

    fn encode_alu_apx_imm(
        &mut self,
        val: i64,
        size: u8,
        alu_op: u8,
        rm: &str,
        ndd: Option<&str>,
        nf: bool,
    ) -> Result<(), String> {
        Self::check_imm32s_q("alu", size, val)?;
        let rm_n = reg_num(rm).ok_or("bad register")?;
        self.emit_apx_evex_rr(size, "", rm, ndd, nf)?;
        // Empty `reg` name: emit_apx_evex_rr uses gp_ext_bits("") → (false,false)
        // which is correct for /digit opcodes (ModRM.reg = alu_op).
        if size == 1 {
            self.bytes.push(0x80);
            self.bytes.push(self.modrm(3, alu_op, rm_n));
            self.bytes.push(val as u8);
        } else if fits_imm8(val, size) {
            self.bytes.push(0x83);
            self.bytes.push(self.modrm(3, alu_op, rm_n));
            self.bytes.push(canonical_imm(val, size) as u8);
        } else {
            self.bytes.push(0x81);
            self.bytes.push(self.modrm(3, alu_op, rm_n));
            if size == 2 {
                self.bytes.extend_from_slice(&(val as i16).to_le_bytes());
            } else {
                self.bytes.extend_from_slice(&(val as i32).to_le_bytes());
            }
        }
        Ok(())
    }

    fn encode_alu_apx_imm_mem(
        &mut self,
        val: i64,
        size: u8,
        alu_op: u8,
        mem: &MemoryOperand,
        ndd: Option<&str>,
        nf: bool,
    ) -> Result<(), String> {
        Self::check_imm32s_q("alu", size, val)?;
        self.emit_apx_evex_rm(size, "", mem, ndd, nf)?;
        if size == 1 {
            self.bytes.push(0x80);
            self.encode_modrm_mem(alu_op, mem)?;
            self.bytes.push(val as u8);
        } else if fits_imm8(val, size) {
            self.bytes.push(0x83);
            self.encode_modrm_mem(alu_op, mem)?;
            self.bytes.push(canonical_imm(val, size) as u8);
        } else {
            self.bytes.push(0x81);
            self.encode_modrm_mem(alu_op, mem)?;
            if size == 2 {
                self.bytes.extend_from_slice(&(val as i16).to_le_bytes());
            } else {
                self.bytes.extend_from_slice(&(val as i32).to_le_bytes());
            }
        }
        Ok(())
    }

    /// `{evex} test` is encoded as CTEST with SCC=0xA (GAS 2.47:
    /// `{evex} testq %rcx, %rax` → `62 f4 84 0a 85 c8`). `{nf} test` is illegal.
    fn encode_test_evex(&mut self, ops: &[Operand], size: u8) -> Result<(), String> {
        let w = size == 8;
        let pp = Self::apx_pp(size);
        let opc = if size == 1 { 0x84u8 } else { 0x85 };
        const SCC_TEST: u8 = 0x0A;
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_n = reg_num(&src.name).ok_or("bad src register")?;
                let dst_n = reg_num(&dst.name).ok_or("bad dst register")?;
                self.emit_evex_ccmp_rr(w, &src.name, &dst.name, 0, SCC_TEST, pp)?;
                self.bytes.push(opc);
                self.bytes.push(self.modrm(3, src_n, dst_n));
                Ok(())
            }
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_n = reg_num(&src.name).ok_or("bad src register")?;
                self.emit_evex_ccmp_rm(w, &src.name, mem, 0, SCC_TEST, pp)?;
                self.bytes.push(opc);
                self.encode_modrm_mem(src_n, mem)
            }
            (Operand::Memory(mem), Operand::Register(reg)) => {
                let n = reg_num(&reg.name).ok_or("bad register")?;
                self.emit_evex_ccmp_rm(w, &reg.name, mem, 0, SCC_TEST, pp)?;
                self.bytes.push(opc);
                self.encode_modrm_mem(n, mem)
            }
            _ => Err("unsupported {evex} test operands".to_string()),
        }
    }

    pub(crate) fn encode_test(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if self.apx_nf {
            return Err("{nf} unsupported for `test`".to_string());
        }
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }

        let size = mnemonic_size_suffix(mnemonic).unwrap_or(8);
        if self.apx_evex {
            return self.encode_test_evex(ops, size);
        }

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                // `test` has no direction bit, but `.s` still swaps the
                // modrm roles (GAS 2.47: `test %edx,%ecx' = 85 d1,
                // `test.s %edx,%ecx' = 85 ca).  The opcode byte itself
                // never changes.
                let ((a, a_num), (b, b_num)) = if self.s_flip {
                    ((&dst.name, dst_num), (&src.name, src_num))
                } else {
                    ((&src.name, src_num), (&dst.name, dst_num))
                };
                self.emit_rex_rr(size, a, b);
                self.bytes.push(if size == 1 { 0x84 } else { 0x85 });
                self.bytes.push(self.modrm(3, a_num, b_num));
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
                    if is_accum(&dst.name) {
                        self.bytes.push(0xA8);
                    } else {
                        self.bytes.push(0xF6);
                        self.bytes.push(self.modrm(3, 0, dst_num));
                    }
                    self.bytes.push(val as u8);
                } else {
                    if is_accum(&dst.name) {
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

    /// NOP with an operand: 0F 1F /r (the canonical long-NOP body).
    /// Register destinations pick their operand size from the register's
    /// own width (GAS 2.47 byte-probed: `nop %al` = 0f 1f c0, `nop %ax` =
    /// 66 0f 1f c0, `nop %rax` = 48 0f 1f c0, `nop %r10` = 49 0f 1f c2 —
    /// REX.W for the 64-bit register, 0x66 for the 16-bit one). `forced`
    /// overrides the register width for the suffixed spellings (`nopw`,
    /// `nopl`, `nopq`); memory operands default to 32-bit.
    pub(crate) fn encode_nop_rm(
        &mut self,
        ops: &[Operand],
        forced: Option<u8>,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("nop with operand requires exactly 1 operand".to_string());
        }
        let size = match &ops[0] {
            Operand::Register(reg) => forced.unwrap_or_else(|| infer_reg_size(&reg.name)),
            _ => forced.unwrap_or(4),
        };
        if size == 2 {
            self.bytes.push(0x66);
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_rex_rm(if size == 8 { 8 } else { 4 }, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0x1F]);
                self.encode_modrm_mem(0, mem)
            }
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                self.emit_rex_unary(if size == 8 { 8 } else { size.min(4) }, &reg.name);
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
                // Under `{nf}`/`{evex}` the EVEX body carries the size in
                // its pp bits — a manual 0x66 in front of an EVEX prefix
                // is a #UD sequence (`{nf} imul %dx` used to leak
                // `66 62 f4 ..`).
                if size == 2 && !self.apx_wants_evex() {
                    self.bytes.push(0x66);
                }
                self.encode_unary_rm(ops, 5, size)
            }
            2 => match (&ops[0], &ops[1]) {
                (Operand::Register(src), Operand::Register(dst)) => {
                    let src_num = reg_num(&src.name).ok_or("bad register")?;
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    if self.apx_wants_evex() {
                        self.emit_apx_evex_rr(size, &dst.name, &src.name, None, self.apx_nf)?;
                        self.bytes.push(0xAF);
                        self.bytes.push(self.modrm(3, dst_num, src_num));
                        return Ok(());
                    }
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
                    if self.apx_wants_evex() {
                        self.emit_apx_evex_rm(size, &dst.name, mem, None, self.apx_nf)?;
                        self.bytes.push(0xAF);
                        return self.encode_modrm_mem(dst_num, mem);
                    }
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
                    if self.apx_wants_evex() {
                        self.emit_apx_evex_rr(size, &dst.name, &dst.name, None, self.apx_nf)?;
                    } else {
                        if size == 2 {
                            self.bytes.push(0x66);
                        }
                        self.emit_rex_rr(size, &dst.name, &dst.name);
                    }
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
                // APX NDD: `imulq %src, %src1, %ndd` (no immediate).
                if let (Operand::Register(src), Operand::Register(src1), Operand::Register(ndd)) =
                    (&ops[0], &ops[1], &ops[2])
                {
                    let src_n = reg_num(&src.name).ok_or("bad register")?;
                    let src1_n = reg_num(&src1.name).ok_or("bad register")?;
                    self.emit_apx_evex_rr(
                        size,
                        &src1.name,
                        &src.name,
                        Some(&ndd.name),
                        self.apx_nf,
                    )?;
                    self.bytes.push(0xAF);
                    self.bytes.push(self.modrm(3, src1_n, src_n));
                    return Ok(());
                }
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
                        // `{nf} imul $3, %ecx, %edx` — EVEX map-4 NF form
                        // (62 f4 7c 0c 6b d1 03, GAS-probed); the immediate
                        // and the register operands keep their legacy
                        // ModRM/imm layout.
                        if self.apx_wants_evex() {
                            self.emit_apx_evex_rr(size, &dst.name, &src.name, None, self.apx_nf)?;
                            self.bytes.push(if short { 0x6B } else { 0x69 });
                            self.bytes.push(self.modrm(3, dst_num, src_num));
                        } else {
                            if size == 2 {
                                self.bytes.push(0x66);
                            }
                            self.emit_rex_rr(size, &dst.name, &src.name);
                            self.bytes.push(if short { 0x6B } else { 0x69 });
                            self.bytes.push(self.modrm(3, dst_num, src_num));
                        }
                    }
                    Operand::Memory(mem) => {
                        if self.apx_wants_evex() {
                            self.emit_apx_evex_rm(size, &dst.name, mem, None, self.apx_nf)?;
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
                        } else {
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

    /// 16-bit unary dispatch for callers that push the legacy 0x66
    /// operand-size prefix themselves. The 0x66 byte belongs to the
    /// LEGACY encoding only: when the two-operand or APX path is taken
    /// the 16-bit-ness rides the EVEX pp field instead, and a pushed
    /// legacy byte would leave `66 62 …` in the stream — a #UD sequence
    /// on real silicon (GAS 2.47: `{nf} divw %cx` = `62 f4 7d 0c f7 f1`,
    /// byte-probed; NOT `66 62 …`).
    pub(crate) fn encode_unary_rm_66(
        &mut self,
        ops: &[Operand],
        op_ext: u8,
        size: u8,
    ) -> Result<(), String> {
        if size == 2 && ops.len() != 2 && !self.apx_wants_evex() {
            self.bytes.push(0x66);
        }
        self.encode_unary_rm(ops, op_ext, size)
    }

    pub(crate) fn encode_unary_rm(
        &mut self,
        ops: &[Operand],
        op_ext: u8,
        size: u8,
    ) -> Result<(), String> {
        if ops.len() == 2 || self.apx_wants_evex() {
            return self.encode_unary_apx(ops, op_ext, size);
        }
        if ops.len() != 1 {
            return Err("unary op requires 1 operand".to_string());
        }
        // inc (op_ext=0) and dec (op_ext=1) use FE/FF, not F6/F7
        let base_opcode = if op_ext <= 1 {
            if size == 1 { 0xFE } else { 0xFF }
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
                // Segment override before REX (see encode_inc_dec's Memory
                // arm for the kernel per-CPU context; inc/dec/neg/not and
                // mul/div/idiv memory forms share this path).
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
        if ops.len() == 2 || self.apx_wants_evex() {
            return self.encode_unary_apx(ops, op_ext, size);
        }
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
                // Segment override before REX (legacy prefixes precede
                // REX; emit_segment_prefix back-scans over 0x66/0x67 so the
                // override lands outermost). The kernel's per-CPU
                // preempt_count unary ops are exactly this form:
                // `preempt_disable()` lowers to `asm("incl %[var]" : "+m"
                // (__my_cpu_var(__preempt_count)))` — the template text
                // carries `%gs:` (the frontend's AS walk is correct), but
                // WITHOUT this prefix the ENCODER silently dropped it: the
                // increment went to the static percpu image (RIP-relative,
                // no %gs) while every read stayed %gs-correct, the
                // per-CPU counter never moved, and 6.18.50 died at PID 1
                // (finish_task_switch "corrupted preempt_count: .../0x1",
                // workqueue "leaked atomic" BUGs, page-fault cascade).
                // Same defect class as the S11 cmpxchg16b fix.
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
    pub(crate) fn check_imm32s_q(mnemonic: &str, size: u8, val: i64) -> Result<(), String> {
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

        if self.apx_wants_evex() || ops.len() == 3 {
            return self.encode_shift_apx(ops, mnemonic, shift_op, size);
        }

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

    fn encode_unary_apx(&mut self, ops: &[Operand], op_ext: u8, size: u8) -> Result<(), String> {
        // APX NF applies to NEG/INC/DEC/MUL/DIV/IDIV, not NOT.
        if self.apx_nf && op_ext == 2 {
            return Err("{nf} unsupported for `not'".to_string());
        }
        // Two-operand unary laws (GAS 2.47, byte-probed in every mode —
        // plain, {nf} and {evex}):
        //   * mul/imul have NO two-operand unary form. `mul %ecx,%eax` is
        //     `number of operands mismatch for `mul'' (the real 2-op imul
        //     is the 0F AF family and never reaches this encoder). The old
        //     dead `matches!(op_ext, 4)` check inside the 6|7 guard let a
        //     silent APX encoding (ND=1!) out for both.
        //   * div/idiv require the second operand to be the implicit
        //     accumulator of the matching size (`{nf} idiv %ecx,%edx` is
        //     `operand type mismatch for `idiv'`).
        if ops.len() == 2 {
            if matches!(op_ext, 4 | 5) {
                return Err(format!(
                    "number of operands mismatch for `{}'",
                    if op_ext == 4 { "mul" } else { "imul" }
                ));
            }
            if matches!(op_ext, 6 | 7) {
                let acc = match size {
                    1 => "al",
                    2 => "ax",
                    4 => "eax",
                    _ => "rax",
                };
                match &ops[1] {
                    Operand::Register(r) if r.name.eq_ignore_ascii_case(acc) => {}
                    _ => {
                        return Err(format!(
                            "operand type mismatch for `{}'",
                            if op_ext == 6 { "div" } else { "idiv" }
                        ));
                    }
                }
                if !self.apx_wants_evex() {
                    // No decorators: the LEGACY unary with the single r/m
                    // operand (`div %ecx,%eax` = `f7 f1`).
                    return self.encode_unary_rm(std::slice::from_ref(&ops[0]), op_ext, size);
                }
                // {nf}/{evex}: fall through to the ND=0 arms below with
                // ops unchanged — vvvv mirrors the (accumulator-checked)
                // second operand, which for %eax stores 1111, the
                // hardware's unused pattern (`{nf} idiv %ecx,%eax` =
                // `62 f4 7c 0c f7 f9`).
            }
        }
        let nf = self.apx_nf;
        let base_opcode = if op_ext <= 1 {
            if size == 1 { 0xFE } else { 0xFF }
        } else if size == 1 {
            0xF6
        } else {
            0xF7
        };
        match ops {
            [Operand::Register(src), Operand::Register(ndd)] => {
                let num = reg_num(&src.name).ok_or("bad register")?;
                // IDIV's promoted two-operand form keeps ND=0: vvvv carries
                // the dividend's high half as a SOURCE (the destination is
                // the implicit rDX:rA pair) — `idiv %ecx, %eax` is
                // `62 f4 7c 0c f7 f9`-shaped (NF=1, ND=0), unlike neg/inc
                // whose vvvv is a true new destination (GAS 2.47).
                if matches!(op_ext, 6 | 7) {
                    self.emit_apx_evex_rr_nd0(size, "", &src.name, &ndd.name, nf)?;
                } else {
                    self.emit_apx_evex_rr(size, "", &src.name, Some(&ndd.name), nf)?;
                }
                self.bytes.push(base_opcode);
                self.bytes.push(self.modrm(3, op_ext, num));
                Ok(())
            }
            [Operand::Memory(mem), Operand::Register(ndd)] => {
                if matches!(op_ext, 6 | 7) {
                    self.emit_apx_evex_rm_nd0(size, "", mem, &ndd.name, nf)?;
                } else {
                    self.emit_apx_evex_rm(size, "", mem, Some(&ndd.name), nf)?;
                }
                self.bytes.push(base_opcode);
                self.encode_modrm_mem(op_ext, mem)
            }
            [Operand::Register(rm)] => {
                let num = reg_num(&rm.name).ok_or("bad register")?;
                self.emit_apx_evex_rr(size, "", &rm.name, None, nf)?;
                self.bytes.push(base_opcode);
                self.bytes.push(self.modrm(3, op_ext, num));
                Ok(())
            }
            [Operand::Memory(mem)] => {
                self.emit_apx_evex_rm(size, "", mem, None, nf)?;
                self.bytes.push(base_opcode);
                self.encode_modrm_mem(op_ext, mem)
            }
            _ => Err("unsupported APX unary operands".to_string()),
        }
    }

    fn encode_shift_apx(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
        shift_op: u8,
        size: u8,
    ) -> Result<(), String> {
        if self.apx_nf && (mnemonic.starts_with("rcl") || mnemonic.starts_with("rcr")) {
            return Err(format!("{{nf}} unsupported for `{mnemonic}`"));
        }
        let nf = self.apx_nf;
        let emit_body = |enc: &mut Self,
                         rm: &str,
                         ndd: Option<&str>,
                         opcode: u8,
                         imm: Option<u8>|
         -> Result<(), String> {
            let num = reg_num(rm).ok_or("bad register")?;
            enc.emit_apx_evex_rr(size, "", rm, ndd, nf)?;
            enc.bytes.push(opcode);
            enc.bytes.push(enc.modrm(3, shift_op, num));
            if let Some(c) = imm {
                enc.bytes.push(c);
            }
            Ok(())
        };
        match ops {
            [
                Operand::Immediate(ImmediateValue::Integer(count)),
                Operand::Register(src),
                Operand::Register(ndd),
            ] => {
                Self::check_shift_imm(mnemonic, size, *count)?;
                let count = *count as u8;
                if count == 1 {
                    emit_body(
                        self,
                        &src.name,
                        Some(&ndd.name),
                        if size == 1 { 0xD0 } else { 0xD1 },
                        None,
                    )
                } else {
                    emit_body(
                        self,
                        &src.name,
                        Some(&ndd.name),
                        if size == 1 { 0xC0 } else { 0xC1 },
                        Some(count),
                    )
                }
            }
            [
                Operand::Register(cl),
                Operand::Register(src),
                Operand::Register(ndd),
            ] if cl.name == "cl" => emit_body(
                self,
                &src.name,
                Some(&ndd.name),
                if size == 1 { 0xD2 } else { 0xD3 },
                None,
            ),
            [
                Operand::Immediate(ImmediateValue::Integer(count)),
                Operand::Register(dst),
            ] => {
                Self::check_shift_imm(mnemonic, size, *count)?;
                let count = *count as u8;
                if count == 1 {
                    emit_body(
                        self,
                        &dst.name,
                        None,
                        if size == 1 { 0xD0 } else { 0xD1 },
                        None,
                    )
                } else {
                    emit_body(
                        self,
                        &dst.name,
                        None,
                        if size == 1 { 0xC0 } else { 0xC1 },
                        Some(count),
                    )
                }
            }
            [Operand::Register(cl), Operand::Register(dst)] if cl.name == "cl" => emit_body(
                self,
                &dst.name,
                None,
                if size == 1 { 0xD2 } else { 0xD3 },
                None,
            ),
            [Operand::Register(dst)] => emit_body(
                self,
                &dst.name,
                None,
                if size == 1 { 0xD0 } else { 0xD1 },
                None,
            ),
            _ => Err(format!("unsupported APX {} operands", mnemonic)),
        }
    }

    pub(crate) fn encode_double_shift(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        size: u8,
    ) -> Result<(), String> {
        if ops.len() == 4 {
            return self.encode_double_shift_ndd(ops, opcode, size);
        }
        if ops.len() != 3 {
            return Err("double shift requires 3 or 4 operands".to_string());
        }

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(count)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if self.apx_wants_evex() {
                    // Map-4 remaps: shld-imm 24, shrd-imm 2C. CL forms keep A5/AD.
                    let map4 = if opcode == 0xA4 { 0x24 } else { 0x2C };
                    self.emit_apx_evex_rr(size, &src.name, &dst.name, None, self.apx_nf)?;
                    self.bytes.push(map4);
                    self.bytes.push(self.modrm(3, src_num, dst_num));
                    self.bytes.push(*count as u8);
                    return Ok(());
                }
                if size == 2 {
                    self.bytes.push(0x66);
                }
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
                if self.apx_wants_evex() {
                    self.emit_apx_evex_rr(size, &src.name, &dst.name, None, self.apx_nf)?;
                    self.bytes.push(opcode + 1);
                    self.bytes.push(self.modrm(3, src_num, dst_num));
                    return Ok(());
                }
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rr(size, &src.name, &dst.name);
                self.bytes.extend_from_slice(&[0x0F, opcode + 1]);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            _ => Err("unsupported double shift operands".to_string()),
        }
    }

    /// APX NDD SHLD/SHRD: `shldq $imm/%cl, %src, %src1, %ndd`.
    fn encode_double_shift_ndd(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        size: u8,
    ) -> Result<(), String> {
        let ndd = match &ops[3] {
            Operand::Register(r) => r.name.as_str(),
            _ => return Err("NDD destination must be a register".to_string()),
        };
        let nf = self.apx_nf;
        let map4_imm = if opcode == 0xA4 { 0x24u8 } else { 0x2C };
        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(count)),
                Operand::Register(src),
                Operand::Register(src1),
            ) => {
                let src_n = reg_num(&src.name).ok_or("bad register")?;
                let src1_n = reg_num(&src1.name).ok_or("bad register")?;
                self.emit_apx_evex_rr(size, &src.name, &src1.name, Some(ndd), nf)?;
                self.bytes.push(map4_imm);
                self.bytes.push(self.modrm(3, src_n, src1_n));
                self.bytes.push(*count as u8);
                Ok(())
            }
            (Operand::Register(cl), Operand::Register(src), Operand::Register(src1))
                if cl.name == "cl" =>
            {
                let src_n = reg_num(&src.name).ok_or("bad register")?;
                let src1_n = reg_num(&src1.name).ok_or("bad register")?;
                self.emit_apx_evex_rr(size, &src.name, &src1.name, Some(ndd), nf)?;
                self.bytes.push(opcode + 1);
                self.bytes.push(self.modrm(3, src_n, src1_n));
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(count)),
                Operand::Register(src),
                Operand::Memory(mem),
            ) => {
                let src_n = reg_num(&src.name).ok_or("bad register")?;
                self.emit_apx_evex_rm(size, &src.name, mem, Some(ndd), nf)?;
                let rc = self.relocations.len();
                self.bytes.push(map4_imm);
                self.encode_modrm_mem(src_n, mem)?;
                self.bytes.push(*count as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            (Operand::Register(cl), Operand::Register(src), Operand::Memory(mem))
                if cl.name == "cl" =>
            {
                let src_n = reg_num(&src.name).ok_or("bad register")?;
                self.emit_apx_evex_rm(size, &src.name, mem, Some(ndd), nf)?;
                self.bytes.push(opcode + 1);
                self.encode_modrm_mem(src_n, mem)
            }
            _ => Err("unsupported NDD double shift operands".to_string()),
        }
    }

    pub(crate) fn encode_bswap(&mut self, ops: &[Operand], size: u8) -> Result<(), String> {
        if self.apx_nf {
            return Err("{nf} unsupported for `bswap`".to_string());
        }
        if self.apx_evex {
            return Err("no EVEX encoding for `bswap`".to_string());
        }
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

        if self.apx_wants_evex() {
            // Map-4 remaps: lzcnt F5, tzcnt F4, popcnt 88.
            let map4 = match mnemonic {
                "lzcntl" | "lzcntq" | "lzcntw" => 0xF5u8,
                "tzcntl" | "tzcntq" | "tzcntw" => 0xF4,
                _ => 0x88,
            };
            match (&ops[0], &ops[1]) {
                (Operand::Register(src), Operand::Register(dst)) => {
                    let src_num = reg_num(&src.name).ok_or("bad register")?;
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    self.emit_apx_evex_rr(size, &dst.name, &src.name, None, self.apx_nf)?;
                    self.bytes.push(map4);
                    self.bytes.push(self.modrm(3, dst_num, src_num));
                    return Ok(());
                }
                (Operand::Memory(mem), Operand::Register(dst)) => {
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    self.emit_apx_evex_rm(size, &dst.name, mem, None, self.apx_nf)?;
                    self.bytes.push(map4);
                    return self.encode_modrm_mem(dst_num, mem);
                }
                _ => return Err(format!("unsupported {} operands", mnemonic)),
            }
        }

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

        if self.apx_nf {
            return Err("{nf} unsupported for `setcc'".to_string());
        }

        // `setzuCC[b]`: APX zero-upper SETcc (ND=1, vvvv=0, pp=3).
        if let Some(rest) = mnemonic.strip_prefix("setzu") {
            let cc = cc_from_mnemonic(rest).or_else(|_| {
                rest.strip_suffix('b')
                    .ok_or_else(|| format!("unknown condition code: {rest}"))
                    .and_then(cc_from_mnemonic)
            })?;
            return match &ops[0] {
                Operand::Register(reg) => {
                    let num = reg_num(&reg.name).ok_or("bad register")?;
                    self.emit_apx_evex_rr_nd1_pp(false, "", &reg.name, false, 3)?;
                    self.bytes.push(0x40 + cc);
                    self.bytes.push(self.modrm(3, 0, num));
                    Ok(())
                }
                // ZU zeros the unused upper bits of a GPR. Memory has no
                // upper bits to clear, and GAS 2.47 rejects `setzu* (%reg)`.
                _ => Err("setzu requires a register operand".to_string()),
            };
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

        // APX auto-promotion (GAS 2.47 byte-probed): a 32/64-bit REGISTER
        // destination cannot use the legacy byte form — that would write
        // only the low byte and call it a day (`setae %eax` silently
        // becoming `setae %al` was a miscompile). GAS promotes to the
        // EVEX map-4 SETcc instead: `setae %eax` = 62 f4 7f 18 43 c0,
        // `setae %rax` = 62 f4 ff 18 43 c0 (W=1), `setae %r12d` =
        // 62 d4 7f 18 43 c4. 16-bit destinations are rejected verbatim
        // ("operand size mismatch for `setae'"); byte registers keep the
        // legacy 0F 90+cc form below. i686 has no APX: its own encoder
        // never reaches this code.
        if !self.apx_evex && !self.apx_rex2 {
            if let Operand::Register(reg) = &ops[0] {
                match infer_reg_size(&reg.name) {
                    4 | 8 => {
                        let w = infer_reg_size(&reg.name) == 8;
                        let num = reg_num(&reg.name).ok_or("bad register")?;
                        // NDD form: P2.ND=1, vvvv=dest, ModRM.reg=0
                        // (`setae %eax` = 62 f4 7f 18 43 c0, GAS-probed).
                        self.emit_apx_evex_rr_pp(w, "", &reg.name, Some(&reg.name), false, 3)?;
                        self.bytes.push(0x40 + cc);
                        self.bytes.push(self.modrm(3, 0, num));
                        return Ok(());
                    }
                    2 => {
                        return Err(format!("operand size mismatch for `{mnemonic}'"));
                    }
                    _ => {}
                }
            }
        }

        // `{evex} setCC` is EVEX without ZU (ND=0). `setzuCC` above is the ZU form.
        // EGPR without `{evex}` stays on the shorter REX2 0F 90+cc encoding.
        if self.apx_evex {
            return match &ops[0] {
                Operand::Register(reg) => {
                    let num = reg_num(&reg.name).ok_or("bad register")?;
                    // 32/64-bit registers take the NDD form (P2.ND=1,
                    // vvvv=dest — `{evex} setae %eax` = 62 f4 7f 18 43 c0);
                    // byte registers the plain promoted form (P2=08).
                    let ndd = if infer_reg_size(&reg.name) >= 4 {
                        Some(reg.name.as_str())
                    } else {
                        None
                    };
                    self.emit_apx_evex_rr_pp(false, "", &reg.name, ndd, false, 3)?;
                    self.bytes.push(0x40 + cc);
                    self.bytes.push(self.modrm(3, 0, num));
                    Ok(())
                }
                Operand::Memory(mem) => {
                    self.emit_apx_evex_rm_pp(false, "", mem, None, false, 3)?;
                    self.bytes.push(0x40 + cc);
                    self.encode_modrm_mem(0, mem)
                }
                _ => Err("setcc requires register or memory operand".to_string()),
            };
        }

        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                self.emit_rex_unary(1, &reg.name);
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
        if ops.len() != 2 && ops.len() != 3 {
            return Err("cmovcc requires 2 or 3 operands".to_string());
        }
        if self.apx_nf {
            return Err("{nf} unsupported for `cmov'".to_string());
        }
        if ops.len() == 2 && self.apx_evex {
            return Err("no EVEX encoding for 2-operand `cmov'".to_string());
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

        if ops.len() == 3 {
            // APX NDD: `cmovzq %src, %src1, %ndd`. GAS encodes ModRM.reg=src1,
            // r/m=src, vvvv=ndd — same operand polarity as IMUL 0xAF.
            return match (&ops[0], &ops[1], &ops[2]) {
                (Operand::Register(src), Operand::Register(src1), Operand::Register(ndd)) => {
                    let src_n = reg_num(&src.name).ok_or("bad register")?;
                    let src1_n = reg_num(&src1.name).ok_or("bad register")?;
                    self.emit_apx_evex_rr(
                        size,
                        &src1.name,
                        &src.name,
                        Some(&ndd.name),
                        self.apx_nf,
                    )?;
                    self.bytes.push(0x40 + cc);
                    self.bytes.push(self.modrm(3, src1_n, src_n));
                    Ok(())
                }
                (Operand::Memory(mem), Operand::Register(src1), Operand::Register(ndd)) => {
                    let src1_n = reg_num(&src1.name).ok_or("bad register")?;
                    self.emit_apx_evex_rm(size, &src1.name, mem, Some(&ndd.name), self.apx_nf)?;
                    self.bytes.push(0x40 + cc);
                    self.encode_modrm_mem(src1_n, mem)
                }
                _ => Err("unsupported cmov NDD operands".to_string()),
            };
        }

        if self.apx_wants_evex() {
            return match (&ops[0], &ops[1]) {
                (Operand::Register(src), Operand::Register(dst)) => {
                    let src_n = reg_num(&src.name).ok_or("bad register")?;
                    let dst_n = reg_num(&dst.name).ok_or("bad register")?;
                    self.emit_apx_evex_rr(size, &dst.name, &src.name, None, self.apx_nf)?;
                    self.bytes.push(0x40 + cc);
                    self.bytes.push(self.modrm(3, dst_n, src_n));
                    Ok(())
                }
                (Operand::Memory(mem), Operand::Register(dst)) => {
                    let dst_n = reg_num(&dst.name).ok_or("bad register")?;
                    self.emit_apx_evex_rm(size, &dst.name, mem, None, self.apx_nf)?;
                    self.bytes.push(0x40 + cc);
                    self.encode_modrm_mem(dst_n, mem)
                }
                _ => Err("unsupported cmov operands for APX EVEX".to_string()),
            };
        }

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
                let sym = strip_plt_suffix(label);
                if self.explicit_data16 && sym.len() != label.len() {
                    // GAS 2.47: `data16 jmp foo@PLT` — "4-byte relocation
                    // cannot be applied to 2-byte field". The field is now
                    // 4 bytes (the 66 is a dead prefix, see below), but the
                    // combination stays rejected for GAS parity: inventing
                    // an accept GAS refuses would diverge the accept set on
                    // a spelling no real code uses. Checked before the
                    // opcode bytes so a rejected form leaves no partial
                    // state in `self.bytes`.
                    return Err("4-byte relocation cannot be applied to 2-byte field".to_string());
                }
                self.bytes.push(0xE9);
                // `data16 jmp` does NOT shrink this field. In 64-bit mode
                // the 66 prefix is architecturally dead on direct near
                // branches (Intel SDM: the operand-size prefix has no
                // effect on near branches in 64-bit mode; hardware-proven:
                // the decoder consumes a FULL 4-byte displacement after
                // `66 e9`, so a 2-byte field desynchronises the instruction
                // stream and jumps through whatever follows). GAS 2.47
                // emits the truncated `66 e9 rel16` here anyway — that is a
                // GAS bug, not a row to copy. The central forced-data16
                // splice still prepends the dead 0x66 (`66 e9 rel32` is the
                // shortest VALID spelling of the request) and the relaxer
                // may shrink near targets to `66 eb rel8` (GAS parity).
                self.add_relocation(sym, R_X86_64_PLT32, -4);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            Operand::Indirect(inner) => match inner.as_ref() {
                Operand::Register(reg) => {
                    let num = reg_num(&reg.name).ok_or("bad register")?;
                    // 16-bit targets take the operand-size prefix (`call
                    // *%ax` = 66 ff d0; without it the same bytes decode as
                    // the 64-bit `call *%rax` — GAS 2.47 byte-verified).
                    if is_reg16(&reg.name) {
                        self.bytes.push(0x66);
                    }
                    self.emit_rex_unary(4, &reg.name);
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
                let sym = strip_plt_suffix(label);
                if self.explicit_data16 && sym.len() != label.len() {
                    // GAS 2.47: a 16-bit jcc field cannot carry a PLT32 —
                    // kept for parity even though the field is 4 bytes now
                    // (see the jmp arm for the full rationale).
                    return Err("4-byte relocation cannot be applied to 2-byte field".to_string());
                }
                self.bytes.extend_from_slice(&[0x0F, 0x80 + cc]);
                // `data16 je` keeps the full rel32 field: the 66 prefix is
                // dead on 64-bit near branches (see encode_jmp). GAS's
                // `66 0f 84 rel16` mis-executes — the decoder reads 4
                // displacement bytes, stealing two bytes from the next
                // instruction. The spliced dead 0x66 plus `0f 8x rel32`
                // is the shortest VALID form; the relaxer shrinks near
                // targets to `66 7x rel8` (GAS parity).
                self.add_relocation(sym, R_X86_64_PLT32, -4);
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
                // Use PLT32 for external function calls (linker will resolve)
                let sym = strip_plt_suffix(label);
                if self.explicit_data16 && sym.len() != label.len() {
                    // GAS 2.47: a 16-bit call field cannot carry a PLT32 —
                    // kept for parity even though the field is 4 bytes now
                    // (see encode_jmp for the full rationale).
                    return Err("4-byte relocation cannot be applied to 2-byte field".to_string());
                }
                self.bytes.push(0xE8);
                // `data16 call` keeps the full rel32 field: the 66 prefix
                // is dead on 64-bit near branches (see encode_jmp). A
                // `66 e8 rel16` pushes an 8-byte return address while
                // reading a 4-byte displacement — GAS's truncated form
                // both desynchronises the stream and computes a wild
                // target. `66 e8 rel32` (spliced dead prefix) is the
                // shortest VALID form; there is no short call row.
                self.add_relocation(sym, R_X86_64_PLT32, -4);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            Operand::Indirect(inner) => {
                match inner.as_ref() {
                    Operand::Register(reg) => {
                        let num = reg_num(&reg.name).ok_or("bad register")?;
                        // 16-bit targets take the operand-size prefix
                        // (`jmp *%ax` = 66 ff e0; without it the same
                        // bytes decode as the 64-bit `jmp *%rax` —
                        // GAS 2.47 byte-verified).
                        if is_reg16(&reg.name) {
                            self.bytes.push(0x66);
                        }
                        self.emit_rex_unary(4, &reg.name);
                        self.bytes.push(0xFF);
                        self.bytes.push(self.modrm(3, 2, num));
                        Ok(())
                    }
                    Operand::Memory(mem) => {
                        // `call *sym@tlscall(%reg)`: the @tlscall marker is
                        // NOT a displacement — GAS emits the plain
                        // register-indirect FF /2 with no disp and an
                        // R_X86_64_TLSDESC_CALL relocation on the
                        // instruction (byte-probed: `call *x@tlscall(%rax)`
                        // = ff 10). Anything but a bare base register is
                        // rejected ("@TLSCALL operator cannot be used...").
                        if let Displacement::SymbolMod(sym, modifier) = &mem.displacement
                            && modifier.eq_ignore_ascii_case("tlscall")
                        {
                            if mem.index.is_some() || mem.base.is_none() {
                                return Err(format!(
                                    "operand type mismatch for `call' after @tlscall"
                                ));
                            }
                            // GAS uses the MEMORY form (mod=00, no
                            // displacement): `call *x@tlscall(%rax)` =
                            // ff 10 — NOT the register form ff d0. A
                            // zero-displacement memory operand reuses the
                            // full SIB/disp0 ModRM machinery.
                            let base = mem.base.clone().unwrap();
                            let zero_mem = MemoryOperand {
                                segment: mem.segment.clone(),
                                displacement: Displacement::Integer(0),
                                base: Some(base),
                                index: None,
                                scale: None,
                                mask: None,
                                zeroing: false,
                                broadcast: None,
                            };
                            self.add_relocation(sym, R_X86_64_TLSDESC_CALL, 0);
                            self.emit_rex_rm(0, "", &zero_mem);
                            self.bytes.push(0xFF);
                            return self.encode_modrm_mem(2, &zero_mem);
                        }
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
        if self.apx_nf {
            return Err("{nf} unsupported for `xchg`".to_string());
        }
        if self.apx_evex {
            return Err("no EVEX encoding for `xchg`".to_string());
        }
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
                // Segment override before REX (see encode_inc_dec's Memory
                // arm for the kernel per-CPU context: this_cpu_xchg lowers
                // to `asm("xchg %[var], %[new]" ... "+m" (__my_cpu_var(...)))`,
                // and the override must survive the encoder).
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
                    let other = if is_accum(&src.name) {
                        Some(&dst.name)
                    } else if is_accum(&dst.name) {
                        Some(&src.name)
                    } else {
                        None
                    };
                    if let Some(other) = other {
                        let other_is_eax = size == 4 && is_accum(other);
                        if !other_is_eax {
                            let onum = reg_num(other).ok_or("bad register")?;
                            if size == 2 {
                                self.bytes.push(0x66);
                            }
                            // `xchg %rax,%rax` needs no REX.W: exchanging the
                            // accumulator with itself is a no-op either way, so
                            // GAS folds it to the canonical one-byte NOP.
                            if !(size == 8 && is_accum(other)) {
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
                // `.s` swaps the ModR/M roles — and therefore the REX.R/
                // REX.B assignments with them (GAS 2.47: `xchg.s %rdx,%rcx`
                // = 48 87 ca vs plain `xchg %rdx,%rcx` = 48 87 d1). The
                // accumulator short form above is unaffected (`xchg.s
                // %rax,%rcx` = 48 91, probed).
                let (reg_field, rm_field) = if self.s_flip {
                    (dst_num, src_num)
                } else {
                    (src_num, dst_num)
                };
                let (reg_name, rm_name) = if self.s_flip {
                    (&dst.name, &src.name)
                } else {
                    (&src.name, &dst.name)
                };
                self.emit_rex_rr(size, reg_name, rm_name);
                self.bytes.push(if size == 1 { 0x86 } else { 0x87 });
                self.bytes.push(self.modrm(3, reg_field, rm_field));
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
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rr(size, &src.name, &dst.name);
                self.bytes
                    .extend_from_slice(&[0x0F, if size == 1 { 0xB0 } else { 0xB1 }]);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
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
                // Segment override before REX (see encode_inc_dec's Memory
                // arm for the kernel per-CPU context: this_cpu_add_return
                // lowers to `lock xadd` on `+m` (__my_cpu_var(...))).
                self.emit_rex_rm(size, &src.name, mem);
                self.bytes
                    .extend_from_slice(&[0x0F, if size == 1 { 0xC0 } else { 0xC1 }]);
                self.encode_modrm_mem(src_num, mem)
            }
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rr(size, &src.name, &dst.name);
                self.bytes
                    .extend_from_slice(&[0x0F, if size == 1 { 0xC0 } else { 0xC1 }]);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
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
                // Prefix order: legacy F3 first, then REX immediately before
                // the opcode (GAS 2.47: `f3 42 0f ae`, NOT `42 f3 ...`). The
                // REX bit is mandatory when an r8-r15 base/index is present
                // (r12 index without REX.X silently re-decodes as base-only
                // addressing); W is NOT set — the instruction is m64-only.
                self.bytes.push(0xF3);
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
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
                // Prefix order as in rstorssp: F3, then REX, then opcode.
                self.bytes.push(0xF3);
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0xAE]);
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

    /// DIV/IDIV with GAS 2.47's optional accumulator-hint second operand.
    ///
    /// `div %cx,%ax` is legal: the FIRST operand is the real r/m (its
    /// register size selects the operation size — 16-bit → `66 f7 f1`),
    /// the second names the implied dividend and MUST be the size-matched
    /// accumulator (`al`/`ax`/`eax`/`rax`; `div %ecx,%rax` — GAS: "register
    /// type mismatch for `div'"). `idiv` behaves identically; `mul` does
    /// NOT accept the hint (dispatch checks the count). There is NO APX
    /// NDD form of div/idiv — every two-register spelling is the hint
    /// form, so this encoder intercepts ALL of them (the suffixless
    /// `div` reaches here after infer_suffix would otherwise pick the
    /// wrong size from the wrong operand, and `divw`/`divl` used to route
    /// 2-op shapes into the NDD path, emitting `f7 f1` for `div %cx,%ax`).
    pub(crate) fn encode_div_idiv(
        &mut self,
        ops: &[Operand],
        ext: u8,
        size: u8,
        mnem: &str,
    ) -> Result<(), String> {
        let (rm, hint) = match ops {
            [single] => (single, None),
            [a, b] => (a, Some(b)),
            _ => {
                return Err(format!("number of operands mismatch for `{mnem}'"));
            }
        };
        if let Some(h) = hint {
            let expected = match size {
                1 => "al",
                2 => "ax",
                4 => "eax",
                _ => "rax",
            };
            match h {
                Operand::Register(r) if r.name.eq_ignore_ascii_case(expected) => {}
                _ => return Err(format!("register type mismatch for `{mnem}'")),
            }
        }
        if size == 2 && !self.apx_wants_evex() {
            self.bytes.push(0x66);
        }
        self.encode_unary_rm(std::slice::from_ref(rm), ext, size)
    }

    /// String operations (MOVS/STOS/LODS/SCAS/CMPS/INS/OUTS), both spellings.
    ///
    /// Bare `movsb` emits the plain opcode. The explicit form
    /// (`movsb %fs:(%esi),%es:(%edi)`) carries GAS 2.47's segment law,
    /// every byte of it oracle-probed:
    ///
    ///  * the EDI-side operand must use `%es` or no segment at all — ES is
    ///    the architectural default, so an explicit `%es` is DROPPED, never
    ///    emitted as a redundant 0x26 (`stosb %ds:(%edi)` is REJECTED with
    ///    "`stos' operand 1 must use `%es' segment");
    ///  * the ESI-side operand may use any segment: `%ds` is dropped as its
    ///    default, every other segment is emitted (`%fs` → 0x64);
    ///  * base registers are NOT validated — `movsb (%rsi),(%rcx)` assembles
    ///    to a plain `a4` (the instruction uses RSI/RDI architecturally
    ///    regardless of what the operands named);
    ///  * prefix order (byte-probed): segment, 0x67, 0x66/REX.W, opcode —
    ///    `cmpsb %es:(%edi),%fs:(%esi)` = `64 67 a6`, `movsw (%esi),(%edi)`
    ///    = `67 66 a5`, `movsq (%rsi),(%rdi)` = `48 a5`.
    ///    (0x67 itself is NOT pushed here: the central address-size splice
    ///    in `encode` detects the 32-bit base registers of both memory
    ///    operands and inserts the byte in canonical position.)
    ///
    /// Operand-shape laws (byte-probed against 2.47):
    ///  * accumulator-carrying spellings exist for STOS and LODS only, with
    ///    the accumulator in its canonical slot and the size-matched name
    ///    (`stosb %al,(%rdi)` = aa, `stosq %rax,(%rdi)` = 48 ab,
    ///    `lodsq (%rsi),%rax` = 48 ad); a wrong register or a reversed
    ///    order is "operand type mismatch" (`stosb (%rdi),%al`,
    ///    `lodsb %rax,(%rsi)`);
    ///  * SCAS takes no register spelling (`scasb %eax,(%rdi)` rejected);
    ///  * INS/OUTS require both operands — the single-memory spelling is
    ///    "number of operands mismatch" (`insb (%dx)` rejected) — and the
    ///    port operand must be exactly `(%dx)` (`insb (%eax),…` rejected,
    ///    "`(%eax)' is not valid here (expected `(%dx)')");
    ///  * the ES-slot segment check names the operand by its 1-BASED
    ///    source position (`movsb (%rsi),%fs:(%rdi)` → "operand 2").
    ///
    /// `esi_idx`/`edi_idx` name the operand INDEX playing each role
    /// (`None` when the family has no such operand). The central
    /// operand-segment choke point in `encode` skips string ops (see
    /// `is_explicit_string_op`), so this is the ONLY place a string-op
    /// segment byte is decided.
    pub(crate) fn encode_string_op(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        size: u8,
        esi_idx: Option<usize>,
        edi_idx: Option<usize>,
        stem: &str,
    ) -> Result<(), String> {
        if ops.is_empty() {
            if size == 2 {
                self.bytes.push(0x66);
            } else if size == 8 {
                self.bytes.push(0x48); // REX.W
            }
            self.bytes.push(if size == 1 { opcode } else { opcode + 1 });
            return Ok(());
        }
        // INS/OUTS carry their port operand explicitly in GAS's templates:
        // the one-operand spellings are rejected outright (2.47:
        // `insb (%dx)' / `outsb (%rsi)' → "number of operands mismatch for
        // `ins'/`outs'"), unlike the accumulator-implicit stos/scas/lods.
        if ops.len() == 1 && matches!(stem, "ins" | "outs") {
            return Err(format!("number of operands mismatch for `{stem}'"));
        }
        // Two-operand INS/OUTS: the port is `(%dx)` or the `%dx` register
        // (2.47 accepts `insb (%dx),%es:(%edi)` AND `insb %dx,(%rdi)` =
        // 6c; `outsb (%rsi),%dx` = 6e), the address operand follows the
        // family's slot law — INS addresses EDI (reject non-%es, 2.47:
        // "`ins' operand 2 must use `%es' segment"), OUTS addresses ESI
        // (keep every non-%ds override, 2.47: `outsb %es:(%rsi),%dx` =
        // 26 6e) — and the order is fixed (`outsb %dx,(%esi)` is
        // "operand type mismatch for `outs'").
        if ops.len() == 2 && matches!(stem, "ins" | "outs") {
            let port_is_dx = |op: &Operand| match op {
                Operand::Register(r) => r.name.eq_ignore_ascii_case("dx"),
                Operand::Memory(m) => {
                    m.base
                        .as_ref()
                        .is_some_and(|b| b.name.eq_ignore_ascii_case("dx"))
                        && m.index.is_none()
                }
                _ => false,
            };
            let addr_idx = if stem == "ins" { 1 } else { 0 };
            if !port_is_dx(&ops[1 - addr_idx]) {
                return Err(format!("operand type mismatch for `{stem}'"));
            }
            let mem =
                mem_of(&ops[addr_idx]).ok_or(format!("operand type mismatch for `{stem}'"))?;
            match (stem, mem.segment.as_deref()) {
                ("ins", None | Some("es")) => {}
                ("ins", Some(_)) => {
                    return Err(format!(
                        "`{stem}' operand {} must use `%es' segment",
                        addr_idx + 1
                    ));
                }
                ("outs", None | Some("ds")) => {}
                ("outs", Some(other)) => {
                    let byte = match other {
                        "es" => 0x26,
                        "cs" => 0x2E,
                        "ss" => 0x36,
                        "fs" => 0x64,
                        "gs" => 0x65,
                        _ => return Err(format!("unsupported segment override: %{other}")),
                    };
                    self.bytes.push(byte);
                }
                _ => unreachable!("stem is ins or outs"),
            }
            if size == 2 {
                self.bytes.push(0x66);
            } else if size == 8 {
                self.bytes.push(0x48); // REX.W
            }
            self.bytes.push(if size == 1 { opcode } else { opcode + 1 });
            return Ok(());
        }
        // Single explicit operand: the stos/scas EDI side, the lods ESI
        // side (`scasb %es:(%edi)` = 67 ae, GAS-probed — one memory
        // operand, no second dummy).
        if ops.len() == 1 && edi_idx.is_some() && esi_idx.is_none() {
            let mem = mem_of(&ops[0]).ok_or(format!("operand type mismatch for `{stem}'"))?;
            match mem.segment.as_deref() {
                None | Some("es") => {}
                Some(_) => {
                    return Err(format!("`{stem}' operand 1 must use `%es' segment"));
                }
            }
            if size == 2 {
                self.bytes.push(0x66);
            } else if size == 8 {
                self.bytes.push(0x48);
            }
            self.bytes.push(if size == 1 { opcode } else { opcode + 1 });
            return Ok(());
        }
        if ops.len() == 1 && esi_idx.is_some() && edi_idx.is_none() {
            let mem = mem_of(&ops[0]).ok_or(format!("operand type mismatch for `{stem}'"))?;
            let seg_byte = match mem.segment.as_deref() {
                None | Some("ds") => None,
                Some("es") => Some(0x26),
                Some("cs") => Some(0x2E),
                Some("ss") => Some(0x36),
                Some("fs") => Some(0x64),
                Some("gs") => Some(0x65),
                Some(other) => {
                    return Err(format!("unsupported segment override: %{other}"));
                }
            };
            if let Some(b) = seg_byte {
                self.bytes.push(b);
            }
            if size == 2 {
                self.bytes.push(0x66);
            } else if size == 8 {
                self.bytes.push(0x48);
            }
            self.bytes.push(if size == 1 { opcode } else { opcode + 1 });
            return Ok(());
        }
        if ops.len() != 2 {
            return Err(format!("number of operands mismatch for `{stem}'"));
        }
        fn mem_of(op: &Operand) -> Option<&MemoryOperand> {
            match op {
                Operand::Memory(m) => Some(m),
                _ => None,
            }
        }
        // Accumulator-carrying spellings (2.47-probed): STOS leads with the
        // accumulator (`stosb %al,(%rdi)` = aa, `stosq %rax,(%rdi)` = 48 ab)
        // and LODS trails with it (`lodsq (%rsi),%rax` = 48 ad). The register
        // must be the size-matched accumulator and the order is fixed; SCAS
        // has no register spelling (2.47: `scasb %eax,(%rdi)` is "operand
        // type mismatch"), and both fall through to the memory-only path,
        // whose `mem_of` rejects a register operand exactly like GAS.
        if edi_idx.is_some() && esi_idx.is_none() && stem == "stos" {
            if !is_size_matched_accumulator(&ops[0], size) {
                return Err(format!("operand type mismatch for `{stem}'"));
            }
            let mem = mem_of(&ops[1]).ok_or(format!("operand type mismatch for `{stem}'"))?;
            match mem.segment.as_deref() {
                None | Some("es") => {}
                Some(_) => {
                    return Err(format!("`{stem}' operand 2 must use `%es' segment"));
                }
            }
            if size == 2 {
                self.bytes.push(0x66);
            } else if size == 8 {
                self.bytes.push(0x48); // REX.W
            }
            self.bytes.push(if size == 1 { opcode } else { opcode + 1 });
            return Ok(());
        }
        if esi_idx.is_some() && edi_idx.is_none() && stem == "lods" {
            let mem = mem_of(&ops[0]).ok_or(format!("operand type mismatch for `{stem}'"))?;
            if !is_size_matched_accumulator(&ops[1], size) {
                return Err(format!("operand type mismatch for `{stem}'"));
            }
            let seg_byte = match mem.segment.as_deref() {
                None | Some("ds") => None,
                Some("es") => Some(0x26),
                Some("cs") => Some(0x2E),
                Some("ss") => Some(0x36),
                Some("fs") => Some(0x64),
                Some("gs") => Some(0x65),
                Some(other) => {
                    return Err(format!("unsupported segment override: %{other}"));
                }
            };
            if let Some(b) = seg_byte {
                self.bytes.push(b);
            }
            if size == 2 {
                self.bytes.push(0x66);
            } else if size == 8 {
                self.bytes.push(0x48); // REX.W
            }
            self.bytes.push(if size == 1 { opcode } else { opcode + 1 });
            return Ok(());
        }
        let m0 = mem_of(&ops[0]).ok_or(format!("operand type mismatch for `{stem}'"))?;
        let m1 = mem_of(&ops[1]).ok_or(format!("operand type mismatch for `{stem}'"))?;
        let mems = [m0, m1];

        // The port operand of INS/OUTS must be spelled `(%dx)` (2.47:
        // `insb (%eax),%es:(%edi)' → "`(%eax)' is not valid here (expected
        // `(%dx)')"). The ESI side of OUTS and the EDI side of INS stay
        // unvalidated — a bare base register is trusted like everywhere
        // else in this family.
        if stem == "ins" {
            let port_ok = mems[0]
                .base
                .as_ref()
                .is_some_and(|b| b.name.eq_ignore_ascii_case("dx"));
            if !port_ok {
                return Err(format!("operand type mismatch for `{stem}'"));
            }
        }
        if stem == "outs" {
            let port_ok = mems[1]
                .base
                .as_ref()
                .is_some_and(|b| b.name.eq_ignore_ascii_case("dx"));
            if !port_ok {
                return Err(format!("operand type mismatch for `{stem}'"));
            }
        }

        // EDI side: %es or nothing.
        if let Some(i) = edi_idx {
            match mems[i].segment.as_deref() {
                None | Some("es") => {}
                Some(_) => {
                    return Err(format!("`{stem}' operand {} must use `%es' segment", i + 1));
                }
            }
        }
        // ESI side: any segment; %ds is the default and dropped.
        let seg_byte = match esi_idx.and_then(|i| mems[i].segment.as_deref()) {
            None | Some("ds") => None,
            Some("es") => Some(0x26),
            Some("cs") => Some(0x2E),
            Some("ss") => Some(0x36),
            Some("fs") => Some(0x64),
            Some("gs") => Some(0x65),
            Some(other) => {
                return Err(format!("unsupported segment override: %{other}"));
            }
        };

        if let Some(b) = seg_byte {
            self.bytes.push(b);
        }
        if size == 2 {
            self.bytes.push(0x66);
        } else if size == 8 {
            self.bytes.push(0x48); // REX.W
        }
        self.bytes.push(if size == 1 { opcode } else { opcode + 1 });
        Ok(())
    }
}

/// True when `op` is the size-matched accumulator register operand of a
/// string op: `%al`/`%ax`/`%eax`/`%rax` for element sizes 1/2/4/8 (GAS 2.47
/// accepts `stosb %al,(%rdi)` and `stosq %rax,(%rdi)` but rejects
/// `stosb %rax,(%rdi)` — "`%rax' not allowed with `stosb'" — and
/// `stosb %cl,(%rdi)` — "operand type mismatch").
pub(crate) fn is_size_matched_accumulator(op: &Operand, size: u8) -> bool {
    let Operand::Register(reg) = op else {
        return false;
    };
    let name = reg.name.to_ascii_lowercase();
    let wanted = match size {
        1 => "al",
        2 => "ax",
        4 => "eax",
        8 => "rax",
        _ => return false,
    };
    name == wanted
}

/// Element size (1/2/4/8) of a sized string-op mnemonic suffix.
pub(crate) fn string_op_size(mnemonic: &str) -> Result<u8, String> {
    Ok(match mnemonic.as_bytes().last() {
        Some(b'b') => 1,
        Some(b'w') => 2,
        Some(b'l') | Some(b'd') => 4,
        Some(b'q') => 8,
        _ => return Err(format!("bad string-op mnemonic: {mnemonic}")),
    })
}

/// True when any operand is a 32-bit GP register — legacy helper kept for
/// callers that want a blunt "is a 32-bit spelling present" predicate.
pub(crate) fn addr_hint_is32(ops: &[Operand]) -> bool {
    ops.iter().any(|op| match op {
        Operand::Register(r) => {
            matches!(
                r.name.to_ascii_lowercase().as_str(),
                "eax" | "ebx" | "ecx" | "edx" | "esi" | "edi" | "ebp" | "esp"
            )
        }
        _ => false,
    })
}

/// One MONITOR-family implicit slot: the register NUMBER (0=A, 1=C, 2=D,
/// 3=B) and the width class (4 or 8) of a 32/64-bit spelling. 16/8-bit
/// spellings and every other name return `None` — the caller splits them
/// into GAS's "operand size mismatch" (16/8-bit) vs "operand type
/// mismatch" (EGPRs, memory, anything else) diagnostics.
fn monitor_slot(name: &str) -> Option<(u8, u8)> {
    match name {
        "rax" => Some((0, 8)),
        "eax" => Some((0, 4)),
        "rcx" => Some((1, 8)),
        "ecx" => Some((1, 4)),
        "rdx" => Some((2, 8)),
        "edx" => Some((2, 4)),
        "rbx" => Some((3, 8)),
        "ebx" => Some((3, 4)),
        _ => None,
    }
}

/// The GAS 2.47 monitor-family law, matrix-probed on 2.47.20260726 over
/// all width/register combinations of both modes. The implicit register
/// NUMBERS are fixed per template — monitor/monitorx (0,1,2) = A,C,D;
/// mwait (0,1) = A,C; mwaitx (0,1,3) = A,C,B — and every operand is a
/// 32/64-bit spelling of its slot register: a 16/8-bit spelling is
/// "operand size mismatch", an EGPR or any other register is "operand
/// type mismatch".
///
/// Width classes (the subtle part, all probe-verified):
///   * monitor/monitorx pin slots 1+2 to ONE width class and leave the
///     hint free: `monitor %rax,%ecx,%edx` = 0f 01 c8 (no 67!),
///     `monitor %eax,%rcx,%rdx` = 67 0f 01 c8, `monitor %rax,%ecx,%rdx`
///     is REJECTED (slots 1+2 mixed widths), `monitor %eax,%ecx,%ebx` is
///     REJECTED (slot 2 register number).
///   * mwait/mwaitx pin ALL slots to one width class:
///     `mwait %eax,%rcx` is "register type mismatch".
///   * only monitor/monitorx carry the 0x67 address-size law, and it is
///     driven by the hint spelling alone (`mwaitx %eax,%ecx,%ebx` =
///     0f 01 fb, never 67).
///
/// Returns `Ok(needs_67)`.
pub(crate) fn check_monitor_family(mnemonic: &str, ops: &[Operand]) -> Result<bool, String> {
    let type_err = || format!("operand type mismatch for `{mnemonic}'");
    let size_err = || format!("operand size mismatch for `{mnemonic}'");
    let regs: Vec<(u8, u8)> = ops
        .iter()
        .map(|op| match op {
            Operand::Register(r) => {
                let n = r.name.to_ascii_lowercase();
                match monitor_slot(&n) {
                    Some(slot) => Ok(slot),
                    None if matches!(
                        n.as_str(),
                        "ax" | "al" | "cx" | "cl" | "dx" | "dl" | "bx" | "bl"
                    ) =>
                    {
                        Err(size_err())
                    }
                    None => Err(type_err()),
                }
            }
            _ => Err(type_err()),
        })
        .collect::<Result<_, _>>()?;
    match (mnemonic, regs.as_slice()) {
        ("monitor" | "monitorx", [(0, w0), (1, w1), (2, w2)]) => {
            if w1 != w2 {
                return Err(type_err());
            }
            Ok(*w0 == 4)
        }
        ("mwait", [(0, w0), (1, w1)]) if w0 == w1 => Ok(false),
        ("mwaitx", [(0, w0), (1, w1), (3, w2)]) if w0 == w1 && w1 == w2 => Ok(false),
        _ => Err(type_err()),
    }
}

/// True when any operand is a register — used to route `movs{b,w,l}` with
/// register operands to their MOVSX spellings and keep two-memory-operand
/// forms on the string path.
pub(crate) fn has_reg_operand(ops: &[Operand]) -> bool {
    ops.iter().any(|op| matches!(op, Operand::Register(_)))
}

/// True when the instruction is an explicit string op with MEMORY operands
/// (one or two): the central operand-segment choke point must NOT emit a
/// segment byte for these (encode_string_op applies the ES/DS default law
/// itself, and an explicit `%es`/`%ds` is DROPPED as the architectural
/// default).
pub(crate) fn is_explicit_string_op(mnemonic: &str, ops: &[Operand]) -> bool {
    if ops.is_empty() {
        return false;
    }
    if ops.len() > 2 {
        return false;
    }
    let m = mnemonic.to_ascii_lowercase();
    let m = m.strip_suffix(".s").unwrap_or(&m);
    let stem = match m.as_bytes().last() {
        Some(b'b' | b'w' | b'l' | b'd' | b'q') => &m[..m.len() - 1],
        _ => return false,
    };
    // All-memory shapes: every family's register-free spelling, including
    // the SSE-collision spellings (`movsd (%esi),(%edi)` is the string op
    // while `movsd %xmm0,(%edi)` stays SSE — the XMM register operand
    // disqualifies the shape here).
    if ops.iter().all(|op| matches!(op, Operand::Memory(_))) {
        return matches!(
            stem,
            "movs" | "stos" | "lods" | "scas" | "cmps" | "ins" | "outs"
        );
    }
    // Register-carrying STOS/LODS spellings (`stosb %al,%es:(%rdi)`,
    // `lodsq %fs:(%rsi),%rax`) and INS/OUTS with the `%dx` register port
    // (`outsb %fs:(%rsi),%dx`): encode_string_op owns their segment law
    // too, so the central choke point must skip them or it would splice a
    // second override around the arm's decision (`stosb %al,%es:(%rdi)`
    // would come out as `26 aa` instead of `aa`, `outsb %fs:(%rsi),%dx`
    // as `64 64 6e` instead of `64 6e`). MOVS/SCAS with a register
    // operand are MOVSX/segment-error shapes and keep the generic law.
    if ops.len() == 2 && matches!(stem, "stos" | "lods" | "ins" | "outs") {
        let mems = ops
            .iter()
            .filter(|op| matches!(op, Operand::Memory(_)))
            .count();
        let regs = ops
            .iter()
            .filter(|op| matches!(op, Operand::Register(_)))
            .count();
        if mems == 1 && regs == 1 {
            return true;
        }
    }
    false
}
