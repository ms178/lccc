use super::*;
use crate::backend::arm::assembler::parser::Operand;

// ── Loads/Stores ─────────────────────────────────────────────────────────

/// Auto-detect LDR/STR size from the first register operand.
pub(crate) fn encode_ldr_str_auto(
    operands: &[Operand],
    is_load: bool,
) -> Result<EncodeResult, String> {
    // Determine size from register: Wn -> 32-bit (size=10), Xn -> 64-bit (size=11)
    // FP: Sn -> 32-bit, Dn -> 64-bit, Qn -> 128-bit
    let reg_name = match operands.first() {
        Some(Operand::Reg(r)) => r.to_lowercase(),
        _ => return Err("ldr/str needs register operand".to_string()),
    };

    let size = if reg_name.starts_with('w') {
        0b10 // 32-bit
    } else if reg_name.starts_with('x') || reg_name == "sp" || reg_name == "xzr" || reg_name == "lr"
    {
        0b11 // 64-bit
    } else if reg_name.starts_with('s') {
        0b10 // 32-bit float
    } else if reg_name.starts_with('d') {
        0b11 // 64-bit float
    } else if reg_name.starts_with('q') {
        0b00 // 128-bit: size=00 with opc adjustment in encode_ldr_str
    } else {
        0b11 // default 64-bit
    };

    let is_128bit = reg_name.starts_with('q');
    encode_ldr_str(operands, is_load, size, false, is_128bit)
}

pub(crate) fn encode_ldr_str(
    operands: &[Operand],
    is_load: bool,
    size: u32,
    is_signed: bool,
    is_128bit: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("ldr/str requires at least 2 operands".to_string());
    }

    let (rt, _) = get_reg(operands, 0)?;
    let fp = is_fp_reg(
        operands
            .first()
            .map(|o| match o {
                Operand::Reg(r) => r.as_str(),
                _ => "",
            })
            .unwrap_or(""),
    );

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
                if is_load {
                    0b11
                } else {
                    0b10
                }
            } else if is_load {
                if is_signed {
                    0b10
                } else {
                    0b01
                }
            } else {
                0b00
            };

            // Check if offset is aligned and fits in 12-bit unsigned field
            let abs_offset = *offset as u64;
            let align = 1u64 << shift;
            if *offset >= 0 && abs_offset % align == 0 {
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

            // Unscaled offset (LDUR/STUR form)
            let imm9 = (*offset as i32) & 0x1FF;
            let opc = if is_128bit {
                if is_load {
                    0b11
                } else {
                    0b10
                }
            } else if is_load {
                if is_signed {
                    0b10
                } else {
                    0b01
                }
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
            let imm9 = (*offset as i32) & 0x1FF;
            let opc = if is_128bit {
                if is_load {
                    0b11
                } else {
                    0b10
                }
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
            let imm9 = (*offset as i32) & 0x1FF;
            let opc = if is_128bit {
                if is_load {
                    0b11
                } else {
                    0b10
                }
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
                    if is_load {
                        0b11
                    } else {
                        0b10
                    }
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
                if is_load {
                    0b11
                } else {
                    0b10
                }
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
                if actual_size == 0b11 {
                    0b01
                } else {
                    0b00
                }
            } else {
                // GP: W=00, X=01
                if actual_size == 0b11 {
                    0b01
                } else {
                    0b00
                }
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
pub(crate) fn encode_ldur_stur(
    operands: &[Operand],
    is_load: bool,
    op2_bits: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("ldur/stur requires 2 operands".to_string());
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
            (rn, *offset as i32)
        }
        _ => {
            return Err(format!(
                "ldur/stur: expected memory operand, got {:?}",
                operands[1]
            ))
        }
    };

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
pub(crate) fn encode_ldtr_sized(
    operands: &[Operand],
    is_load: bool,
    size: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("ldtr/sttr requires 2 operands".to_string());
    }
    let (rt, _) = get_reg(operands, 0)?;
    let opc = if is_load { 0b01u32 } else { 0b00 };
    let (rn, imm9) = match &operands[1] {
        Operand::Mem { base, offset } => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            (rn, *offset as i32)
        }
        _ => return Err("ldtr/sttr: expected memory operand".to_string()),
    };
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

pub(crate) fn encode_ldrsw(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("ldrsw requires 2 operands".to_string());
    }

    let (rt, _) = get_reg(operands, 0)?;

    match operands.get(1) {
        Some(Operand::Mem { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            // LDRSW: size=10 111 V=0 01 opc=10 -> unsigned offset
            // Actually: 10 111 0 01 10 imm12 Rn Rt
            let abs_offset = *offset as u64;
            if *offset >= 0 && abs_offset % 4 == 0 {
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
            let imm9 = (*offset as i32) & 0x1FF;
            let word =
                (((0b10 << 30) | (0b111 << 27)) | (0b10 << 22) | ((imm9 as u32 & 0x1FF) << 12))
                    | (rn << 5)
                    | rt;
            return Ok(EncodeResult::Word(word));
        }

        Some(Operand::MemPostIndex { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            let imm9 = (*offset as i32) & 0x1FF;
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
            let imm9 = (*offset as i32) & 0x1FF;
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
                    ))
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

pub(crate) fn encode_ldrs(operands: &[Operand], size: u32) -> Result<EncodeResult, String> {
    // LDRSB/LDRSH: sign-extending byte/halfword loads
    if operands.len() < 2 {
        return Err("ldrsb/ldrsh requires 2 operands".to_string());
    }

    let (rt, is_64) = get_reg(operands, 0)?;
    let opc = if is_64 { 0b10 } else { 0b11 }; // 64-bit target: opc=10, 32-bit: opc=11

    if let Some(Operand::Mem { base, offset }) = operands.get(1) {
        let rn = parse_reg_num(base).ok_or("invalid base reg")?;
        let shift = size;
        let abs_offset = *offset as u64;
        let align = 1u64 << shift;
        if *offset >= 0 && abs_offset % align == 0 {
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
        let imm9 = (*offset as i32) & 0x1FF;
        let word = (((size << 30) | (0b111 << 27)) | (opc << 22) | ((imm9 as u32 & 0x1FF) << 12))
            | (rn << 5)
            | rt;
        return Ok(EncodeResult::Word(word));
    }

    // Post-index: ldrsb/ldrsh Rt, [Xn], #imm
    if let Some(Operand::MemPostIndex { base, offset }) = operands.get(1) {
        let rn = parse_reg_num(base).ok_or("invalid base reg")?;
        let imm9 = (*offset as i32) & 0x1FF;
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
        let imm9 = (*offset as i32) & 0x1FF;
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

pub(crate) fn encode_ldp_stp(operands: &[Operand], is_load: bool) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("ldp/stp requires 3 operands".to_string());
    }

    let (rt1, is_64) = get_reg(operands, 0)?;
    let (rt2, _) = get_reg(operands, 1)?;
    let fp = is_fp_reg(match &operands[0] {
        Operand::Reg(r) => r.as_str(),
        _ => "",
    });

    let opc = if fp {
        let r = match &operands[0] {
            Operand::Reg(r) => r.to_lowercase(),
            _ => String::new(),
        };
        if r.starts_with('s') {
            0b00
        } else if r.starts_with('d') {
            0b01
        } else if r.starts_with('q') || is_64 {
            0b10
        } else {
            0b00
        }
    } else if is_64 {
        0b10
    } else {
        0b00
    };

    let v = if fp { 1u32 } else { 0u32 };
    let l = if is_load { 1u32 } else { 0u32 };

    // Shift depends on register size
    let shift = if fp {
        let r = match &operands[0] {
            Operand::Reg(r) => r.to_lowercase(),
            _ => String::new(),
        };
        if r.starts_with('s') {
            2
        } else if r.starts_with('d') {
            3
        } else if r.starts_with('q') {
            4
        } else if is_64 {
            3
        } else {
            2
        }
    } else if is_64 {
        3
    } else {
        2
    };

    match operands.get(2) {
        // STP rt1, rt2, [base, #offset]! (pre-index)
        Some(Operand::MemPreIndex { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            let imm7 = ((*offset >> shift) as i32) & 0x7F;
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
            let imm7 = ((*offset >> shift) as i32) & 0x7F;
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
            let imm7 = ((*offset >> shift) as i32) & 0x7F;
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
pub(crate) fn encode_ldnp_stnp(
    operands: &[Operand],
    is_load: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("ldnp/stnp requires 3 operands".to_string());
    }

    let (rt1, is_64) = get_reg(operands, 0)?;
    let (rt2, _) = get_reg(operands, 1)?;

    let opc: u32 = if is_64 { 0b10 } else { 0b00 };
    let l: u32 = if is_load { 1 } else { 0 };
    let shift = if is_64 { 3 } else { 2 }; // scale factor: 8 for 64-bit, 4 for 32-bit

    match operands.get(2) {
        Some(Operand::Mem { base, offset }) => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            let imm7 = ((*offset >> shift) as i32) & 0x7F;
            // LDNP/STNP: opc 101 V=0 000 L imm7 Rt2 Rn Rt
            let word = (opc << 30)
                | (0b101 << 27)
                | (l << 22)
                | ((imm7 as u32 & 0x7F) << 15)
                | (rt2 << 10)
                | (rn << 5)
                | rt1;
            Ok(EncodeResult::Word(word))
        }
        _ => Err(format!("unsupported ldnp/stnp operands: {:?}", operands)),
    }
}

// ── Exclusive loads/stores ───────────────────────────────────────────────

/// Encode LDXR/STXR and byte/halfword variants.
/// `forced_size`: None = auto-detect from register width, Some(0b00) = byte, Some(0b01) = halfword
pub(crate) fn encode_ldxr_stxr(
    operands: &[Operand],
    is_load: bool,
    forced_size: Option<u32>,
) -> Result<EncodeResult, String> {
    if is_load {
        let (rt, is_64) = get_reg(operands, 0)?;
        let rn = match operands.get(1) {
            Some(Operand::Mem { base, .. }) => parse_reg_num(base).ok_or("invalid base")?,
            _ => return Err("ldxr needs memory operand".to_string()),
        };
        let size = forced_size.unwrap_or(if is_64 { 0b11 } else { 0b10 });
        let word = ((size << 30) | (0b001000010 << 21) | (0b11111 << 16))
            | (0b11111 << 10)
            | (rn << 5)
            | rt;
        Ok(EncodeResult::Word(word))
    } else {
        let (ws, _) = get_reg(operands, 0)?;
        let (rt, is_64) = get_reg(operands, 1)?;
        let rn = match operands.get(2) {
            Some(Operand::Mem { base, .. }) => parse_reg_num(base).ok_or("invalid base")?,
            _ => return Err("stxr needs memory operand".to_string()),
        };
        let size = forced_size.unwrap_or(if is_64 { 0b11 } else { 0b10 });
        let word =
            ((size << 30) | (0b001000000 << 21) | (ws << 16)) | (0b11111 << 10) | (rn << 5) | rt;
        Ok(EncodeResult::Word(word))
    }
}

/// Encode LDAXR/STLXR and byte/halfword variants.
pub(crate) fn encode_ldaxr_stlxr(
    operands: &[Operand],
    is_load: bool,
    forced_size: Option<u32>,
) -> Result<EncodeResult, String> {
    if is_load {
        let (rt, is_64) = get_reg(operands, 0)?;
        let rn = match operands.get(1) {
            Some(Operand::Mem { base, .. }) => parse_reg_num(base).ok_or("invalid base")?,
            _ => return Err("ldaxr needs memory operand".to_string()),
        };
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
        let (ws, _) = get_reg(operands, 0)?;
        let (rt, is_64) = get_reg(operands, 1)?;
        let rn = match operands.get(2) {
            Some(Operand::Mem { base, .. }) => parse_reg_num(base).ok_or("invalid base")?,
            _ => return Err("stlxr needs memory operand".to_string()),
        };
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
/// LDXP  Xt1, Xt2, [Xn]  : sz 001000 0 1 1 11111 0 Rt2 Rn Rt
/// LDAXP Xt1, Xt2, [Xn]  : sz 001000 0 1 1 11111 1 Rt2 Rn Rt
/// STXP  Ws, Xt1, Xt2, [Xn] : sz 001000 0 0 1 Rs 0 Rt2 Rn Rt
/// STLXP Ws, Xt1, Xt2, [Xn] : sz 001000 0 0 1 Rs 1 Rt2 Rn Rt
pub(crate) fn encode_ldxp_stxp(
    operands: &[Operand],
    is_load: bool,
    acquire_release: bool,
) -> Result<EncodeResult, String> {
    let o0 = if acquire_release { 1u32 } else { 0 };
    if is_load {
        // LDXP/LDAXP Rt, Rt2, [Rn]
        let (rt, is_64) = get_reg(operands, 0)?;
        let (rt2, _) = get_reg(operands, 1)?;
        let rn = match operands.get(2) {
            Some(Operand::Mem { base, .. }) => {
                parse_reg_num(base).ok_or("ldxp needs memory operand")?
            }
            _ => return Err("ldxp needs memory operand".to_string()),
        };
        let sz = if is_64 { 1u32 } else { 0 };
        // 1 sz 001000 0 1 1 11111 o0 Rt2 Rn Rt  (bit23=0)
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
        // STXP/STLXP Ws, Rt, Rt2, [Rn]
        let (ws, _) = get_reg(operands, 0)?; // status register (always W)
        let (rt, is_64) = get_reg(operands, 1)?;
        let (rt2, _) = get_reg(operands, 2)?;
        let rn = match operands.get(3) {
            Some(Operand::Mem { base, .. }) => {
                parse_reg_num(base).ok_or("stxp needs memory operand")?
            }
            _ => return Err("stxp needs memory operand".to_string()),
        };
        let sz = if is_64 { 1u32 } else { 0 };
        // 1 sz 001000 0 0 1 Rs o0 Rt2 Rn Rt  (bit23=0, bit22=0)
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
pub(crate) fn encode_ldar_stlr(
    operands: &[Operand],
    is_load: bool,
    forced_size: Option<u32>,
) -> Result<EncodeResult, String> {
    let (rt, is_64) = get_reg(operands, 0)?;
    let rn = match operands.get(1) {
        Some(Operand::Mem { base, .. }) => parse_reg_num(base).ok_or("invalid base")?,
        _ => return Err("ldar/stlr needs memory operand".to_string()),
    };
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

pub(crate) fn encode_adrp(operands: &[Operand]) -> Result<EncodeResult, String> {
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
            ))
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

pub(crate) fn encode_adr(operands: &[Operand]) -> Result<EncodeResult, String> {
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
pub(crate) fn encode_prfm(operands: &[Operand]) -> Result<EncodeResult, String> {
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
            ))
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
            let imm12 = (imm / 8) as u32;
            if imm12 > 0xFFF {
                return Err(format!("prfm: offset too large: {}", imm));
            }
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
                | (0b10 << 23)
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

/// Map prefetch operation name to its 5-bit encoding.
pub(crate) fn encode_prfop(name: &str) -> Result<u32, String> {
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
///   ""    relaxed          "b"  byte   (CAS family only)
///   "a"   acquire          "h"  half   (CAS family only)
///   "l"   release
///   "al"  acquire+release
/// Byte/half letters always trail the order letters (`casalb`, `caslb`, ...).
///
/// The parse is exact rather than a `contains` probe: a lenient contains
/// check silently encodes a mistyped mnemonic such as `casq` as relaxed
/// CAS, i.e. the assembler would accept and mis-assemble garbage.
/// Returns `(acquire, release, size_letter)`.
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
/// CAS  SZ  |001000|1|A|1|Rs|R|11111|Rn|Rt   (LLVM AArch64InstrFormats.td;
/// round-trip verified against Capstone for all twelve order/size forms)
pub(crate) fn encode_cas(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err(format!("{} requires 3 operands", mnemonic));
    }
    let (rs, is_64) = get_reg(operands, 0)?;
    let (rt, _) = get_reg(operands, 1)?;
    let rn = match operands.get(2) {
        Some(Operand::Mem { base, .. }) => parse_reg_num(base).ok_or("cas: invalid base")?,
        _ => return Err("cas requires memory operand [Xn]".to_string()),
    };
    let mn = mnemonic.to_lowercase();
    let suffix = mn.strip_prefix("cas").unwrap_or("");
    let (a, l, size_letter) = parse_atomic_order_suffix("cas", suffix)?;
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
/// CASP  0|SZ|001000|0|A|1|Rs|R|11111|Rn|Rt
///
/// Layout per LLVM AArch64InstrFormats.td (Capstone round-trip verified):
/// - bit 31 is always 0 and SZ (bit 30) selects X pairs (1) vs W pairs (0);
///   this is why CASP does NOT live at the same size-field position as CAS.
/// - bit 23 (NP) is 0 here and 1 for CAS — the architectural CAS/CASP split.
/// - A (bit 22) / R (bit 15) are the acquire/release order bits.
///
/// Operand order: `casp<order> Xs, Xs+1, Xt, Xt+1, [Xn]` — the first pair
/// holds the expected (compare) value and receives the loaded old value
/// (both members are in-out), the second pair holds the new value.
/// Only Xs and Xt occupy encoding fields; Xs+1 / Xt+1 are architecturally
/// implied, so the text operands are validated to match exactly (GAS
/// parity: even-numbered start register, consecutive pairs, uniform width).
pub(crate) fn encode_casp(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
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
            ))
        }
        other => {
            return Err(format!(
                "{}: expected memory operand [Xn], got {:?}",
                mnemonic, other
            ))
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
pub(crate) fn encode_swp(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err(format!("{} requires 3 operands", mnemonic));
    }
    let (rs, is_64) = get_reg(operands, 0)?;
    let (rt, _) = get_reg(operands, 1)?;
    let rn = match operands.get(2) {
        Some(Operand::Mem { base, .. }) => parse_reg_num(base).ok_or("swp: invalid base")?,
        _ => return Err("swp requires memory operand [Xn]".to_string()),
    };
    let mn = mnemonic.to_lowercase();
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

/// Encode LDADD/LDCLR/LDEOR/LDSET and their acquire/release/byte/halfword variants (LSE atomics).
/// LDADD Rs, Rt, [Xn]: size 111000 A R 1 Rs 0 opc 00 Rn Rt
/// opc: LDADD=000, LDCLR=001, LDEOR=010, LDSET=011
pub(crate) fn encode_ldop(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err(format!("{} requires 3 operands", mnemonic));
    }
    let (rs, is_64) = get_reg(operands, 0)?;
    let (rt, _) = get_reg(operands, 1)?;
    let rn = match operands.get(2) {
        Some(Operand::Mem { base, .. }) => parse_reg_num(base).ok_or("ldop: invalid base")?,
        _ => return Err(format!("{} requires memory operand [Xn]", mnemonic)),
    };
    let mn = mnemonic.to_lowercase();
    // Determine base op and suffix
    let (base, suffix) = if let Some(s) = mn.strip_prefix("ldadd") {
        (0b000u32, s)
    } else if let Some(s) = mn.strip_prefix("ldclr") {
        (0b001u32, s)
    } else if let Some(s) = mn.strip_prefix("ldeor") {
        (0b010u32, s)
    } else if let Some(s) = mn.strip_prefix("ldset") {
        (0b011u32, s)
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
pub(crate) fn encode_stop(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err(format!("{} requires 2 operands", mnemonic));
    }
    let (rs, is_64) = get_reg(operands, 0)?;
    let rn = match operands.get(1) {
        Some(Operand::Mem { base, .. }) => {
            parse_reg_num(base).ok_or_else(|| format!("{}: invalid base", mnemonic))?
        }
        _ => return Err(format!("{} requires memory operand [Xn]", mnemonic)),
    };
    let mn = mnemonic.to_lowercase();
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
