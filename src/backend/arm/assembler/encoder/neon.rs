use super::*;
use crate::backend::arm::assembler::parser::Operand;

// ── NEON/SIMD ────────────────────────────────────────────────────────────

/// Helper to extract register number from a RegArrangement operand

/// Validate that a NEON register list is consecutive modulo 32 (ARM ISA
/// requirement for ld1/st1/ld2/... multi-register forms). Only the first
/// register is encoded; hardware derives the rest, so a non-consecutive
/// list would silently access DIFFERENT registers.
pub(crate) fn validate_consecutive_reglist(regs: &[Operand], rt: u32) -> Result<(), String> {
    for (k, r) in regs.iter().enumerate().skip(1) {
        let rk = match r {
            Operand::RegArrangement { reg, .. } => {
                parse_reg_num(reg).ok_or("invalid register in list")?
            }
            Operand::Reg(name) => parse_reg_num(name).ok_or("invalid register in list")?,
            _ => return Err("expected register with arrangement in list".to_string()),
        };
        let expected = (rt + k as u32) % 32;
        if rk != expected {
            return Err(format!(
                "register list must be consecutive: expected v{}, got v{}",
                expected, rk
            ));
        }
    }
    Ok(())
}

pub(crate) fn get_neon_reg(operands: &[Operand], idx: usize) -> Result<(u32, String), String> {
    match operands.get(idx) {
        Some(Operand::RegArrangement { reg, arrangement }) => {
            let num =
                parse_reg_num(reg).ok_or_else(|| format!("invalid NEON register: {}", reg))?;
            Ok((num, arrangement.clone()))
        }
        Some(Operand::Reg(name)) => {
            let num = parse_reg_num(name).ok_or_else(|| format!("invalid register: {}", name))?;
            Ok((num, String::new()))
        }
        other => Err(format!(
            "expected NEON register at operand {}, got {:?}",
            idx, other
        )),
    }
}

pub(crate) fn encode_cnt(operands: &[Operand]) -> Result<EncodeResult, String> {
    // CNT Vd.<T>, Vn.<T>
    // Encoding: 0 Q 00 1110 size 10 0000 0101 10 Rn Rd
    // Only valid for .8b (Q=0) and .16b (Q=1)
    if operands.len() < 2 {
        return Err("cnt requires 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _arr_n) = get_neon_reg(operands, 1)?;

    let q: u32 = if arr_d == "16b" { 1 } else { 0 }; // .8b -> Q=0, .16b -> Q=1

    // 0 Q 00 1110 00 10 0000 0101 10 Rn Rd
    let word =
        ((q << 30) | (0b001110 << 24)) | (0b100000 << 16) | (0b010110 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON three-same register operations ──────────────────────────────────

/// Get Q bit and size from arrangement specifier.
pub(crate) fn neon_arr_to_q_size(arr: &str) -> Result<(u32, u32), String> {
    match arr {
        "8b" => Ok((0, 0b00)),
        "16b" => Ok((1, 0b00)),
        "4h" => Ok((0, 0b01)),
        "8h" => Ok((1, 0b01)),
        "2s" => Ok((0, 0b10)),
        "4s" => Ok((1, 0b10)),
        "1d" => Ok((0, 0b11)),
        "2d" => Ok((1, 0b11)),
        _ => Err(format!("unsupported NEON arrangement: {}", arr)),
    }
}

/// Encode NEON three-same-register instructions: CMEQ, UQSUB, SQSUB, CMHI, etc.
///
/// Layout: 0 Q U 01110 size 1 Rm opcode 1 Rn Rd
///         31 30 29 28-24 23-22 21 20-16 15-11 10 9-5 4-0
///
/// `u_bit`: U field (bit 29) - 0 for signed, 1 for unsigned
/// `opcode`: instruction opcode (bits 15-11)
pub(crate) fn encode_neon_three_same(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("NEON three-same requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _arr_n) = get_neon_reg(operands, 1)?;
    let (rm, _arr_m) = get_neon_reg(operands, 2)?;

    let (q, size) = neon_arr_to_q_size(&arr_d)?;

    // 0 Q U 01110 size 1 Rm opcode 1 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (opcode << 11)
        | (1 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON three-different instructions: USUBL, SSUBL, UADDL, SADDL, etc.
///
/// These instructions have wider destination than source operands.
/// Format: 0 Q U 01110 size 1 Rm opcode 00 Rn Rd
///
/// `u_bit`: 0 for signed, 1 for unsigned
/// `opcode`: 4-bit opcode (bits 15-12)
/// `is_high`: true for the "2" variant (upper half, Q=1)
pub(crate) fn encode_neon_three_diff(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("NEON three-different requires 3 operands".to_string());
    }
    let (rd, _arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let (rm, _arr_m) = get_neon_reg(operands, 2)?;

    // Size is determined from the source (narrow) arrangement
    let (q, size) = match arr_n.as_str() {
        "8b" => (0u32, 0b00u32), // base
        "16b" => (1, 0b00),      // "2" variant
        "4h" => (0, 0b01),
        "8h" => (1, 0b01),
        "2s" => (0, 0b10),
        "4s" => (1, 0b10),
        _ => {
            return Err(format!(
                "unsupported source arrangement for three-diff: {}",
                arr_n
            ));
        }
    };

    // For the "2" variant, override Q
    let q = if is_high { 1 } else { q };

    // 0 Q U 01110 size 1 Rm opcode 00 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (opcode << 12)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON SQSHRUN/SQSHRUN2: Signed saturating shift right unsigned narrow
/// Format: 0 Q 1 011110 immh immb 100011 Rn Rd
pub(crate) fn encode_neon_sqshrun(
    operands: &[Operand],
    is_rounding: bool,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("sqshrun requires 3 operands".to_string());
    }
    let (rd, _arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let shift = match &operands[2] {
        Operand::Imm(v) => *v as u32,
        _ => return Err("sqshrun: expected immediate shift".to_string()),
    };

    // immh:immb encode element size and shift amount
    // For source .4s (dest .4h or .8h): immh=001x, shift_amount = 32 - (immh:immb)
    // For source .8h (dest .8b or .16b): immh=0001, shift_amount = 16 - (immh:immb)
    // For source .2d (dest .2s or .4s): immh=01xx, shift_amount = 64 - (immh:immb)
    let (element_bits, immh_base) = match arr_n.as_str() {
        "8h" => (16u32, 0b0001u32),
        "4s" => (32, 0b0010),
        "2d" => (64, 0b0100),
        _ => {
            return Err(format!(
                "sqshrun: unsupported source arrangement: {}",
                arr_n
            ));
        }
    };

    if shift == 0 || shift > element_bits {
        return Err(format!(
            "sqshrun: shift {} out of range for {}-bit elements",
            shift, element_bits
        ));
    }

    let immhb = (element_bits - shift) & 0x7F; // immh:immb combined
    let immh = (immhb >> 3) | immh_base;
    let immb = immhb & 0x7;

    let q = if is_high { 1u32 } else { 0 };

    // 0 Q 1 011110 immh immb opcode 1 Rn Rd
    // SQSHRUN: opcode = 100001, SQRSHRUN: opcode = 100011
    let opcode_bits: u32 = if is_rounding { 0b100011 } else { 0b100001 };
    let word = (q << 30)
        | (1 << 29)
        | (0b011110 << 23)
        | (immh << 19)
        | (immb << 16)
        | (opcode_bits << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON UXTL/SXTL (unsigned/signed extend long).
/// These are aliases for USHLL/SSHLL with shift #0.
///
/// Format: 0 Q U 011110 immh immb 10100 1 Rn Rd
pub(crate) fn encode_neon_xtl(
    operands: &[Operand],
    u_bit: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("NEON uxtl/sxtl requires 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;

    // immh encodes the source element size, immb=0 (shift=0).  The two
    // mnemonics pick which half of the source is widened, and the oracle spells
    // that in the *source* arrangement: `uxtl v0.8h,v1.8b` reads the low eight
    // bytes and `uxtl2 v0.8h,v1.16b` the high eight, while
    // `uxtl v0.8h,v1.16b` is not an instruction.
    let (immh, want_dst) = match (arr_n.as_str(), is_high) {
        ("8b", false) => (0b0001u32, "8h"),
        ("16b", true) => (0b0001, "8h"),
        ("4h", false) => (0b0010, "4s"),
        ("8h", true) => (0b0010, "4s"),
        ("2s", false) => (0b0100, "2d"),
        ("4s", true) => (0b0100, "2d"),
        _ => {
            return Err(format!(
                "uxtl/sxtl: `.{arr_n}` is not a source arrangement of `{}`; the \
                 non-`2` form widens the low half (.8b/.4h/.2s) and the `2` form \
                 the upper half (.16b/.8h/.4s)",
                if is_high { "uxtl2/sxtl2" } else { "uxtl/sxtl" }
            ));
        }
    };
    if arr_d != want_dst {
        return Err(format!(
            "uxtl/sxtl: destination should be `.{want_dst}` for a `.{arr_n}` source"
        ));
    }

    let q = if is_high { 1u32 } else { 0 };

    // 0 Q U 011110 immh immb 10100 1 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b011110 << 23)
        | (immh << 19)
        | (0b101001 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON compare-to-zero: CMEQ Vd, Vn, #0, CMGE Vd, Vn, #0, etc.
///
/// Format: 0 Q U 01110 size 10000 opcode 10 Rn Rd
pub(crate) fn encode_neon_cmp_zero(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("NEON compare-zero requires at least 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;

    // 0 Q U 01110 size 10000 opcode 10 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON two-register miscellaneous narrowing: UQXTN, SQXTN, XTN
///
/// Format: 0 Q U 01110 size 10000 opcode 10 Rn Rd
pub(crate) fn encode_neon_two_misc_narrow(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("NEON two-reg narrow requires 2 operands".to_string());
    }
    let (rd, _arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;

    // Size from source (wider) arrangement
    let size = match arr_n.as_str() {
        "8h" => 0b00u32,
        "4s" => 0b01,
        "2d" => 0b10,
        _ => {
            return Err(format!(
                "unsupported source arrangement for narrow: {}",
                arr_n
            ));
        }
    };

    let q = if is_high { 1u32 } else { 0 };

    // 0 Q U 01110 size 10000 opcode 10 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON vector-by-element long instructions: SMULL/UMULL/SMLAL/UMLAL/SMLSL/UMLSL (elem)
///
/// Format: 0 Q U 01111 size L M Rm opcode H 0 Rn Rd
///
/// These are the widening multiply-by-element forms where the third operand
/// is a register lane (e.g., v0.h[2]).
pub(crate) fn encode_neon_elem_long(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("NEON elem-long requires 3 operands".to_string());
    }
    let (rd, _arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;

    // Third operand is RegLane: v0.h[2]
    let (rm, index) = match &operands[2] {
        Operand::RegLane {
            reg,
            elem_size: _,
            index,
        } => {
            let rm = parse_reg_num(reg).ok_or("invalid NEON register")?;
            (rm, *index)
        }
        _ => {
            return Err(format!(
                "expected register lane operand, got {:?}",
                operands[2]
            ));
        }
    };

    // Determine size and Q from source arrangement
    let (q, size) = match arr_n.as_str() {
        "4h" => (0u32, 0b01u32),
        "8h" => (1, 0b01),
        "2s" => (0, 0b10),
        "4s" => (1, 0b10),
        _ => {
            return Err(format!(
                "unsupported source arrangement for elem-long: {}",
                arr_n
            ));
        }
    };
    let q = if is_high { 1 } else { q };

    // Encode index into H:L:M bits depending on element size
    let (h, l, m) = match size {
        0b01 => {
            // Half-word: index = H:L:M (3 bits), Rm limited to v0-v15
            if index > 7 {
                return Err(format!("element index {} out of range for .h", index));
            }
            let h = (index >> 2) & 1;
            let l = (index >> 1) & 1;
            let m = index & 1;
            (h, l, m)
        }
        0b10 => {
            // Word: index = H:L (2 bits), M=Rm[4]
            if index > 3 {
                return Err(format!("element index {} out of range for .s", index));
            }
            let h = (index >> 1) & 1;
            let l = index & 1;
            let m = (rm >> 4) & 1; // M bit from Rm[4]
            (h, l, m)
        }
        _ => return Err("unsupported element size for by-element".to_string()),
    };

    // Limit Rm for half-word indexing (only v0-v15)
    let rm_enc = if size == 0b01 { rm & 0xF } else { rm & 0x1F };

    // 0 Q U 01111 size L M Rm opcode H 0 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01111 << 24)
        | (size << 22)
        | (l << 21)
        | (m << 20)
        | (rm_enc << 16)
        | (opcode << 12)
        | (h << 11)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON logical operations: ORR/AND/EOR Vd.T, Vn.T, Vm.T
pub(crate) fn encode_neon_logical(operands: &[Operand], opc: u32) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _arr_n) = get_neon_reg(operands, 1)?;
    let (rm, _arr_m) = get_neon_reg(operands, 2)?;

    let q: u32 = if arr_d == "16b" { 1 } else { 0 };

    // NEON logical three-same:
    // ORR: 0 Q 0 01110 10 1 Rm 000111 Rn Rd  (opc=0b01 -> size=10)
    // AND: 0 Q 0 01110 00 1 Rm 000111 Rn Rd  (opc=0b00 -> size=00)
    // EOR: 0 Q 1 01110 00 1 Rm 000111 Rn Rd  (opc=0b10 -> size=00, U=1)
    // BIC: 0 Q 0 01110 01 1 Rm 000111 Rn Rd  (would be opc=0b01 with N=1... but not needed)
    let (u_bit, size_bits): (u32, u32) = match opc {
        0b00 => (0, 0b00), // AND
        0b01 => (0, 0b10), // ORR
        0b10 => (1, 0b00), // EOR
        0b11 => (1, 0b00), // ANDS - not valid for NEON, fall back
        _ => return Err("unsupported NEON logical opc".to_string()),
    };

    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size_bits << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b000111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON MUL Vd.T, Vn.T, Vm.T
pub(crate) fn encode_neon_mul(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;

    // MUL (vector): 0 Q 0 01110 size 1 Rm 10011 1 Rn Rd
    let word = (q << 30)
        | (0b001110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b100111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON PMUL Vd.T, Vn.T, Vm.T (polynomial multiply, bytes only)
pub(crate) fn encode_neon_pmul(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let q: u32 = if arr_d == "16b" { 1 } else { 0 };
    // PMUL: 0 Q 1 01110 00 1 Rm 10011 1 Rn Rd (size=00 for bytes, U=1)
    // PMUL encoding: size=00 (bytes) is implicit (zero bits at [23:22])
    let word = (q << 30)
        | (1 << 29)
        | (0b01110 << 24)
        | (1 << 21)
        | (rm << 16)
        | (0b100111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON MLA Vd.T, Vn.T, Vm.T (multiply-accumulate)
pub(crate) fn encode_neon_mla(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;
    // MLA: 0 Q 0 01110 size 1 Rm 10010 1 Rn Rd
    let word = (q << 30)
        | (0b001110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b100101 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON MLS Vd.T, Vn.T, Vm.T (multiply-subtract)
pub(crate) fn encode_neon_mls(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;
    // MLS: 0 Q 1 01110 size 1 Rm 10010 1 Rn Rd (U=1)
    let word = (q << 30)
        | (1 << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b100101 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON USHR Vd.T, Vn.T, #shift (unsigned shift right immediate)
pub(crate) fn encode_neon_shift_imm(
    operands: &[Operand],
    _is_unsigned: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("ushr requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)?;

    let (q, _size) = neon_arr_to_q_size(&arr_d)?;

    // USHR: 0 Q 1 011110 immh:immb 00000 1 Rn Rd
    // For .16b (bytes, size=8): immh = 0001, immb = 8-shift (3 bits)
    // For .8h (halfwords, size=16): immh = 001x
    // For .4s (words, size=32): immh = 01xx
    // For .2d (doublewords, size=64): immh = 1xxx
    // immh:immb = (element_size * 2 - shift)
    let (elem_bits, immh_immb) = match arr_d.as_str() {
        "8b" | "16b" => (8u32, (16 - shift as u32) & 0xF), // immh:immb is 4 bits for 8-bit elems
        "4h" | "8h" => (16, (32 - shift as u32) & 0x1F),
        "2s" | "4s" => (32, (64 - shift as u32) & 0x3F),
        "2d" => (64, (128 - shift as u32) & 0x7F),
        _ => return Err(format!("unsupported USHR arrangement: {}", arr_d)),
    };
    let _ = elem_bits;

    // Full encoding: 0 Q 1 011110 immh:immb 000001 Rn Rd
    let word = (q << 30)
        | (1 << 29)
        | (0b011110 << 23)
        | (immh_immb << 16)
        | (0b000001 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON EXT (extract from a vector pair).
///
/// `EXT Vd.T, Vn.T, Vm.T, #index` exists for the byte arrangements only: the
/// index selects a byte position in the concatenation of Vn and Vm, so a
/// 16-byte vector indexes 0..15 and an 8-byte one 0..7.  The previous version
/// took Q from `arr == "16b"`, masked the index with `& 0xF` and never looked
/// at Vn/Vm's arrangement, so `ext v0.8b, …, #8` assembled as index 0 of a
/// 16-byte extract and the halfword/word/double spellings assembled as byte
/// extracts -- three different instructions than the one written, all of them
/// silently.
pub(crate) fn encode_neon_ext(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 4 {
        return Err(format!(
            "ext requires exactly 4 operands (Vd.T, Vn.T, Vm.T, #index), got {}",
            operands.len()
        ));
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let (rm, arr_m) = get_neon_reg(operands, 2)?;
    for (slot, arr) in [(1, &arr_n), (2, &arr_m)] {
        if *arr != arr_d {
            return Err(format!(
                "ext: operand {slot} is v.{arr} but the destination is v.{arr_d}; \
                 all three registers must have the same arrangement"
            ));
        }
    }
    let (q, max_index) = match arr_d.as_str() {
        "8b" => (0u32, 7i64),
        "16b" => (1, 15),
        other => {
            return Err(format!(
                "ext: v0.{other} has no EXT encoding (only 8b and 16b; the index \
                 selects a byte in the concatenation of the two sources)"
            ));
        }
    };
    let index = get_imm(operands, 3)?;
    if !(0..=max_index).contains(&index) {
        return Err(format!(
            "ext: index {index} is outside the {}-byte arrangement (allowed 0..={max_index})",
            if q == 1 { 16 } else { 8 }
        ));
    }

    // EXT Vd.T, Vn.T, Vm.T, #index: 0 Q 10 1110 00 0 Rm 0 imm4 0 Rn Rd
    let word =
        ((q << 30) | (0b101110 << 24) | (rm << 16) | ((index as u32) << 11)) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// The destination of an across-vector reduction is a *scalar* whose class has
/// to match the reduction width: `addv b0,v1.8b` writes a byte, `addv h0,v1.4h`
/// a halfword, `addv s0,v1.4s` a word.  Writing `addv b0,v1.4h` (dest class
/// disagreeing with the source arrangement) used to assemble, and the
/// architecture has no encoding for it: the Q/size fields would describe a
/// 4-halfword reduction whose result lands in a register the programmer
/// spelled as a byte.
fn check_across_dest_class(operands: &[Operand], size: u32, mn: &str) -> Result<(), String> {
    let want = match size {
        0b00 => b'b',
        0b01 => b'h',
        _ => b's',
    };
    match operands.first().and_then(neon_operand_class) {
        Some(got) if got == want => Ok(()),
        Some(got) => Err(format!(
            "{mn}: the destination is a `{}` register but the source reduction \
             produces a `{}` value",
            got as char, want as char
        )),
        None => Err(format!(
            "{mn}: the destination must be a scalar register (b0/h0/s0)"
        )),
    }
}

/// Encode the FP across-vector reductions: FMAXV, FMINV, FMAXNMV, FMINNMV.
///
/// ```text
/// 0 Q 1 01110 size 11000 opcode 10 Rn Rd
/// ```
///
/// Only the `4s` source exists (GNU as rejects `fmaxv s0,v1.2s` and
/// `fmaxv h0,v1.8h`), the destination is always spelled as a scalar `s0`, and
/// the two opcodes differ by `size` as well as by the opcode field: FMAX/FMIN
/// use size 00 and 10 respectively.  These were simply unimplemented, so every
/// call to them was a hard error rather than a wrong word -- but a missing
/// instruction in an assembler that a compiler drives is a correctness bug in
/// its own right.
pub(crate) fn encode_neon_fp_across(
    operands: &[Operand],
    mn: &str,
    size: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "{mn} requires exactly 2 operands ({mn} s0, Vn.4s), got {}",
            operands.len()
        ));
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    if neon_operand_class(&operands[0]) != Some(b's') {
        return Err(format!(
            "{mn}: the destination must be a scalar FP register (s0-s31)"
        ));
    }
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    if arr_n != "4s" {
        return Err(format!(
            "{mn}: source arrangement `v.{arr_n}` has no FP reduction encoding \
             (expected v1.4s)"
        ));
    }
    let word = (1 << 30)
        | (1 << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b11000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Arrangement → (Q, size) for the across-vector reductions (ADDV, SMAXV,
/// SMINV, UMAXV, UMINV, ADDP, …).
///
/// These exist for 8B/16B (size 00), 4H/8H (size 01) and 4S (size 10) only.
/// There is no 2S form -- a 64-bit vector of two 32-bit lanes has no reduction
/// instruction -- and no 64-bit-lane form at all, so the `2s`/`1d`/`2d`
/// spellings used to be accepted and encoded as *unallocated* words.
pub(crate) fn across_arr_to_q_size(arr: &str) -> Result<(u32, u32), String> {
    match arr {
        "8b" => Ok((0, 0b00)),
        "16b" => Ok((1, 0b00)),
        "4h" => Ok((0, 0b01)),
        "8h" => Ok((1, 0b01)),
        "4s" => Ok((1, 0b10)),
        other => Err(format!(
            "across-vector: v0.{other} has no reduction encoding \
             (expected 8b, 16b, 4h, 8h or 4s)"
        )),
    }
}

/// The class letter of a scalar or vector NEON operand: `b`, `h`, `s`, `d` or
/// `q` for `b0`/`v0.8b`-style spellings, `None` when the operand is a plain
/// vector register with a multi-lane arrangement the caller must interpret.
///
/// (An `fmaxv`'s destination is written `s0` -- a scalar -- while its source is
/// `v1.4s`; the destination's *class* is what says a 32-bit reduction is being
/// written, and it is the only thing that distinguishes `fmaxv s0,v1.4s` from
/// the several arrangements the family does not have.)
fn neon_operand_class(operand: &Operand) -> Option<u8> {
    let name = match operand {
        Operand::Reg(n) => n.as_str(),
        Operand::RegArrangement { reg, arrangement } => {
            // The parser records `b0`/`s0`/... both as a bare register and as
            // an arrangement; the arrangement is the more specific of the two.
            if matches!(arrangement.as_str(), "b" | "h" | "s" | "d" | "q") {
                return Some(arrangement.as_bytes()[0]);
            }
            reg.as_str()
        }
        _ => return None,
    };
    let c = *name.as_bytes().first()?;
    matches!(c, b'b' | b'h' | b's' | b'd' | b'q').then_some(c)
}

/// Encode NEON ADDV: add across vector lanes
pub(crate) fn encode_neon_addv(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("addv requires 2 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;

    let (q, size) = across_arr_to_q_size(&arr_n)?;
    check_across_dest_class(operands, size, "addv")?;

    // ADDV: 0 Q 0 01110 size 11000 11011 10 Rn Rd
    let word = (q << 30)
        | (0b01110 << 24)
        | (size << 22)
        | (0b11000 << 17)
        | (0b11011 << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON across-vector instructions: UMAXV, UMINV, SMAXV, SMINV
///
/// Format: 0 Q U 01110 size 11000 opcode 10 Rn Rd
///
/// `u_bit`: 0 for signed, 1 for unsigned
/// `opcode`: 5-bit opcode (bits 16-12)
pub(crate) fn encode_neon_across(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("NEON across-vector requires 2 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;

    // The across-vector forms share ADDV's (size, Q) table: `2s`, `1d` and
    // `2d` are not reductions this architecture has.
    let (q, size) = across_arr_to_q_size(&arr_n)?;
    check_across_dest_class(operands, size, "across-vector")?;

    // 0 Q U 01110 size 11000 opcode 10 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b11000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON UMOV: move element to GP register
/// Element width in bits for a NEON arrangement.
pub(crate) fn arrangement_elem_bits(arr: &str) -> Option<u32> {
    Some(match arr {
        "8b" | "16b" => 8,
        "4h" | "8h" => 16,
        "2s" | "4s" => 32,
        "2d" => 64,
        _ => return None,
    })
}

/// Highest lane index that exists for an element class.
fn elem_lane_max(el: &str) -> Option<u32> {
    Some(match el {
        "b" => 15,
        "h" => 7,
        "s" => 3,
        "d" => 1,
        _ => return None,
    })
}

/// Reject a lane that does not exist in its element class.
///
/// Every lane field in this file used to be written `index & mask`, which
/// produces *a* lane for every index: `dup v0.16b,v1.b[16]` assembled as lane 0
/// although GNU as rejects it.  Silent wraparound is the worst possible answer,
/// because the reader gets a different program than the one that was written.
pub(crate) fn check_lane(index: u32, el: &str, mn: &str, what: &str) -> Result<(), String> {
    match elem_lane_max(el) {
        Some(max) if index <= max => Ok(()),
        Some(max) => Err(format!(
            "{mn}: {what} lane {index} does not exist; `.{el}` has lanes 0..={max}"
        )),
        None => Err(format!("{mn}: `.{el}` is not an element type")),
    }
}

/// The destination of a narrowing shift must be the half-width shape of its
/// source, and the `2` forms -- which write the *upper* half -- are spelled
/// with the full destination arrangement: `sqshrn2 v0.16b,v1.8h,#8`, not
/// `sqshrn2 v0.8b,v1.8h,#8`.
fn check_narrow_pair(dst: &str, src: &str, is_high: bool, mn: &str) -> Result<(), String> {
    let want = match (src, is_high) {
        ("8h", false) => "8b",
        ("8h", true) => "16b",
        ("4s", false) => "4h",
        ("4s", true) => "8h",
        ("2d", false) => "2s",
        ("2d", true) => "4s",
        _ => {
            return Err(format!(
                "{mn}: `.{src}` is not a narrowing source (expected .8h, .4s or .2d)"
            ));
        }
    };
    if dst != want {
        return Err(format!(
            "{mn}: destination should be `.{want}` for a `.{src}` source{}",
            if is_high {
                " (the `2` form writes the upper half, so it is spelled with the full arrangement)"
            } else {
                ""
            }
        ));
    }
    Ok(())
}

/// Check a NEON immediate shift against the range the oracle accepts.
///
/// Left shifts run `#0..#(bits-1)`; right shifts run `#1..#bits`, because
/// shifting by the full element width is how these encodings spell "shift by
/// zero".  The encoders used to mask the amount into the immh:immb field, so
/// `ushr v0.16b,v1.16b,#0` -- which GNU as rejects -- quietly encoded a shift
/// by 16.
fn check_shift(shift: u32, bits: u32, right: bool, mn: &str) -> Result<(), String> {
    let (lo, hi) = if right { (1, bits) } else { (0, bits - 1) };
    if !(lo..=hi).contains(&shift) {
        return Err(format!(
            "{mn}: shift #{shift} is outside the encodable range #{lo}..#={hi}"
        ));
    }
    Ok(())
}

/// Encode UMOV/SMOV: an element to a general-purpose register.
///
/// The two mnemonics differ in one bit (SMOV sign-extends, UMOV zero-extends)
/// and in which destinations exist, because the extension has to mean
/// something:
///
/// ```text
/// element | UMOV Wd | UMOV Xd | SMOV Wd | SMOV Xd
/// --------+---------+---------+---------+--------
///   .b    |   yes   |    --   |   yes   |   yes
///   .h    |   yes   |    --   |   yes   |   yes
///   .s    |   yes   |    --   |    --   |   yes
///   .d    |    --   |   yes   |    --   |    --
/// ```
///
/// `.b`/`.h`/`.s` into an X register has no encoding -- a zero-extended 32-bit
/// read already clears the upper half, and the instruction has no 64-bit form
/// for those sizes -- and extension from `.d` is the identity, so only the
/// zero-extending mnemonic exists there.  GNU as enforces exactly this table;
/// the published encoder took the destination width as the Q bit and encoded
/// `umov x0,v1.b[0]` as a valid-looking instruction.
pub(crate) fn encode_neon_umov_smov(
    operands: &[Operand],
    mn: &str,
    is_smov: bool,
) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!("{mn} requires 2 operands, got {}", operands.len()));
    }
    let rd = reg_operand(operands, 0, GpRole::RegOrZr, mn)?;
    let (reg, elem_size, index) = match operands.get(1) {
        Some(Operand::RegLane {
            reg,
            elem_size,
            index,
        }) => (reg, elem_size, *index),
        other => {
            return Err(format!(
                "{mn}: expected `vN.T[lane]` at operand 1, got {other:?}"
            ));
        }
    };
    let rn = parse_reg_num(reg).ok_or_else(|| format!("{mn}: invalid NEON register `{reg}`"))?;
    if rd.is_sp {
        return Err(format!(
            "{mn}: operand 0 is the stack pointer; this field holds a general-purpose register"
        ));
    }
    check_lane(index, elem_size, mn, "element")?;
    let ok = match (elem_size.as_str(), rd.is_64, is_smov) {
        ("b" | "h", false, _) | ("b" | "h", true, true) => true,
        ("s", false, false) | ("s", true, true) => true,
        ("d", true, false) => true,
        _ => false,
    };
    if !ok {
        return Err(format!(
            "{mn}: a `.{elem_size}` element cannot be read into `{}`; \
             see the UMOV/SMOV destination table",
            operand_spelling(operands, 0)
        ));
    }
    let q = u32::from(rd.is_64);
    let imm5 = match elem_size.as_str() {
        "b" => (index << 1) | 0b00001,
        "h" => (index << 2) | 0b00010,
        "s" => (index << 3) | 0b00100,
        _ => (index << 4) | 0b01000,
    };
    // 0 Q 0 01110 000 imm5 op 0111 1 Rn Rd; op is 1111 for UMOV, 1011 for SMOV.
    let op: u32 = if is_smov { 0b001011 } else { 0b001111 };
    let word = (q << 30) | (0b001110000u32 << 21) | (imm5 << 16) | (op << 10) | (rn << 5) | rd.num;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON DUP: broadcast GP register to all vector lanes
pub(crate) fn encode_neon_dup(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("dup requires 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;

    // DUP Vd.T, Rn (general form - broadcast GP reg to vector)
    if matches!(operands.get(1), Some(Operand::Reg(_))) {
        let (q, _) = neon_arr_to_q_size(&arr_d)?;

        // imm5 encoding for element size:
        // .8b/.16b: imm5 = 00001
        // .4h/.8h:  imm5 = 00010
        // .2s/.4s:  imm5 = 00100
        // .2d:      imm5 = 01000
        let (imm5, wants_64) = match arr_d.as_str() {
            "8b" | "16b" => (0b00001u32, false),
            "4h" | "8h" => (0b00010, false),
            "2s" | "4s" => (0b00100, false),
            "2d" => (0b01000, true),
            _ => return Err(format!("unsupported dup arrangement: {}", arr_d)),
        };
        // The source is one element wide: a W register for .b/.h/.s and an X
        // register for .d.  GNU as rejects `dup v0.16b,x1` (and `dup v0.2d,w1`),
        // because the element width and the register width have to agree.
        let src = reg_operand(operands, 1, GpRole::RegOrZr, "dup")?;
        if src.is_64 != wants_64 {
            return Err(format!(
                "dup: `{}` does not feed a `.{arr_d}` broadcast; the source is one \
                 element wide, so it must be a {} register",
                operand_spelling(operands, 1),
                if wants_64 { "64-bit" } else { "32-bit" }
            ));
        }
        let rn = src.num;

        // DUP Vd.T, Rn: 0 Q 0 01110 000 imm5 0 0001 1 Rn Rd
        let word =
            (q << 30) | (0b001110000u32 << 21) | (imm5 << 16) | (0b000011 << 10) | (rn << 5) | rd;
        return Ok(EncodeResult::Word(word));
    }

    // DUP Vd.T, Vn.Ts[index] (broadcast element to all lanes)
    if let Some(Operand::RegLane {
        reg,
        elem_size,
        index,
    }) = operands.get(1)
    {
        let rn = parse_reg_num(reg).ok_or("invalid NEON register")?;
        let (q, _) = neon_arr_to_q_size(&arr_d)?;

        // imm5 encodes both element size and index:
        // .b[i]: imm5 = (i << 1) | 0b00001
        // .h[i]: imm5 = (i << 2) | 0b00010
        // .s[i]: imm5 = (i << 3) | 0b00100
        // .d[i]: imm5 = (i << 4) | 0b01000
        check_lane(*index, elem_size, "dup", "source element")?;
        let imm5 = match elem_size.as_str() {
            "b" => (*index << 1) | 0b00001,
            "h" => (*index << 2) | 0b00010,
            "s" => (*index << 3) | 0b00100,
            "d" => (*index << 4) | 0b01000,
            _ => return Err(format!("unsupported dup element size: {}", elem_size)),
        };

        // DUP Vd.T, Vn.Ts[i]: 0 Q 0 01110 000 imm5 0 0000 1 Rn Rd
        let word =
            (q << 30) | (0b001110000u32 << 21) | (imm5 << 16) | (0b000001 << 10) | (rn << 5) | rd;
        return Ok(EncodeResult::Word(word));
    }

    Err("unsupported dup operands".to_string())
}

/// Encode NEON INS (insert element from GP register): INS Vd.Ts[index], Xn
pub(crate) fn encode_neon_ins(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("ins requires 2 operands".to_string());
    }
    match (&operands[0], &operands[1]) {
        // INS Vd.Ts[dst_idx], Xn (general register to element)
        (
            Operand::RegLane {
                reg,
                elem_size,
                index,
            },
            Operand::Reg(_),
        ) => {
            let rd = parse_reg_num(reg).ok_or("invalid NEON register")?;

            check_lane(*index, elem_size, "ins", "destination element")?;
            // The GP source is one element wide: W for .b/.h/.s, X for .d.
            let src = reg_operand(operands, 1, GpRole::RegOrZr, "ins")?;
            let wants_64 = elem_size == "d";
            if src.is_64 != wants_64 {
                return Err(format!(
                    "ins: `{}` does not feed a `.{elem_size}` element; the source is \
                     one element wide, so it must be a {} register",
                    operand_spelling(operands, 1),
                    if wants_64 { "64-bit" } else { "32-bit" }
                ));
            }
            let rn = src.num;

            let imm5 = match elem_size.as_str() {
                "b" => (*index << 1) | 0b00001,
                "h" => (*index << 2) | 0b00010,
                "s" => (*index << 3) | 0b00100,
                "d" => (*index << 4) | 0b01000,
                _ => return Err(format!("unsupported ins element size: {}", elem_size)),
            };

            // INS Vd.Ts[i], Xn: 0 1 0 01110 000 imm5 0 0011 1 Rn Rd
            let word = (0b01001110000u32 << 21) | (imm5 << 16) | (0b000111 << 10) | (rn << 5) | rd;
            Ok(EncodeResult::Word(word))
        }
        // INS Vd.Ts[dst_idx], Vn.Ts[src_idx] (element to element)
        (
            Operand::RegLane {
                reg: rd_name,
                elem_size: dst_size,
                index: dst_idx,
            },
            Operand::RegLane {
                reg: rn_name,
                elem_size: src_size,
                index: src_idx,
            },
        ) => {
            let rd = parse_reg_num(rd_name).ok_or("invalid NEON rd")?;
            let rn = parse_reg_num(rn_name).ok_or("invalid NEON rn")?;

            // Element-to-element moves keep the element type; only the lane
            // changes.  `ins v0.b[3],v1.h[2]` is not a byte copy of a halfword.
            if src_size != dst_size {
                return Err(format!(
                    "ins: destination element is `.{dst_size}` but the source is \
                     `.{src_size}`; both must be the same element type"
                ));
            }
            check_lane(*dst_idx, dst_size, "ins", "destination element")?;
            check_lane(*src_idx, src_size, "ins", "source element")?;
            let (imm5, imm4) = match dst_size.as_str() {
                "b" => ((*dst_idx << 1) | 0b00001, *src_idx),
                "h" => ((*dst_idx << 2) | 0b00010, *src_idx << 1),
                "s" => ((*dst_idx << 3) | 0b00100, *src_idx << 2),
                "d" => ((*dst_idx << 4) | 0b01000, *src_idx << 3),
                _ => return Err(format!("unsupported ins element size: {}", dst_size)),
            };

            // INS Vd.Ts[dst], Vn.Ts[src]: 0 1 1 01110 000 imm5 0 imm4 1 Rn Rd
            let word =
                (0b01101110000u32 << 21) | (imm5 << 16) | (imm4 << 11) | (1 << 10) | (rn << 5) | rd;
            Ok(EncodeResult::Word(word))
        }
        _ => Err("ins: expected (RegLane, Reg) or (RegLane, RegLane) operands".to_string()),
    }
}

/// Encode NEON NOT (bitwise NOT): NOT Vd.T, Vn.T
pub(crate) fn encode_neon_not(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("not requires 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;

    let q: u32 = if arr_d == "16b" { 1 } else { 0 };

    // NOT Vd.T, Vn.T (alias of MVN): 0 Q 1 01110 00 10000 00101 10 Rn Rd
    let word = ((q << 30) | (1 << 29) | (0b01110 << 24))
        | (0b10000 << 17)
        | (0b00101 << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON MOVI (move immediate to vector)
/// The `cmode`/`op` a MOVI/MVNI shift selector maps to, or the reason this
/// arrangement has no such encoding.
///
/// MOVI and MVNI are one encoding family (AdvSIMD modified immediate):
///
/// ```text
/// 0 Q op 0 1111 0 abc cmode o2 1 defgh Rd
/// ```
///
/// `cmode` chooses the per-lane element size *and* the shift the 8-bit
/// immediate is placed with, so the shift is part of the encoding rather than
/// a cosmetic suffix on the mnemonic: mapping a dropped `msl #8` onto the
/// unshifted `cmode` (0000) makes the instruction materialise a different
/// constant, silently.  The accepted sets below are exactly GNU as's, pinned
/// row by row in the `neon movi shift`/`neon mvni` groups of
/// tests/aarch64/operand-legality.tsv -- including the forms that do *not*
/// exist (no `msl` for 16-bit lanes, no 64-bit MVNI, no shift at all on 8-bit
/// lanes).
///
/// Returns `(cmode, op, imm_is_byte_mask)`: `op` is 1 for MVNI and for the
/// 64-bit MOVI (which is itself an op=1 encoding), and the byte-mask flag
/// tells the caller the immediate must be built from eight 0x00/0xFF bytes
/// instead of being used literally.
fn neon_modified_imm_layout(
    mn: &str,
    arr: &str,
    shift: Option<(&str, u32)>,
    is_mvni: bool,
) -> Result<(u32, u32, bool), String> {
    // `lsl #0` is the unshifted encoding spelled explicitly; GNU as accepts it
    // everywhere the unshifted form exists, so normalise it away first.
    let shift = match shift {
        Some(("lsl", 0)) => None,
        other => other,
    };
    let bad_shift = |kind: &str, amount: u32| -> String {
        format!(
            "{mn}: `{kind} #{amount}` is not an available shift for v0.{arr} \
             (see the architecture's modified-immediate table)"
        )
    };
    let (cmode, op) = match (arr, shift) {
        // 8-bit lanes: the immediate *is* the byte, so no shift is encodable.
        ("8b" | "16b", None) => (0b1110, 0),
        ("8b" | "16b", Some((kind, amount))) => return Err(bad_shift(kind, amount)),
        // 16-bit lanes: lsl #0 or lsl #8 only (there is no msl form here).
        ("4h" | "8h", None) => (0b1000, 0),
        ("4h" | "8h", Some(("lsl", 8))) => (0b1010, 0),
        ("4h" | "8h", Some((kind, amount))) => return Err(bad_shift(kind, amount)),
        // 32-bit lanes: lsl #0/8/16/24 and msl #8/#16.
        ("2s" | "4s", None) => (0b0000, 0),
        ("2s" | "4s", Some(("lsl", 8))) => (0b0010, 0),
        ("2s" | "4s", Some(("lsl", 16))) => (0b0100, 0),
        ("2s" | "4s", Some(("lsl", 24))) => (0b0110, 0),
        ("2s" | "4s", Some(("msl", 8))) => (0b1100, 0),
        ("2s" | "4s", Some(("msl", 16))) => (0b1101, 0),
        ("2s" | "4s", Some((kind, amount))) => return Err(bad_shift(kind, amount)),
        // 64-bit lanes: MOVI only, and the immediate is a byte mask.  Note the
        // cmode here is 1110 with op=1 -- an encoding MVNI never reaches.
        ("2d", None) => (0b1110, 1),
        ("2d", Some((kind, amount))) => return Err(bad_shift(kind, amount)),
        (other, _) => {
            return Err(format!(
                "{mn}: unsupported arrangement `v0.{other}` \
                 (expected 8b, 16b, 4h, 8h, 2s, 4s or 2d)"
            ));
        }
    };
    if is_mvni && matches!(arr, "8b" | "16b" | "2d") {
        return Err(format!(
            "{mn}: v0.{arr} has no MVNI encoding; invert the immediate and use \
             MOVI instead"
        ));
    }
    Ok((cmode, op, arr == "2d"))
}

/// Encode a MOVI/MVNI (vector, modified immediate).
///
/// Shared by both mnemonics because they are one encoding family whose only
/// difference is the `op` bit and the legal arrangement set -- the previous
/// two copies had already drifted (only one of them knew about `msl`, and only
/// one knew about 16-bit-lane shifts), which is how the same source text
/// assembled to two different constants depending on the spelling.
fn encode_neon_modified_imm(
    operands: &[Operand],
    mn: &str,
    is_mvni: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 || operands.len() > 3 {
        return Err(format!(
            "{mn} requires 2 or 3 operands (Vd.T, #imm[, shift]), got {}",
            operands.len()
        ));
    }
    let (rd, arr) = get_neon_reg(operands, 0)?;
    let imm = get_imm(operands, 1)?;

    // For the 8-bit and 16/32-bit lane forms the field holds the immediate
    // itself: 8 bits, spelled either unsigned or as a negative two's
    // complement value, and anything else is rejected rather than truncated.
    //
    // The 64-bit (2d) form is different in kind: there is no numeric
    // immediate, only eight one-bit "this byte is 0xFF" selectors, so its
    // operand is a 64-bit mask whose bytes must each be 0x00 or 0xFF.
    // Treating it as an 8-bit number rejected every real mask -- including
    // every mask whose low byte is not itself 0x00/0xFF-spelled-as-8-bit.
    let imm8 = if arr == "2d" && !is_mvni {
        let mut mask = 0u32;
        for i in 0..8 {
            match ((imm as u64) >> (i * 8)) & 0xFF {
                0x00 => {}
                0xFF => mask |= 1 << i,
                other => {
                    return Err(format!(
                        "{mn} v0.2d: each byte of the immediate must be 0x00 or \
                         0xFF, got 0x{other:02x} in byte {i}"
                    ));
                }
            }
        }
        mask
    } else {
        if !(-128..=255).contains(&imm) {
            return Err(format!(
                "{mn}: immediate {imm} does not fit the 8-bit modified-immediate \
                 field (allowed -128..=255)"
            ));
        }
        (imm & 0xFF) as u32
    };

    let shift = match operands.get(2) {
        None => None,
        Some(Operand::Shift { kind, amount }) => Some((kind.as_str(), *amount)),
        Some(other) => {
            return Err(format!(
                "{mn}: expected `lsl #n`/`msl #n` as the third operand, got {other:?}"
            ));
        }
    };

    let (cmode, layout_op, _byte_mask) = neon_modified_imm_layout(mn, &arr, shift, is_mvni)?;
    // MVNI is MOVI's inverse: same table, `op` = 1 in every encodable case.
    let op = if is_mvni { 1 } else { layout_op };

    let q: u32 = if matches!(arr.as_str(), "16b" | "8h" | "4s" | "2d") {
        1
    } else {
        0
    };
    // imm8 = abc:defgh, with abc in bits 18-16 and defgh in bits 9-5.
    let abc = (imm8 >> 5) & 0x7;
    let defgh = imm8 & 0x1F;

    let word = (q << 30)
        | (op << 29)
        | (0b1111 << 24)
        | (abc << 16)
        | (cmode << 12)
        | (0b01 << 10)
        | (defgh << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON MOVI (vector, modified immediate).
pub(crate) fn encode_neon_movi(operands: &[Operand]) -> Result<EncodeResult, String> {
    encode_neon_modified_imm(operands, "movi", false)
}

/// Encode NEON MVNI (vector, modified immediate): MOVI's inverse.
pub(crate) fn encode_neon_mvni(operands: &[Operand]) -> Result<EncodeResult, String> {
    encode_neon_modified_imm(operands, "mvni", true)
}

/// Encode NEON BIC (bitwise clear vector): BIC Vd.T, Vn.T, Vm.T
pub(crate) fn encode_neon_bic(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("bic requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;

    let q: u32 = if arr_d == "16b" { 1 } else { 0 };

    // BIC Vd.T, Vn.T, Vm.T: 0 Q 0 01110 01 1 Rm 000111 Rn Rd
    let word = (q << 30)
        | (0b001110 << 24)
        | (0b01 << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b000111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON BSL (bitwise select): BSL Vd.T, Vn.T, Vm.T
pub(crate) fn encode_neon_bsl(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("bsl requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;

    let q: u32 = if arr_d == "16b" { 1 } else { 0 };

    // BSL Vd.T, Vn.T, Vm.T: 0 Q 1 01110 01 1 Rm 000111 Rn Rd
    let word = (q << 30)
        | (1 << 29)
        | (0b01110 << 24)
        | (0b01 << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b000111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON REV64: reverse elements within 64-bit doublewords
pub(crate) fn encode_neon_rev64(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("rev64 requires 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;

    let (q, size) = neon_arr_to_q_size(&arr_d)?;

    // REV64 Vd.T, Vn.T: 0 Q 0 01110 size 10 0000 0000 10 Rn Rd
    let word = (q << 30)
        | (0b001110 << 24)
        | (size << 22)
        | (0b100000 << 16)
        | (0b000010 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Destination and index arrangement of a TBL/TBX: both are byte vectors of
/// the same width (`v0.16b, {table}, v2.16b`), and nothing else -- the
/// arrangement of the *table* is fixed at 16b and validated by the caller.
fn tbl_table_lookup_arrangements(operands: &[Operand], mn: &str) -> Result<(u32, String), String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    if !matches!(arr_d.as_str(), "8b" | "16b") {
        return Err(format!(
            "{mn}: destination `v.{arr_d}` is not a byte vector (expected .8b or .16b)"
        ));
    }
    Ok((rd, arr_d))
}

/// Encode TBL/TBX (table vector lookup).
///
/// ```text
/// 0 Q 00 1110 000 Rm 0 len 0 op Rn Rd
/// ```
///
/// `TBL` and `TBX` differ in one bit (`op`), so they are one function here:
/// they had drifted apart in the published version, and only the TBL copy
/// validated the table at all.
fn encode_neon_tbl_tbx(operands: &[Operand], mn: &str, op: u32) -> Result<EncodeResult, String> {
    if operands.len() != 3 {
        return Err(format!(
            "{mn} requires exactly 3 operands (Vd.T, {{Vn.T, ...}}, Vm.T), got {}",
            operands.len()
        ));
    }
    let (rd, arr_d) = tbl_table_lookup_arrangements(operands, mn)?;
    let q: u32 = if arr_d == "16b" { 1 } else { 0 };

    // The second operand is the table: one to four consecutive 16-byte
    // vectors.  The table registers are *always* 16B -- an 8B "table" is not
    // a table, it is the low half of the same bytes, and the architecture has
    // no encoding for it (GNU as rejects `tbl v0.8b,{{v1.8b}},v2.8b`), so the old
    // reader that took `regs[0]`'s arrangement and encoded Q=0 anyway produced
    // a word that reads a different table than the programmer wrote.
    let (rn, num_regs) = match &operands[1] {
        Operand::RegList(regs) => {
            if regs.is_empty() || regs.len() > 4 {
                return Err(format!(
                    "{mn}: the table must have 1 to 4 registers, got {}",
                    regs.len()
                ));
            }
            let mut first_reg = None;
            for (i, r) in regs.iter().enumerate() {
                let (reg, arr) = match r {
                    Operand::RegArrangement { reg, arrangement } => {
                        (reg.as_str(), arrangement.as_str())
                    }
                    Operand::Reg(name) => {
                        return Err(format!(
                            "{mn}: table element {i} (`{name}`) needs an arrangement \
                             (.16b); the table holds whole vectors"
                        ));
                    }
                    _ => return Err(format!("{mn}: expected register in list")),
                };
                if arr != "16b" {
                    return Err(format!(
                        "{mn}: table element {i} is `.{arr}`; every register in a \
                         TBL/TBX table must be .16b (the instruction selects bytes \
                         from one concatenated 16/32/48/64-byte table)"
                    ));
                }
                if !reg.starts_with('v') && !reg.starts_with('V') {
                    return Err(format!("{mn}: `{reg}` is not a vector register"));
                }
                let num = parse_reg_num(reg).ok_or("invalid reg")?;
                if first_reg.is_none() {
                    first_reg = Some(num);
                }
            }
            let first_reg = first_reg.ok_or_else(|| format!("{mn}: empty table"))?;
            validate_consecutive_reglist(regs, first_reg)?;
            (first_reg, regs.len() as u32)
        }
        _ => {
            return Err(format!(
                "{mn}: expected a register list as the second operand"
            ));
        }
    };

    let (rm, arr_m) = get_neon_reg(operands, 2)?;
    if arr_m != arr_d {
        return Err(format!(
            "{mn}: the index register is `.{arr_m}` but the destination is \
             `.{arr_d}`; both must have the same arrangement"
        ));
    }

    // len field: 1 reg -> 00, 2 -> 01, 3 -> 10, 4 -> 11
    let len = (num_regs - 1) & 0x3;

    // TBL/TBX: 0 Q 00 1110 000 Rm 0 len 0 op Rn Rd
    let word =
        (q << 30) | (0b001110 << 24) | (rm << 16) | (len << 13) | (op << 12) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON TBL (table vector lookup, out-of-range lanes read as zero).
pub(crate) fn encode_neon_tbl(operands: &[Operand]) -> Result<EncodeResult, String> {
    encode_neon_tbl_tbx(operands, "tbl", 0)
}

/// Encode NEON TBX (table vector lookup, out-of-range lanes keep their value).
/// Shares the TBL implementation: the two mnemonics differ in one bit, and the
/// published version's separate copy had already lost the table validation.
pub(crate) fn encode_neon_tbx(operands: &[Operand]) -> Result<EncodeResult, String> {
    encode_neon_tbl_tbx(operands, "tbx", 1)
}

/// Encode NEON LD1R (single register).  Shares the family implementation with
/// LD2R/LD3R/LD4R: the two used to be independent copies that disagreed about
/// the opcode field, the register list and the post-index forms.
pub(crate) fn encode_neon_ld1r(operands: &[Operand]) -> Result<EncodeResult, String> {
    encode_neon_ldnr(operands, 1)
}

/// Encode NEON LD1 (vector load, multiple structures)
/// Dispatch LD/ST1-4: choose between "multiple structures" and "single structure (element)" encoding.
pub(crate) fn encode_neon_ld_st_dispatch(
    operands: &[Operand],
    is_load: bool,
    num_structs: u32,
) -> Result<EncodeResult, String> {
    // If the first operand is a RegListIndexed, use single-element encoding
    if let Some(Operand::RegListIndexed { .. }) = operands.first() {
        return encode_neon_ld_st_single(operands, is_load, num_structs);
    }
    // Multiple-structures encoding for ld1-4/st1-4
    encode_neon_ld_st_multi(operands, is_load, num_structs)
}

/// Encode NEON LD/ST single structure (element):
/// st1 {v0.s}[0], [x3]
/// st2 {v0.s, v1.s}[0], [x3]
/// st4 {v0.s, v1.s, v2.s, v3.s}[0], [x3]
/// ld2 {v0.s, v1.s}[0], [x3]
// TODO: add post-index form [Xn], #imm
pub(crate) fn encode_neon_ld_st_single(
    operands: &[Operand],
    is_load: bool,
    num_structs: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err(format!(
            "ld/st{} single element requires at least 2 operands",
            num_structs
        ));
    }

    let (regs, index) = match &operands[0] {
        Operand::RegListIndexed { regs, index } => (regs, *index),
        _ => return Err("expected register list with index".to_string()),
    };

    if regs.len() as u32 != num_structs {
        return Err(format!(
            "expected {} registers in list, got {}",
            num_structs,
            regs.len()
        ));
    }

    // Get element size and first register from the list
    let (rt, elem_size) = match &regs[0] {
        Operand::RegArrangement { reg, arrangement } => (
            parse_reg_num(reg).ok_or("invalid register in list")?,
            arrangement.clone(),
        ),
        _ => return Err("expected register with arrangement in list".to_string()),
    };

    // The lane index is packed into the Q/S/size fields, so an index that does
    // not exist has to be rejected here -- masking it into range encodes a
    // different lane than the one that was written (`ld1 {v0.b}[16],[x1]`
    // assembled as lane 0).
    let max_lane = match elem_size.as_str() {
        "b" => 15u32,
        "h" => 7,
        "s" => 3,
        "d" => 1,
        other => {
            return Err(format!(
                "unsupported element size for ld/st single: {other}"
            ));
        }
    };
    if index > max_lane {
        return Err(format!(
            "ld/st{}: lane {index} does not exist; `.{elem_size}` has lanes 0..={max_lane}",
            num_structs
        ));
    }

    // ARM ISA: multi-register ld/st lists must be CONSECUTIVE (modulo 32,
    // i.e. {v31, v0} wraps legally). Only Rt is encoded; the hardware
    // derives the rest, so a non-consecutive list would silently encode a
    // DIFFERENT register set (GLM audit BUG-5). Reject like GNU as does.
    for (k, r) in regs.iter().enumerate().skip(1) {
        let rk = match r {
            Operand::RegArrangement { reg, .. } => {
                parse_reg_num(reg).ok_or("invalid register in list")?
            }
            _ => return Err("expected register with arrangement in list".to_string()),
        };
        let expected = (rt + k as u32) % 32;
        if rk != expected {
            return Err(format!(
                "register list must be consecutive: expected v{}, got v{}",
                expected, rk
            ));
        }
    }

    // Get base register and check for post-index
    let (rn, post_index) = match &operands[1] {
        Operand::Mem { base, offset: 0 } => {
            let rn =
                parse_reg_num(base).ok_or_else(|| format!("invalid base register: {}", base))?;
            // Check for post-index immediate: operands[2] is the post-index offset
            let pi = if operands.len() > 2 {
                match &operands[2] {
                    Operand::Imm(off) => Some(*off),
                    _ => None,
                }
            } else {
                None
            };
            (rn, pi)
        }
        Operand::MemPostIndex { base, offset } => {
            let rn =
                parse_reg_num(base).ok_or_else(|| format!("invalid base register: {}", base))?;
            (rn, Some(*offset))
        }
        _ => return Err("expected [Xn] memory operand".to_string()),
    };

    let l_bit = if is_load { 1u32 } else { 0u32 };

    // R bit: 0 for 1,3 registers; 1 for 2,4 registers
    let r_bit = match num_structs {
        1 | 3 => 0u32,
        2 | 4 => 1u32,
        _ => return Err(format!("unsupported struct count: {}", num_structs)),
    };

    // Compute opcode, S, Q, size based on element size and index
    let (opcode, s_bit, q_bit, size_field) = match elem_size.as_str() {
        "b" => {
            // opcode = 000 (1,2 regs) or 001 (3,4 regs)
            let base_opc = if num_structs <= 2 { 0b000u32 } else { 0b001u32 };
            // index bits: Q:S:size[1]:size[0] = 4 bits for 0-15
            let q = (index >> 3) & 1;
            let s = (index >> 2) & 1;
            let sz = index & 3;
            (base_opc, s, q, sz)
        }
        "h" => {
            let base_opc = if num_structs <= 2 { 0b010u32 } else { 0b011u32 };
            // index bits: Q:S:size[1] = 3 bits for 0-7, size[0]=0
            let q = (index >> 2) & 1;
            let s = (index >> 1) & 1;
            let sz = (index & 1) << 1;
            (base_opc, s, q, sz)
        }
        "s" => {
            let base_opc = if num_structs <= 2 { 0b100u32 } else { 0b101u32 };
            // index bits: Q:S = 2 bits for 0-3, size=00
            let q = (index >> 1) & 1;
            let s = index & 1;
            (base_opc, s, q, 0b00u32)
        }
        "d" => {
            let base_opc = if num_structs <= 2 { 0b100u32 } else { 0b101u32 };
            // index bits: Q = 1 bit for 0-1, S=0, size=01
            let q = index & 1;
            (base_opc, 0u32, q, 0b01u32)
        }
        _ => {
            return Err(format!(
                "unsupported element size for ld/st single: {}",
                elem_size
            ));
        }
    };

    if let Some(_offset) = post_index {
        // Post-index form: Q 0011011 L R 11111 opcode S size Rn Rt
        // (Rm=11111 means immediate post-index, the amount is implicit from element size)
        let word = (q_bit << 30)
            | (0b0011011 << 23)
            | (l_bit << 22)
            | (r_bit << 21)
            | (0b11111 << 16)
            | (opcode << 13)
            | (s_bit << 12)
            | (size_field << 10)
            | (rn << 5)
            | rt;
        Ok(EncodeResult::Word(word))
    } else {
        // No post-index: Q 0011010 L R 00000 opcode S size Rn Rt
        let word = (q_bit << 30)
            | (0b0011010 << 23)
            | (l_bit << 22)
            | (r_bit << 21)
            | (opcode << 13)
            | (s_bit << 12)
            | (size_field << 10)
            | (rn << 5)
            | rt;
        Ok(EncodeResult::Word(word))
    }
}

/// Common encoder for LD1/ST1 (multiple structures)
pub(crate) fn encode_neon_ld_st_multi(
    operands: &[Operand],
    is_load: bool,
    num_structs: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err(format!(
            "ld{}/st{} requires at least 2 operands",
            num_structs, num_structs
        ));
    }

    // First operand: register list {Vt.T} or {Vt.T, Vt+1.T, ...}
    let (rt, arr, num_regs) = match &operands[0] {
        Operand::RegList(regs) => {
            let (first_reg, arrangement) = match &regs[0] {
                Operand::RegArrangement { reg, arrangement } => (
                    parse_reg_num(reg).ok_or("invalid reg")?,
                    arrangement.clone(),
                ),
                _ => {
                    return Err(format!(
                        "ld{}/st{}: expected RegArrangement in list",
                        num_structs, num_structs
                    ));
                }
            };
            validate_consecutive_reglist(regs, first_reg)?;
            (first_reg, arrangement, regs.len() as u32)
        }
        _ => {
            return Err(format!(
                "ld{}/st{}: expected register list",
                num_structs, num_structs
            ));
        }
    };

    let (q, size) = neon_arr_to_q_size(&arr)?;

    // Second operand: [Xn] memory base or [Xn], #imm (post-index, merged by parser)
    let (rn, post_index) = match &operands[1] {
        Operand::Mem { base, offset: 0 } => {
            let r =
                parse_reg_num(base).ok_or_else(|| format!("invalid base register: {}", base))?;
            (r, None)
        }
        Operand::MemPostIndex { base, offset } => {
            let r =
                parse_reg_num(base).ok_or_else(|| format!("invalid base register: {}", base))?;
            (r, Some(*offset))
        }
        _ => {
            return Err(format!(
                "ld{}/st{}: expected [Xn] memory operand",
                num_structs, num_structs
            ));
        }
    };

    // opcode field based on structure count and number of registers:
    // LD1/ST1: 1 reg=0111, 2 reg=1010, 3 reg=0110, 4 reg=0010
    // LD2/ST2: 2 reg=1000
    // LD3/ST3: 3 reg=0100
    // LD4/ST4: 4 reg=0000
    let opcode = match num_structs {
        1 => match num_regs {
            1 => 0b0111u32,
            2 => 0b1010,
            3 => 0b0110,
            4 => 0b0010,
            _ => return Err(format!("ld1/st1: unsupported register count: {}", num_regs)),
        },
        2 => 0b1000u32,
        3 => 0b0100,
        4 => 0b0000,
        _ => return Err(format!("unsupported structure count: {}", num_structs)),
    };

    let l_bit = if is_load { 1u32 } else { 0u32 };

    // Handle post-index form from merged MemPostIndex
    if let Some(_imm) = post_index {
        // Post-index with immediate: use Rm=11111 (0x1F)
        let word = ((q << 30) | (0b001100 << 24) | (1 << 23) | (l_bit << 22))
            | (0b11111 << 16)
            | (opcode << 12)
            | (size << 10)
            | (rn << 5)
            | rt;
        return Ok(EncodeResult::Word(word));
    }

    // Check for post-index form via separate operands: [Xn], Xm
    if operands.len() > 2 {
        match &operands[2] {
            Operand::Imm(_) => {
                // Post-index with immediate: use Rm=11111
                let word = ((q << 30) | (0b001100 << 24) | (1 << 23) | (l_bit << 22))
                    | (0b11111 << 16)
                    | (opcode << 12)
                    | (size << 10)
                    | (rn << 5)
                    | rt;
                return Ok(EncodeResult::Word(word));
            }
            Operand::Reg(rm_name) => {
                let rm = parse_reg_num(rm_name).ok_or("invalid rm")?;
                let word = ((q << 30) | (0b001100 << 24) | (1 << 23) | (l_bit << 22))
                    | (rm << 16)
                    | (opcode << 12)
                    | (size << 10)
                    | (rn << 5)
                    | rt;
                return Ok(EncodeResult::Word(word));
            }
            _ => {}
        }
    }

    // No post-index: LD1/ST1 {Vt.T...}, [Xn]
    // 0 Q 001100 0 L 0 00000 opcode size Rn Rt
    let word = (((q << 30) | (0b001100 << 24)) | (l_bit << 22))
        | (opcode << 12)
        | (size << 10)
        | (rn << 5)
        | rt;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON UZP1/UZP2/ZIP1/ZIP2
pub(crate) fn encode_neon_zip_uzp(
    operands: &[Operand],
    op_bits: u32,
    _is_zip: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("uzp/zip requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;

    // UZP1: 0 Q 0 01110 size 0 Rm 0 001 10 Rn Rd  (op_bits=001)
    // UZP2: 0 Q 0 01110 size 0 Rm 0 101 10 Rn Rd  (op_bits=101)
    // ZIP1: 0 Q 0 01110 size 0 Rm 0 011 10 Rn Rd  (op_bits=011)
    // ZIP2: 0 Q 0 01110 size 0 Rm 0 111 10 Rn Rd  (op_bits=111)
    let word = (((q << 30) | (0b001110 << 24) | (size << 22)) | (rm << 16))
        | (op_bits << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON EOR3 (three-way XOR, SHA3 extension): EOR3 Vd.16b, Vn.16b, Vm.16b, Vk.16b
pub(crate) fn encode_neon_eor3(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 4 {
        return Err("eor3 requires 4 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (rk, _) = get_neon_reg(operands, 3)?;

    // EOR3 Vd.16b, Vn.16b, Vm.16b, Vk.16b
    // Encoding: 11001110 000 Rm 0 Rk(4:0) 00 Rn Rd
    let word = ((0b11001110u32 << 24) | (rm << 16)) | (rk << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON PMULL/PMULL2 (polynomial multiply long)
pub(crate) fn encode_neon_pmull(
    operands: &[Operand],
    is_pmull2: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("pmull requires 3 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;

    let q = if is_pmull2 { 1u32 } else { 0 };

    // PMULL  Vd.1q, Vn.1d, Vm.1d: 0 0 00 1110 11 1 Rm 11100 0 Rn Rd  (size=11)
    // PMULL2 Vd.1q, Vn.2d, Vm.2d: 0 1 00 1110 11 1 Rm 11100 0 Rn Rd
    let word =
        ((q << 30) | (0b001110 << 24) | (0b11 << 22) | (1 << 21) | (rm << 16) | (0b11100 << 11))
            | (rn << 5)
            | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON AES instructions (AESE, AESD, AESMC, AESIMC)
pub(crate) fn encode_neon_aes(operands: &[Operand], opcode: u32) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("aes instruction requires 2 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;

    // AES instructions: 0100 1110 0010 1000 opcode 10 Rn Rd
    // AESE:  opcode = 00100 (0x4)
    // AESD:  opcode = 00101 (0x5)
    // AESMC: opcode = 00110 (0x6)
    // AESIMC:opcode = 00111 (0x7)
    let word =
        (0b01001110 << 24) | (0b0010100 << 17) | (opcode << 12) | (0b10 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON ADD/SUB (vector integer): ADD/SUB Vd.T, Vn.T, Vm.T
pub(crate) fn encode_neon_add_sub(
    operands: &[Operand],
    is_sub: bool,
) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;
    let u = if is_sub { 1u32 } else { 0u32 };

    // ADD: 0 Q 0 01110 size 1 Rm 10000 1 Rn Rd
    // SUB: 0 Q 1 01110 size 1 Rm 10000 1 Rn Rd
    let word = (q << 30)
        | (u << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b10000 << 11)
        | (1 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON USHR (unsigned shift right immediate)
pub(crate) fn encode_neon_ushr(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("ushr requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;

    let (q, _) = neon_arr_to_q_size(&arr_d)?;

    // USHR Vd.T, Vn.T, #shift
    // 0 Q 1 0 11110 immh:immb 000001 Rn Rd
    let bits = arrangement_elem_bits(&arr_d)
        .ok_or_else(|| format!("ushr: unsupported arrangement `.{arr_d}`"))?;
    check_shift(shift, bits, true, "ushr")?;
    let immh_immb = 2 * bits - shift;

    let word = (q << 30)
        | (1 << 29)
        | (0b011110 << 23)
        | (immh_immb << 16)
        | (0b000001 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON SSHR (signed shift right immediate)
pub(crate) fn encode_neon_sshr(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("sshr requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;

    let (q, _) = neon_arr_to_q_size(&arr_d)?;

    // SSHR Vd.T, Vn.T, #shift
    // 0 Q 0 0 11110 immh:immb 000001 Rn Rd  (U=0)
    let bits = arrangement_elem_bits(&arr_d)
        .ok_or_else(|| format!("sshr: unsupported arrangement `.{arr_d}`"))?;
    check_shift(shift, bits, true, "sshr")?;
    let immh_immb = 2 * bits - shift;

    let word = (q << 30) | (0b011110 << 23) | (immh_immb << 16) | (0b000001 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON SHL (shift left immediate)
pub(crate) fn encode_neon_shl(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("shl requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;

    let (q, _) = neon_arr_to_q_size(&arr_d)?;

    // SHL Vd.T, Vn.T, #shift
    // 0 Q 0 0 11110 immh:immb 010101 Rn Rd
    // immh:immb = element_size + shift
    let bits = arrangement_elem_bits(&arr_d)
        .ok_or_else(|| format!("shl: unsupported arrangement `.{arr_d}`"))?;
    check_shift(shift, bits, false, "shl")?;
    let immh_immb = bits + shift;

    let word = (q << 30) | (0b011110 << 23) | (immh_immb << 16) | (0b010101 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON SLI (shift left and insert)
pub(crate) fn encode_neon_sli(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("sli requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;

    let (q, _) = neon_arr_to_q_size(&arr_d)?;

    // SLI Vd.T, Vn.T, #shift
    // 0 Q 1 0 11110 immh:immb 010101 Rn Rd  (U=1)
    let bits = arrangement_elem_bits(&arr_d)
        .ok_or_else(|| format!("sli: unsupported arrangement `.{arr_d}`"))?;
    check_shift(shift, bits, false, "sli")?;
    let immh_immb = bits + shift;

    let word = (q << 30)
        | (1 << 29)
        | (0b011110 << 23)
        | (immh_immb << 16)
        | (0b010101 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode SRI (Shift Right and Insert) immediate.
/// SRI Vd.T, Vn.T, #shift: 0 Q 1 0 11110 immh:immb 010001 Rn Rd  (U=1)
pub(crate) fn encode_neon_sri(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("sri requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;

    let (q, _) = neon_arr_to_q_size(&arr_d)?;

    // immh:immb = (2*esize - shift) for right shift
    let bits = arrangement_elem_bits(&arr_d)
        .ok_or_else(|| format!("sri: unsupported arrangement `.{arr_d}`"))?;
    check_shift(shift, bits, true, "sri")?;
    let immh_immb = 2 * bits - shift;

    let word = (q << 30)
        | (1 << 29)
        | (0b011110 << 23)
        | (immh_immb << 16)
        | (0b010001 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON RBIT (vector bit reverse) ───────────────────────────────────────

/// Encode NEON RBIT Vd.T, Vn.T (per-byte bit reversal in each element).
pub(crate) fn encode_neon_rbit(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("neon rbit requires 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;

    // Only .8b and .16b arrangements are valid for NEON RBIT
    if arr_d != "8b" && arr_d != "16b" {
        return Err(format!(
            "neon rbit: unsupported arrangement .{}, expected .8b or .16b",
            arr_d
        ));
    }
    let q: u32 = if arr_d == "16b" { 1 } else { 0 };
    // RBIT Vd.T, Vn.T: 0 Q 1 01110 01 10000 00101 10 Rn Rd
    let word = (q << 30)
        | (1 << 29)
        | (0b01110 << 24)
        | (0b01 << 22)
        | (0b10000 << 17)
        | (0b00101 << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON MVNI (move NOT immediate) ───────────────────────────────────────

// ── NEON float three-same ────────────────────────────────────────────────
/// Encode NEON float three-same: FADD, FSUB, FMUL, FDIV, FMLA, FMLS, etc.
/// Format: 0 Q U 01110 size 1 Rm opcode 1 Rn Rd
/// size[1]=size_hi (0 or 1), size[0]=sz (0=single, 1=double)
pub(crate) fn encode_neon_float_three_same(
    operands: &[Operand],
    u_bit: u32,
    size_hi: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (q, sz) = match arr_d.as_str() {
        "2s" => (0u32, 0u32),
        "4s" => (1, 0),
        "2d" => (1, 1),
        _ => {
            return Err(format!(
                "float three-same: unsupported arrangement: {}",
                arr_d
            ));
        }
    };
    let size = (size_hi << 1) | sz;
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (opcode << 11)
        | (1 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON two-register misc (integer) ─────────────────────────────────────
/// Encode NEON two-reg misc: ABS, NEG, CLS, CLZ, etc.
/// Format: 0 Q U 01110 size 10000 opcode 10 Rn Rd
pub(crate) fn encode_neon_two_misc(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON pairwise add long / accumulate ──────────────────────────────────
/// Encode SADDLP/UADDLP/SADALP/UADALP.
/// Format: 0 Q U 01110 size 10000 opcode 10 Rn Rd
/// Unlike most two-reg misc ops, the size field encodes the SOURCE (narrow)
/// element size and Q encodes the source register width — deriving size from
/// the destination (wide) arrangement produces a reserved encoding.
pub(crate) fn encode_neon_pairwise_long(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("NEON pairwise-long requires 2 operands".to_string());
    }
    let (rd, _arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let (q, size) = match arr_n.as_str() {
        "8b" => (0u32, 0b00u32),
        "16b" => (1, 0b00),
        "4h" => (0, 0b01),
        "8h" => (1, 0b01),
        "2s" => (0, 0b10),
        "4s" => (1, 0b10),
        _ => {
            return Err(format!(
                "unsupported source arrangement for pairwise long: {}",
                arr_n
            ));
        }
    };
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON float two-register misc ─────────────────────────────────────────
/// Encode NEON float two-reg misc: UCVTF, SCVTF, FCVTZS, FCVTZU, FNEG, FABS, etc. (vector)
/// Format: 0 Q U 01110 size 10000 opcode 10 Rn Rd
/// size[1]=size_hi, size[0]=sz (0=single, 1=double)
pub(crate) fn encode_neon_float_two_misc(
    operands: &[Operand],
    u_bit: u32,
    size_hi: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (q, sz) = match arr_d.as_str() {
        "2s" => (0u32, 0u32),
        "4s" => (1, 0),
        "2d" => (1, 1),
        _ => {
            return Err(format!(
                "float two-misc: unsupported arrangement: {}",
                arr_d
            ));
        }
    };
    let size = (size_hi << 1) | sz;
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Fixed-point variant of the Advanced-SIMD two-register-miscellaneous
/// conversions: `FCVTZS/U <Vd>.<T>, <Vn>.<T>, #<fbits>` and
/// `SCVTF/UCVTF <Vd>.<T>, <Vn>.<T>, #<fbits>`.
///
/// This is a *different encoding* from the integer form above, not a variant
/// of it:
///
///   0 Q U 0 1 1 1 1 0 immh:immb opcode 111111 Rn Rd   (FCVTZS/U, FP -> int)
///   0 Q U 0 1 1 1 1 0 immh:immb opcode 111001 Rn Rd   (SCVTF/UCVTF, int -> FP)
///
/// `immh:immb` is the usual Advanced-SIMD shift field, which for these
/// instructions encodes `2 * esize - fbits`. lccc routed every vector
/// conversion to the integer form and dropped the operand, so
/// `fcvtzs v20.2d,v12.2d,#13` assembled as `fcvtzs v20.2d,v12.2d` --
/// a conversion wrong by a factor of 2^13.
pub(crate) fn encode_neon_float_two_misc_fixed(
    operands: &[Operand],
    u: u32,
    to_int: bool,
) -> Result<EncodeResult, String> {
    let what = match (u, to_int) {
        (0, true) => "fcvtzs",
        (1, true) => "fcvtzu",
        (0, false) => "scvtf",
        (1, false) => "ucvtf",
        _ => unreachable!(),
    };
    let fbits = match operands.get(2) {
        Some(Operand::Imm(v)) => *v,
        other => {
            return Err(format!(
                "{what} (fixed-point): expected an immediate #fbits operand, \\
                 got {other:?}"
            ));
        }
    };
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;

    let (q, esize) = match arr_d.as_str() {
        "8h" => (1u32, 16i64),
        "4h" => (0, 16),
        "4s" => (1, 32),
        "2s" => (0, 32),
        "2d" => (1, 64),
        _ => {
            return Err(format!(
                "{what} (fixed-point): unsupported arrangement `{arr_d}` \\
                 (expected 4h, 8h, 2s, 4s or 2d)"
            ));
        }
    };
    if !(1..=esize).contains(&fbits) {
        return Err(format!(
            "{what}: fbits must be in 1..={esize} for an arrangement of \\
             {esize}-bit elements, got {fbits}"
        ));
    }

    // immh:immb == 2 * esize - fbits. The range check above guarantees immh is
    // one of the values that selects this element size and is not the reserved
    // 0000 pattern: fbits == esize gives immh == esize >> 3, the smallest legal
    // immh for the size, and fbits == 1 gives the largest.
    let immh_immb = (2 * esize - fbits) as u32;
    // 111111 for the FP-to-integer conversions, 111001 for integer-to-FP.
    let opcode: u32 = if to_int { 0b111111 } else { 0b111001 };

    let word = (q << 30)
        | (u << 29)
        | (0b01111 << 24)
        | (immh_immb << 16)
        | (opcode << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON shift right narrow (SHRN/RSHRN) ─────────────────────────────────
/// Format: 0 Q 0 01111 0 immh immb opcode 1 Rn Rd
/// SHRN opcode=10000, RSHRN opcode=10001
pub(crate) fn encode_neon_shrn(
    operands: &[Operand],
    opcode: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("shrn/rshrn requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;
    check_narrow_pair(&arr_d, &arr_n, is_high, "shrn")?;
    let element_bits = match arr_n.as_str() {
        "8h" => 16u32,
        "4s" => 32,
        "2d" => 64,
        _ => return Err(format!("shrn: unsupported source: {}", arr_n)),
    };
    let half_bits = element_bits / 2;
    if shift == 0 || shift > half_bits {
        return Err(format!("shrn: shift {} out of range", shift));
    }
    let immhb = element_bits - shift;
    let q = if is_high { 1u32 } else { 0 };
    let word = (q << 30)
        | (0b011110 << 23)
        | ((immhb >> 3) << 19)
        | ((immhb & 7) << 16)
        | (opcode << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON shift right accumulate (SSRA/USRA/SRSHR/URSHR) ─────────────────
/// Format: 0 Q U 01111 0 immh immb opcode 1 Rn Rd
pub(crate) fn encode_neon_shift_right(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("shift-right requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;
    let (q, _) = neon_arr_to_q_size(&arr_d)?;
    let element_bits: u32 = match arr_d.as_str() {
        "8b" | "16b" => 8,
        "4h" | "8h" => 16,
        "2s" | "4s" => 32,
        "2d" => 64,
        _ => return Err(format!("shift-right: unsupported: {}", arr_d)),
    };
    if shift == 0 || shift > element_bits {
        return Err(format!("shift {} out of range", shift));
    }
    let immhb = (element_bits * 2) - shift;
    let word = (q << 30)
        | (u_bit << 29)
        | (0b011110 << 23)
        | ((immhb >> 3) << 19)
        | ((immhb & 7) << 16)
        | (opcode << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON SSHLL/USHLL (shift left long) ───────────────────────────────────
/// Format: 0 Q U 011110 immh immb 10100 1 Rn Rd
pub(crate) fn encode_neon_shll(
    operands: &[Operand],
    u_bit: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("sshll/ushll requires 3 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;
    let base_val = match arr_n.as_str() {
        "8b" | "16b" => 8u32,
        "4h" | "8h" => 16,
        "2s" | "4s" => 32,
        _ => return Err(format!("sshll/ushll: unsupported source: {}", arr_n)),
    };
    if shift >= base_val {
        return Err(format!(
            "sshll/ushll: shift #{shift} is outside the encodable range \
             #0..#={} for `.{arr_n}`",
            base_val - 1
        ));
    }
    let immhb = base_val + shift;
    let q = if is_high { 1u32 } else { 0 };
    let word = (q << 30)
        | (u_bit << 29)
        | (0b011110 << 23)
        | ((immhb >> 3) << 19)
        | ((immhb & 7) << 16)
        | (0b101001 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode SHLL/SHLL2, the long shifts that fold the amount into the element
/// size.
///
/// These are *not* `ushll #esize`: the oracle encodes them in their own space
/// -- `shll v0.8h,v1.8b,#8` is 0x2E213820, whose fixed field is 0b01110 rather
/// than the long-shift 0b01111 -- and the two mnemonics pick the half of the
/// source register that is widened:
///
/// ```text
///          source   destination   size
/// shll      8b        8h          00
/// shll2    16b        8h          00
/// shll      4h        4s          01
/// shll2     8h        4s          01
/// shll      2s        2d          10
/// shll2     4s        2d          10
/// ```
///
/// The amount is not free: GNU as accepts only the element size (#8/#16/#32)
/// and rejects every other value, including `#0`.
pub(crate) fn encode_neon_shll_alias(
    operands: &[Operand],
    is_high: bool,
) -> Result<EncodeResult, String> {
    let mn = if is_high { "shll2" } else { "shll" };
    if operands.len() != 3 {
        return Err(format!("{mn} requires 3 operands, got {}", operands.len()));
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;
    let (size, elem_bits) = match (arr_d.as_str(), arr_n.as_str()) {
        ("8h", "8b") if !is_high => (0b00u32, 8u32),
        ("8h", "16b") if is_high => (0b00, 8),
        ("4s", "4h") if !is_high => (0b01, 16),
        ("4s", "8h") if is_high => (0b01, 16),
        ("2d", "2s") if !is_high => (0b10, 32),
        ("2d", "4s") if is_high => (0b10, 32),
        _ => {
            return Err(format!(
                "{mn}: `.{arr_d}` is not the destination of `{mn}` for a                  `.{arr_n}` source; the long shift widens one element step and                  {mn} reads the {} half of the source register",
                if is_high { "upper" } else { "lower" }
            ));
        }
    };
    if shift != elem_bits {
        return Err(format!(
            "{mn}: the amount is folded into the element size and is always              `#{elem_bits}`, not `#{shift}`"
        ));
    }
    let q = u32::from(is_high);
    let word = (q << 30)
        | (1 << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (1 << 16)
        | (0b001110 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON pairwise add (UADDLP/SADDLP/UADALP/SADALP) ────────────────────

// ── NEON three-different extras: UABAL/SABAL/ADDHN/RADDHN/SUBHN/RSUBHN ──
// Already have encode_neon_three_diff which handles these opcodes.

// ── NEON SQXTUN ──────────────────────────────────────────────────────────
// Two-reg misc with U=1, opcode=10010. Reuse encode_neon_two_misc_narrow.

// ── NEON shift right narrow saturating (SQSHRN/UQSHRN/SQRSHRN/UQRSHRN) ─
pub(crate) fn encode_neon_qshrn(
    operands: &[Operand],
    u_bit: u32,
    is_rounding: bool,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("qshrn requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;
    check_narrow_pair(&arr_d, &arr_n, is_high, "sqshrn")?;
    let element_bits = match arr_n.as_str() {
        "8h" => 16u32,
        "4s" => 32,
        "2d" => 64,
        _ => return Err(format!("qshrn: unsupported source: {}", arr_n)),
    };
    if shift == 0 || shift > element_bits / 2 {
        return Err(format!(
            "qshrn: shift {} out of range; the narrowing form shifts by 1..={} \
             for {}-bit sources",
            shift,
            element_bits / 2,
            element_bits
        ));
    }
    let immhb = element_bits - shift;
    let q = if is_high { 1u32 } else { 0 };
    let opcode_bits: u32 = if is_rounding { 0b100111 } else { 0b100101 };
    let word = (q << 30)
        | (u_bit << 29)
        | (0b011110 << 23)
        | ((immhb >> 3) << 19)
        | ((immhb & 7) << 16)
        | (opcode_bits << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON ADDHN/RADDHN/SUBHN/RSUBHN ──────────────────────────────────────
/// Three-different narrowing high: Format: 0 Q U 01110 size 1 Rm opcode 00 Rn Rd
pub(crate) fn encode_neon_three_diff_narrow(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("addhn/subhn requires 3 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let size = match arr_n.as_str() {
        "8h" => 0b00u32,
        "4s" => 0b01,
        "2d" => 0b10,
        _ => return Err(format!("addhn: unsupported source: {}", arr_n)),
    };
    let q = if is_high { 1u32 } else { 0 };
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (opcode << 12)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON LD2R/LD3R/LD4R ──────────────────────────────────────────────────
/// Register list + arrangement for the LD1R/LD2R/LD3R/LD4R family.
///
/// Every form in this family writes the same element into every lane of each
/// destination register, so all registers in the list must have the same
/// arrangement, the list must be consecutive, and its length must equal the
/// mnemonic's structure count.  The published version of this code read only
/// `regs[0]`'s arrangement and validated neither the width nor (for `ld1r`)
/// anything at all beyond the first element; `ld4r {{v0.8b,v1.16b,v2.8b}}` and
/// `ld2r {{v0.16b}}` therefore assembled to instructions touching registers
/// the programmer never named.
fn ld_replicate_reglist(operands: &[Operand], mn: &str, num_structs: u32) -> Result<u32, String> {
    let regs = match operands.first() {
        Some(Operand::RegList(regs)) => regs,
        _ => {
            return Err(format!(
                "{mn}: expected a register list as the first operand"
            ));
        }
    };
    if regs.len() != num_structs as usize {
        return Err(format!(
            "{mn}: expected {num_structs} register{} in the list, got {}",
            if num_structs == 1 { "" } else { "s" },
            regs.len()
        ));
    }
    let mut first_reg: Option<u32> = None;
    let mut arrangement: Option<&str> = None;
    for (i, r) in regs.iter().enumerate() {
        let (reg, arr) = match r {
            Operand::RegArrangement { reg, arrangement } => (reg, arrangement.as_str()),
            _ => {
                return Err(format!(
                    "{mn}: element {i} of the register list must carry an \
                     arrangement ({{v0.16b, ...}})"
                ));
            }
        };
        if !reg.starts_with('v') && !reg.starts_with('V') {
            return Err(format!(
                "{mn}: `{reg}` is not a vector register; this instruction loads \
                 whole vectors, so use v0.8b/v0.16b/4h/8h/2s/4s/1d/2d"
            ));
        }
        let num = parse_reg_num(reg).ok_or_else(|| format!("{mn}: invalid register `{reg}`"))?;
        match arrangement {
            None => {
                arrangement = Some(arr);
                first_reg = Some(num);
            }
            Some(first) if first != arr => {
                return Err(format!(
                    "{mn}: element {i} is `{arr}` but the first is `{first}`; every \
                     register in the list must have the same arrangement"
                ));
            }
            _ => {}
        }
    }
    let first = first_reg.ok_or_else(|| format!("{mn}: empty register list"))?;
    // `arrangement` is set in the same breath as `first_reg` above, so this is
    // unreachable rather than a live case -- but it is an encoder path an
    // attacker-shaped input reaches first, and the crate's rule is that a
    // rejected instruction is an `Err`, never a panic.
    let arr = arrangement.ok_or_else(|| format!("{mn}: empty register list"))?;
    if !matches!(arr, "8b" | "16b" | "4h" | "8h" | "2s" | "4s" | "1d" | "2d") {
        return Err(format!(
            "{mn}: unsupported arrangement `{arr}` \
             (expected 8b, 16b, 4h, 8h, 2s, 4s, 1d or 2d)"
        ));
    }
    Ok(first)
}

/// Arrangement → (Q, size) for LD1R...LD4R: 8B/16B=00, 4H/8H=01, 2S/4S=10, 1D/2D=11.
fn ld_replicate_q_size(arr: &str) -> (u32, u32) {
    match arr {
        "8b" => (0, 0b00),
        "16b" => (1, 0b00),
        "4h" => (0, 0b01),
        "8h" => (1, 0b01),
        "2s" => (0, 0b10),
        "4s" => (1, 0b10),
        "1d" => (0, 0b11),
        _ => (1, 0b11),
    }
}

/// Encode the LD1R/LD2R/LD3R/LD4R family (load one element, replicate it).
///
/// ```text
/// 0 Q 0 01101 W 1 S opcode size Rn Rt
/// ```
///
/// with the post-index kind in Rm and:
///
/// | mnemonic | opcode | S | post-index immediate |
/// |----------|--------|---|----------------------|
/// | ld1r     | 110    | 0 | 1 x esize            |
/// | ld2r     | 110    | 1 | 2 x esize            |
/// | ld3r     | 111    | 0 | 3 x esize            |
/// | ld4r     | 111    | 1 | 4 x esize            |
///
/// The immediate is not a free-form offset: the architecture encodes "advance
/// the base by the total number of bytes transferred", which is the element
/// size times the structure count, and nothing else.  The published version of
/// this function put `opcode`/`S` two bit positions too high and wrote a
/// hard-coded `110` for `ld1r` in a separate copy of the function, so `ld1r`
/// produced a *different* word (0x0D40C000, i.e. Q=0 bit patterns) than the
/// architecture's and `ld2r`/`ld4r` produced unallocated words
/// (0x4D40D000/0x4D40F000) that no hardware executes as a replicate load.
pub(crate) fn encode_neon_ldnr(
    operands: &[Operand],
    num_structs: u32,
) -> Result<EncodeResult, String> {
    let mn = format!("ld{num_structs}r");
    if operands.len() < 2 || operands.len() > 3 {
        return Err(format!(
            "{mn} requires 2 operands ({{Vt.T, ...}}, [Xn|SP]) plus an optional \
             post-index, got {}",
            operands.len()
        ));
    }
    let rt = ld_replicate_reglist(operands, &mn, num_structs)?;
    let arr = match &operands[0] {
        Operand::RegList(regs) => match &regs[0] {
            Operand::RegArrangement { arrangement, .. } => arrangement.clone(),
            _ => unreachable!("validated by ld_replicate_reglist"),
        },
        _ => unreachable!("validated by ld_replicate_reglist"),
    };
    let (q, size) = ld_replicate_q_size(&arr);
    let element_bytes = 1u32 << size;

    // The encoding names the *first* register; hardware derives the rest, and
    // they must be consecutive (checked in ld_replicate_reglist).
    let (opcode, s_bit) = match num_structs {
        1 => (0b110u32, 0u32),
        2 => (0b110, 1),
        3 => (0b111, 0),
        _ => (0b111, 1),
    };

    let (rn, writeback, rm) = match &operands[1] {
        Operand::Mem { base, offset } if *offset == 0 => {
            // `[Xn]` on its own, or `[Xn], Xm` where the parser leaves the
            // post-index register as a separate operand.
            (base_gp_reg(base, &mn)?, 0u32, 0u32)
        }
        Operand::Mem { base, offset } => {
            return Err(format!(
                "{mn}: `[{base}, #{offset}]` is a pre-indexed address, but this \
                 instruction has no pre-indexed form; write `[{base}], #<imm>` or \
                 `[{base}], Xm`"
            ));
        }
        Operand::MemPostIndex { base, offset } => {
            let total = (num_structs * element_bytes) as i64;
            if *offset != total {
                return Err(format!(
                    "{mn}: post-index #{offset} is not the {total} bytes this form \
                     transfers ({} x {element_bytes}-byte elements); the \
                     architecture only encodes `[{{Xn|SP}}], #{total}` or \
                     `[{{Xn|SP}}], Xm`",
                    num_structs,
                ));
            }
            (base_gp_reg(base, &mn)?, 1u32, 0b11111u32)
        }
        Operand::MemRegOffset { base, index, .. } => {
            // `[Xn, Xm]` is a pre-indexed register offset.  The replicate
            // family has no pre-indexed form at all, so this is a diagnostic
            // rather than a writeback -- `[Xn], Xm` is the form that exists.
            return Err(format!(
                "{mn}: `[{base}, {index}]` is a pre-indexed address, but this \
                 instruction only has post-indexed writeback; write `[{base}], \
                 {index}` instead"
            ));
        }
        other => {
            return Err(format!(
                "{mn}: expected a memory operand ([{{Xn|SP}}], [{{Xn|SP}}], #imm or \
                 [{{Xn|SP}}], Xm), got {other:?}"
            ));
        }
    };

    // Register post-index may also arrive as a third operand (the parser keeps
    // `[x0], x1` as a Mem plus a Reg rather than merging it).
    let (writeback, rm) = if matches!(operands[1], Operand::Mem { .. }) && operands.len() == 3 {
        let rm = post_index_reg(
            match &operands[2] {
                Operand::Reg(name) => name,
                other => {
                    return Err(format!(
                        "{mn}: expected a post-index register as the third operand, \
                         got {other:?}"
                    ));
                }
            },
            &mn,
        )?;
        (1u32, rm)
    } else {
        (writeback, rm)
    };

    let word = (q << 30)
        | (0b001101 << 24)
        | (writeback << 23)
        | (1 << 22)
        | (s_bit << 21)
        | (rm << 16)
        | (opcode << 13)
        | (size << 10)
        | ((rn & 0x1F) << 5)
        | rt;
    Ok(EncodeResult::Word(word))
}

/// Post-index register: a 64-bit general-purpose register.  `sp`/`wzr` cannot
/// be spelled here -- field 31 is the "no writeback"/immediate encoding, so
/// accepting them would silently turn `[x0], sp` into a zero writeback.
fn post_index_reg(name: &str, mn: &str) -> Result<u32, String> {
    let lower = name.to_lowercase();
    if lower == "sp" || lower == "wsp" || lower == "xzr" || lower == "wzr" {
        return Err(format!(
            "{mn}: `{name}` is not a valid post-index register (use x0-x30)"
        ));
    }
    if lower.starts_with('w') {
        return Err(format!(
            "{mn}: post-index register `{name}` must be 64-bit (x0-x30)"
        ));
    }
    let num =
        parse_reg_num(name).ok_or_else(|| format!("{mn}: invalid post-index register `{name}`"))?;
    if num > 30 {
        return Err(format!("{mn}: invalid post-index register `{name}`"));
    }
    Ok(num)
}

/// Resolve a base register operand of a memory access, returning its number.
/// `sp` is encodable in every base slot (`Rn` = 31); `wsp` is not, and neither
/// is any 32-bit register.
fn base_gp_reg(name: &str, mn: &str) -> Result<u32, String> {
    let lower = name.to_lowercase();
    if lower == "sp" {
        return Ok(31);
    }
    if lower == "wsp" {
        return Err(format!(
            "{mn}: base register `{name}` must be 64-bit; `wsp` cannot be a base"
        ));
    }
    if lower.starts_with('w') {
        return Err(format!(
            "{mn}: base register `{name}` must be 64-bit (x0-x30 or sp)"
        ));
    }
    let num = parse_reg_num(name).ok_or_else(|| format!("{mn}: invalid base register `{name}`"))?;
    if num > 31 {
        return Err(format!("{mn}: invalid base register `{name}`"));
    }
    Ok(num)
}

// ── NEON float compare-to-zero ───────────────────────────────────────────
/// FCMEQ/FCMLE/FCMLT/FCMGE/FCMGT to zero
/// Format: 0 Q U 01110 size 10000 opcode 10 Rn Rd (float, size = 0sz)
pub(crate) fn encode_neon_float_cmp_zero(
    operands: &[Operand],
    u_bit: u32,
    size_hi: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (q, sz) = match arr_d.as_str() {
        "2s" => (0u32, 0u32),
        "4s" => (1, 0),
        "2d" => (1, 1),
        _ => return Err(format!("float cmp zero: unsupported: {}", arr_d)),
    };
    let size = (size_hi << 1) | sz;
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON by-element (non-long) ───────────────────────────────────────────
/// MUL/MLA/MLS by element: 0 Q U 01111 size L M Rm opcode H 0 Rn Rd
/// `u` is bit 29, the U field of the Advanced-SIMD by-element group. It is
/// **1 only for MLA and MLS**; MUL, SQDMULH and SQRDMULH use 0. Passing 0 for
/// every caller assembled `mla v0.4s,v1.4s,v2.s[1]` as 0x4fa20020, where GAS
/// emits 0x6fa20020 -- and since MUL by element is opcode 1000 vs MLA's 0000,
/// the result was not merely a different instruction, it was a *multiply that
/// discards the accumulator*, i.e. a silently wrong answer.
pub(crate) fn encode_neon_elem(
    operands: &[Operand],
    u: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("NEON by-element requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, index) = match &operands[2] {
        Operand::RegLane { reg, index, .. } => (parse_reg_num(reg).ok_or("invalid reg")?, *index),
        _ => return Err(format!("expected register lane, got {:?}", operands[2])),
    };
    let (q, size) = neon_arr_to_q_size(&arr_d)?;
    let (h, l, m_bit) = match size {
        0b01 => ((index >> 2) & 1, (index >> 1) & 1, index & 1),
        0b10 => ((index >> 1) & 1, index & 1, (rm >> 4) & 1),
        _ => return Err("unsupported element size for by-element".to_string()),
    };
    let rm_enc = if size == 0b01 { rm & 0xF } else { rm & 0x1F };
    let word = (q << 30)
        | (u << 29)
        | (0b01111 << 24)
        | (size << 22)
        | (l << 21)
        | (m_bit << 20)
        | (rm_enc << 16)
        | (opcode << 12)
        | (h << 11)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON float by-element ────────────────────────────────────────────────
/// One operand of an FP by-element instruction: `(reg, element class, Q)`.
///
/// GNU as accepts both spellings for the two lowest registers of these forms
/// -- the scalar (`s0, s1`) and the 64-bit vector (`2s`) -- and they denote
/// the same encoding, with Q=0.  The published reader only understood
/// `Operand::RegArrangement`, so every scalar-spelled form (`fmul s0,s1,v2.s[0]`,
/// i.e. the spelling a compiler emits for scalar code) was rejected outright.
fn fp_elem_operand(
    operands: &[Operand],
    idx: usize,
    mn: &str,
) -> Result<(u32, u8, u32, bool), String> {
    let (reg, class, q, scalar) = match operands.get(idx) {
        Some(Operand::RegArrangement { reg, arrangement }) => {
            let (class, q) = match arrangement.as_str() {
                "2s" => (b's', 0u32),
                "4s" => (b's', 1),
                "4h" => (b'h', 0),
                "8h" => (b'h', 1),
                "1d" => (b'd', 0),
                "2d" => (b'd', 1),
                other => {
                    return Err(format!(
                        "{mn}: `v.{other}` is not an element arrangement for this \
                         instruction (expected 2s/4s, 4h/8h or 1d/2d)"
                    ));
                }
            };
            (reg.as_str(), class, q, false as bool)
        }
        Some(Operand::Reg(name)) => {
            let bytes = name.as_bytes();
            let class = if bytes.len() >= 2 {
                bytes[0].to_ascii_lowercase()
            } else {
                return Err(format!("{mn}: `{name}` is not an FP register"));
            };
            if !matches!(class, b'h' | b's' | b'd') {
                return Err(format!(
                    "{mn}: `{name}` is not a scalar FP register (expected h0-h31, \
                     s0-s31 or d0-d31)"
                ));
            }
            // A scalar spelling is its own encoding, not a synonym for the
            // narrow vector: the oracle's disassembler reads 0x5F829020 back
            // as `fmul s0,s1,v2.s[0]` and 0x4F829020 as `fmul v0.4s,v1.4s,v2.s[0]`.
            // They differ in bit 28 (scalar forms carry 0b11111 at bits 28-24,
            // vector forms 0b01111) -- and in nothing else that matters for the
            // low lane, which is why a reader that only understood vectors
            // "worked" until the two were compared byte for byte.
            (name.as_str(), class, 1u32, true)
        }
        other => {
            return Err(format!(
                "{mn}: expected an FP register at operand {idx}, got {other:?}"
            ));
        }
    };
    let num = parse_reg_num(reg).ok_or_else(|| format!("{mn}: invalid register `{reg}`"))?;
    Ok((num, class, q, scalar))
}

/// Encode the FP by-element family: FMUL, FMULX, FMLA, FMLS by element.
///
/// ```text
/// 0 Q U 01111 size L M Rm opcode H 0 Rn Rd
/// ```
///
/// The element's arrangement (`v2.s[3]`) is not decoration: it fixes the size
/// field, the index range and how the index bits are laid out, and the bit
/// order is not the one a reader would guess.  Writing `n` for the element
/// index, the oracle pins
///
/// ```text
/// 16-bit (.h): bit20 = n[0], bit21 = n[1], bit11 = n[2]   (Rm is 4 bits -> v0-v15)
/// 32-bit (.s): bit21 = n[0], bit11 = n[1]                 (Rm is 5 bits, bit20 = Rm[4])
/// 64-bit (.d): bit11 = n[0]                               (Rm is 5 bits, bit20 = Rm[4])
/// ```
///
/// i.e. the *high* index bit sits next to the opcode while the low one is up in
/// the size field.  The published version rotated the three bits (and the
/// version before this one still had H and L exchanged for 32-bit elements),
/// which is invisible for lanes 0 and 3 -- the only ones a bit-palindrome
/// exercise catches -- and wrong for every other lane.
///
/// The published version also took the *destination* arrangement for the size
/// field and never compared it with the element's, so
/// `fmla v0.4s,v1.4s,v2.d[1]` assembled as a valid-looking instruction.
pub(crate) fn encode_neon_float_elem(
    operands: &[Operand],
    opcode: u32,
    u: u32,
    mn: &str,
) -> Result<EncodeResult, String> {
    if operands.len() != 3 {
        return Err(format!(
            "{mn} requires exactly 3 operands (Rd, Rn, Vm.T[index]), got {}",
            operands.len()
        ));
    }
    let (rd, class_d, q_d, scalar_d) = fp_elem_operand(operands, 0, mn)?;
    let (rn, class_n, q_n, scalar_n) = fp_elem_operand(operands, 1, mn)?;
    let (rm, elem_class, index) = match &operands[2] {
        Operand::RegLane {
            reg,
            elem_size,
            index,
        } => {
            let class = match elem_size.as_str() {
                "h" => b'h',
                "s" => b's',
                "d" => b'd',
                other => {
                    return Err(format!(
                        "{mn}: element type `.{other}` is not encodable here \
                         (expected .h, .s or .d)"
                    ));
                }
            };
            (
                parse_reg_num(reg).ok_or_else(|| format!("{mn}: invalid register `{reg}`"))?,
                class,
                *index,
            )
        }
        other => {
            return Err(format!(
                "{mn}: expected an element operand like `v2.s[1]`, got {other:?}"
            ));
        }
    };

    if class_d != elem_class {
        return Err(format!(
            "{mn}: the element is `.{elem_class}` but the destination is a \
             `.{class_d}` register; the element's type must match the operands'"
        ));
    }
    if class_n != elem_class {
        return Err(format!(
            "{mn}: the element is `.{elem_class}` but the first source is a \
             `.{class_n}` register"
        ));
    }
    if q_d != q_n || scalar_d != scalar_n {
        return Err(format!(
            "{mn}: the destination and first source are different kinds of \
             register (a scalar spelling pairs with a scalar spelling, a vector \
             with a vector of the same width)"
        ));
    }

    // size: 16-bit = 00, 32-bit = 10, 64-bit = 11.  (0b01 is not a form of
    // this group; the 32-bit encodings use 0b10.)
    let size = match elem_class {
        b'h' => 0b00u32,
        b's' => 0b10,
        _ => 0b11,
    };

    let (max_index, l, m_bit, h, rm_enc) = match elem_class {
        b'h' => {
            if rm > 15 {
                return Err(format!(
                    "{mn}: `v{rm}.h[…]` is not encodable; the 16-bit by-element \
                     forms take a table register in v0-v15"
                ));
            }
            (7u32, (index >> 1) & 1, index & 1, (index >> 2) & 1, rm)
        }
        b's' => (3, index & 1, 0u32, (index >> 1) & 1, rm),
        _ => (1, 0u32, 0u32, index & 1, rm),
    };
    if index > max_index {
        return Err(format!(
            "{mn}: lane {index} is outside `v{rm}.{elem_class}[0..={max_index}]`"
        ));
    }
    // bits 28-24 carry 0b01111 for the vector forms and 0b11111 for the scalar
    // ones; the difference is exactly bit 28, which is why every scalar RTL
    // spelling was one bit off until the two spellings were compared.

    let scalar_flag = u32::from(scalar_d);
    let word = (q_d << 30)
        | (u << 29)
        | (scalar_flag << 28)
        | (0b01111 << 24)
        | (size << 22)
        | (l << 21)
        | (m_bit << 20)
        | (rm_enc << 16)
        | (opcode << 12)
        | (h << 11)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON FCVTL/FCVTN ────────────────────────────────────────────────────
/// FCVTL: half→single or single→double widening float convert
/// Format: 0 Q 0 01110 0 sz 10000 10111 10 Rn Rd
pub(crate) fn encode_neon_fcvtl(
    operands: &[Operand],
    is_high: bool,
) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let sz = match arr_d.as_str() {
        "4s" | "2s" => 0u32,
        "2d" => 1,
        _ => return Err(format!("fcvtl: unsupported dest: {}", arr_d)),
    };
    let q = if is_high { 1u32 } else { 0 };
    let word = (q << 30)
        | (0b01110 << 24)
        | (sz << 22)
        | (0b10000 << 17)
        | (0b10111 << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// FCVTN: single→half or double→single narrowing float convert
pub(crate) fn encode_neon_fcvtn(
    operands: &[Operand],
    is_high: bool,
) -> Result<EncodeResult, String> {
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let sz = match arr_n.as_str() {
        "4s" | "2s" => 0u32,
        "2d" => 1,
        _ => return Err(format!("fcvtn: unsupported source: {}", arr_n)),
    };
    let q = if is_high { 1u32 } else { 0 };
    let word = (q << 30)
        | (0b01110 << 24)
        | (sz << 22)
        | (0b10000 << 17)
        | (0b10110 << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── BIT/BIF (bitwise insert if true/false) ──────────────────────────────
/// Encodes BIT (size=10) and BIF (size=11) instructions.
/// Same format as BSL but with different size field.
/// Format: 0 Q 1 01110 ss 1 Rm 000111 Rn Rd
pub(crate) fn encode_neon_bitwise_insert(
    operands: &[Operand],
    size: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("bit/bif requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let q: u32 = if arr_d == "16b" { 1 } else { 0 };
    let word = (q << 30)
        | (1 << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b000111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── FADDP (float pairwise add) ──────────────────────────────────────────
/// FADDP — float add pairwise
/// Vector form: FADDP Vd.T, Vn.T, Vm.T
///   Format: 0 Q 1 01110 0 sz 1 Rm 110101 Rn Rd
/// Scalar form: FADDP Sd, Vn.2S  or FADDP Dd, Vn.2D
///   Format: 01 1 11110 0 sz 11000 01101 10 Rn Rd
pub(crate) fn encode_neon_faddp(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() >= 3 {
        // Vector form: 3 operands
        let (rd, arr_d) = get_neon_reg(operands, 0)?;
        let (rn, _) = get_neon_reg(operands, 1)?;
        let (rm, _) = get_neon_reg(operands, 2)?;
        let (q, sz) = match arr_d.as_str() {
            "2s" => (0u32, 0u32),
            "4s" => (1, 0),
            "2d" => (1, 1),
            _ => return Err(format!("faddp: unsupported arrangement: {}", arr_d)),
        };
        let word = (q << 30)
            | (1 << 29)
            | (0b01110 << 24)
            | (sz << 22)
            | (1 << 21)
            | (rm << 16)
            | (0b110101 << 10)
            | (rn << 5)
            | rd;
        Ok(EncodeResult::Word(word))
    } else if operands.len() == 2 {
        // Scalar form: FADDP Sd, Vn.2S or FADDP Dd, Vn.2D
        let rd = match &operands[0] {
            Operand::Reg(r) => parse_reg_num(r).ok_or("invalid dest reg")?,
            _ => return Err("faddp scalar: expected register".to_string()),
        };
        let (rn, arr_n) = get_neon_reg(operands, 1)?;
        let sz = match arr_n.as_str() {
            "2s" => 0u32,
            "2d" => 1,
            _ => return Err(format!("faddp scalar: unsupported source: {}", arr_n)),
        };
        // 01 1 11110 0 sz 11000 01101 10 Rn Rd
        let word = (0b01 << 30)
            | (1 << 29)
            | (0b11110 << 24)
            | (sz << 22)
            | (0b11000 << 17)
            | (0b01101 << 12)
            | (0b10 << 10)
            | (rn << 5)
            | rd;
        Ok(EncodeResult::Word(word))
    } else {
        Err("faddp requires 2 or 3 operands".to_string())
    }
}

// ── SADDLV/UADDLV (signed/unsigned add long across vector) ─────────────
/// Format: 0 Q U 01110 size 11000 00011 10 Rn Rd
pub(crate) fn encode_neon_across_long(
    operands: &[Operand],
    u: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("saddlv/uaddlv requires 2 operands".to_string());
    }
    // Destination is a scalar register (e.g., s16), source is a vector arrangement
    let rd = match &operands[0] {
        Operand::Reg(r) => parse_reg_num(r).ok_or("invalid dest reg")?,
        Operand::RegArrangement { reg, .. } => parse_reg_num(reg).ok_or("invalid dest reg")?,
        _ => return Err("saddlv: expected register".to_string()),
    };
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let (q, size) = neon_arr_to_q_size(&arr_n)?;
    let word = (q << 30)
        | (u << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b11000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON shift left by immediate (SQSHL, UQSHL, SHL, etc.) ─────────────
/// Format: 0 Q U 011110 immh:immb opcode 1 Rn Rd
/// immh:immb encodes both the element size and the shift amount.
pub(crate) fn encode_neon_shift_left_imm(
    operands: &[Operand],
    u: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("shift left immediate requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;

    let (q, _immh_base, esize) = match arr_d.as_str() {
        "8b" => (0u32, 0b0001u32, 8u32),
        "16b" => (1, 0b0001, 8),
        "4h" => (0, 0b0010, 16),
        "8h" => (1, 0b0010, 16),
        "2s" => (0, 0b0100, 32),
        "4s" => (1, 0b0100, 32),
        "2d" => (1, 0b1000, 64),
        _ => {
            return Err(format!(
                "shift left imm: unsupported arrangement: {}",
                arr_d
            ));
        }
    };

    // immh:immb = esize + shift_amount
    // For 8-bit: immh=0001, shift in 0..7 => immh:immb = 8 + shift
    // For 16-bit: immh=001x, shift in 0..15 => immh:immb = 16 + shift
    // For 32-bit: immh=01xx, shift in 0..31 => immh:immb = 32 + shift
    // For 64-bit: immh=1xxx, shift in 0..63 => immh:immb = 64 + shift
    let immhb = esize + shift;
    let immh = (immhb >> 3) & 0xF;
    let immb = immhb & 0x7;

    let word = (q << 30)
        | (u << 29)
        | (0b011110 << 23)
        | (immh << 19)
        | (immb << 16)
        | (opcode << 11)
        | (1 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── Helper: detect scalar d-register 3-operand NEON operations ──────────────
/// Whether an `add`/`sub` operand list is the NEON *scalar* three-same form
/// (`add d0, d1, d2`) rather than the general-purpose one.
///
/// The test names both grammars, not one operand's first letter.  Asking only
/// "does operand 0 start with `d`" sent `add d0, x1, x2` to the NEON encoder,
/// which read all three operands with the permissive number parser and
/// assembled it -- an instruction with no encoding, whose operands come from
/// two different register files.  A scalar three-same form has one `size`
/// field, so all three registers are the same width as well: `add d0, s1, s2`
/// is not a form either.
pub(crate) fn is_neon_scalar_d_reg_op(operands: &[Operand]) -> bool {
    if operands.len() != 3 {
        return false;
    }
    let mut letter = 0u8;
    for i in 0..operands.len() {
        if !matches!(operands[i], Operand::Reg(_)) {
            return false;
        }
        match fp_reg(operands, i, "add/sub") {
            Ok((_, l, _)) if matches!(l, b'b' | b'h' | b's' | b'd') => {
                if i == 0 {
                    letter = l;
                } else if l != letter {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

// ── NEON scalar three-same: ADD/SUB Dd, Dn, Dm ────────────────────────────
/// Encode scalar NEON three-same: 01 U 11110 size 1 Rm opcode 1 Rn Rd
pub(crate) fn encode_neon_scalar_three_same(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
    size: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("scalar three-same requires 3 operands".to_string());
    }
    // All three operands are floating-point registers of one width: the form
    // has a single `size` field, and the reader says so instead of trusting a
    // per-operand number.  `add d0, x1, x2` (checked by the dispatcher) and
    // `add d0, s1, s2` (checked here) are both refused.
    let (rd, _, _) = fp_reg(operands, 0, "scalar three-same")?;
    let (rn, _, _) = fp_reg(operands, 1, "scalar three-same")?;
    let (rm, _, _) = fp_reg(operands, 2, "scalar three-same")?;
    let word = (0b01 << 30)
        | (u_bit << 29)
        | (0b11110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (opcode << 11)
        | (1 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON scalar ADDP: addp Dd, Vn.2d ──────────────────────────────────────
pub(crate) fn encode_neon_scalar_addp(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("scalar addp requires 2 operands".to_string());
    }
    let rd = match &operands[0] {
        Operand::Reg(r) => parse_reg_num(r).ok_or("invalid reg")?,
        _ => return Err("expected d register".to_string()),
    };
    let rn = match &operands[1] {
        Operand::RegArrangement { reg, arrangement } => {
            if arrangement != "2d" {
                return Err(format!(
                    "scalar addp requires .2d source, got .{}",
                    arrangement
                ));
            }
            parse_reg_num(reg).ok_or("invalid reg")?
        }
        _ => return Err("scalar addp: expected Vn.2d source".to_string()),
    };
    // Scalar ADDP: 01 0 11110 11 11000 11011 10 Rn Rd
    let word = (0b01 << 30)
        | (0b011110 << 24)
        | (0b11 << 22)
        | (0b11000 << 17)
        | (0b11011 << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON scalar two-reg misc: SQABS/SQNEG Hd,Hn / Sd,Sn / Dd,Dn ──────────
pub(crate) fn encode_neon_scalar_two_misc(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("scalar two-misc requires 2 operands".to_string());
    }
    let (rd, rd_name) = match &operands[0] {
        Operand::Reg(r) => (parse_reg_num(r).ok_or("invalid reg")?, r.to_lowercase()),
        _ => return Err("expected register".to_string()),
    };
    let rn = match &operands[1] {
        Operand::Reg(r) => parse_reg_num(r).ok_or("invalid reg")?,
        _ => return Err("expected register".to_string()),
    };
    let size = if rd_name.starts_with('b') {
        0b00u32
    } else if rd_name.starts_with('h') {
        0b01
    } else if rd_name.starts_with('s') {
        0b10
    } else if rd_name.starts_with('d') {
        0b11
    } else {
        return Err(format!(
            "scalar two-misc: unsupported register type: {}",
            rd_name
        ));
    };
    // 01 U 11110 size 10000 opcode 10 Rn Rd
    let word = (0b01 << 30)
        | (u_bit << 29)
        | (0b11110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON scalar SQSHRN: sqshrn Hd,Sn,#shift / sqshrn Sd,Dn,#shift ────────
pub(crate) fn encode_neon_scalar_qshrn(
    operands: &[Operand],
    u_bit: u32,
    is_rounding: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("scalar qshrn requires 3 operands".to_string());
    }
    let (rd, rd_name) = match &operands[0] {
        Operand::Reg(r) => (parse_reg_num(r).ok_or("invalid reg")?, r.to_lowercase()),
        _ => return Err("expected register".to_string()),
    };
    let rn = match &operands[1] {
        Operand::Reg(r) => parse_reg_num(r).ok_or("invalid reg")?,
        _ => return Err("expected register".to_string()),
    };
    let shift = get_imm(operands, 2)? as u32;
    // Determine element bits from destination register type
    let element_bits = if rd_name.starts_with('b') {
        8u32
    }
    // b <- h (narrow from 16-bit)
    else if rd_name.starts_with('h') {
        16
    }
    // h <- s (narrow from 32-bit), immh base = 16
    else if rd_name.starts_with('s') {
        32
    }
    // s <- d (narrow from 64-bit), immh base = 32
    else {
        return Err(format!("scalar qshrn: unsupported dest: {}", rd_name));
    };
    if shift == 0 || shift > element_bits {
        return Err(format!("scalar qshrn: shift {} out of range", shift));
    }
    let immhb = (element_bits * 2) - shift; // source element bits - shift
    let opcode_bits: u32 = if is_rounding { 0b100111 } else { 0b100101 };
    // 01 U 11110 immh:immb opcode 1 Rn Rd
    let word = (0b01 << 30)
        | (u_bit << 29)
        | (0b011110 << 23)
        | ((immhb >> 3) << 19)
        | ((immhb & 7) << 16)
        | (opcode_bits << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON addp (integer pairwise add) — already handled in three-same as addp ──
