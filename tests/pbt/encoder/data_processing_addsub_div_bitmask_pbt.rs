//! Property-based tests for register-width and shift/extend validation in
//! `data_processing.rs`, covering three encoders:
//!
//!   * `encode_add_sub` — both the *immediate* and the *register* (shifted /
//!     extended) forms
//!   * `encode_div`     — the shared SDIV/UDIV encoder
//!   * `encode_bitmask_imm` — the logical-immediate decoder-inverse
//!
//! ## Focus
//!
//! Per the request, the suite concentrates on two contracts:
//!
//! 1. **Register-width validation.** ARMv8 ADD/SUB (shifted register form) and
//!    SDIV/UDIV require `<Rd>, <Rn>, <Rm>` to all share one register width (all
//!    W or all X). The encoders derive `sf` from the destination only and
//!    discard the source widths, so a mixed-width spelling such as
//!    `add x0, w1, x2` or `sdiv x0, w1, w2` is silently mis-typed.
//!    `llvm-mc`/`clang --target=aarch64` *reject* these — confirmed empirically.
//!
//! 2. **Shift/extend validation.** The shifted-register form masks the shift
//!    amount with `& 0x3F` for *both* W and X registers (W must reject ≥32;
//!    X must reject ≥64). The extended-register form maps an unknown extend
//!    mnemonic to UXTX via a catch-all and masks the optional shift with
//!    `& 0x7` (valid range is 0..=4; 5..=7 reserved, ≥8 silently truncated).
//!    The immediate form masks `lsl #12` immediates with `& 0xFFF`.
//!
//! ## Witness policy
//!
//! Every property that demonstrates one of the above defects asserts the
//! *correct* contract and is marked `#[ignore = "documented bug: …"]`, so the
//! default `cargo test` run stays green. Run the witnesses explicitly:
//!
//! ```text
//!   cargo test --test pbt_arm_core data_processing_addsub_div_bitmask_pbt -- --ignored
//! ```
//!
//! ## Oracles
//!
//!   * **Negative/error contract** for the add_sub/div width & shift/extend
//!     defects (oracle: `llvm-mc`/`clang --target=aarch64` rejection).
//!   * **Reference / differential** for `encode_bitmask_imm`: the 10 hardcoded
//!     `(value, is_64) -> (N, immr, imms)` triples below were produced by
//!     `clang --target=aarch64-linux-gnu -c` on `orr Rd, RZR, #imm` and read
//!     out of the `.text` section. They pin the encoder to the canonical
//!     encoding. A reference `decode_bitmask` (the mathematical inverse of the
//!     field semantics, validated against those same 10 vectors) then powers a
//!     full-domain round-trip property.
//!
//! NOTE: `encode_bitmask_imm` is *correct* on every probed case — its
//! properties are all green characterization/reference checks; there is no
//! `#[ignore]` witness for it.

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

// ADD/SUB field extractors.
//   immediate:        sf op S 10001 0 sh imm12 Rn Rd      (sh = bit 22)
//   shifted register: sf op S 01011 shift Rm imm6 Rn Rd
//   extended reg:     sf op S 01011 00 1 Rm option imm3 Rn Rd (bit21=1)
fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
}
fn op_of(w: u32) -> u32 {
    (w >> 30) & 1
}
fn s_of(w: u32) -> u32 {
    (w >> 29) & 1
}
fn opcode5_of(w: u32) -> u32 {
    (w >> 24) & 0x1F
} // bits 28:24
fn sh_of(w: u32) -> u32 {
    (w >> 22) & 1
} // immediate shift bit
fn imm12_of(w: u32) -> u32 {
    (w >> 10) & 0xFFF
}
fn shift_type_of(w: u32) -> u32 {
    (w >> 22) & 0x3
}
fn shift_amt_of(w: u32) -> u32 {
    (w >> 10) & 0x3F
} // imm6
fn rm_of(w: u32) -> u32 {
    (w >> 16) & 0x1F
}
fn rn_of(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
fn rd_of(w: u32) -> u32 {
    w & 0x1F
}
fn ext21_of(w: u32) -> u32 {
    (w >> 21) & 1
} // 1 => extended register
fn option_of(w: u32) -> u32 {
    (w >> 13) & 0x7
}
fn imm3_of(w: u32) -> u32 {
    (w >> 10) & 0x7
}

// SDIV/UDIV: sf 0 0 11010110 Rm 00001 o1 Rn Rd  (o1 = bit 10: 0=UDIV, 1=SDIV)
fn div_o1_of(w: u32) -> u32 {
    (w >> 10) & 1
}

// ── Reference decoder for the AArch64 logical-immediate fields ───────────
// Independent inverse of the field semantics, written from the
// "element size = leading-ones(N:imms) → 2,4,…,64; element = ROR(Ones(s), r),
// replicated" definition and validated against the 10 clang-derived vectors in
// `bitmask_matches_clang_reference`. NOT a copy of `encode_bitmask_imm`.
// n-bit mask; guards the shift-by-64 overflow that `(1u64 << n) - 1` hits.
fn mask_bits(n: u32) -> u64 {
    if n >= 64 { u64::MAX } else { (1u64 << n) - 1 }
}

fn decode_bitmask(n: u32, immr: u32, imms: u32, datasize: u32) -> Option<u64> {
    // Element size: N=1 ⇒ 64; otherwise count the leading 1-bits of imms
    // (MSB-first over 6 bits) and take esize = 2^(5 - leading_ones).
    let esize = if n == 1 {
        64u32
    } else {
        let mut lo = 0u32;
        for b in (0u32..6).rev() {
            if (imms >> b) & 1 == 1 {
                lo += 1;
            } else {
                break;
            }
        }
        if lo > 5 {
            return None; // imms = 0b111111 with N = 0 is a reserved encoding
        }
        1u32 << (5 - lo)
    };
    if esize > datasize {
        return None;
    }
    // Number of set bits in the (pre-rotation) element = (imms & esize-1) + 1.
    let s = (imms & (esize - 1)) + 1;
    if s == 0 || s >= esize {
        return None; // all-zero / all-ones element ⇒ not representable
    }
    let welem = (1u64 << s) - 1;
    let r = (immr % esize) as u64;
    let elem_mask = mask_bits(esize);
    let rot = if r == 0 {
        welem & elem_mask
    } else {
        ((welem >> r) | (welem << (esize as u64 - r))) & elem_mask
    };
    let mut w = 0u64;
    let mut i = 0u64;
    while i < datasize as u64 {
        w |= rot << i;
        i += esize as u64;
    }
    Some(w & mask_bits(datasize))
}

// =========================================================================
//  encode_add_sub — IMMEDIATE form
// =========================================================================
proptest! {
    // Positive: the immediate form derives sf ONLY from the destination, and
    // a 0..=0xFFF immediate is placed verbatim in imm12 with sh=0. Holds for
    // both 32- and 64-bit, ADD/SUB, and with/without flags.
    #[test]
    fn addsub_imm_sf_and_fields_track_destination(
        n in 0u32..=30,
        is_64 in any::<bool>(),
        is_sub in any::<bool>(),
        set_flags in any::<bool>(),
        imm in 0i64..=0xFFF,
    ) {
        let dst = if is_64 { xreg(n) } else { wreg(n) };
        let src = if is_64 { xreg(n) } else { wreg(n) };
        let ops = vec![dst, src, Operand::Imm(imm)];
        let w = expect_word(encode_add_sub(&ops, is_sub, set_flags));
        prop_assert_eq!(sf_of(w), if is_64 { 1 } else { 0 });
        prop_assert_eq!(op_of(w), if is_sub { 1 } else { 0 });
        prop_assert_eq!(s_of(w), if set_flags { 1 } else { 0 });
        prop_assert_eq!(opcode5_of(w), 0b10001); // add/sub immediate
        prop_assert_eq!(sh_of(w), 0);
        prop_assert_eq!(imm12_of(w), imm as u32);
    }

    // Positive: an explicit `lsl #12` with a value that fits the 12-bit field
    // sets the sh bit and places the immediate verbatim. (0x000..=0xFFF.)
    #[test]
    fn addsub_imm_explicit_lsl12_valid_when_fits(
        n in 0u32..=30,
        k in 0u32..=0xFFF,
        is_sub in any::<bool>(),
    ) {
        let ops = vec![xreg(n), xreg(n), Operand::Imm(k as i64),
                       Operand::Shift { kind: "lsl".into(), amount: 12 }];
        let w = expect_word(encode_add_sub(&ops, is_sub, false));
        prop_assert_eq!(opcode5_of(w), 0b10001);
        prop_assert_eq!(sh_of(w), 1);
        prop_assert_eq!(imm12_of(w), k);
    }

    // BUG WITNESS (#[ignore]): the immediate branch masks an `lsl #12`
    // immediate with `& 0xFFF` instead of range-checking it. A value that
    // exceeds the 12-bit field is silently truncated (e.g. #0x1FFF → 0xFFF).
    // `clang --target=aarch64` rejects `add x0, x1, #0x1fff, lsl #12` with
    // "integer in range [0, 4095]". A conforming encoder MUST return Err.
    #[ignore = "documented bug: add/sub immediate lsl #12 silently truncates (>0xFFF) instead of erroring (clang rejects)"]
    #[test]
    fn addsub_imm_lsl12_rejects_oversized_immediate(
        n in 0u32..=30,
        k in 0x1000u32..=0x7FFF,
        is_sub in any::<bool>(),
    ) {
        let ops = vec![xreg(n), xreg(n), Operand::Imm(k as i64),
                       Operand::Shift { kind: "lsl".into(), amount: 12 }];
        prop_assert!(
            encode_add_sub(&ops, is_sub, false).is_err(),
            "`add x{}, x{}, #{:#x}, lsl #12` must be rejected (imm > 12 bits); got {:?}",
            n, n, k, encode_add_sub(&ops, is_sub, false),
        );
    }
}

// =========================================================================
//  encode_add_sub — REGISTER (shifted) form : width + shift validation
// =========================================================================
proptest! {
    // Positive: a uniform-width shifted-register ADD with a valid X-register
    // shift (0..=63) places shift-type, imm6, S, and all register fields per
    // the ARMv8 encoding, and sf tracks the (uniform) width.
    #[test]
    fn addsub_shifted_reg_valid_fields(
        n in 0u32..=30,
        sk in 0u32..=2u32,             // 0=lsl, 1=lsr, 2=asr
        amount in 0u32..=63u32,        // X-register imm6 range
        is_64 in any::<bool>(),
        is_sub in any::<bool>(),
        set_flags in any::<bool>(),
    ) {
        let (kind, want_st) = match sk { 0 => ("lsl", 0u32), 1 => ("lsr", 1u32), _ => ("asr", 2u32) };
        let r = if is_64 { xreg(n) } else { wreg(n) };
        let ops = vec![r.clone(), r.clone(), r.clone(),
                       Operand::Shift { kind: kind.into(), amount }];
        // Note: for W registers amount must be ≤31; keep the positive test in
        // the valid range for the chosen width.
        prop_assume!(is_64 || amount <= 31);
        let w = expect_word(encode_add_sub(&ops, is_sub, set_flags));
        prop_assert_eq!(opcode5_of(w), 0b01011);
        prop_assert_eq!(ext21_of(w), 0);              // shifted, not extended
        prop_assert_eq!(shift_type_of(w), want_st);
        prop_assert_eq!(shift_amt_of(w), amount);
        prop_assert_eq!(s_of(w), if set_flags { 1 } else { 0 });
        prop_assert_eq!(sf_of(w), if is_64 { 1 } else { 0 });
    }

    // BUG WITNESS (#[ignore]): the shifted-register form reads only Rd's width
    // (via get_reg) and reads Rm via parse_reg_num (which discards width), so a
    // mixed-width register form such as `add x0, w1, w2` is silently emitted
    // as a 64-bit instruction with W-numbered sources. ARMv8 requires all three
    // operands to share one width; clang rejects `add x0, w1, x2` /
    // `add w0, x1, x2` with "invalid operand for instruction".
    #[ignore = "documented bug: add/sub shifted-register accepts mixed register widths (clang rejects)"]
    #[test]
    fn addsub_shifted_reg_rejects_mixed_widths(
        n in 0u32..=30,
        rd_is_x in any::<bool>(),
        rn_is_x in any::<bool>(),
        rm_is_x in any::<bool>(),
    ) {
        // require at least one width disagreement
        prop_assume!(!(rd_is_x == rn_is_x && rn_is_x == rm_is_x));
        let rd = if rd_is_x { xreg(n) } else { wreg(n) };
        let rn = if rn_is_x { xreg(n) } else { wreg(n) };
        let rm = if rm_is_x { xreg(n) } else { wreg(n) };
        let ops = vec![rd, rn, rm];
        prop_assert!(
            encode_add_sub(&ops, false, false).is_err(),
            "mixed-width `add {:?},{:?},{:?}` must be rejected; got {:?}",
            ops[0], ops[1], ops[2], encode_add_sub(&ops, false, false),
        );
    }

    // BUG WITNESS (#[ignore]): for a 32-bit (W) shifted-register form, imm6 is
    // masked with `& 0x3F`, so shift amounts 32..=63 (UNDEFINED per the ARMv8
    // ARM for W registers) are silently encoded. clang rejects
    // `add w0, w1, w2, lsl #32` / `lsr #40`.
    #[ignore = "documented bug: add/sub W-reg shifted form accepts shift 32..63 (UNDEFINED; clang rejects)"]
    #[test]
    fn addsub_shifted_reg_w_rejects_shift_above_31(
        n in 0u32..=30,
        amount in 32u32..=63u32,
        sk in 0u32..=2u32,
    ) {
        let kind = match sk { 0 => "lsl", 1 => "lsr", _ => "asr" };
        let ops = vec![wreg(n), wreg(n), wreg(n),
                       Operand::Shift { kind: kind.into(), amount }];
        prop_assert!(
            encode_add_sub(&ops, false, false).is_err(),
            "`add w{}, w{}, w{}, {} #{}` is UNDEFINED for W regs and must be rejected; got {:?}",
            n, n, n, kind, amount, encode_add_sub(&ops, false, false),
        );
    }

    // BUG WITNESS (#[ignore]): for a 64-bit (X) shifted-register form, imm6 is
    // masked with `& 0x3F`, so shift amounts ≥ 64 (out of range) silently wrap
    // (e.g. #64 → 0, #65 → 1). clang rejects `add x0, x1, x2, lsl #64`.
    #[ignore = "documented bug: add/sub X-reg shifted form masks shift ≥64 into imm6 (clang rejects)"]
    #[test]
    fn addsub_shifted_reg_x_rejects_shift_above_63(
        n in 0u32..=30,
        amount in 64u32..=255u32,
        sk in 0u32..=2u32,
    ) {
        let kind = match sk { 0 => "lsl", 1 => "lsr", _ => "asr" };
        let ops = vec![xreg(n), xreg(n), xreg(n),
                       Operand::Shift { kind: kind.into(), amount }];
        prop_assert!(
            encode_add_sub(&ops, false, false).is_err(),
            "`add x{}, x{}, x{}, {} #{}` is out of range (max 63) and must be rejected; got {:?}",
            n, n, n, kind, amount, encode_add_sub(&ops, false, false),
        );
    }
}

// =========================================================================
//  encode_add_sub — EXTENDED-register form : extend validation
// =========================================================================
proptest! {
    // Positive: a known extend kind with amount 0 selects the right `option`
    // field, sets the extended-register indicator (bit 21), and zeroes imm3.
    #[test]
    fn addsub_extended_reg_valid_option_field(
        n in 0u32..=30,
        ek in 0u32..=7u32,
        set_flags in any::<bool>(),
    ) {
        let (kind, want_opt) = match ek {
            0 => ("uxtb", 0b000u32), 1 => ("uxth", 0b001), 2 => ("uxtw", 0b010),
            3 => ("uxtx", 0b011),    4 => ("sxtb", 0b100), 5 => ("sxth", 0b101),
            6 => ("sxtw", 0b110),    _ => ("sxtx", 0b111),
        };
        // Rm's width follows the extend: the 64-to-64 extends (uxtx/sxtx)
        // read an X register, every other extend reads a W register
        // (llvm-mc 23.1.2 + ARMv8 ARM syntax; GNU as also accepts the
        // mismatched spelling and encodes the same word -- a leniency the
        // fail-closed contract does not copy).
        let rm = if matches!(ek, 3 | 7) { xreg(n) } else { wreg(n) };
        let ops = vec![xreg(n), xreg(n), rm,
                       Operand::Extend { kind: kind.into(), amount: 0 }];
        let w = expect_word(encode_add_sub(&ops, false, set_flags));
        prop_assert_eq!(opcode5_of(w), 0b01011);
        prop_assert_eq!(ext21_of(w), 1);             // extended register
        prop_assert_eq!(option_of(w), want_opt);
        prop_assert_eq!(imm3_of(w), 0);
        prop_assert_eq!(s_of(w), if set_flags { 1 } else { 0 });
    }

    // BUG WITNESS (#[ignore]): the extend-kind match has a catch-all
    // `_ => 0b011` (UXTX), so an unrecognized mnemonic (typo / fabricated name)
    // is silently encoded as UXTX instead of being rejected. clang rejects
    // `add x0, x1, x2, foo` with "expected '[su]xt[bhw]' … in range [0, 4]".
    #[ignore = "documented bug: add/sub extended-reg maps unknown extend kind to UXTX (clang rejects)"]
    #[test]
    fn addsub_extended_reg_rejects_unknown_extend_kind(
        n in 0u32..=30,
        kind in "[a-z]{1,6}",              // arbitrary lowercase strings
    ) {
        // skip the eight valid extend mnemonics (and "lsl")
        prop_assume!(!matches!(kind.as_str(),
            "uxtb" | "uxth" | "uxtw" | "uxtx" |
            "sxtb" | "sxth" | "sxtw" | "sxtx" | "lsl"));
        let ops = vec![xreg(n), xreg(n), xreg(n),
                       Operand::Extend { kind: kind.clone(), amount: 0 }];
        prop_assert!(
            encode_add_sub(&ops, false, false).is_err(),
            "unknown extend kind `{}` must be rejected; got {:?}",
            kind, encode_add_sub(&ops, false, false),
        );
    }

    // BUG WITNESS (#[ignore]): the optional extend shift is masked with
    // `& 0x7`. Values 5..=7 are architecturally RESERVED for the imm3 field of
    // the add/sub extended-register form (the permitted range is 0..=4), yet
    // the encoder silently emits them. clang rejects `add x0, x1, x2, uxtw #5`.
    #[ignore = "documented bug: add/sub extended-reg accepts reserved extend shift 5..7 (clang rejects)"]
    #[test]
    fn addsub_extended_reg_rejects_reserved_shift_5_to_7(
        n in 0u32..=30,
        amount in 5u32..=7u32,
        ek in 0u32..=7u32,
    ) {
        let kind = match ek {
            0 => "uxtb", 1 => "uxth", 2 => "uxtw", 3 => "uxtx",
            4 => "sxtb", 5 => "sxth", 6 => "sxtw", _ => "sxtx",
        };
        let ops = vec![xreg(n), xreg(n), xreg(n),
                       Operand::Extend { kind: kind.into(), amount }];
        prop_assert!(
            encode_add_sub(&ops, false, false).is_err(),
            "`add x{}, x{}, x{}, {} #{}` uses a reserved imm3 and must be rejected; got {:?}",
            n, n, n, kind, amount, encode_add_sub(&ops, false, false),
        );
    }

    // BUG WITNESS (#[ignore]): extend shift amounts ≥ 8 are silently truncated
    // by `& 0x7` (e.g. #8 → 0, #9 → 1). The valid range is 0..=4, so these are
    // out of range and MUST be rejected. clang rejects `add x0, x1, x2, uxtw #8`.
    #[ignore = "documented bug: add/sub extended-reg silently truncates extend shift ≥8 (clang rejects)"]
    #[test]
    fn addsub_extended_reg_rejects_truncated_shift_above_7(
        n in 0u32..=30,
        amount in 8u32..=63u32,
        ek in 0u32..=7u32,
    ) {
        let kind = match ek {
            0 => "uxtb", 1 => "uxth", 2 => "uxtw", 3 => "uxtx",
            4 => "sxtb", 5 => "sxth", 6 => "sxtw", _ => "sxtx",
        };
        let ops = vec![xreg(n), xreg(n), xreg(n),
                       Operand::Extend { kind: kind.into(), amount }];
        prop_assert!(
            encode_add_sub(&ops, false, false).is_err(),
            "`add x{}, x{}, x{}, {} #{}` is out of range (>4) and must be rejected; got {:?}",
            n, n, n, kind, amount, encode_add_sub(&ops, false, false),
        );
    }
}

// =========================================================================
//  encode_div — register-width validation
// =========================================================================
proptest! {
    // Positive: uniform-width SDIV/UDIV succeeds; sf tracks the (uniform)
    // width, the fixed bits are correct, and the o1 bit selects signed (1)
    // vs unsigned (0). This is the green baseline the witness below violates.
    #[test]
    fn div_uniform_width_fields(
        n in 0u32..=30,
        is_64 in any::<bool>(),
        unsigned in any::<bool>(),
    ) {
        let r = if is_64 { xreg(n) } else { wreg(n) };
        let ops = vec![r.clone(), r.clone(), r];
        let w = expect_word(encode_div(&ops, unsigned));
        prop_assert_eq!(sf_of(w), if is_64 { 1 } else { 0 });
        prop_assert_eq!((w >> 21) & 0x3FF, 0b0011010110); // fixed bits
        prop_assert_eq!(div_o1_of(w), if unsigned { 0 } else { 1 });
        prop_assert_eq!(rm_of(w), n);
        prop_assert_eq!(rn_of(w), n);
        prop_assert_eq!(rd_of(w), n);
    }

    // BUG WITNESS (#[ignore]): encode_div derives sf from the destination only
    // and binds the source is_64 flags to `_`, so a mixed-width spelling such
    // as `sdiv x0, w1, w2` or `udiv w0, x1, x2` is silently emitted with the
    // destination's width. ARMv8 requires <Rd>,<Rn>,<Rm> to share one width;
    // clang rejects both forms with "invalid operand for instruction".
    #[test]
    fn div_rejects_mixed_register_widths(
        n in 0u32..=30,
        unsigned in any::<bool>(),
        dest_is_x in any::<bool>(),
    ) {
        let (rd, rn, rm) = if dest_is_x {
            (xreg(n), wreg(n), wreg(n)) // e.g. sdiv xN, wN, wN
        } else {
            (wreg(n), xreg(n), xreg(n)) // e.g. udiv wN, xN, xN
        };
        let ops = vec![rd, rn, rm];
        prop_assert!(
            encode_div(&ops, unsigned).is_err(),
            "mixed-width `{}div` operands must be rejected; got {:?}",
            if unsigned { "u" } else { "s" }, encode_div(&ops, unsigned),
        );
    }
}

// =========================================================================
//  encode_bitmask_imm — reference / differential / round-trip (all green)
// =========================================================================
proptest! {
    // Determinism: identical inputs yield identical outputs.
    #[test]
    fn bitmask_is_deterministic(
        v in any::<u64>(),
        is_64 in any::<bool>(),
    ) {
        prop_assert_eq!(encode_bitmask_imm(v, is_64), encode_bitmask_imm(v, is_64));
    }

    // Degenerate rejection: 0 and the all-ones-of-width value have no valid
    // logical-immediate encoding and must return None.
    #[test]
    fn bitmask_rejects_zero_and_all_ones(
        is_64 in any::<bool>(),
    ) {
        prop_assert_eq!(encode_bitmask_imm(0, is_64), None);
        let all_ones = if is_64 { u64::MAX } else { 0xFFFF_FFFF };
        prop_assert_eq!(encode_bitmask_imm(all_ones, is_64), None);
    }

    // Field ranges: every accepted output has N ∈ {0,1}, immr < 64, imms < 64.
    #[test]
    fn bitmask_output_fields_in_range(
        v in any::<u64>(),
        is_64 in any::<bool>(),
    ) {
        if let Some((n, immr, imms)) = encode_bitmask_imm(v, is_64) {
            prop_assert!(n <= 1, "N out of range: {}", n);
            prop_assert!(immr < 64, "immr out of range: {}", immr);
            prop_assert!(imms < 64, "imms out of range: {}", imms);
        }
    }

    // Round-trip: for any valid bitmask value, decoding the encoder's
    // (N, immr, imms) reproduces the input value. The generator constructs a
    // known-valid value directly (element = ROR(Ones(ones), rotation),
    // replicated to the register width), so encode_bitmask_imm must accept it
    // and decode_bitmask must recover it exactly.
    #[test]
    fn bitmask_roundtrip_decodes_to_input(
        is_64 in any::<bool>(),
        size_log in 1u32..=6u32,      // element size 2,4,8,16,32,64
        ones in 1u32..=63u32,         // run length (constrained below)
        rotation in 0u32..=63u32,
    ) {
        let size = 1u32 << size_log;
        let datasize = if is_64 { 64 } else { 32 };
        prop_assume!(size <= datasize);            // size 64 needs is_64
        prop_assume!(ones >= 1 && ones < size);    // 1..=size-1 set bits
        let rotation = rotation % size;
        let elem_mask = mask_bits(size);
        let welem = (1u64 << ones) - 1;
        let r = rotation as u64;
        let rot = if r == 0 {
            welem & elem_mask
        } else {
            ((welem >> r) | (welem << (size as u64 - r))) & elem_mask
        };
        let mut v = 0u64;
        let mut i = 0u64;
        while i < datasize as u64 {
            v |= rot << i;
            i += size as u64;
        }
        let v = v & mask_bits(datasize);

        let enc = encode_bitmask_imm(v, is_64);
        prop_assert!(enc.is_some(), "v={:#x} is_64={} should encode, got None", v, is_64);
        let (n, immr, imms) = enc.unwrap();
        let decoded = decode_bitmask(n, immr, imms, datasize);
        prop_assert_eq!(decoded, Some(v),
            "round-trip failed for v={:#x} is_64={}: (N,immr,imms)=({},{},{}) decoded to {:?}",
            v, is_64, n, immr, imms, decoded);
    }
}

// Differential reference: 10 (value, is_64) → (N, immr, imms) triples produced
// by `clang --target=aarch64-linux-gnu -c` on `orr Rd, RZR, #imm` (the ORR
// logical-immediate encoding reuses encode_bitmask_imm via encode_logical /
// encode_mov). The encoder must reproduce clang's exact fields.
#[test]
fn bitmask_matches_clang_reference() {
    // (value, is_64, expected N, immr, imms)
    let vectors: &[(u64, bool, u32, u32, u32)] = &[
        (0x5555_5555_5555_5555, true, 0, 0, 60), // orr x0,xzr,#0x5555...5555
        (0xaaaa_aaaa_aaaa_aaaa, true, 0, 1, 60), // orr x0,xzr,#0xaaaa...aaaa
        (0x0001_0001_0001_0001, true, 0, 0, 32), // orr x0,xzr,#0x0001000100010001
        (0x0000_0000_ffff_ffff, true, 1, 0, 31), // orr x0,xzr,#0x00000000ffffffff
        (0xffff_0000_ffff_0000, true, 0, 16, 15), // orr x0,xzr,#0xffff0000ffff0000
        (0x0003_ffff_ffff_ffff, true, 1, 0, 49), // orr x0,xzr,#0x0003ffffffffffff
        (0x0000_0000_0000_ffff, false, 0, 0, 15), // orr w0,wzr,#0x0000ffff
        (0x0000_0000_5555_5555, false, 0, 0, 60), // orr w0,wzr,#0x55555555
        (0x0000_0000_00ff_00ff, false, 0, 0, 39), // orr w0,wzr,#0x00ff00ff
        (0x0000_0000_aaaa_aaaa, false, 0, 1, 60), // orr w0,wzr,#0xaaaaaaaa
    ];
    for &(v, is_64, n, immr, imms) in vectors {
        assert_eq!(
            encode_bitmask_imm(v, is_64),
            Some((n, immr, imms)),
            "encode_bitmask_imm({:#018x}, {}) mismatch (clang: N={} immr={} imms={})",
            v,
            is_64,
            n,
            immr,
            imms,
        );
        // cross-check the reference decoder reproduces the value
        let datasize = if is_64 { 64 } else { 32 };
        assert_eq!(
            decode_bitmask(n, immr, imms, datasize),
            Some(v),
            "decode_bitmask({},{},{},{}) != {:#018x}",
            n,
            immr,
            imms,
            datasize,
            v,
        );
    }
}
