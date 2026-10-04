//! Property-based tests for the **16-bit element path** (`.4h`/`.8h`) and the
//! `abc`/`defgh` immediate-splitting logic of `encode_neon_movi`.
//!
//! Scope (see `neon_movi_pbt.rs` for the five already-reported findings on the
//! `.8b`/`.16b`, `.2s`/`.4s` (MSL + invalid-kind), out-of-range-immediate, and
//! `.2d` paths): this module zeroes in on the parts the user flagged that are
//! *not* already covered:
//!   * the `.4h`/`.8h` 16-bit branch, and
//!   * the `abc`/`defgh` immediate splitting (bits [18:16] / [9:5]).
//!
//! ## Encoding layout (MOVI, `Advanced SIMD modified immediate`)
//! ```text
//!   31 30 29 28-24 23-22 21-19 18-16 15-12 11-10 9-5 4-0
//!    0  Q  op  01111  00   000   abc   cmode   01  defgh Rd
//! ```
//! For the 16-bit form, `op = 0`, `cmode = 0b1000`, and `abc:defgh = imm8`.
//!
//! ## Oracle
//! The independent `ref_encode_16bit` mirrors the documented layout directly.
//! It is cross-checked against the clang-derived golden words for `.4h`/`.8h`
//! in `neon_movi_pbt.rs` (`0x0F0286A2` for `movi v2.4h,#0x55`,
//! `0x4F048406` for `movi v6.8h,#0x80`).
//!
//! ## Additional finding (documented by the `#[ignore]`d test at the bottom)
//!  6. **`.4h`/`.8h` silently accept (and drop) a shift operand.** MOVI's
//!     16-bit element forms have *only* the unshifted encoding (cmode=1000):
//!     the ARMv8-A ARM defines no LSL/MSL shift variant for 16-bit elements.
//!     The encoder's `.4h`/`.8h` branch never inspects `operands[2]`, so
//!     `movi v0.4h, #imm, lsl #8` (and `msl #8/#16`) returns `Ok` and emits
//!     the no-shift word, silently changing the instruction's meaning. This is
//!     distinct from the known findings, which all live on the `.2s`/`.4s`
//!     (32-bit) arm. See `movi_16bit_silently_accepts_shift_operand`.
//!
//! The `abc`/`defgh` splitting itself is **correct** — the green property
//! `movi_16bit_abc_defgh_roundtrip` reconstructs `imm8` from the emitted word
//! for every value in `0..=255`, on both `.4h` and `.8h`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_movi;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

fn va(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

fn imm8_strategy() -> impl Strategy<Value = u32> {
    0u32..=255u32
}

fn arr16_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("4h"), Just("8h")]
}

/// Independent reference encoder for the 16-bit MOVI form. Assembles the word
/// field-by-field from the documented layout. NOT a copy of the impl's
/// expression shapes (`abc` is placed with a single shift, not bit-by-bit).
fn ref_encode_16bit(rd: u32, arr: &str, imm8: u32) -> u32 {
    let q: u32 = match arr {
        "4h" => 0,
        "8h" => 1,
        _ => unreachable!("invalid arr in reference: {arr}"),
    };
    let abc = (imm8 >> 5) & 0x7;
    let defgh = imm8 & 0x1F;
    (q << 30) // op = 0 for MOVI (bit 29 clear)
        | (0b0111100u32 << 22) // bits 28-24 = 01111, bit 23 = 0, bit 22 = 0
        | (abc << 16) // bits 18-16
        | (0b1000u32 << 12) // cmode = 1000 for the 16-bit form
        | (0b01u32 << 10) // bits 11-10 = 01
        | (defgh << 5) // bits 9-5
        | rd
}

fn ops_no_shift(rd: u32, arr: &str, imm8: u32) -> Vec<Operand> {
    vec![va(rd, arr), Operand::Imm(imm8 as i64)]
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden cross-check: reference agrees with clang-derived golden words ---

#[test]
fn reference_matches_clang_golden_16bit() {
    // From neon_movi_pbt.rs GOLDEN table (clang-validated).
    assert_eq!(ref_encode_16bit(2, "4h", 0x55), 0x0F0286A2); // movi v2.4h, #0x55
    assert_eq!(ref_encode_16bit(6, "8h", 0x80), 0x4F048406); // movi v6.8h, #0x80
    assert_eq!(ref_encode_16bit(0, "4h", 0x00), 0x0F008400); // movi v0.4h, #0
}

// --- properties (default `cargo test` stays green) ------------------------

proptest! {
    // === Oracle: differential against independent reference encoder ========
    // For every .4h/.8h arrangement, register, and 8-bit immediate, the impl
    // must equal the independently-assembled reference word.
    #[test]
    fn movi_16bit_matches_reference_encoder(
        rd in reg_num_strategy(),
        imm8 in imm8_strategy(),
        arr in arr16_strategy(),
    ) {
        let ops = ops_no_shift(rd, arr, imm8);
        let got = word_of(encode_neon_movi(&ops));
        let want = ref_encode_16bit(rd, arr, imm8);
        prop_assert_eq!(got, want, "movi v{}.{}#{:x}", rd, arr, imm8);
    }

    // === abc/defgh immediate-splitting round-trip (the user's focus) =======
    // The emitted word must carry `imm8` split into abc (bits [18:16]) and
    // defgh (bits [9:5]) such that `(abc << 5) | defgh == imm8` for *every*
    // 8-bit value, on both 16-bit arrangements. This pins the splitting logic
    // down and documents that it is sound (no bug here).
    #[test]
    fn movi_16bit_abc_defgh_roundtrip(
        rd in reg_num_strategy(),
        imm8 in imm8_strategy(),
        arr in arr16_strategy(),
    ) {
        let ops = ops_no_shift(rd, arr, imm8);
        let w = word_of(encode_neon_movi(&ops));

        let expect_abc = (imm8 >> 5) & 0x7;
        let expect_defgh = imm8 & 0x1F;

        let got_abc = (w >> 16) & 0x7;
        let got_defgh = (w >> 5) & 0x1F;

        prop_assert_eq!(got_abc, expect_abc, "abc field for imm8={:#04x}", imm8);
        prop_assert_eq!(got_defgh, expect_defgh, "defgh field for imm8={:#04x}", imm8);

        // The two fields must reconstruct the full 8-bit immediate.
        prop_assert_eq!(
            ((got_abc << 5) | got_defgh) & 0xFF,
            imm8,
            "abc:defgh must reconstruct imm8",
        );

        // ... and must NOT bleed into neighbouring fields.
        prop_assert_eq!(w & 0x1F, rd, "Rd field must equal the register");
        prop_assert_eq!((w >> 12) & 0xF, 0b1000, "cmode must be 1000 for 16-bit");
    }

    // === Fixed-bits + field-placement invariant ===========================
    #[test]
    fn movi_16bit_fixed_bits_and_field_layout(
        rd in reg_num_strategy(),
        imm8 in imm8_strategy(),
        arr in arr16_strategy(),
    ) {
        let ops = ops_no_shift(rd, arr, imm8);
        let w = word_of(encode_neon_movi(&ops));

        // --- architecturally-constant fields ---
        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 0, "op bit (29) must be 0 for MOVI");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01111, "bits 28-24 = 01111");
        prop_assert_eq!((w >> 22) & 0x3, 0b00, "bits 23-22 = 00");
        prop_assert_eq!((w >> 10) & 0x3, 0b01, "bits 11-10 = 01");

        // --- Q tracks the arrangement ---
        let expect_q: u32 = if arr == "8h" { 1 } else { 0 };
        prop_assert_eq!((w >> 30) & 1, expect_q, "Q bit must match arrangement");

        // --- cmode is the 16-bit form ---
        prop_assert_eq!((w >> 12) & 0xF, 0b1000, "cmode = 1000 for .4h/.8h");
    }

    // === Determinism ======================================================
    #[test]
    fn movi_16bit_is_deterministic(
        rd in reg_num_strategy(),
        imm8 in imm8_strategy(),
        arr in arr16_strategy(),
    ) {
        let ops = ops_no_shift(rd, arr, imm8);
        let a = word_of(encode_neon_movi(&ops));
        let b = word_of(encode_neon_movi(&ops));
        prop_assert_eq!(a, b);
    }
}

// --- documented finding 6: `.4h`/`.8h` silently accept a shift operand -----

/// MOVI's 16-bit element forms (`.4h`/`.8h`) have *only* the unshifted
/// encoding (cmode=1000): the ARMv8-A ARM defines no LSL/MSL shift variant for
/// 16-bit elements (the LSL/MSL shift forms exist only for the 32-bit element
/// forms `.2s`/`.4s`). So `movi v0.4h, #imm, lsl #8`, `msl #8`, and
/// `msl #16` must be rejected.
///
/// The implementation's `.4h`/`.8h` branch never inspects `operands[2]`, so it
/// returns `Ok` and emits the no-shift word (cmode=1000), silently dropping
/// the shift and changing the instruction's meaning.
///
/// This is distinct from the known findings: those live on the `.2s`/`.4s`
/// (32-bit) arm (finding 2 = MSL dropped; finding 3 = invalid shift *kind*
/// `lsr`/`asr`/`ror`). Here the shift *kind* is valid (`lsl`/`msl`) but the
/// *arrangement* (`.4h`/`.8h`) admits no shift at all.
///
/// `#[ignore]`d so the default `cargo test` stays green.
/// Run: `cargo test -- --ignored movi_16bit_silently_accepts_shift_operand`
#[test]
#[ignore]
fn movi_16bit_silently_accepts_shift_operand() {
    for &(arr, kind, amt) in &[
        ("4h", "lsl", 8u32),
        ("4h", "msl", 8u32),
        ("4h", "msl", 16u32),
        ("8h", "lsl", 8u32),
        ("8h", "msl", 8u32),
        ("8h", "msl", 16u32),
    ] {
        let ops = vec![
            va(0, arr),
            Operand::Imm(0x2A),
            Operand::Shift {
                kind: kind.to_string(),
                amount: amt,
            },
        ];
        let res = encode_neon_movi(&ops);
        let desc = match &res {
            Ok(EncodeResult::Word(w)) => format!("Ok(0x{w:08X})"),
            other => format!("{other:?}"),
        };
        // Confirm the witness: a real bug returns Ok with the *no-shift* word.
        if let Ok(EncodeResult::Word(w)) = &res {
            let cmode = (w >> 12) & 0xF;
            assert_eq!(
                cmode, 0b1000,
                "movi v0.{arr}, #0x2a, {kind} #{amt}: shift was dropped — emitted \
                 no-shift cmode=1000 ({desc}); 16-bit MOVI admits no shift and must Err",
            );
        }
        assert!(
            res.is_err(),
            "movi v0.{arr}, #0x2a, {kind} #{amt}: 16-bit MOVI has no shift form; \
             expected Err, got {desc} (shift silently dropped)",
        );
    }
}
