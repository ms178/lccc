use super::*;

/// Operand-size index into the suffix-less mnemonic tables below.
///
/// Index 3 is the 64-bit spelling, which is also the fallback: for the
/// spellings these tables serve, a size that is not 1, 2 or 4 bytes can only
/// be the 64-bit case (this is what the `_ => "q"` suffix arms did before).
const fn size_index(size: u8) -> usize {
    match size {
        1 => 0,
        2 => 1,
        4 => 2,
        _ => 3,
    }
}

/// Suffix-less ALU mnemonics indexed by `[digit][size_index]`.
///
/// The digit is the ModRM `/r` extension field (`0` add, `1` or, `2` adc,
/// `3` sbb, `4` and, `5` sub, `6` xor, `7` cmp) and every caller passes a
/// literal from that closed set, so the table is exhaustive by construction.
/// Spellings are assembled at compile time instead of being formatted per
/// instruction: the encoder runs once per emitted ADD/CMP/... row, which is the
/// most frequent row kind in compiled code.
const ALU_NAMES: [[&str; 4]; 8] = [
    ["addb", "addw", "addl", "addq"],
    ["orb", "orw", "orl", "orq"],
    ["adcb", "adcw", "adcl", "adcq"],
    ["sbbb", "sbbw", "sbbl", "sbbq"],
    ["andb", "andw", "andl", "andq"],
    ["subb", "subw", "subl", "subq"],
    ["xorb", "xorw", "xorl", "xorq"],
    ["cmpb", "cmpw", "cmpl", "cmpq"],
];

/// Suffix-less shift/rotate mnemonics indexed by `[digit][size_index]`.
///
/// Digit 6 has no shift encoding; it keeps its dense slot holding the `?`
/// placeholder this code used before, so an impossible digit still reaches
/// `encode_shift` and fails there with the same message rather than panicking
/// on an index.
const SHIFT_NAMES: [[&str; 4]; 8] = [
    ["rolb", "rolw", "roll", "rolq"],
    ["rorb", "rorw", "rorl", "rorq"],
    ["rclb", "rclw", "rcll", "rclq"],
    ["rcrb", "rcrw", "rcrl", "rcrq"],
    ["shlb", "shlw", "shll", "shlq"],
    ["shrb", "shrw", "shrl", "shrq"],
    ["?b", "?w", "?l", "?q"],
    ["sarb", "sarw", "sarl", "sarq"],
];

/// `test` mnemonic indexed by `size_index`.
const TEST_NAMES: [&str; 4] = ["testb", "testw", "testl", "testq"];

/// Resolve a suffix-less mnemonic from one of the tables above.
///
/// An out-of-range digit is a caller bug, not user input, but it still has to
/// produce an error rather than a panic, and the message names the family so
/// the failure is diagnosable from the driver.
fn suffixless_name(
    table: &[[&'static str; 4]; 8],
    digit: u8,
    size: u8,
    family: &str,
) -> Result<&'static str, String> {
    table
        .get(usize::from(digit))
        .map(|names| names[size_index(size)])
        .ok_or_else(|| format!("unsupported {family} opcode digit {digit}"))
}

impl super::InstructionEncoder {
    // ---- BT/BTS/BTR/BTC encoding ----

    /// Encode bit test instructions (bt, bts, btr, btc).
    pub(crate) fn encode_bt(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }
        let size = mnemonic_size_suffix(mnemonic).unwrap_or(8);
        // 16-bit forms need the operand-size prefix.  It is pushed here, before
        // any REX byte is emitted, so that `emit_segment_prefix` (which scans
        // back over 0x66/0x67) still places a segment override outside it and
        // the REX byte produced by `emit_rex_*` lands after it.
        if size == 2 {
            self.bytes.push(0x66);
        }
        // Determine which bt variant
        let base = &mnemonic[..mnemonic.len() - 1]; // strip size suffix
        let (reg_opcode, imm_ext) = match base {
            "bt" => (0xA3u8, 4u8),
            "bts" => (0xAB, 5),
            "btr" => (0xB3, 6),
            "btc" => (0xBB, 7),
            _ => return Err(format!("unknown bt variant: {}", mnemonic)),
        };

        match (&ops[0], &ops[1]) {
            // bt $imm, reg/mem
            (Operand::Immediate(ImmediateValue::Integer(imm)), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.emit_rex_unary(size, &dst.name);
                self.bytes.extend_from_slice(&[0x0F, 0xBA]);
                self.bytes.push(self.modrm(3, imm_ext, dst_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::Integer(imm)), Operand::Memory(mem)) => {
                self.emit_rex_rm(size, "", mem);
                let rc = self.relocations.len();
                self.bytes.extend_from_slice(&[0x0F, 0xBA]);
                self.encode_modrm_mem(imm_ext, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            // bt %reg, reg/mem
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                self.emit_rex_rr(size, &src.name, &dst.name);
                self.bytes.extend_from_slice(&[0x0F, reg_opcode]);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                self.emit_rex_rm(size, &src.name, mem);
                self.bytes.extend_from_slice(&[0x0F, reg_opcode]);
                self.encode_modrm_mem(src_num, mem)
            }
            // bt $imm, label (treat label as RIP-relative memory)
            (Operand::Immediate(ImmediateValue::Integer(imm)), Operand::Label(name)) => {
                let mem = MemoryOperand {
                    segment: None,
                    displacement: Displacement::Symbol(name.clone()),
                    base: None,
                    index: None,
                    scale: None,
                    mask: None,
                    zeroing: false,
                    broadcast: None,
                };
                self.emit_rex_rm(size, "", &mem);
                let rc = self.relocations.len();
                self.bytes.extend_from_slice(&[0x0F, 0xBA]);
                self.encode_modrm_mem(imm_ext, &mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    // ---- x87 FPU encoding ----

    pub(crate) fn encode_x87_mem(
        &mut self,
        ops: &[Operand],
        opcode: &[u8],
        ext: u8,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("x87 mem op requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                // x87 memory forms accept the normal memory-prefix machinery (FS/GS
                // segment overrides and REX.B/X for r8-r15 addressing), but never REX.W.
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(opcode);
                self.encode_modrm_mem(ext, mem)
            }
            _ => Err("x87 mem op requires memory operand".to_string()),
        }
    }

    /// Encode a waiting x87 memory form.  GAS mnemonics without the `fn` prefix
    /// (for example `fstenv`, `fsave`, `fstsw`, `fclex`, and `finit`) are not
    /// distinct opcodes; they are `fwait` followed by the corresponding no-wait
    /// instruction.  The prefix is a separate instruction, so any segment/REX
    /// prefixes for the x87 memory instruction must be emitted after 0x9B.
    pub(crate) fn encode_x87_wait_mem(
        &mut self,
        ops: &[Operand],
        opcode: &[u8],
        ext: u8,
    ) -> Result<(), String> {
        self.bytes.push(0x9B);
        self.encode_x87_mem(ops, opcode, ext)
    }

    // The compares that set the flags: `fcomi %st(2)`, `fcomi %st(2), %st`.
    // The second operand names the top of the stack and nothing else
    // (`fcomi %st(2), %st(2)` and `fcomip %st(2), %cx` were assembled as
    // if it were not there).

    pub(crate) fn encode_fcomip(&mut self, ops: &[Operand]) -> Result<(), String> {
        self.encode_x87_st0_sti(ops, 0xDF, 0xF0, Some(1), "fcomip")
    }

    pub(crate) fn encode_fucomi(&mut self, ops: &[Operand]) -> Result<(), String> {
        self.encode_x87_st0_sti(ops, 0xDB, 0xE8, Some(1), "fucomi")
    }

    /// Encode fcomi/fucomi (compare, no pop): fcomi = DB F0+i, fucomi = DB E8+i.
    pub(crate) fn encode_fcomi(&mut self, ops: &[Operand], base: u8) -> Result<(), String> {
        let name = if base == 0xF0 { "fcomi" } else { "fucomi" };
        self.encode_x87_st0_sti(ops, 0xDB, base, Some(1), name)
    }

    pub(crate) fn encode_fucomip(&mut self, ops: &[Operand]) -> Result<(), String> {
        self.encode_x87_st0_sti(ops, 0xDF, 0xE8, Some(1), "fucomip")
    }

    pub(crate) fn encode_fld_st(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("fld requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let n = parse_st_num(&reg.name)?;
                self.bytes.extend_from_slice(&[0xD9, 0xC0 + n]);
                Ok(())
            }
            _ => Err("fld requires st register".to_string()),
        }
    }

    pub(crate) fn encode_fstp_st(&mut self, ops: &[Operand]) -> Result<(), String> {
        self.encode_x87_st_reg(ops, "fstp", 0xDD, 0xD8)
    }

    /// Encode a one-operand x87 instruction that takes a stack register:
    /// `<opcode> <base + N>` for `%st(N)`.
    ///
    /// `fst %st(N)` is DD D0+N and `fstp %st(N)` is DD D8+N -- adjacent rows of
    /// the same opcode block, so the two differ only in the base byte.
    pub(crate) fn encode_x87_st_reg(
        &mut self,
        ops: &[Operand],
        name: &str,
        opcode: u8,
        base: u8,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err(format!("{} requires 1 operand", name));
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let n = parse_st_num(&reg.name)?;
                self.bytes.extend_from_slice(&[opcode, base + n]);
                Ok(())
            }
            _ => Err(format!("{} requires st register", name)),
        }
    }

    /// Encode fnstsw (no-wait store FPU status word).
    pub(crate) fn encode_fnstsw(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.is_empty() {
            // GAS accepts bare `fnstsw` as `fnstsw %ax`.
            self.bytes.extend_from_slice(&[0xDF, 0xE0]);
            return Ok(());
        }
        if ops.len() != 1 {
            return Err("fnstsw requires 0 or 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) if reg.name == "ax" => {
                self.bytes.extend_from_slice(&[0xDF, 0xE0]);
                Ok(())
            }
            Operand::Memory(_) => self.encode_x87_mem(ops, &[0xDD], 7),
            _ => Err("fnstsw requires %ax or memory operand".to_string()),
        }
    }

    /// Encode fstsw (waiting store FPU status word).
    pub(crate) fn encode_fstsw(&mut self, ops: &[Operand]) -> Result<(), String> {
        self.bytes.push(0x9B);
        self.encode_fnstsw(ops)
    }

    /// Encode x87 instructions whose ModR/M byte is formed from ST(i).
    pub(crate) fn encode_x87_st_i(
        &mut self,
        ops: &[Operand],
        opcode: &[u8],
        base: u8,
        name: &str,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err(format!("{} requires 1 st register operand", name));
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let n = parse_st_num(&reg.name)?;
                self.bytes.extend_from_slice(opcode);
                self.bytes.push(base + n);
                Ok(())
            }
            _ => Err(format!("{} requires st register", name)),
        }
    }

    /// The second operand of a two-operand x87 compare is `%st`, the top of
    /// the stack it compares with (`fcom %st(1), %st(2)` and `fcom (%rax),
    /// %st(2)` were assembled as if it were not there).
    fn second_is_st(ops: &[Operand], name: &str) -> Result<(), String> {
        match ops {
            [Operand::Register(_), Operand::Register(second)]
                if parse_st_num(&second.name) == Ok(0) =>
            {
                Ok(())
            }
            [_, _] => Err(format!("`{name}' requires %st as its second operand")),
            _ => Ok(()),
        }
    }

    /// Encode fcom (compare real, no pop).  Handles the register forms emitted
    /// by GCC/clang-style inline assembly: no operand defaults to ST(1), one
    /// operand is ST(i), and two-operand AT&T syntax uses the first operand as
    /// ST(i) with ST(0) implicit as the other operand.
    pub(crate) fn encode_fcom(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.is_empty() {
            self.bytes.extend_from_slice(&[0xD8, 0xD1]);
            return Ok(());
        }
        if ops.len() == 1 || ops.len() == 2 {
            Self::second_is_st(ops, "fcom")?;
            match &ops[0] {
                Operand::Register(reg) => {
                    let n = parse_st_num(&reg.name)?;
                    self.bytes.extend_from_slice(&[0xD8, 0xD0 + n]);
                    Ok(())
                }
                Operand::Memory(mem) => {
                    // fcom m32: D8 /2 (default for a bare memory operand).
                    self.emit_rex_rm(0, "", mem);
                    self.bytes.extend_from_slice(&[0xD8]);
                    self.encode_modrm_mem(2, mem)
                }
                _ => Err("fcom requires st register or memory operand".to_string()),
            }
        } else {
            Err("fcom requires 0, 1 or 2 operands".to_string())
        }
    }

    /// Encode a sized x87 memory compare: fcoms/fcoml (opcode D8/DC, /2) and
    /// fcomps/fcompl (D8/DC, /3). AT&T suffixes: s=m32, l=m64.
    pub(crate) fn encode_fcom_mem(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        ext: u8,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("sized fcom requires 1 memory operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_rex_rm(0, "", mem);
                self.bytes.push(opcode);
                self.encode_modrm_mem(ext, mem)
            }
            _ => Err("sized fcom requires memory operand".to_string()),
        }
    }

    /// Encode fcomp (compare real and pop).  Operand handling mirrors fcom.
    pub(crate) fn encode_fcomp(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.is_empty() {
            self.bytes.extend_from_slice(&[0xD8, 0xD9]);
            return Ok(());
        }
        if ops.len() == 1 || ops.len() == 2 {
            Self::second_is_st(ops, "fcomp")?;
            match &ops[0] {
                Operand::Register(reg) => {
                    let n = parse_st_num(&reg.name)?;
                    self.bytes.extend_from_slice(&[0xD8, 0xD8 + n]);
                    Ok(())
                }
                Operand::Memory(mem) => {
                    // fcomp m32: D8 /3 (default for a bare memory operand).
                    self.emit_rex_rm(0, "", mem);
                    self.bytes.extend_from_slice(&[0xD8]);
                    self.encode_modrm_mem(3, mem)
                }
                _ => Err("fcomp requires st register or memory operand".to_string()),
            }
        } else {
            Err("fcomp requires 0, 1 or 2 operands".to_string())
        }
    }

    /// Encode an x87 instruction whose ModR/M byte is `base + ST(i)`, with
    /// ST(0) as the implicit other operand (`fcmovb`, `fcomi`, ...).
    ///
    /// AT&T spells the source first, so a two-operand form is
    /// `%st(i), %st` and the register is taken from the *first* operand while
    /// the second is required to name ST(0).  `default_reg` supplies the
    /// implicit operand of the bare spelling; `fcmov*` has none, because the
    /// condition alone does not determine the source register.
    pub(crate) fn encode_x87_st0_sti(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        base: u8,
        default_reg: Option<u8>,
        name: &str,
    ) -> Result<(), String> {
        let n = match ops {
            [] => default_reg.ok_or_else(|| format!("`{name}' requires an st register operand"))?,
            [Operand::Register(reg)] => parse_st_num(&reg.name)?,
            [Operand::Register(first), Operand::Register(second)] => {
                if parse_st_num(&second.name)? != 0 {
                    return Err(format!("`{name}' requires %st as its second operand"));
                }
                parse_st_num(&first.name)?
            }
            _ => return Err(format!("`{name}' requires an st register operand")),
        };
        self.bytes.extend_from_slice(&[opcode, base + n]);
        Ok(())
    }

    /// Encode fucom (`DD E0+i`, no pop) and fucomp (`DD E8+i`, pop).
    ///
    /// The bare spelling compares against ST(1) (SDM Vol. 2B, FUCOM/FUCOMP
    /// "FUCOM ST(1)"), which is the form x87 exit sequences use.
    fn encode_x87_unordered_compare(
        &mut self,
        ops: &[Operand],
        base: u8,
        name: &str,
    ) -> Result<(), String> {
        let n = match ops {
            [] => 1,
            [Operand::Register(reg)] => parse_st_num(&reg.name)?,
            [Operand::Register(first), Operand::Register(second)] => {
                if parse_st_num(&second.name)? != 0 {
                    return Err(format!("`{name}' requires %st as its second operand"));
                }
                parse_st_num(&first.name)?
            }
            _ => return Err(format!("`{name}' requires an st register operand")),
        };
        self.bytes.extend_from_slice(&[0xDD, base + n]);
        Ok(())
    }

    /// Encode fucom (compare unordered, no pop).
    pub(crate) fn encode_fucom(&mut self, ops: &[Operand]) -> Result<(), String> {
        self.encode_x87_unordered_compare(ops, 0xE0, "fucom")
    }

    /// Encode fucomp (compare unordered and pop).
    pub(crate) fn encode_fucomp(&mut self, ops: &[Operand]) -> Result<(), String> {
        self.encode_x87_unordered_compare(ops, 0xE8, "fucomp")
    }

    // ---- Bit scan (BSF/BSR) ----

    pub(crate) fn encode_bit_scan(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
        opcode2: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }
        let size = mnemonic_size_suffix(mnemonic).unwrap_or(8);
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rr(size, &dst.name, &src.name);
                self.bytes.extend_from_slice(&[0x0F, opcode2]);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_rm(size, &dst.name, mem);
                self.bytes.extend_from_slice(&[0x0F, opcode2]);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    // ---- Segment register moves ----

    /// `mov` between a segment register and a general register or memory.
    ///
    /// `size` is the mnemonic's suffix in bytes, if it has one. GNU as 2.47,
    /// byte-probed: `mov %ds,%ax` = `66 8c d8`, `mov %ds,%eax` = `8c d8`,
    /// `mov %ds,%rax` = `8c d8` (the store zero-extends: never `REX.W`),
    /// `mov %ax,%ds` = `mov %eax,%ds` = `8e d8` (the load reads 16 bits
    /// whatever the register is called: never `66`), `movw %ss,(%rax)` =
    /// `8c 10` (memory is 16 bits: no prefix, and no suffix but `w`).
    ///
    /// The other operand has to be what the instruction takes -- a general
    /// register of 16, 32 or 64 bits, of the suffix's width if there is
    /// one -- or memory. Anything else used to be encoded by its register
    /// number: `mov %ds,%xmm0` as `mov %ds,%eax`, `mov %ds,%es` likewise,
    /// and `mov %ds,%al`, taken for a byte move, as `mov %bl,%al`.
    pub(crate) fn encode_mov_seg(
        &mut self,
        ops: &[Operand],
        size: Option<u8>,
    ) -> Result<(), String> {
        let [first, second] = ops else {
            return Err("mov seg requires 2 operands".to_string());
        };
        let mismatch = || "operand type mismatch for `mov'".to_string();
        // The width of the general register beside the segment register.
        let general = |reg: &Register| -> Result<u8, String> {
            match reg_class(&reg.name) {
                RegClass::Gp(width @ (2 | 4 | 8)) if size.is_none_or(|s| s == width) => Ok(width),
                RegClass::Gp(2 | 4 | 8) => Err(format!(
                    "incorrect register `%{}' used with `{}' suffix",
                    reg.name,
                    match size {
                        Some(2) => 'w',
                        Some(4) => 'l',
                        _ => 'q',
                    }
                )),
                _ => Err(mismatch()),
            }
        };
        let segment = |reg: &Register| reg_num(&reg.name).ok_or("bad seg register");
        match (first, second) {
            // 8C /r: MOV r/m16, Sreg.
            (Operand::Register(src), Operand::Register(dst)) if is_segment_reg(&src.name) => {
                let width = general(dst)?;
                let (src_num, dst_num) = (segment(src)?, reg_num(&dst.name).ok_or("bad register")?);
                if width == 2 {
                    self.bytes.push(0x66);
                }
                self.emit_rex_unary(4, &dst.name);
                self.bytes.push(0x8C);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            // 8E /r: MOV Sreg, r/m16. `REX` still names the register
            // (`mov %r8d,%ds` = `41 8e d8`).
            (Operand::Register(src), Operand::Register(dst)) if is_segment_reg(&dst.name) => {
                general(src)?;
                let (src_num, dst_num) = (reg_num(&src.name).ok_or("bad register")?, segment(dst)?);
                self.emit_rex_unary(4, &src.name);
                self.bytes.push(0x8E);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Register(src), Operand::Memory(mem)) if is_segment_reg(&src.name) => {
                if size.is_some_and(|s| s != 2) {
                    return Err(mismatch());
                }
                let src_num = segment(src)?;
                self.emit_rex_rm(0, "", mem);
                self.bytes.push(0x8C);
                self.encode_modrm_mem(src_num, mem)
            }
            (Operand::Memory(mem), Operand::Register(dst)) if is_segment_reg(&dst.name) => {
                if size.is_some_and(|s| s != 2) {
                    return Err(mismatch());
                }
                let dst_num = segment(dst)?;
                self.emit_rex_rm(0, "", mem);
                self.bytes.push(0x8E);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err(mismatch()),
        }
    }

    // ---- MMX movq ----

    pub(crate) fn encode_mmx_movq(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("mmx movq requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            // movq %mm, %mm — `.s` selects the store-direction row
            // 0F 7F (GAS 2.47: `movq.s %mm0,%mm4' = 0f 7f c4; plain
            // movq keeps the 0F 6F load row).
            (Operand::Register(src), Operand::Register(dst))
                if is_mmx(&src.name) && is_mmx(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                if self.s_flip {
                    self.bytes.extend_from_slice(&[0x0F, 0x7F]);
                    self.bytes.push(self.modrm(3, src_num, dst_num));
                } else {
                    self.bytes.extend_from_slice(&[0x0F, 0x6F]);
                    self.bytes.push(self.modrm(3, dst_num, src_num));
                }
                Ok(())
            }
            // movq mem, %mm
            (Operand::Memory(mem), Operand::Register(dst)) if is_mmx(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0x6F]);
                self.encode_modrm_mem(dst_num, mem)
            }
            // movq %mm, mem
            (Operand::Register(src), Operand::Memory(mem)) if is_mmx(&src.name) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0x7F]);
                self.encode_modrm_mem(src_num, mem)
            }
            // movq %gp64, %mm -> 0F 6E (with REX.W)
            (Operand::Register(src), Operand::Register(dst))
                if !is_mmx(&src.name) && is_mmx(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.emit_rex_rr(8, &dst.name, &src.name);
                self.bytes.extend_from_slice(&[0x0F, 0x6E]);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            // movq %mm, %gp64 -> 0F 7E (with REX.W)
            (Operand::Register(src), Operand::Register(dst))
                if is_mmx(&src.name) && !is_mmx(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.emit_rex_rr(8, &src.name, &dst.name);
                self.bytes.extend_from_slice(&[0x0F, 0x7E]);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            _ => Err("unsupported mmx movq operands".to_string()),
        }
    }

    // ---- Suffix-less instruction helpers ----

    pub(crate) fn encode_suffixless_mov(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("mov requires 2 operands".to_string());
        }
        // Check for segment registers
        if ops
            .iter()
            .any(|op| matches!(op, Operand::Register(r) if is_segment_reg(&r.name)))
        {
            return self.encode_mov_seg(ops, None);
        }
        // Check for control registers
        if ops
            .iter()
            .any(|op| matches!(op, Operand::Register(r) if is_control_reg(&r.name)))
        {
            return self.encode_mov_cr(ops);
        }
        // Check for debug registers
        if ops
            .iter()
            .any(|op| matches!(op, Operand::Register(r) if is_debug_reg(&r.name)))
        {
            return self.encode_mov_dr(ops);
        }
        // Suffix-less size law (GAS 2.47): a REGISTER operand decides;
        // immediate-to-MEMORY with no register anywhere defaults to 32-bit
        // (`mov $foo,ebx` in %-prefix-less mode is memory-at-symbol `ebx`,
        // and GAS emits the 32-bit `c7 04 25 ..` — LCCC used to pick the
        // 64-bit `48 c7`, writing 8 bytes where GAS writes 4).
        let size = match (&ops[0], &ops[1]) {
            (Operand::Immediate(_), Operand::Memory(_) | Operand::Label(_))
                if !ops.iter().any(|op| matches!(op, Operand::Register(_))) =>
            {
                4
            }
            _ => infer_operand_size_from_pair(&ops[0], &ops[1]),
        };
        self.encode_mov(ops, size)
    }

    pub(crate) fn encode_suffixless_alu(
        &mut self,
        ops: &[Operand],
        alu_op: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 && ops.len() != 3 {
            return Err("ALU op requires 2 or 3 operands".to_string());
        }
        let size = if ops.len() == 3 {
            match ops.last() {
                Some(Operand::Register(r)) => infer_reg_size(&r.name),
                _ => infer_operand_size_from_pair(&ops[0], &ops[1]),
            }
        } else {
            infer_operand_size_from_pair(&ops[0], &ops[1])
        };
        // `format!` allocated a `String` for every suffix-less ALU row -- the
        // most common row in compiled code -- only to hand the encoder a
        // three-character mnemonic and drop it again.  The name is fully
        // determined by (digit, size), both of which come from small closed
        // domains, so it is a table lookup into `'static` text: no buffer, no
        // bounds arithmetic and no UTF-8 validation on the hot path.
        let mnemonic = suffixless_name(&ALU_NAMES, alu_op, size, "ALU")?;
        self.encode_alu(ops, mnemonic, alu_op)
    }

    pub(crate) fn encode_suffixless_test(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("test requires 2 operands".to_string());
        }
        let size = infer_operand_size_from_pair(&ops[0], &ops[1]);
        // Table lookup for the same reason as in `encode_suffixless_alu`.
        let mnemonic = TEST_NAMES[size_index(size)];
        self.encode_test(ops, mnemonic)
    }

    pub(crate) fn encode_suffixless_shift(
        &mut self,
        ops: &[Operand],
        shift_op: u8,
    ) -> Result<(), String> {
        if ops.len() < 1 || ops.len() > 3 {
            return Err("shift requires 1-3 operands".to_string());
        }
        // Size comes from the destination (last non-%cl register, since the
        // central suffix inference now routes suffixed forms elsewhere this
        // only serves memory-only spellings) — GAS 2.47 defaults a
        // memory-only shift to 32 bits (`shl (%rax)` = `d1 20`).
        let size = ops
            .iter()
            .rev()
            .find_map(|op| match op {
                Operand::Register(r) if r.name != "cl" => Some(infer_reg_size(&r.name)),
                _ => None,
            })
            .unwrap_or(4);
        // Same reasoning as `encode_suffixless_alu`: the mnemonic is a pure
        // function of (digit, size), so it comes from `'static` data.
        let mnemonic = suffixless_name(&SHIFT_NAMES, shift_op, size, "shift")?;
        self.encode_shift(ops, mnemonic, shift_op)
    }

    pub(crate) fn encode_suffixless_unary(
        &mut self,
        ops: &[Operand],
        op_ext: u8,
    ) -> Result<(), String> {
        if ops.len() != 1 && ops.len() != 2 {
            return Err("unary op requires 1 or 2 operands".to_string());
        }
        // GAS 2.47 size law: the register operand decides; a memory-only
        // form defaults to 32 bits (`neg (%rax)` assembles to `f7 18`, NOT
        // `48 f7 18`, with the "no register operands" warning — byte-verified
        // against the 2.47 oracle). The old 64-bit default silently widened
        // every suffixless memory unary, legacy and {nf} alike.
        let size = match ops.last() {
            Some(Operand::Register(r)) => infer_reg_size(&r.name),
            _ => 4,
        };
        if size == 2 && !self.apx_wants_evex() && ops.len() == 1 {
            self.bytes.push(0x66);
        }
        self.encode_unary_rm(ops, op_ext, size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every spelling the removed `format!` code produced must be reproduced
    /// exactly, for every digit that can reach these helpers and every operand
    /// size. A wrong table entry would silently assemble a different operation,
    /// so the whole cross product is pinned rather than sampled.
    #[test]
    fn suffixless_names_match_the_formatted_spellings() {
        let sizes = [1u8, 2, 4, 8];
        let suffixes = ["b", "w", "l", "q"];
        let alu_stems = ["add", "or", "adc", "sbb", "and", "sub", "xor", "cmp"];
        for (digit, stem) in alu_stems.iter().enumerate() {
            for (size, suffix) in sizes.iter().zip(suffixes.iter()) {
                let got = suffixless_name(&ALU_NAMES, digit as u8, *size, "ALU").unwrap();
                assert_eq!(got, format!("{stem}{suffix}"));
            }
        }
        let shift_stems = ["rol", "ror", "rcl", "rcr", "shl", "shr", "?", "sar"];
        for (digit, stem) in shift_stems.iter().enumerate() {
            for (size, suffix) in sizes.iter().zip(suffixes.iter()) {
                let got = suffixless_name(&SHIFT_NAMES, digit as u8, *size, "shift");
                assert_eq!(got.unwrap(), format!("{stem}{suffix}"));
            }
        }
        for (size, suffix) in sizes.iter().zip(suffixes.iter()) {
            assert_eq!(TEST_NAMES[size_index(*size)], format!("test{suffix}"));
        }
    }

    /// A size that is not 1, 2 or 4 bytes keeps the 64-bit spelling, which is
    /// what the `_ => "q"` fallback arms did before the tables existed.
    #[test]
    fn unusual_sizes_keep_the_64_bit_spelling() {
        for size in [0u8, 3, 5, 16, 255] {
            assert_eq!(size_index(size), 3);
            assert_eq!(suffixless_name(&ALU_NAMES, 0, size, "ALU"), Ok("addq"));
        }
    }

    /// An impossible digit is a caller bug, but it must not panic on an index:
    /// it has to come back as a diagnosable error.
    #[test]
    fn out_of_range_digits_are_errors_not_panics() {
        for digit in 8u8..=255 {
            assert!(suffixless_name(&ALU_NAMES, digit, 4, "ALU").is_err());
            assert!(suffixless_name(&SHIFT_NAMES, digit, 4, "shift").is_err());
        }
        let err = suffixless_name(&ALU_NAMES, 8, 4, "ALU").unwrap_err();
        assert!(err.contains("ALU opcode digit 8"));
    }
}
