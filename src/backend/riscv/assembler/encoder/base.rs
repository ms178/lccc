use super::*;

// ── Instruction encoders ──────────────────────────────────────────────

pub(crate) fn encode_lui(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    match &operands.get(1) {
        Some(Operand::Imm(imm)) => Ok(EncodeResult::Word(encode_u(
            OP_LUI,
            rd,
            (*imm as u32) << 12,
        ))),
        Some(Operand::Symbol(s)) => {
            // %hi(symbol)
            Ok(EncodeResult::WordWithReloc {
                word: encode_u(OP_LUI, rd, 0),
                reloc: Relocation {
                    reloc_type: if s.starts_with("%tprel_hi(") {
                        RelocType::TprelHi20
                    } else {
                        RelocType::Hi20
                    },
                    symbol: extract_modifier_symbol(s),
                    addend: 0,
                },
            })
        }
        _ => Err("lui: invalid operands".to_string()),
    }
}

pub(crate) fn encode_auipc(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    match &operands.get(1) {
        Some(Operand::Imm(imm)) => Ok(EncodeResult::Word(encode_u(
            OP_AUIPC,
            rd,
            (*imm as u32) << 12,
        ))),
        Some(Operand::Symbol(s)) => {
            let (reloc_type, symbol) = parse_reloc_modifier(s);
            Ok(EncodeResult::WordWithReloc {
                word: encode_u(OP_AUIPC, rd, 0),
                reloc: Relocation {
                    reloc_type,
                    symbol,
                    addend: 0,
                },
            })
        }
        _ => Err("auipc: invalid operands".to_string()),
    }
}

/// Validate a literal PC-relative branch displacement.
///
/// The B-type (+-4 KiB) and J-type (+-1 MiB) encodings both encode bit 0 of the
/// displacement IMPLICITLY -- `encode_b` splits `imm[4:1]`, `encode_j` splits
/// `imm[10:1]`, and neither layout has an `imm[0]` field. An odd offset is
/// therefore silently truncated: `beq x1, x2, 17` encoded to the *identical*
/// word `0x00208863` as `beq x1, x2, 16`, branching 16 bytes instead of 17.
///
/// Only a LITERAL immediate operand reaches here. A branch whose target is a
/// symbol becomes an `R_RISCV_BRANCH` / `R_RISCV_JAL` relocation that the
/// linker resolves -- deliberately, because linker relaxation can shrink the
/// code after assembly, so the offset is genuinely not knowable here. For a
/// literal it IS knowable, so truncating it is a misassembly, not a deferral.
pub(crate) fn check_branch_offset(imm: i64, lo: i64, hi: i64, what: &str) -> Result<(), String> {
    if imm & 1 != 0 {
        return Err(format!(
            "{what}: misaligned branch offset {imm}; RISC-V branch and jump \
             immediates encode bit 0 implicitly, so {imm} would silently branch \
             to {} -- use an even offset",
            imm & !1
        ));
    }
    if !(lo..hi).contains(&imm) {
        return Err(format!(
            "{what}: branch out of range: offset {imm} is not encodable \
             (allowed {lo}..{hi})"
        ));
    }
    Ok(())
}

pub(crate) fn encode_jal(operands: &[Operand]) -> Result<EncodeResult, String> {
    // jal rd, offset  OR  jal offset (rd = ra)
    //
    // Without this bound a surplus operand is silently discarded, so a typo
    // such as `jal ra, target, extra` assembles as a valid jump instead of
    // being diagnosed.
    if operands.len() > 2 {
        return Err(format!(
            "jal: expected 1 or 2 operands, got {}",
            operands.len()
        ));
    }
    if operands.len() == 1 {
        // jal offset (implicit rd = ra)
        match &operands[0] {
            Operand::Imm(imm) => {
                check_branch_offset(*imm, -(1 << 20), 1 << 20, "jal")?;
                Ok(EncodeResult::Word(encode_j(OP_JAL, 1, *imm as i32)))
            }
            Operand::Symbol(s) | Operand::Label(s) | Operand::Reg(s) => {
                Ok(EncodeResult::WordWithReloc {
                    word: encode_j(OP_JAL, 1, 0),
                    reloc: Relocation {
                        reloc_type: RelocType::Jal,
                        symbol: s.clone(),
                        addend: 0,
                    },
                })
            }
            _ => Err("jal: invalid operand".to_string()),
        }
    } else {
        let rd = get_reg(operands, 0)?;
        match &operands[1] {
            Operand::Imm(imm) => {
                check_branch_offset(*imm, -(1 << 20), 1 << 20, "jal")?;
                Ok(EncodeResult::Word(encode_j(OP_JAL, rd, *imm as i32)))
            }
            Operand::Symbol(s) | Operand::Label(s) | Operand::Reg(s) => {
                Ok(EncodeResult::WordWithReloc {
                    word: encode_j(OP_JAL, rd, 0),
                    reloc: Relocation {
                        reloc_type: RelocType::Jal,
                        symbol: s.clone(),
                        addend: 0,
                    },
                })
            }
            _ => Err("jal: invalid operand".to_string()),
        }
    }
}

pub(crate) fn encode_jalr(operands: &[Operand]) -> Result<EncodeResult, String> {
    // jalr rd, rs1, offset  OR  jalr rd, offset(rs1)  OR  jalr rs1
    match operands.len() {
        1 => {
            // jalr rs1 (rd = ra, offset = 0)
            let rs1 = get_reg(operands, 0)?;
            Ok(EncodeResult::Word(encode_i(OP_JALR, 1, 0, rs1, 0)))
        }
        2 => {
            // jalr rd, rs1  (offset = 0)
            let rd = get_reg(operands, 0)?;
            match &operands[1] {
                Operand::Reg(name) => {
                    let rs1 = reg_num(name).ok_or("invalid register")?;
                    Ok(EncodeResult::Word(encode_i(OP_JALR, rd, 0, rs1, 0)))
                }
                Operand::Mem { base, offset } => {
                    let rs1 = reg_num(base).ok_or("invalid base register")?;
                    Ok(EncodeResult::Word(encode_i(
                        OP_JALR,
                        rd,
                        0,
                        rs1,
                        *offset as i32,
                    )))
                }
                _ => Err("jalr: invalid operands".to_string()),
            }
        }
        3 => {
            let rd = get_reg(operands, 0)?;
            let rs1 = get_reg(operands, 1)?;
            let imm = get_imm(operands, 2)?;
            Ok(EncodeResult::Word(encode_i(
                OP_JALR, rd, 0, rs1, imm as i32,
            )))
        }
        _ => Err("jalr: wrong number of operands".to_string()),
    }
}

pub(crate) fn encode_branch_instr(
    operands: &[Operand],
    funct3: u32,
) -> Result<EncodeResult, String> {
    if operands.len() != 3 {
        return Err(format!(
            "branch: expected 3 operands (rs1, rs2, target), got {}",
            operands.len()
        ));
    }
    let rs1 = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;

    match &operands.get(2) {
        Some(Operand::Imm(imm)) => {
            check_branch_offset(*imm, -(1 << 12), 1 << 12, "branch")?;
            Ok(EncodeResult::Word(encode_b(
                OP_BRANCH,
                funct3,
                rs1,
                rs2,
                *imm as i32,
            )))
        }
        Some(Operand::Symbol(s)) | Some(Operand::Label(s)) | Some(Operand::Reg(s)) => {
            Ok(EncodeResult::WordWithReloc {
                word: encode_b(OP_BRANCH, funct3, rs1, rs2, 0),
                reloc: Relocation {
                    reloc_type: RelocType::Branch,
                    symbol: s.clone(),
                    addend: 0,
                },
            })
        }
        _ => Err("branch: expected offset or label as 3rd operand".to_string()),
    }
}

pub(crate) fn encode_load(operands: &[Operand], funct3: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    match &operands.get(1) {
        Some(Operand::Mem { base, offset }) => {
            let rs1 = reg_num(base).ok_or("invalid base register")?;
            Ok(EncodeResult::Word(encode_i(
                OP_LOAD,
                rd,
                funct3,
                rs1,
                *offset as i32,
            )))
        }
        Some(Operand::MemSymbol { base, symbol, .. }) => {
            let rs1 = reg_num(base).ok_or("invalid base register")?;
            let (reloc_type, sym) = parse_reloc_modifier(symbol);
            let reloc_type = lo12_itype(reloc_type);
            Ok(EncodeResult::WordWithReloc {
                word: encode_i(OP_LOAD, rd, funct3, rs1, 0),
                reloc: Relocation {
                    reloc_type,
                    symbol: sym,
                    addend: 0,
                },
            })
        }
        // Bare symbol: "ld rd, symbol" pseudo-instruction
        // Expand to: auipc rd, %pcrel_hi(symbol) ; ld rd, 0(rd)
        // with R_RISCV_PCREL_HI20 on auipc and R_RISCV_PCREL_LO12_I on ld
        Some(Operand::Symbol(s)) | Some(Operand::Label(s)) => {
            Ok(EncodeResult::WordsWithRelocs(vec![
                (
                    encode_u(OP_AUIPC, rd, 0),
                    Some(Relocation {
                        reloc_type: RelocType::PcrelHi20,
                        symbol: s.clone(),
                        addend: 0,
                    }),
                ),
                (
                    encode_i(OP_LOAD, rd, funct3, rd, 0),
                    Some(Relocation {
                        reloc_type: RelocType::PcrelLo12I,
                        symbol: s.clone(),
                        addend: 0,
                    }),
                ),
            ]))
        }
        _ => Err("load: expected memory operand".to_string()),
    }
}

pub(crate) fn encode_store(operands: &[Operand], funct3: u32) -> Result<EncodeResult, String> {
    let rs2 = get_reg(operands, 0)?;
    match &operands.get(1) {
        Some(Operand::Mem { base, offset }) => {
            let rs1 = reg_num(base).ok_or("invalid base register")?;
            Ok(EncodeResult::Word(encode_s(
                OP_STORE,
                funct3,
                rs1,
                rs2,
                *offset as i32,
            )))
        }
        Some(Operand::MemSymbol { base, symbol, .. }) => {
            let rs1 = reg_num(base).ok_or("invalid base register")?;
            let (reloc_type, sym) = parse_reloc_modifier(symbol);
            let reloc_type = lo12_stype(reloc_type);
            Ok(EncodeResult::WordWithReloc {
                word: encode_s(OP_STORE, funct3, rs1, rs2, 0),
                reloc: Relocation {
                    reloc_type,
                    symbol: sym,
                    addend: 0,
                },
            })
        }
        _ => Err("store: expected memory operand".to_string()),
    }
}

pub(crate) fn encode_alu_imm(operands: &[Operand], funct3: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    match &operands.get(2) {
        Some(Operand::Imm(imm)) => Ok(EncodeResult::Word(encode_i(
            OP_OP_IMM,
            rd,
            funct3,
            rs1,
            *imm as i32,
        ))),
        Some(Operand::Symbol(s)) => {
            let (reloc_type, sym) = parse_reloc_modifier(s);
            let reloc_type = lo12_itype(reloc_type);
            Ok(EncodeResult::WordWithReloc {
                word: encode_i(OP_OP_IMM, rd, funct3, rs1, 0),
                reloc: Relocation {
                    reloc_type,
                    symbol: sym,
                    addend: 0,
                },
            })
        }
        _ => Err("alu_imm: expected immediate".to_string()),
    }
}

// Register shifts mask the count, but an assembly immediate must fit the
// instruction's shamt field. Check the signed value before any narrowing;
// casting first would accept negative and large counts after truncation.
fn checked_shift_amount(operands: &[Operand], bits: u32) -> Result<u32, String> {
    let shamt = get_imm(operands, 2)?;
    let maximum = (1i64 << bits) - 1;
    if !(0..=maximum).contains(&shamt) {
        return Err(format!(
            "shift amount {shamt} out of range (expected 0..{maximum})"
        ));
    }
    Ok(shamt as u32)
}

pub(crate) fn encode_shift_imm(
    operands: &[Operand],
    funct3: u32,
    funct6: u32,
) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    let shamt = checked_shift_amount(operands, 6)?;
    // For RV64, shift amount is 6 bits
    let imm = (funct6 << 6) | (shamt & 0x3F);
    Ok(EncodeResult::Word(encode_i(
        OP_OP_IMM, rd, funct3, rs1, imm as i32,
    )))
}

pub(crate) fn encode_alu_reg(
    operands: &[Operand],
    funct3: u32,
    funct7: u32,
) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    let rs2 = get_reg(operands, 2)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP, rd, funct3, rs1, rs2, funct7,
    )))
}

pub(crate) fn encode_alu_imm_w(operands: &[Operand], funct3: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    let imm = get_imm(operands, 2)? as i32;
    Ok(EncodeResult::Word(encode_i(
        OP_OP_IMM_32,
        rd,
        funct3,
        rs1,
        imm,
    )))
}

pub(crate) fn encode_shift_imm_w(
    operands: &[Operand],
    funct3: u32,
    funct7: u32,
) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    let shamt = checked_shift_amount(operands, 5)?;
    // For RV32/W operations, shift amount is 5 bits
    let imm = (funct7 << 5) | (shamt & 0x1F);
    Ok(EncodeResult::Word(encode_i(
        OP_OP_IMM_32,
        rd,
        funct3,
        rs1,
        imm as i32,
    )))
}

pub(crate) fn encode_alu_reg_w(
    operands: &[Operand],
    funct3: u32,
    funct7: u32,
) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    let rs2 = get_reg(operands, 2)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_32, rd, funct3, rs1, rs2, funct7,
    )))
}

// ── Zbb (bit manipulation) helpers ──

/// Encode a Zbb unary instruction (clz, ctz, cpop, sext.b, sext.h, rev8).
/// These are I-type with funct3=001 and the 12-bit immediate encoding the operation.
pub(crate) fn encode_zbb_unary(operands: &[Operand], imm12: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_OP_IMM,
        rd,
        0b001,
        rs1,
        imm12 as i32,
    )))
}

/// Encode a Zbb unary instruction with funct3=101 (rev8, orc.b).
pub(crate) fn encode_zbb_unary_f5(
    operands: &[Operand],
    imm12: u32,
) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_OP_IMM,
        rd,
        0b101,
        rs1,
        imm12 as i32,
    )))
}

/// Encode a Zbb unary word instruction (clzw, ctzw, cpopw).
/// These are I-type on OP-IMM-32 with funct3=001.
pub(crate) fn encode_zbb_unary_w(operands: &[Operand], imm12: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_OP_IMM_32,
        rd,
        0b001,
        rs1,
        imm12 as i32,
    )))
}

/// Encode zext.h rd, rs1 (R-type on OP-32: funct7=0000100, rs2=0, funct3=100).
pub(crate) fn encode_zbb_zexth(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_32, rd, 0b100, rs1, 0, 0b0000100,
    )))
}

#[cfg(test)]
mod shift_immediate_range_tests {
    use super::*;

    fn operands(count: i64) -> [Operand; 3] {
        [
            Operand::Reg("a0".into()),
            Operand::Reg("a1".into()),
            Operand::Imm(count),
        ]
    }

    #[test]
    fn all_valid_shift_and_rotate_immediates_keep_exact_bits() {
        for (mnemonic, f3, upper, bits, opcode) in [
            ("slli", 1, 0, 6, 0x13),
            ("srli", 5, 0, 6, 0x13),
            ("srai", 5, 0b010000, 6, 0x13),
            ("rori", 5, 0b011000, 6, 0x13),
            ("slliw", 1, 0, 5, 0x1b),
            ("srliw", 5, 0, 5, 0x1b),
            ("sraiw", 5, 0b0100000, 5, 0x1b),
            ("roriw", 5, 0b0110000, 5, 0x1b),
        ] {
            for count in 0..(1 << bits) {
                let encoded = encode_instruction(mnemonic, &operands(count), "").unwrap();
                let EncodeResult::Word(word) = encoded else {
                    panic!("{encoded:?}")
                };
                let imm = ((upper as u32) << bits) | count as u32;
                let expected = (imm << 20) | (11 << 15) | (f3 << 12) | (10 << 7) | opcode;
                assert_eq!(word, expected, "{mnemonic} {count}");
            }
        }
    }

    #[test]
    fn out_of_range_counts_are_not_masked_or_truncated() {
        for (mnemonic, max) in [
            ("slli", 63),
            ("srli", 63),
            ("srai", 63),
            ("rori", 63),
            ("sll", 63),
            ("srl", 63),
            ("sra", 63),
            ("ror", 63),
            ("slliw", 31),
            ("srliw", 31),
            ("sraiw", 31),
            ("roriw", 31),
            ("sllw", 31),
            ("srlw", 31),
            ("sraw", 31),
            ("rorw", 31),
        ] {
            for count in [
                i64::MIN,
                -(1i64 << 32),
                -65,
                -1,
                max + 1,
                max + 2,
                1i64 << 32,
                i64::MAX,
            ] {
                let error = encode_instruction(mnemonic, &operands(count), "").expect_err(mnemonic);
                assert!(
                    error.contains("shift amount"),
                    "{mnemonic} {count}: {error}"
                );
            }
        }
    }

    #[test]
    fn register_count_shifts_are_not_immediate_range_checked() {
        let operands = [
            Operand::Reg("a0".into()),
            Operand::Reg("a1".into()),
            Operand::Reg("t0".into()),
        ];
        for mnemonic in ["sll", "srl", "sra", "ror", "sllw", "srlw", "sraw", "rorw"] {
            assert!(matches!(
                encode_instruction(mnemonic, &operands, "").unwrap(),
                EncodeResult::Word(_)
            ));
        }
    }
}
