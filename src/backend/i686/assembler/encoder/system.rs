//! System and privileged instruction encoders for i686.
//!
//! Handles prefetch, port I/O, INVLPG, VERW, LSL, descriptor table
//! operations, control register moves, segment register moves,
//! and other system-level instructions.

use super::*;

impl super::InstructionEncoder {
    /// The size of an operation that neither a suffix nor a register says
    /// anything about: the code segment's default, 16 bits in `.code16` and
    /// `.code16gcc` and 32 bits otherwise. GNU as assumes it, with a
    /// warning. (It used to be 32 bits everywhere: `add $1, (%bx)` in
    /// real-mode code came out as `66 83 07 01`, an `addl`, and `mov $1,
    /// (%bx)` with a four-byte immediate.)
    pub(super) fn default_operand_size(&self) -> u8 {
        if self.code16 { 2 } else { 4 }
    }

    /// The operand size of `mnemonic ops`: its suffix, else its registers,
    /// else [`Self::default_operand_size`].
    pub(super) fn operand_size(&self, mnemonic: &str, ops: &[Operand]) -> Result<u8, String> {
        infer_op_size(
            mnemonic,
            ops,
            mnemonic_size_suffix(mnemonic),
            self.default_operand_size(),
        )
    }

    /// Encode prefetch instructions (0F 18 /hint)
    pub(super) fn encode_prefetch(&mut self, ops: &[Operand], hint: u8) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("prefetch requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0x18]);
                self.encode_modrm_mem(hint, mem)
            }
            _ => Err("prefetch requires memory operand".to_string()),
        }
    }

    /// Encode prefetchw (0F 0D /1)
    pub(super) fn encode_prefetch_0f0d(&mut self, ops: &[Operand], hint: u8) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("prefetchw requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0x0D]);
                self.encode_modrm_mem(hint, mem)
            }
            _ => Err("prefetchw requires memory operand".to_string()),
        }
    }

    /// Encode INVLPG: 0F 01 /7 (memory operand)
    pub(super) fn encode_invlpg(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("invlpg requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.encode_modrm_mem(7, mem)
            }
            _ => Err("invlpg requires memory operand".to_string()),
        }
    }

    /// Encode SGDT/SIDT/LGDT/LIDT: 0F 01 /N (memory operand)
    /// 0F 00 /r group (sldt/lldt/ltr/str): register form only, r/m16.
    /// GAS encodes `lldt %ax` as `0f 00 d0` — no operand-size prefix; the
    /// operand is architecturally 16-bit regardless of spelling.
    /// RDRAND/RDSEED register form: 0F C7 /6|/7 with mod=11.  The ISA
    /// defines no memory form, and only 16- and 32-bit GPR destinations
    /// exist — byte registers and every non-GPR class are rejected
    /// explicitly instead of silently encoding a wrong reg field (the
    /// shared reg_num table also answers for x87/MMX/XMM names).
    ///
    /// Operand-size handling joins the central `.code16` inversion
    /// (`sized_op`): the encoder is written for 32-bit semantics — 0x66
    /// on a 16-bit destination, none on 32-bit — and
    /// `fixup_code16_prefixes` inverts both directions in real mode
    /// (%eax gains the prefix, %ax loses it).  Hand-rolling mode logic
    /// here would be one more site to get the matrix wrong.
    pub(super) fn encode_rdrand_rdseed(
        &mut self,
        ops: &[Operand],
        reg_ext: u8,
    ) -> Result<(), String> {
        const GPR16: [&str; 8] = ["ax", "cx", "dx", "bx", "sp", "bp", "si", "di"];
        const GPR32: [&str; 8] = ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi"];
        if ops.len() != 1 {
            return Err("rdrand/rdseed requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let name = reg.name.as_str();
                let is16 = GPR16.contains(&name);
                let is32 = GPR32.contains(&name);
                if !is16 && !is32 {
                    return Err(format!(
                        "rdrand/rdseed requires a 16- or 32-bit general register, got '%{}'",
                        name
                    ));
                }
                let rm = reg_num(name)
                    .ok_or_else(|| format!("bad register for rdrand/rdseed: {}", name))?;
                if is16 {
                    self.bytes.push(0x66);
                }
                self.sized_op = true;
                self.bytes.extend_from_slice(&[0x0F, 0xC7]);
                self.bytes.push(self.modrm(3, reg_ext, rm));
                Ok(())
            }
            _ => Err("rdrand/rdseed requires a register operand".to_string()),
        }
    }

    pub(super) fn encode_system_table(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err(format!("{} requires 1 operand", mnemonic));
        }
        // Strip optional 'l' suffix (e.g., "lgdtl" -> "lgdt")
        let base = mnemonic.strip_suffix('l').unwrap_or(mnemonic);
        let reg_ext = match base {
            "sgdt" => 0,
            "sidt" => 1,
            "lgdt" => 2,
            "lidt" => 3,
            _ => return Err(format!("unknown system table instruction: {}", mnemonic)),
        };
        // `lgdtl`/`lidtl` load the 32-bit descriptor form; `lgdtw`/`lgdt`
        // the 16-bit one. In .code16 the 32-bit form needs 0x66, which the
        // prefix inversion supplies once the size choice is recorded.
        if mnemonic.ends_with('l') {
            self.sized_op = true;
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.encode_modrm_mem(reg_ext, mem)
            }
            // Label as absolute memory reference: lgdtl tr_gdt
            Operand::Label(label) => {
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                // mod=00, rm=101 for disp32 (no base register)
                self.encode_abs_addr_modrm(reg_ext, label)?;
                Ok(())
            }
            _ => Err(format!("{} requires memory operand", mnemonic)),
        }
    }

    /// Encode MOV to/from control register: 0F 20 /r (read) or 0F 22 /r (write)
    pub(super) fn encode_mov_cr(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("mov cr requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(cr), Operand::Register(gp)) if is_control_reg(&cr.name) => {
                let cr_num = control_reg_num(&cr.name).ok_or("bad control register")?;
                let gp_num = reg_num(&gp.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, 0x20]);
                self.bytes.push(self.modrm(3, cr_num, gp_num));
                Ok(())
            }
            (Operand::Register(gp), Operand::Register(cr)) if is_control_reg(&cr.name) => {
                let cr_num = control_reg_num(&cr.name).ok_or("bad control register")?;
                let gp_num = reg_num(&gp.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, 0x22]);
                self.bytes.push(self.modrm(3, cr_num, gp_num));
                Ok(())
            }
            _ => Err("unsupported mov cr operands".to_string()),
        }
    }

    /// Load an offset and segment selector from a far pointer in memory.
    pub(super) fn encode_load_far_pointer(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        let [source, Operand::Register(dst)] = ops else {
            return Err("far pointer load requires memory and a register".to_string());
        };
        let width = match dst.name.as_str() {
            "ax" | "cx" | "dx" | "bx" | "sp" | "bp" | "si" | "di" => 2,
            "eax" | "ecx" | "edx" | "ebx" | "esp" | "ebp" | "esi" | "edi" => 4,
            _ => return Err("invalid far pointer destination".to_string()),
        };
        if (mnemonic.ends_with('w') && width != 2) || (mnemonic.ends_with('l') && width != 4) {
            return Err("far pointer destination width mismatch".to_string());
        }
        let mut normalized;
        let source = if matches!(source, Operand::Label(_)) {
            normalized = source.clone();
            normalize_bare_label(&mut normalized);
            &normalized
        } else {
            source
        };
        let Operand::Memory(mem) = source else {
            return Err("far pointer load requires memory".to_string());
        };
        let dst_num = reg_num(&dst.name).ok_or("invalid far pointer destination")?;
        self.sized_op = true;
        if width == 2 {
            self.bytes.push(0x66);
        }
        match &mnemonic[..3] {
            "les" => self.bytes.push(0xC4),
            "lds" => self.bytes.push(0xC5),
            "lss" => self.bytes.extend_from_slice(&[0x0F, 0xB2]),
            "lfs" => self.bytes.extend_from_slice(&[0x0F, 0xB4]),
            "lgs" => self.bytes.extend_from_slice(&[0x0F, 0xB5]),
            _ => return Err("unknown far pointer load".to_string()),
        }
        self.encode_modrm_mem(dst_num, mem)
    }

    /// Encode MOV to/from segment register
    pub(super) fn encode_mov_seg(&mut self, ops: &[Operand], size: u8) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("mov seg requires 2 operands".to_string());
        }

        let seg_num = |name: &str| -> Option<u8> {
            match name {
                "es" => Some(0),
                "cs" => Some(1),
                "ss" => Some(2),
                "ds" => Some(3),
                "fs" => Some(4),
                "gs" => Some(5),
                _ => None,
            }
        };

        let gp_num = |name: &str| -> Result<u8, String> {
            if !matches!(
                name,
                "ax" | "cx"
                    | "dx"
                    | "bx"
                    | "sp"
                    | "bp"
                    | "si"
                    | "di"
                    | "eax"
                    | "ecx"
                    | "edx"
                    | "ebx"
                    | "esp"
                    | "ebp"
                    | "esi"
                    | "edi"
            ) || reg_size(name) != size
            {
                return Err("invalid general register for segment move".to_string());
            }
            reg_num(name).ok_or_else(|| "invalid general register for segment move".to_string())
        };

        match (&ops[0], &ops[1]) {
            // mov %sreg, %reg32
            (Operand::Register(src), Operand::Register(dst)) if is_segment_reg(&src.name) => {
                let sr = seg_num(&src.name).ok_or("bad segment register")?;
                let gp = gp_num(&dst.name)?;
                self.sized_op = true;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.bytes.push(0x8C);
                self.bytes.push(self.modrm(3, sr, gp));
                Ok(())
            }
            // mov %reg32, %sreg
            (Operand::Register(src), Operand::Register(dst)) if is_segment_reg(&dst.name) => {
                let gp = gp_num(&src.name)?;
                let sr = seg_num(&dst.name).ok_or("bad segment register")?;
                self.bytes.push(0x8E);
                self.bytes.push(self.modrm(3, sr, gp));
                Ok(())
            }
            // mov %sreg, mem
            (Operand::Register(src), Operand::Memory(mem)) if is_segment_reg(&src.name) => {
                if size != 2 {
                    return Err("segment memory move requires word size".to_string());
                }
                let sr = seg_num(&src.name).ok_or("bad segment register")?;
                self.bytes.push(0x8C);
                self.encode_modrm_mem(sr, mem)
            }
            // mov mem, %sreg
            (Operand::Memory(mem), Operand::Register(dst)) if is_segment_reg(&dst.name) => {
                if size != 2 {
                    return Err("segment memory move requires word size".to_string());
                }
                let sr = seg_num(&dst.name).ok_or("bad segment register")?;
                self.bytes.push(0x8E);
                self.encode_modrm_mem(sr, mem)
            }
            _ => Err("unsupported mov seg operands".to_string()),
        }
    }

    /// Encode popw (16-bit pop)
    pub(super) fn encode_pop16(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("popw requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                if is_segment_reg(&reg.name) {
                    // A 16-bit pop like any other: `popw %es` is `66 07` in
                    // 32-bit code (four bytes come off the stack without
                    // the prefix), and `07` in 16-bit code, where `encode`
                    // takes the prefix off again.
                    let opcode: &[u8] = match reg.name.as_str() {
                        "es" => &[0x07],
                        "ss" => &[0x17],
                        "ds" => &[0x1F],
                        "fs" => &[0x0F, 0xA1],
                        "gs" => &[0x0F, 0xA9],
                        _ => return Err(format!("cannot pop to {}", reg.name)),
                    };
                    self.sized_op = true;
                    self.bytes.push(0x66);
                    self.bytes.extend_from_slice(opcode);
                    Ok(())
                } else {
                    let num = reg_num(&reg.name).ok_or("bad register")?;
                    self.sized_op = true;
                    self.bytes.push(0x66);
                    self.bytes.push(0x58 + num);
                    Ok(())
                }
            }
            Operand::Memory(mem) => {
                // popw m16 = 66 8F /0 (memory destination).
                self.sized_op = true;
                self.bytes.push(0x66);
                self.bytes.push(0x8F);
                self.encode_modrm_mem(0, mem)
            }
            _ => Err("unsupported popw operand".to_string()),
        }
    }

    /// Encode 16-bit BSF/BSR: bsfw/bsrw
    pub(super) fn encode_bsr_bsf_16(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }
        let opcode = match mnemonic {
            "bsrw" => [0x0F, 0xBD],
            "bsfw" => [0x0F, 0xBC],
            _ => return Err(format!("unknown bit scan: {}", mnemonic)),
        };
        self.sized_op = true;
        self.bytes.push(0x66); // 16-bit operand size prefix
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

    /// Encode `instr` from the table of plain instructions the 64-bit
    /// encoder shares (`x86::assembler::encoder::table`), in place of what
    /// an arm that refused it left from `body` on. `None` when the table
    /// has no row for it, and the arm's own diagnostic stands.
    pub(super) fn encode_from_table(
        &mut self,
        instr: &Instruction,
        body: usize,
        relocations: usize,
    ) -> Option<Result<(), String>> {
        use crate::backend::x86::assembler::encoder::table::{self, RegField, RmField};
        let mnemonic = instr.mnemonic.to_ascii_lowercase();
        // A bare symbol is an address here (`fadds sym`): the table takes
        // it as the memory operand it is, unless a row wants the label
        // itself (`xbegin`).
        let as_written = matches!(table::plan(&mnemonic, &instr.operands, false), Some(Ok(_)));
        let addresses: Vec<Operand>;
        let operands: &[Operand] = if as_written {
            &instr.operands
        } else {
            addresses = instr
                .operands
                .iter()
                .cloned()
                .map(|mut operand| {
                    crate::backend::x86::assembler::encoder::normalize_bare_label(&mut operand);
                    operand
                })
                .collect();
            &addresses
        };
        let plan = table::plan(&mnemonic, operands, false)?.ok()?;
        self.bytes.truncate(body);
        self.relocations.truncate(relocations);
        // 16-bit code spells an address register differently, and nothing
        // that takes one is 16-bit code's to run.
        if self.code16 && plan.names_address {
            return Some(Err(format!(
                "`{mnemonic}' with an address register is not supported in 16-bit mode"
            )));
        }
        // Written as 32-bit code; `encode` inverts the operand-size prefix
        // for 16-bit code where a width was chosen -- by the statement, or
        // by `.code16gcc`, under which a stack operation is 32 bits wide.
        self.sized_op = plan.sized || (plan.stack_default && self.code16gcc);
        if plan.addr_prefix {
            self.bytes.push(0x67);
        }
        if plan.data16 {
            self.bytes.push(0x66);
        }
        if plan.prefix != 0 {
            self.bytes.push(plan.prefix);
        }
        let Some((last, head)) = plan.opcode.split_last() else {
            return Some(Err("internal: a table row without an opcode".to_string()));
        };
        self.bytes.extend_from_slice(head);
        self.bytes.push(last | plan.opcode_low);
        let number =
            |name: &str| reg_num(name).ok_or_else(|| format!("bad register name `%{name}'"));
        let emitted = (|| -> Result<(), String> {
            let field = match plan.reg {
                RegField::None => 0,
                RegField::Digit(digit) => digit,
                RegField::Register(name) => number(name)?,
            };
            match plan.rm {
                Some(RmField::Register(rm)) => {
                    let rm = number(rm)?;
                    self.bytes.push(self.modrm(3, field, rm));
                }
                Some(RmField::Memory(mem)) => self.encode_modrm_mem(field, mem)?,
                None => {}
            }
            for &(bytes, value) in plan.imm.iter().flatten() {
                self.bytes
                    .extend_from_slice(&value.to_le_bytes()[..usize::from(bytes)]);
            }
            if let Some(target) = plan.target {
                self.add_relocation(target, R_386_PC32, -4);
                self.bytes.extend_from_slice(&[0; 4]);
            }
            Ok(())
        })();
        Some(emitted)
    }
}
