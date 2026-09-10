//! APX new opcodes: PUSH2/POP2, PUSHP/POPP, JMPABS, CCMP/CTEST.
//!
//! Encodings verified against GNU as 2.47. 2-op EGPR still prefers REX2
//! (shorter than EVEX); these forms are EVEX- or REX2-only by construction.

use super::*;

impl super::InstructionEncoder {
    /// `pushp %reg` / `popp %reg`: REX2 with W=1, opcode 50+rd / 58+rd.
    /// Distinct from `pushq %r16` which is REX2 W=0.
    pub(crate) fn encode_pushp(&mut self, ops: &[Operand], pop: bool) -> Result<(), String> {
        if self.apx_nf {
            return Err("{nf} unsupported for `pushp'/`popp'".to_string());
        }
        if ops.len() != 1 {
            return Err("pushp/popp requires 1 operand".to_string());
        }
        let r = match &ops[0] {
            Operand::Register(r) => r,
            _ => return Err("pushp/popp requires a register".to_string()),
        };
        let num = reg_num(&r.name).ok_or("bad register")?;
        let (b, b4) = gp_ext_bits(&r.name);
        self.emit_rex2(true, false, false, b, false, false, b4);
        self.bytes.push(if pop { 0x58 } else { 0x50 } + (num & 7));
        Ok(())
    }

    /// `push2 %first, %second` (W=0) / `push2p` (W=1).
    /// EVEX map-4, ND=1, vvvv=second, opcode FF /6, r/m=first. RSP illegal.
    pub(crate) fn encode_push2(&mut self, ops: &[Operand], wide: bool) -> Result<(), String> {
        if self.apx_nf {
            return Err("{nf} unsupported for `push2'".to_string());
        }
        self.encode_push2_pop2(ops, wide, true)
    }

    /// `pop2` / `pop2p`. Opcode 8F /0. Dest registers must be distinct.
    pub(crate) fn encode_pop2(&mut self, ops: &[Operand], wide: bool) -> Result<(), String> {
        if self.apx_nf {
            return Err("{nf} unsupported for `pop2'".to_string());
        }
        self.encode_push2_pop2(ops, wide, false)
    }

    fn encode_push2_pop2(
        &mut self,
        ops: &[Operand],
        wide: bool,
        is_push: bool,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("push2/pop2 requires 2 register operands".to_string());
        }
        let (a, b) = match (&ops[0], &ops[1]) {
            (Operand::Register(a), Operand::Register(b)) => (a, b),
            _ => return Err("push2/pop2 requires two registers".to_string()),
        };
        for n in [&a.name, &b.name] {
            if matches!(n.as_str(), "rsp" | "esp" | "sp") {
                return Err("'rsp' register cannot be used for `push2'/`pop2'".to_string());
            }
        }
        if !is_push {
            let ia = gp_id(&a.name).ok_or("bad register")?;
            let ib = gp_id(&b.name).ok_or("bad register")?;
            if ia == ib {
                return Err("two dest registers must be distinct for `pop2'".to_string());
            }
        }
        let a_n = reg_num(&a.name).ok_or("bad register")?;
        self.emit_apx_evex_rr_pp(wide, "", &a.name, Some(&b.name), false, 0)?;
        if is_push {
            self.bytes.push(0xFF);
            self.bytes.push(self.modrm(3, 6, a_n));
        } else {
            self.bytes.push(0x8F);
            self.bytes.push(self.modrm(3, 0, a_n));
        }
        Ok(())
    }

    /// `jmpabs $imm64`: REX2 payload 0 + opcode A1 + imm64.
    /// Shorter than `movabs $imm,%rax; jmp *%rax` (11 vs 12 bytes).
    pub(crate) fn encode_jmpabs(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("jmpabs requires 1 operand".to_string());
        }
        self.emit_rex2(false, false, false, false, false, false, false);
        self.bytes.push(0xA1);
        match &ops[0] {
            Operand::Immediate(ImmediateValue::Integer(v)) => {
                self.bytes.extend_from_slice(&v.to_le_bytes());
            }
            Operand::Immediate(ImmediateValue::Symbol(sym))
            | Operand::Immediate(ImmediateValue::SymbolPlusOffset(sym, _)) => {
                let addend = match &ops[0] {
                    Operand::Immediate(ImmediateValue::SymbolPlusOffset(_, a)) => *a,
                    _ => 0,
                };
                self.add_relocation(sym, R_X86_64_64, addend);
                self.bytes.extend_from_slice(&[0u8; 8]);
            }
            _ => return Err("jmpabs requires an immediate (absolute 64-bit address)".to_string()),
        }
        Ok(())
    }

    /// CCMP: CMP with a source condition code in EVEX P2[3:0] and DFV in P1.vvvv
    /// (not inverted). No accumulator short form. Parity conditions are illegal.
    pub(crate) fn encode_ccmp(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if self.apx_nf {
            return Err("{nf} unsupported for `ccmp'".to_string());
        }
        let rest = mnemonic
            .strip_prefix("ccmp")
            .ok_or_else(|| format!("not a ccmp mnemonic: {mnemonic}"))?;
        let (cc, mut size) = split_apx_cc_size(rest)?;
        if cc == 10 || cc == 11 {
            return Err("ccmp has no parity condition (PF)".to_string());
        }
        if size == 0 {
            size = infer_ccmp_size(ops);
        }
        self.encode_ccmp_test(ops, size, cc, false)
    }

    pub(crate) fn encode_ctest(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        if self.apx_nf {
            return Err("{nf} unsupported for `ctest'".to_string());
        }
        let rest = mnemonic
            .strip_prefix("ctest")
            .ok_or_else(|| format!("not a ctest mnemonic: {mnemonic}"))?;
        let (cc, mut size) = split_apx_cc_size(rest)?;
        if cc == 10 || cc == 11 {
            return Err("ctest has no parity condition (PF)".to_string());
        }
        if size == 0 {
            size = infer_ccmp_size(ops);
        }
        self.encode_ccmp_test(ops, size, cc, true)
    }

    fn encode_ccmp_test(
        &mut self,
        ops: &[Operand],
        size: u8,
        scc: u8,
        is_test: bool,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("ccmp/ctest requires 2 operands".to_string());
        }
        let dfv = self.apx_dfv;
        let pp = if size == 2 { 1 } else { 0 };
        let w = size == 8;

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_n = reg_num(&src.name).ok_or("bad register")?;
                let dst_n = reg_num(&dst.name).ok_or("bad register")?;
                self.emit_evex_ccmp_rr(w, &src.name, &dst.name, dfv, scc, pp)?;
                let opc = if is_test {
                    if size == 1 { 0x84 } else { 0x85 }
                } else if size == 1 {
                    0x38
                } else {
                    0x39
                };
                self.bytes.push(opc);
                self.bytes.push(self.modrm(3, src_n, dst_n));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_n = reg_num(&dst.name).ok_or("bad register")?;
                self.emit_segment_prefix(mem)?;
                self.emit_evex_ccmp_rm(w, &dst.name, mem, dfv, scc, pp)?;
                let opc = if is_test {
                    if size == 1 { 0x84 } else { 0x85 }
                } else if size == 1 {
                    0x3A
                } else {
                    0x3B
                };
                self.bytes.push(opc);
                self.encode_modrm_mem(dst_n, mem)
            }
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_n = reg_num(&src.name).ok_or("bad register")?;
                self.emit_segment_prefix(mem)?;
                self.emit_evex_ccmp_rm(w, &src.name, mem, dfv, scc, pp)?;
                let opc = if is_test {
                    if size == 1 { 0x84 } else { 0x85 }
                } else if size == 1 {
                    0x38
                } else {
                    0x39
                };
                self.bytes.push(opc);
                self.encode_modrm_mem(src_n, mem)
            }
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(dst)) => {
                let val = *val;
                if !is_test {
                    Self::check_imm32s_q("ccmp", size, val)?;
                }
                let dst_n = reg_num(&dst.name).ok_or("bad register")?;
                self.emit_evex_ccmp_rr(w, "", &dst.name, dfv, scc, pp)?;
                if is_test {
                    self.bytes.push(if size == 1 { 0xF6 } else { 0xF7 });
                    self.bytes.push(self.modrm(3, 0, dst_n));
                    push_imm_sized(&mut self.bytes, val, size);
                } else if size == 1 {
                    self.bytes.push(0x80);
                    self.bytes.push(self.modrm(3, 7, dst_n));
                    self.bytes.push(val as u8);
                } else if fits_imm8(val, size) {
                    self.bytes.push(0x83);
                    self.bytes.push(self.modrm(3, 7, dst_n));
                    self.bytes.push(canonical_imm(val, size) as u8);
                } else {
                    self.bytes.push(0x81);
                    self.bytes.push(self.modrm(3, 7, dst_n));
                    push_imm_sized(&mut self.bytes, val, size);
                }
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Memory(mem)) => {
                let val = *val;
                if !is_test {
                    Self::check_imm32s_q("ccmp", size, val)?;
                }
                self.emit_segment_prefix(mem)?;
                self.emit_evex_ccmp_rm(w, "", mem, dfv, scc, pp)?;
                if is_test {
                    self.bytes.push(if size == 1 { 0xF6 } else { 0xF7 });
                    self.encode_modrm_mem(0, mem)?;
                    push_imm_sized(&mut self.bytes, val, size);
                } else if size == 1 {
                    self.bytes.push(0x80);
                    self.encode_modrm_mem(7, mem)?;
                    self.bytes.push(val as u8);
                } else if fits_imm8(val, size) {
                    self.bytes.push(0x83);
                    self.encode_modrm_mem(7, mem)?;
                    self.bytes.push(canonical_imm(val, size) as u8);
                } else {
                    self.bytes.push(0x81);
                    self.encode_modrm_mem(7, mem)?;
                    push_imm_sized(&mut self.bytes, val, size);
                }
                Ok(())
            }
            _ => Err("unsupported ccmp/ctest operands".to_string()),
        }
    }

    /// `cfcmovCC`: APX conditional-faulting CMOV.
    ///
    /// 2-op reg/reg or mem→reg: ND=0 NF=0, dest in ModRM.reg.
    /// 2-op store (reg→mem):    ND=0 NF=1, src in ModRM.reg.
    /// 3-op NDD:                ND=1 NF=1, dest in vvvv.
    pub(crate) fn encode_cfcmovcc(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        let rest = mnemonic
            .strip_prefix("cfcmov")
            .ok_or_else(|| format!("not a cfcmov mnemonic: {mnemonic}"))?;
        let (cc, mut size) = split_apx_cc_size(rest)?;
        if size == 0 {
            size = infer_ccmp_size(ops);
        }
        if size == 1 {
            return Err("cfcmov has no 8-bit form".to_string());
        }
        match ops {
            [Operand::Register(src), Operand::Register(dst)] => {
                let src_n = reg_num(&src.name).ok_or("bad register")?;
                let dst_n = reg_num(&dst.name).ok_or("bad register")?;
                self.emit_apx_evex_rr(size, &dst.name, &src.name, None, false)?;
                self.bytes.push(0x40 + cc);
                self.bytes.push(self.modrm(3, dst_n, src_n));
                Ok(())
            }
            [Operand::Memory(mem), Operand::Register(dst)] => {
                let dst_n = reg_num(&dst.name).ok_or("bad register")?;
                self.emit_segment_prefix(mem)?;
                self.emit_apx_evex_rm(size, &dst.name, mem, None, false)?;
                self.bytes.push(0x40 + cc);
                self.encode_modrm_mem(dst_n, mem)
            }
            [Operand::Register(src), Operand::Memory(mem)] => {
                let src_n = reg_num(&src.name).ok_or("bad register")?;
                self.emit_segment_prefix(mem)?;
                self.emit_apx_evex_rm(size, &src.name, mem, None, true)?;
                self.bytes.push(0x40 + cc);
                self.encode_modrm_mem(src_n, mem)
            }
            [
                Operand::Register(src),
                Operand::Register(src1),
                Operand::Register(ndd),
            ] => {
                let src_n = reg_num(&src.name).ok_or("bad register")?;
                let src1_n = reg_num(&src1.name).ok_or("bad register")?;
                self.emit_apx_evex_rr(size, &src1.name, &src.name, Some(&ndd.name), true)?;
                self.bytes.push(0x40 + cc);
                self.bytes.push(self.modrm(3, src1_n, src_n));
                Ok(())
            }
            [
                Operand::Memory(mem),
                Operand::Register(src1),
                Operand::Register(ndd),
            ] => {
                let src1_n = reg_num(&src1.name).ok_or("bad register")?;
                self.emit_segment_prefix(mem)?;
                self.emit_apx_evex_rm(size, &src1.name, mem, Some(&ndd.name), true)?;
                self.bytes.push(0x40 + cc);
                self.encode_modrm_mem(src1_n, mem)
            }
            _ => Err("unsupported cfcmov operands".to_string()),
        }
    }

    /// `imulzu` / `imulzuw`: APX zero-upper imul-immediate (opcode 6B/69).
    /// Only 16-bit is defined: ZU zeros bits 63:16 of the destination.
    pub(crate) fn encode_imulzu(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        let size = match mnemonic_size_suffix(mnemonic) {
            Some(2) | None => 2u8,
            Some(s) => {
                return Err(format!(
                    "imulzu is only defined for 16-bit operands (got size {s})"
                ));
            }
        };
        if ops
            .iter()
            .any(|op| matches!(op, Operand::Register(r) if infer_reg_size(&r.name) != 2))
        {
            return Err("imulzu requires 16-bit register operands".to_string());
        }
        let nf = self.apx_nf;
        let (val, src, dst) = match ops {
            [
                Operand::Immediate(ImmediateValue::Integer(v)),
                Operand::Register(d),
            ] => (*v, d, d),
            [
                Operand::Immediate(ImmediateValue::Integer(v)),
                Operand::Register(s),
                Operand::Register(d),
            ] => (*v, s, d),
            [
                Operand::Immediate(ImmediateValue::Integer(v)),
                Operand::Memory(mem),
                Operand::Register(d),
            ] => {
                Self::check_imm32s_q("imulzu", size, *v)?;
                let dst_n = reg_num(&d.name).ok_or("bad register")?;
                let short = fits_imm8(*v, size);
                self.emit_segment_prefix(mem)?;
                self.emit_apx_evex_rm_nd1(size, &d.name, mem, nf)?;
                let rc = self.relocations.len();
                self.bytes.push(if short { 0x6B } else { 0x69 });
                self.encode_modrm_mem(dst_n, mem)?;
                if short {
                    self.bytes.push(canonical_imm(*v, size) as u8);
                    self.adjust_rip_reloc_addend(rc, 1);
                } else {
                    self.bytes
                        .extend_from_slice(&(canonical_imm(*v, 2) as i16).to_le_bytes());
                    self.adjust_rip_reloc_addend(rc, 2);
                }
                return Ok(());
            }
            _ => return Err("imulzu requires $imm, src, dest (or $imm, dest)".to_string()),
        };
        Self::check_imm32s_q("imulzu", size, val)?;
        let src_n = reg_num(&src.name).ok_or("bad register")?;
        let dst_n = reg_num(&dst.name).ok_or("bad register")?;
        let short = fits_imm8(val, size);
        self.emit_apx_evex_rr_nd1(size, &dst.name, &src.name, nf)?;
        self.bytes.push(if short { 0x6B } else { 0x69 });
        self.bytes.push(self.modrm(3, dst_n, src_n));
        if short {
            self.bytes.push(canonical_imm(val, size) as u8);
        } else {
            self.bytes
                .extend_from_slice(&(canonical_imm(val, 2) as i16).to_le_bytes());
        }
        Ok(())
    }
}

fn split_apx_cc_size(rest: &str) -> Result<(u8, u8), String> {
    if rest.is_empty() {
        return Err("missing condition code".to_string());
    }
    if let Ok(cc) = cc_from_mnemonic(rest) {
        return Ok((cc, 0));
    }
    let (cc_str, size) = match rest.as_bytes().last() {
        Some(b'q') => (&rest[..rest.len() - 1], 8u8),
        Some(b'l') => (&rest[..rest.len() - 1], 4),
        Some(b'w') => (&rest[..rest.len() - 1], 2),
        Some(b'b') => (&rest[..rest.len() - 1], 1),
        _ => return Err(format!("unknown ccmp/ctest condition: {rest}")),
    };
    let cc = cc_from_mnemonic(cc_str)?;
    Ok((cc, size))
}

fn infer_ccmp_size(ops: &[Operand]) -> u8 {
    for op in ops {
        if let Operand::Register(r) = op {
            return infer_reg_size(&r.name);
        }
    }
    8
}

fn push_imm_sized(bytes: &mut Vec<u8>, val: i64, size: u8) {
    match size {
        1 => bytes.push(val as u8),
        2 => bytes.extend_from_slice(&(val as i16).to_le_bytes()),
        _ => bytes.extend_from_slice(&(val as i32).to_le_bytes()),
    }
}
