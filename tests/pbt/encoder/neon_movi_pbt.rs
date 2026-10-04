//! Property-based tests for `encode_neon_movi`.
//!
//! `encode_neon_movi` encodes the AArch64 NEON `MOVI` instruction
//! (move immediate to vector) — `MOVI Vd.T, #imm8{, shift}` — in the
//! "Advanced SIMD modified immediate" encoding group, the same family as
//! `MVNI`:
//!
//! ```text
//!   31 30 29 28-24 23 22-19 18-16 15-12 11 10 9-5 4-0
//!    0  Q  op 01111 0  0000  abc   cmode  0  1  defgh Rd
//! ```
//! with `op = 0` (bit 29) distinguishing MOVI from MVNI (`op = 1`).
//! `Q` comes from the arrangement; `abc:defgh` is the 8-bit immediate;
//! `cmode` encodes the element size and optional LSL shift.
//!
//! Valid arrangements (ARMv8-A ARM, "MOVI (vector)"): `8B`, `16B`, `2S`,
//! `4S`, `4H`, `8H`, `2D`. Allowed shifts:
//!   - `.8b`/`.16b`/`.2d`: none (byte / 64-bit forms use fixed cmode)
//!   - `.2s`/`.4s`: none, `lsl #{0,8,16,24}`, `msl #{8,16}`
//!   - `.4h`/`.8h`: none only (16-bit form has cmode=1000, no shift)
//!
//! ## Oracle
//! The golden words below were derived from the clang-validated `MVNI`
//! golden table (see `neon_mvni_pbt.rs`) by clearing `op` (bit 29), then
//! re-derived by hand from the ARMv8-A ARM layout. The independent
//! `ref_encode_movi` mirrors the documented layout directly (it is the MVNI
//! reference with `op = 0` and MOVI's cmode/arrangement set).
//!
//! ## Findings (documented by the `#[ignore]`d tests at the bottom)
//!  1. **`.2d` emits the MVNI opcode bit.** Every MOVI arrangement sets
//!     `op` (bit 29) = 0, but the `.2d` branch hardcodes
//!     `0b01101111 << 24` (top byte `0x6F`, bit 29 = 1). The correct MOVI
//!     word has top byte `0x4F`. Worse, `op = 1` with `cmode = 1110` is an
//!     *UNALLOCATED* encoding, so the emitted instruction does not decode to
//!     any defined MOVI. See `movi_2d_emits_mvni_op_bit`.
//!  2. **MSL shift on 32-bit elements silently dropped.** `movi v0.4s, #0x2a,
//!     msl #8` (and `msl #16`) is valid per the ARMv8-A ARM (cmode=1100/
//!     1101) but the encoder's `.2s/.4s` branch only matches `kind == "lsl"`
//!     and falls through to `cmode = 0000`, returning `Ok` with the
//!     no-shift word. See `movi_drops_msl_shift_on_32bit`.
//!  3. **Invalid shift kinds silently accepted.** `movi v0.4s, #5, lsr #8`
//!     (also `asr`/`ror`) is rejected by the ARMv8-A ARM (only LSL/MSL are
//!     defined), but the encoder falls through to `cmode = 0000` and returns
//!     `Ok`, silently changing the instruction's meaning. See
//!     `movi_silently_accepts_invalid_shift_kind`.
//!  4. **Out-of-range immediate silently truncated.** `movi v0.4s, #256`
//!     (also `#-1`, `#0x1ff`) must be rejected, but the encoder does
//!     `imm as u32 & 0xFF` and emits the masked word. See
//!     `movi_truncates_out_of_range_immediate`.
//!  5. **`.2d` rejects byte patterns the standard `MOVI .2D` accepts.**
//!     The branch only accepts immediates where every byte is `0x00` or
//!     `0xFF` (a byte-mask), but the ARMv8-A ARM `MOVI .2D` (cmode=1110)
//!     replicates `imm8` to *every* byte (e.g. `#0x0101010101010101`). This
//!     is a semantic mismatch layered on top of finding 1. See
//!     `movi_2d_rejects_replicate_form`.

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

/// Architecturally valid (arrangement, shift) combinations that the
/// implementation *claims* to support and handles correctly. (MSL is valid
/// per the spec but dropped by the impl — covered by an `#[ignore]`d test.)
fn valid_arr_shift_strategy() -> impl Strategy<Value = (&'static str, Option<(&'static str, u32)>)>
{
    let shift_32 = prop_oneof![
        Just(None),
        Just(Some(("lsl", 0u32))),
        Just(Some(("lsl", 8u32))),
        Just(Some(("lsl", 16u32))),
        Just(Some(("lsl", 24u32))),
    ];
    prop_oneof![
        (Just("8b"), Just(None)),
        (Just("16b"), Just(None)),
        (Just("2s"), shift_32.clone()),
        (Just("4s"), shift_32),
        (Just("4h"), Just(None)),
        (Just("8h"), Just(None)),
    ]
}

/// Independent reference encoder: assembles the word field-by-field from the
/// documented ARMv8-A ARM layout for the MOVI arrangements the impl handles
/// correctly (everything except `.2d`). It is NOT a copy of the impl's
/// expression shapes — cmode/shift dispatch is an explicit match.
fn ref_encode_movi(rd: u32, arr: &str, imm8: u32, shift: Option<(&str, u32)>) -> u32 {
    let q: u32 = match arr {
        "8b" | "2s" | "4h" => 0,
        "16b" | "4s" | "8h" => 1,
        _ => unreachable!("invalid arr in reference: {arr}"),
    };
    let abc = (imm8 >> 5) & 0x7;
    let defgh = imm8 & 0x1F;
    let cmode: u32 = match arr {
        "8b" | "16b" => 0b1110, // byte-replicate form
        "2s" | "4s" => match shift {
            None | Some(("lsl", 0)) => 0b0000,
            Some(("lsl", 8)) => 0b0010,
            Some(("lsl", 16)) => 0b0100,
            Some(("lsl", 24)) => 0b0110,
            Some(("msl", 8)) => 0b1100,
            Some(("msl", 16)) => 0b1101,
            _ => unreachable!("invalid shift in reference: {shift:?}"),
        },
        "4h" | "8h" => 0b1000, // 16-bit form, no shift
        _ => unreachable!(),
    };
    (q << 30) // op = 0 for MOVI (bit 29 clear)
        | (0b0111100u32 << 22) // bits 28-24 = 01111, bit 23 = 0, bit 22 = 0
        | (abc << 16)
        | (cmode << 12)
        | (0b01u32 << 10) // bits 11-10 = 01
        | (defgh << 5)
        | rd
}

/// cmode expected by the reference — used by the field-layout property to
/// assert cmode placement independently of the impl.
fn ref_cmode(arr: &str, shift: Option<(&str, u32)>) -> u32 {
    match arr {
        "8b" | "16b" => 0b1110,
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

// --- golden table (absolute oracle, derived from the clang-validated MVNI
//     table with bit 29 cleared, and re-derived by hand) -------------------

/// `(Rd, arrangement, imm8, Option<(shift_kind, amount)>, word)`.
/// Each MOVI word equals the corresponding MVNI golden word (see
/// `neon_mvni_pbt.rs`) with the `op` bit (29) cleared.
#[rustfmt::skip]
type GoldenRow = (u32, &'static str, u32, Option<(&'static str, u32)>, u32);

const GOLDEN: &[GoldenRow] = &[
    (0, "8b", 0xAB, None, 0x0F05E560),             // movi v0.8b,  #0xab
    (0, "16b", 0xAB, None, 0x4F05E560),            // movi v0.16b, #0xab
    (0, "4s", 0x00, None, 0x4F000400),             // movi v0.4s,  #0
    (5, "4s", 0x1F, None, 0x4F0007E5),             // movi v5.4s,  #0x1f
    (0, "2s", 0xFF, None, 0x0F0707E0),             // movi v0.2s,  #0xff
    (3, "4s", 0x20, Some(("lsl", 8)), 0x4F012403), // movi v3.4s, #0x20, lsl #8
    (2, "4h", 0x55, None, 0x0F0286A2),             // movi v2.4h, #0x55
    (1, "4s", 0xA5, Some(("lsl", 16)), 0x4F0544A1), // movi v1.4s, #0xa5, lsl #16
    (4, "4s", 0x3C, Some(("lsl", 24)), 0x4F016784), // movi v4.4s, #0x3c, lsl #24
    (6, "8h", 0x80, None, 0x4F048406),             // movi v6.8h, #0x80
    (0, "2s", 0x01, None, 0x0F000420),             // movi v0.2s, #1
];

#[test]
fn movi_matches_golden_table() {
    for &(rd, arr, imm8, shift, expected) in GOLDEN {
        let ops = ops_for(rd, arr, imm8, shift);
        let got = word_of(encode_neon_movi(&ops));
        assert_eq!(
            got,
            expected,
            "movi v{rd}.{arr}, #{imm8:#x}{}: got 0x{got:08X}, want 0x{expected:08X}",
            shift
                .map(|(k, a)| format!(", {k} #{a}"))
                .unwrap_or_default(),
        );
        // The independent reference must agree with the golden value too.
        assert_eq!(
            ref_encode_movi(rd, arr, imm8, shift),
            expected,
            "reference encoder drift for ({rd}, {arr}, {imm8:#x}, {shift:?})",
        );
    }
}

// --- properties (default `cargo test` stays green) ------------------------

proptest! {
    // === Oracle: differential against independent reference encoder ========
    // For every arrangement/shift the impl claims to support and every
    // register/immediate pair, the implementation must equal the
    // independently-assembled reference word.
    #[test]
    fn movi_matches_reference_encoder(
        rd in reg_num_strategy(),
        imm8 in imm8_strategy(),
        (arr, shift) in valid_arr_shift_strategy(),
    ) {
        let ops = ops_for(rd, arr, imm8, shift);
        let got = word_of(encode_neon_movi(&ops));
        let want = ref_encode_movi(rd, arr, imm8, shift);
        prop_assert_eq!(got, want);
    }

    // === Fixed-bits + field-placement invariant ===========================
    // Architecturally-constant bits never vary, and Rd/abc/defgh/cmode/Q
    // round-trip into their documented fields.
    #[test]
    fn movi_fixed_bits_and_field_layout(
        rd in reg_num_strategy(),
        imm8 in imm8_strategy(),
        (arr, shift) in valid_arr_shift_strategy(),
    ) {
        let ops = ops_for(rd, arr, imm8, shift);
        let w = word_of(encode_neon_movi(&ops));

        // --- constant fields ---
        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 0, "op bit (29) must be 0 for MOVI");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01111, "bits 28-24 = 01111");
        prop_assert_eq!((w >> 22) & 0x3, 0b00, "bits 23-22 = 00");
        prop_assert_eq!((w >> 10) & 0x3, 0b01, "bits 11-10 = 01");

        // --- round-tripped fields ---
        let abc = (imm8 >> 5) & 0x7;
        let defgh = imm8 & 0x1F;
        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, defgh, "defgh field");
        prop_assert_eq!((w >> 16) & 0x7, abc, "abc field");

        let expect_q: u32 = match arr {
            "8b" | "2s" | "4h" => 0,
            _ => 1,
        };
        prop_assert_eq!((w >> 30) & 1, expect_q, "Q bit must match arrangement");

        let expect_cmode = ref_cmode(arr, shift);
        prop_assert_eq!((w >> 12) & 0xF, expect_cmode,
            "cmode field must match arrangement/shift");
    }

    // === Determinism ======================================================
    // Encoding the same operands twice must yield the same word.
    #[test]
    fn movi_is_deterministic(
        rd in reg_num_strategy(),
        imm8 in imm8_strategy(),
        (arr, shift) in valid_arr_shift_strategy(),
    ) {
        let ops = ops_for(rd, arr, imm8, shift);
        let a = word_of(encode_neon_movi(&ops));
        let b = word_of(encode_neon_movi(&ops));
        prop_assert_eq!(a, b);
    }

    // === Negative contract: unsupported arrangement rejected ==============
    // Any arrangement other than the documented MOVI set must be an Err.
    #[test]
    fn movi_rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,3}".prop_filter("must be unknown to MOVI", |s| {
            !matches!(s.as_str(), "8b"|"16b"|"2s"|"4s"|"4h"|"8h"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), Operand::Imm(0)];
        prop_assert!(encode_neon_movi(&ops).is_err(),
            "MOVI does not support .{arr}; expected Err");
    }

    // === Negative contract: too few operands rejected =====================
    #[test]
    fn movi_rejects_too_few_operands(
        n in 0usize..2,
    ) {
        let ops: Vec<Operand> = (0..n).map(|_| va(0, "4s")).collect();
        prop_assert!(encode_neon_movi(&ops).is_err(),
            "MOVI requires at least 2 operands; expected Err for {n} operands");
    }

    // === Negative contract: invalid LSL amount rejected ===================
    // LSL amounts outside {0,8,16,24} must be Err for the 32-bit element form.
    #[test]
    fn movi_rejects_invalid_lsl_amount(
        bad_amt in 0u32..32u32,
    ) {
        prop_assume!(!matches!(bad_amt, 0 | 8 | 16 | 24));
        let ops = vec![va(0, "4s"), Operand::Imm(0),
                       Operand::Shift { kind: "lsl".to_string(), amount: bad_amt }];
        prop_assert!(encode_neon_movi(&ops).is_err(),
            "unsupported lsl amount {bad_amt} should be rejected");
    }
}

// --- documented finding 1: `.2d` emits the MVNI opcode bit ----------------

/// Every MOVI arrangement emits `op` (bit 29) = 0 (top byte `0x4F` for the
/// 128-bit forms). The `.2d` branch instead hardcodes
/// `0b01101111 << 24` (top byte `0x6F`, bit 29 = 1), which is the **MVNI**
/// opcode bit. The correct `movi v0.2d, #0` word is `0x4F00E400`; the impl
/// emits `0x6F00E400`. Worse, `op = 1` combined with `cmode = 1110` is an
/// *UNALLOCATED* encoding in the ARMv8-A ARM, so the emitted word does not
/// decode to any defined instruction.
///
/// `#[ignore]`d so the default `cargo test` stays green.
/// Run: `cargo test -- --ignored movi_2d_emits_mvni_op_bit`
#[test]
#[ignore]
fn movi_2d_emits_mvni_op_bit() {
    let ops = vec![va(0, "2d"), Operand::Imm(0)];
    let got = word_of(encode_neon_movi(&ops));

    // op (bit 29) must be 0 for MOVI — every other arrangement gets this right.
    assert_eq!(
        (got >> 29) & 1,
        0u32,
        "MOVI .2d must clear the op bit (29); got 0x{got:08X} with bit 29 set (0x6F = MVNI opcode)",
    );
    // The full word must be the MOVI form (0x4F...), not the MVNI form (0x6F...).
    assert_eq!(
        got, 0x4F00E400,
        "movi v0.2d, #0: got 0x{got:08X}, want 0x4F00E400",
    );
}

// --- documented finding 2: MSL shift on 32-bit elements silently dropped ---

/// MSL (multiply-shift-left) is valid for the 32-bit element form of MOVI
/// (`.2s`/`.4s`): `msl #8` -> cmode=1100, `msl #16` -> cmode=1101. The
/// encoder's `.2s`/`.4s` branch only matches `kind == "lsl"`, so an MSL
/// operand falls through to `cmode = 0000` and the shift is silently dropped.
///
/// `#[ignore]`d so the default `cargo test` stays green.
/// Run: `cargo test -- --ignored movi_drops_msl_shift_on_32bit`
#[test]
#[ignore]
fn movi_drops_msl_shift_on_32bit() {
    for &(arr, amt, expect_cmode) in &[
        ("4s", 8u32, 0b1100u32),
        ("4s", 16, 0b1101),
        ("2s", 8, 0b1100),
    ] {
        let ops = vec![
            va(0, arr),
            Operand::Imm(0x2A),
            Operand::Shift {
                kind: "msl".to_string(),
                amount: amt,
            },
        ];
        let res = encode_neon_movi(&ops);
        let w = match res {
            Ok(EncodeResult::Word(w)) => w,
            other => {
                panic!(
                    "movi v0.{arr}, #0x2a, msl #{amt}: expected Ok with cmode={expect_cmode:#06b}, got {other:?}"
                );
            }
        };
        let cmode = (w >> 12) & 0xF;
        assert_eq!(
            cmode, expect_cmode,
            "movi v0.{arr}, #0x2a, msl #{amt}: cmode dropped to {cmode:#06b} (expected {expect_cmode:#06b}); word=0x{w:08X}",
        );
    }
}

// --- documented finding 3: invalid shift kinds silently accepted -----------

/// `lsr`/`asr`/`ror` are not valid shift operators for MOVI. clang rejects
/// `movi v0.4s, #5, lsr #8` (also `asr`, `ror`); the ARMv8-A ARM allows only
/// LSL and MSL for the modified-immediate group. The implementation, however,
/// falls through the `else` branch to `cmode = 0b0000` and returns `Ok`,
/// silently turning the instruction into the no-shift form.
///
/// `#[ignore]`d so the default `cargo test` stays green.
/// Run: `cargo test -- --ignored movi_silently_accepts_invalid_shift_kind`
#[test]
#[ignore]
fn movi_silently_accepts_invalid_shift_kind() {
    for kind in &["lsr", "asr", "ror"] {
        let ops = vec![
            va(0, "4s"),
            Operand::Imm(5),
            Operand::Shift {
                kind: kind.to_string(),
                amount: 8,
            },
        ];
        let res = encode_neon_movi(&ops);
        assert!(
            res.is_err(),
            "{kind} is not a valid MOVI shift; expected Err but got Ok(0x{:08X})",
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

// --- documented finding 4: out-of-range immediate silently truncated -------

/// MOVI's immediate must be 0x00..=0xFF (clang: "immediate must be an integer
/// in range [0, 255]"). The implementation does `imm as u32 & 0xFF`, so e.g.
/// `#256` -> `#0` and `#-1` -> `#0xFF` silently.
///
/// `#[ignore]`d so the default `cargo test` stays green.
/// Run: `cargo test -- --ignored movi_truncates_out_of_range_immediate`
#[test]
#[ignore]
fn movi_truncates_out_of_range_immediate() {
    for &imm in &[256i64, 0x1FF, -1, 257] {
        let ops = vec![va(0, "4s"), Operand::Imm(imm)];
        assert!(
            encode_neon_movi(&ops).is_err(),
            "MOVI immediate {imm:#x} is out of range [0,255]; expected Err",
        );
    }
}

// --- documented finding 5: `.2d` rejects the standard replicate form -------

/// The ARMv8-A ARM `MOVI Vd.2D, #imm` (cmode=1110) replicates `imm8` to every
/// byte, so e.g. `movi v0.2d, #0x0101010101010101` is encodable (imm8=0x01).
/// The implementation's `.2d` branch instead requires every byte to be `0x00`
/// or `0xFF` (a byte-mask) and rejects the standard replicate form. This
/// compounds finding 1 (the opcode bit is also wrong for `.2d`).
///
/// `#[ignore]`d so the default `cargo test` stays green.
/// Run: `cargo test -- --ignored movi_2d_rejects_replicate_form`
#[test]
#[ignore]
fn movi_2d_rejects_replicate_form() {
    // Each byte == 0x01 -> standard MOVI imm8=0x01 replicate form.
    let ops = vec![va(0, "2d"), Operand::Imm(0x0101010101010101u64 as i64)];
    assert!(
        encode_neon_movi(&ops).is_ok(),
        "movi v0.2d, #0x0101010101010101 is the standard replicate form (imm8=0x01); expected Ok",
    );
}
