//! Advanced-SIMD floating point outside the vector three-same / two-misc core
//! in `neon.rs`: the AdvSIMD *scalar* classes in all three widths (`h` is
//! FEAT_FP16), the scalar pairwise reductions, FCVTXN, FRINT32/64
//! (FEAT_FRINTTS), the unsigned estimates, complex arithmetic (FCMLA/FCADD,
//! FEAT_FCMA), widening half-precision FMA (FMLAL/FMLSL, FEAT_FHM) and the
//! BFloat16 data-processing set (FEAT_BF16).
//!
//! Every word below was measured against GNU as 2.47 (`.arch
//! armv9.4-a+sme+aes+sha3+sm4`, the dispatcher-golden prologue), and every
//! operand-shape rule mirrors a GAS rejection: an over-accepting assembler
//! turns a typo in hand-written NEON into a different instruction.

use super::*;
use crate::backend::arm::assembler::parser::Operand;

/// Read exactly `n` scalar FP register operands that share one width.
/// Returns the register numbers and the width letter (`h`, `s` or `d`).
fn fp_scalar_operands(
    operands: &[Operand],
    n: usize,
    what: &str,
) -> Result<(Vec<u32>, u8), String> {
    if operands.len() != n {
        return Err(format!(
            "{what}: expected {n} operands, got {}",
            operands.len()
        ));
    }
    let mut regs = Vec::with_capacity(n);
    let mut width = 0u8;
    for i in 0..n {
        let (num, letter, name) = fp_reg(operands, i, what)?;
        if !matches!(letter, b'h' | b's' | b'd') {
            return Err(format!(
                "{what}: operand {i} `{name}` must be an h, s or d register"
            ));
        }
        if i == 0 {
            width = letter;
        } else if letter != width {
            return Err(format!(
                "{what}: operand widths differ (`{}` and `{}`); the instruction has one size field",
                char::from(width),
                char::from(letter)
            ));
        }
        regs.push(num);
    }
    Ok((regs, width))
}

/// Advanced-SIMD scalar three-same, FP forms (`FABD`, `FMULX`, `FRECPS`,
/// `FRSQRTS`, `FCMEQ/GE/GT`, `FACGE/GT` on `h`/`s`/`d`).
///
/// `s`/`d`: `01 U 11110 a sz 1 Rm opcode 1 Rn Rd` (`fmulx d3, d29, d17` =
/// 0x5e71dfa3).  `h` lives in the FP16 scalar three-same class, `01 U 11110 a
/// 10 Rm 00 opc 1 Rn Rd`, whose 3-bit `opc` is the low three bits of the
/// single/double opcode -- every FP opcode of this class is `0b11xxx`
/// (`fmulx h3, h29, h17` = 0x5e511fa3, `fabd h3, h29, h17` = 0x7ed117a3).
pub(crate) fn encode_neon_fp_scalar_three_same(
    operands: &[Operand],
    u_bit: u32,
    a: u32,
    opcode: u32,
    what: &str,
) -> Result<EncodeResult, String> {
    let (r, w) = fp_scalar_operands(operands, 3, what)?;
    let (rd, rn, rm) = (r[0], r[1], r[2]);
    let word = if w == b'h' {
        debug_assert_eq!(
            opcode >> 3,
            0b11,
            "FP16 three-same opcodes are the 0b11xxx group"
        );
        (0b01 << 30)
            | (u_bit << 29)
            | (0b11110 << 24)
            | (a << 23)
            | (0b10 << 21)
            | (rm << 16)
            | ((opcode & 0b111) << 11)
            | (1 << 10)
            | (rn << 5)
            | rd
    } else {
        (0b01 << 30)
            | (u_bit << 29)
            | (0b11110 << 24)
            | (a << 23)
            | (u32::from(w == b'd') << 22)
            | (1 << 21)
            | (rm << 16)
            | (opcode << 11)
            | (1 << 10)
            | (rn << 5)
            | rd
    };
    Ok(EncodeResult::Word(word))
}

/// Advanced-SIMD scalar two-register miscellaneous, FP forms (`FRECPE`,
/// `FRSQRTE`, `FRECPX` on `h`/`s`/`d`).
///
/// `s`/`d`: `01 U 11110 a sz 10000 opcode 10 Rn Rd` (`frecpe d3, d29` =
/// 0x5ee1dba3); `h`: `01 U 11110 a 111100 opcode 10 Rn Rd` (`frecpe h3, h29` =
/// 0x5ef9dba3) -- the same opcode, with bits 22:17 naming the FP16 class.
pub(crate) fn encode_neon_fp_scalar_two_misc(
    operands: &[Operand],
    u_bit: u32,
    a: u32,
    opcode: u32,
    what: &str,
) -> Result<EncodeResult, String> {
    let (r, w) = fp_scalar_operands(operands, 2, what)?;
    let class = match w {
        b'h' => 0b111100,
        b'd' => 0b110000,
        _ => 0b010000,
    };
    Ok(EncodeResult::Word(
        (0b01 << 30)
            | (u_bit << 29)
            | (0b11110 << 24)
            | (a << 23)
            | (class << 17)
            | (opcode << 12)
            | (0b10 << 10)
            | (r[1] << 5)
            | r[0],
    ))
}

/// Advanced-SIMD scalar pairwise, FP forms: `FADDP`, `FMAXP`, `FMINP`,
/// `FMAXNMP`, `FMINNMP` with an `h`/`s`/`d` result from a `2h`/`2s`/`2d`
/// source.
///
/// `s`/`d`: `01 1 11110 o1 sz 11000 opcode 10 Rn Rd` (`fmaxnmp d3, v29.2d` =
/// 0x7e70cba3).  The FP16 forms clear U and sz: `faddp h3, v29.2h` =
/// 0x5e30dba3, `fminnmp h3, v29.2h` = 0x5eb0cba3.
pub(crate) fn encode_neon_fp_scalar_pairwise(
    operands: &[Operand],
    o1: u32,
    opcode: u32,
    what: &str,
) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "{what}: expected 2 operands (Fd, Vn.T), got {}",
            operands.len()
        ));
    }
    let (rd, letter, name) = fp_reg(operands, 0, what)?;
    let (u, sz, want) = match letter {
        b'h' => (0u32, 0u32, "2h"),
        b's' => (1, 0, "2s"),
        b'd' => (1, 1, "2d"),
        _ => {
            return Err(format!(
                "{what}: the destination `{name}` must be an h, s or d register"
            ));
        }
    };
    let Some(Operand::RegArrangement { reg, arrangement }) = operands.get(1) else {
        return Err(format!(
            "{what}: the source must be a vector register `Vn.{want}`"
        ));
    };
    if arrangement != want {
        return Err(format!(
            "{what}: a `{name}` result reduces `Vn.{want}`, not `{reg}.{arrangement}`"
        ));
    }
    let rn = parse_reg_num(reg).ok_or_else(|| format!("{what}: bad vector register `{reg}`"))?;
    Ok(EncodeResult::Word(
        (0b01 << 30)
            | (u << 29)
            | (0b11110 << 24)
            | (o1 << 23)
            | (sz << 22)
            | (0b11000 << 17)
            | (opcode << 12)
            | (0b10 << 10)
            | (rn << 5)
            | rd,
    ))
}

/// A three-same FP instruction that exists both as a vector op and in the
/// Advanced-SIMD scalar class (`fabd v0.4s, ...` / `fabd s0, s1, s2`).
pub(crate) fn encode_neon_fp_three_same_any(
    operands: &[Operand],
    u_bit: u32,
    a: u32,
    opcode: u32,
    what: &str,
) -> Result<EncodeResult, String> {
    if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
        encode_neon_float_three_same(operands, u_bit, a, opcode)
    } else {
        encode_neon_fp_scalar_three_same(operands, u_bit, a, opcode, what)
    }
}

/// A two-misc FP instruction with both a vector and an Advanced-SIMD scalar
/// form (`frecpe v0.2d, v1.2d` / `frecpe d0, d1`).
pub(crate) fn encode_neon_fp_two_misc_any(
    operands: &[Operand],
    u_bit: u32,
    a: u32,
    opcode: u32,
    what: &str,
) -> Result<EncodeResult, String> {
    if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
        encode_neon_float_two_misc(operands, u_bit, a, opcode)
    } else {
        encode_neon_fp_scalar_two_misc(operands, u_bit, a, opcode, what)
    }
}

/// The two vector arrangements of operands `0..n`, checked against `want`.
fn vector_operands(operands: &[Operand], want: &[&str], what: &str) -> Result<Vec<u32>, String> {
    if operands.len() < want.len() {
        return Err(format!("{what}: expected {} vector operands", want.len()));
    }
    let mut regs = Vec::with_capacity(want.len());
    for (i, w) in want.iter().enumerate() {
        let Some(Operand::RegArrangement { reg, arrangement }) = operands.get(i) else {
            return Err(format!(
                "{what}: operand {i} must be a vector register `Vn.{w}`"
            ));
        };
        if arrangement != w {
            return Err(format!(
                "{what}: operand {i} must be `Vn.{w}`, got `{reg}.{arrangement}`"
            ));
        }
        regs.push(
            parse_reg_num(reg).ok_or_else(|| format!("{what}: bad vector register `{reg}`"))?,
        );
    }
    Ok(regs)
}

/// FCVTXN / FCVTXN2: double to single with round-to-odd.
///
/// Scalar `fcvtxn Sd, Dn` = `01 1 11110 0 1 10000 10110 10 Rn Rd` (0x7e616ba3
/// for `s3, d29`); vector `fcvtxn Vd.2s, Vn.2d` and `fcvtxn2 Vd.4s, Vn.2d` =
/// `0 Q 1 01110 0 1 10000 10110 10 Rn Rd` (0x2e616ba3 / 0x6e616ba3).
pub(crate) fn encode_fcvtxn(operands: &[Operand], high: bool) -> Result<EncodeResult, String> {
    let mn = if high { "fcvtxn2" } else { "fcvtxn" };
    if operands.len() != 2 {
        return Err(format!("{mn}: expected 2 operands, got {}", operands.len()));
    }
    if !high && matches!(operands.first(), Some(Operand::Reg(_))) {
        let (rd, ld, nd) = fp_reg(operands, 0, mn)?;
        let (rn, ln, nn) = fp_reg(operands, 1, mn)?;
        if ld != b's' || ln != b'd' {
            return Err(format!(
                "{mn}: the scalar form is `Sd, Dn`, got `{nd}, {nn}`"
            ));
        }
        return Ok(EncodeResult::Word(0x7e61_6800 | (rn << 5) | rd));
    }
    let r = vector_operands(operands, &[if high { "4s" } else { "2s" }, "2d"], mn)?;
    Ok(EncodeResult::Word(
        (u32::from(high) << 30) | 0x2e61_6800 | (r[1] << 5) | r[0],
    ))
}

/// BFCVTN / BFCVTN2 (FEAT_BF16): `0 Q 0 01110 10 10000 10110 10 Rn Rd` --
/// `bfcvtn v3.4h, v29.4s` = 0x0ea16ba3, `bfcvtn2 v3.8h, v29.4s` = 0x4ea16ba3.
pub(crate) fn encode_bfcvtn(operands: &[Operand], high: bool) -> Result<EncodeResult, String> {
    let mn = if high { "bfcvtn2" } else { "bfcvtn" };
    if operands.len() != 2 {
        return Err(format!("{mn}: expected 2 operands, got {}", operands.len()));
    }
    let r = vector_operands(operands, &[if high { "8h" } else { "4h" }, "4s"], mn)?;
    Ok(EncodeResult::Word(
        (u32::from(high) << 30) | 0x0ea1_6800 | (r[1] << 5) | r[0],
    ))
}

/// FRINT32Z / FRINT32X / FRINT64Z / FRINT64X (FEAT_FRINTTS): round to an
/// integral value that fits in 32 or 64 bits.  `x` selects the current
/// rounding mode (X) over toward-zero (Z); `wide` selects the 64-bit range.
///
/// Vector (`2s`/`4s`/`2d` only -- there is no FP16 form): two-misc with U=x,
/// opcode `1111:wide` (`frint32x v3.2s, v29.2s` = 0x2e21eba3).  Scalar (`s`/`d`
/// only): FP 1-source opcode `0100:wide:x` (`frint32x s3, s29` = 0x1e28c3a3,
/// `frint64z d3, d29` = 0x1e6943a3).
pub(crate) fn encode_frint_ts(
    operands: &[Operand],
    mn: &str,
    x: bool,
    wide: bool,
) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!("{mn}: expected 2 operands, got {}", operands.len()));
    }
    if let Some(Operand::RegArrangement { arrangement, .. }) = operands.first() {
        if !matches!(arrangement.as_str(), "2s" | "4s" | "2d") {
            return Err(format!("{mn}: no `{arrangement}` form (2s, 4s or 2d)"));
        }
        return encode_neon_float_two_misc(operands, u32::from(x), 0, 0b11110 | u32::from(wide));
    }
    let (_, letter) = fp_scalar_operands(operands, 2, mn)?;
    if letter == b'h' {
        return Err(format!("{mn}: there is no half-precision form"));
    }
    encode_fp_1src(operands, 0b010000 | (u32::from(wide) << 1) | u32::from(x))
}

/// URECPE / URSQRTE: unsigned 32-bit reciprocal (square-root) estimate,
/// `0 Q U 01110 1 0 10000 11100 10 Rn Rd`, `2s`/`4s` only (`urecpe v3.2s,
/// v29.2s` = 0x0ea1cba3, `ursqrte v3.4s, v29.4s` = 0x6ea1cba3).
pub(crate) fn encode_neon_urecpe(
    operands: &[Operand],
    u: u32,
    mn: &str,
) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!("{mn}: expected 2 operands, got {}", operands.len()));
    }
    let q = match operands.first() {
        Some(Operand::RegArrangement { arrangement, .. }) if arrangement == "2s" => 0u32,
        Some(Operand::RegArrangement { arrangement, .. }) if arrangement == "4s" => 1,
        other => {
            return Err(format!(
                "{mn}: the arrangement must be 2s or 4s, got {other:?}"
            ));
        }
    };
    let arr = if q == 1 { "4s" } else { "2s" };
    let r = vector_operands(operands, &[arr, arr], mn)?;
    Ok(EncodeResult::Word(
        (q << 30)
            | (u << 29)
            | (0b01110 << 24)
            | (0b10 << 22)
            | (0b10000 << 17)
            | (0b11100 << 12)
            | (0b10 << 10)
            | (r[1] << 5)
            | r[0],
    ))
}

/// The lane operand of a by-element form: `(register, index)`, checked for
/// element size `elem`.
fn lane_operand(
    operands: &[Operand],
    idx: usize,
    elem: &str,
    what: &str,
) -> Result<(u32, u32), String> {
    let Some(Operand::RegLane {
        reg,
        elem_size,
        index,
    }) = operands.get(idx)
    else {
        return Err(format!(
            "{what}: operand {idx} must be a vector element `Vm.{elem}[i]`"
        ));
    };
    if elem_size != elem {
        return Err(format!(
            "{what}: the element must be `.{elem}[i]`, got `{reg}.{elem_size}[{index}]`"
        ));
    }
    let num = parse_reg_num(reg).ok_or_else(|| format!("{what}: bad vector register `{reg}`"))?;
    Ok((num, *index))
}

/// FMLAL / FMLAL2 / FMLSL / FMLSL2 (FEAT_FHM): half-precision products
/// widened into single-precision accumulators.  `sub` picks FMLSL, `high`
/// the `2` forms (upper half of the half-precision sources).
///
/// Vector: `Vd.2s, Vn.2h, Vm.2h` (Q=0) or `Vd.4s, Vn.4h, Vm.4h` (Q=1);
/// FMLAL = 0x0e20ec00, FMLSL = 0x0ea0ec00, FMLAL2 = 0x2e20cc00, FMLSL2 =
/// 0x2ea0cc00, plus Q, Rm, Rn, Rd (`fmlal v3.2s, v29.2h, v17.2h` =
/// 0x0e31efa3).
///
/// By element: `0 Q U 01111 1 0 L M Rm opcode H 0 Rn Rd` with U = `high`,
/// opcode `high:0:sub:0` (FMLAL 0000, FMLSL 0100, FMLAL2 1000, FMLSL2 1100),
/// index `H:L:M` (0-7) and Rm restricted to v0-v15 (`fmlal v3.4s, v29.4h,
/// v15.h[7]` = 0x4fbf0ba3, `fmlsl2 v3.4s, v29.4h, v15.h[5]` = 0x6f9fcba3).
pub(crate) fn encode_fmlal(
    operands: &[Operand],
    sub: bool,
    high: bool,
) -> Result<EncodeResult, String> {
    let mn = match (sub, high) {
        (false, false) => "fmlal",
        (false, true) => "fmlal2",
        (true, false) => "fmlsl",
        (true, true) => "fmlsl2",
    };
    if operands.len() != 3 {
        return Err(format!("{mn}: expected 3 operands, got {}", operands.len()));
    }
    let q = match operands.first() {
        Some(Operand::RegArrangement { arrangement, .. }) if arrangement == "2s" => 0u32,
        Some(Operand::RegArrangement { arrangement, .. }) if arrangement == "4s" => 1,
        other => {
            return Err(format!(
                "{mn}: the accumulator must be `Vd.2s` or `Vd.4s`, got {other:?}"
            ));
        }
    };
    let (acc, src) = if q == 1 { ("4s", "4h") } else { ("2s", "2h") };
    if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
        let r = vector_operands(operands, &[acc, src], mn)?;
        let (rm, index) = lane_operand(operands, 2, "h", mn)?;
        if rm > 15 {
            return Err(format!(
                "{mn}: the indexed register must be v0-v15, got v{rm}"
            ));
        }
        if index > 7 {
            return Err(format!("{mn}: element index {index} out of range 0 to 7"));
        }
        let (h, l, m) = ((index >> 2) & 1, (index >> 1) & 1, index & 1);
        let opcode = (u32::from(high) << 3) | (u32::from(sub) << 2);
        return Ok(EncodeResult::Word(
            0x0f80_0000
                | (q << 30)
                | (u32::from(high) << 29)
                | (l << 21)
                | (m << 20)
                | (rm << 16)
                | (opcode << 12)
                | (h << 11)
                | (r[1] << 5)
                | r[0],
        ));
    }
    let r = vector_operands(operands, &[acc, src, src], mn)?;
    let base = match (sub, high) {
        (false, false) => 0x0e20_ec00u32,
        (true, false) => 0x0ea0_ec00,
        (false, true) => 0x2e20_cc00,
        (true, true) => 0x2ea0_cc00,
    };
    Ok(EncodeResult::Word(
        base | (q << 30) | (r[2] << 16) | (r[1] << 5) | r[0],
    ))
}

/// The (Q, size) of a complex-arithmetic arrangement: `4h`/`8h` (size 01),
/// `2s`/`4s` (10), `2d` (11).
fn complex_q_size(arrangement: &str) -> Option<(u32, u32)> {
    match arrangement {
        "4h" => Some((0, 0b01)),
        "8h" => Some((1, 0b01)),
        "2s" => Some((0, 0b10)),
        "4s" => Some((1, 0b10)),
        "2d" => Some((1, 0b11)),
        _ => None,
    }
}

/// The rotation immediate of FCMLA (`#0`/`#90`/`#180`/`#270` -> 0-3) or of
/// FCADD (`#90`/`#270` -> 0/1).
fn complex_rotation(op: Option<&Operand>, fcadd: bool, mn: &str) -> Result<u32, String> {
    let deg = match op {
        Some(Operand::Imm(v)) => *v,
        other => {
            return Err(format!(
                "{mn}: expected a rotation immediate, got {other:?}"
            ));
        }
    };
    match (fcadd, deg) {
        (false, 0) => Ok(0),
        (false, 90) => Ok(1),
        (false, 180) => Ok(2),
        (false, 270) => Ok(3),
        (true, 90) => Ok(0),
        (true, 270) => Ok(1),
        _ => Err(format!(
            "{mn}: rotation #{deg} is not encodable ({})",
            if fcadd {
                "#90 or #270"
            } else {
                "#0, #90, #180 or #270"
            }
        )),
    }
}

/// FCMLA (FEAT_FCMA): complex multiply-accumulate with rotation.
///
/// Vector: `0 Q 1 01110 size 0 Rm 110 rot 1 Rn Rd` (`fcmla v3.4s, v29.4s,
/// v17.4s, #0` = 0x6e91c7a3).  By element (`4h`, `8h`, `4s` only):
/// `0 Q 1 01111 size L M Rm 0 rot 1 H 0 Rn Rd` with Rm = M:Rm; the index is
/// `H:L` for half (0-1 for `4h`, 0-3 for `8h`) and `H` for single (0-1):
/// `fcmla v3.8h, v29.8h, v17.h[1], #0` = 0x6f7113a3, `fcmla v3.4s, v29.4s,
/// v31.s[0], #270` = 0x6f9f73a3.
pub(crate) fn encode_fcmla(operands: &[Operand]) -> Result<EncodeResult, String> {
    let mn = "fcmla";
    if operands.len() != 4 {
        return Err(format!("{mn}: expected 4 operands, got {}", operands.len()));
    }
    let rot = complex_rotation(operands.get(3), false, mn)?;
    let arr = match operands.first() {
        Some(Operand::RegArrangement { arrangement, .. }) => arrangement.clone(),
        other => {
            return Err(format!(
                "{mn}: expected a vector destination, got {other:?}"
            ));
        }
    };
    let (q, size) = complex_q_size(&arr).ok_or_else(|| format!("{mn}: no `{arr}` form"))?;
    if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
        let r = vector_operands(operands, &[&arr, &arr], mn)?;
        let (elem, max_index) = match arr.as_str() {
            "4h" => ("h", 1),
            "8h" => ("h", 3),
            "4s" => ("s", 1),
            _ => {
                return Err(format!(
                    "{mn}: the by-element form has no `{arr}` arrangement (4h, 8h or 4s)"
                ));
            }
        };
        let (rm, index) = lane_operand(operands, 2, elem, mn)?;
        if index > max_index {
            return Err(format!(
                "{mn}: element index {index} out of range 0 to {max_index}"
            ));
        }
        let (h, l) = if elem == "h" {
            ((index >> 1) & 1, index & 1)
        } else {
            (index & 1, 0)
        };
        return Ok(EncodeResult::Word(
            (q << 30)
                | (1 << 29)
                | (0b01111 << 24)
                | (size << 22)
                | (l << 21)
                | (rm << 16)
                | (rot << 13)
                | (1 << 12)
                | (h << 11)
                | (r[1] << 5)
                | r[0],
        ));
    }
    let r = vector_operands(operands, &[&arr, &arr, &arr], mn)?;
    Ok(EncodeResult::Word(
        (q << 30)
            | (1 << 29)
            | (0b01110 << 24)
            | (size << 22)
            | (r[2] << 16)
            | (0b110 << 13)
            | (rot << 11)
            | (1 << 10)
            | (r[1] << 5)
            | r[0],
    ))
}

/// FCADD (FEAT_FCMA): complex add with rotation `#90`/`#270`,
/// `0 Q 1 01110 size 0 Rm 111 rot 0 1 Rn Rd` (`fcadd v3.4s, v29.4s, v17.4s,
/// #90` = 0x6e91e7a3, `fcadd v3.2d, v29.2d, v17.2d, #270` = 0x6ed1f7a3).
pub(crate) fn encode_fcadd(operands: &[Operand]) -> Result<EncodeResult, String> {
    let mn = "fcadd";
    if operands.len() != 4 {
        return Err(format!("{mn}: expected 4 operands, got {}", operands.len()));
    }
    let rot = complex_rotation(operands.get(3), true, mn)?;
    let arr = match operands.first() {
        Some(Operand::RegArrangement { arrangement, .. }) => arrangement.clone(),
        other => {
            return Err(format!(
                "{mn}: expected a vector destination, got {other:?}"
            ));
        }
    };
    let (q, size) = complex_q_size(&arr).ok_or_else(|| format!("{mn}: no `{arr}` form"))?;
    let r = vector_operands(operands, &[&arr, &arr, &arr], mn)?;
    Ok(EncodeResult::Word(
        (q << 30)
            | (1 << 29)
            | (0b01110 << 24)
            | (size << 22)
            | (r[2] << 16)
            | (0b111 << 13)
            | (rot << 12)
            | (1 << 10)
            | (r[1] << 5)
            | r[0],
    ))
}

/// BFDOT (FEAT_BF16): BFloat16 pair dot product into single.
///
/// Vector `Vd.2s, Vn.4h, Vm.4h` / `Vd.4s, Vn.8h, Vm.8h` = 0x2e40fc00 | Q
/// (`bfdot v3.2s, v29.4h, v17.4h` = 0x2e51ffa3); by element `Vm.2h[i]`
/// (index `H:L`, 0-3; Rm = M:Rm) = 0x0f40f000 | Q (`bfdot v3.4s, v29.8h,
/// v17.2h[1]` = 0x4f71f3a3).
pub(crate) fn encode_bfdot(operands: &[Operand]) -> Result<EncodeResult, String> {
    let mn = "bfdot";
    if operands.len() != 3 {
        return Err(format!("{mn}: expected 3 operands, got {}", operands.len()));
    }
    let q = match operands.first() {
        Some(Operand::RegArrangement { arrangement, .. }) if arrangement == "2s" => 0u32,
        Some(Operand::RegArrangement { arrangement, .. }) if arrangement == "4s" => 1,
        other => {
            return Err(format!(
                "{mn}: the accumulator must be `Vd.2s` or `Vd.4s`, got {other:?}"
            ));
        }
    };
    let (acc, src) = if q == 1 { ("4s", "8h") } else { ("2s", "4h") };
    if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
        let r = vector_operands(operands, &[acc, src], mn)?;
        let (rm, index) = lane_operand(operands, 2, "2h", mn)?;
        if index > 3 {
            return Err(format!("{mn}: element index {index} out of range 0 to 3"));
        }
        let (h, l) = ((index >> 1) & 1, index & 1);
        return Ok(EncodeResult::Word(
            0x0f40_f000 | (q << 30) | (l << 21) | (rm << 16) | (h << 11) | (r[1] << 5) | r[0],
        ));
    }
    let r = vector_operands(operands, &[acc, src, src], mn)?;
    Ok(EncodeResult::Word(
        0x2e40_fc00 | (q << 30) | (r[2] << 16) | (r[1] << 5) | r[0],
    ))
}

/// BFMMLA (FEAT_BF16) `Vd.4s, Vn.8h, Vm.8h` = 0x6e40ec00 | Rm<<16 | Rn<<5 | Rd
/// (`bfmmla v3.4s, v29.8h, v17.8h` = 0x6e51efa3).
pub(crate) fn encode_bfmmla(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 3 {
        return Err(format!(
            "bfmmla: expected 3 operands, got {}",
            operands.len()
        ));
    }
    let r = vector_operands(operands, &["4s", "8h", "8h"], "bfmmla")?;
    Ok(EncodeResult::Word(
        0x6e40_ec00 | (r[2] << 16) | (r[1] << 5) | r[0],
    ))
}

/// BFMLALB / BFMLALT (FEAT_BF16): even (B) or odd (T) BFloat16 elements
/// widened and multiply-accumulated into single, `Vd.4s, Vn.8h, Vm.8h` =
/// 0x2ec0fc00 | top<<30 (`bfmlalb v3.4s, v29.8h, v17.8h` = 0x2ed1ffa3).  By
/// element `Vm.h[i]` (index `H:L:M`, 0-7, Rm v0-v15) = 0x0fc0f000 | top<<30
/// (`bfmlalt v3.4s, v29.8h, v15.h[7]` = 0x4ffffba3).
pub(crate) fn encode_bfmlal(operands: &[Operand], top: bool) -> Result<EncodeResult, String> {
    let mn = if top { "bfmlalt" } else { "bfmlalb" };
    if operands.len() != 3 {
        return Err(format!("{mn}: expected 3 operands, got {}", operands.len()));
    }
    let q = u32::from(top);
    if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
        let r = vector_operands(operands, &["4s", "8h"], mn)?;
        let (rm, index) = lane_operand(operands, 2, "h", mn)?;
        if rm > 15 {
            return Err(format!(
                "{mn}: the indexed register must be v0-v15, got v{rm}"
            ));
        }
        if index > 7 {
            return Err(format!("{mn}: element index {index} out of range 0 to 7"));
        }
        let (h, l, m) = ((index >> 2) & 1, (index >> 1) & 1, index & 1);
        return Ok(EncodeResult::Word(
            0x0fc0_f000
                | (q << 30)
                | (l << 21)
                | (m << 20)
                | (rm << 16)
                | (h << 11)
                | (r[1] << 5)
                | r[0],
        ));
    }
    let r = vector_operands(operands, &["4s", "8h", "8h"], mn)?;
    Ok(EncodeResult::Word(
        0x2ec0_fc00 | (q << 30) | (r[2] << 16) | (r[1] << 5) | r[0],
    ))
}
