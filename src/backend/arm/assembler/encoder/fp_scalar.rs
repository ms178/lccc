use super::*;
use crate::backend::arm::assembler::parser::Operand;

// ── Floating point ───────────────────────────────────────────────────────

pub fn encode_fmov(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("fmov requires 2 operands".to_string());
    }

    let (rd_name, rm_name) = match (&operands[0], &operands[1]) {
        (Operand::Reg(a), Operand::Reg(b)) => (a.clone(), b.clone()),
        (Operand::Reg(_a), Operand::Imm(_)) => {
            // TODO: implement fmov with float immediate encoding
            return Err("fmov with immediate operand not yet supported".to_string());
        }
        _ => return Err("fmov needs register operands".to_string()),
    };

    let rd = parse_reg_num(&rd_name).ok_or("invalid rd")?;
    let rm = parse_reg_num(&rm_name).ok_or("invalid rm")?;

    let rd_is_fp = is_fp_reg(&rd_name);
    let rm_is_fp = is_fp_reg(&rm_name);
    let rd_lower = rd_name.to_lowercase();
    let rm_lower = rm_name.to_lowercase();

    if rd_is_fp && rm_is_fp {
        // FMOV between FP registers
        // Prefer the destination; fall back to Rm when Rd is not an FP reg.
        let ftype = if rd_lower.starts_with('h') || rm_lower.starts_with('h') {
            0b11
        } else if rd_lower.starts_with('d') || rm_lower.starts_with('d') {
            0b01
        } else {
            fp_ftype(&rd_lower).or_else(|_| fp_ftype(&rm_lower))?
        };
        // 0 00 11110 ftype 1 0000 00 10000 Rn Rd
        let word = (0b00011110 << 24)
            | (ftype << 22)
            | (0b100000 << 16)
            | (0b10000 << 10)
            | (rm << 5)
            | rd;
        return Ok(EncodeResult::Word(word));
    }

    if rd_is_fp && !rm_is_fp {
        // FMOV from GP to FP: FMOV Dn, Xn or FMOV Sn, Wn
        let is_double = rd_lower.starts_with('d');
        if is_double {
            // FMOV Dd, Xn: 1 00 11110 01 1 00 111 000000 Rn Rd
            let word = ((0b1001111001 << 22) | (0b100111 << 16)) | (rm << 5) | rd;
            return Ok(EncodeResult::Word(word));
        } else {
            // FMOV Sd, Wn: 0 00 11110 00 1 00 111 000000 Rn Rd
            let word = ((0b0001111000 << 22) | (0b100111 << 16)) | (rm << 5) | rd;
            return Ok(EncodeResult::Word(word));
        }
    }

    if !rd_is_fp && rm_is_fp {
        // FMOV from FP to GP: FMOV Xn, Dn or FMOV Wn, Sn
        let is_double = rm_lower.starts_with('d');
        if is_double {
            // FMOV Xd, Dn: 1 00 11110 01 1 00 110 000000 Rn Rd
            let word = ((0b1001111001 << 22) | (0b100110 << 16)) | (rm << 5) | rd;
            return Ok(EncodeResult::Word(word));
        } else {
            // FMOV Wd, Sn: 0 00 11110 00 1 00 110 000000 Rn Rd
            let word = ((0b0001111000 << 22) | (0b100110 << 16)) | (rm << 5) | rd;
            return Ok(EncodeResult::Word(word));
        }
    }

    Err(format!(
        "unsupported fmov operands: {} -> {}",
        rd_name, rm_name
    ))
}

pub fn encode_fp_arith(operands: &[Operand], opcode: u32) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;

    let rd_name = match &operands[0] {
        Operand::Reg(r) => r.to_lowercase(),
        _ => String::new(),
    };
    let ftype = fp_ftype(&rd_name)?;

    // 0 00 11110 ftype 1 Rm opcode 10 Rn Rd
    let word = (0b00011110 << 24)
        | (ftype << 22)
        | (1 << 21)
        | (rm << 16)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_fneg(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let rd_name = match &operands[0] {
        Operand::Reg(r) => r.to_lowercase(),
        _ => String::new(),
    };
    let ftype = fp_ftype(&rd_name)?;
    // FNEG: 0 00 11110 ftype 1 0000 10 10000 Rn Rd
    let word =
        (0b00011110 << 24) | (ftype << 22) | (0b100001 << 16) | (0b10000 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_fabs(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let rd_name = match &operands[0] {
        Operand::Reg(r) => r.to_lowercase(),
        _ => String::new(),
    };
    let ftype = fp_ftype(&rd_name)?;
    // FABS: 0 00 11110 ftype 1 0000 01 10000 Rn Rd
    let word =
        (0b00011110 << 24) | (ftype << 22) | (0b100000 << 16) | (0b110000 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_fsqrt(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let rd_name = match &operands[0] {
        Operand::Reg(r) => r.to_lowercase(),
        _ => String::new(),
    };
    let ftype = fp_ftype(&rd_name)?;
    // FSQRT: 0 00 11110 ftype 1 0000 11 10000 Rn Rd
    let word =
        (0b00011110 << 24) | (ftype << 22) | (0b100001 << 16) | (0b110000 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode FP 1-source ops: FRINTN/P/M/Z/A/X/I
/// Format: 0 00 11110 ftype 1 opcode 10000 Rn Rd
pub fn encode_fp_1src(operands: &[Operand], opcode: u32) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let rd_name = match &operands[0] {
        Operand::Reg(r) => r.to_lowercase(),
        _ => String::new(),
    };
    let ftype = fp_ftype(&rd_name)?;
    let word = (0b00011110u32 << 24)
        | (ftype << 22)
        | (1 << 21)
        | (opcode << 15)
        | (0b10000 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// The scalar-FP `type` field, from a register name.
///
/// ```text
///   S (single, 32-bit)  -> 0b00
///   D (double, 64-bit)  -> 0b01
///   H (half,   16-bit)  -> 0b11
/// ```
///
/// Every call site used to ask only "does the name start with `d`?", which is a
/// two-way question with a three-way answer: half-precision registers fell into
/// the single-precision branch, so `fmadd h11, h3, h2, h12` assembled as
/// `fmadd s11, s3, s2, s12`. Nothing rejected it and nothing warned -- the
/// instruction simply operated on the wrong half of the register.
pub(crate) fn fp_ftype(name: &str) -> Result<u32, String> {
    match name.to_lowercase().chars().next() {
        Some('s') => Ok(0b00),
        Some('d') => Ok(0b01),
        Some('h') => Ok(0b11),
        _ => Err(format!(
            "unsupported floating-point register `{name}` \
             (expected h, s or d for this instruction)"
        )),
    }
}

/// Encode FMADD/FMSUB: Rd = Ra +/- (Rn * Rm)
/// Format: 0 00 11111 ftype 0 Rm o1 Ra Rn Rd
pub fn encode_fmadd_fmsub(operands: &[Operand], is_sub: bool) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    let (ra, _) = get_reg(operands, 3)?;
    let rd_name = match &operands[0] {
        Operand::Reg(r) => r.to_lowercase(),
        _ => String::new(),
    };
    let ftype = fp_ftype(&rd_name)?;
    let o1 = if is_sub { 1u32 } else { 0 };
    let word = (0b00011111u32 << 24)
        | (ftype << 22)
        | (rm << 16)
        | (o1 << 15)
        | (ra << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode FNMADD/FNMSUB: Rd = -Ra +/- (Rn * Rm)
/// Format: 0 00 11111 ftype 1 Rm o1 Ra Rn Rd
pub fn encode_fnmadd_fnmsub(operands: &[Operand], is_sub: bool) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    let (ra, _) = get_reg(operands, 3)?;
    let rd_name = match &operands[0] {
        Operand::Reg(r) => r.to_lowercase(),
        _ => String::new(),
    };
    let ftype = fp_ftype(&rd_name)?;
    let o1 = if is_sub { 1u32 } else { 0 };
    let word = (0b00011111u32 << 24)
        | (ftype << 22)
        | (1 << 21)
        | (rm << 16)
        | (o1 << 15)
        | (ra << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_fcmp(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rn, _) = get_reg(operands, 0)?;
    let rn_name = match &operands[0] {
        Operand::Reg(r) => r.to_lowercase(),
        _ => String::new(),
    };
    let ftype = fp_ftype(&rn_name)?;

    // FCMP Dn, #0.0
    if operands.len() < 2 || matches!(operands.get(1), Some(Operand::Imm(0))) {
        let word = ((0b00011110 << 24) | (ftype << 22) | (1 << 21))
            | (0b001000 << 10)
            | (rn << 5)
            | 0b01000;
        return Ok(EncodeResult::Word(word));
    }

    let (rm, _) = get_reg(operands, 1)?;
    // FCMP Dn, Dm: 0 00 11110 ftype 1 Rm 00 1000 Rn 00 000
    let word =
        (0b00011110 << 24) | (ftype << 22) | (1 << 21) | (rm << 16) | (0b001000 << 10) | (rn << 5);
    Ok(EncodeResult::Word(word))
}

pub fn encode_fcvt_rounding(
    operands: &[Operand],
    rmode: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    // Float-to-integer conversion with specified rounding mode
    // Encoding: sf 00 11110 ftype 1 rmode opcode 000000 Rn Rd
    // sf: 0=W dest, 1=X dest
    // ftype: 00=S source, 01=D source
    // rmode+opcode: determines rounding mode and signedness
    if operands.len() < 2 {
        return Err("fcvt* requires 2 operands".to_string());
    }
    let (rd, rd_is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;

    let src_name = match &operands[1] {
        Operand::Reg(name) => name.to_lowercase(),
        _ => return Err("fcvt*: expected register source".to_string()),
    };
    // ftype: 00=S source, 01=D source, 11=H source.
    // The `h` case was missing, so `fcvtzu w4, h17, #13` converted a
    // single-precision value it did not have instead of the half-precision one
    // it was given.
    let ftype = fp_ftype(&src_name)?;
    let sf: u32 = if rd_is_64 { 1 } else { 0 };

    let word =
        ((sf << 31) | (0b11110 << 24) | (ftype << 22) | (1 << 21) | (rmode << 19) | (opcode << 16))
            | (rn << 5)
            | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_ucvtf(operands: &[Operand]) -> Result<EncodeResult, String> {
    encode_int_to_float(operands, false)
}

pub fn encode_scvtf(operands: &[Operand]) -> Result<EncodeResult, String> {
    encode_int_to_float(operands, true)
}

pub fn encode_int_to_float(operands: &[Operand], is_signed: bool) -> Result<EncodeResult, String> {
    // SCVTF/UCVTF: integer-to-float conversion
    // Encoding: sf 00 11110 ftype 1 00 opcode 000000 Rn Rd
    // sf: 0=W source, 1=X source
    // ftype: 00=S dest, 01=D dest
    // opcode: 010=signed (SCVTF), 011=unsigned (UCVTF)
    if operands.len() < 2 {
        return Err("scvtf/ucvtf requires 2 operands".to_string());
    }
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, rn_is_64) = get_reg(operands, 1)?;

    let dst_name = match &operands[0] {
        Operand::Reg(name) => name.to_lowercase(),
        _ => return Err("scvtf/ucvtf: expected register dest".to_string()),
    };
    // ftype: 00=S destination, 01=D destination, 11=H destination.
    let ftype = fp_ftype(&dst_name)?;
    let sf: u32 = if rn_is_64 { 1 } else { 0 };
    let opcode: u32 = if is_signed { 0b010 } else { 0b011 };

    let word = (((sf << 31) | (0b11110 << 24) | (ftype << 22) | (1 << 21)) | (opcode << 16))
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_fcvt_precision(operands: &[Operand]) -> Result<EncodeResult, String> {
    // FCVT: float precision conversion (e.g., FCVT Dd, Sn or FCVT Sd, Dn)
    // Encoding: 0 00 11110 ftype 1 0001 opc 10000 Rn Rd
    // ftype: source precision (00=S, 01=D, 11=H)
    // opc: dest precision (00=S, 01=D, 11=H)
    if operands.len() < 2 {
        return Err("fcvt requires 2 operands".to_string());
    }
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;

    let dst_name = match &operands[0] {
        Operand::Reg(name) => name.to_lowercase(),
        _ => return Err("fcvt: expected register dest".to_string()),
    };
    let src_name = match &operands[1] {
        Operand::Reg(name) => name.to_lowercase(),
        _ => return Err("fcvt: expected register source".to_string()),
    };

    let ftype: u32 = match src_name.chars().next() {
        Some('s') => 0b00,
        Some('d') => 0b01,
        Some('h') => 0b11,
        _ => return Err(format!("fcvt: unsupported source type: {}", src_name)),
    };
    let opc: u32 = match dst_name.chars().next() {
        Some('s') => 0b00,
        Some('d') => 0b01,
        Some('h') => 0b11,
        _ => return Err(format!("fcvt: unsupported dest type: {}", dst_name)),
    };

    let word = (0b00011110 << 24)
        | (ftype << 22)
        | (1 << 21)
        | (0b0001 << 17)
        | (opc << 15)
        | (0b10000 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}
