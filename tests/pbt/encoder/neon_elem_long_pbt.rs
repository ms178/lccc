//! Property-based tests for `encode_neon_elem_long`.
//!
//! `encode_neon_elem_long` encodes the AArch64 NEON "Advanced SIMD vector by
//! element" *long* multiply family — `SMULL`/`UMULL`/`SMLAL`/`UMLAL`/
//! `SMLSL`/`UMLSL`/`SQDMULL`/`SQDMLAL`/`SQDMLSL` (by element) — where the
//! destination is wider than the source, e.g. `SMULL Vd.4s, Vn.4h, Vm.h[i]`.
//!
//! ```text
//!   31 30 29 28-24 23-22 21 20 19-16 15-12 11 10 9-5 4-0
//!    0  Q  U  01111  size  L  M   Rm   opcode  H   0  Rn  Rd
//! ```
//! `size`/`Q` come from the source arrangement; `U` and the 4-bit `opcode`
//! identify the specific mnemonic (see dispatch in `encoder/mod.rs`, e.g.
//! smull→(U=0,opc=0b1010), umull→(U=1,opc=0b1010), smlal→(U=0,opc=0b0010)).
//!
//! Per the ARMv8-A ARM (Advanced SIMD vector by element, long multiply group):
//!   * halfword (`.h`) form: `index` = H:L:M (3 bits, 0..7), and **Rm is
//!     constrained to V0–V15** (the M bit is taken from the index, so only the
//!     low 4 bits of Rm are encodable);
//!   * word (`.s`) form: `index` = H:L (2 bits, 0..3), and M = Rm[4], so Rm
//!     may be V0–V31.
//!
//! ## Oracle
//! The golden words below were hand-derived field-by-field from the ARMv8-A ARM
//! bit layout (ARM DDI 0487) and are independent of this crate's
//! implementation; they anchor absolute correctness of every fixed field. A
//! separate `ref_encode_*` reference encoder re-assembles the word from the
//! documented layout for a differential cross-check.
//!
//! ## Findings (FAILING, shrinking `proptest!` properties)
//! * **Halfword Rm silently truncated** — `prop_halfword_rm_above_v15_must_be_rejected`.
//!   For the halfword form the ARM constrains Rm to V0–V15, but the encoder
//!   masks with `rm & 0xF` instead of validating, so `V16.h[i]`..`V31.h[i]`
//!   alias `V0.h[i]`..`V15.h[i]`. Silent corruption, no `Err`.
//! * **u_bit / opcode not range-validated** — `prop_out_of_range_u_bit_must_error`,
//!   `prop_out_of_range_opcode_must_error`. The sibling `encode_neon_three_same`
//!   rejects out-of-range u_bit/opcode (`neon.rs:3452`), establishing the codebase
//!   contract; `encode_neon_elem_long` does not, so out-of-range values corrupt
//!   adjacent fields (Q, Rm) silently.
//!   See the bug reports under the transplanted campaign notes.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_elem_long;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// `Operand::RegArrangement { reg: "v{n}", arrangement }`.
fn va(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

/// `Operand::RegLane { reg: "v{n}", elem_size, index }` — the by-element operand.
fn lane(n: u32, elem: &str, index: u32) -> Operand {
    Operand::RegLane {
        reg: format!("v{n}"),
        elem_size: elem.to_string(),
        index,
    }
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

/// Real opcodes/bits used by the dispatch for this instruction group.
fn opcode_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![
        Just(0b0010u32), // smlal / umlal
        Just(0b0011u32), // sqdmlal
        Just(0b0110u32), // smlsl / umlsl
        Just(0b0111u32), // sqdmlsl
        Just(0b1010u32), // smull / umull
        Just(0b1011u32), // sqdmull
    ]
}

/// `(arrangement, rm, index)` triples that are architecturally VALID, pairing
/// each source arrangement with the correct element size, Rm range and index
/// range: halfword (.4h/.8h) → Rm 0..15, index 0..7; word (.2s/.4s) → Rm 0..31,
/// index 0..3.
fn valid_case() -> impl Strategy<Value = (&'static str, u32, u32)> {
    prop_oneof![
        (Just("4h"), 0u32..=15, 0u32..=7),
        (Just("8h"), 0u32..=15, 0u32..=7),
        (Just("2s"), 0u32..=31, 0u32..=3),
        (Just("4s"), 0u32..=31, 0u32..=3),
    ]
}

/// Independent reference encoder assembled straight from the ARMv8-A layout.
/// For the word form it sets Rm[3:0] in the Rm field and M=Rm[4] separately
/// (matching the architecture), rather than the implementation's redundant
/// double-write of bit 20.
fn ref_encode_elem_long(
    (rd, rn, rm, index): (u32, u32, u32, u32),
    arr: &str,
    (u_bit, opcode): (u32, u32),
    is_high: bool,
) -> u32 {
    let (q_base, size) = match arr {
        "4h" => (0u32, 0b01u32),
        "8h" => (1u32, 0b01u32),
        "2s" => (0u32, 0b10u32),
        "4s" => (1u32, 0b10u32),
        _ => panic!("bad arrangement in reference encoder: {arr}"),
    };
    let q = if is_high { 1 } else { q_base };

    let (h, l, m, rm_field) = if size == 0b01 {
        assert!(index <= 7 && rm <= 15, "ref: invalid halfword input");
        let h = (index >> 2) & 1;
        let l = (index >> 1) & 1;
        let m = index & 1; // M comes from the index for .h
        (h, l, m, rm & 0xF)
    } else {
        assert!(index <= 3, "ref: invalid word index");
        let h = (index >> 1) & 1;
        let l = index & 1;
        let m = (rm >> 4) & 1; // M = Rm[4] for .s
        (h, l, m, rm & 0xF)
    };

    let mut w = 0u32;
    w |= q << 30;
    w |= u_bit << 29;
    w |= 0b01111u32 << 24;
    w |= size << 22;
    w |= l << 21;
    w |= m << 20;
    w |= rm_field << 16;
    w |= opcode << 12;
    w |= h << 11;
    // bit 10 is architecturally 0
    w |= rn << 5;
    w |= rd;
    w
}

type GoldenRow = (u32, u32, u32, u32, &'static str, u32, u32, bool, u32);

// --- golden table (absolute oracle) ---------------------------------------
//
// Each entry: (Rd, Rn, Rm, index, arrangement, u_bit, opcode, is_high, word),
// hand-derived from the layout in the module header. elem_size is implied by
// the arrangement's element class (".h" vs ".s").
const GOLDEN: &[GoldenRow] = &[
    // smull v0.4s, v1.4h, v2.h[0]   (U=0, opc=1010, narrow halfword)
    (0, 1, 2, 0, "4h", 0, 0b1010, false, 0x0F42A020),
    // smull2 v0.4s, v1.8h, v2.h[0]  (is_high → Q=1)
    (0, 1, 2, 0, "8h", 0, 0b1010, true, 0x4F42A020),
    // umull v0.4s, v1.4h, v2.h[0]   (U=1)
    (0, 1, 2, 0, "4h", 1, 0b1010, false, 0x2F42A020),
    // smull v0.2d, v1.2s, v2.s[0]   (word, size=10)
    (0, 1, 2, 0, "2s", 0, 0b1010, false, 0x0F82A020),
    // smull v0.2d, v1.2s, v16.s[0]  (M bit taken from Rm[4]=1)
    (0, 1, 16, 0, "2s", 0, 0b1010, false, 0x0F90A020),
    // smull v0.4s, v1.4h, v2.h[7]   (halfword max index → H:L:M=111)
    (0, 1, 2, 7, "4h", 0, 0b1010, false, 0x0F72A820),
    // smull v0.2d, v1.2s, v2.s[3]   (word max index → H:L=11)
    (0, 1, 2, 3, "2s", 0, 0b1010, false, 0x0FA2A820),
    // smlal v0.4s, v1.4h, v2.h[0]   (U=0, opc=0010)
    (0, 1, 2, 0, "4h", 0, 0b0010, false, 0x0F422020),
];

#[test]
fn elem_long_matches_golden_table() {
    for &(rd, rn, rm, index, arr, u_bit, opcode, is_high, expected) in GOLDEN {
        let elem = if arr.ends_with('h') { "h" } else { "s" };
        let ops = vec![
            va(rd, dest_arr(arr, is_high)),
            va(rn, arr),
            lane(rm, elem, index),
        ];
        let got = word_of(encode_neon_elem_long(&ops, u_bit, opcode, is_high));
        assert_eq!(
            got, expected,
            "arr={arr} idx={index} rm=v{rm} u={u_bit} opc=0b{opcode:b} hi={is_high}: \
             got 0x{got:08X}, want 0x{expected:08X}",
        );
        // The reference encoder must agree with the hand-derived golden value too.
        assert_eq!(
            ref_encode_elem_long((rd, rn, rm, index), arr, (u_bit, opcode), is_high),
            expected,
            "reference encoder drift",
        );
    }
}

/// The destination arrangement implied by a (source arrangement, is_high) pair.
/// `is_high` selects the wide ("2") destination; otherwise the narrow form.
fn dest_arr(src: &str, is_high: bool) -> &'static str {
    match src {
        "4h" | "8h" if is_high => "8h",
        "4h" | "8h" => "4h",
        "2s" | "4s" if is_high => "4s",
        "2s" | "4s" => "2s",
        _ => "2d",
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential reference encoder ============================
    // For every valid (arrangement, Rm, index, U, opcode, is_high) and register
    // pair, the implementation must equal the independently-assembled word.
    #[test]
    fn matches_reference_encoder(
        rd in 0u32..=31,
        rn in 0u32..=31,
        case in valid_case(),
        u_bit in 0u32..=1,
        opcode in opcode_strategy(),
        is_high in any::<bool>(),
    ) {
        let (arr, rm, index) = case;
        let elem = if arr.ends_with('h') { "h" } else { "s" };
        let ops = vec![va(rd, dest_arr(arr, is_high)), va(rn, arr), lane(rm, elem, index)];
        let got = word_of(encode_neon_elem_long(&ops, u_bit, opcode, is_high));
        let want = ref_encode_elem_long((rd, rn, rm, index), arr, (u_bit, opcode), is_high);
        prop_assert_eq!(got, want);
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits never change for any valid input:
    // bit31=0, bits28-24=01111, bit10=0.
    #[test]
    fn fixed_bits_are_constant(
        rd in 0u32..=31,
        rn in 0u32..=31,
        case in valid_case(),
        u_bit in 0u32..=1,
        opcode in opcode_strategy(),
        is_high in any::<bool>(),
    ) {
        let (arr, rm, index) = case;
        let elem = if arr.ends_with('h') { "h" } else { "s" };
        let ops = vec![va(rd, dest_arr(arr, is_high)), va(rn, arr), lane(rm, elem, index)];
        let w = word_of(encode_neon_elem_long(&ops, u_bit, opcode, is_high));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01111, "bits 28-24 must be 01111");
        prop_assert_eq!((w >> 10) & 1, 0, "bit 10 must be 0");
    }

    // === Field placement & index round-trip ===============================
    // Rd (bits 4-0) and Rn (bits 9-5) round-trip exactly; U (bit 29) and the
    // 4-bit opcode (bits 15-12) map correctly; and the lane index reconstructs
    // from the scattered H(bit11):L(bit21):M(bit20) fields. For the word form
    // the full 5-bit Rm must reconstruct as M:Rm[3:0].
    #[test]
    fn fields_and_index_round_trip(
        rd in 0u32..=31,
        rn in 0u32..=31,
        case in valid_case(),
        u_bit in 0u32..=1,
        opcode in opcode_strategy(),
        is_high in any::<bool>(),
    ) {
        let (arr, rm, index) = case;
        let elem = if arr.ends_with('h') { "h" } else { "s" };
        let ops = vec![va(rd, dest_arr(arr, is_high)), va(rn, arr), lane(rm, elem, index)];
        let w = word_of(encode_neon_elem_long(&ops, u_bit, opcode, is_high));

        prop_assert_eq!(w & 0x1F, rd, "Rd field (bits 4-0)");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field (bits 9-5)");
        prop_assert_eq!((w >> 29) & 1, u_bit, "U bit (bit 29)");
        prop_assert_eq!((w >> 12) & 0xF, opcode, "opcode field (bits 15-12)");

        let h = (w >> 11) & 1;
        let l = (w >> 21) & 1;
        let m = (w >> 20) & 1;
        if arr.ends_with('h') {
            // halfword: index = H:L:M (3 bits)
            prop_assert_eq!((h << 2) | (l << 1) | m, index, "halfword index H:L:M");
        } else {
            // word: index = H:L (2 bits), M = Rm[4]
            prop_assert_eq!((h << 1) | l, index, "word index H:L");
            prop_assert_eq!(m, (rm >> 4) & 1, "word M bit must equal Rm[4]");
            prop_assert_eq!((m << 4) | ((w >> 16) & 0xF), rm, "word Rm = M:Rm[3:0]");
        }
    }

    // === Negative contract: out-of-range lane index rejected ==============
    // Halfword indices must be 0..7, word indices 0..3; anything larger cannot
    // be encoded and must yield Err (no silent wrap).
    #[test]
    fn rejects_out_of_range_index(
        idx_h in 8u32..=0xFFFF,
        idx_s in 4u32..=0xFFFF,
    ) {
        let h_ops = vec![va(0, "4s"), va(1, "4h"), lane(2, "h", idx_h)];
        prop_assert!(encode_neon_elem_long(&h_ops, 0, 0b1010, false).is_err(),
            "halfword index {idx_h} must be rejected");

        let s_ops = vec![va(0, "2d"), va(1, "2s"), lane(2, "s", idx_s)];
        prop_assert!(encode_neon_elem_long(&s_ops, 0, 0b1010, false).is_err(),
            "word index {idx_s} must be rejected");
    }

    // === Negative contract: bad inputs rejected ===========================
    // Unsupported source arrangement, non-RegLane third operand, and too few
    // operands must all return Err rather than emit a malformed word.
    #[test]
    fn rejects_invalid_inputs(
        arr in "[a-z0-9]{1,3}".prop_filter("not a valid elem-long source", |s| {
            !matches!(s.as_str(), "4h" | "8h" | "2s" | "4s")
        }),
    ) {
        // unsupported arrangement
        let bad_arr = vec![va(0, "4s"), va(1, arr.as_str()), lane(2, "h", 0)];
        prop_assert!(encode_neon_elem_long(&bad_arr, 0, 0b1010, false).is_err(),
            "unsupported source arrangement {arr:?} must be rejected");

        // third operand is not a register lane
        let bad_lane = vec![va(0, "4s"), va(1, "4h"), va(2, "4h")];
        prop_assert!(encode_neon_elem_long(&bad_lane, 0, 0b1010, false).is_err(),
            "non-RegLane third operand must be rejected");

        // too few operands
        let short = vec![va(0, "4s"), va(1, "4h")];
        prop_assert!(encode_neon_elem_long(&short, 0, 0b1010, false).is_err(),
            "fewer than 3 operands must be rejected");
    }
}

// --- confirmed findings: FAILING, shrinking `proptest!` properties -------
//
// This property asserts the documented halfword-Rm range contract and FAILS on
// the current implementation. The shrunk witness is cited in
// the transplanted campaign notes.

proptest! {
    // === BUG: halfword Rm silently truncated (V16-V31 alias V0-V15) =======
    // For the halfword (`.h`) by-element long form the ARMv8-A ARM constrains
    // Rm to V0-V15 (the M bit is sourced from the lane index, leaving only a
    // 4-bit Rm field). The encoder masks with `rm & 0xF` instead of
    // validating, so v{N+16}.h[0] aliases v{N}.h[0]. Asserts the range
    // contract; FAILS on the current implementation.
    #[test]
    #[ignore = "documented bug: halfword by-element Rm V16-V31 aliases V0-V15"]
    fn prop_halfword_rm_above_v15_must_be_rejected(rm in 16u32..=31) {
        let ops = vec![va(0, "4s"), va(1, "4h"), lane(rm, "h", 0)];
        let res = encode_neon_elem_long(&ops, 0, 0b1010, false);
        prop_assert!(res.is_err(),
            "halfword by-element Rm must be V0-V15 (ARM DDI 0487); \
             v{rm}.h[0] must be rejected, but got {:?}", res);
    }
}

/// Concrete demonstration of the aliasing: `v{N+16}.h[0]` encodes identically
/// to `v{N}.h[0]` (both collapse to Rm field = N). This pins the buggy
/// behaviour so the finding is unambiguous; it will fail (as intended) once the
/// encoder is fixed to reject V16-V31 for the halfword form.
#[test]
fn halfword_rm_silently_aliases_v16_to_v0() {
    for n in 0u32..16 {
        let lo = word_of(encode_neon_elem_long(
            &[va(0, "4s"), va(1, "4h"), lane(n, "h", 0)],
            0,
            0b1010,
            false,
        ));
        let hi = word_of(encode_neon_elem_long(
            &[va(0, "4s"), va(1, "4h"), lane(n + 16, "h", 0)],
            0,
            0b1010,
            false,
        ));
        assert_eq!(
            lo,
            hi,
            "v{n}.h[0] (0x{lo:08X}) and v{}.h[0] (0x{hi:08X}) encode identically \
             — Rm was silently truncated",
            n + 16,
        );
    }
}
