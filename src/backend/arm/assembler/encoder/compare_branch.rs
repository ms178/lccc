use super::*;
use crate::backend::arm::assembler::parser::Operand;

// ── Compare ──────────────────────────────────────────────────────────────

pub(crate) fn encode_cmp(operands: &[Operand]) -> Result<EncodeResult, String> {
    // CMP Rn, op -> SUBS XZR, Rn, op
    let mut new_ops = vec![Operand::Reg("xzr".to_string())];
    new_ops.extend(operands.iter().cloned());
    // Determine if 32-bit or 64-bit from the first operand
    let is_32 = if let Some(Operand::Reg(r)) = operands.first() {
        is_32bit_reg(r)
    } else {
        false
    };
    if is_32 {
        new_ops[0] = Operand::Reg("wzr".to_string());
    }
    encode_add_sub(&new_ops, true, true)
}

pub(crate) fn encode_cmn(operands: &[Operand]) -> Result<EncodeResult, String> {
    // CMN Rn, op -> ADDS XZR, Rn, op
    let mut new_ops = vec![Operand::Reg("xzr".to_string())];
    new_ops.extend(operands.iter().cloned());
    let is_32 = if let Some(Operand::Reg(r)) = operands.first() {
        is_32bit_reg(r)
    } else {
        false
    };
    if is_32 {
        new_ops[0] = Operand::Reg("wzr".to_string());
    }
    encode_add_sub(&new_ops, false, true)
}

pub(crate) fn encode_tst(operands: &[Operand]) -> Result<EncodeResult, String> {
    // TST Rn, op -> ANDS XZR, Rn, op
    let mut new_ops = vec![Operand::Reg("xzr".to_string())];
    new_ops.extend(operands.iter().cloned());
    let is_32 = if let Some(Operand::Reg(r)) = operands.first() {
        is_32bit_reg(r)
    } else {
        false
    };
    if is_32 {
        new_ops[0] = Operand::Reg("wzr".to_string());
    }
    encode_logical(&new_ops, 0b11, "tst")
}

pub(crate) fn encode_ccmp_ccmn(
    operands: &[Operand],
    is_ccmp: bool,
) -> Result<EncodeResult, String> {
    // CCMP/CCMN Rn, #imm5, #nzcv, cond
    // The only difference: CCMP has bit 30 = 1, CCMN has bit 30 = 0
    let (rn, is_64) = get_reg(operands, 0)?;
    let sf = sf_bit(is_64);
    let op = if is_ccmp { 1u32 << 30 } else { 0u32 };

    if let (Some(Operand::Imm(imm5)), Some(Operand::Imm(nzcv)), Some(Operand::Cond(cond))) =
        (operands.get(1), operands.get(2), operands.get(3))
    {
        let cond_val = encode_cond(cond).ok_or("invalid condition")?;
        let word = (sf << 31)
            | op
            | (1 << 29)
            | (0b11010010 << 21)
            | ((*imm5 as u32 & 0x1F) << 16)
            | (cond_val << 12)
            | (1 << 11)
            | (rn << 5)
            | (*nzcv as u32 & 0xF);
        return Ok(EncodeResult::Word(word));
    }

    // CCMP/CCMN Rn, Rm, #nzcv, cond
    if let (Some(Operand::Reg(rm_name)), Some(Operand::Imm(nzcv)), Some(Operand::Cond(cond))) =
        (operands.get(1), operands.get(2), operands.get(3))
    {
        let rm = parse_reg_num(rm_name).ok_or("invalid rm")?;
        let cond_val = encode_cond(cond).ok_or("invalid condition")?;
        let word = (sf << 31)
            | op
            | (1 << 29)
            | (0b11010010 << 21)
            | (rm << 16)
            | (cond_val << 12)
            | (rn << 5)
            | (*nzcv as u32 & 0xF);
        return Ok(EncodeResult::Word(word));
    }

    let name = if is_ccmp { "ccmp" } else { "ccmn" };
    Err(format!("unsupported {} operands", name))
}

// ── Conditional select ───────────────────────────────────────────────────

pub(crate) fn encode_csel(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    let cond = match operands.get(3) {
        Some(Operand::Cond(c)) => encode_cond(c).ok_or("invalid cond")?,
        _ => return Err("csel requires condition".to_string()),
    };
    let sf = sf_bit(is_64);
    let word = ((sf << 31) | (0b11010100 << 21) | (rm << 16) | (cond << 12)) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_csinc(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    let cond = match operands.get(3) {
        Some(Operand::Cond(c)) => encode_cond(c).ok_or("invalid cond")?,
        _ => return Err("csinc requires condition".to_string()),
    };
    let sf = sf_bit(is_64);
    let word =
        (sf << 31) | (0b11010100 << 21) | (rm << 16) | (cond << 12) | (0b01 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_csinv(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    let cond = match operands.get(3) {
        Some(Operand::Cond(c)) => encode_cond(c).ok_or("invalid cond")?,
        _ => return Err("csinv requires condition".to_string()),
    };
    let sf = sf_bit(is_64);
    let word = (((sf << 31) | (1 << 30)) | (0b11010100 << 21) | (rm << 16) | (cond << 12))
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_csneg(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    let cond = match operands.get(3) {
        Some(Operand::Cond(c)) => encode_cond(c).ok_or("invalid cond")?,
        _ => return Err("csneg requires condition".to_string()),
    };
    let sf = sf_bit(is_64);
    let word = ((sf << 31) | (1 << 30))
        | (0b11010100 << 21)
        | (rm << 16)
        | (cond << 12)
        | (0b01 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_cset(operands: &[Operand]) -> Result<EncodeResult, String> {
    // CSET Rd, cond -> CSINC Rd, XZR, XZR, invert(cond)
    let (rd, is_64) = get_reg(operands, 0)?;
    let cond = match operands.get(1) {
        Some(Operand::Cond(c)) => encode_cond(c).ok_or("invalid cond")?,
        _ => return Err("cset requires condition".to_string()),
    };
    let sf = sf_bit(is_64);
    let inv_cond = cond ^ 1; // invert least significant bit
    let word = (sf << 31)
        | (0b11010100 << 21)
        | (0b11111 << 16)
        | (inv_cond << 12)
        | (0b01 << 10)
        | (0b11111 << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_csetm(operands: &[Operand]) -> Result<EncodeResult, String> {
    // CSETM Rd, cond -> CSINV Rd, XZR, XZR, invert(cond)
    let (rd, is_64) = get_reg(operands, 0)?;
    let cond = match operands.get(1) {
        Some(Operand::Cond(c)) => encode_cond(c).ok_or("invalid cond")?,
        _ => return Err("csetm requires condition".to_string()),
    };
    let sf = sf_bit(is_64);
    let inv_cond = cond ^ 1;
    let word = (((sf << 31) | (1 << 30)) | (0b11010100 << 21) | (0b11111 << 16) | (inv_cond << 12))
        | (0b11111 << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── Branches ─────────────────────────────────────────────────────────────

/// Read the branch-target register of BR/BLR/RET.
///
/// These transfer control to an address *in a register*, so the operand is a
/// 64-bit general-purpose register; XZR is encodable (field 31, `br xzr` is a
/// real instruction) but SP is not -- `br sp` used to assemble as `br xzr`,
/// i.e. a branch to address zero.
fn branch_reg(operands: &[Operand], idx: usize, mn: &str) -> Result<u32, String> {
    match operands.get(idx) {
        Some(Operand::Reg(name)) => {
            let lower = name.to_lowercase();
            if lower == "sp" || lower == "wsp" {
                return Err(format!(
                    "{mn}: `{name}` cannot be branched to; branch registers are \
                     64-bit general-purpose registers (write `xzr` for address 0)"
                ));
            }
            if lower.starts_with('w') {
                return Err(format!(
                    "{mn}: `{name}` is 32-bit; a branch target is a 64-bit register"
                ));
            }
            parse_reg_num(name).ok_or_else(|| format!("{mn}: invalid register `{name}`"))
        }
        Some(other) => Err(format!(
            "{mn}: expected a register to branch to, got {other:?}"
        )),
        None => Err(format!("{mn}: missing branch register")),
    }
}

/// The Rt of CBZ/CBNZ/TBZ/TBNZ: a 32- or 64-bit general-purpose register.
/// ZR is encodable (it is a real register value, 0), SP is not (`cbz sp,.`
/// used to assemble as `cbz xzr,.`), and neither are FP/SIMD registers
/// (`cbz d0,.` used to assemble as `cbz x0,.`).
fn test_reg(operands: &[Operand], idx: usize, mn: &str) -> Result<(u32, bool), String> {
    let reg = reg_operand(operands, idx, GpRole::RegOrZr, mn)?;
    Ok((reg.num, reg.is_64))
}

/// The 14-bit branch offset of CBZ/CBNZ (imm19) and TBZ/TBNZ (imm14), as a
/// *scaled* field, or `None` when the operand is a symbol to be relocated.
fn branch_offset_scaled(
    operands: &[Operand],
    idx: usize,
    bits: u32,
    mn: &str,
) -> Result<Option<u32>, String> {
    match operands.get(idx) {
        Some(Operand::Imm(v)) => {
            let (lo, hi) = (-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1);
            if v % 4 != 0 {
                return Err(format!(
                    "{mn}: branch offset {v} is not a multiple of 4 (instructions \
                     are 4 bytes)"
                ));
            }
            let scaled = v / 4;
            if scaled < lo || scaled > hi {
                return Err(format!(
                    "{mn}: branch offset {v} is out of range for the {bits}-bit \
                     scaled field"
                ));
            }
            Ok(Some((scaled as u32) & ((1 << bits) - 1)))
        }
        _ => Ok(None),
    }
}

pub(crate) fn encode_branch(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 1 {
        return Err(format!(
            "b takes exactly one branch target, got {} operands",
            operands.len()
        ));
    }
    if let Some(scaled) = branch_offset_scaled(operands, 0, 26, "b")? {
        // `b #imm` is a raw PC-relative offset in bytes, like GNU as: the
        // encoded field is the offset divided by four.
        return Ok(EncodeResult::Word((0b000101 << 26) | scaled));
    }
    let (sym, addend) = get_symbol(operands, 0)?;
    // B: 000101 imm26 (filled by linker/assembler)
    Ok(EncodeResult::WordWithReloc {
        word: 0b000101 << 26,
        reloc: Relocation {
            reloc_type: RelocType::Jump26,
            symbol: sym,
            addend,
        },
    })
}

pub(crate) fn encode_bl(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 1 {
        return Err(format!(
            "bl takes exactly one branch target, got {} operands",
            operands.len()
        ));
    }
    if let Some(scaled) = branch_offset_scaled(operands, 0, 26, "bl")? {
        return Ok(EncodeResult::Word((0b100101 << 26) | scaled));
    }
    let (sym, addend) = get_symbol(operands, 0)?;
    // BL: 100101 imm26
    Ok(EncodeResult::WordWithReloc {
        word: 0b100101 << 26,
        reloc: Relocation {
            reloc_type: RelocType::Call26,
            symbol: sym,
            addend,
        },
    })
}

pub(crate) fn encode_cond_branch(cond: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    let cond_val = encode_cond(cond).ok_or_else(|| format!("unknown condition: {}", cond))?;
    if operands.len() != 1 {
        return Err(format!(
            "b.{cond} takes exactly one branch target, got {} operands",
            operands.len()
        ));
    }
    if let Some(scaled) = branch_offset_scaled(operands, 0, 19, "b.cond")? {
        let word = (0b01010100 << 24) | (scaled << 5) | cond_val;
        return Ok(EncodeResult::Word(word));
    }
    let (sym, addend) = get_symbol(operands, 0)?;
    // B.cond: 01010100 imm19 0 cond
    let word = (0b01010100 << 24) | cond_val;
    Ok(EncodeResult::WordWithReloc {
        word,
        reloc: Relocation {
            reloc_type: RelocType::CondBr19,
            symbol: sym,
            addend,
        },
    })
}

pub(crate) fn encode_br(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rn = branch_reg(operands, 0, "br")?;
    // BR: 1101011 0000 11111 000000 Rn 00000
    let word = 0xd61f0000 | (rn << 5);
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_blr(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rn = branch_reg(operands, 0, "blr")?;
    // BLR: 1101011 0001 11111 000000 Rn 00000
    let word = 0xd63f0000 | (rn << 5);
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_ret(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rn = if operands.is_empty() {
        30 // default to x30 (LR)
    } else {
        branch_reg(operands, 0, "ret")?
    };
    // RET: 1101011 0010 11111 000000 Rn 00000
    let word = 0xd65f0000 | (rn << 5);
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_cbz(operands: &[Operand], is_nz: bool) -> Result<EncodeResult, String> {
    let mn = if is_nz { "cbnz" } else { "cbz" };
    if operands.len() != 2 {
        return Err(format!(
            "{mn} takes a register and a label, got {} operands",
            operands.len()
        ));
    }
    let (rt, is_64) = test_reg(operands, 0, mn)?;
    if let Some(scaled) = branch_offset_scaled(operands, 1, 19, mn)? {
        let sf = sf_bit(is_64);
        let op = if is_nz { 1u32 } else { 0u32 };
        return Ok(EncodeResult::Word(
            (sf << 31) | (0b011010 << 25) | (op << 24) | (scaled << 5) | rt,
        ));
    }
    let (sym, addend) = get_symbol(operands, 1)?;
    let sf = sf_bit(is_64);
    let op = if is_nz { 1u32 } else { 0u32 };
    // CBZ/CBNZ: sf 011010 op imm19 Rt
    let word = (sf << 31) | (0b011010 << 25) | (op << 24) | rt;
    Ok(EncodeResult::WordWithReloc {
        word,
        reloc: Relocation {
            reloc_type: RelocType::CondBr19,
            symbol: sym,
            addend,
        },
    })
}

pub(crate) fn encode_tbz(operands: &[Operand], is_nz: bool) -> Result<EncodeResult, String> {
    let mn = if is_nz { "tbnz" } else { "tbz" };
    if operands.len() != 3 {
        return Err(format!(
            "{mn} takes a register, a bit number and a label, got {} operands",
            operands.len()
        ));
    }
    let (rt, is_64) = test_reg(operands, 0, mn)?;
    let bit = get_imm(operands, 1)?;
    // The bit number is six bits wide; `tbz w0,#32,.` has no encoding because
    // a 32-bit register has no bit 32, and the old reader masked with 0x3F so
    // `tbz x0,#64,.` silently became `tbz x0,#0,.`.
    let max_bit = if is_64 { 63 } else { 31 };
    if !(0..=max_bit).contains(&bit) {
        return Err(format!(
            "{mn}: bit {bit} is outside the {}-bit register (allowed 0..={max_bit})",
            if is_64 { 64 } else { 32 }
        ));
    }
    if let Some(scaled) = branch_offset_scaled(operands, 2, 14, mn)? {
        let b5 = ((bit as u32) >> 5) & 1;
        let b40 = (bit as u32) & 0x1F;
        let op = if is_nz { 1u32 } else { 0u32 };
        return Ok(EncodeResult::Word(
            (b5 << 31) | (0b011011 << 25) | (op << 24) | (b40 << 19) | (scaled << 5) | rt,
        ));
    }
    let (sym, addend) = get_symbol(operands, 2)?;
    let b5 = ((bit as u32) >> 5) & 1;
    let b40 = (bit as u32) & 0x1F;
    let op = if is_nz { 1u32 } else { 0u32 };
    // TBZ/TBNZ: b5 011011 op b40 imm14 Rt
    let word = (b5 << 31) | (0b011011 << 25) | (op << 24) | (b40 << 19) | rt;
    Ok(EncodeResult::WordWithReloc {
        word,
        reloc: Relocation {
            reloc_type: RelocType::TstBr14,
            symbol: sym,
            addend,
        },
    })
}

// ── Additional conditional operations ────────────────────────────────────

/// Encode CNEG Rd, Rn, cond -> CSNEG Rd, Rn, Rn, invert(cond)
pub(crate) fn encode_cneg(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let cond = match operands.get(2) {
        Some(Operand::Cond(c)) => {
            encode_cond(c).ok_or_else(|| format!("unknown condition: {}", c))?
        }
        _ => return Err("cneg: expected condition code as third operand".to_string()),
    };
    let sf = sf_bit(is_64);
    // Invert the condition (flip bit 0)
    let inv_cond = cond ^ 1;
    // CSNEG: sf 1 0 11010100 Rm cond 0 1 Rn Rd (with Rm = Rn)
    let word = (sf << 31)
        | (1 << 30)
        | (0b011010100 << 21)
        | (rn << 16)
        | (inv_cond << 12)
        | (0b01 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode CINC Rd, Rn, cond -> CSINC Rd, Rn, Rn, invert(cond)
pub(crate) fn encode_cinc(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let cond = match operands.get(2) {
        Some(Operand::Cond(c)) => {
            encode_cond(c).ok_or_else(|| format!("unknown condition: {}", c))?
        }
        _ => return Err("cinc: expected condition code as third operand".to_string()),
    };
    let sf = sf_bit(is_64);
    let inv_cond = cond ^ 1;
    // CSINC: sf 0 0 11010100 Rm cond 0 1 Rn Rd (with Rm = Rn)
    let word = (sf << 31)
        | (0b011010100 << 21)
        | (rn << 16)
        | (inv_cond << 12)
        | (0b01 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode CINV Rd, Rn, cond -> CSINV Rd, Rn, Rn, invert(cond)
pub(crate) fn encode_cinv(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let cond = match operands.get(2) {
        Some(Operand::Cond(c)) => {
            encode_cond(c).ok_or_else(|| format!("unknown condition: {}", c))?
        }
        _ => return Err("cinv: expected condition code as third operand".to_string()),
    };
    let sf = sf_bit(is_64);
    let inv_cond = cond ^ 1;
    // CSINV: sf 1 0 11010100 Rm cond 0 0 Rn Rd (with Rm = Rn)
    let word = (sf << 31)
        | (1 << 30)
        | (0b011010100 << 21)
        | (rn << 16)
        | (inv_cond << 12)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}
