use super::*;
use crate::backend::arm::assembler::parser::Operand;

// ── MOV ──────────────────────────────────────────────────────────────────

pub(crate) fn encode_mov(operands: &[Operand]) -> Result<EncodeResult, String> {
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
        // `mov` with an immediate is an alias, so its destination may be
        // anything one of its expansions can name: MOVZ/MOVN take `Xd|XZR` and
        // the ORR-immediate expansion takes `Xd|SP`.  GNU as accepts
        // `mov sp, #1` (0xb24003ff, an ORR) and `mov xzr, #15` (0xd28001ff, a
        // MOVZ), and rejects `mov d0, #1` -- which used to reach the GP
        // encoding here and assemble as `movz w0, #1`.
        let rd_reg = reg_operand(operands, 0, GpRole::RegSpOrZr, "mov")?;
        let (rd, is_64) = (rd_reg.num, rd_reg.is_64);
        if let Some(Operand::Shift { kind, .. }) = operands.get(2) {
            return Err(format!(
                "mov: `{kind}` is not allowed here; `mov` with an immediate is an \
                 alias and takes no shift.  Use `movz`/`movk` for a shifted \
                 16-bit chunk"
            ));
        }
        // Encoding 31 in a MOVZ/MOVN destination is the zero register, not the
        // stack pointer, so an SP destination can only use the ORR-immediate
        // expansion.
        let wide_form_ok = !rd_reg.is_sp;
        let imm = *imm;

        // GAS's own preference order, reproduced because the encoding is
        // observable: a single MOVZ when the value has one nonzero 16-bit
        // chunk (`mov x0, #0x10000` -> 0xd2a00020, not the ORR it could also
        // use), a single MOVN when the *inverted* value has one chunk
        // (`mov x0, #-2` -> 0x92800020), then the bitmask ORR.
        let width_mask = if is_64 { u64::MAX } else { 0xFFFF_FFFF };
        let chunks = if is_64 { 4 } else { 2 };
        let value = (imm as u64) & width_mask;
        let inverted = (!value) & width_mask;
        let one_chunk = |v: u64| -> Option<(u32, u32)> {
            let mut found = None;
            for hw in 0..chunks {
                let part = ((v >> (hw * 16)) & 0xFFFF) as u32;
                if part != 0 {
                    if found.is_some() {
                        return None;
                    }
                    found = Some((hw, part));
                }
            }
            Some(found.unwrap_or((0, 0)))
        };

        if let Some((hw, imm16)) = wide_form_ok.then(|| one_chunk(value)).flatten() {
            let word = (sf_bit(is_64) << 31) | (0b10100101 << 23) | (hw << 21) | (imm16 << 5) | rd;
            return Ok(EncodeResult::Word(word));
        }
        if let Some((hw, imm16)) = wide_form_ok.then(|| one_chunk(inverted)).flatten() {
            let word = (sf_bit(is_64) << 31) | (0b00100101 << 23) | (hw << 21) | (imm16 << 5) | rd;
            return Ok(EncodeResult::Word(word));
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

        if !wide_form_ok {
            return Err(format!(
                "mov: #{imm} cannot be moved into the stack pointer; it is not a \
                 logical bitmask immediate, and movz/movk have no SP destination"
            ));
        }
        // `mov` never expands to a sequence: GNU as rejects `mov x0, #0x10001`
        // with "immediate cannot be moved by a single instruction" instead of
        // emitting movz+movk, and silently expanding here would make `mov` mean
        // something no other assembler means by it.
        return Err(format!(
            "mov: immediate #{imm} cannot be moved by a single instruction; use \
             `movz`/`movk` to build it, or `mov` with a value that fits one \
             16-bit chunk or a logical bitmask"
        ));
    }

    // mov Xd, Xm -> ORR Xd, XZR, Xm
    if let (Some(Operand::Reg(rd_name)), Some(Operand::Reg(rm_name))) =
        (operands.first(), operands.get(1))
    {
        // `mov` between registers is two aliases that disagree about what
        // encoding 31 means: ORR reads it as the zero register, and the
        // to/from-SP form is an ADD, which reads it as the stack pointer.  A
        // pair mixing the two identities is neither -- GNU as rejects
        // `mov sp, xzr`, `mov xzr, sp`, `mov wsp, wzr` and `mov wzr, wsp` --
        // and encoding one anyway silently substitutes the wrong register.
        let rd_reg = reg_operand(operands, 0, GpRole::RegSpOrZr, "mov")?;
        let rm_reg = reg_operand(operands, 1, GpRole::RegSpOrZr, "mov")?;
        // `lr` is x30, so a 32-bit destination cannot take it (`mov w0, lr`
        // and `mov x30, w1` are rejected by GNU as on width grounds).
        if !rd_reg.same_width_as(rm_reg) {
            return Err(format!(
                "mov: `{rd_name}` and `{rm_name}` are different widths; \
                 a register move needs both operands the same size"
            ));
        }
        let (rd, rm, is_64) = (rd_reg.num, rm_reg.num, rd_reg.is_64);

        // MOV to/from SP uses ADD Xd, Xn, #0.  Both the 64-bit `sp` and the
        // 32-bit `wsp` spelling take this form (GNU as: `mov w0, wsp` ->
        // 0x110003e0, an ADD, not an ORR).
        if rd_reg.is_sp || rm_reg.is_sp {
            if rd_reg.is_zr || rm_reg.is_zr {
                return Err(format!(
                    "mov: `{rd_name}` and `{rm_name}` cannot be combined; the \
                     to/from-SP form is an ADD, where encoding 31 is the stack \
                     pointer, so the zero register has no spelling here"
                ));
            }
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

/// Classification of an `:abs_g*:` modifier for movz/movk (GAS parity).
#[derive(Debug, Clone, Copy)]
pub(crate) struct AbsGSpec {
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
pub(crate) fn abs_g_spec(kind: &str) -> Result<Option<AbsGSpec>, String> {
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

/// Read the halfword selector of a MOVZ/MOVK/MOVN.
///
/// The selector is `lsl #N` with N a multiple of 16 and at most 48 (64-bit) or
/// 16 (32-bit).  It used to be computed as `amount / 16` for `lsl` and as 0 for
/// anything else, so `movz x0, #1, lsr #16` silently became `movz x0, #1`,
/// `movz x0, #1, lsl #15` became the same word as `lsl #0`, and `lsl #64`
/// overflowed the two-bit field into the immediate.
fn movw_shift(operands: &[Operand], is_64: bool, mn: &str) -> Result<u32, String> {
    let Some(Operand::Shift { kind, amount }) = operands.get(2) else {
        return Ok(0);
    };
    let max = if is_64 { 48 } else { 16 };
    if kind != "lsl" || *amount > max || *amount % 16 != 0 {
        return Err(format!(
            "{mn}: `{kind} #{amount}` is not a valid halfword selector; the shift \
             must be `lsl` by a multiple of 16, at most {max}"
        ));
    }
    Ok(*amount / 16)
}

/// Build a MOVZ (is_movz) / MOVK word with the given halfword selector.
pub(crate) fn movw_word(is_movz: bool, rd: u32, is_64: bool, hw: u32, imm16: u32) -> u32 {
    let sf = sf_bit(is_64);
    let base = if is_movz { 0b1010_0101u32 } else { 0b1110_0101 };
    (sf << 31) | (base << 23) | (hw << 21) | ((imm16 & 0xFFFF) << 5) | rd
}

pub(crate) fn encode_movz(operands: &[Operand]) -> Result<EncodeResult, String> {
    // MOVZ Rd is Xd|XZR (or Wd|WZR). A FP destination used to encode as
    // W0 (`movz d0, #1`); SP used to encode as XZR.
    let rd_reg = reg_operand(operands, 0, GpRole::RegOrZr, "movz")?;
    let (rd, is_64) = (rd_reg.num, rd_reg.is_64);
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

    let imm = get_imm(operands, 1)?;
    if imm < 0 || imm > 0xFFFF {
        return Err(format!(
            "movz: immediate #{imm} is out of range for a 16-bit chunk (0 to 65535); \
             use a shift of a 16-bit value, or `mov` for a wider constant"
        ));
    }

    let hw = movw_shift(operands, is_64, "movz")?;

    let word = (sf << 31) | (0b10100101 << 23) | (hw << 21) | ((imm as u32) << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_movk(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd_reg = reg_operand(operands, 0, GpRole::RegOrZr, "movk")?;
    let (rd, is_64) = (rd_reg.num, rd_reg.is_64);

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

    let imm = get_imm(operands, 1)?;
    if imm < 0 || imm > 0xFFFF {
        return Err(format!(
            "movk: immediate #{imm} is out of range for a 16-bit chunk (0 to 65535)"
        ));
    }

    let hw = movw_shift(operands, is_64, "movk")?;

    let sf = sf_bit(is_64);
    let word = (sf << 31) | (0b11100101 << 23) | (hw << 21) | ((imm as u32) << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_movn(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd_reg = reg_operand(operands, 0, GpRole::RegOrZr, "movn")?;
    let (rd, is_64) = (rd_reg.num, rd_reg.is_64);

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

    let imm = get_imm(operands, 1)?;
    if imm < 0 || imm > 0xFFFF {
        return Err(format!(
            "movn: immediate #{imm} is out of range for a 16-bit chunk (0 to 65535)"
        ));
    }
    let sf = sf_bit(is_64);

    let hw = movw_shift(operands, is_64, "movn")?;

    let word = (sf << 31) | (0b00100101 << 23) | (hw << 21) | ((imm as u32) << 5) | rd;
    Ok(EncodeResult::Word(word))
}

// ── ADD/SUB ──────────────────────────────────────────────────────────────

pub(crate) fn encode_add_sub(
    operands: &[Operand],
    is_sub: bool,
    set_flags: bool,
) -> Result<EncodeResult, String> {
    // ADD/SUB are general-purpose: `add x0, x1, h2` has no encoding and GNU as
    // rejects it, but the permissive operand readers resolve `h2` to 2 and
    // assemble it as `add x0, x1, x2`.
    let mn = match (is_sub, set_flags) {
        (true, true) => "subs",
        (true, false) => "sub",
        (false, true) => "adds",
        (false, false) => "add",
    };
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

    // Encoding 31 stands for a different register in each of the three forms,
    // so the operands are read with the widest role any form allows and then
    // re-checked against the form actually selected:
    //
    //   shifted register    Rd|Rn|Rm are Xd|XZR -- no SP anywhere
    //   extended register   Rd|Rn are Xd|SP, Rm is Xm|XZR
    //   immediate           Rd|Rn are Xd|SP
    //
    // GNU as switches to the extended form whenever the stack pointer appears
    // (`add sp, x0, x1` -> 0x8b21601f, UXTX), which is why `add sp, xzr, x1`
    // has no encoding: a slot that reads 31 as SP cannot also be spelled as
    // the zero register.  The flags-setting forms are the mirror image -- their
    // Rd=31 *is* the zero register, so `adds sp, x1, x2` is rejected while
    // `subs xzr, sp, x0` (the `cmp sp, x0` alias) is fine.
    let rd_reg = reg_operand(
        operands,
        0,
        if set_flags {
            GpRole::RegOrZr
        } else {
            GpRole::RegSpOrZr
        },
        mn,
    )?;
    let rn_reg = reg_operand(operands, 1, GpRole::RegSpOrZr, mn)?;
    if !set_flags && (rd_reg.is_sp || rn_reg.is_sp) && (rd_reg.is_zr || rn_reg.is_zr) {
        return Err(format!(
            "{mn}: `{}` and `{}` cannot be combined; with the stack pointer in \
             one slot this form reads encoding 31 as SP, so the zero register \
             has no spelling here",
            operand_spelling(operands, 0),
            operand_spelling(operands, 1)
        ));
    }
    if !rd_reg.same_width_as(rn_reg) {
        return Err(format!(
            "{mn}: `{}` and `{}` are different widths; both operands must be the \
             same size",
            operand_spelling(operands, 0),
            operand_spelling(operands, 1)
        ));
    }
    let (rd, rn, is_64) = (rd_reg.num, rn_reg.num, rd_reg.is_64);
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
        // The immediate form shifts by 0 or 12 bits only, and the value must
        // still fit imm12 *after* the shift.  It used to test `lsl #12` and
        // then mask, so `add x0, x1, #1, lsl #11`, `add x0, x1, #1, lsl #13`
        // and `add x0, x1, #0x1000, lsl #12` all assembled -- into a different
        // constant than the one written.
        let explicit_shift = match operands.get(3) {
            Some(Operand::Shift { kind, amount }) => {
                if kind != "lsl" || *amount != 12 {
                    return Err(format!(
                        "{mn}: the immediate form shifts by 0 or 12 bits only \
                         (got `{kind} #{amount}`)"
                    ));
                }
                if imm_val > 0xFFF {
                    return Err(format!(
                        "{mn}: immediate #{imm_val} does not fit imm12 once \
                         shifted by 12 (0-4095 before the shift)"
                    ));
                }
                true
            }
            Some(_) => return Err(format!("{mn}: operand 3 must be `lsl #12` or nothing")),
            None => false,
        };

        let (imm12, sh) = if explicit_shift {
            // Explicit lsl #12: the immediate is the shifted value.
            (imm_val as u32, 1u32)
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
    if let Some(Operand::Reg(_)) = operands.get(2) {
        // Rm never reads 31 as SP: GNU as rejects `add x0, x1, sp` in both the
        // shifted and the extended form.
        let rm_reg = reg_operand(operands, 2, GpRole::RegOrZr, mn)?;
        let rm = rm_reg.num;

        // A 32-bit destination cannot take a 64-bit source in *either*
        // register form: the shifted form needs all three operands the same
        // width, and GNU as rejects the extended spelling too (`add w0, w1,
        // x2, uxtw` and `add w0, wsp, x1`).  The extend *option* is not
        // cross-checked against the register -- `add x0, x1, w2, uxtx`
        // assembles -- so only the spelling's width is enforced.
        if !is_64 && rm_reg.is_64 {
            return Err(format!(
                "{mn}: `{}` is 64-bit and cannot be the source of a 32-bit {mn}; \
                 write `w{}` for its 32-bit half",
                operand_spelling(operands, 2),
                if rm_reg.is_zr {
                    "zr".to_string()
                } else {
                    rm_reg.num.to_string()
                }
            ));
        }

        // Check for extended register: add Xd, Xn, Wm, sxtw [#N]
        if let Some(Operand::Extend { kind, amount }) = operands.get(3) {
            let option = extend_option(kind, mn)?;
            if *amount > 4 {
                return Err(format!(
                    "{mn}: extend amount #{amount} is out of range (0-4)"
                ));
            }
            // In the extended form encoding 31 in Rd is the stack pointer, so
            // the zero register has no spelling there -- unless the form sets
            // flags, where Rd=31 *is* the zero register and `cmp x0, x1, uxtx`
            // (0xeb21601f) is fine.
            if !set_flags && rd_reg.is_zr {
                return Err(format!(
                    "{mn}: `{}` cannot be the destination of the extended-register \
                     form, where encoding 31 is the stack pointer; write `sp` if \
                     that is what you meant",
                    operand_spelling(operands, 0)
                ));
            }
            let imm3 = *amount;
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
        // register 31 as XZR, not SP, so the extended register form is needed
        // to get SP semantics.
        if (rn_reg.is_sp || rd_reg.is_sp) && operands.len() <= 4 {
            // Extended register form: sf op S 01011 00 1 Rm option imm3 Rn Rd.
            // The option names the *source* width, not the destination's:
            // `add x0, sp, w1` is UXTW (0x8b2143e0) and `add x0, sp, x1` is
            // UXTX (0x8b2163e0).  Deriving it from the destination instead
            // read the 32-bit `w1` as a 64-bit register.
            let option = if rm_reg.is_64 { 0b011u32 } else { 0b010u32 };
            // In this form the imm3 field *is* the shift amount, so an explicit
            // `lsl #N` becomes UXTX #N -- and only `lsl #0`-`#4` exist: GAS
            // assembles `add x0, sp, x1, lsl #2` (0x8b216be0) and rejects
            // `lsl #5` and `lsr #2`.
            let imm3 = match operands.get(3) {
                Some(Operand::Shift { kind, amount }) => {
                    if kind != "lsl" || *amount > 4 {
                        return Err(format!(
                            "{mn}: with the stack pointer this form's shift is the \
                             extended-register amount, which is `lsl #0`-`lsl #4` \
                             only (got `{kind} #{amount}`)"
                        ));
                    }
                    *amount
                }
                Some(_) => {
                    return Err(format!(
                        "{mn}: operand 3 must be a shift (`lsl #0`-`lsl #4`)"
                    ));
                }
                None => 0,
            };
            let word = (((sf << 31) | (op << 30) | (s_bit << 29) | (0b01011 << 24))
                | (1 << 21)
                | (rm << 16)
                | (option << 13)
                | (imm3 << 10))
                | (rn << 5)
                | rd;
            return Ok(EncodeResult::Word(word));
        }

        // The shifted-register form takes no rotate and its amount is bounded
        // by the operand width: `add x0, x1, x2, lsl #64` used to be masked to
        // `lsl #0` and `add x0, x1, x2, ror #3` silently became an LSL.
        if !rm_reg.same_width_as(rd_reg) {
            return Err(format!(
                "{mn}: `{}` and `{}` are different widths; the shifted-register \
                 form needs all three operands the same size",
                operand_spelling(operands, 0),
                operand_spelling(operands, 2)
            ));
        }
        let (shift_type, shift_amount) = match operands.get(3) {
            Some(Operand::Shift { kind, amount }) => shift_field(kind, *amount, is_64, false, mn)?,
            Some(_) => {
                return Err(format!(
                    "{mn}: operand 3 must be a shift (lsl, lsr or asr) or an extend"
                ));
            }
            None => (0, 0),
        };

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

pub(crate) fn encode_logical(
    operands: &[Operand],
    opc: u32,
    mn: &str,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("logical op requires 3 operands".to_string());
    }

    // NEON vector form: ORR/AND/EOR Vd.T, Vn.T, Vm.T
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        return encode_neon_logical(operands, opc);
    }

    // What encoding 31 means in a logical destination depends on whether the
    // form sets flags, and the shifted-register form has no SP at all:
    //
    //   AND/ORR/EOR/BIC immediate   Rd is Xd|SP (never XZR), Rn is Xn|XZR
    //   ANDS/BICS/TST immediate     Rd is Xd|XZR (never SP), Rn is Xn|XZR
    //   shifted-register forms      Rd|Rn|Rm are all Xd|XZR
    //
    // Verified against GNU as: `and sp, x1, #15` (0x92400c3f) and
    // `ands xzr, x1, #15` (0xf2400c3f) assemble, while `and xzr, x1, #15`,
    // `ands sp, x1, #15`, `and x0, sp, #15` and `and sp, x1, x2` do not.
    let imm_form = matches!(operands.get(2), Some(Operand::Imm(_)));
    let rd_reg = reg_operand(
        operands,
        0,
        if imm_form && opc != 0b11 {
            GpRole::RegOrSp
        } else {
            GpRole::RegOrZr
        },
        mn,
    )?;
    let rn_reg = reg_operand(operands, 1, GpRole::RegOrZr, mn)?;
    if !rd_reg.same_width_as(rn_reg) {
        return Err(format!(
            "{mn}: `{}` and `{}` are different widths; both operands must be the \
             same size",
            operand_spelling(operands, 0),
            operand_spelling(operands, 1)
        ));
    }
    let (rd, rn, is_64) = (rd_reg.num, rn_reg.num, rd_reg.is_64);
    let sf = sf_bit(is_64);

    // AND/ORR/EOR Rd, Rn, #imm (bitmask immediate)
    if let Some(Operand::Imm(imm)) = operands.get(2) {
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
    if let Some(Operand::Reg(_)) = operands.get(2) {
        // The shifted-register form takes no SP in any slot: `and x0, sp, x1`
        // and `and sp, x1, x2` are both rejected by GNU as.
        let (_, _, rm_reg) = logical_reg3(operands, mn)?;
        let rm = rm_reg.num;

        // The logical forms are the ones that *do* have a rotate, and the
        // amount is still bounded by the operand width: `orr w0, w1, w2,
        // lsl #32` used to be masked down to `lsl #0`.
        let (shift_type, shift_amount) = match operands.get(3) {
            Some(Operand::Shift { kind, amount }) => shift_field(kind, *amount, is_64, true, mn)?,
            Some(_) => {
                return Err(format!(
                    "{mn}: operand 3 must be a shift (lsl, lsr, asr or ror)"
                ));
            }
            None => (0, 0),
        };

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
pub(crate) fn encode_bitmask_imm(val: u64, is_64: bool) -> Option<(u32, u32, u32)> {
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

pub(crate) fn encode_mul(operands: &[Operand]) -> Result<EncodeResult, String> {
    // NEON vector form: MUL Vd.T, Vn.T, Vm.T
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        return encode_neon_mul(operands);
    }
    // MUL Rd, Rn, Rm is MADD Rd, Rn, Rm, XZR
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    let sf = sf_bit(is_64);
    let word = (sf << 31) | (0b0011011000 << 21) | (rm << 16) | (0b11111 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_madd(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    let (ra, _) = get_reg(operands, 3)?;
    let sf = sf_bit(is_64);
    let word = ((sf << 31) | (0b0011011000 << 21) | (rm << 16)) | (ra << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_msub(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    let (ra, _) = get_reg(operands, 3)?;
    let sf = sf_bit(is_64);
    let word =
        (sf << 31) | (0b0011011000 << 21) | (rm << 16) | (1 << 15) | (ra << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_div(operands: &[Operand], unsigned: bool) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
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
pub(crate) fn encode_smull(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    // SMADDL: 1 00 11011 001 Rm 0 11111 Rn Rd (Ra=XZR makes it SMULL)
    let word = (1u32 << 31) | (0b0011011001 << 21) | (rm << 16) | (0b011111 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode UMULL Xd, Wn, Wm -> UMADDL Xd, Wn, Wm, XZR
pub(crate) fn encode_umull(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    // UMADDL: 1 00 11011 101 Rm 0 11111 Rn Rd (Ra=XZR makes it UMULL)
    let word = (1u32 << 31) | (0b0011011101 << 21) | (rm << 16) | (0b011111 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode SMADDL Xd, Wn, Wm, Xa (signed multiply-add long)
pub(crate) fn encode_smaddl(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    let (ra, _) = get_reg(operands, 3)?;
    // SMADDL: 1 00 11011 001 Rm 0 Ra Rn Rd
    let word = (1u32 << 31) | (0b0011011001 << 21) | (rm << 16) | (ra << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode UMADDL Xd, Wn, Wm, Xa (unsigned multiply-add long)
pub(crate) fn encode_umaddl(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    let (ra, _) = get_reg(operands, 3)?;
    // UMADDL: 1 00 11011 101 Rm 0 Ra Rn Rd
    let word = (1u32 << 31) | (0b0011011101 << 21) | (rm << 16) | (ra << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode MNEG Xd, Xn, Xm -> MSUB Xd, Xn, Xm, XZR
pub(crate) fn encode_mneg(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
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

pub(crate) fn encode_umulh(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    // UMULH: 1 00 11011 1 10 Rm 0 11111 Rn Rd
    let word = (1u32 << 31) | (0b0011011110 << 21) | (rm << 16) | (0b011111 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_smulh(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    // SMULH: 1 00 11011 0 10 Rm 0 11111 Rn Rd
    let word = (1u32 << 31) | (0b0011011010 << 21) | (rm << 16) | (0b011111 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_neg(operands: &[Operand]) -> Result<EncodeResult, String> {
    // NEG Rd, Rm [, shift #amount] -> SUB Rd, XZR, Rm [, shift #amount].  It
    // inherits the shifted-register rules: no SP, no rotate (`neg x0, x1,
    // ror #3` is rejected by GNU as) and an amount bounded by the width.
    let (rd_reg, rm_reg) = (
        reg_operand(operands, 0, GpRole::RegOrZr, "neg")?,
        reg_operand(operands, 1, GpRole::RegOrZr, "neg")?,
    );
    if !rd_reg.same_width_as(rm_reg) {
        return Err(format!(
            "neg: `{}` and `{}` are different widths; both operands must be the \
             same size",
            operand_spelling(operands, 0),
            operand_spelling(operands, 1)
        ));
    }
    let (rd, rm, is_64) = (rd_reg.num, rm_reg.num, rd_reg.is_64);
    let sf = sf_bit(is_64);
    let (shift_type, shift_amount) = match operands.get(2) {
        Some(Operand::Shift { kind, amount }) => shift_field(kind, *amount, is_64, false, "neg")?,
        Some(_) => return Err("neg: operand 2 must be a shift (lsl, lsr or asr)".to_string()),
        None => (0, 0),
    };
    let word = (sf << 31)
        | (1 << 30)
        | (0b01011 << 24)
        | (shift_type << 22)
        | (rm << 16)
        | (shift_amount << 10)
        | (0b11111 << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_negs(operands: &[Operand]) -> Result<EncodeResult, String> {
    // NEGS Rd, Rm [, shift #amount] -> SUBS Rd, XZR, Rm [, shift #amount]
    let (rd_reg, rm_reg) = (
        reg_operand(operands, 0, GpRole::RegOrZr, "negs")?,
        reg_operand(operands, 1, GpRole::RegOrZr, "negs")?,
    );
    if !rd_reg.same_width_as(rm_reg) {
        return Err(format!(
            "negs: `{}` and `{}` are different widths; both operands must be the \
             same size",
            operand_spelling(operands, 0),
            operand_spelling(operands, 1)
        ));
    }
    let (rd, rm, is_64) = (rd_reg.num, rm_reg.num, rd_reg.is_64);
    let sf = sf_bit(is_64);
    let (shift_type, shift_amount) = match operands.get(2) {
        Some(Operand::Shift { kind, amount }) => shift_field(kind, *amount, is_64, false, "negs")?,
        Some(_) => return Err("negs: operand 2 must be a shift (lsl, lsr or asr)".to_string()),
        None => (0, 0),
    };
    let word = (sf << 31)
        | (1 << 30)
        | (1 << 29)
        | (0b01011 << 24)
        | (shift_type << 22)
        | (rm << 16)
        | (shift_amount << 10)
        | (0b11111 << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_mvn(operands: &[Operand]) -> Result<EncodeResult, String> {
    // NEON vector form: MVN Vd.T, Vn.T (alias of NOT)
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        return encode_neon_not(operands);
    }
    // MVN Rd, Rm [, shift #amount] -> ORN Rd, XZR, Rm [, shift #amount].
    // Being an ORN alias it does have a rotate, and the amount is still
    // bounded: `mvn x0, x1, lsl #64` is rejected by GNU as.
    let (rd_reg, rm_reg) = (
        reg_operand(operands, 0, GpRole::RegOrZr, "mvn")?,
        reg_operand(operands, 1, GpRole::RegOrZr, "mvn")?,
    );
    if !rd_reg.same_width_as(rm_reg) {
        return Err(format!(
            "mvn: `{}` and `{}` are different widths; both operands must be the \
             same size",
            operand_spelling(operands, 0),
            operand_spelling(operands, 1)
        ));
    }
    let (rd, rm, is_64) = (rd_reg.num, rm_reg.num, rd_reg.is_64);
    let sf = sf_bit(is_64);
    let (shift_type, shift_amount) = match operands.get(2) {
        Some(Operand::Shift { kind, amount }) => shift_field(kind, *amount, is_64, true, "mvn")?,
        Some(_) => return Err("mvn: operand 2 must be a shift (lsl, lsr, asr or ror)".to_string()),
        None => (0, 0),
    };
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

pub(crate) fn encode_adc(operands: &[Operand], set_flags: bool) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    let sf = sf_bit(is_64);
    let s = if set_flags { 1u32 } else { 0 };
    let word = ((sf << 31) | (s << 29) | (0b11010000 << 21) | (rm << 16)) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_sbc(operands: &[Operand], set_flags: bool) -> Result<EncodeResult, String> {
    let (rd, is_64) = get_reg(operands, 0)?;
    let (rn, _) = get_reg(operands, 1)?;
    let (rm, _) = get_reg(operands, 2)?;
    let sf = sf_bit(is_64);
    let s = if set_flags { 1u32 } else { 0 };
    let word =
        ((sf << 31) | (1 << 30) | (s << 29) | (0b11010000 << 21) | (rm << 16)) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

// ── Shifts ───────────────────────────────────────────────────────────────

pub(crate) fn encode_shift(operands: &[Operand], shift_type: u32) -> Result<EncodeResult, String> {
    let mn = match shift_type {
        0b00 => "lsl",
        0b01 => "lsr",
        0b10 => "asr",
        _ => "ror",
    };
    let rd_reg = reg_operand(operands, 0, GpRole::RegOrZr, mn)?;
    let rn_reg = reg_operand(operands, 1, GpRole::RegOrZr, mn)?;
    if !rd_reg.same_width_as(rn_reg) {
        return Err(format!(
            "{mn}: `{}` and `{}` are different widths; both operands must be the \
             same size",
            operand_spelling(operands, 0),
            operand_spelling(operands, 1)
        ));
    }
    let (rd, rn, is_64) = (rd_reg.num, rn_reg.num, rd_reg.is_64);
    let width: i64 = if is_64 { 64 } else { 32 };

    // LSL/LSR/ASR Rd, Rn, #imm (immediate form -> UBFM/SBFM)
    if let Some(Operand::Imm(imm)) = operands.get(2) {
        let sf = sf_bit(is_64);
        // The amount is a bit index into a `width`-bit register, so `#width`
        // is out of range.  It used to be accepted and `width - 1 - imm`
        // underflowed, so `lsl x0, x1, #64` emitted the corrupt word
        // 0xfffffc20 instead of an error.
        if *imm < 0 || *imm >= width {
            return Err(format!(
                "{mn}: shift amount #{} is out of range for a {width}-bit operand \
                 (0-{})",
                imm,
                width - 1
            ));
        }
        let imm = *imm as u32;
        let width = width as u32;
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
    if let Some(Operand::Reg(_)) = operands.get(2) {
        let rm_reg = reg_operand(operands, 2, GpRole::RegOrZr, mn)?;
        if !rm_reg.same_width_as(rd_reg) {
            return Err(format!(
                "{mn}: `{}` and `{}` are different widths; all three operands \
                 must be the same size",
                operand_spelling(operands, 0),
                operand_spelling(operands, 2)
            ));
        }
        let rm = rm_reg.num;
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

/// Read the two operands of the sign/zero-extend aliases (`sxtb`, `sxth`,
/// `sxtw`, `uxtb`, `uxth`).
///
/// Every one of these is an SBFM/UBFM alias whose *source* is a 32-bit
/// register, so the source must be spelled `w0`-`w30` or `wzr`: GNU as rejects
/// `sxtb x0, x1`, `sxtb x0, sp`, `sxtb x0, lr` and `sxtb x0, xzr`, and accepts
/// `sxtb x0, wzr`.  Exempting `sp`/`lr` from that rule -- as the previous check
/// did -- turned `sxth x0, sp` into a real encoding of a nonexistent
/// instruction.
///
/// The destination may be either width, because `sxtb w0, w1` is the 32-bit
/// SBFM; `sxtw` is the exception and only exists in 64-bit.
fn extend_alias_pair(
    operands: &[Operand],
    mn: &str,
    dest_64_only: bool,
) -> Result<(u32, bool, u32), String> {
    let rd = reg_operand(operands, 0, GpRole::RegOrZr, mn)?;
    let rn = reg_operand(operands, 1, GpRole::RegOrZr, mn)?;
    if rn.is_64 {
        return Err(format!(
            "{mn}: source `{}` must be the 32-bit (w) form; {} extends a W \
             register into {}",
            operand_spelling(operands, 1),
            mn.to_ascii_uppercase(),
            if rd.is_64 { "an X" } else { "a W" }
        ));
    }
    if dest_64_only && !rd.is_64 {
        return Err(format!(
            "{mn}: destination `{}` must be 64-bit; {} only has an `xd, wn` form",
            operand_spelling(operands, 0),
            mn.to_ascii_uppercase()
        ));
    }
    Ok((rd.num, rd.is_64, rn.num))
}

pub(crate) fn encode_sxtw(operands: &[Operand]) -> Result<EncodeResult, String> {
    // SXTW Xd, Wn -> SBFM Xd, Xn, #0, #31
    let (rd, _, rn) = extend_alias_pair(operands, "sxtw", true)?;
    let word = ((1u32 << 31) | (0b100110 << 23) | (1 << 22)) | (31 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_sxth(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64, rn) = extend_alias_pair(operands, "sxth", false)?;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0 };
    let word = ((sf << 31) | (0b100110 << 23) | (n << 22)) | (15 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_sxtb(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, is_64, rn) = extend_alias_pair(operands, "sxtb", false)?;
    let sf = sf_bit(is_64);
    let n = if is_64 { 1u32 } else { 0 };
    let word = ((sf << 31) | (0b100110 << 23) | (n << 22)) | (7 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_uxtw(operands: &[Operand]) -> Result<EncodeResult, String> {
    // ARM ARM C6 documents UXTW as the 64-bit UBFM alias
    // `UBFM Xd, Xn, #0, #31` (word 0xd3407c20 for `uxtw x0, w1`) and does
    // not list a 32-bit-destination form.  GNU as does not implement that
    // alias.  Measured against Debian GNU as 2.44 (and unchanged on the
    // 2.47 pin when present):
    //
    //   uxtw x0, w1   ->  0x2a0103e0   (== `mov w0, w1` / ORR Wd, WZR, Wm)
    //   uxtw w0, w1   ->  0x2a0103e0   (same word; GAS accepts the W dest)
    //   uxtw x0, x1   ->  rejected
    //   ubfm x0, x1, #0, #31  ->  0xd3407c20   (the ARM/llvm-mc encoding)
    //
    // A 32-bit write already zero-extends into Xd, so the MOV encoding is
    // semantically the zero-extend; it is the *word* GAS emits, which is
    // the assembler-parity contract.  llvm-mc / keystone / the ARM ARM
    // preferred disassembly are a different encoding of the same write
    // and must not be used as a reject-side oracle here (they also
    // reject `uxtw w0, w1`, which GAS accepts).
    let rd = reg_operand(operands, 0, GpRole::RegOrZr, "uxtw")?;
    let rn = reg_operand(operands, 1, GpRole::RegOrZr, "uxtw")?;
    if rn.is_64 {
        return Err(format!(
            "uxtw: source `{}` must be the 32-bit (w) form; GNU as encodes UXTW \
             as `mov wd, wn`",
            operand_spelling(operands, 1)
        ));
    }
    // 32-bit ORR Rd, WZR, Rm -- the MOV alias GNU as actually emits.
    let word = (0b001010100 << 23) | (rn.num << 16) | (0b11111 << 5) | rd.num;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_uxth(operands: &[Operand]) -> Result<EncodeResult, String> {
    // Like UXTW, the zero-extending aliases are 32-bit UBFM: GNU as encodes
    // `UXTH x0, w1` and `uxth w0, w1` to the same word (0x53003c20), so the
    // destination's `x` spelling does not widen the operation.
    let (rd, _, rn) = extend_alias_pair(operands, "uxth", false)?;
    let word = ((0b10 << 29) | (0b100110 << 23)) | (15 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_uxtb(operands: &[Operand]) -> Result<EncodeResult, String> {
    // Like UXTW, the zero-extending aliases are 32-bit UBFM: GNU as encodes
    // `UXTB x0, w1` and `uxtb w0, w1` to the same word (0x53001c20), so the
    // destination's `x` spelling does not widen the operation.
    let (rd, _, rn) = extend_alias_pair(operands, "uxtb", false)?;
    let word = ((0b10 << 29) | (0b100110 << 23)) | (7 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode ORN (logical OR NOT): ORN Rd, Rn, Rm (scalar or vector)
pub(crate) fn encode_orn(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("orn requires 3 operands".to_string());
    }

    // NEON vector form: ORN Vd.T, Vn.T, Vm.T
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        let (rd, arr_d) = get_neon_reg(operands, 0)?;
        let (rn, _) = get_neon_reg(operands, 1)?;
        let (rm, _) = get_neon_reg(operands, 2)?;
        let q: u32 = if arr_d == "16b" { 1 } else { 0 };
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

    // `orn` is general-purpose only, and all three of its slots read encoding
    // 31 as the zero register, so no SP is encodable and all three operands
    // must share one width.  `orn x0, x1, d2` used to reach the encoder and
    // assemble as a real instruction that GNU as rejects.
    let (rd_reg, rn_reg, rm_reg) = logical_reg3(operands, "orn")?;
    let (rd, rn, rm, is_64) = (rd_reg.num, rn_reg.num, rm_reg.num, rd_reg.is_64);
    let sf = sf_bit(is_64);

    let (shift_type, shift_amount) = match operands.get(3) {
        Some(Operand::Shift { kind, amount }) => shift_field(kind, *amount, is_64, true, "orn")?,
        Some(_) => {
            return Err(format!(
                "orn: operand 3 must be a shift (lsl, lsr, asr or ror)"
            ));
        }
        None => (0, 0),
    };

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
pub(crate) fn encode_eon(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("eon requires 3 operands".to_string());
    }
    // `eon` is general-purpose only, and all three of its slots read encoding
    // 31 as the zero register, so no SP is encodable and all three operands
    // must share one width.  `eon x0, x1, d2` used to reach the encoder and
    // assemble as a real instruction that GNU as rejects.
    let (rd_reg, rn_reg, rm_reg) = logical_reg3(operands, "eon")?;
    let (rd, rn, rm, is_64) = (rd_reg.num, rn_reg.num, rm_reg.num, rd_reg.is_64);
    let sf = sf_bit(is_64);

    let (shift_type, shift_amount) = match operands.get(3) {
        Some(Operand::Shift { kind, amount }) => shift_field(kind, *amount, is_64, true, "eon")?,
        Some(_) => {
            return Err(format!(
                "eon: operand 3 must be a shift (lsl, lsr, asr or ror)"
            ));
        }
        None => (0, 0),
    };

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
pub(crate) fn encode_bics(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("bics requires 3 operands".to_string());
    }
    // `bics` is general-purpose only, and all three of its slots read encoding
    // 31 as the zero register, so no SP is encodable and all three operands
    // must share one width.  `bics x0, x1, d2` used to reach the encoder and
    // assemble as a real instruction that GNU as rejects.
    let (rd_reg, rn_reg, rm_reg) = logical_reg3(operands, "bics")?;
    let (rd, rn, rm, is_64) = (rd_reg.num, rn_reg.num, rm_reg.num, rd_reg.is_64);
    let sf = sf_bit(is_64);

    let (shift_type, shift_amount) = match operands.get(3) {
        Some(Operand::Shift { kind, amount }) => shift_field(kind, *amount, is_64, true, "bics")?,
        Some(_) => {
            return Err(format!(
                "bics: operand 3 must be a shift (lsl, lsr, asr or ror)"
            ));
        }
        None => (0, 0),
    };

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
pub(crate) fn encode_bic(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("bic requires 3 operands".to_string());
    }

    // NEON vector form: BIC Vd.T, Vn.T, Vm.T
    if let Some(Operand::RegArrangement { .. }) = operands.first() {
        return encode_neon_bic(operands);
    }

    // BIC (immediate) is the AND-with-inverted-mask alias, so like the other
    // non-flags logical immediates its destination may be the stack pointer
    // (`bic sp, x1, #15` -> 0x927cec3f) and may not be the zero register
    // (`bic wzr, w1, #15` is rejected).  The register form takes no SP at all.
    let imm_form = matches!(operands.get(2), Some(Operand::Imm(_)));
    let rd_reg = reg_operand(
        operands,
        0,
        if imm_form {
            GpRole::RegOrSp
        } else {
            GpRole::RegOrZr
        },
        "bic",
    )?;
    let rn_reg = reg_operand(operands, 1, GpRole::RegOrZr, "bic")?;
    if !rd_reg.same_width_as(rn_reg) {
        return Err(format!(
            "bic: `{}` and `{}` are different widths; both operands must be the \
             same size",
            operand_spelling(operands, 0),
            operand_spelling(operands, 1)
        ));
    }
    let (rd, rn, is_64) = (rd_reg.num, rn_reg.num, rd_reg.is_64);
    let sf = sf_bit(is_64);

    // BIC Xd, Xn, #imm -> AND Xd, Xn, #~imm (bitmask immediate, inverted)
    if let Some(Operand::Imm(imm)) = operands.get(2) {
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
    if let Some(Operand::Reg(_)) = operands.get(2) {
        let (_, _, rm_reg) = logical_reg3(operands, "bic")?;
        let rm = rm_reg.num;

        let (shift_type, shift_amount) = match operands.get(3) {
            Some(Operand::Shift { kind, amount }) => {
                shift_field(kind, *amount, is_64, true, "bic")?
            }
            Some(_) => {
                return Err("bic: operand 3 must be a shift (lsl, lsr, asr or ror)".to_string());
            }
            None => (0, 0),
        };

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
