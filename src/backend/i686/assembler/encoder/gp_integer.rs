//! General-purpose integer instruction encoders for i686.
//!
//! MOV, LEA, PUSH/POP, ALU, TEST, IMUL, shifts, bit operations,
//! conditional set/move, jumps, calls, exchange, and misc GP instructions.

use super::*;

fn infer_movext_dst_size(ops: &[Operand]) -> Result<u8, String> {
    if ops.len() != 2 {
        return Err("mov extension requires 2 operands".to_string());
    }
    match &ops[1] {
        Operand::Register(dst) => {
            let size = reg_size(&dst.name);
            if size == 2 || size == 4 {
                Ok(size)
            } else {
                Err(format!("mov extension destination must be 16/32-bit GP register: {}", dst.name))
            }
        }
        _ => Err("mov extension destination must be a register".to_string()),
    }
}

impl super::InstructionEncoder {
    // ---- Instruction-specific encoders ----

    pub(super) fn encode_mov(&mut self, ops: &[Operand], size: u8) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("mov requires 2 operands, got {}", ops.len()));
        }

        // Check for control register moves
        if let (Operand::Register(r1), Operand::Register(r2)) = (&ops[0], &ops[1]) {
            if is_control_reg(&r1.name) || is_control_reg(&r2.name) {
                return self.encode_mov_cr(ops);
            }
            if is_segment_reg(&r1.name) || is_segment_reg(&r2.name) {
                return self.encode_mov_seg(ops);
            }
        }
        // Check for segment register moves involving memory
        if let (Operand::Register(r), Operand::Memory(_)) = (&ops[0], &ops[1]) {
            if is_segment_reg(&r.name) {
                return self.encode_mov_seg(ops);
            }
        }
        if let (Operand::Memory(_), Operand::Register(r)) = (&ops[0], &ops[1]) {
            if is_segment_reg(&r.name) {
                return self.encode_mov_seg(ops);
            }
        }

        match (&ops[0], &ops[1]) {
            (Operand::Immediate(imm), Operand::Register(dst)) => {
                self.encode_mov_imm_reg(imm, dst, size)
            }
            (Operand::Register(src), Operand::Register(dst)) => {
                self.encode_mov_rr(src, dst, size)
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                self.encode_mov_mem_reg(mem, dst, size)
            }
            (Operand::Register(src), Operand::Memory(mem)) => {
                self.encode_mov_reg_mem(src, mem, size)
            }
            (Operand::Immediate(imm), Operand::Memory(mem)) => {
                self.encode_mov_imm_mem(imm, mem, size)
            }
            // Label as memory source: movl symbol, %reg (absolute address)
            (Operand::Label(label), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or_else(|| format!("bad register: {}", dst.name))?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                // GAS uses the accumulator moffs shortform: A0 (byte) / A1
                // (word/dword) followed by the bare address -- one byte
                // shorter than the ModRM form. Match it exactly (asm-diff
                // oracle: `movl sym, %eax` == a1 <disp32>).
                if dst_num == 0 {
                    self.bytes.push(if size == 1 { 0xA0 } else { 0xA1 });
                    return self.encode_abs_addr_disp_only(label);
                }
                self.bytes.push(if size == 1 { 0x8A } else { 0x8B });
                self.encode_abs_addr_modrm(dst_num, label)
            }
            // Label as memory destination: movl %reg, symbol
            (Operand::Register(src), Operand::Label(label)) => {
                let src_num = reg_num(&src.name).ok_or_else(|| format!("bad register: {}", src.name))?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                // Accumulator shortform: A2 (byte) / A3 (word/dword).
                if src_num == 0 {
                    self.bytes.push(if size == 1 { 0xA2 } else { 0xA3 });
                    return self.encode_abs_addr_disp_only(label);
                }
                self.bytes.push(if size == 1 { 0x88 } else { 0x89 });
                self.encode_abs_addr_modrm(src_num, label)
            }
            // movl $imm, symbol (immediate to memory at absolute address)
            (Operand::Immediate(imm), Operand::Label(label)) => {
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(if size == 1 { 0xC6 } else { 0xC7 });
                self.bytes.push(self.modrm(0, 0, 5));
                if let Ok(addr) = label.parse::<i64>() {
                    self.bytes.extend_from_slice(&(addr as i32).to_le_bytes());
                } else {
                    self.add_relocation_for_label(label, R_386_32);
                    self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                }
                match imm {
                    ImmediateValue::Integer(val) => {
                        match size {
                            1 => self.bytes.push(*val as u8),
                            2 => self.bytes.extend_from_slice(&(*val as i16).to_le_bytes()),
                            4 => self.bytes.extend_from_slice(&(*val as i32).to_le_bytes()),
                            _ => unreachable!(),
                        }
                    }
                    _ => return Err("unsupported immediate for mov to label address".to_string()),
                }
                Ok(())
            }
            _ => Err("unsupported mov operand combination".to_string()),
        }
    }

    /// Handle unsuffixed `mov` from inline asm - infer size from operands
    pub(super) fn encode_mov_infer_size(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("mov requires 2 operands, got {}", ops.len()));
        }
        // Infer size from register operands
        let size = match (&ops[0], &ops[1]) {
            (Operand::Register(r), _) => reg_size(&r.name),
            (_, Operand::Register(r)) => reg_size(&r.name),
            _ => 4, // default to 32-bit
        };
        self.encode_mov(ops, size)
    }

    fn encode_mov_imm_reg(&mut self, imm: &ImmediateValue, dst: &Register, size: u8) -> Result<(), String> {
        let dst_num = reg_num(&dst.name).ok_or_else(|| format!("bad register: {}", dst.name))?;

        match imm {
            ImmediateValue::Integer(val) => {
                let val = *val;
                if size == 4 {
                    // movl $imm32, %reg - compact B8+rd id. sized_op MUST be
                    // set: in .code16 the 32-bit form is 66 B8+rd id (GAS:
                    // `66 b8 78 56 34 12`); without the marker the prefix
                    // inversion skipped it and the imm32 was decoded as
                    // imm16 + stray bytes (silent wrong code).
                    self.sized_op = true;
                    self.bytes.push(0xB8 + dst_num);
                    self.bytes.extend_from_slice(&(val as i32).to_le_bytes());
                } else if size == 2 {
                    self.sized_op = true; self.bytes.push(0x66);
                    self.bytes.push(0xB8 + dst_num);
                    self.bytes.extend_from_slice(&(val as i16).to_le_bytes());
                } else {
                    // 8-bit
                    self.bytes.push(0xB0 + dst_num);
                    self.bytes.push(val as u8);
                }
            }
            ImmediateValue::Symbol(sym) | ImmediateValue::SymbolPlusOffset(sym, _) => {
                let addend = if let ImmediateValue::SymbolPlusOffset(_, a) = imm { *a } else { 0 };
                if size == 4 {
                    self.sized_op = true;
                    self.bytes.push(0xB8 + dst_num);
                    self.add_relocation(sym, R_386_32, addend);
                    self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                } else if size == 2 {
                    // Real-mode boot code (header.S with .code16 semantics):
                    // `movw $_end, %dx` — a 16-bit absolute symbol immediate.
                    // GAS emits B8+rd iw with R_386_16 / R_X86_64_16 on the
                    // 2-byte field; the kernel's build-time relocs tool
                    // whitelists 16-bit relocs in the realmode blob.
                    // (The caller has already emitted the 0x66 prefix in
                    // 32-bit code mode when needed.)
                    self.bytes.push(0xB8 + dst_num);
                    self.add_relocation(sym, R_386_16, addend);
                    self.bytes.extend_from_slice(&[0, 0]);
                } else {
                    return Err("symbol immediate only supported for 16/32-bit mov".to_string());
                }
            }
            ImmediateValue::SymbolDiff(sym_a, sym_b) => {
                // head_64.S (compressed boot, .code32): `movl $(_bss -
                // startup_32), %ecx` and the rva(X) macro produce label
                // differences as mov immediates. Same-section pairs fold to
                // a constant after layout via the diff-reloc path — GAS
                // emits `b9 <imm32>` with the folded value.
                if size == 4 {
                    self.bytes.push(0xB8 + dst_num);
                    self.add_relocation_with_diff(sym_a, R_386_32, 0, sym_b);
                    self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                } else {
                    return Err("symbol-difference immediate only supported for 32-bit mov".to_string());
                }
            }
            ImmediateValue::SymbolMod(_, _) => {
                return Err("unsupported immediate type for mov".to_string());
            }
        }
        Ok(())
    }

    fn encode_mov_rr(&mut self, src: &Register, dst: &Register, size: u8) -> Result<(), String> {
        // Handle segment register moves
        if let Some(seg_num) = seg_reg_num(&dst.name) {
            // mov %r16, %sreg (8E /r)
            let src_num = reg_num(&src.name).ok_or_else(|| format!("bad register: {}", src.name))?;
            self.bytes.push(0x8E);
            self.bytes.push(self.modrm(3, seg_num, src_num));
            return Ok(());
        }
        if let Some(seg_num) = seg_reg_num(&src.name) {
            // mov %sreg, %r16 (8C /r)
            let dst_num = reg_num(&dst.name).ok_or_else(|| format!("bad register: {}", dst.name))?;
            self.bytes.push(0x8C);
            self.bytes.push(self.modrm(3, seg_num, dst_num));
            return Ok(());
        }

        let src_num = reg_num(&src.name).ok_or_else(|| format!("bad register: {}", src.name))?;
        let dst_num = reg_num(&dst.name).ok_or_else(|| format!("bad register: {}", dst.name))?;

        if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 {
            self.bytes.push(0x66);
        }
        if size == 1 {
            self.bytes.push(0x88);
        } else {
            self.bytes.push(0x89);
        }
        self.bytes.push(self.modrm(3, src_num, dst_num));
        Ok(())
    }

    fn encode_mov_mem_reg(&mut self, mem: &MemoryOperand, dst: &Register, size: u8) -> Result<(), String> {
        let dst_num = reg_num(&dst.name).ok_or_else(|| format!("bad register: {}", dst.name))?;

        if let Some(ref seg) = mem.segment {
            match seg.as_str() {
                "fs" => self.bytes.push(0x64),
                "gs" => self.bytes.push(0x65),
                _ => return Err(format!("unsupported segment: {}", seg)),
            }
        }

        if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 {
            self.bytes.push(0x66);
        }
        if size == 1 {
            self.bytes.push(0x8A);
        } else {
            self.bytes.push(0x8B);
        }
        self.encode_modrm_mem(dst_num, mem)
    }

    fn encode_mov_reg_mem(&mut self, src: &Register, mem: &MemoryOperand, size: u8) -> Result<(), String> {
        let src_num = reg_num(&src.name).ok_or_else(|| format!("bad register: {}", src.name))?;

        if let Some(ref seg) = mem.segment {
            match seg.as_str() {
                "fs" => self.bytes.push(0x64),
                "gs" => self.bytes.push(0x65),
                _ => return Err(format!("unsupported segment: {}", seg)),
            }
        }

        if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 {
            self.bytes.push(0x66);
        }
        if size == 1 {
            self.bytes.push(0x88);
        } else {
            self.bytes.push(0x89);
        }
        self.encode_modrm_mem(src_num, mem)
    }

    fn encode_mov_imm_mem(&mut self, imm: &ImmediateValue, mem: &MemoryOperand, size: u8) -> Result<(), String> {
        if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 {
            self.bytes.push(0x66);
        }
        if size == 1 {
            self.bytes.push(0xC6);
        } else {
            self.bytes.push(0xC7);
        }
        self.encode_modrm_mem(0, mem)?;

        match imm {
            ImmediateValue::Integer(val) => {
                match size {
                    1 => self.bytes.push(*val as u8),
                    2 => self.bytes.extend_from_slice(&(*val as i16).to_le_bytes()),
                    4 => self.bytes.extend_from_slice(&(*val as i32).to_le_bytes()),
                    _ => unreachable!(),
                }
            }
            ImmediateValue::Symbol(sym) | ImmediateValue::SymbolPlusOffset(sym, _) => {
                let addend = if let ImmediateValue::SymbolPlusOffset(_, a) = imm { *a } else { 0 };
                if size == 4 {
                    self.add_relocation(sym, R_386_32, addend);
                    self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                } else {
                    return Err("symbol immediate only supported for 32-bit mov to memory".to_string());
                }
            }
            _ => return Err("unsupported immediate for mov to memory".to_string()),
        }
        Ok(())
    }

    pub(super) fn encode_movsx(&mut self, ops: &[Operand], src_size: u8, dst_size: u8) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("movsx requires 2 operands".to_string());
        }

        // movsx/movzx ALWAYS pick a destination width; sized_op must be set
        // for BOTH forms so the .code16 inversion adds 66 to the 32-bit form
        // (GAS: 66 0f b6 in .code32-for-16bit-dst, 0f b6 bare in .code16 for
        // 16-bit dst, 66 0f b6 in .code16 for 32-bit dst).
        self.sized_op = true;
        if dst_size == 2 { self.bytes.push(0x66); }

        let opcode = match src_size {
            1 => vec![0x0F, 0xBE],
            2 => vec![0x0F, 0xBF],
            _ => return Err(format!("unsupported movsx src size: {}", src_size)),
        };

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                self.bytes.extend_from_slice(&opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                self.emit_segment_prefix(mem);
                self.bytes.extend_from_slice(&opcode);
                self.encode_modrm_mem(dst_num, mem)?;
            }
            // Absolute address as a bare label: `movswl sym, %eax`.
            (Operand::Label(label), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                self.bytes.extend_from_slice(&opcode);
                self.encode_abs_addr_modrm(dst_num, label)?;
            }
            _ => return Err("unsupported movsx operands".to_string()),
        }
        Ok(())
    }

    pub(super) fn encode_movzx(&mut self, ops: &[Operand], src_size: u8, dst_size: u8) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("movzx requires 2 operands".to_string());
        }

        // movsx/movzx ALWAYS pick a destination width; sized_op must be set
        // for BOTH forms so the .code16 inversion adds 66 to the 32-bit form
        // (GAS: 66 0f b6 in .code32-for-16bit-dst, 0f b6 bare in .code16 for
        // 16-bit dst, 66 0f b6 in .code16 for 32-bit dst).
        self.sized_op = true;
        if dst_size == 2 { self.bytes.push(0x66); }

        let opcode = match src_size {
            1 => vec![0x0F, 0xB6],
            2 => vec![0x0F, 0xB7],
            _ => return Err(format!("unsupported movzx src size: {}", src_size)),
        };

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                self.bytes.extend_from_slice(&opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                self.emit_segment_prefix(mem);
                self.bytes.extend_from_slice(&opcode);
                self.encode_modrm_mem(dst_num, mem)?;
            }
            // Absolute address given as a bare label: `movzwl sym, %eax`.
            // The parser yields Operand::Label here, not a MemoryOperand, and
            // without this arm the whole instruction was rejected. Routing it
            // through the shared helper also gets the 16-bit form right in
            // `.code16` (rm=110 + disp16 instead of rm=101 + disp32).
            (Operand::Label(label), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                self.bytes.extend_from_slice(&opcode);
                self.encode_abs_addr_modrm(dst_num, label)?;
            }
            _ => return Err("unsupported movzx operands".to_string()),
        }
        Ok(())
    }

    pub(super) fn encode_movsx_infer_dst(&mut self, ops: &[Operand], src_size: u8) -> Result<(), String> {
        let dst_size = infer_movext_dst_size(ops)?;
        self.encode_movsx(ops, src_size, dst_size)
    }

    pub(super) fn encode_movzx_infer_dst(&mut self, ops: &[Operand], src_size: u8) -> Result<(), String> {
        let dst_size = infer_movext_dst_size(ops)?;
        self.encode_movzx(ops, src_size, dst_size)
    }

    pub(super) fn encode_lea(&mut self, ops: &[Operand], _size: u8) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("lea requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                self.bytes.push(0x8D);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("lea requires memory source and register destination".to_string()),
        }
    }

    pub(super) fn encode_push(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("push requires 1 operand".to_string());
        }
        // `pushl`/`popl` push the mode's 32-bit operand: no prefix in 32-bit
        // mode, but 0x66 in `.code16`. Record the size choice so the .code16
        // inversion adds it (GAS: `pushl %eax` in real mode is `66 50`).
        self.sized_op = true;
        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                self.bytes.push(0x50 + num);
                Ok(())
            }
            Operand::Immediate(ImmediateValue::Integer(val)) => {
                if *val >= -128 && *val <= 127 {
                    self.bytes.push(0x6A);
                    self.bytes.push(*val as u8);
                } else {
                    self.bytes.push(0x68);
                    self.bytes.extend_from_slice(&(*val as i32).to_le_bytes());
                }
                Ok(())
            }
            Operand::Immediate(ImmediateValue::Symbol(sym)) |
            Operand::Immediate(ImmediateValue::SymbolPlusOffset(sym, _)) => {
                let addend = if let Operand::Immediate(ImmediateValue::SymbolPlusOffset(_, a)) = &ops[0] { *a } else { 0 };
                self.bytes.push(0x68);
                self.add_relocation(sym, R_386_32, addend);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            Operand::Memory(mem) => {
                self.bytes.push(0xFF);
                self.encode_modrm_mem(6, mem)
            }
            _ => Err("unsupported push operand".to_string()),
        }
    }

    pub(super) fn encode_push16(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("pushw requires 1 operand".to_string());
        }
        match &ops[0] {
            // `pushw %cx` -- 16-bit register push (0x66 prefix + 0x50+rd).
            // Segment registers keep their own one-byte opcodes and take no
            // prefix. Mirrors `encode_pop16`, which already supported all of
            // these; only the push side was immediate-only, so the kernel's
            // `pushw %cx` in arch/x86/boot/startup/efi-mixed.S failed.
            Operand::Register(reg) => {
                if is_segment_reg(&reg.name) {
                    match reg.name.as_str() {
                        "es" => { self.bytes.push(0x06); Ok(()) }
                        "cs" => { self.bytes.push(0x0E); Ok(()) }
                        "ss" => { self.bytes.push(0x16); Ok(()) }
                        "ds" => { self.bytes.push(0x1E); Ok(()) }
                        "fs" => { self.bytes.extend_from_slice(&[0x0F, 0xA0]); Ok(()) }
                        "gs" => { self.bytes.extend_from_slice(&[0x0F, 0xA8]); Ok(()) }
                        _ => Err(format!("cannot push {}", reg.name)),
                    }
                } else {
                    let num = reg_num(&reg.name).ok_or("bad register")?;
                    self.sized_op = true; self.bytes.push(0x66);
                    self.bytes.push(0x50 + num);
                    Ok(())
                }
            }
            // `pushw mem` -- 0x66 FF /6.
            Operand::Memory(mem) => {
                self.sized_op = true; self.bytes.push(0x66);
                self.bytes.push(0xFF);
                self.encode_modrm_mem(6, mem)
            }
            Operand::Immediate(ImmediateValue::Integer(val)) => {
                self.sized_op = true; self.bytes.push(0x66);
                if *val >= -128 && *val <= 127 {
                    self.bytes.push(0x6A);
                    self.bytes.push(*val as u8);
                } else {
                    self.bytes.push(0x68);
                    self.bytes.extend_from_slice(&(*val as i16).to_le_bytes());
                }
                Ok(())
            }
            // `pushw $sym` (header.S: `pushw $6f` lretw trampoline):
            // 68 imm16 + R_386_16 in .code16. No valid GAS form outside
            // 16-bit mode, so reject loudly there.
            Operand::Immediate(ImmediateValue::Symbol(sym)) |
            Operand::Immediate(ImmediateValue::SymbolPlusOffset(sym, _)) => {
                let addend = if let Operand::Immediate(ImmediateValue::SymbolPlusOffset(_, a)) = &ops[0] { *a } else { 0 };
                if !self.code16 {
                    return Err("pushw $symbol is only supported in .code16 mode".to_string());
                }
                self.bytes.push(0x68);
                self.add_relocation(sym, R_386_16, addend);
                self.bytes.extend_from_slice(&[0, 0]);
                Ok(())
            }
            _ => Err("unsupported pushw operand".to_string()),
        }
    }

    pub(super) fn encode_pop(&mut self, ops: &[Operand]) -> Result<(), String> {
        self.sized_op = true;
        if ops.len() != 1 {
            return Err("pop requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                if is_segment_reg(&reg.name) {
                    // Pop to segment register
                    match reg.name.as_str() {
                        "es" => { self.bytes.push(0x07); Ok(()) }
                        "ss" => { self.bytes.push(0x17); Ok(()) }
                        "ds" => { self.bytes.push(0x1F); Ok(()) }
                        "fs" => { self.bytes.extend_from_slice(&[0x0F, 0xA1]); Ok(()) }
                        "gs" => { self.bytes.extend_from_slice(&[0x0F, 0xA9]); Ok(()) }
                        _ => Err(format!("cannot pop to {}", reg.name)),
                    }
                } else {
                    let num = reg_num(&reg.name).ok_or("bad register")?;
                    self.bytes.push(0x58 + num);
                    Ok(())
                }
            }
            Operand::Memory(mem) => {
                // pop m32: 0x8F /0
                self.bytes.push(0x8F);
                self.encode_modrm_mem(0, mem)
            }
            _ => Err("unsupported pop operand".to_string()),
        }
    }

    pub(super) fn encode_alu(&mut self, ops: &[Operand], mnemonic: &str, alu_op: u8) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }

        let size = mnemonic_size_suffix(mnemonic).unwrap_or(4);

        match (&ops[0], &ops[1]) {
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(dst)) => {
                let val = *val;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;

                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }

                if size == 1 {
                    self.bytes.push(0x80);
                    self.bytes.push(self.modrm(3, alu_op, dst_num));
                    self.bytes.push(val as u8);
                } else if (-128..=127).contains(&val) {
                    self.bytes.push(0x83);
                    self.bytes.push(self.modrm(3, alu_op, dst_num));
                    self.bytes.push(val as u8);
                } else {
                    if dst_num == 0 {
                        // Short form: op eax, imm32
                        self.bytes.push(if size == 1 { 0x04 } else { 0x05 } + alu_op * 8);
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
            (Operand::Immediate(ImmediateValue::Symbol(sym)), Operand::Register(dst)) |
            (Operand::Immediate(ImmediateValue::SymbolPlusOffset(sym, _)), Operand::Register(dst)) => {
                let addend = match &ops[0] { Operand::Immediate(ImmediateValue::SymbolPlusOffset(_, a)) => *a, _ => 0 };
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                let opcode_len = if dst_num == 0 {
                    self.bytes.push(0x05 + alu_op * 8);
                    1u32
                } else {
                    self.bytes.push(0x81);
                    self.bytes.push(self.modrm(3, alu_op, dst_num));
                    2u32
                };
                // _GLOBAL_OFFSET_TABLE_ requires R_386_GOTPC (PC-relative to GOT).
                // The implicit addend = opcode length so the PC correction works:
                // ebx (= return addr of thunk call) + (GOT + addend - P) = GOT
                if sym == "_GLOBAL_OFFSET_TABLE_" {
                    self.add_relocation(sym, R_386_GOTPC, 0);
                    self.bytes.extend_from_slice(&opcode_len.to_le_bytes());
                } else {
                    self.add_relocation(sym, R_386_32, addend);
                    self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                }
                Ok(())
            }
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;

                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(if size == 1 { 0x00 } else { 0x01 } + alu_op * 8);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(if size == 1 { 0x02 } else { 0x03 } + alu_op * 8);
                self.encode_modrm_mem(dst_num, mem)
            }
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(if size == 1 { 0x00 } else { 0x01 } + alu_op * 8);
                self.encode_modrm_mem(src_num, mem)
            }
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Memory(mem)) => {
                let val = *val;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }

                if size == 1 {
                    self.bytes.push(0x80);
                    self.encode_modrm_mem(alu_op, mem)?;
                    self.bytes.push(val as u8);
                } else if (-128..=127).contains(&val) {
                    self.bytes.push(0x83);
                    self.encode_modrm_mem(alu_op, mem)?;
                    self.bytes.push(val as u8);
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
            (Operand::Immediate(ImmediateValue::SymbolMod(sym, modifier)), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                let reloc_type = self.tls_reloc_type(modifier);
                if dst_num == 0 {
                    self.bytes.push(0x05 + alu_op * 8);
                } else {
                    self.bytes.push(0x81);
                    self.bytes.push(self.modrm(3, alu_op, dst_num));
                }
                self.add_relocation(sym, reloc_type, 0);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::SymbolMod(sym, modifier)), Operand::Memory(mem)) => {
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                let reloc_type = self.tls_reloc_type(modifier);
                self.bytes.push(0x81);
                self.encode_modrm_mem(alu_op, mem)?;
                self.add_relocation(sym, reloc_type, 0);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::Symbol(sym)), Operand::Memory(mem)) |
            (Operand::Immediate(ImmediateValue::SymbolPlusOffset(sym, _)), Operand::Memory(mem)) => {
                let addend = match &ops[0] { Operand::Immediate(ImmediateValue::SymbolPlusOffset(_, a)) => *a, _ => 0 };
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(0x81);
                self.encode_modrm_mem(alu_op, mem)?;
                self.add_relocation(sym, R_386_32, addend);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            // Symbol difference immediate: e.g. addl $_DYNAMIC-1b, (%esp)
            // Uses R_386_PC32 with diff_symbol so the ELF writer resolves A - B
            (Operand::Immediate(ImmediateValue::SymbolDiff(sym_a, sym_b)), Operand::Memory(mem)) => {
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(0x81);
                self.encode_modrm_mem(alu_op, mem)?;
                self.add_relocation_with_diff(sym_a, R_386_PC32, 0, sym_b);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::SymbolDiff(sym_a, sym_b)), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                if dst_num == 0 {
                    self.bytes.push(0x05 + alu_op * 8);
                } else {
                    self.bytes.push(0x81);
                    self.bytes.push(self.modrm(3, alu_op, dst_num));
                }
                self.add_relocation_with_diff(sym_a, R_386_PC32, 0, sym_b);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            // Label as memory reference: addl %reg, symbol
            (Operand::Register(src), Operand::Label(label)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(if size == 1 { 0x00 } else { 0x01 } + alu_op * 8);
                // Encode as disp32 (mod=00, rm=101)
                self.encode_abs_addr_modrm(src_num, label)?;
                Ok(())
            }
            (Operand::Label(label), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(if size == 1 { 0x02 } else { 0x03 } + alu_op * 8);
                self.encode_abs_addr_modrm(dst_num, label)?;
                Ok(())
            }
            // Immediate to label-as-memory: addl $1, global_counter
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Label(label)) => {
                let val = *val;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }

                if size == 1 {
                    self.bytes.push(0x80);
                } else if (-128..=127).contains(&val) {
                    self.bytes.push(0x83);
                } else {
                    self.bytes.push(0x81);
                }
                // mod=00, rm=101 for disp32 (no base)
                self.encode_abs_addr_modrm(alu_op, label)?;
                if size == 1 || (-128..=127).contains(&val) {
                    self.bytes.push(val as u8);
                } else if size == 2 {
                    self.bytes.extend_from_slice(&(val as i16).to_le_bytes());
                } else {
                    self.bytes.extend_from_slice(&(val as i32).to_le_bytes());
                }
                Ok(())
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    pub(super) fn encode_test(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }

        let size = mnemonic_size_suffix(mnemonic).unwrap_or(4);

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(if size == 1 { 0x84 } else { 0x85 });
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(dst)) => {
                let val = *val;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }

                if size == 1 {
                    if dst_num == 0 {
                        self.bytes.push(0xA8);
                    } else {
                        self.bytes.push(0xF6);
                        self.bytes.push(self.modrm(3, 0, dst_num));
                    }
                    self.bytes.push(val as u8);
                } else {
                    if dst_num == 0 {
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
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Memory(mem)) => {
                let val = *val;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                if size == 1 {
                    self.bytes.push(0xF6);
                } else {
                    self.bytes.push(0xF7);
                }
                self.encode_modrm_mem(0, mem)?;
                if size == 1 {
                    self.bytes.push(val as u8);
                } else if size == 2 {
                    self.bytes.extend_from_slice(&(val as i16).to_le_bytes());
                } else {
                    self.bytes.extend_from_slice(&(val as i32).to_le_bytes());
                }
                Ok(())
            }
            (Operand::Immediate(imm), Operand::Label(label)) => {
                // `testb $CAN_USE_HEAP, loadflags` (header.S): a bare symbol
                // as the memory operand — absolute addressing through a
                // relocation, same shape GAS emits (F6 /0 disp16/32 + reloc).
                let mem = MemoryOperand {
                    segment: None,
                    displacement: Displacement::Symbol(label.clone()),
                    base: None, index: None, scale: None,
                    mask: None, zeroing: false,
                };
                self.encode_test(&[Operand::Immediate(imm.clone()), Operand::Memory(mem)], mnemonic)
            }
            _ => Err("unsupported test operands".to_string()),
        }
    }

    pub(super) fn encode_imul(&mut self, ops: &[Operand], size: u8) -> Result<(), String> {
        // imul with 2/3 operands always has an operand width; the 1-operand
        // form routes through encode_unary_rm which handles sized_op itself.
        if ops.len() >= 2 { self.sized_op = true; }
        match ops.len() {
            1 => self.encode_unary_rm(ops, 5, size),
            2 => {
                match (&ops[0], &ops[1]) {
                    (Operand::Register(src), Operand::Register(dst)) => {
                        let src_num = reg_num(&src.name).ok_or("bad register")?;
                        let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                        self.bytes.extend_from_slice(&[0x0F, 0xAF]);
                        self.bytes.push(self.modrm(3, dst_num, src_num));
                        Ok(())
                    }
                    (Operand::Memory(mem), Operand::Register(dst)) => {
                        let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                        self.bytes.extend_from_slice(&[0x0F, 0xAF]);
                        self.encode_modrm_mem(dst_num, mem)
                    }
                    // imul $imm, %reg  =>  imul $imm, %reg, %reg (dst = src * imm)
                    (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(dst)) => {
                        let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                        if *val >= -128 && *val <= 127 {
                            self.bytes.push(0x6B);
                            self.bytes.push(self.modrm(3, dst_num, dst_num));
                            self.bytes.push(*val as u8);
                        } else {
                            self.bytes.push(0x69);
                            self.bytes.push(self.modrm(3, dst_num, dst_num));
                            self.bytes.extend_from_slice(&(*val as i32).to_le_bytes());
                        }
                        Ok(())
                    }
                    _ => Err("unsupported imul operands".to_string()),
                }
            }
            3 => {
                match (&ops[0], &ops[1], &ops[2]) {
                    (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(src), Operand::Register(dst)) => {
                        let src_num = reg_num(&src.name).ok_or("bad register")?;
                        let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                        if *val >= -128 && *val <= 127 {
                            self.bytes.push(0x6B);
                            self.bytes.push(self.modrm(3, dst_num, src_num));
                            self.bytes.push(*val as u8);
                        } else {
                            self.bytes.push(0x69);
                            self.bytes.push(self.modrm(3, dst_num, src_num));
                            self.bytes.extend_from_slice(&(*val as i32).to_le_bytes());
                        }
                        Ok(())
                    }
                    (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Memory(mem), Operand::Register(dst)) => {
                        let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                        if *val >= -128 && *val <= 127 {
                            self.bytes.push(0x6B);
                            self.encode_modrm_mem(dst_num, mem)?;
                            self.bytes.push(*val as u8);
                        } else {
                            self.bytes.push(0x69);
                            self.encode_modrm_mem(dst_num, mem)?;
                            self.bytes.extend_from_slice(&(*val as i32).to_le_bytes());
                        }
                        Ok(())
                    }
                    _ => Err("unsupported imul operands".to_string()),
                }
            }
            _ => Err("imul requires 1-3 operands".to_string()),
        }
    }

    pub(super) fn encode_unary_rm(&mut self, ops: &[Operand], op_ext: u8, size: u8) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("unary op requires 1 operand".to_string());
        }
        if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                self.bytes.push(if size == 1 { 0xF6 } else { 0xF7 });
                self.bytes.push(self.modrm(3, op_ext, num));
                Ok(())
            }
            Operand::Memory(mem) => {
                self.bytes.push(if size == 1 { 0xF6 } else { 0xF7 });
                self.encode_modrm_mem(op_ext, mem)
            }
            _ => Err("unsupported unary operand".to_string()),
        }
    }

    /// Encode inc/dec instructions.
    /// In 32-bit mode, inc/dec have compact single-byte encodings for 32-bit registers:
    ///   inc: 0x40+reg, dec: 0x48+reg
    /// For memory operands or byte/word sizes, use opcode 0xFE (byte) / 0xFF (word/dword)
    /// with modrm extension /0 (inc) or /1 (dec).
    pub(super) fn encode_inc_dec(&mut self, ops: &[Operand], op_ext: u8, size: u8) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("inc/dec requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                if size == 4 {
                    // Compact single-byte encoding: 0x40+reg (inc) or 0x48+reg
                    // (dec). This IS width-dependent: in .code16 the bare
                    // opcode means the 16-bit register, so sized_op must be
                    // set for the inversion to add 0x66 (incl %eax in real
                    // mode = 66 40; we emitted 40 = incw %ax).
                    self.sized_op = true;
                    let base = if op_ext == 0 { 0x40 } else { 0x48 };
                    self.bytes.push(base + num);
                } else if size == 2 {
                    // 16-bit: operand size prefix + 0x40+reg (inc) or 0x48+reg (dec)
                    self.sized_op = true; self.bytes.push(0x66);
                    let base = if op_ext == 0 { 0x40 } else { 0x48 };
                    self.bytes.push(base + num);
                } else {
                    // 8-bit: use 0xFE /0 (inc) or 0xFE /1 (dec) with modrm
                    self.bytes.push(0xFE);
                    self.bytes.push(self.modrm(3, op_ext, num));
                }
                Ok(())
            }
            Operand::Memory(mem) => {
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(if size == 1 { 0xFE } else { 0xFF });
                self.encode_modrm_mem(op_ext, mem)
            }
            // Label as memory reference: incl symbol or incl symbol+4
            Operand::Label(label) => {
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(if size == 1 { 0xFE } else { 0xFF });
                // Encode as disp32 (mod=00, rm=101)
                self.encode_abs_addr_modrm(op_ext, label)?;
                Ok(())
            }
            _ => Err("unsupported inc/dec operand".to_string()),
        }
    }

    pub(super) fn encode_shift(&mut self, ops: &[Operand], mnemonic: &str, shift_op: u8) -> Result<(), String> {
        let size = mnemonic_size_suffix(mnemonic).unwrap_or(4);

        // Handle 1-operand form: shrl %eax means shift right by 1
        if ops.len() == 1 {
            match &ops[0] {
                Operand::Register(dst) => {
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                    self.bytes.push(if size == 1 { 0xD0 } else { 0xD1 });
                    self.bytes.push(self.modrm(3, shift_op, dst_num));
                    return Ok(());
                }
                Operand::Memory(mem) => {
                    if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                    self.bytes.push(if size == 1 { 0xD0 } else { 0xD1 });
                    return self.encode_modrm_mem(shift_op, mem);
                }
                _ => return Err(format!("unsupported {} operand", mnemonic)),
            }
        }

        if ops.len() != 2 {
            return Err(format!("{} requires 1 or 2 operands", mnemonic));
        }

        match (&ops[0], &ops[1]) {
            (Operand::Immediate(ImmediateValue::Integer(count)), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let count = *count as u8;

                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }

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
            (Operand::Register(cl), Operand::Register(dst)) if cl.name == "cl" => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(if size == 1 { 0xD2 } else { 0xD3 });
                self.bytes.push(self.modrm(3, shift_op, dst_num));
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::Integer(count)), Operand::Memory(mem)) => {
                let count = *count as u8;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                if count == 1 {
                    self.bytes.push(if size == 1 { 0xD0 } else { 0xD1 });
                    self.encode_modrm_mem(shift_op, mem)?;
                } else {
                    self.bytes.push(if size == 1 { 0xC0 } else { 0xC1 });
                    self.encode_modrm_mem(shift_op, mem)?;
                    self.bytes.push(count);
                }
                Ok(())
            }
            (Operand::Register(cl), Operand::Memory(mem)) if cl.name == "cl" => {
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(if size == 1 { 0xD2 } else { 0xD3 });
                self.encode_modrm_mem(shift_op, mem)
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    pub(super) fn encode_double_shift(&mut self, ops: &[Operand], opcode: u8, _size: u8) -> Result<(), String> {
        // Operand-width instruction: mark sized_op so the .code16 prefix
        // inversion emits/strips 0x66 correctly (GAS oracle c16all.s).
        self.sized_op = true;
        if ops.len() != 3 {
            return Err("double shift requires 3 operands".to_string());
        }

        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Immediate(ImmediateValue::Integer(count)), Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, opcode]);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                self.bytes.push(*count as u8);
                Ok(())
            }
            (Operand::Register(cl), Operand::Register(src), Operand::Register(dst)) if cl.name == "cl" => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, opcode + 1]);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            _ => Err("unsupported double shift operands".to_string()),
        }
    }

    pub(super) fn encode_bswap(&mut self, ops: &[Operand]) -> Result<(), String> {
        // Operand-width instruction: mark sized_op so the .code16 prefix
        // inversion emits/strips 0x66 correctly (GAS oracle c16all.s).
        self.sized_op = true;
        if ops.len() != 1 {
            return Err("bswap requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, 0xC8 + num]);
                Ok(())
            }
            _ => Err("bswap requires register operand".to_string()),
        }
    }

    pub(super) fn encode_bit_count(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        // Operand-width instruction: mark sized_op so the .code16 prefix
        // inversion emits/strips 0x66 correctly (GAS oracle c16all.s).
        self.sized_op = true;
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }

        let (prefix, opcode) = match mnemonic {
            "lzcntl" => (0xF3u8, [0x0F, 0xBD]),
            "tzcntl" => (0xF3, [0x0F, 0xBC]),
            "popcntl" => (0xF3, [0x0F, 0xB8]),
            _ => return Err(format!("unknown bit count: {}", mnemonic)),
        };

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.bytes.push(prefix);
                self.bytes.extend_from_slice(&opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    pub(super) fn encode_bsr_bsf(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        // Operand-width instruction: mark sized_op so the .code16 prefix
        // inversion emits/strips 0x66 correctly (GAS oracle c16all.s).
        self.sized_op = true;
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }

        let opcode = match mnemonic {
            "bsrl" | "bsr" => [0x0F, 0xBD],
            "bsfl" | "bsf" => [0x0F, 0xBC],
            _ => return Err(format!("unknown bit scan: {}", mnemonic)),
        };

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    pub(super) fn encode_bt(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        // Operand-width instruction: mark sized_op so the .code16 prefix
        // inversion emits/strips 0x66 correctly (GAS oracle c16all.s).
        self.sized_op = true;
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }

        let (opcode_rr, ext) = match mnemonic {
            "btl" | "bt" => (0xA3u8, 4u8),
            "btsl" | "bts" => (0xAB, 5),
            "btrl" | "btr" => (0xB3, 6),
            "btcl" | "btc" => (0xBB, 7),
            _ => return Err(format!("unknown bt instruction: {}", mnemonic)),
        };

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, opcode_rr]);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, opcode_rr]);
                self.encode_modrm_mem(src_num, mem)
            }
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, 0xBA]);
                self.bytes.push(self.modrm(3, ext, dst_num));
                self.bytes.push(*val as u8);
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Memory(mem)) => {
                self.bytes.extend_from_slice(&[0x0F, 0xBA]);
                self.encode_modrm_mem(ext, mem)?;
                self.bytes.push(*val as u8);
                Ok(())
            }
            // bt $imm, label (treat label as absolute memory reference)
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Label(label)) => {
                self.bytes.extend_from_slice(&[0x0F, 0xBA]);
                // mod=00, rm=101 for disp32 (no base register)
                self.encode_abs_addr_modrm(ext, label)?;
                self.bytes.push(*val as u8);
                Ok(())
            }
            // bt %reg, label (treat label as absolute memory reference)
            (Operand::Register(src), Operand::Label(label)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, opcode_rr]);
                // mod=00, rm=101 for disp32 (no base register)
                self.encode_abs_addr_modrm(src_num, label)?;
                Ok(())
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    pub(super) fn encode_setcc(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("setcc requires 1 operand".to_string());
        }

        let cc = cc_from_mnemonic(&mnemonic[3..])?;

        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, 0x90 + cc]);
                self.bytes.push(self.modrm(3, 0, num));
                Ok(())
            }
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0x90 + cc]);
                self.encode_modrm_mem(0, mem)
            }
            _ => Err("setcc requires register or memory operand".to_string()),
        }
    }

    pub(super) fn encode_cmovcc(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("cmovcc requires 2 operands".to_string());
        }

        let without_prefix = &mnemonic[4..];
        // Strip size suffix if present, otherwise use as-is (unsuffixed = 32-bit default)
        let (cc_str, is_16bit) = if without_prefix.ends_with('w')
            && without_prefix != "w"
            && cc_from_mnemonic(&without_prefix[..without_prefix.len()-1]).is_ok()
        {
            (&without_prefix[..without_prefix.len()-1], true)
        } else if without_prefix.ends_with('l')
            && without_prefix != "l"
            && cc_from_mnemonic(&without_prefix[..without_prefix.len()-1]).is_ok()
        {
            (&without_prefix[..without_prefix.len()-1], false)
        } else {
            (without_prefix, false)
        };
        let cc = cc_from_mnemonic(cc_str)?;

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                // cmov ALWAYS selects an operand width (w or l): mark sized_op
                // unconditionally so the .code16 prefix inversion works both
                // ways. Setting it only for the 16-bit form left `cmovel` in
                // real mode WITHOUT the 66 prefix -- decoded as a 16-bit
                // cmov, silently writing half the register (GAS emits
                // 66 0f 44 c1; we emitted 0f 44 c1).
                self.sized_op = true;
                if is_16bit { self.bytes.push(0x66); }
                self.bytes.extend_from_slice(&[0x0F, 0x40 + cc]);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.sized_op = true;
                if is_16bit { self.bytes.push(0x66); }
                self.bytes.extend_from_slice(&[0x0F, 0x40 + cc]);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported cmov operands".to_string()),
        }
    }

    pub(super) fn encode_jmp(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("jmp requires 1 operand".to_string());
        }

        match &ops[0] {
            Operand::Label(label) => {
                let sym = label.strip_suffix("@PLT").unwrap_or(label.as_str());
                if self.code16 {
                    // .code16 near jmp: E9 rel16 + R_386_PC16 (GAS emits
                    // `e9 00 00` + PC16 for forward targets; short local
                    // targets get relaxed to EB rel8 by GAS — lccc's fixed
                    // rel16 form is semantically identical and local
                    // targets fold to constants in the writer).
                    self.bytes.push(0xE9);
                    self.add_relocation(sym, R_386_PC16, -2);
                    self.bytes.extend_from_slice(&[0, 0]);
                    return Ok(());
                }
                self.bytes.push(0xE9);
                // Always use R_386_PLT32 for branch targets, matching modern GCC/binutils.
                // R_386_PC32 is rejected by ld for PIE executables calling shared lib functions.
                let reloc_type = R_386_PLT32;
                self.add_relocation(sym, reloc_type, -4);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            Operand::Indirect(inner) => {
                match inner.as_ref() {
                    Operand::Register(reg) => {
                        let num = reg_num(&reg.name).ok_or("bad register")?;
                        self.bytes.push(0xFF);
                        self.bytes.push(self.modrm(3, 4, num));
                        Ok(())
                    }
                    Operand::Memory(mem) => {
                        self.emit_segment_prefix(mem);
                        self.bytes.push(0xFF);
                        self.encode_modrm_mem(4, mem)
                    }
                    // `jmp *sym`: absolute-address indirect jump (FF /4,
                    // disp-only ModR/M). GAS: `ff 25 <R_386_32 sym>` in
                    // .code32; `ff 26` + R_386_16 in .code16 (gcc -m16
                    // itself emits this 16-bit form for `jmp *fp` — the low
                    // half of the 32-bit pointer is the whole story in real
                    // mode). Produced by tail-calling the global-fnptr fold.
                    Operand::Label(label) => {
                        self.bytes.push(0xFF);
                        self.encode_abs_addr_modrm(4, label)
                    }
                    _ => Err("unsupported indirect jmp target".to_string()),
                }
            }
            _ => Err("unsupported jmp operand".to_string()),
        }
    }

    /// Encode far jump (ljmpl/ljmp): direct or indirect
    /// Encode LCALL (far call): 9A cp (direct) or FF /3 (indirect memory).
    /// Mirror of `encode_ljmp` with ModRM extension /3 and direct opcode 0x9A.
    pub(super) fn encode_lcall(&mut self, ops: &[Operand]) -> Result<(), String> {
        match ops.len() {
            1 => {
                match &ops[0] {
                    Operand::Indirect(inner) => match inner.as_ref() {
                        Operand::Memory(mem) => {
                            self.emit_segment_prefix(mem);
                            self.bytes.push(0xFF);
                            self.encode_modrm_mem(3, mem)
                        }
                        Operand::Label(label) => {
                            self.bytes.push(0xFF);
                            self.bytes.push(self.modrm(0, 3, 5));
                            self.add_relocation_for_label(label, R_386_32);
                            self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                            Ok(())
                        }
                        _ => Err("lcall indirect requires memory or label operand".to_string()),
                    },
                    Operand::Memory(mem) => {
                        self.emit_segment_prefix(mem);
                        self.bytes.push(0xFF);
                        self.encode_modrm_mem(3, mem)
                    }
                    _ => Err("lcall requires indirect memory or segment:offset operands".to_string()),
                }
            }
            2 => match (&ops[0], &ops[1]) {
                (Operand::Immediate(ImmediateValue::Integer(seg)), Operand::Immediate(ImmediateValue::Integer(off))) => {
                    self.bytes.push(0x9A);
                    self.bytes.extend_from_slice(&(*off as u32).to_le_bytes());
                    self.bytes.extend_from_slice(&(*seg as u16).to_le_bytes());
                    Ok(())
                }
                (Operand::Immediate(ImmediateValue::Integer(seg)), Operand::Immediate(ImmediateValue::Symbol(sym))) => {
                    self.bytes.push(0x9A);
                    self.add_relocation(sym, R_386_32, 0);
                    self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                    self.bytes.extend_from_slice(&(*seg as u16).to_le_bytes());
                    Ok(())
                }
                _ => Err("lcall requires $segment, $offset operands".to_string()),
            },
            _ => Err("lcall requires 1 or 2 operands".to_string()),
        }
    }

    pub(super) fn encode_ljmp(&mut self, ops: &[Operand]) -> Result<(), String> {
        match ops.len() {
            // ljmpl *mem - indirect far jump through memory (FF /5)
            1 => {
                match &ops[0] {
                    Operand::Indirect(inner) => {
                        match inner.as_ref() {
                            Operand::Memory(mem) => {
                                self.emit_segment_prefix(mem);
                                self.bytes.push(0xFF);
                                self.encode_modrm_mem(5, mem)
                            }
                            Operand::Label(label) => {
                                // ljmpl *symbol - indirect far jump via label
                                self.bytes.push(0xFF);
                                self.bytes.push(self.modrm(0, 5, 5));
                                self.add_relocation_for_label(label, R_386_32);
                                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                                Ok(())
                            }
                            _ => Err("ljmp indirect requires memory or label operand".to_string()),
                        }
                    }
                    Operand::Memory(mem) => {
                        // ljmp *mem (without explicit indirect prefix)
                        self.emit_segment_prefix(mem);
                        self.bytes.push(0xFF);
                        self.encode_modrm_mem(5, mem)
                    }
                    _ => Err("ljmp requires indirect memory or segment:offset operands".to_string()),
                }
            }
            // ljmpl $segment, $offset - direct far jump (opcode 0xEA)
            2 => {
                match (&ops[0], &ops[1]) {
                    (Operand::Immediate(ImmediateValue::Integer(seg)), Operand::Immediate(ImmediateValue::Integer(off))) => {
                        // .code16: offset width follows the suffix — ljmpl
                        // marks sized_op (66-prefixed via the inversion) and
                        // keeps imm32; ljmp/ljmpw emit imm16. .code32: imm32.
                        if self.code16 && !self.ljmp_wide {
                            self.bytes.push(0xEA);
                            self.bytes.extend_from_slice(&(*off as u16).to_le_bytes());
                        } else {
                            if self.code16 { self.sized_op = true; }
                            self.bytes.push(0xEA);
                            self.bytes.extend_from_slice(&(*off as u32).to_le_bytes());
                        }
                        self.bytes.extend_from_slice(&(*seg as u16).to_le_bytes());
                        Ok(())
                    }
                    (Operand::Immediate(ImmediateValue::Integer(seg)), Operand::Immediate(ImmediateValue::Symbol(sym))) => {
                        if self.code16 && !self.ljmp_wide {
                            self.bytes.push(0xEA);
                            self.add_relocation(sym, R_386_16, 0);
                            self.bytes.extend_from_slice(&[0, 0]);
                        } else {
                            if self.code16 { self.sized_op = true; }
                            self.bytes.push(0xEA);
                            self.add_relocation(sym, R_386_32, 0);
                            self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                        }
                        self.bytes.extend_from_slice(&(*seg as u16).to_le_bytes());
                        Ok(())
                    }
                    (Operand::Immediate(ImmediateValue::Integer(seg)), Operand::Immediate(ImmediateValue::SymbolDiff(sym, diff))) => {
                        // la57toggle.S (compiled as .code32 inside a 64-bit
                        // kernel object): `ljmpl $__KERNEL_CS, $(.Lret -
                        // trampoline_32bit_src)`. The offset is a
                        // same-section label difference; GAS folds it to a
                        // constant after layout (`ea 08000000 1000`). Record
                        // a diff relocation on the imm32 field.
                        self.bytes.push(0xEA);
                        self.add_relocation_with_diff(sym, R_386_32, 0, diff);
                        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                        self.bytes.extend_from_slice(&(*seg as u16).to_le_bytes());
                        Ok(())
                    }
                    _ => Err("ljmp requires $segment, $offset operands".to_string()),
                }
            }
            _ => Err("ljmp requires 1 or 2 operands".to_string()),
        }
    }

    pub(super) fn encode_jcc(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("jcc requires 1 operand".to_string());
        }

        let cc = cc_from_mnemonic(&mnemonic[1..])?;

        match &ops[0] {
            Operand::Label(label) => {
                let sym = label.strip_suffix("@PLT").unwrap_or(label.as_str());
                if self.code16 {
                    // .code16 Jcc: 0F 8x rel16 + R_386_PC16. GAS relaxes
                    // same-section short targets to 7x rel8; lccc emits the
                    // fixed rel16 form (identical semantics, always in
                    // range for real-mode blobs; local targets resolve to
                    // constants via the writer's PC16 path).
                    self.bytes.extend_from_slice(&[0x0F, 0x80 + cc]);
                    self.add_relocation(sym, R_386_PC16, -2);
                    self.bytes.extend_from_slice(&[0, 0]);
                    return Ok(());
                }
                self.bytes.extend_from_slice(&[0x0F, 0x80 + cc]);
                // Always use R_386_PLT32 for branch targets, matching modern GCC/binutils.
                let reloc_type = R_386_PLT32;
                self.add_relocation(sym, reloc_type, -4);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            _ => Err("jcc requires label operand".to_string()),
        }
    }

    pub(super) fn encode_call(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("call requires 1 operand".to_string());
        }

        match &ops[0] {
            Operand::Label(label) => {
                let sym = label.strip_suffix("@PLT").unwrap_or(label.as_str());
                if self.code16 {
                    // .code16 near call: E8 rel16 + R_386_PC16 (GAS:
                    // `e8 00 00` + PC16 ext_fn-0x2). The explicit 32-bit
                    // spelling `calll` is 66-prefixed rel32 (PC32) — GAS
                    // `66 e8 00 00 00 00` + PC32 ext_fn-0x4 — selected by
                    // the sized_op the dispatch sets for "calll".
                    if self.sized_op {
                        self.bytes.push(0xE8);
                        self.add_relocation(sym, R_386_PC32, -4);
                        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                    } else {
                        self.bytes.push(0xE8);
                        self.add_relocation(sym, R_386_PC16, -2);
                        self.bytes.extend_from_slice(&[0, 0]);
                    }
                    return Ok(());
                }
                self.bytes.push(0xE8);
                // Always use R_386_PLT32 for branch targets, matching modern GCC/binutils.
                let reloc_type = R_386_PLT32;
                self.add_relocation(sym, reloc_type, -4);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            Operand::Indirect(inner) => {
                match inner.as_ref() {
                    Operand::Register(reg) => {
                        let num = reg_num(&reg.name).ok_or("bad register")?;
                        self.bytes.push(0xFF);
                        self.bytes.push(self.modrm(3, 2, num));
                        Ok(())
                    }
                    Operand::Memory(mem) => {
                        self.emit_segment_prefix(mem);
                        self.bytes.push(0xFF);
                        self.encode_modrm_mem(2, mem)
                    }
                    // `call *sym` / `call *sym+4`: absolute-address indirect
                    // call through a global function pointer (FF /2 with
                    // disp-only ModR/M). GAS encodes `call *pio_ops` as
                    // `ff 15 <R_386_32 pio_ops>` (.code32) / `ff 16` +
                    // R_386_16 (.code16). The peephole's global-fnptr fold
                    // emits exactly this shape.
                    Operand::Label(label) => {
                        self.bytes.push(0xFF);
                        self.encode_abs_addr_modrm(2, label)
                    }
                    _ => Err("unsupported indirect call target".to_string()),
                }
            }
            _ => Err("unsupported call operand".to_string()),
        }
    }

    pub(super) fn encode_xchg(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("xchg requires 2 operands".to_string());
        }
        let size = mnemonic_size_suffix(mnemonic).unwrap_or(4);

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(if size == 1 { 0x86 } else { 0x87 });
                self.encode_modrm_mem(src_num, mem)
            }
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.push(if size == 1 { 0x86 } else { 0x87 });
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            _ => Err("unsupported xchg operands".to_string()),
        }
    }

    pub(super) fn encode_cmpxchg(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("cmpxchg requires 2 operands".to_string());
        }
        let size = mnemonic_size_suffix(mnemonic).unwrap_or(4);

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.extend_from_slice(&[0x0F, if size == 1 { 0xB0 } else { 0xB1 }]);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("unsupported cmpxchg operands".to_string()),
        }
    }

    pub(super) fn encode_xadd(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("xadd requires 2 operands".to_string());
        }
        let size = mnemonic_size_suffix(mnemonic).unwrap_or(4);

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                if size == 2 || size == 4 { self.sized_op = true; }
        if size == 2 { self.bytes.push(0x66); }
                self.bytes.extend_from_slice(&[0x0F, if size == 1 { 0xC0 } else { 0xC1 }]);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("unsupported xadd operands".to_string()),
        }
    }

    pub(super) fn encode_clflush(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("clflush requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0xAE]);
                self.encode_modrm_mem(7, mem)
            }
            _ => Err("clflush requires memory operand".to_string()),
        }
    }

    /// Encode SSE memory-only instructions (ldmxcsr, stmxcsr).
    /// Format: 0F AE /ext mem
    pub(super) fn encode_sse_mem_only(&mut self, ops: &[Operand], ext: u8) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("SSE mem-only op requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0xAE]);
                self.encode_modrm_mem(ext, mem)
            }
            _ => Err("SSE mem-only op requires memory operand".to_string()),
        }
    }

    pub(super) fn encode_int(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("int requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Immediate(ImmediateValue::Integer(val)) => {
                if *val == 3 {
                    self.bytes.push(0xCC);
                } else {
                    self.bytes.push(0xCD);
                    self.bytes.push(*val as u8);
                }
                Ok(())
            }
            _ => Err("int requires immediate operand".to_string()),
        }
    }
}
