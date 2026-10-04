//! Property-based tests for the AArch64 sign/zero-extend encoders in
//! `data_processing.rs`: `encode_sxtb`, `encode_sxth`, `encode_sxtw`,
//! `encode_uxtb`, `encode_uxth`, `encode_uxtw`.
//!
//! ## Oracle
//!
//! **Differential reference (llvm-mc-18) + field placement + negative error
//! contract.** Every reference constant below was produced by
//! `llvm-mc-18 --triple=aarch64 --show-encoding` and converted to a 32-bit
//! little-endian word. All six mnemonics alias a BFM instruction:
//!
//! ```text
//!   SBFM/UBFM:  sf opc(2) 100110 N immr(6) imms(6) Rn(5) Rd(5)
//!              opc = 00 -> SBFM (sxt*),  opc = 10 -> UBFM (uxt*)
//! ```
//!
//! The architecturally valid destination widths were confirmed with llvm-mc-18
//! (it ACCEPTS `sxtb/sxth` in *both* Wd and Xd forms, but REJECTS `uxtb/uxth`
//! Xd-destination forms, and REJECTS Wd-destination forms for `sxtw/uxtw`):
//!
//! | mnemonic | valid dest | opc | imms | base constant (Rn=Rd=0)            |
//! |----------|-----------|------|------|-------------------------------------|
//! | sxtb     | Wd & Xd   | 00   | 7    | 0x13001C00 (W) / 0x93401C00 (X)     |
//! | sxth     | Wd & Xd   | 00   | 15   | 0x13003C00 (W) / 0x93403C00 (X)     |
//! | sxtw     | Xd only   | 00   | 31   | 0x93407C00                          |
//! | uxtb     | Wd only   | 10   | 7    | 0x53001C00                          |
//! | uxth     | Wd only   | 10   | 15   | 0x53003C00                          |
//! | uxtw     | Xd only   | 10   | 31   | 0xD3407C00                          |
//!
//! ## Findings (witnessed by the `#[ignore]`d properties below)
//!
//! The encoders perform essentially **no register-width validation**, and
//! `encode_uxtw` emits the wrong instruction entirely. Each `#[ignore]`
//! property asserts the *correct* contract and FAILS against the current SUT,
//! so it is kept out of the default `cargo test` run. Run them explicitly with:
//!
//! ```text
//!   cargo test --lib data_processing_extend_pbt -- --ignored
//! ```
//!
//! * `sxtb` / `sxth`: accept a W destination with an X source (`sxtb w0, x1`),
//!   which llvm-mc-18 rejects — silent mixed-width acceptance.
//! * `sxtw`: accepts a W destination (`sxtw w0, w1`), which llvm-mc-18 rejects.
//! * `uxtb` / `uxth`: accept an X destination (`uxtb x0, x1`), which llvm-mc-18
//!   rejects.
//! * `uxtw`: emits a 32-bit `mov Wd, Wn` (ORR) instead of the required
//!   `UBFM Xd, Xn, #0, #31` for its ONLY valid spelling `uxtw Xd, Wn`, AND
//!   accepts an invalid W destination.

use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── helpers ──────────────────────────────────────────────────────────────

fn xreg(n: u32) -> Operand {
    Operand::Reg(format!("x{}", n))
}
fn wreg(n: u32) -> Operand {
    Operand::Reg(format!("w{}", n))
}

fn expect_word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}

// SBFM / UBFM field extractors.
// sf opc(2) 100110 N immr(6) imms(6) Rn(5) Rd(5)
fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
} // bit 31
fn opc_of(w: u32) -> u32 {
    (w >> 29) & 0x3
} // bits 30:29
fn fixed_of(w: u32) -> u32 {
    (w >> 23) & 0x3F
} // bits 28:23  (== 0b100110)
fn n_of(w: u32) -> u32 {
    (w >> 22) & 1
} // bit 22
fn immr_of(w: u32) -> u32 {
    (w >> 16) & 0x3F
} // bits 21:16
fn imms_of(w: u32) -> u32 {
    (w >> 10) & 0x3F
} // bits 15:10
fn rn_of(w: u32) -> u32 {
    (w >> 5) & 0x1F
} // bits 9:5
fn rd_of(w: u32) -> u32 {
    w & 0x1F
} // bits 4:0

// ──────────────────────────────────────────────────────────────────────────
//  encode_sxtb  (SXTB Rd,Rn -> SBFM Rd,Rn,#0,#7 ; valid for both W and X dest)
// ──────────────────────────────────────────────────────────────────────────
proptest! {
    // P1. Differential reference: matches llvm-mc-18 for both the 32- and
    //     64-bit forms across the full register range. sf/N are derived from
    //     the destination width.
    //       sxtb w0,w1 -> [0x20,0x1c,0x00,0x13] = 0x13001C20
    //       sxtb x0,x1 -> [0x20,0x1c,0x40,0x93] = 0x93401C20
    #[test]
    fn sxtb_reference_encoding(
        rd in 0u32..=31,
        rn in 0u32..=31,
        is_64 in any::<bool>(),
    ) {
        let (dst, src) = if is_64 { (xreg(rd), xreg(rn)) } else { (wreg(rd), wreg(rn)) };
        let w = expect_word(encode_sxtb(&[dst, src]));
        let base = if is_64 { 0x93401C00u32 } else { 0x13001C00u32 };
        prop_assert_eq!(w, base | (rn << 5) | rd);
    }

    // P2. Field placement: opc=00 (SBFM), fixed=100110, N==sf, immr=0, imms=7.
    #[test]
    fn sxtb_field_placement(
        rd in 0u32..=31,
        rn in 0u32..=31,
        is_64 in any::<bool>(),
    ) {
        let (dst, src) = if is_64 { (xreg(rd), xreg(rn)) } else { (wreg(rd), wreg(rn)) };
        let w = expect_word(encode_sxtb(&[dst, src]));
        prop_assert_eq!(sf_of(w), if is_64 { 1 } else { 0 });
        prop_assert_eq!(opc_of(w), 0b00);
        prop_assert_eq!(fixed_of(w), 0b100110);
        prop_assert_eq!(n_of(w), if is_64 { 1 } else { 0 });
        prop_assert_eq!(immr_of(w), 0);
        prop_assert_eq!(imms_of(w), 7);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    // P3. Negative contract: fewer than two operands, or a non-register in
    //     either position, is rejected.
    #[test]
    fn sxtb_rejects_bad_operands(
        n in 0u32..=31,
        bad_pos in 0u32..2,
    ) {
        prop_assert!(encode_sxtb(&[]).is_err());
        prop_assert!(encode_sxtb(&[wreg(n)]).is_err());
        let mut ops = vec![wreg(n), wreg(n)];
        ops[bad_pos as usize] = Operand::Imm(5);
        prop_assert!(encode_sxtb(&ops).is_err());
    }

    // P4. BUG WITNESS (#[ignore]): a 32-bit (W) destination with a 64-bit (X)
    //     source has no valid encoding — llvm-mc-18 rejects `sxtb w0, x1` with
    //     "invalid operand for instruction". A conforming encoder MUST return
    //     Err. `encode_sxtb` ignores the source width and silently emits a
    //     32-bit SBFM word.
    #[ignore = "documented bug: sxtb accepts mixed W-dest/X-src widths (llvm-mc rejects)"]
    #[test]
    fn sxtb_rejects_w_destination_with_x_source(
        n in 0u32..=31,
    ) {
        let ops = vec![wreg(n), xreg(n)]; // sxtb wN, xN -- invalid
        prop_assert!(
            encode_sxtb(&ops).is_err(),
            "W dest + X src is invalid for SXTB (llvm-mc-18); got {:?}",
            encode_sxtb(&ops)
        );
    }
}

// ──────────────────────────────────────────────────────────────────────────
//  encode_sxth  (SXTH Rd,Rn -> SBFM Rd,Rn,#0,#15 ; valid for both W and X dest)
// ──────────────────────────────────────────────────────────────────────────
proptest! {
    // P1. Differential reference:
    //       sxth w0,w1 -> [0x20,0x3c,0x00,0x13] = 0x13003C20
    //       sxth x0,x1 -> [0x20,0x3c,0x40,0x93] = 0x93403C20
    #[test]
    fn sxth_reference_encoding(
        rd in 0u32..=31,
        rn in 0u32..=31,
        is_64 in any::<bool>(),
    ) {
        let (dst, src) = if is_64 { (xreg(rd), xreg(rn)) } else { (wreg(rd), wreg(rn)) };
        let w = expect_word(encode_sxth(&[dst, src]));
        let base = if is_64 { 0x93403C00u32 } else { 0x13003C00u32 };
        prop_assert_eq!(w, base | (rn << 5) | rd);
    }

    // P2. Field placement: opc=00 (SBFM), fixed=100110, N==sf, immr=0, imms=15.
    #[test]
    fn sxth_field_placement(
        rd in 0u32..=31,
        rn in 0u32..=31,
        is_64 in any::<bool>(),
    ) {
        let (dst, src) = if is_64 { (xreg(rd), xreg(rn)) } else { (wreg(rd), wreg(rn)) };
        let w = expect_word(encode_sxth(&[dst, src]));
        prop_assert_eq!(sf_of(w), if is_64 { 1 } else { 0 });
        prop_assert_eq!(opc_of(w), 0b00);
        prop_assert_eq!(fixed_of(w), 0b100110);
        prop_assert_eq!(n_of(w), if is_64 { 1 } else { 0 });
        prop_assert_eq!(immr_of(w), 0);
        prop_assert_eq!(imms_of(w), 15);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    // P3. Negative contract: too few operands / non-register operand -> Err.
    #[test]
    fn sxth_rejects_bad_operands(
        n in 0u32..=31,
        bad_pos in 0u32..2,
    ) {
        prop_assert!(encode_sxth(&[]).is_err());
        prop_assert!(encode_sxth(&[wreg(n)]).is_err());
        let mut ops = vec![wreg(n), wreg(n)];
        ops[bad_pos as usize] = Operand::Imm(5);
        prop_assert!(encode_sxth(&ops).is_err());
    }

    // P4. BUG WITNESS (#[ignore]): `sxth w0, x1` is rejected by llvm-mc-18 but
    //     silently accepted by `encode_sxth` (source width ignored).
    #[ignore = "documented bug: sxth accepts mixed W-dest/X-src widths (llvm-mc rejects)"]
    #[test]
    fn sxth_rejects_w_destination_with_x_source(
        n in 0u32..=31,
    ) {
        let ops = vec![wreg(n), xreg(n)]; // sxth wN, xN -- invalid
        prop_assert!(
            encode_sxth(&ops).is_err(),
            "W dest + X src is invalid for SXTH (llvm-mc-18); got {:?}",
            encode_sxth(&ops)
        );
    }
}

// ──────────────────────────────────────────────────────────────────────────
//  encode_sxtw  (SXTW Xd,Wn -> SBFM Xd,Xn,#0,#31 ; Xd destination ONLY)
// ──────────────────────────────────────────────────────────────────────────
proptest! {
    // P1. Differential reference: sf and N are hardcoded to 1 (64-bit only).
    //       sxtw x0,w1 -> [0x20,0x7c,0x40,0x93] = 0x93407C20
    #[test]
    fn sxtw_reference_encoding(
        rd in 0u32..=31,
        rn in 0u32..=31,
    ) {
        let w = expect_word(encode_sxtw(&[xreg(rd), wreg(rn)]));
        prop_assert_eq!(w, 0x93407C00u32 | (rn << 5) | rd);
    }

    // P2. Field placement: sf=1, opc=00 (SBFM), fixed=100110, N=1, immr=0,
    //     imms=31 — all independent of the (ignored) destination width.
    #[test]
    fn sxtw_field_placement(
        rd in 0u32..=31,
        rn in 0u32..=31,
    ) {
        let w = expect_word(encode_sxtw(&[xreg(rd), wreg(rn)]));
        prop_assert_eq!(sf_of(w), 1);
        prop_assert_eq!(opc_of(w), 0b00);
        prop_assert_eq!(fixed_of(w), 0b100110);
        prop_assert_eq!(n_of(w), 1);
        prop_assert_eq!(immr_of(w), 0);
        prop_assert_eq!(imms_of(w), 31);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    // P3. Negative contract: too few operands / non-register operand -> Err.
    #[test]
    fn sxtw_rejects_bad_operands(
        n in 0u32..=31,
        bad_pos in 0u32..2,
    ) {
        prop_assert!(encode_sxtw(&[]).is_err());
        prop_assert!(encode_sxtw(&[xreg(n)]).is_err());
        let mut ops = vec![xreg(n), wreg(n)];
        ops[bad_pos as usize] = Operand::Imm(5);
        prop_assert!(encode_sxtw(&ops).is_err());
    }

    // P4. BUG WITNESS (#[ignore]): SXTW is defined ONLY as `SXTW <Xd>, <Wn>`.
    //     llvm-mc-18 rejects `sxtw w0, w1` ("invalid operand for instruction").
    //     `encode_sxtw` hardcodes sf=1 and discards the destination width, so
    //     it silently emits the 64-bit SBFM word for an invalid W destination.
    #[test]
    fn sxtw_rejects_w_destination(
        n in 0u32..=31,
    ) {
        let ops = vec![wreg(n), wreg(n)]; // sxtw wN, wN -- invalid destination
        prop_assert!(
            encode_sxtw(&ops).is_err(),
            "W destination is invalid for SXTW (llvm-mc-18); got {:?}",
            encode_sxtw(&ops)
        );
    }
}

// ──────────────────────────────────────────────────────────────────────────
//  encode_uxtb  (UXTB Wd,Wn -> UBFM Wd,Wn,#0,#7 ; Wd destination ONLY)
// ──────────────────────────────────────────────────────────────────────────
proptest! {
    // P1. Differential reference:
    //       uxtb w0,w1 -> [0x20,0x1c,0x00,0x53] = 0x53001C20
    #[test]
    fn uxtb_reference_encoding(
        rd in 0u32..=31,
        rn in 0u32..=31,
    ) {
        let w = expect_word(encode_uxtb(&[wreg(rd), wreg(rn)]));
        prop_assert_eq!(w, 0x53001C00u32 | (rn << 5) | rd);
    }

    // P2. Field placement: sf=0, opc=10 (UBFM), fixed=100110, N=0, immr=0,
    //     imms=7.
    #[test]
    fn uxtb_field_placement(
        rd in 0u32..=31,
        rn in 0u32..=31,
    ) {
        let w = expect_word(encode_uxtb(&[wreg(rd), wreg(rn)]));
        prop_assert_eq!(sf_of(w), 0);
        prop_assert_eq!(opc_of(w), 0b10);
        prop_assert_eq!(fixed_of(w), 0b100110);
        prop_assert_eq!(n_of(w), 0);
        prop_assert_eq!(immr_of(w), 0);
        prop_assert_eq!(imms_of(w), 7);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    // P3. Negative contract: too few operands / non-register operand -> Err.
    #[test]
    fn uxtb_rejects_bad_operands(
        n in 0u32..=31,
        bad_pos in 0u32..2,
    ) {
        prop_assert!(encode_uxtb(&[]).is_err());
        prop_assert!(encode_uxtb(&[wreg(n)]).is_err());
        let mut ops = vec![wreg(n), wreg(n)];
        ops[bad_pos as usize] = Operand::Imm(5);
        prop_assert!(encode_uxtb(&ops).is_err());
    }

    // P4. BUG WITNESS (#[ignore]): UXTB is defined ONLY as `UXTB <Wd>, <Wn>`.
    //     llvm-mc-18 rejects `uxtb x0, x1`. `encode_uxtb` derives sf/N from the
    //     destination, so it silently emits a 64-bit UBFM word for an invalid X
    //     destination.
    #[ignore = "documented bug: uxtb accepts an X destination (llvm-mc rejects)"]
    #[test]
    fn uxtb_rejects_x_destination(
        n in 0u32..=31,
    ) {
        let ops = vec![xreg(n), xreg(n)]; // uxtb xN, xN -- invalid destination
        prop_assert!(
            encode_uxtb(&ops).is_err(),
            "X destination is invalid for UXTB (llvm-mc-18); got {:?}",
            encode_uxtb(&ops)
        );
    }
}

// ──────────────────────────────────────────────────────────────────────────
//  encode_uxth  (UXTH Wd,Wn -> UBFM Wd,Wn,#0,#15 ; Wd destination ONLY)
// ──────────────────────────────────────────────────────────────────────────
proptest! {
    // P1. Differential reference:
    //       uxth w0,w1 -> [0x20,0x3c,0x00,0x53] = 0x53003C20
    #[test]
    fn uxth_reference_encoding(
        rd in 0u32..=31,
        rn in 0u32..=31,
    ) {
        let w = expect_word(encode_uxth(&[wreg(rd), wreg(rn)]));
        prop_assert_eq!(w, 0x53003C00u32 | (rn << 5) | rd);
    }

    // P2. Field placement: sf=0, opc=10 (UBFM), fixed=100110, N=0, immr=0,
    //     imms=15.
    #[test]
    fn uxth_field_placement(
        rd in 0u32..=31,
        rn in 0u32..=31,
    ) {
        let w = expect_word(encode_uxth(&[wreg(rd), wreg(rn)]));
        prop_assert_eq!(sf_of(w), 0);
        prop_assert_eq!(opc_of(w), 0b10);
        prop_assert_eq!(fixed_of(w), 0b100110);
        prop_assert_eq!(n_of(w), 0);
        prop_assert_eq!(immr_of(w), 0);
        prop_assert_eq!(imms_of(w), 15);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    // P3. Negative contract: too few operands / non-register operand -> Err.
    #[test]
    fn uxth_rejects_bad_operands(
        n in 0u32..=31,
        bad_pos in 0u32..2,
    ) {
        prop_assert!(encode_uxth(&[]).is_err());
        prop_assert!(encode_uxth(&[wreg(n)]).is_err());
        let mut ops = vec![wreg(n), wreg(n)];
        ops[bad_pos as usize] = Operand::Imm(5);
        prop_assert!(encode_uxth(&ops).is_err());
    }

    // P4. BUG WITNESS (#[ignore]): UXTH is defined ONLY as `UXTH <Wd>, <Wn>`.
    //     llvm-mc-18 rejects `uxth x0, x1`. `encode_uxth` derives sf/N from the
    //     destination and silently emits a 64-bit UBFM word for an invalid X
    //     destination.
    #[ignore = "documented bug: uxth accepts an X destination (llvm-mc rejects)"]
    #[test]
    fn uxth_rejects_x_destination(
        n in 0u32..=31,
    ) {
        let ops = vec![xreg(n), xreg(n)]; // uxth xN, xN -- invalid destination
        prop_assert!(
            encode_uxth(&ops).is_err(),
            "X destination is invalid for UXTH (llvm-mc-18); got {:?}",
            encode_uxth(&ops)
        );
    }
}

// ──────────────────────────────────────────────────────────────────────────
//  encode_uxtw  (UXTW Xd,Wn -> UBFM Xd,Xn,#0,#31 ; Xd destination ONLY)
//  NOTE: the current implementation emits the WRONG instruction — a 32-bit
//  `mov Wd, Wn` (ORR) — and performs no width validation. The two correctness
//  properties below are therefore #[ignore]d witnesses.
// ──────────────────────────────────────────────────────────────────────────
proptest! {
    // P1. Negative contract: too few operands / non-register operand -> Err.
    //     (This is the only behaviour of encode_uxtw that is correct today.)
    #[test]
    fn uxtw_rejects_bad_operands(
        n in 0u32..=31,
        bad_pos in 0u32..2,
    ) {
        prop_assert!(encode_uxtw(&[]).is_err());
        prop_assert!(encode_uxtw(&[xreg(n)]).is_err());
        let mut ops = vec![xreg(n), wreg(n)];
        ops[bad_pos as usize] = Operand::Imm(5);
        prop_assert!(encode_uxtw(&ops).is_err());
    }

    // P2. BUG WITNESS (#[ignore]) — WRONG INSTRUCTION. The ONLY spec-valid
    //     spelling is `uxtw Xd, Wn`, which llvm-mc-18 encodes as
    //     UBFM Xd, Xn, #0, #31:
    //       uxtw x0,w1 -> [0x20,0x7c,0x40,0xd3] = 0xD3407C20
    //     A conforming encoder MUST emit 0xD3407C00 | (rn << 5) | rd. Instead
    //     `encode_uxtw` emits 0x2A000000 | (rn << 16) | rd — a 32-bit
    //     `mov Wd, Wn` (ORR Wd, WZR, Wn) that neither preserves the 64-bit
    //     destination nor matches any valid `uxtw` encoding.
    #[ignore = "documented bug: uxtw emits a 32-bit mov (ORR) instead of UBFM Xd,Xn,#0,#31"]
    #[test]
    fn uxtw_canonical_encoding_for_xd_wn(
        rd in 0u32..=31,
        rn in 0u32..=31,
    ) {
        let ops = vec![xreg(rd), wreg(rn)]; // uxtw Xd, Wn -- the valid form
        let w = expect_word(encode_uxtw(&ops));
        prop_assert_eq!(
            w, 0xD3407C00u32 | (rn << 5) | rd,
            "`uxtw x{}, w{}` must encode as UBFM Xd, Xn, #0, #31 (llvm-mc-18), got 0x{:08X}",
            rd, rn, w,
        );
    }

    // P3. BUG WITNESS (#[ignore]): UXTW is defined ONLY as `UXTW <Xd>, <Wn>`.
    //     llvm-mc-18 rejects `uxtw w0, w1`. `encode_uxtw` accepts it.
    #[ignore = "documented bug: uxtw accepts a W destination (llvm-mc rejects)"]
    #[test]
    fn uxtw_rejects_w_destination(
        n in 0u32..=31,
    ) {
        let ops = vec![wreg(n), wreg(n)]; // uxtw wN, wN -- invalid destination
        prop_assert!(
            encode_uxtw(&ops).is_err(),
            "W destination is invalid for UXTW (llvm-mc-18); got {:?}",
            encode_uxtw(&ops)
        );
    }
}
