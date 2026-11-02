//! RVC (C extension) compressed instruction support, RV64 and RV32.
//!
//! Implements the RISC-V C (compressed) extension for RV64. After the assembler
//! encodes all instructions as 32-bit words, this module attempts to compress
//! eligible instructions into 16-bit (2-byte) equivalents, matching GCC's
//! default behavior for RV64GC targets.
//!
//! The compression pass runs after instruction encoding but before local branch
//! resolution, so that branch offsets are computed against the final (compressed)
//! layout.

/// Try to compress a 32-bit RISC-V instruction into a 16-bit RV64C instruction.
///
/// Returns `Some(halfword)` if the instruction can be compressed, `None` otherwise.
/// This only handles instructions without relocations; instructions with pending
/// relocations are not candidates for compression.
pub fn try_compress_rv64(word: u32) -> Option<u16> {
    compress_rv64_as_written(word).or_else(|| commuted(word).and_then(compress_rv64_as_written))
}

/// GNU as also matches the commuted operand order of the commutative
/// register-register ops when the order as written has no 2-byte form:
/// `add a0, a1, a0` is `c.add a0, a1` (and, or, xor, addw alike), while
/// `add s0, x0, s0` stays `c.mv s0, s0`.
fn commuted(word: u32) -> Option<u32> {
    let (opcode, funct3, funct7) = (word & 0x7F, (word >> 12) & 0x7, word >> 25);
    let commutative = funct7 == 0
        && match opcode {
            0b0110011 => matches!(funct3, 0b000 | 0b100 | 0b110 | 0b111),
            0b0111011 => funct3 == 0b000,
            _ => false,
        };
    let (rd, rs1, rs2) = ((word >> 7) & 0x1F, (word >> 15) & 0x1F, (word >> 20) & 0x1F);
    (commutative && rd == rs2 && rs1 != rs2)
        .then(|| (word & !((0x1F << 15) | (0x1F << 20))) | (rs2 << 15) | (rs1 << 20))
}

// Binary literals use groupings matching RISC-V compressed instruction format fields.
#[expect(clippy::unusual_byte_groupings)]
fn compress_rv64_as_written(word: u32) -> Option<u16> {
    let opcode = word & 0x7F;
    let rd = (word >> 7) & 0x1F;
    let funct3 = (word >> 12) & 0x7;
    let rs1 = (word >> 15) & 0x1F;
    let rs2 = (word >> 20) & 0x1F;
    let funct7 = (word >> 25) & 0x7F;

    match opcode {
        // ── LUI (opcode 0110111) ──
        0b0110111 => {
            // C.LUI: lui rd, imm  where rd != {x0, x2}
            // imm[17:12] from word[31:12]
            if rd == 0 || rd == 2 {
                return None;
            }
            let imm20 = (word >> 12) as i32;
            // Sign-extend the 20-bit value
            let imm20 = (imm20 << 12) >> 12; // sign-extend from 20 bits
            // C.LUI uses imm[17:12], so the actual nzimm is bits 17:12
            // which is imm20[5:0] (since LUI loads imm into [31:12])
            // C.LUI constraint: nzimm != 0, and it's sign-extended from 6 bits
            let nzimm = imm20; // this is the full 20-bit value
            // C.LUI stores bits [17:12] = nzimm[5:0], sign-extended from bit 17
            // So nzimm must fit in signed 6-bit range: -32..31 (but not 0)
            if nzimm == 0 {
                return None;
            }
            if !(-32..=31).contains(&nzimm) {
                return None;
            }
            let nzimm = nzimm as u32;
            let bit17 = (nzimm >> 5) & 1;
            let bits16_12 = nzimm & 0x1F;
            Some((0b011_0_00000_00000_01 | (bit17 << 12) | (rd << 7) | (bits16_12 << 2)) as u16)
        }

        // ── ADDI (opcode 0010011, funct3=000) ──
        0b0010011 if funct3 == 0b000 => {
            let imm = (word as i32) >> 20; // sign-extended imm[11:0]

            if rd == 0 && rs1 == 0 && imm == 0 {
                // C.NOP
                Some(0b000_0_00000_00000_01)
            } else if rd == rs1 && rd != 0 && imm != 0 && (-32..=31).contains(&imm) {
                // C.ADDI: addi rd, rd, nzimm (including sp)
                // GCC prefers C.ADDI over C.ADDI16SP when both apply
                let nzimm = imm as u32;
                let bit5 = (nzimm >> 5) & 1;
                let bits4_0 = nzimm & 0x1F;
                Some((0b000_0_00000_00000_01 | (bit5 << 12) | (rd << 7) | (bits4_0 << 2)) as u16)
            } else if rd == 2
                && rs1 == 2
                && imm != 0
                && (imm % 16) == 0
                && (-512..=496).contains(&imm)
            {
                // C.ADDI16SP: addi x2, x2, imm (for larger imm not fitting C.ADDI)
                let uimm = imm as u32;
                let bit9 = (uimm >> 9) & 1;
                let bit4 = (uimm >> 4) & 1;
                let bit6 = (uimm >> 6) & 1;
                let bits8_7 = (uimm >> 7) & 0x3;
                let bit5 = (uimm >> 5) & 1;
                Some(
                    (0b011_0_00010_00000_01
                        | (bit9 << 12)
                        | (bit4 << 6)
                        | (bit6 << 5)
                        | (bits8_7 << 3)
                        | (bit5 << 2)) as u16,
                )
            } else if rs1 == 2
                && rd != 0
                && rd != 2
                && is_creg(rd)
                && imm > 0
                && (imm % 4) == 0
                && imm <= 1020
            {
                // C.ADDI4SPN: addi rd', x2, uimm
                let uimm = imm as u32;
                let rd_prime = creg_num(rd);
                let bits5_4 = (uimm >> 4) & 0x3;
                let bits9_6 = (uimm >> 6) & 0xF;
                let bit2 = (uimm >> 2) & 1;
                let bit3 = (uimm >> 3) & 1;
                Some(
                    ((bits5_4 << 11) | (bits9_6 << 7) | (bit2 << 6) | (bit3 << 5) | (rd_prime << 2))
                        as u16,
                )
            } else if rs1 != 0 && rd != 0 && imm == 0 {
                // C.MV: `mv rd, rs` is `addi rd, rs, 0`; GAS compresses it to
                // the same C.MV as `add rd, x0, rs` (both copy rs to rd).
                Some((0b100_0_00000_00000_10 | (rd << 7) | (rs1 << 2)) as u16)
            } else if rs1 == 0 && rd != 0 {
                // C.LI: addi rd, x0, imm  (li rd, imm)
                if !(-32..=31).contains(&imm) {
                    return None;
                }
                let imm_u = imm as u32;
                let bit5 = (imm_u >> 5) & 1;
                let bits4_0 = imm_u & 0x1F;
                Some((0b010_0_00000_00000_01 | (bit5 << 12) | (rd << 7) | (bits4_0 << 2)) as u16)
            } else {
                None
            }
        }

        // ── ADDIW (opcode 0011011, funct3=000) ──
        0b0011011 if funct3 == 0b000 => {
            let imm = (word as i32) >> 20;
            if rd == rs1 && rd != 0 {
                // C.ADDIW: addiw rd, rd, imm  (imm can be 0 for sext.w)
                if !(-32..=31).contains(&imm) {
                    return None;
                }
                let imm_u = imm as u32;
                let bit5 = (imm_u >> 5) & 1;
                let bits4_0 = imm_u & 0x1F;
                Some((0b001_0_00000_00000_01 | (bit5 << 12) | (rd << 7) | (bits4_0 << 2)) as u16)
            } else if rs1 == 0 && rd != 0 {
                // addiw rd, x0, imm => also C.ADDIW if we treat rd as rd/rs1
                // Actually this would be `sext.w rd` which is `addiw rd, rd, 0`
                // This pattern doesn't match C.ADDIW since rd != rs1
                None
            } else {
                None
            }
        }

        // ── SLLI (opcode 0010011, funct3=001) ──
        0b0010011 if funct3 == 0b001 => {
            // slli rd, rs1, shamt  (funct7 encodes high bit of shamt for RV64)
            let shamt = (word >> 20) & 0x3F; // 6-bit shift for RV64
            if rd == rs1 && rd != 0 && shamt != 0 {
                // C.SLLI: slli rd, rd, shamt
                let bit5 = (shamt >> 5) & 1;
                let bits4_0 = shamt & 0x1F;
                Some((0b000_0_00000_00000_10 | (bit5 << 12) | (rd << 7) | (bits4_0 << 2)) as u16)
            } else {
                None
            }
        }

        // ── SRLI/SRAI (opcode 0010011, funct3=101) ──
        0b0010011 if funct3 == 0b101 => {
            let shamt = (word >> 20) & 0x3F;
            let is_srai = (funct7 & 0x20) != 0;
            if rd == rs1 && is_creg(rd) && shamt != 0 {
                let rd_prime = creg_num(rd);
                let bit5 = (shamt >> 5) & 1;
                let bits4_0 = shamt & 0x1F;
                if is_srai {
                    // C.SRAI
                    Some(
                        (0b100_0_01_000_00000_01 | (bit5 << 12) | (rd_prime << 7) | (bits4_0 << 2))
                            as u16,
                    )
                } else {
                    // C.SRLI
                    Some(
                        (0b100_0_00_000_00000_01 | (bit5 << 12) | (rd_prime << 7) | (bits4_0 << 2))
                            as u16,
                    )
                }
            } else {
                None
            }
        }

        // ── ANDI (opcode 0010011, funct3=111) ──
        0b0010011 if funct3 == 0b111 => {
            let imm = (word as i32) >> 20;
            if rd == rs1 && is_creg(rd) {
                if !(-32..=31).contains(&imm) {
                    return None;
                }
                let rd_prime = creg_num(rd);
                let imm_u = imm as u32;
                let bit5 = (imm_u >> 5) & 1;
                let bits4_0 = imm_u & 0x1F;
                // C.ANDI
                Some(
                    (0b100_0_10_000_00000_01 | (bit5 << 12) | (rd_prime << 7) | (bits4_0 << 2))
                        as u16,
                )
            } else {
                None
            }
        }

        // ── ADD/SUB/AND/OR/XOR/ADDW/SUBW (opcode 0110011, R-type) ──
        0b0110011 => {
            match (funct7, funct3) {
                // ADD
                (0b0000000, 0b000) => {
                    if rd == rs1 && rd != 0 && rs2 != 0 {
                        // C.ADD: add rd, rd, rs2
                        Some((0b100_1_00000_00000_10 | (rd << 7) | (rs2 << 2)) as u16)
                    } else if rs1 == 0 && rd != 0 && rs2 != 0 {
                        // C.MV: add rd, x0, rs2  (mv rd, rs2)
                        Some((0b100_0_00000_00000_10 | (rd << 7) | (rs2 << 2)) as u16)
                    } else {
                        None
                    }
                }
                // SUB
                (0b0100000, 0b000) => {
                    if rd == rs1 && is_creg(rd) && is_creg(rs2) {
                        let rd_prime = creg_num(rd);
                        let rs2_prime = creg_num(rs2);
                        // C.SUB
                        Some((0b100_0_11_000_00_000_01 | (rd_prime << 7) | (rs2_prime << 2)) as u16)
                    } else {
                        None
                    }
                }
                // XOR
                (0b0000000, 0b100) => {
                    if rd == rs1 && is_creg(rd) && is_creg(rs2) {
                        let rd_prime = creg_num(rd);
                        let rs2_prime = creg_num(rs2);
                        // C.XOR
                        Some((0b100_0_11_000_01_000_01 | (rd_prime << 7) | (rs2_prime << 2)) as u16)
                    } else {
                        None
                    }
                }
                // OR
                (0b0000000, 0b110) => {
                    if rd == rs1 && is_creg(rd) && is_creg(rs2) {
                        let rd_prime = creg_num(rd);
                        let rs2_prime = creg_num(rs2);
                        // C.OR
                        Some((0b100_0_11_000_10_000_01 | (rd_prime << 7) | (rs2_prime << 2)) as u16)
                    } else {
                        None
                    }
                }
                // AND
                (0b0000000, 0b111) => {
                    if rd == rs1 && is_creg(rd) && is_creg(rs2) {
                        let rd_prime = creg_num(rd);
                        let rs2_prime = creg_num(rs2);
                        // C.AND
                        Some((0b100_0_11_000_11_000_01 | (rd_prime << 7) | (rs2_prime << 2)) as u16)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }

        // ── ADDW/SUBW (opcode 0111011, R-type word ops) ──
        0b0111011 => {
            match (funct7, funct3) {
                // ADDW
                (0b0000000, 0b000) => {
                    if rd == rs1 && is_creg(rd) && is_creg(rs2) {
                        let rd_prime = creg_num(rd);
                        let rs2_prime = creg_num(rs2);
                        // C.ADDW
                        Some((0b100_1_11_000_01_000_01 | (rd_prime << 7) | (rs2_prime << 2)) as u16)
                    } else {
                        None
                    }
                }
                // SUBW
                (0b0100000, 0b000) => {
                    if rd == rs1 && is_creg(rd) && is_creg(rs2) {
                        let rd_prime = creg_num(rd);
                        let rs2_prime = creg_num(rs2);
                        // C.SUBW
                        Some((0b100_1_11_000_00_000_01 | (rd_prime << 7) | (rs2_prime << 2)) as u16)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }

        // ── LD (opcode 0000011, funct3=011) ──
        0b0000011 if funct3 == 0b011 => {
            let imm = (word as i32) >> 20;
            if rs1 == 2 && rd != 0 {
                // C.LDSP: ld rd, offset(sp) - offset must be multiple of 8, 0..504
                let offset = imm;
                if (0..=504).contains(&offset) && (offset % 8) == 0 {
                    let uoff = offset as u32;
                    // Encoding: 011 | uimm[5] | rd | uimm[4:3|8:6] | 10
                    let bit5 = (uoff >> 5) & 1;
                    let bits4_3 = (uoff >> 3) & 0x3;
                    let bits8_6 = (uoff >> 6) & 0x7;
                    Some(
                        (0b011_0_00000_00000_10
                            | (bit5 << 12)
                            | (rd << 7)
                            | (bits4_3 << 5)
                            | (bits8_6 << 2)) as u16,
                    )
                } else {
                    None
                }
            } else if is_creg(rs1) && is_creg(rd) {
                // C.LD: ld rd', offset(rs1') - offset must be multiple of 8, 0..248
                let offset = imm;
                if (0..=248).contains(&offset) && (offset % 8) == 0 {
                    let uoff = offset as u32;
                    let rs1_prime = creg_num(rs1);
                    let rd_prime = creg_num(rd);
                    // Encoding: 011 | uimm[5:3] | rs1' | uimm[7:6] | rd' | 00
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bits7_6 = (uoff >> 6) & 0x3;
                    Some(
                        (0b011_000_000_00_000_00
                            | (bits5_3 << 10)
                            | (rs1_prime << 7)
                            | (bits7_6 << 5)
                            | (rd_prime << 2)) as u16,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── LW (opcode 0000011, funct3=010) ──
        0b0000011 if funct3 == 0b010 => {
            let imm = (word as i32) >> 20;
            if rs1 == 2 && rd != 0 {
                // C.LWSP: lw rd, offset(sp) - offset must be multiple of 4, 0..252
                let offset = imm;
                if (0..=252).contains(&offset) && (offset % 4) == 0 {
                    let uoff = offset as u32;
                    // Encoding: 010 | uimm[5] | rd | uimm[4:2|7:6] | 10
                    let bit5 = (uoff >> 5) & 1;
                    let bits4_2 = (uoff >> 2) & 0x7;
                    let bits7_6 = (uoff >> 6) & 0x3;
                    Some(
                        (0b010_0_00000_00000_10
                            | (bit5 << 12)
                            | (rd << 7)
                            | (bits4_2 << 4)
                            | (bits7_6 << 2)) as u16,
                    )
                } else {
                    None
                }
            } else if is_creg(rs1) && is_creg(rd) {
                // C.LW: lw rd', offset(rs1') - offset must be multiple of 4, 0..124
                let offset = imm;
                if (0..=124).contains(&offset) && (offset % 4) == 0 {
                    let uoff = offset as u32;
                    let rs1_prime = creg_num(rs1);
                    let rd_prime = creg_num(rd);
                    // Encoding: 010 | uimm[5:3] | rs1' | uimm[2|6] | rd' | 00
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bit2 = (uoff >> 2) & 1;
                    let bit6 = (uoff >> 6) & 1;
                    Some(
                        (0b010_000_000_00_000_00
                            | (bits5_3 << 10)
                            | (rs1_prime << 7)
                            | (bit2 << 6)
                            | (bit6 << 5)
                            | (rd_prime << 2)) as u16,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── SD (opcode 0100011, funct3=011) ──
        0b0100011 if funct3 == 0b011 => {
            // S-type: imm[11:5] = funct7, imm[4:0] = rd field
            let imm11_5 = (word >> 25) as i32;
            let imm4_0 = ((word >> 7) & 0x1F) as i32;
            let imm = (imm11_5 << 5) | imm4_0;
            let imm = (imm << 20) >> 20; // sign-extend from 12 bits

            if rs1 == 2 {
                // C.SDSP: sd rs2, offset(sp) - offset must be multiple of 8, 0..504
                if (0..=504).contains(&imm) && (imm % 8) == 0 {
                    let uoff = imm as u32;
                    // Encoding: 111 | uimm[5:3|8:6] | rs2 | 10
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bits8_6 = (uoff >> 6) & 0x7;
                    Some(
                        (0b111_000000_00000_10 | (bits5_3 << 10) | (bits8_6 << 7) | (rs2 << 2))
                            as u16,
                    )
                } else {
                    None
                }
            } else if is_creg(rs1) && is_creg(rs2) {
                // C.SD: sd rs2', offset(rs1') - offset must be multiple of 8, 0..248
                if (0..=248).contains(&imm) && (imm % 8) == 0 {
                    let uoff = imm as u32;
                    let rs1_prime = creg_num(rs1);
                    let rs2_prime = creg_num(rs2);
                    // Encoding: 111 | uimm[5:3] | rs1' | uimm[7:6] | rs2' | 00
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bits7_6 = (uoff >> 6) & 0x3;
                    Some(
                        (0b111_000_000_00_000_00
                            | (bits5_3 << 10)
                            | (rs1_prime << 7)
                            | (bits7_6 << 5)
                            | (rs2_prime << 2)) as u16,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── SW (opcode 0100011, funct3=010) ──
        0b0100011 if funct3 == 0b010 => {
            let imm11_5 = (word >> 25) as i32;
            let imm4_0 = ((word >> 7) & 0x1F) as i32;
            let imm = (imm11_5 << 5) | imm4_0;
            let imm = (imm << 20) >> 20;

            if rs1 == 2 {
                // C.SWSP: sw rs2, offset(sp) - offset must be multiple of 4, 0..252
                if (0..=252).contains(&imm) && (imm % 4) == 0 {
                    let uoff = imm as u32;
                    // Encoding: 110 | uimm[5:2|7:6] | rs2 | 10
                    let bits5_2 = (uoff >> 2) & 0xF;
                    let bits7_6 = (uoff >> 6) & 0x3;
                    Some(
                        (0b110_000000_00000_10 | (bits5_2 << 9) | (bits7_6 << 7) | (rs2 << 2))
                            as u16,
                    )
                } else {
                    None
                }
            } else if is_creg(rs1) && is_creg(rs2) {
                // C.SW: sw rs2', offset(rs1') - offset must be multiple of 4, 0..124
                if (0..=124).contains(&imm) && (imm % 4) == 0 {
                    let uoff = imm as u32;
                    let rs1_prime = creg_num(rs1);
                    let rs2_prime = creg_num(rs2);
                    // Encoding: 110 | uimm[5:3] | rs1' | uimm[2|6] | rs2' | 00
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bit2 = (uoff >> 2) & 1;
                    let bit6 = (uoff >> 6) & 1;
                    Some(
                        (0b110_000_000_00_000_00
                            | (bits5_3 << 10)
                            | (rs1_prime << 7)
                            | (bit2 << 6)
                            | (bit6 << 5)
                            | (rs2_prime << 2)) as u16,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── FLD (opcode 0000111, funct3=011) ──
        0b0000111 if funct3 == 0b011 => {
            let imm = (word as i32) >> 20;
            if rs1 == 2 {
                // C.FLDSP: fld rd, offset(sp) - offset must be multiple of 8, 0..504
                let offset = imm;
                if (0..=504).contains(&offset) && (offset % 8) == 0 {
                    let uoff = offset as u32;
                    let bit5 = (uoff >> 5) & 1;
                    let bits4_3 = (uoff >> 3) & 0x3;
                    let bits8_6 = (uoff >> 6) & 0x7;
                    Some(
                        (0b001_0_00000_00000_10
                            | (bit5 << 12)
                            | (rd << 7)
                            | (bits4_3 << 5)
                            | (bits8_6 << 2)) as u16,
                    )
                } else {
                    None
                }
            } else if is_creg(rs1) && is_creg(rd) {
                // C.FLD: fld rd', offset(rs1') - offset must be multiple of 8, 0..248
                let offset = imm;
                if (0..=248).contains(&offset) && (offset % 8) == 0 {
                    let uoff = offset as u32;
                    let rs1_prime = creg_num(rs1);
                    let rd_prime = creg_num(rd);
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bits7_6 = (uoff >> 6) & 0x3;
                    Some(
                        (0b001_000_000_00_000_00
                            | (bits5_3 << 10)
                            | (rs1_prime << 7)
                            | (bits7_6 << 5)
                            | (rd_prime << 2)) as u16,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── FSD (opcode 0100111, funct3=011) ──
        0b0100111 if funct3 == 0b011 => {
            let imm11_5 = (word >> 25) as i32;
            let imm4_0 = ((word >> 7) & 0x1F) as i32;
            let imm = (imm11_5 << 5) | imm4_0;
            let imm = (imm << 20) >> 20;

            if rs1 == 2 {
                // C.FSDSP: fsd rs2, offset(sp) - offset must be multiple of 8, 0..504
                if (0..=504).contains(&imm) && (imm % 8) == 0 {
                    let uoff = imm as u32;
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bits8_6 = (uoff >> 6) & 0x7;
                    Some(
                        (0b101_000000_00000_10 | (bits5_3 << 10) | (bits8_6 << 7) | (rs2 << 2))
                            as u16,
                    )
                } else {
                    None
                }
            } else if is_creg(rs1) && is_creg(rs2) {
                // C.FSD: fsd rs2', offset(rs1') - offset must be multiple of 8, 0..248
                if (0..=248).contains(&imm) && (imm % 8) == 0 {
                    let uoff = imm as u32;
                    let rs1_prime = creg_num(rs1);
                    let rs2_prime = creg_num(rs2);
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bits7_6 = (uoff >> 6) & 0x3;
                    Some(
                        (0b101_000_000_00_000_00
                            | (bits5_3 << 10)
                            | (rs1_prime << 7)
                            | (bits7_6 << 5)
                            | (rs2_prime << 2)) as u16,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── JALR (opcode 1100111) ──
        0b1100111 => {
            let imm = (word as i32) >> 20;
            if imm == 0 && rs2 == 0 {
                if rd == 0 && rs1 != 0 {
                    // C.JR: jalr x0, 0(rs1)
                    Some((0b100_0_00000_00000_10 | (rs1 << 7)) as u16)
                } else if rd == 1 && rs1 != 0 {
                    // C.JALR: jalr x1, 0(rs1)
                    Some((0b100_1_00000_00000_10 | (rs1 << 7)) as u16)
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── JAL (opcode 1101111) ──
        0b1101111 => {
            // Symbolic `j`/`jal` are relaxed to c.j/c.jal by the writer
            // (`ElfWriter::short_branch`), which knows the distance.
            None
        }

        // ── BEQ/BNE (opcode 1100011) ──
        0b1100011 => {
            // Symbolic branches are relaxed to c.beqz/c.bnez by the writer
            // (`ElfWriter::short_branch`), which knows the distance.
            None
        }

        // ── EBREAK (opcode 1110011) ──
        0b1110011 => {
            if word == 0x00100073 {
                // C.EBREAK
                Some(0b100_1_00000_00000_10)
            } else {
                None
            }
        }

        _ => None,
    }
}

/// GNU as's compression of one relocation-free instruction for the object's
/// XLEN.  RV32 has no `addiw`/`addw`/`subw`/`ld`/`sd`: their RV64C slots are
/// `c.jal` and `c.flw`/`c.fsw`(`sp`) there, and a shift by 32 or more has no
/// compressed form (`shamt[5]` is reserved).  `flw`/`fsw` take the `lw`/`sw`
/// layouts with funct3 bit 13 set; unlike `c.lwsp`, `c.flwsp` may name f0.
pub fn try_compress(word: u32, rv64: bool) -> Option<u16> {
    if rv64 {
        return try_compress_rv64(word);
    }
    let funct3 = (word >> 12) & 0x7;
    match (word & 0x7F, funct3) {
        (0b0011011, _) | (0b0111011, _) | (0b0000011, 0b011) | (0b0100011, 0b011) => None,
        (0b0010011, 0b001 | 0b101) if (word >> 25) & 1 != 0 => None,
        (0b0000111, 0b010) => {
            let (rd, rs1) = ((word >> 7) & 0x1F, (word >> 15) & 0x1F);
            if rd == 0 && rs1 == 2 {
                // c.flwsp f0: try_compress_rv64 refuses `lw x0` (reserved).
                let as_lw = (word & !0x7F & !(0x1F << 7)) | 0b0000011 | (1 << 7);
                return try_compress_rv64(as_lw).map(|h| (h | 0x2000) & !(0x1F << 7));
            }
            try_compress_rv64((word & !0x7F) | 0b0000011).map(|h| h | 0x2000)
        }
        (0b0100111, 0b010) => try_compress_rv64((word & !0x7F) | 0b0100011).map(|h| h | 0x2000),
        _ => try_compress_rv64(word),
    }
}

/// Check if a register number is in the "compressed" range (x8-x15).
/// These are the registers that can be encoded in 3-bit fields.
#[inline]
fn is_creg(reg: u32) -> bool {
    (8..=15).contains(&reg)
}

/// Convert a full register number (x8-x15) to its 3-bit compressed encoding (0-7).
#[inline]
fn creg_num(reg: u32) -> u32 {
    debug_assert!(is_creg(reg), "register x{} is not in compressed range", reg);
    reg - 8
}

/// GNU as compresses by spelling, not by encoding: an instruction takes a
/// 2-byte form only through one of its mnemonic's RVC aliases in the opcode
/// table (opcodes/riscv-opc.c, the `INSN_CLASS_ZCA`/`ZCF`/`ZCD` entries; no
/// Zcb here).  So `mv a0, zero` and `jalr ra, 0(t0)` stay 4 bytes although
/// `c.li a0, 0` and `c.jalr t0` mean the same.  Hand-written assembly sizes
/// code by these rules (equal-length `ALTERNATIVE` sequences, fixed-size
/// stubs), so the assembler compresses exactly what GNU as does; denser
/// spellings are the code generator's business.  `half` is what
/// [`try_compress`] made of the instruction; `operand_count` tells the
/// 1-operand `jr rs`/`jalr rs` (which have aliases) from `jalr rd, 0(rs)`.
pub fn spelling_allows(mnemonic: &str, operand_count: usize, half: u16) -> bool {
    use CKind::*;
    let Some(kind) = CKind::of(half) else {
        return false;
    };
    let lower;
    let mnemonic = if mnemonic.bytes().any(|b| b.is_ascii_uppercase()) {
        lower = mnemonic.to_ascii_lowercase();
        lower.as_str()
    } else {
        mnemonic
    };
    let aliases: &[CKind] = match mnemonic {
        "addi" => &[Addi4spn, Addi, Nop, Addi16sp, Li, Mv],
        "add" => &[Add, Addi, Addi4spn, Addi16sp, Mv],
        "li" => &[Lui, Li],
        "lui" => &[Lui],
        "mv" | "move" => &[Mv],
        "nop" => &[Nop],
        "ebreak" | "sbreak" => &[Ebreak],
        "ret" => &[Jr],
        "jr" if operand_count == 1 => &[Jr],
        "jalr" if operand_count == 1 => &[Jalr],
        "andi" => &[Andi],
        "and" => &[And, Andi],
        "or" => &[Or],
        "xor" => &[Xor],
        "sub" => &[Sub],
        "addiw" | "sext.w" => &[Addiw],
        "addw" => &[Addw, Addiw],
        "subw" => &[Subw],
        "slli" | "sll" => &[Slli],
        "srli" | "srl" => &[Srli],
        "srai" | "sra" => &[Srai],
        "lw" => &[Lw, Lwsp],
        "sw" => &[Sw, Swsp],
        // One slot each: `c.ld`/`c.sd` on RV64, `c.flw`/`c.fsw` on RV32
        // (`try_compress` only makes the one the XLEN has).
        "ld" | "flw" => &[Ld, Ldsp],
        "sd" | "fsw" => &[Sd, Sdsp],
        "fld" => &[Fld, Fldsp],
        "fsd" => &[Fsd, Fsdsp],
        _ => &[],
    };
    aliases.contains(&kind)
}

/// The mnemonic of a word of an `li` expansion (`lui`, `addiw`, `addi`,
/// `slli`), which GNU as assembles, and compresses, as written.
pub fn li_word_mnemonic(word: u32) -> &'static str {
    match (word & 0x7F, (word >> 12) & 0x7) {
        (0b0110111, _) => "lui",
        (0b0011011, 0b000) => "addiw",
        (0b0010011, 0b000) => "addi",
        (0b0010011, 0b001) => "slli",
        _ => "",
    }
}

/// The slot of a 2-byte instruction, as far as [`spelling_allows`] needs:
/// HINT and reserved encodings (which no alias produces) are `None`, and so
/// are the branches and jumps, which branch relaxation handles.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CKind {
    Addi4spn,
    Fld,
    Lw,
    Ld,
    Fsd,
    Sw,
    Sd,
    Nop,
    Addi,
    Addiw,
    Li,
    Lui,
    Addi16sp,
    Srli,
    Srai,
    Andi,
    Sub,
    Xor,
    Or,
    And,
    Subw,
    Addw,
    Slli,
    Fldsp,
    Lwsp,
    Ldsp,
    Jr,
    Mv,
    Ebreak,
    Jalr,
    Add,
    Fsdsp,
    Swsp,
    Sdsp,
}

impl CKind {
    fn of(half: u16) -> Option<Self> {
        use CKind::*;
        let h = u32::from(half);
        let rd = (h >> 7) & 0x1F;
        let rs2 = (h >> 2) & 0x1F;
        let bit12 = (h >> 12) & 1;
        let ci_imm_zero = bit12 == 0 && rs2 == 0;
        Some(match (h & 0b11, h >> 13) {
            (0b00, 0b000) => Addi4spn,
            (0b00, 0b001) => Fld,
            (0b00, 0b010) => Lw,
            (0b00, 0b011) => Ld,
            (0b00, 0b101) => Fsd,
            (0b00, 0b110) => Sw,
            (0b00, 0b111) => Sd,
            (0b01, 0b000) if half == 0x0001 => Nop,
            (0b01, 0b000) if rd != 0 && !ci_imm_zero => Addi,
            (0b01, 0b001) if rd != 0 => Addiw,
            (0b01, 0b010) if rd != 0 => Li,
            (0b01, 0b011) if rd == 2 => Addi16sp,
            (0b01, 0b011) if rd != 0 => Lui,
            (0b01, 0b100) => match ((h >> 10) & 0b11, bit12, (h >> 5) & 0b11) {
                (0b00, _, _) => Srli,
                (0b01, _, _) => Srai,
                (0b10, _, _) => Andi,
                (_, 0, 0b00) => Sub,
                (_, 0, 0b01) => Xor,
                (_, 0, 0b10) => Or,
                (_, 0, 0b11) => And,
                (_, 1, 0b00) => Subw,
                (_, 1, 0b01) => Addw,
                _ => return None,
            },
            (0b10, 0b000) if rd != 0 => Slli,
            (0b10, 0b001) => Fldsp,
            (0b10, 0b010) if rd != 0 => Lwsp,
            (0b10, 0b011) => Ldsp,
            (0b10, 0b100) => match (bit12, rd, rs2) {
                (0, 0, _) | (1, 0, 1..) => return None,
                (0, _, 0) => Jr,
                (0, _, _) => Mv,
                (1, 0, 0) => Ebreak,
                (1, _, 0) => Jalr,
                _ => Add,
            },
            (0b10, 0b101) => Fsdsp,
            (0b10, 0b110) => Swsp,
            (0b10, 0b111) => Sdsp,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_addi_sp() {
        // addi sp, sp, -48 = 0xfd010113
        let result = try_compress_rv64(0xfd010113);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0x7179,
            "c.addi sp, -48 should be 0x7179, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_sd_ra_sp() {
        // sd ra, 40(sp) = 0x02113423
        let result = try_compress_rv64(0x02113423);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0xf406,
            "c.sdsp ra, 40(sp) should be 0xf406, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_sd_s0_sp() {
        // sd s0, 32(sp) = 0x02813023
        let result = try_compress_rv64(0x02813023);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0xf022,
            "c.sdsp s0, 32(sp) should be 0xf022, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_addi_s0_sp_48() {
        // addi s0, sp, 48 = 0x03010413
        // This is addi rd=s0(x8), rs1=sp(x2), imm=48
        // rd != rs1, and rs1=x2, rd=x8 which is in creg range
        // This should be C.ADDI4SPN: addi rd', x2, uimm (scaled x4)
        let result = try_compress_rv64(0x03010413);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0x1800,
            "c.addi4spn s0, sp, 48 should be 0x1800, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_mv_t1_t0() {
        // C.MV t1, t0: 100_0_00110_00101_10 = 0x8316, from either copy form
        // (GAS 2.47 -march=rv64gc compresses both): `mv t1, t0` is
        // `addi t1, t0, 0` = 0x00028313, and `add t1, x0, t0` = 0x00500333.
        assert_eq!(try_compress_rv64(0x00028313), Some(0x8316));
        assert_eq!(try_compress_rv64(0x00500333), Some(0x8316));
        // `addi x0, t0, 0` writes x0 (a HINT): not C.MV. `li t1, 0` is C.LI.
        assert_eq!(try_compress_rv64(0x00028013), None);
        assert_eq!(try_compress_rv64(0x00000313), Some(0x4301), "c.li t1, 0");
    }

    #[test]
    fn test_compress_ld_ra_sp() {
        // ld ra, 40(sp) = 0x02813083
        let result = try_compress_rv64(0x02813083);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0x70a2,
            "c.ldsp ra, 40(sp) should be 0x70a2, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_ld_s0_sp() {
        // ld s0, 32(sp) = 0x02013403
        let result = try_compress_rv64(0x02013403);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0x7402,
            "c.ldsp s0, 32(sp) should be 0x7402, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_addi_sp_48() {
        // addi sp, sp, 48 = 0x03010113
        let result = try_compress_rv64(0x03010113);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0x6145,
            "c.addi16sp 48 should be 0x6145, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_ret() {
        // ret = jalr x0, 0(x1) = 0x00008067
        let result = try_compress_rv64(0x00008067);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0x8082,
            "c.jr ra (ret) should be 0x8082, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_ebreak() {
        let result = try_compress_rv64(0x00100073);
        assert_eq!(result, Some(0x9002));
    }

    #[test]
    fn test_compress_nop() {
        // nop = addi x0, x0, 0
        let result = try_compress_rv64(0x00000013);
        assert_eq!(result, Some(0x0001));
    }

    #[test]
    fn test_compress_li() {
        // li a0, 5 = addi a0, x0, 5 = 0x00500513
        let result = try_compress_rv64(0x00500513);
        assert!(result.is_some());
        // C.LI a0, 5: 010_0_01010_00101_01 = 0x4515 (approximation, verify below)
        let hw = result.unwrap();
        // a0 = x10, imm = 5
        // 010 | 0 | 01010 | 00101 | 01
        let expected: u16 = 0b0100_0101_0001_0101;
        assert_eq!(
            hw, expected,
            "c.li a0, 5 = 0x{:04x}, expected 0x{:04x}",
            hw, expected
        );
    }

    #[test]
    fn test_compress_slli() {
        // slli a0, a0, 3 = 0x00351513
        // a0 = x10, shamt = 3
        let word = 0x00351513;
        let result = try_compress_rv64(word);
        assert!(result.is_some());
    }
}
