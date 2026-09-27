//! x87 FPU instruction encoders for i686.
//!
//! Handles x87 floating-point load/store, arithmetic, comparison,
//! and control instructions.

use super::*;

impl super::InstructionEncoder {
    // ---- x87 FPU encoding (identical to x86-64, no REX needed) ----

    pub(super) fn encode_x87_mem(
        &mut self,
        ops: &[Operand],
        opcode: &[u8],
        ext: u8,
    ) -> Result<(), String> {
        self.encode_x87_mem_raw(ops, opcode, ext)
    }

    /// Body of encode_x87_mem without segment-prefix handling.
    /// encode_x87_wait_mem uses this so the override precedes 0x9B.
    fn encode_x87_mem_raw(
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
                self.bytes.extend_from_slice(opcode);
                self.encode_modrm_mem(ext, mem)
            }
            _ => Err("x87 mem op requires memory operand".to_string()),
        }
    }

    pub(super) fn encode_x87_wait_mem(
        &mut self,
        ops: &[Operand],
        opcode: &[u8],
        ext: u8,
    ) -> Result<(), String> {
        // GAS emits the FWAIT (0x9B) byte BEFORE the segment override:
        // `9b 26 d9 70 08` for `fstenv %es:8(%eax)` (binutils 2.44).
        self.bytes.push(0x9B);
        self.encode_x87_mem_raw(ops, opcode, ext)
    }

    pub(super) fn encode_fcomip(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() == 2 {
            match &ops[0] {
                Operand::Register(reg) => {
                    let n = parse_st_num(&reg.name)?;
                    self.bytes.extend_from_slice(&[0xDF, 0xF0 + n]);
                    Ok(())
                }
                _ => Err("fcomip requires st register".to_string()),
            }
        } else if ops.is_empty() {
            self.bytes.extend_from_slice(&[0xDF, 0xF1]);
            Ok(())
        } else {
            Err("fcomip requires 0 or 2 operands".to_string())
        }
    }

    pub(super) fn encode_fucomip(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() == 2 {
            match &ops[0] {
                Operand::Register(reg) => {
                    let n = parse_st_num(&reg.name)?;
                    self.bytes.extend_from_slice(&[0xDF, 0xE8 + n]);
                    Ok(())
                }
                _ => Err("fucomip requires st register".to_string()),
            }
        } else if ops.is_empty() {
            self.bytes.extend_from_slice(&[0xDF, 0xE9]);
            Ok(())
        } else {
            Err("fucomip requires 0 or 2 operands".to_string())
        }
    }

    pub(super) fn encode_fld_st(&mut self, ops: &[Operand]) -> Result<(), String> {
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

    pub(super) fn encode_fstp_st(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("fstp requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let n = parse_st_num(&reg.name)?;
                self.bytes.extend_from_slice(&[0xDD, 0xD8 + n]);
                Ok(())
            }
            _ => Err("fstp requires st register".to_string()),
        }
    }

    pub(super) fn encode_fxch(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.is_empty() {
            // fxch with no operand defaults to st(1)
            self.bytes.extend_from_slice(&[0xD9, 0xC9]);
            return Ok(());
        }
        if ops.len() == 1 || ops.len() == 2 {
            // With 1 operand: fxch %st(i)
            // With 2 operands: fxch %st(i), %st (AT&T syntax)
            match &ops[0] {
                Operand::Register(reg) => {
                    let n = parse_st_num(&reg.name)?;
                    self.bytes.extend_from_slice(&[0xD9, 0xC8 + n]);
                    Ok(())
                }
                _ => Err("fxch requires st register".to_string()),
            }
        } else {
            Err("fxch requires 0, 1 or 2 operands".to_string())
        }
    }

    /// Encode fnstsw (store FPU status word).
    pub(super) fn encode_fnstsw(&mut self, ops: &[Operand]) -> Result<(), String> {
        self.encode_fnstsw_raw(ops)
    }

    /// Body of encode_fnstsw without segment-prefix handling (see
    /// encode_fstsw, which must place the override before 0x9B).
    fn encode_fnstsw_raw(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.is_empty() {
            // fnstsw with no operand defaults to %ax
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
            Operand::Memory(mem) => {
                self.bytes.push(0xDD);
                self.encode_modrm_mem(7, mem)
            }
            _ => Err("fnstsw requires %ax or memory operand".to_string()),
        }
    }

    pub(super) fn encode_fstsw(&mut self, ops: &[Operand]) -> Result<(), String> {
        // GAS order: FWAIT first, then the segment override (`9b 26 dd 78 08`).
        self.bytes.push(0x9B);
        self.encode_fnstsw_raw(ops)
    }

    pub(super) fn encode_x87_st_i(
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

    pub(super) fn encode_fcom(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() == 1 || ops.len() == 2 {
            match &ops[0] {
                Operand::Register(reg) => {
                    let n = parse_st_num(&reg.name)?;
                    self.bytes.extend_from_slice(&[0xD8, 0xD0 + n]);
                    Ok(())
                }
                _ => Err("fcom requires st register".to_string()),
            }
        } else if ops.is_empty() {
            self.bytes.extend_from_slice(&[0xD8, 0xD1]);
            Ok(())
        } else {
            Err("fcom requires 0, 1 or 2 operands".to_string())
        }
    }

    pub(super) fn encode_fcomp(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() == 1 || ops.len() == 2 {
            match &ops[0] {
                Operand::Register(reg) => {
                    let n = parse_st_num(&reg.name)?;
                    self.bytes.extend_from_slice(&[0xD8, 0xD8 + n]);
                    Ok(())
                }
                _ => Err("fcomp requires st register".to_string()),
            }
        } else if ops.is_empty() {
            self.bytes.extend_from_slice(&[0xD8, 0xD9]);
            Ok(())
        } else {
            Err("fcomp requires 0, 1 or 2 operands".to_string())
        }
    }

    /// Encode fucomi (unordered compare and set EFLAGS).
    pub(super) fn encode_fucomi(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() == 2 {
            match &ops[0] {
                Operand::Register(reg) => {
                    let n = parse_st_num(&reg.name)?;
                    self.bytes.extend_from_slice(&[0xDB, 0xE8 + n]);
                    Ok(())
                }
                _ => Err("fucomi requires st register".to_string()),
            }
        } else if ops.is_empty() {
            self.bytes.extend_from_slice(&[0xDB, 0xE9]);
            Ok(())
        } else {
            Err("fucomi requires 0 or 2 operands".to_string())
        }
    }

    /// Encode fucomp (unordered compare and pop, sets FPU status word).
    pub(super) fn encode_fucomp(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() == 1 {
            match &ops[0] {
                Operand::Register(reg) => {
                    let n = parse_st_num(&reg.name)?;
                    self.bytes.extend_from_slice(&[0xDD, 0xE8 + n]);
                    Ok(())
                }
                _ => Err("fucomp requires st register".to_string()),
            }
        } else if ops.len() == 2 {
            // AT&T syntax: fucomp %st(1), %st  — first operand is the source
            match &ops[0] {
                Operand::Register(reg) => {
                    let n = parse_st_num(&reg.name)?;
                    self.bytes.extend_from_slice(&[0xDD, 0xE8 + n]);
                    Ok(())
                }
                _ => Err("fucomp requires st register".to_string()),
            }
        } else if ops.is_empty() {
            // Default: fucomp %st(1)
            self.bytes.extend_from_slice(&[0xDD, 0xE9]);
            Ok(())
        } else {
            Err("fucomp requires 0, 1 or 2 operands".to_string())
        }
    }

    /// Encode fucom (unordered compare, sets FPU status word, no pop).
    pub(super) fn encode_fucom(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() == 1 {
            match &ops[0] {
                Operand::Register(reg) => {
                    let n = parse_st_num(&reg.name)?;
                    self.bytes.extend_from_slice(&[0xDD, 0xE0 + n]);
                    Ok(())
                }
                _ => Err("fucom requires st register".to_string()),
            }
        } else if ops.len() == 2 {
            // AT&T syntax: fucom %st(1), %st — first operand is the source
            match &ops[0] {
                Operand::Register(reg) => {
                    let n = parse_st_num(&reg.name)?;
                    self.bytes.extend_from_slice(&[0xDD, 0xE0 + n]);
                    Ok(())
                }
                _ => Err("fucom requires st register".to_string()),
            }
        } else if ops.is_empty() {
            // Default: fucom %st(1)
            self.bytes.extend_from_slice(&[0xDD, 0xE1]);
            Ok(())
        } else {
            Err("fucom requires 0, 1 or 2 operands".to_string())
        }
    }

    /// Map TLS modifier string to relocation type.
    pub(super) fn tls_reloc_type(&self, modifier: &str) -> u32 {
        match modifier {
            "NTPOFF" => R_386_TLS_LE,
            "TPOFF" => R_386_TLS_LE_32,
            "TLSGD" => R_386_TLS_GD,
            "TLSLDM" => R_386_TLS_LDM,
            "DTPOFF" => R_386_TLS_LDO_32,
            "GOT" => R_386_GOT32,
            "GOTOFF" => R_386_GOTOFF,
            "PLT" => R_386_PLT32,
            "GOTPC" => R_386_GOTPC,
            // GAS emits R_386_TLS_GOTIE for the GOT-slot initial-exec
            // forms; R_386_TLS_IE in a shared object forces DT_TEXTREL
            // because the linker cannot relax it without the GOT-slot
            // marking.
            "GOTNTPOFF" | "INDNTPOFF" => R_386_TLS_GOTIE,
            _ => R_386_32,
        }
    }

    /// Encode x87 register-register arithmetic (fadd/fmul/fsub/fdiv with st(i) operands).
    pub(super) fn encode_x87_arith_reg(
        &mut self,
        ops: &[Operand],
        opcode_st0: u8,
        opcode_sti: u8,
        base_modrm: u8,
    ) -> Result<(), String> {
        match ops.len() {
            0 => {
                // Bare `fadd` is `faddp %st, %st(1)` (GAS 2.47: DE C1):
                // the no-operand form pops. The D8 default encoded the
                // non-popping `fadd %st(1), %st` instead. DE pop bases
                // coincide with the D8/DC bases, so only the opcode changes.
                self.bytes.extend_from_slice(&[0xDE, base_modrm + 1]);
                Ok(())
            }
            1 => {
                // fadd %st(i) -> st(0) = st(0) op st(i)
                match &ops[0] {
                    Operand::Register(reg) => {
                        let n = parse_st_num(&reg.name)?;
                        self.bytes.extend_from_slice(&[opcode_st0, base_modrm + n]);
                        Ok(())
                    }
                    _ => Err("x87 arith requires st register operand".to_string()),
                }
            }
            2 => {
                // Two operands: fadd %st(i), %st or fadd %st, %st(i)
                match (&ops[0], &ops[1]) {
                    (Operand::Register(src), Operand::Register(dst)) => {
                        let src_n = parse_st_num(&src.name)?;
                        let dst_n = parse_st_num(&dst.name)?;
                        if dst_n == 0 {
                            // fadd %st(i), %st -> D8 (base + i)
                            self.bytes
                                .extend_from_slice(&[opcode_st0, base_modrm + src_n]);
                        } else if src_n == 0 {
                            // fadd %st, %st(i) -> DC (base + i). The DC base
                            // is the instruction's OWN base (verified vs GNU
                            // as 2.47): fsub %st,%st(i) = DC E0+i, fsubr =
                            // DC E8+i, fdiv = DC F0+i, fdivr = DC F8+i.
                            // NO swap -- the old remap table encoded fsub as
                            // fsubr (DC EC for `fsub %st,%st(4)`).
                            self.bytes
                                .extend_from_slice(&[opcode_sti, base_modrm + dst_n]);
                        } else {
                            return Err("x87 arith: one operand must be st(0)".to_string());
                        }
                        Ok(())
                    }
                    _ => Err("x87 arith requires st register operands".to_string()),
                }
            }
            _ => Err("x87 arith requires 0-2 operands".to_string()),
        }
    }

    /// Encode the x87 popping arithmetic forms (`faddp` ... `fdivrp`):
    /// `DE base+N`. Bare is st(1); one register encodes its own index;
    /// two registers encode the nonzero one. A reversed pair is only
    /// legal for `faddp`/`fmulp` (commutative); the rest are GAS's
    /// `operand type mismatch` (the reversal would change the result).
    pub(super) fn encode_x87_pop_reg(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
        base_modrm: u8,
    ) -> Result<(), String> {
        let n = match ops.len() {
            0 => 1,
            1 => match &ops[0] {
                Operand::Register(reg) => parse_st_num(&reg.name)?,
                _ => return Err("x87 pop arith requires st register operand".to_string()),
            },
            2 => match (&ops[0], &ops[1]) {
                (Operand::Register(a), Operand::Register(b)) => {
                    let (x, y) = (parse_st_num(&a.name)?, parse_st_num(&b.name)?);
                    if x == 0 {
                        y
                    } else if y == 0 {
                        if !matches!(mnemonic, "faddp" | "fmulp") {
                            return Err(format!("operand type mismatch for `{mnemonic}'"));
                        }
                        x
                    } else {
                        return Err("x87 pop arith: one operand must be st(0)".to_string());
                    }
                }
                _ => return Err("x87 pop arith requires st register operands".to_string()),
            },
            _ => return Err("x87 pop arith requires 0-2 operands".to_string()),
        };
        self.bytes.extend_from_slice(&[0xDE, base_modrm + n]);
        Ok(())
    }
}

#[cfg(test)]
mod x87_pop_tests {
    use super::*;

    fn instruction(mnemonic: &str, operands: Vec<Operand>) -> Instruction {
        Instruction {
            prefixes: Vec::new(),
            mnemonic: mnemonic.to_owned(),
            operands,
            nf: false,
            force_evex: false,
            vex_hint: None,
            force_rex2: false,
            dfv: 0,
        }
    }

    fn reg(name: &str) -> Operand {
        Operand::Register(Register {
            name: name.to_owned(),
            mask: None,
            zeroing: false,
            broadcast: None,
        })
    }

    fn hex_of(mnemonic: &str, operands: Vec<Operand>) -> String {
        let mut enc = InstructionEncoder::new();
        enc.encode(&instruction(mnemonic, operands)).unwrap();
        enc.bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn pop_forms_encode_their_register() {
        assert_eq!(hex_of("faddp", vec![]), "de c1");
        assert_eq!(hex_of("faddp", vec![reg("st(3)")]), "de c3");
        assert_eq!(hex_of("fsubp", vec![reg("st(7)")]), "de e7");
        assert_eq!(hex_of("faddp", vec![reg("st"), reg("st(2)")]), "de c2");
        assert_eq!(hex_of("fdivrp", vec![reg("st"), reg("st(5)")]), "de fd");
    }

    #[test]
    fn bare_arith_is_the_popping_form() {
        assert_eq!(hex_of("fadd", vec![]), "de c1");
        assert_eq!(hex_of("fsub", vec![]), "de e1");
        assert_eq!(hex_of("fmul", vec![]), "de c9");
    }

    #[test]
    fn dc_two_operand_forms_keep_their_own_base() {
        // The old swap table encoded `fsub %st,%st(4)` as DC EC (fsubr).
        assert_eq!(hex_of("fsub", vec![reg("st"), reg("st(4)")]), "dc e4");
        assert_eq!(hex_of("fsubr", vec![reg("st"), reg("st(4)")]), "dc ec");
        assert_eq!(hex_of("fdiv", vec![reg("st"), reg("st(4)")]), "dc f4");
        assert_eq!(hex_of("fdivr", vec![reg("st"), reg("st(4)")]), "dc fc");
    }
}
