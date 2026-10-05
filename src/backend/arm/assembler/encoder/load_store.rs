use super::*;
use crate::backend::arm::assembler::parser::Operand;

// ── Loads/Stores ─────────────────────────────────────────────────────────

// ── Checked offset fields ──────────────────────────────────────────────────
//
// The AArch64 load/store immediate fields are NARROW and SIGNED, and the
// original code masked every offset into them with `& 0x1FF` / `& 0x7F`.
// Masking cannot fail, so an offset that does not fit was silently wrapped:
//
// ldr x0, [x1, #32768] -> scaled imm12 needs 4096 (too big), so it fell
// through to the unscaled form, where
// 32768 & 0x1FF == 0, assembling `ldur x0, [x1]`
// -- a load from the base register instead of
// base+32768, with no diagnostic.
// stp x0, x1, [x2, #8192] -> (8192 >> 3) & 0x7F == 0, storing to [x2].
//
// Both are the same defect class as the RISC-V branch truncation fixed
// elsewhere in this tree: an immediate that does not fit is mangled into a
// plausible-looking instruction instead of being refused.
//
// These helpers are the single place that decision is made. Every caller must
// go through one of them; the `& MASK` idiom is not to be reintroduced.

/// Encode `offset` as the signed 9-bit immediate of an unscaled, pre-index or
/// post-index load/store. Legal range is -256..=255.
pub(crate) fn checked_imm9(offset: i64, what: &str) -> Result<u32, String> {
    if !(-256..=255).contains(&offset) {
        return Err(format!(
            "{what}: offset {offset} is outside the signed 9-bit immediate of this addressing mode (allowed -256..=255); it would silently wrap. Use a register offset, or materialise the address into a register."
        ));
    }
    Ok((offset as u32) & 0x1FF)
}

/// Encode `offset` as the scaled signed 7-bit immediate of a load/store pair.
/// `shift` is log2 of the access width, so the legal *byte* range is
/// -64*2^shift ..= 63*2^shift, in steps of 2^shift.
pub(crate) fn checked_imm7(offset: i64, shift: u32, what: &str) -> Result<u32, String> {
    let align = 1i64 << shift;
    if offset % align != 0 {
        return Err(format!(
            "{what}: offset {offset} is not a multiple of {align}; this pair accesses {align}-byte elements and the immediate is scaled"
        ));
    }
    let scaled = offset >> shift;
    if !(-64..=63).contains(&scaled) {
        return Err(format!(
            "{what}: offset {offset} scales to {scaled}, which is outside the signed 7-bit immediate of a load/store pair (allowed -64..=63, i.e. {}..={} bytes); it would silently wrap",
            -64 * align,
            63 * align
        ));
    }
    Ok((scaled as u32) & 0x7F)
}

/// Auto-detect LDR/STR size from the first register operand.
pub fn encode_ldr_str_auto(operands: &[Operand], is_load: bool) -> Result<EncodeResult, String> {
    // Determine size from register: Wn -> 32-bit (size=10), Xn -> 64-bit (size=11)
    // FP: Sn -> 32-bit, Dn -> 64-bit, Qn -> 128-bit
    let reg_name = match operands.first() {
        Some(Operand::Reg(r)) => r.to_lowercase(),
        _ => return Err("ldr/str needs register operand".to_string()),
    };

    // `b` and `h` were simply absent from this table, so they fell through to
    // the 64-bit default and every byte/halfword FP access was encoded as a
    // doubleword one: `str b9, [x10]` assembled to the same word as
    // `str d9, [x10]`. The default is now an error rather than a guess --
    // silently picking a width is how the original defect stayed invisible.
    let (size, is_128bit) = if reg_name == "sp" || reg_name == "xzr" || reg_name == "lr" {
        (0b11, false) // 64-bit GPR
    } else if reg_name == "wsp" || reg_name == "wzr" {
        (0b10, false) // 32-bit GPR
    } else {
        match reg_name.chars().next() {
            Some('w') => (0b10, false), // 32-bit GPR
            Some('x') => (0b11, false), // 64-bit GPR
            Some('b') => (0b00, false), // 8-bit FP
            Some('h') => (0b01, false), // 16-bit FP
            Some('s') => (0b10, false), // 32-bit FP
            Some('d') => (0b11, false), // 64-bit FP
            // 128-bit: size=00 with the opc adjustment applied in encode_ldr_str.
            // Only `q` is a scalar 128-bit spelling here: `str v9,[x10]` is an
            // error in GNU as (the SIMD register vector has to name an
            // arrangement to be a register, and ldr/str take no arrangement),
            // so accepting it would invent an encoding for `str q9,[x10]`.
            Some('q') => (0b00, true),
            Some('v') => {
                return Err(
                    "ldr/str: a SIMD register needs a width here -- write q0-q31 \
                     for a 128-bit access (a bare v register is not an operand)"
                        .to_string(),
                );
            }
            _ => {
                return Err(format!(
                    "ldr/str: unrecognised register `{reg_name}` (expected w, x, b, h, s, d or q)"
                ));
            }
        }
    };

    encode_ldr_str(operands, is_load, size, false, is_128bit, false)
}

pub fn encode_ldr_str(
    operands: &[Operand],
    is_load: bool,
    size: u32,
    is_signed: bool,
    is_128bit: bool,
    gpr_byte: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("ldr/str requires at least 2 operands".to_string());
    }

    // Rt may be FP/SIMD (LDR D0/[Q0] are real encodings) but never SP/WSP:
    // field 31 of Rt reads as the zero register in this encoding, so
    // `str sp,[x0]` used to silently store XZR's 0 instead of the stack
    // pointer. GNU as rejects both spellings.
    if let Some(Operand::Reg(r)) = operands.first() {
        if r.eq_ignore_ascii_case("sp") || r.eq_ignore_ascii_case("wsp") {
            return Err(format!(
                "ldr/str: sp/wsp is not valid as the data register \
                 (field 31 of Rt reads as the zero register)"
            ));
        }
    }

    let (rt, rt_is_64) = get_reg(operands, 0)?;
    let fp = is_fp_reg(
        operands
            .first()
            .map(|o| match o {
                Operand::Reg(r) => r.as_str(),
                _ => "",
            })
            .unwrap_or(""),
    );

    // The byte and halfword general-purpose mnemonics take a 32-bit
    // destination: `LDRH X0, [X1]` has no encoding (only the *sign-extending*
    // LDRSH/LDRSB pair widens to X), and an FP/SIMD register would be a
    // different instruction entirely (`ldrh q0,[x1]` used to assemble as
    // `LDR Q0,[X1]`, a 128-bit load).
    //
    // The discriminator is the MNEMONIC, and it has to be: `str b9,[x10]` and
    // `strb b9,[x10]` are the same operands, and GNU as accepts the first
    // (0x3d000149, the FP 8-bit store) while rejecting the second (the scaled
    // FP byte access is spelled `str b9`, not `strb b9`).  Testing the operand
    // class instead -- "size 00/01 with an FP register is an error" -- rejected
    // both, which is what this flag replaces: the FP byte and halfword forms
    // arrive here with `gpr_byte = false` and must be encoded as the FP access
    // they name.
    if gpr_byte && (fp || rt_is_64) {
        return Err(format!(
            "{}: the data register must be a 32-bit general-purpose register \
             (w0-w30); the FP byte/halfword accesses are spelled `ldr b0`/`ldr \
             h0`, and the sign-extending forms ldrsb/ldrsh widen to an x \
             destination",
            if is_load { "ldrb/ldrh" } else { "strb/strh" }
        ));
    }
    if is_128bit && matches!(size, 0b00 | 0b01) && !fp {
        // 128-bit accesses are the `q` spelling: `size == 00` with the opc
        // adjustment.  A b/h register cannot be 128-bit, so this is only
        // reachable through a caller passing a mismatched pair.
        return Err("ldr/str: a 128-bit access needs a q register".to_string());
    }

    // Use the size parameter as-is (auto-detection happens in encode_ldr_str_auto)
    let actual_size = size;

    let v = if fp { 1u32 } else { 0u32 };

    match operands.get(1) {
        // [base, #offset]
        Some(Operand::Mem { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;

            // Unsigned offset encoding
            // Size determines the shift for offset alignment
            // For 128-bit Q registers: shift=4, opc=11 (load) or 10 (store)
            let shift = if is_128bit { 4 } else { actual_size };
            let opc = if is_128bit {
                if is_load { 0b11 } else { 0b10 }
            } else if is_load {
                if is_signed { 0b10 } else { 0b01 }
            } else {
                0b00
            };

            // Check if offset is aligned and fits in 12-bit unsigned field
            let abs_offset = *offset as u64;
            let align = 1u64 << shift;
            if *offset >= 0 && abs_offset.is_multiple_of(align) {
                let imm12 = (abs_offset / align) as u32;
                if imm12 < 4096 {
                    // Unsigned offset form: size 111 V 01 opc imm12 Rn Rt
                    let word = (actual_size << 30)
                        | (0b111 << 27)
                        | (v << 26)
                        | (0b01 << 24)
                        | (opc << 22)
                        | (imm12 << 10)
                        | (rn << 5)
                        | rt;
                    return Ok(EncodeResult::Word(word));
                }
            }

            // Unscaled offset (LDUR/STUR form).
            //
            // The unscaled form *does* exist for 128-bit accesses: `size == 00`
            // with `opc == 10` is STUR/LDUR of a Q register. `str q0,[x1,#8]`
            // is therefore legal (GAS emits 3c808020) even though 8 is not a
            // multiple of the Q access width -- it simply cannot use the
            // scaled form. Do not "helpfully" reject it.
            let imm9 = checked_imm9(*offset, "ldr/str")?;
            let opc = if is_128bit {
                if is_load { 0b11 } else { 0b10 }
            } else if is_load {
                if is_signed { 0b10 } else { 0b01 }
            } else {
                0b00
            };
            let word = (((actual_size << 30) | (0b111 << 27) | (v << 26))
                | (opc << 22)
                | ((imm9 as u32 & 0x1FF) << 12))
                | (rn << 5)
                | rt;
            return Ok(EncodeResult::Word(word));
        }

        // [base, #offset]! (pre-index)
        Some(Operand::MemPreIndex { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            let imm9 = checked_imm9(*offset, if is_load { "ldr/str" } else { "ldr/str" })?;
            let opc = if is_128bit {
                if is_load { 0b11 } else { 0b10 }
            } else if is_load {
                0b01
            } else {
                0b00
            };
            let word = ((actual_size << 30) | (0b111 << 27) | (v << 26))
                | (opc << 22)
                | ((imm9 as u32 & 0x1FF) << 12)
                | (0b11 << 10)
                | (rn << 5)
                | rt;
            return Ok(EncodeResult::Word(word));
        }

        // [base], #offset (post-index)
        Some(Operand::MemPostIndex { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            let imm9 = checked_imm9(*offset, if is_load { "ldr/str" } else { "ldr/str" })?;
            let opc = if is_128bit {
                if is_load { 0b11 } else { 0b10 }
            } else if is_load {
                0b01
            } else {
                0b00
            };
            let word = ((actual_size << 30) | (0b111 << 27) | (v << 26))
                | (opc << 22)
                | ((imm9 as u32 & 0x1FF) << 12)
                | (0b01 << 10)
                | (rn << 5)
                | rt;
            return Ok(EncodeResult::Word(word));
        }

        // [base, Xm] register offset
        Some(Operand::MemRegOffset {
            base,
            index,
            extend,
            shift,
        }) => {
            // Check if index is a :lo12: modifier
            if index.starts_with(':') {
                // Parse modifier from the index string
                let rn = parse_reg_num(base).ok_or("invalid base reg")?;
                let mod_str = index.trim_start_matches(':');
                let (kind, sym) = if let Some(colon_pos) = mod_str.find(':') {
                    (&mod_str[..colon_pos], &mod_str[colon_pos + 1..])
                } else {
                    return Err(format!("malformed modifier in memory operand: {}", index));
                };

                let (symbol, addend) = if let Some(plus_pos) = sym.find('+') {
                    let s = &sym[..plus_pos];
                    let off: i64 = sym[plus_pos + 1..].parse().unwrap_or(0);
                    (s.to_string(), off)
                } else {
                    (sym.to_string(), 0i64)
                };

                let opc = if is_128bit {
                    if is_load { 0b11 } else { 0b10 }
                } else if is_load {
                    0b01
                } else {
                    0b00
                };

                let reloc_type = match kind {
                    "lo12" => {
                        if is_128bit {
                            RelocType::Ldst128AbsLo12
                        } else {
                            match actual_size {
                                0b00 => RelocType::Ldst8AbsLo12,
                                0b01 => RelocType::Ldst16AbsLo12,
                                0b10 => RelocType::Ldst32AbsLo12,
                                0b11 => RelocType::Ldst64AbsLo12,
                                _ => RelocType::Ldst64AbsLo12,
                            }
                        }
                    }
                    "got_lo12" => RelocType::Ld64GotLo12,
                    _ => return Err(format!("unsupported modifier in load/store: {}", kind)),
                };

                let word =
                    ((actual_size << 30) | (0b111 << 27) | (v << 26) | (0b01 << 24) | (opc << 22))
                        | (rn << 5)
                        | rt;
                return Ok(EncodeResult::WordWithReloc {
                    word,
                    reloc: Relocation {
                        reloc_type,
                        symbol,
                        addend,
                    },
                });
            }

            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            let rm = parse_reg_num(index).ok_or("invalid index reg")?;
            let opc = if is_128bit {
                if is_load { 0b11 } else { 0b10 }
            } else if is_load {
                0b01
            } else {
                0b00
            };
            // Register offset: size 111 V opc 1 Rm option S 10 Rn Rt
            // Determine option and S from extend/shift specifiers
            let is_w_index = index.starts_with('w') || index.starts_with('W');
            let shift_amount: u8 = match shift {
                Some(s) => *s,
                None => 0,
            };
            let (option, s_bit) = match extend.as_deref() {
                Some("lsl") => {
                    // LSL with shift: S=1 if shift amount > 0
                    let s_val = if shift_amount > 0 { 1u32 } else { 0u32 };
                    (0b011u32, s_val)
                }
                Some("sxtw") => {
                    let s_val = if shift_amount > 0 { 1u32 } else { 0u32 };
                    (0b110u32, s_val)
                }
                Some("sxtx") => {
                    let s_val = if shift_amount > 0 { 1u32 } else { 0u32 };
                    (0b111u32, s_val)
                }
                Some("uxtw") => {
                    let s_val = if shift_amount > 0 { 1u32 } else { 0u32 };
                    (0b010u32, s_val)
                }
                Some("uxtx") => {
                    let s_val = if shift_amount > 0 { 1u32 } else { 0u32 };
                    (0b011u32, s_val)
                }
                None => {
                    // Default: if W register index, use UXTW; if X register, use LSL
                    if is_w_index {
                        (0b010u32, 0u32) // UXTW, no shift
                    } else {
                        (0b011u32, 0u32) // LSL, no shift
                    }
                }
                _ => (0b011u32, 0u32), // default LSL
            };
            let word = (actual_size << 30)
                | (0b111 << 27)
                | (v << 26)
                | (opc << 22)
                | (1 << 21)
                | (rm << 16)
                | (option << 13)
                | (s_bit << 12)
                | (0b10 << 10)
                | (rn << 5)
                | rt;
            return Ok(EncodeResult::Word(word));
        }

        // LDR (literal): ldr Rt, label — PC-relative load
        Some(Operand::Symbol(sym)) if is_load => {
            // opc V 011 00 imm19 Rt
            // For GP registers: opc=00 → 32-bit (W), opc=01 → 64-bit (X), opc=11 → PRFM
            // For FP/SIMD:      opc=00 → 32-bit (S), opc=01 → 64-bit (D), opc=10 → 128-bit (Q)
            // Note: actual_size uses 10=32-bit, 11=64-bit but LDR literal uses 00=32-bit, 01=64-bit
            let opc = if is_128bit {
                0b10u32
            } else if fp {
                // FP: S=00, D=01 (same mapping as GP)
                if actual_size == 0b11 { 0b01 } else { 0b00 }
            } else {
                // GP: W=00, X=01
                if actual_size == 0b11 { 0b01 } else { 0b00 }
            };
            let word = (opc << 30) | (v << 26) | (0b011 << 27) | rt;
            return Ok(EncodeResult::WordWithReloc {
                word,
                reloc: Relocation {
                    reloc_type: RelocType::Ldr19,
                    symbol: sym.clone(),
                    addend: 0,
                },
            });
        }

        _ => {}
    }

    Err(format!("unsupported ldr/str operands: {:?}", operands))
}

/// Encode LDUR/STUR (unscaled immediate offset load/store)
/// Format: size 111 V 00 opc 0 imm9 00 Rn Rt
pub fn encode_ldur_stur(
    operands: &[Operand],
    is_load: bool,
    op2_bits: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("ldur/stur requires 2 operands".to_string());
    }
    // FP/SIMD Rt is legal here (LDUR D0 is real), but SP/WSP is not:
    // field 31 of Rt reads as XZR, so `ldur sp,[x0,#8]` used to silently
    // load into the zero register. GNU as rejects both spellings.
    if let Some(Operand::Reg(r)) = operands.first() {
        if r.eq_ignore_ascii_case("sp") || r.eq_ignore_ascii_case("wsp") {
            return Err(format!(
                "ldur/stur: sp/wsp is not valid as the data register \
                 (field 31 of Rt reads as the zero register)"
            ));
        }
    }
    let (rt, _) = get_reg(operands, 0)?;
    let reg_name = match &operands[0] {
        Operand::Reg(r) => r.to_lowercase(),
        _ => String::new(),
    };
    let fp = is_fp_reg(&reg_name);
    let v = if fp { 1u32 } else { 0u32 };

    let (size, opc) = if fp {
        if reg_name.starts_with('q') {
            (0b00u32, if is_load { 0b11u32 } else { 0b10 })
        } else if reg_name.starts_with('d') {
            (0b11, if is_load { 0b01 } else { 0b00 })
        } else if reg_name.starts_with('s') {
            (0b10, if is_load { 0b01 } else { 0b00 })
        } else if reg_name.starts_with('h') {
            (0b01, if is_load { 0b01 } else { 0b00 })
        } else if reg_name.starts_with('b') {
            (0b00, if is_load { 0b01 } else { 0b00 })
        } else {
            (0b11, if is_load { 0b01 } else { 0b00 })
        }
    } else {
        let is_64 = reg_name.starts_with('x');
        let sz = if is_64 { 0b11u32 } else { 0b10 };
        (sz, if is_load { 0b01u32 } else { 0b00 })
    };

    let (rn, imm9) = match &operands[1] {
        Operand::Mem { base, offset } => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            (rn, *offset)
        }
        _ => {
            return Err(format!(
                "ldur/stur: expected memory operand, got {:?}",
                operands[1]
            ));
        }
    };

    // The unscaled imm9 field is signed 9-bit; the old code masked with
    // `& 0x1FF`, so `ldur x0,[x1,#-257]` silently wrapped to #255.
    if !(-256..=255).contains(&imm9) {
        return Err(format!(
            "ldur/stur: offset {imm9} is outside the signed 9-bit immediate \
             of the unscaled form (allowed -256..=255)"
        ));
    }
    let imm9_enc = (imm9 as u32) & 0x1FF;
    let word = (size << 30)
        | (0b111 << 27)
        | (v << 26)
        | (opc << 22)
        | (imm9_enc << 12)
        | (op2_bits << 10)
        | (rn << 5)
        | rt;
    Ok(EncodeResult::Word(word))
}

/// Encode LDTR/STTR with explicit size (for ldtrh, ldtrb, etc.)
pub fn encode_ldtr_sized(
    operands: &[Operand],
    is_load: bool,
    size: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("ldtr/sttr requires 2 operands".to_string());
    }
    // Same Rt contract as ldur/stur: FP legal, SP rejected (field 31 = XZR).
    if let Some(Operand::Reg(r)) = operands.first() {
        if r.eq_ignore_ascii_case("sp") || r.eq_ignore_ascii_case("wsp") {
            return Err(format!(
                "ldtr/sttr: sp/wsp is not valid as the data register \
                 (field 31 of Rt reads as the zero register)"
            ));
        }
    }
    let (rt, _) = get_reg(operands, 0)?;
    let opc = if is_load { 0b01u32 } else { 0b00 };
    let (rn, imm9) = match &operands[1] {
        Operand::Mem { base, offset } => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            (rn, *offset)
        }
        _ => return Err("ldtr/sttr: expected memory operand".to_string()),
    };
    // Signed 9-bit imm9; masked before, so out-of-range offsets wrapped.
    if !(-256..=255).contains(&imm9) {
        return Err(format!(
            "ldtr/sttr: offset {imm9} is outside the signed 9-bit immediate \
             of the unprivileged form (allowed -256..=255)"
        ));
    }
    let imm9_enc = (imm9 as u32) & 0x1FF;
    let word = (size << 30)
        | (0b111 << 27)
        | (opc << 22)
        | (imm9_enc << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rt;
    Ok(EncodeResult::Word(word))
}

pub fn encode_ldrsw(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("ldrsw requires 2 operands".to_string());
    }

    // LDRSW's destination is X-only (the 32-bit load is sign-extended to
    // 64 bits; there is no W form), takes no SP and no FP/SIMD register.
    // `ldrsw w0,[x1]` and `ldrsw d0,[x1]` both used to silently assemble
    // as `ldrsw x0`.
    let rt = get_gpr_strict_x(operands, 0)?;

    match operands.get(1) {
        Some(Operand::Mem { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            // LDRSW: size=10 111 V=0 01 opc=10 -> unsigned offset
            // Actually: 10 111 0 01 10 imm12 Rn Rt
            let abs_offset = *offset as u64;
            if *offset >= 0 && abs_offset.is_multiple_of(4) {
                let imm12 = (abs_offset / 4) as u32;
                if imm12 < 4096 {
                    let word = ((0b10 << 30) | (0b111 << 27))
                        | (0b01 << 24)
                        | (0b10 << 22)
                        | (imm12 << 10)
                        | (rn << 5)
                        | rt;
                    return Ok(EncodeResult::Word(word));
                }
            }
            // Unscaled: LDURSW
            let imm9 = checked_imm9(*offset, "ldrsw")?;
            let word =
                (((0b10 << 30) | (0b111 << 27)) | (0b10 << 22) | ((imm9 as u32 & 0x1FF) << 12))
                    | (rn << 5)
                    | rt;
            return Ok(EncodeResult::Word(word));
        }

        Some(Operand::MemPostIndex { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            let imm9 = checked_imm9(*offset, "ldrsw")?;
            let word = ((0b10 << 30) | (0b111 << 27))
                | (0b10 << 22)
                | ((imm9 as u32 & 0x1FF) << 12)
                | (0b01 << 10)
                | (rn << 5)
                | rt;
            return Ok(EncodeResult::Word(word));
        }

        Some(Operand::MemPreIndex { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            let imm9 = checked_imm9(*offset, "ldrsw")?;
            let word = ((0b10 << 30) | (0b111 << 27))
                | (0b10 << 22)
                | ((imm9 as u32 & 0x1FF) << 12)
                | (0b11 << 10)
                | (rn << 5)
                | rt;
            return Ok(EncodeResult::Word(word));
        }

        Some(Operand::MemRegOffset {
            base,
            index,
            extend,
            shift,
        }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            let rm = parse_reg_num(index).ok_or("invalid index reg")?;
            let (option, s_bit) = match (extend.as_deref(), shift) {
                (Some("lsl"), Some(2)) => (0b011u32, 1u32),
                (Some("lsl"), Some(0)) | (Some("lsl"), None) => (0b011, 0),
                (None, None) | (None, Some(0)) => (0b011, 0),
                (Some("sxtw"), Some(2)) => (0b110, 1),
                (Some("sxtw"), Some(0)) | (Some("sxtw"), None) => (0b110, 0),
                (Some("uxtw"), Some(2)) => (0b010, 1),
                (Some("uxtw"), Some(0)) | (Some("uxtw"), None) => (0b010, 0),
                (Some("sxtx"), Some(2)) => (0b111, 1),
                (Some("sxtx"), Some(0)) | (Some("sxtx"), None) => (0b111, 0),
                _ => {
                    return Err(format!(
                        "unsupported ldrsw extend/shift: {:?}/{:?}",
                        extend, shift
                    ));
                }
            };
            // LDRSW reg: 10 111 0 00 10 1 Rm option S 10 Rn Rt
            let word = (0b10 << 30)
                | (0b111 << 27)
                | (0b10 << 22)
                | (1 << 21)
                | (rm << 16)
                | (option << 13)
                | (s_bit << 12)
                | (0b10 << 10)
                | (rn << 5)
                | rt;
            return Ok(EncodeResult::Word(word));
        }

        _ => {}
    }

    Err(format!("unsupported ldrsw operands: {:?}", operands))
}

pub fn encode_ldrs(operands: &[Operand], size: u32) -> Result<EncodeResult, String> {
    // LDRSB/LDRSH: sign-extending byte/halfword loads
    if operands.len() < 2 {
        return Err("ldrsb/ldrsh requires 2 operands".to_string());
    }

    // LDRSB/LDRSH/LDRB/LDRH take a W or X destination (zero/sign-extended
    // to the register width); FP/SIMD spellings and SP are not encodable
    // (`ldrsb q0,[x1]` used to assemble as `ldrsb w0`).
    let (rt, is_64) = get_gpr_strict(operands, 0)?;
    let opc = if is_64 { 0b10 } else { 0b11 }; // 64-bit target: opc=10, 32-bit: opc=11

    if let Some(Operand::Mem { base, offset }) = operands.get(1) {
        let rn = parse_reg_num(base).ok_or("invalid base reg")?;
        let shift = size;
        let abs_offset = *offset as u64;
        let align = 1u64 << shift;
        if *offset >= 0 && abs_offset.is_multiple_of(align) {
            let imm12 = (abs_offset / align) as u32;
            if imm12 < 4096 {
                let word = ((size << 30) | (0b111 << 27))
                    | (0b01 << 24)
                    | (opc << 22)
                    | (imm12 << 10)
                    | (rn << 5)
                    | rt;
                return Ok(EncodeResult::Word(word));
            }
        }
        // Unscaled
        let imm9 = checked_imm9(*offset, "ldrs")?;
        let word = (((size << 30) | (0b111 << 27)) | (opc << 22) | ((imm9 as u32 & 0x1FF) << 12))
            | (rn << 5)
            | rt;
        return Ok(EncodeResult::Word(word));
    }

    // Post-index: ldrsb/ldrsh Rt, [Xn], #imm
    if let Some(Operand::MemPostIndex { base, offset }) = operands.get(1) {
        let rn = parse_reg_num(base).ok_or("invalid base reg")?;
        let imm9 = checked_imm9(*offset, "ldrs")?;
        let word = (size << 30)
            | (0b111 << 27)
            | (opc << 22)
            | ((imm9 as u32 & 0x1FF) << 12)
            | (0b01 << 10)
            | (rn << 5)
            | rt;
        return Ok(EncodeResult::Word(word));
    }

    // Pre-index: ldrsb/ldrsh Rt, [Xn, #imm]!
    if let Some(Operand::MemPreIndex { base, offset }) = operands.get(1) {
        let rn = parse_reg_num(base).ok_or("invalid base reg")?;
        let imm9 = checked_imm9(*offset, "ldrs")?;
        let word = (size << 30)
            | (0b111 << 27)
            | (opc << 22)
            | ((imm9 as u32 & 0x1FF) << 12)
            | (0b11 << 10)
            | (rn << 5)
            | rt;
        return Ok(EncodeResult::Word(word));
    }

    // Register offset: ldrsb/ldrsh Rt, [Xn, Xm{, extend {#amount}}]
    if let Some(Operand::MemRegOffset {
        base,
        index,
        extend,
        shift,
    }) = operands.get(1)
    {
        let rn = parse_reg_num(base).ok_or("invalid base reg")?;
        let rm = parse_reg_num(index).ok_or("invalid index reg")?;
        let is_w_index = index.starts_with('w') || index.starts_with('W');
        let shift_amount: u8 = match shift {
            Some(s) => *s,
            None => 0,
        };
        let (option, s_bit) = match extend.as_deref() {
            Some("lsl") => (0b011u32, if shift_amount > 0 { 1u32 } else { 0 }),
            Some("sxtw") => (0b110u32, if shift_amount > 0 { 1u32 } else { 0 }),
            Some("sxtx") => (0b111u32, if shift_amount > 0 { 1u32 } else { 0 }),
            Some("uxtw") => (0b010u32, if shift_amount > 0 { 1u32 } else { 0 }),
            Some("uxtx") => (0b011u32, if shift_amount > 0 { 1u32 } else { 0 }),
            None => {
                if is_w_index {
                    (0b010u32, 0u32)
                } else {
                    (0b011u32, 0u32)
                }
            }
            _ => (0b011u32, 0u32),
        };
        let word = (size << 30)
            | (0b111 << 27)
            | (opc << 22)
            | (1 << 21)
            | (rm << 16)
            | (option << 13)
            | (s_bit << 12)
            | (0b10 << 10)
            | (rn << 5)
            | rt;
        return Ok(EncodeResult::Word(word));
    }

    Err(format!("unsupported ldrsb/ldrsh operands: {:?}", operands))
}

pub fn encode_ldp_stp(operands: &[Operand], is_load: bool) -> Result<EncodeResult, String> {
    if operands.len() != 3 {
        return Err(format!(
            "ldp/stp requires 3 operands, got {}",
            operands.len()
        ));
    }

    let name1 = reg_name_at(operands, 0)?;
    let name2 = reg_name_at(operands, 1)?;

    // Same classifier as `ldnp/stnp`, deliberately shared: the two encodings
    // differ only in the no-allocate bit, so they cannot disagree about what a
    // register class means. `ldp`/`stp` used to carry its own copy, which
    // mapped `b`/`h` registers onto the S pair (there is no such pair form) and
    // never checked that the two registers of a pair shared a class -- so
    // `stp b0, b1, [x0]` silently assembled as `stp s0, s1, [x0]`.
    let what = if is_load { "ldp" } else { "stp" };
    let (opc, v, shift) = pair_reg_fields(&name1, what)?;
    let (opc2, v2, _) = pair_reg_fields(&name2, what)?;
    if (opc, v) != (opc2, v2) {
        return Err(format!(
            "{what}: `{name1}` and `{name2}` are different register classes; a pair needs two registers of the same kind and width"
        ));
    }

    let rt1 = parse_reg_num(&name1).ok_or_else(|| format!("invalid register: {name1}"))?;
    let rt2 = parse_reg_num(&name2).ok_or_else(|| format!("invalid register: {name2}"))?;
    let l = if is_load { 1u32 } else { 0u32 };

    match operands.get(2) {
        // STP rt1, rt2, [base, #offset]! (pre-index)
        Some(Operand::MemPreIndex { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            let imm7 = checked_imm7(*offset, shift, if is_load { "ldp" } else { "stp" })?;
            let word = (opc << 30)
                | (0b101 << 27)
                | (v << 26)
                | (0b011 << 23)
                | (l << 22)
                | ((imm7 as u32 & 0x7F) << 15)
                | (rt2 << 10)
                | (rn << 5)
                | rt1;
            return Ok(EncodeResult::Word(word));
        }

        // LDP/STP rt1, rt2, [base], #offset (post-index)
        Some(Operand::MemPostIndex { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            let imm7 = checked_imm7(*offset, shift, if is_load { "ldp" } else { "stp" })?;
            let word = (opc << 30)
                | (0b101 << 27)
                | (v << 26)
                | (0b001 << 23)
                | (l << 22)
                | ((imm7 as u32 & 0x7F) << 15)
                | (rt2 << 10)
                | (rn << 5)
                | rt1;
            return Ok(EncodeResult::Word(word));
        }

        // LDP/STP rt1, rt2, [base, #offset] (signed offset)
        Some(Operand::Mem { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            let imm7 = checked_imm7(*offset, shift, if is_load { "ldp" } else { "stp" })?;
            let word = (opc << 30)
                | (0b101 << 27)
                | (v << 26)
                | (0b010 << 23)
                | (l << 22)
                | ((imm7 as u32 & 0x7F) << 15)
                | (rt2 << 10)
                | (rn << 5)
                | rt1;
            return Ok(EncodeResult::Word(word));
        }

        _ => {}
    }

    Err(format!("unsupported ldp/stp operands: {:?}", operands))
}

/// Encode LDNP/STNP (load/store pair non-temporal)
/// Encoding: opc 101 V 000 L imm7 Rt2 Rn Rt
/// TODO: Only handles integer registers (V=0). FP/SIMD register support needed for V=1.
/// Classify a register for the load/store-PAIR encodings.
///
/// Returns `(opc, V, scale)`, where `opc` is the 2-bit class field at 31:30,
/// `V` at bit 26 selects the SIMD/FP encoding space, and `scale` is log2 of the
/// access width -- the shift applied to the byte offset before it is stored in
/// the imm7 field.
///
/// ```text
/// GPR W -> (00, 0, 2) FP S -> (00, 1, 2)
/// GPR X -> (10, 0, 3) FP D -> (01, 1, 3)
/// FP Q -> (10, 1, 4)
/// ```
///
/// `ldnp`/`stnp` previously hardcoded `V = 0` and chose `opc` from "is this a
/// 64-bit register", which is a GPR-only question. Every FP non-temporal pair
/// was therefore encoded as the corresponding GPR pair: `ldnp s24, s13,
/// [x20, #80]` produced exactly the word GAS produces for `ldnp w24, w13,
/// [x20, #80]` -- a pair of 32-bit integer loads in place of two single-precision
/// float loads. `ldp`/`stp` already classified correctly, which is why the
/// defect survived: the common pair instructions were right and the rare
/// non-temporal ones were silently wrong.
fn pair_reg_fields(name: &str, what: &str) -> Result<(u32, u32, u32), String> {
    let n = name.to_lowercase();
    // SP is not a data register: field 31 of Rt/Rt2 reads as the zero register,
    // so `ldp sp,x1,[x2]` used to assemble as `ldp xzr,x1,[x2]` -- and GNU as
    // rejects it.  XZR/WZR are real pair operands and stay.
    if n == "sp" || n == "wsp" {
        return Err(format!(
            "{what}: `{name}` is the stack pointer, but this field reads encoding \
             31 as the zero register; write `{}` if the zero register is what you \
             meant",
            if n == "sp" { "xzr" } else { "wzr" }
        ));
    }
    let fields = match n.as_str() {
        "xzr" | "lr" => (0b10u32, 0u32, 3u32),
        "wzr" => (0b00, 0, 2),
        _ => match n.chars().next() {
            Some('x') => (0b10, 0, 3),
            Some('w') => (0b00, 0, 2),
            Some('s') => (0b00, 1, 2),
            Some('d') => (0b01, 1, 3),
            Some('q') | Some('v') => (0b10, 1, 4),
            _ => {
                return Err(format!(
                    "{what}: unsupported register `{name}` (a load/store pair expects w, x, s, d or q)"
                ));
            }
        },
    };
    Ok(fields)
}

/// Fetch the register *name* at `idx`; the pair encoders need the class, which
/// the numeric `get_reg` does not carry.
fn reg_name_at(operands: &[Operand], idx: usize) -> Result<String, String> {
    match operands.get(idx) {
        Some(Operand::Reg(r)) => Ok(r.clone()),
        other => Err(format!("expected register at operand {idx}, got {other:?}")),
    }
}

pub fn encode_ldnp_stnp(operands: &[Operand], is_load: bool) -> Result<EncodeResult, String> {
    if operands.len() != 3 {
        return Err(format!(
            "ldnp/stnp requires 3 operands, got {}",
            operands.len()
        ));
    }

    let name1 = reg_name_at(operands, 0)?;
    let name2 = reg_name_at(operands, 1)?;

    // One opc/V field covers the whole pair, so both registers must be the
    // same class and width. A mismatched pair has no encoding at all; it must
    // be diagnosed rather than silently downgraded to the GPR form.
    let what = if is_load { "ldnp" } else { "stnp" };
    let (opc, v, shift) = pair_reg_fields(&name1, what)?;
    let (opc2, v2, _) = pair_reg_fields(&name2, what)?;
    if (opc, v) != (opc2, v2) {
        return Err(format!(
            "{what}: `{name1}` and `{name2}` are different register classes; a pair needs two registers of the same kind and width"
        ));
    }

    let rt1 = parse_reg_num(&name1).ok_or_else(|| format!("invalid register: {name1}"))?;
    let rt2 = parse_reg_num(&name2).ok_or_else(|| format!("invalid register: {name2}"))?;
    let l: u32 = if is_load { 1 } else { 0 };

    match operands.get(2) {
        Some(Operand::Mem { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            let align = 1i64 << shift;
            // The imm7 field is scaled, so an unaligned offset would be
            // truncated and the pair would access the wrong address.
            if *offset % align != 0 {
                return Err(format!(
                    "ldnp/stnp: offset {offset} is not a multiple of {align} (required by the {name1} access width)"
                ));
            }
            let imm7 = checked_imm7(*offset, shift, if is_load { "ldnp" } else { "stnp" })?;
            // LDNP/STNP: opc(31:30) 101(29:27) V(26) 000(25:23) L(22)
            // imm7(21:15) Rt2(14:10) Rn(9:5) Rt(4:0)
            let word = (opc << 30)
                | (0b101 << 27)
                | (v << 26)
                | (l << 22)
                | ((imm7 as u32 & 0x7F) << 15)
                | (rt2 << 10)
                | (rn << 5)
                | rt1;
            Ok(EncodeResult::Word(word))
        }
        _ => Err(format!("unsupported ldnp/stnp operands: {operands:?}")),
    }
}

// ── Exclusive loads/stores ───────────────────────────────────────────────

/// Encode LDXR/STXR and byte/halfword variants.
/// `forced_size`: None = auto-detect from register width, Some(0b00) = byte, Some(0b01) = halfword
/// Shared checks for the exclusive/acquire families: `[Xn]` takes no
/// offset (there is no offset field), and a forced size must be a real
/// 2-bit encoding (the old code shifted a size of 4 into the top byte).
fn exclusive_mem_base(operands: &[Operand], idx: usize, mn: &str) -> Result<u32, String> {
    match operands.get(idx) {
        Some(Operand::Mem { base, offset }) => {
            if *offset != 0 {
                return Err(format!(
                    "{mn}: [{},#{}] -- the exclusive encoding has no \
                     offset field; only [Xn] is encodable",
                    base, offset
                ));
            }
            parse_reg_num(base).ok_or_else(|| format!("{mn}: invalid base register"))
        }
        Some(other) => Err(format!("{mn}: expected [Xn], got {other:?}")),
        None => Err(format!("{mn}: missing memory operand")),
    }
}

fn checked_forced_size(forced_size: Option<u32>, mn: &str) -> Result<Option<u32>, String> {
    if let Some(s) = forced_size {
        if s > 0b11 {
            return Err(format!(
                "{mn}: forced size {s} is outside the 2-bit size field (0..=3)"
            ));
        }
    }
    Ok(forced_size)
}

pub fn encode_ldxr_stxr(
    operands: &[Operand],
    is_load: bool,
    forced_size: Option<u32>,
) -> Result<EncodeResult, String> {
    let forced_size = checked_forced_size(forced_size, "ldxr/stxr")?;
    if is_load {
        // The exclusives have no FP/SIMD form and no SP form (field 31 of Rt
        // is XZR there): `ldxr d0,[x1]` used to assemble as `ldxr w0`.
        let (rt, is_64) = get_gpr_strict(operands, 0)?;
        let rn = exclusive_mem_base(operands, 1, "ldxr")?;
        let size = forced_size.unwrap_or(if is_64 { 0b11 } else { 0b10 });
        let word = ((size << 30) | (0b001000010 << 21) | (0b11111 << 16))
            | (0b11111 << 10)
            | (rn << 5)
            | rt;
        Ok(EncodeResult::Word(word))
    } else {
        // The status register is W-only: `stxr x0,x1,[x2]` must be diagnosed
        // rather than silently encoded with Rs=x0.
        let ws = get_gpr_strict_w(operands, 0)?;
        let (rt, is_64) = get_gpr_strict(operands, 1)?;
        let rn = exclusive_mem_base(operands, 2, "stxr")?;
        let size = forced_size.unwrap_or(if is_64 { 0b11 } else { 0b10 });
        let word =
            ((size << 30) | (0b001000000 << 21) | (ws << 16)) | (0b11111 << 10) | (rn << 5) | rt;
        Ok(EncodeResult::Word(word))
    }
}

/// Encode LDAXR/STLXR and byte/halfword variants.
pub fn encode_ldaxr_stlxr(
    operands: &[Operand],
    is_load: bool,
    forced_size: Option<u32>,
) -> Result<EncodeResult, String> {
    let forced_size = checked_forced_size(forced_size, "ldaxr/stlxr")?;
    if is_load {
        // Same GP-only, no-SP contract as the plain exclusives.
        let (rt, is_64) = get_gpr_strict(operands, 0)?;
        let rn = exclusive_mem_base(operands, 1, "ldaxr")?;
        let size = forced_size.unwrap_or(if is_64 { 0b11 } else { 0b10 });
        let word = (size << 30)
            | (0b001000010 << 21)
            | (0b11111 << 16)
            | (1 << 15)
            | (0b11111 << 10)
            | (rn << 5)
            | rt;
        Ok(EncodeResult::Word(word))
    } else {
        let ws = get_gpr_strict_w(operands, 0)?;
        let (rt, is_64) = get_gpr_strict(operands, 1)?;
        let rn = exclusive_mem_base(operands, 2, "stlxr")?;
        let size = forced_size.unwrap_or(if is_64 { 0b11 } else { 0b10 });
        let word = (size << 30)
            | (0b001000000 << 21)
            | (ws << 16)
            | (1 << 15)
            | (0b11111 << 10)
            | (rn << 5)
            | rt;
        Ok(EncodeResult::Word(word))
    }
}

/// Encode LDXP/STXP/LDAXP/STLXP (exclusive pair) instructions.
///
/// LDXP Xt1, Xt2, [Xn] : sz 001000 0 1 1 11111 0 Rt2 Rn Rt
/// LDAXP Xt1, Xt2, [Xn] : sz 001000 0 1 1 11111 1 Rt2 Rn Rt
/// STXP Ws, Xt1, Xt2, [Xn] : sz 001000 0 0 1 Rs 0 Rt2 Rn Rt
/// STLXP Ws, Xt1, Xt2, [Xn] : sz 001000 0 0 1 Rs 1 Rt2 Rn Rt
pub fn encode_ldxp_stxp(
    operands: &[Operand],
    is_load: bool,
    acquire_release: bool,
) -> Result<EncodeResult, String> {
    let o0 = if acquire_release { 1u32 } else { 0 };
    if is_load {
        // LDXP/LDAXP Rt, Rt2, [Rn]: GP-only pair, no SP (field 31 of Rt/Rt2
        // reads as XZR), and both members the same width.
        let (rt, is_64) = get_gpr_strict(operands, 0)?;
        let (rt2, rt2_is_64) = get_gpr_strict(operands, 1)?;
        if is_64 != rt2_is_64 {
            return Err(format!(
                "ldxp/ldaxp: the two data registers must be the same width"
            ));
        }
        let rn = exclusive_mem_base(operands, 2, "ldxp")?;
        let sz = if is_64 { 1u32 } else { 0 };
        // 1 sz 001000 0 1 1 11111 o0 Rt2 Rn Rt (bit23=0)
        let word = (1u32 << 31)
            | (sz << 30)
            | (0b001000 << 24)
            | (1 << 22)
            | (1 << 21)
            | (0b11111 << 16)
            | (o0 << 15)
            | (rt2 << 10)
            | (rn << 5)
            | rt;
        Ok(EncodeResult::Word(word))
    } else {
        // STXP/STLXP Ws, Rt, Rt2, [Rn]: Ws is W-only, the pair GP-only
        // and same-width.
        let ws = get_gpr_strict_w(operands, 0)?; // status register (always W)
        let (rt, is_64) = get_gpr_strict(operands, 1)?;
        let (rt2, rt2_is_64) = get_gpr_strict(operands, 2)?;
        if is_64 != rt2_is_64 {
            return Err(format!(
                "stxp/stlxp: the two data registers must be the same width"
            ));
        }
        let rn = exclusive_mem_base(operands, 3, "stxp")?;
        let sz = if is_64 { 1u32 } else { 0 };
        // 1 sz 001000 0 0 1 Rs o0 Rt2 Rn Rt (bit23=0, bit22=0)
        let word = (1u32 << 31)
            | (sz << 30)
            | (0b001000 << 24)
            | (1 << 21)
            | (ws << 16)
            | (o0 << 15)
            | (rt2 << 10)
            | (rn << 5)
            | rt;
        Ok(EncodeResult::Word(word))
    }
}

/// Encode LDAR/STLR and byte/halfword variants.
pub fn encode_ldar_stlr(
    operands: &[Operand],
    is_load: bool,
    forced_size: Option<u32>,
) -> Result<EncodeResult, String> {
    let forced_size = checked_forced_size(forced_size, "ldar/stlr")?;
    // LDAR/STLR have no FP/SIMD form and no SP form (field 31 of Rt reads
    // as XZR): `ldar d0,[x1]` used to assemble as `ldar w0`.
    let (rt, is_64) = get_gpr_strict(operands, 0)?;
    let rn = exclusive_mem_base(operands, 1, "ldar/stlr")?;
    let size = forced_size.unwrap_or(if is_64 { 0b11 } else { 0b10 });
    let l = if is_load { 1u32 } else { 0 };
    // LDAR/STLR: size 001000 1 L 0 11111 1 11111 Rn Rt
    let word = ((size << 30) | (0b001000 << 24) | (1 << 23) | (l << 22))
        | (0b11111 << 16)
        | (1 << 15)
        | (0b11111 << 10)
        | (rn << 5)
        | rt;
    Ok(EncodeResult::Word(word))
}

// ── Address computation ──────────────────────────────────────────────────

pub fn encode_adrp(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;

    let (sym, addend) = match operands.get(1) {
        Some(Operand::Symbol(s)) => (s.clone(), 0i64),
        Some(Operand::Modifier { kind, symbol }) if kind == "got" => {
            // adrp x0, :got:symbol
            let word = (1u32 << 31) | (0b10000 << 24) | rd;
            return Ok(EncodeResult::WordWithReloc {
                word,
                reloc: Relocation {
                    reloc_type: RelocType::AdrGotPage21,
                    symbol: symbol.clone(),
                    addend: 0,
                },
            });
        }
        Some(Operand::SymbolOffset(s, off)) => (s.clone(), *off),
        Some(Operand::Label(s)) => (s.clone(), 0i64),
        // Parser misclassifies symbol names that collide with register names (s1, v0, d1, etc.),
        // condition codes (cc, lt, le), or barrier names (st, ld).
        // ADRP never takes these as actual operand types, so treat them as symbols.
        Some(Operand::Reg(name)) => (name.clone(), 0i64),
        Some(Operand::Cond(name)) => (name.clone(), 0i64),
        Some(Operand::Barrier(name)) => (name.clone(), 0i64),
        _ => {
            return Err(format!(
                "adrp needs symbol operand, got {:?}",
                operands.get(1)
            ));
        }
    };

    // ADRP: 1 immlo[1:0] 10000 immhi[18:0] Rd
    let word = (1u32 << 31) | (0b10000 << 24) | rd;
    Ok(EncodeResult::WordWithReloc {
        word,
        reloc: Relocation {
            reloc_type: RelocType::AdrpPage21,
            symbol: sym,
            addend,
        },
    })
}

pub fn encode_adr(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, _) = get_reg(operands, 0)?;

    // Check for immediate offset form: adr Rd, #imm
    // TODO: validate 21-bit signed immediate range
    if let Some(Operand::Imm(imm)) = operands.get(1) {
        let imm = *imm;
        // ADR: 0 immlo[1:0] 10000 immhi[18:0] Rd
        let immlo = ((imm as u32) & 3) << 29;
        let immhi = (((imm as u32) >> 2) & 0x7FFFF) << 5;
        let word = immlo | (0b10000 << 24) | immhi | rd;
        return Ok(EncodeResult::Word(word));
    }

    let (sym, addend) = get_symbol(operands, 1)?;
    // ADR: 0 immlo[1:0] 10000 immhi[18:0] Rd
    let word = (0b10000 << 24) | rd;
    Ok(EncodeResult::WordWithReloc {
        word,
        reloc: Relocation {
            reloc_type: RelocType::AdrPrelLo21,
            symbol: sym,
            addend,
        },
    })
}

// ── Prefetch ─────────────────────────────────────────────────────────────

/// Encode the PRFM (prefetch memory) instruction.
/// Format: PRFM <prfop>, [<Xn|SP>{, #<pimm>}]
/// Encoding: 1111 1001 10 imm12 Rn Rt
/// where Rt is the 5-bit prefetch operation type.
pub fn encode_prfm(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("prfm requires 2 operands".to_string());
    }

    // First operand: prefetch operation type (parsed as Symbol)
    let prfop = match &operands[0] {
        Operand::Symbol(s) => encode_prfop(s)?,
        Operand::Imm(v) => {
            if *v < 0 || *v > 31 {
                return Err(format!("prfm: immediate prefetch type out of range: {}", v));
            }
            *v as u32
        }
        _ => {
            return Err(format!(
                "prfm: expected prefetch operation name, got {:?}",
                operands[0]
            ));
        }
    };

    // Second operand: memory address [Xn{, #imm}]
    match &operands[1] {
        Operand::Mem { base, offset } => {
            let rn = parse_reg_num(base)
                .ok_or_else(|| format!("prfm: invalid base register: {}", base))?;
            let imm = *offset;
            if imm < 0 || imm % 8 != 0 {
                return Err(format!(
                    "prfm: offset must be non-negative and 8-byte aligned, got {}",
                    imm
                ));
            }
            // Range-check the scaled value before the u32 cast: the old
            // `(imm / 8) as u32` wrapped for offsets above 32 GiB, so
            // 34359738368 silently encoded as imm12=0.
            if imm / 8 > 0xFFF {
                return Err(format!("prfm: offset too large: {}", imm));
            }
            let imm12 = (imm / 8) as u32;
            // PRFM (imm): 1111 1001 10 imm12(12) Rn(5) Rt(5)
            let word = 0xF9800000 | (imm12 << 10) | (rn << 5) | prfop;
            Ok(EncodeResult::Word(word))
        }
        Operand::Symbol(_sym) => {
            // PRFM (literal) with symbol reference is not yet supported
            Err("prfm with symbol/label operand not yet supported".to_string())
        }
        Operand::MemRegOffset {
            base,
            index,
            extend,
            shift,
        } => {
            // PRFM (register): 11 111 0 00 10 1 Rm option S 10 Rn Rt
            let rn = parse_reg_num(base)
                .ok_or_else(|| format!("prfm: invalid base register: {}", base))?;
            let rm = parse_reg_num(index)
                .ok_or_else(|| format!("prfm: invalid index register: {}", index))?;
            let is_w_index = index.starts_with('w') || index.starts_with('W');
            let shift_amount: u8 = match shift {
                Some(s) => *s,
                None => 0,
            };
            let (option, s_bit) = match extend.as_deref() {
                Some("lsl") => (0b011u32, if shift_amount > 0 { 1u32 } else { 0 }),
                Some("sxtw") => (0b110u32, if shift_amount > 0 { 1u32 } else { 0 }),
                Some("sxtx") => (0b111u32, if shift_amount > 0 { 1u32 } else { 0 }),
                Some("uxtw") => (0b010u32, if shift_amount > 0 { 1u32 } else { 0 }),
                None => {
                    if is_w_index {
                        (0b010u32, 0u32)
                    } else {
                        (0b011u32, 0u32)
                    }
                }
                _ => (0b011u32, 0u32),
            };
            let word = (0b11 << 30)
                | (0b111 << 27)
                | (0b10 << 22) // opc field occupies bits 23-22 (llvm-mc: prfm x?, [x0,x0] = 0xF8A06800)
                | (1 << 21)
                | (rm << 16)
                | (option << 13)
                | (s_bit << 12)
                | (0b10 << 10)
                | (rn << 5)
                | prfop;
            Ok(EncodeResult::Word(word))
        }
        _ => Err(format!(
            "prfm: expected memory operand, got {:?}",
            operands[1]
        )),
    }
}

/// Encode PRFUM (prefetch with an unscaled 9-bit offset).
///
/// `PRFUM <prfop>, [<Xn|SP>{, #imm}]` is PRFM's signed-offset sibling:
/// ```text
/// 11 111 0 00 10 1 0 0 0 imm9 00 Rn Rt   (base 0xF8800000)
/// ```
/// The family was missing entirely, so `prfum pldl1keep,[x0,#8]` was a hard
/// error.  The offset is signed nine-bit; the published PRFM reader rejects
/// negative offsets outright, which is correct for PRFM (where the unsigned
/// imm12 field cannot express them) and wrong here.
pub fn encode_prfum(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 2 {
        return Err(format!(
            "prfum requires exactly 2 operands (prfop, [Xn{{, #imm}}]), got {}",
            operands.len()
        ));
    }
    let prfop = match &operands[0] {
        Operand::Symbol(s) => encode_prfop(s)?,
        Operand::Imm(v) => {
            if !(0..=31).contains(v) {
                return Err(format!("prfum: immediate prefetch type out of range: {v}"));
            }
            *v as u32
        }
        other => {
            return Err(format!(
                "prfum: expected a prefetch operation name, got {other:?}"
            ));
        }
    };
    let (base, offset) = match &operands[1] {
        Operand::Mem { base, offset } => (base, *offset),
        other => {
            return Err(format!(
                "prfum: expected a memory operand ([Xn] or [Xn, #imm]), got {other:?}"
            ));
        }
    };
    if !(-256..=255).contains(&offset) {
        return Err(format!(
            "prfum: offset {offset} does not fit the signed 9-bit field \
             (allowed -256..=255)"
        ));
    }
    let rn = atomic_base_name(base, "prfum")?;
    let imm9 = (offset as u32) & 0x1FF;
    // PRFUM (imm): 1111 1000 10 00 imm9 00 Rn Rt
    let word = 0xF8800000 | (imm9 << 12) | (rn << 5) | prfop;
    Ok(EncodeResult::Word(word))
}

/// Map prefetch operation name to its 5-bit encoding.
pub fn encode_prfop(name: &str) -> Result<u32, String> {
    match name.to_lowercase().as_str() {
        "pldl1keep" => Ok(0b00000),
        "pldl1strm" => Ok(0b00001),
        "pldl2keep" => Ok(0b00010),
        "pldl2strm" => Ok(0b00011),
        "pldl3keep" => Ok(0b00100),
        "pldl3strm" => Ok(0b00101),
        "plil1keep" => Ok(0b01000),
        "plil1strm" => Ok(0b01001),
        "plil2keep" => Ok(0b01010),
        "plil2strm" => Ok(0b01011),
        "plil3keep" => Ok(0b01100),
        "plil3strm" => Ok(0b01101),
        "pstl1keep" => Ok(0b10000),
        "pstl1strm" => Ok(0b10001),
        "pstl2keep" => Ok(0b10010),
        "pstl2strm" => Ok(0b10011),
        "pstl3keep" => Ok(0b10100),
        "pstl3strm" => Ok(0b10101),
        _ => Err(format!("prfm: unknown prefetch operation: {}", name)),
    }
}

// ── LSE Atomics ──────────────────────────────────────────────────────────

/// Parse the order/size suffix shared by the CAS and CASP families
/// (everything after the `cas` / `casp` stem).
///
/// Valid suffixes:
/// "" relaxed "b" byte (CAS family only)
/// "a" acquire "h" half (CAS family only)
/// "l" release
/// "al" acquire+release
/// Byte/half letters always trail the order letters (`casalb`, `caslb`, ...).
///
/// The parse is exact rather than a `contains` probe: a lenient contains
/// check silently encodes a mistyped mnemonic such as `casq` as relaxed
/// CAS, i.e. the assembler would accept and mis-assemble garbage.
/// Returns `(acquire, release, size_letter)`.

/// Reads the `[Xn]` operand of an LSE atomic (CAS/SWP/LDADD/...): the
/// encoding has no offset field and no writeback, so GNU as rejects
/// `cas x0,x1,[x2,#8]` outright. The old code silently dropped the offset.
/// Resolve a base-register name for a memory operand: `x0`-`x30` or `sp`.
/// 32-bit spellings are not encodable in a 64-bit addressing mode.
fn atomic_base_name(name: &str, mn: &str) -> Result<u32, String> {
    let lower = name.to_lowercase();
    if lower == "sp" {
        return Ok(31);
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

fn atomic_base(operands: &[Operand], idx: usize, mn: &str) -> Result<u32, String> {
    match operands.get(idx) {
        Some(Operand::Mem { base, offset }) => {
            if *offset != 0 {
                return Err(format!(
                    "{mn}: [{},#{}] -- the atomic encoding has no \
                     offset field; only [Xn] is encodable",
                    base, offset
                ));
            }
            parse_reg_num(base).ok_or_else(|| format!("{mn}: invalid base register"))
        }
        Some(other) => Err(format!(
            "{mn}: expected a memory operand [Xn], got {other:?}"
        )),
        None => Err(format!("{mn}: missing memory operand")),
    }
}

/// The byte/halfword-suffixed atomics (casb/cash/swpb/ldaddh/...) read and
/// write 8/16 bits of a W register, so an X spelling is an operand mismatch
/// GNU as rejects.
fn atomic_bh_requires_w(is_64: bool, size_letter: Option<char>, mn: &str) -> Result<(), String> {
    if size_letter.is_some() && is_64 {
        return Err(format!(
            "{mn}: the byte/halfword forms take w registers, not x"
        ));
    }
    Ok(())
}

fn parse_atomic_order_suffix(
    stem: &str,
    suffix: &str,
) -> Result<(bool, bool, Option<char>), String> {
    let (order, size) = if let Some(rest) = suffix.strip_suffix('b') {
        (rest, Some('b'))
    } else if let Some(rest) = suffix.strip_suffix('h') {
        (rest, Some('h'))
    } else {
        (suffix, None)
    };
    let (a, l) = match order {
        "" => (false, false),
        "a" => (true, false),
        "l" => (false, true),
        "al" => (true, true),
        _ => return Err(format!("{}: invalid order/size suffix '{}'", stem, suffix)),
    };
    Ok((a, l, size))
}

/// Encode CAS/CASA/CASAL/CASL and byte/halfword variants (Compare and Swap).
/// CAS SZ |001000|1|A|1|Rs|R|11111|Rn|Rt (LLVM AArch64InstrFormats.td;
/// round-trip verified against Capstone for all twelve order/size forms)
pub fn encode_cas(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err(format!("{} requires 3 operands", mnemonic));
    }
    // CAS is GP-only (no FP/SIMD form, no SP: field 31 reads as XZR), and
    // Rs and Rt must be the same width.
    let (rs, is_64) = get_gpr_strict(operands, 0)?;
    let (rt, rt_is_64) = get_gpr_strict(operands, 1)?;
    if is_64 != rt_is_64 {
        return Err(format!(
            "{mnemonic}: the source and destination registers must be the same width"
        ));
    }
    let mn = mnemonic.to_lowercase();
    let rn = atomic_base(operands, 2, &mn)?;
    let suffix = mn.strip_prefix("cas").unwrap_or("");
    let (a, l, size_letter) = parse_atomic_order_suffix("cas", suffix)?;
    atomic_bh_requires_w(is_64, size_letter, &mn)?;
    // Determine size: 'b' suffix = byte (00), 'h' suffix = half (01), else register-based
    let size = match size_letter {
        Some('b') => 0b00u32,
        Some('h') => 0b01u32,
        _ if is_64 => 0b11u32,
        _ => 0b10u32,
    };
    // size 001000 1 A 1 Rs R 11111 Rn Rt
    let word = (size << 30)
        | (0b001000 << 24)
        | (1 << 23)
        | ((a as u32) << 22)
        | (1 << 21)
        | (rs << 16)
        | ((l as u32) << 15)
        | (0b11111 << 10)
        | (rn << 5)
        | rt;
    Ok(EncodeResult::Word(word))
}

/// Encode CASP/CASPA/CASPAL/CASPL (Compare and Swap **Pair**, LSE).
///
/// CASP 0|SZ|001000|0|A|1|Rs|R|11111|Rn|Rt
///
/// Layout per LLVM AArch64InstrFormats.td (Capstone round-trip verified):
/// - bit 31 is always 0 and SZ (bit 30) selects X pairs (1) vs W pairs (0);
/// this is why CASP does NOT live at the same size-field position as CAS.
/// - bit 23 (NP) is 0 here and 1 for CAS — the architectural CAS/CASP split.
/// - A (bit 22) / R (bit 15) are the acquire/release order bits.
///
/// Operand order: `casp<order> Xs, Xs+1, Xt, Xt+1, [Xn]` — the first pair
/// holds the expected (compare) value and receives the loaded old value
/// (both members are in-out), the second pair holds the new value.
/// Only Xs and Xt occupy encoding fields; Xs+1 / Xt+1 are architecturally
/// implied, so the text operands are validated to match exactly (GAS
/// parity: even-numbered start register, consecutive pairs, uniform width).
pub fn encode_casp(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() != 5 {
        return Err(format!(
            "{} requires exactly 5 operands (Xs, Xs+1, Xt, Xt+1, [Xn])",
            mnemonic
        ));
    }
    let (rs, rs_64) = get_reg(operands, 0)?;
    let (rs1, rs1_64) = get_reg(operands, 1)?;
    let (rt, rt_64) = get_reg(operands, 2)?;
    let (rt1, rt1_64) = get_reg(operands, 3)?;

    // SP/xzr (register 31) can never be a pair member: the pair registers
    // are architectural GPRs, and 31 would alias the stack pointer / zero
    // register depending on context.
    if rs == 31 || rs1 == 31 || rt == 31 || rt1 == 31 {
        return Err(format!(
            "{}: SP/xzr/wsp/wzr cannot be a CASP pair register",
            mnemonic
        ));
    }
    // Uniform width across the whole instruction (both pairs).
    if rs_64 != rs1_64 || rt_64 != rt1_64 || rs_64 != rt_64 {
        return Err(format!(
            "{}: CASP pair registers must all be X or all be W",
            mnemonic
        ));
    }
    // Even start register + exact consecutive pairing. Both are architectural
    // requirements (Xs+1/Xt+1 have no encoding field), not just GAS policy:
    // accepting `caspal x0, x2, x4, x6, [x11]` would silently change which
    // registers participate in the atomic operation.
    if rs % 2 != 0 || rt % 2 != 0 {
        return Err(format!(
            "{}: CASP pair start registers must be even (got {}, {})",
            mnemonic, rs, rt
        ));
    }
    if rs1 != rs + 1 || rt1 != rt + 1 {
        return Err(format!(
            "{}: CASP pair registers must be consecutive (got {}, {} and {}, {})",
            mnemonic, rs, rs1, rt, rt1
        ));
    }

    // Base register: plain GPR64sp — no offset/writeback forms exist for
    // CASP, and XZR-as-base is not encodable (31 here means SP).
    let rn = match operands.get(4) {
        Some(Operand::Mem { base, offset: 0 }) => {
            let b = base.to_lowercase();
            if b == "xzr" || b == "wzr" {
                return Err(format!(
                    "{}: xzr/wzr is not a valid CASP base register",
                    mnemonic
                ));
            }
            if !is_64bit_reg(base) {
                return Err(format!(
                    "{}: CASP base register must be an X register or SP",
                    mnemonic
                ));
            }
            parse_reg_num(base).ok_or_else(|| format!("{}: invalid base register", mnemonic))?
        }
        Some(Operand::Mem { .. }) => {
            return Err(format!(
                "{}: memory operand must be a plain base register [Xn] (no offset/writeback)",
                mnemonic
            ));
        }
        other => {
            return Err(format!(
                "{}: expected memory operand [Xn], got {:?}",
                mnemonic, other
            ));
        }
    };

    let mn = mnemonic.to_lowercase();
    let suffix = mn.strip_prefix("casp").unwrap_or("");
    let (a, l, size_letter) = parse_atomic_order_suffix("casp", suffix)?;
    if size_letter.is_some() {
        return Err(format!("{}: CASP has no byte/halfword variants", mnemonic));
    }

    let size_bit = if rs_64 { 1u32 } else { 0 };
    // 0|SZ 001000 0 A 1 Rs R 11111 Rn Rt
    let word = (size_bit << 30)
        | (0b001000 << 24)
        | ((a as u32) << 22)
        | (1 << 21)
        | (rs << 16)
        | ((l as u32) << 15)
        | (0b11111 << 10)
        | (rn << 5)
        | rt;
    Ok(EncodeResult::Word(word))
}

/// Encode SWP/SWPA/SWPAL/SWPL and byte/halfword variants (Swap).
/// SWP Xs, Xt, [Xn]: size 111000 AR 1 Rs 1 000 00 Rn Rt
/// Variants: swp, swpa, swpal, swpl, swpb, swpab, swpalb, swplb, swph, swpah, swpalh, swplh
pub fn encode_swp(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err(format!("{} requires 3 operands", mnemonic));
    }
    // SWP is GP-only and same-width like CAS.
    let (rs, is_64) = get_gpr_strict(operands, 0)?;
    let (rt, rt_is_64) = get_gpr_strict(operands, 1)?;
    if is_64 != rt_is_64 {
        return Err(format!(
            "{mnemonic}: the source and destination registers must be the same width"
        ));
    }
    let mn = mnemonic.to_lowercase();
    let rn = atomic_base(operands, 2, &mn)?;
    let suffix = mn.strip_prefix("swp").unwrap_or("");
    // Determine size: 'b' suffix = byte (00), 'h' suffix = half (01), else register-based
    let size = if suffix.contains('b') {
        0b00u32
    } else if suffix.contains('h') {
        0b01u32
    } else if is_64 {
        0b11u32
    } else {
        0b10u32
    };
    // The byte/halfword forms read and write 8/16 bits of a W register.
    if (suffix.contains('b') || suffix.contains('h')) && is_64 {
        return Err(format!(
            "{mn}: the byte/halfword forms take w registers, not x"
        ));
    }
    let a = if suffix.contains('a') { 1u32 } else { 0u32 };
    let r = if suffix.contains('l') { 1u32 } else { 0u32 };
    // size 111000 A R 1 Rs 1 000 00 Rn Rt
    let word = (size << 30)
        | (0b111000 << 24)
        | (a << 23)
        | (r << 22)
        | (1 << 21)
        | (rs << 16)
        | (1 << 15)
        | (rn << 5)
        | rt;
    Ok(EncodeResult::Word(word))
}

/// Encode the LSE read-modify-write atomics LDADD/LDCLR/LDEOR/LDSET/LDSMAX/
/// LDSMIN/LDUMAX/LDUMIN and their acquire/release/byte/halfword variants.
///
/// LDADD Rs, Rt, [Xn]: size 111000 A R 1 Rs 0 opc 00 Rn Rt
/// opc: LDADD=000, LDCLR=001, LDEOR=010, LDSET=011, LDSMAX=100, LDSMIN=101,
///      LDUMAX=110, LDUMIN=111 (pinned against GNU as in the `ldst exclusives`
///      rows of tests/aarch64/operand-legality.tsv)
///
/// The signed/unsigned min/max family is the only part of this group that was
/// missing; the rest of the plumbing (widths, byte/halfword suffix handling,
/// acquire/release bits, base-register rules) is shared.
pub fn encode_ldop(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err(format!("{} requires 3 operands", mnemonic));
    }
    // The LSE atomic ops are GP-only and same-width like CAS.
    let (rs, is_64) = get_gpr_strict(operands, 0)?;
    let (rt, rt_is_64) = get_gpr_strict(operands, 1)?;
    if is_64 != rt_is_64 {
        return Err(format!(
            "{mnemonic}: the source and destination registers must be the same width"
        ));
    }
    let mn = mnemonic.to_lowercase();
    let rn = atomic_base(operands, 2, &mn)?;
    // Determine base op and suffix
    let (base, suffix) = if let Some(s) = mn.strip_prefix("ldadd") {
        (0b000u32, s)
    } else if let Some(s) = mn.strip_prefix("ldclr") {
        (0b001u32, s)
    } else if let Some(s) = mn.strip_prefix("ldeor") {
        (0b010u32, s)
    } else if let Some(s) = mn.strip_prefix("ldset") {
        (0b011u32, s)
    } else if let Some(s) = mn.strip_prefix("ldsmax") {
        (0b100u32, s)
    } else if let Some(s) = mn.strip_prefix("ldsmin") {
        (0b101u32, s)
    } else if let Some(s) = mn.strip_prefix("ldumax") {
        (0b110u32, s)
    } else if let Some(s) = mn.strip_prefix("ldumin") {
        (0b111u32, s)
    } else {
        return Err(format!("unknown ld atomic op: {}", mnemonic));
    };
    // Determine size: 'b' suffix = byte (00), 'h' suffix = half (01), else register-based
    let size = if suffix.contains('b') {
        0b00u32
    } else if suffix.contains('h') {
        0b01u32
    } else if is_64 {
        0b11u32
    } else {
        0b10u32
    };
    // The byte/halfword forms read and write 8/16 bits of a W register.
    if (suffix.contains('b') || suffix.contains('h')) && is_64 {
        return Err(format!(
            "{mn}: the byte/halfword forms take w registers, not x"
        ));
    }
    let a = if suffix.contains('a') { 1u32 } else { 0u32 };
    let r = if suffix.contains('l') { 1u32 } else { 0u32 };
    // size 111000 A R 1 Rs 0 opc 00 Rn Rt
    let word = (size << 30)
        | (0b111000 << 24)
        | (a << 23)
        | (r << 22)
        | (1 << 21)
        | (rs << 16)
        | (base << 12)
        | (rn << 5)
        | rt;
    Ok(EncodeResult::Word(word))
}

/// Encode STADD/STCLR/STEOR/STSET and their release/byte/halfword variants.
/// These are aliases for LDADD/LDCLR/LDEOR/LDSET with Rt=XZR (register 31).
/// STADD Ws, [Xn] encodes as LDADD Ws, WZR, [Xn]
/// Variants: stadd/stclr/steor/stset, plus 'l' (release), 'b' (byte), 'h' (half).
pub fn encode_stop(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err(format!("{} requires 2 operands", mnemonic));
    }
    // The ST-only LSE aliases are GP-only like every other atomic.
    let (rs, is_64) = get_gpr_strict(operands, 0)?;
    let mn = mnemonic.to_lowercase();
    let rn = atomic_base(operands, 1, &mn)?;
    // Determine base op from the prefix
    let (opc, suffix) = if let Some(s) = mn.strip_prefix("stadd") {
        (0b000u32, s)
    } else if let Some(s) = mn.strip_prefix("stclr") {
        (0b001u32, s)
    } else if let Some(s) = mn.strip_prefix("steor") {
        (0b010u32, s)
    } else if let Some(s) = mn.strip_prefix("stset") {
        (0b011u32, s)
    } else {
        return Err(format!("unknown st atomic op: {}", mnemonic));
    };
    // Determine size: 'b' suffix = byte (00), 'h' suffix = half (01), else register-based
    let size = if suffix.contains('b') {
        0b00u32
    } else if suffix.contains('h') {
        0b01u32
    } else if is_64 {
        0b11u32
    } else {
        0b10u32
    };
    // The byte/halfword forms read 8/16 bits of a W register.
    if (suffix.contains('b') || suffix.contains('h')) && is_64 {
        return Err(format!(
            "{mn}: the byte/halfword forms take w registers, not x"
        ));
    }
    // A=0 (no acquire for store aliases), R from 'l' suffix (release)
    let r = if suffix.contains('l') { 1u32 } else { 0u32 };
    let rt = 31u32; // XZR/WZR - discard result
    // size 111000 A R 1 Rs 0 opc 00 Rn Rt
    let word = (size << 30)
        | (0b111000 << 24)
        | (r << 22)
        | (1 << 21)
        | (rs << 16)
        | (opc << 12)
        | (rn << 5)
        | rt;
    Ok(EncodeResult::Word(word))
}

// ── Tests: LSE atomics (CAS family + CASP pair) ──────────────────────────
//
// Every expected word below was cross-verified by round-tripping the
// encoder output through Capstone (aarch64 little-endian), which decodes
// it back to the exact source mnemonic and operands. The CASP layout
// (0|SZ in bits 31-30, NP=0 in bit 23) follows LLVM's
// AArch64InstrFormats.td; note that a naive "same size field as CAS,
// flip bit 23" reading decodes as LDXP/STXP instead.
#[cfg(test)]
mod lse_atomic_tests {
    use super::*;
    use crate::backend::arm::assembler::parser::Operand;

    fn enc(mnemonic: &str, operands: &[Operand]) -> u32 {
        match encode_any(mnemonic, operands).expect(mnemonic) {
            EncodeResult::Word(w) => w,
            other => panic!("{}: expected single word, got {:?}", mnemonic, other),
        }
    }

    // Route through the same mnemonic sets as the dispatch table so the
    // test also pins the mnemonic -> encoder wiring, not just internals.
    fn encode_any(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
        match mnemonic {
            "cas" | "casa" | "casal" | "casl" | "casb" | "casab" | "casalb" | "caslb" | "cash"
            | "casah" | "casalh" | "caslh" => encode_cas(mnemonic, operands),
            "casp" | "caspa" | "caspal" | "caspl" => encode_casp(mnemonic, operands),
            other => Err(format!("test harness: unknown mnemonic {}", other)),
        }
    }

    fn reg(r: &str) -> Operand {
        Operand::Reg(r.to_string())
    }
    fn mem(base: &str) -> Operand {
        Operand::Mem {
            base: base.to_string(),
            offset: 0,
        }
    }
    fn casp_ops(xs: &str, xs1: &str, xt: &str, xt1: &str, base: &str) -> Vec<Operand> {
        vec![reg(xs), reg(xs1), reg(xt), reg(xt1), mem(base)]
    }

    // ── CAS family (pre-existing encoder, now pinned against drift) ──

    #[test]
    fn cas_family_exact_words() {
        let m = mem("x2");
        let x_ops = [reg("x0"), reg("x1"), m.clone()];
        assert_eq!(enc("cas", &x_ops), 0xC8A0_7C41);
        assert_eq!(enc("casa", &x_ops), 0xC8E0_7C41);
        assert_eq!(enc("casl", &x_ops), 0xC8A0_FC41);
        assert_eq!(enc("casal", &x_ops), 0xC8E0_FC41);
        // Size comes from the W register when there is no b/h suffix.
        let w_ops = [reg("w0"), reg("w1"), m.clone()];
        assert_eq!(enc("cas", &w_ops), 0x88A0_7C41);
        assert_eq!(enc("casb", &w_ops), 0x08A0_7C41);
        assert_eq!(enc("casab", &w_ops), 0x08E0_7C41);
        assert_eq!(enc("caslb", &w_ops), 0x08A0_FC41);
        assert_eq!(enc("casalb", &w_ops), 0x08E0_FC41);
        assert_eq!(enc("cash", &w_ops), 0x48A0_7C41);
        assert_eq!(enc("casah", &w_ops), 0x48E0_7C41);
        assert_eq!(enc("casalh", &w_ops), 0x48E0_FC41);
    }

    #[test]
    fn cas_rejects_mistyped_suffixes() {
        let ops = [reg("x0"), reg("x1"), mem("x2")];
        // A lenient contains-based parser would silently encode these as
        // relaxed CAS instead of rejecting them.
        for bad in ["casq", "casx", "caszr", "caslbq"] {
            assert!(encode_any(bad, &ops).is_err(), "{} must be rejected", bad);
        }
    }

    // ── CASP pair family ─────────────────────────────────────────────

    #[test]
    fn casp_family_exact_words() {
        assert_eq!(
            enc("casp", &casp_ops("x0", "x1", "x2", "x3", "x11")),
            0x4820_7D62
        );
        assert_eq!(
            enc("caspa", &casp_ops("x0", "x1", "x2", "x3", "x11")),
            0x4860_7D62
        );
        assert_eq!(
            enc("caspl", &casp_ops("x0", "x1", "x2", "x3", "x11")),
            0x4820_FD62
        );
        assert_eq!(
            enc("caspal", &casp_ops("x0", "x1", "x2", "x3", "x11")),
            0x4860_FD62
        );
        // W pairs: SZ=0 (bit 30 clear).
        assert_eq!(
            enc("caspal", &casp_ops("w0", "w1", "w2", "w3", "x11")),
            0x0860_FD62
        );
        // SP base is GPR64sp-legal.
        assert_eq!(
            enc("caspal", &casp_ops("x0", "x1", "x2", "x3", "sp")),
            0x4860_FFE2
        );
        // High register pair: Rs=2, Rt=4.
        assert_eq!(
            enc("caspal", &casp_ops("x2", "x3", "x4", "x5", "sp")),
            0x4862_FFE4
        );
    }

    #[test]
    fn casp_rejects_invalid_pairs() {
        // Odd pair-start registers: architecturally unallocated encoding.
        assert!(encode_any("caspal", &casp_ops("x1", "x2", "x2", "x3", "x11")).is_err());
        // Non-consecutive second registers: the encoding field only stores
        // Xs/Xt, so accepting these would silently swap which registers
        // participate in the atomic operation.
        assert!(encode_any("caspal", &casp_ops("x0", "x2", "x4", "x5", "x11")).is_err());
        // Mixed widths across or within pairs.
        assert!(encode_any("caspal", &casp_ops("x0", "x1", "w2", "w3", "x11")).is_err());
        assert!(encode_any("caspal", &casp_ops("w0", "x1", "x2", "x3", "x11")).is_err());
        // SP/xzr as pair member (lr = x30 is fine, sp = 31 is not).
        assert!(encode_any("caspal", &casp_ops("x30", "sp", "x2", "x3", "x11")).is_err());
        assert!(encode_any("caspal", &casp_ops("xzr", "x1", "x2", "x3", "x11")).is_err());
        // xzr as base register.
        assert!(encode_any("caspal", &casp_ops("x0", "x1", "x2", "x3", "xzr")).is_err());
        // Offset/writeback memory forms do not exist for CASP.
        let off = vec![
            reg("x0"),
            reg("x1"),
            reg("x2"),
            reg("x3"),
            Operand::Mem {
                base: "x11".to_string(),
                offset: 8,
            },
        ];
        assert!(encode_any("caspal", &off).is_err());
        // Wrong operand count.
        let short = vec![reg("x0"), reg("x1"), reg("x2"), mem("x11")];
        assert!(encode_any("caspal", &short).is_err());
        // No byte/halfword pair forms; no mistyped order letters.
        assert!(encode_any("caspalb", &casp_ops("x0", "x1", "x2", "x3", "x11")).is_err());
        assert!(encode_any("caspaq", &casp_ops("x0", "x1", "x2", "x3", "x11")).is_err());
        assert!(encode_any("caspx", &casp_ops("x0", "x1", "x2", "x3", "x11")).is_err());
    }
}
