use super::*;
use crate::backend::arm::assembler::parser::Operand;

// ── MOV ──────────────────────────────────────────────────────────────────

pub fn encode_mov(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("mov requires 2 operands".to_string());
    }

    // NEON register-to-register move: mov v1.16b, v0.16b -> ORR v1.16b, v0.16b, v0.16b
    if let (
        Some(Operand::RegArrangement {
            reg: rd_name,
            arrangement: arr_d,
        }),
        Some(Operand::RegArrangement {
            reg: rm_name,
            arrangement: _arr_m,
        }),
    ) = (operands.first(), operands.get(1))
    {
        let rd = parse_reg_num(rd_name).ok_or("invalid NEON rd")?;
        let rm = parse_reg_num(rm_name).ok_or("invalid NEON rm")?;
        let q: u32 = if arr_d == "16b" { 1 } else { 0 };
        // ORR Vd.T, Vm.T, Vm.T: 0 Q 0 01110 10 1 Rm 0 00111 Rn Rd
        let word = (q << 30)
            | (0b001110 << 24)
            | (0b10 << 22)
            | (1 << 21)
            | (rm << 16)
            | (0b000111 << 10)
            | (rm << 5)
            | rd;
        return Ok(EncodeResult::Word(word));
    }

    // NEON lane insert: mov v0.d[1], x1 -> INS Vd.D[index], Xn
    if let (
        Some(Operand::RegLane {
            reg: vd_name,
            elem_size,
            index,
        }),
        Some(Operand::Reg(rn_name)),
    ) = (operands.first(), operands.get(1))
    {
        let vd = parse_reg_num(vd_name).ok_or("invalid NEON vd")?;
        let rn = parse_reg_num(rn_name).ok_or("invalid rn")?;
        // INS Vd.Ts[index], Rn
        // Encoding: 0 1 0 0 1110 000 imm5 0 0011 1 Rn Rd
        // imm5 encoding depends on element size and index
        let imm5 = match elem_size.as_str() {
            "b" => ((*index & 0xF) << 1) | 0b00001,
            "h" => ((*index & 0x7) << 2) | 0b00010,
            "s" => ((*index & 0x3) << 3) | 0b00100,
            "d" => ((*index & 0x1) << 4) | 0b01000,
            _ => return Err(format!("unsupported element size for ins: {}", elem_size)),
        };
        let word = (0b01001110000u32 << 21) | (imm5 << 16) | (0b000111 << 10) | (rn << 5) | vd;
        return Ok(EncodeResult::Word(word));
    }

    // NEON lane extract: mov x0, v0.d[1] -> UMOV Xd, Vn.D[index]
    if let (
        Some(Operand::Reg(rd_name)),
        Some(Operand::RegLane {
            reg: vn_name,
            elem_size,
            index,
        }),
    ) = (operands.first(), operands.get(1))
    {
        let rd = parse_reg_num(rd_name).ok_or("invalid rd")?;
        let vn = parse_reg_num(vn_name).ok_or("invalid NEON vn")?;
        // UMOV Rd, Vn.Ts[index]
        // Encoding: 0 Q 0 0 1110 000 imm5 0 0111 1 Rn Rd
        let (q, imm5) = match elem_size.as_str() {
            "b" => (0u32, ((*index & 0xF) << 1) | 0b00001),
            "h" => (0, ((*index & 0x7) << 2) | 0b00010),
            "s" => (0, ((*index & 0x3) << 3) | 0b00100),
            "d" => (1, ((*index & 0x1) << 4) | 0b01000),
            _ => return Err(format!("unsupported element size for umov: {}", elem_size)),
        };
        let word =
            (q << 30) | (0b001110000u32 << 21) | (imm5 << 16) | (0b001111 << 10) | (vn << 5) | rd;
        return Ok(EncodeResult::Word(word));
    }

    // NEON element-to-element move: mov v0.s[3], v1.s[0] -> INS Vd.Ts[i1], Vn.Ts[i2]
    if let (
        Some(Operand::RegLane {
            reg: vd_name,
            elem_size: es_d,
            index: idx_d,
        }),
        Some(Operand::RegLane {
            reg: vn_name,
            elem_size: _es_n,
            index: idx_n,
        }),
    ) = (operands.first(), operands.get(1))
    {
        let vd = parse_reg_num(vd_name).ok_or("invalid NEON vd")?;
        let vn = parse_reg_num(vn_name).ok_or("invalid NEON vn")?;
        // INS Vd.Ts[i1], Vn.Ts[i2]
        // Encoding: 0 1 1 01110 000 imm5 0 imm4 1 Rn Rd
        let (imm5, imm4) = match es_d.as_str() {
            "b" => ((idx_d << 1) | 0b00001, *idx_n),
            "h" => ((idx_d << 2) | 0b00010, idx_n << 1),
            "s" => ((idx_d << 3) | 0b00100, idx_n << 2),
            "d" => ((idx_d << 4) | 0b01000, idx_n << 3),
            _ => return Err(format!("unsupported element size for ins: {}", es_d)),
        };
        let word =
            ((0b01101110000u32 << 21) | (imm5 << 16)) | (imm4 << 11) | (1 << 10) | (vn << 5) | vd;
        return Ok(EncodeResult::Word(word));
    }

    // mov Xd, #imm -> movz or movn
    if let Some(Operand::Imm(imm)) = operands.get(1) {
        // The destination of an integer immediate must be a general-purpose
        // register. `parse_reg_num` resolves `d0` to a bare number, so
        // `mov d0,#5` used to fall through to the GP path and silently emit
        // `movz w0,#5`. GNU as rejects it (the FP form is `fmov d0,#5.0`,
        // which lccc does not yet accept; fail closed rather than misencode).
        let (rd, is_64) = get_gpr_strict(operands, 0)?;
        let imm = *imm;

        // Check if it can be a simple MOVZ
        if (0..=0xFFFF).contains(&imm) {
            let sf = sf_bit(is_64);
            let word = (sf << 31) | (0b10100101 << 23) | ((imm as u32 & 0xFFFF) << 5) | rd;
            return Ok(EncodeResult::Word(word));
        }

        // Negative: try MOVN
        if imm < 0 {
            let not_imm = !imm;
            if (0..=0xFFFF).contains(&not_imm) {
                let sf = sf_bit(is_64);
                let word = (sf << 31) | (0b00100101 << 23) | ((not_imm as u32 & 0xFFFF) << 5) | rd;
                return Ok(EncodeResult::Word(word));
            }
        }

        // Try encoding as ORR Rd, XZR, #imm (logical/bitmask immediate)
        // This handles patterns like 0x0101010101010101 in a single instruction
        if let Some((n, immr, imms)) = encode_bitmask_imm(imm as u64, is_64) {
            let sf = sf_bit(is_64);
            // ORR Rd, XZR, #imm: sf 01 100100 N immr imms 11111 Rd
            let word = (sf << 31)
                | (0b01 << 29)
                | (0b100100 << 23)
                | (n << 22)
                | (immr << 16)
                | (imms << 10)
                | (0b11111 << 5)
                | rd;
            return Ok(EncodeResult::Word(word));
        }

        // Need movz + movk sequence for large immediates
        return encode_mov_wide_imm(rd, is_64, imm as u64);
    }

    // mov Xd, Xm -> ORR Xd, XZR, Xm
    if let (Some(Operand::Reg(rd_name)), Some(Operand::Reg(rm_name))) =
        (operands.first(), operands.get(1))
    {
        // Both operands must be general-purpose. `parse_reg_num` resolves
        // `d0` and `lr` to bare numbers, so without this class check
        // `mov d0, x1`, `mov x0, d1` and `mov d0, lr` all reached the GP
        // encoding and assembled as `orr`/`add` -- GNU as rejects all three.
        for (i, n) in [(0usize, rd_name), (1, rm_name)] {
            if !is_gp_reg(n) {
                return Err(format!(
                    "mov: operand {i} `{n}` is not a general-purpose register \
                     (expected x0-x30, w0-w30, lr, xzr or wzr)"
                ));
            }
        }
        // `lr` is x30, so a 32-bit destination cannot take it (`mov w0, lr`
        // is rejected by GNU as on width grounds). A register move needs
        // both operands the same size.
        if is_64bit_reg(rd_name) != is_64bit_reg(rm_name) {
            return Err(format!(
                "mov: `{rd_name}` and `{rm_name}` are different widths; \
                 a register move needs both operands the same size"
            ));
        }
        let rd = parse_reg_num(rd_name).ok_or("invalid rd")?;
        let rm = parse_reg_num(rm_name).ok_or("invalid rm")?;
        let is_64 = is_64bit_reg(rd_name);

        // Check for MOV to/from SP: uses ADD Xd, Xn, #0. Both the 64-bit
        // `sp` and the 32-bit `wsp` spelling take this form (GNU as:
        // `mov w0, wsp` -> 0x110003e0, an ADD, not an ORR).
        let is_sp = |n: &str| n.eq_ignore_ascii_case("sp") || n.eq_ignore_ascii_case("wsp");
        if is_sp(rd_name) || is_sp(rm_name) {
            let sf = sf_bit(is_64);
            // ADD Xd, Xn, #0: sf 0 0 10001 00 imm12=0 Rn Rd
            let word = ((sf << 31) | (0b10001 << 24)) | (rm << 5) | rd;
            return Ok(EncodeResult::Word(word));
        }

        let sf = sf_bit(is_64);
        // ORR Rd, XZR, Rm: sf 01 01010 00 0 Rm 000000 11111 Rd
        let word = ((sf << 31) | (0b01 << 29) | (0b01010 << 24)) | (rm << 16) | (0b11111 << 5) | rd;
        return Ok(EncodeResult::Word(word));
    }

    Err(format!("unsupported mov operands: {:?}", operands))
}

pub fn encode_mov_wide_imm(rd: u32, is_64: bool, imm: u64) -> Result<EncodeResult, String> {
    let sf = sf_bit(is_64);
    let mut words = Vec::new();
    let max_hw = if is_64 { 4 } else { 2 };
    let mut first = true;

    for hw in 0..max_hw {
        let chunk = ((imm >> (hw * 16)) & 0xFFFF) as u32;
        if chunk != 0 || (hw == 0 && imm == 0) {
            if first {
                // MOVZ
                let word = (sf << 31) | (0b10100101 << 23) | (hw << 21) | (chunk << 5) | rd;
                words.push(word);
                first = false;
            } else {
                // MOVK
                let word = (sf << 31) | (0b11100101 << 23) | (hw << 21) | (chunk << 5) | rd;
                words.push(word);
            }
        }
    }

    if words.is_empty() {
        // imm is 0
        let word = (sf << 31) | (0b10100101 << 23) | rd;
        words.push(word);
    }

    if words.len() == 1 {
        Ok(EncodeResult::Word(words[0]))
    } else {
        Ok(EncodeResult::Words(words))
    }
}

/// Classification of an `:abs_g*:` modifier for movz/movk (GAS parity).
#[derive(Debug, Clone, Copy)]
pub struct AbsGSpec {
    /// Right-shift applied to the 64-bit value before masking: 0/16/32/48.
    pub shift: u32,
    /// Signed variant (`:abs_g*_s:`) — the field is the sign-extended
    /// bit pattern; the ABI has no `abs_g3_s` form.
    pub signed: bool,
    /// No-check variant (`_nc`): the value is truncated, never
    /// overflow-checked. Used for the later movk chunks of a sequence.
    /// The ABI defines no `abs_g3_nc`.
    pub nc: bool,
}

/// Classify an `:abs_g*:` modifier kind.
///
/// * `Ok(None)`  — the kind is not an abs_g modifier at all (caller falls
///   back to immediate handling).
/// * `Err`       — the kind looks like abs_g but is not a valid ABI form;
///   a precise error beats a misleading "expected immediate" later.
pub fn abs_g_spec(kind: &str) -> Result<Option<AbsGSpec>, String> {
    let spec = match kind {
        "abs_g0" => (0, false, false),
        "abs_g1" => (16, false, false),
        "abs_g2" => (32, false, false),
        "abs_g3" => (48, false, false),
        "abs_g0_nc" => (0, false, true),
        "abs_g1_nc" => (16, false, true),
        "abs_g2_nc" => (32, false, true),
        "abs_g0_s" => (0, true, false),
        "abs_g1_s" => (16, true, false),
        "abs_g2_s" => (32, true, false),
        _ => {
            if kind.starts_with("abs_g") {
                return Err(format!(
                    ":{}: is not a valid AArch64 abs_g modifier (valid: abs_g0..abs_g3, \
                     abs_g0_nc..abs_g2_nc, abs_g0_s..abs_g2_s — the ABI defines no \
                     abs_g3_nc/abs_g3_s)",
                    kind
                ));
            }
            return Ok(None);
        }
    };
    Ok(Some(AbsGSpec {
        shift: spec.0,
        signed: spec.1,
        nc: spec.2,
    }))
}

/// Build a MOVZ (is_movz) / MOVK word with the given halfword selector.
pub fn movw_word(is_movz: bool, rd: u32, is_64: bool, hw: u32, imm16: u32) -> u32 {
    let sf = sf_bit(is_64);
    let base = if is_movz { 0b1010_0101u32 } else { 0b1110_0101 };
    (sf << 31) | (base << 23) | (hw << 21) | ((imm16 & 0xFFFF) << 5) | rd
}

/// Reads the optional `lsl #hw*16` operand of MOVZ/MOVK/MOVN.
///
/// The shift must be `lsl` with an amount that is a multiple of 16 within
/// 0..=48 (0..=16 for the 32-bit form); GNU as rejects every other
/// spelling. The old code silently divided the amount by 16 -- discarding
/// `lsl #8` entirely, mapping `lsl #20` to `hw=1` -- and treated non-`lsl`
/// kinds and non-shift operands as `hw = 0`, so `movz x0,#1,ror #16` and
/// `movz x0,#1,uxtb` both assembled as plain `movz x0,#1`.
fn movw_shift_hw(operands: &[Operand], mn: &str, is_64: bool) -> Result<u32, String> {
    let max_hw = if is_64 { 3 } else { 1 };
    match operands.get(2) {
        None => Ok(0),
        Some(Operand::Shift { kind, amount }) => {
            if kind != "lsl" {
                return Err(format!(
                    "{mn}: shift kind `{kind}` is not valid here \
                     (only `lsl #0/#16/#32/#48` is allowed)"
                ));
            }
            if *amount % 16 != 0 || (*amount / 16) > max_hw {
                return Err(format!(
                    "{mn}: shift amount {amount} is not one of the allowed \
                     values (lsl #0..#{})",
                    max_hw * 16
                ));
            }
            Ok(*amount / 16)
        }
        Some(other) => Err(format!(
            "{mn}: unexpected operand {other:?}; only `lsl #0/#16/#32/#48` \
             may follow the immediate"
        )),
    }
}

/// Reads and validates the 16-bit immediate of MOVZ/MOVK/MOVN.
///
/// The field is exactly 16 bits wide and unsigned; GNU as rejects `-1` and
/// `0x10000` alike. The old code masked with `& 0xFFFF`, silently turning
/// `movz x0,#0x10000` into `movz x0,#0` and `movz x0,#-1` into `movz x0,#0xffff`.
fn movw_imm16(operands: &[Operand], mn: &str) -> Result<u32, String> {
    let imm = get_imm(operands, 1)?;
    if !(0..=0xFFFF).contains(&imm) {
        return Err(format!(
            "{mn}: immediate {imm} does not fit the 16-bit field (0..=65535)"
        ));
    }
    Ok(imm as u32)
}

pub fn encode_movz(operands: &[Operand]) -> Result<EncodeResult, String> {
    // MOVW is GP-only and takes no SP: register 31 in this encoding is XZR.
    // `movz d0,#5` used to fall through `get_reg` and emit `movz w0,#5`.
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let sf = sf_bit(is_64);

    // Handle :abs_g*: modifiers
    if let Some(Operand::Modifier { kind, symbol }) = operands.get(1) {
        if let Some(spec) = abs_g_spec(kind)? {
            if let Ok(val) = crate::backend::asm_expr::parse_integer_expr(symbol) {
                let imm16 = ((val as u64) >> spec.shift) as u32 & 0xFFFF;
                let hw = spec.shift / 16;
                return Ok(EncodeResult::Word(movw_word(true, rd, is_64, hw, imm16)));
            }
            // Symbolic value: the ELF writer resolves these at write time
            // (aliases/labels inline, everything else as a MOVW reloc),
            // so reaching here means the modifier was fed to the encoder
            // directly. Fail with a precise error instead of a misleading
            // "expected immediate".
            return Err(format!(
                "{}: symbolic :{}: value '{}' must be resolved by the ELF writer",
                "movz", kind, symbol
            ));
        }
    }

    let imm = movw_imm16(operands, "movz")?;
    let hw = movw_shift_hw(operands, "movz", is_64)?;

    let word = (sf << 31) | (0b10100101 << 23) | (hw << 21) | ((imm & 0xFFFF) << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_movk(operands: &[Operand]) -> Result<EncodeResult, String> {
    // GP-only, no SP (Rd=31 is XZR in the MOVW encoding).
    let (rd, is_64) = get_gpr_strict(operands, 0)?;

    // Handle :abs_g*: modifiers
    if let Some(Operand::Modifier { kind, symbol }) = operands.get(1) {
        if let Some(spec) = abs_g_spec(kind)? {
            if let Ok(val) = crate::backend::asm_expr::parse_integer_expr(symbol) {
                let imm16 = ((val as u64) >> spec.shift) as u32 & 0xFFFF;
                let hw = spec.shift / 16;
                return Ok(EncodeResult::Word(movw_word(false, rd, is_64, hw, imm16)));
            }
            return Err(format!(
                "movk: symbolic :{}: value '{}' must be resolved by the ELF writer",
                kind, symbol
            ));
        }
    }

    let imm = movw_imm16(operands, "movk")?;
    let hw = movw_shift_hw(operands, "movk", is_64)?;

    let sf = sf_bit(is_64);
    let word = (sf << 31) | (0b11100101 << 23) | (hw << 21) | ((imm & 0xFFFF) << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_movn(operands: &[Operand]) -> Result<EncodeResult, String> {
    // GP-only, no SP (Rd=31 is XZR in the MOVW encoding).
    let (rd, is_64) = get_gpr_strict(operands, 0)?;

    // GAS parity: MOVN supports no :abs_g*: relocations — the MOVW
    // relocation semantics write the un-inverted value, which is wrong
    // for MOVN's inverted immediate. Reject with a precise message.
    if let Some(Operand::Modifier { kind, symbol }) = operands.get(1) {
        if abs_g_spec(kind)?.is_some() {
            return Err(format!(
                "movn does not support :{}: modifiers (operand '{}'): \
                 MOVW relocations cannot express MOVN's inverted immediate",
                kind, symbol
            ));
        }
    }

    let imm = movw_imm16(operands, "movn")?;
    let sf = sf_bit(is_64);
    let hw = movw_shift_hw(operands, "movn", is_64)?;

    let word = (sf << 31) | (0b00100101 << 23) | (hw << 21) | ((imm & 0xFFFF) << 5) | rd;
    Ok(EncodeResult::Word(word))
}

// ── ADD/SUB ──────────────────────────────────────────────────────────────

pub fn encode_add_sub(
    operands: &[Operand],
    is_sub: bool,
    set_flags: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err(format!(
            "add/sub requires 3 operands, got {}",
            operands.len()
        ));
    }

    // NEON vector form: ADD/SUB Vd.T, Vn.T, Vm.T
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        if !set_flags {
            return encode_neon_add_sub(operands, is_sub);
        }
    }

    let mn = match (is_sub, set_flags) {
        (false, false) => "add",
        (false, true) => "adds",
        (true, false) => "sub",
        (true, true) => "subs",
    };

    // ADD/SUB are general-purpose: `add x0, x1, h2` has no encoding and GNU as
    // rejects it, but the permissive operand readers resolve `h2` to 2 and
    // assemble it as `add x0, x1, x2`. SP stays legal (add sp, sp, #16) --
    // its per-form legality is diagnosed below.
    for i in 0..3 {
        if let Some(Operand::Reg(reg)) = operands.get(i) {
            if !is_gp_reg(reg) {
                return Err(format!(
                    "{mn}: operand {i} `{reg}` is not a general-purpose register \
                     (expected x0-x30, w0-w30, lr, sp or xzr)"
                ));
            }
        }
    }

    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, rn_is_64) = get_reg(operands, 1)?;
    // A single sf bit drives the whole encoding, so mixed widths between
    // destination and source are not encodable: GNU as rejects
    // `add x0,w1,x2` and `add w0,sp,x1` ("must use wsp for 32-bit").
    if is_64 != rn_is_64 {
        let rd_name = match operands.first() {
            Some(Operand::Reg(n)) => n.as_str(),
            _ => "<rd>",
        };
        let rn_name = match operands.get(1) {
            Some(Operand::Reg(n)) => n.as_str(),
            _ => "<rn>",
        };
        return Err(format!(
            "{mn}: `{rd_name}` and `{rn_name}` are different widths; \
             add/sub needs the destination and first source the same size"
        ));
    }
    // The flags-setting forms read field 31 of Rd as XZR, so SP as the
    // destination of adds/subs is not encodable. GNU as rejects
    // `adds sp,x1,#15` and `subs sp,x0,x1`.
    let rd_is_sp = matches!(&operands[0], Operand::Reg(n)
        if n.eq_ignore_ascii_case("sp") || n.eq_ignore_ascii_case("wsp"));
    if set_flags && rd_is_sp {
        return Err(format!(
            "{mn}: the destination cannot be sp/wsp in the flags-setting form \
             (register 31 reads as the zero register there)"
        ));
    }
    let rn_is_sp = matches!(&operands[1], Operand::Reg(n)
        if n.eq_ignore_ascii_case("sp") || n.eq_ignore_ascii_case("wsp"));

    let sf = sf_bit(is_64);
    let op = if is_sub { 1u32 } else { 0u32 };
    let s_bit = if set_flags { 1u32 } else { 0u32 };

    // ADD Rd, Rn, #imm
    if let Some(Operand::Imm(imm)) = operands.get(2) {
        let imm_signed = *imm;
        // Handle negative immediates: add #-N -> sub #N and vice versa
        let (imm_val, actual_op) = if imm_signed < 0 {
            ((-imm_signed) as u64, if is_sub { 0u32 } else { 1u32 })
        } else {
            (imm_signed as u64, op)
        };
        // The immediate form's shift field is one bit: `lsl #12` or nothing.
        // GNU as rejects every other fourth operand (`add x0,x1,#15,lsl #4`
        // is an error, and so is any extend). The old code silently ignored
        // a non-`lsl #12` shift and assembled the unshifted immediate.
        let explicit_shift = match operands.get(3) {
            None => false,
            Some(Operand::Shift { kind, amount }) => {
                if kind != "lsl" || (*amount != 0 && *amount != 12) {
                    return Err(format!(
                        "{mn}: shift `{kind} #{amount}` is not valid after an \
                         immediate; only `lsl #0` or `lsl #12` is allowed"
                    ));
                }
                *amount == 12
            }
            Some(other) => {
                return Err(format!(
                    "{mn}: unexpected operand {other:?}; only `lsl #0` or \
                     `lsl #12` may follow an immediate"
                ));
            }
        };

        let (imm12, sh) = if explicit_shift {
            // Explicit lsl #12: use the immediate as-is (must fit in 12 bits)
            ((imm_val as u32) & 0xFFF, 1u32)
        } else if imm_val <= 0xFFF {
            // Fits in 12 bits unshifted
            (imm_val as u32, 0u32)
        } else if (imm_val & 0xFFF) == 0 && (imm_val >> 12) <= 0xFFF {
            // Low 12 bits are zero and shifted value fits: auto-shift
            // e.g., #4096 -> #1, lsl #12
            ((imm_val >> 12) as u32, 1u32)
        } else {
            return Err(format!(
                "immediate {} does not fit in add/sub imm12 encoding",
                imm_val
            ));
        };

        let word = (sf << 31)
            | (actual_op << 30)
            | (s_bit << 29)
            | (0b10001 << 24)
            | (sh << 22)
            | (imm12 << 10)
            | (rn << 5)
            | rd;
        return Ok(EncodeResult::Word(word));
    }

    // ADD Rd, Rn, :lo12:symbol
    if let Some(Operand::Modifier { kind, symbol }) = operands.get(2) {
        if kind == "lo12" {
            let word = ((sf << 31) | (op << 30) | (s_bit << 29) | (0b10001 << 24)) | (rn << 5) | rd;
            return Ok(EncodeResult::WordWithReloc {
                word,
                reloc: Relocation {
                    reloc_type: RelocType::AddAbsLo12,
                    symbol: symbol.clone(),
                    addend: 0,
                },
            });
        }
        if kind == "tprel_lo12_nc" {
            let word = ((sf << 31) | (op << 30) | (s_bit << 29) | (0b10001 << 24)) | (rn << 5) | rd;
            return Ok(EncodeResult::WordWithReloc {
                word,
                reloc: Relocation {
                    reloc_type: RelocType::TlsLeAddTprelLo12,
                    symbol: symbol.clone(),
                    addend: 0,
                },
            });
        }
        if kind == "tprel_hi12" {
            let word = ((sf << 31) | (op << 30) | (s_bit << 29) | (0b10001 << 24) | (1 << 22))
                | (rn << 5)
                | rd;
            return Ok(EncodeResult::WordWithReloc {
                word,
                reloc: Relocation {
                    reloc_type: RelocType::TlsLeAddTprelHi12,
                    symbol: symbol.clone(),
                    addend: 0,
                },
            });
        }
    }
    if let Some(Operand::ModifierOffset {
        kind,
        symbol,
        offset,
    }) = operands.get(2)
    {
        if kind == "lo12" {
            let word = ((sf << 31) | (op << 30) | (s_bit << 29) | (0b10001 << 24)) | (rn << 5) | rd;
            return Ok(EncodeResult::WordWithReloc {
                word,
                reloc: Relocation {
                    reloc_type: RelocType::AddAbsLo12,
                    symbol: symbol.clone(),
                    addend: *offset,
                },
            });
        }
    }

    // ADD Rd, Rn, Rm
    if let Some(Operand::Reg(rm_name)) = operands.get(2) {
        let rm = parse_reg_num(rm_name).ok_or("invalid rm")?;

        // Check for extended register: add Xd, Xn, Wm, sxtw [#N]
        if let Some(Operand::Extend { kind, amount }) = operands.get(3) {
            // Only the eight architectural extends are encodable here; the
            // old `match` mapped anything else (including a `ror` the parser
            // happily produces) to UXTX, silently assembling
            // `add x0,x1,x2,ror #3` as `add x0,x1,x2,lsl #3`.
            let option = match kind.as_str() {
                "uxtb" => 0b000u32,
                "uxth" => 0b001,
                "uxtw" => 0b010,
                "uxtx" => 0b011,
                "sxtb" => 0b100,
                "sxth" => 0b101,
                "sxtw" => 0b110,
                "sxtx" => 0b111,
                other => {
                    return Err(format!(
                        "{mn}: `{other}` is not a valid extend here \
                         (expected uxtb/uxth/uxtw/uxtx/sxtb/sxth/sxtw/sxtx)"
                    ));
                }
            };
            // The imm3 field holds 0..=4; the old code masked with `& 0x7`,
            // so `add x0,x1,x2,uxtb #8` wrapped to `#0` and `sxtw #7` was
            // emitted with an unallocatable imm3. GNU as rejects the range.
            if *amount > 4 {
                return Err(format!("{mn}: extend shift {amount} is out of range 0..=4"));
            }
            // The 64-to-64 extends (uxtx/sxtx) read an X register, but only
            // in the 64-bit form; every other combination reads a W register
            // (the 32-bit ADD always reads Wm whatever the extend says).
            // Distilled from the llvm-mc 23.1.2 + GNU as 2.47 acceptance
            // matrix: llvm enforces exactly this; GNU as additionally
            // accepts the mismatched spellings and encodes the same word,
            // a leniency lccc does not copy.
            let rm_wants_64 = is_64 && matches!(option, 0b011 | 0b111);
            if is_64bit_reg(rm_name) != rm_wants_64 {
                return Err(format!(
                    "{mn}: `{rm_name}` has the wrong width for `{}`; the \
                     64-to-64 extends take an x register, all others a w register",
                    kind
                ));
            }
            let imm3 = *amount & 0x7;
            // Extended register form: sf op S 01011 00 1 Rm option imm3 Rn Rd
            let word = ((sf << 31) | (op << 30) | (s_bit << 29) | (0b01011 << 24))
                | (1 << 21)
                | (rm << 16)
                | (option << 13)
                | (imm3 << 10)
                | (rn << 5)
                | rd;
            return Ok(EncodeResult::Word(word));
        }

        // When Rn or Rd is SP (register 31), the shifted register form encodes
        // register 31 as XZR, not SP, so the SP semantics must come from the
        // extended register form, where field 31 of Rn/Rd is SP. This covers
        // both the plain register form (`add sp,x0,x1` = 0x8b21601f) and the
        // shifted one: `add sp,x0,x1,lsl #2` is `add sp,x0,x1,uxtx #2`
        // (0x8b21681f) in GNU as. The old code only routed the
        // operand-count<=3 case and silently emitted the XZR-writing shifted
        // word for `add sp,x0,x1,lsl #2` (0x8b01081f instead of 0x8b21681f).
        if rn_is_sp || rd_is_sp {
            let shift_amount = match operands.get(3) {
                None => 0u32,
                Some(Operand::Shift { kind, amount }) => {
                    // Only `lsl` is expressible in the extended form's imm3.
                    if kind != "lsl" {
                        return Err(format!(
                            "{mn}: shift `{kind}` cannot accompany sp in the \
                             register form (the sp semantics come from the \
                             extended encoding, which only encodes `lsl`)"
                        ));
                    }
                    *amount
                }
                Some(other) => {
                    return Err(format!(
                        "{mn}: unexpected operand {other:?} with sp in the \
                         register form"
                    ));
                }
            };
            if shift_amount > 4 {
                return Err(format!(
                    "{mn}: shift {shift_amount} with sp is out of range 0..=4 \
                     (the extended encoding's imm3 field is 3 bits, max 4)"
                ));
            }
            // Extended register form with UXTX/UXTW #N:
            // sf op S 01011 00 1 Rm option imm3 Rn Rd
            let option = if is_64 { 0b011u32 } else { 0b010u32 }; // UXTX for 64-bit, UXTW for 32-bit
            let word = (((sf << 31) | (op << 30) | (s_bit << 29) | (0b01011 << 24))
                | (1 << 21)
                | (rm << 16)
                | (option << 13)
                | (shift_amount << 10))
                | (rn << 5)
                | rd;
            return Ok(EncodeResult::Word(word));
        }

        // Check for shifted register: add Xd, Xn, Xm, lsl #N
        let (shift_type, shift_amount) =
            if let Some(Operand::Shift { kind, amount }) = operands.get(3) {
                // ADD/SUB (shifted register) encodes LSL/LSR/ASR only; the
                // old catch-all mapped `ror` (and any unknown kind) to LSL,
                // silently assembling `add x0,x1,x2,ror #3` as `lsl #3`.
                let st = match kind.as_str() {
                    "lsl" => 0b00u32,
                    "lsr" => 0b01,
                    "asr" => 0b10,
                    other => {
                        return Err(format!(
                            "{mn}: shift `{other}` is not valid on add/sub \
                             (only lsl/lsr/asr; ror belongs to the logical ops)"
                        ));
                    }
                };
                (st, *amount)
            } else {
                (0, 0)
            };
        // imm6 is 6 bits and the range halves for the 32-bit form: GNU as
        // rejects `add x0,x1,x2,lsl #64` (which the old `& 0x3F` mask
        // wrapped to `lsl #0`) and `add w0,w1,w2,lsl #32` outright.
        let max_shift = if is_64 { 63 } else { 31 };
        if shift_amount > max_shift {
            return Err(format!(
                "{mn}: shift {shift_amount} is out of range 0..={max_shift} \
                 for the {}-bit form",
                if is_64 { 64 } else { 32 }
            ));
        }
        // The shifted form has one sf bit for all three registers.
        if is_64bit_reg(rm_name) != is_64 {
            return Err(format!(
                "{mn}: `{rm_name}` does not match the width of the destination; \
                 the shifted form needs all registers the same size"
            ));
        }

        let word = ((sf << 31) | (op << 30) | (s_bit << 29) | (0b01011 << 24) | (shift_type << 22))
            | (rm << 16)
            | (shift_amount << 10)
            | (rn << 5)
            | rd;
        return Ok(EncodeResult::Word(word));
    }

    Err(format!("unsupported add/sub operands: {:?}", operands))
}

// ── Logical ──────────────────────────────────────────────────────────────

pub fn encode_logical(operands: &[Operand], opc: u32) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("logical op requires 3 operands".to_string());
    }

    // NEON vector form: ORR/AND/EOR Vd.T, Vn.T, Vm.T
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        return encode_neon_logical(operands, opc);
    }

    // SP is legal ONLY as the destination of the non-flags immediate forms
    // (ARM ARM "Logical (immediate)": AND/ORR/EOR Rd may be SP; ANDS and the
    // register forms take no SP anywhere). Oracle: `and sp, x1, #15` assembles
    // in GNU as; `and x0, sp, #15` and `and sp, x1, x2` do not.
    let sp_ok_in_rd = matches!(operands.get(2), Some(Operand::Imm(_))) && opc != 0b11;
    let (rd, is_64) = if sp_ok_in_rd {
        get_gpr_or_sp(operands, 0)?
    } else {
        get_gpr_strict(operands, 0)?
    };
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    check_same_width(is_64, &[rn64])?;
    let sf = sf_bit(is_64);

    // AND/ORR/EOR Rd, Rn, #imm (bitmask immediate)
    if let Some(Operand::Imm(imm)) = operands.get(2) {
        if operands.len() != 3 {
            return Err(format!(
                "logical op takes 3 operands for the immediate form, got {}",
                operands.len()
            ));
        }
        if let Some((n, immr, imms)) = encode_bitmask_imm(*imm as u64, is_64) {
            let word = (sf << 31)
                | (opc << 29)
                | (0b100100 << 23)
                | (n << 22)
                | (immr << 16)
                | (imms << 10)
                | (rn << 5)
                | rd;
            return Ok(EncodeResult::Word(word));
        }
        return Err(format!("cannot encode bitmask immediate: 0x{:x}", imm));
    }

    // AND/ORR/EOR Rd, Rn, Rm [, shift #amount]
    if matches!(operands.get(2), Some(Operand::Reg(_))) {
        // Rm must be a general-purpose register of the same width. Reading
        // it through `parse_reg_num` silently aliased FP/SIMD names (`s5`)
        // and SP into the shared 0-31 number space: `and w0, w1, s5` used to
        // encode as `and w0, w1, w5`, and `and x0, x1, sp` as
        // `and x0, x1, xzr`. GNU as rejects both with "operand mismatch".
        let (rm, rm64) = get_gpr_strict(operands, 2)?;
        check_same_width(is_64, &[rn64, rm64])?;

        if operands.len() > 4 {
            return Err(format!(
                "logical op takes at most 4 operands, got {}",
                operands.len()
            ));
        }
        let (shift_type, shift_amount) = get_shift_imm6(operands, 3, is_64)?;

        let word = ((sf << 31) | (opc << 29) | (0b01010 << 24) | (shift_type << 22))
            | (rm << 16)
            | (shift_amount << 10)
            | (rn << 5)
            | rd;
        return Ok(EncodeResult::Word(word));
    }

    Err("unsupported logical operands".to_string())
}

/// Encode a bitmask immediate for AArch64.
/// Returns (N, immr, imms) if the value is a valid bitmask immediate.
pub fn encode_bitmask_imm(val: u64, is_64: bool) -> Option<(u32, u32, u32)> {
    if val == 0 || (!is_64 && val == 0xFFFFFFFF) || (is_64 && val == u64::MAX) {
        return None; // Not a valid bitmask immediate
    }

    let width = if is_64 { 64 } else { 32 };
    let val = if !is_64 { val & 0xFFFFFFFF } else { val };

    // Try each possible element size: 2, 4, 8, 16, 32, 64
    for size in [2u32, 4, 8, 16, 32, 64] {
        if size > width {
            continue;
        }

        let mask = if size == 64 {
            u64::MAX
        } else {
            (1u64 << size) - 1
        };
        let elem = val & mask;

        // Check that the pattern repeats
        let mut repeats = true;
        let mut pos = size;
        while pos < width {
            if ((val >> pos) & mask) != elem {
                repeats = false;
                break;
            }
            pos += size;
        }
        if !repeats {
            continue;
        }

        // Check that elem is a contiguous run of 1s (possibly rotated)
        let ones = elem.count_ones();
        if ones == 0 || ones == size {
            continue; // All zeros or all ones in element
        }

        // Find rotation: rotate elem right until the least significant bit is 1
        // and the run of 1s starts at bit 0.
        // The `r` we find is the right-rotation from actual -> base.
        // immr is the right-rotation from base -> actual = size - r (mod size).
        let mut found_rotation = false;
        let mut rotation = 0u32;
        for r in 0..size {
            let rot = if r == 0 {
                elem
            } else {
                ((elem >> r) | (elem << (size - r))) & mask
            };
            // Check if this is a contiguous run from bit 0
            let run = rot.trailing_ones();
            if run == ones {
                // r rotates actual -> base, so immr = size - r (mod size) rotates base -> actual
                rotation = if r == 0 { 0 } else { size - r };
                found_rotation = true;
                break;
            }
        }
        if !found_rotation {
            continue;
        }

        // Encode the fields
        let n = if size == 64 { 1u32 } else { 0u32 };
        let immr = rotation;
        let imms = match size {
            2 => 0b111100 | (ones - 1),
            4 => 0b111000 | (ones - 1),
            8 => 0b110000 | (ones - 1),
            16 => 0b100000 | (ones - 1),
            32 => ones - 1,
            64 => ones - 1,
            _ => unreachable!(),
        };

        return Some((n, immr, imms));
    }

    None
}

// ── MUL/DIV ──────────────────────────────────────────────────────────────

pub fn encode_mul(operands: &[Operand]) -> Result<EncodeResult, String> {
    // NEON vector form: MUL Vd.T, Vn.T, Vm.T
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        return encode_neon_mul(operands);
    }
    // MUL Rd, Rn, Rm is MADD Rd, Rn, Rm, XZR
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    check_same_width(is_64, &[rn64, rm64])?;
    let sf = sf_bit(is_64);
    let word = (sf << 31) | (0b0011011000 << 21) | (rm << 16) | (0b11111 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_madd(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    let (ra, ra64) = get_gpr_strict(operands, 3)?;
    check_same_width(is_64, &[rn64, rm64, ra64])?;
    let sf = sf_bit(is_64);
    let word = ((sf << 31) | (0b0011011000 << 21) | (rm << 16)) | (ra << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_msub(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    let (ra, ra64) = get_gpr_strict(operands, 3)?;
    check_same_width(is_64, &[rn64, rm64, ra64])?;
    let sf = sf_bit(is_64);
    let word =
        (sf << 31) | (0b0011011000 << 21) | (rm << 16) | (1 << 15) | (ra << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_div(operands: &[Operand], unsigned: bool) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    check_same_width(is_64, &[rn64, rm64])?;
    let sf = sf_bit(is_64);
    let o1 = if unsigned { 0u32 } else { 1u32 };
    // Data-processing (2 source): sf 0 S=0 11010110 Rm 00001 o1 Rn Rd
    let word = (sf << 31)
        | (0b0011010110 << 21)
        | (rm << 16)
        | (0b00001 << 11)
        | (o1 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode SMULL Xd, Wn, Wm -> SMADDL Xd, Wn, Wm, XZR
pub fn encode_smull(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_gpr_strict_x(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    check_same_width(false, &[rn64, rm64])?;
    // SMADDL: 1 00 11011 001 Rm 0 11111 Rn Rd (Ra=XZR makes it SMULL)
    let word = (1u32 << 31) | (0b0011011001 << 21) | (rm << 16) | (0b011111 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode UMULL Xd, Wn, Wm -> UMADDL Xd, Wn, Wm, XZR
pub fn encode_umull(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_gpr_strict_x(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    check_same_width(false, &[rn64, rm64])?;
    // UMADDL: 1 00 11011 101 Rm 0 11111 Rn Rd (Ra=XZR makes it UMULL)
    let word = (1u32 << 31) | (0b0011011101 << 21) | (rm << 16) | (0b011111 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode SMADDL Xd, Wn, Wm, Xa (signed multiply-add long)
pub fn encode_smaddl(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_gpr_strict_x(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    let ra = get_gpr_strict_x(operands, 3)?;
    check_same_width(false, &[rn64, rm64])?;
    // SMADDL: 1 00 11011 001 Rm 0 Ra Rn Rd
    let word = (1u32 << 31) | (0b0011011001 << 21) | (rm << 16) | (ra << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode UMADDL Xd, Wn, Wm, Xa (unsigned multiply-add long)
pub fn encode_umaddl(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_gpr_strict_x(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    let ra = get_gpr_strict_x(operands, 3)?;
    check_same_width(false, &[rn64, rm64])?;
    // UMADDL: 1 00 11011 101 Rm 0 Ra Rn Rd
    let word = (1u32 << 31) | (0b0011011101 << 21) | (rm << 16) | (ra << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode MNEG Xd, Xn, Xm -> MSUB Xd, Xn, Xm, XZR
pub fn encode_mneg(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    check_same_width(is_64, &[rn64, rm64])?;
    let sf = sf_bit(is_64);
    // MSUB with Ra=XZR: sf 00 11011 000 Rm 1 11111 Rn Rd
    let word = (sf << 31)
        | (0b0011011000 << 21)
        | (rm << 16)
        | (1 << 15)
        | (0b11111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_umulh(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_gpr_strict_x(operands, 0)?;
    let rn = get_gpr_strict_x(operands, 1)?;
    let rm = get_gpr_strict_x(operands, 2)?;
    // UMULH: 1 00 11011 1 10 Rm 0 11111 Rn Rd
    let word = (1u32 << 31) | (0b0011011110 << 21) | (rm << 16) | (0b011111 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_smulh(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_gpr_strict_x(operands, 0)?;
    let rn = get_gpr_strict_x(operands, 1)?;
    let rm = get_gpr_strict_x(operands, 2)?;
    // SMULH: 1 00 11011 0 10 Rm 0 11111 Rn Rd
    let word = (1u32 << 31) | (0b0011011010 << 21) | (rm << 16) | (0b011111 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_neg(operands: &[Operand]) -> Result<EncodeResult, String> {
    // NEG Rd, Rm [, shift #amount] -> SUB Rd, XZR, Rm [, shift #amount]
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rm, rm64) = get_gpr_strict(operands, 1)?;
    check_same_width(is_64, &[rm64])?;
    let sf = sf_bit(is_64);
    let (shift_type, shift_amount) = if let Some(Operand::Shift { kind, amount }) = operands.get(2)
    {
        let st = match kind.as_str() {
            "lsl" => 0b00u32,
            "lsr" => 0b01,
            "asr" => 0b10,
            _ => 0b00,
        };
        (st, *amount)
    } else {
        (0, 0)
    };
    let word = (sf << 31)
        | (1 << 30)
        | (0b01011 << 24)
        | (shift_type << 22)
        | (rm << 16)
        | ((shift_amount & 0x3F) << 10)
        | (0b11111 << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_negs(operands: &[Operand]) -> Result<EncodeResult, String> {
    // NEGS Rd, Rm [, shift #amount] -> SUBS Rd, XZR, Rm [, shift #amount]
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rm, rm64) = get_gpr_strict(operands, 1)?;
    check_same_width(is_64, &[rm64])?;
    let sf = sf_bit(is_64);
    let (shift_type, shift_amount) = if let Some(Operand::Shift { kind, amount }) = operands.get(2)
    {
        let st = match kind.as_str() {
            "lsl" => 0b00u32,
            "lsr" => 0b01,
            "asr" => 0b10,
            _ => 0b00,
        };
        (st, *amount)
    } else {
        (0, 0)
    };
    let word = (sf << 31)
        | (1 << 30)
        | (1 << 29)
        | (0b01011 << 24)
        | (shift_type << 22)
        | (rm << 16)
        | ((shift_amount & 0x3F) << 10)
        | (0b11111 << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_mvn(operands: &[Operand]) -> Result<EncodeResult, String> {
    // NEON vector form: MVN Vd.T, Vn.T (alias of NOT)
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        return encode_neon_not(operands);
    }
    // MVN Rd, Rm [, shift #amount] -> ORN Rd, XZR, Rm [, shift #amount]
    if operands.len() > 3 {
        return Err(format!(
            "mvn takes at most 3 operands, got {}",
            operands.len()
        ));
    }
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rm, rm64) = get_gpr_strict(operands, 1)?;
    check_same_width(is_64, &[rm64])?;
    let sf = sf_bit(is_64);
    let (shift_type, shift_amount) = get_shift_imm6(operands, 2, is_64)?;
    let word = (sf << 31)
        | (0b01 << 29)
        | (0b01010 << 24)
        | (shift_type << 22)
        | (1 << 21)
        | (rm << 16)
        | (shift_amount << 10)
        | (0b11111 << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_adc(operands: &[Operand], set_flags: bool) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    check_same_width(is_64, &[rn64, rm64])?;
    let sf = sf_bit(is_64);
    let s = if set_flags { 1u32 } else { 0 };
    let word = ((sf << 31) | (s << 29) | (0b11010000 << 21) | (rm << 16)) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub fn encode_sbc(operands: &[Operand], set_flags: bool) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    check_same_width(is_64, &[rn64, rm64])?;
    let sf = sf_bit(is_64);
    let s = if set_flags { 1u32 } else { 0 };
    let word =
        ((sf << 31) | (1 << 30) | (s << 29) | (0b11010000 << 21) | (rm << 16)) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

// ── Shifts ───────────────────────────────────────────────────────────────

pub fn encode_shift(operands: &[Operand], shift_type: u32) -> Result<EncodeResult, String> {
    // LSL/LSR/ASR/ROR alias UBFM/SBFM/EXTR or the 2-source register forms:
    // every operand is GP-only, and GNU as rejects an FP/SIMD spelling
    // (`lsl d0,d1,#2` used to assemble as a 32-bit UBFM on register 0).
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rn, rn_is_64) = get_gpr_strict(operands, 1)?;
    if is_64 != rn_is_64 {
        let rd_name = match operands.first() {
            Some(Operand::Reg(n)) => n.as_str(),
            _ => "<rd>",
        };
        let rn_name = match operands.get(1) {
            Some(Operand::Reg(n)) => n.as_str(),
            _ => "<rn>",
        };
        return Err(format!(
            "shift: `{rd_name}` and `{rn_name}` are different widths; \
             the shifted forms need both operands the same size"
        ));
    }

    // LSL/LSR/ASR Rd, Rn, #imm (immediate form -> UBFM/SBFM)
    if let Some(Operand::Imm(imm)) = operands.get(2) {
        let sf = sf_bit(is_64);
        // The immediate must be in 0..width-1. The old code computed
        // `width - 1 - imm` in unsigned arithmetic, so `lsl x0,x1,#64`
        // underflowed to an all-ones imms field and emitted a corrupted
        // word (0xfffffc20) instead of an error. GNU as rejects the range.
        let width = if is_64 { 64 } else { 32 };
        if *imm < 0 || *imm as u32 >= width {
            return Err(format!(
                "shift: immediate {imm} is out of range 0..={}",
                width - 1
            ));
        }
        let imm = *imm as u32;
        let n = if is_64 { 1u32 } else { 0u32 };

        match shift_type {
            0b00 => {
                // LSL #imm -> UBFM Rd, Rn, #(-imm mod width), #(width-1-imm)
                let immr = (width - imm) % width;
                let imms = width - 1 - imm;
                let word = (sf << 31)
                    | (0b10 << 29)
                    | (0b100110 << 23)
                    | (n << 22)
                    | (immr << 16)
                    | (imms << 10)
                    | (rn << 5)
                    | rd;
                return Ok(EncodeResult::Word(word));
            }
            0b01 => {
                // LSR #imm -> UBFM Rd, Rn, #imm, #(width-1)
                let immr = imm;
                let imms = width - 1;
                let word = (sf << 31)
                    | (0b10 << 29)
                    | (0b100110 << 23)
                    | (n << 22)
                    | (immr << 16)
                    | (imms << 10)
                    | (rn << 5)
                    | rd;
                return Ok(EncodeResult::Word(word));
            }
            0b10 => {
                // ASR #imm -> SBFM Rd, Rn, #imm, #(width-1)
                let immr = imm;
                let imms = width - 1;
                let word = (sf << 31)
                    | (0b100110 << 23)
                    | (n << 22)
                    | (immr << 16)
                    | (imms << 10)
                    | (rn << 5)
                    | rd;
                return Ok(EncodeResult::Word(word));
            }
            0b11 => {
                // ROR #imm -> EXTR Rd, Rn, Rn, #imm
                // EXTR: sf 0 0 100111 N 0 Rm imms Rn Rd
                let word = (sf << 31)
                    | (0b00100111 << 23)
                    | (n << 22)
                    | (rn << 16)
                    | (imm << 10)
                    | (rn << 5)
                    | rd;
                return Ok(EncodeResult::Word(word));
            }
            _ => {}
        }
    }

    // LSL/LSR/ASR Rd, Rn, Rm (register form)
    if let Some(Operand::Reg(rm_name)) = operands.get(2) {
        // Same class and width contract as the immediate form: the 2-source
        // encoding has a single sf bit, so `lsl x0,x1,w2` must be diagnosed
        // rather than silently encoded with the destination's width.
        if !is_gp_reg(rm_name) {
            return Err(format!(
                "shift: operand 2 `{rm_name}` is not a general-purpose register \
                 (expected x0-x30, w0-w30, lr, xzr or wzr)"
            ));
        }
        if is_64bit_reg(rm_name) != is_64 {
            return Err(format!(
                "shift: `{rm_name}` does not match the width of the destination; \
                 the register forms need all operands the same size"
            ));
        }
        let rm = parse_reg_num(rm_name).ok_or("invalid rm")?;
        let sf = sf_bit(is_64);
        // Data-processing (2 source): sf 0 S=0 11010110 Rm 0010 op2 Rn Rd
        let op2 = shift_type; // 00=LSL, 01=LSR, 10=ASR, 11=ROR
        let word = (sf << 31)
            | (0b0011010110 << 21)
            | (rm << 16)
            | (0b0010 << 12)
            | (op2 << 10)
            | (rn << 5)
            | rd;
        return Ok(EncodeResult::Word(word));
    }

    Err("unsupported shift operands".to_string())
}

// ── Extensions ───────────────────────────────────────────────────────────

/// Encode SXTW Xd, Wn -> SBFM Xd, Xn, #0, #31.
///
/// The only architecturally defined form (ARMv8 ARM, SXTW): an X
/// destination with a W source. A W destination is unallocated, and the X
/// source spelling is rejected by GNU as ("operand mismatch") and absent
/// from the ARM — both must be diagnosed, not silently re-banked.
pub fn encode_sxtw(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "sxtw requires exactly 2 operands, got {}",
            operands.len()
        ));
    }
    let rd = get_gpr_strict_x(operands, 0)?;
    let rn = get_gpr_strict_w(operands, 1)?;
    // SBFM Xd, Xn, #0, #31: sf=1 opc=00 100110 N=1 immr=0 imms=31
    let word = (1u32 << 31) | (0b100110 << 23) | (1 << 22) | (31 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode SXTH Rd, Wn -> SBFM Rd, Xn, #0, #15 (W or X destination).
pub fn encode_sxth(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "sxth requires exactly 2 operands, got {}",
            operands.len()
        ));
    }
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let rn = get_gpr_strict_w(operands, 1)?;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0 };
    let word = ((sf << 31) | (0b100110 << 23) | (n << 22)) | (15 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode SXTB Rd, Wn -> SBFM Rd, Xn, #0, #7 (W or X destination).
pub fn encode_sxtb(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "sxtb requires exactly 2 operands, got {}",
            operands.len()
        ));
    }
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let rn = get_gpr_strict_w(operands, 1)?;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0 };
    let word = ((sf << 31) | (0b100110 << 23) | (n << 22)) | (7 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode UXTW Wd/Xd, Wn.
///
/// The W destination is the documented `MOV Wd, Wn` alias (ARM ARM
/// "UXTW Wd, Wn"); the X destination zero-extends the 32-bit source and is
/// emitted as the literal alias expansion `UBFM Xd, Xn, #0, #31` (llvm-mc's
/// canonical word; GNU as canonicalises it to the 32-bit MOV instead, which
/// produces the same architectural state -- a known canonicalisation
/// preference, not a defect).
pub fn encode_uxtw(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "uxtw requires exactly 2 operands, got {}",
            operands.len()
        ));
    }
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let rn = get_gpr_strict_w(operands, 1)?;
    if !is_64 {
        // UXTW Wd, Wn -> MOV Wd, Wn: 32-bit ORR with XZR
        let word = (0b01 << 29) | (0b01010 << 24) | (rn << 16) | (0b11111 << 5) | rd;
        return Ok(EncodeResult::Word(word));
    }
    // UXTW Xd, Wn -> UBFM Xd, Xn, #0, #31: sf=1 opc=10 100110 N=1 immr=0 imms=31
    let word =
        (1u32 << 31) | (0b10 << 29) | (0b100110 << 23) | (1 << 22) | (31 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode UXTH Wd/Xd, Wn -> UBFM Rd, Rn, #0, #15.
///
/// The destination may be W or X; only the source is W-only. For the X
/// destination both GNU as and llvm-mc canonicalise to the 32-bit UBFM
/// (writing Wd zeroes the upper half, so the zero-extend is identical);
/// lccc matches that byte-for-byte. (Contrast SXTB/SXTH, where the 64-bit
/// op is required and all three assemblers emit the X form.)
pub fn encode_uxth(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "uxth requires exactly 2 operands, got {}",
            operands.len()
        ));
    }
    // Destination class/width is validated; the encoding is always the
    // 32-bit UBFM (see the doc comment above).
    let (rd, _is_64) = get_gpr_strict(operands, 0)?;
    let rn = get_gpr_strict_w(operands, 1)?;
    let word = (0b10 << 29) | (0b100110 << 23) | (15 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode UXTB Wd/Xd, Wn -> UBFM Rd, Rn, #0, #7.
///
/// The destination may be W or X; only the source is W-only. For the X
/// destination both GNU as and llvm-mc canonicalise to the 32-bit UBFM
/// (writing Wd zeroes the upper half, so the byte zero-extend into Xd is
/// identical); lccc matches that byte-for-byte.
pub fn encode_uxtb(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "uxtb requires exactly 2 operands, got {}",
            operands.len()
        ));
    }
    let (rd, _is_64) = get_gpr_strict(operands, 0)?;
    let rn = get_gpr_strict_w(operands, 1)?;
    let word = (0b10 << 29) | (0b100110 << 23) | (7 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode ORN (logical OR NOT): ORN Rd, Rn, Rm (scalar or vector)
pub fn encode_orn(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("orn requires 3 operands".to_string());
    }

    // NEON vector form: ORN Vd.T, Vn.T, Vm.T
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        let (rd, rn, rm, q) = get_neon_logical_operands(operands, "orn")?;
        // ORN Vd.T, Vn.T, Vm.T: 0 Q 0 01110 11 1 Rm 000111 Rn Rd
        let word = (q << 30)
            | (0b001110 << 24)
            | (0b11 << 22)
            | (1 << 21)
            | (rm << 16)
            | (0b000111 << 10)
            | (rn << 5)
            | rd;
        return Ok(EncodeResult::Word(word));
    }

    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    check_same_width(is_64, &[rn64, rm64])?;
    let sf = sf_bit(is_64);

    if operands.len() > 4 {
        return Err(format!(
            "orn takes at most 4 operands, got {}",
            operands.len()
        ));
    }
    let (shift_type, shift_amount) = get_shift_imm6(operands, 3, is_64)?;

    // ORN Rd, Rn, Rm [, shift #amount]: sf 01 01010 shift 1 Rm imm6 Rn Rd
    let word = (sf << 31)
        | (0b01 << 29)
        | (0b01010 << 24)
        | (shift_type << 22)
        | (1 << 21)
        | (rm << 16)
        | (shift_amount << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode EON (exclusive OR NOT): EON Rd, Rn, Rm [, shift #amount]
pub fn encode_eon(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("eon requires 3 operands".to_string());
    }
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    check_same_width(is_64, &[rn64, rm64])?;
    let sf = sf_bit(is_64);

    if operands.len() > 4 {
        return Err(format!(
            "eon takes at most 4 operands, got {}",
            operands.len()
        ));
    }
    let (shift_type, shift_amount) = get_shift_imm6(operands, 3, is_64)?;

    // EON Rd, Rn, Rm [, shift #amount]: sf 10 01010 shift 1 Rm imm6 Rn Rd (opc=10, N=1)
    let word = (sf << 31)
        | (0b10 << 29)
        | (0b01010 << 24)
        | (shift_type << 22)
        | (1 << 21)
        | (rm << 16)
        | (shift_amount << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode BICS (bitwise clear, setting flags): BICS Rd, Rn, Rm [, shift #amount]
pub fn encode_bics(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("bics requires 3 operands".to_string());
    }
    let (rd, is_64) = get_gpr_strict(operands, 0)?;
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    let (rm, rm64) = get_gpr_strict(operands, 2)?;
    check_same_width(is_64, &[rn64, rm64])?;
    let sf = sf_bit(is_64);

    if operands.len() > 4 {
        return Err(format!(
            "bics takes at most 4 operands, got {}",
            operands.len()
        ));
    }
    let (shift_type, shift_amount) = get_shift_imm6(operands, 3, is_64)?;

    // BICS Rd, Rn, Rm [, shift #amount]: sf 11 01010 shift 1 Rm imm6 Rn Rd
    let word = (sf << 31)
        | (0b11 << 29)
        | (0b01010 << 24)
        | (shift_type << 22)
        | (1 << 21)
        | (rm << 16)
        | (shift_amount << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode BIC instruction - disambiguates between scalar and NEON forms.
/// Scalar register: BIC Xd, Xn, Xm [, shift #amount] -> AND NOT (opc=00, N=1)
/// Scalar immediate: BIC Xd, Xn, #imm -> AND Xd, Xn, #~imm
/// NEON vector: BIC Vd.T, Vn.T, Vm.T
pub fn encode_bic(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("bic requires 3 operands".to_string());
    }

    // NEON vector form: BIC Vd.T, Vn.T, Vm.T
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        return encode_neon_bic(operands);
    }

    // BIC (immediate) is an alias of AND (immediate) with the inverted
    // bitmask, so — exactly like AND/ORR/EOR (immediate) — SP is a legal
    // destination (GNU as: `bic sp, x1, #15` assembles; the register form
    // takes no SP anywhere).
    let sp_ok_in_rd = matches!(operands.get(2), Some(Operand::Imm(_)));
    let (rd, is_64) = if sp_ok_in_rd {
        get_gpr_or_sp(operands, 0)?
    } else {
        get_gpr_strict(operands, 0)?
    };
    let (rn, rn64) = get_gpr_strict(operands, 1)?;
    check_same_width(is_64, &[rn64])?;
    let sf = sf_bit(is_64);

    // BIC Xd, Xn, #imm -> AND Xd, Xn, #~imm (bitmask immediate, inverted)
    if let Some(Operand::Imm(imm)) = operands.get(2) {
        if operands.len() != 3 {
            return Err(format!(
                "bic takes 3 operands for the immediate form, got {}",
                operands.len()
            ));
        }
        let inverted = if is_64 {
            !(*imm as u64)
        } else {
            (!(*imm as u32)) as u64
        };
        if let Some((n, immr, imms)) = encode_bitmask_imm(inverted, is_64) {
            // AND Rd, Rn, #~imm: sf 00 100100 N immr imms Rn Rd
            // AND Rd, Rn, #~imm encoding: sf=bit31, opc=00 (bits29:30), 100100 (bits23:28), N, immr, imms, Rn, Rd
            let word = (sf << 31)
                | (0b100100 << 23)
                | (n << 22)
                | (immr << 16)
                | (imms << 10)
                | (rn << 5)
                | rd;
            return Ok(EncodeResult::Word(word));
        }
        return Err(format!(
            "cannot encode bitmask immediate for bic: 0x{:x} (inverted: 0x{:x})",
            imm, inverted
        ));
    }

    // BIC Xd, Xn, Xm [, shift #amount]: sf 00 01010 shift 1 Rm imm6 Rn Rd (N=1)
    if matches!(operands.get(2), Some(Operand::Reg(_))) {
        // Strict GP validation for Rm: FP/SIMD names and SP must not be
        // silently aliased into the register number space (GNU as rejects
        // `bic w0, w1, s5` / `bic x0, x1, sp` with "operand mismatch").
        let (rm, rm64) = get_gpr_strict(operands, 2)?;
        check_same_width(is_64, &[rn64, rm64])?;

        if operands.len() > 4 {
            return Err(format!(
                "bic takes at most 4 operands, got {}",
                operands.len()
            ));
        }
        let (shift_type, shift_amount) = get_shift_imm6(operands, 3, is_64)?;

        // BIC is AND with N=1 (bit 21): sf opc=00(bits29:30) 01010 shift 1 Rm imm6 Rn Rd
        let word = (sf << 31)
            | (0b01010 << 24)
            | (shift_type << 22)
            | (1 << 21)
            | (rm << 16)
            | (shift_amount << 10)
            | (rn << 5)
            | rd;
        return Ok(EncodeResult::Word(word));
    }

    Err("unsupported bic operands".to_string())
}
