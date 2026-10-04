//! Property-based tests focused on **register-bank and precision validation**
//! for the AArch64 scalar floating-point encoders:
//!
//!   * `encode_fp_arith`      — FADD/FSUB/FMUL/FDIV (FP data-processing, 2 source)
//!   * `encode_fp_1src`       — generic FP 1-source path (FABS/FNEG/FSQRT/FRINT*)
//!   * `encode_scvtf`         — signed integer→float conversion
//!   * `encode_ucvtf`         — unsigned integer→float conversion
//!   * `encode_fnmadd_fnmsub` — negated fused multiply-add/subtract
//!
//! # Campaign focus
//!
//! Every function above is an **FP** instruction. That imposes two hard contracts
//! the encoder ought to enforce:
//!
//!   1. **Register bank** — every FP operand position must receive an FP/SIMD
//!      register (S/D/H/Q), never a general-purpose (W/X) register. For
//!      `encode_scvtf`/`encode_ucvtf` the contract is inverted and stricter:
//!      the *destination* must be FP and the *source* must be GP — the only
//!      legal combination.
//!   2. **Precision homogeneity** — for the FP arith / 1-source / fnmadd forms,
//!      all operands must share one precision (all-S or all-D). `ftype` is a
//!      single 2-bit field for the whole instruction, so mixed precision has no
//!      encoding.
//!
//! # Reference oracle
//!
//! All field layouts and the embedded known-answer words are cross-checked
//! against `llvm-mc --triple=aarch64 --show-encoding` (big-endian 32-bit word =
//! the four little-endian encoding bytes reversed). Verified worked examples:
//!
//!   * FP arith (2 source), opcode `[15:12]`: FMUL=0000 FDIV=0001 FADD=0010 FSUB=0011
//!     `fmul s1,s2,s3` = 0x1E230841   `fadd s1,s2,s3` = 0x1E232841
//!     `fsub s1,s2,s3` = 0x1E233841   `fdiv s1,s2,s3` = 0x1E231841
//!   * FP 1-source, opcode `[20:15]`: FMOV=000000 FABS=000001 FNEG=000010 FSQRT=000011
//!     FRINTN=001000 FRINTP=001001 FRINTM=001010 FRINTZ=001011 FRINTA=001100
//!     `fabs s1,s2` = 0x1E20C041   `frintn s1,s2` = 0x1E244041
//!   * SCVTF/UCVTF (int→float), opcode `[18:16]`: SCVTF=010 UCVTF=011
//!     `scvtf s0,w0` = 0x1E220000  `scvtf d0,x0` = 0x9E620000
//!     `ucvtf s0,w0` = 0x1E230000  `ucvtf d0,x0` = 0x9E630000
//!   * FNMADD/FNMSUB (3 source), layout `0 00 11111 ftype 1 Rm o1 Ra Rn Rd`
//!     (bit 21 = 1 distinguishes these from FMADD/FMSUB which have bit 21 = 0):
//!     `fnmadd s0,s0,s0,s0` = 0x1F200000  `fnmadd d0,d0,d0,d0` = 0x1F600000
//!     `fnmsub s0,s0,s0,s0` = 0x1F208000  `fnmsub d0,d0,d0,d0` = 0x1F608000
//!     `fnmadd s1,s2,s3,s4` = 0x1F231041 (pins Rd=1,Rn=2,Rm=3,Ra=4)
//!
//! # Findings surfaced
//!
//! The bit-packing is **correct** for valid homogeneous-FP operands: every field
//! lands at its canonical ARMv8 position (properties `P*`), and the only
//! validation that exists today is arity + 5-bit register-number range
//! (`>= 32` is rejected). Out-of-range register numbers are never silently
//! masked.
//!
//! **Five confirmation bugs** are exposed as witness properties `W1_*`, one per
//! function. Each is marked `#[ignore]` so `cargo test` stays green; run a
//! witness and watch it fail against the current encoder with:
//!
//! ```text
//! cargo test --lib fp_scalar_bank_precision -- --ignored
//! ```
//!
//!   * `W1_fp_arith`      — FADD/FSUB/FMUL/FDIV accept GP (W/X) operands and
//!     mixed-precision (Dd,Sn,Sm) operands without error.
//!   * `W1_fp_1src`       — the generic 1-source path (FABS/FRINT*…) accepts GP
//!     operands and mixed-precision operands.
//!   * `W1_scvtf`         — SCVTF accepts a GP destination (dest must be FP) and
//!     an FP source (source must be GP).
//!   * `W1_ucvtf`         — UCVTF accepts a GP destination and an FP source.
//!   * `W1_fnmadd_fnmsub` — FNMADD/FNMSUB accept GP operands and mixed-precision
//!     operands (shares the root cause with FMADD/FMSUB).
//!
//! Root cause is uniform across the five functions: `ftype`/`sf` are derived
//! purely from a single operand's textual prefix, and `get_reg` validates only
//! the numeric range — neither the register *bank* nor precision homogeneity is
//! ever checked.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── Shared field extractors (ARMv8-A scalar FP layout) ───────────────────
fn rd_of(w: u32) -> u32 {
    w & 0x1F
}
fn rn_of(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
fn rm_of(w: u32) -> u32 {
    (w >> 16) & 0x1F
}
fn ra_of(w: u32) -> u32 {
    (w >> 10) & 0x1F
}
fn ftype_of(w: u32) -> u32 {
    (w >> 22) & 0x3
}
fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
}
fn arith_opc_of(w: u32) -> u32 {
    (w >> 12) & 0xF
} // FP arith opcode [15:12]
fn fp1_opc_of(w: u32) -> u32 {
    (w >> 15) & 0x3F
} // FP 1-source opcode [20:15]
fn itf_opc_of(w: u32) -> u32 {
    (w >> 16) & 0x7
} // int<->float opcode [18:16]

fn expect_word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}

// A small 5-bit register-number domain that is always valid (0..=31).
const REG: std::ops::Range<u32> = 0u32..32;

proptest! {
    // ════════════════════════════════════════════════════════════════════════
    // encode_fp_arith — FP data-processing (2 source): FADD/FSUB/FMUL/FDIV
    // Layout: 0 00 11110 ftype 1 Rm opcode 10 Rn Rd  (opcode is [15:12], 4-bit)
    // ════════════════════════════════════════════════════════════════════════

    // P1 — reference/field layout. Homogeneous-precision FP operands + a valid
    // 4-bit opcode => every field at its canonical bit position, no truncation.
    #[test]
    fn p1_fp_arith_valid_homogeneous_places_fields(
        rd in REG, rn in REG, rm in REG, opcode in 0u32..16u32, dbl in any::<bool>(),
    ) {
        let p = if dbl { "d" } else { "s" };
        let ops = vec![
            Operand::Reg(format!("{}{}", p, rd)),
            Operand::Reg(format!("{}{}", p, rn)),
            Operand::Reg(format!("{}{}", p, rm)),
        ];
        let w = expect_word(encode_fp_arith(&ops, opcode));
        let ftype = if dbl { 0b01u32 } else { 0b00u32 };

        // Canonical reference reconstruction (cross-checked vs llvm-mc).
        let expected = (0b00011110u32 << 24) | (ftype << 22) | (1u32 << 21)
            | (rm << 16) | (opcode << 12) | (0b10u32 << 10) | (rn << 5) | rd;
        prop_assert_eq!(w, expected);

        prop_assert_eq!(w >> 24, 0x1Eu32);       // [31:24]
        prop_assert_eq!((w >> 21) & 1, 1u32);    // bit 21
        prop_assert_eq!((w >> 10) & 0b11, 0b10u32); // [11:10]
        prop_assert_eq!(rd_of(w), rd);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(arith_opc_of(w), opcode);
        prop_assert_eq!(ftype_of(w), ftype);
        prop_assert_eq!(sf_of(w), 0);            // scalar FP, sf always 0
    }

    // P2 — ground-truth KATs for the four arithmetic opcodes (single+double),
    // pinning Rd=1,Rn=2,Rm=3 to externally verified 32-bit words from llvm-mc.
    #[test]
    fn p2_fp_arith_matches_llvm_mc_kats(opcode in 0u32..4u32, dbl in any::<bool>()) {
        let p = if dbl { "d" } else { "s" };
        let ops = vec![
            Operand::Reg(format!("{}1", p)),
            Operand::Reg(format!("{}2", p)),
            Operand::Reg(format!("{}3", p)),
        ];
        let w = expect_word(encode_fp_arith(&ops, opcode));
        // opcode mnemonic: 0=FMUL 1=FDIV 2=FADD 3=FSUB; single/double selects ftype.
        let expected = match (opcode, dbl) {
            (0, false) => 0x1E230841u32, // fmul s1,s2,s3
            (0, true)  => 0x1E630841u32, // fmul d1,d2,d3
            (1, false) => 0x1E231841u32, // fdiv s1,s2,s3
            (1, true)  => 0x1E631841u32, // fdiv d1,d2,d3
            (2, false) => 0x1E232841u32, // fadd s1,s2,s3
            (2, true)  => 0x1E632841u32, // fadd d1,d2,d3
            (3, false) => 0x1E233841u32, // fsub s1,s2,s3
            (3, true)  => 0x1E633841u32, // fsub d1,d2,d3
            _ => unreachable!(),
        };
        prop_assert_eq!(w, expected);
    }

    // P3 — validated negative contract: out-of-range FP register (>= 32) in any
    // of the three positions MUST be rejected, never masked into 5 bits.
    #[test]
    fn p3_fp_arith_rejects_out_of_range_reg(n in 32u32..256u32, pos in 0u32..3u32) {
        let mut names = vec!["d0".to_string(), "d0".to_string(), "d0".to_string()];
        names[pos as usize] = format!("d{}", n);
        let ops: Vec<Operand> = names.into_iter().map(Operand::Reg).collect();
        prop_assert!(
            encode_fp_arith(&ops, 0b0010).is_err(),
            "register d{} must be rejected (5-bit field), not silently masked", n
        );
    }

    // W1 — WITNESS (bug, #[ignore]). FP arithmetic requires FP-register operands
    // all of the same precision. GP-bank operands and mixed precision must be
    // rejected; the encoder derives ftype only from operands[0] and validates
    // neither, so it silently emits illegal encodings.
    #[test]
    #[ignore = "documented bug: fp_arith accepts GP operands and mixed precision (no bank/precision validation)"]
    fn w1_fp_arith_rejects_gp_and_mixed_precision(n in REG) {
        // (a) GP-bank operands are not valid for FP arithmetic.
        let gp = vec![
            Operand::Reg(format!("x{}", n)),
            Operand::Reg(format!("x{}", n)),
            Operand::Reg(format!("x{}", n)),
        ];
        prop_assert!(
            encode_fp_arith(&gp, 0b0010).is_err(),
            "GP operands (x{}) must be rejected for FP arithmetic; got {:?}",
            n, encode_fp_arith(&gp, 0b0010)
        );
        // (b) Mixed precision (D dest, S sources) is not encodable.
        let mix = vec![
            Operand::Reg(format!("d{}", n)),
            Operand::Reg(format!("s{}", n)),
            Operand::Reg(format!("s{}", n)),
        ];
        prop_assert!(
            encode_fp_arith(&mix, 0b0010).is_err(),
            "mixed precision (Dd,Sn,Sm) must be rejected; got {:?}",
            encode_fp_arith(&mix, 0b0010)
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // encode_fp_1src — FP data-processing (1 source): FABS/FNEG/FSQRT/FRINT*
    // Layout: 0 00 11110 ftype 1 opcode 10000 Rn Rd  (opcode is [20:15], 6-bit)
    // ════════════════════════════════════════════════════════════════════════

    // P1 — reference/field layout. Homogeneous-precision FP operands + a valid
    // 6-bit opcode => every field at its canonical bit position.
    #[test]
    fn p1_fp_1src_valid_homogeneous_places_fields(
        rd in REG, rn in REG, opcode in 0u32..64u32, dbl in any::<bool>(),
    ) {
        let p = if dbl { "d" } else { "s" };
        let ops = vec![
            Operand::Reg(format!("{}{}", p, rd)),
            Operand::Reg(format!("{}{}", p, rn)),
        ];
        let w = expect_word(encode_fp_1src(&ops, opcode));
        let ftype = if dbl { 0b01u32 } else { 0b00u32 };

        let expected = (0b00011110u32 << 24) | (ftype << 22) | (1u32 << 21)
            | (opcode << 15) | (0b10000u32 << 10) | (rn << 5) | rd;
        prop_assert_eq!(w, expected);

        prop_assert_eq!(w >> 24, 0x1Eu32);
        prop_assert_eq!((w >> 21) & 1, 1u32);
        prop_assert_eq!((w >> 10) & 0x1F, 0b10000u32);
        prop_assert_eq!(rd_of(w), rd);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(fp1_opc_of(w), opcode);
        prop_assert_eq!(ftype_of(w), ftype);
        prop_assert_eq!(sf_of(w), 0);
    }

    // P2 — ground-truth KATs (Rd=1,Rn=2) for representative 1-source opcodes,
    // verified against llvm-mc.
    #[test]
    fn p2_fp_1src_matches_llvm_mc_kats(opcode in proptest::sample::select(vec![0u32,1,2,3,8,9,10,11,12]), dbl in any::<bool>()) {
        let p = if dbl { "d" } else { "s" };
        let ops = vec![Operand::Reg(format!("{}1", p)), Operand::Reg(format!("{}2", p))];
        let w = expect_word(encode_fp_1src(&ops, opcode));
        let dbl32 = if dbl { 0x0040_0000u32 } else { 0 };
        // Single-precision base word per opcode (from llvm-mc), | 0x400000 for double.
        let base = match opcode {
            0  => 0x1E204041u32, // fmov s1,s2
            1  => 0x1E20C041u32, // fabs s1,s2
            2  => 0x1E214041u32, // fneg s1,s2
            3  => 0x1E21C041u32, // fsqrt s1,s2
            8  => 0x1E244041u32, // frintn s1,s2
            9  => 0x1E24C041u32, // frintp s1,s2
            10 => 0x1E254041u32, // frintm s1,s2
            11 => 0x1E25C041u32, // frintz s1,s2
            12 => 0x1E264041u32, // frinta s1,s2
            _ => unreachable!(),
        };
        prop_assert_eq!(w, base | dbl32);
    }

    // P3 — validated negative contract: out-of-range FP register rejected.
    #[test]
    fn p3_fp_1src_rejects_out_of_range_reg(n in 32u32..256u32, pos in 0u32..2u32) {
        let mut names = vec!["d0".to_string(), "d0".to_string()];
        names[pos as usize] = format!("d{}", n);
        let ops: Vec<Operand> = names.into_iter().map(Operand::Reg).collect();
        prop_assert!(
            encode_fp_1src(&ops, 0b001000).is_err(),
            "register d{} must be rejected (5-bit field), not silently masked", n
        );
    }

    // W1 — WITNESS (bug, #[ignore]). The generic 1-source path requires
    // homogeneous FP-register operands; GP-bank operands and mixed precision
    // must be rejected.
    #[test]
    #[ignore = "documented bug: fp_1src accepts GP operands and mixed precision (no bank/precision validation)"]
    fn w1_fp_1src_rejects_gp_and_mixed_precision(n in REG) {
        let gp = vec![
            Operand::Reg(format!("x{}", n)),
            Operand::Reg(format!("x{}", n)),
        ];
        prop_assert!(
            encode_fp_1src(&gp, 0b001000).is_err(),
            "GP operands (x{}) must be rejected for FP 1-source; got {:?}", n, encode_fp_1src(&gp, 0b001000)
        );
        let mix = vec![
            Operand::Reg(format!("d{}", n)),
            Operand::Reg(format!("s{}", n)),
        ];
        prop_assert!(
            encode_fp_1src(&mix, 0b001000).is_err(),
            "mixed precision (Dd,Sn) must be rejected; got {:?}", encode_fp_1src(&mix, 0b001000)
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // encode_scvtf — signed integer→float. Dest MUST be FP, source MUST be GP.
    // Layout: sf 00 11110 ftype 1 00 opcode 000000 Rn Rd  (opcode [18:16]=010)
    // ════════════════════════════════════════════════════════════════════════

    // P1 — reference/field layout through the PUBLIC wrapper. Valid FP dest +
    // GP source => sf from source GP width, ftype from dest prefix, opcode=010.
    #[test]
    fn p1_scvtf_valid_banks_places_fields(
        rd in REG, rn in REG, dbl_dst in any::<bool>(), src64 in any::<bool>(),
    ) {
        let dst_reg = if dbl_dst { format!("d{}", rd) } else { format!("s{}", rd) };
        let src_reg = if src64 { format!("x{}", rn) } else { format!("w{}", rn) };
        let ops = vec![Operand::Reg(dst_reg), Operand::Reg(src_reg)];
        let sf    = if src64  { 1u32 } else { 0u32 };
        let ftype = if dbl_dst { 0b01u32 } else { 0b00u32 };
        let w = expect_word(encode_scvtf(&ops));

        let expected = (sf << 31) | (0x1Eu32 << 24) | (ftype << 22)
            | (1u32 << 21) | (0b010u32 << 16) | (rn << 5) | rd;
        prop_assert_eq!(w, expected);

        prop_assert_eq!((w >> 24) & 0x7F, 0x1Eu32); // [30:24]
        prop_assert_eq!((w >> 21) & 1, 1u32);
        prop_assert_eq!((w >> 19) & 0x3, 0u32);     // [20:19] = 00
        prop_assert_eq!((w >> 10) & 0x3F, 0u32);    // [15:10] = 000000
        prop_assert_eq!(sf_of(w), sf);
        prop_assert_eq!(ftype_of(w), ftype);
        prop_assert_eq!(itf_opc_of(w), 0b010u32);   // SCVTF == signed
        prop_assert_eq!(rd_of(w), rd);
        prop_assert_eq!(rn_of(w), rn);
    }

    // P2 — ground-truth KATs (Rd=Rn=0) for all four (ftype,sf) corners.
    #[test]
    fn p2_scvtf_matches_llvm_mc_kats(dbl_dst in any::<bool>(), src64 in any::<bool>()) {
        let dst = if dbl_dst { "d0" } else { "s0" };
        let src = if src64 { "x0" } else { "w0" };
        let ops = vec![Operand::Reg(dst.into()), Operand::Reg(src.into())];
        let w = expect_word(encode_scvtf(&ops));
        let expected = match (dbl_dst, src64) {
            (false, false) => 0x1E220000u32, // scvtf s0,w0
            (true,  false) => 0x1E620000u32, // scvtf d0,w0
            (false, true)  => 0x9E220000u32, // scvtf s0,x0
            (true,  true)  => 0x9E620000u32, // scvtf d0,x0
        };
        prop_assert_eq!(w, expected);
    }

    // P3 — validated negative contract: out-of-range register rejected.
    #[test]
    fn p3_scvtf_rejects_out_of_range_reg(n in 32u32..256u32, pos in 0u32..2u32) {
        let prefix = if pos == 0 { "s" } else { "w" };
        let mut names = vec!["s0".to_string(), "w0".to_string()];
        names[pos as usize] = format!("{}{}", prefix, n);
        let ops: Vec<Operand> = names.into_iter().map(Operand::Reg).collect();
        prop_assert!(
            encode_scvtf(&ops).is_err(),
            "register {}{} must be rejected (5-bit field), not silently masked", prefix, n
        );
    }

    // W1 — WITNESS (bug, #[ignore]). SCVTF dest MUST be FP and source MUST be
    // GP; the encoder never checks banks, so it accepts a GP dest (mis-deriving
    // ftype=00 from 'w') and an FP source (mis-deriving sf=0 from 'd').
    #[test]
    #[ignore = "documented bug: scvtf accepts GP destination and FP source (no operand-bank validation)"]
    fn w1_scvtf_rejects_wrong_banks(n in REG) {
        // (a) GP destination — SCVTF Wd,Wn is not a valid instruction.
        let gp_dst = vec![Operand::Reg(format!("w{}", n)), Operand::Reg(format!("w{}", n))];
        prop_assert!(
            encode_scvtf(&gp_dst).is_err(),
            "GP destination (w{}) must be rejected; SCVTF dest must be FP, got {:?}", n, encode_scvtf(&gp_dst)
        );
        // (b) FP source — SCVTF Dd,Dn is not a valid instruction.
        let fp_src = vec![Operand::Reg(format!("d{}", n)), Operand::Reg(format!("d{}", n))];
        prop_assert!(
            encode_scvtf(&fp_src).is_err(),
            "FP source (d{}) must be rejected; SCVTF source must be GP, got {:?}", n, encode_scvtf(&fp_src)
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // encode_ucvtf — unsigned integer→float. Same bank contract as SCVTF;
    // opcode [18:16]=011 (distinguishes from SCVTF only in bit 16).
    // ════════════════════════════════════════════════════════════════════════

    // P1 — reference/field layout. opcode hard-wired to 011 (unsigned).
    #[test]
    fn p1_ucvtf_valid_banks_places_fields(
        rd in REG, rn in REG, dbl_dst in any::<bool>(), src64 in any::<bool>(),
    ) {
        let dst_reg = if dbl_dst { format!("d{}", rd) } else { format!("s{}", rd) };
        let src_reg = if src64 { format!("x{}", rn) } else { format!("w{}", rn) };
        let ops = vec![Operand::Reg(dst_reg), Operand::Reg(src_reg)];
        let sf    = if src64  { 1u32 } else { 0u32 };
        let ftype = if dbl_dst { 0b01u32 } else { 0b00u32 };
        let w = expect_word(encode_ucvtf(&ops));

        let expected = (sf << 31) | (0x1Eu32 << 24) | (ftype << 22)
            | (1u32 << 21) | (0b011u32 << 16) | (rn << 5) | rd;
        prop_assert_eq!(w, expected);
        prop_assert_eq!(itf_opc_of(w), 0b011u32); // UCVTF == unsigned
        prop_assert_eq!(sf_of(w), sf);
        prop_assert_eq!(ftype_of(w), ftype);
    }

    // P2 — signedness differential: UCVTF vs SCVTF differ ONLY in bit 16
    // (opcode 011 vs 010), and bit 16 is 1 for UCVTF / 0 for SCVTF.
    #[test]
    fn p2_ucvtf_differs_from_scvtf_only_in_bit16(
        rd in REG, rn in REG, dbl_dst in any::<bool>(), src64 in any::<bool>(),
    ) {
        let dst_reg = if dbl_dst { format!("d{}", rd) } else { format!("s{}", rd) };
        let src_reg = if src64 { format!("x{}", rn) } else { format!("w{}", rn) };
        let ops = vec![Operand::Reg(dst_reg), Operand::Reg(src_reg)];
        let wu = expect_word(encode_ucvtf(&ops));
        let ws = expect_word(encode_scvtf(&ops));
        prop_assert_eq!(ws ^ wu, 1u32 << 16);
        prop_assert_eq!((wu >> 16) & 1, 1u32); // UCVTF low opcode bit = 1
        prop_assert_eq!((ws >> 16) & 1, 0u32); // SCVTF low opcode bit = 0
    }

    // P3 — ground-truth KATs (Rd=Rn=0) for all four corners.
    #[test]
    fn p3_ucvtf_matches_llvm_mc_kats(dbl_dst in any::<bool>(), src64 in any::<bool>()) {
        let dst = if dbl_dst { "d0" } else { "s0" };
        let src = if src64 { "x0" } else { "w0" };
        let ops = vec![Operand::Reg(dst.into()), Operand::Reg(src.into())];
        let w = expect_word(encode_ucvtf(&ops));
        let expected = match (dbl_dst, src64) {
            (false, false) => 0x1E230000u32, // ucvtf s0,w0
            (true,  false) => 0x1E630000u32, // ucvtf d0,w0
            (false, true)  => 0x9E230000u32, // ucvtf s0,x0
            (true,  true)  => 0x9E630000u32, // ucvtf d0,x0
        };
        prop_assert_eq!(w, expected);
    }

    // P4 — validated negative contract: out-of-range register rejected.
    #[test]
    fn p4_ucvtf_rejects_out_of_range_reg(n in 32u32..256u32, pos in 0u32..2u32) {
        let prefix = if pos == 0 { "s" } else { "w" };
        let mut names = vec!["s0".to_string(), "w0".to_string()];
        names[pos as usize] = format!("{}{}", prefix, n);
        let ops: Vec<Operand> = names.into_iter().map(Operand::Reg).collect();
        prop_assert!(
            encode_ucvtf(&ops).is_err(),
            "register {}{} must be rejected (5-bit field), not silently masked", prefix, n
        );
    }

    // W1 — WITNESS (bug, #[ignore]). UCVTF dest MUST be FP and source MUST be
    // GP; same bank-validation gap as SCVTF.
    #[test]
    #[ignore = "documented bug: ucvtf accepts GP destination and FP source (no operand-bank validation)"]
    fn w1_ucvtf_rejects_wrong_banks(n in REG) {
        let gp_dst = vec![Operand::Reg(format!("w{}", n)), Operand::Reg(format!("w{}", n))];
        prop_assert!(
            encode_ucvtf(&gp_dst).is_err(),
            "GP destination (w{}) must be rejected; UCVTF dest must be FP, got {:?}", n, encode_ucvtf(&gp_dst)
        );
        let fp_src = vec![Operand::Reg(format!("d{}", n)), Operand::Reg(format!("d{}", n))];
        prop_assert!(
            encode_ucvtf(&fp_src).is_err(),
            "FP source (d{}) must be rejected; UCVTF source must be GP, got {:?}", n, encode_ucvtf(&fp_src)
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // encode_fnmadd_fnmsub — negated fused multiply-add/subtract.
    // Layout: 0 00 11111 ftype 1 Rm o1 Ra Rn Rd  (bit 21 = 1; FMADD/FMSUB have bit 21 = 0)
    // ════════════════════════════════════════════════════════════════════════

    // P1 — reference/field layout. Homogeneous-precision FP operands => every
    // field at its canonical bit position; bit 21 must be 1 (the FNMADD/FNMSUB
    // marker) and sf must be 0.
    #[test]
    fn p1_fnmadd_valid_homogeneous_places_fields(
        rd in REG, rn in REG, rm in REG, ra in REG,
        dbl in any::<bool>(), is_sub in any::<bool>(),
    ) {
        let p = if dbl { "d" } else { "s" };
        let ops = vec![
            Operand::Reg(format!("{}{}", p, rd)),
            Operand::Reg(format!("{}{}", p, rn)),
            Operand::Reg(format!("{}{}", p, rm)),
            Operand::Reg(format!("{}{}", p, ra)),
        ];
        let w = expect_word(encode_fnmadd_fnmsub(&ops, is_sub));
        let ftype = if dbl { 0b01u32 } else { 0b00u32 };
        let o1 = if is_sub { 1u32 } else { 0u32 };

        let expected = (0b00011111u32 << 24) | (ftype << 22) | (1u32 << 21)
            | (rm << 16) | (o1 << 15) | (ra << 10) | (rn << 5) | rd;
        prop_assert_eq!(w, expected);

        prop_assert_eq!(w >> 24, 0x1Fu32);       // [31:24] = 00011111
        prop_assert_eq!((w >> 21) & 1, 1u32);    // bit 21 = 1 (FNMADD/FNMSUB)
        prop_assert_eq!(sf_of(w), 0u32);
        prop_assert_eq!(ftype_of(w), ftype);
        prop_assert_eq!((w >> 15) & 1, o1);      // o1 = is_sub
        prop_assert_eq!(rd_of(w), rd);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(ra_of(w), ra);
    }

    // P2 — differential vs FMADD/FMSUB: on identical operands the ONLY
    // difference is bit 21 (FNMADD/FNMSUB set it, FMADD/FMSUB clear it).
    #[test]
    fn p2_fnmadd_differs_from_fmadd_only_in_bit21(
        rd in REG, rn in REG, rm in REG, ra in REG,
        dbl in any::<bool>(), is_sub in any::<bool>(),
    ) {
        let p = if dbl { "d" } else { "s" };
        let ops = vec![
            Operand::Reg(format!("{}{}", p, rd)),
            Operand::Reg(format!("{}{}", p, rn)),
            Operand::Reg(format!("{}{}", p, rm)),
            Operand::Reg(format!("{}{}", p, ra)),
        ];
        let wn = expect_word(encode_fnmadd_fnmsub(&ops, is_sub));
        let wp = expect_word(encode_fmadd_fmsub(&ops, is_sub));
        prop_assert_eq!(wn ^ wp, 1u32 << 21);
        prop_assert_eq!((wn >> 21) & 1, 1u32); // FNMADD/FNMSUB: bit 21 set
        prop_assert_eq!((wp >> 21) & 1, 0u32); // FMADD/FMSUB:   bit 21 clear
    }

    // P3 — ground-truth KATs (all-zero and pinned registers) from llvm-mc.
    #[test]
    fn p3_fnmadd_matches_llvm_mc_kats(
        dbl in any::<bool>(), is_sub in any::<bool>(), pinned in any::<bool>(),
    ) {
        let p = if dbl { "d" } else { "s" };
        let (rd, rn, rm, ra) = if pinned { (1u32, 2u32, 3u32, 4u32) } else { (0u32, 0u32, 0u32, 0u32) };
        let ops = vec![
            Operand::Reg(format!("{}{}", p, rd)),
            Operand::Reg(format!("{}{}", p, rn)),
            Operand::Reg(format!("{}{}", p, rm)),
            Operand::Reg(format!("{}{}", p, ra)),
        ];
        let w = expect_word(encode_fnmadd_fnmsub(&ops, is_sub));
        let expected = match (dbl, is_sub, pinned) {
            (false, false, false) => 0x1F200000u32, // fnmadd s0,s0,s0,s0
            (true,  false, false) => 0x1F600000u32, // fnmadd d0,d0,d0,d0
            (false, true,  false) => 0x1F208000u32, // fnmsub s0,s0,s0,s0
            (true,  true,  false) => 0x1F608000u32, // fnmsub d0,d0,d0,d0
            (false, false, true)  => 0x1F231041u32, // fnmadd s1,s2,s3,s4
            (true,  false, true)  => 0x1F631041u32, // fnmadd d1,d2,d3,d4
            (false, true,  true)  => 0x1F239041u32, // fnmsub s1,s2,s3,s4
            (true,  true,  true)  => 0x1F639041u32, // fnmsub d1,d2,d3,d4
        };
        prop_assert_eq!(w, expected);
    }

    // P4 — validated negative contract: out-of-range FP register in any of the
    // four positions MUST be rejected.
    #[test]
    fn p4_fnmadd_rejects_out_of_range_reg(n in 32u32..256u32, pos in 0u32..4u32) {
        let mut names = vec!["d0".to_string(), "d0".to_string(),
                             "d0".to_string(), "d0".to_string()];
        names[pos as usize] = format!("d{}", n);
        let ops: Vec<Operand> = names.into_iter().map(Operand::Reg).collect();
        prop_assert!(
            encode_fnmadd_fnmsub(&ops, false).is_err(),
            "register d{} must be rejected (5-bit field), not silently masked", n
        );
        prop_assert!(encode_fnmadd_fnmsub(&[], false).is_err()); // arity
    }

    // W1 — WITNESS (bug, #[ignore]). FNMADD/FNMSUB operate only on FP registers
    // of one shared precision; GP-bank operands and mixed precision must be
    // rejected. The encoder derives ftype only from operands[0] and never
    // validates banks or homogeneity (shares the FMADD/FMSUB root cause).
    #[test]
    #[ignore = "documented bug: fnmadd/fnmsub accept GP operands and mixed precision (no bank/precision validation)"]
    fn w1_fnmadd_rejects_gp_and_mixed_precision(n in REG) {
        // (a) GP-bank operands.
        let gp = vec![
            Operand::Reg(format!("x{}", n)),
            Operand::Reg(format!("x{}", n)),
            Operand::Reg(format!("x{}", n)),
            Operand::Reg(format!("x{}", n)),
        ];
        prop_assert!(
            encode_fnmadd_fnmsub(&gp, false).is_err(),
            "GP operands (x{}) must be rejected for FNMADD/FNMSUB; got {:?}", n, encode_fnmadd_fnmsub(&gp, false)
        );
        // (b) Mixed precision (D dest, S sources).
        let mix = vec![
            Operand::Reg(format!("d{}", n)),
            Operand::Reg(format!("s{}", n)),
            Operand::Reg(format!("s{}", n)),
            Operand::Reg(format!("s{}", n)),
        ];
        prop_assert!(
            encode_fnmadd_fnmsub(&mix, false).is_err(),
            "mixed precision (Dd,Sn,Sm,Sa) must be rejected; got {:?}", encode_fnmadd_fnmsub(&mix, false)
        );
    }
}
