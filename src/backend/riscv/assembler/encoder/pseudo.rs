use super::base::check_branch_offset;
use super::*;

// ── Pseudo-instruction encoders ──────────────────────────────────────

/// `li rd, imm`, expanded exactly as GNU as does, so that hand-written
/// assembly gets the sizes it was written against.  A value in 12 bits is
/// `addi rd, x0, imm` (the opcode table's `li d,j`); a larger one is
/// `load_const` (gas/config/tc-riscv.c), after `normalize_constant_expr`,
/// which on RV32 reads a value that fits 32 bits unsigned as signed.  The
/// caller compresses each word as GNU as's `md_assemblef` does: by the
/// word's own mnemonic.
pub(crate) fn encode_li(operands: &[Operand], rv64: bool) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let mut imm = get_imm(operands, 1)?;
    if !rv64 && (0..=0xFFFF_FFFF).contains(&imm) {
        imm = i64::from(imm as u32 as i32);
    }
    let mut words = Vec::new();
    if (-2048..=2047).contains(&imm) {
        words.push(encode_i(OP_OP_IMM, rd, 0, 0, imm as i32));
    } else {
        load_const(rd, imm, rv64, &mut words);
    }
    Ok(if words.len() == 1 {
        EncodeResult::Word(words[0])
    } else {
        EncodeResult::Words(words)
    })
}

/// Whether `li`'s own RVC aliases (`li d,Co`, `li d,Cv`) can match.  GNU as
/// matches them against the operand as written; only its later `li d,j`
/// entry, which has no 2-byte form, reads an RV32 value in 2^31..2^32 as
/// signed.  So RV32 `li a0, 0xFFFFFFFF` is a 4-byte `addi a0, zero, -1`
/// while `li a0, -1` is `c.li` (a `lui` or longer expansion compresses
/// either way).
pub(crate) fn li_rvc_aliases_match(operands: &[Operand], rv64: bool) -> bool {
    rv64 || !get_imm(operands, 1).is_ok_and(|imm| (0x8000_0000..=0xFFFF_FFFF).contains(&imm))
}

/// GNU as's `load_const`: a value that is not a sign-extended 32-bit one
/// (RV64 only) is its upper part shifted right past its trailing zeros,
/// loaded recursively, then `slli` and `addi` of the low 12 bits; otherwise
/// `lui` of the upper 20 bits and/or `addiw` (RV32: `addi`) of the low 12.
/// On RV32 a wider value keeps its low 32 bits, as there.
fn load_const(rd: u32, value: i64, rv64: bool, words: &mut Vec<u32>) {
    let lower = ((value & 0xFFF) ^ 0x800) - 0x800;
    let upper = value.wrapping_sub(lower);
    if rv64 && value != i64::from(value as i32) {
        // `upper` is nonzero here: `lower` is a sign-extended 12-bit value.
        let mut shift = 12;
        while (upper >> shift) & 1 == 0 {
            shift += 1;
        }
        load_const(rd, upper >> shift, rv64, words);
        words.push(encode_i(OP_OP_IMM, rd, 0b001, rd, shift)); // slli
        if lower != 0 {
            words.push(encode_i(OP_OP_IMM, rd, 0, rd, lower as i32)); // addi
        }
    } else {
        let mut hi_reg = 0;
        if upper != 0 {
            words.push(encode_u(OP_LUI, rd, upper as u32)); // lui (bits 31:12)
            hi_reg = rd;
        }
        if lower != 0 || hi_reg == 0 {
            let opcode = if rv64 { OP_OP_IMM_32 } else { OP_OP_IMM };
            words.push(encode_i(opcode, rd, 0, hi_reg, lower as i32)); // addiw / addi
        }
    }
}

pub(crate) fn encode_mv(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs = get_reg(operands, 1)?;
    // The ISA's definition of MV and GAS 2.47's encoding: `addi rd, rs, 0`
    // (00028513 for `mv a0, t0`). Compression is unaffected: GAS (and
    // `compress.rs`) turn this form into C.MV exactly like `add rd, x0, rs`.
    Ok(EncodeResult::Word(encode_i(OP_OP_IMM, rd, 0b000, rs, 0))) // addi rd, rs, 0
}

pub(crate) fn encode_not(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(OP_OP_IMM, rd, 0b100, rs1, -1))) // xori rd, rs1, -1
}

pub(crate) fn encode_neg(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP, rd, 0b000, 0, rs2, 0b0100000,
    ))) // sub rd, x0, rs2
}

pub(crate) fn encode_negw(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_32, rd, 0b000, 0, rs2, 0b0100000,
    )))
}

pub(crate) fn encode_sext_w(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(OP_OP_IMM_32, rd, 0, rs1, 0))) // addiw rd, rs1, 0
}

pub(crate) fn encode_seqz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(OP_OP_IMM, rd, 0b011, rs1, 1))) // sltiu rd, rs1, 1
}

pub(crate) fn encode_snez(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP, rd, 0b011, 0, rs2, 0b0000000,
    ))) // sltu rd, x0, rs2
}

pub(crate) fn encode_sltz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP, rd, 0b010, rs1, 0, 0b0000000,
    ))) // slt rd, rs1, x0
}

pub(crate) fn encode_sgtz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP, rd, 0b010, 0, rs2, 0b0000000,
    ))) // slt rd, x0, rs2
}

// Branch pseudo-instructions
pub(crate) fn encode_beqz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let label = get_branch_target(operands, 1)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b000, rs1, 0, 0),
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bnez(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let label = get_branch_target(operands, 1)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b001, rs1, 0, 0),
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_blez(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs2 = get_reg(operands, 0)?;
    let label = get_branch_target(operands, 1)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b101, 0, rs2, 0), // bge x0, rs
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bgez(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let label = get_branch_target(operands, 1)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b101, rs1, 0, 0), // bge rs, x0
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bltz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let label = get_branch_target(operands, 1)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b100, rs1, 0, 0), // blt rs, x0
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bgtz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs2 = get_reg(operands, 0)?;
    let label = get_branch_target(operands, 1)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b100, 0, rs2, 0), // blt x0, rs
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bgt(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    let label = get_branch_target(operands, 2)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b100, rs2, rs1, 0), // blt rs2, rs1
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_ble(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    let label = get_branch_target(operands, 2)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b101, rs2, rs1, 0), // bge rs2, rs1
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bgtu(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    let label = get_branch_target(operands, 2)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b110, rs2, rs1, 0), // bltu rs2, rs1
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bleu(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    let label = get_branch_target(operands, 2)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b111, rs2, rs1, 0), // bgeu rs2, rs1
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn get_branch_target(operands: &[Operand], idx: usize) -> Result<String, String> {
    match operands.get(idx) {
        Some(Operand::Symbol(s)) | Some(Operand::Label(s)) => Ok(s.clone()),
        Some(Operand::Imm(v)) => Ok(format!("{}", v)),
        // A register name can also be a symbol/label name (e.g. `beqz a0, t1`
        // where t1 is a label). Treat Reg as symbol in branch target context.
        Some(Operand::Reg(s)) => Ok(s.clone()),
        _ => Err(format!("expected branch target at operand {}", idx)),
    }
}

pub(crate) fn encode_j_pseudo(operands: &[Operand]) -> Result<EncodeResult, String> {
    // j offset -> jal x0, offset
    //
    // `j` is the ordinary spelling of `jal x0, offset`, so a literal target
    // has to be validated exactly as `encode_jal` validates one. It was not:
    // `j 17` encoded to the same word as `j 16`, silently branching to the
    // wrong address, because the J-type immediate has no bit-0 field.
    //
    // The length check also removes an index-out-of-bounds panic: the previous
    // code indexed `operands[0]` without checking that any operand existed.
    if operands.len() != 1 {
        return Err(format!(
            "j: expected 1 operand (offset or label), got {}",
            operands.len()
        ));
    }
    match &operands[0] {
        Operand::Symbol(s) | Operand::Label(s) | Operand::Reg(s) => {
            // Symbolic targets stay relocations so the linker can relax them;
            // the offset is not knowable at assembly time.
            Ok(EncodeResult::WordWithReloc {
                word: encode_j(OP_JAL, 0, 0),
                reloc: Relocation {
                    reloc_type: RelocType::Jal,
                    symbol: s.clone(),
                    addend: 0,
                },
            })
        }
        Operand::Imm(imm) => {
            check_branch_offset(*imm, -(1 << 20), 1 << 20, "j")?;
            Ok(EncodeResult::Word(encode_j(OP_JAL, 0, *imm as i32)))
        }
        _ => Err("j: expected offset or label".to_string()),
    }
}

pub(crate) fn encode_jr(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    Ok(EncodeResult::Word(encode_i(OP_JALR, 0, 0, rs1, 0)))
}

/// `call foo@plt` / `tail foo@plt`: the legacy spelling GAS 2.47 still
/// accepts (lowercase, no addend) and drops -- the relocation is
/// R_RISCV_CALL_PLT against `foo` either way. Kept in the name, `@plt` would
/// read as symbol `foo` with a symbol version called "plt".
fn call_target(operands: &[Operand]) -> Result<(String, i64), String> {
    let (symbol, addend) = get_symbol(operands, 0)?;
    match symbol.strip_suffix("@plt") {
        Some(base) if addend == 0 && !base.is_empty() => Ok((base.to_string(), 0)),
        Some(_) => Err(format!("illegal operands `{symbol}`")),
        None => Ok((symbol, addend)),
    }
}

pub(crate) fn encode_call(operands: &[Operand]) -> Result<EncodeResult, String> {
    // call symbol -> auipc ra, %pcrel_hi(symbol) ; jalr ra, %pcrel_lo(symbol)(ra)
    let (symbol, addend) = call_target(operands)?;
    Ok(EncodeResult::WordsWithRelocs(vec![
        (
            encode_u(OP_AUIPC, 1, 0),
            Some(Relocation {
                reloc_type: RelocType::CallPlt,
                symbol: symbol.clone(),
                addend,
            }),
        ),
        (encode_i(OP_JALR, 1, 0, 1, 0), None), // jalr ra, 0(ra)
    ]))
}

pub(crate) fn encode_tail(operands: &[Operand]) -> Result<EncodeResult, String> {
    // tail symbol -> auipc t1, %pcrel_hi(symbol) ; jalr x0, %pcrel_lo(symbol)(t1)
    let (symbol, addend) = call_target(operands)?;
    Ok(EncodeResult::WordsWithRelocs(vec![
        (
            encode_u(OP_AUIPC, 6, 0),
            Some(Relocation {
                // t1 = x6
                reloc_type: RelocType::CallPlt,
                symbol: symbol.clone(),
                addend,
            }),
        ),
        (encode_i(OP_JALR, 0, 0, 6, 0), None),
    ]))
}

pub(crate) fn encode_jump(operands: &[Operand]) -> Result<EncodeResult, String> {
    // jump label, temp_reg -> auipc temp, %pcrel_hi(label) ; jalr x0, %pcrel_lo(label)(temp)
    // Our codegen emits: jump .LBB42, t6
    let (symbol, addend) = get_symbol(operands, 0)?;
    let temp = if operands.len() > 1 {
        get_reg(operands, 1)?
    } else {
        31 // t6
    };

    Ok(EncodeResult::WordsWithRelocs(vec![
        (
            encode_u(OP_AUIPC, temp, 0),
            Some(Relocation {
                reloc_type: RelocType::CallPlt,
                symbol: symbol.clone(),
                addend,
            }),
        ),
        (encode_i(OP_JALR, 0, 0, temp, 0), None),
    ]))
}

pub(crate) fn encode_la(operands: &[Operand]) -> Result<EncodeResult, String> {
    // `la` outside `.option pic` is `lla` (GAS 2.47); under `.option pic` the
    // ELF writer expands it as `lga` (see `encode_got_pseudo`).
    encode_lla(operands)
}

/// The GOT-slot address pseudos, which need XLEN (`ld` vs `lw`) and so are
/// expanded by the ELF writer rather than `encode_instruction`:
///
/// * `lga rd, sym` (and `la` under `.option pic`): `auipc rd, %got_pcrel_hi`
///   + `l[dw] rd, %pcrel_lo(rd)` — the symbol's address from its GOT slot.
/// * `la.tls.ie rd, sym`: the same with R_RISCV_TLS_GOT_HI20 — the TP offset.
/// * `la.tls.gd rd, sym`: `auipc` R_RISCV_TLS_GD_HI20 + `addi` — the address
///   of the GD GOT pair, `__tls_get_addr`'s argument.
///
/// The `%pcrel_lo` relocation is re-targeted at the `auipc`'s synthetic
/// label by the writer, exactly as for `lla`. Byte-identical to GAS 2.47.
pub(crate) fn encode_got_pseudo(
    mnemonic: &str,
    operands: &[Operand],
    xlen64: bool,
) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let (symbol, addend) = get_symbol(operands, 1)?;
    let (hi, second) = match mnemonic {
        "lga" | "la" | "la.tls.ie" => {
            let hi = if mnemonic == "la.tls.ie" {
                RelocType::TlsGotHi20
            } else {
                RelocType::GotHi20
            };
            let funct3 = if xlen64 { 0b011 } else { 0b010 }; // ld / lw
            (hi, encode_i(OP_LOAD, rd, funct3, rd, 0))
        }
        "la.tls.gd" => (RelocType::TlsGdHi20, encode_i(OP_OP_IMM, rd, 0, rd, 0)),
        _ => return Err(format!("not a GOT pseudo: {mnemonic}")),
    };
    Ok(EncodeResult::WordsWithRelocs(vec![
        (
            encode_u(OP_AUIPC, rd, 0),
            Some(Relocation {
                reloc_type: hi,
                symbol: symbol.clone(),
                addend,
            }),
        ),
        (
            second,
            Some(Relocation {
                reloc_type: RelocType::PcrelLo12I,
                symbol,
                addend,
            }),
        ),
    ]))
}

pub(crate) fn encode_lla(operands: &[Operand]) -> Result<EncodeResult, String> {
    // lla rd, symbol -> auipc rd, %pcrel_hi(symbol) ; addi rd, rd, %pcrel_lo(symbol)
    let rd = get_reg(operands, 0)?;
    let (symbol, addend) = get_symbol(operands, 1)?;

    Ok(EncodeResult::WordsWithRelocs(vec![
        (
            encode_u(OP_AUIPC, rd, 0),
            Some(Relocation {
                reloc_type: RelocType::PcrelHi20,
                symbol: symbol.clone(),
                addend,
            }),
        ),
        (
            encode_i(OP_OP_IMM, rd, 0, rd, 0),
            Some(Relocation {
                reloc_type: RelocType::PcrelLo12I,
                symbol, // TODO: This should reference the auipc label, not the symbol directly
                addend,
            }),
        ),
    ]))
}

pub(crate) fn encode_rdcsr(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let csr = match mnemonic {
        "rdcycle" => 0xC00,
        "rdtime" => 0xC01,
        "rdinstret" => 0xC02,
        _ => return Err(format!("unknown CSR pseudo: {}", mnemonic)),
    };
    Ok(EncodeResult::Word(encode_i(OP_SYSTEM, rd, 0b010, 0, csr))) // csrrs rd, csr, x0
}

pub(crate) fn encode_csrr(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let csr = get_csr_num(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_SYSTEM, rd, 0b010, 0, csr as i32,
    )))
}

pub(crate) fn encode_csrw(operands: &[Operand]) -> Result<EncodeResult, String> {
    let csr = get_csr_num(operands, 0)?;
    if matches!(operands.get(1), Some(Operand::Imm(_))) {
        let zimm = get_imm(operands, 1)? as u32 & 0x1F;
        return Ok(EncodeResult::Word(encode_i(
            OP_SYSTEM, 0, 0b101, zimm, csr as i32,
        )));
    }
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_SYSTEM, 0, 0b001, rs1, csr as i32,
    )))
}

pub(crate) fn encode_csrs(operands: &[Operand]) -> Result<EncodeResult, String> {
    let csr = get_csr_num(operands, 0)?;
    if matches!(operands.get(1), Some(Operand::Imm(_))) {
        let zimm = get_imm(operands, 1)? as u32 & 0x1F;
        return Ok(EncodeResult::Word(encode_i(
            OP_SYSTEM, 0, 0b110, zimm, csr as i32,
        )));
    }
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_SYSTEM, 0, 0b010, rs1, csr as i32,
    )))
}

pub(crate) fn encode_csrc(operands: &[Operand]) -> Result<EncodeResult, String> {
    let csr = get_csr_num(operands, 0)?;
    if matches!(operands.get(1), Some(Operand::Imm(_))) {
        let zimm = get_imm(operands, 1)? as u32 & 0x1F;
        return Ok(EncodeResult::Word(encode_i(
            OP_SYSTEM, 0, 0b111, zimm, csr as i32,
        )));
    }
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_SYSTEM, 0, 0b011, rs1, csr as i32,
    )))
}

// Float pseudo-instructions
pub(crate) fn encode_fmv_s(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    // fsgnj.s rd, rs, rs
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b000, rs1, rs1, 0b0010000,
    )))
}

pub(crate) fn encode_fmv_d(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b000, rs1, rs1, 0b0010001,
    )))
}

pub(crate) fn encode_fabs_s(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    // fsgnjx.s rd, rs, rs
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b010, rs1, rs1, 0b0010000,
    )))
}

pub(crate) fn encode_fabs_d(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b010, rs1, rs1, 0b0010001,
    )))
}

pub(crate) fn encode_fneg_s(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    // fsgnjn.s rd, rs, rs
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b001, rs1, rs1, 0b0010000,
    )))
}

pub(crate) fn encode_fneg_d(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b001, rs1, rs1, 0b0010001,
    )))
}

// ── Relocation modifier parsing ──────────────────────────────────────

/// Extract symbol name from %modifier(symbol) expressions
pub(crate) fn extract_modifier_symbol(s: &str) -> String {
    if let Some(start) = s.find('(') {
        if let Some(end) = s.rfind(')') {
            return s[start + 1..end].to_string();
        }
    }
    s.to_string()
}

/// The low-12 relocation an I-type instruction (load, `addi`, `jalr`) carries
/// for a parsed modifier (`%lo`, `%pcrel_lo`, `%tprel_lo`, or a hi form).
pub(crate) fn lo12_itype(r: RelocType) -> RelocType {
    match r {
        RelocType::PcrelHi20 => RelocType::PcrelLo12I,
        RelocType::Hi20 => RelocType::Lo12I,
        RelocType::TprelHi20 => RelocType::TprelLo12I,
        other => other,
    }
}

/// The S-type twin of [`lo12_itype`], for stores. A store splits its
/// immediate (imm[11:5] in bits 31:25, imm[4:0] in bits 11:7); an I-type
/// fixup writes bits 31:20 instead, i.e. it overwrites the store's `rs2`
/// field with address bits once the symbol's low 12 bits are non-zero.
/// `parse_reloc_modifier` yields the I-type kinds for the `_lo` modifiers,
/// so they must be remapped here (GAS 2.47: `sw a1, %lo(g)(a0)` is
/// R_RISCV_LO12_S, `%tprel_lo` R_RISCV_TPREL_LO12_S, `%pcrel_lo`
/// R_RISCV_PCREL_LO12_S).
pub(crate) fn lo12_stype(r: RelocType) -> RelocType {
    match r {
        RelocType::PcrelHi20 | RelocType::PcrelLo12I => RelocType::PcrelLo12S,
        RelocType::Hi20 | RelocType::Lo12I => RelocType::Lo12S,
        RelocType::TprelHi20 | RelocType::TprelLo12I => RelocType::TprelLo12S,
        other => other,
    }
}

/// Parse a relocation modifier like %pcrel_hi(symbol) and return (RelocType, symbol)
pub(crate) fn parse_reloc_modifier(s: &str) -> (RelocType, String) {
    if s.starts_with("%pcrel_hi(") {
        (RelocType::PcrelHi20, extract_modifier_symbol(s))
    } else if s.starts_with("%pcrel_lo(") {
        (RelocType::PcrelLo12I, extract_modifier_symbol(s))
    } else if s.starts_with("%hi(") {
        (RelocType::Hi20, extract_modifier_symbol(s))
    } else if s.starts_with("%lo(") {
        (RelocType::Lo12I, extract_modifier_symbol(s))
    } else if s.starts_with("%tprel_hi(") {
        (RelocType::TprelHi20, extract_modifier_symbol(s))
    } else if s.starts_with("%tprel_lo(") {
        (RelocType::TprelLo12I, extract_modifier_symbol(s))
    } else if s.starts_with("%tprel_add(") {
        (RelocType::TprelAdd, extract_modifier_symbol(s))
    } else if s.starts_with("%got_pcrel_hi(") {
        (RelocType::GotHi20, extract_modifier_symbol(s))
    } else if s.starts_with("%tls_ie_pcrel_hi(") {
        (RelocType::TlsGotHi20, extract_modifier_symbol(s))
    } else if s.starts_with("%tls_gd_pcrel_hi(") {
        (RelocType::TlsGdHi20, extract_modifier_symbol(s))
    } else {
        // Plain symbol - use as PC-relative
        (RelocType::PcrelHi20, s.to_string())
    }
}
