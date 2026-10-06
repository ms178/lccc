use super::*;
use crate::backend::arm::assembler::parser::Operand;

// ── Floating point ───────────────────────────────────────────────────────

/// Whether `name` spells a floating-point/SIMD register: a `b`/`h`/`s`/`d`/
/// `q`/`v` prefix followed by a canonical register number.
///
/// The first-letter test this replaces (`matches!(c, 'd' | 's' | ...)`) counted
/// *any* name starting with `s` as single-precision, so `sp` was classified as
/// an FP register and `fmov d0, sp` was diagnosed as a floating-point width
/// mismatch -- a true statement about the wrong operand.  Classification now
/// decides between two disjoint grammars: `GpReg::by_name` owns the GP
/// spellings and this owns the FP ones, so a name is never both.
fn is_fp_spelling(name: &str) -> bool {
    let Some((first, rest)) = name.as_bytes().split_first() else {
        return false;
    };
    if !matches!(
        first,
        b'b' | b'B' | b'h' | b'H' | b's' | b'S' | b'd' | b'D' | b'q' | b'Q' | b'v' | b'V'
    ) {
        return false;
    }
    !rest.is_empty()
        && rest.len() <= 2
        && rest.iter().all(u8::is_ascii_digit)
        // Canonical decimal only, matching the GP grammar: `s007` is not a
        // register spelling.
        && !(rest.len() == 2 && rest[0] == b'0')
}

/// The mnemonic for a scalar-FP arithmetic opcode field, for diagnostics.
///
/// The dispatcher passes the field, not the name; a diagnostic that says
/// "fadd: `s0` and `d1` are different widths" is worth the four lines, and a
/// diagnostic that says "fp_arith: ..." is not.
const fn fp_arith_name(opcode: u32) -> &'static str {
    match opcode {
        0b0000 => "fmul",
        0b0001 => "fdiv",
        0b0010 => "fadd",
        0b0011 => "fsub",
        0b0100 => "fmax",
        0b0101 => "fmin",
        0b0110 => "fmaxnm",
        0b0111 => "fminnm",
        _ => "scalar-FP arithmetic",
    }
}

/// Read `n` floating-point register operands that must all share one width.
///
/// Every scalar-FP form carries a single `type` field, so the operand widths
/// are not independent: `fadd s0, d1, d2` has no encoding, and the encoder
/// used to accept it and emit `fadd s0, s1, s2` -- the widths of the *source*
/// operands were simply not read.  Reading them here makes the check a
/// property of the reader rather than a rule each of the eleven arithmetic
/// encoders has to remember, and it is what the `sweep register class` rows
/// like `fadd s0, d1, d2` / `fadd s0, d1, d1` pin.
///
/// Returns the register numbers and the shared width letter.
fn fp_same_width(operands: &[Operand], n: usize, mn: &str) -> Result<(Vec<u32>, u8), String> {
    let mut nums = Vec::with_capacity(n);
    let mut letter = 0u8;
    let mut first_name = String::new();
    for i in 0..n {
        let (num, l, name) = fp_reg(operands, i, mn)?;
        if i == 0 {
            letter = l;
            first_name = name;
        } else if l != letter {
            return Err(format!(
                "{mn}: `{first_name}` and `{name}` are different floating-point widths; \
                 this instruction has one type field, so every register operand of {mn} \
                 must be the same size (`{}`-register form or `{}`-register form)",
                char::from(letter),
                char::from(l)
            ));
        }
        nums.push(num);
    }
    Ok((nums, letter))
}

/// Read the integer operand of a conversion.
///
/// `FCVTZS Wd, Sn`, `SCVTF Sd, Wn` and their relatives take `Wd|Xd` / `Wn|Xn`
/// with encoding 31 read as the *zero* register -- never the stack pointer,
/// which is what `reg_operand(..., GpRole::RegOrZr, ..)` enforces.
fn fp_conv_gp(operands: &[Operand], idx: usize, mn: &str) -> Result<(u32, bool), String> {
    let r = reg_operand(operands, idx, GpRole::RegOrZr, mn)?;
    Ok((r.num, r.is_64))
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
    // The GP slot of FMOV (general) is `Xn|XZR` or `Wn|WZR`: encoding 31 here
    // is the zero register, so the stack pointer is not encodable and neither
    // is the role-free `x31`/`w31` spelling.  `fmov h0, wsp` used to assemble
    // as `FMOV H0, WZR` (0x1ee703e0) -- silently moving the zero register
    // where the stack pointer was written -- and GNU as rejects it even with
    // `-march=armv8.2-a+fp16`, which is what makes the H form legal at all.
    let gp = GpReg::by_name(gp_name).ok_or_else(|| {
        format!(
            "fmov: `{gp_name}` is not a general-purpose register (expected \
             w0-w30, x0-x30, lr, wzr or xzr)"
        )
    })?;
    if gp.is_sp {
        return Err(format!(
            "fmov: `{gp_name}` is the stack pointer, which FMOV (general) cannot \
             move; encoding 31 in this slot is the zero register, so write `{}` \
             if that is what you meant",
            if gp.is_64 { "xzr" } else { "wzr" }
        ));
    }
    let gp_is_64 = gp.is_64;

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
    let gp_num = gp.num;

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

    // Classify by identity: see `is_fp_spelling` for why the first-letter
    // test sent `fmov d0, sp` down the FP-to-FP path.
    let rd_is_fp = is_fp_spelling(&rd_name);
    let rm_is_fp = is_fp_spelling(&rm_name);

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
    let mn = fp_arith_name(opcode);
    let (regs, letter) = fp_same_width(operands, 3, mn)?;
    let (rd, rn, rm) = (regs[0], regs[1], regs[2]);
    let ftype = fp_ftype_letter(letter, mn)?;

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
    let (regs, letter) = fp_same_width(operands, 2, "fneg")?;
    let (rd, rn) = (regs[0], regs[1]);
    let ftype = fp_ftype_letter(letter, "fneg")?;
    // FNEG: 0 00 11110 ftype 1 0000 10 10000 Rn Rd
    let word =
        (0b00011110 << 24) | (ftype << 22) | (0b100001 << 16) | (0b10000 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_fabs(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (regs, letter) = fp_same_width(operands, 2, "fabs")?;
    let (rd, rn) = (regs[0], regs[1]);
    let ftype = fp_ftype_letter(letter, "fabs")?;
    // FABS: 0 00 11110 ftype 1 0000 01 10000 Rn Rd
    let word =
        (0b00011110 << 24) | (ftype << 22) | (0b100000 << 16) | (0b110000 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_fsqrt(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (regs, letter) = fp_same_width(operands, 2, "fsqrt")?;
    let (rd, rn) = (regs[0], regs[1]);
    let ftype = fp_ftype_letter(letter, "fsqrt")?;
    // FSQRT: 0 00 11110 ftype 1 0000 11 10000 Rn Rd
    let word =
        (0b00011110 << 24) | (ftype << 22) | (0b100001 << 16) | (0b110000 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode FP 1-source ops: FRINTN/P/M/Z/A/X/I
/// Format: 0 00 11110 ftype 1 opcode 10000 Rn Rd
pub(crate) fn encode_fp_1src(operands: &[Operand], opcode: u32) -> Result<EncodeResult, String> {
    let (regs, letter) = fp_same_width(operands, 2, "frint*")?;
    let (rd, rn) = (regs[0], regs[1]);
    let ftype = fp_ftype_letter(letter, "frint*")?;
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
    let mn = if is_sub { "fmsub" } else { "fmadd" };
    let (regs, letter) = fp_same_width(operands, 4, mn)?;
    let (rd, rn, rm, ra) = (regs[0], regs[1], regs[2], regs[3]);
    let ftype = fp_ftype_letter(letter, mn)?;
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
    let mn = if is_sub { "fnmsub" } else { "fnmadd" };
    let (regs, letter) = fp_same_width(operands, 4, mn)?;
    let (rd, rn, rm, ra) = (regs[0], regs[1], regs[2], regs[3]);
    let ftype = fp_ftype_letter(letter, mn)?;
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
    let (rn, letter, _) = fp_reg(operands, 0, "fcmp")?;
    let ftype = fp_ftype_letter(letter, "fcmp")?;

    // FCMP Dn, #0.0
    if operands.len() < 2 || matches!(operands.get(1), Some(Operand::Imm(0))) {
        let word = ((0b00011110 << 24) | (ftype << 22) | (1 << 21))
            | (0b001000 << 10)
            | (rn << 5)
            | 0b01000;
        return Ok(EncodeResult::Word(word));
    }

    let (rm, rm_letter, rm_name) = fp_reg(operands, 1, "fcmp")?;
    if rm_letter != letter {
        return Err(format!(
            "fcmp: the two operands must be the same floating-point width \
             (`{}`-register form and `{}`-register form given)",
            char::from(letter),
            char::from(rm_letter)
        ));
    }
    let _ = rm_name;
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
    let what = fcvt_rounding_name(rmode, opcode);
    let (rd, rd_is_64) = fp_conv_gp(operands, 0, what)?;
    let (rn, src_letter, _) = fp_reg(operands, 1, what)?;
    // ftype: 00=S source, 01=D source, 11=H source.
    // The `h` case was missing, so `fcvtzu w4, h17, #13` converted a
    // single-precision value it did not have instead of the half-precision one
    // it was given.
    let ftype = fp_ftype_letter(src_letter, what)?;
    let sf: u32 = if rd_is_64 { 1 } else { 0 };

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
    let what = if is_signed { "scvtf" } else { "ucvtf" };
    let (rd, dst_letter, _) = fp_reg(operands, 0, what)?;
    let (rn, rn_is_64) = fp_conv_gp(operands, 1, what)?;
    // ftype: 00=S destination, 01=D destination, 11=H destination.
    let ftype = fp_ftype_letter(dst_letter, what)?;
    let sf: u32 = if rn_is_64 { 1 } else { 0 };
    let opcode: u32 = if is_signed { 0b010 } else { 0b011 };

    let (bit21, scale) = fp_fbits_field(operands, if rn_is_64 { 64 } else { 32 }, what)?;

    let word = (((sf << 31) | (0b11110 << 24) | (ftype << 22) | (bit21 << 21)) | (opcode << 16))
        | (scale << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Base words (`s`/`d` form, `h` form) of the twelve scalar SIMD&FP
/// conversions whose two operands live in one register file at one width.
///
/// `fcvtms s0, s1` is *not* `fcvtms w0, s1` with an odd destination: it is the
/// scalar register-file form, and the rounding mode is baked into the opcode
/// field instead of living in a `rmode` field, so every mnemonic has a base of
/// its own.  The `h` form is a separate base as well, because its element-size
/// field is the half-precision one and not the `size` bit of the `s`/`d` pair.
///
/// Each base below is the value binutils' own AArch64 encoding table gives
/// (`opcodes/aarch64-tbl.h`, `SIMD_INSN`/`SF16_INSN` with class `asisdmisc`
/// and qualifier `QL_S_2SAMESD`/`QL_S_2SAMEH`), and the mask that comes with it
/// — 0xffbffc00 for the `s`/`d` base, 0xfffffc00 for the `h` one — says
/// exactly what the encoder varies: bit 22 (`size`, `s` = 0, `d` = 1) and the
/// two register fields.  GNU as 2.47 assembles `fcvtms s0,s1` to 0x5e21b820,
/// `fcvtms d0,d1` to 0x5e61b820 and `fcvtms h0,h1` to 0x5e79b820: these bases
/// with both register fields zero.
const FCVT_SIMD_SCALAR_BASES: &[(&str, u32, u32)] = &[
    ("fcvtns", 0x5e21a800, 0x5e79a800),
    ("fcvtnu", 0x7e21a800, 0x7e79a800),
    ("fcvtps", 0x5ea1a800, 0x5ef9a800),
    ("fcvtpu", 0x7ea1a800, 0x7ef9a800),
    ("fcvtms", 0x5e21b800, 0x5e79b800),
    ("fcvtmu", 0x7e21b800, 0x7e79b800),
    ("fcvtzs", 0x5ea1b800, 0x5ef9b800),
    ("fcvtzu", 0x7ea1b800, 0x7ef9b800),
    ("fcvtas", 0x5e21c800, 0x5e79c800),
    ("fcvtau", 0x7e21c800, 0x7e79c800),
    ("scvtf", 0x5e21d800, 0x5e79d800),
    ("ucvtf", 0x7e21d800, 0x7e79d800),
];

/// `(mnemonic, name of the error's instruction)` for the twelve scalar
/// SIMD&FP conversions above — the set the dispatcher hands to
/// [`encode_fp_convert_scalar`] rather than to the general-purpose-destination
/// encoders.
pub(crate) fn is_scalar_fp_convert(name: &str) -> bool {
    FCVT_SIMD_SCALAR_BASES.iter().any(|(m, _, _)| *m == name)
}

/// Encode a scalar SIMD&FP conversion: `fcvtms s0,s1`, `scvtf d0,d1`, and the
/// fixed-point forms `fcvtzs s0,s1,#fbits` and `ucvtf h0,h1,#fbits`.
///
/// Both register operands are read from the floating-point register file and
/// must be spelled at the same width; every mixed spelling (`fcvtms s0,d1`,
/// `scvtf d0,s1`, `fcvtms s0,h1`) is refused by GNU as and has no encoding.
/// The third operand, when present, is the fixed-point scale — a bit count, so
/// its range is the element width, not the register number space — and only
/// `fcvtzs`, `fcvtzu`, `scvtf` and `ucvtf` have that form.
///
/// The fixed-point field is the `immh`-style scale: `64 - fbits` for the `s`
/// and `d` elements (with the element size as its top bit, so `d` reads
/// `0x40 | (64 - fbits)`) and `32 - fbits` for a half-precision one, which is
/// exactly how GNU as lays out `fcvtzs d0,d1,#7` (0x5f79fc20),
/// `fcvtzs s0,s1,#32` (0x5f20fc20) and `fcvtzs h0,h1,#16` (0x5f10fc20).
pub(crate) fn encode_fp_convert_scalar(
    name: &str,
    operands: &[Operand],
) -> Result<EncodeResult, String> {
    let (rd, dst_letter, dst_name) = fp_reg(operands, 0, name)?;
    let (rn, src_letter, src_name) = fp_reg(operands, 1, name)?;
    if !matches!(dst_letter, b'h' | b's' | b'd') {
        return Err(format!(
            "{name}: `{dst_name}` is not a 16/32/64-bit floating-point register; the \
             scalar form takes h0-h31, s0-s31 or d0-d31 (a vector form needs an \
             arrangement, for example `{name} v0.4s, v1.4s`)"
        ));
    }
    if dst_letter != src_letter {
        return Err(format!(
            "{name}: `{dst_name}` and `{src_name}` have different floating-point widths; \
             the scalar form of {name} converts within one width (h, s or d)"
        ));
    }
    // The element width is the only width either operand is allowed to have,
    // and it is also the fixed-point bit count's range.
    let elem_bits: u32 = match dst_letter {
        b'h' => 16,
        b's' => 32,
        _ => 64,
    };

    if operands.len() == 3 {
        if !matches!(name, "fcvtzs" | "fcvtzu" | "scvtf" | "ucvtf") {
            return Err(format!(
                "{name} has no fixed-point form: the `#fbits` operand is only available \
                 on fcvtzs, fcvtzu, scvtf and ucvtf"
            ));
        }
        let fbits = match operands.get(2) {
            Some(Operand::Imm(v)) => *v,
            other => {
                return Err(format!(
                    "{name}: expected an immediate fbits operand at operand 3, got {other:?}"
                ));
            }
        };
        if !(1..=elem_bits as i64).contains(&fbits) {
            return Err(format!(
                "{name}: fbits must be in 1..={elem_bits} for a {elem_bits}-bit \
                 floating-point operand, got {fbits}"
            ));
        }
        // 64 - fbits, with the element size as the field's top bit for `d`;
        // the half-precision element uses 32 - fbits, whose value always has
        // the bit the `h` base sets.
        let immh: u32 = match dst_letter {
            b'h' => 32 - fbits as u32,
            b's' => 64 - fbits as u32,
            _ => 64 + (64 - fbits as u32),
        };
        let base: u32 = match name {
            "fcvtzs" => 0x5f00fc00,
            "fcvtzu" => 0x7f00fc00,
            "scvtf" => 0x5f00e400,
            _ => 0x7f00e400,
        };
        return Ok(EncodeResult::Word(base | (immh << 16) | (rn << 5) | rd));
    }
    if operands.len() != 2 {
        return Err(format!(
            "{name} takes 2 or 3 operands (Sd, Sn or Sd, Sn, #fbits), got {}",
            operands.len()
        ));
    }

    let (_, base_sd, base_h) = FCVT_SIMD_SCALAR_BASES
        .iter()
        .find(|(m, _, _)| *m == name)
        .ok_or_else(|| format!("{name}: no scalar SIMD&FP encoding"))?;
    // Bit 22 is `size`: the H form has no such bit and its own base instead.
    let size = if dst_letter == b'd' { 1u32 << 22 } else { 0 };
    let base = if dst_letter == b'h' {
        *base_h
    } else {
        *base_sd
    };
    Ok(EncodeResult::Word(base | size | (rn << 5) | rd))
}

pub(crate) fn encode_fcvt_precision(operands: &[Operand]) -> Result<EncodeResult, String> {
    // FCVT: float precision conversion (e.g., FCVT Dd, Sn or FCVT Sd, Dn)
    // Encoding: 0 00 11110 ftype 1 0001 opc 10000 Rn Rd
    // ftype: source precision (00=S, 01=D, 11=H)
    // opc: dest precision (00=S, 01=D, 11=H)
    if operands.len() < 2 {
        return Err("fcvt requires 2 operands".to_string());
    }
    let (rd, dst_letter, dst_name) = fp_reg(operands, 0, "fcvt")?;
    let (rn, src_letter, src_name) = fp_reg(operands, 1, "fcvt")?;
    // FCVT is a *precision* conversion: it re-encodes a value in a different
    // floating-point width, so the two operands must differ.  The instruction
    // has no same-width form at all (that is what makes it an alias-free
    // conversion rather than a rounding step), and GNU as refuses `fcvt s0,s1`
    // exactly as it refuses `fcvt h0,h1` and `fcvt d0,d1`.  The old code took
    // the two width letters independently and assembled all three, emitting an
    // undefined encoding whose disassembly is `.inst`, not `fcvt`.
    if dst_letter == src_letter {
        return Err(format!(
            "fcvt: `{dst_name}` and `{src_name}` are the same floating-point width; fcvt \
             converts between widths (h to s, h to d, s to d and back), so the two \
             operands must differ (for a same-width rounding conversion use fcvtms, \
             fcvtns, fcvtas, fcvtzs, ...)"
        ));
    }
    let ftype: u32 = fp_ftype_letter(src_letter, "fcvt")?;
    let opc: u32 = fp_ftype_letter(dst_letter, "fcvt")?;

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

#[cfg(test)]
mod fmov_gp_operand_tests {
    use super::*;

    fn word(r: Result<EncodeResult, String>) -> u32 {
        match r {
            Ok(EncodeResult::Word(w)) => w,
            Ok(other) => panic!("expected a single word, got {other:?}"),
            Err(e) => panic!("expected an encoding, got error: {e}"),
        }
    }

    fn err(r: Result<EncodeResult, String>) -> String {
        match r {
            Err(e) => e,
            Ok(v) => panic!("expected an error, got {v:?}"),
        }
    }

    /// `lr` is x30, so it is a 64-bit GP register and pairs with D and H.
    /// Every expected word below was produced by GNU as, which assembles
    /// `fmov d0,lr` to exactly the word it assembles `fmov d0,x30` to.
    #[test]
    fn lr_is_a_64bit_general_purpose_register() {
        assert_eq!(
            word(encode_fmov_general("d0", "lr", true)),
            word(encode_fmov_general("d0", "x30", true)),
            "`lr` and `x30` must be indistinguishable to every caller"
        );
        assert_eq!(word(encode_fmov_general("d0", "lr", true)), 0x9e6703c0);
        assert_eq!(
            word(encode_fmov_general("d0", "LR", true)),
            0x9e6703c0,
            "register names are case-insensitive"
        );
        // A 32-bit-only pairing therefore rejects it, exactly as GAS does.
        assert!(encode_fmov_general("s0", "lr", true).is_err());
    }

    /// Encoding 31 in the GP slot of FMOV (general) is the zero register, so
    /// `xzr`/`wzr` assemble and `sp`/`wsp` do not.  This is the wrong-code bug
    /// the width-only check allowed: `fmov h0, wsp` encoded as `FMOV H0, WZR`
    /// (0x1ee703e0) instead of failing, because H pairs with either GP width
    /// and so reached the encoder at all.
    #[test]
    fn the_gp_slot_takes_the_zero_register_but_never_the_stack_pointer() {
        assert_eq!(word(encode_fmov_general("d0", "xzr", true)), 0x9e6703e0);
        assert_eq!(word(encode_fmov_general("h0", "wzr", true)), 0x1ee703e0);
        for gp in ["sp", "wsp", "SP", "Wsp"] {
            let e = err(encode_fmov_general("h0", gp, true));
            assert!(
                e.contains("stack pointer") && e.contains("zero register"),
                "`fmov h0, {gp}` must explain the 31 ambiguity, got: {e}"
            );
        }
        // The full dispatcher must reject it too, not just the leaf encoder.
        let ops = vec![
            Operand::Reg("h0".to_string()),
            Operand::Reg("wsp".to_string()),
        ];
        assert!(encode_fmov(&ops).is_err());
    }

    /// The GP grammar is `x0`-`x30`, `w0`-`w30`, `lr`, `xzr`/`wzr`.  `x31`/`w31`
    /// are role-free spellings of encoding 31 and are refused, as are FP
    /// spellings, out-of-range numbers and non-canonical decimals.
    #[test]
    fn rejects_spellings_that_are_not_general_purpose_registers() {
        for name in [
            "d0", "s0", "h0", "q0", "b0", "v0", "", "x32", "x007", "x31", "w31", "x+5",
        ] {
            assert!(
                encode_fmov_general("d0", name, true).is_err(),
                "`fmov d0, {name}` must be rejected"
            );
        }
        // `sp` is a *valid* GP spelling that this instruction cannot use, so it
        // is reported as a role error rather than an unknown register.
        let e = err(encode_fmov_general("d0", "sp", true));
        assert!(e.contains("stack pointer"), "unexpected message: {e}");
    }

    /// The two operand grammars are disjoint, so classification cannot send a
    /// stack pointer down the FP-to-FP path (which reported a floating-point
    /// width mismatch for `fmov d0, sp`).
    #[test]
    fn operand_classification_separates_the_two_grammars() {
        for name in ["sp", "wsp", "x0", "w30", "lr", "xzr", "wzr"] {
            assert!(!is_fp_spelling(name), "`{name}` is not an FP register");
            assert!(GpReg::by_name(name).is_some(), "`{name}` is a GP register");
        }
        for name in ["s0", "d31", "h1", "q0", "b7", "v3", "S15"] {
            assert!(is_fp_spelling(name), "`{name}` is an FP register");
            assert!(
                GpReg::by_name(name).is_none(),
                "`{name}` is not a GP register"
            );
        }
        for name in ["s", "s007", "", "sp0"] {
            assert!(
                !is_fp_spelling(name),
                "`{name}` is not a canonical spelling"
            );
        }
    }
}

#[cfg(test)]
mod scalar_conversion_tests {
    use super::*;

    fn reg(name: &str) -> Operand {
        Operand::Reg(name.to_string())
    }
    fn imm(v: i64) -> Operand {
        Operand::Imm(v)
    }
    fn word(mn: &str, operands: &[Operand]) -> u32 {
        match encode_fp_convert_scalar(mn, operands) {
            Ok(EncodeResult::Word(w)) => w,
            Ok(other) => panic!("{mn}: expected a single word, got {other:?}"),
            Err(e) => panic!("{mn}: expected an encoding, got error: {e}"),
        }
    }
    fn err(mn: &str, operands: &[Operand]) -> String {
        match encode_fp_convert_scalar(mn, operands) {
            Err(e) => e,
            Ok(v) => panic!("{mn}: expected an error, got {v:?}"),
        }
    }

    /// Every word below is GNU as 2.47's own output, and the bases come from
    /// binutils' AArch64 encoding table.  All twelve mnemonics are checked at
    /// all three widths, because "one base word per mnemonic" is a claim about
    /// a family: a single spot check would not notice a base that is right for
    /// `s` and wrong for `h`.
    #[test]
    fn two_operand_conversions_match_gnu_as() {
        for (mn, s_word, d_word, h_word) in [
            ("fcvtns", 0x5e21a820u32, 0x5e61a820, 0x5e79a820),
            ("fcvtnu", 0x7e21a820, 0x7e61a820, 0x7e79a820),
            ("fcvtps", 0x5ea1a820, 0x5ee1a820, 0x5ef9a820),
            ("fcvtpu", 0x7ea1a820, 0x7ee1a820, 0x7ef9a820),
            ("fcvtms", 0x5e21b820, 0x5e61b820, 0x5e79b820),
            ("fcvtmu", 0x7e21b820, 0x7e61b820, 0x7e79b820),
            ("fcvtzs", 0x5ea1b820, 0x5ee1b820, 0x5ef9b820),
            ("fcvtzu", 0x7ea1b820, 0x7ee1b820, 0x7ef9b820),
            ("fcvtas", 0x5e21c820, 0x5e61c820, 0x5e79c820),
            ("fcvtau", 0x7e21c820, 0x7e61c820, 0x7e79c820),
            ("scvtf", 0x5e21d820, 0x5e61d820, 0x5e79d820),
            ("ucvtf", 0x7e21d820, 0x7e61d820, 0x7e79d820),
        ] {
            assert_eq!(word(mn, &[reg("s0"), reg("s1")]), s_word, "{mn} s0, s1");
            assert_eq!(word(mn, &[reg("d0"), reg("d1")]), d_word, "{mn} d0, d1");
            assert_eq!(word(mn, &[reg("h0"), reg("h1")]), h_word, "{mn} h0, h1");
            // The register fields are the low 10 bits, in both halves: `Rn` is
            // bits 9..5 and `Rd` bits 4..0, so a high-numbered pair proves the
            // fields are placed rather than merely OR'd into the base.
            assert_eq!(
                word(mn, &[reg("s31"), reg("s30")]),
                (s_word & !0x3FF) | (30 << 5) | 31,
                "{mn} s31, s30"
            );
        }
    }

    /// The fixed-point forms, measured from GNU as at both ends of every
    /// element width's range: the bit count is *inclusive* at the element width
    /// (`#32` for a single, `#64` for a double, `#16` for a half) and its field
    /// is the complement, with the element size as that field's top bit.
    #[test]
    fn fixed_point_forms_match_gnu_as() {
        for (mn, w, f, want) in [
            ("fcvtzs", "s", 1, 0x5f3ffc20u32),
            ("fcvtzs", "s", 3, 0x5f3dfc20),
            ("fcvtzs", "s", 32, 0x5f20fc20),
            ("fcvtzs", "d", 7, 0x5f79fc20),
            ("fcvtzs", "d", 64, 0x5f40fc20),
            ("fcvtzs", "h", 1, 0x5f1ffc20),
            ("fcvtzs", "h", 16, 0x5f10fc20),
            ("fcvtzu", "s", 16, 0x7f30fc20),
            ("fcvtzu", "h", 3, 0x7f1dfc20),
            ("scvtf", "s", 3, 0x5f3de420),
            ("scvtf", "d", 3, 0x5f7de420),
            ("ucvtf", "s", 32, 0x7f20e420),
            ("ucvtf", "d", 64, 0x7f40e420),
            ("ucvtf", "h", 16, 0x7f10e420),
        ] {
            let ops = [
                reg(&format!("{w}0")),
                reg(&format!("{w}1")),
                imm(i64::from(f)),
            ];
            assert_eq!(word(mn, &ops), want, "{mn} {w}0, {w}1, #{f}");
        }
    }

    #[test]
    fn the_fixed_point_bit_count_is_bounded_by_the_element_width() {
        for (w, max) in [("h", 16i64), ("s", 32), ("d", 64)] {
            let ok = [reg(&format!("{w}0")), reg(&format!("{w}1")), imm(max)];
            assert!(
                encode_fp_convert_scalar("fcvtzs", &ok).is_ok(),
                "fcvtzs {w}0, {w}1, #{max} is encodable"
            );
            for bad in [0, max + 1] {
                let ops = [reg(&format!("{w}0")), reg(&format!("{w}1")), imm(bad)];
                let e = err("fcvtzs", &ops);
                assert!(
                    e.contains("fbits must be in"),
                    "fcvtzs {w}0, {w}1, #{bad}: {e}"
                );
            }
        }
    }

    /// Two register operands of different widths, and every spelling that is
    /// not a scalar element register, are refused: the family converts within
    /// one width, and `v0` needs an arrangement to name one.
    #[test]
    fn mixed_width_and_non_scalar_spellings_are_refused() {
        for (mn, a, b) in [
            ("fcvtms", "s0", "d1"),
            ("fcvtms", "d0", "s1"),
            ("fcvtms", "s0", "h1"),
            ("fcvtms", "h0", "d1"),
            ("scvtf", "d0", "s1"),
            ("ucvtf", "s0", "h1"),
        ] {
            let e = err(mn, &[reg(a), reg(b)]);
            assert!(
                e.contains("different floating-point widths"),
                "{mn} {a}, {b}: {e}"
            );
        }
        // Every non-element spelling is refused, and the message names the
        // offending register: `q0`/`b0` are floating-point registers of the
        // wrong width, `x0`/`w0`/`sp` are not floating-point registers at all.
        for bad in ["v0", "q0", "b0", "x0", "w0", "sp"] {
            let e = err("fcvtms", &[reg(bad), reg("s1")]);
            assert!(e.contains(bad), "fcvtms {bad}, s1 must be refused: {e}");
            assert!(
                e.contains("floating-point") || e.contains("register"),
                "fcvtms {bad}, s1: {e}"
            );
        }
        let e = err("fcvtms", &[reg("v0"), reg("v1")]);
        assert!(e.contains("vector"), "{e}");
    }

    #[test]
    fn only_the_integer_conversions_have_a_fixed_point_form() {
        for mn in [
            "fcvtas", "fcvtau", "fcvtns", "fcvtnu", "fcvtps", "fcvtpu", "fcvtms", "fcvtmu",
        ] {
            let ops = [reg("s0"), reg("s1"), imm(3)];
            let e = err(mn, &ops);
            assert!(
                e.contains("no fixed-point form"),
                "{mn} s0, s1, #3 must name the missing form: {e}"
            );
        }
    }
}

#[cfg(test)]
mod fcvt_precision_tests {
    use super::*;

    fn reg(name: &str) -> Operand {
        Operand::Reg(name.to_string())
    }
    fn word(ops: &[Operand]) -> u32 {
        match encode_fcvt_precision(ops) {
            Ok(EncodeResult::Word(w)) => w,
            Ok(other) => panic!("expected a single word, got {other:?}"),
            Err(e) => panic!("expected an encoding, got error: {e}"),
        }
    }

    /// `fcvt` is a *precision* conversion: every mixed width pair is one
    /// instruction, and every equal-width pair has no encoding at all (the old
    /// encoder assembled those to a word that disassembles as `.inst`).
    #[test]
    fn converts_between_widths_and_refuses_equal_ones() {
        for (d, s, want) in [
            ("s", "d", 0x1e624020u32),
            ("d", "s", 0x1e22c020),
            ("h", "s", 0x1e23c020),
            ("h", "d", 0x1e63c020),
            ("s", "h", 0x1ee24020),
            ("d", "h", 0x1ee2c020),
        ] {
            assert_eq!(
                word(&[reg(&format!("{d}0")), reg(&format!("{s}1"))]),
                want,
                "fcvt {d}0, {s}1"
            );
        }
        for w in ["h", "s", "d"] {
            let e = match encode_fcvt_precision(&[reg(&format!("{w}0")), reg(&format!("{w}1"))]) {
                Err(e) => e,
                Ok(v) => panic!("fcvt {w}0, {w}1 must be refused, got {v:?}"),
            };
            assert!(e.contains("same floating-point width"), "{w}: {e}");
        }
    }
}
