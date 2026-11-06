//! Advanced-SIMD integer forms outside the vector leaves in `neon.rs`: the
//! Advanced-SIMD *scalar* integer classes (`ushr d0, d0, #32`, `cmeq d0, d1,
//! #0`, `sqadd b0, b1, b2`, `sqxtn s0, d1`, `movi d0, #0`, ...), the dot
//! products and matrix multiplies (FEAT_DotProd, FEAT_I8MM), rounding doubling
//! multiply-accumulate (FEAT_RDM), SQSHLU, SUQADD/USQADD, ORR/BIC (vector,
//! immediate) and the SHA1/SHA2/SHA512/SHA3/SM3/SM4 cryptographic extensions.
//!
//! Every word was measured against GNU as 2.47 (`.arch
//! armv9.4-a+sme+aes+sha3+sm4`), and every operand rule mirrors a GAS rejection.
//! These forms are what GCC and Clang emit (`movi d0, #0` zeroes an FP
//! register; `ushr d0, d0, #32` is a 64-bit shift kept in the vector file) and
//! what hand-written NEON and the BoringSSL/OpenSSL `.S` files use; each was a
//! hard "unsupported instruction" error before.

use super::*;
use crate::backend::arm::assembler::parser::Operand;

/// Bit set of the scalar element sizes a form accepts: `B`=1, `H`=2, `S`=4,
/// `D`=8 (bit `size`).
const SZ_B: u8 = 1;
const SZ_H: u8 = 2;
const SZ_S: u8 = 4;
const SZ_D: u8 = 8;
const SZ_HS: u8 = SZ_H | SZ_S;
const SZ_BHS: u8 = SZ_B | SZ_H | SZ_S;
const SZ_ALL: u8 = SZ_B | SZ_H | SZ_S | SZ_D;

/// Whether `name` spells a scalar SIMD&FP register `b0-b31`/`h`/`s`/`d`.
fn is_simd_scalar_name(name: &str) -> bool {
    matches!(
        name.as_bytes().first(),
        Some(b'b' | b'h' | b's' | b'd' | b'B' | b'H' | b'S' | b'D')
    ) && is_fp_spelling_pub(name)
}

/// Operand `idx` as a scalar SIMD register: `(number, size)` with size 0-3
/// for `b`/`h`/`s`/`d`.
fn simd_scalar(operands: &[Operand], idx: usize, what: &str) -> Result<(u32, u32), String> {
    let (num, letter, name) = fp_reg(operands, idx, what)?;
    let size = match letter {
        b'b' => 0,
        b'h' => 1,
        b's' => 2,
        b'd' => 3,
        _ => {
            return Err(format!(
                "{what}: operand {idx} `{name}` must be a b, h, s or d register"
            ));
        }
    };
    Ok((num, size))
}

fn size_letter(size: u32) -> char {
    ['b', 'h', 's', 'd'][size as usize & 3]
}

fn check_size(size: u32, allowed: u8, what: &str) -> Result<(), String> {
    if allowed & (1 << size) == 0 {
        let list: Vec<String> = (0..4u32)
            .filter(|s| allowed & (1 << s) != 0)
            .map(|s| size_letter(s).to_string())
            .collect();
        return Err(format!(
            "{what}: there is no `{}`-register form (the scalar form takes {})",
            size_letter(size),
            list.join("/")
        ));
    }
    Ok(())
}

/// Every operand in `idxs` is a scalar of the same size; returns the numbers
/// and that size.
fn same_scalars(
    operands: &[Operand],
    n: usize,
    allowed: u8,
    what: &str,
) -> Result<(Vec<u32>, u32), String> {
    if operands.len() != n {
        return Err(format!(
            "{what}: expected {n} operands, got {}",
            operands.len()
        ));
    }
    let mut regs = Vec::with_capacity(n);
    let mut size = 0;
    for i in 0..n {
        let (num, s) = simd_scalar(operands, i, what)?;
        if i == 0 {
            size = s;
        } else if s != size {
            return Err(format!(
                "{what}: `{}` and `{}` registers differ in size; the scalar form has one size field",
                size_letter(size),
                size_letter(s)
            ));
        }
        regs.push(num);
    }
    check_size(size, allowed, what)?;
    Ok((regs, size))
}

fn imm_operand(operands: &[Operand], idx: usize, what: &str) -> Result<i64, String> {
    match operands.get(idx) {
        Some(Operand::Imm(v)) => Ok(*v),
        other => Err(format!(
            "{what}: expected an immediate at operand {idx}, got {other:?}"
        )),
    }
}

/// `01 U 11110 size 1 Rm opcode 1 Rn Rd` (`sqadd b3, b29, b17`, `cmgt d0, d1,
/// d2` = 0x5ee23420).
fn scalar_three_same(
    operands: &[Operand],
    u: u32,
    opcode: u32,
    allowed: u8,
    what: &str,
) -> Result<EncodeResult, String> {
    let (r, size) = same_scalars(operands, 3, allowed, what)?;
    Ok(EncodeResult::Word(
        0x5e20_0400 | (u << 29) | (size << 22) | (r[2] << 16) | (opcode << 11) | (r[1] << 5) | r[0],
    ))
}

/// `01 U 11110 size 10000 opcode 10 Rn Rd` (`suqadd h3, h29` = 0x5e603ba3).
fn scalar_two_misc(
    operands: &[Operand],
    u: u32,
    opcode: u32,
    allowed: u8,
    what: &str,
) -> Result<EncodeResult, String> {
    let (r, size) = same_scalars(operands, 2, allowed, what)?;
    Ok(EncodeResult::Word(
        0x5e20_0800 | (u << 29) | (size << 22) | (opcode << 12) | (r[1] << 5) | r[0],
    ))
}

/// Compare against zero, `d` only: two-misc with a `#0` third operand
/// (`cmeq d0, d1, #0` = 0x5ee09820).
fn scalar_cmp_zero(
    operands: &[Operand],
    u: u32,
    opcode: u32,
    what: &str,
) -> Result<EncodeResult, String> {
    if operands.len() != 3 || !matches!(operands.get(2), Some(Operand::Imm(0))) {
        return Err(format!(
            "{what}: the compare-with-zero form is `{what} Dd, Dn, #0`"
        ));
    }
    scalar_two_misc(&operands[..2], u, opcode, SZ_D, what)
}

/// Saturating extract-narrow: `Vd` one size below `Vn`, size field = the
/// destination's (`sqxtn s3, d29`).
fn scalar_narrow(
    operands: &[Operand],
    u: u32,
    opcode: u32,
    what: &str,
) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "{what}: expected 2 operands, got {}",
            operands.len()
        ));
    }
    let (rd, sd) = simd_scalar(operands, 0, what)?;
    let (rn, sn) = simd_scalar(operands, 1, what)?;
    check_size(sd, SZ_BHS, what)?;
    if sn != sd + 1 {
        return Err(format!(
            "{what}: a `{}` result narrows a `{}` source, not `{}`",
            size_letter(sd),
            size_letter(sd + 1),
            size_letter(sn)
        ));
    }
    Ok(EncodeResult::Word(
        0x5e20_0800 | (u << 29) | (sd << 22) | (opcode << 12) | (rn << 5) | rd,
    ))
}

/// Shift by immediate, `01 U 111110 immh:immb opcode 1 Rn Rd`.  Left shifts
/// encode `esize + shift` (0 <= shift < esize); right shifts `2*esize - shift`
/// (1 <= shift <= esize).  `ushr d0, d0, #32` = 0x7f600400, `shl d0, d1, #1`
/// = 0x5f415420, `sqshlu b3, b29, #7` = 0x7f0f67a3.
fn scalar_shift_imm(
    operands: &[Operand],
    u: u32,
    opcode: u32,
    allowed: u8,
    right: bool,
    what: &str,
) -> Result<EncodeResult, String> {
    if operands.len() != 3 {
        return Err(format!(
            "{what}: expected 3 operands, got {}",
            operands.len()
        ));
    }
    let (r, size) = same_scalars(&operands[..2], 2, allowed, what)?;
    let shift = imm_operand(operands, 2, what)?;
    let esize = 8i64 << size;
    let immhb = if right {
        if !(1..=esize).contains(&shift) {
            return Err(format!("{what}: shift #{shift} out of range 1 to {esize}"));
        }
        2 * esize - shift
    } else {
        if !(0..esize).contains(&shift) {
            return Err(format!(
                "{what}: shift #{shift} out of range 0 to {}",
                esize - 1
            ));
        }
        esize + shift
    };
    Ok(EncodeResult::Word(
        0x5f00_0400 | (u << 29) | ((immhb as u32) << 16) | (opcode << 11) | (r[1] << 5) | r[0],
    ))
}

/// Saturating shift-right-narrow: `Vd` one size below `Vn`, shift 1 to the
/// destination element size, `immh:immb = 2*esize(Vd) - shift`.
fn scalar_shift_right_narrow(
    operands: &[Operand],
    u: u32,
    opcode: u32,
    what: &str,
) -> Result<EncodeResult, String> {
    if operands.len() != 3 {
        return Err(format!(
            "{what}: expected 3 operands, got {}",
            operands.len()
        ));
    }
    let (rd, sd) = simd_scalar(operands, 0, what)?;
    let (rn, sn) = simd_scalar(operands, 1, what)?;
    check_size(sd, SZ_BHS, what)?;
    if sn != sd + 1 {
        return Err(format!(
            "{what}: a `{}` result narrows a `{}` source, not `{}`",
            size_letter(sd),
            size_letter(sd + 1),
            size_letter(sn)
        ));
    }
    let shift = imm_operand(operands, 2, what)?;
    let esize = 8i64 << sd;
    if !(1..=esize).contains(&shift) {
        return Err(format!("{what}: shift #{shift} out of range 1 to {esize}"));
    }
    let immhb = (2 * esize - shift) as u32;
    Ok(EncodeResult::Word(
        0x5f00_0400 | (u << 29) | (immhb << 16) | (opcode << 11) | (rn << 5) | rd,
    ))
}

/// Signed saturating doubling multiply long, scalar three-different:
/// `01 0 11110 size 1 Rm opcode 00 Rn Rd`, size = the source's (`h` -> `s`
/// result, `s` -> `d`); `sqdmull d3, s29, s17` = 0x5eb1d3a3.
fn scalar_three_diff(
    operands: &[Operand],
    opcode: u32,
    what: &str,
) -> Result<EncodeResult, String> {
    if operands.len() != 3 {
        return Err(format!(
            "{what}: expected 3 operands, got {}",
            operands.len()
        ));
    }
    let (rd, sd) = simd_scalar(operands, 0, what)?;
    let (r, sn) = same_scalars(&operands[1..], 2, SZ_HS, what)?;
    if sd != sn + 1 {
        return Err(format!(
            "{what}: `{}` sources widen into a `{}` result, not `{}`",
            size_letter(sn),
            size_letter(sn + 1),
            size_letter(sd)
        ));
    }
    Ok(EncodeResult::Word(
        0x5e20_0000 | (sn << 22) | (r[1] << 16) | (opcode << 12) | (r[0] << 5) | rd,
    ))
}

/// The `(Rm field, H, L, M)` of a by-element operand of element size `size`
/// (1 = `h`: index `H:L:M`, Rm v0-v15; 2 = `s`: index `H:L`, Rm = `M:Rm`).
fn elem_index_fields(
    operands: &[Operand],
    idx: usize,
    size: u32,
    what: &str,
) -> Result<(u32, u32, u32, u32), String> {
    let Some(Operand::RegLane {
        reg,
        elem_size,
        index,
    }) = operands.get(idx)
    else {
        return Err(format!(
            "{what}: operand {idx} must be a vector element `Vm.T[i]`"
        ));
    };
    let want = if size == 1 { "h" } else { "s" };
    if elem_size != want {
        return Err(format!(
            "{what}: the element must be `.{want}[i]`, got `{reg}.{elem_size}[{index}]`"
        ));
    }
    let rm = parse_reg_num(reg).ok_or_else(|| format!("{what}: bad vector register `{reg}`"))?;
    if size == 1 {
        if rm > 15 {
            return Err(format!(
                "{what}: a `.h` element register must be v0-v15, got v{rm}"
            ));
        }
        if *index > 7 {
            return Err(format!("{what}: element index {index} out of range 0 to 7"));
        }
        Ok((rm, (index >> 2) & 1, (index >> 1) & 1, index & 1))
    } else {
        if *index > 3 {
            return Err(format!("{what}: element index {index} out of range 0 to 3"));
        }
        Ok((rm & 0xF, (index >> 1) & 1, index & 1, rm >> 4))
    }
}

/// Scalar by element, `01 U 11111 size L M Rm opcode H 0 Rn Rd`.  `long`
/// forms (SQDMULL/SQDMLAL/SQDMLSL) take a destination one size above the
/// source; the others are same-size `h`/`s`.  `sqdmull s3, h29, v15.h[7]` =
/// 0x5f7fbba3, `sqrdmlah h3, h29, v15.h[7]` = 0x7f7fdba3.
fn scalar_by_element(
    operands: &[Operand],
    u: u32,
    opcode: u32,
    long: bool,
    what: &str,
) -> Result<EncodeResult, String> {
    if operands.len() != 3 {
        return Err(format!(
            "{what}: expected 3 operands, got {}",
            operands.len()
        ));
    }
    let (rd, sd) = simd_scalar(operands, 0, what)?;
    let (rn, sn) = simd_scalar(operands, 1, what)?;
    check_size(sn, SZ_HS, what)?;
    let want_d = if long { sn + 1 } else { sn };
    if sd != want_d {
        return Err(format!(
            "{what}: a `{}` source gives a `{}` result, not `{}`",
            size_letter(sn),
            size_letter(want_d),
            size_letter(sd)
        ));
    }
    let (rm, h, l, m) = elem_index_fields(operands, 2, sn, what)?;
    Ok(EncodeResult::Word(
        0x5f00_0000
            | (u << 29)
            | (sn << 22)
            | (l << 21)
            | (m << 20)
            | (rm << 16)
            | (opcode << 12)
            | (h << 11)
            | (rn << 5)
            | rd,
    ))
}

/// DUP (element) into a scalar, the canonical spelling of `mov Bd, Vn.b[i]`:
/// `01 0 11110000 imm5 0 0000 1 Rn Rd`, imm5 = `index:1:0...` by size
/// (`dup b3, v29.b[3]` = 0x5e0707a3).
fn scalar_dup(operands: &[Operand], what: &str) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "{what}: expected 2 operands, got {}",
            operands.len()
        ));
    }
    let (rd, size) = simd_scalar(operands, 0, what)?;
    let Some(Operand::RegLane {
        reg,
        elem_size,
        index,
    }) = operands.get(1)
    else {
        return Err(format!(
            "{what}: the source must be a vector element `Vn.T[i]`"
        ));
    };
    let letter = size_letter(size).to_string();
    if *elem_size != letter {
        return Err(format!(
            "{what}: a `{letter}` result takes a `.{letter}[i]` element, got `.{elem_size}`"
        ));
    }
    let max = (16u32 >> size) - 1;
    if *index > max {
        return Err(format!(
            "{what}: element index {index} out of range 0 to {max}"
        ));
    }
    let rn = parse_reg_num(reg).ok_or_else(|| format!("{what}: bad vector register `{reg}`"))?;
    let imm5 = (index << (size + 1)) | (1 << size);
    Ok(EncodeResult::Word(
        0x5e00_0400 | (imm5 << 16) | (rn << 5) | rd,
    ))
}

/// MOVI `Dd, #imm`: the 64-bit byte-mask immediate (each byte 0x00 or 0xFF)
/// with op=1, cmode=1110, Q=0 -- `movi d0, #0` = 0x2f00e400, the zero idiom
/// GCC emits for `double x = 0.0`.
fn scalar_movi(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "movi: expected 2 operands (Dd, #imm), got {}",
            operands.len()
        ));
    }
    let (rd, size) = simd_scalar(operands, 0, "movi")?;
    if size != 3 {
        return Err("movi: the only scalar form is `movi Dd, #imm64`".to_string());
    }
    let imm = imm_operand(operands, 1, "movi")? as u64;
    let mut imm8 = 0u32;
    for i in 0..8 {
        match (imm >> (i * 8)) & 0xFF {
            0x00 => {}
            0xFF => imm8 |= 1 << i,
            other => {
                return Err(format!(
                    "movi Dd: each byte of the immediate must be 0x00 or 0xFF, got 0x{other:02x} in byte {i}"
                ));
            }
        }
    }
    Ok(EncodeResult::Word(
        0x2f00_e400 | ((imm8 >> 5) << 16) | ((imm8 & 0x1F) << 5) | rd,
    ))
}

/// The Advanced-SIMD scalar integer forms, chosen when operand 0 is a scalar
/// SIMD register (`b`/`h`/`s`/`d`).  Returns `None` for every other operand
/// shape and mnemonic so the vector and general-register arms keep them.
pub(crate) fn encode_advsimd_scalar_integer(
    mn: &str,
    operands: &[Operand],
) -> Option<Result<EncodeResult, String>> {
    if !matches!(operands.first(), Some(Operand::Reg(n)) if is_simd_scalar_name(n)) {
        return None;
    }
    let op2_imm = matches!(operands.get(2), Some(Operand::Imm(_)));
    let op2_lane = matches!(operands.get(2), Some(Operand::RegLane { .. }));
    Some(match mn {
        "cmeq" if op2_imm => scalar_cmp_zero(operands, 0, 0b01001, mn),
        "cmge" if op2_imm => scalar_cmp_zero(operands, 1, 0b01000, mn),
        "cmgt" if op2_imm => scalar_cmp_zero(operands, 0, 0b01000, mn),
        "cmle" => scalar_cmp_zero(operands, 1, 0b01001, mn),
        "cmlt" => scalar_cmp_zero(operands, 0, 0b01010, mn),
        "cmeq" => scalar_three_same(operands, 1, 0b10001, SZ_D, mn),
        "cmge" => scalar_three_same(operands, 0, 0b00111, SZ_D, mn),
        "cmgt" => scalar_three_same(operands, 0, 0b00110, SZ_D, mn),
        "cmhi" => scalar_three_same(operands, 1, 0b00110, SZ_D, mn),
        "cmhs" => scalar_three_same(operands, 1, 0b00111, SZ_D, mn),
        "cmtst" => scalar_three_same(operands, 0, 0b10001, SZ_D, mn),
        "sshl" => scalar_three_same(operands, 0, 0b01000, SZ_D, mn),
        "ushl" => scalar_three_same(operands, 1, 0b01000, SZ_D, mn),
        "srshl" => scalar_three_same(operands, 0, 0b01010, SZ_D, mn),
        "urshl" => scalar_three_same(operands, 1, 0b01010, SZ_D, mn),
        "sqadd" => scalar_three_same(operands, 0, 0b00001, SZ_ALL, mn),
        "uqadd" => scalar_three_same(operands, 1, 0b00001, SZ_ALL, mn),
        "sqsub" => scalar_three_same(operands, 0, 0b00101, SZ_ALL, mn),
        "uqsub" => scalar_three_same(operands, 1, 0b00101, SZ_ALL, mn),
        "sqshl" if op2_imm => scalar_shift_imm(operands, 0, 0b01110, SZ_ALL, false, mn),
        "uqshl" if op2_imm => scalar_shift_imm(operands, 1, 0b01110, SZ_ALL, false, mn),
        "sqshl" => scalar_three_same(operands, 0, 0b01001, SZ_ALL, mn),
        "uqshl" => scalar_three_same(operands, 1, 0b01001, SZ_ALL, mn),
        "sqrshl" => scalar_three_same(operands, 0, 0b01011, SZ_ALL, mn),
        "uqrshl" => scalar_three_same(operands, 1, 0b01011, SZ_ALL, mn),
        "sqdmulh" if op2_lane => scalar_by_element(operands, 0, 0b1100, false, mn),
        "sqrdmulh" if op2_lane => scalar_by_element(operands, 0, 0b1101, false, mn),
        "sqrdmlah" if op2_lane => scalar_by_element(operands, 1, 0b1101, false, mn),
        "sqrdmlsh" if op2_lane => scalar_by_element(operands, 1, 0b1111, false, mn),
        "sqdmull" if op2_lane => scalar_by_element(operands, 0, 0b1011, true, mn),
        "sqdmlal" if op2_lane => scalar_by_element(operands, 0, 0b0011, true, mn),
        "sqdmlsl" if op2_lane => scalar_by_element(operands, 0, 0b0111, true, mn),
        "sqdmulh" => scalar_three_same(operands, 0, 0b10110, SZ_HS, mn),
        "sqrdmulh" => scalar_three_same(operands, 1, 0b10110, SZ_HS, mn),
        // FEAT_RDM scalar three-same (extra): `01 1 11110 size 0 Rm 1000 S 1
        // Rn Rd`, S selecting SQRDMLSH (`sqrdmlsh s3, s29, s17` = 0x7e918fa3).
        "sqrdmlah" | "sqrdmlsh" => same_scalars(operands, 3, SZ_HS, mn).map(|(r, size)| {
            EncodeResult::Word(
                0x7e00_8400
                    | (size << 22)
                    | (r[2] << 16)
                    | (u32::from(mn == "sqrdmlsh") << 11)
                    | (r[1] << 5)
                    | r[0],
            )
        }),
        "sqdmull" => scalar_three_diff(operands, 0b1101, mn),
        "sqdmlal" => scalar_three_diff(operands, 0b1001, mn),
        "sqdmlsl" => scalar_three_diff(operands, 0b1011, mn),
        "abs" => scalar_two_misc(operands, 0, 0b01011, SZ_D, mn),
        "neg" => scalar_two_misc(operands, 1, 0b01011, SZ_D, mn),
        "sqabs" => scalar_two_misc(operands, 0, 0b00111, SZ_ALL, mn),
        "sqneg" => scalar_two_misc(operands, 1, 0b00111, SZ_ALL, mn),
        "suqadd" => scalar_two_misc(operands, 0, 0b00011, SZ_ALL, mn),
        "usqadd" => scalar_two_misc(operands, 1, 0b00011, SZ_ALL, mn),
        "sqxtn" => scalar_narrow(operands, 0, 0b10100, mn),
        "uqxtn" => scalar_narrow(operands, 1, 0b10100, mn),
        "sqxtun" => scalar_narrow(operands, 1, 0b10010, mn),
        "shl" => scalar_shift_imm(operands, 0, 0b01010, SZ_D, false, mn),
        "sli" => scalar_shift_imm(operands, 1, 0b01010, SZ_D, false, mn),
        "sqshlu" => scalar_shift_imm(operands, 1, 0b01100, SZ_ALL, false, mn),
        "sshr" => scalar_shift_imm(operands, 0, 0b00000, SZ_D, true, mn),
        "ushr" => scalar_shift_imm(operands, 1, 0b00000, SZ_D, true, mn),
        "ssra" => scalar_shift_imm(operands, 0, 0b00010, SZ_D, true, mn),
        "usra" => scalar_shift_imm(operands, 1, 0b00010, SZ_D, true, mn),
        "srshr" => scalar_shift_imm(operands, 0, 0b00100, SZ_D, true, mn),
        "urshr" => scalar_shift_imm(operands, 1, 0b00100, SZ_D, true, mn),
        "srsra" => scalar_shift_imm(operands, 0, 0b00110, SZ_D, true, mn),
        "ursra" => scalar_shift_imm(operands, 1, 0b00110, SZ_D, true, mn),
        "sri" => scalar_shift_imm(operands, 1, 0b01000, SZ_D, true, mn),
        "sqshrn" => scalar_shift_right_narrow(operands, 0, 0b10010, mn),
        "uqshrn" => scalar_shift_right_narrow(operands, 1, 0b10010, mn),
        "sqrshrn" => scalar_shift_right_narrow(operands, 0, 0b10011, mn),
        "uqrshrn" => scalar_shift_right_narrow(operands, 1, 0b10011, mn),
        "sqshrun" => scalar_shift_right_narrow(operands, 1, 0b10000, mn),
        "sqrshrun" => scalar_shift_right_narrow(operands, 1, 0b10001, mn),
        "dup" => scalar_dup(operands, mn),
        "movi" => scalar_movi(operands),
        _ => return None,
    })
}

/// The Q bit of a vector operand from its arrangement, checked against the
/// `(Q=0, Q=1)` pair a form accepts.
fn q_of(operands: &[Operand], idx: usize, pair: (&str, &str), what: &str) -> Result<u32, String> {
    match operands.get(idx) {
        Some(Operand::RegArrangement { arrangement, .. }) if arrangement == pair.0 => Ok(0),
        Some(Operand::RegArrangement { arrangement, .. }) if arrangement == pair.1 => Ok(1),
        other => Err(format!(
            "{what}: operand {idx} must be `.{}` or `.{}`, got {other:?}",
            pair.0, pair.1
        )),
    }
}

/// Vector operands with the arrangements in `want`.
fn vregs(operands: &[Operand], want: &[&str], what: &str) -> Result<Vec<u32>, String> {
    if operands.len() < want.len() {
        return Err(format!("{what}: expected {} vector operands", want.len()));
    }
    want.iter()
        .enumerate()
        .map(|(i, w)| match operands.get(i) {
            Some(Operand::RegArrangement { reg, arrangement }) if arrangement == w => {
                parse_reg_num(reg).ok_or_else(|| format!("{what}: bad vector register `{reg}`"))
            }
            other => Err(format!(
                "{what}: operand {i} must be `Vn.{w}`, got {other:?}"
            )),
        })
        .collect()
}

/// SDOT / UDOT (FEAT_DotProd) and USDOT / SUDOT (FEAT_I8MM): four 8-bit
/// products accumulated into each 32-bit lane.
///
/// Vector: `0 Q U 01110 10 0 Rm 1 0010 1 Rn Rd` for SDOT/UDOT (`sdot v3.2s,
/// v29.8b, v17.8b` = 0x0e9197a3), `0 Q 0 01110 10 0 Rm 1 0011 1 Rn Rd` for
/// USDOT (0x4e919fa3).  By element `Vm.4b[i]` (index `H:L`, Rm = `M:Rm`):
/// `0 Q U 01111 10 L M Rm 1110 H 0 Rn Rd` for SDOT/UDOT (`sdot v3.4s, v29.16b,
/// v17.4b[3]` = 0x4fb1eba3), `0 Q 0 01111 sz L M Rm 1111 H 0 Rn Rd` with sz=10
/// for USDOT (0x0f91fba3) and sz=00 for SUDOT (0x4f31f3a3).  SUDOT has no
/// vector form.
pub(crate) fn encode_neon_dot(operands: &[Operand], mn: &str) -> Result<EncodeResult, String> {
    if operands.len() != 3 {
        return Err(format!("{mn}: expected 3 operands, got {}", operands.len()));
    }
    let q = q_of(operands, 0, ("2s", "4s"), mn)?;
    let src = if q == 1 { "16b" } else { "8b" };
    let acc = if q == 1 { "4s" } else { "2s" };
    if let Some(Operand::RegLane {
        reg,
        elem_size,
        index,
    }) = operands.get(2)
    {
        let r = vregs(operands, &[acc, src], mn)?;
        if elem_size != "4b" {
            return Err(format!(
                "{mn}: the indexed operand is `Vm.4b[i]`, got `{reg}.{elem_size}[{index}]`"
            ));
        }
        if *index > 3 {
            return Err(format!("{mn}: element index {index} out of range 0 to 3"));
        }
        let rm = parse_reg_num(reg).ok_or_else(|| format!("{mn}: bad vector register `{reg}`"))?;
        let (h, l) = ((index >> 1) & 1, index & 1);
        let base = match mn {
            "sdot" => 0x0f80_e000u32,
            "udot" => 0x2f80_e000,
            "usdot" => 0x0f80_f000,
            _ => 0x0f00_f000, // sudot
        };
        return Ok(EncodeResult::Word(
            base | (q << 30) | (l << 21) | (rm << 16) | (h << 11) | (r[1] << 5) | r[0],
        ));
    }
    let base = match mn {
        "sdot" => 0x0e80_9400u32,
        "udot" => 0x2e80_9400,
        "usdot" => 0x0e80_9c00,
        _ => {
            return Err(format!(
                "{mn}: there is only the by-element form `{mn} Vd.T, Vn.T, Vm.4b[i]`"
            ));
        }
    };
    let r = vregs(operands, &[acc, src, src], mn)?;
    Ok(EncodeResult::Word(
        base | (q << 30) | (r[2] << 16) | (r[1] << 5) | r[0],
    ))
}

/// SMMLA / UMMLA / USMMLA (FEAT_I8MM) `Vd.4s, Vn.16b, Vm.16b`:
/// 0x4e80a400 / 0x6e80a400 / 0x4e80ac00 | Rm<<16 | Rn<<5 | Rd (`smmla v3.4s,
/// v29.16b, v17.16b` = 0x4e91a7a3).
pub(crate) fn encode_neon_mmla(operands: &[Operand], mn: &str) -> Result<EncodeResult, String> {
    if operands.len() != 3 {
        return Err(format!("{mn}: expected 3 operands, got {}", operands.len()));
    }
    let r = vregs(operands, &["4s", "16b", "16b"], mn)?;
    let base = match mn {
        "smmla" => 0x4e80_a400u32,
        "ummla" => 0x6e80_a400,
        _ => 0x4e80_ac00, // usmmla
    };
    Ok(EncodeResult::Word(base | (r[2] << 16) | (r[1] << 5) | r[0]))
}

/// SQRDMLAH / SQRDMLSH (FEAT_RDM), vector (`4h`/`8h`/`2s`/`4s`):
/// `0 Q 1 01110 size 0 Rm 1000 S 1 Rn Rd` (`sqrdmlah v3.4h, v29.4h, v17.4h` =
/// 0x2e5187a3, `sqrdmlsh v3.4s, v29.4s, v17.4s` = 0x6e918fa3); by element
/// through the shared integer by-element leaf with U=1, opcode 1101/1111.
pub(crate) fn encode_neon_sqrdmlxh(
    operands: &[Operand],
    sub: bool,
) -> Result<EncodeResult, String> {
    let mn = if sub { "sqrdmlsh" } else { "sqrdmlah" };
    if operands.len() != 3 {
        return Err(format!("{mn}: expected 3 operands, got {}", operands.len()));
    }
    let arr = match operands.first() {
        Some(Operand::RegArrangement { arrangement, .. }) => arrangement.clone(),
        other => {
            return Err(format!(
                "{mn}: expected a vector destination, got {other:?}"
            ));
        }
    };
    let (q, size) = match arr.as_str() {
        "4h" => (0u32, 1u32),
        "8h" => (1, 1),
        "2s" => (0, 2),
        "4s" => (1, 2),
        _ => return Err(format!("{mn}: no `.{arr}` form (4h, 8h, 2s or 4s)")),
    };
    if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
        let r = vregs(operands, &[&arr, &arr], mn)?;
        let (rm, h, l, m) = elem_index_fields(operands, 2, size, mn)?;
        let opcode = if sub { 0b1111 } else { 0b1101 };
        return Ok(EncodeResult::Word(
            0x2f00_0000
                | (q << 30)
                | (size << 22)
                | (l << 21)
                | (m << 20)
                | (rm << 16)
                | (opcode << 12)
                | (h << 11)
                | (r[1] << 5)
                | r[0],
        ));
    }
    let r = vregs(operands, &[&arr, &arr, &arr], mn)?;
    Ok(EncodeResult::Word(
        0x2e00_8400
            | (q << 30)
            | (size << 22)
            | (r[2] << 16)
            | (u32::from(sub) << 11)
            | (r[1] << 5)
            | r[0],
    ))
}

/// The (Q, size) of an integer vector arrangement other than `1d`.
fn int_q_size_no_1d(arr: &str, what: &str) -> Result<(u32, u32), String> {
    match arr {
        "8b" => Ok((0, 0)),
        "16b" => Ok((1, 0)),
        "4h" => Ok((0, 1)),
        "8h" => Ok((1, 1)),
        "2s" => Ok((0, 2)),
        "4s" => Ok((1, 2)),
        "2d" => Ok((1, 3)),
        _ => Err(format!("{what}: no `.{arr}` form")),
    }
}

/// SUQADD / USQADD (vector): two-misc opcode 00011, every arrangement but
/// `1d` (`suqadd v3.8b, v29.8b` = 0x0e203ba3, `usqadd v3.2d, v29.2d` =
/// 0x6ee03ba3).
pub(crate) fn encode_neon_sqadd_acc(
    operands: &[Operand],
    u: u32,
    mn: &str,
) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!("{mn}: expected 2 operands, got {}", operands.len()));
    }
    let arr = match operands.first() {
        Some(Operand::RegArrangement { arrangement, .. }) => arrangement.clone(),
        other => return Err(format!("{mn}: expected a vector register, got {other:?}")),
    };
    let (q, size) = int_q_size_no_1d(&arr, mn)?;
    let r = vregs(operands, &[&arr, &arr], mn)?;
    Ok(EncodeResult::Word(
        (q << 30)
            | (u << 29)
            | (0b01110 << 24)
            | (size << 22)
            | (0b10000 << 17)
            | (0b00011 << 12)
            | (0b10 << 10)
            | (r[1] << 5)
            | r[0],
    ))
}

/// SQSHLU (vector, immediate): `0 Q 1 011110 immh:immb 01100 1 Rn Rd`,
/// `immh:immb = esize + shift`, 0 <= shift < esize (`sqshlu v3.16b, v29.16b,
/// #7` = 0x6f0f67a3, `sqshlu v3.2d, v29.2d, #63` = 0x6f7f67a3).
pub(crate) fn encode_neon_sqshlu(operands: &[Operand]) -> Result<EncodeResult, String> {
    let mn = "sqshlu";
    if operands.len() != 3 {
        return Err(format!("{mn}: expected 3 operands, got {}", operands.len()));
    }
    let arr = match operands.first() {
        Some(Operand::RegArrangement { arrangement, .. }) => arrangement.clone(),
        other => return Err(format!("{mn}: expected a vector register, got {other:?}")),
    };
    let (q, size) = int_q_size_no_1d(&arr, mn)?;
    let r = vregs(operands, &[&arr, &arr], mn)?;
    let shift = imm_operand(operands, 2, mn)?;
    let esize = 8i64 << size;
    if !(0..esize).contains(&shift) {
        return Err(format!(
            "{mn}: shift #{shift} out of range 0 to {}",
            esize - 1
        ));
    }
    Ok(EncodeResult::Word(
        0x2f00_6400 | (q << 30) | (((esize + shift) as u32) << 16) | (r[1] << 5) | r[0],
    ))
}

/// ORR / BIC (vector, immediate): `0 Q op 0111100000 a:b:c cmode 0 1
/// d:e:f:g:h Rd`, op = 1 for BIC, with MOVI's shifted 16/32-bit cmode plus
/// one (`orr v3.4s, #0x12, lsl #16` = 0x4f005643, `bic v3.8h, #0x12` =
/// 0x6f009643).  Only `4h`/`8h`/`2s`/`4s` with an `lsl` shift exist.
pub(crate) fn encode_neon_orr_bic_imm(
    operands: &[Operand],
    bic: bool,
) -> Result<EncodeResult, String> {
    let mn = if bic { "bic" } else { "orr" };
    if !(2..=3).contains(&operands.len()) {
        return Err(format!("{mn}: expected `{mn} Vd.T, #imm8{{, lsl #n}}`"));
    }
    let (reg, arr) = match operands.first() {
        Some(Operand::RegArrangement { reg, arrangement }) => (reg.clone(), arrangement.clone()),
        other => {
            return Err(format!(
                "{mn}: expected a vector destination, got {other:?}"
            ));
        }
    };
    let rd = parse_reg_num(&reg).ok_or_else(|| format!("{mn}: bad vector register `{reg}`"))?;
    let imm = imm_operand(operands, 1, mn)?;
    if !(0..=255).contains(&imm) {
        return Err(format!("{mn}: immediate {imm} does not fit 8 bits (0-255)"));
    }
    let shift = match operands.get(2) {
        None => 0,
        Some(Operand::Shift { kind, amount }) if kind.eq_ignore_ascii_case("lsl") => *amount,
        Some(other) => return Err(format!("{mn}: expected `lsl #n`, got {other:?}")),
    };
    let (q, cmode) = match (arr.as_str(), shift) {
        ("4h" | "8h", 0) => (u32::from(arr == "8h"), 0b1001u32),
        ("4h" | "8h", 8) => (u32::from(arr == "8h"), 0b1011),
        ("2s" | "4s", 0) => (u32::from(arr == "4s"), 0b0001),
        ("2s" | "4s", 8) => (u32::from(arr == "4s"), 0b0011),
        ("2s" | "4s", 16) => (u32::from(arr == "4s"), 0b0101),
        ("2s" | "4s", 24) => (u32::from(arr == "4s"), 0b0111),
        _ => {
            return Err(format!(
                "{mn}: `.{arr}` with `lsl #{shift}` has no immediate form"
            ));
        }
    };
    let imm8 = imm as u32;
    Ok(EncodeResult::Word(
        (q << 30)
            | (u32::from(bic) << 29)
            | (0b0111100000 << 19)
            | ((imm8 >> 5) << 16)
            | (cmode << 12)
            | (1 << 10)
            | ((imm8 & 0x1F) << 5)
            | rd,
    ))
}

/// The cryptographic extensions: SHA1/SHA256 (FEAT_SHA1/SHA256), SHA512 and
/// SHA3 (`rax1`, `xar`, `bcax`; `eor3` lives in neon.rs), SM3/SM4.
///
/// | form | word |
/// |---|---|
/// | `sha1c/p/m Qd, Sn, Vm.4s`, `sha1su0`, `sha256h/h2 Qd, Qn, Vm.4s`, `sha256su1` | 0x5e000000 \| Rm<<16 \| opc<<12 (0-6) |
/// | `sha1h Sd, Sn`, `sha1su1`, `sha256su0` | 0x5e280800 \| opc<<12 (0-2) |
/// | `sha512h/h2 Qd, Qn, Vm.2d`, `sha512su1`, `rax1` | 0xce608000 \| Rm<<16 \| opc<<10 (0-3) |
/// | `sm3partw1/2`, `sm4ekey` | 0xce60c000 \| Rm<<16 \| opc<<10 (0-2) |
/// | `sha512su0 Vd.2d, Vn.2d`, `sm4e Vd.4s, Vn.4s` | 0xcec08000 \| opc<<10 |
/// | `bcax` (16b) / `sm3ss1` (4s), four registers | 0xce200000 / 0xce400000 \| Rm<<16 \| Ra<<10 |
/// | `xar Vd.2d, Vn.2d, Vm.2d, #imm6` | 0xce800000 \| Rm<<16 \| imm6<<10 |
/// | `sm3tt1a/1b/2a/2b Vd.4s, Vn.4s, Vm.s[i]` | 0xce408000 \| Rm<<16 \| i<<12 \| opc<<10 |
///
/// (`sha1c q3, s29, v17.4s` = 0x5e1103a3, `sha512h q3, q29, v17.2d` =
/// 0xce7183a3, `xar v3.2d, v29.2d, v17.2d, #13` = 0xce9137a3, `sm3tt1a v3.4s,
/// v29.4s, v17.s[3]` = 0xce51b3a3, GNU as 2.47.)
pub(crate) fn encode_crypto_ext(operands: &[Operand], mn: &str) -> Result<EncodeResult, String> {
    let fp = |idx: usize, letter: u8| -> Result<u32, String> {
        let (num, l, name) = fp_reg(operands, idx, mn)?;
        if l != letter {
            return Err(format!(
                "{mn}: operand {idx} must be a `{}` register, got `{name}`",
                char::from(letter)
            ));
        }
        Ok(num)
    };
    let count = |n: usize| -> Result<(), String> {
        if operands.len() == n {
            Ok(())
        } else {
            Err(format!(
                "{mn}: expected {n} operands, got {}",
                operands.len()
            ))
        }
    };
    let word = match mn {
        "sha1c" | "sha1p" | "sha1m" => {
            count(3)?;
            let opc = match mn {
                "sha1c" => 0,
                "sha1p" => 1,
                _ => 2,
            };
            let rd = fp(0, b'q')?;
            let rn = fp(1, b's')?;
            let rm = vregs(&operands[2..], &["4s"], mn)?[0];
            0x5e00_0000 | (rm << 16) | (opc << 12) | (rn << 5) | rd
        }
        "sha256h" | "sha256h2" => {
            count(3)?;
            let rd = fp(0, b'q')?;
            let rn = fp(1, b'q')?;
            let rm = vregs(&operands[2..], &["4s"], mn)?[0];
            0x5e00_0000 | (rm << 16) | ((4 + u32::from(mn == "sha256h2")) << 12) | (rn << 5) | rd
        }
        "sha1su0" | "sha256su1" => {
            count(3)?;
            let r = vregs(operands, &["4s", "4s", "4s"], mn)?;
            let opc = if mn == "sha1su0" { 3 } else { 6 };
            0x5e00_0000 | (r[2] << 16) | (opc << 12) | (r[1] << 5) | r[0]
        }
        "sha1h" => {
            count(2)?;
            0x5e28_0800 | (fp(1, b's')? << 5) | fp(0, b's')?
        }
        "sha1su1" | "sha256su0" => {
            count(2)?;
            let r = vregs(operands, &["4s", "4s"], mn)?;
            0x5e28_0800 | ((1 + u32::from(mn == "sha256su0")) << 12) | (r[1] << 5) | r[0]
        }
        "sha512h" | "sha512h2" => {
            count(3)?;
            let rd = fp(0, b'q')?;
            let rn = fp(1, b'q')?;
            let rm = vregs(&operands[2..], &["2d"], mn)?[0];
            0xce60_8000 | (rm << 16) | (u32::from(mn == "sha512h2") << 10) | (rn << 5) | rd
        }
        "sha512su1" | "rax1" => {
            count(3)?;
            let r = vregs(operands, &["2d", "2d", "2d"], mn)?;
            let opc = if mn == "sha512su1" { 2 } else { 3 };
            0xce60_8000 | (r[2] << 16) | (opc << 10) | (r[1] << 5) | r[0]
        }
        "sm3partw1" | "sm3partw2" | "sm4ekey" => {
            count(3)?;
            let r = vregs(operands, &["4s", "4s", "4s"], mn)?;
            let opc = match mn {
                "sm3partw1" => 0,
                "sm3partw2" => 1,
                _ => 2,
            };
            0xce60_c000 | (r[2] << 16) | (opc << 10) | (r[1] << 5) | r[0]
        }
        "sha512su0" => {
            count(2)?;
            let r = vregs(operands, &["2d", "2d"], mn)?;
            0xcec0_8000 | (r[1] << 5) | r[0]
        }
        "sm4e" => {
            count(2)?;
            let r = vregs(operands, &["4s", "4s"], mn)?;
            0xcec0_8400 | (r[1] << 5) | r[0]
        }
        "bcax" | "sm3ss1" => {
            count(4)?;
            let arr = if mn == "bcax" { "16b" } else { "4s" };
            let r = vregs(operands, &[arr, arr, arr, arr], mn)?;
            let base = if mn == "bcax" {
                0xce20_0000u32
            } else {
                0xce40_0000
            };
            base | (r[2] << 16) | (r[3] << 10) | (r[1] << 5) | r[0]
        }
        "xar" => {
            count(4)?;
            let r = vregs(operands, &["2d", "2d", "2d"], mn)?;
            let imm = imm_operand(operands, 3, mn)?;
            if !(0..=63).contains(&imm) {
                return Err(format!("{mn}: rotate #{imm} out of range 0 to 63"));
            }
            0xce80_0000 | (r[2] << 16) | ((imm as u32) << 10) | (r[1] << 5) | r[0]
        }
        "sm3tt1a" | "sm3tt1b" | "sm3tt2a" | "sm3tt2b" => {
            count(3)?;
            let r = vregs(operands, &["4s", "4s"], mn)?;
            let Some(Operand::RegLane {
                reg,
                elem_size,
                index,
            }) = operands.get(2)
            else {
                return Err(format!("{mn}: operand 2 must be `Vm.s[i]`"));
            };
            if elem_size != "s" || *index > 3 {
                return Err(format!(
                    "{mn}: operand 2 must be `Vm.s[0..3]`, got `{reg}.{elem_size}[{index}]`"
                ));
            }
            let rm =
                parse_reg_num(reg).ok_or_else(|| format!("{mn}: bad vector register `{reg}`"))?;
            let opc = match mn {
                "sm3tt1a" => 0,
                "sm3tt1b" => 1,
                "sm3tt2a" => 2,
                _ => 3,
            };
            0xce40_8000 | (rm << 16) | (index << 12) | (opc << 10) | (r[1] << 5) | r[0]
        }
        _ => return Err(format!("{mn}: not a cryptographic-extension mnemonic")),
    };
    Ok(EncodeResult::Word(word))
}
