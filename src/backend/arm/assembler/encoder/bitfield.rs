use super::*;
use crate::backend::arm::assembler::parser::Operand;

// ── Bitfield extract/insert ──────────────────────────────────────────────

/// Encode UBFX Rd, Rn, #lsb, #width -> UBFM Rd, Rn, #lsb, #(lsb+width-1)
pub(crate) fn encode_ubfx(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (regs, is_64) = gp_same_width(operands, 2, "ubfx")?;
    let (rd, rn) = (regs[0], regs[1]);
    let _ = rn;
    let lsb = get_imm(operands, 2)? as u32;
    let width = get_imm(operands, 3)? as u32;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    let immr = lsb;
    let imms = lsb + width - 1;
    // UBFM: sf 10 100110 N immr imms Rn Rd
    let word = (sf << 31)
        | (0b10 << 29)
        | (0b100110 << 23)
        | (n << 22)
        | (immr << 16)
        | (imms << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode SBFX Rd, Rn, #lsb, #width -> SBFM Rd, Rn, #lsb, #(lsb+width-1)
pub(crate) fn encode_sbfx(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (regs, is_64) = gp_same_width(operands, 2, "sbfx")?;
    let (rd, rn) = (regs[0], regs[1]);
    let _ = rn;
    let lsb = get_imm(operands, 2)? as u32;
    let width = get_imm(operands, 3)? as u32;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    let immr = lsb;
    let imms = lsb + width - 1;
    // SBFM: sf 00 100110 N immr imms Rn Rd
    let word =
        (sf << 31) | (0b100110 << 23) | (n << 22) | (immr << 16) | (imms << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode UBFM Rd, Rn, #immr, #imms (raw form)
pub(crate) fn encode_ubfm(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (regs, is_64) = gp_same_width(operands, 2, "ubfm")?;
    let (rd, rn) = (regs[0], regs[1]);
    let _ = rn;
    let immr = get_imm(operands, 2)? as u32;
    let imms = get_imm(operands, 3)? as u32;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    let word = (sf << 31)
        | (0b10 << 29)
        | (0b100110 << 23)
        | (n << 22)
        | (immr << 16)
        | (imms << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode SBFM Rd, Rn, #immr, #imms (raw form)
pub(crate) fn encode_sbfm(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (regs, is_64) = gp_same_width(operands, 2, "sbfm")?;
    let (rd, rn) = (regs[0], regs[1]);
    let _ = rn;
    let immr = get_imm(operands, 2)? as u32;
    let imms = get_imm(operands, 3)? as u32;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    let word =
        (sf << 31) | (0b100110 << 23) | (n << 22) | (immr << 16) | (imms << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode SBFIZ Rd, Rn, #lsb, #width — alias for SBFM Rd, Rn, #(-lsb MOD regsize), #(width-1)
pub(crate) fn encode_sbfiz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (regs, is_64) = gp_same_width(operands, 2, "sbfiz")?;
    let (rd, rn) = (regs[0], regs[1]);
    let _ = rn;
    let lsb = get_imm(operands, 2)? as u32;
    let width = get_imm(operands, 3)? as u32;
    let regsize = if is_64 { 64u32 } else { 32 };
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    let immr = (regsize.wrapping_sub(lsb)) & (regsize - 1);
    let imms = width - 1;
    let word =
        (sf << 31) | (0b100110 << 23) | (n << 22) | (immr << 16) | (imms << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode UBFIZ Rd, Rn, #lsb, #width — alias for UBFM Rd, Rn, #(-lsb MOD regsize), #(width-1)
pub(crate) fn encode_ubfiz(operands: &[Operand]) -> Result<EncodeResult, String> {
    // Two general-purpose slots that share one width: these forms read
    // encoding 31 as the zero register, and `sf` with `N` is a single field,
    // so neither `bfi wzr, x1, #1, #2` nor `bfi x0, w1, #1, #2` is an
    // encoding.
    let (regs, is_64) = gp_same_width(operands, 2, "ubfiz")?;
    let (rd, rn) = (regs[0], regs[1]);
    let lsb = get_imm(operands, 2)? as u32;
    let width = get_imm(operands, 3)? as u32;
    let regsize = if is_64 { 64u32 } else { 32 };
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    let immr = (regsize.wrapping_sub(lsb)) & (regsize - 1);
    let imms = width - 1;
    let word = (sf << 31)
        | (0b10 << 29)
        | (0b100110 << 23)
        | (n << 22)
        | (immr << 16)
        | (imms << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode BFM Rd, Rn, #immr, #imms (bitfield move)
pub(crate) fn encode_bfm(operands: &[Operand]) -> Result<EncodeResult, String> {
    // Two general-purpose slots that share one width: these forms read
    // encoding 31 as the zero register, and `sf` with `N` is a single field,
    // so neither `bfi wzr, x1, #1, #2` nor `bfi x0, w1, #1, #2` is an
    // encoding.
    let (regs, is_64) = gp_same_width(operands, 2, "bfm")?;
    let (rd, rn) = (regs[0], regs[1]);
    let immr = get_imm(operands, 2)? as u32;
    let imms = get_imm(operands, 3)? as u32;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    // BFM: sf 01 100110 N immr imms Rn Rd
    let word = (sf << 31)
        | (0b01 << 29)
        | (0b100110 << 23)
        | (n << 22)
        | (immr << 16)
        | (imms << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode BFI Rd, Rn, #lsb, #width -> BFM Rd, Rn, #(-lsb mod width_reg), #(width-1)
pub(crate) fn encode_bfi(operands: &[Operand]) -> Result<EncodeResult, String> {
    // Two general-purpose slots that share one width: these forms read
    // encoding 31 as the zero register, and `sf` with `N` is a single field,
    // so neither `bfi wzr, x1, #1, #2` nor `bfi x0, w1, #1, #2` is an
    // encoding.
    let (regs, is_64) = gp_same_width(operands, 2, "bfi")?;
    let (rd, rn) = (regs[0], regs[1]);
    let lsb = get_imm(operands, 2)? as u32;
    let width = get_imm(operands, 3)? as u32;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    let reg_width = if is_64 { 64u32 } else { 32u32 };
    let immr = (reg_width - lsb) % reg_width;
    let imms = width - 1;
    let word = (sf << 31)
        | (0b01 << 29)
        | (0b100110 << 23)
        | (n << 22)
        | (immr << 16)
        | (imms << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode BFXIL Rd, Rn, #lsb, #width -> BFM Rd, Rn, #lsb, #(lsb+width-1)
pub(crate) fn encode_bfxil(operands: &[Operand]) -> Result<EncodeResult, String> {
    // Two general-purpose slots that share one width: these forms read
    // encoding 31 as the zero register, and `sf` with `N` is a single field,
    // so neither `bfi wzr, x1, #1, #2` nor `bfi x0, w1, #1, #2` is an
    // encoding.
    let (regs, is_64) = gp_same_width(operands, 2, "bfxil")?;
    let (rd, rn) = (regs[0], regs[1]);
    let lsb = get_imm(operands, 2)? as u32;
    let width = get_imm(operands, 3)? as u32;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    let immr = lsb;
    let imms = lsb + width - 1;
    let word = (sf << 31)
        | (0b01 << 29)
        | (0b100110 << 23)
        | (n << 22)
        | (immr << 16)
        | (imms << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode EXTR Rd, Rn, Rm, #lsb
pub(crate) fn encode_extr(operands: &[Operand]) -> Result<EncodeResult, String> {
    // All three slots read encoding 31 as the zero register and share one
    // width through `sf`/`N`; `extr sp, x1, x2, #3` used to assemble.
    let (regs, is_64) = gp_same_width(operands, 3, "extr")?;
    let (rd, rn, rm) = (regs[0], regs[1], regs[2]);
    let lsb = get_imm(operands, 3)? as u32;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    // EXTR: sf 0 0 100111 N 0 Rm imms Rn Rd
    let word =
        (sf << 31) | (0b00100111 << 23) | (n << 22) | (rm << 16) | (lsb << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

// ── Bit manipulation ─────────────────────────────────────────────────────

pub(crate) fn encode_clz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, rn) = dp1src_reg_pair(operands, "clz")?;
    let sf = sf_bit(rd.is_64);
    let (rd, rn) = (rd.num, rn.num);
    // CLZ: sf 1 0 11010110 00000 00010 0 Rn Rd
    let word = ((sf << 31) | (1 << 30) | (0b011010110 << 21)) | (0b000100 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_cls(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, rn) = dp1src_reg_pair(operands, "cls")?;
    let sf = sf_bit(rd.is_64);
    let (rd, rn) = (rd.num, rn.num);
    let word = ((sf << 31) | (1 << 30) | (0b011010110 << 21)) | (0b000101 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_rbit(operands: &[Operand]) -> Result<EncodeResult, String> {
    // NEON vector form: RBIT Vd.T, Vn.T (reverse bits in each byte)
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        let (rd, arr_d) = get_neon_reg(operands, 0)?;
        let (rn, _) = get_neon_reg(operands, 1)?;
        let q: u32 = if arr_d == "16b" { 1 } else { 0 };
        // RBIT (vector): 0 Q 1 01110 01 10000 00101 10 Rn Rd
        let word = (q << 30)
            | (1 << 29)
            | (0b01110 << 24)
            | (0b01 << 22)
            | (0b10000 << 17)
            | (0b00101 << 12)
            | (0b10 << 10)
            | (rn << 5)
            | rd;
        return Ok(EncodeResult::Word(word));
    }
    // Scalar form: RBIT Rd, Rn
    let (rd, rn) = dp1src_reg_pair(operands, "rbit")?;
    let sf = sf_bit(rd.is_64);
    let (rd, rn) = (rd.num, rn.num);
    let word = ((sf << 31) | (1 << 30) | (0b011010110 << 21)) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_rev(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, rn) = dp1src_reg_pair(operands, "rev")?;
    let sf = sf_bit(rd.is_64);
    let opc = if rd.is_64 { 0b000011 } else { 0b000010 };
    let (rd, rn) = (rd.num, rn.num);
    let word = ((sf << 31) | (1 << 30) | (0b011010110 << 21)) | (opc << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_rev16(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, rn) = dp1src_reg_pair(operands, "rev16")?;
    let sf = sf_bit(rd.is_64);
    let (rd, rn) = (rd.num, rn.num);
    let word = ((sf << 31) | (1 << 30) | (0b011010110 << 21)) | (0b000001 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_rev32(operands: &[Operand]) -> Result<EncodeResult, String> {
    // Check for NEON vector form: REV32 Vd.T, Vn.T
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        let (rd, arr_d) = get_neon_reg(operands, 0)?;
        let (rn, _) = get_neon_reg(operands, 1)?;
        let (q, size) = neon_arr_to_q_size(&arr_d)?;
        // REV32 Vd.T, Vn.T: 0 Q 1 01110 size 10 0000 0000 10 Rn Rd
        let word = (q << 30)
            | (1 << 29)
            | (0b01110 << 24)
            | (size << 22)
            | (0b100000 << 16)
            | (0b000010 << 10)
            | (rn << 5)
            | rd;
        return Ok(EncodeResult::Word(word));
    }
    let (rd, rn) = dp1src_reg_pair(operands, "rev32")?;
    if !rd.is_64 {
        return Err(
            "rev32: the scalar form is 64-bit only (`rev32 xd, xn`); the 32-bit \
             reversal of a 32-bit register is plain `rev`"
                .to_string(),
        );
    }
    let (rd, rn) = (rd.num, rn.num);
    // REV32 is 64-bit only: 1 1 0 11010110 00000 000010 Rn Rd
    let word = ((1u32 << 31) | (1 << 30) | (0b011010110 << 21)) | (0b000010 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

// ── CRC32 ────────────────────────────────────────────────────────────────

pub(crate) fn encode_crc32(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    // `crc32b/h/w` are W-register instructions end to end; the `x` forms keep
    // the 32-bit accumulator and destination but take a 64-bit source, so the
    // three slots cannot be read as one width list.  `crc32x w0,w1,w2`,
    // `crc32b w0,w1,x2` and `crc32x x0,w1,x2` all used to assemble.
    let rd = gp_widened(operands, 0, false, mnemonic)?;
    let rn = gp_widened(operands, 1, false, mnemonic)?;
    let rm = gp_widened(operands, 2, mnemonic.ends_with('x'), mnemonic)?;

    let is_c = mnemonic.contains("crc32c");
    let c_bit = if is_c { 1u32 } else { 0 };

    let (sf, sz) = match mnemonic {
        "crc32b" | "crc32cb" => (0u32, 0b00u32),
        "crc32h" | "crc32ch" => (0, 0b01),
        "crc32w" | "crc32cw" => (0, 0b10),
        "crc32x" | "crc32cx" => (1, 0b11),
        _ => (0, 0b00),
    };

    // CRC32: sf 0 0 11010110 Rm 010 C sz Rn Rd
    let word = (sf << 31)
        | (0b0011010110 << 21)
        | (rm << 16)
        | (0b010 << 13)
        | (c_bit << 12)
        | (sz << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}
