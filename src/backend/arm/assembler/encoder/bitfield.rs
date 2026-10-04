use super::*;
use crate::backend::arm::assembler::parser::Operand;

// ── shared validation helpers ─────────────────────────────────────────────

/// Read the two general-purpose operands (Rd at 0, Rn at 1) shared by every
/// bitfield / bit-manipulation instruction, enforcing the strict GP register
/// class and identical operand widths.
///
/// SBFM/UBFM/BFM/EXTR and the CLZ/CLS/RBIT/REV family take no SP and no
/// FP/SIMD register in any operand slot; a mixed x/w pair assembles to a
/// word whose sf bit matches only the destination (GNU as: "operand
/// mismatch"). Both defect classes were previously accepted silently
/// because the operands were read through the permissive `get_reg`, which
/// maps any register spelling onto the shared 0-31 number space.
fn bitfield_reg_pair(operands: &[Operand]) -> Result<((u32, u32), bool), String> {
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    check_same_width(is_64, &[rn64])?;
    Ok(((rd, rn), is_64))
}

/// Read an unsigned immediate at `idx` constrained to `0..=hi`.
///
/// Negative immediates used to reach the field packing as `as u32` wrapping
/// casts (`#-1` became 0xFFFFFFFF and landed verbatim in immr/imms),
/// assembling garbage words instead of diagnosing the range violation the
/// way GNU as does.
fn imm_in_range(operands: &[Operand], idx: usize, hi: u32, what: &str) -> Result<u32, String> {
    let v = get_imm(operands, idx)?;
    if !(0..=hi as i64).contains(&v) {
        return Err(format!("operand {idx}: {what} {v} out of range 0..={hi}"));
    }
    Ok(v as u32)
}

/// Validate the `(lsb, width)` immediate pair of UBFX/SBFX/BFI/SBFIZ/UBFIZ/
/// BFXIL: `1 <= width`, `lsb < regsize` and `lsb + width <= regsize`
/// (GNU as: "immediate value out of range 1 to N"/"must be N"). Returns
/// them as `u32`s safe to combine into immr/imms without wraparound.
fn lsb_width_pair(operands: &[Operand], regsize: u32) -> Result<(u32, u32), String> {
    let lsb = imm_in_range(operands, 2, regsize - 1, "lsb")?;
    let width = imm_in_range(operands, 3, regsize, "width")?;
    if width == 0 {
        return Err(format!(
            "operand 3: width 0 out of range 1..={}",
            regsize - lsb
        ));
    }
    if lsb + width > regsize {
        return Err(format!(
            "operand 3: width {width} out of range 1..={} (lsb {lsb} + width must be <= {regsize})",
            regsize - lsb
        ));
    }
    Ok((lsb, width))
}

// ── Bitfield extract/insert ──────────────────────────────────────────────

/// Encode UBFX Rd, Rn, #lsb, #width -> UBFM Rd, Rn, #lsb, #(lsb+width-1)
pub fn encode_ubfx(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 4 {
        return Err(format!(
            "ubfx requires exactly 4 operands, got {}",
            operands.len()
        ));
    }
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let regsize = if is_64 { 64u32 } else { 32 };
    let (lsb, width) = lsb_width_pair(operands, regsize)?;
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
pub fn encode_sbfx(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 4 {
        return Err(format!(
            "sbfx requires exactly 4 operands, got {}",
            operands.len()
        ));
    }
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let regsize = if is_64 { 64u32 } else { 32 };
    let (lsb, width) = lsb_width_pair(operands, regsize)?;
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
pub fn encode_ubfm(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 4 {
        return Err(format!(
            "ubfm requires exactly 4 operands, got {}",
            operands.len()
        ));
    }
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let regsize = if is_64 { 64u32 } else { 32 };
    let immr = imm_in_range(operands, 2, regsize - 1, "immr")?;
    let imms = imm_in_range(operands, 3, regsize - 1, "imms")?;
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
pub fn encode_sbfm(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 4 {
        return Err(format!(
            "sbfm requires exactly 4 operands, got {}",
            operands.len()
        ));
    }
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let regsize = if is_64 { 64u32 } else { 32 };
    let immr = imm_in_range(operands, 2, regsize - 1, "immr")?;
    let imms = imm_in_range(operands, 3, regsize - 1, "imms")?;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    let word =
        (sf << 31) | (0b100110 << 23) | (n << 22) | (immr << 16) | (imms << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode SBFIZ Rd, Rn, #lsb, #width — alias for SBFM Rd, Rn, #(-lsb MOD regsize), #(width-1)
pub fn encode_sbfiz(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 4 {
        return Err(format!(
            "sbfiz requires exactly 4 operands, got {}",
            operands.len()
        ));
    }
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let regsize = if is_64 { 64u32 } else { 32 };
    let (lsb, width) = lsb_width_pair(operands, regsize)?;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    let immr = (regsize - lsb) % regsize;
    let imms = width - 1;
    let word =
        (sf << 31) | (0b100110 << 23) | (n << 22) | (immr << 16) | (imms << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode UBFIZ Rd, Rn, #lsb, #width — alias for UBFM Rd, Rn, #(-lsb MOD regsize), #(width-1)
pub fn encode_ubfiz(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 4 {
        return Err(format!(
            "ubfiz requires exactly 4 operands, got {}",
            operands.len()
        ));
    }
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let regsize = if is_64 { 64u32 } else { 32 };
    let (lsb, width) = lsb_width_pair(operands, regsize)?;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    let immr = (regsize - lsb) % regsize;
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
pub fn encode_bfm(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 4 {
        return Err(format!(
            "bfm requires exactly 4 operands, got {}",
            operands.len()
        ));
    }
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let regsize = if is_64 { 64u32 } else { 32 };
    let immr = imm_in_range(operands, 2, regsize - 1, "immr")?;
    let imms = imm_in_range(operands, 3, regsize - 1, "imms")?;
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
pub fn encode_bfi(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 4 {
        return Err(format!(
            "bfi requires exactly 4 operands, got {}",
            operands.len()
        ));
    }
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let regsize = if is_64 { 64u32 } else { 32 };
    let (lsb, width) = lsb_width_pair(operands, regsize)?;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    let immr = (regsize - lsb) % regsize;
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
pub fn encode_bfxil(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 4 {
        return Err(format!(
            "bfxil requires exactly 4 operands, got {}",
            operands.len()
        ));
    }
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let regsize = if is_64 { 64u32 } else { 32 };
    let (lsb, width) = lsb_width_pair(operands, regsize)?;
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
pub fn encode_extr(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 4 {
        return Err(format!(
            "extr requires exactly 4 operands, got {}",
            operands.len()
        ));
    }
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    check_same_width(is_64, &[rn64, rm64])?;
    let regsize = if is_64 { 64u32 } else { 32 };
    let lsb = imm_in_range(operands, 3, regsize - 1, "lsb")?;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0u32 };
    // EXTR: sf 0 0 100111 N 0 Rm imms Rn Rd
    let word =
        (sf << 31) | (0b00100111 << 23) | (n << 22) | (rm << 16) | (lsb << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

// ── Bit manipulation ─────────────────────────────────────────────────────

pub fn encode_clz(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "clz requires exactly 2 operands, got {}",
            operands.len()
        ));
    }
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let sf = sf_bit(is_64);
    // CLZ: sf 1 0 11010110 00000 00010 0 Rn Rd
    let word = ((sf << 31) | (1 << 30) | (0b011010110 << 21)) | (0b000100 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_cls(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "cls requires exactly 2 operands, got {}",
            operands.len()
        ));
    }
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let sf = sf_bit(is_64);
    let word = ((sf << 31) | (1 << 30) | (0b011010110 << 21)) | (0b000101 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_rbit(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "rbit requires exactly 2 operands, got {}",
            operands.len()
        ));
    }
    // NEON vector form: RBIT Vd.T, Vn.T (reverse bits in each byte) —
    // defined only for .8b/.16b with both arrangements identical.
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        let (rd, rn, q) = get_neon_logical2_operands(operands, "rbit")?;
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
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let sf = sf_bit(is_64);
    let word = ((sf << 31) | (1 << 30) | (0b011010110 << 21)) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_rev(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "rev requires exactly 2 operands, got {}",
            operands.len()
        ));
    }
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let sf = sf_bit(is_64);
    let opc = if is_64 { 0b000011 } else { 0b000010 };
    let word = ((sf << 31) | (1 << 30) | (0b011010110 << 21)) | (opc << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_rev16(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "rev16 requires exactly 2 operands, got {}",
            operands.len()
        ));
    }
    let ((rd, rn), is_64) = bitfield_reg_pair(operands)?;
    let sf = sf_bit(is_64);
    let word = ((sf << 31) | (1 << 30) | (0b011010110 << 21)) | (0b000001 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_rev32(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "rev32 requires exactly 2 operands, got {}",
            operands.len()
        ));
    }
    // Check for NEON vector form: REV32 Vd.T, Vn.T — defined for .4h/.8h
    // (size=01) and .2s/.4s (size=10), both arrangements identical.
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        let (rd, arr_d) = get_vreg_arrangement(operands, 0)?;
        let (rn, arr_n) = get_vreg_arrangement(operands, 1)?;
        if arr_n != arr_d {
            return Err(format!(
                "rev32: source arrangement .{arr_n} does not match destination .{arr_d}"
            ));
        }
        let (q, size) = match arr_d.as_str() {
            "4h" => (0u32, 0b01u32),
            "8h" => (1, 0b01),
            "2s" => (0, 0b10),
            "4s" => (1, 0b10),
            _ => {
                return Err(format!(
                    "rev32: arrangement .{arr_d} is not valid (use .4h/.8h/.2s/.4s)"
                ));
            }
        };
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
    // Scalar REV32 is 64-bit only: 1 1 0 11010110 00000 000010 Rn Rd
    // (a W destination/source pair is unallocated; GNU as: "operand
    // mismatch" for `rev32 w0, w1`).
    let rd = get_gpr_strict_x(operands, 0)?;
    let rn = get_gpr_strict_x(operands, 1)?;
    let word = ((1u32 << 31) | (1 << 30) | (0b011010110 << 21)) | (0b000010 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

// ── CRC32 ────────────────────────────────────────────────────────────────

/// Encode the CRC32/CRC32C family: `crc32{b,h,w} Wd, Wn, Wm` and
/// `crc32{x} Wd, Wn, Xm`.
///
/// Exactly eight mnemonics exist; anything else is an error, not a default
/// to `(crc32b)`. The register-width contract per the ARMv8 ARM: Rd and Rn
/// are always the 32-bit register view; only Rm widens to X for the 64-bit
/// data variants (GNU as: `crc32x w0, w1, x2` -> 0x9ac24c20; every other
/// width combination is an "operand mismatch").
pub fn encode_crc32(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 3 {
        return Err(format!(
            "crc32 requires exactly 3 operands, got {}",
            operands.len()
        ));
    }
    let (c_bit, sz, rm_is_64) = match mnemonic {
        "crc32b" => (0u32, 0b00u32, false),
        "crc32h" => (0, 0b01, false),
        "crc32w" => (0, 0b10, false),
        "crc32x" => (0, 0b11, true),
        "crc32cb" => (1, 0b00, false),
        "crc32ch" => (1, 0b01, false),
        "crc32cw" => (1, 0b10, false),
        "crc32cx" => (1, 0b11, true),
        _ => {
            return Err(format!(
                "unknown CRC mnemonic `{mnemonic}` (crc32{{b,h,w,x}} / crc32c{{b,h,w,x}})"
            ));
        }
    };
    // Rd and Rn are the 32-bit view for every variant; sf in the encoding
    // reflects the *data* size carried by Rm.
    let rd = get_gpr_strict_w(operands, 0)?;
    let rn = get_gpr_strict_w(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    if rm64 != rm_is_64 {
        return Err(format!(
            "operand 2: `{}` requires a {}-bit source register",
            mnemonic,
            if rm_is_64 { "64-bit x" } else { "32-bit w" }
        ));
    }
    let sf = if rm_is_64 { 1u32 } else { 0 };

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
