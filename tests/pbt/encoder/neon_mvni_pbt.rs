//! Property-based tests for `encode_neon_mvni`.
//!
//! `encode_neon_mvni` encodes the AArch64 NEON `MVNI` instruction
//! (move inverted immediate) — `MVNI Vd.T, #imm8{, shift}` — in the
//! "Advanced SIMD modified immediate" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23 22-19 18-16 15-12 11 10 9-5 4-0
//!    0  Q  U  01111 0  0000  abc   cmode  0  1  defgh Rd
//! ```
//! with `U = 1` (bit 29) distinguishing MVNI from MOVI (`U = 0`).
//! `Q` comes from the arrangement; `abc:defgh` is the 8-bit immediate;
//! `cmode` encodes the element size and optional LSL/MSL shift.
//!
//! Valid arrangements (ARMv8-A ARM, "MVNI (vector)"): `2S`, `4S`, `4H`, `8H`.
//! Allowed shifts:
//!   - `.2s`/`.4s`: none, `lsl #{0,8,16,24}`, `msl #{8,16}`
//!   - `.4h`/`.8h`: none only (16-bit form has cmode=1000, no shift)
//!
//! ## Oracle
//! The golden words were assembled by clang's integrated assembler
//! (`clang --target=aarch64-linux-gnu`) — an independent reference — and
//! cross-checked by hand against the ARMv8-A ARM bit layout. The
//! independent `ref_encode_mvni` mirrors the documented layout directly.
//!
//! ## Findings (documented by the `#[ignore]`d tests at the bottom)
//!  1. **Invalid shift kinds silently accepted.** `mvni v0.4s, #5, lsr #8`
//!     (also `asr`/`ror`) is rejected by clang/ARM ARM, but the encoder
//!     falls through to `cmode = 0000` and returns `Ok` — the instruction
//!     silently changes meaning. See `mvni_rejects_invalid_shift_kind`.
//!  2. **MSL shift on 16-bit elements silently dropped.** `mvni v6.4h, #0x80,
//!     msl #8` is rejected by clang/ARM ARM (MSL is 32-bit only), but the
//!     encoder's `.4h/.8h` branch ignores the shift operand entirely and
//!     returns the no-shift 16-bit word. See `mvni_rejects_msl_on_16bit`.
//!  3. **Out-of-range immediate silently truncated.** `mvni v0.4s, #256`
//!     (also `#-1`, `#0x1ff`) is rejected by clang with "immediate must be
//!     an integer in range [0, 255]", but the encoder does `imm & 0xFF` and
//!     emits the masked word. See `mvni_rejects_out_of_range_immediate`.
//!     See `pbt-out/REPORT.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_mvni;
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

/// Architecturally valid (arrangement, shift) combinations for MVNI.
/// The shift is `None` or `(kind, amount)`.
fn valid_arr_shift_strategy() -> impl Strategy<Value = (&'static str, Option<(&'static str, u32)>)>
{
    let shift_32 = prop_oneof![
        Just(None),
        Just(Some(("lsl", 0u32))),
        Just(Some(("lsl", 8u32))),
        Just(Some(("lsl", 16u32))),
        Just(Some(("lsl", 24u32))),
        Just(Some(("msl", 8u32))),
        Just(Some(("msl", 16u32))),
    ];
    prop_oneof![
        (Just("2s"), shift_32.clone()),
        (Just("4s"), shift_32),
        (Just("4h"), Just(None)),
        (Just("8h"), Just(None)),
    ]
}

/// Independent reference encoder: assembles the word field-by-field from the
/// documented ARMv8-A ARM layout (NOT a copy of the implementation's
/// expression shapes — it dispatches shifts via an explicit match).
fn ref_encode_mvni(rd: u32, arr: &str, imm8: u32, shift: Option<(&str, u32)>) -> u32 {
    let q: u32 = match arr {
        "2s" => 0,
        "4s" | "8h" => 1,
        "4h" => 0,
        _ => unreachable!("invalid arr in reference: {arr}"),
    };
    let abc = (imm8 >> 5) & 0x7;
    let defgh = imm8 & 0x1F;
    let cmode: u32 = match arr {
        "2s" | "4s" => match shift {
            None | Some(("lsl", 0)) => 0b0000,
            Some(("lsl", 8)) => 0b0010,
            Some(("lsl", 16)) => 0b0100,
            Some(("lsl", 24)) => 0b0110,
            Some(("msl", 8)) => 0b1100,
            Some(("msl", 16)) => 0b1101,
            _ => unreachable!("invalid shift in reference: {shift:?}"),
        },
        "4h" | "8h" => 0b1000,
        _ => unreachable!(),
    };
    (q << 30)
        | (1u32 << 29) // U = 1 for MVNI
        | (0b0111100u32 << 22) // bits 28-24 = 01111, bit 23 = 0, bit 22 = 0
        | (abc << 16)
        | (cmode << 12)
        | (0b01u32 << 10) // bits 11-10
        | (defgh << 5)
        | rd
}

fn ops_for(rd: u32, arr: &str, imm8: u32, shift: Option<(&str, u32)>) -> Vec<Operand> {
    let mut v = vec![va(rd, arr), Operand::Imm(imm8 as i64)];
    if let Some((kind, amt)) = shift {
        v.push(Operand::Shift {
            kind: kind.to_string(),
            amount: amt,
        });
    }
    v
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle, cross-checked against clang) ----------

/// Each expected word was produced by clang's integrated assembler
/// (`clang --target=aarch64-linux-gnu`) and re-derived by hand from the
/// ARMv8-A ARM layout.  `(Rd, arrangement, imm8, Option<(shift_kind, amount)>, word)`
#[rustfmt::skip]
type GoldenRow = (u32, &'static str, u32, Option<(&'static str, u32)>, u32);

const GOLDEN: &[GoldenRow] = &[
    (0, "4s", 0x00, None, 0x6F000400),              // mvni v0.4s, #0
    (5, "4s", 0x1F, None, 0x6F0007E5),              // mvni v5.4s, #0x1f
    (0, "2s", 0xFF, None, 0x2F0707E0),              // mvni v0.2s, #0xff
    (3, "4s", 0x20, Some(("lsl", 8)), 0x6F012403),  // mvni v3.4s, #0x20, lsl #8
    (2, "4h", 0x55, None, 0x2F0286A2),              // mvni v2.4h, #0x55
    (1, "4s", 0xA5, Some(("lsl", 16)), 0x6F0544A1), // mvni v1.4s, #0xa5, lsl #16
    (4, "4s", 0x3C, Some(("lsl", 24)), 0x6F016784), // mvni v4.4s, #0x3c, lsl #24
    (9, "4s", 0x2A, Some(("msl", 8)), 0x6F01C549),  // mvni v9.4s, #0x2a, msl #8
    (6, "8h", 0x80, None, 0x6F048406),              // mvni v6.8h, #0x80
    (0, "2s", 0x01, None, 0x2F000420),              // mvni v0.2s, #1
    (8, "4s", 0x00, Some(("msl", 16)), 0x6F00D408), // mvni v8.4s, #0, msl #16
];

#[test]
fn mvni_matches_golden_table() {
    for &(rd, arr, imm8, shift, expected) in GOLDEN {
        let ops = ops_for(rd, arr, imm8, shift);
        let got = word_of(encode_neon_mvni(&ops));
        assert_eq!(
            got,
            expected,
            "mvni v{rd}.{arr}, #{imm8:#x}{}: got 0x{got:08X}, want 0x{expected:08X}",
            shift
                .map(|(k, a)| format!(", {k} #{a}"))
                .unwrap_or_default(),
        );
        // The independent reference must agree with the golden value too.
        assert_eq!(
            ref_encode_mvni(rd, arr, imm8, shift),
            expected,
            "reference encoder drift for ({rd}, {arr}, {imm8:#x}, {shift:?})",
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential against independent reference encoder ========
    // For every valid (arrangement, shift) and register/immediate pair, the
    // implementation must equal the independently-assembled reference word.
    #[test]
    fn mvni_matches_reference_encoder(
        rd in reg_num_strategy(),
        imm8 in imm8_strategy(),
        (arr, shift) in valid_arr_shift_strategy(),
    ) {
        let ops = ops_for(rd, arr, imm8, shift);
        let got = word_of(encode_neon_mvni(&ops));
        let want = ref_encode_mvni(rd, arr, imm8, shift);
        prop_assert_eq!(got, want);
    }

    // === Fixed-bits + field-placement invariant ===========================
    // Architecturally-constant bits never vary, and Rd/abc/defgh/cmode/Q
    // round-trip into their documented fields.
    #[test]
    fn mvni_fixed_bits_and_field_layout(
        rd in reg_num_strategy(),
        imm8 in imm8_strategy(),
        (arr, shift) in valid_arr_shift_strategy(),
    ) {
        let ops = ops_for(rd, arr, imm8, shift);
        let w = word_of(encode_neon_mvni(&ops));

        // --- constant fields ---
        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 1, "U bit (29) must be 1 for MVNI");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01111, "bits 28-24 = 01111");
        prop_assert_eq!((w >> 22) & 0x3, 0b00, "bits 23-22 = 00");
        prop_assert_eq!((w >> 10) & 0x3, 0b01, "bits 11-10 = 01");

        // --- round-tripped fields ---
        let abc = (imm8 >> 5) & 0x7;
        let defgh = imm8 & 0x1F;
        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, defgh, "defgh field");
        prop_assert_eq!((w >> 16) & 0x7, abc, "abc field");

        let expect_q: u32 = match arr { "2s" | "4h" => 0, _ => 1 };
        prop_assert_eq!((w >> 30) & 1, expect_q, "Q bit must match arrangement");

        let expect_cmode = ref_cmode(arr, shift);
        prop_assert_eq!((w >> 12) & 0xF, expect_cmode, "cmode field must match arrangement/shift");
    }

    // === Negative contract: unsupported arrangement rejected ==============
    // Any arrangement other than 2s/4s/4h/8h must be an Err.
    #[test]
    fn mvni_rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,3}".prop_filter("must be unknown to MVNI", |s| {
            !matches!(s.as_str(), "2s"|"4s"|"4h"|"8h")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), Operand::Imm(0)];
        prop_assert!(encode_neon_mvni(&ops).is_err(),
            "MVNI does not support .{arr}; expected Err");
    }

    // === Negative contract: out-of-range immediate rejected (FAILING) =====
    // MVNI's immediate must be 0x00..=0xFF (clang: "immediate must be an
    // integer in range [0, 255]"). The implementation masks with `& 0xFF`,
    // so e.g. #256 -> #0 and #-1 -> #0xFF silently. Currently FAILS.
    #[test]
    #[ignore]
    fn mvni_rejects_out_of_range_immediate(
        rd in reg_num_strategy(),
        imm in (256i64..=0xFFFFi64),
        arr in prop_oneof![Just("2s"), Just("4s"), Just("4h"), Just("8h")],
    ) {
        let ops = vec![va(rd, arr), Operand::Imm(imm)];
        prop_assert!(encode_neon_mvni(&ops).is_err(),
            "MVNI immediate {imm:#x} is out of range [0,255]; expected Err");
    }

    // === Negative contract: negative immediate rejected (FAILING) =========
    // clang rejects `mvni v0.4s, #-1` (range [0,255]). The implementation's
    // `imm as u32 & 0xFF` turns -1 into 0xFF silently. Currently FAILS.
    #[test]
    #[ignore]
    fn mvni_rejects_negative_immediate(
        rd in reg_num_strategy(),
        imm in (-256i64..=-1i64),
        arr in prop_oneof![Just("2s"), Just("4s"), Just("4h"), Just("8h")],
    ) {
        let ops = vec![va(rd, arr), Operand::Imm(imm)];
        prop_assert!(encode_neon_mvni(&ops).is_err(),
            "MVNI does not accept negative immediate {imm}; expected Err");
    }

    // === Negative contract: invalid shift amounts rejected ================
    // LSL amounts outside {0,8,16,24} and MSL amounts outside {8,16} must
    // be Err for the 32-bit element form.
    #[test]
    fn mvni_rejects_invalid_shift_amount(
        kind in prop_oneof![Just("lsl"), Just("msl")],
        bad_amt in 0u32..32u32,
    ) {
        // only test genuinely-bad amounts
        prop_assume!(!matches!((kind, bad_amt),
            ("lsl", 0) | ("lsl", 8) | ("lsl", 16) | ("lsl", 24) |
            ("msl", 8) | ("msl", 16)));
        let ops = vec![va(0, "4s"), Operand::Imm(0),
                       Operand::Shift { kind: kind.to_string(), amount: bad_amt }];
        prop_assert!(encode_neon_mvni(&ops).is_err(),
            "unsupported shift kind/amount should be rejected");
    }
}

/// cmode expected by the reference for a (arrangement, shift) — used by the
/// field-layout property to assert cmode placement independently of the impl.
fn ref_cmode(arr: &str, shift: Option<(&str, u32)>) -> u32 {
    match arr {
        "2s" | "4s" => match shift {
            None | Some(("lsl", 0)) => 0b0000,
            Some(("lsl", 8)) => 0b0010,
            Some(("lsl", 16)) => 0b0100,
            Some(("lsl", 24)) => 0b0110,
            Some(("msl", 8)) => 0b1100,
            Some(("msl", 16)) => 0b1101,
            _ => 0,
        },
        "4h" | "8h" => 0b1000,
        _ => 0,
    }
}

// --- documented finding 1: invalid shift kinds silently accepted -----------

/// `lsr`/`asr`/`ror` are not valid shift operators for MVNI. clang rejects
/// `mvni v0.4s, #5, lsr #8` (also `asr`, `ror`); the ARMv8-A ARM allows only
/// LSL and MSL. The implementation, however, falls through the `else` branch
/// to `cmode = 0b0000` and returns `Ok`, silently turning the instruction
/// into the no-shift form.
///
/// `#[ignore]`d because it documents an unfixed contract gap.
/// Run: `cargo test -- --ignored mvni_rejects_invalid_shift_kind`
#[test]
#[ignore]
fn mvni_rejects_invalid_shift_kind() {
    for kind in &["lsr", "asr", "ror"] {
        let ops = vec![
            va(0, "4s"),
            Operand::Imm(5),
            Operand::Shift {
                kind: kind.to_string(),
                amount: 8,
            },
        ];
        let res = encode_neon_mvni(&ops);
        assert!(
            res.is_err(),
            "{kind} is not a valid MVNI shift; expected Err but got Ok(0x{:08X})",
            res.as_ref()
                .ok()
                .map(|e| match e {
                    EncodeResult::Word(w) => *w,
                    _ => 0,
                })
                .unwrap_or(0),
        );
    }
}

// --- documented finding 2: MSL shift on 16-bit elements silently dropped ---

/// The MSL (multiply-shift-left) shift is defined ONLY for the 32-bit element
/// form of MVNI (`.2s`/`.4s`). clang rejects `mvni v6.4h, #0x80, msl #8`.
/// The implementation's `.4h`/`.8h` branch ignores `operands.get(2)` entirely
/// and hardcodes `cmode = 1000`, so it returns `Ok` with the no-shift 16-bit
/// word, silently dropping the MSL.
///
/// `#[ignore]`d because it documents an unfixed contract gap.
/// Run: `cargo test -- --ignored mvni_rejects_msl_on_16bit`
#[test]
#[ignore]
fn mvni_rejects_msl_on_16bit() {
    for arr in &["4h", "8h"] {
        let ops = vec![
            va(6, arr),
            Operand::Imm(0x80),
            Operand::Shift {
                kind: "msl".to_string(),
                amount: 8,
            },
        ];
        let res = encode_neon_mvni(&ops);
        assert!(
            res.is_err(),
            "MSL is not valid for MVNI .{arr}; expected Err but got Ok(0x{:08X})",
            res.as_ref()
                .ok()
                .map(|e| match e {
                    EncodeResult::Word(w) => *w,
                    _ => 0,
                })
                .unwrap_or(0),
        );
    }
}
