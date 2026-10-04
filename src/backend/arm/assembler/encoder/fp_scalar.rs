use super::*;
use crate::backend::arm::assembler::parser::Operand;

// ── Floating point ───────────────────────────────────────────────────────

/// Width of a general-purpose register operand, in bits.
///
/// Returns `Err` for anything that is not `w`/`x`/`wsp`/`sp`/`wzr`/`xzr`, so a
/// mistyped register is diagnosed instead of being defaulted to 32-bit.
fn gp_reg_width(name: &str) -> Result<u32, String> {
    match name.to_lowercase().as_str() {
        "sp" | "xzr" => Ok(64),
        "wsp" | "wzr" => Ok(32),
        other => match other.chars().next() {
            Some('x') => Ok(64),
            Some('w') => Ok(32),
            _ => Err(format!(
                "fmov: `{name}` is not a general-purpose register (expected w, x, wzr or xzr)"
            )),
        },
    }
}

/// FMOV between two FP registers.
///
/// This is a *same-width* move: the instruction carries a single `type` field,
/// so the operands must agree. The old code preferred the destination and
/// silently fell back to Rm, so `fmov s0,d1` and `fmov d0,s1` both assembled --
/// and produced different encodings depending on which end you wrote first --
/// even though GAS rejects both.
fn encode_fmov_fp_fp(rd_name: &str, rm_name: &str) -> Result<EncodeResult, String> {
    let ftype = fp_ftype(rd_name)?;
    let rm_ftype = fp_ftype(rm_name)?;
    if ftype != rm_ftype {
        return Err(format!(
            "fmov: `{rd_name}` and `{rm_name}` have different floating-point widths; fmov between FP registers moves bits within one width, so both operands must be the same (h, s or d)"
        ));
    }
    let rd = parse_reg_num(rd_name).ok_or_else(|| format!("invalid rd: {rd_name}"))?;
    let rm = parse_reg_num(rm_name).ok_or_else(|| format!("invalid rm: {rm_name}"))?;
    // 0 00 11110 ftype 1 0000 00 10000 Rm Rd
    let word =
        (0b00011110 << 24) | (ftype << 22) | (0b100000 << 16) | (0b10000 << 10) | (rm << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// FMOV (general): move between a general-purpose register and the low bits of
/// a scalar FP register.
///
/// The legal matrix is narrow, and it used to be approximated by "is the FP
/// register named `d`?" -- which silently produced the *single* encoding for
/// `fmov h0,w1` (GAS emits 1ee70020; lccc emitted 1e270020, which is
/// `fmov s0,w1`).
///
/// S <-> W sf=0 type=00
/// D <-> X sf=1 type=01
/// H <-> W sf=0 type=11
/// H <-> X sf=1 type=11
///
/// `type` follows the FP operand and `sf` follows the GP operand, so for S and
/// D the two must agree -- `fmov s0,x1` and `fmov d0,w1` are invalid. H is the
/// only FP width that pairs with either GP width. Q and B have no FMOV
/// (general) form at all, which `fp_ftype` already rejects by name.
fn encode_fmov_general(fp_name: &str, gp_name: &str, to_fp: bool) -> Result<EncodeResult, String> {
    let ftype = fp_ftype(fp_name)?;
    let gp_is_64 = gp_reg_width(gp_name)? == 64;

    let sf = match ftype {
        // S only pairs with a 32-bit GP register.
        0b00 if gp_is_64 => {
            return Err(format!(
                "fmov: `{fp_name}` is single-precision and only moves to or from a 32-bit general-purpose register, but `{gp_name}` is 64-bit"
            ));
        }
        // D only pairs with a 64-bit GP register.
        0b01 if !gp_is_64 => {
            return Err(format!(
                "fmov: `{fp_name}` is double-precision and only moves to or from a 64-bit general-purpose register, but `{gp_name}` is 32-bit"
            ));
        }
        0b00 => 0,
        0b01 => 1,
        // H (type=11) pairs with either width; `sf` follows the GP register.
        _ => u32::from(gp_is_64),
    };

    let fp_num = parse_reg_num(fp_name).ok_or_else(|| format!("invalid register: {fp_name}"))?;
    let gp_num = parse_reg_num(gp_name).ok_or_else(|| format!("invalid register: {gp_name}"))?;

    // In both directions Rn is the source and Rd the destination.
    let (rn, rd) = if to_fp {
        (gp_num, fp_num)
    } else {
        (fp_num, gp_num)
    };
    // opcode 111 = FMOV <Fd>, <Rn>; 110 = FMOV <Rd>, <Fn>
    let opcode: u32 = if to_fp { 0b111 } else { 0b110 };

    // sf 00 11110 type 1 00 opcode 000000 Rn Rd
    let word = (sf << 31)
        | (0b0011110 << 24)
        | (ftype << 22)
        | (1 << 21)
        | (opcode << 16)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_fmov(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!("fmov requires 2 operands, got {}", operands.len()));
    }

    let (rd_name, rm_name) = match (&operands[0], &operands[1]) {
        (Operand::Reg(a), Operand::Reg(b)) => (a.clone(), b.clone()),
        (Operand::Reg(_), Operand::Imm(_)) => {
            return Err(
                "fmov with an immediate operand is not supported; materialise the constant with `mov` into a GP register, or load it from .rodata"
                    .to_string(),
            );
        }
        _ => return Err("fmov needs register operands".to_string()),
    };

    let rd_is_fp = is_fp_reg(&rd_name);
    let rm_is_fp = is_fp_reg(&rm_name);

    match (rd_is_fp, rm_is_fp) {
        (true, true) => encode_fmov_fp_fp(&rd_name, &rm_name),
        // FMOV <Fd>, <Rn> -- destination is FP, source is GP
        (true, false) => encode_fmov_general(&rd_name, &rm_name, true),
        // FMOV <Rd>, <Fn> -- destination is GP, source is FP
        (false, true) => encode_fmov_general(&rm_name, &rd_name, false),
        (false, false) => Err(format!(
            "fmov: at least one operand must be a floating-point register, but `{rd_name}` and `{rm_name}` are both general-purpose (use `mov`)"
        )),
    }
}

pub(crate) fn encode_fp_arith(operands: &[Operand], opcode: u32) -> Result<EncodeResult, String> {
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

pub(crate) fn encode_fneg(operands: &[Operand]) -> Result<EncodeResult, String> {
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

pub(crate) fn encode_fabs(operands: &[Operand]) -> Result<EncodeResult, String> {
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

pub(crate) fn encode_fsqrt(operands: &[Operand]) -> Result<EncodeResult, String> {
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
pub(crate) fn encode_fp_1src(operands: &[Operand], opcode: u32) -> Result<EncodeResult, String> {
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
/// S (single, 32-bit) -> 0b00
/// D (double, 64-bit) -> 0b01
/// H (half, 16-bit) -> 0b11
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
            "unsupported floating-point register `{name}` (expected h, s or d for this instruction)"
        )),
    }
}

/// Encode FMADD/FMSUB: Rd = Ra +/- (Rn * Rm)
/// Format: 0 00 11111 ftype 0 Rm o1 Ra Rn Rd
pub(crate) fn encode_fmadd_fmsub(
    operands: &[Operand],
    is_sub: bool,
) -> Result<EncodeResult, String> {
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
pub(crate) fn encode_fnmadd_fnmsub(
    operands: &[Operand],
    is_sub: bool,
) -> Result<EncodeResult, String> {
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

pub(crate) fn encode_fcmp(operands: &[Operand]) -> Result<EncodeResult, String> {
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

/// Resolve the optional fixed-point `fbits` operand shared by the FCVT* and
/// *CVTF conversions.
///
/// Two things change when a third operand is present, and the encoder used to
/// do neither:
///
/// * bit 21 flips from 1 to 0 -- it is the bit that distinguishes the integer
///   form (`sf 00 11110 type 1 rmode opcode 000000 Rn Rd`) from the fixed-point
///   form (`sf 00 11110 type 0 rmode opcode scale Rn Rd`);
/// * `scale`, bits 15..10, becomes `64 - fbits`.
///
/// Ignoring the operand meant `fcvtzs x10,s30,#55` assembled as
/// `fcvtzs x10,s30` (0x9e3803ca) instead of 0x9e1827ca -- a conversion whose
/// result is wrong by a factor of 2^55, with nothing in the output to show it.
///
/// Returns `(bit21, scale)`.
fn fp_fbits_field(operands: &[Operand], int_width: u32, what: &str) -> Result<(u32, u32), String> {
    match operands.get(2) {
        None => Ok((1, 0)),
        Some(Operand::Imm(fbits)) => {
            if !(1..=(int_width as i64)).contains(fbits) {
                return Err(format!(
                    "{what}: fbits must be in 1..={int_width} for a {int_width}-bit \
                     integer operand, got {fbits}"
                ));
            }
            Ok((0, (64 - fbits) as u32))
        }
        Some(other) => Err(format!(
            "{what}: expected an immediate fbits operand, got {other:?}"
        )),
    }
}

/// The float-to-integer mnemonic for an (rmode, opcode) pair, used only so
/// diagnostics can name the instruction instead of shrugging "fcvt*".
fn fcvt_rounding_name(rmode: u32, opcode: u32) -> &'static str {
    match (rmode, opcode) {
        (0b11, 0b000) => "fcvtzs",
        (0b11, 0b001) => "fcvtzu",
        (0b00, 0b100) => "fcvtas",
        (0b00, 0b101) => "fcvtau",
        (0b00, 0b000) => "fcvtns",
        (0b00, 0b001) => "fcvtnu",
        (0b10, 0b000) => "fcvtms",
        (0b10, 0b001) => "fcvtmu",
        (0b01, 0b000) => "fcvtps",
        (0b01, 0b001) => "fcvtpu",
        _ => "fcvt*",
    }
}

pub(crate) fn encode_fcvt_rounding(
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

    let what = fcvt_rounding_name(rmode, opcode);
    // Only FCVTZS and FCVTZU have a fixed-point form; the other eight rounding
    // modes are integer-only. GAS rejects `fcvtas w1,s2,#8`, and so must we --
    // accepting it would encode a rounding mode the hardware does not have.
    let (bit21, scale) = if rmode == 0b11 {
        fp_fbits_field(operands, if rd_is_64 { 64 } else { 32 }, what)?
    } else if operands.get(2).is_some() {
        return Err(format!(
            "{what} has no fixed-point form: the `#fbits` operand is only \
             available on fcvtzs and fcvtzu, whose rounding mode is toward zero"
        ));
    } else {
        (1, 0)
    };

    let word = ((sf << 31)
        | (0b11110 << 24)
        | (ftype << 22)
        | (bit21 << 21)
        | (rmode << 19)
        | (opcode << 16)
        | (scale << 10))
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_ucvtf(operands: &[Operand]) -> Result<EncodeResult, String> {
    encode_int_to_float(operands, false)
}

pub(crate) fn encode_scvtf(operands: &[Operand]) -> Result<EncodeResult, String> {
    encode_int_to_float(operands, true)
}

pub(crate) fn encode_int_to_float(
    operands: &[Operand],
    is_signed: bool,
) -> Result<EncodeResult, String> {
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

    let what = if is_signed { "scvtf" } else { "ucvtf" };
    let (bit21, scale) = fp_fbits_field(operands, if rn_is_64 { 64 } else { 32 }, what)?;

    let word = (((sf << 31) | (0b11110 << 24) | (ftype << 22) | (bit21 << 21)) | (opcode << 16))
        | (scale << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_fcvt_precision(operands: &[Operand]) -> Result<EncodeResult, String> {
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
